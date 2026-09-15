//! Saved-intent authority, retry identity and conservative recovery screening share real vault IO.
use super::super::replay::ReplaySync;
use super::*;
use catcoms_replication::SignedOp;
use catcoms_rt::ManualClock;

#[test]
fn registry_replay_rotation_uses_saved_author_intents_and_checks_staged_then_retained_tombstones() {
    use super::settlement::{source_fixture, TestSource};
    use catcoms_replication::{CloseRecord, RecoveryTransition};
    let root = tempfile::tempdir().unwrap();
    let TestSource {
        f,
        mut store,
        mut budget,
        close,
        receipt,
    } = source_fixture(root.path(), true);
    let (_, mut intents) = budgets(&mut store, &f);
    let excluded = journal(&mut store, &f, put(&f, 10, 10), &mut budget, &mut intents);
    let unaccepted = journal(&mut store, &f, put(&f, 12, 12), &mut budget, &mut intents);
    // An independently retained version includes a deletion excluded by the selected receipt.
    // The real typed planner constructs this evidence; no malformed payload is used for the hold.
    let mut fork = f.load(&store).unwrap().unit;
    fork.edit(&f.device, &f.group, &mut rng(), &tombstone(&f, 13))
        .unwrap();
    fork.seal(receipt.clone(), &f.group, 0).unwrap();
    let deleted = fork
        .prepare_settlement(&CloseRecord::decode(&close).unwrap(), &f.group, 0)
        .unwrap()
        .recovery_snapshot()
        .unwrap()
        .unwrap();
    store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            receipt.clone(),
            0,
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    let (_, successor) = store
        .install_registry_checkpoint(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            &receipt.encode(),
            &close,
            0,
            &ManualClock::new(0),
            &mut rng(),
            &mut budget,
            &mut intents,
        )
        .unwrap();
    assert!(store
        .replay_registry_intent(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            excluded,
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .is_err());
    let first = signed(
        &f,
        replay(&mut store, &f, excluded, &mut budget, &mut intents).unwrap(),
    );
    assert_eq!(first.doc_id, successor.doc_id());
    assert_eq!(
        f.load(&store).unwrap().projection().unwrap().pointers[&f.key],
        10
    );
    let state = store.load_epoch_recovery(SERVER, &f.document).unwrap();
    let mut second = state.retained().next().unwrap().clone();
    // Distinct typed historical receipt identity, without tombstones. Both retained slots are
    // clean; only the third, staged snapshot carries the deletion that must hold new authoring.
    second.projection[1 + 4 + f.group.group_id().len() + 1 + 4] ^= 1;
    for snapshot in [second, deleted] {
        store
            .update_epoch_recovery_accounted(
                SERVER,
                &f.document,
                EpochRecoveryAction::Stage(snapshot),
                &ManualClock::new(1),
                &mut rng(),
                &mut budget,
            )
            .unwrap();
    }
    let state = store.load_epoch_recovery(SERVER, &f.document).unwrap();
    let RecoveryTransition::EvictionPending {
        oldest_snapshot,
        staged_snapshot,
        ..
    } = state.eviction_pending().unwrap().unwrap()
    else {
        panic!("expected staged deletion")
    };
    for retained in [false, true] {
        if retained {
            store
                .update_epoch_recovery_accounted(
                    SERVER,
                    &f.document,
                    EpochRecoveryAction::Acknowledge {
                        oldest_snapshot,
                        staged_snapshot,
                    },
                    &ManualClock::new(2),
                    &mut rng(),
                    &mut budget,
                )
                .unwrap();
        }
        let before = fs::read(f.path(&store)).unwrap();
        let ledger = intent_bytes(&store, &f);
        assert!(matches!(
            replay(&mut store, &f, unaccepted, &mut budget, &mut intents)
                .unwrap()
                .0,
            RegistryReplayOutcome::Held(RegistryReplayHold::DeletedPointer)
        ));
        assert_eq!(
            signed(
                &f,
                replay(&mut store, &f, excluded, &mut budget, &mut intents).unwrap()
            ),
            first
        );
        assert_eq!(fs::read(f.path(&store)).unwrap(), before);
        assert_eq!(intent_bytes(&store, &f), ledger);
    }
    // Even the exact-log exception must check every typed slot rather than skip corrupt evidence.
    let mut opaque = store
        .load_epoch_recovery(SERVER, &f.document)
        .unwrap()
        .retained()
        .next()
        .unwrap()
        .clone();
    opaque.projection = b"opaque legacy record".to_vec();
    store
        .update_epoch_recovery_accounted(
            SERVER,
            &f.document,
            EpochRecoveryAction::Stage(opaque),
            &ManualClock::new(3),
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    let before = fs::read(f.path(&store)).unwrap();
    assert!(replay(&mut store, &f, excluded, &mut budget, &mut intents).is_err());
    assert!(budget.requires_reconciliation());
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
}

#[test]
fn registry_replay_checks_source_intent_recovery_inventory_and_budget_freshness() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let zero = f.op(0);
    f.ingest(&mut store, &zero, &mut budget).unwrap();
    let (_, mut stale_intents) = budgets(&mut store, &f);
    let id = journal(&mut store, &f, put(&f, 1, 1), &mut budget, &mut intents);
    assert!(replay(&mut store, &f, id, &mut budget, &mut stale_intents).is_err());
    // Every record family is checked; omitting either existing epoch or intent cannot fund replay.
    let inv = inventory(&mut store);
    let all = inv.records_for_server(SERVER, &f.group.group_id()).unwrap();
    let before = fs::read(f.path(&store)).unwrap();
    let ledger = intent_bytes(&store, &f);
    for omitted in 0..all.len() {
        let records = all
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != omitted)
            .map(|(_, r)| *r)
            .collect::<Vec<_>>();
        let mut incomplete = EpochStorageBudget::from_inventory(
            StorageScope::new(SERVER, &f.group.group_id()).unwrap(),
            records,
        )
        .unwrap();
        assert!(replay(&mut store, &f, id, &mut incomplete, &mut intents).is_err());
        intents = EpochIntentBudget::from_inventory(&inv).unwrap();
    }
    // A valid recovery record appears after the scan: its omitted footprint must also refuse.
    // A generic malformed slot is independently checked in the rotation test. Here use an
    // authentic empty record so the failure is stale accounting rather than typed decoding.
    store
        .update_epoch_recovery(
            SERVER,
            &f.document,
            EpochRecoveryAction::AdvanceTime,
            &ManualClock::new(0),
            &mut rng(),
        )
        .unwrap();
    assert!(replay(&mut store, &f, id, &mut budget, &mut intents).is_err());
    assert!(budget.requires_reconciliation());
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
    assert_eq!(intent_bytes(&store, &f), ledger);
}

pub(super) fn budgets(
    store: &mut ServerStore,
    f: &Fixture,
) -> (EpochStorageBudget, EpochIntentBudget) {
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
pub(super) fn put(f: &Fixture, n: u8, epoch: u64) -> DomainOp {
    RegistryOp::Put {
        key: f.key.clone(),
        epoch,
    }
    .domain_op(&f.group.group_id(), [n; 16])
    .unwrap()
}
pub(super) fn tombstone(f: &Fixture, n: u8) -> DomainOp {
    RegistryOp::Tombstone { key: f.key.clone() }
        .domain_op(&f.group.group_id(), [n; 16])
        .unwrap()
}
pub(super) fn journal(
    store: &mut ServerStore,
    f: &Fixture,
    domain: DomainOp,
    budget: &mut EpochStorageBudget,
    intents: &mut EpochIntentBudget,
) -> [u8; 32] {
    let id = domain.id(&f.device.device_id());
    store
        .prepare_epoch_intent(
            SERVER,
            &f.document,
            domain,
            &f.device,
            &f.group,
            &mut rng(),
            budget,
            intents,
        )
        .unwrap();
    id
}
fn replay(
    store: &mut ServerStore,
    f: &Fixture,
    id: [u8; 32],
    budget: &mut EpochStorageBudget,
    intents: &mut EpochIntentBudget,
) -> Result<(RegistryReplayOutcome, EpochRegistryState), AppError> {
    let doc = f.load(store).unwrap().doc_id();
    store.replay_registry_intent(
        SERVER,
        &f.group,
        f.key.bucket(),
        doc,
        &f.device,
        id,
        &mut rng(),
        budget,
        intents,
    )
}
fn signed(f: &Fixture, result: (RegistryReplayOutcome, EpochRegistryState)) -> SignedOp {
    let RegistryReplayOutcome::Prepared(op) = result.0 else {
        panic!("expected prepared replay")
    };
    op.open(
        &f.group
            .channel_secret(&f.device, op.doc_type, op.doc_id)
            .unwrap(),
    )
    .unwrap()
}
pub(super) fn sync(step: ReplaySync, path: &Path, bytes: u64) -> Result<(), AppError> {
    match step {
        ReplaySync::Intent => crate::store::epoch_intents::sync_intent(path, bytes),
        _ => sync_registry(path, bytes),
    }
}
pub(super) fn intent_bytes(store: &ServerStore, f: &Fixture) -> Vec<u8> {
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.document).unwrap();
    fs::read(
        store
            .dir
            .join("servers")
            .join(format!("{}.intents", blake3::hash(&scope).to_hex())),
    )
    .unwrap()
}

#[test]
fn registry_replay_saved_edit_and_exact_retry_preserve_newer_work_and_pending_intent() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let zero = f.op(0);
    f.ingest(&mut store, &zero, &mut budget).unwrap();
    let id = journal(&mut store, &f, put(&f, 1, 1), &mut budget, &mut intents);
    let original = signed(
        &f,
        replay(&mut store, &f, id, &mut budget, &mut intents).unwrap(),
    );
    assert_eq!(original.parsed_domain_op().unwrap(), Some(put(&f, 1, 1)));
    let newer = journal(&mut store, &f, put(&f, 2, 2), &mut budget, &mut intents);
    signed(
        &f,
        replay(&mut store, &f, newer, &mut budget, &mut intents).unwrap(),
    );
    let before = fs::read(f.path(&store)).unwrap();
    let ledger = intent_bytes(&store, &f);
    drop(store);
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    assert_eq!(
        signed(
            &f,
            replay(&mut store, &f, id, &mut budget, &mut intents).unwrap()
        ),
        original
    );
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
    assert_eq!(intent_bytes(&store, &f), ledger);
    assert_eq!(
        f.load(&store).unwrap().projection().unwrap().pointers[&f.key],
        2
    );
    assert_eq!(
        store
            .load_epoch_intents(SERVER, &f.document)
            .unwrap()
            .pending()
            .len(),
        2
    );
}

#[test]
fn registry_replay_holds_new_authoring_for_current_deletion_or_superseding_pointer() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let newer = f.op(5);
    f.ingest(&mut store, &newer, &mut budget).unwrap();
    let id = journal(&mut store, &f, put(&f, 1, 1), &mut budget, &mut intents);
    for expected in [
        RegistryReplayHold::SupersededPointer,
        RegistryReplayHold::DeletedPointer,
    ] {
        if expected == RegistryReplayHold::DeletedPointer {
            let delete = f
                .source
                .edit(&f.device, &f.group, &mut rng(), &tombstone(&f, 6))
                .unwrap();
            f.ingest(&mut store, &delete, &mut budget).unwrap();
        }
        let before = fs::read(f.path(&store)).unwrap();
        let ledger = intent_bytes(&store, &f);
        let (outcome, _) = replay(&mut store, &f, id, &mut budget, &mut intents).unwrap();
        assert!(matches!(outcome, RegistryReplayOutcome::Held(reason) if reason == expected));
        assert_eq!(fs::read(f.path(&store)).unwrap(), before);
        assert_eq!(intent_bytes(&store, &f), ledger);
    }
}

#[test]
fn registry_replay_failed_tombstone_save_and_flush_retry_the_exact_signed_change() {
    for mode in 0..3 {
        // refusal before write, uncertain rename, writer unwind
        let root = tempfile::tempdir().unwrap();
        let mut f = Fixture::new();
        let mut store = open(root.path());
        let (mut budget, mut intents) = budgets(&mut store, &f);
        let zero = f.op(0);
        f.ingest(&mut store, &zero, &mut budget).unwrap();
        let operation = tombstone(&f, 1);
        let id = journal(&mut store, &f, operation.clone(), &mut budget, &mut intents);
        let ledger = intent_bytes(&store, &f);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.replay_registry_intent_with_io(
                SERVER,
                &f.group,
                f.key.bucket(),
                f.source.doc_id(),
                &f.device,
                id,
                &mut rng(),
                &mut budget,
                &mut intents,
                |path, bytes| {
                    if mode == 1 {
                        atomic_write(path, bytes)?;
                    }
                    if mode == 2 {
                        panic!("injected replay writer unwind");
                    }
                    Err(AppError::Io("injected replay save failure".into()))
                },
                &mut sync,
            )
        }));
        assert!(result.is_err() || result.unwrap().is_err());
        assert!(budget.requires_reconciliation());
        assert_eq!(intent_bytes(&store, &f), ledger);
        let mut held = f.load(&store).unwrap().unit;
        let saved_signed = if mode == 1 {
            let sealed = held
                .edit_or_reseal(&f.device, &f.group, &mut rng(), &operation)
                .unwrap();
            Some(
                sealed
                    .open(
                        &f.group
                            .channel_secret(&f.device, sealed.doc_type, sealed.doc_id)
                            .unwrap(),
                    )
                    .unwrap(),
            )
        } else {
            None
        };
        drop(store);
        let mut store = open(root.path());
        let (mut budget, mut intents) = budgets(&mut store, &f);
        let actual = signed(
            &f,
            replay(&mut store, &f, id, &mut budget, &mut intents).unwrap(),
        );
        if let Some(saved) = saved_signed {
            assert_eq!(actual, saved);
        }
        assert_eq!(actual.parsed_domain_op().unwrap(), Some(operation));
        assert_eq!(f.load(&store).unwrap().op_count(), 2);
        assert!(f
            .load(&store)
            .unwrap()
            .projection()
            .unwrap()
            .tombstones
            .contains(&f.key));
        // A failed flush on an exact retry grants no ciphertext and poisons both inventories.
        let result = store.replay_registry_intent_with_io(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            id,
            &mut rng(),
            &mut budget,
            &mut intents,
            |_, _| panic!("retry must not rewrite"),
            &mut |step, path, bytes| {
                if step == ReplaySync::Intent {
                    Err(AppError::Io("intent sync failed".into()))
                } else {
                    sync(step, path, bytes)
                }
            },
        );
        assert!(result.is_err());
        assert!(budget.requires_reconciliation());
        assert_eq!(intent_bytes(&store, &f), ledger);
    }
}

#[test]
fn registry_replay_every_flush_failure_and_unwind_withholds_ciphertext() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let zero = f.op(0);
    f.ingest(&mut store, &zero, &mut budget).unwrap();
    let id = journal(&mut store, &f, put(&f, 1, 1), &mut budget, &mut intents);
    let original = signed(
        &f,
        replay(&mut store, &f, id, &mut budget, &mut intents).unwrap(),
    );
    let before = fs::read(f.path(&store)).unwrap();
    let ledger = intent_bytes(&store, &f);
    // Even a byte-identical retry must cross all three barriers. A sync error or unwind must
    // never expose Prepared, trust stale accounting, rewrite the change, or retire the intent.
    for target in [ReplaySync::Source, ReplaySync::Intent, ReplaySync::Epoch] {
        for unwind in [false, true] {
            let (mut budget, mut intents) = budgets(&mut store, &f);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                store.replay_registry_intent_with_io(
                    SERVER,
                    &f.group,
                    f.key.bucket(),
                    f.source.doc_id(),
                    &f.device,
                    id,
                    &mut rng(),
                    &mut budget,
                    &mut intents,
                    |_, _| panic!("exact replay must not rewrite the epoch"),
                    &mut |step, path, bytes| {
                        if step == target {
                            if unwind {
                                panic!("injected {target:?} sync unwind");
                            }
                            Err(AppError::Io(format!("injected {target:?} sync failure")))
                        } else {
                            sync(step, path, bytes)
                        }
                    },
                )
            }));
            if unwind {
                assert!(result.is_err());
            } else {
                assert!(result.unwrap().is_err());
            }
            assert!(budget.requires_reconciliation());
            assert_eq!(fs::read(f.path(&store)).unwrap(), before);
            assert_eq!(intent_bytes(&store, &f), ledger);
            // Reopening and reconciling after any failed barrier still finds the same signed
            // operation and its durable intent, so a later successful retry can publish it.
            drop(store);
            store = open(root.path());
            let (mut budget, mut intents) = budgets(&mut store, &f);
            assert_eq!(
                signed(
                    &f,
                    replay(&mut store, &f, id, &mut budget, &mut intents).unwrap()
                ),
                original
            );
            assert_eq!(fs::read(f.path(&store)).unwrap(), before);
            assert_eq!(intent_bytes(&store, &f), ledger);
        }
    }
}

#[test]
fn registry_replay_rejects_unknown_foreign_conflicting_stale_and_closed_requests() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let zero = f.op(0);
    f.ingest(&mut store, &zero, &mut budget).unwrap();
    let id = journal(&mut store, &f, put(&f, 1, 1), &mut budget, &mut intents);
    let before = fs::read(f.path(&store)).unwrap();
    let ledger = intent_bytes(&store, &f);
    assert!(replay(&mut store, &f, [255; 32], &mut budget, &mut intents).is_err());
    assert!(store
        .replay_registry_intent(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id() ^ 1,
            &f.device,
            id,
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .is_err());
    let peer = MlsDevice::generate().unwrap();
    assert!(store
        .replay_registry_intent(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &peer,
            id,
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .is_err());
    f.group
        .add_member(&f.device, peer.key_package().unwrap())
        .unwrap();
    assert!(store
        .replay_registry_intent(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &peer,
            id,
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .unwrap_err()
        .to_string()
        .contains("another device"));
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
    assert_eq!(intent_bytes(&store, &f), ledger);
    // Same nonce/id arrived via another path with a different body: neither journal nor log wins
    // silently; the exact-current-log retry exception must reject the mismatch.
    let conflict = f
        .source
        .edit(&f.device, &f.group, &mut rng(), &put(&f, 1, 99))
        .unwrap();
    f.ingest(&mut store, &conflict, &mut budget).unwrap();
    assert!(replay(&mut store, &f, id, &mut budget, &mut intents).is_err());
    let seal = f.receipt(7);
    store
        .seal_registry_epoch(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            seal,
            0,
            &mut rng(),
            &mut budget,
        )
        .unwrap();
    assert!(replay(&mut store, &f, id, &mut budget, &mut intents).is_err());
    assert_eq!(intent_bytes(&store, &f), ledger);
}
