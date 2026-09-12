use super::settlement::{source_fixture, TestSource};
use super::*;
use catcoms_replication::registry::RegistryRecovery;
use catcoms_replication::{RecoverySnapshot, RecoveryTransition};
use catcoms_rt::ManualClock;

fn seal(source: &mut TestSource) {
    let f = &source.f;
    source
        .store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            source.receipt.clone(),
            0,
            &mut rng(),
            &mut source.budget,
        )
        .unwrap();
}

fn stage(source: &mut TestSource, now: u64) -> Result<Option<EpochRecoveryUpdate>, AppError> {
    let f = &source.f;
    source.store.stage_registry_recovery(
        SERVER,
        &f.group,
        f.key.bucket(),
        &f.device,
        &source.close,
        0,
        &ManualClock::new(now),
        &mut rng(),
        &mut source.budget,
    )
}

fn recovery(source: &TestSource) -> EpochRecoveryState {
    source
        .store
        .load_epoch_recovery(SERVER, &source.f.document)
        .unwrap()
}

#[test]
fn registry_recovery_stage_is_accounted_durable_and_retry_stable_after_quarantine() {
    let root = tempfile::tempdir().unwrap();
    let mut source = source_fixture(root.path(), true);
    assert!(stage(&mut source, 0).is_err(), "Open is not settlement");
    // Prepare a valid late change before the seal, without publishing it to the target store.
    let mut writer = source.f.load(&source.store).unwrap().unit;
    let domain = RegistryOp::Put {
        key: source.f.key.clone(),
        epoch: 12,
    }
    .domain_op(&source.f.group.group_id(), [12; 16])
    .unwrap();
    let late = writer
        .edit(&source.f.device, &source.f.group, &mut rng(), &domain)
        .unwrap();
    seal(&mut source);
    let before = fs::read(source.f.path(&source.store)).unwrap();
    let saved = stage(&mut source, 10).unwrap().unwrap();
    assert_eq!(saved.transition, RecoveryTransition::Promoted);
    let snapshot = saved.state.retained().next().unwrap().clone();
    let typed =
        RegistryRecovery::from_snapshot(&snapshot, &source.f.document, source.f.key.bucket())
            .unwrap();
    assert_eq!(typed.excluded_operations().len(), 1);
    assert_eq!(typed.projection().pointers[&source.f.key], 10);
    assert_eq!(fs::read(source.f.path(&source.store)).unwrap(), before);
    source
        .f
        .ingest(&mut source.store, &late, &mut source.budget)
        .unwrap();
    let before = fs::read(source.f.path(&source.store)).unwrap();
    let retry = stage(&mut source, 20).unwrap().unwrap();
    assert_eq!(retry.transition, RecoveryTransition::Unchanged);
    assert_eq!(retry.state.retained().len(), 1);
    assert_eq!(retry.state.retained().next().unwrap(), &snapshot);
    assert_eq!(fs::read(source.f.path(&source.store)).unwrap(), before);
    let TestSource {
        f,
        store,
        close,
        receipt,
        ..
    } = source;
    drop(store);
    let mut store = open(root.path());
    let budget = budget(&mut store, &f);
    let mut source = TestSource {
        f,
        store,
        budget,
        close,
        receipt,
    };
    assert_eq!(
        stage(&mut source, 30).unwrap().unwrap().transition,
        RecoveryTransition::Unchanged
    );
    assert_eq!(recovery(&source).retained().next().unwrap(), &snapshot);
    assert_eq!(fs::read(source.f.path(&source.store)).unwrap(), before);
}

#[test]
fn registry_recovery_stage_save_failures_preserve_source_and_retry_after_reconciliation() {
    let root = tempfile::tempdir().unwrap();
    let mut source = source_fixture(root.path(), true);
    seal(&mut source);
    let before = fs::read(source.f.path(&source.store)).unwrap();
    for after_rename in [false, true] {
        let f = &source.f;
        let result = source.store.stage_registry_recovery_with_writer(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            &source.close,
            0,
            &ManualClock::new(10),
            &mut rng(),
            &mut source.budget,
            |path, bytes| {
                if after_rename {
                    atomic_write(path, bytes)?;
                }
                Err(AppError::Io("injected recovery save failure".into()))
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(source.f.path(&source.store)).unwrap(), before);
        assert_eq!(
            recovery(&source).retained().len(),
            usize::from(after_rename)
        );
        assert!(
            stage(&mut source, 20).is_err(),
            "uncertain budget must reconcile before retry"
        );
        source.budget = budget(&mut source.store, &source.f);
    }
    let retry = stage(&mut source, 30).unwrap().unwrap();
    assert_eq!(retry.transition, RecoveryTransition::Unchanged);
    assert_eq!(retry.state.retained().len(), 1);
    assert_eq!(fs::read(source.f.path(&source.store)).unwrap(), before);
}

#[test]
fn registry_recovery_stage_refuses_stale_inventory_invalid_old_slots_and_storage_cap() {
    let root = tempfile::tempdir().unwrap();
    let mut source = source_fixture(root.path(), true);
    seal(&mut source);
    let before = fs::read(source.f.path(&source.store)).unwrap();
    let mut incomplete = EpochStorageBudget::from_inventory(
        StorageScope::new(SERVER, &source.f.group.group_id()).unwrap(),
        vec![],
    )
    .unwrap();
    let f = &source.f;
    assert!(source
        .store
        .stage_registry_recovery(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            &source.close,
            0,
            &ManualClock::new(0),
            &mut rng(),
            &mut incomplete
        )
        .is_err());
    assert_eq!(recovery(&source).retained().len(), 0);
    // Model the rest of a correctly inventoried server filling the content pool. No source
    // deletion may be credited early, and refusal must occur before the writer is called.
    let mut records = inventory(&mut source.store)
        .records_for_server(SERVER, &source.f.group.group_id())
        .unwrap();
    let content: u64 = records.iter().map(|record| record.footprint.content).sum();
    records.push(StorageRecord {
        id: [77; 32],
        document: [77; 32],
        footprint: Footprint {
            content: super::super::super::epoch_budget::CONTENT_ALLOWANCE_BYTES - content,
            ..Footprint::default()
        },
    });
    let mut full = EpochStorageBudget::from_inventory(
        StorageScope::new(SERVER, &source.f.group.group_id()).unwrap(),
        records,
    )
    .unwrap();
    let f = &source.f;
    let refused = source.store.stage_registry_recovery_with_writer(
        SERVER,
        &f.group,
        f.key.bucket(),
        &f.device,
        &source.close,
        0,
        &ManualClock::new(0),
        &mut rng(),
        &mut full,
        |_, _| panic!("storage refusal must precede I/O"),
    );
    assert!(refused.unwrap_err().to_string().contains("storage limit"));
    assert!(!full.requires_reconciliation());
    // A legacy generic snapshot is not validated just because its outer type/key happen to fit.
    let mut wrong = source
        .store
        .plan_registry_settlement(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            &source.close,
            0,
        )
        .unwrap()
        .recovery_snapshot()
        .unwrap()
        .unwrap();
    wrong.projection = b"opaque old payload".to_vec();
    source
        .store
        .update_epoch_recovery_accounted(
            SERVER,
            &source.f.document,
            EpochRecoveryAction::Stage(wrong.clone()),
            &ManualClock::new(0),
            &mut rng(),
            &mut source.budget,
        )
        .unwrap();
    assert!(stage(&mut source, 10).is_err());
    assert_eq!(recovery(&source).retained().next().unwrap(), &wrong);
    assert_eq!(fs::read(source.f.path(&source.store)).unwrap(), before);
}

#[test]
fn registry_recovery_stage_empty_plan_does_not_write_or_consume_a_slot() {
    let root = tempfile::tempdir().unwrap();
    let mut source = source_fixture(root.path(), false);
    seal(&mut source);
    let before = fs::read(source.f.path(&source.store)).unwrap();
    assert!(stage(&mut source, 0).unwrap().is_none());
    assert!(stage(&mut source, 10).unwrap().is_none());
    assert_eq!(recovery(&source).retained().len(), 0);
    assert_eq!(fs::read(source.f.path(&source.store)).unwrap(), before);
    assert_eq!(
        inventory(&mut source.store)
            .records_for_server(SERVER, &source.f.group.group_id())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn registry_recovery_stage_third_version_keeps_warning_deadline_across_retry() {
    let root = tempfile::tempdir().unwrap();
    let mut source = source_fixture(root.path(), true);
    seal(&mut source);
    let f = &source.f;
    let snapshot = source
        .store
        .plan_registry_settlement(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            &source.close,
            0,
        )
        .unwrap()
        .recovery_snapshot()
        .unwrap()
        .unwrap();
    // Valid historical typed versions occupy both retained slots. Changing only the selected
    // receipt hash makes distinct historical evidence; it grants no current receipt authority.
    for n in [1, 2] {
        let mut prior: RecoverySnapshot = snapshot.clone();
        let receipt_offset = 1 + 4 + f.group.group_id().len() + 1 + 4;
        prior.projection[receipt_offset] ^= n;
        RegistryRecovery::from_snapshot(&prior, &f.document, f.key.bucket()).unwrap();
        source
            .store
            .update_epoch_recovery_accounted(
                SERVER,
                &f.document,
                EpochRecoveryAction::Stage(prior),
                &ManualClock::new(n as u64),
                &mut rng(),
                &mut source.budget,
            )
            .unwrap();
    }
    let before = fs::read(source.f.path(&source.store)).unwrap();
    let pending = stage(&mut source, 10).unwrap().unwrap();
    assert!(matches!(
        pending.transition,
        RecoveryTransition::EvictionPending { .. }
    ));
    assert_eq!(pending.state.retained().len(), 2);
    assert_eq!(pending.state.staged(), Some(&snapshot));
    let retry = stage(&mut source, 50).unwrap().unwrap();
    assert_eq!(retry.transition, pending.transition);
    assert_eq!(
        recovery(&source).eviction_pending().unwrap(),
        Some(pending.transition)
    );
    assert_eq!(
        source.f.load(&source.store).unwrap().phase(),
        EpochPhase::Closing
    );
    assert_eq!(fs::read(source.f.path(&source.store)).unwrap(), before);
}
