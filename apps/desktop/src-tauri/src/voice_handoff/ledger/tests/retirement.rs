//! Native history owns the SAME paid Phase after local compaction. These actual protected
//! actions use the existing synthetic desktop lifecycle fixture, not live Actor/WebView flow.
use super::*;

#[test]
fn compacted_final_history_retains_original_proof_until_actual_root_input_drops() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (join, action, pair) = actual_action(&coordinator, &selected);
    // Deliberately outside Phase's local copy count: local16KiB return cannot observe this.
    let root_input = action.input().unwrap();
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    let work = row.work().unwrap();
    let phase = work.phase().clone();
    let original = Arc::downgrade(&phase);
    action.settle_engine();
    phase.close_confirmation().unwrap();
    assert_eq!(
        row.confirmation_status().unwrap(),
        ConfirmationStatus::Closing
    );
    drop(work);
    assert!(phase.is_drained());
    assert_eq!(
        row.confirmation_status().unwrap(),
        ConfirmationStatus::Draining
    );

    let pool = selected.resources();
    let group = selected.binding().scope.group;
    let pressure = pool
        .charge(group, 256 * 1024 - pool.counts().0, ResourceKind::Retained)
        .unwrap();
    let full = pool.counts();
    assert!(ledger.compact(id).unwrap());
    assert!(
        row.status().unwrap().drained,
        "only LOCAL compaction is complete"
    );
    assert_eq!(
        row.confirmation_status().unwrap(),
        ConfirmationStatus::Draining
    );
    assert!(
        !ledger.channel_may_close(&selected),
        "final proof still needs Control"
    );
    for _ in 0..32 {
        assert!(ledger.compact(id).unwrap());
        assert_eq!(
            row.confirmation_status().unwrap(),
            ConfirmationStatus::Draining
        );
    }
    assert_eq!(
        pool.counts(),
        full,
        "snapshots/compaction need no fresh capacity"
    );
    drop((phase, action, join));
    assert!(
        original.upgrade().is_some(),
        "Row owns the original paid Phase"
    );
    assert_eq!(
        row.confirmation_status().unwrap(),
        ConfirmationStatus::Draining
    );
    drop(root_input);
    // Root sends no Row notification. A retained snapshot sees actual last-owner retirement.
    assert_eq!(
        row.confirmation_status().unwrap(),
        ConfirmationStatus::Drained
    );
    assert!(ledger.channel_may_close(&selected));
    drop(pressure);
}

#[test]
fn already_retired_compaction_keeps_the_same_original_phase() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (join, action, pair) = actual_action(&coordinator, &selected);
    let id = ticket(&ledger, selected.clone(), selector)
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    let work = row.work().unwrap();
    let phase = work.phase().clone();
    action.settle_engine();
    phase.close_confirmation().unwrap();
    drop(work);
    drop((join, action));
    assert_eq!(phase.confirmation_status(), ConfirmationStatus::Drained);
    // The separate held-input regression pins compaction WHILE Root remains unready. This
    // happy path must preserve the SAME original proof when Root has already retired.
    assert!(ledger.compact(id).unwrap());
    assert_eq!(
        row.confirmation_status().unwrap(),
        ConfirmationStatus::Drained
    );
    assert!(ledger.channel_may_close(&selected));
    let state = row.row.state.lock().unwrap();
    assert!(Arc::ptr_eq(state.final_phase.as_ref().unwrap(), &phase));
}

#[test]
fn ordinary_compaction_drops_phase_and_never_invents_final_retirement() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (join, action, pair) = actual_action(&coordinator, &selected);
    let id = ticket(&ledger, selected.clone(), selector)
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    let work = row.work().unwrap();
    let phase = work.phase().clone();
    let original = Arc::downgrade(&phase);
    action.settle_engine();
    phase.close();
    drop(work);
    assert!(ledger.compact(id).unwrap());
    assert_eq!(
        row.confirmation_status().unwrap(),
        ConfirmationStatus::NotRequested
    );
    assert!(ledger.channel_may_close(&selected));
    assert!(row.row.state.lock().unwrap().final_phase.is_none());
    drop((phase, action, join));
    assert!(
        original.upgrade().is_none(),
        "ordinary continuation retains no final proof"
    );
    assert_eq!(
        row.confirmation_status().unwrap(),
        ConfirmationStatus::NotRequested
    );
}

#[test]
fn concurrent_compaction_keeps_same_original_phase_without_replacing_proof() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (join, action, pair) = actual_action(&coordinator, &selected);
    let root_input = action.input().unwrap();
    let id = ticket(&ledger, selected.clone(), selector)
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let work = ledger.work(id).unwrap();
    let phase = work.phase().clone();
    action.settle_engine();
    phase.close_confirmation().unwrap();
    drop(work);
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            barrier.wait();
            ledger.compact(id)
        });
        let b = scope.spawn(|| {
            barrier.wait();
            ledger.compact(id)
        });
        // The existing sole-observation gate may temporarily refuse the other contender.
        for result in [a.join().unwrap(), b.join().unwrap()] {
            assert!(matches!(result, Ok(true) | Err(VoiceError::Capacity)));
        }
    });
    assert!(ledger.compact(id).unwrap());
    {
        let row = ledger.row(id).unwrap();
        let state = row.state.lock().unwrap();
        assert!(Arc::ptr_eq(state.final_phase.as_ref().unwrap(), &phase));
    }
    assert_eq!(phase.confirmation_status(), ConfirmationStatus::Draining);
    drop((root_input, join, action));
    assert_eq!(phase.confirmation_status(), ConfirmationStatus::Drained);
}

#[test]
fn pending_closed_and_removed_native_rows_cannot_report_root_retirement() {
    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let id = registration.id();
    let orphan = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    assert_eq!(
        orphan.confirmation_status().unwrap(),
        ConfirmationStatus::NotRequested
    );
    drop(registration); // proven unstarted removal, despite a late metadata owner
    assert_eq!(orphan.confirmation_status(), Err(VoiceError::Unauthorized));
    let replacement = ticket(&ledger, selected.clone(), selector);
    assert_ne!(replacement.id().serial, id.serial);
    assert_eq!(orphan.confirmation_status(), Err(VoiceError::Unauthorized));
    replacement.mark_handoff().unwrap();
    let current = ledger
        .resolve(&selected, Purpose::Derive(selector), replacement.id())
        .unwrap();
    drop(replacement);
    let closed = current.close_pending().unwrap();
    assert!(!closed.drained && !closed.engine.settled);
    assert_eq!(
        current.confirmation_status().unwrap(),
        ConfirmationStatus::NotRequested
    );
    assert!(ledger.channel_may_close(&selected));
}
