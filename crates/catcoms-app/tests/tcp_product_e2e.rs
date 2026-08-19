//! Product-layer end-to-end over **real OS sockets**: the one thing `product_e2e.rs` cannot do.
//!
//! Everything else in this crate's product suite runs on the in-memory mesh, which is faster,
//! deterministic and sufficient. It has one blind spot that happens to sit exactly on top of a
//! feature that shipped broken: `Hub` models no connections, so it never emits `PeerConnected`
//! or `PeerDisconnected`, and **presence is defined in terms of those two events**. Over the
//! memory transport a node that closes its app stays lit in everyone's roster forever, which
//! would make an in-memory "the dot goes out" test assert nothing at all.
//!
//! So this file founds a server on a real TCP listener, joins it from a second real libp2p node,
//! runs the product's own discovery tick until presence lights up, and then closes the second
//! node's app for real and watches the dot go out. It drives `spawn`/`ServerActor`/`AppEvent`
//! like the rest of the product suite, not the sync layer.
//!
//! What is deliberately *not* here: the phase-9g cross-session re-dial over real sockets.
//! `publish_self_record` strips non-routable addresses, and every address available in CI is
//! loopback, so a record published here carries no dialable address by design and the address
//! cache has nothing to hold. The re-dial's plan/policy half is covered in `product_e2e.rs`; the
//! socket half needs a routable address and therefore a live host, not a test.

use std::sync::Arc;
use std::time::Duration;

use catcoms_app::{channel_id, spawn, AppEvent, Server, ServerActor};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_net::MeshService;
use catcoms_rt::{MeshTransport, OsCryptoRng, SystemClock, TransportEvent};
use libp2p::Multiaddr;
use tokio::time::timeout;

/// Ceiling on every wait. Real sockets and a real swarm, so this is loose enough that only a
/// genuinely dead path reaches it, and bounded so that a dead path fails rather than wedges CI.
const WAIT: Duration = Duration::from_secs(60);

/// This device's fingerprint, read off the roster the way the UI reads it.
async fn my_fp(actor: &ServerActor) -> String {
    actor
        .members()
        .await
        .into_iter()
        .find(|m| m.is_self)
        .expect("this device is in its own roster")
        .fingerprint
}

/// One discovery tick, then a query to prove it finished. Serialising the two nodes' passes this
/// way is required, not stylistic: an actor running a PEX pass is blocked on its own request and
/// is therefore not in `sync_once` to answer anyone else's, so two simultaneous passes wait each
/// other out to the per-request deadline.
async fn discovery_pass(actor: &ServerActor) {
    let _ = actor.drive_discovery().await;
    let _ = actor.member_count().await;
}

/// A member's presence dot follows a real connection: it lights when the two nodes have
/// exchanged records over a live link, and it goes out when that node closes its app.
///
/// Both halves shipped broken. The lighting half was dead because nothing in the product ever
/// published or requested a peer record, so `connected_member_fingerprints` read an empty map;
/// the going-out half was dead earlier because `PeerDisconnected` was dropped on the floor, so
/// the live set only ever grew. A test that cannot disconnect a peer cannot see the second bug,
/// which is why this one uses sockets.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn presence_follows_a_real_connection_and_goes_dark_when_the_peer_closes_its_app() {
    catcoms_log::init_test();
    let general = channel_id("general");

    // --- Alice founds on a real, ephemeral TCP loopback port ---
    let (a_mesh, _a_id) = MeshService::new_tcp(Some("/ip4/127.0.0.1/tcp/0".parse().unwrap()), &[])
        .expect("bind a TCP listener");
    let a_addr: Multiaddr = timeout(WAIT, a_mesh.next_listen_addr())
        .await
        .expect("listen-addr timeout")
        .expect("Alice bound a TCP address");
    let a_peer = a_mesh.local_peer();

    let mut alice = Server::found(
        a_mesh,
        MlsDevice::generate().expect("a fresh MLS provider per device"),
        OsCryptoRng,
        Box::new(SystemClock),
        "alice",
    )
    .expect("found");
    alice.subscribe_control().await.expect("subscribe control");
    let (alice, mut alice_events, alice_task) = spawn(alice);
    alice.open_channel(general).await;
    let alice_fp = my_fp(&alice).await;

    let invite = alice
        .mint_invite([7u8; 16], u64::MAX, vec![a_addr.to_string()])
        .await
        .expect("the owner may mint an invite");
    let invite = InviteToken::decode(&invite).expect("the minted invite decodes");

    // --- Bob dials the address out of the invite and joins over the wire ---
    let (b_mesh, _b_id) =
        MeshService::new_tcp(None, std::slice::from_ref(&a_addr)).expect("build the joiner node");
    let b_mesh = Arc::new(b_mesh);
    timeout(WAIT, async {
        loop {
            match b_mesh.next_event().await {
                Some(TransportEvent::PeerConnected(p)) if p == a_peer => break,
                Some(_) => continue,
                None => panic!("the joiner's transport closed before it connected"),
            }
        }
    })
    .await
    .expect("the joiner did not connect to the founder over TCP");
    let b_mesh = Arc::try_unwrap(b_mesh).expect("sole owner once the connect wait is done");

    let bob = Server::join(
        b_mesh,
        MlsDevice::generate().expect("a fresh MLS provider per device"),
        OsCryptoRng,
        Box::new(SystemClock),
        "bob",
        a_peer,
        &invite,
    )
    .await
    .expect("join over real sockets");
    let (bob, mut bob_events, bob_task) = spawn(bob);
    bob.open_channel(general).await;
    bob.catch_up(a_peer, general).await;
    let bob_fp = my_fp(&bob).await;
    assert_eq!(
        alice.member_count().await,
        2,
        "the roster grew over the wire"
    );

    // The whole product over a socket, in one line, before presence is even considered: if this
    // does not converge nothing below means anything.
    alice.send_message(general, "hello over tcp").await;
    timeout(WAIT, async {
        loop {
            if bob
                .messages(general)
                .await
                .iter()
                .any(|m| m.text == "hello over tcp")
            {
                return;
            }
            let _ = timeout(Duration::from_millis(20), bob_events.recv()).await;
        }
    })
    .await
    .expect("the joiner never received the founder's message over TCP");

    // --- presence lights up ---
    //
    // Both nodes publish a record with the address they are actually reachable on. Loopback is
    // stripped as non-routable, so these records carry no dialable address at all; that is the
    // documented LAN-only shape, and presence is unaffected because it matches on the `peer_id`
    // each member signed into its **own** record, not on an address.
    alice
        .publish_self_record(vec![a_addr.to_string()], 65_536)
        .await;
    bob.publish_self_record(Vec::new(), 65_536).await;
    discovery_pass(&alice).await;
    discovery_pass(&bob).await;

    assert_eq!(
        alice.online_members().await,
        vec![bob_fp.clone()],
        "the founder's roster lights the joiner's dot"
    );
    assert_eq!(bob.online_members().await, vec![alice_fp.clone()]);

    // The UI renders presence off the event, not off a poll, so the event has to fire too.
    timeout(WAIT, async {
        loop {
            match alice_events.recv().await {
                Some(AppEvent::ConnectivityChanged { online }) if online.contains(&bob_fp) => {
                    return
                }
                Some(_) => continue,
                None => panic!("the founder's actor closed"),
            }
        }
    })
    .await
    .expect("no ConnectivityChanged carried the joiner's fingerprint");

    // --- and goes out when the peer closes its app ---
    //
    // Shutting the actor down drops the `Server`, which drops the `MeshService`, which closes
    // the swarm and with it the TCP connection. The founder learns about it the only way anyone
    // ever does: a `PeerDisconnected` off a real socket.
    bob.shutdown().await;
    let _ = bob_task.await;

    timeout(WAIT, async {
        loop {
            match alice_events.recv().await {
                Some(AppEvent::ConnectivityChanged { online }) if online.is_empty() => return,
                Some(_) => continue,
                None => panic!("the founder's actor closed"),
            }
        }
    })
    .await
    .expect("the departed member's dot never went out");
    assert!(
        alice.online_members().await.is_empty(),
        "and the query agrees with the event"
    );
    assert_eq!(
        alice.member_count().await,
        2,
        "going offline is not leaving: the roster is unchanged, only the dot"
    );

    alice.shutdown().await;
    let _ = alice_task.await;
}
