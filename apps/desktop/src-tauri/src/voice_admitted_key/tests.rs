//! Actual original Actor/Root publication; no browser SDK or audible-call claim.
use super::*;
use crate::voice_activation_tests::Fixture;
use crate::voice_authority::AdmittedPublicationPlan;
use catcoms_rt::ManualClock;
use std::task::Waker;

fn point() -> [u8; 65] {
    hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5")
        .unwrap().try_into().unwrap()
}
fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}
async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::select! {
        value = future => value,
        _ = catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(30)) => panic!("native key ownership test timed out"),
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
fn prepare(f: &Fixture, handle: &ActivationHandle) -> PendingPublication {
    let paid = PrepaidPublication::reserve(handle).unwrap();
    let work = {
        let mut authority = f.state.voice_authority.lock().unwrap();
        let admission = authority.admit_admitted_join_key(handle, [9; 32]).unwrap();
        let AdmittedPublicationPlan::Start(work) = authority
            .prepare_admitted_join_key_publication(handle, [9; 32], &admission.permit, point())
            .unwrap()
        else {
            panic!("original publication");
        };
        work
    };
    let pending = PendingPublication::new(
        handle.clone(),
        work,
        paid,
        crate::voice_runtime(&f.state).clock(),
    );
    pending.install(&f.state.voice_authority).unwrap();
    pending
}

#[test]
fn complete_publication_graph_has_a_preallocation_guard() {
    let actual = publication_layout_bytes();
    eprintln!("admitted native publication graph {actual}/{PUBLICATION_BYTES}");
    assert!(actual <= PUBLICATION_BYTES);
}

#[tokio::test]
async fn actual_root_publication_and_lost_reply_retain_the_original_key() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let (handle, mut call) = crate::voice_call_activation::tests::active_handle_fixture(&f).await;
    let pending = prepare(&f, &handle);
    let commit = f.state.ui_session_commit.clone().lock_owned().await;
    let mut task = Box::pin(pending.initial_task(&f.state, commit));
    assert!(poll_once(task.as_mut()).is_pending());
    assert!(f.state.ui_session_commit.try_lock().is_err());
    f.actor.member_count().await; // original command and actual argument retirement
    assert!(poll_once(task.as_mut()).is_ready());
    drop(task);
    assert!(pending.task_returned());
    assert!(
        handle.active(),
        "successful publication must not close its call"
    );
    assert!(f.state.ui_session_commit.try_lock().is_ok());
    let first = pending.reply(&f.state).await.unwrap();
    let retry = pending.reply(&f.state).await.unwrap();
    assert_eq!(first.key_ref, retry.key_ref);
    assert_eq!(first.join, retry.join);
    assert_eq!(first.key_ref.len(), 64);
    let settled = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .settle_admitted_join_key(handle.id(), [9; 32])
        .unwrap();
    assert!(settled.finish_outside_guard().is_none());
    handle.close();
    clock.advance_ms(250);
    assert!(poll_once(call.as_mut()).is_pending());
    f.actor.member_count().await;
    assert!(poll_once(call.as_mut()).is_ready());
    drop(call);
    drop(pending);
    drop(handle);
    f.close().await;
}

#[tokio::test]
async fn registered_lock_between_outer_snapshot_and_coordinator_blocks_positive_completion() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let (handle, mut call) = crate::voice_call_activation::tests::active_handle_fixture(&f).await;
    let pending = prepare(&f, &handle);
    let commit = f.state.ui_session_commit.clone().lock_owned().await;
    let mut task = Box::pin(pending.initial_task(&f.state, commit));
    assert!(poll_once(task.as_mut()).is_pending());
    f.actor.member_count().await; // the Root really accepted, before lock registration
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    *pending.cell.before_coordinator.lock().unwrap() = Some((entered.clone(), resume.clone()));
    let (task, polled) = std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            let polled = poll_once(task.as_mut());
            (task, polled)
        });
        entered.wait();
        // This is the production lock registration. Deliberately do NOT run
        // revoke_all yet: the final admission must observe its atomic fence,
        // not depend on winning a race against the later revocation hook.
        crate::register_ui_lock_request(&f.state);
        assert!(handle.active());
        resume.wait();
        worker.join().unwrap()
    });
    assert!(polled.is_ready());
    drop(task);
    assert!(pending.reply(&f.state).await.is_err());
    assert!(matches!(pending.public_reply(&f.state).await, PublicationReply::Accepted { .. }),
        "the actual Root publication remains historical even though current native admission lost to lock");
    assert!(handle.original().is_closed());
    assert!(f.state.ui_session_commit.try_lock().is_ok());
    let settled = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .settle_admitted_join_key(handle.id(), [9; 32])
        .unwrap();
    assert!(settled.finish_outside_guard().is_none());
    clock.advance_ms(250);
    assert!(poll_once(call.as_mut()).is_pending());
    f.actor.member_count().await;
    assert!(poll_once(call.as_mut()).is_ready());
    drop(call);
    drop(pending);
    drop(handle);
    f.close().await;
}

#[tokio::test]
async fn acknowledged_result_and_sdk_settlement_cannot_collect_a_still_destroying_execution() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let (handle, mut call) = crate::voice_call_activation::tests::active_handle_fixture(&f).await;
    let pending = prepare(&f, &handle);
    let commit = f.state.ui_session_commit.clone().lock_owned().await;
    let mut task = Box::pin(pending.initial_task(&f.state, commit));
    assert!(poll_once(task.as_mut()).is_pending());
    f.actor.member_count().await;
    let (entered, observed) = tokio::sync::oneshot::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    let resume = ResumeOnDrop(Some(resume));
    *pending.cell.before_execution_free.lock().unwrap() = Some((entered, resumed));
    let (finished, done) = tokio::sync::oneshot::channel();
    drop(std::thread::spawn(move || {
        let polled = poll_once(task.as_mut());
        drop(task);
        let _ = finished.send(polled);
    }));
    bounded(observed).await.unwrap();
    assert!(pending.reply(&f.state).await.is_ok());
    pending.acknowledge_result().unwrap();
    let settled = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .settle_admitted_join_key(handle.id(), [9; 32])
        .unwrap();
    assert!(settled.finish_outside_guard().is_none());
    assert!(!pending.is_collectible());
    assert!(f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .collect_completed_admitted_publication(handle.id())
        .is_none());
    assert!(f.state.ui_session_commit.try_lock().is_ok());
    drop(resume);
    assert!(bounded(done).await.unwrap().is_ready());
    assert!(pending.is_collectible());
    let collected = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .collect_completed_admitted_publication(handle.id())
        .unwrap();
    assert!(collected.is_same(&pending));
    drop(collected);
    handle.close();
    clock.advance_ms(250);
    assert!(poll_once(call.as_mut()).is_pending());
    f.actor.member_count().await;
    assert!(poll_once(call.as_mut()).is_ready());
    drop(call);
    drop(pending);
    drop(handle);
    f.close().await;
}

#[tokio::test]
async fn typed_root_refusal_survives_end_and_lost_reply_until_ack_before_settlement() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let (handle, mut call) = crate::voice_call_activation::tests::active_handle_fixture(&f).await;
    let pending = prepare(&f, &handle);
    let commit = f.state.ui_session_commit.clone().lock_owned().await;
    let mut task = Box::pin(pending.initial_task(&f.state, commit));
    assert!(poll_once(task.as_mut()).is_pending());
    handle.close(); // actual queued Root turn must refuse the closed original
    f.actor.member_count().await;
    assert!(poll_once(task.as_mut()).is_ready());
    drop(task);
    let first = serde_json::to_value(pending.public_reply(&f.state).await).unwrap();
    assert_eq!(first["kind"], "rejected");
    assert_eq!(first["code"], "refused");
    let recovered = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_publication_task_for_cleanup(handle.id(), [9; 32])
        .unwrap();
    assert!(recovered.is_same(&pending));
    assert_eq!(
        serde_json::to_value(recovered.public_reply(&f.state).await).unwrap(),
        first
    );
    // Model a lost ACK reply. SDK custody still pins the same terminal cell,
    // so repeating the explicit ACK neither generates nor publishes anything.
    acknowledge_inner(&f.state, handle.id(), [9; 32]).unwrap();
    acknowledge_inner(&f.state, handle.id(), [9; 32]).unwrap();
    assert!(f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_publication_task_for_cleanup(handle.id(), [9; 32])
        .is_some());
    settle_inner(&f.state, handle.id(), [9; 32]).await.unwrap();
    clock.advance_ms(250);
    assert!(poll_once(call.as_mut()).is_pending());
    f.actor.member_count().await;
    assert!(poll_once(call.as_mut()).is_ready());
    drop(call);
    drop(recovered);
    drop(pending);
    drop(handle);
    f.close().await;
}

#[tokio::test]
async fn unpolled_publication_joins_actual_root_return_without_inventing_rejection() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let (handle, mut call) = crate::voice_call_activation::tests::active_handle_fixture(&f).await;
    let pending = prepare(&f, &handle);
    let commit = f.state.ui_session_commit.clone().lock_owned().await;
    drop(pending.initial_task(&f.state, commit));
    assert!(pending.task_returned());
    assert!(!pending.cell.completed.load(Ordering::Acquire));
    clock.advance_ms(250);
    assert!(poll_once(call.as_mut()).is_pending());
    f.actor.member_count().await;
    assert!(poll_once(call.as_mut()).is_ready());
    drop(call);
    assert!(handle.original().root_returned().is_some());
    let mut recovery = Box::pin(pending.negative_task(&f.state).unwrap());
    assert!(poll_once(recovery.as_mut()).is_ready());
    drop(recovery);
    let reply = serde_json::to_value(pending.public_reply(&f.state).await).unwrap();
    assert_eq!(reply["kind"], "closed");
    assert_eq!(reply["code"], "unconfirmed");
    acknowledge_inner(&f.state, handle.id(), [9; 32]).unwrap();
    settle_inner(&f.state, handle.id(), [9; 32]).await.unwrap();
    drop(pending);
    drop(handle);
    f.close().await;
}

#[tokio::test]
async fn closed_unfrozen_generation_proves_not_started_after_prepublication_capacity_refusal() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let (handle, mut call) = crate::voice_call_activation::tests::active_handle_fixture(&f).await;
    let admission = admit_inner(&f.state, handle.id(), [9; 32]).await.unwrap();
    let resources = crate::voice_runtime(&f.state).resources();
    let group = handle.binding().scope.group;
    let occupied = resources.group_counts(group).unwrap().0;
    let pressure = resources
        .charge(
            group,
            catcoms_sync::voice_v2::resources::GROUP_BYTES - occupied,
            catcoms_sync::voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    assert!(
        publish_public_inner(&f.state, handle.id(), [9; 32], &admission.permit, point())
            .await
            .is_err()
    );
    assert!(
        handle.active(),
        "prepublication capacity must not fabricate an entered request"
    );
    handle.close();
    clock.advance_ms(250);
    assert!(poll_once(call.as_mut()).is_pending());
    f.actor.member_count().await;
    assert!(poll_once(call.as_mut()).is_ready());
    drop(call);
    let before = resources.counts();
    let result = publish_public_inner(&f.state, handle.id(), [9; 32], &admission.permit, point())
        .await
        .unwrap();
    let encoded = serde_json::to_value(result).unwrap();
    assert_eq!(encoded["kind"], "not-started");
    assert_eq!(encoded["code"], "ended");
    assert_eq!(encoded["join"], admission.join);
    assert_eq!(
        resources.counts(),
        before,
        "proof uses existing metadata, not a fresh task/RPC fee"
    );
    assert!(
        acknowledge_inner(&f.state, handle.id(), [9; 32]).is_err(),
        "no terminal Cell was invented"
    );
    settle_inner(&f.state, handle.id(), [9; 32]).await.unwrap();
    drop(pressure);
    drop(handle);
    f.close().await;
}
