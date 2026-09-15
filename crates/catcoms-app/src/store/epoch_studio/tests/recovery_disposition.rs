use super::*;
use catcoms_rt::ManualClock;
use std::collections::BTreeSet;

#[test]
fn studio_manual_recovery_disposition_is_recovery_first_crash_safe_and_not_seed_finality() {
    for art in [false, true] {
        for recovery_boundary in [false, true] {
            for after_write in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let mut store = open(root.path());
                let f = Fixture::new(art);
                let op = f.insert();
                let ids = BTreeSet::from([op.id(&f.device.device_id())]);
                let mut b = budget(&mut store, &f);
                f.edit(&mut store, &mut b, op);
                let (receipt, seed) = super::adoption::checkpoint(&f, &store, 0, 8);
                let (_, state) =
                    super::adoption::adopt(&f, &mut store, &receipt, Some(seed.bytes()));
                assert_eq!(
                    f.intents(&store),
                    1,
                    "adoption alone is not a finality proof"
                );
                let epoch = state.doc_id();
                let before = store.load_epoch_recovery(SERVER, &f.logical).unwrap();
                let snapshot = before.retained().next().unwrap().id().unwrap();
                let mut b = budget(&mut store, &f);
                let mut hit = false;
                assert!(store
                    .move_studio_intents_to_recovery_with_io(
                        SERVER,
                        &f.group,
                        f.target,
                        &f.device,
                        epoch,
                        &ids,
                        &ManualClock::new(1000),
                        &mut rng(),
                        &mut b,
                        &mut |recovery, path, bytes| {
                            if recovery == recovery_boundary {
                                hit = true;
                                if after_write {
                                    atomic_write(path, bytes)?;
                                }
                                return Err(AppError::Io("injected disposition barrier".into()));
                            }
                            atomic_write(path, bytes)
                        },
                        &mut super::super::super::epoch_intents::sync_intent,
                    )
                    .is_err());
                assert!(hit);
                drop(store);
                store = open(root.path());
                assert_eq!(
                    f.intents(&store),
                    if !recovery_boundary && after_write {
                        0
                    } else {
                        1
                    }
                );
                assert_eq!(
                    store
                        .load_epoch_recovery(SERVER, &f.logical)
                        .unwrap()
                        .retained()
                        .next()
                        .unwrap()
                        .id()
                        .unwrap(),
                    snapshot
                );
                let mut b = budget(&mut store, &f);
                let mut synced = false;
                store
                    .move_studio_intents_to_recovery_with_io(
                        SERVER,
                        &f.group,
                        f.target,
                        &f.device,
                        epoch,
                        &ids,
                        &ManualClock::new(1000),
                        &mut rng(),
                        &mut b,
                        &mut |_, p, b| atomic_write(p, b),
                        &mut |p, b| {
                            synced = true;
                            super::super::super::epoch_intents::sync_intent(p, b)
                        },
                    )
                    .unwrap();
                assert_eq!(
                    synced,
                    !recovery_boundary && after_write,
                    "empty retry flushes the actual ledger"
                );
                assert_eq!(f.intents(&store), 0);
                let current = f.load(&store).unwrap();
                assert_eq!(current.op_count(), 0, "disposition authors nothing");
                assert_eq!(
                    current.phase(),
                    EpochPhase::Open,
                    "the new epoch remains provisional"
                );
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
fn studio_manual_recovery_refuses_current_log_stale_epoch_and_missing_evidence() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let op = f.insert();
    let id = op.id(&f.device.device_id());
    let ids = BTreeSet::from([id]);
    let mut b = budget(&mut store, &f);
    let (_, state) = f.edit(&mut store, &mut b, op.clone());
    let snapshot = catcoms_replication::studio::StudioRecovery::snapshot(
        &state.projection().unwrap(),
        None,
        catcoms_replication::RecoveryReason::Excluded,
        [1; 32],
        &state.current_operations().unwrap(),
    )
    .unwrap();
    store
        .update_epoch_recovery(
            SERVER,
            &f.logical,
            EpochRecoveryAction::Stage(snapshot.clone()),
            &ManualClock::new(100),
            &mut rng(),
        )
        .unwrap();
    for expected in [f.id, f.id.wrapping_add(1)] {
        let mut b = budget(&mut store, &f);
        assert!(store
            .move_studio_intents_to_recovery(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                expected,
                &ids,
                &ManualClock::new(100),
                &mut rng(),
                &mut b
            )
            .is_err());
        assert_eq!(f.intents(&store), 1);
    }
    let (receipt, seed) = super::adoption::checkpoint(&f, &store, 0, 8);
    let (_, state) = super::adoption::adopt(&f, &mut store, &receipt, Some(seed.bytes()));
    // A valid ledger-only failed edit has no recovery envelope and must stay pending.
    let late = f.domain(
        FlipnoteOp::SetHeader(FlipnoteHeader::Title("unsaved".into()))
            .encode()
            .unwrap(),
        99,
    );
    let mut b = budget(&mut store, &f);
    store
        .prepare_epoch_intent(
            SERVER,
            &f.logical,
            late.clone(),
            &f.device,
            &f.group,
            &mut rng(),
            &mut b.storage,
            &mut b.intents,
        )
        .unwrap();
    let mut b = budget(&mut store, &f);
    assert!(store
        .move_studio_intents_to_recovery(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            state.doc_id(),
            &BTreeSet::from([late.id(&f.device.device_id())]),
            &ManualClock::new(100),
            &mut rng(),
            &mut b
        )
        .is_err());
    assert_eq!(f.intents(&store), 2);
}
