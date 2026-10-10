//! Flow R at store level (design 6.4.2): R1, R2 and R3 held against the synchronous resolver.
//!
//! The contract is equivalence. Every test here that resolves compares the staged result with a
//! copy of the same vault resolved by `resolve_studio_handoff`, the resolver the fences keep, by
//! every record's bytes **and** by the intents record's authenticated plaintext, which
//! `canonical()` deliberately omits.
use super::*;
use crate::store::epoch_recovery::inventory::inline_studio_validations_for_test;
use crate::store::epoch_studio::source::studio_full_restores_for_test;
use crate::store::{StudioResolveStart, StudioResolved};

/// The intents record by authenticated plaintext digest and physical size: what `canonical()`
/// leaves out, and what a resolution writes.
fn intents(store: &ServerStore, f: &Fixture) -> (blake3::Hash, u64) {
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let record = store.read_scoped_intent_plain(&scope).unwrap().unwrap();
    (blake3::hash(&record.plain), record.physical_bytes)
}

/// The source record by authenticated plaintext digest and physical size, as R1's stamp sees it.
fn source_record(store: &ServerStore, f: &Fixture) -> (blake3::Hash, u64) {
    let record = store
        .read_studio_record(&scope_bytes(SERVER, &f.logical).unwrap())
        .unwrap()
        .unwrap();
    (blake3::hash(&record.plain), record.physical_bytes)
}

/// The target source's inventory key, for counting its inline validations alone.
fn source_key(f: &Fixture) -> [u8; 32] {
    *blake3::hash(&scope_bytes(SERVER, &f.logical).unwrap()).as_bytes()
}

/// The fences' synchronous resolver, as the control.
fn resolve_synchronously(f: &Fixture, store: &mut ServerStore) -> Result<(), AppError> {
    let mut b = budget(store, f);
    store.resolve_studio_handoff(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
}

fn capture(f: &Fixture, store: &mut ServerStore) -> Box<crate::store::StudioResolveCapture> {
    match store
        .capture_studio_resolution(SERVER, &f.group, f.target, &f.device)
        .unwrap()
    {
        StudioResolveStart::Captured(capture) => capture,
        StudioResolveStart::NotPrepared => panic!("R1 found no Prepared record"),
        StudioResolveStart::Hold => panic!("R1 held a record the fixture meant to resolve"),
    }
}

/// R3 exactly as the runtime runs it: warm the inventory, build the budget, commit.
fn commit(
    f: &Fixture,
    store: &mut ServerStore,
    mut plan: crate::store::StudioResolvePlan,
    hooks: &mut WriteHooks<'_>,
) -> Result<StudioResolved, AppError> {
    store.warm_studio_resolution(&mut plan);
    let mut b = budget(store, f);
    store.commit_studio_resolution_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        plan,
        &mut rng(),
        &mut b,
        hooks,
    )
}

/// The two states an interrupted H5 leaves that resolution acts on: the Source write landed
/// (Complete evidence), or it never did (Absent).
const OUTCOMES: [(WriteTag, StudioResolved); 2] = [
    (WriteTag::Completed, StudioResolved::Completed),
    (WriteTag::Source, StudioResolved::Returned),
];

/// The equivalence oracle, for an Index and a Flipnote target and both outcomes, from a cold
/// vault: a fresh `open`, so nothing is cached. It also proves where the restores went: none in
/// R1 or R3 on this thread, counted both as full restores and as the inventory's inline
/// validations of this source (design 6.4.3, HIGH-2).
#[test]
fn flow_r_writes_exactly_what_the_synchronous_resolver_writes() {
    for art in [false, true] {
        for (crash_at, outcome) in OUTCOMES {
            let f = Fixture::new(art);
            let staged = tempfile::tempdir().unwrap();
            interrupted_handoff(&f, staged.path(), crash_at);
            let control = tempfile::tempdir().unwrap();
            copy_vault(staged.path(), control.path());

            let mut synchronous = open(control.path());
            let cold = inline_studio_validations_for_test(source_key(&f));
            resolve_synchronously(&f, &mut synchronous).unwrap();
            assert!(
                inline_studio_validations_for_test(source_key(&f)) > cold,
                "precondition: the synchronous path validates this cold source inline, so the \
                 counter can see what Flow R must avoid"
            );

            let mut store = open(staged.path());
            let (restores, inline) = (
                studio_full_restores_for_test(),
                inline_studio_validations_for_test(source_key(&f)),
            );
            let capture = capture(&f, &mut store);
            assert_eq!(
                (
                    studio_full_restores_for_test(),
                    inline_studio_validations_for_test(source_key(&f))
                ),
                (restores, inline),
                "R1 restored the source under custody"
            );
            let plan = capture.resolve().expect("R2 resolves the record");
            let (restores, inline) = (
                studio_full_restores_for_test(),
                inline_studio_validations_for_test(source_key(&f)),
            );
            let resolved = commit(&f, &mut store, plan, &mut WriteHooks::None).unwrap();
            assert_eq!(
                (
                    studio_full_restores_for_test(),
                    inline_studio_validations_for_test(source_key(&f))
                ),
                (restores, inline),
                "R3 restored the source under custody (art {art}, {crash_at:?})"
            );

            assert_eq!(resolved, outcome, "R3 reported the wrong outcome");
            assert_eq!(
                canonical(&store),
                canonical(&synchronous),
                "Flow R wrote different records from the synchronous resolver"
            );
            assert_eq!(
                intents(&store, &f),
                intents(&synchronous, &f),
                "Flow R wrote a different intents record from the synchronous resolver"
            );
            assert!(!store
                .load_epoch_intents(SERVER, &f.logical)
                .unwrap()
                .handoff_prepared());
        }
    }
}

/// The warm install's precondition, from a warm but **stale** cache (re-review of 6.4.2, M-1).
///
/// An H5 that refuses after its Source write, with no restart, leaves the inventory cache holding
/// the source as H1's scan saw it. Unless R1 evicts that entry, `IfVacant` refuses R2's
/// validation of the new bytes and R3's scan restores the source inline after all. A fresh `open`
/// empties the cache, which is why the oracle above cannot see this.
#[test]
fn flow_r_restores_nothing_under_custody_from_a_stale_cache() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    // The interruption builds its own budget, which caches the pre-H5 source.
    super::fences::interrupt(&f, &mut store, basis, WriteTag::Completed);
    assert!(store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .handoff_prepared());

    let plan = capture(&f, &mut store).resolve().unwrap();
    let inline = inline_studio_validations_for_test(source_key(&f));
    let resolved = commit(&f, &mut store, plan, &mut WriteHooks::None).unwrap();
    assert_eq!(resolved, StudioResolved::Completed);
    assert_eq!(
        inline_studio_validations_for_test(source_key(&f)),
        inline,
        "a stale cached version refused the warm install, so R3 validated the source inline"
    );
}

/// R1's Hold early exit (design 6.4.3, question 4): a Prepared branch whose source went back to
/// the Closing epoch is Hold from the framing alone. R1 refuses with no restore and nothing for a
/// worker, and the vault is unchanged; the synchronous resolver refuses the same record.
#[test]
fn flow_r_holds_at_r1_with_no_restore() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _, closing_record) = super::prepare_keeping_closing(&f, &mut store);
    super::fences::interrupt(&f, &mut store, basis, WriteTag::Source);
    write_for_test(&f.path(&store), &closing_record).unwrap();
    let (records, intent) = (canonical(&store), intents(&store, &f));

    let (restores, inline) = (
        studio_full_restores_for_test(),
        inline_studio_validations_for_test(source_key(&f)),
    );
    let held = store
        .capture_studio_resolution(SERVER, &f.group, f.target, &f.device)
        .unwrap();
    assert!(
        matches!(held, StudioResolveStart::Hold),
        "R1 did not stop a Hold record before a worker"
    );
    assert_eq!(
        (
            studio_full_restores_for_test(),
            inline_studio_validations_for_test(source_key(&f))
        ),
        (restores, inline),
        "R1's Hold exit restored the source"
    );
    assert_eq!((canonical(&store), intents(&store, &f)), (records, intent));
    assert!(resolve_synchronously(&f, &mut store).is_err());
}

/// HIGH-1 (design 6.4.3). A received operation lands on the Prepared destination between R2 and
/// R3, a legitimate history-preserving write, so the source no longer matches the stamp. R3 must
/// still resolve, now, through the synchronous resolver, report the right outcome, and leave
/// exactly what that resolver leaves when the same operation arrives first.
#[test]
fn flow_r_resolves_at_once_when_a_received_operation_changed_the_source() {
    for (crash_at, outcome) in OUTCOMES {
        let f = Fixture::new(true);
        let staged = tempfile::tempdir().unwrap();
        interrupted_handoff(&f, staged.path(), crash_at);
        let control = tempfile::tempdir().unwrap();
        copy_vault(staged.path(), control.path());

        // A separate sender's signed edit, built once so both copies receive the same bytes.
        let packet = {
            let store = open(staged.path());
            let mut sender = f.load(&store).unwrap();
            let mut late = f.title();
            late.nonce = [81; 16];
            sender
                .unit
                .edit_or_reseal(&f.device, &f.group, &mut rng(), &late, 100)
                .unwrap()
        };
        let receive = |store: &mut ServerStore| {
            let mut b = budget(store, &f);
            store
                .ingest_studio_epoch(
                    SERVER,
                    &f.group,
                    f.target,
                    &f.device,
                    &packet,
                    &mut rng(),
                    &mut b,
                )
                .expect("a Prepared destination takes a history-preserving write")
        };

        let mut synchronous = open(control.path());
        receive(&mut synchronous);
        resolve_synchronously(&f, &mut synchronous).unwrap();

        let mut store = open(staged.path());
        let plan = capture(&f, &mut store).resolve().unwrap();
        let before = source_record(&store, &f);
        receive(&mut store);
        assert_ne!(
            source_record(&store, &f),
            before,
            "precondition: the received operation changed the Prepared destination's source, \
             by plaintext digest, which is what the stamp compares"
        );
        let resolved = commit(&f, &mut store, plan, &mut WriteHooks::None)
            .expect("a changed source must resolve at once, not refuse and back off");
        assert_eq!(resolved, outcome, "the fallback reported the wrong outcome");
        assert_eq!(canonical(&store), canonical(&synchronous));
        assert_eq!(intents(&store, &f), intents(&synchronous, &f));
    }
}

/// A fence resolves the record while R2 is detached. R3 sees the intent record changed and no
/// longer Prepared, writes nothing, and reports it superseded; the fence's resolution stands.
#[test]
fn flow_r_stands_aside_when_a_fence_resolved_first() {
    for (crash_at, _) in OUTCOMES {
        let f = Fixture::new(true);
        let root = tempfile::tempdir().unwrap();
        interrupted_handoff(&f, root.path(), crash_at);
        let mut store = open(root.path());
        let plan = capture(&f, &mut store).resolve().unwrap();
        resolve_synchronously(&f, &mut store).unwrap();
        let (records, intent) = (canonical(&store), intents(&store, &f));

        let resolved = commit(&f, &mut store, plan, &mut WriteHooks::None)
            .expect("R3 refused a record a fence had already resolved");
        assert_eq!(resolved, StudioResolved::Superseded);
        assert_eq!(
            (canonical(&store), intents(&store, &f)),
            (records, intent),
            "R3 wrote over a fence's resolution"
        );
    }
}

/// R3's intent comparison: a same-size authenticated replacement of the intent record between R1
/// and R3 is refused with nothing written. (The replaced record no longer decodes, so this is the
/// "changed and still unreadable" arm: a refusal, never a superseded success.)
#[test]
fn flow_r_refuses_an_intent_record_replaced_since_the_capture() {
    let f = Fixture::new(true);
    let root = tempfile::tempdir().unwrap();
    interrupted_handoff(&f, root.path(), WriteTag::Completed);
    let mut store = open(root.path());
    let plan = capture(&f, &mut store).resolve().unwrap();
    let mut b = budget(&mut store, &f);
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let path = store.epoch_intent_path(&scope);
    super::fences::replace_at_the_same_size(&store, &path);
    let (records, replaced) = (canonical(&store), fs::read(&path).unwrap());

    let mut plan = plan;
    store.warm_studio_resolution(&mut plan);
    let refused = store.commit_studio_resolution_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        plan,
        &mut rng(),
        &mut b,
        &mut WriteHooks::None,
    );
    assert!(
        refused.is_err(),
        "R3 resolved against an intent record that changed since the capture: {refused:?}"
    );
    assert_eq!(canonical(&store), records);
    assert_eq!(
        fs::read(&path).unwrap(),
        replaced,
        "R3 wrote the intent record"
    );
}

/// R3's mount check: a plan captured before a reopen is refused after it, with nothing written,
/// although every byte on disk is unchanged. (R3's `current_member` is not separately pinned here:
/// a device removed from its own group needs an MLS commit this fixture cannot make, and a
/// different device is refused first by the stamp's actor and key.)
#[test]
fn flow_r_refuses_a_plan_from_before_a_reopen() {
    let f = Fixture::new(true);
    let root = tempfile::tempdir().unwrap();
    interrupted_handoff(&f, root.path(), WriteTag::Completed);

    let mut store = open(root.path());
    let plan = capture(&f, &mut store).resolve().unwrap();
    drop(store);
    let mut store = open(root.path());
    let (records, intent) = (canonical(&store), intents(&store, &f));
    let refused = commit(&f, &mut store, plan, &mut WriteHooks::None);
    assert!(
        refused.is_err(),
        "a plan from before a reopen was committed: {refused:?}"
    );
    assert_eq!((canonical(&store), intents(&store, &f)), (records, intent));
}

/// The shape predicate (design 6.4.3, M-3): a plan whose next state was built for the other arm
/// is refused with nothing written. A Completed-shaped state in the Absent arm would skip the flush
/// and the reference check.
#[test]
fn flow_r_refuses_a_next_state_built_for_the_other_outcome() {
    let f = Fixture::new(true);
    let absent_root = tempfile::tempdir().unwrap();
    let (absent_basis, _) = interrupted_handoff(&f, absent_root.path(), WriteTag::Source);
    let complete_root = tempfile::tempdir().unwrap();
    let (complete_basis, _) = interrupted_handoff(&f, complete_root.path(), WriteTag::Completed);
    assert_eq!(
        absent_basis, complete_basis,
        "precondition: both copies hold the same branch"
    );

    let mut absent_store = open(absent_root.path());
    let absent = capture(&f, &mut absent_store).resolve().unwrap();
    let mut complete_store = open(complete_root.path());
    let complete = capture(&f, &mut complete_store).resolve().unwrap();
    assert!(!absent.is_complete_for_test() && complete.is_complete_for_test());
    let (completed_next, returned_next) = (complete.next_for_test(), absent.next_for_test());

    for (store, plan, wrong) in [
        (&mut absent_store, absent, completed_next),
        (&mut complete_store, complete, returned_next),
    ] {
        let (records, intent) = (canonical(store), intents(store, &f));
        let refused = commit(
            &f,
            store,
            plan.with_next_for_test(wrong),
            &mut WriteHooks::None,
        );
        assert!(
            refused.is_err(),
            "R3 wrote a next state shaped for the other outcome: {refused:?}"
        );
        assert_eq!((canonical(store), intents(store, &f)), (records, intent));
    }
}

/// R3 checks R2's accounting fact for both outcomes (design 6.4.3, M-2): a plan claiming a
/// different protocol-byte figure for the source is refused before any write, as the resolver's
/// own `verify_record` would refuse a restore that disagreed with the inventory.
#[test]
fn flow_r_refuses_a_plan_whose_accounting_disagrees_with_the_inventory() {
    for (crash_at, _) in OUTCOMES {
        let f = Fixture::new(true);
        let root = tempfile::tempdir().unwrap();
        interrupted_handoff(&f, root.path(), crash_at);
        let mut store = open(root.path());
        let plan = capture(&f, &mut store).resolve().unwrap();
        let wrong = plan.protocol_bytes_for_test() + 1;
        let (records, intent) = (canonical(&store), intents(&store, &f));
        let refused = commit(
            &f,
            &mut store,
            plan.with_protocol_bytes_for_test(wrong),
            &mut WriteHooks::None,
        );
        assert!(
            refused.is_err(),
            "R3 trusted the worker's accounting for the source ({crash_at:?}): {refused:?}"
        );
        assert_eq!((canonical(&store), intents(&store, &f)), (records, intent));
    }
}

/// R3 interrupted at each write it makes: its Active or Completed intents write, and the
/// Complete arm's source flush. Each leaves a state the resolver settles on reopen exactly as it
/// settles the never-staged control.
#[test]
fn flow_r_interrupted_at_each_write_reopens_and_resolves() {
    for (crash_at, _) in OUTCOMES {
        let mut interruptions =
            vec![
                WriteHooks::fail_before_write(FailError::Io("injected crash")).at(
                    if crash_at == WriteTag::Completed {
                        WriteTag::Completed
                    } else {
                        WriteTag::Active
                    },
                ),
            ];
        if crash_at == WriteTag::Completed {
            interruptions.push(
                WriteHooks::fail_before_sync(FailError::Io("injected crash")).at(WriteTag::Source),
            );
        }
        for mut hooks in interruptions {
            let f = Fixture::new(true);
            let staged = tempfile::tempdir().unwrap();
            interrupted_handoff(&f, staged.path(), crash_at);
            let control = tempfile::tempdir().unwrap();
            copy_vault(staged.path(), control.path());
            let mut synchronous = open(control.path());
            resolve_synchronously(&f, &mut synchronous).unwrap();

            let mut store = open(staged.path());
            let plan = capture(&f, &mut store).resolve().unwrap();
            let interrupted = commit(&f, &mut store, plan, &mut hooks);
            assert!(
                matches!(interrupted, Err(ref e) if e.to_string().contains("injected crash")),
                "R3 was not interrupted where the test meant: {interrupted:?}"
            );
            drop(store);
            let mut store = open(staged.path());
            resolve_synchronously(&f, &mut store).unwrap();
            assert_eq!(canonical(&store), canonical(&synchronous));
            assert_eq!(intents(&store, &f), intents(&synchronous, &f));
        }
    }
}
