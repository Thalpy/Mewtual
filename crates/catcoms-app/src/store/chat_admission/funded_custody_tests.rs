//! Real local ownership/accounting primitives, not Saved/Fresh/V2 Current proofs. Actual V2
//! call sites separately use their genuine saved artifacts; this fixture prepays a resource
//! reservation and observes the real Runtime/receiver/drop paths without a fake watch channel.
use super::*;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const FEE: usize = 16 * 1024;
struct PaidResource {
    store: std::sync::Weak<ServerStore>,
    drops: Arc<AtomicUsize>,
    outside_admission: Arc<AtomicBool>,
    // LAST: all test metadata drains before the real resource account refunds.
    _fee: CatalogReadReservation,
}
impl std::fmt::Debug for PaidResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaidResource").finish_non_exhaustive()
    }
}
impl Drop for PaidResource {
    fn drop(&mut self) {
        let outside = self
            .store
            .upgrade()
            .is_none_or(|store| store.chat_mount.runtime.try_lock().is_ok());
        self.outside_admission.store(outside, Ordering::SeqCst);
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
struct Fixture {
    store: Arc<ServerStore>,
    pool: CriticalCatalogReadPool,
    rng: ChaCha20Rng,
    _directory: tempfile::TempDir,
}
fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc0570d);
    let store = ServerStore::open(directory.path(), b"funded owner", &mut rng)
        .unwrap()
        .into_critical_namespace_store(b"funded owner", &mut rng)
        .unwrap();
    Fixture {
        store: Arc::new(store),
        pool: CriticalCatalogReadPool::new(4 * FEE).unwrap(),
        rng,
        _directory: directory,
    }
}
fn resource(f: &Fixture) -> (Arc<PaidResource>, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let drops = Arc::new(AtomicUsize::new(0));
    let outside = Arc::new(AtomicBool::new(false));
    (
        Arc::new(PaidResource {
            store: Arc::downgrade(&f.store),
            drops: drops.clone(),
            outside_admission: outside.clone(),
            _fee: f.pool.try_reserve(FEE).unwrap(),
        }),
        drops,
        outside,
    )
}
fn key() -> ChatAdmissionKey {
    ChatAdmissionKey::new(b"resource primitive", [7; 32]).unwrap()
}
fn record() -> Record {
    Record {
        nonce: [8; 32],
        revision: 1,
        phase: Phase::Initializing,
        origin: Origin::Legacy,
        provenance: [9; 32],
    }
}
fn install(f: &Fixture, paid: Arc<PaidResource>) -> ChatAdmissionPreparation {
    let key = key();
    let row = record();
    let (preparation, owner) = f
        .store
        .new_reservation_with_custody(&key, &row, paid)
        .unwrap();
    let mut runtime = f.store.chat_mount.locked().unwrap();
    runtime.table.as_mut().unwrap().insert(key.clone(), row);
    runtime.owners.insert(key, owner);
    preparation
}

#[test]
fn runtime_owner_alone_keeps_actual_payment_after_preparation_and_observers_drop() {
    let f = fixture();
    let (paid, drops, outside) = resource(&f);
    let weak = Arc::downgrade(&paid);
    let prepared = install(&f, paid.clone());
    drop(paid);
    drop(prepared);
    assert_eq!(weak.strong_count(), 1, "only Runtime owner remains");
    assert_eq!(f.pool.charged_for_test(), FEE);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(f.store);
    assert_eq!(f.pool.charged_for_test(), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(outside.load(Ordering::SeqCst));
}

#[test]
fn mismatch_detaches_only_resource_and_drops_it_after_original_admission_unlocks() {
    let f = fixture();
    let (paid, drops, outside) = resource(&f);
    drop(install(&f, paid.clone()));
    drop(paid);
    {
        let mut runtime = f.store.chat_mount.locked().unwrap();
        runtime
            .table
            .as_mut()
            .unwrap()
            .get_mut(&key())
            .unwrap()
            .revision = 2;
    }
    // Actual generic capture invokes reservation_locked on the mismatched original. No V2
    // source or Current is inferred; this test reaches the real delayed-resource caller.
    let captured = f.store.capture_chat_admission(key()).unwrap();
    assert_eq!(f.pool.charged_for_test(), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(outside.load(Ordering::SeqCst));
    drop(captured);
}

#[test]
fn replacement_preserves_old_receiver_close_before_error_flush_and_defers_last_resource() {
    let mut f = fixture();
    let capture = f.store.capture_chat_admission(key()).unwrap();
    let (paid, drops, outside) = resource(&f);
    let row = record();
    let (prepared, owner) = f
        .store
        .new_reservation_with_custody(&key(), &row, paid.clone())
        .unwrap();
    // An orphan owner is a NEGATIVE mismatch fixture, not a fake Saved/Current. The actual
    // replacement code must not drop its possible last metadata resource under admission.
    f.store
        .chat_mount
        .locked()
        .unwrap()
        .owners
        .insert(key(), owner);
    drop(prepared);
    drop(paid);
    f.store
        .chat_fence_pre_rename_failure
        .store(true, Ordering::SeqCst);
    assert!(f
        .store
        .prepare_legacy_chat_admission(&capture, b"actual replacement", &mut f.rng)
        .is_err());
    assert_eq!(f.pool.charged_for_test(), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(outside.load(Ordering::SeqCst));
}

#[test]
fn retirement_error_keeps_detached_payment_until_original_admission_ends() {
    let mut f = fixture();
    let (paid, drops, outside) = resource(&f);
    drop(install(&f, paid.clone()));
    drop(paid);
    f.store
        .chat_parent_sync_failure
        .store(true, Ordering::SeqCst);
    {
        // This is the exact outer declaration order used by public retirement/recovery.
        let mut retired = None;
        {
            let mut runtime = f.store.chat_mount.locked().unwrap();
            assert!(f
                .store
                .retire_chat_row(
                    &mut runtime,
                    &key(),
                    record().nonce,
                    1,
                    &mut f.rng,
                    &mut retired
                )
                .is_err());
            assert!(retired.is_some());
            assert_eq!(f.pool.charged_for_test(), FEE);
            assert_eq!(drops.load(Ordering::SeqCst), 0);
            assert!(!runtime.owners.contains_key(&key()));
        }
        assert_eq!(drops.load(Ordering::SeqCst), 0);
    }
    assert_eq!(f.pool.charged_for_test(), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(outside.load(Ordering::SeqCst));
}

#[test]
fn full_retired_holder_refuses_before_mutating_or_revoking_another_owner() {
    let f = fixture();
    let (paid, _, _) = resource(&f);
    let prepared = install(&f, paid.clone());
    let mut retired: Option<SharedRequestKeepalive> = Some(paid);
    let mut runtime = f.store.chat_mount.locked().unwrap();
    let mut mismatch = record();
    mismatch.revision = 2;
    let identity = runtime.owners.get(&key()).unwrap().life.identity();
    assert!(f
        .store
        .reservation_locked(&mut runtime, &key(), &mismatch, &mut retired)
        .is_err());
    assert!(Arc::ptr_eq(
        &identity,
        &runtime.owners.get(&key()).unwrap().life.identity()
    ));
    assert!(!prepared.admission.is_cancelled());
}

#[test]
fn legacy_none_reservation_and_exact_owner_binder_never_invent_resource_or_owner() {
    let f = fixture();
    let mut row = record();
    row.phase = Phase::Current;
    row.revision = 2;
    let (prepared, owner) = f.store.new_reservation(&key(), &row).unwrap();
    assert!(owner.custody.is_none());
    assert!(prepared.admission.keepalive().is_none());
    let identity = prepared.owner_identity.clone();
    let mut runtime = f.store.chat_mount.locked().unwrap();
    runtime.table.as_mut().unwrap().insert(key(), row);
    assert!(f
        .store
        .bind_owned_chat_admission_locked(&runtime, &key(), record().nonce, 2, &identity)
        .is_err());
    assert!(runtime.owners.is_empty(), "exact bind cannot mint an owner");
    runtime.owners.insert(key(), owner);
    let binding = f
        .store
        .bind_owned_chat_admission_locked(&runtime, &key(), record().nonce, 2, &identity)
        .unwrap();
    assert!(binding.publication.keepalive().is_none());
    assert!(Arc::ptr_eq(&binding.owner_identity, &identity));
    assert_eq!(f.pool.charged_for_test(), 0);
}
