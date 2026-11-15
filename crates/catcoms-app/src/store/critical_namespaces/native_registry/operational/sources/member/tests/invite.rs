//! Real whole native runtime, held Current4 and the same member verifier. No
//! Signed/SavedInvite, native caller authentication or actual join is claimed.
use super::super::super::tests::selected_governed;
use super::*;
use crate::store::critical_namespaces::{
    current_layout::set_after_original_source_hook_for_test, governance_layout::GOVERNANCE_MARKER,
    ARCHIVE_BOOTSTRAP_ROOT,
};

#[tokio::test]
async fn native_runtime_invite_original_uses_current4_and_does_not_block_ordinary_member() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe701);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let original = prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([41; 32], 60_000, &session)
        .unwrap();
    assert_ne!(original.0, [0; 32]);
    assert!(original.1 >= 60_000);
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_invite_intent([41; 32], 60_000, &session),
        Ok(original)
    );
    // Different token/settings cannot replace the retained draw.
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([42; 32], 60_000, &session)
        .is_err());
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([41; 32], 60_001, &session)
        .is_err());
    let text = body("ordinary authoring after unsigned invite preparation");
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_invite_intent([41; 32], 60_000, &session),
        Ok(original)
    );
}

#[tokio::test]
async fn native_runtime_invite_refuses_plain_native3_without_disabling_ordinary_member() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe702);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_invite_intent([43; 32], 60_000, &session),
        Err(V2MemberSaveRefusal::Failed)
    );
    let text = body("old-family member authoring still works");
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
}

#[tokio::test]
async fn native_runtime_invite_requires_original4_after_outer_admission() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe703);
    let marker = directory
        .path()
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(GOVERNANCE_MARKER);
    let parked = directory.path().join("invite-original4.held");
    let target = marker.clone();
    let keep = parked.clone();
    let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = ran.clone();
    set_after_original_source_hook_for_test(Box::new(move || {
        std::fs::rename(&target, keep).unwrap();
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }));
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([44; 32], 60_000, &session)
        .is_err());
    assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
    std::fs::rename(parked, marker).unwrap();
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([44; 32], 60_000, &session)
        .is_ok());
}

#[tokio::test]
async fn native_runtime_invite_cancelled_or_closed_source_never_returns_prepared_success() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe704);
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let job = prepare(&owner, &row, &vault, &budget);
    sender.send_replace(true);
    assert_eq!(
        job.prepare_invite_intent([45; 32], 60_000, &session),
        Err(V2MemberSaveRefusal::Cancelled)
    );
    sender.send_replace(false);
    let job = prepare(&owner, &row, &vault, &budget);
    owner.entries.lock().unwrap()[0].owner.close();
    assert_eq!(
        job.prepare_invite_intent([45; 32], 60_000, &session),
        Err(V2MemberSaveRefusal::Unavailable)
    );
}
