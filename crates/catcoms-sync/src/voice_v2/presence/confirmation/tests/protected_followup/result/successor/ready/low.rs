//! Both ends have actual native protected Finish history. Signatures, runtime loans,
//! request/reply owners and Sources are real; the preceding AEAD remains mocked.
use super::*;
use crate::voice_v2::ready::admission::ReadyAdmission;
use futures::FutureExt;

fn install(p: &mut ProtectedFixture, helpers: &[DeviceId]) -> SignedObject {
    let signer = Signer::original(p);
    p.f.high
        .join
        .install_high_ready_receive(
            &p.high,
            &room(&p.f.high),
            p.f.id,
            helpers,
            &signer,
            &ca(&p.f.high.context, &p.f.high.lease),
            &p.f.clock,
            &mut Rng::new(),
            |_, _| Ok(()),
        )
        .unwrap();
    cache(p).artifact_for_test().unwrap().0.clone()
}
fn admit_envelope(
    p: &mut ProtectedFixture,
    envelope: &Envelope,
    requester: DeviceId,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<Arc<ReadyAdmission>, VoiceError> {
    let frame = envelope.encode();
    let ticket = p.low.admit_transport_verification(requester, &frame)?;
    // Exercise the actual authenticated decoder too; a peek can never certify signatures.
    let envelope = Envelope::decode(&frame)?;
    p.f.low.join.admit_protected_ready(
        &p.low,
        &ticket,
        &envelope,
        &room(&p.f.low),
        &ca(&p.f.low.context, &p.f.low.lease),
        final_owner,
    )
}
fn admit_ready(
    p: &mut ProtectedFixture,
    object: &SignedObject,
) -> Result<Arc<ReadyAdmission>, VoiceError> {
    let requester = p.f.high.context.local;
    admit_envelope(p, &Envelope::Direct(object.clone()), requester, |_, _| {
        Ok(())
    })
}
fn advance(p: &mut ProtectedFixture) -> Result<SdpSuccessor, VoiceError> {
    p.f.low.join.prepare_low_sdp_successor(
        &p.low,
        &room(&p.f.low),
        p.f.id,
        &ca(&p.f.low.context, &p.f.low.lease),
        |_, _| Ok(()),
    )
}
fn prepay(p: &mut ProtectedFixture) {
    p.f.low
        .join
        .prepay_low_sdp_successor(&p.low, p.f.id, &ca(&p.f.low.context, &p.f.low.lease))
        .unwrap();
}

/// Capture the actual LOW key observer while its Finish action is still pending, then
/// complete the real HIGH open. No copied identifier can manufacture this fence owner.
fn confirmed_with_low_keys() -> (
    ProtectedFixture,
    crate::voice_v2::crypto_actions::EstablishedKeyCustody,
) {
    let (mut p, finish) = super::super::super::transition::ready_to_finish();
    let control = finish.confirmation_control().unwrap();
    let plan = finish.protected_plan().unwrap();
    let (object, plain) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let mono = p.f.clock.monotonic_ms();
    let finite = plan.confirmation_custody(&control, mono).unwrap();
    let keys = plan.established_key_custody(&finite, mono).unwrap();
    let (_, action) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    opened(
        &mut p.f.high,
        &p.high,
        &action,
        &plain,
        &p.f.clock,
        &mut p.f.rng,
    );
    (p, keys)
}

#[test]
fn low_key_release_fences_ready_before_admission_at_final_publication_and_after_acceptance() {
    for ordering in 0..3 {
        let (mut p, keys) = confirmed_with_low_keys();
        let object = install(&mut p, &[]);
        prepay(&mut p);
        match ordering {
            0 => {
                keys.close_successor_effects();
                assert_eq!(
                    admit_ready(&mut p, &object).err(),
                    Some(VoiceError::Unauthorized)
                );
            }
            1 => {
                let requester = p.f.high.context.local;
                assert_eq!(
                    admit_envelope(
                        &mut p,
                        &Envelope::Direct(object.clone()),
                        requester,
                        |_, _| {
                            keys.close_successor_effects();
                            Ok(())
                        },
                    )
                    .err(),
                    Some(VoiceError::Unauthorized)
                );
                assert!(
                    p.f.low.join.confirmations.pairs[&p.f.id]
                        .ready_admission
                        .is_none(),
                    "a winning final release cannot publish causal Ready"
                );
            }
            _ => {
                let accepted = admit_ready(&mut p, &object).unwrap();
                keys.close_successor_effects();
                assert!(!accepted.authorized_at(p.f.clock.now_ms(), p.f.clock.monotonic_ms()));
            }
        }
        assert_eq!(
            advance(&mut p).err(),
            Some(VoiceError::Unauthorized),
            "none of the orderings may extract/activate a released key's successor"
        );
        assert!(!keys.is_current());
    }
}
fn changed(
    p: &ProtectedFixture,
    object: &SignedObject,
    edit: impl FnOnce(&mut ReadyHeader),
) -> SignedObject {
    let Body::PairReadyRound {
        header,
        sequence,
        profile,
    } = object.body()
    else {
        panic!("Ready")
    };
    let mut header = header.clone();
    edit(&mut header);
    SignedObject::issue(
        object.scope(),
        &p.f.high.signer,
        object.id(),
        object.issued(),
        object.expires(),
        Body::PairReadyRound {
            sequence: *sequence,
            header,
            profile: *profile,
        },
    )
    .unwrap()
}

#[test]
fn low_ready_actual_ticket_and_actual_reply_work_each_delay_original_lane_transfer() {
    let mut p = confirmed_high();
    let object = install(&mut p, &[]);
    prepay(&mut p);
    let envelope = Envelope::Direct(object.clone());
    let ticket = p
        .low
        .admit_transport_verification(p.f.high.context.local, &envelope.encode())
        .unwrap();
    let reply_work = ticket.actual_work();
    let receipt =
        p.f.low
            .join
            .admit_protected_ready(
                &p.low,
                &ticket,
                &envelope,
                &room(&p.f.low),
                &ca(&p.f.low.context, &p.f.low.lease),
                |_, _| Ok(()),
            )
            .unwrap();
    assert_eq!(advance(&mut p).err(), Some(VoiceError::Capacity));
    assert!(matches!(
        p.f.low.join.confirmations.pairs[&p.f.id].sdp_successor,
        Some(Preparation::Reserved { .. })
    ));
    drop(ticket);
    assert_eq!(advance(&mut p).err(), Some(VoiceError::Capacity));
    drop(reply_work);
    let successor = advance(&mut p).unwrap();
    assert!(successor.confirmation_drained());
    successor
        .owner
        .assert_inactive_round_lenders_for_test(p.f.low.join.ledger.packet_slots());
    successor
        .owner
        .assert_inactive_packet_lenders_for_test(p.f.low.join.ledger.packet_slots());
    assert!(
        !successor.owner.is_live(),
        "Ready admission does not activate generic SDP/RTC"
    );
    assert!(Arc::ptr_eq(
        &receipt,
        &admit_ready(&mut p, &object).unwrap()
    ));
}

#[test]
fn low_ready_lost_ack_after_transfer_recovers_same_receipt_with_full_ordinary_bytes_and_crypto() {
    let mut helpers = [
        DeviceKeypair::from_seed(&[88; 32]).device_id(),
        DeviceKeypair::from_seed(&[89; 32]).device_id(),
    ];
    helpers.sort();
    let mut p = confirmed_high_in(fixture_with_members(&helpers));
    let object = install(&mut p, &helpers);
    prepay(&mut p);
    p.low
        .with_limits(|limits| limits.prepare_exchange_members(&helpers, &p.f.pool, 1000))
        .unwrap();
    let first = admit_ready(&mut p, &object).unwrap();
    let successor = advance(&mut p).unwrap();
    let identities = successor.owner.capacity_identities_for_test();
    let group = p.f.low.context.scope.group;
    let crypto: Vec<_> = (0..3)
        .map(|_| p.f.pool.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    let pressure =
        p.f.pool
            .charge(
                group,
                GROUP_BYTES - p.f.pool.counts().0,
                ResourceKind::Retained,
            )
            .unwrap();
    let full = p.f.pool.counts();
    assert!(Arc::ptr_eq(&first, &admit_ready(&mut p, &object).unwrap()));
    for helper in helpers {
        let envelope = Envelope::HelperDelivery {
            object: object.clone(),
            permit: RoutePermit::issue(&p.f.high.signer, &object, helper, 1000, 16000).unwrap(),
        };
        assert!(Arc::ptr_eq(
            &first,
            &admit_envelope(&mut p, &envelope, helper, |_, _| Ok(())).unwrap()
        ));
    }
    assert_eq!(p.f.pool.counts(), full);
    assert_eq!(successor.owner.capacity_identities_for_test(), identities);
    assert!(!successor.owner.is_live());
    drop((crypto, pressure));
}

#[test]
fn low_ready_wrong_original_finish_epoch_join_key_and_replacement_ids_do_not_commit() {
    let edits: [fn(&mut ReadyHeader); 7] = [
        |h| h.finish_message[0] ^= 1,
        |h| h.finish_object_hash[0] ^= 1,
        |h| h.epoch += 1,
        |h| h.source_join[0] ^= 1,
        |h| h.target_join[0] ^= 1,
        |h| h.source_key_ref[0] ^= 1,
        |h| h.target_key_ref[0] ^= 1,
    ];
    let mut p = confirmed_high();
    let object = install(&mut p, &[]);
    for edit in edits {
        let wrong = changed(&p, &object, edit);
        assert!(admit_ready(&mut p, &wrong).is_err());
        assert!(p.f.low.join.confirmations.pairs[&p.f.id]
            .ready_admission
            .is_none());
        assert!(!p.f.low.join.confirmations.pairs[&p.f.id]
            .chain
            .as_ref()
            .unwrap()
            .low_ready_accepted());
    }
    let first = admit_ready(&mut p, &object).unwrap();
    let replacement = SignedObject::issue(
        object.scope(),
        &p.f.high.signer,
        [211; 32],
        object.issued(),
        object.expires(),
        object.body().clone(),
    )
    .unwrap();
    assert_eq!(
        admit_ready(&mut p, &replacement).err(),
        Some(VoiceError::RetryMismatch)
    );
    assert!(Arc::ptr_eq(&first, &admit_ready(&mut p, &object).unwrap()));
}

#[test]
fn low_ready_final_refusal_never_stops_finish_or_keeps_an_unpublished_fee() {
    let mut p = confirmed_high();
    let object = install(&mut p, &[]);
    let before = p.f.pool.counts();
    let high = p.f.high.context.local;
    assert_eq!(
        admit_envelope(&mut p, &Envelope::Direct(object.clone()), high, |_, _| Err(
            VoiceError::Capacity
        ))
        .err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(p.f.pool.counts(), before);
    let pair = &p.f.low.join.confirmations.pairs[&p.f.id];
    assert!(pair.ready_admission.is_none());
    assert!(!pair.chain.as_ref().unwrap().low_ready_accepted());
    assert!(p.f.low.join.published_source_packets().any(|object| matches!(object.body(), Body::Encrypted { header, .. } if header.kind == EncryptedKind::ConfirmFinish)));
    admit_ready(&mut p, &object).unwrap();
    assert!(p.f.low.join.published_source_packets().next().is_none());
}

#[test]
fn low_ready_final_native_clock_close_and_lock_unlock_never_revive_old_acceptance() {
    use std::sync::atomic::Ordering;
    let mut baseline = confirmed_high();
    let object = install(&mut baseline, &[]);
    let before = baseline.origin_clock.reads.load(Ordering::Acquire);
    admit_ready(&mut baseline, &object).unwrap();
    let reads = baseline.origin_clock.reads.load(Ordering::Acquire) - before;
    for at in 1..=reads {
        let mut p = confirmed_high();
        let object = install(&mut p, &[]);
        let before = p.origin_clock.reads.load(Ordering::Acquire);
        *p.origin_clock.revoke.lock().unwrap() = Some((before + at, p.f.low.lease.clone()));
        assert!(
            admit_ready(&mut p, &object).is_err(),
            "native observation {at}"
        );
        assert!(p.f.low.join.confirmations.pairs[&p.f.id]
            .ready_admission
            .is_none());
        p._runtimes[0].revoke_session();
        let session = p._runtimes[0].session_ticket();
        p._runtimes[0].activate_session(&session).unwrap();
        assert!(admit_ready(&mut p, &object).is_err());
    }
}

#[test]
fn low_ready_strict_original_deadline_does_not_renew_on_retry_or_wall_rollback() {
    let mut p = confirmed_high();
    let object = install(&mut p, &[]);
    let first = admit_ready(&mut p, &object).unwrap();
    let until = first.deadline();
    p.f.clock.advance_ms(1000);
    assert!(Arc::ptr_eq(&first, &admit_ready(&mut p, &object).unwrap()));
    assert_eq!(first.deadline(), until);
    p.f.clock.advance_ms(until - p.f.clock.monotonic_ms());
    p.f.clock.set_wall_ms(1000);
    assert!(!first.authorized());
    assert!(admit_ready(&mut p, &object).is_err());
    assert!(advance(&mut p).is_err());
}

#[test]
fn low_ready_initial_admission_and_transfer_use_original_prepayment_with_ordinary_tables_full() {
    let mut p = confirmed_high();
    let object = install(&mut p, &[]);
    prepay(&mut p);
    let tables =
        p.f.low
            .join
            .ledger
            .packet_slots()
            .fill_unreserved_for_test();
    let group = p.f.low.context.scope.group;
    let crypto: Vec<_> = (0..3)
        .map(|_| p.f.pool.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    let pressure =
        p.f.pool
            .charge(
                group,
                GROUP_BYTES - p.f.pool.counts().0,
                ResourceKind::Retained,
            )
            .unwrap();
    assert_eq!(p.f.pool.counts().0, GROUP_BYTES);
    admit_ready(&mut p, &object).unwrap();
    let successor = advance(&mut p).unwrap();
    assert!(successor.confirmation_drained());
    assert!(!successor.owner.is_live());
    drop((tables, crypto, pressure));
}

#[test]
fn low_ready_unselected_helper_is_refused_even_with_ordinary_verification_before_and_after_transfer(
) {
    let mut helpers = [
        DeviceKeypair::from_seed(&[88; 32]).device_id(),
        DeviceKeypair::from_seed(&[89; 32]).device_id(),
    ];
    helpers.sort();
    let mut p = confirmed_high_in(fixture_with_members(&helpers));
    let object = install(&mut p, &helpers[..1]);
    let wrong = Envelope::HelperDelivery {
        object: object.clone(),
        permit: RoutePermit::issue(&p.f.high.signer, &object, helpers[1], 1000, 16000).unwrap(),
    };
    // Ordinary capacity is available. Failure of the protected selector alone cannot
    // establish rejection; actual route/native admission must still reject the helper.
    assert!(admit_envelope(&mut p, &wrong, helpers[1], |_, _| Ok(())).is_err());
    assert!(p.f.low.join.confirmations.pairs[&p.f.id]
        .ready_admission
        .is_none());
    let first = admit_ready(&mut p, &object).unwrap();
    let successor = advance(&mut p).unwrap();
    assert!(admit_envelope(&mut p, &wrong, helpers[1], |_, _| Ok(())).is_err());
    let changed = changed(&p, &object, |h| h.helpers = vec![helpers[1]]);
    assert_eq!(
        admit_ready(&mut p, &changed).err(),
        Some(VoiceError::RetryMismatch)
    );
    assert!(Arc::ptr_eq(&first, &admit_ready(&mut p, &object).unwrap()));
    assert_eq!(successor.owner.helpers(), &helpers[..1]);
}

#[test]
fn low_ready_new_legitimate_helper_row_waits_for_ordinary_capacity_then_retries_same_identity() {
    let helper = DeviceKeypair::from_seed(&[88; 32]).device_id();
    let mut p = confirmed_high_in(fixture_with_members(&[helper]));
    let object = install(&mut p, &[helper]);
    let stamp = cache(&p).stamp_for_test().unwrap();
    prepay(&mut p);
    let envelope = Envelope::HelperDelivery {
        object: object.clone(),
        permit: RoutePermit::issue(&p.f.high.signer, &object, helper, 1000, 16000).unwrap(),
    };
    let group = p.f.low.context.scope.group;
    let pressure =
        p.f.pool
            .charge(
                group,
                GROUP_BYTES - p.f.pool.counts().0,
                ResourceKind::Retained,
            )
            .unwrap();
    let before = p.f.pool.counts();
    assert_eq!(admit_envelope(&mut p,&envelope,helper,|_,_|Ok(())).err(),Some(VoiceError::Capacity),
        "the protected Crypto loan cannot bypass a previously unseen requester's ordinary record fee");
    assert_eq!(p.f.pool.counts(), before);
    assert!(p.f.low.join.confirmations.pairs[&p.f.id]
        .ready_admission
        .is_none());
    drop(pressure);
    let receipt = admit_envelope(&mut p, &envelope, helper, |_, _| Ok(())).unwrap();
    assert_eq!(receipt.hash, object.hash());
    assert_eq!(cache(&p).stamp_for_test().unwrap(), stamp);
    assert!(Arc::ptr_eq(
        &receipt,
        &admit_envelope(&mut p, &envelope, helper, |_, _| Ok(())).unwrap()
    ));
}

#[test]
fn low_ready_original_epoch_policy_session_and_room_revocation_refuse_history_and_transfer() {
    for change in ["epoch", "policy", "session", "room"] {
        let mut p = confirmed_high();
        let object = install(&mut p, &[]);
        let receipt = admit_ready(&mut p, &object).unwrap();
        match change {
            "epoch" | "policy" => p
                .low
                .sync_view(View {
                    epoch: p.f.low.context.epoch + u64::from(change == "epoch"),
                    members: [p.f.low.context.local, p.f.high.context.local].into(),
                    policy: if change == "policy" {
                        Some([233; 32])
                    } else {
                        p.f.low.context.scope.policy
                    },
                    eligible: true,
                    indirect: true,
                })
                .unwrap(),
            "session" => {
                p._runtimes[0].revoke_session();
                let ticket = p._runtimes[0].session_ticket();
                p._runtimes[0].activate_session(&ticket).unwrap();
            }
            "room" => p.f.low.lease.revoke(),
            _ => unreachable!(),
        }
        assert!(!receipt.authorized(), "{change}");
        assert!(admit_ready(&mut p, &object).is_err(), "{change}");
        assert!(advance(&mut p).is_err(), "{change}");
    }
}

#[test]
fn low_ready_fixed_history_does_not_pin_progress_registry_or_actual_lane_after_expiry() {
    let mut p = confirmed_high();
    let object = install(&mut p, &[]);
    let receipt = admit_ready(&mut p, &object).unwrap();
    let successor = advance(&mut p).unwrap();
    let pool = p.f.pool.clone();
    let group = p.f.low.context.scope.group;
    p.f.clock
        .advance_ms(receipt.deadline() - p.f.clock.monotonic_ms());
    p.f.low.join.collect_expired(&p.f.clock);
    assert!(p.f.low.join.confirmations.pairs[&p.f.id]
        .sdp_successor
        .is_none());
    assert!(!receipt.authorized());
    assert!(
        pool.exchange(group).is_some(),
        "exported original capacity is still real custody"
    );
    drop(successor);
    assert!(
        pool.exchange(group).is_none(),
        "fixed receipt cannot retain Exchange"
    );
    assert!(
        pool.workflow(group).is_none(),
        "fixed receipt cannot retain Workflow"
    );
    assert_eq!(pool.counts().1, 0);
    drop(p);
    assert!(
        pool.counts().0 > 0,
        "fixed receipt pays its retained gates and original history"
    );
    assert!(!receipt.authorized());
    drop(receipt);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn low_ready_late_transfer_refusal_retains_same_extracted_owner_without_activation_or_rebirth() {
    for end in [false, true] {
        let mut p = confirmed_high();
        let object = install(&mut p, &[]);
        let receipt = admit_ready(&mut p, &object).unwrap();
        let call = p.f.low.lease.clone();
        assert!(p
            .f
            .low
            .join
            .prepare_low_sdp_successor(
                &p.low,
                &room(&p.f.low),
                p.f.id,
                &ca(&p.f.low.context, &p.f.low.lease),
                |_, _| {
                    if end {
                        call.revoke();
                        Ok(())
                    } else {
                        Err(VoiceError::Capacity)
                    }
                },
            )
            .is_err());
        let Some(Preparation::Transferred { owner, .. }) =
            &p.f.low.join.confirmations.pairs[&p.f.id].sdp_successor
        else {
            panic!("late refusal retains actual extracted owner")
        };
        let owner = owner.clone();
        let identity = owner.capacity_identities_for_test();
        assert!(!owner.is_live());
        if end {
            assert!(!receipt.authorized());
            assert!(advance(&mut p).is_err());
            p.f.low.join.collect_expired(&p.f.clock);
            assert!(p.f.low.join.confirmations.pairs[&p.f.id]
                .sdp_successor
                .is_none());
            assert!(advance(&mut p).is_err());
        } else {
            let successor = advance(&mut p).unwrap();
            assert!(Arc::ptr_eq(&owner, &successor.owner));
            assert_eq!(owner.capacity_identities_for_test(), identity);
            assert!(!owner.is_live());
        }
    }
}

#[tokio::test]
async fn low_ready_stops_real_finish_source_without_tick_but_actual_lower_delays_drainage() {
    let mut p = confirmed_high();
    let object = p.f.low.join.published_source_packets().find(|object| matches!(object.body(), Body::Encrypted { header, .. } if header.kind == EncryptedKind::ConfirmFinish)).unwrap().clone();
    let activity =
        p.f.low
            .join
            .source_packet_activity(
                &object,
                &ca(&p.f.low.context, &p.f.low.lease),
                p._runtimes[0].clock(),
            )
            .unwrap();
    let source = p.low.source().unwrap();
    source
        .admit_borrowed(&p.low, &object, activity, room(&p.f.low))
        .unwrap();
    let routes = super::source::routes(&[p.f.high.context.local]);
    let job = source.take_ready(&p.low, &routes).unwrap().pop().unwrap();
    let authority = job.authority();
    let mut canceled = Box::pin(authority.revoked());
    assert!(futures::poll!(&mut canceled).is_pending());
    let ready = install(&mut p, &[]);
    prepay(&mut p);
    assert!(
        authority.authorized(),
        "local Finish/Established and remote artifact are insufficient"
    );
    admit_ready(&mut p, &ready).unwrap();
    assert!(
        canceled.now_or_never().is_some(),
        "actual native Ready wakes old Finish without time advance"
    );
    let successor = advance(&mut p).unwrap();
    assert!(
        !successor.confirmation_drained(),
        "actual Lower still owns original Workflow"
    );
    assert_eq!(source.counts(), (1, 1, 1));
    drop(job);
    assert_eq!(source.counts(), (0, 0, 0));
    assert!(
        !successor.confirmation_drained(),
        "external request authority still owns original packet"
    );
    drop(authority);
    assert!(successor.confirmation_drained());
}

#[test]
fn current_one_shot_ready_cannot_treat_a_higher_signed_sequence_as_replacement_authority() {
    let mut p = confirmed_high();
    let original = install(&mut p, &[]);
    let Body::PairReadyRound {
        header, profile, ..
    } = original.body()
    else {
        panic!("Ready")
    };
    for sequence in [2, u64::MAX] {
        let changed = SignedObject::issue(
            original.scope(),
            &p.f.high.signer,
            original.id(),
            original.issued(),
            original.expires(),
            Body::PairReadyRound {
                header: header.clone(),
                sequence,
                profile: *profile,
            },
        )
        .unwrap();
        // Ordinary authenticated verification still cannot advance or consume the one-shot.
        assert!(admit_ready(&mut p, &changed).is_err());
        assert!(p.f.low.join.confirmations.pairs[&p.f.id]
            .ready_admission
            .is_none());
    }
    let receipt = admit_ready(&mut p, &original).unwrap();
    assert_eq!(receipt.message, original.id());
}
