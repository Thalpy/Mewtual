//! Actual protected result publication and token/signature history, not boolean race stand-ins.
use super::*;
use crate::voice_v2::resources::{ResourceKind, GROUP_BYTES};

pub(super) fn ready_to_finish() -> (ProtectedFixture, Arc<CryptoAction>) {
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
fn native_key_custody_requires_actual_finish_and_retains_exact_window_without_retiring_low_source()
{
    let (mut p, finish) = ready_to_finish();
    let control = finish.confirmation_control().unwrap();
    let plan = finish.protected_plan().unwrap();
    assert_eq!(
        plan.confirmation_custody(&control, 1_000).err(),
        Some(VoiceError::Capacity)
    );
    let (object, _) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let pool = p.f.pool.clone();
    let fill = pool
        .charge(
            p.f.low.context.scope.group,
            GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = pool.counts();
    let custody = plan
        .confirmation_custody(&control, p.f.clock.monotonic_ms())
        .unwrap();
    assert!(plan.contains_custody(&custody));
    assert_eq!(
        (
            custody.message(),
            custody.object_hash(),
            custody.epoch(),
            custody.transition_until()
        ),
        (object.id(), object.hash(), p.f.low.context.epoch, 31_000)
    );
    assert_eq!(custody.remaining_at(1_000), Ok(30_000));
    assert_eq!(custody.remaining_at(999), Err(VoiceError::Freshness));
    assert_eq!(
        pool.counts(),
        before,
        "fixed original custody has no new reserve or allocation"
    );
    let activity =
        p.f.low
            .join
            .source_packet_activity(
                &object,
                &ca(&p.f.low.context, &p.f.low.lease),
                Arc::new(p.f.clock.clone()),
            )
            .unwrap();
    assert!(
        activity.check(&object).is_ok(),
        "key custody did not close LOW Finish"
    );
    p.f.clock.advance_ms(15_000);
    assert!(
        activity.check(&object).is_err(),
        "old Source authority kept its first deadline"
    );
    assert_eq!(custody.remaining_at(16_000), Ok(15_000));
    assert_eq!(
        plan.confirmation_custody(&control, 16_000)
            .unwrap()
            .transition_until(),
        31_000
    );
    assert_eq!(custody.remaining_at(31_000), Err(VoiceError::Freshness));
    assert_eq!(
        plan.confirmation_custody(&control, 31_000).err(),
        Some(VoiceError::Freshness)
    );
    drop(fill);
}

#[test]
fn native_key_custody_equal_wire_labels_cannot_adopt_another_actual_plan_or_control() {
    let (mut p, finish) = ready_to_finish();
    let control = finish.confirmation_control().unwrap();
    let plan = finish.protected_plan().unwrap();
    protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let custody = plan.confirmation_custody(&control, 1_000).unwrap();
    let (mut foreign, action) = ready_to_finish();
    let other_control = action.confirmation_control().unwrap();
    let other_plan = action.protected_plan().unwrap();
    protected_seal(&mut foreign.f.low, &foreign.low, &action, &foreign.f.clock);
    assert_eq!(
        plan.handshake(),
        other_plan.handshake(),
        "identical public labels are intentional"
    );
    assert!(!other_plan.contains_custody(&custody));
    assert_eq!(
        other_plan.confirmation_custody(&control, 1_000).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        plan.confirmation_custody(&other_control, 1_000).err(),
        Some(VoiceError::Unauthorized)
    );
}

#[test]
fn native_key_custody_survives_high_affine_extraction_but_never_grants_old_action_or_sdp_authority()
{
    let (mut p, finish) = ready_to_finish();
    let (object, plain) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let (_, high) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    let control = high.confirmation_control().unwrap();
    let plan = high.protected_plan().unwrap();
    opened(
        &mut p.f.high,
        &p.high,
        &high,
        &plain,
        &p.f.clock,
        &mut p.f.rng,
    );
    let custody = plan.confirmation_custody(&control, 1_000).unwrap();
    let room = room(&p.f.high);
    let successor =
        p.f.high
            .join
            .prepare_high_sdp_successor(
                &p.high,
                &room,
                p.f.id,
                &[],
                &ca(&p.f.high.context, &p.f.high.lease),
                &p.f.clock,
                |_, _| Ok(()),
            )
            .unwrap();
    assert_eq!(custody.remaining_at(1_000), Ok(30_000));
    assert!(plan.contains_custody(&plan.confirmation_custody(&control, 1_000).unwrap()));
    assert!(!successor.owner.is_live());
    assert!(high.admit_engine(high.engine_bytes()).is_err());
    assert!(p
        .f
        .high
        .join
        .prepare_crypto_chain(p.f.id, &ca(&p.f.high.context, &p.f.high.lease), &p.f.clock)
        .is_err());
    assert_eq!(custody.transition_until(), 31_000);
}

#[test]
fn native_key_custody_end_and_original_epoch_revocation_refuse_new_and_retained_proofs() {
    for end in [false, true] {
        let (mut p, finish) = ready_to_finish();
        let control = finish.confirmation_control().unwrap();
        let plan = finish.protected_plan().unwrap();
        protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
        let custody = plan.confirmation_custody(&control, 1_000).unwrap();
        if end {
            p.f.low.lease.revoke();
        } else {
            p.low
                .sync_view(View {
                    epoch: p.f.low.context.epoch + 1,
                    members: [p.f.low.context.local, p.f.high.context.local].into(),
                    policy: p.f.low.context.scope.policy,
                    eligible: true,
                    indirect: true,
                })
                .unwrap();
        }
        assert_eq!(custody.remaining_at(1_000), Err(VoiceError::Unauthorized));
        assert_eq!(
            plan.confirmation_custody(&control, 1_000).err(),
            Some(VoiceError::Unauthorized)
        );
        assert_eq!(
            custody.transition_until(),
            31_000,
            "historical bound cannot be rewritten"
        );
    }
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
