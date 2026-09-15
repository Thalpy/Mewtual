use super::*;
use catcoms_replication::SignedOp;
use catcoms_replication::{
    registry::{PointerKey, RegistryOp},
    registry_epoch::RegistryEpoch,
};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn setup() -> (
    ChannelSync<MemNetwork, ChaCha20Rng>,
    RegistryEpoch,
    SealedOp,
    u8,
    ManualClock,
) {
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let clock = ManualClock::new(0);
    let mut rng = ChaCha20Rng::seed_from_u64(830);
    let key = PointerKey::new(DocType::StudioObject, b"private-registry-cat".to_vec()).unwrap();
    let bucket = key.bucket();
    let op = RegistryOp::Put { key, epoch: 0 }
        .domain_op(&group.group_id(), [1; 16])
        .unwrap();
    let mut source = RegistryEpoch::new(&group, bucket, device.device_id()).unwrap();
    let sealed = source.edit(&device, &group, &mut rng, &op).unwrap();
    let node = ChannelSync::new(
        Hub::new().join(PeerId::from_u64(1)),
        group,
        device,
        rng,
        Box::new(clock.clone()),
    );
    (node, source, sealed, bucket, clock)
}

#[test]
fn registry_inbox_rejects_unwatched_wrong_topic_scope_auth_and_stale_queued_work() {
    let (mut node, source, sealed, bucket, _) = setup();
    let topic = node
        .channel_topic_for(DocType::DocRegistry, source.doc_id(), 0)
        .unwrap();
    assert!(node.on_registry_gossip(&topic, &sealed.encode()));
    assert!(node.registry_ingress.queue.is_empty());
    let watch = node.watch_registry(bucket, source.doc_id());
    assert_eq!(format!("{watch:?}"), "RegistryWatch { .. }");
    node.on_registry_gossip(&Topic::new(b"wrong".to_vec()), &sealed.encode());
    assert!(node.registry_ingress.queue.is_empty());
    for case in 0..5 {
        let mut wrong = sealed.clone();
        match case {
            0 => wrong.epoch += 1,
            1 => wrong.doc_id ^= 1,
            2 => wrong.blob.ciphertext[0] ^= 1,
            3 => wrong.blob.nonce[0] ^= 1,
            4 => wrong.blob.ciphertext = vec![0; MAX_PACKET],
            _ => unreachable!(),
        }
        node.on_registry_gossip(&topic, &wrong.encode());
        assert!(node.registry_ingress.queue.is_empty());
    }
    node.on_registry_gossip(&topic, &sealed.encode());
    assert_eq!(node.registry_ingress.queue.len(), 1);
    node.device = MlsDevice::generate().unwrap();
    assert!(matches!(
        node.drain_registry_inbound(&watch, |_, _, _, _| panic!("no unauthorized I/O")),
        Err(SyncError::Unauthorized)
    ));
    assert!(node.registry_ingress.queue.is_empty());
    assert!(node.docs.is_empty());
    assert_eq!(node.stats.ops_ingested, 0);
}

#[test]
fn registry_inbox_queue_and_rate_debt_stay_bounded_across_rewatch_and_clock_changes() {
    let (mut node, source, sealed, bucket, clock) = setup();
    let topic = node
        .channel_topic_for(DocType::DocRegistry, source.doc_id(), 0)
        .unwrap();
    let mut watch = node.watch_registry(bucket, source.doc_id());
    for _ in 0..100 {
        node.on_registry_gossip(&topic, &sealed.encode());
    }
    assert_eq!(node.registry_ingress.queue.len(), MAX_QUEUE);
    assert!(
        node.registry_ingress
            .queue
            .iter()
            .map(|item| item.sealed.blob.ciphertext.len())
            .sum::<usize>()
            <= MAX_QUEUE * (MAX_SIGNED_EPOCH_OP_BYTES + 20)
    );
    for _ in 0..MAX_QUEUE {
        node.drain_registry_inbound(&watch, |_, _, _, _| ())
            .unwrap()
            .unwrap();
    }
    // Charge the rest of this full author's burst, replacing its watch on every frame. No reset.
    for _ in MAX_QUEUE..50 {
        watch = node.watch_registry(bucket, source.doc_id());
        node.on_registry_gossip(&topic, &sealed.encode());
        node.drain_registry_inbound(&watch, |_, _, _, _| ())
            .unwrap()
            .unwrap();
    }
    node.on_registry_gossip(&topic, &sealed.encode());
    assert!(node.registry_ingress.queue.is_empty());
    clock.set_wall_ms(999_999);
    node.on_registry_gossip(&topic, &sealed.encode());
    assert!(
        node.registry_ingress.queue.is_empty(),
        "wall time is not refill time"
    );
    clock.advance_ms(99);
    node.on_registry_gossip(&topic, &sealed.encode());
    assert!(node.registry_ingress.queue.is_empty());
    clock.advance_ms(1);
    node.on_registry_gossip(&topic, &sealed.encode());
    assert_eq!(node.registry_ingress.queue.len(), 1);
    let replacement = node.watch_registry(bucket, source.doc_id());
    assert!(node.registry_ingress.queue.is_empty());
    assert!(node
        .drain_registry_inbound(&watch, |_, _, _, _| ())
        .is_err());
    assert!(node.unwatch_registry(&watch).is_err());
    node.unwatch_registry(&replacement).unwrap();
    assert!(node.registry_ingress.watches.is_empty());
}

#[test]
fn registry_inbox_preauth_and_limiter_table_have_hard_caps_and_refill_without_wrap() {
    let (mut node, source, sealed, bucket, clock) = setup();
    let topic = node
        .channel_topic_for(DocType::DocRegistry, source.doc_id(), 0)
        .unwrap();
    let watch = node.watch_registry(bucket, source.doc_id());
    let mut corrupt = sealed.clone();
    corrupt.blob.ciphertext[0] ^= 1;
    for _ in 0..200 {
        node.on_registry_gossip(&topic, &corrupt.encode());
    }
    assert!(
        node.registry_ingress.authors.is_empty(),
        "unverified identities get no rows"
    );
    node.on_registry_gossip(&topic, &sealed.encode());
    assert!(node.registry_ingress.queue.is_empty());
    clock.advance_ms(20);
    node.on_registry_gossip(&topic, &sealed.encode());
    node.drain_registry_inbound(&watch, |_, _, _, _| ())
        .unwrap()
        .unwrap();
    node.registry_ingress.authors.clear();
    for i in 0..MAX_RATE_ROWS {
        node.registry_ingress.authors.insert(
            (
                node.device.device_id(),
                source.doc_id().wrapping_add(1 + i as u128),
            ),
            Rate {
                units: 0,
                at_ms: 20,
            },
        );
    }
    clock.advance_ms(20);
    node.on_registry_gossip(&topic, &sealed.encode());
    assert!(node.registry_ingress.queue.is_empty());
    assert_eq!(node.registry_ingress.authors.len(), MAX_RATE_ROWS);
    clock.advance_ms(5_000);
    node.on_registry_gossip(&topic, &sealed.encode());
    assert_eq!(node.registry_ingress.authors.len(), 1);
    let mut rate = Rate {
        units: 0,
        at_ms: 100,
    };
    assert!(!rate.charge(99, 10, 50));
    assert_eq!(rate.at_ms, 100);
    assert!(rate.charge(u64::MAX, 10, 50));
    assert_eq!(rate.units, 49_000);
}

#[test]
fn registry_inbox_checks_full_signed_author_bucket_and_membership_after_queueing() {
    let (mut node, mut source, sealed, bucket, _) = setup();
    let watch = node.watch_registry(bucket, source.doc_id());
    let topic = node
        .channel_topic_for(DocType::DocRegistry, source.doc_id(), 0)
        .unwrap();
    let key = node
        .group
        .channel_secret(&node.device, DocType::DocRegistry, source.doc_id())
        .unwrap();
    let signed = sealed.open(&key).unwrap();
    let domain = DomainOp::decode(signed.domain_op.as_ref().unwrap()).unwrap();
    let foreign = MlsDevice::generate().unwrap();
    let bad = SignedOp::sign_domain(
        &foreign,
        DocType::DocRegistry,
        source.doc_id(),
        signed.delta.clone(),
        &domain,
    )
    .unwrap();
    let packet = SealedOp::seal(&bad, &node.group, &node.device, &mut node.rng).unwrap();
    node.on_registry_gossip(&topic, &packet.encode());
    assert!(node.registry_ingress.queue.is_empty());
    assert!(node.registry_ingress.authors.is_empty());
    // Even a valid local signature cannot publish another bucket's domain on this watch.
    let other_key = (0..1000)
        .map(|n| PointerKey::new(DocType::StudioObject, format!("other-{n}").into_bytes()).unwrap())
        .find(|key| key.bucket() != bucket)
        .unwrap();
    let other_domain = RegistryOp::Put {
        key: other_key,
        epoch: 0,
    }
    .domain_op(&node.group.group_id(), [2; 16])
    .unwrap();
    let bad = SignedOp::sign_domain(
        &node.device,
        DocType::DocRegistry,
        source.doc_id(),
        signed.delta,
        &other_domain,
    )
    .unwrap();
    let packet = SealedOp::seal(&bad, &node.group, &node.device, &mut node.rng).unwrap();
    node.on_registry_gossip(&topic, &packet.encode());
    assert!(node.registry_ingress.queue.is_empty());
    node.on_registry_gossip(&topic, &sealed.encode());
    node.group
        .add_member(&node.device, foreign.key_package().unwrap())
        .unwrap();
    assert!(matches!(
        node.drain_registry_inbound(&watch, |_, _, _, _| panic!("old MLS packet")),
        Err(SyncError::Malformed)
    ));
    // A current freshly sealed retry is accepted; no past-key or generic catch-up fallback.
    let fresh = source
        .edit_or_reseal(&node.device, &node.group, &mut node.rng, &domain)
        .unwrap();
    node.on_registry_gossip(&topic, &fresh.encode());
    assert!(node
        .drain_registry_inbound(&watch, |_, _, _, _| ())
        .unwrap()
        .is_some());
}

#[derive(Clone)]
struct InterruptSubscribe {
    inner: MemNetwork,
    mode: Arc<std::sync::atomic::AtomicU8>,
    subscriptions: Arc<std::sync::Mutex<HashSet<Topic>>>,
}
#[async_trait::async_trait]
impl MeshTransport for InterruptSubscribe {
    fn local_peer(&self) -> PeerId {
        self.inner.local_peer()
    }
    async fn subscribe(&self, topic: Topic) -> Result<(), catcoms_rt::TransportError> {
        self.inner.subscribe(topic.clone()).await?;
        self.subscriptions.lock().unwrap().insert(topic);
        match self.mode.load(std::sync::atomic::Ordering::SeqCst) {
            1 => futures::future::pending().await,
            2 => Err(catcoms_rt::TransportError::Closed),
            _ => Ok(()),
        }
    }
    async fn unsubscribe(&self, topic: Topic) -> Result<(), catcoms_rt::TransportError> {
        self.inner.unsubscribe(topic.clone()).await?;
        self.subscriptions.lock().unwrap().remove(&topic);
        match self.mode.load(std::sync::atomic::Ordering::SeqCst) {
            3 => futures::future::pending().await,
            4 => Err(catcoms_rt::TransportError::Closed),
            _ => Ok(()),
        }
    }
    async fn publish(&self, topic: Topic, bytes: Bytes) -> Result<(), catcoms_rt::TransportError> {
        self.inner.publish(topic, bytes).await
    }
    async fn request(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        bytes: Bytes,
    ) -> Result<Bytes, catcoms_rt::TransportError> {
        self.inner.request(peer, protocol, bytes).await
    }
    async fn next_event(&self) -> Option<TransportEvent> {
        self.inner.next_event().await
    }
    async fn request_cancellable(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        bytes: Bytes,
        cancellation: catcoms_rt::RequestCancellation,
    ) -> Result<Bytes, catcoms_rt::TransportError> {
        self.inner
            .request_cancellable(peer, protocol, bytes, cancellation)
            .await
    }
}

#[tokio::test]
async fn registry_inbox_cancelled_or_failed_subscribe_retains_cleanup_and_retry_ownership() {
    use std::future::Future;
    use std::sync::atomic::{AtomicU8, Ordering as AtomicOrdering};
    let (mut node, source, _, bucket, _) = setup();
    let transport = InterruptSubscribe {
        inner: Hub::new().join(PeerId::from_u64(2)),
        mode: Arc::new(AtomicU8::new(1)),
        subscriptions: Arc::default(),
    };
    let mut node = ChannelSync::restore(
        &node.snapshot().unwrap(),
        transport.clone(),
        ChaCha20Rng::seed_from_u64(831),
        Box::new(ManualClock::new(0)),
    )
    .unwrap();
    let watch = node.watch_registry(bucket, source.doc_id());
    let mut flush = Box::pin(node.flush_registry_subscriptions());
    assert!(flush
        .as_mut()
        .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
        .is_pending());
    drop(flush);
    assert!(node.needs_resync);
    assert!(node.routing_subscription_pending.is_some());
    assert_eq!(transport.subscriptions.lock().unwrap().len(), 1);
    node.unwatch_registry(&watch).unwrap();
    transport.mode.store(0, AtomicOrdering::SeqCst);
    node.flush_registry_subscriptions().await.unwrap();
    assert!(transport.subscriptions.lock().unwrap().is_empty());
    assert!(node.routing_subscription_pending.is_none());
    let watch = node.watch_registry(bucket, source.doc_id());
    transport.mode.store(2, AtomicOrdering::SeqCst);
    assert!(node.flush_registry_subscriptions().await.is_err());
    assert!(node.needs_resync);
    transport.mode.store(0, AtomicOrdering::SeqCst);
    node.flush_registry_subscriptions().await.unwrap();
    assert!(node.registry_watch_is_current(&watch));
    assert!(!node.needs_resync);
    assert_eq!(transport.subscriptions.lock().unwrap().len(), 1);
    node.routing_label = 1;
    node.routing_secrets.insert(1, Zeroizing::new([22; 32]));
    node.flush_registry_subscriptions().await.unwrap();
    assert_eq!(transport.subscriptions.lock().unwrap().len(), 2);
    node.unwatch_registry(&watch).unwrap();
    node.flush_registry_subscriptions().await.unwrap();
    assert!(transport.subscriptions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn registry_inbox_cancelled_or_failed_unsubscribe_then_rewatch_really_resubscribes() {
    use std::future::Future;
    use std::sync::atomic::{AtomicU8, Ordering as AtomicOrdering};
    for mode in [3, 4] {
        let (mut node, source, _, bucket, _) = setup();
        let transport = InterruptSubscribe {
            inner: Hub::new().join(PeerId::from_u64(2)),
            mode: Arc::new(AtomicU8::new(0)),
            subscriptions: Arc::default(),
        };
        let mut node = ChannelSync::restore(
            &node.snapshot().unwrap(),
            transport.clone(),
            ChaCha20Rng::seed_from_u64(832),
            Box::new(ManualClock::new(0)),
        )
        .unwrap();
        let watch = node.watch_registry(bucket, source.doc_id());
        node.flush_registry_subscriptions().await.unwrap();
        node.unwatch_registry(&watch).unwrap();
        transport.mode.store(mode, AtomicOrdering::SeqCst);
        let mut flush = Box::pin(node.flush_registry_subscriptions());
        let result = flush
            .as_mut()
            .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()));
        if mode == 3 {
            assert!(result.is_pending());
        } else {
            assert!(matches!(result, std::task::Poll::Ready(Err(_))));
        }
        drop(flush);
        assert!(
            transport.subscriptions.lock().unwrap().is_empty(),
            "transport side effect occurred"
        );
        let watch = node.watch_registry(bucket, source.doc_id());
        transport.mode.store(0, AtomicOrdering::SeqCst);
        node.flush_registry_subscriptions().await.unwrap();
        assert!(node.registry_watch_is_current(&watch));
        assert_eq!(
            transport.subscriptions.lock().unwrap().len(),
            1,
            "rewatch must actually subscribe"
        );
    }
}
