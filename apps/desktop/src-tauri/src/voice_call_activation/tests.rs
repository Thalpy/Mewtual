//! Actual inactive native/Actor boundaries. These are not HTTP, browser, media or
//! audible acceptance. NativeActive here exposes no key/LocalJoin/packet producer.
use super::*;
use crate::voice_activation_tests::Fixture;
use catcoms_rt::ManualClock;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::{Context as TaskContext, Wake, Waker};

async fn prepared(f: &Fixture) -> Handle {
    let existing = f.candidate().await;
    let runtime = crate::voice_runtime(&f.state);
    let now = runtime.clock().monotonic_ms();
    let (id, controller, binding) = {
        let mut authority = f.state.voice_authority.lock().unwrap();
        let voice_authority::CandidateKind::Call {
            controller,
            binding,
        } = authority
            .candidate(&existing, catcoms_rt::SystemClock.monotonic_ms())
            .unwrap()
        else {
            panic!("Existing fixture candidate");
        };
        let id = authority
            .prepare(
                voice_authority::CandidateKind::AdmittedCall {
                    controller: controller.clone(),
                    binding: binding.clone(),
                },
                now,
            )
            .unwrap();
        (id, controller, binding)
    };
    let payment = ActivationRpcPermit::reserve(&runtime.resources(), binding.scope).unwrap();
    let pair = f
        .actor
        .prepare_admitted_voice_call(binding.scope.room, payment)
        .await
        .unwrap();
    Handle::new(id, controller, binding, pair, runtime.clock()).unwrap()
}
async fn native_winner(f: &Fixture, handle: &Handle) -> Worker {
    let commit = f.state.ui_session_commit.clone().lock_owned().await;
    f.state.ui_session_gate().require_unlocked().await.unwrap();
    f.state
        .ui_session_gate()
        .check_generation(handle.binding().generation)
        .unwrap();
    // RAII exists before CAS, spawn or first polling. No invoke waiter owns it.
    let worker = handle.initial_worker(Context::new(&f.state), commit);
    let now = handle.cell.clock.monotonic_ms();
    let old = {
        let mut authority = f.state.voice_authority.lock().unwrap();
        authority.retain_admitted_activation(handle).unwrap();
        authority.activate_admitted(handle, [55; 32], now).unwrap()
    };
    drop(old);
    worker
}
fn poll_once<F: Future>(future: Pin<&mut F>, waker: &Waker) -> Poll<F::Output> {
    future.poll(&mut TaskContext::from_waker(waker))
}
struct WakeProbe {
    authority: Weak<StdMutex<voice_authority::Coordinator>>,
    calls: AtomicUsize,
}
impl Wake for WakeProbe {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if let Some(authority) = self.authority.upgrade() {
            assert!(
                !matches!(
                    authority.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ),
                "new native task waker ran while Coordinator was held"
            );
        }
        self.calls.fetch_add(1, Ordering::AcqRel);
    }
}
fn probe(f: &Fixture) -> (Arc<WakeProbe>, Waker) {
    let probe = Arc::new(WakeProbe {
        authority: Arc::downgrade(&f.state.voice_authority),
        calls: AtomicUsize::new(0),
    });
    (probe.clone(), Waker::from(probe))
}
fn point() -> [u8; 65] {
    hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()
}

#[tokio::test]
async fn root_completion_admits_without_clock_advance_and_lost_reply_reuses_original() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = prepared(&f).await;
    assert!(!handle.original().is_active());
    let worker = native_winner(&f, &handle).await;
    assert!(
        f.state
            .voice_authority
            .lock()
            .unwrap()
            .owner(handle.id())
            .is_none(),
        "old key/crypto/handoff API has no admitted owner, even before Root selection"
    );
    let (_, waker) = probe(&f);
    let mut run = Box::pin(worker.run());
    assert!(poll_once(run.as_mut(), &waker).is_pending());
    assert!(f.state.ui_session_commit.try_lock().is_err());
    f.actor.member_count().await; // actual queued Root selection and argument retirement
    assert!(poll_once(run.as_mut(), &waker).is_pending());
    assert!(handle.active());
    assert!(f.state.ui_session_commit.try_lock().is_ok());
    let first = handle.reply().await;
    let first_projection = first.result().unwrap();
    drop(first); // a lost reply is never an implicit acknowledgement
    let recovered = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_activation(handle.id())
        .unwrap();
    assert!(recovered.same(&handle));
    let retry = recovered.reply().await;
    assert_eq!(retry.result().unwrap().join, first_projection.join);
    drop(retry);
    let grant = handle.grant().unwrap();
    assert!(
        f.actor
            .start_voice_endpoint(grant, [1; 32], point())
            .await
            .is_err(),
        "NativeActive is not a legacy LocalJoin/key producer permission"
    );
    let existing = f.candidate().await;
    assert!(
        crate::activate_voice_owner_inner(&f.state, existing)
            .await
            .is_err(),
        "an old native entry cannot replace the healthy admitted profile"
    );
    assert!(handle.active());
    handle.close();
    clock.advance_ms(CADENCE_MS);
    assert!(poll_once(run.as_mut(), &waker).is_pending());
    f.actor.member_count().await; // actual exact negative clear, not a synthetic return flag
    assert!(poll_once(run.as_mut(), &waker).is_ready());
    drop(run);
    assert!(
        !handle.collectible(),
        "cleanup is independent of the missing result ACK"
    );
    assert!(f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .forget_admitted_activation(&handle)
        .is_none());
    recovered.reply().await.acknowledge();
    assert!(handle.collectible());
    let removed = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .forget_admitted_activation(&handle)
        .unwrap();
    drop(removed); // all heavy final destruction belongs outside Coordinator
    f.close().await;
}

#[tokio::test]
async fn parked_root_lock_closes_before_ui_release_without_a_child_watch_subscriber() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = prepared(&f).await;
    let worker = native_winner(&f, &handle).await;
    let expected_context = f.actor.durable_send_context().await.unwrap();
    let paused = f
        .actor
        .prepare_durable_send(catcoms_app::durable_chat::DurableSendRequest {
            token: [121; 16],
            expected_context,
            channel: crate::channel_id("general"),
            text: "never authored".into(),
            reply_to: String::new(),
        })
        .await
        .unwrap();
    let (probe, waker) = probe(&f);
    let mut run = Box::pin(worker.run());
    assert!(poll_once(run.as_mut(), &waker).is_pending());
    let work = handle.work().unwrap();
    assert!(!work.is_retired());
    let before = probe.calls.load(Ordering::Acquire);
    crate::register_ui_lock_request(&f.state);
    f.state.voice_authority.lock().unwrap().revoke_all();
    assert_eq!(
        probe.calls.load(Ordering::Acquire),
        before,
        "new pending task subscribed only to its Clock/Work, not guarded admitted child watch"
    );
    assert!(handle.original().is_closed());
    assert!(
        f.state.ui_session_commit.try_lock().is_err(),
        "one original tick may still be pending"
    );
    clock.advance_ms(CADENCE_MS);
    assert!(poll_once(run.as_mut(), &waker).is_pending());
    assert!(
        f.state.ui_session_commit.try_lock().is_ok(),
        "lock no longer waits behind parked Root"
    );
    assert!(
        !work.is_retired(),
        "lock did not discard actual Actor argument ownership"
    );
    assert!(!handle.collectible());
    assert!(handle.reply().await.result().is_err());
    drop(paused);
    f.actor.member_count().await;
    assert!(poll_once(run.as_mut(), &waker).is_pending());
    f.actor.member_count().await;
    assert!(poll_once(run.as_mut(), &waker).is_ready());
    drop(run);
    assert!(work.is_retired());
    assert!(!handle.active());
    handle.reply().await.acknowledge();
    let removed = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .forget_admitted_activation(&handle)
        .unwrap();
    drop(removed);
    f.close().await;
}

#[tokio::test]
async fn unpolled_task_drop_closes_original_before_owned_ui_guard_release() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock).await;
    let handle = prepared(&f).await;
    let worker = native_winner(&f, &handle).await;
    let run = worker.run(); // never polled; RAII is already an owned future argument
    drop(run);
    assert!(handle.original().is_closed());
    assert!(f.state.ui_session_commit.try_lock().is_ok());
    assert!(handle
        .cell
        .state
        .lock()
        .unwrap()
        .task
        .returned
        .load(Ordering::Acquire));
    assert!(
        !handle.collectible(),
        "unpolled return is not Root/issuer cleanup or ACK"
    );
    let retry = handle.negative_worker(Context::new(&f.state)).unwrap();
    assert!(
        handle.negative_worker(Context::new(&f.state)).is_none(),
        "same returned ticket has one negative claimant"
    );
    let (_, waker) = probe(&f);
    let mut run = Box::pin(retry.run());
    assert!(poll_once(run.as_mut(), &waker).is_pending());
    f.actor.member_count().await;
    assert!(poll_once(run.as_mut(), &waker).is_ready());
    drop(run);
    handle.reply().await.acknowledge();
    let removed = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .forget_admitted_activation(&handle)
        .unwrap();
    drop(removed);
    f.close().await;
}

#[derive(Debug)]
struct InlineClock {
    samples: AtomicUsize,
    sleeps: AtomicUsize,
    exhausted: bool,
}
impl Clock for InlineClock {
    fn now_ms(&self) -> u64 {
        1000
    }
    fn monotonic_ms(&self) -> u64 {
        assert!(
            self.samples.fetch_add(1, Ordering::AcqRel) < 8,
            "clock retry spun in one poll"
        );
        if self.exhausted {
            u64::MAX
        } else {
            1000
        }
    }
    fn sleep(&self, _: std::time::Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.sleeps.fetch_add(1, Ordering::AcqRel);
        Box::pin(async {})
    }
}
#[tokio::test]
async fn cadence_registers_once_and_rejects_inline_or_exhausted_clock_without_spinning() {
    for exhausted in [false, true] {
        let clock = InlineClock {
            samples: AtomicUsize::new(0),
            sleeps: AtomicUsize::new(0),
            exhausted,
        };
        assert_eq!(cadence_tick(&clock).await, Err(Failure::Interrupted));
        assert_eq!(
            clock.sleeps.load(Ordering::Acquire),
            usize::from(!exhausted)
        );
    }
    let clock = ManualClock::new(1000);
    let mut tick = Box::pin(cadence_tick(&clock));
    let waker = Waker::noop();
    assert!(poll_once(tick.as_mut(), waker).is_pending());
    clock.advance_ms(CADENCE_MS - 1);
    assert!(poll_once(tick.as_mut(), waker).is_pending());
    clock.advance_ms(1);
    assert!(matches!(
        poll_once(tick.as_mut(), waker),
        Poll::Ready(Ok(()))
    ));
}
