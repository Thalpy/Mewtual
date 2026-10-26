//! Planner tests: order, chunk positions, pins on every class, thresholds and migration. Sizes are
//! small integers; the planner never looks at real bytes.

use super::*;

fn cid(n: u8) -> Cid {
    Cid::from_bytes([n; 32])
}

fn index(entries: &[(u8, u64)]) -> CacheIndex {
    CacheIndex::from_sizes(entries.iter().map(|(n, s)| (cid(*n), *s)).collect(), true)
}

fn row(position: usize, file: u8, chunks: &[u8]) -> PlanRow {
    PlanRow {
        position,
        file: cid(file),
        chunks: chunks.iter().map(|n| cid(*n)).collect(),
    }
}

fn set(ns: &[u8]) -> BTreeSet<Cid> {
    ns.iter().map(|n| cid(*n)).collect()
}

struct Case {
    index: CacheIndex,
    rows: Vec<PlanRow>,
    profile: BTreeSet<Cid>,
    pins: BTreeSet<Cid>,
    retained: BTreeSet<Cid>,
    limit: Option<u64>,
    unclaimed_allowed: bool,
}

impl Case {
    fn new(entries: &[(u8, u64)], rows: Vec<PlanRow>, limit: Option<u64>) -> Self {
        Self {
            index: index(entries),
            rows,
            profile: BTreeSet::new(),
            pins: BTreeSet::new(),
            retained: BTreeSet::new(),
            limit,
            unclaimed_allowed: true,
        }
    }
    fn plan(&self) -> CachePlan {
        plan(PlanInput {
            index: &self.index,
            rows: &self.rows,
            profile_refs: &self.profile,
            pins: &self.pins,
            retained: &self.retained,
            limit: self.limit,
            unclaimed_allowed: self.unclaimed_allowed,
        })
    }
}

#[test]
fn oldest_listing_leaves_first_and_eviction_stops_at_the_limit() {
    // Three files of 40 bytes each (two chunks of 20), listed at positions 0, 1, 2.
    let case = Case::new(
        &[(1, 20), (2, 20), (3, 20), (4, 20), (5, 20), (6, 20)],
        vec![
            row(0, 101, &[1, 2]),
            row(1, 102, &[3, 4]),
            row(2, 103, &[5, 6]),
        ],
        Some(80),
    );
    let plan = case.plan();
    assert_eq!(plan.usage, 120);
    assert_eq!(
        plan.evict,
        vec![cid(1), cid(2)],
        "the oldest file leaves whole"
    );
    assert_eq!(plan.usage_after(), 80);
    assert!(!plan.over_after);
    // Not last viewed: nothing in the input can reorder by access.
    let mut reordered = case;
    reordered.rows.reverse();
    assert_eq!(reordered.plan().evict, vec![cid(1), cid(2)]);
}

#[test]
fn a_chunk_takes_its_newest_listing_position() {
    // File 101 (position 0) and a relisting of the same chunks at position 5 make them new; a
    // hostile old row (position 1) naming file 103's chunks cannot drag them out early.
    let case = Case::new(
        &[(1, 10), (2, 10), (3, 10), (9, 10)],
        vec![
            row(0, 101, &[1]),
            row(1, 66, &[3, 9]),
            row(2, 102, &[2]),
            row(5, 101, &[1]),
            row(6, 103, &[3]),
        ],
        Some(20),
    );
    let plan = case.plan();
    // Positions: 9 → 1 (only the hostile row), 2 → 2, 1 → 5, 3 → 6.
    assert_eq!(plan.evict, vec![cid(9), cid(2)]);
    // And a hostile row naming newer chunks does not shield its own older chunk (9).
}

#[test]
fn unclaimed_blobs_go_first_but_only_from_the_mount_set_and_never_profile_images() {
    let mut case = Case::new(
        &[(1, 10), (2, 10), (7, 10), (8, 10), (9, 10)],
        vec![row(0, 101, &[1]), row(1, 102, &[2])],
        Some(20),
    );
    case.profile = set(&[8]);
    let plan = case.plan();
    assert_eq!(plan.evict, vec![cid(7), cid(9), cid(1)]);
    assert_eq!(
        plan.leaving_soon_files, 2,
        "unclaimed bytes are not counted as files"
    );

    // A CID written during this mount has left the mount set and is never unclaimed.
    let sizes = [(1u8, 10u64), (2, 10), (7, 10)]
        .into_iter()
        .map(|(n, s)| (cid(n), s))
        .collect();
    case.index = CacheIndex::with_mount(sizes, set(&[1, 2]), true);
    case.profile.clear();
    case.limit = Some(20);
    assert_eq!(
        case.plan().evict,
        vec![cid(1)],
        "7 is held but not unclaimed"
    );
}

#[test]
fn hidden_rows_skip_only_the_unclaimed_class() {
    // Rows past the 256 cap (or an unreadable profile) make "unclaimed" unknowable, but visible
    // listed files can still leave safely: a hidden row's chunks are never offered in class 2.
    let mut case = Case::new(
        &[(1, 10), (2, 10), (9, 10)],
        vec![row(0, 101, &[1]), row(1, 102, &[2])],
        Some(20),
    );
    case.unclaimed_allowed = false;
    let plan = case.plan();
    assert_eq!(plan.evict, vec![cid(1)], "9 may belong to a hidden row");
    case.unclaimed_allowed = true;
    assert_eq!(case.plan().evict, vec![cid(9)]);
}

#[test]
fn every_pin_set_protects_every_class() {
    // 1 = unclaimed, 2 = listed old, 3 = listed newer, 4 = completed-Keep duplicate.
    let mut case = Case::new(
        &[(1, 10), (2, 10), (3, 10), (4, 10)],
        vec![row(0, 102, &[2]), row(1, 103, &[3]), row(2, 104, &[4])],
        Some(10),
    );
    case.retained = set(&[4]);
    case.pins = set(&[1, 2, 4]);
    let plan = case.plan();
    assert!(plan.duplicates.is_empty(), "a pinned Keep duplicate stays");
    assert_eq!(plan.evict, vec![cid(3)]);
    assert_eq!(plan.pinned_bytes, 30);
    assert!(plan.over_after, "pins alone keep usage above the limit");
    case.pins = set(&[1, 2, 3, 4]);
    let plan = case.plan();
    assert!(plan.evict.is_empty() && plan.over_after);
    assert_eq!(
        plan.leaving_soon_files, 0,
        "nothing pinned is ever counted as leaving"
    );
}

#[test]
fn completed_keep_duplicates_drop_even_under_the_limit() {
    let mut case = Case::new(&[(1, 10), (2, 10)], vec![row(0, 101, &[1, 2])], Some(1000));
    case.retained = set(&[2]);
    let plan = case.plan();
    assert_eq!(plan.duplicates, vec![cid(2)]);
    assert!(plan.evict.is_empty());
    assert_eq!(plan.usage_after(), 10);
    case.limit = None;
    assert_eq!(
        case.plan().duplicates,
        vec![cid(2)],
        "No limit still drops duplicates"
    );
}

#[test]
fn no_limit_evicts_nothing_and_reports_nothing_leaving() {
    let case = Case::new(&[(1, 10), (9, 10)], vec![row(0, 101, &[1])], None);
    let plan = case.plan();
    assert!(plan.evict.is_empty());
    assert_eq!((plan.leaving_soon_files, plan.leaving_soon_bytes), (0, 0));
    assert!(!plan.over_after);
}

#[test]
fn leaving_soon_counts_what_the_next_tenth_of_the_limit_pushes_out() {
    // Limit 100; headroom 10. Five files of 18 bytes at positions 0..5.
    let entries: Vec<(u8, u64)> = (1..=5).map(|n| (n, 18)).collect();
    let rows: Vec<PlanRow> = (1..=5).map(|n| row(n as usize, 100 + n, &[n])).collect();
    let mut case = Case::new(&entries, rows, Some(100));
    // Usage 90: 90 + 10 = 100, nothing pushed out yet.
    let plan = case.plan();
    assert_eq!((plan.leaving_soon_files, plan.leaving_soon_bytes), (0, 0));
    assert!(plan.evict.is_empty());
    // Usage 110 (one more 20-byte file): over by 10, so file 1 goes now (down to 92). With the
    // headroom, 120 needs 20 bytes freed: file 1 (18) is not enough, so file 2 is leaving too.
    case.index = index(&[(1, 18), (2, 18), (3, 18), (4, 18), (5, 18), (6, 20)]);
    case.rows.push(row(6, 106, &[6]));
    let plan = case.plan();
    assert_eq!(plan.evict, vec![cid(1)]);
    assert_eq!(plan.usage_after(), 92);
    assert_eq!(plan.leaving_soon_files, 2);
    assert_eq!(plan.leaving_soon_bytes, 36);
    // Exactly at the boundary nothing extra is counted: 108 + 10 - 18 = 100 meets the limit.
    case.index = index(&[(1, 18), (2, 18), (3, 18), (4, 18), (5, 18), (6, 18)]);
    let plan = case.plan();
    assert_eq!((plan.leaving_soon_files, plan.leaving_soon_bytes), (1, 18));
}

#[test]
fn a_file_with_several_variants_counts_once() {
    // Two manifests of one plaintext file (101) and its relisting share the count.
    let case = Case::new(
        &[(1, 10), (2, 10), (3, 10)],
        vec![row(0, 101, &[1]), row(0, 101, &[2]), row(1, 102, &[3])],
        Some(10),
    );
    let plan = case.plan();
    assert_eq!(plan.evict, vec![cid(1), cid(2)]);
    assert_eq!(
        plan.leaving_soon_files, 2,
        "files 101 and 102, not three rows"
    );
}

#[test]
fn migration_never_starts_below_current_usage_or_above_the_maximum() {
    const GIB: u64 = MIN_CACHE_LIMIT;
    assert_eq!(migrated_limit(0), Some(DEFAULT_CACHE_LIMIT));
    assert_eq!(migrated_limit(4 * GIB), Some(DEFAULT_CACHE_LIMIT));
    // 4 GiB + 1 byte needs 5 GiB + 2 bytes, rounded up to 6 GiB.
    assert_eq!(migrated_limit(4 * GIB + 1), Some(6 * GIB));
    assert_eq!(migrated_limit(8 * GIB), Some(10 * GIB));
    for usage in [0, 1, GIB, 7 * GIB + 3, 800 * GIB] {
        let limit = migrated_limit(usage).unwrap();
        assert!(usage * 10 <= limit * 8, "starts at or below 80%: {usage}");
        assert!(valid_limit(Some(limit)));
    }
    // Usage / 0.8 above 1 TiB would be an illegal limit: No limit instead, never a smaller one.
    assert_eq!(migrated_limit(820 * GIB), None);
    assert_eq!(migrated_limit(u64::MAX), None);
}

#[test]
fn only_limits_in_range_are_valid() {
    assert!(valid_limit(None));
    assert!(valid_limit(Some(MIN_CACHE_LIMIT)));
    assert!(valid_limit(Some(MAX_CACHE_LIMIT)));
    assert!(!valid_limit(Some(MIN_CACHE_LIMIT - 1)));
    assert!(!valid_limit(Some(MAX_CACHE_LIMIT + 1)));
    assert!(!valid_limit(Some(0)));
}
