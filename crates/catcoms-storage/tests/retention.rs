//! Retention/GC tests with injected time and a stub holder oracle: expiry
//! precedence, decorrelated eviction, the holder probe (no last-copy eviction),
//! metadata-kept-on-eviction (refetchable), cache clearing, rehydration, and the
//! typed MissingNoHolder state.

use std::collections::HashMap;

use catcoms_storage::{
    BlobKind, BlobState, BlobStore, Cid, Expiry, ExpiryPolicy, HolderOracle, MemoryBlobStore,
    RetentionIndex, ServerId, ONE_MONTH_MS,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn rng(seed: u64) -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(seed)
}

fn server() -> ServerId {
    ServerId(b"server-a".to_vec())
}

/// Holder oracle reporting a fixed count for every blob.
struct FixedHolders(usize);
impl HolderOracle for FixedHolders {
    fn reachable_holder_count(&self, _cid: &Cid) -> usize {
        self.0
    }
}

/// Holder oracle with per-CID counts.
struct MapHolders(HashMap<Cid, usize>);
impl HolderOracle for MapHolders {
    fn reachable_holder_count(&self, cid: &Cid) -> usize {
        *self.0.get(cid).unwrap_or(&0)
    }
}

/// Store a blob and index it as a `File` created at `now`.
fn store_file(
    store: &mut MemoryBlobStore,
    index: &mut RetentionIndex,
    body: &[u8],
    now: u64,
    r: &mut ChaCha20Rng,
) -> Cid {
    let cid = store.put(body).unwrap();
    index.insert(cid, server(), body.len() as u64, BlobKind::File, now, r);
    cid
}

#[test]
fn expiry_precedence_file_over_server_over_global() {
    let mut policy = ExpiryPolicy::with_global(1000);
    policy.set_server(server(), 500);
    let cid = Cid::of(b"x");
    policy.set_file(cid, Expiry::Never);

    // File override wins.
    assert_eq!(policy.effective(&cid, &server()), Expiry::Never);
    // Server override wins over global for an un-overridden file.
    assert_eq!(
        policy.effective(&Cid::of(b"y"), &server()),
        Expiry::After(500)
    );
    // Global default for an unknown server.
    assert_eq!(
        policy.effective(&Cid::of(b"z"), &ServerId(b"other".to_vec())),
        Expiry::After(1000)
    );
}

#[test]
fn gc_evicts_bytes_but_keeps_metadata_when_replicated() {
    let mut store = MemoryBlobStore::new();
    let mut index = RetentionIndex::new(ExpiryPolicy::with_global(1000));
    let mut r = rng(1);
    let cid = store_file(&mut store, &mut index, b"old file", 0, &mut r);

    // Past the (jittered) deadline; >=1 other holder confirmed.
    let report = index.gc(&mut store, &FixedHolders(3), 10_000, 1).unwrap();
    assert_eq!(report.evicted, 1);
    assert!(!store.has(&cid)); // bytes gone locally
    assert!(index.get(&cid).unwrap().evicted); // ...but metadata kept
    assert_eq!(
        index.blob_state(&store, &FixedHolders(3), &cid),
        BlobState::EvictedRefetchable
    );
}

#[test]
fn gc_will_not_evict_the_last_copy() {
    let mut store = MemoryBlobStore::new();
    let mut index = RetentionIndex::new(ExpiryPolicy::with_global(1000));
    let mut r = rng(1);
    let cid = store_file(&mut store, &mut index, b"only copy", 0, &mut r);

    // Expired, but no other holder -> must NOT evict.
    let report = index.gc(&mut store, &FixedHolders(0), 10_000, 1).unwrap();
    assert_eq!(report.evicted, 0);
    assert_eq!(report.kept_no_replica, 1);
    assert!(store.has(&cid));
    assert!(!index.get(&cid).unwrap().evicted);
}

#[test]
fn inline_media_and_pinned_blobs_are_never_age_evicted() {
    let mut store = MemoryBlobStore::new();
    let mut index = RetentionIndex::new(ExpiryPolicy::with_global(1000));
    let mut r = rng(1);

    let media = store.put(b"an image").unwrap();
    index.insert(media, server(), 8, BlobKind::InlineMedia, 0, &mut r);
    let pinned = store.put(b"archive").unwrap();
    index.insert(pinned, server(), 7, BlobKind::File, 0, &mut r);
    index.set_pinned(&pinned, true);

    assert_eq!(index.deadline(&media), None);
    assert_eq!(index.deadline(&pinned), None);

    let report = index
        .gc(&mut store, &FixedHolders(5), ONE_MONTH_MS * 12, 1)
        .unwrap();
    assert_eq!(report.evicted, 0);
    assert!(store.has(&media));
    assert!(store.has(&pinned));
}

#[test]
fn eviction_deadlines_are_decorrelated_by_jitter() {
    // Many blobs, same creation time and policy -> deadlines must spread out.
    let mut store = MemoryBlobStore::new();
    let mut index = RetentionIndex::new(ExpiryPolicy::with_global(4000));
    let mut r = rng(7);

    let mut deadlines = Vec::new();
    for i in 0..50u32 {
        let cid = store_file(&mut store, &mut index, &i.to_be_bytes(), 0, &mut r);
        deadlines.push(index.deadline(&cid).unwrap());
    }
    // All within [global, global + maxJitter] = [4000, 5000].
    assert!(deadlines.iter().all(|&d| (4000..=5000).contains(&d)));
    // And genuinely spread, not all identical.
    let distinct: std::collections::HashSet<_> = deadlines.iter().collect();
    assert!(distinct.len() > 5, "deadlines should be decorrelated");
}

#[test]
fn clear_cache_older_than_evicts_old_files() {
    let mut store = MemoryBlobStore::new();
    let mut index = RetentionIndex::new(ExpiryPolicy::with_global(ONE_MONTH_MS));
    let mut r = rng(1);
    let old = store_file(&mut store, &mut index, b"old", 0, &mut r);
    let recent = store_file(&mut store, &mut index, b"recent", 9_500, &mut r);

    // "Clear files older than 1000ms" at now=10000.
    let report = index
        .clear_older_than(&mut store, &FixedHolders(2), 1_000, 10_000, 1, false)
        .unwrap();
    assert_eq!(report.evicted, 1);
    assert!(!store.has(&old));
    assert!(store.has(&recent)); // only 500ms old -> kept
}

#[test]
fn evicted_blob_can_be_rehydrated_on_demand() {
    let mut store = MemoryBlobStore::new();
    let mut index = RetentionIndex::new(ExpiryPolicy::with_global(1000));
    let mut r = rng(1);
    let cid = store_file(&mut store, &mut index, b"refetch me", 0, &mut r);

    index.gc(&mut store, &FixedHolders(2), 10_000, 1).unwrap();
    assert!(!store.has(&cid));

    // Re-fetch supplies the bytes; rehydrate verifies the CID and restores them.
    index.rehydrate(&mut store, &cid, b"refetch me").unwrap();
    assert!(store.has(&cid));
    assert!(!index.get(&cid).unwrap().evicted);
    assert_eq!(
        index.blob_state(&store, &FixedHolders(0), &cid),
        BlobState::Available
    );

    // Rehydrating with the wrong bytes is rejected.
    assert!(index.rehydrate(&mut store, &cid, b"wrong bytes").is_err());
}

#[test]
fn fully_abandoned_blob_surfaces_as_missing_no_holder() {
    let mut store = MemoryBlobStore::new();
    let mut index = RetentionIndex::new(ExpiryPolicy::with_global(1000));
    let mut r = rng(1);
    let cid = store_file(&mut store, &mut index, b"doomed", 0, &mut r);

    // Evict while replicated...
    index.gc(&mut store, &FixedHolders(2), 10_000, 1).unwrap();
    // ...then every other holder also drops it.
    let holders = MapHolders(HashMap::new());
    assert_eq!(
        index.blob_state(&store, &holders, &cid),
        BlobState::MissingNoHolder
    );
}

#[test]
fn unknown_blob_reports_unknown_state() {
    let store = MemoryBlobStore::new();
    let index = RetentionIndex::new(ExpiryPolicy::default());
    assert_eq!(
        index.blob_state(&store, &FixedHolders(0), &Cid::of(b"never seen")),
        BlobState::Unknown
    );
}
