//! The rendezvous server (6e-3d-3): a throwaway `rendezvous::client` registers under
//! a namespace and the server grants it. Memory transport, deterministic. The mesh
//! client integration (into `MeshBehaviour`) is a separate slice (6e-3d-4); here the
//! client is a minimal stand-in so this slice tests independently.

use std::time::Duration;

use catcoms_net::{build_memory_rendezvous_swarm, run_rendezvous};
use futures::StreamExt as _;
use libp2p::core::transport::MemoryTransport;
use libp2p::core::upgrade::Version;
use libp2p::swarm::SwarmEvent;
use libp2p::{noise, rendezvous, yamux, Multiaddr, Swarm, SwarmBuilder, Transport};

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
