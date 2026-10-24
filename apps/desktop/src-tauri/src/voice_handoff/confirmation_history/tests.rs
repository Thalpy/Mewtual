//! Real AEAD frames and original native parent/child custody. Pending fixtures deliberately
//! do not claim a successful provider or Actor result; positive result recovery is also
//! exercised by the permitted-TCP confirmation fixture.
use super::super::custody::CloseMode;
use super::super::ledger::{Begin, ConfirmationPurpose, PendingStatus};
use super::super::service::tests::{opened_frame, opened_reply};
use super::*;

type Case = super::super::derivation_result::tests::Case;

async fn pending() -> (Case, RowId) {
    let case = Case::unadmitted().await;
    let parent = case.row();
    parent.settle().unwrap();
    parent
        .close_phase(case.route.session().identity(), CloseMode::Return)
        .unwrap();
    assert!(parent.try_compact().unwrap());
    let (Begin::Factory(child), trigger) = case
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
        panic!("first child")
    };
    assert!(trigger.is_none());
    child.mark_handoff().unwrap();
    let id = child.id();
    drop(child); // original uncertain publication, not absence evidence
    (case, id)
}
fn bytes(case: &Case) -> Vec<u8> {
    let mut body = vec![2];
    body.extend_from_slice(case.selector.peer.as_bytes());
    body.extend_from_slice(&case.selector.join);
    body.extend_from_slice(&case.selector.key_ref);
    body.extend_from_slice(&case.selector.epoch.to_be_bytes());
    body.extend_from_slice(&case.id.logical());
    body.push(0); // Start(None)
    body.push(0); // Begin(false)
    body
}
fn query(case: &Case, sequence: u64, operation: u8, logical: [u8; 16], body: &[u8]) -> Vec<u8> {
    let incoming = opened_frame(
        &case.public,
        &case.route,
        (sequence, operation, 0, logical),
        body,
        None,
    );
    let reply = recover(&case.context, &case.route, incoming)
        .seal()
        .unwrap();
    opened_reply(
        &case.public,
        (sequence, operation, 0, logical),
        body,
        &reply,
    )
}

async fn close(case: Case) {
    let parent = case.row();
    case.native
        .state
        .voice_handoff
        .lookup_child(
            &case.selected,
            &parent,
            ConfirmationPurpose::Start(None),
            None,
        )
        .unwrap()
        .close_pending()
        .unwrap();
    // The fixture's old Phase is already Return-closed. Cleanup must use the original
    // parent's separate control, never relabel that immutable Return as Phase Retire.
    parent.retire_plan().unwrap();
    case.native.close().await;
}

#[tokio::test]
async fn unknown_wrong_parent_selector_purpose_header_retry_or_opcode_cannot_reserve_history() {
    let (case, id) = pending().await;
    let original = bytes(&case);
    let before = case.selected.resources().counts();
    let mut malformed = Vec::new();
    for offset in [1, 33, 65, 97, 105, 120, 122] {
        let mut changed = original.clone();
        changed[offset] ^= 1;
        malformed.push(changed);
    }
    let mut wrong_purpose = original.clone();
    wrong_purpose.splice(121..122, [3, 2]); // Incoming(2) is not Start(None)
    malformed.push(wrong_purpose);
    for (i, body) in malformed.iter().enumerate() {
        let reply = query(&case, i as u64 + 1, 8, [0; 16], body);
        assert_eq!(
            reply.len(),
            3,
            "no historical success from a changed identity"
        );
        assert_eq!(case.selected.resources().counts(), before);
    }
    for (seq, operation, logical) in [(20, 8, id.logical()), (21, 1, id.logical())] {
        assert_eq!(query(&case, seq, operation, logical, &original).len(), 3);
    }
    let expected = super::super::replies::status(
        id,
        &case
            .native
            .state
            .voice_handoff
            .lookup_child(
                &case.selected,
                &case.row(),
                ConfirmationPurpose::Start(None),
                Some(id),
            )
            .unwrap()
            .status()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(query(&case, 22, 8, [0; 16], &original), expected);
    assert_eq!(case.selected.resources().counts(), before);
    close(case).await;
}

#[tokio::test]
async fn original_pending_and_closed_pending_recover_after_end_lock_and_full_ordinary_capacity() {
    use catcoms_sync::voice_v2::resources::ResourceKind;
    use std::sync::atomic::Ordering;
    let (case, id) = pending().await;
    let row = case
        .native
        .state
        .voice_handoff
        .lookup_child(
            &case.selected,
            &case.row(),
            ConfirmationPurpose::Start(None),
            Some(id),
        )
        .unwrap();
    let body = bytes(&case);
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
    let pool = case.selected.resources();
    let group = case.selected.binding().scope.group;
    let mut pressure = Vec::new();
    for shift in (0..=18).rev() {
        if let Ok(credit) = pool.charge(group, 1 << shift, ResourceKind::Retained) {
            pressure.push(credit);
        }
    }
    assert!(pool.charge(group, 1, ResourceKind::Retained).is_err());
    let full = pool.counts();
    let status = row.status().unwrap();
    assert!(matches!(status.pending, PendingStatus::Open));
    assert_eq!(
        query(&case, 1, 8, [0; 16], &body),
        super::super::replies::status(id, &status).unwrap()
    );
    let closed = row.close_pending().unwrap();
    assert!(matches!(
        closed.pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    assert!(!closed.drained && !closed.engine.settled && closed.engine.receipt.is_none());
    let after_close = pool.counts();
    assert!(after_close.0 <= full.0);
    assert_eq!(
        query(&case, 2, 8, [0; 16], &body),
        super::super::replies::status(id, &closed).unwrap()
    );
    assert_eq!(pool.counts(), after_close);
    drop(pressure);
    close(case).await;
}

#[tokio::test]
async fn equal_labels_foreign_session_refuses_and_uncollected_reply_keeps_original_position() {
    let (case, id) = pending().await;
    let (foreign, _) = pending().await;
    let body = bytes(&case);
    let foreign_incoming = opened_frame(
        &foreign.public,
        &foreign.route,
        (1, 8, 0, [0; 16]),
        &body,
        None,
    );
    let reply = recover(&case.context, &case.route, foreign_incoming)
        .seal()
        .unwrap();
    let refused = opened_reply(&foreign.public, (1, 8, 0, [0; 16]), &body, &reply);
    assert_eq!(refused.len(), 3);
    drop(reply);
    let incoming = opened_frame(&case.public, &case.route, (1, 8, 0, [0; 16]), &body, None);
    let response = recover(&case.context, &case.route, incoming);
    assert!(case.route.session().header().is_err());
    let reply = response.seal().unwrap();
    assert!(case.route.session().header().is_err());
    let status = case
        .native
        .state
        .voice_handoff
        .lookup_child(
            &case.selected,
            &case.row(),
            ConfirmationPurpose::Start(None),
            Some(id),
        )
        .unwrap()
        .status()
        .unwrap();
    assert_eq!(
        opened_reply(&case.public, (1, 8, 0, [0; 16]), &body, &reply),
        super::super::replies::status(id, &status).unwrap()
    );
    drop(reply);
    drop(case.route.session().header().unwrap());
    close(case).await;
    close(foreign).await;
}
