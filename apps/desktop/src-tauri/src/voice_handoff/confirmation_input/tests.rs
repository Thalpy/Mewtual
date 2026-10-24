//! Rejection/admission boundaries use real opened frames, not asserted authorization.
//! Genuine positive Input/engine/cleanup is exercised by the blocked-TCP native fixture.
use super::super::custody::CloseMode;
use super::super::ledger::{Begin, ConfirmationPurpose};
use super::super::service::tests::{opened_frame, opened_reply};
use super::*;
use std::sync::atomic::Ordering;
type Case = super::super::derivation_result::tests::Case;
async fn pending() -> (Case, RowId, Vec<u8>) {
    let case = Case::unadmitted().await;
    let parent = case.row();
    parent.settle().unwrap();
    parent
        .close_phase(case.route.session().identity(), CloseMode::Return)
        .unwrap();
    assert!(parent.try_compact().unwrap());
    let (Begin::Factory(child), _) = case
        .native
        .state
        .voice_handoff
        .begin_child(
            case.selected.clone(),
            &parent,
            ConfirmationPurpose::Start(None),
        )
        .unwrap()
    else {
        panic!("actual pending child")
    };
    let id = child.id();
    child.mark_handoff().unwrap();
    drop(child);
    let mut prefix = vec![2];
    prefix.extend_from_slice(case.selector.peer.as_bytes());
    prefix.extend_from_slice(&case.selector.join);
    prefix.extend_from_slice(&case.selector.key_ref);
    prefix.extend_from_slice(&case.selector.epoch.to_be_bytes());
    prefix.extend_from_slice(&case.id.logical());
    prefix.push(0);
    (case, id, prefix)
}
fn row(case: &Case, id: RowId) -> ResolvedRow {
    case.native
        .state
        .voice_handoff
        .lookup_child(
            &case.selected,
            &case.row(),
            ConfirmationPurpose::Start(None),
            Some(id),
        )
        .unwrap()
}
async fn close(case: Case, id: RowId) {
    row(&case, id).close_pending().unwrap();
    case.row().retire_plan().unwrap();
    case.native.close().await;
}

#[tokio::test]
async fn pending_unknown_and_foreign_input_cannot_create_work_or_relabel_parent() {
    let (case, id, prefix) = pending().await;
    let (foreign, foreign_id, _) = pending().await;
    let before = case.selected.resources().counts();
    let initial = row(&case, id).status().unwrap();
    for (sequence, offset) in [
        None,
        Some(1),
        Some(33),
        Some(65),
        Some(97),
        Some(105),
        Some(120),
    ]
    .into_iter()
    .enumerate()
    {
        let mut body = prefix.clone();
        if let Some(offset) = offset {
            body[offset] ^= 1;
        }
        let header = (sequence as u64 + 1, 1, 0, id.logical());
        let reply = pull(
            &case.context,
            &case.route,
            opened_frame(&case.public, &case.route, header, &body, None),
        )
        .await
        .unwrap();
        assert_eq!(opened_reply(&case.public, header, &body, &reply).len(), 3);
        assert_eq!(case.selected.resources().counts(), before);
        assert!(row(&case, id).status().unwrap() == initial);
    }
    let header = (1, 1, 0, id.logical());
    let incoming = opened_frame(&foreign.public, &foreign.route, header, &prefix, None);
    let reply = pull(&case.context, &case.route, incoming).await.unwrap();
    assert_eq!(
        opened_reply(&foreign.public, header, &prefix, &reply).len(),
        3
    );
    drop(reply);
    close(case, id).await;
    close(foreign, foreign_id).await;
}

#[tokio::test]
async fn child_admission_rejects_fake_action_and_parent_header_even_after_unlock() {
    let (case, id, mut body) = pending().await;
    body.extend_from_slice(&1u64.to_be_bytes());
    body.extend_from_slice(&1u64.to_be_bytes());
    body.extend_from_slice(&[6; 32]);
    body.extend_from_slice(&4096u32.to_be_bytes());
    let before = case.selected.resources().counts();
    let initial = row(&case, id).status().unwrap();
    for (sequence, logical) in [(1, id.logical()), (2, case.id.logical()), (3, id.logical())] {
        if sequence == 3 {
            // A new UI generation supplies no actual child action to this old Session.
            case.native
                .state
                .session_lock_requested
                .store(true, Ordering::SeqCst);
            case.native
                .state
                .ui_session_generation
                .fetch_add(1, Ordering::SeqCst);
            case.native
                .state
                .session_lock_requested
                .store(false, Ordering::SeqCst);
        }
        let header = (sequence, 2, 0, logical);
        let reply = crate::voice_handoff::confirmation_admission::admit(
            &case.context,
            &case.route,
            opened_frame(&case.public, &case.route, header, &body, None),
        )
        .await
        .unwrap();
        assert_eq!(opened_reply(&case.public, header, &body, &reply).len(), 3);
        assert_eq!(case.selected.resources().counts(), before);
        assert!(row(&case, id).status().unwrap() == initial);
    }
    close(case, id).await;
}

#[tokio::test]
async fn child_cleanup_cannot_acknowledge_invented_receipt_or_turn_pending_into_drained() {
    let (case, id, prefix) = pending().await;
    let initial = row(&case, id).status().unwrap();
    let before = case.selected.resources().counts();
    for (sequence, operation) in [(1, 4), (2, 5)] {
        let mut body = prefix.clone();
        if operation == 6 {
            body.push(0);
        } else {
            body.extend_from_slice(&1u64.to_be_bytes());
            body.extend_from_slice(&1u64.to_be_bytes());
            body.extend_from_slice(&[6; 32]);
            if operation == 4 {
                body.extend_from_slice(&[7; 32]);
            }
        }
        let header = (sequence, operation, 0, id.logical());
        let reply = crate::voice_handoff::confirmation_settlement::apply(
            &case.context,
            &case.route,
            opened_frame(&case.public, &case.route, header, &body, None),
        )
        .seal()
        .unwrap();
        assert_eq!(opened_reply(&case.public, header, &body, &reply).len(), 3);
        assert_eq!(case.selected.resources().counts(), before);
        assert!(row(&case, id).status().unwrap() == initial);
    }
    // Exact Return intentionally closes uncertain local Pending preparation. It cannot
    // manufacture an Action, settlement or drained Phase, including after End/lock.
    case.native
        .state
        .session_lock_requested
        .store(true, Ordering::SeqCst);
    case.native
        .state
        .voice_authority
        .lock()
        .unwrap()
        .end(case.selected.id());
    let mut body = prefix.clone();
    body.push(0);
    for sequence in [3, 4] {
        let header = (sequence, 6, 0, id.logical());
        let reply = crate::voice_handoff::confirmation_settlement::apply(
            &case.context,
            &case.route,
            opened_frame(&case.public, &case.route, header, &body, None),
        )
        .seal()
        .unwrap();
        let status = row(&case, id).status().unwrap();
        assert!(matches!(
            status.pending,
            super::super::ledger::PendingStatus::Closed {
                actor_handoff: true
            }
        ));
        assert!(status.action.is_none() && !status.engine.settled && !status.drained);
        assert_eq!(
            opened_reply(&case.public, header, &body, &reply),
            super::super::replies::status(id, &status).unwrap()
        );
        assert!(case.selected.resources().counts().0 <= before.0);
    }
    close(case, id).await;
}
