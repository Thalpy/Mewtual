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
/// Actual native UI/CAS -> original Actor selection -> final native admission.
/// The caller owns only this test-harness outer Box; it must retain/poll or drop
/// the original runner. No test flips the issuer's activation phase directly.
pub(crate) async fn active_handle_fixture(
    f: &Fixture,
) -> (Handle, Pin<Box<dyn Future<Output = ()> + Send>>) {
    let handle = prepared(f).await;
    let worker = native_winner(f, &handle).await;
    let mut runner = Box::pin(worker.run());
    assert!(poll_once(runner.as_mut(), Waker::noop()).is_pending());
    f.actor.member_count().await;
    assert!(poll_once(runner.as_mut(), Waker::noop()).is_pending());
    assert!(handle.active());
    (handle, runner)
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
    assert!(handle.cell.state.lock().unwrap().task.has_returned());
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

#[tokio::test]
async fn complete_native_graph_and_sanctioned_clock_bodies_fit_before_admission() {
    let bytes = native_layout_bytes();
    eprintln!("activation complete native graph {bytes}/{NATIVE_BYTES}");
    assert!(bytes <= NATIVE_BYTES);
    fn arc_bytes<T: ?Sized>(value: &T) -> usize {
        std::alloc::Layout::new::<[usize; 2]>()
            .extend(std::alloc::Layout::for_value(value))
            .unwrap()
            .0
            .pad_to_align()
            .size()
    }
    // Measure the actual private EffectiveClock through Runtime, not merely a
    // raw source. ManualClock additionally owns two scalar and one Notify Arcs.
    let sources: [(Arc<dyn Clock>, usize); 2] = [
        (Arc::new(catcoms_rt::SystemClock), 0),
        (
            Arc::new(ManualClock::new(1000)),
            2 * arc_bytes(&AtomicU64::new(0)) + arc_bytes(&tokio::sync::Notify::new()),
        ),
    ];
    for (source, source_backing) in sources {
        let runtime = catcoms_sync::voice_v2::runtime::Runtime::new(source.clone());
        let clock = runtime.clock();
        let sleeper = clock.sleep(std::time::Duration::from_millis(CADENCE_MS));
        assert!(std::mem::size_of_val(sleeper.as_ref().get_ref()) <= CLOCK_SLEEP_BYTES);
        let bytes = arc_bytes(clock.as_ref()) + arc_bytes(source.as_ref()) + source_backing;
        eprintln!("activation Clock owner chain {bytes}/{CLOCK_OWNER_BYTES}");
        assert!(bytes <= CLOCK_OWNER_BYTES);
    }
}

struct ResumeOnDrop(Option<std::sync::mpsc::Sender<()>>);
impl Drop for ResumeOnDrop {
    fn drop(&mut self) {
        if let Some(resume) = self.0.take() {
            let _ = resume.send(());
        }
    }
}
async fn bounded<F: Future>(future: F) -> F::Output {
    tokio::select! {
        output = future => output,
        _ = catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(30)) => {
            panic!("native destruction barrier did not finish");
        }
    }
}

#[tokio::test]
async fn runner_joins_actual_execution_drop_before_return_for_completion_and_cancellation() {
    // This drives the ACTUAL outer owner, not a helper passed a literal returned
    // flag. A future whose destructor is parked still owns its Box and capture.
    struct Execution {
        entered: Option<tokio::sync::oneshot::Sender<()>>,
        resume: std::sync::mpsc::Receiver<()>,
        _capture: Box<[u8; 1024]>,
    }
    impl Future for Execution {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }
    impl Drop for Execution {
        fn drop(&mut self) {
            let _ = self.entered.take().unwrap().send(());
            self.resume.recv().expect("RAII barrier release");
        }
    }
    for completed in [false, true] {
        let resources = Arc::new(catcoms_sync::voice_v2::resources::Resources::default());
        let funding = Arc::new(Funding {
            _credit: resources
                .charge(
                    [72; 32],
                    NATIVE_BYTES,
                    catcoms_sync::voice_v2::resources::ResourceKind::Retained,
                )
                .unwrap(),
        });
        let returned = Arc::new(AtomicU64::new(0));
        let (entered, at_drop) = tokio::sync::oneshot::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let release = ResumeOnDrop(Some(resume));
        let running = RunningWorker {
            future: Some(Box::pin(Execution {
                entered: Some(entered),
                resume: resumed,
                _capture: Box::new([0; 1024]),
            })),
            returned: Some(TaskReturned {
                signal: returned.clone(),
                serial: 1,
                cell: Weak::new(), // no real native Cell in this execution-only fixture
                _funding: funding,
            }),
        };
        let (done, finished) = tokio::sync::oneshot::channel();
        // No blocking join on the async executor; the completion channel proves
        // the original thread has actually returned, even after an assertion panic.
        drop(std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if completed {
                    let mut running = Box::pin(running);
                    assert!(poll_once(running.as_mut(), Waker::noop()).is_ready());
                } else {
                    drop(running);
                }
            }));
            let _ = done.send(result);
        }));
        bounded(at_drop).await.unwrap();
        assert_eq!(returned.load(Ordering::Acquire), 0);
        assert_eq!(resources.counts().0, NATIVE_BYTES);
        drop(release);
        bounded(finished).await.unwrap().unwrap();
        assert_eq!(returned.load(Ordering::Acquire), 1);
        assert_eq!(resources.counts().0, 0);
    }
}

#[tokio::test]
async fn suspended_native_cancellation_recovers_only_after_its_box_returns_and_joins_same_work() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock).await;
    let handle = prepared(&f).await;
    let worker = native_winner(&f, &handle).await;
    let expected_context = f.actor.durable_send_context().await.unwrap();
    let parked = f
        .actor
        .prepare_durable_send(catcoms_app::durable_chat::DurableSendRequest {
            token: [122; 16],
            expected_context,
            channel: crate::channel_id("general"),
            text: "never authored".into(),
            reply_to: String::new(),
        })
        .await
        .unwrap();
    let (_, waker) = probe(&f);
    let mut run = Box::pin(worker.run());
    assert!(poll_once(run.as_mut(), &waker).is_pending());
    let original_work = handle.work().unwrap();
    handle.close();
    assert!(handle.negative_worker(Context::new(&f.state)).is_none());
    drop(run); // joins actual execution Box before publishing the original return
    assert!(handle.cell.state.lock().unwrap().task.has_returned());
    assert!(!original_work.is_retired());
    let retry = handle.negative_worker(Context::new(&f.state)).unwrap();
    let mut run = Box::pin(retry.run());
    assert!(poll_once(run.as_mut(), &waker).is_pending());
    assert!(handle.work().unwrap().is_same(&original_work));
    assert!(!handle.collectible());
    drop(parked);
    f.actor.member_count().await;
    assert!(poll_once(run.as_mut(), &waker).is_pending());
    f.actor.member_count().await;
    assert!(poll_once(run.as_mut(), &waker).is_ready());
    assert!(original_work.is_retired());
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

#[tokio::test]
async fn coordinator_deallocation_keeps_original_native_payer_through_backing_return_and_unwind() {
    for revocation_panics in [false, true] {
        let f = Fixture::with_voice_clock(Arc::new(ManualClock::new(1000))).await;
        let handle = prepared(&f).await;
        let funding = Arc::downgrade(&handle.funding);
        let runtime = crate::voice_runtime(&f.state);
        let resources = runtime.resources();
        let mut coordinator = {
            let mut authority = f.state.voice_authority.lock().unwrap();
            authority.retain_admitted_activation(&handle).unwrap();
            std::mem::replace(
                &mut *authority,
                voice_authority::Coordinator::with_runtime(runtime),
            )
        };
        drop(handle); // the actual index now owns the sole native Cell/payer
        assert_eq!(funding.strong_count(), 1);
        let (entered, at_return) = tokio::sync::oneshot::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let release = ResumeOnDrop(Some(resume));
        let (done, finished) = tokio::sync::oneshot::channel();
        drop(std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                coordinator.destroy_activation_index(
                    |coordinator| {
                        coordinator.revoke_all(); // actual Runtime/controller/owner revocation
                        assert!(!revocation_panics, "inherited revocation callback unwind");
                    },
                    || {
                        let _ = entered.send(()); // actual full Vec has been deallocated
                        resumed.recv().expect("RAII backing barrier release");
                    },
                );
            }));
            assert_eq!(result.is_err(), revocation_panics);
            drop(coordinator);
            let _ = done.send(());
        }));
        bounded(at_return).await.unwrap();
        assert_eq!(
            funding.strong_count(),
            1,
            "SAME original pays after Vec deallocation"
        );
        assert!(resources.counts().0 >= NATIVE_BYTES);
        drop(release);
        bounded(finished).await.unwrap();
        assert_eq!(funding.strong_count(), 0);
        drop(funding);
        f.close().await;
    }
}

#[tokio::test]
async fn repeated_negative_recovery_uses_one_exact_return_stamp_and_checked_serial() {
    let f = Fixture::with_voice_clock(Arc::new(ManualClock::new(1000))).await;
    let handle = prepared(&f).await;
    drop(native_winner(&f, &handle).await.run());
    let original = handle.cell.state.lock().unwrap().task.returned.clone();
    let before = crate::voice_runtime(&f.state).resources().counts();
    for _ in 0..65 {
        let retry = handle.negative_worker(Context::new(&f.state)).unwrap();
        {
            let state = handle.cell.state.lock().unwrap();
            assert!(Arc::ptr_eq(&state.task.returned, &original));
            assert!(Arc::ptr_eq(
                &retry.returned.as_ref().unwrap().signal,
                &original
            ));
            assert!(
                !state.task.has_returned(),
                "old completed stamp is not current return"
            );
        }
        assert!(handle.negative_worker(Context::new(&f.state)).is_none());
        drop(retry.run()); // SAME actual Box return boundary; no per-retry Arc allocation
        assert!(handle.cell.state.lock().unwrap().task.has_returned());
        assert_eq!(crate::voice_runtime(&f.state).resources().counts(), before);
    }
    let serial = handle.cell.state.lock().unwrap().task.serial;
    original.store(serial + 1, Ordering::Release);
    assert!(
        handle.negative_worker(Context::new(&f.state)).is_none(),
        "future stamp is not equality"
    );
    {
        let mut state = handle.cell.state.lock().unwrap();
        state.task.serial = u64::MAX;
        original.store(u64::MAX, Ordering::Release);
    }
    assert!(
        handle.negative_worker(Context::new(&f.state)).is_none(),
        "overflow never remints a ticket"
    );
    {
        let mut state = handle.cell.state.lock().unwrap();
        assert_eq!(state.task.serial, u64::MAX);
        assert!(Arc::ptr_eq(&state.task.returned, &original));
        state.task.serial = serial;
        original.store(serial, Ordering::Release);
    }
    let (_, waker) = probe(&f);
    let mut cleanup = Box::pin(
        handle
            .negative_worker(Context::new(&f.state))
            .unwrap()
            .run(),
    );
    assert!(poll_once(cleanup.as_mut(), &waker).is_pending());
    f.actor.member_count().await;
    assert!(poll_once(cleanup.as_mut(), &waker).is_ready());
    drop(cleanup);
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
