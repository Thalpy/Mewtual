//! Actual protected result publication and token/signature history, not boolean race stand-ins.
use super::*;
use crate::voice_v2::resources::{ResourceKind, GROUP_BYTES};

fn ready_to_finish() -> (ProtectedFixture, Arc<CryptoAction>) {
    let mut p = fixture();
    let hello = start(&mut p, None);
    let (object, plain) = protected_seal(&mut p.f.low, &p.low, &hello, &p.f.clock);
    let (_, high_open) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    opened(
        &mut p.f.high,
        &p.high,
        &high_open,
        &plain,
        &p.f.clock,
        &mut p.f.rng,
    );
    let proof = respond(
        &mut p.f.high,
        &p.high,
        p.f.id,
        &high_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    let (object, plain) = protected_seal(&mut p.f.high, &p.high, &proof, &p.f.clock);
    let (_, low_open) = admit(&mut p.f.low, &p.low, object, &p.f.clock);
    opened(
        &mut p.f.low,
        &p.low,
        &low_open,
        &plain,
        &p.f.clock,
        &mut p.f.rng,
    );
    let finish = respond(
        &mut p.f.low,
        &p.low,
        p.f.id,
        &low_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    (p, finish)
}
fn fingerprint(
    control: &crate::voice_v2::chain::ConfirmationControl,
) -> Option<([u8; 32], [u8; 32], u64)> {
    control
        .confirmed_finish()
        .map(|row| (row.message(), row.object_hash(), row.transition_until()))
}

#[test]
fn await_ready_actual_finish_keeps_native_identity_and_first_expiry_through_lost_replies_and_old_source_expiry(
) {
    let (mut p, finish) = ready_to_finish();
    let control = finish.confirmation_control().unwrap();
    let pool = p.f.pool.clone();
    let fill = pool
        .charge(
            p.f.low.context.scope.group,
            GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let (object, plain) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let original = (object.id(), object.hash(), 31_000);
    assert_eq!(fingerprint(&control), Some(original));
    let activity =
        p.f.low
            .join
            .source_packet_activity(
                &object,
                &ca(&p.f.low.context, &p.f.low.lease),
                Arc::new(p.f.clock.clone()),
            )
            .unwrap();
    let receipt = finish.terminal_status().receipt.unwrap();
    let cipher = vec![22; plain.len() + 16];
    p.f.clock.advance_ms(1_000);
    let room = room(&p.f.low);
    let mut request = |bytes: &[u8]| {
        p.f.low.join.submit_protected_confirmation_result(
            ProtectedSealedResult {
                actor: &p.low,
                room: &room,
                action: &finish,
                ciphertext: bytes,
                claimed_tag: finish.output_tag(bytes).unwrap(),
            },
            &p.f.low.signer,
            &ca(&p.f.low.context, &p.f.low.lease),
            &p.f.clock,
            |_, _| panic!("history must not arm or publish again"),
        )
    };
    let retry = request(&cipher).unwrap();
    assert!(receipt.same_transition(&retry));
    let mut changed = cipher.clone();
    changed[0] ^= 1;
    assert_eq!(request(&changed).err(), Some(VoiceError::RetryMismatch));
    assert_eq!(fingerprint(&control), Some(original));
    // Original Source/action deadlines are unchanged although both collectors preserve custody.
    p.f.clock.advance_ms(14_000);
    assert!(activity.check(&object).is_err());
    p.f.low.join.collect_expired(&p.f.clock);
    pool.collect_workflows(p.f.clock.monotonic_ms());
    assert!(!control.is_drained());
    assert_eq!(fingerprint(&control), Some(original));
    drop(fill);
    p.f.clock.advance_ms(15_000);
    p.f.low.join.collect_expired(&p.f.clock);
    pool.collect_workflows(p.f.clock.monotonic_ms());
    // A retained old source activity remains actual custody; history is not drainage proof.
    assert_eq!(fingerprint(&control), Some(original));
    drop(activity);
}

#[test]
fn await_ready_actual_high_finish_records_authenticated_id_under_full_pressure_and_rejected_plaintext_never_arms(
) {
    for valid in [true, false] {
        let (mut p, finish) = ready_to_finish();
        let (object, mut plain) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
        let expected = (object.id(), object.hash(), 31_000);
        let (_, action) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
        let control = action.confirmation_control().unwrap();
        let resources = p._runtimes[1].resources();
        let _full = resources
            .charge(
                p.f.high.context.scope.group,
                GROUP_BYTES - resources.counts().0,
                ResourceKind::Retained,
            )
            .unwrap();
        if !valid {
            plain[0] ^= 1;
        }
        let result = opened(
            &mut p.f.high,
            &p.high,
            &action,
            &plain,
            &p.f.clock,
            &mut p.f.rng,
        );
        assert_eq!(fingerprint(&control), valid.then_some(expected));
        assert_eq!(
            p.f.high.join.confirmations.pairs[&p.f.id].established,
            valid
        );
        if valid {
            assert_eq!(result.outcome, CryptoOutcome::Opened);
        } else {
            assert!(matches!(result.outcome, CryptoOutcome::Rejected(_)));
        }
        p.f.clock.advance_ms(1_000);
        let retry = checked(
            &mut p.f.high,
            &p.high,
            (&action, &plain),
            &p.f.clock,
            &mut p.f.rng,
            |_, _| panic!("history only"),
        )
        .unwrap();
        assert!(retry.same_transition(&result));
        assert_eq!(fingerprint(&control), valid.then_some(expected));
    }
}

#[test]
fn await_ready_actual_result_refuses_a_collector_claim_before_any_signature_or_token_effect() {
    for high in [false, true] {
        let (mut p, finish) = ready_to_finish();
        if high {
            let (object, plain) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
            let (_, action) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
            let chain = p.f.high.join.confirmations.pairs[&p.f.id]
                .chain
                .as_ref()
                .unwrap()
                .clone();
            let control = action.confirmation_control().unwrap();
            let input = action.input().unwrap();
            let CryptoInput::DecryptConfirmation(packet) = input.as_input() else {
                panic!("original native decrypt")
            };
            let token = packet.token;
            action.admit_engine(action.engine_bytes()).unwrap();
            let result = checked(
                &mut p.f.high,
                &p.high,
                (&action, &plain),
                &p.f.clock,
                &mut p.f.rng,
                |_, _| {
                    // Park the REAL collector after its atomic expiry claim but before cleanup,
                    // which can wait for this actual Engine. Original action leases remain live.
                    assert!(chain.claim_window_expiry_for_test(16_000));
                    assert!(chain.lease().is_live());
                    Ok(())
                },
            );
            assert_eq!(result.err(), Some(VoiceError::Unauthorized));
            assert!(action.terminal_status().receipt.is_none());
            assert!(p
                .f
                .high
                .join
                .ledger
                .validate_inbound_commit(
                    token,
                    packet.aad(),
                    packet.header.sequence,
                    packet.ciphertext(),
                    &ca(&p.f.high.context, &p.f.high.lease),
                    &p.f.clock
                )
                .is_ok());
            assert!(!p.f.high.join.confirmations.pairs[&p.f.id].established);
            assert!(fingerprint(&control).is_none());
            action.settle_engine();
            chain.close();
        } else {
            let input = finish.input().unwrap();
            let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
                panic!("original native seal")
            };
            let cipher = vec![22; flight.plaintext.len() + 16];
            let chain = p.f.low.join.confirmations.pairs[&p.f.id]
                .chain
                .as_ref()
                .unwrap()
                .clone();
            let control = finish.confirmation_control().unwrap();
            let room = room(&p.f.low);
            finish.admit_engine(finish.engine_bytes()).unwrap();
            let result = p.f.low.join.submit_protected_confirmation_result(
                ProtectedSealedResult {
                    actor: &p.low,
                    room: &room,
                    action: &finish,
                    ciphertext: &cipher,
                    claimed_tag: finish.output_tag(&cipher).unwrap(),
                },
                &p.f.low.signer,
                &ca(&p.f.low.context, &p.f.low.lease),
                &p.f.clock,
                |_, _| {
                    assert!(chain.claim_window_expiry_for_test(16_000));
                    assert!(chain.lease().is_live());
                    Ok(())
                },
            );
            assert_eq!(result.err(), Some(VoiceError::Unauthorized));
            assert!(finish.terminal_status().receipt.is_none());
            let pair = &p.f.low.join.confirmations.pairs[&p.f.id];
            assert!(!pair.established);
            assert!(pair.action.as_ref().unwrap().signed.is_none());
            assert!(fingerprint(&control).is_none());
            finish.settle_engine();
            chain.close();
        }
    }
}
