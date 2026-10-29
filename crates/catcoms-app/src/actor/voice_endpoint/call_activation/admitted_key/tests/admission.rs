//! Actual sealed Actor -> Root neutral preparation, not native Slot/Ready proof.
use super::*;
use catcoms_rt::{Clock, SystemClock};
use catcoms_sync::voice_v2::admission::{AdmissionRpcPermit, ADMISSION_RPC_BYTES};
use catcoms_sync::voice_v2::admitted_admission::AdmittedAdmissionObserver;
use std::time::Duration;

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::select! {
        value=future=>value,
        _=SystemClock.sleep(Duration::from_secs(30))=>panic!("actual admitted Actor preparation exceeded its bound"),
    }
}
#[derive(Default)]
struct State {
    work: Option<VoiceAdmittedAdmissionWork>,
    result: Option<VoiceAdmittedAdmissionResult>,
    refusal: Option<(VoiceEndpointError, Option<AdmissionRpcPermit>)>,
    unknown: bool,
}
struct ResultCell {
    state: Mutex<State>,
    changed: tokio::sync::Notify,
    _credit: catcoms_sync::voice_v2::resources::Credit,
}
impl ResultCell {
    fn new(original: &ActivationObserver) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
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
                let state = self.state.lock().unwrap();
                if state.result.is_some() || state.refusal.is_some() || state.unknown {
                    return;
                }
            }
            notified.await;
        }
    }
}
struct AdmissionPublisher {
    cell: Arc<ResultCell>,
    finished: bool,
    panic_after_entry: bool,
    pause: Option<(
        oneshot::Sender<()>,
        std::sync::mpsc::Receiver<()>,
        VoiceActivationControl,
    )>,
}
impl AdmissionPublisher {
    fn new(cell: &Arc<ResultCell>) -> Box<Self> {
        Box::new(Self {
            cell: cell.clone(),
            finished: false,
            panic_after_entry: false,
            pause: None,
        })
    }
}
impl VoiceAdmittedAdmissionPublisher for AdmissionPublisher {
    fn accepted(&mut self, work: VoiceAdmittedAdmissionWork) -> Result<(), VoiceEndpointError> {
        let mut state = self.cell.state.lock().unwrap();
        assert!(state.work.is_none());
        state.work = Some(work);
        drop(state);
        if let Some((entered, resume, control)) = self.pause.take() {
            let owner = control.upgrade().unwrap();
            assert!(
                owner.owner.send.try_lock().is_ok(),
                "accepted callback held the send fence"
            );
            let _ = entered.send(());
            let _ = resume.recv();
        }
        Ok(())
    }
    fn completed(
        mut self: Box<Self>,
        result: VoiceAdmittedAdmissionResult,
        work: VoiceAdmittedAdmissionWork,
    ) {
        assert!(
            !self.panic_after_entry,
            "controlled entered admitted-preparation callback unwind"
        );
        {
            let mut state = self.cell.state.lock().unwrap();
            assert!(state.work.as_ref().unwrap().is_same(&work));
            state.result = Some(result);
        }
        self.finished = true;
        self.cell.changed.notify_waiters();
    }
    fn refused(
        mut self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceAdmittedAdmissionWork>,
        payment: Option<AdmissionRpcPermit>,
    ) {
        {
            let mut state = self.cell.state.lock().unwrap();
            match (&state.work, &work) {
                (Some(a), Some(b)) => assert!(a.is_same(b)),
                (None, None) => {}
                _ => panic!("original admitted Work changed"),
            };
            state.refusal = Some((error, payment));
        }
        self.finished = true;
        self.cell.changed.notify_waiters();
    }
}
impl Drop for AdmissionPublisher {
    fn drop(&mut self) {
        if !self.finished {
            self.cell.state.lock().unwrap().unknown = true;
            self.cell.changed.notify_waiters();
        }
    }
}
async fn published() -> Call {
    let call = Call::new().await;
    let sink = Sink::new(&call.pair.observer());
    let work = call.queue(&sink, [81; 32], Call::point());
    bounded(sink.ready()).await;
    bounded(work.wait_retired()).await;
    assert!(sink.slot.lock().unwrap().result.take().unwrap().is_ok());
    call
}
fn queue(
    call: &Call,
    cell: &Arc<ResultCell>,
    original: Option<AdmittedAdmissionObserver>,
) -> VoiceAdmittedAdmissionWork {
    call.pair
        .control()
        .try_queue_voice_admitted_admission(
            call.pair.grant().clone(),
            call.pair.observer(),
            original,
            1,
            AdmissionRpcPermit::reserve(&call._runtime.resources(), call.pair.grant().scope())
                .unwrap(),
            AdmissionPublisher::new(cell),
        )
        .unwrap()
}

#[test]
fn actual_admitted_preparation_and_existing_actor_arms_fit_original_four_kib_payment() {
    let native = std::mem::size_of::<AdmissionPublisher>() + 128;
    let graph = voice_admitted_admission_rpc_bytes() + native;
    eprintln!("admitted neutral Actor graph {graph}/{ADMISSION_RPC_BYTES}");
    assert!(graph <= ADMISSION_RPC_BYTES);
    assert!(voice_admitted_presence_rpc_bytes() <= ACTIVATION_RPC_BYTES);
    assert!(voice_call_activation_rpc_bytes() <= ACTIVATION_RPC_BYTES);
}

#[tokio::test]
async fn actual_admitted_actor_stages_whole_registration_and_reuses_same_observer_after_reply_loss()
{
    let call = bounded(published()).await;
    let original = call.pair.observer();
    let cell = ResultCell::new(&original);
    let work = queue(&call, &cell, None);
    bounded(cell.ready()).await;
    bounded(work.wait_retired()).await;
    assert!(work.is_retired());
    // The observing request never owned this result; the canonical paid Cell did.
    let (session, capacity) = cell
        .state
        .lock()
        .unwrap()
        .result
        .take()
        .unwrap()
        .unwrap()
        .into_parts();
    let session = session.unwrap();
    let capacity = capacity.unwrap();
    assert!(capacity.belongs_to(&session) && capacity.is_current());
    let observer = session.observer();
    let incarnation = observer.incarnation();
    drop(capacity);
    let retry = ResultCell::new(&original);
    let work = queue(&call, &retry, Some(observer));
    bounded(retry.ready()).await;
    bounded(work.wait_retired()).await;
    let (replacement, capacity) = retry
        .state
        .lock()
        .unwrap()
        .result
        .take()
        .unwrap()
        .unwrap()
        .into_parts();
    assert!(replacement.is_none());
    assert!(capacity.unwrap().belongs_to(&session));
    assert_eq!(session.incarnation(), incarnation);
    drop(session);
    bounded(call.close()).await;
}

#[tokio::test]
async fn actual_admitted_actor_same_rpc_funding_survives_retired_result_and_concurrent_last_work() {
    // No Presence is published here. Root's real Unauthorized result creates no
    // pools/registration, isolating the exact invocation allocation/payment tail.
    let call = bounded(Call::new()).await;
    let original = call.pair.observer();
    let cell = ResultCell::new(&original);
    let pool = call._runtime.resources();
    let before = pool.counts().0;
    let work = queue(&call, &cell, None);
    bounded(cell.ready()).await;
    bounded(work.wait_retired()).await;
    {
        let mut state = cell.state.lock().unwrap();
        assert!(matches!(state.result, Some(Err(VoiceError::Unauthorized))));
        assert!(!state.unknown && state.refusal.is_none());
        drop(state.work.take());
        drop(state.result.take());
    }
    assert_eq!(pool.counts().0, before + ADMISSION_RPC_BYTES);
    let first = work.clone();
    let second = work.clone();
    drop(work);
    let (one, one_done) = oneshot::channel();
    let (two, two_done) = oneshot::channel();
    drop(std::thread::spawn(move || {
        drop(first);
        let _ = one.send(());
    }));
    drop(std::thread::spawn(move || {
        drop(second);
        let _ = two.send(());
    }));
    bounded(async {
        one_done.await.unwrap();
        two_done.await.unwrap();
    })
    .await;
    assert_eq!(
        pool.counts().0,
        before,
        "the SAME funding returns exactly once"
    );
    bounded(call.close()).await;
}

#[tokio::test]
async fn actual_admitted_actor_capacity_refusal_still_stages_original_registration_for_retry() {
    let call = bounded(published()).await;
    let original = call.pair.observer();
    let cell = ResultCell::new(&original);
    let pressure: Vec<_> = (0..4)
        .map(|_| {
            call._runtime
                .resources()
                .charge(
                    call.pair.grant().scope().group,
                    1,
                    catcoms_sync::voice_v2::resources::ResourceKind::Crypto,
                )
                .unwrap()
        })
        .collect();
    let work = queue(&call, &cell, None);
    bounded(cell.ready()).await;
    bounded(work.wait_retired()).await;
    let (session, capacity) = cell
        .state
        .lock()
        .unwrap()
        .result
        .take()
        .unwrap()
        .unwrap()
        .into_parts();
    assert_eq!(capacity.err(), Some(VoiceError::Capacity));
    let session = session.expect("actual affine registration must not be discarded");
    drop(pressure);
    let retry = ResultCell::new(&original);
    let work = queue(&call, &retry, Some(session.observer()));
    bounded(retry.ready()).await;
    bounded(work.wait_retired()).await;
    let (fresh, capacity) = retry
        .state
        .lock()
        .unwrap()
        .result
        .take()
        .unwrap()
        .unwrap()
        .into_parts();
    assert!(fresh.is_none());
    assert!(capacity.unwrap().belongs_to(&session));
    drop(session);
    bounded(call.close()).await;
}

#[tokio::test]
async fn actual_admitted_actor_closed_pre_enqueue_returns_payment_without_fabricating_work() {
    let call = bounded(published()).await;
    let original = call.pair.observer();
    let cell = ResultCell::new(&original);
    let payment =
        AdmissionRpcPermit::reserve(&call._runtime.resources(), call.pair.grant().scope()).unwrap();
    original.close();
    let before = call._runtime.resources().counts();
    assert_eq!(
        call.pair
            .control()
            .try_queue_voice_admitted_admission(
                call.pair.grant().clone(),
                original.clone(),
                None,
                1,
                payment,
                AdmissionPublisher::new(&cell)
            )
            .err(),
        Some(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
    );
    let mut state = cell.state.lock().unwrap();
    assert!(state.work.is_none() && state.result.is_none() && !state.unknown);
    let returned = state.refusal.take().unwrap().1.unwrap();
    assert!(returned.matches_activation(&original));
    drop(state);
    assert_eq!(call._runtime.resources().counts(), before);
    drop(returned);
    assert_eq!(
        call._runtime.resources().counts(),
        (before.0 - ADMISSION_RPC_BYTES, before.1)
    );
    bounded(call.close()).await;
}

#[tokio::test]
async fn actual_admitted_actor_rejects_foreign_pool_scope_control_and_zero_generation_before_work()
{
    let call = bounded(published()).await;
    let other = bounded(published()).await;
    let original = call.pair.observer();
    let resources = call._runtime.resources();
    let foreign = Arc::new(catcoms_sync::voice_v2::resources::Resources::default());
    let scope = call.pair.grant().scope();
    let mut different_scope = scope;
    different_scope.room ^= 1;
    for case in 0..4 {
        let cell = ResultCell::new(&original);
        let payer = if case == 0 {
            foreign.clone()
        } else {
            resources.clone()
        };
        let booked_scope = if case == 1 { different_scope } else { scope };
        let payment = AdmissionRpcPermit::reserve(&payer, booked_scope).unwrap();
        let before = payer.counts();
        let actual_before = resources.counts();
        let control = if case == 2 {
            other.pair.control()
        } else {
            call.pair.control()
        };
        let generation = if case == 3 { 0 } else { 1 };
        let expected = VoiceEndpointError::Protocol(if case == 3 {
            VoiceError::Bounds
        } else {
            VoiceError::Scope
        });
        assert_eq!(
            control
                .try_queue_voice_admitted_admission(
                    call.pair.grant().clone(),
                    original.clone(),
                    None,
                    generation,
                    payment,
                    AdmissionPublisher::new(&cell)
                )
                .err(),
            Some(expected)
        );
        let mut state = cell.state.lock().unwrap();
        assert!(state.work.is_none() && state.result.is_none() && !state.unknown);
        let (error, returned) = state.refusal.take().unwrap();
        assert_eq!(error, expected);
        let returned = returned.expect("same affine pre-enqueue payment returned");
        assert_eq!(returned.scope(), booked_scope);
        assert_eq!(returned.matches_activation(&original), case >= 2);
        assert_eq!(payer.counts(), before);
        assert_eq!(resources.counts(), actual_before);
        drop(state);
        drop(returned);
        assert_eq!(payer.counts(), (before.0 - ADMISSION_RPC_BYTES, before.1));
    }
    bounded(other.close()).await;
    bounded(call.close()).await;
}

#[tokio::test]
async fn actual_admitted_actor_entered_callback_unwind_is_unknown_not_unstarted_even_after_work_return(
) {
    let call = bounded(published()).await;
    let original = call.pair.observer();
    let cell = ResultCell::new(&original);
    let mut publisher = AdmissionPublisher::new(&cell);
    publisher.panic_after_entry = true;
    let work = call
        .pair
        .control()
        .try_queue_voice_admitted_admission(
            call.pair.grant().clone(),
            original.clone(),
            None,
            1,
            AdmissionRpcPermit::reserve(&call._runtime.resources(), call.pair.grant().scope())
                .unwrap(),
            publisher,
        )
        .unwrap();
    bounded(cell.ready()).await;
    bounded(work.wait_retired()).await;
    {
        let state = cell.state.lock().unwrap();
        assert!(state.unknown && state.result.is_none() && state.refusal.is_none());
    }
    let error = bounded(call.task).await.unwrap_err();
    assert!(error.is_panic());
    bounded(call.drain).await.unwrap();
    assert!(
        original.root_returned().is_some(),
        "observe actual Root return separately from Work retirement"
    );
    // Neither returned Work nor Unknown lets a native consumer repeat None.
    assert!(work.is_retired());
}

#[tokio::test]
async fn actual_admitted_actor_abort_between_accept_and_send_with_ordinary_sender_alive_is_unstarted(
) {
    struct Resume(Option<std::sync::mpsc::SyncSender<()>>);
    impl Drop for Resume {
        fn drop(&mut self) {
            if let Some(send) = self.0.take() {
                let _ = send.try_send(());
            }
        }
    }
    let call = bounded(published()).await;
    let original = call.pair.observer();
    let cell = ResultCell::new(&original);
    let (entered, arrived) = oneshot::channel();
    let (send, resume) = std::sync::mpsc::sync_channel(1);
    let resume_guard = Resume(Some(send));
    let mut publisher = AdmissionPublisher::new(&cell);
    let control = call.pair.control().clone();
    publisher.pause = Some((entered, resume, control.clone()));
    let grant = call.pair.grant().clone();
    let activation = original.clone();
    let payment = AdmissionRpcPermit::reserve(&call._runtime.resources(), grant.scope()).unwrap();
    let queued = tokio::task::spawn_blocking(move || {
        control.try_queue_voice_admitted_admission(grant, activation, None, 1, payment, publisher)
    });
    bounded(arrived).await.unwrap();
    let ordinary_sender = call.actor.clone();
    call.task.abort();
    let error = bounded(call.task).await.unwrap_err();
    assert!(error.is_cancelled());
    drop(resume_guard);
    assert_eq!(
        bounded(queued).await.unwrap().err(),
        Some(VoiceEndpointError::Stopped)
    );
    bounded(cell.ready()).await;
    let work = {
        let state = cell.state.lock().unwrap();
        assert!(!state.unknown && state.result.is_none());
        assert_eq!(
            state.refusal.as_ref().unwrap().0,
            VoiceEndpointError::Stopped
        );
        state.work.as_ref().unwrap().clone()
    };
    bounded(work.wait_retired()).await;
    assert!(work.is_retired());
    bounded(call.drain).await.unwrap();
    drop(ordinary_sender);
}
