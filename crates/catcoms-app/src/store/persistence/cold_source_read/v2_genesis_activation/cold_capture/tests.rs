//! Real remounted row1/row2 artifacts. These are capture/resource tests, not cold ownership,
//! repair, native disclosure or permission to run the restored SDK.
use super::*;
use crate::store::critical_namespaces::current_layout::cold_read::tests::{
    fixture, fixture_at_cut, ColdFixture, ColdFixtureCut,
};
use crate::store::persistence::v2_genesis_activation::{
    ColdV2GenesisActivationSeed, ColdV2GenesisCurrentHead,
};
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn genuine_complete_seed_before_scope_is_neither_loaded_initializing_nor_current() {
    let f = fixture_at_cut(0xc0ff, ColdFixtureCut::CompleteSeedBeforeScope);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    source.validate_original(store).unwrap();
    // A complete journal is not a row or mounted owner. The new no-row consumer must
    // establish its own actual birth protocol; neither historical role may be coerced.
    assert!(store
        .with_cold_v2_genesis_initializing_read(&source, |_| -> Result<(), AppError> {
            panic!("absent row entered loaded Initializing1 reader")
        })
        .is_err());
    assert!(store
        .with_cold_v2_genesis_current_read(&source, |_| -> Result<(), AppError> {
            panic!("absent row entered loaded Current2 reader")
        })
        .is_err());
    source.validate_original(store).unwrap();
    assert!(!source.source().capture.target.join("head.bin").exists());
    assert!(!source.source().capture.target.join("head.stage").exists());
}

#[cfg(windows)]
fn seed_path(f: &ColdFixture, capture: &PreparedColdV2GenesisCapture) -> PathBuf {
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let resources = capture.read_resources();
    store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            let guard = store.lock_cold_current_genesis_group(admission, &resources)?;
            Ok(guard.directory_data().0.join(SEED))
        })
        .unwrap()
}

#[cfg(windows)]
#[test]
fn cold_seed_absent_capture_keeps_same_selection_without_inventing_an_activation_id() {
    let f = fixture(0xcd50, false);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let mut capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let seed = seed_path(&f, &capture);
    // Hostile/missing-seed fixture only. This does not claim the actual settlement writer
    // has run: the separate owning tests also require the new real covering scope write.
    fs::remove_file(&seed).unwrap();
    let head = read_current(&f, &mut capture).unwrap();
    assert!(head.requires_scope_covering());
    assert!(head.activation_id_data().is_none());
    assert!(Arc::ptr_eq(&head, &read_current(&f, &mut capture).unwrap()));
    head.retains_original(store, &source).unwrap();
    assert!(!seed.exists());
}

#[cfg(windows)]
#[test]
fn cold_seed_absent_selection_rejects_present_seed_failure_and_preserves_unknown_work() {
    let f = fixture(0xcd51, false);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let mut present = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let seed = seed_path(&f, &present);
    let original = fs::read(&seed).unwrap();
    let mut corrupted = original.clone();
    *corrupted.last_mut().unwrap() ^= 1;
    fs::write(&seed, corrupted).unwrap();
    assert!(read_current(&f, &mut present).is_err());
    assert!(matches!(
        present.captured.as_ref().unwrap().seed,
        ColdSeedSelection::Present(_)
    ));
    fs::remove_file(&seed).unwrap();
    assert!(read_current(&f, &mut present).is_err());
    let mut absent = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let saved = read_current(&f, &mut absent).unwrap();
    let target = &source.source().capture.target;
    for path in [
        seed.clone(),
        target.join("head.stage"),
        target.join("operation.pending"),
        seed.parent().unwrap().join("private-head.bin"),
    ] {
        // Even equal seed bytes must not change this already-selected Absent role.
        fs::write(&path, &original).unwrap();
        assert!(read_current(&f, &mut absent).is_err());
        assert_eq!(
            fs::read(&path).unwrap(),
            original,
            "never clean unknown work"
        );
        assert!(Arc::ptr_eq(&saved, absent.current.as_ref().unwrap()));
        fs::remove_file(&path).unwrap();
        assert!(Arc::ptr_eq(&saved, &read_current(&f, &mut absent).unwrap()));
    }
    // The independent pin/head decoder must authenticate the actual original objects too.
    for path in [&saved.closure.pin_path, &saved.closure.head_path] {
        let original = fs::read(path).unwrap();
        let mut corrupted = original.clone();
        *corrupted.last_mut().unwrap() ^= 1;
        fs::write(path, corrupted).unwrap();
        assert!(read_current(&f, &mut absent).is_err());
        fs::write(path, original).unwrap();
        assert!(Arc::ptr_eq(&saved, &read_current(&f, &mut absent).unwrap()));
    }
}

fn read_current(
    f: &ColdFixture,
    capture: &mut PreparedColdV2GenesisCapture,
) -> Result<Arc<ColdV2GenesisCurrentHead>, AppError> {
    let store = f.layout.checked_store()?;
    let source = f.source.borrow_source_data();
    let resources = capture.read_resources();
    store.with_cold_v2_genesis_current_read(&source, |admission| {
        let guard = store.lock_cold_current_genesis_group(admission, &resources)?;
        capture.capture_current(&guard)
    })
}
fn read_initializing(
    f: &ColdFixture,
    capture: &mut PreparedColdV2GenesisCapture,
) -> Result<Arc<ColdV2GenesisActivationSeed>, AppError> {
    let store = f.layout.checked_store()?;
    let source = f.source.borrow_source_data();
    let resources = capture.read_resources();
    store.with_cold_v2_genesis_initializing_read(&source, |admission| {
        let guard = store.lock_cold_initializing_genesis_group(admission, &resources)?;
        capture.capture_initializing(&guard)
    })
}

#[test]
fn complete_cold_roles_return_original_typed_receipts_and_exact_artifact_ids() {
    for initializing in [false, true] {
        let f = fixture(0xcd01 + u64::from(initializing), initializing);
        let store = f.layout.checked_store().unwrap();
        let source = f.source.borrow_source_data();
        let baseline = f.pool.charged_for_test();
        let mut capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
        assert_eq!(
            f.pool.charged_for_test(),
            baseline + COLD_CAPTURE_METADATA_FEE
        );
        macro_rules! check {
            ($saved:expr) => {{
                let saved = $saved;
                assert_eq!(saved.expectation_data().source_id, source.data().source);
                assert_eq!(&saved.expectation_data().context, source.data().context);
                assert_eq!(saved.scope_basis_data(), &saved.closure.basis);
                assert_eq!(
                    saved.activation_id_data(),
                    Some(V2ActivationSeedId::of(
                        &saved.closure.seed.present().unwrap().frame
                    ))
                );
                assert_eq!(
                    saved.head_id_data(),
                    V2GenesisHeadId::of(&saved.closure.head.frame)
                );
                assert_eq!(
                    saved.pin_id_data(),
                    V2GroupPinId::of(&saved.closure.pin.frame)
                );
                assert_eq!(saved.directory_data().2, saved.closure.group_id);
                saved.retains_original(store, &source).unwrap();
                saved
            }};
        }
        if initializing {
            let saved = check!(read_initializing(&f, &mut capture).unwrap());
            let resources = capture.read_resources();
            store
                .with_cold_v2_genesis_initializing_read(&source, |admission| {
                    let guard =
                        store.lock_cold_initializing_genesis_group(admission, &resources)?;
                    saved.validate_under_archive_root(store, &source, guard.archive_root())
                })
                .unwrap();
        } else {
            let saved = check!(read_current(&f, &mut capture).unwrap());
            let resources = saved.read_resources();
            resources
                .require_same_original(&capture.read_resources())
                .unwrap();
            store
                .with_cold_v2_genesis_current_read(&source, |admission| {
                    let guard = store.lock_cold_current_genesis_group(admission, &resources)?;
                    saved.validate_under_archive_root(store, &source, guard.archive_root())
                })
                .unwrap();
        }
        assert_eq!(
            f.pool.charged_for_test(),
            baseline + COLD_CAPTURE_METADATA_FEE
        );
        drop(capture);
        assert_eq!(f.pool.charged_for_test(), baseline);
    }
}

#[test]
fn actual_cold_receipt_keeps_only_its_metadata_payment_after_source_and_store_drop() {
    let f = fixture(0xcd03, false);
    let pool = f.pool.clone();
    let store = f.layout.checked_store().unwrap();
    let mut capture =
        PreparedColdV2GenesisCapture::prepare(store, &f.source.borrow_source_data()).unwrap();
    let saved = read_current(&f, &mut capture).unwrap();
    drop(capture);
    drop(f);
    assert!(saved.closure.resources.body.negative.is_cancelled());
    assert_eq!(
        pool.charged_for_test(),
        COLD_CAPTURE_METADATA_FEE,
        "receipt retains no source/SDK/global Store payment or graph"
    );
    drop(saved);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn cancellation_during_receipt_allocation_caches_selection_but_refuses_handoff() {
    for initializing in [false, true] {
        let f = fixture(0xcd10 + u64::from(initializing), initializing);
        let store = f.layout.checked_store().unwrap();
        let mut capture =
            PreparedColdV2GenesisCapture::prepare(store, &f.source.borrow_source_data()).unwrap();
        let revoker = store.chat_mount_revoker();
        capture.after_receipt_allocation = Some(Box::new(move || revoker.revoke()));
        if initializing {
            assert!(read_initializing(&f, &mut capture).is_err());
            assert!(capture.initializing.is_some());
        } else {
            assert!(read_current(&f, &mut capture).is_err());
            assert!(capture.current.is_some());
        }
        let selected = capture.captured.as_ref().unwrap();
        assert_eq!(
            selected.resources.body.expected.source_id,
            f.source.data().source
        );
        assert!(capture
            .resources
            .require_original_negative(store, &f.source.borrow_source_data())
            .is_err());
    }
}

#[test]
fn final_source_wait_cannot_reuse_an_earlier_layout_marker_proof() {
    let f = fixture(0xcd20, false);
    let store = f.layout.checked_store().unwrap();
    let mut capture =
        PreparedColdV2GenesisCapture::prepare(store, &f.source.borrow_source_data()).unwrap();
    let saved = read_current(&f, &mut capture).unwrap();
    let marker = store
        .dir
        .join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(crate::store::critical_namespaces::current_layout::CURRENT_MARKER);
    let bytes = fs::read(&marker).unwrap();
    let seed = fs::read(&saved.closure.seed_path).unwrap();
    for replacement in [false, true] {
        let displaced = f.directory.path().join("held-cold-capture-marker");
        let changed = marker.clone();
        let held = displaced.clone();
        let original = bytes.clone();
        let reached = Arc::new(AtomicBool::new(false));
        let observed = reached.clone();
        AFTER_FINAL_SOURCE.with(|slot| {
            assert!(slot
                .replace(Some(Box::new(move || {
                    if replacement {
                        fs::rename(&changed, &held).unwrap();
                        fs::write(&changed, original).unwrap();
                    } else {
                        let mut corrupted = original;
                        *corrupted.last_mut().unwrap() ^= 1;
                        fs::write(&changed, corrupted).unwrap();
                    }
                    observed.store(true, Ordering::SeqCst);
                })))
                .is_none());
        });
        assert!(read_current(&f, &mut capture).is_err());
        assert!(reached.load(Ordering::SeqCst));
        assert!(Arc::ptr_eq(&saved, capture.current.as_ref().unwrap()));
        assert_eq!(fs::read(&saved.closure.seed_path).unwrap(), seed);
        if replacement {
            fs::remove_file(&marker).unwrap();
            fs::rename(&displaced, &marker).unwrap();
        } else {
            fs::write(&marker, &bytes).unwrap();
        }
        assert!(Arc::ptr_eq(
            &saved,
            &read_current(&f, &mut capture).unwrap()
        ));
    }
}

#[test]
fn equal_pin_replacement_after_receipt_allocation_cannot_be_adopted_on_retry() {
    let f = fixture(0xcd30, false);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let mut capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let pin = store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            let guard = store.lock_cold_current_genesis_group(admission, &resources)?;
            Ok(guard.directory_data().0.join("pin.bin"))
        })
        .unwrap();
    let bytes = fs::read(&pin).unwrap();
    let displaced = f.directory.path().join("held-original-pin");
    let changed = pin.clone();
    let held = displaced.clone();
    capture.after_receipt_allocation = Some(Box::new(move || {
        fs::rename(&changed, &held).unwrap();
        fs::write(&changed, bytes).unwrap();
    }));
    assert!(read_current(&f, &mut capture).is_err());
    let original = capture.current.as_ref().unwrap().clone();
    assert!(
        read_current(&f, &mut capture).is_err(),
        "equal replacement stays foreign"
    );
    fs::remove_file(&pin).unwrap();
    fs::rename(&displaced, &pin).unwrap();
    assert!(Arc::ptr_eq(
        &original,
        &read_current(&f, &mut capture).unwrap()
    ));
}

#[test]
fn incomplete_read_attempt_stays_closed_after_files_are_restored() {
    use rand_core::RngCore;
    let mut f = fixture(0xcd40, false);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let mut capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let pin = store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            let guard = store.lock_cold_current_genesis_group(admission, &resources)?;
            Ok(guard.directory_data().0.join("pin.bin"))
        })
        .unwrap();
    let displaced = f.directory.path().join("held-incomplete-pin");
    fs::rename(&pin, &displaced).unwrap();
    assert!(read_current(&f, &mut capture).is_err());
    assert!(capture.partial_read_failed && capture.captured.is_none());
    fs::rename(&displaced, &pin).unwrap();
    assert!(read_current(&f, &mut capture).is_err());
    // This is a new local paid read, never a renewal of the closed capture or source-write
    // grant. Existing partial data is not deleted, repaired, or treated as an empty history.
    let mut new_read = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    read_current(&f, &mut new_read).unwrap();
    // A complete genesis reader cannot adopt an unknown pending head as a committed head
    // or erase it to make the directory appear empty. Its origin/content is untrusted.
    let stage = source.source().capture.target.join("head.stage");
    let mut untrusted_stage = [0; 64];
    f.rng.fill_bytes(&mut untrusted_stage);
    fs::write(&stage, untrusted_stage).unwrap();
    let mut with_stage = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    assert!(read_current(&f, &mut with_stage).is_err());
    assert_eq!(fs::read(&stage).unwrap(), untrusted_stage);
}
