use super::settlement::source_fixture;
use super::*;
use catcoms_rt::ManualClock;

#[test]
fn registry_seed_store_reads_installed_bytes_only_without_mutation_and_detects_lost_files() {
    let root = tempfile::tempdir().unwrap();
    let mut s = source_fixture(root.path(), false);
    let seed =
        s.f.load(&s.store)
            .unwrap()
            .projection()
            .unwrap()
            .checkpoint(s.receipt.close_record_hash)
            .unwrap();
    let id = seed.origin().doc_id();
    let hash = seed.change_hash();
    let read = |s: &mut super::settlement::TestSource, id, hash| {
        s.store.read_registry_seed(
            SERVER,
            &s.f.group,
            s.f.key.bucket(),
            &s.f.device,
            id,
            hash,
            &mut s.budget,
        )
    };
    assert!(read(&mut s, id, hash).unwrap().is_none());
    s.store
        .seal_registry_epoch(
            SERVER,
            &s.f.group,
            s.f.key.bucket(),
            &s.f.device,
            s.receipt.clone(),
            0,
            &mut rng(),
            &mut s.budget,
        )
        .unwrap();
    let closing = fs::read(s.f.path(&s.store)).unwrap();
    assert!(
        read(&mut s, id, hash).unwrap().is_none(),
        "head is not an installed seed"
    );
    assert_eq!(fs::read(s.f.path(&s.store)).unwrap(), closing);
    let inv = inventory(&mut s.store);
    let mut intents = EpochIntentBudget::from_inventory(&inv).unwrap();
    s.store
        .install_registry_checkpoint(
            SERVER,
            &s.f.group,
            s.f.key.bucket(),
            &s.f.device,
            &s.receipt.encode(),
            &s.close,
            0,
            &ManualClock::new(1),
            &mut rng(),
            &mut s.budget,
            &mut intents,
        )
        .unwrap();
    let before = fs::read(s.f.path(&s.store)).unwrap();
    assert_eq!(read(&mut s, id, hash).unwrap().unwrap(), seed.bytes());
    assert!(read(&mut s, id ^ 1, hash).unwrap().is_none());
    assert!(read(&mut s, id, [0; 32]).unwrap().is_none());
    assert_eq!(
        fs::read(s.f.path(&s.store)).unwrap(),
        before,
        "serving never reseeds or writes"
    );
    drop(s.store);
    s.store = open(root.path());
    s.budget = budget(&mut s.store, &s.f);
    assert_eq!(read(&mut s, id, hash).unwrap().unwrap(), seed.bytes());
    // Test-only deletion of one exact temp-vault file, not a product cleanup operation.
    fs::remove_file(s.f.path(&s.store)).unwrap();
    assert!(
        read(&mut s, id, hash).is_err(),
        "lost indexed state cannot masquerade as absence"
    );
}
