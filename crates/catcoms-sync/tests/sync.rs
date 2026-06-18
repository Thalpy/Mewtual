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

/// Build an `n`-member group (member 0 founds it and admits the rest one by one,
/// single-committer/synchronous). Every member runs a control-subscribed
/// `ChannelSync` sharing `clock`; all end at epoch `n-1`. Returns the syncs and the
/// members' device ids (index-aligned).
async fn build_members(
    hub: &std::sync::Arc<Hub>,
    clock: &ManualClock,
    n: usize,
) -> (
    Vec<ChannelSync<MemNetwork, ChaCha20Rng>>,
    Vec<catcoms_crypto::DeviceId>,
) {
    let founder = MlsDevice::generate().unwrap();
    let mut ids = vec![founder.device_id()];
    let founder_group = ServerGroup::create(&founder).unwrap();
    let founder_peer = PeerId::from_u64(1);
    let mut syncs = vec![ChannelSync::new(
        hub.join(founder_peer),
        founder_group,
        founder,
        rng(1),
        Box::new(clock.clone()),
    )];
    syncs[0].subscribe_control().await.unwrap();

    for i in 1..n {
        let dev = MlsDevice::generate().unwrap();
        ids.push(dev.device_id());
        let invite = syncs[0]
            .mint_invite([i as u8; 16], u64::MAX, vec![])
            .unwrap();
        let net = hub.join(PeerId::from_u64(1 + i as u64));
        let (joined, _) = tokio::join!(
            catcoms_sync::request_join(&net, founder_peer, &dev, &invite),
            syncs[0].run_once(),
        );
        let mut new_sync = ChannelSync::new(
            net,
            joined.unwrap(),
            dev,
            rng(1 + i as u64),
            Box::new(clock.clone()),
        );
        new_sync.subscribe_control().await.unwrap();
        syncs.push(new_sync);
        // Previously-joined members (1..i) apply the broadcast Add to reach epoch i.
        for s in syncs.iter_mut().skip(1).take(i - 1) {
            assert!(s.run_once().await.unwrap());
        }
    }
    for s in &syncs {
        assert_eq!(
            s.epoch(),
            (n - 1) as u64,
            "all members reach the same epoch"
        );
    }
    (syncs, ids)
}

/// The fork-resolution config: concurrent committers up to `rank`, with `window`.
fn fork_cfg(rank: u32, window: u64) -> catcoms_sync::SyncConfig {
    catcoms_sync::SyncConfig {
        max_committer_rank: rank,
        stage_decision_window_ms: window,
        ..Default::default()
    }
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

/// 6d-2a: two committers concurrently produce a membership commit at the same
/// epoch from the same base (a fork). With `max_committer_rank >= 1` enabled, both
/// converge deterministically on the lowest-`commit_id` winner; the loser aborts
/// its own staged commit and adopts the winner. No epoch skip, no divergence.
#[tokio::test]
async fn concurrent_removes_resolve_to_a_single_winner() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let window = 1_000u64;

    // Alice founds; admits Bob (epoch 1) then Carol (epoch 2). Alice and Bob both
    // run sync nodes and subscribe to the control topic.
    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let alice_peer = PeerId::from_u64(1);
    let mut asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(1),
        Box::new(clock.clone()),
    );
    asy.subscribe_control().await.unwrap();

    let bob = MlsDevice::generate().unwrap();
    let inv_b = asy.mint_invite([1u8; 16], u64::MAX, vec![]).unwrap();
    let bob_net = hub.join(PeerId::from_u64(2));
    let (bob_g, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &inv_b),
        asy.run_once(),
    );
    let mut bsy = ChannelSync::new(
        bob_net,
        bob_g.unwrap(),
        bob,
        rng(2),
        Box::new(clock.clone()),
    );
    bsy.subscribe_control().await.unwrap();

    let carol = MlsDevice::generate().unwrap();
    let inv_c = asy.mint_invite([2u8; 16], u64::MAX, vec![]).unwrap();
    let carol_net = hub.join(PeerId::from_u64(3));
    let (carol_g, _) = tokio::join!(
        catcoms_sync::request_join(&carol_net, alice_peer, &carol, &inv_c),
        asy.run_once(),
    );
    carol_g.unwrap();
    assert!(bsy.run_once().await.unwrap()); // Bob applies Carol's join commit
    assert_eq!(asy.epoch(), 2);
    assert_eq!(bsy.epoch(), 2);
    assert!(asy.contains_member(&carol.device_id()));

    // Enable concurrent committers (rank 1) on both, with a real contest window.
    let cfg = catcoms_sync::SyncConfig {
        max_committer_rank: 1,
        stage_decision_window_ms: window,
        ..Default::default()
    };
    asy.set_config(cfg);
    bsy.set_config(cfg);

    // Alice (leaf 0) and Bob (leaf 1) BOTH remove Carol at epoch 2 — a same-base
    // fork. Each stages and broadcasts its competing commit.
    asy.remove(&carol.device_id()).await.unwrap();
    bsy.remove(&carol.device_id()).await.unwrap();
    assert_eq!(asy.stats().pending_commits, 0); // (this is the recovery buffer, not the contest)

    // Each ingests the other's competing commit; nothing applies while the window
    // is open.
    assert!(asy.run_once().await.unwrap());
    assert!(bsy.run_once().await.unwrap());
    assert_eq!(asy.epoch(), 2, "no resolution before the window closes");
    assert_eq!(bsy.epoch(), 2);

    // Close the contest window; both resolve to the lowest-commit_id winner.
    clock.advance_ms(window);
    assert!(asy.run_once().await.unwrap());
    assert!(bsy.run_once().await.unwrap());

    assert_eq!(asy.epoch(), 3, "fork resolved with a single epoch advance");
    assert_eq!(bsy.epoch(), 3, "Bob converged to the same epoch");
    assert!(
        !asy.contains_member(&carol.device_id()),
        "Carol was removed"
    );
    assert!(!bsy.contains_member(&carol.device_id()));
    assert_eq!(asy.member_count(), bsy.member_count(), "rosters converged");
    assert_eq!(asy.stats().forks_resolved, 1);
    assert_eq!(bsy.stats().forks_resolved, 1);
    assert_eq!(
        asy.stats().forks_lost + bsy.stats().forks_lost,
        1,
        "exactly one committer lost the fork and aborted"
    );
}

/// 6d-2a: with concurrent committers enabled but no actual contention, a staged
/// remove still resolves correctly after the window — the committer merges its own
/// commit and an applier adopts it. (Single-candidate contest = the common case.)
#[tokio::test]
async fn uncontested_staged_remove_merges_after_the_window() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let window = 500u64;

    let alice = MlsDevice::generate().unwrap();
    let alice_group = ServerGroup::create(&alice).unwrap();
    let alice_peer = PeerId::from_u64(1);
    let mut asy = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        rng(1),
        Box::new(clock.clone()),
    );
    asy.subscribe_control().await.unwrap();

    let bob = MlsDevice::generate().unwrap();
    let inv_b = asy.mint_invite([1u8; 16], u64::MAX, vec![]).unwrap();
    let bob_net = hub.join(PeerId::from_u64(2));
    let (bob_g, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &inv_b),
        asy.run_once(),
    );
    let mut bsy = ChannelSync::new(
        bob_net,
        bob_g.unwrap(),
        bob,
        rng(2),
        Box::new(clock.clone()),
    );
    bsy.subscribe_control().await.unwrap();

    let carol = MlsDevice::generate().unwrap();
    let inv_c = asy.mint_invite([2u8; 16], u64::MAX, vec![]).unwrap();
    let carol_net = hub.join(PeerId::from_u64(3));
    let (carol_g, _) = tokio::join!(
        catcoms_sync::request_join(&carol_net, alice_peer, &carol, &inv_c),
        asy.run_once(),
    );
    carol_g.unwrap();
    assert!(bsy.run_once().await.unwrap());

    let cfg = catcoms_sync::SyncConfig {
        max_committer_rank: 1,
        stage_decision_window_ms: window,
        ..Default::default()
    };
    asy.set_config(cfg);
    bsy.set_config(cfg);

    // Only Alice removes Carol; Bob just ingests the broadcast.
    asy.remove(&carol.device_id()).await.unwrap();
    assert!(bsy.run_once().await.unwrap()); // Bob opens a single-candidate contest
    assert_eq!(asy.epoch(), 2);
    assert_eq!(bsy.epoch(), 2);

    clock.advance_ms(window);
    assert!(asy.run_once().await.unwrap()); // Alice merges her own staged commit
    assert!(bsy.run_once().await.unwrap()); // Bob adopts it
    assert_eq!(asy.epoch(), 3);
    assert_eq!(bsy.epoch(), 3);
    assert!(!asy.contains_member(&carol.device_id()));
    assert!(!bsy.contains_member(&carol.device_id()));
    assert_eq!(asy.stats().forks_lost, 0, "uncontested: nobody loses");
    assert_eq!(bsy.stats().forks_lost, 0);
}

/// 6d-2a: a member that did NOT commit anything (a pure applier) still resolves a
/// fork it observes to the same winner as the committers — convergence holds for
/// the whole roster, not just the participants.
#[tokio::test]
async fn a_non_committing_member_converges_on_the_fork_winner() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let window = 1_000u64;
    // Alice(0), Bob(1) are committers; Carol(2) is a pure applier; Dave(3) is the
    // removal target.
    let (mut s, ids) = build_members(&hub, &clock, 4).await;
    for sync in &mut s {
        sync.set_config(fork_cfg(1, window));
    }
    let dave = ids[3];

    s[0].remove(&dave).await.unwrap();
    s[1].remove(&dave).await.unwrap();
    // Alice & Bob each ingest the other's commit; Carol ingests both.
    assert!(s[0].run_once().await.unwrap());
    assert!(s[1].run_once().await.unwrap());
    assert!(s[2].run_once().await.unwrap());
    assert!(s[2].run_once().await.unwrap());

    clock.advance_ms(window);
    for sync in s.iter_mut().take(3) {
        assert!(sync.run_once().await.unwrap());
    }
    for (i, sync) in s.iter().take(3).enumerate() {
        assert_eq!(
            sync.epoch(),
            4,
            "member {i} converged on the post-fork epoch"
        );
        assert!(!sync.contains_member(&dave), "Dave removed for member {i}");
        assert_eq!(sync.member_count(), 3);
        assert_eq!(sync.stats().forks_resolved, 1);
    }
    // The applier never staged anything, so it never "lost".
    assert_eq!(s[2].stats().forks_lost, 0);
}

/// 6d-2a: a three-way fork (three committers, `max_committer_rank = 2`) still
/// collapses to one winner on every node.
#[tokio::test]
async fn three_way_fork_collapses_to_one_winner() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let window = 1_000u64;
    // Alice(0), Bob(1), Carol(2) are all committers (rank<=2); Dave(3) is removed.
    let (mut s, ids) = build_members(&hub, &clock, 4).await;
    for sync in &mut s {
        sync.set_config(fork_cfg(2, window));
    }
    let dave = ids[3];

    s[0].remove(&dave).await.unwrap();
    s[1].remove(&dave).await.unwrap();
    s[2].remove(&dave).await.unwrap();
    // Each of the three committers must ingest the other two's competing commits.
    for _ in 0..2 {
        for sync in s.iter_mut().take(3) {
            assert!(sync.run_once().await.unwrap());
        }
    }

    clock.advance_ms(window);
    for sync in s.iter_mut().take(3) {
        assert!(sync.run_once().await.unwrap());
    }
    for (i, sync) in s.iter().take(3).enumerate() {
        assert_eq!(sync.epoch(), 4, "committer {i} advanced exactly once");
        assert!(!sync.contains_member(&dave));
        assert_eq!(sync.member_count(), 3);
    }
    // Two of the three lost the tie-break and aborted.
    let lost: u64 = s.iter().take(3).map(|sync| sync.stats().forks_lost).sum();
    assert_eq!(lost, 2, "exactly two of three committers lost");
}

/// 6d-2a: after a fork resolves, the group is fully functional and its epoch state
/// is byte-identical across winner and loser — proven by exchanging an
/// end-to-end-encrypted channel message that converges (it only decrypts if both
/// derived the same epoch/channel key).
#[tokio::test]
async fn group_is_functional_after_a_fork() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let window = 1_000u64;
    // Alice(0), Bob(1) committers; Carol(2) the removal target.
    let (mut s, ids) = build_members(&hub, &clock, 3).await;
    for sync in &mut s {
        sync.set_config(fork_cfg(1, window));
    }
    let carol = ids[2];

    s[0].remove(&carol).await.unwrap();
    s[1].remove(&carol).await.unwrap();
    assert!(s[0].run_once().await.unwrap());
    assert!(s[1].run_once().await.unwrap());
    clock.advance_ms(window);
    assert!(s[0].run_once().await.unwrap());
    assert!(s[1].run_once().await.unwrap());
    assert_eq!(s[0].epoch(), 3);
    assert_eq!(s[1].epoch(), 3);
    assert!(!s[0].contains_member(&carol));

    // Winner and loser now exchange encrypted chat on a fresh channel.
    let (mut bsy, _) = (s.remove(1), ()); // Bob
    let mut asy = s.remove(0); // Alice
    asy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    bsy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    asy.post(DocType::Channel, CHANNEL, |d| {
        d.put(ROOT, "after_fork", "still works")
    })
    .await
    .unwrap();
    assert!(bsy.run_once().await.unwrap());
    assert_eq!(
        get_str(&bsy, "after_fork").as_deref(),
        Some("still works"),
        "post-fork epoch state must be identical for the message to decrypt"
    );
}

/// 6d-2a: with concurrent committers enabled, an admission is **staged** (not
/// merged synchronously) and the joiner is admitted via the two-phase flow — a
/// `JOIN_PENDING` ack, then a pushed Welcome once the staged commit wins and
/// merges. The joiner ends up a real member.
#[tokio::test]
async fn staged_join_admits_via_the_welcome_push() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let window = 500u64;
    let (mut s, _ids) = build_members(&hub, &clock, 1).await; // just Alice (founder)
    s[0].set_config(fork_cfg(1, window));
    let alice_peer = PeerId::from_u64(1);

    let invite = s[0].mint_invite([9u8; 16], u64::MAX, vec![]).unwrap();
    let bob = MlsDevice::generate().unwrap();
    let bob_net = hub.join(PeerId::from_u64(50));

    let (bob_result, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &invite),
        async {
            // Serve the join → stage + return JOIN_PENDING.
            s[0].run_once().await.unwrap();
            // Close the contest window → merge → push the Welcome to Bob.
            clock.advance_ms(window);
            s[0].run_once().await.unwrap();
        }
    );

    let bob_group = bob_result.expect("bob joined via the two-phase staged path");
    assert_eq!(bob_group.epoch(), 1);
    assert!(bob_group.contains_device(&bob.device_id()));
    assert_eq!(s[0].epoch(), 1, "the staged admission merged");
    assert!(s[0].contains_member(&bob.device_id()));
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
