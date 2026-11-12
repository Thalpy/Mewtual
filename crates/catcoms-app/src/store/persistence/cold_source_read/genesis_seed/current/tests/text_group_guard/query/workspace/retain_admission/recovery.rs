//! SAME original first-A warm recovery. Direct truncation here models torn private
//! storage; these deterministic cuts do not claim to simulate a physical power failure.
//! Every positive permit still comes from actual Current, actual named files and the
//! original SDK/cell fixture. No parsed WAL or manually installed writer flag is used.
mod custody;
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextRetainRecoveryRead, AdmittedV2TextRetainRepair, AdmittedV2TextRetainScratch,
    PreparedV2TextRetainRecoveryCapture, PreparedV2TextRetainRepairLease,
    PreparedV2TextRetainScratchLease,
};
use crate::store::persistence::v2_text::{
    PreparedV2TextRetainRecovery, V2TextRetainRecoveryTestCut,
    V2TextRetainWarmRecoveryClassData as Class,
};
use std::path::PathBuf;

fn target(f: &RetainFixture) -> PathBuf {
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

fn prepare_recovery(f: &RetainFixture) -> PreparedV2TextRetainRecovery {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    PreparedV2TextRetainRecovery::prepare(store, owner.source(), &f.transaction).unwrap()
}

fn read_admission(
    f: &RetainFixture,
    recovery: &mut PreparedV2TextRetainRecovery,
) -> AdmittedV2TextRetainRecoveryRead {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = recovery.resources().clone();
    let mut capture = Some(
        PreparedV2TextRetainRecoveryCapture::prepare(store, owner.source(), recovery).unwrap(),
    );
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_recovery_read_into(
                &f.lane,
                &f.predecessor,
                recovery,
                &mut capture,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(capture.is_none());
    admitted.unwrap()
}

fn repair_admission(
    f: &RetainFixture,
    recovery: &PreparedV2TextRetainRecovery,
) -> AdmittedV2TextRetainRepair {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let ready = recovery.ready_repair().unwrap();
    let resources = ready.resources().clone();
    let mut lease =
        Some(PreparedV2TextRetainRepairLease::prepare(store, owner.source(), ready).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_repair_into(
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

fn scratch_admission(
    f: &RetainFixture,
    recovery: &PreparedV2TextRetainRecovery,
) -> AdmittedV2TextRetainScratch {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let ready = recovery.ready_scratch().unwrap();
    let resources = ready.resources().clone();
    let mut lease =
        Some(PreparedV2TextRetainScratchLease::prepare(store, owner.source(), ready).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_scratch_into(
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

fn fail_stage(f: &mut RetainFixture, cut: V2TextRetainTestCut) {
    f.transaction.set_work_fault_for_test(cut);
    let admitted = stage_admission(f);
    assert!(f.transaction.run_stage(admitted).is_err());
    assert!(f.transaction.ready_selection().is_err());
    assert!(f.transaction.saved_retained_action().is_err());
}

fn no_new_nonce(f: &mut RetainFixture) {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    f.transaction
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .unwrap();
}

fn ordinary_stage_is_refused(f: &RetainFixture) {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.transaction.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_stage_into(
                &f.lane,
                &f.predecessor,
                &f.transaction,
                &mut lease,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none() && lease.is_some());
}

#[test]
fn actual_clean_unstarted_and_complete_wal_stage_are_distinct_nonmutating_read_results() {
    let mut f = retain_fixture(0, 0x7f700);
    let dir = target(&f);
    let old = std::fs::read(dir.join("head.bin")).unwrap();
    let mut recovery = prepare_recovery(&f);
    let read = read_admission(&f, &mut recovery);
    recovery.run_read(read).unwrap();
    assert_eq!(recovery.class_data(), Some(Class::CleanUnstarted));
    assert!(recovery.ready_repair().is_err() && recovery.ready_scratch().is_err());
    assert!(!recovery.install_into(&mut f.transaction).unwrap());
    assert!(!dir.join("text.wal").exists() && !dir.join("text.wal.stage.v1").exists());
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), old);
    drop(recovery);

    fail_stage(&mut f, V2TextRetainTestCut::WalStaged);
    let original_wal = std::fs::read(dir.join("text.wal.stage.v1")).unwrap();
    let mut recovery = prepare_recovery(&f);
    let read = read_admission(&f, &mut recovery);
    recovery.fail_after_ready_for_test();
    assert!(recovery.run_read(read).is_err());
    assert_eq!(recovery.class_data(), Some(Class::CompleteStage));
    assert!(recovery.ready_repair().is_err() && recovery.ready_scratch().is_err());
    assert_eq!(
        std::fs::read(dir.join("text.wal.stage.v1")).unwrap(),
        original_wal
    );
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), old);
    drop(recovery);
    no_new_nonce(&mut f);
    let stage = stage_admission(&f);
    f.transaction.run_stage(stage).unwrap();
    assert_eq!(std::fs::read(dir.join("text.wal")).unwrap(), original_wal);
    let commit = commit_admission(&f);
    f.transaction.run_commit(commit).unwrap();
    assert!(f.transaction.saved_retained_action().is_ok());
}

#[test]
fn actual_original_torn_wal_scratch_recovers_lost_unlink_without_resealing_or_selecting() {
    let mut f = retain_fixture(1, 0x7f710);
    let dir = target(&f);
    let old = std::fs::read(dir.join("head.bin")).unwrap();
    fail_stage(&mut f, V2TextRetainTestCut::WalStaged);
    let path = dir.join("text.wal.stage.v1");
    let expected = std::fs::read(&path).unwrap();
    // Truncate the SAME actual stage File; the recorded native attempt is unchanged.
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len((expected.len() / 2) as u64).unwrap();
    drop(file);
    let prefix = std::fs::read(&path).unwrap();
    let mut recovery = prepare_recovery(&f);
    let read = read_admission(&f, &mut recovery);
    recovery.run_read(read).unwrap();
    assert_eq!(recovery.class_data(), Some(Class::FirstWalScratch));
    assert_eq!(std::fs::read(&path).unwrap(), prefix);
    assert!(recovery.ready_repair().is_err());
    let scratch = scratch_admission(&f, &recovery);
    recovery.set_work_fault_for_test(V2TextRetainRecoveryTestCut::ScratchUnlinked);
    assert!(recovery.run_scratch(scratch).is_err());
    assert!(!path.exists());
    assert!(!recovery.has_completed_scratch_data());
    assert!(!recovery.install_into(&mut f.transaction).unwrap());
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), old);
    // Only this SAME Ready carries the actual unlink bit through the lost barrier.
    let scratch = scratch_admission(&f, &recovery);
    recovery.run_scratch(scratch).unwrap();
    assert!(recovery.has_completed_scratch_data());
    assert!(recovery.install_into(&mut f.transaction).unwrap());
    assert!(recovery.install_into(&mut f.transaction).unwrap());
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), old);
    drop(recovery);
    no_new_nonce(&mut f);
    let stage = stage_admission(&f);
    f.transaction.run_stage(stage).unwrap();
    assert_eq!(std::fs::read(dir.join("text.wal")).unwrap(), expected);
    let commit = commit_admission(&f);
    f.transaction.run_commit(commit).unwrap();
    assert!(f.transaction.saved_retained_action().is_ok());
}

#[test]
fn actual_authenticated_main_repairs_only_listed_partial_outputs_and_installs_same_ready() {
    let mut f = retain_fixture(2, 0x7f720);
    let dir = target(&f);
    let old = std::fs::read(dir.join("head.bin")).unwrap();
    fail_stage(&mut f, V2TextRetainTestCut::ArtifactCreated);
    let main = std::fs::read(dir.join("text.wal")).unwrap();
    let mut recovery = prepare_recovery(&f);
    let read = read_admission(&f, &mut recovery);
    recovery.run_read(read).unwrap();
    assert_eq!(recovery.class_data(), Some(Class::CompleteMain));
    assert!(recovery.ready_scratch().is_err());
    let repair = repair_admission(&f, &recovery);
    recovery.set_work_fault_for_test(V2TextRetainRecoveryTestCut::RepairReadyCached);
    assert!(recovery.run_repair(repair).is_err());
    assert!(recovery.has_completed_repair_data());
    assert!(f.transaction.ready_selection().is_err());
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), old);
    assert_eq!(std::fs::read(dir.join("text.wal")).unwrap(), main);
    assert!(recovery.install_into(&mut f.transaction).unwrap());
    assert!(f.transaction.ready_selection().is_ok());
    assert!(f.transaction.saved_retained_action().is_err());
    drop(recovery);
    no_new_nonce(&mut f);
    let commit = commit_admission(&f);
    f.transaction.run_commit(commit).unwrap();
    assert_eq!(std::fs::read(dir.join("text.wal")).unwrap(), main);
    assert!(f.transaction.saved_retained_action().is_ok());
}

#[test]
fn actual_main_repair_preserves_wrong_or_oversize_listed_child_without_ready_or_selection() {
    let mut f = retain_fixture(0, 0x7f730);
    let dir = target(&f);
    let old = std::fs::read(dir.join("head.bin")).unwrap();
    fail_stage(&mut f, V2TextRetainTestCut::ArtifactCreated);
    // The actual cut creates one empty listed child before its first byte, not a
    // fabricated descriptor. Find that exact empty file among the real page objects.
    let child = std::fs::read_dir(dir.join("text.pages"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.is_file() && std::fs::metadata(path).unwrap().len() == 0)
        .expect("actual ArtifactCreated cut left its listed empty File");
    let main = std::fs::read(dir.join("text.wal")).unwrap();
    let mut recovery = prepare_recovery(&f);
    let read = read_admission(&f, &mut recovery);
    recovery.run_read(read).unwrap();
    for bytes in [vec![0xff; 8], vec![0xee; 600 * 1024]] {
        std::fs::write(&child, &bytes).unwrap();
        let repair = repair_admission(&f, &recovery);
        assert!(recovery.run_repair(repair).is_err());
        assert!(!recovery.has_completed_repair_data());
        assert!(!recovery.install_into(&mut f.transaction).unwrap());
        assert_eq!(std::fs::read(&child).unwrap(), bytes);
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), old);
        assert_eq!(std::fs::read(dir.join("text.wal")).unwrap(), main);
    }
    assert!(f.transaction.ready_selection().is_err());
    assert!(f.transaction.saved_retained_action().is_err());
}

#[test]
fn actual_recovery_whole_plan_and_shared_capture_effect_claims_remain_affine() {
    let mut f = retain_fixture(0, 0x7f740);
    fail_stage(&mut f, V2TextRetainTestCut::ArtifactCreated);
    let mut recovery = prepare_recovery(&f);
    ordinary_stage_is_refused(&f);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    assert!(PreparedV2TextRetainRecovery::prepare(store, owner.source(), &f.transaction).is_err());
    let capture =
        PreparedV2TextRetainRecoveryCapture::prepare(store, owner.source(), &recovery).unwrap();
    assert!(
        PreparedV2TextRetainRecoveryCapture::prepare(store, owner.source(), &recovery).is_err()
    );
    drop(capture);
    drop(owner);
    let read = read_admission(&f, &mut recovery);
    let owner = f.original.owner.borrow_current().unwrap();
    assert!(
        PreparedV2TextRetainRecoveryCapture::prepare(store, owner.source(), &recovery).is_err()
    );
    drop(owner);
    recovery.run_read(read).unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let lease = PreparedV2TextRetainRepairLease::prepare(
        store,
        owner.source(),
        recovery.ready_repair().unwrap(),
    )
    .unwrap();
    assert!(PreparedV2TextRetainRepairLease::prepare(
        store,
        owner.source(),
        recovery.ready_repair().unwrap(),
    )
    .is_err());
    assert!(
        PreparedV2TextRetainRecoveryCapture::prepare(store, owner.source(), &recovery).is_err()
    );
    drop(lease);
    drop(owner);
    let repair = repair_admission(&f, &recovery);
    let owner = f.original.owner.borrow_current().unwrap();
    assert!(
        PreparedV2TextRetainRecoveryCapture::prepare(store, owner.source(), &recovery).is_err()
    );
    drop(owner);
    recovery.run_repair(repair).unwrap();
    let capture = {
        let owner = f.original.owner.borrow_current().unwrap();
        PreparedV2TextRetainRecoveryCapture::prepare(store, owner.source(), &recovery).unwrap()
    };
    drop(recovery);
    // The last active precursor keeps the WHOLE phase exclusion even though it has
    // no captured source yet. Ordinary Stage cannot race its original plan/copies.
    ordinary_stage_is_refused(&f);
    let owner = f.original.owner.borrow_current().unwrap();
    assert!(PreparedV2TextRetainRecovery::prepare(store, owner.source(), &f.transaction).is_err());
    drop(capture);
    // Fresh actual Stage admission requires every recovery alias to drain. Dropping
    // this UNSTARTED marker changes no file and cannot issue a Saved receipt.
    drop(stage_admission(&f));
    let next =
        PreparedV2TextRetainRecovery::prepare(store, owner.source(), &f.transaction).unwrap();
    drop(next);
}

#[test]
fn actual_missing_recorded_wal_attempt_never_becomes_clean_or_unlink_authority() {
    let mut f = retain_fixture(0, 0x7f750);
    fail_stage(&mut f, V2TextRetainTestCut::WalStaged);
    let dir = target(&f);
    let path = dir.join("text.wal.stage.v1");
    let old = std::fs::read(dir.join("head.bin")).unwrap();
    // Remove the original attempt before any recovery operation. This is deliberately
    // different from the same-plan cached successful unlink in the scratch test above.
    std::fs::remove_file(path).unwrap();
    let mut recovery = prepare_recovery(&f);
    let read = read_admission(&f, &mut recovery);
    assert!(recovery.run_read(read).is_err());
    assert_eq!(recovery.class_data(), None);
    assert!(recovery.ready_scratch().is_err() && recovery.ready_repair().is_err());
    assert!(!recovery.install_into(&mut f.transaction).unwrap());
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), old);
}

#[test]
fn actual_late_recovery_capture_refusal_keeps_files_lease_and_writer_end_until_outside_drop() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = retain_fixture(0, 0x7f760);
    let mut recovery = prepare_recovery(&f);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let head = owner.source().source().capture.target.join("head.bin");
    let revocation = owner.warm_scope().unwrap().revocation();
    recovery.set_after_capture_files_for_test(Box::new(move || revocation.revoke()));
    let resources = recovery.resources().clone();
    let mut capture = Some(
        PreparedV2TextRetainRecoveryCapture::prepare(store, owner.source(), &recovery).unwrap(),
    );
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_recovery_read_into(
                &f.lane,
                &f.predecessor,
                &mut recovery,
                &mut capture,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none());
    assert!(capture.as_ref().unwrap().has_source_for_test());
    assert!(f.lane.workspace_active_for_test());
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&head)
        .is_err());
    drop(owner);
    drop(resources);
    let RetainFixture {
        transaction,
        lane,
        predecessor,
        original,
    } = f;
    let TextFixture {
        owner,
        layout,
        _directory: directory,
    } = original;
    drop(owner);
    drop(layout);
    assert!(matches!(
        catcoms_storage::acquire_vault_session(directory.path()),
        Err(catcoms_storage::StorageError::VaultBusy)
    ));
    // The actual captured Source is first, then native lease, then real writer End.
    // Both original DATA owners remain alive and cannot hide a dormant FD/session.
    drop(capture.take());
    assert!(!lane.workspace_active_for_test());
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(head)
        .unwrap();
    drop(file);
    let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    assert_eq!(recovery.class_data(), None);
    assert!(transaction.ready_selection().is_err());
    drop(session);
    drop(predecessor);
    drop(lane);
    drop(transaction);
    drop(recovery);
}

#[test]
fn actual_failed_capture_retry_uses_same_writer_end_without_renewing_its_source_claim() {
    let f = retain_fixture(0, 0x7f770);
    let mut recovery = prepare_recovery(&f);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let head = owner.source().source().capture.target.join("head.bin");
    let held_name = head.with_file_name("head-held-by-recovery-test");
    let rename_from = head.clone();
    let rename_to = held_name.clone();
    recovery.set_after_capture_files_for_test(Box::new(move || {
        std::fs::rename(rename_from, rename_to).unwrap();
    }));
    let resources = recovery.resources().clone();
    let mut capture = Some(
        PreparedV2TextRetainRecoveryCapture::prepare(store, owner.source(), &recovery).unwrap(),
    );
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_recovery_read_into(
                &f.lane,
                &f.predecessor,
                &mut recovery,
                &mut capture,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none() && capture.as_ref().unwrap().has_source_for_test());
    assert!(f.lane.workspace_active_for_test());
    // Restore that SAME still-held File. We neither reconstruct nor accept a new O.
    std::fs::rename(held_name, head).unwrap();
    assert!(
        PreparedV2TextRetainRecoveryCapture::prepare(store, owner.source(), &recovery).is_err()
    );
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_retain_recovery_read_into(
                &f.lane,
                &f.predecessor,
                &mut recovery,
                &mut capture,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(capture.is_none() && f.lane.workspace_active_for_test());
    recovery.run_read(admitted.take().unwrap()).unwrap();
    assert_eq!(recovery.class_data(), Some(Class::CleanUnstarted));
    assert!(!f.lane.workspace_active_for_test());
}

#[test]
fn actual_stage_recovery_phase_interlock_covers_both_orderings_and_last_cleanup_tail() {
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let f = retain_fixture(0, 0x7f780);
    let mut stage = stage_admission(&f);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    assert!(PreparedV2TextRetainRecovery::prepare(store, owner.source(), &f.transaction).is_err());
    let (paused_tx, paused_rx) = sync_channel(0);
    let (release_tx, release_rx) = sync_channel(0);
    stage.set_after_writer_gate_clear_for_test(Box::new(move || {
        paused_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    }));
    // Drop a REAL unstarted admitted owner on another thread. Its physical lease is
    // already gone at this actual End hook, but the Stage phase fence must be LAST.
    let worker = std::thread::spawn(move || drop(stage));
    paused_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    assert!(PreparedV2TextRetainRecovery::prepare(store, owner.source(), &f.transaction).is_err());
    assert!(f.lane.workspace_active_for_test());
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    assert!(!f.lane.workspace_active_for_test());
    let recovery =
        PreparedV2TextRetainRecovery::prepare(store, owner.source(), &f.transaction).unwrap();
    // Reverse ordering: the actual whole Recovery claim wins before Stage's final
    // cached CAS, so no marker may escape. This is not a caller-installed state bit.
    ordinary_stage_is_refused(&f);
    drop(recovery);
    drop(stage_admission(&f));
    let dir = target(&f);
    assert!(!dir.join("text.wal").exists() && !dir.join("text.wal.stage.v1").exists());
    assert!(f.transaction.saved_retained_action().is_err());
}
