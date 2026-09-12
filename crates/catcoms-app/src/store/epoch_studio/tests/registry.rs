use super::*;
use catcoms_replication::registry::{registry_document, PointerKey, RegistryOp};

fn refresh(f: &Fixture, store: &mut ServerStore) -> Option<EpochRegistryState> {
    let mut b = budget(store, f);
    store
        .refresh_studio_registry_pointer(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .unwrap()
}

#[test]
fn studio_registry_explicit_restore_preserves_automatic_tombstone_hold_and_current_delete_wins() {
    use catcoms_replication::registry_epoch::RegistryEpoch;
    use catcoms_rt::ManualClock;
    let f = Fixture::new(true);
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    assert!(store
        .restore_studio_registry_pointer(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .is_err());
    let (_, source) = f.edit(&mut store, &mut b, f.insert());
    store.retain_studio_source(&f.group, &f.device, source);
    let key = PointerKey::new(f.logical.doc_type, f.logical.logical_key.clone()).unwrap();
    let bucket = key.bucket();
    let document = registry_document(&f.group.group_id(), bucket).unwrap();
    // Real typed recovery from a fork containing this pointer's deletion; the actual current
    // Registry below is still empty. Ordinary refresh must not treat that history as absent.
    let mut fork = RegistryEpoch::new(&f.group, bucket, f.device.device_id()).unwrap();
    for (n, op) in [
        (
            1,
            RegistryOp::Put {
                key: key.clone(),
                epoch: 0,
            },
        ),
        (2, RegistryOp::Tombstone { key: key.clone() }),
    ] {
        fork.edit(
            &f.device,
            &f.group,
            &mut rng(),
            &op.domain_op(&f.group.group_id(), [n; 16]).unwrap(),
        )
        .unwrap();
    }
    let seed = fork.projection().unwrap().checkpoint([4; 32]).unwrap();
    let receipt = Receipt::sign(
        document.clone(),
        0,
        [4; 32],
        seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        &f.device,
    )
    .unwrap();
    fork.begin_checkpoint_adoption(receipt.clone(), &f.group, 0)
        .unwrap();
    let recovery = fork
        .prepare_checkpoint_adoption(&receipt, seed.bytes(), &f.group, 0)
        .unwrap()
        .recovery_snapshot()
        .unwrap()
        .clone();
    store
        .update_epoch_recovery(
            SERVER,
            &document,
            super::super::super::EpochRecoveryAction::Stage(recovery),
            &ManualClock::new(100),
            &mut rng(),
        )
        .unwrap();
    assert!(refresh(&f, &mut store).is_none());
    let mut b = budget(&mut store, &f);
    let (epoch, registry_id) = store
        .restore_studio_registry_pointer(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .unwrap();
    assert_eq!(epoch, 0);
    let held = store
        .load_registry_epoch(SERVER, &f.group, bucket, &f.device)
        .unwrap()
        .unwrap();
    assert_eq!(held.projection().unwrap().pointers.get(&key), Some(&0));
    assert_eq!(held.op_count(), 1);
    store
        .restore_studio_registry_pointer(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .unwrap();
    assert_eq!(
        store
            .load_registry_epoch(SERVER, &f.group, bucket, &f.device)
            .unwrap()
            .unwrap()
            .op_count(),
        1
    );
    store
        .edit_registry_epoch(
            SERVER,
            &f.group,
            bucket,
            registry_id,
            &f.device,
            RegistryOp::Tombstone { key: key.clone() }
                .domain_op(&f.group.group_id(), [8; 16])
                .unwrap(),
            &mut rng(),
            &mut b.storage,
            &mut b.intents,
        )
        .unwrap();
    let error = store
        .restore_studio_registry_pointer(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .unwrap_err();
    assert!(error.to_string().contains("tombstoned Registry epoch"));
    drop(store);
    let store = open(root.path());
    assert!(store
        .load_registry_epoch(SERVER, &f.group, bucket, &f.device)
        .unwrap()
        .unwrap()
        .projection()
        .unwrap()
        .tombstones
        .contains(&key));
    assert_eq!(
        store
            .load_epoch_recovery(SERVER, &document)
            .unwrap()
            .retained()
            .len(),
        1
    );
}

#[test]
fn studio_registry_refresh_uses_actual_checkpoint_and_never_duplicates_or_rewinds() {
    for art in [false, true] {
        let f = Fixture::new(art);
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        assert!(
            refresh(&f, &mut store).is_none(),
            "absence must not publish a pointer"
        );
        let key = PointerKey::new(f.logical.doc_type, f.logical.logical_key.clone()).unwrap();
        let bucket = key.bucket();
        let document = registry_document(&f.group.group_id(), bucket).unwrap();
        let mut b = budget(&mut store, &f);
        let (_, state) = f.edit(&mut store, &mut b, f.insert());
        store.retain_studio_source(&f.group, &f.device, state);
        let first = refresh(&f, &mut store).unwrap();
        assert_eq!(first.projection().unwrap().pointers.get(&key), Some(&0));
        assert_eq!(first.op_count(), 1);
        assert!(refresh(&f, &mut store).is_none());
        assert_eq!(
            store
                .load_epoch_intents(SERVER, &document)
                .unwrap()
                .pending()
                .len(),
            1
        );
        // Install an actually receipt-verified successor using the existing adoption fixture.
        // It reuses the original insert nonce, an exact accepted retry, then adopts the seed.
        super::discovery::prepared(&f, &mut store);
        let next = refresh(&f, &mut store).unwrap();
        assert_eq!(next.projection().unwrap().pointers.get(&key), Some(&1));
        assert_eq!(next.op_count(), 2);
        for op in [
            RegistryOp::Put {
                key: key.clone(),
                epoch: 7,
            },
            RegistryOp::Tombstone { key: key.clone() },
        ] {
            let mut b = budget(&mut store, &f);
            store
                .edit_registry_epoch(
                    SERVER,
                    &f.group,
                    bucket,
                    next.doc_id(),
                    &f.device,
                    op.domain_op(
                        &f.group.group_id(),
                        if matches!(op, RegistryOp::Put { .. }) {
                            [61; 16]
                        } else {
                            [62; 16]
                        },
                    )
                    .unwrap(),
                    &mut rng(),
                    &mut b.storage,
                    &mut b.intents,
                )
                .unwrap();
            assert!(
                refresh(&f, &mut store).is_none(),
                "ordinary refresh cannot overwrite a later/deleted hint"
            );
            let mut checked = budget(&mut store, &f);
            assert!(
                store
                    .restore_studio_registry_pointer(
                        SERVER,
                        &f.group,
                        f.target,
                        &f.device,
                        &mut rng(),
                        &mut checked
                    )
                    .is_err(),
                "explicit Restore cannot rewind a newer pointer or override a current tombstone"
            );
        }
        let final_state = store
            .load_registry_epoch(SERVER, &f.group, bucket, &f.device)
            .unwrap()
            .unwrap();
        assert_eq!(final_state.op_count(), 4);
        assert!(final_state.projection().unwrap().tombstones.contains(&key));
    }
}

#[test]
fn studio_registry_pointer_restore_refuses_full_closing_and_fault_buckets() {
    use catcoms_replication::registry_epoch::RegistryEpoch;
    use catcoms_rt::ManualClock;
    let f = Fixture::new(true);
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    let (_, source) = f.edit(&mut store, &mut b, f.insert());
    store.retain_studio_source(&f.group, &f.device, source);
    let key = PointerKey::new(f.logical.doc_type, f.logical.logical_key.clone()).unwrap();
    let bucket = key.bucket();
    let document = registry_document(&f.group.group_id(), bucket).unwrap();
    let mut projection = RegistryEpoch::new(&f.group, bucket, f.device.device_id())
        .unwrap()
        .projection()
        .unwrap();
    let mut n = 0u128;
    while projection.pointers.len() < catcoms_replication::registry::MAX_REGISTRY_POINTERS {
        let candidate = PointerKey::new(
            catcoms_wire::DocType::StudioObject,
            n.to_be_bytes().to_vec(),
        )
        .unwrap();
        n += 1;
        if candidate.bucket() == bucket && candidate != key {
            projection.pointers.insert(candidate, 0);
        }
    }
    let seed = projection.checkpoint([3; 32]).unwrap();
    let receipt = Receipt::sign(
        document.clone(),
        0,
        [3; 32],
        seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        &f.device,
    )
    .unwrap();
    for bytes in [None, Some(seed.bytes())] {
        store
            .adopt_registry_checkpoint(
                SERVER,
                &f.group,
                bucket,
                &f.device,
                &receipt,
                bytes,
                0,
                &ManualClock::new(10),
                &mut rng(),
                &mut b.storage,
            )
            .unwrap();
    }
    let mut b = budget(&mut store, &f);
    assert!(store
        .restore_studio_registry_pointer(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .unwrap_err()
        .to_string()
        .contains("bucket is full"));
    for n in [4, 5] {
        let receipt = Receipt::sign(
            document.clone(),
            1,
            [n; 32],
            [n; 32],
            0,
            InheritedCheckpoint::EpochZero,
            &f.device,
        )
        .unwrap();
        store
            .seal_registry_epoch(
                SERVER,
                &f.group,
                bucket,
                &f.device,
                receipt,
                0,
                &mut rng(),
                &mut b.storage,
            )
            .unwrap();
        b = budget(&mut store, &f);
        assert!(store
            .restore_studio_registry_pointer(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &mut rng(),
                &mut b
            )
            .unwrap_err()
            .to_string()
            .contains("Registry Open"));
        assert_eq!(
            store
                .load_registry_epoch(SERVER, &f.group, bucket, &f.device)
                .unwrap()
                .unwrap()
                .phase(),
            if n == 4 {
                EpochPhase::Closing
            } else {
                EpochPhase::Fault
            }
        );
    }
}
