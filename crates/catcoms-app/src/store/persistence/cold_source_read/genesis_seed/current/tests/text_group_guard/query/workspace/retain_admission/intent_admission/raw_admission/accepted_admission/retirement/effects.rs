//! Admission-only boundaries use actual B/final/drained owners. Physical fault,
//! byte-preservation and finite worker matrices live separately in worker.rs.
use super::worker::{deletion_admission, worker_retirement_fixture};
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextAcceptedRetirementCompletion, AdmittedV2TextAcceptedRetirementDeletion,
};
use crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test;
use std::os::windows::fs::OpenOptionsExt;

#[test]
fn actual_retirement_foreign_current_refuses_before_any_original_journal_file_open() {
    let a = worker_retirement_fixture(0, 0x82400);
    let b = worker_retirement_fixture(0, 0x82401);
    let store_a = a.f.original.layout.checked_store().unwrap();
    let owner_a = a.f.original.owner.borrow_current().unwrap();
    let resources_a = a.f.accepted.resources().clone();
    let mut physical = Some(
        PreparedV2TextPhysicalLease::prepare(store_a, owner_a.source(), &resources_a).unwrap(),
    );
    let wal = b_target(&a.f).join("text.wal");
    let original = std::fs::read(&wal).unwrap();
    // A sharing error would expose a too-late resource association check. The
    // foreign runtime must reject identity without trying to touch this real WAL.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&wal)
        .unwrap();
    let store_b = b.f.original.layout.checked_store().unwrap();
    let owner_b = b.f.original.owner.borrow_current().unwrap();
    let resources_b = b.f.accepted.resources().clone();
    let mut admitted: Option<AdmittedV2TextAcceptedRetirementDeletion> = None;
    let result = store_b.with_v2_text_current(&owner_b, |admission| {
        let mut guard = store_b.lock_v2_text_current_group(admission, &resources_b)?;
        guard.admit_text_retirement_deletion_into(
            &a.f.lane,
            &a.f.accepted,
            &a.basis,
            &a.drained,
            &mut physical,
            &mut admitted,
        )
    });
    assert!(matches!(result, Err(AppError::Invalid(_))));
    assert!(admitted.is_none() && physical.is_some());
    assert!(!a.f.lane.workspace_active_for_test());
    assert!(!b.f.lane.workspace_active_for_test());
    drop(held);
    assert_eq!(std::fs::read(wal).unwrap(), original);
    drop(physical);
    // The exact original remains usable; the failed foreign proof did not cancel
    // its lane, spend a work allowance, or replace the retained physical input.
    drop(deletion_admission(&a).unwrap());
}

#[test]
fn actual_domain_stop_refuses_new_deletion_and_completion_with_native_current_still_live() {
    for after_deletion in [false, true] {
        let mut t = worker_retirement_fixture(0, 0x82410 + u64::from(after_deletion));
        if after_deletion {
            let marker = deletion_admission(&t).unwrap();
            t.f.accepted.run_retirement_deletion(marker).unwrap();
            assert!(t.f.accepted.ready_retirement_completion().is_ok());
        }
        let wal = b_target(&t.f).join("text.wal");
        let original_wal = std::fs::read(&wal).unwrap();
        let old = t.f.accepted.original_retired_paths_for_test().unwrap();
        let original_objects: Vec<_> = old
            .iter()
            .map(|p| (p.clone(), std::fs::read(p).ok()))
            .collect();
        let store = t.f.original.layout.checked_store().unwrap();
        let owner = t.f.original.owner.borrow_current().unwrap();
        let resources = t.f.accepted.resources().clone();
        t.f.accepted.cancel_original_projection_domain_for_test();
        // Distinguish this independent Root/domain fence from a native ScopeStop.
        store.with_v2_text_current(&owner, |_| Ok(())).unwrap();
        let mut physical =
            Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
        let mut deletion: Option<AdmittedV2TextAcceptedRetirementDeletion> = None;
        let mut completion: Option<AdmittedV2TextAcceptedRetirementCompletion> = None;
        let result = store.with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            if after_deletion {
                guard.admit_text_retirement_completion_into(
                    &t.f.lane,
                    t.f.accepted.ready_retirement_completion()?,
                    &t.basis,
                    &t.drained,
                    &mut physical,
                    &mut completion,
                )
            } else {
                guard.admit_text_retirement_deletion_into(
                    &t.f.lane,
                    &t.f.accepted,
                    &t.basis,
                    &t.drained,
                    &mut physical,
                    &mut deletion,
                )
            }
        });
        assert!(result.is_err());
        assert!(physical.is_some() && deletion.is_none() && completion.is_none());
        assert!(!t.f.lane.workspace_active_for_test());
        assert_eq!(std::fs::read(wal).unwrap(), original_wal);
        for (path, bytes) in original_objects {
            assert_eq!(std::fs::read(path).ok(), bytes);
        }
        assert!(t.f.accepted.saved_retirement_completion().is_err());
    }
}

#[test]
fn actual_unstarted_deletion_gate_blocks_parent_read_until_same_marker_and_lease_drain() {
    let t = worker_retirement_fixture(0, 0x82420);
    let marker = deletion_admission(&t).unwrap();
    assert!(t.f.lane.workspace_active_for_test());
    let store = t.f.original.layout.checked_store().unwrap();
    let owner = t.f.original.owner.borrow_current().unwrap();
    let mut read = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
    let resources = read.resources().clone();
    let result = store.with_v2_text_current(&owner, |admission| {
        let guard = store.lock_v2_text_current_group(admission, &resources)?;
        read.capture(&guard).map(|_| ())
    });
    assert!(result.is_err());
    assert_eq!(text_query_control_io_for_test(store), 0);
    drop(marker);
    assert!(!t.f.lane.workspace_active_for_test());
    store
        .with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            read.capture(&guard).map(|_| ())
        })
        .unwrap();
    assert!(t.f.accepted.saved_retirement_deletion().is_err());
}
