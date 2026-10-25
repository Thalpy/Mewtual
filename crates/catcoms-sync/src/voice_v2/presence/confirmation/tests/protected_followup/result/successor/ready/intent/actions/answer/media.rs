//! Real native original-round identities/results; AEAD/RTC completion uses the surrounding
//! trusted-output fixture. This is NOT a browser peer transfer or actual media acceptance.
use super::*;
use crate::voice_v2::crypto_actions::{CryptoOutcome, CryptoResultReceipt};
use crate::voice_v2::sdp::{MediaAdmission, SdpMedia};

struct Completed {
    p: ProtectedFixture,
    phase: Arc<Phase>,
    intent: Arc<InitialSdpIntent>,
    seal: Arc<CryptoAction>,
    open: Arc<CryptoAction>,
    receipt: CryptoResultReceipt,
    keys: EstablishedKeyCustody,
    control: Option<crate::voice_v2::chain::ConfirmationControl>,
}
fn completed(ack: bool, settle: bool) -> Completed {
    let (mut p, keys) = confirmed_with_keys();
    let control = p.f.high.join.confirmations.pairs[&p.f.id]
        .chain
        .as_ref()
        .unwrap()
        .workflow()
        .unwrap()
        .confirmation_control();
    super::super::super::super::prepare(&mut p, |_, _| Ok(())).unwrap();
    prepare_native_ready(&mut p);
    let phase = keys.prepare_sdp_handoff().unwrap();
    phase.arm().unwrap();
    let intent = intent(&mut p, &keys, &phase, 1).unwrap();
    let object = super::super::super::super::admission::offer(&p, 219, 4);
    let original_room = room(&p.f.high);
    super::super::super::super::admission::admit(&mut p, &object, &original_room, |_, _| Ok(()))
        .unwrap();
    let open = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    open.admit_engine(open.engine_bytes()).unwrap();
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    let result = submit(&mut p, &open, output(&open, &bytes), |_, _| Ok(())).unwrap();
    open.acknowledge_result(&result.receipt).unwrap();
    open.settle_engine();
    let delivery = result.delivery.unwrap();
    deliver(&p, &open, &delivery, |_, _| Ok(())).unwrap();
    complete(&p, &open, &delivery, |_, _| Ok(())).unwrap();
    let body = b"v=0\r\n";
    let seal = answer(&mut p, &intent, &delivery, body, |_, _| Ok(())).unwrap();
    let ciphertext = vec![44; body.len() + 16];
    seal.admit_engine(seal.engine_bytes()).unwrap();
    let receipt =
        p.f.high
            .join
            .submit_sdp_sealed_result(
                crate::voice_v2::presence::ProtectedSealedResult {
                    actor: &p.high,
                    room: &room(&p.f.high),
                    action: &seal,
                    ciphertext: &ciphertext,
                    claimed_tag: seal.output_tag(&ciphertext).unwrap(),
                },
                &p.f.high.signer,
                &ca(&p.f.high.context, &p.f.high.lease),
                &p.f.clock,
                |_, _| Ok(()),
            )
            .unwrap();
    assert!(matches!(receipt.outcome, CryptoOutcome::Sealed { .. }));
    if ack {
        seal.acknowledge_result(&receipt).unwrap();
    }
    if settle {
        seal.settle_engine();
    }
    Completed {
        p,
        phase,
        intent,
        seal,
        open,
        receipt,
        keys,
        control: Some(control),
    }
}
fn prepare(c: &Completed) -> Result<Arc<SdpMedia>, VoiceError> {
    c.intent.prepare_media(
        &c.phase,
        &c.seal,
        &c.open,
        c.p.f.clock.now_ms(),
        c.p.f.clock.monotonic_ms(),
    )
}
fn admit(
    c: &Completed,
    media: &Arc<SdpMedia>,
    publish: impl FnOnce(MediaAdmission<'_>) -> Result<(), VoiceError>,
) -> Result<(), VoiceError> {
    c.p.f.high.join.with_initial_media_admission(
        &c.p.high,
        &room(&c.p.f.high),
        &c.intent,
        media,
        &c.seal,
        &c.open,
        &ca(&c.p.f.high.context, &c.p.f.high.lease),
        |_, _| Ok(()),
        publish,
    )
}

#[test]
fn media_preparation_requires_actual_settlement_and_ack_before_spending_its_partition() {
    for (ack, settle) in [(false, true), (true, false), (false, false)] {
        let c = completed(ack, settle);
        let resources = c.p._runtimes[1].resources();
        let before = resources.counts();
        assert_eq!(prepare(&c).err(), Some(VoiceError::Unauthorized));
        assert_eq!(resources.counts(), before);
        c.seal.settle_engine();
        c.seal.acknowledge_result(&c.receipt).unwrap();
        let media = prepare(&c).unwrap();
        assert!(!media.was_admitted());
        assert!(!media.is_current());
        admit(&c, &media, |step| step.commit()).unwrap();
        assert!(media.is_current());
        c.phase.close();
    }
}

#[test]
fn media_first_admission_requires_commit_and_post_commit_error_cannot_erase_it() {
    let c = completed(true, true);
    let media = prepare(&c).unwrap();
    assert_eq!(admit(&c, &media, |_| Ok(())), Err(VoiceError::Unauthorized));
    assert!(!media.was_admitted());
    assert_eq!(
        admit(&c, &media, |step| {
            assert!(step.matches(&media, &c.intent));
            step.commit()?;
            Err(VoiceError::Capacity)
        }),
        Ok(())
    );
    assert!(media.is_current());
    media.close();
    assert!(
        media.was_admitted(),
        "historical admission survives its irreversible close"
    );
    assert!(!media.is_current());
    assert_eq!(
        admit(&c, &media, |step| step.commit()),
        Err(VoiceError::Unauthorized)
    );
    c.phase.close();
}

#[test]
fn prepared_media_cannot_outlive_first_ready_phase_or_successor_authority() {
    for fence in ["ready", "phase", "successor", "call", "media"] {
        let c = completed(true, true);
        let media = prepare(&c).unwrap();
        match fence {
            "ready" => {
                c.p.f.clock.advance_ms(30_000);
            }
            "phase" => c.phase.request_close(),
            "successor" => {
                c.keys.close_successor_effects();
            }
            "call" => c.p.f.high.lease.revoke(),
            "media" => media.close(),
            _ => unreachable!(),
        }
        assert_eq!(
            admit(&c, &media, |step| step.commit()),
            Err(VoiceError::Unauthorized),
            "{fence}"
        );
        assert!(!media.was_admitted());
        c.phase.close();
    }
}

#[test]
fn admitted_media_survives_only_old_negotiation_retirement_not_call_revocation() {
    let mut c = completed(true, true);
    let media = prepare(&c).unwrap();
    admit(&c, &media, |step| step.commit()).unwrap();
    c.p.f.clock.advance_ms(30_000);
    c.phase.close();
    c.keys.close_successor_effects();
    drop(c.control.take().unwrap().retire_all());
    assert!(!c
        .intent
        .current_at(c.p.f.clock.now_ms(), c.p.f.clock.monotonic_ms()));
    assert!(
        media.is_current(),
        "an admitted PC is independent of the original Ready/phase"
    );
    c.p.f.high.lease.revoke();
    assert!(!media.is_current());
    assert!(media.was_admitted());
}

#[test]
fn media_first_admission_rechecks_original_key_release_after_borrowed_step_preparation() {
    let c = completed(true, true);
    let media = prepare(&c).unwrap();
    assert_eq!(
        admit(&c, &media, |step| {
            c.keys.close_successor_effects();
            let _native = NativeFenceProbe::new(&c.p.origin_clock);
            step.commit()
        }),
        Err(VoiceError::Unauthorized)
    );
    assert!(!media.was_admitted());
    c.phase.close();
}

#[test]
fn media_original_clock_revocation_or_expiry_before_final_origin_cannot_admit() {
    use std::sync::atomic::Ordering;
    for expire in [false, true] {
        let c = completed(true, true);
        let media = prepare(&c).unwrap();
        let next = c.p.origin_clock.reads.load(Ordering::Acquire) + 1;
        if expire {
            *c.p.origin_clock.advance.lock().unwrap() = Some((next, 30_000));
        } else {
            *c.p.origin_clock.revoke.lock().unwrap() = Some((next, c.p.f.high.lease.clone()));
        }
        assert_eq!(
            admit(&c, &media, |step| step.commit()),
            Err(VoiceError::Unauthorized)
        );
        assert!(!media.was_admitted());
        assert!(c.p.origin_clock.revoke.lock().unwrap().is_none());
        assert!(c.p.origin_clock.advance.lock().unwrap().is_none());
        c.phase.close();
    }
}

#[test]
fn media_original_actor_session_epoch_policy_and_pair_revocation_are_still_effective() {
    for fence in ["session", "epoch", "policy", "pair"] {
        let mut c = completed(true, true);
        let media = prepare(&c).unwrap();
        admit(&c, &media, |step| step.commit()).unwrap();
        match fence {
            "session" => c.p._runtimes[1].revoke_session(),
            "pair" => c.p.f.high.join.confirmations.pairs[&c.p.f.id].pin.revoke(),
            "epoch" | "policy" => {
                let mut view = View {
                    epoch: c.p.f.high.context.epoch,
                    members: [c.p.f.low.context.local, c.p.f.high.context.local].into(),
                    policy: c.p.f.high.context.scope.policy,
                    eligible: true,
                    indirect: true,
                };
                if fence == "epoch" {
                    view.epoch += 1;
                } else {
                    let mut pin = view.policy.unwrap_or([0; 32]);
                    pin[0] ^= 1;
                    view.policy = Some(pin);
                }
                c.p.high.sync_view(view).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(!media.is_current(), "{fence}");
        c.phase.close();
    }
}

#[test]
fn media_close_winning_after_final_preparation_never_admits_or_revives() {
    let c = completed(true, true);
    let media = prepare(&c).unwrap();
    assert_eq!(
        admit(&c, &media, |step| {
            media.close();
            step.commit()
        }),
        Err(VoiceError::Unauthorized)
    );
    assert!(!media.was_admitted());
    assert_eq!(
        admit(&c, &media, |step| step.commit()),
        Err(VoiceError::Unauthorized)
    );
    c.phase.close();
}

#[test]
fn media_wrong_original_phase_intent_actions_and_call_refuse_without_allocating() {
    let c = completed(true, true);
    let other = completed(true, true);
    let resources = c.p._runtimes[1].resources();
    let before = resources.counts();
    assert_eq!(
        c.intent
            .prepare_media(&other.phase, &c.seal, &c.open, 1000, 1000)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        c.intent
            .prepare_media(&c.phase, &other.seal, &c.open, 1000, 1000)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        c.intent
            .prepare_media(&c.phase, &c.seal, &other.open, 1000, 1000)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(resources.counts(), before);
    let media = prepare(&c).unwrap();
    assert!(!media.uses_intent(&other.intent));
    assert_eq!(
        admit(&other, &media, |_| panic!(
            "foreign media owner reached native hook"
        )),
        Err(VoiceError::Unauthorized)
    );
    let foreign = CallLease::default();
    assert_eq!(
        c.p.f.high.join.with_initial_media_admission(
            &c.p.high,
            &room(&c.p.f.high),
            &c.intent,
            &media,
            &c.seal,
            &c.open,
            &ca(&c.p.f.high.context, &foreign),
            |_, _| Ok(()),
            |_| panic!("foreign call reached hook"),
        ),
        Err(VoiceError::Unauthorized)
    );
    admit(&c, &media, |step| step.commit()).unwrap();
    c.phase.close();
    other.phase.close();
}

#[test]
fn media_admission_retries_same_paid_candidate_under_actual_group_and_policy_contention() {
    for policy in [false, true] {
        let c = completed(true, true);
        let media = prepare(&c).unwrap();
        let (held_tx, held_rx) = std::sync::mpsc::sync_channel(0);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
        let (start_tx, start_rx) = std::sync::mpsc::sync_channel(0);
        let hold =
            c.p.high
                .native_view_holder_for_test(policy, held_tx, release_rx);
        let holder = std::thread::spawn(move || {
            if start_rx.recv().is_ok() {
                hold();
            }
        });
        let result = admit(&c, &media, |step| {
            start_tx.send(()).unwrap();
            held_rx.recv().unwrap();
            let _native = NativeFenceProbe::new(&c.p.origin_clock);
            step.commit()
        });
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        assert_eq!(result, Err(VoiceError::Capacity));
        assert!(!media.was_admitted());
        let bytes = c.p._runtimes[1].resources().counts();
        admit(&c, &media, |step| {
            let _native = NativeFenceProbe::new(&c.p.origin_clock);
            step.commit()
        })
        .unwrap();
        assert_eq!(c.p._runtimes[1].resources().counts(), bytes);
        c.phase.close();
    }
}

#[test]
fn media_uses_original_prepaid_capacity_even_with_all_group_and_process_bytes_occupied() {
    use crate::voice_v2::resources::{ResourceKind, GROUP_BYTES, PROCESS_BYTES};
    let c = completed(true, true);
    let resources = c.p._runtimes[1].resources();
    let group = c.p.f.high.context.scope.group;
    let mut pressure = vec![resources
        .charge(
            group,
            GROUP_BYTES - resources.counts().0,
            ResourceKind::Retained,
        )
        .unwrap()];
    for index in 1..4 {
        pressure.push(
            resources
                .charge([index; 32], GROUP_BYTES, ResourceKind::Retained)
                .unwrap(),
        );
    }
    assert_eq!(resources.counts().0, PROCESS_BYTES);
    let media = prepare(&c).unwrap();
    let work = media.prepay_work(1024).unwrap();
    assert_eq!(resources.counts().0, PROCESS_BYTES);
    admit(&c, &media, |step| step.commit()).unwrap();
    assert!(media.is_current());
    c.phase.close();
    drop((work, pressure));
}

#[test]
fn detached_media_pins_no_strong_original_intent_phase_or_variable_negotiation_graph() {
    use crate::voice_v2::sdp::media::BYTES;
    let c = completed(true, true);
    let media = prepare(&c).unwrap();
    admit(&c, &media, |step| step.commit()).unwrap();
    let resources = c.p._runtimes[1].resources();
    let intent = Arc::downgrade(&c.intent);
    let phase = Arc::downgrade(&c.phase);
    c.phase.close();
    drop(c);
    assert!(intent.upgrade().is_none());
    assert!(phase.upgrade().is_none());
    assert!(
        !media.is_current(),
        "dropping the real call still revokes admitted media"
    );
    // The weak Intent allocation is paid inside media metadata, while only the small
    // original PlanBinding keeps its own2KiB fee. No old Phase/Exchange reserve survives.
    assert_eq!(resources.counts().0, BYTES + 2048);
    let held = media.prepay_work(1024).unwrap();
    media.close();
    drop(media);
    assert_eq!(
        resources.counts().0,
        BYTES,
        "actual child loan keeps its original detached capacity"
    );
    drop(held);
    assert_eq!(resources.counts().0, 0);
}

#[test]
fn media_future_work_boundary_subtracts_its_child_header_and_fixed_metadata() {
    let c = completed(true, true);
    let media = prepare(&c).unwrap();
    let bytes = SdpMedia::work_capacity();
    assert!(bytes < 4096);
    let loan = media.prepay_work(bytes).unwrap();
    assert_eq!(media.prepay_work(1).err(), Some(VoiceError::Capacity));
    drop(loan);
    assert_eq!(
        media.prepay_work(bytes + 1).err(),
        Some(VoiceError::Capacity)
    );
    drop(media.prepay_work(bytes).unwrap());
    c.phase.close();
}
