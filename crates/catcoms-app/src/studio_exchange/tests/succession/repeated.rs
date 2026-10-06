//! A -> B -> A with the SAME key, through real MLS membership changes, the actor, a restart at
//! each transition and real Studio rotation (design 9.2 T1 and T2; 17.1 N-T1 and N-T6).
//!
//! The acceptance line this exists for: real actor A -> B -> A flows "reject an earlier tenure's
//! otherwise valid same-key receipt and make progress only with independently valid new
//! authority". Here:
//!
//! - **Progress with new authority:** B, after A's removal, issues its first receipt under its own
//!   observed tenure, through ordinary idle passes after a restart, inheriting A's checkpoint.
//! - **The same-key refusal:** A's key is then admitted again and owns again. A's old receipt is
//!   therefore cryptographically perfect, and verifies under the tenure it claims. Only the
//!   tenure start the witness actually *observed* tells the two tenures apart, and that refuses
//!   it, including after the witness restarts.
//!
//! **Why A owns again by its own admission.** MLS fills the leftmost blank leaf, and the lowest leaf
//! is the designated committer. So a device that left and returns always takes the founder's freed
//! leaf and owns from its admission epoch. That is also why this runs on a legacy group:
//! authenticated P2P policy refuses an admission that changes the owner (`joining.rs`'s negative
//! case).
//!
//! **What this cannot show yet:** the returning device's *own* side. It cannot process the
//! Welcome, because its MLS provider still holds its old membership of the same group. The test
//! pins that failure so that the day it is lifted, the returning owner's first receipt of its
//! second tenure is added here.
use super::joining::{drain_events, step};
use super::*;
use catcoms_replication::Receipt;

/// Ordinary actor idle passes until `target`'s source is at `epoch`, Open, and the Registry
/// pointer names it. Returns whether that state was reached in the bounded window.
async fn rotate_to(
    actor: &crate::ServerActor,
    store: &Arc<Mutex<Option<ServerStore>>>,
    verifier: &mut Node,
    clock: &ManualClock,
    target: StudioTarget,
    epoch: u64,
) -> bool {
    let logical = target.document(&verifier.group_id()).unwrap();
    let pointer = PointerKey::new(logical.doc_type, logical.logical_key.clone()).unwrap();
    // The actor models its own process, so it gets its own preparation pools rather than the
    // process-wide ones every parallel test draws on. A bounded step window over the shared pool
    // is the shape `joining.rs` measured refusing 72 times under parallel load.
    actor
        .studio_preparation_pools_for_test(
            Arc::new(tokio::sync::Semaphore::new(4)),
            Arc::new(tokio::sync::Semaphore::new(3)),
        )
        .await;
    // An ordinary Read first, as the UI would, so the actor's receiver knows the document.
    save(actor, store, StudioRequest::Read { target })
        .await
        .unwrap();
    for _ in 0..60 {
        clock.advance_ms(1000);
        step(actor, store).await;
        actor.wait_studio_preparation().await;
        let guard = store.lock().await;
        let held = guard.as_ref().unwrap();
        let done = verifier.sync.with_registry_context(|g, d, _, _| {
            held.load_studio_epoch(SERVER, g, target, d)
                .unwrap()
                .is_some_and(|s| s.epoch() == epoch && s.phase() == EpochPhase::Open)
                && held
                    .load_registry_epoch(SERVER, g, pointer.bucket(), d)
                    .unwrap()
                    .is_some_and(|r| r.projection().unwrap().pointers.get(&pointer) == Some(&epoch))
        });
        if done {
            return true;
        }
    }
    false
}

/// The receipt `store` published for `target`, with no decision still pending.
fn published(store: &ServerStore, group_id: &[u8], target: StudioTarget) -> Receipt {
    let logical = target.document(group_id).unwrap();
    let journal = store.load_epoch_owner_receipts(SERVER, &logical).unwrap();
    assert!(journal.pending().is_none());
    journal.published().expect("a published receipt").clone()
}

#[tokio::test]
async fn studio_actor_a_to_b_to_a_progresses_under_new_tenure_and_refuses_the_returning_keys_old_one(
) {
    // --- Tenure 0. A owns: it closes epoch 0 and installs epoch 1 under R0, signed by A's key. B,
    // a member while A is provably owner, adopts that checkpoint, and holds A's eligible history in
    // epoch 1, enough to reach the production rotation threshold.
    let mut p = Pair::new_legacy().await;
    let target = target();
    let (r0, seed, _) = super::super::discovery::prepared_checkpoint(&mut p, target);
    assert_eq!(r0.tenure_start_group_epoch, 0);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    p.bob.sync.with_registry_context(|g, d, _, rng| {
        let (outcome, _) = p
            .b_store
            .adopt_studio_checkpoint(
                SERVER,
                g,
                target,
                d,
                &r0,
                Some(seed.bytes()),
                0,
                &p.clock,
                rng,
                &mut b,
            )
            .unwrap();
        assert_eq!(outcome, crate::store::StudioAdoptionOutcome::Installed);
    });
    p.alice.sync.with_registry_context(|g, d, _, _| {
        crate::store::fill_studio_epoch_fixture(&mut p.b_store, SERVER, g, d, target)
    });
    // A's exact key, kept so that the returning device is the same identity, not a new one.
    let a_key = p
        .alice
        .sync
        .with_registry_context(|_, d, _, _| d.duplicate().unwrap());
    let a_id = a_key.device_id();

    // --- A -> B. The staged Remove supplies a real observed transition; strict policy is restored
    // before any Studio actor runs (the same fixture `succession.rs` and `joining.rs` use).
    p.bob.sync.set_config(catcoms_sync::SyncConfig {
        max_committer_rank: 1,
        stage_decision_window_ms: 0,
        ..Default::default()
    });
    p.bob.sync.remove(&a_id).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.bob.sync.set_config(catcoms_sync::SyncConfig::default());
    assert!(p.bob.is_owner());
    let t1 = p.bob.sync.authoring_owner_tenure_start().unwrap();
    assert!(t1 > 0, "B's tenure starts at the removal, after A's");

    // Restart B (T2), then let ordinary idle passes close epoch 1 under B's tenure.
    let snapshot = p.bob.snapshot().unwrap();
    p.bob.sync.with_registry_context(|_, _, _, rng| {
        p.b_store.save_server(SERVER, &snapshot, rng).unwrap()
    });
    let Pair {
        b_root,
        b_store,
        clock,
        alice,
        bob,
        ..
    } = p;
    drop(alice);
    drop(bob);
    drop(b_store);
    let restored = Node::restore(
        &open(b_root.path()).load_server(SERVER).unwrap(),
        Net::new(Hub::new().join(PeerId::from_u64(22))),
        rng(),
        Box::new(clock.clone()),
        "B after the first owner change",
    )
    .unwrap();
    assert_eq!(restored.sync.authoring_owner_tenure_start(), Some(t1));
    let mut b_verifier = Node::restore(
        &snapshot,
        Net::new(Hub::new().join(PeerId::from_u64(99))),
        rng(),
        Box::new(clock.clone()),
        "read-only B verifier",
    )
    .unwrap();
    let store = Arc::new(Mutex::new(Some(open(b_root.path()))));
    let (actor, events, task) = crate::spawn(restored);
    let drained = drain_events(events);
    assert!(
        rotate_to(&actor, &store, &mut b_verifier, &clock, target, 2).await,
        "B's idle passes must close epoch 1 and install its successor"
    );
    let b_snapshot = actor.snapshot().await.unwrap();
    actor.shutdown().await;
    task.await.unwrap();
    drained.await.unwrap();
    let r1 = {
        let mut guard = store.lock().await;
        let held = guard.as_mut().unwrap();
        b_verifier.sync.with_registry_context(|_, _, _, rng| {
            held.save_server(SERVER, &b_snapshot, rng).unwrap()
        });
        let r1 = published(held, &b_verifier.group_id(), target);
        drop(guard.take());
        r1
    };
    // B's first receipt inherits A's checkpoint across the owner change (N-T1's inheritance).
    assert_eq!(r1.tenure_start_group_epoch, t1);
    assert_eq!(r1.closed_epoch, 1);
    assert_eq!(
        r1.inherited,
        InheritedCheckpoint::Checkpoint {
            epoch: 1,
            close_record_hash: r0.close_record_hash,
            seed_change_hash: r0.seed_change_hash,
        }
    );
    // The positive control for every later refusal of R1: under B's own tenure, while B owns, it
    // verifies as the current owner's receipt. A malformed R1 would fail here, not pass below.
    b_verifier.sync.with_registry_context(|g, _, _, _| {
        r1.verify_current_owner(g, t1)
            .expect("R1 is current while B owns");
    });

    // --- B -> A. A returns with its own key and takes the founder's freed leaf, so it owns again,
    // from its join epoch. B, restarted, is the witness that admits it.
    let hub = Hub::new();
    let mut witness = Node::restore(
        &open(b_root.path()).load_server(SERVER).unwrap(),
        Net::new(hub.join(PeerId::from_u64(22))),
        rng(),
        Box::new(clock.clone()),
        "B, witness to A's return",
    )
    .unwrap();
    let invite = witness.mint_invite([45; 16], u64::MAX, vec![]).unwrap();
    let (joined, tick) = tokio::join!(
        Node::join(
            Net::new(hub.join(PeerId::from_u64(3))),
            a_key,
            rng(),
            Box::new(clock.clone()),
            "A, returning with its own key",
            witness.local_peer(),
            &invite
        ),
        witness.sync_once()
    );
    tick.unwrap();
    // **Today's limitation, pinned rather than hidden.** The witness admits A's key, but the
    // returning device itself cannot process the Welcome: its MLS provider still holds its old
    // membership of this very group, under the same GroupId. Product joins always mint a fresh
    // device, so this never arises there, and a returning A is a new key whose old receipts fail
    // on the key alone. A device that keeps its key across a removal has no way back in until a
    // reviewed change lets it discard the stale group. When that lands, this assertion fails, and
    // the returning owner's own progress (its first receipt of the second tenure) belongs here.
    let Err(refused) = joined else {
        panic!("same-key re-entry needs a stale-group discard that does not exist yet");
    };
    assert!(
        refused.to_string().contains("already exists"),
        "the returning device must fail on its stale group, not on anything else: {refused}"
    );

    // The witness side is real and complete. It admitted A's key into the founder's freed leaf, so
    // A owns again, observed from the admission epoch.
    assert!(!witness.is_owner());
    witness.sync.with_registry_context(|g, _, _, _| {
        assert_eq!(
            g.designated_committer(),
            Some(a_id),
            "the same key owns again"
        )
    });
    let t2 = witness.sync.authoring_owner_tenure_start().unwrap();
    assert!(t2 > t1, "the witness observed a new tenure, after B's");
    assert_ne!(t2, r0.tenure_start_group_epoch, "A's two tenures differ");

    // --- The earlier tenure's same-key receipt. Under its OWN claimed start R0 still verifies: the
    // key is A's, A is the designated committer again, and that start is not in the future. That
    // is what "otherwise valid" means, and why a verifier that took a receipt's word for its
    // tenure would accept it. Checked against the start this witness observed, it is refused.
    // B's own R1 is no longer current either: its key is not the owner's.
    let refusals = |witness: &mut Node| {
        witness.sync.with_registry_context(|g, _, _, _| {
            assert!(
                r0.verify_current_owner(g, r0.tenure_start_group_epoch)
                    .is_ok(),
                "precondition: A's old receipt is otherwise valid now that A owns again"
            );
            assert!(
                r0.verify_current_owner(g, t2).is_err(),
                "A's earlier-tenure receipt is refused under the tenure the witness observed"
            );
            assert!(r1.verify_current_owner(g, t1).is_err());
            assert!(r1.verify_current_owner(g, t2).is_err());
        })
    };
    refusals(&mut witness);

    // --- The same refusal through a real consumer, not only the bare check (the review's M1).
    // Adoption is what turns a receipt into installed history, and it verifies with whatever tenure
    // its caller hands it. On an empty vault at the witness, handed the tenure R0 claims, it would
    // install A's old checkpoint. That is the positive oracle, and it shows the tenure is the only
    // difference. Handed the start the witness observed, it refuses and installs nothing.
    let adopt = |witness: &mut Node, tenure: u64| {
        let root = tempfile::tempdir().unwrap();
        let mut vault = open(root.path());
        let mut b = budget(witness, &mut vault);
        let outcome = witness.sync.with_registry_context(|g, d, _, rng| {
            vault
                .adopt_studio_checkpoint(
                    SERVER,
                    g,
                    target,
                    d,
                    &r0,
                    Some(seed.bytes()),
                    tenure,
                    &clock,
                    rng,
                    &mut b,
                )
                .map(|(outcome, _)| outcome)
        });
        let installed = witness.sync.with_registry_context(|g, d, _, _| {
            vault
                .load_studio_epoch(SERVER, g, target, d)
                .unwrap()
                .is_some()
        });
        (outcome, installed)
    };
    let (claimed, installed) = adopt(&mut witness, r0.tenure_start_group_epoch);
    assert!(
        matches!(claimed, Ok(crate::store::StudioAdoptionOutcome::Installed)) && installed,
        "precondition: a consumer that took R0's word for its tenure would install it: {claimed:?}"
    );
    let (observed, installed) = adopt(&mut witness, t2);
    assert!(
        observed.is_err() && !installed,
        "adoption under the observed start must refuse A's same-key receipt: {observed:?}"
    );

    // And across a restart of the witness (T2): the observed start survives, and so do the
    // refusals it decides.
    let witness_snapshot = witness.snapshot().unwrap();
    drop(witness);
    let mut witness = Node::restore(
        &witness_snapshot,
        Net::new(Hub::new().join(PeerId::from_u64(23))),
        rng(),
        Box::new(clock.clone()),
        "B, witness, restarted",
    )
    .unwrap();
    assert_eq!(witness.sync.authoring_owner_tenure_start(), Some(t2));
    refusals(&mut witness);
}
