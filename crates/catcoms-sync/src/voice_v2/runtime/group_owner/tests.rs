use super::*;
use catcoms_rt::ManualClock;
use std::future::Future;

// Fixed harness-only hooks, not production registry/allocation. Raw keys never
// authorize anything and cannot outlive their actual still-owned body allocation.
struct Hook {
    key: usize,
    entered: tokio::sync::oneshot::Sender<bool>,
    resume: std::sync::mpsc::Receiver<()>,
    map_returned: bool,
}
static HOOKS: Mutex<[Option<Hook>; 4]> = Mutex::new([None, None, None, None]);
pub(in crate::voice_v2) fn before_return(key: usize) {
    let hook = {
        let mut hooks = HOOKS.lock().unwrap();
        hooks
            .iter_mut()
            .find(|hook| hook.as_ref().is_some_and(|hook| hook.key == key))
            .and_then(Option::take)
    };
    if let Some(hook) = hook {
        let _ = hook.entered.send(hook.map_returned);
        let _ = hook.resume.recv();
    }
}
pub(super) fn map_returned(key: usize) {
    let mut hooks = HOOKS.lock().unwrap();
    if let Some(hook) = hooks.iter_mut().flatten().find(|hook| hook.key == key) {
        hook.map_returned = true;
    }
}
struct Resume(Option<std::sync::mpsc::Sender<()>>);
impl Drop for Resume {
    fn drop(&mut self) {
        if let Some(resume) = self.0.take() {
            let _ = resume.send(());
        }
    }
}
fn park(key: usize) -> (tokio::sync::oneshot::Receiver<bool>, Resume) {
    let (entered, receive) = tokio::sync::oneshot::channel();
    let (resume, wait) = std::sync::mpsc::channel();
    let mut hooks = HOOKS.lock().unwrap();
    assert!(!hooks.iter().flatten().any(|hook| hook.key == key));
    let slot = hooks
        .iter_mut()
        .find(|hook| hook.is_none())
        .expect("four bounded harness hooks");
    *slot = Some(Hook {
        key,
        entered,
        resume: wait,
        map_returned: false,
    });
    (receive, Resume(Some(resume)))
}
async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    let clock = catcoms_rt::SystemClock;
    let timeout = clock.sleep(std::time::Duration::from_secs(30));
    futures::pin_mut!(future, timeout);
    match futures::future::select(future, timeout).await {
        futures::future::Either::Left((result, _)) => result,
        futures::future::Either::Right(_) => panic!("sanctioned owner-return watchdog"),
    }
}
fn active() -> (Arc<Runtime>, ActorOwner, ActorRegistration) {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let device = DeviceId::from_bytes([3; 32]);
    let (mut actor, registration) = runtime.prepare_actor([7; 32], device, &ticket).unwrap();
    registration.register().unwrap();
    actor
        .sync_view(View {
            epoch: 1,
            members: BTreeSet::from([device]),
            policy: Some([8; 32]),
            eligible: true,
            indirect: true,
        })
        .unwrap();
    (runtime, actor, registration)
}

#[tokio::test]
async fn group_owner_escaped_ledgers_keep_same_fee_through_actual_group_and_concurrent_ledger_return(
) {
    bounded(async {
        let (runtime, actor, registration) = active();
        let pool = runtime.resources();
        let helper = actor.helper().unwrap();
        let source = actor.source().unwrap();
        assert!(helper.is_same(&actor.helper().unwrap()));
        assert!(source.is_same(&actor.source().unwrap()));
        let group_key = Arc::as_ptr(actor.group.body.as_ref().unwrap()) as usize;
        let source_key = Arc::as_ptr(source.body.as_ref().unwrap()) as usize;
        let (group_entered, group_resume) = park(group_key);
        drop(registration);
        drop(actor);
        let (group_done, group_returned) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            drop(runtime);
            let _ = group_done.send(());
        });
        assert!(
            group_entered.await.unwrap(),
            "actual map backing returned before the final Group can destroy its payer"
        );
        assert!(
            pool.counts().0 >= GROUP_METADATA_BYTES,
            "same payer while actual Group destructor is paused"
        );
        drop(group_resume);
        group_returned.await.unwrap();
        assert_eq!(
            pool.counts().0,
            GROUP_METADATA_BYTES,
            "both escaped empty ledgers retain original fee after actual Group/map return"
        );
        drop(helper);
        assert_eq!(
            pool.counts().0,
            GROUP_METADATA_BYTES,
            "one returned ledger cannot refund another physical owner"
        );
        let clone = source.clone();
        assert!(clone.is_same(&source));
        let (source_entered, source_resume) = park(source_key);
        let (first_done, first_returned) = tokio::sync::oneshot::channel();
        let (last_done, last_returned) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            drop(source);
            let _ = first_done.send(());
        });
        std::thread::spawn(move || {
            drop(clone);
            let _ = last_done.send(());
        });
        source_entered.await.unwrap();
        assert_eq!(
            pool.counts().0,
            GROUP_METADATA_BYTES,
            "concurrent aliases keep SAME fee through body/Arc destruction"
        );
        drop(source_resume);
        first_returned.await.unwrap();
        last_returned.await.unwrap();
        assert_eq!(
            pool.counts(),
            (0, 0),
            "final actual body, funding Arc/header and plain Credit returned"
        );
        assert_eq!(pool.group_counter_capacity_for_test(), (0, 0));
    })
    .await;
}

#[tokio::test]
async fn group_owner_runtime_unwind_installs_node_retirement_before_actual_request_revocation_waker(
) {
    bounded(async {
        struct PanicOnce {
            actor: Mutex<Option<ActorOwner>>,
            fired: AtomicBool,
        }
        impl std::task::Wake for PanicOnce {
            fn wake(self: Arc<Self>) {
                Self::wake_by_ref(&self);
            }
            fn wake_by_ref(self: &Arc<Self>) {
                if self.fired.swap(true, Ordering::AcqRel) {
                    return;
                }
                let actor = self.actor.lock().unwrap().take();
                drop(actor); // outside all guards; nested wakes are now inert
                panic!("injected genuine request revocation waker");
            }
        }
        let (runtime, actor, registration) = active();
        let pool = runtime.resources();
        let device = DeviceId::from_bytes([3; 32]);
        let effect = actor.effect(device, device, 16_000).unwrap();
        let key = Arc::as_ptr(actor.group.body.as_ref().unwrap()) as usize;
        let (entered, resume) = park(key);
        drop(registration);
        let panic = Arc::new(PanicOnce {
            actor: Mutex::new(Some(actor)),
            fired: AtomicBool::new(false),
        });
        let waker = std::task::Waker::from(panic.clone());
        let mut wait = Box::pin(effect.revoked());
        assert!(wait
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&waker))
            .is_pending());
        let before = pool.counts().0;
        let (done, returned) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let unwound =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(runtime))).is_err();
            let _ = done.send(unwound);
        });
        assert!(
            entered.await.unwrap(),
            "RAII freed actual map backing even after first native waker panicked"
        );
        assert!(
            pool.counts().0 >= GROUP_METADATA_BYTES,
            "original fee survives the actual paused body destruction on unwind"
        );
        drop(resume);
        assert!(returned.await.unwrap());
        assert!(
            panic.actor.lock().unwrap().is_none(),
            "the actual original Actor owner returned in the waker"
        );
        assert_eq!(before - pool.counts().0, GROUP_METADATA_BYTES);
        drop(wait);
        drop(effect);
        drop(waker);
        drop(panic);
        assert_eq!(pool.counts(), (0, 0));
        assert_eq!(pool.group_counter_capacity_for_test(), (0, 0));
    })
    .await;
}

#[test]
fn group_owner_complete_invoice_is_guarded_before_new_group_allocation() {
    let invoice = inventory_bytes();
    eprintln!("Complete Group/ledgers/funding/BTree/hash/constructor invoice {invoice}/{GROUP_METADATA_BYTES}");
    assert!(
        invoice <= GROUP_METADATA_BYTES,
        "never cut the graph to fit a fee"
    );
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let pool = runtime.resources();
    let held = pool
        .charge(
            [7; 32],
            super::super::super::resources::GROUP_BYTES - GROUP_METADATA_BYTES + 1,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = pool.counts();
    assert_eq!(
        runtime
            .prepare_actor(
                [7; 32],
                DeviceId::from_bytes([3; 32]),
                &runtime.session_ticket()
            )
            .err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(pool.counts(), before);
    assert!(
        runtime.state.lock().unwrap().groups.is_empty(),
        "failed first fee publishes no Group/ledger owner"
    );
    drop(held);
}
