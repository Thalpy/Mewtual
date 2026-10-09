//! Design 13.7: component costs of a C-3 scan.
//!
//! # What is timed, and what is not
//!
//! A scan's custody is not one number and this harness does not produce one. It times four
//! phases separately, because they are moved by different things:
//!
//! | phase | covers | sampling |
//! |---|---|---|
//! | `read_and_park` | one `step_epoch_storage_scan`: directory entry, family from the filename, metadata, that family's sealed cap, the aggregate and cold-byte prechecks, read, authenticate, scope decode, filename binding, digest, cache probe, classify, park | one sample per record per trial |
//! | `validation` | `validate_record_body`, through [`ParkedEpochRecord::revalidate`] | batched `REPETITIONS` times on one resident plaintext |
//! | `install` | `install_validated_record`: for an accounting scan an entry insert; for a **reference** scan a CID-set and dependency merge, which is not the same cost | one sample per record per trial |
//! | `finish` | `finish_epoch_storage_scan` | one sample per trial |
//!
//! An earlier version of this module timed only the first two and described the first as "the
//! visit's measured custody minus" the second. Both were wrong: nothing was subtracted - the
//! step is timed directly - and installation and finalisation were not measured at all. That
//! matters most for reference scans, where installation merges CID sets rather than inserting an
//! accounting record.
//!
//! This harness also holds `&mut ServerStore` for the whole of [`profile_scan`]. These are
//! timings of prospective stage bodies, not observed actor custody releases.
//!
//! # Ordering: what the classifier can and cannot move
//!
//! `validation_fits` is called after a record has been read and authenticated, so changing its
//! threshold cannot move that preceding work. That is a property of **where the call currently
//! sits**, not a claim that no scheduling decision could precede authentication. The scanner
//! already derives a candidate family from the filename and an untrusted size from the file
//! metadata, and already uses both conservatively - to select the family's reader, to refuse a
//! file over that family's sealed cap, and to apply the aggregate and cold-byte prechecks -
//! before any body is read. Using an untrusted size to *limit* work is not the same as using it
//! to *accept* a record as authentic.
//!
//! So the defensible name for that term is **retained by the current read/authentication
//! boundary**, not "unavoidable". Moving authentication itself would be a separate design
//! question about key custody, input binding and lifecycle fences, and nothing here proposes it.
//!
//! # Conditions these numbers hold under
//!
//! Component costs under one specific condition, not a cold-storage or worst-case benchmark:
//!
//! - validation is repeated on a **resident** plaintext already in memory;
//! - the files were **written immediately before** the scan, so the page cache is warm;
//! - "cold" in `uncached_bytes` and `check_cold_bytes` means the **validation cache**. That is a
//!   different thing from a cold filesystem cache and must not be reported as one;
//! - [`ParkedEpochRecord::revalidate`] times `run(&self)` and the drop of its temporary result.
//!   It does **not** time the consuming `validate(self)` lifecycle, including release of the
//!   parked plaintext. Sharing `run` prevents validator drift; it does not make the two
//!   lifecycles identical.
//!
//! # Resolution
//!
//! `scripts/check-no-ambient.sh` forbids direct OS clock reads everywhere under `crates/`, test code
//! included, so the finest clock available is `catcoms_rt::Clock` at milliseconds. A scan step
//! cannot be replayed on an advanced cursor, but an equivalent scan can be repeated on a fresh
//! one, so the per-record phases are summed over `TRIALS` complete scans with the individual
//! samples retained. A single one-millisecond sample is not evidence of a ratio, and the
//! reported fraction is a fraction of two measured components - not a measured share of total
//! custody, and not a before/after speedup.

use super::*;
use crate::store::measure::Spread;
use catcoms_replication::studio::{FlipnoteOp, StudioEpoch, StudioRecovery, StudioTarget};
use catcoms_replication::DomainOp;
use catcoms_rt::SystemClock;
use catcoms_storage::Cid;
use rand_core::RngCore;
use std::collections::BTreeMap;

/// Reference collection refuses anything narrower: a partial inventory must not be allowed to
/// replace a transient pre-publication hold.
const REFERENCE_COVERAGE: EpochInventoryCoverage =
    EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsRegistryAndStudio;
/// Enough repetitions that a millisecond clock resolves the per-record validation figure.
const REPETITIONS: usize = 64;
/// Complete scans, so the single-sample phases are summed rather than reported from one tick.
const TRIALS: usize = 8;

/// One record's per-phase cost, as retained samples rather than a running sum.
#[derive(Debug, Clone, Default)]
struct RecordCost {
    family: Option<EpochRecordKind>,
    size: u64,
    references: bool,
    /// One `step_epoch_storage_scan` sample per trial, in ms.
    read_and_park: Vec<u64>,
    /// One `install_validated_record` sample per trial, in ms.
    install: Vec<u64>,
    /// One batch of `REPETITIONS` validations per trial, in ms.
    validation_batch: Vec<u64>,
}

impl RecordCost {
    fn trials(&self) -> usize {
        self.read_and_park.len()
    }
    /// Every phase vector holds one sample per trial. A spread over vectors of different lengths
    /// would compare different trial sets, so the checker enforces this rather than assuming it.
    fn vectors_aligned(&self) -> bool {
        let n = self.read_and_park.len();
        self.install.len() == n && self.validation_batch.len() == n
    }
    /// A spread of **batch means**: each sample is the time for `REPETITIONS` validations divided
    /// by `REPETITIONS`, so this is the distribution of eight averages rather than of 512
    /// individual timings. It cannot show a single slow validation hidden inside an ordinary
    /// batch, which is what a conservative worst-case classifier would eventually want.
    fn validation_batch_mean(&self) -> Spread {
        Spread::of(&self.validation_batch, REPETITIONS as u128)
    }
    fn read_and_park(&self) -> Spread {
        Spread::of(&self.read_and_park, 1)
    }
    fn install(&self) -> Spread {
        Spread::of(&self.install, 1)
    }
    /// The share of the two *measured* per-record components that `validation_fits` is in a
    /// position to move, taken from the medians. Not a share of total scan custody, and not a
    /// speedup.
    ///
    /// Reported only when both components resolved; a ratio built on a phase that never rose
    /// above the clock's resolution says nothing.
    fn deferrable_fraction(&self) -> Option<u128> {
        let validation = self.validation_batch_mean();
        let retained = self.read_and_park();
        // Both phases must be resolved to better than a single clock tick. Two weaker rules were
        // tried and both produced confident-looking nonsense: checking the raw *sum* let seven
        // zeros plus one 1 ms sample through and printed 100% deferrable, and rejecting only a
        // zero median let a one-tick median through, which is a value known to within 100% of
        // itself. "Nonzero" and "good enough to divide by" are different predicates.
        if !validation.resolved_for_ratio() || !retained.resolved_for_ratio() {
            return None;
        }
        let (v, r) = (validation.upper_median_us, retained.upper_median_us);
        Some(v * 100 / (v + r))
    }
}

/// Whether the validation cache is allowed to carry across trials.
///
/// Only Registry and Studio are cacheable, and only in accounting mode (`cacheable` requires
/// `references.is_none()`). The distinction is load-bearing for them and irrelevant for the
/// rest, so it is an explicit parameter rather than a property of the fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CachePolicy {
    /// Clear between trials. Every trial performs a fresh validation, so `TRIALS` batches
    /// actually happen and the mean divides by the right count.
    Fresh,
    /// Leave it warm. After the first trial the record is a cache hit - and a cache hit is
    /// *never parked*, by design, because the expensive thing is exactly what the cache
    /// avoided. So this measures the hit path, and contributes no validation samples at all.
    Warm,
}

/// Whole-scan figures, kept per trial rather than averaged.
#[derive(Debug, Default)]
struct ScanCost {
    trials: usize,
    visits: usize,
    finish_ms: u64,
    begin_ms: u64,
    /// One entry per trial: every `step_epoch_storage_scan` sample in that trial summed,
    /// parking or not. This is what compares a warm-cache scan against a fresh one, since a
    /// warm scan parks nothing and so has no per-record rows.
    step_total: Vec<u64>,
    /// One entry per trial: `reused_records` from that trial's final progress - validation-cache
    /// hits, which are never parked and so never contribute a validation sample.
    ///
    /// Per trial rather than a single scalar, because the first warm-mode trial is not like the
    /// rest: it populates the cache and therefore still parks. Keeping only the last trial's
    /// count could not distinguish "hit on every trial" from "hit on all but the first", and
    /// those are different experiments.
    reused: Vec<usize>,
    /// One entry per trial: how many records parked in that trial.
    parked: Vec<usize>,
    records: Vec<RecordCost>,
}

impl ScanCost {
    fn record_mut(
        &mut self,
        family: EpochRecordKind,
        size: u64,
        references: bool,
    ) -> &mut RecordCost {
        if let Some(index) = self
            .records
            .iter()
            .position(|r| r.family == Some(family) && r.size == size && r.references == references)
        {
            return &mut self.records[index];
        }
        self.records.push(RecordCost {
            family: Some(family),
            size,
            references,
            ..RecordCost::default()
        });
        self.records.last_mut().expect("just pushed")
    }
    fn largest(&self, family: EpochRecordKind) -> Option<&RecordCost> {
        self.records
            .iter()
            .filter(|r| r.family == Some(family))
            .max_by_key(|r| r.size)
    }
    fn step_total(&self) -> Spread {
        Spread::of(&self.step_total, 1)
    }
}

/// What a fixture actually turned out to be, as opposed to what was asked for.
///
/// This exists because a requested operation count is not a measured one. The Studio builder
/// stops early when the epoch is nearly full (`if bytes >= MAX_EPOCH_BYTES - 64 * 1024 { break }`)
/// and returns however many operations it managed, with no requirement that the count match.
/// At 160 KiB per message the 4 MiB epoch allows about 25, so a request for 32 silently produced
/// fewer, and a per-operation figure computed from the *request* divided by the wrong number.
/// Every axis value here is now read back from the fixture and checked.
#[derive(Debug, Clone, Default)]
struct FixtureShape {
    requested_ops: Option<usize>,
    /// Observed: the operations the builder actually created and the store accepted.
    actual_ops: Option<usize>,
    physical_bytes: Option<u64>,
    /// Observed count of distinct CIDs the fixture plants. `Some(0)` is meaningful and different
    /// from `None`: it says a reference-mode case has nothing to collect, which is exactly the
    /// state the title-only Studio source was silently measured in.
    cids: Option<usize>,
}

impl std::fmt::Display for FixtureShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let show = |v: Option<usize>| v.map_or_else(|| "na".to_string(), |v| v.to_string());
        write!(
            f,
            "requested_ops={} actual_ops={} physical_bytes={} cids={}",
            show(self.requested_ops),
            show(self.actual_ops),
            self.physical_bytes
                .map_or_else(|| "na".to_string(), |v| v.to_string()),
            show(self.cids),
        )
    }
}

/// The exact reference result a timed scan must produce.
///
/// Two shapes, because two kinds of case need checking. A fixture that plants CIDs names its group
/// and its set. A family that collects none - Registry, and the title-only Studio sources - has no
/// group to name, and the right expectation is that **nothing at all** was collected, which
/// `CreativeReferences::is_empty` can state directly. Recording the second as an empty set against
/// some arbitrary group would check nothing.
#[derive(Debug, Clone)]
struct ExpectedRefs {
    /// `None` means no group should hold any reference.
    group: Option<Vec<u8>>,
    cids: std::collections::BTreeSet<Cid>,
}

impl ExpectedRefs {
    fn none() -> Self {
        Self {
            group: None,
            cids: std::collections::BTreeSet::new(),
        }
    }
    fn group(group: &[u8], cids: std::collections::BTreeSet<Cid>) -> Self {
        Self {
            group: Some(group.to_vec()),
            cids,
        }
    }
}

/// One fixture plus the scan mode to profile it in, carrying its own store.
///
/// Cases exist so trials can be **interleaved**. Running every trial of case A and then every
/// trial of case B makes a comparison between them a comparison of two different moments: any
/// drift over the run - thermal, allocator, whatever else the machine is doing - lands entirely
/// on one side. Round-robin spreads it across all of them, which is the only reason a
/// difference between two cases in the same profile means anything.
///
/// The `TempDir` is held here because dropping it deletes the vault the store is reading.
struct Case {
    label: String,
    _root: tempfile::TempDir,
    store: ServerStore,
    coverage: EpochInventoryCoverage,
    references: bool,
    cache: CachePolicy,
    shape: FixtureShape,
    /// What a reference-mode case must actually collect, checked on every trial. Required for any
    /// case with `references` set: see the oracle in [`run_trial`].
    expected_refs: Option<ExpectedRefs>,
    cost: ScanCost,
}

/// One complete budgeted scan of one case, timing every phase.
///
/// The budget is deliberately enormous: the point is not to observe the deadline firing, it is
/// to put the cursor in the mode where every record parks, so the validation phase can be timed
/// on its own.
fn run_trial(case: &mut Case, clock: &dyn catcoms_rt::Clock) {
    let Case {
        store,
        coverage,
        references,
        cache,
        expected_refs,
        cost,
        ..
    } = case;
    let (coverage, references) = (*coverage, *references);
    let expected_refs = expected_refs.as_ref();
    if *cache == CachePolicy::Fresh {
        store.inventory_cache.clear_for_test();
    }
    cost.trials += 1;
    let t = clock.monotonic_ms();
    let mut cursor = store.begin_epoch_storage_scan(coverage).unwrap();
    if references {
        store
            .collect_cursor_creative_references(&mut cursor)
            .unwrap();
    }
    cost.begin_ms += clock.monotonic_ms().saturating_sub(t);
    let mut step_total = 0;
    let mut parked_this_trial = 0;
    let mut last;
    loop {
        let t = clock.monotonic_ms();
        let progress = store
            .step_epoch_storage_scan(&mut cursor, ENTRIES_PER_STEP, Some((clock, u64::MAX)))
            .unwrap();
        let read_and_park_ms = clock.monotonic_ms().saturating_sub(t);
        cost.visits += 1;
        step_total += read_and_park_ms;
        last = progress;
        if let Some(parked) = store.take_parked_record(&mut cursor) {
            parked_this_trial += 1;
            let (family, size, refs) = parked.classification();
            let t = clock.monotonic_ms();
            for _ in 0..REPETITIONS {
                // Pinned on the **input** side. `revalidate` returns `Result<(), AppError>`, so
                // black-boxing its result pins a unit value and buys nothing - which is what the
                // first version of this did, while claiming to prevent elision. Pinning the
                // receiver is what stops the call being hoisted out of the loop or
                // common-subexpressioned across iterations.
                std::hint::black_box(&parked).revalidate().unwrap();
            }
            let validation_batch_ms = clock.monotonic_ms().saturating_sub(t);
            let validated = parked.validate().unwrap();
            let t = clock.monotonic_ms();
            store
                .install_validated_record(&mut cursor, validated)
                .unwrap();
            let install_ms = clock.monotonic_ms().saturating_sub(t);

            let entry = cost.record_mut(family, size, refs);
            // A parked record ends its visit, so the step just timed read exactly this one.
            entry.read_and_park.push(read_and_park_ms);
            entry.validation_batch.push(validation_batch_ms);
            entry.install.push(install_ms);
            continue;
        }
        if progress.complete {
            break;
        }
    }
    cost.step_total.push(step_total);
    cost.reused.push(last.reused_records);
    cost.parked.push(parked_this_trial);
    let t = clock.monotonic_ms();
    let collected = if references {
        Some(store.finish_cursor_creative_references(cursor).unwrap())
    } else {
        store.finish_epoch_storage_scan(cursor).unwrap();
        None
    };
    cost.finish_ms += clock.monotonic_ms().saturating_sub(t);

    // The reference-result oracle, deliberately **after** the timer stops so checking it cannot
    // be charged to the phase it is checking.
    //
    // Without this the profile timed a reference scan and threw its result away, asserting only
    // that the *fixture* contained N CIDs - never that the scan returned them. A Studio collector
    // regression that completed successfully with an empty set would have satisfied every
    // structural check and produced a fast, meaningless number. "The fixture contains 128
    // references" and "the profiled scan returned those 128 references" are different claims, and
    // only the second makes the timing worth anything.
    if let (Some(collected), Some(expected)) = (collected, expected_refs) {
        match &expected.group {
            Some(group) => {
                let got: std::collections::BTreeSet<_> =
                    collected.for_group(group).copied().collect();
                assert_eq!(
                    &got,
                    &expected.cids,
                    "the profiled Studio reference result differs from the fixture's expected \
                     set: collected {} of {} expected CIDs for this group",
                    got.len(),
                    expected.cids.len(),
                );
                // And nothing beyond this group, so a collector attributing references to the
                // wrong document cannot pass by coincidence.
                assert_eq!(
                    collected.len(),
                    expected.cids.len(),
                    "the profiled reference result holds {} references in total against {} \
                     expected for the fixture's only group",
                    collected.len(),
                    expected.cids.len(),
                );
            }
            None => assert!(
                collected.is_empty(),
                "a family that collects no references returned {} of them",
                collected.len()
            ),
        }
    }
}

/// How a run schedules its trials across cases.
///
/// Both exist so the two can be **compared on one fixed corpus**, which is the only way to say
/// what the scheduling itself is worth. Earlier work observed a 2.5x shift when moving from
/// blocked to interleaved and attributed it to ordering; that was not established, because the
/// change that produced it altered store construction, the summary statistic and the
/// fixture-to-measurement delay at the same time. Holding everything else fixed and varying only
/// this is what isolates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Protocol {
    /// Every trial of case A, then every trial of case B. Any drift over the run lands entirely
    /// on whichever cases ran late.
    Blocked,
    /// One trial of every case, then the second trial of every case, each round in a fresh
    /// **seeded permutation** so drift is spread across cases and no case keeps a fixed
    /// predecessor.
    Interleaved,
}

/// Seed for the interleaved rounds' permutations, printed with every result so an order can be
/// reproduced exactly.
const ORDER_SEED: u64 = 0x1307_2026;

impl Protocol {
    fn label(self) -> &'static str {
        match self {
            Protocol::Blocked => "blocked",
            Protocol::Interleaved => "interleaved",
        }
    }
}

/// The interleaved schedule: one seeded permutation of `0..n` per trial.
///
/// **This is the single source of the order.** `run_scheduled` consumes it and so does the test
/// that checks its predecessor property - which matters, because the first version of that test
/// reimplemented the Fisher-Yates loop itself. Reverting the dispatcher to the broken cyclic
/// rotation would have left the test shuffling its own private copy and passing. A regression
/// test for a scheduler has to observe the schedule the scheduler actually uses.
fn interleaved_rounds(n: usize) -> Vec<Vec<usize>> {
    if n == 0 {
        return Vec::new();
    }
    let mut rng = ChaCha20Rng::seed_from_u64(ORDER_SEED);
    (0..TRIALS)
        .map(|_| {
            let mut order: Vec<usize> = (0..n).collect();
            for i in (1..n).rev() {
                let j = (rng.next_u32() as usize) % (i + 1);
                order.swap(i, j);
            }
            order
        })
        .collect()
}

/// Run `TRIALS` trials of every case under `protocol`.
///
/// Under `Interleaved` each round runs the cases in a **fresh seeded permutation**.
///
/// Rotating the start index was tried first and does nothing: `(round + offset) % n` emits
/// `c[r], c[r+1], … c[r+n-1]`, which preserves the cyclic order, so every case except the round's
/// first still follows exactly the same predecessor it did before. At the profile's real shape
/// that left most cases with a single predecessor across all eight rounds - which is the very
/// thing interleaving is supposed to stop mattering. A permutation is what actually varies it.
fn run_scheduled(cases: &mut [Case], protocol: Protocol, clock: &dyn catcoms_rt::Clock) {
    match protocol {
        Protocol::Blocked => {
            for case in cases.iter_mut() {
                for _ in 0..TRIALS {
                    run_trial(case, clock);
                }
            }
        }
        Protocol::Interleaved => {
            for round in interleaved_rounds(cases.len()) {
                for index in round {
                    run_trial(&mut cases[index], clock);
                }
            }
        }
    }
}

/// Run `TRIALS` trials of every case, **round-robin rather than case by case**.
///
/// See [`Case`] for why the ordering is the point. This is the whole of the repetition
/// discipline: it does not make any single figure more accurate, it makes differences *between*
/// cases in one profile comparable, which the block-ordered version could not claim.
fn run_interleaved(cases: &mut [Case], clock: &dyn catcoms_rt::Clock) {
    run_scheduled(cases, Protocol::Interleaved, clock);
}

/// Build a case around a store that has already been populated.
///
/// Every parameter is one axis of the fixture matrix these benchmarks sweep, so collapsing them
/// into a struct would only move the same eight values one line up at each call site.
///
/// **Every case's store detaches every validation.** The harness times `validate()` on each
/// parked record, and with the calibrated classifier a small uncached record would validate
/// inline and contribute no sample. The switch keeps every record in the measurement, whatever
/// the classifier's constants are (C-3 runtime design 14.6).
#[allow(clippy::too_many_arguments)]
fn case(
    label: impl Into<String>,
    root: tempfile::TempDir,
    mut store: ServerStore,
    coverage: EpochInventoryCoverage,
    references: bool,
    cache: CachePolicy,
    shape: FixtureShape,
    expected_refs: Option<ExpectedRefs>,
) -> Case {
    store.detach_every_validation_for_test();
    Case {
        label: label.into(),
        _root: root,
        store,
        coverage,
        references,
        cache,
        shape,
        expected_refs,
        cost: ScanCost::default(),
    }
}

/// Convenience for the frozen-clock smoke tests, which profile one case and assert structure.
#[allow(clippy::too_many_arguments)]
fn profile_scan(
    root: tempfile::TempDir,
    store: ServerStore,
    coverage: EpochInventoryCoverage,
    references: bool,
    cache: CachePolicy,
    expected_refs: Option<ExpectedRefs>,
    clock: &dyn catcoms_rt::Clock,
) -> Case {
    let cids = expected_refs.as_ref().map(|e| e.cids.len());
    let mut one = [case(
        "smoke",
        root,
        store,
        coverage,
        references,
        cache,
        // Smoke fixtures have no operation axis. The CID count comes from the expected set, so the
        // two cannot disagree.
        FixtureShape {
            cids,
            ..FixtureShape::default()
        },
        expected_refs,
    )];
    run_interleaved(&mut one, clock);
    one.into_iter().next().expect("one case")
}

/// Stage a Recovery record whose authenticated body is at least `projection` bytes of opaque
/// filler.
///
/// **This is the accounting-only control.** `EpochRecoveryState::decode` and `footprint` treat
/// the projection as bytes, so it measures the accounting validator honestly - but reference
/// collection would refuse it, because the inspector decodes and validates every operation in
/// the projection. Kept alongside [`stage_canonical`] rather than replaced by it, so the
/// original measurements stay comparable.
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

/// Stage a Recovery record built the way production builds one, so it can be reference-collected.
///
/// A real `StudioEpoch` carrying `frames` inserted frames, its canonical projection taken through
/// `StudioRecovery::snapshot`. The inspector decodes and validates every operation in that
/// projection and returns the CIDs the frames name, so this measures the reference validator
/// doing real work rather than refusing or returning an empty set.
///
/// Returns the CIDs it planted, so the caller can assert the collected set rather than only time
/// it: a fast empty result is not evidence. The returned CIDs are **distinct**, asserted here,
/// so `frames` is also the reference count and the reference-count axis means what it says.
fn stage_canonical(
    store: &mut ServerStore,
    server: u64,
    group: &catcoms_mls::ServerGroup,
    device: &catcoms_mls::MlsDevice,
    target: StudioTarget,
    frames: usize,
) -> Vec<Cid> {
    let logical = target.document(&group.group_id()).unwrap();
    let mut blobs = store.blob_store(&hex::encode(group.group_id())).unwrap();
    let mut planted = Vec::new();
    let mut unit = StudioEpoch::new(group, target, device.device_id()).unwrap();
    for n in 0..frames {
        // Distinct content per frame, so distinct CIDs. An earlier version wrote
        // `[(n % 251) as u8; 10]`, which silently collapsed to 251 distinct blobs: at 512 frames
        // the fixture planted 512 frames but only 251 references, and the set comparison still
        // passed because both sides are sets. A reference-count axis built on that would have
        // been fiction. `frames_are_distinct` below is what stops it recurring.
        let cid = blobs.put(&(n as u64).to_be_bytes()).unwrap();
        planted.push(cid);
        let mut nonce = [0; 16];
        nonce[..8].copy_from_slice(&(n as u64).to_be_bytes());
        let mut frame = [0; 16];
        frame[..8].copy_from_slice(&(n as u64).to_be_bytes());
        let operation = DomainOp {
            nonce,
            doc_type: logical.doc_type,
            logical_key: logical.logical_key.clone(),
            body: FlipnoteOp::InsertFrame {
                frame,
                after: None,
                cid: *cid.as_bytes(),
                bytes: 10,
            }
            .encode()
            .unwrap(),
        };
        unit.edit_or_reseal(
            device,
            group,
            &mut ChaCha20Rng::seed_from_u64(31),
            &operation,
            100,
        )
        .unwrap();
    }
    drop(blobs);
    let distinct: std::collections::BTreeSet<_> = planted.iter().copied().collect();
    assert_eq!(
        distinct.len(),
        frames,
        "the fixture planted {frames} frames but only {} distinct CIDs, so its reference-count \
         axis is fiction and the set comparison would still pass",
        distinct.len()
    );
    let snapshot = StudioRecovery::snapshot(
        &unit.projection().unwrap(),
        None,
        RecoveryReason::Rewound,
        [0; 32],
        &BTreeMap::new(),
    )
    .unwrap();
    store
        .update_epoch_recovery(
            server,
            &logical,
            EpochRecoveryAction::Stage(snapshot),
            &ManualClock::new(100),
            &mut ChaCha20Rng::seed_from_u64(31),
        )
        .unwrap();
    planted
}

/// Print one scan's results with every condition that qualifies them.
/// Every timing is printed as `min/median/max` in microseconds, never as a mean.
///
/// The spread is the part that says whether a difference between two rows is worth reading. A
/// mean concealed exactly that in the first profiles of this design.
fn report(case: &Case, order: &str, profile: &str) {
    let label = &case.label;
    let cost = &case.cost;
    let mut records = cost.records.clone();
    records.sort_by_key(|r| r.size);
    for r in &records {
        println!(
            "C3_PROFILE scan={label} family={} bytes={} references={} \
             read_and_park_us={} validation_batch_mean_us={} install_us={} \
             validation_batch_ms_total={} trials={} deferrable_fraction_of_measured={}",
            r.family
                .map_or_else(|| "none".to_string(), |f| format!("{f:?}")),
            r.size,
            r.references,
            r.read_and_park(),
            r.validation_batch_mean(),
            r.install(),
            r.validation_batch_mean().raw_total_ms,
            r.trials(),
            r.deferrable_fraction()
                .map_or_else(|| "unresolved".to_string(), |v| v.to_string()),
        );
    }
    println!(
        "C3_PROFILE scan={label} {} trials={} visits={} records={} begin_ms={} finish_ms={} \
         step_total_us={} cache_hits_per_trial={:?} parked_per_trial={:?} repetitions={} \
         order={order} order_seed={ORDER_SEED:#x} build={profile} \
         page_cache=warm_written_immediately_before \
         units=min/upper_median/max_us_and_zero_sample_count_and_raw_upper_median_ms",
        case.shape,
        cost.trials,
        cost.visits,
        cost.records.len(),
        cost.begin_ms,
        cost.finish_ms,
        cost.step_total(),
        cost.reused,
        cost.parked,
        REPETITIONS,
    );
}

/// The accounting-only control: opaque projections across a size range, no reference collection.
fn recovery_accounting_case(sizes: &[usize]) -> Case {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    for (n, size) in sizes.iter().enumerate() {
        let key = format!("recovery-{n}");
        stage_sized(&mut store, 7, &document(b"group", key.as_bytes()), *size);
    }
    // Recovery is not a cacheable family, so the policy is immaterial here; `Fresh` states the
    // intent rather than relying on that.
    case(
        "recovery_accounting",
        root,
        store,
        EpochInventoryCoverage::RecoveryOnly,
        false,
        CachePolicy::Fresh,
        // The axis here is bytes of opaque projection, with no operation structure at all - which
        // is exactly why it cannot settle whether cost follows bytes or operations.
        FixtureShape::default(),
        None,
    )
}

/// The canonical case: real projections that reference collection actually inspects.
///
/// Times **and checks** the result: a reference scan that returned an empty or short CID set
/// quickly would otherwise look like a cheap one.
/// One case per reference count.
///
/// The collected set is **verified here, before any timing**, because the store is moved into
/// the case afterwards. That ordering is better anyway: a fixture that collects the wrong CIDs
/// should fail before it contributes a single sample, not after.
fn recovery_reference_cases(frames: &[usize], clock: &dyn catcoms_rt::Clock) -> Vec<Case> {
    frames
        .iter()
        .map(|count| {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let device = catcoms_mls::MlsDevice::generate().unwrap();
            let group = catcoms_mls::ServerGroup::create(&device).unwrap();
            let target = StudioTarget::Flipnote {
                channel: [7; 16],
                object: [9; 16],
            };
            let planted = stage_canonical(&mut store, 7, &group, &device, target, *count);

            // Reference collection is contractually full-coverage: `collect_creative_references`
            // refuses anything narrower, because a partial inventory cannot be allowed to
            // replace a transient pre-publication hold.
            let mut cursor = store.begin_epoch_storage_scan(REFERENCE_COVERAGE).unwrap();
            store
                .collect_cursor_creative_references(&mut cursor)
                .unwrap();
            loop {
                let progress = store
                    .step_epoch_storage_scan(&mut cursor, ENTRIES_PER_STEP, Some((clock, u64::MAX)))
                    .unwrap();
                if let Some(parked) = store.take_parked_record(&mut cursor) {
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
            let collected = store.finish_cursor_creative_references(cursor).unwrap();
            let got: std::collections::BTreeSet<_> =
                collected.for_group(&group.group_id()).copied().collect();
            let want: std::collections::BTreeSet<_> = planted.iter().copied().collect();
            assert_eq!(
                got, want,
                "the reference scan did not collect the frames' CIDs, so its timing would be the \
                 timing of an incomplete result"
            );
            assert_eq!(
                got.len(),
                *count,
                "the reference count axis is wrong: {count} frames collected {} CIDs",
                got.len()
            );
            // The verification scan installed protection; reset it so the timed scans install
            // their own rather than being refused as duplicates.
            store.creative_protection.lock().unwrap().unknown_for_test();
            case(
                format!("recovery_references_frames{count}"),
                root,
                store,
                REFERENCE_COVERAGE,
                true,
                CachePolicy::Warm,
                FixtureShape {
                    // Frame count, CID count and projection bytes all rise together here, so this
                    // axis cannot separate which of them drives the cost.
                    requested_ops: Some(*count),
                    actual_ops: Some(*count),
                    physical_bytes: None,
                    cids: Some(got.len()),
                },
                // The oracle: every timed trial must return exactly this set, not merely finish.
                Some(ExpectedRefs::group(&group.group_id(), got)),
            )
        })
        .collect()
}

/// Registry: one of the two families whose expensive typed reconstruction motivated C-3.
/// One vault holding a record of **five of the six** scanned families, measured in a single scan.
///
/// The sixth is DraftArchive, which this does not write: `REFERENCE_COVERAGE` would scan it, so
/// "every family" would be wrong.
///
/// A fixture for two outstanding 13.7 items - it is not itself the measurement, and no
/// OwnerReceipts or Intents figures are recorded until the profile is run and read. It supplies
/// records of those two families, which had none at all; and a **realistic scan shape** - visits,
/// per-visit custody and a per-family breakdown over a vault holding more than one family. Every
/// earlier case held a single family (though not always a single record), so the visits figure
/// described the fixture rather than a scan.
///
/// Built through the production writers - `prepare_epoch_owner_receipt`, `prepare_epoch_intent`,
/// `update_epoch_recovery` and the registry and studio fixtures - rather than by widening other
/// modules' test helpers, which would have meant making their whole test modules reachable.
fn multi_family_case(cache: CachePolicy, references: bool) -> Case {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let device = catcoms_mls::MlsDevice::generate().unwrap();
    let group = catcoms_mls::ServerGroup::create(&device).unwrap();
    let target = StudioTarget::Flipnote {
        channel: [7; 16],
        object: [9; 16],
    };
    let studio_doc = target.document(&group.group_id()).unwrap();

    // Studio, and with it a Registry record for the same vault.
    crate::store::epoch_studio::tests::performance::save_studio_source_fixture_ops(
        &mut store, 7, &group, &device, target, 3, 160_000,
    );
    crate::store::epoch_registry::tests::performance::save_inventory_fixture_ops(&mut store, 2);

    // Recovery, through `stage_canonical` rather than `stage_sized`.
    //
    // This vault is scanned in reference mode as well as accounting mode, and `recovery_cids`
    // runs `inspect_vault_references` for a Studio-typed document - which needs a canonically
    // valid projection and refuses opaque filler. `stage_sized` plants filler, which is stated
    // in its own doc comment, and using it here made the reference-mode case fail with "creative
    // reference scan incomplete, unsupported or over bound". A caveat recorded on a helper is no
    // use if the next fixture ignores it.
    let planted = stage_canonical(&mut store, 7, &group, &device, target, 8);

    // OwnerReceipts, through the production writer.
    let receipt = catcoms_replication::Receipt::sign(
        studio_doc.clone(),
        0,
        [11; 32],
        [12; 32],
        group.epoch(),
        catcoms_replication::InheritedCheckpoint::EpochZero,
        &device,
    )
    .unwrap();
    let mut owner_budget = family_budget(&mut store, 7, &studio_doc);
    store
        .prepare_epoch_owner_receipt(
            7,
            receipt,
            &group,
            group.epoch(),
            &mut ChaCha20Rng::seed_from_u64(5),
            &mut owner_budget,
        )
        .expect("owner receipt");

    // Intents, likewise.
    let mut storage_budget = family_budget(&mut store, 7, &studio_doc);
    let mut intent_budget = crate::store::epoch_intents::EpochIntentBudget::from_inventory(
        &collect_with(&mut store, REFERENCE_COVERAGE),
    )
    .expect("intent budget");
    store
        .prepare_epoch_intent(
            7,
            &studio_doc,
            DomainOp {
                nonce: [3; 16],
                doc_type: studio_doc.doc_type,
                logical_key: studio_doc.logical_key.clone(),
                body: FlipnoteOp::SetHeader(catcoms_replication::studio::FlipnoteHeader::Title(
                    "multi family".into(),
                ))
                .encode()
                .unwrap(),
            },
            &device,
            &group,
            &mut ChaCha20Rng::seed_from_u64(6),
            &mut storage_budget,
            &mut intent_budget,
        )
        .expect("intent");

    // What the scan actually found, so the case is labelled by observation.
    let inventory = collect_with(&mut store, REFERENCE_COVERAGE);
    let families: std::collections::BTreeSet<_> =
        inventory.records().map(|entry| entry.kind).collect();
    // The exact set, not a count. `>= 5` is `== 5` in disguise when only five are written, and
    // it would not say *which* five.
    assert_eq!(
        families,
        [
            EpochRecordKind::Recovery,
            EpochRecordKind::OwnerReceipts,
            EpochRecordKind::Intents,
            EpochRecordKind::Registry,
            EpochRecordKind::Studio,
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>(),
        "the multi-family vault does not hold the five families it claims"
    );

    case(
        format!(
            "multi_family_{}_{}",
            if references {
                "references"
            } else {
                "accounting"
            },
            if cache == CachePolicy::Fresh {
                "fresh"
            } else {
                "warm"
            }
        ),
        root,
        store,
        REFERENCE_COVERAGE,
        references,
        cache,
        FixtureShape {
            cids: references.then_some(planted.len()),
            ..FixtureShape::default()
        },
        // The canonical Recovery record's pixels are the only references in this vault: the
        // Studio source is title-only and the Registry arm collects none. So the expectation is
        // that exact set - checked on every trial like any other reference case, rather than
        // waved through because this fixture's purpose is scan shape.
        references
            .then(|| ExpectedRefs::group(&group.group_id(), planted.iter().copied().collect())),
    )
}

/// An `EpochStorageBudget` for one document, from a fresh full-coverage inventory.
fn family_budget(
    store: &mut ServerStore,
    server: u64,
    doc: &LogicalDocument,
) -> crate::store::epoch_budget::EpochStorageBudget {
    let inventory = collect_with(store, REFERENCE_COVERAGE);
    crate::store::epoch_budget::EpochStorageBudget::from_inventory(
        StorageScope::new(server, &doc.server_id).unwrap(),
        inventory
            .records_for_server(server, &doc.server_id)
            .unwrap(),
    )
    .expect("storage budget")
}

/// The three modes every cacheable family needs, as separate cases over separate fixtures.
///
/// A `Case` owns its store, so each mode gets its own build of the same fixture shape. That is
/// deliberate rather than merely necessary: the three modes are now interleaved with each other
/// *and* with every other size and family, so comparing them is not comparing three consecutive
/// moments in a long run.
const MODES: [(&str, bool, CachePolicy); 3] = [
    ("accounting_fresh", false, CachePolicy::Fresh),
    ("accounting_warm", false, CachePolicy::Warm),
    ("references", true, CachePolicy::Warm),
];

/// Registry: one of the two families whose expensive typed reconstruction motivated C-3.
///
/// The axis is **operation count**, not bytes: `save_inventory_fixture_ops` builds a real signed
/// registry log of `ops` operations at 160 KiB per message, and the reconstruction walks them.
/// A byte axis alone would not distinguish a large record from a structurally deep one.
fn registry_cases(op_counts: &[usize]) -> Vec<Case> {
    let mut cases = Vec::new();
    for ops in op_counts {
        for (mode, references, cache) in MODES {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let path = crate::store::epoch_registry::tests::performance::save_inventory_fixture_ops(
                &mut store, *ops,
            );
            let bytes = fs::metadata(&path).unwrap().len();
            cases.push(case(
                format!("registry_{mode}_ops{ops}"),
                root,
                store,
                REFERENCE_COVERAGE,
                references,
                cache,
                FixtureShape {
                    requested_ops: Some(*ops),
                    // `Source::fill` ingests `count + 1` and asserts that any acceptance has
                    // `n < count`, so it panics unless exactly `count` were accepted and the
                    // last refused. The count is guaranteed by that builder rather than read
                    // back here - unlike Studio's, which silently truncates.
                    actual_ops: Some(*ops),
                    physical_bytes: Some(bytes),
                    // The Registry validator arm collects no CIDs in either mode. Checked, not
                    // assumed: the oracle requires the scan to return nothing at all.
                    cids: Some(0),
                },
                references.then(ExpectedRefs::none),
            ));
        }
    }
    cases
}

/// Studio: the other family C-3 was designed for. Same three modes, same reasoning.
/// Studio: the other family C-3 was designed for. Same three modes, same reasoning.
///
/// **The operation count is read back, not assumed.** `build` stops early once the epoch is
/// nearly full, and at 160 KiB per message the 4 MiB epoch fits about 25 operations, so a
/// request for more silently produces fewer. `check_case_structure` fails the case if the
/// builder did not deliver what was asked for, which is why the requested counts here are
/// chosen to fit rather than to look round.
///
/// **These sources carry title headers, not frames.** `title_op` emits
/// `FlipnoteOp::SetHeader(Title(..))`, which names no CID, so the reference mode of these cases
/// collects an **empty** set. That is recorded as `cids=0` rather than left implicit: the timing
/// is the timing of reference collection over a source with nothing to collect, and says nothing
/// about a source that has frames. [`studio_frame_cases`] is the one that does.
fn studio_cases(op_counts: &[usize]) -> Vec<Case> {
    let mut cases = Vec::new();
    for ops in op_counts {
        for (mode, references, cache) in MODES {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let device = catcoms_mls::MlsDevice::generate().unwrap();
            let group = catcoms_mls::ServerGroup::create(&device).unwrap();
            let target = StudioTarget::Flipnote {
                channel: [7; 16],
                object: [9; 16],
            };
            let operations =
                crate::store::epoch_studio::tests::performance::save_studio_source_fixture_ops(
                    &mut store, 7, &group, &device, target, *ops, 160_000,
                );
            // Observed two ways: what the builder returned, and what the persisted source says
            // it accepted. They must agree, or the fixture is not what either number claims.
            let persisted = store
                .load_studio_epoch(7, &group, target, &device)
                .unwrap()
                .expect("the fixture wrote a Studio source")
                .op_count();
            assert_eq!(
                operations.len(),
                persisted,
                "the builder returned {} operations but the persisted source holds {persisted}",
                operations.len()
            );
            cases.push(case(
                format!("studio_titles_{mode}_ops{ops}"),
                root,
                store,
                REFERENCE_COVERAGE,
                references,
                cache,
                FixtureShape {
                    requested_ops: Some(*ops),
                    actual_ops: Some(persisted),
                    physical_bytes: None,
                    // Title headers name no CIDs. Stated, not implied - and the oracle requires
                    // the scan to actually return none, so "these sources have nothing to
                    // collect" is an observation rather than an assumption about `title_op`.
                    cids: Some(0),
                },
                references.then(ExpectedRefs::none),
            ));
        }
    }
    cases
}

/// A Studio source whose operations actually name pixels, so reference mode has work to do.
///
/// The title-only cases above cannot support any claim about the cost of Studio reference
/// collection, because their CID set is empty. This builds frames through the same
/// `edit_or_reseal` path, each naming a distinct blob, and records the count.
fn studio_frame_cases(frame_counts: &[usize]) -> Vec<Case> {
    studio_frame_factorial(&frame_counts.iter().map(|n| (*n, *n)).collect::<Vec<_>>())
}

/// The distinct-reference axis, decoupled from frame count.
///
/// **What this does decouple:** `(128, 1)` against `(128, 128)` holds frame count *and* encoded
/// size fixed - the record carries the same 128 signed operations either way, each naming one
/// 32-byte CID - and varies only how many of those references are distinct.
///
/// **What it does not:** `(16, 1)` against `(128, 1)` holds the reference count fixed but frames
/// and bytes still move together, because eight times the operations is eight times the signed
/// history. So frame count is still confounded with encoded size, exactly as the earlier Studio
/// and Recovery axes were. Separating those needs a payload axis at fixed frame count, which
/// `build`'s `message` padding could supply and this does not use. An earlier version of this
/// comment claimed the factorial settled which of the three drives cost; it settles one of the
/// three.
///
/// **A prediction worth recording, so a null result is read correctly:** `blob_cids()` parses
/// every signed operation regardless of how many distinct CIDs result, so the *validation* figure
/// should barely move between `(128, 1)` and `(128, 128)`. If the reference count costs anything,
/// it should appear in `install` and `finish`, where the set is merged.
fn studio_frame_factorial(shapes: &[(usize, usize)]) -> Vec<Case> {
    let mut cases = Vec::new();
    for (frames, distinct) in shapes.iter().copied() {
        for (mode, references, cache) in MODES {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let device = catcoms_mls::MlsDevice::generate().unwrap();
            let group = catcoms_mls::ServerGroup::create(&device).unwrap();
            let target = StudioTarget::Flipnote {
                channel: [7; 16],
                object: [9; 16],
            };
            let (planted, accepted) =
                crate::store::epoch_studio::tests::performance::save_studio_frame_fixture(
                    &mut store, 7, &group, &device, target, frames, distinct,
                );
            let cids: std::collections::BTreeSet<_> = planted.iter().copied().collect();
            // The distinct count is now a deliberate axis rather than an accident, so it is
            // checked against what was asked for rather than against the frame count.
            assert_eq!(
                cids.len(),
                distinct,
                "the frame fixture planted {} distinct CIDs where {distinct} were requested, so \
                 its reference-count axis is fiction",
                cids.len()
            );
            cases.push(case(
                format!("studio_frames_{mode}_n{frames}_c{distinct}"),
                root,
                store,
                REFERENCE_COVERAGE,
                references,
                cache,
                FixtureShape {
                    requested_ops: Some(frames),
                    actual_ops: Some(accepted),
                    physical_bytes: None,
                    cids: Some(cids.len()),
                },
                // The oracle that was missing: the fixture containing N references and the timed
                // scan *returning* them are different claims, and only the second makes the
                // reference-mode timing mean anything.
                references.then(|| ExpectedRefs::group(&group.group_id(), cids.clone())),
            ));
        }
    }
    cases
}

/// The structural claims every reported figure rests on, checked after the run.
///
/// These are not timing assertions. They say the samples were gathered the way the report
/// claims: a cacheable family parks every trial when the cache is cleared and stops parking
/// when it is not, a reference scan never sees a cache hit, and every record that contributed
/// contributed in every trial.
fn check_case_structure(case: &Case) {
    let label = &case.label;
    let cost = &case.cost;
    assert_eq!(cost.trials, TRIALS, "{label}: a trial did not complete");

    // Sampling preconditions, enforced for *every* case rather than inferred from one smoke
    // fixture. `trials()` reads only the read-and-park vector's length, so without this a
    // validation or install vector of a different length would go unnoticed and its spread would
    // be over a different trial set.
    for r in &cost.records {
        assert!(
            r.vectors_aligned(),
            "{label}: phase vectors are ragged for a {:?} record ({} read, {} validation, {} \
             install), so their spreads are not over the same trials",
            r.family,
            r.read_and_park.len(),
            r.validation_batch.len(),
            r.install.len(),
        );
    }
    for (name, len) in [
        ("step_total", cost.step_total.len()),
        ("reused", cost.reused.len()),
        ("parked", cost.parked.len()),
    ] {
        assert_eq!(
            len, TRIALS,
            "{label}: {name} has {len} entries for {TRIALS} trials"
        );
    }

    // A fixture's axis must be observed, not requested. See `FixtureShape`.
    if let (Some(requested), Some(actual)) = (case.shape.requested_ops, case.shape.actual_ops) {
        assert_eq!(
            requested, actual,
            "{label}: asked for {requested} operations and the fixture built {actual}; a \
             per-operation figure divided by the request would be wrong. Pick a count that fits \
             the epoch, or label this case capacity-limited and divide by the actual count."
        );
    }
    // A reference-mode case with nothing to collect measures an empty merge. That is a legal
    // thing to measure and an illegal thing to report as "reference collection costs X".
    if case.references {
        assert!(
            case.shape.cids.is_some(),
            "{label}: a reference-mode case did not record how many CIDs its fixture plants, so \
             its timing cannot be distinguished from the timing of an empty collection"
        );
        // And the fixture's contents are not the scan's output. Every reference-mode case must
        // carry an expected set for `run_trial`'s oracle to check the returned one against;
        // without it a collector that returned nothing would still produce a clean profile.
        let expected = case.expected_refs.as_ref().unwrap_or_else(|| {
            panic!("{label}: a reference-mode case carries no expected CID set")
        });
        assert_eq!(
            expected.cids.len(),
            case.shape.cids.unwrap(),
            "{label}: the expected CID set and the recorded fixture CID count disagree, so one of \
             them is wrong"
        );
    }

    if case.references {
        assert!(
            cost.reused.iter().all(|n| *n == 0),
            "{label}: a reference scan reported validation-cache hits, but nothing is cacheable \
             in that mode: {:?}",
            cost.reused
        );
    }
    match case.cache {
        CachePolicy::Fresh => {
            assert!(
                cost.reused.iter().all(|n| *n == 0),
                "{label}: a cleared cache still reported hits: {:?}",
                cost.reused
            );
            assert!(
                !cost.records.is_empty(),
                "{label}: nothing parked, so no validation phase was measured"
            );
            assert!(
                cost.records.iter().all(|r| r.trials() == TRIALS),
                "{label}: a record did not park in every trial, so its spread is over the wrong \
                 sample count"
            );
        }
        CachePolicy::Warm if !case.references => {
            // The first warm trial populates the cache and therefore still parks; only the rest
            // are hits. Asserting that shape explicitly is what distinguishes "hit every trial"
            // from "hit on all but the first", which a single last-trial scalar could not.
            assert_eq!(
                cost.reused[0], 0,
                "{label}: the first warm trial reported a hit, so the cache was already warm \
                 and this case is not measuring a cold-then-warm sequence"
            );
            assert!(
                cost.reused[1..].iter().all(|n| *n > 0),
                "{label}: a warm-cache accounting scan of a cacheable family stopped reporting \
                 hits after the first trial: {:?}",
                cost.reused
            );
            assert_eq!(
                cost.parked[0], 1,
                "{label}: the first warm trial must park once to populate the cache"
            );
            assert!(
                cost.parked[1..].iter().all(|n| *n == 0),
                "{label}: a cache hit is never parked, so no trial after the first may park: \
                 {:?}",
                cost.parked
            );
        }
        CachePolicy::Warm => {
            assert!(
                cost.records.iter().all(|r| r.trials() == TRIALS),
                "{label}: a record did not park in every trial of a reference scan"
            );
        }
    }
}

/// The harness itself, on a `ManualClock` that never advances.
///
/// Asserts the structure every reported figure depends on, and deliberately asserts nothing
/// about duration: a frozen clock reports zero for all of them, and a timing assertion in the
/// ordinary suite is a machine-speed assertion in disguise.
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
    let profiled = profile_scan(
        root,
        store,
        EpochInventoryCoverage::RecoveryOnly,
        false,
        CachePolicy::Fresh,
        None,
        &clock,
    );
    let cost = &profiled.cost;

    assert_eq!(
        cost.records.len(),
        sizes.len(),
        "a record did not park, so the harness is no longer detaching everything and this \
         measurement no longer isolates the validation phase"
    );
    // Phase coverage: every parked result was installed, every trial finished, and each record
    // contributed a sample in every trial. Without these the per-phase spreads are over an
    // unknown number of occurrences and say nothing.
    check_case_structure(&profiled);
    assert!(
        cost.visits > cost.records.len() * TRIALS,
        "a parked record ends its visit, so {} records over {TRIALS} trials cannot have taken \
         only {} visits",
        cost.records.len(),
        cost.visits,
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
    // A frozen clock must not produce a fraction: every component is zero.
    assert!(
        cost.records
            .iter()
            .all(|r| r.deferrable_fraction().is_none()),
        "a ratio was reported from a clock that never advanced"
    );
    // And every retained sample set is the right length, which is what a spread divides by.
    assert!(
        cost.records.iter().all(|r| r.read_and_park.len() == TRIALS
            && r.validation_batch.len() == TRIALS
            && r.install.len() == TRIALS),
        "a phase recorded a different number of samples from the others, so the spreads are not \
         over the same trials"
    );
    assert_eq!(
        cost.step_total.len(),
        TRIALS,
        "one step total per trial, or the warm-versus-fresh comparison is over ragged samples"
    );
}

/// A phase whose median is zero must not produce a fraction, even when a stray nonzero sample
/// makes its raw total positive.
///
/// This is the same trap as the earlier `visit_ms == 0` case, one level down: `resolved()` looks
/// at the sum, so seven zeros and one 1 ms sample passes it while the median is still 0, and the
/// ratio would print 100% deferrable. A real profile hit exactly that on the small
/// reference-scan rows before it was fixed.
#[test]
fn c3_a_phase_resolved_to_one_tick_or_less_reports_no_fraction() {
    // Seven zeros and one 1 ms sample: the raw *sum* is positive, which is what made this
    // reachable under the first rule.
    let mostly_zero = RecordCost {
        family: Some(EpochRecordKind::Recovery),
        size: 1,
        references: true,
        read_and_park: vec![0, 0, 0, 0, 0, 0, 0, 1],
        install: vec![0; 8],
        validation_batch: vec![640; 8],
    };
    assert!(
        mostly_zero.read_and_park().raw_total_ms > 0,
        "control: the raw total is positive, which is what makes this trap reachable"
    );
    assert_eq!(mostly_zero.read_and_park().upper_median_us, 0);
    assert_eq!(
        mostly_zero.read_and_park().zero_samples,
        7,
        "the zero count is what tells a reader how much of the phase the clock could not see"
    );
    assert_eq!(
        mostly_zero.deferrable_fraction(),
        None,
        "a fraction was reported against a phase whose upper median is below the clock's \
         resolution"
    );

    // A phase resolved to exactly one tick is known only to within 100% of itself, so it must
    // also report nothing. The earlier rule rejected only a zero median and let this through.
    let one_tick = RecordCost {
        read_and_park: vec![1; 8],
        ..mostly_zero.clone()
    };
    assert_eq!(one_tick.read_and_park().upper_median_us, 1_000);
    assert_eq!(
        one_tick.deferrable_fraction(),
        None,
        "a fraction was reported from a phase measured as a single clock tick"
    );

    // Positive control: above one tick, a fraction appears.
    let resolved = RecordCost {
        read_and_park: vec![5; 8],
        ..mostly_zero
    };
    assert!(
        resolved.deferrable_fraction().is_some(),
        "control is broken: a phase resolved beyond one tick must still report a fraction"
    );
}

/// The canonical reference fixture, small, on a frozen clock. Proves the fixture is valid and
/// its CIDs are collected; the profiling test below is what times it.
#[test]
fn c3_canonical_reference_fixture_collects_its_planted_cids() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let device = catcoms_mls::MlsDevice::generate().unwrap();
    let group = catcoms_mls::ServerGroup::create(&device).unwrap();
    let target = StudioTarget::Flipnote {
        channel: [7; 16],
        object: [9; 16],
    };
    let planted = stage_canonical(&mut store, 7, &group, &device, target, 4);
    assert_eq!(planted.len(), 4, "the fixture planted no CIDs to find");

    let clock = ManualClock::new(0);
    // `Warm` is deliberate: in a reference scan nothing is cacheable, so a warm cache must make
    // no difference. If that ever changes, the trials assertion below fails.
    let profiled = profile_scan(
        root,
        store,
        REFERENCE_COVERAGE,
        true,
        CachePolicy::Warm,
        // The oracle, on the frozen clock too: every trial must return exactly the planted set.
        Some(ExpectedRefs::group(
            &group.group_id(),
            planted.iter().copied().collect(),
        )),
        &clock,
    );
    check_case_structure(&profiled);
    let cost = &profiled.cost;
    assert_eq!(
        cost.records
            .iter()
            .filter(|r| r.family == Some(EpochRecordKind::Recovery))
            .count(),
        1,
        "the canonical fixture should stage exactly one Recovery record"
    );
    assert!(
        cost.records.iter().all(|r| r.references),
        "the reference flag did not reach a parked record"
    );
}

/// The cacheable families behave differently across trials, and the profile depends on exactly
/// how. Pin it on a frozen clock rather than discovering it in a timing run.
///
/// Registry stands for both: `cacheable` is `matches!(family, Registry | Studio) &&
/// references.is_none()`, so the three modes are structurally distinct and a profile that
/// conflated them would divide its means by the wrong count without saying so.
#[test]
fn c3_cacheable_family_parks_when_fresh_and_hits_cache_when_warm() {
    let clock = ManualClock::new(0);
    // The real path the profile uses: three cases, one per mode, run round-robin.
    let mut cases = registry_cases(&[2]);
    assert_eq!(cases.len(), MODES.len());
    run_interleaved(&mut cases, &clock);

    let by = |mode: &str| {
        cases
            .iter()
            .find(|c| c.label.contains(mode))
            .unwrap_or_else(|| panic!("no case for {mode}"))
    };
    let parked_every_trial = |c: &Case| {
        c.cost
            .records
            .iter()
            .any(|r| r.family == Some(EpochRecordKind::Registry) && r.trials() == TRIALS)
    };

    let fresh = by("accounting_fresh");
    assert!(
        fresh.cost.reused.iter().all(|n| *n == 0),
        "a cleared cache still reported hits, so clear_for_test is not clearing: {:?}",
        fresh.cost.reused
    );
    assert!(
        parked_every_trial(fresh),
        "clearing the cache between trials must make every trial a fresh validation, or the \
         per-phase spreads divide by the wrong count"
    );

    // Warm is a *sequence*, not a state: the first trial populates the cache and parks, and only
    // the rest are hits. A single last-trial scalar could not tell those apart.
    let warm = by("accounting_warm");
    assert_eq!(
        warm.cost.reused[0], 0,
        "the first warm trial must be a miss - it is what populates the cache"
    );
    assert!(
        warm.cost.reused[1..].iter().all(|n| *n > 0),
        "a warm cache produced no hits after the first trial, so the hit path is unmeasured: {:?}",
        warm.cost.reused
    );
    assert!(
        !parked_every_trial(warm),
        "a cache hit is never parked by design, so a warm scan cannot park the Registry record \
         in every trial"
    );

    // And in reference mode the cache is bypassed entirely, warm or not.
    let refs = by("references");
    assert!(
        refs.cost.reused.iter().all(|n| *n == 0),
        "nothing is cacheable while collecting references: {:?}",
        refs.cost.reused
    );
    assert!(
        parked_every_trial(refs),
        "a reference scan must park the Registry record in every trial"
    );

    // The structural checks the profile itself applies, on the same cases.
    for case in &cases {
        check_case_structure(case);
    }
}

/// The multi-family vault really holds several families, and the scan really visits them.
///
/// Without this the "realistic full scan" label rests on the fixture builder having worked. A
/// vault that silently ended up with one family would still produce a scan, a visits count and a
/// per-family row - just not the ones the label claims. The builder asserts at least five
/// families are present; this additionally requires the **scan** to park a record from more than
/// one of them, which is what makes the visits figure a scan shape rather than an artifact.
#[test]
fn c3_multi_family_scan_parks_records_from_several_families() {
    let clock = ManualClock::new(0);
    let mut cases = [multi_family_case(CachePolicy::Fresh, false)];
    run_interleaved(&mut cases, &clock);
    check_case_structure(&cases[0]);

    let families: std::collections::BTreeSet<_> = cases[0]
        .cost
        .records
        .iter()
        .filter_map(|r| r.family)
        .collect();
    // Named, not counted. A threshold like "at least three" is satisfied by Recovery, Registry
    // and Studio alone - the three families already measured - so it would pass with the two
    // this fixture exists for entirely absent. The cache is cleared and `case` switches the
    // store to detach every validation, so every record present must park.
    for required in [
        EpochRecordKind::Recovery,
        EpochRecordKind::OwnerReceipts,
        EpochRecordKind::Intents,
        EpochRecordKind::Registry,
        EpochRecordKind::Studio,
    ] {
        assert!(
            families.contains(&required),
            "the scan parked no {required:?} record; parked families were {families:?}. \
             OwnerReceipts and Intents are the two this fixture exists to measure, so a count \
             threshold would have passed with both missing"
        );
    }
    assert!(
        cases[0].cost.visits > cases[0].cost.records.len() * TRIALS,
        "a parked record ends its visit, so a multi-family scan cannot take fewer visits than \
         records x trials"
    );
}

/// 13.7's restart rate under concurrent writes - what an `EpochInventoryJob` does when the vault
/// will not hold still.
///
/// **Deterministic, so it is a structural test rather than a profile.** Restart behaviour is
/// decided by counting, not by timing: a write either lands between two steps or it does not, and
/// a restart discards the cursor's progress whatever the machine's speed. That makes this the one
/// 13.7 item that can assert its result in the ordinary suite instead of printing a number.
///
/// `write_every` writes once per that many steps; `None` means an undisturbed scan. Returns the
/// restarts consumed and whether the job produced an inventory.
fn restart_rate(records: usize, write_every: Option<usize>) -> (usize, bool) {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    for n in 0..records {
        let key = format!("restart-{n}");
        stage_sized(&mut store, 7, &document(b"group", key.as_bytes()), 1024);
    }

    let mut job = store
        .begin_epoch_inventory_job(EpochInventoryCoverage::RecoveryOnly)
        .unwrap();
    let mut restarts = 0;
    let mut steps = 0;
    // A ceiling so a job that can never finish terminates the test rather than the test runner.
    // `MAX_INVENTORY_RESTARTS` bounds restarts, not steps, and a job that restarts forever would
    // otherwise spin.
    let ceiling = (records + 4) * (MAX_INVENTORY_RESTARTS + 2) * 4;
    loop {
        if steps >= ceiling {
            return (restarts, false);
        }
        steps += 1;
        // The interfering write, landing *between* steps, which is exactly the window C-3 exists
        // to survive: the old scanner held the store for its whole life so this could not happen.
        if write_every.is_some_and(|every| steps % every == 0) {
            store.epoch_mutation_guard();
        }
        match store.step_epoch_inventory_job(&mut job, 1, None).unwrap() {
            EpochInventoryStep::Parked => {
                let parked = store
                    .take_parked_job_record(&mut job)
                    .expect("Parked means a record is waiting");
                let validated = parked.validate().unwrap();
                if let EpochInventoryStep::Restarted = store
                    .install_validated_job_record(&mut job, validated)
                    .unwrap()
                {
                    restarts += 1;
                }
            }
            EpochInventoryStep::Restarted => restarts += 1,
            EpochInventoryStep::Unstable => return (restarts, false),
            EpochInventoryStep::Stepped(progress) => {
                if progress.complete {
                    return match store.finish_epoch_inventory_job(job).unwrap() {
                        EpochInventoryOutcome::Complete(_) => (restarts, true),
                        EpochInventoryOutcome::Restarted(next) => {
                            job = *next;
                            restarts += 1;
                            continue;
                        }
                        EpochInventoryOutcome::Unstable => (restarts, false),
                    };
                }
            }
        }
    }
}

/// The restart budget bounds retries; it does not make a moving vault scannable.
///
/// This is 13.7's last item, and the answer is a shape rather than a rate. An undisturbed scan
/// completes with no restarts. A vault written to rarely enough completes having spent some of
/// the budget. A vault written to on every step **never** completes, however large the budget,
/// because a restart discards all progress - so the scan can never get further than one step
/// before being overtaken again.
///
/// That is what makes L6's "under sustained writes a commit is held and retried" a statement
/// about liveness rather than latency: the failure is not a slow scan, it is `Unstable` and a
/// caller that must back off.
#[test]
fn c3_restart_budget_bounds_retries_but_does_not_survive_sustained_writes() {
    // Undisturbed: the control. No restarts, and an inventory.
    let (restarts, completed) = restart_rate(6, None);
    assert_eq!(restarts, 0, "an undisturbed scan restarted");
    assert!(
        completed,
        "an undisturbed scan did not produce an inventory"
    );

    // A write before every step. The scan cannot make progress between interruptions, so the
    // budget is spent and the job reports Unstable rather than completing slowly.
    let (restarts, completed) = restart_rate(6, Some(1));
    assert!(
        !completed,
        "a scan interrupted before every step produced an inventory, which would mean a restart \
         preserved progress it is specified to discard"
    );
    assert!(
        restarts <= MAX_INVENTORY_RESTARTS,
        "the job consumed {restarts} restarts against a budget of {MAX_INVENTORY_RESTARTS}"
    );

    // And the bound is real: the budget is spent, not merely large.
    assert_eq!(
        restarts, MAX_INVENTORY_RESTARTS,
        "sustained writes should consume the whole restart budget before giving up"
    );
}

/// The reference-mode multi-family vault, in the ordinary suite.
///
/// This exists because its absence cost a debugging cycle. The reference-mode case was only
/// instantiated inside the `#[ignore]`d profile, so when the vault was built with an
/// opaque-projection Recovery record - which `recovery_cids` refuses, as that helper's own doc
/// comment says - nothing failed until the profile was run by hand. A frozen-clock case keeps the
/// CID oracle load-bearing in CI for this fixture as it is for the others.
#[test]
fn c3_multi_family_reference_scan_collects_the_vaults_cids() {
    let clock = ManualClock::new(0);
    let mut cases = [multi_family_case(CachePolicy::Warm, true)];
    assert_eq!(
        cases[0].shape.cids,
        Some(8),
        "the canonical Recovery record is the only source of references in this vault"
    );
    // `run_trial`'s oracle checks the returned set against the expectation on every trial.
    run_interleaved(&mut cases, &clock);
    check_case_structure(&cases[0]);
}

/// Interleaved rounds must actually vary which case follows which.
///
/// This is a regression test for a claim that was false. The first implementation rotated the
/// start index, `(round + offset) % n`, and its comment said that stopped each case having a fixed
/// predecessor. It does not: rotating a cyclic sequence preserves the order, so every case except
/// the round's first keeps exactly the predecessor it had. At 35 cases over 8 rounds most cases
/// had precisely one predecessor throughout - the scheduling property the interleaved arm is
/// documented to have was simply absent.
///
/// Reproduces the ordering arithmetic rather than driving real cases, because the property is
/// about the schedule and nothing else: a real run costs minutes and would test the same integers.
#[test]
fn c3_interleaved_rounds_vary_each_case_predecessor() {
    let n = 35;
    // The generator `run_scheduled` itself consumes. An earlier version of this test built its
    // own copy of the shuffle, so reverting the dispatcher to the cyclic rotation would have left
    // this passing against a private permutation - a regression test that could not see the
    // regression.
    let orders = interleaved_rounds(n);
    assert_eq!(orders.len(), TRIALS, "one round per trial");

    // Every case still runs exactly once per round.
    for order in &orders {
        let mut seen: Vec<usize> = order.clone();
        seen.sort_unstable();
        assert_eq!(
            seen,
            (0..n).collect::<Vec<_>>(),
            "a round is not a permutation"
        );
    }

    // The property the rotation failed: collect each case's set of predecessors across rounds.
    let mut predecessors: Vec<std::collections::BTreeSet<usize>> = vec![Default::default(); n];
    for order in &orders {
        for w in order.windows(2) {
            predecessors[w[1]].insert(w[0]);
        }
    }
    let single = predecessors.iter().filter(|p| p.len() <= 1).count();
    assert!(
        single <= 2,
        "{single} of {n} cases have at most one predecessor across {TRIALS} rounds, so the \
         schedule is not varying what precedes each case - which is what rotating the start index \
         wrongly claimed to do"
    );

    // And the orders are genuinely different from each other, not one permutation repeated.
    assert!(
        orders.windows(2).any(|w| w[0] != w[1]),
        "every round used the same order"
    );
}

/// The distinct-CID count is settable at a fixed frame count, and the scan returns that set.
///
/// Named for what it establishes. It does **not** show the two axes are independent in any
/// stronger sense: the cells' timings are never compared, and on a frozen clock they could not be.
///
/// What running the cases adds, beyond `studio_frame_factorial`'s own count assertion: a fixture
/// that planted one CID per frame but *reported* a truncated set would satisfy that assertion and
/// still fail here, because `run_trial`'s oracle compares the returned set and its total against
/// the expectation. An earlier comment claimed such a fixture "would pass every other check",
/// which was wrong - the factorial's own assertion catches the simple case, as its mutation
/// showed.
#[test]
fn c3_distinct_cid_count_is_settable_at_a_fixed_frame_count() {
    let clock = ManualClock::new(0);
    // Same frame count, different reference counts: the axis that was previously confounded.
    let mut cases = studio_frame_factorial(&[(8, 1), (8, 8)]);
    assert_eq!(cases.len(), 2 * MODES.len());

    let cids_for = |cases: &[Case], suffix: &str| {
        cases
            .iter()
            .find(|c| c.label.ends_with(suffix))
            .unwrap_or_else(|| panic!("no case ending {suffix}"))
            .shape
            .cids
    };
    assert_eq!(cids_for(&cases, "n8_c1"), Some(1));
    assert_eq!(cids_for(&cases, "n8_c8"), Some(8));
    assert!(
        cases
            .iter()
            .all(|c| c.shape.requested_ops == Some(8) && c.shape.actual_ops == Some(8)),
        "the frame axis moved when only the reference axis was supposed to"
    );

    // And the oracle tracks the reference axis, not the frame count: the one-CID case must
    // collect exactly one.
    run_interleaved(&mut cases, &clock);
    for case in &cases {
        check_case_structure(case);
    }

    // The fixed-bytes premise, observed rather than argued, through the shared checker the
    // measured corpus also uses.
    assert_factorial_premise(&cases, 8, "accounting_fresh");
}

/// The factorial's premise: two cells differing only in distinct-CID count must hold Studio
/// records of the **same authenticated size**.
///
/// Shared, because the first version of this check lived inside the eight-frame smoke test and so
/// said nothing about the 16- and 128-frame cells the profile actually measures. A size change
/// affecting only the larger shapes, or only reference mode, would have gone unseen while the
/// prose claimed bytes were held fixed. The scan reads each record's authenticated size when it
/// parks it, so the premise costs nothing to observe - and it is checked **after** the scans,
/// outside any timed interval.
fn assert_factorial_premise(cases: &[Case], frames: usize, mode: &str) {
    let sizes = |distinct: usize| -> Vec<u64> {
        let suffix = format!("n{frames}_c{distinct}");
        let case = cases
            .iter()
            .find(|c| c.label.ends_with(&suffix) && c.label.contains(mode))
            .unwrap_or_else(|| panic!("no {mode} case ending {suffix}"));
        let mut out: Vec<u64> = case
            .cost
            .records
            .iter()
            .filter(|r| r.family == Some(EpochRecordKind::Studio))
            .map(|r| r.size)
            .collect();
        assert!(
            !out.is_empty(),
            "{}: no Studio record was parked, so its size was never observed",
            case.label
        );
        out.sort_unstable();
        out
    };
    assert_eq!(
        sizes(1),
        sizes(frames),
        "the {frames}-frame cells' Studio records differ in authenticated size under {mode}, so \
         that factorial varies bytes as well as distinct-CID count and cannot attribute a \
         difference to either"
    );
}

/// The Studio frame path, in the ordinary suite, so the reference oracle is load-bearing here and
/// not only in the `#[ignore]`d profile.
///
/// This exists because of a gap found by mutation. `studio_frame_cases` is the only fixture that
/// reaches the **Studio** arm of `validate_record_body` with references on;
/// `c3_canonical_reference_fixture_collects_its_planted_cids` stages a *Recovery* record and
/// exercises a different arm. So replacing `cids.extend(inspected.cids)` with a discard in the
/// Studio arm left every ordinary test passing, and would have been caught only by someone
/// running the opt-in profile. Four frames keeps it cheap.
#[test]
fn c3_studio_frame_reference_scan_returns_its_planted_cids() {
    let clock = ManualClock::new(0);
    let mut cases = studio_frame_cases(&[4]);
    assert_eq!(cases.len(), MODES.len());
    let reference_cases = cases.iter().filter(|c| c.references).count();
    assert_eq!(
        reference_cases, 1,
        "exactly one of the three modes collects references; without it this test proves nothing"
    );
    assert!(
        cases
            .iter()
            .filter(|c| c.references)
            .all(|c| c.shape.cids == Some(4)),
        "the frame fixture did not plant four distinct CIDs"
    );
    // The oracle inside `run_trial` is what checks the returned set on every trial.
    run_interleaved(&mut cases, &clock);
    for case in &cases {
        check_case_structure(case);
    }
}

/// The blocked-versus-interleaved comparison: one fixed corpus **shape**, four arms, ABBA order.
///
/// Earlier work reported a 2.5x shift on moving from blocked to interleaved and attributed it to
/// ordering. That was withdrawn, because the same patch also changed store construction, the
/// central statistic and the fixture-to-measurement delay. This holds those fixed - identical
/// fixture shapes, identical cache rules, identical summary statistic, the same `run_trial` on
/// both sides - so the schedule is the *intended* variable.
///
/// **What still differs between arms, stated rather than glossed.** Each arm builds its own
/// corpus, because cases cannot be reused without inheriting the previous arm's warm state - so
/// the corpus *shape* is fixed and the corpus *instance* is not: new temporary directories and,
/// for the Registry fixtures, freshly generated device keys. Arms also occupy different global
/// positions, and each follows a different predecessor (arm 1 follows the main run's reporting;
/// arms 2 to 4 follow the previous arm's teardown of four stores). And the schedules differ by
/// construction in fixture-to-first-measurement delay: Blocked first measures case *k* after
/// `8k` trials, Interleaved after *k*. That delay is part of what "the schedule" means here, not
/// a confound to be removed.
///
/// **What the ABBA order can and cannot do.** The order is Blocked, Interleaved, Interleaved,
/// Blocked, so Blocked holds global slots 1 and 4 and Interleaved slots 2 and 3; the `slot`
/// labels are within-pair. Averaging the two arms of each protocol cancels a *linear* drift. It
/// cancels neither a first-arm step nor curvature, and Interleaved never occupies the cold first
/// slot. With one observation per (protocol, slot) cell there is **no estimate of arm-to-arm
/// noise**, in a module that has recorded up to 86% movement between identical re-measurements.
/// So this comparison can only speak to an effect much larger than that; a small difference
/// between arms is not evidence of anything and must not be read as one. Replicating the arms is
/// what would fix that, and has not been done.
fn protocol_comparison(clock: &dyn catcoms_rt::Clock, profile: &str) {
    // Deliberately small: this measures scheduling, not families, and four corpus builds are the
    // cost of counterbalancing.
    let corpus = || {
        let mut cases = vec![recovery_accounting_case(&[256 * 1024, 4 * 1024 * 1024])];
        cases.extend(registry_cases(&[8]));
        cases
    };
    println!(
        "C3_PROFILE block=protocol_comparison arms=4 order=ABBA note=one_observation_per_cell_\
         so_only_a_large_effect_is_readable"
    );
    let mut arm = 0;
    for (first, second) in [
        (Protocol::Blocked, Protocol::Interleaved),
        (Protocol::Interleaved, Protocol::Blocked),
    ] {
        for (position, protocol) in [("first", first), ("second", second)] {
            arm += 1;
            let mut cases = corpus();
            run_scheduled(&mut cases, protocol, clock);
            for case in &cases {
                check_case_structure(case);
                report(
                    case,
                    protocol.label(),
                    &format!("{profile} arm={arm} slot={position}"),
                );
            }
        }
    }
}

// Design 13.7, part two (2026-10-08): the uncached families at their accepted ceilings.
//
// `validation_fits` must decide before it validates, from what it knows then: the family (from
// the filename) and the authenticated size. So for each family the question is the worst
// validation cost at a given size, across the shapes that make one size expensive, up to that
// family's accepted ceiling. Three families were measured only at trivial sizes or not at all:
// Intents (by count, by bytes, and with a retained branch), OwnerReceipts and DraftArchive.

/// Write one document's ordinary intent ledger directly.
///
/// Built in memory through `IntentLedger::prepare`, which applies the same per-document caps as
/// the production writer, then sealed and framed at the record's path exactly as the production
/// writer frames it (the technique of `per_document_count_and_byte_caps_survive_vault_decode_and_
/// prepare`). Writing 10,000 intents one `prepare_epoch_intent` at a time would rewrite the
/// growing record ten thousand times. Returns the number of intents written.
fn write_intent_ledger(
    store: &ServerStore,
    server: u64,
    document: &LogicalDocument,
    author: &catcoms_mls::MlsDevice,
    ops: Vec<DomainOp>,
    seed: u64,
) -> usize {
    let written = ops.len();
    let mut ledger = catcoms_replication::IntentLedger::new(document.clone());
    for op in ops {
        ledger
            .prepare(author.device_id(), op)
            .expect("the fixture stays within the per-document caps");
    }
    let scope = crate::store::epoch_intents::scope_bytes(server, document).unwrap();
    let state = crate::store::epoch_intents::EpochIntentState {
        ledger,
        overlay: None,
    };
    let sealed = catcoms_crypto::seal(
        &store.keys.db_key().unwrap(),
        &state.encode(&scope).unwrap(),
        &mut ChaCha20Rng::seed_from_u64(seed),
    )
    .unwrap();
    fs::write(
        store.epoch_intent_path(&scope),
        crate::store::frame(&sealed),
    )
    .unwrap();
    written
}

/// A real Flipnote document in the profile's group. A reference-mode scan decodes each intent as a
/// Studio operation for its document, which needs the document's real logical-key shape; the
/// generic `document` helper's arbitrary keys satisfy the structural decode but not that.
fn flipnote_document(object: u8) -> LogicalDocument {
    StudioTarget::Flipnote {
        channel: [7; 16],
        object: [object; 16],
    }
    .document(b"group")
    .unwrap()
}

/// A valid Flipnote title intent, so a reference-mode scan decodes it as it would a real one.
fn title_intent(document: &LogicalDocument, n: u128, text: &str) -> DomainOp {
    DomainOp {
        nonce: n.to_be_bytes(),
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        body: FlipnoteOp::SetHeader(catcoms_replication::studio::FlipnoteHeader::Title(
            text.into(),
        ))
        .encode()
        .unwrap(),
    }
}

/// Intents on the **count** axis: many small, valid intents, the densest shape per byte, up to
/// `MAX_INTENTS_PER_DOCUMENT`. One document per count, so each is its own row. Title intents name
/// no blob, so a reference-mode case collects nothing, and says so.
fn intents_count_case(counts: &[usize], references: bool) -> Case {
    let root = tempfile::tempdir().unwrap();
    let store = open(root.path());
    let author = catcoms_mls::MlsDevice::generate().unwrap();
    for (n, &count) in counts.iter().enumerate() {
        let doc = flipnote_document(n as u8);
        let ops = (0..count)
            .map(|n| title_intent(&doc, n as u128, "t"))
            .collect();
        assert_eq!(
            write_intent_ledger(&store, 7, &doc, &author, ops, count as u64),
            count
        );
    }
    case(
        format!(
            "intents_count_{}",
            if references {
                "references"
            } else {
                "accounting"
            }
        ),
        root,
        store,
        if references {
            REFERENCE_COVERAGE
        } else {
            EpochInventoryCoverage::RecoveryOwnerReceiptsAndIntents
        },
        references,
        CachePolicy::Fresh,
        FixtureShape {
            cids: references.then_some(0),
            ..FixtureShape::default()
        },
        references.then(ExpectedRefs::none),
    )
}

/// Intents on the **byte** axis: maximal 64 KiB envelopes filling the given operation bytes, up to
/// `MAX_INTENT_BYTES_PER_DOCUMENT`. Accounting only: the bodies are opaque filler, which the
/// structural decode does not interpret and a reference scan would refuse.
fn intents_bytes_case(op_bytes: &[usize]) -> Case {
    let root = tempfile::tempdir().unwrap();
    let store = open(root.path());
    let author = catcoms_mls::MlsDevice::generate().unwrap();
    for (n, &total) in op_bytes.iter().enumerate() {
        let doc = flipnote_document(0x80 + n as u8);
        let overhead = {
            let blank = title_intent(&doc, 0, "");
            blank.encode().unwrap().len() - blank.body.len()
        };
        let mut ops = Vec::new();
        let mut left = total;
        let mut n = 0u128;
        while left > 0 {
            let len = left.min(catcoms_replication::epoch::MAX_DOMAIN_OP_BYTES);
            let mut next = title_intent(&doc, n, "");
            next.body = vec![b'x'; len - overhead];
            ops.push(next);
            left -= len;
            n += 1;
        }
        write_intent_ledger(&store, 7, &doc, &author, ops, total as u64);
    }
    case(
        "intents_bytes_accounting",
        root,
        store,
        EpochInventoryCoverage::RecoveryOwnerReceiptsAndIntents,
        false,
        CachePolicy::Fresh,
        FixtureShape::default(),
        None,
    )
}

/// Intents with a **retained Closing branch** of `ops` accepted operations, through the production
/// Save path (`studio_handoff_ready_fixture`), up to `MAX_STUDIO_OVERLAY_OPS`. The structural
/// decode skips the branch's replay, which is C-1's point; this measures what it still costs.
/// The vault also holds the document's Studio and Registry records, reported as their own rows.
///
/// In reference mode the branch's base CIDs and every pending operation's blob CID are collected.
/// The fixture's operations are all title edits, which name no blob, so the oracle is the empty
/// set: what this times is the decode reference collection forces, not a large merge.
fn intents_branch_cases(op_counts: &[usize], references: bool) -> Vec<Case> {
    op_counts
        .iter()
        .map(|&ops| {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let device = catcoms_mls::MlsDevice::generate().unwrap();
            let group = catcoms_mls::ServerGroup::create(&device).unwrap();
            let target = StudioTarget::Flipnote {
                channel: [7; 16],
                object: [ops as u8; 16],
            };
            crate::store::epoch_studio::tests::performance::studio_handoff_ready_fixture(
                &mut store, 7, &group, &device, target, ops,
            );
            case(
                format!(
                    "intents_branch_{ops}_{}",
                    if references {
                        "references"
                    } else {
                        "accounting"
                    }
                ),
                root,
                store,
                REFERENCE_COVERAGE,
                references,
                CachePolicy::Fresh,
                FixtureShape {
                    requested_ops: Some(ops),
                    cids: references.then_some(0),
                    ..FixtureShape::default()
                },
                references.then(ExpectedRefs::none),
            )
        })
        .collect()
}

/// OwnerReceipts, through the production writer. The journal is bounded by construction
/// (`MAX_OWNER_RECEIPT_JOURNAL_BYTES`: three receipt roles, a repair pair, one retired receipt and
/// close, about 12 KiB), so its ceiling is small; this measures the ordinary one-receipt journal.
fn owner_receipts_case() -> Case {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let device = catcoms_mls::MlsDevice::generate().unwrap();
    let group = catcoms_mls::ServerGroup::create(&device).unwrap();
    let doc = StudioTarget::Flipnote {
        channel: [7; 16],
        object: [3; 16],
    }
    .document(&group.group_id())
    .unwrap();
    let receipt = catcoms_replication::Receipt::sign(
        doc.clone(),
        0,
        [11; 32],
        [12; 32],
        group.epoch(),
        catcoms_replication::InheritedCheckpoint::EpochZero,
        &device,
    )
    .unwrap();
    let mut budget = family_budget(&mut store, 7, &doc);
    store
        .prepare_epoch_owner_receipt(
            7,
            receipt,
            &group,
            group.epoch(),
            &mut ChaCha20Rng::seed_from_u64(5),
            &mut budget,
        )
        .expect("owner receipt");
    case(
        "owner_receipts_accounting",
        root,
        store,
        EpochInventoryCoverage::RecoveryAndOwnerReceipts,
        false,
        CachePolicy::Fresh,
        FixtureShape::default(),
        None,
    )
}

/// DraftArchive in accounting mode, up to the payload ceiling. Accounting does not decode the
/// payload at all, only authenticates and names the record, so the body here is opaque filler
/// (`write_draft_archive_for_test`, whose framing, binding and authentication are the real ones).
fn draft_archive_case(body_bytes: &[usize]) -> Case {
    let root = tempfile::tempdir().unwrap();
    let store = open(root.path());
    for (n, &bytes) in body_bytes.iter().enumerate() {
        let doc = document(b"group", format!("archive-{n}").as_bytes());
        crate::store::epoch_draft_archive::write_draft_archive_for_test(
            &store,
            7,
            &doc,
            &vec![9u8; bytes],
            &mut ChaCha20Rng::seed_from_u64(n as u64),
        )
        .unwrap();
    }
    case(
        "draft_archive_accounting",
        root,
        store,
        REFERENCE_COVERAGE,
        false,
        CachePolicy::Fresh,
        FixtureShape::default(),
        None,
    )
}

/// Not ignored: small versions of every fixture the uncached-families profile builds must scan
/// cleanly in the mode they are profiled in, so a fixture that stops being valid fails here rather
/// than halfway through an opt-in profile run, or worse, is measured while refused.
#[test]
fn uncached_family_fixtures_scan_cleanly() {
    let mut cases = vec![
        intents_count_case(&[1, 10], false),
        intents_count_case(&[1, 10], true),
        intents_bytes_case(&[64 * 1024]),
        owner_receipts_case(),
        draft_archive_case(&[1024]),
    ];
    cases.extend(intents_branch_cases(&[1], false));
    cases.extend(intents_branch_cases(&[1], true));
    for case in &mut cases {
        let mut scan = case.store.scan_epoch_files(case.coverage).unwrap();
        if case.references {
            scan.collect_creative_references().unwrap();
        }
        while !scan
            .step()
            .unwrap_or_else(|e| panic!("{}: {e}", case.label))
            .complete
        {}
        if case.references {
            scan.finish_creative_references()
                .unwrap_or_else(|e| panic!("{}: {e}", case.label));
        } else {
            scan.finish()
                .unwrap_or_else(|e| panic!("{}: {e}", case.label));
        }
    }
}

/// Opt-in, real clock: the uncached families at their accepted ceilings, interleaved with the
/// Recovery size sweep for comparison with the earlier profile. Prints; asserts correctness,
/// never machine speed. Run on a quiet machine:
/// `RUST_MIN_STACK=33554432 cargo test --release -p catcoms-app --lib -- --ignored
/// profile_c3_uncached_families --nocapture`.
#[test]
#[ignore = "opt-in design 13.7 profiling of the uncached families; no machine-speed assertion"]
fn profile_c3_uncached_families() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let clock = &SystemClock;
    let mut all = vec![
        recovery_accounting_case(&[1024, 16 * 1024, 256 * 1024, 1024 * 1024, 4 * 1024 * 1024]),
        intents_count_case(&[1, 100, 1_000, 10_000], false),
        intents_count_case(&[1, 100, 1_000, 10_000], true),
        intents_bytes_case(&[
            256 * 1024,
            1024 * 1024,
            catcoms_replication::epoch::MAX_INTENT_BYTES_PER_DOCUMENT,
        ]),
        owner_receipts_case(),
        draft_archive_case(&[
            1024,
            1024 * 1024,
            catcoms_replication::studio::MAX_STUDIO_DRAFT_ARCHIVE_BYTES - 1024,
        ]),
    ];
    for references in [false, true] {
        all.extend(intents_branch_cases(
            &[1, 64, catcoms_replication::studio::MAX_STUDIO_OVERLAY_OPS],
            references,
        ));
    }
    println!(
        "C3_PROFILE group=uncached_families cases={} trials={TRIALS} order=interleaved",
        all.len()
    );
    run_interleaved(&mut all, clock);
    for case in &all {
        check_case_structure(case);
        report(case, Protocol::Interleaved.label(), profile);
    }
}

/// Opt-in, real clock, real sizes. Prints; asserts correctness, never machine speed.
///
/// **Every case is built first, then all of them are run round-robin.** Earlier versions ran
/// each case's trials in a block, which meant a comparison between two cases was a comparison
/// between two different moments in a long run - and re-measuring identical fixtures across
/// runs moved by up to 86%, so that was not a safe thing to do. Interleaving does not make any
/// single figure more accurate; it makes differences *within one profile* mean something.
///
/// Results are printed as `min/median/max` microseconds, never as a mean.
#[test]
#[ignore = "opt-in design 13.7 profiling of C-3 scan phases; no machine-speed assertion"]
fn profile_c3_visit_cost() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let clock = &SystemClock;

    // **Every case in one interleaved set.** The lists below are named only to keep the builders
    // readable; they are concatenated and run together, so any two cases in the profile are
    // comparable under the same schedule.
    //
    // This was briefly split into separately-timed groups, because 37 cases alive at once aborted
    // the release binary with no panic and that looked like resource exhaustion. It was not: the
    // cause was **stack size**, and with `RUST_MIN_STACK=33554432` all 37 run to completion
    // interleaved. Two things wrong with the grouping are worth recording rather than quietly
    // deleting - it was a fix for a misdiagnosed problem, and it did not even do what its comment
    // claimed, because every builder ran before the first group was timed, so all 35 stores were
    // live anyway. Full interleaving also removes the "comparisons across groups are not
    // comparable" limitation the grouping had introduced.
    let mut all: Vec<Case> = Vec::new();
    let groups: Vec<(&str, Vec<Case>)> = vec![
        (
            "recovery",
            vec![recovery_accounting_case(&[
                1024,
                16 * 1024,
                256 * 1024,
                1024 * 1024,
                4 * 1024 * 1024,
            ])],
        ),
        (
            "recovery_references",
            recovery_reference_cases(&[1, 16, 128, 512], clock),
        ),
        ("registry", registry_cases(&[2, 8, 24])),
        // 24 and not 32: `build` stops once the epoch is nearly full, and at 160 KiB per message
        // the 4 MiB epoch fits about 25. An earlier run requested 32, silently got fewer, and
        // divided by 32 anyway. `check_case_structure` now fails rather than letting that recur.
        ("studio_titles", studio_cases(&[3, 12, 24])),
        // Studio with actual pixels, because the title-only sources above collect no CIDs at all.
        // The factorial: (128,1) against (128,128) isolates reference count at fixed frame count;
        // (16,1) against (128,1) isolates frame count at fixed reference count. Kept in one group
        // so those two comparisons are interleaved.
        (
            "studio_frames",
            studio_frame_factorial(&[(16, 1), (16, 16), (128, 1), (128, 128)]),
        ),
    ];

    for (_, cases) in groups {
        all.extend(cases);
    }
    all.push(multi_family_case(CachePolicy::Fresh, false));
    all.push(multi_family_case(CachePolicy::Warm, true));
    println!(
        "C3_PROFILE group=all cases={} trials={TRIALS} order=interleaved",
        all.len()
    );
    run_interleaved(&mut all, clock);
    for case in &all {
        check_case_structure(case);
        report(case, Protocol::Interleaved.label(), profile);
    }
    // The factorial's fixed-bytes premise, on the cells actually measured rather than only on the
    // smoke fixture's eight-frame pair, and in every mode the factorial reports. Checked after
    // the scans so it costs no timed interval, and before the figures are used for anything.
    for frames in [16, 128] {
        for mode in ["accounting_fresh", "accounting_warm", "references"] {
            assert_factorial_premise(&all, frames, mode);
        }
    }
    drop(all);

    // Deliberately after the main run's cases are dropped: the comparison's own stores should not
    // be competing with 35 live ones, and its labels would otherwise be ambiguous against the
    // rows above.
    protocol_comparison(clock, profile);
}
