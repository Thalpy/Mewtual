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
    sync::{
        atomic::{AtomicU8, Ordering},
        mpsc::sync_channel,
    },
    time::Duration,
};

type WorkSlotHook = Option<(bool, Box<dyn FnOnce()>)>;
thread_local! {
    static BEFORE_WORK_SLOT: std::cell::RefCell<WorkSlotHook> = const {std::cell::RefCell::new(None)};
    static AFTER_WORK_SLOT: std::cell::RefCell<WorkSlotHook> = const {std::cell::RefCell::new(None)};
    static WORK_SLOT_CLAIMS: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};
}
pub(super) fn after_work_slot(ui: bool) {
    let hook = AFTER_WORK_SLOT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.as_ref().is_some_and(|(kind, _)| *kind == ui) {
            slot.take()
        } else {
            None
        }
    });
    if let Some((_, hook)) = hook {
        hook();
    }
}
pub(super) fn before_work_slot(ui: bool) {
    let hook = BEFORE_WORK_SLOT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.as_ref().is_some_and(|(kind, _)| *kind == ui) {
            slot.take()
        } else {
            None
        }
    });
    if let Some((_, hook)) = hook {
        hook();
    }
}
pub(super) fn work_slot_claimed() {
    WORK_SLOT_CLAIMS.with(|calls| calls.set(calls.get() + 1));
}

struct InlineWorkWake {
    native: Arc<Mutex<()>>,
    wakes: std::sync::atomic::AtomicUsize,
    under_guard: std::sync::atomic::AtomicBool,
}
impl std::task::Wake for InlineWorkWake {
    fn wake(self: Arc<Self>) {
        if self.native.try_lock().is_err() {
            self.under_guard.store(true, Ordering::SeqCst);
        }
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }
}
type WaitingWork = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

#[test]
fn native_work_retirement_post_cas_close_keeps_both_whole_capsules_until_outside_guard_drop() {
    use std::task::{Context, Poll, Waker};
    for ui in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xfa15 + u64::from(ui));
        let owner = mounted(dir.path(), &mut rng);
        let native = Arc::new(Mutex::new(()));
        let wake = Arc::new(InlineWorkWake {
            native: native.clone(),
            wakes: std::sync::atomic::AtomicUsize::new(0),
            under_guard: std::sync::atomic::AtomicBool::new(false),
        });
        let waker = Waker::from(wake.clone());
        let waiting = std::rc::Rc::new(std::cell::RefCell::new(None::<WaitingWork>));
        let result = waiting.clone();
        let closing = owner.physical.clone();
        let registered = waker.clone();
        AFTER_WORK_SLOT.with(|slot| {
            *slot.borrow_mut() = Some((
                ui,
                Box::new(move || {
                    closing.close_admission();
                    let original = closing.observe_closed_work();
                    assert!(
                        !original.is_drained(),
                        "the actual admitted capsule owns ACTIVE"
                    );
                    let mut future: WaitingWork = Box::pin(async move { original.wait().await });
                    assert!(future
                        .as_mut()
                        .poll(&mut Context::from_waker(&registered))
                        .is_pending());
                    *result.borrow_mut() = Some(future); // keep the real registered waiter alive
                }),
            ))
        });
        let (mut metadata, ui_work) = {
            let _held = native.lock().unwrap();
            let metadata = if ui {
                None
            } else {
                owner.capture_metadata_work()
            };
            let ui_work = if ui { owner.capture_ui_work() } else { None };
            assert_eq!(
                wake.wakes.load(Ordering::SeqCst),
                0,
                "capture must not notify under the simulated native Store writer"
            );
            assert!(
                if ui {
                    ui_work.is_some()
                } else {
                    metadata.is_some()
                },
                "successful CAS must return its whole stale capsule"
            );
            (metadata, ui_work)
        };
        // Passive capture grants no late IO: its ordinary operations reject
        // the closed original outside the guard, without discarding custody.
        if let Some(work) = metadata.as_mut() {
            assert!(work.row_count_data().is_err());
        }
        if let Some(work) = ui_work.as_ref() {
            assert!(work.read().is_err());
        }
        drop(metadata);
        drop(ui_work);
        assert_eq!(
            wake.wakes.load(Ordering::SeqCst),
            1,
            "both kinds must notify their real registered waiter"
        );
        assert!(!wake.under_guard.load(Ordering::SeqCst));
        let mut future = waiting
            .borrow_mut()
            .take()
            .expect("actual original wait was registered");
        assert_eq!(
            future.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Ready(())
        );
    }
}

#[test]
fn native_work_retirement_close_between_actual_open_check_and_slot_cas_prevents_both_physical_clones(
) {
    for ui in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xfa10 + u64::from(ui));
        let owner = mounted(dir.path(), &mut rng);
        let closing = owner.physical.clone();
        let reached = Arc::new(AtomicU8::new(0));
        let seen = reached.clone();
        BEFORE_WORK_SLOT.with(|slot| {
            *slot.borrow_mut() = Some((
                ui,
                Box::new(move || {
                    closing.close_admission();
                    let original = closing.observe_closed_work();
                    assert!(
                        original.is_drained(),
                        "both original slots closed before delayed CAS"
                    );
                    seen.store(1, Ordering::SeqCst);
                }),
            ))
        });
        let before = WORK_SLOT_CLAIMS.with(|calls| calls.get());
        if ui {
            assert!(owner.capture_ui_work().is_none());
        } else {
            assert!(owner.capture_metadata_work().is_none());
        }
        assert_eq!(reached.load(Ordering::SeqCst), 1);
        assert_eq!(
            WORK_SLOT_CLAIMS.with(|calls| calls.get()),
            before,
            "no actual slot admission/Physical-clone branch"
        );
        assert!(owner.physical.observe_closed_work().is_drained());
    }
}

#[test]
fn native_work_retirement_metadata_ui_zero_does_not_clear_independent_completed_debt_lease() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xfa12);
    let owner = mounted(dir.path(), &mut rng);
    let input = request(&owner);
    let mut work = owner.capture_metadata_work().unwrap();
    let (_, debt) = work.rename_metadata_data(&input, &mut rng).unwrap();
    assert!(
        debt.is_some(),
        "actual completed index IO has outgoing custody"
    );
    owner.close_admission();
    let original = owner.observe_closed_work();
    assert!(!original.is_drained());
    drop(work);
    assert!(
        original.is_drained(),
        "only the metadata/UI capsule scopes drained"
    );
    drop(owner);
    assert!(
        busy(dir.path()),
        "completed debt has its own actual physical-session lease"
    );
    drop(debt);
    drop(reopen(dir.path())); // observer alone holds no Physical or cold evidence
}

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
        assert!(!owner.physical.slot.active_for_test());
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
                    .is_some_and(|slot| slot.active_for_test());
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
