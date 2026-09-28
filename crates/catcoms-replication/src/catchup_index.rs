//! Conservative, bounded certificates of ancestry in this document's exact signed-log order.
//! Derived only from locally available changes; never serialized or populated by peer claims.
use std::collections::{BTreeMap, VecDeque};

use automerge::ChangeHash;

pub(crate) const MAX_ENTRIES: usize = 4_096;
pub(crate) const MAX_RANGES: usize = 8;
pub(crate) const MAX_PARENTS: usize = 32;
pub(crate) const MAX_MERGED_RANGES: usize = MAX_PARENTS * MAX_RANGES + 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LogRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct Certificate {
    ranges: [LogRange; MAX_RANGES],
    len: u8,
    /// All dependency branches were represented when recorded. Later duplicate envelopes may
    /// still lie outside these ranges; the optimization may resend those, never hide a gap.
    pub complete: bool,
}

impl Certificate {
    pub(crate) fn ranges(&self) -> &[LogRange] {
        &self.ranges[..usize::from(self.len)]
    }
}

pub(crate) struct AncestryIndex {
    certificates: BTreeMap<ChangeHash, Certificate>,
    insertion_order: VecDeque<ChangeHash>,
    capacity: usize,
}

impl Default for AncestryIndex {
    fn default() -> Self {
        Self::with_capacity(MAX_ENTRIES)
    }
}

impl AncestryIndex {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            certificates: BTreeMap::new(),
            insertion_order: VecDeque::new(),
            capacity: capacity.min(MAX_ENTRIES),
        }
    }

    pub(crate) fn get(&self, hash: &ChangeHash) -> Option<&Certificate> {
        self.certificates.get(hash)
    }

    /// The caller must first verify that this exact change is available in its Automerge DAG.
    /// At most 32 dependencies and 257 fixed-size ranges are inspected/merged per insertion.
    pub(crate) fn record(&mut self, position: usize, hash: ChangeHash, deps: &[ChangeHash]) {
        let Some(end) = position.checked_add(1).and_then(|n| u32::try_from(n).ok()) else {
            return;
        };
        if self.capacity == 0 {
            return;
        }
        let mut ranges = Vec::with_capacity(MAX_MERGED_RANGES);
        let mut complete = true;
        if let Some(existing) = self.certificates.get(&hash) {
            // Two signed envelopes can carry the same change. Extend its existing certificate
            // to the new slot without another queue entry or unbounded dependency union.
            ranges.extend_from_slice(existing.ranges());
            complete = existing.complete;
        } else {
            complete &= deps.len() <= MAX_PARENTS;
            for dependency in deps.iter().take(MAX_PARENTS) {
                if let Some(parent) = self.certificates.get(dependency) {
                    ranges.extend_from_slice(parent.ranges());
                    complete &= parent.complete;
                } else {
                    complete = false;
                }
            }
        }
        ranges.push(LogRange {
            start: end - 1,
            end,
        });
        debug_assert!(ranges.len() <= MAX_MERGED_RANGES);
        merge_ranges(&mut ranges);
        if ranges.len() > MAX_RANGES {
            complete = false;
            // Retain the broadest proved intervals, earliest first on ties. Never manufacture
            // an interval across an unknown sibling; dropped facts only cost duplicate payload.
            ranges.sort_unstable_by_key(|range| {
                (std::cmp::Reverse(range.end - range.start), range.start)
            });
            ranges.truncate(MAX_RANGES);
            ranges.sort_unstable_by_key(|range| range.start);
        }
        let mut certificate = Certificate {
            ranges: [LogRange::default(); MAX_RANGES],
            len: ranges.len() as u8,
            complete,
        };
        certificate.ranges[..ranges.len()].copy_from_slice(&ranges);
        if !self.certificates.contains_key(&hash) {
            if self.certificates.len() >= self.capacity {
                if let Some(oldest) = self.insertion_order.pop_front() {
                    self.certificates.remove(&oldest);
                }
            }
            self.insertion_order.push_back(hash);
        }
        self.certificates.insert(hash, certificate);
    }
}

pub(crate) fn merge_ranges(ranges: &mut Vec<LogRange>) {
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    let mut written = 0;
    for read in 0..ranges.len() {
        let next = ranges[read];
        if written > 0 && ranges[written - 1].end >= next.start {
            ranges[written - 1].end = ranges[written - 1].end.max(next.end);
        } else {
            ranges[written] = next;
            written += 1;
        }
    }
    ranges.truncate(written);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(n: usize) -> ChangeHash {
        let mut bytes = [0; 32];
        bytes[..8].copy_from_slice(&(n as u64).to_le_bytes());
        ChangeHash(bytes)
    }

    #[test]
    fn a_two_entry_cache_carries_a_long_certified_prefix_without_retaining_the_graph() {
        let mut index = AncestryIndex::with_capacity(2);
        for n in 0..10_000 {
            let parent = (n > 0).then(|| hash(n - 1));
            index.record(n, hash(n), parent.as_slice());
            assert!(index.certificates.len() <= 2);
            assert!(index.insertion_order.len() <= 2);
            assert_eq!(
                index.get(&hash(n)).unwrap().ranges(),
                &[LogRange {
                    start: 0,
                    end: n as u32 + 1
                }]
            );
        }
        assert!(index.get(&hash(0)).is_none());
        let last = index.get(&hash(9_999)).unwrap();
        assert!(last.complete);
        assert_eq!(
            last.ranges(),
            &[LogRange {
                start: 0,
                end: 10_000
            }]
        );
    }

    #[test]
    fn fragmentation_and_dependency_limits_never_bridge_unknown_siblings() {
        let mut index = AncestryIndex::default();
        for n in 0..MAX_PARENTS + 3 {
            index.record(n * 2, hash(n), &[]);
        }
        let deps: Vec<_> = (0..MAX_PARENTS + 3).map(hash).collect();
        index.record(100, hash(100), &deps);
        let certificate = index.get(&hash(100)).unwrap();
        assert!(!certificate.complete);
        assert_eq!(certificate.ranges().len(), MAX_RANGES);
        assert!(certificate
            .ranges()
            .iter()
            .all(|range| range.end == range.start + 1 && range.start % 2 == 0));
        assert_eq!(certificate.ranges()[0], LogRange { start: 0, end: 1 });
        assert_eq!(MAX_MERGED_RANGES, 257);
        assert!(std::mem::size_of::<Certificate>() <= 72);
    }

    #[test]
    fn duplicate_envelopes_and_unrepresentable_positions_do_not_grow_the_cache() {
        let mut index = AncestryIndex::with_capacity(2);
        index.record(0, hash(0), &[]);
        index.record(2, hash(0), &[]);
        assert_eq!(index.insertion_order.len(), 1);
        assert_eq!(
            index.get(&hash(0)).unwrap().ranges(),
            &[LogRange { start: 0, end: 1 }, LogRange { start: 2, end: 3 }]
        );
        index.record(u32::MAX as usize, hash(1), &[]);
        assert!(index.get(&hash(1)).is_none());
        let mut disabled = AncestryIndex::with_capacity(0);
        disabled.record(0, hash(0), &[]);
        assert!(disabled.certificates.is_empty());
    }
}
