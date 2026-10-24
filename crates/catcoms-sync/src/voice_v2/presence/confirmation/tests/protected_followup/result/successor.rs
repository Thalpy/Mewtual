//! Actual native Finish-open is the prerequisite; only browser AEAD output is mocked here.
use super::*;
use crate::voice_v2::negotiation::{Exchange, SdpSuccessor, Stage};
use crate::voice_v2::presence::confirmation::successor::Preparation;
use crate::voice_v2::resources::{ResourceKind, GROUP_BYTES};

fn confirmed_high() -> ProtectedFixture {
    let (mut p, finish) = super::transition::ready_to_finish();
    let (object, plain) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let (_, action) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    opened(
        &mut p.f.high,
        &p.high,
        &action,
        &plain,
        &p.f.clock,
        &mut p.f.rng,
    );
    p
}
fn prepare(
    p: &mut ProtectedFixture,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<SdpSuccessor, VoiceError> {
    let room = room(&p.f.high);
    p.f.high.join.prepare_high_sdp_successor(
        &p.high,
        &room,
        p.f.id,
        &[],
        &ca(&p.f.high.context, &p.f.high.lease),
        &p.f.clock,
        final_owner,
    )
}
fn pending(p: &ProtectedFixture) -> Arc<Exchange> {
    match &p.f.high.join.confirmations.pairs[&p.f.id].sdp_successor {
        Some(Preparation::Transferred(owner)) => owner.clone(),
        _ => panic!("actual transferred native holder"),
    }
}

#[test]
fn sdp_successor_low_refusal_preserves_original_queued_finish_authority_without_payment() {
    let (mut p, finish) = super::transition::ready_to_finish();
    let (object, _) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let activity =
        p.f.low
            .join
            .source_packet_activity(
                &object,
                &ca(&p.f.low.context, &p.f.low.lease),
                Arc::new(p.f.clock.clone()),
            )
            .unwrap();
    let before = p.f.pool.counts();
    let room = room(&p.f.low);
    assert_eq!(
        p.f.low
            .join
            .prepare_high_sdp_successor(
                &p.low,
                &room,
                p.f.id,
                &[],
                &ca(&p.f.low.context, &p.f.low.lease),
                &p.f.clock,
                |_, _| panic!("no low successor admission"),
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(p.f.pool.counts(), before);
    assert!(
        activity.check(&object).is_ok(),
        "original Finish still may reach high"
    );
    assert!(p.f.low.join.confirmations.pairs[&p.f.id]
        .sdp_successor
        .is_none());
    assert_eq!(
        prepare(&mut p, |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized),
        "high cannot prepare before opening Finish"
    );
}

#[test]
fn sdp_successor_preserves_same_owner_across_final_refusal_and_every_lending_surface_stays_closed()
{
    let mut p = confirmed_high();
    assert_eq!(
        prepare(&mut p, |_, _| Err(VoiceError::Capacity)).err(),
        Some(VoiceError::Capacity)
    );
    let first = pending(&p);
    let pool = p._runtimes[1].resources();
    let before = pool.counts();
    let output = prepare(&mut p, |_, _| Ok(())).unwrap();
    assert!(Arc::ptr_eq(&first, &output.owner));
    assert_eq!(
        pool.counts(),
        before,
        "recovery does not allocate a new phase"
    );
    assert_eq!(output.transition_until(), 31_000);
    assert!(!first.is_live());
    for stage in [
        Stage::LocalPacket,
        Stage::RemotePacket,
        Stage::Source,
        Stage::Lower,
        Stage::Crypto,
    ] {
        assert!(first.charge(stage, 1).is_err());
    }
    assert!(first
        .prepare_action_credits(Stage::EncryptAction, 1, 1)
        .is_err());
    assert!(first
        .prepare_action_credits(Stage::DecryptAction, 1, 1)
        .is_err());
    assert!(first
        .action_slot(&p.f.high.join.ledger.action_slots())
        .is_err());
    assert!(first.retain_result(b"v=0\r\n").is_err());
    for budget in [first.encryption_budget(), first.decryption_budget()] {
        assert!(budget
            .charge(
                &pool,
                p.f.high.context.scope.group,
                1,
                ResourceKind::Retained
            )
            .is_err());
    }
    let message = [81; 32];
    let object = SignedObject::issue(
        p.f.low.context.scope,
        &p.f.low.signer,
        message,
        1000,
        16_000,
        Body::Encrypted {
            header: EncryptedHeader {
                source_join: p.f.low.join.key.join,
                target: p.f.high.context.local,
                target_join: p.f.high.join.key.join,
                source_key_ref: p.f.low.join.key.key_ref,
                target_key_ref: p.f.high.join.key.key_ref,
                handshake: p.f.id,
                low_to_high: true,
                kind: EncryptedKind::Offer,
                sequence: 3,
                offer: Some(OfferId {
                    issuer: p.f.low.context.local,
                    message,
                }),
            },
            ciphertext: vec![3; 17],
        },
    )
    .unwrap();
    let frame = Envelope::Direct(object).encode();
    assert!(Envelope::peek_encrypted(&frame).is_ok());
    assert!(!first.lends_verification(p.f.low.context.local, &frame));
    assert!(
        !p.f.high.join.ledger.has_sdp_for_test(),
        "pending cannot reach ordinary SDP pruning/intake"
    );
    let room = room(&p.f.high);
    assert_eq!(
        p.f.high
            .join
            .prepare_sdp_exchange(
                p.f.id,
                &ca(&p.f.high.context, &p.f.high.lease),
                &mut p.f.high.limits,
                &[],
                &p.f.clock
            )
            .err(),
        Some(VoiceError::Unauthorized),
        "protected pair cannot acquire a fresh lane"
    );
    assert!(room.lease.is_live());
    assert_eq!(pool.counts(), before);
}

#[test]
fn sdp_successor_reserved_busy_retry_moves_under_full_ordinary_bytes_and_crypto_pressure() {
    let mut p = confirmed_high();
    let chain = p.f.high.join.confirmations.pairs[&p.f.id]
        .chain
        .clone()
        .unwrap();
    let workflow = chain.workflow().unwrap();
    let handoff = workflow.borrow_handoff(2048).unwrap();
    assert_eq!(
        prepare(&mut p, |_, _| Ok(())).err(),
        Some(VoiceError::Capacity)
    );
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    assert!(matches!(
        p.f.high.join.confirmations.pairs[&p.f.id].sdp_successor,
        Some(Preparation::Reserved { .. })
    ));
    let busy: Vec<_> = (0..3)
        .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    let fill = pool
        .charge(group, GROUP_BYTES - pool.counts().0, ResourceKind::Retained)
        .unwrap();
    assert!(pool.charge(group, 1, ResourceKind::Retained).is_err());
    assert!(pool.charge(group, 0, ResourceKind::Crypto).is_err());
    drop(handoff);
    let output = prepare(&mut p, |_, _| Ok(())).unwrap();
    assert_eq!(output.transition_until(), 31_000);
    assert!(!workflow.is_live());
    assert!(!output.owner.is_live());
    assert!(
        pool.charge(group, 0, ResourceKind::Crypto).is_err(),
        "original lane stayed occupied"
    );
    drop((fill, busy));
}

#[test]
fn sdp_successor_actual_lower_delays_retirement_without_reopening_or_renewing_old_phase() {
    let mut p = confirmed_high();
    let chain = p.f.high.join.confirmations.pairs[&p.f.id]
        .chain
        .clone()
        .unwrap();
    let workflow = chain.workflow().unwrap();
    let lower = workflow
        .charge(
            &p._runtimes[1].resources(),
            p.f.high.context.scope.group,
            128,
            ResourceKind::Forward,
        )
        .unwrap();
    let output = prepare(&mut p, |_, _| Ok(())).unwrap();
    drop(workflow);
    p.f.clock.advance_ms(20_001);
    p.f.high.join.collect_expired(&p.f.clock);
    assert!(
        !output.confirmation_drained(),
        "actual Lower still owns original Workflow"
    );
    drop(lower);
    assert!(output.confirmation_drained());
    assert_eq!(
        prepare(&mut p, |_, _| Ok(())).unwrap().transition_until(),
        31_000
    );
}

#[test]
fn sdp_successor_registry_refusal_keeps_same_native_owner_and_quiet_housekeeping_expires_it() {
    let mut p = confirmed_high();
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    // Preserve the genuine pending owner through a refused final fence, then let actual
    // old packet custody retire. Stacking both192KiB exchanges on that old96KiB Workflow
    // plus infrastructure would hit the earlier byte gate, never the registry under test.
    assert_eq!(
        prepare(&mut p, |_, _| Err(VoiceError::Capacity)).err(),
        Some(VoiceError::Capacity)
    );
    let original = pending(&p);
    p.f.clock.advance_ms(20_001);
    p.f.high.join.collect_expired(&p.f.clock);
    let blocker =
        Exchange::new_for_test(pool.clone(), group, p.f.high.join.ledger.action_slots()).unwrap();
    pool.register_exchange(group, &blocker).unwrap();
    assert_eq!(
        prepare(&mut p, |_, _| Ok(())).err(),
        Some(VoiceError::Capacity)
    );
    assert!(Arc::ptr_eq(&original, &pending(&p)));
    assert!(Arc::ptr_eq(&pool.exchange(group).unwrap(), &blocker));
    assert_eq!(
        prepare(&mut p, |_, _| Ok(())).err(),
        Some(VoiceError::Capacity)
    );
    assert!(Arc::ptr_eq(&original, &pending(&p)));
    // This is the actual permission-independent LocalJoin housekeeping used by Root. The
    // original pending owner is NOT in the global weak index; collection must not rely on it.
    p.f.clock.advance_ms(9_999);
    p.f.high.join.collect_expired(&p.f.clock);
    assert!(!original.successor_current_at(
        &p.f.high
            .join
            .chain_binding(p.f.id, &ca(&p.f.high.context, &p.f.high.lease)),
        &pool,
        31_000
    ));
    drop(blocker);
    assert_eq!(
        prepare(&mut p, |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_successor
        .is_none());
    assert!(!original.is_live());
    assert!(
        pool.exchange(group).is_none(),
        "expired pending cannot newly register"
    );
    // The unused lane has returned although historical native/observation owners remain.
    let crypto: Vec<_> = (0..4)
        .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    assert!(pool.charge(group, 0, ResourceKind::Crypto).is_err());
    drop(crypto);
}

#[test]
fn sdp_successor_native_final_expiry_keeps_original_closed_custody_without_registering() {
    let mut p = confirmed_high();
    let next = p.origin_clock.reads.load(Ordering::Acquire) + 1;
    *p.origin_clock.advance.lock().unwrap() = Some((next, 30_000));
    // Native time is sampled after extraction and outside Group/Policy. The final native
    // admission must refuse the now-expired window, not renew it or lose the taken lane.
    assert_eq!(
        prepare(&mut p, |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    let owner = pending(&p);
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    assert!(pool.exchange(group).is_none());
    p.f.high.join.collect_expired(&p.f.clock);
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_successor
        .is_none());
    assert!(!owner.is_live());
    assert_eq!(
        prepare(&mut p, |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    let crypto: Vec<_> = (0..4)
        .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    drop(crypto);
}

#[test]
fn sdp_successor_end_in_final_fence_and_original_epoch_change_cannot_revive_capacity() {
    for before_payment in [false, true] {
        let mut p = confirmed_high();
        let pool = p._runtimes[1].resources();
        let group = p.f.high.context.scope.group;
        if before_payment {
            p.high
                .sync_view(View {
                    epoch: p.f.high.context.epoch + 1,
                    members: [p.f.low.context.local, p.f.high.context.local].into(),
                    policy: p.f.high.context.scope.policy,
                    eligible: true,
                    indirect: true,
                })
                .unwrap();
            let before = pool.counts();
            assert_eq!(
                prepare(&mut p, |_, _| panic!("epoch changed before payment")).err(),
                Some(VoiceError::Unauthorized)
            );
            assert_eq!(pool.counts(), before);
            assert!(p.f.high.join.confirmations.pairs[&p.f.id]
                .sdp_successor
                .is_none());
        } else {
            let original = p.f.high.lease.clone();
            assert_eq!(
                prepare(&mut p, |_, _| {
                    original.revoke();
                    Ok(())
                })
                .err(),
                Some(VoiceError::Unauthorized)
            );
            let owner = pending(&p);
            p.f.high.join.collect_expired(&p.f.clock);
            assert!(p.f.high.join.confirmations.pairs[&p.f.id]
                .sdp_successor
                .is_none());
            assert!(!owner.is_live());
            let crypto: Vec<_> = (0..4)
                .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
                .collect();
            drop(crypto);
        }
        assert!(pool.exchange(group).is_none());
    }
}

#[test]
fn sdp_successor_native_helper_membership_refuses_spoofed_callback_and_choice_stays_immutable() {
    let mut p = confirmed_high();
    let outsider = DeviceKeypair::from_seed(&[8; 32]).device_id();
    let room = room(&p.f.high);
    // ca's convenience membership callback says true for everyone. The actual native
    // borrowed Group must still refuse a helper outside the original current member set.
    assert_eq!(
        p.f.high
            .join
            .prepare_high_sdp_successor(
                &p.high,
                &room,
                p.f.id,
                &[outsider],
                &ca(&p.f.high.context, &p.f.high.lease),
                &p.f.clock,
                |_, _| Ok(())
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    let original = pending(&p);
    assert_eq!(
        prepare(&mut p, |_, _| Ok(())).err(),
        Some(VoiceError::Scope)
    );
    assert!(Arc::ptr_eq(&original, &pending(&p)));
    assert!(p._runtimes[1]
        .resources()
        .exchange(p.f.high.context.scope.group)
        .is_none());
}

#[test]
fn sdp_successor_registered_expiry_releases_native_registry_custody_but_not_real_observations() {
    let mut p = confirmed_high();
    let output = prepare(&mut p, |_, _| Ok(())).unwrap();
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    assert!(Arc::ptr_eq(&pool.exchange(group).unwrap(), &output.owner));
    p.f.clock.advance_ms(30_000);
    p.f.high.join.collect_expired(&p.f.clock);
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_successor
        .is_none());
    assert!(!output.owner.is_live());
    assert_eq!(
        output.transition_until(),
        31_000,
        "history never renews the window"
    );
    let next =
        Exchange::new_for_test(pool.clone(), group, p.f.high.join.ledger.action_slots()).unwrap();
    assert_eq!(
        pool.register_exchange(group, &next),
        Err(VoiceError::Capacity),
        "actual historical output still retains its original allocation"
    );
    drop(output);
    assert!(
        pool.exchange(group).is_none(),
        "native custody did not strand the registry"
    );
    pool.register_exchange(group, &next).unwrap();
    assert!(Arc::ptr_eq(&pool.exchange(group).unwrap(), &next));
    assert_eq!(
        prepare(&mut p, |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized),
        "the expired original pair cannot manufacture another successor"
    );
}

#[test]
fn sdp_successor_registered_expiry_with_no_outputs_frees_registry_without_ending_join() {
    let mut p = confirmed_high();
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    drop(prepare(&mut p, |_, _| Ok(())).unwrap());
    assert!(
        pool.exchange(group).is_some(),
        "native Pair still owns pending capacity"
    );
    p.f.clock.advance_ms(30_000);
    p.f.high.join.collect_expired(&p.f.clock);
    assert!(pool.exchange(group).is_none());
    assert!(
        p.f.high.lease.is_live(),
        "cleanup did not require End/rejoin"
    );
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_successor
        .is_none());
    let next =
        Exchange::new_for_test(pool.clone(), group, p.f.high.join.ledger.action_slots()).unwrap();
    pool.register_exchange(group, &next).unwrap();
}

#[test]
fn sdp_successor_pair_drop_closes_unused_capacity_while_original_output_survives() {
    let mut p = confirmed_high();
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    let output = prepare(&mut p, |_, _| Ok(())).unwrap();
    drop(p);
    assert!(!output.owner.is_live());
    assert!(
        Arc::ptr_eq(&pool.exchange(group).unwrap(), &output.owner),
        "actual surviving output still owns metadata"
    );
    let crypto: Vec<_> = (0..4)
        .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    assert!(pool.charge(group, 0, ResourceKind::Crypto).is_err());
    assert_eq!(output.transition_until(), 31_000);
    drop(crypto);
    drop(output);
    assert_eq!(pool.counts(), (0, 0));
    assert!(pool.exchange(group).is_none());
}
