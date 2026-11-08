//! Actual warm recovery IO/Stop custody, separate from classifier DATA and parser tests.
//! The pause runs with a real helper-local File, not a manually set busy/count flag.
use super::*;

#[test]
fn actual_repair_output_file_and_vault_lease_outlive_store_until_finite_error_cleanup() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let mut f = retain_fixture(0, 0x7f790);
    fail_stage(&mut f, V2TextRetainTestCut::ArtifactCreated);
    let dir = target(&f);
    let child = std::fs::read_dir(dir.join("text.pages"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.is_file() && std::fs::metadata(path).unwrap().len() == 0)
        .unwrap();
    let mut recovery = prepare_recovery(&f);
    let read = read_admission(&f, &mut recovery);
    recovery.run_read(read).unwrap();
    let repair = repair_admission(&f, &recovery);
    let (paused_tx, paused_rx) = sync_channel(0);
    let (release_tx, release_rx) = sync_channel(0);
    recovery.set_work_pause_for_test(
        V2TextRetainRecoveryTestCut::RepairOutputOpened,
        Box::new(move || {
            paused_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        }),
    );
    recovery.set_work_fault_for_test(V2TextRetainRecoveryTestCut::RepairOutputOpened);
    let worker = std::thread::spawn(move || {
        let result = recovery.run_repair(repair);
        (recovery, result)
    });
    paused_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&child)
        .is_err());
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
    drop(lane);
    drop(predecessor);
    drop(layout);
    assert!(matches!(
        catcoms_storage::acquire_vault_session(directory.path()),
        Err(catcoms_storage::StorageError::VaultBusy)
    ));
    release_tx.send(()).unwrap();
    let (recovery, result) = worker.join().unwrap();
    assert!(result.is_err());
    assert!(!recovery.has_completed_repair_data());
    assert!(transaction.ready_selection().is_err());
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(child)
        .unwrap();
    drop(file);
    let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    // Actual DATA debt deliberately remains alive; neither contains a dormant File
    // or silently keeps the physical lease after helper cleanup and actual End.
    drop(session);
    drop(recovery);
    drop(transaction);
}

#[test]
fn actual_scope_stop_after_recovery_read_or_repair_or_scratch_admission_preserves_finite_result() {
    for role in 0..3 {
        let mut f = retain_fixture(0, 0x7f7a0 + role * 16);
        let dir = target(&f);
        let old = std::fs::read(dir.join("head.bin")).unwrap();
        if role == 2 {
            fail_stage(&mut f, V2TextRetainTestCut::WalStaged);
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(dir.join("text.wal.stage.v1"))
                .unwrap();
            let length = file.metadata().unwrap().len();
            file.set_len(length / 2).unwrap();
            drop(file);
        } else {
            fail_stage(&mut f, V2TextRetainTestCut::ArtifactCreated);
        }
        let mut recovery = prepare_recovery(&f);
        let read = read_admission(&f, &mut recovery);
        if role == 0 {
            // This hook runs AFTER actual admission, with real captured Files owned
            // by its lease marker. Source loss must not erase completed classified DATA.
            let owner = f.original.owner.borrow_current().unwrap();
            let revocation = owner.warm_scope().unwrap().revocation();
            recovery.set_after_run_source_for_test(Box::new(move || revocation.revoke()));
            recovery.run_read(read).unwrap();
            assert_eq!(recovery.class_data(), Some(Class::CompleteMain));
            assert!(recovery.ready_repair().is_ok());
            assert!(!recovery.has_completed_repair_data());
        } else if role == 1 {
            recovery.run_read(read).unwrap();
            let repair = repair_admission(&f, &recovery);
            let owner = f.original.owner.borrow_current().unwrap();
            owner.warm_scope().unwrap().revocation().revoke();
            drop(owner);
            recovery.run_repair(repair).unwrap();
            assert!(recovery.has_completed_repair_data());
            assert!(recovery.install_into(&mut f.transaction).unwrap());
            assert!(f.transaction.ready_selection().is_ok());
        } else {
            recovery.run_read(read).unwrap();
            let scratch = scratch_admission(&f, &recovery);
            let owner = f.original.owner.borrow_current().unwrap();
            owner.warm_scope().unwrap().revocation().revoke();
            drop(owner);
            recovery.run_scratch(scratch).unwrap();
            assert!(recovery.has_completed_scratch_data());
            assert!(recovery.install_into(&mut f.transaction).unwrap());
            assert!(!dir.join("text.wal.stage.v1").exists());
        }
        assert!(!f.lane.workspace_active_for_test());
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), old);
        assert!(f.transaction.saved_retained_action().is_err());
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
        // A previously classified/historical Ready cannot renew permission after Stop.
        if let Ok(ready) = recovery.ready_repair() {
            assert!(
                PreparedV2TextRetainRepairLease::prepare(store, owner.source(), ready).is_err()
            );
        }
    }
}
