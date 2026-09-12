//! Cross-namespace inventory/cleanup regressions. These exercise real sealed journal and recovery
//! files; no registry or live owner is needed to account historical bytes after a restart.

use super::*;
use catcoms_mls::MlsDevice;
use catcoms_replication::{InheritedCheckpoint, RecoveryReason, RecoverySnapshot};
use catcoms_rt::ManualClock;
use catcoms_wire::DocType;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(17)
}

fn open(path: &Path) -> ServerStore {
    ServerStore::open(path, b"inventory-union", &mut rng()).unwrap()
}

fn fixture() -> (MlsDevice, ServerGroup, LogicalDocument) {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let doc = LogicalDocument::new(
        group.group_id(),
        DocType::StudioObject,
        b"private-cat".to_vec(),
    )
    .unwrap();
    (owner, group, doc)
}

fn collect(mut scan: EpochStorageScan<'_>) -> Result<EpochStorageInventory, AppError> {
    let coverage = scan.coverage();
    let mut before = EpochStorageScanProgress::default();
    loop {
        let after = scan.step()?;
        assert!(after.visited_entries - before.visited_entries <= 64);
        assert!(
            (after.recovery_records + after.owner_receipt_records)
                - (before.recovery_records + before.owner_receipt_records)
                <= 1
        );
        if coverage == EpochInventoryCoverage::RecoveryOnly {
            assert_eq!(after.owner_receipt_records, 0);
        }
        before = after;
        if after.complete {
            break;
        }
    }
    let result = scan.finish()?;
    assert_eq!(result.coverage(), coverage);
    Ok(result)
}

// These fixtures contain only the two covered P1 families. This is not the future production
// coordinator: it must inventory other types and retain exclusive access through admission too.
fn budget(store: &mut ServerStore, server: u64, doc: &LogicalDocument) -> EpochStorageBudget {
    let inventory = collect(store.scan_epoch_storage().unwrap()).unwrap();
    EpochStorageBudget::from_inventory(
        StorageScope::new(server, &doc.server_id).unwrap(),
        inventory
            .records_for_server(server, &doc.server_id)
            .unwrap(),
    )
    .unwrap()
}

fn prepare(
    store: &mut ServerStore,
    server: u64,
    owner: &MlsDevice,
    group: &ServerGroup,
    doc: &LogicalDocument,
) -> Receipt {
    let signed = Receipt::sign(
        doc.clone(),
        0,
        [11; 32],
        [12; 32],
        group.epoch(),
        InheritedCheckpoint::EpochZero,
        owner,
    )
    .unwrap();
    let mut budget = budget(store, server, doc);
    store
        .prepare_epoch_owner_receipt(
            server,
            signed.clone(),
            group,
            group.epoch(),
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    signed
}

fn recovery(store: &mut ServerStore, server: u64, doc: &LogicalDocument, epoch: u64) {
    store
        .update_epoch_recovery(
            server,
            doc,
            EpochRecoveryAction::Stage(RecoverySnapshot {
                doc_type: doc.doc_type,
                logical_key: doc.logical_key.clone(),
                epoch,
                base_close_record_hash: None,
                reason: RecoveryReason::Excluded,
                projection: vec![epoch as u8],
                tombstones: vec![],
                elements: vec![],
                conflicts: vec![],
                applied_ops: vec![],
            }),
            &ManualClock::new(epoch),
            &mut rng(),
        )
        .unwrap();
}

fn recovery_path(root: &Path, store: &ServerStore, server: u64, doc: &LogicalDocument) -> PathBuf {
    let record = store
        .epoch_recovery_inventory_record(server, doc)
        .unwrap()
        .unwrap();
    root.join("servers")
        .join(format!("{}.recovery", hex::encode(record.id)))
}

fn clean(
    mut cleanup: EpochStorageCleanup<'_>,
) -> Result<(EpochStorageCleanupProgress, EpochStorageInventory), AppError> {
    let coverage = cleanup.coverage();
    let mut before = EpochStorageCleanupProgress::default();
    loop {
        let after = cleanup.step()?;
        assert!(after.visited_entries - before.visited_entries <= 64);
        before = after;
        if after.complete {
            break;
        }
    }
    let scan = cleanup.into_inventory()?;
    assert_eq!(scan.coverage(), coverage);
    Ok((before, collect(scan)?))
}

#[test]
fn mixed_discovery_preserves_scope_pools_and_pending_decisions_without_a_registry() {
    let root = tempfile::tempdir().unwrap();
    let (owner, group, doc) = fixture();
    let mut store = open(root.path());
    let signed = prepare(&mut store, 7, &owner, &group, &doc);
    for epoch in 0..3 {
        recovery(&mut store, 7, &doc, epoch);
    }
    let owner_record = store
        .epoch_owner_receipt_inventory_record(7, &doc)
        .unwrap()
        .unwrap();
    let recovery_record = store
        .epoch_recovery_inventory_record(7, &doc)
        .unwrap()
        .unwrap();
    assert_eq!(owner_record.document, recovery_record.document);
    assert_ne!(owner_record.id, recovery_record.id);
    // Same local mount id with a different full MLS group and same key, plus another mount id.
    let (other_owner, other_group, other_doc) = fixture();
    prepare(&mut store, 7, &other_owner, &other_group, &other_doc);
    prepare(&mut store, 8, &owner, &group, &doc);
    drop(store);
    let mut store = open(root.path());
    let inventory = collect(store.scan_epoch_storage().unwrap()).unwrap();
    assert_eq!(
        inventory.coverage(),
        EpochInventoryCoverage::RecoveryAndOwnerReceipts
    );
    assert_eq!(inventory.records().len(), 4);
    assert_eq!(
        inventory.records_for_server(7, &doc.server_id).unwrap(),
        vec![recovery_record, owner_record]
    );
    assert_eq!(
        inventory
            .records_for_server(7, &other_doc.server_id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        inventory
            .records_for_server(8, &doc.server_id)
            .unwrap()
            .len(),
        1
    );
    assert!(inventory
        .records_for_server(9, &doc.server_id)
        .unwrap()
        .is_empty());
    assert_eq!(
        store.load_epoch_owner_receipts(7, &doc).unwrap().pending(),
        Some(&signed)
    );
    assert!(!format!(
        "{inventory:?} {:?}",
        inventory.records().collect::<Vec<_>>()
    )
    .contains("private-cat"));
    assert!(recovery_record.footprint.settlement > 0);
    assert!(owner_record.footprint.protocol > 0);
}

#[test]
fn legacy_recovery_coverage_ignores_owner_finals_temporaries_and_owner_only_aliases() {
    let root = tempfile::tempdir().unwrap();
    let (owner, group, doc) = fixture();
    let mut store = open(root.path());
    prepare(&mut store, 7, &owner, &group, &doc);
    recovery(&mut store, 7, &doc, 0);
    let owner_path = store.epoch_owner_path(&scope_bytes(7, &doc).unwrap());
    let saved = fs::read(&owner_path).unwrap();
    let owner_temp = staging_candidate(&owner_path, 900);
    let recovery_temp = staging_candidate(&recovery_path(root.path(), &store, 7, &doc), 901);
    let alias = root
        .path()
        .join("servers")
        .join(format!("{}.OWNER-RECEIPTS", "aa".repeat(32)));
    fs::write(&owner_temp, b"unfinished").unwrap();
    fs::write(&recovery_temp, []).unwrap();
    fs::write(&alias, []).unwrap();
    // Keep legacy type names compiling, while preserving the explicit recovery-only coverage.
    let scan: EpochRecoveryScan<'_> = store.scan_epoch_recovery().unwrap();
    let inventory: EpochRecoveryInventory = collect(scan).unwrap();
    assert_eq!(inventory.coverage(), EpochInventoryCoverage::RecoveryOnly);
    assert_eq!(inventory.records().len(), 1);
    assert_eq!(inventory.orphans().len(), 1);
    let cleanup: EpochRecoveryCleanup<'_> = store.cleanup_epoch_recovery_staging().unwrap();
    let (progress, inventory) = clean(cleanup).unwrap();
    assert_eq!(progress.removed_files, 1);
    assert_eq!(inventory.coverage(), EpochInventoryCoverage::RecoveryOnly);
    assert_eq!(fs::read(&owner_path).unwrap(), saved);
    assert_eq!(fs::read(&owner_temp).unwrap(), b"unfinished");
    assert!(alias.exists());
    assert!(collect(store.scan_epoch_storage().unwrap()).is_err());
    // Combined cleanup may remove canonical temporaries before reaching the alias, but never
    // the malformed alias or the saved journal. A partial failure grants no accounting credit.
    assert!(clean(store.cleanup_epoch_storage_staging().unwrap()).is_err());
    assert!(alias.exists());
    assert_eq!(fs::read(owner_path).unwrap(), saved);
}

#[test]
fn identical_digest_text_in_the_other_namespace_cannot_assign_orphan_ownership() {
    let root = tempfile::tempdir().unwrap();
    let (owner, group, doc) = fixture();
    let mut store = open(root.path());
    prepare(&mut store, 7, &owner, &group, &doc);
    recovery(&mut store, 7, &doc, 0);
    let owner_record = store
        .epoch_owner_receipt_inventory_record(7, &doc)
        .unwrap()
        .unwrap();
    let recovery_record = store
        .epoch_recovery_inventory_record(7, &doc)
        .unwrap()
        .unwrap();
    for (id, suffix) in [
        (owner_record.id, "recovery"),
        (recovery_record.id, "owner-receipts"),
    ] {
        let fake_destination = root
            .path()
            .join("servers")
            .join(format!("{}.{suffix}", hex::encode(id)));
        fs::write(staging_candidate(&fake_destination, 900), []).unwrap();
    }
    let inventory = collect(store.scan_epoch_storage().unwrap()).unwrap();
    assert_eq!(inventory.unresolved_orphans(), 2);
    assert_eq!(inventory.orphans().len(), 2);
    for server in [7, 8] {
        assert!(inventory
            .records_for_server(server, &doc.server_id)
            .is_err());
    }
    let (progress, after) = clean(store.cleanup_epoch_storage_staging().unwrap()).unwrap();
    assert_eq!(progress.removed_files, 2);
    assert_eq!(after.unresolved_orphans(), 0);
    assert_eq!(after.records().len(), 2);
}

#[test]
fn failed_journal_write_restarts_cleans_both_families_reconciles_and_retries() {
    let root = tempfile::tempdir().unwrap();
    let (owner, group, doc) = fixture();
    let mut store = open(root.path());
    let signed = prepare(&mut store, 7, &owner, &group, &doc);
    for epoch in 0..3 {
        recovery(&mut store, 7, &doc, epoch);
    }
    let owner_path = store.epoch_owner_path(&scope_bytes(7, &doc).unwrap());
    let recovery_path = recovery_path(root.path(), &store, 7, &doc);
    let owner_before = fs::read(&owner_path).unwrap();
    let recovery_before = fs::read(&recovery_path).unwrap();
    let mut accounted = budget(&mut store, 7, &doc);
    let orphan = staging_candidate(&owner_path, 900);
    assert!(store
        .update_epoch_owner_with_writer(
            7,
            &doc,
            &mut rng(),
            &mut accounted,
            |journal| journal.mark_published(signed.hash()).map_err(invalid),
            |_, bytes| {
                fs::write(&orphan, bytes).unwrap();
                Err(AppError::Io("before rename".into()))
            }
        )
        .is_err());
    assert!(accounted.requires_reconciliation());
    fs::write(staging_candidate(&recovery_path, 901), b"partial recovery").unwrap();
    drop(store);
    let mut store = open(root.path());
    let inventory = collect(store.scan_epoch_storage().unwrap()).unwrap();
    assert_eq!(inventory.orphans().len(), 2);
    let scope = StorageScope::new(7, &doc.server_id).unwrap();
    accounted
        .reconcile(
            &scope,
            inventory.records_for_server(7, &doc.server_id).unwrap(),
        )
        .unwrap();
    let before_cleanup = accounted.usage();
    let (progress, inventory) = clean(store.cleanup_epoch_storage_staging().unwrap()).unwrap();
    assert_eq!(progress.removed_files, 2);
    assert_eq!(
        inventory.coverage(),
        EpochInventoryCoverage::RecoveryAndOwnerReceipts
    );
    assert_eq!(accounted.usage(), before_cleanup); // Removing names does not refund this budget.
    assert_eq!(fs::read(&owner_path).unwrap(), owner_before);
    assert_eq!(fs::read(&recovery_path).unwrap(), recovery_before);
    accounted
        .reconcile(
            &scope,
            inventory.records_for_server(7, &doc.server_id).unwrap(),
        )
        .unwrap();
    assert_eq!(
        before_cleanup.settlement - accounted.usage().settlement,
        progress.removed_ciphertext_bytes
    );
    assert!(store
        .load_epoch_recovery(7, &doc)
        .unwrap()
        .staged()
        .is_some());
    assert_eq!(
        store.load_epoch_owner_receipts(7, &doc).unwrap().pending(),
        Some(&signed)
    );
    let saved = store
        .mark_epoch_owner_receipt_published(7, &doc, signed.hash(), &mut rng(), &mut accounted)
        .unwrap();
    assert_eq!(saved.published(), Some(&signed));
    assert!(saved.pending().is_none());
}

#[test]
fn a_first_write_orphan_never_becomes_a_published_owner_decision() {
    let root = tempfile::tempdir().unwrap();
    let (owner, group, doc) = fixture();
    let mut store = open(root.path());
    let signed = Receipt::sign(
        doc.clone(),
        0,
        [11; 32],
        [12; 32],
        group.epoch(),
        InheritedCheckpoint::EpochZero,
        &owner,
    )
    .unwrap();
    let mut accounted = budget(&mut store, 7, &doc);
    let path = store.epoch_owner_path(&scope_bytes(7, &doc).unwrap());
    let orphan = staging_candidate(&path, 900);
    assert!(store
        .prepare_epoch_owner_with_writer(
            7,
            signed.clone(),
            &group,
            group.epoch(),
            &mut rng(),
            &mut accounted,
            |_, bytes| {
                fs::write(&orphan, bytes).unwrap();
                Err(AppError::Io("first write interrupted".into()))
            }
        )
        .is_err());
    drop(store);
    let mut store = open(root.path());
    let inventory = collect(store.scan_epoch_storage().unwrap()).unwrap();
    assert_eq!(inventory.unresolved_orphans(), 1);
    assert!(inventory.records_for_server(7, &doc.server_id).is_err());
    assert!(store
        .load_epoch_owner_receipts(7, &doc)
        .unwrap()
        .pending()
        .is_none());
    let (_, inventory) = clean(store.cleanup_epoch_storage_staging().unwrap()).unwrap();
    assert!(!path.exists() && !orphan.exists());
    accounted
        .reconcile(
            &StorageScope::new(7, &doc.server_id).unwrap(),
            inventory.records_for_server(7, &doc.server_id).unwrap(),
        )
        .unwrap();
    let saved = store
        .prepare_epoch_owner_receipt(
            7,
            signed.clone(),
            &group,
            group.epoch(),
            &mut rng(),
            &mut accounted,
        )
        .unwrap();
    assert_eq!(saved.pending(), Some(&signed));
}

#[test]
fn owner_family_caps_scope_domains_and_canonical_names_are_not_weakened_by_union() {
    for mode in [
        "oversize",
        "corrupt",
        "wrong-hash",
        "wrong-family",
        "directory",
        "uppercase",
    ] {
        let root = tempfile::tempdir().unwrap();
        let (owner, group, doc) = fixture();
        let mut store = open(root.path());
        prepare(&mut store, 7, &owner, &group, &doc);
        let path = store.epoch_owner_path(&scope_bytes(7, &doc).unwrap());
        match mode {
            "oversize" => {
                OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_len(MAX_SEALED_BYTES as u64 + 1)
                    .unwrap();
            }
            "corrupt" => {
                fs::write(&path, [0; 40]).unwrap();
            }
            "wrong-hash" => {
                fs::rename(
                    &path,
                    path.with_file_name(format!("{}.owner-receipts", "01".repeat(32))),
                )
                .unwrap();
            }
            "wrong-family" => {
                recovery(&mut store, 7, &doc, 0);
                // An authenticated recovery envelope in an owner-named file is not a journal.
                fs::copy(recovery_path(root.path(), &store, 7, &doc), &path).unwrap();
            }
            "directory" => {
                fs::remove_file(&path).unwrap();
                fs::create_dir(&path).unwrap();
            }
            "uppercase" => {
                let temp = path.with_file_name("rename-intermediate");
                fs::rename(&path, &temp).unwrap();
                fs::rename(
                    &temp,
                    path.with_file_name(
                        path.file_name()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_ascii_uppercase(),
                    ),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            collect(store.scan_epoch_storage().unwrap()).is_err(),
            "{mode}"
        );
    }
}
