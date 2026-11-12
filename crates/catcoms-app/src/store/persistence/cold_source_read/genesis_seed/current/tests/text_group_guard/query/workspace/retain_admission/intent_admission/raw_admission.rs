//! Genuine S selection over an already signed original I. Every raw signature is
//! produced through the actual selected-I guard; no raw bytes or receipts are adopted.
use super::raw_call::{selected_i_fixture, sign_i};
use super::*;
mod candidate;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextRawSignatureCommit, AdmittedV2TextRawSignatureStage,
};
use crate::store::persistence::v2_text::{
    PreparedV2TextRawSignature, V2TextRawSignatureTestCut, V2TextTransactionMetadataResources,
};
use std::os::windows::fs::OpenOptionsExt;

struct RawFixture {
    signature: PreparedV2TextRawSignature,
    lane: V2TextCreateLane,
    predecessor: BoundV2TextCreatePredecessor,
    model_dropped: Arc<AtomicBool>,
    sdk_dropped: Arc<AtomicBool>,
    original: TextFixture,
}

fn raw_fixture(family: usize, number: u64) -> RawFixture {
    let (mut f, saved) = selected_i_fixture(family, number);
    sign_i(&mut f, &saved).unwrap();
    f.intent.assert_existing_raw_valid_for_test();
    drop(saved);
    let IntentFixture {
        intent,
        lane,
        predecessor,
        model_dropped,
        sdk_dropped,
        original,
    } = f;
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let mut input = Some(intent);
    let mut signature = None;
    PreparedV2TextRawSignature::take_into(store, owner.source(), &mut input, &mut signature)
        .unwrap();
    assert!(input.is_none());
    let mut signature = signature.unwrap();
    signature
        .prepare_candidates(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(number + 4),
        )
        .unwrap();
    drop(owner);
    RawFixture {
        signature,
        lane,
        predecessor,
        model_dropped,
        sdk_dropped,
        original,
    }
}

fn stage_s(f: &RawFixture) -> AdmittedV2TextRawSignatureStage {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.signature.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_raw_signature_stage_into(
                &f.lane,
                &f.predecessor,
                &f.signature,
                &mut lease,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(lease.is_none());
    admitted.unwrap()
}

fn commit_s(f: &RawFixture) -> AdmittedV2TextRawSignatureCommit {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let ready = f.signature.ready_selection().unwrap();
    let resources = ready.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_raw_signature_commit_into(
                &f.lane,
                &f.predecessor,
                ready,
                &mut lease,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(lease.is_none());
    admitted.unwrap()
}

fn validate_saved_s(f: &RawFixture) -> Result<(), AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let saved = f.signature.saved_raw_signature()?;
    let resources = saved.resources().clone();
    store.with_v2_text_current(&owner, |a| {
        let guard = store.lock_v2_text_current_group(a, &resources)?;
        saved.validate_selected_under_current_guard(&guard)
    })
}

fn s_target(f: &RawFixture) -> std::path::PathBuf {
    f.original
        .owner
        .borrow_current()
        .unwrap()
        .source()
        .source()
        .capture
        .target
        .to_path_buf()
}

fn named_artifact(target: &std::path::Path, prefix: &str) -> std::path::PathBuf {
    std::fs::read_dir(target.join("text.pages"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(prefix)
        })
        .expect("the genuine first transaction created this artifact")
}

#[test]
fn actual_three_family_s_stage_keeps_i_selected_until_fresh_commit_selects_original_raw() {
    for family in 0..3 {
        let mut f = raw_fixture(family, 0x7fb00 + family as u64 * 16);
        let target = s_target(&f);
        let selected_i = std::fs::read(target.join("head.bin")).unwrap();
        let wal_i = std::fs::read(target.join("text.wal")).unwrap();
        let stage = stage_s(&f);
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        let mut read = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
        let resources = read.resources().clone();
        assert!(store
            .with_v2_text_current(&owner, |a| {
                let guard = store.lock_v2_text_current_group(a, &resources)?;
                read.capture(&guard).map(|_| ())
            })
            .is_err());
        assert!(read.selected_data().is_none());
        drop(owner);
        f.signature.run_stage(stage).unwrap();
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected_i);
        assert_ne!(std::fs::read(target.join("text.wal")).unwrap(), wal_i);
        assert!(f.signature.has_ready_data() && !f.signature.has_saved_data());
        let commit = commit_s(&f);
        f.signature.run_commit(commit).unwrap();
        validate_saved_s(&f).unwrap();
        assert_ne!(std::fs::read(target.join("head.bin")).unwrap(), selected_i);
        assert!(!f.model_dropped.load(Ordering::SeqCst));
        assert!(!f.sdk_dropped.load(Ordering::SeqCst));
    }
}

#[test]
fn actual_s_constructor_refuses_absent_raw_and_preserves_the_same_whole_unsigned_owner() {
    let (f, saved) = selected_i_fixture(0, 0x7fb40);
    let IntentFixture {
        intent,
        lane,
        predecessor,
        model_dropped,
        sdk_dropped,
        original,
    } = f;
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let original_intent = intent.observe_original();
    let mut input = Some(intent);
    let mut output = None;
    assert!(
        PreparedV2TextRawSignature::take_into(store, owner.source(), &mut input, &mut output,)
            .is_err()
    );
    assert!(output.is_none());
    original_intent
        .require_same_original(&input.as_ref().unwrap().observe_original())
        .unwrap();
    assert!(input
        .as_ref()
        .unwrap()
        .existing_raw_signature_data()
        .is_err());
    drop(owner);
    let mut restored = IntentFixture {
        intent: input.take().unwrap(),
        lane,
        predecessor,
        model_dropped,
        sdk_dropped,
        original,
    };
    // Reuse exactly the preserved owning input through the real guarded signer.
    sign_i(&mut restored, &saved).unwrap();
    restored.intent.assert_existing_raw_valid_for_test();
    let owner = restored.original.owner.borrow_current().unwrap();
    let store = restored.original.layout.checked_store().unwrap();
    let mut input = Some(restored.intent);
    PreparedV2TextRawSignature::take_into(store, owner.source(), &mut input, &mut output).unwrap();
    assert!(input.is_none() && output.is_some());
}

#[test]
fn actual_s_lost_results_and_uncertain_renames_reuse_exact_original_candidates_with_never_rng() {
    for (number, cut) in [
        (0x7fb50, V2TextRawSignatureTestCut::WalReplaced),
        (0x7fb60, V2TextRawSignatureTestCut::ReadyCached),
        (0x7fb70, V2TextRawSignatureTestCut::HeadRenamed),
        (0x7fb80, V2TextRawSignatureTestCut::SavedCached),
    ] {
        let mut f = raw_fixture(0, number);
        let target = s_target(&f);
        if matches!(
            cut,
            V2TextRawSignatureTestCut::WalReplaced | V2TextRawSignatureTestCut::ReadyCached
        ) {
            f.signature.set_work_fault_for_test(cut);
            let stage = stage_s(&f);
            assert!(f.signature.run_stage(stage).is_err());
            let wal = std::fs::read(target.join("text.wal")).unwrap();
            let store = f.original.layout.checked_store().unwrap();
            let owner = f.original.owner.borrow_current().unwrap();
            f.signature
                .prepare_candidates(store, owner.source(), &mut NeverRng)
                .unwrap();
            drop(owner);
            let stage = stage_s(&f);
            f.signature.run_stage(stage).unwrap();
            assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        } else {
            let stage = stage_s(&f);
            f.signature.run_stage(stage).unwrap();
        }
        let candidate = std::fs::read(target.join("head.stage")).unwrap();
        let wal = std::fs::read(target.join("text.wal")).unwrap();
        if matches!(
            cut,
            V2TextRawSignatureTestCut::HeadRenamed | V2TextRawSignatureTestCut::SavedCached
        ) {
            f.signature.set_work_fault_for_test(cut);
            let commit = commit_s(&f);
            assert!(f.signature.run_commit(commit).is_err());
            if !f.signature.has_saved_data() {
                let commit = commit_s(&f);
                f.signature.run_commit(commit).unwrap();
            }
        } else {
            let commit = commit_s(&f);
            f.signature.run_commit(commit).unwrap();
        }
        validate_saved_s(&f).unwrap();
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        f.signature
            .prepare_candidates(store, owner.source(), &mut NeverRng)
            .unwrap();
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), candidate);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
    }
}

#[test]
fn actual_s_commit_rechecks_same_file_same_length_full_ws_after_admission() {
    let mut f = raw_fixture(0, 0x7fb90);
    let target = s_target(&f);
    let selected_i = std::fs::read(target.join("head.bin")).unwrap();
    let stage = stage_s(&f);
    f.signature.run_stage(stage).unwrap();
    let path = target.join("text.wal");
    let expected = std::fs::read(&path).unwrap();
    let native = catcoms_storage::kept::session_id(&std::fs::File::open(&path).unwrap()).unwrap();
    let commit = commit_s(&f);
    let mut changed = expected.clone();
    let at = changed.len() / 2;
    changed[at] ^= 0x04;
    std::fs::write(&path, &changed).unwrap();
    assert_eq!(
        catcoms_storage::kept::session_id(&std::fs::File::open(&path).unwrap()).unwrap(),
        native
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        expected.len() as u64
    );
    assert!(f.signature.run_commit(commit).is_err());
    assert!(!f.signature.has_saved_data());
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected_i);
    assert_eq!(std::fs::read(&path).unwrap(), changed);
    std::fs::write(&path, expected).unwrap();
    let commit = commit_s(&f);
    f.signature.run_commit(commit).unwrap();
    validate_saved_s(&f).unwrap();
}

#[test]
fn actual_s_and_original_i_files_are_held_before_final_source_and_replacement_refuses_permission() {
    let mut f = raw_fixture(0, 0x7fba0);
    let stage = stage_s(&f);
    f.signature.run_stage(stage).unwrap();
    let target = s_target(&f);
    for prefix in ["s-", "i-"] {
        let path = named_artifact(&target, prefix);
        let moved = path.with_extension("held-original");
        let expected = std::fs::read(&path).unwrap();
        let hook_path = path.clone();
        let hook_moved = moved.clone();
        let called = Arc::new(AtomicBool::new(false));
        let hook_called = called.clone();
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        let ready = f.signature.ready_selection().unwrap();
        let resources = ready.resources().clone();
        let mut lease =
            Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
        let mut admitted = None;
        assert!(store
            .with_v2_text_current(&owner, |a| {
                let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                guard.after_original_source_for_test(Box::new(move || {
                    assert!(std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(0)
                        .open(&hook_path)
                        .is_err());
                    std::fs::rename(&hook_path, &hook_moved).unwrap();
                    std::fs::write(&hook_path, expected).unwrap();
                    hook_called.store(true, Ordering::SeqCst);
                }));
                guard.admit_text_raw_signature_commit_into(
                    &f.lane,
                    &f.predecessor,
                    ready,
                    &mut lease,
                    &mut admitted,
                )
            })
            .is_err());
        assert!(called.load(Ordering::SeqCst));
        assert!(admitted.is_none() && lease.is_some());
        assert!(!f.lane.workspace_active_for_test());
        drop(
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&moved)
                .unwrap(),
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::rename(moved, path).unwrap();
    }
    let commit = commit_s(&f);
    f.signature.run_commit(commit).unwrap();
    validate_saved_s(&f).unwrap();
}

#[test]
fn actual_s_admitted_stage_and_commit_finish_after_scope_stop_without_renewed_current() {
    for committing in [false, true] {
        let mut f = raw_fixture(0, 0x7fbb0 + u64::from(committing));
        let target = s_target(&f);
        let selected_i = std::fs::read(target.join("head.bin")).unwrap();
        let stage = stage_s(&f);
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        if committing {
            f.signature.run_stage(stage).unwrap();
            let commit = commit_s(&f);
            owner.warm_scope().unwrap().revocation().revoke();
            f.signature.run_commit(commit).unwrap();
            assert!(f.signature.has_saved_data());
            assert_ne!(std::fs::read(target.join("head.bin")).unwrap(), selected_i);
        } else {
            owner.warm_scope().unwrap().revocation().revoke();
            f.signature.run_stage(stage).unwrap();
            assert!(f.signature.has_ready_data() && !f.signature.has_saved_data());
            assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected_i);
        }
        // In the Stage-only branch there is no SavedS yet. Check real Current
        // refusal directly rather than treating missing Saved DATA as evidence.
        assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
        assert!(validate_saved_s(&f).is_err());
        assert!(!f.lane.workspace_active_for_test());
        assert!(!f.model_dropped.load(Ordering::SeqCst));
        assert!(!f.sdk_dropped.load(Ordering::SeqCst));
    }
}

#[test]
fn actual_signature_file_and_os_lease_keep_vault_busy_until_finite_cleanup() {
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let mut f = raw_fixture(0, 0x7fbc0);
    let stage = stage_s(&f);
    let target = s_target(&f);
    let model_dropped = f.model_dropped.clone();
    let sdk_dropped = f.sdk_dropped.clone();
    let (paused_tx, paused_rx) = sync_channel(0);
    let (release_tx, release_rx) = sync_channel(0);
    f.signature.set_work_pause_for_test(
        V2TextRawSignatureTestCut::SignatureCreated,
        Box::new(move || {
            paused_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        }),
    );
    f.signature
        .set_work_fault_for_test(V2TextRawSignatureTestCut::SignatureCreated);
    let RawFixture {
        mut signature,
        lane,
        predecessor,
        model_dropped: _,
        sdk_dropped: _,
        original,
    } = f;
    let worker = std::thread::spawn(move || {
        let result = signature.run_stage(stage);
        (signature, result)
    });
    paused_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    let path = named_artifact(&target, "s-");
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .is_err());
    let TextFixture {
        owner,
        layout,
        _directory: directory,
    } = original;
    drop(owner);
    drop(lane);
    drop(predecessor);
    drop(layout);
    assert!(matches!(
        catcoms_storage::acquire_vault_session(directory.path()),
        Err(catcoms_storage::StorageError::VaultBusy)
    ));
    release_tx.send(()).unwrap();
    let (signature, result) = worker.join().unwrap();
    assert!(result.is_err());
    assert!(!signature.has_ready_data() && !signature.has_saved_data());
    drop(
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .unwrap(),
    );
    drop(
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(target.join("text.wal"))
            .unwrap(),
    );
    let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    assert!(!model_dropped.load(Ordering::SeqCst));
    assert!(!sdk_dropped.load(Ordering::SeqCst));
    drop(session);
    drop(signature);
    assert!(model_dropped.load(Ordering::SeqCst));
    assert!(sdk_dropped.load(Ordering::SeqCst));
}

#[test]
fn actual_sole_saved_s_drops_selected_facts_before_exact_metadata_refund() {
    let fee_dropped = Arc::new(AtomicBool::new(false));
    let facts_dropped = Arc::new(AtomicBool::new(false));
    let mut f = V2TextTransactionMetadataResources::with_fee_drop_notice_for_test(
        fee_dropped.clone(),
        || raw_fixture(0, 0x7fbd0),
    );
    f.signature
        .observe_saved_facts_drop_for_test(facts_dropped.clone(), fee_dropped.clone());
    let stage = stage_s(&f);
    f.signature.run_stage(stage).unwrap();
    let commit = commit_s(&f);
    f.signature.run_commit(commit).unwrap();
    let saved = Arc::clone(f.signature.saved_raw_signature().unwrap());
    let RawFixture {
        signature,
        lane,
        predecessor,
        model_dropped,
        sdk_dropped,
        original,
    } = f;
    drop(signature);
    drop(predecessor);
    drop(lane);
    assert!(model_dropped.load(Ordering::SeqCst));
    assert!(sdk_dropped.load(Ordering::SeqCst));
    // The real accepted-query registry also retains its exact SavedA parent after
    // its reader End drains. Drop that actual Store owner before calling SavedS the
    // sole metadata tail; reader0 alone is not retirement and no test clears it.
    // Keep the original native owner/SDK and directory alive independently.
    let TextFixture {
        owner,
        layout,
        _directory: directory,
    } = original;
    drop(layout);
    assert!(!facts_dropped.load(Ordering::SeqCst));
    assert!(!fee_dropped.load(Ordering::SeqCst));
    drop(saved);
    assert!(facts_dropped.load(Ordering::SeqCst));
    assert!(fee_dropped.load(Ordering::SeqCst));
    drop(owner);
    drop(directory);
}

mod semantic;

mod foreign_guard;
mod warm;

mod accepted_admission;
