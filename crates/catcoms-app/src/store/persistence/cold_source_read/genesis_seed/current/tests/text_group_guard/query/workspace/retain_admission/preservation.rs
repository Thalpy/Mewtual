//! Original Ready/Saved recovery and selected first-A closure preservation.
//! No parsed result, replacement directory or replayed Boolean can become a Saved owner.
use super::*;
use crate::store::persistence::v2_text::V2TextTransactionMetadataResources;
use std::sync::atomic::{AtomicBool, Ordering};

fn validate_saved(f: &RetainFixture) -> Result<(), AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let saved = f.transaction.saved_retained_action()?;
    let resources = saved.resources().clone();
    store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        saved.validate_selected_under_current_guard(&mut guard)
    })
}

#[test]
fn actual_a_ready_and_saved_survive_lost_returns_without_another_nonce_or_head_rewrite() {
    let mut f = retain_fixture(0, 0x7f6a0);
    f.transaction
        .set_work_fault_for_test(V2TextRetainTestCut::ReadyCached);
    let stage = stage_admission(&f);
    assert!(f.transaction.run_stage(stage).is_err());
    assert!(f.transaction.ready_selection().is_ok());
    assert!(f.transaction.saved_retained_action().is_err());
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let old_head = std::fs::read(target.join("head.bin")).unwrap();
    let wal = std::fs::read(target.join("text.wal")).unwrap();
    let candidate = std::fs::read(target.join("head.stage")).unwrap();
    f.transaction
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .unwrap();
    drop(owner);
    let stage = stage_admission(&f);
    f.transaction.run_stage(stage).unwrap();
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), old_head);
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
    assert_eq!(std::fs::read(target.join("head.stage")).unwrap(), candidate);
    f.transaction
        .set_work_fault_for_test(V2TextRetainTestCut::SavedCached);
    let commit = commit_admission(&f);
    assert!(f.transaction.run_commit(commit).is_err());
    assert!(f.transaction.saved_retained_action().is_ok());
    validate_saved(&f).unwrap();
    let selected = std::fs::read(target.join("head.bin")).unwrap();
    assert_eq!(selected, candidate);
    let owner = f.original.owner.borrow_current().unwrap();
    f.transaction
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .unwrap();
    drop(owner);
    // Recover from the actual cached terminal result, not another Stage/Commit permit.
    validate_saved(&f).unwrap();
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected);
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
}

#[test]
fn actual_saved_a_refuses_missing_corrupt_selected_artifacts_and_replaced_pages_directory() {
    let mut f = retain_fixture(0, 0x7f6b0);
    let stage = stage_admission(&f);
    f.transaction.run_stage(stage).unwrap();
    let commit = commit_admission(&f);
    f.transaction.run_commit(commit).unwrap();
    validate_saved(&f).unwrap();
    let (_, archive, token, pages) = f
        .transaction
        .saved_retained_action()
        .unwrap()
        .selected_paths_for_test();
    let archive = archive.to_path_buf();
    let token = token.to_path_buf();
    let pages = pages.to_path_buf();
    for path in [&archive, &token] {
        let expected = std::fs::read(path).unwrap();
        assert!(!expected.is_empty());
        let mut changed = expected.clone();
        let at = changed.len() / 2;
        changed[at] ^= 0x40;
        std::fs::write(path, changed).unwrap();
        assert!(
            validate_saved(&f).is_err(),
            "same-name/same-length corruption is not selected A"
        );
        std::fs::write(path, &expected).unwrap();
        validate_saved(&f).unwrap();
        let moved = path.with_file_name(format!(
            "{}-held-by-test",
            path.file_name().unwrap().to_string_lossy()
        ));
        std::fs::rename(path, &moved).unwrap();
        assert!(
            validate_saved(&f).is_err(),
            "missing H_A/token is not a historical receipt fallback"
        );
        std::fs::rename(&moved, path).unwrap();
        validate_saved(&f).unwrap();
    }
    let original_pages = pages.with_file_name("text.pages-held-by-test");
    std::fs::rename(&pages, &original_pages).unwrap();
    std::fs::create_dir(&pages).unwrap();
    // Even exact selected ciphertexts in a different directory cannot adopt its original
    // namespace identity. Copying only the selected pair does not fabricate missing data.
    std::fs::copy(
        original_pages.join(archive.file_name().unwrap()),
        pages.join(archive.file_name().unwrap()),
    )
    .unwrap();
    std::fs::copy(
        original_pages.join(token.file_name().unwrap()),
        pages.join(token.file_name().unwrap()),
    )
    .unwrap();
    assert!(validate_saved(&f).is_err());
    std::fs::remove_file(pages.join(archive.file_name().unwrap())).unwrap();
    std::fs::remove_file(pages.join(token.file_name().unwrap())).unwrap();
    std::fs::remove_dir(&pages).unwrap();
    std::fs::rename(original_pages, pages).unwrap();
    validate_saved(&f).unwrap();
}

#[test]
fn actual_sole_saved_a_drops_selected_facts_before_its_exact_transaction_metadata_refund() {
    let fee_dropped = Arc::new(AtomicBool::new(false));
    let facts_dropped = Arc::new(AtomicBool::new(false));
    let mut f = V2TextTransactionMetadataResources::with_fee_drop_notice_for_test(
        fee_dropped.clone(),
        || retain_fixture(0, 0x7f6c0),
    );
    f.transaction
        .observe_saved_facts_drop_for_test(facts_dropped.clone(), fee_dropped.clone());
    let stage = stage_admission(&f);
    f.transaction.run_stage(stage).unwrap();
    let commit = commit_admission(&f);
    f.transaction.run_commit(commit).unwrap();
    let saved = f.transaction.saved_retained_action().unwrap().clone();
    let RetainFixture {
        transaction,
        lane,
        predecessor,
        original,
    } = f;
    drop(transaction);
    drop(predecessor);
    drop(lane);
    assert!(!facts_dropped.load(Ordering::SeqCst));
    assert!(!fee_dropped.load(Ordering::SeqCst));
    // SAME original source/SDK/Scope stays alive. The actual selected-facts LAST notice
    // runs after its frame/path fields and asserts the precise metadata fee is still held.
    // Source/read/SDK refunds cannot accidentally satisfy this transaction debit check.
    drop(saved);
    assert!(facts_dropped.load(Ordering::SeqCst));
    assert!(fee_dropped.load(Ordering::SeqCst));
    drop(original);
}
