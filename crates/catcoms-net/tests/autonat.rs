//! AutoNAT v2 integration at the real `MeshService` boundary.
//!
//! The important property is not merely that both behaviours compile into their swarms: a mesh
//! client must offer an address candidate, a public infrastructure swarm must really dial it back,
//! and the result must survive the actor channel used by the desktop diagnostics.

use std::time::Duration;

use catcoms_net::{build_memory_rendezvous_swarm, run_rendezvous, MeshService};
use catcoms_rt::MeshTransport;
use futures::StreamExt;
use libp2p::{multiaddr::Protocol, swarm::SwarmEvent, Multiaddr};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(15);

async fn autonat_server(memory_port: u64) -> Multiaddr {
    let mut swarm = build_memory_rendezvous_swarm();
    let peer = *swarm.local_peer_id();
    swarm
        .listen_on(format!("/memory/{memory_port}").parse().unwrap())
        .unwrap();
    let mut address = timeout(WAIT, async {
        loop {
            if let SwarmEvent::NewListenAddr { address, .. } = swarm.select_next_some().await {
                break address;
            }
        }
    })
    .await
    .expect("AutoNAT server did not start listening");
    address.push(Protocol::P2p(peer));
    tokio::spawn(run_rendezvous(swarm));
    address
}

async fn result_for(client: &MeshService, wanted: &Multiaddr) -> catcoms_net::AutoNatResult {
    timeout(WAIT, async {
        loop {
            let snapshot = client
                .next_autonat_snapshot()
                .await
                .expect("AutoNAT snapshot stream closed");
            // Identify also contributes the observed outbound memory address as a candidate. It
            // is deliberately tested too; this helper selects the explicit candidate whose
            // product-facing result this regression test pins.
            if let Some(result) = snapshot
                .results
                .into_iter()
                .find(|result| &result.address == wanted)
            {
                break result;
            }
        }
    })
    .await
    .expect("AutoNAT result for explicit candidate timed out")
}

#[tokio::test]
async fn public_infrastructure_dials_a_mesh_candidate_back() {
    let server = autonat_server(41_001).await;
    let listen: Multiaddr = "/memory/41002".parse().unwrap();
    let client =
        MeshService::new_memory(Some(listen.clone()), &[server]).expect("build AutoNAT client");

    // Wait for the server connection so its protocol support is known before the five-second
    // probe tick. This is observable at the same public boundary the product uses.
    timeout(WAIT, client.next_event())
        .await
        .expect("client did not connect to AutoNAT server")
        .expect("client actor stopped");
    client
        .add_external_address(listen.clone())
        .await
        .expect("offer candidate");

    let result = result_for(&client, &listen).await;
    assert_eq!(result.address, listen);
    assert!(result.reachable, "callback failed: {:?}", result.error);
    assert_eq!(result.error, None);
}

#[tokio::test]
async fn a_failed_callback_is_scoped_to_the_tested_address() {
    let server = autonat_server(41_011).await;
    let listen: Multiaddr = "/memory/41012".parse().unwrap();
    let unreachable: Multiaddr = "/memory/41013".parse().unwrap();
    let client = MeshService::new_memory(Some(listen), &[server]).expect("build AutoNAT client");
    timeout(WAIT, client.next_event())
        .await
        .expect("client did not connect to AutoNAT server")
        .expect("client actor stopped");
    client
        .add_external_address(unreachable.clone())
        .await
        .expect("offer candidate");

    let result = result_for(&client, &unreachable).await;
    assert_eq!(result.address, unreachable);
    assert!(!result.reachable);
    assert!(result.error.is_some(), "a rejection needs a diagnostic");
}
