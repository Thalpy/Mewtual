use super::super::adoption::{AdoptionSync, AdoptionWrite};
use super::*;
use catcoms_replication::studio::StudioRecovery;
use catcoms_replication::{CheckpointSeed, RecoveryTransition};
use catcoms_rt::ManualClock;

pub(super) fn checkpoint(
    f: &Fixture,
    store: &ServerStore,
    epoch: u64,
    salt: u8,
) -> (Receipt, CheckpointSeed) {
    let mut projection = f.load(store).unwrap().projection().unwrap();
    match &mut projection {
        StudioProjection::Index(p) => p.epoch = epoch,
        StudioProjection::Flipnote(p) => p.epoch = epoch,
    }
    let seed = projection.checkpoint([salt; 32]).unwrap();
    let receipt = Receipt::sign(
        f.logical.clone(),
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
pub(super) fn adopt(
    f: &Fixture,
    store: &mut ServerStore,
    receipt: &Receipt,
    seed: Option<&[u8]>,
) -> (StudioAdoptionOutcome, EpochStudioState) {
    let mut b = budget(store, f);
    store
        .adopt_studio_checkpoint(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            receipt,
            seed,
            0,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
        )
        .unwrap()
}

#[test]
fn studio_adoption_store_above_registry_ceiling_survives_source_only_restart() {
    for art in [false, true] {
        for epoch in [4096, 4097] {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let f = Fixture::new(art);
            let mut b = budget(&mut store, &f);
            f.edit(&mut store, &mut b, f.insert());
            let (receipt, seed) = checkpoint(&f, &store, epoch, 7);
            assert_eq!(
                adopt(&f, &mut store, &receipt, None).0,
                StudioAdoptionOutcome::AwaitingSeed
            );
            drop(store);
            store = open(root.path());
            assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Closing);
            let (outcome, state) = adopt(&f, &mut store, &receipt, Some(seed.bytes()));
            assert_eq!(outcome, StudioAdoptionOutcome::Installed);
            assert_eq!(state.epoch(), epoch + 1);
            drop(store);
            assert_eq!(f.load(&open(root.path())).unwrap().epoch(), epoch + 1);
        }
    }
}

#[test]
fn studio_adoption_store_recovery_is_durable_before_replacement_and_intents_survive() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let f = Fixture::new(art);
        let mut b = budget(&mut store, &f);
        f.edit(&mut store, &mut b, f.insert());
        let before = f.load(&store).unwrap().projection().unwrap();
        let (receipt, seed) = checkpoint(&f, &store, 10, 10);
        let (outcome, state) = adopt(&f, &mut store, &receipt, None);
        assert_eq!(outcome, StudioAdoptionOutcome::AwaitingSeed);
        assert_eq!(
            (state.epoch(), state.phase(), state.op_count()),
            (0, EpochPhase::Closing, 1)
        );
        drop(store);
        store = open(root.path());
        assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Closing);
        let (outcome, state) = adopt(&f, &mut store, &receipt, Some(seed.bytes()));
        assert_eq!(outcome, StudioAdoptionOutcome::Installed);
        assert_eq!(
            (state.epoch(), state.phase(), state.op_count()),
            (11, EpochPhase::Open, 0)
        );
        let held = store.load_epoch_recovery(SERVER, &f.logical).unwrap();
        assert_eq!(held.retained().len(), 1);
        let recovery = StudioRecovery::from_snapshot(
            held.retained().next().unwrap(),
            &f.logical,
            f.target.channel(),
        )
        .unwrap();
        assert_eq!(recovery.projection(), &before);
        assert_eq!(recovery.operations().len(), 1);
        assert_eq!(
            f.intents(&store),
            1,
            "a matching seed value cannot retire a local intent"
        );
        drop(store);
        store = open(root.path());
        assert_eq!(f.load(&store).unwrap().epoch(), 11);
        assert_eq!(
            adopt(&f, &mut store, &receipt, Some(seed.bytes())).0,
            StudioAdoptionOutcome::AlreadyInstalled
        );
        assert_eq!(
            store
                .load_epoch_recovery(SERVER, &f.logical)
                .unwrap()
                .retained()
                .len(),
            1
        );
        if art {
            let refs = store.creative_pinned_cids().unwrap();
            assert!(refs
                .for_group(&f.group.group_id())
                .any(|cid| *cid == catcoms_storage::Cid::from_bytes([3; 32])));
        }
    }
}

#[test]
fn studio_adoption_store_all_write_boundaries_preserve_recovery_and_exact_retry() {
    for art in [false, true] {
        for boundary in [
            AdoptionWrite::Source,
            AdoptionWrite::Recovery,
            AdoptionWrite::Successor,
        ] {
            for after_write in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let mut store = open(root.path());
                let mut f = Fixture::new(art);
                let mut b = budget(&mut store, &f);
                f.edit(&mut store, &mut b, f.insert());
                let (receipt, seed) = checkpoint(&f, &store, 4, 4);
                let mut b = budget(&mut store, &f);
                let mut hit = false;
                assert!(store
                    .adopt_studio_checkpoint_with_io(
                        SERVER,
                        &f.group,
                        f.target,
                        &f.device,
                        &receipt,
                        Some(seed.bytes()),
                        0,
                        &ManualClock::new(1000),
                        &mut rng(),
                        &mut b,
                        &mut |phase, path, bytes| {
                            if phase == boundary {
                                hit = true;
                                if after_write {
                                    atomic_write(path, bytes)?;
                                }
                                return Err(AppError::Io("injected settlement boundary".into()));
                            }
                            atomic_write(path, bytes)
                        },
                        &mut |_, path, bytes| sync_studio(path, bytes)
                    )
                    .is_err());
                assert!(hit);
                assert!(b.requires_reconciliation());
                drop(store);
                store = open(root.path());
                let state = f.load(&store).unwrap();
                let installed = boundary == AdoptionWrite::Successor && after_write;
                if installed {
                    assert_eq!(state.epoch(), 5);
                    assert_eq!(
                        store
                            .load_epoch_recovery(SERVER, &f.logical)
                            .unwrap()
                            .retained()
                            .len(),
                        1
                    );
                    f.id = state.doc_id();
                    let mut b = budget(&mut store, &f);
                    f.edit(&mut store, &mut b, f.title());
                } else {
                    assert_eq!(state.epoch(), 0);
                    assert_eq!(state.op_count(), 1);
                }
                let (outcome, state) = adopt(&f, &mut store, &receipt, Some(seed.bytes()));
                assert_eq!(
                    outcome,
                    if installed {
                        StudioAdoptionOutcome::AlreadyInstalled
                    } else {
                        StudioAdoptionOutcome::Installed
                    }
                );
                assert_eq!(state.op_count(), usize::from(installed));
                assert_eq!(f.intents(&store), if installed { 2 } else { 1 });
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
fn studio_adoption_store_failed_closing_flush_stops_before_recovery_or_seed() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, f.insert());
    let (receipt, seed) = checkpoint(&f, &store, 8, 8);
    adopt(&f, &mut store, &receipt, None);
    let mut b = budget(&mut store, &f);
    let mut flushed = false;
    assert!(store
        .adopt_studio_checkpoint_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &receipt,
            Some(seed.bytes()),
            0,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
            &mut |_, _, _| panic!("unchanged source must flush before any further write"),
            &mut |phase, _, _| {
                assert_eq!(phase, AdoptionSync::Source);
                flushed = true;
                Err(AppError::Io("flush failed".into()))
            }
        )
        .is_err());
    assert!(flushed);
    assert!(b.requires_reconciliation());
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Closing);
    assert_eq!(
        store
            .load_epoch_recovery(SERVER, &f.logical)
            .unwrap()
            .retained()
            .len(),
        0
    );
    assert_eq!(
        adopt(&f, &mut store, &receipt, Some(seed.bytes())).0,
        StudioAdoptionOutcome::Installed
    );
}

#[test]
fn studio_adoption_store_third_snapshot_warning_survives_restart_and_holds_source() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, f.insert());
    for epoch in [2, 4] {
        let (receipt, seed) = checkpoint(&f, &store, epoch, epoch as u8);
        assert_eq!(
            adopt(&f, &mut store, &receipt, Some(seed.bytes())).0,
            StudioAdoptionOutcome::Installed
        );
    }
    let (receipt, seed) = checkpoint(&f, &store, 6, 6);
    assert_eq!(
        adopt(&f, &mut store, &receipt, Some(seed.bytes())).0,
        StudioAdoptionOutcome::RecoveryPending
    );
    let warning = store
        .load_epoch_recovery(SERVER, &f.logical)
        .unwrap()
        .eviction_pending()
        .unwrap()
        .unwrap();
    drop(store);
    store = open(root.path());
    assert_eq!(f.load(&store).unwrap().epoch(), 5);
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Closing);
    assert_eq!(
        adopt(&f, &mut store, &receipt, Some(seed.bytes())).0,
        StudioAdoptionOutcome::RecoveryPending
    );
    let held = store.load_epoch_recovery(SERVER, &f.logical).unwrap();
    assert_eq!(held.retained().len(), 2);
    assert!(held.staged().is_some());
    assert_eq!(held.eviction_pending().unwrap(), Some(warning));
    let RecoveryTransition::EvictionPending {
        oldest_snapshot,
        staged_snapshot,
        ..
    } = warning
    else {
        panic!("warning");
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
            &ManualClock::new(1000),
            &mut rng(),
            &mut b.storage,
        )
        .unwrap();
    assert_eq!(
        adopt(&f, &mut store, &receipt, Some(seed.bytes())).0,
        StudioAdoptionOutcome::Installed
    );
    let held = store.load_epoch_recovery(SERVER, &f.logical).unwrap();
    assert_eq!(held.retained().len(), 2);
    assert!(held.staged().is_none());
    assert_eq!(f.intents(&store), 1);
}
