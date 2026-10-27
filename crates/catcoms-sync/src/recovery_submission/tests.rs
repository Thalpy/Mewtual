use super::*;
use crate::tests::{
    build_members, converge_and_publish_test_routes, prove_live_member, serve_one_test_request,
};
use catcoms_rt::{ManualClock, SharedRequestKeepalive};
use futures::{Future, FutureExt, StreamExt};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[derive(Debug)]
struct ObservedPermit(Arc<AtomicBool>);
impl DialPermit for ObservedPermit {
    fn address(&self) -> &str {
        "/ip4/203.0.113.9/tcp/9000"
    }
    fn commit_if_current(self: Box<Self>) -> Option<String> {
        self.0.store(true, Ordering::Release);
        Some(self.address().to_owned())
    }
}
fn isolated_capacity() -> Arc<Capacity> {
    Arc::new(Capacity {
        slots: Arc::new(Semaphore::new(PROCESS_SUBMISSIONS)),
        changed: watch::channel(0).0,
    })
}

#[tokio::test]
async fn queued_dial_creates_retry_evidence_only_after_a_current_submitted_completion() {
    let (_hub, mut members, ids) = build_members(2).await;
    converge_and_publish_test_routes(&mut members);
    let mut alice = members.remove(0);
    alice.recovery_submissions = Submissions::with_capacity(isolated_capacity());
    let reference = alice.descriptor_ref(ids[1]).unwrap();
    let peer = reference.peer;
    let epochs = vec![(
        FreshnessPrincipal::Device(ids[1].as_bytes().to_vec()),
        reference.seq,
    )];
    let committed = Arc::new(AtomicBool::new(false));
    assert!(alice.queue_recovery_dials(
        peer,
        vec![reference],
        vec![Box::new(ObservedPermit(committed.clone()))],
        epochs,
        false,
        u64::MAX
    ));
    assert!(
        !committed.load(Ordering::Acquire),
        "local queue admission has no driver effect"
    );
    assert!(
        alice.dial_retries.is_empty(),
        "admission is not a successful dial"
    );
    let completed = alice.recovery_submissions.jobs.next().await.unwrap();
    assert!(committed.load(Ordering::Acquire));
    assert_eq!(completed.tally.submitted, 1);
    assert!(
        alice.dial_retries.is_empty(),
        "workers cannot mutate Root health"
    );
    alice.complete_recovery_submission(completed);
    assert_eq!(alice.dial_retries[&peer.as_bytes().to_vec()].attempts, 1);
}

#[tokio::test]
async fn detached_dial_admission_rejects_replaced_descriptor_policy_and_signed_removal() {
    for fault in ["descriptor", "policy", "removal"] {
        let (_hub, mut members, ids) = build_members(2).await;
        converge_and_publish_test_routes(&mut members);
        let mut bob = members.pop().unwrap();
        let mut alice = members.pop().unwrap();
        alice.recovery_submissions = Submissions::with_capacity(isolated_capacity());
        let reference = alice.descriptor_ref(ids[1]).unwrap();
        let committed = Arc::new(AtomicBool::new(false));
        assert!(alice.queue_recovery_dials(
            bob.local_peer(),
            vec![reference],
            vec![Box::new(ObservedPermit(committed.clone()))],
            Vec::new(),
            false,
            u64::MAX
        ));
        match fault {
            "descriptor" => {
                bob.publish_self_record(Vec::new(), bob.self_record().unwrap().seq + 1)
                    .unwrap();
                alice.ingest_peer_record(bob.self_record().unwrap().clone());
                assert_eq!(
                    alice.peer_record(&ids[1]).unwrap().seq,
                    bob.self_record().unwrap().seq
                );
            }
            "policy" => {
                let previous = alice.group_policy_active;
                alice.group_policy_active = !previous;
                alice.reconcile_voice_actor();
                alice.group_policy_active = previous;
                alice.reconcile_voice_actor();
            }
            "removal" => {
                let clock = catcoms_rt::ManualClock::new(alice.clock.now_ms());
                alice.clock = Arc::new(clock.clone());
                alice.remove(&ids[1]).await.unwrap();
                clock.advance_ms(alice.config.stage_decision_window_ms);
                assert!(alice.resolve_pending_if_expired());
                assert!(!alice.group.contains_device(&ids[1]));
            }
            _ => unreachable!(),
        }
        let completed = alice.recovery_submissions.jobs.next().await.unwrap();
        assert_eq!(
            completed.tally.refundable, 1,
            "{fault} is pre-commit suppression"
        );
        assert_eq!(completed.tally.submitted, 0);
        alice.complete_recovery_submission(completed);
        assert!(
            !committed.load(Ordering::Acquire),
            "{fault} reached the lower dial effect"
        );
        assert!(
            alice.dial_retries.is_empty(),
            "stale completion must not change peer health"
        );
    }
}

#[tokio::test]
async fn dropped_root_retires_an_already_transferred_driver_permit() {
    let (_hub, mut members, ids) = build_members(2).await;
    converge_and_publish_test_routes(&mut members);
    let mut alice = members.remove(0);
    alice.recovery_submissions = Submissions::with_capacity(isolated_capacity());
    let reference = alice.descriptor_ref(ids[1]).unwrap();
    let (gate, flight) = alice
        .recovery_submission_gate(
            vec![reference.clone()],
            None,
            Some(reference.peer),
            u64::MAX,
        )
        .ok()
        .unwrap();
    let committed = Arc::new(AtomicBool::new(false));
    let transferred = Box::new(GuardedPermit {
        inner: Box::new(ObservedPermit(committed.clone())),
        gate,
        spent: Arc::new(AtomicBool::new(false)),
        _flight: flight,
    });
    // The command owns this permit independently of its original Root. Replacing Root must
    // permanently close driver admission rather than merely discard its completion receiver.
    drop(alice);
    assert!(transferred.commit_if_current().is_none());
    assert!(!committed.load(Ordering::Acquire));
}

#[tokio::test]
async fn stale_outbox_head_does_not_block_another_current_helper() {
    let (_hub, mut members, ids) = build_members(3).await;
    converge_and_publish_test_routes(&mut members);
    let mut alice = members.remove(0);
    alice.recovery_submissions = Submissions::with_capacity(isolated_capacity());
    for (member, device) in members.iter().zip(&ids[1..]) {
        alice.connected_peers.insert(member.local_peer());
        alice.promote_member_peer(member.local_peer(), *device);
        alice.repair_outbox.push_back(RepairOutboxItem {
            target: member.local_peer(),
            payload: vec![0],
            expires_at_ms: u64::MAX,
            required_refs: vec![alice.descriptor_ref(*device).unwrap()],
        });
    }
    alice.demote_member_peer(members[0].local_peer());
    alice.queue_repair_outbox();
    assert!(
        alice.repair_outbox.is_empty(),
        "stale first helper was discarded, current second admitted"
    );
    assert_eq!(alice.recovery_submissions.jobs.len(), 1);
    let gate = alice.recovery_submissions.active[0].upgrade().unwrap();
    assert_eq!(gate.connected.as_ref().unwrap().0, members[1].local_peer());
    assert!(gate.authorized());
}

#[tokio::test]
async fn notification_old_connection_cannot_submit_through_same_peer_replacement() {
    let (hub, mut members, ids) = build_members(2).await;
    converge_and_publish_test_routes(&mut members);
    let mut alice = members.remove(0);
    let peer = members[0].local_peer();
    alice.recovery_submissions = Submissions::with_capacity(isolated_capacity());
    alice.connected_peers.insert(peer);
    alice.promote_member_peer(peer, ids[1]);
    assert!(alice
        .queue_recovery_notification(RepairOutboxItem {
            target: peer,
            payload: vec![0],
            expires_at_ms: u64::MAX,
            required_refs: vec![alice.descriptor_ref(ids[1]).unwrap()]
        })
        .is_ok());
    let gate = alice.recovery_submissions.active[0].upgrade().unwrap();
    let replacement = hub.join(peer);
    assert!(
        !gate.authorized(),
        "driver lease closes before Root consumes any disconnect event"
    );
    let completed = alice.recovery_submissions.jobs.next().await.unwrap();
    alice.complete_recovery_submission(completed);
    // The replacement receives ordinary connection events, but never the old owner's payload.
    while let Some(event) = replacement.next_event().now_or_never().flatten() {
        assert!(!matches!(event, TransportEvent::Request { .. }));
    }
}

#[tokio::test]
async fn submission_slots_remain_shared_until_actual_last_custody_retires() {
    let capacity = isolated_capacity();
    let a = Submissions::with_capacity(capacity.clone());
    let b = Submissions::with_capacity(capacity.clone());
    let waiting = Submissions::with_capacity(capacity.clone());
    let mut held_a: Vec<_> = (0..LOCAL_SUBMISSIONS)
        .map(|_| a.reserve().unwrap())
        .collect();
    let held_b: Vec<_> = (0..LOCAL_SUBMISSIONS)
        .map(|_| b.reserve().unwrap())
        .collect();
    assert!(a.reserve().is_none());
    assert!(waiting.reserve().is_none());
    let outer = held_a.pop().unwrap();
    let actual_lower = outer.clone();
    drop(outer);
    assert!(
        waiting.reserve().is_none(),
        "caller retirement cannot recycle lower work"
    );
    let mut changed = waiting.changed.clone();
    assert!(changed.changed().now_or_never().is_none());
    drop(actual_lower);
    assert!(changed.changed().now_or_never().is_some());
    assert!(waiting.reserve().is_some());
    drop((held_a, held_b));
    assert_eq!(capacity.slots.available_permits(), PROCESS_SUBMISSIONS);
}

/// The local notification ACK is deliberately slower than repeated sole-owner polls. Both
/// notification seams share the gate, so the regression covers the released prefix and its
/// detached replacement with identical transport behavior and genuine request/response bytes.
struct HeldReceiptAck {
    inner: catcoms_rt::MemNetwork,
    release: watch::Receiver<bool>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
    /// Let one real local admission complete, then hold later acknowledgements. False preserves
    /// the original receipt fixture's identical held behavior at both notification seams.
    pass_first: bool,
}
#[async_trait::async_trait]
impl MeshTransport for HeldReceiptAck {
    fn local_peer(&self) -> PeerId {
        self.inner.local_peer()
    }
    fn connection_snapshot(&self) -> Vec<catcoms_rt::PeerConnectionSnapshot> {
        self.inner.connection_snapshot()
    }
    fn supports_connection_leases(&self) -> bool {
        self.inner.supports_connection_leases()
    }
    fn connection_lease(&self, peer: PeerId) -> Option<catcoms_rt::ConnectionLease> {
        self.inner.connection_lease(peer)
    }
    async fn subscribe(&self, topic: Topic) -> Result<(), TransportError> {
        self.inner.subscribe(topic).await
    }
    async fn unsubscribe(&self, topic: Topic) -> Result<(), TransportError> {
        self.inner.unsubscribe(topic).await
    }
    async fn publish(&self, topic: Topic, bytes: Bytes) -> Result<(), TransportError> {
        self.inner.publish(topic, bytes).await
    }
    async fn request(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        bytes: Bytes,
    ) -> Result<Bytes, TransportError> {
        self.inner.request(peer, protocol, bytes).await
    }
    async fn request_cancellable(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        bytes: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.inner
            .request_cancellable(peer, protocol, bytes, cancellation)
            .await
    }
    async fn next_event(&self) -> Option<TransportEvent> {
        self.inner.next_event().await
    }
    async fn notify_connected(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        bytes: Bytes,
    ) -> Result<(), TransportError> {
        let first = self.calls.fetch_add(1, Ordering::AcqRel) == 0;
        let mut release = self.release.clone();
        while !(*release.borrow() || self.pass_first && first) {
            release
                .changed()
                .await
                .map_err(|_| TransportError::Closed)?;
        }
        self.inner.notify_connected(peer, protocol, bytes).await
    }
    async fn notify_connected_cancellable(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        bytes: Bytes,
        mut cancellation: RequestCancellation,
    ) -> Result<(), TransportError> {
        let first = self.calls.fetch_add(1, Ordering::AcqRel) == 0;
        let mut release = self.release.clone();
        while !(*release.borrow() || self.pass_first && first) {
            tokio::select! {
                value = release.changed() => value.map_err(|_| TransportError::Closed)?,
                _ = cancellation.cancelled() => return Err(TransportError::Closed),
            }
        }
        self.inner
            .notify_connected_cancellable(peer, protocol, bytes, cancellation)
            .await
    }
}

#[tokio::test]
async fn held_delivery_receipt_ack_cannot_starve_inbound_catchup_across_cancelled_owner_polls() {
    use automerge::{transaction::Transactable, ROOT};
    use std::task::Poll;
    let (hub, mut members, ids) = build_members(2).await;
    converge_and_publish_test_routes(&mut members);
    let mut a = members.remove(0);
    let mut b = members.remove(0);
    a.open_channel(DocType::Channel, 912).await.unwrap();
    b.open_channel(DocType::Channel, 912).await.unwrap();
    let checkpoint = b.snapshot().unwrap();
    let peer = b.local_peer();
    drop(b);
    let (_release, receiver) = watch::channel(false);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut b = ChannelSync::restore(
        &checkpoint,
        HeldReceiptAck {
            inner: hub.join(peer),
            release: receiver,
            calls: calls.clone(),
            pass_first: false,
        },
        ChaCha20Rng::seed_from_u64(170),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    b.recovery_submissions = Submissions::with_capacity(isolated_capacity());
    crate::tests::prove_live_member(&mut b, a.local_peer(), ids[0]);
    crate::tests::prove_live_member(&mut a, peer, ids[1]);
    a.post(DocType::Channel, 912, |doc| {
        doc.put(ROOT, "origin", "genuine signed operation")
    })
    .await
    .unwrap();
    let Some(TransportEvent::Gossip { from, data, .. }) = b.transport.next_event().await else {
        panic!("actual live op")
    };
    b.on_gossip(from, &data);
    assert_eq!(b.delivery_receipt_outbox.len(), 1);
    b.post(DocType::Channel, 912, |doc| {
        doc.put(ROOT, "reply", "only catchup may supply this")
    })
    .await
    .unwrap();
    assert!(matches!(
        a.transport.next_event().await,
        Some(TransportEvent::Gossip { .. })
    ));
    let mut answer = None;
    let mut catchup = Box::pin(a.request_catchup(peer, DocType::Channel, 912));
    for _ in 0..32 {
        if answer.is_none() {
            if let Poll::Ready(value) =
                futures::future::poll_fn(|cx| Poll::Ready(catchup.as_mut().poll(cx))).await
            {
                answer = Some(value);
            }
        }
        let mut owner = Box::pin(b.run_once());
        let _ = futures::future::poll_fn(|cx| Poll::Ready(owner.as_mut().poll(cx))).await;
        drop(owner); // Model the next ready actor command cancelling this poll.
    }
    assert_eq!(
        answer.unwrap().unwrap(),
        1,
        "receipt ACK must not hide the actual inbound request"
    );
    assert_eq!(
        calls.load(Ordering::Acquire),
        1,
        "one retained submission, not re-created on every owner poll"
    );
}

#[tokio::test]
async fn mesh_repair_target_first_admission_survives_a_cancelled_second_helper_ack() {
    let (hub, mut members, ids) = build_members(5).await;
    converge_and_publish_test_routes(&mut members);
    let mut original = members.remove(0);
    let snapshot = original.snapshot().unwrap();
    let peer = original.local_peer();
    drop(original);
    let clock = ManualClock::new(1_000);
    for member in &mut members {
        member.clock = Arc::new(clock.clone());
    }
    let (_release, release) = watch::channel(false);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut alice = ChannelSync::restore(
        &snapshot,
        HeldReceiptAck {
            inner: hub.join(peer),
            release,
            calls: calls.clone(),
            pass_first: true,
        },
        ChaCha20Rng::seed_from_u64(171),
        Box::new(clock.clone()),
    )
    .unwrap();
    for (helper, device) in members.iter_mut().take(2).zip(&ids[1..3]) {
        prove_live_member(&mut alice, helper.local_peer(), *device);
        prove_live_member(helper, peer, ids[0]);
    }
    alice.topology_promotions.push_front(ids[3]);
    // The first notify really enters MemNetwork; the second suspends at its local ACK before
    // handoff. Dropping this one poll models an unrelated actor command cancelling the await.
    assert!(alice.drive_mesh_repair().now_or_never().is_none());
    assert_eq!(calls.load(Ordering::Acquire), 2);
    assert_eq!(alice.repair_target_after, Some(ids[3]));
    let first = alice.pending_indirect_probes[0].1.helper.device;
    let helper = &mut members[ids[1..3].iter().position(|id| *id == first).unwrap()];
    serve_one_test_request(helper).await;
    assert_eq!(helper.repair_outbox.len(), 1);
    helper.drain_repair_outbox().await;
    serve_one_test_request(&mut alice).await;
    assert!(alice.indirect_reachability[&ids[3]]
        .negative_responses
        .contains(&first));
    // The second, never-completed pending probe keeps its original finite expiry. Once it
    // expires, its cancellation must not restore the first target's old priority monopoly.
    clock.advance_ms(REPAIR_INTENT_LIFETIME_MS + 1);
    alice.queue_mesh_repair().await;
    assert_eq!(alice.repair_target_after, Some(ids[4]));
    assert_eq!(alice.pending_indirect_probes.len(), 2);
    assert!(alice
        .pending_indirect_probes
        .iter()
        .all(|(_, pending)| pending.target.device == ids[4]));
}

#[tokio::test]
async fn receipts_and_repair_notifications_both_gain_admission_as_actual_slots_retire() {
    use automerge::{transaction::Transactable, ROOT};
    use std::task::Poll;
    let (_hub, mut members, ids) = build_members(2).await;
    converge_and_publish_test_routes(&mut members);
    let mut a = members.remove(0);
    let mut b = members.remove(0);
    crate::tests::prove_live_member(&mut b, a.local_peer(), ids[0]);
    a.open_channel(DocType::Channel, 913).await.unwrap();
    b.open_channel(DocType::Channel, 913).await.unwrap();
    a.post(DocType::Channel, 913, |doc| doc.put(ROOT, "one", 1))
        .await
        .unwrap();
    let Some(TransportEvent::Gossip { from, data, .. }) = b.transport.next_event().await else {
        panic!("real signed op")
    };
    b.on_gossip(from, &data);
    let receipt = b.delivery_receipt_outbox.pop_front().unwrap();
    b.catchup_queue.clear();
    b.recovery_submissions = Submissions::with_capacity(isolated_capacity());
    let mut lower_owners: Vec<SharedRequestKeepalive> = (0..LOCAL_SUBMISSIONS)
        .map(|_| {
            let original = b.recovery_submissions.reserve().unwrap();
            let lower: SharedRequestKeepalive = original.clone();
            drop(original); // Caller retirement cannot refund the sole remaining actual owner.
            lower
        })
        .collect();
    let mut receipt_admitted = 0;
    let mut repair_admitted = 0;
    for _ in 0..8 {
        if b.delivery_receipt_outbox.is_empty() {
            b.delivery_receipt_outbox.push_back(receipt);
        }
        if b.repair_outbox.is_empty() {
            b.repair_outbox.push_back(RepairOutboxItem {
                target: a.local_peer(),
                payload: vec![0],
                expires_at_ms: u64::MAX,
                required_refs: vec![b.descriptor_ref(ids[0]).unwrap()],
            });
        }
        assert_eq!(b.recovery_submissions.local.available_permits(), 0);
        let mut full = Box::pin(b.run_once());
        let _ = futures::future::poll_fn(|cx| Poll::Ready(full.as_mut().poll(cx))).await;
        drop(full);
        assert_eq!(
            b.delivery_receipt_outbox.len(),
            1,
            "Busy preserves receipt intent"
        );
        assert_eq!(b.repair_outbox.len(), 1, "Busy preserves repair intent");
        drop(lower_owners.pop().unwrap());
        let mut owner = Box::pin(b.run_once());
        let _ = futures::future::poll_fn(|cx| Poll::Ready(owner.as_mut().poll(cx))).await;
        drop(owner);
        receipt_admitted += usize::from(b.delivery_receipt_outbox.is_empty());
        repair_admitted += usize::from(b.repair_outbox.is_empty());
        // The real MemNetwork queued responder now holds any newly submitted Flight. Do not
        // consume that remote request: each following iteration releases exactly one old owner.
        assert_eq!(b.recovery_submissions.local.available_permits(), 0);
    }
    assert!(receipt_admitted > 0 && repair_admitted > 0,
        "both classes need admission under replenished receipt traffic: receipts={receipt_admitted}, repairs={repair_admitted}");
}
