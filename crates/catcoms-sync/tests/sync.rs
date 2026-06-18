//! Two members synchronizing a channel over the in-memory mesh transport:
//! live gossip convergence and request/response catch-up. Deterministic (no real
//! sockets, seeded RNG), so it is reliable in CI; the same `ChannelSync` runs
//! unchanged over the libp2p mesh.

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ReadDoc, ROOT};
use catcoms_mls::{InviteLedger, MlsDevice, ServerGroup};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_sync::ChannelSync;
use catcoms_wire::DocType;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const CHANNEL: u128 = 1;

fn rng(seed: u64) -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(seed)
}

fn get_str(sync: &ChannelSync<MemNetwork, ChaCha20Rng>, key: &str) -> Option<String> {
    let doc: &AutoCommit = sync.doc(DocType::Channel, CHANNEL)?.doc();
    doc.get(ROOT, key)
        .unwrap()
        .map(|(v, _)| v.into_string().unwrap())
}

/// Build a two-member group and a `ChannelSync` for each over a shared hub.
fn pair() -> (
    ChannelSync<MemNetwork, ChaCha20Rng>,
    ChannelSync<MemNetwork, ChaCha20Rng>,
    PeerId,
    PeerId,
) {
    let alice = MlsDevice::generate().unwrap();
    let mut alice_group = ServerGroup::create(&alice).unwrap();
    let bob = MlsDevice::generate().unwrap();
    let mut ledger = InviteLedger::new();
    let token = alice_group
        .mint_invite(&alice, [1u8; 16], 10_000, vec![])
        .unwrap();
    let kp = bob
        .key_package_for_invite(&alice_group.group_id(), token.invite_nonce)
        .unwrap();
    let welcome = alice_group
        .add_member_via_invite(&alice, kp, &token, &mut ledger, 1_000)
        .unwrap();
    let bob_group = ServerGroup::join(&bob, &welcome).unwrap();

    let hub = Hub::new();
    let alice_peer = PeerId::from_u64(1);
    let bob_peer = PeerId::from_u64(2);
    let asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(10),
        Box::new(ManualClock::new(1_000)),
    );
    let bsy = ChannelSync::new(
        hub.join(bob_peer),
        bob_group,
        bob,
        rng(20),
        Box::new(ManualClock::new(1_000)),
    );
    (asy, bsy, alice_peer, bob_peer)
}

#[tokio::test]
async fn live_post_replicates_over_gossip() {
    catcoms_log::init_test();
    let (mut asy, mut bsy, _, _) = pair();

    asy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    bsy.open_channel(DocType::Channel, CHANNEL).await.unwrap();

    asy.post(DocType::Channel, CHANNEL, |d| {
        d.put(ROOT, "msg", "hi over the mesh")
    })
    .await
    .unwrap();

    // Bob processes the one gossiped op and converges.
    assert!(bsy.run_once().await.unwrap());
    assert_eq!(get_str(&bsy, "msg").as_deref(), Some("hi over the mesh"));
}

#[tokio::test]
async fn bidirectional_gossip_converges() {
    catcoms_log::init_test();
    let (mut asy, mut bsy, _, _) = pair();
    asy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    bsy.open_channel(DocType::Channel, CHANNEL).await.unwrap();

    asy.post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "from_a", "a"))
        .await
        .unwrap();
    bsy.post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "from_b", "b"))
        .await
        .unwrap();

    // Each ingests the other's op.
    assert!(bsy.run_once().await.unwrap());
    assert!(asy.run_once().await.unwrap());

    for s in [&asy, &bsy] {
        assert_eq!(get_str(s, "from_a").as_deref(), Some("a"));
        assert_eq!(get_str(s, "from_b").as_deref(), Some("b"));
    }
}

#[tokio::test]
async fn fresh_device_joins_via_invite_over_the_transport() {
    catcoms_log::init_test();
    // Alice founds a group and runs a sync node that can admit members.
    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let hub = Hub::new();
    let alice_peer = PeerId::from_u64(1);
    let bob_peer = PeerId::from_u64(2);
    let mut asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(1),
        Box::new(ManualClock::new(1_000)),
    );

    // Alice mints an invite; Bob (a brand-new device with no group) redeems it
    // over the transport.
    let invite = asy.mint_invite([7u8; 16], 10_000, vec![]).unwrap();
    let bob = MlsDevice::generate().unwrap();
    let bob_net = hub.join(bob_peer);

    let (joined, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &invite),
        asy.run_once(),
    );
    let bob_group = joined.expect("bob joined");
    assert_eq!(bob_group.epoch(), 1);
    assert!(bob_group.contains_device(&bob.device_id()));

    // Bob is now a member: he and Alice converge on a channel.
    let mut bsy = ChannelSync::new(
        bob_net,
        bob_group,
        bob,
        rng(2),
        Box::new(ManualClock::new(1_000)),
    );
    asy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    bsy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    asy.post(DocType::Channel, CHANNEL, |d| {
        d.put(ROOT, "welcome", "you're in")
    })
    .await
    .unwrap();
    assert!(bsy.run_once().await.unwrap());
    assert_eq!(get_str(&bsy, "welcome").as_deref(), Some("you're in"));
}

#[tokio::test]
async fn joining_via_a_non_inviter_node_is_rejected() {
    catcoms_log::init_test();
    // Alice founds group A and issues an invite.
    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let hub = Hub::new();
    let alice_peer = PeerId::from_u64(1);
    let asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(1),
        Box::new(ManualClock::new(1_000)),
    );
    let invite = asy.mint_invite([7u8; 16], 10_000, vec![]).unwrap();

    // Carol runs an unrelated group/node.
    let carol = MlsDevice::generate().unwrap();
    let carol_group = ServerGroup::create(&carol).unwrap();
    let carol_peer = PeerId::from_u64(2);
    let mut csy = ChannelSync::new(
        hub.join(carol_peer),
        carol_group,
        carol,
        rng(2),
        Box::new(ManualClock::new(1_000)),
    );

    // Bob tries to redeem Alice's invite by talking to Carol's node — rejected.
    let bob = MlsDevice::generate().unwrap();
    let bob_net = hub.join(PeerId::from_u64(3));
    let (joined, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, carol_peer, &bob, &invite),
        csy.run_once(),
    );
    assert!(matches!(joined, Err(catcoms_sync::SyncError::JoinRejected)));
}

#[tokio::test]
async fn spent_invite_cannot_be_reused_for_a_second_join() {
    catcoms_log::init_test();
    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let hub = Hub::new();
    let alice_peer = PeerId::from_u64(1);
    let mut asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(1),
        Box::new(ManualClock::new(1_000)),
    );
    let invite = asy.mint_invite([7u8; 16], 10_000, vec![]).unwrap();

    // First join succeeds.
    let bob = MlsDevice::generate().unwrap();
    let bob_net = hub.join(PeerId::from_u64(2));
    let (first, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &invite),
        asy.run_once(),
    );
    assert!(first.is_ok());

    // Replaying the same invite for a different device is rejected (single use).
    let mallory = MlsDevice::generate().unwrap();
    let mallory_net = hub.join(PeerId::from_u64(3));
    let (second, _) = tokio::join!(
        catcoms_sync::request_join(&mallory_net, alice_peer, &mallory, &invite),
        asy.run_once(),
    );
    assert!(matches!(second, Err(catcoms_sync::SyncError::JoinRejected)));
}

#[tokio::test]
async fn catch_up_transfers_history_over_request_response() {
    catcoms_log::init_test();
    let (mut asy, mut bsy, alice_peer, _) = pair();
    asy.open_channel(DocType::Channel, CHANNEL).await.unwrap();

    // Alice writes history Bob has never seen.
    asy.post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "h1", "one"))
        .await
        .unwrap();
    asy.post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "h2", "two"))
        .await
        .unwrap();

    // Bob requests catch-up; Alice serves it (drive both concurrently).
    let (result, _) = tokio::join!(
        bsy.request_catchup(alice_peer, DocType::Channel, CHANNEL),
        asy.run_once(),
    );
    assert_eq!(result.unwrap(), 2);
    assert_eq!(get_str(&bsy, "h1").as_deref(), Some("one"));
    assert_eq!(get_str(&bsy, "h2").as_deref(), Some("two"));
}
