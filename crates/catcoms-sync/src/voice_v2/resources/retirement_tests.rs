use super::*;
use catcoms_rt::Clock;
use std::hash::BuildHasher;

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
    // This monotone grow/shrink sequence has no intervening insert after a
    // deletion. Matching pinned raw::capacity_to_buckets maps live rows to the
    // minimum allocation after shrink_to(len), NOT reported logical capacity.
    let buckets = |rows: usize| match rows {
        0 => 0,
        1..=3 => 4,
        4..=7 => 8,
        _ => (rows * 8 / 7).next_power_of_two(),
    };
    for n in 1..=GROUP_RECORDS {
        let before = pool.group_counter_capacity_for_test();
        let mut group = [0; 32];
        group[..8].copy_from_slice(&(n as u64).to_be_bytes());
        credits.push(pool.charge(group, 1024, ResourceKind::Retained).unwrap());
        let after = pool.group_counter_capacity_for_test();
        let overlap = if buckets(before.0) == buckets(after.0) {
            buckets(after.0)
        } else {
            buckets(before.0) + buckets(after.0)
        };
        assert!(
            overlap <= 4 * after.0,
            "nominal monotone growth exceeds its stronger four-bucket bound at {n}: {before:?} -> {after:?}"
        );
    }
    let mut logical_changes = 0;
    while !credits.is_empty() {
        let before = pool.group_counter_capacity_for_test();
        drop(credits.pop());
        let after = pool.group_counter_capacity_for_test();
        if after.0 == 0 {
            assert_eq!(after.1, 0);
            break;
        }
        let overlap = if buckets(before.0) == buckets(after.0) {
            buckets(after.0)
        } else {
            buckets(before.0) + buckets(after.0)
        };
        if before.1 != after.1 && buckets(before.0) == buckets(after.0) {
            logical_changes += 1;
            if logical_changes <= 3 {
                eprintln!("Logical counter capacity changed without bucket resizing: {before:?} -> {after:?}; minimum raw buckets {}",buckets(after.0));
            }
        }
        assert!(
            overlap <= 4 * after.0,
            "monotone shrink/sparse backing exceeds its stronger four-bucket bound: {before:?} -> {after:?}"
        );
    }
    eprintln!("Pinned monotone profile observed {logical_changes} logical-capacity changes without bucket resizing");
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn group_owner_counter_adversarial_tombstone_churn_needs_eight_bucket_invoice_without_resetting_survivors(
) {
    let pool = Arc::new(Resources::default());
    let hasher = pool.state.lock().unwrap().groups.hasher().clone();
    let mut initial = Vec::new();
    let mut replacements = Vec::new();
    // Bounded deterministic search against this ACTUAL existing hasher. No new
    // RNG/hasher or fake production adapter. All initial keys start at bucket0;
    // replacements use the untouched half-table, preserving DELETED controls.
    for n in 1..=1_000_000u64 {
        let mut key = [0; 32];
        key[..8].copy_from_slice(&n.to_be_bytes());
        match hasher.hash_one(key) & 63 {
            0 if initial.len() < 47 => initial.push(key),
            32 if replacements.len() < 32 => replacements.push(key),
            _ => {}
        }
        if initial.len() == 47 && replacements.len() == 32 {
            break;
        }
    }
    assert_eq!(
        (initial.len(), replacements.len()),
        (47, 32),
        "bounded adversarial harness corpus"
    );
    let mut credits: std::collections::VecDeque<_> = initial
        .iter()
        .map(|key| {
            (
                *key,
                pool.charge(*key, 2048, ResourceKind::Retained).unwrap(),
            )
        })
        .collect();
    assert_eq!(pool.group_counter_capacity_for_test(), (47, 56)); //64 actual buckets before churn
    let survivor = *initial.last().unwrap();
    pool.admit_voice_control(survivor, 1000).unwrap();
    let service = {
        let state = pool.state.lock().unwrap();
        let s = state.groups[&survivor].control;
        (s.last, s.tokens, s.initialized)
    };
    let mut witnessed = false;
    for key in replacements {
        drop(credits.pop_front()); // actual pure removal/shrink path;46 survivors
        assert_eq!(pool.group_counter_capacity_for_test().0, 46);
        credits.push_back((key, pool.charge(key, 2048, ResourceKind::Retained).unwrap()));
        let observed = pool.group_counter_capacity_for_test();
        if observed.1 > 56 {
            // capacity() is only a lower bound, so112 positively requires128
            // raw buckets. The old64+new128 peak exceedsH4 at47 rows. This is
            // real Resources execution, not an assumption from tombstone math.
            assert_eq!(observed, (47, 112));
            assert!(64 + 128 > 4 * observed.0);
            assert!(64 + 128 <= 8 * observed.0);
            eprintln!("Actual tombstone grow: 64 ->128 raw buckets;47 live rows; reported capacity112; old+new192 >H4=188, <=H8=376");
            witnessed = true;
            break;
        }
    }
    assert!(
        witnessed,
        "actual forced tombstone growth must exercise the real underinvoice regression"
    );
    let state = pool.state.lock().unwrap();
    let s = state.groups[&survivor].control;
    assert_eq!((s.last, s.tokens, s.initialized), service);
    drop(state);
    drop(credits);
    assert_eq!(pool.group_counter_capacity_for_test(), (0, 0));
}

#[test]
fn group_owner_eight_bucket_structural_bound_covers_pinned_small_and_tombstone_resize_cases() {
    // Exact pinned small capacity minima: raw4/8 cannot retain DELETED controls
    // on native16-byte groups; their only allocating grow needs4/8 live rows.
    for (old, new, live) in [(0, 4, 1), (4, 8, 4), (8, 16, 8)] {
        assert!(old + new <= 4 * live);
    }
    for exponent in 4..=12 {
        let raw = 1usize << exponent;
        for live in (7 * raw / 16 + 1)..=7 * raw / 8 {
            assert!(
                raw + 2 * raw < 8 * live,
                "allocating grow at >half the7/8 capacity"
            );
        }
    }
    // General immediate shrink after such early growth: retained oldB<=32n/7,
    // minimal newB<16n/7, hence old+new<(48/7)n<8n. Keep controls/alignment
    // separate in production; this structural proof is not allocator tracing.
    assert!(48 < 8 * 7);
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
