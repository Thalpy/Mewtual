//! Actual signed Studio history and durable warm ingest. Initial disk installation alone is
//! batched through the private test seam, as in the existing registry profiling harness.
use super::*;
use automerge::transaction::{CommitOptions, Transactable};
use automerge::{AutoCommit, Change, ROOT};
use catcoms_replication::epoch::MAX_EPOCH_BYTES;
use catcoms_replication::studio::StudioOverlaySave;
use catcoms_replication::CloseRecord;
use catcoms_rt::{Clock, ManualClock, SystemClock};

use crate::store::measure::Spread;

fn title_op(target: StudioTarget, n: usize) -> DomainOp {
    let logical = target.document(b"fixture-type-and-key").unwrap();
    let mut nonce = [0; 16];
    nonce[..8].copy_from_slice(&(n as u64).to_be_bytes());
    DomainOp {
        nonce,
        doc_type: logical.doc_type,
        logical_key: logical.logical_key,
        body: FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!("frame history {n}")))
            .encode()
            .unwrap(),
    }
}

fn candidate(
    writer: &AutoCommit,
    group: &ServerGroup,
    device: &MlsDevice,
    target: StudioTarget,
    n: usize,
    message: usize,
) -> (AutoCommit, SealedOp, usize) {
    let domain = title_op(target, n);
    // Canonical frame-header record. Production signed/causal/type/preflight validation below
    // verifies these fixture bytes; they are never installed as an unchecked CRDT snapshot.
    let mut record = vec![1];
    record.extend_from_slice(device.device_id().as_bytes());
    record.extend_from_slice(&100u64.to_be_bytes());
    record.extend_from_slice(&[0, 0]); // no frame insertion anchors for a title
    record.extend_from_slice(&domain.encode().unwrap());
    let mut next = writer.clone();
    next.put(ROOT, "h/title", record).unwrap();
    next.put(
        ROOT,
        format!("_p1/op/{}", hex::encode(domain.id(&device.device_id()))),
        1u64,
    )
    .unwrap();
    next.commit_with(CommitOptions::default().with_message("x".repeat(message)));
    let logical = target.document(&group.group_id()).unwrap();
    let signed = SignedOp::sign_domain(
        device,
        logical.doc_type,
        epoch_zero_id(logical.doc_type, &logical.logical_key),
        next.get_last_local_change().unwrap().raw_bytes().to_vec(),
        &domain,
    )
    .unwrap();
    let bytes = signed.encode().len();
    (
        next,
        SealedOp::seal(&signed, group, device, &mut rng()).unwrap(),
        bytes,
    )
}

fn build(
    group: &ServerGroup,
    device: &MlsDevice,
    target: StudioTarget,
    count: usize,
    message: usize,
) -> (StudioEpoch, AutoCommit, Vec<SealedOp>) {
    let mut unit = StudioEpoch::new(group, target, device.device_id()).unwrap();
    let first = unit
        .edit_or_reseal(device, group, &mut rng(), &title_op(target, 0), 100)
        .unwrap();
    let key = group
        .channel_secret(device, first.doc_type, first.doc_id)
        .unwrap();
    let signed = first.open(&key).unwrap();
    let mut bytes = signed.encode().len();
    let mut writer = AutoCommit::new().with_actor(automerge::ActorId::from(
        device.device_id().as_bytes().to_vec(),
    ));
    writer
        .apply_changes([Change::from_bytes(signed.delta).unwrap()])
        .unwrap();
    let mut operations = vec![first];
    for n in 1..count {
        // Leave room for the actual three NEW measured edits; a full epoch can only refuse.
        if bytes >= MAX_EPOCH_BYTES - 64 * 1024 {
            break;
        }
        let (next, sealed, size) = candidate(&writer, group, device, target, n, message);
        assert_eq!(
            unit.ingest(&sealed, group, device).unwrap(),
            Admission::Accepted
        );
        writer = next;
        bytes += size;
        operations.push(sealed);
        if n % 1024 == 0 {
            println!("STUDIO_PROFILE setup_ops={} signed_bytes={bytes}", n + 1);
        }
    }
    (unit, writer, operations)
}

/// Real >256-KiB source for two-member receive tests. The returned historical packets let the
/// second member independently validate/persist the same history, not copy another vault file.
pub(crate) fn save_studio_source_fixture(
    store: &mut ServerStore,
    server: u64,
    group: &ServerGroup,
    device: &MlsDevice,
    target: StudioTarget,
) -> Vec<SealedOp> {
    save_studio_source_fixture_ops(store, server, group, device, target, 3, 160_000)
}

/// `save_studio_source_fixture` with the shape exposed.
///
/// `pub(crate)` for design 13.7's C-3 profile: Studio is one of the two families whose expensive
/// typed reconstruction motivated C-3, and measuring it needs an operation-count axis rather
/// than the single fixed shape the wrapper above pins.
/// A Studio source whose operations name real pixels, for design 13.7's reference-mode profile.
///
/// The title-header sources the other builders produce name **no CIDs at all**, so timing a
/// reference scan over them times an empty collection. This inserts `frames` frames, each naming
/// a distinct blob that is actually stored, and returns those CIDs so a measurement can assert
/// the collected set rather than assume it.
/// `distinct_cids` is what makes frame count and reference count **independent axes**.
///
/// Every frame still names a pixel, but the pixels repeat cyclically once `distinct_cids` is
/// exhausted, so a 128-frame source can hold 1 distinct reference or 128. Without that, frame
/// count, reference count and encoded bytes all rise together and no measurement can say which
/// drives the cost. Passing `distinct_cids == frames` gives the all-distinct case.
///
/// Returns the **distinct** CIDs planted, not one per frame, so a caller comparing collected
/// references against this set is comparing like with like.
pub(crate) fn save_studio_frame_fixture(
    store: &mut ServerStore,
    server: u64,
    group: &ServerGroup,
    device: &MlsDevice,
    target: StudioTarget,
    frames: usize,
    distinct_cids: usize,
) -> (Vec<catcoms_storage::Cid>, usize) {
    // `frames == 0` is a legal empty source and needs no pixels; the range only has to hold when
    // there are frames to name them. Stating it as `1..=frames` unconditionally made the bound
    // unsatisfiable at zero, with a message that read as nonsense there.
    assert!(
        frames == 0 || (1..=frames).contains(&distinct_cids),
        "with {frames} frames, distinct_cids must be in 1..={frames}, got {distinct_cids}"
    );
    let logical = target.document(&group.group_id()).unwrap();
    let mut blobs = store.blob_store(&hex::encode(group.group_id())).unwrap();
    let mut pixels = Vec::new();
    for n in 0..distinct_cids {
        pixels.push(blobs.put(&(n as u64).to_be_bytes()).unwrap());
    }
    let planted = pixels.clone();
    let mut unit = StudioEpoch::new(group, target, device.device_id()).unwrap();
    for n in 0..frames {
        let cid = pixels[n % distinct_cids];
        let mut nonce = [0; 16];
        nonce[..8].copy_from_slice(&(n as u64).to_be_bytes());
        let mut frame = [0; 16];
        frame[..8].copy_from_slice(&(n as u64).to_be_bytes());
        unit.edit_or_reseal(
            device,
            group,
            &mut rng(),
            &DomainOp {
                nonce,
                doc_type: logical.doc_type,
                logical_key: logical.logical_key.clone(),
                body: FlipnoteOp::InsertFrame {
                    frame,
                    after: None,
                    cid: *cid.as_bytes(),
                    bytes: 8,
                }
                .encode()
                .unwrap(),
            },
            100,
        )
        .unwrap();
    }
    drop(blobs);
    let accepted = unit.op_count();
    let inv = inventory(store);
    let mut b = store.studio_storage_budget(server, group, &inv).unwrap();
    store
        .save_studio_source(
            server,
            unit,
            None,
            &[],
            WritePurpose::Ordinary,
            &mut rng(),
            &mut b.storage,
            WriteStep::new(WriteTag::Source),
            &mut WriteHooks::None,
        )
        .unwrap();
    (planted, accepted)
}

pub(crate) fn save_studio_source_fixture_ops(
    store: &mut ServerStore,
    server: u64,
    group: &ServerGroup,
    device: &MlsDevice,
    target: StudioTarget,
    count: usize,
    message: usize,
) -> Vec<SealedOp> {
    let (unit, _, operations) = build(group, device, target, count, message);
    let inv = inventory(store);
    let mut b = store.studio_storage_budget(server, group, &inv).unwrap();
    store
        .save_studio_source(
            server,
            unit,
            None,
            &[],
            WritePurpose::Ordinary,
            &mut rng(),
            &mut b.storage,
            WriteStep::new(WriteTag::Source),
            &mut WriteHooks::None,
        )
        .unwrap();
    operations
}

/// Fill the actual current epoch (including a receipt-opened successor) with accepted signed
/// edits. Only initial fixture disk writes are batched; normal typed admission and production
/// close limits still apply. This allows runtime tests to rotate repeatedly without 10,000 ops.
pub(crate) fn fill_studio_epoch_fixture(
    store: &mut ServerStore,
    server: u64,
    group: &ServerGroup,
    device: &MlsDevice,
    target: StudioTarget,
) {
    let inv = inventory(store);
    let mut budget = store.studio_storage_budget(server, group, &inv).unwrap();
    let (mut unit, observed, before) = store
        .checked_studio_source(server, group, target, device, true, &mut budget.storage)
        .unwrap();
    let logical = unit.document().clone();
    for n in 0..10 {
        let mut op = title_op(target, n);
        if let StudioProjection::Index(index) = unit.projection().unwrap() {
            op.body = if index.objects.contains_key(&[7; 16]) {
                IndexOp::SetTitle {
                    object: [7; 16],
                    title: format!("index history {n}"),
                }
            } else {
                IndexOp::PutObject {
                    object: [7; 16],
                    kind: catcoms_replication::studio::StudioKind::Flipnote,
                    title: format!("index history {n}"),
                    created_by: device.device_id(),
                    ts: 100,
                    expiry: catcoms_replication::studio::StudioExpiry::Unrecorded,
                }
            }
            .encode()
            .unwrap();
        }
        op.nonce[8..].copy_from_slice(&unit.epoch().to_be_bytes());
        let mut copy =
            StudioEpoch::restore(&unit.snapshot().unwrap(), group, target, device.device_id())
                .unwrap();
        let packet = copy
            .edit_or_reseal(device, group, &mut rng(), &op, 100)
            .unwrap();
        let key = group
            .channel_secret(device, packet.doc_type, packet.doc_id)
            .unwrap();
        let mut change = Change::from_bytes(packet.open(&key).unwrap().delta)
            .unwrap()
            .decode();
        change.message = Some("x".repeat(220_000));
        let signed = SignedOp::sign_domain(
            device,
            logical.doc_type,
            unit.doc_id(),
            Change::from(change).raw_bytes().to_vec(),
            &op,
        )
        .unwrap();
        let packet = SealedOp::seal(&signed, group, device, &mut rng()).unwrap();
        assert_eq!(
            unit.ingest(&packet, group, device).unwrap(),
            Admission::Accepted
        );
    }
    assert!(unit.close_candidate_ready());
    let state = store
        .save_studio_source(
            server,
            unit,
            observed,
            &before,
            WritePurpose::Ordinary,
            &mut rng(),
            &mut budget.storage,
            WriteStep::new(WriteTag::Source),
            &mut WriteHooks::None,
        )
        .unwrap();
    store.retain_studio_source(group, device, state);
}

/// An actual eligible close over the retained source, signed by its current owner. Runtime
/// takeover tests use this only to arrange an interrupted old-owner seal, never to manufacture
/// the successor owner's receipt or bypass the ordinary idle-worker installation.
pub(crate) fn studio_owner_decision_fixture(
    store: &ServerStore,
    server: u64,
    group: &ServerGroup,
    owner: &MlsDevice,
    target: StudioTarget,
    previous: Option<&Receipt>,
) -> catcoms_replication::studio::StudioOwnerDecision {
    store
        .load_studio_epoch(server, group, target, owner)
        .unwrap()
        .unwrap()
        .unit
        .new_owner_decision(group, owner, 0, previous)
        .unwrap()
}

fn measure(count: usize, clock: &dyn Clock) {
    let f = Fixture::new(true);
    let start = clock.monotonic_ms();
    let (unit, mut writer, operations) = build(&f.group, &f.device, f.target, count, 0);
    let setup = clock.monotonic_ms().saturating_sub(start);
    let initial = operations.len();
    drop(operations);
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    store
        .save_studio_source(
            SERVER,
            unit,
            None,
            &[],
            WritePurpose::Ordinary,
            &mut rng(),
            &mut b.storage,
            WriteStep::new(WriteTag::Source),
            &mut WriteHooks::None,
        )
        .unwrap();
    let bytes = fs::metadata(f.path(&store)).unwrap().len();
    let start = clock.monotonic_ms();
    let state = f.load(&store).unwrap();
    let cold = clock.monotonic_ms().saturating_sub(start);
    store.retain_studio_source(&f.group, &f.device, state);
    println!("STUDIO_PROFILE ops={initial} physical_bytes={bytes} setup_ms={setup} cold_restore_ms={cold}");
    let restores = crate::store::studio_full_restores_for_test();
    for n in initial..initial + 3 {
        let (next, sealed, _) = candidate(&writer, &f.group, &f.device, f.target, n, 0);
        let start = clock.monotonic_ms();
        let mut scan = store.scan_studio_receive_inventory().unwrap();
        let p = loop {
            let p = scan.step().unwrap();
            if p.complete {
                break p;
            }
        };
        assert_eq!(p.reused_records, 1);
        assert_eq!(p.uncached_bytes, 0);
        let inv = scan.finish().unwrap();
        let mut b = store.studio_storage_budget(SERVER, &f.group, &inv).unwrap();
        let (admission, state) = store
            .ingest_studio_epoch_reusing(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &sealed,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        assert_eq!(admission, Admission::Accepted);
        assert_eq!(state.op_count(), n + 1);
        store.retain_studio_source(&f.group, &f.device, state);
        println!(
            "STUDIO_PROFILE pass={} warm_inventory_ingest_save_ms={}",
            n - initial,
            clock.monotonic_ms().saturating_sub(start)
        );
        assert_eq!(crate::store::studio_full_restores_for_test(), restores);
        writer = next;
    }
    drop(store);
    let store = open(root.path());
    assert_eq!(f.load(&store).unwrap().op_count(), initial + 3);
}

/// A real Flow S capture, produced entirely through the production path: source, owner decision,
/// seal, basis, then `start_studio_closing_overlay` returning `Captured`. The runtime tests need a
/// genuine `StudioOverlayCapture` and must not fabricate one.
///
/// `exhaust_intents` pre-fills the document's ordinary intent ledger to `MAX_INTENT_BYTES_PER_
/// DOCUMENT`, which makes the capture's later `plan()` refuse at `IntentLedger::prepare`. That is
/// a real refusal on a real capture: classification never calls `prepare`, so it is reachable only
/// in the detached stage, which is exactly the RT-001 case. The pre-fill uses the production
/// `prepare`, `encode`, `seal` and framing at the canonical path, as the existing per-document cap
/// test does, so nothing it produces bypasses a check the reader performs.
pub(crate) fn studio_closing_capture_fixture(
    store: &mut ServerStore,
    server: u64,
    group: &ServerGroup,
    device: &MlsDevice,
    target: StudioTarget,
    exhaust_intents: bool,
) -> crate::store::StudioOverlayCapture {
    // Creates the source and fills it to rotation eligibility, which an owner decision requires.
    fill_studio_epoch_fixture(store, server, group, device, target);
    let decision = studio_owner_decision_fixture(store, server, group, device, target, None);
    let close = decision.close().clone();
    let mut b = {
        let inv = inventory(store);
        store.studio_storage_budget(server, group, &inv).unwrap()
    };
    store
        .seal_studio_epoch(
            server,
            group,
            target,
            device,
            decision.receipt().clone(),
            0,
            &mut ChaCha20Rng::seed_from_u64(7),
            &mut b,
        )
        .unwrap();
    let basis = store
        .prepare_studio_closing_overlay(server, group, target, device, &close, Some(0), &mut b)
        .unwrap();

    let logical = target.document(&group.group_id()).unwrap();
    if exhaust_intents {
        let mut ledger = catcoms_replication::IntentLedger::new(logical.clone());
        let blank = title_op(target, 0);
        let overhead = blank.encode().unwrap().len() - blank.body.len();
        let mut left = catcoms_replication::epoch::MAX_INTENT_BYTES_PER_DOCUMENT;
        let mut n = 0u128;
        while left > 0 {
            let len = left.min(catcoms_replication::epoch::MAX_DOMAIN_OP_BYTES);
            let mut next = title_op(target, 0);
            next.nonce = n.to_be_bytes();
            next.body = vec![b'x'; len - overhead];
            ledger.prepare(device.device_id(), next).unwrap();
            left -= len;
            n += 1;
        }
        let scope = crate::store::epoch_intents::scope_bytes(server, &logical).unwrap();
        let state = crate::store::epoch_intents::EpochIntentState {
            ledger,
            overlay: None,
        };
        let sealed = seal(
            &store.keys.db_key().unwrap(),
            &state.encode(&scope).unwrap(),
            &mut ChaCha20Rng::seed_from_u64(11),
        )
        .unwrap();
        fs::write(store.epoch_intent_path(&scope), frame(&sealed)).unwrap();
    }

    let mut b = {
        let inv = inventory(store);
        store.studio_storage_budget(server, group, &inv).unwrap()
    };
    let branch = store
        .studio_overlay_request_branch(server, group, target, &basis, &mut b)
        .unwrap();
    let mut b = {
        let inv = inventory(store);
        store.studio_storage_budget(server, group, &inv).unwrap()
    };
    let mut op = title_op(target, 9_000);
    op.nonce = [77; 16];
    match store
        .start_studio_closing_overlay(
            server,
            group,
            target,
            device,
            &close,
            crate::studio::StudioOwnerTenure::Known(0),
            basis.fingerprint(),
            branch,
            op,
            300,
            &mut ChaCha20Rng::seed_from_u64(13),
            &mut b,
        )
        .unwrap()
    {
        crate::store::StudioOverlayStart::Captured(capture) => *capture,
        crate::store::StudioOverlayStart::Settled(_) => {
            panic!("the fixture request was classified as already accepted")
        }
    }
}

/// A vault left exactly where automatic handoff becomes possible: a local draft accepted on a
/// Closing document, its successor settled and installed, and the installed head completed.
/// Returns the branch's basis, which is what the H1 probe rediscovers for itself.
///
/// Every step is the production one. The runtime tests need this state and must not fabricate it.
pub(crate) fn studio_handoff_ready_fixture(
    store: &mut ServerStore,
    server: u64,
    group: &ServerGroup,
    device: &MlsDevice,
    target: StudioTarget,
    operations: usize,
) -> [u8; 32] {
    fill_studio_epoch_fixture(store, server, group, device, target);
    let decision = studio_owner_decision_fixture(store, server, group, device, target, None);
    let close = decision.close().clone();
    let mut b = {
        let inv = inventory(store);
        store.studio_storage_budget(server, group, &inv).unwrap()
    };
    store
        .seal_studio_epoch(
            server,
            group,
            target,
            device,
            decision.receipt().clone(),
            0,
            &mut ChaCha20Rng::seed_from_u64(21),
            &mut b,
        )
        .unwrap();
    let basis = store
        .prepare_studio_closing_overlay(server, group, target, device, &close, Some(0), &mut b)
        .unwrap();
    // The generation-1 branch the first Save opens; every later one appends to it.
    let branch = store
        .studio_overlay_request_branch(server, group, target, &basis, &mut b)
        .unwrap();
    // `operations` accepted entries, so a caller can build a branch long enough for H3 to page
    // across background turns rather than finishing in one slice.
    for n in 0..operations {
        let mut op = title_op(target, 7_100 + n);
        op.nonce = (n as u128 + 6_000).to_be_bytes();
        store
            .save_studio_closing_overlay(
                server,
                group,
                target,
                device,
                &close,
                crate::studio::StudioOwnerTenure::Known(0),
                basis.fingerprint(),
                branch,
                op,
                300 + n as u64,
                &mut ChaCha20Rng::seed_from_u64(22),
                &mut b,
            )
            .unwrap();
    }

    // Persist the settlement decision to the owner journal, then rotate. Without the journal
    // entry the installed head and the journal disagree, which is the check that caught an
    // earlier version of this fixture taking a shortcut.
    let mut source = store
        .load_studio_epoch(server, group, target, device)
        .unwrap()
        .unwrap();
    let receipt = source.unit.receipt_head().unwrap().cloned().unwrap();
    let decision = source
        .unit
        .resume_owner_decision(group, device, 0, &receipt, &close)
        .unwrap();
    let mut b = {
        let inv = inventory(store);
        store.studio_storage_budget(server, group, &inv).unwrap()
    };
    store
        .prepare_studio_owner_decision_with_writer(
            server,
            &decision,
            group,
            0,
            &mut ChaCha20Rng::seed_from_u64(23),
            &mut b.storage,
            &mut WriteHooks::None,
        )
        .unwrap();

    // Re-save unchanged so the retained source carries a real physical stamp, as the rotation
    // path requires; never invent one from a normalized unit.
    let (unit, observed, before) = store
        .checked_studio_source(server, group, target, device, false, &mut b.storage)
        .unwrap();
    let logical = target.document(&group.group_id()).unwrap();
    let source_scope = scope_bytes(server, &logical).unwrap();
    let actual = store.read_studio_record(&source_scope).unwrap().unwrap();
    let version = store
        .studio_source_version(server, &unit, &actual.plain, actual.physical_bytes)
        .unwrap();
    let warmed = store
        .save_studio_source_reusing(
            server,
            unit,
            observed,
            &before,
            WritePurpose::Settlement,
            &mut ChaCha20Rng::seed_from_u64(24),
            &mut b.storage,
            WriteStep::new(WriteTag::Source),
            &mut WriteHooks::None,
            Some(version),
        )
        .unwrap();
    store.retain_studio_source(group, device, warmed);

    let mut b = {
        let inv = inventory(store);
        store.studio_storage_budget(server, group, &inv).unwrap()
    };
    let (_, installed) = store
        .rotate_studio_owner(
            server,
            group,
            target,
            device,
            0,
            &ManualClock::new(1000),
            &mut ChaCha20Rng::seed_from_u64(26),
            &mut b,
        )
        .unwrap();
    store.retain_studio_source(group, device, installed);
    let mut b = {
        let inv = inventory(store);
        store.studio_storage_budget(server, group, &inv).unwrap()
    };
    store
        .complete_studio_installed_head(
            server,
            group,
            target,
            device,
            0,
            &mut ChaCha20Rng::seed_from_u64(25),
            &mut b,
        )
        .unwrap();
    basis.fingerprint()
}

/// `studio_handoff_ready_fixture`, then a real synchronous handoff interrupted just before one of
/// its writes, leaving the durable Prepared record that Flow R resolves (design 6.4.2).
///
/// `source_written` chooses the crash: just before the Completed write, so the Source write
/// landed and the evidence is Complete; or just before the Source write, so it never happened and
/// the evidence is Absent. Returns the branch's basis.
pub(crate) fn studio_handoff_interrupted_fixture(
    store: &mut ServerStore,
    server: u64,
    group: &ServerGroup,
    device: &MlsDevice,
    target: StudioTarget,
    operations: usize,
    source_written: bool,
) -> [u8; 32] {
    let basis = studio_handoff_ready_fixture(store, server, group, device, target, operations);
    let mut b = {
        let inv = inventory(store);
        store.studio_storage_budget(server, group, &inv).unwrap()
    };
    let interrupted = store.handoff_studio_overlay_with_io(
        server,
        group,
        target,
        device,
        basis,
        Some(0),
        &mut ChaCha20Rng::seed_from_u64(27),
        &mut b,
        &mut WriteHooks::fail_before_write(FailError::Io("injected crash")).at(if source_written {
            WriteTag::Completed
        } else {
            WriteTag::Source
        }),
    );
    assert!(
        matches!(interrupted, Err(ref e) if e.to_string().contains("injected crash")),
        "the handoff was not interrupted where the fixture meant: {interrupted:?}"
    );
    assert!(store
        .load_epoch_intents(server, &target.document(&group.group_id()).unwrap())
        .unwrap()
        .handoff_prepared());
    basis
}

#[test]
fn studio_source_profile_smoke() {
    measure(33, &ManualClock::new(0));
}

#[test]
#[ignore = "opt-in release profiling of real dense Studio ingest; no machine-speed assertion"]
fn profile_studio_source_operations() {
    measure(20_000, &SystemClock);
}

/// Design 13.5's stage axis: Flow S custody per stage at 1, 32 and 255 accepted operations.
///
/// What this covers and what it does not, stated here because the obligation has two clauses.
/// **Covered:** the operation-count axis, with the custody stages separated from the detached one
/// through the same split API the scheduled runtime uses, so no second algorithm is timed. Every
/// accepted operation is produced by the production `save_studio_closing_overlay`; nothing is
/// spliced, and the depth is read back through `local_draft()` rather than trusted.
/// **Not covered:** "against a maximal Closing source and seed". The source here is
/// `fill_studio_epoch_fixture`'s, which fills to `close_candidate_ready()` - rotation eligibility,
/// not the byte ceiling - and its physical size is printed so the gap is visible rather than
/// implied. The maximal-shape clause belongs to 13.2 and is measured there or not at all.
///
/// **Why this is a curve and not three points.** `catcoms_rt::Clock` is millisecond-only, so a
/// single accept at a single depth is one tick-resolution reading and carries almost no
/// information. Every depth from 1 to 255 is therefore timed, the three depths 13.5 names are
/// reported as points on that curve, and each is accompanied by a [`Spread`] over its immediate
/// neighbourhood so a reader can see whether the point means anything. That neighbourhood is
/// **not** a spread at a fixed depth - it mixes five adjacent depths - and is labelled as such.
///
/// 255 rather than 256 is the design's number because `MAX_STUDIO_OVERLAY_OPS` is 256: 255 is the
/// largest accepted count from which a further append is still legal. `assert_overlay_headroom`
/// pins that premise instead of trusting this comment.
struct FlowSAccept {
    /// Accepted operations **after** this accept committed.
    depth: usize,
    start_ms: u64,
    plan_ms: u64,
    commit_ms: u64,
}

/// One distinct, legal operation per depth, against the **real** logical document.
///
/// Not `title_op`: that builds its `logical_key` from the fixture constant
/// `b"fixture-type-and-key"` rather than the group, and `fill_studio_epoch_fixture` has already
/// consumed its nonces 0 to 9 on the source. Reusing either collides - the first accept refused
/// with "domain-operation nonce was reused with conflicting bytes" - so the nonce here carries a
/// `flow-s` tag in its second half to stay clear of the fixture's epoch-tagged range, and the
/// body varies with `n` so no two accepts are the same operation.
fn flow_s_op(f: &Fixture, n: usize) -> DomainOp {
    let mut nonce = [0u8; 16];
    nonce[..8].copy_from_slice(&(n as u64).to_be_bytes());
    nonce[8..].copy_from_slice(b"flow-s\0\0");
    DomainOp {
        nonce,
        doc_type: f.logical.doc_type,
        logical_key: f.logical.logical_key.clone(),
        body: match f.target {
            StudioTarget::Index { .. } => IndexOp::SetTitle {
                object: [1; 16],
                title: format!("flow s {n}"),
            }
            .encode()
            .unwrap(),
            _ => FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!("flow s {n}")))
                .encode()
                .unwrap(),
        },
    }
}

/// The premise 255 rests on. If the cap ever moves, the chosen depths stop meaning what the
/// section says they mean, and this fails rather than silently measuring something else.
fn assert_overlay_headroom(depths: &[usize]) {
    use catcoms_replication::studio::MAX_STUDIO_OVERLAY_OPS;
    for &d in depths {
        assert!(
            d <= MAX_STUDIO_OVERLAY_OPS,
            "depth {d} exceeds MAX_STUDIO_OVERLAY_OPS {MAX_STUDIO_OVERLAY_OPS}"
        );
    }
    assert_eq!(
        *depths.last().unwrap() + 1,
        MAX_STUDIO_OVERLAY_OPS,
        "the deepest depth must be the last one from which a further append is still legal"
    );
}

/// A real Closing document with its basis derived: filled source, owner decision, seal.
///
/// The same sequence as `studio_closing_capture_fixture`, which cannot be reused here because it
/// consumes the first accept into a capture and this needs accepts to accumulate.
///
/// Returns the close, the basis fingerprint, the branch every accept in a curve names - the
/// generation-1 branch the first one opens and the rest append to - and the source's size.
fn flow_s_closing(store: &mut ServerStore, f: &Fixture) -> (CloseRecord, [u8; 32], [u8; 32], u64) {
    fill_studio_epoch_fixture(store, SERVER, &f.group, &f.device, f.target);
    let decision =
        studio_owner_decision_fixture(store, SERVER, &f.group, &f.device, f.target, None);
    let close = decision.close().clone();
    let mut b = budget(store, f);
    store
        .seal_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            decision.receipt().clone(),
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    let source_bytes = fs::metadata(f.path(store)).unwrap().len();
    let mut b = budget(store, f);
    let basis = store
        .prepare_studio_closing_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &close,
            Some(0),
            &mut b,
        )
        .unwrap();
    let branch = store
        .studio_overlay_request_branch(SERVER, &f.group, f.target, &basis, &mut b)
        .unwrap();
    (close, basis.fingerprint(), branch, source_bytes)
}

/// Accepted operations as the production reader sees them, not as the writer counted them.
fn flow_s_depth(store: &ServerStore, f: &Fixture) -> usize {
    store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .local_draft()
        .unwrap()
        .map(|draft| draft.accepted())
        .unwrap_or(0)
}

/// Times all three stages of every accept from depth 1 to `max_depth`, through the split API.
fn flow_s_curve(max_depth: usize, clock: &dyn Clock) -> (Vec<FlowSAccept>, u64) {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, fingerprint, branch, source_bytes) = flow_s_closing(&mut store, &f);
    let mut curve = Vec::with_capacity(max_depth);
    for n in 0..max_depth {
        let mut b = budget(&mut store, &f);
        let t = clock.monotonic_ms();
        let capture = match store
            .start_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                crate::studio::StudioOwnerTenure::Known(0),
                fingerprint,
                branch,
                flow_s_op(&f, n),
                300 + n as u64,
                &mut rng(),
                &mut b,
            )
            .unwrap()
        {
            crate::store::StudioOverlayStart::Captured(capture) => *capture,
            crate::store::StudioOverlayStart::Settled(_) => {
                panic!("a fresh operation was classified as already accepted at depth {n}")
            }
        };
        let start_ms = clock.monotonic_ms().saturating_sub(t);

        // S2. Detached in the scheduled runtime, which is why it is reported apart from custody.
        let t = clock.monotonic_ms();
        let plan = capture
            .plan()
            .unwrap_or_else(|e| panic!("S2 refused at depth {n}: {e}"));
        let plan_ms = clock.monotonic_ms().saturating_sub(t);

        let t = clock.monotonic_ms();
        let mut b = budget(&mut store, &f);
        store
            .commit_studio_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                crate::studio::StudioOwnerTenure::Known(0),
                plan,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        let commit_ms = clock.monotonic_ms().saturating_sub(t);

        curve.push(FlowSAccept {
            depth: n + 1,
            start_ms,
            plan_ms,
            commit_ms,
        });
    }
    assert_eq!(
        flow_s_depth(&store, &f),
        max_depth,
        "the curve did not reach the depth it timed"
    );
    // The basis must not have moved: local acceptance writes no source, and every accept above
    // was authorized against the one fingerprint derived before the first of them.
    let mut b = budget(&mut store, &f);
    let after = store
        .prepare_studio_closing_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &close,
            Some(0),
            &mut b,
        )
        .unwrap();
    assert_eq!(
        after.fingerprint(),
        fingerprint,
        "local acceptance moved the Closing basis"
    );
    (curve, source_bytes)
}

/// Repeated samples of the two Flow S stages that can be re-run at a fixed depth without
/// changing it: the S1b/S3 basis derivation, which writes nothing, and S0+S1 classification of an
/// accepted retry, which writes but does not append.
///
/// These are the only stages a real [`Spread`] is available for, because the accept stages move
/// the depth they are measured at.
fn flow_s_repeatable(depth: usize, repeats: usize, clock: &dyn Clock) -> (Spread, Spread) {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, fingerprint, branch, _) = flow_s_closing(&mut store, &f);
    for n in 0..depth {
        let mut b = budget(&mut store, &f);
        store
            .save_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                crate::studio::StudioOwnerTenure::Known(0),
                fingerprint,
                branch,
                flow_s_op(&f, n),
                300 + n as u64,
                &mut rng(),
                &mut b,
            )
            .unwrap();
    }
    assert_eq!(flow_s_depth(&store, &f), depth);

    let mut basis = Vec::with_capacity(repeats);
    let mut retry = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let mut b = budget(&mut store, &f);
        let t = clock.monotonic_ms();
        store
            .prepare_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                Some(0),
                &mut b,
            )
            .unwrap();
        basis.push(clock.monotonic_ms().saturating_sub(t));

        // The last accepted operation, resubmitted. `exact_retry` recognizes it and returns
        // before any source read, so this path is S0 plus S1's structural classification plus the
        // one accounted intent write the retry performs - not the whole of Flow S.
        let mut b = budget(&mut store, &f);
        let t = clock.monotonic_ms();
        let settled = store
            .save_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                crate::studio::StudioOwnerTenure::Known(0),
                fingerprint,
                branch,
                flow_s_op(&f, depth - 1),
                300 + depth as u64 - 1,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        retry.push(clock.monotonic_ms().saturating_sub(t));
        assert!(
            matches!(settled, StudioOverlaySave::Acknowledged { .. }),
            "the resubmitted operation was not classified as an accepted retry"
        );
        assert_eq!(
            flow_s_depth(&store, &f),
            depth,
            "an accepted retry changed the accepted count"
        );
    }
    (Spread::of(&basis, 1), Spread::of(&retry, 1))
}

/// A [`Spread`] over the five depths centred on `depth`, clipped to the curve.
///
/// This mixes adjacent depths deliberately: at millisecond resolution one accept at one depth is
/// a single tick-resolution reading. It is a local spread, not a spread at a fixed depth, and
/// every caller labels it that way.
fn flow_s_neighbourhood(curve: &[FlowSAccept], depth: usize) -> (Spread, Spread, Spread) {
    let lo = depth.saturating_sub(3);
    let hi = (depth + 2).min(curve.len());
    let band = &curve[lo..hi];
    (
        Spread::of(&band.iter().map(|c| c.start_ms).collect::<Vec<_>>(), 1),
        Spread::of(&band.iter().map(|c| c.plan_ms).collect::<Vec<_>>(), 1),
        Spread::of(&band.iter().map(|c| c.commit_ms).collect::<Vec<_>>(), 1),
    )
}

/// Depth 12 rather than 2, which is the whole point of the number.
///
/// The first version of this ran to depth 2 and passed, while the release profile refused on its
/// first accept with "domain-operation nonce was reused with conflicting bytes". The smoke test
/// was too shallow to reach the collision: `fill_studio_epoch_fixture` consumes `title_op`'s
/// nonces 0 to 9 on the source, so an accept sequence sharing that builder only conflicts once it
/// reaches them. 12 crosses the whole range, so the guard now fails where the profile fails
/// instead of certifying a fixture the profile cannot use.
#[test]
fn flow_s_stage_profile_smoke() {
    assert_overlay_headroom(&[1, 32, 255]);
    let (curve, source_bytes) = flow_s_curve(12, &ManualClock::new(0));
    assert_eq!(curve.len(), 12);
    assert_eq!(curve[0].depth, 1);
    assert_eq!(curve[11].depth, 12);
    // The source this axis is measured against is emphatically not maximal, and the section says
    // so. Pin it, so "not maximal" stays a fact about the fixture rather than a remark about it.
    assert!(
        source_bytes < crate::store::epoch_studio::MAX_SEALED_BYTES as u64 / 2,
        "the fixture source is {source_bytes} bytes, which is no longer the small shape 13.5 \
         reports it as - 13.2's maximal-shape clause may now be in scope"
    );
    // Depth 1 here, not 12: this call builds a second whole fixture, and the curve above already
    // crosses the colliding nonce range. What is left to check is the retry classification and
    // the spread plumbing, both of which depth 1 exercises.
    let (basis, retry) = flow_s_repeatable(1, 2, &ManualClock::new(0));
    assert_eq!(basis.samples, 2);
    assert_eq!(retry.samples, 2);
}

#[test]
#[ignore = "opt-in release profiling of Flow S stage custody; no machine-speed assertion"]
fn profile_flow_s_stages() {
    let depths = [1, 32, 255];
    assert_overlay_headroom(&depths);
    let (curve, source_bytes) = flow_s_curve(*depths.last().unwrap(), &SystemClock);
    println!(
        "FLOW_S_PROFILE source_bytes={source_bytes} timed_accepts={}",
        curve.len()
    );
    for depth in depths {
        let c = &curve[depth - 1];
        let (start, plan, commit) = flow_s_neighbourhood(&curve, depth);
        // Custody is start + commit. `plan` is the detached stage and is reported apart from it,
        // as 13.5 and L1 both require.
        println!(
            "FLOW_S_PROFILE depth={} start_ms={} plan_detached_ms={} commit_ms={} \
             custody_ms={} | neighbourhood(+-2 depths) start={} plan={} commit={}",
            c.depth,
            c.start_ms,
            c.plan_ms,
            c.commit_ms,
            c.start_ms + c.commit_ms,
            start,
            plan,
            commit,
        );
        let (basis, retry) = flow_s_repeatable(depth, 16, &SystemClock);
        println!("FLOW_S_PROFILE depth={depth} s1b_s3_basis={basis} s0_s1_accepted_retry={retry}");
    }
    let custody: u64 = curve.iter().map(|c| c.start_ms + c.commit_ms).sum();
    let detached: u64 = curve.iter().map(|c| c.plan_ms).sum();
    // The aggregate is the best-resolved figure in the run: 255 accepts summed, rather than one
    // tick-resolution sample. It is a total over a *growing* overlay, not 255x a fixed cost.
    println!(
        "FLOW_S_PROFILE cumulative_over_{}_accepts custody_ms={custody} detached_ms={detached}",
        curve.len()
    );
}
