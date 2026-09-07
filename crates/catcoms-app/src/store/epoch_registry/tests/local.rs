//! The local publication boundary is two durable records, not just a marker in memory.
use super::*;
use crate::store::epoch_intents::sync_intent;
use catcoms_replication::SignedOp;

fn domain(f: &Fixture, n: u8) -> DomainOp {
    RegistryOp::Put {
        key: f.key.clone(),
        epoch: u64::from(n),
    }
    .domain_op(&f.group.group_id(), [n; 16])
    .unwrap()
}
fn budgets(store: &mut ServerStore, f: &Fixture) -> (EpochStorageBudget, EpochIntentBudget) {
    let inv = inventory(store);
    (
        EpochStorageBudget::from_inventory(
            StorageScope::new(SERVER, &f.group.group_id()).unwrap(),
            inv.records_for_server(SERVER, &f.group.group_id()).unwrap(),
        )
        .unwrap(),
        EpochIntentBudget::from_inventory(&inv).unwrap(),
    )
}
fn edit(
    store: &mut ServerStore,
    f: &Fixture,
    n: u8,
    budget: &mut EpochStorageBudget,
    intents: &mut EpochIntentBudget,
) -> (SealedOp, EpochRegistryState) {
    store
        .edit_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            domain(f, n),
            &mut rng(),
            budget,
            intents,
        )
        .unwrap()
}
fn signed(f: &Fixture, op: &SealedOp) -> SignedOp {
    op.open(
        &f.group
            .channel_secret(&f.device, op.doc_type, op.doc_id)
            .unwrap(),
    )
    .unwrap()
}
fn intent_count(store: &ServerStore, f: &Fixture) -> usize {
    store
        .load_epoch_intents(SERVER, &f.document)
        .unwrap()
        .pending()
        .len()
}
fn intent_path(store: &ServerStore, f: &Fixture) -> PathBuf {
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.document).unwrap();
    store
        .dir
        .join("servers")
        .join(format!("{}.intents", blake3::hash(&scope).to_hex()))
}

#[test]
fn registry_local_publication_reopens_and_retries_exact_change_after_newer_heads() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let (first, state) = edit(&mut store, &f, 1, &mut budget, &mut intents);
    assert_eq!(state.op_count(), 1);
    assert_eq!(intent_count(&store, &f), 1);
    let original = signed(&f, &first);
    assert_eq!(original.parsed_domain_op().unwrap(), Some(domain(&f, 1)));
    let (second, _) = edit(&mut store, &f, 2, &mut budget, &mut intents);
    let saved = fs::read(f.path(&store)).unwrap();
    let ledger = fs::read(intent_path(&store, &f)).unwrap();
    drop(store);
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let (retry, state) = edit(&mut store, &f, 1, &mut budget, &mut intents);
    assert_eq!(
        signed(&f, &retry),
        original,
        "not a new change over the newer heads"
    );
    assert_eq!(state.op_count(), 2);
    assert_eq!(state.projection().unwrap().pointers[&f.key], 2);
    assert_eq!(
        intent_count(&store, &f),
        2,
        "a live marker never retires an intent"
    );
    assert_eq!(fs::read(f.path(&store)).unwrap(), saved);
    assert_eq!(fs::read(intent_path(&store, &f)).unwrap(), ledger);
    let mut peer = RegistryEpoch::new(&f.group, f.key.bucket(), f.device.device_id()).unwrap();
    assert_eq!(
        peer.ingest(&first, &f.group, &f.device).unwrap(),
        Admission::Accepted
    );
    assert_eq!(
        peer.ingest(&second, &f.group, &f.device).unwrap(),
        Admission::Accepted
    );
    assert_eq!(
        peer.ingest(&retry, &f.group, &f.device).unwrap(),
        Admission::Duplicate
    );
    assert_eq!(peer.projection().unwrap(), state.projection().unwrap());
}

#[test]
fn registry_local_each_write_failure_retains_only_safe_state_and_retry_recovers() {
    // Every failure boundary returns no ciphertext. After-rename visibility is NOT durability.
    for stage in 0..2 {
        for mode in 0..3 {
            // before write, after rename, writer unwind
            let root = tempfile::tempdir().unwrap();
            let f = Fixture::new();
            let mut store = open(root.path());
            let (mut budget, mut intents) = budgets(&mut store, &f);
            let required_intent = intent_path(&store, &f);
            let fail = |path: &Path, bytes: &[u8]| {
                if mode == 1 {
                    atomic_write(path, bytes)?;
                }
                if mode == 2 {
                    panic!("injected writer panic");
                }
                Err(AppError::Io("injected failed persistence".into()))
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                store.edit_registry_epoch_with_io(
                    SERVER,
                    &f.group,
                    f.key.bucket(),
                    &f.device,
                    domain(&f, 1),
                    &mut rng(),
                    &mut budget,
                    &mut intents,
                    |path, bytes| {
                        if stage == 0 {
                            fail(path, bytes)
                        } else {
                            atomic_write(path, bytes)
                        }
                    },
                    sync_intent,
                    |path, bytes| {
                        assert!(required_intent.exists(), "intent first");
                        if stage == 1 {
                            fail(path, bytes)
                        } else {
                            atomic_write(path, bytes)
                        }
                    },
                    sync_registry,
                )
            }));
            assert!(result.is_err() || result.unwrap().is_err());
            assert!(budget.requires_reconciliation());
            assert_eq!(
                intent_count(&store, &f),
                usize::from(stage == 1 || mode == 1)
            );
            assert_eq!(
                f.load(&store).map(|s| s.op_count()).unwrap_or(0),
                usize::from(stage == 1 && mode == 1)
            );
            drop(store);
            let mut store = open(root.path());
            let (mut budget, mut intents) = budgets(&mut store, &f);
            let (_, state) = edit(&mut store, &f, 1, &mut budget, &mut intents);
            assert_eq!(state.op_count(), 1);
            assert_eq!(intent_count(&store, &f), 1);
        }
    }
}

#[test]
fn registry_local_duplicate_flush_failure_cannot_release_ciphertext() {
    for stage in 0..2 {
        for panic in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let f = Fixture::new();
            let mut store = open(root.path());
            let (mut budget, mut intents) = budgets(&mut store, &f);
            let (first, _) = edit(&mut store, &f, 1, &mut budget, &mut intents);
            let saved = fs::read(f.path(&store)).unwrap();
            let ledger = fs::read(intent_path(&store, &f)).unwrap();
            let fail = || {
                if panic {
                    panic!("injected sync panic");
                }
                Err(AppError::Io("injected failed flush".into()))
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                store.edit_registry_epoch_with_io(
                    SERVER,
                    &f.group,
                    f.key.bucket(),
                    &f.device,
                    domain(&f, 1),
                    &mut rng(),
                    &mut budget,
                    &mut intents,
                    |_, _| panic!("duplicate intent must not rewrite"),
                    |path, n| {
                        if stage == 0 {
                            fail()
                        } else {
                            sync_intent(path, n)
                        }
                    },
                    |_, _| panic!("duplicate registry must not rewrite"),
                    |path, n| {
                        if stage == 1 {
                            fail()
                        } else {
                            sync_registry(path, n)
                        }
                    },
                )
            }));
            assert!(result.is_err() || result.unwrap().is_err());
            assert!(budget.requires_reconciliation());
            assert_eq!(fs::read(f.path(&store)).unwrap(), saved);
            assert_eq!(fs::read(intent_path(&store, &f)).unwrap(), ledger);
            let (mut budget, mut intents) = budgets(&mut store, &f);
            let (retry, state) = edit(&mut store, &f, 1, &mut budget, &mut intents);
            assert_eq!(signed(&f, &first), signed(&f, &retry));
            assert_eq!(state.op_count(), 1);
        }
    }
}

#[test]
fn registry_local_invalid_or_conflicting_input_cannot_strand_an_intent() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let inbound = f.op(1);
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    f.ingest(&mut store, &inbound, &mut budget).unwrap();
    assert_eq!(intent_count(&store, &f), 0);
    let original = fs::read(f.path(&store)).unwrap();
    let mut conflict = domain(&f, 2);
    conflict.nonce = domain(&f, 1).nonce; // same id, different body, NO existing local ledger
    let mut oversized = domain(&f, 1);
    oversized.body = vec![b' '; 1025];
    let mut wrong_scope = domain(&f, 1);
    wrong_scope.logical_key[0] ^= 1;
    let mut wrong_type = domain(&f, 1);
    wrong_type.doc_type = DocType::StudioObject;
    let mut noncanonical = domain(&f, 1);
    noncanonical.body.push(b' ');
    for op in [conflict, oversized, wrong_scope, wrong_type, noncanonical] {
        assert!(store
            .edit_registry_epoch(
                SERVER,
                &f.group,
                f.key.bucket(),
                &f.device,
                op,
                &mut rng(),
                &mut budget,
                &mut intents
            )
            .is_err());
        assert_eq!(intent_count(&store, &f), 0);
        assert!(!budget.requires_reconciliation());
        assert_eq!(fs::read(f.path(&store)).unwrap(), original);
    }
    let outsider = MlsDevice::generate().unwrap();
    assert!(store
        .edit_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &outsider,
            domain(&f, 1),
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .is_err());
    assert_eq!(intent_count(&store, &f), 0);
    let (retry, state) = edit(&mut store, &f, 1, &mut budget, &mut intents);
    assert_eq!(signed(&f, &inbound), signed(&f, &retry));
    assert_eq!(state.op_count(), 1);
    assert_eq!(intent_count(&store, &f), 1);
}

#[test]
fn registry_local_closed_or_faulted_epochs_retain_intents_but_do_not_publish() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    edit(&mut store, &f, 1, &mut budget, &mut intents);
    for close in [7, 8] {
        store
            .seal_registry_epoch(
                SERVER,
                &f.group,
                f.key.bucket(),
                &f.device,
                f.receipt(close),
                0,
                &mut rng(),
                &mut budget,
            )
            .unwrap();
        let saved = fs::read(f.path(&store)).unwrap();
        let ledger = fs::read(intent_path(&store, &f)).unwrap();
        for n in [1, 2] {
            assert!(store
                .edit_registry_epoch(
                    SERVER,
                    &f.group,
                    f.key.bucket(),
                    &f.device,
                    domain(&f, n),
                    &mut rng(),
                    &mut budget,
                    &mut intents
                )
                .is_err());
        }
        assert_eq!(fs::read(f.path(&store)).unwrap(), saved);
        assert_eq!(fs::read(intent_path(&store, &f)).unwrap(), ledger);
        assert_eq!(intent_count(&store, &f), 1);
        assert!(!budget.requires_reconciliation());
    }
}

#[test]
fn registry_local_retry_after_owner_succession_fits_at_content_cap() {
    use crate::store::epoch_budget::CONTENT_ALLOWANCE_BYTES;
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let original_owner = f.device.device_id();
    let author = MlsDevice::generate().unwrap();
    let welcome = f
        .group
        .add_member(&f.device, author.key_package().unwrap())
        .unwrap()
        .welcome;
    f.group = ServerGroup::join(&author, &welcome).unwrap();
    f.device = author;
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let (first, _) = edit(&mut store, &f, 1, &mut budget, &mut intents);
    let original = signed(&f, &first);
    let saved = fs::read(f.path(&store)).unwrap();
    let ledger = fs::read(intent_path(&store, &f)).unwrap();
    f.group.remove_member(&f.device, &original_owner).unwrap();
    assert_eq!(f.group.designated_committer(), Some(f.device.device_id()));
    drop(store);
    let mut store = open(root.path());
    let inv = inventory(&mut store);
    let records = inv.records_for_server(SERVER, &f.group.group_id()).unwrap();
    let used: u64 = records.iter().map(|r| r.footprint.content).sum();
    // Synthetic unrelated content fills the budget without writing 2 GiB of test garbage.
    let filler = StorageRecord {
        id: [0x93; 32],
        document: [0x94; 32],
        footprint: Footprint {
            content: CONTENT_ALLOWANCE_BYTES - used,
            ..Footprint::default()
        },
    };
    let mut budget = EpochStorageBudget::from_inventory(
        StorageScope::new(SERVER, &f.group.group_id()).unwrap(),
        records.into_iter().chain([filler]),
    )
    .unwrap();
    let mut intents = EpochIntentBudget::from_inventory(&inv).unwrap();
    let (retry, state) = edit(&mut store, &f, 1, &mut budget, &mut intents);
    assert_eq!(retry.epoch, f.group.epoch());
    assert_ne!(retry.epoch, first.epoch);
    assert_eq!(signed(&f, &retry), original);
    assert_eq!(state.op_count(), 1);
    assert_eq!(
        fs::read(f.path(&store)).unwrap(),
        saved,
        "derived owner refresh needs no copy"
    );
    assert_eq!(fs::read(intent_path(&store, &f)).unwrap(), ledger);
    assert!(store
        .edit_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            domain(&f, 2),
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .is_err());
    assert_eq!(intent_count(&store, &f), 1);
    assert!(!budget.requires_reconciliation());
}

#[test]
fn registry_local_stale_intent_budget_and_corrupt_source_cannot_publish() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    edit(&mut store, &f, 1, &mut budget, &mut intents);
    let mut cleanup = store.cleanup_epoch_storage_staging_with_registry().unwrap();
    while !cleanup.step().unwrap().complete {}
    drop(cleanup);
    assert!(store
        .edit_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            domain(&f, 1),
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .is_err());
    assert_eq!(f.load(&store).unwrap().op_count(), 1);
    let (mut budget, mut intents) = budgets(&mut store, &f);
    fs::write(f.path(&store), b"corrupt").unwrap();
    assert!(store
        .edit_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            domain(&f, 2),
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .is_err());
    assert!(budget.requires_reconciliation());
    assert_eq!(intent_count(&store, &f), 1);
    assert_eq!(fs::read(f.path(&store)).unwrap(), b"corrupt");
}
