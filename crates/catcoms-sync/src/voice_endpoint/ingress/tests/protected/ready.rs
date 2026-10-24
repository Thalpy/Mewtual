//! Root namespace and actual native admission, with real MLS/call/directory/confirmation.
//! Only AEAD is mocked. Synthetic aliases isolate Root namespace pressure from byte pressure;
//! this does not claim that255 separately paid network admissions fit alongside Ready.
use super::*;
use voice_v2::presence::{OpenedFlight, ReceiveProgress};
mod source;

async fn confirmed() -> (Fixture, voice_v2::Nonce) {
    confirmed_in(fixture().await).await
}
async fn confirmed_in(mut f: Fixture) -> (Fixture, voice_v2::Nonce) {
    let hello = f.object.clone();
    let Body::Encrypted { header, .. } = hello.body() else {
        panic!("Hello")
    };
    let handshake = header.handshake;
    admit(&mut f, hello).unwrap();
    let open = actions(&mut f).pop().unwrap();
    open.admit_engine(open.engine_bytes()).unwrap();
    let result = f
        .receiver
        .commit_voice_confirmation_result(
            &f.grant,
            &open,
            &f.plaintext,
            open.output_tag(&f.plaintext).unwrap(),
        )
        .unwrap();
    open.settle_engine();
    open.acknowledge_result(&result).unwrap();
    let proof = f
        .receiver
        .respond_voice_confirmation(&f.grant, handshake, &open)
        .unwrap();
    let input = proof.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("Proof")
    };
    let plain = flight.plaintext.clone();
    let ciphertext = vec![33; plain.len() + 16];
    proof.admit_engine(proof.engine_bytes()).unwrap();
    let sealed = f
        .receiver
        .submit_voice_confirmation_result(
            &f.grant,
            &proof,
            &ciphertext,
            proof.output_tag(&ciphertext).unwrap(),
        )
        .unwrap();
    let context = f.receiver.voice_v2_context(1).unwrap();
    let object = f
        .receiver
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .participant
        .as_ref()
        .unwrap()
        .join
        .published_source_packets()
        .next()
        .unwrap()
        .clone();
    proof.settle_engine();
    proof.acknowledge_result(&sealed).unwrap();
    drop((input, proof, open));
    let remote_context = Context {
        local: f.sender.device_id(),
        ..context
    };
    let auth = authority(&remote_context, &f.remote_lease);
    let ReceiveProgress::Decrypt(packet) = f
        .remote
        .receive_confirmation(object, &auth, f.clock.as_ref())
        .unwrap()
    else {
        panic!("first Proof input")
    };
    f.remote
        .commit_confirmation(
            OpenedFlight {
                token: packet.token,
                aad: packet.aad(),
                sequence: packet.header.sequence,
                ciphertext: packet.ciphertext(),
                plaintext: &plain,
            },
            &auth,
            f.clock.as_ref(),
            &mut f.sender.rng,
        )
        .unwrap();
    let Some(ConfirmationOutput::Encrypt(flight)) = f
        .remote
        .prepare_confirmation(handshake, &auth, f.clock.as_ref(), &mut f.sender.rng)
        .unwrap()
    else {
        panic!("Finish")
    };
    let finish = f
        .remote
        .submit_confirmation(
            FlightSubmission {
                handshake,
                action: flight.action,
                reservation: flight.reservation.id,
                ciphertext: &vec![44; flight.plaintext.len() + 16],
            },
            &f.sender.device,
            &auth,
            f.clock.as_ref(),
        )
        .unwrap();
    admit(&mut f, finish).unwrap();
    let open = actions(&mut f)
        .into_iter()
        .find(|a| a.is_pending())
        .unwrap();
    open.admit_engine(open.engine_bytes()).unwrap();
    let result = f
        .receiver
        .commit_voice_confirmation_result(
            &f.grant,
            &open,
            &flight.plaintext,
            open.output_tag(&flight.plaintext).unwrap(),
        )
        .unwrap();
    open.settle_engine();
    open.acknowledge_result(&result).unwrap();
    drop(open);
    (f, handshake)
}

fn offer(f: &Fixture, handshake: voice_v2::Nonce, id: u8) -> SignedObject {
    let round = f
        .receiver
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .participant
        .as_ref()
        .unwrap()
        .join
        .sdp_round_for_test(handshake)
        .unwrap();
    offer_in_round(f, handshake, id, round)
}
fn offer_in_round(
    f: &Fixture,
    handshake: voice_v2::Nonce,
    id: u8,
    round: voice_v2::SdpRound,
) -> SignedObject {
    let context = f.receiver.voice_v2_context(1).unwrap();
    let remote = f.remote.key();
    let local = f
        .receiver
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .participant
        .as_ref()
        .unwrap()
        .join
        .key();
    let message = [id; 32];
    SignedObject::issue(
        context.scope,
        &f.sender.device,
        message,
        f.clock.now_ms(),
        f.clock.now_ms() + 15000,
        Body::Encrypted {
            header: voice_v2::EncryptedHeader {
                source_join: remote.join,
                target: context.local,
                target_join: local.join,
                source_key_ref: remote.key_ref,
                target_key_ref: local.key_ref,
                handshake,
                low_to_high: true,
                kind: EncryptedKind::OfferRound(round),
                sequence: 4,
                offer: Some(voice_v2::OfferId {
                    issuer: f.sender.device_id(),
                    message,
                }),
            },
            ciphertext: vec![55; voice_v2::MAX_SDP_PLAINTEXT + 16],
        },
    )
    .unwrap()
}

fn fill_namespace(f: &mut Fixture, total: usize) {
    let context = f.receiver.voice_v2_context(1).unwrap();
    let room = f.receiver.voice_room_authority(1).unwrap();
    let voice = f.receiver.voice_runtime.as_mut().unwrap();
    let actor = voice.current.as_ref().unwrap();
    let participant = voice.endpoints.participant.as_ref().unwrap();
    let object = SignedObject::issue(
        context.scope,
        &f.sender.device,
        [199; 32],
        1000,
        16000,
        Body::WatchChallenge {
            target: context.local,
            join: participant.join.key().join,
            key_ref: participant.join.key().key_ref,
            watcher: [201; 32],
            challenge: [202; 32],
        },
    )
    .unwrap();
    let credit = voice
        .resources
        .charge(
            context.scope.group,
            object.encode().len() + 1024,
            ResourceKind::Retained,
        )
        .unwrap();
    let packet = Arc::new(Packet {
        hash: object.hash(),
        object,
        room,
        targets: Targets::Join(participant.watch.id, participant.lease.clone()),
        effect: actor
            .effect(f.sender.device_id(), context.local, 16000)
            .unwrap(),
        deadline: 16000,
        _credit: credit,
    });
    // UNREACHABLE synthetic pressure state: aliases share one real paid Packet so receipt
    // namespace exhaustion is tested independently of the512KiB byte limit. They are not
    //255 feasible network admissions. The admitted Offer and native pair remain genuine;
    // the separate maximum-byte regression exercises the original real payment composition.
    while voice.endpoints.public.packets.len() + voice.endpoints.public.encrypted.len() < total {
        let mut id = [198; 32];
        id[..8].copy_from_slice(&(voice.endpoints.public.packets.len() as u64).to_le_bytes());
        voice.endpoints.public.packets.insert(
            (f.sender.device_id(), id),
            Work {
                packet: packet.clone(),
                processed: BTreeSet::new(),
            },
        );
    }
}
fn positions(f: &Fixture) -> usize {
    let inbox = &f.receiver.voice_runtime.as_ref().unwrap().endpoints.public;
    inbox
        .positions
        .total_for_test(inbox.packets.len() + inbox.encrypted.len())
}

#[tokio::test]
async fn root_first_offer_reserved_position_survives_full_shared_table_and_lost_reply() {
    let (mut f, handshake) = confirmed().await;
    fill_namespace(&mut f, PACKETS - 1);
    assert!(f
        .receiver
        .prepare_voice_high_ready(&f.grant, handshake, &[])
        .unwrap());
    assert!(f
        .receiver
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .public
        .full());
    assert_eq!(positions(&f), PACKETS);
    // A new public packet cannot steal the last promised position.
    let ordinary = super::super::announcement(&f.sender, f.grant.scope());
    assert_eq!(
        admit(&mut f, ordinary.object().clone()).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(
        positions(&f),
        PACKETS,
        "refusal cannot consume the promised position"
    );
    let object = offer(&f, handshake, 212);
    let first = admit(&mut f, object.clone()).unwrap();
    assert!(matches!(first, EndpointReceipt::FirstOffer(_)));
    let inbox = &f.receiver.voice_runtime.as_ref().unwrap().endpoints.public;
    assert_eq!(inbox.packets.len() + inbox.encrypted.len(), PACKETS);
    let held = f.runtime.resources().counts();
    let recovered = admit(&mut f, object).unwrap();
    let (EndpointReceipt::FirstOffer(first), EndpointReceipt::FirstOffer(recovered)) =
        (first, recovered)
    else {
        unreachable!()
    };
    assert!(Arc::ptr_eq(&first, &recovered));
    assert_eq!(f.runtime.resources().counts(), held);
    assert_eq!(
        positions(&f),
        PACKETS,
        "retry cannot consume another position"
    );
    assert!(!f
        .receiver
        .prepare_voice_high_ready(&f.grant, handshake, &[])
        .unwrap());
}

#[tokio::test]
async fn root_first_offer_full_table_before_ready_refuses_without_abandoning_pair() {
    let (mut f, handshake) = confirmed().await;
    fill_namespace(&mut f, PACKETS);
    assert_eq!(
        f.receiver
            .prepare_voice_high_ready(&f.grant, handshake, &[]),
        Err(VoiceError::Capacity)
    );
    let object = offer_in_round(
        &f,
        handshake,
        213,
        voice_v2::SdpRound {
            ready: [201; 32],
            profile: voice_v2::SdpProfile::CompleteSdpV1,
        },
    );
    assert_eq!(admit(&mut f, object).err(), Some(VoiceError::Capacity));
    assert_eq!(
        positions(&f),
        PACKETS,
        "failed Ready leaves no held position"
    );
    // Release one pressure row. Retry uses the same original confirmed pair, not a new join.
    f.receiver
        .voice_runtime
        .as_mut()
        .unwrap()
        .endpoints
        .public
        .packets
        .pop_first();
    assert!(f
        .receiver
        .prepare_voice_high_ready(&f.grant, handshake, &[])
        .unwrap());
    let object = offer(&f, handshake, 213);
    assert!(admit(&mut f, object).unwrap().authorized());
}

#[tokio::test]
async fn root_first_offer_cross_kind_identity_refuses_before_consuming_ready_and_end_returns_position(
) {
    let (mut f, handshake) = confirmed().await;
    assert!(f
        .receiver
        .prepare_voice_high_ready(&f.grant, handshake, &[])
        .unwrap());
    let object = offer(&f, handshake, 214);
    let context = f.receiver.voice_v2_context(1).unwrap();
    let key = f
        .receiver
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .participant
        .as_ref()
        .unwrap()
        .join
        .key();
    let public = SignedObject::issue(
        context.scope,
        &f.sender.device,
        object.id(),
        1000,
        16000,
        Body::WatchChallenge {
            target: context.local,
            join: key.join,
            key_ref: key.key_ref,
            watcher: [201; 32],
            challenge: [202; 32],
        },
    )
    .unwrap();
    let before = positions(&f);
    admit(&mut f, public.clone()).unwrap();
    assert_eq!(positions(&f), before + 1);
    assert_eq!(
        admit(&mut f, object.clone()).err(),
        Some(VoiceError::RetryMismatch)
    );
    assert_eq!(
        positions(&f),
        before + 1,
        "collision never spends the original reservation"
    );
    // Remove only the arranged conflicting public identity. It cannot have published a
    // first-Offer token/replay/role; the same native receive claim is still available.
    f.receiver
        .voice_runtime
        .as_mut()
        .unwrap()
        .endpoints
        .public
        .packets
        .remove(&(object.issuer(), object.id()));
    admit(&mut f, object).unwrap();
    assert_eq!(positions(&f), before);
    assert_eq!(admit(&mut f, public).err(), Some(VoiceError::RetryMismatch));

    let (mut pending, handshake) = confirmed().await;
    let before = positions(&pending);
    pending
        .receiver
        .prepare_voice_high_ready(&pending.grant, handshake, &[])
        .unwrap();
    assert_eq!(positions(&pending), before + 1);
    pending.receiver.stop_voice_endpoint(&pending.grant);
    assert_eq!(
        positions(&pending),
        before,
        "End drops the exact unconsumed namespace position"
    );
}
