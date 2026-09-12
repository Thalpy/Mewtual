//! Real bounded closures exercise the production settlement ordering and restart file paths.
use super::super::installation::{InstallSync, InstallWrite};
use super::settlement::{source_fixture, TestSource};
use super::*;
use catcoms_replication::registry::RegistryRecovery;
use catcoms_rt::ManualClock;

fn refresh(s: &mut TestSource) -> EpochIntentBudget {
    let inv = inventory(&mut s.store);
    s.budget = EpochStorageBudget::from_inventory(
        StorageScope::new(SERVER, &s.f.group.group_id()).unwrap(),
        inv.records_for_server(SERVER, &s.f.group.group_id())
            .unwrap(),
    )
    .unwrap();
    EpochIntentBudget::from_inventory(&inv).unwrap()
}
fn domain(f: &Fixture, nonce: u8, epoch: u64) -> DomainOp {
    RegistryOp::Put {
        key: f.key.clone(),
        epoch,
    }
    .domain_op(&f.group.group_id(), [nonce; 16])
    .unwrap()
}
fn journal(s: &mut TestSource, intents: &mut EpochIntentBudget, nonce: u8, epoch: u64) {
    s.store
        .prepare_epoch_intent(
            SERVER,
            &s.f.document,
            domain(&s.f, nonce, epoch),
            &s.f.device,
            &s.f.group,
            &mut rng(),
            &mut s.budget,
            intents,
        )
        .unwrap();
}
fn seal(s: &mut TestSource) {
    s.store
        .seal_registry_epoch(
            SERVER,
            &s.f.group,
            s.f.key.bucket(),
            &s.f.device,
            s.receipt.clone(),
            0,
            &mut rng(),
            &mut s.budget,
        )
        .unwrap();
}
fn install(
    s: &mut TestSource,
    intents: &mut EpochIntentBudget,
) -> Result<(RegistryInstallOutcome, EpochRegistryState), AppError> {
    s.store.install_registry_checkpoint(
        SERVER,
        &s.f.group,
        s.f.key.bucket(),
        &s.f.device,
        &s.receipt.encode(),
        &s.close,
        0,
        &ManualClock::new(10),
        &mut rng(),
        &mut s.budget,
        intents,
    )
}
fn pending(s: &TestSource) -> Vec<u8> {
    s.store
        .load_epoch_intents(SERVER, &s.f.document)
        .unwrap()
        .pending()
        .map(|(_, intent)| intent.operation.nonce[0])
        .collect()
}
fn sync(step: InstallSync, path: &Path, bytes: u64) -> Result<(), AppError> {
    match step {
        InstallSync::Intents => crate::store::epoch_intents::sync_intent(path, bytes),
        _ => sync_registry(path, bytes),
    }
}

#[test]
fn registry_install_retires_only_exact_covered_intents_and_retry_preserves_new_edits() {
    let root = tempfile::tempdir().unwrap();
    let mut s = source_fixture(root.path(), true);
    let mut intents = refresh(&mut s);
    journal(&mut s, &mut intents, 1, 1); // covered
    journal(&mut s, &mut intents, 10, 10); // accepted but excluded
    journal(&mut s, &mut intents, 12, 12); // intent not yet accepted anywhere
    seal(&mut s);
    let (outcome, state) = install(&mut s, &mut intents).unwrap();
    assert_eq!(outcome, RegistryInstallOutcome::Installed);
    assert_eq!(
        (state.epoch(), state.phase(), state.op_count()),
        (1, EpochPhase::Open, 0)
    );
    assert_eq!(state.projection().unwrap().pointers[&s.f.key], 9);
    assert_eq!(pending(&s).len(), 2);
    let original_close = s.close.clone();
    let mut bad_close = catcoms_replication::CloseRecord::decode(&original_close).unwrap();
    bad_close.signature[0] ^= 1; // unsigned close hash is deliberately unchanged
    s.close = bad_close.encode();
    let before = fs::read(s.f.path(&s.store)).unwrap();
    assert!(
        install(&mut s, &mut intents).is_err(),
        "installed retry must verify close signature too"
    );
    assert_eq!(fs::read(s.f.path(&s.store)).unwrap(), before);
    s.close = original_close;
    assert!(!pending(&s).contains(&1));
    let recovery = s.store.load_epoch_recovery(SERVER, &s.f.document).unwrap();
    let snapshot = recovery.retained().next().unwrap();
    let typed = RegistryRecovery::from_snapshot(snapshot, &s.f.document, s.f.key.bucket()).unwrap();
    assert_eq!(typed.projection().pointers[&s.f.key], 10);
    let before = fs::read(s.f.path(&s.store)).unwrap();
    let stale = s.store.edit_registry_epoch(
        SERVER,
        &s.f.group,
        s.f.key.bucket(),
        s.f.source.doc_id(),
        &s.f.device,
        domain(&s.f, 1, 1),
        &mut rng(),
        &mut s.budget,
        &mut intents,
    );
    assert!(stale.unwrap_err().to_string().contains("retired epoch"));
    assert_eq!(fs::read(s.f.path(&s.store)).unwrap(), before);
    assert_eq!(
        pending(&s).len(),
        2,
        "stale Save cannot recreate a finalized intent"
    );
    // Explicitly targeting the freshly read epoch permits author-owned excluded replay.
    s.store
        .edit_registry_epoch(
            SERVER,
            &s.f.group,
            s.f.key.bucket(),
            state.doc_id(),
            &s.f.device,
            domain(&s.f, 10, 10),
            &mut rng(),
            &mut s.budget,
            &mut intents,
        )
        .unwrap();
    let current = s.f.load(&s.store).unwrap();
    let next_seed = current.projection().unwrap().checkpoint([99; 32]).unwrap();
    let next_receipt = Receipt::sign(
        s.f.document.clone(),
        1,
        [99; 32],
        next_seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        &s.f.device,
    )
    .unwrap();
    s.store
        .seal_registry_epoch(
            SERVER,
            &s.f.group,
            s.f.key.bucket(),
            &s.f.device,
            next_receipt,
            0,
            &mut rng(),
            &mut s.budget,
        )
        .unwrap();
    let before = fs::read(s.f.path(&s.store)).unwrap();
    drop(s.store);
    s.store = open(root.path());
    intents = refresh(&mut s);
    let (outcome, state) = install(&mut s, &mut intents).unwrap();
    assert_eq!(outcome, RegistryInstallOutcome::AlreadyInstalled);
    assert_eq!(state.op_count(), 1);
    assert_eq!(
        state.phase(),
        EpochPhase::Closing,
        "installed retry preserves newer seal"
    );
    assert_eq!(state.projection().unwrap().pointers[&s.f.key], 10);
    assert_eq!(
        fs::read(s.f.path(&s.store)).unwrap(),
        before,
        "retry syncs actual successor"
    );
    assert_eq!(pending(&s).len(), 2);
}

#[test]
fn registry_install_empty_recovery_and_missing_ledger_create_only_successor() {
    let root = tempfile::tempdir().unwrap();
    let mut s = source_fixture(root.path(), false);
    let mut intents = refresh(&mut s);
    seal(&mut s);
    assert_eq!(
        install(&mut s, &mut intents).unwrap().0,
        RegistryInstallOutcome::Installed
    );
    assert_eq!(
        inventory(&mut s.store)
            .records_for_server(SERVER, &s.f.group.group_id())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(intents.bytes(), 0);
}

#[test]
fn registry_install_crash_boundaries_keep_source_or_successor_and_resume_without_loss() {
    for step in [
        InstallWrite::Recovery,
        InstallWrite::Intents,
        InstallWrite::Successor,
    ] {
        for mode in 0..3 {
            // before write, after rename, writer unwind
            let root = tempfile::tempdir().unwrap();
            let mut s = source_fixture(root.path(), true);
            let mut intents = refresh(&mut s);
            journal(&mut s, &mut intents, 1, 1);
            journal(&mut s, &mut intents, 10, 10);
            seal(&mut s);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                s.store.install_registry_checkpoint_with_io(
                    SERVER,
                    &s.f.group,
                    s.f.key.bucket(),
                    &s.f.device,
                    &s.receipt.encode(),
                    &s.close,
                    0,
                    &ManualClock::new(10),
                    &mut rng(),
                    &mut s.budget,
                    &mut intents,
                    &mut |at, path, bytes| {
                        if at == step {
                            if mode == 1 {
                                atomic_write(path, bytes)?;
                            }
                            if mode == 2 {
                                panic!("injected installation panic");
                            }
                            return Err(AppError::Io("injected installation write failure".into()));
                        }
                        atomic_write(path, bytes)
                    },
                    &mut sync,
                )
            }));
            assert!(result.is_err() || result.unwrap().is_err());
            assert!(s.budget.requires_reconciliation());
            let after = s.f.load(&s.store).unwrap();
            assert_eq!(
                after.epoch(),
                u64::from(step == InstallWrite::Successor && mode == 1)
            );
            if after.epoch() == 0 {
                assert_eq!((after.phase(), after.op_count()), (EpochPhase::Closing, 11));
            }
            assert!(
                pending(&s).contains(&10),
                "never retire excluded author work"
            );
            if step == InstallWrite::Successor {
                assert!(
                    !pending(&s).contains(&1),
                    "retirement durable before selection"
                );
            }
            assert!(
                install(&mut s, &mut intents).is_err(),
                "reconcile uncertain I/O first"
            );
            drop(s.store);
            s.store = open(root.path());
            intents = refresh(&mut s);
            let (_, state) = install(&mut s, &mut intents).unwrap();
            assert_eq!((state.epoch(), state.op_count()), (1, 0));
            assert_eq!(pending(&s), vec![10]);
            let recovery = s.store.load_epoch_recovery(SERVER, &s.f.document).unwrap();
            assert_eq!(
                recovery.retained().len(),
                1,
                "retries do not allocate extra recovery slots"
            );
        }
    }
}

#[test]
fn registry_install_sync_failure_never_retires_before_source_flush() {
    let root = tempfile::tempdir().unwrap();
    let mut s = source_fixture(root.path(), true);
    let mut intents = refresh(&mut s);
    journal(&mut s, &mut intents, 1, 1);
    seal(&mut s);
    let before = fs::read(s.f.path(&s.store)).unwrap();
    let result = s.store.install_registry_checkpoint_with_io(
        SERVER,
        &s.f.group,
        s.f.key.bucket(),
        &s.f.device,
        &s.receipt.encode(),
        &s.close,
        0,
        &ManualClock::new(10),
        &mut rng(),
        &mut s.budget,
        &mut intents,
        &mut |_, _, _| panic!("source durability must precede every write"),
        &mut |step, _, _| {
            assert_eq!(step, InstallSync::Source);
            Err(AppError::Io("source flush failed".into()))
        },
    );
    assert!(result.is_err());
    assert_eq!(pending(&s), vec![1]);
    assert_eq!(fs::read(s.f.path(&s.store)).unwrap(), before);
    assert_eq!(
        s.store
            .load_epoch_recovery(SERVER, &s.f.document)
            .unwrap()
            .retained()
            .len(),
        0
    );
}

#[test]
fn registry_install_eviction_warning_holds_retirement_until_exact_acknowledgement() {
    let root = tempfile::tempdir().unwrap();
    let mut s = source_fixture(root.path(), true);
    let mut intents = refresh(&mut s);
    journal(&mut s, &mut intents, 1, 1);
    seal(&mut s);
    let snapshot = s
        .store
        .plan_registry_settlement(
            SERVER,
            &s.f.group,
            s.f.key.bucket(),
            &s.f.device,
            &s.close,
            0,
        )
        .unwrap()
        .recovery_snapshot()
        .unwrap()
        .unwrap();
    // Two valid typed historical versions fill the retained slots. The third must wait.
    for n in [1, 2] {
        let mut prior = snapshot.clone();
        let receipt_offset = 1 + 4 + s.f.group.group_id().len() + 1 + 4;
        prior.projection[receipt_offset] ^= n;
        RegistryRecovery::from_snapshot(&prior, &s.f.document, s.f.key.bucket()).unwrap();
        s.store
            .update_epoch_recovery_accounted(
                SERVER,
                &s.f.document,
                EpochRecoveryAction::Stage(prior),
                &ManualClock::new(0),
                &mut rng(),
                &mut s.budget,
            )
            .unwrap();
    }
    let before = fs::read(s.f.path(&s.store)).unwrap();
    for _ in 0..2 {
        let (outcome, state) = install(&mut s, &mut intents).unwrap();
        assert_eq!(outcome, RegistryInstallOutcome::RecoveryPending);
        assert_eq!(state.phase(), EpochPhase::Closing);
        assert_eq!(pending(&s), vec![1]);
        assert_eq!(fs::read(s.f.path(&s.store)).unwrap(), before);
    }
    let state = s.store.load_epoch_recovery(SERVER, &s.f.document).unwrap();
    let catcoms_replication::RecoveryTransition::EvictionPending {
        oldest_snapshot,
        staged_snapshot,
        ..
    } = state.eviction_pending().unwrap().unwrap()
    else {
        panic!("missing warning")
    };
    s.store
        .update_epoch_recovery_accounted(
            SERVER,
            &s.f.document,
            EpochRecoveryAction::Acknowledge {
                oldest_snapshot,
                staged_snapshot,
            },
            &ManualClock::new(11),
            &mut rng(),
            &mut s.budget,
        )
        .unwrap();
    assert_eq!(
        install(&mut s, &mut intents).unwrap().0,
        RegistryInstallOutcome::Installed
    );
    assert!(pending(&s).is_empty());
    assert_eq!(
        s.store
            .load_epoch_recovery(SERVER, &s.f.document)
            .unwrap()
            .retained()
            .len(),
        2
    );
}

#[test]
fn registry_install_empty_plan_still_checks_existing_recovery_and_complete_inventory() {
    let root = tempfile::tempdir().unwrap();
    let mut s = source_fixture(root.path(), false);
    let mut intents = refresh(&mut s);
    seal(&mut s);
    let before = fs::read(s.f.path(&s.store)).unwrap();
    let mut incomplete = EpochStorageBudget::from_inventory(
        StorageScope::new(SERVER, &s.f.group.group_id()).unwrap(),
        vec![],
    )
    .unwrap();
    std::mem::swap(&mut s.budget, &mut incomplete);
    assert!(install(&mut s, &mut intents).is_err());
    intents = refresh(&mut s);
    // An opaque generic recovery record cannot be silently ignored by an empty new plan.
    let opaque = catcoms_replication::RecoverySnapshot {
        doc_type: s.f.document.doc_type,
        logical_key: s.f.document.logical_key.clone(),
        epoch: 0,
        base_close_record_hash: None,
        reason: catcoms_replication::RecoveryReason::Excluded,
        projection: b"opaque legacy payload".to_vec(),
        tombstones: vec![],
        elements: vec![],
        conflicts: vec![],
        applied_ops: vec![],
    };
    s.store
        .update_epoch_recovery_accounted(
            SERVER,
            &s.f.document,
            EpochRecoveryAction::Stage(opaque),
            &ManualClock::new(0),
            &mut rng(),
            &mut s.budget,
        )
        .unwrap();
    assert!(install(&mut s, &mut intents).is_err());
    assert!(s.budget.requires_reconciliation());
    assert_eq!(fs::read(s.f.path(&s.store)).unwrap(), before);
    assert_eq!(s.f.load(&s.store).unwrap().op_count(), 10);
}

#[test]
fn registry_install_refuses_conflicting_intent_body_and_wrong_receipt_without_pruning() {
    let root = tempfile::tempdir().unwrap();
    let mut s = source_fixture(root.path(), true);
    let mut intents = refresh(&mut s);
    journal(&mut s, &mut intents, 1, 99); // identical author/nonce/id; different unreceipted body
    assert!(
        install(&mut s, &mut intents).is_err(),
        "Open source cannot install"
    );
    seal(&mut s);
    let before = fs::read(s.f.path(&s.store)).unwrap();
    let selected = s.receipt.clone();
    s.receipt.signature[0] ^= 1;
    assert!(install(&mut s, &mut intents).is_err());
    assert!(!s.budget.requires_reconciliation());
    s.receipt = selected;
    assert!(install(&mut s, &mut intents)
        .unwrap_err()
        .to_string()
        .contains("intent envelope conflicts"));
    assert_eq!(pending(&s), vec![1]);
    assert_eq!(fs::read(s.f.path(&s.store)).unwrap(), before);
    assert_eq!(s.f.load(&s.store).unwrap().op_count(), 11);
}
