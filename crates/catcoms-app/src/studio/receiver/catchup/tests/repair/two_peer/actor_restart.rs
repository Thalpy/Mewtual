//! Preserve the complete real-actor repair/restart/newcomer evidence while keeping
//! its successive node fixtures out of one default-stack Tokio test future.
//!
//! The root Linux/Windows suites use ordinary test-thread stacks. The original
//! monolithic future retained the first cast and the restored cast together with
//! nested setup/network futures; moving/polling that debug frame overflowed before
//! assertions could finish. No larger runtime/thread stack or alternate repair path
//! is used here. Every network turn, assertion and actor shutdown remains genuine.
use super::*;
use std::future::Future;
use zeroize::Zeroizing;

/// Only evidence needed after ALL first-generation actors have shut down and both
/// mounts have closed. Dave is the same pre-succession member, held by one pointer;
/// no restarted Server or live actor/lease can accidentally cross this phase boundary.
struct RestartEvidence {
    hub: Arc<Hub>,
    clock: ManualClock,
    dave: Box<Node>,
    bob_root: tempfile::TempDir,
    carol_root: tempfile::TempDir,
    bob_snapshot: Zeroizing<Vec<u8>>,
    carol_snapshot: Zeroizing<Vec<u8>>,
    group_id: Vec<u8>,
    target: StudioTarget,
    bob_view: StudioView,
}

/// The registered regression awaits both boxed phases sequentially. It still runs
/// the entire original scenario on the test runtime's unchanged default stack.
pub(super) async fn run() {
    let restart = Box::pin(repair_and_stop()).await;
    Box::pin(restart_and_join(restart)).await;
}

async fn repair_and_stop() -> Box<RestartEvidence> {
    let Cast {
        hub,
        clock,
        bob,
        carol,
        dave,
    } = Box::pin(cast()).await;
    // Retain one pointer per live fixture across actor/network awaits. The nodes
    // remain the same originals; only their residence changes.
    let (mut bob, mut carol, dave) = (Box::new(bob), Box::new(carol), Box::new(dave));

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
    let (bob_actor, bob_task, bob_drain) = spawn(*bob);
    let (carol_actor, carol_task, carol_drain) = spawn(*carol);
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

    Box::new(RestartEvidence {
        hub,
        clock,
        dave,
        bob_root,
        carol_root,
        bob_snapshot,
        carol_snapshot,
        group_id,
        target,
        bob_view,
    })
}

async fn restart_and_join(restart: Box<RestartEvidence>) {
    let RestartEvidence {
        hub,
        clock,
        mut dave,
        bob_root,
        carol_root,
        bob_snapshot,
        carol_snapshot,
        group_id,
        target,
        bob_view,
    } = *restart;
    let mut bob = Box::new(
        Node::restore(
            &bob_snapshot,
            hub.join(PeerId::from_u64(2)),
            ChaCha20Rng::seed_from_u64(301),
            Box::new(clock.clone()),
            "bob",
        )
        .unwrap(),
    );
    let mut carol = Box::new(
        Node::restore(
            &carol_snapshot,
            hub.join(PeerId::from_u64(3)),
            ChaCha20Rng::seed_from_u64(302),
            Box::new(clock.clone()),
            "carol",
        )
        .unwrap(),
    );
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
    let (bob_actor, bob_task, bob_drain) = spawn(*bob);
    let (carol_actor, carol_task, carol_drain) = spawn(*carol);
    let (dave_actor, dave_task, dave_drain) = spawn(*dave);
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

#[test]
fn actor_repair_restart_driver_has_no_inline_cast_or_scenario_future() {
    // Inspect the concrete driver future TYPE without constructing/polling a
    // fixture. A direct await of either large stage (or setup) must not silently
    // put it back inside the Tokio test's inline frame. The loose 16KiB ceiling
    // allows compiler/target layout variation while retaining >100x headroom in
    // the ordinary 2MiB test stack for poll/call frames and crypto/storage work.
    fn future_bytes<F: Future>(_: impl FnOnce() -> F) -> usize {
        std::mem::size_of::<F>()
    }
    let bytes = future_bytes(run);
    assert!(
        bytes <= 16 * 1024,
        "repair/restart driver future grew to {bytes} bytes"
    );
}
