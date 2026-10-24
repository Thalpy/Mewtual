//! Actual native first-Offer custody and recovery; only the preceding confirmation AEAD is
//! mocked by the parent fixture. No successful SDP decrypt/RTC or audible result is asserted.
use super::*;
use crate::voice_v2::helper::RoomAuthority;

fn install(p: &mut ProtectedFixture, room: &RoomAuthority) {
    let signer = Signer::original(p);
    let mut rng = Rng::new();
    p.f.high
        .join
        .install_high_ready_receive(
            &p.high,
            room,
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
pub(super) fn offer(p: &ProtectedFixture, message: u8, sequence: u64) -> SignedObject {
    let now = p.f.clock.now_ms();
    let message = [message; 32];
    SignedObject::issue(
        p.f.low.context.scope,
        &p.f.low.signer,
        message,
        now,
        now + 15000,
        Body::Encrypted {
            header: EncryptedHeader {
                source_join: p.f.low.join.key.join,
                target: p.f.high.context.local,
                target_join: p.f.high.join.key.join,
                source_key_ref: p.f.low.join.key.key_ref,
                target_key_ref: p.f.high.join.key.key_ref,
                handshake: p.f.id,
                low_to_high: true,
                kind: EncryptedKind::OfferRound(crate::voice_v2::SdpRound {
                    ready: cache(p).artifact_for_test().unwrap().0.id(),
                    profile: crate::voice_v2::SdpProfile::CompleteSdpV1,
                }),
                sequence,
                offer: Some(OfferId {
                    issuer: p.f.low.context.local,
                    message,
                }),
            },
            ciphertext: vec![42; MAX_SDP_PLAINTEXT + 16],
        },
    )
    .unwrap()
}
pub(super) fn admit(
    p: &mut ProtectedFixture,
    object: &SignedObject,
    room: &RoomAuthority,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<Arc<crate::voice_v2::negotiation::admission::SdpAdmission>, VoiceError> {
    let envelope = Envelope::Direct(object.clone());
    let ticket = p
        .high
        .admit_transport_verification(p.f.low.context.local, &envelope.encode())?;
    p.f.high.join.admit_protected_first_offer(
        &p.high,
        &ticket,
        &envelope,
        room,
        &ca(&p.f.high.context, &p.f.high.lease),
        p._runtimes[1].clock(),
        final_owner,
    )
}
#[test]
fn native_first_offer_lost_reply_recovers_same_token_before_and_after_consumption_without_another_event(
) {
    let mut p = confirmed_high();
    let room = room(&p.f.high);
    install(&mut p, &room);
    let object = offer(&p, 211, 4);
    let first = admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
    assert!(pending(&p).is_live());
    let original_input = p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_input
        .as_ref()
        .unwrap()
        .as_ref() as *const InboundPacket;
    let before = p._runtimes[1].resources().counts();
    for _ in 0..3 {
        let recovered = admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
        assert!(Arc::ptr_eq(&first, &recovered));
        assert_eq!(
            p.f.high.join.confirmations.pairs[&p.f.id]
                .sdp_input
                .as_ref()
                .unwrap()
                .as_ref() as *const InboundPacket,
            original_input
        );
        assert_eq!(p._runtimes[1].resources().counts(), before);
    }
    let input =
        p.f.high
            .join
            .confirmations
            .pairs
            .get_mut(&p.f.id)
            .unwrap()
            .sdp_input
            .take()
            .unwrap();
    let committed =
        p.f.high
            .join
            .ledger
            .commit_inbound(
                input.token,
                input.aad(),
                input.header.sequence,
                input.ciphertext(),
                &ca(&p.f.high.context, &p.f.high.lease),
                &p.f.clock,
            )
            .unwrap();
    drop((input, committed));
    let recovered = admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
    assert!(Arc::ptr_eq(&first, &recovered));
    assert_eq!(first.token, recovered.token);
    assert!(
        p.f.high.join.confirmations.pairs[&p.f.id]
            .sdp_input
            .is_none(),
        "cached history cannot recreate an input"
    );
}
#[test]
fn native_first_offer_final_refusal_and_replay_leave_original_receive_claim_unspent() {
    let mut p = confirmed_high();
    let room = room(&p.f.high);
    install(&mut p, &room);
    let object = offer(&p, 212, 4);
    let owner = pending(&p);
    for _ in 0..3 {
        assert_eq!(
            admit(&mut p, &object, &room, |_, _| Err(VoiceError::Capacity)).err(),
            Some(VoiceError::Capacity)
        );
        assert!(!owner.is_live());
        assert!(!p.f.high.join.ledger.has_sdp_for_test());
        assert!(p.f.high.join.confirmations.pairs[&p.f.id]
            .first_offer
            .is_none());
        assert!(p.f.high.join.confirmations.pairs[&p.f.id]
            .sdp_input
            .is_none());
    }
    let replay = offer(&p, 213, 1);
    assert_eq!(
        admit(&mut p, &replay, &room, |_, _| Ok(())).err(),
        Some(VoiceError::Replay)
    );
    let first = admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
    let changed = offer(&p, 212, 5);
    assert_eq!(
        admit(&mut p, &changed, &room, |_, _| Ok(())).err(),
        Some(VoiceError::RetryMismatch)
    );
    assert!(Arc::ptr_eq(
        &first,
        &admit(&mut p, &object, &room, |_, _| Ok(())).unwrap()
    ));
}
#[test]
fn native_first_offer_full_ordinary_memory_tables_and_crypto_use_same_original_capacity() {
    let mut p = confirmed_high();
    let room = room(&p.f.high);
    install(&mut p, &room);
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    let ordinary_crypto: Vec<_> = (0..3)
        .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    let ordinary = pool
        .charge(group, GROUP_BYTES - pool.counts().0, ResourceKind::Retained)
        .unwrap();
    let tables =
        p.f.high
            .join
            .ledger
            .packet_slots()
            .fill_unreserved_for_test();
    let full = pool.counts();
    let object = offer(&p, 214, 4);
    let first = admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
    assert!(first.authorized());
    assert_eq!(pool.counts(), full);
    drop((ordinary_crypto, ordinary, tables));
}
#[test]
fn native_first_offer_late_deadline_and_quiet_cleanup_leave_only_fixed_history_after_actual_last_input_drop(
) {
    let mut p = confirmed_high();
    p.f.clock.advance_ms(29000);
    let room = room(&p.f.high);
    install(&mut p, &room);
    let object = offer(&p, 215, 4);
    let first = admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
    assert_eq!(
        first.deadline(),
        31000,
        "late Offer cannot extend original browser key custody"
    );
    let actual_input =
        p.f.high
            .join
            .confirmations
            .pairs
            .get_mut(&p.f.id)
            .unwrap()
            .sdp_input
            .take()
            .unwrap();
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    p.f.clock.advance_ms(1000);
    p.f.high.join.collect_expired(&p.f.clock);
    assert!(!first.authorized());
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_successor
        .is_none());
    assert!(!p.f.high.join.ledger.has_sdp_for_test());
    assert!(
        pool.exchange(group).is_some(),
        "actual input still owns original live capacity"
    );
    drop(actual_input);
    assert!(
        pool.exchange(group).is_none(),
        "fixed history cannot pin Exchange/lane/registry"
    );
    assert_eq!(
        p.f.high.join.confirmations.pairs[&p.f.id]
            .first_offer
            .as_ref()
            .unwrap()
            .token,
        first.token
    );
    let next =
        Exchange::new_for_test(pool.clone(), group, p.f.high.join.ledger.action_slots()).unwrap();
    pool.register_exchange(group, &next).unwrap();
    assert!(Arc::ptr_eq(&pool.exchange(group).unwrap(), &next));
}

#[test]
fn native_first_offer_final_native_clock_and_close_unlock_never_publish_or_revive_old_input() {
    use std::sync::atomic::Ordering;
    let mut baseline = confirmed_high();
    let baseline_room = room(&baseline.f.high);
    install(&mut baseline, &baseline_room);
    let before = baseline.origin_clock.reads.load(Ordering::Acquire);
    let object = offer(&baseline, 216, 4);
    admit(&mut baseline, &object, &baseline_room, |_, _| Ok(())).unwrap();
    let reads = baseline.origin_clock.reads.load(Ordering::Acquire) - before;
    assert!(reads > 0, "actual Runtime native clock was traversed");
    for at in 1..=reads {
        let mut p = confirmed_high();
        let room = room(&p.f.high);
        install(&mut p, &room);
        let object = offer(&p, 216, 4);
        let owner = pending(&p);
        let before = p.origin_clock.reads.load(Ordering::Acquire);
        *p.origin_clock.revoke.lock().unwrap() = Some((before + at, p.f.high.lease.clone()));
        assert!(
            admit(&mut p, &object, &room, |_, _| Ok(())).is_err(),
            "clock boundary {at}"
        );
        assert!(!owner.is_live());
        assert!(!p.f.high.join.ledger.has_sdp_for_test());
        assert!(p.f.high.join.confirmations.pairs[&p.f.id]
            .first_offer
            .is_none());
        assert!(p.f.high.join.confirmations.pairs[&p.f.id]
            .sdp_input
            .is_none());
    }
    for session_closes in [false, true] {
        let mut p = confirmed_high();
        let room = room(&p.f.high);
        install(&mut p, &room);
        let object = offer(&p, 217, 4);
        let owner = pending(&p);
        let runtime = p._runtimes[1].clone();
        let call = p.f.high.lease.clone();
        let closed = p.high.revocation_observer_for_test();
        let runtime_for_close = runtime.clone();
        let mut revoker = None;
        assert!(admit(&mut p, &object, &room, |_, _| {
            if session_closes {
                // Lock first closes its atomic gate, then waits for the Group/Policy
                // fence held here. Do not run the locking half reentrantly in this thread.
                revoker = Some(std::thread::spawn(move || {
                    runtime_for_close.revoke_session()
                }));
                futures::executor::block_on(closed.wait_revoked());
            } else {
                call.revoke();
            }
            Ok(())
        })
        .is_err());
        if let Some(revoker) = revoker {
            revoker.join().unwrap();
        }
        assert!(!owner.is_live());
        assert!(!p.f.high.join.ledger.has_sdp_for_test());
        assert!(p.f.high.join.confirmations.pairs[&p.f.id]
            .first_offer
            .is_none());
        let new_ticket = runtime.session_ticket();
        runtime.activate_session(&new_ticket).unwrap();
        assert!(admit(&mut p, &object, &room, |_, _| Ok(())).is_err());
    }
}

#[test]
fn native_first_offer_frozen_helpers_enforced_with_ordinary_verification_and_cached_recovery() {
    let mut helpers = [
        DeviceKeypair::from_seed(&[88; 32]).device_id(),
        DeviceKeypair::from_seed(&[89; 32]).device_id(),
    ];
    helpers.sort();
    for initial_helper in [false, true] {
        let mut p = confirmed_high_in(fixture_with_members(&helpers));
        let signer = Signer::original(&p);
        let room = room(&p.f.high);
        p.f.high
            .join
            .install_high_ready_receive(
                &p.high,
                &room,
                p.f.id,
                &helpers[..1],
                &signer,
                &ca(&p.f.high.context, &p.f.high.lease),
                &p.f.clock,
                &mut Rng::new(),
                |_, _| Ok(()),
            )
            .unwrap();
        let object = offer(&p, 218, 4);
        let owner = pending(&p);
        let wrong = Envelope::HelperDelivery {
            object: object.clone(),
            permit: RoutePermit::issue(&p.f.low.signer, &object, helpers[1], 1000, 16000).unwrap(),
        };
        assert!(!owner.lends_ready_offer_verification(helpers[1], &wrong.encode(), 1000, 1000));
        let through = |p: &mut ProtectedFixture, envelope: &Envelope, helper| {
            // Ordinary Crypto is available: a false protected-loan selector must not be
            // mistaken for refusal. Native final admission still checks frozen helpers.
            let ticket = p
                .high
                .admit_transport_verification(helper, &envelope.encode())
                .unwrap();
            p.f.high.join.admit_protected_first_offer(
                &p.high,
                &ticket,
                envelope,
                &room,
                &ca(&p.f.high.context, &p.f.high.lease),
                p._runtimes[1].clock(),
                |_, _| Ok(()),
            )
        };
        // The immediate requester's normal2048B origin row is retained even on refusal.
        // Prepay it without spending/refilling any allowance so the comparison below
        // isolates native token/event admission, not legitimate requester bookkeeping.
        p.high
            .with_limits(|limits| {
                limits.prepare_exchange_members(&helpers, &p._runtimes[1].resources(), 1000)
            })
            .unwrap();
        let before = p._runtimes[1].resources().counts();
        assert_eq!(
            through(&mut p, &wrong, helpers[1]).err(),
            Some(VoiceError::Unauthorized)
        );
        assert_eq!(p._runtimes[1].resources().counts(), before);
        assert!(!p.f.high.join.ledger.has_sdp_for_test());
        assert!(p.f.high.join.confirmations.pairs[&p.f.id]
            .first_offer
            .is_none());
        let selected = Envelope::HelperDelivery {
            object: object.clone(),
            permit: RoutePermit::issue(&p.f.low.signer, &object, helpers[0], 1000, 16000).unwrap(),
        };
        let first = if initial_helper {
            through(&mut p, &selected, helpers[0]).unwrap()
        } else {
            admit(&mut p, &object, &room, |_, _| Ok(())).unwrap()
        };
        assert_eq!(
            through(&mut p, &wrong, helpers[1]).err(),
            Some(VoiceError::Unauthorized)
        );
        assert!(Arc::ptr_eq(
            &first,
            &through(&mut p, &selected, helpers[0]).unwrap()
        ));
        assert!(Arc::ptr_eq(
            &first,
            &admit(&mut p, &object, &room, |_, _| Ok(())).unwrap()
        ));
    }
}

#[test]
fn native_first_offer_empty_ready_helper_selection_refuses_even_valid_member_permit() {
    let helper = DeviceKeypair::from_seed(&[88; 32]).device_id();
    let mut p = confirmed_high_in(fixture_with_members(&[helper]));
    let room = room(&p.f.high);
    install(&mut p, &room);
    let object = offer(&p, 219, 4);
    let envelope = Envelope::HelperDelivery {
        object: object.clone(),
        permit: RoutePermit::issue(&p.f.low.signer, &object, helper, 1000, 16000).unwrap(),
    };
    for before_direct in [true, false] {
        if !before_direct {
            admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
        }
        let ticket = p
            .high
            .admit_transport_verification(helper, &envelope.encode())
            .unwrap();
        assert_eq!(
            p.f.high
                .join
                .admit_protected_first_offer(
                    &p.high,
                    &ticket,
                    &envelope,
                    &room,
                    &ca(&p.f.high.context, &p.f.high.lease),
                    p._runtimes[1].clock(),
                    |_, _| Ok(())
                )
                .err(),
            Some(VoiceError::Unauthorized)
        );
    }
}

#[test]
fn native_first_offer_wrong_round_and_legacy_mode_with_ordinary_verification_spend_nothing() {
    let mut p = confirmed_high();
    let room = room(&p.f.high);
    install(&mut p, &room);
    let original = offer(&p, 221, 4);
    let Body::Encrypted { header, ciphertext } = original.body() else {
        panic!("Offer")
    };
    let mut wrong = header.kind.round().unwrap();
    wrong.ready[0] ^= 1;
    let before = p.f.pool.counts();
    for kind in [
        EncryptedKind::Offer,
        EncryptedKind::OfferRound(wrong),
        EncryptedKind::Ice,
    ] {
        let mut changed = header.clone();
        changed.kind = kind;
        let object = SignedObject::issue(
            original.scope(),
            &p.f.low.signer,
            original.id(),
            original.issued(),
            original.expires(),
            Body::Encrypted {
                header: changed,
                ciphertext: ciphertext
                    .iter()
                    .copied()
                    .take(kind.max_plaintext() + 16)
                    .collect(),
            },
        )
        .unwrap();
        let frame = Envelope::Direct(object.clone()).encode();
        assert!(!pending(&p).lends_ready_offer_verification(
            p.f.low.context.local,
            &frame,
            1000,
            1000
        ));
        // This goes through the ordinary authenticated verifier. It cannot borrow Ready
        // capacity, nor consume its single inbox, replay bit, token or origin admission.
        assert_eq!(
            admit(&mut p, &object, &room, |_, _| panic!(
                "refusal must precede final origin"
            ))
            .err(),
            Some(VoiceError::Unauthorized)
        );
        assert_eq!(p.f.pool.counts(), before);
        assert!(!pending(&p).is_live());
        assert!(p.f.high.join.confirmations.pairs[&p.f.id]
            .first_offer
            .is_none());
    }
    // Same identity/sequence must still work, so none of the refused variants consumed it.
    let first = admit(&mut p, &original, &room, |_, _| Ok(())).unwrap();
    assert_eq!(Some(first.round), header.kind.round());
    assert!(Arc::ptr_eq(
        &first,
        &admit(&mut p, &original, &room, |_, _| Ok(())).unwrap()
    ));
}
