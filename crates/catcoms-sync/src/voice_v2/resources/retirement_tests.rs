use super::*;
use catcoms_rt::Clock;

// Fixed harness-only pause after actual row removal and before native bucket
// return. Production has no callback, allocation or registry in this pure fence.
struct Hook {
    key: usize,
    entered: tokio::sync::oneshot::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}
static HOOKS: Mutex<[Option<Hook>; 2]> = Mutex::new([None, None]);
pub(super) fn before_shrink(key: usize) {
    let hook = {
        let mut hooks = HOOKS.lock().unwrap();
        hooks
            .iter_mut()
            .find(|hook| hook.as_ref().is_some_and(|hook| hook.key == key))
            .and_then(Option::take)
    };
    if let Some(hook) = hook {
        let _ = hook.entered.send(());
        let _ = hook.resume.recv();
    }
}
struct Resume(Option<std::sync::mpsc::Sender<()>>);
impl Drop for Resume {
    fn drop(&mut self) {
        if let Some(send) = self.0.take() {
            let _ = send.send(());
        }
    }
}
async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    let clock = catcoms_rt::SystemClock;
    let timeout = clock.sleep(std::time::Duration::from_secs(30));
    futures::pin_mut!(future, timeout);
    match futures::future::select(future, timeout).await {
        futures::future::Either::Left((value, _)) => value,
        futures::future::Either::Right(_) => panic!("sanctioned counter-return watchdog"),
    }
}
#[test]
fn group_owner_counter_backing_shrinks_sparse_rows_and_frees_actual_empty_credit_and_reserve_paths()
{
    let pool = Arc::new(Resources::default());
    let mut credits: Vec<_> = (1..=32)
        .map(|n| pool.charge([n; 32], 4096, ResourceKind::Retained).unwrap())
        .collect();
    pool.admit_voice_control([1; 32], 1000).unwrap();
    let service = {
        let state = pool.state.lock().unwrap();
        let service = state.groups[&[1; 32]].control;
        (service.last, service.tokens, service.initialized)
    };
    let large = pool.group_counter_capacity_for_test();
    while credits.len() > 1 {
        drop(credits.pop());
    }
    let sparse = pool.group_counter_capacity_for_test();
    assert_eq!(sparse.0, 1);
    assert!(
        sparse.1 <= 3 && sparse.1 < large.1,
        "no uncharged retained sparse buckets"
    );
    let state = pool.state.lock().unwrap();
    let current = state.groups[&[1; 32]].control;
    assert_eq!(
        (current.last, current.tokens, current.initialized),
        service,
        "live service horizon survives rehash untouched"
    );
    drop(state);
    drop(credits);
    assert_eq!(pool.counts(), (0, 0));
    assert_eq!(pool.group_counter_capacity_for_test(), (0, 0));
    let mut retained = pool.charge([9; 32], 4096, ResourceKind::Retained).unwrap();
    retained.release_retained_bytes(4096);
    assert_eq!(pool.counts(), (0, 0));
    assert_eq!(pool.group_counter_capacity_for_test(), (0, 0));
    drop(retained);
    for _ in 0..16 {
        drop(pool.charge([9; 32], 4096, ResourceKind::Retained).unwrap());
        assert_eq!(
            pool.group_counter_capacity_for_test(),
            (0, 0),
            "churn never retains unpaid empty backing"
        );
    }
}

#[test]
fn group_owner_counter_capacity_profile_fits_full_growth_and_shrink_invoice_at_group_bound() {
    let pool = Arc::new(Resources::default());
    let mut credits = Vec::with_capacity(GROUP_RECORDS);
    let buckets = |capacity: usize| {
        if capacity == 0 {
            0
        } else {
            (capacity + 1).next_power_of_two()
        }
    };
    for n in 1..=GROUP_RECORDS {
        let before = pool.group_counter_capacity_for_test();
        let mut group = [0; 32];
        group[..8].copy_from_slice(&(n as u64).to_be_bytes());
        credits.push(pool.charge(group, 1024, ResourceKind::Retained).unwrap());
        let after = pool.group_counter_capacity_for_test();
        let overlap = if before.1 == after.1 {
            buckets(after.1)
        } else {
            buckets(before.1) + buckets(after.1)
        };
        assert!(
            overlap <= 4 * after.0,
            "actual pinned capacity growth exceeds the full four-bucket invoice at {n}"
        );
    }
    while !credits.is_empty() {
        let before = pool.group_counter_capacity_for_test();
        drop(credits.pop());
        let after = pool.group_counter_capacity_for_test();
        if after.0 == 0 {
            assert_eq!(after.1, 0);
            break;
        }
        let overlap = if before.1 == after.1 {
            buckets(after.1)
        } else {
            buckets(before.1) + buckets(after.1)
        };
        assert!(
            overlap <= 4 * after.0,
            "actual shrink/sparse backing exceeds the complete survivor invoice"
        );
    }
    assert_eq!(pool.counts(), (0, 0));
}
#[tokio::test]
async fn group_owner_counter_retirement_keeps_budget_fence_closed_until_actual_buckets_return() {
    bounded(async {
        let pool = Arc::new(Resources::default());
        let original = pool.charge([1; 32], 4096, ResourceKind::Retained).unwrap();
        let key = {
            let state = pool.state.lock().unwrap();
            &*state as *const State as usize
        };
        let (entered, receive) = tokio::sync::oneshot::channel();
        let (resume, wait) = std::sync::mpsc::channel();
        {
            let mut hooks = HOOKS.lock().unwrap();
            *hooks.iter_mut().find(|hook| hook.is_none()).unwrap() = Some(Hook {
                key,
                entered,
                resume: wait,
            });
        }
        let resume = Resume(Some(resume));
        let (done, returned) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            drop(original);
            let _ = done.send(());
        });
        receive.await.unwrap();
        assert!(
            matches!(
                pool.state.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ),
            "no observer/admission can enter the logical-refund/physical-bucket gap"
        );
        let (started, starting) = tokio::sync::oneshot::channel();
        let (new_done, mut accepted) = tokio::sync::oneshot::channel();
        let next_pool = pool.clone();
        std::thread::spawn(move || {
            let _ = started.send(());
            let result = next_pool.charge([2; 32], 4096, ResourceKind::Retained);
            let _ = new_done.send(result);
        });
        starting.await.unwrap();
        assert!(
            accepted.try_recv().is_err(),
            "replacement cannot observe released budget before actual shrink finishes"
        );
        drop(resume);
        returned.await.unwrap();
        let next = accepted.await.unwrap().unwrap();
        assert_eq!(pool.counts().0, 4096);
        drop(next);
        assert_eq!(pool.group_counter_capacity_for_test(), (0, 0));
    })
    .await;
}
