//! Actual original-source factory -> paid SDK -> actual ready transfer. No literal paid/current
//! flag replaces the Store, original canonical source, scope or real writer/save barriers.
use super::*;
const GRAPH: usize = 128 * 1024;
const WORK: usize = 512 * 1024 * 1024;
fn open_sdk(path: &std::path::Path, rng: &mut ChaCha20Rng) -> ServerStore {
    let store = ServerStore::open(path, SECRET, rng)
        .unwrap()
        .into_owner_evidence_store(SECRET, rng)
        .unwrap();
    store
        .configure_owner_evidence_budget(budget().with_closed_sdk_restore(GRAPH, WORK).unwrap())
        .unwrap();
    store
}
fn retained_source(store: &ServerStore, rng: &mut ChaCha20Rng, peer: u64) -> (Vec<u8>, [u8; 32]) {
    let mut initial = founder(store, peer);
    let owner = initial.retry(store, rng).unwrap();
    let group = owner.group();
    let signer = *owner.founder();
    drop(owner);
    drop(initial);
    (group, signer)
}
#[test]
fn actual_original_pool_fee_moves_through_private_restore_current_barrier_and_owned_server() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5d10);
    let store = open_sdk(dir.path(), &mut rng);
    let (group, signer) = retained_source(&store, &mut rng, 0x5d10);
    assert_eq!(store.owner_evidence.canonical.lock().unwrap().hot_bytes, 0);
    let OwnerRestoreClassification::Current(prepared) = store
        .prepare_owner_evidence_restore(&group, signer)
        .unwrap()
    else {
        panic!("real Current source")
    };
    let read_fee = store.owner_evidence.canonical.lock().unwrap().hot_bytes;
    let mut restored = decode(prepared, &store, 0x5d11);
    assert_eq!(
        store.owner_evidence.canonical.lock().unwrap().hot_bytes,
        read_fee + GRAPH
    );
    let owner = restored.retry(&store, &mut rng).unwrap();
    assert!(owner.is_live());
    assert_eq!(
        store.owner_evidence.canonical.lock().unwrap().hot_bytes,
        GRAPH,
        "source buffer refund cannot refund the retained SDK graph"
    );
    drop(restored);
    assert_eq!(
        store.owner_evidence.canonical.lock().unwrap().hot_bytes,
        GRAPH
    );
    drop(owner);
    assert_eq!(store.owner_evidence.canonical.lock().unwrap().hot_bytes, 0);
}
#[test]
fn actual_resource_exhaustion_refuses_private_decode_and_preserves_canonical_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5d12);
    let store = open_sdk(dir.path(), &mut rng);
    let (group, signer) = retained_source(&store, &mut rng, 0x5d12);
    let path = dir
        .path()
        .join("owner-state")
        .join(format!("{}.bin", hex::encode(key(&group, &signer))));
    let before = fs::read(&path).unwrap();
    let OwnerRestoreClassification::Current(prepared) = store
        .prepare_owner_evidence_restore(&group, signer)
        .unwrap()
    else {
        panic!("real Current source")
    };
    // Independently admitted resource users consume the SAME pool; no paid=true shortcut.
    let mut others = Vec::new();
    for _ in 0..64 {
        match store.reserve_owner_sdk_graphs(false) {
            Ok(fee) => others.push(fee),
            Err(_) => break,
        }
    }
    assert!(!others.is_empty());
    assert!(prepared
        .restore_private(
            &store,
            Hub::new().join(PeerId::from_u64(0x5d13)),
            ChaCha20Rng::seed_from_u64(0x5d13),
            Box::new(ManualClock::new(1000)),
            "capacity refusal"
        )
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        store.owner_evidence.canonical.lock().unwrap().hot_bytes,
        others.len() * GRAPH
    );
    drop(others);
    assert_eq!(store.owner_evidence.canonical.lock().unwrap().hot_bytes, 0);
}
#[test]
fn fixed_invoice_refusal_refunds_graph_and_keeps_original_source_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5d14);
    let store = ServerStore::open(dir.path(), SECRET, &mut rng)
        .unwrap()
        .into_owner_evidence_store(SECRET, &mut rng)
        .unwrap();
    // A real fee exists, but its negative cap is too small even for the fixed SDK invoice.
    store
        .configure_owner_evidence_budget(budget().with_closed_sdk_restore(1024, WORK).unwrap())
        .unwrap();
    let (group, signer) = retained_source(&store, &mut rng, 0x5d14);
    let path = dir
        .path()
        .join("owner-state")
        .join(format!("{}.bin", hex::encode(key(&group, &signer))));
    let before = fs::read(&path).unwrap();
    let OwnerRestoreClassification::Current(prepared) = store
        .prepare_owner_evidence_restore(&group, signer)
        .unwrap()
    else {
        panic!("real Current")
    };
    assert!(prepared
        .restore_private(
            &store,
            Hub::new().join(PeerId::from_u64(0x5d15)),
            ChaCha20Rng::seed_from_u64(0x5d15),
            Box::new(ManualClock::new(1000)),
            "closed shape cap"
        )
        .is_err());
    assert_eq!(store.owner_evidence.canonical.lock().unwrap().hot_bytes, 0);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[tokio::test]
async fn actual_first_add_stage_requires_two_independent_graphs_before_child_then_reopens_exactly()
{
    use std::time::Duration;
    let source_dir = tempfile::tempdir().unwrap();
    let member_dir = tempfile::tempdir().unwrap();
    let mut source_rng = ChaCha20Rng::seed_from_u64(0x5d20);
    let mut member_rng = ChaCha20Rng::seed_from_u64(0x5d21);
    let base = OwnerEvidenceBudget::checkpoint(
        1,
        128 * 1024 * 1024,
        128 * 1024 * 1024,
        24 * 1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap();
    // Source reading and actual first-Add use the same base+extra fee. Leave one byte LESS
    // than two additional graph caps; one-graph accounting would incorrectly let restore enter.
    let read_fee = base.buffer_fee().unwrap()
        + catcoms_sync::owner_evidence::OWNER_FIRST_ADD_EXTRA_PRIVATE_BYTES;
    let hot = read_fee + 2 * GRAPH - 1;
    let constrained = OwnerEvidenceBudget::checkpoint(
        1,
        128 * 1024 * 1024,
        hot,
        24 * 1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap()
    .with_closed_sdk_restore(GRAPH, WORK)
    .unwrap();
    let source = ServerStore::open(source_dir.path(), SECRET, &mut source_rng)
        .unwrap()
        .into_owner_operations_store(SECRET, &mut source_rng)
        .unwrap();
    source.configure_owner_evidence_budget(constrained).unwrap();
    let destination = ServerStore::open(member_dir.path(), SECRET, &mut member_rng)
        .unwrap()
        .into_owner_evidence_store(SECRET, &mut member_rng)
        .unwrap();
    destination.configure_owner_evidence_budget(base).unwrap();
    let hub = Hub::new();
    let source_peer = PeerId::from_u64(0x5d20);
    let mut initialization = source
        .prepare_owner_evidence_founder(
            hub.join(source_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(0x5d20),
            Box::new(ManualClock::new(1000)),
            "genuine parent",
            9,
        )
        .unwrap();
    let mut owner = initialization.retry(&source, &mut source_rng).unwrap();
    drop(initialization);
    let group = owner.group();
    let signer = *owner.founder();
    let invite = owner
        .mint_owner_checkpoint_invite(&source, [7; 16], 100_000, vec![])
        .unwrap();
    let attempt = destination
        .prepare_owner_evidence_checkpoint_join(
            &source,
            &owner,
            MlsDevice::generate().unwrap(),
            invite,
        )
        .unwrap();
    let mut direct = attempt.bind_direct_transport(hub.join(PeerId::from_u64(0x5d21)), source_peer);
    source
        .owner_evidence
        .canonical_fault
        .store(3, Ordering::SeqCst); // exact TempSynced gen2 stage
    {
        let mut join = Box::pin(direct.finish_direct(
            ChaCha20Rng::seed_from_u64(0x5d21),
            Box::new(ManualClock::new(1000)),
            "actual selected member",
        ));
        tokio::time::timeout(Duration::from_secs(20),async{
            tokio::select! {
                result=&mut join => panic!("reply cannot precede failed covering save: {}",result.is_ok()),
                ()=async {
                    for _ in 0..32 { if owner.run_owner_checkpoint_once(&source,&mut source_rng).await.is_err(){return;} }
                    panic!("genuine first-Add request did not reach retained stage");
                } => {},
            }
        }).await.expect("bounded actual network/save fixture");
    }
    drop(direct);
    drop(owner);
    let captured = source
        .prepare_owner_canonical_restore(
            source.capture_mounted_owner_profile(&group).unwrap(),
            signer,
        )
        .unwrap();
    assert!(
        captured.operation_parent_snapshot().is_some(),
        "real authenticated gen1 parent plus gen2 stage retained"
    );
    assert_eq!(
        source.owner_evidence.canonical.lock().unwrap().hot_bytes,
        read_fee
    );
    let CanonicalOwnerRestore::Current(prepared) =
        source.classify_owner_canonical_restore(captured).unwrap()
    else {
        panic!("original Current lineage")
    };
    assert!(prepared
        .restore_private(
            &source,
            Hub::new().join(PeerId::from_u64(0x5d22)),
            ChaCha20Rng::seed_from_u64(0x5d22),
            Box::new(ManualClock::new(1000)),
            "two graph refusal"
        )
        .is_err());
    assert_eq!(source.owner_evidence.canonical.lock().unwrap().hot_bytes, 0);
    drop(source);
    // New physical source capture, with enough independently configured local capacity. No old
    // task/source/lease is renewed; full stage+parent/source identity is revalidated after open.
    let reopened = ServerStore::open(source_dir.path(), SECRET, &mut source_rng).unwrap();
    reopened
        .configure_owner_evidence_budget(base.with_closed_sdk_restore(GRAPH, WORK).unwrap())
        .unwrap();
    let OwnerRestoreClassification::Current(prepared) = reopened
        .prepare_owner_evidence_restore(&group, signer)
        .unwrap()
    else {
        panic!("retained Current")
    };
    let buffers = reopened.owner_evidence.canonical.lock().unwrap().hot_bytes;
    let mut restored = decode(prepared, &reopened, 0x5d23);
    assert_eq!(
        reopened.owner_evidence.canonical.lock().unwrap().hot_bytes,
        buffers + GRAPH,
        "parent graph drained after actual child+parent validation"
    );
    let owner = restored.retry(&reopened, &mut source_rng).unwrap();
    assert_eq!(
        reopened.owner_evidence.canonical.lock().unwrap().hot_bytes,
        GRAPH
    );
    drop(restored);
    drop(owner);
    assert_eq!(
        reopened.owner_evidence.canonical.lock().unwrap().hot_bytes,
        0
    );
}

#[tokio::test]
async fn restored_closed_owner_cannot_erase_operation_fence_by_legacy_candidate_copy() {
    use std::time::Duration;
    let source_dir = tempfile::tempdir().unwrap();
    let member_dir = tempfile::tempdir().unwrap();
    let hub = Hub::new();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5d30);
    let mut member_rng = ChaCha20Rng::seed_from_u64(0x5d31);
    // This is the real first-Add capacity profile, so rejection cannot be explained by a tiny
    // snapshot/operation allowance before the actual Sync candidate-copy fence.
    let base = OwnerEvidenceBudget::checkpoint(
        1,
        128 * 1024 * 1024,
        128 * 1024 * 1024,
        24 * 1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap();
    let source = ServerStore::open(source_dir.path(), SECRET, &mut rng)
        .unwrap()
        .into_owner_operations_store(SECRET, &mut rng)
        .unwrap();
    source
        .configure_owner_evidence_budget(base.with_closed_sdk_restore(GRAPH, WORK).unwrap())
        .unwrap();
    let destination = ServerStore::open(member_dir.path(), SECRET, &mut member_rng)
        .unwrap()
        .into_owner_evidence_store(SECRET, &mut member_rng)
        .unwrap();
    destination.configure_owner_evidence_budget(base).unwrap();
    let mut initial = source
        .prepare_owner_evidence_founder(
            hub.join(PeerId::from_u64(0x5d30)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(0x5d30),
            Box::new(ManualClock::new(1000)),
            "original founder",
            9,
        )
        .unwrap();
    let original = initial.retry(&source, &mut rng).unwrap();
    let group = original.group();
    let signer = *original.founder();
    drop(initial);
    drop(original);
    let OwnerRestoreClassification::Current(prepared) = source
        .prepare_owner_evidence_restore(&group, signer)
        .unwrap()
    else {
        panic!("real Current")
    };
    let mut decoded = prepared
        .restore_private(
            &source,
            hub.join(PeerId::from_u64(0x5d32)),
            ChaCha20Rng::seed_from_u64(0x5d32),
            Box::new(ManualClock::new(1000)),
            "closed restored owner",
        )
        .unwrap();
    let mut owner = decoded.retry(&source, &mut rng).unwrap();
    drop(decoded);
    assert_eq!(
        source.owner_evidence.canonical.lock().unwrap().hot_bytes,
        GRAPH
    );
    let path = source_dir
        .path()
        .join("owner-state")
        .join(format!("{}.bin", hex::encode(key(&group, &signer))));
    let before = fs::read(&path).unwrap();
    let invite = owner
        .mint_owner_checkpoint_invite(&source, [8; 16], 100_000, vec![])
        .unwrap();
    let attempt = destination
        .prepare_owner_evidence_checkpoint_join(
            &source,
            &owner,
            MlsDevice::generate().unwrap(),
            invite,
        )
        .unwrap();
    let mut direct =
        attempt.bind_direct_transport(hub.join(PeerId::from_u64(0x5d31)), PeerId::from_u64(0x5d32));
    {
        let join = direct.finish_direct(
            ChaCha20Rng::seed_from_u64(0x5d31),
            Box::new(ManualClock::new(1000)),
            "candidate must refuse",
        );
        tokio::pin!(join);
        tokio::time::timeout(Duration::from_secs(20),async{
            tokio::select! {
                result=&mut join=>assert!(result.is_err(),"completed load cannot silently mint an uncharged Add candidate"),
                ()=async{for _ in 0..32{owner.run_owner_checkpoint_once(&source,&mut rng).await.unwrap();}panic!("expected original request refusal")}=>{},
            }
        }).await.expect("bounded actual candidate-copy refusal");
    }
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        source.owner_evidence.canonical.lock().unwrap().hot_bytes,
        GRAPH,
        "no candidate/response fee or changed graph retained"
    );
    assert!(
        owner.is_live(),
        "resource refusal must not retire original Current"
    );
    drop(direct);
    drop(owner);
    assert_eq!(source.owner_evidence.canonical.lock().unwrap().hot_bytes, 0);
}
