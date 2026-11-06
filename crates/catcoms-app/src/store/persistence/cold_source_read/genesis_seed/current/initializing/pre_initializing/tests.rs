//! Genuine complete-journal cut BEFORE reserveRow1, full warm drain/remount/new paid SDK,
//! then actual birth, checked same-owner conversion and ordinary repair. No row, receipt,
//! task watch or successor is constructed by a test.
use super::*;
#[cfg(windows)]
use crate::store::critical_namespaces::current_layout::cold_read::tests::ColdFixture;
use crate::store::critical_namespaces::current_layout::cold_read::tests::{
    fixture_at_cut, ColdFixtureCut,
};
#[cfg(windows)]
use catcoms_rt::MemNetwork;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};

struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("replay/refusal reached RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("replay/refusal reached RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("replay/refusal reached RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("replay/refusal reached RNG")
    }
}
impl CryptoRng for NeverRng {}

#[cfg(windows)]
type Preparation = PreparedColdV2GenesisPreInitializing<MemNetwork, ChaCha20Rng>;
#[cfg(windows)]
struct Fixture {
    preparation: Preparation,
    layout: V2CurrentLayoutStore,
    rng: ChaCha20Rng,
    pool: crate::store::critical_namespaces::read_pool::CriticalCatalogReadPool,
    group: Vec<u8>,
    signer: [u8; 32],
    pin: std::path::PathBuf,
    head: std::path::PathBuf,
    stage: std::path::PathBuf,
    seed: std::path::PathBuf,
    original: Vec<u8>,
    directory: tempfile::TempDir,
}
#[cfg(windows)]
fn fixture(number: u64) -> Fixture {
    let ColdFixture {
        source,
        layout,
        rng,
        pool,
        group,
        signer,
        directory,
    } = fixture_at_cut(number, ColdFixtureCut::CompleteSeedBeforeScope);
    let source_ref = source.borrow_source_data();
    let store = layout.checked_store().unwrap();
    let suffix = {
        let root = store.archive_bootstrap_root.lock().unwrap();
        let root = root.as_ref().unwrap();
        crate::store::critical_namespaces::current_group::group_suffix(
            &root.descriptor.vault,
            &root.descriptor.root,
            &group,
        )
        .unwrap()
    };
    let path = source_ref
        .source()
        .capture
        .root
        .join(format!("g-{}", std::str::from_utf8(&suffix).unwrap()));
    let seed = path.join("activation.seed");
    let pin = path.join("pin.bin");
    let head = source_ref.source().capture.target.join("head.bin");
    let stage = source_ref.source().capture.target.join("head.stage");
    let original = std::fs::read(&seed).unwrap();
    assert!(!pin.exists() && !head.exists() && !stage.exists());
    let preparation = Preparation::new(
        source,
        Arc::new(Hub::new().join(PeerId::from_u64(number))),
        ChaCha20Rng::seed_from_u64(number),
        Arc::new(ManualClock::new(30)),
    );
    Fixture {
        preparation,
        layout,
        rng,
        pool,
        group,
        signer,
        pin,
        head,
        stage,
        seed,
        original,
        directory,
    }
}

#[cfg(windows)]
#[test]
fn no_row_birth_keeps_same_owner_read_sdk_task_and_refunds_old_capture_before_inner_current() {
    let mut f = fixture(0xcb80);
    let old_dropped = Arc::new(AtomicBool::new(false));
    let source = f.preparation.inner.graph.source().unwrap();
    let id = source.data().source;
    let read = source.source().observe_original();
    let sdk = source.seed().observe_original_seed_data();
    let mut capture =
        PreparedColdV2PreInitializingCapture::prepare(f.layout.checked_store().unwrap(), &source)
            .unwrap();
    capture
        .observe_metadata_drop_for_test(old_dropped.clone())
        .unwrap();
    f.preparation.capture = Some(capture);
    f.preparation.fail_after_born = true;
    assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
    let born = f.preparation.born.as_ref().unwrap();
    let (owner, nonce) = born.owner_data_for_test();
    let receipt = born.born_repair_seed().unwrap().clone();
    let new_seed = receipt.repair_seed().clone();
    let task = f
        .preparation
        .inner
        .task
        .as_ref()
        .unwrap()
        .observe_original();
    assert!(!old_dropped.load(Ordering::SeqCst));
    assert!(!f.pin.exists() && !f.head.exists() && !f.stage.exists());
    // Holding the temporary Born receipt would intentionally hold the old first loan.
    // Only final normal RepairSeed is retained for the exact old fee release assertion.
    drop(receipt);
    f.preparation.fail_after_conversion = true;
    assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
    assert!(f.preparation.born.is_none());
    assert!(
        f.preparation.origin.is_none()
            && f.preparation.capture.is_none()
            && f.preparation.born_capture.is_none()
    );
    assert!(
        old_dropped.load(Ordering::SeqCst),
        "specific original 1MiB capture refunds while new RepairSeed, owner and Store remain live"
    );
    assert!(Arc::ptr_eq(
        f.preparation.inner.scope.as_ref().unwrap().seed(),
        &new_seed
    ));
    assert!(!f.pin.exists() && !f.head.exists() && !f.stage.exists());
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    assert_eq!(current.data().source, id);
    use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
    assert!(Arc::ptr_eq(&owner, &current.scope.owner_identity()));
    assert_eq!(nonce, current.scope.nonce());
    task.require_same_original(&current.task).unwrap();
    read.require_same_original(&current.resources.source)
        .unwrap();
    sdk.require_same_seed_data(current.source().seed()).unwrap();
    assert_eq!(std::fs::read(&f.seed).unwrap(), f.original);
    assert!(f.pin.exists() && f.head.exists() && !f.stage.exists());
    drop(f.preparation);
    current.validate_original(&f.layout).unwrap();
}

#[cfg(windows)]
#[test]
fn both_birth_scope_write_faults_retry_same_actual_owner_and_original_absent_targets() {
    for parent in [false, true] {
        let mut f = fixture(0xcb81 + u64::from(parent));
        let store = f.layout.checked_store().unwrap();
        if parent {
            store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
        } else {
            store
                .chat_fence_pre_rename_failure
                .store(true, Ordering::SeqCst);
        }
        assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
        let born = f.preparation.born.as_ref().unwrap();
        assert!(born.born_repair_seed().is_none());
        let (owner, nonce) = born.owner_data_for_test();
        let task = f
            .preparation
            .inner
            .task
            .as_ref()
            .unwrap()
            .observe_original();
        assert_eq!(std::fs::read(&f.seed).unwrap(), f.original);
        assert!(!f.pin.exists() && !f.head.exists() && !f.stage.exists());
        f.preparation.retry(&f.layout, &mut f.rng).unwrap();
        let current = f.preparation.take_current(&f.layout).unwrap().unwrap();
        use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
        assert!(Arc::ptr_eq(&owner, &current.scope.owner_identity()));
        assert_eq!(nonce, current.scope.nonce());
        task.require_same_original(&current.task).unwrap();
        current.validate_original(&f.layout).unwrap();
    }
}

#[cfg(windows)]
#[test]
fn committed_successor_lost_return_retries_same_claim_without_reregistering() {
    let mut f = fixture(0xcb83);
    f.preparation.fail_after_handoff = true;
    assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
    let born = f.preparation.born.as_ref().unwrap();
    let (owner, nonce) = born.owner_data_for_test();
    let original = f
        .preparation
        .inner
        .task
        .as_ref()
        .unwrap()
        .observe_original();
    let seed = born.born_repair_seed().unwrap().repair_seed().clone();
    assert!(f.preparation.inner.scope.is_none());
    assert!(f
        .layout
        .checked_store()
        .unwrap()
        .require_no_pending_v2_group(&f.group)
        .is_ok());
    assert!(f
        .preparation
        .inner
        .task
        .as_ref()
        .unwrap()
        .cold_initializing_seed_data()
        .is_some_and(|actual| Arc::ptr_eq(actual, &seed)));
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let writer = f
        .preparation
        .inner
        .writer
        .as_ref()
        .unwrap()
        .observe_original();
    f.preparation.retry(&f.layout, &mut NeverRng).unwrap();
    let current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
    assert!(Arc::ptr_eq(&owner, &current.scope.owner_identity()));
    assert_eq!(nonce, current.scope.nonce());
    original.require_same_original(&current.task).unwrap();
    current.writer.require_same_observation(&writer).unwrap();
    drop(f.preparation);
    current.validate_original(&f.layout).unwrap();
}

#[cfg(windows)]
#[test]
fn foreign_pin_before_handoff_preserves_claim_and_same_owner_succeeds_after_original_absence_restored(
) {
    let mut f = fixture(0xcb84);
    f.preparation.fail_after_born = true;
    assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
    let (owner, nonce) = f.preparation.born.as_ref().unwrap().owner_data_for_test();
    let before = f.pool.charged_for_test();
    std::fs::write(&f.pin, b"foreign pin must remain held").unwrap();
    assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
    assert_eq!(
        std::fs::read(&f.pin).unwrap(),
        b"foreign pin must remain held"
    );
    assert_eq!(std::fs::read(&f.seed).unwrap(), f.original);
    let task = f.preparation.inner.task.as_ref().unwrap();
    assert!(task.cold_initializing_seed_data().is_none());
    assert!(f
        .layout
        .checked_store()
        .unwrap()
        .require_no_pending_v2_group(&f.group)
        .is_err());
    assert_eq!(f.pool.charged_for_test(), before);
    std::fs::remove_file(&f.pin).unwrap(); // removes only the fixture's own foreign object
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
    assert!(Arc::ptr_eq(&owner, &current.scope.owner_identity()));
    assert_eq!(nonce, current.scope.nonce());
    current.validate_original(&f.layout).unwrap();
}

#[cfg(windows)]
#[test]
fn cancelled_born_owner_refuses_before_rng_and_wrapper_pressure_preserves_original_journal() {
    let mut f = fixture(0xcb85);
    let before = f.pool.charged_for_test();
    let pressure = f
        .pool
        .try_reserve(1024 * 1024 * 1024 - before - (WRAPPER_FEE - 1))
        .unwrap();
    let held = f.pool.charged_for_test();
    assert!(f.preparation.retry(&f.layout, &mut NeverRng).is_err());
    assert!(f.preparation.born.is_none() && f.preparation.inner.task.is_none());
    assert_eq!(f.pool.charged_for_test(), held);
    assert_eq!(std::fs::read(&f.seed).unwrap(), f.original);
    drop(pressure);
    f.preparation.fail_after_born = true;
    assert!(f.preparation.retry(&f.layout, &mut f.rng).is_err());
    let held = f.pool.charged_for_test();
    f.preparation.born.as_ref().unwrap().cancel_original();
    assert!(f.preparation.retry(&f.layout, &mut NeverRng).is_err());
    assert_eq!(f.pool.charged_for_test(), held);
    assert_eq!(std::fs::read(&f.seed).unwrap(), f.original);
    assert!(!f.pin.exists() && !f.head.exists() && !f.stage.exists());
}

#[cfg(windows)]
#[test]
fn newly_born_repaired_current_drains_then_new_mount_uses_existing_current_reader() {
    let mut f = fixture(0xcb86);
    let id = f.preparation.inner.graph.source().unwrap().data().source;
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let mut current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    current.validate_original(&f.layout).unwrap();
    let read_dropped = Arc::new(AtomicBool::new(false));
    let capture = Arc::get_mut(&mut current.resources.source.capture)
        .expect("actual output solely owns source capture");
    Arc::get_mut(&mut capture.fee)
        .expect("actual sole read invoice")
        .observe_drop_for_test(read_dropped.clone());
    let head = current.scope.saved_head().unwrap().clone();
    let writer = head.writer_observation().clone();
    drop(current);
    assert!(read_dropped.load(Ordering::SeqCst));
    assert!(writer.is_cancelled());
    drop(head);
    drop(writer);
    drop(f.preparation);
    drop(f.layout);
    use crate::store::critical_namespaces::{
        archive_bootstrap as archive, archive_current_namespace_codec as layout_codec,
    };
    let policy = crate::store::V2CurrentLayoutPolicy::new(
        crate::store::ArchiveBootstrapCatalogPolicy::new(
            f.pool.clone(),
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
        f.directory.path(),
        b"genuine task-free cold group reads",
        &mut f.rng,
        policy,
    )
    .unwrap();
    opening.retry(&mut f.rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    let read = layout
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
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
        Arc::new(Hub::new().join(PeerId::from_u64(0xcb87))),
        ChaCha20Rng::seed_from_u64(0xcb87),
        Arc::new(ManualClock::new(60)),
    );
    cold.retry(&layout, &mut f.rng).unwrap();
    let current = cold.take_current(&layout).unwrap().unwrap();
    current.validate_original(&layout).unwrap();
    assert_eq!(current.data().source, id);
    assert_eq!(std::fs::read(&f.seed).unwrap(), f.original);
}

#[cfg(not(windows))]
#[test]
fn unsupported_no_row_recovery_refuses_before_wrapper_fee_rng_or_journal_change() {
    let f = fixture_at_cut(0xcb88, ColdFixtureCut::CompleteSeedBeforeScope);
    let before = f.pool.charged_for_test();
    let mut prepared = PreparedColdV2GenesisPreInitializing::new(
        f.source,
        Arc::new(Hub::new().join(PeerId::from_u64(0xcb88))),
        ChaCha20Rng::seed_from_u64(0xcb88),
        Arc::new(ManualClock::new(60)),
    );
    assert!(prepared.retry(&f.layout, &mut NeverRng).is_err());
    assert_eq!(f.pool.charged_for_test(), before);
    assert!(prepared.capture.is_none() && prepared.born.is_none() && prepared.fee.is_none());
    assert!(prepared.inner.task.is_none());
    assert!(prepared.take_current(&f.layout).unwrap().is_none());
}
