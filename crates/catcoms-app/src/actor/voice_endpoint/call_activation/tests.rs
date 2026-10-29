use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Slot {
    work: Option<VoiceCallActivationWork>,
    result: Option<VoiceCallActivationResult>,
    refused: Option<(
        VoiceEndpointError,
        Option<VoiceCallActivationWork>,
        Option<ActivationRpcPermit>,
    )>,
    uncertain: bool,
}
struct Sink {
    slot: Mutex<Slot>,
    changed: tokio::sync::Notify,
    _credit: catcoms_sync::voice_v2::resources::Credit,
}
impl Sink {
    fn new(original: &ActivationObserver) -> Arc<Self> {
        let credit = original.reserve_retained(4096).unwrap();
        Arc::new(Self {
            slot: Mutex::new(Slot::default()),
            changed: tokio::sync::Notify::new(),
            _credit: credit,
        })
    }
    async fn ready(&self) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let slot = self.slot.lock().unwrap();
                if slot.result.is_some() || slot.refused.is_some() || slot.uncertain {
                    return;
                }
            }
            notified.await;
        }
    }
    fn take(&self) -> VoiceCallActivationResult {
        self.slot.lock().unwrap().result.take().unwrap()
    }
}
struct Publisher {
    sink: Arc<Sink>,
    settled: bool,
    panic_after_entry: bool,
}
impl Publisher {
    fn new(sink: &Arc<Sink>) -> Box<Self> {
        Box::new(Self {
            sink: sink.clone(),
            settled: false,
            panic_after_entry: false,
        })
    }
}
impl Drop for Publisher {
    fn drop(&mut self) {
        if !self.settled {
            self.sink.slot.lock().unwrap().uncertain = true;
            self.sink.changed.notify_waiters();
        }
    }
}
impl VoiceCallActivationPublisher for Publisher {
    fn accepted(&mut self, work: VoiceCallActivationWork) -> Result<(), VoiceEndpointError> {
        let mut slot = self.sink.slot.lock().unwrap();
        assert!(slot.work.is_none());
        slot.work = Some(work);
        Ok(())
    }
    fn completed(
        mut self: Box<Self>,
        result: VoiceCallActivationResult,
        work: VoiceCallActivationWork,
    ) {
        if self.panic_after_entry {
            panic!("controlled entered activation callback unwind");
        }
        {
            let mut slot = self.sink.slot.lock().unwrap();
            assert!(slot.work.as_ref().unwrap().is_same(&work));
            slot.result = Some(result);
        }
        self.settled = true;
        self.sink.changed.notify_waiters();
    }
    fn refused(
        mut self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceCallActivationWork>,
        payment: Option<ActivationRpcPermit>,
    ) {
        self.sink.slot.lock().unwrap().refused = Some((error, work, payment));
        self.settled = true;
        self.sink.changed.notify_waiters();
    }
}

#[tokio::test]
async fn actual_preparation_selection_lost_reply_and_negative_clear_keep_original_owners() {
    let (actor, mut events, task, runtime, old) = super::super::tests::start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let pair = actor
        .prepare_admitted_voice_call(
            old.scope().room,
            ActivationRpcPermit::reserve(&runtime.resources(), old.scope()).unwrap(),
        )
        .await
        .unwrap();
    let original = pair.observer();
    let sink = Sink::new(&original);
    let work = pair
        .control()
        .try_queue_voice_call_selection(
            pair.grant().clone(),
            original.clone(),
            original.reserve_rpc().unwrap(),
            Publisher::new(&sink),
        )
        .unwrap();
    sink.ready().await;
    work.wait_retired().await;
    let VoiceCallActivationResult::Selected(Ok(selected)) = sink.take() else {
        panic!("actual Root selection");
    };
    assert!(!original.is_active());
    assert!(actor
        .start_voice_endpoint(old.clone(), [8; 32], super::super::tests::point())
        .await
        .is_err());
    // App-level boundary only: actual UI-guard admission is tested in native.
    selected.commit_native(pair.grant()).unwrap();
    let retry = Sink::new(&original);
    let retried = pair
        .control()
        .try_queue_voice_call_selection(
            pair.grant().clone(),
            original.clone(),
            original.reserve_rpc().unwrap(),
            Publisher::new(&retry),
        )
        .unwrap();
    retry.ready().await;
    retried.wait_retired().await;
    let VoiceCallActivationResult::Selected(Ok(selected)) = retry.take() else {
        panic!("same exact recovery");
    };
    assert!(selected.original().is_same(&original));
    drop(selected);
    let control = pair.control();
    original.close();
    drop(pair); // no positive Grant/Room kept while awaiting actual Root residence return
    let clear = Sink::new(&original);
    let cleared = control
        .try_queue_voice_call_clear(
            original.clone(),
            original.reserve_rpc().unwrap(),
            Publisher::new(&clear),
        )
        .unwrap();
    clear.ready().await;
    cleared.wait_retired().await;
    assert!(matches!(
        clear.take(),
        VoiceCallActivationResult::Cleared(Ok(()))
    ));
    assert!(original.root_returned().unwrap().matches(&original));
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn full_mailbox_returns_same_affine_unstarted_payment_and_never_selects_root() {
    let (actor, mut events, task, runtime, old) = super::super::tests::start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let pair = actor
        .prepare_admitted_voice_call(
            old.scope().room,
            ActivationRpcPermit::reserve(&runtime.resources(), old.scope()).unwrap(),
        )
        .await
        .unwrap();
    let original = pair.observer();
    let paused = super::super::tests::park(&actor).await;
    let mut permits = Vec::new();
    while let Ok(permit) = actor.cmd_tx.tx.try_reserve() {
        permits.push(permit);
    }
    let sink = Sink::new(&original);
    let before = runtime.resources().counts();
    let payment = original.reserve_rpc().unwrap();
    assert_eq!(
        pair.control()
            .try_queue_voice_call_selection(
                pair.grant().clone(),
                original.clone(),
                payment,
                Publisher::new(&sink)
            )
            .unwrap_err(),
        VoiceEndpointError::Protocol(VoiceError::Capacity)
    );
    let (_, work, payment) = sink.slot.lock().unwrap().refused.take().unwrap();
    assert!(work.is_none());
    assert!(payment.as_ref().unwrap().matches(&original));
    assert_eq!(
        runtime.resources().counts().0,
        before.0 + ACTIVATION_RPC_BYTES
    );
    original.close();
    assert!(original
        .seal_unclaimed_residence()
        .unwrap()
        .matches(&original));
    drop(payment);
    drop((permits, paused));
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn closed_original_queued_before_apply_cannot_select_after_revocation() {
    let (actor, mut events, task, runtime, old) = super::super::tests::start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let pair = actor
        .prepare_admitted_voice_call(
            old.scope().room,
            ActivationRpcPermit::reserve(&runtime.resources(), old.scope()).unwrap(),
        )
        .await
        .unwrap();
    let original = pair.observer();
    let paused = super::super::tests::park(&actor).await;
    let sink = Sink::new(&original);
    let work = pair
        .control()
        .try_queue_voice_call_selection(
            pair.grant().clone(),
            original.clone(),
            original.reserve_rpc().unwrap(),
            Publisher::new(&sink),
        )
        .unwrap();
    original.close();
    assert!(original.seal_unclaimed_residence().is_some());
    assert!(!work.is_retired(), "original command arguments still exist");
    drop(paused);
    sink.ready().await;
    work.wait_retired().await;
    assert!(matches!(
        sink.take(),
        VoiceCallActivationResult::Selected(Err(VoiceError::Unauthorized))
    ));
    assert!(!original.is_active());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn unread_queued_command_reports_unstarted_but_keeps_work_until_actual_argument_drop() {
    let (actor, mut events, task, runtime, old) = super::super::tests::start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let pair = actor
        .prepare_admitted_voice_call(
            old.scope().room,
            ActivationRpcPermit::reserve(&runtime.resources(), old.scope()).unwrap(),
        )
        .await
        .unwrap();
    let original = pair.observer();
    let paused = super::super::tests::park(&actor).await;
    let sink = Sink::new(&original);
    let work = pair
        .control()
        .try_queue_voice_call_selection(
            pair.grant().clone(),
            original.clone(),
            original.reserve_rpc().unwrap(),
            Publisher::new(&sink),
        )
        .unwrap();
    assert!(
        !work.is_retired(),
        "the actual unread envelope still owns its arguments"
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    sink.ready().await;
    work.wait_retired().await;
    let (error, returned, payment) = sink.slot.lock().unwrap().refused.take().unwrap();
    assert_eq!(error, VoiceEndpointError::Stopped);
    assert!(returned.unwrap().is_same(&work));
    assert!(
        payment.is_none(),
        "accepted Work keeps the original payer until actual destruction"
    );
    assert!(!sink.slot.lock().unwrap().uncertain);
    original.close();
    assert!(original
        .seal_unclaimed_residence()
        .unwrap()
        .matches(&original));
    assert!(
        original.root_returned().is_none(),
        "never installed is not an actual residence return"
    );
    drop(paused);
    drop(actor);
    drain.await.unwrap();
}

#[tokio::test]
async fn callback_unwind_after_actual_root_selection_is_uncertain_not_unstarted() {
    let (actor, mut events, task, runtime, old) = super::super::tests::start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let pair = actor
        .prepare_admitted_voice_call(
            old.scope().room,
            ActivationRpcPermit::reserve(&runtime.resources(), old.scope()).unwrap(),
        )
        .await
        .unwrap();
    let original = pair.observer();
    let sink = Sink::new(&original);
    let work = pair
        .control()
        .try_queue_voice_call_selection(
            pair.grant().clone(),
            original.clone(),
            original.reserve_rpc().unwrap(),
            Box::new(Publisher {
                sink: sink.clone(),
                settled: false,
                panic_after_entry: true,
            }),
        )
        .unwrap();
    sink.ready().await;
    assert!(task.await.unwrap_err().is_panic());
    work.wait_retired().await;
    {
        let slot = sink.slot.lock().unwrap();
        assert!(slot.uncertain);
        assert!(
            slot.refused.is_none(),
            "Root was entered; no unstarted certificate may be invented"
        );
    }
    assert!(
        original.seal_unclaimed_residence().is_none(),
        "the original acquisition right was used before the callback unwound"
    );
    assert!(
        original.root_returned().unwrap().matches(&original),
        "actual Root destruction, not callback failure, returns the residence"
    );
    assert!(original.is_closed() && !original.is_active());
    drop(actor);
    drain.await.unwrap();
}

#[test]
fn actual_activation_actor_graph_fits_its_separate_original_payment() {
    let bytes = voice_call_activation_rpc_bytes();
    eprintln!("activation Actor graph {bytes}/{ACTIVATION_RPC_BYTES}");
    assert!(bytes <= ACTIVATION_RPC_BYTES);
}
