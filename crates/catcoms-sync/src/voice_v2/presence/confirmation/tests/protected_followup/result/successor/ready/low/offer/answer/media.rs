//! LOW completes the opposite canonical Seal/Open order using actual native factories,
//! signatures and Result/Applied records. AEAD/SDK output is synthetic, as in its parent.
use super::*;
use crate::voice_v2::presence::ProtectedSealedResult;
use crate::voice_v2::sdp::SdpOpenOutput;

#[test]
fn low_media_requires_its_original_applied_answer_and_preserves_admitted_currency_after_ready() {
    let (mut p, phase, intent, _exchange) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let body = b"v=0\r\n";
    let seal = offer(&mut p, &intent, &mut namespace, body, |_, _| Ok(())).unwrap();
    seal.admit_engine(seal.engine_bytes()).unwrap();
    let cipher = vec![42; body.len() + 16];
    let receipt =
        p.f.low
            .join
            .submit_sdp_sealed_result(
                ProtectedSealedResult {
                    actor: &p.low,
                    room: &room(&p.f.low),
                    action: &seal,
                    ciphertext: &cipher,
                    claimed_tag: seal.output_tag(&cipher).unwrap(),
                },
                &p.f.low.signer,
                &ca(&p.f.low.context, &p.f.low.lease),
                &p.f.clock,
                |_, _| Ok(()),
            )
            .unwrap();
    let signed = p.f.low.join.published_source_packets()
        .find(|object| matches!(object.body(), Body::Encrypted { header, .. } if header.kind.is_sdp()))
        .unwrap().clone();
    seal.settle_engine();
    seal.acknowledge_result(&receipt).unwrap();
    let object = answer(&p, &signed, 4);
    admit_answer(&mut p, &namespace, &object, |_, _| Ok(())).unwrap();
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
    open.admit_engine(open.engine_bytes()).unwrap();
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    let opened =
        p.f.low
            .join
            .submit_sdp_open_result(
                &p.low,
                &room(&p.f.low),
                &open,
                SdpOpenOutput::Plaintext {
                    bytes: &bytes,
                    claimed_tag: open.output_tag(&bytes).unwrap(),
                },
                &ca(&p.f.low.context, &p.f.low.lease),
                &p.f.clock,
                |_, _| Ok(()),
            )
            .unwrap();
    open.settle_engine();
    open.acknowledge_result(&opened.receipt).unwrap();
    let delivery = opened.delivery.unwrap();
    assert_eq!(
        intent.prepare_media(&phase, &seal, &open, 1000, 1000).err(),
        Some(VoiceError::Unauthorized),
        "accepted Open alone is not actual Applied history"
    );
    for applied in [false, true] {
        p.f.low
            .join
            .with_sdp_delivery_admission(
                &p.low,
                &room(&p.f.low),
                &open,
                &delivery,
                &ca(&p.f.low.context, &p.f.low.lease),
                applied,
                |_, _| Ok(()),
                |step| step.commit(),
            )
            .unwrap();
    }
    let media = intent
        .prepare_media(&phase, &seal, &open, 1000, 1000)
        .unwrap();
    p.f.low
        .join
        .with_initial_media_admission(
            &p.low,
            &room(&p.f.low),
            &intent,
            &media,
            &seal,
            &open,
            &ca(&p.f.low.context, &p.f.low.lease),
            |_, _| Ok(()),
            |step| step.commit(),
        )
        .unwrap();
    assert!(media.is_current());
    assert!(media.round() == intent.round());
    assert!(media.offer() == delivery.offer());
    phase.close();
    p.f.clock.advance_ms(30_000);
    assert!(media.is_current());
    p.f.low.lease.revoke();
    assert!(!media.is_current());
    assert!(media.was_admitted());
}
