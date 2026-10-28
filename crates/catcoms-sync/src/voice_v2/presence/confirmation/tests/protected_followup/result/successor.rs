//! Actual native Finish-open is the prerequisite; only browser AEAD output is mocked here.
use super::*;
use crate::voice_v2::negotiation::{Exchange, SdpSuccessor, Stage};
use crate::voice_v2::presence::confirmation::successor::Preparation;
use crate::voice_v2::resources::{ResourceKind, GROUP_BYTES};
mod ready;

fn confirmed_high() -> ProtectedFixture {
    confirmed_high_in(fixture())
}
fn confirmed_high_in(p: ProtectedFixture) -> ProtectedFixture {
    let (mut p, finish) = super::transition::ready_to_finish_in(p);
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

#[test]
fn third_native_hello_finishes_with_every_proof_slot_retained_then_moves_original_packet_rights() {
    let mut p = fixture();
    let mut predecessor = None;
    for attempt in 0..3 {
        let hello = start(&mut p, predecessor);
        let (hello_object, hello_plain) = protected_seal(&mut p.f.low, &p.low, &hello, &p.f.clock);
        predecessor = Some(hello_object.id());
        let (_, high_open) = admit(&mut p.f.high, &p.high, hello_object, &p.f.clock);
        opened(
            &mut p.f.high,
            &p.high,
            &high_open,
            &hello_plain,
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
        let (proof_object, proof_plain) =
            protected_seal(&mut p.f.high, &p.high, &proof, &p.f.clock);
        if attempt != 2 {
            // Real published Proof rows remain retained while LOW observes lost replies.
            // No manufactured settled/candidate boolean substitutes for the next handshake.
            continue;
        }
        let (_, low_open) = admit(&mut p.f.low, &p.low, proof_object, &p.f.clock);
        opened(
            &mut p.f.low,
            &p.low,
            &low_open,
            &proof_plain,
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
        let (finish_object, finish_plain) =
            protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
        let (_, high_finish) = admit(&mut p.f.high, &p.high, finish_object, &p.f.clock);
        opened(
            &mut p.f.high,
            &p.high,
            &high_finish,
            &finish_plain,
            &p.f.clock,
            &mut p.f.rng,
        );
    }
    assert_eq!(p.f.high.join.published_source_packets().count(), 3);
    let before =
        p.f.high
            .join
            .ledger
            .packet_slots()
            .fill_unreserved_for_test();
    assert_eq!((before.0.len(), before.1.len()), (26, 29));
    let output = prepare(&mut p, |_, _| Ok(())).unwrap();
    assert!(output.confirmation_drained());
    assert!(p.f.high.join.published_source_packets().next().is_none());
    let surplus =
        p.f.high
            .join
            .ledger
            .packet_slots()
            .fill_unreserved_for_test();
    assert_eq!((surplus.0.len(), surplus.1.len()), (5, 2));
    // One incoming and one outgoing child remain prepaid even after all released surplus
    // is occupied. This establishes packet capacity, not causal Ready/Offer/RTC admission.
    let remaining =
        p.f.high
            .join
            .ledger
            .packet_slots()
            .fill_unreserved_for_test();
    assert!(remaining.0.is_empty() && remaining.1.is_empty());
}
fn pending(p: &ProtectedFixture) -> Arc<Exchange> {
    match &p.f.high.join.confirmations.pairs[&p.f.id].sdp_successor {
        Some(Preparation::Transferred { owner, .. }) => owner.clone(),
        _ => panic!("actual transferred native holder"),
    }
}

fn ready_producer(p: &ProtectedFixture) -> Arc<crate::voice_v2::ready::producer::Producer> {
    match &p.f.high.join.confirmations.pairs[&p.f.id].sdp_successor {
        Some(Preparation::Transferred {
            ready: Some(ready), ..
        }) => ready.producer_for_test(),
        _ => panic!("actual original Ready producer capacity"),
    }
}

#[test]
fn ready_producer_is_retained_across_final_refusal_and_issuance_never_resets_on_drop() {
    let mut p = confirmed_high();
    assert_eq!(
        prepare(&mut p, |_, _| Err(VoiceError::Capacity)).err(),
        Some(VoiceError::Capacity)
    );
    let first = ready_producer(&p);
    let output = prepare(&mut p, |_, _| Ok(())).unwrap();
    assert!(Arc::ptr_eq(&first, &ready_producer(&p)));
    assert_eq!(
        output.owner.prepare_ready_cache().err(),
        Some(VoiceError::Capacity)
    );
    let Some(Preparation::Transferred { ready, .. }) = &mut p
        .f
        .high
        .join
        .confirmations
        .pairs
        .get_mut(&p.f.id)
        .unwrap()
        .sdp_successor
    else {
        panic!("retained preparation")
    };
    drop(ready.take());
    drop(first);
    assert_eq!(
        output.owner.prepare_ready_cache().err(),
        Some(VoiceError::Capacity),
        "actual last cache/producer drop cannot rearm issuance"
    );
}

#[test]
fn ready_producer_full_peak_uses_original_shared_lanes_and_independent_stage_quotas() {
    let mut p = confirmed_high();
    let output = prepare(&mut p, |_, _| Ok(())).unwrap();
    let producer = ready_producer(&p);
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    let ordinary_crypto: Vec<_> = (0..3)
        .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    assert!(pool.charge(group, 0, ResourceKind::Crypto).is_err());
    let pressure = pool
        .charge(group, GROUP_BYTES - pool.counts().0, ResourceKind::Retained)
        .unwrap();
    for (stage, maximum) in [
        (Stage::LocalPacket, 2048),
        // Ready now pays its actual retained Packet and standalone map/set leaf graph.
        (Stage::Source, 692 + 512 + 3072),
        (
            Stage::Lower,
            2 * (692 + 512) + 4096 + crate::voice_v2::ROUTE_METADATA_BYTES,
        ),
        (Stage::Crypto, 6656),
    ] {
        assert_eq!(
            producer.charge(&pool, group, stage, 0).err(),
            Some(VoiceError::Bounds)
        );
        assert_eq!(
            producer.charge(&pool, group, stage, maximum + 1).err(),
            Some(VoiceError::Bounds)
        );
    }
    let foreign = Arc::new(crate::voice_v2::resources::Resources::default());
    assert_eq!(
        producer.charge(&foreign, group, Stage::Source, 1).err(),
        Some(VoiceError::Scope)
    );
    assert_eq!(
        producer.charge(&pool, [55; 32], Stage::Source, 1).err(),
        Some(VoiceError::Scope)
    );
    for stage in [
        Stage::RemotePacket,
        Stage::EncryptAction,
        Stage::DecryptAction,
        Stage::Result,
    ] {
        assert_eq!(
            producer.charge(&pool, group, stage, 1).err(),
            Some(VoiceError::Scope)
        );
    }
    assert_eq!(
        producer.charge(&pool, group, Stage::LocalPacket, 1).err(),
        Some(VoiceError::Capacity),
        "native Pair already owns sole cache claim"
    );
    let source = producer
        .charge(&pool, group, Stage::Source, 692 + 512 + 3072)
        .unwrap();
    assert_eq!(
        producer.charge(&pool, group, Stage::Source, 1).err(),
        Some(VoiceError::Capacity)
    );
    let lower_bytes = 2 * (692 + 512) + 4096 + crate::voice_v2::ROUTE_METADATA_BYTES;
    let first = producer
        .charge(&pool, group, Stage::Lower, lower_bytes)
        .unwrap();
    let second = producer
        .charge(&pool, group, Stage::Lower, lower_bytes)
        .unwrap();
    assert_eq!(
        producer.charge(&pool, group, Stage::Lower, 1).err(),
        Some(VoiceError::Capacity)
    );
    let crypto = producer.charge(&pool, group, Stage::Crypto, 6656).unwrap();
    assert_eq!(
        producer.charge(&pool, group, Stage::Crypto, 1).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(pool.counts().0, GROUP_BYTES);
    assert!(
        output.owner.charge(Stage::Source, 1).is_err(),
        "capacity grants no generic SDP or transmission authority"
    );
    drop(crypto);
    let crypto = producer.charge(&pool, group, Stage::Crypto, 6656).unwrap();
    output.owner.close();
    for stage in [
        Stage::LocalPacket,
        Stage::Source,
        Stage::Lower,
        Stage::Crypto,
    ] {
        assert_eq!(
            producer.charge(&pool, group, stage, 1).err(),
            Some(VoiceError::Unauthorized)
        );
    }
    drop((source, first, second, crypto, pressure, ordinary_crypto));
}

#[test]
fn ready_actual_credit_delays_whole_plan_cleanup_after_native_pair_custody_is_removed() {
    for stage in [Stage::Source, Stage::Lower, Stage::Crypto] {
        let mut p = confirmed_high();
        let chain = p.f.high.join.confirmations.pairs[&p.f.id]
            .chain
            .clone()
            .unwrap();
        let control = chain.workflow().unwrap().confirmation_control();
        let output = prepare(&mut p, |_, _| Ok(())).unwrap();
        let producer = ready_producer(&p);
        let pool = p._runtimes[1].resources();
        let group = p.f.high.context.scope.group;
        let actual = Arc::new(producer.charge(&pool, group, stage, 128).unwrap());
        let last = actual.clone();
        let whole = control.retire_all();
        output.owner.close();
        let preparation =
            p.f.high
                .join
                .confirmations
                .pairs
                .get_mut(&p.f.id)
                .unwrap()
                .sdp_successor
                .take();
        drop((preparation, producer, output, actual));
        assert!(
            !whole.is_drained(),
            "actual Ready credit still retains the original Exchange"
        );
        drop(last);
        assert!(
            whole.is_drained(),
            "last actual loan returns before whole-plan drainage"
        );
    }
}

#[test]
fn prepared_packet_claims_cannot_lend_before_native_activation() {
    let mut p = confirmed_high();
    let high = prepare(&mut p, |_, _| Ok(())).unwrap();
    let binding =
        p.f.high
            .join
            .chain_binding(p.f.id, &ca(&p.f.high.context, &p.f.high.lease));
    assert!(high.owner.successor_current_at(
        &binding,
        &p._runtimes[1].resources(),
        p.f.clock.monotonic_ms()
    ));
    high.owner
        .assert_inactive_packet_lenders_for_test(p.f.high.join.ledger.packet_slots());

    // LOW extraction is test-scoped capacity construction only. Production still has no
    // LOW factory before authenticated Ready. Its actual local Finish/epoch must exist so
    // no independent proof gate can hide accidental removal of the activation check.
    let chain = p.f.low.join.confirmations.pairs[&p.f.id]
        .chain
        .clone()
        .unwrap();
    let workflow = chain.workflow().unwrap();
    let control = workflow.confirmation_control();
    let ticket = control
        .transfer_sdp_capacity(control.prepare_sdp_capacity().unwrap())
        .unwrap();
    let low = Exchange::from_transferred(ticket, Vec::new());
    low.prepare_packet_claims().unwrap();
    let binding =
        p.f.low
            .join
            .chain_binding(p.f.id, &ca(&p.f.low.context, &p.f.low.lease));
    assert!(low.successor_current_at(
        &binding,
        &p._runtimes[0].resources(),
        p.f.clock.monotonic_ms()
    ));
    low.assert_inactive_packet_lenders_for_test(p.f.low.join.ledger.packet_slots());
    assert_eq!(
        low.prepare_ready_cache().err(),
        Some(VoiceError::Unauthorized),
        "LOW cannot construct HIGH Ready capacity"
    );
}

#[test]
fn sdp_successor_releases_obsolete_high_caches_without_waiting_for_signed_expiry() {
    let mut p = confirmed_high();
    let now = p.f.clock.monotonic_ms();
    assert!(p.f.high.join.published_source_packets().next().is_some());
    let output = prepare(&mut p, |_, _| Ok(())).unwrap();
    assert!(p.f.high.lease.is_live());
    assert!(p.f.high.join.confirmations.pairs[&p.f.id].established);
    assert!(p.f.high.join.published_source_packets().next().is_none());
    assert!(
        output.confirmation_drained(),
        "idle native caches must not pin old capacity"
    );
    assert_eq!(
        p.f.clock.monotonic_ms(),
        now,
        "no expiry or retry window was advanced"
    );
}

#[test]
fn sdp_successor_obsolete_source_waits_for_actual_lower_before_old_drain() {
    let mut p = confirmed_high();
    let object = p.f.high.join.published_source_packets().find(|object| {
        matches!(object.body(), Body::Encrypted { header, .. } if header.kind == EncryptedKind::ConfirmProof)
    }).unwrap().clone();
    let source = p.high.source().unwrap();
    let activity =
        p.f.high
            .join
            .source_packet_activity(
                &object,
                &ca(&p.f.high.context, &p.f.high.lease),
                Arc::new(p.f.clock.clone()),
            )
            .unwrap();
    source
        .admit_borrowed(&p.high, &object, activity, room(&p.f.high))
        .unwrap();
    let job = source
        .take_ready(
            &p.high,
            &[crate::voice_v2::helper::BoundRoute {
                device: p.f.low.context.local,
                peer: catcoms_rt::PeerId::from_u64(91),
            }],
        )
        .unwrap()
        .pop()
        .unwrap();
    let lower = job.keepalive();
    let output = prepare(&mut p, |_, _| Ok(())).unwrap();
    assert!(!job.authority().authorized());
    assert!(!output.confirmation_drained());
    assert_eq!(source.counts(), (1, 1, 1));
    drop(job);
    assert_eq!(
        source.counts(),
        (1, 1, 1),
        "actual detached Lower still retains flight"
    );
    assert!(!output.confirmation_drained());
    drop(lower);
    assert_eq!(source.counts(), (0, 0, 0));
    assert!(output.confirmation_drained());
    // Exact old signed identity cannot recreate attempts after terminal cache collection.
    assert!(p
        .f
        .high
        .join
        .source_packet_activity(
            &object,
            &ca(&p.f.high.context, &p.f.high.lease),
            Arc::new(p.f.clock.clone()),
        )
        .is_err());
}

#[test]
fn terminal_high_cleanup_cannot_remove_a_foreign_ledgers_confirmation_cache() {
    let mut p = confirmed_high();
    let chain = p.f.high.join.confirmations.pairs[&p.f.id]
        .chain
        .clone()
        .unwrap();
    let control = chain.workflow().unwrap().confirmation_control();
    let before =
        p.f.low
            .join
            .published_source_packets()
            .map(SignedObject::encode)
            .collect::<Vec<_>>();
    let _output = prepare(&mut p, |_, _| Ok(())).unwrap();
    assert_eq!(
        p.f.low.join.ledger.retire_high_confirmation_cache(&chain),
        Err(VoiceError::Scope)
    );
    let _whole = control.retire_all();
    assert_eq!(
        p.f.low.join.ledger.retire_whole_confirmation_cache(&chain),
        Err(VoiceError::Scope)
    );
    assert_eq!(
        p.f.low
            .join
            .published_source_packets()
            .map(SignedObject::encode)
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn terminal_high_cleanup_preserves_an_unrelated_incoming_token_and_its_replay_floor() {
    let mut p = confirmed_high();
    let peer = DeviceKeypair::from_seed(&[77; 32]);
    let handshake = [78; 32];
    let authority = ca(&p.f.high.context, &p.f.high.lease);
    // Build an ordinary unrelated packet at the PacketLedger seam. This tests cache
    // isolation only; it does not claim the outsider passed native Root membership.
    p.f.high
        .join
        .ledger
        .register_pair(
            PairBinding {
                peer: peer.device_id(),
                join: [79; 32],
                key_ref: [80; 32],
                handshake,
            },
            &authority,
        )
        .unwrap();
    p.f.high
        .join
        .ledger
        .establish(&handshake, &authority)
        .unwrap();
    let object = SignedObject::issue(
        p.f.high.context.scope,
        &peer,
        [81; 32],
        1000,
        16000,
        Body::Encrypted {
            header: EncryptedHeader {
                source_join: [79; 32],
                target: p.f.high.context.local,
                target_join: p.f.high.join.key.join,
                source_key_ref: [80; 32],
                target_key_ref: p.f.high.join.key.key_ref,
                handshake,
                low_to_high: peer.device_id() < p.f.high.context.local,
                kind: EncryptedKind::Ice,
                sequence: 1,
                offer: Some(OfferId {
                    issuer: peer.device_id(),
                    message: [82; 32],
                }),
            },
            ciphertext: vec![5; 17],
        },
    )
    .unwrap();
    let prepared =
        p.f.high
            .join
            .ledger
            .prepare_inbound(object.clone(), &authority, &p.f.clock)
            .unwrap();
    let event =
        p.f.high
            .join
            .ledger
            .commit_prepared_inbound(prepared, &authority, &p.f.clock, |_| Ok(()))
            .unwrap();
    let _output = prepare(&mut p, |_, _| Ok(())).unwrap();
    let authority = ca(&p.f.high.context, &p.f.high.lease);
    p.f.high
        .join
        .ledger
        .commit_inbound(
            event.token,
            event.aad(),
            event.header.sequence,
            event.ciphertext(),
            &authority,
            &p.f.clock,
        )
        .unwrap();
    assert_eq!(
        p.f.high
            .join
            .ledger
            .prepare_inbound(object, &authority, &p.f.clock)
            .err(),
        Some(VoiceError::Replay)
    );
}

#[test]
fn source_pruning_does_not_treat_low_local_finish_as_received_ready() {
    let (mut p, finish) = super::transition::ready_to_finish();
    let (object, _) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let source = p.low.source().unwrap();
    let activity =
        p.f.low
            .join
            .source_packet_activity(
                &object,
                &ca(&p.f.low.context, &p.f.low.lease),
                Arc::new(p.f.clock.clone()),
            )
            .unwrap();
    source
        .admit_borrowed(&p.low, &object, activity, room(&p.f.low))
        .unwrap();
    let chain = p.f.low.join.confirmations.pairs[&p.f.id]
        .chain
        .clone()
        .unwrap();
    let old = chain.workflow().unwrap().confirmation_control().retire();
    assert_eq!(
        source.counts(),
        (1, 0, 0),
        "HIGH-only pruning cannot substitute for LOW Ready admission"
    );
    assert!(!old.is_drained());
    assert!(p.f.low.lease.is_live());
    p.f.low.lease.revoke();
    assert_eq!(source.counts(), (0, 0, 0));
}

#[test]
fn explicit_whole_plan_closure_drops_only_idle_confirmation_caches_before_ready() {
    let (mut p, finish) = super::transition::ready_to_finish();
    let held_input = finish.input().unwrap();
    let (object, _) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let source = p.low.source().unwrap();
    let activity =
        p.f.low
            .join
            .source_packet_activity(
                &object,
                &ca(&p.f.low.context, &p.f.low.lease),
                Arc::new(p.f.clock.clone()),
            )
            .unwrap();
    source
        .admit_borrowed(&p.low, &object, activity, room(&p.f.low))
        .unwrap();
    let chain = p.f.low.join.confirmations.pairs[&p.f.id]
        .chain
        .clone()
        .unwrap();
    let before = p.f.clock.monotonic_ms();
    let whole = chain
        .workflow()
        .unwrap()
        .confirmation_control()
        .retire_all();
    p.f.low.join.collect_expired(&p.f.clock);
    assert_eq!(source.counts(), (0, 0, 0),
        "an explicit whole-Plan fence must retire idle LOW Finish without inventing Ready or waiting for signed expiry");
    assert!(p.f.low.lease.is_live());
    assert!(!p.f.low.join.confirmations.pairs[&p.f.id].retired);
    assert!(p.f.low.join.confirmations.pairs[&p.f.id].pin.is_live(),
        "old confirmation cache cleanup cannot retire the established pair used by independent media");
    assert_eq!(
        p.f.clock.monotonic_ms(),
        before,
        "cleanup never advances the original clock/deadline"
    );
    assert!(
        !whole.is_drained(),
        "actual borrowed Input must retain its original Workflow payment"
    );
    drop(held_input);
    assert!(
        whole.is_drained(),
        "only real last-owner return certifies variable retirement"
    );
}

#[test]
fn key_only_successor_closure_does_not_retire_old_confirmation_retry_custody() {
    let (mut p, finish) = super::transition::ready_to_finish();
    let control = finish.confirmation_control().unwrap();
    let plan = finish.protected_plan().unwrap();
    let (object, _) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let finite = plan
        .confirmation_custody(&control, p.f.clock.monotonic_ms())
        .unwrap();
    let keys = plan
        .established_key_custody(&finite, p.f.clock.monotonic_ms())
        .unwrap();
    let source = p.low.source().unwrap();
    let activity =
        p.f.low
            .join
            .source_packet_activity(
                &object,
                &ca(&p.f.low.context, &p.f.low.lease),
                Arc::new(p.f.clock.clone()),
            )
            .unwrap();
    source
        .admit_borrowed(&p.low, &object, activity.clone(), room(&p.f.low))
        .unwrap();
    keys.close_successor_effects();
    p.f.low.join.collect_expired(&p.f.clock);
    assert_eq!(source.counts(), (1, 0, 0));
    assert!(
        activity.check(&object).is_ok(),
        "key-only closure cannot broaden into old confirmation cancellation"
    );
    assert!(!keys.is_current());
}

#[test]
fn whole_plan_cache_pruning_cannot_recreate_source_before_control_actually_closes() {
    let (mut p, finish) = super::transition::ready_to_finish();
    let control = finish.confirmation_control().unwrap(); // Capture BEFORE actual engine settlement.
    let (object, _) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let source = p.low.source().unwrap();
    let activity =
        p.f.low
            .join
            .source_packet_activity(
                &object,
                &ca(&p.f.low.context, &p.f.low.lease),
                Arc::new(p.f.clock.clone()),
            )
            .unwrap();
    source
        .admit_borrowed(&p.low, &object, activity.clone(), room(&p.f.low))
        .unwrap();
    let whole = control.retire_all_before_control_for_test(|| {
        // Original Control/Workflow leases are STILL live at this exact hook. Only
        // the new whole-plan latch can prevent a pruned record being re-admitted.
        assert_eq!(source.counts(), (0, 0, 0));
        assert_eq!(
            source
                .admit_borrowed(&p.low, &object, activity.clone(), room(&p.f.low))
                .err(),
            Some(VoiceError::Unauthorized)
        );
        assert_eq!(
            source.counts(),
            (0, 0, 0),
            "no fresh attempt record/allowance after closure"
        );
    });
    p.f.low.join.collect_expired(&p.f.clock);
    assert!(
        !whole.is_drained(),
        "retained original Source authority remains a real paid owner"
    );
    drop(activity);
    assert!(whole.is_drained());
}

#[tokio::test]
async fn explicit_whole_plan_closure_cancels_finish_but_actual_lower_and_ack_keep_their_payment() {
    use futures::FutureExt;
    let (mut p, finish) = super::transition::ready_to_finish();
    let control = finish.confirmation_control().unwrap(); // Capture BEFORE actual engine settlement.
    let (object, _) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let source = p.low.source().unwrap();
    let activity =
        p.f.low
            .join
            .source_packet_activity(
                &object,
                &ca(&p.f.low.context, &p.f.low.lease),
                Arc::new(p.f.clock.clone()),
            )
            .unwrap();
    source
        .admit_borrowed(&p.low, &object, activity, room(&p.f.low))
        .unwrap();
    let job = source
        .take_ready(
            &p.low,
            &[crate::voice_v2::helper::BoundRoute {
                device: p.f.high.context.local,
                peer: catcoms_rt::PeerId::from_u64(91),
            }],
        )
        .unwrap()
        .pop()
        .unwrap();
    let lower = job.keepalive();
    let authority = job.authority();
    let mut canceled = Box::pin(authority.revoked());
    assert!(futures::poll!(&mut canceled).is_pending());
    let whole = control.retire_all();
    p.f.low.join.collect_expired(&p.f.clock);
    assert!(!authority.authorized());
    assert!(
        canceled.now_or_never().is_some(),
        "terminal original packet wakes cancellation without clock advance"
    );
    assert_eq!(source.counts(), (1, 1, 1));
    assert!(!whole.is_drained());
    drop(job);
    assert_eq!(
        source.counts(),
        (1, 1, 1),
        "detached actual Lower retains the original active flight"
    );
    assert!(!whole.is_drained());
    drop(lower);
    assert_eq!(source.counts(), (0, 0, 0));
    assert!(
        !whole.is_drained(),
        "an actual retained ACK authority still owns the original packet/payment"
    );
    drop(authority);
    assert!(whole.is_drained());
}

#[test]
fn sdp_successor_whole_retirement_fences_real_confirmed_owner_and_waits_for_its_last_drop() {
    let mut p = confirmed_high();
    let chain = p.f.high.join.confirmations.pairs[&p.f.id]
        .chain
        .clone()
        .unwrap();
    let control = chain.workflow().unwrap().confirmation_control();
    let output = prepare(&mut p, |_, _| Ok(())).unwrap();
    let pool = p._runtimes[1].resources();
    let binding =
        p.f.high
            .join
            .chain_binding(p.f.id, &ca(&p.f.high.context, &p.f.high.lease));
    assert!(output.owner.successor_current_at(&binding, &pool, 1000));
    let whole = control.retire_all();
    assert!(
        !output.owner.successor_current_at(&binding, &pool, 1000),
        "whole parent closure immediately fences the identical confirmed successor"
    );
    assert!(!whole.is_drained());
    // Actual old Source expiry plus permission-independent pending housekeeping, not a
    // fabricated witness. The retained output still owns the actual successor envelope.
    p.f.clock.advance_ms(20_001);
    p.f.high.join.collect_expired(&p.f.clock);
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_successor
        .is_none());
    assert!(
        output.confirmation_drained(),
        "old Workflow has its separate drainage meaning"
    );
    assert!(
        !whole.is_drained(),
        "original successor output is still a real owner"
    );
    drop(output);
    assert!(whole.is_drained());
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
fn sdp_successor_progression_refusal_preserves_same_pending_owner_and_quiet_expiry() {
    let mut p = confirmed_high();
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    // Preserve the genuine pending owner through a refused final fence, then let actual
    // old packet custody retire. The group progression claim now refuses an independent
    // foreign Exchange BEFORE registration; it cannot displace or block this original.
    assert_eq!(
        prepare(&mut p, |_, _| Err(VoiceError::Capacity)).err(),
        Some(VoiceError::Capacity)
    );
    let original = pending(&p);
    p.f.clock.advance_ms(20_001);
    p.f.high.join.collect_expired(&p.f.clock);
    let before = pool.counts();
    assert_eq!(
        Exchange::new_for_test(pool.clone(), group, p.f.high.join.ledger.action_slots()).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(pool.counts(), before);
    assert!(Arc::ptr_eq(&original, &pending(&p)));
    let recovered = prepare(&mut p, |_, _| Ok(())).unwrap();
    assert!(Arc::ptr_eq(&original, &recovered.owner));
    assert!(Arc::ptr_eq(&pool.exchange(group).unwrap(), &original));
    assert!(Arc::ptr_eq(&original, &pending(&p)));
    // Actual permission-independent LocalJoin housekeeping must close this SAME original
    // even after its final-fence retry registered it. Historical outputs retain its index.
    p.f.clock.advance_ms(9_999);
    p.f.high.join.collect_expired(&p.f.clock);
    assert!(!original.successor_current_at(
        &p.f.high
            .join
            .chain_binding(p.f.id, &ca(&p.f.high.context, &p.f.high.lease)),
        &pool,
        31_000
    ));
    drop(recovered);
    assert_eq!(
        prepare(&mut p, |_, _| Ok(())).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_successor
        .is_none());
    assert!(!original.is_live());
    assert!(Arc::ptr_eq(&pool.exchange(group).unwrap(), &original));
    // The unused lane has returned although historical native/observation owners remain.
    let crypto: Vec<_> = (0..4)
        .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    assert!(pool.charge(group, 0, ResourceKind::Crypto).is_err());
    drop(crypto);
    drop(original);
    assert!(
        pool.exchange(group).is_none(),
        "actual last output unregisters"
    );
}

#[test]
fn sdp_successor_native_final_expiry_keeps_original_closed_custody_without_registering() {
    let mut p = confirmed_high();
    // First native monotonic sample prepays original member rows. The next sample is
    // final local admission AFTER extraction; expiry must be forced at that actual fence.
    let next = p.origin_clock.reads.load(Ordering::Acquire) + 2;
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
    let before = p._runtimes[1].resources().counts();
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
    // Native prepayment now rejects the forged member BEFORE any reservation/extraction
    // or immutable helper stamp. An invalid request must not strand otherwise healthy work.
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_successor
        .is_none());
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .chain
        .as_ref()
        .unwrap()
        .workflow()
        .unwrap()
        .is_live());
    assert_eq!(p._runtimes[1].resources().counts(), before);
    assert!(p._runtimes[1]
        .resources()
        .exchange(p.f.high.context.scope.group)
        .is_none());
    prepare(&mut p, |_, _| Ok(())).unwrap();
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
    assert_eq!(
        Exchange::new_for_test(pool.clone(), group, p.f.high.join.ledger.action_slots()).err(),
        Some(VoiceError::Capacity),
        "actual historical output still retains the original progression claim"
    );
    drop(output);
    assert!(
        pool.exchange(group).is_none(),
        "native custody did not strand the registry"
    );
    let next =
        Exchange::new_for_test(pool.clone(), group, p.f.high.join.ledger.action_slots()).unwrap();
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
