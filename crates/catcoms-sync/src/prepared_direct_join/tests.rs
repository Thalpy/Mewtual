//! Real selected-KP candidates and in-memory request cancellation. These tests establish no
//! remote durable Add or crash/reopen custody. Authored and UNRUN during compilation pause.
use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork, Responder};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn limits() -> DirectJoinLimits {
    DirectJoinLimits::new(512 * 1024, 512 * 1024, 1024 * 1024).unwrap()
}
fn owner(hub: &std::sync::Arc<Hub>, peer: PeerId) -> ChannelSync<MemNetwork, ChaCha20Rng> {
    ChannelSync::found(
        hub.join(peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(0x5ac1),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap()
}
fn prepare(node: &mut ChannelSync<MemNetwork, ChaCha20Rng>) -> PreparedDirectJoin {
    let invite = node.mint_invite([0x51; 16], 10_000, Vec::new()).unwrap();
    PreparedDirectJoin::prepare(MlsDevice::generate().unwrap(), invite, limits()).unwrap()
}

// A polled MemNetwork request synchronously queues its event before awaiting a response. Poll
// only the finite initial lifecycle prefix; never leave a broken harness blocked in next_event.
async fn queued_request(transport: &MemNetwork) -> (PeerId, Bytes, Responder) {
    for _ in 0..16 {
        let mut event = Box::pin(transport.next_event());
        match futures::poll!(&mut event) {
            std::task::Poll::Ready(Some(TransportEvent::Request {
                from,
                data,
                responder,
                ..
            })) => return (from, data, responder),
            std::task::Poll::Ready(Some(_)) => {}
            _ => panic!("the polled original request must already be queued"),
        }
    }
    panic!("unexpected lifecycle prefix exceeded harness bound");
}

#[test]
fn signed_invalid_routing_consumes_only_candidate_and_original_ready_still_completes() {
    let hub = Hub::new();
    let mut node = owner(&hub, PeerId::from_u64(0x5101));
    let source = prepare(&mut node);
    let ready = node
        .serve_join(PeerId::from_u64(0x5102), &source.request[1..])
        .unwrap();
    let (welcome, _, routing) = decode_join_resp(&ready[1..]).unwrap();
    let invalid_routing = b"signed but not a sealed routing transfer";
    let signature = node
        .device
        .sign(&join_transcript(
            &source.invite.group_id,
            &source.invite.invite_nonce,
            &welcome,
            invalid_routing,
        ))
        .unwrap();
    let bad = encode_join_resp(&welcome, &signature, invalid_routing);
    assert!(
        source.verify_original_ready_body(&bad).is_ok(),
        "an inviter-signed body is DATA, not proof of selected-KP/routing acceptance"
    );
    let master = source.master.clone();
    // Pin the original bug: once SDK selection happened, routing rejection does not leave
    // that SAME mutable provider usable. Merely retrying finish_join on it is insufficient.
    let candidate =
        restore_join_device_provider_bounded(&source.master, limits().provider, limits().backing)
            .unwrap();
    assert!(finish_join(
        &candidate,
        &source.invite,
        &welcome,
        &signature,
        invalid_routing,
        &source.expected
    )
    .is_err());
    let (_, good_signature, _) = decode_join_resp(&ready[1..]).unwrap();
    assert!(finish_join(
        &candidate,
        &source.invite,
        &welcome,
        &good_signature,
        &routing,
        &source.expected
    )
    .is_err());
    assert!(source.finish_authenticated_body(&bad).is_err());
    assert_eq!(source.master.as_slice(), master.as_slice());
    let completed = source.finish_authenticated_body(&ready[1..]).unwrap();
    let (device, group, routing) = completed.into_parts();
    assert_eq!(
        device.public_key_bytes().as_slice(),
        source.signer.as_slice()
    );
    assert_eq!(group.group_id(), source.invite.group_id);
    assert_eq!(
        routing
            .fresh_chat_admission
            .as_ref()
            .unwrap()
            .key_package_reference(),
        Some(source.expected)
    );
    assert_eq!(source.master.as_slice(), master.as_slice());
}

#[tokio::test]
async fn normalized_signed_input_is_exact_data_without_sdk_completion_or_master_mutation() {
    let hub = Hub::new();
    let peer = PeerId::from_u64(0xc301);
    let mut node = owner(&hub, peer);
    let source = prepare(&mut node);
    let ready = node
        .serve_join(PeerId::from_u64(0xc302), &source.request[1..])
        .unwrap();
    let master = source.master.clone();
    let network = hub.join(PeerId::from_u64(0xc302));
    let mut prefix = PreOwnerConnectionHandoff::default();
    let body = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        source.receive_original_ready_body(&network, &mut prefix, peer, Bytes::from(ready.clone())),
    )
    .await
    .expect("inline normalization must complete within the harness bound")
    .unwrap();
    assert_eq!(body, ready[1..]);
    assert!(source.verify_original_ready_body(&body).is_ok());
    assert!(!source.completed);
    assert_eq!(source.master.as_slice(), master.as_slice());
    assert!(
        source.finish_authenticated_body(&body).is_ok(),
        "only later actual SDK checks may complete the private candidate"
    );
    assert!(
        !source.completed,
        "only the actual handoff marks legacy completion"
    );
}

#[test]
fn signed_old_welcome_cannot_select_an_older_private_package_retained_in_the_master() {
    let hub = Hub::new();
    let mut node = owner(&hub, PeerId::from_u64(0x5103));
    let invite = node.mint_invite([0x53; 16], 10_000, Vec::new()).unwrap();
    let device = MlsDevice::generate().unwrap();
    let old_package = device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let old_reference = device.key_package_reference(&old_package).unwrap();
    let old_wire = serialize_key_package(&old_package).unwrap();
    let source = PreparedDirectJoin::prepare(device, invite, limits()).unwrap();
    assert_ne!(source.expected, old_reference);
    let (welcome, routing, signature) = node.admit_now(&source.invite, &old_wire, 1_000).unwrap();
    let candidate =
        restore_join_device_provider_bounded(&source.master, limits().provider, limits().backing)
            .unwrap();
    assert!(ServerGroup::join_for_key_package(&candidate, &welcome, &old_reference).is_ok());
    let master = source.master.clone();
    assert!(source
        .finish_authenticated_body(&encode_join_resp(&welcome, &signature, &routing))
        .is_err());
    assert_eq!(source.master.as_slice(), master.as_slice());
}

#[test]
fn forged_truncated_oversized_and_signed_invalid_welcome_leave_master_request_unchanged() {
    let hub = Hub::new();
    let mut node = owner(&hub, PeerId::from_u64(0x5104));
    let source = prepare(&mut node);
    let ready = node
        .serve_join(PeerId::from_u64(0x5105), &source.request[1..])
        .unwrap();
    let (welcome, mut signature, routing) = decode_join_resp(&ready[1..]).unwrap();
    let master = source.master.clone();
    let request = source.request.clone();
    signature[0] ^= 1;
    assert!(source
        .finish_authenticated_body(&encode_join_resp(&welcome, &signature, &routing))
        .is_err());
    for length in [0, 1, 4, ready.len() - 2] {
        assert!(source
            .finish_authenticated_body(&ready[1..][..length])
            .is_err());
    }
    assert!(source
        .finish_authenticated_body(&vec![0; limits().exchange + 1])
        .is_err());
    let broken_welcome = b"actual inviter signed malformed MLS";
    let signature = node
        .device
        .sign(&join_transcript(
            &source.invite.group_id,
            &source.invite.invite_nonce,
            broken_welcome,
            &routing,
        ))
        .unwrap();
    assert!(source
        .finish_authenticated_body(&encode_join_resp(broken_welcome, &signature, &routing))
        .is_err());
    assert_eq!(source.master.as_slice(), master.as_slice());
    assert_eq!(source.request, request);
    assert!(source.finish_authenticated_body(&ready[1..]).is_ok());
    assert!(
        !source.completed,
        "only the public actual handoff, not a failed/preflight helper, marks completion"
    );
}

#[tokio::test]
async fn cancellation_after_remote_add_retries_identical_request_and_hands_off_only_once() {
    let hub = Hub::new();
    let owner_peer = PeerId::from_u64(0x5106);
    let member_peer = PeerId::from_u64(0x5107);
    let mut node = owner(&hub, owner_peer);
    let mut source = prepare(&mut node);
    let network = hub.join(member_peer);
    let master = source.master.clone();
    let request = source.request.clone();
    let mut handoff = PreOwnerConnectionHandoff::default();
    {
        let mut future = Box::pin(source.finish_tracking(&network, &mut handoff, owner_peer));
        assert!(futures::poll!(&mut future).is_pending());
        let (from, bytes, responder) = queued_request(&node.transport).await;
        assert_eq!(bytes, request);
        let ready = node.serve_join(from, &bytes[1..]).unwrap();
        assert_eq!(
            node.member_count(),
            2,
            "the owner actually applied one Add before reply loss"
        );
        drop(future); // original master is outside the cancelled future
        responder.respond(Bytes::from(ready));
    }
    assert!(!source.completed);
    assert_eq!(source.master.as_slice(), master.as_slice());
    let mut future = Box::pin(source.finish_tracking(&network, &mut handoff, owner_peer));
    assert!(futures::poll!(&mut future).is_pending());
    let (from, bytes, responder) = queued_request(&node.transport).await;
    assert_eq!(bytes, request);
    let ready = node.serve_join(from, &bytes[1..]).unwrap();
    responder.respond(Bytes::from(ready));
    let completed = future.await.unwrap();
    assert_eq!(node.member_count(), 2);
    assert_eq!(completed.group.group_id(), source.invite.group_id);
    assert!(source.completed);
    assert!(source
        .finish_tracking(&network, &mut handoff, owner_peer)
        .await
        .is_err());
}

#[tokio::test]
async fn pending_uses_same_private_source_and_ignores_a_foreign_pushed_welcome() {
    let hub = Hub::new();
    let owner_peer = PeerId::from_u64(0x5108);
    let member_peer = PeerId::from_u64(0x5109);
    let mut node = owner(&hub, owner_peer);
    let mut source = prepare(&mut node);
    let network = hub.join(member_peer);
    let attacker = hub.join(PeerId::from_u64(0x5110));
    let mut handoff = PreOwnerConnectionHandoff::default();
    let mut future = Box::pin(source.finish_tracking(&network, &mut handoff, owner_peer));
    assert!(futures::poll!(&mut future).is_pending());
    let (from, bytes, responder) = queued_request(&node.transport).await;
    let ready = node.serve_join(from, &bytes[1..]).unwrap();
    responder.respond(Bytes::from_static(&[JOIN_PENDING]));
    assert!(futures::poll!(&mut future).is_pending());
    let mut foreign = (0..20)
        .map(|_| {
            Box::pin(attacker.request(
                member_peer,
                ProtocolId(RR_PROTOCOL),
                Bytes::from_static(&[KIND_WELCOME]),
            ))
        })
        .collect::<Vec<_>>();
    for request in &mut foreign {
        assert!(futures::poll!(request.as_mut()).is_pending());
    }
    let mut data = vec![KIND_WELCOME];
    data.extend_from_slice(&ready);
    let mut push = Box::pin(node.transport.request(
        member_peer,
        ProtocolId(RR_PROTOCOL),
        Bytes::from(data),
    ));
    assert!(futures::poll!(&mut push).is_pending());
    assert!(
        futures::poll!(&mut future).is_pending(),
        "sixteen ready events must yield before the later valid push"
    );
    let completed = future.await.unwrap();
    assert_eq!(push.await.unwrap(), Bytes::new());
    for request in foreign {
        assert_eq!(request.await.unwrap(), Bytes::new());
    }
    assert_eq!(
        completed
            .routing
            .fresh_chat_admission
            .as_ref()
            .unwrap()
            .key_package_reference(),
        Some(source.expected)
    );
}

#[test]
fn post_welcome_raw_provider_must_fit_original_retained_invoice_before_handoff() {
    let hub = Hub::new();
    let mut node = owner(&hub, PeerId::from_u64(0x5111));
    for _ in 0..5 {
        let existing = MlsDevice::generate().unwrap();
        node.group
            .add_member(&node.device, existing.key_package().unwrap())
            .unwrap();
    }
    let source = prepare(&mut node);
    let ready = node
        .serve_join(PeerId::from_u64(0x5112), &source.request[1..])
        .unwrap();
    // Exact actual prejoin source under a smaller POLICY in this private test fixture, not a
    // public raw-source issuer. Its master genuinely fits; the newly written group does not.
    let narrowed = PreparedDirectJoin {
        master: source.master.clone(),
        request: source.request.clone(),
        invite: source.invite.clone(),
        signer: source.signer,
        expected: source.expected,
        limits: DirectJoinLimits::new(source.master.len(), limits().backing, limits().exchange)
            .unwrap(),
        completed: false,
    };
    assert!(restore_join_device_provider_bounded(
        &narrowed.master,
        narrowed.limits.provider,
        narrowed.limits.backing
    )
    .is_ok());
    assert!(narrowed.finish_authenticated_body(&ready[1..]).is_err());
    assert_eq!(narrowed.master.as_slice(), source.master.as_slice());
    assert!(!narrowed.completed);
    assert!(
        source.finish_authenticated_body(&ready[1..]).is_ok(),
        "same real accepted result fits the genuinely larger original budget"
    );
}

#[test]
fn provider_and_network_caps_are_independent_for_a_small_real_join() {
    let hub = Hub::new();
    let mut node = owner(&hub, PeerId::from_u64(0x5113));
    let invite = node.mint_invite([0x54; 16], 10_000, Vec::new()).unwrap();
    let limits = DirectJoinLimits::new(8 * 1024 * 1024, 512 * 1024, 64 * 1024).unwrap();
    let source =
        PreparedDirectJoin::prepare(MlsDevice::generate().unwrap(), invite, limits).unwrap();
    let ready = node
        .serve_join(PeerId::from_u64(0x5114), &source.request[1..])
        .unwrap();
    assert!(ready.len() < limits.exchange);
    assert!(source.finish_authenticated_body(&ready[1..]).is_ok());
}
