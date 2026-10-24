//! Real native Finish/signatures and actual Runtime verification. Initial confirmation
//! AEAD remains mocked by the parent fixture; these are not RTC/physical audio tests.
use super::*;

fn install(
    p: &mut ProtectedFixture,
    helpers: &[DeviceId],
    signer: &dyn VoiceSigner,
    rng: &mut Rng,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<(), VoiceError> {
    p.f.high.join.install_high_ready_receive(
        &p.high,
        &room(&p.f.high),
        p.f.id,
        helpers,
        signer,
        &ca(&p.f.high.context, &p.f.high.lease),
        &p.f.clock,
        rng,
        final_owner,
    )
}
fn offer(p: &ProtectedFixture, edit: impl FnOnce(&mut EncryptedHeader)) -> SignedObject {
    let message = [181; 32];
    let mut header = EncryptedHeader {
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
        sequence: 4,
        offer: Some(OfferId {
            issuer: p.f.low.context.local,
            message,
        }),
    };
    edit(&mut header);
    SignedObject::issue(
        p.f.low.context.scope,
        &p.f.low.signer,
        message,
        1000,
        16000,
        Body::Encrypted {
            header,
            ciphertext: vec![4; MAX_SDP_PLAINTEXT + 16],
        },
    )
    .unwrap()
}

#[test]
fn ready_receive_requires_actual_inbox_and_final_marker_before_restricted_verification() {
    let mut p = confirmed_high();
    let signer = Signer::original(&p);
    let mut rng = Rng::new();
    prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
    let owner = pending(&p);
    let object = offer(&p, |_| {});
    let frame = Envelope::Direct(object).encode();
    assert!(!owner.lends_ready_offer_verification(p.f.low.context.local, &frame, 1000, 1000));
    assert_eq!(
        install(&mut p, &[], &signer, &mut rng, |_, _| Err(
            VoiceError::Capacity
        )),
        Err(VoiceError::Capacity)
    );
    let original = cache(&p).receive_for_test();
    assert!(!owner.lends_ready_offer_verification(p.f.low.context.local, &frame, 1000, 1000));
    install(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
    assert!(Arc::ptr_eq(&original, &cache(&p).receive_for_test()));
    assert!(owner.lends_ready_offer_verification(p.f.low.context.local, &frame, 1000, 1000));
    owner.assert_inactive_round_lenders_for_test(p.f.high.join.ledger.packet_slots());
    let ticket = p
        .high
        .admit_transport_verification(p.f.low.context.local, &frame)
        .unwrap();
    assert_eq!(rng.fills, 1);
    assert_eq!(signer.calls.get(), 1);
    assert!(!owner.is_live());
    assert!(!owner.lends_verification(p.f.low.context.local, &frame));
    assert!(owner.charge(Stage::Crypto, 1).is_err());
    assert!(p.f.high.join.published_source_packets().next().is_none());
    assert!(!p.f.high.join.ledger.has_sdp_for_test());
    drop(ticket);
}

#[test]
fn ready_receive_uses_original_slot_and_bytes_under_full_ordinary_pressure_and_restores_unstarted_claim(
) {
    let mut p = confirmed_high();
    let signer = Signer::original(&p);
    let mut rng = Rng::new();
    prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
    let pool = p._runtimes[1].resources();
    let ordinary_crypto: Vec<_> = (0..3)
        .map(|_| {
            pool.charge(p.f.high.context.scope.group, 0, ResourceKind::Crypto)
                .unwrap()
        })
        .collect();
    assert!(pool
        .charge(p.f.high.context.scope.group, 0, ResourceKind::Crypto)
        .is_err());
    let ordinary = pool
        .charge(
            p.f.high.context.scope.group,
            GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let ordinary_slots =
        p.f.high
            .join
            .ledger
            .packet_slots()
            .fill_unreserved_for_test();
    let before = pool.counts();
    install(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
    assert_eq!(
        pool.counts(),
        before,
        "original reserve covers actual receive bytes"
    );
    let object = offer(&p, |_| {});
    let frame = Envelope::Direct(object.clone()).encode();
    let ticket = p
        .high
        .admit_transport_verification(p.f.low.context.local, &frame)
        .unwrap();
    let receive = cache(&p).receive_for_test();
    let mut prepared = receive.prepare_offer(&object, 1000, 1000).unwrap();
    assert_eq!(prepared.object(), &object);
    assert_eq!(
        receive.prepare_offer(&object, 1000, 1000).err(),
        Some(VoiceError::Capacity)
    );
    prepared.discard_before_restore_for_test(|| {
        // Re-enter the actual factory after old canonical buffers have been destroyed but
        // before their position is visible again. No second paid payload can yet exist.
        assert_eq!(
            receive.prepare_offer(&object, 1000, 1000).err(),
            Some(VoiceError::Capacity)
        );
    });
    drop(prepared);
    assert!(
        p.f.high
            .join
            .ledger
            .packet_slots()
            .fill_unreserved_for_test()
            .0
            .is_empty(),
        "abandonment cannot refund to ordinary table"
    );
    let prepared = receive.prepare_offer(&object, 1000, 1000).unwrap();
    prepared.check_at(1000, 1000).unwrap();
    let retained = prepared.retain();
    assert_eq!(retained.object(), &object);
    assert_eq!(retained.aad(), object.aad().unwrap());
    assert_eq!(retained.hash(), object.hash());
    drop(retained);
    assert_eq!(
        receive.prepare_offer(&object, 1000, 1000).err(),
        Some(VoiceError::Capacity),
        "published claim never rearms"
    );
    assert_eq!(
        pool.counts(),
        before,
        "verification borrows the original lane too"
    );
    drop((ticket, receive, ordinary, ordinary_slots, ordinary_crypto));
}

#[test]
fn ready_receive_rejects_wrong_binding_helpers_and_bad_signature_does_not_admit_a_packet() {
    let mut helpers = [
        DeviceKeypair::from_seed(&[88; 32]).device_id(),
        DeviceKeypair::from_seed(&[89; 32]).device_id(),
    ];
    helpers.sort();
    let mut p = confirmed_high_in(fixture_with_members(&helpers));
    let signer = Signer::original(&p);
    let mut rng = Rng::new();
    install(&mut p, &helpers[..1], &signer, &mut rng, |_, _| Ok(())).unwrap();
    let owner = pending(&p);
    let original = offer(&p, |_| {});
    for change in 0..6 {
        let object = offer(&p, |header| match change {
            0 => header.target_join = [98; 32],
            1 => header.source_join = [98; 32],
            2 => header.source_key_ref = [98; 32],
            3 => header.target_key_ref = [98; 32],
            4 => header.handshake = [98; 32],
            _ => {
                header.kind = EncryptedKind::Answer;
                header.offer = Some(OfferId {
                    issuer: header.target,
                    message: [182; 32],
                });
            }
        });
        assert!(!owner.lends_ready_offer_verification(
            p.f.low.context.local,
            &Envelope::Direct(object.clone()).encode(),
            1000,
            1000
        ));
        assert!(cache(&p)
            .receive_for_test()
            .prepare_offer(&object, 1000, 1000)
            .is_err());
    }
    let permit = RoutePermit::issue(&p.f.low.signer, &original, helpers[0], 1000, 16000).unwrap();
    let relay = Envelope::HelperDelivery {
        object: original.clone(),
        permit,
    }
    .encode();
    assert!(owner.lends_ready_offer_verification(helpers[0], &relay, 1000, 1000));
    assert!(!owner.lends_ready_offer_verification(helpers[1], &relay, 1000, 1000));
    assert!(!owner.lends_ready_offer_verification(
        helpers[0],
        &Envelope::Direct(original.clone()).encode(),
        1000,
        1000
    ));
    let mut invalid = Envelope::Direct(original.clone()).encode();
    *invalid.last_mut().unwrap() ^= 1;
    assert!(
        owner.lends_ready_offer_verification(p.f.low.context.local, &invalid, 1000, 1000),
        "peek selects capacity, never signature authority"
    );
    let before = p._runtimes[1].resources().counts();
    let ticket = p
        .high
        .admit_transport_verification(p.f.low.context.local, &invalid)
        .unwrap();
    assert_eq!(
        Envelope::decode(&invalid).err(),
        Some(VoiceError::Signature)
    );
    drop(ticket);
    assert_eq!(p._runtimes[1].resources().counts(), before);
    assert!(cache(&p)
        .receive_for_test()
        .prepare_offer(&original, 1000, 1000)
        .is_ok());
    assert!(!owner.is_live());
}

#[test]
fn ready_receive_expiry_close_epoch_and_final_races_never_activate_or_rearm() {
    for fault in 0..4 {
        let mut p = confirmed_high();
        let signer = Signer::original(&p);
        let mut rng = Rng::new();
        install(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
        let receive = cache(&p).receive_for_test();
        let object = offer(&p, |_| {});
        let prepared = receive.prepare_offer(&object, 1000, 1000).unwrap();
        let owner = pending(&p);
        match fault {
            0 => {
                p.f.clock.advance_ms(15000);
                p.f.clock.set_wall_ms(1);
            }
            1 => p._runtimes[1].revoke_session(),
            2 => {
                p.high
                    .sync_view(View {
                        epoch: p.f.high.context.epoch + 1,
                        members: [p.f.low.context.local, p.f.high.context.local]
                            .into_iter()
                            .collect(),
                        policy: p.f.high.context.scope.policy,
                        eligible: true,
                        indirect: true,
                    })
                    .unwrap();
            }
            _ => owner.close(),
        }
        assert!(prepared
            .check_at(p.f.clock.now_ms(), p.f.clock.monotonic_ms())
            .is_err());
        assert!(!owner.lends_ready_offer_verification(
            p.f.low.context.local,
            &Envelope::Direct(object.clone()).encode(),
            p.f.clock.now_ms(),
            p.f.clock.monotonic_ms()
        ));
        drop(prepared);
        assert!(receive
            .prepare_offer(&object, p.f.clock.now_ms(), p.f.clock.monotonic_ms())
            .is_err());
        assert!(!owner.is_live());
    }
}

#[test]
fn retained_offer_keeps_actual_slot_and_original_whole_plan_custody_until_last_drop() {
    let mut p = confirmed_high();
    let control = p.f.high.join.confirmations.pairs[&p.f.id]
        .chain
        .as_ref()
        .unwrap()
        .workflow()
        .unwrap()
        .confirmation_control();
    let signer = Signer::original(&p);
    let mut rng = Rng::new();
    install(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
    let object = offer(&p, |_| {});
    let receive = cache(&p).receive_for_test();
    let retained = receive.prepare_offer(&object, 1000, 1000).unwrap().retain();
    let owner = pending(&p);
    let whole = control.retire_all();
    owner.close();
    drop(receive);
    p.f.high
        .join
        .confirmations
        .pairs
        .get_mut(&p.f.id)
        .unwrap()
        .sdp_successor
        .take();
    drop(owner);
    assert!(!whole.is_drained());
    let slots =
        p.f.high
            .join
            .ledger
            .packet_slots()
            .fill_unreserved_for_test();
    assert_eq!(slots.0.len(), 31);
    drop(retained);
    assert!(whole.is_drained());
    assert_eq!(
        p.f.high
            .join
            .ledger
            .packet_slots()
            .fill_unreserved_for_test()
            .0
            .len(),
        1
    );
}

#[test]
fn ready_receive_final_close_wins_before_marker_and_unlock_cannot_revive_original_owner() {
    for close_before_marker in [false, true] {
        let mut p = confirmed_high();
        let signer = Signer::original(&p);
        let mut rng = Rng::new();
        let call = p.f.high.lease.clone();
        let result = install(&mut p, &[], &signer, &mut rng, |_, _| {
            if close_before_marker {
                call.revoke();
            }
            Ok(())
        });
        if close_before_marker {
            assert_eq!(result, Err(VoiceError::Unauthorized));
        } else {
            result.unwrap();
        }
        let owner = pending(&p);
        let receive = cache(&p).receive_for_test();
        let object = offer(&p, |_| {});
        assert_eq!(
            owner.ready_publication_current_at(1000, 1000),
            !close_before_marker
        );
        // The actual process replaces Session. Re-activating its new ticket preserves the
        // old receiver's original dead-session/actor/call bindings; it grants no adoption.
        p._runtimes[1].revoke_session();
        let fresh = p._runtimes[1].session_ticket();
        p._runtimes[1].activate_session(&fresh).unwrap();
        assert!(!owner.ready_publication_current_at(1000, 1000));
        assert!(receive.prepare_offer(&object, 1000, 1000).is_err());
        assert!(install(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).is_err());
        assert!(!owner.is_live());
    }
}
