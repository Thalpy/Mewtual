//! Real warm SourceSaved -> paid SDK/node -> actual scope2/writer -> seed cleanup faults.
//! These tests never construct a Saved/Current receipt or raw task/owner flag. The supported
//! native inventory profile supports Windows and Linux; other-platform refusal is covered
//! in the shared inventory tests rather than presented as successful cleanup coverage.
use super::*;
use crate::store::persistence::v2_genesis_activation::settlement::{
    GenesisSeedSettlementProgressData, GENESIS_SEED_SETTLEMENT_METADATA_FEE,
};
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
use std::sync::atomic::Ordering;

#[cfg(any(windows, target_os = "linux"))]
fn saved(number: u64) -> (Fixture, UnexposedV2GenesisCurrent<MemNetwork, ChaCha20Rng>) {
    let mut f = fixture(number);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    current.validate_original(&f.layout).unwrap();
    (f, current)
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn genuine_original_node_settles_only_activation_seed_and_keeps_all_other_artifacts() {
    let (mut f, mut current) = saved(0x5e710);
    let group = current
        .task
        .saved_directory()
        .unwrap()
        .directory_data()
        .0
        .to_path_buf();
    let target = current.resources.source.capture.target.clone();
    let pin = std::fs::read(group.join("pin.bin")).unwrap();
    let head = std::fs::read(target.join("head.bin")).unwrap();
    let origin = std::fs::read(target.join("origin")).unwrap();
    let source = current.data().source;
    let writer = current.writer.observe_original();
    let nonce = current.scope.nonce();
    let owner = current.scope.owner_identity();
    let sdk_entries = super::super::super::SDK_ENTRIES.with(|n| n.get());
    current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .unwrap();
    assert!(!group.join("activation.seed").exists());
    assert!(group.is_dir());
    assert_eq!(std::fs::read(group.join("pin.bin")).unwrap(), pin);
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
    assert_eq!(std::fs::read(target.join("origin")).unwrap(), origin);
    for name in ["source.seed", "source.bin", "records.bin", "manifest.bin"] {
        assert!(target.join(name).is_file(), "preserve {name}");
    }
    assert_eq!(current.data().source, source);
    assert_eq!(current.scope.nonce(), nonce);
    assert!(Arc::ptr_eq(&owner, &current.scope.owner_identity()));
    current.writer.require_same_observation(&writer).unwrap();
    assert_eq!(
        super::super::super::SDK_ENTRIES.with(|n| n.get()),
        sdk_entries
    );
    current.validate_original(&f.layout).unwrap();
    current
        .retry_seed_settlement(&f.layout, &mut NeverRng)
        .unwrap();
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn every_actual_cleanup_cut_retries_same_node_after_unlink_without_seed_present_fallback() {
    for point in 210..=215 {
        let (mut f, mut current) = saved(0x5e720 + u64::from(point));
        let store = f.layout.checked_store().unwrap();
        let group = current
            .task
            .saved_directory()
            .unwrap()
            .directory_data()
            .0
            .to_path_buf();
        let source = current.data().source;
        let writer = current.writer.observe_original();
        let owner = current.scope.owner_identity();
        let nonce = current.scope.nonce();
        store
            .critical_namespace_fault
            .store(point, Ordering::SeqCst);
        assert!(
            current
                .retry_seed_settlement(&f.layout, &mut f.rng)
                .is_err(),
            "point{point}"
        );
        let worker = current.seed_settlement.as_ref().unwrap();
        let expected = match point {
            210 => GenesisSeedSettlementProgressData::OriginalPresent,
            211 => GenesisSeedSettlementProgressData::UnlinkAdmitted,
            212 => GenesisSeedSettlementProgressData::AbsentParentPending,
            _ => GenesisSeedSettlementProgressData::ParentSynced,
        };
        assert_eq!(worker.progress_data_for_test(), expected);
        assert_eq!(group.join("activation.seed").is_file(), point < 212);
        let cached = worker.saved_outcome().cloned();
        if point == 215 {
            assert!(cached.is_some());
        }
        store.critical_namespace_fault.store(0, Ordering::SeqCst);
        current
            .retry_seed_settlement(&f.layout, &mut f.rng)
            .unwrap();
        assert!(!group.join("activation.seed").exists());
        if let Some(cached) = cached {
            assert!(Arc::ptr_eq(
                current
                    .seed_settlement
                    .as_ref()
                    .unwrap()
                    .saved_outcome()
                    .unwrap(),
                &cached
            ));
        }
        assert_eq!(current.data().source, source);
        assert_eq!(current.scope.nonce(), nonce);
        assert!(Arc::ptr_eq(&owner, &current.scope.owner_identity()));
        current.writer.require_same_observation(&writer).unwrap();
        current.validate_original(&f.layout).unwrap();
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn equal_foreign_seed_is_preserved_until_same_original_object_returns() {
    let (mut f, mut current) = saved(0x5e730);
    let group = current
        .task
        .saved_directory()
        .unwrap()
        .directory_data()
        .0
        .to_path_buf();
    let path = group.join("activation.seed");
    let original = std::fs::read(&path).unwrap();
    let displaced = f._directory.path().join("original-activation-seed");
    // Move the FILE, not the held nonempty directory (Windows may forbid the latter).
    std::fs::rename(&path, &displaced).unwrap();
    std::fs::write(&path, &original).unwrap();
    assert!(current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .is_err());
    assert_eq!(
        current
            .seed_settlement
            .as_ref()
            .unwrap()
            .progress_data_for_test(),
        GenesisSeedSettlementProgressData::OriginalPresent
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&displaced, &path).unwrap();
    current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .unwrap();
    current.validate_original(&f.layout).unwrap();
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn possible_later_private_data_or_stage_refuses_cleanup_without_repairing_history() {
    for name in ["head.stage", "private-successor.bin"] {
        let (mut f, mut current) = saved(0x5e740 + u64::from(name == "head.stage"));
        let group = current
            .task
            .saved_directory()
            .unwrap()
            .directory_data()
            .0
            .to_path_buf();
        let seed = std::fs::read(group.join("activation.seed")).unwrap();
        let later = current.resources.source.capture.target.join(name);
        std::fs::write(&later, b"possible essential newer state").unwrap();
        assert!(current
            .retry_seed_settlement(&f.layout, &mut f.rng)
            .is_err());
        assert_eq!(
            std::fs::read(&later).unwrap(),
            b"possible essential newer state"
        );
        assert_eq!(std::fs::read(group.join("activation.seed")).unwrap(), seed);
        assert_eq!(
            current
                .seed_settlement
                .as_ref()
                .unwrap()
                .progress_data_for_test(),
            GenesisSeedSettlementProgressData::OriginalPresent
        );
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn one_byte_short_metadata_refuses_before_scope_rng_and_keeps_original_node() {
    let (f, mut current) = saved(0x5e750);
    let group = current
        .task
        .saved_directory()
        .unwrap()
        .directory_data()
        .0
        .to_path_buf();
    let before = current.data().source;
    let spare = CAPACITY - f.pool.charged_for_test() - GENESIS_SEED_SETTLEMENT_METADATA_FEE + 1;
    let pressure = f.pool.try_reserve(spare).unwrap();
    assert!(current
        .retry_seed_settlement(&f.layout, &mut NeverRng)
        .is_err());
    assert!(current.seed_settlement.is_none());
    assert!(group.join("activation.seed").is_file());
    assert_eq!(current.data().source, before);
    drop(pressure);
    current.validate_original(&f.layout).unwrap();
}

#[test]
fn marker_changed_after_last_seed_comparison_preserves_seed_and_same_owner_retry() {
    for replace in [false, true] {
        let (mut f, mut current) = saved(0x5e760 + u64::from(replace));
        let store = f.layout.checked_store().unwrap();
        let owner = current.settlement_owner_ref().unwrap();
        let mut cleanup = crate::store::persistence::v2_genesis_activation::settlement::PreparedGenesisSeedSettlement::prepare(store, &owner).unwrap();
        let resources = cleanup.resources();
        let group = current
            .task
            .saved_directory()
            .unwrap()
            .directory_data()
            .0
            .to_path_buf();
        let seed = std::fs::read(group.join("activation.seed")).unwrap();
        let marker = store
            .dir
            .join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
            .join("current-layout.bin");
        let original = std::fs::read(&marker).unwrap();
        let displaced = f
            ._directory
            .path()
            .join("original-layout-before-settlement");
        let reached = std::rc::Rc::new(std::cell::Cell::new(false));
        let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
        assert!(store.with_durable_genesis_current2_settlement(&owner, &mut cleanup, &buffers, &mut f.rng,
            |admission, worker| {
                let guard = store.lock_genesis_seed_settlement_group(admission, &resources)?;
                let marker = marker.clone();
                let displaced = displaced.clone();
                let bytes = original.clone();
                let reached = reached.clone();
                // Arm AFTER the real Durable admission and namespace guard; their earlier
                // proofs cannot accidentally consume this test's last-comparison seam.
                crate::store::persistence::v2_genesis_activation::settlement::after_last_seed_comparison_for_test(Box::new(move || {
                    reached.set(true);
                    if replace {
                        std::fs::rename(&marker, &displaced).unwrap();
                        std::fs::write(&marker, bytes).unwrap();
                    } else {
                        let mut damaged = bytes;
                        *damaged.last_mut().unwrap() ^= 1;
                        std::fs::write(&marker, damaged).unwrap();
                    }
                }));
                worker.settle(&guard)
            }).is_err());
        drop(buffers);
        assert!(reached.get());
        assert_eq!(
            cleanup.progress_data_for_test(),
            GenesisSeedSettlementProgressData::OriginalPresent
        );
        assert!(cleanup.saved_outcome().is_none());
        assert_eq!(std::fs::read(group.join("activation.seed")).unwrap(), seed);
        if replace {
            std::fs::remove_file(&marker).unwrap();
            std::fs::rename(&displaced, &marker).unwrap();
        } else {
            std::fs::write(&marker, original).unwrap();
        }
        // Move ONLY the same actual cleanup state into its owning node; retry preserves
        // every genuine scope/task/writer/source association instead of preparing anew.
        current.seed_settlement = Some(cleanup);
        current
            .retry_seed_settlement(&f.layout, &mut f.rng)
            .unwrap();
        current.validate_original(&f.layout).unwrap();
    }
}

#[test]
fn actual_cleanup_then_new_mount_uses_new_paid_absent_capture_and_same_genesis_source() {
    let (mut f, mut current) = saved(0x5e770);
    let group = current.data().context.spec_data().group.to_vec();
    let signer = current.data().context.spec_data().signer;
    let source_id = current.data().source;
    let debit = current.resources.source.capture.debit.frame.to_vec();
    current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .unwrap();
    current.validate_original(&f.layout).unwrap();
    // Drain the real original node/writer/task/mount. No warm Saved or SDK graph crosses
    // this boundary; the historical source reader and SDK restore are separately paid.
    drop(current);
    let Fixture {
        preparation,
        layout,
        pool,
        mut rng,
        _directory,
    } = f;
    drop(preparation);
    drop(layout);
    let policy = V2CurrentLayoutPolicy::new(
        archive(&pool),
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * critical_namespaces::archive_current_namespace_codec::MAX_DECLARATION_FRAME,
    )
    .unwrap();
    let mut opening =
        PreparedV2CurrentLayoutStore::open(_directory.path(), SECRET, &mut rng, policy).unwrap();
    opening.retry(&mut rng).unwrap();
    let reopened = opening.into_current_layout_store().unwrap();
    let store = reopened.checked_store().unwrap();
    let read = store
        .read_saved_bootstrap_source_data(&group, signer)
        .unwrap();
    assert_eq!(read.capture.debit.frame.as_slice(), debit);
    let budget = GenesisSeedBudget::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap();
    let seed = store.prepare_genesis_seed(read, budget).unwrap();
    let mut cold = PreparedColdV2GenesisCurrent::new(
        seed,
        Arc::new(Hub::new().join(PeerId::from_u64(0x5e771))),
        ChaCha20Rng::seed_from_u64(0x5e771),
        Arc::new(ManualClock::new(30)),
    );
    cold.retry(&reopened, &mut rng).unwrap();
    let mut cold_current = cold.take_current(&reopened).unwrap().unwrap();
    assert_eq!(cold_current.data().source, source_id);
    cold_current.validate_original(&reopened).unwrap();
    // Already-absent recovery does not manufacture a Present cleanup/Saved receipt. It is
    // independently valid before and after the strict Present-only cleanup refusal.
    assert!(cold_current
        .retry_seed_settlement(&reopened, &mut rng)
        .is_err());
    cold_current.validate_original(&reopened).unwrap();
    drop(cold_current);
    drop(cold);
    drop(reopened);
    drop(_directory);
}

#[test]
fn saved_metadata_keeps_only_paid_controls_after_exact_source_read_refund() {
    let (mut f, mut current) = saved(0x5e780);
    current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .unwrap();
    let read_dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // These exact mutable loans require that no hidden ReadCapture/fee strong alias remains
    // after the real reader worker drained. Weak identities are separate tiny allocations.
    let capture = Arc::get_mut(&mut current.resources.source.capture)
        .expect("only original output owns actual read capture");
    Arc::get_mut(&mut capture.fee)
        .expect("only original read capture owns actual read invoice")
        .observe_drop_for_test(read_dropped.clone());
    let saved = current
        .seed_settlement
        .as_ref()
        .unwrap()
        .saved_outcome()
        .unwrap()
        .clone();
    let original_read = current.resources.source.observe_original();
    let original_sdk = current
        .node
        .borrow_genesis_seed_data()
        .observe_original_seed_data();
    let before = f.pool.charged_for_test();
    assert!(!read_dropped.load(Ordering::SeqCst));
    drop(current);
    let after = f.pool.charged_for_test();
    // The SPECIFIC original source-read invoice destructs while Saved still holds its
    // original metadata/Files/frames. Other refunded accounts cannot satisfy this notice.
    // This is a ReadCapture refund control; dual-half SDK fee lifetime has separate actual
    // MLS regressions. Source structure here contains only its tiny SDK weak observation.
    assert!(read_dropped.load(Ordering::SeqCst));
    assert!(after < before);
    assert!(after >= GENESIS_SEED_SETTLEMENT_METADATA_FEE);
    drop(original_read);
    drop(original_sdk);
    drop(saved);
}
