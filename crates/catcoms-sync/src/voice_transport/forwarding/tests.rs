use super::*;
use catcoms_rt::{Clock, ManualClock, RngCore};
use std::future::Future;
use voice_v2::resources::{ResourceKind, GROUP_BYTES};
use voice_v2::runtime::Runtime;

type Node = ChannelSync<catcoms_rt::mem::MemNetwork, rand_chacha::ChaCha20Rng>;
type Fixture = (Node, Node, Arc<Runtime>, Arc<ManualClock>);

async fn bounded<T>(what: &'static str, future: impl Future<Output = T>) -> T {
    let clock = catcoms_rt::SystemClock;
    tokio::select! {
        value = future => value,
        _ = clock.sleep(std::time::Duration::from_secs(30)) => panic!("route custody regression stalled: {what}"),
    }
}
fn attach(node: &mut Node, runtime: &Runtime) {
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    node.prepare_voice_actor(
        runtime,
        &ticket,
        991,
        |_| Ok(BTreeSet::from([1])),
        |_| Ok(BTreeSet::new()),
    )
    .unwrap()
    .register()
    .unwrap();
}
async fn fixture() -> Fixture {
    let (_, mut nodes, _) = crate::tests::build_members(2).await;
    crate::tests::converge_and_publish_test_routes(&mut nodes);
    nodes[0]
        .initialize_group_policy(GroupMode::PeerToPeer)
        .unwrap();
    nodes[0].snapshot().unwrap();
    nodes[0].publish_group_policy().unwrap();
    let policy = nodes[0].group_policy.as_ref().unwrap().encode();
    nodes[1].on_group_policy(&policy);
    let mut remote = nodes.pop().unwrap();
    let mut node = nodes.pop().unwrap();
    assert!(node.policy_allows_member_mesh() && remote.policy_allows_member_mesh());
    let clock = Arc::new(ManualClock::new(1000));
    node.clock = clock.clone();
    remote.clock = clock.clone();
    let (connected, served) = bounded("actual member finalization", async {
        tokio::join!(
            node.finalize_member_connection(remote.local_peer()),
            remote.run_once()
        )
    })
    .await;
    assert!(connected.unwrap() && served.unwrap());
    let runtime = Runtime::new(clock.clone());
    attach(&mut node, &runtime);
    // Finish actual current-view/directory installation before measuring charges.
    drop(node.voice_endpoint_changes().unwrap());
    (node, remote, runtime, clock)
}
fn key() -> [u8; 65] {
    hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()
}
struct ResumeOnDrop(Option<std::sync::mpsc::Sender<()>>);
impl ResumeOnDrop {
    fn release(mut self) {
        self.0.take().unwrap().send(()).unwrap();
    }
}
impl Drop for ResumeOnDrop {
    fn drop(&mut self) {
        if let Some(resume) = self.0.take() {
            let _ = resume.send(());
        }
    }
}
fn pause_gate(gate: &Arc<RouteGate>) -> (tokio::sync::oneshot::Receiver<()>, ResumeOnDrop) {
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    *gate.before_deallocation.lock().unwrap() = Some((entered, resumed));
    (waiting, ResumeOnDrop(Some(resume)))
}
fn drop_in_thread<T: Send + 'static>(value: T) -> tokio::sync::oneshot::Receiver<()> {
    let (done, finished) = tokio::sync::oneshot::channel();
    let _thread = std::thread::spawn(move || {
        drop(value);
        let _ = done.send(());
    });
    finished
}
fn source_pending(
    node: &mut Node,
) -> (
    Arc<voice_v2::CallGrant>,
    PendingVoiceSource<catcoms_rt::mem::MemNetwork>,
) {
    let grant = Arc::new(node.voice_call_grant(1).unwrap());
    node.start_voice_endpoint(grant.clone(), [22; 32], key())
        .unwrap();
    node.pump_voice_endpoint().unwrap();
    let mut pending = node.prepare_voice_sources().unwrap();
    assert_eq!(pending.len(), 1);
    (grant, pending.pop().unwrap())
}

#[tokio::test]
async fn route_retired_root_unexecuted_pending_and_awaited_driver_cancel_keep_original_payer_last()
{
    for driver in [false, true] {
        let (mut node, _remote, runtime, _clock) = fixture().await;
        let (_grant, pending) = source_pending(&mut node);
        let (entered, release) = pause_gate(&pending.0.authority.route);
        let finished;
        let charged;
        if driver {
            let (_signal, receive) = tokio::sync::watch::channel(false);
            // The exact public driver object with the SAME actual Source Flight
            // and native route authority used by Pending::execute, not a stand-in.
            let mut cancellation =
                RequestCancellation::new(receive, Some(pending.0.job.keepalive()))
                    .with_authority(pending.0.authority.clone());
            let mut waiting = Box::pin(cancellation.cancelled());
            std::future::poll_fn(|cx| {
                assert!(waiting.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            node.retire_voice_actor();
            bounded("actual driver cancellation after Root retirement", waiting).await;
            drop(pending);
            charged = runtime.resources().counts();
            finished = drop_in_thread(cancellation);
        } else {
            node.retire_voice_actor();
            charged = runtime.resources().counts();
            finished = drop_in_thread(pending);
        }
        bounded("retired Root final external route Drop", entered)
            .await
            .unwrap();
        assert_eq!(
            runtime.resources().counts(),
            charged,
            "same external preparation/driver payer remains after internal Gate custody drops"
        );
        release.release();
        bounded("retired Root final external return", finished)
            .await
            .unwrap();
        assert!(runtime.resources().counts().0 < charged.0);
    }
}

fn refusal_envelope(source: &Node, receiver: &mut Node) -> voice_v2::Envelope {
    let scope = receiver.voice_v2_context(1).unwrap().scope;
    voice_v2::Envelope::Direct(
        voice_v2::SignedObject::issue(
            scope,
            &source.device,
            [31; 32],
            1000,
            16_000,
            voice_v2::Body::WatchChallenge {
                watcher: [32; 32],
                target: receiver.device_id(),
                join: [33; 32],
                key_ref: [34; 32],
                challenge: [35; 32],
            },
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn route_retired_root_prepared_driver_reply_and_bytes_clone_keep_verification_payer_last() {
    for mode in [0, 1, 2] {
        let (mut node, mut remote, runtime, _clock) = fixture().await;
        node.set_voice_routing(crate::VoiceRouting::Forwarding);
        let envelope = refusal_envelope(&remote, &mut node);
        let request = frame(&mut remote, &envelope);
        let (responder, receive) = catcoms_rt::Responder::channel();
        let reply = node
            .prepare_voice_transport_reply(
                remote.local_peer(),
                &request,
                Some(responder.cancellation_authority()),
            )
            .unwrap();
        let gate = node
            .voice_runtime
            .as_ref()
            .unwrap()
            .routes
            .active
            .last()
            .unwrap()
            .gate();
        let (entered, release) = pause_gate(gate);
        let finished;
        if mode == 2 {
            node.retire_voice_actor();
            // The ordinary refused public response path must retain payment too.
            let (done, returned) = tokio::sync::oneshot::channel();
            let _thread = std::thread::spawn(move || {
                assert!(matches!(
                    reply.respond(responder),
                    Err(TransportError::Cancelled)
                ));
                let _ = done.send(());
            });
            finished = returned;
            drop(receive);
        } else {
            reply.respond(responder).unwrap();
            let driver_reply = bounded("actual driver response admission", receive.recv_owned())
                .await
                .unwrap();
            node.retire_voice_actor();
            assert!(!driver_reply.is_current());
            if mode == 0 {
                finished = drop_in_thread(driver_reply);
            } else {
                // A real byte-owner clone can outlive the driver envelope. Keep
                // the actual Bytes owner; copying to Vec would erase the oracle.
                let bytes = driver_reply.data().clone();
                drop(driver_reply);
                finished = drop_in_thread(bytes);
            }
        }
        assert_eq!(runtime.resources().counts().1, 1);
        bounded("retired Root reply final route Drop", entered)
            .await
            .unwrap();
        assert_eq!(
            runtime.resources().counts().1,
            1,
            "original verification credit remains through final Gate/Arc destruction"
        );
        release.release();
        bounded("retired Root reply actual return", finished)
            .await
            .unwrap();
        assert_eq!(runtime.resources().counts().1, 0);
    }
}

#[tokio::test]
async fn route_registry_retains_actual_source_payment_through_paused_final_arc_destruction() {
    let (mut node, _remote, runtime, _clock) = fixture().await;
    // The native grant is affine: retain its actual owner, rather than letting
    // argument destruction End the call before the real source is prepared.
    let grant = Arc::new(node.voice_call_grant(1).unwrap());
    node.start_voice_endpoint(grant.clone(), [22; 32], key())
        .unwrap();
    node.pump_voice_endpoint().unwrap();
    let mut pending = node.prepare_voice_sources().unwrap();
    assert_eq!(pending.len(), 1);
    let pending = pending.pop().unwrap();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    let release = ResumeOnDrop(Some(resume));
    *pending
        .0
        .authority
        .route
        .before_deallocation
        .lock()
        .unwrap() = Some((entered, resumed));
    let charged = runtime.resources().counts();
    assert_eq!(charged.1, 0);
    drop(pending);
    // Registry deliberately owns the last Arc. Detach the SAME real production
    // row, then pause after Gate's internal custody drops but before Arc free.
    let row = node
        .voice_runtime
        .as_mut()
        .unwrap()
        .routes
        .take_unused()
        .unwrap();
    let (done, finished) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        drop(row);
        let _ = done.send(());
    });
    bounded("paused actual RouteGate destructor", waiting)
        .await
        .unwrap();
    node.prune_voice_routes(); // Concurrent old Weak-count pruning was unsafe here.
    assert_eq!(
        runtime.resources().counts(),
        charged,
        "outside row payer survives internal custody destruction and final Arc tail"
    );
    release.release();
    bounded("actual detached row return", finished)
        .await
        .unwrap();
    assert!(
        runtime.resources().counts().0 < charged.0,
        "original Lower credit returns only after actual Gate allocation destruction"
    );
}

fn frame(node: &mut Node, envelope: &voice_v2::Envelope) -> Vec<u8> {
    let inner = envelope.encode();
    let key = node.device.public_key_bytes();
    let mut nonce = [0; 16];
    node.rng.fill_bytes(&mut nonce);
    let auth = RequestAuth {
        ts: node.voice_v2_clock().now_ms(),
        nonce,
        epoch: node.group.epoch(),
    };
    let transcript = catchup_auth_transcript(
        &node.group.group_id(),
        KIND,
        &inner,
        &key,
        auth.ts,
        &auth.nonce,
        auth.epoch,
        Some(&node.local_peer()),
    );
    let signature = node.device.sign(&transcript).unwrap();
    encode_authed_request_frame(
        KIND,
        &inner,
        &key,
        auth.ts,
        &auth.nonce,
        auth.epoch,
        &signature,
    )
}
async fn exchange_result(
    from: &Node,
    receiver: &mut Node,
    request: &[u8],
) -> Result<Bytes, TransportError> {
    let transport = from.transport.clone();
    let peer = receiver.local_peer();
    bounded("actual kind28 request/response", async {
        let request = transport.request_connected(
            peer,
            ProtocolId(RR_PROTOCOL),
            Bytes::copy_from_slice(request),
        );
        let driver = async {
            loop {
                assert!(receiver.run_once().await.unwrap());
            }
        };
        match futures::future::select(Box::pin(request), Box::pin(driver)).await {
            futures::future::Either::Left((response, _)) => response,
            futures::future::Either::Right(((), _)) => panic!("Root driver stopped"),
        }
    })
    .await
}
async fn exchange(from: &Node, receiver: &mut Node, request: &[u8]) -> Bytes {
    exchange_result(from, receiver, request).await.unwrap()
}

#[tokio::test]
async fn route_registry_quiet_collection_releases_shared_process_crypto_for_another_root() {
    let (mut first, mut source_first, runtime, clock) = fixture().await;
    let (mut second, mut source_second, _old_second_runtime, _second_clock) = fixture().await;
    let (mut third, mut source_third, _old_third_runtime, _third_clock) = fixture().await;
    second.retire_voice_actor();
    third.retire_voice_actor();
    attach(&mut second, &runtime);
    attach(&mut third, &runtime);
    for node in [&mut first, &mut second, &mut third] {
        node.set_voice_routing(crate::VoiceRouting::Forwarding);
    }
    let scope = first.voice_v2_context(1).unwrap().scope;
    assert_ne!(scope.group, second.voice_v2_context(1).unwrap().scope.group);
    assert_ne!(scope.group, third.voice_v2_context(1).unwrap().scope.group);
    assert!(Arc::ptr_eq(
        &first.voice_runtime.as_ref().unwrap().resources,
        &second.voice_runtime.as_ref().unwrap().resources
    ));
    assert!(Arc::ptr_eq(
        &first.voice_runtime.as_ref().unwrap().resources,
        &third.voice_runtime.as_ref().unwrap().resources
    ));
    let _harness = runtime
        .resources()
        .charge(scope.group, 16 * 1024, ResourceKind::Retained)
        .unwrap();
    let first_envelope = refusal_envelope(&source_first, &mut first);
    let second_envelope = refusal_envelope(&source_second, &mut second);
    let third_envelope = refusal_envelope(&source_third, &mut third);
    let mut held = Vec::with_capacity(8);
    for _ in 0..4 {
        let request = frame(&mut source_first, &first_envelope);
        held.push(exchange(&source_first, &mut first, &request).await);
        let request = frame(&mut source_second, &second_envelope);
        held.push(exchange(&source_second, &mut second, &request).await);
    }
    assert_eq!(runtime.resources().counts().1, 8);
    drop(held);
    assert_eq!(
        runtime.resources().counts().1,
        8,
        "both idle registries still sponsor their final route metadata"
    );
    let request = frame(&mut source_third, &third_envelope);
    assert!(matches!(
        exchange_result(&source_third, &mut third, &request).await,
        Err(TransportError::NoResponse)
    ));
    clock.advance_ms(1001);
    // Only actual quiet housekeeping on the OTHER two Roots runs here. Third
    // Root's own ingress pruning cannot release their shared process lanes.
    first.collect_expired_voice();
    second.collect_expired_voice();
    assert_eq!(runtime.resources().counts().1, 0);
    let request = frame(&mut source_third, &third_envelope);
    let response = exchange(&source_third, &mut third, &request).await;
    assert!(!response.is_empty());
    assert_eq!(runtime.resources().counts().1, 1);
}

#[tokio::test]
async fn route_registry_idle_unjoined_root_reclaims_crypto_before_admitting_next_request() {
    let (mut receiver, mut source, runtime, clock) = fixture().await;
    let source_runtime = Runtime::new(clock);
    attach(&mut source, &source_runtime);
    receiver.set_voice_routing(crate::VoiceRouting::Forwarding);
    let scope = receiver.voice_v2_context(1).unwrap().scope;
    let object = voice_v2::SignedObject::issue(
        scope,
        &source.device,
        [31; 32],
        1000,
        16_000,
        voice_v2::Body::WatchChallenge {
            watcher: [32; 32],
            target: receiver.device_id(),
            join: [33; 32],
            key_ref: [34; 32],
            challenge: [35; 32],
        },
    )
    .unwrap();
    let envelope = voice_v2::Envelope::Direct(object.clone());
    // Explicit harness payment covers the held request/reply buffers and vector;
    // it is not a production claim of more than the existing four Crypto lanes.
    let _harness = runtime
        .resources()
        .charge(scope.group, 8192, ResourceKind::Retained)
        .unwrap();
    let mut responses = Vec::with_capacity(4);
    for _ in 0..4 {
        let request = frame(&mut source, &envelope);
        let response = exchange(&source, &mut receiver, &request).await;
        assert_eq!(
            source.verify_voice_transport_response(
                receiver.local_peer(),
                &request,
                object.hash(),
                &response
            ),
            Ok(Status::Refused)
        );
        responses.push(response);
    }
    assert_eq!(
        runtime.resources().counts().1,
        4,
        "real driver response owners keep all four original verification credits"
    );
    for _ in 0..12 {
        drop(responses.pop().unwrap());
        assert_eq!(
            runtime.resources().counts().1,
            4,
            "sole registry row still owns that SAME verification payer"
        );
        let request = frame(&mut source, &envelope);
        let response = exchange(&source, &mut receiver, &request).await;
        assert_eq!(
            source.verify_voice_transport_response(
                receiver.local_peer(),
                &request,
                object.hash(),
                &response
            ),
            Ok(Status::Refused)
        );
        responses.push(response);
        assert_eq!(runtime.resources().counts().1, 4);
        let endpoints = &receiver.voice_runtime.as_ref().unwrap().endpoints;
        assert!(endpoints.participant_join_for_test().is_none());
        assert_eq!(endpoints.observer_count_for_test(), 0);
        assert!(!receiver.has_admitted_voice_activation());
    }
    drop(responses);
    receiver.prune_voice_routes();
    assert_eq!(runtime.resources().counts().1, 0);
}

#[tokio::test]
async fn route_registry_structural_shared_capacity_counts_all_sponsored_rows_and_rolls_over() {
    let (mut node, _remote, runtime, _clock) = fixture().await;
    let route = node.voice_bound_routes()[0];
    let scope = node.voice_v2_context(1).unwrap().scope;
    // Structural capacity oracle with actual bound-route factories and distinct
    // real metadata sponsors. It does not claim64 feasible flights/native calls.
    let _harness = runtime
        .resources()
        .charge(scope.group, 4096, ResourceKind::Retained)
        .unwrap();
    let mut held = Vec::with_capacity(ROUTES);
    for _ in 0..ROUTES {
        let credit = runtime
            .resources()
            .charge(
                scope.group,
                voice_v2::ROUTE_METADATA_BYTES,
                ResourceKind::Retained,
            )
            .unwrap();
        held.push(
            node.capture_voice_reply_route(route, Arc::new(credit))
                .unwrap(),
        );
    }
    assert_eq!(node.available_voice_routes().unwrap(), 0);
    let credit = runtime
        .resources()
        .charge(
            scope.group,
            voice_v2::ROUTE_METADATA_BYTES,
            ResourceKind::Retained,
        )
        .unwrap();
    assert_eq!(
        node.capture_voice_reply_route(route, Arc::new(credit))
            .err(),
        Some(VoiceError::Capacity)
    );
    drop(held.pop());
    assert_eq!(node.available_voice_routes().unwrap(), 1);
    let credit = runtime
        .resources()
        .charge(
            scope.group,
            voice_v2::ROUTE_METADATA_BYTES,
            ResourceKind::Retained,
        )
        .unwrap();
    held.push(
        node.capture_voice_reply_route(route, Arc::new(credit))
            .unwrap(),
    );
    assert_eq!(node.available_voice_routes().unwrap(), 0);
    drop(held);
    assert_eq!(node.available_voice_routes().unwrap(), ROUTES);
    assert_eq!(
        node.voice_runtime
            .as_ref()
            .unwrap()
            .routes
            .active
            .capacity(),
        ROUTES
    );
}

#[tokio::test]
async fn route_registry_first_attachment_capacity_and_stale_ticket_roll_back_before_publication() {
    for capacity in [true, false] {
        let (_, mut nodes, _) = crate::tests::build_members(1).await;
        let node = &mut nodes[0];
        let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
        let ticket = runtime.session_ticket();
        runtime.activate_session(&ticket).unwrap();
        let group = voice_v2::Scope::for_group(&node.group.group_id(), None, 0).group;
        let pressure = if capacity {
            Some(
                runtime
                    .resources()
                    .charge(
                        group,
                        GROUP_BYTES - REGISTRY_BYTES + 1,
                        ResourceKind::Retained,
                    )
                    .unwrap(),
            )
        } else {
            runtime.revoke_session();
            None
        };
        let before = runtime.resources().counts();
        assert_eq!(
            node.prepare_voice_actor(
                &runtime,
                &ticket,
                991,
                |_| Ok(BTreeSet::from([1])),
                |_| Ok(BTreeSet::new())
            )
            .err(),
            Some(if capacity {
                VoiceError::Capacity
            } else {
                VoiceError::Unauthorized
            })
        );
        assert!(node.voice_runtime.is_none());
        assert_eq!(runtime.resources().counts(), before,
            "registry fee/backing actually returned; no Runtime group was entered in these refusal paths");
        drop(pressure);
    }
}

#[test]
fn route_registry_complete_backing_and_inline_growth_fit_prepaid_metadata() {
    use std::mem::size_of;
    let graph = RouteOwners::graph_bytes();
    let route = RouteOwners::route_metadata_bytes();
    eprintln!("route registry {graph}/{REGISTRY_BYTES}; per-route {route}/{}; RouteOwners {}; previous Vec24 {}; VoiceRuntime {}",
        voice_v2::ROUTE_METADATA_BYTES, size_of::<RouteOwners>(), size_of::<Vec<usize>>(), size_of::<crate::voice_runtime::VoiceRuntime>());
    assert!(graph <= REGISTRY_BYTES);
    assert!(route <= voice_v2::ROUTE_METADATA_BYTES);
}

#[tokio::test]
async fn route_registry_actual_pending_and_completion_wrappers_fit_original_lower_overhead() {
    use std::mem::{size_of, size_of_val};
    let (mut node, _remote, _runtime, _clock) = fixture().await;
    let (_grant, pending) = source_pending(&mut node);
    let (_signal, receive) = tokio::sync::watch::channel(false);
    let execution = pending.execute(receive);
    // Actual opaque native execution body plus its completed output, enclosing
    // scheduler discriminant/box and bounded ACK/metadata overlap. Serialized
    // packet copies and the transport's independently owned driver graph retain
    // their original separate buffer/driver accounting; do not hide them here.
    let graph = size_of_val(&execution)
        + size_of::<CompletedVoiceForward>()
        + size_of::<crate::voice_schedule::VoiceFlight>()
        + size_of::<futures::future::BoxFuture<'static, crate::voice_schedule::VoiceFlight>>()
        + 512
        + 256;
    eprintln!(
        "actual route execution/completion/scheduler graph {graph}/4096; execution {}",
        size_of_val(&execution)
    );
    assert!(graph <= 4096);
    drop(execution);
    node.prune_voice_routes();
}
