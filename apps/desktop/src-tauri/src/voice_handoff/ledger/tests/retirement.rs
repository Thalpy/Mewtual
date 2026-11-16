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
        .charge(
            group,
            catcoms_sync::voice_v2::resources::GROUP_BYTES - pool.group_counts(group).unwrap().0,
            ResourceKind::Retained,
        )
        .unwrap();
    assert_eq!(
        pool.group_counts(group).unwrap().0,
        catcoms_sync::voice_v2::resources::GROUP_BYTES
    );
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
    assert!(
        !ledger.channel_may_close(&selected),
        "Root retirement alone is not final delivery"
    );
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
    assert!(
        !ledger.channel_may_close(&selected),
        "no original Session final issuance/ACK"
    );
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
    assert!(
        !ledger.channel_may_close(&selected),
        "ordinary Return is not whole-plan retirement"
    );
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
    assert!(
        !ledger.channel_may_close(&selected),
        "actual LocalJoin drop retires Root, but original final delivery still needs Control"
    );
}

#[test]
fn original_plan_anchor_survives_compaction_and_cleanup_between_children_waits_for_real_input() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (join, action, pair) = actual_action(&coordinator, &selected);
    let root_input = action.input().unwrap();
    let id = ticket(&ledger, selected.clone(), selector)
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    assert!(row.contains_plan_action(&action).unwrap());
    assert_eq!(
        row.retire_plan(),
        Err(VoiceError::Capacity),
        "unsettled work cannot close the plan"
    );
    let work = row.work().unwrap();
    action.settle_engine();
    work.phase().close(); // immutable ordinary Return, with NO successor child
    drop(work);
    assert!(ledger.compact(id).unwrap());
    assert!(
        row.contains_plan_action(&action).unwrap(),
        "same opaque plan remains after parent compaction"
    );
    assert!(
        !row.retire_plan().unwrap(),
        "real extra Root input outlives native local compaction"
    );
    assert!(
        !row.contains_plan_action(&action).unwrap(),
        "new child adoption is frozen before close"
    );
    assert!(!ledger.channel_may_close(&selected));
    drop((join, action));
    assert!(
        !row.retire_plan().unwrap(),
        "lost caller recovers the SAME pending retirement"
    );
    drop(root_input);
    assert!(row.retire_plan().unwrap());
    assert!(
        row.retire_plan().unwrap(),
        "repeated cleanup never captures a replacement"
    );
    assert!(
        !ledger.channel_may_close(&selected),
        "Root drainage leaves final delivery outstanding"
    );
    assert_eq!(
        row.confirmation_status().unwrap(),
        ConfirmationStatus::NotRequested,
        "whole-plan close never relabels the action Phase Return as Retire"
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
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            barrier.wait();
            row.try_compact()
        });
        let b = scope.spawn(|| {
            barrier.wait();
            row.try_compact()
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
