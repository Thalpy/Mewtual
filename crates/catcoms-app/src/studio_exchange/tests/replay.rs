use super::*;
use crate::studio::StudioReceiver;
use catcoms_replication::{EpochPhase, InheritedCheckpoint, Receipt};
use catcoms_rt::Clock;

fn install_empty(p: &mut Pair) -> (Receipt, u128) {
    let (receipt, seed) = p.alice.sync.with_registry_context(|g, d, _, _| {
        let empty =
            catcoms_replication::studio::StudioEpoch::new(g, target(), d.device_id()).unwrap();
        let seed = empty.projection().unwrap().checkpoint([81; 32]).unwrap();
        let receipt = Receipt::sign(
            target().document(&g.group_id()).unwrap(),
            0,
            [81; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap();
        (receipt, seed)
    });
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let epoch = p.bob.sync.with_registry_context(|g, d, _, rng| {
        let (_, state) = p
            .b_store
            .adopt_studio_checkpoint(
                SERVER,
                g,
                target(),
                d,
                &receipt,
                Some(seed.bytes()),
                0,
                &p.clock,
                rng,
                &mut b,
            )
            .unwrap();
        let epoch = state.doc_id();
        p.b_store.retain_studio_source(g, d, state);
        epoch
    });
    (receipt, epoch)
}
fn own_title(p: &mut Pair, epoch: u128, n: u8, text: &str) {
    p.bob
        .studio_transaction(
            &mut p.b_store,
            SERVER,
            StudioRequest::Apply {
                target: target(),
                epoch_id: epoch,
                nonce: [n; 16],
                body: title(n, text).body,
            },
        )
        .unwrap();
}
fn count(p: &Pair) -> usize {
    p.b_store
        .load_epoch_intents(SERVER, &target().document(&p.bob.group_id()).unwrap())
        .unwrap()
        .pending()
        .len()
}
fn watch(p: &mut Pair) -> StudioReceiver {
    let mut receiver = StudioReceiver::default();
    // A private pool: the manual move now holds a preparation permit while a record is out for
    // validation, and on the process-wide four-slot pool that would contend with every other
    // test in the process that prepares a source.
    receiver.inject_overlay_pool_for_test(4);
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target: target() }),
        )
        .unwrap();
    receiver
}
/// Background visits until this receiver completes one more replay pass, driving each detached
/// job the way the actor does. Returns how many inventory validations were detached on the way.
///
/// C-3 step 2 made the manual move take its budget from the shared, resumable inventory job
/// instead of one synchronous whole-vault scan, so a pass whose vault holds records the
/// validation cache has not seen (every record, after a restart) now finishes over several turns
/// with a detached validation between them. That is an intentional contract change: what the pass
/// ends with is unchanged, but "one `run`" no longer means "the manual move happened".
async fn settle_replay(p: &mut Pair, receiver: &mut StudioReceiver) -> usize {
    settle_replay_with(p, receiver, |_, _| {}).await
}
/// [`settle_replay`] with `between` run before every visit, outside it, as another actor's
/// writes land between this actor's visits.
async fn settle_replay_with(
    p: &mut Pair,
    receiver: &mut StudioReceiver,
    between: fn(&mut Pair, u8),
) -> usize {
    let before = receiver.replay_state_for_test().1;
    let mut validations = 0;
    for visit in 0..64u8 {
        between(p, visit);
        p.clock.advance_ms(1000);
        receiver
            .run(&mut p.bob, &mut p.b_store, SERVER, None)
            .unwrap();
        // Every visit, including one whose replay turn wrote, ends by marking the token as it
        // left it; without the mark every restart looks foreign and N-M1 never applies.
        assert!(
            receiver.inventory_marked_for_test(&p.b_store),
            "a visit ended without its inventory mark"
        );
        if let Some(job) = receiver.detach(&mut p.bob) {
            let inventory = job.kind_for_test() == "inventory-validate";
            let before = receiver.pending(&p.bob);
            receiver.complete(&mut p.bob, job.run(None).await);
            if inventory {
                validations += 1;
                // The result waits for replay's next paced turn, a second away. Arriving must not
                // make it driver work in the meantime, or the driver would spin until that turn.
                assert_eq!(
                    receiver.pending(&p.bob),
                    before,
                    "a validation result held the driver awake before its owner's turn"
                );
            }
        }
        let (active, completed) = receiver.replay_state_for_test();
        if !active && completed > before {
            return validations;
        }
    }
    panic!("the replay pass never completed");
}
/// The common start for the manual-move tests: two own titles, and B replayed over an empty
/// install. After the caller restarts the store (`drop` first: the vault is locked), the next pass
/// has exactly one manual move to make, superseded A, which leaves `count` at 1, and the reopened
/// store has validated nothing.
fn stage_manual_move(p: &mut Pair) {
    let doc = target().document(&p.bob.group_id()).unwrap();
    let initial = epoch_zero_id(doc.doc_type, &doc.logical_key);
    own_title(p, initial, 91, "A");
    own_title(p, initial, 92, "B");
    install_empty(p);
    let mut receiver = watch(p);
    receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .unwrap();
    assert_eq!(count(p), 2, "B replayed, nothing moved yet");
}
/// Another actor's five-family write, between this actor's visits, reduced to what it does to a
/// live cursor: the token rotates. A real Apply here would also schedule catch-up and network
/// work on this server, which changes what the receiver's turns do and so tests something else.
fn foreign_write(p: &mut Pair, _: u8) {
    p.b_store.overtake_inventory_for_test();
}

#[tokio::test]
async fn studio_replay_nonowner_restarts_replays_selected_own_title_and_archives_only_old_evidence()
{
    let mut p = Pair::new().await;
    let doc = target().document(&p.bob.group_id()).unwrap();
    let initial = epoch_zero_id(doc.doc_type, &doc.logical_key);
    own_title(&mut p, initial, 91, "A");
    own_title(&mut p, initial, 92, "B");
    let (_, epoch) = install_empty(&mut p);
    assert_eq!(count(&p), 2);
    let mut receiver = watch(&mut p);
    let (_saved, updated) = receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .unwrap();
    assert_eq!(updated, Some(target()));
    let StudioProjection::Flipnote(art) = p.state().unwrap().projection().unwrap() else {
        panic!()
    };
    assert_eq!(art.title.unwrap().selected.value, "B");
    assert_eq!(count(&p), 2);
    // Restart after replay Save but before publication: B is a current signed-log operation,
    // not a new overwrite. Only superseded A moves to durably flushed manual recovery.
    drop(receiver);
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    // Every validation detaches, so the manual move's budget can only come from a job that
    // parked and detached at least one record: the turn-based path, not a scan. Without the
    // switch, the calibrated classifier validates this vault's small records inline and the
    // receiver's restore warms the Studio source, so the move needs no detach at all, and a
    // detach count could not tell the job from a synchronous scan (C-3 runtime 14.6).
    p.b_store.detach_every_validation_for_test();
    let mut receiver = watch(&mut p);
    assert!(
        settle_replay(&mut p, &mut receiver).await > 0,
        "the manual move did not take its budget from the shared inventory job"
    );
    assert_eq!(count(&p), 1);
    assert_eq!(p.state().unwrap().op_count(), 1);
    assert!(
        p.b_store
            .load_epoch_recovery(SERVER, &doc)
            .unwrap()
            .retained()
            .len()
            > 0
    );
    assert_eq!(p.state().unwrap().doc_id(), epoch);
    assert!(!receiver.replay_state_for_test().0);
    // The next owner checkpoint includes B. A non-owner has the seed but not the closure:
    // its exact old envelope is preserved in adoption recovery, so it leaves pending under
    // the manual policy, NOT an invented seed-only receipt proof. Ordinary cycles do not leak.
    let projection = p.state().unwrap().projection().unwrap();
    let seed = projection.checkpoint([82; 32]).unwrap();
    let receipt = p.alice.sync.with_registry_context(|g, d, _, _| {
        Receipt::sign(
            target().document(&g.group_id()).unwrap(),
            1,
            [82; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap()
    });
    let mut b = budget(&mut p.bob, &mut p.b_store);
    p.bob
        .sync
        .with_registry_context(|g, d, _, rng| {
            p.b_store.adopt_studio_checkpoint(
                SERVER,
                g,
                target(),
                d,
                &receipt,
                Some(seed.bytes()),
                0,
                &p.clock,
                rng,
                &mut b,
            )
        })
        .unwrap();
    let mut receiver = watch(&mut p);
    settle_replay(&mut p, &mut receiver).await;
    assert_eq!(count(&p), 0);
    assert_eq!(p.state().unwrap().epoch(), 2);
    assert_eq!(
        p.b_store
            .load_epoch_recovery(SERVER, &doc)
            .unwrap()
            .retained()
            .len(),
        2
    );
}

/// Review of step 2, HIGH-1. A manual move that meets a store-wide inventory backoff is not held
/// by it: its turn mints from one synchronous receive-profile scan, as the site did before step 2,
/// with nothing detached and the runtime's backoff left in place for its other owners.
#[tokio::test]
async fn studio_replay_manual_move_inside_an_inventory_backoff_scans_synchronously() {
    let mut p = Pair::new().await;
    stage_manual_move(&mut p);
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    // Switched, so a job that did step inside its backoff would park and detach, and the count
    // below would catch it. Without the switch this vault inlines everything, and zero detaches
    // could not tell a job that never stepped from one that stepped (C-3 runtime 14.6).
    p.b_store.detach_every_validation_for_test();
    let mut receiver = watch(&mut p);
    receiver.inventory_back_off_for_test(p.clock.monotonic_ms());
    assert_eq!(
        settle_replay(&mut p, &mut receiver).await,
        0,
        "the shared job stepped inside its backoff"
    );
    assert_eq!(count(&p), 1, "the superseded A did not move");
    assert_eq!(receiver.inventory_state_for_test(), "backoff");
}

/// HIGH-1, the patience bound. A job that cannot progress (here no preparation permit is ever
/// free, so it never steps) holds the move for `MANUAL_MOVE_PATIENCE_MS` and no longer.
#[tokio::test]
async fn studio_replay_manual_move_falls_back_once_its_patience_is_spent() {
    let mut p = Pair::new().await;
    stage_manual_move(&mut p);
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    let mut receiver = watch(&mut p);
    let patience = StudioReceiver::manual_move_patience_for_test();
    receiver.inject_overlay_pool_for_test(0);
    let mut waited = None;
    for _ in 0..(patience / 1000 + 8) {
        p.clock.advance_ms(1000);
        receiver
            .run(&mut p.bob, &mut p.b_store, SERVER, None)
            .unwrap();
        if let Some(job) = receiver.detach(&mut p.bob) {
            receiver.complete(&mut p.bob, job.run(None).await);
        }
        let now = p.clock.monotonic_ms();
        if receiver.replay_state_for_test().0 && count(&p) == 2 {
            waited.get_or_insert(now);
            continue;
        }
        let since = waited.expect("the move never waited on the job");
        assert!(
            now - since >= patience,
            "fell back after {} ms, before its patience",
            now - since
        );
        assert_eq!(count(&p), 1);
        return;
    }
    panic!("the move outlived its patience");
}

/// HIGH-1, the defect itself. Another actor writes between every replay turn, so the job never
/// sees two quiet gaps in a row. Before the fallback the move never completed, and its pass held
/// replay for every other target meanwhile; now it completes within a bounded number of visits.
#[tokio::test]
async fn studio_replay_manual_move_completes_with_a_write_between_every_visit() {
    let mut p = Pair::new().await;
    stage_manual_move(&mut p);
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    let mut receiver = watch(&mut p);
    settle_replay_with(&mut p, &mut receiver, foreign_write).await;
    assert_eq!(count(&p), 1, "the superseded A did not move");
}

/// C-3 runtime design 14.2 at the receiver: on a small vault the ordinary manual move needs no
/// detach at all. Its uncached records validate inline under the classifier, and the receiver's
/// restore leaves the Studio source warm, so the shared job finishes in the turn that starts it.
/// The switched tests in this module cover the detached path; this pins that the ordinary one
/// no longer needs it.
#[tokio::test]
async fn studio_replay_manual_move_on_a_small_vault_needs_no_detach() {
    let mut p = Pair::new().await;
    stage_manual_move(&mut p);
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    let mut receiver = watch(&mut p);
    assert_eq!(
        settle_replay(&mut p, &mut receiver).await,
        0,
        "the manual move detached a record the classifier should have validated inline"
    );
    assert_eq!(count(&p), 1, "the superseded A did not move");
    assert_ne!(
        receiver.inventory_state_for_test(),
        "backoff",
        "the move completed by falling back, not through the shared job"
    );
}

/// The runtime's receiver-level wiring (review of step 2, MEDIUM-1), each against the state the
/// production `run` leaves: a parked body is pending work; it detaches even while a catch-up
/// network pass is in flight; a cancelled validation is routed to the runtime, not to catch-up's
/// default arm; and pause and the lock reset release a parked body and its permit.
#[tokio::test]
async fn studio_replay_inventory_lifecycle_at_the_receiver() {
    let mut p = Pair::new().await;
    stage_manual_move(&mut p);
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    // This test's subject is a parked body's lifecycle, and this vault's records would otherwise
    // validate inline or be warm, so nothing would park (C-3 runtime 14.6).
    p.b_store.detach_every_validation_for_test();
    let mut receiver = watch(&mut p);
    let pool = receiver.inject_overlay_pool_for_test(4);
    // Visit until the manual move's job has parked a body, detaching nothing else on the way.
    let park = |receiver: &mut StudioReceiver, p: &mut Pair| {
        for _ in 0..16 {
            p.clock.advance_ms(1000);
            receiver
                .run(&mut p.bob, &mut p.b_store, SERVER, None)
                .unwrap();
            if receiver.inventory_state_for_test() == "parked" {
                return;
            }
        }
        panic!("the job never parked");
    };

    park(&mut receiver, &mut p);
    assert_eq!(
        pool.available_permits(),
        3,
        "the parked body holds its permit"
    );
    assert!(
        receiver.pending(&p.bob),
        "a parked body was not pending work"
    );
    receiver.hold_catchup_in_flight_for_test(true);
    let job = receiver
        .detach(&mut p.bob)
        .expect("a parked body waited behind a network pass in flight");
    assert_eq!(job.kind_for_test(), "inventory-validate");
    // Catch-up's network pass stays "in flight" through the cancelled validation's completion:
    // routed to the default arm, that completion would clear catch-up's pass state (H3).
    let (cancel, signal) = tokio::sync::watch::channel(false);
    cancel.send_replace(true);
    let result = job
        .run(Some(catcoms_rt::RequestCancellation::new(signal, None)))
        .await;
    receiver.complete(&mut p.bob, result);
    assert!(
        receiver.catchup_in_flight_for_test(),
        "a cancelled validation tore down catch-up's network pass"
    );
    receiver.hold_catchup_in_flight_for_test(false);
    assert_eq!(
        receiver.inventory_state_for_test(),
        "idle",
        "a cancelled validation was not routed to the inventory runtime"
    );
    assert_eq!(
        pool.available_permits(),
        4,
        "the cancelled worker kept its permit"
    );

    park(&mut receiver, &mut p);
    receiver.pause_at_for_test(&p.bob);
    assert_eq!(
        receiver.inventory_state_for_test(),
        "idle",
        "pause kept the job"
    );
    assert_eq!(pool.available_permits(), 4, "pause kept the parked permit");
    drop(receiver);

    let mut receiver = watch(&mut p);
    let pool = receiver.inject_overlay_pool_for_test(4);
    park(&mut receiver, &mut p);
    receiver.clear_previews();
    assert_eq!(
        receiver.inventory_state_for_test(),
        "idle",
        "the lock reset kept the job"
    );
    assert_eq!(
        pool.available_permits(),
        4,
        "the lock reset kept the parked permit"
    );
}

#[tokio::test]
async fn studio_replay_sealed_active_pass_clears_and_stays_bounded() {
    let mut p = Pair::new().await;
    let doc = target().document(&p.bob.group_id()).unwrap();
    let initial = epoch_zero_id(doc.doc_type, &doc.logical_key);
    own_title(&mut p, initial, 91, "A");
    own_title(&mut p, initial, 92, "B");
    let (_, epoch) = install_empty(&mut p);
    let mut receiver = watch(&mut p);
    receiver
        .replay_step_for_test(&mut p.bob, &mut p.b_store, SERVER)
        .unwrap();
    assert!(receiver.replay_state_for_test().0);
    // Pending advertises actionable replay, not an unvisited watch or a not-yet-due pass.
    // Native idle inspection still runs every five seconds when no packet has arrived.
    assert!(!receiver.pending(&p.bob));
    p.clock.advance_ms(999);
    assert!(!receiver.pending(&p.bob));
    p.clock.advance_ms(1);
    assert!(receiver.pending(&p.bob));
    let receipt = p.alice.sync.with_registry_context(|g, d, _, _| {
        Receipt::sign(
            target().document(&g.group_id()).unwrap(),
            1,
            [82; 32],
            [82; 32],
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap()
    });
    let mut b = budget(&mut p.bob, &mut p.b_store);
    p.bob
        .sync
        .with_registry_context(|g, d, _, rng| {
            p.b_store
                .seal_studio_epoch(SERVER, g, target(), d, receipt, 0, rng, &mut b)
        })
        .unwrap();
    for _ in 0..100 {
        p.clock.advance_ms(1000);
        receiver
            .replay_step_for_test(&mut p.bob, &mut p.b_store, SERVER)
            .unwrap();
        assert_eq!(receiver.replay_state_for_test(), (false, 1));
    }
    assert_eq!(p.state().unwrap().phase(), EpochPhase::Closing);
    assert_eq!(p.state().unwrap().doc_id(), epoch);
    assert_eq!(count(&p), 2, "seal does not authorize ledger removal");
}
