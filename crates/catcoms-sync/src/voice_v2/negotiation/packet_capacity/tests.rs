//! Capacity-only fixtures, not Ready/SDK authorization. Real native Finish tests exercise
//! the same movement through LocalJoin separately; these isolate full table/last-slot cases.
use super::*;
use crate::voice_v2::chain::{Binding, PreparedChain, Workflow};
use crate::voice_v2::packets::PacketSlots;

fn fixture(
    low: bool,
) -> (
    Arc<Resources>,
    Arc<PreparedChain>,
    Arc<Workflow>,
    PacketSlots,
) {
    let pool = Arc::new(Resources::default());
    let tables = PacketSlots::new();
    let binding = Binding {
        scope: Scope::for_group(b"original packet rights", Some([7; 32]), 44),
        epoch: 3,
        local: DeviceId::from_bytes([1; 32]),
        local_join: [1; 32],
        local_key: [2; 32],
        handshake: [3; 32],
        peer: DeviceId::from_bytes([4; 32]),
        peer_join: [5; 32],
        peer_key: [6; 32],
        generation: 1,
        call: CallLease::default(),
        pair: CallLease::default(),
        deadline: 16000,
        low,
    };
    let chain = PreparedChain::new(
        pool.clone(),
        binding,
        Arc::new(crate::voice_v2::crypto_actions::slots::Slots::default()),
        &tables,
    )
    .unwrap();
    let workflow = chain.workflow().unwrap();
    (pool, chain, workflow, tables)
}

#[test]
fn fully_occupied_original_claims_move_without_refund_and_partial_split_retries_same_parents() {
    for low in [false, true] {
        let (pool, chain, workflow, tables) = fixture(low);
        let control = workflow.confirmation_control();
        let (mut incoming, mut outgoing) = workflow.occupy_packet_slots_for_test(&tables);
        assert_eq!(
            (incoming.len(), outgoing.len()),
            if low { (3, 6) } else { (6, 3) }
        );
        let ordinary = tables.fill_unreserved_for_test();
        assert_eq!(
            (
                ordinary.0.len() + incoming.len(),
                ordinary.1.len() + outgoing.len()
            ),
            (32, 32)
        );
        let ticket = control
            .transfer_sdp_capacity(control.prepare_sdp_capacity().unwrap())
            .unwrap();
        let owner = Exchange::from_transferred(ticket, Vec::new());
        assert_eq!(owner.prepare_packet_claims(), Err(VoiceError::Capacity));
        assert!(tables.fill_unreserved_for_test().0.is_empty());
        drop(incoming.pop().unwrap());
        // First child succeeds, second refuses. Its abandoned first child must return to
        // the original parent, without briefly returning a position to the ordinary table.
        assert_eq!(owner.prepare_packet_claims(), Err(VoiceError::Capacity));
        let free = tables.fill_unreserved_for_test();
        assert!(free.0.is_empty() && free.1.is_empty());
        drop(outgoing.pop().unwrap());
        owner.prepare_packet_claims().unwrap();
        owner.prepare_packet_claims().unwrap();
        let free = tables.fill_unreserved_for_test();
        assert!(free.0.is_empty() && free.1.is_empty());
        drop((incoming, outgoing));
        owner.close();
        let whole = control.retire_all();
        drop((workflow, owner));
        assert!(whole.is_drained());
        drop((whole, chain, ordinary));
        assert_eq!(pool.counts(), (0, 0));
        let all = tables.fill_unreserved_for_test();
        assert_eq!((all.0.len(), all.1.len()), (32, 32));
    }
}

#[test]
fn actual_successor_packet_slot_retains_exchange_after_bytes_and_other_owners_are_gone() {
    let (pool, chain, workflow, tables) = fixture(false);
    let control = workflow.confirmation_control();
    let ticket = control
        .transfer_sdp_capacity(control.prepare_sdp_capacity().unwrap())
        .unwrap();
    let owner = Exchange::from_transferred(ticket, Vec::new());
    owner.prepare_packet_claims().unwrap();
    let actual = {
        let state = owner.state.lock().unwrap();
        let claims = state.packet_claims.as_ref().unwrap();
        let slot = claims.incoming_slot(&tables).unwrap();
        assert_eq!(
            claims.incoming_slot(&tables).err(),
            Some(VoiceError::Capacity)
        );
        PacketSlot::successor(slot, owner.clone())
    };
    let whole = control.retire_all();
    owner.close();
    drop((workflow, owner));
    assert!(!whole.is_drained());
    let ordinary = tables.fill_unreserved_for_test();
    assert_eq!((ordinary.0.len(), ordinary.1.len()), (31, 32));
    drop(actual);
    assert!(whole.is_drained());
    let last = tables.fill_unreserved_for_test();
    assert_eq!((last.0.len(), last.1.len()), (1, 0));
    drop((whole, chain, ordinary, last));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn abandoned_extracted_ticket_closes_unused_original_claims_before_a_delayed_old_slot_returns() {
    let (pool, chain, workflow, tables) = fixture(false);
    let control = workflow.confirmation_control();
    let (incoming, mut outgoing) = workflow.occupy_packet_slots_for_test(&tables);
    let delayed = outgoing.pop().unwrap();
    drop((incoming, outgoing));
    let ticket = control
        .transfer_sdp_capacity(control.prepare_sdp_capacity().unwrap())
        .unwrap();
    drop(ticket);
    let available = tables.fill_unreserved_for_test();
    assert_eq!((available.0.len(), available.1.len()), (32, 31));
    drop(delayed);
    let last = tables.fill_unreserved_for_test();
    assert_eq!((last.0.len(), last.1.len()), (0, 1));
    let whole = control.retire_all();
    drop(workflow);
    assert!(whole.is_drained());
    drop((whole, chain, available, last));
    assert_eq!(pool.counts(), (0, 0));
}

pub(super) fn assert_inactive_lenders(owner: &Arc<Exchange>, tables: &PacketSlots) {
    let low = owner.successor.as_ref().unwrap().binding().low;
    let binding = owner.successor.as_ref().unwrap().binding();
    for incoming in [false, true] {
        let offer = if incoming { !low } else { low };
        let header = EncryptedHeader {
            source_join: if incoming {
                binding.peer_join
            } else {
                binding.local_join
            },
            source_key_ref: if incoming {
                binding.peer_key
            } else {
                binding.local_key
            },
            target: if incoming {
                binding.local
            } else {
                binding.peer
            },
            target_join: if incoming {
                binding.local_join
            } else {
                binding.peer_join
            },
            target_key_ref: if incoming {
                binding.local_key
            } else {
                binding.peer_key
            },
            handshake: binding.handshake,
            kind: if offer {
                EncryptedKind::Offer
            } else {
                EncryptedKind::Answer
            },
            low_to_high: offer,
            sequence: 1,
            offer: Some(super::super::super::OfferId {
                issuer: if low { binding.local } else { binding.peer },
                message: [22; 32],
            }),
        };
        let result = if incoming {
            owner.incoming_sdp_slot(&header, tables)
        } else {
            owner.outgoing_sdp_slot(&header, tables)
        };
        assert_eq!(result.err(), Some(VoiceError::Unauthorized));
    }
    // Bypass authorization ONLY in this capacity fixture: rejection must leave both
    // original positions available, even when an ordinary table is already full.
    let ordinary = tables.fill_unreserved_for_test();
    let additional = tables.fill_unreserved_for_test();
    assert!(additional.0.is_empty() && additional.1.is_empty());
    {
        let state = owner.state.lock().unwrap();
        let claims = state.packet_claims.as_ref().unwrap();
        let incoming = claims.incoming_slot(tables).unwrap();
        let outgoing = claims.outgoing_slot(tables).unwrap();
        drop((incoming, outgoing));
    }
    drop(ordinary);
}
