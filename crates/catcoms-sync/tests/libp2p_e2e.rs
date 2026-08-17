//! The full stack over **real libp2p** (Phase 6e): a fresh device joins a founded
//! server over a libp2p connection and converges on a channel; exercising the MLS
//! join handshake and encrypted CRDT catch-up over `MeshService` instead of the
//! in-memory hub. Uses the libp2p memory transport (real swarms, real Noise +
//! request/response, no OS sockets) so it is reliable in CI; the same code runs
//! over TCP unchanged.

use std::sync::Arc;
use std::time::Duration;

use automerge::transaction::Transactable;
use automerge::{ReadDoc, ROOT};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_net::{build_memory_relay_swarm, run_relay, MeshService};
use catcoms_rt::{MeshTransport, OsCryptoRng, SystemClock, TransportEvent};
use catcoms_sync::ChannelSync;
use catcoms_wire::DocType;
use libp2p::Multiaddr;
use tokio::time::timeout;

const CHANNEL: u128 = 1;

/// Await a `…/p2p-circuit` listen address from a relay reservation.
async fn await_circuit_addr(mesh: &MeshService) -> Multiaddr {
    timeout(Duration::from_secs(15), async {
        loop {
            match mesh.next_listen_addr().await {
                Some(a) if a.to_string().contains("p2p-circuit") => return a,
                Some(_) => continue,
                None => panic!("actor stopped"),
            }
        }
    })
    .await
    .expect("relay reservation not granted")
}

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
    let (bob_group, bob_routing) = timeout(
        Duration::from_secs(20),
        catcoms_sync::request_join(&*b_mesh, a_peer, &bob, &invite),
    )
    .await
    .expect("join timed out")
    .expect("Bob joined the server over libp2p");
    assert_eq!(bob_group.epoch(), 1);
    assert!(bob_group.contains_device(&bob.device_id()));

    // Bob is now a member; build his sync node over the same connection and catch up
    // Alice's channel history (request/response; no gossip-mesh formation needed).
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
    assert_eq!(applied, 1, "Bob applied Alice's one history op");

    let doc = bsy.doc(DocType::Channel, CHANNEL).unwrap().doc();
    let greeting = doc
        .get(ROOT, "greeting")
        .unwrap()
        .map(|(v, _)| v.into_string().unwrap());
    assert_eq!(greeting.as_deref(), Some("welcome over libp2p"));

    alice_loop.abort();
}

/// 6e-3b: a joiner reaches a server it can only contact **through a relay**; the
/// server reserves a circuit slot, advertises that circuit address, and the joiner
/// dials it (routed by the relay). The MLS join + encrypted catch-up then run over
/// the relayed connection, unchanged. This is the NAT-traversal path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn joins_a_server_only_reachable_through_a_relay() {
    catcoms_log::init_test();

    // A relay-server node.
    let relay_addr: Multiaddr = "/memory/990100".parse().unwrap();
    let mut relay_swarm = build_memory_relay_swarm();
    let relay_id = *relay_swarm.local_peer_id();
    relay_swarm.listen_on(relay_addr.clone()).unwrap();
    let relay_task = tokio::spawn(run_relay(relay_swarm));

    // The server reserves a circuit slot on the relay and advertises that address.
    // It deliberately does NOT listen on any direct address, so the joiner can only
    // reach it via the relay.
    let s_mesh = MeshService::new_memory(None, std::slice::from_ref(&relay_addr)).unwrap();
    let circuit: Multiaddr = format!("{relay_addr}/p2p/{relay_id}/p2p-circuit")
        .parse()
        .unwrap();
    s_mesh.listen_on(circuit).await.unwrap();
    let server_circuit_addr = await_circuit_addr(&s_mesh).await;
    let s_peer = s_mesh.local_peer();

    let server = MlsDevice::generate().unwrap();
    let server_group = ServerGroup::create(&server).unwrap();
    let mut ssy = ChannelSync::new(
        s_mesh,
        server_group,
        server,
        OsCryptoRng,
        Box::new(SystemClock),
    );
    ssy.subscribe_control().await.unwrap();
    ssy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    ssy.post(DocType::Channel, CHANNEL, |d| {
        d.put(ROOT, "greeting", "reached via relay")
    })
    .await
    .unwrap();
    // The invite's bootstrap is the server's *circuit* address.
    let invite = ssy
        .mint_invite([8u8; 16], u64::MAX, vec![server_circuit_addr.to_string()])
        .unwrap();
    let server_loop = tokio::spawn(async move { while ssy.run_once().await.unwrap_or(false) {} });

    // The joiner dials the server's circuit address (routed through the relay).
    let j_mesh = MeshService::new_memory(None, std::slice::from_ref(&server_circuit_addr)).unwrap();
    timeout(Duration::from_secs(20), async {
        loop {
            if let Some(TransportEvent::PeerConnected(p)) = j_mesh.next_event().await {
                if p == s_peer {
                    break;
                }
            }
        }
    })
    .await
    .expect("joiner did not connect to the server through the relay");

    let joiner = MlsDevice::generate().unwrap();
    let (joiner_group, joiner_routing) = timeout(
        Duration::from_secs(20),
        catcoms_sync::request_join(&j_mesh, s_peer, &joiner, &invite),
    )
    .await
    .expect("join timed out")
    .expect("joined through the relay");
    assert_eq!(joiner_group.epoch(), 1);

    let mut jsy = ChannelSync::new_joined(
        j_mesh,
        joiner_group,
        joiner,
        OsCryptoRng,
        Box::new(SystemClock),
        joiner_routing,
    );
    jsy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    let applied = timeout(
        Duration::from_secs(20),
        jsy.request_catchup(s_peer, DocType::Channel, CHANNEL),
    )
    .await
    .expect("catch-up timed out")
    .expect("catch-up succeeded");
    assert_eq!(applied, 1);

    let greeting = jsy
        .doc(DocType::Channel, CHANNEL)
        .unwrap()
        .doc()
        .get(ROOT, "greeting")
        .unwrap()
        .map(|(v, _)| v.into_string().unwrap());
    assert_eq!(greeting.as_deref(), Some("reached via relay"));

    server_loop.abort();
    relay_task.abort();
}
