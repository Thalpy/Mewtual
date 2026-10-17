use super::*;
use catcoms_replication::studio::{FlipnoteHeader, StudioEpoch};
use catcoms_replication::SignedOp;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId, PublishOnceError};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(896)
}
fn target() -> StudioTarget {
    StudioTarget::Flipnote {
        channel: [1; 16],
        object: [2; 16],
    }
}
fn setup() -> (
    ChannelSync<MemNetwork, ChaCha20Rng>,
    MemNetwork,
    StudioEpoch,
    DomainOp,
    SealedOp,
    ManualClock,
) {
    let hub = Hub::new();
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let mut source = StudioEpoch::new(&group, target(), device.device_id()).unwrap();
    let logical = target().document(&group.group_id()).unwrap();
    let domain = DomainOp {
        nonce: [1; 16],
        doc_type: logical.doc_type,
        logical_key: logical.logical_key,
        body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("private moon".into()))
            .encode()
            .unwrap(),
    };
    let sealed = source
        .edit_or_reseal(&device, &group, &mut rng(), &domain, 10)
        .unwrap();
    let clock = ManualClock::new(10);
    let node = ChannelSync::new(
        hub.join(PeerId::from_u64(1)),
        group,
        device,
        rng(),
        Box::new(clock.clone()),
    );
    (
        node,
        hub.join(PeerId::from_u64(2)),
        source,
        domain,
        sealed,
        clock,
    )
}

#[test]
fn studio_inbox_has_logical_watch_queue_and_rate_caps_without_legacy_fallback() {
    let (mut node, _, source, _, sealed, clock) = setup();
    let topic = node
        .channel_topic_for(sealed.doc_type, sealed.doc_id, 0)
        .unwrap();
    assert!(node.on_studio_gossip(&topic, &sealed.encode()));
    assert!(node.studio_exchange.queue.is_empty());
    assert!(node.on_studio_gossip(&topic, &DocType::StudioIndex.tag().to_be_bytes()));
    let mut watch = node.watch_studio(target(), source.doc_id()).unwrap();
    for _ in 0..100 {
        assert!(node.on_studio_gossip(&topic, &sealed.encode()));
    }
    assert_eq!(node.studio_exchange.queue.len(), MAX_QUEUE);
    for _ in 0..MAX_QUEUE {
        node.drain_studio_inbound(&watch, |_, _, _, _| ())
            .unwrap()
            .unwrap();
    }
    for _ in MAX_QUEUE..50 {
        watch = node.watch_studio(target(), source.doc_id()).unwrap();
        node.on_studio_gossip(&topic, &sealed.encode());
        node.drain_studio_inbound(&watch, |_, _, _, _| ())
            .unwrap()
            .unwrap();
    }
    node.on_studio_gossip(&topic, &sealed.encode());
    assert!(node.studio_exchange.queue.is_empty());
    clock.set_wall_ms(999_999);
    clock.advance_ms(99);
    node.on_studio_gossip(&topic, &sealed.encode());
    assert!(node.studio_exchange.queue.is_empty());
    clock.advance_ms(1);
    node.on_studio_gossip(&topic, &sealed.encode());
    assert_eq!(node.studio_exchange.queue.len(), 1);
    // Same object under another channel is a replacement, never another watch/rate slot.
    let other_channel = StudioTarget::Flipnote {
        channel: [9; 16],
        object: [2; 16],
    };
    let replacement = node.watch_studio(other_channel, source.doc_id()).unwrap();
    assert_eq!(node.studio_exchange.watches.len(), 1);
    assert!(node.studio_exchange.queue.is_empty());
    assert!(node.unwatch_studio(&watch).is_err());
    assert!(node
        .drain_studio_inbound(&watch, |_, _, _, _| panic!())
        .is_err());
    for n in 0..15 {
        node.watch_studio(StudioTarget::Index { channel: [n; 16] }, n as u128)
            .unwrap();
    }
    assert!(node
        .watch_studio(StudioTarget::Index { channel: [99; 16] }, 99)
        .is_err());
    assert_eq!(node.studio_exchange.watches.len(), MAX_WATCHES);
    node.unwatch_studio(&replacement).unwrap();
    assert!(node.docs.is_empty());
    assert!(node.outbox.is_empty());
}

#[test]
fn studio_inbox_preauth_and_author_rows_refuse_without_forgiving_debt() {
    let (mut node, _, source, _, sealed, clock) = setup();
    let topic = node
        .channel_topic_for(sealed.doc_type, sealed.doc_id, 0)
        .unwrap();
    let watch = node.watch_studio(target(), source.doc_id()).unwrap();
    let mut corrupt = sealed.clone();
    corrupt.blob.ciphertext[0] ^= 1;
    for _ in 0..200 {
        node.on_studio_gossip(&topic, &corrupt.encode());
    }
    assert!(node.studio_exchange.authors.is_empty());
    node.on_studio_gossip(&topic, &sealed.encode());
    assert!(node.studio_exchange.queue.is_empty());
    clock.advance_ms(20);
    node.on_studio_gossip(&topic, &sealed.encode());
    node.drain_studio_inbound(&watch, |_, _, _, _| ())
        .unwrap()
        .unwrap();
    node.studio_exchange.authors.clear();
    for i in 0..MAX_RATE_ROWS {
        let mut debt = Rate::full(20, 1);
        assert!(debt.charge(20, 10, 1));
        node.studio_exchange.authors.insert(
            (
                node.device.device_id(),
                DocType::StudioIndex,
                (i as u128).to_be_bytes(),
            ),
            debt,
        );
    }
    clock.advance_ms(20);
    node.on_studio_gossip(&topic, &sealed.encode());
    assert!(node.studio_exchange.queue.is_empty());
    assert_eq!(node.studio_exchange.authors.len(), MAX_RATE_ROWS);
    clock.advance_ms(5000);
    node.on_studio_gossip(&topic, &sealed.encode());
    assert_eq!(node.studio_exchange.authors.len(), 1);
    assert_eq!(node.studio_exchange.queue.len(), 1);
}

#[test]
fn studio_inbox_rechecks_mls_full_author_logical_scope_and_topic() {
    let (mut node, _, mut source, domain, sealed, _) = setup();
    let topic = node
        .channel_topic_for(sealed.doc_type, sealed.doc_id, 0)
        .unwrap();
    let watch = node.watch_studio(target(), source.doc_id()).unwrap();
    let secret = node
        .group
        .channel_secret(&node.device, sealed.doc_type, sealed.doc_id)
        .unwrap();
    let signed = sealed.open(&secret).unwrap();
    let stranger = MlsDevice::generate().unwrap();
    let foreign = SignedOp::sign_domain(
        &stranger,
        sealed.doc_type,
        sealed.doc_id,
        signed.delta.clone(),
        &domain,
    )
    .unwrap();
    let bad = SealedOp::seal(&foreign, &node.group, &node.device, &mut rng()).unwrap();
    node.on_studio_gossip(&topic, &bad.encode());
    assert!(node.studio_exchange.queue.is_empty());
    let mut wrong_domain = domain.clone();
    wrong_domain.logical_key[0] ^= 1;
    let wrong = SignedOp::sign_domain(
        &node.device,
        sealed.doc_type,
        sealed.doc_id,
        signed.delta,
        &wrong_domain,
    )
    .unwrap();
    let bad = SealedOp::seal(&wrong, &node.group, &node.device, &mut rng()).unwrap();
    node.on_studio_gossip(&topic, &bad.encode());
    let wrong_topic = node
        .channel_topic_for(sealed.doc_type, sealed.doc_id + 1, 0)
        .unwrap();
    node.on_studio_gossip(&wrong_topic, &sealed.encode());
    assert!(node.studio_exchange.queue.is_empty());
    node.on_studio_gossip(&topic, &sealed.encode());
    node.group
        .add_member(&node.device, stranger.key_package().unwrap())
        .unwrap();
    assert!(node
        .drain_studio_inbound(&watch, |_, _, _, _| panic!("old MLS"))
        .is_err());
    let fresh = source
        .edit_or_reseal(&node.device, &node.group, &mut rng(), &domain, 10)
        .unwrap();
    node.on_studio_gossip(&topic, &fresh.encode());
    node.device = MlsDevice::generate().unwrap();
    assert!(node
        .drain_studio_inbound(&watch, |_, _, _, _| panic!("removed receiver"))
        .is_err());
}

#[test]
fn studio_receive_precheck_preserves_valid_queue_and_consumes_stale_mls() {
    let (mut node, _, source, _, sealed, _) = setup();
    let topic = node
        .channel_topic_for(sealed.doc_type, sealed.doc_id, 0)
        .unwrap();
    let watch = node.watch_studio(target(), source.doc_id()).unwrap();
    assert!(!node.check_studio_inbound(&watch).unwrap());
    node.on_studio_gossip(&topic, &sealed.encode());
    assert!(node.studio_has_inbound(&watch));
    assert!(node.check_studio_inbound(&watch).unwrap());
    assert_eq!(node.studio_exchange.queue.len(), 1);
    let stranger = MlsDevice::generate().unwrap();
    node.group
        .add_member(&node.device, stranger.key_package().unwrap())
        .unwrap();
    assert!(node.check_studio_inbound(&watch).is_err());
    assert!(!node.studio_has_inbound(&watch));
    assert!(node.studio_exchange.queue.is_empty());
    node.unwatch_studio(&watch).unwrap();
    assert!(node.check_studio_inbound(&watch).is_err());
}

#[tokio::test]
async fn studio_sender_rejects_wrong_packets_and_uses_one_shot_current_route() {
    let (mut node, observer, source, _, sealed, _) = setup();
    let topic = node
        .channel_topic_for(sealed.doc_type, sealed.doc_id, 0)
        .unwrap();
    observer.subscribe(topic.clone()).await.unwrap();
    for n in 0..6 {
        let mut wrong = sealed.clone();
        match n {
            0 => wrong.doc_type = DocType::Channel,
            1 => wrong.doc_id ^= 1,
            2 => wrong.epoch += 1,
            3 => wrong.blob.nonce[0] ^= 1,
            4 => wrong.blob.ciphertext[0] ^= 1,
            _ => wrong.blob.ciphertext = vec![0; MAX_SIGNED_EPOCH_OP_BYTES + 21],
        }
        assert!(node
            .publish_local_studio_once(target(), source.doc_id(), wrong)
            .await
            .is_err());
    }
    let mut pending = observer.next_event();
    assert!(std::future::Future::poll(
        pending.as_mut(),
        &mut std::task::Context::from_waker(std::task::Waker::noop())
    )
    .is_pending());
    drop(pending);
    assert_eq!(
        node.publish_local_studio_once(target(), source.doc_id(), sealed.clone())
            .await
            .unwrap(),
        PublishSubmission::Submitted
    );
    let TransportEvent::Gossip {
        topic: got, data, ..
    } = observer.next_event().await.unwrap()
    else {
        panic!()
    };
    assert_eq!(got, topic);
    assert_eq!(data.as_ref(), sealed.encode());
    observer.unsubscribe(topic).await.unwrap();
    assert!(matches!(
        node.publish_local_studio_once(target(), source.doc_id(), sealed)
            .await,
        Err(SyncError::Publication(PublishOnceError::NoPeers))
    ));
    assert!(node.docs.is_empty());
    assert!(node.outbox.is_empty());
}

#[tokio::test]
async fn studio_watch_reconciles_unwatch_and_rejects_restored_incarnation() {
    let (mut node, observer, source, _, sealed, _) = setup();
    let watch = node.watch_studio(target(), source.doc_id()).unwrap();
    node.flush_studio_subscriptions().await.unwrap();
    let topic = node
        .channel_topic_for(sealed.doc_type, sealed.doc_id, 0)
        .unwrap();
    observer
        .publish_once(topic.clone(), Bytes::from(sealed.encode()))
        .await
        .unwrap();
    node.run_once().await.unwrap();
    assert_eq!(node.studio_exchange.queue.len(), 1);
    node.unwatch_studio(&watch).unwrap();
    node.flush_studio_subscriptions().await.unwrap();
    assert!(node.studio_exchange.queue.is_empty());
    assert!(matches!(
        observer
            .publish_once(topic, Bytes::from(sealed.encode()))
            .await,
        Err(PublishOnceError::NoPeers)
    ));
    let snapshot = node.snapshot().unwrap();
    let restored = ChannelSync::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(7)),
        rng(),
        Box::new(ManualClock::new(10)),
    )
    .unwrap();
    assert!(!restored.studio_watch_is_current(&watch));
    assert!(restored.studio_exchange.watches.is_empty());
}

#[tokio::test]
async fn studio_sender_refuses_foreign_current_author_and_reseals_on_new_route() {
    let (mut node, observer, mut source, domain, old, _) = setup();
    let other = MlsDevice::generate().unwrap();
    node.group
        .add_member(&node.device, other.key_package().unwrap())
        .unwrap();
    assert!(node
        .publish_local_studio_once(target(), source.doc_id(), old)
        .await
        .is_err());
    let fresh = source
        .edit_or_reseal(&node.device, &node.group, &mut rng(), &domain, 10)
        .unwrap();
    let secret = node
        .group
        .channel_secret(&node.device, fresh.doc_type, fresh.doc_id)
        .unwrap();
    let signed = fresh.open(&secret).unwrap();
    let foreign =
        SignedOp::sign_domain(&other, fresh.doc_type, fresh.doc_id, signed.delta, &domain).unwrap();
    let wrong = SealedOp::seal(&foreign, &node.group, &node.device, &mut rng()).unwrap();
    assert!(matches!(
        node.publish_local_studio_once(target(), source.doc_id(), wrong)
            .await,
        Err(SyncError::Unauthorized)
    ));
    node.routing_label = 1;
    node.routing_secrets.insert(1, Zeroizing::new([19; 32]));
    let topic = node
        .channel_topic_for(fresh.doc_type, fresh.doc_id, 1)
        .unwrap();
    observer.subscribe(topic.clone()).await.unwrap();
    node.publish_local_studio_once(target(), source.doc_id(), fresh)
        .await
        .unwrap();
    assert!(
        matches!(observer.next_event().await.unwrap(), TransportEvent::Gossip { topic: actual, .. } if actual == topic)
    );
    assert!(node.outbox.is_empty());
}
