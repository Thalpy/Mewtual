//! Deterministic reservation/accounting tests. Real Actor/Session attachment is
//! covered separately by bootstrap Slot tests; these create no fake Root result.
use super::*;
use crate::voice_handoff::ledger::tests::selection;
use catcoms_sync::voice_v2::resources::GROUP_BYTES;

fn identity() -> Arc<SessionIdentity> {
    Arc::new(SessionIdentity([31; 32]))
}

#[test]
fn parent_claim_owns_original_fees_and_no_row_or_action() {
    let (_coordinator, selected, _) = selection();
    let ledger = Arc::new(Ledger::default());
    let session = identity();
    let before = selected.resources().counts();
    let claim = ledger
        .reserve_parent(selected.clone(), session.clone(), 1)
        .unwrap();
    assert!(claim.matches(&selected, &session, 1));
    assert!(claim.belongs_to(&ledger));
    assert_eq!(
        selected.resources().counts(),
        (before.0 + ROW_BYTES + 2048, before.1)
    );
    let id = claim.returned.id;
    assert!(matches!(ledger.row(id), Err(VoiceError::Unauthorized)));
    {
        let index = ledger.index.lock().unwrap();
        assert!(index.rows.iter().all(Option::is_none));
        assert_eq!(
            index.reserved[id.slot].map(NonZeroU64::get),
            Some(id.serial)
        );
    }
    drop(claim);
    assert_eq!(selected.resources().counts(), before);
    assert!(ledger
        .index
        .lock()
        .unwrap()
        .reserved
        .iter()
        .all(Option::is_none));
}

#[test]
fn parent_claim_rejects_equal_labels_foreign_pool_ledger_session_and_generation() {
    let (_coordinator, selected, _) = selection();
    let (_other_coordinator, foreign, _) = selection();
    let ledger = Arc::new(Ledger::default());
    let other_ledger = Arc::new(Ledger::default());
    let session = identity();
    let foreign_before = foreign.resources().counts();
    let claim = ledger
        .reserve_parent(selected.clone(), session.clone(), 7)
        .unwrap();
    assert!(!claim.matches(&foreign, &session, 7));
    assert!(
        !claim.matches(&selected, &identity(), 7),
        "equal bytes are not physical provenance"
    );
    assert!(!claim.matches(&selected, &session, 6));
    assert!(!claim.matches(&selected, &session, 8));
    assert!(!claim.matches(&selected, &session, 0));
    assert!(!claim.belongs_to(&other_ledger));
    assert_eq!(foreign.resources().counts(), foreign_before);
}

#[test]
fn parent_claim_blocks_all_32_positions_before_any_ordinary_factory() {
    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let session = identity();
    let mut claims = Vec::new();
    for generation in 1..=ROWS as u64 {
        claims.push(
            ledger
                .reserve_parent(selected.clone(), session.clone(), generation)
                .unwrap(),
        );
    }
    let full = selected.resources().counts();
    assert!(matches!(
        ledger.begin(selected.clone(), Purpose::Derive(selector)),
        Err(VoiceError::Capacity)
    ));
    assert!(matches!(
        ledger.reserve_parent(selected.clone(), session.clone(), 33),
        Err(VoiceError::Capacity)
    ));
    assert_eq!(
        selected.resources().counts(),
        full,
        "refusal returns both speculative fees"
    );
    let original = claims.remove(0).returned.id;
    let Begin::Factory(ticket) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("actual factory")
    };
    assert_eq!(ticket.id().slot, original.slot);
    assert!(ticket.id().serial > original.serial);
    assert_eq!(
        ledger
            .index
            .lock()
            .unwrap()
            .reserved
            .iter()
            .flatten()
            .count(),
        ROWS - 1
    );
}

#[test]
fn parent_claim_excludes_the_confirmation_child_allocator() {
    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let Begin::Factory(parent) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("parent")
    };
    let parent_row = ledger.row(parent.id()).unwrap();
    let session = identity();
    let mut claims = Vec::new();
    for generation in 1..ROWS as u64 {
        claims.push(
            ledger
                .reserve_parent(selected.clone(), session.clone(), generation)
                .unwrap(),
        );
    }
    let purpose = Purpose::Confirmation {
        parent: parent.id(),
        request: ConfirmationPurpose::Start(None),
    };
    let before = selected.resources().counts();
    // This is the actual allocator shared by begin_child. Its outer Root-plan
    // authorization is deliberately not claimed by this allocator-only test.
    let mut index = ledger.index.lock().unwrap();
    assert!(matches!(
        ledger.begin_indexed(selected.clone(), purpose, Some(&parent_row), &mut index),
        Err(VoiceError::Capacity)
    ));
    drop(index);
    assert_eq!(selected.resources().counts(), before);
    let returned = claims.pop().unwrap().returned.id;
    let mut index = ledger.index.lock().unwrap();
    let Begin::Factory(child) = ledger
        .begin_indexed(selected, purpose, Some(&parent_row), &mut index)
        .unwrap()
    else {
        panic!("child")
    };
    assert_eq!(child.id().slot, returned.slot);
    assert!(child.id().serial > returned.serial);
    drop(index); // actual ticket destruction is always outside Index
}

#[test]
fn parent_claim_partial_fee_or_index_pressure_returns_the_same_allowance() {
    let (_coordinator, selected, _) = selection();
    let ledger = Arc::new(Ledger::default());
    let resources = selected.resources();
    let group = selected.binding().scope.group;
    let filler = resources
        .charge(
            group,
            GROUP_BYTES - resources.group_counts(group).unwrap().0 - ROW_BYTES,
            ResourceKind::Retained,
        )
        .unwrap();
    let full = resources.counts();
    assert!(matches!(
        ledger.reserve_parent(selected.clone(), identity(), 1),
        Err(VoiceError::Capacity)
    ));
    assert_eq!(
        resources.counts(),
        full,
        "prepared-phase refusal returns the first Row fee"
    );
    assert!(ledger
        .index
        .lock()
        .unwrap()
        .reserved
        .iter()
        .all(Option::is_none));
    drop(filler);
    let before = resources.counts();
    let index = ledger.index.lock().unwrap();
    assert!(matches!(
        ledger.reserve_parent(selected, identity(), 1),
        Err(VoiceError::Capacity)
    ));
    assert_eq!(
        resources.counts(),
        before,
        "busy Index cannot strand paid preparation"
    );
    assert!(index.reserved.iter().all(Option::is_none));
}

#[test]
fn parent_claim_zero_dead_lease_and_exhausted_serial_never_install_a_stamp() {
    let (_coordinator, selected, _) = selection();
    let ledger = Arc::new(Ledger::default());
    let before = selected.resources().counts();
    assert!(matches!(
        ledger.reserve_parent(selected.clone(), identity(), 0),
        Err(VoiceError::Unauthorized)
    ));
    ledger.index.lock().unwrap().serial = u64::MAX;
    assert!(matches!(
        ledger.reserve_parent(selected.clone(), identity(), 1),
        Err(VoiceError::Capacity)
    ));
    assert_eq!(selected.resources().counts(), before);
    assert!(ledger
        .index
        .lock()
        .unwrap()
        .reserved
        .iter()
        .all(Option::is_none));
    selected.lease().revoke();
    assert!(matches!(
        ledger.reserve_parent(selected, identity(), 1),
        Err(VoiceError::Unauthorized)
    ));
}

#[test]
fn parent_claim_stamp_survives_actual_fee_destruction_until_original_return() {
    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let before = selected.resources().counts();
    let mut claim = ledger
        .reserve_parent(selected.clone(), identity(), 1)
        .unwrap();
    let id = claim.returned.id;
    let (entered, at_return) = std::sync::mpsc::sync_channel(0);
    let (release, released) = std::sync::mpsc::channel();
    claim.returned.before_release = Some((entered, released));
    let worker = std::thread::spawn(move || drop(claim));
    let reached = at_return.recv_timeout(std::time::Duration::from_secs(5));
    let during = selected.resources().counts();
    let Begin::Factory(other) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("other actual factory")
    };
    let other_slot = other.id().slot;
    let occupied = ledger.index.lock().unwrap().reserved[id.slot].map(NonZeroU64::get);
    let _ = release.send(());
    worker.join().unwrap();
    reached.unwrap();
    assert_eq!(
        during, before,
        "both actual fees have already returned at this barrier"
    );
    assert_eq!(occupied, Some(id.serial));
    assert_ne!(
        other_slot, id.slot,
        "original final-return frame still owns the stamp"
    );
    assert!(ledger.index.lock().unwrap().reserved[id.slot].is_none());
}

#[test]
fn parent_claim_unwind_destroys_original_preparation_before_reusing_the_stamp() {
    struct Unwind {
        _claim: ParentClaim,
    }
    impl Drop for Unwind {
        fn drop(&mut self) {
            panic!("controlled owner unwind");
        }
    }
    let (_coordinator, selected, _) = selection();
    let ledger = Arc::new(Ledger::default());
    let before = selected.resources().counts();
    let claim = ledger
        .reserve_parent(selected.clone(), identity(), 1)
        .unwrap();
    let id = claim.returned.id;
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(Unwind {
            _claim: claim
        })))
        .is_err()
    );
    assert_eq!(selected.resources().counts(), before);
    let next = ledger.reserve_parent(selected, identity(), 2).unwrap();
    assert_eq!(next.returned.id.slot, id.slot);
    assert!(next.returned.id.serial > id.serial);
}

#[test]
fn parent_claim_inventory_and_reservation_index_fit_existing_fees() {
    let bytes = std::mem::size_of::<ParentClaim>() + ParentClaim::preparation_layout_bytes();
    eprintln!(
        "inert parent graph {bytes}/{ROW_BYTES}; shared index {}/1024",
        std::mem::size_of::<Index>()
    );
    assert!(bytes <= ROW_BYTES, "inert parent graph {bytes}/{ROW_BYTES}");
    assert!(
        std::mem::size_of::<Index>() <= 1024,
        "index {} /1024",
        std::mem::size_of::<Index>()
    );
}
