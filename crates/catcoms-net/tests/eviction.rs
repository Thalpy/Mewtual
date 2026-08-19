//! **P6**: the eviction primitive, exercised over real TCP loopback sockets.
//!
//! Removing a member rotates the routing secret, which takes its keys. Before this it took
//! nothing else: the ex-member's established connections and anything scoped to them (a granted
//! circuit reservation, most importantly, once rung 2 puts a member on other members' traffic
//! path) survived removal indefinitely. That is not a removal, and it is why no configuration
//! assertion is enough here. What is worth proving is behavioural:
//!
//! 1. an evicted peer is **actually disconnected**, and **cannot** reconnect;
//! 2. a peer evicted while offline is refused **on arrival**, never admitted once;
//! 3. a peer this node uses as **infrastructure** cannot be evicted at all, because the peer id
//!    an eviction names is a value a removed member chose and can point at anybody;
//! 4. lifting an eviction actually lets the peer back in, which is what a re-invite needs.
//!
//! The interesting assertions are negative ("no connection appears"), which is the shape of test
//! that passes for the wrong reason when a window is too short or a node was never listening. So
//! every negative assertion is measured **together with** a positive control in a single drain of
//! the event stream: one loop over the window collects every `PeerConnected` into a set, and the
//! assertions are made against that set afterwards. Collecting first is what stops a control loop
//! from swallowing the very event the negative assertion is looking for and then waiting out the
//! window on a stream that can no longer carry it, which is a test that asserts the bug away.
//!
//! Nothing here asserts on the *evicted* node's own view. The denial happens on the evictor, and
//! what a refused dialler observes (a closed connection, a failed upgrade, a silent timeout)
//! depends on where in the upgrade the refusal lands; asserting on it would be a timing
//! assumption, not a property.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use catcoms_net::{phase0_peer_id, MeshService};
use catcoms_rt::{MeshTransport, PeerId, TransportEvent};
use libp2p::Multiaddr;

/// How long connections are given to appear. Generous: it bounds how long the negative
/// assertions wait, and every one of them is paired with a positive control drawn from the same
/// drain, so a longer window only makes the test slower, never weaker.
const WINDOW: Duration = Duration::from_secs(10);

/// Start a node listening on an ephemeral loopback port and return it with its dialable address.
async fn listener() -> (Arc<MeshService>, Multiaddr, libp2p::PeerId) {
    let (node, id) =
        MeshService::new_tcp(Some("/ip4/127.0.0.1/tcp/0".parse().unwrap()), &[]).unwrap();
    let node = Arc::new(node);
    let addr = tokio::time::timeout(WINDOW, node.next_listen_addr())
        .await
        .expect("listen-addr timeout")
        .expect("bound a listen address")
        .with(libp2p::multiaddr::Protocol::P2p(id));
    (node, addr, id)
}

/// Drain `node`'s events for the whole window, collecting **every** peer that connects.
///
/// One drain, no early return: an early return is what lets a positive control consume the
/// negative assertion's evidence. `until` lets a caller stop as soon as the question is answered
/// in the affirmative, which is only ever used where waiting longer cannot change the answer.
async fn connected_within(
    node: &MeshService,
    until: impl Fn(&HashSet<PeerId>) -> bool,
) -> HashSet<PeerId> {
    let mut seen = HashSet::new();
    let _ = tokio::time::timeout(WINDOW, async {
        loop {
            match node.next_event().await {
                Some(TransportEvent::PeerConnected(p)) => {
                    seen.insert(p);
                    if until(&seen) {
                        return;
                    }
                }
                Some(_) => continue,
                None => panic!("mesh actor stopped"),
            }
        }
    })
    .await;
    seen
}

/// Drive `node`'s events until `want` disconnects, or give up after `WINDOW`.
async fn disconnects(node: &MeshService, want: PeerId) -> bool {
    tokio::time::timeout(WINDOW, async {
        loop {
            match node.next_event().await {
                Some(TransportEvent::PeerDisconnected(p)) if p == want => return,
                Some(_) => continue,
                None => panic!("mesh actor stopped"),
            }
        }
    })
    .await
    .is_ok()
}

/// An evicted peer is disconnected, and dialling again does not get it back.
///
/// The positive control is the *same* peer over the *same* socket path before the eviction: it
/// connects inside `WINDOW`, so a later failure to connect inside the same window is the deny
/// binding and not the test being impatient.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_evicted_peer_is_disconnected_and_cannot_reconnect() {
    let (evictor, addr, _evictor_id) = listener().await;

    let (member, member_id) = MeshService::new_tcp(None, std::slice::from_ref(&addr)).unwrap();
    let member_peer = phase0_peer_id(&member_id);

    // Control: without an eviction in force this peer connects, inside the same window the
    // negative assertion below uses.
    let seen = connected_within(&evictor, |s| s.contains(&member_peer)).await;
    assert!(
        seen.contains(&member_peer),
        "control: an un-evicted peer must connect (if this fails the window is too short, \
         and every negative assertion below is meaningless)"
    );

    // Evict it. This is what an applied Remove commit drives.
    evictor.evict_peer(member_peer).await.unwrap();

    assert!(
        disconnects(&evictor, member_peer).await,
        "an evicted peer's established connection must be severed, not left running"
    );

    // …and it stays out. Redial repeatedly: one refused dial could be a race, a sustained
    // inability to get back in is the deny.
    for _ in 0..5 {
        let _ = member.dial_addr(&addr.to_string()).await;
    }
    let seen = connected_within(&evictor, |_| false).await;
    assert!(
        !seen.contains(&member_peer),
        "an evicted peer must not be able to reconnect"
    );
}

/// Eviction also binds for a peer that was **never connected** when it was evicted, and it is
/// targeted: a bystander dialling the same listener in the same window still gets in.
///
/// Both are read out of **one** drain. Asserting the control first over its own drain is how the
/// earlier version of this test could pass vacuously: had the block failed and the stranger
/// connected first, the control's loop would have swallowed that `PeerConnected` and the
/// negative assertion would then have waited out the window on a stream that could never carry
/// it again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_evicted_while_offline_is_refused_when_it_shows_up() {
    let (evictor, addr, _evictor_id) = listener().await;

    // Evict a peer we have never seen, so nothing but the hashed id is known about it.
    let (stranger, stranger_id) = MeshService::new_tcp(None, &[]).unwrap();
    let stranger_peer = phase0_peer_id(&stranger_id);
    evictor.evict_peer(stranger_peer).await.unwrap();

    // A bystander that was never evicted, dialling the same listener.
    let (_bystander, bystander_id) =
        MeshService::new_tcp(None, std::slice::from_ref(&addr)).unwrap();
    let bystander_peer = phase0_peer_id(&bystander_id);

    for _ in 0..5 {
        let _ = stranger.dial_addr(&addr.to_string()).await;
    }

    // One drain over the whole window, so the control cannot consume the evidence. It runs to the
    // end of the window rather than stopping at the bystander, because the stranger's connection
    // (if the deny failed) may well arrive after it.
    let seen = connected_within(&evictor, |_| false).await;
    assert!(
        seen.contains(&bystander_peer),
        "control: a peer that was never evicted must still connect"
    );
    assert!(
        !seen.contains(&stranger_peer),
        "a peer evicted while offline must be refused on arrival, not admitted once"
    );
}

/// **F1, check 3.** A peer this node dials as infrastructure or bootstrap cannot be evicted.
///
/// The peer id an eviction names comes out of a removed member's self-signed record, and nothing
/// binds that value to its signer, so a member with a modified client can name the group's relay,
/// get itself removed in the ordinary way, and take NAT traversal down for everyone. What a peer
/// *is* to this node is decided locally by what this node was told to dial, and no record from
/// the wire may override that.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn infrastructure_this_node_dials_cannot_be_evicted() {
    let (_infra, infra_addr, infra_id) = listener().await;
    let infra_peer = phase0_peer_id(&infra_id);

    // A node constructed to dial that address: it is this node's bootstrap, chosen locally.
    let (client, _client_id) =
        MeshService::new_tcp(None, std::slice::from_ref(&infra_addr)).unwrap();

    // A hostile eviction naming it, exactly as a removed member's forged record would produce.
    client.evict_peer(infra_peer).await.unwrap();
    // Redial, so the test does not merely depend on the first connection having raced ahead.
    for _ in 0..5 {
        let _ = client.dial_addr(&infra_addr.to_string()).await;
    }

    let seen = connected_within(&client, |s| s.contains(&infra_peer)).await;
    assert!(
        seen.contains(&infra_peer),
        "a bootstrap/infrastructure peer must survive an eviction aimed at it"
    );
}

/// **F2.** Lifting an eviction lets the peer back in, which is what re-inviting a removed member
/// depends on. Node identities are stable across restarts, so without this the re-invited
/// member's join is refused at the connection handler and times out undiagnosably.
///
/// The negative half (still out before the lift) and the positive half (back in after it) are
/// separate drains here, and that is safe in this direction: the negative assertion runs first,
/// so it cannot consume evidence the positive one needs, and the positive assertion failing is
/// the failure mode being tested.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lifting_an_eviction_lets_a_re_invited_peer_back_in() {
    let (evictor, addr, _evictor_id) = listener().await;

    let (member, member_id) = MeshService::new_tcp(None, &[]).unwrap();
    let member_peer = phase0_peer_id(&member_id);
    evictor.evict_peer(member_peer).await.unwrap();

    for _ in 0..5 {
        let _ = member.dial_addr(&addr.to_string()).await;
    }
    let seen = connected_within(&evictor, |_| false).await;
    assert!(
        !seen.contains(&member_peer),
        "precondition: the eviction is in force"
    );

    // Re-invited: the membership layer lifts the deny.
    evictor.unevict_peer(member_peer).await.unwrap();
    for _ in 0..5 {
        let _ = member.dial_addr(&addr.to_string()).await;
    }
    let seen = connected_within(&evictor, |s| s.contains(&member_peer)).await;
    assert!(
        seen.contains(&member_peer),
        "a re-invited member must be able to connect again, or the re-invite silently times out"
    );
}
