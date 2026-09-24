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
//! `scripts/check-no-ambient.sh` forbids `Instant::now` everywhere under `crates/`, test code
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

/// One record's per-phase cost, summed over trials, grouped by the facts the classifier sees.
#[derive(Debug, Clone)]
struct RecordCost {
    family: EpochRecordKind,
    size: u64,
    references: bool,
    /// Summed over `trials` samples of one `step_epoch_storage_scan`.
    read_and_park_ms: u64,
    /// Summed over `trials` samples of `install_validated_record`.
    install_ms: u64,
    /// Summed over `trials` batches of `REPETITIONS` validations.
    validation_batch_ms: u64,
    trials: usize,
}

impl RecordCost {
    fn validation_mean_us(&self) -> u128 {
        if self.trials == 0 {
            return 0;
        }
        self.validation_batch_ms as u128 * 1_000 / (REPETITIONS as u128 * self.trials as u128)
    }
    fn read_and_park_mean_us(&self) -> u128 {
        if self.trials == 0 {
            return 0;
        }
        self.read_and_park_ms as u128 * 1_000 / self.trials as u128
    }
    fn install_mean_us(&self) -> u128 {
        if self.trials == 0 {
            return 0;
        }
        self.install_ms as u128 * 1_000 / self.trials as u128
    }
    /// The share of the two *measured* per-record components that `validation_fits` is in a
    /// position to move. Not a share of total scan custody, and not a speedup.
    ///
    /// Reported only when both components resolved across the whole trial set; a ratio taken
    /// from a single millisecond tick says nothing.
    fn deferrable_fraction(&self) -> Option<u128> {
        let validation = self.validation_mean_us();
        let retained = self.read_and_park_mean_us();
        if self.read_and_park_ms == 0 || self.validation_batch_ms == 0 {
            return None;
        }
        Some(validation * 100 / (validation + retained))
    }
}

/// Whole-scan figures, kept per trial rather than averaged.
#[derive(Debug, Default)]
struct ScanCost {
    trials: usize,
    visits: usize,
    finish_ms: u64,
    begin_ms: u64,
    /// `reused_records` from the final progress of each trial: validation-cache hits, which are
    /// never parked and so never contribute a validation sample.
    reused: usize,
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
            .position(|r| r.family == family && r.size == size && r.references == references)
        {
            return &mut self.records[index];
        }
        self.records.push(RecordCost {
            family,
            size,
            references,
            read_and_park_ms: 0,
            install_ms: 0,
            validation_batch_ms: 0,
            trials: 0,
        });
        self.records.last_mut().expect("just pushed")
    }
    fn largest(&self, family: EpochRecordKind) -> Option<&RecordCost> {
        self.records
            .iter()
            .filter(|r| r.family == family)
            .max_by_key(|r| r.size)
    }
}

/// Drive `TRIALS` complete budgeted scans, timing every phase.
///
/// The budget is deliberately enormous: the point is not to observe the deadline firing, it is
/// to put the cursor in the mode where every record parks, so the validation phase can be timed
/// on its own.
fn profile_scan(
    store: &mut ServerStore,
    coverage: EpochInventoryCoverage,
    references: bool,
    clock: &dyn catcoms_rt::Clock,
) -> ScanCost {
    let mut out = ScanCost::default();
    for _ in 0..TRIALS {
        out.trials += 1;
        let t = clock.monotonic_ms();
        let mut cursor = store.begin_epoch_storage_scan(coverage).unwrap();
        if references {
            store
                .collect_cursor_creative_references(&mut cursor)
                .unwrap();
        }
        out.begin_ms += clock.monotonic_ms().saturating_sub(t);
        let mut last;
        loop {
            let t = clock.monotonic_ms();
            let progress = store
                .step_epoch_storage_scan(&mut cursor, ENTRIES_PER_STEP, Some((clock, u64::MAX)))
                .unwrap();
            let read_and_park_ms = clock.monotonic_ms().saturating_sub(t);
            out.visits += 1;
            last = progress;
            if let Some(parked) = store.take_parked_record(&mut cursor) {
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

                let entry = out.record_mut(family, size, refs);
                entry.trials += 1;
                // A parked record ends its visit, so the step just timed read exactly this one.
                entry.read_and_park_ms += read_and_park_ms;
                entry.validation_batch_ms += validation_batch_ms;
                entry.install_ms += install_ms;
                continue;
            }
            if progress.complete {
                break;
            }
        }
        out.reused = last.reused_records;
        let t = clock.monotonic_ms();
        if references {
            store.finish_cursor_creative_references(cursor).unwrap();
        } else {
            store.finish_epoch_storage_scan(cursor).unwrap();
        }
        out.finish_ms += clock.monotonic_ms().saturating_sub(t);
    }
    out
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
fn report(label: &str, cost: &ScanCost, profile: &str) {
    let mut records = cost.records.clone();
    records.sort_by_key(|r| r.size);
    for r in &records {
        println!(
            "C3_PROFILE scan={label} family={:?} bytes={} references={} \
             read_and_park_us={} validation_us={} install_us={} \
             validation_batch_ms={} trials={} deferrable_fraction_of_measured={}",
            r.family,
            r.size,
            r.references,
            r.read_and_park_mean_us(),
            r.validation_mean_us(),
            r.install_mean_us(),
            r.validation_batch_ms,
            r.trials,
            r.deferrable_fraction()
                .map_or_else(|| "unresolved".to_string(), |v| v.to_string()),
        );
    }
    println!(
        "C3_PROFILE scan={label} trials={} visits={} records={} begin_ms={} finish_ms={} \
         cache_hits={} repetitions={} build={profile} page_cache=warm_written_immediately_before",
        cost.trials,
        cost.visits,
        cost.records.len(),
        cost.begin_ms,
        cost.finish_ms,
        cost.reused,
        REPETITIONS,
    );
}

/// The accounting-only control: opaque projections across a size range, no reference collection.
fn measure_recovery_accounting(sizes: &[usize], clock: &dyn catcoms_rt::Clock, profile: &str) {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    for (n, size) in sizes.iter().enumerate() {
        let key = format!("recovery-{n}");
        stage_sized(&mut store, 7, &document(b"group", key.as_bytes()), *size);
    }
    let cost = profile_scan(
        &mut store,
        EpochInventoryCoverage::RecoveryOnly,
        false,
        clock,
    );
    assert_eq!(
        cost.records.len(),
        sizes.len(),
        "every staged record must have parked, or the validation phase was not isolated"
    );
    report("recovery_accounting", &cost, profile);
}

/// The canonical case: real projections that reference collection actually inspects.
///
/// Times **and checks** the result: a reference scan that returned an empty or short CID set
/// quickly would otherwise look like a cheap one.
fn measure_recovery_references(frames: &[usize], clock: &dyn catcoms_rt::Clock, profile: &str) {
    for count in frames {
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
        // refuses anything narrower, because a partial inventory cannot be allowed to replace a
        // transient pre-publication hold.
        let cost = profile_scan(&mut store, REFERENCE_COVERAGE, true, clock);
        let recovery: Vec<_> = cost
            .records
            .iter()
            .filter(|r| r.family == EpochRecordKind::Recovery)
            .collect();
        assert_eq!(
            recovery.len(),
            1,
            "the canonical fixture should stage exactly one Recovery record"
        );
        assert!(
            cost.records.iter().all(|r| r.references),
            "the reference flag did not reach a parked record, so this timed an accounting scan"
        );

        // The collected set, checked rather than assumed. Re-run once outside the timing loop.
        store.creative_protection.lock().unwrap().unknown_for_test();
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
            "the reference scan did not collect the frames' CIDs, so its timing is the timing of \
             an incomplete result"
        );
        assert_eq!(
            got.len(),
            *count,
            "the reference count axis is wrong: {count} frames collected {} CIDs",
            got.len()
        );
        println!(
            "C3_PROFILE scan=recovery_references frames={count} cids_collected={}",
            got.len()
        );
        report("recovery_references", &cost, profile);
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
    let cost = profile_scan(
        &mut store,
        EpochInventoryCoverage::RecoveryOnly,
        false,
        &clock,
    );

    assert_eq!(
        cost.records.len(),
        sizes.len(),
        "a record did not park, so the classifier is no longer detaching everything and this \
         measurement no longer isolates the validation phase"
    );
    // Phase coverage: every parked result was installed, every trial finished, and each record
    // contributed a sample in every trial. Without these the per-phase sums are over an unknown
    // number of occurrences and the means are meaningless.
    assert_eq!(cost.trials, TRIALS, "a trial did not run to completion");
    assert!(
        cost.records.iter().all(|r| r.trials == TRIALS),
        "a record was not parked in every trial, so its per-phase means divide by the wrong count"
    );
    assert!(
        cost.visits > cost.records.len() * TRIALS,
        "a parked record ends its visit, so {} records over {TRIALS} trials cannot have taken \
         only {} visits",
        cost.records.len(),
        cost.visits,
    );
    assert_eq!(
        cost.reused, 0,
        "a Recovery scan reported validation-cache hits, which are never parked, so a record \
         would silently stop contributing a validation sample"
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
    let cost = profile_scan(&mut store, REFERENCE_COVERAGE, true, &clock);
    assert_eq!(
        cost.records
            .iter()
            .filter(|r| r.family == EpochRecordKind::Recovery)
            .count(),
        1,
        "the canonical fixture should stage exactly one Recovery record"
    );
    assert!(
        cost.records.iter().all(|r| r.references),
        "the reference flag did not reach a parked record"
    );
    assert!(
        cost.records.iter().all(|r| r.trials == TRIALS),
        "a record was not parked in every trial; in a reference scan nothing is cacheable, so \
         every record must park every time"
    );
}

/// Opt-in, real clock, real sizes. Prints; asserts correctness, never machine speed.
#[test]
#[ignore = "opt-in design 13.7 profiling of C-3 scan phases; no machine-speed assertion"]
fn profile_c3_visit_cost() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    measure_recovery_accounting(
        &[1024, 16 * 1024, 256 * 1024, 1024 * 1024, 4 * 1024 * 1024],
        &SystemClock,
        profile,
    );
    measure_recovery_references(&[1, 16, 128, 512], &SystemClock, profile);
}
