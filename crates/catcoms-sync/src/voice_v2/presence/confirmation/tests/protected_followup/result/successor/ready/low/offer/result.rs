//! Actual Root-side seal transaction and its original signed cache/engine custody.
use super::*;
use crate::voice_v2::crypto_actions::{CryptoOutcome, CryptoResultReceipt};
use crate::voice_v2::presence::ProtectedSealedResult;

fn signer(p: &ProtectedFixture) -> Signer {
    let mut signer = Signer::original(p);
    signer.key = [1, 2]
        .into_iter()
        .map(|seed| DeviceKeypair::from_seed(&[seed; 32]))
        .find(|key| key.device_id() == p.f.low.context.local)
        .unwrap();
    signer
}
fn submit(
    p: &mut ProtectedFixture,
    action: &Arc<CryptoAction>,
    ciphertext: &[u8],
    signer: &Signer,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<CryptoResultReceipt, VoiceError> {
    p.f.low.join.submit_sdp_sealed_result(
        ProtectedSealedResult {
            actor: &p.low,
            room: &room(&p.f.low),
            action,
            ciphertext,
            claimed_tag: action.output_tag(ciphertext)?,
        },
        signer,
        &ca(&p.f.low.context, &p.f.low.lease),
        &p.f.clock,
        final_owner,
    )
}

#[test]
fn sdp_seal_maximum_result_final_capacity_reuses_signature_and_retains_original_engine_until_settlement(
) {
    let (mut p, phase, intent, exchange) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let body = vec![b'a'; MAX_SDP_PLAINTEXT];
    let action = offer(&mut p, &intent, &mut namespace, &body, |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let ciphertext = vec![42; MAX_SDP_PLAINTEXT + 16];
    let signer = signer(&p);
    for _ in 0..3 {
        assert_eq!(
            submit(&mut p, &action, &ciphertext, &signer, |_, _| Err(
                VoiceError::Capacity
            ))
            .err(),
            Some(VoiceError::Capacity)
        );
        assert_eq!(
            signer.calls.get(),
            1,
            "exact inert signature survives every local refusal"
        );
        assert!(p.f.low.join.published_source_packets().all(|object| !matches!(object.body(), Body::Encrypted { header, .. } if header.kind.is_sdp())));
        assert!(action.terminal_status().receipt.is_none());
    }
    let receipt = submit(&mut p, &action, &ciphertext, &signer, |_, _| Ok(())).unwrap();
    assert!(matches!(receipt.outcome, CryptoOutcome::Sealed { .. }));
    assert_eq!(signer.calls.get(), 1);
    assert_eq!(p.f.low.join.published_source_packets().filter(|object| matches!(object.body(), Body::Encrypted { header, .. } if header.kind.is_sdp())).count(), 1);
    assert!(
        matches!(
            exchange.charge(crate::voice_v2::negotiation::Stage::Crypto, 1),
            Err(VoiceError::Capacity)
        ),
        "terminal success cannot refund actual engine custody or allow Source scratch yet"
    );
    action.acknowledge_result(&receipt).unwrap();
    action.settle_engine();
    drop(
        exchange
            .charge(crate::voice_v2::negotiation::Stage::Crypto, 1)
            .unwrap(),
    );
    assert!(action.input().is_err());
    phase.close();
    let retry = submit(&mut p, &action, &ciphertext, &signer, |_, _| {
        panic!("historical terminal never republishes")
    })
    .unwrap();
    assert!(retry.same_transition(&receipt));
    assert!(retry.retired);
    let mut changed = ciphertext;
    changed[0] ^= 1;
    assert_eq!(
        submit(&mut p, &action, &changed, &signer, |_, _| Ok(())).err(),
        Some(VoiceError::RetryMismatch)
    );
    assert_eq!(signer.calls.get(), 1);
}

#[test]
fn sdp_seal_pc_close_in_final_origin_refuses_first_publication_without_refunding_running_engine() {
    let (mut p, phase, intent, exchange) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let action = offer(&mut p, &intent, &mut namespace, b"v=0\r\n", |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let signer = signer(&p);
    assert_eq!(
        submit(&mut p, &action, &[42; 21], &signer, |_, _| {
            phase.request_close();
            Ok(())
        })
        .err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(action.terminal_status().receipt.is_none());
    assert!(p.f.low.join.published_source_packets().all(
        |object| !matches!(object.body(), Body::Encrypted { header, .. } if header.kind.is_sdp())
    ));
    assert!(matches!(
        exchange.charge(crate::voice_v2::negotiation::Stage::Crypto, 1),
        Err(VoiceError::Capacity)
    ));
    action.settle_engine();
    phase.close();
}

#[test]
fn sdp_seal_actual_settlement_inside_signer_cannot_publish_a_late_first_terminal() {
    let (mut p, phase, intent, _) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let action = offer(&mut p, &intent, &mut namespace, b"v=0\r\n", |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let mut signer = signer(&p);
    let original = action.clone();
    signer.callback = Some(Box::new(move || original.settle_engine()));
    assert_eq!(
        submit(&mut p, &action, &[42; 21], &signer, |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(action.is_settled());
    assert!(action.terminal_status().receipt.is_none());
    assert!(p.f.low.join.published_source_packets().all(
        |object| !matches!(object.body(), Body::Encrypted { header, .. } if header.kind.is_sdp())
    ));
    phase.close();
}
