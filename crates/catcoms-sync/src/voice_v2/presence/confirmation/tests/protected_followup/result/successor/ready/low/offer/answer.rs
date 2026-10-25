//! Actual signed Answer intake, exact lost-ACK recovery, original replay/token and PC close.
//! The enclosing fixture deliberately mocks AEAD; it does not claim native/browser RTC.
use super::*;
use crate::voice_v2::negotiation::admission::SdpAdmission;

fn answer(p: &ProtectedFixture, signed_offer: &SignedObject, sequence: u64) -> SignedObject {
    let Body::Encrypted { header, .. } = signed_offer.body() else {
        panic!("Offer")
    };
    let header = EncryptedHeader {
        source_join: header.target_join,
        target: signed_offer.issuer(),
        target_join: header.source_join,
        source_key_ref: header.target_key_ref,
        target_key_ref: header.source_key_ref,
        handshake: header.handshake,
        low_to_high: false,
        kind: EncryptedKind::AnswerRound(header.kind.round().unwrap()),
        sequence,
        offer: header.offer,
    };
    SignedObject::issue(
        p.f.low.context.scope,
        &p.f.high.signer,
        [223; 32],
        1000,
        1000 + crate::voice_v2::MAX_PACKET_LIFETIME_MS,
        Body::Encrypted {
            header,
            ciphertext: vec![42; MAX_SDP_PLAINTEXT + 16],
        },
    )
    .unwrap()
}
fn admit_answer(
    p: &mut ProtectedFixture,
    namespace: &ReceiptNamespace,
    object: &SignedObject,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<Arc<SdpAdmission>, VoiceError> {
    let envelope = Envelope::Direct(object.clone());
    let ticket = p
        .low
        .admit_transport_verification(object.issuer(), &envelope.encode())?;
    let envelope = Envelope::decode(&envelope.encode())?;
    p.f.low.join.admit_protected_answer(
        &p.low,
        &ticket,
        &envelope,
        &room(&p.f.low),
        namespace,
        &ca(&p.f.low.context, &p.f.low.lease),
        p._runtimes[0].clock(),
        final_owner,
    )
}

#[test]
fn low_answer_final_refusal_preserves_same_namespace_slot_then_one_exact_token_and_open() {
    let (mut p, phase, intent, _) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let seal = offer(&mut p, &intent, &mut namespace, b"v=0\r\n", |_, _| Ok(())).unwrap();
    let signed = sign_offer(&mut p, &seal);
    let object = answer(&p, &signed, 4);
    let before = p._runtimes[0].resources().counts();
    for _ in 0..3 {
        assert!(p.f.low.join.has_answer_position(&object, &namespace));
        assert_eq!(
            admit_answer(&mut p, &namespace, &object, |_, _| Err(
                VoiceError::Capacity
            ))
            .err(),
            Some(VoiceError::Capacity)
        );
        assert_eq!(namespace.total_for_test(0), 1);
        assert_eq!(p._runtimes[0].resources().counts(), before);
    }
    let receipt = admit_answer(&mut p, &namespace, &object, |_, _| Ok(())).unwrap();
    assert_eq!(
        namespace.total_for_test(0),
        0,
        "position consumed exactly once by receipt publication"
    );
    assert!(!p.f.low.join.has_answer_position(&object, &namespace));
    assert!(Arc::ptr_eq(
        &receipt,
        &admit_answer(&mut p, &namespace, &object, |_, _| Ok(())).unwrap()
    ));
    let open =
        p.f.low
            .join
            .prepare_low_sdp_open(
                &p.low,
                &room(&p.f.low),
                p.f.id,
                &intent,
                &ca(&p.f.low.context, &p.f.low.lease),
                p._runtimes[0].clock(),
                |_, _| Ok(()),
                None,
            )
            .unwrap();
    assert_eq!(open.operation(), CryptoOperation::OpenSdp);
    let input = open.input().unwrap();
    let CryptoInput::OpenSdp(packet) = input.as_input() else {
        panic!("original Answer Open")
    };
    assert_eq!(packet.token, receipt.token);
    let token = packet.token;
    // Exercise actual native token consumption, then receipt recovery. Result/RTC delivery
    // is a separate boundary; a receipt must remain recoverable after token disappearance.
    p.f.low
        .join
        .ledger
        .commit_inbound(
            token,
            packet.aad(),
            packet.header.sequence,
            packet.ciphertext(),
            &ca(&p.f.low.context, &p.f.low.lease),
            &p.f.clock,
        )
        .unwrap();
    assert!(Arc::ptr_eq(
        &receipt,
        &admit_answer(&mut p, &namespace, &object, |_, _| Ok(())).unwrap()
    ));
    assert_eq!(
        open.admit_engine(open.engine_bytes()),
        Err(VoiceError::Unauthorized)
    );
    open.settle_engine();
    seal.settle_engine();
    phase.close();
}

#[tokio::test]
async fn low_answer_receipt_wait_observes_actual_pc_close_with_same_input_still_retained() {
    use std::future::Future;
    use std::task::Poll;
    let (mut p, phase, intent, owner) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let seal = offer(&mut p, &intent, &mut namespace, b"v=0\r\n", |_, _| Ok(())).unwrap();
    let signed = sign_offer(&mut p, &seal);
    let object = answer(&p, &signed, 4);
    let receipt = admit_answer(&mut p, &namespace, &object, |_, _| Ok(())).unwrap();
    let held = p.f.low.join.confirmations.pairs[&p.f.id]
        .sdp_input
        .as_ref()
        .unwrap()
        .clone();
    let mut wait = std::pin::pin!(receipt.revoked());
    std::future::poll_fn(|cx| {
        assert!(wait.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    phase.close();
    assert!(!receipt.authorized_at(1000, 1000));
    std::future::poll_fn(|cx| {
        assert!(wait.as_mut().poll(cx).is_ready());
        Poll::Ready(())
    })
    .await;
    assert!(matches!(
        owner.charge(crate::voice_v2::negotiation::Stage::RemotePacket, 1),
        Err(VoiceError::Capacity)
    ));
    assert_eq!(held.token, receipt.token);
    assert_eq!(held.ciphertext().len(), MAX_SDP_PLAINTEXT + 16);
    seal.settle_engine();
}

#[test]
fn low_answer_close_at_final_admission_refuses_without_spending_namespace_or_replay() {
    let (mut p, phase, intent, _) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let seal = offer(&mut p, &intent, &mut namespace, b"v=0\r\n", |_, _| Ok(())).unwrap();
    let signed = sign_offer(&mut p, &seal);
    let object = answer(&p, &signed, 4);
    assert_eq!(
        admit_answer(&mut p, &namespace, &object, |_, _| {
            phase.request_close();
            Ok(())
        })
        .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(namespace.total_for_test(0), 1);
    assert!(p.f.low.join.confirmations.pairs[&p.f.id]
        .answer_receipt
        .is_none());
    assert_eq!(
        admit_answer(&mut p, &namespace, &object, |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    p.f.low.join.collect_confirmations(&p.f.clock);
    assert_eq!(
        namespace.total_for_test(0),
        0,
        "quiet collection returns unused original position after PC close"
    );
    seal.settle_engine();
    phase.close();
}
