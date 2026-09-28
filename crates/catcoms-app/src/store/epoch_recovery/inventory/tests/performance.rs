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
use catcoms_replication::studio::{FlipnoteOp, StudioEpoch, StudioRecovery, StudioTarget};
use catcoms_replication::DomainOp;
use catcoms_rt::SystemClock;
use catcoms_storage::Cid;
use std::collections::BTreeMap;

/// Reference collection refuses anything narrower: a partial inventory must not be allowed to
/// replace a transient pre-publication hold.
const REFERENCE_COVERAGE: EpochInventoryCoverage =
    EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsRegistryAndStudio;
/// Enough repetitions that a millisecond clock resolves the per-record validation figure.
const REPETITIONS: usize = 64;
/// Complete scans, so the single-sample phases are summed rather than reported from one tick.
const TRIALS: usize = 8;

/// Min, **upper median**, max of a sample set, converted to microseconds per unit of work.
///
/// A single summary figure carries no information about whether a difference between two cases is
/// real, and re-measuring identical Recovery fixtures moved 36%, 39% and 86% between runs. The
/// spread is what makes that visible.
///
/// **The middle figure is the upper median**, `sorted[len / 2]`, not an interpolated one: for
/// eight samples it is the fifth sorted value. For `0,0,0,0,1000,1000,1000,1000` it reports
/// 1000, where an arithmetic midpoint would be 500. That convention is deliberate - it never
/// invents a value the clock did not produce - but it must be labelled, because it is not what
/// "median" unqualified would mean.
///
/// `zero_samples` is carried for the same reason: a printed `0/1000/1000` is compatible with
/// several different zero counts, so the spread alone cannot say how much of the phase fell
/// below the clock's resolution. The count can.
///
/// `per` is how many units of work one sample covers: 1 for a phase timed once per trial,
/// `REPETITIONS` for the batched validation - in which case each sample is itself a batch mean,
/// and this is a spread *of means*, not of individual timings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Spread {
    min_us: u128,
    upper_median_us: u128,
    max_us: u128,
    samples: usize,
    /// How many raw samples read exactly zero milliseconds.
    zero_samples: usize,
    /// Sum of the raw millisecond samples, so a spread of all-zeros is visibly "below the
    /// clock's resolution" rather than "free".
    raw_total_ms: u64,
}

impl Spread {
    fn of(samples: &[u64], per: u128) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        let mut us: Vec<u128> = samples.iter().map(|ms| *ms as u128 * 1_000 / per).collect();
        us.sort_unstable();
        Self {
            min_us: us[0],
            upper_median_us: us[us.len() / 2],
            max_us: us[us.len() - 1],
            samples: us.len(),
            zero_samples: samples.iter().filter(|ms| **ms == 0).count(),
            raw_total_ms: samples.iter().sum(),
        }
    }
    /// Resolved *well enough to take a ratio from*, which is a stronger condition than nonzero.
    ///
    /// A phase whose upper median is a single clock tick is not measured to better than 100%:
    /// the true value lies somewhere in one whole millisecond. Requiring strictly more than one
    /// tick is what makes the reported fraction mean anything, and it is the condition the
    /// status ledger states - the earlier implementation only rejected a median of zero, which
    /// left one-tick medians producing confident-looking fractions.
    fn resolved_for_ratio(&self) -> bool {
        self.upper_median_us > 1_000
    }
}

impl std::fmt::Display for Spread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{}/{}(z{})",
            self.min_us, self.upper_median_us, self.max_us, self.zero_samples
        )
    }
}

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
        cost,
        ..
    } = case;
    let (coverage, references) = (*coverage, *references);
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
                parked.revalidate().unwrap();
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
    if references {
        store.finish_cursor_creative_references(cursor).unwrap();
    } else {
        store.finish_epoch_storage_scan(cursor).unwrap();
    }
    cost.finish_ms += clock.monotonic_ms().saturating_sub(t);
}

/// Run `TRIALS` trials of every case, **round-robin rather than case by case**.
///
/// See [`Case`] for why the ordering is the point. This is the whole of the repetition
/// discipline: it does not make any single figure more accurate, it makes differences *between*
/// cases in one profile comparable, which the block-ordered version could not claim.
fn run_interleaved(cases: &mut [Case], clock: &dyn catcoms_rt::Clock) {
    for _ in 0..TRIALS {
        for case in cases.iter_mut() {
            run_trial(case, clock);
        }
    }
}

/// Build a case around a store that has already been populated.
fn case(
    label: impl Into<String>,
    root: tempfile::TempDir,
    store: ServerStore,
    coverage: EpochInventoryCoverage,
    references: bool,
    cache: CachePolicy,
    shape: FixtureShape,
) -> Case {
    Case {
        label: label.into(),
        _root: root,
        store,
        coverage,
        references,
        cache,
        shape,
        cost: ScanCost::default(),
    }
}

/// Convenience for the frozen-clock smoke tests, which profile one case and assert structure.
fn profile_scan(
    root: tempfile::TempDir,
    store: ServerStore,
    coverage: EpochInventoryCoverage,
    references: bool,
    cache: CachePolicy,
    clock: &dyn catcoms_rt::Clock,
) -> Case {
    let mut one = [case(
        "smoke",
        root,
        store,
        coverage,
        references,
        cache,
        // Smoke fixtures have no operation axis; `cids: Some(0)` where a reference mode is used
        // says so explicitly rather than tripping the shape check.
        FixtureShape {
            cids: references.then_some(0),
            ..FixtureShape::default()
        },
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
fn report(case: &Case, profile: &str) {
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
         interleaved=true build={profile} page_cache=warm_written_immediately_before \
         units=min/upper_median/max_us_and_zero_sample_count",
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
            )
        })
        .collect()
}

/// Registry: one of the two families whose expensive typed reconstruction motivated C-3.
///
/// The axis is **operation count**, not bytes: `save_inventory_fixture_ops` builds a real signed
/// registry log of `ops` operations at 160 KiB per message, and the reconstruction walks them.
/// A byte axis alone would not distinguish a large record from a structurally deep one.
///
/// Measured in all three modes, because Registry is cacheable and the modes are not variations
/// of one number: fresh accounting validation, the accounting cache-hit path, and reference
/// collection (in which nothing is cacheable at all).
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
                    // The Registry validator arm collects no CIDs in either mode.
                    cids: Some(0),
                },
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
                    // Title headers name no CIDs. Stated, not implied.
                    cids: Some(0),
                },
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
    let mut cases = Vec::new();
    for frames in frame_counts {
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
                    &mut store, 7, &group, &device, target, *frames,
                );
            let distinct: std::collections::BTreeSet<_> = planted.iter().copied().collect();
            assert_eq!(
                distinct.len(),
                *frames,
                "the frame fixture planted duplicate CIDs, so its reference-count axis is fiction"
            );
            cases.push(case(
                format!("studio_frames_{mode}_n{frames}"),
                root,
                store,
                REFERENCE_COVERAGE,
                references,
                cache,
                FixtureShape {
                    requested_ops: Some(*frames),
                    actual_ops: Some(accepted),
                    physical_bytes: None,
                    cids: Some(distinct.len()),
                },
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
        &clock,
    );
    let cost = &profiled.cost;

    assert_eq!(
        cost.records.len(),
        sizes.len(),
        "a record did not park, so the classifier is no longer detaching everything and this \
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

    let mut cases = vec![recovery_accounting_case(&[
        1024,
        16 * 1024,
        256 * 1024,
        1024 * 1024,
        4 * 1024 * 1024,
    ])];
    cases.extend(recovery_reference_cases(&[1, 16, 128, 512], clock));
    cases.extend(registry_cases(&[2, 8, 24]));
    // 24 and not 32: `build` stops once the epoch is nearly full, and at 160 KiB per message the
    // 4 MiB epoch fits about 25. The previous run requested 32, silently got fewer, and divided
    // by 32 anyway. `check_case_structure` now fails rather than letting that recur.
    cases.extend(studio_cases(&[3, 12, 24]));
    // Studio with actual pixels, because the title-only sources above collect no CIDs at all.
    cases.extend(studio_frame_cases(&[16, 128]));
    println!(
        "C3_PROFILE run cases={} trials={TRIALS} order=interleaved",
        cases.len()
    );

    run_interleaved(&mut cases, clock);

    for case in &cases {
        check_case_structure(case);
        report(case, profile);
    }
}
