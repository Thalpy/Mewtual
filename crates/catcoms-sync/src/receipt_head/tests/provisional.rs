use super::*;
use crate::registry_seed::{
    CompletedProvisionalStudioDiscovery, PendingProvisionalStudioDiscovery, ProvisionalStudioHint,
};
use crate::StudioWatch;
use catcoms_replication::studio::StudioTarget;

fn target(object: u8) -> StudioTarget {
    if object == 0 {
        StudioTarget::Index { channel: [3; 16] }
    } else {
        StudioTarget::Flipnote {
            channel: [3; 16],
            object: [object; 16],
        }
    }
}
fn watch(node: &mut Node, target: StudioTarget) -> StudioWatch {
    let logical = target.document(&node.group.group_id()).unwrap();
    node.watch_studio(
        target,
        catcoms_replication::epoch_zero_id(logical.doc_type, &logical.logical_key),
    )
    .unwrap()
}
fn candidate(provider: &Node, target: StudioTarget) -> Receipt {
    Receipt::sign(
        target.document(&provider.group.group_id()).unwrap(),
        0,
        [7; 32],
        [8; 32],
        0,
        InheritedCheckpoint::EpochZero,
        &provider.device,
    )
    .unwrap()
}
async fn pair() -> (Node, Node, ManualClock) {
    let (_, mut nodes, ids) = build_members(2).await;
    let mut client = nodes.pop().unwrap();
    let mut owner = nodes.pop().unwrap();
    let clock = ManualClock::new(1000);
    owner.clock = Arc::new(clock.clone());
    client.clock = Arc::new(clock.clone());
    client.promote_member_peer_bound(owner.local_peer(), ids[0], true);
    owner.promote_member_peer_bound(client.local_peer(), ids[1], true);
    (owner, client, clock)
}
async fn reply(
    provider: &mut Node,
    pending: PendingProvisionalStudioDiscovery<MemNetwork>,
    target: StudioTarget,
    receipt: Option<&Receipt>,
    prove: bool,
) -> CompletedProvisionalStudioDiscovery {
    let service = provider
        .watch_checkpoint_head(CheckpointTarget::Studio(target))
        .unwrap();
    let snapshot = prove.then(|| {
        provider
            .prepare_receipt_head_snapshot(|_, _| Ok::<_, ()>(()))
            .unwrap()
            .unwrap()
    });
    let (completed, ()) = tokio::join!(pending.fetch(), async {
        provider.run_once().await.unwrap();
        provider
            .serve_receipt_head(&service, snapshot.as_ref(), |_, _, _, _| {
                Ok::<_, ()>(ReceiptHeadSelection {
                    receipt: receipt.cloned(),
                    prove,
                })
            })
            .unwrap()
            .unwrap()
            .unwrap();
    });
    completed
}
async fn discover(
    provider: &mut Node,
    client: &mut Node,
    watch: &StudioWatch,
    receipt: &Receipt,
) -> ProvisionalStudioHint {
    let pending = client
        .prepare_provisional_studio_discovery(provider.local_peer(), watch)
        .unwrap();
    let completed = reply(provider, pending, watch.target, Some(receipt), false).await;
    client
        .complete_provisional_studio_discovery(completed)
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn provisional_head_authenticates_nonowner_delivery_without_confirming_receipt_authority() {
    for target in [target(0), target(7)] {
        let (mut client, mut provider, _) = pair().await;
        assert_ne!(
            provider.group.designated_committer(),
            Some(provider.device.device_id())
        );
        let watch = watch(&mut client, target);
        let receipt = candidate(&provider, target);
        assert!(receipt.verify_current_owner(&client.group, 0).is_err());
        let hint = discover(&mut provider, &mut client, &watch, &receipt).await;
        client
            .with_provisional_studio_hint(&hint, |value| {
                assert_eq!(value.target, target);
                assert_eq!(value.peer, provider.local_peer());
                assert_eq!(value.provider, provider.device.device_id());
                assert_eq!(value.receipt, &receipt);
            })
            .unwrap();
        assert!(client.receipt_heads.selections.is_empty());
        assert!(client.docs.is_empty());
        assert!(!format!("{hint:?}").contains(&hex::encode(receipt.hash())));
    }
}

#[tokio::test]
async fn provisional_head_owner_proof_or_absence_does_not_mint_or_revoke_selection() {
    let (mut provider, mut client, clock) = pair().await;
    let target = target(7);
    let receipt = candidate(&provider, target);
    let watch = watch(&mut client, target);
    // An existing authoritative selection must survive the provisional request/response.
    let service = provider
        .watch_checkpoint_head(CheckpointTarget::Studio(target))
        .unwrap();
    let snapshot = provider
        .prepare_receipt_head_snapshot(|_, _| Ok::<_, ()>(()))
        .unwrap()
        .unwrap();
    let pending = client
        .prepare_checkpoint_discovery(provider.local_peer(), CheckpointTarget::Studio(target))
        .unwrap();
    let (completed, ()) = tokio::join!(pending.fetch(), async {
        provider.run_once().await.unwrap();
        provider
            .serve_receipt_head(&service, Some(&snapshot), |_, _, _, _| {
                Ok::<_, ()>(ReceiptHeadSelection {
                    receipt: Some(receipt.clone()),
                    prove: true,
                })
            })
            .unwrap()
            .unwrap()
            .unwrap();
    });
    let Some(crate::registry_seed::RegistrySeedDiscovery::Selected(selected)) =
        client.complete_checkpoint_discovery(completed).unwrap()
    else {
        panic!("expected actual selection")
    };
    for prove in [true, false] {
        clock.advance_ms(2000);
        let pending = client
            .prepare_provisional_studio_discovery(provider.local_peer(), &watch)
            .unwrap();
        let completed = reply(
            &mut provider,
            pending,
            target,
            prove.then_some(&receipt),
            prove,
        )
        .await;
        assert!(client
            .complete_provisional_studio_discovery(completed)
            .unwrap()
            .is_none());
        assert!(client.registry_seed_fetch_is_current(&selected));
        assert_eq!(client.receipt_heads.selections.len(), 1);
    }
    // Both discarded responses released provisional custody.
    let slots: Vec<_> = (0..2)
        .map(|_| client.reserve_provisional_checkpoint_capacity().unwrap())
        .collect();
    assert!(client.reserve_provisional_checkpoint_capacity().is_err());
    assert!(client
        .prepare_checkpoint_discovery(provider.local_peer(), CheckpointTarget::Registry(4))
        .is_ok());
    drop(slots);
}

#[tokio::test]
async fn provisional_head_capacity_follows_pending_completed_candidate_and_cancelled_custody() {
    for stage in ["pending", "completed", "candidate", "cancelled"] {
        let (mut provider, mut client, clock) = pair().await;
        let watches: Vec<_> = (1..=4).map(|n| watch(&mut client, target(n))).collect();
        let mut held: Vec<Box<dyn std::any::Any>> = Vec::new();
        for watch in watches.iter().take(3) {
            let pending = client
                .prepare_provisional_studio_discovery(provider.local_peer(), watch)
                .unwrap();
            match stage {
                "pending" => held.push(Box::new(pending)),
                "cancelled" => {
                    let mut future = Box::pin(pending.fetch());
                    assert!(futures::poll!(future.as_mut()).is_pending());
                    drop(future);
                }
                _ => {
                    let receipt = candidate(&provider, watch.target);
                    let completed =
                        reply(&mut provider, pending, watch.target, Some(&receipt), false).await;
                    if stage == "completed" {
                        held.push(Box::new(completed));
                    } else {
                        let hint = client
                            .complete_provisional_studio_discovery(completed)
                            .unwrap()
                            .unwrap();
                        assert!(client.provisional_studio_hint_is_current(&hint));
                        held.push(Box::new(hint));
                    }
                    clock.advance_ms(1000);
                }
            }
        }
        assert!(
            client
                .prepare_provisional_studio_discovery(provider.local_peer(), &watches[3])
                .is_err(),
            "{stage}: no fourth provisional request"
        );
        let authoritative = client
            .prepare_checkpoint_discovery(provider.local_peer(), CheckpointTarget::Registry(4))
            .unwrap();
        clock.advance_ms(60_000);
        assert!(
            client.reserve_provisional_checkpoint_capacity().is_err(),
            "{stage}: expiry cannot refund memory"
        );
        assert!(client
            .prepare_checkpoint_discovery(provider.local_peer(), CheckpointTarget::Registry(5))
            .is_err());
        if stage == "cancelled" {
            provider.run_once().await.unwrap();
        } else {
            held.pop();
        }
        let refill = client
            .prepare_provisional_studio_discovery(provider.local_peer(), &watches[3])
            .unwrap();
        assert!(client.reserve_provisional_checkpoint_capacity().is_err());
        drop((refill, authoritative, held));
    }
}

#[tokio::test]
async fn provisional_head_same_key_rewatch_rejects_completion_and_retained_candidate() {
    for completed_first in [false, true] {
        let (mut provider, mut client, _) = pair().await;
        let target = target(7);
        let old_watch = watch(&mut client, target);
        let receipt = candidate(&provider, target);
        let pending = client
            .prepare_provisional_studio_discovery(provider.local_peer(), &old_watch)
            .unwrap();
        let completed = reply(&mut provider, pending, target, Some(&receipt), false).await;
        let (hint, completed) = if completed_first {
            let hint = client
                .complete_provisional_studio_discovery(completed)
                .unwrap()
                .unwrap();
            assert!(client.provisional_studio_hint_is_current(&hint));
            (Some(hint), None)
        } else {
            (None, Some(completed))
        };
        client.unwatch_studio(&old_watch).unwrap();
        let current_watch = watch(&mut client, target);
        assert_eq!(current_watch.doc_id, old_watch.doc_id);
        assert!(client
            .prepare_provisional_studio_discovery(provider.local_peer(), &old_watch)
            .is_err());
        if let Some(completed) = completed {
            assert!(client
                .complete_provisional_studio_discovery(completed)
                .is_err());
        }
        if let Some(hint) = hint {
            assert!(client
                .with_provisional_studio_hint(&hint, |_| panic!("stale watch callback"))
                .is_err());
        }
    }
}

#[tokio::test]
async fn provisional_head_candidate_rechecks_membership_endpoint_attempt_instance_and_expiry() {
    for change in ["membership", "endpoint", "attempt", "instance", "expiry"] {
        let (mut provider, mut client, clock) = pair().await;
        let target = target(7);
        let watch = watch(&mut client, target);
        let receipt = candidate(&provider, target);
        let hint = discover(&mut provider, &mut client, &watch, &receipt).await;
        assert!(client.provisional_studio_hint_is_current(&hint));
        match change {
            "membership" => {
                let joined = MlsDevice::generate().unwrap();
                let invite = provider.mint_invite([55; 16], u64::MAX, vec![]).unwrap();
                let kp = joined
                    .key_package_for_invite(&invite.group_id, invite.invite_nonce)
                    .unwrap();
                provider
                    .admit_now(
                        &invite,
                        &serialize_key_package(&kp).unwrap(),
                        clock.now_ms(),
                    )
                    .unwrap();
                assert!(client.apply_commit_in_order(provider.commit_log.back().unwrap()));
            }
            "endpoint" => {
                client.member_peers.clear();
                client.promote_member_peer_bound(
                    provider.local_peer(),
                    client.device.device_id(),
                    true,
                );
            }
            "attempt" => {
                drop(
                    client
                        .prepare_checkpoint_discovery(
                            provider.local_peer(),
                            CheckpointTarget::Studio(target),
                        )
                        .unwrap(),
                );
                // Pruning a dead replacement generation cannot resurrect the older candidate.
                drop(
                    client
                        .prepare_checkpoint_discovery(
                            provider.local_peer(),
                            CheckpointTarget::Registry(5),
                        )
                        .unwrap(),
                );
            }
            "instance" => {
                client = Node::restore(
                    &client.snapshot().unwrap(),
                    Hub::new().join(PeerId::from_u64(99)),
                    ChaCha20Rng::seed_from_u64(817),
                    Box::new(clock.clone()),
                )
                .unwrap();
            }
            _ => {
                clock.advance_ms(60_000);
            }
        }
        assert!(
            !client.provisional_studio_hint_is_current(&hint),
            "{change}"
        );
        assert!(client
            .with_provisional_studio_hint(&hint, |_| panic!("stale candidate callback"))
            .is_err());
    }
}

#[tokio::test]
async fn provisional_head_rejects_late_head_and_superseded_completion() {
    for superseded in [false, true] {
        let (mut provider, mut client, clock) = pair().await;
        let target = target(7);
        let watch = watch(&mut client, target);
        let receipt = candidate(&provider, target);
        let pending = client
            .prepare_provisional_studio_discovery(provider.local_peer(), &watch)
            .unwrap();
        let completed = reply(&mut provider, pending, target, Some(&receipt), false).await;
        if superseded {
            drop(
                client
                    .prepare_provisional_studio_discovery(provider.local_peer(), &watch)
                    .unwrap(),
            );
        } else {
            clock.advance_ms(10_000);
        }
        assert!(client
            .complete_provisional_studio_discovery(completed)
            .is_err());
    }
}

#[tokio::test]
async fn provisional_head_rejects_signed_wrong_scope_and_response_transcript_tampering() {
    for tamper in [
        "signature",
        "channel",
        "requester",
        "peer",
        "document",
        "oversized",
    ] {
        let (mut provider, mut client, _) = pair().await;
        let target = target(7);
        let watch = watch(&mut client, target);
        let receipt = candidate(&provider, target);
        provider
            .watch_checkpoint_head(CheckpointTarget::Studio(target))
            .unwrap();
        let pending = client
            .prepare_provisional_studio_discovery(provider.local_peer(), &watch)
            .unwrap();
        let (completed, ()) = tokio::join!(pending.fetch(), async {
            provider.run_once().await.unwrap();
            let item = provider.receipt_heads.pending.pop_front().unwrap();
            let receipt = if tamper == "document" {
                candidate(&provider, super::provisional::target(8))
            } else {
                receipt
            };
            let body = encode_answer(
                &ReceiptHeadAnswer {
                    receipt: Some(receipt.clone()),
                    proof: None,
                    repair: None,
                },
                &receipt.document,
            )
            .unwrap();
            let mut query = item.inner.clone();
            if tamper == "channel" {
                query[7] ^= 1;
            }
            let requester = if tamper == "requester" {
                provider.device.public_key_bytes()
            } else {
                item.key.clone()
            };
            let peer = if tamper == "peer" {
                PeerId::from_u64(99)
            } else {
                provider.local_peer()
            };
            let mut signature = provider
                .device
                .sign(&scoped_transcript(
                    item.target.head_domain(),
                    &provider.group.group_id(),
                    &requester,
                    &item.auth,
                    peer,
                    &query,
                    &body,
                ))
                .unwrap();
            if tamper == "signature" {
                signature[0] ^= 1;
            }
            let bytes = if tamper == "oversized" {
                vec![0; 8 * 1024]
            } else {
                encode_signed_commit_resp(&provider.device.public_key_bytes(), &signature, &body)
            };
            item.responder.respond(Bytes::from(bytes));
        });
        assert!(
            client
                .complete_provisional_studio_discovery(completed)
                .is_err(),
            "{tamper}"
        );
        assert!(client.receipt_heads.selections.is_empty());
        assert!(client.docs.is_empty());
    }
}
