//! Exact original-row terminal metadata. The existing fixture uses actual protected Root
//! actions and signed discovery, but synthetic desktop grants: no normal Actor/MLS claim.
use super::super::super::custody::CloseMode;
use super::*;

// Independently construct the exact deterministic name BEFORE native issuance. Knowing
// these bytes is deliberately insufficient: ACK still requires stored original issuance.
fn predicted_proof(row: &ResolvedRow, session: &SessionIdentity) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let (serial, version, input) = row.status().unwrap().action.unwrap();
    let Purpose::Derive(selector) = row.row.key.purpose else {
        panic!("this proof fixture requires the original Derive parent")
    };
    let mut hash = Sha256::new();
    hash.update(b"Mewtual native voice final retirement v1\0");
    hash.update(session.0);
    hash.update(row.id().logical());
    hash.update([0]);
    hash.update(selector.peer.as_bytes());
    hash.update(selector.join);
    hash.update(selector.key_ref);
    hash.update(selector.epoch.to_be_bytes());
    hash.update(serial.to_be_bytes());
    hash.update(version.to_be_bytes());
    hash.update(input);
    hash.update([1]);
    hash.finalize().into()
}

#[test]
fn terminal_intent_rejects_an_equal_hash_foreign_session_before_and_after_issuance() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (join, action, pair) = actual_action(&coordinator, &selected);
    let id = ticket(&ledger, selected.clone(), selector)
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    let first = FixtureChannel::new(selected.resources(), selected.binding().scope.group);
    let foreign = FixtureChannel::new(selected.resources(), selected.binding().scope.group);
    assert_eq!(first.session.identity().0, foreign.session.identity().0);
    assert!(!Arc::ptr_eq(
        first.session.identity(),
        foreign.session.identity()
    ));
    action.settle_engine();
    row.close_phase(first.session.identity(), CloseMode::Retire)
        .unwrap();
    assert_eq!(
        row.close_phase(foreign.session.identity(), CloseMode::Retire),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        row.issue_final(foreign.session.identity()),
        Err(VoiceError::Unauthorized)
    );
    assert!(row.try_compact().unwrap());
    drop((join, action));
    let original = row.issue_final(first.session.identity()).unwrap();
    assert!(original.proof.is_some());
    assert_eq!(
        row.acknowledge_final(foreign.session.identity(), original.proof.unwrap()),
        Err(VoiceError::Unauthorized)
    );
    assert!(!ledger.channel_may_close(&selected));
    let acked = row
        .acknowledge_final(first.session.identity(), original.proof.unwrap())
        .unwrap();
    assert!(acked.acknowledged && ledger.channel_may_close(&selected));
    assert_eq!(row.issue_final(first.session.identity()).unwrap(), acked);
}

#[test]
fn unissued_guess_and_other_row_proof_never_acknowledge_or_close_the_channel() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let id = ticket(&ledger, selected.clone(), selector)
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    let endpoint = FixtureChannel::new(selected.resources(), selected.binding().scope.group);
    action.settle_engine();
    row.close_phase(endpoint.session.identity(), CloseMode::Retire)
        .unwrap();
    assert!(row.try_compact().unwrap());
    drop(action);
    let exact_but_unissued = predicted_proof(&row, endpoint.session.identity());
    assert_eq!(
        row.acknowledge_final(endpoint.session.identity(), exact_but_unissued),
        Err(VoiceError::Unauthorized)
    );
    assert!(row
        .row
        .state
        .lock()
        .unwrap()
        .final_delivery()
        .unwrap()
        .issued
        .is_none());
    let issued = row.issue_final(endpoint.session.identity()).unwrap();
    assert_eq!(issued.proof, Some(exact_but_unissued));
    // A genuinely different native row's issued proof cannot ACK this original row,
    // even when the ACK uses this row's correct Session. Both fixtures use real actions.
    let (other_coordinator, other_selected, other_selector) = selection();
    let second = Arc::new(Ledger::default());
    let mut dummy_selector = other_selector;
    dummy_selector.epoch += 1;
    let unused = ticket(&second, other_selected.clone(), dummy_selector);
    unused.mark_handoff().unwrap();
    drop(unused);
    let (_other_join, other_action, other_pair) =
        actual_action(&other_coordinator, &other_selected);
    let other_id = ticket(&second, other_selected.clone(), other_selector)
        .attach(other_action.clone(), other_pair, 16 * 1024)
        .unwrap();
    assert_ne!(other_id, id);
    let other_row = second
        .resolve(&other_selected, Purpose::Derive(other_selector), other_id)
        .unwrap();
    let other_endpoint = FixtureChannel::new(
        other_selected.resources(),
        other_selected.binding().scope.group,
    );
    other_action.settle_engine();
    other_row
        .close_phase(other_endpoint.session.identity(), CloseMode::Retire)
        .unwrap();
    assert!(other_row.try_compact().unwrap());
    drop(other_action);
    let other = other_row
        .issue_final(other_endpoint.session.identity())
        .unwrap();
    assert_ne!(other.proof, issued.proof);
    assert_eq!(
        row.acknowledge_final(endpoint.session.identity(), other.proof.unwrap()),
        Err(VoiceError::Unauthorized)
    );
    let mut foreign = issued.proof.unwrap();
    foreign[0] ^= 1;
    assert_eq!(
        row.acknowledge_final(endpoint.session.identity(), foreign),
        Err(VoiceError::Unauthorized)
    );
    assert!(!ledger.channel_may_close(&selected));
    assert_eq!(
        row.issue_final(endpoint.session.identity()).unwrap(),
        issued
    );
}

#[test]
fn orphan_and_reused_slots_cannot_latch_intent_issue_or_ack_on_old_metadata() {
    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let id = registration.id();
    let orphan = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    drop(registration);
    let replacement = ticket(&ledger, selected.clone(), selector);
    assert_ne!(replacement.id().serial, id.serial);
    let endpoint = FixtureChannel::new(selected.resources(), selected.binding().scope.group);
    for mode in [CloseMode::Return, CloseMode::Retire] {
        assert_eq!(
            orphan.close_phase(endpoint.session.identity(), mode),
            Err(VoiceError::Unauthorized)
        );
    }
    assert_eq!(
        orphan.issue_final(endpoint.session.identity()),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        orphan.acknowledge_final(endpoint.session.identity(), [0; 32]),
        Err(VoiceError::Unauthorized)
    );
    assert!(orphan.row.state.lock().unwrap().final_delivery().is_none());
}

#[test]
fn closure_modes_remain_distinct_while_copies_are_delayed_and_after_compaction() {
    for first_mode in [CloseMode::Return, CloseMode::Retire] {
        let (coordinator, selected, selector) = selection();
        let ledger = Arc::new(Ledger::default());
        let (_join, action, pair) = actual_action(&coordinator, &selected);
        let id = ticket(&ledger, selected.clone(), selector)
            .attach(action.clone(), pair, 16 * 1024)
            .unwrap();
        let row = ledger
            .resolve(&selected, Purpose::Derive(selector), id)
            .unwrap();
        let work = row.work().unwrap();
        let copy = work.phase().buffer(1).unwrap();
        let endpoint = FixtureChannel::new(selected.resources(), selected.binding().scope.group);
        action.settle_engine();
        assert_eq!(
            row.close_phase(endpoint.session.identity(), first_mode),
            Err(VoiceError::Capacity)
        );
        drop(work);
        row.close_phase(endpoint.session.identity(), first_mode)
            .unwrap();
        let second_mode = if first_mode == CloseMode::Return {
            CloseMode::Retire
        } else {
            CloseMode::Return
        };
        assert_eq!(
            row.close_phase(endpoint.session.identity(), second_mode),
            Err(VoiceError::Unauthorized)
        );
        assert!(!row.try_compact().unwrap());
        drop(copy);
        assert!(row.try_compact().unwrap());
        row.close_phase(endpoint.session.identity(), first_mode)
            .unwrap();
        assert_eq!(
            row.close_phase(endpoint.session.identity(), second_mode),
            Err(VoiceError::Unauthorized)
        );
        if first_mode == CloseMode::Return {
            assert_eq!(
                row.issue_final(endpoint.session.identity()),
                Err(VoiceError::Unauthorized)
            );
        }
    }
}
