//! Real SourceSaved -> warm head/scope -> complete drain/remount -> NEW paid cold read.
//! No row, watch, directory receipt or cold admission is manufactured by these fixtures.
use super::*;
use crate::store::critical_namespaces::{
    archive_bootstrap as archive, archive_current_namespace_codec as layout_codec,
    read_pool::CriticalCatalogReadPool,
};
use crate::store::owner_evidence::canonical::v2_registration::PreparedV2WriterRegistration;
use crate::store::persistence::cold_group_read_entries_for_test;
use crate::store::persistence::v2_genesis_activation::{
    PreparedColdV2GenesisCapture, PreparedV2GenesisActivation, COLD_CAPTURE_METADATA_FEE,
};
use crate::store::{
    ArchiveFounderBegin, ArchiveFounderInputs, ArchiveFounderPolicy, GenesisSeedBudget,
    RetainedBootstrapGenesisSeed,
};
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use catcoms_sync::owner_v2::FounderSnapshotLimits;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const SECRET: &[u8] = b"genuine task-free cold group reads";
const CAPACITY: usize = 1024 * 1024 * 1024;

/// A genuine private test resource owner. Returning it issues no production proof; every
/// test still calls the actual reader/SDK, Runtime admission and physical guard constructors.
pub(in crate::store) struct ColdFixture {
    pub(in crate::store) layout: V2CurrentLayoutStore,
    pub(in crate::store) source: RetainedBootstrapGenesisSeed,
    pub(in crate::store) pool: CriticalCatalogReadPool,
    pub(in crate::store) group: Vec<u8>,
    pub(in crate::store) signer: [u8; 32],
    pub(in crate::store) rng: ChaCha20Rng,
    // Root's actual cold artifact regressions may borrow the fixture directory; ownership
    // stays LAST so every original File/resource above drains before temporary cleanup.
    pub(in crate::store) directory: tempfile::TempDir,
}
fn archive_policy(pool: &CriticalCatalogReadPool) -> ArchiveBootstrapCatalogPolicy {
    ArchiveBootstrapCatalogPolicy::new(pool.clone(), 4 * archive::MAX_SEED_FRAMED)
        .unwrap()
        .with_bootstrap_source_data_read()
}
fn layout_policy(pool: &CriticalCatalogReadPool) -> V2CurrentLayoutPolicy {
    V2CurrentLayoutPolicy::new(
        archive_policy(pool),
        CanonicalFilenameInventoryPolicy::new(32, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * layout_codec::MAX_DECLARATION_FRAME,
    )
    .unwrap()
}
fn sdk_budget() -> GenesisSeedBudget {
    GenesisSeedBudget::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap()
}

/// Root's cold-capture regressions may reuse this exact real producer. `initializing` only
/// chooses the existing injected scope flush failure AFTER actual save_head, never a row
/// constructor. Cold role selection must still be made by the actual Runtime on remount.
pub(in crate::store) fn fixture(number: u64, initializing: bool) -> ColdFixture {
    fixture_at_cut(
        number,
        if initializing {
            ColdFixtureCut::CompleteInitializing
        } else {
            ColdFixtureCut::Current
        },
    )
}

/// Cuts in the ACTUAL original warm producer, not post-hoc file truncation or invented
/// rows. Root's partial repair tests require a new mount/read/SDK after these owners drain.
#[derive(Clone, Copy)]
pub(in crate::store) enum ColdFixtureCut {
    Current,
    CompleteInitializing,
    BeforePin,
    EmptyPin,
    ShortPin,
    EmptyStage,
    ShortStage,
    CompleteStage,
}

pub(in crate::store) fn fixture_at_cut(number: u64, cut: ColdFixtureCut) -> ColdFixture {
    let path = tempfile::tempdir().unwrap();
    let pool = CriticalCatalogReadPool::new(CAPACITY).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(number);
    drop(
        ServerStore::open(path.path(), SECRET, &mut rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap(),
    );
    let mut opening = crate::store::PreparedArchiveBootstrapStore::open(
        path.path(),
        SECRET,
        &mut rng,
        archive_policy(&pool),
    )
    .unwrap();
    opening.retry(&mut rng).unwrap();
    let first = opening.into_store().unwrap();
    let device = MlsDevice::generate().unwrap();
    let signer = device.public_key_bytes().try_into().unwrap();
    let limits = FounderSnapshotLimits::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
    )
    .unwrap();
    let mut creator = match first
        .prepare_archive_founder(
            [0x45; 32],
            "genuine cold read origin",
            ArchiveFounderPolicy::new(limits, 3, archive::founder_codec::PERSISTENT_CONTENT_BYTES)
                .unwrap(),
            ArchiveFounderInputs::new(
                Hub::new().join(PeerId::from_u64(number)),
                device,
                ChaCha20Rng::seed_from_u64(number),
                Box::new(ManualClock::new(10)),
            ),
        )
        .unwrap()
    {
        ArchiveFounderBegin::Original(creator) => creator,
        _ => panic!("actual original source creation required"),
    };
    creator.advance(&first, &mut rng).unwrap();
    let group = creator.group_data().unwrap().to_vec();
    let mut plan = creator.into_ram_plan(&first).unwrap();
    plan.save_founding_intent(&first, &mut rng).unwrap();
    let mut original = plan.into_source_preparation(&first).unwrap();
    original.save_source_terminal(&first, &mut rng).unwrap();
    drop(original);
    drop(first);

    let mut opening =
        PreparedV2CurrentLayoutStore::open(path.path(), SECRET, &mut rng, layout_policy(&pool))
            .unwrap();
    opening.retry(&mut rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    {
        let store = layout.checked_store().unwrap();
        let read = layout
            .read_saved_bootstrap_source_data(&group, signer)
            .unwrap();
        let source = store.prepare_genesis_seed(read, sdk_budget()).unwrap();
        let source_ref = source.borrow_source_data();
        let mut task = layout.prepare_genesis_group_namespace(&source_ref).unwrap();
        task.retry_directory(store, &source_ref).unwrap();
        let mut activation =
            PreparedV2GenesisActivation::prepare(store, &source_ref, &mut rng).unwrap();
        let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
        let saved_seed = store
            .with_vacant_v2_genesis_admission(&source_ref, |admission| {
                activation.save_seed(&task.lock_vacant(admission)?)
            })
            .unwrap();
        let mut scope = None;
        store
            .reserve_v2_genesis_initializing(
                &source_ref,
                &task,
                saved_seed,
                &mut scope,
                &buffers,
                &mut rng,
            )
            .unwrap();
        let scope = scope.as_mut().unwrap();
        let mut writer =
            PreparedV2WriterRegistration::prepare(store, &source_ref, &task, scope).unwrap();
        let metadata_fault = match cut {
            ColdFixtureCut::Current
            | ColdFixtureCut::CompleteInitializing
            | ColdFixtureCut::BeforePin => 0,
            ColdFixtureCut::EmptyPin => 182,
            ColdFixtureCut::ShortPin => 183,
            ColdFixtureCut::EmptyStage => 184,
            ColdFixtureCut::ShortStage => 185,
            ColdFixtureCut::CompleteStage => 186,
        };
        store
            .critical_namespace_fault
            .store(metadata_fault, Ordering::SeqCst);
        let mut saved_head = false;
        let result = store.finish_v2_genesis_current(
            scope,
            &source_ref,
            &task,
            &buffers,
            &mut rng,
            |admission, _| {
                let mut owner = task.lock_initializing_owner_namespace(admission)?;
                owner.install_writer(&mut writer)?;
                let guard = owner.lock_archive_root_for_writer(&writer)?;
                if matches!(cut, ColdFixtureCut::BeforePin) {
                    // Real durable scope1 and installed writer, before the first pin open.
                    return Err(crate::store::critical_namespaces::refused());
                }
                let saved = activation.save_head(&guard)?;
                saved_head = true;
                if matches!(cut, ColdFixtureCut::CompleteInitializing) {
                    // The complete real pin/head exists, but the disk still has the real
                    // earlier Initializing1 row. Warm Runtime2 is intentionally discarded.
                    store
                        .chat_fence_pre_rename_failure
                        .store(true, Ordering::SeqCst);
                }
                Ok(saved)
            },
        );
        match cut {
            ColdFixtureCut::Current => {
                assert!(
                    saved_head,
                    "fixture must cross actual complete head barriers"
                );
                result.unwrap();
            }
            ColdFixtureCut::CompleteInitializing => {
                assert!(
                    saved_head,
                    "fixture must cross actual complete head barriers"
                );
                assert!(result.is_err());
            }
            ColdFixtureCut::BeforePin
            | ColdFixtureCut::EmptyPin
            | ColdFixtureCut::ShortPin
            | ColdFixtureCut::EmptyStage
            | ColdFixtureCut::ShortStage
            | ColdFixtureCut::CompleteStage => {
                assert!(
                    !saved_head,
                    "fault must precede the actual saved-head receipt"
                );
                assert!(result.is_err());
            }
        }
        // Every warm owner/writer/task/SDK/read and temporary credit drains before remount.
        // The implicit scope above owns the real preparation; no stale sender is retained.
    }
    drop(layout);
    let mut opening =
        PreparedV2CurrentLayoutStore::open(path.path(), SECRET, &mut rng, layout_policy(&pool))
            .unwrap();
    opening.retry(&mut rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    let read = layout
        .read_saved_bootstrap_source_data(&group, signer)
        .unwrap();
    let source = layout
        .checked_store()
        .unwrap()
        .prepare_genesis_seed(read, sdk_budget())
        .unwrap();
    ColdFixture {
        layout,
        source,
        pool,
        group,
        signer,
        rng,
        directory: path,
    }
}

#[test]
fn both_real_historical_roles_capture_without_creating_an_owner_or_task() {
    for initializing in [true, false] {
        let f = fixture(0xc100 + u64::from(initializing), initializing);
        let store = f.layout.checked_store().unwrap();
        let source = f.source.borrow_source_data();
        let mut capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
        let resources = capture.read_resources();
        let charged = f.pool.charged_for_test();
        if initializing {
            let basis = store
                .with_cold_v2_genesis_initializing_read(&source, |admission| {
                    let guard =
                        store.lock_cold_initializing_genesis_group(admission, &resources)?;
                    guard.validate_source_only()?;
                    let one = capture.capture_initializing(&guard)?;
                    let two = capture.capture_initializing(&guard)?;
                    assert!(Arc::ptr_eq(&one, &two));
                    Ok(guard.scope_basis_data().clone())
                })
                .unwrap();
            store
                .with_cold_v2_genesis_initializing_read(&source, |admission| {
                    assert_eq!(&basis, admission.scope_basis_data());
                    Ok(())
                })
                .unwrap();
            assert!(store
                .with_cold_v2_genesis_current_read(&source, |_| Ok(()))
                .is_err());
        } else {
            let basis = store
                .with_cold_v2_genesis_current_read(&source, |admission| {
                    let guard = store.lock_cold_current_genesis_group(admission, &resources)?;
                    let one = capture.capture_current(&guard)?;
                    let two = capture.capture_current(&guard)?;
                    assert!(Arc::ptr_eq(&one, &two));
                    Ok(guard.scope_basis_data().clone())
                })
                .unwrap();
            store
                .with_cold_v2_genesis_current_read(&source, |admission| {
                    assert_eq!(&basis, admission.scope_basis_data());
                    Ok(())
                })
                .unwrap();
            assert!(store
                .with_cold_v2_genesis_initializing_read(&source, |_| Ok(()))
                .is_err());
        }
        assert_eq!(f.pool.charged_for_test(), charged);
    }
}

#[test]
fn one_byte_short_capture_payment_enters_no_directory_path_or_open() {
    let f = fixture(0xc110, false);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let baseline = f.pool.charged_for_test();
    let blocker = f
        .pool
        .try_reserve(CAPACITY - baseline - COLD_CAPTURE_METADATA_FEE + 1)
        .unwrap();
    let entries = cold_group_read_entries_for_test();
    assert!(PreparedColdV2GenesisCapture::prepare(store, &source).is_err());
    assert_eq!(cold_group_read_entries_for_test(), entries);
    source.validate_original(store).unwrap();
    drop(blocker);
    let mut capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            capture
                .capture_current(&store.lock_cold_current_genesis_group(admission, &resources)?)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(cold_group_read_entries_for_test(), entries + 1);
}

#[test]
fn original_loan_rejects_equal_new_paid_read_and_capture_rejects_foreign_loan() {
    let f = fixture(0xc111, false);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let mut original = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let original_resources = original.read_resources();
    let other_read = f
        .layout
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let other = store
        .prepare_genesis_seed(other_read, sdk_budget())
        .unwrap();
    let other_source = other.borrow_source_data();
    assert_eq!(source.data().source, other_source.data().source);
    let entries = cold_group_read_entries_for_test();
    assert!(store
        .with_cold_v2_genesis_current_read(&other_source, |admission| {
            store
                .lock_cold_current_genesis_group(admission, &original_resources)
                .map(|_| ())
        })
        .is_err());
    assert_eq!(cold_group_read_entries_for_test(), entries);
    let other_capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let other_resources = other_capture.read_resources();
    store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            let guard = store.lock_cold_current_genesis_group(admission, &other_resources)?;
            assert!(original.capture_current(&guard).is_err());
            Ok(())
        })
        .unwrap();
    // Refusing an unrelated loan does not corrupt/re-target the original capture.
    store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            original.capture_current(
                &store.lock_cold_current_genesis_group(admission, &original_resources)?,
            )?;
            Ok(())
        })
        .unwrap();
}

#[test]
fn late_equal_directory_replacement_is_not_adopted_after_full_proof() {
    let f = fixture(0xc112, false);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let path = store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            let guard = store.lock_cold_current_genesis_group(admission, &resources)?;
            Ok(guard.directory_data().0.to_path_buf())
        })
        .unwrap();
    let displaced = path.with_extension("held-original-for-test");
    let reached = Arc::new(AtomicBool::new(false));
    let reached_in_hook = reached.clone();
    let original = path.clone();
    let moved = displaced.clone();
    BEFORE_FINAL_DIRECTORY.with(|slot| {
        assert!(slot
            .replace(Some(Box::new(move || {
                std::fs::rename(&original, &moved).unwrap();
                std::fs::create_dir(&original).unwrap();
                reached_in_hook.store(true, Ordering::SeqCst);
            })))
            .is_none());
    });
    assert!(store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            store
                .lock_cold_current_genesis_group(admission, &resources)
                .map(|_| ())
        })
        .is_err());
    assert!(reached.load(Ordering::SeqCst));
    assert!(displaced.join("activation.seed").is_file());
    assert!(displaced.join("pin.bin").is_file());
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&displaced, &path).unwrap();
    store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            store
                .lock_cold_current_genesis_group(admission, &resources)
                .map(|_| ())
        })
        .unwrap();
}

#[test]
fn marker_mutation_or_equal_replacement_during_late_source_proof_refuses() {
    for replacement in [false, true] {
        let f = fixture(0xc120 + u64::from(replacement), false);
        let store = f.layout.checked_store().unwrap();
        let source = f.source.borrow_source_data();
        let capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
        let resources = capture.read_resources();
        let group_path = store
            .with_cold_v2_genesis_current_read(&source, |admission| {
                let guard = store.lock_cold_current_genesis_group(admission, &resources)?;
                Ok(guard.directory_data().0.to_path_buf())
            })
            .unwrap();
        let seed = std::fs::read(group_path.join("activation.seed")).unwrap();
        let marker = store
            .dir
            .join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
            .join(super::super::CURRENT_MARKER);
        let original_bytes = std::fs::read(&marker).unwrap();
        let displaced = marker.with_extension("held-marker-for-test");
        let marker_in_hook = marker.clone();
        let displaced_in_hook = displaced.clone();
        let bytes_in_hook = original_bytes.clone();
        let reached = Arc::new(AtomicBool::new(false));
        let reached_in_hook = reached.clone();
        BEFORE_FINAL_DIRECTORY.with(|slot| {
            assert!(slot
                .replace(Some(Box::new(move || {
                    if replacement {
                        std::fs::rename(&marker_in_hook, &displaced_in_hook).unwrap();
                        std::fs::write(&marker_in_hook, &bytes_in_hook).unwrap();
                    } else {
                        let mut changed = bytes_in_hook;
                        *changed.last_mut().unwrap() ^= 1;
                        std::fs::write(&marker_in_hook, changed).unwrap();
                    }
                    reached_in_hook.store(true, Ordering::SeqCst);
                })))
                .is_none());
        });
        // The seam runs AFTER source/old-name reads, where the original draft reused a
        // stale marker proof. No returned guard or capture may survive either mutation.
        assert!(store
            .with_cold_v2_genesis_current_read(&source, |admission| {
                store
                    .lock_cold_current_genesis_group(admission, &resources)
                    .map(|_| ())
            })
            .is_err());
        assert!(reached.load(Ordering::SeqCst));
        assert_eq!(
            std::fs::read(group_path.join("activation.seed")).unwrap(),
            seed
        );
        if replacement {
            assert_eq!(std::fs::read(&marker).unwrap(), original_bytes);
            std::fs::remove_file(&marker).unwrap();
            std::fs::rename(&displaced, &marker).unwrap();
        } else {
            std::fs::write(&marker, &original_bytes).unwrap();
        }
        store
            .with_cold_v2_genesis_current_read(&source, |admission| {
                store
                    .lock_cold_current_genesis_group(admission, &resources)
                    .map(|_| ())
            })
            .unwrap();
    }
}

#[test]
fn missing_or_regular_group_name_refuses_without_creating_or_removing_anything() {
    let f = fixture(0xc113, false);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let path = store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            let guard = store.lock_cold_current_genesis_group(admission, &resources)?;
            Ok(guard.directory_data().0.to_path_buf())
        })
        .unwrap();
    let displaced = path.with_extension("original-kept-for-test");
    std::fs::rename(&path, &displaced).unwrap();
    assert!(store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            store
                .lock_cold_current_genesis_group(admission, &resources)
                .map(|_| ())
        })
        .is_err());
    assert!(!path.exists());
    std::fs::write(&path, b"not a directory or a saved result").unwrap();
    assert!(store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            store
                .lock_cold_current_genesis_group(admission, &resources)
                .map(|_| ())
        })
        .is_err());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"not a directory or a saved result"
    );
    assert!(displaced.join("activation.seed").is_file());
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&displaced, &path).unwrap();
}

#[test]
fn cancellation_during_last_full_proof_does_not_return_a_read_guard() {
    let f = fixture(0xc114, false);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let revoked = store.chat_mount_revoker();
    let reached = Arc::new(AtomicBool::new(false));
    let reached_in_hook = reached.clone();
    BEFORE_FINAL_DIRECTORY.with(|slot| {
        assert!(slot
            .replace(Some(Box::new(move || {
                reached_in_hook.store(true, Ordering::SeqCst);
                revoked.revoke();
            })))
            .is_none());
    });
    assert!(store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            store
                .lock_cold_current_genesis_group(admission, &resources)
                .map(|_| ())
        })
        .is_err());
    assert!(reached.load(Ordering::SeqCst));
    assert!(resources.require_original_negative(store, &source).is_err());
    assert!(source.validate_original(store).is_err());
}

#[test]
fn a_late_unknown_old_owner_file_cannot_borrow_the_complete_startup_walk() {
    for suffix in [".bin", ".replace.tmp"] {
        let f = fixture(0xc115 + u64::from(suffix == ".bin"), false);
        let store = f.layout.checked_store().unwrap();
        let source = f.source.borrow_source_data();
        let capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
        let resources = capture.read_resources();
        let parent = store.dir.join("owner-state");
        std::fs::create_dir_all(&parent).unwrap();
        let identifier = crate::store::owner_evidence::canonical::key(&f.group, &f.signer);
        let path = parent.join(format!("{}{suffix}", hex::encode(identifier)));
        std::fs::write(&path, b"unknown retained old owner source").unwrap();
        let entries = cold_group_read_entries_for_test();
        assert!(store
            .with_cold_v2_genesis_current_read(&source, |admission| {
                store
                    .lock_cold_current_genesis_group(admission, &resources)
                    .map(|_| ())
            })
            .is_err());
        assert_eq!(cold_group_read_entries_for_test(), entries);
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"unknown retained old owner source"
        );
    }
}
