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

/// Spawn an actor whose event stream must never report a paused receiver.
fn spawn(
    node: Node,
) -> (
    crate::ServerActor,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
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

#[tokio::test]
async fn a_fault_is_decided_replaced_and_survives_restart_and_a_newcomer_through_the_actors() {
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

    // The fork: Bob installed R1 and then met R2; Carol holds both receipts and no seed.
    let target = StudioTarget::Index {
        channel: crate::channel_id("general").to_be_bytes(),
    };
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
