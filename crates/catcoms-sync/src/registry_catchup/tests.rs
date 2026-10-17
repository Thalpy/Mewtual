use super::*;
use catcoms_replication::{
    registry::{PointerKey, RegistryOp},
    registry_epoch::{catchup::RegistryPageProvider, RegistryEpoch},
};
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::future::Future;

mod studio;

fn setup() -> (
    ChannelSync<MemNetwork, ChaCha20Rng>,
    RegistryEpoch,
    RegistryWatch,
    ManualClock,
) {
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(980);
    let key = PointerKey::new(DocType::StudioObject, b"page-wire-test".to_vec()).unwrap();
    let mut source = RegistryEpoch::new(&group, key.bucket(), device.device_id()).unwrap();
    for n in 0..33u8 {
        let op = RegistryOp::Put {
            key: key.clone(),
            epoch: n as u64,
        }
        .domain_op(&group.group_id(), [n; 16])
        .unwrap();
        source.edit(&device, &group, &mut rng, &op).unwrap();
    }
    let mut node = ChannelSync::new(
        Hub::new().join(PeerId::from_u64(1)),
        group,
        device,
        rng,
        Box::new(clock.clone()),
    );
    let watch = node.watch_registry(key.bucket(), source.doc_id());
    (node, source, watch, clock)
}
fn query(watch: &RegistryWatch) -> RegistryPageQuery<'_> {
    RegistryPageQuery {
        bucket: watch.bucket,
        doc_id: watch.doc_id,
        heads: &[],
        seed: None,
        cursor: None,
    }
}
fn enqueue(
    node: &mut ChannelSync<MemNetwork, ChaCha20Rng>,
    watch: &RegistryWatch,
) -> catcoms_rt::ResponderRx {
    let inner = encode_query(&query(watch)).unwrap();
    let (request, _) = node
        .build_authed_request(KIND_REGISTRY_PAGE, &inner)
        .unwrap();
    let (responder, rx) = Responder::channel();
    node.queue_registry_page_request(node.transport.local_peer(), &request[1..], responder);
    rx
}

#[tokio::test]
async fn registry_page_global_queue_service_and_debt_tables_are_bounded() {
    let (mut node, _, watch, clock) = setup();
    let members: Vec<_> = (0..9).map(|_| MlsDevice::generate().unwrap()).collect();
    for member in &members {
        node.group
            .add_member(&node.device, member.key_package().unwrap())
            .unwrap();
    }
    let inner = encode_query(&query(&watch)).unwrap();
    let mut replies = Vec::new();
    for (n, member) in members.iter().enumerate() {
        let from = PeerId::from_u64(100 + n as u64);
        let key = member.public_key_bytes();
        let ts = node.clock.now_ms();
        let nonce = [n as u8; 16];
        let epoch = node.group.epoch();
        let signature = member
            .sign(&catchup_auth_transcript(
                &node.group.group_id(),
                KIND_REGISTRY_PAGE,
                &inner,
                &key,
                ts,
                &nonce,
                epoch,
                Some(&from),
            ))
            .unwrap();
        let request = encode_authed_request(&inner, &key, ts, &nonce, epoch, &signature);
        let (tx, rx) = Responder::channel();
        node.queue_registry_page_request(from, &request, tx);
        replies.push(rx);
    }
    assert_eq!(node.registry_pages.pending.len(), MAX_PENDING);
    assert!(replies.pop().unwrap().recv().await.is_none());
    for _ in 0..4 {
        assert!(node
            .serve_registry_request(&watch, |_, _, _, _| Ok::<_, ()>(
                RegistryPageOutcome::Restart
            ))
            .unwrap()
            .unwrap()
            .is_ok());
    }
    let blocked =
        node.serve_registry_request::<()>(&watch, |_, _, _, _| panic!("global source rail"));
    assert!(blocked.unwrap().is_none());
    assert_eq!(node.registry_pages.pending.len(), 4);
    clock.advance_ms(499);
    assert!(node
        .serve_registry_request::<()>(&watch, |_, _, _, _| panic!("too early"))
        .unwrap()
        .is_none());
    clock.advance_ms(1);
    // Source errors consume one bounded request without sending a success or earning an ack.
    assert_eq!(
        node.serve_registry_request(&watch, |_, _, _, _| Err::<RegistryPageOutcome, _>(
            "source failed"
        ))
        .unwrap()
        .unwrap(),
        Err("source failed")
    );
    node.watch_registry(watch.bucket, watch.doc_id);
    for _ in 0..20 {
        let (tx, _) = Responder::channel();
        node.queue_registry_page_request(node.transport.local_peer(), &[0], tx);
    }
    assert!(
        enqueue(&mut node, &watch).recv().await.is_none(),
        "preauth debt survives replacement"
    );
    clock.advance_ms(1000);
    node.registry_pages.requesters.clear();
    for n in 0..MAX_REQUESTERS {
        let mut rate = Rate::full(clock.monotonic_ms(), 2);
        rate.charge(clock.monotonic_ms(), 1, 2);
        node.registry_pages
            .requesters
            .insert(DeviceId::from_public_key_bytes(&n.to_be_bytes()), rate);
    }
    assert!(enqueue(&mut node, &watch).recv().await.is_none());
    assert_eq!(node.registry_pages.requesters.len(), MAX_REQUESTERS);
    clock.advance_ms(1000);
    let rx = enqueue(&mut node, &watch);
    assert_eq!(
        node.registry_pages.requesters.len(),
        1,
        "only fully refilled debt may be reclaimed"
    );
    drop(rx);
}

#[test]
fn registry_page_wire_is_canonical_bounded_and_scope_checked() {
    let (mut node, source, watch, clock) = setup();
    let bytes = encode_query(&RegistryPageQuery {
        bucket: 7,
        doc_id: 9,
        heads: &[],
        seed: None,
        cursor: None,
    })
    .unwrap();
    // Fixed-width BE id, then zero heads and two empty length-framed optional fields.
    assert_eq!(
        hex::encode(&bytes),
        "010700000000000000000000000000000009000000000000000000"
    );
    for len in 0..bytes.len() {
        assert!(decode_query(&bytes[..len]).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(decode_query(&trailing).is_err());
    let mut bad = bytes.clone();
    bad[0] = 2;
    assert!(decode_query(&bad).is_err());
    let mut bad = bytes.clone();
    bad[18] = 65;
    assert!(decode_query(&bad).is_err());
    for heads in [vec![[1; 32]; 2], vec![[1; 32]; 65], vec![[2; 32], [1; 32]]] {
        assert!(encode_query(&RegistryPageQuery {
            heads: &heads,
            ..query(&watch)
        })
        .is_err());
    }
    for bytes in [&[][..], &[1][..], &[0; 81][..], &[1; 82][..]] {
        assert!(RegistryPageCursor::from_bytes(bytes).is_err());
    }
    let mut provider =
        RegistryPageProvider::new(node.device.device_id(), Arc::new(clock), &mut node.rng);
    let result = provider
        .page(
            &source,
            &node.group,
            &node.device,
            RegistryPageRequest {
                requester: node.device.device_id(),
                doc_id: watch.doc_id,
                heads: &[],
                seed: None,
                cursor: None,
            },
            &mut node.rng,
        )
        .unwrap();
    let bytes = encode_answer(&result, watch.doc_id, node.group.epoch()).unwrap();
    assert!(
        matches!(decode_answer(&bytes, watch.doc_id, node.group.epoch()).unwrap(), RegistryPageOutcome::Page(page) if page.operations.len() == 32 && page.next.is_some())
    );
    assert!(decode_answer(&bytes, watch.doc_id ^ 1, node.group.epoch()).is_err());
    assert!(decode_answer(&bytes, watch.doc_id, node.group.epoch() + 1).is_err());
    let RegistryPageOutcome::Page(mut page) = result else {
        panic!()
    };
    page.operations.clear();
    assert!(encode_answer(
        &RegistryPageOutcome::Page(page),
        watch.doc_id,
        node.group.epoch()
    )
    .is_err());
    for status in [
        RegistryPageOutcome::Restart,
        RegistryPageOutcome::CheckpointRequired,
        RegistryPageOutcome::HistoricalAuthorizationRequired,
    ] {
        let mut bytes = encode_answer(&status, 0, 0).unwrap();
        assert!(decode_answer(&bytes, 0, 0).is_ok());
        bytes.push(0);
        assert!(decode_answer(&bytes, 0, 0).is_err());
    }
    assert!(decode_response(&vec![0; MAX_RESPONSE + 1]).is_err());
    assert!(!format!("{:?}", query(&watch)).contains("page-wire-test"));
}

#[tokio::test]
async fn registry_page_request_rechecks_authority_expiry_and_exact_watch_before_source_io() {
    for mode in 0..7 {
        let (mut node, _, watch, clock) = setup();
        let rx = enqueue(&mut node, &watch);
        assert_eq!(node.registry_pages.pending.len(), 1);
        match mode {
            0 => {
                clock.advance_ms(QUEUE_TTL_MS);
            }
            1 => {
                node.watch_registry(watch.bucket, watch.doc_id);
            }
            2 => {
                node.unwatch_registry(&watch).unwrap();
            }
            3 => {
                node.device = MlsDevice::generate().unwrap();
            }
            4 => {
                node.group
                    .add_member(
                        &node.device,
                        MlsDevice::generate().unwrap().key_package().unwrap(),
                    )
                    .unwrap();
            }
            5 => {
                clock.set_wall_ms(1000 + MAX_REQUEST_AGE_MS + 1);
            }
            6 => {
                node.registry_pages.pending[0].key =
                    MlsDevice::generate().unwrap().public_key_bytes();
            }
            _ => unreachable!(),
        }
        let _: Result<Option<Result<(), ()>>, _> = node
            .serve_registry_request(&watch, |_, _, _, _| {
                panic!("stale request must not read source")
            });
        assert!(rx.recv().await.is_none());
    }
}

#[tokio::test]
async fn registry_page_request_rates_bind_full_identity_and_survive_watch_replacement() {
    let (mut node, _, mut watch, clock) = setup();
    for _ in 0..2 {
        let _rx = enqueue(&mut node, &watch);
        assert_eq!(node.registry_pages.pending.len(), 1);
        let duplicate = enqueue(&mut node, &watch);
        assert!(duplicate.recv().await.is_none());
        assert_eq!(node.registry_pages.pending.len(), 1);
        watch = node.watch_registry(watch.bucket, watch.doc_id);
    }
    assert!(enqueue(&mut node, &watch).recv().await.is_none());
    clock.set_wall_ms(999_000);
    assert!(enqueue(&mut node, &watch).recv().await.is_none());
    clock.advance_ms(999);
    assert!(enqueue(&mut node, &watch).recv().await.is_none());
    clock.advance_ms(1);
    let rx = enqueue(&mut node, &watch);
    assert!(node
        .serve_registry_request(&watch, |_, _, _, _| Ok::<_, ()>(
            RegistryPageOutcome::Restart
        ))
        .unwrap()
        .unwrap()
        .is_ok());
    let bytes = rx.recv().await.unwrap();
    let (_, _, answer) = decode_response(&bytes).unwrap();
    assert!(matches!(
        decode_answer(answer, watch.doc_id, node.group.epoch()).unwrap(),
        RegistryPageOutcome::Restart
    ));
    assert!(node.docs.is_empty());
    assert_eq!(node.stats.ops_ingested, 0);
}

#[tokio::test]
async fn registry_page_request_authentication_binds_peer_group_kind_and_current_epoch() {
    for mode in 0..6 {
        let (mut node, _, watch, _) = setup();
        let inner = encode_query(&query(&watch)).unwrap();
        let (mut bytes, _) = node
            .build_authed_request(
                if mode == 1 {
                    KIND_CATCHUP_SINCE
                } else {
                    KIND_REGISTRY_PAGE
                },
                &inner,
            )
            .unwrap();
        let from = if mode == 0 {
            PeerId::from_u64(999)
        } else {
            node.transport.local_peer()
        };
        match mode {
            2 => {
                *bytes.last_mut().unwrap() ^= 1;
            }
            3 => {
                node.group
                    .add_member(
                        &node.device,
                        MlsDevice::generate().unwrap().key_package().unwrap(),
                    )
                    .unwrap();
            }
            4 => {
                node.group = ServerGroup::create(&node.device).unwrap();
            }
            5 => {
                bytes.resize(MAX_REQUEST + 2, 0);
            }
            _ => {}
        }
        let (tx, rx) = Responder::channel();
        node.queue_registry_page_request(from, &bytes[1..], tx);
        assert!(node.registry_pages.pending.is_empty(), "mode {mode}");
        assert!(rx.recv().await.is_none());
    }
}

#[derive(Clone)]
struct DelayedDriver {
    inner: MemNetwork,
    retained: Arc<std::sync::Mutex<Vec<RequestCancellation>>>,
}

#[tokio::test]
async fn registry_page_client_rejects_rebound_and_malformed_signed_responses() {
    let (mut original, _, watch, _) = setup();
    let other = MlsDevice::generate().unwrap();
    // Snapshot fixtures must observe the same applied transition as the live MLS paths.
    original
        .with_observed_mls_transition(|node| {
            node.group
                .add_member(&node.device, other.key_package().unwrap())
        })
        .unwrap();
    let snapshot = original.snapshot().unwrap();
    for mode in 0..12 {
        let hub = Hub::new();
        let peer = PeerId::from_u64(2);
        let remote = hub.join(peer);
        let mut client = ChannelSync::restore(
            &snapshot,
            hub.join(PeerId::from_u64(1)),
            ChaCha20Rng::seed_from_u64(982),
            Box::new(ManualClock::new(1000)),
        )
        .unwrap();
        client.promote_member_peer_bound(peer, original.device.device_id(), true);
        let (result, ()) = tokio::join!(client.request_registry_page(peer, query(&watch)), async {
            let Some(TransportEvent::Request {
                data, responder, ..
            }) = remote.next_event().await
            else {
                panic!("request")
            };
            assert_eq!(data[0], KIND_REGISTRY_PAGE);
            let (mut inner, key, ts, mut nonce, mut epoch, _) =
                decode_authed_request(&data[1..]).unwrap();
            let mut answer = vec![1, 1]; // canonical Restart
            let mut group = original.group.group_id();
            let mut provider = peer;
            match mode {
                1 => inner[1] ^= 1,
                2 => nonce[0] ^= 1,
                3 => epoch += 1,
                4 => provider = PeerId::from_u64(3),
                5 => group[0] ^= 1,
                6 => answer.push(0),
                7 => {
                    responder.respond(Bytes::new());
                    return;
                }
                8 => {
                    responder.respond(Bytes::from(vec![0; MAX_RESPONSE + 1]));
                    return;
                }
                9 => {
                    // signed empty nonterminal page must not create a continuation loop
                    answer = vec![1, 0, 0, 0, 0, 0, 81];
                    answer.extend_from_slice(&[1; 81]);
                }
                _ => {}
            }
            let transcript = response_transcript(
                &group,
                &key,
                &RequestAuth { ts, nonce, epoch },
                provider,
                &inner,
                &answer,
            );
            // A valid CURRENT member signature is still wrong if it is not the full identity
            // already bound to this endpoint. This pins the expected-provider comparison.
            let signer = if mode == 11 { &other } else { &original.device };
            let mut signature = signer.sign(&transcript).unwrap();
            if mode == 10 {
                signature[0] ^= 1;
            }
            responder.respond(Bytes::from(encode_signed_commit_resp(
                &signer.public_key_bytes(),
                &signature,
                &answer,
            )));
        });
        match mode {
            0 => assert!(matches!(
                result.unwrap(),
                Some(RegistryPageOutcome::Restart)
            )),
            7 => assert!(result.unwrap().is_none()),
            _ => assert!(result.is_err(), "mode {mode}"),
        }
        assert!(client.docs.is_empty());
        assert_eq!(client.stats.ops_ingested, 0);
    }
}
#[async_trait::async_trait]
impl MeshTransport for DelayedDriver {
    async fn request_connected_cancellable(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.request_cancellable(peer, proto, data, cancellation)
            .await
    }
    fn local_peer(&self) -> PeerId {
        self.inner.local_peer()
    }
    async fn subscribe(&self, topic: Topic) -> Result<(), TransportError> {
        self.inner.subscribe(topic).await
    }
    async fn unsubscribe(&self, topic: Topic) -> Result<(), TransportError> {
        self.inner.unsubscribe(topic).await
    }
    async fn publish(&self, _: Topic, _: Bytes) -> Result<(), TransportError> {
        panic!("no publish")
    }
    async fn request(&self, _: PeerId, _: ProtocolId, _: Bytes) -> Result<Bytes, TransportError> {
        panic!("no uncancellable request")
    }
    async fn next_event(&self) -> Option<TransportEvent> {
        self.inner.next_event().await
    }
    async fn request_cancellable(
        &self,
        _: PeerId,
        _: ProtocolId,
        _: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.retained.lock().unwrap().push(cancellation);
        futures::future::pending().await
    }
}

#[test]
fn registry_page_client_cancellation_keeps_driver_permits_and_never_sends_to_candidates() {
    let (mut original, _, watch, clock) = setup();
    let transport = DelayedDriver {
        inner: Hub::new().join(PeerId::from_u64(1)),
        retained: Arc::default(),
    };
    let mut node = ChannelSync::restore(
        &original.snapshot().unwrap(),
        transport.clone(),
        ChaCha20Rng::seed_from_u64(981),
        Box::new(clock.clone()),
    )
    .unwrap();
    let peer = PeerId::from_u64(2);
    let poll = |future: std::pin::Pin<&mut dyn Future<Output = _>>| {
        future.poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
    };
    let mut future = Box::pin(node.request_registry_page(peer, query(&watch)));
    assert!(matches!(
        poll(future.as_mut()),
        std::task::Poll::Ready(Err(SyncError::Unauthorized))
    ));
    drop(future);
    assert!(transport.retained.lock().unwrap().is_empty());
    for (device, bound) in [
        (node.device.device_id(), false),
        (MlsDevice::generate().unwrap().device_id(), true),
    ] {
        // Inject a cached stale proof for the latter case: ordinary Remove proactively purges
        // it, but the send boundary must reject it even if a future cache bug leaves it behind.
        node.member_peers.clear();
        node.member_peers.push_back(ProvenMemberPeer {
            peer,
            device,
            bound,
        });
        let mut future = Box::pin(node.request_registry_page(peer, query(&watch)));
        assert!(matches!(
            poll(future.as_mut()),
            std::task::Poll::Ready(Err(SyncError::Unauthorized))
        ));
        drop(future);
        assert!(transport.retained.lock().unwrap().is_empty());
    }
    node.member_peers.clear();
    node.promote_member_peer_bound(peer, node.device.device_id(), true);
    for i in 0..4 {
        let mut future = Box::pin(node.request_registry_page(peer, query(&watch)));
        assert!(poll(future.as_mut()).is_pending());
        if i == 0 {
            clock.advance_ms(REQUEST_MS);
            assert!(matches!(
                poll(future.as_mut()),
                std::task::Poll::Ready(Err(_))
            ));
        }
        drop(future);
        let retained = transport.retained.lock().unwrap();
        assert_eq!(retained.len(), i + 1);
        assert!(retained.iter().all(RequestCancellation::is_cancelled));
    }
    let mut future = Box::pin(node.request_registry_page(peer, query(&watch)));
    assert!(matches!(
        poll(future.as_mut()),
        std::task::Poll::Ready(Err(_))
    ));
    drop(future);
    assert_eq!(transport.retained.lock().unwrap().len(), 4);
    transport.retained.lock().unwrap().clear();
    let mut future = Box::pin(node.request_registry_page(peer, query(&watch)));
    assert!(poll(future.as_mut()).is_pending());
    drop(future);
    assert_eq!(transport.retained.lock().unwrap().len(), 1);
}
