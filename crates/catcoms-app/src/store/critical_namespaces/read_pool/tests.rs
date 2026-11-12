//! Resource-only account and actual configured opener; no catalog/join authority is fabricated.
//! All tests are authored and UNRUN under the explicit compilation pause.
use super::*;
use crate::store::{critical_namespaces::Slot, ServerStore};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::atomic::Ordering;

const SECRET: &[u8] = b"actual catalog read pool fixture";
fn prepared(path: &std::path::Path, seed: u64) {
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    let store = ServerStore::open(path, SECRET, &mut rng)
        .unwrap()
        .into_critical_namespace_store(SECRET, &mut rng)
        .unwrap();
    drop(store);
}
fn used(pool: &CriticalCatalogReadPool) -> usize {
    pool.inner.state.lock().unwrap().used
}
fn entries(pool: &CriticalCatalogReadPool) -> usize {
    pool.inner.entries.load(Ordering::SeqCst)
}

#[test]
fn shared_pool_exact_capacity_clone_debits_and_strong_original_fee_drop_once() {
    assert!(CriticalCatalogReadPool::new(0).is_err());
    assert!(CriticalCatalogReadPool::new(usize::MAX).is_err());
    let pool = CriticalCatalogReadPool::new(EMPTY_READ_FEE).unwrap();
    let same = pool.clone();
    let fee = pool.try_reserve(EMPTY_READ_FEE).unwrap();
    assert_eq!(used(&same), EMPTY_READ_FEE);
    assert!(same.try_reserve(1).is_err());
    assert!(fee.validate(&same, EMPTY_READ_FEE).is_ok());
    let foreign = CriticalCatalogReadPool::new(EMPTY_READ_FEE).unwrap();
    assert!(fee.validate(&foreign, EMPTY_READ_FEE).is_err());
    let account = Arc::downgrade(&pool.inner);
    drop(pool);
    drop(same);
    assert!(
        account.upgrade().is_some(),
        "the real outstanding fee keeps its ORIGINAL account alive"
    );
    drop(fee);
    assert!(account.upgrade().is_none());
    let short = CriticalCatalogReadPool::new(EMPTY_READ_FEE - 1).unwrap();
    assert!(short.try_reserve(EMPTY_READ_FEE).is_err());
    assert_eq!(used(&short), 0);
}

#[test]
fn pool_poison_underflow_and_overflow_never_reopen_guessed_credits() {
    let pool = CriticalCatalogReadPool::new(EMPTY_READ_FEE).unwrap();
    let fee = pool.try_reserve(EMPTY_READ_FEE).unwrap();
    pool.inner.state.lock().unwrap().used = 0; // deliberate internal-account corruption fixture
    drop(fee);
    assert!(pool.inner.state.lock().unwrap().unavailable);
    assert!(pool.try_reserve(1).is_err());
    let pool = CriticalCatalogReadPool::new(EMPTY_READ_FEE).unwrap();
    pool.inner.state.lock().unwrap().used = usize::MAX;
    assert!(pool.try_reserve(1).is_err());
    assert_eq!(used(&pool), usize::MAX);
    let pool = CriticalCatalogReadPool::new(EMPTY_READ_FEE).unwrap();
    let inner = pool.inner.clone();
    assert!(std::panic::catch_unwind(move || {
        let _guard = inner.state.lock().unwrap();
        panic!("poison actual account in a negative-only fixture");
    })
    .is_err());
    assert!(pool.try_reserve(1).is_err());
}

#[test]
fn configured_open_exhaustion_refuses_before_buffer_entry_and_preserves_original_bytes() {
    let directory = tempfile::tempdir().unwrap();
    prepared(directory.path(), 0x6ca2_0001);
    let pool = CriticalCatalogReadPool::new(EMPTY_READ_FEE).unwrap();
    let fee = pool.try_reserve(EMPTY_READ_FEE).unwrap();
    let catalog = std::fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let wrapper = std::fs::read(directory.path().join("vault.bin")).unwrap();
    let before = entries(&pool);
    let mut rng = ChaCha20Rng::seed_from_u64(0x6ca2_0002);
    assert!(ServerStore::open_with_catalog_read_pool(
        directory.path(),
        SECRET,
        &mut rng,
        pool.clone()
    )
    .is_err());
    assert_eq!(entries(&pool), before);
    assert_eq!(
        std::fs::read(directory.path().join(Slot::Current.name())).unwrap(),
        catalog
    );
    assert_eq!(
        std::fs::read(directory.path().join("vault.bin")).unwrap(),
        wrapper
    );
    drop(fee);
    let store =
        ServerStore::open_with_catalog_read_pool(directory.path(), SECRET, &mut rng, pool.clone())
            .unwrap();
    assert!(entries(&pool) > before);
    assert_eq!(
        used(&pool),
        0,
        "Empty retains no plaintext/file after its actual barriers"
    );
    drop(store);
    let short = CriticalCatalogReadPool::new(EMPTY_READ_FEE - 1).unwrap();
    assert!(ServerStore::open_with_catalog_read_pool(
        directory.path(),
        SECRET,
        &mut rng,
        short.clone()
    )
    .is_err());
    assert_eq!(entries(&short), 0);
}

#[test]
fn configured_open_unknown_stage_and_callback_unwind_release_only_after_sources_drop() {
    let directory = tempfile::tempdir().unwrap();
    prepared(directory.path(), 0x6ca2_0003);
    let pool = CriticalCatalogReadPool::new(EMPTY_READ_FEE).unwrap();
    let stage = directory.path().join(Slot::Stage.name());
    std::fs::write(&stage, b"unsupported retained source").unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x6ca2_0004);
    assert!(ServerStore::open_with_catalog_read_pool(
        directory.path(),
        SECRET,
        &mut rng,
        pool.clone()
    )
    .is_err());
    assert_eq!(used(&pool), 0);
    assert_eq!(
        std::fs::read(&stage).unwrap(),
        b"unsupported retained source"
    );
    std::fs::remove_file(stage).unwrap(); // explicit test fixture reset, no production cleaner
    *pool.inner.before_read.lock().unwrap() = Some(Box::new(|| {
        panic!("unwind after reservation BEFORE raw allocation")
    }));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ServerStore::open_with_catalog_read_pool(directory.path(), SECRET, &mut rng, pool.clone())
    }));
    assert!(result.is_err());
    assert_eq!(used(&pool), 0);
    assert!(ServerStore::open_with_catalog_read_pool(
        directory.path(),
        SECRET,
        &mut rng,
        pool.clone()
    )
    .is_ok());
}

// This is harness scheduling only: ten-second bounded channel waits do not replace a protocol
// Clock/RNG. Drop releases a blocked worker even when a main-thread assertion unwinds.
struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[test]
fn two_physical_vault_readers_using_configured_open_clones_share_the_original_live_reservation() {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    prepared(first.path(), 0x6ca2_0005);
    prepared(second.path(), 0x6ca2_0006);
    let pool = CriticalCatalogReadPool::new(EMPTY_READ_FEE).unwrap();
    // Authenticate this independent physical vault BEFORE holding the worker. The critical
    // overlap below measures resource exclusion, not Argon2 scheduling/performance under load.
    let second_store = ServerStore::open_with_catalog_read_pool(
        second.path(),
        SECRET,
        &mut ChaCha20Rng::seed_from_u64(0x6ca2_0008),
        pool.clone(),
    )
    .unwrap();
    let (reached_tx, reached_rx) = channel();
    let (release_tx, release_rx) = channel();
    let mut release = ReleaseOnDrop(Some(release_tx));
    *pool.inner.before_read.lock().unwrap() = Some(Box::new(move || {
        reached_tx.send(()).unwrap();
        release_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("bounded harness release");
    }));
    let worker_path = first.path().to_path_buf();
    let worker_pool = pool.clone();
    let (done_tx, done_rx) = channel();
    let worker = std::thread::spawn(move || {
        let result = ServerStore::open_with_catalog_read_pool(
            worker_path,
            SECRET,
            &mut ChaCha20Rng::seed_from_u64(0x6ca2_0007),
            worker_pool,
        );
        let success = result.is_ok();
        drop(result); // actual Store/files and fee drain BEFORE completion observation
        let _ = done_tx.send(success);
    });
    reached_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("actual reader reached paid before-buffer boundary");
    assert_eq!(used(&pool), EMPTY_READ_FEE);
    let before = entries(&pool);
    assert!(second_store.inspect_empty_critical_catalog().is_err());
    assert_eq!(entries(&pool), before);
    release.0.take().unwrap().send(()).unwrap();
    assert!(done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("bounded worker completion"));
    worker.join().unwrap(); // completion already observed; no implicit blocking scope/unwind join
    assert_eq!(used(&pool), 0);
    let actual_source = second_store
        .inspect_empty_critical_catalog()
        .unwrap()
        .unwrap();
    assert_eq!(
        used(&pool),
        EMPTY_READ_FEE,
        "actual source holds its SAME original debit through barriers/files"
    );
    drop(actual_source);
    assert_eq!(used(&pool), 0);
}
