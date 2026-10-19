use super::*;
use crate::{BlobStore, FsBlobStore, MemoryBlobStore, SealingBlobStore};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn owned_read_all_stores_authenticate_and_fence_delete_replacement_and_drop() {
    for kind in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let mut store: Box<dyn BlobStore> = match kind {
            0 => Box::new(MemoryBlobStore::new()),
            1 => Box::new(FsBlobStore::open(dir.path()).unwrap()),
            _ => Box::new(
                SealingBlobStore::open(dir.path(), [7; 32], ChaCha20Rng::seed_from_u64(1)).unwrap(),
            ),
        };
        let cid = store.put(b"held content").unwrap();
        let result = store
            .prepare_read_bounded(&cid, 12)
            .unwrap()
            .run()
            .unwrap()
            .unwrap();
        assert_eq!(result.bytes(), b"held content");
        assert!(result.is_current());
        let waiting = store.prepare_read_bounded(&cid, 12).unwrap();
        store.delete(&cid).unwrap();
        store.put(b"held content").unwrap();
        assert!(!result.is_current());
        assert!(waiting.run().unwrap().is_none());
        let fresh = store
            .prepare_read_bounded(&cid, 12)
            .unwrap()
            .run()
            .unwrap()
            .unwrap();
        assert!(fresh.is_current());
        let waiting = store.prepare_read_bounded(&cid, 12).unwrap();
        drop(store);
        assert!(!fresh.is_current());
        assert!(waiting.run().unwrap().is_none());
    }
}
#[test]
fn staged_bytes_are_never_a_source_and_bounds_and_corruption_are_checked() {
    for kind in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let mut store: Box<dyn BlobStore> = match kind {
            0 => Box::new(MemoryBlobStore::new()),
            1 => Box::new(FsBlobStore::open(dir.path()).unwrap()),
            _ => Box::new(
                SealingBlobStore::open(dir.path(), [7; 32], ChaCha20Rng::seed_from_u64(1)).unwrap(),
            ),
        };
        let cid = store.put_staged(b"held content").unwrap();
        assert!(store
            .prepare_read_bounded(&cid, 12)
            .unwrap()
            .run()
            .unwrap()
            .is_none());
        store.promote_staged(&cid).unwrap();
        let prepared = store.prepare_read_bounded(&cid, 11);
        assert!(prepared.is_err() || prepared.unwrap().run().is_err());
        assert!(store.prepare_read_bounded(&cid, MAX_BYTES + 1).is_err());
        if kind != 0 {
            std::fs::write(dir.path().join(cid.to_hex()), b"wrong").unwrap();
            assert!(store.prepare_read_bounded(&cid, 12).unwrap().run().is_err());
            std::fs::File::create(dir.path().join(cid.to_hex()))
                .unwrap()
                .set_len(128 * 1024 * 1024)
                .unwrap();
            assert!(matches!(
                store.prepare_read_bounded(&cid, 12).unwrap().run(),
                Err(StorageError::BlobSizeLimit)
            ));
        }
    }
}
#[test]
fn memory_eviction_retires_captured_snapshots_and_metadata_composition_is_closed() {
    let mut store = MemoryBlobStore::with_budget(12);
    let cid = store.put(b"held content").unwrap();
    let result = store
        .prepare_read_bounded(&cid, 12)
        .unwrap()
        .run()
        .unwrap()
        .unwrap();
    let old = store.prepare_read_bounded(&cid, 12).unwrap();
    store.put(b"a replacement").unwrap();
    assert!(!result.is_current());
    assert!(old.run().unwrap().is_none());
    let epoch = ReadEpoch::default();
    let guard = epoch.guard();
    let mut plan = BlobReadPlan::empty(cid, 12).unwrap();
    for _ in 0..MAX_SOURCES {
        plan = plan
            .then(BlobReadPlan::plain(cid, 12, PathBuf::from("not opened"), guard.clone()).unwrap())
            .unwrap();
    }
    assert!(plan
        .then(BlobReadPlan::plain(cid, 12, PathBuf::from("also not opened"), guard).unwrap())
        .is_err());
    let a = BlobReadPlan::memory(cid, 12, Some(b"held content"), epoch.guard()).unwrap();
    let b = BlobReadPlan::memory(cid, 12, Some(b"held content"), epoch.guard()).unwrap();
    assert!(
        a.then(b).is_err(),
        "captured snapshots cannot multiply the fixed resident bound"
    );
    let mut plan =
        BlobReadPlan::plain(cid, 12, PathBuf::from("not opened"), epoch.guard()).unwrap();
    for _ in 0..7 {
        plan = plan.with_parent(&epoch.guard()).unwrap();
    }
    assert!(plan.with_parent(&epoch.guard()).is_err());
    assert!(BlobReadPlan::plain(
        cid,
        12,
        PathBuf::from("x".repeat(MAX_PATH_BYTES + 1)),
        epoch.guard()
    )
    .is_err());
}
#[test]
fn sealed_maximum_read_has_a_derived_full_buffer_bound_and_current_result() {
    let dir = tempfile::tempdir().unwrap();
    let mut store =
        SealingBlobStore::open(dir.path(), [7; 32], ChaCha20Rng::seed_from_u64(1)).unwrap();
    let max = crate::CHUNK_PAD_CEILING + 44;
    let bytes = vec![3; max];
    let cid = store.put(&bytes).unwrap();
    let bound = BlobReadPlan::memory_bound(max).unwrap();
    assert!(bound > 32 * 1024 * 1024 && bound < 40 * 1024 * 1024);
    let result = store
        .prepare_read_bounded(&cid, max)
        .unwrap()
        .run()
        .unwrap()
        .unwrap();
    assert_eq!(result.bytes(), bytes);
    assert!(result.is_current());
}

#[test]
fn compact_paths_discard_spare_capacity_and_capture_closed_faults_only() {
    let epoch = ReadEpoch::default();
    let cid = Cid::of(b"data");
    let mut path = PathBuf::with_capacity(4 * 1024 * 1024);
    path.push("short");
    assert!(path.capacity() > MAX_PATH_BYTES);
    let plan = BlobReadPlan::plain(cid, 4, path, epoch.guard()).unwrap();
    let Source::Plain(path) = &plan.sources[0].source else {
        panic!("expected plain source");
    };
    assert!(path.capacity() <= MAX_PATH_BYTES);
    assert_eq!(path, Path::new("short"));
    let mut message = String::with_capacity(4 * 1024 * 1024);
    message.push_str("small error");
    let plan = BlobReadPlan::failed(cid, 4, StorageError::Io(message), epoch.guard()).unwrap();
    assert!(matches!(
        plan.sources[0].source,
        Source::Fault(ReadFault::Unavailable)
    ));
    assert_eq!(
        std::mem::size_of::<ReadFault>(),
        1,
        "captured faults have no heap-owned fields"
    );
    let fallback = BlobReadPlan::memory(cid, 4, Some(b"data"), epoch.guard()).unwrap();
    assert_eq!(
        plan.then(fallback).unwrap().run().unwrap().unwrap().bytes(),
        b"data"
    );
}

#[test]
fn oversized_primary_preparation_error_still_allows_a_healthy_kept_fallback() {
    // This deliberately broken adapter demonstrates why error capacity cannot become part of
    // the retained read plan, even though errors from built-in stores are normally very small.
    struct FaultyPrimary;
    impl BlobStore for FaultyPrimary {
        fn prepare_read_bounded(&self, _: &Cid, _: usize) -> Result<BlobReadPlan, StorageError> {
            let mut error = String::with_capacity(4 * 1024 * 1024);
            error.push_str("unavailable");
            Err(StorageError::Io(error))
        }
        fn put(&mut self, _: &[u8]) -> Result<Cid, StorageError> {
            Err(StorageError::Malformed)
        }
        fn get(&self, _: &Cid) -> Result<Option<Vec<u8>>, StorageError> {
            Ok(None)
        }
        fn get_bounded(&self, _: &Cid, _: usize) -> Result<Option<Vec<u8>>, StorageError> {
            Ok(None)
        }
        fn has(&self, _: &Cid) -> bool {
            false
        }
        fn delete(&mut self, _: &Cid) -> Result<bool, StorageError> {
            Ok(false)
        }
        fn cids(&self) -> Vec<Cid> {
            Vec::new()
        }
        fn put_staged(&mut self, _: &[u8]) -> Result<Cid, StorageError> {
            Err(StorageError::Malformed)
        }
        fn promote_staged(&mut self, _: &Cid) -> Result<bool, StorageError> {
            Ok(false)
        }
        fn promote_staged_bounded(&mut self, _: &Cid, _: usize) -> Result<bool, StorageError> {
            Ok(false)
        }
        fn drop_staged(&mut self, _: &Cid) -> Result<bool, StorageError> {
            Ok(false)
        }
        fn clear_staging(&mut self) -> Result<usize, StorageError> {
            Ok(0)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut store = crate::kept::KeptBlobStore::open(
        Box::new(FaultyPrimary),
        dir.path().to_path_buf(),
        [7; 32],
        [8; 32],
        ChaCha20Rng::seed_from_u64(1),
    );
    store
        .configure_keep_scope(crate::kept::KeepScope {
            group: vec![1; 32],
            device: catcoms_crypto::DeviceId::from_bytes([2; 32]),
        })
        .unwrap();
    let cid = Cid::of(b"data");
    let run = store
        .begin_keep(crate::kept::KeepPlan {
            cid,
            version: [1; 32],
            manifest: vec![1],
            chunks: vec![(cid, 4)],
        })
        .unwrap();
    store.put_keep(run, b"data").unwrap();
    store.finish_keep(run).unwrap();
    let plan = store.prepare_read_bounded(&cid, 4).unwrap();
    assert!(matches!(
        plan.sources[0].source,
        Source::Fault(ReadFault::Unavailable)
    ));
    assert_eq!(plan.sources.len(), 2);
    let result = plan.run().unwrap().unwrap();
    assert!(result.is_current());
    assert_eq!(result.bytes(), b"data");
}
