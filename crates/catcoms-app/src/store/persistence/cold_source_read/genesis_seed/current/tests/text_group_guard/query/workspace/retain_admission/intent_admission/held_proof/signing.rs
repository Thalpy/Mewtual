//! Actual selected-I signing proof only; no key cut is made by these tests.
use super::*;
use crate::store::persistence::v2_text::SavedV2UnsignedAppendIntent;

fn saved_signing_fixture(number: u64) -> (IntentFixture, Arc<SavedV2UnsignedAppendIntent>) {
    let mut f = intent_fixture(0, number);
    let stage = stage_i(&f);
    f.intent.run_stage(stage).unwrap();
    let commit = commit_i(&f);
    f.intent.run_commit(commit).unwrap();
    let saved = Arc::clone(f.intent.saved_unsigned_intent().unwrap());
    (f, saved)
}

fn signing_proof(f: &IntentFixture, saved: &SavedV2UnsignedAppendIntent) -> Result<(), AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let resources = saved.resources().clone();
    store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        let signing = guard.admit_unsigned_signing(&f.lane, saved, &f.intent)?;
        signing.validate_negative()
    })
}

fn native_id(path: &std::path::Path) -> catcoms_storage::kept::SessionId {
    let file = std::fs::File::open(path).unwrap();
    catcoms_storage::kept::session_id(&file).unwrap()
}

#[test]
fn actual_selected_i_signing_proof_holds_original_wi_before_final_source_chain_and_refuses_replacement(
) {
    let mut f = intent_fixture(0, 0x7f9c0);
    let stage = stage_i(&f);
    f.intent.run_stage(stage).unwrap();
    let commit = commit_i(&f);
    f.intent.run_commit(commit).unwrap();
    let saved = Arc::clone(f.intent.saved_unsigned_intent().unwrap());
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let wal = owner.source().source().capture.target.join("text.wal");
    let moved = wal.with_file_name("text.wal-held-by-signing-test");
    let bytes = std::fs::read(&wal).unwrap();
    let called = Arc::new(AtomicBool::new(false));
    let resources = saved.resources().clone();
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.after_original_source_for_test(replace_held_wal(
                wal.clone(),
                moved.clone(),
                bytes.clone(),
                called.clone(),
            ));
            let signing = guard.admit_unsigned_signing(&f.lane, &saved, &f.intent)?;
            signing.validate_negative()
        })
        .is_err());
    assert!(called.load(Ordering::SeqCst));
    assert!(!f.lane.workspace_active_for_test());
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
    restore_control(&wal, &moved);
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            let signing = guard.admit_unsigned_signing(&f.lane, &saved, &f.intent)?;
            signing.validate_negative()
        })
        .unwrap();
    assert!(f.intent.has_saved_data());
    // Only guard construction was exercised: no raw key call was made by this test.
}

#[test]
fn actual_selected_i_signing_proof_refuses_same_native_same_length_wi_corruption_before_capture() {
    let (f, saved) = saved_signing_fixture(0x7f9d0);
    let dir = i_target(&f);
    let wal = dir.join("text.wal");
    let expected = std::fs::read(&wal).unwrap();
    let original_native = native_id(&wal);
    let head = std::fs::read(dir.join("head.bin")).unwrap();
    let mut changed = expected.clone();
    let at = changed.len() / 2;
    changed[at] ^= 0x40;
    std::fs::write(&wal, &changed).unwrap();
    assert_eq!(native_id(&wal), original_native);
    assert_eq!(
        std::fs::metadata(&wal).unwrap().len(),
        expected.len() as u64
    );
    assert!(signing_proof(&f, &saved).is_err());
    assert_eq!(std::fs::read(&wal).unwrap(), changed);
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), head);
    assert!(!f.lane.workspace_active_for_test());
    let store = f.original.layout.checked_store().unwrap();
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
    std::fs::write(&wal, expected).unwrap();
    assert_eq!(native_id(&wal), original_native);
    signing_proof(&f, &saved).unwrap();
}

#[test]
fn actual_selected_i_signing_proof_rechecks_same_held_wi_bytes_after_final_source_wait() {
    let (f, saved) = saved_signing_fixture(0x7f9e0);
    let dir = i_target(&f);
    let wal = dir.join("text.wal");
    let expected = std::fs::read(&wal).unwrap();
    let original_native = native_id(&wal);
    let head = std::fs::read(dir.join("head.bin")).unwrap();
    let mut changed = expected.clone();
    let at = changed.len() / 2;
    changed[at] ^= 0x08;
    let hook_wal = wal.clone();
    let hook_changed = changed.clone();
    let called = Arc::new(AtomicBool::new(false));
    let hook_called = called.clone();
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = saved.resources().clone();
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.after_original_source_for_test(Box::new(move || {
                assert!(std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(&hook_wal)
                    .is_err());
                // First exact comparison has ALREADY happened. Equal metadata and
                // unchanged native/name identity cannot replace its final byte proof
                // after this source wait; recheck that SAME held File.
                std::fs::write(&hook_wal, hook_changed).unwrap();
                assert_eq!(native_id(&hook_wal), original_native);
                hook_called.store(true, Ordering::SeqCst);
            }));
            let signing = guard.admit_unsigned_signing(&f.lane, &saved, &f.intent)?;
            signing.validate_negative()
        })
        .is_err());
    assert!(called.load(Ordering::SeqCst));
    assert_eq!(native_id(&wal), original_native);
    assert_eq!(
        std::fs::metadata(&wal).unwrap().len(),
        expected.len() as u64
    );
    assert_eq!(std::fs::read(&wal).unwrap(), changed);
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), head);
    assert!(!f.lane.workspace_active_for_test());
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
    std::fs::write(&wal, expected).unwrap();
    signing_proof(&f, &saved).unwrap();
}
