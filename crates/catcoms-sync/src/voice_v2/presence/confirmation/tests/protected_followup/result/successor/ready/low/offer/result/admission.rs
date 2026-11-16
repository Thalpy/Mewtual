//! Native fence stand-in around the real signed ledger/terminal transaction. These
//! tests do not claim browser/RTC execution or native mutex integration.
use super::*;
use crate::voice_v2::sdp::{SdpResultAdmission, SdpResultIdentity};

fn scoped(
    p: &mut ProtectedFixture,
    action: &Arc<CryptoAction>,
    ciphertext: &[u8],
    signer: &Signer,
    publish: impl FnOnce(SdpResultAdmission<'_>) -> Result<(), VoiceError>,
) -> Result<CryptoResultReceipt, VoiceError> {
    p.f.low.join.submit_sdp_sealed_result_scoped(
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
        |_, _| Ok(()),
        publish,
    )
}

#[test]
fn sdp_result_scoped_seal_refusal_noop_and_unwind_preserve_original_signature_and_pending_work() {
    let (mut p, phase, intent, exchange) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let action = offer(&mut p, &intent, &mut namespace, b"v=0", |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let signer = signer(&p);
    let clock = p.origin_clock.clone();
    let bytes = [42; 19];
    assert_eq!(
        scoped(&mut p, &action, &bytes, &signer, |_| Err(
            VoiceError::Capacity
        ))
        .err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(
        scoped(&mut p, &action, &bytes, &signer, |_| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = scoped(&mut p, &action, &bytes, &signer, |_| {
            panic!("native unwinds before final admission")
        });
    }));
    assert!(panic.is_err());
    assert_eq!(
        signer.calls.get(),
        1,
        "one inert signature survives all proven local refusals"
    );
    assert!(action.terminal_status().receipt.is_none());
    action.with_pending(|| Ok(())).unwrap();
    assert!(
        matches!(
            exchange.charge(crate::voice_v2::negotiation::Stage::Crypto, 1),
            Err(VoiceError::Capacity)
        ),
        "refusal does not settle running SDK work"
    );
    let receipt = scoped(&mut p, &action, &bytes, &signer, |step| {
        let _native = NativeFenceProbe::new(&clock);
        assert!(step.matches(
            &action,
            SdpResultIdentity::Bytes(action.output_tag(&bytes)?)
        ));
        assert!(!step.matches(&action, SdpResultIdentity::Bytes([0; 32])));
        assert!(!step.matches(&action, SdpResultIdentity::AuthenticationFailed));
        step.commit()
    })
    .unwrap();
    assert!(matches!(receipt.outcome, CryptoOutcome::Sealed { .. }));
    action.acknowledge_result(&receipt).unwrap();
    action.settle_engine();
    phase.close();
    let retry = scoped(&mut p, &action, &bytes, &signer, |_| {
        panic!("historical retry bypasses native hook")
    })
    .unwrap();
    assert!(retry.same_transition(&receipt));
}

#[test]
fn sdp_result_preparation_actual_signing_capacities_fit_original_engine_at_growth_boundaries() {
    // These are actual Rust Vec capacities from the implemented header/child/joined
    // and outer encoder stages, not claimed output lengths. Original packet-cache
    // backing is already paid; include its EXTRA returned/prepared clone here.
    for length in [
        1,
        2,
        3,
        7,
        8,
        15,
        16,
        17,
        31,
        32,
        33,
        63,
        64,
        65,
        127,
        128,
        129,
        255,
        256,
        257,
        511,
        512,
        513,
        1023,
        1024,
        1025,
        2047,
        2048,
        2049,
        4095,
        4096,
        4097,
        8191,
        MAX_SDP_PLAINTEXT,
    ] {
        let (mut p, phase, intent, _) = fixture();
        let mut namespace = ReceiptNamespace::default();
        let action = offer(
            &mut p,
            &intent,
            &mut namespace,
            &vec![b'a'; length],
            |_, _| Ok(()),
        )
        .unwrap();
        action.admit_engine(action.engine_bytes()).unwrap();
        let signer = signer(&p);
        let bytes = vec![42; length + 16];
        let receipt = scoped(&mut p, &action, &bytes, &signer, |step| step.commit()).unwrap();
        let signed = p.f.low.join.published_source_packets().find(|object| matches!(object.body(), Body::Encrypted { header, .. } if header.kind.is_sdp())).unwrap();
        let Body::Encrypted { ciphertext, .. } = signed.body() else {
            unreachable!()
        };
        let header = signed.header();
        let header_capacity = header.capacity();
        let mut child = catcoms_wire::Encoder::new();
        child.put_bytes(ciphertext).unwrap();
        let child = child.finish();
        let mut joined = header;
        joined.extend_from_slice(&child);
        let joining = header_capacity + child.capacity() + joined.capacity();
        drop(child);
        let encoded = signed.encode();
        let encoding = joined.capacity() + encoded.capacity();
        let extra_clone = ciphertext.clone().capacity();
        let variable = extra_clone + joining.max(encoding);
        let wire_bound = 4 * (signed.aad().unwrap().len() + ciphertext.len()) + 2048;
        assert!(
            variable <= wire_bound,
            "length{length}: actual backing {variable}/{wire_bound}"
        );
        let all = crate::voice_v2::sdp::sdp_result_preparation_bytes(
            signed.aad().unwrap().len(),
            ciphertext.len(),
        )
        .unwrap();
        assert!(
            all <= action.engine_bytes(),
            "length{length}: original engine {all}/{}",
            action.engine_bytes()
        );
        action.acknowledge_result(&receipt).unwrap();
        action.settle_engine();
        phase.close();
    }
}

#[test]
fn sdp_result_scoped_seal_committed_terminal_survives_lost_callback_return_and_pc_close() {
    let (mut p, phase, intent, _) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let action = offer(&mut p, &intent, &mut namespace, b"v=0", |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let signer = signer(&p);
    let bytes = [42; 19];
    let refused = scoped(&mut p, &action, &bytes, &signer, |step| {
        step.commit()?;
        phase.request_close();
        Err(VoiceError::Capacity)
    });
    assert_eq!(refused.err(), Some(VoiceError::Capacity));
    let receipt = action
        .recover_result(&bytes, action.output_tag(&bytes).unwrap())
        .unwrap()
        .unwrap();
    assert!(matches!(receipt.outcome, CryptoOutcome::Sealed { .. }));
    let retry = scoped(&mut p, &action, &bytes, &signer, |_| {
        panic!("positive history cannot be downgraded")
    })
    .unwrap();
    assert!(retry.same_transition(&receipt));
    action.acknowledge_result(&receipt).unwrap();
    action.settle_engine();
    phase.close();
}

#[test]
fn sdp_result_scoped_seal_final_engine_contention_and_closed_pc_have_no_terminal_effect() {
    let (mut p, phase, intent, _) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let action = offer(&mut p, &intent, &mut namespace, b"v=0", |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let signer = signer(&p);
    let bytes = [42; 19];
    assert_eq!(
        scoped(&mut p, &action, &bytes, &signer, |step| action
            .with_engine_fence_for_test(|| step.commit()))
        .err(),
        Some(VoiceError::Capacity)
    );
    assert!(action.terminal_status().receipt.is_none());
    assert_eq!(
        scoped(&mut p, &action, &bytes, &signer, |step| {
            phase.request_close();
            step.commit()
        })
        .err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(action.terminal_status().receipt.is_none());
    action.settle_engine();
    phase.close();
}
