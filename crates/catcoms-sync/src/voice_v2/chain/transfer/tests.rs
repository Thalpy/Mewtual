//! Capacity-only regressions. These never manufacture protocol readiness or SDP authority:
//! they exercise the original production owners under pressure and deterministic races.
use super::*;
use crate::voice_v2::chain::tests::{binding, prepare_chain, publish};

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
    let first = ticket._prepared.inner._slots.loan(&slots).unwrap();
    let second = ticket._prepared.inner._slots.loan(&slots).unwrap();
    assert!(slots.charge(1).is_err());
    let budget = ByteBudget::ReservedLane {
        reservation: ticket._prepared.inner._reserve.clone(),
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
        &*transferred._prepared.inner as *const PreparedCapacity,
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
    let fill = resources
        .charge(
            group,
            super::super::super::resources::GROUP_BYTES - resources.counts().0,
            ResourceKind::Retained,
        )
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
        + 3 * 2 * size_of::<usize>();
    assert!(
        bytes < super::super::super::negotiation::METADATA_BYTES,
        "successor metadata graph grew to {bytes}"
    );
}
