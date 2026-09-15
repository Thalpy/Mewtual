//! Pass orchestration tests use the real checked replay adapter and vault. Only the failure
//! seam injects IO errors/unwinds; authority, intent selection and publication gating stay real.
use super::replay::{budgets, intent_bytes, journal, put, sync, tombstone};
use super::*;
use crate::store::epoch_registry::pass::REPLAY_INTERVAL_MS;
use catcoms_replication::SignedOp;
use catcoms_rt::{Clock, ManualClock};

fn start(
    store: &ServerStore,
    f: &Fixture,
    budget: &mut EpochStorageBudget,
    intents: &mut EpochIntentBudget,
) -> RegistryReplayPass {
    store
        .begin_registry_replay(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            budget,
            intents,
        )
        .unwrap()
}

fn step(
    store: &mut ServerStore,
    f: &Fixture,
    pass: &mut RegistryReplayPass,
    clock: &dyn Clock,
    budget: &mut EpochStorageBudget,
    intents: &mut EpochIntentBudget,
) -> Result<RegistryReplayStep, AppError> {
    store.step_registry_replay(
        pass,
        &f.group,
        &f.device,
        clock,
        &mut rng(),
        budget,
        intents,
    )
}

fn prepared(f: &Fixture, result: RegistryReplayStep) -> ([u8; 32], RegistryReplayTicket, SignedOp) {
    let RegistryReplayStep::Prepared {
        intent_id,
        ticket,
        op,
        ..
    } = result
    else {
        panic!("expected prepared replay, got {result:?}");
    };
    let signed = op
        .open(
            &f.group
                .channel_secret(&f.device, op.doc_type, op.doc_id)
                .unwrap(),
        )
        .unwrap();
    (intent_id, ticket, signed)
}

#[test]
fn registry_pass_snapshots_only_own_ids_visits_once_and_never_retires_on_submission() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    // Empty traversal says nothing about epoch existence: there is not even a source yet.
    let mut empty = start(&store, &f, &mut budget, &mut intents);
    let clock = ManualClock::new(0);
    assert_eq!(empty.progress(), RegistryReplayProgress::default());
    assert!(matches!(
        step(
            &mut store,
            &f,
            &mut empty,
            &clock,
            &mut budget,
            &mut intents
        )
        .unwrap(),
        RegistryReplayStep::Complete
    ));
    let zero = f.op(0);
    f.ingest(&mut store, &zero, &mut budget).unwrap();
    let mut expected = Vec::new();
    for n in 1..=3 {
        expected.push(journal(
            &mut store,
            &f,
            put(&f, n, 1),
            &mut budget,
            &mut intents,
        ));
    }
    let peer = MlsDevice::generate().unwrap();
    f.group
        .add_member(&f.device, peer.key_package().unwrap())
        .unwrap();
    store
        .prepare_epoch_intent(
            SERVER,
            &f.document,
            put(&f, 4, 1),
            &peer,
            &f.group,
            &mut rng(),
            &mut budget,
            &mut intents,
        )
        .unwrap();
    let mut pass = start(&store, &f, &mut budget, &mut intents);
    assert_eq!(pass.progress().selected, 3);
    journal(&mut store, &f, put(&f, 5, 1), &mut budget, &mut intents);
    let ledger = intent_bytes(&store, &f);
    expected.sort();
    for (index, id) in expected.into_iter().enumerate() {
        let result = step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap();
        assert_eq!(format!("{result:?}"), "Prepared { .. }");
        let (actual, ticket, _) = prepared(&f, result);
        assert_eq!(actual, id);
        assert_eq!(pass.progress().visited, index);
        // Neither polling nor a very long elapsed interval can invent submission evidence.
        clock.advance_ms(10_000);
        assert!(matches!(
            step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap(),
            RegistryReplayStep::AwaitingSubmission
        ));
        pass.submitted(&ticket).unwrap();
        assert!(pass.submitted(&ticket).is_err());
        assert_eq!(intent_bytes(&store, &f), ledger);
    }
    assert_eq!(
        pass.progress(),
        RegistryReplayProgress {
            selected: 3,
            visited: 3,
            submitted: 3,
            held: 0
        }
    );
    assert!(matches!(
        step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap(),
        RegistryReplayStep::Complete
    ));
    // The later own intent needs another pass, while foreign saved work is never impersonated.
    assert_eq!(
        start(&store, &f, &mut budget, &mut intents)
            .progress()
            .selected,
        4
    );
    assert_eq!(
        store
            .load_epoch_intents(SERVER, &f.document)
            .unwrap()
            .pending()
            .len(),
        5
    );
    assert!(!format!("{pass:?}").contains("private-registry-cat"));
}

#[test]
fn registry_pass_failed_submission_reseals_exactly_and_stale_cross_pass_tickets_do_not_advance() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let zero = f.op(0);
    f.ingest(&mut store, &zero, &mut budget).unwrap();
    let id = journal(&mut store, &f, put(&f, 1, 1), &mut budget, &mut intents);
    let mut pass = start(&store, &f, &mut budget, &mut intents);
    let mut other = start(&store, &f, &mut budget, &mut intents);
    let clock = ManualClock::new(0);
    let (actual, old, signed) = prepared(
        &f,
        step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap(),
    );
    assert_eq!(actual, id);
    let (_, foreign, _) = prepared(
        &f,
        step(
            &mut store,
            &f,
            &mut other,
            &clock,
            &mut budget,
            &mut intents,
        )
        .unwrap(),
    );
    assert!(pass.submitted(&foreign).is_err());
    assert!(pass.retry_submission(&foreign).is_err());
    assert!(pass.retry_failed().is_err());
    let before = fs::read(f.path(&store)).unwrap();
    let ledger = intent_bytes(&store, &f);
    pass.retry_submission(&old).unwrap();
    assert!(pass.submitted(&old).is_err());
    clock.set_wall_ms(u64::MAX); // wall correction is not permission for earlier replay work
    assert!(matches!(
        step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap(),
        RegistryReplayStep::Wait {
            retry_at_ms: REPLAY_INTERVAL_MS
        }
    ));
    for monotonic in [0, REPLAY_INTERVAL_MS - 1] {
        clock.set_ms(monotonic);
        assert!(matches!(
            step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap(),
            RegistryReplayStep::Wait {
                retry_at_ms: REPLAY_INTERVAL_MS
            }
        ));
    }
    clock.set_ms(REPLAY_INTERVAL_MS);
    let (actual, fresh, retry) = prepared(
        &f,
        step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap(),
    );
    assert_eq!(actual, id);
    assert_eq!(retry, signed);
    assert!(pass.submitted(&old).is_err());
    assert!(pass.retry_submission(&old).is_err());
    assert_eq!(pass.progress().visited, 0);
    pass.submitted(&fresh).unwrap();
    other.submitted(&foreign).unwrap();
    assert_eq!(pass.progress().submitted, 1);
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
    assert_eq!(intent_bytes(&store, &f), ledger);
}

#[test]
fn registry_pass_holds_are_visited_once_but_remain_pending_for_recovery() {
    for deleted in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut f = Fixture::new();
        let mut store = open(root.path());
        let (mut budget, mut intents) = budgets(&mut store, &f);
        let newest = f.op(9);
        f.ingest(&mut store, &newest, &mut budget).unwrap();
        if deleted {
            let deletion = f
                .source
                .edit(&f.device, &f.group, &mut rng(), &tombstone(&f, 10))
                .unwrap();
            f.ingest(&mut store, &deletion, &mut budget).unwrap();
        }
        let id = journal(&mut store, &f, put(&f, 1, 1), &mut budget, &mut intents);
        let before = fs::read(f.path(&store)).unwrap();
        let ledger = intent_bytes(&store, &f);
        let mut pass = start(&store, &f, &mut budget, &mut intents);
        let clock = ManualClock::new(0);
        let RegistryReplayStep::Held {
            intent_id, reason, ..
        } = step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap()
        else {
            panic!("expected held edit");
        };
        assert_eq!(intent_id, id);
        assert_eq!(
            reason,
            if deleted {
                RegistryReplayHold::DeletedPointer
            } else {
                RegistryReplayHold::SupersededPointer
            }
        );
        for _ in 0..2 {
            assert!(matches!(
                step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap(),
                RegistryReplayStep::Complete
            ));
        }
        assert_eq!(
            pass.progress(),
            RegistryReplayProgress {
                selected: 1,
                visited: 1,
                submitted: 0,
                held: 1
            }
        );
        assert_eq!(fs::read(f.path(&store)).unwrap(), before);
        assert_eq!(intent_bytes(&store, &f), ledger);
    }
}

#[test]
fn registry_pass_wrong_scope_mount_stale_epoch_and_removed_author_cannot_prepare() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let zero = f.op(0);
    f.ingest(&mut store, &zero, &mut budget).unwrap();
    journal(&mut store, &f, put(&f, 1, 1), &mut budget, &mut intents);
    let mut pass = start(&store, &f, &mut budget, &mut intents);
    let other = Fixture::new();
    let clock = ManualClock::new(0);
    assert!(store
        .step_registry_replay(
            &mut pass,
            &other.group,
            &f.device,
            &clock,
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .is_err());
    assert!(store
        .step_registry_replay(
            &mut pass,
            &f.group,
            &other.device,
            &clock,
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .is_err());
    assert!(pass.retry_failed().is_err()); // wrong callers cannot consume the legitimate pass
    let mut stale = store
        .begin_registry_replay(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id() ^ 1,
            &f.device,
            &mut budget,
            &mut intents,
        )
        .unwrap();
    assert!(step(
        &mut store,
        &f,
        &mut stale,
        &clock,
        &mut budget,
        &mut intents
    )
    .is_err());
    assert!(matches!(
        step(
            &mut store,
            &f,
            &mut stale,
            &clock,
            &mut budget,
            &mut intents
        )
        .unwrap(),
        RegistryReplayStep::Paused
    ));
    // A dropped/reopened mount cannot continue a cursor even with identical saved bytes/keys.
    let before = fs::read(f.path(&store)).unwrap();
    let ledger = intent_bytes(&store, &f);
    drop(store);
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    assert!(step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).is_err());
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
    assert_eq!(intent_bytes(&store, &f), ledger);
    let peer = MlsDevice::generate().unwrap();
    f.group
        .add_member(&f.device, peer.key_package().unwrap())
        .unwrap();
    store
        .prepare_epoch_intent(
            SERVER,
            &f.document,
            put(&f, 2, 2),
            &peer,
            &f.group,
            &mut rng(),
            &mut budget,
            &mut intents,
        )
        .unwrap();
    let mut peer_pass = store
        .begin_registry_replay(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &peer,
            &mut budget,
            &mut intents,
        )
        .unwrap();
    assert_eq!(peer_pass.progress().selected, 1);
    f.group.remove_member(&f.device, &peer.device_id()).unwrap();
    assert!(store
        .step_registry_replay(
            &mut peer_pass,
            &f.group,
            &peer,
            &clock,
            &mut rng(),
            &mut budget,
            &mut intents
        )
        .is_err());
    assert_eq!(peer_pass.progress().visited, 0);
    assert!(store
        .begin_registry_replay(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &peer,
            &mut budget,
            &mut intents
        )
        .is_err());
}

#[test]
fn registry_pass_post_rename_failure_and_unwind_pause_without_skipping_or_early_retry() {
    for unwind in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut f = Fixture::new();
        let mut store = open(root.path());
        let (mut budget, mut intents) = budgets(&mut store, &f);
        let zero = f.op(0);
        f.ingest(&mut store, &zero, &mut budget).unwrap();
        let id = journal(&mut store, &f, put(&f, 1, 1), &mut budget, &mut intents);
        let ledger = intent_bytes(&store, &f);
        let mut pass = start(&store, &f, &mut budget, &mut intents);
        let clock = ManualClock::new(0);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pass.step_with(&clock, |selected| {
                assert_eq!(selected, id);
                store.replay_registry_intent_with_io(
                    SERVER,
                    &f.group,
                    f.key.bucket(),
                    f.source.doc_id(),
                    &f.device,
                    selected,
                    &mut rng(),
                    &mut budget,
                    &mut intents,
                    |path, bytes| {
                        atomic_write(path, bytes)?;
                        if unwind {
                            panic!("injected post-rename replay-pass unwind");
                        }
                        Err(AppError::Io(
                            "injected post-rename replay-pass failure".into(),
                        ))
                    },
                    &mut sync,
                )
            })
        }));
        assert!(result.is_err() || result.unwrap().is_err());
        assert!(budget.requires_reconciliation());
        assert_eq!(pass.progress().visited, 0);
        assert!(matches!(
            step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap(),
            RegistryReplayStep::Paused
        ));
        let before = fs::read(f.path(&store)).unwrap();
        pass.retry_failed().unwrap();
        assert!(matches!(
            step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap(),
            RegistryReplayStep::Wait {
                retry_at_ms: REPLAY_INTERVAL_MS
            }
        ));
        clock.advance_ms(REPLAY_INTERVAL_MS);
        // Explicit retry is not budget repair. Refusal pauses again without advancing.
        assert!(step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).is_err());
        let (mut budget, mut intents) = budgets(&mut store, &f);
        pass.retry_failed().unwrap();
        clock.advance_ms(REPLAY_INTERVAL_MS);
        let (selected, ticket, _) = prepared(
            &f,
            step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap(),
        );
        assert_eq!(selected, id);
        pass.submitted(&ticket).unwrap();
        assert_eq!(fs::read(f.path(&store)).unwrap(), before);
        assert_eq!(intent_bytes(&store, &f), ledger);
    }
}

#[test]
fn registry_pass_clock_exhaustion_refuses_before_work_and_never_wraps() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    journal(&mut store, &f, put(&f, 1, 1), &mut budget, &mut intents);
    let mut pass = start(&store, &f, &mut budget, &mut intents);
    let clock = ManualClock::new(u64::MAX - REPLAY_INTERVAL_MS);
    let mut attempts = 0;
    assert!(pass
        .step_with(&clock, |_| {
            attempts += 1;
            Err(AppError::Io("test attempt failed".into()))
        })
        .is_err());
    assert_eq!(attempts, 1);
    pass.retry_failed().unwrap();
    assert!(matches!(
        pass.step_with(&clock, |_| panic!("too early")).unwrap(),
        RegistryReplayStep::Wait {
            retry_at_ms: u64::MAX
        }
    ));
    clock.set_ms(u64::MAX);
    for _ in 0..2 {
        assert!(pass
            .step_with(&clock, |_| panic!("overflow must not run replay"))
            .is_err());
        assert_eq!(pass.progress().visited, 0);
        pass.retry_failed().unwrap();
    }
}

#[test]
fn registry_pass_rotation_requires_a_fresh_pass_and_does_not_restore_retired_ids() {
    use super::settlement::{source_fixture, TestSource};
    let root = tempfile::tempdir().unwrap();
    let TestSource {
        f,
        mut store,
        mut budget,
        close,
        receipt,
    } = source_fixture(root.path(), true);
    let (_, mut intents) = budgets(&mut store, &f);
    journal(&mut store, &f, put(&f, 1, 1), &mut budget, &mut intents);
    let mut pass = start(&store, &f, &mut budget, &mut intents);
    let excluded = journal(&mut store, &f, put(&f, 10, 10), &mut budget, &mut intents);
    let clock = ManualClock::new(0);
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
    assert!(step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).is_err());
    let (_, successor) = store
        .install_registry_checkpoint(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            &receipt.encode(),
            &close,
            0,
            &clock,
            &mut rng(),
            &mut budget,
            &mut intents,
        )
        .unwrap();
    pass.retry_failed().unwrap();
    clock.advance_ms(REPLAY_INTERVAL_MS);
    assert!(
        step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents)
            .unwrap_err()
            .to_string()
            .contains("saved replay intent is missing")
    );
    assert_eq!(pass.progress().visited, 0);
    let mut fresh = store
        .begin_registry_replay(
            SERVER,
            &f.group,
            f.key.bucket(),
            successor.doc_id(),
            &f.device,
            &mut budget,
            &mut intents,
        )
        .unwrap();
    assert_eq!(fresh.progress().selected, 1);
    let (id, ticket, op) = prepared(
        &f,
        step(
            &mut store,
            &f,
            &mut fresh,
            &clock,
            &mut budget,
            &mut intents,
        )
        .unwrap(),
    );
    assert_eq!(id, excluded);
    assert_eq!(op.doc_id, successor.doc_id());
    fresh.submitted(&ticket).unwrap();
    assert_eq!(
        store
            .load_epoch_intents(SERVER, &f.document)
            .unwrap()
            .pending()
            .len(),
        1
    );
}

#[test]
fn registry_pass_abandoned_prepared_result_retries_exactly_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let zero = f.op(0);
    f.ingest(&mut store, &zero, &mut budget).unwrap();
    let id = journal(&mut store, &f, put(&f, 1, 1), &mut budget, &mut intents);
    let mut pass = start(&store, &f, &mut budget, &mut intents);
    let clock = ManualClock::new(0);
    let (_, old_ticket, signed) = prepared(
        &f,
        step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap(),
    );
    let before = fs::read(f.path(&store)).unwrap();
    let ledger = intent_bytes(&store, &f);
    drop(pass); // no submission acknowledgement, as if the caller lost the prepared result
    drop(store);
    let mut store = open(root.path());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let mut pass = start(&store, &f, &mut budget, &mut intents);
    let (actual, ticket, retry) = prepared(
        &f,
        step(&mut store, &f, &mut pass, &clock, &mut budget, &mut intents).unwrap(),
    );
    assert_eq!(actual, id);
    assert_eq!(retry, signed);
    assert!(pass.submitted(&old_ticket).is_err());
    pass.submitted(&ticket).unwrap();
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
    assert_eq!(intent_bytes(&store, &f), ledger);
}

#[test]
fn registry_pass_maximal_ledger_keeps_only_bounded_ids_and_checks_inventory_at_begin() {
    use catcoms_replication::{epoch::MAX_INTENTS_PER_DOCUMENT, IntentLedger};
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let mut store = open(root.path());
    let (mut stale_budget, mut stale_intents) = budgets(&mut store, &f);
    let mut ledger = IntentLedger::new(f.document.clone());
    for n in 0..MAX_INTENTS_PER_DOCUMENT {
        let mut operation = put(&f, 0, 0);
        operation.nonce = (n as u128).to_be_bytes();
        ledger.prepare(f.device.device_id(), operation).unwrap();
    }
    let mut extra = put(&f, 0, 0);
    extra.nonce = (MAX_INTENTS_PER_DOCUMENT as u128).to_be_bytes();
    assert!(ledger.prepare(f.device.device_id(), extra).is_err());
    // One authentic bulk fixture, avoiding ten thousand intentionally expensive atomic saves.
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.document).unwrap();
    let mut e = Encoder::new();
    e.put_bytes(&scope).unwrap();
    e.put_bytes(&ledger.encode().unwrap()).unwrap();
    let path = store
        .dir
        .join("servers")
        .join(format!("{}.intents", blake3::hash(&scope).to_hex()));
    let sealed = frame(&seal(&store.keys.db_key().unwrap(), &e.finish(), &mut rng()).unwrap());
    atomic_write(&path, &sealed).unwrap();
    assert!(store
        .begin_registry_replay(
            SERVER,
            &f.group,
            f.key.bucket(),
            f.source.doc_id(),
            &f.device,
            &mut stale_budget,
            &mut stale_intents
        )
        .is_err());
    assert!(stale_budget.requires_reconciliation());
    let (mut budget, mut intents) = budgets(&mut store, &f);
    let pass = start(&store, &f, &mut budget, &mut intents);
    assert_eq!(pass.progress().selected, MAX_INTENTS_PER_DOCUMENT);
    assert_eq!(fs::read(&path).unwrap(), sealed);
}
