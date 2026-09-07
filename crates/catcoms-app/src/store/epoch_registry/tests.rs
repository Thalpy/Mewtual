use super::*;
use catcoms_replication::registry::{PointerKey, RegistryOp};
use catcoms_replication::{InheritedCheckpoint, Receipt};
use catcoms_wire::DocType;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

mod local;
mod settlement;

const SERVER: u64 = 73;
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(710)
}
fn open(path: &Path) -> ServerStore {
    ServerStore::open(path, b"registry-vault-test", &mut rng()).unwrap()
}

struct Fixture {
    device: MlsDevice,
    group: ServerGroup,
    key: PointerKey,
    source: RegistryEpoch,
    document: LogicalDocument,
}
impl Fixture {
    fn new() -> Self {
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let key = PointerKey::new(DocType::StudioObject, b"private-registry-cat".to_vec()).unwrap();
        let document = registry_document(&group.group_id(), key.bucket()).unwrap();
        let source = RegistryEpoch::new(&group, key.bucket(), device.device_id()).unwrap();
        Self {
            device,
            group,
            key,
            source,
            document,
        }
    }
    fn op(&mut self, n: u8) -> SealedOp {
        let op = RegistryOp::Put {
            key: self.key.clone(),
            epoch: u64::from(n),
        }
        .domain_op(&self.group.group_id(), [n; 16])
        .unwrap();
        self.source
            .edit(&self.device, &self.group, &mut rng(), &op)
            .unwrap()
    }
    fn receipt(&self, close: u8) -> Receipt {
        let seed = self
            .source
            .projection()
            .unwrap()
            .checkpoint([close; 32])
            .unwrap();
        Receipt::sign(
            self.document.clone(),
            0,
            [close; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &self.device,
        )
        .unwrap()
    }
    fn ingest(
        &self,
        store: &mut ServerStore,
        op: &SealedOp,
        budget: &mut EpochStorageBudget,
    ) -> Result<(Admission, EpochRegistryState), AppError> {
        store.ingest_registry_epoch(
            SERVER,
            &self.group,
            self.key.bucket(),
            &self.device,
            op,
            &mut rng(),
            budget,
        )
    }
    fn load(&self, store: &ServerStore) -> Option<EpochRegistryState> {
        store
            .load_registry_epoch(SERVER, &self.group, self.key.bucket(), &self.device)
            .unwrap()
    }
    fn path(&self, store: &ServerStore) -> PathBuf {
        store.registry_epoch_path(&scope_bytes(SERVER, &self.document).unwrap())
    }
}

fn inventory(store: &mut ServerStore) -> EpochStorageInventory {
    let mut scan = store.scan_epoch_storage_with_registry().unwrap();
    while !scan.step().unwrap().complete {}
    scan.finish().unwrap()
}
fn budget(store: &mut ServerStore, f: &Fixture) -> EpochStorageBudget {
    EpochStorageBudget::from_inventory(
        StorageScope::new(SERVER, &f.group.group_id()).unwrap(),
        inventory(store)
            .records_for_server(SERVER, &f.group.group_id())
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn registry_store_inbound_edits_survive_reopen_without_exposing_content_in_debug() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let first = f.op(1);
    let second = f.op(2);
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &f);
    assert!(f.load(&store).is_none());
    let (result, state) = f.ingest(&mut store, &first, &mut budget).unwrap();
    assert_eq!(result, Admission::Accepted);
    assert_eq!(state.op_count(), 1);
    assert!(!format!("{state:?}").contains("private-registry-cat"));
    let disk = fs::read(f.path(&store)).unwrap();
    assert!(!disk
        .windows(b"private-registry-cat".len())
        .any(|w| w == b"private-registry-cat"));
    drop(store);
    let mut store = open(root.path());
    let mut budget = self::budget(&mut store, &f);
    assert_eq!(
        f.load(&store).unwrap().projection().unwrap().pointers[&f.key],
        1
    );
    f.ingest(&mut store, &second, &mut budget).unwrap();
    assert_eq!(f.load(&store).unwrap().op_count(), 2);
    let held = fs::read(f.path(&store)).unwrap();
    assert_eq!(
        f.ingest(&mut store, &first, &mut budget).unwrap().0,
        Admission::Duplicate
    );
    assert_eq!(
        fs::read(f.path(&store)).unwrap(),
        held,
        "exact retry flushes without re-encryption"
    );
}

#[test]
fn registry_store_seal_and_duplicate_quarantine_are_durable_and_leave_source_intact() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let first = f.op(1);
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &f);
    f.ingest(&mut store, &first, &mut budget).unwrap();
    let receipt = f.receipt(7);
    let (result, state) = store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            receipt.clone(),
            0,
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    assert_eq!(result, ReceiptIngest::Advanced);
    assert_eq!(state.phase(), EpochPhase::Closing);
    let late = f.op(2);
    assert_eq!(
        f.ingest(&mut store, &late, &mut budget).unwrap().0,
        Admission::Quarantined
    );
    let held = fs::read(f.path(&store)).unwrap();
    for _ in 0..3 {
        assert_eq!(
            f.ingest(&mut store, &late, &mut budget).unwrap().0,
            Admission::Quarantined
        );
    }
    assert_eq!(fs::read(f.path(&store)).unwrap(), held);
    drop(store);
    let mut store = open(root.path());
    let mut budget = self::budget(&mut store, &f);
    let state = f.load(&store).unwrap();
    assert_eq!(state.phase(), EpochPhase::Closing);
    assert_eq!(state.op_count(), 1);
    assert_eq!(state.quarantined_len(), 1);
    assert_eq!(state.projection().unwrap().pointers[&f.key], 1);
    assert_eq!(
        store
            .seal_registry_epoch(
                SERVER,
                &f.group,
                f.key.bucket(),
                &f.device,
                receipt,
                0,
                &mut rng(),
                &mut budget
            )
            .unwrap()
            .0,
        ReceiptIngest::Duplicate
    );
}

#[test]
fn registry_store_failed_write_and_post_rename_failure_require_reconciliation_and_retry() {
    for committed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut f = Fixture::new();
        let op = f.op(1);
        let mut store = open(root.path());
        let mut budget = budget(&mut store, &f);
        let result = store.update_registry_with_io(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            true,
            WritePurpose::Ordinary,
            &mut rng(),
            &mut budget,
            |unit, _| unit.ingest(&op, &f.group, &f.device).map_err(invalid),
            |path, bytes| {
                if committed {
                    atomic_write(path, bytes)?;
                }
                Err(AppError::Io("injected write/durability failure".into()))
            },
            |_, _| panic!("first write cannot sync-only"),
        );
        assert!(result.is_err());
        assert!(budget.requires_reconciliation());
        assert_eq!(f.load(&store).is_some(), committed);
        assert!(f.ingest(&mut store, &op, &mut budget).is_err());
        drop(store);
        let mut store = open(root.path());
        let mut budget = self::budget(&mut store, &f);
        let mut synced = false;
        let (result, state) = store
            .update_registry_with_io(
                SERVER,
                &f.group,
                f.key.bucket(),
                &f.device,
                true,
                WritePurpose::Ordinary,
                &mut rng(),
                &mut budget,
                |unit, _| unit.ingest(&op, &f.group, &f.device).map_err(invalid),
                |path, bytes| {
                    assert!(!committed);
                    atomic_write(path, bytes)
                },
                |path, size| {
                    synced = true;
                    sync_registry(path, size)
                },
            )
            .unwrap();
        assert_eq!(synced, committed);
        assert_eq!(
            result,
            if committed {
                Admission::Duplicate
            } else {
                Admission::Accepted
            }
        );
        assert_eq!(state.op_count(), 1);
    }
}

#[test]
fn registry_store_failed_duplicate_sync_or_writer_panic_cannot_acknowledge() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let op = f.op(1);
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &f);
    f.ingest(&mut store, &op, &mut budget).unwrap();
    let held = fs::read(f.path(&store)).unwrap();
    assert!(store
        .update_registry_with_io(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            true,
            WritePurpose::Ordinary,
            &mut rng(),
            &mut budget,
            |unit, _| unit.ingest(&op, &f.group, &f.device).map_err(invalid),
            |_, _| panic!("duplicate must not replace"),
            |_, _| Err(AppError::Io("injected sync failure".into()))
        )
        .is_err());
    assert!(budget.requires_reconciliation());
    assert_eq!(fs::read(f.path(&store)).unwrap(), held);
    let mut budget = self::budget(&mut store, &f);
    let next = f.op(2);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.update_registry_with_io(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            true,
            WritePurpose::Ordinary,
            &mut rng(),
            &mut budget,
            |unit, _| unit.ingest(&next, &f.group, &f.device).map_err(invalid),
            |_, _| panic!("injected writer panic"),
            sync_registry,
        )
    }));
    assert!(caught.is_err());
    assert!(budget.requires_reconciliation());
    assert_eq!(f.load(&store).unwrap().op_count(), 1);
}

#[test]
fn registry_store_content_ceiling_still_allows_seal_fault_and_first_owner_journal() {
    use super::super::epoch_budget::CONTENT_ALLOWANCE_BYTES;
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let op = f.op(1);
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &f);
    f.ingest(&mut store, &op, &mut budget).unwrap();
    let inv = inventory(&mut store);
    let records = inv.records_for_server(SERVER, &f.group.group_id()).unwrap();
    let initial = records[0];
    assert_eq!(initial.footprint.protocol, 0);
    // Only the registry file is real; a synthetic other-content record fills the accounting
    // fixture to the 1984-MiB ceiling without writing gigabytes of irrelevant test data.
    let filler = StorageRecord {
        id: [0x99; 32],
        document: [0x98; 32],
        footprint: Footprint {
            content: CONTENT_ALLOWANCE_BYTES - initial.footprint.content,
            ..Footprint::default()
        },
    };
    let scope = StorageScope::new(SERVER, &f.group.group_id()).unwrap();
    budget = EpochStorageBudget::from_inventory(scope.clone(), [initial, filler]).unwrap();
    let next = f.op(2);
    assert!(f.ingest(&mut store, &next, &mut budget).is_err());
    let receipt = f.receipt(7);
    store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            receipt.clone(),
            0,
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    let closed = inventory(&mut store)
        .records_for_server(SERVER, &f.group.group_id())
        .unwrap()[0];
    assert_eq!(closed.footprint.content, initial.footprint.content);
    assert!(closed.footprint.protocol > 0);
    store
        .prepare_epoch_owner_receipt(SERVER, receipt, &f.group, 0, &mut rng(), &mut budget)
        .unwrap();
    let (_, state) = store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            f.receipt(8),
            0,
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    assert_eq!(state.phase(), EpochPhase::Fault);
    drop(store);
    let mut store = open(root.path());
    let inv = inventory(&mut store);
    let registry = inv
        .records()
        .find(|e| e.kind == EpochRecordKind::Registry)
        .unwrap();
    assert_eq!(registry.record.footprint.content, initial.footprint.content);
    assert!(registry.record.footprint.protocol > closed.footprint.protocol);
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
    let restored = EpochStorageBudget::from_inventory(
        scope,
        inv.records_for_server(SERVER, &f.group.group_id())
            .unwrap()
            .into_iter()
            .chain([filler]),
    )
    .unwrap();
    assert!(!restored.requires_reconciliation());
}

#[test]
fn registry_store_input_bounds_scope_and_missing_source_refuse_without_resetting() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut op = f.op(1);
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &f);
    assert!(store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            f.receipt(7),
            0,
            &mut rng(),
            &mut budget
        )
        .is_err());
    assert!(!budget.requires_reconciliation());
    let mut outsider_receipt = f.receipt(7);
    outsider_receipt.signature[0] ^= 1;
    assert!(store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            outsider_receipt,
            0,
            &mut rng(),
            &mut budget,
        )
        .is_err());
    assert!(!budget.requires_reconciliation());
    assert_eq!(inventory(&mut store).records().len(), 0);
    let good = op.clone();
    op.blob.ciphertext = vec![0; MAX_INBOUND_CIPHERTEXT + 1];
    assert!(f.ingest(&mut store, &op, &mut budget).is_err());
    assert!(f.load(&store).is_none());
    let mut wrong = good.clone();
    wrong.doc_id ^= 1;
    assert!(f.ingest(&mut store, &wrong, &mut budget).is_err());
    f.ingest(&mut store, &good, &mut budget).unwrap();
    let original = fs::read(f.path(&store)).unwrap();
    let mut bad_receipt = f.receipt(7);
    bad_receipt.document.logical_key = vec![0; 193];
    assert!(store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            bad_receipt,
            0,
            &mut rng(),
            &mut budget
        )
        .is_err());
    assert_eq!(fs::read(f.path(&store)).unwrap(), original);
    fs::write(f.path(&store), b"corrupt").unwrap();
    assert!(store
        .load_registry_epoch(SERVER, &f.group, f.key.bucket(), &f.device)
        .is_err());
    assert!(f.ingest(&mut store, &good, &mut budget).is_err());
    assert!(budget.requires_reconciliation());
    assert_eq!(fs::read(f.path(&store)).unwrap(), b"corrupt");
    let file = File::create(f.path(&store)).unwrap();
    file.set_len(MAX_SEALED_BYTES as u64 + 1).unwrap();
    assert!(store
        .load_registry_epoch(SERVER, &f.group, f.key.bucket(), &f.device)
        .is_err());
}

#[test]
fn registry_store_scope_splices_and_inconsistent_snapshot_fail_inventory() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let op = f.op(1);
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &f);
    f.ingest(&mut store, &op, &mut budget).unwrap();
    let bytes = fs::read(f.path(&store)).unwrap();
    let foreign = store.registry_epoch_path(&scope_bytes(SERVER + 1, &f.document).unwrap());
    fs::write(&foreign, &bytes).unwrap();
    assert!(store
        .load_registry_epoch(SERVER + 1, &f.group, f.key.bucket(), &f.device)
        .is_err());
    let mut scan = store.scan_epoch_storage_with_registry().unwrap();
    assert!(loop {
        match scan.step() {
            Err(_) => break true,
            Ok(p) if p.complete => break false,
            _ => {}
        }
    });
    assert!(scan.finish().is_err());
    fs::remove_file(foreign).unwrap();
    let scope = scope_bytes(SERVER, &f.document).unwrap();
    let plain = store
        .read_registry_record(&scope_bytes(SERVER, &f.document).unwrap())
        .unwrap()
        .unwrap()
        .plain;
    let (_, snapshot) = decode_record(&plain, &scope, &f.document).unwrap();
    for corrupt_inner in [false, true] {
        let mut malformed = if corrupt_inner {
            let mut snapshot = snapshot.to_vec();
            snapshot.push(0);
            let mut e = Encoder::new();
            e.put_bytes(&scope).unwrap();
            e.put_u8(f.key.bucket());
            e.put_bytes(&snapshot).unwrap();
            let wrapped = e.finish();
            // Valid wrapper, invalid raw snapshot: pin the full shared inventory validator.
            assert!(decode_record(&wrapped, &scope, &f.document).is_ok());
            wrapped
        } else {
            plain.to_vec()
        };
        if !corrupt_inner {
            malformed.push(0);
        }
        let sealed = seal(&store.keys.db_key().unwrap(), &malformed, &mut rng()).unwrap();
        fs::write(f.path(&store), frame(&sealed)).unwrap();
        let mut scan = store.scan_epoch_storage_with_registry().unwrap();
        assert!(scan.step().is_err());
        assert!(scan.finish().is_err());
    }
}

#[test]
fn registry_store_missing_indexed_source_still_invalidates_accounting() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let op = f.op(1);
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &f);
    f.ingest(&mut store, &op, &mut budget).unwrap();
    fs::remove_file(f.path(&store)).unwrap();
    assert!(store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            f.receipt(7),
            0,
            &mut rng(),
            &mut budget,
        )
        .is_err());
    assert!(budget.requires_reconciliation());
    assert!(f.load(&store).is_none());
}

#[test]
fn registry_store_inventory_and_cleanup_have_explicit_coverage_and_preserve_intent_freshness() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let op = f.op(1);
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &f);
    f.ingest(&mut store, &op, &mut budget).unwrap();
    let mut old_scan = store.scan_epoch_storage_with_intents().unwrap();
    while !old_scan.step().unwrap().complete {}
    assert_eq!(
        old_scan.finish().unwrap().records().len(),
        0,
        "old coverage remains exact"
    );
    let inv = inventory(&mut store);
    assert_eq!(
        inv.coverage(),
        EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsAndRegistry
    );
    let mut intents = EpochIntentBudget::from_inventory(&inv).unwrap();
    // Even an empty four-family cleanup invalidates the old intent-inventory token.
    let mut cleanup = store.cleanup_epoch_storage_staging_with_registry().unwrap();
    while !cleanup.step().unwrap().complete {}
    let mut scan = cleanup.into_inventory().unwrap();
    while !scan.step().unwrap().complete {}
    let fresh = scan.finish().unwrap();
    assert_eq!(fresh.coverage(), inv.coverage());
    let domain = RegistryOp::Put {
        key: f.key.clone(),
        epoch: 3,
    }
    .domain_op(&f.group.group_id(), [3; 16])
    .unwrap();
    assert!(store
        .prepare_epoch_intent(
            SERVER,
            &f.document,
            domain.clone(),
            &f.device,
            &f.group,
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .is_err());
    intents.reconcile(&fresh).unwrap();
    store
        .prepare_epoch_intent(
            SERVER,
            &f.document,
            domain,
            &f.device,
            &f.group,
            &mut rng(),
            &mut budget,
            &mut intents,
        )
        .unwrap();
    let inv = inventory(&mut store);
    assert_eq!(
        inv.records()
            .filter(|e| e.kind == EpochRecordKind::Registry)
            .count(),
        1
    );
    assert_eq!(
        inv.records()
            .filter(|e| e.kind == EpochRecordKind::Intents)
            .count(),
        1
    );
}

#[test]
fn registry_store_cleanup_removes_only_unpublished_attempts_and_rescans_ownership() {
    for published in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut f = Fixture::new();
        let op = f.op(1);
        let mut store = open(root.path());
        let mut budget = budget(&mut store, &f);
        if published {
            f.ingest(&mut store, &op, &mut budget).unwrap();
        }
        let path = f.path(&store);
        let orphan = staging_candidate(&path, 901);
        fs::write(&orphan, b"partial opaque bytes").unwrap();
        let saved = fs::read(&path).ok();
        let inv = inventory(&mut store);
        assert_eq!(inv.orphans().len(), 1);
        assert_eq!(
            inv.orphans().next().unwrap().kind(),
            EpochRecordKind::Registry
        );
        if published {
            let records = inv.records_for_server(SERVER, &f.group.group_id()).unwrap();
            let scratch = records
                .iter()
                .find(|r| {
                    r.id != *blake3::hash(&scope_bytes(SERVER, &f.document).unwrap()).as_bytes()
                })
                .unwrap();
            assert_eq!(
                scratch.footprint.content,
                b"partial opaque bytes".len() as u64
            );
            assert_eq!(scratch.footprint.protocol + scratch.footprint.settlement, 0);
        } else {
            assert_eq!(inv.unresolved_orphans(), 1);
            assert!(inv.records_for_server(SERVER, &f.group.group_id()).is_err());
        }
        // Older cleanup must leave this namespace untouched.
        let mut old = store.cleanup_epoch_storage_staging_with_intents().unwrap();
        while !old.step().unwrap().complete {}
        drop(old);
        assert!(orphan.exists());
        let mut cleanup = store.cleanup_epoch_storage_staging_with_registry().unwrap();
        let final_progress = loop {
            let p = cleanup.step().unwrap();
            if p.complete {
                break p;
            }
        };
        assert_eq!(final_progress.removed_files, 1);
        let mut scan = cleanup.into_inventory().unwrap();
        while !scan.step().unwrap().complete {}
        let inv = scan.finish().unwrap();
        assert_eq!(inv.orphans().len(), 0);
        assert!(!orphan.exists());
        assert_eq!(fs::read(&path).ok(), saved);
        let mut budget = self::budget(&mut store, &f);
        assert_eq!(
            f.ingest(&mut store, &op, &mut budget).unwrap().0,
            if published {
                Admission::Duplicate
            } else {
                Admission::Accepted
            }
        );
    }
}

#[test]
fn registry_store_receipt_retry_after_failed_flush_and_removed_owner_inventory() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let op = f.op(1);
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &f);
    f.ingest(&mut store, &op, &mut budget).unwrap();
    let receipt = f.receipt(7);
    let result = store.update_registry_with_io(
        SERVER,
        &f.group,
        f.key.bucket(),
        &f.device,
        false,
        WritePurpose::Settlement,
        &mut rng(),
        &mut budget,
        |unit, _| unit.seal(receipt.clone(), &f.group, 0).map_err(invalid),
        |path, bytes| {
            atomic_write(path, bytes)?;
            Err(AppError::Io("post-rename failure".into()))
        },
        sync_registry,
    );
    assert!(result.is_err());
    assert!(budget.requires_reconciliation());
    drop(store);
    let mut store = open(root.path());
    let mut budget = self::budget(&mut store, &f);
    let held = fs::read(f.path(&store)).unwrap();
    assert_eq!(
        store
            .seal_registry_epoch(
                SERVER,
                &f.group,
                f.key.bucket(),
                &f.device,
                receipt,
                0,
                &mut rng(),
                &mut budget
            )
            .unwrap()
            .0,
        ReceiptIngest::Duplicate
    );
    assert_eq!(fs::read(f.path(&store)).unwrap(), held);
    let next = MlsDevice::generate().unwrap();
    let welcome = f
        .group
        .add_member(&f.device, next.key_package().unwrap())
        .unwrap()
        .welcome;
    let mut group = ServerGroup::join(&next, &welcome).unwrap();
    group.remove_member(&next, &f.device.device_id()).unwrap();
    let inv = inventory(&mut store); // No ServerGroup or current owner is passed to inventory.
    assert_eq!(
        inv.records()
            .filter(|e| e.kind == EpochRecordKind::Registry)
            .count(),
        1
    );
    assert_eq!(
        store
            .load_registry_epoch(SERVER, &group, f.key.bucket(), &next)
            .unwrap()
            .unwrap()
            .phase(),
        EpochPhase::Closing
    );
}
