//! The cache eviction planner: a pure function from facts to a plan.
//!
//! Inputs are the primary cache's size index (sizes plus the mount set, from
//! [`catcoms_storage::CacheIndex`]), the live file-index rows in listing order, the avatar and
//! banner references of the profile document, the union of every pin set (each one a set of chunk
//! addresses taken from its owner's own saved plans), the chunks a completed Keep copy holds, and
//! the limit. Nothing here touches a disk, a clock or the network, so the order and the pins are
//! tested exhaustively below; the actor only gathers the inputs and executes the deletes.
//!
//! Order (see `docs/design-cache-housekeeping.md`, section 1):
//!
//! 0. cache duplicates of chunks a completed Keep copy holds, always, over the limit or not;
//! 1. unclaimed blobs (in the mount set, named by no live row or profile), in CID order;
//! 2. listed chunks, oldest **chunk position** first, one listing position at a time.
//!
//! A chunk's position is the newest position of any live row naming it. That is what stops a
//! hostile old row from dragging a newer file's chunks out early, and what lets a hostile row that
//! names newer chunks *not* shield its own older chunks: they keep their own, older, position.
//! Pinned chunks are excluded from every class, including 0 and 1.

use std::collections::{BTreeMap, BTreeSet};

use catcoms_storage::{CacheIndex, Cid};

/// The smallest limit a user may set: the largest legal file, so one file always fits.
pub const MIN_CACHE_LIMIT: u64 = 1024 * 1024 * 1024;
/// The largest limit a user may set.
pub const MAX_CACHE_LIMIT: u64 = 1024 * MIN_CACHE_LIMIT;
/// The default limit for a server with no setting yet.
pub const DEFAULT_CACHE_LIMIT: u64 = 5 * MIN_CACHE_LIMIT;
/// "Leaving soon" looks this fraction of the limit ahead (10%).
pub const LEAVING_SOON_DIVISOR: u64 = 10;
/// The bar turns amber at this percentage of the limit.
pub const NEARLY_FULL_PERCENT: u64 = 90;

/// One live file-index row as the planner needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRow {
    /// Position in the replicated index (append order).
    pub position: usize,
    /// The file's whole-plaintext CID, which groups variants and repeated listings into one file.
    pub file: Cid,
    /// Ciphertext CIDs of this row's manifest.
    pub chunks: Vec<Cid>,
}

/// Everything the planner reads.
#[derive(Debug, Clone, Copy)]
pub struct PlanInput<'a> {
    pub index: &'a CacheIndex,
    pub rows: &'a [PlanRow],
    pub profile_refs: &'a BTreeSet<Cid>,
    pub pins: &'a BTreeSet<Cid>,
    pub retained: &'a BTreeSet<Cid>,
    /// `None` = No limit.
    pub limit: Option<u64>,
    /// False when some rows are invisible (over the 256-row cap) or the profile document cannot be
    /// read: "named by no row or profile" is then unknowable, so class 1 is skipped. Class 2 stays
    /// safe because a hidden row's chunks are never offered in it.
    pub unclaimed_allowed: bool,
}

/// What the planner decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CachePlan {
    pub usage: u64,
    pub limit: Option<u64>,
    /// Held bytes a pin set holds (they count in `usage` and are never offered for deletion).
    pub pinned_bytes: u64,
    /// Class 0: held chunks a completed Keep copy also holds.
    pub duplicates: Vec<Cid>,
    /// Classes 1 then 2, in order: exactly what brings usage (after class 0) to the limit.
    pub evict: Vec<Cid>,
    /// Bytes the whole plan frees (classes 0, 1 and 2).
    pub freed: u64,
    /// Listed files the next 10% of the limit would push out, including any `evict` touches now.
    pub leaving_soon_files: usize,
    pub leaving_soon_bytes: u64,
    /// Usage still above the limit after the plan: only pins can cause this.
    pub over_after: bool,
}

impl CachePlan {
    /// Usage after the plan runs.
    pub fn usage_after(&self) -> u64 {
        self.usage.saturating_sub(self.freed)
    }
}

/// One deletion candidate with the file it belongs to (for counting) and its size.
struct Candidate {
    cid: Cid,
    file: Option<Cid>,
    bytes: u64,
}

/// Build the plan. See the module docs for the order.
pub fn plan(input: PlanInput<'_>) -> CachePlan {
    let sizes = input.index.sizes();
    let usage = input.index.usage();
    let pinned_bytes = sizes
        .iter()
        .filter(|(cid, _)| input.pins.contains(cid))
        .fold(0u64, |sum, (_, n)| sum.saturating_add(*n));

    // Class 0 runs whatever the limit says.
    let duplicates: Vec<Cid> = sizes
        .keys()
        .filter(|cid| input.retained.contains(cid) && !input.pins.contains(cid))
        .copied()
        .collect();
    let freed_duplicates = duplicates
        .iter()
        .fold(0u64, |sum, cid| sum.saturating_add(sizes[cid]));
    let dropped: BTreeSet<Cid> = duplicates.iter().copied().collect();

    // Newest position of any live row naming each chunk, and the file that row lists.
    let mut chunk_position: BTreeMap<Cid, (usize, Cid)> = BTreeMap::new();
    for row in input.rows {
        for chunk in &row.chunks {
            let entry = chunk_position
                .entry(*chunk)
                .or_insert((row.position, row.file));
            if row.position >= entry.0 {
                *entry = (row.position, row.file);
            }
        }
    }

    let eligible =
        |cid: &Cid| sizes.contains_key(cid) && !input.pins.contains(cid) && !dropped.contains(cid);
    let mut ordered: Vec<Vec<Candidate>> = Vec::new();
    // Class 1, one candidate per group so the stop rule can end mid-class.
    for cid in input
        .index
        .mount_set()
        .iter()
        .filter(|_| input.unclaimed_allowed)
    {
        if eligible(cid) && !chunk_position.contains_key(cid) && !input.profile_refs.contains(cid) {
            ordered.push(vec![Candidate {
                cid: *cid,
                file: None,
                bytes: sizes[cid],
            }]);
        }
    }
    // Class 2, grouped by listing position so a file normally leaves as a whole.
    let mut by_position: BTreeMap<usize, Vec<Candidate>> = BTreeMap::new();
    for (cid, (position, file)) in &chunk_position {
        if eligible(cid) {
            by_position.entry(*position).or_default().push(Candidate {
                cid: *cid,
                file: Some(*file),
                bytes: sizes[cid],
            });
        }
    }
    ordered.extend(by_position.into_values());

    let after_duplicates = usage.saturating_sub(freed_duplicates);
    let mut evict = Vec::new();
    let mut freed = freed_duplicates;
    let mut over_after = false;
    let mut leaving_soon_files = 0;
    let mut leaving_soon_bytes = 0u64;
    if let Some(limit) = input.limit {
        // Evict now: stop as soon as usage is at or under the limit.
        let mut current = after_duplicates;
        for group in &ordered {
            if current <= limit {
                break;
            }
            for candidate in group {
                evict.push(candidate.cid);
                current = current.saturating_sub(candidate.bytes);
                freed = freed.saturating_add(candidate.bytes);
            }
        }
        over_after = current > limit;
        // Leaving soon: what the next 10% of the limit would push out, counted from the same
        // starting point so it includes what is about to go now.
        let headroom = limit / LEAVING_SOON_DIVISOR;
        let mut projected = after_duplicates.saturating_add(headroom);
        let mut files = BTreeSet::new();
        for group in &ordered {
            if projected <= limit {
                break;
            }
            for candidate in group {
                projected = projected.saturating_sub(candidate.bytes);
                leaving_soon_bytes = leaving_soon_bytes.saturating_add(candidate.bytes);
                if let Some(file) = candidate.file {
                    files.insert(file);
                }
            }
        }
        leaving_soon_files = files.len();
    }
    CachePlan {
        usage,
        limit: input.limit,
        pinned_bytes,
        duplicates,
        evict,
        freed,
        leaving_soon_files,
        leaving_soon_bytes,
        over_after,
    }
}

/// The starting limit for a cache with no saved setting: `max(5 GiB, usage / 0.8)` rounded up to
/// a whole GiB, or `None` (No limit) when that would exceed [`MAX_CACHE_LIMIT`]. The upgrade
/// therefore drops nothing and starts below the 90% warning. Callers must not migrate from an
/// incomplete scan: unseen bytes would make the starting limit too small.
pub fn migrated_limit(usage: u64) -> Option<u64> {
    // usage / 0.8 == usage * 5 / 4, computed without overflow and rounded up.
    let needed = (usage as u128 * 5).div_ceil(4);
    let whole = needed.div_ceil(MIN_CACHE_LIMIT as u128) * MIN_CACHE_LIMIT as u128;
    let limit = whole.max(DEFAULT_CACHE_LIMIT as u128);
    (limit <= MAX_CACHE_LIMIT as u128).then_some(limit as u64)
}

/// Whether a user-chosen limit is allowed. `None` (No limit) always is.
pub fn valid_limit(limit: Option<u64>) -> bool {
    limit.is_none_or(|bytes| (MIN_CACHE_LIMIT..=MAX_CACHE_LIMIT).contains(&bytes))
}

#[cfg(test)]
mod tests;
