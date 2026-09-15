use super::*;

#[test]
fn registry_page_batch_is_atomic_duplicate_durable_and_restart_safe() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let page: Vec<_> = (1..=3).map(|n| f.op(n)).collect();
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &f);
    let empty = store
        .ingest_registry_page(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            &[],
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    assert!(empty.1.is_none());
    assert!(f.load(&store).is_none());
    let mut invalid = page.clone();
    invalid[1].blob.ciphertext[0] ^= 1;
    assert!(store
        .ingest_registry_page(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            &invalid,
            &mut rng(),
            &mut budget
        )
        .is_err());
    assert!(f.load(&store).is_none(), "a good prefix never escaped");
    assert!(store
        .ingest_registry_page(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            &[page[0].clone(), page[2].clone()],
            &mut rng(),
            &mut budget
        )
        .is_err());
    assert!(
        f.load(&store).is_none(),
        "missing middle dependency never commits its prefix"
    );
    let (counts, state) = store
        .ingest_registry_page(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            &page,
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    assert_eq!(counts.accepted, 3);
    assert_eq!(state.unwrap().op_count(), 3);
    let held = fs::read(f.path(&store)).unwrap();
    let (counts, _) = store
        .ingest_registry_page(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            &page,
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    assert_eq!(counts.duplicates, 3);
    assert_eq!(counts.accepted, 0);
    assert_eq!(fs::read(f.path(&store)).unwrap(), held);
    drop(store);
    let store = open(root.path());
    assert_eq!(
        f.load(&store).unwrap().projection().unwrap().pointers[&f.key],
        3
    );
}

#[test]
fn registry_page_batch_uncertain_rename_and_duplicate_flush_do_not_grant_success() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let page = [f.op(1), f.op(2)];
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &f);
    let result = store.ingest_registry_page_with_io(
        SERVER,
        &f.group,
        f.key.bucket(),
        f.source.doc_id(),
        &f.device,
        &page,
        &mut rng(),
        &mut budget,
        |path, bytes| {
            atomic_write(path, bytes)?;
            Err(invalid("lost acknowledgement"))
        },
        sync_registry,
    );
    assert!(result.is_err());
    assert_eq!(
        f.load(&store).unwrap().op_count(),
        2,
        "rename may have happened"
    );
    assert!(
        store
            .ingest_registry_page(
                SERVER,
                &f.group,
                f.key.bucket(),
                f.source.doc_id(),
                &f.device,
                &page,
                &mut rng(),
                &mut budget
            )
            .is_err(),
        "reconcile first"
    );
    budget = super::budget(&mut store, &f);
    assert!(store
        .ingest_registry_page_with_io(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            &page,
            &mut rng(),
            &mut budget,
            |_, _| panic!("duplicate must not replace"),
            |_, _| Err(invalid("flush failed"))
        )
        .is_err());
    budget = super::budget(&mut store, &f);
    let (counts, _) = store
        .ingest_registry_page(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            &page,
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    assert_eq!(counts.duplicates, 2);
}

#[test]
fn registry_page_batch_empty_completion_requires_open_scope_inventory_and_flush() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let first = f.op(1);
    let mut store = open(root.path());
    let mut budget = budget(&mut store, &f);
    f.ingest(&mut store, &first, &mut budget).unwrap();
    assert!(store
        .ingest_registry_page(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id() ^ 1,
            &f.device,
            &[],
            &mut rng(),
            &mut budget
        )
        .is_err());
    assert!(store
        .ingest_registry_page_with_io(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            &[],
            &mut rng(),
            &mut budget,
            |_, _| panic!("empty must not replace"),
            |_, _| Err(invalid("empty flush failed"))
        )
        .is_err());
    budget = super::budget(&mut store, &f);
    store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            f.receipt(4),
            0,
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    let held = fs::read(f.path(&store)).unwrap();
    assert!(store
        .ingest_registry_page(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            &[],
            &mut rng(),
            &mut budget
        )
        .is_err());
    assert!(store
        .ingest_registry_page(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            &[first],
            &mut rng(),
            &mut budget
        )
        .is_err());
    assert_eq!(fs::read(f.path(&store)).unwrap(), held);
    // Removing just this fixture's managed record simulates lost indexed storage; empty is not
    // an invitation to recreate epoch zero or call this prefix complete.
    fs::remove_file(f.path(&store)).unwrap();
    assert!(store
        .ingest_registry_page(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            &[],
            &mut rng(),
            &mut budget
        )
        .is_err());
}
