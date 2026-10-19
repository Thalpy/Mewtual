use super::*;
use catcoms_rt::ManualClock;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

pub(super) async fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    let mut future = future;
    poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
}

type TestMember = ChannelSync<catcoms_rt::MemNetwork, ChaCha20Rng>;

/// Deliver the actual request and authenticated response owned by PendingCatchup. Every gossip
/// event is deliberately discarded, so only this released request path can add document state.
pub(super) async fn answer_owned_request(
    requester: &mut TestMember,
    source: &mut TestMember,
    legacy: bool,
) {
    let pending = requester
        .pending_catchup
        .as_mut()
        .expect("an owned request");
    assert_eq!(pending.peer, source.local_peer());
    assert!(poll_once(pending.response.as_mut()).await.is_pending());
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match source.transport.next_event().await.expect("live provider") {
                TransportEvent::Request {
                    from,
                    data,
                    responder,
                    ..
                } => {
                    let bytes = if legacy && data.first() == Some(&KIND_CATCHUP_SINCE) {
                        Vec::new() // The released pre-paging unsupported response.
                    } else {
                        source.handle_request(from, &data)
                    };
                    responder.respond(Bytes::from(bytes));
                    break;
                }
                TransportEvent::Gossip { .. } => {}
                event => panic!("unexpected provider event: {event:?}"),
            }
        }
        requester
            .pending_catchup
            .as_mut()
            .unwrap()
            .response
            .as_mut()
            .await
    })
    .await
    .expect("owned request completed");
    requester.complete_queued_catchup(response);
}

#[tokio::test]
async fn positive_owned_catchup_keeps_searching_other_bound_sources_at_the_new_frontier() {
    use automerge::transaction::Transactable;
    use automerge::ROOT;
    for legacy in [false, true] {
        let (_hub, mut members, ids) = crate::tests::build_members(3).await;
        crate::tests::converge_and_publish_test_routes(&mut members);
        let mut c = members.pop().unwrap();
        let mut b = members.pop().unwrap();
        let mut a = members.pop().unwrap();
        let clock = ManualClock::new(1_000);
        for node in [&mut a, &mut b, &mut c] {
            node.clock = Arc::new(clock.clone());
        }
        for node in [&mut a, &mut b, &mut c] {
            node.open_channel(DocType::Channel, 902).await.unwrap();
        }
        crate::tests::prove_live_member(&mut b, a.local_peer(), ids[0]);
        crate::tests::prove_live_member(&mut b, c.local_peer(), ids[2]);
        a.post(DocType::Channel, 902, |doc| {
            doc.put(ROOT, "from_a", "original_a")
        })
        .await
        .unwrap();
        c.post(DocType::Channel, 902, |doc| {
            doc.put(ROOT, "from_c", "original_c")
        })
        .await
        .unwrap();
        // The requester receives neither live operation. Its only obligation is the initial
        // queued search; the harness never schedules a discovery/reconciliation retry for it.
        for _ in 0..2 {
            assert!(matches!(
                b.transport.next_event().await,
                Some(TransportEvent::Gossip { .. })
            ));
        }
        b.catchup_queue.clear();
        b.pending_catchup = None;
        b.catchup_inflight = None;
        b.enqueue_doc_catchup(DocType::Channel, 902);
        for _ in 0..16 {
            b.start_queued_catchup();
            let Some(pending) = &b.pending_catchup else {
                if b.catchup_queue.is_empty() {
                    break;
                }
                // The existing per-source cooldown remains a real obligation. Let the injected
                // clock expire it; no discovery or manual requeue is supplied by the harness.
                clock.advance_ms(CATCHUP_PEER_COOLDOWN_MS + 1_000);
                continue;
            };
            if pending.peer == a.local_peer() {
                answer_owned_request(&mut b, &mut a, legacy).await;
            } else {
                answer_owned_request(&mut b, &mut c, legacy).await;
            }
        }
        assert_eq!(
            b.doc_version(DocType::Channel, 902),
            2,
            "both original signed operations are imported, legacy={legacy}"
        );
        assert_eq!(b.sources_checked(DocType::Channel, 902).len(), 2);
        assert!(b.catchup_queue.is_empty() && b.pending_catchup.is_none());
    }
}

#[tokio::test]
async fn quiet_completed_search_has_an_owned_reconciliation_wake_without_discovery_commands() {
    use automerge::transaction::Transactable;
    use automerge::ROOT;
    let (_hub, mut members, ids) = crate::tests::build_members(2).await;
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    let clock = ManualClock::new(1_000);
    b.clock = Arc::new(clock.clone());
    a.open_channel(DocType::Channel, 903).await.unwrap();
    b.open_channel(DocType::Channel, 903).await.unwrap();
    crate::tests::prove_live_member(&mut b, a.local_peer(), ids[0]);
    assert!(b.schedule_reconciliation() > 0);
    b.catchup_queue.clear();
    b.pending_catchup = None;
    b.catchup_inflight = None;
    b.note_source_checked(a.local_peer(), DocType::Channel, 903);
    let mut idle = Box::pin(b.run_once());
    assert!(poll_once(idle.as_mut()).await.is_pending());
    clock.advance_ms(CATCHUP_PEER_COOLDOWN_MS - 1);
    assert!(poll_once(idle.as_mut()).await.is_pending());
    drop(idle); // Repeated actor commands must not renew the original timer anchor.
    let anchor = b.reconciliation_next_ms;
    let mut idle = Box::pin(b.run_once());
    assert!(poll_once(idle.as_mut()).await.is_pending());
    clock.advance_ms(1);
    // Inbound data is also ready at the deadline. The bounded local timer gets its turn,
    // then the ordinary event remains available and must be consumed on the next owner turn.
    a.post(DocType::Channel, 903, |doc| {
        doc.put(ROOT, "ready", "live operation")
    })
    .await
    .unwrap();
    assert!(
        matches!(poll_once(idle.as_mut()).await, Poll::Ready(Ok(true))),
        "the sole owner must wake its quiet reconciliation without a UI/network command"
    );
    drop(idle);
    assert!(b.catchup_queue.contains(&CatchupTask::Doc {
        doc_type: DocType::Channel,
        doc_id: 903
    }));
    assert!(b.sources_checked(DocType::Channel, 903).is_empty());
    assert_eq!(
        b.doc_version(DocType::Channel, 903),
        0,
        "the timer won one turn before ready gossip"
    );
    assert!(b.reconciliation_next_ms > anchor);
    assert_eq!(
        b.next_reconciliation_delay(),
        Some(CATCHUP_PEER_COOLDOWN_MS)
    );
    b.run_once().await.unwrap();
    assert_eq!(
        b.doc_version(DocType::Channel, 903),
        1,
        "ordinary inbound progress was not starved"
    );
}

#[tokio::test]
async fn quiet_reconciliation_has_no_authority_from_unbound_or_disconnected_sources() {
    let (_hub, mut members, ids) = crate::tests::build_members(2).await;
    let mut b = members.pop().unwrap();
    let a = members.pop().unwrap();
    b.open_channel(DocType::Channel, 904).await.unwrap();
    b.note_peer_connected(a.local_peer());
    b.promote_member_peer_bound(a.local_peer(), ids[0], false);
    assert!(
        b.next_reconciliation_delay().is_none(),
        "unbound member proof grants no mandatory background search"
    );
    b.promote_member_peer_bound(a.local_peer(), ids[0], true);
    assert!(b.next_reconciliation_delay().is_some());
    let peer = a.local_peer();
    drop(a);
    assert!(
        b.connected_peers.contains(&peer),
        "leave the ordered disconnect observation stale"
    );
    assert!(
        b.next_reconciliation_delay().is_none(),
        "actual disconnected source cannot arm the timer"
    );
}

#[tokio::test]
async fn quiet_owned_timer_imports_new_original_history_without_gossip_or_discovery() {
    use automerge::transaction::Transactable;
    use automerge::ROOT;
    let (_hub, mut members, ids) = crate::tests::build_members(2).await;
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    let clock = ManualClock::new(1_000);
    for node in [&mut a, &mut b] {
        node.clock = Arc::new(clock.clone());
        node.open_channel(DocType::Channel, 907).await.unwrap();
    }
    crate::tests::prove_live_member(&mut b, a.local_peer(), ids[0]);
    b.schedule_reconciliation();
    b.catchup_queue.clear();
    b.note_source_checked(a.local_peer(), DocType::Channel, 907);
    a.post(DocType::Channel, 907, |doc| {
        doc.put(ROOT, "quiet", "original signed history")
    })
    .await
    .unwrap();
    assert!(
        matches!(
            b.transport.next_event().await,
            Some(TransportEvent::Gossip { .. })
        ),
        "discard the sole live operation"
    );
    let mut idle = Box::pin(b.run_once());
    assert!(poll_once(idle.as_mut()).await.is_pending());
    clock.advance_ms(CATCHUP_PEER_COOLDOWN_MS);
    assert!(matches!(
        poll_once(idle.as_mut()).await,
        Poll::Ready(Ok(true))
    ));
    drop(idle);
    assert_eq!(b.doc_version(DocType::Channel, 907), 0);
    // No manual requeue or discovery call intervenes: the actual next owner turn issues its
    // timer-created request, and the real current member signs the original request's response.
    let (outcome, ()) = tokio::join!(b.run_once(), async {
        let Some(TransportEvent::Request {
            from,
            data,
            responder,
            ..
        }) = a.transport.next_event().await
        else {
            panic!("actual timer-triggered request")
        };
        assert_eq!(data.first(), Some(&KIND_CATCHUP_SINCE));
        responder.respond(Bytes::from(a.handle_request(from, &data)));
    });
    outcome.unwrap();
    assert_eq!(b.doc_version(DocType::Channel, 907), 1);
    assert_eq!(b.stats().doc_catchups_requested, 1);
}

#[tokio::test]
async fn quiet_reconciliation_clock_exhaustion_cannot_leave_a_perpetually_due_deadline() {
    for now in [u64::MAX - CATCHUP_PEER_COOLDOWN_MS + 1, u64::MAX] {
        let (_hub, mut members, ids) = crate::tests::build_members(2).await;
        let mut b = members.pop().unwrap();
        let a = members.pop().unwrap();
        b.open_channel(DocType::Channel, 905).await.unwrap();
        crate::tests::prove_live_member(&mut b, a.local_peer(), ids[0]);
        b.clock = Arc::new(ManualClock::new(now));
        b.catchup_queue.clear();
        b.reconciliation_next_ms = now;
        for _ in 0..3 {
            assert_eq!(b.schedule_reconciliation(), 0);
            assert!(b.next_reconciliation_delay().is_none());
            assert!(b.catchup_queue.is_empty());
        }
    }
}

#[tokio::test]
async fn quiet_reconciliation_stops_when_signed_removal_retires_local_membership() {
    let (_hub, mut members, ids) = crate::tests::build_members(2).await;
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    b.open_channel(DocType::Channel, 906).await.unwrap();
    crate::tests::prove_live_member(&mut b, a.local_peer(), ids[0]);
    assert!(b.next_reconciliation_delay().is_some());
    a.commit_remove_now(&ids[1]);
    assert!(b.apply_commit_in_order(a.commit_log.back().unwrap()));
    assert!(b.next_reconciliation_delay().is_none());
    assert_eq!(b.schedule_reconciliation(), 0);
}

#[derive(Debug)]
struct PausedPublisher {
    peer: PeerId,
    allow: Arc<AtomicBool>,
    submitted: Arc<Mutex<Vec<Vec<u8>>>>,
}

#[async_trait::async_trait]
impl MeshTransport for PausedPublisher {
    fn local_peer(&self) -> PeerId {
        self.peer
    }
    async fn subscribe(&self, _: Topic) -> Result<(), TransportError> {
        Ok(())
    }
    async fn unsubscribe(&self, _: Topic) -> Result<(), TransportError> {
        Ok(())
    }
    async fn publish(&self, _: Topic, data: Bytes) -> Result<(), TransportError> {
        if !self.allow.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        self.submitted.lock().unwrap().push(data.to_vec());
        Ok(())
    }
    async fn request(&self, _: PeerId, _: ProtocolId, _: Bytes) -> Result<Bytes, TransportError> {
        std::future::pending().await
    }
    async fn request_cancellable(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        data: Bytes,
        mut cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        tokio::select! {
            result = self.request(peer, protocol, data) => result,
            _ = cancellation.cancelled() => Err(TransportError::Cancelled),
        }
    }
    async fn next_event(&self) -> Option<TransportEvent> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn interrupted_commit_wait_merges_newly_detected_gap_into_one_obligation() {
    let (_, mut members, _) = crate::tests::build_members(2).await;
    let mut node = members.pop().unwrap();
    let epoch = node.group.epoch();
    node.catchup_queue.clear();
    let held = CatchupTask::Commits {
        from_epoch: epoch,
        gap_at: Some(epoch + 1),
        avoid: Some(PeerId::from_u64(8)),
    };
    node.catchup_inflight = Some((held, Some(PeerId::from_u64(7))));
    node.enqueue_commit_catchup_for(epoch + 1, Some(epoch + 3), Some(PeerId::from_u64(9)));
    node.requeue_catchup(held);
    assert_eq!(
        node.catchup_queue,
        vec![CatchupTask::Commits {
            from_epoch: epoch,
            gap_at: Some(epoch + 3),
            avoid: None,
        }],
        "a stale completion cannot duplicate or weaken the newer proven gap"
    );
}

#[tokio::test]
async fn stale_commit_wait_keeps_its_stronger_gap_when_a_bare_probe_fills_the_queue() {
    let (_, mut members, ids) = crate::tests::build_members(2).await;
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    // The bare core fixture has no host handoff; remember its actual admission contact only
    // as an untrusted candidate. Response authority still comes from the signed exchange.
    b.note_candidate_peer(a.local_peer());
    let epoch = b.group.epoch();
    b.config.max_catchup_queue = 1;
    b.catchup_queue.clear();
    b.enqueue_commit_catchup_for(epoch, Some(epoch + 5), None);
    b.start_queued_catchup();
    let pending = b
        .pending_catchup
        .as_mut()
        .expect("strong gap owns a request");
    assert!(
        poll_fn(|cx| Poll::Ready(pending.response.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    b.enqueue_commit_catchup_for(epoch, None, None);
    assert_eq!(b.catchup_queue.len(), 1, "the sole queue slot is full");
    assert!(matches!(
        b.catchup_queue[0],
        CatchupTask::Commits { gap_at: None, .. }
    ));

    a.commit_remove_now(&ids[1]);
    assert!(b.apply_commit_in_order(a.commit_log.back().unwrap()));
    assert!(b.group.epoch() > epoch);
    b.start_queued_catchup(); // Drop the obsolete wait and merge its admitted obligation.
    assert!(b.pending_catchup.is_none());
    assert_eq!(
        b.catchup_queue,
        vec![CatchupTask::Commits {
            from_epoch: epoch,
            gap_at: Some(epoch + 5),
            avoid: None,
        }],
        "capacity cannot let a weaker probe erase the held proven gap"
    );
}

#[tokio::test]
async fn cancelled_owner_polls_retain_the_original_signed_control_publication() {
    let (_, mut members, ids) = crate::tests::build_members(2).await;
    let mut original = members.remove(0);
    let saved = original.snapshot().unwrap();
    let allow = Arc::new(AtomicBool::new(false));
    let submitted = Arc::new(Mutex::new(Vec::new()));
    let mut owner = ChannelSync::restore(
        &saved,
        PausedPublisher {
            peer: original.local_peer(),
            allow: Arc::clone(&allow),
            submitted: Arc::clone(&submitted),
        },
        ChaCha20Rng::seed_from_u64(90),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    // Actual MLS removal creates the accepted, originally signed control frame.
    owner.commit_remove_now(&ids[1]);
    let expected = owner.outbox.clone();
    assert!(!expected.is_empty());
    for _ in 0..64 {
        let mut tick = Box::pin(owner.run_once());
        assert!(poll_once(tick.as_mut()).await.is_pending());
        drop(tick); // exactly the cancellation caused by a ready actor command
        assert_eq!(
            owner.outbox, expected,
            "a pending publisher cannot lose admitted signed bytes"
        );
    }
    assert!(submitted.lock().unwrap().is_empty());
    allow.store(true, Ordering::SeqCst);
    let mut tick = Box::pin(owner.run_once());
    assert!(poll_once(tick.as_mut()).await.is_pending());
    drop(tick);
    assert!(owner.outbox.is_empty());
    assert_eq!(
        *submitted.lock().unwrap(),
        expected
            .into_iter()
            .map(|(_, bytes)| bytes)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn removed_member_discards_owned_response_without_retiring_its_recovery_obligation() {
    use automerge::{transaction::Transactable, ROOT};
    let (_, mut members, ids) = crate::tests::build_members(2).await;
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    b.note_candidate_peer(a.local_peer());
    a.open_channel(DocType::Channel, 901).await.unwrap();
    b.open_channel(DocType::Channel, 901).await.unwrap();
    a.post(DocType::Channel, 901, |doc| {
        doc.put(ROOT, "held", "signed history")
    })
    .await
    .unwrap();
    // Discard live delivery so only the held request can supply this history.
    assert!(matches!(
        b.transport.next_event().await,
        Some(TransportEvent::Gossip { .. })
    ));
    b.catchup_queue.clear();
    b.enqueue_doc_catchup(DocType::Channel, 901);
    let before = b.doc_version(DocType::Channel, 901);
    let checkpoint = b.snapshot().unwrap();
    b.start_queued_catchup();
    let request = b.pending_catchup.as_mut().expect("queued request");
    assert!(
        poll_fn(|cx| Poll::Ready(request.response.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    let Some(TransportEvent::Request {
        from,
        data,
        responder,
        ..
    }) = a.transport.next_event().await
    else {
        panic!("real catch-up request");
    };
    let bytes = a.handle_request(from, &data);
    assert!(!bytes.is_empty());
    let control_hub = catcoms_rt::Hub::new();
    let mut control = ChannelSync::restore(
        &checkpoint,
        control_hub.join(b.local_peer()),
        ChaCha20Rng::seed_from_u64(91),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    let admitted = control
        .apply_catchup_since_response(
            a.local_peer(),
            DocType::Channel,
            901,
            b.pending_catchup.as_ref().unwrap().auth,
            &bytes,
        )
        .unwrap()
        .unwrap();
    assert!(
        admitted > 0,
        "the exact answer passes request binding and admission before removal"
    );
    assert!(control.doc_version(DocType::Channel, 901) > before);
    // The response is valid for its original request, but a real signed removal now retires us.
    a.commit_remove_now(&ids[1]);
    let removal = a.commit_log.back().unwrap().clone();
    assert!(b.apply_commit_in_order(&removal));
    assert!(!b.group.is_active());
    responder.respond(Bytes::from(bytes.clone()));
    let response = b
        .pending_catchup
        .as_mut()
        .unwrap()
        .response
        .as_mut()
        .await
        .unwrap();
    assert_eq!(
        response.as_ref(),
        bytes.as_slice(),
        "the owned wait returned the valid response"
    );
    b.complete_queued_catchup(Ok(response));
    assert_eq!(b.doc_version(DocType::Channel, 901), before);
    assert!(b.catchup_queue.contains(&CatchupTask::Doc {
        doc_type: DocType::Channel,
        doc_id: 901
    }));
    b.start_queued_catchup();
    assert!(
        b.pending_catchup.is_none(),
        "removed local membership cannot start a replacement request"
    );
}

#[tokio::test]
async fn newly_bound_second_source_reopens_completed_history_without_a_periodic_wait() {
    use automerge::{transaction::Transactable, ROOT};
    let (_hub, mut members, ids) = crate::tests::build_members(3).await;
    crate::tests::converge_and_publish_test_routes(&mut members);
    let mut c = members.pop().unwrap();
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    let clock = ManualClock::new(1_000);
    for node in [&mut a, &mut b, &mut c] {
        node.clock = Arc::new(clock.clone());
        node.open_channel(DocType::Channel, 911).await.unwrap();
        node.open_channel(DocType::Channel, 912).await.unwrap();
    }
    // A's only completed source was empty B. Discharge the restore's one-time first proof
    // independently: the next source must wake ordinary owned recovery too.
    a.first_proof_sweep_owed = true;
    crate::tests::prove_live_member(&mut a, b.local_peer(), ids[1]);
    assert!(!a.first_proof_sweep_owed);
    a.catchup_queue.clear();
    a.catchup_inflight = None;
    a.pending_catchup = None;
    a.note_source_checked(b.local_peer(), DocType::Channel, 911);
    a.note_source_checked(b.local_peer(), DocType::Channel, 912);
    a.reconciliation_next_ms = clock.monotonic_ms() + CATCHUP_PEER_COOLDOWN_MS;
    c.post(DocType::Channel, 911, |doc| {
        doc.put(ROOT, "unique", "original signed operation")
    })
    .await
    .unwrap();
    assert!(matches!(
        a.transport.next_event().await,
        Some(TransportEvent::Gossip { .. })
    ));
    // Its connection arrived before endpoint authentication. PEX's unbound proof cannot
    // grant a background sweep. A real signed exchange on another document later binds C.
    a.note_peer_connected(c.local_peer());
    a.promote_member_peer(c.local_peer(), ids[2]);
    assert!(a.catchup_queue.is_empty());
    a.begin_catchup_wait(
        CatchupTask::Doc {
            doc_type: DocType::Channel,
            doc_id: 912,
        },
        c.local_peer(),
        Shape::Since,
    )
    .unwrap();
    answer_owned_request(&mut a, &mut c, false).await;
    assert_eq!(a.doc_version(DocType::Channel, 911), 0);
    // No discovery, gossip, manual queue or time advance follows the bound promotion.
    let mut owner = Box::pin(a.run_once());
    let _ = poll_once(owner.as_mut()).await;
    drop(owner);
    assert!(
        a.pending_catchup.is_some() || !a.catchup_queue.is_empty(),
        "new bound source must create prompt owned recovery work"
    );
    for _ in 0..8 {
        a.start_queued_catchup();
        let Some(pending) = &a.pending_catchup else {
            break;
        };
        if pending.peer == b.local_peer() {
            answer_owned_request(&mut a, &mut b, false).await;
        } else {
            answer_owned_request(&mut a, &mut c, false).await;
        }
        if a.doc_version(DocType::Channel, 911) == 1 {
            break;
        }
    }
    assert_eq!(a.doc_version(DocType::Channel, 911), 1);
    assert_eq!(
        clock.monotonic_ms(),
        1_000,
        "recovery did not spend a periodic interval"
    );
    a.catchup_queue.clear();
    a.pending_catchup = None;
    a.catchup_inflight = None;
    a.promote_member_peer_bound(c.local_peer(), ids[2], true);
    let mut repeated = Box::pin(a.run_once());
    let _ = poll_once(repeated.as_mut()).await;
    drop(repeated);
    assert!(
        a.pending_catchup.is_none() && a.catchup_queue.is_empty(),
        "repeat proof cannot reopen a completed source search"
    );
}
