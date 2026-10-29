//! Actual actor admission/retirement, independent of the native TCP acceptance.
use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Slot {
    work: Option<VoiceAdmittedSupportWork>,
    result: Option<VoiceAdmittedSupportResult>,
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
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let slot = self.slot.lock().unwrap();
                if slot.result.is_some() || slot.refusal.is_some() || slot.uncertain {
                    return;
                }
            }
            changed.await;
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
impl Drop for Publisher {
    fn drop(&mut self) {
        if !self.finished {
            self.sink.slot.lock().unwrap().uncertain = true;
            self.sink.changed.notify_waiters();
        }
    }
}
impl VoiceAdmittedSupportPublisher for Publisher {
    fn accepted(&mut self, work: VoiceAdmittedSupportWork) -> Result<(), VoiceEndpointError> {
        let mut slot = self.sink.slot.lock().unwrap();
        assert!(slot.work.is_none());
        slot.work = Some(work);
        drop(slot);
        if let Some((entered, resume, control)) = self.pause.take() {
            let owner = control.upgrade().unwrap();
            assert!(
                owner.owner.send.try_lock().is_ok(),
                "callback must run outside send fence"
            );
            entered.send(()).unwrap();
            resume.recv().unwrap();
        }
        Ok(())
    }
    fn completed(
        mut self: Box<Self>,
        result: VoiceAdmittedSupportResult,
        work: VoiceAdmittedSupportWork,
    ) {
        assert!(
            !self.panic_after_entry,
            "controlled entered support callback unwind"
        );
        let mut slot = self.sink.slot.lock().unwrap();
        assert!(slot.work.as_ref().unwrap().is_same(&work));
        slot.result = Some(result);
        drop(slot);
        self.finished = true;
        self.sink.changed.notify_waiters();
    }
    fn refused(
        mut self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceAdmittedSupportWork>,
        payment: Option<ActivationRpcPermit>,
    ) {
        let mut slot = self.sink.slot.lock().unwrap();
        match (&slot.work, work.as_ref()) {
            (Some(a), Some(b)) => assert!(a.is_same(b)),
            (None, None) => {}
            _ => panic!("support Work identity changed"),
        }
        slot.refusal = Some((error, payment));
        drop(slot);
        self.finished = true;
        self.sink.changed.notify_waiters();
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
        panic!("actual selection refused: {error}");
    }
}
async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    use catcoms_rt::Clock;
    tokio::select! {
        value = future => value,
        _ = catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(30)) => panic!("admitted support actor bound"),
    }
}
struct Call {
    actor: ServerActor,
    pair: PreparedVoiceCall,
    runtime: Arc<catcoms_sync::voice_v2::runtime::Runtime>,
    task: tokio::task::JoinHandle<()>,
    drain: tokio::task::JoinHandle<()>,
}
impl Call {
    async fn new() -> Self {
        let (actor, mut events, task, runtime, old) = super::super::super::tests::start().await;
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        let pair = bounded(actor.prepare_admitted_voice_call(
            old.scope().room,
            ActivationRpcPermit::reserve(&runtime.resources(), old.scope()).unwrap(),
        ))
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
        let VoiceCallActivationResult::Selected(Ok(selected)) = bounded(receive).await.unwrap()
        else {
            panic!("actual Root selection");
        };
        bounded(work.wait_retired()).await;
        selected.commit_native(pair.grant()).unwrap();
        Self {
            actor,
            pair,
            runtime,
            task,
            drain,
        }
    }
    fn queue(&self, sink: &Arc<Sink>) -> VoiceAdmittedSupportWork {
        let original = self.pair.observer();
        self.pair
            .control()
            .try_queue_ensure_admitted_endpoint_support(
                self.pair.grant().clone(),
                original.clone(),
                DeviceId::from_bytes([91; 32]),
                original.reserve_rpc().unwrap(),
                Publisher::new(sink),
            )
            .unwrap()
    }
    async fn close(self) {
        self.actor.shutdown().await;
        bounded(self.task).await.unwrap();
        bounded(self.drain).await.unwrap();
    }
}
#[test]
fn admitted_support_complete_actor_box_and_results_fit_original_rpc_payment() {
    let bytes = voice_admitted_support_rpc_bytes();
    eprintln!("admitted support Actor graph {bytes}/{ACTIVATION_RPC_BYTES}");
    assert!(bytes <= ACTIVATION_RPC_BYTES);
    assert!(voice_admitted_presence_rpc_bytes() <= ACTIVATION_RPC_BYTES);
    assert!(voice_call_activation_rpc_bytes() <= ACTIVATION_RPC_BYTES);
}
#[tokio::test]
async fn admitted_support_foreign_control_payment_and_grant_refuse_before_work() {
    let a = Call::new().await;
    let b = Call::new().await;
    let original = a.pair.observer();
    for boundary in ["control", "payment", "grant"] {
        let sink = Sink::new(&original);
        let control = if boundary == "control" {
            b.pair.control()
        } else {
            a.pair.control()
        };
        let payment = if boundary == "payment" {
            b.pair.observer().reserve_rpc().unwrap()
        } else {
            original.reserve_rpc().unwrap()
        };
        let grant = if boundary == "grant" {
            b.pair.grant().clone()
        } else {
            a.pair.grant().clone()
        };
        let expected = if boundary == "grant" {
            VoiceError::Unauthorized
        } else {
            VoiceError::Scope
        };
        assert!(
            matches!(control.try_queue_ensure_admitted_endpoint_support(grant, original.clone(), DeviceId::from_bytes([91;32]), payment, Publisher::new(&sink)), Err(VoiceEndpointError::Protocol(error)) if error == expected)
        );
        let mut slot = sink.slot.lock().unwrap();
        assert!(slot.work.is_none() && slot.result.is_none() && !slot.uncertain);
        assert!(
            slot.refusal.take().unwrap().1.is_some(),
            "same pre-enqueue affine payment returns"
        );
    }
    a.close().await;
    b.close().await;
}
#[tokio::test]
async fn admitted_support_whole_root_refusal_and_actual_box_retirement_are_distinct() {
    let call = Call::new().await;
    let sink = Sink::new(&call.pair.observer());
    let work = call.queue(&sink);
    bounded(sink.ready()).await;
    bounded(work.wait_retired()).await;
    let slot = sink.slot.lock().unwrap();
    assert!(slot.work.as_ref().unwrap().is_same(&work));
    assert!(matches!(
        slot.result.as_ref(),
        Some(VoiceAdmittedSupportResult::EndpointQuery(Err(
            VoiceError::Unauthorized
        )))
    ));
    assert!(slot.refusal.is_none() && !slot.uncertain);
    drop(slot);
    call.close().await;
}
struct Resume(Option<std::sync::mpsc::Sender<()>>);
impl Drop for Resume {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
#[tokio::test]
async fn admitted_support_entered_callback_panic_is_unknown_until_actual_retirement() {
    let call = Call::new().await;
    let original = call.pair.observer();
    let sink = Sink::new(&original);
    let work = call
        .pair
        .control()
        .try_queue_ensure_admitted_endpoint_support(
            call.pair.grant().clone(),
            original.clone(),
            DeviceId::from_bytes([91; 32]),
            original.reserve_rpc().unwrap(),
            Box::new(Publisher {
                sink: sink.clone(),
                finished: false,
                panic_after_entry: true,
                pause: None,
            }),
        )
        .unwrap();
    bounded(sink.ready()).await;
    assert!(bounded(call.task).await.unwrap_err().is_panic());
    bounded(work.wait_retired()).await;
    let slot = sink.slot.lock().unwrap();
    assert!(slot.uncertain && slot.result.is_none() && slot.refusal.is_none());
    drop(slot);
    assert!(original.is_closed());
    drop(call.actor);
    drop(call.pair);
    drop(call.runtime);
    bounded(call.drain).await.unwrap();
}
#[tokio::test]
async fn admitted_support_paused_accept_cannot_send_after_actual_actor_abort() {
    let call = Call::new().await;
    let original = call.pair.observer();
    let sink = Sink::new(&original);
    let control = call.pair.control();
    let grant = call.pair.grant().clone();
    let payment = original.reserve_rpc().unwrap();
    let (entered, accepted) = oneshot::channel();
    let (resume, wait) = std::sync::mpsc::channel();
    let resume = Resume(Some(resume));
    let publisher = Box::new(Publisher {
        sink: sink.clone(),
        finished: false,
        panic_after_entry: false,
        pause: Some((entered, wait, control.clone())),
    });
    let (sent, done) = oneshot::channel();
    drop(std::thread::spawn(move || {
        let _ = sent.send(control.try_queue_ensure_admitted_endpoint_support(
            grant,
            original,
            DeviceId::from_bytes([91; 32]),
            payment,
            publisher,
        ));
    }));
    bounded(accepted).await.unwrap();
    let work = sink.slot.lock().unwrap().work.as_ref().unwrap().clone();
    assert!(!work.is_retired());
    call.task.abort();
    assert!(bounded(call.task).await.unwrap_err().is_cancelled());
    assert!(
        !work.is_retired(),
        "paused producer still owns actual boxed body"
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
    drop(call.runtime);
    bounded(call.drain).await.unwrap();
}
