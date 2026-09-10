//! Two members synchronizing a channel over the in-memory mesh transport:
//! live gossip convergence and request/response catch-up. Deterministic (no real
//! sockets, seeded RNG), so it is reliable in CI; the same `ChannelSync` runs
//! unchanged over the libp2p mesh.

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ReadDoc, ROOT};
use catcoms_mls::{InviteLedger, MlsDevice, ServerGroup};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_sync::{fingerprint, ChannelSync, SyncError};
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
        let (group, routing) = joined.unwrap();
        let mut new_sync = ChannelSync::new_joined(
            net,
            group,
            dev,
            rng(1 + i as u64),
            Box::new(clock.clone()),
            routing,
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

/// Build a joiner's `ChannelSync` from a `request_join` result, adopting the
/// transferred routing state so it derives the same topics/namespaces as the group.
fn joined_sync(
    net: MemNetwork,
    joined: Result<(ServerGroup, catcoms_sync::RoutingState), catcoms_sync::SyncError>,
    device: MlsDevice,
    rng: ChaCha20Rng,
    clock: ManualClock,
) -> ChannelSync<MemNetwork, ChaCha20Rng> {
    let (group, routing) = joined.expect("joined");
    ChannelSync::new_joined(net, group, device, rng, Box::new(clock), routing)
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
async fn delivery_evidence_names_the_peer_that_built_on_a_change() {
    catcoms_log::init_test();
    let (mut asy, mut bsy, _, _) = pair();
    asy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    bsy.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    let alice_fp = fingerprint(&asy.device_id());
    let bob_fp = fingerprint(&bsy.device_id());

    let change = asy
        .post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "msg", "hi"))
        .await
        .unwrap();
    // Publishing proves nothing: gossip is fire-and-forget, so until a peer writes on top of
    // the change there is no evidence anyone received it.
    assert!(asy
        .peers_with_change(DocType::Channel, CHANNEL, change)
        .is_empty());

    // Bob ingests the op, then authors his own; which necessarily descends from Alice's.
    assert!(bsy.run_once().await.unwrap());
    assert_eq!(
        bsy.peers_with_change(DocType::Channel, CHANNEL, change),
        vec![alice_fp.clone()],
        "from Bob's side only the change's own author is proven to hold it"
    );
    let bob_change = bsy
        .post(DocType::Channel, CHANNEL, |d| {
            d.put(ROOT, "reply", "got it")
        })
        .await
        .unwrap();
    assert!(asy.run_once().await.unwrap());

    assert_eq!(
        asy.peers_with_change(DocType::Channel, CHANNEL, change),
        vec![bob_fp.clone()],
        "Bob's change builds on Alice's, so he provably holds it"
    );
    // Bob still has no evidence for his *own* op: Alice holds it but has not written on top of
    // it, so nothing proves she did. Absence is "unknown", never "not delivered".
    assert!(bsy
        .peers_with_change(DocType::Channel, CHANNEL, bob_change)
        .is_empty());
    assert!(
        !asy.peers_with_change(DocType::Channel, CHANNEL, change)
            .contains(&alice_fp),
        "the querying device never counts itself"
    );

    // A change the document doesn't hold, and an unopened document, both report nothing.
    assert!(asy
        .peers_with_change(DocType::Channel, CHANNEL, automerge::ChangeHash([9u8; 32]))
        .is_empty());
    assert!(asy
        .peers_with_change(DocType::Channel, CHANNEL + 1, change)
        .is_empty());

    // The batch form answers the whole set in one pass, index-aligned with its input.
    let batch = asy.peers_with_changes(DocType::Channel, CHANNEL, &[change, bob_change]);
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0], vec![bob_fp.clone()]);
    assert_eq!(batch[1], vec![bob_fp], "Bob authored it, and he is not us");
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
    let (bob_group, bob_routing) = joined.expect("bob joined");
    assert_eq!(bob_group.epoch(), 1);
    assert!(bob_group.contains_device(&bob.device_id()));

    // Bob is now a member: he and Alice converge on a channel.
    let mut bsy = ChannelSync::new_joined(
        bob_net,
        bob_group,
        bob,
        rng(2),
        Box::new(ManualClock::new(1_000)),
        bob_routing,
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

    // Bob tries to redeem Alice's invite by talking to Carol's node; rejected.
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
    let mut bsy = joined_sync(bob_net, bob_joined, bob, rng(2), ManualClock::new(1_000));
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
    let mut csy = joined_sync(
        carol_net,
        carol_joined,
        carol,
        rng(3),
        ManualClock::new(1_000),
    );
    csy.subscribe_control().await.unwrap();
    assert_eq!(asy.epoch(), 2);
    assert_eq!(csy.epoch(), 2);

    // Bob; who admitted no one; receives the broadcast commit and advances.
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

    // Bob joins (member, leaf 1; NOT the designated committer).
    let bob = MlsDevice::generate().unwrap();
    let invite_b = asy.mint_invite([1u8; 16], 10_000, vec![]).unwrap();
    let bob_peer = PeerId::from_u64(2);
    let bob_net = hub.join(bob_peer);
    let (bob_joined, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &invite_b),
        asy.run_once(),
    );
    let mut bsy = joined_sync(bob_net, bob_joined, bob, rng(2), ManualClock::new(1_000));

    // Bob (a plain member) invites Carol and tries to admit her. Bob is not the committer, and
    // there is no published admin roster here; so Bob cannot *predict* the owner's decision and
    // relays the request (rather than self-gating; treating an unreadable roster as "unknown, not
    // unauthorized" is what stops a junk roster-overwrite from disabling every admin's relay).
    // The owner is not driven in this test, so the Welcome never comes: the join must NOT succeed,
    // and Carol must never be admitted. (In production a caller-side timeout; `Server::join`'s
    // `JOIN_TIMEOUT_SECS`; turns the un-pushed Welcome into a clean error; the sync crate itself
    // holds no timer, so the test bounds the wait.)
    let invite_c = bsy.mint_invite([2u8; 16], 10_000, vec![]).unwrap();
    let carol = MlsDevice::generate().unwrap();
    let carol_net = hub.join(PeerId::from_u64(3));
    let joined = tokio::time::timeout(std::time::Duration::from_millis(200), async {
        let (r, _) = tokio::join!(
            catcoms_sync::request_join(&carol_net, bob_peer, &carol, &invite_c),
            async {
                for _ in 0..4 {
                    let _ = bsy.run_once().await;
                }
            },
        );
        r
    })
    .await;
    // Either the relayed request is refused outright, or it hangs waiting for a Welcome that never
    // comes (the timeout); never a successful join.
    assert!(
        matches!(joined, Err(_) | Ok(Err(_))),
        "a non-committer's relayed admission must not succeed"
    );
    assert!(
        !bsy.contains_member(&carol.device_id()),
        "Carol was never admitted"
    );
}

/// 6d-1b: an op sealed under the epoch just before a membership change still
/// decrypts after the holder advances, via the bounded past-epoch key window;
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
    let mut bsy = joined_sync(bob_net, bob_joined, bob, rng(2), ManualClock::new(1_000));
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

    // Alice ingests the epoch-1 op while at epoch 2; recovered via the retained
    // past-epoch key rather than dropped.
    assert!(asy.run_once().await.unwrap());
    assert_eq!(get_str(&asy, "late").as_deref(), Some("from epoch 1"));
    assert_eq!(asy.stats().ops_recovered_past_epoch, 1);
}

/// 6d-1b: when the past-epoch key window has evicted the needed epoch, an op
/// sealed under it is not silently lost; it is counted dropped and a document
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
    let mut bsy = joined_sync(bob_net, bob_joined, bob, rng(2), ManualClock::new(1_000));
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
    let mut bsy = joined_sync(bob_net, bob_joined, bob, rng(2), ManualClock::new(1_000));
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
    let mut bsy = joined_sync(bob_net, bob_joined, bob, rng(2), ManualClock::new(1_000));

    // Carol joins (1->2) while Bob is still off the control topic; he misses it.
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
    // replays epoch 1 then epoch 2, converging; all driven by run_once.
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
    let mut bsy = joined_sync(bob_net, bob_g, bob, rng(2), clock.clone());
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

    // Alice (leaf 0) and Bob (leaf 1) BOTH remove Carol at epoch 2; a same-base
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
/// remove still resolves correctly after the window; the committer merges its own
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
    let mut bsy = joined_sync(bob_net, bob_g, bob, rng(2), clock.clone());
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
/// fork it observes to the same winner as the committers; convergence holds for
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
/// is byte-identical across winner and loser; proven by exchanging an
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
/// merged synchronously) and the joiner is admitted via the two-phase flow; a
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

    let (bob_group, _) = bob_result.expect("bob joined via the two-phase staged path");
    assert_eq!(bob_group.epoch(), 1);
    assert!(bob_group.contains_device(&bob.device_id()));
    assert_eq!(s[0].epoch(), 1, "the staged admission merged");
    assert!(s[0].contains_member(&bob.device_id()));
}

/// Removal is **owner-only**, enforced at the protocol layer (THREAT-MODEL R1): the designated
/// committer (the owner) removes directly, and a non-owner's `request_remove` is rejected
/// outright; a modified member cannot get anyone removed.
#[tokio::test]
async fn removal_is_owner_only_and_a_non_owner_request_is_rejected() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    // Alice(0)=owner/committer, Bob(1)=non-owner, Carol(2)=target; default config.
    let (mut s, ids) = build_members(&hub, &clock, 3).await;
    let carol = ids[2];
    assert_eq!(s[0].epoch(), 2);

    // A non-owner cannot remove; the request is rejected and nothing is broadcast.
    assert!(matches!(
        s[1].request_remove(&carol).await,
        Err(SyncError::Unauthorized)
    ));
    assert_eq!(s[1].epoch(), 2, "a rejected request changes nothing");
    assert!(s[0].contains_member(&carol), "Carol is still a member");
    assert_eq!(s[0].epoch(), 2, "the committer did not act on a non-owner");

    // The owner removes Carol directly; the others converge on the broadcast commit.
    s[0].request_remove(&carol).await.unwrap();
    assert_eq!(s[0].epoch(), 3, "the owner executed the removal");
    assert!(!s[0].contains_member(&carol));
    assert!(s[1].run_once().await.unwrap());
    assert_eq!(s[1].epoch(), 3);
    assert!(!s[1].contains_member(&carol));
    assert_eq!(s[0].member_count(), s[1].member_count());
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

// --- 6e-3d-1: routing label (ns_secret_L) + rendezvous namespaces ------------
//
// The blinded gossip topics and rendezvous namespaces rotate only on member
// *removal* (ARCHITECTURE §2.5). These exercise the pure derivation + the
// removal-indexed snapshot store; cross-joiner baseline (L=0) convergence needs
// the join transfer (6e-3d-9), so they assert the *current* namespace converges
// among members who share a removal, not the grandfathered baseline.

/// Two stand-in rendezvous peer ids; the namespace is bound per-rendezvous.
const RZ: &[u8] = b"catcoms-rendezvous-node-one-0001";
const RZ2: &[u8] = b"catcoms-rendezvous-node-two-0002";

#[tokio::test]
async fn routing_namespace_rotates_only_on_member_removal() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    // Alice(0)=owner/committer, Bob(1)=applier, Carol(2)=removal target.
    let (mut s, ids) = build_members(&hub, &clock, 3).await;

    // Two Adds built the group; neither rotated the routing label.
    for sync in &s {
        assert_eq!(
            sync.routing_label(),
            0,
            "Adds/Updates must not rotate the routing label"
        );
    }
    let before = s[0].rendezvous_namespaces(RZ);
    assert_eq!(before.len(), 1, "only the current namespace exists at L=0");
    // The namespace is bound per-rendezvous (colluding rendezvous can't join logs).
    assert_ne!(
        before[0],
        s[0].rendezvous_namespaces(RZ2)[0],
        "the namespace must differ per rendezvous node"
    );

    // Remove Carol: Alice (owner) removes directly + fans out the commit; Bob applies. This
    // exercises both rotation paths; the local committer (commit_remove_now) and the inbound
    // apply (note_commit_applied via process_incoming).
    s[0].request_remove(&ids[2]).await.unwrap();
    assert!(s[1].run_once().await.unwrap()); // Bob applies the broadcast commit

    // The removal rotated L to 1 on both remaining members, identically.
    assert_eq!(s[0].routing_label(), 1, "committer rotated on removal");
    assert_eq!(s[1].routing_label(), 1, "applier rotated on removal");
    let a = s[0].rendezvous_namespaces(RZ);
    let b = s[1].rendezvous_namespaces(RZ);
    assert_eq!(
        a[0], b[0],
        "remaining members converge on the post-removal namespace"
    );
    assert_ne!(a[0], before[0], "the namespace changed on the removal");
    assert_eq!(a.len(), 2, "current + one grandfathered namespace at L=1");

    // Carol, removed, never advanced her label and cannot compute the post-removal
    // namespace; her advertised set excludes it.
    assert_eq!(s[2].routing_label(), 0);
    assert!(
        !s[2].rendezvous_namespaces(RZ).contains(&a[0]),
        "a removed member cannot compute the post-removal namespace"
    );
}

#[tokio::test]
async fn grandfathered_namespaces_are_bounded_to_two_windows() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    // Alice(0)=owner/committer, Bob(1)=applier; members 2/3/4 are removal targets.
    // Driving only Alice and Bob avoids the multi-applier gossip-ordering dance; Alice
    // removes directly and Bob receives only the resulting commit, so one tick each.
    let (mut s, ids) = build_members(&hub, &clock, 5).await;
    let ns_l0 = s[0].rendezvous_namespaces(RZ)[0].clone();

    // Three successive removals; Alice (owner) removes directly, Bob applies each.
    for target in [4usize, 3, 2] {
        s[0].request_remove(&ids[target]).await.unwrap();
        assert!(s[1].run_once().await.unwrap()); // Bob applies
    }

    assert_eq!(s[0].routing_label(), 3, "three removals advanced L to 3");
    let ns = s[0].rendezvous_namespaces(RZ);
    assert_eq!(
        ns.len(),
        3,
        "the window is bounded to the current + two grandfathered labels"
    );
    // All three retained namespaces are distinct.
    assert_ne!(ns[0], ns[1]);
    assert_ne!(ns[1], ns[2]);
    assert_ne!(ns[0], ns[2]);
    // The L=0 namespace has aged out of the window (evicted + zeroized).
    assert!(
        !ns.contains(&ns_l0),
        "the oldest label is evicted past the two-window bound"
    );
}

#[tokio::test]
async fn distinct_groups_derive_distinct_namespaces() {
    catcoms_log::init_test();
    let clock = ManualClock::new(1_000);
    let hub_a = Hub::new();
    let (sa, _) = build_members(&hub_a, &clock, 1).await; // founder-only group A
    let hub_b = Hub::new();
    let (sb, _) = build_members(&hub_b, &clock, 1).await; // founder-only group B
    assert_ne!(
        sa[0].rendezvous_namespaces(RZ)[0],
        sb[0].rendezvous_namespaces(RZ)[0],
        "two distinct groups must never collide on a rendezvous namespace"
    );
}

// --- 6e-3d-2: routing-state transfer on join -------------------------------
//
// A joiner captures a different L=0 baseline than the founder (the routing secret
// is epoch-specific), so without the transfer it would derive a different
// namespace. `build_members` builds joiners via `new_joined`, which adopts the
// routing state sealed into the join response; so they converge.

#[tokio::test]
async fn a_joiner_adopts_the_routing_state_and_converges_on_the_namespace() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    // Alice founds at epoch 0; Bob joins at epoch 1 (a different local baseline).
    let (s, _ids) = build_members(&hub, &clock, 2).await;
    assert_eq!(s[0].routing_label(), 0);
    assert_eq!(s[1].routing_label(), 0);
    assert_eq!(
        s[0].rendezvous_namespaces(RZ),
        s[1].rendezvous_namespaces(RZ),
        "a joiner that adopted the transfer derives the founder's namespace"
    );
}

#[tokio::test]
async fn a_member_joining_after_a_removal_converges_via_the_transfer() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let alice_peer = PeerId::from_u64(1);
    // Alice(0)=owner/committer, Bob(1)=applier, Carol(2)=target.
    let (mut s, ids) = build_members(&hub, &clock, 3).await;

    // Remove Carol: the group advances to L=1 with a post-removal ns_secret that a
    // *future* joiner cannot derive on its own (it was never at that epoch).
    s[0].request_remove(&ids[2]).await.unwrap();
    assert!(s[1].run_once().await.unwrap());
    assert_eq!(s[0].routing_label(), 1);

    // Dave joins *after* the removal; the transfer carries L=1 and the post-removal
    // secret, so he converges on the current namespace despite never being present.
    let dave = MlsDevice::generate().unwrap();
    let invite = s[0].mint_invite([9u8; 16], u64::MAX, vec![]).unwrap();
    let dave_net = hub.join(PeerId::from_u64(99));
    let (dave_joined, _) = tokio::join!(
        catcoms_sync::request_join(&dave_net, alice_peer, &dave, &invite),
        s[0].run_once(),
    );
    let dsy = joined_sync(dave_net, dave_joined, dave, rng(99), clock.clone());
    assert_eq!(
        dsy.routing_label(),
        1,
        "Dave adopted the post-removal label via the transfer"
    );
    assert_eq!(
        dsy.rendezvous_namespaces(RZ)[0],
        s[0].rendezvous_namespaces(RZ)[0],
        "a post-removal joiner converges on the current namespace"
    );
}

// --- 6e-3d-2b: topics re-keyed from ns_secret_L (closes A1) -----------------

#[tokio::test]
async fn channel_delivery_survives_a_topic_rotation_on_removal() {
    catcoms_log::init_test();
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let (mut s, ids) = build_members(&hub, &clock, 3).await;

    // Alice and Bob open #general and converge on a first post (label-0 topic).
    s[0].open_channel(DocType::Channel, CHANNEL).await.unwrap();
    s[1].open_channel(DocType::Channel, CHANNEL).await.unwrap();
    s[0].post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "before", "1"))
        .await
        .unwrap();
    assert!(s[1].run_once().await.unwrap());
    assert_eq!(get_str(&s[1], "before").as_deref(), Some("1"));

    // Remove Carol: the routing label rotates, so the channel + control topics change.
    s[0].request_remove(&ids[2]).await.unwrap(); // Alice (owner) commits + rotates + resubscribes
    assert!(s[1].run_once().await.unwrap()); // Bob applies + rotates + resubscribes
    assert_eq!(s[0].routing_label(), 1);
    assert_eq!(s[1].routing_label(), 1);

    // Alice posts on the NEW (label-1) channel topic; Bob, re-subscribed across the
    // rotation, still receives it.
    s[0].post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "after", "2"))
        .await
        .unwrap();
    assert!(s[1].run_once().await.unwrap());
    assert_eq!(get_str(&s[1], "after").as_deref(), Some("2"));
}

// --- micelle convergence: chained heal across three partitions ---------------
//
// The scenario `docs/MESSAGE-FLOW.md` section 10.1 specifies. Everything else in this file tests
// two members, or a group that stays whole. This tests the property the product actually
// promises: that a group which splits into independent sub-groups, writes in each, loses several
// authors outright, and is then rejoined one link at a time by members who never all meet, still
// ends with the same history everywhere.
//
// The load-bearing implementation detail is that `EncryptedDoc::export_catchup_since` iterates
// the serving node's *entire* signed-op log rather than the ops it authored, and
// `apply_signed_tracked` pushes every accepted op into that same log. A node therefore relays
// third-party history on the strength of holding it. This test is what turns that from a reading
// of the code into a fact about the system.

/// Peer ids, stable across every topology below.
const P_A: u64 = 1;
const P_B: u64 = 2;
const P_C: u64 = 3;
const P_D: u64 = 4;
const P_E: u64 = 5;
const P_F: u64 = 6;

/// Move one member's durable state onto a different network topology.
///
/// A partition is not something `MemNetwork` can express: its `Hub` delivers every publication to
/// every subscriber of a topic, so all six members sharing one hub would hear everything. A
/// micelle is therefore its own hub, and a member changes micelle by snapshotting and restoring
/// onto the new one. That is also the honest model of the real event: the member's durable state
/// survives, its neighbourhood does not. It exercises the restore path as a bonus, which is where
/// `first_proof_sweep_owed` lives.
async fn relocate(
    sync: &mut ChannelSync<MemNetwork, ChaCha20Rng>,
    hub: &std::sync::Arc<Hub>,
    peer: u64,
    clock: &ManualClock,
    seed: u64,
) -> ChannelSync<MemNetwork, ChaCha20Rng> {
    let snapshot = sync
        .snapshot()
        .expect("snapshot the member's durable state");
    let mut moved = ChannelSync::restore(
        &snapshot,
        hub.join(PeerId::from_u64(peer)),
        rng(seed),
        Box::new(clock.clone()),
    )
    .expect("restore onto the new topology");
    // Subscriptions live on the hub, not in the snapshot, so they must be re-established.
    moved.subscribe_control().await.unwrap();
    moved.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    moved
}

/// One catch-up exchange: what the requester applied from a single round.
async fn sync_round(
    requester: &mut ChannelSync<MemNetwork, ChaCha20Rng>,
    server: &mut ChannelSync<MemNetwork, ChaCha20Rng>,
    server_peer: u64,
) -> usize {
    let (applied, served) = tokio::join!(
        requester.request_catchup(PeerId::from_u64(server_peer), DocType::Channel, CHANNEL),
        server.run_once(),
    );
    assert!(served.expect("the serving transport stays open"));
    applied.expect("catch-up")
}

/// Pull everything `server` holds that `requester` does not, driving both sides until the
/// exchange stops yielding. Returns the total applied.
async fn sync_from(
    requester: &mut ChannelSync<MemNetwork, ChaCha20Rng>,
    server: &mut ChannelSync<MemNetwork, ChaCha20Rng>,
    server_peer: u64,
) -> usize {
    let mut total = 0;
    for _ in 0..64 {
        let applied = sync_round(requester, server, server_peer).await;
        total += applied;
        if applied == 0 {
            break;
        }
    }
    total
}

/// The message keys this member holds, sorted. Each write below uses a distinct key, so this is
/// the set of logical messages.
fn held(sync: &ChannelSync<MemNetwork, ChaCha20Rng>) -> Vec<String> {
    let doc = sync
        .doc(DocType::Channel, CHANNEL)
        .expect("the channel is open")
        .doc();
    let mut keys: Vec<String> = doc.keys(ROOT).collect();
    keys.sort();
    keys
}

/// The document's Automerge frontier, sorted. Cloned because `get_heads` needs `&mut`, and the
/// public accessor hands out a shared reference.
fn heads(sync: &ChannelSync<MemNetwork, ChaCha20Rng>) -> Vec<automerge::ChangeHash> {
    let mut doc = sync
        .doc(DocType::Channel, CHANNEL)
        .expect("the channel is open")
        .doc()
        .clone();
    let mut heads = doc.get_heads();
    heads.sort();
    heads
}

/// How many signed ops this member holds.
fn ops(sync: &ChannelSync<MemNetwork, ChaCha20Rng>) -> usize {
    sync.doc(DocType::Channel, CHANNEL)
        .expect("the channel is open")
        .op_count()
}

#[tokio::test]
async fn three_micelles_heal_in_a_chain_and_converge_without_their_authors() {
    catcoms_log::init_test();
    let clock = ManualClock::new(1_000);

    // --- phase 0: one group, one shared ancestor -----------------------------
    let hub0 = Hub::new();
    let (mut syncs, _ids) = build_members(&hub0, &clock, 6).await;
    for s in syncs.iter_mut() {
        s.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    }
    syncs[0]
        .post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "seed", "0"))
        .await
        .unwrap();
    for s in syncs.iter_mut().skip(1) {
        assert!(s.run_once().await.unwrap());
    }
    for s in syncs.iter() {
        assert_eq!(held(s), vec!["seed"], "everyone starts from the same state");
    }

    // --- phase 1: split into three micelles ----------------------------------
    // Three hubs, and no member is ever registered on two of them at once. That is what makes
    // the isolation structural rather than a matter of which calls the test remembers to skip.
    let hub_ab = Hub::new();
    let hub_cd = Hub::new();
    let hub_ef = Hub::new();
    let mut a = relocate(&mut syncs[0], &hub_ab, P_A, &clock, 101).await;
    let mut b = relocate(&mut syncs[1], &hub_ab, P_B, &clock, 102).await;
    let mut c = relocate(&mut syncs[2], &hub_cd, P_C, &clock, 103).await;
    let mut d = relocate(&mut syncs[3], &hub_cd, P_D, &clock, 104).await;
    let mut e = relocate(&mut syncs[4], &hub_ef, P_E, &clock, 105).await;
    let mut f = relocate(&mut syncs[5], &hub_ef, P_F, &clock, 106).await;
    drop(syncs);

    a.post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "a1", "from A"))
        .await
        .unwrap();
    assert!(b.run_once().await.unwrap());
    b.post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "b1", "from B"))
        .await
        .unwrap();
    assert!(a.run_once().await.unwrap());

    c.post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "g1", "from C"))
        .await
        .unwrap();
    assert!(d.run_once().await.unwrap());
    d.post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "d1", "from D"))
        .await
        .unwrap();
    assert!(c.run_once().await.unwrap());

    e.post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "e1", "from E"))
        .await
        .unwrap();
    assert!(f.run_once().await.unwrap());
    f.post(DocType::Channel, CHANNEL, |d| d.put(ROOT, "f1", "from F"))
        .await
        .unwrap();
    assert!(e.run_once().await.unwrap());

    // The intermediate assertions, and the reason this test can only pass for the right reason.
    // Establishing that B holds A's message *before* A dies is what later makes E's receipt of
    // it unambiguous evidence of relay rather than of some direct contact.
    assert_eq!(held(&b), vec!["a1", "b1", "seed"], "B holds A's message");
    assert_eq!(held(&c), vec!["d1", "g1", "seed"], "C holds D's message");
    assert_eq!(held(&e), vec!["e1", "f1", "seed"], "E holds F's message");
    // And no micelle has heard any other micelle's writes.
    assert_eq!(held(&a), vec!["a1", "b1", "seed"]);
    assert_eq!(held(&d), vec!["d1", "g1", "seed"]);
    assert_eq!(held(&f), vec!["e1", "f1", "seed"]);

    // --- phase 2: A, D and F are gone, permanently ---------------------------
    drop(a);
    drop(d);
    drop(f);

    // --- heal 1: B <-> C -----------------------------------------------------
    let hub_bc = Hub::new();
    let mut b = relocate(&mut b, &hub_bc, P_B, &clock, 201).await;
    let mut c = relocate(&mut c, &hub_bc, P_C, &clock, 202).await;
    assert_eq!(
        sync_from(&mut b, &mut c, P_C).await,
        2,
        "B learns C's micelle"
    );
    assert_eq!(
        sync_from(&mut c, &mut b, P_B).await,
        2,
        "C learns B's micelle"
    );

    let after_first_heal = vec!["a1", "b1", "d1", "g1", "seed"];
    assert_eq!(held(&b), after_first_heal);
    assert_eq!(held(&c), after_first_heal);
    // C now holds history authored by two members it has never exchanged a packet with, one of
    // whom no longer exists. This is the store half of store-and-forward.
    assert!(held(&c).contains(&"a1".to_string()));

    // --- heal 2: C <-> E, with B disconnected --------------------------------
    // B is not relocated onto `hub_ce` and is never registered on it, so B and E have no path to
    // each other, directly or transitively, for the whole of this phase.
    let hub_ce = Hub::new();
    let mut c = relocate(&mut c, &hub_ce, P_C, &clock, 301).await;
    let mut e = relocate(&mut e, &hub_ce, P_E, &clock, 302).await;
    assert_eq!(sync_from(&mut e, &mut c, P_C).await, 4, "E learns four ops");
    assert_eq!(
        sync_from(&mut c, &mut e, P_E).await,
        2,
        "C learns E's micelle"
    );

    let everything = vec!["a1", "b1", "d1", "e1", "f1", "g1", "seed"];
    assert_eq!(held(&c), everything);
    assert_eq!(held(&e), everything);
    // The whole point. `a1` was authored by A, relayed by B, stored by C, and delivered to E
    // after A was gone and while B was unreachable from E.
    assert!(
        held(&e).contains(&"a1".to_string()),
        "third-party history reached E through a relay that did not author it"
    );

    // --- phase 3: reconnect the survivors ------------------------------------
    let hub_end = Hub::new();
    let mut b = relocate(&mut b, &hub_end, P_B, &clock, 401).await;
    let mut c = relocate(&mut c, &hub_end, P_C, &clock, 402).await;
    let mut e = relocate(&mut e, &hub_end, P_E, &clock, 403).await;
    assert_eq!(sync_from(&mut b, &mut c, P_C).await, 2, "B learns the rest");
    // Already converged, so these must be no-ops rather than a source of duplicates.
    assert_eq!(sync_from(&mut c, &mut e, P_E).await, 0);
    assert_eq!(sync_from(&mut e, &mut b, P_B).await, 0);

    for s in [&b, &c, &e] {
        assert_eq!(held(s), everything, "every survivor holds every message");
        assert_eq!(ops(s), 7, "and holds each of them exactly once");
    }
    assert_eq!(heads(&b), heads(&c), "B and C agree on the frontier");
    assert_eq!(heads(&c), heads(&e), "C and E agree on the frontier");
}

/// The heal interrupted partway through, with a write landing during the reconciliation.
///
/// The chained test above heals over links that stay up. Real ones do not: a bridge peer meets
/// another, gets some of the way through a chunked exchange, loses the link, and both sides carry
/// on writing before it comes back. This puts requeue, deduplication, the paging continuation, the
/// completion sweep's version reset and concurrent-head preservation in one scenario.
///
/// It also covers the paging cursor's deliberate impermanence. A resume position is bound to the
/// runtime that issued it, so the reconnect below cannot use the one the interrupted walk was
/// holding; the walk has to complete anyway, from a frontier that now describes a partially
/// applied history.
#[tokio::test]
async fn a_heal_interrupted_midway_resumes_and_keeps_what_was_written_during_it() {
    catcoms_log::init_test();
    let clock = ManualClock::new(1_000);

    let hub0 = Hub::new();
    let (mut syncs, _ids) = build_members(&hub0, &clock, 2).await;
    for s in syncs.iter_mut() {
        s.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    }

    // B writes a history comfortably larger than one catch-up chunk, so the exchange below has to
    // page and can therefore be interrupted in the middle of one.
    let body = "z".repeat(16_384);
    const POSTS: usize = 40;
    for i in 0..POSTS {
        syncs[0]
            .post(DocType::Channel, CHANNEL, |d| {
                d.put(ROOT, format!("b{i}"), body.as_str())
            })
            .await
            .unwrap();
    }

    // C never consumed the live gossip, and moving it to a fresh hub discards the queue, which is
    // the state a member is actually in after being away: the history exists and it has none of
    // it. Both land on one hub so the exchange can begin.
    let hub_link = Hub::new();
    let mut b = relocate(&mut syncs[0], &hub_link, P_B, &clock, 501).await;
    let mut c = relocate(&mut syncs[1], &hub_link, P_C, &clock, 502).await;
    drop(syncs);
    assert!(held(&c).is_empty(), "C starts with none of it");

    // One round, and only one: the link drops with the walk unfinished.
    let first = sync_round(&mut c, &mut b, P_B).await;
    assert!(first > 0, "the first round delivered something");
    assert!(
        first < POSTS,
        "and not all of it, or there is no interruption to test"
    );
    let midway = ops(&c);

    // The link is gone. Neither is registered on the other's hub, so nothing can pass between
    // them, and both keep working.
    let hub_b = Hub::new();
    let hub_c = Hub::new();
    let mut b = relocate(&mut b, &hub_b, P_B, &clock, 503).await;
    let mut c = relocate(&mut c, &hub_c, P_C, &clock, 504).await;
    assert_eq!(ops(&c), midway, "the partial history survived the break");

    c.post(DocType::Channel, CHANNEL, |d| {
        d.put(ROOT, "written_during_the_heal", "yes")
    })
    .await
    .unwrap();
    b.post(DocType::Channel, CHANNEL, |d| {
        d.put(ROOT, "written_on_the_other_side", "yes")
    })
    .await
    .unwrap();

    // Back together, on a hub neither has used, so the interrupted walk's resume position is
    // meaningless here and the exchange has to stand on its own.
    let hub_back = Hub::new();
    let mut b = relocate(&mut b, &hub_back, P_B, &clock, 505).await;
    let mut c = relocate(&mut c, &hub_back, P_C, &clock, 506).await;
    let resumed = sync_from(&mut c, &mut b, P_B).await;
    assert!(resumed > 0, "the interrupted walk continued");
    sync_from(&mut b, &mut c, P_C).await;

    // Everything, once each. Convergence alone would not notice a resume that re-offered what the
    // first round already delivered, because deduplication absorbs it; the operation count is
    // what would show the log growing past what was actually written.
    let expected = POSTS + 2;
    for s in [&b, &c] {
        assert_eq!(
            ops(s),
            expected,
            "every operation exactly once, with nothing duplicated by the resume"
        );
    }
    for i in 0..POSTS {
        assert!(
            held(&c).contains(&format!("b{i}")),
            "the whole of B's history reached C"
        );
    }
    // The two concurrent writes are the part an interrupted heal is most likely to lose: each was
    // authored while the other side was unreachable, so they are concurrent branches that the
    // merge has to keep rather than one overwriting the other.
    for s in [&b, &c] {
        assert!(held(s).contains(&"written_during_the_heal".to_string()));
        assert!(held(s).contains(&"written_on_the_other_side".to_string()));
    }
    assert_eq!(heads(&b), heads(&c), "and both agree on the frontier");
}
