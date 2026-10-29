//! Actual original-channel retirement, not a weak-upgrade/closed-channel return proof.
//! These tests keep ordinary Sender handles alive across receiver destruction. Otherwise
//! dropping the last Sender could conceal an unread late-send ownership cycle.
use super::*;
use crate::actor::voice_endpoint::tests::{found, park, start};
use catcoms_rt::Clock;
use rand_core::SeedableRng;
use std::future::Future;
use std::sync::mpsc as thread_channel;

/// Application behavior still uses the fixture's ManualClock. Only this outer
/// test-harness watchdog uses the sanctioned real Clock seam, so a broken send
/// fence/retirement cannot hang the entire suite waiting for an OS thread.
async fn bounded<T>(what: &'static str, future: impl Future<Output = T>) -> T {
    let clock = catcoms_rt::SystemClock;
    tokio::select! {
        value = future => value,
        _ = clock.sleep(std::time::Duration::from_secs(30)) => panic!("control regression timed out: {what}"),
    }
}

struct ResumeOnDrop(Option<thread_channel::Sender<()>>);
impl ResumeOnDrop {
    fn release(mut self) {
        self.0.take().unwrap().send(()).unwrap();
    }
}
impl Drop for ResumeOnDrop {
    fn drop(&mut self) {
        // Any failed assertion/timeout releases accepted(), before the test drops
        // its ordinary Actor handles. No detached thread stays on this receiver.
        if let Some(resume) = self.0.take() {
            let _ = resume.send(());
        }
    }
}
fn producer(
    run: impl FnOnce() -> Result<VoiceCallActivationWork, VoiceEndpointError> + Send + 'static,
) -> oneshot::Receiver<Result<VoiceCallActivationWork, VoiceEndpointError>> {
    let (done, result) = oneshot::channel();
    // Do not synchronously join an OS thread from the executor. Queue-call return
    // is observed through this bounded channel; original Work retirement remains
    // its independent exact signal, never inferred from this harness completion.
    drop(std::thread::spawn(move || {
        let _ = done.send(run());
    }));
    result
}

#[derive(Default)]
struct Outcome {
    work: Option<VoiceCallActivationWork>,
    result: Option<VoiceCallActivationResult>,
    refused: Option<(
        VoiceEndpointError,
        Option<VoiceCallActivationWork>,
        Option<ActivationRpcPermit>,
    )>,
    uncertain: bool,
}
struct Record {
    outcome: Mutex<Outcome>,
    changed: tokio::sync::Notify,
    // Deliberately retain the real control in Publisher -> Record. A Tokio
    // WeakSender or full Actor here would reintroduce the channel cycle.
    _control: VoiceActivationControl,
    _credit: catcoms_sync::voice_v2::resources::Credit,
}
impl Record {
    fn new(original: &ActivationObserver, control: &VoiceActivationControl) -> Arc<Self> {
        let credit = original.reserve_retained(4096).unwrap();
        Arc::new(Self {
            outcome: Mutex::new(Outcome::default()),
            changed: tokio::sync::Notify::new(),
            _control: control.clone(),
            _credit: credit,
        })
    }
    fn work(&self) -> VoiceCallActivationWork {
        self.outcome.lock().unwrap().work.as_ref().unwrap().clone()
    }
    fn assert_outside_send_fence(&self) {
        if let Some(owner) = self._control.upgrade() {
            assert!(
                !matches!(
                    owner.owner.send.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ),
                "publisher callback must not run under the original send fence"
            );
        }
    }
    async fn ready(&self) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let outcome = self.outcome.lock().unwrap();
                if outcome.result.is_some() || outcome.refused.is_some() || outcome.uncertain {
                    return;
                }
            }
            notified.await;
        }
    }
    fn assert_unstarted(&self, expected: &VoiceCallActivationWork) {
        let outcome = self.outcome.lock().unwrap();
        let (error, work, payment) = outcome
            .refused
            .as_ref()
            .expect("exact never-entered outcome");
        assert_eq!(*error, VoiceEndpointError::Stopped);
        assert!(work.as_ref().unwrap().is_same(expected));
        assert!(
            payment.is_none(),
            "accepted Work still owns the original payment"
        );
        assert!(!outcome.uncertain);
    }
}
struct AcceptancePause {
    entered: Option<oneshot::Sender<()>>,
    resume: thread_channel::Receiver<()>,
}
struct Publisher {
    record: Arc<Record>,
    pause: Option<AcceptancePause>,
    settled: bool,
}
impl Publisher {
    fn new(record: &Arc<Record>) -> Box<Self> {
        Box::new(Self {
            record: record.clone(),
            pause: None,
            settled: false,
        })
    }
    fn paused(record: &Arc<Record>) -> (Box<Self>, oneshot::Receiver<()>, ResumeOnDrop) {
        let (entered, observed) = oneshot::channel();
        let (resume, resumed) = thread_channel::channel();
        (
            Box::new(Self {
                record: record.clone(),
                pause: Some(AcceptancePause {
                    entered: Some(entered),
                    resume: resumed,
                }),
                settled: false,
            }),
            observed,
            ResumeOnDrop(Some(resume)),
        )
    }
}
impl VoiceCallActivationPublisher for Publisher {
    fn accepted(&mut self, work: VoiceCallActivationWork) -> Result<(), VoiceEndpointError> {
        // No retirement has started in either paused fixture yet. Probe BEFORE
        // pausing, so moving accepted() under the gate fails instead of deadlocking
        // the Actor's close behind this callback's resume barrier.
        if self.pause.is_some() {
            self.record.assert_outside_send_fence();
        }
        {
            let mut outcome = self.record.outcome.lock().unwrap();
            assert!(outcome.work.is_none());
            outcome.work = Some(work);
        }
        // This real callback is OUTSIDE the production send fence. No test hook
        // or arbitrary callback is added under that fence to force the ordering.
        if let Some(mut pause) = self.pause.take() {
            pause.entered.take().unwrap().send(()).unwrap();
            pause.resume.recv().unwrap();
        }
        Ok(())
    }
    fn completed(
        mut self: Box<Self>,
        result: VoiceCallActivationResult,
        work: VoiceCallActivationWork,
    ) {
        {
            let mut outcome = self.record.outcome.lock().unwrap();
            assert!(outcome.work.as_ref().unwrap().is_same(&work));
            outcome.result = Some(result);
        }
        self.settled = true;
        self.record.changed.notify_waiters();
    }
    fn refused(
        mut self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceCallActivationWork>,
        payment: Option<ActivationRpcPermit>,
    ) {
        self.record.assert_outside_send_fence();
        self.record.outcome.lock().unwrap().refused = Some((error, work, payment));
        self.settled = true;
        self.record.changed.notify_waiters();
    }
}
impl Drop for Publisher {
    fn drop(&mut self) {
        if !self.settled {
            self.record.outcome.lock().unwrap().uncertain = true;
            self.record.changed.notify_waiters();
        }
    }
}
async fn prepare(
    actor: &ServerActor,
    runtime: &catcoms_sync::voice_v2::runtime::Runtime,
    old: &Arc<CallGrant>,
) -> PreparedVoiceCall {
    bounded(
        "paid preparation",
        actor.prepare_admitted_voice_call(
            old.scope().room,
            ActivationRpcPermit::reserve(&runtime.resources(), old.scope()).unwrap(),
        ),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn accepted_before_fence_then_actual_receiver_drop_cannot_late_send_with_ordinary_sender_alive(
) {
    let (actor, mut events, task, runtime, old) = bounded("Actor fixture", start()).await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let pair = prepare(&actor, &runtime, &old).await;
    let original = pair.observer();
    let control = pair.control();
    let paused_actor = bounded("park actual Actor", park(&actor)).await;
    let record = Record::new(&original, &control);
    let (publisher, accepted, resume) = Publisher::paused(&record);
    let queued_control = control.clone();
    let queued_original = original.clone();
    let grant = pair.grant().clone();
    let payment = original.reserve_rpc().unwrap();
    let completed = producer(move || {
        queued_control.try_queue_voice_call_selection(grant, queued_original, payment, publisher)
    });
    bounded("producer acceptance", accepted).await.unwrap();
    let work = record.work();
    assert!(!work.is_retired());

    task.abort();
    assert!(bounded("aborted Actor receiver destruction", task)
        .await
        .unwrap_err()
        .is_cancelled());
    assert!(
        actor.cmd_tx.tx.is_closed(),
        "ordinary Actor still owns its Sender"
    );
    assert!(
        !work.is_retired(),
        "the producer still physically owns its accepted arguments"
    );
    assert!(record.outcome.lock().unwrap().refused.is_none());
    assert!(original.root_returned().is_none());
    resume.release();
    assert_eq!(
        bounded("rejected late producer", completed)
            .await
            .unwrap()
            .unwrap_err(),
        VoiceEndpointError::Stopped
    );
    assert!(
        work.is_retired(),
        "queue-call return must have destroyed the actual refused arguments"
    );
    record.assert_unstarted(&work);
    assert!(
        control.upgrade().is_none(),
        "a real weak control cannot keep the owner/channel body alive"
    );
    assert!(
        original.root_returned().is_none(),
        "failed control upgrade is not a Root-return certificate"
    );
    original.close();
    assert!(original
        .seal_unclaimed_residence()
        .unwrap()
        .matches(&original));
    drop(paused_actor);
    drop(actor);
    bounded("event drain after abort", drain).await.unwrap();
}

#[tokio::test]
async fn successful_send_before_receiver_drop_retires_unread_work_with_ordinary_sender_alive() {
    let (actor, mut events, task, runtime, old) = bounded("Actor fixture", start()).await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let pair = prepare(&actor, &runtime, &old).await;
    let original = pair.observer();
    let control = pair.control();
    let paused_actor = bounded("park actual Actor", park(&actor)).await;
    let record = Record::new(&original, &control);
    // Returning Ok proves the production gate admitted and published this envelope
    // before the later abort. The parked Actor has not entered its Root action.
    let work = control
        .try_queue_voice_call_selection(
            pair.grant().clone(),
            original.clone(),
            original.reserve_rpc().unwrap(),
            Publisher::new(&record),
        )
        .unwrap();
    assert!(!work.is_retired());
    task.abort();
    assert!(bounded("sent envelope receiver destruction", task)
        .await
        .unwrap_err()
        .is_cancelled());
    assert!(actor.cmd_tx.tx.is_closed());
    assert!(
        work.is_retired(),
        "actual Actor destruction drains the sent unread envelope"
    );
    record.assert_unstarted(&work);
    assert!(control.upgrade().is_none());
    assert!(original.root_returned().is_none());
    original.close();
    assert!(original.seal_unclaimed_residence().is_some());
    drop(paused_actor);
    drop(actor);
    bounded("sent envelope event drain", drain).await.unwrap();
}

#[tokio::test]
async fn unpolled_actor_future_drops_actual_inbox_while_ordinary_sender_is_retained() {
    // The default current-thread executor cannot poll this new Actor between spawn
    // and abort: there is no await. No voice owner can have been created yet.
    let (actor, events, task) = crate::actor::spawn(found(819));
    task.abort();
    assert!(bounded("unpolled Actor destruction", task)
        .await
        .unwrap_err()
        .is_cancelled());
    assert!(actor.cmd_tx.tx.is_closed());
    drop(events);
    drop(actor);
}

#[tokio::test]
async fn blocked_closed_event_does_not_retain_control_or_an_accepted_late_clear() {
    let (actor, mut events, task, runtime, old) = bounded("Actor fixture", start()).await;
    let pair = prepare(&actor, &runtime, &old).await;
    let original = pair.observer();
    let control = pair.control();
    let selected = Record::new(&original, &control);
    let selection = control
        .try_queue_voice_call_selection(
            pair.grant().clone(),
            original.clone(),
            original.reserve_rpc().unwrap(),
            Publisher::new(&selected),
        )
        .unwrap();
    bounded("initial Root selection", selected.ready()).await;
    bounded("initial selection arguments", selection.wait_retired()).await;
    assert!(matches!(
        selected.outcome.lock().unwrap().result,
        Some(VoiceCallActivationResult::Selected(Ok(_)))
    ));
    assert!(original.root_returned().is_none());
    let paused_actor = bounded("park actual Actor", park(&actor)).await;
    while events.try_recv().is_ok() {}
    {
        let sender = actor.test_events.upgrade().unwrap();
        while sender
            .try_send(TracedEvent {
                trace: Trace::NONE,
                event: AppEvent::SnapshotNeeded,
            })
            .is_ok()
        {}
        assert_eq!(sender.capacity(), 0);
    }

    let record = Record::new(&original, &control);
    let (publisher, accepted, resume) = Publisher::paused(&record);
    let queued_control = control.clone();
    let queued_original = original.clone();
    let payment = original.reserve_rpc().unwrap();
    let completed = producer(move || {
        queued_control.try_queue_voice_call_clear(queued_original, payment, publisher)
    });
    bounded("clear acceptance", accepted).await.unwrap();
    let work = record.work();
    let (stopped, confirmed) = oneshot::channel();
    bounded(
        "queue terminal stop",
        actor.cmd_tx.send(AppCommand::StopAndWait { stopped }),
    )
    .await
    .unwrap();
    drop(paused_actor);
    bounded("terminal Root acknowledgement", confirmed)
        .await
        .unwrap();
    // On this current-thread executor, StopAndWait reaches its blocked Closed
    // event await before the acknowledged caller can resume.
    assert!(!task.is_finished());
    assert!(actor.cmd_tx.tx.is_closed());
    assert!(original.root_returned().unwrap().matches(&original));
    assert!(
        !work.is_retired(),
        "Root return is separate from the producer's actual arguments"
    );
    resume.release();
    assert_eq!(
        bounded("late clear return", completed)
            .await
            .unwrap()
            .unwrap_err(),
        VoiceEndpointError::Stopped
    );
    assert!(
        work.is_retired(),
        "late clear arguments returned independently of the blocked event"
    );
    record.assert_unstarted(&work);
    assert!(control.upgrade().is_none());
    assert!(
        !task.is_finished(),
        "negative retirement does not depend on Closed event delivery"
    );
    bounded("blocked Closed delivery", async {
        while let Some(event) = events.recv().await {
            if matches!(event.event, AppEvent::Closed) {
                return;
            }
        }
        panic!("Closed event missing");
    })
    .await;
    bounded("Actor after Closed delivery", task).await.unwrap();
}

#[tokio::test]
async fn control_trace_reuses_original_owner_and_foreign_pin_cannot_replace_it() {
    let (actor, mut events, task, runtime, old) = bounded("Actor fixture", start()).await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let first = prepare(&actor, &runtime, &old).await;
    let traced_actor = actor.with_trace(731);
    let second = prepare(&traced_actor, &runtime, &old).await;
    let a = first.control();
    let b = second.control();
    assert!(a.is_same(&b));
    assert_eq!(a.trace, Trace::NONE);
    assert_eq!(b.trace, Trace(731));
    assert!(a.is_same(&a.with_trace(99)));
    assert_eq!(a.with_trace(99).trace, Trace(99));

    let (other, mut other_events, other_task, other_runtime, other_old) =
        bounded("foreign Actor fixture", start()).await;
    let other_drain = tokio::spawn(async move { while other_events.recv().await.is_some() {} });
    let foreign = prepare(&other, &other_runtime, &other_old).await;
    let foreign_original = foreign.observer();
    let foreign_record = Record::new(&foreign_original, &foreign.control());
    assert_eq!(
        a.try_queue_voice_call_clear(
            foreign_original.clone(),
            foreign_original.reserve_rpc().unwrap(),
            Publisher::new(&foreign_record)
        )
        .unwrap_err(),
        VoiceEndpointError::Protocol(VoiceError::Scope)
    );
    {
        let outcome = foreign_record.outcome.lock().unwrap();
        let (_, work, payment) = outcome.refused.as_ref().unwrap();
        assert!(work.is_none());
        assert!(payment.as_ref().unwrap().matches(&foreign_original));
    }
    // Exercise the actual cache-mismatch branch with the SAME original owner,
    // not a fabricated replacement. Remove the borrowed cache alias before Drop
    // so this test does not independently retire the live Actor's shared gate.
    let mut cache = ActorControl {
        seed: None,
        owner: a.upgrade(),
    };
    let (foreign_pair, foreign_control) = foreign.into_parts();
    assert!(matches!(
        cache.wrap(foreign_pair, Trace(811)),
        Err(VoiceError::Scope)
    ));
    let same = cache.owner.as_ref().unwrap();
    assert!(Weak::ptr_eq(&a.owner, &Arc::downgrade(&same.owner)));
    assert!(same.owner.pin.is_same(&a.pin));
    drop(cache.owner.take());
    drop(cache);
    let third = prepare(&actor, &runtime, &old).await;
    assert!(
        third.control().is_same(&a),
        "mismatch never replaces or closes the original owner"
    );
    assert!(*a.upgrade().unwrap().owner.send.lock().unwrap());
    drop(foreign_control);
    bounded("original shutdown", actor.shutdown()).await;
    bounded("foreign shutdown", other.shutdown()).await;
    bounded("original Actor return", task).await.unwrap();
    bounded("foreign Actor return", other_task).await.unwrap();
    bounded("original event drain", drain).await.unwrap();
    bounded("foreign event drain", other_drain).await.unwrap();
}

#[tokio::test]
async fn original_control_payment_survives_owner_body_and_last_weak_tail() {
    let (actor, mut events, task, runtime, old) = bounded("Actor fixture", start()).await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let pair = prepare(&actor, &runtime, &old).await;
    let original = pair.observer();
    let control = pair.control();
    original.close();
    drop(pair);
    bounded("weak-tail Actor shutdown", actor.shutdown()).await;
    bounded("weak-tail Actor return", task).await.unwrap();
    bounded("weak-tail event drain", drain).await.unwrap();
    assert!(control.upgrade().is_none());
    assert!(
        actor.cmd_tx.tx.is_closed(),
        "an unrelated ordinary Sender still exists"
    );
    assert!(
        original.root_returned().is_none(),
        "owner death is not a fabricated residence certificate"
    );
    let before = runtime.resources().counts().0;
    let last = control.with_trace(5);
    drop(control);
    assert_eq!(
        runtime.resources().counts().0,
        before,
        "a real weak tail still owns the original payer"
    );
    drop(last);
    assert_eq!(
        runtime.resources().counts().0,
        before - VOICE_ACTIVATION_CONTROL_BYTES
    );
}

#[tokio::test]
async fn same_group_and_resources_do_not_adopt_a_different_actual_root_pin() {
    // A genuine restore retains the group/device while creating a distinct Root.
    // Both use the SAME Runtime, so resources/group rejection cannot conceal a
    // missing actual-instance check. Installing B may revoke A's positive Actor;
    // exact negative routing on A must still reject B before Work is allocated.
    let mut first_server = found(821);
    let snapshot = first_server.snapshot().unwrap();
    let second_server = Server::restore(
        &snapshot,
        catcoms_rt::Hub::new().join(catcoms_rt::PeerId::from_u64(822)),
        rand_chacha::ChaCha20Rng::seed_from_u64(822),
        Box::new(catcoms_rt::ManualClock::new(1000)),
        "alice",
    )
    .unwrap();
    let (actor, mut events, task) = crate::actor::spawn(first_server);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let runtime =
        catcoms_sync::voice_v2::runtime::Runtime::new(Arc::new(catcoms_rt::ManualClock::new(1000)));
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    bounded(
        "first shared-Runtime registration",
        actor.prepare_voice_actor(runtime.clone(), ticket),
    )
    .await
    .unwrap()
    .register()
    .unwrap();
    let old = Arc::new(
        bounded(
            "first shared-Runtime grant",
            actor.voice_call_grant(crate::channel_id("general")),
        )
        .await
        .unwrap(),
    );
    let first = prepare(&actor, &runtime, &old).await;
    let control = first.control();
    let first_original = first.observer();

    let (other, mut other_events, other_task) = crate::actor::spawn(second_server);
    let other_drain = tokio::spawn(async move { while other_events.recv().await.is_some() {} });
    bounded(
        "second shared-Runtime registration",
        other.prepare_voice_actor(runtime.clone(), runtime.session_ticket()),
    )
    .await
    .unwrap()
    .register()
    .unwrap();
    let other_old = Arc::new(
        bounded(
            "second shared-Runtime grant",
            other.voice_call_grant(crate::channel_id("general")),
        )
        .await
        .unwrap(),
    );
    let foreign = prepare(&other, &runtime, &other_old).await;
    let original = foreign.observer();
    assert_eq!(first_original.scope().group, original.scope().group);
    assert!(!control.pin.is_same(&foreign.control().pin));
    let record = Record::new(&original, &foreign.control());
    let payment = original.reserve_rpc().unwrap();
    let before = runtime.resources().counts();
    assert_eq!(
        control
            .try_queue_voice_call_clear(original.clone(), payment, Publisher::new(&record))
            .unwrap_err(),
        VoiceEndpointError::Protocol(VoiceError::Scope)
    );
    {
        let outcome = record.outcome.lock().unwrap();
        let (_, work, payment) = outcome.refused.as_ref().unwrap();
        assert!(
            work.is_none(),
            "foreign Root refuses before queue/Work admission"
        );
        assert!(payment.as_ref().unwrap().matches(&original));
        assert_eq!(
            runtime.resources().counts(),
            before,
            "the same affine payer is retained"
        );
    }

    let mut cache = ActorControl {
        seed: None,
        owner: control.upgrade(),
    };
    let (foreign_pair, foreign_control) = foreign.into_parts();
    assert!(matches!(
        cache.wrap(foreign_pair, Trace(823)),
        Err(VoiceError::Scope)
    ));
    assert!(Weak::ptr_eq(
        &control.owner,
        &Arc::downgrade(&cache.owner.as_ref().unwrap().owner)
    ));
    drop(cache.owner.take());
    drop(cache);
    drop(foreign_control);
    bounded("first Root shutdown", actor.shutdown()).await;
    bounded("second Root shutdown", other.shutdown()).await;
    bounded("first Root return", task).await.unwrap();
    bounded("second Root return", other_task).await.unwrap();
    bounded("first Root event drain", drain).await.unwrap();
    bounded("second Root event drain", other_drain)
        .await
        .unwrap();
}

#[test]
fn independently_paid_control_owner_and_weak_graph_fit_original_allowance() {
    let bytes = control_layout_bytes();
    assert!(
        bytes <= VOICE_ACTIVATION_CONTROL_BYTES,
        "control owner/weak graph {bytes}/{VOICE_ACTIVATION_CONTROL_BYTES}"
    );
}
