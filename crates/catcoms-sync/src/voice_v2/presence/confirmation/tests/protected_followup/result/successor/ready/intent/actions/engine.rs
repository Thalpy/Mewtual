//! Actual maximum Open/Exchange and final Engine/Group/Policy barriers. The native
//! fence below is a deterministic Root stand-in, not native HTTP/SDK acceptance.
use super::*;
use crate::voice_v2::crypto_actions::SdpEngineRead;
use crate::voice_v2::sdp::SdpEngineAdmission;

fn with_step<T>(
    p: &ProtectedFixture,
    intent: &Arc<InitialSdpIntent>,
    action: &Arc<CryptoAction>,
    publish: impl FnOnce(SdpEngineAdmission<'_>) -> Result<T, VoiceError>,
) -> Result<T, VoiceError> {
    let prepared = action.prepare_sdp_engine_candidate(intent, action.engine_bytes())?;
    let room = room(&p.f.high);
    let watch = crate::voice_v2::WatchLease::new(p.f.high.lease.clone());
    let current = p.f.high.join.prepare_sdp_input_current(
        &p.high,
        &room,
        action,
        intent,
        &p.f.high.context,
        &p.f.high.lease,
        &watch,
    );
    prepared.with_observation(|observed| publish(SdpEngineAdmission::new(observed, current)))
}

#[test]
fn sdp_engine_maximum_open_uses_original_exchange_under_full_ordinary_pressure() {
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    let input = action.input().unwrap();
    let CryptoInput::OpenSdp(packet) = input.as_input() else {
        panic!("original Open")
    };
    assert_eq!(packet.ciphertext().len(), MAX_SDP_PLAINTEXT + 16);
    let resources = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    let pressure = resources
        .charge(
            group,
            crate::voice_v2::resources::GROUP_BYTES - resources.group_counts(group).unwrap().0,
            crate::voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    let SdpEngineRead::Permit(serial) = with_step(&p, &intent, &action, |step| {
        let _native = NativeFenceProbe::new(&p.origin_clock);
        step.commit()
    })
    .unwrap() else {
        panic!("original prepaid engine")
    };
    assert_eq!(serial, action.serial());
    let charged = resources.counts();
    assert!(
        matches!(with_step(&p, &intent, &action, |step| step.commit()).unwrap(), SdpEngineRead::Permit(same) if same == serial)
    );
    assert_eq!(resources.counts(), charged);
    drop((input, pressure));
    action.settle_engine();
    phase.close();
}

#[test]
fn sdp_engine_actual_final_engine_group_and_policy_contention_refuse_without_installing_credit() {
    for which in 0..3 {
        let (mut p, phase, intent) = fixture();
        let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
        let resources = p._runtimes[1].resources();
        let before = resources.counts();
        let result = if which == 0 {
            with_step(&p, &intent, &action, |step| {
                action.with_engine_fence_for_test(|| step.commit())
            })
        } else {
            let (held_tx, held_rx) = std::sync::mpsc::sync_channel(0);
            let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
            let (start_tx, start_rx) = std::sync::mpsc::sync_channel(0);
            let hold = p
                .high
                .native_view_holder_for_test(which == 2, held_tx, release_rx);
            let holder = std::thread::spawn(move || {
                start_rx.recv().unwrap();
                hold();
            });
            let result = with_step(&p, &intent, &action, |step| {
                start_tx.send(()).unwrap();
                held_rx.recv().unwrap();
                step.commit()
            });
            release_tx.send(()).unwrap();
            holder.join().unwrap();
            result
        };
        assert_eq!(result.err(), Some(VoiceError::Capacity));
        assert_eq!(
            resources.counts(),
            before,
            "refused candidate returned outside guards"
        );
        assert!(matches!(
            with_step(&p, &intent, &action, |step| step.commit()).unwrap(),
            SdpEngineRead::Permit(_)
        ));
        action.settle_engine();
        phase.close();
    }
}

#[test]
fn sdp_engine_candidate_precedes_origin_and_action_expiry_observation() {
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    let prepared = action
        .prepare_sdp_engine_candidate(&intent, action.engine_bytes())
        .unwrap();
    let remaining = intent
        .remaining_at(p.f.clock.now_ms(), p.f.clock.monotonic_ms())
        .unwrap();
    p.f.clock.advance_ms(remaining);
    let room = room(&p.f.high);
    let watch = crate::voice_v2::WatchLease::new(p.f.high.lease.clone());
    let current = p.f.high.join.prepare_sdp_input_current(
        &p.high,
        &room,
        &action,
        &intent,
        &p.f.high.context,
        &p.f.high.lease,
        &watch,
    );
    let result =
        prepared.with_observation(|observed| SdpEngineAdmission::new(observed, current).commit());
    assert_eq!(result.err(), Some(VoiceError::Unauthorized));
    action.retire_if_unstarted();
    assert!(action.handoff_status().engine.settled);
    phase.close();
}

#[test]
fn sdp_engine_step_matches_actual_action_allocation_before_any_permit_effect() {
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    let (mut other, other_phase, other_intent) = fixture();
    let foreign = open(&mut other, &other_intent, |_, _| Ok(())).unwrap();
    // The two deterministic fixtures deliberately produce equal wire labels. Native
    // must still compare the actual indexed Action before committing an engine effect.
    assert_eq!(action.serial(), foreign.serial());
    assert_eq!(action.version(), foreign.version());
    assert_eq!(action.input_tag(), foreign.input_tag());
    let result = with_step(&p, &intent, &action, |step| {
        assert!(step.belongs_to_action(&action));
        assert!(!step.belongs_to_action(&foreign));
        Err::<SdpEngineRead, _>(VoiceError::Unauthorized)
    });
    assert_eq!(result.err(), Some(VoiceError::Unauthorized));
    for original in [&action, &foreign] {
        original.retire_if_unstarted();
        assert!(
            original.handoff_status().engine.settled,
            "no wrong-action engine was admitted"
        );
    }
    phase.close();
    other_phase.close();
}
