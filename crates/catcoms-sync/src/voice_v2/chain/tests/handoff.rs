//! Application-copy custody shares the existing protected envelope and survives End.
use super::*;

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
