//! Phase 7; the full **discovery bootstrap over real TCP sockets**: a joiner finds the
//! inviter at a zero-knowledge rendezvous under the pre-join `join_ns` and joins with
//! **no hard-coded server address**, all over genuine OS sockets (not the libp2p memory
//! transport that the 6e-3d-9 memory e2e uses). This is the 6e-3d-9 headline property
//! proven over real networking: rendezvous server on TCP loopback, inviter registers
//! under `join_ns`, joiner discovers → dials (via the `DiscoveryPolicy`) → joins.

use std::time::Duration;

use catcoms_discovery::{Candidate, DiscoveryPolicy, PolicyConfig, Source};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_net::{
    build_rendezvous_swarm, phase0_peer_id, run_rendezvous, validate_rendezvous_addrs, MeshService,
};
use catcoms_rt::{MeshTransport, OsCryptoRng, PeerId, SystemClock, TransportEvent};
use catcoms_sync::{join_namespace, request_join, ChannelSync};
use catcoms_wire::DocType;
use futures::StreamExt as _;
use libp2p::swarm::SwarmEvent;
use libp2p::Multiaddr;
use tokio::time::timeout;

const CHANNEL: u128 = 1;

/// Block until `mesh` reports a connection to `target` (a phase-0 peer id).
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn joiner_discovers_inviter_via_rendezvous_and_joins_over_real_tcp() {
    catcoms_log::init_test();

    // --- Rendezvous server on TCP loopback. Capture its bound address, then run it. ---
    let mut rz_swarm = build_rendezvous_swarm().unwrap();
    let rz_id = *rz_swarm.local_peer_id();
    rz_swarm
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .unwrap();
    let rz_addr: Multiaddr = timeout(Duration::from_secs(10), async {
        loop {
            if let SwarmEvent::NewListenAddr { address, .. } = rz_swarm.select_next_some().await {
                return address;
            }
        }
    })
    .await
    .expect("rendezvous did not bind a TCP address");
    let rz_dial: Multiaddr = format!("{rz_addr}/p2p/{rz_id}").parse().unwrap();
    let rz_task = tokio::spawn(run_rendezvous(rz_swarm));

    // --- Alice (inviter): binds an ephemeral TCP port, founds a group, connects to the
    //     rendezvous, advertises her bound address, and registers under join_ns. ---
    let (a_mesh, a_id) = MeshService::new_tcp(
        Some("/ip4/127.0.0.1/tcp/0".parse().unwrap()),
        std::slice::from_ref(&rz_dial),
    )
    .unwrap();
    let a_peer = a_mesh.local_peer();
    let a_addr: Multiaddr = timeout(Duration::from_secs(10), a_mesh.next_listen_addr())
        .await
        .expect("Alice listen-addr timeout")
        .expect("Alice bound a TCP address");
    wait_connected(&a_mesh, phase0_peer_id(&rz_id), "the rendezvous").await;

    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let nonce = [9u8; 16];
    let invite = alice_group
        .mint_invite_with_rendezvous(&alice, nonce, u64::MAX, vec![], vec![rz_dial.to_string()])
        .unwrap();
    let group_id = alice_group.group_id();
    let join_ns = join_namespace(&group_id, &nonce, &rz_id.to_bytes());

    // Advertise Alice's reachable TCP address and register under join_ns (all on the raw
    // mesh, before it is handed to ChannelSync to serve).
    let a_route: Multiaddr = format!("{a_addr}/p2p/{a_id}").parse().unwrap();
    a_mesh.add_external_address(a_route).await.unwrap();
    a_mesh.rendezvous_register(&join_ns, rz_id).await.unwrap();
    timeout(Duration::from_secs(20), a_mesh.next_registered())
        .await
        .expect("Alice registration timed out")
        .expect("Alice actor stopped");

    let mut asy = ChannelSync::new(
        a_mesh,
        alice_group,
        alice,
        OsCryptoRng,
        Box::new(SystemClock),
    );
    asy.subscribe_control().await.unwrap();
    asy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    let alice_loop = tokio::spawn(async move { while asy.run_once().await.unwrap_or(false) {} });

    // --- Bob (joiner): knows ONLY the rendezvous address (plus the pasted invite). ---
    let (b_mesh, _b_id) = MeshService::new_tcp(None, std::slice::from_ref(&rz_dial)).unwrap();
    wait_connected(&b_mesh, phase0_peer_id(&rz_id), "the rendezvous").await;

    let targets = validate_rendezvous_addrs(&invite.rendezvous).unwrap();
    assert_eq!(targets[0].peer, rz_id);
    let bob_join_ns = join_namespace(&invite.group_id, &invite.invite_nonce, &rz_id.to_bytes());
    assert_eq!(bob_join_ns, join_ns, "the joiner derives the same join_ns");

    b_mesh
        .rendezvous_discover(&bob_join_ns, rz_id)
        .await
        .unwrap();
    let discovered = timeout(Duration::from_secs(20), b_mesh.next_discovered())
        .await
        .expect("discovery timed out")
        .expect("Bob actor stopped");
    assert_eq!(
        phase0_peer_id(&discovered.peer),
        a_peer,
        "Bob discovered Alice under join_ns over TCP"
    );
    assert!(!discovered.addresses.is_empty());

    // The DiscoveryPolicy decides what to dial (no auto-dial); dial the planned address.
    let mut policy = DiscoveryPolicy::with_config(PolicyConfig::default());
    let candidate = Candidate {
        peer: discovered.peer.to_bytes(),
        addresses: discovered.addresses.iter().map(|a| a.to_string()).collect(),
        source: Source::Rendezvous(rz_id.to_bytes()),
        seq: 1,
        tag_verified: false,
    };
    let mut rng = OsCryptoRng;
    let dialed = policy
        .plan(vec![candidate], 2, &SystemClock, &mut rng)
        .into_iter()
        .next()
        .expect("the policy offers the discovered inviter to dial");
    for addr in &dialed.addresses {
        b_mesh.dial(addr.parse().unwrap()).await.unwrap();
    }
    wait_connected(&b_mesh, a_peer, "the server").await;

    // Join the MLS group over the discovered + dialed real-TCP connection.
    let bob = MlsDevice::generate().unwrap();
    let (bob_group, _routing) = timeout(
        Duration::from_secs(20),
        request_join(&b_mesh, a_peer, &bob, &invite),
    )
    .await
    .expect("join timed out")
    .expect("Bob joined the inviter discovered via rendezvous over TCP");
    assert_eq!(bob_group.epoch(), 1);
    assert!(bob_group.contains_device(&bob.device_id()));

    alice_loop.abort();
    rz_task.abort();
}
