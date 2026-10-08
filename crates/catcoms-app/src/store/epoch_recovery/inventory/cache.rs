//! Memoized pure record validation, NOT a cached inventory or a continuing write permit.
//! Every hit follows a fresh bounded authenticated read and exact full-wrapper digest match.
//!
//! **What keeps a hit correct is that every put's record is the true pure validation of the bytes
//! its digest names.** A hit needs the exact size and digest of what a scan has just read, so such
//! an entry can only ever be returned for those bytes. The three warm sites (an installed scan
//! record, a refused detached result, and the Studio and Registry write paths) all put a record
//! validated from, or established for, exactly those bytes.
//!
//! **Each put should also describe bytes that are, or were, on disk for that record**, never bytes
//! a writer is about to write. That is about usefulness, not correctness: an entry for bytes not
//! on disk is evicted by the next read ([`RecordCache::evict_mismatch`]) or simply never hits.

use super::{EpochRecordKind, StorageRecord};
use std::collections::VecDeque;

type Key = (EpochRecordKind, [u8; 32]);
const MAX_RECORDS: usize = 64;

struct Verified {
    key: Key,
    digest: blake3::Hash,
    physical_bytes: u64,
    record: StorageRecord,
}

/// Mount-local metadata only: no plaintext, mutable CRDT, authority key or scan budget survives.
/// One version per physical record prevents retained old versions from multiplying the cache.
#[derive(Default)]
pub(in crate::store) struct RecordCache(VecDeque<Verified>);
impl RecordCache {
    /// A candidate permits bounded authentication work, NEVER reuse before matching its digest.
    pub(super) fn candidate(&self, key: Key, size: u64) -> bool {
        self.0
            .iter()
            .any(|v| v.key == key && v.physical_bytes == size)
    }
    pub(super) fn get(
        &mut self,
        key: Key,
        size: u64,
        digest: blake3::Hash,
    ) -> Option<StorageRecord> {
        let i = self
            .0
            .iter()
            .position(|v| v.key == key && v.physical_bytes == size && v.digest == digest)?;
        let value = self.0.remove(i).expect("cache index");
        let record = value.record;
        self.0.push_back(value);
        Some(record)
    }
    pub(in crate::store) fn put(
        &mut self,
        key: Key,
        size: u64,
        digest: blake3::Hash,
        record: StorageRecord,
    ) {
        self.0.retain(|v| v.key != key);
        if self.0.len() == MAX_RECORDS {
            self.0.pop_front();
        }
        self.0.push_back(Verified {
            key,
            digest,
            physical_bytes: size,
            record,
        });
    }

    /// [`Self::put`], unless any version of `key` is already cached. Returns whether it stored.
    ///
    /// For a result whose job was overtaken (C-3 runtime design 14.3, piece 3). The read evicted
    /// any version its bytes contradicted ([`Self::evict_mismatch`]), so an entry present now is
    /// either the same bytes or was put since the read, perhaps by a write path for newer bytes.
    /// A stale result must never displace the latter, and adds nothing over the former.
    ///
    /// This is not a guarantee that nothing newer exists. LRU can push a newer entry out between
    /// the read and the refusal, and then the stale result is stored. That costs a miss, never a
    /// wrong record, because a hit needs the digest of the bytes a later scan reads.
    pub(super) fn put_if_vacant(
        &mut self,
        key: Key,
        size: u64,
        digest: blake3::Hash,
        record: StorageRecord,
    ) -> bool {
        if self.0.iter().any(|v| v.key == key) {
            return false;
        }
        self.put(key, size, digest, record);
        true
    }

    /// How many entries belong to `family`. Test-only, for checking what the memo admits.
    #[cfg(test)]
    pub(in crate::store) fn entries_for_test(&self, family: EpochRecordKind) -> usize {
        self.0.iter().filter(|v| v.key.0 == family).count()
    }

    /// Drop the entry for `key` if it describes bytes other than the ones a scan just read.
    ///
    /// The cache keeps one version per record, and the file no longer holds that version, so the
    /// entry can never hit again. Leaving it in place would also make every later
    /// [`Self::put_if_vacant`] for this record refuse, so a writer that does not warm the cache
    /// (the core Studio writer, or a warm its own checks declined) would leave the record cold
    /// for good (C-3 runtime design 14.3, piece 1).
    pub(super) fn evict_mismatch(&mut self, key: Key, size: u64, digest: blake3::Hash) {
        self.0
            .retain(|v| v.key != key || (v.physical_bytes == size && v.digest == digest));
    }

    /// Drop every memoized validation.
    ///
    /// Only design 13.7's measurement needs this. Registry and Studio are the two cacheable
    /// families, so a repeated-trial profile of them measures one fresh validation and then
    /// seven cache hits unless the cache is cleared between trials - and a cache hit is never
    /// parked, so those seven trials would contribute no validation sample at all while the
    /// mean silently divided by the wrong count. Clearing makes "fresh validation" repeatable;
    /// *not* clearing is the separate cache-hit measurement.
    #[cfg(test)]
    pub(in crate::store) fn clear_for_test(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::epoch_budget::Footprint;
    #[test]
    fn inventory_cache_is_bounded_lru_and_requires_family_size_and_full_digest() {
        let mut cache = RecordCache::default();
        let record = StorageRecord {
            id: [0; 32],
            document: [1; 32],
            footprint: Footprint::default(),
        };
        let digest = blake3::hash(b"complete authenticated wrapper");
        for n in 0..64 {
            cache.put((EpochRecordKind::Registry, [n; 32]), 77, digest, record);
        }
        assert_eq!(cache.0.len(), 64);
        assert!(cache
            .get((EpochRecordKind::Registry, [0; 32]), 77, digest)
            .is_some());
        cache.put((EpochRecordKind::Registry, [64; 32]), 77, digest, record);
        assert!(!cache.candidate((EpochRecordKind::Registry, [1; 32]), 77));
        assert!(!cache.candidate((EpochRecordKind::Studio, [0; 32]), 77));
        assert!(!cache.candidate((EpochRecordKind::Registry, [0; 32]), 78));
        assert!(cache
            .get(
                (EpochRecordKind::Registry, [0; 32]),
                77,
                blake3::hash(b"gate-only change")
            )
            .is_none());
        cache.put(
            (EpochRecordKind::Registry, [0; 32]),
            77,
            blake3::hash(b"new version"),
            record,
        );
        assert_eq!(cache.0.len(), 64);
        assert!(cache
            .get((EpochRecordKind::Registry, [0; 32]), 77, digest)
            .is_none());
    }

    fn record() -> StorageRecord {
        StorageRecord {
            id: [0; 32],
            document: [1; 32],
            footprint: Footprint::default(),
        }
    }

    /// A vacant put stores; any version already cached, matching or not, makes it refuse.
    #[test]
    fn a_vacant_only_put_never_displaces_an_existing_version() {
        let mut cache = RecordCache::default();
        let key = (EpochRecordKind::Studio, [3; 32]);
        let (old, new) = (blake3::hash(b"v1"), blake3::hash(b"v2"));
        assert!(cache.put_if_vacant(key, 10, old, record()));
        assert!(
            !cache.put_if_vacant(key, 10, new, record()),
            "a vacant-only put replaced the version already cached"
        );
        assert!(cache.get(key, 10, old).is_some());
        assert!(cache.get(key, 10, new).is_none());
        // Another record's key is unaffected by this one's entry.
        assert!(cache.put_if_vacant((EpochRecordKind::Registry, [3; 32]), 10, new, record()));
    }

    /// Eviction removes only a version of the same record that the bytes just read contradict:
    /// a different size or a different digest. The matching version, and every other record,
    /// stay.
    #[test]
    fn eviction_removes_only_a_contradicted_version_of_the_same_record() {
        let mut cache = RecordCache::default();
        let key = (EpochRecordKind::Studio, [4; 32]);
        let other = (EpochRecordKind::Studio, [5; 32]);
        let (v1, v2) = (blake3::hash(b"v1"), blake3::hash(b"v2"));
        cache.put(key, 10, v1, record());
        cache.put(other, 10, v1, record());

        cache.evict_mismatch(key, 10, v1);
        assert!(
            cache.get(key, 10, v1).is_some(),
            "the matching version was evicted"
        );

        cache.evict_mismatch(key, 11, v1);
        assert!(
            cache.get(key, 10, v1).is_none(),
            "a size mismatch did not evict"
        );

        cache.put(key, 10, v1, record());
        cache.evict_mismatch(key, 10, v2);
        assert!(
            cache.get(key, 10, v1).is_none(),
            "a digest mismatch did not evict"
        );

        assert!(
            cache.get(other, 10, v1).is_some(),
            "evicting one record removed another"
        );
        // With the contradicted version gone, a vacant-only put for the bytes just read stores.
        assert!(cache.put_if_vacant(key, 10, v2, record()));
    }
}
