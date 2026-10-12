//! Hidden higher old-tenure history (design 9.2 T4; 17.1 N-T5), through the real discovery path.
//!
//! The rule under test: history above the new tenure's inherited checkpoint enters recovery as a
//! rewind, is preserved, and is never silently adopted.
//!
//! **The shape.** A owns. B and C both adopt A's first checkpoint (closing epoch 0). A then works on
//! in epoch 1, and C catches up to A's latest checkpoint (closing epoch 2), so C holds A's epoch 3
//! with that work in it. B never saw it. A is removed and B owns. B's first receipt, issued by
//! ordinary actor idle passes, closes B's epoch 1 and inherits the checkpoint B's tenure started
//! from, the one that opened epoch 1. Everything above that checkpoint is A's history hidden from B.
//! C then discovers B's receipt from B over the real head and seed wire, verifying it under the
//! tenure C observed. That is the path the receiver takes.
//!
//! **Why C is two closes ahead and not one.** C ends at epoch 3, while B's receipt closes epoch 1
//! and opens epoch 2. So a rule that ordered receipts by epoch alone would refuse B's receipt at C
//! as stale, and C would stay on A's history forever. The receipt book orders by tenure first, and
//! a new tenure's receipt advances whatever its epoch (`ReceiptBook::ingest_verified`).
//!
//! **What C must end with:**
//! - B's history as current, identical to what B holds;
//! - A's hidden work only in recovery, as a `Rewound` snapshot of the epoch it was in, still readable
//!   after C's vault is reopened;
//! - none of A's hidden work in C's current document, and A's latest receipt refused at C from
//!   then on, so the hidden history cannot come back as current either.
//!
//! **What runs where.** B's receipt is issued through the actor. C's side runs through the Server
//! discovery stages the receiver drives, over the real wire, but not through the receiver's own
//! scheduling loop.
//!
//! **Fixture levers, stated.** C adopts A's checkpoints directly through `adopt_studio_checkpoint`
//! with A's tenure handed in, as `repeated.rs` does for B. How C came to hold A's history is not
//! under test; what C does when the new owner's receipt arrives is. A's later receipt is signed by
//! hand from A's own projection, the way `prepared_checkpoint` signs the first. B's source is warmed
//! by the same detached preparation the actor runs before it serves a source past the on-actor read
//! bound. B
//! removes A at committer rank 1 with no stage window (`contested`), restored before any actor runs.
use super::joining::drain_events;
use super::repeated::{catch_up, contested, published, rotate_to};
use super::*;
use crate::studio_exchange::discovery::{ServerCheckpointDiscovery, ServerStudioCheckpointWatch};
use catcoms_replication::{CheckpointSeed, Receipt, RecoveryReason};
use catcoms_sync::checkpoint_exchange::CheckpointTarget;

/// The title A wrote after B last heard from it. It must end up only in C's recovery.
const HIDDEN: &str = "A's work above B's checkpoint";

/// A's receipt closing `closed`, signed by A's key under A's tenure (0), with a seed of A's current
/// projection at that epoch.
///
/// `inherited` names the checkpoint a *tenure* started from, not the one a receipt's epoch opened,
/// so every receipt of A's tenure repeats `first`'s. A receipt of the same tenure that named another
/// is equivocation, and faults the document (`receipts_conflict`).
fn a_receipt(
    p: &mut Pair,
    target: StudioTarget,
    first: &Receipt,
    closed: u64,
    close: [u8; 32],
) -> (Receipt, CheckpointSeed) {
    p.alice.sync.with_registry_context(|g, d, _, _| {
        let source = p
            .a_store
            .load_studio_epoch(SERVER, g, target, d)
            .unwrap()
            .unwrap();
        let mut projection = source.projection().unwrap();
        match &mut projection {
            StudioProjection::Flipnote(art) => art.epoch = closed,
            StudioProjection::Index(index) => index.epoch = closed,
        };
        let seed = projection.checkpoint(close).unwrap();
        let receipt = Receipt::sign(
            target.document(&g.group_id()).unwrap(),
            closed,
            close,
            seed.change_hash(),
            0,
            first.inherited.clone(),
            d,
        )
        .unwrap();
        (receipt, seed)
    })
}

/// `node` installs `receipt`'s checkpoint into `store`, verified under `tenure`.
fn adopt(
    node: &mut Node,
    store: &mut ServerStore,
    target: StudioTarget,
    receipt: &Receipt,
    seed: &CheckpointSeed,
    tenure: u64,
    clock: &ManualClock,
) {
    let mut b = budget(node, store);
    node.sync.with_registry_context(|g, d, _, rng| {
        let (outcome, _) = store
            .adopt_studio_checkpoint(
                SERVER,
                g,
                target,
                d,
                receipt,
                Some(seed.bytes()),
                tenure,
                clock,
                rng,
                &mut b,
            )
            .unwrap();
        assert_eq!(outcome, crate::store::StudioAdoptionOutcome::Installed);
    });
}

fn installed(node: &mut Node, store: &ServerStore, target: StudioTarget) -> EpochStudioState {
    node.sync.with_registry_context(|g, d, _, _| {
        store
            .load_studio_epoch(SERVER, g, target, d)
            .unwrap()
            .expect("an installed source")
    })
}

fn title_of(state: &EpochStudioState) -> String {
    let StudioProjection::Flipnote(art) = state.projection().unwrap() else {
        panic!("the fixture's target is a Flipnote")
    };
    art.title.expect("a title").selected.value
}

/// The owner's current head, discovered by `member` from `owner` over the real wire, then its seed
/// fetched and installed. Returns the outcome of each install step.
async fn discover_and_install(
    member: &mut Node,
    member_store: &mut ServerStore,
    owner: &mut Node,
    owner_store: &mut ServerStore,
    watch: &ServerStudioCheckpointWatch,
    target: StudioTarget,
) -> (
    crate::store::StudioAdoptionOutcome,
    crate::store::StudioAdoptionOutcome,
    EpochStudioState,
) {
    let snapshot = owner
        .prepare_owner_head_snapshot(owner_store, SERVER)
        .unwrap();
    let mut b = budget(owner, owner_store);
    let attempt = member
        .prepare_checkpoint_discovery(
            member_store,
            SERVER,
            owner.local_peer(),
            CheckpointTarget::Studio(target),
        )
        .unwrap();
    let (completed, ()) = tokio::join!(attempt.fetch(), async {
        while owner
            .serve_studio_head_step(owner_store, watch, Some(&snapshot), &mut b)
            .unwrap()
            .is_none()
        {
            owner.sync_once().await.unwrap();
        }
    });
    let Some(ServerCheckpointDiscovery::Selected(mut pass)) = member
        .complete_checkpoint_discovery(member_store, SERVER, completed)
        .unwrap()
    else {
        panic!("the member must select the current owner's head")
    };
    let mut b = budget(member, member_store);
    let (sealed, _) = member
        .install_studio_seed_step(member_store, SERVER, &pass, &mut b)
        .unwrap();
    let mut b = budget(owner, owner_store);
    let pending = member
        .prepare_checkpoint_seed_fetch(&mut pass, owner.local_peer())
        .unwrap()
        .expect("a seed to fetch");
    let (completed, ()) = tokio::join!(pending.fetch(), async {
        while owner
            .serve_studio_seed_step(owner_store, watch, &mut b)
            .unwrap()
            .is_none()
        {
            owner.sync_once().await.unwrap();
        }
    });
    assert!(member
        .complete_checkpoint_seed_fetch(&mut pass, completed)
        .unwrap());
    let mut b = budget(member, member_store);
    let (outcome, state) = member
        .install_studio_seed_step(member_store, SERVER, &pass, &mut b)
        .unwrap();
    (sealed, outcome, state)
}

#[tokio::test]
async fn studio_discovery_rewinds_hidden_old_tenure_history_into_recovery_never_adopting_it() {
    let mut p = Pair::new_legacy().await;
    let target = target();
    let logical = target.document(&p.alice.group_id()).unwrap();

    // --- C joins while A owns, and B learns of it, so A's later removal leaves one roster.
    p.bob.subscribe_control().await.unwrap();
    let invite = p.alice.mint_invite([46; 16], u64::MAX, vec![]).unwrap();
    let (joined, tick) = tokio::join!(
        Node::join(
            Net::new(p.hub.join(PeerId::from_u64(4))),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(p.clock.clone()),
            "C, which holds A's whole history",
            p.alice.local_peer(),
            &invite
        ),
        p.alice.sync_once()
    );
    tick.unwrap();
    let mut c = joined.unwrap();
    c.subscribe_control().await.unwrap();
    c.sync.set_config(contested());
    catch_up(&mut p.bob, p.alice.epoch(), "B, for C's admission").await;
    let c_root = tempfile::tempdir().unwrap();
    let mut c_store = open(c_root.path());

    // --- A's tenure (0). R0 closes epoch 0. B and C both adopt it.
    let (r0, seed0, epoch_one) = super::super::discovery::prepared_checkpoint(&mut p, target);
    let clock = p.clock.clone();
    adopt(&mut p.bob, &mut p.b_store, target, &r0, &seed0, 0, &clock);
    adopt(&mut c, &mut c_store, target, &r0, &seed0, 0, &clock);
    // B holds enough eligible history in epoch 1 to reach the production rotation threshold.
    p.alice.sync.with_registry_context(|g, d, _, _| {
        crate::store::fill_studio_epoch_fixture(&mut p.b_store, SERVER, g, d, target)
    });

    // A's hidden work, in its epoch 1, and A's next two closes. Only C adopts them.
    let hidden = title(2, HIDDEN);
    p.alice
        .studio_transaction(
            &mut p.a_store,
            SERVER,
            StudioRequest::Apply {
                target,
                epoch_id: epoch_one,
                nonce: hidden.nonce,
                body: hidden.body,
            },
        )
        .unwrap();
    // C catches up straight to A's latest checkpoint, as discovery would: a member selects the
    // current head, not every receipt on the way. (One adoption per close would also work, but
    // each retains a version, and a full journal makes the later adoption wait on an eviction
    // warning first. That is its own path, and not this test's.)
    let (r0_2, seed2) = a_receipt(&mut p, target, &r0, 2, [10; 32]);
    adopt(&mut c, &mut c_store, target, &r0_2, &seed2, 0, &clock);
    let before = installed(&mut c, &c_store, target);
    assert_eq!(before.epoch(), 3, "precondition: C holds A's epoch 3");
    assert_eq!(
        title_of(&before),
        HIDDEN,
        "precondition: and A's hidden work"
    );
    assert_ne!(
        title_of(&installed(&mut p.bob, &p.b_store, target)),
        HIDDEN,
        "precondition: B never saw A's later work"
    );
    let retained_before = c_store
        .load_epoch_recovery(SERVER, &logical)
        .unwrap()
        .retained()
        .count();

    // --- A -> B. B removes A and owns from the removal; C observes the transition.
    let a_id = p
        .alice
        .sync
        .with_registry_context(|_, d, _, _| d.device_id());
    p.bob.sync.set_config(contested());
    p.bob.sync.remove(&a_id).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.bob.sync.set_config(catcoms_sync::SyncConfig::default());
    assert!(p.bob.is_owner());
    let t1 = p.bob.sync.authoring_owner_tenure_start().unwrap();
    catch_up(&mut c, p.bob.epoch(), "C, for A's removal").await;
    c.sync.set_config(catcoms_sync::SyncConfig::default());
    assert_eq!(
        c.observed_owner_tenure(),
        crate::studio::StudioOwnerTenure::Known(t1)
    );

    // B's first receipt, by ordinary actor idle passes after a restart. It closes B's epoch 1 and
    // inherits the checkpoint that opened it: R0's. Everything of A's above that is hidden from B.
    let snapshot = p.bob.snapshot().unwrap();
    p.bob.sync.with_registry_context(|_, _, _, rng| {
        p.b_store.save_server(SERVER, &snapshot, rng).unwrap()
    });
    let Pair {
        hub,
        b_root,
        b_store,
        alice,
        bob,
        ..
    } = p;
    drop((alice, bob, b_store));
    let restored = Node::restore(
        &open(b_root.path()).load_server(SERVER).unwrap(),
        Net::new(Hub::new().join(PeerId::from_u64(22))),
        rng(),
        Box::new(clock.clone()),
        "B, the new owner",
    )
    .unwrap();
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
    // The case an epoch-only order gets wrong: C's latest receipt closed a higher epoch than R1.
    assert!(r0_2.closed_epoch > r1.closed_epoch);

    // --- C discovers B's head from B, over the real wire, and installs it.
    let mut owner = Node::restore(
        &open(b_root.path()).load_server(SERVER).unwrap(),
        Net::new(hub.join(PeerId::from_u64(22))),
        rng(),
        Box::new(clock.clone()),
        "B, serving its head",
    )
    .unwrap();
    let mut owner_store = open(b_root.path());
    // B's source is past the on-actor read bound (the fill fixture took it over the rotation
    // threshold), so it is served warm, after the detached preparation the actor would run.
    let capture = owner
        .sync
        .with_registry_context(|g, d, _, _| owner_store.capture_studio_source(SERVER, g, target, d))
        .unwrap()
        .expect("B's installed source");
    let prepared = capture.rebuild().unwrap();
    assert!(owner
        .sync
        .with_registry_context(
            |g, d, _, _| owner_store.install_prepared_studio_source(g, d, prepared)
        )
        .unwrap());
    owner.subscribe_control().await.unwrap();
    // C binds the restarted owner's new peer to B's device the way a member does: through the
    // authenticated directory catch-up the desktop join command uses. Discovery asks only a bound
    // peer for a head.
    let (bound, tick) = tokio::join!(
        c.request_channel_index_catchup(owner.local_peer()),
        owner.sync_once()
    );
    bound.unwrap();
    tick.unwrap();
    let watch = owner
        .watch_studio_checkpoint(&owner_store, SERVER, target)
        .unwrap();
    let (sealed, outcome, after) = discover_and_install(
        &mut c,
        &mut c_store,
        &mut owner,
        &mut owner_store,
        &watch,
        target,
    )
    .await;
    assert_eq!(
        sealed,
        crate::store::StudioAdoptionOutcome::AwaitingSeed,
        "a new tenure's receipt advances C whatever its epoch; it is not refused as stale"
    );
    assert_eq!(outcome, crate::store::StudioAdoptionOutcome::Installed);

    // B's history is current at C, and it is exactly what B holds.
    let b_current = installed(&mut owner, &owner_store, target);
    assert_eq!(after.epoch(), 2);
    assert_eq!(
        after.doc_id(),
        catcoms_replication::epoch::epoch_id(
            logical.doc_type,
            &logical.logical_key,
            2,
            &r1.close_record_hash
        ),
        "C's current document is the one R1 opened"
    );
    assert_eq!(
        after.projection().unwrap(),
        b_current.projection().unwrap(),
        "C converges on the new owner's history"
    );
    assert_ne!(
        title_of(&after),
        HIDDEN,
        "A's hidden work is never silently adopted as current"
    );

    // A's hidden work is preserved, in recovery, as a rewind of the epoch it was in.
    let check_preserved = |c: &mut Node, c_store: &ServerStore| {
        let recovery = c_store.load_epoch_recovery(SERVER, &logical).unwrap();
        assert_eq!(
            recovery.retained().count(),
            retained_before + 1,
            "the adoption retained exactly one more version"
        );
        let rewound = recovery
            .retained()
            .find(|s| s.epoch == 3)
            .expect("A's epoch 3 is retained");
        assert_eq!(rewound.reason, RecoveryReason::Rewound);
        let recovered = StudioRecovery::from_snapshot(rewound, &logical, channel()).unwrap();
        let StudioProjection::Flipnote(art) = recovered.projection() else {
            panic!("a Flipnote recovery")
        };
        assert_eq!(
            art.title
                .as_ref()
                .expect("a recovered title")
                .selected
                .value,
            HIDDEN,
            "A's hidden work is preserved in recovery"
        );
        assert_ne!(title_of(&installed(c, c_store, target)), HIDDEN);
    };
    check_preserved(&mut c, &c_store);

    // And across a reopen of C's vault. The node and its sync are kept, so this speaks for what
    // the vault persisted.
    drop(c_store);
    let mut c_store = open(c_root.path());
    check_preserved(&mut c, &c_store);

    // A's hidden history stays out for good, not only now. A's latest receipt, which C once held
    // as current, is not the current owner's under the tenure C observed, so a later adoption of
    // it is refused by the receipt-authority check and installs nothing.
    let mut b = budget(&mut c, &mut c_store);
    let refused = c.sync.with_registry_context(|g, d, _, rng| {
        c_store
            .adopt_studio_checkpoint(
                SERVER,
                g,
                target,
                d,
                &r0_2,
                Some(seed2.bytes()),
                t1,
                &clock,
                rng,
                &mut b,
            )
            .map(|(outcome, _)| outcome)
    });
    assert!(
        matches!(
            &refused,
            Err(crate::AppError::Invalid(reason))
                if reason.contains("epoch-close signature or authority is invalid")
        ),
        "A's earlier-tenure receipt is refused at C once B owns: {refused:?}"
    );
    assert_eq!(
        installed(&mut c, &c_store, target).doc_id(),
        after.doc_id(),
        "and C's current document is still the one R1 opened"
    );
}
