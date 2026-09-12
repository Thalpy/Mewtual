use super::super::adoption::{AdoptionSync, AdoptionWrite};
use super::*;
use catcoms_replication::{registry::RegistryRecovery, CheckpointSeed, RecoveryReason};
use catcoms_rt::ManualClock;

fn target(f: &Fixture, epoch: u64, salt: u8) -> (Receipt, CheckpointSeed) {
    let mut projection = f.source.projection().unwrap();
    projection.epoch = epoch;
    projection.pointers.insert(f.key.clone(), u64::from(salt));
    let seed = projection.checkpoint([salt; 32]).unwrap();
    let receipt = Receipt::sign(
        f.document.clone(),
        epoch,
        [salt; 32],
        seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        &f.device,
    )
    .unwrap();
    (receipt, seed)
}
fn adopt(
    store: &mut ServerStore,
    f: &Fixture,
    receipt: &Receipt,
    seed: Option<&[u8]>,
    budget: &mut EpochStorageBudget,
) -> Result<(RegistryAdoptionOutcome, EpochRegistryState), AppError> {
    store.adopt_registry_checkpoint(
        SERVER,
        &f.group,
        f.key.bucket(),
        &f.device,
        receipt,
        seed,
        0,
        &ManualClock::new(100),
        &mut rng(),
        budget,
    )
}

#[test]
fn registry_adoption_store_saves_recovery_and_never_retires_intents_or_reseeds_retry() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut f = Fixture::new();
    let mut budget = budget(&mut store, &f);
    let mut intents = EpochIntentBudget::from_inventory(&inventory(&mut store)).unwrap();
    let domain = RegistryOp::Put {
        key: f.key.clone(),
        epoch: 1,
    }
    .domain_op(&f.group.group_id(), [1; 16])
    .unwrap();
    store
        .edit_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            domain,
            &mut rng(),
            &mut budget,
            &mut intents,
        )
        .unwrap();
    let intent_scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.document).unwrap();
    let intent_path = store
        .dir
        .join("servers")
        .join(format!("{}.intents", blake3::hash(&intent_scope).to_hex()));
    let ledger = fs::read(&intent_path).unwrap();
    let (receipt, seed) = target(&f, 10, 10);
    let (outcome, state) = adopt(&mut store, &f, &receipt, None, &mut budget).unwrap();
    assert_eq!(outcome, RegistryAdoptionOutcome::AwaitingSeed);
    assert_eq!((state.phase(), state.op_count()), (EpochPhase::Closing, 1));
    assert!(adopt(&mut store, &f, &receipt, Some(&[]), &mut budget).is_err());
    assert_eq!(f.load(&store).unwrap().op_count(), 1);
    let (outcome, state) =
        adopt(&mut store, &f, &receipt, Some(seed.bytes()), &mut budget).unwrap();
    assert_eq!(outcome, RegistryAdoptionOutcome::Installed);
    assert_eq!(
        (state.epoch(), state.phase(), state.op_count()),
        (11, EpochPhase::Open, 0)
    );
    assert_eq!(fs::read(&intent_path).unwrap(), ledger);
    let recovery = store.load_epoch_recovery(SERVER, &f.document).unwrap();
    let snapshot = recovery.retained().next().unwrap();
    assert_eq!(snapshot.reason, RecoveryReason::Rewound);
    let typed = RegistryRecovery::from_snapshot(snapshot, &f.document, f.key.bucket()).unwrap();
    assert_eq!(typed.projection().pointers[&f.key], 1);
    assert_eq!(typed.excluded_operations().len(), 1);
    // Accepted post-install history must survive a retry even without re-fetching the seed.
    f.source = RegistryEpoch::from_checkpoint(
        &f.group,
        f.key.bucket(),
        f.device.device_id(),
        receipt.clone(),
        0,
        seed.bytes(),
    )
    .unwrap();
    let op = f.op(20);
    f.ingest(&mut store, &op, &mut budget).unwrap();
    let bytes = fs::read(f.path(&store)).unwrap();
    let (outcome, state) = adopt(&mut store, &f, &receipt, None, &mut budget).unwrap();
    assert_eq!(outcome, RegistryAdoptionOutcome::AlreadyInstalled);
    assert_eq!(state.op_count(), 1);
    assert_eq!(fs::read(f.path(&store)).unwrap(), bytes);
    drop(store);
    let mut store = open(root.path());
    let mut budget = self::budget(&mut store, &f);
    assert_eq!(
        adopt(&mut store, &f, &receipt, None, &mut budget)
            .unwrap()
            .0,
        RegistryAdoptionOutcome::AlreadyInstalled
    );
    assert_eq!(
        f.load(&store).unwrap().projection().unwrap().pointers[&f.key],
        20
    );
}

#[test]
fn registry_adoption_store_fault_precedes_missing_seed_and_corrupt_recovery() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut f = Fixture::new();
    let mut budget = budget(&mut store, &f);
    let op = f.op(1);
    f.ingest(&mut store, &op, &mut budget).unwrap();
    let (receipt, _) = target(&f, 10, 10);
    adopt(&mut store, &f, &receipt, None, &mut budget).unwrap();
    // This opaque generic snapshot is authenticated and accounted, but not a valid registry
    // payload. It must stop installation without stopping the earlier independent fault save.
    let bad = catcoms_replication::RecoverySnapshot {
        doc_type: DocType::DocRegistry,
        logical_key: f.document.logical_key.clone(),
        epoch: 0,
        base_close_record_hash: None,
        reason: RecoveryReason::Excluded,
        projection: vec![99],
        tombstones: vec![],
        elements: vec![],
        conflicts: vec![],
        applied_ops: vec![],
    };
    store
        .update_epoch_recovery_accounted(
            SERVER,
            &f.document,
            EpochRecoveryAction::Stage(bad),
            &ManualClock::new(100),
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    let (conflict, _) = target(&f, 10, 11);
    let (outcome, state) = adopt(&mut store, &f, &conflict, None, &mut budget).unwrap();
    assert_eq!(outcome, RegistryAdoptionOutcome::Fault);
    assert_eq!((state.phase(), state.op_count()), (EpochPhase::Fault, 1));
    drop(store);
    let mut store = open(root.path());
    let mut budget = self::budget(&mut store, &f);
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
    assert_eq!(
        adopt(&mut store, &f, &conflict, Some(&[]), &mut budget)
            .unwrap()
            .0,
        RegistryAdoptionOutcome::Fault
    );
}

#[test]
fn registry_adoption_store_rejects_unmatched_inventory_and_bad_seed_without_losing_source() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut f = Fixture::new();
    let mut budget = budget(&mut store, &f);
    let op = f.op(1);
    f.ingest(&mut store, &op, &mut budget).unwrap();
    let (receipt, seed) = target(&f, 10, 10);
    let before = fs::read(f.path(&store)).unwrap();
    let mut wrong = receipt.clone();
    wrong.signature[0] ^= 1;
    assert!(adopt(&mut store, &f, &wrong, Some(seed.bytes()), &mut budget).is_err());
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
    // A disappeared indexed source must not be reset to epoch zero, even with a valid receipt.
    let source_path = f.path(&store);
    let saved_path = source_path.with_extension("test-held");
    fs::rename(&source_path, &saved_path).unwrap();
    assert!(adopt(&mut store, &f, &receipt, Some(seed.bytes()), &mut budget).is_err());
    assert!(!source_path.exists());
    fs::rename(&saved_path, &source_path).unwrap();
    assert!(
        adopt(&mut store, &f, &receipt, Some(seed.bytes()), &mut budget).is_err(),
        "uncertain inventory requires reconciliation"
    );
    budget = self::budget(&mut store, &f);
    assert_eq!(
        adopt(&mut store, &f, &receipt, Some(seed.bytes()), &mut budget)
            .unwrap()
            .0,
        RegistryAdoptionOutcome::Installed
    );
}

#[test]
fn registry_adoption_store_warning_retarget_restart_and_ack_or_timeout_keep_one_snapshot() {
    use catcoms_replication::RecoveryTransition;
    for acknowledge in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let mut f = Fixture::new();
        let mut budget = budget(&mut store, &f);
        let op = f.op(1);
        f.ingest(&mut store, &op, &mut budget).unwrap();
        for epoch in [10, 20, 30] {
            let (receipt, seed) = target(&f, epoch, epoch as u8);
            let (outcome, _) =
                adopt(&mut store, &f, &receipt, Some(seed.bytes()), &mut budget).unwrap();
            assert_eq!(
                outcome,
                if epoch == 30 {
                    RegistryAdoptionOutcome::RecoveryPending
                } else {
                    RegistryAdoptionOutcome::Installed
                }
            );
            f.source = f.load(&store).unwrap().unit;
        }
        let recovery = store.load_epoch_recovery(SERVER, &f.document).unwrap();
        assert_eq!(recovery.retained().count(), 2);
        assert!(recovery.staged().is_some());
        let warning = recovery.eviction_pending().unwrap().unwrap();
        let (receipt, seed) = target(&f, 40, 40);
        assert_eq!(
            adopt(&mut store, &f, &receipt, Some(seed.bytes()), &mut budget)
                .unwrap()
                .0,
            RegistryAdoptionOutcome::RecoveryPending
        );
        assert_eq!(
            store
                .load_epoch_recovery(SERVER, &f.document)
                .unwrap()
                .eviction_pending()
                .unwrap(),
            Some(warning)
        );
        drop(store);
        let mut store = open(root.path());
        let mut budget = self::budget(&mut store, &f);
        assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Closing);
        let RecoveryTransition::EvictionPending {
            oldest_snapshot,
            staged_snapshot,
            deadline_ms,
        } = warning
        else {
            panic!("warning");
        };
        // Premature time advancement cannot discard anything.
        store
            .update_epoch_recovery_accounted(
                SERVER,
                &f.document,
                EpochRecoveryAction::AdvanceTime,
                &ManualClock::new(deadline_ms - 1),
                &mut rng(),
                &mut budget,
            )
            .unwrap();
        assert_eq!(
            adopt(&mut store, &f, &receipt, Some(seed.bytes()), &mut budget)
                .unwrap()
                .0,
            RegistryAdoptionOutcome::RecoveryPending
        );
        let action = if acknowledge {
            EpochRecoveryAction::Acknowledge {
                oldest_snapshot,
                staged_snapshot,
            }
        } else {
            EpochRecoveryAction::AdvanceTime
        };
        store
            .update_epoch_recovery_accounted(
                SERVER,
                &f.document,
                action,
                &ManualClock::new(deadline_ms),
                &mut rng(),
                &mut budget,
            )
            .unwrap();
        assert_eq!(
            adopt(&mut store, &f, &receipt, Some(seed.bytes()), &mut budget)
                .unwrap()
                .0,
            RegistryAdoptionOutcome::Installed
        );
        let recovery = store.load_epoch_recovery(SERVER, &f.document).unwrap();
        assert!(recovery.eviction_pending().unwrap().is_none());
        assert_eq!(recovery.retained().count(), 2);
        assert!(recovery
            .retained()
            .any(|snapshot| snapshot.id().unwrap() == staged_snapshot));
        assert_eq!(f.load(&store).unwrap().epoch(), 41);
    }
}

#[test]
fn registry_adoption_store_empty_newcomer_installs_without_a_recovery_file() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new();
    let mut budget = budget(&mut store, &f);
    let (receipt, seed) = target(&f, 0, 1);
    let (outcome, state) =
        adopt(&mut store, &f, &receipt, Some(seed.bytes()), &mut budget).unwrap();
    assert_eq!(outcome, RegistryAdoptionOutcome::Installed);
    assert_eq!(state.epoch(), 1);
    assert!(store
        .epoch_recovery_inventory_record(SERVER, &f.document)
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .read_registry_seed(
                SERVER,
                &f.group,
                f.key.bucket(),
                &f.device,
                state.doc_id(),
                seed.change_hash(),
                &mut budget
            )
            .unwrap()
            .as_deref(),
        Some(seed.bytes())
    );
}

#[test]
fn registry_adoption_store_crash_boundaries_keep_source_or_successor_and_retry() {
    #[derive(Clone, Copy, Debug)]
    enum Failure {
        Write(AdoptionWrite, bool),
        Flush(AdoptionSync),
    }
    let failures = [
        Failure::Write(AdoptionWrite::Source, false),
        Failure::Write(AdoptionWrite::Source, true),
        Failure::Write(AdoptionWrite::Recovery, false),
        Failure::Write(AdoptionWrite::Recovery, true),
        Failure::Write(AdoptionWrite::Successor, false),
        Failure::Write(AdoptionWrite::Successor, true),
        Failure::Flush(AdoptionSync::Source),
    ];
    for failure in failures {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let mut f = Fixture::new();
        let mut budget = budget(&mut store, &f);
        let op = f.op(1);
        f.ingest(&mut store, &op, &mut budget).unwrap();
        let (receipt, seed) = target(&f, 10, 10);
        if matches!(failure, Failure::Flush(AdoptionSync::Source)) {
            adopt(&mut store, &f, &receipt, None, &mut budget).unwrap();
        }
        let fired = std::cell::Cell::new(false);
        let result = store.adopt_registry_checkpoint_with_io(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            &receipt,
            Some(seed.bytes()),
            0,
            &ManualClock::new(100),
            &mut rng(),
            &mut budget,
            &mut |step, path, bytes| {
                if let Failure::Write(wanted, after) = failure {
                    if step == wanted {
                        if after {
                            atomic_write(path, bytes)?;
                        }
                        fired.set(true);
                        return Err(invalid("injected adoption write failure"));
                    }
                }
                atomic_write(path, bytes)
            },
            &mut |step, path, bytes| {
                if matches!(failure, Failure::Flush(wanted) if wanted == step) {
                    fired.set(true);
                    return Err(invalid("injected adoption flush failure"));
                }
                sync_registry(path, bytes)
            },
        );
        assert!(fired.get(), "{failure:?}");
        assert!(result.is_err(), "{failure:?}");
        let state = f.load(&store).unwrap();
        if state.epoch() == 0 {
            assert_eq!(state.op_count(), 1, "{failure:?}");
        } else {
            assert_eq!(state.epoch(), 11);
            assert_eq!(
                store
                    .load_epoch_recovery(SERVER, &f.document)
                    .unwrap()
                    .retained()
                    .count(),
                1
            );
        }
        drop(store);
        let mut store = open(root.path());
        let mut budget = self::budget(&mut store, &f);
        // A rename may have succeeded even though its flush/report failed. After restart and
        // inventory reconciliation, accepted successor edits must survive retrying that receipt.
        let post_rename_edit = matches!(failure, Failure::Write(AdoptionWrite::Successor, true));
        if post_rename_edit {
            f.source = f.load(&store).unwrap().unit;
            let newer = f.op(20);
            f.ingest(&mut store, &newer, &mut budget).unwrap();
        }
        let (outcome, state) =
            adopt(&mut store, &f, &receipt, Some(seed.bytes()), &mut budget).unwrap();
        assert!(matches!(
            outcome,
            RegistryAdoptionOutcome::Installed | RegistryAdoptionOutcome::AlreadyInstalled
        ));
        assert_eq!(state.epoch(), 11);
        if post_rename_edit {
            assert_eq!(outcome, RegistryAdoptionOutcome::AlreadyInstalled);
            assert_eq!(state.op_count(), 1);
            assert_eq!(state.projection().unwrap().pointers[&f.key], 20);
        }
        assert_eq!(
            store
                .load_epoch_recovery(SERVER, &f.document)
                .unwrap()
                .retained()
                .count(),
            1
        );
    }
}
