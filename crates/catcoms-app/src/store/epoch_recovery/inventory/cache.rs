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
//! [`RecordCache::prune`] drops the entries for files a scan did not find, at every finish that
//! issues an inventory, so in steady state it tracks the vault rather than its history. Memory is
//! about 300 bytes per entry, about 20 MiB at the bound, resident for the mount's life (15.7,
//! LOW). The bound is the only unconditional limit: entries for records deleted since the last
//! completed scan stay until another completes.
//!
//! It is indexed (a keyed map plus an access order), so get, put and eviction are logarithmic:
//! the linear scan the 64-entry form used would make a scan of 65 536 records quadratic.
//!
//! **Every family's accounting validation is memoized** (C-3 runtime design 15.2, M1), not only
//! Registry's and Studio's. Each entry holds what a hit must restore: the accounting record, and
//! for an Intents record its inventory facts too, which `EpochIntentBudget::from_inventory` builds
//! the Unconfirmed tally from. **An Intents entry always carries its facts, and no other family's
//! entry has any**; [`RecordCache::put_validated`] refuses anything else, because a fact-less
//! Intents hit would undercount live branches and mint too generous a budget.

use super::{EpochIntentInventoryFacts, EpochRecordKind, StorageRecord};
use crate::store::epoch_budget::MAX_ACCOUNTED_RECORDS;
use std::collections::BTreeMap;

type Key = (EpochRecordKind, [u8; 32]);
const MAX_RECORDS: usize = MAX_ACCOUNTED_RECORDS;

struct Verified {
    digest: blake3::Hash,
    physical_bytes: u64,
    record: StorageRecord,
    /// Present exactly when the key's family is Intents. Boxed, so the other families' entries,
    /// most of the memo, carry a pointer rather than the facts inline: about 64 bytes more per
    /// entry otherwise, and about 25 MiB at the bound instead of about 20 (review of M1, LOW-2).
    intent: Option<Box<EpochIntentInventoryFacts>>,
    /// Position in [`RecordCache::order`]: the entry with the smallest tick is evicted first.
    tick: u64,
}

/// What a hit restores: the accounting record and, for an Intents record, its inventory facts.
pub(in crate::store) type Memoized = (StorageRecord, Option<EpochIntentInventoryFacts>);

/// Whether `intent` is the right shape for an entry of `family` and physical `size`: facts for
/// Intents, none otherwise, and facts that charge exactly the entry's size, since a validation
/// charges the record's physical bytes (review of M1, LOW-1). A value whose facts came from
/// another record would otherwise go in silently.
fn facts_fit(
    family: EpochRecordKind,
    size: u64,
    intent: &Option<EpochIntentInventoryFacts>,
) -> bool {
    (family == EpochRecordKind::Intents) == intent.is_some()
        && intent.is_none_or(|facts| facts.charged_bytes == size)
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
    pub(super) fn get(&mut self, key: Key, size: u64, digest: blake3::Hash) -> Option<Memoized> {
        self.entries
            .get(&key)
            .filter(|v| v.physical_bytes == size && v.digest == digest)?;
        let tick = self.take_tick();
        let value = self.entries.get_mut(&key).expect("matched just above");
        self.order.remove(&value.tick);
        value.tick = tick;
        self.order.insert(tick, key);
        Some((value.record, value.intent.as_deref().copied()))
    }

    /// Memoize a family's accounting record, for a family with no inventory facts. The writer
    /// paths' form: Registry and Studio writers establish their own record and carry no facts.
    /// An Intents record must go through [`Self::put_validated`] with its facts, and is refused
    /// here (see the module documentation).
    pub(in crate::store) fn put(
        &mut self,
        key: Key,
        size: u64,
        digest: blake3::Hash,
        record: StorageRecord,
    ) {
        self.put_validated(key, size, digest, record, None);
    }

    /// Memoize a validation result: the accounting record, and the inventory facts that come with
    /// it, which must be present exactly for an Intents record. A result of the wrong shape is a
    /// caller error. It is refused, so a later hit can never return an Intents record without its
    /// facts, and it fails a debug build loudly. Returns whether it stored; a refusal leaves any
    /// version already cached for the record in place.
    pub(in crate::store) fn put_validated(
        &mut self,
        key: Key,
        size: u64,
        digest: blake3::Hash,
        record: StorageRecord,
        intent: Option<EpochIntentInventoryFacts>,
    ) -> bool {
        if !facts_fit(key.0, size, &intent) {
            debug_assert!(
                false,
                "a {:?} memo entry with the wrong inventory facts",
                key.0
            );
            return false;
        }
        self.remove(&key);
        // Evict until there is room, rather than once. With `entries` and `order` in step one
        // eviction always suffices, but the bound is what keeps this memo's memory finite, so it
        // must not rest on that alone: an order entry that named no live entry would otherwise
        // let `entries` grow past the bound by one each time it was popped (review of M2, LOW-1).
        while self.entries.len() >= MAX_RECORDS {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            self.entries.remove(&oldest);
        }
        let tick = self.take_tick();
        self.order.insert(tick, key);
        self.entries.insert(
            key,
            Verified {
                digest,
                physical_bytes: size,
                record,
                intent: intent.map(Box::new),
                tick,
            },
        );
        self.debug_check();
        true
    }

    /// `entries` and `order` name the same keys, each exactly once. Checked after every mutation
    /// in debug and test builds, where it is cheap enough at the sizes tests use; a release build
    /// relies on [`Self::put`]'s eviction loop for the bound.
    fn debug_check(&self) {
        debug_assert_eq!(
            self.entries.len(),
            self.order.len(),
            "the memo's index and access order disagree"
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
        self.debug_check();
    }

    /// Remove `key` from both maps. Every removal goes through here, so they cannot drift apart.
    fn remove(&mut self, key: &Key) {
        if let Some(old) = self.entries.remove(key) {
            self.order.remove(&old.tick);
        }
        self.debug_check();
    }

    fn take_tick(&mut self) -> u64 {
        let tick = self.next_tick;
        self.next_tick += 1;
        tick
    }

    /// [`Self::put_validated`], unless any version of `key` is already cached. Returns whether it
    /// stored.
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
        intent: Option<EpochIntentInventoryFacts>,
    ) -> bool {
        !self.entries.contains_key(&key) && self.put_validated(key, size, digest, record, intent)
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
    /// Design 13.7's measurement needs this, and so does any test that needs a record cold after a
    /// scan has read it. Every family is cacheable (C-3 runtime 15.2), so a repeated-trial profile
    /// measures one fresh validation and then
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
        assert!(
            cache
                .get((EpochRecordKind::Registry, hash(0)), 78, digest)
                .is_none(),
            "a hit ignored the physical size"
        );
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

    /// An Intents entry without its inventory facts is refused: a later hit would return the
    /// record without them, and the Unconfirmed tally built from them would undercount (C-3
    /// runtime 15.2). A debug build fails loudly; a release build refuses and reports it.
    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "wrong inventory facts"))]
    fn an_intents_entry_without_its_facts_is_refused() {
        let mut cache = RecordCache::default();
        let key = (EpochRecordKind::Intents, hash(1));
        assert!(!cache.put_validated(key, 10, blake3::hash(b"v"), record(), None));
        assert!(!cache.holds_for_test(key));
    }

    /// Facts that charge a different size from the entry's are refused (review of M1, LOW-1): a
    /// validation charges the record's own physical bytes, so they belong to another record. The
    /// same facts at the entry's own size are accepted, and a hit returns them.
    #[test]
    fn facts_from_another_record_are_refused_and_matching_facts_round_trip() {
        let mut cache = RecordCache::default();
        let key = (EpochRecordKind::Intents, hash(1));
        let facts = |charged_bytes| EpochIntentInventoryFacts {
            provenance: None,
            charged_bytes,
        };
        let digest = blake3::hash(b"v");
        assert!(cache.put_validated(key, 10, digest, record(), Some(facts(10))));
        assert_eq!(
            cache.get(key, 10, digest).and_then(|(_, intent)| intent),
            Some(facts(10)),
            "a hit did not return the facts it was given"
        );
        let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cache.put_validated(key, 10, digest, record(), Some(facts(11)))
        }));
        // A debug build fails loudly; a release build refuses quietly. Either way nothing is
        // stored, and the matching entry already cached is left in place.
        assert!(
            !matches!(refused, Ok(true)),
            "facts from another record were stored"
        );
        assert_eq!(
            cache.get(key, 10, digest).and_then(|(_, intent)| intent),
            Some(facts(10)),
            "facts from another record replaced the entry's own"
        );
    }

    /// And the converse: facts on any other family's entry are refused too, since no other
    /// family's validation produces them.
    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "wrong inventory facts"))]
    fn facts_on_another_familys_entry_are_refused() {
        let mut cache = RecordCache::default();
        let key = (EpochRecordKind::Studio, hash(1));
        let facts = EpochIntentInventoryFacts {
            provenance: None,
            charged_bytes: 10,
        };
        assert!(!cache.put_validated(key, 10, blake3::hash(b"v"), record(), Some(facts)));
        assert!(!cache.holds_for_test(key));
    }

    /// The bound holds after an eviction on mismatch, which removes an entry outside `put`'s own
    /// path (review of M2, LOW-1). Had the eviction left its access-order entry behind, a later
    /// put at the bound would pop that dangling tick and let the memo grow past its bound.
    #[test]
    fn the_bound_and_the_index_survive_an_eviction_on_mismatch() {
        let mut cache = RecordCache::default();
        let (v1, v2) = (blake3::hash(b"v1"), blake3::hash(b"v2"));
        let key = (EpochRecordKind::Studio, hash(0));
        cache.put(key, 10, v1, record());
        cache.evict_mismatch(key, 10, v2);
        assert!(consistent(&cache), "an eviction left the order behind");
        cache.put(key, 10, v2, record());
        for n in 1..=MAX_RECORDS {
            cache.put((EpochRecordKind::Registry, hash(n)), 10, v1, record());
        }
        assert_eq!(cache.entries.len(), MAX_RECORDS, "the bound was not held");
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
        assert!(cache.put_if_vacant(gone, 10, digest, record(), None));
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
        assert!(cache.put_if_vacant(key, 10, old, record(), None));
        assert!(
            !cache.put_if_vacant(key, 10, new, record(), None),
            "a vacant-only put replaced the version already cached"
        );
        assert!(cache.get(key, 10, old).is_some());
        assert!(cache.get(key, 10, new).is_none());
        // Another record's key is unaffected by this one's entry.
        assert!(cache.put_if_vacant(
            (EpochRecordKind::Registry, [3; 32]),
            10,
            new,
            record(),
            None
        ));
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
        assert!(cache.put_if_vacant(key, 10, v2, record(), None));
    }
}
