//! Root/native HIGH Answer ordering. SDK application is represented only by the trusted
//! completion call here; these tests do not claim a browser SDK or a real RTC negotiation.
use super::open_results::{deliver, output, submit};
use super::*;
use crate::voice_v2::crypto_actions::CryptoAction;
use crate::voice_v2::sdp::SdpDelivery;

type Opened = (
    ProtectedFixture,
    Arc<Phase>,
    Arc<InitialSdpIntent>,
    Arc<CryptoAction>,
    Arc<SdpDelivery>,
);
fn opened() -> Opened {
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    let result = submit(&mut p, &action, output(&action, &bytes), |_, _| Ok(())).unwrap();
    action.acknowledge_result(&result.receipt).unwrap();
    action.settle_engine();
    (p, phase, intent, action, result.delivery.unwrap())
}
fn complete(
    p: &ProtectedFixture,
    action: &Arc<CryptoAction>,
    delivery: &Arc<SdpDelivery>,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<(), VoiceError> {
    p.f.high.join.complete_sdp_delivery(
        &p.high,
        &room(&p.f.high),
        action,
        delivery,
        &ca(&p.f.high.context, &p.f.high.lease),
        final_owner,
    )
}
fn answer(
    p: &mut ProtectedFixture,
    intent: &Arc<InitialSdpIntent>,
    delivery: &Arc<SdpDelivery>,
    bytes: &[u8],
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<Arc<CryptoAction>, VoiceError> {
    p.f.high.join.prepare_high_sdp_answer(
        &p.high,
        &room(&p.f.high),
        p.f.id,
        intent,
        delivery,
        bytes,
        &ca(&p.f.high.context, &p.f.high.lease),
        p._runtimes[1].clock(),
        &mut p.f.rng,
        final_owner,
    )
}
fn generic(
    p: &mut ProtectedFixture,
    intent: &Arc<InitialSdpIntent>,
    delivery: &Arc<SdpDelivery>,
) -> Result<ReservedPacket, VoiceError> {
    p.f.high.join.ledger.reserve(
        *p.f.high.join.announcement.as_ref().unwrap().public_key(),
        ReserveRequest {
            handshake: p.f.id,
            kind: EncryptedKind::AnswerRound(intent.round()),
            offer: Some(delivery.offer()),
            ciphertext_bytes: 21,
        },
        &ca(&p.f.high.context, &p.f.high.lease),
        &p.f.clock,
        &mut p.f.rng,
    )
}

#[test]
fn high_answer_requires_applied_original_open_and_generic_reservation_always_refuses() {
    let (mut p, phase, intent, open, delivery) = opened();
    let before = p.f.high.join.ledger.initial_offer_state_for_test(p.f.id);
    assert_eq!(
        complete(&p, &open, &delivery, |_, _| Ok(())),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        answer(&mut p, &intent, &delivery, b"v=0\r\n", |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    deliver(&p, &open, &delivery, |_, _| Ok(())).unwrap();
    assert_eq!(
        answer(&mut p, &intent, &delivery, b"v=0\r\n", |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        complete(&p, &open, &delivery, |_, _| Err(VoiceError::Capacity)),
        Err(VoiceError::Capacity)
    );
    assert!(!delivery.was_applied());
    assert_eq!(
        generic(&mut p, &intent, &delivery).err(),
        Some(VoiceError::Unauthorized)
    );
    complete(&p, &open, &delivery, |_, _| Ok(())).unwrap();
    assert!(delivery.was_applied());
    // Lost old admission and completion replies may arrive after Applied. Neither retry
    // may reset the original marker and leave HIGH stuck without a first Answer.
    deliver(&p, &open, &delivery, |_, _| Ok(())).unwrap();
    complete(&p, &open, &delivery, |_, _| Ok(())).unwrap();
    assert!(delivery.was_applied());
    assert_eq!(
        generic(&mut p, &intent, &delivery).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        p.f.high.join.ledger.initial_offer_state_for_test(p.f.id),
        before
    );
    let answer = answer(&mut p, &intent, &delivery, b"v=0\r\n", |_, _| Ok(())).unwrap();
    let input = answer.input().unwrap();
    let CryptoInput::SealSdp(flight) = input.as_input() else {
        panic!("actual HIGH Seal")
    };
    assert_eq!(flight.reservation.sequence, before.0 + 1);
    assert_eq!(
        p.f.high.join.ledger.initial_offer_state_for_test(p.f.id),
        (before.0 + 1, Some(flight.reservation.id))
    );
    answer.settle_engine();
    phase.close();
}

#[test]
fn high_answer_final_capacity_preserves_original_claims_then_same_maximum_action_survives_compaction(
) {
    let (mut p, phase, intent, open, delivery) = opened();
    deliver(&p, &open, &delivery, |_, _| Ok(())).unwrap();
    complete(&p, &open, &delivery, |_, _| Ok(())).unwrap();
    let resources = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    let crypto_fill: Vec<_> = (0..16)
        .map_while(|_| {
            resources
                .charge(group, 1, crate::voice_v2::resources::ResourceKind::Crypto)
                .ok()
        })
        .collect();
    assert!(resources
        .charge(group, 1, crate::voice_v2::resources::ResourceKind::Crypto)
        .is_err());
    let packet_fill =
        p.f.high
            .join
            .ledger
            .packet_slots()
            .fill_unreserved_for_test();
    let action_slots = p.f.high.join.ledger.action_slots();
    let action_fill: Vec<_> = (0..32).map_while(|_| action_slots.charge(1).ok()).collect();
    assert!(action_slots.charge(1).is_err());
    let ordinary = resources
        .charge(
            group,
            crate::voice_v2::resources::GROUP_BYTES - resources.group_counts(group).unwrap().0,
            crate::voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    let counts = resources.counts();
    let before = p.f.high.join.ledger.initial_offer_state_for_test(p.f.id);
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    for _ in 0..3 {
        assert_eq!(
            answer(&mut p, &intent, &delivery, &bytes, |_, _| Err(
                VoiceError::Capacity
            ))
            .err(),
            Some(VoiceError::Capacity)
        );
        assert_eq!(
            p.f.high.join.ledger.initial_offer_state_for_test(p.f.id),
            before
        );
        assert_eq!(resources.counts(), counts);
        assert!(!p.f.high.join.has_initial_sdp_seal(p.f.id));
    }
    let first = answer(&mut p, &intent, &delivery, &bytes, |_, _| Ok(())).unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &answer(&mut p, &intent, &delivery, &bytes, |_, _| panic!(
            "one publication"
        ))
        .unwrap()
    ));
    first.admit_engine(first.engine_bytes()).unwrap(); // original prepaid Crypto lane still works
    first.settle_engine(); // fixture engine is finished; no fabricated terminal result
    assert!(first.input().is_err());
    assert!(Arc::ptr_eq(
        &first,
        &answer(&mut p, &intent, &delivery, &bytes, |_, _| panic!(
            "historical exact action"
        ))
        .unwrap()
    ));
    assert_eq!(
        answer(&mut p, &intent, &delivery, b"changed", |_, _| Ok(())).err(),
        Some(VoiceError::RetryMismatch)
    );
    drop((ordinary, crypto_fill, packet_fill, action_fill));
    phase.close();
}

#[test]
fn high_completion_and_answer_final_pc_close_leave_no_fresh_action_or_counter() {
    for completion_close in [false, true] {
        let (mut p, phase, intent, open, delivery) = opened();
        deliver(&p, &open, &delivery, |_, _| Ok(())).unwrap();
        let before = p.f.high.join.ledger.initial_offer_state_for_test(p.f.id);
        if completion_close {
            assert_eq!(
                complete(&p, &open, &delivery, |_, _| {
                    phase.request_close();
                    Ok(())
                }),
                Err(VoiceError::Unauthorized)
            );
            assert!(!delivery.was_applied());
        } else {
            complete(&p, &open, &delivery, |_, _| Ok(())).unwrap();
            assert_eq!(
                answer(&mut p, &intent, &delivery, b"v=0\r\n", |_, _| {
                    phase.request_close();
                    Ok(())
                })
                .err(),
                Some(VoiceError::Unauthorized)
            );
            assert!(
                delivery.was_applied(),
                "close retains immutable completion history"
            );
        }
        assert!(!p.f.high.join.has_initial_sdp_seal(p.f.id));
        assert_eq!(
            p.f.high.join.ledger.initial_offer_state_for_test(p.f.id),
            before
        );
        assert_eq!(
            answer(&mut p, &intent, &delivery, b"v=0\r\n", |_, _| Ok(())).err(),
            Some(VoiceError::Unauthorized)
        );
        phase.close();
    }
}

#[test]
fn high_answer_and_completion_reject_foreign_original_allocations_with_equal_offer_labels() {
    let (mut p, phase, intent, open, delivery) = opened();
    let (other, other_phase, other_intent, other_open, other_delivery) = opened();
    assert_eq!(
        delivery.offer(),
        other_delivery.offer(),
        "deterministic equal metadata is deliberately insufficient"
    );
    deliver(&p, &open, &delivery, |_, _| Ok(())).unwrap();
    assert_eq!(
        complete(&p, &open, &other_delivery, |_, _| Ok(())),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        complete(&p, &other_open, &delivery, |_, _| Ok(())),
        Err(VoiceError::Unauthorized)
    );
    deliver(&other, &other_open, &other_delivery, |_, _| Ok(())).unwrap();
    complete(&other, &other_open, &other_delivery, |_, _| Ok(())).unwrap();
    complete(&p, &open, &delivery, |_, _| Ok(())).unwrap();
    assert_eq!(
        answer(&mut p, &intent, &other_delivery, b"v=0\r\n", |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        answer(&mut p, &other_intent, &delivery, b"v=0\r\n", |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(!p.f.high.join.has_initial_sdp_seal(p.f.id));
    phase.close();
    other_phase.close();
}
