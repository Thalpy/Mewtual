//! Actual Actor/Root queue and affine staging. These controlled publishers are
//! lifecycle fixtures, not production native Session/UI or signed Ready authority.
use super::*;
use catcoms_sync::voice_v2::admission::{
    AdmissionPrepared, AdmissionRpcPermit, RootAdmissionSession, RootHighPreclaim,
    ADMISSION_RPC_BYTES,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

struct Staged {
    registration: Option<RootAdmissionSession>,
    capacity: Result<RootHighPreclaim, VoiceError>,
    work: VoiceAdmissionWork,
}
#[derive(Default)]
struct Slot {
    accepted: Option<VoiceAdmissionWork>,
    staged: Option<Staged>,
    unstarted: Option<VoiceEndpointError>,
    uncertain: bool,
    cancel: u64,
}
#[derive(Default)]
struct Sink {
    slot: Mutex<Slot>,
    changed: tokio::sync::Notify,
}
impl Sink {
    async fn changed(&self) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let slot = self.slot.lock().unwrap();
            let ready = slot.staged.is_some() || slot.unstarted.is_some() || slot.uncertain;
            drop(slot);
            if ready {
                return;
            }
            notified.await;
        }
    }
    fn take(&self) -> Staged {
        // Move the original heavy result out. Never destroy it under the native
        // fixture lock, even on cancellation or an assertion failure afterward.
        let (staged, accepted) = {
            let mut slot = self.slot.lock().unwrap();
            (slot.staged.take().unwrap(), slot.accepted.take())
        };
        drop(accepted);
        staged
    }
    fn take_accepted(&self) -> Option<VoiceAdmissionWork> {
        self.slot.lock().unwrap().accepted.take()
    }
}
struct Publisher {
    sink: Arc<Sink>,
    completed: bool,
    panic_after_entry: bool,
    reject_accept: bool,
    returned: Option<(oneshot::Sender<()>, std::sync::mpsc::Receiver<()>)>,
    refused_while_paid: Option<(
        Arc<catcoms_sync::voice_v2::resources::Resources>,
        usize,
        Arc<AtomicBool>,
    )>,
}
impl Publisher {
    fn new(sink: &Arc<Sink>) -> Box<Self> {
        Box::new(Self {
            sink: sink.clone(),
            completed: false,
            panic_after_entry: false,
            reject_accept: false,
            returned: None,
            refused_while_paid: None,
        })
    }
}
impl Drop for Publisher {
    fn drop(&mut self) {
        if !self.completed {
            self.sink.slot.lock().unwrap().uncertain = true;
            self.sink.changed.notify_waiters();
        }
    }
}
impl VoiceAdmissionPublisher for Publisher {
    fn accepted(&mut self, work: VoiceAdmissionWork) -> Result<(), VoiceEndpointError> {
        let mut slot = self.sink.slot.lock().unwrap();
        assert!(slot.accepted.is_none());
        slot.accepted = Some(work);
        if self.reject_accept {
            Err(VoiceEndpointError::Protocol(VoiceError::Capacity))
        } else {
            Ok(())
        }
    }
    fn completed(
        mut self: Box<Self>,
        result: Result<AdmissionPrepared, VoiceError>,
        work: VoiceAdmissionWork,
    ) {
        if self.panic_after_entry {
            panic!("controlled publisher unwind after Root entry");
        }
        let (registration, capacity) = match result {
            Ok(prepared) => prepared.into_parts(),
            Err(error) => (None, Err(error)),
        };
        let mut outcome = Some(Staged {
            registration,
            capacity,
            work,
        });
        {
            let mut slot = self.sink.slot.lock().unwrap();
            assert!(
                slot.staged.is_none(),
                "the same invocation stages at most once"
            );
            if let Some(owner) = outcome.as_ref().unwrap().registration.as_ref() {
                if slot.cancel != 0 {
                    owner.observer().cancel_through(slot.cancel).unwrap();
                }
            }
            slot.staged = outcome.take();
        }
        self.completed = true;
        self.sink.changed.notify_waiters();
        if let Some((entered, resume)) = self.returned.take() {
            let _ = entered.send(());
            let _ = resume.recv(); // Sender Drop releases even a failing test.
        }
    }
    fn refused(mut self: Box<Self>, error: VoiceEndpointError, work: Option<VoiceAdmissionWork>) {
        if let Some((resources, held, recorded)) = self.refused_while_paid.take() {
            recorded.store(resources.counts().0 == held, Ordering::Release);
        }
        let mut slot = self.sink.slot.lock().unwrap();
        assert!(slot.staged.is_none());
        if let Some(work) = work {
            if let Some(original) = &slot.accepted {
                assert!(original.is_same(&work));
            } else {
                slot.accepted = Some(work);
            }
        }
        slot.unstarted = Some(error);
        drop(slot);
        self.completed = true;
        self.sink.changed.notify_waiters();
    }
}

#[test]
fn admission_actor_queue_and_concrete_fixture_publisher_fit_the_fixed_rpc_fee() {
    // Sink/Barriers are external harness state. The production publisher must
    // separately count its concrete canonical-Row wrapper before Box allocation.
    let bytes = voice_admission_rpc_bytes()
        + std::mem::size_of::<Publisher>()
        + 2 * std::mem::size_of::<usize>();
    assert!(
        bytes <= ADMISSION_RPC_BYTES,
        "actual Actor/publisher graph {bytes}/{ADMISSION_RPC_BYTES}"
    );
}

#[tokio::test]
async fn queued_admission_cancel_keeps_the_actual_mint_and_old_high_paid_through_collection() {
    let (actor, mut events, task, runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let parked = park(&actor).await;
    let resources = runtime.resources();
    let before = resources.counts();
    let sink = Arc::new(Sink::default());
    let payment = AdmissionRpcPermit::reserve(&resources, grant.scope()).unwrap();
    let work = actor
        .try_queue_voice_admission(grant.clone(), None, 1, payment, Publisher::new(&sink))
        .unwrap();
    assert!(!work.is_retired());
    sink.slot.lock().unwrap().cancel = 1;
    drop(parked);
    sink.changed().await;
    work.wait_retired().await;
    let staged = sink.take();
    let owner = staged.registration.as_ref().unwrap();
    let id = owner.incarnation();
    assert!(
        owner.is_current(),
        "one cancelled generation does not close the stable registration"
    );
    assert!(!staged.capacity.as_ref().unwrap().is_current());
    let held = resources.counts();
    drop(work);
    assert_eq!(
        resources.counts(),
        held,
        "canonical output retains its actual RPC fee"
    );
    let Staged {
        registration,
        capacity,
        work,
    } = staged;
    drop(capacity);
    let owner = registration.unwrap();
    assert_eq!(owner.incarnation(), id);
    drop(work);
    assert_eq!(
        resources.counts().0,
        before.0 + catcoms_sync::voice_v2::admission::REGISTRATION_METADATA_BYTES
    );
    drop(owner);
    assert_eq!(resources.counts(), before);
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn full_admission_mailbox_is_proven_unstarted_paid_during_refusal_and_retries_same_generation(
) {
    let (actor, mut events, task, runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let parked = park(&actor).await;
    let mut permits = Vec::new();
    while let Ok(permit) = actor.cmd_tx.tx.try_reserve() {
        permits.push(permit);
    }
    let resources = runtime.resources();
    let before = resources.counts();
    let payment = AdmissionRpcPermit::reserve(&resources, grant.scope()).unwrap();
    let recorded = Arc::new(AtomicBool::new(false));
    let sink = Arc::new(Sink::default());
    let mut publisher = Publisher::new(&sink);
    publisher.refused_while_paid =
        Some((resources.clone(), resources.counts().0, recorded.clone()));
    assert_eq!(
        actor
            .try_queue_voice_admission(grant.clone(), None, 1, payment, publisher)
            .err(),
        Some(VoiceEndpointError::Protocol(VoiceError::Capacity))
    );
    assert!(recorded.load(Ordering::Acquire));
    assert_eq!(resources.counts(), before);
    assert!(!sink.slot.lock().unwrap().uncertain);
    drop(permits);
    let retry = Arc::new(Sink::default());
    let payment = AdmissionRpcPermit::reserve(&resources, grant.scope()).unwrap();
    let work = actor
        // Keep the fixture's original call grant alive across the exact-return
        // assertion. Moving its LAST Arc would revoke the call and independently
        // collect LocalJoin, changing the baseline being measured.
        .try_queue_voice_admission(grant.clone(), None, 1, payment, Publisher::new(&retry))
        .unwrap();
    drop(parked);
    work.wait_retired().await;
    drop(retry.take());
    drop(work);
    assert_eq!(resources.counts(), before);
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn admission_command_destroyed_before_apply_stages_never_entered_before_actual_retirement() {
    let (actor, mut events, task, runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let parked = park(&actor).await;
    let resources = runtime.resources();
    let sink = Arc::new(Sink::default());
    let payment = AdmissionRpcPermit::reserve(&resources, grant.scope()).unwrap();
    let work = actor
        .try_queue_voice_admission(grant, None, 1, payment, Publisher::new(&sink))
        .unwrap();
    assert!(sink
        .slot
        .lock()
        .unwrap()
        .accepted
        .as_ref()
        .unwrap()
        .is_same(&work));
    drop(work); // Actual native-worker observer loss BEFORE Actor ever applies it.
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(parked);
    drain.await.unwrap();
    let work = sink
        .take_accepted()
        .expect("queued-drop refusal preserves actual retirement custody");
    work.wait_retired().await;
    assert_eq!(
        sink.slot.lock().unwrap().unstarted,
        Some(VoiceEndpointError::Stopped)
    );
    assert!(!sink.slot.lock().unwrap().uncertain);
    let held = resources.counts();
    drop(work);
    assert_eq!(resources.counts().0, held.0 - ADMISSION_RPC_BYTES);
}

#[tokio::test]
async fn admission_caller_loss_while_queued_keeps_canonical_work_and_the_original_successful_mint()
{
    let (actor, mut events, task, runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let parked = park(&actor).await;
    let resources = runtime.resources();
    let before = resources.counts();
    let sink = Arc::new(Sink::default());
    let payment = AdmissionRpcPermit::reserve(&resources, grant.scope()).unwrap();
    let work = actor
        .try_queue_voice_admission(grant.clone(), None, 1, payment, Publisher::new(&sink))
        .unwrap();
    assert!(sink
        .slot
        .lock()
        .unwrap()
        .accepted
        .as_ref()
        .unwrap()
        .is_same(&work));
    drop(work);
    assert_eq!(
        resources.counts(),
        (before.0 + ADMISSION_RPC_BYTES, before.1)
    );
    drop(parked);
    sink.changed().await;
    let staged = sink.take();
    staged.work.wait_retired().await;
    let owner = staged.registration.as_ref().unwrap();
    assert!(staged.capacity.as_ref().unwrap().belongs_to(owner));
    let original = owner.incarnation();
    assert_eq!(owner.observer().incarnation(), original);
    drop(staged);
    assert_eq!(resources.counts(), before);
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn admission_publisher_unwind_after_root_entry_is_uncertain_not_unused_or_remint_permission()
{
    let (actor, mut events, task, runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let sink = Arc::new(Sink::default());
    let payment = AdmissionRpcPermit::reserve(&runtime.resources(), grant.scope()).unwrap();
    let mut publisher = Publisher::new(&sink);
    publisher.panic_after_entry = true;
    let work = actor
        .try_queue_voice_admission(grant, None, 1, payment, publisher)
        .unwrap();
    drop(work); // Canonical accepted custody outlives its actual native caller.
    assert!(task.await.unwrap_err().is_panic());
    drain.await.unwrap();
    let work = sink
        .take_accepted()
        .expect("unknown Root publication keeps its original retirement witness");
    work.wait_retired().await;
    let slot = sink.slot.lock().unwrap();
    assert!(slot.uncertain);
    assert!(slot.unstarted.is_none() && slot.staged.is_none());
}

#[tokio::test]
async fn native_custody_refusal_after_work_install_never_enqueues_root_or_loses_retirement() {
    let (actor, mut events, task, runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let resources = runtime.resources();
    let before = resources.counts();
    let sink = Arc::new(Sink::default());
    let payment = AdmissionRpcPermit::reserve(&resources, grant.scope()).unwrap();
    let mut publisher = Publisher::new(&sink);
    publisher.reject_accept = true;
    assert_eq!(
        actor
            .try_queue_voice_admission(grant.clone(), None, 1, payment, publisher)
            .err(),
        Some(VoiceEndpointError::Protocol(VoiceError::Capacity))
    );
    let work = sink.take_accepted().unwrap();
    work.wait_retired().await;
    assert_eq!(
        sink.slot.lock().unwrap().unstarted,
        Some(VoiceEndpointError::Protocol(VoiceError::Capacity))
    );
    assert!(!sink.slot.lock().unwrap().uncertain);
    assert_eq!(
        resources.counts(),
        (before.0 + ADMISSION_RPC_BYTES, before.1)
    );
    drop(work);
    assert_eq!(resources.counts(), before);
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn staged_root_outcome_does_not_retire_while_the_actual_publisher_callback_is_held() {
    let (actor, mut events, task, runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let sink = Arc::new(Sink::default());
    let (entered, entered_waiter) = oneshot::channel();
    let (resume, resume_waiter) = std::sync::mpsc::channel();
    let payment = AdmissionRpcPermit::reserve(&runtime.resources(), grant.scope()).unwrap();
    let mut publisher = Publisher::new(&sink);
    publisher.returned = Some((entered, resume_waiter));
    let work = actor
        .try_queue_voice_admission(grant, None, 1, payment, publisher)
        .unwrap();
    let entered_result = bounded(entered_waiter).await;
    let retired_while_held = work.is_retired();
    let staged_while_held = sink.slot.lock().unwrap().staged.is_some();
    // Release/join BEFORE assertions so the intended regression fails instead of
    // blocking the actual Actor thread during runtime teardown.
    drop(resume);
    let retired_result = bounded(work.wait_retired()).await;
    if staged_while_held {
        drop(sink.take());
    }
    drop(work);
    let stopped = bounded(actor.shutdown()).await;
    if stopped.is_err() {
        task.abort();
    }
    let mut task = task;
    let joined = bounded(&mut task).await;
    if joined.is_err() {
        task.abort();
        let _ = bounded(&mut task).await;
    }
    let mut drain = drain;
    let drained = bounded(&mut drain).await;
    if drained.is_err() {
        drain.abort();
        let _ = bounded(&mut drain).await;
    }
    assert!(matches!(entered_result, Ok(Ok(()))));
    assert!(retired_result.is_ok() && stopped.is_ok());
    assert!(matches!(joined, Ok(Ok(()))) && matches!(drained, Ok(Ok(()))));
    assert!(!retired_while_held, "staging is not actual callback return");
    assert!(staged_while_held);
}

/// Harness failure bound only. Protocol state continues to use ManualClock.
async fn bounded<T>(future: impl std::future::Future<Output = T>) -> Result<T, &'static str> {
    use catcoms_rt::Clock;
    let clock = catcoms_rt::SystemClock;
    tokio::select! {
        value = future => Ok(value),
        _ = clock.sleep(std::time::Duration::from_secs(10)) => Err("admission harness bound elapsed"),
    }
}
