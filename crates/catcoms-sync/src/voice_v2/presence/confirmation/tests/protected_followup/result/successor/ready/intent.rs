//! Actual protected Finish and signed Ready produce these intent tests. Only the earlier
//! confirmation AEAD is mocked; no SDK/PC/physical media acceptance is claimed here.
use super::*;
use crate::voice_v2::chain::sdp_handoff::{Phase, Status};
use crate::voice_v2::chain::EstablishedKeyCustody;
use crate::voice_v2::sdp::InitialSdpIntent;

fn confirmed_with_keys() -> (ProtectedFixture, EstablishedKeyCustody) {
    let (p, key, _) = confirmed_with_keys_and_plan();
    (p, key)
}
fn confirmed_with_keys_and_plan() -> (
    ProtectedFixture,
    EstablishedKeyCustody,
    crate::voice_v2::crypto_actions::ProtectedCryptoPlan,
) {
    let (mut p, finish) = super::super::super::transition::ready_to_finish();
    let (object, plain) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let (_, action) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    // Native captures these opaque original owners before the actual action returns;
    // a settled action deliberately cannot reconstruct a new confirmation Control.
    let plan = action.protected_plan().unwrap();
    let control = action.confirmation_control().unwrap();
    opened(
        &mut p.f.high,
        &p.high,
        &action,
        &plain,
        &p.f.clock,
        &mut p.f.rng,
    );
    let custody = plan.confirmation_custody(&control, 1_000).unwrap();
    let key = plan.established_key_custody(&custody, 1_000).unwrap();
    (p, key, plan)
}
fn prepare_native_ready(p: &mut ProtectedFixture) {
    let signer = Signer::original(p);
    let mut rng = Rng::new();
    p.f.high
        .join
        .install_high_ready_receive(
            &p.high,
            &room(&p.f.high),
            p.f.id,
            &[],
            &signer,
            &ca(&p.f.high.context, &p.f.high.lease),
            &p.f.clock,
            &mut rng,
            |_, _| Ok(()),
        )
        .unwrap();
}
fn intent(
    p: &mut ProtectedFixture,
    keys: &EstablishedKeyCustody,
    phase: &Arc<Phase>,
    generation: u64,
) -> Result<Arc<InitialSdpIntent>, VoiceError> {
    p.f.high.join.prepare_initial_sdp_intent(
        &p.high,
        &room(&p.f.high),
        p.f.id,
        keys,
        phase.effect(),
        generation,
        &ca(&p.f.high.context, &p.f.high.lease),
    )
}

#[test]
fn original_sdp_storage_records_group_and_process_refusal_then_reuses_the_same_key_custody() {
    use crate::voice_v2::resources::{ResourceKind, GROUP_BYTES, PROCESS_BYTES};
    let (mut p, keys) = confirmed_with_keys();
    prepare(&mut p, |_, _| Ok(())).unwrap();
    let resources = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    let request = crate::voice_v2::chain::sdp_handoff::BYTES;
    let baseline = resources.counts();
    let group_bytes = resources.group_counts(group).unwrap().0;
    let pressure = resources
        .charge(
            group,
            GROUP_BYTES - group_bytes - request + 1,
            ResourceKind::Retained,
        )
        .unwrap();
    let mut observation = None;
    assert_eq!(
        keys.prepare_sdp_handoff_recording_refusal(&mut observation)
            .err(),
        Some(VoiceError::Capacity)
    );
    let group_refusal = observation.unwrap();
    assert_eq!(group_refusal.requested_bytes, request);
    assert!(group_refusal.group_bytes_exceeded);
    assert!(!group_refusal.process_bytes_exceeded);
    assert!(keys.is_current());
    drop(pressure);
    assert_eq!(resources.counts(), baseline);

    let pressure = resources
        .charge_process_retained(PROCESS_BYTES - baseline.0 - request + 1)
        .unwrap();
    assert_eq!(
        keys.prepare_sdp_handoff_recording_refusal(&mut observation)
            .err(),
        Some(VoiceError::Capacity)
    );
    let process_refusal = observation.unwrap();
    assert!(process_refusal.process_bytes_exceeded);
    assert!(!process_refusal.group_bytes_exceeded);
    assert_eq!(process_refusal.group_bytes, Some(group_bytes));
    drop(pressure);

    // Both refusals precede Phase/action publication. Only unrelated pressure
    // returned; the original protected key/domain and its clocks are unchanged.
    let phase = keys
        .prepare_sdp_handoff_recording_refusal(&mut observation)
        .unwrap();
    assert!(observation.is_none());
    assert!(keys.contains_sdp_handoff(&phase));
    assert_eq!(resources.counts().0, baseline.0 + request);
    phase.close();
    drop(phase);
    assert_eq!(resources.counts(), baseline);
    keys.close_successor_effects();
    observation = Some(process_refusal);
    assert_eq!(
        keys.prepare_sdp_handoff_recording_refusal(&mut observation)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(
        observation.is_none(),
        "stale refusal cannot describe a later authorization failure"
    );
}

#[test]
fn initial_sdp_intent_requires_original_ready_key_and_armed_phase_without_allocating_on_refusal() {
    let (mut p, keys) = confirmed_with_keys();
    let (other, foreign) = confirmed_with_keys();
    // Prepare/extract the core before paying the new handoff envelope.
    prepare(&mut p, |_, _| Ok(())).unwrap();
    let phase = keys.prepare_sdp_handoff().unwrap();
    let resources = p._runtimes[1].resources();
    let before = resources.counts();
    assert_eq!(
        intent(&mut p, &keys, &phase, 1).err(),
        Some(VoiceError::Unauthorized)
    );
    phase.arm().unwrap();
    // An armed phase is custody only; no native signed Ready/receive promise exists yet.
    assert_eq!(
        intent(&mut p, &keys, &phase, 1).err(),
        Some(VoiceError::Unauthorized)
    );
    prepare_native_ready(&mut p);
    assert_eq!(
        intent(&mut p, &foreign, &phase, 1).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        intent(&mut p, &keys, &phase, 2).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        resources.counts(),
        before,
        "refusal creates no extra phase/action/intent fee"
    );
    let issued = intent(&mut p, &keys, &phase, 1).unwrap();
    assert!(issued.current_at(1_000, 1_000));
    assert_eq!(issued.generation(), 1);
    assert!(!issued.local_is_low());
    assert_eq!(
        issued.round().ready,
        cache(&p).artifact_for_test().unwrap().0.id()
    );
    assert!(issued.uses_phase(&phase));
    let first_bound = issued.remaining_at(1_000, 1_000).unwrap();
    assert!(first_bound > 0 && first_bound <= 30_000);
    let retry = intent(&mut p, &keys, &phase, 1).unwrap();
    assert!(
        Arc::ptr_eq(&issued, &retry),
        "lost reply recovers one original intent"
    );
    assert_eq!(
        resources.counts(),
        before,
        "fixed existing metadata covers the single intent"
    );
    drop(other);
    phase.close();
}

#[test]
fn initial_sdp_intent_close_fences_effects_while_real_copy_still_blocks_storage_drainage() {
    let (mut p, keys) = confirmed_with_keys();
    prepare_native_ready(&mut p);
    let phase = keys.prepare_sdp_handoff().unwrap();
    phase.arm().unwrap();
    let issued = intent(&mut p, &keys, &phase, 1).unwrap();
    let copy = phase.copy(128).unwrap().adopt(vec![42; 128]).unwrap();
    let foreign_phase = keys.prepare_sdp_handoff().unwrap();
    foreign_phase.arm().unwrap();
    assert!(!issued.uses_phase(&foreign_phase));
    assert_eq!(
        intent(&mut p, &keys, &foreign_phase, 1).err(),
        Some(VoiceError::RetryMismatch)
    );
    assert_eq!(
        intent(&mut p, &keys, &phase, 2).err(),
        Some(VoiceError::RetryMismatch)
    );
    phase.request_close();
    assert!(!issued.current_at(1_000, 1_000));
    let recovered = intent(&mut p, &keys, &phase, 1).unwrap();
    assert!(Arc::ptr_eq(&issued, &recovered));
    assert!(
        !recovered.current_at(1_000, 1_000),
        "recovery cannot restore effects"
    );
    phase.close();
    assert_eq!(phase.status(), Status::Closing);
    assert_eq!(copy.bytes(), &[42; 128]);
    drop(copy);
    assert_eq!(phase.status(), Status::Drained);
    foreign_phase.close();
}

#[test]
fn initial_sdp_intent_history_never_renews_its_original_ready_window_or_key_domain() {
    let (mut p, keys) = confirmed_with_keys();
    prepare_native_ready(&mut p);
    let phase = keys.prepare_sdp_handoff().unwrap();
    phase.arm().unwrap();
    let issued = intent(&mut p, &keys, &phase, 1).unwrap();
    let remaining = issued.remaining_at(1_000, 1_000).unwrap();
    p.f.clock.advance_ms(remaining);
    assert!(!issued.current_at(p.f.clock.now_ms(), p.f.clock.monotonic_ms()));
    let same = intent(&mut p, &keys, &phase, 1).unwrap();
    assert!(Arc::ptr_eq(&issued, &same));
    assert!(same
        .remaining_at(p.f.clock.now_ms(), p.f.clock.monotonic_ms())
        .is_err());
    assert!(
        keys.is_current(),
        "old finite expiry does not erase adopted keys"
    );
    keys.close_successor_effects();
    assert!(
        !same.current_at(1_000, 1_000),
        "old-clock retry cannot revive key release"
    );
    phase.close();
}

#[test]
fn historical_initial_sdp_intent_keeps_ready_observations_paid_until_their_actual_drop() {
    let (mut p, keys) = confirmed_with_keys();
    prepare_native_ready(&mut p);
    let resources = p._runtimes[1].resources();
    let phase = keys.prepare_sdp_handoff().unwrap();
    phase.arm().unwrap();
    let mut issued = intent(&mut p, &keys, &phase, 1).unwrap();
    phase.close();
    drop(phase);
    drop(keys);
    drop(p);
    // Pair/receiver/Phase/keys no longer keep any alternative EffectObserver alive.
    // The sentinel runs after Ready is destroyed but before the last paid anchor, using
    // Rust's actual field destruction rather than a manual imitation of the intended order.
    let expected = resources.counts().0;
    assert!(expected >= 4096);
    Arc::get_mut(&mut issued)
        .expect("only the historical intent remains")
        .before_effect_drop = Some(crate::voice_v2::sdp::AssertCredit {
        resources: resources.clone(),
        expected,
    });
    drop(issued);
    assert!(resources.counts().0 <= expected - 4096);
}

#[test]
fn initial_sdp_intent_rechecks_native_origin_on_every_clock_boundary_before_publication() {
    use std::sync::atomic::Ordering;
    fn ready() -> (ProtectedFixture, EstablishedKeyCustody, Arc<Phase>) {
        let (mut p, keys) = confirmed_with_keys();
        prepare_native_ready(&mut p);
        let phase = keys.prepare_sdp_handoff().unwrap();
        phase.arm().unwrap();
        (p, keys, phase)
    }
    let (mut baseline, keys, phase) = ready();
    let before = baseline.origin_clock.reads.load(Ordering::Acquire);
    intent(&mut baseline, &keys, &phase, 1).unwrap();
    let reads = baseline.origin_clock.reads.load(Ordering::Acquire) - before;
    assert!(
        reads >= 2,
        "preparation and actual origin both sample native time"
    );
    phase.close();
    for at in 1..=reads {
        for expire in [false, true] {
            let (mut p, keys, phase) = ready();
            let before = p.origin_clock.reads.load(Ordering::Acquire);
            if expire {
                *p.origin_clock.advance.lock().unwrap() = Some((before + at, 30_000));
            } else {
                *p.origin_clock.revoke.lock().unwrap() =
                    Some((before + at, p.f.high.lease.clone()));
            }
            assert_eq!(
                intent(&mut p, &keys, &phase, 1).err(),
                Some(VoiceError::Unauthorized),
                "native boundary {at}/{reads}, expiry {expire}"
            );
            assert!(
                p.f.high.join.confirmations.pairs[&p.f.id]
                    .sdp_intent
                    .is_none(),
                "no partial publication after final native refusal"
            );
            phase.close();
        }
    }
}

mod actions;
