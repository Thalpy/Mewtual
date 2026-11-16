//! Small physical-input regressions supplement (never replace) the genuine source/scope/
//! task/writer/new-mount owning-caller scenarios. No positive receipt is fabricated here.
use super::*;
#[cfg(windows)]
use crate::store::critical_namespaces::current_layout::cold_read::tests::{
    fixture_at_cut, ColdFixtureCut,
};

fn selected(path: &Path, bytes: &[u8]) -> SelectedTarget {
    fs::write(path, bytes).unwrap();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    SelectedTarget::new(file)
}

#[test]
fn original_prefix_handle_accepts_empty_short_and_complete_without_freezing_length() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("candidate");
    let target = selected(&path, b"");
    assert!(target.identity.get().is_none());
    assert_eq!(
        target
            .require_prefix(&path, b"original encrypted candidate")
            .unwrap(),
        0
    );
    let identity = target.identity().unwrap();
    let mut file = &target.file;
    file.write_all(b"original").unwrap();
    assert_eq!(
        target
            .require_prefix(&path, b"original encrypted candidate")
            .unwrap(),
        8
    );
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"original encrypted candidate").unwrap();
    assert_eq!(
        target
            .require_prefix(&path, b"original encrypted candidate")
            .unwrap(),
        28
    );
    assert_eq!(target.identity().unwrap(), identity);
}

#[test]
fn mismatch_and_oversize_are_preserved_without_truncation() {
    for bytes in [
        b"conflicting".as_slice(),
        b"original encrypted candidate plus foreign suffix".as_slice(),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("candidate");
        let target = selected(&path, bytes);
        assert!(target
            .require_prefix(&path, b"original encrypted candidate")
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn byte_identical_replacement_is_not_the_original_selected_object() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("candidate");
    let parked = directory.path().join("original");
    let expected = b"original encrypted candidate";
    let target = selected(&path, expected);
    target.require_prefix(&path, expected).unwrap();
    fs::rename(&path, &parked).unwrap();
    fs::write(&path, expected).unwrap();
    assert!(target.require_prefix(&path, expected).is_err());
    assert_eq!(fs::read(&parked).unwrap(), expected);
    assert_eq!(fs::read(&path).unwrap(), expected);
    fs::remove_file(&path).unwrap();
    fs::rename(&parked, &path).unwrap();
    target.require_prefix(&path, expected).unwrap();
}

#[test]
fn a_nonregular_target_is_not_absence_or_a_repair_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("candidate");
    fs::create_dir(&path).unwrap();
    assert!(path_present(&path).is_err());
    assert!(open_selected_target(&path).is_err());
    assert!(path.is_dir());
}

#[cfg(windows)]
#[test]
fn genuine_warm_partial_cuts_capture_after_old_owner_drain_and_new_paid_read_sdk() {
    for (number, cut) in [
        (0xca30, ColdFixtureCut::BeforePin),
        (0xca31, ColdFixtureCut::EmptyPin),
        (0xca32, ColdFixtureCut::ShortPin),
        (0xca33, ColdFixtureCut::EmptyStage),
        (0xca34, ColdFixtureCut::ShortStage),
        (0xca3b, ColdFixtureCut::CompleteStage),
    ] {
        let fixture = fixture_at_cut(number, cut);
        let store = fixture.layout.checked_store().unwrap();
        let source = fixture.source.borrow_source_data();
        let baseline = fixture.pool.charged_for_test();
        let mut capture = PreparedColdV2GenesisRepairCapture::prepare(store, &source).unwrap();
        assert_eq!(
            fixture.pool.charged_for_test(),
            baseline + cold_capture::COLD_CAPTURE_METADATA_FEE
        );
        let resources = capture.read_resources();
        store
            .with_cold_v2_genesis_initializing_read(&source, |admission| {
                let guard = store.lock_cold_initializing_genesis_group(admission, &resources)?;
                let first = capture.capture_initializing(&guard)?;
                let retry = capture.capture_initializing(&guard)?;
                assert!(Arc::ptr_eq(&first, &retry));
                assert_eq!(first.expectation_data().source_id, source.data().source);
                let files = &first.files;
                let compared = files.compare_targets(Goal::Repairable)?;
                assert!(files.compare_targets(Goal::Complete).is_err());
                match cut {
                    ColdFixtureCut::BeforePin => {
                        assert!(compared.pin_len.is_none());
                        assert!(matches!(compared.head, HeadName::Absent));
                    }
                    ColdFixtureCut::EmptyPin => {
                        assert_eq!(compared.pin_len, Some(0));
                        assert!(matches!(compared.head, HeadName::Absent));
                    }
                    ColdFixtureCut::ShortPin => {
                        let length = compared.pin_len.unwrap();
                        assert!(length > 0 && length < files.candidates()?.pin.len());
                        assert!(matches!(compared.head, HeadName::Absent));
                    }
                    ColdFixtureCut::EmptyStage => {
                        assert_eq!(compared.pin_len, Some(files.candidates()?.pin.len()));
                        assert_eq!(compared.head_len, Some(0));
                        assert!(matches!(compared.head, HeadName::Stage));
                    }
                    ColdFixtureCut::ShortStage => {
                        assert_eq!(compared.pin_len, Some(files.candidates()?.pin.len()));
                        let length = compared.head_len.unwrap();
                        assert!(length > 0 && length < files.candidates()?.head.len());
                        assert!(matches!(compared.head, HeadName::Stage));
                    }
                    ColdFixtureCut::CompleteStage => {
                        assert_eq!(compared.pin_len, Some(files.candidates()?.pin.len()));
                        assert_eq!(compared.head_len, Some(files.candidates()?.head.len()));
                        assert!(matches!(compared.head, HeadName::Stage));
                    }
                    _ => unreachable!("only real partial producer cuts are inputs"),
                }
                Ok(())
            })
            .unwrap();
        drop(resources);
        drop(capture);
        assert_eq!(fixture.pool.charged_for_test(), baseline);
    }
}

#[cfg(windows)]
#[test]
fn genuine_partial_current_is_preserved_and_never_reclassified_as_stage() {
    let fixture = fixture_at_cut(0xca35, ColdFixtureCut::ShortStage);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let target = &source.source().capture.target;
    let stage = target.join(STAGE);
    let head = target.join(HEAD);
    let prefix = fs::read(&stage).unwrap();
    fs::rename(&stage, &head).unwrap();
    let mut capture = PreparedColdV2GenesisRepairCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    store
        .with_cold_v2_genesis_initializing_read(&source, |admission| {
            let guard = store.lock_cold_initializing_genesis_group(admission, &resources)?;
            assert!(capture.capture_initializing(&guard).is_err());
            assert!(capture.saved.is_none());
            Ok(())
        })
        .unwrap();
    assert_eq!(fs::read(&head).unwrap(), prefix);
    assert!(!stage.exists());
}

#[cfg(windows)]
#[test]
fn genuine_extra_private_work_refuses_without_erasing_or_reselecting_originals() {
    let fixture = fixture_at_cut(0xca36, ColdFixtureCut::ShortStage);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let target = &source.source().capture.target;
    let extra = target.join("later-private-work.bin");
    fs::write(&extra, b"must be preserved").unwrap();
    let mut capture = PreparedColdV2GenesisRepairCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    store
        .with_cold_v2_genesis_initializing_read(&source, |admission| {
            let guard = store.lock_cold_initializing_genesis_group(admission, &resources)?;
            assert!(capture.capture_initializing(&guard).is_err());
            let original = capture.selected.as_ref().unwrap().clone();
            assert!(capture.saved.is_none());
            assert_eq!(fs::read(&extra).unwrap(), b"must be preserved");
            // Test-only removal restores the original supported folder. The same attempt
            // can now prove its SAME selected Files; it cannot reopen or mint a new source.
            fs::remove_file(&extra).unwrap();
            let saved = capture.capture_initializing(&guard)?;
            assert!(Arc::ptr_eq(&original, &saved.files));
            Ok(())
        })
        .unwrap();
    assert!(target.join(STAGE).exists());
    assert!(!target.join(HEAD).exists());
}

#[cfg(windows)]
#[test]
fn incomplete_original_selection_closes_only_that_attempt_without_presence_fallback() {
    let fixture = fixture_at_cut(0xca37, ColdFixtureCut::ShortStage);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let mut capture = PreparedColdV2GenesisRepairCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    // Independent preparations/fees are acquired outside admission and Root writers.
    let mut independent = PreparedColdV2GenesisRepairCapture::prepare(store, &source).unwrap();
    store
        .with_cold_v2_genesis_initializing_read(&source, |admission| {
            let guard = store.lock_cold_initializing_genesis_group(admission, &resources)?;
            let seed = guard.directory_data().0.join(SEED);
            let parked = fixture.directory.path().join("original-seed");
            let original = fs::read(&seed).unwrap();
            fs::rename(&seed, &parked).unwrap();
            assert!(capture.capture_initializing(&guard).is_err());
            assert!(
                capture.selection_failed && capture.selected.is_none() && capture.saved.is_none()
            );
            fs::rename(&parked, &seed).unwrap();
            assert!(
                capture.capture_initializing(&guard).is_err(),
                "terminal attempt must never recapture"
            );
            let saved = independent.capture_initializing(&guard);
            // A different resource loan cannot be silently substituted into this actual guard.
            assert!(saved.is_err());
            assert_eq!(fs::read(&seed).unwrap(), original);
            Ok(())
        })
        .unwrap();
    let new_resources = independent.read_resources();
    store
        .with_cold_v2_genesis_initializing_read(&source, |admission| {
            let guard = store.lock_cold_initializing_genesis_group(admission, &new_resources)?;
            independent.capture_initializing(&guard)?;
            Ok(())
        })
        .unwrap();
}

#[cfg(windows)]
#[test]
fn original_repair_receipt_outlives_no_source_sdk_store_or_scope_sender() {
    let fixture = fixture_at_cut(0xca38, ColdFixtureCut::ShortStage);
    let pool = fixture.pool.clone();
    let saved = {
        let store = fixture.layout.checked_store().unwrap();
        let source = fixture.source.borrow_source_data();
        let mut capture = PreparedColdV2GenesisRepairCapture::prepare(store, &source).unwrap();
        let resources = capture.read_resources();
        store
            .with_cold_v2_genesis_initializing_read(&source, |admission| {
                let guard = store.lock_cold_initializing_genesis_group(admission, &resources)?;
                capture.capture_initializing(&guard)
            })
            .unwrap()
    };
    drop(fixture);
    assert_eq!(
        pool.charged_for_test(),
        cold_capture::COLD_CAPTURE_METADATA_FEE,
        "saved repair metadata must not retain source/SDK/Store payments or graph"
    );
    drop(saved);
    assert_eq!(pool.charged_for_test(), 0);
}

#[cfg(windows)]
#[test]
fn real_source_revocation_after_prefix_comparison_cannot_issue_a_repair_seed() {
    let fixture = fixture_at_cut(0xca39, ColdFixtureCut::ShortStage);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let stage = source.source().capture.target.join(STAGE);
    let prefix = fs::read(&stage).unwrap();
    let mut capture = PreparedColdV2GenesisRepairCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let revoker = store.chat_mount_revoker();
    after_comparers_for_test(Box::new(move || revoker.revoke()));
    store
        .with_cold_v2_genesis_initializing_read(&source, |admission| {
            let guard = store.lock_cold_initializing_genesis_group(admission, &resources)?;
            assert!(capture.capture_initializing(&guard).is_err());
            assert!(capture.selected.is_some() && capture.saved.is_none());
            Ok(())
        })
        .unwrap();
    assert_eq!(fs::read(&stage).unwrap(), prefix);
    assert!(!source.source().capture.target.join(HEAD).exists());
}

#[cfg(windows)]
#[test]
fn original_id3_marker_is_rechecked_after_the_last_source_wait() {
    let fixture = fixture_at_cut(0xca3a, ColdFixtureCut::ShortStage);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let marker = store
        .dir
        .join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(crate::store::critical_namespaces::current_layout::CURRENT_MARKER);
    let parked = fixture.directory.path().join("original-current-marker");
    let bytes = fs::read(&marker).unwrap();
    let mut capture = PreparedColdV2GenesisRepairCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let saved = store
        .with_cold_v2_genesis_initializing_read(&source, |admission| {
            let guard = store.lock_cold_initializing_genesis_group(admission, &resources)?;
            capture.capture_initializing(&guard)
        })
        .unwrap();
    let replaced = marker.clone();
    let original = parked.clone();
    after_source_for_test(Box::new(move || {
        fs::rename(&replaced, &original).unwrap();
        fs::write(&replaced, bytes).unwrap();
    }));
    store
        .with_cold_v2_genesis_initializing_read(&source, |admission| {
            let guard = store.lock_cold_initializing_genesis_group(admission, &resources)?;
            assert!(capture.capture_initializing(&guard).is_err());
            assert!(Arc::ptr_eq(capture.saved.as_ref().unwrap(), &saved));
            Ok(())
        })
        .unwrap();
    fs::remove_file(&marker).unwrap();
    fs::rename(&parked, &marker).unwrap();
    store
        .with_cold_v2_genesis_initializing_read(&source, |admission| {
            let guard = store.lock_cold_initializing_genesis_group(admission, &resources)?;
            let retry = capture.capture_initializing(&guard)?;
            assert!(Arc::ptr_eq(&retry, &saved));
            Ok(())
        })
        .unwrap();
}

#[cfg(windows)]
#[test]
fn source_revocation_during_seed_receipt_allocation_retains_exact_cached_selection() {
    let fixture = fixture_at_cut(0xca3c, ColdFixtureCut::ShortStage);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let mut capture = PreparedColdV2GenesisRepairCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let revoker = store.chat_mount_revoker();
    capture.after_seed_allocation = Some(Box::new(move || revoker.revoke()));
    store
        .with_cold_v2_genesis_initializing_read(&source, |admission| {
            let guard = store.lock_cold_initializing_genesis_group(admission, &resources)?;
            assert!(capture.capture_initializing(&guard).is_err());
            let saved = capture.saved.as_ref().unwrap();
            assert!(Arc::ptr_eq(
                capture.selected.as_ref().unwrap(),
                &saved.files
            ));
            assert!(saved.retains_original(store, &source).is_err());
            assert!(!saved.files.head_path.exists());
            Ok(())
        })
        .unwrap();
}
