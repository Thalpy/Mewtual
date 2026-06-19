//! The full stack over **real libp2p** (Phase 6e): a fresh device joins a founded
//! server over a libp2p connection and converges on a channel — exercising the MLS
//! join handshake and encrypted CRDT catch-up over `MeshService` instead of the
//! in-memory hub. Uses the libp2p memory transport (real swarms, real Noise +
//! request/response, no OS sockets) so it is reliable in CI; the same code runs
//! over TCP unchanged.

use std::sync::Arc;
use std::time::Duration;

use automerge::transaction::Transactable;
use automerge::{ReadDoc, ROOT};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_net::MeshService;
use catcoms_rt::{MeshTransport, OsCryptoRng, SystemClock, TransportEvent};
use catcoms_sync::ChannelSync;
use catcoms_wire::DocType;
use libp2p::Multiaddr;
use tokio::time::timeout;

const CHANNEL: u128 = 1;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_device_joins_and_converges_over_libp2p() {
    catcoms_log::init_test();

    // --- Alice: a listening server node ---
    let addr: Multiaddr = "/memory/424242".parse().unwrap();
    let a_mesh = MeshService::new_memory(Some(addr.clone()), &[]).unwrap();
    let a_peer = a_mesh.local_peer();

    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let mut asy = ChannelSync::new(
        a_mesh,
        alice_group,
        alice,
        OsCryptoRng,
        Box::new(SystemClock),
    );
    asy.subscribe_control().await.unwrap();
    asy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    // Alice writes history before anyone joins.
    asy.post(DocType::Channel, CHANNEL, |d| {
        d.put(ROOT, "greeting", "welcome over libp2p")
    })
    .await
    .unwrap();
    let invite = asy.mint_invite([5u8; 16], u64::MAX, vec![]).unwrap();

    // Alice serves indefinitely in the background (join + catch-up requests).
    let alice_loop = tokio::spawn(async move { while asy.run_once().await.unwrap_or(false) {} });

    // --- Bob: dials Alice and joins over the wire ---
    let b_mesh = Arc::new(MeshService::new_memory(None, std::slice::from_ref(&addr)).unwrap());

    // Wait for the libp2p connection to Alice to come up before requesting.
    timeout(Duration::from_secs(20), async {
        loop {
            if let Some(TransportEvent::PeerConnected(_)) = b_mesh.next_event().await {
                break;
            }
        }
    })
    .await
    .expect("Bob did not connect to Alice");

    let bob = MlsDevice::generate().unwrap();
    let bob_group = timeout(
        Duration::from_secs(20),
        catcoms_sync::request_join(&*b_mesh, a_peer, &bob, &invite),
    )
    .await
    .expect("join timed out")
    .expect("Bob joined the server over libp2p");
    assert_eq!(bob_group.epoch(), 1);
    assert!(bob_group.contains_device(&bob.device_id()));

    // Bob is now a member; build his sync node over the same connection and catch up
    // Alice's channel history (request/response — no gossip-mesh formation needed).
    let b_mesh = Arc::try_unwrap(b_mesh).expect("sole owner");
    let mut bsy = ChannelSync::new(b_mesh, bob_group, bob, OsCryptoRng, Box::new(SystemClock));
    bsy.open_channel(DocType::Channel, CHANNEL).await.unwrap();

    let applied = timeout(
        Duration::from_secs(20),
        bsy.request_catchup(a_peer, DocType::Channel, CHANNEL),
    )
    .await
    .expect("catch-up timed out")
    .expect("catch-up succeeded");
    assert_eq!(applied, 1, "Bob applied Alice's one history op");

    let doc = bsy.doc(DocType::Channel, CHANNEL).unwrap().doc();
    let greeting = doc
        .get(ROOT, "greeting")
        .unwrap()
        .map(|(v, _)| v.into_string().unwrap());
    assert_eq!(greeting.as_deref(), Some("welcome over libp2p"));

    alice_loop.abort();
}
