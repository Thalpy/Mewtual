//! Memoized pure record validation, NOT a cached inventory or a continuing write permit.
//! Every hit follows a fresh bounded authenticated read and exact full-wrapper digest match.

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
}
