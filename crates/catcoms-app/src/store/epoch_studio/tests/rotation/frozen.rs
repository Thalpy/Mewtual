use super::*;

fn sealed_old_owner(f: &mut Fixture, store: &mut ServerStore) -> Receipt {
    eligible(f, store);
    let mut state = f.load(store).unwrap();
    let old = state
        .unit
        .new_owner_decision(&f.group, &f.device, 0, None)
        .unwrap();
    let mut b = budget(store, f);
    store
        .prepare_studio_owner_decision_with_writer(
            SERVER,
            &old,
            &f.group,
            0,
            &mut rng(),
            &mut b.storage,
            atomic_write,
        )
        .unwrap();
    store
        .seal_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            old.receipt().clone(),
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    old.receipt().clone()
}

fn handoff(f: &mut Fixture) -> u64 {
    let next = MlsDevice::generate().unwrap();
    let welcome = f
        .group
        .add_member(&f.device, next.key_package().unwrap())
        .unwrap()
        .welcome;
    let mut group = ServerGroup::join(&next, &welcome).unwrap();
    group.remove_member(&next, &f.device.device_id()).unwrap();
    f.group = group;
    f.device = next;
    f.group.epoch()
}

#[test]
fn studio_frozen_owner_store_crash_matrix_retains_full_source_then_recovery_then_successor() {
    for art in [false, true] {
        for failure in [
            RotationWrite::Journal,
            RotationWrite::Source,
            RotationWrite::Recovery,
            RotationWrite::Successor,
        ] {
            for after_write in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let mut f = Fixture::new(art);
                let mut store = open(root.path());
                let old = sealed_old_owner(&mut f, &mut store);
                let original = f.load(&store).unwrap().projection().unwrap();
                let original_ops = f.load(&store).unwrap().op_count();
                let tenure = handoff(&mut f);
                drop(store);
                let mut store = open(root.path());
                warm(&f, &mut store);
                let mut b = budget(&mut store, &f);
                let mut hit = false;
                let result = store.rotate_studio_owner_with_io(
                    SERVER,
                    &f.group,
                    f.target,
                    &f.device,
                    tenure,
                    &ManualClock::new(1000),
                    &mut rng(),
                    &mut b,
                    &mut |step, path, bytes| {
                        if step == failure && !hit {
                            hit = true;
                            if after_write {
                                atomic_write(path, bytes)?;
                            }
                            return Err(invalid("injected frozen takeover crash"));
                        }
                        atomic_write(path, bytes)
                    },
                    &mut sync,
                );
                assert!(hit && result.is_err(), "{art}/{failure:?}/{after_write}");
                let held = f.load(&store).unwrap();
                let installed = held.epoch() == 1;
                if !installed {
                    assert_eq!(held.phase(), EpochPhase::Closing);
                    assert_eq!(held.projection().unwrap(), original);
                    assert_eq!(held.op_count(), original_ops);
                }
                let journal = store.load_epoch_owner_receipts(SERVER, &f.logical).unwrap();
                let selected = journal.pending().unwrap().clone();
                let exact = (selected.tenure_start_group_epoch == tenure).then(|| {
                    (
                        selected.clone(),
                        journal.close_for(&selected).unwrap().encode(),
                    )
                });
                if exact.is_none() {
                    assert_eq!(selected, old);
                }
                if installed {
                    assert!(store
                        .load_epoch_recovery(SERVER, &f.logical)
                        .unwrap()
                        .retained()
                        .next()
                        .is_some());
                }
                // A takeover is adoption, not proof that this device owns the old author's
                // envelope. It keeps that ledger; later author-local replay/manual recovery owns it.
                assert_eq!(f.intents(&store), 1);
                drop(store);
                let mut store = open(root.path());
                warm(&f, &mut store);
                let mut b = budget(&mut store, &f);
                let (outcome, state) = store
                    .rotate_studio_owner(
                        SERVER,
                        &f.group,
                        f.target,
                        &f.device,
                        tenure,
                        &ManualClock::new(2000),
                        &mut rng(),
                        &mut b,
                    )
                    .unwrap();
                assert!(matches!(
                    outcome,
                    StudioRotationOutcome::Installed { .. }
                        | StudioRotationOutcome::AlreadyInstalled { .. }
                ));
                assert_eq!((state.epoch(), state.phase()), (1, EpochPhase::Open));
                let recovery = store.load_epoch_recovery(SERVER, &f.logical).unwrap();
                assert_eq!(recovery.retained().len(), 1);
                let recovered = StudioRecovery::from_snapshot(
                    recovery.retained().next().unwrap(),
                    &f.logical,
                    f.target.channel(),
                )
                .unwrap();
                assert_eq!(recovered.projection(), &original);
                let journal = store.load_epoch_owner_receipts(SERVER, &f.logical).unwrap();
                if let Some((receipt, close)) = exact {
                    assert_eq!(journal.pending(), Some(&receipt));
                    assert_eq!(journal.close_for(&receipt).unwrap().encode(), close);
                }
                assert_eq!(journal.pending().unwrap().tenure_start_group_epoch, tenure);
                assert_eq!(f.intents(&store), 1);
                drop(store);
                let store = open(root.path());
                assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Open);
                assert_eq!(
                    store
                        .load_epoch_recovery(SERVER, &f.logical)
                        .unwrap()
                        .retained()
                        .len(),
                    1
                );
            }
        }
    }
}

#[test]
fn studio_frozen_owner_store_warning_holds_across_restart_until_exact_acknowledgement() {
    use catcoms_replication::{RecoveryReason, RecoveryTransition};
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut f = Fixture::new(art);
        let mut store = open(root.path());
        sealed_old_owner(&mut f, &mut store);
        let original = f.load(&store).unwrap().projection().unwrap();
        let clock = ManualClock::new(1000);
        let mut b = budget(&mut store, &f);
        for salt in [41, 42] {
            let snapshot = StudioRecovery::snapshot(
                &original,
                None,
                RecoveryReason::Excluded,
                [salt; 32],
                &Default::default(),
            )
            .unwrap();
            store
                .update_epoch_recovery_accounted(
                    SERVER,
                    &f.logical,
                    EpochRecoveryAction::Stage(snapshot),
                    &clock,
                    &mut rng(),
                    &mut b.storage,
                )
                .unwrap();
        }
        let tenure = handoff(&mut f);
        drop(store);
        let mut store = open(root.path());
        warm(&f, &mut store);
        let mut b = budget(&mut store, &f);
        let (outcome, state) = store
            .rotate_studio_owner(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                tenure,
                &clock,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        assert_eq!(outcome, StudioRotationOutcome::RecoveryPending);
        assert_eq!(state.phase(), EpochPhase::Closing);
        assert_eq!(state.projection().unwrap(), original);
        let pending = store
            .load_epoch_recovery(SERVER, &f.logical)
            .unwrap()
            .eviction_pending()
            .unwrap()
            .unwrap();
        let journal = store.load_epoch_owner_receipts(SERVER, &f.logical).unwrap();
        let receipt = journal.pending().unwrap().clone();
        drop(store);
        let mut store = open(root.path());
        warm(&f, &mut store);
        let mut b = budget(&mut store, &f);
        assert_eq!(
            store
                .rotate_studio_owner(
                    SERVER,
                    &f.group,
                    f.target,
                    &f.device,
                    tenure,
                    &clock,
                    &mut rng(),
                    &mut b
                )
                .unwrap()
                .0,
            StudioRotationOutcome::RecoveryPending
        );
        assert_eq!(
            store
                .load_epoch_recovery(SERVER, &f.logical)
                .unwrap()
                .eviction_pending()
                .unwrap(),
            Some(pending)
        );
        let RecoveryTransition::EvictionPending {
            oldest_snapshot,
            staged_snapshot,
            ..
        } = pending
        else {
            panic!()
        };
        let mut b = budget(&mut store, &f);
        store
            .update_epoch_recovery_accounted(
                SERVER,
                &f.logical,
                EpochRecoveryAction::Acknowledge {
                    oldest_snapshot,
                    staged_snapshot,
                },
                &clock,
                &mut rng(),
                &mut b.storage,
            )
            .unwrap();
        warm(&f, &mut store);
        let mut b = budget(&mut store, &f);
        let (_, state) = store
            .rotate_studio_owner(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                tenure,
                &clock,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        assert_eq!((state.epoch(), state.phase()), (1, EpochPhase::Open));
        assert_eq!(
            store
                .load_epoch_owner_receipts(SERVER, &f.logical)
                .unwrap()
                .pending(),
            Some(&receipt)
        );
        assert_eq!(
            store
                .load_epoch_recovery(SERVER, &f.logical)
                .unwrap()
                .retained()
                .len(),
            2
        );
        assert_eq!(
            f.intents(&store),
            1,
            "takeover does not retire another author's intent"
        );
    }
}

/// The seven-day grace is the bound on the hold, so an owner who never presses Acknowledge must
/// still settle. Before the persisted deadline the pass must hold and write no recovery record.
#[test]
fn studio_frozen_owner_store_promotes_staged_recovery_at_its_deadline_without_acknowledgement() {
    use catcoms_replication::{RecoveryReason, RecoveryTransition};
    use catcoms_rt::Clock;
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut f = Fixture::new(art);
        let mut store = open(root.path());
        sealed_old_owner(&mut f, &mut store);
        let original = f.load(&store).unwrap().projection().unwrap();
        let clock = ManualClock::new(1000);
        let mut b = budget(&mut store, &f);
        for salt in [41, 42] {
            let snapshot = StudioRecovery::snapshot(
                &original,
                None,
                RecoveryReason::Excluded,
                [salt; 32],
                &Default::default(),
            )
            .unwrap();
            store
                .update_epoch_recovery_accounted(
                    SERVER,
                    &f.logical,
                    EpochRecoveryAction::Stage(snapshot),
                    &clock,
                    &mut rng(),
                    &mut b.storage,
                )
                .unwrap();
        }
        let tenure = handoff(&mut f);
        drop(store);
        let mut store = open(root.path());
        warm(&f, &mut store);
        let mut b = budget(&mut store, &f);
        assert_eq!(
            store
                .rotate_studio_owner(
                    SERVER,
                    &f.group,
                    f.target,
                    &f.device,
                    tenure,
                    &clock,
                    &mut rng(),
                    &mut b
                )
                .unwrap()
                .0,
            StudioRotationOutcome::RecoveryPending
        );
        let warning = store
            .load_epoch_recovery(SERVER, &f.logical)
            .unwrap()
            .eviction_pending()
            .unwrap()
            .unwrap();
        let RecoveryTransition::EvictionPending {
            staged_snapshot,
            deadline_ms,
            ..
        } = warning
        else {
            panic!()
        };
        drop(store);
        let mut store = open(root.path());
        warm(&f, &mut store);
        clock.advance_ms(deadline_ms - 1 - clock.now_ms());
        let mut b = budget(&mut store, &f);
        let (outcome, state) = store
            .rotate_studio_owner_with_io(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                tenure,
                &clock,
                &mut rng(),
                &mut b,
                &mut |step, path, bytes| {
                    assert_ne!(
                        step,
                        RotationWrite::Recovery,
                        "an idle pass inside the grace must not rewrite the warning"
                    );
                    atomic_write(path, bytes)
                },
                &mut sync,
            )
            .unwrap();
        assert_eq!(outcome, StudioRotationOutcome::RecoveryPending);
        assert_eq!(state.phase(), EpochPhase::Closing);
        assert_eq!(
            store
                .load_epoch_recovery(SERVER, &f.logical)
                .unwrap()
                .eviction_pending()
                .unwrap(),
            Some(warning),
            "a restart resurfaces the original deadline, it does not restart the grace"
        );
        clock.advance_ms(1);
        warm(&f, &mut store);
        let mut b = budget(&mut store, &f);
        let (outcome, state) = store
            .rotate_studio_owner(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                tenure,
                &clock,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        assert!(
            matches!(outcome, StudioRotationOutcome::Installed { .. }),
            "{outcome:?}"
        );
        assert_eq!((state.epoch(), state.phase()), (1, EpochPhase::Open));
        let recovery = store.load_epoch_recovery(SERVER, &f.logical).unwrap();
        assert!(recovery.eviction_pending().unwrap().is_none());
        assert!(recovery.staged().is_none());
        assert_eq!(recovery.retained().len(), 2);
        assert!(
            recovery
                .retained()
                .any(|held| held.id().unwrap() == staged_snapshot),
            "the promoted version is the one the warning named"
        );
        drop(store);
        let store = open(root.path());
        assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Open);
        assert_eq!(
            store
                .load_epoch_recovery(SERVER, &f.logical)
                .unwrap()
                .retained()
                .len(),
            2
        );
    }
}
