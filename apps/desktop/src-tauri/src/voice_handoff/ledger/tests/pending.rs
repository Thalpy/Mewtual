use super::*;

#[test]
fn an_already_compacted_root_input_still_leaves_the_same_known_publication_for_cleanup() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    action.settle_engine();
    assert!(action.input().is_err());
    let registration = ticket(&ledger, selected.clone(), selector);
    let id = registration.id();
    assert!(matches!(
        registration.attach(action.clone(), pair, 16 * 1024),
        Err(VoiceError::Unauthorized)
    ));
    let retry = ticket(&ledger, selected.clone(), selector);
    assert!(retry.id() == id && retry.has_publication().unwrap());
    assert!(matches!(
        retry.attach_published(16 * 1024),
        Err(VoiceError::Unauthorized)
    ));
    let row = ledger.row(id).unwrap();
    {
        let state = row.state.lock().unwrap();
        let Body::Pending(pending) = &state.body else {
            panic!("known Pending")
        };
        assert!(Arc::ptr_eq(pending.published.as_ref().unwrap(), &action));
        assert!(pending.prepared.is_some() && !pending.worker);
    }
    let closed = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap()
        .close_pending()
        .unwrap();
    assert!(matches!(
        closed.pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    assert!(!closed.drained && closed.action.is_none());
}

#[test]
fn actual_publication_and_preparation_survive_busy_loan_then_attach_under_full_pool_pressure() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let id = registration.id();
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    // This is the ACTUAL original loan. Until its last owner returns, a native adapter
    // must preserve the already-published action rather than calling the Actor again.
    let prior_loan = action.borrow_handoff(16 * 1024).unwrap();
    assert!(matches!(
        registration.attach(action.clone(), pair, 16 * 1024),
        Err(VoiceError::Capacity)
    ));
    {
        let row = ledger.row(id).unwrap();
        let state = row.state.lock().unwrap();
        let Body::Pending(pending) = &state.body else {
            panic!("retained Pending")
        };
        assert!(Arc::ptr_eq(pending.published.as_ref().unwrap(), &action));
        assert!(
            pending.prepared.is_some(),
            "same fixed metadata survives refusal"
        );
        assert!(!pending.worker && pending.handed_off);
    }
    let pool = selected.resources();
    let group = selected.binding().scope.group;
    let pressure = pool
        .charge(
            group,
            catcoms_sync::voice_v2::resources::GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let full = pool.counts();
    let retry = ticket(&ledger, selected.clone(), selector);
    assert!(retry.id() == id);
    assert!(retry.has_publication().unwrap());
    assert_eq!(pool.counts(), full, "no replacement preparation payment");
    drop(prior_loan);
    assert!(retry.attach_published(16 * 1024).unwrap() == id);
    assert_eq!(pool.counts(), full, "original Workflow funds attachment");
    let work = ledger.work(id).unwrap();
    assert!(Arc::ptr_eq(work.action(), &action));
    drop((work, pressure));
}

#[test]
fn a_foreign_equal_metadata_publication_cannot_replace_the_retained_original() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let prior_loan = action.borrow_handoff(16 * 1024).unwrap();
    let registration = ticket(&ledger, selected.clone(), selector);
    let id = registration.id();
    assert!(matches!(
        registration.attach(action.clone(), pair, 16 * 1024),
        Err(VoiceError::Capacity)
    ));
    // A second LocalJoin cannot bind the SAME live call lease. Use a real independent
    // native owner with equal public binding instead; its lease/pool remain foreign.
    let (other_coordinator, other_selected, _) = selection();
    let (_other_join, other, other_pair) = actual_action(&other_coordinator, &other_selected);
    assert!(!Arc::ptr_eq(&action, &other));
    assert!(action.protected_binding().unwrap() == other.protected_binding().unwrap());
    assert!(!action.same_protected_plan(&other));
    assert!(matches!(
        ticket(&ledger, selected.clone(), selector).attach(other, other_pair, 16 * 1024),
        Err(VoiceError::Unauthorized)
    ));
    drop(prior_loan);
    let retry = ticket(&ledger, selected, selector);
    assert!(retry.attach_published(16 * 1024).unwrap() == id);
    assert!(Arc::ptr_eq(ledger.work(id).unwrap().action(), &action));
}

#[test]
fn end_after_publication_cannot_revive_attachment_or_turn_local_closure_into_root_proof() {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let prior_loan = action.borrow_handoff(16 * 1024).unwrap();
    let registration = ticket(&ledger, selected.clone(), selector);
    let id = registration.id();
    assert!(matches!(
        registration.attach(action.clone(), pair, 16 * 1024),
        Err(VoiceError::Capacity)
    ));
    coordinator.end(selected.id());
    drop(prior_loan);
    assert!(matches!(
        ledger.begin(selected.clone(), Purpose::Derive(selector)),
        Err(VoiceError::Unauthorized)
    ));
    let original = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    let closed = original.close_pending().unwrap();
    assert!(matches!(
        closed.pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    assert!(closed.action.is_none() && !closed.drained && !closed.engine.settled);
    assert!(matches!(
        original.confirmation_status().unwrap(),
        ConfirmationStatus::NotRequested
    ));
    assert!(matches!(ledger.work(id), Err(VoiceError::Unauthorized)));
    // The other real owner is deliberate: local closure asserts nothing about this action,
    // the still-retained Root pair, an SDK engine, or whole-plan retirement.
    assert!(action.protected_plan().is_ok());
}

#[test]
fn orphaned_unstarted_handle_can_never_close_or_mutate_history_after_slot_reuse() {
    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let unstarted = ticket(&ledger, selected.clone(), selector);
    let old = ledger
        .resolve(&selected, Purpose::Derive(selector), unstarted.id())
        .unwrap();
    let old_id = old.id();
    drop(unstarted); // proven unstarted row leaves Index despite old metadata ownership
    let replacement = ticket(&ledger, selected.clone(), selector);
    let fresh_id = replacement.id();
    assert!(fresh_id != old_id);
    for _ in 0..3 {
        assert!(matches!(old.close_pending(), Err(VoiceError::Unauthorized)));
        assert!(matches!(old.status(), Err(VoiceError::Unauthorized)));
        assert!(matches!(
            old.row.state.lock().unwrap().status.pending,
            PendingStatus::Open
        ));
        assert!(matches!(
            ledger.status(fresh_id).unwrap().pending,
            PendingStatus::Open
        ));
    }
    replacement.mark_handoff().unwrap();
    drop(replacement);
    let fresh = ledger
        .resolve(&selected, Purpose::Derive(selector), fresh_id)
        .unwrap();
    assert!(matches!(
        fresh.close_pending().unwrap().pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    assert!(matches!(old.close_pending(), Err(VoiceError::Unauthorized)));
}

#[test]
fn automatic_discovery_waits_for_registered_parent_but_preserves_original_pending_retry() {
    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    assert!(ledger.automatic_parent_available(&selected).unwrap());
    let registration = ticket(&ledger, selected.clone(), selector);
    let id = registration.id();
    assert!(!ledger.automatic_parent_available(&selected).unwrap());
    drop(registration); // Proven unstarted: no Root handoff, actual row is unlinked.
    assert!(ledger.automatic_parent_available(&selected).unwrap());
    let registration = ticket(&ledger, selected.clone(), selector);
    let id2 = registration.id();
    assert_ne!(id, id2);
    registration.mark_handoff().unwrap();
    drop(registration);
    assert!(!ledger.automatic_parent_available(&selected).unwrap());
    let retry = ticket(&ledger, selected.clone(), selector);
    assert_eq!(
        retry.id(),
        id2,
        "same original Pending resumes, never another parent"
    );
    drop(retry);
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id2)
        .unwrap();
    row.close_pending().unwrap();
    assert!(
        !ledger.automatic_parent_available(&selected).unwrap(),
        "ClosedPending is not Root retirement and cannot admit unrelated work"
    );
}

#[test]
fn automatic_discovery_refuses_collection_ticket_even_when_index_has_no_parent() {
    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    drop(registration);
    let held = ledger.test_hold_collection().unwrap();
    assert!(matches!(
        ledger.automatic_parent_available(&selected),
        Err(VoiceError::Capacity)
    ));
    drop(held);
    assert!(ledger.automatic_parent_available(&selected).unwrap());
    let index = ledger.index.lock().unwrap();
    assert!(
        matches!(
            ledger.automatic_parent_available(&selected),
            Err(VoiceError::Capacity)
        ),
        "index contention must never be classified as available capacity"
    );
    drop(index);
}

#[test]
fn automatic_discovery_ignores_a_different_original_selection() {
    let (_first_coordinator, first, selector) = selection();
    let (_second_coordinator, second, _) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, first.clone(), selector);
    assert!(!ledger.automatic_parent_available(&first).unwrap());
    assert!(
        ledger.automatic_parent_available(&second).unwrap(),
        "equal public labels do not identify the same original selected allocation"
    );
    drop(registration);
}
