//! Studio repair transactions through the real store: issuance (B1), application (B2), B3,
//! recovery-gated replacement (B4 to B6) and terminal recycling back to ordinary owner state.
use super::adoption::{adopt, checkpoint};
use super::*;
use catcoms_replication::registry::{registry_document, PointerKey, RegistryOp};
use catcoms_replication::studio::StudioRecovery;
use catcoms_replication::{
    InheritedCheckpoint, ReceiptRepair, RecoveryReason, RepairDisposition, ReplError,
};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_wire::DocType;

type HistoricalNode = crate::Server<MemNetwork, rand_chacha::ChaCha20Rng>;

fn historical_pair(node: &mut HistoricalNode, document: &LogicalDocument) -> [Receipt; 2] {
    let tenure = node
        .sync
        .authoring_owner_tenure_start()
        .expect("the signing owner tenure was observed");
    node.sync.with_registry_context(|_, device, _, _| {
        let mut pair = [
            Receipt::sign(
                document.clone(),
                0,
                [21; 32],
                [31; 32],
                tenure,
                InheritedCheckpoint::EpochZero,
                device,
            )
            .unwrap(),
            Receipt::sign(
                document.clone(),
                0,
                [22; 32],
                [32; 32],
                tenure,
                InheritedCheckpoint::EpochZero,
                device,
            )
            .unwrap(),
        ];
        pair.sort_by_key(Receipt::hash);
        pair
    })
}

fn distinct_historical_registry_key(prefix: &str, excluded: Option<u8>) -> PointerKey {
    (0_u16..=u16::MAX)
        .find_map(|suffix| {
            let key = PointerKey::new(
                DocType::StudioObject,
                format!("{prefix}-{suffix}").into_bytes(),
            )
            .unwrap();
            (Some(key.bucket()) != excluded).then_some(key)
        })
        .expect("the Registry key space contains another bucket")
}

fn historical_storage_budget(
    store: &mut ServerStore,
    node: &mut HistoricalNode,
) -> EpochStorageBudget {
    let current = inventory(store);
    node.sync.with_registry_context(|group, _, _, _| {
        store
            .studio_storage_budget(SERVER, group, &current)
            .unwrap()
            .storage
    })
}

/// Persist a real Registry source observed by `observer` while `owner` is still live, then feed
/// it two owner-valid conflicting receipts. The resulting Fault therefore predates succession;
/// the later archived witness is not manufactured by this store fixture.
fn fault_historical_registry_source(
    store: &mut ServerStore,
    observer: &mut HistoricalNode,
    owner: &mut HistoricalNode,
    key: &PointerKey,
    budget: &mut EpochStorageBudget,
) -> [Receipt; 2] {
    let bucket = key.bucket();
    let document = registry_document(&observer.group_id(), bucket).unwrap();
    let sealed = observer
        .sync
        .with_registry_context(|group, device, _, random| {
            let mut source = catcoms_replication::registry_epoch::RegistryEpoch::new(
                group,
                bucket,
                device.device_id(),
            )
            .unwrap();
            let operation = RegistryOp::Put {
                key: key.clone(),
                epoch: 1,
            }
            .domain_op(&group.group_id(), [bucket; 16])
            .unwrap();
            source.edit(device, group, random, &operation).unwrap()
        });
    observer
        .sync
        .with_registry_context(|group, device, _, random| {
            store
                .ingest_registry_epoch(SERVER, group, bucket, device, &sealed, random, budget)
                .unwrap();
        });

    let (closed_epoch, seeds) = observer.sync.with_registry_context(|group, device, _, _| {
        let state = store
            .load_registry_epoch(SERVER, group, bucket, device)
            .unwrap()
            .expect("the observer persisted the Registry source");
        let projection = state.projection().unwrap();
        (
            state.epoch(),
            [71_u8, 72_u8].map(|close| projection.checkpoint([close; 32]).unwrap().change_hash()),
        )
    });
    let tenure = owner
        .sync
        .authoring_owner_tenure_start()
        .expect("the Registry receipt owner tenure was observed");
    let mut pair = owner.sync.with_registry_context(|_, device, _, _| {
        [71_u8, 72_u8].map(|close| {
            Receipt::sign(
                document.clone(),
                closed_epoch,
                [close; 32],
                seeds[usize::from(close - 71)],
                tenure,
                InheritedCheckpoint::EpochZero,
                device,
            )
            .unwrap()
        })
    });
    observer
        .sync
        .with_registry_context(|group, device, _, random| {
            for receipt in &pair {
                store
                    .seal_registry_epoch(
                        SERVER,
                        group,
                        bucket,
                        device,
                        receipt.clone(),
                        tenure,
                        random,
                        budget,
                    )
                    .unwrap();
            }
            assert_eq!(
                store
                    .load_registry_epoch(SERVER, group, bucket, device)
                    .unwrap()
                    .unwrap()
                    .phase(),
                EpochPhase::Fault
            );
        });
    pair.sort_by_key(Receipt::hash);
    pair
}

async fn join_historical_member(
    hub: &std::sync::Arc<Hub>,
    founder: &mut HistoricalNode,
    peer: u64,
    name: &str,
    nonce: u8,
    clock: &ManualClock,
) -> HistoricalNode {
    let invite = founder.mint_invite([nonce; 16], u64::MAX, vec![]).unwrap();
    let (joined, tick) = tokio::join!(
        HistoricalNode::join(
            hub.join(PeerId::from_u64(peer)),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(clock.clone()),
            name,
            founder.local_peer(),
            &invite,
        ),
        founder.sync_once(),
    );
    tick.unwrap();
    joined.unwrap()
}

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
fn studio_repair_sequence_exhaustion_is_typed_and_never_wraps() {
    use super::super::super::epoch_owner::next_repair_sequence;

    assert_eq!(next_repair_sequence(u64::MAX - 1, 7).unwrap(), u64::MAX);
    assert!(matches!(
        next_repair_sequence(u64::MAX, 7),
        Err(ReplError::RepairSequenceExhausted)
    ));
    assert_eq!(next_repair_sequence(3, 8).unwrap(), 9);
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
fn a_peer_applies_an_owner_repair_to_its_own_fault_and_writes_no_owner_record() {
    let root = tempfile::tempdir().unwrap();
    let mut owner = Fixture::new(false);
    let peer = MlsDevice::generate().unwrap();
    let welcome = owner
        .group
        .add_member(&owner.device, peer.key_package().unwrap())
        .unwrap()
        .welcome;
    let peer_group = ServerGroup::join(&peer, &welcome).unwrap();
    // The peer's own source, faulted on two receipts the owner signed in its tenure.
    let f = Fixture {
        device: peer,
        group: peer_group,
        target: owner.target,
        logical: owner.logical.clone(),
        id: owner.id,
    };
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    let (_, state) = f.edit(&mut store, &mut b, f.insert());
    let signed = |close: u8| {
        Receipt::sign(
            f.logical.clone(),
            state.epoch(),
            [close; 32],
            state
                .projection()
                .unwrap()
                .checkpoint([close; 32])
                .unwrap()
                .change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &owner.device,
        )
        .unwrap()
    };
    let mut pair = [signed(7), signed(8)];
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
    pair.sort_by_key(Receipt::hash);
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
    let repair = ReceiptRepair::sign_in_tenure(
        f.logical.clone(),
        pair[0].tenure_id,
        [pair[0].hash(), pair[1].hash()],
        pair[1].hash(),
        1,
        0,
        &owner.device,
    )
    .unwrap();
    // A repair signed by someone other than the current owner never applies.
    let forged = ReceiptRepair::sign_in_tenure(
        f.logical.clone(),
        pair[0].tenure_id,
        [pair[0].hash(), pair[1].hash()],
        pair[1].hash(),
        1,
        0,
        &f.device,
    )
    .unwrap();
    assert!(apply(&f, &mut store, &forged, &pair, None).is_err());
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
    let (outcome, state) = apply(&f, &mut store, &repair, &pair, None).unwrap();
    assert_eq!(outcome, StudioRepairOutcome::Repaired);
    assert_eq!(state.phase(), EpochPhase::Closing);
    assert!(
        store
            .epoch_owner_receipt_inventory_record(SERVER, &f.logical)
            .unwrap()
            .is_none(),
        "a peer keeps no owner record"
    );
    drop(store);
    let store = open(root.path());
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Closing);
}

#[test]
fn the_owner_never_applies_a_decision_it_did_not_persist_first() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let pair = faulted(&f, &mut store);
    let unpersisted = ReceiptRepair::sign_in_tenure(
        f.logical.clone(),
        pair[0].tenure_id,
        [pair[0].hash(), pair[1].hash()],
        pair[0].hash(),
        1,
        0,
        &f.device,
    )
    .unwrap();
    assert!(apply(&f, &mut store, &unpersisted, &pair, None).is_err());
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
    assert!(owner_is_ordinary(&f, &store));
}

#[test]
fn head_service_serves_an_applied_repair_but_never_proves_while_it_is_held() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(false);
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, f.insert());
    let (chosen, seed) = checkpoint(&f, &store, 10, 10);
    let (rival, _) = checkpoint(&f, &store, 10, 11);
    adopt(&f, &mut store, &chosen, None);
    adopt(&f, &mut store, &rival, None);
    let mut pair = [chosen.clone(), rival.clone()];
    pair.sort_by_key(Receipt::hash);
    let (repair, outcome, state) =
        issue(&f, &mut store, request([&chosen, &rival], &chosen), None).unwrap();
    assert_eq!(outcome, StudioRepairOutcome::AwaitingSeed);
    // Head service never cold-restores inside a request; the runtime retains each saved state.
    store.retain_studio_source(&f.group, &f.device, state);
    let mut b = budget(&mut store, &f);
    let (selection, served) = store
        .prepare_studio_head_with_fault_repair(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            Some(0),
            None,
            None,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert!(!selection.prove, "a held repair never permits a proof");
    assert_eq!(selection.receipt.as_ref(), Some(&chosen));
    assert_eq!(served.as_ref(), Some(&repair), "applied at B2, so servable");
    // Without a durable owner tenure nothing is served as the owner's repair.
    let mut b = budget(&mut store, &f);
    let (_, served) = store
        .prepare_studio_head_with_fault_repair(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            None,
            None,
            None,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert!(served.is_none());
    // Ordinary discovery cannot install anything but the held decision's own replacement.
    let mut b = budget(&mut store, &f);
    assert!(
        store
            .adopt_studio_checkpoint(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &rival,
                None,
                0,
                &ManualClock::new(1000),
                &mut rng(),
                &mut b,
            )
            .is_err(),
        "ordinary discovery must not install into a held target"
    );
    // The runtime defers by the same rule, per target, instead of letting the installer fail.
    let state = f.load(&store).unwrap();
    store.retain_studio_source(&f.group, &f.device, state);
    let defers = |store: &ServerStore, receipt: &Receipt| {
        store
            .studio_install_deferred_by_repair(SERVER, &f.group, f.target, &f.device, receipt)
            .unwrap()
    };
    assert!(
        defers(&store, &rival),
        "a different receipt waits on the held decision"
    );
    assert!(
        !defers(&store, &chosen),
        "the decision's own replacement may proceed"
    );
    // The warm source names the repair it owes and its full pair, so the runtime can fetch the
    // selected seed itself instead of waiting for an owner that may never prove it again.
    let (owed, owed_pair) = store
        .owed_studio_repair(SERVER, &f.group, f.target, &f.device)
        .expect("B2 crossed, replacement pending");
    assert_eq!(owed, repair);
    assert_eq!(owed_pair, pair);
    // A repaired seed pass installs the selected checkpoint through ordinary adoption, whose
    // install half still preserves the losing version as Repair recovery first.
    let mut b = budget(&mut store, &f);
    let (adopted, state) = store
        .adopt_studio_checkpoint(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &chosen,
            Some(seed.bytes()),
            0,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(adopted, StudioAdoptionOutcome::Installed);
    assert_eq!(
        store
            .load_epoch_recovery(SERVER, &f.logical)
            .unwrap()
            .retained()
            .next()
            .expect("losing version retained")
            .reason,
        RecoveryReason::Repair
    );
    store.retain_studio_source(&f.group, &f.device, state);
    assert!(
        store
            .owed_studio_repair(SERVER, &f.group, f.target, &f.device)
            .is_none(),
        "an installed replacement is owed by nobody"
    );
    assert!(!owner_is_ordinary(&f, &store), "the decision is still held");
    // The owner's next resume finds the replacement done and recycles its record.
    let (outcome, state) = apply(&f, &mut store, &repair, &pair, None).unwrap();
    assert_eq!(outcome, StudioRepairOutcome::AlreadyRepaired);
    assert!(owner_is_ordinary(&f, &store));
    store.retain_studio_source(&f.group, &f.device, state);
    let mut b = budget(&mut store, &f);
    let (selection, served) = store
        .prepare_studio_head_with_fault_repair(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            Some(0),
            None,
            None,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(
        served.as_ref(),
        Some(&repair),
        "still the source's disposition"
    );
    assert_eq!(selection.receipt.as_ref(), Some(&chosen));
}

#[test]
fn repair_only_head_repeats_uncertain_b2_durability_before_service() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(false);
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, f.insert());
    let (chosen, _) = checkpoint(&f, &store, 10, 10);
    let (rival, _) = checkpoint(&f, &store, 10, 11);
    adopt(&f, &mut store, &chosen, None);
    adopt(&f, &mut store, &rival, None);
    let mut pair = [chosen.clone(), rival.clone()];
    pair.sort_by_key(Receipt::hash);

    // B1 lands, then B2's source replacement becomes visible while its acknowledgement is
    // uncertain. `AfterWrite` is the store's committed/not-durable failure model: the retry must
    // treat the visible record as needing another explicit source barrier.
    let error = issue_with(
        &f,
        &mut store,
        request([&chosen, &rival], &chosen),
        None,
        &mut WriteHooks::fail_after_write(FailError::NotDurable("uncertain B2 source"))
            .at(WriteTag::Source),
    )
    .unwrap_err();
    assert!(matches!(error, AppError::CommittedButNotDurable(_)));
    drop(store);

    let mut store = open(root.path());
    let state = f.load(&store).expect("the B2 replacement is visible");
    let repair = state
        .unit
        .repair_state()
        .expect("the visible source carries the repair")
        .repair;
    store.retain_studio_source(&f.group, &f.device, state);

    let mut b = budget(&mut store, &f);
    let refused = store.prepare_studio_head_and_repair_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        Some(0),
        None,
        None,
        &mut rng(),
        &mut b,
        &mut WriteHooks::fail_before_sync(FailError::NotDurable(
            "source durability still unavailable",
        ))
        .at(WriteTag::Source),
    );
    assert!(refused.is_err(), "repair service bypassed the source flush");

    let mut b = budget(&mut store, &f);
    let refused = store.prepare_studio_head_and_repair_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        Some(0),
        None,
        None,
        &mut rng(),
        &mut b,
        &mut WriteHooks::fail_before_write(FailError::NotDurable(
            "owner repair journal durability still unavailable",
        ))
        .at(WriteTag::Journal),
    );
    assert!(
        refused.is_err(),
        "repair service bypassed the owner-record re-save"
    );

    let mut b = budget(&mut store, &f);
    let refused = store.prepare_studio_head_and_repair_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        Some(0),
        None,
        None,
        &mut rng(),
        &mut b,
        &mut WriteHooks::fail_after_write(FailError::NotDurable(
            "owner repair journal replacement is visible but uncertain",
        ))
        .at(WriteTag::Journal),
    );
    assert!(
        matches!(refused, Err(AppError::CommittedButNotDurable(_))),
        "repair service treated an uncertain B3 owner-record replacement as durable"
    );
    drop(store);

    // A fresh mount must not inherit an in-memory success from the uncertain write. Re-open the
    // exact replacement source and make both barriers succeed before repair carriage resumes.
    let mut store = open(root.path());
    let state = f
        .load(&store)
        .expect("the replacement remains readable after the uncertain B3 write");
    store.retain_studio_source(&f.group, &f.device, state);

    // Reconcile the invalidated inventory and retry. B3 is intentionally still outstanding:
    // successful B2/B3 durability, not replacement installation, is the service prerequisite.
    let mut b = budget(&mut store, &f);
    let (selection, served) = store
        .prepare_studio_head_with_fault_repair(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            Some(0),
            None,
            None,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert!(!selection.prove);
    assert_eq!(served, Some(repair));
}

fn head(
    f: &Fixture,
    store: &mut ServerStore,
    report: Option<&[Receipt; 2]>,
) -> catcoms_sync::receipt_head::ReceiptHeadSelection {
    let mut b = budget(store, f);
    store
        .prepare_studio_head_with_fault_repair(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            Some(0),
            None,
            report,
            &mut rng(),
            &mut b,
        )
        .unwrap()
        .0
}

fn sorted_pair(a: &Receipt, b: &Receipt) -> [Receipt; 2] {
    let mut pair = [a.clone(), b.clone()];
    pair.sort_by_key(Receipt::hash);
    pair
}

/// Found by design 10.3's two-peer actor run. A faulted peer reports its frozen pair on every
/// discovery until it has applied the repair, so some reports reach the owner after its own
/// decision has finished and recycled the record. Staging one again reopened a decided pair: it
/// suppressed proof of the very receipt the repair selected (no newcomer could install the
/// repaired document) and offered the pair for a second decision. Such a report is answered by
/// the repair the source carries; any other pair still stages.
#[test]
fn a_report_of_a_pair_the_owner_already_repaired_is_answered_not_restaged() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(false);
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, f.insert());
    let (chosen, seed) = checkpoint(&f, &store, 10, 10);
    let (rival, _) = checkpoint(&f, &store, 10, 11);
    let (other, _) = checkpoint(&f, &store, 10, 12);
    adopt(&f, &mut store, &chosen, None);
    adopt(&f, &mut store, &rival, None);
    let pair = sorted_pair(&chosen, &rival);
    let (repair, _, _) = issue(&f, &mut store, request([&chosen, &rival], &chosen), None).unwrap();
    let (outcome, state) = apply(&f, &mut store, &repair, &pair, Some(seed.bytes())).unwrap();
    assert_eq!(outcome, StudioRepairOutcome::Installed);
    assert!(owner_is_ordinary(&f, &store), "the decision finished");
    store.retain_studio_source(&f.group, &f.device, state);

    // The peer's late report of exactly that pair.
    let mut b = budget(&mut store, &f);
    let (_, served) = store
        .prepare_studio_head_with_fault_repair(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            Some(0),
            None,
            Some(&pair),
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(
        served.as_ref(),
        Some(&repair),
        "the answer carries the repair that resolves the report"
    );
    assert!(
        owner_is_ordinary(&f, &store),
        "a pair the owner already repaired is not staged again"
    );

    // Exact pair only: a report this source carries no repair for still stages.
    let state = f.load(&store).unwrap();
    store.retain_studio_source(&f.group, &f.device, state);
    head(&f, &mut store, Some(&sorted_pair(&chosen, &other)));
    assert!(
        !owner_is_ordinary(&f, &store),
        "a different pair is still evidence the owner must decide"
    );
}

/// N50 / CORE-005: the application consumes only the witness produced by a real, contiguous MLS
/// retirement and carried through a durably saved sync snapshot. No test constructor or receipt
/// claim supplies historical authority here.
#[tokio::test]
async fn archived_observed_tenure_admits_only_its_exact_pair_after_restart() {
    let hub = Hub::new();
    let clock = ManualClock::new(1000);
    let mut alice = HistoricalNode::found(
        hub.join(PeerId::from_u64(301)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(clock.clone()),
        "historical alice",
    )
    .unwrap();
    alice.subscribe_control().await.unwrap();
    let mut bob = join_historical_member(&hub, &mut alice, 302, "historical bob", 41, &clock).await;
    bob.subscribe_control().await.unwrap();
    let mut carol =
        join_historical_member(&hub, &mut alice, 303, "historical carol", 42, &clock).await;
    carol.subscribe_control().await.unwrap();

    // Bob must first learn Carol's admission. Otherwise Alice's removal would leave two peers
    // with different rosters and the later retirement would not be the contiguous B -> C path.
    while bob.epoch() != alice.epoch() {
        bob.sync_once().await.unwrap();
    }
    let document = target(false).document(&alice.group_id()).unwrap();
    let prepared_registry_key = distinct_historical_registry_key("historical-prepared", None);
    let explicit_registry_key = distinct_historical_registry_key(
        "historical-explicit",
        Some(prepared_registry_key.bucket()),
    );
    let prepared_registry_document = catcoms_replication::registry::registry_document(
        &alice.group_id(),
        prepared_registry_key.bucket(),
    )
    .unwrap();
    let explicit_registry_document = catcoms_replication::registry::registry_document(
        &alice.group_id(),
        explicit_registry_key.bucket(),
    )
    .unwrap();
    let alice_pair = historical_pair(&mut alice, &document);
    let alice_registry_pair = historical_pair(&mut alice, &prepared_registry_document);
    let alice_id = alice
        .sync
        .with_registry_context(|_, device, _, _| device.device_id());
    let bob_id = bob
        .sync
        .with_registry_context(|_, device, _, _| device.device_id());
    let contested = catcoms_sync::SyncConfig {
        max_committer_rank: 1,
        stage_decision_window_ms: 0,
        ..Default::default()
    };
    bob.sync.set_config(contested);
    carol.sync.set_config(contested);

    // A -> B is observed by Carol, establishing B's start. Alice was known to the joiners only
    // through Welcome, so her otherwise-valid pair must never be covered by the later archive.
    bob.sync.remove(&alice_id).await.unwrap();
    bob.sync_once().await.unwrap();
    while carol.epoch() != bob.epoch() {
        carol.sync_once().await.unwrap();
    }
    assert!(bob.is_owner());
    let bob_pair = historical_pair(&mut bob, &document);

    // Carol's physical Registry sources enter Fault while B is still the real current owner.
    // Their later service refusal therefore cannot be dismissed as a synthetic post-succession
    // state, and neither source needs to be reconstructed inside either head adapter.
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut registry_budget = historical_storage_budget(&mut store, &mut carol);
    let bob_prepared_registry_pair = fault_historical_registry_source(
        &mut store,
        &mut carol,
        &mut bob,
        &prepared_registry_key,
        &mut registry_budget,
    );
    let bob_explicit_registry_pair = fault_historical_registry_source(
        &mut store,
        &mut carol,
        &mut bob,
        &explicit_registry_key,
        &mut registry_budget,
    );
    drop(registry_budget);

    // B -> C retires the tenure Carol saw begin. Carol is now the live owner and the only
    // historical tuple its snapshot may expose is B's exact key/start/id.
    carol.sync.remove(&bob_id).await.unwrap();
    carol.sync_once().await.unwrap();
    assert!(carol.is_owner());
    carol.sync.set_config(Default::default());

    let snapshot = carol.snapshot().unwrap();
    carol.sync.with_registry_context(|_, _, _, random| {
        store.save_server(SERVER, &snapshot, random).unwrap()
    });
    drop(store);
    drop(carol);

    // Reopen both the authenticated sync snapshot and the application store before admitting.
    // This pins the durability boundary rather than trusting an in-memory transition.
    let mut store = open(root.path());
    let mut carol = HistoricalNode::restore(
        &store.load_server(SERVER).unwrap(),
        hub.join(PeerId::from_u64(304)),
        rng(),
        Box::new(clock.clone()),
        "historical carol reopened",
    )
    .unwrap();
    let mut studio_budget = {
        let current = inventory(&mut store);
        carol
            .sync
            .with_registry_context(|group, _, _, _| {
                store.studio_storage_budget(SERVER, group, &current)
            })
            .unwrap()
    };
    let operation = carol
        .sync
        .with_registry_context(|_, device, _, _| DomainOp {
            nonce: [61; 16],
            doc_type: document.doc_type,
            logical_key: document.logical_key.clone(),
            body: IndexOp::PutObject {
                object: [62; 16],
                kind: StudioKind::Flipnote,
                title: "current healthy source".into(),
                created_by: device.device_id(),
                ts: 1000,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap(),
        });
    let (_, source) = carol
        .sync
        .with_registry_context(|group, device, _, random| {
            store
                .edit_studio_epoch(
                    SERVER,
                    group,
                    target(false),
                    epoch_zero_id(document.doc_type, &document.logical_key),
                    device,
                    operation,
                    1000,
                    random,
                    &mut studio_budget,
                )
                .unwrap()
        });
    carol.sync.with_registry_context(|group, device, _, _| {
        store.retain_studio_source(group, device, source)
    });
    let permit = carol.prepare_owner_head_snapshot(&store, SERVER).unwrap();

    // Alice's fully self-signed conflict has no matching Observed witness. Exercise both
    // negatives while the historical slot is genuinely empty, then assert that B0 wrote no file;
    // a later capacity refusal must not be able to mask a broken authority check.
    let studio_owner_record = store.epoch_owner_path(
        &super::super::super::epoch_owner::scope_bytes(SERVER, &document).unwrap(),
    );
    assert!(!studio_owner_record.exists());
    let refused_without_archive = carol
        .sync
        .with_durable_owner_history(&permit.inner, |group, device, random, tenure, _| {
            store.admit_fault_report(
                SERVER,
                &document,
                group,
                device,
                tenure,
                None,
                &alice_pair,
                None,
                random,
                &mut studio_budget.storage,
            )
        })
        .unwrap()
        .unwrap();
    assert!(
        refused_without_archive.is_none(),
        "a self-signed historical pair was admitted without an archive"
    );
    let refused_wrong_archive = carol
        .sync
        .with_durable_owner_history(&permit.inner, |group, device, random, tenure, archive| {
            store.admit_fault_report(
                SERVER,
                &document,
                group,
                device,
                tenure,
                archive,
                &alice_pair,
                None,
                random,
                &mut studio_budget.storage,
            )
        })
        .unwrap()
        .unwrap();
    assert!(
        refused_wrong_archive.is_none(),
        "the archive for B authorized the Unknown founder tenure"
    );
    assert!(
        !studio_owner_record.exists(),
        "a rejected historical report created an owner record"
    );

    let admitted = carol
        .sync
        .with_durable_owner_history(&permit.inner, |group, device, random, tenure, archive| {
            let archived = archive.expect("B's observed retirement survived restart");
            assert_eq!(archived.tenure_id(), &bob_pair[0].tenure_id);
            store.admit_fault_report(
                SERVER,
                &document,
                group,
                device,
                tenure,
                Some(archived),
                &bob_pair,
                None,
                random,
                &mut studio_budget.storage,
            )
        })
        .unwrap()
        .unwrap();
    assert!(
        admitted.is_some(),
        "the exact archived B pair was not staged"
    );
    let before = carol.sync.with_registry_context(|group, device, _, _| {
        store
            .load_epoch_owner_repair_state(SERVER, &document, &device.device_id(), group.epoch())
            .unwrap()
            .0
            .retained_pairs()
            .1
            .unwrap()
            .hashes()
    });
    let retry = carol
        .sync
        .with_durable_owner_history(&permit.inner, |group, device, random, tenure, _| {
            store.admit_fault_report(
                SERVER,
                &document,
                group,
                device,
                tenure,
                None,
                &bob_pair,
                None,
                random,
                &mut studio_budget.storage,
            )
        })
        .unwrap()
        .unwrap();
    assert!(
        retry.is_some(),
        "retained exact admission depended on archive lookup"
    );
    assert_eq!(before, [bob_pair[0].hash(), bob_pair[1].hash()]);
    drop(studio_budget);

    // Exercise both production Registry adapters against real sources already frozen on B's
    // receipts. Wrong historical authority and a failed B0 must leave no owner record; a valid
    // archived report must survive the expected ReceiptConflict refusal and remain durable.
    let (prepared_stamp, prepared_source) = store
        .capture_registry_page_source(SERVER, &carol.group_id(), prepared_registry_key.bucket())
        .unwrap()
        .expect("the faulted prepared Registry source is present")
        .rebuild()
        .unwrap();
    let prepared_owner_record = store.epoch_owner_path(
        &super::super::super::epoch_owner::scope_bytes(SERVER, &prepared_registry_document)
            .unwrap(),
    );
    let explicit_owner_record = store.epoch_owner_path(
        &super::super::super::epoch_owner::scope_bytes(SERVER, &explicit_registry_document)
            .unwrap(),
    );
    assert!(!prepared_owner_record.exists());
    assert!(!explicit_owner_record.exists());

    let mut registry_budget = historical_storage_budget(&mut store, &mut carol);
    let wrong_archive = carol
        .sync
        .with_durable_owner_history(&permit.inner, |group, device, random, tenure, archive| {
            store.prepare_registry_head_prepared_with_fault_repair(
                SERVER,
                group,
                prepared_registry_key.bucket(),
                device,
                Some(tenure),
                archive,
                Some(&alice_registry_pair),
                random,
                Some((&prepared_stamp, &prepared_source)),
                &mut registry_budget,
            )
        })
        .unwrap()
        .unwrap_err();
    assert!(
        wrong_archive
            .to_string()
            .contains("conflicting epoch-close receipt"),
        "wrong archive did not reach the ordinary Fault refusal: {wrong_archive}"
    );
    assert!(
        !prepared_owner_record.exists(),
        "a wrong archived Registry authority wrote B0"
    );
    drop(registry_budget);

    let mut registry_budget = historical_storage_budget(&mut store, &mut carol);
    let failed_b0 = carol
        .sync
        .with_durable_owner_history(&permit.inner, |group, device, random, tenure, archive| {
            store.prepare_registry_head_and_repair(
                SERVER,
                group,
                explicit_registry_key.bucket(),
                device,
                Some(tenure),
                archive,
                Some(&bob_explicit_registry_pair),
                random,
                &mut registry_budget,
                &mut WriteHooks::fail_before_write(FailError::Io("injected historical B0 failure"))
                    .at(WriteTag::Journal),
            )
        })
        .unwrap()
        .unwrap_err();
    assert!(
        failed_b0
            .to_string()
            .contains("injected historical B0 failure"),
        "the later Fault refusal masked the B0 failure: {failed_b0}"
    );
    assert!(
        !explicit_owner_record.exists(),
        "a failed B0 left a claimed durable owner record"
    );
    drop(registry_budget);

    let mut registry_budget = historical_storage_budget(&mut store, &mut carol);
    let prepared_full_loads = crate::store::registry_full_loads_for_test();
    let prepared_fault = carol
        .sync
        .with_durable_owner_history(&permit.inner, |group, device, random, tenure, archive| {
            store.prepare_registry_head_prepared_with_fault_repair(
                SERVER,
                group,
                prepared_registry_key.bucket(),
                device,
                Some(tenure),
                archive,
                Some(&bob_prepared_registry_pair),
                random,
                Some((&prepared_stamp, &prepared_source)),
                &mut registry_budget,
            )
        })
        .unwrap()
        .unwrap_err();
    assert!(
        prepared_fault
            .to_string()
            .contains("conflicting epoch-close receipt"),
        "the prepared Fault source did not refuse head service: {prepared_fault}"
    );
    assert_eq!(
        crate::store::registry_full_loads_for_test(),
        prepared_full_loads,
        "archived B0 admission reconstructed a prepared Registry source"
    );
    assert!(
        prepared_owner_record.exists(),
        "prepared service skipped B0"
    );
    drop(registry_budget);

    let mut registry_budget = historical_storage_budget(&mut store, &mut carol);
    let explicit_fault = carol
        .sync
        .with_durable_owner_history(&permit.inner, |group, device, random, tenure, archive| {
            store.prepare_registry_head_with_fault_repair(
                SERVER,
                group,
                explicit_registry_key.bucket(),
                device,
                Some(tenure),
                archive,
                Some(&bob_explicit_registry_pair),
                random,
                &mut registry_budget,
            )
        })
        .unwrap()
        .unwrap_err();
    assert!(
        explicit_fault
            .to_string()
            .contains("conflicting epoch-close receipt"),
        "the explicit Fault source did not refuse head service: {explicit_fault}"
    );
    assert!(
        explicit_owner_record.exists(),
        "explicit service skipped B0"
    );
    drop(registry_budget);

    // An exact retry can use the retained attestation after archive lookup disappears, but it
    // still cannot turn a faulted Registry source into a hint or proof.
    let mut registry_budget = historical_storage_budget(&mut store, &mut carol);
    let retained_retry = carol
        .sync
        .with_durable_owner_history(&permit.inner, |group, device, random, tenure, _| {
            store.prepare_registry_head_with_fault_repair(
                SERVER,
                group,
                explicit_registry_key.bucket(),
                device,
                Some(tenure),
                None,
                Some(&bob_explicit_registry_pair),
                random,
                &mut registry_budget,
            )
        })
        .unwrap()
        .unwrap_err();
    assert!(
        retained_retry
            .to_string()
            .contains("conflicting epoch-close receipt"),
        "retained admission incorrectly made the Fault servable: {retained_retry}"
    );
    drop(registry_budget);

    // The live owner can now make an explicit signed decision for the retained historical pair.
    // Its healthy current source is screened rather than faulted or rewritten as if B were live.
    let mut studio_budget = {
        let current = inventory(&mut store);
        carol
            .sync
            .with_registry_context(|group, _, _, _| {
                store.studio_storage_budget(SERVER, group, &current)
            })
            .unwrap()
    };
    let request = StudioRepairRequest {
        receipt_a: bob_pair[0].hash(),
        receipt_b: bob_pair[1].hash(),
        selected: bob_pair[0].hash(),
    };
    let (repair, outcome, _) = carol
        .sync
        .with_durable_owner_snapshot(&permit.inner, |group, device, random, tenure| {
            store.issue_studio_repair(
                SERVER,
                group,
                target(false),
                device,
                tenure,
                request,
                None,
                &clock,
                random,
                &mut studio_budget,
            )
        })
        .unwrap()
        .unwrap();
    assert_eq!(outcome, StudioRepairOutcome::Screened);
    assert_eq!(repair.tenure_id, bob_pair[0].tenure_id);

    drop(studio_budget);
    drop(permit);
    drop(prepared_source);
    drop(prepared_stamp);
    drop(store);
    let store = open(root.path());
    carol.sync.with_registry_context(|group, device, _, _| {
        for (document, pair, bucket) in [
            (
                &prepared_registry_document,
                &bob_prepared_registry_pair,
                prepared_registry_key.bucket(),
            ),
            (
                &explicit_registry_document,
                &bob_explicit_registry_pair,
                explicit_registry_key.bucket(),
            ),
        ] {
            let retained = store
                .load_epoch_owner_repair_state(SERVER, document, &device.device_id(), group.epoch())
                .unwrap()
                .0
                .retained_pairs()
                .1
                .expect("the service path persisted its archived pair")
                .hashes();
            assert_eq!(retained, [pair[0].hash(), pair[1].hash()]);
            assert_eq!(
                store
                    .load_registry_epoch(SERVER, group, bucket, device)
                    .unwrap()
                    .expect("the faulted Registry source survived reopen")
                    .phase(),
                EpochPhase::Fault,
                "B0 admission must not repair or relabel the source"
            );
        }
    });
}

#[test]
fn a_current_tenure_report_stages_suppresses_proof_and_is_decided_from_the_reserved_slot() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    let (_, state) = f.edit(&mut store, &mut b, f.insert());
    let [r1, r2, r3] = [7, 8, 9].map(|close| f.receipt(&state, close));
    let (_, sealed) = store
        .seal_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            r1.clone(),
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    store
        .prepare_epoch_owner_receipt(SERVER, r1.clone(), &f.group, 0, &mut rng(), &mut b.storage)
        .unwrap();
    store.retain_studio_source(&f.group, &f.device, sealed);
    assert!(
        head(&f, &mut store, None).prove,
        "baseline: the owner proves its head"
    );

    // A pair the owner cannot attest, signed outside its tenure, writes nothing even while the
    // reserved slot is empty. Keeping this before the valid report is load-bearing mutation
    // coverage: the historical-capacity fence below must not mask removal of current admission.
    let owner_record = store.epoch_owner_path(
        &super::super::super::epoch_owner::scope_bytes(SERVER, &f.logical).unwrap(),
    );
    let before = fs::read(&owner_record).unwrap();
    let outsider = MlsDevice::generate().unwrap();
    let foreign = |close: u8| {
        Receipt::sign(
            f.logical.clone(),
            0,
            [close; 32],
            [close; 32],
            0,
            InheritedCheckpoint::EpochZero,
            &outsider,
        )
        .unwrap()
    };
    head(&f, &mut store, Some(&sorted_pair(&foreign(1), &foreign(2))));
    assert_eq!(
        fs::read(&owner_record).unwrap(),
        before,
        "only a current-tenure pair can be staged as live"
    );

    // A peer reports a current-tenure equivocation it is frozen on.
    let first = sorted_pair(&r1, &r2);
    let gated = head(&f, &mut store, Some(&first));
    assert!(
        !gated.prove,
        "a staged live pair suppresses proof in the same answer"
    );
    assert!(
        gated.receipt.is_none(),
        "a disputed receipt is not offered as a hint"
    );
    drop(store);
    let mut store = open(root.path());
    let restored = f.load(&store).unwrap();
    store.retain_studio_source(&f.group, &f.device, restored);
    assert!(
        !head(&f, &mut store, None).prove,
        "suppression is durable, not a property of the reporting exchange"
    );

    // A second pair while the reserved slot is occupied: only its fingerprint is kept.
    let second = sorted_pair(&r1, &r3);
    assert!(!head(&f, &mut store, Some(&second)).prove);

    // Decide the reserved pair. The source is healthy, so this screens; the overflow still
    // records a conflict this owner authenticated, so proof stays suppressed.
    let (repair, outcome, state) =
        issue(&f, &mut store, request([&first[0], &first[1]], &r1), None).unwrap();
    assert_eq!(outcome, StudioRepairOutcome::Screened);
    assert_eq!(repair.repair_sequence, 1);
    store.retain_studio_source(&f.group, &f.device, state);
    assert!(
        !head(&f, &mut store, None).prove,
        "live overflow still suppresses"
    );

    // The reporter retries; the freed slot takes the pair and its fingerprint is released.
    let overflow = |store: &ServerStore| {
        store
            .load_epoch_owner_repair_state(SERVER, &f.logical, &f.device.device_id(), 0)
            .unwrap()
            .0
            .fault_overflow_fingerprints()
    };
    assert_eq!(
        overflow(&store),
        Some(1),
        "only the second pair's fingerprint is held"
    );
    assert!(!head(&f, &mut store, Some(&second)).prove);
    assert_eq!(
        overflow(&store),
        None,
        "a stored pair releases its fingerprint and an empty hold canonicalises away"
    );
    let (repair, outcome, state) =
        issue(&f, &mut store, request([&second[0], &second[1]], &r1), None).unwrap();
    assert_eq!(outcome, StudioRepairOutcome::Screened);
    assert_eq!(repair.repair_sequence, 2);
    store.retain_studio_source(&f.group, &f.device, state);
    assert!(
        owner_is_ordinary(&f, &store),
        "nothing retained: tag 3 omitted"
    );
    assert!(head(&f, &mut store, None).prove, "ordinary proofs resume");
}

/// The rollback case: the owner's own head is the loser of a reported current-tenure pair, so
/// the repair retargets its healthy source (case 6c) and replaces it after Repair recovery.
#[test]
fn a_rolled_back_owner_is_retargeted_with_no_pre_b2_hint_rotation_or_publication() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    let (_, state) = f.edit(&mut store, &mut b, f.insert());
    let projection = state.projection().unwrap();
    let [lost, kept] = [7, 8].map(|close| f.receipt(&state, close));
    let kept_seed = projection.checkpoint([8; 32]).unwrap();
    let (_, sealed) = store
        .seal_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            lost.clone(),
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    store
        .prepare_epoch_owner_receipt(
            SERVER,
            lost.clone(),
            &f.group,
            0,
            &mut rng(),
            &mut b.storage,
        )
        .unwrap();
    store
        .mark_epoch_owner_receipt_published(
            SERVER,
            &f.logical,
            lost.hash(),
            &mut rng(),
            &mut b.storage,
        )
        .unwrap();
    store.retain_studio_source(&f.group, &f.device, sealed);
    let pair = sorted_pair(&lost, &kept);
    assert!(!head(&f, &mut store, Some(&pair)).prove);

    // B1 succeeds, B2 fails: the decision is held and the source still sits on the loser.
    let mut fail_source = |tag: WriteTag, _: &Path, _: &[u8]| {
        if tag == WriteTag::Source {
            Intercept::Fail(AppError::Io("b2".into()))
        } else {
            Intercept::Continue
        }
    };
    assert!(issue_with(
        &f,
        &mut store,
        request([&pair[0], &pair[1]], &kept),
        None,
        &mut WriteHooks::Hooked {
            before: Some(&mut fail_source),
            before_sync: None,
            before_unlink: None,
            after: None,
        },
    )
    .is_err());
    let restored = f.load(&store).unwrap();
    store.retain_studio_source(&f.group, &f.device, restored);
    let held = head(&f, &mut store, None);
    assert!(!held.prove);
    assert!(
        held.receipt.is_none(),
        "before B2 the source still holds the repudiated receipt: no hint at all"
    );
    let mut b = budget(&mut store, &f);
    assert!(
        !store
            .studio_owner_rotation_needed(SERVER, &f.group, f.target, &f.device, &mut b)
            .unwrap(),
        "a held decision is not rotated around"
    );
    let mut b = budget(&mut store, &f);
    assert!(
        store
            .complete_studio_head(SERVER, &kept, &mut rng(), &mut b)
            .is_err(),
        "publication refuses while a repair is held"
    );

    // Resume: B2 retargets onto the kept receipt, then the seed replaces the losing version.
    let (repair, outcome, state) =
        issue(&f, &mut store, request([&pair[0], &pair[1]], &kept), None).unwrap();
    assert_eq!(outcome, StudioRepairOutcome::AwaitingSeed);
    assert_eq!(
        state.unit.repair_state().unwrap().disposition,
        RepairDisposition::Retargeted
    );
    let (outcome, state) = apply(&f, &mut store, &repair, &pair, Some(kept_seed.bytes())).unwrap();
    assert_eq!(outcome, StudioRepairOutcome::Installed);
    assert_eq!(state.phase(), EpochPhase::Open);
    let recovery = store.load_epoch_recovery(SERVER, &f.logical).unwrap();
    let held = recovery.retained().next().expect("losing version retained");
    assert_eq!(held.reason, RecoveryReason::Repair);
    assert_eq!(f.intents(&store), 1, "a repair retires no intent");
}

/// Section 9: replacement binds the DURABLE predecessor. If the source on disk changes after
/// its Repair recovery was staged, the successor is refused rather than replacing a version
/// that recovery never preserved.
#[test]
fn a_successor_is_refused_when_the_durable_predecessor_changed_after_recovery() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(false);
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, f.insert());
    let (chosen, seed) = checkpoint(&f, &store, 10, 10);
    let (rival, _) = checkpoint(&f, &store, 10, 11);
    adopt(&f, &mut store, &chosen, None);
    adopt(&f, &mut store, &rival, None);
    let mut pair = [chosen.clone(), rival.clone()];
    pair.sort_by_key(Receipt::hash);
    let source = f.path(&store);
    let faulted_bytes = fs::read(&source).unwrap();
    let (repair, outcome, _) =
        issue(&f, &mut store, request([&chosen, &rival], &chosen), None).unwrap();
    assert_eq!(outcome, StudioRepairOutcome::AwaitingSeed);
    // Between the recovery stage returning and the successor write, put an older valid sealed
    // version back on disk.
    let mut swap = |op: CompletedOperation, tag: WriteTag, _: &Path| {
        if tag == WriteTag::Recovery && op == CompletedOperation::Write {
            fs::write(&source, &faulted_bytes).unwrap();
        }
        AfterIntercept::Continue
    };
    let mut b = budget(&mut store, &f);
    let refused = store
        .apply_studio_repair_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &repair,
            &pair,
            0,
            Some(seed.bytes()),
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
            &mut WriteHooks::Hooked {
                before: None,
                before_sync: None,
                before_unlink: None,
                after: Some(&mut swap),
            },
        )
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("recovery capability"),
        "the successor must not replace an unpreserved version; got: {refused}"
    );
    assert_eq!(
        fs::read(&source).unwrap(),
        faulted_bytes,
        "nothing was replaced"
    );
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

/// Design 10.3's cost obligation: how long a repair job holds actor/store custody at a large
/// source, with the detached S2 reported apart. Each stage runs through the same store call the
/// job makes (`capture_studio_source`, `rebuild`, `install_prepared_studio_source`, then the
/// unchanged transaction), so no second algorithm is timed. The cold custody restore printed
/// beside it is what the synchronous repair paid inside custody before the job existed.
///
/// Millisecond `Clock` resolution, one sample per stage: these are orders of magnitude, not
/// benchmarks, and nothing here asserts a speed.
fn repair_job_stages(count: usize, clock: &dyn catcoms_rt::Clock) {
    let timed = |run: &mut dyn FnMut()| {
        let start = clock.monotonic_ms();
        run();
        clock.monotonic_ms().saturating_sub(start)
    };
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    // A Flipnote: the dense builder's operations are Flipnote edits, as `measure` uses them.
    let f = Fixture::new(true);
    super::performance::save_studio_source_fixture_ops(
        &mut store, SERVER, &f.group, &f.device, f.target, count, 0,
    );
    let (chosen, seed) = checkpoint(&f, &store, 10, 10);
    let (rival, _) = checkpoint(&f, &store, 10, 11);
    adopt(&f, &mut store, &chosen, None);
    adopt(&f, &mut store, &rival, None);
    let pair = sorted_pair(&chosen, &rival);
    let bytes = fs::metadata(f.path(&store)).unwrap().len();
    let ops = f.load(&store).unwrap().op_count();
    let cold = timed(&mut || drop(f.load(&store).unwrap()));

    // One whole job: S1 under custody, S2 detached, S3 under custody.
    let job = |store: &mut ServerStore, s3: &mut dyn FnMut(&mut ServerStore)| {
        let mut capture = None;
        let s1 = timed(&mut || {
            capture = store
                .capture_studio_source(SERVER, &f.group, f.target, &f.device)
                .unwrap();
        });
        let mut prepared = None;
        let s2 = timed(&mut || prepared = Some(capture.take().unwrap().rebuild().unwrap()));
        let mut installed = false;
        let install = timed(&mut || {
            installed = store
                .install_prepared_studio_source(&f.group, &f.device, prepared.take().unwrap())
                .unwrap();
        });
        assert!(installed, "the rebuild is current");
        let transaction = timed(&mut || s3(store));
        (s1, s2, install, transaction)
    };
    let mut repair = None;
    let (s1, s2, install, decide) = job(&mut store, &mut |store| {
        let (issued, outcome, _) =
            issue(&f, store, request([&chosen, &rival], &chosen), None).unwrap();
        assert_eq!(outcome, StudioRepairOutcome::AwaitingSeed);
        repair = Some(issued);
    });
    println!(
        "REPAIR_PROFILE ops={ops} physical_bytes={bytes} cold_restore_ms={cold} \
         decide: s1_capture_ms={s1} s2_rebuild_detached_ms={s2} s3_install_ms={install} \
         s3_transaction_ms={decide} custody_ms={}",
        s1 + install + decide
    );
    let repair = repair.unwrap();
    let (s1, s2, install, replace) = job(&mut store, &mut |store| {
        let (outcome, _) = apply(&f, store, &repair, &pair, Some(seed.bytes())).unwrap();
        assert_eq!(outcome, StudioRepairOutcome::Installed);
    });
    println!(
        "REPAIR_PROFILE ops={ops} replace: s1_capture_ms={s1} s2_rebuild_detached_ms={s2} \
         s3_install_ms={install} s3_transaction_ms={replace} custody_ms={}",
        s1 + install + replace
    );
}

#[test]
fn repair_job_stage_profile_smoke() {
    repair_job_stages(64, &ManualClock::new(1_000));
}

#[test]
#[ignore = "opt-in release profiling of repair job custody; no machine-speed assertion"]
fn profile_repair_job_stages() {
    repair_job_stages(20_000, &catcoms_rt::SystemClock);
}
