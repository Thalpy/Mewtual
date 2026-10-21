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
    let mut corrupted = answer.to_vec(); let last = corrupted.len() - 1; corrupted[last] ^= 1;
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
    reply_clock: Arc<ReplySampleClock>,
    _endpoints: Vec<Arc<Runtime>>,
}
/// One-shot synchronous retirement at a native Clock seam. No wall time or sleeping is used
/// to place these driver-currency races; the callback owns only observing/test-native handles.
type ClockRetirement = (usize, Box<dyn FnOnce() + Send>);
struct ReplySampleClock {
    inner: Arc<ManualClock>,
    on_mono: std::sync::Mutex<Option<ClockRetirement>>,
}
impl std::fmt::Debug for ReplySampleClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("ReplySampleClock { .. }") }
}
impl catcoms_rt::Clock for ReplySampleClock {
    fn now_ms(&self) -> u64 { self.inner.now_ms() }
    fn monotonic_ms(&self) -> u64 {
        let callback = {
            let mut armed = self.on_mono.lock().unwrap();
            match armed.as_mut() {
                Some((skip, _)) if *skip > 0 => { *skip -= 1; None },
                Some(_) => armed.take().map(|(_, action)| action),
                None => None,
            }
        };
        if let Some(callback) = callback { callback(); }
        self.inner.monotonic_ms()
    }
    fn sleep(&self, duration: std::time::Duration) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.inner.sleep(duration)
    }
}
fn incoming_helper_frame(f: &mut VoiceForwardFixture, message: u8) -> Vec<u8> {
    let object = voice_v2::SignedObject::issue(f._source.voice_v2_context(1).unwrap().scope,
        &f._source.device, [message; 32], 1000, 16_000,
        voice_v2::Body::WatchChallenge { watcher: [1; 32], target: f.recipient.device_id(),
            join: [2; 32], key_ref: [3; 32], challenge: [4; 32] }).unwrap();
    let permit = voice_v2::RoutePermit::issue(&f._source.device, &object, f.helper.device_id(), 1000, 16_000).unwrap();
    voice_frame(&mut f._source, &voice_v2::Envelope::ToHelper { object, permit })
}

#[tokio::test]
async fn voice_transport_prepared_ack_keeps_verification_through_driver_and_last_byte_owner() {
    let mut f = VoiceForwardFixture::empty().await;
    let frame = incoming_helper_frame(&mut f, 150);
    let pool = f.helper_runtime.resources();
    let before = pool.counts().1;
    let reply = f.helper.receive_voice_transport(f._source.local_peer(), &frame).unwrap();
    assert_eq!(pool.counts().1, before + 1);
    let (responder, receiver) = catcoms_rt::Responder::channel();
    reply.respond(responder).unwrap();
    let response = receiver.recv_owned().await.unwrap();
    assert!(response.is_current());
    let bytes = response.data().clone();
    // Model the actual driver's separate serialized-buffer custody. Byte and lower-send
    // holders are independent; dropping either one cannot refund the other.
    let lower = response.keepalive().unwrap();
    drop(response);
    f.clock.advance_ms(60_001);
    f.helper_runtime.collect_expired();
    assert_eq!(pool.counts().1, before + 1, "expiry suppresses effects without refunding live bytes");
    drop(lower);
    assert_eq!(pool.counts().1, before + 1, "the byte clone still owns verification custody");
    drop(bytes);
    assert_eq!(pool.counts().1, before);
}

#[tokio::test]
async fn voice_transport_driver_time_sample_precedes_all_negative_and_positive_reply_owner_checks() {
    for positive in [false, true] {
        let mut f = VoiceForwardFixture::empty().await;
        let frame = if positive {
            incoming_helper_frame(&mut f, 156)
        } else {
            let object = voice_v2::SignedObject::issue(f._source.voice_v2_context(1).unwrap().scope,
                &f._source.device, [157; 32], 1000, 16_000,
                voice_v2::Body::WatchChallenge { watcher: [1; 32], target: f.helper.device_id(),
                    join: [2; 32], key_ref: [3; 32], challenge: [4; 32] }).unwrap();
            voice_frame(&mut f._source, &voice_v2::Envelope::Direct(object))
        };
        let reply = f.helper.receive_voice_transport(f._source.local_peer(), &frame).unwrap();
        let (responder, receiver) = catcoms_rt::Responder::channel();
        reply.respond(responder).unwrap();
        let response = receiver.recv_owned().await.unwrap();
        assert!(response.is_current());
        let before = f.helper_runtime.resources().counts();
        if positive {
            let hub = f._hub.clone();
            let peer = f._source.local_peer();
            // Old composition checked the route, then sampled another Clock for its packet
            // receipt. Retiring that route on the second sample escaped those earlier checks.
            *f.reply_clock.on_mono.lock().unwrap() = Some((1, Box::new(move || { drop(hub.join(peer)); })));
        } else {
            let runtime = Arc::downgrade(&f.helper_runtime);
            // A Refused reply has no positive receipt to incidentally recheck the session.
            *f.reply_clock.on_mono.lock().unwrap() = Some((0, Box::new(move || { runtime.upgrade().unwrap().revoke_session(); })));
        }
        assert!(!response.is_current(), "positive={positive}");
        assert_eq!(f.helper_runtime.resources().counts(), before, "retirement does not refund held response bytes");
        drop(response);
        assert_eq!(f.helper_runtime.resources().counts().1, 0);
    }
}

#[tokio::test]
async fn voice_transport_pending_driver_checks_all_route_and_room_owners_after_original_clocks() {
    for boundary in ["wrapper root", "packet connection", "packet room"] {
        let mut f = VoiceForwardFixture::new().await;
        let mut pending = f.helper.prepare_voice_forwards().unwrap().pop().unwrap();
        let resources = f.helper_runtime.resources();
        let authority = if boundary == "wrapper root" {
            let helper = Arc::new(std::sync::Mutex::new(f.helper));
            let original = helper.clone();
            let clock = Arc::new(ReplySampleClock {
                inner: f.clock.clone(),
                on_mono: std::sync::Mutex::new(Some((0, Box::new(move || {
                    original.lock().unwrap().retire_voice_actor();
                })))),
            });
            pending.authority_with_clock_for_test(clock)
        } else {
            // Packet clocks use the original Runtime probe. Keep the outer wrapper at the
            // same monotonic origin, with no callback, to isolate a late packet-clock change.
            let authority = pending.authority_with_clock_for_test(f.clock.clone());
            let retire: Box<dyn FnOnce() + Send> = if boundary == "packet connection" {
                let hub = f._hub.clone(); let peer = f.recipient.local_peer();
                Box::new(move || { drop(hub.join(peer)); })
            } else {
                let room = f.helper.voice_room_authority(1).unwrap().lease;
                Box::new(move || room.revoke())
            };
            *f.reply_clock.on_mono.lock().unwrap() = Some((0, retire));
            authority
        };
        assert!(!authority.authorized(), "{boundary}");
        let held = resources.counts().0;
        assert!(held >= 512, "dead effects still retain actual route/flight custody");
        drop(pending); drop(authority);
        assert!(resources.counts().0 < held, "{boundary}: release only on actual last owner");
    }
}

#[tokio::test]
async fn voice_transport_reply_route_capacity_refuses_before_original_packet_admission() {
    let mut f = VoiceForwardFixture::empty().await;
    let frame = incoming_helper_frame(&mut f, 155);
    let route = f.helper.voice_bound_routes().into_iter().find(|route| route.device == f._source.device_id()).unwrap();
    // Fill only the small route-registry boundary. These trusted unit-fixture owners do not
    // claim to model64 simultaneous real Crypto operations or bypass their separate limits.
    let held: Vec<_> = (0..64).map(|_| f.helper.capture_voice_reply_route(route, Arc::new(())).unwrap()).collect();
    assert!(f.helper.receive_voice_transport(f._source.local_peer(), &frame).is_none());
    assert_eq!(f.helper.voice_transport_actor().unwrap().helper().unwrap().counts(), (0,0,0));
    assert_eq!(f.helper_runtime.resources().counts().1, 0);
    drop(held);
    let reply = f.helper.receive_voice_transport(f._source.local_peer(), &frame).unwrap();
    assert_eq!(f.helper.voice_transport_actor().unwrap().helper().unwrap().counts(), (1,1,0));
    drop(reply);
    assert_eq!(f.helper_runtime.resources().counts().1, 0);
}

#[tokio::test]
async fn voice_transport_old_held_ingress_cannot_capture_new_connection_and_admit_helper_work() {
    use futures::FutureExt;
    let mut f = VoiceForwardFixture::empty().await;
    let (_watcher, _object, _activity) = queued_source_watch(&mut f, voice_v2::CallLease::default());
    let pending = f._source.prepare_voice_sources().unwrap().pop().unwrap();
    let (_publisher, signal) = tokio::sync::watch::channel(false);
    let mut flight = Box::pin(pending.execute(signal));
    assert!(flight.as_mut().now_or_never().is_none());
    let (from, data, responder) = loop {
        if let Some(TransportEvent::Request { from, data, responder, .. }) = f.helper.transport.next_event().await {
            break (from, data, responder);
        }
    };
    assert!(responder.is_current());
    let replacement = f._hub.join(f._source.local_peer());
    // Mem's canceled request drops its original response receiver on this poll. Production
    // ingress additionally observes the actual driver connection directly, before event drain.
    drop(flight.as_mut().now_or_never().expect("original driver connection retired"));
    assert!(!responder.is_current());
    f.helper.handle_voice_transport_request(from, &data, responder);
    assert_eq!(f.helper.voice_transport_actor().unwrap().helper().unwrap().counts(), (0,0,0));
    assert_eq!(f.helper_runtime.resources().counts().1, 0);
    let fresh = incoming_helper_frame(&mut f, 158);
    assert!(f.helper.receive_voice_transport(from, &fresh).is_some(), "fresh ingress on the replacement connection remains eligible");
    drop(replacement);
}

#[tokio::test]
async fn voice_transport_prepared_ack_refuses_original_route_and_request_deadline_replacement() {
    for scenario in 0..5 {
        let mut f = VoiceForwardFixture::empty().await;
        let frame = incoming_helper_frame(&mut f, 151);
        let reply = f.helper.receive_voice_transport(f._source.local_peer(), &frame).unwrap();
        let mut replacement = None;
        match scenario {
            0 => { replacement = Some(f._hub.join(f._source.local_peer())); } // Driver closes before Root sees events.
            1 => { f._source.publish_self_record(Vec::new(), 131_073).unwrap(); f.helper.ingest_peer_record(f._source.self_record().unwrap().clone()); }
            2 => { f.helper.demote_member_peer(f._source.local_peer()); f.helper.promote_member_peer_bound(f._source.local_peer(), f._source.device_id(), true); }
            3 => { f.clock.advance_ms(5_000); }
            _ => { f.helper_runtime.revoke_session(); }
        }
        let (responder, receiver) = catcoms_rt::Responder::channel();
        assert!(matches!(reply.respond(responder), Err(catcoms_rt::TransportError::Cancelled)), "scenario {scenario}");
        assert!(receiver.recv_owned().await.is_none());
        assert_eq!(f.helper_runtime.resources().counts().1, 0);
        drop(replacement);
    }
}

#[tokio::test]
async fn voice_transport_queued_ack_epoch_change_retires_even_when_all_existing_members_survive() {
    let mut f = VoiceForwardFixture::empty().await;
    let frame = incoming_helper_frame(&mut f, 152);
    let reply = f.helper.receive_voice_transport(f._source.local_peer(), &frame).unwrap();
    let (responder, receiver) = catcoms_rt::Responder::channel();
    reply.respond(responder).unwrap();
    let response = receiver.recv_owned().await.unwrap();
    assert!(response.is_current());
    let original_epoch = f.helper.group.epoch();
    let joiner = MlsDevice::generate().unwrap();
    let key_package = joiner.key_package().unwrap();
    f.helper.with_observed_mls_transition(|node| node.group.add_member(&node.device, key_package)).unwrap();
    assert!(f.helper.group.epoch() > original_epoch);
    assert!(f.helper.group.contains_device(&f._source.device_id()));
    assert!(f.helper.group.contains_device(&f.recipient.device_id()));
    assert!(!response.is_current(), "the driver must reject an old-epoch ACK before the next actor await");
    assert_eq!(f.helper_runtime.resources().counts().1, 1, "retired response still owns its credit");
    drop(response);
    assert_eq!(f.helper_runtime.resources().counts().1, 0);
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
        let reply_clock = Arc::new(ReplySampleClock { inner: clock.clone(), on_mono: std::sync::Mutex::new(None) });
        let helper_runtime = Runtime::new(reply_clock.clone());
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
        Self { _hub: hub, _source: source, helper, recipient, helper_runtime, clock, reply_clock, _endpoints: endpoints }
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
                    if status == 2 {
                        f.helper.handle_voice_transport_request(from, &data, responder);
                    } else {
                        responder.respond(Bytes::from(voice_answer(&f.helper, from, &data, original.hash(), status)));
                    }
                    break;
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

fn queued_source_offer(f: &mut VoiceForwardFixture) -> (voice_v2::presence::LocalJoin, voice_v2::SignedObject) {
    use voice_v2::packets::{Authority, PairBinding, ReserveRequest};
    use voice_v2::presence::LocalJoin;
    use voice_v2::{EncryptedKind, JoinKey, MAX_SDP_PLAINTEXT};
    let context = f._source.voice_v2_context(1).unwrap();
    let lease = f._source.voice_call_lease(1).unwrap();
    let public = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap();
    let key = JoinKey::new([121; 32], public).unwrap();
    let mut join = LocalJoin::new(&context, key, lease.clone(), f._endpoints[0].resources()).unwrap();
    let authority = Authority { context: &context, lease: &lease, member: &|_| true };
    // This test begins at the native established-pair seam; it claims actual Root framing,
    // transport submission and ACK verification, not browser AEAD or live RTC acceptance.
    let ledger = join.packet_ledger_for_test();
    let handshake = [122; 32];
    f._source.voice_transport_actor().unwrap().with_limits(|limits| {
        ledger.established_sdp_pair_for_test(PairBinding { peer: f.recipient.device.device_id(), join: [123; 32], key_ref: [124; 32], handshake }, &authority, limits, &[f.helper.device.device_id()], f.clock.as_ref())
    }).unwrap();
    let reservation = ledger.reserve(f._source.device.public_key_bytes().try_into().unwrap(), ReserveRequest {
        handshake, kind: EncryptedKind::Offer, offer: None, ciphertext_bytes: MAX_SDP_PLAINTEXT + 16,
    }, &authority, f.clock.as_ref(), &mut f._source.rng).unwrap();
    let object = ledger.submit(reservation.id, &vec![5; MAX_SDP_PLAINTEXT + 16], &f._source.device, &authority, f.clock.as_ref()).unwrap();
    let activity = join.source_packet_activity(&object, &authority, f._endpoints[0].clock()).unwrap();
    f._source.queue_voice_source(object.clone(), activity).unwrap();
    (join, object)
}

#[tokio::test]
async fn voice_source_route_registry_pressure_preserves_attempts_and_limits_ready_batch() {
    let mut f = VoiceForwardFixture::empty().await;
    let (_watcher, first, activity) = queued_source_watch(&mut f, voice_v2::CallLease::default());
    for message in [162, 163] {
        let object = voice_v2::SignedObject::issue(first.scope(), &f._source.device,
            [message; 32], first.issued(), first.expires(), first.body().clone()).unwrap();
        f._source.queue_voice_source(object, activity.clone()).unwrap();
    }
    let route = f._source.voice_bound_routes().into_iter().find(|r| r.device == f.helper.device_id()).unwrap();
    let mut held: Vec<_> = (0..64).map(|_| f._source.capture_voice_reply_route(route, Arc::new(())).unwrap()).collect();
    let ledger = f._source.voice_transport_actor().unwrap().source().unwrap();
    for _ in 0..5 {
        assert!(f._source.prepare_voice_sources().unwrap().is_empty());
        f.clock.advance_ms(500);
    }
    assert_eq!(ledger.counts(), (3, 0, 0), "local registry pressure spends no source attempt");
    drop(held.pop());
    let pending = f._source.prepare_voice_sources().unwrap();
    assert_eq!(pending.len(), 1, "one free route cannot debit a four-job batch");
    assert_eq!(ledger.counts(), (3, 1, 1));
    drop(held);
    drop(pending);
    let pending = f._source.prepare_voice_sources().unwrap();
    assert_eq!(pending.len(), 2, "other packets retain their fair first opportunity");
    assert_eq!(ledger.counts(), (3, 3, 2));
}

#[tokio::test]
async fn voice_helper_route_registry_pressure_does_not_exhaust_forwarding_before_handoff() {
    let mut f = VoiceForwardFixture::new().await;
    for message in [164, 165] {
        let frame = incoming_helper_frame(&mut f, message);
        assert!(f.helper.receive_voice_transport(f._source.local_peer(), &frame).is_some());
    }
    let route = f.helper.voice_bound_routes().into_iter().find(|r| r.device == f.recipient.device_id()).unwrap();
    let mut held: Vec<_> = (0..64).map(|_| f.helper.capture_voice_reply_route(route, Arc::new(())).unwrap()).collect();
    for _ in 0..5 {
        assert!(f.helper.prepare_voice_forwards().unwrap().is_empty());
        f.clock.advance_ms(500);
    }
    drop(held.pop());
    let pending = f.helper.prepare_voice_forwards().unwrap();
    assert_eq!(pending.len(), 1);
    drop(held);
    drop(pending);
    let pending = f.helper.prepare_voice_forwards().unwrap();
    assert_eq!(pending.len(), 3, "none of the three packet edges was exhausted by local pressure");
}

#[tokio::test]
async fn delayed_authenticated_target_ack_confirms_after_helper_hedge_but_not_after_retirement() {
    use voice_v2::source::SourceOutcome;
    for retire in ["none", "end", "route", "deadline"] {
        let mut f = VoiceForwardFixture::empty().await;
        let parent = voice_v2::CallLease::default();
        let (_watcher, original, _activity) = queued_source_watch(&mut f, parent.clone());
        let (ok, served) = tokio::join!(f._source.finalize_member_connection(f.recipient.local_peer()), f.recipient.run_once());
        assert!(ok.unwrap() && served.unwrap());
        let pending = f._source.prepare_voice_sources().unwrap().pop().unwrap();
        assert_eq!(pending.peer(), f.recipient.local_peer());
        let (_publisher, signal) = tokio::sync::watch::channel(false);
        let (direct, ()) = tokio::join!(pending.execute(signal), async {
            loop {
                if let Some(TransportEvent::Request { from, data, responder, .. }) = f.recipient.transport.next_event().await {
                    responder.respond(Bytes::from(voice_answer(&f.recipient, from, &data, original.hash(), 1)));
                    break;
                }
            }
        });
        f.clock.advance_ms(500);
        let hedge = f._source.prepare_voice_sources().unwrap().pop().unwrap();
        assert_eq!(hedge.peer(), f.helper.local_peer());
        let (_publisher, signal) = tokio::sync::watch::channel(false);
        let (helper, ()) = tokio::join!(hedge.execute(signal), async {
            loop {
                if let Some(TransportEvent::Request { from, data, responder, .. }) = f.helper.transport.next_event().await {
                    responder.respond(Bytes::from(voice_answer(&f.helper, from, &data, original.hash(), 2)));
                    break;
                }
            }
        });
        let replacement = match retire {
            "end" => { parent.revoke(); None }
            "route" => Some(f.recipient.transport.rejoin()),
            "deadline" => { f.clock.advance_ms(3_000); None }
            _ => None,
        };
        let result = f._source.complete_voice_source(direct);
        if retire == "none" {
            assert_eq!(result.unwrap(), (crate::VoiceTransportStatus::EndpointAdmitted, SourceOutcome::EndpointAdmitted));
            assert_eq!(f._source.complete_voice_source(helper).unwrap(),
                (crate::VoiceTransportStatus::HelperQueued, SourceOutcome::EndpointAdmitted));
            f.clock.advance_ms(500);
            assert!(f._source.prepare_voice_sources().unwrap().is_empty(), "confirmed target needs no third submission");
        } else {
            assert_eq!(result.map_err(|failure| failure.into_error()), Err(VoiceError::Unauthorized), "{retire}");
        }
        drop(replacement);
    }
}

#[tokio::test]
async fn root_prepares_and_submits_prepaid_sdp_with_the_ordinary_pool_full() {
    use voice_v2::resources::ResourceKind;
    let mut f = VoiceForwardFixture::empty().await;
    let (_join, object) = queued_source_offer(&mut f);
    let context = f._source.voice_v2_context(1).unwrap();
    let resources = f._endpoints[0].resources();
    let ordinary = resources.charge(context.scope.group, 256 * 1024 - resources.counts().0, ResourceKind::Retained).unwrap();
    let mut pending = f._source.prepare_voice_sources().unwrap();
    assert_eq!(pending.len(), 1, "prepaid lower work includes Root route metadata before spending an attempt");
    assert_eq!(resources.counts().0, 256 * 1024);
    assert!(resources.charge(context.scope.group, 1, ResourceKind::Retained).is_err());
    let (_publisher, signal) = tokio::sync::watch::channel(false);
    let (completed, ()) = tokio::join!(pending.pop().unwrap().execute(signal), async {
        loop {
            if let Some(TransportEvent::Request { from, data, responder, .. }) = f.helper.transport.next_event().await {
                let (inner, ..) = decode_authed_request(&data[1..]).unwrap();
                let envelope = voice_v2::Envelope::decode(&inner).unwrap();
                assert_eq!(envelope.object(), &object);
                f.helper.handle_voice_transport_request(from, &data, responder);
                break;
            }
        }
    });
    assert_eq!(f._source.complete_voice_source(completed).unwrap(), (crate::VoiceTransportStatus::HelperQueued, voice_v2::source::SourceOutcome::Pending));
    assert_eq!(f._source.voice_transport_actor().unwrap().source().unwrap().counts(), (1, 1, 0));
    drop(ordinary);
}

#[tokio::test]
async fn root_prepaid_sdp_routes_keep_two_actual_lowers_charged_after_cancellation() {
    use voice_v2::resources::ResourceKind;
    let mut f = VoiceForwardFixture::empty().await;
    let (_join, object) = queued_source_offer(&mut f);
    let resources = f._endpoints[0].resources();
    let group = object.scope().group;
    let ordinary = resources.charge(group, 256 * 1024 - resources.counts().0, ResourceKind::Retained).unwrap();
    let first = f._source.prepare_voice_sources().unwrap().pop().unwrap();
    f.clock.advance_ms(500);
    let second = f._source.prepare_voice_sources().unwrap().pop().unwrap();
    let source = f._source.voice_transport_actor().unwrap().source().unwrap();
    assert_eq!(source.counts(), (1, 2, 2));
    f.clock.advance_ms(500);
    assert!(f._source.prepare_voice_sources().unwrap().is_empty());
    assert_eq!(source.counts(), (1, 2, 2), "two retained lower loans leave no third phase or attempt");
    let (cancel_first, signal_first) = tokio::sync::watch::channel(false);
    let (cancel_second, signal_second) = tokio::sync::watch::channel(false);
    let (first, second, responders) = tokio::join!(first.execute(signal_first), second.execute(signal_second), async {
        let mut responders = Vec::new();
        while responders.len() < 2 {
            if let Some(TransportEvent::Request { from, data, responder, .. }) = f.helper.transport.next_event().await {
                let (inner, ..) = decode_authed_request(&data[1..]).unwrap();
                assert_eq!(voice_v2::Envelope::decode(&inner).unwrap().object(), &object);
                assert!(f.helper.receive_voice_transport(from, &data).is_some());
                responders.push(responder);
            }
        }
        cancel_first.send_replace(true);
        cancel_second.send_replace(true);
        responders
    });
    assert!(f._source.complete_voice_source(first).is_err());
    assert!(f._source.complete_voice_source(second).is_err());
    assert_eq!(source.counts(), (1, 2, 2), "upper cancellation and completions cannot refund actual lower custody");
    drop(ordinary);
    f._endpoints[0].revoke_session();
    drop(_join); // closes unused exchange headroom; actual lower phase loans remain owned
    let retained = resources.counts().0;
    drop(responders);
    assert_eq!(source.counts(), (1, 2, 0));
    assert!(resources.counts().0 < retained, "route metadata and both lower phase/lane charges retire with the real responders");
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
            node.handle_voice_transport_request(from, &data, responder);
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
