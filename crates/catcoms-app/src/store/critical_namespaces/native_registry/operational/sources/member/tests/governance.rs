//! Genuine governed-native Current/Source/member path. Synthetic UI atoms are
//! lifecycle fixtures, not evidence of Tauri caller authentication or an Add.
use super::super::super::tests::selected_governed;
use super::*;
use crate::store::critical_namespaces::{
    current_layout::set_after_original_source_hook_for_test,
    governance_layout::{self as governance, GOVERNANCE_MARKER},
    ARCHIVE_BOOTSTRAP_ROOT,
};
use std::{
    fs,
    sync::atomic::{AtomicBool, Ordering::SeqCst},
};

#[tokio::test]
async fn governed_native_member_current_checks_actual4_without_relocking_held_root() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe404);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let original = body("ordinary text in governed native storage");
    assert_eq!(
        prepare(&owner, &row, &vault, &budget)
            .prepare_member_save(sign_request(&original), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
}

#[tokio::test]
async fn governed_native_member_refuses_missing_or_replaced4_after_original_source_proof() {
    for replaced in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (vault, budget) = selected_governed(directory.path());
        let (owner, row) = active(&vault, &budget, 0xe405 + u64::from(replaced));
        let marker = directory
            .path()
            .join(ARCHIVE_BOOTSTRAP_ROOT)
            .join(GOVERNANCE_MARKER);
        let parked = directory.path().join("original-governance-marker.held");
        let original_frame = fs::read(&marker).unwrap();
        let hook_ran = Arc::new(AtomicBool::new(false));
        let ran = hook_ran.clone();
        let target = marker.clone();
        let keep = parked.clone();
        let frame = original_frame.clone();
        set_after_original_source_hook_for_test(Box::new(move || {
            // All outer checked_store/source reads already succeeded. The
            // actual group guard must now reprove its original mounted4 file.
            fs::rename(&target, &keep).unwrap();
            if replaced {
                fs::write(&target, frame).unwrap();
            }
            ran.store(true, SeqCst);
        }));
        let (_sender, receiver) = tokio::sync::watch::channel(false);
        let session = RequestCancellation::new(receiver, None);
        let original = body("retain this request across the namespace refusal");
        assert!(prepare(&owner, &row, &vault, &budget)
            .prepare_member_save(sign_request(&original), &session)
            .is_err());
        assert!(
            hook_ran.load(SeqCst),
            "must reach actual Current, not an earlier opener refusal"
        );
        if replaced {
            fs::remove_file(&marker).unwrap();
        }
        fs::rename(&parked, &marker).unwrap();
        assert_eq!(fs::read(&marker).unwrap(), original_frame);
        // Restore the SAME held object, not another facade or reconstructed
        // Source. Its original token can now finish normally.
        assert_eq!(
            prepare(&owner, &row, &vault, &budget)
                .prepare_member_save(sign_request(&original), &session),
            Ok(V2MemberSaveOutcome::Ready)
        );
    }
}

#[tokio::test]
async fn governed_native_member_refuses_late_same_inode4_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe407);
    let marker = directory
        .path()
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(GOVERNANCE_MARKER);
    let original_frame = fs::read(&marker).unwrap();
    let first_hook = Arc::new(AtomicBool::new(false));
    let marker_hook = Arc::new(AtomicBool::new(false));
    let first_ran = first_hook.clone();
    let marker_ran = marker_hook.clone();
    let target = marker.clone();
    let mut changed = original_frame.clone();
    *changed.last_mut().unwrap() ^= 1;
    set_after_original_source_hook_for_test(Box::new(move || {
        first_ran.store(true, SeqCst);
        governance::set_marker_verification_hook_for_test(Box::new(move || {
            // Same file identity and length, AFTER actual authentication. The
            // final retained-file byte comparison must still reject it.
            fs::write(target, changed).unwrap();
            marker_ran.store(true, SeqCst);
        }));
    }));
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let original = body("retry after repairing the original local marker");
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_member_save(sign_request(&original), &session)
        .is_err());
    assert!(first_hook.load(SeqCst) && marker_hook.load(SeqCst));
    fs::write(&marker, original_frame).unwrap();
    assert_eq!(
        prepare(&owner, &row, &vault, &budget)
            .prepare_member_save(sign_request(&original), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
}
