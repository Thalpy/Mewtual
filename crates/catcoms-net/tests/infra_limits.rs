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
//! Then the two admission limits that are the whole reason the node is safe to expose, exercised
//! the same way (turn them down until a connection is refused):
//!
//! 4. the **per-source-prefix connection quota** refuses the second caller from one address, no
//!    matter what identity it presents;
//! 5. the **per-source-prefix byte budget** disconnects and refuses the *address* that blew it, so
//!    a caller that mints a fresh keypair and comes straight back is still refused. That is the
//!    difference between a defence that costs an attacker addresses and one that costs them a
//!    keypair, and the shipped code had only the latter.
//!
//! Plus the P11 dial gate: an existing connection must suppress a redundant dial to an infra node.
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

/// Whether a freshly-built node can reach `target` within `secs`.
///
/// Every call mints a brand new `MeshService`, and therefore a brand new libp2p identity, from the
/// same loopback address. That is exactly the shape of the evasion these limits exist to stop: a
/// defence keyed on `PeerId` sees a stranger every time and lets them all in.
async fn fresh_node_reaches(target: &Multiaddr, secs: u64) -> bool {
    let (probe, _id) = MeshService::new_tcp(None, &[]).unwrap();
    probe.dial(target.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(secs), async {
        loop {
            match probe.next_event().await {
                Some(TransportEvent::PeerConnected(_)) => return,
                Some(_) => continue,
                None => panic!("mesh actor stopped"),
            }
        }
    })
    .await
    .is_ok()
}

/// Wait until **a specific** peer is currently connected without consuming its lifecycle event.
///
/// These real-socket fixtures run concurrently inside this test binary and alongside the rest of
/// the workspace on CI. A 20-second event wait intermittently expired on a loaded Linux runner,
/// before the test reached the dial-suppression assertion. The connection snapshot is the actual
/// premise these tests need, survives an event that arrived before the waiter, and leaves ordered
/// events available to the later disconnect assertion. The 90-second outer deadline matches the
/// circuit-reservation fixture's existing full-suite contention allowance while still bounding a
/// genuinely dead actor or socket.
async fn await_peer(node: &MeshService, want: catcoms_rt::PeerId) {
    tokio::time::timeout(Duration::from_secs(90), node.wait_for_peer_connected(want))
        .await
        .expect("the target peer never connected before the CI contention deadline")
        .expect("mesh actor stopped while waiting for the target peer");
}

/// Reserve a circuit slot on `relay_addr` and return the granted circuit address.
async fn reserve_circuit(
    node: &MeshService,
    relay_addr: &Multiaddr,
    relay_id: libp2p::PeerId,
) -> Multiaddr {
    // Runtime dialing deliberately refuses a bare socket: without the terminal identity there is
    // no authenticated target for the actor to bind the connection to. The relay reservation
    // fixture must exercise the same canonical route production uses.
    let relay_route: Multiaddr = format!("{relay_addr}/p2p/{relay_id}").parse().unwrap();
    node.dial(relay_route).await.unwrap();
    await_peer(node, phase0_peer_id(&relay_id)).await;
    let circuit: Multiaddr = format!("{relay_addr}/p2p/{relay_id}/p2p-circuit")
        .parse()
        .unwrap();
    node.listen_on(circuit).await.unwrap();
    // Generous on purpose. A reservation needs a TCP connect, a Noise handshake and a relay
    // round trip, and `cargo test` runs these real-socket tests alongside every other test
    // binary on the machine. A 20s budget passed in isolation and intermittently blew up under
    // that parallel load, which reads as a product failure and is not one.
    tokio::time::timeout(Duration::from_secs(90), async {
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
    await_peer(&client, phase0_peer_id(&relay_id)).await;
    tokio::time::timeout(Duration::from_secs(30), rx.recv())
        .await
        .expect("the relay never saw the first connection")
        .expect("channel closed");

    // Now hammer the same address the way an unjittered discovery timer does.
    for _ in 0..8 {
        client.dial(target.clone()).await.unwrap();
    }
    // A negative assertion over a wall-clock window is only meaningful while the premise holds:
    // the gate suppresses a dial because the peer is *still connected*, and the dial ledger is
    // deliberately cleared when a peer goes away so a later tick may dial it again. If the link
    // dropped underneath us (an idle timeout, or the machine starving this task under parallel
    // load) then a fresh connection is the gate working, not failing, and asserting otherwise
    // reports a product bug that is not there. So distinguish the two outcomes.
    let extra = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
    if extra.is_ok() {
        // Drain whatever the client already observed, without blocking on a quiet stream, and
        // look for the link having gone away. A dropped link makes a second connection correct.
        let mut dropped = false;
        while let Ok(Some(ev)) =
            tokio::time::timeout(Duration::from_millis(50), client.next_event()).await
        {
            if matches!(ev, TransportEvent::PeerDisconnected(_)) {
                dropped = true;
            }
        }
        assert!(
            dropped,
            "an existing connection must suppress further dials; the relay saw another connection \
             while the first was still up"
        );
    }

    relay_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sized_relay_carries_a_payload_the_upstream_default_would_kill() {
    // Keep the sweep out of the way; this test is about the circuit budget, not the shed path.
    let limits = RelayLimits {
        sweep_secs: 3_600,
        rate_window_secs: 3_600,
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
        rate_window_secs: 3_600,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_per_source_prefix_connection_quota_binds() {
    // Turned down to one connection per source network. Loopback is one `/24`, so the second
    // caller is refused however many identities it is willing to generate: the quota is keyed on
    // the address, which is the only identifier on the wire that costs an attacker anything.
    let limits = RelayLimits {
        sweep_secs: 3_600,
        rate_window_secs: 3_600,
        admission: catcoms_net::admission::AdmissionConfig {
            max_conns_per_prefix: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let (relay_addr, relay_id, _meters, relay_task) = spawn_relay(limits).await;
    let target: Multiaddr = format!("{relay_addr}/p2p/{relay_id}").parse().unwrap();

    // The first caller from this prefix takes the only slot.
    let (first, _) = MeshService::new_tcp(None, &[]).unwrap();
    first.dial(target.clone()).await.unwrap();
    await_peer(&first, phase0_peer_id(&relay_id)).await;

    // A second, with a different peer id, is refused. Not slow: refused.
    assert!(
        !fresh_node_reaches(&target, 5).await,
        "the per-prefix connection quota did not bind: a second identity from the same source \
         address was admitted"
    );

    relay_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blowing_the_byte_budget_denies_the_address_not_just_the_keypair() {
    // CRITICAL 2, relay half, over real sockets. The byte budget used to deny the `PeerId` that
    // blew it, and a `PeerId` is a self-minted keypair: rotating it evaded the per-peer budget
    // entirely, which is what left only the node aggregate binding and made the whole node
    // reachable by a handful of self-owned circuits.
    //
    // Budgets are turned down to a quarter of the test payload so one relayed transfer blows them,
    // and the sweep runs every second so the shed lands inside the test.
    let payload = PAYLOAD as u64;
    let limits = RelayLimits {
        sweep_secs: 1,
        peer_budget_bytes: payload / 4,
        prefix_budget_bytes: payload / 4,
        shed_cooldown_secs: 600,
        ..Default::default()
    };
    let (relay_addr, relay_id, meters, relay_task) = spawn_relay(limits).await;
    let target: Multiaddr = format!("{relay_addr}/p2p/{relay_id}").parse().unwrap();

    // Control: before anything is spent, a fresh identity from this address is admitted. Without
    // this the test would also pass against a relay that was simply never listening.
    assert!(
        fresh_node_reaches(&target, 20).await,
        "the relay refused a fresh caller before any budget was spent"
    );

    // Move enough bytes through a circuit to blow the budget. The relay charges the payload twice
    // (in from the source, out to the destination), so one transfer is well over.
    let (server, server_id) = MeshService::new_tcp(None, &[]).unwrap();
    let circuit = reserve_circuit(&server, &relay_addr, relay_id).await;
    let responder = tokio::spawn(async move {
        while let Some(event) = server.next_event().await {
            if let TransportEvent::Request {
                data, responder, ..
            } = event
            {
                responder.respond(Bytes::from(vec![0u8; data.len()]));
            }
        }
    });
    let (client, _) = MeshService::new_tcp(None, std::slice::from_ref(&circuit)).unwrap();
    await_peer(&client, phase0_peer_id(&server_id)).await;
    let _ = tokio::time::timeout(
        Duration::from_secs(60),
        client.request(
            phase0_peer_id(&server_id),
            ProtocolId("/catcoms/rr/1"),
            Bytes::from(vec![7u8; PAYLOAD]),
        ),
    )
    .await;
    assert!(
        meters.total_bytes() > payload / 4,
        "the transfer did not spend the budget: only {} bytes metered",
        meters.total_bytes()
    );

    // Wait for the shed to land rather than guessing at it: the sweep disconnects everything in
    // the offending prefix, so the client losing its peer *is* the signal that the sweep has run.
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match client.next_event().await {
                Some(TransportEvent::PeerDisconnected(_)) => return,
                Some(_) => continue,
                None => panic!("mesh actor stopped"),
            }
        }
    })
    .await
    .expect("the relay never shed the peers that blew its byte budget");

    // Now the deny has to reach the *address*. This probe is a brand new identity from the same
    // source, which is exactly what a per-peer deny lets straight back in.
    assert!(
        !fresh_node_reaches(&target, 8).await,
        "a source that blew the relay's byte budget was still admitted under a fresh keypair: the \
         deny is on the identity, not the address"
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
