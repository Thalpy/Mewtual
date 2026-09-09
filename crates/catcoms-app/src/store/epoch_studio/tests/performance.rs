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
    let (unit, _, operations) = build(group, device, target, 3, 160_000);
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
            atomic_write,
            sync_studio,
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
            atomic_write,
            sync_studio,
        )
        .unwrap();
    store.retain_studio_source(group, device, state);
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
            atomic_write,
            sync_studio,
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

#[test]
fn studio_source_profile_smoke() {
    measure(33, &ManualClock::new(0));
}

#[test]
#[ignore = "opt-in release profiling of real dense Studio ingest; no machine-speed assertion"]
fn profile_studio_source_operations() {
    measure(20_000, &SystemClock);
}
