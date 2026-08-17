//! Phase 7; the full stack end-to-end over **real OS sockets** (TCP loopback), not the
//! libp2p memory transport. A founder binds an ephemeral `127.0.0.1` port; a fresh
//! device dials it over real TCP, runs the MLS join handshake, catches up the encrypted
//! channel, and both converge; exercising every layer (identity, MLS, invites,
//! encrypted CRDT replication, channel sync) across a genuine socket. The `serve`/`join`
//! CLI runs this same path across separate OS processes; this is the automated,
//! in-CI single-process form over the same TCP transport.

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
async fn fresh_device_joins_and_converges_over_real_tcp() {
    catcoms_log::init_test();

    // --- Alice: a founder listening on an ephemeral TCP loopback port ---
    let (a_mesh, _a_id) =
        MeshService::new_tcp(Some("/ip4/127.0.0.1/tcp/0".parse().unwrap()), &[]).unwrap();
    // Discover the actual bound address (the OS picked the port) before serving.
    let a_addr: Multiaddr = timeout(Duration::from_secs(10), a_mesh.next_listen_addr())
        .await
        .expect("listen-addr timeout")
        .expect("Alice bound a TCP address");
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
    asy.post(DocType::Channel, CHANNEL, |d| {
        d.put(ROOT, "greeting", "welcome over real tcp")
    })
    .await
    .unwrap();
    let invite = asy
        .mint_invite([5u8; 16], u64::MAX, vec![a_addr.to_string()])
        .unwrap();
    let alice_loop = tokio::spawn(async move { while asy.run_once().await.unwrap_or(false) {} });

    // --- Bob: dials Alice's bound TCP address and joins over the wire ---
    let (b_mesh, _b_id) = MeshService::new_tcp(None, std::slice::from_ref(&a_addr)).unwrap();
    let b_mesh = Arc::new(b_mesh);
    timeout(Duration::from_secs(20), async {
        loop {
            if let Some(TransportEvent::PeerConnected(p)) = b_mesh.next_event().await {
                if p == a_peer {
                    break;
                }
            }
        }
    })
    .await
    .expect("Bob did not connect to Alice over TCP");

    let bob = MlsDevice::generate().unwrap();
    let (bob_group, bob_routing) = timeout(
        Duration::from_secs(20),
        catcoms_sync::request_join(&*b_mesh, a_peer, &bob, &invite),
    )
    .await
    .expect("join timed out")
    .expect("Bob joined the server over real TCP");
    assert_eq!(bob_group.epoch(), 1);
    assert!(bob_group.contains_device(&bob.device_id()));

    let b_mesh = Arc::try_unwrap(b_mesh).expect("sole owner");
    let mut bsy = ChannelSync::new_joined(
        b_mesh,
        bob_group,
        bob,
        OsCryptoRng,
        Box::new(SystemClock),
        bob_routing,
    );
    bsy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    let applied = timeout(
        Duration::from_secs(20),
        bsy.request_catchup(a_peer, DocType::Channel, CHANNEL),
    )
    .await
    .expect("catch-up timed out")
    .expect("catch-up succeeded");
    assert_eq!(applied, 1, "Bob applied Alice's one history op over TCP");

    let greeting = bsy
        .doc(DocType::Channel, CHANNEL)
        .unwrap()
        .doc()
        .get(ROOT, "greeting")
        .unwrap()
        .map(|(v, _)| v.into_string().unwrap());
    assert_eq!(greeting.as_deref(), Some("welcome over real tcp"));

    alice_loop.abort();
}
