use super::*;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

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
