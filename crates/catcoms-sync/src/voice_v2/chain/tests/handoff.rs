//! Application-copy custody shares the existing protected envelope and survives End.
use super::*;

#[test]
fn workflow_rejects_foreign_same_group_pool_without_charging_either_pool() {
    let pool = Arc::new(Resources::default());
    let foreign = Arc::new(Resources::default());
    let chain = prepare_chain(pool.clone(), binding(8), Arc::new(slots::Slots::default())).unwrap();
    let workflow = chain
        .control
        .lock()
        .unwrap()
        .workflow
        .as_ref()
        .unwrap()
        .clone();
    let before = pool.counts();
    for kind in [
        ResourceKind::Retained,
        ResourceKind::Crypto,
        ResourceKind::Forward,
    ] {
        assert_eq!(
            workflow
                .charge(&foreign, chain.binding.scope.group, 512, kind)
                .err(),
            Some(VoiceError::Scope)
        );
        assert_eq!(pool.counts(), before);
        assert_eq!(foreign.counts(), (0, 0));
    }
    // The borrowed pool still admits exactly the original resource owner.
    let credit = workflow
        .charge(
            &pool,
            chain.binding.scope.group,
            512,
            ResourceKind::Retained,
        )
        .unwrap();
    drop(credit);
    chain.close();
    drop((workflow, chain));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn retirement_observer_keeps_original_metadata_pool_after_external_owners_drop() {
    let pool = Arc::new(Resources::default());
    let original_pool = Arc::downgrade(&pool);
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (queue, action) = publish(&chain, &pool, &slots);
    let loan = action.borrow_handoff(16 * 1024).unwrap();
    let input = action.input().unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    action.settle_engine();
    let retirement = loan.retire_confirmation_observed();
    drop((pool, chain, action, queue, slots));
    assert!(!retirement.is_drained());
    drop(input);
    assert!(retirement.is_drained());
    assert_eq!(
        original_pool.upgrade().unwrap().counts(),
        (METADATA_BYTES, 0)
    );
    drop(retirement);
    assert!(
        original_pool.upgrade().is_none(),
        "no hidden pool or notifier owner survives"
    );
}

#[test]
fn observed_retirement_waits_for_original_inputs_and_lower_owners_without_pinning_workflow() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (queue, action) = publish(&chain, &pool, &slots);
    let loan = action.borrow_handoff(16 * 1024).unwrap();
    let original = Arc::downgrade(&loan.workflow);
    let input = action.input().unwrap();
    let lower = loan
        .workflow
        .charge(&pool, chain.binding.scope.group, 512, ResourceKind::Forward)
        .unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    action.settle_engine();
    let retirement = loan.retire_confirmation_observed();
    assert!(!retirement.is_drained());
    assert!(chain.control.lock().unwrap().closed);
    drop((chain, action, queue));
    assert!(original.upgrade().is_some());
    drop(input);
    assert!(
        !retirement.is_drained(),
        "actual lower credit still owns the original Workflow"
    );
    drop(lower);
    assert!(
        original.upgrade().is_none(),
        "observer must not itself pin variable capacity"
    );
    assert!(retirement.is_drained());
    assert_eq!(
        pool.counts(),
        (METADATA_BYTES, 0),
        "only original observer metadata remains"
    );
    let next = prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
    let replacement = pool.counts();
    // Observation has no close/effect method and does not adopt a replacement reservation.
    assert!(retirement.is_drained());
    assert_eq!(pool.counts(), replacement);
    assert!(!next.control.lock().unwrap().closed);
    next.close();
    drop(next);
    assert_eq!(pool.counts(), (METADATA_BYTES, 0));
    drop(retirement);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn original_publisher_proves_field_destruction_instead_of_weak_zero() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    // This isolated private field-order fixture intentionally extracts the ACTUAL open
    // Workflow before closing Control/history. The separate consumed-loan test above proves
    // the public observer's provenance. Explicit close would already release unused bytes.
    let held = chain.control.lock().unwrap().workflow.take().unwrap();
    let retirement = ConfirmationRetirement {
        binding: chain.binding.clone(),
    };
    let original = Arc::downgrade(&held);
    chain.close();
    drop(chain);
    let mut workflow = match Arc::try_unwrap(held) {
        Ok(workflow) => workflow,
        Err(_) => panic!("fixture must have the actual sole remaining Workflow owner"),
    };
    assert_eq!(original.strong_count(), 0);
    assert!(pool.counts().0 > METADATA_BYTES);
    assert!(
        !retirement.is_drained(),
        "weak zero precedes actual field destruction"
    );
    let publisher = workflow._retirement.take().unwrap(); // park the SAME original last field
    drop(workflow); // normal Drop + every real capacity field, no unsafe/destructor bypass
    assert_eq!(pool.counts(), (METADATA_BYTES, 0));
    assert!(!retirement.is_drained());
    drop(publisher); // only the SAME original last-field publisher can establish readiness
    assert!(retirement.is_drained());
    drop(retirement);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn handoff_settlement_before_first_admission_leaves_no_renewable_position() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (queue, action) = publish(&chain, &pool, &slots);
    action.settle_engine();
    let settled = pool.counts();
    assert_eq!(
        action.borrow_handoff(16 * 1024).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(pool.counts(), settled);
    assert!(
        !chain
            .control
            .lock()
            .unwrap()
            .workflow
            .as_ref()
            .unwrap()
            .state
            .lock()
            .unwrap()
            .handoff_busy
    );
    chain.close();
    drop((chain, action, queue));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn revocation_alone_does_not_prove_original_variable_drainage() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots).unwrap();
    let held = chain
        .control
        .lock()
        .unwrap()
        .workflow
        .as_ref()
        .unwrap()
        .clone();
    // Private flag unit fixture. The consuming-loan test separately pins public provenance.
    let retirement = ConfirmationRetirement {
        binding: chain.binding.clone(),
    };
    assert!(!retirement.is_drained());
    chain.close();
    assert!(!chain.live.is_live());
    assert!(
        !retirement.is_drained(),
        "revocation is not actual owner drainage"
    );
    drop(held);
    assert!(retirement.is_drained());
    assert!(
        !chain.live.is_live(),
        "observation cannot revive plan authority"
    );
    drop((retirement, chain));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn returning_a_handoff_loan_does_not_retire_but_explicit_confirmation_retirement_does() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (queue, action) = publish(&chain, &pool, &slots);
    let before = pool.counts();
    drop(action.borrow_handoff(16 * 1024).unwrap());
    assert!(!chain.control.lock().unwrap().closed);
    assert_eq!(
        pool.counts(),
        before,
        "ordinary Drop returns bytes to the open96KiB reserve"
    );
    let loan = action.borrow_handoff(16 * 1024).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    action.settle_engine();
    loan.retire_confirmation();
    assert!(chain.control.lock().unwrap().closed);
    assert!(chain.prepare_derive().is_err());
    assert_eq!(
        action.borrow_handoff(1).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        action.admit_engine(action.engine_bytes()),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(pool.counts(), (METADATA_BYTES + ROW_BYTES, 0));
    let history = action.terminal_status();
    let next = prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
    let replacement = pool.counts();
    chain.close(); // delayed/replayed original closure cannot retire a replacement Workflow
    action.settle_engine();
    assert!(!next.control.lock().unwrap().closed);
    assert_eq!(action.terminal_status(), history);
    assert_eq!(pool.counts(), replacement);
    next.close();
    drop((next, chain, action, queue));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn explicit_confirmation_retirement_keeps_actual_input_and_lower_work_paid_until_their_last_drop() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (queue, action) = publish(&chain, &pool, &slots);
    let loan = action.borrow_handoff(16 * 1024).unwrap();
    let input = action.input().unwrap();
    let lower = loan
        .workflow
        .charge(&pool, chain.binding.scope.group, 512, ResourceKind::Forward)
        .unwrap();
    let original = Arc::downgrade(&loan.workflow);
    action.admit_engine(action.engine_bytes()).unwrap();
    action.settle_engine();
    loan.retire_confirmation();
    assert!(chain.control.lock().unwrap().closed);
    assert!(pool.counts().0 > METADATA_BYTES + ROW_BYTES);
    assert!(prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).is_err());
    drop(input);
    assert!(
        original.upgrade().is_some(),
        "actual lower work still owns the original workflow"
    );
    assert_eq!(pool.counts(), (METADATA_BYTES + ROW_BYTES + 512, 0));
    assert!(prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).is_err());
    drop(lower);
    assert!(original.upgrade().is_none());
    assert_eq!(pool.counts(), (METADATA_BYTES + ROW_BYTES, 0));
    let next = prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
    next.close();
    drop((next, original, chain, action, queue));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn retiring_the_original_loan_after_control_destruction_never_reopens_or_adopts_another_plan() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (queue, action) = publish(&chain, &pool, &slots);
    let loan = action.borrow_handoff(16 * 1024).unwrap();
    let original = Arc::downgrade(&loan.workflow);
    action.admit_engine(action.engine_bytes()).unwrap();
    action.settle_engine();
    drop(chain); // its real destructor closes the plan, while this loan retains actual work
    assert!(loan.workflow.control.upgrade().is_none());
    assert!(original.upgrade().is_some());
    loan.retire_confirmation();
    assert!(original.upgrade().is_none());
    assert_eq!(pool.counts(), (METADATA_BYTES + ROW_BYTES, 0));
    let next = prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
    assert!(!next.control.lock().unwrap().closed);
    next.close();
    drop((next, original, action, queue));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn original_confirmation_retirement_and_concurrent_close_release_only_actual_retired_owners() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (queue, action) = publish(&chain, &pool, &slots);
    let loan = action.borrow_handoff(16 * 1024).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    action.settle_engine();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let closing = scope.spawn(|| {
            barrier.wait();
            chain.close();
        });
        barrier.wait();
        loan.retire_confirmation();
        closing.join().unwrap();
    });
    assert!(chain.control.lock().unwrap().closed);
    assert_eq!(pool.counts(), (METADATA_BYTES + ROW_BYTES, 0));
    drop((chain, action, queue));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn handoff_uses_reserved_bytes_without_borrowing_a_second_crypto_lane() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let bound = binding(8);
    let chain = prepare_chain(pool.clone(), bound.clone(), slots.clone()).unwrap();
    let (_queue, action) = publish(&chain, &pool, &slots);
    let before = pool.counts();
    let pressure = pool
        .charge(
            bound.scope.group,
            256 * 1024 - before.0,
            ResourceKind::Retained,
        )
        .unwrap();
    let other_engines: Vec<_> = (0..3)
        .map(|_| {
            pool.charge(bound.scope.group, 0, ResourceKind::Crypto)
                .unwrap()
        })
        .collect();
    let full = pool.counts();
    let loan = action.borrow_handoff(16 * 1024).unwrap();
    assert_eq!(
        pool.counts(),
        full,
        "loan must use prepaid bytes and no extra Crypto slot"
    );
    assert_eq!(action.borrow_handoff(1).err(), Some(VoiceError::Capacity));
    assert_eq!(
        pool.counts(),
        full,
        "a duplicate cannot disturb the admitted loan"
    );
    action.admit_engine(action.engine_bytes()).unwrap();
    assert_eq!(
        pool.counts(),
        full,
        "the original protected engine remains usable"
    );
    action.settle_engine();
    assert_eq!(
        action.borrow_handoff(1).err(),
        Some(VoiceError::Unauthorized)
    );
    drop((loan, pressure, other_engines));
    chain.close();
    drop((chain, action, _queue));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn handoff_settlement_and_close_keep_actual_workflow_until_the_last_copy_owner_drops() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (queue, action) = publish(&chain, &pool, &slots);
    let descriptor = action.protected_binding().unwrap();
    let loan = Arc::new(action.borrow_handoff(16 * 1024).unwrap());
    let actual_reader = loan.clone();
    action.admit_engine(action.engine_bytes()).unwrap();
    action.settle_engine();
    chain.close();
    assert!(
        action.input().is_err(),
        "native payload drained independently of IPC work"
    );
    assert_eq!(pool.counts(), (METADATA_BYTES + ROW_BYTES + 16 * 1024, 0));
    assert!(prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).is_err());
    drop(loan);
    assert_eq!(pool.counts(), (METADATA_BYTES + ROW_BYTES + 16 * 1024, 0));
    drop(actual_reader);
    assert_eq!(pool.counts(), (METADATA_BYTES + ROW_BYTES, 0));
    let next = prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
    next.close();
    drop((next, chain));
    // The retained action's descriptor shares the original paid fixed binding, not Workflow.
    assert_eq!(action.protected_binding().unwrap(), descriptor);
    assert_eq!(pool.counts(), (METADATA_BYTES + ROW_BYTES, 0));
    drop((action, queue));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn failed_handoff_claims_do_not_spend_the_shared_position() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (_queue, action) = publish(&chain, &pool, &slots);
    for bytes in [0, VARIABLE_BYTES + 1] {
        assert_eq!(action.borrow_handoff(bytes).err(), Some(VoiceError::Bounds));
    }
    assert_eq!(
        action.borrow_handoff(VARIABLE_BYTES).err(),
        Some(VoiceError::Capacity)
    );
    let loan = action.borrow_handoff(16 * 1024).unwrap();
    drop(loan);
    let exact_retry = action.borrow_handoff(16 * 1024).unwrap();
    chain.close();
    assert_eq!(
        action.borrow_handoff(1).err(),
        Some(VoiceError::Unauthorized)
    );
    drop(exact_retry);
    drop((chain, action, _queue));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn close_racing_handoff_admission_never_reopens_the_chain() {
    for _ in 0..8 {
        let pool = Arc::new(Resources::default());
        let slots = Arc::new(slots::Slots::default());
        let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
        let (_queue, action) = publish(&chain, &pool, &slots);
        let barrier = std::sync::Barrier::new(2);
        let loan = std::thread::scope(|scope| {
            let admission = scope.spawn(|| {
                barrier.wait();
                action.borrow_handoff(16 * 1024)
            });
            barrier.wait();
            chain.close();
            admission.join().unwrap()
        });
        assert_eq!(
            action.borrow_handoff(1).err(),
            Some(VoiceError::Unauthorized)
        );
        assert_eq!(
            pool.counts().0,
            METADATA_BYTES + ROW_BYTES + if loan.is_ok() { 16 * 1024 } else { 0 }
        );
        drop(loan);
        drop((chain, action, _queue));
        assert_eq!(pool.counts(), (0, 0));
    }
}
