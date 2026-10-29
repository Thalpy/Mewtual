//! Actual Root publication and argument ownership, without browser SDK/audio claims.
use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Slot {
    work: Option<VoiceAdmittedPresenceWork>,
    result: Option<VoiceAdmittedPresenceResult>,
    refusal: Option<(VoiceEndpointError, Option<ActivationRpcPermit>)>,
    uncertain: bool,
}
struct Sink {
    slot: Mutex<Slot>,
    changed: tokio::sync::Notify,
    _credit: catcoms_sync::voice_v2::resources::Credit,
}
impl Sink {
    fn new(original: &ActivationObserver) -> Arc<Self> {
        Arc::new(Self {
            slot: Mutex::new(Slot::default()),
            changed: tokio::sync::Notify::new(),
            _credit: original.reserve_retained(4096).unwrap(),
        })
    }
    async fn ready(&self) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let slot = self.slot.lock().unwrap();
                if slot.result.is_some() || slot.refusal.is_some() || slot.uncertain {
                    return;
                }
            }
            notified.await;
        }
    }
}
struct Publisher {
    sink: Arc<Sink>,
    finished: bool,
    panic_after_entry: bool,
    pause: Option<(
        oneshot::Sender<()>,
        std::sync::mpsc::Receiver<()>,
        VoiceActivationControl,
    )>,
}
impl Publisher {
    fn new(sink: &Arc<Sink>) -> Box<Self> {
        Box::new(Self {
            sink: sink.clone(),
            finished: false,
            panic_after_entry: false,
            pause: None,
        })
    }
}
impl VoiceAdmittedPresencePublisher for Publisher {
    fn accepted(&mut self, work: VoiceAdmittedPresenceWork) -> Result<(), VoiceEndpointError> {
        let mut slot = self.sink.slot.lock().unwrap();
        assert!(slot.work.is_none());
        slot.work = Some(work);
        drop(slot);
        if let Some((entered, resume, control)) = self.pause.take() {
            let owner = control.upgrade().unwrap();
            assert!(
                owner.owner.send.try_lock().is_ok(),
                "accepted callback ran under send fence"
            );
            entered.send(()).unwrap();
            resume.recv().unwrap();
        }
        Ok(())
    }
    fn completed(
        mut self: Box<Self>,
        result: VoiceAdmittedPresenceResult,
        work: VoiceAdmittedPresenceWork,
    ) {
        assert!(
            !self.panic_after_entry,
            "controlled entered key-publication callback unwind"
        );
        {
            let mut slot = self.sink.slot.lock().unwrap();
            assert!(slot.work.as_ref().unwrap().is_same(&work));
            slot.result = Some(result);
        }
        self.finished = true;
        self.sink.changed.notify_waiters();
    }
    fn refused(
        mut self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceAdmittedPresenceWork>,
        payment: Option<ActivationRpcPermit>,
    ) {
        {
            let mut slot = self.sink.slot.lock().unwrap();
            match (&slot.work, work.as_ref()) {
                (Some(a), Some(b)) => assert!(a.is_same(b)),
                (None, None) => {}
                _ => panic!("publication Work identity changed"),
            }
            slot.refusal = Some((error, payment));
        }
        self.finished = true;
        self.sink.changed.notify_waiters();
    }
}
impl Drop for Publisher {
    fn drop(&mut self) {
        if !self.finished {
            self.sink.slot.lock().unwrap().uncertain = true;
            self.sink.changed.notify_waiters();
        }
    }
}

struct Selection(Option<oneshot::Sender<VoiceCallActivationResult>>);
impl VoiceCallActivationPublisher for Selection {
    fn accepted(&mut self, _: VoiceCallActivationWork) -> Result<(), VoiceEndpointError> {
        Ok(())
    }
    fn completed(
        mut self: Box<Self>,
        result: VoiceCallActivationResult,
        _: VoiceCallActivationWork,
    ) {
        let _ = self.0.take().unwrap().send(result);
    }
    fn refused(
        self: Box<Self>,
        error: VoiceEndpointError,
        _: Option<VoiceCallActivationWork>,
        _: Option<ActivationRpcPermit>,
    ) {
        panic!("actual admitted selection refused: {error}");
    }
}
struct Call {
    actor: ServerActor,
    pair: PreparedVoiceCall,
    // Session custody is affine too: dropping Runtime closes the actor's epoch
    // before a later publication can be admitted. Keep the actual fixture owner.
    _runtime: Arc<catcoms_sync::voice_v2::runtime::Runtime>,
    task: tokio::task::JoinHandle<()>,
    drain: tokio::task::JoinHandle<()>,
}
impl Call {
    async fn new() -> Self {
        let (actor, mut events, task, runtime, old) = super::super::super::tests::start().await;
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        let pair = actor
            .prepare_admitted_voice_call(
                old.scope().room,
                ActivationRpcPermit::reserve(&runtime.resources(), old.scope()).unwrap(),
            )
            .await
            .unwrap();
        let original = pair.observer();
        let (send, receive) = oneshot::channel();
        let work = pair
            .control()
            .try_queue_voice_call_selection(
                pair.grant().clone(),
                original.clone(),
                original.reserve_rpc().unwrap(),
                Box::new(Selection(Some(send))),
            )
            .unwrap();
        let VoiceCallActivationResult::Selected(Ok(selected)) = receive.await.unwrap() else {
            panic!("actual selected Root activation");
        };
        work.wait_retired().await;
        selected.commit_native(pair.grant()).unwrap(); // trusted native boundary fixture only
        Self {
            actor,
            pair,
            _runtime: runtime,
            task,
            drain,
        }
    }
    fn point() -> [u8; 65] {
        super::super::super::tests::point()
    }
    fn queue(&self, sink: &Arc<Sink>, join: Nonce, point: [u8; 65]) -> VoiceAdmittedPresenceWork {
        let original = self.pair.observer();
        self.pair
            .control()
            .try_queue_voice_admitted_presence(
                self.pair.grant().clone(),
                original.clone(),
                join,
                point,
                original.reserve_rpc().unwrap(),
                Publisher::new(sink),
            )
            .unwrap()
    }
    async fn close(self) {
        self.actor.shutdown().await;
        self.task.await.unwrap();
        self.drain.await.unwrap();
    }
}

#[test]
fn actual_key_receipt_and_both_command_graphs_fit_original_actor_payments() {
    let bytes = voice_admitted_presence_rpc_bytes();
    eprintln!("admitted presence Actor graph {bytes}/{ACTIVATION_RPC_BYTES}");
    assert!(bytes <= ACTIVATION_RPC_BYTES);
    assert!(voice_call_activation_rpc_bytes() <= ACTIVATION_RPC_BYTES);
}

#[tokio::test]
async fn actual_publication_receipt_and_lost_reply_recover_same_original_without_legacy_access() {
    let call = Call::new().await;
    let original = call.pair.observer();
    for _ in 0..2 {
        let sink = Sink::new(&original);
        let work = call.queue(&sink, [31; 32], Call::point());
        sink.ready().await;
        work.wait_retired().await;
        let receipt = sink.slot.lock().unwrap().result.take().unwrap().unwrap();
        assert!(receipt.matches(&original, call.pair.grant(), [31; 32], Call::point()));
        drop(receipt); // loss of a result is not permission to change the original key
    }
    let sink = Sink::new(&original);
    let work = call.queue(&sink, [32; 32], Call::point());
    sink.ready().await;
    work.wait_retired().await;
    assert!(matches!(
        sink.slot.lock().unwrap().result.take().unwrap(),
        Err(VoiceError::RetryMismatch)
    ));
    assert!(call
        .actor
        .start_voice_endpoint(call.pair.grant().clone(), [31; 32], Call::point())
        .await
        .is_err());
    assert!(original.is_active());
    call.close().await;
}

#[tokio::test]
async fn closed_original_refuses_before_work_allocation_and_returns_same_affine_payment() {
    let call = Call::new().await;
    let original = call.pair.observer();
    let sink = Sink::new(&original);
    original.close();
    let result = call.pair.control().try_queue_voice_admitted_presence(
        call.pair.grant().clone(),
        original.clone(),
        [31; 32],
        Call::point(),
        original.reserve_rpc().unwrap(),
        Publisher::new(&sink),
    );
    assert!(matches!(
        result,
        Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
    ));
    {
        let mut slot = sink.slot.lock().unwrap();
        assert!(slot.work.is_none());
        let (_, payment) = slot.refusal.take().unwrap();
        assert!(payment.unwrap().matches(&original));
        assert!(!slot.uncertain);
    }
    call.close().await;
}

#[tokio::test]
async fn entered_callback_unwind_retains_ambiguity_until_actual_actor_and_arguments_return() {
    let call = Call::new().await;
    let original = call.pair.observer();
    let sink = Sink::new(&original);
    let work = call
        .pair
        .control()
        .try_queue_voice_admitted_presence(
            call.pair.grant().clone(),
            original.clone(),
            [31; 32],
            Call::point(),
            original.reserve_rpc().unwrap(),
            Box::new(Publisher {
                sink: sink.clone(),
                finished: false,
                panic_after_entry: true,
                pause: None,
            }),
        )
        .unwrap();
    sink.ready().await;
    assert!(call.task.await.unwrap_err().is_panic());
    work.wait_retired().await;
    {
        let slot = sink.slot.lock().unwrap();
        assert!(slot.uncertain);
        assert!(slot.refusal.is_none());
        assert!(slot.result.is_none());
    }
    assert!(original.is_closed());
    assert!(original.root_returned().unwrap().matches(&original));
    drop(call.actor);
    drop(call.pair);
    call.drain.await.unwrap();
}

struct ResumeOnDrop(Option<std::sync::mpsc::Sender<()>>);
impl Drop for ResumeOnDrop {
    fn drop(&mut self) {
        if let Some(resume) = self.0.take() {
            let _ = resume.send(());
        }
    }
}
async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    use catcoms_rt::Clock;
    tokio::select! {
        value = future => value,
        _ = catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(30)) => panic!("admitted publication fence timed out"),
    }
}

#[tokio::test]
async fn accepted_publication_cannot_send_after_actual_actor_retirement_with_sender_alive() {
    let call = Call::new().await;
    let original = call.pair.observer();
    let sink = Sink::new(&original);
    let control = call.pair.control();
    let grant = call.pair.grant().clone();
    let payment = original.reserve_rpc().unwrap();
    let (entered, accepted) = oneshot::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    let resume = ResumeOnDrop(Some(resume));
    let publisher = Box::new(Publisher {
        sink: sink.clone(),
        finished: false,
        panic_after_entry: false,
        pause: Some((entered, resumed, control.clone())),
    });
    let (sent, done) = oneshot::channel();
    drop(std::thread::spawn(move || {
        let _ = sent.send(control.try_queue_voice_admitted_presence(
            grant,
            original,
            [31; 32],
            Call::point(),
            payment,
            publisher,
        ));
    }));
    bounded(accepted).await.unwrap();
    let work = sink.slot.lock().unwrap().work.as_ref().unwrap().clone();
    assert!(!work.is_retired());
    // Retain the ordinary ServerActor and prepared control across actual future
    // destruction. Last-sender closure must not conceal a late unread send.
    call.task.abort();
    assert!(bounded(call.task).await.unwrap_err().is_cancelled());
    assert!(call.pair.observer().root_returned().is_some());
    assert!(
        !work.is_retired(),
        "paused producer still owns actual arguments"
    );
    drop(resume);
    assert!(matches!(
        bounded(done).await.unwrap(),
        Err(VoiceEndpointError::Stopped)
    ));
    bounded(work.wait_retired()).await;
    let slot = sink.slot.lock().unwrap();
    assert!(matches!(
        slot.refusal.as_ref(),
        Some((VoiceEndpointError::Stopped, None))
    ));
    assert!(slot.result.is_none() && !slot.uncertain);
    drop(slot);
    drop(call.actor);
    drop(call.pair);
    bounded(call.drain).await.unwrap();
}
