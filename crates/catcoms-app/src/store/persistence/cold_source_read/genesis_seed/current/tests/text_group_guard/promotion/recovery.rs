//! Actual same-owner recovery through distinct seed/scratch admission roles.
//! This is not new-mount/cold replay or an ordinary-message durability claim.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    PreparedV2TextPromotionScratchCapture, PreparedV2TextPromotionSeedRepairCapture,
};
use crate::store::persistence::v2_text::{
    PreparedV2TextPromotionScratch, PreparedV2TextPromotionSeedRepair,
};

fn fail_actual_stage(f: &mut PromotionFixture, cut: V2TextPromotionTestCut) {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.prepared.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_stage_into(&f.lane, &f.prepared, &mut lease, &mut admitted)
        })
        .unwrap();
    f.prepared.set_work_fault_for_test(cut);
    assert!(f.prepared.run_stage(admitted.take().unwrap()).is_err());
    assert!(f.prepared.ready_selection().is_err());
    assert!(f.prepared.saved_promotion().is_err());
}

fn commit_ready(f: &mut PromotionFixture) {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.prepared.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_commit_into(
                &f.lane,
                f.prepared.ready_selection()?,
                &mut lease,
                &mut admitted,
            )
        })
        .unwrap();
    f.prepared.run_commit(admitted.take().unwrap()).unwrap();
    assert!(f.prepared.saved_promotion().is_ok());
}

#[test]
fn actual_complete_main_repairs_closed_partial_child_without_resealing_or_selecting() {
    let mut f = promotion_fixture(0, 0x7ec00);
    fail_actual_stage(&mut f, V2TextPromotionTestCut::DirectoryCreated);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let original_seed = std::fs::read(target.join("text.seed")).unwrap();
    let original_head = std::fs::read(target.join("head.bin")).unwrap();
    let mut repair =
        PreparedV2TextPromotionSeedRepair::prepare(store, owner.source(), &f.prepared).unwrap();
    let resources = repair.resources().clone();
    let mut capture = Some(
        PreparedV2TextPromotionSeedRepairCapture::prepare(store, owner.source(), &repair).unwrap(),
    );
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_seed_repair_into(
                &f.lane,
                &mut repair,
                &mut capture,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(
        capture.is_none(),
        "whole source/lease bundle moves with admission"
    );
    repair.run(admitted.take().unwrap()).unwrap();
    repair.install_ready_into(&mut f.prepared).unwrap();
    assert_eq!(
        std::fs::read(target.join("text.seed")).unwrap(),
        original_seed
    );
    assert_eq!(
        std::fs::read(target.join("head.bin")).unwrap(),
        original_head
    );
    assert!(f.prepared.ready_selection().is_ok());
    assert!(f.prepared.saved_promotion().is_err());
    f.prepared
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .unwrap();
    drop(capture);
    drop(resources);
    drop(repair);
    drop(owner);
    commit_ready(&mut f);
    let mut lane = Some(f.lane);
    finish_promotion_lane(&f.original, &f.prepared, &mut lane);
}

#[test]
fn actual_late_seed_capture_error_keeps_files_and_os_lease_coupled_until_outside_drop() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut f = promotion_fixture(0, 0x7ec10);
    fail_actual_stage(&mut f, V2TextPromotionTestCut::DirectoryCreated);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let main = owner.source().source().capture.target.join("text.seed");
    let mut repair =
        PreparedV2TextPromotionSeedRepair::prepare(store, owner.source(), &f.prepared).unwrap();
    let resources = repair.resources().clone();
    let mut capture = Some(
        PreparedV2TextPromotionSeedRepairCapture::prepare(store, owner.source(), &repair).unwrap(),
    );
    let revocation = owner.warm_scope().unwrap().revocation();
    repair.set_after_capture_files_for_test(Box::new(move || revocation.revoke()));
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_seed_repair_into(
                &f.lane,
                &mut repair,
                &mut capture,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none());
    assert!(
        capture.is_some(),
        "failed complete source stays in its ONE active owner"
    );
    assert!(
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&main)
            .is_err(),
        "the actual captured MAIN File remains held after the late proof failure"
    );
    drop(owner);
    drop(resources);
    let PromotionFixture {
        prepared,
        lane,
        original,
    } = f;
    let TextFixture {
        owner,
        layout,
        _directory: directory,
    } = original;
    drop(owner);
    drop(lane);
    drop(layout);
    assert!(matches!(
        catcoms_storage::acquire_vault_session(directory.path()),
        Err(catcoms_storage::StorageError::VaultBusy)
    ));
    // A single structural drop, with no manual source-take-before-lease convention.
    drop(capture.take());
    let main = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(main)
        .unwrap();
    drop(main);
    let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    assert!(prepared.ready_selection().is_err());
    assert!(prepared.saved_promotion().is_err());
    drop(session);
    drop(repair);
    drop(prepared);
}

#[test]
fn actual_partial_pre_main_scratch_cleanup_reuses_original_seed_bytes_with_fresh_admission() {
    let mut f = promotion_fixture(0, 0x7ec20);
    fail_actual_stage(&mut f, V2TextPromotionTestCut::SeedPartial);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let partial_seed = std::fs::read(target.join("text.seed.stage.v1")).unwrap();
    let original_head = std::fs::read(target.join("head.bin")).unwrap();
    assert!(!partial_seed.is_empty());
    assert!(!target.join("text.seed").exists());
    assert!(!target.join("text.pages").exists());
    let mut scratch =
        PreparedV2TextPromotionScratch::prepare(store, owner.source(), &f.prepared).unwrap();
    let resources = scratch.resources().clone();
    let mut capture = Some(
        PreparedV2TextPromotionScratchCapture::prepare(store, owner.source(), &scratch).unwrap(),
    );
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_scratch_into(
                &f.lane,
                &mut scratch,
                &mut capture,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(capture.is_none());
    scratch.run(admitted.take().unwrap()).unwrap();
    scratch.install_cleanup_into(&mut f.prepared).unwrap();
    assert!(!target.join("text.seed.stage.v1").exists());
    assert!(!target.join("text.seed").exists());
    assert!(!target.join("text.pages").exists());
    assert_eq!(
        std::fs::read(target.join("head.bin")).unwrap(),
        original_head
    );
    f.prepared
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .unwrap();
    drop(capture);
    drop(resources);
    drop(scratch);
    drop(owner);
    stage_and_commit(&mut f);
    let complete_seed = std::fs::read(target.join("text.seed")).unwrap();
    assert!(
        complete_seed.starts_with(&partial_seed),
        "same original AEAD frame, no reseal"
    );
    let mut lane = Some(f.lane);
    finish_promotion_lane(&f.original, &f.prepared, &mut lane);
}

#[test]
fn actual_unstarted_stage_end_refuses_both_recovery_roles_before_caching_any_source() {
    let f = promotion_fixture(0, 0x7ec30);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = f.prepared.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut stage = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_stage_into(&f.lane, &f.prepared, &mut lease, &mut stage)
        })
        .unwrap();
    assert!(stage.is_some());
    let mut repair =
        PreparedV2TextPromotionSeedRepair::prepare(store, owner.source(), &f.prepared).unwrap();
    let mut capture = Some(
        PreparedV2TextPromotionSeedRepairCapture::prepare(store, owner.source(), &repair).unwrap(),
    );
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_seed_repair_into(
                &f.lane,
                &mut repair,
                &mut capture,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none());
    assert!(!capture.as_ref().unwrap().has_source_for_test());
    let mut scratch =
        PreparedV2TextPromotionScratch::prepare(store, owner.source(), &f.prepared).unwrap();
    let mut scratch_capture = Some(
        PreparedV2TextPromotionScratchCapture::prepare(store, owner.source(), &scratch).unwrap(),
    );
    let mut admitted = None;
    assert!(store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_scratch_into(
                &f.lane,
                &mut scratch,
                &mut scratch_capture,
                &mut admitted,
            )
        })
        .is_err());
    assert!(admitted.is_none());
    assert!(!scratch_capture.as_ref().unwrap().has_source_for_test());
    // Original reobservation still succeeds: the refusals did not release, replace or
    // cancel its ONE actual unstarted work owner. They acquired no new positive marker.
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_stage_into(&f.lane, &f.prepared, &mut lease, &mut stage)
        })
        .unwrap();
    drop(capture);
    drop(scratch_capture);
    drop(stage);
}

mod preservation;
