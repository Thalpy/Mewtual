//! 6e-3d-9 end-to-end (memory transport, real swarms): a joiner finds the inviter at a
//! zero-knowledge **rendezvous** under the pre-join `join_ns`; derivable from the
//! invite alone; then dials the discovered address **through the `DiscoveryPolicy`**
//! (the only thing that decides what to dial; the net Actor never auto-dials) and joins
//! the MLS group, with **no hard-coded server address**. Then, now a member, it
//! discovers the inviter again under the steady-state (secret) **member namespace**,
//! demonstrating the bootstrap → steady-state discovery transition.

use std::time::Duration;

use catcoms_discovery::{Candidate, DiscoveryPolicy, PolicyConfig, Source};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_net::{
    build_memory_rendezvous_swarm, build_memory_swarm, phase0_peer_id, run_rendezvous,
    validate_rendezvous_addrs, MeshService,
};
use catcoms_rt::{ManualClock, MeshTransport, OsCryptoRng, PeerId, SystemClock, TransportEvent};
use catcoms_sync::{join_namespace, request_join, ChannelSync};
use libp2p::Multiaddr;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use tokio::time::timeout;

/// Block until `mesh` reports a connection to `target` (a phase-0 peer id).
async fn wait_connected(mesh: &MeshService, target: PeerId) {
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
    .expect("did not connect to the target peer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn joiner_discovers_inviter_via_join_ns_then_joins_with_no_hardcoded_addr() {
    catcoms_log::init_test();

    // --- Zero-knowledge rendezvous server ---
    let rz_addr: Multiaddr = "/memory/660066".parse().unwrap();
    let mut rz = build_memory_rendezvous_swarm();
    let rz_id = *rz.local_peer_id();
    rz.listen_on(rz_addr.clone()).unwrap();
    let rz_task = tokio::spawn(run_rendezvous(rz));
    let rz_dial: Multiaddr = format!("{rz_addr}/p2p/{rz_id}").parse().unwrap();

    // --- Alice (inviter): listens directly, founds a group, connects to the rendezvous,
    //     advertises her address, and registers under BOTH the pre-join join_ns and her
    //     steady-state member namespace. ---
    let a_addr: Multiaddr = "/memory/660067".parse().unwrap();
    let mut a_swarm = build_memory_swarm();
    let a_id = *a_swarm.local_peer_id();
    a_swarm.listen_on(a_addr.clone()).unwrap();
    a_swarm.dial(rz_dial.clone()).unwrap();
    let a_mesh = MeshService::spawn(a_swarm);
    let a_peer = a_mesh.local_peer();
    wait_connected(&a_mesh, phase0_peer_id(&rz_id)).await;

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

    // Mint an invite carrying ONLY the rendezvous address (no direct server bootstrap).
    let nonce = [9u8; 16];
    let invite = asy
        .mint_invite_with_rendezvous(nonce, u64::MAX, vec![], vec![rz_dial.to_string()])
        .unwrap();
    let group_id = invite.group_id.clone();

    // Register under join_ns (pre-join, invite-derivable) and the member namespace.
    let join_ns = join_namespace(&group_id, &nonce, &rz_id.to_bytes());
    let member_ns = asy.rendezvous_namespaces(&rz_id.to_bytes())[0].clone();
    let a_route: Multiaddr = format!("{a_addr}/p2p/{a_id}").parse().unwrap();
    asy.transport().add_external_address(a_route).await.unwrap();
    asy.transport()
        .rendezvous_register(&join_ns, rz_id)
        .await
        .unwrap();
    asy.transport()
        .rendezvous_register(&member_ns, rz_id)
        .await
        .unwrap();
    for _ in 0..2 {
        timeout(Duration::from_secs(20), asy.transport().next_registered())
            .await
            .expect("Alice registration timed out")
            .expect("Alice actor stopped");
    }

    // Alice serves in the background (admits the joiner, answers requests).
    let alice_loop = tokio::spawn(async move { while asy.run_once().await.unwrap_or(false) {} });

    // --- Bob (joiner): knows ONLY the rendezvous address (plus the pasted invite). ---
    let b_mesh = MeshService::new_memory(None, std::slice::from_ref(&rz_dial)).unwrap();
    wait_connected(&b_mesh, phase0_peer_id(&rz_id)).await;

    // Validate the invite's rendezvous set, derive the SAME join_ns, and discover Alice.
    let targets = validate_rendezvous_addrs(&invite.rendezvous).unwrap();
    assert_eq!(targets.len(), 1);
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
        "Bob discovered Alice under join_ns"
    );
    assert!(!discovered.addresses.is_empty());

    // The DiscoveryPolicy is the ONLY thing that decides what to dial (no auto-dial).
    let mut policy = DiscoveryPolicy::with_config(PolicyConfig::default());
    let candidate = Candidate {
        peer: discovered.peer.to_bytes(),
        addresses: discovered.addresses.iter().map(|a| a.to_string()).collect(),
        source: Source::Rendezvous(rz_id.to_bytes()),
        freshness: catcoms_discovery::FreshnessPrincipal::Transport(discovered.peer.to_bytes()),
        seq: 1,
        tag_verified: false,
    };
    let clock = ManualClock::new(0);
    let mut rng = ChaCha20Rng::seed_from_u64(7);
    let plan = policy.plan(vec![candidate], 2, &clock, &mut rng);
    assert!(
        !plan.is_empty(),
        "the policy offers the discovered inviter to dial"
    );

    // Dial the planned address; Bob never held a hard-coded server address.
    for addr in &plan[0].addresses {
        b_mesh.dial(addr.parse().unwrap()).await.unwrap();
    }
    wait_connected(&b_mesh, a_peer).await;

    // Join the MLS group over the discovered+dialed connection.
    let bob = MlsDevice::generate().unwrap();
    let (bob_group, bob_routing) = timeout(
        Duration::from_secs(20),
        request_join(&b_mesh, a_peer, &bob, &invite),
    )
    .await
    .expect("join timed out")
    .expect("Bob joined the server discovered via rendezvous");
    assert_eq!(bob_group.epoch(), 1);
    assert!(bob_group.contains_device(&bob.device_id()));

    // --- Now a member, Bob discovers Alice again under the steady-state member namespace,
    //     derived from the routing state transferred at join. ---
    let bsy = ChannelSync::new_joined(
        b_mesh,
        bob_group,
        bob,
        OsCryptoRng,
        Box::new(SystemClock),
        bob_routing,
    );
    let bob_member_ns = bsy.rendezvous_namespaces(&rz_id.to_bytes())[0].clone();
    assert_eq!(
        bob_member_ns, member_ns,
        "the joiner derives the same member namespace from the transferred routing state"
    );
    bsy.transport()
        .rendezvous_discover(&bob_member_ns, rz_id)
        .await
        .unwrap();
    let member = timeout(Duration::from_secs(20), bsy.transport().next_discovered())
        .await
        .expect("member discovery timed out")
        .expect("Bob actor stopped");
    assert_eq!(
        phase0_peer_id(&member.peer),
        a_peer,
        "Bob discovers a fellow member (Alice) under the secret member namespace"
    );

    alice_loop.abort();
    rz_task.abort();
}
