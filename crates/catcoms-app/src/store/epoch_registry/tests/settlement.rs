use super::*;
use automerge::transaction::{CommitOptions, Transactable};
use automerge::{AutoCommit, ROOT};
use catcoms_replication::{CloseRecord, SignedOp};

pub(super) struct TestSource {
    pub(super) f: Fixture,
    pub(super) store: ServerStore,
    pub(super) budget: EpochStorageBudget,
    pub(super) close: Vec<u8>,
    pub(super) receipt: Receipt,
}

/// Real 2-MiB closure plus an optional excluded edit, through checked ingest and vault writes.
pub(super) fn source_fixture(path: &Path, excluded: bool) -> TestSource {
    let f = Fixture::new();
    let mut store = open(path);
    let mut budget = budget(&mut store, &f);
    let mut writer = AutoCommit::new().with_actor(automerge::ActorId::from(
        f.device.device_id().as_bytes().to_vec(),
    ));
    let mut close = None;
    let mut receipt = None;
    for n in 0..if excluded { 11u8 } else { 10u8 } {
        let domain = RegistryOp::Put {
            key: f.key.clone(),
            epoch: u64::from(n),
        }
        .domain_op(&f.group.group_id(), [n; 16])
        .unwrap();
        writer
            .put(ROOT, "bucket", u64::from(f.key.bucket()))
            .unwrap();
        writer.put(ROOT, "epoch", 0u64).unwrap();
        writer
            .put(ROOT, "key", hex::encode(&f.document.logical_key))
            .unwrap();
        writer.put(ROOT, "kind", "registry").unwrap();
        writer.put(ROOT, "v", 1u64).unwrap();
        writer
            .put(
                ROOT,
                format!("p/0010/{}", hex::encode(f.key.logical_key())),
                u64::from(n),
            )
            .unwrap();
        writer
            .put(
                ROOT,
                format!("_p1/op/{}", hex::encode(domain.id(&f.device.device_id()))),
                1u64,
            )
            .unwrap();
        // Charge real signed bytes, including advisory metadata, to meet the production lower
        // bound in ten changes. Both ingest and vault save use the production admission path.
        writer.commit_with(CommitOptions::default().with_message("x".repeat(220_000)));
        let op = SignedOp::sign_domain(
            &f.device,
            DocType::DocRegistry,
            f.source.doc_id(),
            writer.get_last_local_change().unwrap().raw_bytes().to_vec(),
            &domain,
        )
        .unwrap();
        let op = SealedOp::seal(&op, &f.group, &f.device, &mut rng()).unwrap();
        let (_, state) = f.ingest(&mut store, &op, &mut budget).unwrap();
        if n == 9 {
            let selected = CloseRecord::sign(
                &f.document,
                f.source.doc_id(),
                0,
                writer.get_heads().into_iter().map(|head| head.0).collect(),
                &f.device,
            )
            .unwrap();
            let seed = state
                .projection()
                .unwrap()
                .checkpoint(selected.hash())
                .unwrap();
            receipt = Some(
                Receipt::sign(
                    f.document.clone(),
                    0,
                    selected.hash(),
                    seed.change_hash(),
                    0,
                    InheritedCheckpoint::EpochZero,
                    &f.device,
                )
                .unwrap(),
            );
            close = Some(selected.encode());
        }
    }
    TestSource {
        f,
        store,
        budget,
        close: close.unwrap(),
        receipt: receipt.unwrap(),
    }
}

#[test]
fn registry_store_settlement_plan_checks_exact_durable_source_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let TestSource {
        f,
        mut store,
        mut budget,
        close,
        receipt,
    } = source_fixture(root.path(), true);
    let plan = |store: &ServerStore| {
        store.plan_registry_settlement(SERVER, &f.group, f.key.bucket(), &f.device, &close, 0)
    };
    let before = fs::read(f.path(&store)).unwrap();
    assert!(plan(&store).is_err(), "Open cannot prepare settlement");
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
    store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            receipt,
            0,
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    let before = fs::read(f.path(&store)).unwrap();
    let prepared = plan(&store).unwrap();
    assert_eq!(prepared.included_operation_ids().len(), 10);
    assert_eq!(prepared.excluded_operations().len(), 1);
    assert_eq!(prepared.source_projection().pointers[&f.key], 10);
    assert!(!format!("{prepared:?}").contains("private-registry-cat"));
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Closing);
    assert_eq!(f.load(&store).unwrap().op_count(), 11);
    assert!(store
        .plan_registry_settlement(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            &vec![0; 4097],
            0
        )
        .is_err());
    assert!(
        store
            .plan_registry_settlement(
                SERVER,
                &f.group,
                f.key.bucket().wrapping_add(1),
                &f.device,
                &close,
                0
            )
            .is_err(),
        "missing is not an empty source"
    );
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
    drop(store);
    let store = open(root.path());
    let retry = plan(&store).unwrap();
    assert_eq!(prepared.source_version(), retry.source_version());
    assert_eq!(prepared.excluded_operations(), retry.excluded_operations());
    assert_eq!(prepared.checkpoint().bytes(), retry.checkpoint().bytes());
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
    // Authenticated local state is mandatory; no recovery inputs escape a damaged record.
    let mut damaged = before;
    let end = damaged.len() - 1;
    damaged[end] ^= 1;
    fs::write(f.path(&store), &damaged).unwrap();
    assert!(plan(&store).is_err());
    assert_eq!(fs::read(f.path(&store)).unwrap(), damaged);
}
