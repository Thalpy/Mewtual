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
            &mut WriteHooks::None,
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
            WriteTag::Journal,
            WriteTag::Source,
            WriteTag::Recovery,
            WriteTag::Successor,
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
                let source_before = fs::read(f.path(&store)).unwrap();
                let hit = std::cell::Cell::new(false);
                // As in the ordinary matrix: the takeover flushes the held source before it
                // seals it, so a Source-tagged after decision sees two events on one record.
                // This fixture masks the difference especially well, because the predecessor is
                // already Closing and the "unchanged projection" checks below hold either way.
                let completed: std::cell::RefCell<Vec<(CompletedOperation, WriteTag)>> =
                    std::cell::RefCell::new(Vec::new());
                let result = store.rotate_studio_owner_with_io(
                    SERVER,
                    &f.group,
                    f.target,
                    &f.device,
                    tenure,
                    &ManualClock::new(1000),
                    &mut rng(),
                    &mut b,
                    &mut WriteHooks::Hooked {
                        before: Some(&mut |step: WriteTag, _: &Path, _: &[u8]| {
                            if step == failure && !hit.get() && !after_write {
                                hit.set(true);
                                return Intercept::Fail(invalid("injected frozen takeover crash"));
                            }
                            Intercept::Continue
                        }),
                        before_sync: None,
                        before_unlink: None,
                        after: Some(&mut |op: CompletedOperation, step: WriteTag, _: &Path| {
                            completed.borrow_mut().push((op, step));
                            if op == CompletedOperation::Write
                                && step == failure
                                && !hit.get()
                                && after_write
                            {
                                hit.set(true);
                                return AfterIntercept::Fail(invalid(
                                    "injected frozen takeover crash",
                                ));
                            }
                            AfterIntercept::Continue
                        }),
                    },
                );
                assert!(
                    hit.get() && result.is_err(),
                    "{art}/{failure:?}/{after_write}"
                );
                if after_write {
                    let seen = completed.borrow();
                    assert_eq!(
                        seen.last(),
                        Some(&(CompletedOperation::Write, failure)),
                        "the injection fired at an earlier operation, not after the {failure:?} \
                         replacement it names: {seen:?}"
                    );
                }
                // The predecessor is already Closing, so phase and projection cannot tell a
                // failure after the takeover's Source replacement from one before it. The
                // record's own bytes can.
                if failure == WriteTag::Source {
                    let durable = fs::read(f.path(&store)).unwrap();
                    if after_write {
                        assert_ne!(
                            durable, source_before,
                            "the takeover's sealed source never reached disk"
                        );
                    } else {
                        assert_eq!(
                            durable, source_before,
                            "a failure before the Source replacement must leave the \
                             predecessor's source untouched"
                        );
                    }
                }
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
                &mut WriteHooks::Hooked {
                    before: Some(&mut |step: WriteTag, _: &Path, _: &[u8]| {
                        assert_ne!(
                            step,
                            WriteTag::Recovery,
                            "an idle pass inside the grace must not rewrite the warning"
                        );
                        Intercept::Continue
                    }),
                    before_sync: None,
                    before_unlink: None,
                    after: None,
                },
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
