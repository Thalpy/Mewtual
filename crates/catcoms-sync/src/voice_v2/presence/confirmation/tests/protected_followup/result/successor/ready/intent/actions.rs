//! Actual native first-Offer admission feeds typed Open without copying its paid body.
use super::*;
use crate::voice_v2::crypto_actions::{CryptoInput, CryptoOperation};
use crate::voice_v2::packets::InboundPacket;
mod answer;
mod engine;
mod input;
mod open_results;
mod results;

fn fixture() -> (ProtectedFixture, Arc<Phase>, Arc<InitialSdpIntent>) {
    let (mut p, phase, intent) = before_offer();
    let room = room(&p.f.high);
    let object = super::super::admission::offer(&p, 219, 4);
    super::super::admission::admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
    (p, phase, intent)
}
fn before_offer() -> (ProtectedFixture, Arc<Phase>, Arc<InitialSdpIntent>) {
    let (mut p, keys) = confirmed_with_keys();
    prepare(&mut p, |_, _| Ok(())).unwrap();
    prepare_native_ready(&mut p);
    let phase = keys.prepare_sdp_handoff().unwrap();
    phase.arm().unwrap();
    let intent = intent(&mut p, &keys, &phase, 1).unwrap();
    (p, phase, intent)
}

#[test]
fn original_open_waits_for_delayed_offer_then_retries_same_outcome_without_rearming() {
    use crate::voice_v2::chain::sdp_handoff::NativeRowPurpose;
    use crate::voice_v2::sdp::publication::{History, Prepared};
    let (mut p, phase, intent) = before_offer();
    let fee = phase.native_history(NativeRowPurpose::Open).unwrap();
    let slot = Prepared::new(intent.clone(), fee, None).unwrap().install();
    for _ in 0..3 {
        let mut writer = slot.try_writer().unwrap();
        writer.enter(&intent, None).unwrap();
        let result = open_with_outcome(&mut p, &intent, |_, _| Ok(()), Some(&mut writer));
        assert_eq!(result.err(), Some(VoiceError::Capacity));
        writer.refused(VoiceError::Capacity);
        drop(writer);
        assert!(slot.permits_local_retry());
        assert!(matches!(
            slot.history(),
            History::Refused(VoiceError::Capacity)
        ));
        assert!(p.f.high.join.confirmations.pairs[&p.f.id]
            .sdp_input
            .is_none());
        assert!(p.f.high.join.confirmations.pairs[&p.f.id]
            .first_offer
            .is_none());
    }
    let object = super::super::admission::offer(&p, 219, 4);
    let room = room(&p.f.high);
    super::super::admission::admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
    let mut writer = slot.try_writer().unwrap();
    writer.enter(&intent, None).unwrap();
    let action = open_with_outcome(&mut p, &intent, |_, _| Ok(()), Some(&mut writer)).unwrap();
    let History::Published(same) = slot.history() else {
        panic!("original publication");
    };
    assert!(Arc::ptr_eq(&action, &same));
    assert!(!slot.permits_local_retry());
    action.retire_if_unstarted();
    phase.close();
}

#[test]
fn missing_incoming_sdp_does_not_make_closed_or_expired_original_pc_retryable() {
    for closed in [false, true] {
        let (mut p, phase, intent) = before_offer();
        if closed {
            phase.request_close();
        } else {
            let remaining = intent
                .remaining_at(p.f.clock.now_ms(), p.f.clock.monotonic_ms())
                .unwrap();
            p.f.clock.advance_ms(remaining);
        }
        assert_eq!(
            open(&mut p, &intent, |_, _| Ok(())).err(),
            Some(VoiceError::Unauthorized)
        );
        phase.close();
    }
}

#[test]
fn missing_offer_rechecks_independent_room_and_deadline_after_actual_clock_callback() {
    for revoke_room in [false, true] {
        let (mut p, phase, intent) = before_offer();
        let mut room = room(&p.f.high);
        room.lease = CallLease::default();
        let next = p
            .origin_clock
            .reads
            .load(std::sync::atomic::Ordering::Acquire)
            + 1;
        if revoke_room {
            *p.origin_clock.revoke.lock().unwrap() = Some((next, room.lease.clone()));
        } else {
            let remaining = intent.remaining_at(1000, 1000).unwrap();
            *p.origin_clock.advance.lock().unwrap() = Some((next, remaining));
        }
        assert_eq!(
            p.f.high
                .join
                .prepare_high_sdp_open(
                    &p.high,
                    &room,
                    p.f.id,
                    &intent,
                    &ca(&p.f.high.context, &p.f.high.lease),
                    p._runtimes[1].clock(),
                    |_, _| panic!("waiting must not reach final publication"),
                    None,
                )
                .err(),
            Some(VoiceError::Unauthorized)
        );
        assert!(p.f.high.join.confirmations.pairs[&p.f.id]
            .first_offer
            .is_none());
        phase.close();
    }
}

#[test]
fn delayed_offer_exact_open_history_survives_pc_close_without_recreating_input() {
    let (mut p, phase, intent) = before_offer();
    assert_eq!(
        open(&mut p, &intent, |_, _| panic!("no input")).err(),
        Some(VoiceError::Capacity)
    );
    let object = super::super::admission::offer(&p, 219, 4);
    let room = room(&p.f.high);
    super::super::admission::admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
    let original = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    phase.close();
    let recovered = open(&mut p, &intent, |_, _| panic!("history is not admission")).unwrap();
    assert!(Arc::ptr_eq(&original, &recovered));
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_input
        .is_none());
    assert_eq!(
        recovered.admit_engine(recovered.engine_bytes()),
        Err(VoiceError::Unauthorized)
    );
    original.retire_if_unstarted();
}

#[test]
fn high_first_offer_observes_installed_pc_close_before_during_and_after_admission() {
    for ordering in 0..3 {
        let (mut p, phase, _intent) = before_offer();
        let object = super::super::admission::offer(&p, 219, 4);
        let room = room(&p.f.high);
        let owner = pending(&p);
        if ordering == 0 {
            phase.request_close()
        }
        if ordering < 2 {
            assert_eq!(
                super::super::admission::admit(&mut p, &object, &room, |_, _| {
                    if ordering == 1 {
                        phase.request_close()
                    }
                    Ok(())
                })
                .err(),
                Some(VoiceError::Unauthorized)
            );
            let pair = &p.f.high.join.confirmations.pairs[&p.f.id];
            assert!(pair.first_offer.is_none());
            assert!(pair.sdp_input.is_none());
            assert!(
                !owner.is_live(),
                "closed PC cannot activate native first-Offer lane"
            );
            assert!(!p.f.high.join.ledger.has_sdp_for_test());
        } else {
            let receipt =
                super::super::admission::admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
            phase.request_close();
            assert!(!receipt.authorized_at(1000, 1000));
            assert_eq!(
                super::super::admission::admit(&mut p, &object, &room, |_, _| Ok(())).err(),
                Some(VoiceError::Unauthorized)
            );
        }
        phase.close();
    }
}
fn open(
    p: &mut ProtectedFixture,
    intent: &Arc<InitialSdpIntent>,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<Arc<crate::voice_v2::crypto_actions::CryptoAction>, VoiceError> {
    open_with_outcome(p, intent, final_owner, None)
}
fn open_with_outcome(
    p: &mut ProtectedFixture,
    intent: &Arc<InitialSdpIntent>,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    outcome: Option<&mut crate::voice_v2::sdp::publication::Writer>,
) -> Result<Arc<crate::voice_v2::crypto_actions::CryptoAction>, VoiceError> {
    p.f.high.join.prepare_high_sdp_open(
        &p.high,
        &room(&p.f.high),
        p.f.id,
        intent,
        &ca(&p.f.high.context, &p.f.high.lease),
        p._runtimes[1].clock(),
        final_owner,
        outcome,
    )
}
fn input(p: &ProtectedFixture) -> Arc<InboundPacket> {
    p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_input
        .as_ref()
        .unwrap()
        .clone()
}

#[test]
fn unrecorded_open_chooses_profile_only_at_actual_publication() {
    use crate::voice_v2::chain::sdp_handoff::NativeRowPurpose;
    use crate::voice_v2::sdp::publication::Prepared;
    let (mut p, phase, intent) = fixture();
    assert_eq!(
        open(&mut p, &intent, |_, _| Err(VoiceError::Capacity)).err(),
        Some(VoiceError::Capacity)
    );
    assert!(intent.check_publication_profile(false).is_ok());
    assert!(intent.check_publication_profile(true).is_err());
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    let fee = phase.native_history(NativeRowPurpose::Open).unwrap();
    assert!(matches!(
        Prepared::new(intent.clone(), fee, None),
        Err(VoiceError::Unauthorized)
    ));
    let recovered = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    assert!(Arc::ptr_eq(&action, &recovered));
    action.settle_engine();
    phase.close();
}

#[test]
fn native_sdp_preparation_cannot_mint_a_second_fixed_binding_for_the_same_affine_issue() {
    use crate::voice_v2::crypto_actions::{Allocation, Key};
    use crate::voice_v2::negotiation::Stage;
    use crate::voice_v2::resources::ByteBudget;
    use crate::voice_v2::sdp::action::Direction;
    let (mut p, phase, intent) = fixture();
    let packet = input(&p);
    let pair = &p.f.high.join.confirmations.pairs[&p.f.id];
    let receipt = pair.first_offer.as_ref().unwrap();
    let message = receipt.message;
    let Some(crate::voice_v2::presence::confirmation::successor::Preparation::Transferred {
        owner,
        ..
    }) = &pair.sdp_successor
    else {
        panic!("original native lane")
    };
    let (payload_bytes, engine_bytes) =
        crate::voice_v2::crypto_actions::sdp_bytes(packet.aad().len(), packet.ciphertext().len())
            .unwrap();
    let mut prepared =
        p.f.high
            .join
            .actions
            .prepare(
                Key::SdpOpen(packet.token),
                Allocation {
                    resources: p._runtimes[1].resources(),
                    scope: p.f.high.context.scope,
                    call: p.f.high.lease.clone(),
                    budget: ByteBudget::Exchange {
                        exchange: owner.clone(),
                        stage: Stage::DecryptAction,
                    },
                    history: None,
                    payload_bytes,
                    engine_bytes,
                },
            )
            .unwrap();
    let first = prepared
        .sdp_binding(
            intent.clone(),
            &packet.header,
            message,
            Direction::Open,
            packet.object_hash(),
            1000,
            1000,
        )
        .unwrap();
    let owners = Arc::strong_count(&intent);
    assert_eq!(
        prepared
            .sdp_binding(
                intent.clone(),
                &packet.header,
                message,
                Direction::Open,
                packet.object_hash(),
                1000,
                1000
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        Arc::strong_count(&intent),
        owners,
        "no second fixed record allocation"
    );
    drop((first, prepared));
    // Unpublished abandonment returns the original direction, not another charged record.
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    action.settle_engine();
    phase.close();
}

#[test]
fn native_sdp_open_final_capacity_refusal_preserves_same_paid_event_token_and_direction() {
    let (mut p, phase, intent) = fixture();
    let packet = input(&p);
    let original_token = packet.token;
    let pool = p._runtimes[1].resources();
    let before = pool.counts();
    for _ in 0..3 {
        assert_eq!(
            open(&mut p, &intent, |_, _| Err(VoiceError::Capacity)).err(),
            Some(VoiceError::Capacity)
        );
        assert!(Arc::ptr_eq(&packet, &input(&p)));
        assert_eq!(input(&p).token, original_token);
        assert_eq!(pool.counts(), before);
    }
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    assert_eq!(action.operation(), CryptoOperation::OpenSdp);
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_input
        .is_none());
    let lease = action.input().unwrap();
    let CryptoInput::OpenSdp(retained) = lease.as_input() else {
        panic!("closed SDP input")
    };
    assert!(
        std::ptr::eq(retained, packet.as_ref()),
        "same payload allocation, not another ciphertext copy"
    );
    assert!(Arc::ptr_eq(
        &action,
        &open(&mut p, &intent, |_, _| panic!(
            "exact retry must not republish"
        ))
        .unwrap()
    ));
    assert_eq!(retained.token, original_token);
    action.settle_engine();
    phase.close();
}

#[test]
fn native_sdp_open_pc_close_in_final_fence_keeps_input_and_prevents_action_or_late_engine() {
    let (mut p, phase, intent) = fixture();
    let packet = input(&p);
    assert_eq!(
        open(&mut p, &intent, |_, _| {
            phase.request_close();
            Ok(())
        })
        .err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(Arc::ptr_eq(&packet, &input(&p)));
    assert_eq!(
        open(&mut p, &intent, |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    phase.close();
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    phase.request_close();
    assert_eq!(
        action.admit_engine(action.engine_bytes()),
        Err(VoiceError::Unauthorized)
    );
    action.settle_engine();
    phase.close();
}

#[test]
fn native_sdp_open_maximum_output_and_real_engine_stay_bound_to_pc_closure() {
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    let packet = action.sdp_binding().unwrap();
    assert!(
        !action.matches_sdp_plaintext_commitment(&packet.body_tag_for_test()),
        "Open packet hash is not a Seal commitment"
    );
    assert_eq!(
        action.output_tag(&vec![0; MAX_SDP_PLAINTEXT - 1]),
        Err(VoiceError::Bounds)
    );
    assert_eq!(
        action.output_tag(&vec![0; MAX_SDP_PLAINTEXT + 1]),
        Err(VoiceError::Bounds)
    );
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    let tag = action.output_tag(&bytes).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let Some(crate::voice_v2::presence::confirmation::successor::Preparation::Transferred {
        owner,
        ..
    }) = &p.f.high.join.confirmations.pairs[&p.f.id].sdp_successor
    else {
        panic!("actual successor")
    };
    assert!(matches!(
        owner.charge(crate::voice_v2::negotiation::Stage::Crypto, 1),
        Err(VoiceError::Capacity)
    ));
    phase.request_close();
    assert_eq!(
        action.with_result(&bytes, tag, || panic!(
            "closed PC cannot publish a first result"
        )),
        Err(VoiceError::Unauthorized)
    );
    assert!(
        matches!(
            owner.charge(crate::voice_v2::negotiation::Stage::Crypto, 1),
            Err(VoiceError::Capacity)
        ),
        "closure never refunds the actual occupied Crypto stage"
    );
    action.settle_engine();
    drop(
        owner
            .charge(crate::voice_v2::negotiation::Stage::Crypto, 1)
            .unwrap(),
    );
    phase.close();
}

#[tokio::test]
async fn native_sdp_action_actual_phase_close_wakes_its_original_effect_observer() {
    use std::future::Future;
    use std::task::Poll;
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    let binding = action.sdp_binding().unwrap();
    let mut wait = std::pin::pin!(binding.closed());
    std::future::poll_fn(|cx| {
        assert!(wait.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    phase.close();
    std::future::poll_fn(|cx| {
        assert!(wait.as_mut().poll(cx).is_ready());
        Poll::Ready(())
    })
    .await;
    action.settle_engine();
}

#[tokio::test]
async fn high_offer_receipt_wait_observes_actual_pc_close_without_refunding_held_input() {
    use std::future::Future;
    use std::task::Poll;
    let (p, phase, _) = fixture();
    let pair = &p.f.high.join.confirmations.pairs[&p.f.id];
    let receipt = pair.first_offer.as_ref().unwrap();
    let held = pair.sdp_input.as_ref().unwrap().clone();
    let owner = pending(&p);
    let mut wait = std::pin::pin!(receipt.revoked());
    std::future::poll_fn(|cx| {
        assert!(wait.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    phase.close();
    assert!(!receipt.authorized_at(1000, 1000));
    std::future::poll_fn(|cx| {
        assert!(wait.as_mut().poll(cx).is_ready());
        Poll::Ready(())
    })
    .await;
    // PC close cannot refund the actual packet's independently occupied stage.
    assert_eq!(held.ciphertext().len(), MAX_SDP_PLAINTEXT + 16);
    assert!(matches!(
        owner.charge(crate::voice_v2::negotiation::Stage::RemotePacket, 1),
        Err(VoiceError::Capacity)
    ));
}

#[tokio::test]
async fn high_offer_receipt_wait_uses_earlier_original_pc_ready_deadline_without_phase_close() {
    use std::future::Future;
    use std::task::Poll;
    let (mut p, phase, intent) = before_offer();
    let original_until = 1000 + intent.remaining_at(1000, 1000).unwrap();
    p.f.clock.advance_ms(original_until - 2000);
    let object = super::super::admission::offer(&p, 219, 4);
    let room = room(&p.f.high);
    let receipt = super::super::admission::admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
    assert_eq!(receipt.deadline(), original_until);
    let owner = pending(&p);
    let mut wait = std::pin::pin!(receipt.revoked());
    std::future::poll_fn(|cx| {
        assert!(wait.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    p.f.clock.advance_ms(1000);
    assert!(
        phase.effect().is_live(),
        "deadline, not actual PC close, ends this operation"
    );
    assert!(!receipt.authorized_at(p.f.clock.now_ms(), p.f.clock.monotonic_ms()));
    std::future::poll_fn(|cx| {
        assert!(wait.as_mut().poll(cx).is_ready());
        Poll::Ready(())
    })
    .await;
    assert!(matches!(
        owner.charge(crate::voice_v2::negotiation::Stage::RemotePacket, 1),
        Err(VoiceError::Capacity)
    ));
    phase.close();
}

#[test]
fn scoped_original_open_commits_without_clock_beneath_native_publication() {
    let (mut p, phase, intent) = fixture();
    let clock = p.origin_clock.clone();
    let mut held = None;
    let action =
        p.f.high
            .join
            .prepare_sdp_open_scoped(
                &p.high,
                &room(&p.f.high),
                p.f.id,
                &intent,
                &ca(&p.f.high.context, &p.f.high.lease),
                p._runtimes[1].clock(),
                |_, _| Ok(()),
                None,
                |step| {
                    held = Some(NativeFenceProbe::new(&clock));
                    step.commit()
                },
            )
            .unwrap();
    drop(held);
    action.retire_if_unstarted();
    phase.close();
}
