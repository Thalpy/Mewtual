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
//! **What the first test cannot show:** the returning device's *own* side. A cannot process the
//! Welcome, because its MLS provider still holds the same group. The test pins that failure. The
//! second test shows the returning side anyway, by turning the roles round: B -> A -> B, where B
//! never leaves. B's first receipt of its second tenure is issued through the actor.
//!
//! **The newcomer (N-T2).** C joins during A's first tenure and reads `Unknown` there: neither its
//! Welcome nor any receipt names a start. It learns each later start only by observing the
//! transition, holds the same value as the owner, and refuses each earlier same-key receipt.
//!
//! **Why C joins that early.** A member can only join while the lowest leaf is occupied, or MLS
//! puts it there and it owns by its own join (9.6). And it cannot join during A's second tenure,
//! because only the owner admits, and A's device here cannot act.
use super::joining::{drain_events, step};
use super::*;
use catcoms_replication::{CheckpointSeed, Receipt};

/// Ordinary actor idle passes until `target`'s source is at `epoch`, Open, and the Registry
/// pointer names it. Returns whether that state was reached in the bounded window.
pub(super) async fn rotate_to(
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
pub(super) fn published(store: &ServerStore, group_id: &[u8], target: StudioTarget) -> Receipt {
    let logical = target.document(group_id).unwrap();
    let journal = store.load_epoch_owner_receipts(SERVER, &logical).unwrap();
    assert!(journal.pending().is_none());
    journal.published().expect("a published receipt").clone()
}

/// One ordinary tick to flush what the previous one queued. A tick drains its outbox (queued commit
/// broadcasts) at the top, then blocks on the next transport event, so the wait is cut short here.
/// A tick is cancel-safe by design: its network waits stay owned on the node.
pub(super) async fn flush(node: &mut Node) {
    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), node.sync_once()).await;
}

/// Ordinary sync ticks until `node` has processed every commit up to `epoch`. Bounded in ticks and
/// in time, because a tick blocks on the next transport event: a commit that never arrives must
/// fail the test by name instead of hanging it.
///
/// It ends with one more tick. A member at the contested setting applies a commit when its stage
/// resolves, and that tick returns early, so a removal's routing-label rotation is only noted. The
/// member subscribes to the new label's topics on its NEXT tick. Without that tick it would miss
/// every later commit, which is published on the new label.
pub(super) async fn catch_up(node: &mut Node, epoch: u64, who: &str) {
    for _ in 0..50 {
        if node.epoch() == epoch {
            break;
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), node.sync_once())
            .await
            .unwrap_or_else(|_| panic!("{who} never received the commit for epoch {epoch}"))
            .unwrap();
    }
    assert_eq!(node.epoch(), epoch, "{who} never processed the commit");
    flush(node).await;
}

/// The contested-commit setting the transitions use: a member at committer rank 1 may commit, with
/// no stage window. It is how B removes A here while A is still the designated committer. Every
/// member that must accept such a commit carries it too.
pub(super) fn contested() -> catcoms_sync::SyncConfig {
    catcoms_sync::SyncConfig {
        max_committer_rank: 1,
        stage_decision_window_ms: 0,
        ..Default::default()
    }
}

/// Where both tests stand once A's key has been admitted again: A owns its second tenure at the
/// witness, and everything earlier is real receipts and real membership changes.
struct Returned {
    /// B, restarted from its saved snapshot, which admitted A's key and is not the owner.
    witness: Node,
    /// C, which joined during A's first tenure and has observed every transition since. Still at
    /// the contested setting, so it can accept a later rank-1 commit.
    newcomer: Node,
    clock: ManualClock,
    /// B's vault. Holds R1 published and the Studio source at epoch 2, Open.
    b_root: tempfile::TempDir,
    target: StudioTarget,
    /// A's receipt, tenure 0, closing epoch 0.
    r0: Receipt,
    seed: CheckpointSeed,
    /// B's first receipt, tenure `t1`, closing epoch 1 and inheriting R0's checkpoint.
    r1: Receipt,
    /// B's tenure start, at A's removal.
    t1: u64,
    /// A's second tenure start, at its admission, as the witness observed it.
    t2: u64,
    a_id: crate::DeviceId,
}

/// A owns, B succeeds it through the actor and a restart, then A's same key is admitted again.
async fn a_returns_at_the_witness() -> Returned {
    // --- Tenure 0. A owns: it closes epoch 0 and installs epoch 1 under R0, signed by A's key. B,
    // a member while A is provably owner, adopts that checkpoint, and holds A's eligible history in
    // epoch 1, enough to reach the production rotation threshold.
    let mut p = Pair::new_legacy().await;
    let target = target();

    // --- C joins while A owns its first tenure, so it never saw any tenure begin (N-T2). It stays
    // for every later transition and observes each one.
    // B listens on the control topic first (the Pair fixture never subscribed it), or the Add that
    // admits C is published before anyone but A can hear it.
    p.bob.subscribe_control().await.unwrap();
    let invite = p.alice.mint_invite([44; 16], u64::MAX, vec![]).unwrap();
    let (joined, tick) = tokio::join!(
        Node::join(
            Net::new(p.hub.join(PeerId::from_u64(4))),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(p.clock.clone()),
            "C, a newcomer during A's first tenure",
            p.alice.local_peer(),
            &invite
        ),
        p.alice.sync_once()
    );
    tick.unwrap();
    let mut newcomer = joined.unwrap();
    newcomer.subscribe_control().await.unwrap();
    newcomer.sync.set_config(contested());
    // B must learn C's admission first, or A's removal would leave the two with different rosters.
    catch_up(&mut p.bob, p.alice.epoch(), "B, for C's admission").await;
    let welcome_epoch = newcomer.epoch();
    assert!(welcome_epoch > 0);
    assert!(!newcomer.is_owner());
    // Nothing C holds names a start: not its Welcome's epoch, not any receipt's claim. It reads
    // Unknown, has no verification value, and cannot author.
    assert_eq!(
        newcomer.observed_owner_tenure(),
        crate::studio::StudioOwnerTenure::Unknown,
        "a newcomer that is not the committer reads Unknown, not its Welcome's epoch"
    );
    assert_eq!(newcomer.sync.verification_owner_tenure_start(), None);
    assert!(
        newcomer.require_observed_owner_tenure().is_err(),
        "a newcomer that saw no tenure begin cannot author"
    );

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
    p.bob.sync.set_config(contested());
    p.bob.sync.remove(&a_id).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.bob.sync.set_config(catcoms_sync::SyncConfig::default());
    assert!(p.bob.is_owner());
    let t1 = p.bob.sync.authoring_owner_tenure_start().unwrap();
    assert!(t1 > 0, "B's tenure starts at the removal, after A's");
    // C observed the transition, so it now holds B's start, and the same value as B.
    catch_up(&mut newcomer, p.bob.epoch(), "C, for A's removal").await;
    assert_eq!(
        newcomer.observed_owner_tenure(),
        crate::studio::StudioOwnerTenure::Known(t1)
    );
    assert!(
        t1 > welcome_epoch,
        "learned by observation, not from the Welcome"
    );

    // Restart B (T2), then let ordinary idle passes close epoch 1 under B's tenure.
    let snapshot = p.bob.snapshot().unwrap();
    p.bob.sync.with_registry_context(|_, _, _, rng| {
        p.b_store.save_server(SERVER, &snapshot, rng).unwrap()
    });
    let Pair {
        hub,
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
    // from its join epoch. B, restarted, is the witness that admits it, on the hub C listens on.
    let mut witness = Node::restore(
        &open(b_root.path()).load_server(SERVER).unwrap(),
        Net::new(hub.join(PeerId::from_u64(22))),
        rng(),
        Box::new(clock.clone()),
        "B, witness to A's return",
    )
    .unwrap();
    witness.subscribe_control().await.unwrap();
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
    // C observed A's admission too, and agrees with the witness. The tick that admitted A queued
    // its commit broadcast; the next one sends it.
    flush(&mut witness).await;
    catch_up(&mut newcomer, witness.epoch(), "C, for A's admission").await;
    newcomer
        .sync
        .with_registry_context(|g, _, _, _| assert_eq!(g.designated_committer(), Some(a_id)));
    assert_eq!(
        newcomer.observed_owner_tenure(),
        crate::studio::StudioOwnerTenure::Known(t2)
    );
    Returned {
        witness,
        newcomer,
        clock,
        b_root,
        target,
        r0,
        seed,
        r1,
        t1,
        t2,
        a_id,
    }
}

#[tokio::test]
async fn studio_actor_a_to_b_to_a_progresses_under_new_tenure_and_refuses_the_returning_keys_old_one(
) {
    let Returned {
        mut witness,
        mut newcomer,
        clock,
        b_root: _b_root,
        target,
        r0,
        seed,
        r1,
        t1,
        t2,
        ..
    } = a_returns_at_the_witness().await;

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
    // And at C, the newcomer that joined during A's first tenure and read Unknown there (N-T2): the
    // replayed same-key receipt of that tenure is refused under the start C has since observed.
    refusals(&mut newcomer);

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
    // Named, not a broad `is_err()`: the refusal must be the receipt's authority check, the one
    // the tenure feeds, and not some other failure that merely depends on the tenure argument.
    assert!(
        matches!(
            &observed,
            Err(crate::AppError::Invalid(reason))
                if reason.contains("epoch-close signature or authority is invalid")
        ) && !installed,
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

/// B -> A -> B: the same-key return made by a device that works, observed by the newcomer.
///
/// The first test cannot show the returning owner's own side, because A's device cannot process
/// its Welcome. B never leaves, so B owning a second time is the same-key return made by a working
/// device. That device's first receipt of its second tenure is issued here, by ordinary actor idle
/// passes, and it inherits across A's tenure in between.
///
/// C, the newcomer from the shared prefix, read `Unknown` when it joined and has observed every
/// transition since. It learns B's second start the same way, holds exactly B's value, keeps it
/// across a restart, and refuses B's first-tenure receipt under it.
///
/// **A fixture lever, stated.** While A owns, A is the designated committer, and its device here
/// cannot commit (it never joined). B therefore removes A at committer rank 1 with no stage window,
/// the same contested-commit setting the first transition uses. It is restored before any Studio
/// actor runs.
#[tokio::test]
async fn studio_actor_b_to_a_to_b_issues_the_returning_keys_receipt_under_its_new_tenure_and_the_newcomer_learns_it_by_observation(
) {
    let Returned {
        mut witness,
        mut newcomer,
        clock,
        b_root,
        target,
        r0,
        r1,
        t1,
        t2,
        a_id,
        ..
    } = a_returns_at_the_witness().await;

    // --- A -> B again. B removes A, and is then the lowest occupied leaf, so it owns a second time.
    witness.sync.set_config(contested());
    witness.sync.remove(&a_id).await.unwrap();
    witness.sync_once().await.unwrap();
    witness.sync.set_config(catcoms_sync::SyncConfig::default());
    catch_up(&mut newcomer, witness.epoch(), "C, for A's second removal").await;
    newcomer
        .sync
        .set_config(catcoms_sync::SyncConfig::default());
    assert!(witness.is_owner(), "B owns again");
    let t3 = witness.sync.authoring_owner_tenure_start().unwrap();
    assert!(
        t3 > t2,
        "B's second tenure starts at A's removal, after A's second one"
    );
    assert_ne!(t3, t1, "B's two tenures differ");
    // C learned B's second start by observing the transition, and holds exactly B's value.
    assert_eq!(
        newcomer.observed_owner_tenure(),
        crate::studio::StudioOwnerTenure::Known(t3)
    );

    // --- B's first-tenure receipt is otherwise valid again: B's key, B the committer, a start
    // that is not in the future. Under the start each node observed it is refused, at B and at C.
    let refusals = |node: &mut Node| {
        node.sync.with_registry_context(|g, _, _, _| {
            assert!(
                r1.verify_current_owner(g, t1).is_ok(),
                "precondition: B's first-tenure receipt is otherwise valid now that B owns again"
            );
            assert!(
                r1.verify_current_owner(g, t3).is_err(),
                "B's first-tenure receipt is refused under its second tenure"
            );
            assert!(r0.verify_current_owner(g, t3).is_err());
        })
    };
    refusals(&mut witness);
    refusals(&mut newcomer);

    // --- Progress under the new authority, through the actor after a restart. B holds epoch 2
    // (opened by its own R1) and enough eligible history in it to reach the production threshold.
    let snapshot = witness.snapshot().unwrap();
    let mut held = open(b_root.path());
    witness.sync.with_registry_context(|g, d, _, rng| {
        held.save_server(SERVER, &snapshot, rng).unwrap();
        crate::store::fill_studio_epoch_fixture(&mut held, SERVER, g, d, target);
    });
    drop(witness);
    let restored = Node::restore(
        &held.load_server(SERVER).unwrap(),
        Net::new(Hub::new().join(PeerId::from_u64(22))),
        rng(),
        Box::new(clock.clone()),
        "B, owner again, restarted",
    )
    .unwrap();
    assert_eq!(restored.sync.authoring_owner_tenure_start(), Some(t3));
    let mut b_verifier = Node::restore(
        &snapshot,
        Net::new(Hub::new().join(PeerId::from_u64(98))),
        rng(),
        Box::new(clock.clone()),
        "read-only B verifier, second tenure",
    )
    .unwrap();
    let store = Arc::new(Mutex::new(Some(held)));
    let (actor, events, task) = crate::spawn(restored);
    let drained = drain_events(events);
    assert!(
        rotate_to(&actor, &store, &mut b_verifier, &clock, target, 3).await,
        "B's idle passes in its second tenure must close epoch 2 and install its successor"
    );
    let b_snapshot = actor.snapshot().await.unwrap();
    actor.shutdown().await;
    task.await.unwrap();
    drained.await.unwrap();
    let r2 = {
        let mut guard = store.lock().await;
        let held = guard.as_mut().unwrap();
        b_verifier.sync.with_registry_context(|_, _, _, rng| {
            held.save_server(SERVER, &b_snapshot, rng).unwrap()
        });
        let r2 = published(held, &b_verifier.group_id(), target);
        drop(guard.take());
        r2
    };
    // The returning key's first receipt of its second tenure: the same key as R1, the new start,
    // and inheriting the checkpoint R1 opened, across A's tenure in between.
    assert_eq!(r2.owner_public_key, r1.owner_public_key, "the same key");
    assert_eq!(r2.tenure_start_group_epoch, t3);
    assert_eq!(r2.closed_epoch, 2);
    assert_eq!(
        r2.inherited,
        InheritedCheckpoint::Checkpoint {
            epoch: 2,
            close_record_hash: r1.close_record_hash,
            seed_change_hash: r1.seed_change_hash,
        }
    );
    let current = |node: &mut Node| {
        node.sync.with_registry_context(|g, _, _, _| {
            r2.verify_current_owner(g, t3)
                .expect("R2 is the current owner's receipt under the observed start");
        })
    };
    current(&mut b_verifier);
    current(&mut newcomer);

    // And C across a restart: the observed start survives, and so does everything it decides.
    let newcomer_snapshot = newcomer.snapshot().unwrap();
    drop(newcomer);
    let mut newcomer = Node::restore(
        &newcomer_snapshot,
        Net::new(Hub::new().join(PeerId::from_u64(24))),
        rng(),
        Box::new(clock.clone()),
        "C, restarted",
    )
    .unwrap();
    assert_eq!(
        newcomer.observed_owner_tenure(),
        crate::studio::StudioOwnerTenure::Known(t3)
    );
    refusals(&mut newcomer);
    current(&mut newcomer);
}
