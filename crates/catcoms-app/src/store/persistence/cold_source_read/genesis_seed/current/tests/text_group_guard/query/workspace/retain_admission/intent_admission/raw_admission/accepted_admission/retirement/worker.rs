//! Real physical retirement under actual canonical close/drain and distinct fresh
//! effect markers. No parsed plan, fake Saved or count-based Drained appears here.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextAcceptedRetirementCompletion, AdmittedV2TextAcceptedRetirementDeletion,
};
use crate::store::persistence::v2_text::V2TextRetirementTestCut as Cut;
use std::os::windows::fs::OpenOptionsExt;
use std::sync::atomic::AtomicUsize;

pub(super) struct WorkerRetirementFixture {
    pub(super) f: AcceptedFixture,
    pub(super) basis: V2AdvancedAppendBasis,
    pub(super) drained: V2TextRetirementRootsDrained,
}
pub(super) fn worker_retirement_fixture(family: usize, number: u64) -> WorkerRetirementFixture {
    let mut f = accepted_fixture(family, number);
    select_and_finalize(&mut f);
    let basis = f.accepted.accepted_final_basis_data().unwrap().clone();
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    f.accepted
        .prepare_retirement_data(store, owner.source())
        .unwrap();
    drop(owner);
    let mut prepared = Some(prepare_close(&f, &basis));
    let mut closed = None;
    close(&f, &basis, &mut prepared, &mut closed).unwrap();
    f.accepted
        .drain_original_query_readers_outside_writers()
        .unwrap();
    let mut drained = None;
    assert!(V2TextRetirementRootsClosed::poll_drained_into(&mut closed, &mut drained).unwrap());
    let mut drained = drained.unwrap();
    let mut removed = None;
    detach(&f, &basis, &mut drained, &mut removed).unwrap();
    // Actual removed owning metadata drains outside writers, never a fake clear.
    assert!(removed.is_some());
    drop(removed);
    WorkerRetirementFixture { f, basis, drained }
}
pub(super) fn deletion_admission(
    t: &WorkerRetirementFixture,
) -> Result<AdmittedV2TextAcceptedRetirementDeletion, AppError> {
    let store = t.f.original.layout.checked_store()?;
    let owner = t.f.original.owner.borrow_current()?;
    let resources = t.f.accepted.resources().clone();
    let mut lease = Some(PreparedV2TextPhysicalLease::prepare(
        store,
        owner.source(),
        &resources,
    )?);
    let mut admitted = None;
    let result = store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        guard.admit_text_retirement_deletion_into(
            &t.f.lane,
            &t.f.accepted,
            &t.basis,
            &t.drained,
            &mut lease,
            &mut admitted,
        )
    });
    // These helpers introduce no late fault; always preserve any actual marker.
    if let Some(marker) = admitted {
        assert!(lease.is_none());
        return Ok(marker);
    }
    result?;
    Err(AppError::Invalid(
        "missing actual deletion admission".into(),
    ))
}
pub(super) fn completion_admission(
    t: &WorkerRetirementFixture,
) -> Result<AdmittedV2TextAcceptedRetirementCompletion, AppError> {
    let store = t.f.original.layout.checked_store()?;
    let owner = t.f.original.owner.borrow_current()?;
    let ready = t.f.accepted.ready_retirement_completion()?;
    let resources = ready.resources().clone();
    let mut lease = Some(PreparedV2TextPhysicalLease::prepare(
        store,
        owner.source(),
        &resources,
    )?);
    let mut admitted = None;
    let result = store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        guard.admit_text_retirement_completion_into(
            &t.f.lane,
            ready,
            &t.basis,
            &t.drained,
            &mut lease,
            &mut admitted,
        )
    });
    if let Some(marker) = admitted {
        assert!(lease.is_none());
        return Ok(marker);
    }
    result?;
    Err(AppError::Invalid(
        "missing actual completion admission".into(),
    ))
}
fn run_deletion(
    t: &mut WorkerRetirementFixture,
    marker: AdmittedV2TextAcceptedRetirementDeletion,
) -> Result<(), AppError> {
    let calls = Arc::new(AtomicUsize::new(0));
    let result = PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
        calls.clone(),
        None,
        || t.f.accepted.run_retirement_deletion(marker),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    result
}
fn run_completion(
    t: &mut WorkerRetirementFixture,
    marker: AdmittedV2TextAcceptedRetirementCompletion,
) -> Result<(), AppError> {
    let calls = Arc::new(AtomicUsize::new(0));
    let result = PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
        calls.clone(),
        None,
        || t.f.accepted.run_retirement_completion(marker),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    result
}
fn idle(t: &WorkerRetirementFixture) {
    assert!(!t.f.lane.workspace_active_for_test());
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(
            t.f.original.layout.checked_store().unwrap()
        ),
        0
    );
}
fn validate_wc(t: &WorkerRetirementFixture) -> Result<(), AppError> {
    let store = t.f.original.layout.checked_store()?;
    let owner = t.f.original.owner.borrow_current()?;
    let resources = t.f.accepted.resources().clone();
    store.with_v2_text_current(&owner, |a| {
        let guard = store.lock_v2_text_current_group(a, &resources)?;
        t.f.accepted
            .validate_retirement_completion_under_current_guard(&guard, &t.drained)
    })
}
fn wal(t: &WorkerRetirementFixture) -> std::path::PathBuf {
    b_target(&t.f).join("text.wal")
}

#[test]
fn actual_wd_precedes_any_unlink_and_fresh_wc_alone_records_completed_quota_credit() {
    let mut t = worker_retirement_fixture(0, 0x82200);
    let old = t.f.accepted.original_retired_paths_for_test().unwrap();
    assert_eq!(old.len(), 4);
    let (_, wd) =
        t.f.accepted
            .original_retirement_output_for_test(Cut::DeletionCreated)
            .unwrap();
    let pages = b_target(&t.f).join("text.pages");
    let unchanged: Vec<_> = std::fs::read_dir(&pages)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !old.contains(p))
        .map(|p| {
            let bytes = std::fs::read(&p).unwrap();
            (p, bytes)
        })
        .collect();
    let held_old = old[0].clone();
    let all = old.clone();
    let main = wal(&t);
    let expected = wd.to_vec();
    t.f.accepted.set_retirement_pause_for_test(
        Cut::ObjectOpened,
        Box::new(move || {
            assert_eq!(std::fs::read(main).unwrap(), expected);
            assert!(all.iter().all(|p| p.is_file()));
            assert!(std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(held_old)
                .is_err());
        }),
    );
    t.f.accepted
        .set_retirement_fault_for_test(Cut::ObjectOpened);
    let marker = deletion_admission(&t).unwrap();
    assert!(run_deletion(&mut t, marker).is_err());
    assert!(t.f.accepted.saved_retirement_deletion().is_ok());
    assert!(t.f.accepted.ready_retirement_completion().is_err());
    assert!(t.f.accepted.saved_retirement_completion().is_err());
    assert!(old.iter().all(|p| p.is_file()));
    idle(&t);
    drop(
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&old[0])
            .unwrap(),
    );
    let marker = deletion_admission(&t).unwrap();
    run_deletion(&mut t, marker).unwrap();
    assert!(old.iter().all(|p| !p.exists()));
    for (p, bytes) in &unchanged {
        assert_eq!(&std::fs::read(p).unwrap(), bytes);
    }
    assert_eq!(std::fs::read(wal(&t)).unwrap(), wd[..]);
    assert!(t.f.accepted.saved_retirement_completion().is_err());
    idle(&t);
    // Actual deletion completion DATA does not excuse a new present foreign old
    // name. The DISTINCT fresh WC admission must reprove every absence.
    std::fs::write(&old[0], b"foreign private object, preserve").unwrap();
    assert!(completion_admission(&t).is_err());
    assert_eq!(
        std::fs::read(&old[0]).unwrap(),
        b"foreign private object, preserve"
    );
    std::fs::remove_file(&old[0]).unwrap();
    let marker = completion_admission(&t).unwrap();
    run_completion(&mut t, marker).unwrap();
    validate_wc(&t).unwrap();
    idle(&t);
    let (baseline, reclaimed) = t.f.accepted.retirement_baseline_for_test().unwrap();
    let saved = t.f.accepted.saved_retirement_completion().unwrap();
    let effective = saved.effective_baseline_data().unwrap();
    assert_eq!(effective.bytes, baseline.bytes - reclaimed.bytes);
    assert_eq!(effective.objects, baseline.objects - 4);
    assert_eq!(effective, saved.effective_baseline_data().unwrap());
    for (p, bytes) in unchanged {
        assert_eq!(std::fs::read(p).unwrap(), bytes);
    }
}

#[test]
fn actual_same_original_wd_wc_partial_and_lost_outcomes_resume_without_signing_or_resealing() {
    let mut t = worker_retirement_fixture(0, 0x82210);
    let (_, wd) =
        t.f.accepted
            .original_retirement_output_for_test(Cut::DeletionCreated)
            .unwrap();
    let (_, wc) =
        t.f.accepted
            .original_retirement_output_for_test(Cut::CompletionCreated)
            .unwrap();
    for (index, cut) in [
        Cut::DeletionCreated,
        Cut::DeletionStaged,
        Cut::DeletionRenamed,
        Cut::DeletionSaved,
        Cut::ObjectUnlinked,
        Cut::ReadyCached,
    ]
    .into_iter()
    .enumerate()
    {
        if index == 0 {
            t.f.accepted
                .set_retirement_partial_write_fault_for_test(cut, 13);
        } else {
            t.f.accepted.set_retirement_fault_for_test(cut);
        }
        let marker = deletion_admission(&t).unwrap();
        assert!(run_deletion(&mut t, marker).is_err(), "cut {cut:?}");
        idle(&t);
        assert!(t.f.accepted.saved_retirement_completion().is_err());
    }
    assert!(t.f.accepted.ready_retirement_completion().is_ok());
    assert_eq!(std::fs::read(wal(&t)).unwrap(), wd[..]);
    for (index, cut) in [
        Cut::CompletionCreated,
        Cut::CompletionStaged,
        Cut::CompletionRenamed,
        Cut::CompletionSaved,
    ]
    .into_iter()
    .enumerate()
    {
        if index == 0 {
            t.f.accepted
                .set_retirement_partial_write_fault_for_test(cut, 0);
        } else {
            t.f.accepted.set_retirement_fault_for_test(cut);
        }
        let marker = completion_admission(&t).unwrap();
        assert!(run_completion(&mut t, marker).is_err(), "cut {cut:?}");
        idle(&t);
    }
    assert_eq!(std::fs::read(wal(&t)).unwrap(), wc[..]);
    assert!(t.f.accepted.saved_retirement_completion().is_ok());
    validate_wc(&t).unwrap();
    let (_, same_wd) =
        t.f.accepted
            .original_retirement_output_for_test(Cut::DeletionCreated)
            .unwrap();
    let (_, same_wc) =
        t.f.accepted
            .original_retirement_output_for_test(Cut::CompletionCreated)
            .unwrap();
    assert_eq!(wd[..], same_wd[..]);
    assert_eq!(wc[..], same_wc[..]);
}

#[test]
fn actual_retirement_preserves_missing_replaced_nonprefix_oversized_stages_and_post_admission_bad_journals(
) {
    let mut t = worker_retirement_fixture(0, 0x82220);
    let (path, expected) =
        t.f.accepted
            .original_retirement_output_for_test(Cut::DeletionCreated)
            .unwrap();
    t.f.accepted
        .set_retirement_partial_write_fault_for_test(Cut::DeletionCreated, 13);
    let marker = deletion_admission(&t).unwrap();
    assert!(run_deletion(&mut t, marker).is_err());
    let prefix = expected[..13].to_vec();
    let held = path.with_extension("retirement-held");
    // Missing recorded output is not an unstarted stage or a new creation grant.
    std::fs::rename(&path, &held).unwrap();
    assert!(deletion_admission(&t).is_err());
    assert!(!path.exists());
    std::fs::rename(&held, &path).unwrap();
    // Byte-equal replacement with another native object remains foreign.
    std::fs::rename(&path, &held).unwrap();
    std::fs::write(&path, &prefix).unwrap();
    assert!(deletion_admission(&t).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), prefix);
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&held, &path).unwrap();
    let mut wrong = prefix.clone();
    wrong[4] ^= 1;
    std::fs::write(&path, &wrong).unwrap();
    assert!(deletion_admission(&t).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), wrong);
    let oversized = vec![0x73; expected.len() + 1];
    std::fs::write(&path, &oversized).unwrap();
    assert!(deletion_admission(&t).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), oversized);
    std::fs::write(&path, &prefix).unwrap();
    // Complete parent bytes are rechecked by the FINITE worker too, after actual
    // admission but before any prefix suffix. Same native/length corruption fails.
    let marker = deletion_admission(&t).unwrap();
    let main = wal(&t);
    let original = std::fs::read(&main).unwrap();
    let mut wrong = original.clone();
    let at = wrong.len() / 2;
    wrong[at] ^= 1;
    std::fs::write(&main, &wrong).unwrap();
    assert!(run_deletion(&mut t, marker).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), prefix);
    assert_eq!(std::fs::read(&main).unwrap(), wrong);
    std::fs::write(&main, &original).unwrap();
    let marker = deletion_admission(&t).unwrap();
    run_deletion(&mut t, marker).unwrap();
    t.f.accepted
        .set_retirement_partial_write_fault_for_test(Cut::CompletionCreated, 13);
    let marker = completion_admission(&t).unwrap();
    assert!(run_completion(&mut t, marker).is_err());
    let wc_prefix = std::fs::read(&path).unwrap();
    let marker = completion_admission(&t).unwrap();
    let original = std::fs::read(&main).unwrap();
    let mut wrong = original.clone();
    wrong[30] ^= 1;
    std::fs::write(&main, &wrong).unwrap();
    assert!(run_completion(&mut t, marker).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), wc_prefix);
    assert_eq!(std::fs::read(&main).unwrap(), wrong);
    std::fs::write(&main, original).unwrap();
    let marker = completion_admission(&t).unwrap();
    run_completion(&mut t, marker).unwrap();
    validate_wc(&t).unwrap();
    idle(&t);
}

#[test]
fn actual_retirement_named_and_unlinked_objects_keep_same_vault_lease_until_finite_cleanup() {
    for (index, cut) in [Cut::ObjectOpened, Cut::ObjectUnlinkedHeld]
        .into_iter()
        .enumerate()
    {
        let mut t = worker_retirement_fixture(0, 0x82230 + index as u64);
        let old = t.f.accepted.original_retired_paths_for_test().unwrap();
        let first = old[0].clone();
        let target = b_target(&t.f);
        let marker = deletion_admission(&t).unwrap();
        let (tx, rx) = sync_channel(0);
        let (release, wait) = sync_channel(0);
        t.f.accepted.set_retirement_pause_for_test(
            cut,
            Box::new(move || {
                tx.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(15)).unwrap();
            }),
        );
        t.f.accepted.set_retirement_fault_for_test(cut);
        let WorkerRetirementFixture { f, basis, drained } = t;
        let AcceptedFixture {
            mut accepted,
            lane,
            predecessor,
            model_dropped,
            sdk_dropped,
            original,
        } = f;
        let worker = std::thread::spawn(move || {
            let result = accepted.run_retirement_deletion(marker);
            (accepted, result)
        });
        rx.recv_timeout(Duration::from_secs(15)).unwrap();
        if cut == Cut::ObjectOpened {
            assert!(
                first.is_file(),
                "ObjectOpened must still hold the NAMED original"
            );
            assert!(std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&first)
                .is_err());
        } else {
            // This distinct production seam is BEFORE drop(original), after the
            // actual successful unlink. Name absence alone is not the FD drain.
            assert!(
                matches!(std::fs::symlink_metadata(&first),Err(e) if e.kind()==std::io::ErrorKind::NotFound)
            );
            assert!(old[1..].iter().all(|p| p.is_file()));
        }
        let TextFixture {
            owner,
            layout,
            _directory: directory,
        } = original;
        drop(owner);
        drop(lane);
        drop(predecessor);
        drop(layout);
        drop(basis);
        drop(drained);
        assert!(
            matches!(
                catcoms_storage::acquire_vault_session(directory.path()),
                Err(catcoms_storage::StorageError::VaultBusy)
            ),
            "actual finite lease/End remains held while the original File is parked at {cut:?}"
        );
        release.send(()).unwrap();
        let (accepted, result) = worker.join().unwrap();
        assert!(result.is_err());
        assert!(accepted.saved_retirement_deletion().is_ok());
        assert!(accepted.ready_retirement_completion().is_err());
        if cut == Cut::ObjectOpened {
            drop(
                std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(&first)
                    .unwrap(),
            );
        } else {
            assert!(
                matches!(std::fs::symlink_metadata(&first),Err(e) if e.kind()==std::io::ErrorKind::NotFound)
            );
        }
        // The same-run WAL and all other helper Files also drained; no dormant
        // metadata/receipt keeps the OS vault lease or writer End after join.
        drop(
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(target.join("text.wal"))
                .unwrap(),
        );
        let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
        assert!(!model_dropped.load(Ordering::SeqCst));
        assert!(!sdk_dropped.load(Ordering::SeqCst));
        drop(session);
        drop(accepted);
        assert!(model_dropped.load(Ordering::SeqCst));
        assert!(sdk_dropped.load(Ordering::SeqCst));
    }
}

#[test]
fn actual_admitted_retirement_finishes_after_stop_but_stop_between_phases_keeps_wd_debt() {
    for completing in [false, true] {
        let mut t = worker_retirement_fixture(0, 0x82240 + u64::from(completing));
        let (_, wd) =
            t.f.accepted
                .original_retirement_output_for_test(Cut::DeletionCreated)
                .unwrap();
        if completing {
            let marker = deletion_admission(&t).unwrap();
            run_deletion(&mut t, marker).unwrap();
            let marker = completion_admission(&t).unwrap();
            let store = t.f.original.layout.checked_store().unwrap();
            let owner = t.f.original.owner.borrow_current().unwrap();
            owner.warm_scope().unwrap().revocation().revoke();
            t.f.accepted.run_retirement_completion(marker).unwrap();
            assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
            drop(owner);
            assert!(t.f.accepted.saved_retirement_completion().is_ok());
        } else {
            let marker = deletion_admission(&t).unwrap();
            let store = t.f.original.layout.checked_store().unwrap();
            let owner = t.f.original.owner.borrow_current().unwrap();
            owner.warm_scope().unwrap().revocation().revoke();
            t.f.accepted.run_retirement_deletion(marker).unwrap();
            assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
            drop(owner);
            assert!(t.f.accepted.ready_retirement_completion().is_ok());
            assert!(t.f.accepted.saved_retirement_completion().is_err());
            assert_eq!(std::fs::read(wal(&t)).unwrap(), wd[..]);
            assert!(completion_admission(&t).is_err());
        }
        assert!(validate_wc(&t).is_err());
        idle(&t);
        assert!(!t.f.model_dropped.load(Ordering::SeqCst));
        assert!(!t.f.sdk_dropped.load(Ordering::SeqCst));
    }
}
