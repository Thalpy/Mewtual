//! Scoped final delivery is separate from Open, engine settlement and historical status.
//! These tests use the actual original Action/delivery and Actor clock/view boundaries.
use super::*;
use crate::voice_v2::sdp::SdpDeliveryAdmission;

fn accepted() -> (
    ProtectedFixture,
    Arc<Phase>,
    Arc<CryptoAction>,
    Arc<SdpDelivery>,
) {
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    let result = submit(&mut p, &action, output(&action, &bytes), |_, _| Ok(())).unwrap();
    action.acknowledge_result(&result.receipt).unwrap();
    action.settle_engine();
    assert!(
        action.input().is_err(),
        "ACK and actual settlement compact input"
    );
    (p, phase, action, result.delivery.unwrap())
}
fn scoped(
    p: &ProtectedFixture,
    action: &Arc<CryptoAction>,
    delivery: &Arc<SdpDelivery>,
    applied: bool,
    publish: impl FnOnce(SdpDeliveryAdmission<'_>) -> Result<(), VoiceError>,
) -> Result<(), VoiceError> {
    p.f.high.join.with_sdp_delivery_admission(
        &p.high,
        &room(&p.f.high),
        action,
        delivery,
        &ca(&p.f.high.context, &p.f.high.lease),
        applied,
        |_, _| Ok(()),
        publish,
    )
}

#[test]
fn sdp_delivery_scoped_after_input_compaction_has_no_clock_in_native_commit() {
    let (p, phase, action, delivery) = accepted();
    assert!(!delivery.was_admitted());
    scoped(&p, &action, &delivery, false, |step| {
        assert!(step.matches(&action, &delivery, false));
        assert!(!step.matches(&action, &delivery, true));
        let reads = p.origin_clock.reads.load(Ordering::Acquire);
        step.commit()?;
        assert_eq!(p.origin_clock.reads.load(Ordering::Acquire), reads);
        Ok(())
    })
    .unwrap();
    assert!(delivery.was_admitted());
    assert!(
        !delivery.was_applied(),
        "native admission is not SDK completion"
    );
    scoped(&p, &action, &delivery, true, |step| step.commit()).unwrap();
    assert!(delivery.was_applied());
    phase.close();
}

#[test]
fn sdp_delivery_scoped_noop_refusal_unwind_and_postcommit_error_preserve_truth() {
    let (p, phase, action, delivery) = accepted();
    assert_eq!(
        scoped(&p, &action, &delivery, false, |_| Ok(())),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        scoped(&p, &action, &delivery, false, |_| Err(VoiceError::Capacity)),
        Err(VoiceError::Capacity)
    );
    assert!(!delivery.was_admitted());
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = scoped(&p, &action, &delivery, false, |_| {
            panic!("abandoned original hook")
        });
    }));
    assert!(unwind.is_err());
    assert!(!delivery.was_admitted());
    assert_eq!(
        scoped(&p, &action, &delivery, false, |step| {
            step.commit()?;
            Err(VoiceError::Capacity)
        }),
        Ok(()),
        "actual publication cannot be contradicted by a later caller error"
    );
    phase.close();
}

#[test]
fn sdp_delivery_scoped_binds_actual_action_and_delivery_allocations() {
    let (p, phase, action, delivery) = accepted();
    let (_other, other_phase, foreign_action, foreign_delivery) = accepted();
    scoped(&p, &action, &delivery, false, |step| {
        assert!(!step.matches(&foreign_action, &delivery, false));
        assert!(!step.matches(&action, &foreign_delivery, false));
        Err(VoiceError::Unauthorized)
    })
    .unwrap_err();
    assert_eq!(
        scoped(&p, &action, &foreign_delivery, false, |_| panic!(
            "foreign delivery cannot reach hook"
        )),
        Err(VoiceError::Unauthorized)
    );
    assert!(!delivery.was_admitted());
    phase.close();
    other_phase.close();
}

#[test]
fn sdp_delivery_scoped_completion_needs_admission_and_refuses_first_apply_after_expiry() {
    let (p, phase, action, delivery) = accepted();
    assert_eq!(
        scoped(&p, &action, &delivery, true, |_| panic!(
            "completion before admission"
        )),
        Err(VoiceError::Unauthorized)
    );
    scoped(&p, &action, &delivery, false, |step| step.commit()).unwrap();
    p.f.clock.advance_ms(30_000);
    assert_eq!(
        scoped(&p, &action, &delivery, true, |step| step.commit()),
        Err(VoiceError::Unauthorized)
    );
    assert!(!delivery.was_applied());
    assert_eq!(
        scoped(&p, &action, &delivery, false, |step| step.commit()),
        Err(VoiceError::Unauthorized)
    );
    phase.close();
}

#[test]
fn sdp_delivery_scoped_exact_applied_history_recovers_after_close_without_fresh_grant() {
    let (p, phase, action, delivery) = accepted();
    scoped(&p, &action, &delivery, false, |step| step.commit()).unwrap();
    scoped(&p, &action, &delivery, true, |step| step.commit()).unwrap();
    phase.close();
    p.f.high.lease.revoke();
    assert_eq!(
        scoped(&p, &action, &delivery, true, |_| panic!(
            "historical Applied needs no new hook"
        )),
        Ok(())
    );
    assert_eq!(
        scoped(&p, &action, &delivery, false, |step| step.commit()),
        Err(VoiceError::Unauthorized)
    );
}

#[test]
fn sdp_delivery_scoped_native_revocation_before_commit_has_no_effect() {
    let (p, phase, action, delivery) = accepted();
    assert_eq!(
        scoped(&p, &action, &delivery, false, |step| {
            phase.request_close();
            step.commit()
        }),
        Err(VoiceError::Unauthorized)
    );
    assert!(!delivery.was_admitted());
    phase.close();
}

#[test]
fn sdp_delivery_scoped_actual_group_policy_contention_refuses_without_effect_or_clock_under_fence()
{
    for policy in [false, true] {
        let (p, phase, action, delivery) = accepted();
        let (held_tx, held_rx) = std::sync::mpsc::sync_channel(0);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
        let (start_tx, start_rx) = std::sync::mpsc::sync_channel(0);
        let hold = p
            .high
            .native_view_holder_for_test(policy, held_tx, release_rx);
        let holder = std::thread::spawn(move || {
            if start_rx.recv().is_ok() {
                hold();
            }
        });
        let result = scoped(&p, &action, &delivery, false, |step| {
            start_tx.send(()).unwrap();
            held_rx.recv().unwrap();
            let _native = NativeFenceProbe::new(&p.origin_clock);
            step.commit()
        });
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        assert_eq!(result, Err(VoiceError::Capacity));
        assert!(!delivery.was_admitted());
        scoped(&p, &action, &delivery, false, |step| step.commit()).unwrap();
        phase.close();
    }
}

#[test]
fn sdp_delivery_scoped_original_clock_revocation_after_preparation_refuses_effect() {
    let (baseline, phase, action, delivery) = accepted();
    let before = baseline.origin_clock.reads.load(Ordering::Acquire);
    scoped(&baseline, &action, &delivery, false, |step| step.commit()).unwrap();
    let samples = baseline.origin_clock.reads.load(Ordering::Acquire) - before;
    assert!(samples > 0);
    phase.close();
    let (p, phase, action, delivery) = accepted();
    let trigger = p.origin_clock.reads.load(Ordering::Acquire) + samples;
    *p.origin_clock.revoke.lock().unwrap() = Some((trigger, p.f.high.lease.clone()));
    assert_eq!(
        scoped(&p, &action, &delivery, false, |step| step.commit()),
        Err(VoiceError::Unauthorized)
    );
    assert!(p.origin_clock.revoke.lock().unwrap().is_none());
    assert!(!delivery.was_admitted());
    phase.close();
}
