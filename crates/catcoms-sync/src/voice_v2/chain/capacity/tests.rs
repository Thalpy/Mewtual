//! Real pools/credits and original capacity owners; no fabricated Ready/effect authority.
use super::*;
use crate::voice_v2::chain::tests::binding;
use crate::voice_v2::negotiation::Exchange;
use crate::voice_v2::packets::PacketSlots;
use crate::voice_v2::resources::GROUP_BYTES;
use std::sync::Barrier;

fn reserve(
    pool: &Arc<Resources>,
    slots: &Arc<slots::Slots>,
    packets: &PacketSlots,
) -> ChainCapacity {
    ChainCapacity::reserve(
        pool.clone(),
        binding(1).scope.group,
        true,
        slots.clone(),
        packets,
    )
    .unwrap()
}
fn assert_positions_returned(slots: &Arc<slots::Slots>, packets: &PacketSlots) {
    let all = slots.charge(32).unwrap();
    let (incoming, outgoing) = packets.fill_unreserved_for_test();
    assert_eq!((incoming.len(), outgoing.len()), (32, 32));
    drop((all, incoming, outgoing));
}

#[test]
fn neutral_capacity_reserves_both_roles_once_without_consuming_an_action() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let packets = PacketSlots::new();
    let original = ChainCapacity::reserve_neutral(
        pool.clone(),
        binding(1).scope.group,
        slots.clone(),
        &packets,
    )
    .unwrap();
    let actions = slots.charge(20).unwrap();
    assert_eq!(slots.charge(1).err(), Some(VoiceError::Capacity));
    let (incoming, outgoing) = packets.fill_unreserved_for_test();
    assert_eq!((incoming.len(), outgoing.len()), (26, 26));
    drop((actions, incoming, outgoing));
    drop(original);
    assert_positions_returned(&slots, &packets);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn neutral_capacity_cannot_be_relabelled_by_either_existing_chain_consumer() {
    for low in [false, true] {
        let pool = Arc::new(Resources::default());
        let slots = Arc::new(slots::Slots::default());
        let packets = PacketSlots::new();
        let mut purpose = binding(1);
        purpose.low = low;
        let original = ChainCapacity::reserve_neutral(
            pool.clone(),
            purpose.scope.group,
            slots.clone(),
            &packets,
        )
        .unwrap();
        let decomposed = std::cell::Cell::new(false);
        assert_eq!(
            original
                .into_chain_after_decomposition(&pool, purpose, &slots, &packets, || decomposed
                    .set(true))
                .err(),
            Some(VoiceError::Scope)
        );
        assert!(
            !decomposed.get(),
            "legacy refusal must precede original decomposition"
        );
        assert_positions_returned(&slots, &packets);
        assert_eq!(pool.counts(), (0, 0));
    }
}

#[test]
fn neutral_partial_packet_refusal_returns_exact_unused_positions_and_payments() {
    for full_incoming in [false, true] {
        let pool = Arc::new(Resources::default());
        let slots = Arc::new(slots::Slots::default());
        let packets = PacketSlots::new();
        let (incoming, outgoing) = packets.fill_unreserved_for_test();
        let pressure = if full_incoming {
            drop(outgoing);
            incoming
        } else {
            drop(incoming);
            outgoing
        };
        assert_eq!(
            ChainCapacity::reserve_neutral(
                pool.clone(),
                binding(1).scope.group,
                slots.clone(),
                &packets
            )
            .err(),
            Some(VoiceError::Capacity)
        );
        assert_eq!(pool.counts(), (0, 0));
        drop(pressure);
        assert_positions_returned(&slots, &packets);
    }
}

#[test]
fn undersized_confirmation_row_refuses_before_payment_allocation_or_position_debit() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let packets = PacketSlots::new();
    let group = binding(1).scope.group;
    for allowance in [0, 1536, usize::MAX] {
        let observed = std::cell::Cell::new(false);
        let refused = ChainCapacity::reserve_with_row_bytes(
            pool.clone(),
            group,
            true,
            slots.clone(),
            &packets,
            allowance,
            || observed.set(true),
        );
        assert_eq!(refused.err(), Some(VoiceError::Capacity));
        assert!(!observed.get(), "metadata payment hook must not run");
        assert_eq!(pool.counts(), (0, 0));
        assert_positions_returned(&slots, &packets);
    }
    let funded = reserve(&pool, &slots, &packets);
    assert!(funded.inner.is_some());
    drop(funded);
    assert_positions_returned(&slots, &packets);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn payment_precedes_packet_allocation_and_each_refusal_returns_exact_original_owners() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let packets = PacketSlots::new();
    let group = binding(1).scope.group;
    let observed = std::cell::Cell::new(0);
    let pressure = pool
        .charge(group, GROUP_BYTES, ResourceKind::Retained)
        .unwrap();
    let refused = ChainCapacity::reserve_after_payment(
        pool.clone(),
        group,
        true,
        slots.clone(),
        &packets,
        || observed.set(1),
    );
    assert_eq!(refused.err(), Some(VoiceError::Capacity));
    assert_eq!(
        observed.get(),
        0,
        "no packet allocation after failed payment"
    );
    assert_eq!(pool.counts(), (GROUP_BYTES, 0));
    assert_positions_returned(&slots, &packets);
    drop(pressure);

    for incoming_full in [false, true] {
        let (incoming, outgoing) = packets.fill_unreserved_for_test();
        let held = if incoming_full {
            drop(outgoing);
            incoming
        } else {
            drop(incoming);
            outgoing
        };
        observed.set(0);
        let refused = ChainCapacity::reserve_after_payment(
            pool.clone(),
            group,
            true,
            slots.clone(),
            &packets,
            || {
                assert_eq!(
                    pool.counts(),
                    (METADATA_BYTES, 0),
                    "original fee exists before packet refusal"
                );
                observed.set(1);
            },
        );
        assert_eq!(refused.err(), Some(VoiceError::Capacity));
        assert_eq!(observed.get(), 1);
        assert_eq!(pool.counts(), (0, 0));
        drop(held);
        assert_positions_returned(&slots, &packets);
    }
}

#[test]
fn inert_preclaim_has_no_workflow_or_exchange_and_reserves_exact_role_positions() {
    for low in [false, true] {
        let pool = Arc::new(Resources::default());
        let slots = Arc::new(slots::Slots::default());
        let packets = PacketSlots::new();
        let group = binding(1).scope.group;
        let capacity =
            ChainCapacity::reserve(pool.clone(), group, low, slots.clone(), &packets).unwrap();
        assert_eq!(pool.retained_phase_indices_for_test(group), (false, false));
        assert!(pool.workflow(group).is_none());
        assert!(pool.exchange(group).is_none());
        let remaining_actions = slots.charge(20).unwrap();
        assert_eq!(slots.charge(1).err(), Some(VoiceError::Capacity));
        let (incoming, outgoing) = packets.fill_unreserved_for_test();
        assert_eq!(
            (incoming.len(), outgoing.len()),
            if low { (29, 26) } else { (26, 29) }
        );
        drop((remaining_actions, incoming, outgoing, capacity));
        assert_positions_returned(&slots, &packets);
        assert_eq!(pool.counts(), (0, 0));
    }
}

#[test]
fn consume_under_full_ordinary_pressure_preserves_all_original_claims_and_deadline() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let packets = PacketSlots::new();
    let capacity = reserve(&pool, &slots, &packets);
    let original_claim = capacity.inner.as_ref().unwrap().claim.share();
    let group = binding(1).scope.group;
    let pressure = pool
        .charge(group, GROUP_BYTES - pool.counts().0, ResourceKind::Retained)
        .unwrap();
    let before = pool.counts();
    let chain =
        PreparedChain::from_capacity(pool.clone(), binding(1), slots.clone(), &packets, capacity)
            .unwrap();
    assert_eq!(
        pool.counts(),
        before,
        "consumption makes no new ordinary charge"
    );
    assert_eq!(
        chain.binding.deadline, 16_000,
        "original deadline moves unchanged"
    );
    let workflow = chain.workflow().unwrap();
    assert!(original_claim.same_claim(&workflow._retirement.as_ref().unwrap().progress));
    assert_eq!(pool.retained_phase_indices_for_test(group), (true, false));
    let derive = chain.prepare_derive().unwrap();
    drop(derive);
    chain.close();
    drop((workflow, chain, original_claim, pressure));
    assert_positions_returned(&slots, &packets);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn consume_rejects_foreign_resources_tables_group_and_role_without_effects() {
    for wrong in 0..5 {
        let pool = Arc::new(Resources::default());
        let other_pool = Arc::new(Resources::default());
        let slots = Arc::new(slots::Slots::default());
        let other_slots = Arc::new(slots::Slots::default());
        let packets = PacketSlots::new();
        let replacement_packets = PacketSlots::new();
        let capacity = reserve(&pool, &slots, &packets);
        let mut current = binding(1);
        if wrong == 3 {
            current.scope.group = [99; 32];
        }
        if wrong == 4 {
            current.low = false;
        }
        let result = PreparedChain::from_capacity(
            if wrong == 0 {
                other_pool.clone()
            } else {
                pool.clone()
            },
            current,
            if wrong == 1 {
                other_slots.clone()
            } else {
                slots.clone()
            },
            if wrong == 2 {
                &replacement_packets
            } else {
                &packets
            },
            capacity,
        );
        assert_eq!(
            result.err(),
            Some(VoiceError::Scope),
            "wrong identity case {wrong}"
        );
        assert_eq!(pool.counts(), (0, 0));
        assert_eq!(other_pool.counts(), (0, 0));
        assert_positions_returned(&slots, &packets);
        assert_positions_returned(&other_slots, &replacement_packets);
    }
}

#[test]
fn dead_binding_is_terminal_before_reservation_and_at_prepaid_consumption() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let packets = PacketSlots::new();
    let dead = binding(1);
    dead.call.revoke();
    let group = dead.scope.group;
    let pressure = pool
        .charge(group, GROUP_BYTES, ResourceKind::Retained)
        .unwrap();
    assert_eq!(
        PreparedChain::new(pool.clone(), dead, slots.clone(), &packets).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(pool.counts(), (GROUP_BYTES, 0));
    drop(pressure);
    assert_positions_returned(&slots, &packets);

    let capacity = reserve(&pool, &slots, &packets);
    let pressure_bytes = GROUP_BYTES - pool.counts().0;
    let pressure = pool
        .charge(group, pressure_bytes, ResourceKind::Retained)
        .unwrap();
    let dead = binding(2);
    dead.pair.revoke();
    assert_eq!(
        PreparedChain::from_capacity(pool.clone(), dead, slots.clone(), &packets, capacity).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(pool.counts(), (pressure_bytes, 0));
    assert_positions_returned(&slots, &packets);
    drop(pressure);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn held_inert_capacity_excludes_standalone_and_confirmation_in_both_orders() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let packets = PacketSlots::new();
    let group = binding(1).scope.group;
    let capacity = reserve(&pool, &slots, &packets);
    let before = pool.counts();
    assert_eq!(
        Exchange::new_for_test(pool.clone(), group, Arc::new(slots::Slots::default())).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(
        PreparedChain::new(
            pool.clone(),
            binding(2),
            Arc::new(slots::Slots::default()),
            &PacketSlots::new()
        )
        .err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(pool.counts(), before);
    drop(capacity);
    let standalone = Exchange::new_for_test(pool.clone(), group, slots.clone()).unwrap();
    assert_eq!(
        ChainCapacity::reserve(pool.clone(), group, true, slots.clone(), &packets).err(),
        Some(VoiceError::Capacity)
    );
    drop(standalone);
    let capacity = reserve(&pool, &slots, &packets);
    drop(capacity);
    assert_positions_returned(&slots, &packets);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn inert_outer_payment_survives_actual_body_destruction_and_final_claim_release() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let packets = PacketSlots::new();
    let mut capacity = reserve(&pool, &slots, &packets);
    let (entered, resume) = (Arc::new(Barrier::new(2)), Arc::new(Barrier::new(2)));
    capacity
        .inner
        .as_mut()
        .unwrap()
        .claim
        .park_release_for_test(entered.clone(), resume.clone());
    let worker = std::thread::spawn(move || drop(capacity));
    entered.wait();
    let parked = pool.counts();
    // No workflow or exchange was ever installed; the exact progression slot still
    // excludes a candidate until this actual final publisher is released.
    let refusal = ChainCapacity::reserve(
        pool.clone(),
        binding(1).scope.group,
        true,
        slots.clone(),
        &packets,
    )
    .err();
    resume.wait();
    worker.join().unwrap();
    assert_eq!(parked, (crate::voice_v2::ready::capacity::METADATA, 0));
    assert_eq!(refusal, Some(VoiceError::Capacity));
    assert_positions_returned(&slots, &packets);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn original_root_payment_survives_actual_return_of_all_its_inert_capacity_objects() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let packets = PacketSlots::new();
    let mut capacity = reserve(&pool, &slots, &packets);
    let (entered, resume) = (Arc::new(Barrier::new(2)), Arc::new(Barrier::new(2)));
    capacity.inner.as_mut().unwrap().before_root_metadata = Some(RootReturnBoundary {
        entered: entered.clone(),
        resume: resume.clone(),
    });
    let worker = std::thread::spawn(move || drop(capacity));
    entered.wait();
    let actual = pool.counts();
    // The ORIGINAL real action/packet owners have actually returned, while their
    // Root fee must still exist. Record first, resume/join, then assert to avoid leaks.
    let all_actions = slots.charge(32);
    let (incoming, outgoing) = packets.fill_unreserved_for_test();
    let positions = (all_actions.is_ok(), incoming.len(), outgoing.len());
    resume.wait();
    worker.join().unwrap();
    assert_eq!(
        actual,
        (
            METADATA_BYTES + crate::voice_v2::ready::capacity::METADATA,
            0
        )
    );
    assert_eq!(positions, (true, 32, 32));
    drop((all_actions, incoming, outgoing));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn unwind_after_real_decomposition_returns_all_original_capacity_without_a_bound_owner() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let packets = PacketSlots::new();
    let capacity = reserve(&pool, &slots, &packets);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        capacity.into_chain_after_decomposition(&pool, binding(1), &slots, &packets, || {
            panic!("deterministic failure after original inert decomposition");
        })
    }));
    assert!(result.is_err());
    assert_eq!(
        pool.retained_phase_indices_for_test(binding(1).scope.group),
        (false, false)
    );
    assert_positions_returned(&slots, &packets);
    assert_eq!(pool.counts(), (0, 0));
}
