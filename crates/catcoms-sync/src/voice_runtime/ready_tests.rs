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
async fn confirmed() -> (BridgeFixture, [Arc<CallGrant>; 2], [u8; 32], usize) {
    let mut f = BridgeFixture::new().await;
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
    let plain = seal(low_node, &grants[low], finish);
    drop(action);
    let action = incoming(&mut f, &grants, high, EncryptedKind::ConfirmFinish).await;
    let high_node = if high == 0 { &mut f.a } else { &mut f.c };
    open(high_node, &grants[high], &action, &plain);
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
    (f, grants, handshake, low)
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
        .find(|job| matches!(job.object().body(), Body::PairReady { .. }))
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
