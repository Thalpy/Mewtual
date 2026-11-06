//! Actual partial repair to same paid node to original-seed settlement, with no restart or
//! Warm/Cold proof conversion. Parent tests::partial supplies the real producer fault cut.
use super::*;
use crate::store::persistence::v2_genesis_activation::settlement::{
    GenesisSeedSettlementProgressData as Phase, GENESIS_SEED_SETTLEMENT_METADATA_FEE,
};
use rand_core::{CryptoRng, RngCore};

struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("settled replay/refusal reached RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("settled replay/refusal reached RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("settled replay/refusal reached RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("settled replay/refusal reached RNG")
    }
}
impl CryptoRng for NeverRng {}

type Current = UnexposedRepairedV2GenesisCurrent<MemNetwork, ChaCha20Rng>;
fn saved(number: u64) -> (PartialFixture, Current) {
    let mut f = partial(number);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    current.validate_original(&f.layout).unwrap();
    (f, current)
}

#[test]
fn repaired_same_node_settles_and_refunds_original_read_before_new_paid_absent_mount() {
    let (mut f, mut current) = saved(0xc900);
    let source = current.data().source;
    let debit = current.resources.source.capture.debit.frame.to_vec();
    let owner = current.scope.owner_identity();
    let nonce = current.scope.nonce();
    let writer = current.writer.observe_original();
    let task = current.task.observe_original();
    let original_read = current.resources.source.observe_original();
    let original_sdk = current
        .node
        .borrow_genesis_seed_data()
        .observe_original_seed_data();
    let head = current.scope.saved_head().unwrap().clone();
    let pin = std::fs::read(&f.pin).unwrap();
    let head_bytes = std::fs::read(&f.head).unwrap();
    let target = current.resources.source.capture.target.clone();
    let source_seed = std::fs::read(target.join("source.seed")).unwrap();
    let entries = super::super::super::super::SDK_ENTRIES.with(|n| n.get());
    current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .unwrap();
    assert!(!f.seed.exists());
    assert_eq!(std::fs::read(&f.pin).unwrap(), pin);
    assert_eq!(std::fs::read(&f.head).unwrap(), head_bytes);
    assert_eq!(
        std::fs::read(target.join("source.seed")).unwrap(),
        source_seed
    );
    for name in ["origin", "source.bin", "records.bin", "manifest.bin"] {
        assert!(target.join(name).is_file());
    }
    assert_eq!(source, current.data().source);
    assert_eq!(nonce, current.scope.nonce());
    assert!(Arc::ptr_eq(&owner, &current.scope.owner_identity()));
    assert!(Arc::ptr_eq(&head, current.scope.saved_head().unwrap()));
    current.writer.require_same_observation(&writer).unwrap();
    task.require_same_original(&current.task).unwrap();
    original_read
        .require_same_original(&current.resources.source)
        .unwrap();
    original_sdk
        .require_same_seed_data(current.node.borrow_genesis_seed_data())
        .unwrap();
    assert_eq!(
        super::super::super::super::SDK_ENTRIES.with(|n| n.get()),
        entries
    );
    current.validate_original(&f.layout).unwrap();
    let saved = current
        .seed_settlement
        .as_ref()
        .unwrap()
        .saved_outcome()
        .unwrap()
        .clone();
    current
        .retry_seed_settlement(&f.layout, &mut NeverRng)
        .unwrap();
    assert!(Arc::ptr_eq(
        &saved,
        current
            .seed_settlement
            .as_ref()
            .unwrap()
            .saved_outcome()
            .unwrap()
    ));
    // Observe the SPECIFIC original read reservation while Saved metadata stays strong;
    // aggregate refunds could hide an unrelated retained fee. SDK dual-half fee lifetime
    // has independent MLS controls; this notice pins ReadCapture destruction specifically.
    let read_dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let capture =
        Arc::get_mut(&mut current.resources.source.capture).expect("unique original read capture");
    Arc::get_mut(&mut capture.fee)
        .expect("unique original read payment")
        .observe_drop_for_test(read_dropped.clone());
    assert!(!read_dropped.load(Ordering::SeqCst));
    drop(current);
    assert!(read_dropped.load(Ordering::SeqCst));
    assert!(writer.is_cancelled());
    assert!(f.pool.charged_for_test() >= GENESIS_SEED_SETTLEMENT_METADATA_FEE);
    // No original node/owner crosses this NEW mount. Saved remains inert metadata while
    // new source/SDK/cold claim proofs are independently obtained from the real files.
    let PartialFixture {
        preparation,
        layout,
        pool,
        group,
        signer,
        mut rng,
        directory,
        ..
    } = f;
    drop(preparation);
    drop(layout);
    let policy = crate::store::V2CurrentLayoutPolicy::new(
        crate::store::ArchiveBootstrapCatalogPolicy::new(
            pool.clone(),
            4 * archive::MAX_SEED_FRAMED,
        )
        .unwrap()
        .with_bootstrap_source_data_read(),
        crate::store::CanonicalFilenameInventoryPolicy::new(32, 144 * 1024 * 1024 + 1024 + 40)
            .unwrap(),
        4 * layout_codec::MAX_DECLARATION_FRAME,
    )
    .unwrap();
    let mut opening = crate::store::PreparedV2CurrentLayoutStore::open(
        directory.path(),
        b"genuine task-free cold group reads",
        &mut rng,
        policy,
    )
    .unwrap();
    opening.retry(&mut rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    let read = layout
        .read_saved_bootstrap_source_data(&group, signer)
        .unwrap();
    assert_eq!(read.capture.debit.frame.as_slice(), debit);
    let budget = crate::store::GenesisSeedBudget::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap();
    let seed = layout
        .checked_store()
        .unwrap()
        .prepare_genesis_seed(read, budget)
        .unwrap();
    let mut cold = crate::store::PreparedColdV2GenesisCurrent::new(
        seed,
        Arc::new(Hub::new().join(PeerId::from_u64(0xc901))),
        ChaCha20Rng::seed_from_u64(0xc901),
        Arc::new(ManualClock::new(30)),
    );
    cold.retry(&layout, &mut rng).unwrap();
    let mut cold = cold.take_current(&layout).unwrap().unwrap();
    assert_eq!(cold.data().source, source);
    cold.validate_original(&layout).unwrap();
    assert!(
        cold.retry_seed_settlement(&layout, &mut NeverRng).is_err(),
        "absent capture cannot fabricate a Present cleanup receipt"
    );
    cold.validate_original(&layout).unwrap();
    drop(saved);
}

#[test]
fn every_repaired_cleanup_cut_preserves_owner_head_and_phase_before_mandatory_seed_validation() {
    for point in 210..=215 {
        let (mut f, mut current) = saved(0xc910 + u64::from(point));
        let store = f.layout.checked_store().unwrap();
        let source = current.data().source;
        let owner = current.scope.owner_identity();
        let nonce = current.scope.nonce();
        let writer = current.writer.observe_original();
        let head = current.scope.saved_head().unwrap().clone();
        let read = current.resources.source.observe_original();
        let sdk = current
            .node
            .borrow_genesis_seed_data()
            .observe_original_seed_data();
        store
            .critical_namespace_fault
            .store(point, Ordering::SeqCst);
        assert!(
            current
                .retry_seed_settlement(&f.layout, &mut f.rng)
                .is_err(),
            "cut{point}"
        );
        let worker = current.seed_settlement.as_ref().unwrap();
        let expected = match point {
            210 => Phase::OriginalPresent,
            211 => Phase::UnlinkAdmitted,
            212 => Phase::AbsentParentPending,
            _ => Phase::ParentSynced,
        };
        assert_eq!(worker.progress_data_for_test(), expected);
        assert_eq!(f.seed.exists(), point < 212);
        let cached = worker.saved_outcome().cloned();
        if point == 215 {
            assert!(cached.is_some());
        }
        store.critical_namespace_fault.store(0, Ordering::SeqCst);
        if (211..=214).contains(&point) {
            assert!(
                current.validate_original(&f.layout).is_err(),
                "admitted cleanup without Saved must never fall back to seed-mandatory validation"
            );
        }
        if point == 215 {
            current
                .retry_seed_settlement(&f.layout, &mut NeverRng)
                .unwrap();
        } else {
            current
                .retry_seed_settlement(&f.layout, &mut f.rng)
                .unwrap();
        }
        if let Some(cached) = cached {
            assert!(Arc::ptr_eq(
                &cached,
                current
                    .seed_settlement
                    .as_ref()
                    .unwrap()
                    .saved_outcome()
                    .unwrap()
            ));
        }
        assert!(!f.seed.exists());
        assert_eq!(source, current.data().source);
        assert_eq!(nonce, current.scope.nonce());
        assert!(Arc::ptr_eq(&owner, &current.scope.owner_identity()));
        assert!(Arc::ptr_eq(&head, current.scope.saved_head().unwrap()));
        current.writer.require_same_observation(&writer).unwrap();
        read.require_same_original(&current.resources.source)
            .unwrap();
        sdk.require_same_seed_data(current.node.borrow_genesis_seed_data())
            .unwrap();
        current.validate_original(&f.layout).unwrap();
    }
}

#[test]
fn repaired_even_clean_covering_write_faults_preserve_seed_and_same_scope2_binding() {
    for parent_fault in [false, true] {
        let (mut f, mut current) = saved(0xc920 + u64::from(parent_fault));
        let store = f.layout.checked_store().unwrap();
        let owner = current.scope.owner_identity();
        let nonce = current.scope.nonce();
        let writer = current.writer.observe_original();
        let original = std::fs::read(&f.seed).unwrap();
        if parent_fault {
            store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
        } else {
            store
                .chat_fence_pre_rename_failure
                .store(true, Ordering::SeqCst);
        }
        assert!(current
            .retry_seed_settlement(&f.layout, &mut f.rng)
            .is_err());
        let worker = current.seed_settlement.as_ref().unwrap();
        assert_eq!(worker.progress_data_for_test(), Phase::OriginalPresent);
        assert!(worker.saved_outcome().is_none());
        assert_eq!(std::fs::read(&f.seed).unwrap(), original);
        current
            .retry_seed_settlement(&f.layout, &mut f.rng)
            .unwrap();
        assert!(Arc::ptr_eq(&owner, &current.scope.owner_identity()));
        assert_eq!(nonce, current.scope.nonce());
        current.writer.require_same_observation(&writer).unwrap();
        current.validate_original(&f.layout).unwrap();
    }
}

#[test]
fn repaired_cleanup_preserves_foreign_seed_later_stage_and_seed_reappearance() {
    let (mut f, mut current) = saved(0xc930);
    let original = std::fs::read(&f.seed).unwrap();
    let displaced = f
        .directory
        .path()
        .join("held-original-repaired-activation-seed");
    std::fs::rename(&f.seed, &displaced).unwrap();
    std::fs::write(&f.seed, &original).unwrap();
    assert!(current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .is_err());
    assert_eq!(std::fs::read(&f.seed).unwrap(), original);
    assert_eq!(
        current
            .seed_settlement
            .as_ref()
            .unwrap()
            .progress_data_for_test(),
        Phase::OriginalPresent
    );
    std::fs::remove_file(&f.seed).unwrap();
    std::fs::rename(&displaced, &f.seed).unwrap();
    let target = current.resources.source.capture.target.clone();
    for name in ["head.stage", "private-successor.bin"] {
        let path = target.join(name);
        std::fs::write(&path, b"possible newer indispensable state").unwrap();
        assert!(current
            .retry_seed_settlement(&f.layout, &mut f.rng)
            .is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"possible newer indispensable state"
        );
        assert_eq!(std::fs::read(&f.seed).unwrap(), original);
        std::fs::remove_file(path).unwrap(); // Remove only this test's unadmitted fixture.
    }
    f.layout
        .checked_store()
        .unwrap()
        .critical_namespace_fault
        .store(212, Ordering::SeqCst);
    assert!(current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .is_err());
    assert!(!f.seed.exists());
    f.layout
        .checked_store()
        .unwrap()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    std::fs::write(&f.seed, &original).unwrap();
    assert!(current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .is_err());
    assert_eq!(std::fs::read(&f.seed).unwrap(), original);
    assert_eq!(
        current
            .seed_settlement
            .as_ref()
            .unwrap()
            .progress_data_for_test(),
        Phase::AbsentParentPending
    );
    assert!(current
        .seed_settlement
        .as_ref()
        .unwrap()
        .saved_outcome()
        .is_none());
    std::fs::remove_file(&f.seed).unwrap();
    current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .unwrap();
    current.validate_original(&f.layout).unwrap();
}

#[test]
fn repaired_payment_and_cancellation_refuse_without_replacing_cleanup_or_node_owners() {
    let (f, mut current) = saved(0xc940);
    let source = current.data().source;
    let seed = std::fs::read(&f.seed).unwrap();
    let pressure = f
        .pool
        .try_reserve(
            1024 * 1024 * 1024 - f.pool.charged_for_test() - GENESIS_SEED_SETTLEMENT_METADATA_FEE
                + 1,
        )
        .unwrap();
    assert!(current
        .retry_seed_settlement(&f.layout, &mut NeverRng)
        .is_err());
    assert!(current.seed_settlement.is_none());
    assert_eq!(source, current.data().source);
    assert_eq!(std::fs::read(&f.seed).unwrap(), seed);
    drop(pressure);
    current.validate_original(&f.layout).unwrap();
    current.scope.cancel_original();
    assert!(current
        .retry_seed_settlement(&f.layout, &mut NeverRng)
        .is_err());
    assert!(current.seed_settlement.is_none());
    assert_eq!(std::fs::read(&f.seed).unwrap(), seed);
    assert!(current.validate_original(&f.layout).is_err());
    let (mut other, mut current) = saved(0xc941);
    other
        .layout
        .checked_store()
        .unwrap()
        .critical_namespace_fault
        .store(212, Ordering::SeqCst);
    assert!(current
        .retry_seed_settlement(&other.layout, &mut other.rng)
        .is_err());
    assert!(!other.seed.exists());
    other
        .layout
        .checked_store()
        .unwrap()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    current.writer.cancel_original();
    assert!(current
        .retry_seed_settlement(&other.layout, &mut NeverRng)
        .is_err());
    assert!(current.validate_original(&other.layout).is_err());
    let worker = current.seed_settlement.as_ref().unwrap();
    assert_eq!(worker.progress_data_for_test(), Phase::AbsentParentPending);
    assert!(worker.saved_outcome().is_none());
    assert!(!other.seed.exists());
}
