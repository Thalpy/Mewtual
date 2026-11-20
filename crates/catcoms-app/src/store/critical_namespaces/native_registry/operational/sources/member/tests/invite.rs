//! Real whole native runtime, held Current4 and the same member verifier. No
//! Signed/SavedInvite, native caller authentication or actual join is claimed.
use super::super::super::tests::selected_governed;
use super::*;
use crate::store::critical_namespaces::{
    current_layout::set_after_original_source_hook_for_test, governance_layout::GOVERNANCE_MARKER,
    ARCHIVE_BOOTSTRAP_ROOT,
};

fn find_named(root: &std::path::Path, name: &str) -> Vec<std::path::PathBuf> {
    fn visit(root: &std::path::Path, name: &str, depth: u8, result: &mut Vec<std::path::PathBuf>) {
        assert!(depth < 12, "bounded isolated fixture tree");
        for entry in std::fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == name {
                result.push(entry.path());
            }
            if entry.file_type().unwrap().is_dir() {
                visit(&entry.path(), name, depth + 1, result);
            }
        }
    }
    let mut result = Vec::new();
    visit(root, name, 0, &mut result);
    result
}
fn only_named(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let mut paths = find_named(root, name);
    assert_eq!(paths.len(), 1);
    paths.pop().unwrap()
}

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
    let journal = only_named(directory.path(), "invite.intent.v1");
    let frame = std::fs::read(&journal).unwrap();
    assert!(frame.len() > 40 && frame.len() <= 8192);
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_invite_intent([41; 32], 60_000, &session),
        Ok(original)
    );
    assert_eq!(
        std::fs::read(&journal).unwrap(),
        frame,
        "exact retry preserves the same sealed frame"
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
    assert_eq!(
        std::fs::read(journal).unwrap(),
        frame,
        "ordinary authoring preserves the invitation family"
    );
}

#[tokio::test]
async fn native_runtime_invite_partial_write_reuses_the_same_original_and_ciphertext() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe706);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    crate::store::PreparedV2TextHost::<MeshService, ChaCha20Rng>::fail_partial_invite_write_for_test();
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([47; 32], 60_000, &session)
        .is_err());
    let path = only_named(directory.path(), "invite.intent.v1");
    let prefix = std::fs::read(&path).unwrap();
    assert_eq!(prefix.len(), 128);
    let original = prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([47; 32], 60_000, &session)
        .unwrap();
    let frame = std::fs::read(&path).unwrap();
    assert!(frame.len() > prefix.len() && frame.starts_with(&prefix));
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_invite_intent([47; 32], 60_000, &session),
        Ok(original)
    );
    assert_eq!(std::fs::read(path).unwrap(), frame);
}

#[tokio::test]
async fn native_runtime_invite_corrupt_or_missing_covered_journal_never_becomes_empty_usage() {
    for missing in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (vault, budget) = selected_governed(directory.path());
        let (owner, row) = active(&vault, &budget, 0xe707 + u64::from(missing));
        let (_sender, receiver) = tokio::sync::watch::channel(false);
        let session = RequestCancellation::new(receiver, None);
        prepare(&owner, &row, &vault, &budget)
            .prepare_invite_intent([48; 32], 60_000, &session)
            .unwrap();
        let path = only_named(directory.path(), "invite.intent.v1");
        let original = std::fs::read(&path).unwrap();
        if missing {
            std::fs::rename(&path, path.with_extension("held")).unwrap();
        } else {
            std::fs::write(&path, b"torn quota/intent").unwrap();
        }
        assert!(prepare(&owner, &row, &vault, &budget)
            .prepare_invite_intent([48; 32], 60_000, &session)
            .is_err());
        assert!(prepare(&owner, &row, &vault, &budget)
            .prepare_invite_intent([49; 32], 60_000, &session)
            .is_err());
        if missing {
            assert!(!path.exists());
        } else {
            assert_eq!(std::fs::read(&path).unwrap(), b"torn quota/intent");
        }
        assert!(original.len() > 40);
    }
}

#[tokio::test]
async fn native_runtime_invite_unknown_preexisting_state_is_preserved_and_ordinary_saves_still_work(
) {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe709);
    // Enter the real ordinary lane first to ensure its generation-1 pages exist.
    let text = body("ordinary state before the invitation journal");
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let path = only_named(directory.path(), "text.pages").join("invite.intent.v1");
    std::fs::write(&path, b"unknown/future journal").unwrap();
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([50; 32], 60_000, &session)
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"unknown/future journal");
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
    std::fs::remove_file(&path).unwrap();
    assert!(
        prepare(&owner, &row, &vault, &budget)
            .prepare_invite_intent([50; 32], 60_000, &session)
            .is_err(),
        "removing previously unknown quota evidence cannot reset usage"
    );
    assert!(!path.exists());
}

#[tokio::test]
async fn native_runtime_invite_lost_reply_after_cover_retries_without_rewriting_the_intent() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected_governed(directory.path());
    let (owner, row) = active(&vault, &budget, 0xe70a);
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    super::super::super::invite::AFTER_COVER.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            sender.send_replace(true);
        }));
    });
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_invite_intent([51; 32], 60_000, &session),
        Err(V2MemberSaveRefusal::Cancelled)
    );
    let path = only_named(directory.path(), "invite.intent.v1");
    let frame = std::fs::read(&path).unwrap();
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let fresh = RequestCancellation::new(receiver, None);
    assert!(prepare(&owner, &row, &vault, &budget)
        .prepare_invite_intent([51; 32], 60_000, &fresh)
        .is_ok());
    assert_eq!(std::fs::read(path).unwrap(), frame);
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
