//! Studio repair transactions through the real store: issuance (B1), application (B2), B3,
//! recovery-gated replacement (B4 to B6) and terminal recycling back to ordinary owner state.
use super::adoption::{adopt, checkpoint};
use super::*;
use catcoms_replication::studio::StudioRecovery;
use catcoms_replication::{ReceiptRepair, RecoveryReason, RepairDisposition};
use catcoms_rt::ManualClock;

fn request(pair: [&Receipt; 2], selected: &Receipt) -> StudioRepairRequest {
    StudioRepairRequest {
        receipt_a: pair[0].hash(),
        receipt_b: pair[1].hash(),
        selected: selected.hash(),
    }
}

fn issue_with(
    f: &Fixture,
    store: &mut ServerStore,
    request: StudioRepairRequest,
    seed: Option<&[u8]>,
    hooks: &mut WriteHooks<'_>,
) -> Result<(ReceiptRepair, StudioRepairOutcome, EpochStudioState), AppError> {
    let mut b = budget(store, f);
    store.issue_studio_repair_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        0,
        request,
        seed,
        &ManualClock::new(1000),
        &mut rng(),
        &mut b,
        hooks,
    )
}

fn issue(
    f: &Fixture,
    store: &mut ServerStore,
    request: StudioRepairRequest,
    seed: Option<&[u8]>,
) -> Result<(ReceiptRepair, StudioRepairOutcome, EpochStudioState), AppError> {
    issue_with(f, store, request, seed, &mut WriteHooks::None)
}

fn apply(
    f: &Fixture,
    store: &mut ServerStore,
    repair: &ReceiptRepair,
    pair: &[Receipt; 2],
    seed: Option<&[u8]>,
) -> Result<(StudioRepairOutcome, EpochStudioState), AppError> {
    let mut b = budget(store, f);
    store.apply_studio_repair(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        repair,
        pair,
        0,
        seed,
        &ManualClock::new(1000),
        &mut rng(),
        &mut b,
    )
}

/// A source faulted on two conflicting current-tenure receipts, as a real seal leaves it.
fn faulted(f: &Fixture, store: &mut ServerStore) -> [Receipt; 2] {
    let mut b = budget(store, f);
    let (_, state) = f.edit(store, &mut b, f.insert());
    let pair = [f.receipt(&state, 7), f.receipt(&state, 8)];
    for receipt in &pair {
        store
            .seal_studio_epoch(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                receipt.clone(),
                0,
                &mut rng(),
                &mut b,
            )
            .unwrap();
    }
    assert_eq!(f.load(store).unwrap().phase(), EpochPhase::Fault);
    let mut sorted = pair;
    sorted.sort_by_key(Receipt::hash);
    sorted
}

fn owner_is_ordinary(f: &Fixture, store: &ServerStore) -> bool {
    store.load_epoch_owner_receipts(SERVER, &f.logical).is_ok()
}

#[test]
fn terminal_repair_recycles_to_ordinary_owner_state_across_save_and_reopen() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let pair = faulted(&f, &mut store);
        let selected = &pair[1];
        let (repair, outcome, state) = issue(
            &f,
            &mut store,
            request([&pair[0], &pair[1]], selected),
            None,
        )
        .unwrap();
        assert_eq!(outcome, StudioRepairOutcome::Repaired);
        assert_eq!(state.phase(), EpochPhase::Closing);
        assert_eq!(repair.selected_receipt_hash, selected.hash());
        assert_eq!(repair.repair_sequence, 1);
        assert_eq!(f.intents(&store), 1, "a repair retires no intent");
        // Recycling omitted tag 3, so the ordinary owner paths are open again.
        assert!(owner_is_ordinary(&f, &store));
        drop(store);
        let mut store = open(root.path());
        let restored = f.load(&store).unwrap();
        assert_eq!(restored.phase(), EpochPhase::Closing);
        let resolved = restored.unit.repair_state().unwrap();
        assert_eq!(resolved.repair, repair);
        assert_eq!(resolved.disposition, RepairDisposition::Transitioned);
        assert!(owner_is_ordinary(&f, &store));
        // Ordinary operation resumes: the owner can persist the selected decision.
        let mut b = budget(&mut store, &f);
        store
            .prepare_epoch_owner_receipt(
                SERVER,
                selected.clone(),
                &f.group,
                0,
                &mut rng(),
                &mut b.storage,
            )
            .unwrap();
        // An exact retry after completion is honest and changes nothing durable.
        let (outcome, _) = apply(&f, &mut store, &repair, &pair, None).unwrap();
        assert_eq!(outcome, StudioRepairOutcome::Repaired);
        assert!(store.load_epoch_owner_receipts(SERVER, &f.logical).is_ok());
        // A completed repair leaves nothing decidable to issue against.
        assert!(issue(
            &f,
            &mut store,
            request([&pair[0], &pair[1]], selected),
            None
        )
        .is_err());
    }
}

#[test]
fn stale_unrelated_or_unauthorized_decisions_refuse_before_any_write() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new(true);
    let peer = MlsDevice::generate().unwrap();
    let welcome = f
        .group
        .add_member(&f.device, peer.key_package().unwrap())
        .unwrap()
        .welcome;
    let peer_group = ServerGroup::join(&peer, &welcome).unwrap();
    let mut store = open(root.path());
    let pair = faulted(&f, &mut store);
    let source = fs::read(f.path(&store)).unwrap();
    let mut wrong = request([&pair[0], &pair[1]], &pair[0]);
    wrong.selected = [3; 32];
    let mut unrelated = request([&pair[0], &pair[1]], &pair[0]);
    unrelated.receipt_b = [4; 32];
    for bad in [wrong, unrelated] {
        assert!(issue_with(
            &f,
            &mut store,
            bad,
            None,
            &mut WriteHooks::MustNotWrite("a stale decision wrote")
        )
        .is_err());
    }
    // A non-owner member cannot decide, even holding the same evidence.
    let mut b = budget(&mut store, &f);
    assert!(store
        .issue_studio_repair_with_io(
            SERVER,
            &peer_group,
            f.target,
            &peer,
            0,
            request([&pair[0], &pair[1]], &pair[0]),
            None,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
            &mut WriteHooks::MustNotWrite("a non-owner wrote"),
        )
        .is_err());
    // A tenure the device did not observe cannot mint a current admission.
    let mut b = budget(&mut store, &f);
    assert!(store
        .issue_studio_repair_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            1,
            request([&pair[0], &pair[1]], &pair[0]),
            None,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
            &mut WriteHooks::MustNotWrite("an unobserved tenure wrote"),
        )
        .is_err());
    assert_eq!(fs::read(f.path(&store)).unwrap(), source);
    assert!(owner_is_ordinary(&f, &store));
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
}

#[test]
fn a_b1_failure_retries_exactly_and_a_held_decision_owns_the_target_until_resumed() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let pair = faulted(&f, &mut store);
    let choose = request([&pair[0], &pair[1]], &pair[0]);
    // B1 fails before its replacement: nothing durable changed.
    assert!(issue_with(
        &f,
        &mut store,
        choose,
        None,
        &mut WriteHooks::fail_before_write(FailError::Io("b1"))
    )
    .is_err());
    assert!(owner_is_ordinary(&f, &store));
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
    // B1 succeeds, then B2 fails: the decision is durable and owns the target.
    let mut fail_source = |tag: WriteTag, _: &Path, _: &[u8]| {
        if tag == WriteTag::Source {
            Intercept::Fail(AppError::Io("b2".into()))
        } else {
            Intercept::Continue
        }
    };
    let result = issue_with(
        &f,
        &mut store,
        choose,
        None,
        &mut WriteHooks::Hooked {
            before: Some(&mut fail_source),
            before_sync: None,
            before_unlink: None,
            after: None,
        },
    );
    assert!(result.is_err());
    drop(store);
    let mut store = open(root.path());
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
    assert!(
        !owner_is_ordinary(&f, &store),
        "a persisted repair must fence ordinary owner work before B2"
    );
    let other = request([&pair[0], &pair[1]], &pair[1]);
    assert!(
        issue_with(
            &f,
            &mut store,
            other,
            None,
            &mut WriteHooks::MustNotWrite("a second decision wrote")
        )
        .is_err(),
        "a different selection must not replace the held decision"
    );
    let (repair, outcome, state) = issue(&f, &mut store, choose, None).unwrap();
    assert_eq!(outcome, StudioRepairOutcome::Repaired);
    assert_eq!(repair.selected_receipt_hash, pair[0].hash());
    assert_eq!(state.phase(), EpochPhase::Closing);
    assert!(owner_is_ordinary(&f, &store));
}

#[test]
fn an_adopting_fault_replaces_only_after_repair_recovery_and_keeps_every_intent() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let f = Fixture::new(art);
        let mut b = budget(&mut store, &f);
        f.edit(&mut store, &mut b, f.insert());
        let before = f.load(&store).unwrap().projection().unwrap();
        let (chosen, seed) = checkpoint(&f, &store, 10, 10);
        let (rival, rival_seed) = checkpoint(&f, &store, 10, 11);
        assert_eq!(
            adopt(&f, &mut store, &chosen, None).0,
            StudioAdoptionOutcome::AwaitingSeed
        );
        assert_eq!(
            adopt(&f, &mut store, &rival, None).0,
            StudioAdoptionOutcome::Fault
        );
        let mut pair = [chosen.clone(), rival.clone()];
        pair.sort_by_key(Receipt::hash);
        let (repair, outcome, state) =
            issue(&f, &mut store, request([&chosen, &rival], &chosen), None).unwrap();
        assert_eq!(outcome, StudioRepairOutcome::AwaitingSeed);
        assert_eq!(state.phase(), EpochPhase::Closing);
        assert!(state.unit.repair_install_pending());
        assert!(!owner_is_ordinary(&f, &store), "replacement still owed");
        // The rival's seed cannot complete the chosen checkpoint and replaces nothing.
        assert!(apply(&f, &mut store, &repair, &pair, Some(rival_seed.bytes())).is_err());
        assert_eq!(f.load(&store).unwrap().epoch(), 0);
        drop(store);
        let mut store = open(root.path());
        assert!(f.load(&store).unwrap().unit.repair_install_pending());
        let (outcome, state) = apply(&f, &mut store, &repair, &pair, Some(seed.bytes())).unwrap();
        assert_eq!(outcome, StudioRepairOutcome::Installed);
        assert_eq!((state.epoch(), state.phase()), (11, EpochPhase::Open));
        let held = store.load_epoch_recovery(SERVER, &f.logical).unwrap();
        let recovery = held.retained().next().expect("losing version retained");
        assert_eq!(recovery.reason, RecoveryReason::Repair);
        let recovery =
            StudioRecovery::from_snapshot(recovery, &f.logical, f.target.channel()).unwrap();
        assert_eq!(recovery.projection(), &before);
        assert_eq!(f.intents(&store), 1, "a repair retires no intent");
        assert!(owner_is_ordinary(&f, &store));
        drop(store);
        let mut store = open(root.path());
        let (outcome, _) = apply(&f, &mut store, &repair, &pair, Some(seed.bytes())).unwrap();
        assert_eq!(outcome, StudioRepairOutcome::AlreadyRepaired);
        assert_eq!(
            store
                .load_epoch_recovery(SERVER, &f.logical)
                .unwrap()
                .retained()
                .len(),
            1,
            "an exact retry stages nothing new"
        );
    }
}
