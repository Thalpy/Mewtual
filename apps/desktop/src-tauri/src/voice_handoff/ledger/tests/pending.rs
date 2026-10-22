use super::*;

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
