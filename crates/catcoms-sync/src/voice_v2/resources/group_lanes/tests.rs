use super::*;

#[test]
fn accessor_holds_paid_strong_custody_across_original_owner_drop_and_never_exports_a_weak() {
    for accessor_wins in [false, true] {
        let pool = Arc::new(Resources::default());
        let group = [1; 32];
        let unrelated = pool.charge(group, 1, ResourceKind::Retained).unwrap();
        let original = pool.group_forward_lanes(group).unwrap();
        let accessor = if accessor_wins {
            pool.existing_group_forward_lanes(group)
        } else {
            None
        };
        drop(original);
        if let Some(accessor) = accessor {
            assert_eq!(pool.counts(), (GROUP_FORWARD_METADATA_BYTES + 1, 0));
            assert!(pool.existing_group_forward_lanes(group).is_some());
            drop(accessor);
        }
        assert_eq!(pool.counts(), (1, 0));
        assert!(pool.existing_group_forward_lanes(group).is_none());
        drop(unrelated);
        assert_eq!(pool.counts(), (0, 0));
    }
}

#[test]
fn refused_two_lane_preparation_refunds_only_its_demonstrably_unstarted_candidate() {
    let pool = Arc::new(Resources::default());
    let group = [1; 32];
    let mut work: Vec<_> = (0..3)
        .map(|_| pool.charge(group, 1, ResourceKind::Forward).unwrap())
        .collect();
    let before = pool.counts();
    assert_eq!(
        pool.group_forward_lanes(group).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(pool.counts(), before);
    assert_eq!(pool.state.lock().unwrap().groups[&group].slots[7], 3);
    assert!(pool.existing_group_forward_lanes(group).is_none());
    drop(work.pop()); // actual unrelated work returns; no synthetic attempt/slot refund
    let lanes = pool.group_forward_lanes(group).unwrap();
    assert_eq!(pool.state.lock().unwrap().groups[&group].slots[7], 4);
    drop((work, lanes));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn concurrent_factories_install_one_original_group_owner_without_losing_the_winner() {
    let pool = Arc::new(Resources::default());
    let barrier = std::sync::Barrier::new(2);
    let (left, right) = std::thread::scope(|threads| {
        let other = threads.spawn(|| {
            barrier.wait();
            pool.group_forward_lanes([1; 32])
        });
        barrier.wait();
        let here = pool.group_forward_lanes([1; 32]);
        (here, other.join().unwrap())
    });
    let owner = pool.group_forward_lanes([1; 32]).unwrap();
    // A transient local refusal is allowed when a competitor is still preparing, but every
    // successful reply names the SINGLE installed owner; no waiter restarts its original job.
    for reply in [&left, &right] {
        match reply {
            Ok(reply) => assert!(Arc::ptr_eq(reply, &owner)),
            Err(error) => assert_eq!(*error, VoiceError::Capacity),
        }
    }
    assert_eq!(pool.state.lock().unwrap().groups[&[1; 32]].slots[7], 2);
    drop((left, right, owner));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn independent_shared_graph_and_borrow_wrappers_fit_their_declared_payment() {
    let bytes = std::mem::size_of::<GroupForwardLanes>()
        + 2 * std::mem::size_of::<WorkLane>()
        + 3 * 2 * std::mem::size_of::<usize>() // actual Arc allocation headers
        + std::mem::size_of::<std::sync::Weak<GroupForwardLanes>>()
        + 2 * std::mem::size_of::<LaneBorrow>()
        + 128; // weak index node/key and synchronous factory/completed-handle scratch
    assert!(
        bytes <= GROUP_FORWARD_METADATA_BYTES,
        "shared owner graph {bytes} exceeds its independent payment {GROUP_FORWARD_METADATA_BYTES}"
    );
}

#[test]
fn same_group_phases_share_two_real_lanes_and_phase_close_cannot_retire_the_other() {
    let pool = Arc::new(Resources::default());
    let group = [1; 32];
    let a = pool.reserve(group, 4096).unwrap();
    let b = pool.reserve(group, 4096).unwrap();
    let lanes = pool.group_forward_lanes(group).unwrap();
    let other = pool.group_forward_lanes(group).unwrap();
    assert!(Arc::ptr_eq(&lanes, &other));
    assert_eq!(pool.state.lock().unwrap().groups[&group].slots[7], 2);
    let lower_a = lanes.charge(&pool, group, &a, 512).unwrap();
    let lower_b = other.charge(&pool, group, &b, 512).unwrap();
    let before = pool.counts();
    assert_eq!(
        other.charge(&pool, group, &b, 1).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(pool.counts(), before);
    a.close();
    drop((a, lanes));
    drop(lower_a); // actual old borrower returns, not a timeout/refund
    let next = other.charge(&pool, group, &b, 512).unwrap();
    drop((next, lower_b, b, other));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn ordinary_helpers_borrow_idle_shared_slots_and_keep_original_owner_through_last_credit() {
    let pool = Arc::new(Resources::default());
    let group = [1; 32];
    let lanes = pool.group_forward_lanes(group).unwrap();
    let base: Vec<_> = (0..2)
        .map(|_| pool.charge(group, 16, ResourceKind::Forward).unwrap())
        .collect();
    let first = pool.charge(group, 16, ResourceKind::Forward).unwrap();
    let late = Arc::new(pool.charge(group, 16, ResourceKind::Forward).unwrap());
    assert_eq!(pool.state.lock().unwrap().groups[&group].slots[7], 4);
    assert!(pool.charge(group, 1, ResourceKind::Forward).is_err());
    drop((lanes, first));
    let same = pool.group_forward_lanes(group).unwrap();
    assert!(Arc::ptr_eq(
        late.lane.as_ref().unwrap().group.as_ref().unwrap(),
        &same
    ));
    let again = pool.charge(group, 16, ResourceKind::Forward).unwrap();
    drop((again, same, base));
    assert!(
        pool.existing_group_forward_lanes(group).is_some(),
        "actual late Lower keeps the same owner indexed"
    );
    let last = late.clone();
    drop(late);
    assert!(pool.existing_group_forward_lanes(group).is_some());
    drop(last);
    assert!(pool.existing_group_forward_lanes(group).is_none());
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn failed_cross_pool_group_or_byte_admission_never_spends_an_idle_lane() {
    let pool = Arc::new(Resources::default());
    let foreign = Arc::new(Resources::default());
    let group = [1; 32];
    let reserve = pool.reserve(group, 8).unwrap();
    let wrong = foreign.reserve(group, 8).unwrap();
    let lanes = pool.group_forward_lanes(group).unwrap();
    let before = pool.counts();
    for result in [
        lanes.charge(&foreign, group, &reserve, 1),
        lanes.charge(&pool, [2; 32], &reserve, 1),
        lanes.charge(&pool, group, &wrong, 1),
    ] {
        assert_eq!(result.err(), Some(VoiceError::Scope));
    }
    assert_eq!(
        lanes.charge(&pool, group, &reserve, 9).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(pool.counts(), before);
    let a = lanes.charge(&pool, group, &reserve, 1).unwrap();
    let b = lanes.charge(&pool, group, &reserve, 1).unwrap();
    drop((a, b, lanes, reserve, wrong));
    assert_eq!(pool.counts(), (0, 0));
    assert_eq!(foreign.counts(), (0, 0));
}

#[test]
fn dead_weak_index_is_removed_before_payment_returns_while_other_group_metadata_stays_live() {
    let pool = Arc::new(Resources::default());
    let group = [1; 32];
    let retained = pool.charge(group, 1, ResourceKind::Retained).unwrap();
    let lanes = pool.group_forward_lanes(group).unwrap();
    drop(lanes);
    let state = pool.state.lock().unwrap();
    assert!(
        std::sync::Weak::ptr_eq(
            &state.groups[&group].outgoing_lanes,
            &std::sync::Weak::new()
        ),
        "the live group cannot pin an unpaid dead shared-owner allocation"
    );
    drop(state);
    assert_eq!(pool.counts(), (1, 0));
    drop(retained);
    assert_eq!(pool.counts(), (0, 0));
}
