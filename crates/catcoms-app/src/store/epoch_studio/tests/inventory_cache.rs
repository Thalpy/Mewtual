use super::*;

#[test]
fn studio_inventory_cache_never_skips_reference_enumeration() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, f.insert());
    inventory(&mut store); // Warm the exact complete Studio wrapper.
    let mut warm = store.scan_epoch_storage_with_studio().unwrap();
    let p = loop {
        let p = warm.step().unwrap();
        if p.complete {
            break p;
        }
    };
    assert_eq!(p.reused_records, 1);
    warm.finish().unwrap();
    let mut references = store.scan_epoch_storage_with_studio().unwrap();
    references.collect_creative_references().unwrap();
    let p = loop {
        let p = references.step().unwrap();
        if p.complete {
            break p;
        }
    };
    assert_eq!(p.reused_records, 0);
    let refs = references.finish_creative_references().unwrap();
    assert!(refs
        .for_group(&f.group.group_id())
        .any(|cid| *cid == catcoms_storage::Cid::from_bytes([3; 32])));
}

#[test]
fn studio_inventory_cache_does_not_authorize_a_large_mutable_target() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let mut b = budget(&mut store, &f);
    let mut unit = StudioEpoch::new(&f.group, f.target, f.device.device_id()).unwrap();
    for n in 1..=2 {
        let mut domain = f.title();
        domain.nonce = [n; 16];
        let sealed = unit
            .edit_or_reseal(&f.device, &f.group, &mut rng(), &domain, 100)
            .unwrap();
        let signed = f.signed(&sealed);
        let mut expanded = automerge::Change::from_bytes(signed.delta)
            .unwrap()
            .decode();
        expanded.message = Some("x".repeat(160_000));
        let change = automerge::Change::from(expanded);
        let signed = SignedOp::sign_domain(
            &f.device,
            f.logical.doc_type,
            f.id,
            change.raw_bytes().to_vec(),
            &domain,
        )
        .unwrap();
        let sealed = SealedOp::seal(&signed, &f.group, &f.device, &mut rng()).unwrap();
        let (outcome, state) = store
            .ingest_studio_epoch(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &sealed,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        assert_eq!(outcome, Admission::Accepted);
        unit = state.unit;
    }
    assert!(fs::metadata(f.path(&store)).unwrap().len() > 256 * 1024);
    inventory(&mut store);
    let mut warm = store.scan_studio_receive_inventory().unwrap();
    let p = loop {
        let p = warm.step().unwrap();
        if p.complete {
            break p;
        }
    };
    assert_eq!(p.reused_records, 1);
    warm.finish().unwrap();
    assert!(store
        .check_studio_receive_source_bound(SERVER, &f.group.group_id(), f.target)
        .unwrap_err()
        .to_string()
        .contains("target source exceeds cold byte limit"));
    assert_eq!(
        f.load(&store).unwrap().op_count(),
        2,
        "explicit load remains available"
    );
}
