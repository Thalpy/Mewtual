use super::*;
use crate::store::EpochStorageScanProgress;

fn scan(store: &mut ServerStore) -> (EpochStorageInventory, EpochStorageScanProgress) {
    let mut scan = store.scan_epoch_storage_with_studio().unwrap();
    let progress = loop {
        let p = scan.step().unwrap();
        if p.complete {
            break p;
        }
    };
    (scan.finish().unwrap(), progress)
}

#[test]
fn inventory_cache_reauthenticates_and_revalidates_gate_only_changes() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut f = Fixture::new();
    let mut budget = budget(&mut store, &f);
    let first = f.op(1);
    f.ingest(&mut store, &first, &mut budget).unwrap();
    let (cold, p) = scan(&mut store);
    assert_eq!(p.reused_records, 0);
    let (warm, p) = scan(&mut store);
    assert_eq!(p.reused_records, 1);
    assert_eq!(p.uncached_bytes, 0);
    assert_eq!(
        cold.records_for_server(SERVER, &f.group.group_id())
            .unwrap(),
        warm.records_for_server(SERVER, &f.group.group_id())
            .unwrap()
    );

    // Same signed history, different receipt/gate: the complete wrapper must miss the cache.
    store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            f.receipt(7),
            0,
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    let (_, p) = scan(&mut store);
    assert_eq!(p.reused_records, 0);
    assert!(p.uncached_bytes > 0);
    assert_eq!(scan(&mut store).1.reused_records, 1);

    let path = f.path(&store);
    // A separately valid owner receipt changes fixed-width hashes/signatures only. Equal
    // ciphertext length and identical operation history still cannot establish a cache hit.
    f.source.seal(f.receipt(8), &f.group, 0).unwrap();
    let mut e = Encoder::new();
    e.put_bytes(&scope_bytes(SERVER, &f.document).unwrap())
        .unwrap();
    e.put_u8(f.key.bucket());
    e.put_bytes(&f.source.snapshot().unwrap()).unwrap();
    let replacement = frame(&seal(&store.keys.db_key().unwrap(), &e.finish(), &mut rng()).unwrap());
    assert_eq!(replacement.len() as u64, fs::metadata(&path).unwrap().len());
    fs::write(&path, replacement).unwrap();
    assert_eq!(scan(&mut store).1.reused_records, 0);
    assert_eq!(scan(&mut store).1.reused_records, 1);

    // A cached final record never bypasses enumeration/accounting of a new staging copy.
    let staged = crate::store::staging_candidate(&path, 917);
    fs::write(&staged, [0; 123]).unwrap();
    let (with_orphan, p) = scan(&mut store);
    assert_eq!(p.reused_records, 1);
    assert_eq!(p.orphan_files, 1);
    assert_eq!(
        with_orphan
            .records_for_server(SERVER, &f.group.group_id())
            .unwrap()
            .len(),
        2
    );
    fs::remove_file(staged).unwrap();

    let original = fs::read(&path).unwrap();
    let mut corrupt = original.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    fs::write(&path, corrupt).unwrap();
    let mut job = store.scan_studio_receive_inventory().unwrap();
    assert!(
        job.step().is_err(),
        "same-size ciphertext damage cannot reuse validation"
    );
    assert!(job.finish().is_err());
    fs::write(&path, &original).unwrap();
    assert_eq!(scan(&mut store).1.reused_records, 1);
    drop(store);
    let mut store = open(root.path());
    assert_eq!(scan(&mut store).1.reused_records, 0, "remount starts cold");
    fs::remove_file(&path).unwrap();
    let (empty, p) = scan(&mut store);
    assert_eq!(p.reused_records, 0);
    assert!(empty
        .records_for_server(SERVER, &f.group.group_id())
        .unwrap()
        .is_empty());
}

#[test]
fn inventory_cache_large_source_needs_explicit_warmup_and_counts_actual_bytes() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let path = performance::save_inventory_fixture(&mut store);
    let original = fs::read(&path).unwrap();
    let mut cold = store.scan_studio_receive_inventory().unwrap();
    assert!(cold
        .step()
        .unwrap_err()
        .to_string()
        .contains("cold byte limit"));
    assert!(cold.finish().is_err());
    let (_, p) = scan(&mut store);
    assert_eq!(p.reused_records, 0);
    let mut warm = store.scan_studio_receive_inventory().unwrap();
    let p = loop {
        let p = warm.step().unwrap();
        if p.complete {
            break p;
        }
    };
    assert_eq!(p.reused_records, 1);
    assert_eq!(p.authenticated_bytes, original.len() as u64);
    assert_eq!(p.uncached_bytes, 0);
    warm.finish().unwrap();
    assert_eq!(fs::read(&path).unwrap(), original);

    // Authenticate a changed, equal-size wrapper whose inner history is deliberately damaged.
    // The second cold check must reject BEFORE attempting its expensive validator. Without
    // that check this fails with a history/parser error instead, or spends the cold CPU budget.
    let mut changed = store
        .read_epoch_registry_plain(&path)
        .unwrap()
        .unwrap()
        .plain;
    *changed.last_mut().unwrap() ^= 1;
    let replacement = frame(&seal(&store.keys.db_key().unwrap(), &changed, &mut rng()).unwrap());
    assert_eq!(replacement.len(), original.len());
    fs::write(&path, replacement).unwrap();
    let mut changed = store.scan_studio_receive_inventory().unwrap();
    assert!(changed
        .step()
        .unwrap_err()
        .to_string()
        .contains("cold byte limit"));
    assert!(changed.finish().is_err());
}

#[test]
fn inventory_cache_warm_records_cannot_exceed_aggregate_read_rail() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let paths: Vec<_> = (0..3)
        .map(|_| performance::save_inventory_fixture_ops(&mut store, 20))
        .collect();
    assert!(
        paths
            .iter()
            .map(|p| fs::metadata(p).unwrap().len())
            .sum::<u64>()
            > 8 * 1024 * 1024
    );
    scan(&mut store); // Every record is valid and warm, but their sum still exceeds the rail.
    let mut job = store.scan_studio_receive_inventory().unwrap();
    let error = loop {
        match job.step() {
            Err(e) => break e,
            Ok(p) => assert!(!p.complete),
        }
    };
    assert!(error.to_string().contains("inventory byte limit"));
    assert!(job.finish().is_err());
}
