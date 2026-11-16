//! Actual registered maximum Open input and final Engine/input/Group/Policy mutexes.
//! Full MLS comparison is exercised separately through ChannelSync's public scoped API.
use super::*;
use crate::voice_v2::crypto_actions::{CryptoAction, SdpInputRead};
use crate::voice_v2::sdp::SdpInputAdmission;

pub(super) fn with_step<T>(
    p: &ProtectedFixture,
    intent: &Arc<InitialSdpIntent>,
    action: &Arc<CryptoAction>,
    publish: impl FnOnce(SdpInputAdmission<'_>) -> Result<T, VoiceError>,
) -> Result<T, VoiceError> {
    let observed = action.observe_sdp_input(intent)?;
    if let Some(SdpInputRead::History(history)) = observed.known_history()? {
        return publish(SdpInputAdmission::history(history));
    }
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
    let observed = action.observe_sdp_input(intent)?;
    publish(SdpInputAdmission::current(observed, current))
}

#[test]
fn sdp_input_maximum_open_loan_is_the_original_input_and_does_not_admit_engine() {
    let (mut p, phase, intent) = fixture();
    let original = super::input(&p).object_hash();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    let SdpInputRead::Data(input) = with_step(&p, &intent, &action, |step| step.commit()).unwrap()
    else {
        panic!("current input")
    };
    assert!(input.belongs_to(&action));
    let CryptoInput::OpenSdp(packet) = input.input() else {
        panic!("Open input")
    };
    assert_eq!(packet.ciphertext().len(), MAX_SDP_PLAINTEXT + 16);
    assert_eq!(packet.object_hash(), original);
    action.retire_if_unstarted();
    assert!(
        action.handoff_status().engine.settled,
        "read did not admit a crypto permit"
    );
    assert!(action.input().is_err());
    assert!(
        matches!(input.input(), CryptoInput::OpenSdp(_)),
        "actual loan survives input compaction"
    );
    drop(input);
    phase.close();
}

#[test]
fn sdp_input_actual_engine_and_input_contention_refuse_without_waiting() {
    for payload in [false, true] {
        let (mut p, phase, intent) = fixture();
        let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
        action.admit_engine(action.engine_bytes()).unwrap();
        let refusal = with_step(&p, &intent, &action, |step| {
            if payload {
                action.with_input_fence_held_for_test(|| step.commit())
            } else {
                action.with_pending(|| step.commit())
            }
        });
        assert_eq!(refusal.err(), Some(VoiceError::Capacity));
        assert!(action.terminal_status().receipt.is_none());
        let SdpInputRead::Data(input) =
            with_step(&p, &intent, &action, |step| step.commit()).unwrap()
        else {
            panic!("same original input after contention")
        };
        assert!(input.belongs_to(&action));
        drop(input);
        action.settle_engine();
        phase.close();
    }
}

#[test]
fn sdp_input_refused_current_gate_never_clones_a_payload_loan_under_outer_native_guards() {
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    let original = action.input().unwrap();
    assert_eq!(
        original.references_for_test(),
        2,
        "Action plus this actual paid probe"
    );
    let observed = action.observe_sdp_input(&intent).unwrap();
    let refused = observed.commit(|| {
        // This runs at the ACTUAL fallible current gate beneath Engine/input. A
        // premature clone here could become the last destructor after Engine unlock
        // while the trusted native caller still holds its own outer admission guards.
        assert_eq!(
            original.references_for_test(),
            2,
            "no new loan before current admission"
        );
        Err(VoiceError::Capacity)
    });
    assert_eq!(refused.err(), Some(VoiceError::Capacity));
    assert_eq!(original.references_for_test(), 2);
    action.retire_if_unstarted();
    assert_eq!(original.references_for_test(), 1);
    drop(original);
    phase.close();
}

#[test]
fn sdp_input_actual_final_group_and_policy_contention_preserve_same_action() {
    for policy in [false, true] {
        let (mut p, phase, intent) = fixture();
        let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
        let (held_tx, held_rx) = std::sync::mpsc::sync_channel(0);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
        let (start_tx, start_rx) = std::sync::mpsc::sync_channel(0);
        let hold = p
            .high
            .native_view_holder_for_test(policy, held_tx, release_rx);
        let holder = std::thread::spawn(move || {
            start_rx.recv().unwrap();
            hold();
        });
        let refusal = with_step(&p, &intent, &action, |step| {
            start_tx.send(()).unwrap();
            held_rx.recv().unwrap(); // park the ACTUAL view only after Clock preparation
            step.commit()
        });
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        assert_eq!(refusal.err(), Some(VoiceError::Capacity));
        assert!(action.handoff_status().engine.receipt.is_none());
        assert!(matches!(
            with_step(&p, &intent, &action, |step| step.commit()).unwrap(),
            SdpInputRead::Data(_)
        ));
        action.retire_if_unstarted();
        phase.close();
    }
}
