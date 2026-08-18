//! The infra-node limits, exercised over **real sockets** rather than asserted from config.
//!
//! Three things are worth proving end to end, because all three were silently wrong before:
//!
//! 1. a relay sized by [`catcoms_net::RelayLimits`] actually carries a payload that the upstream
//!    `relay::Config::default()` (128 KiB per circuit, both directions summed) would have killed;
//! 2. the byte accounting really sees relayed traffic, which is invisible to every
//!    application-layer limit in the stack because a circuit is a pipe between two *other* peers;
//! 3. the per-circuit byte cap **binds**: turn it down and the transfer dies. A limit nobody has
//!    watched fire is a limit nobody knows the units of.
//!
//! Plus the P11 dial gate: an existing connection must suppress a redundant dial.
//!
//! All three nodes are on TCP loopback. Neither mesh node takes a listen address, so neither has
//! a direct address for DCUtR to punch to and the traffic provably stays on the relay: without
//! that, a loopback hole-punch would move the bytes off the relay and the test would prove
//! nothing.

use std::time::Duration;

use bytes::Bytes;
use catcoms_net::{build_relay_swarm, phase0_peer_id, MeshService, RelayLimits, RelayNode};
use catcoms_rt::{MeshTransport, ProtocolId, TransportEvent};
use futures::StreamExt as _;
use libp2p::swarm::SwarmEvent;
use libp2p::Multiaddr;

/// Payload for the relayed transfer: eight times the 128 KiB that upstream's default budget
/// allows for a whole circuit, in **both** directions summed.
const PAYLOAD: usize = 1024 * 1024;

/// Drive a mesh node until it reports a peer connection, or fail the test.
async fn await_connected(node: &MeshService) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match node.next_event().await {
                Some(TransportEvent::PeerConnected(_)) => return,
                Some(_) => continue,
                None => panic!("mesh actor stopped"),
            }
        }
    })
    .await
    .expect("node did not connect");
}

/// Drive a mesh node until **a specific** peer connects.
///
/// Dialing a circuit address produces two connections in sequence: first the transport-level one
/// to the relay, then the relayed one to the target. Waiting for "a" connection therefore returns
/// on the relay and races the request against a peer the actor has not mapped yet.
async fn await_peer(node: &MeshService, want: catcoms_rt::PeerId) {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match node.next_event().await {
                Some(TransportEvent::PeerConnected(p)) if p == want => return,
                Some(_) => continue,
                None => panic!("mesh actor stopped"),
            }
        }
    })
    .await
    .expect("the target peer never connected");
}

/// Reserve a circuit slot on `relay_addr` and return the granted circuit address.
async fn reserve_circuit(
    node: &MeshService,
    relay_addr: &Multiaddr,
    relay_id: libp2p::PeerId,
) -> Multiaddr {
    node.dial(relay_addr.clone()).await.unwrap();
    await_connected(node).await;
    let circuit: Multiaddr = format!("{relay_addr}/p2p/{relay_id}/p2p-circuit")
        .parse()
        .unwrap();
    node.listen_on(circuit).await.unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match node.next_listen_addr().await {
                Some(a) if a.to_string().contains("p2p-circuit") => return a,
                Some(_) => continue,
                None => panic!("mesh actor stopped"),
            }
        }
    })
    .await
    .expect("circuit reservation was not granted")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dial_is_suppressed_when_the_peer_is_already_connected() {
    // P11 (second half). `Swarm::dial(Multiaddr)` produces `DialOpts` with `peer_id: None` and
    // `PeerCondition::Always`, so every discovery tick opened another connection to the same
    // infra node. Count establishments on the *server* side: exactly one must happen no matter
    // how many times the client is told to dial.
    let mut relay = build_relay_swarm().unwrap();
    let relay_id = *relay.local_peer_id();
    relay
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .unwrap();
    let bound: Multiaddr = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let SwarmEvent::NewListenAddr { address, .. } = relay.select_next_some().await {
                return address;
            }
        }
    })
    .await
    .expect("relay did not bind");
    let target: Multiaddr = format!("{bound}/p2p/{relay_id}").parse().unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let relay_task = tokio::spawn(async move {
        loop {
            if let SwarmEvent::ConnectionEstablished { .. } = relay.select_next_some().await {
                let _ = tx.send(());
            }
        }
    });

    // No listen address: this node only dials.
    let (client, _id) = MeshService::new_tcp(None, &[]).unwrap();
    client.dial(target.clone()).await.unwrap();
    await_connected(&client).await;
    tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("the relay never saw the first connection")
        .expect("channel closed");

    // Now hammer the same address the way an unjittered discovery timer does.
    for _ in 0..8 {
        client.dial(target.clone()).await.unwrap();
    }
    let extra = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await;
    assert!(
        extra.is_err(),
        "an existing connection must suppress further dials; the relay saw another connection"
    );

    relay_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sized_relay_carries_a_payload_the_upstream_default_would_kill() {
    // Keep the sweep out of the way; this test is about the circuit budget, not the shed path.
    let limits = RelayLimits {
        sweep_secs: 3_600,
        ..Default::default()
    };
    let (relay_addr, relay_id, meters, relay_task) = spawn_relay(limits).await;

    // Neither mesh node listens directly, so DCUtR has nothing to punch to and the bytes stay on
    // the relay for the whole test.
    let (server, server_id) = MeshService::new_tcp(None, &[]).unwrap();
    let circuit = reserve_circuit(&server, &relay_addr, relay_id).await;

    let responder = tokio::spawn(async move {
        while let Some(event) = server.next_event().await {
            if let TransportEvent::Request {
                data, responder, ..
            } = event
            {
                assert_eq!(data.len(), PAYLOAD);
                responder.respond(Bytes::from_static(b"ok"));
            }
        }
    });

    let (client, _) = MeshService::new_tcp(None, std::slice::from_ref(&circuit)).unwrap();
    await_peer(&client, phase0_peer_id(&server_id)).await;

    let reply = tokio::time::timeout(
        Duration::from_secs(60),
        client.request(
            phase0_peer_id(&server_id),
            ProtocolId("/catcoms/rr/1"),
            Bytes::from(vec![7u8; PAYLOAD]),
        ),
    )
    .await
    .expect("the relayed transfer timed out")
    .expect("the relayed transfer failed");
    assert_eq!(&reply[..], b"ok");

    // The accounting must have seen it. The relay moves the payload twice (in from the source,
    // out to the destination), so the node total is at least two payloads plus framing; this is
    // the number an operator's bill is denominated in, and nothing in libp2p reports it.
    let total = meters.total_bytes();
    assert!(
        total >= 2 * PAYLOAD as u64,
        "relay accounting saw only {total} bytes for a {PAYLOAD}-byte relayed transfer"
    );

    responder.abort();
    relay_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_per_circuit_byte_cap_binds() {
    // Same shape, but with the circuit budget turned down to a quarter of the payload. The
    // transfer must fail, which is what proves the knob is load-bearing and that the previous
    // 128 KiB default really did make files and calls impossible.
    let limits = RelayLimits {
        sweep_secs: 3_600,
        max_circuit_bytes: (PAYLOAD / 4) as u64,
        ..Default::default()
    };
    let (relay_addr, relay_id, _meters, relay_task) = spawn_relay(limits).await;

    let (server, server_id) = MeshService::new_tcp(None, &[]).unwrap();
    let circuit = reserve_circuit(&server, &relay_addr, relay_id).await;
    let responder = tokio::spawn(async move {
        while let Some(event) = server.next_event().await {
            if let TransportEvent::Request {
                data, responder, ..
            } = event
            {
                assert_eq!(data.len(), PAYLOAD);
                responder.respond(Bytes::from_static(b"ok"));
            }
        }
    });

    let (client, _) = MeshService::new_tcp(None, std::slice::from_ref(&circuit)).unwrap();
    await_peer(&client, phase0_peer_id(&server_id)).await;

    let outcome = tokio::time::timeout(
        Duration::from_secs(60),
        client.request(
            phase0_peer_id(&server_id),
            ProtocolId("/catcoms/rr/1"),
            Bytes::from(vec![7u8; PAYLOAD]),
        ),
    )
    .await
    .expect("the request neither completed nor failed within the timeout");
    assert!(
        outcome.is_err(),
        "a {PAYLOAD}-byte transfer must not fit in a {}-byte circuit budget",
        PAYLOAD / 4
    );

    responder.abort();
    relay_task.abort();
}

/// Build a `RelayNode`, bind it to a concrete loopback address, and run it in the background.
///
/// The bound port has to be known before `run()` takes the node, so the port is claimed with a
/// scratch `std::net::TcpListener` and released immediately: a port the OS just handed out is
/// overwhelmingly likely to still be free a microsecond later, and a collision would surface as a
/// plain listen error rather than a silent pass.
async fn spawn_relay(
    limits: RelayLimits,
) -> (
    Multiaddr,
    libp2p::PeerId,
    catcoms_net::ByteMeters,
    tokio::task::JoinHandle<()>,
) {
    let port = {
        let scratch = std::net::TcpListener::bind("127.0.0.1:0").expect("scratch bind");
        scratch.local_addr().expect("scratch addr").port()
    };
    let addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/{port}").parse().unwrap();

    let key = libp2p::identity::Keypair::generate_ed25519();
    let mut node = RelayNode::build(key, limits, None).expect("relay builds");
    let relay_id = node.local_peer_id();
    let meters = node.meters();
    node.listen_on(addr.clone()).expect("relay listens");
    node.add_external_address(addr.clone())
        .expect("a concrete loopback address is advertisable");
    let task = tokio::spawn(async move {
        node.run().await.expect("relay start-up config is valid");
    });
    (addr, relay_id, meters, task)
}
