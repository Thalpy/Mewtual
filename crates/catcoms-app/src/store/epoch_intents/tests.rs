use super::*;
use catcoms_replication::epoch::{MAX_INTENTS_PER_DOCUMENT, MAX_INTENT_BYTES_PER_DOCUMENT};
use catcoms_wire::DocType;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SERVER: u64 = 7;

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(17)
}
fn open(path: &Path) -> ServerStore {
    ServerStore::open(path, b"intent-tests", &mut rng()).unwrap()
}
fn fixture() -> (MlsDevice, ServerGroup, LogicalDocument) {
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let document = LogicalDocument::new(
        group.group_id(),
        DocType::StudioObject,
        b"private-intent-cat".to_vec(),
    )
    .unwrap();
    (device, group, document)
}
fn op(document: &LogicalDocument, n: u128) -> DomainOp {
    DomainOp {
        nonce: n.to_be_bytes(),
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        body: b"private replay instructions".to_vec(),
    }
}
fn inventory(store: &mut ServerStore) -> EpochStorageInventory {
    let mut scan = store.scan_epoch_storage_with_intents().unwrap();
    let mut prior = EpochStorageScanProgress::default();
    loop {
        let next = scan.step().unwrap();
        assert!(next.visited_entries - prior.visited_entries <= 64);
        let bodies = |p: EpochStorageScanProgress| {
            p.recovery_records + p.owner_receipt_records + p.intent_records
        };
        assert!(bodies(next) - bodies(prior) <= 1);
        if next.complete {
            break;
        }
        prior = next;
    }
    scan.finish().unwrap()
}
// This fixture's ONLY managed files are the three inventoried families. Production must compose
// all other P1 record types under the sole coordinator before constructing the server budget.
fn budgets(
    store: &mut ServerStore,
    doc: &LogicalDocument,
) -> (EpochStorageBudget, EpochIntentBudget) {
    let view = inventory(store);
    (
        EpochStorageBudget::from_inventory(
            StorageScope::new(SERVER, &doc.server_id).unwrap(),
            view.records_for_server(SERVER, &doc.server_id).unwrap(),
        )
        .unwrap(),
        EpochIntentBudget::from_inventory(&view).unwrap(),
    )
}
fn prepare(
    store: &mut ServerStore,
    doc: &LogicalDocument,
    op: DomainOp,
    device: &MlsDevice,
    group: &ServerGroup,
    budgets: &mut (EpochStorageBudget, EpochIntentBudget),
) -> Result<EpochIntentState, AppError> {
    store.prepare_epoch_intent(
        SERVER,
        doc,
        op,
        device,
        group,
        &mut rng(),
        &mut budgets.0,
        &mut budgets.1,
    )
}
fn staging_path(store: &ServerStore, doc: &LogicalDocument, sequence: u64) -> PathBuf {
    let path = store.epoch_intent_path(&scope_bytes(SERVER, doc).unwrap());
    path.with_file_name(format!(
        ".{}.mewtual-stage-7-{sequence}.tmp",
        path.file_name().unwrap().to_str().unwrap()
    ))
}
// Simulate a process death that leaves its securely created staging sibling behind. Returning a
// normal error would let the production RAII guard unlink it, unlike a killed process.
fn leave_staging(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let (mut file, mut staging) = create_staging_file(path)?;
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
    staging.remove_on_drop = false;
    Err(AppError::Io("interrupted before rename".into()))
}
fn cleanup(store: &mut ServerStore) -> EpochStorageInventory {
    let mut cleanup = store.cleanup_epoch_storage_staging_with_intents().unwrap();
    while !cleanup.step().unwrap().complete {}
    let mut scan = cleanup.into_inventory().unwrap();
    assert_eq!(
        scan.coverage(),
        EpochInventoryCoverage::RecoveryOwnerReceiptsAndIntents
    );
    while !scan.step().unwrap().complete {}
    scan.finish().unwrap()
}

#[test]
fn prepare_restart_and_exact_retry_preserve_ids_newer_intents_and_sealed_content() {
    let root = tempfile::tempdir().unwrap();
    let (device, group, doc) = fixture();
    let first = op(&doc, 1);
    let id = first.id(&device.device_id());
    let mut store = open(root.path());
    let mut limits = budgets(&mut store, &doc);
    assert_eq!(
        store
            .load_epoch_intents(SERVER, &doc)
            .unwrap()
            .pending()
            .len(),
        0
    );
    assert_eq!(
        prepare(
            &mut store,
            &doc,
            first.clone(),
            &device,
            &group,
            &mut limits
        )
        .unwrap()
        .pending()
        .next()
        .unwrap()
        .0,
        &id
    );
    prepare(&mut store, &doc, op(&doc, 2), &device, &group, &mut limits).unwrap();
    let path = store.epoch_intent_path(&scope_bytes(SERVER, &doc).unwrap());
    let sealed = fs::read(&path).unwrap();
    assert!(!sealed.windows(first.body.len()).any(|b| b == first.body));
    drop(store);
    let mut store = open(root.path());
    // A budget cannot cross mounted sessions, even at the same path with the same vault key.
    assert!(prepare(
        &mut store,
        &doc,
        first.clone(),
        &device,
        &group,
        &mut limits
    )
    .is_err());
    let mut limits = budgets(&mut store, &doc);
    let state = prepare(
        &mut store,
        &doc,
        first.clone(),
        &device,
        &group,
        &mut limits,
    )
    .unwrap();
    assert_eq!(state.pending().len(), 2);
    assert!(state.pending().any(|(saved_id, value)| *saved_id == id
        && value.operation == first
        && value.author == device.device_id()));
    assert_eq!(
        fs::read(path).unwrap(),
        sealed,
        "exact retry flushes, never rewrites"
    );
    for debug in [
        format!("{state:?}"),
        format!("{:?}", limits.1),
        format!("{:?}", inventory(&mut store)),
    ] {
        assert!(!debug.contains("private"));
        assert!(!debug.contains(&hex::encode(id)));
    }
}

#[test]
fn conflicting_nonce_scope_author_and_oversized_operation_reject_before_io() {
    let root = tempfile::tempdir().unwrap();
    let (device, group, doc) = fixture();
    let mut store = open(root.path());
    let mut limits = budgets(&mut store, &doc);
    let first = op(&doc, 1);
    prepare(
        &mut store,
        &doc,
        first.clone(),
        &device,
        &group,
        &mut limits,
    )
    .unwrap();
    let path = store.epoch_intent_path(&scope_bytes(SERVER, &doc).unwrap());
    let before = fs::read(&path).unwrap();
    let mut conflicting = first.clone();
    conflicting.body.push(b'!');
    let mut wrong_key = op(&doc, 2);
    wrong_key.logical_key.push(b'!');
    let mut wrong_type = op(&doc, 3);
    wrong_type.doc_type = DocType::StudioIndex;
    let mut huge = op(&doc, 4);
    huge.body = vec![0; MAX_DOMAIN_OP_BYTES + 1];
    for invalid in [conflicting, wrong_key, wrong_type, huge] {
        assert!(prepare(&mut store, &doc, invalid, &device, &group, &mut limits).is_err());
    }
    let outsider = MlsDevice::generate().unwrap();
    assert!(prepare(
        &mut store,
        &doc,
        op(&doc, 5),
        &outsider,
        &group,
        &mut limits
    )
    .is_err());
    let foreign = ServerGroup::create(&outsider).unwrap();
    assert!(prepare(
        &mut store,
        &doc,
        op(&doc, 6),
        &outsider,
        &foreign,
        &mut limits
    )
    .is_err());
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(
        store
            .load_epoch_intents(SERVER, &doc)
            .unwrap()
            .pending()
            .len(),
        1
    );
    assert!(!limits.0.requires_reconciliation());
    assert!(limits.1.ready);
}

#[test]
fn old_coverage_other_vault_and_stale_duplicate_budgets_cannot_authorize_intent_writes() {
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let (device, group, doc) = fixture();
    let mut store = open(root.path());
    let mut peer_store = open(other.path());
    for mut scan in [store.scan_epoch_recovery().unwrap()] {
        while !scan.step().unwrap().complete {}
        assert!(EpochIntentBudget::from_inventory(&scan.finish().unwrap()).is_err());
    }
    let mut scan = store.scan_epoch_storage().unwrap();
    while !scan.step().unwrap().complete {}
    assert!(EpochIntentBudget::from_inventory(&scan.finish().unwrap()).is_err());
    let mut foreign = budgets(&mut peer_store, &doc);
    assert!(prepare(&mut store, &doc, op(&doc, 1), &device, &group, &mut foreign).is_err());
    let before = inventory(&mut store);
    let mut a = budgets(&mut store, &doc);
    let mut b = budgets(&mut store, &doc);
    prepare(&mut store, &doc, op(&doc, 1), &device, &group, &mut a).unwrap();
    assert!(prepare(&mut store, &doc, op(&doc, 2), &device, &group, &mut b).is_err());
    // Even constructing a new budget from a completed OLD inventory cannot revive its token.
    let mut stale = EpochIntentBudget::from_inventory(&before).unwrap();
    assert!(store
        .prepare_epoch_intent(
            SERVER,
            &doc,
            op(&doc, 2),
            &device,
            &group,
            &mut rng(),
            &mut a.0,
            &mut stale
        )
        .is_err());
    // A cancelled/empty cleanup attempt invalidates old intent budgets as well.
    cleanup(&mut store);
    assert!(prepare(&mut store, &doc, op(&doc, 2), &device, &group, &mut a).is_err());
}

#[test]
fn interrupted_first_save_is_not_an_accepted_edit_and_cleanup_does_not_promote_it() {
    let root = tempfile::tempdir().unwrap();
    let (device, group, doc) = fixture();
    let mut store = open(root.path());
    let mut limits = budgets(&mut store, &doc);
    let result = store.prepare_epoch_intent_with_io(
        SERVER,
        &doc,
        op(&doc, 1),
        &device,
        &group,
        &mut rng(),
        &mut limits.0,
        &mut limits.1,
        leave_staging,
        sync_intent,
    );
    assert!(result.is_err());
    assert!(limits.0.requires_reconciliation());
    assert!(!limits.1.ready);
    drop(store);
    let mut store = open(root.path());
    let view = inventory(&mut store);
    assert_eq!(view.unresolved_orphans(), 1);
    assert!(view.records_for_server(SERVER, &doc.server_id).is_err());
    assert!(EpochIntentBudget::from_inventory(&view).unwrap().bytes() > 0);
    assert_eq!(
        store
            .load_epoch_intents(SERVER, &doc)
            .unwrap()
            .pending()
            .len(),
        0
    );
    let cleaned = cleanup(&mut store);
    assert_eq!(
        EpochIntentBudget::from_inventory(&cleaned).unwrap().bytes(),
        0
    );
    let mut limits = budgets(&mut store, &doc);
    prepare(&mut store, &doc, op(&doc, 1), &device, &group, &mut limits).unwrap();
}

#[test]
fn failed_replacement_counts_orphans_as_content_and_global_bytes_until_reconciliation() {
    let root = tempfile::tempdir().unwrap();
    let (device, group, doc) = fixture();
    let mut store = open(root.path());
    let mut limits = budgets(&mut store, &doc);
    prepare(&mut store, &doc, op(&doc, 1), &device, &group, &mut limits).unwrap();
    let before = limits.0.usage();
    assert!(store
        .prepare_epoch_intent_with_io(
            SERVER,
            &doc,
            op(&doc, 2),
            &device,
            &group,
            &mut rng(),
            &mut limits.0,
            &mut limits.1,
            leave_staging,
            sync_intent
        )
        .is_err());
    let view = inventory(&mut store);
    let orphan = view.orphans().next().unwrap();
    assert_eq!(orphan.kind(), EpochRecordKind::Intents);
    let observed = EpochStorageBudget::from_inventory(
        StorageScope::new(SERVER, &doc.server_id).unwrap(),
        view.records_for_server(SERVER, &doc.server_id).unwrap(),
    )
    .unwrap();
    assert_eq!(observed.usage().content, before.content + orphan.bytes());
    assert_eq!(observed.usage().settlement, 0);
    assert_eq!(
        EpochIntentBudget::from_inventory(&view).unwrap().bytes(),
        observed.usage().content
    );
    cleanup(&mut store);
    assert!(limits.0.requires_reconciliation());
    assert!(!limits.1.ready);
    let mut limits = budgets(&mut store, &doc);
    assert_eq!(
        prepare(&mut store, &doc, op(&doc, 2), &device, &group, &mut limits)
            .unwrap()
            .pending()
            .len(),
        2
    );
}

#[test]
fn committed_first_save_retries_at_both_caps_without_another_copy() {
    let root = tempfile::tempdir().unwrap();
    let (device, group, doc) = fixture();
    let mut filler = doc.clone();
    filler.logical_key = b"filler".to_vec();
    let mut store = open(root.path());
    let mut limits = budgets(&mut store, &doc);
    prepare(
        &mut store,
        &filler,
        op(&filler, 1),
        &device,
        &group,
        &mut limits,
    )
    .unwrap();
    let mut next = EpochIntentState {
        ledger: IntentLedger::new(doc.clone()),
    };
    next.ledger
        .prepare(device.device_id(), op(&doc, 1))
        .unwrap();
    let next_len = next
        .encode(&scope_bytes(SERVER, &doc).unwrap())
        .unwrap()
        .len() as u64
        + 40;
    // Real file lengths reach the vault cap; temporary bytes are never read, so a sparse sibling
    // exercises the 64MiB rail cheaply. It has a verified destination and remains across retry.
    let orphan = staging_path(&store, &filler, 99);
    File::create(&orphan)
        .unwrap()
        .set_len(MAX_VAULT_INTENT_BYTES - limits.1.bytes() - next_len)
        .unwrap();
    let view = inventory(&mut store);
    // Synthetic unrelated managed content exercises the server cap without a two-GiB fixture.
    // No omitted intent files: the vault budget above is reconstructed from actual file lengths.
    let mut records = view.records_for_server(SERVER, &doc.server_id).unwrap();
    let filler_record = StorageRecord {
        id: [241; 32],
        document: [241; 32],
        footprint: Footprint {
            content: super::super::epoch_budget::CONTENT_ALLOWANCE_BYTES - MAX_VAULT_INTENT_BYTES,
            ..Footprint::default()
        },
    };
    records.push(filler_record);
    limits.0 = EpochStorageBudget::from_inventory(
        StorageScope::new(SERVER, &doc.server_id).unwrap(),
        records,
    )
    .unwrap();
    limits.1 = EpochIntentBudget::from_inventory(&view).unwrap();
    assert!(store
        .prepare_epoch_intent_with_io(
            SERVER,
            &doc,
            op(&doc, 1),
            &device,
            &group,
            &mut rng(),
            &mut limits.0,
            &mut limits.1,
            |path, bytes| atomic_write_with_hook_and_sync(
                path,
                bytes,
                |_, _| {},
                |_| Err(std::io::Error::other("after rename"))
            ),
            sync_intent
        )
        .is_err());
    drop(store);
    let mut store = open(root.path());
    let view = inventory(&mut store);
    let mut records = view.records_for_server(SERVER, &doc.server_id).unwrap();
    records.push(filler_record);
    let mut budget = EpochStorageBudget::from_inventory(
        StorageScope::new(SERVER, &doc.server_id).unwrap(),
        records,
    )
    .unwrap();
    let mut intents = EpochIntentBudget::from_inventory(&view).unwrap();
    assert_eq!(intents.bytes(), MAX_VAULT_INTENT_BYTES);
    assert_eq!(
        budget.usage().content,
        super::super::epoch_budget::CONTENT_ALLOWANCE_BYTES
    );
    let state = store
        .prepare_epoch_intent_with_io(
            SERVER,
            &doc,
            op(&doc, 1),
            &device,
            &group,
            &mut rng(),
            &mut budget,
            &mut intents,
            |_, _| panic!("an exact retry must not rewrite ciphertext"),
            sync_intent,
        )
        .unwrap();
    assert_eq!(state.pending().len(), 1);
    assert_eq!(intents.bytes(), MAX_VAULT_INTENT_BYTES);
    assert!(!budget.requires_reconciliation());
    assert!(store
        .prepare_epoch_intent(
            SERVER,
            &doc,
            op(&doc, 2),
            &device,
            &group,
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .is_err());
}

#[test]
fn writer_and_retry_flush_panics_or_errors_poison_both_budgets() {
    let root = tempfile::tempdir().unwrap();
    let (device, group, doc) = fixture();
    let mut store = open(root.path());
    let mut limits = budgets(&mut store, &doc);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.prepare_epoch_intent_with_io(
            SERVER,
            &doc,
            op(&doc, 1),
            &device,
            &group,
            &mut rng(),
            &mut limits.0,
            &mut limits.1,
            |_, _| panic!("writer panic"),
            sync_intent,
        )
    }));
    assert!(panic.is_err());
    assert!(limits.0.requires_reconciliation());
    assert!(!limits.1.ready);
    let mut limits = budgets(&mut store, &doc);
    prepare(&mut store, &doc, op(&doc, 1), &device, &group, &mut limits).unwrap();
    assert!(store
        .prepare_epoch_intent_with_io(
            SERVER,
            &doc,
            op(&doc, 1),
            &device,
            &group,
            &mut rng(),
            &mut limits.0,
            &mut limits.1,
            |_, _| panic!("no rewrite"),
            |_, _| Err(AppError::Io("flush failed".into()))
        )
        .is_err());
    assert!(limits.0.requires_reconciliation());
    assert!(!limits.1.ready);
    let mut limits = budgets(&mut store, &doc);
    assert_eq!(
        prepare(&mut store, &doc, op(&doc, 1), &device, &group, &mut limits)
            .unwrap()
            .pending()
            .len(),
        1
    );
}

#[test]
fn intent_namespace_aliases_and_corruption_never_reset_or_escape_discovery() {
    for mode in 0..5 {
        let root = tempfile::tempdir().unwrap();
        let (device, group, doc) = fixture();
        let mut store = open(root.path());
        let mut limits = budgets(&mut store, &doc);
        prepare(&mut store, &doc, op(&doc, 1), &device, &group, &mut limits).unwrap();
        let path = store.epoch_intent_path(&scope_bytes(SERVER, &doc).unwrap());
        match mode {
            0 => {
                fs::write(&path, b"corrupt").unwrap();
            }
            1 => {
                OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_len(MAX_SEALED_BYTES as u64 + 1)
                    .unwrap();
            }
            2 => {
                fs::remove_file(&path).unwrap();
                fs::create_dir(&path).unwrap();
            }
            3 => {
                let alias = path.with_file_name(
                    path.file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_ascii_uppercase(),
                );
                fs::rename(&path, alias).unwrap();
            }
            _ => {
                let other = doc_with_key(&doc, b"other");
                fs::copy(
                    &path,
                    store.epoch_intent_path(&scope_bytes(SERVER, &other).unwrap()),
                )
                .unwrap();
            }
        }
        let mut scan = store.scan_epoch_storage_with_intents().unwrap();
        while let Ok(p) = scan.step() {
            assert!(!p.complete, "bad file was omitted");
        }
        assert!(scan.step().is_err());
        assert!(scan.finish().is_err());
        if mode < 3 {
            assert!(prepare(&mut store, &doc, op(&doc, 2), &device, &group, &mut limits).is_err());
            assert!(limits.0.requires_reconciliation());
            assert!(!limits.1.ready);
        }
    }
}

fn doc_with_key(doc: &LogicalDocument, key: &[u8]) -> LogicalDocument {
    LogicalDocument::new(doc.server_id.clone(), doc.doc_type, key.to_vec()).unwrap()
}

#[test]
fn empty_ledger_inner_scope_substitution_and_cross_local_mount_reject() {
    let root = tempfile::tempdir().unwrap();
    let (_, _, doc) = fixture();
    let mut store = open(root.path());
    let foreign = doc_with_key(&doc, b"other");
    let scope = scope_bytes(SERVER, &doc).unwrap();
    let bad = EpochIntentState {
        ledger: IntentLedger::new(foreign),
    }
    .encode(&scope)
    .unwrap();
    let sealed = seal(&store.keys.db_key().unwrap(), &bad, &mut rng()).unwrap();
    fs::write(store.epoch_intent_path(&scope), frame(&sealed)).unwrap();
    assert!(store.load_epoch_intents(SERVER, &doc).is_err());
    let mut scan = store.scan_epoch_storage_with_intents().unwrap();
    assert!(scan.step().is_err());
    drop(scan);
    let state = EpochIntentState {
        ledger: IntentLedger::new(doc.clone()),
    };
    let sealed = seal(
        &store.keys.db_key().unwrap(),
        &state.encode(&scope).unwrap(),
        &mut rng(),
    )
    .unwrap();
    fs::write(
        store.epoch_intent_path(&scope_bytes(SERVER + 1, &doc).unwrap()),
        frame(&sealed),
    )
    .unwrap();
    assert!(store.load_epoch_intents(SERVER + 1, &doc).is_err());
}

#[test]
fn old_scans_and_cleanup_ignore_intent_family_while_new_cleanup_preserves_finals() {
    let root = tempfile::tempdir().unwrap();
    let (device, group, doc) = fixture();
    let mut store = open(root.path());
    let mut limits = budgets(&mut store, &doc);
    prepare(&mut store, &doc, op(&doc, 1), &device, &group, &mut limits).unwrap();
    let final_path = store.epoch_intent_path(&scope_bytes(SERVER, &doc).unwrap());
    let before = fs::read(&final_path).unwrap();
    let orphan = staging_path(&store, &doc, 3);
    fs::write(&orphan, b"unpublished").unwrap();
    let alias = store
        .dir
        .join("servers")
        .join(format!("{}.INTENTS", "a".repeat(64)));
    fs::write(&alias, b"bad").unwrap();
    let mut scan = store.scan_epoch_storage().unwrap();
    while !scan.step().unwrap().complete {}
    assert_eq!(scan.finish().unwrap().records().len(), 0);
    let mut old_cleanup = store.cleanup_epoch_storage_staging().unwrap();
    while !old_cleanup.step().unwrap().complete {}
    drop(old_cleanup);
    assert!(orphan.exists());
    assert!(alias.exists());
    fs::remove_file(alias).unwrap();
    let view = cleanup(&mut store);
    assert_eq!(view.orphans().len(), 0);
    assert_eq!(view.records().len(), 1);
    assert_eq!(fs::read(final_path).unwrap(), before);
}

#[test]
fn per_document_count_and_byte_caps_survive_vault_decode_and_prepare() {
    for count_limited in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let (device, group, doc) = fixture();
        let mut ledger = IntentLedger::new(doc.clone());
        if count_limited {
            for n in 0..MAX_INTENTS_PER_DOCUMENT {
                ledger
                    .prepare(device.device_id(), op(&doc, n as u128))
                    .unwrap();
            }
        } else {
            // Fill the exact aggregate cap using valid canonical domain envelopes.
            let overhead = op(&doc, 0).encode().unwrap().len() - op(&doc, 0).body.len();
            let mut left = MAX_INTENT_BYTES_PER_DOCUMENT;
            let mut n = 0;
            while left > 0 {
                let len = left.min(MAX_DOMAIN_OP_BYTES);
                let mut next = op(&doc, n);
                next.body = vec![b'x'; len - overhead];
                ledger.prepare(device.device_id(), next).unwrap();
                left -= len;
                n += 1;
            }
        }
        let mut store = open(root.path());
        let scope = scope_bytes(SERVER, &doc).unwrap();
        let state = EpochIntentState { ledger };
        let sealed = seal(
            &store.keys.db_key().unwrap(),
            &state.encode(&scope).unwrap(),
            &mut rng(),
        )
        .unwrap();
        fs::write(store.epoch_intent_path(&scope), frame(&sealed)).unwrap();
        let mut limits = budgets(&mut store, &doc);
        assert!(prepare(
            &mut store,
            &doc,
            op(&doc, u128::MAX),
            &device,
            &group,
            &mut limits
        )
        .is_err());
        assert!(!limits.0.requires_reconciliation());
        assert!(limits.1.ready);
    }
}

#[test]
fn all_three_families_share_document_ownership_but_keep_distinct_pools_and_temp_namespaces() {
    use catcoms_replication::{InheritedCheckpoint, Receipt, RecoveryReason, RecoverySnapshot};
    use catcoms_rt::ManualClock;
    let root = tempfile::tempdir().unwrap();
    let (device, group, doc) = fixture();
    let mut store = open(root.path());
    store
        .update_epoch_recovery(
            SERVER,
            &doc,
            EpochRecoveryAction::Stage(RecoverySnapshot {
                doc_type: doc.doc_type,
                logical_key: doc.logical_key.clone(),
                epoch: 0,
                base_close_record_hash: None,
                reason: RecoveryReason::Excluded,
                projection: b"recovery".to_vec(),
                tombstones: vec![],
                elements: vec![],
                conflicts: vec![],
                applied_ops: vec![],
            }),
            &ManualClock::new(0),
            &mut rng(),
        )
        .unwrap();
    let mut limits = budgets(&mut store, &doc);
    let receipt = Receipt::sign(
        doc.clone(),
        0,
        [1; 32],
        [2; 32],
        group.epoch(),
        InheritedCheckpoint::EpochZero,
        &device,
    )
    .unwrap();
    store
        .prepare_epoch_owner_receipt(
            SERVER,
            receipt,
            &group,
            group.epoch(),
            &mut rng(),
            &mut limits.0,
        )
        .unwrap();
    prepare(&mut store, &doc, op(&doc, 1), &device, &group, &mut limits).unwrap();
    let view = inventory(&mut store);
    let entries: Vec<_> = view.records().collect();
    assert_eq!(entries.len(), 3);
    assert!(entries
        .iter()
        .all(|e| e.record.document == entries[0].record.document));
    let owner = entries
        .iter()
        .find(|e| e.kind == EpochRecordKind::OwnerReceipts)
        .unwrap();
    let intent = entries
        .iter()
        .find(|e| e.kind == EpochRecordKind::Intents)
        .unwrap();
    assert!(owner.record.footprint.protocol > 0);
    assert_eq!(owner.record.footprint.content, 0);
    assert!(intent.record.footprint.content > 0);
    assert_eq!(intent.record.footprint.protocol, 0);
    assert_eq!(limits.1.bytes(), intent.record.footprint.content);
    // A digest equal to an owner file does not authenticate an intent temporary's destination.
    let fake = store.dir.join("servers").join(format!(
        ".{}.intents.mewtual-stage-7-555.tmp",
        hex::encode(owner.record.id)
    ));
    fs::write(&fake, b"not attributable").unwrap();
    let with_orphan = inventory(&mut store);
    assert_eq!(with_orphan.unresolved_orphans(), 1);
    assert!(with_orphan
        .records_for_server(SERVER, &doc.server_id)
        .is_err());
    assert_eq!(
        EpochIntentBudget::from_inventory(&with_orphan)
            .unwrap()
            .bytes(),
        limits.1.bytes() + 16
    );
    let saved: Vec<_> = entries.iter().map(|entry| entry.record).collect();
    let cleaned = cleanup(&mut store);
    assert_eq!(
        cleaned.records().map(|e| e.record).collect::<Vec<_>>(),
        saved
    );
    assert_eq!(
        store
            .load_epoch_intents(SERVER, &doc)
            .unwrap()
            .pending()
            .len(),
        1
    );
}

#[test]
fn vault_cap_counts_other_servers_and_unknown_orphans_and_reconcile_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let (device, group, doc) = fixture();
    let mut store = open(root.path());
    let mut limits = budgets(&mut store, &doc);
    prepare(&mut store, &doc, op(&doc, 1), &device, &group, &mut limits).unwrap();
    let (_, _, foreign_doc) = fixture();
    let final_path = store.epoch_intent_path(&scope_bytes(SERVER + 1, &foreign_doc).unwrap());
    let foreign = EpochIntentState {
        ledger: IntentLedger::new(foreign_doc.clone()),
    };
    let encoded = foreign
        .encode(&scope_bytes(SERVER + 1, &foreign_doc).unwrap())
        .unwrap();
    let sealed = frame(&seal(&store.keys.db_key().unwrap(), &encoded, &mut rng()).unwrap());
    fs::write(final_path, &sealed).unwrap();
    let view = inventory(&mut store);
    assert_eq!(
        EpochIntentBudget::from_inventory(&view).unwrap().bytes(),
        limits.1.bytes() + sealed.len() as u64
    );
    let orphan = staging_path(&store, &doc_with_key(&doc, b"no destination"), 99);
    File::create(&orphan)
        .unwrap()
        .set_len(MAX_VAULT_INTENT_BYTES)
        .unwrap();
    let over_cap = inventory(&mut store);
    assert_eq!(over_cap.unresolved_orphans(), 1);
    assert!(limits.1.reconcile(&over_cap).is_err());
    assert!(!limits.1.ready);
    let cleaned = cleanup(&mut store);
    limits.1.reconcile(&cleaned).unwrap();
    assert!(limits.1.ready);
}

#[test]
fn intent_metadata_rail_and_server_admission_refuse_without_poisoning_untouched_budgets() {
    let root = tempfile::tempdir().unwrap();
    let (device, group, doc) = fixture();
    let mut store = open(root.path());
    let mut limits = budgets(&mut store, &doc);
    // Inject the count boundary without constructing 65,536 sealed files. Constructor coverage
    // and actual directory rails are separately pinned by shared-inventory tests.
    limits.1.record_slots = super::super::epoch_budget::MAX_ACCOUNTED_RECORDS;
    assert!(prepare(&mut store, &doc, op(&doc, 1), &device, &group, &mut limits).is_err());
    assert!(limits.1.ready);
    assert!(!limits.0.requires_reconciliation());
    let mut limits = budgets(&mut store, &doc);
    let full = StorageRecord {
        id: [99; 32],
        document: [99; 32],
        footprint: Footprint {
            content: super::super::epoch_budget::CONTENT_ALLOWANCE_BYTES,
            ..Footprint::default()
        },
    };
    limits.0 = EpochStorageBudget::from_inventory(
        StorageScope::new(SERVER, &doc.server_id).unwrap(),
        [full],
    )
    .unwrap();
    assert!(prepare(&mut store, &doc, op(&doc, 1), &device, &group, &mut limits).is_err());
    assert!(limits.1.ready);
    assert!(!limits.0.requires_reconciliation());
    assert_eq!(inventory(&mut store).records().len(), 0);
}
