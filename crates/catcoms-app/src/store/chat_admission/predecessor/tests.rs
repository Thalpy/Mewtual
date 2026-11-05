//! Admission-family negative ownership tests. The confirm callback is a trusted internal test
//! fixture, not evidence of a saved JOIN/ARCHIVE source. Actual JOIN producers are tested apart.
//! Authored UNRUN under the compiler/test/build pause.
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};

fn fixture() -> (tempfile::TempDir, ServerStore, ChaCha20Rng) {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc201);
    let store = ServerStore::open(dir.path(), b"predecessor fixture", &mut rng)
        .unwrap()
        .into_critical_namespace_store(b"predecessor fixture", &mut rng)
        .unwrap();
    (dir, store, rng)
}
fn key(n: u8) -> ChatAdmissionKey {
    ChatAdmissionKey::new(&[n], [n; 32]).unwrap()
}
fn pool() -> CriticalCatalogReadPool {
    CriticalCatalogReadPool::new(1024 * 1024).unwrap()
}
fn register(
    store: &ServerStore,
    capture: &ChatAdmissionCapture,
    source: u8,
    revision: u64,
) -> OriginalPredecessorTask {
    let mut prepared =
        PreparedOriginalPredecessor::join_checkpoint(capture, [source; 32], revision, &pool())
            .unwrap();
    store
        .register_original_predecessor(capture, &mut prepared, |permit| permit.validate_original())
        .unwrap();
    prepared.into_task()
}
fn issue(
    store: &ServerStore,
    key: ChatAdmissionKey,
    rng: &mut ChaCha20Rng,
) -> ChatAdmissionBinding {
    let capture = store.capture_chat_admission(key).unwrap();
    let prepared = store
        .prepare_legacy_chat_admission(&capture, b"real negative fixture", rng)
        .unwrap();
    store
        .finish_chat_admission(&prepared, rng, |_, _| Ok(()))
        .unwrap()
        .into_binding()
}

/// Rejecting before BOTH first issuance and old issued retry means no nonce/sealing RNG can
/// be reached. A panicking fixture makes accidentally admitted work fail deterministically.
struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("protected predecessor reached RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("protected predecessor reached RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("protected predecessor reached RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("protected predecessor reached RNG")
    }
}
impl CryptoRng for NeverRng {}

#[test]
fn absent_and_retired_original_exclusions_precede_generic_fresh_and_legacy_rng() {
    let (_dir, store, mut rng) = fixture();
    let absent = store.capture_chat_admission(key(1)).unwrap();
    let absent_task = register(&store, &absent, 1, 1);
    assert!(store
        .prepare_legacy_chat_admission(&absent, b"replacement", &mut NeverRng)
        .is_err());
    assert!(store
        .prepare_fresh_chat_admission(&absent, [1; 32], &mut NeverRng)
        .is_err());
    let binding = issue(&store, key(2), &mut rng);
    store.retire_chat_admission(&binding, &mut rng).unwrap();
    let retired = store.capture_chat_admission(key(2)).unwrap();
    let retired_task = register(&store, &retired, 2, 1);
    assert!(store
        .prepare_fresh_chat_admission(&retired, [2; 32], &mut NeverRng)
        .is_err());
    assert!(store
        .prepare_legacy_chat_admission(&retired, b"replacement", &mut NeverRng)
        .is_err());
    assert!(!absent_task.is_closed());
    assert!(!retired_task.is_closed());
}

#[test]
fn exclusion_precedes_old_issued_retry_and_dirty_flush_without_revoking_new_source() {
    let (_dir, store, mut rng) = fixture();
    let old = store.capture_chat_admission(key(3)).unwrap();
    store
        .prepare_legacy_chat_admission(&old, b"old issued original", &mut rng)
        .unwrap();
    let initializing = store.capture_chat_admission(key(3)).unwrap();
    let task = register(&store, &initializing, 3, 1);
    store.chat_mount.runtime.lock().unwrap().dirty = true;
    assert!(store
        .prepare_legacy_chat_admission(&old, b"old issued original", &mut NeverRng)
        .is_err());
    assert!(
        store.chat_mount.runtime.lock().unwrap().dirty,
        "old retry cannot flush protected metadata"
    );
    assert!(!task.is_closed());
}

#[test]
fn real_current_and_retire_mutations_close_exact_basis_even_when_retire_save_fails() {
    let (_dir, store, mut rng) = fixture();
    let capture = store.capture_chat_admission(key(4)).unwrap();
    let original = store
        .prepare_legacy_chat_admission(&capture, b"old original", &mut rng)
        .unwrap();
    let initializing = store.capture_chat_admission(key(4)).unwrap();
    let first = register(&store, &initializing, 4, 1);
    let binding = store
        .finish_chat_admission(&original, &mut rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    assert!(
        first.is_closed(),
        "actual phase change invalidates the old exact basis"
    );
    // A separate key starts with no predecessor debt, then exercises the actual pre-rename fault.
    let other = issue(&store, key(5), &mut rng);
    let current = store.capture_chat_admission(key(5)).unwrap();
    let task = register(&store, &current, 5, 1);
    store
        .chat_fence_pre_rename_failure
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(store.retire_chat_admission(&other, &mut rng).is_err());
    assert!(
        task.is_closed(),
        "negative closure precedes fallible file replacement"
    );
    assert!(store.chat_mount.runtime.lock().unwrap().dirty);
    store.repair_chat_admission_inventory(&mut rng).unwrap();
    assert!(
        store
            .with_chat_admission_storage(&binding, |_| Ok(()))
            .is_ok(),
        "unrelated existing control survives"
    );
}

#[test]
fn old_already_retired_and_already_current_noops_do_not_close_new_same_basis_source() {
    let (_dir, store, mut rng) = fixture();
    let binding = issue(&store, key(6), &mut rng);
    store.retire_chat_admission(&binding, &mut rng).unwrap();
    let retired = store.capture_chat_admission(key(6)).unwrap();
    let task = register(&store, &retired, 6, 1);
    store.retire_chat_admission(&binding, &mut rng).unwrap();
    assert!(
        !task.is_closed(),
        "same-nonce Retired flush is not a new row mutation"
    );

    let capture = store.capture_chat_admission(key(7)).unwrap();
    let preparation = store
        .prepare_legacy_chat_admission(&capture, b"old accepted", &mut rng)
        .unwrap();
    store
        .finish_chat_admission(&preparation, &mut rng, |_, _| Ok(()))
        .unwrap();
    let current = store.capture_chat_admission(key(7)).unwrap();
    let current_task = register(&store, &current, 7, 1);
    let result = store
        .finish_chat_admission(&preparation, &mut rng, |_, _| {
            panic!("AlreadyCurrent must not save")
        })
        .unwrap();
    assert!(matches!(result, ChatAdmissionFinish::AlreadyCurrent(_)));
    assert!(!current_task.is_closed());
}

#[test]
fn weak_exclusion_never_keeps_task_alive_or_rebinds_same_revision_and_stale_identity_stays_old() {
    let (_dir, store, _) = fixture();
    let capture = store.capture_chat_admission(key(8)).unwrap();
    let old = register(&store, &capture, 8, 1);
    let negative = old.observer();
    let identity = old.identity();
    drop(old);
    assert!(negative.is_cancelled());
    let mut same =
        PreparedOriginalPredecessor::join_checkpoint(&capture, [8; 32], 1, &pool()).unwrap();
    assert!(store
        .register_original_predecessor(&capture, &mut same, |p| p.validate_original())
        .is_err());
    let new = register(&store, &capture, 8, 3);
    assert!(!Arc::ptr_eq(&identity, &new.identity()));
    store
        .close_original_join_predecessor(&capture, &[8; 32], 1)
        .unwrap();
    assert!(!new.is_closed());
    assert!(store
        .prepare_legacy_chat_admission(&capture, b"no weak-expiry bypass", &mut NeverRng)
        .is_err());
}

#[test]
fn five_unresolved_slots_are_bounded_and_existing_saved_group_needs_no_new_slot() {
    let (_dir, store, mut rng) = fixture();
    let saved = issue(&store, key(20), &mut rng);
    let mut tasks = Vec::new();
    for n in 10..15 {
        let capture = store.capture_chat_admission(key(n)).unwrap();
        tasks.push(register(&store, &capture, n, 1));
    }
    let capture = store.capture_chat_admission(key(15)).unwrap();
    let mut sixth =
        PreparedOriginalPredecessor::join_checkpoint(&capture, [15; 32], 1, &pool()).unwrap();
    assert!(store
        .register_original_predecessor(&capture, &mut sixth, |p| p.validate_original())
        .is_err());
    drop(tasks);
    assert!(
        store
            .register_original_predecessor(&capture, &mut sixth, |p| p.validate_original())
            .is_err(),
        "expiry cannot reclaim durable debt by inference"
    );
    assert!(store
        .with_chat_admission_storage(&saved, |_| Ok(()))
        .is_ok());
}

#[test]
fn archive_kind_is_negative_exclusion_without_join_schedule_renewal_or_scope_permission() {
    let (_dir, store, _) = fixture();
    let capture = store.capture_chat_admission(key(21)).unwrap();
    let pool = pool();
    let mut prepared =
        PreparedOriginalPredecessor::archive_founder(&capture, [21; 32], 1, &pool).unwrap();
    store
        .register_original_predecessor(&capture, &mut prepared, |p| p.validate_original())
        .unwrap();
    let task = prepared.into_task();
    assert!(store
        .prepare_legacy_chat_admission(&capture, b"no scope permission from kind", &mut NeverRng)
        .is_err());
    task.close();
    let mut later =
        PreparedOriginalPredecessor::archive_founder(&capture, [21; 32], 2, &pool).unwrap();
    assert!(
        store
            .register_original_predecessor(&capture, &mut later, |p| p.validate_original())
            .is_err(),
        "archive source/debt handoff requires its actual future consumer"
    );
    assert!(store
        .chat_mount
        .runtime
        .lock()
        .unwrap()
        .table
        .as_ref()
        .unwrap()
        .get(&key(21))
        .is_none());
}
