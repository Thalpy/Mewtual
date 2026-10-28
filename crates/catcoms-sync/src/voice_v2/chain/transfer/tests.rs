//! Capacity-only regressions. These never manufacture protocol readiness or SDP authority:
//! they exercise the original production owners under pressure and deterministic races.
use super::*;
use crate::voice_v2::chain::tests::{binding, prepare_chain, publish};

#[test]
fn original_affine_successor_prepays_ready_and_never_acquires_another_crypto_lane() {
    let (pool, chain, workflow, slots) = fixture();
    let control = workflow.confirmation_control();
    let before = pool.counts();
    let ticket = control
        .transfer_sdp_capacity(control.prepare_sdp_capacity().unwrap())
        .unwrap();
    let exchange = super::super::super::negotiation::Exchange::from_transferred(ticket, Vec::new());
    assert!(exchange.ready_prepaid_for_test());
    assert_eq!(pool.counts().1, before.1);
    let whole = control.retire_all();
    drop(workflow);
    assert!(!whole.is_drained());
    drop(exchange);
    assert!(whole.is_drained());
    drop((whole, chain));
    assert_eq!(pool.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn whole_successor_backing_is_prepaid_before_derivation_and_reused_under_pressure() {
    let (pool, chain, workflow, slots) = fixture();
    let control = workflow.confirmation_control();
    let group = chain.binding.scope.group;
    // Intentional contract change: ordinary pressure AFTER Derive admission cannot
    // remove the promised variable SDP/Ready backing. Refusal moves before exposure.
    let pressure = pool
        .charge(
            group,
            super::super::super::resources::GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = pool.counts();
    let prepared = control.prepare_sdp_capacity().unwrap();
    let core = Arc::as_ptr(&prepared.inner._reserve);
    let ready = Arc::as_ptr(&prepared.inner.ready);
    assert_eq!(pool.counts(), before, "no late ordinary224KiB allocation");
    drop(prepared);
    assert!(!chain.binding.successor_busy.load(Ordering::Acquire));
    let prepared = control.prepare_sdp_capacity().unwrap();
    assert_eq!(Arc::as_ptr(&prepared.inner._reserve), core);
    assert_eq!(Arc::as_ptr(&prepared.inner.ready), ready);
    assert_eq!(pool.counts(), before);
    let whole = control.retire_all();
    drop(workflow);
    assert!(!whole.is_drained());
    drop(prepared);
    assert!(whole.is_drained());
    drop((whole, chain, pressure));
    assert_eq!(pool.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn whole_retirement_after_ready_payment_refuses_original_slots_as_terminal_closure() {
    let (pool, chain, workflow, slots) = fixture();
    let left = workflow.confirmation_control();
    let right = workflow.confirmation_control();
    let mut whole = None;
    assert_eq!(
        left.prepare_sdp_capacity_before_slots(|| {
            whole = Some(right.retire_all());
            assert!(
                !whole.as_ref().unwrap().is_drained(),
                "actual preparation still owns bytes"
            );
        })
        .err(),
        Some(VoiceError::Unauthorized)
    );
    drop(workflow);
    assert!(whole.as_ref().unwrap().is_drained());
    drop((whole, left, chain));
    assert_eq!(pool.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn actual_engine_preflight_before_commit_prevents_transfer_and_late_old_lender_cannot_borrow() {
    let (resources, chain, workflow, slots) = fixture();
    let (queue, action) = publish(&chain, &resources, &slots);
    let control = workflow.confirmation_control();
    let mut prepared = Some(control.prepare_sdp_capacity().unwrap());
    // The production method has acquired its actual lender credit, but has not installed an
    // Engine permit. A callback deterministically parks precisely that admission boundary.
    assert_eq!(
        action.with_engine_admission(action.engine_bytes(), |_admission| {
            let (error, original) = control
                .transfer_sdp_capacity(prepared.take().unwrap())
                .unwrap_err();
            assert_eq!(error, VoiceError::Capacity);
            prepared = Some(original);
            Err::<(), _>(VoiceError::Capacity)
        }),
        Err(VoiceError::Capacity)
    );
    // This is the SAME lender clone a delayed engine preflight would have captured. Closing
    // WorkflowState under transfer must fence it before it can acquire the moved lane.
    let old_lender = ByteBudget::Workflow(workflow.clone());
    let ticket = control
        .transfer_sdp_capacity(prepared.take().unwrap())
        .unwrap();
    assert_eq!(
        old_lender
            .charge(
                &resources,
                chain.binding.scope.group,
                1,
                ResourceKind::Crypto
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        action.admit_engine(action.engine_bytes()).err(),
        Some(VoiceError::Unauthorized)
    );
    drop((old_lender, ticket, action, queue, control, workflow, chain));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

fn fixture() -> (
    Arc<Resources>,
    Arc<PreparedChain>,
    Arc<Workflow>,
    Arc<slots::Slots>,
) {
    let resources = Arc::new(Resources::default());
    // The original twelve claims occupy this entire table. A successor cannot obtain two
    // fresh ordinary positions, even briefly, while moving its protected continuation.
    let slots = Arc::new(slots::Slots::with_limit(ROWS));
    let chain = prepare_chain(resources.clone(), binding(1), slots.clone()).unwrap();
    let workflow = chain
        .control
        .lock()
        .unwrap()
        .workflow
        .as_ref()
        .unwrap()
        .clone();
    (resources, chain, workflow, slots)
}

#[test]
fn whole_retirement_waits_for_original_preparation_after_old_workflow_really_drains() {
    let (resources, chain, workflow, slots) = fixture();
    let control = workflow.confirmation_control();
    let prepared = control.prepare_sdp_capacity().unwrap();
    let old = ConfirmationRetirement {
        binding: chain.binding.clone(),
    };
    let whole = control.retire_all();
    drop(workflow);
    assert!(old.is_drained(), "old Workflow actually retired");
    assert!(
        !whole.is_drained(),
        "prepared capacity is a distinct actual owner"
    );
    assert!(chain.binding.successor_closed.load(Ordering::Acquire));
    drop(prepared);
    assert!(whole.is_drained());
    drop((old, whole, chain));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn extracted_ticket_survives_old_drain_and_cannot_publish_premature_whole_retirement() {
    let (resources, chain, workflow, slots) = fixture();
    let control = workflow.confirmation_control();
    let prepared = control.prepare_sdp_capacity().unwrap();
    let ticket = control.transfer_sdp_capacity(prepared).unwrap();
    let whole = control.retire_all();
    drop(workflow);
    assert!(ticket.confirmation_drained());
    assert!(
        !whole.is_drained(),
        "extracted lane/slots/reserve still exist before a constructor"
    );
    drop(ticket);
    assert!(whole.is_drained());
    drop((whole, chain));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn retained_exchange_and_its_real_last_owner_delay_whole_retirement_without_a_parent_cycle() {
    let (resources, chain, workflow, slots) = fixture();
    let control = workflow.confirmation_control();
    let ticket = control
        .transfer_sdp_capacity(control.prepare_sdp_capacity().unwrap())
        .unwrap();
    let exchange = super::super::super::negotiation::Exchange::from_transferred(ticket, Vec::new());
    let actual_last_owner = exchange.clone();
    let whole = control.retire_all();
    drop(workflow);
    exchange.close();
    assert!(!whole.is_drained());
    drop(exchange);
    assert!(!whole.is_drained());
    drop(actual_last_owner);
    assert!(whole.is_drained());
    drop((whole, chain));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn whole_close_before_exchange_construction_preserves_custody_but_refuses_successor_liveness() {
    let (resources, chain, workflow, slots) = fixture();
    let control = workflow.confirmation_control();
    let ticket = control
        .transfer_sdp_capacity(control.prepare_sdp_capacity().unwrap())
        .unwrap();
    let whole = control.retire_all();
    drop(workflow);
    let exchange = super::super::super::negotiation::Exchange::from_transferred(ticket, Vec::new());
    assert!(chain.binding.successor_closed.load(Ordering::Acquire));
    assert!(!whole.is_drained());
    assert!(exchange.collect_pending_successor(1));
    drop(exchange);
    assert!(whole.is_drained());
    drop((whole, chain));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn failed_preparation_releases_its_guard_and_unpublished_abandonment_allows_same_plan_retry() {
    let (resources, chain, workflow, slots) = fixture();
    let control = workflow.confirmation_control();
    let group = chain.binding.scope.group;
    // A real old-phase loan can occupy its original96KiB. The new metadata loan
    // must refuse locally without closing/refunding that debt or latching preparation.
    let pressure = workflow
        .charge(&resources, group, VARIABLE_BYTES, ResourceKind::Retained)
        .unwrap();
    assert_eq!(
        control.prepare_sdp_capacity().err(),
        Some(VoiceError::Capacity)
    );
    assert!(!chain.binding.successor_busy.load(Ordering::Acquire));
    drop(pressure);
    let prepared = control.prepare_sdp_capacity().unwrap();
    drop(prepared);
    assert!(!chain.binding.successor_busy.load(Ordering::Acquire));
    let prepared = control.prepare_sdp_capacity().unwrap();
    let whole = control.retire_all();
    drop(workflow);
    assert!(!whole.is_drained());
    drop(prepared);
    assert!(whole.is_drained());
    drop((whole, chain));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn partial_preparation_failure_after_payment_returns_all_credits_before_releasing_its_guard() {
    let (resources, chain, workflow, slots) = fixture();
    let control = workflow.confirmation_control();
    // Actual original row loans leave only one of twelve slots. Reservation and metadata
    // succeed first; the later two-slot child subreserve must then refuse without leaking
    // credits or leaving sdp_preparing/busy latched. No slot limit is reduced for the test.
    let held: Vec<_> = (0..ROWS - 1)
        .map(|_| chain.history.slots.loan(&slots).unwrap())
        .collect();
    let before = resources.counts();
    assert_eq!(
        control.prepare_sdp_capacity().err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(resources.counts(), before);
    assert!(!chain.binding.successor_busy.load(Ordering::Acquire));
    assert!(!workflow.state.lock().unwrap().sdp_preparing);
    drop(held);
    let prepared = control.prepare_sdp_capacity().unwrap();
    let whole = control.retire_all();
    drop(workflow);
    assert!(!whole.is_drained());
    drop(prepared);
    assert!(whole.is_drained());
    drop((whole, chain));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn whole_retirement_winning_first_never_allows_a_new_preparation_or_capacity_charge() {
    let (resources, chain, workflow, slots) = fixture();
    let left = workflow.confirmation_control();
    let right = workflow.confirmation_control();
    let whole = left.retire_all();
    let before = resources.counts();
    assert_eq!(
        right.prepare_sdp_capacity().err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(resources.counts(), before);
    drop(workflow);
    assert!(whole.is_drained());
    drop((right, whole, chain));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn simultaneous_whole_retirement_and_real_preparation_never_outpace_actual_custody() {
    let (resources, chain, workflow, slots) = fixture();
    let left = workflow.confirmation_control();
    let right = workflow.confirmation_control();
    let barrier = std::sync::Barrier::new(2);
    let (preparation, whole) = std::thread::scope(|scope| {
        let prepare = scope.spawn(|| {
            barrier.wait();
            left.prepare_sdp_capacity()
        });
        barrier.wait();
        let whole = right.retire_all();
        (prepare.join().unwrap(), whole)
    });
    drop(workflow);
    match preparation {
        Ok(prepared) => {
            assert!(!whole.is_drained());
            drop(prepared);
        }
        Err(error) => assert_eq!(error, VoiceError::Unauthorized),
    }
    assert!(whole.is_drained());
    drop((whole, left, chain));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn whole_close_between_open_observation_and_guard_cas_cannot_overtake_original_control() {
    let (pool, chain, workflow, slots) = fixture();
    let right = workflow.confirmation_control();
    let (observed, wait_observed) = std::sync::mpsc::channel();
    let (closed, wait_closed) = std::sync::mpsc::channel();
    let (custody, whole) = std::thread::scope(|scope| {
        let closing = scope.spawn(move || {
            wait_observed.recv().unwrap();
            right.retire_all_after_close(|| closed.send(()).unwrap())
        });
        // Actual preparation's Control -> Workflow order, before its first payment. The
        // closer stores closed=true, then must block at THIS same original Control fence.
        let control = chain.control.lock().unwrap();
        let state = workflow.state.lock().unwrap();
        let custody = SuccessorCustody::begin_after_open_check(&chain.binding, || {
            observed.send(()).unwrap();
            wait_closed.recv().unwrap();
        })
        .unwrap();
        assert!(chain.binding.successor_closed.load(Ordering::Acquire));
        assert!(chain.binding.successor_busy.load(Ordering::Acquire));
        assert!(!chain.binding.workflow_drained.load(Ordering::Acquire));
        drop(state);
        drop(control);
        (custody, closing.join().unwrap())
    });
    drop(workflow);
    assert!(chain.binding.workflow_drained.load(Ordering::Acquire));
    assert!(
        !whole.is_drained(),
        "the late published guard still owns original preparation"
    );
    drop(custody);
    assert!(whole.is_drained());
    drop((whole, chain));
    assert_eq!(pool.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn successor_moves_original_crypto_and_slots_under_full_ordinary_pressure() {
    let (resources, chain, workflow, slots) = fixture();
    let control = workflow.confirmation_control();
    let prepared = control.prepare_sdp_capacity().unwrap();
    assert!(slots.charge(1).is_err());
    let group = chain.binding.scope.group;
    // The original lane is one of four group Crypto positions. Preparation creates NONE.
    let busy: Vec<_> = (0..3)
        .map(|_| resources.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    assert!(resources.charge(group, 0, ResourceKind::Crypto).is_err());
    let fill = resources
        .charge(
            group,
            super::super::super::resources::GROUP_BYTES - resources.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let ticket = control.transfer_sdp_capacity(prepared).unwrap();
    assert!(!workflow.is_live());
    // Old history released its ten unused positions. Fill them, then consume BOTH original
    // successor positions; no release/recharge window, larger pool or new reservation exists.
    let ordinary = slots.charge(ROWS - 2).unwrap();
    let first = ticket
        ._prepared
        .as_ref()
        .unwrap()
        .inner
        ._slots
        .loan(&slots)
        .unwrap();
    let second = ticket
        ._prepared
        .as_ref()
        .unwrap()
        .inner
        ._slots
        .loan(&slots)
        .unwrap();
    assert!(slots.charge(1).is_err());
    let budget = ByteBudget::ReservedLane {
        reservation: ticket._prepared.as_ref().unwrap().inner._reserve.clone(),
        lane: ticket.lane.as_ref().unwrap().clone(),
    };
    let actual = budget
        .charge(&resources, group, 1, ResourceKind::Crypto)
        .unwrap();
    assert!(budget
        .charge(&resources, group, 1, ResourceKind::Crypto)
        .is_err());
    drop(actual);
    // Every old close/expiry path sees None. It cannot close the moved lane through an old
    // Workflow handle, even though its first confirmation deadline has elapsed.
    chain.close();
    workflow.collect_expired(u64::MAX);
    drop(
        budget
            .charge(&resources, group, 1, ResourceKind::Crypto)
            .unwrap(),
    );
    drop(workflow);
    assert!(ticket.confirmation_drained());
    drop((
        budget, ticket, control, chain, first, second, ordinary, fill, busy,
    ));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn actual_engine_and_handoff_refusals_preserve_same_preparation_for_retry() {
    let (resources, chain, workflow, slots) = fixture();
    let control = workflow.confirmation_control();
    let prepared = control.prepare_sdp_capacity().unwrap();
    let original = &*prepared.inner as *const PreparedCapacity;
    let held = workflow
        .charge(
            &resources,
            chain.binding.scope.group,
            1,
            ResourceKind::Crypto,
        )
        .unwrap();
    let (error, prepared) = control.transfer_sdp_capacity(prepared).unwrap_err();
    assert_eq!(error, VoiceError::Capacity);
    assert_eq!(&*prepared.inner as *const PreparedCapacity, original);
    assert!(workflow.is_live());
    drop(held);
    let held = workflow.borrow_handoff(2048).unwrap();
    let (error, prepared) = control.transfer_sdp_capacity(prepared).unwrap_err();
    assert_eq!(error, VoiceError::Capacity);
    assert_eq!(&*prepared.inner as *const PreparedCapacity, original);
    drop(held);
    let transferred = control.transfer_sdp_capacity(prepared).unwrap();
    assert_eq!(
        &*transferred._prepared.as_ref().unwrap().inner as *const PreparedCapacity,
        original
    );
    drop((transferred, control, workflow, chain));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn old_real_lower_and_last_handoff_credit_keep_retirement_false_after_lane_transfer() {
    let (resources, chain, workflow, _) = fixture();
    let control = workflow.confirmation_control();
    let prepared = control.prepare_sdp_capacity().unwrap();
    let lower = workflow
        .charge(
            &resources,
            chain.binding.scope.group,
            1,
            ResourceKind::Forward,
        )
        .unwrap();
    let mut handoff = workflow.borrow_handoff(2048).unwrap();
    // Deterministically park Drop AFTER the original busy flag clears but BEFORE the actual
    // credited input owner drops. This is the real destructor ordering, without sleeps.
    handoff.workflow.state.lock().unwrap().handoff_busy = false;
    let transferred = control.transfer_sdp_capacity(prepared).unwrap();
    assert!(!transferred.confirmation_drained());
    let witness = ConfirmationRetirement {
        binding: transferred.retirement.binding.clone(),
    };
    drop((transferred, control, workflow, chain));
    assert!(!witness.is_drained());
    drop(lower);
    assert!(!witness.is_drained());
    // Replace the credited field by a separately paid one solely to park the ACTUAL original
    // credit until after HandoffLoan's destructor. It must remain the final Workflow owner.
    let actual_credit = std::mem::replace(
        &mut handoff._credit,
        resources
            .charge(
                handoff.workflow.binding.scope.group,
                1,
                ResourceKind::Retained,
            )
            .unwrap(),
    );
    // Restore only Drop's private debug precondition on this ALREADY CLOSED Workflow.
    // It confers no authority; the destructor immediately clears it again. The original
    // credit remains parked independently to test the last-owner publication ordering.
    handoff.workflow.state.lock().unwrap().handoff_busy = true;
    drop(handoff);
    assert!(!witness.is_drained());
    drop(actual_credit);
    assert!(witness.is_drained());
    drop(witness);
    assert_eq!(resources.counts(), (0, 0));
}

#[test]
fn equal_foreign_plan_never_adopts_capacity_and_refusal_returns_its_original_owner() {
    let (resources, chain, workflow, _) = fixture();
    let foreign_resources = Arc::new(Resources::default());
    let foreign_chain = prepare_chain(
        foreign_resources.clone(),
        chain.binding.value.clone(),
        Arc::new(slots::Slots::default()),
    )
    .unwrap();
    let foreign_workflow = foreign_chain
        .control
        .lock()
        .unwrap()
        .workflow
        .as_ref()
        .unwrap()
        .clone();
    let foreign_control = foreign_workflow.confirmation_control();
    let prepared = foreign_control.prepare_sdp_capacity().unwrap();
    let original = &*prepared.inner as *const PreparedCapacity;
    let control = workflow.confirmation_control();
    let (error, prepared) = control.transfer_sdp_capacity(prepared).unwrap_err();
    assert_eq!(error, VoiceError::Scope);
    assert_eq!(&*prepared.inner as *const PreparedCapacity, original);
    assert!(workflow.is_live());
    let ticket = foreign_control.transfer_sdp_capacity(prepared).unwrap();
    drop((
        ticket,
        control,
        workflow,
        chain,
        foreign_control,
        foreign_workflow,
        foreign_chain,
    ));
    assert_eq!(resources.counts(), (0, 0));
    assert_eq!(foreign_resources.counts(), (0, 0));
}

#[test]
fn competing_controls_cannot_prepare_twice_and_close_racing_transfer_never_reopens() {
    let (resources, chain, workflow, slots) = fixture();
    let left = workflow.confirmation_control();
    let right = workflow.confirmation_control();
    let left_prepared = left.prepare_sdp_capacity().unwrap();
    let before = resources.counts();
    assert_eq!(
        right.prepare_sdp_capacity().err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(resources.counts(), before);
    let barrier = std::sync::Barrier::new(2);
    let (left_result, right_result) = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            barrier.wait();
            left.transfer_sdp_capacity(left_prepared)
        });
        barrier.wait();
        let right = right.retire();
        (worker.join().unwrap(), right)
    });
    if let Err((error, _)) = &left_result {
        assert_eq!(*error, VoiceError::Unauthorized);
    }
    drop((left_result, right_result, left));
    assert!(workflow
        .confirmation_control()
        .prepare_sdp_capacity()
        .is_err());
    assert_eq!(
        workflow
            .charge(
                &resources,
                chain.binding.scope.group,
                1,
                ResourceKind::Crypto
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    drop((workflow, chain));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn failed_preparation_and_abandonment_preserve_original_confirmation() {
    let (resources, chain, workflow, slots) = fixture();
    let control = workflow.confirmation_control();
    let group = chain.binding.scope.group;
    // Original variable debt, rather than late ordinary headroom, can still refuse
    // the preparation metadata. Keep the original loan until that local refusal returns.
    let fill = workflow
        .charge(&resources, group, VARIABLE_BYTES, ResourceKind::Retained)
        .unwrap();
    let full = resources.counts();
    assert_eq!(
        control.prepare_sdp_capacity().err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(resources.counts(), full);
    drop(fill);
    let before = resources.counts();
    drop(control.prepare_sdp_capacity().unwrap());
    assert_eq!(resources.counts(), before);
    // Abandonment returns the unique preparation latch as well as the original two claims.
    drop(control.prepare_sdp_capacity().unwrap());
    assert_eq!(resources.counts(), before);
    assert!(slots.charge(1).is_err());
    drop(
        workflow
            .charge(&resources, group, 1, ResourceKind::Crypto)
            .unwrap(),
    );
    chain.binding.call.revoke();
    assert_eq!(
        control.prepare_sdp_capacity().err(),
        Some(VoiceError::Unauthorized)
    );
    drop((control, workflow, chain));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn successor_metadata_covers_box_child_reserve_and_affine_ticket_layouts() {
    use std::mem::size_of;
    let bytes = size_of::<PreparedCapacity>()
        + size_of::<PreparedSdpCapacity>()
        + size_of::<TransferredSdpCapacity>()
        + size_of::<slots::ReservedSlots>()
        + size_of::<Reservation>()
        + size_of::<ProgressFunding>()
        + 2 * size_of::<usize>() // shared enclosing metadata funding allocation
        + 3 * 2 * size_of::<usize>();
    assert!(
        bytes < super::super::super::negotiation::METADATA_BYTES,
        "successor metadata graph grew to {bytes}"
    );
}

#[test]
fn successor_inert_exchange_consumes_exact_original_reserve_lane_slots_and_lower_owner() {
    let (resources, chain, workflow, slots) = fixture();
    let control = workflow.confirmation_control();
    let prepared = control.prepare_sdp_capacity().unwrap();
    let expected = [
        Arc::as_ptr(&prepared.inner._reserve) as usize,
        Arc::as_ptr(workflow.state.lock().unwrap().crypto.as_ref().unwrap()) as usize,
        Arc::as_ptr(&prepared.inner._slots) as usize,
        Arc::as_ptr(&prepared.inner._lowers) as usize,
    ];
    let group = chain.binding.scope.group;
    let busy: Vec<_> = (0..3)
        .map(|_| resources.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    let fill = resources
        .charge(
            group,
            super::super::super::resources::GROUP_BYTES - resources.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let ticket = control.transfer_sdp_capacity(prepared).unwrap();
    let exchange = super::super::super::negotiation::Exchange::from_transferred(ticket, Vec::new());
    assert_eq!(exchange.capacity_identities_for_test(), expected);
    assert!(!exchange.is_live(), "capacity movement is never activation");
    assert!(resources.charge(group, 0, ResourceKind::Crypto).is_err());
    let other_slots = slots.charge(ROWS - 2).unwrap();
    assert!(
        slots.charge(1).is_err(),
        "two original positions remain protected"
    );
    exchange.close();
    let released = slots.charge(2).unwrap();
    drop((
        released,
        other_slots,
        exchange,
        control,
        workflow,
        chain,
        fill,
        busy,
    ));
    assert_eq!(resources.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn shared_core_does_not_expand_original_confirmation_allowance() {
    let (pool, chain, workflow, slots) = fixture();
    let group = chain.binding.scope.group;
    let before = pool.counts();
    let loan = workflow
        .charge(&pool, group, VARIABLE_BYTES, ResourceKind::Retained)
        .unwrap();
    assert_eq!(pool.counts(), before);
    assert_eq!(
        workflow
            .charge(&pool, group, 1, ResourceKind::Retained)
            .err(),
        Some(VoiceError::Capacity)
    );
    drop(loan);
    let again = workflow
        .charge(&pool, group, VARIABLE_BYTES, ResourceKind::Retained)
        .unwrap();
    drop((again, workflow, chain));
    assert_eq!(pool.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn whole_backing_refusal_precedes_first_chain_exposure_and_is_atomic() {
    let pool = Arc::new(Resources::default());
    let bound = binding(1);
    let group = bound.scope.group;
    let slots = Arc::new(slots::Slots::default());
    // Leave less than the whole variable requirement, independently of fixed history.
    let pressure = pool
        .charge(
            group,
            super::super::super::resources::GROUP_BYTES
                - super::super::super::negotiation::EXCHANGE_BYTES
                - super::super::super::ready::capacity::BYTES
                + 1,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = pool.counts();
    assert_eq!(
        prepare_chain(pool.clone(), bound, slots.clone()).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(
        pool.counts(),
        before,
        "no partially exposed chain or retained backing"
    );
    drop(slots.charge(32).unwrap());
    drop(pressure);
    let chain = prepare_chain(pool.clone(), binding(1), slots.clone()).unwrap();
    drop(chain);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn old_repeated_close_cannot_close_transferred_backing_and_abandoned_ticket_closes_it() {
    let (pool, chain, workflow, slots) = fixture();
    let group = chain.binding.scope.group;
    let control = workflow.confirmation_control();
    let lower = workflow
        .charge(&pool, group, 4096, ResourceKind::Forward)
        .unwrap();
    let prepared = control.prepare_sdp_capacity().unwrap();
    let core = prepared.inner._reserve.clone();
    let ready = prepared.inner.ready.clone();
    let ticket = control.transfer_sdp_capacity(prepared).unwrap();
    let old = ConfirmationRetirement {
        binding: chain.binding.clone(),
    };
    workflow.close();
    workflow.close();
    let probe = ByteBudget::Reserved(core.clone())
        .charge(&pool, group, 1, ResourceKind::Retained)
        .unwrap();
    drop(probe);
    assert!(
        !old.is_drained(),
        "actual original Lower and observer remain"
    );
    drop(ticket); // No Exchange constructor: the affine ticket owns both close rights.
    assert_eq!(
        ByteBudget::Reserved(core.clone())
            .charge(&pool, group, 1, ResourceKind::Retained)
            .err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(
        ByteBudget::Reserved(ready.reserve.clone())
            .charge(&pool, group, 1, ResourceKind::Retained)
            .err(),
        Some(VoiceError::Capacity)
    );
    drop(workflow);
    assert!(
        !old.is_drained(),
        "abandonment does not refund actual Lower custody"
    );
    drop(lower);
    assert!(old.is_drained());
    drop((control, old, ready, core, chain));
    assert_eq!(pool.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}

#[test]
fn ordinary_progress_close_keeps_ready_metadata_through_actual_escrow_destruction() {
    #[derive(Debug)]
    struct EscrowDrop {
        ready: Weak<super::super::super::ready::capacity::ReadyCapacity>,
        observed: Arc<AtomicBool>,
    }
    impl Drop for EscrowDrop {
        fn drop(&mut self) {
            // Nonpanicking observer of ACTUAL Credit custody destruction, after its
            // reservation return. Holding no strong Ready/Workflow cannot mask early fee return.
            self.observed
                .store(self.ready.upgrade().is_some(), Ordering::Release);
        }
    }
    let (pool, chain, workflow, slots) = fixture();
    let progress = workflow.state.lock().unwrap().progress.take().unwrap();
    let ProgressCapacity { closing, escrow } = progress.into_parts();
    let ready = Arc::downgrade(&closing.ready);
    let observed = Arc::new(AtomicBool::new(false));
    let escrow = escrow.with_custody(Arc::new(EscrowDrop {
        ready: ready.clone(),
        observed: observed.clone(),
    })); // Test observer only: production escrow NEVER retains Workflow custody.
    workflow.state.lock().unwrap().progress = Some(ProgressCapacityOwner::new(closing, escrow));
    workflow.close();
    assert!(
        observed.load(Ordering::Acquire),
        "fee must survive actual escrow last Drop"
    );
    assert!(
        ready.upgrade().is_none(),
        "outer anchor returns only after body deallocation"
    );
    drop((ready, workflow, chain));
    assert_eq!(pool.counts(), (0, 0));
    drop(slots.charge(ROWS).unwrap());
}
