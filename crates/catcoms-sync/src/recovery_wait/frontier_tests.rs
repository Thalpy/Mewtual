use super::tests::{answer_owned_request, poll_once};
use super::*;
use automerge::{transaction::Transactable, ROOT};
use catcoms_rt::ManualClock;

fn request_frontier(
    source: &mut ChannelSync<catcoms_rt::MemNetwork, rand_chacha::ChaCha20Rng>,
    target: &mut ChannelSync<catcoms_rt::MemNetwork, rand_chacha::ChaCha20Rng>,
    doc: u128,
    heads: &[[u8; 32]],
) -> Option<Vec<u8>> {
    let (request, _) = source
        .build_authed_request(
            KIND_CATCHUP_SINCE,
            &encode_catchup_since_req(DocType::Channel, doc, heads, None),
        )
        .unwrap();
    target.serve_catchup_since(source.local_peer(), &request[1..])
}

#[tokio::test]
async fn frontier_hints_reject_invalid_scope_and_known_ancestors_without_disturbing_serving() {
    let (_hub, mut members, _) = crate::tests::build_members(2).await;
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    a.note_peer_connected(b.local_peer());
    a.open_channel(DocType::Channel, 917).await.unwrap();
    a.post(DocType::Channel, 917, |doc| doc.put(ROOT, "count", 1))
        .await
        .unwrap();
    let ancestors = a.docs.get_mut(&(DocType::Channel, 917)).unwrap().heads();
    a.post(DocType::Channel, 917, |doc| doc.put(ROOT, "count", 2))
        .await
        .unwrap();
    a.catchup_queue.clear();
    assert!(request_frontier(&mut b, &mut a, 917, &ancestors).is_some());
    assert!(request_frontier(
        &mut b,
        &mut a,
        917,
        &vec![ancestors[0]; MAX_CATCHUP_SINCE_HEADS]
    )
    .is_some());
    assert!(request_frontier(
        &mut b,
        &mut a,
        917,
        &vec![[0xab; 32]; MAX_CATCHUP_SINCE_HEADS + 1]
    )
    .is_none());
    assert!(a.catchup_queue.is_empty() && a.catchup_frontier_hints.is_empty());
    let (mut request, _) = b
        .build_authed_request(
            KIND_CATCHUP_SINCE,
            &encode_catchup_since_req(DocType::Channel, 917, &[[0xab; 32]], None),
        )
        .unwrap();
    *request.last_mut().unwrap() ^= 1;
    assert!(a
        .serve_catchup_since(b.local_peer(), &request[1..])
        .is_none());
    assert!(
        request_frontier(&mut b, &mut a, 918, &[[0xab; 32]]).is_some(),
        "missing document still receives its normal signed absent answer"
    );
    assert!(a.catchup_queue.is_empty() && a.catchup_frontier_hints.is_empty());
}

#[tokio::test]
async fn a_frontier_hint_does_not_displace_a_full_queue_or_amplify_repeated_requests() {
    let (_hub, mut members, ids) = crate::tests::build_members(2).await;
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    a.note_peer_connected(b.local_peer());
    a.open_channel(DocType::Channel, 921).await.unwrap();
    a.catchup_queue = (0..a.config.max_catchup_queue)
        .map(|index| CatchupTask::Doc {
            doc_type: DocType::Channel,
            doc_id: 10_000 + index as u128,
        })
        .collect();
    let owned = a.catchup_queue.clone();
    assert!(request_frontier(&mut b, &mut a, 921, &[[0xab; 32]]).is_some());
    let budget = a.catchup_frontier_hints.clone();
    assert!(budget.contains_key(&(DocType::Channel, 921, ids[1])));
    assert_eq!(
        a.catchup_queue, owned,
        "hint admission never evicts existing recovery ownership"
    );
    assert!(request_frontier(&mut b, &mut a, 921, &[[0xcd; 32]]).is_some());
    assert_eq!(a.catchup_frontier_hints, budget);
    assert_eq!(
        a.catchup_queue, owned,
        "saturation retains the periodic fallback, not a fresh hint grant"
    );
}

#[tokio::test]
async fn a_queued_request_after_disconnect_cannot_grant_reverse_network_work() {
    let (_hub, mut members, _) = crate::tests::build_members(2).await;
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    a.open_channel(DocType::Channel, 922).await.unwrap();
    let peer = b.local_peer();
    a.note_peer_connected(peer);
    let (request, _) = b
        .build_authed_request(
            KIND_CATCHUP_SINCE,
            &encode_catchup_since_req(DocType::Channel, 922, &[[0xab; 32]], None),
        )
        .unwrap();
    drop(b);
    assert!(
        a.connected_peers.contains(&peer),
        "leave the ordered disconnect observation stale"
    );
    a.catchup_queue.clear();
    assert!(a.serve_catchup_since(peer, &request[1..]).is_some());
    assert!(a.catchup_queue.is_empty() && a.catchup_frontier_hints.is_empty());
}

#[tokio::test]
async fn frontier_hint_capacity_refuses_until_expiry_without_clearing_other_members_budget() {
    let (_hub, mut members, ids) = crate::tests::build_members(2).await;
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    let clock = ManualClock::new(1_000);
    a.clock = Arc::new(clock.clone());
    a.note_peer_connected(b.local_peer());
    a.open_channel(DocType::Channel, 919).await.unwrap();
    a.catchup_queue.clear();
    for index in 0..MAX_CATCHUP_STALL_ENTRIES {
        a.catchup_frontier_hints
            .insert((DocType::Channel, 10_000 + index as u128, ids[1]), 2_000);
    }
    assert!(request_frontier(&mut b, &mut a, 919, &[[0xab; 32]]).is_some());
    assert_eq!(a.catchup_frontier_hints.len(), MAX_CATCHUP_STALL_ENTRIES);
    assert!(
        a.catchup_queue.is_empty(),
        "full unexpired table grants no uncharged new hint"
    );
    clock.advance_ms(1_000);
    assert!(request_frontier(&mut b, &mut a, 919, &[[0xab; 32]]).is_some());
    assert_eq!(a.catchup_frontier_hints.len(), 1);
    assert!(!a.catchup_queue.is_empty());
}

#[tokio::test]
async fn frontier_hint_budget_follows_the_full_member_across_transport_rotation() {
    let (hub, mut members, ids) = crate::tests::build_members(2).await;
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    a.note_peer_connected(b.local_peer());
    a.open_channel(DocType::Channel, 920).await.unwrap();
    a.catchup_queue.clear();
    assert!(request_frontier(&mut b, &mut a, 920, &[[0xab; 32]]).is_some());
    let original_budget = a.catchup_frontier_hints.clone();
    a.catchup_queue.clear();
    b.transport = Arc::new(hub.join(PeerId::from_u64(88_888)));
    a.note_peer_connected(b.local_peer());
    assert!(request_frontier(&mut b, &mut a, 920, &[[0xcd; 32]]).is_some());
    assert_eq!(a.catchup_frontier_hints, original_budget);
    assert!(a
        .catchup_frontier_hints
        .contains_key(&(DocType::Channel, 920, ids[1])));
    assert!(
        a.catchup_queue.is_empty(),
        "new Noise identity cannot refill the same member's hint allowance"
    );
}

#[tokio::test]
async fn serving_a_later_frontier_recovers_onward_history_without_gossip_or_a_timer() {
    let (_hub, mut members, ids) = crate::tests::build_members(3).await;
    crate::tests::converge_and_publish_test_routes(&mut members);
    let mut c = members.pop().unwrap();
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    let clock = ManualClock::new(1_000);
    crate::tests::prove_live_member(&mut a, b.local_peer(), ids[1]);
    for node in [&mut a, &mut b, &mut c] {
        node.clock = Arc::new(clock.clone());
        node.open_channel(DocType::Channel, 915).await.unwrap();
    }
    a.note_source_checked(b.local_peer(), DocType::Channel, 915);
    a.catchup_queue.clear();
    a.reconciliation_next_ms = 31_000;
    c.post(DocType::Channel, 915, |doc| {
        doc.put(ROOT, "onward", "original C operation")
    })
    .await
    .unwrap();
    for node in [&mut a, &mut b] {
        assert!(matches!(
            node.transport.next_event().await,
            Some(TransportEvent::Gossip { .. })
        ));
    }
    b.begin_catchup_wait(
        CatchupTask::Doc {
            doc_type: DocType::Channel,
            doc_id: 915,
        },
        c.local_peer(),
        Shape::Since,
    )
    .unwrap();
    answer_owned_request(&mut b, &mut c, false).await;
    assert_eq!(b.doc_version(DocType::Channel, 915), 1);
    // B's ordinary request carries its later frontier. A serves it without any UI, discovery,
    // hint command, injected queue or elapsed interval. That request must reopen A's search.
    b.begin_catchup_wait(
        CatchupTask::Doc {
            doc_type: DocType::Channel,
            doc_id: 915,
        },
        a.local_peer(),
        Shape::Since,
    )
    .unwrap();
    answer_owned_request(&mut b, &mut a, false).await;
    let mut turn = Box::pin(a.run_once());
    let _ = poll_once(turn.as_mut()).await;
    drop(turn);
    assert!(
        a.pending_catchup.is_some(),
        "serving the new frontier must start owned reverse recovery"
    );
    answer_owned_request(&mut a, &mut b, false).await;
    assert_eq!(a.doc_version(DocType::Channel, 915), 1);
    assert_eq!(clock.monotonic_ms(), 1_000);
}

#[tokio::test]
async fn two_successive_verified_onward_acquisitions_reopen_the_same_source_without_a_timer() {
    let (_hub, mut members, ids) = crate::tests::build_members(3).await;
    crate::tests::converge_and_publish_test_routes(&mut members);
    let mut c = members.pop().unwrap();
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    let clock = ManualClock::new(1_000);
    for node in [&mut a, &mut b, &mut c] {
        node.clock = Arc::new(clock.clone());
        node.open_channel(DocType::Channel, 923).await.unwrap();
    }
    crate::tests::prove_live_member(&mut a, b.local_peer(), ids[1]);
    a.catchup_queue.clear();
    a.bound_source_reconciliation_owed = false;
    a.reconciliation_next_ms = 31_000;
    for wave in 1..=2 {
        c.post(DocType::Channel, 923, |doc| doc.put(ROOT, "wave", wave))
            .await
            .unwrap();
        // Drop every live publication. A can learn these original signed operations only
        // through B's actual authenticated request/response, with no UI or elapsed interval.
        for node in [&mut a, &mut b] {
            assert!(matches!(
                node.transport.next_event().await,
                Some(TransportEvent::Gossip { .. })
            ));
        }
        b.begin_catchup_wait(
            CatchupTask::Doc {
                doc_type: DocType::Channel,
                doc_id: 923,
            },
            c.local_peer(),
            Shape::Since,
        )
        .unwrap();
        answer_owned_request(&mut b, &mut c, false).await;
        assert_eq!(b.doc_version(DocType::Channel, 923), wave as u64);
        b.begin_catchup_wait(
            CatchupTask::Doc {
                doc_type: DocType::Channel,
                doc_id: 923,
            },
            a.local_peer(),
            Shape::Since,
        )
        .unwrap();
        answer_owned_request(&mut b, &mut a, false).await;
        let mut turn = Box::pin(a.run_once());
        let _ = poll_once(turn.as_mut()).await;
        drop(turn);
        assert!(
            a.pending_catchup.is_some(),
            "verified progress from the first hint must not suppress onward wave {wave}"
        );
        answer_owned_request(&mut a, &mut b, false).await;
        assert_eq!(a.doc_version(DocType::Channel, 923), wave as u64);
        // Complete the first search through an ACTUAL empty signed reply, so the next wave
        // cannot rely on a pre-existing queue, pending request or unchecked-source marker.
        a.start_queued_catchup();
        assert!(
            a.pending_catchup.is_some(),
            "verified import retains its follow-up"
        );
        answer_owned_request(&mut a, &mut b, false).await;
        assert!(a
            .sources_checked(DocType::Channel, 923)
            .contains(&b.local_peer()));
        assert!(a.catchup_queue.is_empty() && a.pending_catchup.is_none());
    }
    assert_eq!(clock.monotonic_ms(), 1_000);
}

#[tokio::test]
async fn only_new_fully_accepted_history_from_the_exact_source_rearms_its_hint() {
    let (_hub, mut members, _) = crate::tests::build_members(3).await;
    crate::tests::converge_and_publish_test_routes(&mut members);
    let mut c = members.pop().unwrap();
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    let clock = ManualClock::new(1_000);
    let key = (DocType::Channel, 924);
    for node in [&mut a, &mut b, &mut c] {
        node.clock = Arc::new(clock.clone());
        node.open_channel(key.0, key.1).await.unwrap();
    }
    let b_key = (key.0, key.1, b.device.device_id());
    let c_key = (key.0, key.1, c.device.device_id());
    assert!(request_frontier(&mut b, &mut a, key.1, &[[0xab; 32]]).is_some());
    assert!(request_frontier(&mut c, &mut a, key.1, &[[0xcd; 32]]).is_some());
    let b_budget = a.catchup_frontier_hints[&b_key];
    c.post(key.0, key.1, |doc| doc.put(ROOT, "from_c", 1))
        .await
        .unwrap();
    for node in [&mut a, &mut b] {
        assert!(matches!(
            node.transport.next_event().await,
            Some(TransportEvent::Gossip { .. })
        ));
    }
    a.begin_catchup_wait(
        CatchupTask::Doc {
            doc_type: key.0,
            doc_id: key.1,
        },
        c.local_peer(),
        Shape::Since,
    )
    .unwrap();
    answer_owned_request(&mut a, &mut c, false).await;
    assert_eq!(a.doc_version(key.0, key.1), 1);
    assert!(!a.catchup_frontier_hints.contains_key(&c_key));
    assert_eq!(
        a.catchup_frontier_hints[&b_key], b_budget,
        "another source's real progress cannot replenish B's grant"
    );

    assert!(request_frontier(&mut c, &mut a, key.1, &[[0xee; 32]]).is_some());
    let c_budget = a.catchup_frontier_hints[&c_key];
    // A real empty answer still comes from C, but it has supplied no further history.
    a.begin_catchup_wait(
        CatchupTask::Doc {
            doc_type: key.0,
            doc_id: key.1,
        },
        c.local_peer(),
        Shape::Since,
    )
    .unwrap();
    answer_owned_request(&mut a, &mut c, false).await;
    assert_eq!(a.catchup_frontier_hints[&c_key], c_budget);

    // Re-send a valid ORIGINAL operation in a fresh authenticated response. Byte count,
    // signatures and a successful parse are insufficient: duplicate history is no progress.
    let repeated = c.docs[&key]
        .export_catchup(&c.group, &c.device, &mut c.rng)
        .unwrap();
    let (request, auth) = a
        .build_authed_request(
            KIND_CATCHUP_SINCE,
            &encode_catchup_since_req(key.0, key.1, &[], None),
        )
        .unwrap();
    let mut answer = vec![CATCHUP_SINCE_UNDERSTOOD];
    answer.extend_from_slice(&encode_bundle(&repeated));
    let response = crate::tests::signed_catchup_answer(&mut c, a.local_peer(), &request, answer);
    assert_eq!(
        a.apply_catchup_since_response(c.local_peer(), key.0, key.1, auth, &response)
            .unwrap(),
        Some(0)
    );
    assert_eq!(a.catchup_frontier_hints[&c_key], c_budget);

    b.post(key.0, key.1, |doc| doc.put(ROOT, "from_b", 1))
        .await
        .unwrap();
    let first = b.docs[&key]
        .export_catchup(&b.group, &b.device, &mut b.rng)
        .unwrap()
        .remove(0);
    let mut bad = first.clone();
    bad.epoch = bad.epoch.saturating_add(1); // Well-framed second item; unavailable epoch.
    let (request, auth) = a
        .build_authed_request(
            KIND_CATCHUP_SINCE,
            &encode_catchup_since_req(key.0, key.1, &[], None),
        )
        .unwrap();
    let mut answer = vec![CATCHUP_SINCE_UNDERSTOOD];
    answer.extend_from_slice(&encode_bundle(&[first, bad]));
    let response = crate::tests::signed_catchup_answer(&mut b, a.local_peer(), &request, answer);
    assert!(a
        .apply_catchup_since_response(b.local_peer(), key.0, key.1, auth, &response)
        .is_err());
    assert_eq!(
        a.doc_version(key.0, key.1),
        2,
        "valid prefix remains in the additive ledger"
    );
    assert_eq!(
        a.catchup_frontier_hints[&b_key], b_budget,
        "a rejected tail cannot refill the exact source's hint allowance"
    );
    assert!(request_frontier(&mut b, &mut a, key.1, &[[0xef; 32]]).is_some());
    assert_eq!(
        a.catchup_frontier_hints[&b_key], b_budget,
        "changing fabricated hashes at unchanged progress stays throttled"
    );
    assert_eq!(clock.monotonic_ms(), 1_000);
}

#[tokio::test]
async fn understood_more_and_paged_progress_rearm_but_duplicate_and_cursor_only_pages_do_not() {
    for marker in [
        CATCHUP_SINCE_UNDERSTOOD,
        CATCHUP_SINCE_MORE,
        CATCHUP_SINCE_PAGE,
    ] {
        let (_hub, mut members, _) = crate::tests::build_members(2).await;
        crate::tests::converge_and_publish_test_routes(&mut members);
        let mut b = members.pop().unwrap();
        let mut a = members.pop().unwrap();
        for node in [&mut a, &mut b] {
            node.open_channel(DocType::Channel, 925).await.unwrap();
        }
        let hint = (DocType::Channel, 925, b.device.device_id());
        assert!(request_frontier(&mut b, &mut a, 925, &[[0xab; 32]]).is_some());
        b.post(DocType::Channel, 925, |doc| doc.put(ROOT, "verified", 1))
            .await
            .unwrap();
        let ops = b.docs[&(DocType::Channel, 925)]
            .export_catchup(&b.group, &b.device, &mut b.rng)
            .unwrap();
        let page = CatchupCursor {
            provider: b.catchup_provider,
            position: 1,
        };
        let mut answer = vec![marker];
        if marker == CATCHUP_SINCE_PAGE {
            answer.extend_from_slice(&page.encode());
        }
        answer.extend_from_slice(&encode_bundle(&ops));
        let (request, auth) = a
            .build_authed_request(
                KIND_CATCHUP_SINCE,
                &encode_catchup_since_req(DocType::Channel, 925, &[], None),
            )
            .unwrap();
        let response =
            crate::tests::signed_catchup_answer(&mut b, a.local_peer(), &request, answer.clone());
        assert_eq!(
            a.apply_catchup_since_response(b.local_peer(), DocType::Channel, 925, auth, &response)
                .unwrap(),
            Some(1)
        );
        assert!(
            !a.catchup_frontier_hints.contains_key(&hint),
            "marker {marker}"
        );
        assert!(request_frontier(&mut b, &mut a, 925, &[[0xcd; 32]]).is_some());
        let budget = a.catchup_frontier_hints[&hint];
        let (request, auth) = a
            .build_authed_request(
                KIND_CATCHUP_SINCE,
                &encode_catchup_since_req(DocType::Channel, 925, &[], None),
            )
            .unwrap();
        let response =
            crate::tests::signed_catchup_answer(&mut b, a.local_peer(), &request, answer);
        assert_eq!(
            a.apply_catchup_since_response(b.local_peer(), DocType::Channel, 925, auth, &response)
                .unwrap(),
            Some(0)
        );
        assert_eq!(
            a.catchup_frontier_hints[&hint], budget,
            "duplicate marker {marker}"
        );
        if marker == CATCHUP_SINCE_PAGE {
            let mut answer = vec![CATCHUP_SINCE_PAGE];
            answer.extend_from_slice(
                &CatchupCursor {
                    position: 2,
                    ..page
                }
                .encode(),
            );
            answer.extend_from_slice(&encode_bundle(&[]));
            let (request, auth) = a
                .build_authed_request(
                    KIND_CATCHUP_SINCE,
                    &encode_catchup_since_req(DocType::Channel, 925, &[], Some(&page)),
                )
                .unwrap();
            let response =
                crate::tests::signed_catchup_answer(&mut b, a.local_peer(), &request, answer);
            assert_eq!(
                a.apply_catchup_since_response(
                    b.local_peer(),
                    DocType::Channel,
                    925,
                    auth,
                    &response
                )
                .unwrap(),
                Some(0)
            );
            assert_eq!(
                a.catchup_frontier_hints[&hint], budget,
                "an advancing cursor is not verified history"
            );
        }
    }
}

#[tokio::test]
async fn an_old_empty_reply_cannot_retire_a_newer_authenticated_frontier_hint() {
    for legacy in [false, true] {
        let (_hub, mut members, ids) = crate::tests::build_members(2).await;
        crate::tests::converge_and_publish_test_routes(&mut members);
        let mut b = members.pop().unwrap();
        let mut a = members.pop().unwrap();
        for node in [&mut a, &mut b] {
            node.open_channel(DocType::Channel, 916).await.unwrap();
        }
        crate::tests::prove_live_member(&mut a, b.local_peer(), ids[1]);
        a.bound_source_reconciliation_owed = false;
        a.catchup_queue.clear();
        a.begin_catchup_wait(
            CatchupTask::Doc {
                doc_type: DocType::Channel,
                doc_id: 916,
            },
            b.local_peer(),
            Shape::Since,
        )
        .unwrap();
        assert!(
            poll_once(a.pending_catchup.as_mut().unwrap().response.as_mut())
                .await
                .is_pending()
        );
        let Some(TransportEvent::Request {
            from,
            data,
            responder,
            ..
        }) = b.transport.next_event().await
        else {
            panic!("original request");
        };
        let old_empty = if legacy {
            Vec::new()
        } else {
            b.handle_request(from, &data)
        };
        b.post(DocType::Channel, 916, |doc| {
            doc.put(ROOT, "new", "after the old empty answer")
        })
        .await
        .unwrap();
        assert!(matches!(
            a.transport.next_event().await,
            Some(TransportEvent::Gossip { .. })
        ));
        b.begin_catchup_wait(
            CatchupTask::Doc {
                doc_type: DocType::Channel,
                doc_id: 916,
            },
            a.local_peer(),
            Shape::Since,
        )
        .unwrap();
        answer_owned_request(&mut b, &mut a, false).await;
        responder.respond(Bytes::from(old_empty));
        let old = a.pending_catchup.as_mut().unwrap().response.as_mut().await;
        a.complete_queued_catchup(old);
        if legacy {
            assert!(
                a.pending_catchup.as_ref().unwrap().completeness_dirty,
                "Since-to-Full fallback retains the already admitted hint"
            );
            answer_owned_request(&mut a, &mut b, false).await;
            assert_eq!(a.doc_version(DocType::Channel, 916), 1);
            continue;
        }
        assert!(
            !a.sources_checked(DocType::Channel, 916)
                .contains(&b.local_peer()),
            "old completion must not overwrite the admitted later hint"
        );
        a.start_queued_catchup();
        assert!(a.pending_catchup.is_some());
        answer_owned_request(&mut a, &mut b, false).await;
        assert_eq!(a.doc_version(DocType::Channel, 916), 1);
    }
}
