//! The one group progression slot follows actual old and successor work, not either
//! phase's weak lookup entry. These tests use the production factories and real credits.
use super::*;
use crate::voice_v2::negotiation::{Exchange, Stage};
use std::sync::Barrier;

fn parked_drop<T: Send + 'static>(
    owner: T,
    pool: &Resources,
    entered: &Barrier,
    resume: &Barrier,
    bytes: usize,
) {
    let worker = std::thread::spawn(move || drop(owner));
    entered.wait();
    // Always release and join before asserting, so even a failed fee regression cannot
    // leave its genuine final publisher parked in a detached thread.
    let actual = pool.counts();
    resume.wait();
    worker.join().unwrap();
    assert_eq!(
        actual,
        (bytes, 0),
        "each enclosing fee stays paid through actual claim release"
    );
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn original_workflow_payment_survives_actual_blocked_last_publisher() {
    let pool = Arc::new(Resources::default());
    let first = chain(&pool, 1).unwrap();
    let workflow = first.workflow().unwrap();
    first.close();
    drop(first);
    let mut workflow = Arc::try_unwrap(workflow)
        .ok()
        .expect("actual sole Workflow");
    let (entered, resume) = (Arc::new(Barrier::new(2)), Arc::new(Barrier::new(2)));
    workflow
        ._retirement
        .as_mut()
        .unwrap()
        .progress
        .park_release_for_test(entered.clone(), resume.clone());
    parked_drop(
        workflow,
        &pool,
        &entered,
        &resume,
        METADATA_BYTES + crate::voice_v2::ready::capacity::METADATA,
    );
}

#[test]
fn prepared_box_payment_survives_actual_blocked_claim_and_box_destruction() {
    let pool = Arc::new(Resources::default());
    let first = chain(&pool, 1).unwrap();
    let workflow = first.workflow().unwrap();
    let control = workflow.confirmation_control();
    let mut prepared = control.prepare_sdp_capacity().unwrap();
    drop(control.retire_all());
    drop((workflow, first));
    let (entered, resume) = (Arc::new(Barrier::new(2)), Arc::new(Barrier::new(2)));
    prepared.park_progress_release_for_test(entered.clone(), resume.clone());
    parked_drop(
        prepared,
        &pool,
        &entered,
        &resume,
        crate::voice_v2::negotiation::METADATA_BYTES + crate::voice_v2::ready::capacity::METADATA,
    );
}

#[test]
fn protected_exchange_own_payment_survives_actual_blocked_final_claim() {
    let pool = Arc::new(Resources::default());
    let first = chain(&pool, 1).unwrap();
    let workflow = first.workflow().unwrap();
    let control = workflow.confirmation_control();
    let ticket = control
        .transfer_sdp_capacity(control.prepare_sdp_capacity().unwrap())
        .unwrap();
    let successor = Exchange::from_transferred(ticket, Vec::new());
    drop(control.retire_all());
    drop((workflow, first));
    let mut successor = Arc::try_unwrap(successor)
        .ok()
        .expect("actual sole successor");
    let (entered, resume) = (Arc::new(Barrier::new(2)), Arc::new(Barrier::new(2)));
    successor.park_progress_release_for_test(entered.clone(), resume.clone());
    parked_drop(
        successor,
        &pool,
        &entered,
        &resume,
        crate::voice_v2::negotiation::METADATA_BYTES + crate::voice_v2::ready::capacity::METADATA,
    );
}

#[test]
fn standalone_exchange_own_payment_survives_actual_blocked_final_claim() {
    let pool = Arc::new(Resources::default());
    let mut successor = Arc::try_unwrap(exchange(&pool).unwrap())
        .ok()
        .expect("actual sole standalone Exchange");
    let (entered, resume) = (Arc::new(Barrier::new(2)), Arc::new(Barrier::new(2)));
    successor.park_progress_release_for_test(entered.clone(), resume.clone());
    parked_drop(
        successor,
        &pool,
        &entered,
        &resume,
        crate::voice_v2::negotiation::METADATA_BYTES,
    );
}

fn chain(pool: &Arc<Resources>, seed: u8) -> Result<Arc<PreparedChain>, VoiceError> {
    prepare_chain(
        pool.clone(),
        binding(seed),
        Arc::new(slots::Slots::default()),
    )
}
fn exchange(pool: &Arc<Resources>) -> Result<Arc<Exchange>, VoiceError> {
    Exchange::new_for_test(
        pool.clone(),
        binding(1).scope.group,
        Arc::new(slots::Slots::default()),
    )
}

#[test]
fn standalone_sdp_and_confirmation_cannot_overlap_in_either_order() {
    let pool = Arc::new(Resources::default());
    let first = exchange(&pool).unwrap();
    let before = pool.counts();
    assert_eq!(chain(&pool, 1).err(), Some(VoiceError::Capacity));
    assert_eq!(
        pool.counts(),
        before,
        "losing construction returns every owner"
    );
    first.close();
    assert_eq!(
        chain(&pool, 2).err(),
        Some(VoiceError::Capacity),
        "close is not actual return"
    );
    drop(first);
    let first = chain(&pool, 3).unwrap();
    let before = pool.counts();
    assert_eq!(exchange(&pool).err(), Some(VoiceError::Capacity));
    assert_eq!(pool.counts(), before);
    drop(first);
    drop(exchange(&pool).unwrap());
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn closed_standalone_actual_lower_blocks_confirmation_until_its_last_drop() {
    let pool = Arc::new(Resources::default());
    let first = exchange(&pool).unwrap();
    let lower = first.charge(Stage::Lower, 512).unwrap();
    first.close();
    drop(first);
    assert_eq!(chain(&pool, 1).err(), Some(VoiceError::Capacity));
    drop(lower);
    drop(chain(&pool, 2).unwrap());
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn transferred_successor_keeps_group_slot_after_old_workflow_dies() {
    let pool = Arc::new(Resources::default());
    let first = chain(&pool, 1).unwrap();
    let workflow = first.workflow().unwrap();
    let control = workflow.confirmation_control();
    let prepared = control.prepare_sdp_capacity().unwrap();
    let ticket = control.transfer_sdp_capacity(prepared).unwrap();
    let whole = control.retire_all();
    drop(workflow);
    assert!(first.binding.workflow_drained.load(Ordering::Acquire));
    assert_eq!(chain(&pool, 2).err(), Some(VoiceError::Capacity));
    assert_eq!(exchange(&pool).err(), Some(VoiceError::Capacity));
    drop(ticket);
    assert!(whole.is_drained());
    drop((first, whole));
    drop(chain(&pool, 3).unwrap());
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn unextracted_preparation_keeps_group_slot_independently_of_old_workflow() {
    let pool = Arc::new(Resources::default());
    let first = chain(&pool, 1).unwrap();
    let workflow = first.workflow().unwrap();
    let control = workflow.confirmation_control();
    let prepared = control.prepare_sdp_capacity().unwrap();
    let whole = control.retire_all();
    drop(workflow);
    assert!(first.binding.workflow_drained.load(Ordering::Acquire));
    assert_eq!(chain(&pool, 2).err(), Some(VoiceError::Capacity));
    assert_eq!(exchange(&pool).err(), Some(VoiceError::Capacity));
    drop(prepared);
    assert!(whole.is_drained());
    drop((first, whole));
    drop(chain(&pool, 3).unwrap());
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn both_old_actual_lower_and_successor_must_return_in_either_order() {
    for old_first in [false, true] {
        let pool = Arc::new(Resources::default());
        let first = chain(&pool, 1).unwrap();
        let workflow = first.workflow().unwrap();
        let lower = workflow
            .charge(&pool, first.binding.scope.group, 512, ResourceKind::Forward)
            .unwrap();
        let control = workflow.confirmation_control();
        let ticket = control
            .transfer_sdp_capacity(control.prepare_sdp_capacity().unwrap())
            .unwrap();
        let successor = Exchange::from_transferred(ticket, Vec::new());
        let whole = control.retire_all();
        successor.close();
        drop(workflow);
        if old_first {
            drop(lower);
            assert_eq!(chain(&pool, 2).err(), Some(VoiceError::Capacity));
            drop(successor);
        } else {
            drop(successor);
            assert_eq!(chain(&pool, 2).err(), Some(VoiceError::Capacity));
            drop(lower);
        }
        assert!(whole.is_drained());
        drop((whole, first));
        drop(chain(&pool, 3).unwrap());
        assert_eq!(pool.counts(), (0, 0));
    }
}
