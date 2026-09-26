//! Quiet-network and store-and-forward regressions. The chain transport refuses non-neighbour
//! requests and drops live gossip, so backlog convergence cannot accidentally use a full mesh.
use super::*;
use automerge::{transaction::Transactable, ReadDoc, ROOT};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerConnectionSnapshot};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::Mutex;

type ChainMember = ChannelSync<ChainTransport, ChaCha20Rng>;
const CHANNEL: u128 = 701;

#[derive(Debug)]
struct ChainTransport {
    inner: MemNetwork,
    neighbours: HashSet<PeerId>,
    requests: Arc<Mutex<Vec<(PeerId, PeerId)>>>,
}

#[async_trait::async_trait]
impl MeshTransport for ChainTransport {
    fn local_peer(&self) -> PeerId {
        self.inner.local_peer()
    }

    fn connection_snapshot(&self) -> Vec<PeerConnectionSnapshot> {
        self.inner
            .connection_snapshot()
            .into_iter()
            .filter(|row| self.neighbours.contains(&row.peer))
            .collect()
    }

    async fn subscribe(&self, _topic: Topic) -> Result<(), TransportError> {
        Ok(())
    }
    async fn unsubscribe(&self, _topic: Topic) -> Result<(), TransportError> {
        Ok(())
    }
    async fn publish(&self, _topic: Topic, _data: Bytes) -> Result<(), TransportError> {
        Ok(())
    }

    async fn request(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        self.requests
            .lock()
            .unwrap()
            .push((self.local_peer(), peer));
        assert!(
            self.neighbours.contains(&peer),
            "a non-neighbour request escaped the chain"
        );
        self.inner.request(peer, proto, data).await
    }

    async fn request_cancellable(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.requests
            .lock()
            .unwrap()
            .push((self.local_peer(), peer));
        assert!(
            self.neighbours.contains(&peer),
            "a non-neighbour request escaped the chain"
        );
        self.inner
            .request_cancellable(peer, proto, data, cancellation)
            .await
    }

    async fn next_event(&self) -> Option<TransportEvent> {
        self.inner.next_event().await
    }
}

async fn serve_requests(member: &mut ChainMember) {
    loop {
        let Some(TransportEvent::Request {
            from,
            data,
            responder,
            ..
        }) = member.transport.next_event().await
        else {
            panic!("only request/response is enabled on this transport");
        };
        assert!(member.transport.neighbours.contains(&from));
        responder.respond(Bytes::from(member.handle_request(from, &data)));
    }
}

async fn pull(requester: &mut ChainMember, provider: &mut ChainMember) -> usize {
    let peer = provider.local_peer();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::select! {
            result = requester.request_catchup(peer, DocType::Channel, CHANNEL) => result.unwrap(),
            _ = serve_requests(provider) => unreachable!(),
        }
    })
    .await
    .expect("the neighbour answers")
}

async fn drain(requester: &mut ChainMember, provider: &mut ChainMember) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::select! {
            attempted = requester.drain_catchup_queue() => assert!(attempted),
            _ = serve_requests(provider) => unreachable!(),
        }
    })
    .await
    .expect("queued reconciliation reaches its neighbour");
}

#[tokio::test]
async fn a_quiet_chain_reopens_old_answers_and_relays_retained_history_in_both_directions() {
    let (_old_hub, mut original, _) = tests::build_members(3).await;
    tests::converge_and_publish_test_routes(&mut original);
    for member in &mut original {
        member
            .open_channel(DocType::Channel, CHANNEL)
            .await
            .unwrap();
    }
    let peers: Vec<_> = original.iter().map(|member| member.local_peer()).collect();
    let snapshots: Vec<_> = original
        .iter_mut()
        .map(|member| member.snapshot().unwrap())
        .collect();
    drop(original);

    let hub = Hub::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let clock = ManualClock::new(1_000);
    let mut members: Vec<ChainMember> = snapshots
        .iter()
        .enumerate()
        .map(|(index, snapshot)| {
            let neighbours = if index == 1 {
                vec![peers[0], peers[2]]
            } else {
                vec![peers[1]]
            };
            ChannelSync::restore(
                snapshot,
                ChainTransport {
                    inner: hub.join(peers[index]),
                    neighbours: neighbours.into_iter().collect(),
                    requests: requests.clone(),
                },
                ChaCha20Rng::seed_from_u64(index as u64 + 90),
                Box::new(clock.clone()),
            )
            .unwrap()
        })
        .collect();
    for member in &mut members {
        for peer in member.transport.neighbours.clone() {
            member.note_peer_connected(peer);
        }
        member.catchup_queue.clear();
        // These snapshots only provision the test's three equal MLS rosters. Startup recovery
        // is covered separately; this scenario starts after its first neighbour exchange.
        member.first_proof_sweep_owed = false;
    }
    let mut nodes = members.into_iter();
    let (mut alice, mut beth, mut chris) = (
        nodes.next().unwrap(),
        nodes.next().unwrap(),
        nodes.next().unwrap(),
    );

    // Establish genuine bound responses, then let Alice finish with Beth's empty replica.
    assert_eq!(pull(&mut alice, &mut beth).await, 0);
    assert!(alice.catchup_queue.is_empty());
    assert!(alice
        .sources_checked(DocType::Channel, CHANNEL)
        .contains(&peers[1]));
    assert_eq!(pull(&mut beth, &mut alice).await, 0);

    chris
        .post(DocType::Channel, CHANNEL, |doc| {
            doc.put(ROOT, "from-chris", "retained")
        })
        .await
        .unwrap();
    assert_eq!(pull(&mut beth, &mut chris).await, 1);
    // Chris disappears before Alice learns anything. Beth remains a valid history provider.
    beth.note_peer_disconnected(peers[2]);
    drop(chris);
    assert_eq!(alice.doc(DocType::Channel, CHANNEL).unwrap().op_count(), 0);
    assert!(
        alice.catchup_queue.is_empty(),
        "nothing arriving at Alice discovers the gap"
    );

    assert_eq!(alice.schedule_reconciliation(), 1);
    drain(&mut alice, &mut beth).await;
    drain(&mut alice, &mut beth).await;
    assert!(alice
        .doc(DocType::Channel, CHANNEL)
        .unwrap()
        .doc()
        .get(ROOT, "from-chris")
        .unwrap()
        .is_some());
    assert!(alice.catchup_queue.is_empty());

    // Beth previously completed Alice too. Alice's local write must travel back over that same
    // established edge without a new connection or any live gossip.
    beth.catchup_queue.clear();
    assert_eq!(pull(&mut beth, &mut alice).await, 0);
    alice
        .post(DocType::Channel, CHANNEL, |doc| {
            doc.put(ROOT, "from-alice", "returned")
        })
        .await
        .unwrap();
    assert_eq!(beth.schedule_reconciliation(), 1);
    drain(&mut beth, &mut alice).await;
    drain(&mut beth, &mut alice).await;
    assert_eq!(beth.doc(DocType::Channel, CHANNEL).unwrap().op_count(), 2);
    assert_eq!(alice.doc(DocType::Channel, CHANNEL).unwrap().op_count(), 2);
    assert!(beth
        .doc(DocType::Channel, CHANNEL)
        .unwrap()
        .doc()
        .get(ROOT, "from-alice")
        .unwrap()
        .is_some());
    assert!(requests
        .lock()
        .unwrap()
        .iter()
        .all(|(from, to)| *from == peers[1] || *to == peers[1]));
}

#[tokio::test]
async fn quiet_catchup_wakes_after_cooldown_without_a_network_event() {
    let (_hub, mut members, ids) = tests::build_members(2).await;
    let mut bob = members.pop().unwrap();
    let mut alice = members.pop().unwrap();
    let clock = ManualClock::new(1_000);
    bob.clock = Arc::new(clock.clone());
    alice.clock = Arc::new(clock.clone());
    let peer = alice.local_peer();
    for member in [&mut alice, &mut bob] {
        member
            .open_channel(DocType::Channel, CHANNEL)
            .await
            .unwrap();
    }
    bob.note_peer_connected(peer);
    bob.promote_member_peer_bound(peer, ids[0], true);
    bob.catchup_queue.clear();
    bob.enqueue_doc_catchup(DocType::Channel, CHANNEL);
    bob.cool_off_catchup_peer(peer, DocType::Channel, CHANNEL);
    let retry_after = bob.next_catchup_retry_delay().unwrap();
    assert!(retry_after >= CATCHUP_PEER_COOLDOWN_MS);
    {
        let tick = bob.run_once();
        futures::pin_mut!(tick);
        assert!(futures::poll!(&mut tick).is_pending());
        clock.advance_ms(retry_after - 1);
        assert!(futures::poll!(&mut tick).is_pending());
        clock.advance_ms(1);
        assert!(tick.await.unwrap(), "the cooldown itself wakes the owner");
    }
    assert_eq!(bob.stats.doc_catchups_requested, 0);
    let (tick, ()) = tokio::join!(bob.run_once(), async {
        let Some(TransportEvent::Request {
            from,
            data,
            responder,
            ..
        }) = alice.transport.next_event().await
        else {
            panic!("catch-up request expected")
        };
        responder.respond(Bytes::from(alice.handle_request(from, &data)));
    });
    assert!(tick.unwrap());
    assert_eq!(bob.stats.doc_catchups_requested, 1);
    assert!(bob.catchup_queue.is_empty());
    assert_eq!(
        bob.next_catchup_retry_delay(),
        None,
        "completed work must not spin on an expired timer"
    );

    // A leftover task and cooldown are not enough to wake: the drain must be able to select
    // that same peer. These rows model completion, failed sources, and evicted pool entries.
    bob.enqueue_doc_catchup(DocType::Channel, CHANNEL);
    bob.cool_off_catchup_peer(peer, DocType::Channel, CHANNEL);
    clock.advance_ms(CATCHUP_PEER_COOLDOWN_MS + 1_000);
    assert_eq!(
        bob.next_catchup_retry_delay(),
        None,
        "a checked source is not retry work"
    );
    bob.clear_sources_checked(DocType::Channel, CHANNEL);
    bob.note_failed_catchup_peer(peer);
    assert_eq!(
        bob.next_catchup_retry_delay(),
        None,
        "global failure excludes the source"
    );
    bob.failed_catchup_peers.clear();
    bob.known_peers.clear();
    bob.member_peers.clear();
    assert_eq!(
        bob.next_catchup_retry_delay(),
        None,
        "a cooldown orphan cannot spin"
    );
}

#[tokio::test]
async fn simultaneous_reciprocal_catchup_breaks_the_timeout_lockstep() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (_old_hub, mut original, _) = tests::build_members(2).await;
    for member in &mut original {
        member
            .open_channel(DocType::Channel, CHANNEL)
            .await
            .unwrap();
    }
    let peers: Vec<_> = original.iter().map(|member| member.local_peer()).collect();
    let snapshots: Vec<_> = original
        .iter_mut()
        .map(|member| member.snapshot().unwrap())
        .collect();
    drop(original);
    let hub = Hub::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let clock = ManualClock::new(1_000);
    let mut members: Vec<ChainMember> = snapshots
        .iter()
        .enumerate()
        .map(|(index, snapshot)| {
            ChannelSync::restore(
                snapshot,
                ChainTransport {
                    inner: hub.join(peers[index]),
                    neighbours: HashSet::from([peers[1 - index]]),
                    requests: requests.clone(),
                },
                ChaCha20Rng::seed_from_u64(index as u64 + 93),
                Box::new(clock.clone()),
            )
            .unwrap()
        })
        .collect();
    for (index, member) in members.iter_mut().enumerate() {
        member.note_peer_connected(peers[1 - index]);
        member.catchup_queue.clear();
        member.first_proof_sweep_owed = false;
    }
    let mut bob = members.pop().unwrap();
    let mut alice = members.pop().unwrap();
    assert_eq!(pull(&mut alice, &mut bob).await, 0);
    assert_eq!(pull(&mut bob, &mut alice).await, 0);
    alice
        .post(DocType::Channel, CHANNEL, |doc| doc.put(ROOT, "alice", "A"))
        .await
        .unwrap();
    bob.post(DocType::Channel, CHANNEL, |doc| doc.put(ROOT, "bob", "B"))
        .await
        .unwrap();
    assert_eq!(alice.schedule_reconciliation(), 1);
    assert_eq!(bob.schedule_reconciliation(), 1);

    // Neither owner serves inbound while its first outbound catch-up is waiting. Finish both
    // timed-out ticks before allowing either owner to consume the abandoned inbound request.
    {
        let ticks = futures::future::join(alice.run_once(), bob.run_once());
        futures::pin_mut!(ticks);
        assert!(futures::poll!(&mut ticks).is_pending());
        clock.advance_ms(CATCHUP_REQUEST_MS);
        let (a, b) = ticks.await;
        assert!(a.unwrap() && b.unwrap());
    }
    let a_delay = alice.next_catchup_retry_delay().unwrap();
    let b_delay = bob.next_catchup_retry_delay().unwrap();
    assert_eq!(a_delay.min(b_delay), CATCHUP_PEER_COOLDOWN_MS);
    assert_eq!(
        a_delay.abs_diff(b_delay),
        1_000,
        "opposite directions must not wake together"
    );

    let observed_a = AtomicUsize::new(1);
    let observed_b = AtomicUsize::new(1);
    {
        let drive = futures::future::join(
            async {
                loop {
                    assert!(alice.run_once().await.unwrap());
                    observed_a.store(
                        alice.doc(DocType::Channel, CHANNEL).unwrap().op_count(),
                        Ordering::SeqCst,
                    );
                }
            },
            async {
                loop {
                    assert!(bob.run_once().await.unwrap());
                    observed_b.store(
                        bob.doc(DocType::Channel, CHANNEL).unwrap().op_count(),
                        Ordering::SeqCst,
                    );
                }
            },
        );
        futures::pin_mut!(drive);
        // Only the earlier direction becomes retryable; the other owner remains available to
        // serve. Polling these are the production run_once loops, with no special serve helper.
        assert!(futures::poll!(&mut drive).is_pending());
        clock.advance_ms(a_delay.min(b_delay));
        for _ in 0..16 {
            assert!(futures::poll!(&mut drive).is_pending());
            tokio::task::yield_now().await;
            if observed_a.load(Ordering::SeqCst) == 2 || observed_b.load(Ordering::SeqCst) == 2 {
                break;
            }
        }
        assert!(
            observed_a.load(Ordering::SeqCst) == 2 || observed_b.load(Ordering::SeqCst) == 2,
            "one side must progress while the other remains in its original cooldown"
        );
        clock.advance_ms(a_delay.abs_diff(b_delay));
        for _ in 0..16 {
            assert!(futures::poll!(&mut drive).is_pending());
            tokio::task::yield_now().await;
            if observed_a.load(Ordering::SeqCst) == 2 && observed_b.load(Ordering::SeqCst) == 2 {
                break;
            }
        }
        assert_eq!(observed_a.load(Ordering::SeqCst), 2);
        assert_eq!(observed_b.load(Ordering::SeqCst), 2);
    }
    assert_eq!(alice.doc(DocType::Channel, CHANNEL).unwrap().op_count(), 2);
    assert_eq!(bob.doc(DocType::Channel, CHANNEL).unwrap().op_count(), 2);
}

#[tokio::test]
async fn reconciliation_is_bound_paced_fair_and_preserves_active_work() {
    let (_hub, mut members, ids) = tests::build_members(2).await;
    let peer = members[1].local_peer();
    let alice = &mut members[0];
    let clock = ManualClock::new(1_000);
    alice.clock = Arc::new(clock.clone());
    for channel in 1..=5 {
        alice.open_channel(DocType::Channel, channel).await.unwrap();
    }
    alice.note_peer_connected(peer);
    alice.promote_member_peer(peer, ids[1]);
    alice.catchup_queue.clear();
    assert_eq!(
        alice.schedule_reconciliation(),
        0,
        "PEX/unbound proof cannot start a background sweep"
    );
    alice.promote_member_peer_bound(peer, ids[1], true);
    alice.config.max_catchup_queue = 2;
    alice.note_source_checked(peer, DocType::Channel, 1);
    alice.enqueue_doc_catchup(DocType::Channel, 1);
    alice.cool_off_catchup_peer(peer, DocType::Channel, 1);
    let cursor = CatchupCursor {
        provider: [4; 16],
        position: 8,
    };
    alice
        .catchup_cursors
        .insert((DocType::Channel, 1, peer), cursor);
    alice.note_catchup_continuation(peer, DocType::Channel, 1);
    assert_eq!(alice.schedule_reconciliation(), 1);
    assert!(
        alice.sources_checked(DocType::Channel, 1).contains(&peer),
        "in-progress sweep is not reset"
    );
    assert!(alice.catchup_peer_is_cooling(peer, DocType::Channel, 1));
    assert_eq!(
        alice.catchup_cursors.get(&(DocType::Channel, 1, peer)),
        Some(&cursor)
    );
    assert_eq!(
        alice.catchup_continuation_source(DocType::Channel, 1),
        Some(peer)
    );
    assert_eq!(
        alice.schedule_reconciliation(),
        0,
        "repeated local wakeups are coalesced"
    );
    let mut visited = HashSet::new();
    for _ in 0..4 {
        for task in alice.catchup_queue.drain(..) {
            if let CatchupTask::Doc { doc_id, .. } = task {
                visited.insert(doc_id);
            }
        }
        clock.advance_ms(CATCHUP_PEER_COOLDOWN_MS);
        assert!(alice.schedule_reconciliation() <= 2);
        assert!(alice.catchup_queue.len() <= 2);
    }
    assert_eq!(
        visited.len(),
        5,
        "a small queue must not permanently exclude later documents"
    );
}

#[tokio::test]
async fn an_expired_live_retry_is_not_shadowed_by_a_stale_connected_source() {
    let (_hub, mut members, ids) = tests::build_members(3).await;
    tests::converge_and_publish_test_routes(&mut members);
    let mut carol = members.pop().unwrap();
    let mut bob = members.pop().unwrap();
    let live = carol.local_peer();
    // A connection snapshot may already omit Alice while the owner's queued disconnect edge
    // has not arrived. Her stale most-recent proof must not shadow the usable Carol connection.
    let stale = PeerId::from_u64(9_999);
    let clock = ManualClock::new(1_000);
    bob.clock = Arc::new(clock.clone());
    carol.clock = Arc::new(clock.clone());
    for member in [&mut bob, &mut carol] {
        member
            .open_channel(DocType::Channel, CHANNEL)
            .await
            .unwrap();
    }
    bob.note_peer_connected(live);
    bob.promote_member_peer_bound(live, ids[2], true);
    bob.note_peer_connected(stale);
    bob.promote_member_peer_bound(stale, ids[0], true);
    bob.catchup_queue.clear();
    bob.enqueue_doc_catchup(DocType::Channel, CHANNEL);
    bob.cool_off_catchup_peer(live, DocType::Channel, CHANNEL);
    clock.advance_ms(CATCHUP_PEER_COOLDOWN_MS + 1_000);
    assert_eq!(bob.next_catchup_retry_delay(), Some(0));
    assert!(bob.connected_peers.contains(&stale));
    assert!(!bob.peer_is_connected(stale));
    carol
        .post(DocType::Channel, CHANNEL, |doc| {
            doc.put(ROOT, "reachable", "history")
        })
        .await
        .unwrap();
    let attempted = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::select! {
            attempted = bob.drain_catchup_queue() => attempted,
            _ = async {
                loop {
                    let Some(TransportEvent::Request { from, data, responder, .. }) = carol.transport.next_event().await else { panic!("catch-up expected") };
                    responder.respond(Bytes::from(carol.handle_request(from, &data)));
                }
            } => unreachable!(),
        }
    }).await.unwrap();
    assert!(attempted, "a zero-delay wake must reach the live source");
    assert_eq!(bob.doc(DocType::Channel, CHANNEL).unwrap().op_count(), 1);
}

#[tokio::test]
async fn a_full_stalled_queue_rotates_documents_without_forgetting_provider_progress() {
    let (_hub, mut members, ids) = tests::build_members(2).await;
    tests::converge_and_publish_test_routes(&mut members);
    let peer = members[1].local_peer();
    let member = &mut members[0];
    let clock = ManualClock::new(1000);
    member.clock = Arc::new(clock.clone());
    member.note_peer_connected(peer);
    member.promote_member_peer_bound(peer, ids[1], true);
    for id in 1..=5 {
        member.open_channel(DocType::Channel, id).await.unwrap();
    }
    member.config.max_catchup_queue = 2;
    member.catchup_queue.clear();
    member.enqueue_doc_catchup(DocType::Channel, 1);
    member.enqueue_doc_catchup(DocType::Channel, 2);
    let cursor = CatchupCursor {
        provider: [9; 16],
        position: 256,
    };
    member
        .catchup_cursors
        .insert((DocType::Channel, 1, peer), cursor);
    member.note_catchup_continuation(peer, DocType::Channel, 1);
    let mut visited = HashSet::new();
    for _ in 0..10 {
        for task in &member.catchup_queue {
            if let CatchupTask::Doc { doc_id, .. } = task {
                visited.insert(*doc_id);
            }
        }
        clock.advance_ms(CATCHUP_PEER_COOLDOWN_MS);
        member.schedule_reconciliation();
        assert!(member.catchup_queue.len() <= 2);
        // Deliberately never drain or complete a queued task.
    }
    assert_eq!(visited.len(), 5);
    assert_eq!(
        member.catchup_cursors.get(&(DocType::Channel, 1, peer)),
        Some(&cursor)
    );
}
