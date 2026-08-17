//! Phase 7; full-stack join **through a relay over real TCP sockets** (NAT traversal).
//! The server is reachable *only* via a zero-knowledge circuit relay (it never binds a
//! directly-dialable address); the joiner dials the server's relayed circuit address,
//! and the MLS join + encrypted catch-up run over the relayed connection; over genuine
//! OS sockets, not the libp2p memory transport. Together with `tcp_e2e` (direct) and
//! `tcp_rendezvous_e2e` (discovered), this proves all three networking paths over real
//! sockets.

use std::time::Duration;

use automerge::transaction::Transactable;
use automerge::{ReadDoc, ROOT};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_net::{build_relay_swarm, run_relay, MeshService};
use catcoms_rt::{MeshTransport, OsCryptoRng, PeerId, SystemClock, TransportEvent};
use catcoms_sync::ChannelSync;
use catcoms_wire::DocType;
use futures::StreamExt as _;
use libp2p::swarm::SwarmEvent;
use libp2p::Multiaddr;
use tokio::time::timeout;

const CHANNEL: u128 = 1;

/// Block until `mesh` reports a connection to `target`.
async fn wait_connected(mesh: &MeshService, target: PeerId, what: &str) {
    timeout(Duration::from_secs(20), async {
        loop {
            if let Some(TransportEvent::PeerConnected(p)) = mesh.next_event().await {
                if p == target {
                    return;
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("did not connect to {what}"));
}

/// Await a `…/p2p-circuit` listen address from a relay reservation.
async fn await_circuit_addr(mesh: &MeshService) -> Multiaddr {
    timeout(Duration::from_secs(20), async {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn joins_a_server_reachable_only_through_a_relay_over_real_tcp() {
    catcoms_log::init_test();

    // --- Relay on TCP loopback. Capture its bound address, then run it. ---
    let mut relay_swarm = build_relay_swarm().unwrap();
    let relay_id = *relay_swarm.local_peer_id();
    relay_swarm
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .unwrap();
    let relay_addr: Multiaddr = timeout(Duration::from_secs(10), async {
        loop {
            if let SwarmEvent::NewListenAddr { address, .. } = relay_swarm.select_next_some().await
            {
                relay_swarm.add_external_address(address.clone());
                return address;
            }
        }
    })
    .await
    .expect("relay did not bind a TCP address");
    let relay_task = tokio::spawn(run_relay(relay_swarm));

    // --- Server: reserves a circuit slot on the relay and advertises that circuit
    //     address. It binds NO direct address, so the joiner can reach it only via the
    //     relay. ---
    let (s_mesh, _s_id) = MeshService::new_tcp(None, std::slice::from_ref(&relay_addr)).unwrap();
    wait_connected(&s_mesh, catcoms_net::phase0_peer_id(&relay_id), "the relay").await;
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
        d.put(ROOT, "greeting", "reached via relay over tcp")
    })
    .await
    .unwrap();
    // The invite's bootstrap is the server's *circuit* address.
    let invite = ssy
        .mint_invite([8u8; 16], u64::MAX, vec![server_circuit_addr.to_string()])
        .unwrap();
    let server_loop = tokio::spawn(async move { while ssy.run_once().await.unwrap_or(false) {} });

    // --- Joiner: dials the server's circuit address (routed through the relay). ---
    let (j_mesh, _j_id) =
        MeshService::new_tcp(None, std::slice::from_ref(&server_circuit_addr)).unwrap();
    wait_connected(&j_mesh, s_peer, "the server (through the relay)").await;

    let joiner = MlsDevice::generate().unwrap();
    let (joiner_group, joiner_routing) = timeout(
        Duration::from_secs(20),
        catcoms_sync::request_join(&j_mesh, s_peer, &joiner, &invite),
    )
    .await
    .expect("join timed out")
    .expect("joined through the relay over real TCP");
    assert_eq!(joiner_group.epoch(), 1);
    assert!(joiner_group.contains_device(&joiner.device_id()));

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
    assert_eq!(
        applied, 1,
        "the joiner caught up the server's history via the relay"
    );

    let greeting = jsy
        .doc(DocType::Channel, CHANNEL)
        .unwrap()
        .doc()
        .get(ROOT, "greeting")
        .unwrap()
        .map(|(v, _)| v.into_string().unwrap());
    assert_eq!(greeting.as_deref(), Some("reached via relay over tcp"));

    server_loop.abort();
    relay_task.abort();
}
