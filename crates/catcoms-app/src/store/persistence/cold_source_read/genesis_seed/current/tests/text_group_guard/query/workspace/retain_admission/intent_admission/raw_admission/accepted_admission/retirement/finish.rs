//! Genuine WC -> actual removed ordinary lane -> DISTINCT fresh finish guard.
//! No fabricated Saved, missing-entry receipt or caller work count appears here.
use super::worker::{
    completion_admission, deletion_admission, worker_retirement_fixture, WorkerRetirementFixture,
};
use super::*;
use crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test;
use crate::store::owner_evidence::canonical::text_lane::{
    active_text_lane_count_for_test, RetiredV2TextCreateLane,
};

/// Actual owning slots mirror the consuming production boundary. The old native
/// owner remains whole; no fake replacement lane or SDK/model extractor is needed.
pub(super) struct LaneFinishFixture {
    pub(super) accepted: Option<Box<PreparedV2TextAcceptedCreate>>,
    pub(super) predecessor: BoundV2TextCreatePredecessor,
    pub(super) original: TextFixture,
    pub(super) model_dropped: Arc<AtomicBool>,
    pub(super) sdk_dropped: Arc<AtomicBool>,
    pub(super) basis: V2AdvancedAppendBasis,
    pub(super) lane: Option<V2TextCreateLane>,
    pub(super) drained: Option<V2TextRetirementRootsDrained>,
    pub(super) retired: Option<RetiredV2TextCreateLane>,
}

fn into_lane_fixture(t: WorkerRetirementFixture) -> LaneFinishFixture {
    let WorkerRetirementFixture { f, basis, drained } = t;
    let AcceptedFixture {
        accepted,
        lane,
        predecessor,
        model_dropped,
        sdk_dropped,
        original,
    } = f;
    LaneFinishFixture {
        accepted: Some(accepted),
        predecessor,
        original,
        model_dropped,
        sdk_dropped,
        basis,
        lane: Some(lane),
        drained: Some(drained),
        retired: None,
    }
}

fn actual_completed_fixture(family: usize, number: u64) -> LaneFinishFixture {
    let mut t = worker_retirement_fixture(family, number);
    let marker = deletion_admission(&t).unwrap();
    t.f.accepted.run_retirement_deletion(marker).unwrap();
    let marker = completion_admission(&t).unwrap();
    t.f.accepted.run_retirement_completion(marker).unwrap();
    // The flat resident parent is copied/paid OUTSIDE Current before any finish
    // proof. A guard never performs that work or infers readiness from SavedWC.
    t.f.accepted.prepare_completed_resident_data().unwrap();
    into_lane_fixture(t)
}

pub(super) fn retire_lane(t: &mut LaneFinishFixture) -> Result<(), AppError> {
    let store = t.original.layout.checked_store()?;
    let owner = t.original.owner.borrow_current()?;
    let accepted = t
        .accepted
        .as_ref()
        .ok_or_else(|| AppError::Invalid("missing actual B".into()))?;
    let resources = accepted.resources().clone();
    store.with_v2_text_current(&owner, |admission| {
        let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
        guard.retire_completed_text_lane_into(
            accepted,
            &t.basis,
            &mut t.lane,
            &mut t.drained,
            &mut t.retired,
        )
    })
}

/// Reused by the actual resident test child. All phase transitions are genuine:
/// B/Core-final -> authoritative close/drain/detach -> distinct WD/WC -> lane take.
pub(super) fn completed_lane_fixture(family: usize, number: u64) -> LaneFinishFixture {
    let mut t = actual_completed_fixture(family, number);
    retire_lane(&mut t).unwrap();
    assert!(t.lane.is_none() && t.drained.is_none() && t.retired.is_some());
    t
}

pub(super) fn validate_finish(t: &LaneFinishFixture) -> Result<(), AppError> {
    let accepted = t
        .accepted
        .as_ref()
        .ok_or_else(|| AppError::Invalid("missing actual B".into()))?;
    // Genuine receipt Arc is cloned OUTSIDE Current; PreparedB's temporary borrow
    // never supplies guard lifetime or blocks a later mutable owning cut.
    let saved = accepted.saved_retirement_completion()?.clone();
    let store = t.original.layout.checked_store()?;
    let owner = t.original.owner.borrow_current()?;
    let resources = accepted.resources().clone();
    store.with_v2_text_current(&owner, |admission| {
        let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
        let finish = guard.admit_completed_finish(t.retired.as_ref().unwrap(), &saved, accepted)?;
        finish.require_original_accepted(&accepted.observe_original())?;
        finish.require_original_unsigned(&accepted.original_unsigned())?;
        finish.require_original_retain(&accepted.original_retain())?;
        finish.require_original_completion(&saved)?;
        finish.require_completion_frame_id_data(&saved.completion_frame_id_data())?;
        finish.validate_negative()
    })
}

#[test]
fn actual_wc_retires_each_original_lane_once_and_fresh_finish_preserves_files_and_paid_owners() {
    let mut previous: Option<LaneFinishFixture> = None;
    for family in 0..3 {
        let mut t = actual_completed_fixture(family, 0x82500 + family as u64);
        let original_lane = t.lane.as_ref().unwrap().observe_original();
        let store = t.original.layout.checked_store().unwrap();
        assert_eq!(active_text_lane_count_for_test(store), 1);
        let target = b_target_from_original(&t.original);
        let paths = [target.join("head.bin"), target.join("text.wal")];
        let before: Vec<_> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
        let charged = store
            .archive_bootstrap_policy
            .as_ref()
            .unwrap()
            .pending
            .pool
            .charged_for_test();
        retire_lane(&mut t).unwrap();
        assert!(t.lane.is_none() && t.drained.is_none() && t.retired.is_some());
        assert!(original_lane.is_cancelled());
        assert_eq!(
            active_text_lane_count_for_test(t.original.layout.checked_store().unwrap()),
            0
        );
        validate_finish(&t).unwrap();
        if let Some(foreign) = previous.as_ref() {
            // Reuse the previous genuine family, keeping its native Source live:
            // equal receipt shape is never association with this retired lane.
            let wrong_saved = foreign
                .accepted
                .as_ref()
                .unwrap()
                .saved_retirement_completion()
                .unwrap()
                .clone();
            let accepted = t.accepted.as_ref().unwrap();
            let store = t.original.layout.checked_store().unwrap();
            let owner = t.original.owner.borrow_current().unwrap();
            let resources = accepted.resources().clone();
            assert!(store
                .with_v2_text_current(&owner, |admission| {
                    let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                    guard
                        .admit_completed_finish(t.retired.as_ref().unwrap(), &wrong_saved, accepted)
                        .map(|_| ())
                })
                .is_err());
            validate_finish(&t).unwrap();
        }
        retire_lane(&mut t).unwrap();
        validate_finish(&t).unwrap();
        assert_eq!(
            t.original
                .layout
                .checked_store()
                .unwrap()
                .archive_bootstrap_policy
                .as_ref()
                .unwrap()
                .pending
                .pool
                .charged_for_test(),
            charged
        );
        for (path, bytes) in paths.iter().zip(before) {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
        assert!(!t.model_dropped.load(Ordering::SeqCst));
        assert!(!t.sdk_dropped.load(Ordering::SeqCst));
        assert_eq!(
            text_query_control_io_for_test(t.original.layout.checked_store().unwrap()),
            0
        );
        if family == 2 {
            let accepted = t.accepted.as_ref().unwrap();
            accepted.cancel_original_projection_domain_for_test();
            let store = t.original.layout.checked_store().unwrap();
            let owner = t.original.owner.borrow_current().unwrap();
            store.with_v2_text_current(&owner, |_| Ok(())).unwrap();
            assert!(validate_finish(&t).is_err());
            assert!(t.retired.is_some());
        }
        previous = Some(t);
    }
}

// Same existing test-only original DATA loan as b_target, with no constructor,
// owner mutation or key/SDK extractor introduced by this fixture.
fn b_target_from_original(original: &TextFixture) -> std::path::PathBuf {
    original
        .owner
        .borrow_current()
        .unwrap()
        .source()
        .source()
        .capture
        .target
        .to_path_buf()
}

#[test]
fn actual_lane_retirement_refuses_before_wc_and_preserves_inputs_after_same_length_wc_damage() {
    let mut t = into_lane_fixture(worker_retirement_fixture(0, 0x82510));
    assert!(retire_lane(&mut t).is_err());
    assert!(t.lane.is_some() && t.drained.is_some() && t.retired.is_none());
    assert_eq!(
        active_text_lane_count_for_test(t.original.layout.checked_store().unwrap()),
        1
    );
    // Complete physical work through its concrete actual admissions, then put the
    // SAME owning slots back; no reconstructed lane or Drained is introduced.
    let LaneFinishFixture {
        accepted,
        predecessor,
        original,
        model_dropped,
        sdk_dropped,
        basis,
        mut lane,
        mut drained,
        retired,
    } = t;
    assert!(retired.is_none());
    let mut worker = WorkerRetirementFixture {
        f: AcceptedFixture {
            accepted: accepted.unwrap(),
            lane: lane.take().unwrap(),
            predecessor,
            model_dropped,
            sdk_dropped,
            original,
        },
        basis,
        drained: drained.take().unwrap(),
    };
    let marker = deletion_admission(&worker).unwrap();
    worker.f.accepted.run_retirement_deletion(marker).unwrap();
    let marker = completion_admission(&worker).unwrap();
    worker.f.accepted.run_retirement_completion(marker).unwrap();
    worker.f.accepted.prepare_completed_resident_data().unwrap();
    let wal = b_target(&worker.f).join("text.wal");
    let original_wal = std::fs::read(&wal).unwrap();
    let mut corrupt = original_wal.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    std::fs::write(&wal, &corrupt).unwrap();
    let mut t = into_lane_fixture(worker);
    assert!(retire_lane(&mut t).is_err());
    assert!(t.lane.is_some() && t.drained.is_some() && t.retired.is_none());
    assert_eq!(std::fs::read(&wal).unwrap(), corrupt);
    std::fs::write(&wal, &original_wal).unwrap();
    retire_lane(&mut t).unwrap();
    validate_finish(&t).unwrap();
    // The fresh finish proof, independently of the old retirement proof, rejects
    // the same real WC content damage and does not lose the retired lane owner.
    std::fs::write(&wal, &corrupt).unwrap();
    assert!(validate_finish(&t).is_err());
    assert!(t.retired.is_some());
    assert_eq!(std::fs::read(&wal).unwrap(), corrupt);
    std::fs::write(&wal, &original_wal).unwrap();
    validate_finish(&t).unwrap();
}

#[test]
fn actual_late_scope_stop_keeps_removed_lane_and_drained_outputs_but_refuses_fresh_finish() {
    let mut t = actual_completed_fixture(0, 0x82520);
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let revoke = owner.warm_scope().unwrap().revocation();
    let accepted = t.accepted.as_ref().unwrap();
    let resources = accepted.resources().clone();
    let result = store.with_v2_text_current(&owner, |admission| {
        let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
        guard.retire_completed_text_lane_into(
            accepted,
            &t.basis,
            &mut t.lane,
            &mut t.drained,
            &mut t.retired,
        )?;
        assert!(t.lane.is_none() && t.drained.is_none() && t.retired.is_some());
        // Real native postcheck, after both owning takes/removal have cached.
        revoke.revoke();
        Ok(())
    });
    assert!(result.is_err());
    assert!(t.lane.is_none() && t.drained.is_none() && t.retired.is_some());
    assert_eq!(active_text_lane_count_for_test(store), 0);
    assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
    assert!(validate_finish(&t).is_err());
    assert!(accepted.saved_retirement_completion().is_ok());
    assert!(!t.model_dropped.load(Ordering::SeqCst));
    assert!(!t.sdk_dropped.load(Ordering::SeqCst));
}
