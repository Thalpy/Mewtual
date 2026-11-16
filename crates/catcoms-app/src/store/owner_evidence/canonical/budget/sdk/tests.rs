//! Actual shared-account resource tests; no source/scope/current permissions are constructed.
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
const SECRET: &[u8] = b"real SDK resource pool fixture";
const GRAPH: usize = 128 * 1024;
const WORK: usize = 512 * 1024 * 1024;
fn budget() -> OwnerEvidenceBudget {
    OwnerEvidenceBudget::checkpoint(
        4,
        16 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap()
    .with_closed_sdk_restore(GRAPH, WORK)
    .unwrap()
}
fn open(path: &std::path::Path, rng: &mut ChaCha20Rng) -> ServerStore {
    let store = ServerStore::open(path, SECRET, rng)
        .unwrap()
        .into_owner_evidence_store(SECRET, rng)
        .unwrap();
    store.configure_owner_evidence_budget(budget()).unwrap();
    store
}
#[test]
fn typed_policy_is_explicit_immutable_and_requires_two_graph_overlap_capacity() {
    let base = OwnerEvidenceBudget::checkpoint(
        4,
        16 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap();
    assert!(base.sdk.is_none());
    assert!(base.with_closed_sdk_restore(0, WORK).is_err());
    assert!(base.with_closed_sdk_restore(GRAPH, 0).is_err());
    assert!(base.with_closed_sdk_restore(1024 * 1024, WORK).is_err());
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5d01);
    let store = open(dir.path(), &mut rng);
    let changed = base.with_closed_sdk_restore(2 * GRAPH, WORK).unwrap();
    assert!(store.configure_owner_evidence_budget(changed).is_err());
    assert_eq!(
        store
            .owner_evidence
            .canonical
            .lock()
            .unwrap()
            .budget
            .unwrap(),
        budget()
    );
}
#[test]
fn independent_graph_debits_survive_handle_and_store_drop_without_retaining_vault_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5d02);
    let store = open(dir.path(), &mut rng);
    let original = store.owner_evidence.canonical.clone();
    let weak = Arc::downgrade(&original);
    let (child, parent) = store.reserve_owner_sdk_graphs(true).unwrap();
    assert_eq!(original.lock().unwrap().hot_bytes, 2 * GRAPH);
    drop(child);
    assert_eq!(original.lock().unwrap().hot_bytes, GRAPH);
    drop(store);
    drop(original);
    let account = weak
        .upgrade()
        .expect("resource debt retains SAME original accounting");
    // Accounting has no Store/keys/session: a replacement physical mount can acquire its lock.
    let reopened = ServerStore::open(dir.path(), SECRET, &mut rng).unwrap();
    drop(reopened);
    assert_eq!(account.lock().unwrap().hot_bytes, GRAPH);
    drop(parent);
    assert_eq!(account.lock().unwrap().hot_bytes, 0);
    drop(account);
    assert!(weak.upgrade().is_none());
}
#[test]
fn exact_two_graph_capacity_one_byte_short_and_single_graph_progress_are_checked_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5d03);
    let store = open(dir.path(), &mut rng);
    let pool = &store.owner_evidence.canonical;
    let cap = pool.lock().unwrap().budget.unwrap().hot_bytes;
    pool.lock().unwrap().hot_bytes = cap - 2 * GRAPH;
    let exact = store.reserve_owner_sdk_graphs(true).unwrap();
    assert_eq!(pool.lock().unwrap().hot_bytes, cap);
    drop(exact);
    assert_eq!(pool.lock().unwrap().hot_bytes, cap - 2 * GRAPH);
    pool.lock().unwrap().hot_bytes += 1;
    assert!(store.reserve_owner_sdk_graphs(true).is_err());
    assert_eq!(pool.lock().unwrap().hot_bytes, cap - 2 * GRAPH + 1);
    let one = store.reserve_owner_sdk_graphs(false).unwrap();
    assert_eq!(pool.lock().unwrap().hot_bytes, cap - GRAPH + 1);
    drop(one);
}
#[test]
fn underflow_latches_typed_admission_closed_without_guessing_a_refund() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5d04);
    let store = open(dir.path(), &mut rng);
    let held = store.reserve_owner_sdk_graphs(false).unwrap();
    store.owner_evidence.canonical.lock().unwrap().hot_bytes = 0;
    drop(held);
    let pool = store.owner_evidence.canonical.lock().unwrap();
    assert!(pool.sdk_unavailable);
    assert_eq!(pool.hot_bytes, 0);
    drop(pool);
    assert!(store.reserve_owner_sdk_graphs(false).is_err());
}
