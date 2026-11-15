//! Genuine SourceSaved -> complete activation seed -> NO row -> full drain/new mount/
//! NEW paid source+SDK. These tests never manufacture a receipt, watch, row or scope permit.
use super::*;
#[cfg(windows)]
use crate::store::critical_namespaces::current_layout::cold_read::tests::ColdFixture;
use crate::store::critical_namespaces::current_layout::cold_read::tests::{
    fixture_at_cut, ColdFixtureCut,
};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(windows)]
fn capture(
    fixture: &ColdFixture,
) -> (
    PreparedColdV2PreInitializingCapture,
    Arc<ColdV2PreInitializingSeed>,
) {
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let mut capture = PreparedColdV2PreInitializingCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let seed = store
        .with_cold_v2_genesis_vacant_read(&source, |admission| {
            let guard = store.lock_cold_vacant_genesis_group(admission, &resources)?;
            capture.capture_vacant(&guard)
        })
        .unwrap();
    (capture, seed)
}

#[cfg(windows)]
#[test]
fn complete_original_seed_before_row_retries_exact_objects_and_retains_original_ciphertexts() {
    let fixture = fixture_at_cut(0xcad0, ColdFixtureCut::CompleteSeedBeforeScope);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let before = fixture.pool.charged_for_test();
    let mut capture = PreparedColdV2PreInitializingCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    assert_eq!(
        fixture.pool.charged_for_test(),
        before + cold_capture::COLD_CAPTURE_METADATA_FEE
    );
    store
        .with_cold_v2_genesis_vacant_read(&source, |admission| {
            let guard = store.lock_cold_vacant_genesis_group(admission, &resources)?;
            let seed = capture.capture_vacant(&guard)?;
            let again = capture.capture_vacant(&guard)?;
            assert!(Arc::ptr_eq(&seed, &again));
            assert!(Arc::ptr_eq(&seed.files, capture.selected.as_ref().unwrap()));
            assert!(seed.vacancy_basis_data() == guard.vacancy_basis_data());
            assert!(seed.expectation_data().context == *source.data().context);
            assert!(seed.expectation_data().source_id == source.data().source);
            let original = seed.original_files()?;
            assert!(fs::read(original.seed_path).unwrap() == original.seed.frame.as_slice());
            let key = Zeroizing::new(store.keys.db_key()?);
            let plain = codec::open_activation_seed_data(&key, &original.seed.frame)?;
            let decoded = codec::parse_activation_seed_data(&key, seed.expectation_data(), &plain)?;
            assert!(decoded.pin_frame == original.pin_frame);
            assert!(decoded.head_frame == original.head_frame);
            for path in [original.pin_path, original.head_path, original.stage_path] {
                assert!(!path.exists());
            }
            Ok(())
        })
        .unwrap();
    // Actual vacancy remains available: the physical capture inserted no row or owner.
    store
        .with_cold_v2_genesis_vacant_read(&source, |_| Ok(()))
        .unwrap();
    drop(resources);
    drop(capture);
    assert_eq!(fixture.pool.charged_for_test(), before);
}

#[cfg(windows)]
#[test]
fn each_existing_target_or_private_extra_is_preserved_and_never_a_pre_row_repair() {
    let fixture = fixture_at_cut(0xcad1, ColdFixtureCut::CompleteSeedBeforeScope);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let (mut capture, seed) = capture(&fixture);
    let resources = capture.read_resources();
    let journal = fs::read(&seed.files.seed_path).unwrap();
    let paths = [
        seed.files.pin_path.clone(),
        seed.files.head_path.clone(),
        seed.files.stage_path.clone(),
        seed.files.group_path.join("unknown-private.bin"),
        source.source().capture.target.join("later-operation.bin"),
    ];
    for path in paths {
        fs::write(&path, b"unexplained private bytes").unwrap();
        let result = store.with_cold_v2_genesis_vacant_read(&source, |admission| {
            let guard = store.lock_cold_vacant_genesis_group(admission, &resources)?;
            capture.capture_vacant(&guard)
        });
        assert!(result.is_err());
        assert!(fs::read(&path).unwrap() == b"unexplained private bytes");
        assert!(fs::read(&seed.files.seed_path).unwrap() == journal);
        assert!(Arc::ptr_eq(capture.saved.as_ref().unwrap(), &seed));
        // Test-only restoration proves retry uses the SAME original seed, not a recapture.
        fs::remove_file(&path).unwrap();
        let same = store
            .with_cold_v2_genesis_vacant_read(&source, |admission| {
                let guard = store.lock_cold_vacant_genesis_group(admission, &resources)?;
                capture.capture_vacant(&guard)
            })
            .unwrap();
        assert!(Arc::ptr_eq(&same, &seed));
    }
}

#[cfg(windows)]
#[test]
fn missing_or_changed_first_seed_never_falls_back_or_reselects_an_equal_replacement() {
    let fixture = fixture_at_cut(0xcad2, ColdFixtureCut::CompleteSeedBeforeScope);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let (mut captured, seed) = capture(&fixture);
    let resources = captured.read_resources();
    let path = seed.files.seed_path.clone();
    let parked = fixture.directory.path().join("original-pre-seed");
    let original = fs::read(&path).unwrap();
    fs::rename(&path, &parked).unwrap();
    for replacement in [
        None,
        Some(original[..original.len() - 1].to_vec()),
        Some(original.clone()),
    ] {
        if let Some(bytes) = replacement {
            fs::write(&path, bytes).unwrap();
        }
        assert!(store
            .with_cold_v2_genesis_vacant_read(&source, |admission| {
                let guard = store.lock_cold_vacant_genesis_group(admission, &resources)?;
                captured.capture_vacant(&guard)
            })
            .is_err());
        assert!(Arc::ptr_eq(captured.saved.as_ref().unwrap(), &seed));
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
    }
    fs::rename(&parked, &path).unwrap();
    let retry = store
        .with_cold_v2_genesis_vacant_read(&source, |admission| {
            let guard = store.lock_cold_vacant_genesis_group(admission, &resources)?;
            captured.capture_vacant(&guard)
        })
        .unwrap();
    assert!(Arc::ptr_eq(&retry, &seed));
}

#[cfg(windows)]
#[test]
fn incomplete_initial_selection_poison_and_wrong_resource_loan_do_not_create_receipts() {
    let fixture = fixture_at_cut(0xcad3, ColdFixtureCut::CompleteSeedBeforeScope);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let mut failed = PreparedColdV2PreInitializingCapture::prepare(store, &source).unwrap();
    let mut independent = PreparedColdV2PreInitializingCapture::prepare(store, &source).unwrap();
    let resources = failed.read_resources();
    let other_resources = independent.read_resources();
    store
        .with_cold_v2_genesis_vacant_read(&source, |admission| {
            let guard = store.lock_cold_vacant_genesis_group(admission, &other_resources)?;
            assert!(failed.capture_vacant(&guard).is_err());
            assert!(failed.selected.is_none() && failed.saved.is_none());
            Ok(())
        })
        .unwrap();
    let mut journal = None;
    store
        .with_cold_v2_genesis_vacant_read(&source, |admission| {
            let guard = store.lock_cold_vacant_genesis_group(admission, &resources)?;
            let path = guard.directory_data().0.join(SEED);
            let bytes = fs::read(&path).unwrap();
            journal = Some((path.clone(), bytes.clone()));
            fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
            assert!(failed.capture_vacant(&guard).is_err());
            // The complete selection may exist for auth failure; it is then immutable and must
            // reject a changed frame. A short original object never grants a recovery fallback.
            fs::write(&path, bytes).unwrap();
            assert!(failed.capture_vacant(&guard).is_err());
            assert!(failed.saved.is_none());
            Ok(())
        })
        .unwrap();
    let original = independent.read_resources();
    let saved = store
        .with_cold_v2_genesis_vacant_read(&source, |admission| {
            let guard = store.lock_cold_vacant_genesis_group(admission, &original)?;
            independent.capture_vacant(&guard)
        })
        .unwrap();
    assert!(saved.expectation_data().source_id == source.data().source);
    let (path, bytes) = journal.unwrap();
    assert!(fs::read(path).unwrap() == bytes);
}

#[cfg(windows)]
#[test]
fn pre_metadata_pressure_refuses_before_capture_and_retries_same_source_after_refund() {
    const CAPACITY: usize = 1024 * 1024 * 1024;
    let fixture = fixture_at_cut(0xcad4, ColdFixtureCut::CompleteSeedBeforeScope);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let charged = fixture.pool.charged_for_test();
    let pressure = fixture
        .pool
        .try_reserve(CAPACITY - charged - (cold_capture::COLD_CAPTURE_METADATA_FEE - 1))
        .unwrap();
    let held = fixture.pool.charged_for_test();
    assert!(PreparedColdV2PreInitializingCapture::prepare(store, &source).is_err());
    assert_eq!(fixture.pool.charged_for_test(), held);
    drop(pressure);
    let (capture, seed) = capture(&fixture);
    assert!(seed.expectation_data().source_id == source.data().source);
    drop(capture);
    drop(seed);
    assert_eq!(fixture.pool.charged_for_test(), charged);
}

#[cfg(windows)]
#[test]
fn final_source_marker_and_receipt_edges_recheck_real_negative_fences() {
    // Real revocation AFTER all initial candidate comparers: no receipt may escape.
    let fixture = fixture_at_cut(0xcad5, ColdFixtureCut::CompleteSeedBeforeScope);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let mut capture = PreparedColdV2PreInitializingCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let revoke = store.chat_mount_revoker();
    EDGE_HOOK.with(|slot| {
        *slot.borrow_mut() = Some((TestEdge::Comparers, Box::new(move || revoke.revoke())))
    });
    assert!(store
        .with_cold_v2_genesis_vacant_read(&source, |admission| {
            let guard = store.lock_cold_vacant_genesis_group(admission, &resources)?;
            capture.capture_vacant(&guard)
        })
        .is_err());
    assert!(capture.saved.is_none());
    // A separate actual source/mount exercises final ID3 replacement after source reads.
    let f = fixture_at_cut(0xcad6, ColdFixtureCut::CompleteSeedBeforeScope);
    let s = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let mut capture = PreparedColdV2PreInitializingCapture::prepare(s, &source).unwrap();
    let resources = capture.read_resources();
    let marker = s
        .dir
        .join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(crate::store::critical_namespaces::current_layout::CURRENT_MARKER);
    let parked = f.directory.path().join("original-pre-current-marker");
    let bytes = fs::read(&marker).unwrap();
    let replacement = marker.clone();
    let original = parked.clone();
    EDGE_HOOK.with(|slot| {
        *slot.borrow_mut() = Some((
            TestEdge::Source,
            Box::new(move || {
                fs::rename(&replacement, &original).unwrap();
                fs::write(&replacement, bytes).unwrap();
            }),
        ))
    });
    assert!(s
        .with_cold_v2_genesis_vacant_read(&source, |admission| {
            let guard = s.lock_cold_vacant_genesis_group(admission, &resources)?;
            capture.capture_vacant(&guard)
        })
        .is_err());
    assert!(capture.saved.is_none());
    fs::remove_file(&marker).unwrap();
    fs::rename(&parked, &marker).unwrap();
    let revoke = s.chat_mount_revoker();
    EDGE_HOOK.with(|slot| {
        *slot.borrow_mut() = Some((TestEdge::Receipt, Box::new(move || revoke.revoke())))
    });
    assert!(s
        .with_cold_v2_genesis_vacant_read(&source, |admission| {
            let guard = s.lock_cold_vacant_genesis_group(admission, &resources)?;
            capture.capture_vacant(&guard)
        })
        .is_err());
    let retained = capture.saved.as_ref().unwrap();
    assert!(Arc::ptr_eq(
        &retained.files,
        capture.selected.as_ref().unwrap()
    ));
    assert!(retained.retains_original(s, &source).is_err());
}

#[cfg(windows)]
#[test]
fn original_pre_receipt_and_exact_fee_tail_keep_no_source_sdk_or_store_alive() {
    let fixture = fixture_at_cut(0xcad7, ColdFixtureCut::CompleteSeedBeforeScope);
    let pool = fixture.pool.clone();
    let notice = Arc::new(AtomicBool::new(false));
    let saved = {
        let store = fixture.layout.checked_store().unwrap();
        let source = fixture.source.borrow_source_data();
        let mut capture = PreparedColdV2PreInitializingCapture::prepare(store, &source).unwrap();
        capture
            .observe_metadata_drop_for_test(notice.clone())
            .unwrap();
        let resources = capture.read_resources();
        store
            .with_cold_v2_genesis_vacant_read(&source, |admission| {
                let guard = store.lock_cold_vacant_genesis_group(admission, &resources)?;
                capture.capture_vacant(&guard)
            })
            .unwrap()
    };
    let ColdFixture {
        layout,
        source,
        directory,
        ..
    } = fixture;
    drop(source);
    drop(layout);
    assert!(!notice.load(Ordering::SeqCst));
    assert_eq!(
        pool.charged_for_test(),
        cold_capture::COLD_CAPTURE_METADATA_FEE
    );
    drop(saved);
    assert!(notice.load(Ordering::SeqCst));
    assert_eq!(pool.charged_for_test(), 0);
    drop(directory);
}

#[cfg(not(any(windows, target_os = "linux")))]
#[test]
fn unsupported_native_inventory_profile_has_no_implicit_portable_fallback() {
    // The real portable producer supplies the complete journal and no row. Calling actual
    // preparation pins the pre-debit platform gate, not just its standalone predicate.
    let fixture = fixture_at_cut(0xcad9, ColdFixtureCut::CompleteSeedBeforeScope);
    let store = fixture.layout.checked_store().unwrap();
    let source = fixture.source.borrow_source_data();
    let before = fixture.pool.charged_for_test();
    let source_id = source.data().source;
    let suffix = {
        let root = store.archive_bootstrap_root.lock().unwrap();
        let root = root.as_ref().unwrap();
        crate::store::critical_namespaces::current_group::group_suffix(
            &root.descriptor.vault,
            &root.descriptor.root,
            &fixture.group,
        )
        .unwrap()
    };
    let group = source
        .source()
        .capture
        .root
        .join(format!("g-{}", std::str::from_utf8(&suffix).unwrap()));
    let path = group.join(SEED);
    let original = fs::read(&path).unwrap();
    assert!(PreparedColdV2PreInitializingCapture::prepare(store, &source).is_err());
    assert_eq!(fixture.pool.charged_for_test(), before);
    assert!(source.data().source == source_id);
    assert!(fs::read(path).unwrap() == original);
    assert!(!group.join(PIN).exists());
    assert!(!source.source().capture.target.join(HEAD).exists());
    assert!(!source.source().capture.target.join(STAGE).exists());
    source.validate_original(store).unwrap();
    // Born cannot be genuinely constructed under this unsupported profile; its identical
    // first gate is source-inspected, not claimed as positive/negative executed coverage.
}

#[cfg(windows)]
#[test]
fn actual_born_barrier_mints_actual_basis_and_normal_seed_owns_only_second_metadata_payment() {
    let mut fixture = fixture_at_cut(0xcad8, ColdFixtureCut::CompleteSeedBeforeScope);
    let pool = fixture.pool.clone();
    let old_released = Arc::new(AtomicBool::new(false));
    let normal = {
        let store = fixture.layout.checked_store().unwrap();
        let source = fixture.source.borrow_source_data();
        let mut pre_capture =
            PreparedColdV2PreInitializingCapture::prepare(store, &source).unwrap();
        pre_capture
            .observe_metadata_drop_for_test(old_released.clone())
            .unwrap();
        let old_resources = pre_capture.read_resources();
        let pre = store
            .with_cold_v2_genesis_vacant_read(&source, |admission| {
                let guard = store.lock_cold_vacant_genesis_group(admission, &old_resources)?;
                pre_capture.capture_vacant(&guard)
            })
            .unwrap();
        let journal = fs::read(&pre.files.seed_path).unwrap();
        let task = PreparedV2GenesisGroupNamespace::from_cold_pre_initializing(
            store,
            &source,
            pre.clone(),
        )
        .unwrap();
        let mut scope = None;
        store
            .prepare_cold_v2_genesis_birth(&source, &task, pre.clone(), &mut scope)
            .unwrap();
        let mut born_capture =
            PreparedBornV2GenesisRepairCapture::prepare(store, &source, pre.clone()).unwrap();
        let new_resources = born_capture.read_resources();
        assert!(new_resources.require_same_original(&old_resources).is_err());
        let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
        let marker = store
            .dir
            .join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
            .join(crate::store::critical_namespaces::current_layout::CURRENT_MARKER);
        let marker_bytes = fs::read(&marker).unwrap();
        let changed_marker = marker.clone();
        let mut invalid_marker = marker_bytes.clone();
        invalid_marker[0] ^= 1;
        let late_marker_change: Box<dyn FnOnce()> = Box::new(move || {
            fs::write(&changed_marker, invalid_marker).unwrap();
        });
        let mut observed_basis = None;
        store
            .save_born_v2_genesis_row1(
                scope.as_mut().unwrap(),
                &source,
                &task,
                &buffers,
                &mut fixture.rng,
                |admission| {
                    // Only this genuine post-file/parent-barrier role can supply row1 Basis.
                    let guard =
                        store.lock_durable_born_v2_genesis_group(admission, &new_resources)?;
                    // The normal repair comparer runs AFTER the independent body and
                    // temporary Born receipt have been cached. Its final ID3 failure must
                    // retain those same objects for retry, not recopy/reselect another seed.
                    initializing_repair::after_source_for_test(late_marker_change);
                    assert!(born_capture.capture_born(&guard).is_err());
                    let cached = born_capture.saved.as_ref().unwrap().clone();
                    let cached_repair = born_capture.repair.as_ref().unwrap().clone();
                    fs::write(&marker, &marker_bytes).unwrap();
                    let first = born_capture.capture_born(&guard)?;
                    let retry = born_capture.capture_born(&guard)?;
                    assert!(Arc::ptr_eq(&first, &retry));
                    assert!(Arc::ptr_eq(&first, &cached));
                    assert!(Arc::ptr_eq(first.repair_seed(), &cached_repair));
                    assert!(Arc::ptr_eq(first.pre_initializing_seed(), &pre));
                    assert!(first.scope_basis_data() == guard.scope_basis_data());
                    observed_basis = Some(guard.scope_basis_data().clone());
                    first
                        .repair_seed()
                        .read_resources()
                        .require_same_original(&new_resources)?;
                    assert!(first
                        .repair_seed()
                        .read_resources()
                        .require_same_original(&old_resources)
                        .is_err());
                    // A legitimate row1 now exists. The pre receipt's PHYSICAL proof must
                    // still work, without incorrectly requiring Runtime owner absence.
                    pre.validate_under_archive_root(store, &source, guard.archive_root())?;
                    first.retains_original(store, &source, &task, guard.owner_identity())?;
                    assert!(fs::read(&pre.files.seed_path).unwrap() == journal);
                    assert!(!pre.files.pin_path.exists() && !pre.files.head_path.exists());
                    Ok(first)
                },
            )
            .unwrap();
        drop(buffers); // the real134MiB temporary debit drains OUTSIDE all writers
        assert!(store
            .with_cold_v2_genesis_vacant_read(&source, |_| Ok(()))
            .is_err());
        let saved = scope.as_ref().unwrap().born_repair_seed().unwrap();
        let repair = saved.repair_seed().clone();
        assert!(Some(repair.scope_basis_data()) == observed_basis.as_ref());
        assert!(!old_released.load(Ordering::SeqCst));
        repair
        // Original scope/task/pre/born locals drain here; Store may still hold their inert
        // registered debt. This physical test does not claim the later live handoff ran.
    };
    let ColdFixture {
        layout,
        source,
        directory,
        ..
    } = fixture;
    drop(source);
    drop(layout);
    assert!(old_released.load(Ordering::SeqCst));
    assert_eq!(
        pool.charged_for_test(),
        cold_capture::COLD_CAPTURE_METADATA_FEE,
        "normal repair retains its NEW payment, no original pre/born/source/SDK/Store"
    );
    drop(normal);
    assert_eq!(pool.charged_for_test(), 0);
    drop(directory);
}
