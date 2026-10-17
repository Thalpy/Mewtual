//! Gate 7 carrier acceptance: two actual libp2p clients, sealed vaults, cold Studio catch-up,
//! real content-addressed pixels/packages and durable reopen. This is not desktop UI/audio QA.
use std::{sync::Arc, time::Duration};

use catcoms_app::{
    channel_id, spawn,
    store::ServerStore,
    studio::{types::*, StudioRequest, StudioVaultLease, StudioView},
    AppEvent, Server, ServerActor,
};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_net::MeshService;
use catcoms_rt::{Clock, MeshTransport, OsCryptoRng, SystemClock, TransportEvent};
use catcoms_storage::Cid;
use libp2p::Multiaddr;
use tokio::{sync::Mutex, time::timeout};

const SERVER: u64 = 707;
const WAIT: Duration = Duration::from_secs(60);
type Vault = Arc<Mutex<Option<ServerStore>>>;

fn art() -> StudioTarget {
    StudioTarget::Flipnote {
        channel: channel_id("general").to_be_bytes(),
        object: [7; 16],
    }
}
fn score() -> StudioTarget {
    StudioTarget::Score {
        channel: art().channel(),
        object: [8; 16],
    }
}
async fn transaction(
    actor: &ServerActor,
    vault: &Vault,
    request: StudioRequest,
) -> Option<StudioView> {
    actor
        .studio_begin(request)
        .await
        .unwrap()
        .execute(StudioVaultLease::new(
            vault.clone().try_lock_owned().unwrap(),
            SERVER,
            (),
        ))
        .await
        .unwrap()
}
async fn idle(actor: &ServerActor, vault: &Vault) {
    actor
        .studio_receive_begin()
        .await
        .unwrap()
        .execute(StudioVaultLease::new(
            vault.clone().try_lock_owned().unwrap(),
            SERVER,
            (),
        ))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gate7_tcp_cold_join_fetches_art_score_and_export_then_reopens_sealed_vault() {
    timeout(WAIT, lifecycle())
        .await
        .expect("bounded Studio TCP lifecycle");
}

async fn lifecycle() {
    let a_root = tempfile::tempdir().unwrap();
    let b_root = tempfile::tempdir().unwrap();
    let a_store = ServerStore::open(a_root.path(), b"gate7-a", &mut OsCryptoRng).unwrap();
    let b_store = ServerStore::open(b_root.path(), b"gate7-b", &mut OsCryptoRng).unwrap();
    // Actual sockets and blocking parser tasks need actual elapsed deadlines. Advancing a
    // manual second every few milliseconds would expire healthy work on a loaded CI host.
    let clock = SystemClock;
    let listen: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    let (mesh, _, _) = MeshService::new_tcp_with_key(
        libp2p::identity::Keypair::ed25519_from_bytes([71; 32]).unwrap(),
        std::slice::from_ref(&listen),
        &[],
    )
    .unwrap();
    let address = mesh.next_listen_addr().await.unwrap();
    let peer = mesh.local_peer();
    let mut node = Server::found(
        mesh,
        MlsDevice::generate().unwrap(),
        OsCryptoRng,
        Box::new(clock),
        "Alice",
    )
    .unwrap();
    node.subscribe_control().await.unwrap();
    node.set_blob_store(a_store.blob_store(&hex::encode(node.group_id())).unwrap());
    let a_vault = Arc::new(Mutex::new(Some(a_store)));
    let (alice, mut events, a_task) = spawn(node);
    let a_drain = tokio::spawn(async move {
        while let Some(e) = events.recv().await {
            assert!(!matches!(e.event, AppEvent::StudioReceivePaused));
        }
    });
    alice
        .publish_self_record(vec![address.to_string()], 65_536)
        .await;
    // All content predates Bob's admission and subscriptions. He must discover it over the
    // production page protocol; live gossip or a test-installed source cannot supply it.
    let made = transaction(
        &alice,
        &a_vault,
        StudioRequest::Create {
            channel: art().channel(),
            object: [7; 16],
            nonce: [1; 16],
            title: "TCP animation".into(),
            ts: 1000,
        },
    )
    .await
    .unwrap();
    let music = transaction(
        &alice,
        &a_vault,
        StudioRequest::CreateScore {
            channel: art().channel(),
            object: [8; 16],
            nonce: [2; 16],
            title: "TCP score".into(),
            ts: 1000,
        },
    )
    .await
    .unwrap();
    transaction(
        &alice,
        &a_vault,
        StudioRequest::Apply {
            target: score(),
            epoch_id: music.epoch_id,
            nonce: [3; 16],
            body: ScoreOp::SetCell {
                step: 15,
                row: 24,
                value: ScoreCell::Drum { pad: 9 },
            }
            .encode()
            .unwrap(),
        },
    )
    .await
    .unwrap();
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../catcoms-replication/tests/fixtures/studio-export-v1.json"
    ))
    .unwrap();
    let package = hex::decode(vectors["pixaHex"].as_str().unwrap()).unwrap();
    let pixels = catcoms_app::creative::validate_pixa(&package)
        .unwrap()
        .frames()[0]
        .to_vec();
    let pixel_cid =
        Cid::from_hex(&alice.publish_pix(pixels.clone(), None).await.unwrap().cid).unwrap();
    let package_cid =
        Cid::from_hex(&alice.publish_pixa(package.clone(), None).await.unwrap().cid).unwrap();
    for (nonce, operation) in [
        (
            4,
            FlipnoteOp::InsertFrame {
                frame: [9; 16],
                after: None,
                cid: *pixel_cid.as_bytes(),
                bytes: pixels.len() as u64,
            },
        ),
        (
            5,
            FlipnoteOp::SetHeader(FlipnoteHeader::Score(Some([8; 16]))),
        ),
        (
            6,
            FlipnoteOp::SetExport {
                export: [10; 16],
                cid: *package_cid.as_bytes(),
                bytes: package.len() as u64,
                expiry: StudioExpiry::Never,
            },
        ),
    ] {
        transaction(
            &alice,
            &a_vault,
            StudioRequest::Apply {
                target: art(),
                epoch_id: made.epoch_id,
                nonce: [nonce; 16],
                body: operation.encode().unwrap(),
            },
        )
        .await
        .unwrap();
    }
    let expected_art = transaction(&alice, &a_vault, StudioRequest::Read { target: art() })
        .await
        .unwrap();
    let expected_score = transaction(&alice, &a_vault, StudioRequest::Read { target: score() })
        .await
        .unwrap();
    let index = StudioTarget::Index {
        channel: art().channel(),
    };
    let expected_index = transaction(&alice, &a_vault, StudioRequest::Read { target: index })
        .await
        .unwrap();
    let token = alice
        .mint_invite([9; 16], u64::MAX, vec![address.to_string()])
        .await
        .unwrap();
    let (mesh, _, _) = MeshService::new_tcp_with_key(
        libp2p::identity::Keypair::ed25519_from_bytes([72; 32]).unwrap(),
        std::slice::from_ref(&listen),
        std::slice::from_ref(&address),
    )
    .unwrap();
    loop {
        match mesh.next_event().await {
            Some(TransportEvent::PeerConnected(p)) if p == peer => break,
            Some(_) => {}
            None => panic!("TCP joiner closed before connecting"),
        }
    }
    let mut node = Server::join(
        mesh,
        MlsDevice::generate().unwrap(),
        OsCryptoRng,
        Box::new(clock),
        "Bob",
        peer,
        &InviteToken::decode(&token).unwrap(),
    )
    .await
    .unwrap();
    node.request_channel_index_catchup(peer).await.unwrap();
    node.request_pex(peer).await.unwrap();
    node.set_blob_store(b_store.blob_store(&hex::encode(node.group_id())).unwrap());
    let group = node.group_id();
    assert!(!b_store
        .blob_store(&hex::encode(&group))
        .unwrap()
        .has(&pixel_cid));
    assert!(!b_store
        .blob_store(&hex::encode(&group))
        .unwrap()
        .has(&package_cid));
    let b_vault = Arc::new(Mutex::new(Some(b_store)));
    let (bob, mut events, b_task) = spawn(node);
    let (updates, mut received) = tokio::sync::mpsc::channel(64);
    let b_drain = tokio::spawn(async move {
        while let Some(e) = events.recv().await {
            assert!(!matches!(e.event, AppEvent::StudioReceivePaused));
            if matches!(e.event, AppEvent::StudioUpdated { .. }) {
                updates.try_send(()).expect("bounded acceptance event rail");
            }
        }
    });
    // Bob's authenticated request teaches Alice the reverse transport binding too.
    bob.publish_self_record(Vec::new(), 65_536).await;
    for target in [index, art(), score()] {
        transaction(&bob, &b_vault, StudioRequest::Read { target }).await;
    }
    loop {
        clock.sleep(Duration::from_millis(100)).await;
        tokio::join!(idle(&alice, &a_vault), idle(&bob, &b_vault));
        // Inspect after actual repaint edges, without repeatedly re-opening targets before
        // their catch-up jobs finish. The enclosing deadline bounds the entire lifecycle.
        if timeout(Duration::from_millis(20), received.recv())
            .await
            .is_err()
        {
            continue;
        }
        let a = transaction(&bob, &b_vault, StudioRequest::Read { target: art() }).await;
        let s = transaction(&bob, &b_vault, StudioRequest::Read { target: score() }).await;
        let i = transaction(&bob, &b_vault, StudioRequest::Read { target: index }).await;
        if a.is_some_and(|v| v.projection == expected_art.projection)
            && s.is_some_and(|v| v.projection == expected_score.projection)
            && i.is_some_and(|v| v.projection == expected_index.projection)
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    {
        let mut vault = b_vault.lock().await;
        let blobs = vault
            .as_mut()
            .unwrap()
            .blob_store(&hex::encode(&group))
            .unwrap();
        assert!(
            !blobs.has(&pixel_cid) && !blobs.has(&package_cid),
            "shared document references do not themselves fetch media"
        );
    }
    assert_eq!(
        bob.request_blob_bounded(pixel_cid, pixels.len(), None)
            .await
            .unwrap(),
        Some(pixels.clone())
    );
    assert_eq!(
        bob.request_blob_bounded(package_cid, package.len(), None)
            .await
            .unwrap(),
        Some(package.clone())
    );
    assert!(
        bob.request_blob_bounded(package_cid, package.len() - 1, None)
            .await
            .is_err(),
        "a cached blob cannot evade the caller's size bound"
    );
    // Both clients stop before a cold vault and actor reopen. This checks persistence without
    // accidentally fetching missing state from a live provider during the final readback.
    bob.shutdown().await;
    alice.shutdown().await;
    b_task.await.unwrap();
    a_task.await.unwrap();
    b_drain.await.unwrap();
    a_drain.await.unwrap();
    drop(bob);
    drop(alice);
    drop(b_vault.lock().await.take());
    let held = ServerStore::open(b_root.path(), b"gate7-b", &mut OsCryptoRng).unwrap();
    // Bob performed no local authoring. Require the RECEIVE path's persisted membership/device
    // state, with no live snapshot or explicit pre-shutdown save covering a missing barrier.
    let snapshot = held
        .load_server(SERVER)
        .expect("receive-only peer snapshot must survive on disk");
    let network = catcoms_rt::Hub::new().join(catcoms_rt::PeerId::from_u64(73));
    let mut restored =
        Server::restore(&snapshot, network, OsCryptoRng, Box::new(clock), "Bob").unwrap();
    restored.set_blob_store(held.blob_store(&hex::encode(&group)).unwrap());
    let vault = Arc::new(Mutex::new(Some(held)));
    let (restored, mut events, task) = spawn(restored);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    for (target, expected) in [
        (index, expected_index),
        (art(), expected_art),
        (score(), expected_score),
    ] {
        let read = transaction(&restored, &vault, StudioRequest::Read { target })
            .await
            .unwrap();
        assert_eq!(read.projection, expected.projection);
    }
    assert_eq!(
        restored
            .request_blob_bounded(pixel_cid, pixels.len(), None)
            .await
            .unwrap(),
        Some(pixels)
    );
    assert_eq!(
        restored
            .request_blob_bounded(package_cid, package.len(), None)
            .await
            .unwrap(),
        Some(package)
    );
    assert_eq!(
        vault
            .lock()
            .await
            .as_mut()
            .unwrap()
            .creative_pinned_cids()
            .unwrap()
            .for_group(&group)
            .count(),
        2
    );
    restored.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}
