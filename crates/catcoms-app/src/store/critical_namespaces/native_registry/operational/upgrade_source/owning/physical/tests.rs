//! Real original mounts, cell effects and affine Store Drop. Physical custody
//! is not a native command credential, SDK entry or proof of joined membership.
use super::*;
use crate::store::critical_namespaces::native_registry::{
    lifecycle::{
        cell::tests::request,
        preparation::{Policy, PREPARATION_FEE},
        wal::tests::mounted,
    },
    preparation::tests::{current_policy, NeverRng, SECRET},
};
use crate::store::persistence::native_registry::operational as cuts;
use crate::store::{CriticalCatalogReadPool, NativeIndexStore};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::{
    fs,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{atomic::AtomicU8, mpsc::sync_channel},
    time::Duration,
};

fn index(path: &Path) -> PathBuf {
    path.join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(crate::store::critical_namespaces::native_registry::preparation::INDEX_NAME)
}
fn reopen(path: &Path) -> Store {
    NativeIndexStore::open_lifecycle3_for_test(path, SECRET, &mut NeverRng, current_policy())
        .unwrap()
}
fn busy(path: &Path) -> bool {
    matches!(
        NativeIndexStore::open_lifecycle3_for_test(path, SECRET, &mut NeverRng, current_policy()),
        Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
    )
}

#[test]
fn lifecycle3_source_work_retains_original_without_occupying_metadata_or_ui_slots() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc830);
    let owner = mounted(dir.path(), &mut rng);
    let source = owner.capture_source_work().unwrap();
    let mut metadata = owner.capture_metadata_work().unwrap();
    let ui = owner.capture_ui_work().unwrap();
    assert!(source.is_live_cached());
    assert_eq!(metadata.row_count_data().unwrap(), 1);
    assert!(
        source.metadata().is_none(),
        "one index slot remains exclusive"
    );
    drop(metadata);
    assert_eq!(source.metadata().unwrap().row_count_data().unwrap(), 1);
    drop(ui);
    assert!(owner.capture_ui_work().is_some());
}

#[test]
fn lifecycle3_source_work_owner_close_is_negative_and_actual_files_retire_last() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc831);
    let owner = mounted(dir.path(), &mut rng);
    let source = owner.capture_source_work().unwrap();
    drop(owner);
    assert!(!source.is_live_cached());
    assert!(source.metadata().is_none());
    assert!(
        busy(dir.path()),
        "source custody still owns the actual files"
    );
    drop(source);
    drop(reopen(dir.path()));
}

#[test]
fn lifecycle3_physical_work_capture_uses_one_cached_slot_without_entering_the_cell() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc801);
    let owner = mounted(dir.path(), &mut rng);
    let guard = owner.physical.cell.lock().unwrap();
    crate::store::chat_admission::take_revocation_observer_constructions();
    let mut work = owner
        .capture_metadata_work()
        .expect("prepaid slot capture is lock-free");
    assert!(
        owner.capture_metadata_work().is_none(),
        "one original capsule, not an Arc-based capacity increase"
    );
    assert_eq!(
        crate::store::chat_admission::take_revocation_observer_constructions(),
        0
    );
    assert!(
        work.row_count_data().is_err(),
        "actual reads still acquire the real cell"
    );
    drop(guard);
    assert_eq!(work.row_count_data().unwrap(), 1);
    drop(work);
    let mut next = owner.capture_metadata_work().unwrap();
    assert_eq!(next.row_count_data().unwrap(), 1);
}

#[test]
fn lifecycle3_physical_work_capture_refuses_each_original_negative_without_taking_slot() {
    for (n, close_gate) in [true, false].into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc803 + n as u64);
        let owner = mounted(dir.path(), &mut rng);
        if close_gate {
            owner.physical.handler.close();
            assert!(
                owner
                    .physical
                    .current()
                    .unwrap()
                    .physical_mount_is_live_cached(),
                "gate-close control must leave the independent mount live"
            );
        } else {
            owner.actual_store_for_test().unwrap().revoke_chat_mount();
            assert!(
                owner.physical.handler.is_open_cached(),
                "mount-revocation control must leave the independent gate open"
            );
        }
        crate::store::chat_admission::take_revocation_observer_constructions();
        assert!(owner.capture_metadata_work().is_none());
        assert!(!owner.physical.slot.taken.load(Ordering::SeqCst));
        assert_eq!(
            crate::store::chat_admission::take_revocation_observer_constructions(),
            0
        );
    }
}

#[test]
fn lifecycle3_physical_work_can_recapture_the_same_active_transaction_for_exact_retry() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc802);
    let mut owner = mounted(dir.path(), &mut rng);
    let input = request(&owner);
    let mut work = owner.capture_metadata_work().unwrap();
    cuts::arm_fault(cuts::Fault::IndexStage);
    assert!(work.rename_metadata_data(&input, &mut rng).is_err());
    assert!(cuts::take_fault_seen());
    drop(work);
    assert!(owner.row_count_data().is_err());
    let mut retry = owner
        .capture_metadata_work()
        .expect("active claim is custody, not a closed mount");
    let edited = rename::Request {
        id: input.id,
        identity: input.identity,
        expected_revision: input.expected_revision,
        name: "edited retry".into(),
    };
    assert!(retry.rename_metadata_data(&edited, &mut NeverRng).is_err());
    let (covered, debt) = retry.rename_metadata_data(&input, &mut NeverRng).unwrap();
    assert_eq!(covered.revision, input.expected_revision + 1);
    assert!(owner.row_count_data().is_err());
    drop(debt);
    assert_eq!(owner.row_count_data().unwrap(), 1);
}

#[test]
fn lifecycle3_physical_work_actual_owner_drop_orders_both_sides_of_worker_admission() {
    for (n, edge) in [Edge::BeforeMetadataAdmission, Edge::AfterMetadataCommit]
        .into_iter()
        .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc810 + n as u64);
        let owner = mounted(dir.path(), &mut rng);
        let input = request(&owner);
        let work = owner.capture_metadata_work().unwrap();
        let original = fs::read(index(dir.path())).unwrap();
        let (entered, seen) = sync_channel(1);
        let (release, wait) = sync_channel(1);
        let edge_result = Arc::new(AtomicU8::new(0));
        let observed = edge_result.clone();
        let worker = std::thread::spawn(move || {
            at_edge(
                edge,
                Box::new(move || {
                    let entered = entered.try_send(()).is_ok();
                    let released = wait.recv_timeout(Duration::from_secs(10)).is_ok();
                    observed.store(if entered && released { 1 } else { 2 }, Ordering::SeqCst);
                }),
            );
            let mut work = work;
            let mut rng = ChaCha20Rng::seed_from_u64(0xc820 + n as u64);
            let refused = work.rename_metadata_data(&input, &mut rng).is_err();
            (refused, work, input)
        });
        let ready = seen.recv_timeout(Duration::from_secs(10));
        drop(owner); // FIRST real affine closer, while worker owns actual custody
        let _ = release.try_send(());
        let joined = worker.join(); // always release/join before assertions
        assert!(ready.is_ok());
        let (refused, mut work, input) = joined.unwrap();
        assert_eq!(edge_result.load(Ordering::SeqCst), 1);
        assert!(refused, "closed original claim cannot expose warm success");
        assert!(work.row_count_data().is_err());
        assert!(
            busy(dir.path()),
            "real worker retains the original physical session"
        );
        if edge == Edge::BeforeMetadataAdmission {
            assert_eq!(fs::read(index(dir.path())).unwrap(), original);
        }
        drop(work);
        let mut cold = reopen(dir.path());
        let actual = request(&cold);
        assert_eq!(
            actual.expected_revision,
            u64::from(edge == Edge::AfterMetadataCommit)
        );
        if edge == Edge::AfterMetadataCommit {
            let (observed, unchanged) = cold.rename_metadata_data(&actual, &mut NeverRng).unwrap();
            assert_eq!(observed.revision, input.expected_revision + 1);
            assert!(unchanged.is_none());
        }
    }
}

#[test]
fn lifecycle3_physical_work_last_arc_drains_actual_io_before_slot_and_payment() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc831);
    let owner = mounted(dir.path(), &mut rng);
    let input = request(&owner);
    let mut work = owner.capture_metadata_work().unwrap();
    cuts::arm_fault(cuts::Fault::IndexStage);
    assert!(work.rename_metadata_data(&input, &mut rng).is_err());
    assert!(cuts::take_fault_seen());
    let slot = Arc::downgrade(&owner.physical.slot);
    let observed_slot = slot.clone();
    let pool = owner.physical.handler.pool().clone();
    let observed_pool = pool.clone();
    let observed = Arc::new(AtomicU8::new(0));
    let recorded = observed.clone();
    let path = dir.path().to_path_buf();
    owner
        .physical
        .cell
        .lock()
        .unwrap()
        .observe_pending_io_retirement_for_test(Box::new(move || {
            // This callback runs in an actual IO destructor. Never assert/panic;
            // record observations and check them after the whole work has retired.
            let flags = catch_unwind(AssertUnwindSafe(|| {
                let occupied = observed_slot
                    .upgrade()
                    .is_some_and(|slot| slot.taken.load(Ordering::SeqCst));
                let funded = observed_pool.charged_for_test() == PREPARATION_FEE;
                let excluded = busy(&path);
                u8::from(occupied) | (u8::from(funded) << 1) | (u8::from(excluded) << 2)
            }))
            .unwrap_or(0);
            recorded.store(flags, Ordering::SeqCst);
        }));
    drop(owner);
    assert!(busy(dir.path()));
    drop(work); // actual last Physical: Cell/IO -> Current -> Handler -> SlotAck
    assert_eq!(observed.load(Ordering::SeqCst), 7);
    assert!(slot.upgrade().is_none(), "no slot/Physical/Handler cycle");
    assert_eq!(pool.charged_for_test(), 0);
    assert_eq!(request(&reopen(dir.path())).expected_revision, 1);
}

#[test]
fn lifecycle3_physical_work_outgoing_debt_excludes_new_work_after_capsule_and_owner_drop() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc841);
    let owner = mounted(dir.path(), &mut rng);
    let input = request(&owner);
    let mut work = owner.capture_metadata_work().unwrap();
    let (_, debt) = work.rename_metadata_data(&input, &mut rng).unwrap();
    let debt = debt.unwrap();
    drop(work);
    let mut next = owner.capture_metadata_work().unwrap();
    assert!(next.row_count_data().is_err());
    assert!(
        next.rename_metadata_data(&input, &mut NeverRng).is_err(),
        "slot availability cannot replace claim retirement"
    );
    drop(next);
    drop(owner);
    assert!(
        busy(dir.path()),
        "completed debt still owns its original finite physical lease"
    );
    drop(debt);
    let cold = reopen(dir.path());
    assert_eq!(request(&cold).expected_revision, 1);
}

#[test]
fn lifecycle3_physical_work_never_clears_the_original_cell_poison() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc851);
    let mut owner = mounted(dir.path(), &mut rng);
    let input = request(&owner);
    let mut work = owner.capture_metadata_work().unwrap();
    at_edge(
        Edge::AfterMetadataCommit,
        Box::new(|| panic!("actual physical worker effect")),
    );
    assert!(catch_unwind(AssertUnwindSafe(
        || work.rename_metadata_data(&input, &mut rng)
    ))
    .is_err());
    assert!(owner.row_count_data().is_err());
    assert!(work.row_count_data().is_err());
    drop(work);
    let mut replacement = owner.capture_metadata_work().unwrap();
    assert!(replacement
        .rename_metadata_data(&input, &mut NeverRng)
        .is_err());
    drop(replacement);
    drop(owner);
    assert_eq!(request(&reopen(dir.path())).expected_revision, 1);
}

#[test]
fn lifecycle3_physical_work_private_empty_assembly_cannot_read_write_or_capture() {
    // Only paid metadata exists here: no Current, source, node or ready state.
    let pool = CriticalCatalogReadPool::new(PREPARATION_FEE).unwrap();
    let policy = Policy::new(pool.clone(), Policy::required_persistent_capacity()).unwrap();
    let handler =
        lifecycle_admission::Handler::from_reserved(&policy, policy.reserve().unwrap()).unwrap();
    let physical = Physical::allocate(handler);
    assert!(physical.capture().is_none());
    assert!(physical.row_count_data().is_err());
    let absent = rename::Request {
        id: 1,
        identity: [1; 32],
        expected_revision: 0,
        name: "negative fixture".into(),
    };
    assert!(physical
        .rename_metadata_data(&absent, &mut NeverRng)
        .is_err());
    drop(physical);
    assert_eq!(pool.charged_for_test(), 0);
}
