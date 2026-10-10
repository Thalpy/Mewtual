//! Design 10.3's end-to-end evidence: a real Fault, the owner's decision, a peer's replacement,
//! a restart and a newcomer, all through spawned actors and their native-facing calls. Nothing
//! here calls a repair transaction or a runtime method directly: every step is a receive turn, a
//! Read or a control request, exactly as the native coordinator drives them.
//!
//! A peer may apply an offered repair only under a tenure it observed begin (N16), so ownership
//! must change after the peer joined. Alice founds, Bob and Carol join, and Bob removes Alice
//! while Carol watches: Bob is then the owner and Carol has observed his tenure start.
//!
//! The fault is the realistic fork. Bob installed receipt R1 with its seed (so he can serve it)
//! and then saw a rival R2 from his own tenure for the same epoch; Carol saw both and has
//! neither seed. Bob decides R1. That is terminal for him, but Carol must apply the repair from
//! his answer, fetch R1's seed from him and install it as a replacement.
//!
//! The newcomer is a document newcomer: Dave, admitted before the succession, whose vault has
//! never held this document and first reads it after the repair and the restart. A fresh MLS
//! member cannot be used here. After A -> B leaf 0 is vacant, so a pinned group refuses any
//! admission (`AdmissionAuthorityUnavailable`, until an authenticated succession proof exists),
//! and a policy-less group would put the joiner in leaf 0 and make it the owner. An MLS
//! newcomer's refusal of an offered repair (N16) is pinned at the runtime level instead, by
//! `a_newcomer_refuses_an_offered_repair_before_reserving_anything`.
use super::*;
use crate::store::StudioAdoptionOutcome;
use crate::studio::{
    StudioControlAction, StudioControlRequest, StudioControlResponse, StudioRequest,
    StudioVaultLease, StudioView,
};
use tokio::sync::Mutex;

type Node = Server<MemNetwork, ChaCha20Rng>;
type Store = Arc<Mutex<Option<ServerStore>>>;

async fn join(
    hub: &Arc<Hub>,
    sponsor: &mut Node,
    peer: u64,
    name: &str,
    nonce: u8,
    clock: &ManualClock,
) -> Node {
    let invite = sponsor.mint_invite([nonce; 16], u64::MAX, vec![]).unwrap();
    let (joined, tick) = tokio::join!(
        Node::join(
            hub.join(PeerId::from_u64(peer)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(peer + 100),
            Box::new(clock.clone()),
            name,
            sponsor.local_peer(),
            &invite,
        ),
        sponsor.sync_once(),
    );
    tick.unwrap();
    joined.unwrap()
}

/// Prove `server` to `serving` as a Studio page peer, through the same authenticated directory
/// catch-up the desktop uses. Endpoint proof is never restored from a snapshot.
async fn prove(server: &mut Node, serving: &mut Node) {
    serving.open_channel_index().await.unwrap();
    let peer = serving.local_peer();
    tokio::select! {
        result = server.request_channel_index_catchup(peer) => { result.unwrap(); }
        _ = async { loop { serving.sync_once().await.unwrap(); } } => unreachable!(),
    }
}

fn lease(store: &Store) -> StudioVaultLease {
    StudioVaultLease::new(store.clone().try_lock_owned().unwrap(), SERVER, ())
}

async fn read(
    actor: &crate::ServerActor,
    store: &Store,
    target: StudioTarget,
) -> Result<Option<StudioView>, String> {
    actor
        .studio_begin(StudioRequest::Read { target })
        .await?
        .execute(lease(store))
        .await
}

async fn control(
    actor: &crate::ServerActor,
    store: &Store,
    target: StudioTarget,
    action: StudioControlAction,
) -> Result<StudioControlResponse, String> {
    actor
        .studio_control_begin(StudioControlRequest { target, action })
        .await?
        .execute(lease(store))
        .await
}

async fn turn(actor: &crate::ServerActor, store: &Store) {
    actor
        .studio_receive_begin()
        .await
        .unwrap()
        .execute(lease(store))
        .await
        .unwrap();
}

/// One synthetic second for every actor at once, so none waits on another's response while
/// holding its Server. Detached CPU work (preparation, a repair rebuild) is then awaited
/// without spending network deadlines on it.
async fn tick(clock: &ManualClock, actors: &[(&crate::ServerActor, &Store)]) {
    assert!(
        actors.len() <= 3,
        "this fixture drives at most three actors"
    );
    clock.advance_ms(1_000);
    let turn_at = |i: usize| async move {
        if let Some((actor, store)) = actors.get(i) {
            turn(actor, store).await;
        }
    };
    tokio::join!(turn_at(0), turn_at(1), turn_at(2));
    let wait_at = |i: usize| async move {
        if let Some((actor, _)) = actors.get(i) {
            actor.wait_studio_preparation().await;
        }
    };
    tokio::join!(wait_at(0), wait_at(1), wait_at(2));
}

async fn fault_view(
    actor: &crate::ServerActor,
    store: &Store,
    target: StudioTarget,
) -> crate::studio::StudioFaultView {
    match control(actor, store, target, StudioControlAction::ReadFault).await {
        Ok(StudioControlResponse::Fault(view)) => *view,
        other => panic!("a fault view: {other:?}"),
    }
}

/// Spawn an actor whose event stream must never report a paused receiver. Its preparation pools
/// are private, as the process it models would have them: these actors retain Registry graphs
/// (and their slots) for up to 30 s, which must not starve other tests sharing the process pool.
fn spawn(
    node: Node,
) -> (
    crate::ServerActor,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    crate::actor::studio_pools_for_next_spawn(
        Arc::new(tokio::sync::Semaphore::new(4)),
        Arc::new(tokio::sync::Semaphore::new(3)),
    );
    let (actor, mut events, task) = crate::spawn(node);
    let drain = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            assert!(
                !matches!(event.event, crate::AppEvent::StudioReceivePaused),
                "a repair must never pause catch-up"
            );
        }
    });
    (actor, task, drain)
}

async fn stop(
    actor: crate::ServerActor,
    task: tokio::task::JoinHandle<()>,
    drain: tokio::task::JoinHandle<()>,
) {
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

/// Bob's edit, and the two receipts his tenure signs closing epoch 0 over it: R1 (`chosen`, with
/// its seed) and the rival R2. The owner journal accepts only an adjacent close, so this is the
/// epoch his source actually sits on.
fn fork(bob: &mut Node, store: &mut ServerStore, target: StudioTarget) -> ([Receipt; 2], Vec<u8>) {
    let StudioOwnerTenure::Known(start) = bob.observed_owner_tenure() else {
        panic!("the owner observes its own tenure")
    };
    let mut b = CatchupRuntime::budget(bob, store, SERVER).unwrap();
    bob.sync.with_registry_context(|group, device, _, rng| {
        let logical = target.document(&group.group_id()).unwrap();
        let op = DomainOp {
            nonce: [1; 16],
            doc_type: logical.doc_type,
            logical_key: logical.logical_key.clone(),
            body: IndexOp::PutObject {
                object: [1; 16],
                kind: StudioKind::Flipnote,
                title: "moon".into(),
                created_by: device.device_id(),
                ts: 100,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap(),
        };
        let id = catcoms_replication::epoch_zero_id(logical.doc_type, &logical.logical_key);
        let (_, state) = store
            .edit_studio_epoch(SERVER, group, target, id, device, op, 100, rng, &mut b)
            .unwrap();
        let signed = [10u8, 11].map(|salt| {
            let mut projection = state.projection().unwrap();
            let StudioProjection::Index(index) = &mut projection else {
                panic!("an Index target")
            };
            index.epoch = 0;
            let seed = projection.checkpoint([salt; 32]).unwrap();
            let receipt = Receipt::sign(
                logical.clone(),
                0,
                [salt; 32],
                seed.change_hash(),
                start,
                InheritedCheckpoint::EpochZero,
                device,
            )
            .unwrap();
            (receipt, seed.bytes().to_vec())
        });
        let [(chosen, seed), (rival, _)] = signed;
        ([chosen, rival], seed)
    })
}

fn adopt(
    node: &mut Node,
    store: &mut ServerStore,
    target: StudioTarget,
    receipt: &Receipt,
    seed: Option<&[u8]>,
) -> StudioAdoptionOutcome {
    let tenure = node
        .sync
        .authoring_owner_tenure_start()
        .expect("an observed owner tenure");
    let mut b = CatchupRuntime::budget(node, store, SERVER).unwrap();
    node.sync
        .with_registry_context(|group, device, clock, rng| {
            store
                .adopt_studio_checkpoint(
                    SERVER, group, target, device, receipt, seed, tenure, clock, rng, &mut b,
                )
                .unwrap()
                .0
        })
}

/// The owner's own publication of a receipt it installed, as an ordinary close records it. Only
/// a receipt in the owner's journal is ever proved to a newcomer.
fn publish(node: &mut Node, store: &mut ServerStore, receipt: &Receipt) {
    let tenure = node
        .sync
        .authoring_owner_tenure_start()
        .expect("the owner observes its own tenure");
    let mut b = CatchupRuntime::budget(node, store, SERVER).unwrap();
    node.sync.with_registry_context(|group, _, _, rng| {
        store
            .with_studio_protocol_budget(SERVER, group, &mut b, |store, storage| {
                store.prepare_epoch_owner_receipt(
                    SERVER,
                    receipt.clone(),
                    group,
                    tenure,
                    rng,
                    storage,
                )
            })
            .unwrap();
    });
}

fn store_at(root: &tempfile::TempDir, seed: u64) -> ServerStore {
    ServerStore::open(
        root.path(),
        b"two-peer-repair",
        &mut ChaCha20Rng::seed_from_u64(seed),
    )
    .unwrap()
}

/// The cast every real-peer repair test here needs. Alice founds; Bob, Carol and Dave join; Bob
/// removes Alice while Carol and Dave watch. Bob is then the owner, and Carol has observed his
/// tenure begin, so she may apply his repairs (N16). Bob and Carol are proven to each other as
/// Studio page peers. Dave is proven to nobody yet.
struct Cast {
    hub: Arc<Hub>,
    clock: ManualClock,
    bob: Node,
    carol: Node,
    dave: Node,
}

async fn cast() -> Cast {
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let mut alice = Node::found(
        hub.join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(101),
        Box::new(clock.clone()),
        "alice",
    )
    .unwrap();
    alice.subscribe_control().await.unwrap();
    let mut bob = join(&hub, &mut alice, 2, "bob", 61, &clock).await;
    bob.subscribe_control().await.unwrap();
    let mut carol = join(&hub, &mut alice, 3, "carol", 62, &clock).await;
    carol.subscribe_control().await.unwrap();
    let mut dave = join(&hub, &mut alice, 4, "dave", 63, &clock).await;
    dave.subscribe_control().await.unwrap();
    while bob.epoch() != alice.epoch() {
        bob.sync_once().await.unwrap();
    }
    while carol.epoch() != alice.epoch() {
        carol.sync_once().await.unwrap();
    }

    // A -> B, observed by Carol: Bob's tenure starts while Carol is a member.
    let alice_id = alice
        .sync
        .with_registry_context(|_, device, _, _| device.device_id());
    let contested = catcoms_sync::SyncConfig {
        max_committer_rank: 1,
        stage_decision_window_ms: 0,
        ..Default::default()
    };
    bob.sync.set_config(contested);
    carol.sync.set_config(contested);
    dave.sync.set_config(contested);
    bob.sync.remove(&alice_id).await.unwrap();
    bob.sync_once().await.unwrap();
    while carol.epoch() != bob.epoch() {
        carol.sync_once().await.unwrap();
    }
    while dave.epoch() != bob.epoch() {
        dave.sync_once().await.unwrap();
    }
    drop(alice);
    bob.sync.set_config(Default::default());
    carol.sync.set_config(Default::default());
    dave.sync.set_config(Default::default());
    assert!(bob.is_owner());
    assert!(
        matches!(carol.observed_owner_tenure(), StudioOwnerTenure::Known(_)),
        "Carol watched Bob's tenure begin, so she may apply his repairs"
    );
    prove(&mut bob, &mut carol).await;
    prove(&mut carol, &mut bob).await;
    Cast {
        hub,
        clock,
        bob,
        carol,
        dave,
    }
}

fn index_target() -> StudioTarget {
    StudioTarget::Index {
        channel: crate::channel_id("general").to_be_bytes(),
    }
}

#[tokio::test]
async fn a_fault_is_decided_replaced_and_survives_restart_and_a_newcomer_through_the_actors() {
    let Cast {
        hub,
        clock,
        mut bob,
        mut carol,
        mut dave,
    } = cast().await;

    // The fork: Bob installed R1 and then met R2; Carol holds both receipts and no seed.
    let target = index_target();
    let (bob_root, carol_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut bob_store = store_at(&bob_root, 201);
    let mut carol_store = store_at(&carol_root, 202);
    let ([chosen, rival], chosen_seed) = fork(&mut bob, &mut bob_store, target);
    assert_eq!(
        adopt(
            &mut bob,
            &mut bob_store,
            target,
            &chosen,
            Some(&chosen_seed)
        ),
        StudioAdoptionOutcome::Installed
    );
    publish(&mut bob, &mut bob_store, &chosen);
    assert_eq!(
        adopt(&mut bob, &mut bob_store, target, &rival, None),
        StudioAdoptionOutcome::Fault
    );
    assert_eq!(
        adopt(&mut carol, &mut carol_store, target, &chosen, None),
        StudioAdoptionOutcome::AwaitingSeed
    );
    assert_eq!(
        adopt(&mut carol, &mut carol_store, target, &rival, None),
        StudioAdoptionOutcome::Fault
    );
    let mut pair = [chosen.clone(), rival.clone()];
    pair.sort_by_key(Receipt::hash);
    let group_id = bob.group_id();

    // Everything from here is the actors.
    let (bob_snapshot, carol_snapshot) = (bob.snapshot().unwrap(), carol.snapshot().unwrap());
    let bob_store: Store = Arc::new(Mutex::new(Some(bob_store)));
    let carol_store: Store = Arc::new(Mutex::new(Some(carol_store)));
    let (bob_actor, bob_task, bob_drain) = spawn(bob);
    let (carol_actor, carol_task, carol_drain) = spawn(carol);
    let actors = [(&bob_actor, &bob_store), (&carol_actor, &carol_store)];
    // Both watch the document; Bob's first turn saves his durable owner snapshot.
    let _ = read(&bob_actor, &bob_store, target).await;
    let _ = read(&carol_actor, &carol_store, target).await;
    tick(&clock, &actors).await;

    // The visit model: `Busy` reserved nothing and asks the caller to try again. Here that is
    // ordinary, since these actors share the process-wide preparation pool with whatever else
    // runs in the test process; the decision must still be scheduled within a few visits.
    let request = StudioRepairRequest {
        receipt_a: pair[0].hash(),
        receipt_b: pair[1].hash(),
        selected: chosen.hash(),
    };
    let mut scheduled = false;
    for _ in 0..30 {
        let decided = control(
            &bob_actor,
            &bob_store,
            target,
            StudioControlAction::RepairFault(Box::new(request)),
        )
        .await
        .unwrap();
        match decided {
            StudioControlResponse::RepairStarted {
                start: StudioRepairStart::Scheduled,
                ..
            } => {
                scheduled = true;
                break;
            }
            StudioControlResponse::RepairStarted {
                start: StudioRepairStart::Busy,
                ..
            } => tick(&clock, &actors).await,
            other => panic!("a decision answers RepairStarted: {other:?}"),
        }
    }
    assert!(scheduled, "the decision is scheduled as a job");

    let mut converged = None;
    for _ in 0..120 {
        tick(&clock, &actors).await;
        if let Ok(Some(view)) = read(&carol_actor, &carol_store, target).await {
            if view.phase == EpochPhase::Open && view.epoch > 0 {
                converged = Some(view);
                break;
            }
        }
    }
    let carol_view = converged.expect("Carol applied the repair and installed its replacement");
    let bob_view = read(&bob_actor, &bob_store, target)
        .await
        .unwrap()
        .expect("Bob's source");
    assert_eq!(bob_view.phase, EpochPhase::Open);
    assert_eq!(
        (carol_view.epoch_id, carol_view.epoch),
        (bob_view.epoch_id, bob_view.epoch),
        "both sit on R1's successor"
    );
    assert_eq!(carol_view.projection, bob_view.projection);
    let bob_fault = fault_view(&bob_actor, &bob_store, target).await;
    assert!(
        matches!(
            bob_fault.last_attempt,
            Some(StudioRepairReport::Completed(outcome)) if outcome.is_terminal()
        ),
        "the owner's own job ended terminal: {:?}",
        bob_fault.last_attempt
    );
    let carol_fault = fault_view(&carol_actor, &carol_store, target).await;
    assert_eq!(
        carol_fault.last_attempt,
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::Installed
        )),
        "Carol's last job installed the replacement"
    );
    assert!(
        bob_fault
            .repair
            .as_ref()
            .is_some_and(|status| !status.held && status.disposition.is_some()),
        "the owner's decision is resolved and no longer held: {:?}",
        bob_fault.repair
    );
    // Carol kept reporting the pair until she applied the repair, so some reports reached Bob
    // after his own decision finished. They are answered by the repair, never staged again:
    // the decided pair is not offered for a second decision.
    assert_eq!(
        bob_fault.blocked_by,
        Some(StudioRepairBlocker::NoFault),
        "nothing is left to decide: {:?}",
        bob_fault.candidates
    );
    assert_ne!(
        carol_fault.blocked_by,
        Some(StudioRepairBlocker::Scheduled),
        "no job still owns Carol's document"
    );

    // Restart: both actors and mounts go away and come back from disk and snapshot.
    stop(bob_actor, bob_task, bob_drain).await;
    stop(carol_actor, carol_task, carol_drain).await;
    drop(bob_store.lock().await.take());
    drop(carol_store.lock().await.take());
    let mut bob = Node::restore(
        &bob_snapshot,
        hub.join(PeerId::from_u64(2)),
        ChaCha20Rng::seed_from_u64(301),
        Box::new(clock.clone()),
        "bob",
    )
    .unwrap();
    let mut carol = Node::restore(
        &carol_snapshot,
        hub.join(PeerId::from_u64(3)),
        ChaCha20Rng::seed_from_u64(302),
        Box::new(clock.clone()),
        "carol",
    )
    .unwrap();
    bob.subscribe_control().await.unwrap();
    carol.subscribe_control().await.unwrap();
    assert!(bob.is_owner());
    assert!(
        matches!(carol.observed_owner_tenure(), StudioOwnerTenure::Known(_)),
        "the observed tenure is restored with the snapshot"
    );
    {
        // What a restart finds on disk: Bob's source has left Fault and his owner record is
        // ordinary again, so nothing is held, owed or awaiting a second decision.
        let mut reopened = store_at(&bob_root, 201);
        let faulted = bob.sync.with_registry_context(|g, d, _, _| {
            reopened
                .studio_fault_evidence(SERVER, g, target, d, None)
                .unwrap()
                .expect("Bob's saved source")
                .source_faulted
        });
        assert!(!faulted, "the owner's source has left Fault");
        assert!(
            reopened
                .load_epoch_owner_receipts(SERVER, &target.document(&group_id).unwrap())
                .is_ok(),
            "the owner record is ordinary on disk"
        );
    }
    // The document newcomer: Dave has been a member throughout but never held this document,
    // and first asks for it now, after the repair and the restart.
    prove(&mut bob, &mut carol).await;
    prove(&mut carol, &mut bob).await;
    prove(&mut dave, &mut bob).await;
    prove(&mut bob, &mut dave).await;
    let dave_root = tempfile::tempdir().unwrap();
    let bob_store: Store = Arc::new(Mutex::new(Some(store_at(&bob_root, 201))));
    let carol_store: Store = Arc::new(Mutex::new(Some(store_at(&carol_root, 202))));
    let dave_store: Store = Arc::new(Mutex::new(Some(store_at(&dave_root, 203))));
    let (bob_actor, bob_task, bob_drain) = spawn(bob);
    let (carol_actor, carol_task, carol_drain) = spawn(carol);
    let (dave_actor, dave_task, dave_drain) = spawn(dave);
    let actors = [
        (&bob_actor, &bob_store),
        (&carol_actor, &carol_store),
        (&dave_actor, &dave_store),
    ];
    let _ = read(&bob_actor, &bob_store, target).await;
    let _ = read(&carol_actor, &carol_store, target).await;
    let _ = read(&dave_actor, &dave_store, target).await;
    let mut joined = None;
    for _ in 0..120 {
        tick(&clock, &actors).await;
        if let Ok(Some(view)) = read(&dave_actor, &dave_store, target).await {
            if view.phase == EpochPhase::Open && view.epoch == bob_view.epoch {
                joined = Some(view);
                break;
            }
        }
    }
    // Only an owner proof installs a document a vault has never held, so this also shows the
    // owner proving the repair's selected receipt again after the restart.
    let dave_view = joined.expect("the newcomer installed the repaired document ordinarily");
    assert_eq!(dave_view.projection, bob_view.projection);
    // Nothing reran after the restart: no job ran on any of the three, and both earlier
    // participants still sit on the repaired epoch.
    for (actor, store) in actors {
        let view = fault_view(actor, store, target).await;
        assert_ne!(
            view.blocked_by,
            Some(StudioRepairBlocker::Scheduled),
            "no job owns the document"
        );
        assert!(
            view.last_attempt.is_none(),
            "no repair ran after the restart"
        );
        let read = read(actor, store, target).await.unwrap().unwrap();
        assert_eq!((read.epoch, read.phase), (bob_view.epoch, EpochPhase::Open));
    }
    stop(bob_actor, bob_task, bob_drain).await;
    stop(carol_actor, carol_task, carol_drain).await;
    stop(dave_actor, dave_task, dave_drain).await;

    // The owner's record went back to ordinary in the same transaction that resolved it.
    let bob_store = bob_store.lock().await.take().unwrap();
    let logical = target.document(&group_id).unwrap();
    assert!(
        bob_store
            .load_epoch_owner_receipts(SERVER, &logical)
            .is_ok(),
        "the owner record is ordinary again"
    );
}

/// The Registry bucket behind `target`, with one signed pointer, and the two receipts Bob's tenure
/// signs closing its epoch 0: R1 (`chosen`, with its seed) and the rival R2.
fn bucket_fork(
    bob: &mut Node,
    store: &mut ServerStore,
    target: StudioTarget,
) -> (u8, [Receipt; 2], Vec<u8>) {
    let StudioOwnerTenure::Known(start) = bob.observed_owner_tenure() else {
        panic!("the owner observes its own tenure")
    };
    let bucket = bob.studio_registry_bucket(target).unwrap();
    let mut b = CatchupRuntime::budget(bob, store, SERVER).unwrap();
    let (receipts, seed) = bob.sync.with_registry_context(|group, device, _, rng| {
        use catcoms_replication::registry::{registry_document, PointerKey, RegistryOp};
        let logical = target.document(&group.group_id()).unwrap();
        let key = PointerKey::new(logical.doc_type, logical.logical_key).unwrap();
        let op = RegistryOp::Put { key, epoch: 1 }
            .domain_op(&group.group_id(), [1; 16])
            .unwrap();
        let sealed = catcoms_replication::registry_epoch::RegistryEpoch::new(
            group,
            bucket,
            device.device_id(),
        )
        .unwrap()
        .edit(device, group, rng, &op)
        .unwrap();
        store
            .with_studio_protocol_budget(SERVER, group, &mut b, |store, budget| {
                store.ingest_registry_epoch(SERVER, group, bucket, device, &sealed, rng, budget)
            })
            .unwrap();
        let state = store
            .load_registry_epoch(SERVER, group, bucket, device)
            .unwrap()
            .unwrap();
        let document = registry_document(&group.group_id(), bucket).unwrap();
        let signed = [20u8, 21].map(|salt| {
            let mut projection = state.projection().unwrap();
            projection.epoch = 0;
            let seed = projection.checkpoint([salt; 32]).unwrap();
            let receipt = Receipt::sign(
                document.clone(),
                0,
                [salt; 32],
                seed.change_hash(),
                start,
                InheritedCheckpoint::EpochZero,
                device,
            )
            .unwrap();
            (receipt, seed.bytes().to_vec())
        });
        let [(chosen, seed), (rival, _)] = signed;
        ([chosen, rival], seed)
    });
    (bucket, receipts, seed)
}

fn adopt_bucket(
    node: &mut Node,
    store: &mut ServerStore,
    bucket: u8,
    receipt: &Receipt,
    seed: Option<&[u8]>,
) -> crate::store::RegistryAdoptionOutcome {
    let tenure = node
        .sync
        .authoring_owner_tenure_start()
        .expect("an observed owner tenure");
    let mut b = CatchupRuntime::budget(node, store, SERVER).unwrap();
    node.sync
        .with_registry_context(|group, device, clock, rng| {
            store
                .with_studio_protocol_budget(SERVER, group, &mut b, |store, budget| {
                    store.adopt_registry_checkpoint(
                        SERVER, group, bucket, device, receipt, seed, tenure, clock, rng, budget,
                    )
                })
                .unwrap()
                .0
        })
}

async fn registry_fault_view(
    actor: &crate::ServerActor,
    store: &Store,
    target: StudioTarget,
) -> crate::studio::StudioFaultView {
    match control(actor, store, target, StudioControlAction::ReadRegistryFault).await {
        Ok(StudioControlResponse::Fault(view)) => *view,
        other => panic!("a Registry fault view: {other:?}"),
    }
}

/// Registry Flow D on a real second peer, through the actors. Bob installed and published his
/// bucket's R1, then met a rival R2 from his own tenure; Carol holds both receipts and no seed.
/// Bob decides R1 as a job. Bob's real discovery answer then carries the repair to Carol, through
/// the bucket leg of a query: either her fault-reporting Registry turn or an ordinary Studio
/// discovery, which always asks for the bucket first. The head answer carries an applied repair
/// whether or not a report was sent, so this pins that the repair comes from a real answer, not
/// which leg. Her job applies it, she owes R1's bucket seed, fetches it from Bob through a
/// repaired pass, and the router hands it to a Replace job. No step here calls a repair
/// transaction or runtime method directly.
#[tokio::test]
async fn a_peer_applies_a_bucket_repair_from_a_real_answer_and_installs_its_replacement() {
    let Cast {
        clock,
        mut bob,
        mut carol,
        ..
    } = cast().await;
    let target = index_target();
    let (bob_root, carol_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut bob_store = store_at(&bob_root, 211);
    let mut carol_store = store_at(&carol_root, 212);
    let (bucket, [chosen, rival], chosen_seed) = bucket_fork(&mut bob, &mut bob_store, target);
    use crate::store::RegistryAdoptionOutcome;
    assert_eq!(
        adopt_bucket(
            &mut bob,
            &mut bob_store,
            bucket,
            &chosen,
            Some(&chosen_seed)
        ),
        RegistryAdoptionOutcome::Installed
    );
    publish(&mut bob, &mut bob_store, &chosen);
    assert_eq!(
        adopt_bucket(&mut bob, &mut bob_store, bucket, &rival, None),
        RegistryAdoptionOutcome::Fault
    );
    assert_eq!(
        adopt_bucket(&mut carol, &mut carol_store, bucket, &chosen, None),
        RegistryAdoptionOutcome::AwaitingSeed
    );
    assert_eq!(
        adopt_bucket(&mut carol, &mut carol_store, bucket, &rival, None),
        RegistryAdoptionOutcome::Fault
    );
    let mut pair = [chosen.clone(), rival.clone()];
    pair.sort_by_key(Receipt::hash);

    let bob_store: Store = Arc::new(Mutex::new(Some(bob_store)));
    let carol_store: Store = Arc::new(Mutex::new(Some(carol_store)));
    let (bob_actor, bob_task, bob_drain) = spawn(bob);
    let (carol_actor, carol_task, carol_drain) = spawn(carol);
    let actors = [(&bob_actor, &bob_store), (&carol_actor, &carol_store)];
    let _ = read(&bob_actor, &bob_store, target).await;
    let _ = read(&carol_actor, &carol_store, target).await;
    tick(&clock, &actors).await;

    let request = StudioRepairRequest {
        receipt_a: pair[0].hash(),
        receipt_b: pair[1].hash(),
        selected: chosen.hash(),
    };
    let mut scheduled = false;
    for _ in 0..30 {
        match control(
            &bob_actor,
            &bob_store,
            target,
            StudioControlAction::RepairRegistryFault(Box::new(request)),
        )
        .await
        .unwrap()
        {
            StudioControlResponse::RepairStarted {
                scope: crate::studio::StudioFaultScope::RegistryBucket(scoped),
                start: StudioRepairStart::Scheduled,
                ..
            } => {
                assert_eq!(scoped, bucket);
                scheduled = true;
                break;
            }
            StudioControlResponse::RepairStarted {
                start: StudioRepairStart::Busy,
                ..
            } => tick(&clock, &actors).await,
            other => panic!("a bucket decision answers RepairStarted: {other:?}"),
        }
    }
    assert!(scheduled, "the bucket decision is scheduled as a job");

    let mut installed = None;
    for _ in 0..150 {
        tick(&clock, &actors).await;
        let view = registry_fault_view(&carol_actor, &carol_store, target).await;
        if view.last_attempt
            == Some(StudioRepairReport::Completed(
                StudioRepairOutcome::Installed,
            ))
        {
            installed = Some(view);
            break;
        }
    }
    let carol_view = installed.expect("Carol applied the bucket repair and installed its seed");
    let bob_view = registry_fault_view(&bob_actor, &bob_store, target).await;
    assert!(
        matches!(
            bob_view.last_attempt,
            Some(StudioRepairReport::Completed(outcome)) if outcome.is_terminal()
        ),
        "the owner's bucket job ended terminal: {:?}",
        bob_view.last_attempt
    );
    assert_eq!(carol_view.source.phase, EpochPhase::Open);
    assert_eq!(
        (carol_view.source.epoch_id, carol_view.source.epoch),
        (bob_view.source.epoch_id, bob_view.source.epoch),
        "both buckets sit on R1's successor"
    );
    assert_ne!(
        carol_view.blocked_by,
        Some(StudioRepairBlocker::Scheduled),
        "no job still owns Carol's bucket"
    );
    stop(bob_actor, bob_task, bob_drain).await;
    stop(carol_actor, carol_task, carol_drain).await;
}

/// Run one scheduled job's S2 and S3, as the actor would.
async fn run_job(
    runtime: &mut CatchupRuntime,
    node: &mut Node,
    store: &mut ServerStore,
    clock: &ManualClock,
) {
    let job = runtime
        .repair_detach::<MemNetwork>()
        .expect("a captured job detaches");
    let StudioBackgroundResult::Repair(completion) = job.run(None).await else {
        panic!("a repair rebuild completes as a repair result")
    };
    runtime.repair_complete(completion, clock.monotonic_ms());
    runtime.repair_commit(node, store, SERVER).unwrap();
}

/// Review MEDIUM-1, the held half, on a real peer with an observed tenure. Carol has applied Bob's
/// repair and owes its replacement. A second current-tenure repair of the same pair with the same
/// sequence number, choosing the other receipt, is held by the transaction with
/// `SequenceNotNewer`. That holds only that repair, never her document, so the real repair's seed
/// is still fetched, and the held one costs no further job while held. (A byte-identical replay
/// never reaches a job at all: the owed filter in `offer_repair` sends it to the seed fetch.)
#[tokio::test]
async fn a_held_replay_on_a_real_peer_holds_only_itself_and_the_owed_seed_is_still_fetched() {
    let Cast {
        clock,
        mut bob,
        mut carol,
        ..
    } = cast().await;
    let target = index_target();
    let scope = CheckpointTarget::Studio(target);
    let (bob_root, carol_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut bob_store = store_at(&bob_root, 221);
    let mut carol_store = store_at(&carol_root, 222);
    let ([chosen, rival], chosen_seed) = fork(&mut bob, &mut bob_store, target);
    adopt(
        &mut bob,
        &mut bob_store,
        target,
        &chosen,
        Some(&chosen_seed),
    );
    publish(&mut bob, &mut bob_store, &chosen);
    adopt(&mut bob, &mut bob_store, target, &rival, None);
    adopt(&mut carol, &mut carol_store, target, &chosen, None);
    assert_eq!(
        adopt(&mut carol, &mut carol_store, target, &rival, None),
        StudioAdoptionOutcome::Fault
    );
    let mut pair = [chosen.clone(), rival.clone()];
    pair.sort_by_key(Receipt::hash);

    // Bob's real decision, and a replay: his own signature, the same tenure and pair, no newer.
    let snapshot = bob.prepare_owner_head_snapshot(&bob_store, SERVER).unwrap();
    let mut b = CatchupRuntime::budget(&mut bob, &mut bob_store, SERVER).unwrap();
    let (real, _, _) = bob
        .issue_studio_fault_repair(
            &mut bob_store,
            SERVER,
            target,
            &snapshot,
            StudioRepairRequest {
                receipt_a: pair[0].hash(),
                receipt_b: pair[1].hash(),
                selected: chosen.hash(),
            },
            None,
            &mut b,
        )
        .unwrap();
    let StudioOwnerTenure::Known(start) = bob.observed_owner_tenure() else {
        panic!("the owner observes its own tenure")
    };
    let replay = bob.sync.with_registry_context(|_, device, _, _| {
        ReceiptRepair::sign_in_tenure(
            chosen.document.clone(),
            chosen.tenure_id,
            [pair[0].hash(), pair[1].hash()],
            rival.hash(),
            real.repair_sequence,
            start,
            device,
        )
        .unwrap()
    });
    assert_ne!(replay.hash(), real.hash());

    let mut runtime = CatchupRuntime {
        target: Some(target),
        ..Default::default()
    };
    runtime.inject_overlay_pool_for_test(4);
    assert!(
        runtime.offer_repair(
            &mut carol,
            &mut carol_store,
            SERVER,
            target,
            &real,
            None,
            false
        ),
        "Carol, with an observed tenure, takes the real repair as a job"
    );
    run_job(&mut runtime, &mut carol, &mut carol_store, &clock).await;
    assert_eq!(
        runtime.repair_report(scope),
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::AwaitingSeed
        ))
    );
    assert!(
        runtime
            .checkpoint
            .as_ref()
            .is_some_and(|pass| pass.inner.selected_receipt() == &chosen),
        "the real repair's seed is fetched from Bob"
    );
    // That fetch is dropped, as a failed one would be, and the rival repair arrives.
    runtime.checkpoint = None;
    assert!(runtime.offer_repair(
        &mut carol,
        &mut carol_store,
        SERVER,
        target,
        &replay,
        None,
        false
    ));
    run_job(&mut runtime, &mut carol, &mut carol_store, &clock).await;
    assert_eq!(
        runtime.repair_report(scope),
        Some(StudioRepairReport::Completed(StudioRepairOutcome::Held(
            catcoms_replication::RepairHold::SequenceNotNewer
        ))),
        "the transaction holds a repair no newer than the one applied"
    );
    assert!(
        runtime
            .repair_unverifiable
            .contains_key(&(scope, replay.hash())),
        "the replay is held"
    );
    assert!(
        !runtime.repair_backoff.contains_key(&scope),
        "a held replay never holds the document it names"
    );
    runtime.await_repaired_seed(
        &mut carol,
        &carol_store,
        SERVER,
        scope,
        Some(target),
        &real,
        &pair,
    );
    assert!(
        runtime
            .checkpoint
            .as_ref()
            .is_some_and(|pass| pass.inner.selected_receipt() == &chosen),
        "the real repair's replacement is still fetched"
    );
    runtime.checkpoint = None;
    assert!(
        !runtime.offer_repair(
            &mut carol,
            &mut carol_store,
            SERVER,
            target,
            &replay,
            None,
            false
        ),
        "the held replay costs no further job"
    );
    assert!(runtime.repair_job_target().is_none());
}

/// One local Flipnote title edit on epoch 0, through the ordinary Save transaction.
fn edit_title(node: &mut Node, store: &mut ServerStore, target: StudioTarget, n: u8) {
    use catcoms_replication::studio::{FlipnoteHeader, FlipnoteOp};
    let logical = target.document(&node.group_id()).unwrap();
    node.studio_transaction(
        store,
        SERVER,
        StudioRequest::Apply {
            target,
            epoch_id: catcoms_replication::epoch_zero_id(logical.doc_type, &logical.logical_key),
            nonce: [n; 16],
            body: FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!("edit {n}")))
                .encode()
                .unwrap(),
        },
    )
    .unwrap();
}

/// Plan D: S3 completes within bounded turns while a `PageReady` page pass is pending on the very
/// target the job claims. A page pass parks catch-up (`replay_ready`), and the claim defers that
/// page, so the job must never wait on it: one receive turn commits S3 first and only then, the
/// claim released, persists the page. Carol's page comes from Bob through the real fetch adapters.
#[tokio::test]
async fn a_pending_page_on_the_claimed_target_never_delays_s3() {
    use crate::studio_exchange::StudioReceiveState;
    let Cast {
        clock,
        mut bob,
        mut carol,
        ..
    } = cast().await;
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [7; 16],
    };
    let scope = CheckpointTarget::Studio(target);
    let (bob_root, carol_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut bob_store = store_at(&bob_root, 231);
    let mut carol_store = store_at(&carol_root, 232);
    edit_title(&mut carol, &mut carol_store, target, 1);
    edit_title(&mut bob, &mut bob_store, target, 2);
    let ops = |node: &mut Node, store: &ServerStore| {
        node.sync.with_registry_context(|g, d, _, _| {
            store
                .load_studio_epoch(SERVER, g, target, d)
                .unwrap()
                .unwrap()
                .op_count()
        })
    };
    assert_eq!(ops(&mut carol, &carol_store), 1);

    let mut receiver = crate::studio::StudioReceiver::default();
    receiver.preparation_pools_for_test(
        Arc::new(tokio::sync::Semaphore::new(4)),
        Arc::new(tokio::sync::Semaphore::new(3)),
    );
    receiver
        .run(
            &mut carol,
            &mut carol_store,
            SERVER,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();

    // Bob's page, fetched to PageReady through the real adapters: independent frontiers restart
    // first, then request the bounded prefix.
    let carol_watch = carol
        .watch_studio_epoch(&carol_store, SERVER, target)
        .unwrap();
    let bob_watch = bob.watch_studio_epoch(&bob_store, SERVER, target).unwrap();
    let mut b = CatchupRuntime::budget(&mut carol, &mut carol_store, SERVER).unwrap();
    let mut pass = carol
        .begin_studio_receive(&mut carol_store, &carol_watch, bob.local_peer(), &mut b)
        .unwrap();
    let mut provider = bob.studio_page_provider(&bob_store, SERVER);
    let mut ready = false;
    for _ in 0..3 {
        let attempt = carol
            .prepare_studio_receive_step(&mut pass)
            .unwrap()
            .unwrap();
        let (result, ()) = tokio::join!(attempt.fetch(), async {
            bob.sync_once().await.unwrap();
            bob.serve_studio_request_step(&mut bob_store, &mut provider, &bob_watch)
                .unwrap();
        });
        if carol
            .complete_studio_receive_step(&mut pass, result)
            .unwrap()
            == StudioReceiveState::PageReady
        {
            ready = true;
            break;
        }
        clock.advance_ms(1_000);
    }
    assert!(ready, "Bob's page is ready to persist");

    // A repair job claims the same document; its rebuild finishes and parks for S3. The repair is
    // Bob's but names receipts Carol never held, so S3 holds that repair and writes nothing.
    let StudioOwnerTenure::Known(start) = bob.observed_owner_tenure() else {
        panic!("the owner observes its own tenure")
    };
    let repair = bob.sync.with_registry_context(|group, device, _, _| {
        ReceiptRepair::sign_in_tenure(
            target.document(&group.group_id()).unwrap(),
            [3; 32],
            [[1; 32], [2; 32]],
            [1; 32],
            1,
            start,
            device,
        )
        .unwrap()
    });
    assert!(receiver.catchup.offer_repair(
        &mut carol,
        &mut carol_store,
        SERVER,
        target,
        &repair,
        None,
        false
    ));
    assert!(receiver.catchup.repair_claimed(scope));
    let job = receiver
        .catchup
        .repair_detach::<MemNetwork>()
        .expect("the job detaches");
    let StudioBackgroundResult::Repair(completion) = job.run(None).await else {
        panic!("a repair result")
    };
    receiver
        .catchup
        .repair_complete(completion, clock.monotonic_ms());
    assert!(receiver.catchup.repair_parked());

    // The page arrives while the job is parked on the claimed target.
    receiver.hold_page_for_test(target, pass);
    receiver
        .run(&mut carol, &mut carol_store, SERVER, None)
        .unwrap();
    assert!(
        receiver.catchup.repair_job_target().is_none() && !receiver.catchup.repair_claimed(scope),
        "S3 committed in the first turn despite the pending page"
    );
    assert!(
        matches!(
            receiver.catchup.repair_report(scope),
            Some(StudioRepairReport::Failed(ref why)) if why.contains("cannot verify")
        ),
        "{:?}",
        receiver.catchup.repair_report(scope)
    );
    assert_eq!(
        ops(&mut carol, &carol_store),
        2,
        "the claim released, the same turn persisted Bob's page"
    );
    assert!(!receiver.take_pause_notice());
}

/// One local Index edit on whichever epoch the source sits on now.
fn edit_index(node: &mut Node, store: &mut ServerStore, target: StudioTarget, n: u8) {
    let mut b = CatchupRuntime::budget(node, store, SERVER).unwrap();
    node.sync.with_registry_context(|group, device, _, rng| {
        let logical = target.document(&group.group_id()).unwrap();
        let id = store
            .load_studio_epoch(SERVER, group, target, device)
            .unwrap()
            .unwrap()
            .doc_id();
        let op = DomainOp {
            nonce: [n; 16],
            doc_type: logical.doc_type,
            logical_key: logical.logical_key.clone(),
            body: IndexOp::PutObject {
                object: [n; 16],
                kind: StudioKind::Flipnote,
                title: format!("star {n}"),
                created_by: device.device_id(),
                ts: 200,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap(),
        };
        store
            .edit_studio_epoch(SERVER, group, target, id, device, op, 200, rng, &mut b)
            .unwrap();
    });
}

/// Review LOW-4 on the fairness round: an S3 that writes, with a page pending on the same target.
/// Bob and Carol both installed R1 and sit healthy on its successor, and Carol has fetched Bob's
/// page of that epoch to `PageReady`. Then Bob's repair choosing the rival R2 reaches her, and S3
/// retargets her source onto R2. S3 returns before the page step, so the page would be saved on
/// the next turn against a source that is no longer the one it was fetched for. That save is
/// refused with the pass already `Paused`, which pauses all catch-up. `finish_studio_repair`
/// drops the stale page instead; a later pass fetches against the repaired source.
#[tokio::test]
async fn a_repair_that_retargets_a_source_with_a_pending_page_never_pauses_catch_up() {
    use crate::studio_exchange::StudioReceiveState;
    let Cast {
        clock,
        mut bob,
        mut carol,
        ..
    } = cast().await;
    let target = index_target();
    let scope = CheckpointTarget::Studio(target);
    let (bob_root, carol_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut bob_store = store_at(&bob_root, 241);
    let mut carol_store = store_at(&carol_root, 242);
    let ([chosen, rival], chosen_seed) = fork(&mut bob, &mut bob_store, target);
    for (node, store) in [(&mut bob, &mut bob_store), (&mut carol, &mut carol_store)] {
        assert_eq!(
            adopt(node, store, target, &chosen, Some(&chosen_seed)),
            StudioAdoptionOutcome::Installed
        );
    }
    edit_index(&mut bob, &mut bob_store, target, 5);

    let mut receiver = crate::studio::StudioReceiver::default();
    receiver.preparation_pools_for_test(
        Arc::new(tokio::sync::Semaphore::new(4)),
        Arc::new(tokio::sync::Semaphore::new(3)),
    );
    receiver
        .run(
            &mut carol,
            &mut carol_store,
            SERVER,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();

    // Bob's page of R1's successor, fetched to PageReady through the real adapters. Pages are
    // served from and saved into prepared sources only; the preparation's install is a retain.
    for (node, store) in [(&mut bob, &mut bob_store), (&mut carol, &mut carol_store)] {
        node.sync.with_registry_context(|g, d, _, _| {
            let state = store
                .load_studio_epoch(SERVER, g, target, d)
                .unwrap()
                .unwrap();
            store.retain_studio_source(g, d, state);
        });
    }
    let carol_watch = carol
        .watch_studio_epoch(&carol_store, SERVER, target)
        .unwrap();
    let bob_watch = bob.watch_studio_epoch(&bob_store, SERVER, target).unwrap();
    let mut b = CatchupRuntime::budget(&mut carol, &mut carol_store, SERVER).unwrap();
    let mut pass = carol
        .begin_studio_receive(&mut carol_store, &carol_watch, bob.local_peer(), &mut b)
        .unwrap();
    let mut provider = bob.studio_page_provider(&bob_store, SERVER);
    let mut ready = false;
    for _ in 0..3 {
        let attempt = carol
            .prepare_studio_receive_step(&mut pass)
            .unwrap()
            .unwrap();
        let (result, ()) = tokio::join!(attempt.fetch(), async {
            bob.sync_once().await.unwrap();
            bob.serve_studio_request_step(&mut bob_store, &mut provider, &bob_watch)
                .unwrap();
        });
        if carol
            .complete_studio_receive_step(&mut pass, result)
            .unwrap()
            == StudioReceiveState::PageReady
        {
            ready = true;
            break;
        }
        clock.advance_ms(1_000);
    }
    assert!(ready, "Bob's page is ready to persist");

    // Bob's repair of the pair, choosing the rival. Carol's source holds R1 and the offer carries
    // R2, so she can assemble the pair, and her healthy source on R1 is retargeted.
    let StudioOwnerTenure::Known(start) = bob.observed_owner_tenure() else {
        panic!("the owner observes its own tenure")
    };
    let mut pair = [chosen.clone(), rival.clone()];
    pair.sort_by_key(Receipt::hash);
    let repair = bob.sync.with_registry_context(|_, device, _, _| {
        ReceiptRepair::sign_in_tenure(
            chosen.document.clone(),
            chosen.tenure_id,
            [pair[0].hash(), pair[1].hash()],
            rival.hash(),
            1,
            start,
            device,
        )
        .unwrap()
    });
    assert!(receiver.catchup.offer_repair(
        &mut carol,
        &mut carol_store,
        SERVER,
        target,
        &repair,
        Some(&rival),
        false
    ));
    let job = receiver
        .catchup
        .repair_detach::<MemNetwork>()
        .expect("the job detaches");
    let StudioBackgroundResult::Repair(completion) = job.run(None).await else {
        panic!("a repair result")
    };
    receiver
        .catchup
        .repair_complete(completion, clock.monotonic_ms());
    assert!(receiver.catchup.repair_parked());

    // The page is pending when S3 commits the retarget.
    receiver.hold_page_for_test(target, pass);
    receiver
        .run(&mut carol, &mut carol_store, SERVER, None)
        .unwrap();
    assert_eq!(
        receiver.catchup.repair_report(scope),
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::AwaitingSeed
        )),
        "S3 retargeted the source onto R2, whose seed it now owes"
    );
    // The next turns reach the page step, which used to save the stale page and pause.
    for _ in 0..3 {
        clock.advance_ms(1_000);
        let turn = receiver.run(&mut carol, &mut carol_store, SERVER, None);
        assert!(
            turn.is_ok() && !receiver.take_pause_notice(),
            "a repair must never pause catch-up: {:?}",
            turn.as_ref().err()
        );
    }
    assert!(
        receiver.catchup.pass.is_none(),
        "the page fetched for the old source was dropped, never saved"
    );
}

/// Bob's repair of the fork's pair, choosing `selected`, signed in his current tenure.
fn sign_repair(
    bob: &mut Node,
    [chosen, rival]: [&Receipt; 2],
    selected: &Receipt,
    sequence: u64,
) -> ReceiptRepair {
    let StudioOwnerTenure::Known(start) = bob.observed_owner_tenure() else {
        panic!("the owner observes its own tenure")
    };
    let mut hashes = [chosen.hash(), rival.hash()];
    hashes.sort();
    bob.sync.with_registry_context(|_, device, _, _| {
        ReceiptRepair::sign_in_tenure(
            chosen.document.clone(),
            chosen.tenure_id,
            hashes,
            selected.hash(),
            sequence,
            start,
            device,
        )
        .unwrap()
    })
}

/// A peer-side runtime watching `target`, with a private pool.
fn peer_runtime(target: StudioTarget) -> CatchupRuntime {
    let mut runtime = CatchupRuntime {
        target: Some(target),
        ..Default::default()
    };
    runtime.inject_overlay_pool_for_test(4);
    runtime
}

/// PR #36 review MEDIUM-1: the same signed repair can arrive again, while its job runs, with the
/// receipt the first answer lacked. Carol sits healthy on R1, and Bob's repair choosing R2 names a
/// pair she can complete only with R2 from an answer. The first offer carries no receipt; while
/// its rebuild is detached, the same repair arrives with R2. That receipt is kept for S3, which
/// retargets her source, instead of holding the repair as unverifiable for a minute.
#[tokio::test]
async fn a_repeated_offer_brings_the_receipt_its_running_job_lacked() {
    let Cast {
        clock,
        mut bob,
        mut carol,
        ..
    } = cast().await;
    let target = index_target();
    let scope = CheckpointTarget::Studio(target);
    let (bob_root, carol_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut bob_store = store_at(&bob_root, 251);
    let mut carol_store = store_at(&carol_root, 252);
    let ([chosen, rival], chosen_seed) = fork(&mut bob, &mut bob_store, target);
    assert_eq!(
        adopt(
            &mut carol,
            &mut carol_store,
            target,
            &chosen,
            Some(&chosen_seed)
        ),
        StudioAdoptionOutcome::Installed
    );
    let repair = sign_repair(&mut bob, [&chosen, &rival], &rival, 1);
    let mut runtime = peer_runtime(target);
    assert!(
        runtime.offer_repair(
            &mut carol,
            &mut carol_store,
            SERVER,
            target,
            &repair,
            None,
            false
        ),
        "the first answer, with no receipt, starts a job"
    );
    let job = runtime
        .repair_detach::<MemNetwork>()
        .expect("its rebuild detaches");
    // The same repair again, now with R2, while that rebuild runs.
    assert!(runtime.offer_repair(
        &mut carol,
        &mut carol_store,
        SERVER,
        target,
        &repair,
        Some(&rival),
        false
    ));
    let StudioBackgroundResult::Repair(completion) = job.run(None).await else {
        panic!("a repair result")
    };
    runtime.repair_complete(completion, clock.monotonic_ms());
    runtime
        .repair_commit(&mut carol, &mut carol_store, SERVER)
        .unwrap();
    assert_eq!(
        runtime.repair_report(scope),
        Some(StudioRepairReport::Completed(
            StudioRepairOutcome::AwaitingSeed
        )),
        "S3 used the receipt the second answer brought"
    );
    assert!(!runtime
        .repair_unverifiable
        .contains_key(&(scope, repair.hash())));
}

/// PR #36 residual LOW-1: an offered repair whose rebuild went stale (the source changed during
/// S2) holds that repair for the short stale wait, never the document, so another legitimate
/// repair is taken at once.
#[tokio::test]
async fn a_stale_rebuild_of_an_offer_holds_only_that_offer() {
    let Cast {
        clock,
        mut bob,
        mut carol,
        ..
    } = cast().await;
    let target = index_target();
    let scope = CheckpointTarget::Studio(target);
    let (bob_root, carol_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut bob_store = store_at(&bob_root, 271);
    let mut carol_store = store_at(&carol_root, 272);
    let ([chosen, rival], chosen_seed) = fork(&mut bob, &mut bob_store, target);
    assert_eq!(
        adopt(
            &mut carol,
            &mut carol_store,
            target,
            &chosen,
            Some(&chosen_seed)
        ),
        StudioAdoptionOutcome::Installed
    );
    let first = sign_repair(&mut bob, [&chosen, &rival], &rival, 1);
    let mut runtime = peer_runtime(target);
    assert!(runtime.offer_repair(
        &mut carol,
        &mut carol_store,
        SERVER,
        target,
        &first,
        Some(&rival),
        false
    ));
    let job = runtime
        .repair_detach::<MemNetwork>()
        .expect("its rebuild detaches");
    // Carol's source changes while the rebuild runs, so S3 finds it stale.
    edit_index(&mut carol, &mut carol_store, target, 7);
    let StudioBackgroundResult::Repair(completion) = job.run(None).await else {
        panic!("a repair result")
    };
    runtime.repair_complete(completion, clock.monotonic_ms());
    runtime
        .repair_commit(&mut carol, &mut carol_store, SERVER)
        .unwrap();
    assert!(
        matches!(
            runtime.repair_report(scope),
            Some(StudioRepairReport::Failed(ref why)) if why.contains("changed")
        ),
        "{:?}",
        runtime.repair_report(scope)
    );
    assert_eq!(
        runtime.repair_unverifiable.get(&(scope, first.hash())),
        Some(&(clock.monotonic_ms() + 5_000)),
        "the stale offer waits the short stale retry"
    );
    assert!(
        !runtime.repair_backoff.contains_key(&scope),
        "the document does not wait behind a stale offer"
    );
    let second = sign_repair(&mut bob, [&chosen, &rival], &rival, 2);
    assert!(
        runtime.offer_repair(
            &mut carol,
            &mut carol_store,
            SERVER,
            target,
            &second,
            Some(&rival),
            false
        ),
        "another repair is taken at once"
    );
}

/// PR #36 review MEDIUM-2: an offered repair for a document this peer holds no copy of holds that
/// repair, never the document. Once a copy arrives, another legitimate repair of it is taken at
/// once; a document-wide hold kept that one waiting a minute.
#[tokio::test]
async fn an_offer_for_a_document_this_peer_lacks_holds_only_that_offer() {
    let Cast {
        mut bob, mut carol, ..
    } = cast().await;
    let target = index_target();
    let scope = CheckpointTarget::Studio(target);
    let (bob_root, carol_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut bob_store = store_at(&bob_root, 261);
    let mut carol_store = store_at(&carol_root, 262);
    let ([chosen, rival], chosen_seed) = fork(&mut bob, &mut bob_store, target);
    let first = sign_repair(&mut bob, [&chosen, &rival], &chosen, 1);
    let mut runtime = peer_runtime(target);
    assert!(
        !runtime.offer_repair(
            &mut carol,
            &mut carol_store,
            SERVER,
            target,
            &first,
            None,
            false
        ),
        "no copy here, so nothing to repair"
    );
    assert!(runtime.repair_job_target().is_none());
    assert!(
        runtime
            .repair_unverifiable
            .contains_key(&(scope, first.hash())),
        "that offer is held"
    );
    assert!(
        !runtime.repair_backoff.contains_key(&scope),
        "its document is not held"
    );
    // A copy arrives, and with it another legitimate repair of the document.
    assert_eq!(
        adopt(
            &mut carol,
            &mut carol_store,
            target,
            &chosen,
            Some(&chosen_seed)
        ),
        StudioAdoptionOutcome::Installed
    );
    let second = sign_repair(&mut bob, [&chosen, &rival], &rival, 2);
    assert!(
        runtime.offer_repair(
            &mut carol,
            &mut carol_store,
            SERVER,
            target,
            &second,
            Some(&rival),
            false
        ),
        "another repair of the document is taken at once"
    );
    assert_eq!(runtime.repair_job_target(), Some(scope));
}
