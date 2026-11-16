//! Actual Root discovery/confirmation/admission across the permitted member route. This
//! fixture mocks AEAD outputs, never native Pair/Finish/Ready/receipt state. Mem evidence
//! does not establish physical A-C exclusion, browser SDP/RTC or audible media.
use super::*;
use crate::voice_endpoint::EndpointReceipt;
use voice_v2::crypto_actions::{CryptoAction, CryptoInput, CryptoOperation};
use voice_v2::presence::{PairSelector, VoicePairConfirmation};
use voice_v2::{Body, CallGrant, EncryptedKind, Envelope, SignedObject};

fn pending(node: &mut VoiceTestNode, grant: &Arc<CallGrant>) -> Vec<Arc<CryptoAction>> {
    let permit = voice_v2::crypto_actions::CryptoActionSnapshotPermit::new(
        &node.voice_runtime.as_ref().unwrap().resources,
        grant.scope(),
    )
    .unwrap();
    let snapshot = node.voice_confirmation_actions(grant, permit);
    snapshot
        .result()
        .unwrap()
        .iter()
        .filter(|action| action.is_pending())
        .cloned()
        .collect()
}
fn derive(node: &mut VoiceTestNode, grant: &Arc<CallGrant>, handshake: [u8; 32]) {
    let action = node.prepare_voice_derivation(grant, handshake).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let result = node
        .commit_voice_derivation_result(grant, &action, action.output_tag(&[]).unwrap())
        .unwrap();
    action.settle_engine();
    action.acknowledge_result(&result).unwrap();
}
fn seal(node: &mut VoiceTestNode, grant: &Arc<CallGrant>, action: Arc<CryptoAction>) -> Vec<u8> {
    let input = action.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("native seal")
    };
    let plain = flight.plaintext.clone();
    let ciphertext = vec![23; plain.len() + 16];
    drop(input);
    action.admit_engine(action.engine_bytes()).unwrap();
    let result = node
        .submit_voice_confirmation_result(
            grant,
            &action,
            &ciphertext,
            action.output_tag(&ciphertext).unwrap(),
        )
        .unwrap();
    action.settle_engine();
    action.acknowledge_result(&result).unwrap();
    plain
}
fn open(
    node: &mut VoiceTestNode,
    grant: &Arc<CallGrant>,
    action: &Arc<CryptoAction>,
    plain: &[u8],
) {
    action.admit_engine(action.engine_bytes()).unwrap();
    let result = node
        .commit_voice_confirmation_result(grant, action, plain, action.output_tag(plain).unwrap())
        .unwrap();
    action.settle_engine();
    action.acknowledge_result(&result).unwrap();
}
async fn incoming(
    f: &mut BridgeFixture,
    grants: &[Arc<CallGrant>; 2],
    at: usize,
    kind: EncryptedKind,
) -> Arc<CryptoAction> {
    for _ in 0..80 {
        f.tick(3).await;
        let node = if at == 0 { &mut f.a } else { &mut f.c };
        if let Some(action)=pending(node,&grants[at]).into_iter().find(|action| {
            action.operation()==CryptoOperation::DecryptConfirmation
                && matches!(action.input().unwrap().as_input(),CryptoInput::DecryptConfirmation(packet) if packet.header.kind==kind)
        }) {return action;}
        f.clock.advance_ms(25);
    }
    panic!("actual Root {at} never received {kind:?}");
}
#[path = "ready_tests/sdp.rs"]
mod sdp;

async fn confirmed() -> (BridgeFixture, [Arc<CallGrant>; 2], [u8; 32], usize) {
    let (fixture, grants, handshake, low, _) = confirmed_with_custody().await;
    (fixture, grants, handshake, low)
}
type ConfirmedFixture = (
    BridgeFixture,
    [Arc<CallGrant>; 2],
    [u8; 32],
    usize,
    [voice_v2::chain::EstablishedKeyCustody; 2],
);
async fn confirmed_with_custody() -> ConfirmedFixture {
    let fixture = Box::new(Box::pin(BridgeFixture::new()).await);
    Box::pin(confirmed_with_custody_using(fixture)).await
}
async fn confirmed_with_custody_using(mut f: Box<BridgeFixture>) -> ConfirmedFixture {
    let grants = [
        Arc::new(f.a.voice_call_grant(1).unwrap()),
        Arc::new(f.c.voice_call_grant(1).unwrap()),
    ];
    let keys = (public_join_key(141), public_join_key(142));
    f.a.start_voice_endpoint(grants[0].clone(), keys.0.join, keys.0.public_key)
        .unwrap();
    f.c.start_voice_endpoint(grants[1].clone(), keys.1.join, keys.1.public_key)
        .unwrap();
    assert!(f.converge(40, &keys).await);
    let epoch = f.a.voice_v2_context(1).unwrap().epoch;
    let pairs = [
        f.a.prepare_voice_pair(
            &grants[0],
            PairSelector {
                peer: f.c.device_id(),
                join: keys.1.join,
                key_ref: keys.1.key_ref,
                epoch,
            },
        )
        .unwrap(),
        f.c.prepare_voice_pair(
            &grants[1],
            PairSelector {
                peer: f.a.device_id(),
                join: keys.0.join,
                key_ref: keys.0.key_ref,
                epoch,
            },
        )
        .unwrap(),
    ];
    assert_eq!(pairs[0].handshake, pairs[1].handshake);
    let handshake = pairs[0].handshake;
    derive(&mut f.a, &grants[0], handshake);
    derive(&mut f.c, &grants[1], handshake);
    let mut custody: [Option<voice_v2::chain::EstablishedKeyCustody>; 2] =
        std::array::from_fn(|_| None);
    let low = usize::from(!pairs[0].local_is_low);
    let high = 1 - low;
    let low_node = if low == 0 { &mut f.a } else { &mut f.c };
    let hello = low_node
        .start_voice_confirmation(&grants[low], handshake, None)
        .unwrap();
    let plain = seal(low_node, &grants[low], hello);
    let action = incoming(&mut f, &grants, high, EncryptedKind::ConfirmHello).await;
    let high_node = if high == 0 { &mut f.a } else { &mut f.c };
    open(high_node, &grants[high], &action, &plain);
    let proof = high_node
        .respond_voice_confirmation(&grants[high], handshake, &action)
        .unwrap();
    let plain = seal(high_node, &grants[high], proof);
    drop(action);
    let action = incoming(&mut f, &grants, low, EncryptedKind::ConfirmProof).await;
    let low_node = if low == 0 { &mut f.a } else { &mut f.c };
    open(low_node, &grants[low], &action, &plain);
    let finish = low_node
        .respond_voice_confirmation(&grants[low], handshake, &action)
        .unwrap();
    let plan = finish.protected_plan().unwrap();
    let control = finish.confirmation_control().unwrap();
    let plain = seal(low_node, &grants[low], finish);
    let mono = f.clock.monotonic_ms();
    let finite = plan.confirmation_custody(&control, mono).unwrap();
    custody[low] = Some(plan.established_key_custody(&finite, mono).unwrap());
    drop((finite, control, plan));
    drop(action);
    let action = incoming(&mut f, &grants, high, EncryptedKind::ConfirmFinish).await;
    let high_node = if high == 0 { &mut f.a } else { &mut f.c };
    let plan = action.protected_plan().unwrap();
    let control = action.confirmation_control().unwrap();
    open(high_node, &grants[high], &action, &plain);
    let mono = f.clock.monotonic_ms();
    let finite = plan.confirmation_custody(&control, mono).unwrap();
    custody[high] = Some(plan.established_key_custody(&finite, mono).unwrap());
    drop((finite, control, plan));
    drop(action);
    f.tick(8).await;
    assert_eq!(
        f.a.voice_pair_confirmation(
            &grants[0],
            PairSelector {
                peer: f.c.device_id(),
                join: keys.1.join,
                key_ref: keys.1.key_ref,
                epoch
            },
            handshake
        ),
        Ok(VoicePairConfirmation::Established)
    );
    assert_eq!(
        f.c.voice_pair_confirmation(
            &grants[1],
            PairSelector {
                peer: f.a.device_id(),
                join: keys.0.join,
                key_ref: keys.0.key_ref,
                epoch
            },
            handshake
        ),
        Ok(VoicePairConfirmation::Established)
    );
    (
        *f,
        grants,
        handshake,
        low,
        [custody[0].take().unwrap(), custody[1].take().unwrap()],
    )
}
fn admit(node: &mut VoiceTestNode, envelope: Envelope) -> Result<EndpointReceipt, VoiceError> {
    let context = node.voice_v2_context(1)?;
    let room = node.voice_room_authority(1)?;
    let requester = match &envelope {
        Envelope::HelperDelivery { permit, .. } => permit.helper(),
        _ => envelope.object().issuer(),
    };
    let ticket = node
        .voice_transport_actor()?
        .admit_transport_verification(requester, &envelope.encode())?;
    node.admit_voice_endpoint(ticket, envelope, &context, room)
}

#[tokio::test]
async fn root_connected_ready_selects_unjoined_bound_helper_and_preserves_it_across_churn() {
    let (mut f, grants, handshake, low) = confirmed().await;
    let high = 1 - low;
    let helper = f.b.as_ref().unwrap().device_id();
    let mut prepared = false;
    for _ in 0..80 {
        let node = if high == 0 { &mut f.a } else { &mut f.c };
        match node.prepare_voice_high_ready_connected(&grants[high], handshake) {
            Ok(true) => {
                prepared = true;
                break;
            }
            Err(VoiceError::Capacity) => {}
            other => panic!("original connected Ready preparation refused: {other:?}"),
        }
        f.tick(3).await;
        f.clock.advance_ms(25);
    }
    assert!(prepared);
    let object = {
        let node = if high == 0 { &mut f.a } else { &mut f.c };
        assert!(!node
            .queue_voice_high_ready(&grants[high], handshake)
            .unwrap());
        let actor = node.voice_transport_actor().unwrap();
        let jobs = actor
            .source()
            .unwrap()
            .take_ready(actor, &node.voice_bound_routes())
            .unwrap();
        jobs.iter()
            .find(|job| matches!(job.object().body(), Body::PairReadyRound { .. }))
            .expect("actual native Ready Source")
            .object()
            .clone()
    };
    let Body::PairReadyRound { header, .. } = object.body() else {
        unreachable!();
    };
    assert_eq!(
        header.helpers,
        vec![helper],
        "unbound members and endpoints are not helper candidates"
    );
    assert!(f
        .b
        .as_ref()
        .unwrap()
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .implicit_watch_for_test()
        .is_none());
    drop(f.b.take());
    bind_nodes(&mut f.a, &mut f.d).await;
    bind_nodes(&mut f.d, &mut f.c).await;
    let node = if high == 0 { &mut f.a } else { &mut f.c };
    assert!(node
        .prepare_voice_high_ready_connected(&grants[high], handshake)
        .unwrap());
    assert!(
        node.queue_voice_high_ready(&grants[high], handshake)
            .unwrap(),
        "new connections recover the SAME first Ready, never a replacement Source budget"
    );
}

#[tokio::test]
async fn root_ready_retries_do_not_rebuild_roster_or_directory_and_stale_epoch_refuses() {
    let (mut f, grants, handshake, low) = confirmed().await;
    let high = 1 - low;
    let counters = |node: &VoiceTestNode| {
        let runtime = node.voice_runtime.as_ref().unwrap();
        (runtime.view_builds, runtime.directory_reads)
    };
    let before = [counters(&f.a), counters(&f.c)];
    for _ in 0..3 {
        let high_node = if high == 0 { &mut f.a } else { &mut f.c };
        match high_node.prepare_voice_high_ready_connected(&grants[high], handshake) {
            Ok(true) | Err(VoiceError::Capacity) => {}
            other => panic!("original HIGH Ready retry refused: {other:?}"),
        }
        match high_node.queue_voice_high_ready(&grants[high], handshake) {
            Ok(_) | Err(VoiceError::Unauthorized | VoiceError::Capacity) => {}
            other => panic!("original HIGH queue retry refused: {other:?}"),
        }
        let low_node = if low == 0 { &mut f.a } else { &mut f.c };
        match low_node.prepay_voice_low_sdp(&grants[low], handshake) {
            Ok(()) | Err(VoiceError::Capacity) => {}
            other => panic!("original LOW prepayment refused: {other:?}"),
        }
        assert_eq!(
            low_node.prepare_voice_low_sdp(&grants[low], handshake),
            Err(VoiceError::Unauthorized),
            "no remote Ready has been dispatched in this test"
        );
    }
    assert_eq!([counters(&f.a), counters(&f.c)], before,
        "even refused/retried native Ready operations must not clone the full roster or parse the directory");
    let removed = f.d.device_id();
    let node = if high == 0 { &mut f.a } else { &mut f.c };
    // Simulate an actual MLS epoch change before its normal finally-style reconciliation
    // hook runs. Validation-only must DENY the old Actor view, never refresh old authority.
    node.group.remove_member(&node.device, &removed).unwrap();
    let before = counters(node);
    assert_eq!(
        node.prepare_voice_high_ready_connected(&grants[high], handshake),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        node.queue_voice_high_ready(&grants[high], handshake),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(counters(node), before);
}

#[tokio::test]
async fn root_low_ready_shared_namespace_actual_success_retry_collision_and_delayed_transfer() {
    let (mut f, grants, handshake, low) = confirmed().await;
    let high = 1 - low;
    let helper = f.b.as_ref().unwrap().device_id();
    let mut prepared = false;
    for _ in 0..80 {
        let high_node = if high == 0 { &mut f.a } else { &mut f.c };
        match high_node.prepare_voice_high_ready(&grants[high], handshake, &[helper]) {
            Ok(true) => {
                prepared = true;
                break;
            }
            Err(VoiceError::Capacity) => {}
            other => panic!("original HIGH preparation refused: {other:?}"),
        }
        // Native IO/reply workers still own real capacity after confirmation observed.
        // Retry the same preparation only; never replace its identity or widen a deadline.
        f.tick(3).await;
        f.clock.advance_ms(25);
    }
    let high_node = if high == 0 { &mut f.a } else { &mut f.c };
    assert!(
        prepared,
        "HIGH actual reply/capacity did not drain: {:?}",
        high_node.voice_runtime.as_ref().unwrap().resources.counts()
    );
    high_node
        .queue_voice_high_ready(&grants[high], handshake)
        .unwrap();
    // Borrow the actual queued signed Ready. This consumes a real original Source attempt;
    // it constructs no pair, Finish marker, receipt, grant or signed packet substitute.
    let actor = high_node.voice_transport_actor().unwrap();
    let jobs = actor
        .source()
        .unwrap()
        .take_ready(actor, &high_node.voice_bound_routes())
        .unwrap();
    let object = jobs
        .iter()
        .find(|job| matches!(job.object().body(), Body::PairReadyRound { .. }))
        .expect("native queued Ready")
        .object()
        .clone();
    drop(jobs);
    let permit = voice_v2::RoutePermit::issue(
        &high_node.device,
        &object,
        helper,
        object.issued(),
        object.expires(),
    )
    .unwrap();
    let envelope = Envelope::HelperDelivery {
        object: object.clone(),
        permit,
    };
    let low_node = if low == 0 { &mut f.a } else { &mut f.c };
    low_node
        .prepay_voice_low_sdp(&grants[low], handshake)
        .unwrap();
    let positions = low_node.occupy_voice_receipt_namespace_for_test();
    assert!(!positions.is_empty());
    assert_eq!(
        admit(low_node, envelope.clone()).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(
        low_node.prepare_voice_low_sdp(&grants[low], handshake),
        Err(VoiceError::Unauthorized),
        "full shared namespace must not publish a causal marker"
    );
    drop(positions);
    let context = low_node.voice_v2_context(1).unwrap();
    let room = low_node.voice_room_authority(1).unwrap();
    let ticket = low_node
        .voice_transport_actor()
        .unwrap()
        .admit_transport_verification(helper, &envelope.encode())
        .unwrap();
    let reply = ticket.actual_work();
    let receipt = low_node
        .admit_voice_endpoint(ticket, envelope.clone(), &context, room)
        .unwrap();
    let EndpointReceipt::Ready(first) = receipt else {
        panic!("Ready must not produce a generic public receipt")
    };
    assert_eq!(
        low_node.prepare_voice_low_sdp(&grants[low], handshake),
        Err(VoiceError::Capacity)
    );
    assert_eq!(
        admit(low_node, envelope.clone()).err(),
        Some(VoiceError::Capacity),
        "the original physical reply still owns the sole verifier lane"
    );
    // A different signed control with the SAME issuer/ID cannot reuse the shared namespace.
    let high_node = if high == 0 { &mut f.a } else { &mut f.c };
    let epoch = high_node.group.epoch();
    let collision = SignedObject::issue(
        object.scope(),
        &high_node.device,
        object.id(),
        object.issued(),
        object.expires(),
        Body::Announcement {
            key: if high == 0 {
                public_join_key(141)
            } else {
                public_join_key(142)
            },
            epoch,
            renewal: 2,
        },
    )
    .unwrap();
    let low_node = if low == 0 { &mut f.a } else { &mut f.c };
    assert_eq!(
        admit(low_node, Envelope::Direct(collision)).err(),
        Some(VoiceError::RetryMismatch)
    );
    drop(reply);
    let EndpointReceipt::Ready(retry) = admit(low_node, envelope.clone()).unwrap() else {
        panic!("exact Ready retry after original reply drains")
    };
    assert!(Arc::ptr_eq(&first, &retry));
    low_node
        .voice_transport_actor()
        .unwrap()
        .source()
        .unwrap()
        .counts();
    assert!(low_node
        .prepare_voice_low_sdp(&grants[low], handshake)
        .unwrap());
    let EndpointReceipt::Ready(after) = admit(low_node, envelope).unwrap() else {
        panic!("post-transfer Ready recovery")
    };
    assert!(Arc::ptr_eq(&first, &after));
    assert!(
        f.b.as_ref()
            .unwrap()
            .voice_runtime
            .as_ref()
            .unwrap()
            .endpoints
            .implicit_watch_for_test()
            .is_none(),
        "member bridge joins nothing"
    );
}

fn frozen_ready_helpers(
    node: &VoiceTestNode,
    grant: &Arc<CallGrant>,
    handshake: [u8; 32],
) -> Option<[Option<DeviceId>; 2]> {
    let context = node.voice_v2_context(1).unwrap();
    let runtime = node.voice_runtime.as_ref().unwrap();
    let join = runtime.endpoints.participant_join_for_test().unwrap();
    let lease = grant.lease();
    runtime
        .current
        .as_ref()
        .unwrap()
        .try_with_current_members(&context, |member| {
            join.high_ready_helpers(
                handshake,
                &voice_v2::packets::Authority {
                    context: &context,
                    lease: &lease,
                    member,
                },
            )
        })
        .unwrap()
        .1
}

#[tokio::test]
async fn root_connected_ready_waits_for_original_authenticated_route_before_freezing_helpers() {
    let (mut f, grants, handshake, low) = confirmed().await;
    let high = 1 - low;
    let helper = f.b.as_ref().unwrap().device_id();
    {
        let node = if high == 0 { &mut f.a } else { &mut f.c };
        let peers = std::mem::take(&mut node.member_peers);
        let before = node.voice_runtime.as_ref().unwrap().resources.counts();
        assert_eq!(
            node.prepare_voice_high_ready_connected(&grants[high], handshake),
            Err(VoiceError::Capacity)
        );
        assert_eq!(frozen_ready_helpers(node, &grants[high], handshake), None,
            "no Ready identity/helper choice may be frozen while there is no authenticated usable path");
        assert_eq!(
            node.voice_runtime.as_ref().unwrap().resources.counts(),
            before
        );
        node.member_peers = peers;
    }
    let mut delivered = false;
    for _ in 0..80 {
        let node = if high == 0 { &mut f.a } else { &mut f.c };
        match node.prepare_voice_high_ready_connected(&grants[high], handshake) {
            Ok(true) => {
                assert_eq!(
                    frozen_ready_helpers(node, &grants[high], handshake),
                    Some([Some(helper), None])
                );
                node.queue_voice_high_ready(&grants[high], handshake)
                    .unwrap();
            }
            Err(VoiceError::Capacity) => {}
            other => panic!("original delayed route refused: {other:?}"),
        }
        let node = if low == 0 { &mut f.a } else { &mut f.c };
        match node.prepay_voice_low_sdp(&grants[low], handshake) {
            Ok(()) | Err(VoiceError::Capacity) => {}
            other => panic!("LOW prepay: {other:?}"),
        }
        match node.prepare_voice_low_sdp(&grants[low], handshake) {
            Ok(true) => {
                delivered = true;
                break;
            }
            Ok(false) | Err(VoiceError::Capacity | VoiceError::Unauthorized) => {}
            other => panic!("actual original Ready delivery: {other:?}"),
        }
        f.tick(3).await;
        f.clock.advance_ms(25);
    }
    assert!(
        delivered,
        "restored B must carry actual signed Ready and allow LOW extraction"
    );
}

#[tokio::test]
async fn root_connected_ready_allows_only_an_unambiguous_authenticated_direct_only_choice() {
    for case in ["valid", "unbound", "ambiguous", "nonmember"] {
        let (mut f, grants, handshake, low) = confirmed().await;
        let high = 1 - low;
        bind_nodes(&mut f.a, &mut f.c).await;
        let endpoint = if low == 0 {
            f.a.device_id()
        } else {
            f.c.device_id()
        };
        let alias = f.d.device_id();
        let node = if high == 0 { &mut f.a } else { &mut f.c };
        node.member_peers.retain(|proof| proof.device == endpoint);
        assert!(
            !node.member_peers.is_empty(),
            "actual direct signed binding must precede the test"
        );
        match case {
            "unbound" => {
                for proof in &mut node.member_peers {
                    proof.bound = false;
                }
            }
            "ambiguous" => {
                node.peer_records
                    .insert(alias, node.peer_records[&endpoint].clone());
            }
            "nonmember" => {
                let outsider = DeviceId::from_bytes([241; 32]);
                node.peer_records
                    .insert(outsider, node.peer_records[&endpoint].clone());
                for proof in &mut node.member_peers {
                    proof.device = outsider;
                }
            }
            _ => {}
        }
        let before = node.voice_runtime.as_ref().unwrap().resources.counts();
        let result = node.prepare_voice_high_ready_connected(&grants[high], handshake);
        if case == "valid" {
            assert!(matches!(result, Ok(true) | Err(VoiceError::Capacity)));
            assert_eq!(
                frozen_ready_helpers(node, &grants[high], handshake),
                Some([None, None]),
                "a real authenticated direct route legitimately needs no member helper"
            );
        } else {
            assert_eq!(result, Err(VoiceError::Capacity), "{case}");
            assert_eq!(
                frozen_ready_helpers(node, &grants[high], handshake),
                None,
                "{case}"
            );
            assert_eq!(
                node.voice_runtime.as_ref().unwrap().resources.counts(),
                before,
                "{case}"
            );
        }
    }
}
