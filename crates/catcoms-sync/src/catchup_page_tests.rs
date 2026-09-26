//! The requester must accept bounded empty scan pages without accepting an endless empty walk.
use super::*;
use automerge::{transaction::Transactable, ReadDoc, ROOT};
use catcoms_rt::{ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;

type Member = ChannelSync<MemNetwork, ChaCha20Rng>;
const CHANNEL: u128 = 913;

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
    let peer = provider.local_peer();
    let (applied, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
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
                            responder.respond(Bytes::from(response));
                            return;
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
    applied.expect("the signed response authenticates against this request and provider")
}

fn empty_page(provider: [u8; 16], position: u32) -> Vec<u8> {
    let mut answer = vec![CATCHUP_SINCE_PAGE];
    answer.extend_from_slice(&CatchupCursor { provider, position }.encode());
    answer.extend_from_slice(&encode_bundle(&[]));
    answer
}

#[tokio::test]
async fn the_requester_crosses_eight_empty_bounded_pages_without_cooling_an_honest_provider() {
    let (mut alice, mut bob, _clock) = pair().await;
    for count in 0..catcoms_replication::doc::MAX_CATCHUP_PAGE_CLOSURE_STEPS {
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
    for round in 1..=MAX_EMPTY_CATCHUP_PAGE_GRACE {
        assert_eq!(exchange(&mut alice, &mut bob, None).await, 0);
        assert!(!bob.catchup_peer_is_cooling(peer, DocType::Channel, CHANNEL));
        assert_eq!(
            bob.catchup_empty_page_grace
                .get(&(DocType::Channel, CHANNEL, peer)),
            Some(&round)
        );
        assert_eq!(
            bob.catchup_cursors[&(DocType::Channel, CHANNEL, peer)].position as usize,
            round * catcoms_replication::doc::MAX_CATCHUP_PAGE_SCANNED_OPS
        );
    }
    assert_eq!(exchange(&mut alice, &mut bob, None).await, 1);
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
