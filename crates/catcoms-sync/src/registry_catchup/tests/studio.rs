use super::*;

fn target() -> StudioTarget {
    StudioTarget::Flipnote {
        channel: [7; 16],
        object: [8; 16],
    }
}

#[test]
fn studio_page_peers_prioritizes_connected_proven_owner_before_four_peer_cap() {
    let hub = Hub::new();
    let owner = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&owner).unwrap();
    let members: Vec<_> = (0..4).map(|_| MlsDevice::generate().unwrap()).collect();
    for member in &members {
        group
            .add_member(&owner, member.key_package().unwrap())
            .unwrap();
    }
    let mut node = ChannelSync::new(
        hub.join(PeerId::from_u64(9)),
        group,
        owner,
        ChaCha20Rng::seed_from_u64(19),
        Box::new(ManualClock::new(1000)),
    );
    let owner_peer = PeerId::from_u64(10);
    let _owner_net = hub.join(owner_peer);
    node.promote_member_peer_bound(owner_peer, node.device.device_id(), true);
    let mut nets = Vec::new();
    for (i, member) in members.iter().enumerate() {
        let peer = PeerId::from_u64(11 + i as u64);
        nets.push(hub.join(peer));
        node.promote_member_peer_bound(peer, member.device_id(), true);
    }
    let peers = node.studio_page_peers();
    assert_eq!(peers.len(), 4);
    assert_eq!(peers[0], owner_peer, "owner was fifth in recency order");
    node.member_peers
        .iter_mut()
        .find(|p| p.peer == owner_peer)
        .unwrap()
        .bound = false;
    assert!(
        !node.studio_page_peers().contains(&owner_peer),
        "priority never upgrades an unproven endpoint"
    );
}

#[tokio::test]
async fn epoch_service_unopened_page_does_not_create_or_replace_receive_watches() {
    let (mut node, _, _, _) = setup();
    let user = node.watch_studio(target(), 42).unwrap();
    node.enable_epoch_service();
    // Another concrete epoch is a service request, never a request to retarget the user's inbox.
    let bytes = encode_scoped_query(PageScope::Studio(target()), 99, &[], None, None).unwrap();
    let (bytes, _) = node.build_authed_request(KIND_STUDIO_PAGE, &bytes).unwrap();
    let (tx, rx) = Responder::channel();
    node.queue_epoch_page_request(KIND_STUDIO_PAGE, node.local_peer(), &bytes[1..], tx);
    let interest = node.reserve_epoch_service_interest().unwrap();
    assert_eq!(interest.doc_id(), Some(99));
    assert!(node.studio_watch_is_current(&user));
    assert!(node.reserve_epoch_service_interest().is_none());
    node.serve_epoch_page_interest(&interest, |_, _, _, _| {
        Ok::<_, ()>(RegistryPageOutcome::CheckpointRequired)
    })
    .unwrap()
    .unwrap()
    .unwrap();
    assert!(rx.recv().await.is_some());
    assert!(node.studio_watch_is_current(&user));
    assert_eq!(node.studio_exchange.watches.len(), 1);
}

#[tokio::test]
async fn studio_page_transport_shares_registry_debt_and_rejects_watch_scope_replay() {
    let (mut node, _, registry, clock) = setup();
    let studio = node.watch_studio(target(), 42).unwrap();
    let request = |node: &mut ChannelSync<MemNetwork, ChaCha20Rng>| {
        let inner = encode_scoped_query(PageScope::Studio(target()), 42, &[], None, None).unwrap();
        let (request, _) = node.build_authed_request(KIND_STUDIO_PAGE, &inner).unwrap();
        request
    };
    // One queue and one full-identity debt across both kinds, not a fresh budget per adapter.
    let _held = enqueue(&mut node, &registry);
    let bytes = request(&mut node);
    let (responder, rx) = Responder::channel();
    node.queue_epoch_page_request(KIND_STUDIO_PAGE, node.local_peer(), &bytes[1..], responder);
    assert!(rx.recv().await.is_none());
    node.watch_registry(registry.bucket, registry.doc_id);
    let bytes = request(&mut node);
    let (responder, rx) = Responder::channel();
    node.queue_epoch_page_request(KIND_STUDIO_PAGE, node.local_peer(), &bytes[1..], responder);
    assert_eq!(node.registry_pages.pending.len(), 1);
    node.unwatch_studio(&studio).unwrap();
    assert!(rx.recv().await.is_none());
    assert!(enqueue(&mut node, &registry).recv().await.is_none());
    clock.advance_ms(1000);
    let watch = node.watch_studio(target(), 42).unwrap();
    let bytes = request(&mut node);
    let (responder, rx) = Responder::channel();
    node.queue_epoch_page_request(KIND_STUDIO_PAGE, node.local_peer(), &bytes[1..], responder);
    clock.advance_ms(5000);
    assert!(node
        .serve_studio_request(&watch, |_, _, _, _| -> Result<_, ()> {
            panic!("expired query read source")
        })
        .unwrap()
        .is_none());
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn studio_page_detached_response_binds_channel_kind_peer_and_completion_lifetime() {
    let (mut original, _, _, _) = setup();
    let snapshot = original.snapshot().unwrap();
    for mode in 0..7 {
        let hub = Hub::new();
        let peer = PeerId::from_u64(2);
        let remote = hub.join(peer);
        let clock = ManualClock::new(1000);
        let mut client = ChannelSync::restore(
            &snapshot,
            hub.join(PeerId::from_u64(1)),
            ChaCha20Rng::seed_from_u64(182),
            Box::new(clock.clone()),
        )
        .unwrap();
        let watch = client.watch_studio(target(), 42).unwrap();
        let query = || StudioPageQuery {
            target: target(),
            doc_id: 42,
            heads: &[],
            seed: None,
            cursor: None,
        };
        assert!(
            client.prepare_studio_page(&watch, peer, query()).is_err(),
            "unproven endpoint cannot receive identifiers"
        );
        client.promote_member_peer_bound(peer, original.device.device_id(), true);
        let request = client.prepare_studio_page(&watch, peer, query()).unwrap();
        let (completed, ()) = tokio::join!(request.fetch(), async {
            let Some(TransportEvent::Request {
                data, responder, ..
            }) = remote.next_event().await
            else {
                panic!()
            };
            assert_eq!(data[0], 23);
            let (mut inner, key, ts, nonce, epoch, _) = decode_authed_request(&data[1..]).unwrap();
            if mode == 1 {
                inner[3] ^= 1;
            } // channel
            if mode == 2 {
                inner[19] ^= 1;
            } // object
            let answer = vec![1, 1];
            let domain = if mode == 3 {
                RESPONSE_DOMAIN
            } else {
                "catcoms/studio-page-response/v1"
            };
            let endpoint = if mode == 4 {
                PeerId::from_u64(99)
            } else {
                peer
            };
            let bound = scoped_response_transcript(
                domain,
                &original.group.group_id(),
                &key,
                &RequestAuth { ts, nonce, epoch },
                endpoint,
                &inner,
                &answer,
            );
            responder.respond(Bytes::from(encode_signed_commit_resp(
                &original.device.public_key_bytes(),
                &original.device.sign(&bound).unwrap(),
                &answer,
            )));
        });
        if mode == 5 {
            clock.advance_ms(10_000);
        }
        if mode == 6 {
            client.watch_studio(target(), 42).unwrap();
        }
        let result = client.complete_studio_page(completed);
        if mode == 0 {
            assert!(matches!(
                result.unwrap(),
                Some(RegistryPageOutcome::Restart)
            ));
        } else {
            assert!(result.is_err(), "mode {mode}");
        }
    }
}

#[test]
fn studio_page_scope_wire_is_fixed_width_and_index_canonical() {
    let bytes = encode_scoped_query(PageScope::Studio(target()), 42, &[], None, None).unwrap();
    // Independent fixed field vector: version, tag16, channel, object, concrete id, zero heads,
    // empty seed and cursor. Registry's original one-byte bucket query is unchanged.
    let mut expected = vec![1, 0, 16];
    expected.extend([7; 16]);
    expected.extend([8; 16]);
    expected.extend(42u128.to_be_bytes());
    expected.extend([0; 9]);
    assert_eq!(bytes, expected);
    assert_eq!(
        decode_scoped_query(23, &bytes).unwrap().scope,
        PageScope::Studio(target())
    );
    assert!(decode_query(&bytes).is_err());
    let mut index = encode_scoped_query(
        PageScope::Studio(StudioTarget::Index { channel: [7; 16] }),
        42,
        &[],
        None,
        None,
    )
    .unwrap();
    index[19] = 1;
    assert!(decode_scoped_query(23, &index).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(decode_scoped_query(23, &trailing).is_err());
}

#[test]
fn studio_page_cancelled_submitted_attempt_keeps_shared_lower_driver_capacity() {
    let (mut original, _, _, clock) = setup();
    let transport = DelayedDriver {
        inner: Hub::new().join(PeerId::from_u64(1)),
        retained: Arc::default(),
    };
    let mut node = ChannelSync::restore(
        &original.snapshot().unwrap(),
        transport.clone(),
        ChaCha20Rng::seed_from_u64(191),
        Box::new(clock.clone()),
    )
    .unwrap();
    let peer = PeerId::from_u64(2);
    node.promote_member_peer_bound(peer, node.device.device_id(), true);
    let watch = node.watch_studio(target(), 42).unwrap();
    let query = || StudioPageQuery {
        target: target(),
        doc_id: 42,
        heads: &[],
        seed: None,
        cursor: None,
    };
    // Time spent queued before polling is part of the deadline, with no transport admission.
    let request = node.prepare_studio_page(&watch, peer, query()).unwrap();
    clock.advance_ms(10_000);
    let mut future = Box::pin(request.fetch());
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(future.as_mut().poll(&mut cx).is_ready());
    drop(future);
    assert!(transport.retained.lock().unwrap().is_empty());
    for _ in 0..4 {
        let request = node.prepare_studio_page(&watch, peer, query()).unwrap();
        let mut future = Box::pin(request.fetch());
        assert!(future.as_mut().poll(&mut cx).is_pending());
        drop(future);
    }
    assert!(transport
        .retained
        .lock()
        .unwrap()
        .iter()
        .all(RequestCancellation::is_cancelled));
    assert!(node.prepare_studio_page(&watch, peer, query()).is_err());
    transport.retained.lock().unwrap().clear();
    assert!(node.prepare_studio_page(&watch, peer, query()).is_ok());
}
