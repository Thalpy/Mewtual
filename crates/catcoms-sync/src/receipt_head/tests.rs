use super::*;
use crate::tests::build_members;
use catcoms_replication::InheritedCheckpoint;
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;
mod service;
mod studio;
fn node() -> (Node, ManualClock) {
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let clock = ManualClock::new(1000);
    (
        Node::new(
            Hub::new().join(PeerId::from_u64(1)),
            group,
            device,
            ChaCha20Rng::seed_from_u64(817),
            Box::new(clock.clone()),
        ),
        clock,
    )
}
fn receipt(node: &Node, bucket: u8) -> Receipt {
    Receipt::sign(
        registry_document(&node.group.group_id(), bucket).unwrap(),
        0,
        [7; 32],
        [8; 32],
        0,
        InheritedCheckpoint::EpochZero,
        &node.device,
    )
    .unwrap()
}
fn enqueue(node: &mut Node, bucket: u8) -> catcoms_rt::ResponderRx {
    let document = registry_document(&node.group.group_id(), bucket).unwrap();
    let query = encode_query(&document, [3; 16]).unwrap();
    let (request, _) = node
        .build_authed_request(KIND_RECEIPT_HEAD, &query)
        .unwrap();
    let (tx, rx) = Responder::channel();
    node.queue_receipt_head(node.transport.local_peer(), &request[1..], tx);
    rx
}
fn selection(receipt: Receipt, prove: bool) -> Result<ReceiptHeadSelection, ()> {
    Ok(ReceiptHeadSelection {
        receipt: Some(receipt),
        prove,
    })
}

#[tokio::test]
async fn receipt_head_handoff_requires_an_accepted_owner_proof_not_a_hint_or_dropped_reply() {
    let (mut n, clock) = node();
    let watch = n.watch_registry_head(4);
    let permit = n
        .prepare_receipt_head_snapshot(|_, _| Ok::<_, ()>(()))
        .unwrap()
        .unwrap();
    let receipt = receipt(&n, 4);
    let rx = enqueue(&mut n, 4);
    let served = n
        .serve_receipt_head_with_handoff(&watch, Some(&permit), |_, _, _, _| {
            selection(receipt.clone(), true)
        })
        .unwrap()
        .unwrap()
        .unwrap();
    let ReceiptHeadServed::Owner(handoff) = served else {
        panic!("owner handoff");
    };
    // Acceptance is observable before the receiver reads; this deliberately is not a delivery ACK.
    assert_eq!(
        n.with_receipt_head_handoff(handoff, |r, _| r.hash())
            .unwrap(),
        receipt.hash()
    );
    assert!(rx.recv().await.is_some());
    clock.advance_ms(1000);
    let rx = enqueue(&mut n, 4);
    drop(rx);
    assert!(matches!(
        n.serve_receipt_head_with_handoff(&watch, Some(&permit), |_, _, _, _| {
            selection(receipt.clone(), true)
        }),
        Err(SyncError::Transport(TransportError::Closed))
    ));
    clock.advance_ms(1000);
    let rx = enqueue(&mut n, 4);
    assert!(matches!(
        n.serve_receipt_head_with_handoff(&watch, None, |_, _, _, _| { selection(receipt, false) })
            .unwrap()
            .unwrap()
            .unwrap(),
        ReceiptHeadServed::Hint
    ));
    assert!(rx.recv().await.is_some());
}

#[tokio::test]
async fn receipt_head_handoff_completion_rechecks_expiry_watch_membership_and_runtime() {
    for change in ["expiry", "watch", "membership", "runtime"] {
        let (mut n, clock) = node();
        let watch = n.watch_registry_head(4);
        let permit = n
            .prepare_receipt_head_snapshot(|_, _| Ok::<_, ()>(()))
            .unwrap()
            .unwrap();
        let receipt = receipt(&n, 4);
        let rx = enqueue(&mut n, 4);
        let served = n
            .serve_receipt_head_with_handoff(&watch, Some(&permit), |_, _, _, _| {
                selection(receipt, true)
            })
            .unwrap()
            .unwrap()
            .unwrap();
        let ReceiptHeadServed::Owner(handoff) = served else {
            panic!("owner handoff");
        };
        match change {
            "expiry" => {
                clock.advance_ms(QUEUE_MS);
            }
            "watch" => {
                n.watch_registry_head(4);
            }
            "membership" => {
                let peer = MlsDevice::generate().unwrap();
                n.with_observed_mls_transition(|n| {
                    n.group.add_member(&n.device, peer.key_package().unwrap())
                })
                .unwrap();
            }
            "runtime" => {
                n = Node::restore(
                    &n.snapshot().unwrap(),
                    Hub::new().join(PeerId::from_u64(2)),
                    ChaCha20Rng::seed_from_u64(8),
                    Box::new(clock.clone()),
                )
                .unwrap();
                n.watch_registry_head(4);
            }
            _ => unreachable!(),
        }
        assert!(n
            .with_receipt_head_handoff(handoff, |_, _| panic!("stale completion"))
            .is_err());
        // Refusing completion does not retract an already accepted response.
        assert!(rx.recv().await.is_some());
    }
}

#[test]
fn receipt_head_wire_is_exact_bounded_and_scope_checked() {
    let document = LogicalDocument::new(vec![3; 16], DocType::DocRegistry, vec![7; 32]).unwrap();
    let bytes = encode_query(&document, [9; 16]).unwrap();
    let mut expected = vec![1, 0, 18, 0, 0, 0, 32];
    expected.extend([7; 32]);
    expected.extend([0, 0, 0, 16]);
    expected.extend([9; 16]);
    assert_eq!(bytes, expected);
    assert_eq!(
        decode_query(&bytes, &document.server_id).unwrap(),
        (document.clone(), [9; 16])
    );
    for end in 0..bytes.len() {
        assert!(decode_query(&bytes[..end], &document.server_id).is_err());
    }
    for offset in [0, 2, 6, 42] {
        let mut bad = bytes.clone();
        bad[offset] ^= 1;
        assert!(decode_query(&bad, &document.server_id).is_err());
    }
    assert!(decode_query(&vec![0; MAX_QUERY + 1], &document.server_id).is_err());
    let empty = ReceiptHeadAnswer {
        receipt: None,
        repair: None,
        proof: None,
    };
    let wire = encode_answer(&empty, &document).unwrap();
    assert_eq!(wire, [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(decode_answer(&wire, &document).unwrap(), empty);
    let mut trailing = wire;
    trailing.push(0);
    assert!(decode_answer(&trailing, &document).is_err());
    assert!(decode_response(&vec![0; MAX_ANSWER + 109]).is_err());
    let (node, _) = node();
    let r = receipt(&node, 4);
    let mut answer = ReceiptHeadAnswer {
        proof: Some(
            ReceiptHeadProof::sign(&r, node.device.device_id(), [5; 16], &node.device).unwrap(),
        ),
        receipt: Some(r.clone()),
        repair: None,
    };
    let bytes = encode_answer(&answer, &r.document).unwrap();
    assert_eq!(decode_answer(&bytes, &r.document).unwrap(), answer);
    assert!(decode_answer(&bytes, &document).is_err());
    answer.receipt = None;
    assert!(encode_answer(&answer, &r.document).is_err());
}

#[tokio::test]
async fn receipt_head_snapshot_barrier_watch_expiry_and_stale_permits_fail_closed() {
    let (mut node, clock) = node();
    let watch = node.watch_registry_head(4);
    assert!(node
        .prepare_receipt_head_snapshot(|_, _| Err::<(), _>("flush failed"))
        .unwrap()
        .is_err());
    let mut saved = Vec::new();
    let permit = node
        .prepare_receipt_head_snapshot(|bytes, _| {
            saved = bytes.to_vec();
            Ok::<_, ()>(())
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        node.with_durable_owner_snapshot(&permit, |group, device, _, tenure| {
            assert_eq!(group.designated_committer(), Some(device.device_id()));
            tenure
        })
        .unwrap(),
        0
    );
    assert!(!saved.is_empty());
    let r = receipt(&node, 4);
    let rx = enqueue(&mut node, 4);
    assert!(node
        .serve_receipt_head(&watch, Some(&permit), |_, _, _, source| {
            assert_eq!(source.tenure, Some(0));
            selection(r.clone(), true)
        })
        .unwrap()
        .unwrap()
        .is_ok());
    let response = rx.recv().await.unwrap();
    let (_, _, bytes) = decode_response(&response).unwrap();
    assert!(decode_answer(bytes, &r.document).unwrap().proof.is_some());
    let rx = enqueue(&mut node, 4);
    clock.advance_ms(QUEUE_MS);
    assert!(node
        .serve_receipt_head::<()>(&watch, Some(&permit), |_, _, _, _| panic!(
            "expired before I/O"
        ))
        .unwrap()
        .is_none());
    assert!(rx.recv().await.is_none());
    let replacement = node.watch_registry_head(4);
    assert!(node.unwatch_registry_head(&watch).is_err());
    let joined = MlsDevice::generate().unwrap();
    node.with_observed_mls_transition(|n| {
        n.group.add_member(&n.device, joined.key_package().unwrap())
    })
    .unwrap();
    assert!(!node.head_snapshot_is_current(&permit));
    assert!(node
        .with_durable_owner_snapshot(&permit, |_, _, _, _| panic!(
            "stale snapshot must not enter owner transaction"
        ))
        .is_err());
    let rx = enqueue(&mut node, 4);
    assert!(node
        .serve_receipt_head(&replacement, Some(&permit), |_, _, _, s| {
            assert_eq!(s.tenure, None);
            selection(r.clone(), false)
        })
        .unwrap()
        .unwrap()
        .is_ok());
    assert!(rx.recv().await.is_some());
    clock.advance_ms(1000);
    let rx = enqueue(&mut node, 4);
    assert!(node
        .serve_receipt_head(&replacement, None, |_, _, _, _| selection(r.clone(), true))
        .is_err());
    assert!(rx.recv().await.is_none());
    let mut restored = Node::restore(
        &saved,
        Hub::new().join(PeerId::from_u64(2)),
        ChaCha20Rng::seed_from_u64(8),
        Box::new(clock),
    )
    .unwrap();
    assert!(!restored.head_snapshot_is_current(&permit));
    assert!(restored
        .with_durable_owner_snapshot(&permit, |_, _, _, _| panic!(
            "replaced runtime must not enter owner transaction"
        ))
        .is_err());
}

#[tokio::test]
async fn receipt_head_serving_rechecks_time_after_source_and_never_sends_stale_success() {
    let (mut node, clock) = node();
    let watch = node.watch_registry_head(4);
    let rx = enqueue(&mut node, 4);
    let r = receipt(&node, 4);
    assert!(node
        .serve_receipt_head(&watch, None, |_, _, _, _| {
            clock.advance_ms(QUEUE_MS);
            selection(r, false)
        })
        .is_err());
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn receipt_head_request_authentication_binds_transport_kind_and_current_membership() {
    let (mut node, _) = node();
    let watch = node.watch_registry_head(4);
    let query = encode_query(
        &registry_document(&node.group.group_id(), 4).unwrap(),
        [4; 16],
    )
    .unwrap();
    for kind in [KIND_REGISTRY_PAGE, KIND_RECEIPT_HEAD] {
        let (bytes, _) = node.build_authed_request(kind, &query).unwrap();
        let (tx, rx) = Responder::channel();
        let from = if kind == KIND_RECEIPT_HEAD {
            PeerId::from_u64(222)
        } else {
            node.transport.local_peer()
        };
        node.queue_receipt_head(from, &bytes[1..], tx);
        assert!(rx.recv().await.is_none());
    }
    assert!(node.receipt_heads.pending.is_empty());
    let rx = enqueue(&mut node, 4);
    node.unwatch_registry_head(&watch).unwrap();
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn receipt_head_client_uses_fresh_nonces_and_checks_current_owner_endpoint() {
    let (_, mut nodes, ids) = build_members(2).await;
    let (a, b) = nodes.split_at_mut(1);
    let owner = &mut a[0];
    let joiner = &mut b[0];
    let peer = owner.local_peer();
    joiner.promote_member_peer_bound(peer, ids[0], true);
    let watch = owner.watch_registry_head(4);
    let permit = owner
        .prepare_receipt_head_snapshot(|_, _| Ok::<_, ()>(()))
        .unwrap()
        .unwrap();
    let r = receipt(owner, 4);
    let mut nonces = Vec::new();
    for _ in 0..2 {
        let (answer, ()) = tokio::join!(joiner.request_registry_head(peer, 4), async {
            owner.run_once().await.unwrap();
            owner
                .serve_receipt_head(&watch, Some(&permit), |_, _, _, source| {
                    nonces.push(source.nonce);
                    selection(r.clone(), true)
                })
                .unwrap()
                .unwrap()
                .unwrap();
        });
        assert_eq!(answer.unwrap().unwrap().receipt.as_ref(), Some(&r));
    }
    assert_ne!(nonces[0], nonces[1]);
    // Disclosure is refused BEFORE requesting an unproven endpoint.
    joiner.member_peers.clear();
    assert!(matches!(
        joiner.request_registry_head(peer, 4).await,
        Err(SyncError::Unauthorized)
    ));
}

#[tokio::test]
async fn receipt_head_cancelled_requests_keep_driver_owned_capacity_until_release() {
    let (_, mut nodes, ids) = build_members(2).await;
    let (a, b) = nodes.split_at_mut(1);
    let owner = &mut a[0];
    let joiner = &mut b[0];
    let peer = owner.local_peer();
    joiner.promote_member_peer_bound(peer, ids[0], true);
    for _ in 0..4 {
        let mut future = Box::pin(joiner.request_registry_head(peer, 4));
        assert!(futures::poll!(future.as_mut()).is_pending());
        drop(future);
    }
    assert_eq!(
        joiner
            .receipt_heads
            .outbound
            .iter()
            .filter(|p| p.strong_count() > 0)
            .count(),
        4
    );
    assert!(joiner.request_registry_head(peer, 4).await.is_err());
    // The four unserved transport requests own their cancellation accounting until drained.
    for _ in 0..4 {
        owner.run_once().await.unwrap();
    }
    assert!(joiner
        .receipt_heads
        .outbound
        .iter()
        .all(|p| p.strong_count() == 0));
}

#[tokio::test]
async fn receipt_head_queue_and_service_limits_survive_rewatch_and_clock_changes() {
    let (mut node, clock) = node();
    let watch = node.watch_registry_head(4);
    let members: Vec<_> = (0..9).map(|_| MlsDevice::generate().unwrap()).collect();
    for member in &members {
        node.with_observed_mls_transition(|n| {
            n.group.add_member(&n.device, member.key_package().unwrap())
        })
        .unwrap();
    }
    let query = encode_query(
        &registry_document(&node.group.group_id(), 4).unwrap(),
        [9; 16],
    )
    .unwrap();
    let mut replies = Vec::new();
    for (n, member) in members.iter().enumerate() {
        let peer = PeerId::from_u64(100 + n as u64);
        let key = member.public_key_bytes();
        let auth = RequestAuth {
            ts: 1000,
            nonce: [n as u8; 16],
            epoch: node.epoch(),
        };
        let sig = member
            .sign(&catchup_auth_transcript(
                &node.group.group_id(),
                KIND_RECEIPT_HEAD,
                &query,
                &key,
                auth.ts,
                &auth.nonce,
                auth.epoch,
                Some(&peer),
            ))
            .unwrap();
        let bytes = encode_authed_request(&query, &key, auth.ts, &auth.nonce, auth.epoch, &sig);
        let (tx, rx) = Responder::channel();
        node.queue_receipt_head(peer, &bytes, tx);
        replies.push(rx);
    }
    assert_eq!(node.receipt_heads.pending.len(), 8);
    assert!(replies.pop().unwrap().recv().await.is_none());
    for _ in 0..4 {
        node.serve_receipt_head(&watch, None, |_, _, _, _| {
            Ok::<_, ()>(ReceiptHeadSelection {
                receipt: None,
                prove: false,
            })
        })
        .unwrap()
        .unwrap()
        .unwrap();
    }
    assert!(node
        .serve_receipt_head::<()>(&watch, None, |_, _, _, _| panic!(
            "service rail before source"
        ))
        .unwrap()
        .is_none());
    clock.advance_ms(499);
    assert!(node
        .serve_receipt_head::<()>(&watch, None, |_, _, _, _| panic!("no early refill"))
        .unwrap()
        .is_none());
    clock.advance_ms(1);
    assert_eq!(
        node.serve_receipt_head(&watch, None, |_, _, _, _| Err::<ReceiptHeadSelection, _>(
            "source failed"
        ))
        .unwrap()
        .unwrap(),
        Err("source failed")
    );
    let replacement = node.watch_registry_head(4);
    assert!(node.receipt_heads.pending.is_empty());
    assert!(
        !node.receipt_heads.requesters.is_empty(),
        "rewatch must not clear charged identity rows"
    );
    assert!(!node.registry_head_watch_is_current(&watch));
    assert!(node.registry_head_watch_is_current(&replacement));
}

async fn spoof(
    client: &mut Node,
    provider: &mut Node,
    make: impl FnOnce(&Node, &Pending) -> ReceiptHeadAnswer,
) -> Result<Option<ReceiptHeadAnswer>, SyncError> {
    let (answer, ()) = tokio::join!(
        client.request_registry_head(provider.local_peer(), 4),
        async {
            provider.run_once().await.unwrap();
            let item = provider.receipt_heads.pending.pop_front().unwrap();
            let document = registry_document(&provider.group.group_id(), 4).unwrap();
            let bytes = encode_answer(&make(provider, &item), &document).unwrap();
            let sig = provider
                .device
                .sign(&transcript(
                    &provider.group.group_id(),
                    &item.key,
                    &item.auth,
                    provider.local_peer(),
                    &item.inner,
                    &bytes,
                ))
                .unwrap();
            item.responder
                .respond(Bytes::from(encode_signed_commit_resp(
                    &provider.device.public_key_bytes(),
                    &sig,
                    &bytes,
                )));
        }
    );
    answer
}

#[tokio::test]
async fn receipt_head_client_rejects_old_inner_proof_and_nonowner_relay_even_with_valid_outer_signature(
) {
    let (_, mut nodes, ids) = build_members(2).await;
    let (a, b) = nodes.split_at_mut(1);
    let owner = &mut a[0];
    let joiner = &mut b[0];
    joiner.promote_member_peer_bound(owner.local_peer(), ids[0], true);
    owner.watch_registry_head(4);
    let r = receipt(owner, 4);
    let old = ReceiptHeadProof::sign(&r, ids[1], [99; 16], &owner.device).unwrap();
    assert!(spoof(joiner, owner, |_, _| ReceiptHeadAnswer {
        receipt: Some(r.clone()),
        proof: Some(old),
        repair: None
    })
    .await
    .is_err());
    // Reverse the request: the actual owner has independent tenure zero. A non-owner endpoint
    // carries a freshly signed owner proof, but may not act as that owner's freshness oracle.
    owner.promote_member_peer_bound(joiner.local_peer(), ids[1], true);
    joiner.watch_registry_head(4);
    // Build the inner proof after capturing the query, with a distinct local owner signer restored
    // from its own key snapshot, avoiding an overlapping borrow of the requester runtime.
    let snapshot = snapshot_server(&owner.device, &owner.group).unwrap();
    let (owner_signer, _) = restore_server(&snapshot).unwrap();
    assert!(spoof(owner, joiner, |_, p| ReceiptHeadAnswer {
        receipt: Some(r.clone()),
        proof: Some(ReceiptHeadProof::sign(&r, ids[0], p.nonce, &owner_signer).unwrap()),
        repair: None
    })
    .await
    .is_err());
}

#[tokio::test]
async fn receipt_head_outer_bindings_and_ready_response_deadline_reject_independently() {
    // Baseline, wrong query, wrong requester, wrong provider transport, another admitted signer,
    // then a correctly bound response already ready when the receiver's deadline has elapsed.
    for case in 0..6 {
        let (_, mut nodes, ids) = build_members(3).await;
        let commits: Vec<_> = nodes[0].commit_log.iter().cloned().collect();
        for record in commits {
            if record.commit_epoch == nodes[1].epoch() {
                assert!(nodes[1].apply_commit_in_order(&record));
            }
        }
        let (a, rest) = nodes.split_at_mut(1);
        let (b, c) = rest.split_at_mut(1);
        let provider = &mut a[0];
        let client = &mut b[0];
        let other = &c[0];
        assert!(
            client.group.contains_device(&ids[2]),
            "alternative signer is actually admitted"
        );
        provider.watch_registry_head(4);
        client.promote_member_peer_bound(provider.local_peer(), ids[0], true);
        let clock = ManualClock::new(1000);
        client.clock = Arc::new(clock.clone());
        let peer = provider.local_peer();
        let (result, ()) = tokio::join!(client.request_registry_head(peer, 4), async {
            provider.run_once().await.unwrap();
            let item = provider.receipt_heads.pending.pop_front().unwrap();
            let document = registry_document(&provider.group.group_id(), 4).unwrap();
            let answer = encode_answer(
                &ReceiptHeadAnswer {
                    receipt: None,
                    repair: None,
                    proof: None,
                },
                &document,
            )
            .unwrap();
            let mut query = item.inner.clone();
            if case == 1 {
                query.push(1);
            }
            let key = if case == 2 {
                other.device.public_key_bytes()
            } else {
                item.key.clone()
            };
            let bound_peer = if case == 3 { other.local_peer() } else { peer };
            let signer = if case == 4 {
                &other.device
            } else {
                &provider.device
            };
            let signature = signer
                .sign(&transcript(
                    &provider.group.group_id(),
                    &key,
                    &item.auth,
                    bound_peer,
                    &query,
                    &answer,
                ))
                .unwrap();
            item.responder
                .respond(Bytes::from(encode_signed_commit_resp(
                    &signer.public_key_bytes(),
                    &signature,
                    &answer,
                )));
            if case == 5 {
                clock.advance_ms(REQUEST_MS);
            }
        });
        if case == 0 {
            assert!(result.unwrap().is_some());
        } else {
            assert!(
                matches!(result, Err(SyncError::Unauthorized)),
                "binding/deadline case {case}: {result:?}"
            );
        }
    }
}

#[tokio::test]
async fn receipt_head_queued_requester_removal_rejects_before_source_access() {
    let (_, mut nodes, ids) = build_members(2).await;
    let (a, b) = nodes.split_at_mut(1);
    let owner = &mut a[0];
    let member = &mut b[0];
    let watch = owner.watch_registry_head(4);
    let query = encode_query(
        &registry_document(&owner.group.group_id(), 4).unwrap(),
        [4; 16],
    )
    .unwrap();
    let (bytes, _) = member
        .build_authed_request(KIND_RECEIPT_HEAD, &query)
        .unwrap();
    let (tx, rx) = Responder::channel();
    owner.queue_receipt_head(member.local_peer(), &bytes[1..], tx);
    assert_eq!(owner.receipt_heads.pending.len(), 1);
    owner.commit_remove_now(&ids[1]);
    assert!(!owner.group.contains_device(&ids[1]));
    assert!(matches!(
        owner.serve_receipt_head::<()>(&watch, None, |_, _, _, _| panic!(
            "removed requester never reaches storage"
        )),
        Err(SyncError::Unauthorized)
    ));
    assert!(rx.recv().await.is_none());
}
