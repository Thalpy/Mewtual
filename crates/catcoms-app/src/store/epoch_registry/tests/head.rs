use super::*;

fn select(
    store: &mut ServerStore,
    f: &Fixture,
    budget: &mut EpochStorageBudget,
    tenure: Option<u64>,
) -> Result<catcoms_sync::receipt_head::ReceiptHeadSelection, AppError> {
    store.prepare_registry_head(
        SERVER,
        &f.group,
        f.key.bucket(),
        &f.device,
        tenure,
        &mut rng(),
        budget,
    )
}
fn prepare(
    store: &mut ServerStore,
    f: &Fixture,
    receipt: Receipt,
    budget: &mut EpochStorageBudget,
) {
    store
        .prepare_epoch_owner_receipt(SERVER, receipt, &f.group, 0, &mut rng(), budget)
        .unwrap();
}
fn seal(store: &mut ServerStore, f: &Fixture, receipt: Receipt, budget: &mut EpochStorageBudget) {
    store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            receipt,
            0,
            &mut rng(),
            budget,
        )
        .unwrap();
}
#[test]
fn registry_head_requires_matching_decision_source_and_keeps_publication_pending() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut f = Fixture::new();
    let mut budget = budget(&mut store, &f);
    assert!(select(&mut store, &f, &mut budget, Some(0))
        .unwrap()
        .receipt
        .is_none());
    let op = f.op(1);
    f.ingest(&mut store, &op, &mut budget).unwrap();
    let receipt = f.receipt(7);
    prepare(&mut store, &f, receipt.clone(), &mut budget);
    let hint = select(&mut store, &f, &mut budget, Some(0)).unwrap();
    assert_eq!(hint.receipt.as_ref(), Some(&receipt));
    assert!(!hint.prove);
    seal(&mut store, &f, receipt.clone(), &mut budget);
    assert!(!select(&mut store, &f, &mut budget, None).unwrap().prove);
    assert!(!select(&mut store, &f, &mut budget, Some(1)).unwrap().prove);
    assert!(select(&mut store, &f, &mut budget, Some(0)).unwrap().prove);
    let journal = store
        .load_epoch_owner_receipts(SERVER, &f.document)
        .unwrap();
    assert_eq!(journal.pending(), Some(&receipt));
    assert!(journal.published().is_none());
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Closing);
    drop(store);
    let mut store = open(root.path());
    let mut budget = super::budget(&mut store, &f);
    assert!(select(&mut store, &f, &mut budget, Some(0)).unwrap().prove);
    store
        .mark_epoch_owner_receipt_published(
            SERVER,
            &f.document,
            receipt.hash(),
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    let next = Receipt::sign(
        f.document.clone(),
        1,
        [11; 32],
        [12; 32],
        0,
        InheritedCheckpoint::EpochZero,
        &f.device,
    )
    .unwrap();
    prepare(&mut store, &f, next.clone(), &mut budget);
    let hint = select(&mut store, &f, &mut budget, Some(0)).unwrap();
    assert_eq!(hint.receipt, Some(next));
    assert!(!hint.prove, "never attest the older published fallback");
}
#[test]
fn registry_head_disagreement_fault_and_deleted_indexed_source_cannot_become_fresh_proof() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut f = Fixture::new();
    let mut budget = budget(&mut store, &f);
    let op = f.op(1);
    f.ingest(&mut store, &op, &mut budget).unwrap();
    let chosen = f.receipt(7);
    let alternative = f.receipt(8);
    prepare(&mut store, &f, chosen.clone(), &mut budget);
    seal(&mut store, &f, alternative, &mut budget);
    assert!(!select(&mut store, &f, &mut budget, Some(0)).unwrap().prove);
    seal(&mut store, &f, chosen, &mut budget);
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
    assert!(select(&mut store, &f, &mut budget, Some(0)).is_err());
    // Test-only deletion of this exact temp fixture record simulates storage loss after inventory.
    fs::remove_file(f.path(&store)).unwrap();
    assert!(select(&mut store, &f, &mut budget, Some(0)).is_err());
    assert!(select(&mut store, &f, &mut budget, None).is_err());
}
#[test]
fn registry_head_uncertain_source_flush_returns_no_proof_and_blocks_budget_until_rescan() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut f = Fixture::new();
    let mut budget = budget(&mut store, &f);
    let op = f.op(1);
    f.ingest(&mut store, &op, &mut budget).unwrap();
    let receipt = f.receipt(7);
    prepare(&mut store, &f, receipt.clone(), &mut budget);
    seal(&mut store, &f, receipt, &mut budget);
    assert!(store
        .prepare_registry_head_with_sync(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            Some(0),
            &mut rng(),
            &mut budget,
            |_, _| Err(AppError::CommittedButNotDurable("injected sync".into()))
        )
        .is_err());
    assert!(select(&mut store, &f, &mut budget, Some(0)).is_err());
    let mut budget = super::budget(&mut store, &f);
    assert!(select(&mut store, &f, &mut budget, Some(0)).unwrap().prove);
}
