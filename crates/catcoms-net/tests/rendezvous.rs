//! The rendezvous server (6e-3d-3): a throwaway `rendezvous::client` registers under
//! a namespace and the server grants it. Memory transport, deterministic. The mesh
//! client integration (into `MeshBehaviour`) is a separate slice (6e-3d-4); here the
//! client is a minimal stand-in so this slice tests independently.

use std::collections::HashSet;
use std::time::Duration;

use catcoms_net::{
    build_memory_relay_swarm, build_memory_rendezvous_swarm, phase0_peer_id, run_relay,
    run_rendezvous, MeshService,
};
use catcoms_rt::{MeshTransport, TransportEvent};
use futures::StreamExt as _;
use libp2p::core::transport::MemoryTransport;
use libp2p::core::upgrade::Version;
use libp2p::swarm::SwarmEvent;
use libp2p::{noise, rendezvous, yamux, Multiaddr, Swarm, SwarmBuilder, Transport};

#[test]
fn rendezvous_address_validation_rejects_circuits_and_duplicates() {
    use catcoms_net::validate_rendezvous_addrs;
    let p1 = libp2p::PeerId::random();
    let p2 = libp2p::PeerId::random();
    let a1 = format!("/ip4/1.2.3.4/tcp/5000/p2p/{p1}");
    let a2 = format!("/ip4/5.6.7.8/tcp/5000/p2p/{p2}");
    // Two distinct direct addresses validate and parse to their peer ids.
    let ok = validate_rendezvous_addrs(&[a1.clone(), a2]).unwrap();
    assert_eq!(ok.len(), 2);
    assert_eq!(ok[0].peer, p1);
    // The same PeerId twice is rejected (misconfig guard).
    assert!(validate_rendezvous_addrs(&[a1.clone(), a1.clone()]).is_err());
    // A /p2p-circuit address is rejected (rendezvous must be direct).
    let circuit = format!("/ip4/1.2.3.4/tcp/4000/p2p/{p1}/p2p-circuit/p2p/{p2}");
    assert!(validate_rendezvous_addrs(&[circuit]).is_err());
    // An address with no peer id is rejected.
    assert!(validate_rendezvous_addrs(&["/ip4/1.2.3.4/tcp/5000".to_string()]).is_err());
}

/// A minimal memory-transport swarm whose only behaviour is the rendezvous client.
fn build_rendezvous_client() -> Swarm<rendezvous::client::Behaviour> {
    SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_other_transport(|key| {
            MemoryTransport::default()
                .upgrade(Version::V1)
                .authenticate(noise::Config::new(key).expect("noise"))
                .multiplex(yamux::Config::default())
        })
        .expect("memory transport")
        .with_behaviour(|key| rendezvous::client::Behaviour::new(key.clone()))
        .expect("client behaviour")
        .build()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rendezvous_server_grants_a_registration() {
    // A rendezvous server, run via the production event loop.
    let server_addr: Multiaddr = "/memory/880088".parse().unwrap();
    let mut server = build_memory_rendezvous_swarm();
    let server_id = *server.local_peer_id();
    server.listen_on(server_addr.clone()).unwrap();
    let server_task = tokio::spawn(run_rendezvous(server));

    // A client given an external address (so `register` is permitted — registrations
    // advertise the registrant's reachable address) dials the server and registers.
    let mut client = build_rendezvous_client();
    client.add_external_address("/memory/991100".parse().unwrap());
    client.dial(server_addr).unwrap();
    let namespace = rendezvous::Namespace::new("catcoms-rendezvous-test".to_string()).unwrap();

    let granted = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match client.select_next_some().await {
                SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == server_id => {
                    client
                        .behaviour_mut()
                        .register(namespace.clone(), server_id, None)
                        .expect("register issued");
                }
                SwarmEvent::Behaviour(rendezvous::client::Event::Registered {
                    namespace: ns,
                    rendezvous_node,
                    ..
                }) => {
                    assert_eq!(ns, namespace);
                    assert_eq!(rendezvous_node, server_id);
                    return true;
                }
                SwarmEvent::Behaviour(rendezvous::client::Event::RegisterFailed {
                    error, ..
                }) => panic!("the server refused the registration: {error:?}"),
                _ => {}
            }
        }
    })
    .await
    .expect("registration was not granted within the timeout");

    assert!(granted);
    server_task.abort();
}

/// Wait until `mesh` has connected to every peer in `targets` (phase-0 ids).
async fn wait_connected(mesh: &MeshService, targets: &[catcoms_rt::PeerId]) {
    let mut remaining: HashSet<_> = targets.iter().copied().collect();
    tokio::time::timeout(Duration::from_secs(15), async {
        while !remaining.is_empty() {
            if let Some(TransportEvent::PeerConnected(p)) = mesh.next_event().await {
                remaining.remove(&p);
            }
        }
    })
    .await
    .expect("did not connect to all targets");
}

/// 6e-3d-4: a `MeshService` node reserves a relay circuit (its external address),
/// registers that record at a rendezvous, and a second node discovers it — surfaced,
/// never auto-dialed. Relay + rendezvous + two mesh nodes over the memory transport.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_node_registers_via_a_circuit_and_another_discovers_it() {
    // A relay and a rendezvous server.
    let relay_addr: Multiaddr = "/memory/770011".parse().unwrap();
    let mut relay = build_memory_relay_swarm();
    let relay_id = *relay.local_peer_id();
    relay.listen_on(relay_addr.clone()).unwrap();
    let relay_task = tokio::spawn(run_relay(relay));

    let rz_addr: Multiaddr = "/memory/770012".parse().unwrap();
    let mut rz = build_memory_rendezvous_swarm();
    let rz_id = *rz.local_peer_id();
    rz.listen_on(rz_addr.clone()).unwrap();
    let rz_task = tokio::spawn(run_rendezvous(rz));

    let namespace = "catcoms-3d4-namespace";

    // Node A: dials the relay and the rendezvous, reserves a circuit (its external
    // address), then registers that record.
    let a = MeshService::new_memory(None, &[relay_addr.clone(), rz_addr.clone()]).unwrap();
    wait_connected(&a, &[phase0_peer_id(&relay_id), phase0_peer_id(&rz_id)]).await;

    let circuit: Multiaddr = format!("{relay_addr}/p2p/{relay_id}/p2p-circuit")
        .parse()
        .unwrap();
    a.listen_on(circuit).await.unwrap();
    // Await the granted circuit reservation (gives A an external address to advertise).
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match a.next_listen_addr().await {
                Some(addr) if addr.to_string().contains("p2p-circuit") => return,
                Some(_) => continue,
                None => panic!("A's actor stopped"),
            }
        }
    })
    .await
    .expect("A's circuit reservation was not granted");

    a.rendezvous_register(namespace, rz_id).await.unwrap();
    let registered = tokio::time::timeout(Duration::from_secs(20), a.next_registered())
        .await
        .expect("A registration timed out")
        .expect("A actor stopped");
    assert_eq!(registered.namespace, namespace);
    assert_eq!(registered.rendezvous_node, rz_id);

    // Node B: dials the rendezvous and discovers A.
    let b = MeshService::new_memory(None, std::slice::from_ref(&rz_addr)).unwrap();
    wait_connected(&b, &[phase0_peer_id(&rz_id)]).await;
    b.rendezvous_discover(namespace, rz_id).await.unwrap();

    let discovered = tokio::time::timeout(Duration::from_secs(20), b.next_discovered())
        .await
        .expect("B discovery timed out")
        .expect("B actor stopped");
    assert_eq!(
        phase0_peer_id(&discovered.peer),
        a.local_peer(),
        "B discovered A's record"
    );
    assert!(
        !discovered.addresses.is_empty(),
        "the discovered record carries A's advertised (circuit) address"
    );
    assert_eq!(discovered.namespace, namespace);

    // No auto-dial: discovering A must NOT make B connect to A (a higher layer decides
    // whether/when to dial — that is where eclipse-resistance lives).
    let dialed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(TransportEvent::PeerConnected(p)) = b.next_event().await {
                if p == a.local_peer() {
                    return true;
                }
            }
        }
    })
    .await;
    assert!(
        dialed.is_err(),
        "the transport must not auto-dial a rendezvous-discovered peer"
    );

    relay_task.abort();
    rz_task.abort();
}
