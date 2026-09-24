//! Design 13.7: what one C-3 visit costs, and what the classifier can actually bound.
//!
//! `validation_fits` decides exactly one thing: whether a record's typed validation runs inside
//! the visit that read it, or is parked for a later one. Calibrating it therefore needs the cost
//! of the thing it defers, which is **not** the cost of a visit. A visit does two kinds of work
//! per record, and only the second is deferrable:
//!
//! 1. Open the file, read and authenticate it, decode its scope, check the filename against the
//!    authenticated scope, digest the plaintext, consult the validation cache. This happens
//!    *before* classification - it has to, because the classifier's inputs are the family and the
//!    authenticated size, neither of which is known until the body is authenticated. No threshold
//!    can defer it. It is the custody floor per record.
//! 2. [`validate_record_body`]: the typed decode, the footprint, and for a reference scan the
//!    canonical reference inspection. This is the whole of what parking moves out of the visit.
//!
//! A single "cost per record" figure would calibrate nothing, because it would mix a term the
//! threshold controls with one it cannot. These measurements separate them, and both come from
//! the production path rather than a reimplementation of it: because `validation_fits` currently
//! returns false for every record, a budgeted scan parks every one, and the parked record's own
//! validation is term 2 in isolation. Term 1 is then the visit's measured custody minus it.
//!
//! **Resolution.** `scripts/check-no-ambient.sh` forbids `Instant::now` everywhere under
//! `crates/`, test code included, so the finest clock available is `catcoms_rt::Clock` at
//! milliseconds. One record's validation can round to zero against that. Every per-record figure
//! here is therefore a batch of `REPETITIONS` runs divided by the count, and is reported in
//! microseconds only to carry that division - not to claim microsecond measurement.
//!
//! **What this does not measure.** Reference collection for the Recovery family needs a
//! canonically valid projection, because the inspector decodes and validates every operation in
//! it; the opaque projections staged here would be refused. The with-references half of 13.7 is
//! therefore not covered for Recovery by this module, and is recorded as outstanding in the
//! status ledger rather than reported as a zero.

use super::*;
use catcoms_rt::SystemClock;

/// Enough repetitions that a millisecond clock resolves the per-record figure.
const REPETITIONS: usize = 64;

/// One record's deferrable cost, grouped by the facts the classifier sees.
#[derive(Debug, Clone, Copy)]
struct RecordCost {
    family: EpochRecordKind,
    size: u64,
    references: bool,
    /// `validate_record_body` alone, batched over `REPETITIONS` and divided.
    detached_us: u128,
    /// The raw batch total. Reported so a `detached_us` of 0 is visibly "below what a
    /// millisecond clock resolves even over a batch" rather than "free".
    batch_ms: u64,
    /// Custody of the visit that read and parked this record: term 1, which no threshold can
    /// defer. Captured here so the two terms can be compared per record rather than in aggregate.
    visit_ms: u64,
}

/// The whole-scan figures 13.7 asks for alongside the per-record ones.
#[derive(Debug, Default)]
struct ScanCost {
    visits: usize,
    /// Continuous custody per visit, in milliseconds. 13.7's "maximum continuous custody per
    /// scan slice" is the maximum of these, not their mean.
    custody_ms: Vec<u64>,
    records: Vec<RecordCost>,
}

impl ScanCost {
    fn max_custody_ms(&self) -> u64 {
        self.custody_ms.iter().copied().max().unwrap_or(0)
    }
    fn largest(&self, family: EpochRecordKind) -> Option<RecordCost> {
        self.records
            .iter()
            .filter(|r| r.family == family)
            .copied()
            .max_by_key(|r| r.size)
    }
}

/// Drive one full budgeted scan, timing each visit and each detached validation.
///
/// The budget is deliberately large: the point is not to observe the deadline firing, it is to
/// put the cursor in the mode where every record parks, so that term 2 can be timed on its own.
fn profile_scan(
    store: &mut ServerStore,
    coverage: EpochInventoryCoverage,
    clock: &dyn catcoms_rt::Clock,
) -> ScanCost {
    let mut out = ScanCost::default();
    let mut cursor = store.begin_epoch_storage_scan(coverage).unwrap();
    loop {
        let before = clock.monotonic_ms();
        let progress = store
            .step_epoch_storage_scan(&mut cursor, ENTRIES_PER_STEP, Some((clock, u64::MAX)))
            .unwrap();
        out.visits += 1;
        let visit_ms = clock.monotonic_ms().saturating_sub(before);
        out.custody_ms.push(visit_ms);
        if let Some(parked) = store.take_parked_record(&mut cursor) {
            let (family, size, references) = parked.classification();
            let start = clock.monotonic_ms();
            for _ in 0..REPETITIONS {
                parked.revalidate().unwrap();
            }
            let batch_ms = clock.monotonic_ms().saturating_sub(start);
            out.records.push(RecordCost {
                family,
                size,
                references,
                detached_us: batch_ms as u128 * 1_000 / REPETITIONS as u128,
                batch_ms,
                // A parked record ends its visit, so the visit just timed is the one that read
                // and parked exactly this record.
                visit_ms,
            });
            let validated = parked.validate().unwrap();
            store
                .install_validated_record(&mut cursor, validated)
                .unwrap();
            continue;
        }
        if progress.complete {
            break;
        }
    }
    store.finish_epoch_storage_scan(cursor).unwrap();
    out
}

/// Stage a Recovery record whose authenticated body is at least `projection` bytes.
///
/// The projection is opaque filler, which is legal for everything except reference collection:
/// `EpochRecoveryState::decode` and `footprint` treat it as bytes. See the module note on what
/// that excludes.
fn stage_sized(
    store: &mut ServerStore,
    server: u64,
    document: &LogicalDocument,
    projection: usize,
) {
    let snapshot = RecoverySnapshot {
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        epoch: 0,
        base_close_record_hash: None,
        reason: RecoveryReason::Excluded,
        projection: vec![7; projection],
        tombstones: vec![],
        elements: vec![],
        conflicts: vec![],
        applied_ops: vec![],
    };
    store
        .update_epoch_recovery(
            server,
            document,
            EpochRecoveryAction::Stage(snapshot),
            &ManualClock::new(0),
            &mut ChaCha20Rng::seed_from_u64(9),
        )
        .unwrap();
}

/// The 13.7 measurement for the Recovery family, across a size range.
///
/// Reports one line per record so the size dependence is visible rather than averaged away: a
/// threshold needs to know whether the deferrable term is dominated by a per-record constant or
/// by bytes, and a single mean cannot say.
fn measure_recovery(sizes: &[usize], clock: &dyn catcoms_rt::Clock) {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    for (n, size) in sizes.iter().enumerate() {
        let key = format!("recovery-{n}");
        stage_sized(&mut store, 7, &document(b"group", key.as_bytes()), *size);
    }

    let cost = profile_scan(&mut store, EpochInventoryCoverage::RecoveryOnly, clock);
    assert_eq!(
        cost.records.len(),
        sizes.len(),
        "every staged record must have parked, or the detached term was not measured for it"
    );
    let mut records = cost.records.clone();
    records.sort_by_key(|r| r.size);
    for record in &records {
        // `deferrable_pct` is the whole point of the pair: it is the share of one record's visit
        // that the classifier is in a position to move, and 100 minus it is the share that stays
        // no matter what threshold is chosen.
        // Unlike `detached_us`, `visit_ms` is a single unbatched sample at millisecond
        // resolution - a scan step cannot be repeated the way a pure validation can. Below about
        // a megabyte it reads 0, which would make the ratio report 100% deferrable when what it
        // actually means is "term 1 was too small for this clock to see". Say so rather than
        // printing a number that invites the wrong conclusion.
        let total = record.visit_ms as u128 * 1_000 + record.detached_us;
        let deferrable = if record.visit_ms == 0 || total == 0 {
            "unresolved".to_string()
        } else {
            format!("{}", record.detached_us * 100 / total)
        };
        println!(
            "C3_PROFILE family={:?} bytes={} references={} detached_us={} batch_ms={} \
             visit_ms={} deferrable_pct={}",
            record.family,
            record.size,
            record.references,
            record.detached_us,
            record.batch_ms,
            record.visit_ms,
            deferrable,
        );
    }
    println!(
        "C3_PROFILE scan=recovery visits={} records={} max_custody_ms={}",
        cost.visits,
        cost.records.len(),
        cost.max_custody_ms()
    );
    let largest = cost.largest(EpochRecordKind::Recovery).unwrap();
    println!(
        "C3_PROFILE largest_single_record family=Recovery bytes={} references={} \
         detached_us={} unavoidable_visit_ms={}",
        largest.size, largest.references, largest.detached_us, largest.visit_ms
    );
}

/// The harness itself, on a `ManualClock` that never advances.
///
/// Asserts the structure the profile depends on - every record parks, one visit per parked
/// record plus a final one, the largest record is the one staged largest - and deliberately
/// asserts nothing about durations. A frozen clock reports zero for all of them, which is the
/// point: this runs in the ordinary suite on any machine, and a timing assertion there would be
/// a machine-speed assertion in disguise.
#[test]
fn c3_visit_profile_smoke() {
    let sizes = [512, 4 * 1024, 32 * 1024];
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    for (n, size) in sizes.iter().enumerate() {
        let key = format!("recovery-{n}");
        stage_sized(&mut store, 7, &document(b"group", key.as_bytes()), *size);
    }
    let clock = ManualClock::new(0);
    let cost = profile_scan(&mut store, EpochInventoryCoverage::RecoveryOnly, &clock);

    assert_eq!(
        cost.records.len(),
        sizes.len(),
        "a record did not park, so the classifier is no longer detaching everything and this \
         measurement no longer isolates the deferrable term"
    );
    assert!(
        cost.records.iter().all(|r| !r.references),
        "a reference scan was profiled by a fixture whose projections are opaque filler"
    );
    assert_eq!(
        cost.largest(EpochRecordKind::Recovery).unwrap().size,
        cost.records.iter().map(|r| r.size).max().unwrap(),
        "largest() did not select the largest record"
    );
    assert!(
        cost.visits > cost.records.len(),
        "a parked record ends its visit, so a scan that parked {} records cannot have taken \
         fewer than {} visits",
        cost.records.len(),
        cost.records.len() + 1
    );
}

/// Opt-in, real clock, real sizes. Prints; asserts nothing about machine speed.
#[test]
#[ignore = "opt-in design 13.7 profiling of C-3 visit cost; no machine-speed assertion"]
fn profile_c3_visit_cost() {
    measure_recovery(
        &[1024, 16 * 1024, 256 * 1024, 1024 * 1024, 4 * 1024 * 1024],
        &SystemClock,
    );
}
