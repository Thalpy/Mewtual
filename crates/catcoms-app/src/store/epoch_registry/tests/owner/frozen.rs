use super::*;

#[test]
fn registry_frozen_owner_store_restarts_each_boundary_without_changing_the_decision() {
    for failure in [
        OwnerRotationStep::JournalSaved,
        OwnerRotationStep::SourceSealed,
        OwnerRotationStep::SuccessorInstalled,
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut s = source_fixture(root.path(), false);
        let mut intents = EpochIntentBudget::from_inventory(&inventory(&mut s.store)).unwrap();
        // Interrupt the original owner after the source seal, before any installation.
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
                |step| if step == OwnerRotationStep::SourceSealed {
                    Err(invalid("old owner stops"))
                } else {
                    Ok(())
                }
            )
            .is_err());
        assert_eq!(s.f.load(&s.store).unwrap().phase(), EpochPhase::Closing);
        let original = s.f.load(&s.store).unwrap().projection().unwrap();
        let next = MlsDevice::generate().unwrap();
        let welcome =
            s.f.group
                .add_member(&s.f.device, next.key_package().unwrap())
                .unwrap()
                .welcome;
        let mut group = ServerGroup::join(&next, &welcome).unwrap();
        group.remove_member(&next, &s.f.device.device_id()).unwrap();
        s.f.device = next;
        s.f.group = group;
        let tenure = s.f.group.epoch();
        drop(s.store);
        s.store = open(root.path());
        s.budget = budget(&mut s.store, &s.f);
        intents = EpochIntentBudget::from_inventory(&inventory(&mut s.store)).unwrap();
        let mut hit = false;
        let result = s.store.rotate_registry_owner_with_hook(
            SERVER,
            &s.f.group,
            s.f.key.bucket(),
            &s.f.device,
            tenure,
            &ManualClock::new(200),
            &mut rng(),
            &mut s.budget,
            &mut intents,
            |step| {
                if step == failure {
                    hit = true;
                    Err(invalid("new owner stops"))
                } else {
                    Ok(())
                }
            },
        );
        assert!(hit && result.is_err());
        let journal = s
            .store
            .load_epoch_owner_receipts(SERVER, &s.f.document)
            .unwrap();
        let receipt = journal.pending().unwrap().clone();
        let close = journal.close_for(&receipt).unwrap().encode();
        assert_eq!(receipt.tenure_start_group_epoch, tenure);
        let state = s.f.load(&s.store).unwrap();
        if state.epoch() == 0 {
            assert_eq!(state.phase(), EpochPhase::Closing);
            assert_eq!(state.projection().unwrap(), original);
        } else {
            assert_eq!(
                s.store
                    .load_epoch_recovery(SERVER, &s.f.document)
                    .unwrap()
                    .retained()
                    .len(),
                1
            );
        }
        drop(s.store);
        s.store = open(root.path());
        s.budget = budget(&mut s.store, &s.f);
        intents = EpochIntentBudget::from_inventory(&inventory(&mut s.store)).unwrap();
        let (outcome, state) = s
            .store
            .rotate_registry_owner(
                SERVER,
                &s.f.group,
                s.f.key.bucket(),
                &s.f.device,
                tenure,
                &ManualClock::new(300),
                &mut rng(),
                &mut s.budget,
                &mut intents,
            )
            .unwrap();
        assert!(matches!(
            outcome,
            RegistryOwnerRotationOutcome::Installed { .. }
                | RegistryOwnerRotationOutcome::AlreadyInstalled { .. }
        ));
        assert_eq!((state.epoch(), state.phase()), (1, EpochPhase::Open));
        let journal = s
            .store
            .load_epoch_owner_receipts(SERVER, &s.f.document)
            .unwrap();
        assert_eq!(journal.pending(), Some(&receipt));
        assert_eq!(journal.close_for(&receipt).unwrap().encode(), close);
        let recovery = s.store.load_epoch_recovery(SERVER, &s.f.document).unwrap();
        assert_eq!(recovery.retained().len(), 1);
        let recovered = RegistryRecovery::from_snapshot(
            recovery.retained().next().unwrap(),
            &s.f.document,
            s.f.key.bucket(),
        )
        .unwrap();
        assert_eq!(recovered.projection(), &original);
        drop(s.store);
        s.store = open(root.path());
        assert_eq!(s.f.load(&s.store).unwrap().phase(), EpochPhase::Open);
    }
}
