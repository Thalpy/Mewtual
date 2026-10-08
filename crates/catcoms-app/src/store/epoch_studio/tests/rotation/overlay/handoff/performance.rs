//! Opt-in custody measurements before actor activation. Initial fixture writes are batched;
//! measured decoding, projection, signing and durable handoff use the production adapters.
use super::*;
use crate::store::epoch_intents::EpochIntentState;
use crate::store::measure::Spread;
use catcoms_replication::studio::{StudioOverlay, StudioOverlayState};
use catcoms_rt::{Clock, SystemClock};

#[test]
fn studio_overlay_handoff_prepared_signing_full_count_keeps_source_private() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, expected) = fixture(&f, &mut store, 256);
        let before = canonical(&store);
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        let metadata = state.handoff_metadata().unwrap().clone();
        let authority = metadata.handoff_authority(&f.device, &f.group, 0).unwrap();
        let source = f.load(&store).unwrap().unit;
        let clock = SystemClock;
        let start = clock.monotonic_ms();
        let mut batch = metadata
            .prepare_handoff_detached(source, state.ledger.clone(), authority)
            .unwrap();
        let prepare_ms = clock.monotonic_ms() - start;
        assert_eq!(batch.remaining(), 256);
        let mut max_turn_ms = 0;
        for remaining in (0..256).rev() {
            let start = clock.monotonic_ms();
            assert!(batch.sign_next(&f.device, &f.group, 0).unwrap());
            max_turn_ms = max_turn_ms.max(clock.monotonic_ms() - start);
            assert_eq!(
                batch.remaining(),
                remaining,
                "full-count signing turn exceeded one operation"
            );
        }
        assert!(!batch.sign_next(&f.device, &f.group, 0).unwrap());
        assert_eq!(
            canonical(&store),
            before,
            "signing exposed a durable prefix"
        );
        let start = clock.monotonic_ms();
        let (mut candidate, prepared) = batch.finish().unwrap().into_parts();
        let finish_ms = clock.monotonic_ms() - start;
        assert_eq!(candidate.op_count(), 256);
        assert_eq!(candidate.projection().unwrap(), expected);
        for (_, intent) in state.ledger.pending() {
            assert!(candidate
                .contains_exact_operation(intent.author, &intent.operation)
                .unwrap());
        }
        assert_eq!(
            prepared.evidence(&candidate, &state.ledger).unwrap(),
            catcoms_replication::studio::StudioHandoffEvidence::Complete
        );
        let restored = StudioEpoch::restore(
            &candidate.snapshot().unwrap(),
            &f.group,
            f.target,
            f.device.device_id(),
        )
        .unwrap();
        assert_eq!(restored.op_count(), 256);
        assert_eq!(restored.projection().unwrap(), expected);
        assert_eq!(
            canonical(&store),
            before,
            "finish installed a source outside the durable writer"
        );
        println!("HANDOFF_SIGNING_PROFILE art={art} operations=256 prepare_ms={prepare_ms} max_turn_ms={max_turn_ms} finish_ms={finish_ms}");
    }
}

pub(super) fn fixture(
    f: &Fixture,
    store: &mut ServerStore,
    count: usize,
) -> ([u8; 32], StudioProjection) {
    let (close, basis) = closing(f, store);
    let mut state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let mut prefix = Vec::new();
    let mut records = Vec::new();
    // Splice all but the last entry, then add the last through the real `StudioOverlayState`
    // append. The total is unchanged, and the record ends up in the format production writes.
    //
    // Why that matters: `StudioOverlay::encode_vault` emits version 1, and `decode_vault` takes a
    // compatibility branch for v1 that *hard-codes* `prepared: None`, `completed: None`,
    // `minimum_new_basis_closed_epoch: 0` and `legacy: true`. A record assembled purely by
    // splicing is therefore a legacy record: the v2 header parse and `validate()` never run, and
    // any test comparing those three fields is comparing constants rather than decoded values.
    // `StudioOverlayState::append` sets `legacy = false`, which is what makes it v2.
    let spliced = count.saturating_sub(1);
    for n in 0..spliced {
        let mut op = f.title();
        op.nonce = (n as u128 + 100).to_be_bytes();
        op.body = match f.target {
            StudioTarget::Index { .. } => IndexOp::SetTitle {
                object: [1; 16],
                title: format!("overlay measurement {n}"),
            }
            .encode()
            .unwrap(),
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!("overlay measurement {n}")))
                    .encode()
                    .unwrap()
            }
        };
        let id = state.ledger.prepare(f.device.device_id(), op).unwrap();
        // As in the existing operation-cap fixture: obtain each annotation by typed append,
        // assemble consecutive sequences, then require complete production decode and replay.
        // This avoids timing hundreds of redundant fixture-only Closing-source restorations.
        let mut single = StudioOverlay::new(&basis);
        single.append(&basis, &state.ledger, id, n as u64).unwrap();
        let bytes = single.encode_vault(&state.ledger).unwrap();
        let split = bytes.len() - 88;
        if prefix.is_empty() {
            prefix.extend_from_slice(&bytes[..split]);
        }
        let mut entry = bytes[split..].to_vec();
        entry[72..80].copy_from_slice(&(n as u64 + 1).to_be_bytes());
        records.extend_from_slice(&entry);
    }
    let mut overlay = if spliced == 0 {
        StudioOverlayState::new(&basis)
    } else {
        let end = prefix.len();
        prefix[end - 12..end - 4].copy_from_slice(&(spliced as u64 + 1).to_be_bytes());
        prefix[end - 4..].copy_from_slice(&(spliced as u32).to_be_bytes());
        prefix.extend_from_slice(&records);
        StudioOverlayState::decode_vault(&prefix, &state.ledger).unwrap()
    };
    // The promoting append. Its operation is the `count`-th, so the totals every caller asserts
    // are unchanged; what changes is that the encoded record is now v2.
    let mut last = f.title();
    last.nonce = (count as u128 + 99).to_be_bytes();
    last.body = match f.target {
        StudioTarget::Index { .. } => IndexOp::SetTitle {
            object: [1; 16],
            title: format!("overlay measurement {}", count - 1),
        }
        .encode()
        .unwrap(),
        StudioTarget::Flipnote { .. } => FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!(
            "overlay measurement {}",
            count - 1
        )))
        .encode()
        .unwrap(),
    };
    let last_id = state.ledger.prepare(f.device.device_id(), last).unwrap();
    overlay
        .append(&basis, &state.ledger, last_id, spliced as u64)
        .unwrap();
    state.overlay = Some(overlay);
    let draft = state.local_draft().unwrap().unwrap();
    assert_eq!(draft.accepted(), count);
    let expected = draft.projection().clone();
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let (_, old) = store.read_epoch_intent_record(&scope, &f.logical).unwrap();
    let mut b = budget(store, f);
    store
        .write_prepared_intents(
            SERVER,
            &f.logical,
            state,
            old,
            false,
            &mut rng(),
            &mut b.storage,
            &mut b.intents,
            WriteStep::new(WriteTag::Intents),
            &mut WriteHooks::None,
        )
        .unwrap();
    let next = install(f, store, &close);
    assert_eq!((next.epoch(), next.op_count()), (1, 0));
    store.retain_studio_source(&f.group, &f.device, next);
    (basis.fingerprint(), expected)
}

fn profile(art: bool) {
    let clock = SystemClock;
    println!("OVERLAY_PROFILE pid={} art={art}", std::process::id());
    for count in [1, 32, 256] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let start = clock.monotonic_ms();
        let (basis, expected) = fixture(&f, &mut store, count);
        println!(
            "OVERLAY_PROFILE count={count} setup_ms={}",
            clock.monotonic_ms() - start
        );

        let start = clock.monotonic_ms();
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        let decode_ms = clock.monotonic_ms() - start;
        let start = clock.monotonic_ms();
        let draft = state.local_draft().unwrap().unwrap();
        let draft_ms = clock.monotonic_ms() - start;
        assert_eq!(draft.accepted(), count);
        assert_eq!(draft.projection(), &expected);
        let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
        let intent_bytes = fs::metadata(store.epoch_intent_path(&scope)).unwrap().len();
        let source_bytes = fs::metadata(f.path(&store)).unwrap().len();

        // Already authenticated/restored source: this isolates existing private candidate work
        // (including typed admission, signing and manifest creation), not disk or inventory.
        let mut source = f.load(&store).unwrap().unit;
        let start = clock.monotonic_ms();
        let candidate = state
            .handoff_metadata()
            .unwrap()
            .prepare_handoff(
                &mut source,
                &state.ledger,
                &f.device,
                &f.group,
                0,
                &mut rng(),
            )
            .unwrap();
        let candidate_ms = clock.monotonic_ms() - start;
        let (candidate, _) = candidate.into_parts();
        assert_eq!(candidate.op_count(), count);
        assert_eq!(candidate.projection().unwrap(), expected);
        drop(candidate);
        drop(source);
        drop(state);
        drop(draft);

        let start = clock.monotonic_ms();
        let mut b = budget(&mut store, &f);
        let inventory_ms = clock.monotonic_ms() - start;
        let start = clock.monotonic_ms();
        let outcome = store
            .handoff_studio_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                basis,
                Some(0),
                &mut rng(),
                &mut b,
            )
            .unwrap();
        let handoff_ms = clock.monotonic_ms() - start;
        assert_eq!(outcome.accepted, count);
        let saved_bytes = fs::read(f.path(&store)).unwrap();
        drop(store);
        let mut store = open(root.path());
        let actual = f.load(&store).unwrap().unit;
        assert_eq!((actual.epoch(), actual.op_count()), (1, count));
        assert_eq!(actual.projection().unwrap(), expected);
        let saved = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        assert!(saved.overlay().is_none());
        assert_eq!(saved.pending().len(), count);
        let mut b = budget(&mut store, &f);
        let start = clock.monotonic_ms();
        let retry = store
            .handoff_studio_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                basis,
                None,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        let retry_ms = clock.monotonic_ms() - start;
        assert_eq!(retry, outcome);
        assert_eq!(fs::read(f.path(&store)).unwrap(), saved_bytes);
        println!(
            "OVERLAY_PROFILE art={art} count={count} intent_bytes={intent_bytes} source_bytes={source_bytes} decode_ms={decode_ms} draft_ms={draft_ms} candidate_ms={candidate_ms} inventory_ms={inventory_ms} handoff_ms={handoff_ms} retry_ms={retry_ms}"
        );
    }
}

/// The handoff, stage by stage, for C-3 runtime design 15.7's step 2: what H5's commit costs on
/// its own, after design 9.1 removed its restores.
///
/// H1 and H5 run under custody and H2 and H4 are detached, so only H1, the H3 slices and H5 count
/// against a visit. H5 is timed alone, with its budget minted beforehand and reported separately:
/// in the scheduled runtime each visit mints a fresh one, and that inventory is C-3's cost, not
/// the commit's. Each count is repeated `TRIALS` times on a fresh vault, and the spread is
/// reported, because a single millisecond sample says little.
const STAGE_TRIALS: usize = 5;

/// One handoff, H1 to H5, each stage's milliseconds pushed into its slot of `samples`: H1, H2,
/// H3, H4, H5's budget inventory, H5. Requires the commit to accept `expected` operations.
///
/// Slots 6 to 8 price one each of the structure-driven terms H5 repeats under custody (design 18.3
/// review, F3), timed on the commit before H5 consumes it: a full candidate `snapshot()` encode
/// (H5 does three), a candidate `blob_cids()` projection (at least three), and one seed graph
/// load through `base_blob_cids()` (two). They are not subtracted from slot 5; they say how much
/// of it reusing H4's bytes or caching the projections could remove.
fn time_stages(
    f: &Fixture,
    store: &mut ServerStore,
    basis: [u8; 32],
    expected: usize,
    samples: &mut [Vec<u64>; 9],
) {
    let clock = SystemClock;
    let mut timed = |slot: usize, start: u64| {
        samples[slot].push(clock.monotonic_ms().saturating_sub(start));
    };
    let mut b = budget(store, f);
    let start = clock.monotonic_ms();
    let started = store
        .start_studio_handoff_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            Some(0),
            &mut rng(),
            &mut b,
            &mut WriteHooks::None,
        )
        .unwrap();
    timed(0, start);
    let crate::store::StudioHandoffStart::Captured(capture) = started else {
        panic!("the fixture branch settled instead of capturing");
    };
    let start = clock.monotonic_ms();
    let mut plan = capture.prepare().unwrap();
    timed(1, start);
    let start = clock.monotonic_ms();
    assert!(plan
        .sign_slice(&f.device, &f.group, 0, false, usize::MAX, None)
        .unwrap()
        .complete());
    timed(2, start);
    let start = clock.monotonic_ms();
    let mut commit = plan.assemble().unwrap();
    timed(3, start);
    let start = clock.monotonic_ms();
    // `snapshot` takes `&mut` (an Automerge save). H5 calls it on this same unit three times.
    let encoded = commit.candidate.snapshot().unwrap();
    timed(6, start);
    assert_eq!(
        encoded.as_slice(),
        commit.snapshot.as_slice(),
        "H4's bytes are not the candidate's encoding"
    );
    let start = clock.monotonic_ms();
    commit.candidate.blob_cids().unwrap();
    timed(7, start);
    let start = clock.monotonic_ms();
    if let Some(overlay) = commit.prepared.overlay() {
        overlay.base_blob_cids().unwrap();
    }
    timed(8, start);
    let start = clock.monotonic_ms();
    let mut b = budget(store, f);
    timed(4, start);
    let start = clock.monotonic_ms();
    let outcome = store
        .commit_studio_handoff_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            commit,
            Some(0),
            &mut rng(),
            &mut b,
            &mut WriteHooks::None,
        )
        .unwrap();
    timed(5, start);
    assert_eq!(outcome.accepted, expected);
}

/// Print one shape's spreads.
fn report_stages(shape: &str, count: usize, source_bytes: u64, samples: [Vec<u64>; 9]) {
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let [h1, h2, h3, h4, inventory, h5, encode, cids, seed] = samples.map(|s| Spread::of(&s, 1));
    println!(
        "HANDOFF_STAGES build={build} shape={shape} count={count} source_bytes={source_bytes} trials={STAGE_TRIALS} units=min/upper_median/max_us(zero_samples raw_upper_median_ms) h1={h1} h2_detached={h2} h3_signing={h3} h4_detached={h4} h5_inventory={inventory} h5_commit={h5} one_snapshot_encode={encode} one_blob_cids={cids} one_seed_graph={seed}"
    );
}

/// Title-only branches of `count` operations, on the fixture's small source.
fn stages(art: bool) {
    for count in [1, 32, 256] {
        let mut samples: [Vec<u64>; 9] = Default::default();
        let mut source_bytes = 0;
        for _ in 0..STAGE_TRIALS {
            let root = tempfile::tempdir().unwrap();
            let f = Fixture::new(art);
            let mut store = open(root.path());
            let (basis, _) = fixture(&f, &mut store, count);
            time_stages(&f, &mut store, basis, count, &mut samples);
            source_bytes = fs::metadata(f.path(&store)).unwrap().len();
        }
        let shape = if art {
            "flipnote_titles"
        } else {
            "index_titles"
        };
        report_stages(shape, count, source_bytes, samples);
    }
}

/// A Flipnote branch of `frames` frame insertions, each naming a distinct stored blob, chained one
/// after another. What H5 still walks after 9.1 (the candidate's projection and blob references)
/// grows with frames, not with titles, so this is the shape the commit phase must be priced at.
///
/// Frame ids come from a wide counter, never from `n as u8`: the base frame is `[1; 16]`
/// (`Fixture::insert`), and a byte counter wraps onto it within a full-length branch. The blob
/// tint is a byte, so at most 256 frames get distinct blobs.
fn frame_branch(f: &Fixture, store: &mut ServerStore, frames: usize) -> [u8; 32] {
    assert!(
        frames <= 256,
        "blob tints are a byte; frames would share a blob"
    );
    let (close, basis) = closing(f, store);
    let mut after = [1u8; 16];
    for n in 0..frames {
        let (cid, bytes) = published_pix(store, f, n as u8);
        let frame = (n as u128 + 10_000).to_be_bytes();
        let mut op = f.domain(
            FlipnoteOp::InsertFrame {
                frame,
                after: Some(after),
                cid,
                bytes,
            }
            .encode()
            .unwrap(),
            0,
        );
        op.nonce = (n as u128 + 5_000).to_be_bytes();
        save(f, store, &close, basis.fingerprint(), op, 123);
        after = frame;
    }
    let next = install(f, store, &close);
    store.retain_studio_source(&f.group, &f.device, next);
    basis.fingerprint()
}

/// An Index branch whose PutObjects name `objects` distinct existing Flipnotes, each given one
/// title edit so it holds work. H5 reads each referenced object's record once (9.1.1, A1).
///
/// The base Index already lists one object (`Fixture::insert` puts `[1; 16]`), and admission caps
/// an Index at `MAX_INDEX_OBJECTS` across base and branch, so a branch holds at most one fewer.
fn index_branch(f: &Fixture, store: &mut ServerStore, objects: usize) -> [u8; 32] {
    assert!(objects < catcoms_replication::studio::MAX_INDEX_OBJECTS);
    let (close, basis) = closing(f, store);
    for n in 0..objects {
        let object = [n as u8 + 20; 16];
        let mut op = f.domain(
            IndexOp::PutObject {
                object,
                kind: StudioKind::Flipnote,
                title: format!("object {n}"),
                created_by: f.device.device_id(),
                ts: 123,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap(),
            0,
        );
        op.nonce = (n as u128 + 6_000).to_be_bytes();
        save(f, store, &close, basis.fingerprint(), op, 123);
    }
    install(f, store, &close);
    for n in 0..objects {
        let referenced = StudioTarget::Flipnote {
            channel: f.target.channel(),
            object: [n as u8 + 20; 16],
        };
        let logical = referenced.document(&f.group.group_id()).unwrap();
        let body = DomainOp {
            doc_type: logical.doc_type,
            logical_key: logical.logical_key.clone(),
            nonce: (n as u128 + 7_000).to_be_bytes(),
            body: FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!("object {n}")))
                .encode()
                .unwrap(),
        };
        let mut b = budget(store, f);
        store
            .edit_studio_epoch(
                SERVER,
                &f.group,
                referenced,
                epoch_zero_id(logical.doc_type, &logical.logical_key),
                &f.device,
                body,
                123,
                &mut rng(),
                &mut b,
            )
            .unwrap();
    }
    basis.fingerprint()
}

/// The shapes C-3 runtime design 15.7's step 2 asks for beyond title-only branches: frame
/// insertions up to a full branch, and an Index branch at its object ceiling.
fn heavy_stages() {
    for frames in [32, 128, catcoms_replication::studio::MAX_STUDIO_OVERLAY_OPS] {
        let mut samples: [Vec<u64>; 9] = Default::default();
        let mut source_bytes = 0;
        for _ in 0..STAGE_TRIALS {
            let root = tempfile::tempdir().unwrap();
            let f = Fixture::new(true);
            let mut store = open(root.path());
            let basis = frame_branch(&f, &mut store, frames);
            time_stages(&f, &mut store, basis, frames, &mut samples);
            source_bytes = fs::metadata(f.path(&store)).unwrap().len();
        }
        report_stages("flipnote_frames", frames, source_bytes, samples);
    }
    for objects in [16, catcoms_replication::studio::MAX_INDEX_OBJECTS - 1] {
        let mut samples: [Vec<u64>; 9] = Default::default();
        let mut source_bytes = 0;
        for _ in 0..STAGE_TRIALS {
            let root = tempfile::tempdir().unwrap();
            let f = Fixture::new(false);
            let mut store = open(root.path());
            let basis = index_branch(&f, &mut store, objects);
            time_stages(&f, &mut store, basis, objects, &mut samples);
            source_bytes = fs::metadata(f.path(&store)).unwrap().len();
        }
        report_stages("index_put_objects", objects, source_bytes, samples);
    }
}

#[test]
#[ignore = "opt-in custody measurement, not a latency acceptance test"]
fn profile_studio_overlay_handoff_stages() {
    stages(false);
    stages(true);
    heavy_stages();
}

/// Not ignored: the heavy shapes the stage profile measures must build and hand off cleanly at a
/// small size, so a fixture that stops being valid fails here rather than in an opt-in run.
#[test]
fn studio_overlay_handoff_stage_profile_fixtures_hand_off() {
    let mut samples: [Vec<u64>; 9] = Default::default();
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let basis = frame_branch(&f, &mut store, 2);
    time_stages(&f, &mut store, basis, 2, &mut samples);
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let basis = index_branch(&f, &mut store, 2);
    time_stages(&f, &mut store, basis, 2, &mut samples);
    assert!(samples.iter().all(|s| s.len() == 2));
}

#[test]
#[ignore = "opt-in custody measurement, not a latency acceptance test"]
fn profile_studio_overlay_handoff_index() {
    profile(false);
}

#[test]
#[ignore = "opt-in custody measurement, not a latency acceptance test"]
fn profile_studio_overlay_handoff_flipnote() {
    profile(true);
}

// ---------------------------------------------------------------------------------------------
// Design 13.6: what C-1's structural decode actually saves.
//
// Here rather than with the C-3 profile because the fixture that produces a vault with large
// retained branches is here, and 13.6's subject is the overlay decoder.
// ---------------------------------------------------------------------------------------------

const C1_TRIALS: usize = 8;
/// Batched repetitions for the pure decoder pair, which is far below millisecond resolution per
/// call at small operation counts.
const REPETITIONS: usize = 64;
/// Fewer than `REPETITIONS` because each one performs a real read; still enough that a
/// millisecond clock resolves the per-call figure.
const C1_IO_REPETITIONS: usize = 16;

/// The C-1 oracle: the two decoders must produce **byte-identical** state on valid input.
///
/// Structural decode is not supposed to replay the branch - that is the intended difference, not
/// a defect - so this cannot compare projections. What it can require is that everything
/// structural *does* compute matches, and the strongest available form of that is re-encoding
/// both and comparing the bytes, which subsumes the basis fingerprint, author, entry ids,
/// envelopes, sequences and timestamps, the Prepared and Completed contents, the legacy flag and
/// the ledger's intents. Comparing a handful of accessors instead - as the first version did -
/// left every one of those unchecked.
///
/// **What this oracle does not do**, stated because its previous doc comment claimed otherwise:
/// it cannot catch a structural path that skips a *refusal* check. Both decoders are run on one
/// valid record, and a path that stopped verifying entry sequences, authorship, envelope hashes
/// or canonical encoding would produce identical output here while accepting records it should
/// reject - and would look faster, so a timing comparison alone would reward it. Refusal coverage
/// for the structural path lives in `catcoms-replication`'s handoff tests;
/// [`c1_structural_and_full_decode_both_refuse_a_tampered_sequence`] adds the store-level case.
fn assert_c1_agreement(scope: &[u8], full: &EpochIntentState, structural: &EpochIntentState) {
    match (full.handoff_metadata(), structural.handoff_metadata()) {
        (Some(_), Some(_)) => {}
        (None, None) => {
            panic!("the fixture has no overlay extension, so neither decoder does 13.6's work")
        }
        (a, b) => panic!(
            "one decoder produced an overlay and the other did not: full={} structural={}",
            a.is_some(),
            b.is_some()
        ),
    }
    assert_eq!(
        full.encode(scope).unwrap().to_vec(),
        structural.encode(scope).unwrap().to_vec(),
        "structural decode produced state that does not re-encode identically to the full \
         decode's, so the two disagree somewhere in the identity, ledger, entry or accounting \
         fields that structural decode is supposed to compute"
    );
    // The fixture has an active, replayable branch, so the full decode really did replay - which
    // is what makes the timing comparison below a comparison of two different amounts of work.
    //
    // This does **not** prove structural skipped the replay, and an earlier comment claiming it
    // did was wrong: `local_draft` replays on demand from the decoded state, so it returns `Some`
    // for the structural state too. That property is proven where it belongs, by the
    // replication-crate test that decodes structurally a branch the full decoder cannot replay.
    assert!(
        full.local_draft().unwrap().is_some(),
        "the full decode produced no draft, so this fixture is not exercising replay at all"
    );
}

/// 13.6, part one: the **pure** decoder pair, on identical bytes, with no I/O between them.
///
/// Timed separately from the end-to-end pair because an end-to-end figure includes a file read
/// that is identical on both sides and can conceal the decoder difference, which is the only
/// thing C-1 changed.
fn c1_pure(
    plain: &[u8],
    scope: &[u8],
    logical: &LogicalDocument,
    clock: &dyn Clock,
) -> (Spread, Spread) {
    let mut full = Vec::new();
    let mut structural = Vec::new();
    for _ in 0..C1_TRIALS {
        let t = clock.monotonic_ms();
        for _ in 0..REPETITIONS {
            EpochIntentState::decode(plain, scope, logical).unwrap();
        }
        full.push(clock.monotonic_ms().saturating_sub(t));

        let t = clock.monotonic_ms();
        for _ in 0..REPETITIONS {
            EpochIntentState::decode_structural(plain, scope, logical).unwrap();
        }
        structural.push(clock.monotonic_ms().saturating_sub(t));
    }
    (
        Spread::of(&full, REPETITIONS as u128),
        Spread::of(&structural, REPETITIONS as u128),
    )
}

/// 13.6, part two: the same comparison **end to end**, through the production entry points that
/// read the record from disk.
fn c1_end_to_end(
    store: &ServerStore,
    logical: &LogicalDocument,
    clock: &dyn Clock,
) -> (Spread, Spread) {
    let mut full = Vec::new();
    let mut structural = Vec::new();
    for _ in 0..C1_TRIALS {
        let t = clock.monotonic_ms();
        for _ in 0..C1_IO_REPETITIONS {
            store.load_epoch_intents(SERVER, logical).unwrap();
        }
        full.push(clock.monotonic_ms().saturating_sub(t));

        let t = clock.monotonic_ms();
        for _ in 0..C1_IO_REPETITIONS {
            store
                .load_epoch_intents_structural(SERVER, logical)
                .unwrap();
        }
        structural.push(clock.monotonic_ms().saturating_sub(t));
    }
    (
        Spread::of(&full, C1_IO_REPETITIONS as u128),
        Spread::of(&structural, C1_IO_REPETITIONS as u128),
    )
}

fn c1_measure(art: bool, counts: &[usize], clock: &dyn Clock, profile: &str) {
    for count in counts {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        fixture(&f, &mut store, *count);

        let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
        let record_bytes = fs::metadata(store.epoch_intent_path(&scope)).unwrap().len();
        let plain = store.read_scoped_intent_plain(&scope).unwrap().unwrap();

        // Correctness before timing: a faster path that computed less would otherwise be reported
        // as a saving.
        let full = EpochIntentState::decode(&plain.plain, &scope, &f.logical).unwrap();
        let structural =
            EpochIntentState::decode_structural(&plain.plain, &scope, &f.logical).unwrap();
        // The **branch** length, read back from the record on disk - not `pending().len()`, which
        // counts the ledger and is decoded identically by both paths whatever the overlay holds.
        // Replay cost scales with the branch, so the ledger count is the wrong axis to label with.
        assert_eq!(
            full.local_draft().unwrap().unwrap().accepted(),
            *count,
            "the record on disk does not hold a {count}-entry branch, so ops={count} mislabels it"
        );
        assert_c1_agreement(&scope, &full, &structural);
        let plain_bytes = plain.plain.len();
        drop((full, structural));

        let (pure_full, pure_structural) = c1_pure(&plain.plain, &scope, &f.logical, clock);
        let (io_full, io_structural) = c1_end_to_end(&store, &f.logical, clock);
        println!(
            "C1_PROFILE art={art} ops={count} record_bytes={record_bytes} \
             plain_bytes={plain_bytes} pure_full_us={pure_full} \
             pure_structural_us={pure_structural} io_full_us={io_full} \
             io_structural_us={io_structural} trials={C1_TRIALS} pure_reps={REPETITIONS} \
             io_reps={C1_IO_REPETITIONS} build={profile} \
             units=min/upper_median/max_us_and_zero_sample_count"
        );
    }
}

/// The harness, on the ordinary suite, asserting the **agreement** only.
///
/// Agreement is a correctness property and machine-independent, so it belongs here; duration does
/// not, and nothing here asserts any.
#[test]
fn c1_structural_and_full_decode_agree_on_everything_structural_computes() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    fixture(&f, &mut store, 8);
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let plain = store.read_scoped_intent_plain(&scope).unwrap().unwrap();

    let full = EpochIntentState::decode(&plain.plain, &scope, &f.logical).unwrap();
    let structural = EpochIntentState::decode_structural(&plain.plain, &scope, &f.logical).unwrap();
    assert_eq!(full.local_draft().unwrap().unwrap().accepted(), 8);

    // The record must be **v2**, the format production writes, not the v1 compatibility format.
    //
    // This is the assertion that would have caught the original defect. A purely spliced fixture
    // encodes as v1, whose decode branch hard-codes `prepared`, `completed` and
    // `minimum_new_basis_closed_epoch` - so the v2 header parse and `validate()` never ran, the
    // oracle compared constants, and the structural side did half the entry passes a real record
    // costs. Checking the version byte is the difference between measuring the format callers
    // have and measuring a compatibility path.
    let re_encoded = full
        .handoff_metadata()
        .unwrap()
        .encode_vault(&full.ledger)
        .unwrap();
    assert_ne!(
        re_encoded.first(),
        Some(&1),
        "the fixture produced a legacy v1 overlay record, so this measures the compatibility \
         decode rather than the one production writes"
    );

    assert_c1_agreement(&scope, &full, &structural);

    // And through the production entry points, so the agreement is a property of the paths
    // callers actually use rather than only of the decoders.
    let loaded_full = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let loaded_structural = store
        .load_epoch_intents_structural(SERVER, &f.logical)
        .unwrap();
    assert_c1_agreement(&scope, &loaded_full, &loaded_structural);
}

/// The refusal case the agreement oracle structurally cannot provide.
///
/// Comparing two decoders' output on one valid record says nothing about what either refuses. A
/// structural path that stopped checking entry sequences would agree on every valid input, run
/// faster, and accept a branch whose entries are out of order - so the timing comparison would
/// reward it. This corrupts the sequence field of the last spliced entry and requires **both**
/// decoders to refuse, which is what makes `decode_structural`'s "same checks, minus the replay"
/// claim testable at the store layer rather than only in `catcoms-replication`.
#[test]
fn c1_structural_and_full_decode_both_refuse_a_tampered_sequence() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    fixture(&f, &mut store, 8);
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let plain = store.read_scoped_intent_plain(&scope).unwrap().unwrap();

    // Control: untouched bytes decode both ways.
    EpochIntentState::decode(&plain.plain, &scope, &f.logical)
        .expect("control: the untampered record must decode fully");
    EpochIntentState::decode_structural(&plain.plain, &scope, &f.logical)
        .expect("control: the untampered record must decode structurally");

    // Locate the last entry's `sequence` by **searching for it**, not by indexing back from the
    // end of the record.
    //
    // An earlier version computed `end - 88 + 72`, which was right only while the final 88-byte
    // entry ended the buffer. Promoting the fixture to v2 appended a Completed-presence byte
    // after the nested active branch, so that index moved one byte earlier and selected the last
    // seven bytes of `sequence` plus the **first byte of `ts`**. Adding to it left the sequence at
    // 8 and turned the timestamp into 0x0700000000000007, far past `MAX_STUDIO_INTEGER` - so both
    // decoders refused, through timestamp validation, and the test passed while proving nothing
    // about the consecutive-sequence check it is named for. That is the third masked refusal in
    // this work, and the second written after the lesson was recorded.
    //
    // An entry is `id`(4+32) `envelope`(4+32) `sequence`(8) `ts`(8). The last one carries sequence
    // 8 and ts 7 for this fixture: seven spliced entries take sequences 1 to 7, the header is
    // patched to 8, and the promoting append lands there with `ts = spliced`. Searching for that
    // exact 16-byte pair is specific enough to be unique and fails loudly if the shape changes,
    // which a fixed offset cannot.
    let expected_sequence: u64 = 8;
    let expected_ts: u64 = 7;
    let mut needle = expected_sequence.to_be_bytes().to_vec();
    needle.extend_from_slice(&expected_ts.to_be_bytes());
    let found: Vec<usize> = plain
        .plain
        .windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle.as_slice())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected exactly one (sequence {expected_sequence}, ts {expected_ts}) pair in the \
         record, found {}. The entry layout or the fixture's shape has changed, and tampering \
         would corrupt the wrong field",
        found.len()
    );
    let seq = found[0];

    let mut tampered = plain.plain.to_vec();
    let bumped = expected_sequence.wrapping_add(7);
    tampered[seq..seq + 8].copy_from_slice(&bumped.to_be_bytes());

    // The timestamp must be untouched, or this is not a sequence test. This assertion is the one
    // that would have caught the defect above.
    assert_eq!(
        u64::from_be_bytes(tampered[seq + 8..seq + 16].try_into().unwrap()),
        expected_ts,
        "the tamper moved the timestamp, so a refusal could come from timestamp validation \
         instead of the sequence check"
    );
    assert_eq!(
        tampered.len(),
        plain.plain.len(),
        "the tamper changed the record's length"
    );
    assert_ne!(tampered, plain.plain.to_vec(), "the tamper changed nothing");

    assert!(
        EpochIntentState::decode(&tampered, &scope, &f.logical).is_err(),
        "the full decode accepted an out-of-order branch"
    );
    assert!(
        EpochIntentState::decode_structural(&tampered, &scope, &f.logical).is_err(),
        "structural decode accepted an out-of-order branch, so it is not performing the entry \
         checks its contract claims - and it would look faster for it"
    );
}

#[test]
#[ignore = "opt-in design 13.6 measurement of C-1's structural decode; no machine-speed assertion"]
fn profile_c1_structural_decode() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    c1_measure(true, &[1, 32, 256], &SystemClock, profile);
    c1_measure(false, &[256], &SystemClock, profile);
}
