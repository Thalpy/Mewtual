//! The actual original Open token/terminal/delivery transaction around a scoped fence.
use super::*;
use crate::voice_v2::sdp::{SdpResultAdmission, SdpResultIdentity};

fn scoped(
    p: &mut ProtectedFixture,
    action: &Arc<CryptoAction>,
    output: SdpOpenOutput<'_>,
    publish: impl FnOnce(SdpResultAdmission<'_>) -> Result<(), VoiceError>,
) -> Result<SdpOpenResult, VoiceError> {
    p.f.high.join.submit_sdp_open_result_scoped(
        &p.high,
        &room(&p.f.high),
        action,
        output,
        &ca(&p.f.high.context, &p.f.high.lease),
        &p.f.clock,
        |_, _| Ok(()),
        publish,
    )
}

#[test]
fn sdp_result_scoped_open_refusal_noop_and_unwind_do_not_consume_token_or_grant_delivery() {
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    let clock = p.origin_clock.clone();
    let output = output(&action, &bytes);
    assert_eq!(
        scoped(&mut p, &action, output, |_| Err(VoiceError::Capacity)).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(
        scoped(&mut p, &action, output, |_| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = scoped(&mut p, &action, output, |_| {
            panic!("native refuses by unwinding")
        });
    }))
    .is_err());
    action.with_pending(|| Ok(())).unwrap();
    assert!(action.recover_sdp_open_history().unwrap().is_none());
    let first = scoped(&mut p, &action, output, |step| {
        let _native = NativeFenceProbe::new(&clock);
        assert!(step.matches(
            &action,
            SdpResultIdentity::Bytes(action.output_tag(&bytes)?)
        ));
        assert!(!step.matches(&action, SdpResultIdentity::AuthenticationFailed));
        step.commit()
    })
    .unwrap();
    let delivery = first.delivery.as_ref().unwrap();
    assert!(!delivery.was_admitted());
    action.acknowledge_result(&first.receipt).unwrap();
    action.settle_engine();
    phase.close();
    let retry = scoped(&mut p, &action, output, |_| {
        panic!("history needs no new native admission")
    })
    .unwrap();
    assert!(retry.receipt.same_transition(&first.receipt));
    assert!(Arc::ptr_eq(delivery, retry.delivery.as_ref().unwrap()));
}

#[test]
fn sdp_result_scoped_open_distinct_aead_failure_and_invalid_utf8_are_exact_recoverable_rejections()
{
    for aead in [false, true] {
        let (mut p, phase, intent) = fixture();
        let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
        action.admit_engine(action.engine_bytes()).unwrap();
        let bytes = vec![0xff; MAX_SDP_PLAINTEXT];
        let output = if aead {
            SdpOpenOutput::AuthenticationFailed
        } else {
            output(&action, &bytes)
        };
        let identity = if aead {
            SdpResultIdentity::AuthenticationFailed
        } else {
            SdpResultIdentity::Bytes(action.output_tag(&bytes).unwrap())
        };
        let first = scoped(&mut p, &action, output, |step| {
            assert!(step.matches(&action, identity));
            step.commit()
        })
        .unwrap();
        assert_eq!(
            first.receipt.outcome,
            CryptoOutcome::Rejected(VoiceError::Malformed)
        );
        assert!(first.delivery.is_none());
        let different = if aead {
            self::output(&action, &bytes)
        } else {
            SdpOpenOutput::AuthenticationFailed
        };
        assert_eq!(
            scoped(&mut p, &action, different, |_| panic!(
                "changed output cannot republish"
            ))
            .err(),
            Some(VoiceError::RetryMismatch)
        );
        action.acknowledge_result(&first.receipt).unwrap();
        action.settle_engine();
        phase.close();
        assert!(scoped(&mut p, &action, output, |_| panic!(
            "rejected history bypasses hook"
        ))
        .unwrap()
        .receipt
        .same_transition(&first.receipt));
    }
}

#[test]
fn sdp_result_scoped_open_actual_action_identity_and_committed_history_win_over_late_refusal() {
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let (mut other, other_phase, other_intent) = fixture();
    let foreign = open(&mut other, &other_intent, |_, _| Ok(())).unwrap();
    assert_eq!(action.serial(), foreign.serial());
    assert_eq!(action.input_tag(), foreign.input_tag());
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    let output = output(&action, &bytes);
    let identity = SdpResultIdentity::Bytes(action.output_tag(&bytes).unwrap());
    assert_eq!(
        scoped(&mut p, &action, output, |step| {
            assert!(step.matches(&action, identity));
            assert!(!step.matches(&foreign, identity));
            step.commit()?;
            phase.request_close();
            Err(VoiceError::Capacity)
        })
        .err(),
        Some(VoiceError::Capacity)
    );
    let past = action.recover_sdp_open_history().unwrap().unwrap();
    assert_eq!(past.receipt.outcome, CryptoOutcome::Opened);
    assert!(!past.delivery.as_ref().unwrap().was_admitted());
    assert!(scoped(&mut p, &action, output, |_| panic!(
        "original terminal wins"
    ))
    .unwrap()
    .receipt
    .same_transition(&past.receipt));
    action.acknowledge_result(&past.receipt).unwrap();
    action.settle_engine();
    foreign.retire_if_unstarted();
    phase.close();
    other_phase.close();
}
