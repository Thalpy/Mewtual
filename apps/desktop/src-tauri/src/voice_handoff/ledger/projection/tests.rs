use super::*;
use crate::voice_handoff::ledger::tests::{actual_action, selection};

#[test]
fn confirmation_terminal_oracle_retries_actual_row_contention_without_claiming_completion() {
    use crate::voice_handoff::integration::webcrypto::automatic_terminal;
    use std::sync::mpsc;

    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let Begin::Factory(registration) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("new original Pending")
    };
    let id = registration.id();
    let row = ledger.index.lock().unwrap().rows[id.slot]
        .as_ref()
        .unwrap()
        .clone();
    let before = selected.resources().counts();
    let held = row.state.lock().unwrap();
    let (observed, observation) = mpsc::channel();
    let worker_ledger = ledger.clone();
    let worker_selected = selected.clone();
    let worker = std::thread::spawn(move || {
        observed
            .send(automatic_terminal(&worker_ledger, &worker_selected, None))
            .unwrap();
    });
    // The actual parent mutex, rather than a fabricated Capacity result, drives the
    // refusal. The watchdog releases it before joining even if a regression waits.
    let result = observation.recv_timeout(std::time::Duration::from_secs(5));
    drop(held);
    worker.join().unwrap();
    assert!(!result.expect("passive oracle must refuse actual row contention"));
    assert!(!automatic_terminal(&ledger, &selected, None));
    assert_eq!(selected.resources().counts(), before);
    assert_eq!(
        registration.id(),
        id,
        "observation cannot replace original work"
    );
}

#[test]
fn work_snapshot_is_original_noncreating_and_revisits_pending_attachment_in_same_slot() {
    let (mut coordinator, selected, selector) = selection();
    let (_foreign_coordinator, foreign, _) = selection();
    let ledger = Arc::new(Ledger::default());
    assert!(ledger
        .project_work(&selected, 0)
        .unwrap()
        .metadata
        .is_none());
    let Begin::Factory(registration) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("new native Pending")
    };
    let id = registration.id();
    let pending = ledger.project_work(&selected, 0).unwrap();
    assert_eq!(pending.metadata.as_ref().unwrap().row, id);
    assert!(pending.metadata.as_ref().unwrap().pending == PendingStatus::Open);
    assert!(pending.metadata.as_ref().unwrap().action.is_none());
    assert!(
        ledger.project_work(&foreign, 0).unwrap().metadata.is_none(),
        "equal labels cannot select another original grant"
    );
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    assert_eq!(registration.attach(action, pair, 16 * 1024).unwrap(), id);
    assert!(
        pending.metadata.as_ref().unwrap().pending == PendingStatus::Open,
        "old snapshot stays immutable"
    );
    let attached = ledger.project_work(&selected, 0).unwrap();
    assert_eq!(attached.metadata.as_ref().unwrap().row, id);
    assert!(attached.metadata.as_ref().unwrap().pending == PendingStatus::NotPending);
    assert!(attached.metadata.as_ref().unwrap().action.is_some());
    assert!(ledger
        .project_work(&selected, attached.next_slot)
        .unwrap()
        .metadata
        .is_none());
    coordinator.end(selected.id());
    let ended = ledger.project_work(&selected, 0).unwrap();
    assert_eq!(
        ended.metadata.as_ref().unwrap().row,
        id,
        "historical cleanup survives End without a grant upgrade"
    );
    assert!(ledger.project_work(&selected, 33).is_err());
}

#[test]
fn unseen_pending_closure_is_observed_without_inventing_attached_action_or_drainage() {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let Begin::Factory(registration) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("native Pending")
    };
    let id = registration.id();
    registration.mark_handoff().unwrap();
    drop(registration); // Native production reply was lost before the browser ever saw it.
    coordinator.end(selected.id());
    let pending = ledger.project_work(&selected, 0).unwrap();
    assert!(pending.metadata.as_ref().unwrap().pending == PendingStatus::Open);
    let original = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    let closed = original.close_pending().unwrap();
    assert!(
        closed.pending
            == PendingStatus::Closed {
                actor_handoff: true
            }
    );
    let projected = ledger.project_work(&selected, 0).unwrap();
    assert!(projected.metadata.as_ref().unwrap().pending == closed.pending);
    assert!(projected.metadata.as_ref().unwrap().action.is_none());
    assert!(
        !closed.drained,
        "local Pending closure does not attest uncertain Root/SDK retirement"
    );
    assert!(pending.metadata.as_ref().unwrap().pending == PendingStatus::Open);
}

#[test]
fn an_old_snapshot_keeps_its_identity_when_an_unstarted_slot_is_reused() {
    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let Begin::Factory(original) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("original Pending")
    };
    let old_id = original.id();
    let delayed = ledger.project_work(&selected, 0).unwrap();
    // Dropping a proven-unstarted registration really removes it. The delayed public
    // snapshot owns its old paid Row, but cannot prevent safe reuse of the index slot.
    drop(original);
    let Begin::Factory(replacement) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("replacement Pending")
    };
    let new_id = replacement.id();
    assert_eq!(old_id.slot, new_id.slot);
    assert_ne!(old_id.serial, new_id.serial);
    assert_eq!(delayed.metadata.as_ref().unwrap().row, old_id);
    assert!(ledger
        .resolve(&selected, Purpose::Derive(selector), old_id)
        .is_err());
    assert!(ledger
        .project_work(&selected, delayed.next_slot)
        .unwrap()
        .metadata
        .is_none());
    // A continuation slot is not a replay cursor. Rescanning is required to see a new
    // original serial, just as it is required for Pending -> attached in the same row.
    let rescanned = ledger.project_work(&selected, 0).unwrap();
    assert_eq!(rescanned.metadata.as_ref().unwrap().row, new_id);
    assert!(rescanned.metadata.as_ref().unwrap().action.is_none());
}
