use super::*;
use crate::tests::{build_members, converge_and_publish_test_routes};
use futures::{FutureExt, StreamExt};

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
