use super::*;
use crate::tests::build_members;
use catcoms_replication::{registry_epoch::RegistryEpoch, CheckpointSeed, InheritedCheckpoint};
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(819)
}
async fn pair() -> (Node, Node, ManualClock) {
    let (_, mut nodes, ids) = build_members(2).await;
    let clock = ManualClock::new(1000);
    let mut client = nodes.pop().unwrap();
    let mut owner = nodes.pop().unwrap();
    client.clock = Arc::new(clock.clone());
    owner.clock = Arc::new(clock.clone());
    client.promote_member_peer_bound(owner.local_peer(), ids[0], true);
    (owner, client, clock)
}
fn seed(owner: &Node, bucket: u8) -> (Receipt, CheckpointSeed) {
    let source = RegistryEpoch::new(&owner.group, bucket, owner.device.device_id()).unwrap();
    let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
    let receipt = Receipt::sign(
        registry_document(&owner.group.group_id(), bucket).unwrap(),
        0,
        [7; 32],
        seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        &owner.device,
    )
    .unwrap();
    (receipt, seed)
}
async fn discover(
    owner: &mut Node,
    client: &mut Node,
    receipt: &Receipt,
    bucket: u8,
) -> RegistrySeedFetch {
    let watch = owner.watch_registry_head(bucket);
    let permit = owner
        .prepare_receipt_head_snapshot(|_, _| Ok::<_, ()>(()))
        .unwrap()
        .unwrap();
    let (answer, ()) = tokio::join!(
        client.discover_registry_seed(owner.local_peer(), bucket),
        async {
            owner.run_once().await.unwrap();
            owner
                .serve_receipt_head(&watch, Some(&permit), |_, _, _, _| {
                    Ok::<_, ()>(receipt_head::ReceiptHeadSelection {
                        receipt: Some(receipt.clone()),
                        prove: true,
                    })
                })
                .unwrap()
                .unwrap()
                .unwrap();
        }
    );
    match answer.unwrap().unwrap() {
        RegistrySeedDiscovery::Selected(pass) => pass,
        _ => panic!("fresh owner selection"),
    }
}
async fn fetch(
    owner: &mut Node,
    client: &mut Node,
    pass: &mut RegistrySeedFetch,
    raw: Option<Vec<u8>>,
) -> Result<bool, SyncError> {
    let watch = owner.watch_registry_seed(pass.selection.bucket);
    let (answer, ()) = tokio::join!(
        client.fetch_registry_seed(pass, owner.local_peer()),
        async {
            owner.run_once().await.unwrap();
            owner
                .serve_registry_seed(&watch, |_, _, _, _| Ok::<_, ()>(raw))
                .unwrap()
                .unwrap()
                .unwrap();
        }
    );
    answer
}

#[test]
fn registry_seed_wire_exact_query_caps_padding_and_authenticated_bad_frames() {
    let group = [3; 16];
    let query = Query {
        bucket: 4,
        doc_id: 27,
        hash: [9; 32],
    };
    let bytes = encode_query(&query, &group).unwrap();
    let mut expected = vec![1, 0, 18, 0, 0, 0, 32];
    expected.extend(registry_document(&group, 4).unwrap().logical_key);
    expected.extend(27u128.to_be_bytes());
    expected.extend([0, 0, 0, 32]);
    expected.extend([9; 32]);
    assert_eq!(bytes, expected);
    assert_eq!(bytes.len(), 91);
    assert_eq!(decode_query(&bytes, &group).unwrap(), query);
    for end in 0..bytes.len() {
        assert!(decode_query(&bytes[..end], &group).is_err());
    }
    assert!(decode_query(&bytes, &[4; 16]).is_err());
    for offset in [0, 2, 6, 8, 58] {
        let mut bad = bytes.clone();
        bad[offset] ^= 1;
        assert!(decode_query(&bad, &group).is_err());
    }
    let mut trailing = bytes;
    trailing.push(0);
    assert!(decode_query(&trailing, &group).is_err());
    assert!(decode_query(&vec![0; MAX_QUERY + 1], &group).is_err());
    let key = [8; 32];
    for n in [
        1,
        512,
        513,
        OP_PAD_CEILING,
        OP_PAD_CEILING + 1,
        MAX_CHECKPOINT_BYTES,
    ] {
        let raw = vec![6; n];
        let sealed = seal_seed(&raw, &key, &mut rng()).unwrap();
        assert_eq!(
            sealed.len(),
            pad::padded_len(n, OP_PAD_FLOOR, OP_PAD_CEILING) + 52
        );
        assert_eq!(*open_seed(&sealed, &key).unwrap(), raw);
        assert!(open_seed(&sealed, &[7; 32]).is_err());
    }
    assert!(seal_seed(&[], &key, &mut rng()).is_err());
    assert!(seal_seed(&vec![0; MAX_CHECKPOINT_BYTES + 1], &key, &mut rng()).is_err());
    assert!(open_seed(&vec![0; MAX_SEALED + 1], &key).is_err());
    assert!(decode_response(&vec![0; MAX_RESPONSE + 1]).is_err());
    let mut noncanonical = pad::pad(&[6], OP_PAD_FLOOR, OP_PAD_CEILING).unwrap();
    noncanonical[1] = 1;
    for malformed in [noncanonical, vec![6; 100]] {
        let sealed = encode_sealed(&seal(&key, &malformed, &mut rng()).unwrap());
        assert!(
            open_seed(&sealed, &key).is_err(),
            "valid AEAD cannot bypass padding"
        );
    }
}

#[tokio::test]
async fn registry_seed_selection_without_bytes_is_scoped_superseded_and_expiring() {
    let (mut owner, mut client, clock) = pair().await;
    let (receipt, _) = seed(&owner, 4);
    let pass = discover(&mut owner, &mut client, &receipt, 4).await;
    client
        .with_registry_seed_selection(&pass, |group, device, _, selected| {
            assert_eq!(selected.receipt, &receipt);
            assert!(selected.checkpoint.is_none());
            assert_eq!(selected.bucket, 4);
            selected
                .receipt
                .verify_current_owner(group, selected.tenure)
                .unwrap();
            assert!(group.member_signature_key(&device.device_id()).is_some());
        })
        .unwrap();
    assert!(client
        .with_registry_seed(&pass, |_, _, _, _| panic!("not fetched"))
        .is_err());
    clock.advance_ms(1000);
    let current = discover(&mut owner, &mut client, &receipt, 4).await;
    assert!(client
        .with_registry_seed_selection(&pass, |_, _, _, _| panic!("superseded"))
        .is_err());
    clock.advance_ms(60_001);
    assert!(client
        .with_registry_seed_selection(&current, |_, _, _, _| panic!("expired"))
        .is_err());
    assert!(client.docs.is_empty());
}

#[tokio::test]
async fn registry_seed_joined_member_fetches_only_exact_typed_seed_without_installing() {
    let (mut owner, mut client, _) = pair().await;
    let (receipt, seed) = seed(&owner, 4);
    let mut pass = discover(&mut owner, &mut client, &receipt, 4).await;
    assert!(!pass.is_fetched());
    assert!(client.with_registry_seed(&pass, |_, _, _, _| ()).is_err());
    assert!(fetch(
        &mut owner,
        &mut client,
        &mut pass,
        Some(seed.bytes().to_vec())
    )
    .await
    .unwrap());
    assert!(pass.is_fetched());
    client
        .with_registry_seed(&pass, |_, _, _, selected| {
            assert_eq!(selected.receipt, &receipt);
            assert_eq!(selected.checkpoint.bytes(), seed.bytes());
            assert_eq!(
                selected.checkpoint.origin().doc_id(),
                seed.origin().doc_id()
            );
            assert_eq!(selected.tenure, 0);
        })
        .unwrap();
    assert!(
        client.docs.is_empty(),
        "fetch never enters legacy documents or persists state"
    );
    assert!(!format!("{pass:?}").contains("receipt"));
}

#[tokio::test]
async fn registry_seed_missing_wrong_hash_and_compressed_chunks_spend_bounded_attempts() {
    let (mut owner, mut client, clock) = pair().await;
    let (receipt, seed) = seed(&owner, 4);
    let mut pass = discover(&mut owner, &mut client, &receipt, 4).await;
    assert!(!fetch(&mut owner, &mut client, &mut pass, None)
        .await
        .unwrap());
    assert!(
        client
            .fetch_registry_seed(&mut pass, owner.local_peer())
            .await
            .is_err(),
        "paced"
    );
    clock.advance_ms(1000);
    let mut wrong = seed.bytes().to_vec();
    *wrong.last_mut().unwrap() ^= 1;
    assert!(fetch(&mut owner, &mut client, &mut pass, Some(wrong))
        .await
        .is_err());
    clock.advance_ms(1000);
    let mut wrong = seed.bytes().to_vec();
    wrong[8] = 2;
    assert!(fetch(&mut owner, &mut client, &mut pass, Some(wrong))
        .await
        .is_err());
    clock.advance_ms(1000);
    assert!(
        client
            .fetch_registry_seed(&mut pass, owner.local_peer())
            .await
            .is_err(),
        "three attempts only"
    );
    assert!(!pass.is_fetched());
    assert_eq!(pass.attempts, 3);
}

#[tokio::test]
async fn registry_seed_matching_receipted_hash_still_requires_the_typed_registry_schema() {
    use automerge::{transaction::Transactable, ROOT};
    let (mut owner, mut client, _) = pair().await;
    let document = registry_document(&owner.group.group_id(), 4).unwrap();
    let malformed = CheckpointSeed::build(&document, 1, [7; 32], |doc| {
        // Raw canonical change metadata is correct, but this is the wrong registry bucket.
        doc.put(ROOT, "bucket", 5u64).unwrap();
        doc.put(ROOT, "epoch", 1u64).unwrap();
        let key: String = document
            .logical_key
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        doc.put(ROOT, "key", key).unwrap();
        doc.put(ROOT, "kind", "registry").unwrap();
        doc.put(ROOT, "v", 1u64).unwrap();
        Ok(())
    })
    .unwrap();
    let receipt = Receipt::sign(
        document,
        0,
        [7; 32],
        malformed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        &owner.device,
    )
    .unwrap();
    let verified = receipt.verify_current_owner(&owner.group, 0).unwrap();
    assert!(
        CheckpointSeed::verify(&verified, malformed.bytes(), |_, _, _| Ok(())).is_ok(),
        "generic seed/hash checks deliberately pass; typed validation must reject"
    );
    let mut pass = discover(&mut owner, &mut client, &receipt, 4).await;
    assert!(fetch(
        &mut owner,
        &mut client,
        &mut pass,
        Some(malformed.bytes().to_vec())
    )
    .await
    .is_err());
    assert!(!pass.is_fetched());
    assert!(client.docs.is_empty());
}

#[tokio::test]
async fn registry_seed_selection_revokes_on_rediscovery_restart_membership_and_expiry() {
    let (mut owner, mut client, clock) = pair().await;
    let (receipt, seed) = seed(&owner, 4);
    let mut first = discover(&mut owner, &mut client, &receipt, 4).await;
    assert!(fetch(
        &mut owner,
        &mut client,
        &mut first,
        Some(seed.bytes().to_vec())
    )
    .await
    .unwrap());
    let second = discover(&mut owner, &mut client, &receipt, 4).await;
    assert!(!client.registry_seed_fetch_is_current(&first));
    assert!(client
        .with_registry_seed(&first, |_, _, _, _| panic!("revoked"))
        .is_err());
    assert!(client.registry_seed_fetch_is_current(&second));
    let restored = Node::restore(
        &client.snapshot().unwrap(),
        Hub::new().join(PeerId::from_u64(99)),
        rng(),
        Box::new(clock.clone()),
    )
    .unwrap();
    assert!(!restored.registry_seed_fetch_is_current(&second));
    clock.advance_ms(FETCH_MS);
    assert!(!client.registry_seed_fetch_is_current(&second));
    let third = discover(&mut owner, &mut client, &receipt, 4).await;
    let joined = MlsDevice::generate().unwrap();
    let invite = owner.mint_invite([55; 16], u64::MAX, vec![]).unwrap();
    let kp = joined
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    owner
        .admit_now(
            &invite,
            &serialize_key_package(&kp).unwrap(),
            clock.now_ms(),
        )
        .unwrap();
    assert!(client.apply_commit_in_order(owner.commit_log.back().unwrap()));
    assert!(
        !client.registry_seed_fetch_is_current(&third),
        "same-owner MLS advance revokes"
    );
}

#[tokio::test]
async fn registry_seed_retained_capacity_is_not_refunded_by_revocation_or_timeout() {
    let (mut owner, mut client, clock) = pair().await;
    let mut held = Vec::new();
    for bucket in 0..4 {
        let (receipt, _) = seed(&owner, bucket);
        held.push(discover(&mut owner, &mut client, &receipt, bucket).await);
        clock.advance_ms(1000);
    }
    assert!(client
        .discover_registry_seed(owner.local_peer(), 5)
        .await
        .is_err());
    clock.advance_ms(FETCH_MS);
    assert!(client
        .discover_registry_seed(owner.local_peer(), 5)
        .await
        .is_err());
    held.pop();
    let (receipt, _) = seed(&owner, 5);
    let fifth = discover(&mut owner, &mut client, &receipt, 5).await;
    assert!(client.registry_seed_fetch_is_current(&fifth));
}

#[tokio::test]
async fn registry_seed_cancelled_fetch_keeps_transport_capacity_and_spends_attempt() {
    let (mut owner, mut client, clock) = pair().await;
    let (receipt, _) = seed(&owner, 4);
    let mut pass = discover(&mut owner, &mut client, &receipt, 4).await;
    for _ in 0..3 {
        let mut future = Box::pin(client.fetch_registry_seed(&mut pass, owner.local_peer()));
        assert!(futures::poll!(future.as_mut()).is_pending());
        drop(future);
        clock.advance_ms(1000);
    }
    assert_eq!(pass.attempts, 3);
    assert_eq!(
        client
            .registry_seeds
            .outbound
            .iter()
            .filter(|p| p.strong_count() > 0)
            .count(),
        3
    );
    assert!(client
        .fetch_registry_seed(&mut pass, owner.local_peer())
        .await
        .is_err());
    for _ in 0..3 {
        owner.run_once().await.unwrap();
    }
    assert!(client
        .registry_seeds
        .outbound
        .iter()
        .all(|p| p.strong_count() == 0));
}

fn enqueue(node: &mut Node, query: &Query) -> catcoms_rt::ResponderRx {
    let inner = encode_query(query, &node.group.group_id()).unwrap();
    let (request, _) = node
        .build_authed_request(KIND_REGISTRY_SEED, &inner)
        .unwrap();
    let (tx, rx) = Responder::channel();
    node.queue_registry_seed(node.local_peer(), &request[1..], tx);
    rx
}

#[tokio::test]
async fn registry_seed_can_fetch_from_an_independently_proven_non_owner_member() {
    let (_, mut nodes, ids) = build_members(3).await;
    let commits: Vec<_> = nodes[0].commit_log.iter().cloned().collect();
    for node in nodes.iter_mut().skip(1) {
        for commit in &commits {
            if commit.commit_epoch == node.epoch() {
                assert!(node.apply_commit_in_order(commit));
            }
        }
    }
    let mut client = nodes.pop().unwrap();
    let mut provider = nodes.pop().unwrap();
    let mut owner = nodes.pop().unwrap();
    client.promote_member_peer_bound(owner.local_peer(), ids[0], true);
    let (receipt, seed) = seed(&owner, 4);
    let mut pass = discover(&mut owner, &mut client, &receipt, 4).await;
    assert!(
        client
            .fetch_registry_seed(&mut pass, provider.local_peer())
            .await
            .is_err(),
        "no disclosure to unproven endpoint"
    );
    assert_eq!(pass.attempts, 0);
    client.promote_member_peer_bound(provider.local_peer(), ids[1], true);
    assert!(fetch(
        &mut provider,
        &mut client,
        &mut pass,
        Some(seed.bytes().to_vec())
    )
    .await
    .unwrap());
    client
        .with_registry_seed(&pass, |_, _, _, selected| {
            assert_eq!(selected.receipt, &receipt)
        })
        .unwrap();
}

#[tokio::test]
async fn registry_seed_client_rejects_authenticated_wrong_bindings_and_late_ready_response() {
    let (mut owner, mut client, clock) = pair().await;
    let (receipt, seed) = seed(&owner, 4);
    let alternate = client.device.duplicate().unwrap();
    for case in 0..7 {
        clock.advance_ms(1000);
        let mut pass = discover(&mut owner, &mut client, &receipt, 4).await;
        let _watch = owner.watch_registry_seed(4);
        let peer = owner.local_peer();
        let (result, ()) = tokio::join!(client.fetch_registry_seed(&mut pass, peer), async {
            owner.run_once().await.unwrap();
            let item = owner.registry_seeds.pending.pop_front().unwrap();
            let mut query = item.inner.clone();
            let mut requester = item.key.clone();
            let mut auth = item.auth;
            let mut response_peer = peer;
            let signer = if case == 3 { &alternate } else { &owner.device };
            match case {
                0 => query[10] ^= 1,
                1 => requester[0] ^= 1,
                2 => response_peer = PeerId::from_u64(882),
                4 => auth.nonce[0] ^= 1,
                _ => (),
            }
            let id = if case == 5 {
                item.query.doc_id ^ 1
            } else {
                item.query.doc_id
            };
            let key = owner
                .group
                .channel_secret(&owner.device, DocType::DocRegistry, id)
                .unwrap();
            let body = seal_seed(seed.bytes(), &key, &mut owner.rng).unwrap();
            let signature = signer
                .sign(&transcript(
                    &owner.group.group_id(),
                    &requester,
                    &auth,
                    response_peer,
                    &query,
                    &body,
                ))
                .unwrap();
            if case == 6 {
                clock.advance_ms(REQUEST_MS);
            }
            item.responder
                .respond(Bytes::from(encode_signed_commit_resp(
                    &signer.public_key_bytes(),
                    &signature,
                    &body,
                )));
        });
        assert!(result.is_err(), "binding case {case}");
        assert!(!pass.is_fetched());
    }
}

#[tokio::test]
async fn registry_seed_provider_queue_and_source_rates_stay_bounded_after_rewatch() {
    let (mut owner, _, clock) = pair().await;
    let watch = owner.watch_registry_seed(4);
    let members: Vec<_> = (0..9).map(|_| MlsDevice::generate().unwrap()).collect();
    for member in &members {
        owner
            .with_observed_mls_transition(|n| {
                n.group.add_member(&n.device, member.key_package().unwrap())
            })
            .unwrap();
    }
    let inner = encode_query(
        &Query {
            bucket: 4,
            doc_id: 27,
            hash: [9; 32],
        },
        &owner.group.group_id(),
    )
    .unwrap();
    let mut replies = Vec::new();
    for (n, member) in members.iter().enumerate() {
        let peer = PeerId::from_u64(100 + n as u64);
        let key = member.public_key_bytes();
        let auth = RequestAuth {
            ts: clock.now_ms(),
            nonce: [n as u8; 16],
            epoch: owner.epoch(),
        };
        let signature = member
            .sign(&catchup_auth_transcript(
                &owner.group.group_id(),
                KIND_REGISTRY_SEED,
                &inner,
                &key,
                auth.ts,
                &auth.nonce,
                auth.epoch,
                Some(&peer),
            ))
            .unwrap();
        let bytes =
            encode_authed_request(&inner, &key, auth.ts, &auth.nonce, auth.epoch, &signature);
        let (tx, rx) = Responder::channel();
        owner.queue_registry_seed(peer, &bytes, tx);
        replies.push(rx);
    }
    assert_eq!(owner.registry_seeds.pending.len(), 8);
    assert!(replies.pop().unwrap().recv().await.is_none());
    for _ in 0..2 {
        owner
            .serve_registry_seed(&watch, |_, _, _, _| Ok::<_, ()>(None))
            .unwrap()
            .unwrap()
            .unwrap();
    }
    assert!(owner
        .serve_registry_seed::<()>(&watch, |_, _, _, _| panic!("service rail before I/O"))
        .unwrap()
        .is_none());
    clock.advance_ms(999);
    assert!(owner
        .serve_registry_seed::<()>(&watch, |_, _, _, _| panic!("no early refill"))
        .unwrap()
        .is_none());
    clock.advance_ms(1);
    assert_eq!(
        owner
            .serve_registry_seed(&watch, |_, _, _, _| Err::<Option<Vec<u8>>, _>(
                "read failed"
            ))
            .unwrap()
            .unwrap(),
        Err("read failed")
    );
    owner.watch_registry_seed(4);
    assert!(owner.registry_seeds.pending.is_empty());
    assert_eq!(
        owner.registry_seeds.requesters.len(),
        8,
        "registration does not reset debt"
    );
}
#[tokio::test]
async fn registry_seed_provider_expiry_revocation_and_post_io_deadline_prevent_reply() {
    let (mut owner, _, clock) = pair().await;
    let watch = owner.watch_registry_seed(4);
    let query = Query {
        bucket: 4,
        doc_id: 27,
        hash: [9; 32],
    };
    let rx = enqueue(&mut owner, &query);
    clock.advance_ms(QUEUE_MS);
    assert!(owner
        .serve_registry_seed::<()>(&watch, |_, _, _, _| panic!("expired before I/O"))
        .unwrap()
        .is_none());
    assert!(rx.recv().await.is_none());
    let rx = enqueue(&mut owner, &query);
    assert!(owner
        .serve_registry_seed(&watch, |_, _, _, _| {
            clock.advance_ms(QUEUE_MS);
            Ok::<_, ()>(Some(vec![6]))
        })
        .is_err());
    assert!(rx.recv().await.is_none());
    let rx = enqueue(&mut owner, &query);
    let next = owner.watch_registry_seed(4);
    assert!(rx.recv().await.is_none());
    assert!(owner
        .serve_registry_seed::<()>(&watch, |_, _, _, _| panic!("revoked"))
        .is_err());
    assert!(owner.registry_seed_watch_is_current(&next));
    assert!(!owner.registry_seeds.requesters.is_empty());
}

#[tokio::test]
async fn registry_seed_request_rejects_wrong_kind_peer_and_queued_removed_member() {
    let (mut owner, mut client, _) = pair().await;
    let watch = owner.watch_registry_seed(4);
    let query = encode_query(
        &Query {
            bucket: 4,
            doc_id: 27,
            hash: [9; 32],
        },
        &owner.group.group_id(),
    )
    .unwrap();
    for kind in [KIND_RECEIPT_HEAD, KIND_REGISTRY_SEED] {
        let (request, _) = client.build_authed_request(kind, &query).unwrap();
        let (tx, rx) = Responder::channel();
        owner.queue_registry_seed(
            if kind == KIND_REGISTRY_SEED {
                PeerId::from_u64(88)
            } else {
                client.local_peer()
            },
            &request[1..],
            tx,
        );
        assert!(rx.recv().await.is_none());
    }
    let (request, _) = client
        .build_authed_request(KIND_REGISTRY_SEED, &query)
        .unwrap();
    let (tx, rx) = Responder::channel();
    owner.queue_registry_seed(client.local_peer(), &request[1..], tx);
    assert_eq!(owner.registry_seeds.pending.len(), 1);
    owner
        .with_observed_mls_transition(|n| {
            n.group.remove_member(&n.device, &client.device.device_id())
        })
        .unwrap();
    assert!(owner
        .serve_registry_seed::<()>(&watch, |_, _, _, _| panic!("removed before I/O"))
        .is_err());
    assert!(rx.recv().await.is_none());
}
