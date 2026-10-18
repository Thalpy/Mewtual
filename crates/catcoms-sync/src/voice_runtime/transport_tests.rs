// Included beside the actual Root/native Runtime attachment fixtures.
fn voice_frame<T: MeshTransport, R: CryptoRngCore>(node: &mut ChannelSync<T, R>, envelope: &voice_v2::Envelope) -> Vec<u8> {
    let inner = envelope.encode();
    let key = node.device.public_key_bytes();
    let mut nonce = [0; 16]; node.rng.fill_bytes(&mut nonce);
    let auth = RequestAuth { ts: node.voice_v2_clock().now_ms(), nonce, epoch: node.group.epoch() };
    let transcript = catchup_auth_transcript(&node.group.group_id(), KIND_VOICE_TRANSPORT, &inner, &key, auth.ts, &auth.nonce, auth.epoch, Some(&node.local_peer()));
    let signature = node.device.sign(&transcript).unwrap();
    let mut result = vec![KIND_VOICE_TRANSPORT];
    result.extend_from_slice(&encode_authed_request(&inner, &key, auth.ts, &auth.nonce, auth.epoch, &signature));
    result
}

// Deliberately forge a semantically false but cryptographically valid current-member ACK.
// The verifier must reject the role claim, rather than relying on signature corruption tests.
fn voice_answer<T: MeshTransport, R: CryptoRngCore>(node: &ChannelSync<T, R>, from: PeerId, frame: &[u8], hash: [u8; 32], status: u8) -> Vec<u8> {
    let (inner, key, ts, nonce, epoch, _) = decode_authed_request(&frame[1..]).unwrap();
    let auth = RequestAuth { ts, nonce, epoch };
    let mut body = Encoder::new();
    body.put_u8(2); body.put_u8(status); body.put_bytes(&hash).unwrap();
    body.put_bytes(blake3::hash(&inner).as_bytes()).unwrap(); body.put_u32(inner.len() as u32);
    let body = body.finish();
    let transcript = crate::voice_transport::response_transcript(&node.group.group_id(), &key, auth, from, node.local_peer(), &body);
    encode_signed_commit_resp(&node.device.public_key_bytes(), &node.device.sign(&transcript).unwrap(), &body)
}

#[tokio::test]
async fn voice_transport_root_requires_bound_noise_identity_and_keeps_helper_ack_typed() {
    let (_hub, mut nodes, ids) = crate::tests::build_members(3).await;
    crate::tests::converge_and_publish_test_routes(&mut nodes);
    nodes[0].initialize_group_policy(GroupMode::PeerToPeer).unwrap(); nodes[0].snapshot().unwrap(); nodes[0].publish_group_policy().unwrap();
    let policy = nodes[0].group_policy.as_ref().unwrap().encode();
    for node in nodes.iter_mut().skip(1) { node.on_group_policy(&policy); }
    let mut nodes = nodes.into_iter();
    let mut source = nodes.next().unwrap(); let mut helper = nodes.next().unwrap(); let mut recipient = nodes.next().unwrap();
    let source_runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let helper_runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(&mut source, &source_runtime); attach(&mut helper, &helper_runtime);
    let _ = helper.voice_room_authority(1).unwrap();
    let scope = source.voice_v2_context(1).unwrap().scope;
    let object = voice_v2::SignedObject::issue(scope, &source.device, [81; 32], 1000, 16_000,
        voice_v2::Body::WatchChallenge { watcher: [1; 32], target: ids[2], join: [2; 32], key_ref: [3; 32], challenge: [4; 32] }).unwrap();
    let permit = voice_v2::RoutePermit::issue(&source.device, &object, ids[1], 1000, 16_000).unwrap();
    let envelope = voice_v2::Envelope::ToHelper { object: object.clone(), permit };
    let frame = voice_frame(&mut source, &envelope);
    helper.promote_member_peer(source.local_peer(), ids[0]); // actual released PEX-level proof only
    let before = helper_runtime.resources().counts();
    assert!(helper.receive_voice_transport(source.local_peer(), &frame).is_none());
    assert_eq!(helper_runtime.resources().counts(), before, "unbound peers spend no crypto/origin resources");
    // Real policy/endpoint-bound request/response establishes both relations, rather than
    // injecting a stronger proof for this new handler to trust.
    let (finalized, served) = tokio::join!(source.finalize_member_connection(helper.local_peer()), helper.run_once());
    assert!(finalized.unwrap() && served.unwrap());
    let answer = helper.receive_voice_transport(source.local_peer(), &frame).unwrap();
    assert_eq!(source.verify_voice_transport_response(helper.local_peer(), &frame, object.hash(), &answer).unwrap(), crate::VoiceTransportStatus::HelperQueued);
    let false_endpoint = voice_answer(&helper, source.local_peer(), &frame, object.hash(), 1);
    assert_eq!(source.verify_voice_transport_response(helper.local_peer(), &frame, object.hash(), &false_endpoint), Err(VoiceError::Unauthorized), "B's valid signature cannot claim C's admission");
    let helper_ledger = helper.voice_transport_actor().unwrap().helper().unwrap();
    assert_eq!(helper_ledger.counts(), (1, 1, 0), "target edge is retained without a route or a dial");
    let mut corrupted = answer.clone(); let last = corrupted.len() - 1; corrupted[last] ^= 1;
    assert!(source.verify_voice_transport_response(helper.local_peer(), &frame, object.hash(), &corrupted).is_err());
    let changed_request = voice_frame(&mut source, &envelope);
    assert_eq!(source.verify_voice_transport_response(helper.local_peer(), &changed_request, object.hash(), &answer), Err(VoiceError::Signature), "ACK is bound to the exact request nonce");
    let (finalized, served) = tokio::join!(helper.finalize_member_connection(recipient.local_peer()), recipient.run_once());
    assert!(finalized.unwrap() && served.unwrap());
    let (finalized, served) = tokio::join!(source.finalize_member_connection(recipient.local_peer()), recipient.run_once());
    assert!(finalized.unwrap() && served.unwrap());
    let wrong_helper = voice_answer(&recipient, source.local_peer(), &frame, object.hash(), 2);
    assert_eq!(source.verify_voice_transport_response(recipient.local_peer(), &frame, object.hash(), &wrong_helper), Err(VoiceError::Unauthorized), "a different current bound member cannot acknowledge B's request");
    let direct = voice_frame(&mut source, &voice_v2::Envelope::Direct(object.clone()));
    let false_custody = voice_answer(&recipient, source.local_peer(), &direct, object.hash(), 2);
    assert_eq!(source.verify_voice_transport_response(recipient.local_peer(), &direct, object.hash(), &false_custody), Err(VoiceError::Unauthorized), "a direct endpoint cannot answer as a helper");
    let before = helper_runtime.resources().counts();
    assert!(helper.receive_voice_transport(recipient.local_peer(), &frame).is_none(), "C cannot relay A's peer-bound outer signature");
    assert_eq!(helper_runtime.resources().counts(), before);
    assert!(helper.handle_request(source.local_peer(), &frame).is_empty(), "kind28 is not enabled by the integration seam alone");
}

#[tokio::test]
async fn voice_transport_root_direct_legacy_refuses_absent_endpoint_without_downgrade_or_original_charge() {
    let (_hub, mut nodes, ids) = crate::tests::build_members(2).await;
    crate::tests::converge_and_publish_test_routes(&mut nodes);
    let mut nodes = nodes.into_iter(); let mut source = nodes.next().unwrap(); let mut recipient = nodes.next().unwrap();
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let source_runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    attach(&mut source, &source_runtime); attach(&mut recipient, &runtime);
    // Legacy groups cannot use kind26. The released incremental document proof is bound at
    // both ends and remains available; use its real signed exchange for initial V2 eligibility.
    source.open_channel(DocType::ChannelIndex, INDEX).await.unwrap();
    recipient.open_channel(DocType::ChannelIndex, INDEX).await.unwrap();
    let (pulled, served) = tokio::join!(source.request_catchup_since(recipient.local_peer(), DocType::ChannelIndex, INDEX, &[]), recipient.run_once());
    assert!(pulled.unwrap().is_some() && served.unwrap());
    // The opposite response establishes the recipient's source relation as well.
    let (pulled, served) = tokio::join!(recipient.request_catchup_since(source.local_peer(), DocType::ChannelIndex, INDEX, &[]), source.run_once());
    assert!(pulled.unwrap().is_some() && served.unwrap());
    let object = voice_v2::SignedObject::issue(source.voice_v2_context(1).unwrap().scope, &source.device, [82; 32], 1000, 16_000,
        voice_v2::Body::WatchChallenge { watcher: [1; 32], target: ids[1], join: [2; 32], key_ref: [3; 32], challenge: [4; 32] }).unwrap();
    let envelope = voice_v2::Envelope::Direct(object.clone());
    let frame = voice_frame(&mut source, &envelope);
    let answer = recipient.receive_voice_transport(source.local_peer(), &frame).unwrap();
    assert_eq!(source.verify_voice_transport_response(recipient.local_peer(), &frame, object.hash(), &answer).unwrap(), crate::VoiceTransportStatus::Refused);
    assert_eq!(recipient.voice_transport_actor().unwrap().helper().unwrap().counts().0, 0);
    let mut legacy = frame.clone(); legacy[0] = KIND_CALL_SIGNAL;
    assert!(recipient.receive_voice_transport(source.local_peer(), &legacy).is_none());
}

type VoiceTestNode = ChannelSync<catcoms_rt::mem::MemNetwork, rand_chacha::ChaCha20Rng>;
struct VoiceForwardFixture {
    _hub: Arc<catcoms_rt::mem::Hub>,
    _source: VoiceTestNode,
    helper: VoiceTestNode,
    recipient: VoiceTestNode,
    helper_runtime: Arc<Runtime>,
    clock: Arc<ManualClock>,
    _endpoints: Vec<Arc<Runtime>>,
}
impl VoiceForwardFixture {
    async fn new() -> Self {
        Self::build(true).await
    }
    async fn empty() -> Self {
        Self::build(false).await
    }
    async fn build(seed_helper_packet: bool) -> Self {
        let (hub, mut nodes, ids) = crate::tests::build_members(3).await;
        crate::tests::converge_and_publish_test_routes(&mut nodes);
        nodes[0].initialize_group_policy(GroupMode::PeerToPeer).unwrap();
        nodes[0].snapshot().unwrap(); nodes[0].publish_group_policy().unwrap();
        let policy = nodes[0].group_policy.as_ref().unwrap().encode();
        for node in nodes.iter_mut().skip(1) { node.on_group_policy(&policy); }
        let mut nodes = nodes.into_iter();
        let mut source = nodes.next().unwrap(); let mut helper = nodes.next().unwrap(); let mut recipient = nodes.next().unwrap();
        let clock = Arc::new(ManualClock::new(1000));
        let helper_runtime = Runtime::new(clock.clone());
        let endpoints = vec![Runtime::new(clock.clone()), Runtime::new(clock.clone())];
        attach(&mut source, &endpoints[0]); attach(&mut helper, &helper_runtime); attach(&mut recipient, &endpoints[1]);
        let (ok, served) = tokio::join!(source.finalize_member_connection(helper.local_peer()), helper.run_once()); assert!(ok.unwrap() && served.unwrap());
        let (ok, served) = tokio::join!(helper.finalize_member_connection(recipient.local_peer()), recipient.run_once()); assert!(ok.unwrap() && served.unwrap());
        if seed_helper_packet {
        let object = voice_v2::SignedObject::issue(source.voice_v2_context(1).unwrap().scope, &source.device, [92; 32], 1000, 16_000,
            voice_v2::Body::WatchChallenge { watcher: [1; 32], target: ids[2], join: [2; 32], key_ref: [3; 32], challenge: [4; 32] }).unwrap();
        let permit = voice_v2::RoutePermit::issue(&source.device, &object, ids[1], 1000, 16_000).unwrap();
        let frame = voice_frame(&mut source, &voice_v2::Envelope::ToHelper { object, permit });
        assert!(helper.receive_voice_transport(source.local_peer(), &frame).is_some());
        }
        Self { _hub: hub, _source: source, helper, recipient, helper_runtime, clock, _endpoints: endpoints }
    }
    async fn reply(&mut self, status: u8) -> crate::CompletedVoiceForward {
        let pending = self.helper.prepare_voice_forwards().unwrap().pop().unwrap();
        let (_publisher, signal) = tokio::sync::watch::channel(false);
        let (completed, ()) = tokio::join!(pending.execute(signal), async {
            loop {
                if let Some(TransportEvent::Request { from, data, responder, .. }) = self.recipient.transport.next_event().await {
                    let (inner, ..) = decode_authed_request(&data[1..]).unwrap();
                    let object = voice_v2::Envelope::decode(&inner).unwrap().object().hash();
                    // Test only an authenticated endpoint assertion, not a live endpoint queue
                    // or end-to-end V2 completion. Kind28 remains absent from normal dispatch.
                    responder.respond(Bytes::from(voice_answer(&self.recipient, from, &data, object, status)));
                    break;
                }
            }
        });
        completed
    }
}

#[tokio::test]
async fn voice_transport_busy_ack_verification_returns_exact_affine_completion_for_retry() {
    use voice_v2::resources::ResourceKind;
    for retire_before_retry in [false, true] {
        let mut f = VoiceForwardFixture::new().await;
        let completed = f.reply(1).await;
        let group = f.helper.voice_v2_context(1).unwrap().scope.group;
        let resources = f.helper_runtime.resources();
        let occupied: Vec<_> = (0..4).map(|_| resources.charge(group, 0, ResourceKind::Crypto).unwrap()).collect();
        let ledger = f.helper.voice_transport_actor().unwrap().helper().unwrap();
        let held = ledger.counts();
        let mut completed = completed;
        for _ in 0..160 {
            let failure = f.helper.complete_voice_forward(completed).unwrap_err();
            assert_eq!(failure.error(), &VoiceError::Capacity, "busy polling cannot exhaust work buckets");
            completed = failure.into_retry().expect("local pressure preserves the exact signed ACK and flight");
        }
        assert_eq!(ledger.counts(), held, "no lower/attempt retirement on a retryable verification refusal");
        assert!(f.helper.prepare_voice_forwards().unwrap().is_empty(), "retained completion must not reauthor a request");
        drop(occupied);
        let requester = f.recipient.device.device_id();
        let now = f.clock.monotonic_ms();
        f.helper.voice_transport_actor().unwrap().with_limits(|limits| {
            for _ in 0..40 { let _ = limits.requester(requester, now); }
            Ok(())
        }).unwrap();
        let failure = f.helper.complete_voice_forward(completed).unwrap_err();
        assert_eq!(failure.error(), &VoiceError::RateLimited, "genuine unrelated requester pressure stays retryable");
        let completed = failure.into_retry().unwrap();
        f.clock.advance_ms(1000);
        if retire_before_retry { f.helper_runtime.revoke_session(); }
        let answer = f.helper.complete_voice_forward(completed);
        if retire_before_retry {
            let failure = answer.unwrap_err();
            assert_eq!(failure.error(), &VoiceError::Unauthorized);
            assert!(failure.into_retry().is_none(), "stale authority is terminal");
        } else {
            assert_eq!(answer.unwrap(), crate::VoiceTransportStatus::EndpointAdmitted);
            assert!(f.helper.prepare_voice_forwards().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn voice_transport_detached_helper_requires_exact_endpoint_ack_and_bounds_refused_retries() {
    let mut f = VoiceForwardFixture::new().await;
    let completed = f.reply(2).await; // A final endpoint cannot turn itself into another helper.
    assert_eq!(f.helper.complete_voice_forward(completed).map_err(|failure| failure.into_error()), Err(VoiceError::Unauthorized));
    let completed = f.reply(3).await;
    assert_eq!(f.helper.complete_voice_forward(completed).unwrap(), crate::VoiceTransportStatus::Refused);
    let completed = f.reply(1).await;
    assert_eq!(f.helper.complete_voice_forward(completed).unwrap(), crate::VoiceTransportStatus::EndpointAdmitted);
    assert!(f.helper.prepare_voice_forwards().unwrap().is_empty());
    let mut f = VoiceForwardFixture::new().await;
    for _ in 0..3 { let completed = f.reply(3).await; f.helper.complete_voice_forward(completed).unwrap(); }
    assert!(f.helper.prepare_voice_forwards().unwrap().is_empty(), "fourth lower helper submission is forbidden");
}

#[tokio::test]
async fn voice_transport_detached_reply_refuses_replaced_route_owner_and_native_lock() {
    for scenario in 0..6 {
        let mut f = VoiceForwardFixture::new().await;
        let completed = f.reply(1).await;
        let mut replacement = None;
        match scenario {
            0 => { replacement = Some(f._hub.join(f.recipient.local_peer())); f.helper.note_peer_disconnected(f.recipient.local_peer()); f.helper.note_peer_connected(f.recipient.local_peer()); }
            1 => { f.recipient.publish_self_record(Vec::new(), 131_073).unwrap(); f.helper.ingest_peer_record(f.recipient.self_record().unwrap().clone()); }
            2 => { f.helper.demote_member_peer(f.recipient.local_peer()); f.helper.promote_member_peer_bound(f.recipient.local_peer(), f.recipient.device_id(), true); }
            3 => { f.clock.advance_ms(3_000); }
            4 => { f.helper_runtime.revoke_session(); }
            _ => { f.helper.blob_fetch_instance = Arc::new(()); }
        }
        assert_eq!(f.helper.complete_voice_forward(completed).map_err(|failure| failure.into_error()), Err(VoiceError::Unauthorized), "scenario {scenario}");
        // No valid late ACK may settle the edge. Except after native lock, a current route can
        // still reserve a bounded fresh attempt; it never revives the old route generation.
        if scenario != 4 { assert_eq!(f.helper.prepare_voice_forwards().unwrap().len(), 1); }
        drop(replacement);
    }
}

#[tokio::test]
async fn voice_transport_deadline_keeps_helper_slot_until_actual_lower_responder_retires() {
    let mut f = VoiceForwardFixture::new().await;
    let pending = f.helper.prepare_voice_forwards().unwrap().pop().unwrap();
    let (_publisher, signal) = tokio::sync::watch::channel(false);
    let (completed, held) = tokio::join!(pending.execute(signal), async {
        loop {
            if let Some(TransportEvent::Request { responder, .. }) = f.recipient.transport.next_event().await {
                f.clock.advance_ms(3_000);
                break responder;
            }
        }
    });
    let before = f.helper_runtime.resources().counts();
    assert_eq!(f.helper.complete_voice_forward(completed).map_err(|failure| failure.into_error()), Err(VoiceError::Unauthorized));
    assert!(f.helper.prepare_voice_forwards().unwrap().is_empty(), "upper timeout cannot recycle the lower edge");
    assert!(f.helper_runtime.resources().counts().0 >= before.0 - 512, "flight remains charged; only retired route metadata may drop");
    drop(held);
    assert_eq!(f.helper.prepare_voice_forwards().unwrap().len(), 1);
}

#[tokio::test]
async fn voice_transport_driver_reconnect_rejects_queued_and_completed_old_work_before_events() {
    for already_replied in [false, true] {
        let mut f = VoiceForwardFixture::new().await;
        let prepared = if already_replied { None } else { Some(f.helper.prepare_voice_forwards().unwrap().pop().unwrap()) };
        let completed = if already_replied { Some(f.reply(1).await) } else { None };
        let replacement = f._hub.join(f.recipient.local_peer());
        // The sync event reader has not consumed any disconnect. Current peer presence alone
        // would accept the old work here; the original driver-owned lease is already retired.
        let completed = if let Some(prepared) = prepared {
            let (_publisher, signal) = tokio::sync::watch::channel(false);
            prepared.execute(signal).await
        } else { completed.unwrap() };
        assert_eq!(f.helper.complete_voice_forward(completed).map_err(|failure| failure.into_error()), Err(VoiceError::Unauthorized));
        use futures::FutureExt;
        assert!(replacement.next_event().now_or_never().is_none(), "old queued request never enters replacement inbox");
    }
}

#[tokio::test]
async fn voice_transport_old_disconnect_event_does_not_revoke_replacement_driver_generation() {
    let mut f = VoiceForwardFixture::new().await;
    let prepared = f.helper.prepare_voice_forwards().unwrap().pop().unwrap();
    let replacement = f._hub.join(f.recipient.local_peer());
    let (_publisher, signal) = tokio::sync::watch::channel(false);
    let completed = prepared.execute(signal).await;
    assert_eq!(f.helper.complete_voice_forward(completed).map_err(|failure| failure.into_error()), Err(VoiceError::Unauthorized));
    let prepared = f.helper.prepare_voice_forwards().unwrap().pop().unwrap();
    // A delayed old event is advisory only for driver generations. It cannot burn this fresh
    // generation's pending request after the actual driver has already reconnected the peer.
    f.helper.note_peer_disconnected(f.recipient.local_peer());
    let (_publisher, signal) = tokio::sync::watch::channel(false);
    let (completed, ()) = tokio::join!(prepared.execute(signal), async {
        if let Some(TransportEvent::Request { from, data, responder, .. }) = replacement.next_event().await {
            let (inner, ..) = decode_authed_request(&data[1..]).unwrap();
            let hash = voice_v2::Envelope::decode(&inner).unwrap().object().hash();
            responder.respond(Bytes::from(voice_answer(&f.recipient, from, &data, hash, 1)));
        } else { panic!("fresh request must arrive"); }
    });
    assert_eq!(f.helper.complete_voice_forward(completed).unwrap(), crate::VoiceTransportStatus::EndpointAdmitted);
}

#[tokio::test]
async fn voice_transport_actual_root_drop_rejects_prepared_driver_submission() {
    use futures::FutureExt;
    let mut f = VoiceForwardFixture::new().await;
    let prepared = f.helper.prepare_voice_forwards().unwrap().pop().unwrap();
    drop(f.helper);
    let (_publisher, signal) = tokio::sync::watch::channel(false);
    drop(prepared.execute(signal).await);
    loop {
        match f.recipient.transport.next_event().now_or_never() {
            Some(Some(TransportEvent::Request { .. })) => panic!("retired Root submitted a request"),
            Some(Some(_)) => {}, _ => break,
        }
    }
}

#[tokio::test]
async fn voice_transport_local_driver_retirement_rejects_queued_worker_and_retained_ack() {
    for replied in [false, true] {
        let mut f = VoiceForwardFixture::new().await;
        let prepared = if replied { None } else { Some(f.helper.prepare_voice_forwards().unwrap().pop().unwrap()) };
        let completed = if replied { Some(f.reply(1).await) } else { None };
        let replacement = f.helper.transport.rejoin();
        let completed = if let Some(prepared) = prepared {
            let (_publisher, signal) = tokio::sync::watch::channel(false); prepared.execute(signal).await
        } else { completed.unwrap() };
        assert_eq!(f.helper.complete_voice_forward(completed).map_err(|failure| failure.into_error()), Err(VoiceError::Unauthorized));
        assert!(f.helper.transport.connection_snapshot().is_empty());
        // The replacement has a live independently owned driver; old A cannot send through it.
        assert!(replacement.connection_lease(f.recipient.local_peer()).unwrap().is_live());
    }
}

fn queued_source_watch(f: &mut VoiceForwardFixture, parent: voice_v2::CallLease) -> (voice_v2::presence::Watcher, voice_v2::SignedObject, voice_v2::source::SourceActivity) {
    use voice_v2::presence::{WatchAuthority, Watcher};
    use rand_chacha::rand_core::SeedableRng;
    let context = f._source.voice_v2_context(1).unwrap();
    let watch = f._source.voice_watch_lease(1).unwrap().with_parent(parent).unwrap();
    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(901);
    let mut watcher = Watcher::new(&context, watch.clone(), f._endpoints[0].resources(), &mut rng).unwrap();
    let key = voice_v2::JoinKey::new([3; 32], hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()).unwrap();
    let announcement = voice_v2::SignedObject::issue(context.scope, &f.recipient.device, [95; 32], 1000, 31_000,
        voice_v2::Body::Announcement { key, epoch: context.epoch, renewal: 1 }).unwrap();
    let authority = WatchAuthority { context: &context, lease: &watch, member: &|_| true };
    let challenge = f._source.voice_transport_actor().unwrap().with_limits(|limits| watcher.challenge(&announcement,
        &f._source.device, &authority, limits, f._source.voice_v2_clock().as_ref(), &mut rng)).unwrap();
    let activity = watcher.source_activity(&authority).unwrap();
    assert!(!f._source.queue_voice_source(challenge.clone(), activity.clone()).unwrap());
    (watcher, challenge, activity)
}

#[tokio::test]
async fn voice_transport_source_helper_ack_and_direct_upgrade_preserve_original_and_total_budget() {
    use voice_v2::source::SourceOutcome;
    let mut f = VoiceForwardFixture::new().await;
    let (_watcher, original, activity) = queued_source_watch(&mut f, voice_v2::CallLease::default());
    for status in [1, 2] {
        let pending = f._source.prepare_voice_sources().unwrap().pop().unwrap();
        assert_eq!(pending.peer(), f.helper.local_peer(), "A has no bound direct route to C");
        let (_publisher, signal) = tokio::sync::watch::channel(false);
        let (completed, ()) = tokio::join!(pending.execute(signal), async {
            loop {
                if let Some(TransportEvent::Request { from, data, responder, .. }) = f.helper.transport.next_event().await {
                    let (inner, ..) = decode_authed_request(&data[1..]).unwrap();
                    let envelope = voice_v2::Envelope::decode(&inner).unwrap();
                    assert!(matches!(envelope, voice_v2::Envelope::ToHelper { .. }));
                    assert_eq!(envelope.object(), &original, "the sole native source keeps original signed bytes");
                    let answer = if status == 2 { f.helper.receive_voice_transport(from, &data).unwrap() }
                        else { voice_answer(&f.helper, from, &data, original.hash(), status) };
                    responder.respond(Bytes::from(answer)); break;
                }
            }
        });
        if status == 1 {
            assert_eq!(f._source.complete_voice_source(completed).map_err(|failure| failure.into_error()), Err(VoiceError::Unauthorized), "a helper signature cannot assert final custody");
        } else {
            assert_eq!(f._source.complete_voice_source(completed).unwrap(), (crate::VoiceTransportStatus::HelperQueued, SourceOutcome::Pending));
        }
        f.clock.advance_ms(500);
    }
    let (ok, served) = tokio::join!(f._source.finalize_member_connection(f.recipient.local_peer()), f.recipient.run_once());
    assert!(ok.unwrap() && served.unwrap());
    let pending = f._source.prepare_voice_sources().unwrap().pop().unwrap();
    assert_eq!(pending.peer(), f.recipient.local_peer(), "newly bound final route wins the last source attempt");
    let (_publisher, signal) = tokio::sync::watch::channel(false);
    let (completed, ()) = tokio::join!(pending.execute(signal), async {
        loop {
            if let Some(TransportEvent::Request { from, data, responder, .. }) = f.recipient.transport.next_event().await {
                let (inner, ..) = decode_authed_request(&data[1..]).unwrap();
                let envelope = voice_v2::Envelope::decode(&inner).unwrap();
                assert!(matches!(envelope, voice_v2::Envelope::Direct(_)));
                assert_eq!(envelope.object(), &original);
                // This pins source transport/ACK settlement only. Native endpoint queues remain
                // an integration gate; a signed fixture assertion is not evidence of delivery.
                responder.respond(Bytes::from(voice_answer(&f.recipient, from, &data, original.hash(), 1))); break;
            }
        }
    });
    assert_eq!(f._source.complete_voice_source(completed).unwrap(), (crate::VoiceTransportStatus::EndpointAdmitted, SourceOutcome::EndpointAdmitted));
    assert!(f._source.queue_voice_source(original, activity).unwrap(), "exact retry cannot refill the native source budget");
    f.clock.advance_ms(500);
    assert!(f._source.prepare_voice_sources().unwrap().is_empty());
    assert_eq!(f._source.voice_transport_actor().unwrap().source().unwrap().counts(), (1, 3, 0));
}

#[tokio::test]
async fn voice_transport_source_end_and_driver_replacement_reject_queued_or_completed_work() {
    for scenario in 0..4 {
        let mut f = VoiceForwardFixture::new().await;
        let parent = voice_v2::CallLease::default();
        let (_watcher, original, activity) = queued_source_watch(&mut f, parent.clone());
        let pending = f._source.prepare_voice_sources().unwrap().pop().unwrap();
        let (_publisher, signal) = tokio::sync::watch::channel(false);
        let mut replacement = None;
        let completed = if scenario % 2 == 0 {
            if scenario == 0 { parent.revoke(); } else { replacement = Some(f.helper.transport.rejoin()); }
            pending.execute(signal).await
        } else {
            let (completed, ()) = tokio::join!(pending.execute(signal), async {
                loop {
                    if let Some(TransportEvent::Request { from, data, responder, .. }) = f.helper.transport.next_event().await {
                        responder.respond(Bytes::from(voice_answer(&f.helper, from, &data, original.hash(), 2))); break;
                    }
                }
            });
            if scenario == 1 { parent.revoke(); } else { replacement = Some(f.helper.transport.rejoin()); }
            completed
        };
        assert_eq!(f._source.complete_voice_source(completed).map_err(|failure| failure.into_error()), Err(VoiceError::Unauthorized), "scenario {scenario}");
        assert_eq!(f._source.voice_transport_actor().unwrap().source().unwrap().counts().1, 1);
        if scenario < 2 { assert_eq!(f._source.queue_voice_source(original, activity), Err(VoiceError::Unauthorized)); }
        drop(replacement);
    }
}

#[tokio::test]
async fn voice_transport_source_timeout_keeps_actual_lower_flight_independent_of_new_attempt() {
    let mut f = VoiceForwardFixture::new().await;
    let (_watcher, original, activity) = queued_source_watch(&mut f, voice_v2::CallLease::default());
    let pending = f._source.prepare_voice_sources().unwrap().pop().unwrap();
    let (_publisher, signal) = tokio::sync::watch::channel(false);
    let (completed, held) = tokio::join!(pending.execute(signal), async {
        loop {
            if let Some(TransportEvent::Request { responder, .. }) = f.helper.transport.next_event().await {
                f.clock.advance_ms(3_000);
                break responder;
            }
        }
    });
    assert_eq!(f._source.complete_voice_source(completed).map_err(|failure| failure.into_error()), Err(VoiceError::Unauthorized));
    let ledger = f._source.voice_transport_actor().unwrap().source().unwrap();
    assert_eq!(ledger.counts(), (1, 1, 1), "upper timeout retains the actual driver's flight");
    assert!(f._source.queue_voice_source(original, activity).unwrap());
    let second = f._source.prepare_voice_sources().unwrap().pop().unwrap();
    assert_eq!(ledger.counts(), (1, 2, 2), "small packet hedge has separately charged custody");
    let charged = f._endpoints[0].resources().counts().0;
    drop(held);
    assert_eq!(ledger.counts(), (1, 2, 1), "only the retired lower attempt leaves");
    assert!(f._endpoints[0].resources().counts().0 < charged);
    drop(second);
    assert_eq!(ledger.counts(), (1, 2, 0), "dropping preparation never refunds its attempt");
}

fn public_join_key(n: u8) -> voice_v2::JoinKey {
    voice_v2::JoinKey::new([n; 32], hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()).unwrap()
}
async fn serve_public_voice(node: &mut VoiceTestNode) {
    loop {
        if let Some(TransportEvent::Request { from, data, responder, .. }) = node.transport.next_event().await {
            assert_eq!(data[0], KIND_VOICE_TRANSPORT);
            responder.respond(Bytes::from(node.receive_voice_transport(from, &data).unwrap()));
            return;
        }
    }
}
async fn public_bridge_pass(f: &mut VoiceForwardFixture) {
    f._source.pump_voice_endpoint().unwrap();
    f.recipient.pump_voice_endpoint().unwrap();
    for pending in f._source.prepare_voice_sources().unwrap() {
        assert_eq!(pending.peer(), f.helper.local_peer(), "no bound A-C route exists");
        let (_publisher, signal) = tokio::sync::watch::channel(false);
        let (completed, ()) = tokio::join!(pending.execute(signal), serve_public_voice(&mut f.helper));
        f._source.complete_voice_source(completed).unwrap();
    }
    for pending in f.recipient.prepare_voice_sources().unwrap() {
        assert_eq!(pending.peer(), f.helper.local_peer(), "no bound C-A route exists");
        let (_publisher, signal) = tokio::sync::watch::channel(false);
        let (completed, ()) = tokio::join!(pending.execute(signal), serve_public_voice(&mut f.helper));
        f.recipient.complete_voice_source(completed).unwrap();
    }
    for pending in f.helper.prepare_voice_forwards().unwrap() {
        let target = pending.peer();
        let (_publisher, signal) = tokio::sync::watch::channel(false);
        let completed = if target == f._source.local_peer() {
            let (completed, ()) = tokio::join!(pending.execute(signal), serve_public_voice(&mut f._source)); completed
        } else {
            assert_eq!(target, f.recipient.local_peer());
            let (completed, ()) = tokio::join!(pending.execute(signal), serve_public_voice(&mut f.recipient)); completed
        };
        f.helper.complete_voice_forward(completed).unwrap();
    }
}

#[tokio::test]
async fn voice_transport_native_public_presence_proves_both_joins_through_unjoined_bound_bridge() {
    let mut f = VoiceForwardFixture::empty().await;
    let a = Arc::new(f._source.voice_call_grant(1).unwrap());
    let c = Arc::new(f.recipient.voice_call_grant(1).unwrap());
    let ak = public_join_key(71); let ck = public_join_key(72);
    f._source.start_voice_endpoint(a.clone(), ak.join, ak.public_key).unwrap();
    f.recipient.start_voice_endpoint(c.clone(), ck.join, ck.public_key).unwrap();
    let aw = f._source.voice_runtime.as_ref().unwrap().endpoints.implicit_watch_for_test().unwrap();
    let cw = f.recipient.voice_runtime.as_ref().unwrap().endpoints.implicit_watch_for_test().unwrap();
    assert!(f._source.voice_watch_peers(&aw).unwrap().is_empty());
    assert!(f.recipient.voice_watch_peers(&cw).unwrap().is_empty());
    let mut reached = false;
    for _ in 0..12 {
        public_bridge_pass(&mut f).await;
        let ap = f._source.voice_watch_peers(&aw).unwrap();
        let cp = f.recipient.voice_watch_peers(&cw).unwrap();
        if ap.len() == 1 && cp.len() == 1 {
            assert_eq!(ap[0].announcement().issuer(), f.recipient.device_id());
            assert_eq!(cp[0].announcement().issuer(), f._source.device_id());
            assert!(matches!(ap[0].announcement().body(), voice_v2::Body::Announcement { key, .. } if key == &ck));
            assert!(matches!(cp[0].announcement().body(), voice_v2::Body::Announcement { key, .. } if key == &ak));
            reached = true; break;
        }
        f.clock.advance_ms(500);
    }
    assert!(reached, "both actual native challenge/proof flows must finish");
    assert!(f.helper.voice_runtime.as_ref().unwrap().endpoints.implicit_watch_for_test().is_none(), "forwarding confers no call participation");
    // Mem models connected links. This proves native public protocol/route ownership, not an
    // A-C-blocked physical topology, WebCrypto confirmation, SDP forwarding or audible media.
}

#[tokio::test]
async fn voice_transport_public_queue_capacity_and_exact_retry_do_not_spend_extra_origin_allowance() {
    let mut f = VoiceForwardFixture::empty().await;
    let parent = voice_v2::CallLease::default();
    let watch = f.recipient.start_voice_watch(1, parent).unwrap();
    let scope = f.recipient.voice_v2_context(1).unwrap().scope;
    // First create only the immediate-requester accounting record, without an eligible local
    // participant or any original room work. That isolates persistent queue refusal below.
    let refusal = voice_v2::SignedObject::issue(scope, &f.helper.device, [110; 32], 1000, 16_000,
        voice_v2::Body::WatchChallenge { watcher: [1; 32], target: f.recipient.device_id(), join: [2; 32], key_ref: [3; 32], challenge: [4; 32] }).unwrap();
    let frame = voice_frame(&mut f.helper, &voice_v2::Envelope::Direct(refusal.clone()));
    let answer = f.recipient.receive_voice_transport(f.helper.local_peer(), &frame).unwrap();
    assert_eq!(f.helper.verify_voice_transport_response(f.recipient.local_peer(), &frame, refusal.hash(), &answer).unwrap(), crate::VoiceTransportStatus::Refused);
    let object = voice_v2::SignedObject::issue(scope, &f.helper.device, [111; 32], 1000, 31_000,
        voice_v2::Body::Announcement { key: public_join_key(79), epoch: f.helper.group.epoch(), renewal: 1 }).unwrap();
    let envelope = voice_v2::Envelope::Direct(object.clone());
    let frame = voice_frame(&mut f.helper, &envelope);
    let pool = f._endpoints[1].resources();
    let baseline = pool.counts().0;
    let verification = 5 * envelope.encode().len() + 4096;
    let full = pool.charge(scope.group, 256 * 1024 - baseline - verification - 512, voice_v2::resources::ResourceKind::Retained).unwrap();
    let answer = f.recipient.receive_voice_transport(f.helper.local_peer(), &frame).unwrap();
    assert_eq!(f.helper.verify_voice_transport_response(f.recipient.local_peer(), &frame, object.hash(), &answer).unwrap(), crate::VoiceTransportStatus::Refused);
    drop(full);
    for _ in 0..12 {
        let answer = f.recipient.receive_voice_transport(f.helper.local_peer(), &frame).unwrap();
        assert_eq!(f.helper.verify_voice_transport_response(f.recipient.local_peer(), &frame, object.hash(), &answer).unwrap(), crate::VoiceTransportStatus::EndpointAdmitted);
    }
    let changed = voice_v2::SignedObject::issue(scope, &f.helper.device, object.id(), 1000, 31_000,
        voice_v2::Body::Announcement { key: public_join_key(80), epoch: f.helper.group.epoch(), renewal: 1 }).unwrap();
    let substituted = voice_frame(&mut f.helper, &voice_v2::Envelope::Direct(changed.clone()));
    let answer = f.recipient.receive_voice_transport(f.helper.local_peer(), &substituted).unwrap();
    assert_eq!(f.helper.verify_voice_transport_response(f.recipient.local_peer(), &substituted, changed.hash(), &answer).unwrap(), crate::VoiceTransportStatus::Refused,
        "a valid member signature cannot replace an admitted packet ID");
    // Eight new room packets fit the existing origin burst, including the first above. Neither
    // the capacity refusal nor twelve exact retries may consume its original allowance.
    for id in 112..=119 {
        let object = voice_v2::SignedObject::issue(scope, &f.helper.device, [id; 32], 1000, 31_000,
            voice_v2::Body::Announcement { key: public_join_key(79), epoch: f.helper.group.epoch(), renewal: 1 }).unwrap();
        let frame = voice_frame(&mut f.helper, &voice_v2::Envelope::Direct(object.clone()));
        let answer = f.recipient.receive_voice_transport(f.helper.local_peer(), &frame).unwrap();
        let status = f.helper.verify_voice_transport_response(f.recipient.local_peer(), &frame, object.hash(), &answer).unwrap();
        assert_eq!(status, if id < 119 { crate::VoiceTransportStatus::EndpointAdmitted } else { crate::VoiceTransportStatus::Refused });
    }
    assert!(f.recipient.voice_watch_peers(&watch).unwrap().is_empty(), "queue admission/announcements alone never prove presence");
}

#[tokio::test]
async fn voice_transport_replaced_watcher_seeds_fresh_challenge_without_adopting_cached_ack() {
    let mut f = VoiceForwardFixture::empty().await;
    let parent = voice_v2::CallLease::default();
    let old_watch = f.recipient.start_voice_watch(1, parent.clone()).unwrap();
    let scope = old_watch.scope();
    let object = voice_v2::SignedObject::issue(scope, &f.helper.device, [121; 32], 1000, 31_000,
        voice_v2::Body::Announcement { key: public_join_key(81), epoch: f.helper.group.epoch(), renewal: 1 }).unwrap();
    let frame = voice_frame(&mut f.helper, &voice_v2::Envelope::Direct(object.clone()));
    let answer = f.recipient.receive_voice_transport(f.helper.local_peer(), &frame).unwrap();
    assert_eq!(f.helper.verify_voice_transport_response(f.recipient.local_peer(), &frame, object.hash(), &answer).unwrap(), crate::VoiceTransportStatus::EndpointAdmitted);
    f.recipient.pump_voice_endpoint().unwrap();
    let actor = f.recipient.voice_transport_actor().unwrap();
    let ledger = actor.source().unwrap();
    let old = ledger.take_ready(actor, &f.recipient.voice_bound_routes()).unwrap().pop().unwrap();
    let old_object = old.object().clone();
    let lower = old.keepalive();
    parent.revoke();
    assert!(!old.authority().authorized(), "native watcher parent closes queued source before a Root turn");
    let new_watch = f.recipient.start_voice_watch(1, voice_v2::CallLease::default()).unwrap();
    let answer = f.recipient.receive_voice_transport(f.helper.local_peer(), &frame).unwrap();
    assert_eq!(f.helper.verify_voice_transport_response(f.recipient.local_peer(), &frame, object.hash(), &answer).unwrap(), crate::VoiceTransportStatus::Refused,
        "old admitted native recipients cannot authorize a replacement watcher ACK");
    f.recipient.pump_voice_endpoint().unwrap();
    let actor = f.recipient.voice_transport_actor().unwrap();
    let fresh = ledger.take_ready(actor, &f.recipient.voice_bound_routes()).unwrap().pop().unwrap();
    let (voice_v2::Body::WatchChallenge { watcher: first, challenge: first_challenge, .. },
        voice_v2::Body::WatchChallenge { watcher: second, challenge: second_challenge, .. }) = (old_object.body(), fresh.object().body()) else { panic!("native challenges required"); };
    assert_ne!(first, second);
    assert_ne!(first_challenge, second_challenge);
    assert!(fresh.authority().authorized());
    assert_eq!(ledger.counts(), (2, 2, 2), "candidate reuse creates fresh watcher work, no source-attempt refill");
    assert!(f.recipient.voice_watch_peers(&new_watch).unwrap().is_empty(), "cached candidates grant no liveness");
    assert_eq!(ledger.complete(old.finish(None)).unwrap(), voice_v2::source::SourceOutcome::Retired);
    drop(lower);
    assert_eq!(ledger.counts(), (2, 2, 1));
    drop(fresh);
}
