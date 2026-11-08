//! Runtime ordering evidence: actual controls must already be held at the final
//! original-source/ID3 seam. Old post-ID3 opens fail the exclusive-open assertion.
use super::*;
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;

fn replace_held_wal(
    wal: PathBuf,
    moved: PathBuf,
    expected: Vec<u8>,
    called: Arc<AtomicBool>,
) -> Box<dyn FnOnce()> {
    Box::new(move || {
        // A real FD is already open before Source/ID3, not just a saved native label.
        // The former Ready validators opened this control AFTER the hook, so an
        // exclusive open would have succeeded and this assertion would fail.
        assert!(std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&wal)
            .is_err());
        called.store(true, Ordering::SeqCst);
        assert!(!moved.exists());
        std::fs::rename(&wal, moved).unwrap();
        std::fs::write(wal, expected).unwrap();
    })
}

fn restore_control(wal: &std::path::Path, moved: &std::path::Path) {
    // Failed proof must have closed ALL of its helper-local controls. The outside
    // unused physical lease is not a File handle and cannot conceal premature drops.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(moved)
        .unwrap();
    drop(held);
    std::fs::remove_file(wal).unwrap();
    std::fs::rename(moved, wal).unwrap();
}

#[test]
fn actual_ready_a_holds_wal_before_source_id3_and_rejects_replacement_without_admitting_work() {
    let mut f = retain_fixture(0, 0x7f9a0);
    let stage = stage_admission(&f);
    f.transaction.run_stage(stage).unwrap();
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let wal = owner.source().source().capture.target.join("text.wal");
    let moved = wal.with_file_name("text.wal-held-by-ready-a-test");
    let bytes = std::fs::read(&wal).unwrap();
    let called = Arc::new(AtomicBool::new(false));
    let resources = f.transaction.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.after_original_source_for_test(replace_held_wal(
                wal.clone(),
                moved.clone(),
                bytes.clone(),
                called.clone(),
            ));
            guard.admit_text_retain_commit_into(
                &f.lane,
                &f.predecessor,
                f.transaction.ready_selection()?,
                &mut lease,
                &mut admitted,
            )
        })
        .is_err());
    assert!(called.load(Ordering::SeqCst));
    assert!(admitted.is_none() && lease.is_some());
    assert!(!f.lane.workspace_active_for_test());
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
    restore_control(&wal, &moved);
    drop(lease);
    drop(owner);
    let commit = commit_admission(&f);
    f.transaction.run_commit(commit).unwrap();
    assert!(f.transaction.saved_retained_action().is_ok());
}

#[test]
fn actual_ready_i_holds_wi_before_source_id3_and_rejects_replacement_without_admitting_work() {
    let mut f = intent_fixture(0, 0x7f9b0);
    let stage = stage_i(&f);
    f.intent.run_stage(stage).unwrap();
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let wal = owner.source().source().capture.target.join("text.wal");
    let moved = wal.with_file_name("text.wal-held-by-ready-i-test");
    let bytes = std::fs::read(&wal).unwrap();
    let called = Arc::new(AtomicBool::new(false));
    let resources = f.intent.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.after_original_source_for_test(replace_held_wal(
                wal.clone(),
                moved.clone(),
                bytes.clone(),
                called.clone(),
            ));
            guard.admit_text_intent_commit_into(
                &f.lane,
                &f.predecessor,
                f.intent.ready_selection()?,
                &mut lease,
                &mut admitted,
            )
        })
        .is_err());
    assert!(called.load(Ordering::SeqCst));
    assert!(admitted.is_none() && lease.is_some());
    assert!(!f.lane.workspace_active_for_test());
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
    restore_control(&wal, &moved);
    drop(lease);
    drop(owner);
    let commit = commit_i(&f);
    f.intent.run_commit(commit).unwrap();
    validate_saved_i(&f).unwrap();
}

// Separate activation: these cases require the concrete selected-I signing proof.
mod signing;
