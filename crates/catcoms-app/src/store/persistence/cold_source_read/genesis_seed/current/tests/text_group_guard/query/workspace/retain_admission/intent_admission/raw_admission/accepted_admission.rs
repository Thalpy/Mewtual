//! Genuine B selection from the SAME guarded raw, original SDK and verified Core
//! candidate. These tests do not manufacture receipts, index misses or a model.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextAcceptedCreateCommit, AdmittedV2TextAcceptedCreateStage,
};
use crate::store::persistence::v2_text::{
    PreparedV2TextAcceptedCreate, V2TextAcceptedCreateTestCut,
};

struct AcceptedFixture {
    accepted: Box<PreparedV2TextAcceptedCreate>,
    lane: V2TextCreateLane,
    predecessor: BoundV2TextCreatePredecessor,
    model_dropped: Arc<AtomicBool>,
    sdk_dropped: Arc<AtomicBool>,
    original: TextFixture,
}

fn accepted_fixture(family: usize, number: u64) -> AcceptedFixture {
    let mut f = raw_fixture(family, number);
    let stage = stage_s(&f);
    f.signature.run_stage(stage).unwrap();
    let commit = commit_s(&f);
    f.signature.run_commit(commit).unwrap();
    let store = f.original.layout.checked_store().unwrap();
    // No node-backed Source loan survives the actual SDK Header/Core derivation.
    f.signature
        .materialize_candidate_after_saved_s(store, &f.original.owner)
        .unwrap();
    let RawFixture {
        signature,
        lane,
        predecessor,
        model_dropped,
        sdk_dropped,
        original,
    } = f;
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let mut input = Some(signature);
    let mut accepted = None;
    PreparedV2TextAcceptedCreate::take_into(store, owner.source(), &mut input, &mut accepted)
        .unwrap();
    assert!(input.is_none());
    let mut accepted = accepted.unwrap();
    accepted
        .prepare_candidates(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(number + 5),
        )
        .unwrap();
    drop(owner);
    AcceptedFixture {
        accepted,
        lane,
        predecessor,
        model_dropped,
        sdk_dropped,
        original,
    }
}

fn stage_b(f: &AcceptedFixture) -> AdmittedV2TextAcceptedCreateStage {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.accepted.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_accepted_create_stage_into(
                &f.lane,
                &f.predecessor,
                &f.accepted,
                &mut lease,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(lease.is_none());
    admitted.unwrap()
}

fn commit_b(f: &AcceptedFixture) -> AdmittedV2TextAcceptedCreateCommit {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let ready = f.accepted.ready_selection().unwrap();
    let resources = ready.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_accepted_create_commit_into(
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

fn validate_saved_b(f: &AcceptedFixture) -> Result<(), AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let saved = f.accepted.saved_accepted_create()?;
    let resources = saved.resources().clone();
    store.with_v2_text_current(&owner, |a| {
        let guard = store.lock_v2_text_current_group(a, &resources)?;
        saved.validate_selected_under_current_guard(&guard)
    })
}

fn b_target(f: &AcceptedFixture) -> std::path::PathBuf {
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

fn advance_b(f: &mut AcceptedFixture) -> Result<(), AppError> {
    let saved = Arc::clone(f.accepted.saved_accepted_create()?);
    let resources = saved.resources().clone();
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        let advance = guard.admit_accepted_advance(&f.lane, &saved, &f.accepted)?;
        f.accepted.advance_accepted_under_guard(&advance)
    })
}

fn final_b(f: &mut AcceptedFixture) -> Result<(), AppError> {
    let saved = Arc::clone(f.accepted.saved_accepted_create()?);
    let resources = saved.resources().clone();
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        let final_guard = guard.admit_accepted_final(&f.lane, &saved, &f.accepted)?;
        f.accepted.confirm_accepted_final_under_guard(&final_guard)
    })
}

#[test]
fn actual_three_family_b_stage_keeps_s_selected_and_distinct_commit_selects_verified_candidate() {
    for family in 0..3 {
        let mut f = accepted_fixture(family, 0x7ff00 + family as u64 * 16);
        let target = b_target(&f);
        let selected_s = std::fs::read(target.join("head.bin")).unwrap();
        let wal_s = std::fs::read(target.join("text.wal")).unwrap();
        let stage = stage_b(&f);
        f.accepted.run_stage(stage).unwrap();
        assert!(f.accepted.has_ready_data() && !f.accepted.has_saved_data());
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected_s);
        assert_ne!(std::fs::read(target.join("text.wal")).unwrap(), wal_s);
        let commit = commit_b(&f);
        f.accepted.run_commit(commit).unwrap();
        validate_saved_b(&f).unwrap();
        assert_ne!(std::fs::read(target.join("head.bin")).unwrap(), selected_s);
        assert!(
            f.accepted.accepted_final_basis_data().is_err(),
            "B alone does not advance/finalize the verifier"
        );
        assert!(!f.model_dropped.load(Ordering::SeqCst));
        assert!(!f.sdk_dropped.load(Ordering::SeqCst));
    }
}

#[test]
fn actual_b_lost_outcomes_and_uncertain_renames_reuse_original_frames_with_never_rng() {
    for (number, cut) in [
        (0x7ff40, V2TextAcceptedCreateTestCut::WalReplaced),
        (0x7ff50, V2TextAcceptedCreateTestCut::ReadyCached),
        (0x7ff60, V2TextAcceptedCreateTestCut::HeadRenamed),
        (0x7ff70, V2TextAcceptedCreateTestCut::SavedCached),
    ] {
        let mut f = accepted_fixture(0, number);
        let target = b_target(&f);
        if matches!(
            cut,
            V2TextAcceptedCreateTestCut::WalReplaced | V2TextAcceptedCreateTestCut::ReadyCached
        ) {
            f.accepted.set_work_fault_for_test(cut);
            let stage = stage_b(&f);
            assert!(f.accepted.run_stage(stage).is_err());
            let wal = std::fs::read(target.join("text.wal")).unwrap();
            let store = f.original.layout.checked_store().unwrap();
            let owner = f.original.owner.borrow_current().unwrap();
            f.accepted
                .prepare_candidates(store, owner.source(), &mut NeverRng)
                .unwrap();
            drop(owner);
            let stage = stage_b(&f);
            f.accepted.run_stage(stage).unwrap();
            assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        } else {
            let stage = stage_b(&f);
            f.accepted.run_stage(stage).unwrap();
        }
        let head = std::fs::read(target.join("head.stage")).unwrap();
        let wal = std::fs::read(target.join("text.wal")).unwrap();
        if matches!(
            cut,
            V2TextAcceptedCreateTestCut::HeadRenamed | V2TextAcceptedCreateTestCut::SavedCached
        ) {
            f.accepted.set_work_fault_for_test(cut);
            let commit = commit_b(&f);
            assert!(f.accepted.run_commit(commit).is_err());
            if !f.accepted.has_saved_data() {
                let commit = commit_b(&f);
                f.accepted.run_commit(commit).unwrap();
            }
        } else {
            let commit = commit_b(&f);
            f.accepted.run_commit(commit).unwrap();
        }
        validate_saved_b(&f).unwrap();
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        f.accepted
            .prepare_candidates(store, owner.source(), &mut NeverRng)
            .unwrap();
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
    }
}

#[test]
fn actual_b_commit_refuses_full_same_file_wb_corruption_after_real_admission() {
    let mut f = accepted_fixture(0, 0x7ff80);
    let target = b_target(&f);
    let head = std::fs::read(target.join("head.bin")).unwrap();
    let stage = stage_b(&f);
    f.accepted.run_stage(stage).unwrap();
    let wal = target.join("text.wal");
    let expected = std::fs::read(&wal).unwrap();
    let native = catcoms_storage::kept::session_id(&std::fs::File::open(&wal).unwrap()).unwrap();
    let commit = commit_b(&f);
    let mut changed = expected.clone();
    let at = changed.len() / 2;
    changed[at] ^= 0x20;
    std::fs::write(&wal, &changed).unwrap();
    assert_eq!(
        catcoms_storage::kept::session_id(&std::fs::File::open(&wal).unwrap()).unwrap(),
        native
    );
    assert_eq!(
        std::fs::metadata(&wal).unwrap().len(),
        expected.len() as u64
    );
    assert!(f.accepted.run_commit(commit).is_err());
    assert!(!f.accepted.has_saved_data());
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
    assert_eq!(std::fs::read(&wal).unwrap(), changed);
    std::fs::write(&wal, expected).unwrap();
    let commit = commit_b(&f);
    f.accepted.run_commit(commit).unwrap();
    validate_saved_b(&f).unwrap();
}

#[test]
fn actual_b_admitted_stage_and_commit_finish_after_stop_but_real_current_refuses() {
    for committing in [false, true] {
        let mut f = accepted_fixture(0, 0x7ff90 + u64::from(committing));
        let target = b_target(&f);
        let head = std::fs::read(target.join("head.bin")).unwrap();
        let stage = stage_b(&f);
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        if committing {
            f.accepted.run_stage(stage).unwrap();
            let commit = commit_b(&f);
            owner.warm_scope().unwrap().revocation().revoke();
            f.accepted.run_commit(commit).unwrap();
            assert!(f.accepted.has_saved_data());
            assert_ne!(std::fs::read(target.join("head.bin")).unwrap(), head);
        } else {
            owner.warm_scope().unwrap().revocation().revoke();
            f.accepted.run_stage(stage).unwrap();
            assert!(f.accepted.has_ready_data() && !f.accepted.has_saved_data());
            assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        }
        assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
        assert!(!f.lane.workspace_active_for_test());
        assert!(!f.model_dropped.load(Ordering::SeqCst));
        assert!(!f.sdk_dropped.load(Ordering::SeqCst));
    }
}

#[test]
fn actual_b_flat_saved_source_remains_valid_without_retired_private_predecessors_and_refuses_new_damage(
) {
    let mut f = accepted_fixture(0, 0x7ffa0);
    let target = b_target(&f);
    let old = f.accepted.original_retired_paths_for_test().unwrap();
    assert_eq!(old.len(), 4);
    let before: Vec<_> = std::fs::read_dir(target.join("text.pages"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let stage = stage_b(&f);
    f.accepted.run_stage(stage).unwrap();
    let commit = commit_b(&f);
    f.accepted.run_commit(commit).unwrap();
    validate_saved_b(&f).unwrap();
    // Storage-scenario only: model the four legitimate post-WD absences. This test
    // does NOT mint WD, a retirement basis, deletion authority or completed WC.
    for path in old {
        std::fs::remove_file(path).unwrap();
    }
    validate_saved_b(&f).unwrap();
    let mut new: Vec<_> = std::fs::read_dir(target.join("text.pages"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !before.contains(p))
        .collect();
    new.sort();
    let path = new.first().unwrap();
    let expected = std::fs::read(path).unwrap();
    let mut changed = expected.clone();
    let at = changed.len() / 2;
    changed[at] ^= 1;
    std::fs::write(path, &changed).unwrap();
    assert!(validate_saved_b(&f).is_err());
    std::fs::write(path, expected).unwrap();
    validate_saved_b(&f).unwrap();
    // The actual verifier still needs a fresh new-B proof; no old I/S/TA read may
    // reappear at advance/final now that those legitimate predecessors are absent.
    advance_b(&mut f).unwrap();
    assert!(final_b(&mut f).is_err());
    f.accepted.drain_accepted_retired_math_if_present().unwrap();
    final_b(&mut f).unwrap();
    assert!(f.accepted.accepted_final_basis_data().is_ok());
    advance_b(&mut f).unwrap();
    final_b(&mut f).unwrap();
}

#[test]
fn actual_b_created_artifact_and_lease_keep_vault_busy_until_finite_worker_cleanup() {
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let mut f = accepted_fixture(0, 0x7ffb0);
    let target = b_target(&f);
    let before: Vec<_> = std::fs::read_dir(target.join("text.pages"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let stage = stage_b(&f);
    let model = f.model_dropped.clone();
    let sdk = f.sdk_dropped.clone();
    let (tx, rx) = sync_channel(0);
    let (release, wait) = sync_channel(0);
    f.accepted.set_work_pause_for_test(
        V2TextAcceptedCreateTestCut::ArtifactCreated,
        Box::new(move || {
            tx.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(15)).unwrap();
        }),
    );
    f.accepted
        .set_work_fault_for_test(V2TextAcceptedCreateTestCut::ArtifactCreated);
    let AcceptedFixture {
        mut accepted,
        lane,
        predecessor,
        model_dropped: _,
        sdk_dropped: _,
        original,
    } = f;
    let worker = std::thread::spawn(move || {
        let result = accepted.run_stage(stage);
        (accepted, result)
    });
    rx.recv_timeout(Duration::from_secs(15)).unwrap();
    let path = std::fs::read_dir(target.join("text.pages"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| !before.contains(p) && std::fs::metadata(p).unwrap().len() == 0)
        .expect("actual B ArtifactCreated owns its new zero-length File");
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
    release.send(()).unwrap();
    let (accepted, result) = worker.join().unwrap();
    assert!(result.is_err());
    assert!(!accepted.has_ready_data() && !accepted.has_saved_data());
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
    assert!(!model.load(Ordering::SeqCst));
    assert!(!sdk.load(Ordering::SeqCst));
    drop(session);
    drop(accepted);
    assert!(model.load(Ordering::SeqCst));
    assert!(sdk.load(Ordering::SeqCst));
}

#[test]
fn actual_sole_saved_b_drops_flat_facts_before_its_exact_new_metadata_debit() {
    let fee = Arc::new(AtomicBool::new(false));
    let facts = Arc::new(AtomicBool::new(false));
    let mut f =
        PreparedV2TextAcceptedCreate::with_metadata_fee_drop_notice_for_test(fee.clone(), || {
            accepted_fixture(0, 0x7ffc0)
        });
    f.accepted
        .observe_saved_facts_drop_for_test(facts.clone(), fee.clone());
    let stage = stage_b(&f);
    f.accepted.run_stage(stage).unwrap();
    let commit = commit_b(&f);
    f.accepted.run_commit(commit).unwrap();
    let saved = Arc::clone(f.accepted.saved_accepted_create().unwrap());
    let AcceptedFixture {
        accepted,
        lane,
        predecessor,
        model_dropped,
        sdk_dropped,
        original,
    } = f;
    drop(accepted);
    drop(predecessor);
    drop(lane);
    assert!(model_dropped.load(Ordering::SeqCst));
    assert!(sdk_dropped.load(Ordering::SeqCst));
    assert!(!facts.load(Ordering::SeqCst));
    assert!(!fee.load(Ordering::SeqCst));
    // There is no B-generation query in this fixture; actual old A query-registry
    // ownership pays its separate OLD metadata and cannot satisfy this NEW debit.
    drop(saved);
    assert!(facts.load(Ordering::SeqCst));
    assert!(fee.load(Ordering::SeqCst));
    drop(original);
}

mod capacity;
mod constructor;
mod lifecycle;
mod semantic;
mod warm;

// Actual old-root closure, native-End join and deterministic real-wait entry.
mod retirement;
