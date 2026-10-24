//! Actual signed Studio history and durable warm ingest. Initial disk installation alone is
//! batched through the private test seam, as in the existing registry profiling harness.
use super::*;
use automerge::transaction::{CommitOptions, Transactable};
use automerge::{AutoCommit, Change, ROOT};
use catcoms_replication::epoch::MAX_EPOCH_BYTES;
use catcoms_rt::{Clock, ManualClock, SystemClock};

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
    let mut op = title_op(target, 9_000);
    op.nonce = [77; 16];
    match store
        .start_studio_closing_overlay(
            server,
            group,
            target,
            device,
            &close,
            Some(0),
            basis.fingerprint(),
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
                Some(0),
                basis.fingerprint(),
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

#[test]
fn studio_source_profile_smoke() {
    measure(33, &ManualClock::new(0));
}

#[test]
#[ignore = "opt-in release profiling of real dense Studio ingest; no machine-speed assertion"]
fn profile_studio_source_operations() {
    measure(20_000, &SystemClock);
}
