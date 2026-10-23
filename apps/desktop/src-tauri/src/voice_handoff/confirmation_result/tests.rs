use super::*;
#[test]
fn concrete_worker_and_publication_fit_the_original_task_contribution() {
    fn size<F: std::future::Future>(_: impl FnOnce(Worker) -> F) -> usize {
        worker_layout_bytes(std::mem::size_of::<F>())
    }
    let bytes = size(run);
    assert!(bytes <= TASK_BYTES, "result worker requires {bytes}");
    // Worst admitted Data body already includes its AEAD tag. Actor metadata pays its
    // bounded ResultOwner/Bytes/BodyState/flight/mailbox/completion graph; Publication's
    // pointee and Arc header are separately counted in worker_layout_bytes above.
    let total = bytes + catcoms_app::VOICE_CONFIRMATION_BODY_METADATA_BYTES + 2064;
    assert!(
        total <= 16 * 1024,
        "result worker/body/frame requires {total}"
    );
}

/// Exercises this actual paid worker, not the child decoder: the native Derive fixture
/// deliberately cannot publish a confirmation result. Lock must refuse before that
/// operation check, while the uncollected failure still pays its real task/physical lane.
#[tokio::test]
async fn caller_loss_during_binding_query_keeps_paid_worker_without_mounted_context() {
    use super::super::derivation_result::tests::Case;
    use super::super::factory::tests::park;
    use std::future::{poll_fn, Future};
    use std::task::Poll;
    let case = Case::new().await;
    let phase = case.row().work().unwrap().phase().clone();
    let incoming = case.frame(1, &case.bytes());
    let work = case.row().work().unwrap();
    let payment = work.phase().copy(TASK_BYTES).unwrap();
    let physical = incoming.hold_position().unwrap();
    let parked = park(&case.native.actor).await;
    let store_owners = Arc::strong_count(&case.native.state.store);
    let server_owners = Arc::strong_count(&case.native.state.servers);
    let mut worker = Box::pin(run(Worker {
        context: Some(case.context.clone()),
        route: Some(case.route.clone()),
        work,
        tag: case.action.output_tag(&[]).unwrap(),
        output: 187..187,
        publication: Some(Arc::new(PublicationCustody::preparing(physical, payment))),
        incoming,
    }));
    assert!(poll_fn(|cx| Poll::Ready(worker.as_mut().poll(cx).is_pending())).await);
    assert_eq!(Arc::strong_count(&case.native.state.store), store_owners);
    assert_eq!(Arc::strong_count(&case.native.state.servers), server_owners);
    assert!(case.route.session().fixture_counts().0);
    assert_eq!(phase.copy(TASK_BYTES).err(), Some(VoiceError::Capacity));
    let caller = tokio::spawn(worker);
    drop(caller); // actual detached worker, never a fabricated completed/drained bit
    crate::register_ui_lock_request(&case.native.state);
    drop(parked);
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        case.route.session().fixture_wait_physical_idle(),
    )
    .await
    .unwrap();
    assert!(case.action.terminal_status().receipt.is_none());
    drop(phase.copy(16 * 1024).unwrap());
    crate::lock_session_inner(&case.native.state, None)
        .await
        .unwrap();
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn pre_admission_refusal_keeps_actual_task_payment_and_position_until_collection() {
    use super::super::derivation_result::tests::Case;
    use std::future::{poll_fn, Future};
    use std::task::Poll;
    let case = Case::new().await;
    let phase = case.row().work().unwrap().phase().clone();
    let incoming = case.frame(1, &case.bytes());
    let work = case.row().work().unwrap();
    let payment = work.phase().copy(TASK_BYTES).unwrap();
    let physical = incoming.hold_position().unwrap();
    let publication = Arc::new(PublicationCustody::preparing(physical, payment));
    let boundary = case.native.state.ui_session_commit.lock().await;
    let mut worker = Box::pin(run(Worker {
        context: Some(case.context.clone()),
        route: Some(case.route.clone()),
        work,
        tag: case.action.output_tag(&[]).unwrap(),
        output: 187..187,
        publication: Some(publication),
        incoming,
    }));
    assert!(poll_fn(|cx| Poll::Ready(worker.as_mut().poll(cx).is_pending())).await);
    case.native.actor.member_count().await; // actual earlier binding query has completed
    assert!(poll_fn(|cx| Poll::Ready(worker.as_mut().poll(cx).is_pending())).await);
    // Waiting owns only the original commit handle, not a strong mounted-store context.
    assert_eq!(Arc::strong_count(&case.native.state.store), 1);
    crate::register_ui_lock_request(&case.native.state);
    drop(boundary);
    let task = tokio::spawn(worker); // the actual task is independent of a socket waiter
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(phase.copy(TASK_BYTES).err(), Some(VoiceError::Capacity));
    assert!(case.route.session().fixture_counts().0);
    assert!(case.action.terminal_status().receipt.is_none());
    let response = task.await.unwrap();
    assert!(matches!(response.result, Err(Refused::Closed)));
    assert!(response.work.is_some() && response.payment.is_some());
    assert_eq!(phase.copy(TASK_BYTES).err(), Some(VoiceError::Capacity));
    assert!(case.route.session().fixture_counts().0);
    drop(response);
    assert!(!case.route.session().fixture_counts().0);
    drop(phase.copy(16 * 1024).unwrap());
    crate::lock_session_inner(&case.native.state, None)
        .await
        .unwrap();
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn cancelling_early_worker_cannot_clear_physical_position_before_actual_work_retires() {
    use super::super::derivation_result::tests::Case;
    use std::future::{poll_fn, Future};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    use std::task::Poll;
    let case = Case::new().await;
    let incoming = case.frame(1, &case.bytes());
    let row = case.row();
    let work = row.work().unwrap();
    let payment = work.phase().copy(TASK_BYTES).unwrap();
    let physical = incoming.hold_position().unwrap();
    let boundary = case.native.state.ui_session_commit.lock().await;
    let mut worker = Box::pin(run(Worker {
        context: Some(case.context.clone()),
        route: Some(case.route.clone()),
        work,
        tag: case.action.output_tag(&[]).unwrap(),
        output: 187..187,
        publication: Some(Arc::new(PublicationCustody::preparing(physical, payment))),
        incoming,
    }));
    assert!(poll_fn(|cx| Poll::Ready(worker.as_mut().poll(cx).is_pending())).await);
    case.native.actor.member_count().await;
    assert!(poll_fn(|cx| Poll::Ready(worker.as_mut().poll(cx).is_pending())).await);
    let guard = row.fixture_hold_work_retirement();
    let before = Arc::strong_count(&case.action);
    let cancel = AtomicBool::new(false);
    // Work::drop first releases its actual Action Arc, THEN takes this held Row State.
    // Observe that real destructor progress, rather than a sleep or a fabricated flag.
    let (observed, busy) = std::thread::scope(|scope| {
        let action = &case.action;
        let cancelled = &cancel;
        let (sent, received) = mpsc::channel();
        let observer = scope.spawn(move || {
            while Arc::strong_count(action) >= before && !cancelled.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            if !cancelled.load(Ordering::Acquire) {
                let _ = sent.send(());
            }
        });
        let dropped = scope.spawn(move || drop(worker)); // actual future cancellation
        let observed = received.recv_timeout(std::time::Duration::from_secs(5));
        let busy = case.route.session().fixture_counts().0;
        cancel.store(true, Ordering::Release);
        drop(guard); // release BEFORE joins/assertions, including a failing regression
        dropped.join().unwrap();
        observer.join().unwrap();
        (observed, busy)
    });
    observed.unwrap();
    assert!(
        busy,
        "physical idle preceded actual blocked Work retirement"
    );
    assert!(!case.route.session().fixture_counts().0);
    assert!(case.action.terminal_status().receipt.is_none());
    drop(boundary);
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn renderer_result_labels_cannot_create_a_native_child_or_admit_actor_work() {
    use super::super::derivation_result::tests::Case;
    use super::super::service::tests::opened_frame;
    let case = Case::unadmitted().await;
    let foreign = Case::unadmitted().await;
    let mut row = case.id.logical();
    row[15] += 1; // canonical child-shaped header, with NO actual registered child
    let mut body = vec![2];
    body.extend_from_slice(case.selector.peer.as_bytes());
    body.extend_from_slice(&case.selector.join);
    body.extend_from_slice(&case.selector.key_ref);
    body.extend_from_slice(&case.selector.epoch.to_be_bytes());
    body.extend_from_slice(&case.id.logical());
    body.push(0); // Start(None)
    body.extend_from_slice(&case.action.serial().to_be_bytes());
    body.extend_from_slice(&case.action.version().to_be_bytes());
    body.extend_from_slice(&case.action.input_tag());
    body.extend_from_slice(&<[u8; 32]>::from(Sha256::digest([9; 16])));
    body.extend_from_slice(&16u16.to_be_bytes());
    body.extend_from_slice(&[9; 16]);
    let phase = case.row().work().unwrap().phase().clone();
    let before = case.selected.resources().counts();
    for (index, offset) in [
        None,
        Some(0),
        Some(1),
        Some(105),
        Some(121),
        Some(170),
        Some(204),
    ]
    .into_iter()
    .enumerate()
    {
        let mut bytes = body.clone();
        if let Some(offset) = offset {
            bytes[offset] ^= 1;
        }
        let incoming = opened_frame(
            &case.public,
            &case.route,
            (index as u64 + 1, 3, 1, row),
            &bytes,
            Some(&phase),
        );
        let response = commit(&case.context, &case.route, incoming).await.unwrap();
        assert!(response.result.is_err());
        assert!(response.work.is_none() && response.payment.is_none());
        assert_eq!(case.selected.resources().counts(), before);
        assert!(case.native.state.ui_session_commit.try_lock().is_ok());
        drop(response);
    }
    let foreign_phase = foreign.row().work().unwrap().phase().clone();
    let incoming = opened_frame(
        &foreign.public,
        &foreign.route,
        (1, 3, 1, row),
        &body,
        Some(&foreign_phase),
    );
    let response = commit(&case.context, &case.route, incoming).await.unwrap();
    assert!(matches!(
        response.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    assert!(response.work.is_none() && response.payment.is_none());
    drop(response);
    assert!(case.action.terminal_status().receipt.is_none());
    case.retire();
    foreign.retire();
    case.native.close().await;
    foreign.native.close().await;
}
