use super::settlement::{source_fixture, TestSource};
use super::*;
use crate::store::epoch_registry::owner::OwnerRotationStep;
use catcoms_replication::registry::RegistryRecovery;
use catcoms_rt::ManualClock;

mod frozen;

fn rotate(
    s: &mut TestSource,
    intents: &mut EpochIntentBudget,
) -> Result<(RegistryOwnerRotationOutcome, EpochRegistryState), AppError> {
    s.store.rotate_registry_owner(
        SERVER,
        &s.f.group,
        s.f.key.bucket(),
        &s.f.device,
        0,
        &ManualClock::new(100),
        &mut rng(),
        &mut s.budget,
        intents,
    )
}

#[test]
fn registry_owner_rotation_crash_resume_keeps_exact_decision_and_later_edits() {
    for step in [
        OwnerRotationStep::JournalSaved,
        OwnerRotationStep::SourceSealed,
        OwnerRotationStep::SuccessorInstalled,
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut s = source_fixture(root.path(), false);
        let mut intents = EpochIntentBudget::from_inventory(&inventory(&mut s.store)).unwrap();
        assert!(s
            .store
            .rotate_registry_owner_with_hook(
                SERVER,
                &s.f.group,
                s.f.key.bucket(),
                &s.f.device,
                0,
                &ManualClock::new(100),
                &mut rng(),
                &mut s.budget,
                &mut intents,
                |at| {
                    if at == step {
                        Err(invalid("injected crash"))
                    } else {
                        Ok(())
                    }
                }
            )
            .is_err());
        let journal = s
            .store
            .load_epoch_owner_receipts(SERVER, &s.f.document)
            .unwrap();
        let receipt = journal.pending().unwrap().clone();
        let close = journal.close_for(&receipt).unwrap().encode();
        // The exact candidate must match the known eligible closure, not just some valid seed.
        assert_eq!(receipt, s.receipt);
        assert_eq!(close, s.close);
        let current = s.f.load(&s.store).unwrap();
        if step != OwnerRotationStep::SourceSealed {
            let op = RegistryOp::Put {
                key: s.f.key.clone(),
                epoch: 77,
            }
            .domain_op(&s.f.group.group_id(), [77; 16])
            .unwrap();
            s.store
                .edit_registry_epoch(
                    SERVER,
                    &s.f.group,
                    s.f.key.bucket(),
                    current.doc_id(),
                    &s.f.device,
                    op,
                    &mut rng(),
                    &mut s.budget,
                    &mut intents,
                )
                .unwrap();
        }
        drop(s.store);
        s.store = open(root.path());
        s.budget = budget(&mut s.store, &s.f);
        intents = EpochIntentBudget::from_inventory(&inventory(&mut s.store)).unwrap();
        let (outcome, state) = rotate(&mut s, &mut intents).unwrap();
        assert_eq!(state.epoch(), 1);
        assert_eq!(
            outcome,
            if step == OwnerRotationStep::SuccessorInstalled {
                RegistryOwnerRotationOutcome::AlreadyInstalled {
                    publication_pending: true,
                }
            } else {
                RegistryOwnerRotationOutcome::Installed {
                    publication_pending: true,
                }
            }
        );
        let after = s
            .store
            .load_epoch_owner_receipts(SERVER, &s.f.document)
            .unwrap();
        assert_eq!(after.pending(), Some(&receipt));
        assert_eq!(after.close_for(&receipt).unwrap().encode(), close);
        if step == OwnerRotationStep::JournalSaved {
            assert_eq!(state.projection().unwrap().pointers[&s.f.key], 9);
            let recovery = s.store.load_epoch_recovery(SERVER, &s.f.document).unwrap();
            let typed = RegistryRecovery::from_snapshot(
                recovery.retained().next().unwrap(),
                &s.f.document,
                s.f.key.bucket(),
            )
            .unwrap();
            assert_eq!(typed.projection().pointers[&s.f.key], 77);
            assert_eq!(
                s.store
                    .load_epoch_intents(SERVER, &s.f.document)
                    .unwrap()
                    .pending()
                    .count(),
                1
            );
        } else if step == OwnerRotationStep::SuccessorInstalled {
            assert_eq!(state.projection().unwrap().pointers[&s.f.key], 77);
            assert_eq!(state.op_count(), 1);
        }
        assert_eq!(
            rotate(&mut s, &mut intents).unwrap().0,
            RegistryOwnerRotationOutcome::AlreadyInstalled {
                publication_pending: true
            }
        );
    }
}

#[test]
fn registry_owner_rotation_legacy_pending_without_close_holds_and_is_not_regenerated() {
    let root = tempfile::tempdir().unwrap();
    let mut s = source_fixture(root.path(), false);
    s.store
        .prepare_epoch_owner_receipt(
            SERVER,
            s.receipt.clone(),
            &s.f.group,
            0,
            &mut rng(),
            &mut s.budget,
        )
        .unwrap();
    let before = fs::read(s.f.path(&s.store)).unwrap();
    let mut intents = EpochIntentBudget::from_inventory(&inventory(&mut s.store)).unwrap();
    assert_eq!(
        rotate(&mut s, &mut intents).unwrap().0,
        RegistryOwnerRotationOutcome::DecisionNeedsClose
    );
    assert_eq!(fs::read(s.f.path(&s.store)).unwrap(), before);
    assert_eq!(
        s.store
            .load_epoch_owner_receipts(SERVER, &s.f.document)
            .unwrap()
            .pending(),
        Some(&s.receipt)
    );
}

#[test]
fn registry_owner_rotation_combined_journal_write_is_atomic_and_uncertain_io_needs_rescan() {
    for after_write in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut s = source_fixture(root.path(), false);
        let mut state = s.f.load(&s.store).unwrap();
        let decision = state
            .unit
            .new_owner_decision(&s.f.group, &s.f.device, 0, None)
            .unwrap();
        assert!(s
            .store
            .prepare_registry_owner_decision_with_writer(
                SERVER,
                &decision,
                &s.f.group,
                0,
                &mut rng(),
                &mut s.budget,
                |path, bytes| {
                    if after_write {
                        atomic_write(path, bytes)?;
                    }
                    Err(invalid("injected combined journal write/flush failure"))
                }
            )
            .is_err());
        let journal = s
            .store
            .load_epoch_owner_receipts(SERVER, &s.f.document)
            .unwrap();
        if after_write {
            assert_eq!(journal.pending(), Some(decision.receipt()));
            assert_eq!(
                journal.close_for(decision.receipt()).unwrap().encode(),
                decision.close().encode()
            );
        } else {
            assert!(journal.pending().is_none());
        }
        let mut intents = EpochIntentBudget::from_inventory(&inventory(&mut s.store)).unwrap();
        assert!(
            rotate(&mut s, &mut intents).is_err(),
            "the invalidated budget cannot grant a retry"
        );
        assert_eq!(s.f.load(&s.store).unwrap().phase(), EpochPhase::Open);
        drop(s.store);
        s.store = open(root.path());
        s.budget = budget(&mut s.store, &s.f);
        intents = EpochIntentBudget::from_inventory(&inventory(&mut s.store)).unwrap();
        assert_eq!(
            rotate(&mut s, &mut intents).unwrap().0,
            RegistryOwnerRotationOutcome::Installed {
                publication_pending: true
            }
        );
        assert_eq!(
            s.store
                .load_epoch_owner_receipts(SERVER, &s.f.document)
                .unwrap()
                .pending(),
            Some(decision.receipt())
        );
    }
}
