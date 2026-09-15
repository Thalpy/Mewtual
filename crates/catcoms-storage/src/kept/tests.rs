use super::*;
use crate::MemoryBlobStore;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn open(path: &Path) -> KeptBlobStore<ChaCha20Rng> {
    KeptBlobStore::open(
        Box::new(MemoryBlobStore::new()),
        path.to_path_buf(),
        [7; 32],
        ChaCha20Rng::seed_from_u64(6),
    )
}
fn plan(bytes: &[u8]) -> KeepPlan {
    KeepPlan {
        cid: Cid::of(bytes),
        version: [1; 32],
        manifest: b"opaque wrapped manifest".to_vec(),
        chunks: vec![(Cid::of(bytes), bytes.len() as u64)],
    }
}

#[test]
fn kept_copy_is_durable_separate_and_requires_recheck_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = b"authenticated ciphertext";
    let plan = plan(bytes);
    let cid = plan.cid;
    let mut store = open(dir.path());
    let token = store.begin_keep(plan.clone()).unwrap();
    let charge = store.kept_files().allocated_bytes;
    assert!(charge > bytes.len() as u64 + 40);
    assert!(
        store.finish_keep(token).is_err(),
        "incomplete copies cannot be committed"
    );
    store.put_keep(token, bytes).unwrap();
    assert_eq!(
        store.get(&cid).unwrap(),
        None,
        "in-progress copies cannot be served"
    );
    assert!(
        store.primary.cids().is_empty(),
        "retention never writes the ordinary cache"
    );
    store.finish_keep(token).unwrap();
    assert_eq!(store.kept_files().allocated_bytes, charge);
    assert!(store.kept_files().files[0].checked);
    assert_eq!(store.get(&cid).unwrap().unwrap(), bytes);
    store.delete(&cid).unwrap();
    store.clear_staging().unwrap();
    assert!(
        store.has(&cid),
        "ordinary GC cannot release local ownership"
    );
    drop(store);
    let mut store = open(dir.path());
    assert!(store.kept_files().error.is_none());
    assert!(!store.kept_files().files[0].checked);
    assert_eq!(store.kept_files().allocated_bytes, charge);
    let token = store.begin_keep(plan).unwrap();
    store.put_keep(token, bytes).unwrap();
    store.finish_keep(token).unwrap();
    assert!(store.kept_files().files[0].checked);
    store.forget_kept(&cid).unwrap();
    assert!(!store.has(&cid));
    assert_eq!(store.kept_files().allocated_bytes, 0);
}

#[test]
fn incomplete_and_failed_keeps_remain_bounded_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let token = store.begin_keep(plan(b"partial")).unwrap();
    assert!(store.put_keep(token, b"unreserved bytes").is_err());
    store.put_keep(token, b"partial").unwrap();
    assert!(store.begin_keep(plan(b"another")).is_err());
    drop(store);
    let mut store = open(dir.path());
    assert!(store.kept_files().files.is_empty());
    assert_eq!(store.kept_files().allocated_bytes, 0);
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        1,
        "only the mount lease remains"
    );
    let token = store.begin_keep(plan(b"x")).unwrap();
    // A surprising nested directory blocks cleanup; it must not trigger recursive deletion or
    // recycle the still-live reservation. Startup fails closed on it as well.
    fs::create_dir(dir.path().join(PENDING).join("unexpected")).unwrap();
    assert!(store.abort_keep(token).is_err());
    assert!(store.kept_files().allocated_bytes > 0);
    assert!(store.begin_keep(plan(b"y")).is_err());
    drop(store);
    let mut store = open(dir.path());
    assert!(store.kept_files().error.is_some());
    assert!(store.begin_keep(plan(b"z")).is_err());
    store.put(b"ordinary uploads still work").unwrap();
}

#[test]
fn quota_and_exact_plan_tokens_cannot_be_bypassed() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let mut oversized = plan(b"x");
    oversized.chunks = (0..CHUNK_LIMIT)
        .map(|i| (Cid::of(&i.to_le_bytes()), CHUNK_LIMIT_BYTES))
        .collect();
    assert!(
        store.begin_keep(oversized).is_err(),
        "ciphertext + disk overhead exceed 1 GiB"
    );
    let mut repeated = plan(b"x");
    repeated.chunks.push(repeated.chunks[0]);
    assert!(store.begin_keep(repeated).is_err());
    let old = store.begin_keep(plan(b"a")).unwrap();
    store.abort_keep(old).unwrap();
    let new = store.begin_keep(plan(b"b")).unwrap();
    assert_ne!(old, new);
    assert!(store.put_keep(old, b"a").is_err());
    assert!(store.abort_keep(old).is_err());
    assert!(store.put_keep(new, b"a").is_err());
    store.put_keep(new, b"b").unwrap();
    store.finish_keep(new).unwrap();
    let mut replaced = plan(b"b");
    replaced.version = [2; 32];
    assert!(
        store.begin_keep(replaced).is_err(),
        "replacement requires explicit local release"
    );
}

#[test]
fn primary_corruption_falls_back_and_recheck_repairs_retained_records() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = b"ciphertext";
    let p = plan(bytes);
    let mut store = open(dir.path());
    let token = store.begin_keep(p.clone()).unwrap();
    store.put_keep(token, bytes).unwrap();
    store.finish_keep(token).unwrap();
    let record = dir.path().join(p.cid.to_hex()).join(p.cid.to_hex());
    fs::write(&record, b"corrupt").unwrap();
    store.primary.put(bytes).unwrap();
    let token = store.begin_keep(p.clone()).unwrap();
    assert!(!store.kept_files().files[0].checked);
    store.put_keep(token, bytes).unwrap();
    store.finish_keep(token).unwrap();
    store.primary.delete(&p.cid).unwrap();
    assert_eq!(
        store.get(&p.cid).unwrap().unwrap(),
        bytes,
        "recheck repaired the kept record itself"
    );
}

#[test]
fn metadata_and_chunks_draw_distinct_nonces_from_one_injected_rng() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let mut nonces = HashSet::new();
    for bytes in [b"one", b"two"] {
        let p = plan(bytes);
        let token = store.begin_keep(p.clone()).unwrap();
        store.put_keep(token, bytes).unwrap();
        store.finish_keep(token).unwrap();
        for path in entries(&dir.path().join(p.cid.to_hex()), 2).unwrap() {
            assert!(nonces.insert(fs::read(path).unwrap()[..24].to_vec()));
        }
    }
}

#[test]
fn overlapping_mounts_cannot_sweep_or_fork_retained_accounting() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = open(dir.path());
    let token = first.begin_keep(plan(b"owned")).unwrap();
    first.put_keep(token, b"owned").unwrap();
    let mut second = open(dir.path());
    assert!(second.kept_files().error.is_some());
    assert!(second.begin_keep(plan(b"another")).is_err());
    assert!(dir.path().join(PENDING).join(META).exists());
    first.finish_keep(token).unwrap();
    assert!(second.forget_kept(&Cid::of(b"owned")).is_err());
    assert!(first.kept_files().files[0].checked);
    drop(first);
    let reopened = open(dir.path());
    assert!(reopened.kept_files().error.is_none());
    assert_eq!(reopened.kept_files().files.len(), 1);
}

#[test]
fn interrupted_release_is_recoverable_even_after_manifest_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let p = plan(b"owned");
    let token = store.begin_keep(p.clone()).unwrap();
    store.put_keep(token, b"owned").unwrap();
    store.finish_keep(token).unwrap();
    // Crash after the release-intent rename and deletion of its manifest, before other leaves.
    fs::rename(dir.path().join(p.cid.to_hex()), dir.path().join(PENDING)).unwrap();
    fs::remove_file(dir.path().join(PENDING).join(META)).unwrap();
    drop(store);
    let store = open(dir.path());
    assert!(store.kept_files().error.is_none());
    assert!(store.kept_files().files.is_empty());
    assert!(!dir.path().join(PENDING).exists());
}

#[test]
fn failed_directory_flush_never_certifies_a_repaired_copy() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let p = plan(b"owned");
    let token = store.begin_keep(p.clone()).unwrap();
    store.put_keep(token, b"owned").unwrap();
    store.finish_keep(token).unwrap();
    let charge = store.kept_files().allocated_bytes;
    std::fs::remove_file(dir.path().join(p.cid.to_hex()).join(p.cid.to_hex())).unwrap();
    let token = store.begin_keep(p.clone()).unwrap();
    store.put_keep(token, b"owned").unwrap();
    let result = store.finish_keep_with_sync(token, |path| {
        assert_eq!(path, dir.path().join(p.cid.to_hex()));
        Err(StorageError::CommittedButNotDurable(
            "injected directory sync failure".into(),
        ))
    });
    assert!(result.is_err());
    assert!(!store.kept_files().files[0].checked);
    assert_eq!(store.kept_files().allocated_bytes, charge);
    store.abort_keep(token).unwrap();
    drop(store);
    let store = open(dir.path());
    assert!(!store.kept_files().files[0].checked);
    assert_eq!(store.get(&p.cid).unwrap().unwrap(), b"owned");
}

#[test]
fn corrupt_primary_record_falls_back_to_the_completed_kept_copy() {
    let root = tempfile::tempdir().unwrap();
    let primary_dir = root.path().join("primary");
    let primary =
        SealingBlobStore::open(&primary_dir, [5; 32], ChaCha20Rng::seed_from_u64(4)).unwrap();
    let mut store = KeptBlobStore::open(
        Box::new(primary),
        root.path().join("kept"),
        [6; 32],
        ChaCha20Rng::seed_from_u64(5),
    );
    let p = plan(b"owned");
    store.put(b"owned").unwrap();
    let token = store.begin_keep(p.clone()).unwrap();
    store.put_keep(token, b"owned").unwrap();
    store.finish_keep(token).unwrap();
    fs::write(primary_dir.join(p.cid.to_hex()), b"corrupt primary seal").unwrap();
    assert!(store.primary.get(&p.cid).is_err());
    assert_eq!(store.get(&p.cid).unwrap().unwrap(), b"owned");
}
