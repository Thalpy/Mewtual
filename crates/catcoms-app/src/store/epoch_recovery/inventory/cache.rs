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
//!
//! **Sized to the inventory's own bound, and pruned to the vault** (C-3 runtime design 15.3, M2).
//! It used to be a 64-entry LRU, so a scan of more memoizable records than that missed on every
//! record: each pass evicted what the next needed. It now holds up to `MAX_ACCOUNTED_RECORDS`
//! entries, the most records any inventory can account, so a complete scan never thrashes it.
//! [`RecordCache::prune`] drops the entries for files a completed scan did not find, so in steady
//! state it tracks the vault rather than its history. Memory is about 300 bytes per entry, about
//! 20 MiB at the bound, resident for the mount's life (15.7, LOW).
//!
//! It is indexed (a keyed map plus an access order), so get, put and eviction are logarithmic:
//! the linear scan the 64-entry form used would make a scan of 65 536 records quadratic.

use super::{EpochRecordKind, StorageRecord};
use crate::store::epoch_budget::MAX_ACCOUNTED_RECORDS;
use std::collections::BTreeMap;

type Key = (EpochRecordKind, [u8; 32]);
const MAX_RECORDS: usize = MAX_ACCOUNTED_RECORDS;

struct Verified {
    digest: blake3::Hash,
    physical_bytes: u64,
    record: StorageRecord,
    /// Position in [`RecordCache::order`]: the entry with the smallest tick is evicted first.
    tick: u64,
}

/// Mount-local metadata only: no plaintext, mutable CRDT, authority key or scan budget survives.
/// One version per physical record prevents retained old versions from multiplying the cache.
///
/// `entries` and `order` always describe the same set of keys: every method that touches one
/// updates the other, and nothing outside this type sees either.
#[derive(Default)]
pub(in crate::store) struct RecordCache {
    entries: BTreeMap<Key, Verified>,
    /// Least recently used first. A tick is never reused, so no two entries share one.
    order: BTreeMap<u64, Key>,
    next_tick: u64,
}
impl RecordCache {
    /// A candidate permits bounded authentication work, NEVER reuse before matching its digest.
    pub(super) fn candidate(&self, key: Key, size: u64) -> bool {
        self.entries
            .get(&key)
            .is_some_and(|v| v.physical_bytes == size)
    }
    pub(super) fn get(
        &mut self,
        key: Key,
        size: u64,
        digest: blake3::Hash,
    ) -> Option<StorageRecord> {
        self.entries
            .get(&key)
            .filter(|v| v.physical_bytes == size && v.digest == digest)?;
        let tick = self.take_tick();
        let value = self.entries.get_mut(&key).expect("matched just above");
        self.order.remove(&value.tick);
        value.tick = tick;
        self.order.insert(tick, key);
        Some(value.record)
    }
    pub(in crate::store) fn put(
        &mut self,
        key: Key,
        size: u64,
        digest: blake3::Hash,
        record: StorageRecord,
    ) {
        self.remove(&key);
        if self.entries.len() >= MAX_RECORDS {
            if let Some((_, oldest)) = self.order.pop_first() {
                self.entries.remove(&oldest);
            }
        }
        let tick = self.take_tick();
        self.order.insert(tick, key);
        self.entries.insert(
            key,
            Verified {
                digest,
                physical_bytes: size,
                record,
                tick,
            },
        );
    }

    /// Drop every entry `present` says is not in the vault, among the families `covered` names.
    ///
    /// For a scan that has just completed (C-3 runtime design 15.3, M2). Such a scan has seen
    /// every record in its coverage, so an entry it did not see names a file that is gone. Only
    /// the scan's own coverage is pruned: a narrower scan saw nothing of the other families, so
    /// it cannot tell their entries are stale (15.7, LOW). Pruning is about memory, never about
    /// correctness: a stale entry could only ever hit for bytes that are on disk.
    pub(super) fn prune(
        &mut self,
        covered: impl Fn(EpochRecordKind) -> bool,
        present: impl Fn(&Key) -> bool,
    ) {
        let entries = &mut self.entries;
        self.order.retain(|_, key| {
            let keep = !covered(key.0) || present(key);
            if !keep {
                entries.remove(key);
            }
            keep
        });
    }

    fn remove(&mut self, key: &Key) {
        if let Some(old) = self.entries.remove(key) {
            self.order.remove(&old.tick);
        }
    }

    fn take_tick(&mut self) -> u64 {
        let tick = self.next_tick;
        self.next_tick += 1;
        tick
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
        if self.entries.contains_key(&key) {
            return false;
        }
        self.put(key, size, digest, record);
        true
    }

    /// How many entries belong to `family`. Test-only, for checking what the memo admits.
    #[cfg(test)]
    pub(in crate::store) fn entries_for_test(&self, family: EpochRecordKind) -> usize {
        self.entries.keys().filter(|key| key.0 == family).count()
    }

    /// Whether any version of `key` is cached. Test-only.
    #[cfg(test)]
    pub(in crate::store) fn holds_for_test(&self, key: Key) -> bool {
        self.entries.contains_key(&key)
    }

    /// Drop the entry for `key` if it describes bytes other than the ones a scan just read.
    ///
    /// The cache keeps one version per record, and the file no longer holds that version, so the
    /// entry can never hit again. Leaving it in place would also make every later
    /// [`Self::put_if_vacant`] for this record refuse, so a writer that does not warm the cache
    /// (the core Studio writer, or a warm its own checks declined) would leave the record cold
    /// for good (C-3 runtime design 14.3, piece 1).
    pub(super) fn evict_mismatch(&mut self, key: Key, size: u64, digest: blake3::Hash) {
        if self
            .entries
            .get(&key)
            .is_some_and(|v| v.physical_bytes != size || v.digest != digest)
        {
            self.remove(&key);
        }
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
        self.entries.clear();
        self.order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::epoch_budget::Footprint;

    /// The `n`th distinct record hash.
    fn hash(n: usize) -> [u8; 32] {
        let mut hash = [0; 32];
        hash[..8].copy_from_slice(&(n as u64).to_le_bytes());
        hash
    }

    /// `entries` and `order` name the same keys, each exactly once.
    fn consistent(cache: &RecordCache) -> bool {
        cache.entries.len() == cache.order.len()
            && cache
                .order
                .iter()
                .all(|(tick, key)| cache.entries.get(key).is_some_and(|v| v.tick == *tick))
    }

    /// The bound is the inventory's own (C-3 runtime design 15.3, M2), and a put at the bound
    /// evicts the least recently used entry. A hit counts as a use. A hit also needs the family,
    /// the size and the full digest, and a put replaces any other version of its record.
    ///
    /// Contract change, recorded: until M2 the bound was 64.
    #[test]
    fn inventory_cache_is_bounded_lru_and_requires_family_size_and_full_digest() {
        let mut cache = RecordCache::default();
        let record = StorageRecord {
            id: [0; 32],
            document: [1; 32],
            footprint: Footprint::default(),
        };
        let digest = blake3::hash(b"complete authenticated wrapper");
        for n in 0..MAX_RECORDS {
            cache.put((EpochRecordKind::Registry, hash(n)), 77, digest, record);
        }
        assert_eq!(cache.entries.len(), MAX_RECORDS);
        assert!(
            consistent(&cache),
            "the index and the access order disagree"
        );
        // Record 0 is used, so record 1 is now the least recently used.
        assert!(cache
            .get((EpochRecordKind::Registry, hash(0)), 77, digest)
            .is_some());
        cache.put(
            (EpochRecordKind::Registry, hash(MAX_RECORDS)),
            77,
            digest,
            record,
        );
        assert_eq!(cache.entries.len(), MAX_RECORDS, "the bound was not held");
        assert!(!cache.candidate((EpochRecordKind::Registry, hash(1)), 77));
        assert!(
            cache.candidate((EpochRecordKind::Registry, hash(0)), 77),
            "a hit did not count as a use"
        );
        assert!(!cache.candidate((EpochRecordKind::Studio, hash(0)), 77));
        assert!(!cache.candidate((EpochRecordKind::Registry, hash(0)), 78));
        assert!(cache
            .get(
                (EpochRecordKind::Registry, hash(0)),
                77,
                blake3::hash(b"gate-only change")
            )
            .is_none());
        cache.put(
            (EpochRecordKind::Registry, hash(0)),
            77,
            blake3::hash(b"new version"),
            record,
        );
        assert_eq!(cache.entries.len(), MAX_RECORDS);
        assert!(cache
            .get((EpochRecordKind::Registry, hash(0)), 77, digest)
            .is_none());
        assert!(consistent(&cache));
    }

    /// Pruning drops exactly the covered entries the scan did not find, and nothing outside its
    /// coverage, and keeps the index consistent.
    #[test]
    fn pruning_drops_only_covered_entries_the_scan_did_not_find() {
        let mut cache = RecordCache::default();
        let digest = blake3::hash(b"v1");
        let present = (EpochRecordKind::Studio, hash(1));
        let gone = (EpochRecordKind::Studio, hash(2));
        let uncovered = (EpochRecordKind::Registry, hash(3));
        for key in [present, gone, uncovered] {
            cache.put(key, 10, digest, record());
        }
        cache.prune(
            |family| family == EpochRecordKind::Studio,
            |key| *key == present,
        );
        assert!(
            cache.holds_for_test(present),
            "a record still on disk was pruned"
        );
        assert!(
            !cache.holds_for_test(gone),
            "a deleted record's entry survived pruning"
        );
        assert!(
            cache.holds_for_test(uncovered),
            "pruning reached a family outside the scan's coverage"
        );
        assert!(consistent(&cache));
        // The pruned record can be cached again, with a vacant-only put.
        assert!(cache.put_if_vacant(gone, 10, digest, record()));
        assert!(consistent(&cache));
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
