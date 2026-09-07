use super::*;
use catcoms_replication::registry::PointerKey;
use catcoms_replication::{registry_epoch::RegistryEpoch, Admission, SignedOp};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId, PublishOnceError, TransportEvent};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(811)
}

fn setup() -> (
    ChannelSync<MemNetwork, ChaCha20Rng>,
    MemNetwork,
    RegistryEpoch,
    DomainOp,
    SealedOp,
) {
    let hub = Hub::new();
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let key = PointerKey::new(DocType::StudioObject, b"sender-test".to_vec()).unwrap();
    let domain = RegistryOp::Put {
        key: key.clone(),
        epoch: 1,
    }
    .domain_op(&group.group_id(), [1; 16])
    .unwrap();
    let mut source = RegistryEpoch::new(&group, key.bucket(), device.device_id()).unwrap();
    let sealed = source.edit(&device, &group, &mut rng(), &domain).unwrap();
    let node = ChannelSync::new(
        hub.join(PeerId::from_u64(1)),
        group,
        device,
        rng(),
        Box::new(ManualClock::new(0)),
    );
    (node, hub.join(PeerId::from_u64(2)), source, domain, sealed)
}

#[tokio::test]
async fn registry_publication_routes_exact_bytes_and_typed_ingress_without_legacy_docs() {
    let (mut node, observer, source, domain, sealed) = setup();
    let topic = node
        .channel_topic_for(DocType::DocRegistry, source.doc_id(), node.routing_label)
        .unwrap();
    observer.subscribe(topic.clone()).await.unwrap();
    assert_eq!(
        node.publish_local_registry_once(source.doc_id(), sealed.clone())
            .await
            .unwrap(),
        PublishSubmission::Submitted
    );
    let TransportEvent::Gossip {
        data,
        topic: actual,
        ..
    } = observer.next_event().await.unwrap()
    else {
        panic!("gossip")
    };
    assert_eq!(actual, topic);
    assert_eq!(data.as_ref(), sealed.encode());
    let key = RegistryOp::decode(&domain.body).unwrap();
    let RegistryOp::Put { key, .. } = key else {
        unreachable!()
    };
    let mut receiver =
        RegistryEpoch::new(&node.group, key.bucket(), node.device.device_id()).unwrap();
    assert_eq!(
        receiver
            .ingest(&SealedOp::decode(&data).unwrap(), &node.group, &node.device)
            .unwrap(),
        Admission::Accepted
    );
    assert_eq!(receiver.projection().unwrap().pointers[&key], 1);
    assert!(node.docs.is_empty(), "never register P1 with legacy ingest");
    assert!(node.outbox.is_empty());
}

#[tokio::test]
async fn registry_publication_rejects_stale_scope_ciphertext_authorship_and_oversize() {
    let (mut node, observer, mut source, domain, sealed) = setup();
    let doc_id = source.doc_id();
    // A live subscriber makes an accidental send succeed rather than hiding it behind NoPeers.
    // Poll the broker as well: no rejected packet may leave even if dispatch later reports error.
    let topic = node
        .channel_topic_for(DocType::DocRegistry, doc_id, node.routing_label)
        .unwrap();
    observer.subscribe(topic).await.unwrap();
    for change in 0..7 {
        let mut wrong = sealed.clone();
        match change {
            0 => wrong.doc_type = DocType::Channel,
            1 => wrong.doc_id ^= 1,
            2 => wrong.epoch += 1,
            3 => wrong.blob.nonce[0] ^= 1,
            4 => wrong.blob.ciphertext[0] ^= 1,
            5 => wrong.blob.ciphertext = vec![0; MAX_SIGNED_EPOCH_OP_BYTES + 21],
            6 => {
                let other = MlsDevice::generate().unwrap();
                node.group
                    .add_member(&node.device, other.key_package().unwrap())
                    .unwrap();
                let old_key = node
                    .group
                    .channel_secret(&node.device, DocType::DocRegistry, doc_id)
                    .unwrap();
                // A second current member's valid signature is still NOT our local replay.
                let delta = source
                    .edit_or_reseal(&node.device, &node.group, &mut rng(), &domain)
                    .unwrap()
                    .open(&old_key)
                    .unwrap()
                    .delta;
                let foreign =
                    SignedOp::sign_domain(&other, DocType::DocRegistry, doc_id, delta, &domain)
                        .unwrap();
                wrong = SealedOp::seal(&foreign, &node.group, &node.device, &mut rng()).unwrap();
            }
            _ => unreachable!(),
        }
        let error = node
            .publish_local_registry_once(doc_id, wrong)
            .await
            .unwrap_err();
        match change {
            0..=2 | 5 => assert!(matches!(error, SyncError::Malformed)),
            6 => assert!(matches!(error, SyncError::Unauthorized)),
            _ => assert!(!matches!(error, SyncError::Publication(_))),
        }
        let mut event = observer.next_event();
        assert!(std::future::Future::poll(
            event.as_mut(),
            &mut std::task::Context::from_waker(std::task::Waker::noop()),
        )
        .is_pending());
        assert!(node.outbox.is_empty());
    }
    let packet = source
        .edit_or_reseal(&node.device, &node.group, &mut rng(), &domain)
        .unwrap();
    node.device = MlsDevice::generate().unwrap();
    assert!(matches!(
        node.publish_local_registry_once(doc_id, packet).await,
        Err(SyncError::Unauthorized)
    ));
    let mut event = observer.next_event();
    assert!(std::future::Future::poll(
        event.as_mut(),
        &mut std::task::Context::from_waker(std::task::Waker::noop()),
    )
    .is_pending());
}

#[tokio::test]
async fn registry_publication_uses_new_mls_and_routing_state_and_restores_new_instance() {
    let (mut node, observer, mut source, domain, old) = setup();
    let instance = node.registry_instance();
    let doc_id = source.doc_id();
    let other = MlsDevice::generate().unwrap();
    node.group
        .add_member(&node.device, other.key_package().unwrap())
        .unwrap();
    assert!(node.publish_local_registry_once(doc_id, old).await.is_err());
    // Exercise the current label rather than accidentally selecting a grandfathered route.
    node.routing_label = 1;
    node.routing_secrets.insert(1, Zeroizing::new([19; 32]));
    let fresh = node.with_registry_context(|group, device, clock, rng| {
        assert_eq!(clock.monotonic_ms(), 0);
        source.edit_or_reseal(device, group, rng, &domain).unwrap()
    });
    let topic = node
        .channel_topic_for(DocType::DocRegistry, doc_id, 1)
        .unwrap();
    observer.subscribe(topic.clone()).await.unwrap();
    assert_eq!(
        node.publish_local_registry_once(doc_id, fresh)
            .await
            .unwrap(),
        PublishSubmission::Submitted
    );
    assert!(node.matches_registry_instance(&instance));
    assert!(
        matches!(observer.next_event().await.unwrap(), TransportEvent::Gossip { topic: actual, .. } if actual == topic)
    );
    let snapshot = node.snapshot().unwrap();
    let restored = ChannelSync::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(3)),
        rng(),
        Box::new(ManualClock::new(0)),
    )
    .unwrap();
    assert_eq!(restored.group_id(), node.group_id());
    assert_eq!(restored.device_id(), node.device_id());
    assert!(!restored.matches_registry_instance(&instance));
    node.routing_secrets.clear();
    let packet =
        node.with_registry_context(|g, d, _, r| source.edit_or_reseal(d, g, r, &domain).unwrap());
    assert!(matches!(
        node.publish_local_registry_once(doc_id, packet).await,
        Err(SyncError::NoSuchDoc)
    ));
    assert!(node.outbox.is_empty());
    // One-shot refusal remains visible, not converted into legacy enqueue success.
    let (mut alone, _, source, _, packet) = setup();
    assert!(matches!(
        alone
            .publish_local_registry_once(source.doc_id(), packet)
            .await,
        Err(SyncError::Publication(PublishOnceError::NoPeers))
    ));
}
