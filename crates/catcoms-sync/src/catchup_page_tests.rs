//! The requester must accept bounded empty scan pages without accepting an endless empty walk.
use super::*;
use automerge::{transaction::Transactable, ReadDoc, ROOT};
use catcoms_rt::{ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;

type Member = ChannelSync<MemNetwork, ChaCha20Rng>;
const CHANNEL: u128 = 913;

/// Exercise real request/response signatures and import without transport scheduling. The
/// rejected inner frame is still inside a valid current member's exact response transcript.
fn signed_answer(
    provider: &Member,
    requester: &mut Member,
    answer: Vec<u8>,
) -> Result<Option<usize>, SyncError> {
    let (_, auth) = requester
        .build_authed_request(
            KIND_CATCHUP_SINCE,
            &encode_catchup_since_req(DocType::Channel, CHANNEL, &[], None),
        )
        .unwrap();
    let response = provider
        .sign_doc_catchup_answer(&requester.device.public_key_bytes(), &auth, answer)
        .unwrap();
    requester.apply_catchup_since_response(
        provider.local_peer(),
        DocType::Channel,
        CHANNEL,
        auth,
        &response,
    )
}

#[tokio::test]
async fn rejected_chat_page_never_advances_past_unprocessed_history_and_retains_verified_prefix_work(
) {
    let (mut provider, mut requester, _) = pair().await;
    let peer = provider.local_peer();
    let mut good = Vec::new();
    for index in 0..3 {
        good.push(
            provider
                .docs
                .get_mut(&(DocType::Channel, CHANNEL))
                .unwrap()
                .edit(
                    &provider.device,
                    &provider.group,
                    &mut provider.rng,
                    |doc| doc.put(ROOT, format!("message-{index}"), index as u64),
                )
                .unwrap(),
        );
    }
    let key = provider
        .group
        .channel_secret(&provider.device, DocType::Channel, CHANNEL)
        .unwrap();
    let mut bad = good[1].open(&key).unwrap();
    bad.signature[0] ^= 1;
    let bad = SealedOp::seal(&bad, &provider.group, &provider.device, &mut provider.rng).unwrap();
    let mut forged = automerge::AutoCommit::new().with_actor(automerge::ActorId::from(
        provider.device.device_id().as_bytes().to_vec(),
    ));
    forged.put(ROOT, "forged-writer", true).unwrap();
    forged.commit();
    let forged = catcoms_replication::SignedOp::sign(
        &requester.device,
        DocType::Channel,
        CHANNEL,
        forged.get_last_local_change().unwrap().raw_bytes().to_vec(),
    )
    .unwrap();
    let forged = SealedOp::seal(
        &forged,
        &provider.group,
        &provider.device,
        &mut provider.rng,
    )
    .unwrap();
    let original = CatchupCursor {
        provider: [91; 16],
        position: 7,
    };
    let offered = CatchupCursor {
        provider: original.provider,
        position: 10,
    };
    for bad in [bad, forged] {
        for marker in [
            Some(CATCHUP_SINCE_PAGE),
            Some(CATCHUP_SINCE_UNDERSTOOD),
            Some(CATCHUP_SINCE_MORE),
            None,
        ] {
            let saved = EncryptedDoc::new(DocType::Channel, CHANNEL, &requester.device.device_id());
            requester.docs.insert((DocType::Channel, CHANNEL), saved);
            requester.catchup_queue.clear();
            requester
                .catchup_cursors
                .insert((DocType::Channel, CHANNEL, peer), original);
            requester.note_source_checked(peer, DocType::Channel, CHANNEL);
            requester
                .catchup_stalls
                .insert((DocType::Channel, CHANNEL, peer), 2);
            let mut answer: Vec<u8> = marker.into_iter().collect();
            if marker == Some(CATCHUP_SINCE_PAGE) {
                answer.extend_from_slice(&offered.encode());
            }
            answer.extend_from_slice(&encode_bundle(&[
                good[0].clone(),
                bad.clone(),
                good[1].clone(),
            ]));
            if marker.is_some() {
                assert!(signed_answer(&provider, &mut requester, answer).is_err());
            } else {
                assert!(requester
                    .apply_full_catchup_response(peer, DocType::Channel, CHANNEL, &answer)
                    .is_err());
            }
            assert_eq!(
                requester.doc(DocType::Channel, CHANNEL).unwrap().op_count(),
                1
            );
            assert_eq!(
                requester
                    .catchup_cursors
                    .get(&(DocType::Channel, CHANNEL, peer))
                    .copied(),
                Some(original),
                "failed page/completion cannot skip its unprocessed tail or erase the prior walk"
            );
            assert!(
                requester.catchup_queue.contains(&CatchupTask::Doc {
                    doc_type: DocType::Channel,
                    doc_id: CHANNEL
                }),
                "an accepted prefix must retain recovery work even when a later frame fails"
            );
            assert!(
                !requester
                    .catchup_sources_checked
                    .contains_key(&(DocType::Channel, CHANNEL)),
                "a verified prefix invalidates older completeness even on later rejection"
            );
            assert_eq!(
                requester
                    .catchup_stalls
                    .get(&(DocType::Channel, CHANNEL, peer)),
                Some(&2),
                "a rejected completion cannot clear the provider's existing stall"
            );
            assert!(requester
                .doc(DocType::Channel, CHANNEL)
                .unwrap()
                .doc()
                .get(ROOT, "message-1")
                .unwrap()
                .is_none());
            // A corrected response from the same original position, then a later page, can finish.
            let mut corrected = vec![CATCHUP_SINCE_PAGE];
            corrected.extend_from_slice(&offered.encode());
            corrected.extend_from_slice(&encode_bundle(&good[..2]));
            assert_eq!(
                signed_answer(&provider, &mut requester, corrected).unwrap(),
                Some(1)
            );
            let mut last = vec![CATCHUP_SINCE_UNDERSTOOD];
            last.extend_from_slice(&encode_bundle(&good[2..]));
            assert_eq!(
                signed_answer(&provider, &mut requester, last).unwrap(),
                Some(1)
            );
            assert_eq!(
                requester
                    .docs
                    .get_mut(&(DocType::Channel, CHANNEL))
                    .unwrap()
                    .heads(),
                provider
                    .docs
                    .get_mut(&(DocType::Channel, CHANNEL))
                    .unwrap()
                    .heads()
            );
            assert_eq!(
                requester.doc(DocType::Channel, CHANNEL).unwrap().op_count(),
                3
            );
        }
    }
}

async fn pair() -> (Member, Member, ManualClock) {
    let (_hub, mut members, ids) = tests::build_members(2).await;
    tests::converge_and_publish_test_routes(&mut members);
    let mut bob = members.pop().unwrap();
    let mut alice = members.pop().unwrap();
    let clock = ManualClock::new(1_000);
    alice.clock = Arc::new(clock.clone());
    bob.clock = Arc::new(clock.clone());
    alice.note_peer_connected(bob.local_peer());
    bob.note_peer_connected(alice.local_peer());
    alice.promote_member_peer_bound(bob.local_peer(), ids[1], true);
    bob.promote_member_peer_bound(alice.local_peer(), ids[0], true);
    alice.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    bob.open_channel(DocType::Channel, CHANNEL).await.unwrap();
    (alice, bob, clock)
}

async fn exchange(provider: &mut Member, requester: &mut Member, answer: Option<Vec<u8>>) -> usize {
    exchange_with_cost(provider, requester, answer).await.0
}

async fn exchange_with_cost(
    provider: &mut Member,
    requester: &mut Member,
    answer: Option<Vec<u8>>,
) -> (usize, usize) {
    let peer = provider.local_peer();
    let (applied, transferred) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(
            requester.request_catchup(peer, DocType::Channel, CHANNEL),
            async {
                for _ in 0..64 {
                    match provider.transport.next_event().await {
                        Some(TransportEvent::Request {
                            from,
                            data,
                            responder,
                            ..
                        }) if data.first() == Some(&KIND_CATCHUP_SINCE) => {
                            let response = match answer {
                                None => provider.handle_request(from, &data),
                                Some(answer) => {
                                    let (_, pubkey, auth) = provider
                                        .authenticate_request(KIND_CATCHUP_SINCE, &data[1..], from)
                                        .expect("the actual requester is a current member");
                                    provider
                                        .sign_doc_catchup_answer(&pubkey, &auth, answer)
                                        .unwrap()
                                }
                            };
                            let (_, _, signed_answer) =
                                decode_signed_commit_resp(&response).unwrap();
                            let bundle = match signed_answer.first() {
                                Some(&CATCHUP_SINCE_PAGE) => {
                                    &signed_answer[1 + CATCHUP_CURSOR_BYTES..]
                                }
                                Some(&CATCHUP_SINCE_UNDERSTOOD) | Some(&CATCHUP_SINCE_MORE) => {
                                    &signed_answer[1..]
                                }
                                _ => panic!("expected a history page"),
                            };
                            let transferred = decode_bundle(bundle).unwrap().len();
                            responder.respond(Bytes::from(response));
                            return transferred;
                        }
                        Some(_) => continue,
                        None => panic!("transport ended before the request"),
                    }
                }
                panic!("request never reached the provider");
            }
        )
    })
    .await
    .expect("one signed page must finish without advancing the injected clock");
    (
        applied.expect("the signed response authenticates against this request and provider"),
        transferred,
    )
}

fn empty_page(provider: [u8; 16], position: u32) -> Vec<u8> {
    let mut answer = vec![CATCHUP_SINCE_PAGE];
    answer.extend_from_slice(&CatchupCursor { provider, position }.encode());
    answer.extend_from_slice(&encode_bundle(&[]));
    answer
}

#[tokio::test]
async fn the_real_requester_recovers_a_large_recent_gap_without_duplicate_operation_payload() {
    let (mut alice, mut bob, _clock) = pair().await;
    // Exceeds both the 2,048-step query budget and the 4,096-entry derived cache.
    // The separate public-API cost regression covers the full 10,000-operation example.
    for count in 0..4_098 {
        let operation = alice
            .docs
            .get_mut(&(DocType::Channel, CHANNEL))
            .unwrap()
            .edit(&alice.device, &alice.group, &mut alice.rng, |doc| {
                doc.put(ROOT, "count", count as u64)
            })
            .unwrap();
        bob.docs
            .get_mut(&(DocType::Channel, CHANNEL))
            .unwrap()
            .ingest(&operation, &bob.group, &bob.device)
            .unwrap();
    }
    alice
        .docs
        .get_mut(&(DocType::Channel, CHANNEL))
        .unwrap()
        .edit(&alice.device, &alice.group, &mut alice.rng, |doc| {
            doc.put(ROOT, "missing", "retained after the duplicate prefix")
        })
        .unwrap();
    let peer = alice.local_peer();
    assert_eq!(exchange_with_cost(&mut alice, &mut bob, None).await, (1, 1));
    assert!(!bob.catchup_peer_is_cooling(peer, DocType::Channel, CHANNEL));
    assert!(bob
        .doc(DocType::Channel, CHANNEL)
        .unwrap()
        .doc()
        .get(ROOT, "missing")
        .unwrap()
        .is_some());
    assert!(!bob
        .catchup_empty_page_grace
        .contains_key(&(DocType::Channel, CHANNEL, peer)));
    assert_eq!(
        alice
            .docs
            .get_mut(&(DocType::Channel, CHANNEL))
            .unwrap()
            .heads(),
        bob.docs
            .get_mut(&(DocType::Channel, CHANNEL))
            .unwrap()
            .heads()
    );

    // A provider restart cannot reuse another runtime's scan position. Its document-local
    // certificates are rebuilt by the ordinary restore before any request is served.
    let bytes = alice
        .docs
        .get_mut(&(DocType::Channel, CHANNEL))
        .unwrap()
        .snapshot()
        .unwrap();
    alice.docs.insert(
        (DocType::Channel, CHANNEL),
        EncryptedDoc::restore_for_actor(&bytes, &alice.device.device_id()).unwrap(),
    );
    let old_provider = alice.catchup_provider;
    alice.catchup_provider = [73; 16];
    bob.catchup_cursors.insert(
        (DocType::Channel, CHANNEL, peer),
        CatchupCursor {
            provider: old_provider,
            position: u32::MAX,
        },
    );
    alice
        .docs
        .get_mut(&(DocType::Channel, CHANNEL))
        .unwrap()
        .edit(&alice.device, &alice.group, &mut alice.rng, |doc| {
            doc.put(ROOT, "after_restart", true)
        })
        .unwrap();
    assert_eq!(exchange_with_cost(&mut alice, &mut bob, None).await, (1, 1));
}

#[tokio::test]
async fn endlessly_advancing_empty_pages_exhaust_grace_and_cooldown_does_not_renew_it() {
    let (mut alice, mut bob, clock) = pair().await;
    let peer = alice.local_peer();
    let limit = MAX_EMPTY_CATCHUP_PAGE_GRACE + usize::from(MAX_NONPROGRESSING_CATCHUP_ROUNDS);
    for round in 1..=limit {
        assert_eq!(
            exchange(
                &mut alice,
                &mut bob,
                Some(empty_page([9; 16], round as u32 * 256))
            )
            .await,
            0
        );
        assert_eq!(
            bob.catchup_peer_is_cooling(peer, DocType::Channel, CHANNEL),
            round == limit
        );
        assert!(bob
            .catchup_continuation_source(DocType::Channel, CHANNEL)
            .is_none());
    }
    clock.advance_ms(CATCHUP_PEER_COOLDOWN_MS + 1_000);
    assert!(!bob.catchup_peer_is_cooling(peer, DocType::Channel, CHANNEL));
    for round in 1..=usize::from(MAX_NONPROGRESSING_CATCHUP_ROUNDS) {
        let page = empty_page([9; 16], (limit + round) as u32 * 256);
        assert_eq!(exchange(&mut alice, &mut bob, Some(page)).await, 0);
        assert_eq!(
            bob.catchup_peer_is_cooling(peer, DocType::Channel, CHANNEL),
            round == usize::from(MAX_NONPROGRESSING_CATCHUP_ROUNDS)
        );
    }
    assert_eq!(
        bob.catchup_empty_page_grace
            .get(&(DocType::Channel, CHANNEL, peer)),
        Some(&MAX_EMPTY_CATCHUP_PAGE_GRACE)
    );
}

#[tokio::test]
async fn repeated_positions_and_alternating_providers_keep_the_original_stall_bound() {
    for alternate_provider in [false, true] {
        let (mut alice, mut bob, _clock) = pair().await;
        let peer = alice.local_peer();
        bob.catchup_cursors.insert(
            (DocType::Channel, CHANNEL, peer),
            CatchupCursor {
                provider: [9; 16],
                position: 256,
            },
        );
        for round in 1..=MAX_NONPROGRESSING_CATCHUP_ROUNDS {
            let provider = if alternate_provider && round % 2 == 1 {
                [8; 16]
            } else {
                [9; 16]
            };
            let position = if alternate_provider {
                (u32::from(round) + 1) * 256
            } else {
                256
            };
            assert_eq!(
                exchange(&mut alice, &mut bob, Some(empty_page(provider, position))).await,
                0
            );
            assert_eq!(
                bob.catchup_peer_is_cooling(peer, DocType::Channel, CHANNEL),
                round == MAX_NONPROGRESSING_CATCHUP_ROUNDS
            );
            assert!(!bob
                .catchup_empty_page_grace
                .contains_key(&(DocType::Channel, CHANNEL, peer)));
        }
    }
}
