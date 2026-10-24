use super::*;
use crate::{spawn, ServerActor};
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::Arc;
use tokio::sync::Mutex;

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(79)
}
fn channel() -> [u8; 16] {
    crate::channel_id("general").to_be_bytes()
}
fn target() -> StudioTarget {
    StudioTarget::Flipnote {
        channel: channel(),
        object: [7; 16],
    }
}
fn create() -> StudioRequest {
    StudioRequest::Create {
        channel: channel(),
        object: [7; 16],
        nonce: [9; 16],
        title: "moon cat".into(),
        ts: 123,
    }
}
fn read() -> StudioRequest {
    StudioRequest::Read { target: target() }
}
async fn run(
    actor: &ServerActor,
    store: &Arc<Mutex<Option<ServerStore>>>,
    req: StudioRequest,
) -> Result<Option<StudioView>, String> {
    let ready = actor.studio_begin(req).await?;
    let lease = StudioVaultLease::new(store.clone().try_lock_owned().unwrap(), 7, ());
    ready.execute(lease).await
}

#[tokio::test]
async fn studio_actor_create_real_pixels_restart_and_retry() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Some(
        ServerStore::open(dir.path(), b"studio-test", &mut rng()).unwrap(),
    )));
    let hub = Hub::new();
    let mut server = Server::found(
        hub.join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(123)),
        "alice",
    )
    .unwrap();
    let group = server.group_id();
    server.set_blob_store(
        store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .blob_store(&hex::encode(&group))
            .unwrap(),
    );
    let (actor, mut events, task) = spawn(server);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    assert!(run(&actor, &store, read()).await.unwrap().is_none());
    let first = run(&actor, &store, create()).await.unwrap().unwrap();
    let mut pix = crate::creative::tests::golden()[..23].to_vec();
    pix[4] = 191;
    pix[5] = 143;
    pix.extend((0..108).flat_map(|_| [255, 0]));
    let published = actor.publish_pix(pix.clone(), None).await.unwrap();
    let body = FlipnoteOp::InsertFrame {
        frame: [1; 16],
        after: None,
        cid: *crate::Cid::from_hex(&published.cid).unwrap().as_bytes(),
        bytes: pix.len() as u64,
    }
    .encode()
    .unwrap();
    let edit = || StudioRequest::Apply {
        target: target(),
        epoch_id: first.epoch_id,
        nonce: [8; 16],
        body: body.clone(),
    };
    let saved = run(&actor, &store, edit()).await.unwrap().unwrap();
    let list = run(
        &actor,
        &store,
        StudioRequest::Read {
            target: StudioTarget::Index { channel: channel() },
        },
    )
    .await
    .unwrap()
    .unwrap();
    let StudioProjection::Index(index) = list.projection else {
        panic!()
    };
    assert_eq!(index.objects.len(), 1);
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    drop(store.lock().await.take());
    *store.lock().await = Some(ServerStore::open(dir.path(), b"studio-test", &mut rng()).unwrap());
    // The Studio command itself must have saved this current MLS/device snapshot, not the test.
    let snapshot = store.lock().await.as_ref().unwrap().load_server(7).unwrap();
    let mut server = Server::restore(
        &snapshot,
        hub.join(PeerId::from_u64(1)),
        rng(),
        Box::new(ManualClock::new(999)),
        "alice",
    )
    .unwrap();
    server.set_blob_store(
        store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .blob_store(&hex::encode(&group))
            .unwrap(),
    );
    let (actor, mut events, task) = spawn(server);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    assert_eq!(
        run(&actor, &store, read())
            .await
            .unwrap()
            .unwrap()
            .projection,
        saved.projection
    );
    assert_eq!(
        run(&actor, &store, edit())
            .await
            .unwrap()
            .unwrap()
            .projection,
        saved.projection
    );
    assert_eq!(
        run(&actor, &store, create())
            .await
            .unwrap()
            .unwrap()
            .projection,
        saved.projection
    );
    assert_eq!(
        actor
            .request_blob_bounded(
                crate::Cid::from_hex(&published.cid).unwrap(),
                pix.len(),
                None
            )
            .await
            .unwrap()
            .unwrap(),
        pix
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn studio_actor_ready_drop_and_held_vault_do_not_strand_actor() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Some(
        ServerStore::open(dir.path(), b"studio-test", &mut rng()).unwrap(),
    )));
    let clock = ManualClock::new(1);
    let server = Server::found(
        Hub::new().join(PeerId::from_u64(2)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(clock.clone()),
        "alice",
    )
    .unwrap();
    let (actor, mut events, task) = spawn(server);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    // A native task already holding the store can reach Ready without lending it to the queue.
    let held = store.clone().lock_owned().await;
    let ready = actor.studio_begin(create()).await.unwrap();
    assert!(store.clone().try_lock_owned().is_err());
    drop(ready);
    assert_eq!(actor.member_count().await, 1);
    drop(held);
    let ready = actor.studio_begin(create()).await.unwrap();
    clock.advance_ms(6_000);
    assert_eq!(actor.member_count().await, 1);
    assert!(ready
        .execute(StudioVaultLease::new(
            store.clone().lock_owned().await,
            7,
            ()
        ))
        .await
        .is_err());
    assert!(run(&actor, &store, read()).await.unwrap().is_none());
    let ready = actor.studio_begin(create()).await.unwrap();
    clock.advance_ms(5_000);
    // No yield: lease and expiry are both ready for the actor's biased select.
    assert!(ready
        .execute(StudioVaultLease::new(
            store.clone().try_lock_owned().unwrap(),
            7,
            ()
        ))
        .await
        .is_err());
    assert!(run(&actor, &store, read()).await.unwrap().is_none());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn studio_create_partial_failure_retries_same_object_after_index_space_is_freed() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(dir.path(), b"studio-test", &mut rng()).unwrap();
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(123)),
        "alice",
    )
    .unwrap();
    let index = StudioTarget::Index { channel: channel() };
    let epoch_id = epoch_zero_id(catcoms_wire::DocType::StudioIndex, &channel());
    for n in 1..=64u8 {
        server
            .studio_transaction(
                &mut store,
                7,
                StudioRequest::Apply {
                    target: index,
                    epoch_id,
                    nonce: [n + 100; 16],
                    body: IndexOp::PutObject {
                        object: [n + 20; 16],
                        kind: StudioKind::Flipnote,
                        title: "old".into(),
                        created_by: server.device_id(),
                        ts: 1,
                        expiry: StudioExpiry::Never,
                    }
                    .encode()
                    .unwrap(),
                },
            )
            .unwrap();
    }
    assert!(server.studio_transaction(&mut store, 7, create()).is_err());
    let unlisted = server
        .studio_transaction(&mut store, 7, read())
        .unwrap()
        .unwrap();
    server
        .studio_transaction(
            &mut store,
            7,
            StudioRequest::Apply {
                target: index,
                epoch_id,
                nonce: [88; 16],
                body: IndexOp::TombstoneObject { object: [21; 16] }
                    .encode()
                    .unwrap(),
            },
        )
        .unwrap();
    let completed = server
        .studio_transaction(&mut store, 7, create())
        .unwrap()
        .unwrap();
    assert_eq!(unlisted.projection, completed.projection);
    let state = server.sync.with_registry_context(|group, device, _, _| {
        store
            .load_studio_epoch(7, group, target(), device)
            .unwrap()
            .unwrap()
    });
    assert_eq!(
        state.op_count(),
        1,
        "retry must not author the object twice"
    );
}

#[tokio::test]
async fn studio_refuses_missing_or_wrong_pixels_unknown_channels_and_failed_server_save() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(dir.path(), b"studio-test", &mut rng()).unwrap();
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(123)),
        "alice",
    )
    .unwrap();
    let unknown = StudioRequest::Read {
        target: StudioTarget::Index { channel: [255; 16] },
    };
    assert!(server.studio_transaction(&mut store, 7, unknown).is_err());
    let request = |cid, bytes| StudioRequest::Apply {
        target: target(),
        epoch_id: epoch_zero_id(catcoms_wire::DocType::StudioObject, &[7; 16]),
        nonce: [8; 16],
        body: FlipnoteOp::InsertFrame {
            frame: [1; 16],
            after: None,
            cid,
            bytes,
        }
        .encode()
        .unwrap(),
    };
    assert!(server
        .studio_transaction(&mut store, 7, request([0; 32], 10))
        .is_err());
    let pixels = crate::creative::tests::golden();
    let mut blobs = store.blob_store(&hex::encode(server.group_id())).unwrap();
    let cid = blobs.put(&pixels).unwrap();
    assert!(server
        .studio_transaction(&mut store, 7, request(*cid.as_bytes(), pixels.len() as u64))
        .is_err());
    assert!(server
        .studio_transaction(&mut store, 7, read())
        .unwrap()
        .is_none());
    // The exact server snapshot path is a directory: failure must precede any Studio intent.
    std::fs::create_dir(dir.path().join("servers").join("7.bin")).unwrap();
    assert!(server.studio_transaction(&mut store, 7, create()).is_err());
    assert!(server
        .studio_transaction(&mut store, 7, read())
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn studio_create_cannot_overwrite_existing_with_a_new_nonce() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(dir.path(), b"studio-test", &mut rng()).unwrap();
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(123)),
        "alice",
    )
    .unwrap();
    let first = server
        .studio_transaction(&mut store, 7, create())
        .unwrap()
        .unwrap();
    let before = server.sync.with_registry_context(|g, d, _, _| {
        store
            .load_studio_epoch(7, g, target(), d)
            .unwrap()
            .unwrap()
            .op_count()
    });
    let request = StudioRequest::Create {
        channel: channel(),
        object: [7; 16],
        nonce: [10; 16],
        title: "overwrite".into(),
        ts: 123,
    };
    assert!(server.studio_transaction(&mut store, 7, request).is_err());
    assert_eq!(
        server
            .studio_transaction(&mut store, 7, read())
            .unwrap()
            .unwrap()
            .projection,
        first.projection
    );
    let after = server.sync.with_registry_context(|g, d, _, _| {
        assert_eq!(
            store
                .load_epoch_intents(7, &target().document(&g.group_id()).unwrap())
                .unwrap()
                .pending()
                .len(),
            1
        );
        store
            .load_studio_epoch(7, g, target(), d)
            .unwrap()
            .unwrap()
            .op_count()
    });
    assert_eq!(before, after);
    // Retry after an ordinary newer title preserves the newer value instead of overwriting it.
    let renamed = server
        .studio_transaction(
            &mut store,
            7,
            StudioRequest::Apply {
                target: target(),
                epoch_id: first.epoch_id,
                nonce: [11; 16],
                body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("newer".into()))
                    .encode()
                    .unwrap(),
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        server
            .studio_transaction(&mut store, 7, create())
            .unwrap()
            .unwrap()
            .projection,
        renamed.projection
    );
}

#[tokio::test]
async fn studio_context_refuses_before_snapshot_or_blob_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(dir.path(), b"studio-test", &mut rng()).unwrap();
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(123)),
        "alice",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("servers").join("7.bin")).unwrap();
    std::fs::write(dir.path().join("blobs"), b"blocked").unwrap();
    let req = StudioRequest::Apply {
        target: StudioTarget::Index { channel: channel() },
        epoch_id: 0,
        nonce: [1; 16],
        body: IndexOp::PutObject {
            object: [1; 16],
            kind: StudioKind::Flipnote,
            title: "bad author".into(),
            created_by: crate::DeviceId::from_bytes([0; 32]),
            ts: 1,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap(),
    };
    let err = server.studio_transaction(&mut store, 7, req).unwrap_err();
    assert!(
        matches!(err, AppError::Invalid(_)),
        "must reject contextual authority before filesystem error"
    );
    assert!(StudioRequest::Create {
        channel: channel(),
        object: [1; 16],
        nonce: [1; 16],
        title: "x".repeat(65500),
        ts: 1
    }
    .validate()
    .is_err());
}
