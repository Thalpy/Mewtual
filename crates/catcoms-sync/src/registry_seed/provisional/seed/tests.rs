use super::*;
use crate::tests::build_members;
use catcoms_replication::studio::{
    FlipnoteHeader, FlipnoteOp, IndexOp, StudioEpoch, StudioExpiry, StudioKind,
};
use catcoms_replication::{CheckpointSeed, InheritedCheckpoint};
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;
fn target(art: bool) -> StudioTarget {
    if art {
        StudioTarget::Flipnote {
            channel: [3; 16],
            object: [7; 16],
        }
    } else {
        StudioTarget::Index { channel: [3; 16] }
    }
}
async fn pair() -> (Node, Node, ManualClock) {
    let (_, mut nodes, ids) = build_members(2).await;
    let mut provider = nodes.pop().unwrap(); // A current member who is NOT the designated owner.
    let mut client = nodes.pop().unwrap();
    let clock = ManualClock::new(1000);
    client.clock = Arc::new(clock.clone());
    provider.clock = Arc::new(clock.clone());
    client.promote_member_peer_bound(provider.local_peer(), ids[1], true);
    provider.promote_member_peer_bound(client.local_peer(), ids[0], true);
    assert_ne!(
        provider.group.designated_committer(),
        Some(provider.device.device_id())
    );
    (provider, client, clock)
}
fn seed(provider: &Node, target: StudioTarget) -> (Receipt, CheckpointSeed) {
    let mut source =
        StudioEpoch::new(&provider.group, target, provider.device.device_id()).unwrap();
    let body = match target {
        StudioTarget::Flipnote { .. } => {
            FlipnoteOp::SetHeader(FlipnoteHeader::Title("unconfirmed title".into()))
                .encode()
                .unwrap()
        }
        StudioTarget::Index { .. } => IndexOp::PutObject {
            object: [7; 16],
            kind: StudioKind::Flipnote,
            title: "unconfirmed title".into(),
            created_by: provider.device.device_id(),
            ts: 1000,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap(),
    };
    let logical = target.document(&provider.group.group_id()).unwrap();
    source
        .edit_or_reseal(
            &provider.device,
            &provider.group,
            &mut ChaCha20Rng::seed_from_u64(839),
            &catcoms_replication::DomainOp {
                doc_type: logical.doc_type,
                logical_key: logical.logical_key,
                body,
                nonce: [6; 16],
            },
            1000,
        )
        .unwrap();
    let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
    let receipt = signed(provider, target, seed.change_hash());
    (receipt, seed)
}
fn signed(provider: &Node, target: StudioTarget, hash: [u8; 32]) -> Receipt {
    Receipt::sign(
        target.document(&provider.group.group_id()).unwrap(),
        0,
        [7; 32],
        hash,
        0,
        InheritedCheckpoint::EpochZero,
        &provider.device,
    )
    .unwrap()
}
async fn hint(
    provider: &mut Node,
    client: &mut Node,
    target: StudioTarget,
    receipt: &Receipt,
) -> ProvisionalStudioHint {
    let logical = target.document(&client.group.group_id()).unwrap();
    let watch = client
        .watch_studio(
            target,
            catcoms_replication::epoch_zero_id(logical.doc_type, &logical.logical_key),
        )
        .unwrap();
    let service = provider
        .watch_checkpoint_head(CheckpointTarget::Studio(target))
        .unwrap();
    let pending = client
        .prepare_provisional_studio_discovery(provider.local_peer(), &watch)
        .unwrap();
    let (completed, ()) = tokio::join!(pending.fetch(), async {
        provider.run_once().await.unwrap();
        provider
            .serve_receipt_head(&service, None, |_, _, _, _| {
                Ok::<_, ()>(receipt_head::ReceiptHeadSelection {
                    receipt: Some(receipt.clone()),
                    prove: false,
                })
            })
            .unwrap()
            .unwrap()
            .unwrap();
    });
    client
        .complete_provisional_studio_discovery(completed)
        .unwrap()
        .unwrap()
}
async fn response(
    provider: &mut Node,
    pending: PendingProvisionalStudioSeed<MemNetwork>,
    target: StudioTarget,
    raw: Option<Vec<u8>>,
) -> CompletedProvisionalStudioSeed {
    let service = provider
        .watch_checkpoint_seed(CheckpointTarget::Studio(target))
        .unwrap();
    let (completed, ()) = tokio::join!(pending.fetch(), async {
        provider.run_once().await.unwrap();
        provider
            .serve_registry_seed(&service, |_, _, _, _| Ok::<_, ()>(raw))
            .unwrap()
            .unwrap()
            .unwrap();
    });
    completed
}
fn retained(client: &Node) -> usize {
    client
        .registry_seeds
        .retained
        .iter()
        .filter(|s| s.strong_count() > 0)
        .count()
}

#[tokio::test]
async fn provisional_seed_nonowner_index_and_flipnote_are_only_unconfirmed_typed_data() {
    for art in [false, true] {
        let (mut provider, mut client, _) = pair().await;
        let target = target(art);
        let (receipt, seed) = seed(&provider, target);
        assert!(receipt.verify_current_owner(&client.group, 0).is_err());
        let hint = hint(&mut provider, &mut client, target, &receipt).await;
        let pending = client.prepare_provisional_studio_seed(hint).unwrap();
        let completed = response(&mut provider, pending, target, Some(seed.bytes().to_vec())).await;
        let preparation = client
            .complete_provisional_studio_seed(completed)
            .unwrap()
            .unwrap();
        assert_eq!(retained(&client), 1);
        let prepared = preparation.prepare().unwrap();
        client
            .with_provisional_studio_seed(&prepared, |value| {
                assert_eq!(value.candidate.receipt, &receipt);
                assert_eq!(value.candidate.provider, provider.device.device_id());
                assert_eq!(value.candidate.target, target);
                assert_eq!(value.projection.epoch(), 1);
                assert_eq!(value.projection.document(), &receipt.document);
                assert_eq!(value.projection.channel(), target.channel());
                match value.projection {
                    StudioProjection::Flipnote(p) => assert_eq!(
                        p.title.as_ref().unwrap().selected.value,
                        "unconfirmed title"
                    ),
                    StudioProjection::Index(p) => assert!(p.objects.contains_key(&[7; 16])),
                }
            })
            .unwrap();
        assert_eq!(retained(&client), 1);
        drop(prepared);
        assert_eq!(retained(&client), 0);
    }
}

#[tokio::test]
async fn provisional_seed_rejects_authenticated_bad_receipt_hash_actor_root_channel_and_encoding() {
    use automerge::transaction::Transactable;
    for bad in [
        "signature",
        "hash",
        "actor",
        "root",
        "channel",
        "compressed",
        "trailing",
    ] {
        let (mut provider, mut client, _) = pair().await;
        let target = target(true);
        let (mut receipt, seed) = seed(&provider, target);
        let mut raw = seed.bytes().to_vec();
        let mut requested = target;
        match bad {
            "signature" => receipt.signature[0] ^= 1,
            "hash" => *raw.last_mut().unwrap() ^= 1,
            "compressed" => raw[8] = 2,
            "trailing" => raw.push(0),
            "channel" => {
                requested = StudioTarget::Flipnote {
                    channel: [4; 16],
                    object: [7; 16],
                }
            }
            "actor" => {
                // A valid typed seed under another close, with the hash advertised honestly.
                // Its actor/document binding must fail even though its root schema is valid.
                let invalid =
                    StudioEpoch::new(&provider.group, target, provider.device.device_id())
                        .unwrap()
                        .projection()
                        .unwrap()
                        .checkpoint([9; 32])
                        .unwrap();
                receipt = signed(&provider, target, invalid.change_hash());
                raw = invalid.bytes().to_vec();
            }
            "root" => {
                let invalid = CheckpointSeed::build(&receipt.document, 1, [7; 32], |doc| {
                    doc.put(automerge::ROOT, "unexpected", "seed").unwrap();
                    Ok(())
                })
                .unwrap();
                receipt = signed(&provider, target, invalid.change_hash());
                raw = invalid.bytes().to_vec();
            }
            _ => unreachable!(),
        }
        let hint = hint(&mut provider, &mut client, requested, &receipt).await;
        let pending = client.prepare_provisional_studio_seed(hint).unwrap();
        let completed = response(&mut provider, pending, requested, Some(raw)).await;
        let preparation = client
            .complete_provisional_studio_seed(completed)
            .unwrap()
            .unwrap();
        assert!(preparation.prepare().is_err(), "{bad}");
        assert_eq!(retained(&client), 0, "{bad}");
    }
}

#[tokio::test]
async fn provisional_seed_transport_rejects_tampering_absence_and_late_response() {
    for bad in ["signature", "absent", "late"] {
        let (mut provider, mut client, clock) = pair().await;
        let target = target(true);
        let (receipt, seed) = seed(&provider, target);
        let hint = hint(&mut provider, &mut client, target, &receipt).await;
        let pending = client.prepare_provisional_studio_seed(hint).unwrap();
        let raw = (bad != "absent").then(|| seed.bytes().to_vec());
        let mut completed = response(&mut provider, pending, target, raw).await;
        if bad == "signature" {
            let bytes = completed.transfer.response.as_ref().unwrap();
            let (key, mut signature, body) = decode_response(bytes).unwrap();
            signature[0] ^= 1;
            completed.transfer.response = Ok(Bytes::from(encode_signed_commit_resp(
                key, &signature, body,
            )));
        }
        if bad == "late" {
            clock.advance_ms(REQUEST_MS);
        }
        let result = client.complete_provisional_studio_seed(completed);
        if bad == "absent" {
            assert!(result.unwrap().is_none());
        } else {
            assert!(result.is_err(), "{bad}");
        }
        assert_eq!(retained(&client), 0);
    }
}

#[tokio::test]
async fn provisional_seed_completion_and_prepared_use_recheck_lifecycle() {
    for after_parse in [false, true] {
        for change in [
            "watch",
            "attempt",
            "endpoint",
            "membership",
            "instance",
            "expiry",
        ] {
            let (mut provider, mut client, clock) = pair().await;
            let target = target(true);
            let (receipt, seed) = seed(&provider, target);
            let hint = hint(&mut provider, &mut client, target, &receipt).await;
            let pending = client.prepare_provisional_studio_seed(hint).unwrap();
            let completed =
                response(&mut provider, pending, target, Some(seed.bytes().to_vec())).await;
            let (completed, prepared) = if after_parse {
                let prepared = client
                    .complete_provisional_studio_seed(completed)
                    .unwrap()
                    .unwrap()
                    .prepare()
                    .unwrap();
                (None, Some(prepared))
            } else {
                (Some(completed), None)
            };
            match change {
                "watch" => {
                    client.watch_studio(target, 99).unwrap();
                }
                "attempt" => {
                    drop(
                        client
                            .prepare_checkpoint_head(
                                provider.local_peer(),
                                CheckpointTarget::Studio(target),
                            )
                            .unwrap(),
                    );
                }
                "endpoint" => {
                    client.member_peers.clear();
                    client.promote_member_peer_bound(
                        provider.local_peer(),
                        client.device.device_id(),
                        true,
                    );
                }
                "membership" => {
                    let joined = MlsDevice::generate().unwrap();
                    let invite = client.mint_invite([55; 16], u64::MAX, vec![]).unwrap();
                    let kp = joined
                        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
                        .unwrap();
                    client
                        .admit_now(
                            &invite,
                            &catcoms_mls::serialize_key_package(&kp).unwrap(),
                            clock.now_ms(),
                        )
                        .unwrap();
                }
                "instance" => {
                    client = Node::restore(
                        &client.snapshot().unwrap(),
                        Hub::new().join(PeerId::from_u64(99)),
                        ChaCha20Rng::seed_from_u64(834),
                        Box::new(clock.clone()),
                    )
                    .unwrap();
                }
                _ => {
                    clock.advance_ms(FETCH_MS);
                }
            }
            if let Some(completed) = completed {
                assert!(
                    client.complete_provisional_studio_seed(completed).is_err(),
                    "{change}"
                );
            }
            if let Some(prepared) = prepared {
                assert!(client
                    .with_provisional_studio_seed(&prepared, |_| panic!("revoked {change}"))
                    .is_err());
            }
        }
    }
}

#[derive(Debug)]
struct ParseDeadlineClock {
    base: ManualClock,
    expiry: u64,
    reads: std::sync::atomic::AtomicUsize,
}
impl Clock for ParseDeadlineClock {
    fn now_ms(&self) -> u64 {
        self.base.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        if self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            self.expiry - 1
        } else {
            self.expiry
        }
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.base.sleep(duration)
    }
}
#[tokio::test]
async fn provisional_seed_parser_rechecks_original_deadline_after_valid_parse() {
    let (mut provider, mut client, clock) = pair().await;
    let target = target(true);
    let (receipt, seed) = seed(&provider, target);
    let hint = hint(&mut provider, &mut client, target, &receipt).await;
    let pending = client.prepare_provisional_studio_seed(hint).unwrap();
    let completed = response(&mut provider, pending, target, Some(seed.bytes().to_vec())).await;
    let mut preparation = client
        .complete_provisional_studio_seed(completed)
        .unwrap()
        .unwrap();
    preparation.clock = Arc::new(ParseDeadlineClock {
        base: clock,
        expiry: preparation.hint.expires,
        reads: std::sync::atomic::AtomicUsize::new(0),
    });
    assert!(preparation.prepare().is_err());
    assert_eq!(retained(&client), 0);
}

#[tokio::test]
async fn provisional_seed_capacity_follows_cancelled_lower_transport_parse_and_ready_result() {
    for stage in ["cancelled", "preparation", "ready"] {
        let (mut provider, mut client, clock) = pair().await;
        let target = target(true);
        let (receipt, seed) = seed(&provider, target);
        let hint = hint(&mut provider, &mut client, target, &receipt).await;
        let pending = client.prepare_provisional_studio_seed(hint).unwrap();
        let (preparation, ready) = if stage == "cancelled" {
            let mut future = Box::pin(pending.fetch());
            assert!(futures::poll!(future.as_mut()).is_pending());
            drop(future);
            (None, None)
        } else {
            let completed =
                response(&mut provider, pending, target, Some(seed.bytes().to_vec())).await;
            let preparation = client
                .complete_provisional_studio_seed(completed)
                .unwrap()
                .unwrap();
            if stage == "ready" {
                (None, Some(preparation.prepare().unwrap()))
            } else {
                (Some(preparation), None)
            }
        };
        let second = client.reserve_provisional_checkpoint_capacity().unwrap();
        let third = client.reserve_provisional_checkpoint_capacity().unwrap();
        assert!(client.reserve_provisional_checkpoint_capacity().is_err());
        let authoritative = client
            .prepare_checkpoint_discovery(provider.local_peer(), CheckpointTarget::Registry(4))
            .unwrap();
        assert_eq!(retained(&client), 4);
        clock.advance_ms(FETCH_MS);
        assert!(
            client.reserve_provisional_checkpoint_capacity().is_err(),
            "expired {stage} still owns memory"
        );
        if stage == "cancelled" {
            provider.run_once().await.unwrap();
        }
        if let Some(preparation) = preparation {
            assert!(preparation.prepare().is_err());
        }
        drop(ready);
        assert!(client.reserve_provisional_checkpoint_capacity().is_ok());
        drop((second, third, authoritative));
    }
}
