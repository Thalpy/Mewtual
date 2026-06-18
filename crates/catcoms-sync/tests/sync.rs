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
        .unwrap()
        .welcome;
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
async fn multi_member_join_propagates_to_existing_members() {
    catcoms_log::init_test();
    let hub = Hub::new();

    // Alice founds the group (she is leaf 0 = the designated committer).
    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let alice_peer = PeerId::from_u64(1);
    let mut asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(1),
        Box::new(ManualClock::new(1_000)),
    );
    asy.subscribe_control().await.unwrap();

    // Bob joins via Alice.
    let bob = MlsDevice::generate().unwrap();
    let invite_b = asy.mint_invite([1u8; 16], 10_000, vec![]).unwrap();
    let bob_net = hub.join(PeerId::from_u64(2));
    let (bob_joined, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &invite_b),
        asy.run_once(),
    );
    let mut bsy = ChannelSync::new(
        bob_net,
        bob_joined.unwrap(),
        bob,
        rng(2),
        Box::new(ManualClock::new(1_000)),
    );
    bsy.subscribe_control().await.unwrap();
    assert_eq!(asy.epoch(), 1);
    assert_eq!(bsy.epoch(), 1);

    // Carol joins via Alice. Alice broadcasts the Add commit on the control topic.
    let carol = MlsDevice::generate().unwrap();
    let invite_c = asy.mint_invite([2u8; 16], 10_000, vec![]).unwrap();
    let carol_net = hub.join(PeerId::from_u64(3));
    let (carol_joined, _) = tokio::join!(
        catcoms_sync::request_join(&carol_net, alice_peer, &carol, &invite_c),
        asy.run_once(),
    );
    let mut csy = ChannelSync::new(
        carol_net,
        carol_joined.unwrap(),
        carol,
        rng(3),
        Box::new(ManualClock::new(1_000)),
    );
    csy.subscribe_control().await.unwrap();
    assert_eq!(asy.epoch(), 2);
    assert_eq!(csy.epoch(), 2);

    // Bob — who admitted no one — receives the broadcast commit and advances.
    assert!(bsy.run_once().await.unwrap());
    assert_eq!(bsy.epoch(), 2, "Bob must learn of Carol's join");

    // All three at epoch 2: Carol posts and Bob can decrypt it.
    asy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    bsy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    csy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    csy.post(DocType::Channel, CHANNEL, |d| {
        d.put(ROOT, "hi", "from carol")
    })
    .await
    .unwrap();
    assert!(bsy.run_once().await.unwrap());
    assert_eq!(get_str(&bsy, "hi").as_deref(), Some("from carol"));
}

#[tokio::test]
async fn a_non_committer_cannot_admit() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let alice_peer = PeerId::from_u64(1);
    let mut asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(1),
        Box::new(ManualClock::new(1_000)),
    );

    // Bob joins (member, leaf 1 — NOT the designated committer).
    let bob = MlsDevice::generate().unwrap();
    let invite_b = asy.mint_invite([1u8; 16], 10_000, vec![]).unwrap();
    let bob_peer = PeerId::from_u64(2);
    let bob_net = hub.join(bob_peer);
    let (bob_joined, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &invite_b),
        asy.run_once(),
    );
    let mut bsy = ChannelSync::new(
        bob_net,
        bob_joined.unwrap(),
        bob,
        rng(2),
        Box::new(ManualClock::new(1_000)),
    );

    // Bob invites Carol and tries to admit her — refused, Bob is not the committer.
    let invite_c = bsy.mint_invite([2u8; 16], 10_000, vec![]).unwrap();
    let carol = MlsDevice::generate().unwrap();
    let carol_net = hub.join(PeerId::from_u64(3));
    let (carol_joined, _) = tokio::join!(
        catcoms_sync::request_join(&carol_net, bob_peer, &carol, &invite_c),
        bsy.run_once(),
    );
    assert!(matches!(
        carol_joined,
        Err(catcoms_sync::SyncError::JoinRejected)
    ));
}

/// 6d-1b: an op sealed under the epoch just before a membership change still
/// decrypts after the holder advances, via the bounded past-epoch key window —
/// instead of being silently dropped as `EpochUnavailable`.
#[tokio::test]
async fn op_sealed_before_an_epoch_advance_still_opens() {
    catcoms_log::init_test();
    let hub = Hub::new();

    // Alice (leaf 0, the committer) and Bob, both at epoch 1 with #general open.
    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let alice_peer = PeerId::from_u64(1);
    let mut asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(1),
        Box::new(ManualClock::new(1_000)),
    );
    asy.subscribe_control().await.unwrap();

    let bob = MlsDevice::generate().unwrap();
    let invite_b = asy.mint_invite([1u8; 16], 10_000, vec![]).unwrap();
    let bob_peer = PeerId::from_u64(2);
    let bob_net = hub.join(bob_peer);
    let (bob_joined, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &invite_b),
        asy.run_once(),
    );
    let mut bsy = ChannelSync::new(
        bob_net,
        bob_joined.unwrap(),
        bob,
        rng(2),
        Box::new(ManualClock::new(1_000)),
    );
    asy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    bsy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    assert_eq!(asy.epoch(), 1);
    assert_eq!(bsy.epoch(), 1);

    // Carol joins via Alice. Alice snapshots her epoch-1 #general key, admits
    // Carol, and advances to epoch 2.
    let carol = MlsDevice::generate().unwrap();
    let invite_c = asy.mint_invite([2u8; 16], 10_000, vec![]).unwrap();
    let carol_net = hub.join(PeerId::from_u64(3));
    let (carol_joined, _) = tokio::join!(
        catcoms_sync::request_join(&carol_net, alice_peer, &carol, &invite_c),
        asy.run_once(),
    );
    carol_joined.unwrap();
    assert_eq!(asy.epoch(), 2);

    // Bob never processed Carol's commit, so he is still at epoch 1 and seals his
    // op under epoch 1. It is delivered into Alice's (epoch-2) inbox.
    assert_eq!(bsy.epoch(), 1);
    bsy.post(DocType::Channel, CHANNEL, |d| {
        d.put(ROOT, "late", "from epoch 1")
    })
    .await
    .unwrap();

    // Alice ingests the epoch-1 op while at epoch 2 — recovered via the retained
    // past-epoch key rather than dropped.
    assert!(asy.run_once().await.unwrap());
    assert_eq!(get_str(&asy, "late").as_deref(), Some("from epoch 1"));
    assert_eq!(asy.stats().ops_recovered_past_epoch, 1);
}

/// 6d-1b: when the past-epoch key window has evicted the needed epoch, an op
/// sealed under it is not silently lost — it is counted dropped and a document
/// catch-up is queued to reconcile it.
#[tokio::test]
async fn op_past_the_key_window_falls_back_to_doc_catchup() {
    catcoms_log::init_test();
    let hub = Hub::new();

    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let alice_peer = PeerId::from_u64(1);
    let mut asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(1),
        Box::new(ManualClock::new(1_000)),
    );
    // Retain NO past epochs, so the window evicts the old key on every advance.
    asy.set_config(catcoms_sync::SyncConfig {
        max_past_epochs: 0,
        ..catcoms_sync::SyncConfig::default()
    });
    asy.subscribe_control().await.unwrap();

    let bob = MlsDevice::generate().unwrap();
    let invite_b = asy.mint_invite([1u8; 16], 10_000, vec![]).unwrap();
    let bob_net = hub.join(PeerId::from_u64(2));
    let (bob_joined, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &invite_b),
        asy.run_once(),
    );
    let mut bsy = ChannelSync::new(
        bob_net,
        bob_joined.unwrap(),
        bob,
        rng(2),
        Box::new(ManualClock::new(1_000)),
    );
    asy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    bsy.open_channel(DocType::Channel, CHANNEL).await.unwrap();

    // Carol joins; Alice advances to epoch 2 and (with the zero-width window) keeps
    // no epoch-1 key.
    let carol = MlsDevice::generate().unwrap();
    let invite_c = asy.mint_invite([2u8; 16], 10_000, vec![]).unwrap();
    let carol_net = hub.join(PeerId::from_u64(3));
    let (carol_joined, _) = tokio::join!(
        catcoms_sync::request_join(&carol_net, alice_peer, &carol, &invite_c),
        asy.run_once(),
    );
    carol_joined.unwrap();
    assert_eq!(asy.epoch(), 2);
    assert_eq!(
        asy.stats().past_keys_retained,
        0,
        "zero-width window keeps nothing"
    );

    // Bob (still epoch 1) seals an op under epoch 1; Alice can't open it (evicted).
    bsy.post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "x", "lost-key"))
        .await
        .unwrap();
    assert!(asy.run_once().await.unwrap());
    assert_eq!(asy.stats().ops_recovered_past_epoch, 0);
    assert_eq!(
        asy.stats().ops_dropped_old_epoch,
        1,
        "counted, not silently lost"
    );
}

/// 6d-1b: the catch-up serve endpoints are members-only. A node that is not a
/// current member of the group (its own group / device) is refused, so an outsider
/// cannot harvest a group's membership history or document metadata.
#[tokio::test]
async fn catch_up_is_refused_to_a_non_member() {
    catcoms_log::init_test();
    let hub = Hub::new();

    // Alice founds a group and admits Bob, so she has commits + history to serve.
    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let alice_peer = PeerId::from_u64(1);
    let mut asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(1),
        Box::new(ManualClock::new(1_000)),
    );
    let bob = MlsDevice::generate().unwrap();
    let invite_b = asy.mint_invite([1u8; 16], 10_000, vec![]).unwrap();
    let bob_net = hub.join(PeerId::from_u64(2));
    let (_bob, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &invite_b),
        asy.run_once(),
    );
    asy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    asy.post(DocType::Channel, CHANNEL, |d| {
        d.put(ROOT, "secret", "members only")
    })
    .await
    .unwrap();

    // Mallory runs her OWN unrelated group; her device is not in Alice's group.
    let mallory = MlsDevice::generate().unwrap();
    let mallory_group = ServerGroup::create(&mallory).unwrap();
    let mut msy = ChannelSync::new(
        hub.join(PeerId::from_u64(9)),
        mallory_group,
        mallory,
        rng(9),
        Box::new(ManualClock::new(1_000)),
    );

    // Both catch-up endpoints refuse her: she gets nothing, Alice counts rejections.
    let (commits, _) = tokio::join!(msy.request_commit_catchup(alice_peer, 0), asy.run_once(),);
    assert_eq!(commits.unwrap(), 0, "non-member gets no commits");
    let (ops, _) = tokio::join!(
        msy.request_catchup(alice_peer, DocType::Channel, CHANNEL),
        asy.run_once(),
    );
    assert_eq!(ops.unwrap(), 0, "non-member gets no document history");
    assert_eq!(asy.stats().requests_rejected, 2);
}

/// 6d-1b: a member that missed a membership commit recovers it on demand with
/// ordered replay over `request_commit_catchup` (the explicit recovery API).
#[tokio::test]
async fn missed_commits_recover_in_order_via_commit_catchup() {
    catcoms_log::init_test();
    let hub = Hub::new();

    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let alice_peer = PeerId::from_u64(1);
    let mut asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(1),
        Box::new(ManualClock::new(1_000)),
    );
    asy.subscribe_control().await.unwrap();

    // Bob joins (Alice 0->1). Bob starts at epoch 1.
    let bob = MlsDevice::generate().unwrap();
    let invite_b = asy.mint_invite([1u8; 16], 10_000, vec![]).unwrap();
    let bob_net = hub.join(PeerId::from_u64(2));
    let (bob_joined, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &invite_b),
        asy.run_once(),
    );
    let mut bsy = ChannelSync::new(
        bob_net,
        bob_joined.unwrap(),
        bob,
        rng(2),
        Box::new(ManualClock::new(1_000)),
    );
    assert_eq!(bsy.epoch(), 1);

    // Carol then Dave join via Alice (1->2, 2->3). Bob is NOT subscribed to the
    // control topic, so he misses both commits.
    for (nonce, peer) in [([2u8; 16], 3u64), ([3u8; 16], 4u64)] {
        let dev = MlsDevice::generate().unwrap();
        let invite = asy.mint_invite(nonce, 10_000, vec![]).unwrap();
        let net = hub.join(PeerId::from_u64(peer));
        let (joined, _) = tokio::join!(
            catcoms_sync::request_join(&net, alice_peer, &dev, &invite),
            asy.run_once(),
        );
        joined.unwrap();
    }
    assert_eq!(asy.epoch(), 3);
    assert_eq!(bsy.epoch(), 1, "Bob missed both commits");

    // Bob explicitly catches up from epoch 1; Alice serves the ordered bundle.
    let (applied, _) = tokio::join!(bsy.request_commit_catchup(alice_peer, 1), asy.run_once(),);
    assert_eq!(applied.unwrap(), 2, "two commits replayed in order");
    assert_eq!(bsy.epoch(), 3, "Bob converged to the current epoch");
    assert_eq!(asy.stats().commits_served, 1);
}

/// 6d-1b: a member that missed a commit and later sees a *future* one detects the
/// gap, buffers it, and **auto-recovers** through `run_once` (no explicit call).
#[tokio::test]
async fn out_of_order_commit_auto_recovers_through_run_once() {
    catcoms_log::init_test();
    let hub = Hub::new();

    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let alice_peer = PeerId::from_u64(1);
    let mut asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(1),
        Box::new(ManualClock::new(1_000)),
    );
    asy.subscribe_control().await.unwrap();

    // Bob joins (Alice 0->1). Bob starts at epoch 1, NOT yet on the control topic.
    let bob = MlsDevice::generate().unwrap();
    let invite_b = asy.mint_invite([1u8; 16], 10_000, vec![]).unwrap();
    let bob_net = hub.join(PeerId::from_u64(2));
    let (bob_joined, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &invite_b),
        asy.run_once(),
    );
    let mut bsy = ChannelSync::new(
        bob_net,
        bob_joined.unwrap(),
        bob,
        rng(2),
        Box::new(ManualClock::new(1_000)),
    );

    // Carol joins (1->2) while Bob is still off the control topic — he misses it.
    let carol = MlsDevice::generate().unwrap();
    let invite_c = asy.mint_invite([2u8; 16], 10_000, vec![]).unwrap();
    let carol_net = hub.join(PeerId::from_u64(3));
    let (carol_joined, _) = tokio::join!(
        catcoms_sync::request_join(&carol_net, alice_peer, &carol, &invite_c),
        asy.run_once(),
    );
    carol_joined.unwrap();

    // Bob comes online (subscribes to control) at epoch 1, having missed epoch-1's
    // commit. Dave joins (2->3); Bob receives that future commit.
    bsy.subscribe_control().await.unwrap();
    assert_eq!(bsy.epoch(), 1);
    let dave = MlsDevice::generate().unwrap();
    let invite_d = asy.mint_invite([3u8; 16], 10_000, vec![]).unwrap();
    let dave_net = hub.join(PeerId::from_u64(4));
    let (dave_joined, _) = tokio::join!(
        catcoms_sync::request_join(&dave_net, alice_peer, &dave, &invite_d),
        asy.run_once(),
    );
    dave_joined.unwrap();
    assert_eq!(asy.epoch(), 3);

    // Tick 1: Bob processes Dave's epoch-2 commit, sees the gap (he is at epoch 1),
    // buffers it, and queues a commit catch-up.
    assert!(bsy.run_once().await.unwrap());
    assert_eq!(bsy.epoch(), 1, "future commit is buffered, not applied");
    assert_eq!(bsy.stats().commits_buffered, 1);

    // Tick 2: Bob's queued catch-up fires; Alice serves the ordered bundle and Bob
    // replays epoch 1 then epoch 2, converging — all driven by run_once.
    let (_, _) = tokio::join!(bsy.run_once(), asy.run_once());
    assert_eq!(bsy.epoch(), 3, "Bob auto-recovered to the current epoch");
    assert_eq!(bsy.stats().commit_catchups_requested, 1);
    assert!(bsy.stats().commits_applied >= 2);
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
