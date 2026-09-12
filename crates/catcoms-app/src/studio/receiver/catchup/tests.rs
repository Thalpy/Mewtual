use super::*;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[tokio::test]
async fn studio_owner_superseded_preparation_waits_its_local_deadline() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(181);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"pace", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::save_studio_source_fixture(&mut store, 83, g, d, target)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            83,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    // Displace the sole graph without adding another watch. The owner turn must capture A.
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::save_studio_source_fixture(
            &mut store,
            83,
            g,
            d,
            StudioTarget::Flipnote {
                channel: target.channel(),
                object: [8; 16],
            },
        )
    });
    server
        .studio_transaction(
            &mut store,
            83,
            StudioRequest::Read {
                target: StudioTarget::Flipnote {
                    channel: target.channel(),
                    object: [8; 16],
                },
            },
        )
        .unwrap();
    receiver.run(&mut server, &mut store, 83, None).unwrap();
    let work = receiver.detach(&mut server).expect("owner source capture");
    assert!(work.is_preparation_for_test());
    let result = work.run(None).await;
    // Valid local/remote edits can win while the worker owns an older detached snapshot.
    let logical = target.document(&server.group_id()).unwrap();
    server
        .studio_transaction(
            &mut store,
            83,
            StudioRequest::Apply {
                target,
                epoch_id: catcoms_replication::epoch_zero_id(
                    logical.doc_type,
                    &logical.logical_key,
                ),
                nonce: [91; 16],
                body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("newer".into()))
                    .encode()
                    .unwrap(),
            },
        )
        .unwrap();
    receiver.complete(&mut server, result);
    receiver.run(&mut server, &mut store, 83, None).unwrap();
    assert_eq!(receiver.catchup.owner_next_at, 6000);
    for _ in 0..20 {
        assert!(!receiver.pending(&server));
        receiver.run(&mut server, &mut store, 83, None).unwrap();
        assert!(receiver.detach(&mut server).is_none());
    }
    clock.advance_ms(4999);
    assert!(!receiver.pending(&server));
    clock.advance_ms(1);
    assert!(receiver.pending(&server));
    receiver.run(&mut server, &mut store, 83, None).unwrap();
    assert_eq!(receiver.catchup.owner_next_at, 11000);
    assert!(!receiver.take_pause_notice());
}

#[tokio::test]
async fn studio_solo_owner_rotates_repeatedly_on_idle_and_reopens_latest_checkpoint() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(179);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"solo-rotation", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    let logical = target.document(&server.group_id()).unwrap();
    let mut receiver = StudioReceiver::default();
    for epoch in 1..=3 {
        server.sync.with_registry_context(|g, d, _, _| {
            crate::store::fill_studio_epoch_fixture(&mut store, 83, g, d, target)
        });
        if epoch == 1 {
            receiver
                .run(
                    &mut server,
                    &mut store,
                    83,
                    Some(StudioRequest::Read { target }),
                )
                .unwrap();
        }
        clock.advance_ms(5_000);
        let mut progressed = false;
        for _ in 0..12 {
            let (_, update) = receiver.run(&mut server, &mut store, 83, None).unwrap();
            assert!(
                !receiver.take_pause_notice(),
                "idle rotation must not pause receive"
            );
            if let Some(work) = receiver.detach(&mut server) {
                receiver.complete(&mut server, work.run(None).await);
            }
            if update == Some(target) {
                progressed = true;
                break;
            }
        }
        assert!(
            progressed,
            "epoch {epoch}: {:?}",
            receiver.catchup.owner_failure
        );
        let state = server
            .sync
            .with_registry_context(|g, d, _, _| store.load_studio_epoch(83, g, target, d))
            .unwrap()
            .unwrap();
        assert_eq!(state.epoch(), epoch);
        assert_eq!(state.phase(), catcoms_replication::EpochPhase::Open);
        assert_eq!(state.op_count(), 0);
        let journal = store.load_epoch_owner_receipts(83, &logical).unwrap();
        assert!(
            journal.pending().is_none(),
            "availability needs no remote query"
        );
        assert_eq!(receiver.watches.front().unwrap().1, state.doc_id());
        // Crash/reopen loses all volatile preparation, watches and owner permits. An ordinary
        // Read establishes a fresh binding; subsequent rotation must reuse the saved journal.
        drop(store);
        store = ServerStore::open(root.path(), b"solo-rotation", &mut rng).unwrap();
        receiver = StudioReceiver::default();
        receiver
            .run(
                &mut server,
                &mut store,
                83,
                Some(StudioRequest::Read { target }),
            )
            .unwrap();
    }
}

#[test]
fn studio_discovery_superseded_watch_drops_all_scheduled_work_without_rebinding() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(78);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"revoked", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    let old = server.sync.watch_studio(target, 4).unwrap();
    let mut runtime = CatchupRuntime {
        target: Some(target),
        discovery_watch: Some(old.copy_binding()),
        discovery_needed: Some(target),
        ..Default::default()
    };
    runtime.discovery_plan = Some(DiscoveryPlan {
        mount: store.registry_mount(),
        server: 83,
        peer: PeerId::from_u64(2),
        target: CheckpointTarget::Studio(target),
    });
    runtime.after_registry = Some(DiscoveryPlan {
        mount: store.registry_mount(),
        server: 83,
        peer: PeerId::from_u64(2),
        target: CheckpointTarget::Studio(target),
    });
    let newer = server.sync.watch_studio(target, 5).unwrap();
    assert!(runtime
        .advance_checkpoint(&mut server, &mut store, 83)
        .unwrap()
        .is_none());
    assert!(
        runtime.discovery_plan.is_none()
            && runtime.after_registry.is_none()
            && runtime.discovery_needed.is_none()
    );
    assert!(
        runtime.binding.is_none(),
        "no late work may recreate the old UI watch"
    );
    assert!(server.sync.studio_watch_is_current(&newer));
}

#[test]
fn studio_owner_lifecycle_retries_failed_snapshot_on_local_clock_not_request_frequency() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(77);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = ServerStore::open(root.path(), b"retry", &mut rng).unwrap();
    let mut runtime = CatchupRuntime::default();
    runtime.lifecycle_with(&mut server, &store, 83, |_, _, _| {
        Err(invalid("injected source flush failure"))
    });
    assert!(runtime.owner_snapshot.is_none());
    for _ in 0..100 {
        runtime.lifecycle_with(&mut server, &store, 83, |_, _, _| {
            panic!("request frequency must not retry a failed lifecycle save")
        });
    }
    clock.advance_ms(30_000);
    runtime.lifecycle(&mut server, &store, 83);
    assert!(server.owner_head_snapshot_is_current(
        &store,
        83,
        runtime
            .owner_snapshot
            .as_ref()
            .expect("idle recovery without MLS or UI change")
    ));
    clock.advance_ms(30_000);
    runtime.lifecycle_with(&mut server, &store, 83, |_, _, _| {
        panic!("successful current lifecycle must be reused")
    });
}
