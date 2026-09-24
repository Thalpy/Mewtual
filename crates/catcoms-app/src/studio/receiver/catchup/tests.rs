use super::*;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[tokio::test]
async fn studio_owner_superseded_preparation_waits_its_local_deadline() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(181);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"pace", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::save_studio_source_fixture(&mut store, 83, g, d, target)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            83,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    // Displace the sole graph without adding another watch. The owner turn must capture A.
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::save_studio_source_fixture(
            &mut store,
            83,
            g,
            d,
            StudioTarget::Flipnote {
                channel: target.channel(),
                object: [8; 16],
            },
        )
    });
    server
        .studio_transaction(
            &mut store,
            83,
            StudioRequest::Read {
                target: StudioTarget::Flipnote {
                    channel: target.channel(),
                    object: [8; 16],
                },
            },
        )
        .unwrap();
    receiver.run(&mut server, &mut store, 83, None).unwrap();
    let work = receiver.detach(&mut server).expect("owner source capture");
    assert!(work.is_preparation_for_test());
    let result = work.run(None).await;
    // Valid local/remote edits can win while the worker owns an older detached snapshot.
    let logical = target.document(&server.group_id()).unwrap();
    server
        .studio_transaction(
            &mut store,
            83,
            StudioRequest::Apply {
                target,
                epoch_id: catcoms_replication::epoch_zero_id(
                    logical.doc_type,
                    &logical.logical_key,
                ),
                nonce: [91; 16],
                body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("newer".into()))
                    .encode()
                    .unwrap(),
            },
        )
        .unwrap();
    receiver.complete(&mut server, result);
    receiver.run(&mut server, &mut store, 83, None).unwrap();
    assert_eq!(receiver.catchup.owner_next_at, 6000);
    for _ in 0..20 {
        assert!(!receiver.pending(&server));
        receiver.run(&mut server, &mut store, 83, None).unwrap();
        assert!(receiver.detach(&mut server).is_none());
    }
    clock.advance_ms(4999);
    assert!(!receiver.pending(&server));
    clock.advance_ms(1);
    assert!(receiver.pending(&server));
    receiver.run(&mut server, &mut store, 83, None).unwrap();
    assert_eq!(receiver.catchup.owner_next_at, 11000);
    assert!(!receiver.take_pause_notice());
}

#[tokio::test]
async fn studio_solo_owner_rotates_repeatedly_on_idle_and_reopens_latest_checkpoint() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(179);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"solo-rotation", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    let logical = target.document(&server.group_id()).unwrap();
    let mut receiver = StudioReceiver::default();
    for epoch in 1..=3 {
        server.sync.with_registry_context(|g, d, _, _| {
            crate::store::fill_studio_epoch_fixture(&mut store, 83, g, d, target)
        });
        if epoch == 1 {
            receiver
                .run(
                    &mut server,
                    &mut store,
                    83,
                    Some(StudioRequest::Read { target }),
                )
                .unwrap();
        }
        clock.advance_ms(5_000);
        let mut progressed = false;
        for _ in 0..12 {
            let (_, update) = receiver.run(&mut server, &mut store, 83, None).unwrap();
            assert!(
                !receiver.take_pause_notice(),
                "idle rotation must not pause receive"
            );
            if let Some(work) = receiver.detach(&mut server) {
                receiver.complete(&mut server, work.run(None).await);
            }
            if update == Some(target) {
                progressed = true;
                break;
            }
        }
        assert!(
            progressed,
            "epoch {epoch}: {:?}",
            receiver.catchup.owner_failure
        );
        let state = server
            .sync
            .with_registry_context(|g, d, _, _| store.load_studio_epoch(83, g, target, d))
            .unwrap()
            .unwrap();
        assert_eq!(state.epoch(), epoch);
        assert_eq!(state.phase(), catcoms_replication::EpochPhase::Open);
        assert_eq!(state.op_count(), 0);
        let journal = store.load_epoch_owner_receipts(83, &logical).unwrap();
        assert!(
            journal.pending().is_none(),
            "availability needs no remote query"
        );
        assert_eq!(receiver.watches.front().unwrap().1, state.doc_id());
        // Crash/reopen loses all volatile preparation, watches and owner permits. An ordinary
        // Read establishes a fresh binding; subsequent rotation must reuse the saved journal.
        drop(store);
        store = ServerStore::open(root.path(), b"solo-rotation", &mut rng).unwrap();
        receiver = StudioReceiver::default();
        receiver
            .run(
                &mut server,
                &mut store,
                83,
                Some(StudioRequest::Read { target }),
            )
            .unwrap();
    }
}

#[test]
fn studio_discovery_superseded_watch_drops_all_scheduled_work_without_rebinding() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(78);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"revoked", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    let old = server.sync.watch_studio(target, 4).unwrap();
    let mut runtime = CatchupRuntime {
        target: Some(target),
        discovery_watch: Some(old.copy_binding()),
        discovery_needed: Some(target),
        ..Default::default()
    };
    runtime.discovery_plan = Some(DiscoveryPlan {
        mount: store.registry_mount(),
        server: 83,
        peer: PeerId::from_u64(2),
        target: CheckpointTarget::Studio(target),
    });
    runtime.after_registry = Some(DiscoveryPlan {
        mount: store.registry_mount(),
        server: 83,
        peer: PeerId::from_u64(2),
        target: CheckpointTarget::Studio(target),
    });
    let newer = server.sync.watch_studio(target, 5).unwrap();
    assert!(runtime
        .advance_checkpoint(&mut server, &mut store, 83)
        .unwrap()
        .is_none());
    assert!(
        runtime.discovery_plan.is_none()
            && runtime.after_registry.is_none()
            && runtime.discovery_needed.is_none()
    );
    assert!(
        runtime.binding.is_none(),
        "no late work may recreate the old UI watch"
    );
    assert!(server.sync.studio_watch_is_current(&newer));
}

#[test]
fn studio_owner_lifecycle_retries_failed_snapshot_on_local_clock_not_request_frequency() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(77);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = ServerStore::open(root.path(), b"retry", &mut rng).unwrap();
    let mut runtime = CatchupRuntime::default();
    runtime.lifecycle_with(&mut server, &store, 83, |_, _, _| {
        Err(invalid("injected source flush failure"))
    });
    assert!(runtime.owner_snapshot.is_none());
    for _ in 0..100 {
        runtime.lifecycle_with(&mut server, &store, 83, |_, _, _| {
            panic!("request frequency must not retry a failed lifecycle save")
        });
    }
    clock.advance_ms(30_000);
    runtime.lifecycle(&mut server, &store, 83);
    assert!(server.owner_head_snapshot_is_current(
        &store,
        83,
        runtime
            .owner_snapshot
            .as_ref()
            .expect("idle recovery without MLS or UI change")
    ));
    clock.advance_ms(30_000);
    runtime.lifecycle_with(&mut server, &store, 83, |_, _, _| {
        panic!("successful current lifecycle must be reused")
    });
}

fn pooled(permits: usize) -> CatchupRuntime {
    CatchupRuntime {
        overlay_pool: Some(Arc::new(tokio::sync::Semaphore::new(permits))),
        ..Default::default()
    }
}

/// N15, the half that a per-actor test can prove exactly: there is no overlay-only pool. The
/// production reservation draws from the one process-wide preparation semaphore that source and
/// registry preparation use, which is why a full pool is a retryable refusal rather than a queue.
#[test]
fn overlay_reservation_shares_the_one_preparation_pool() {
    assert!(
        Arc::ptr_eq(
            &CatchupRuntime::default().overlay_pool(),
            crate::registry_catchup::preparation_pool()
        ),
        "overlay work was given a pool of its own"
    );
}

/// N15. A full shared pool refuses the reservation and nothing else happens: no body is read, no
/// blob touched, no admission consumed. Releasing one slot makes the identical retry succeed.
#[test]
fn a_full_preparation_pool_refuses_an_overlay_reservation_and_recovers() {
    let mut runtime = pooled(4);
    let pool = runtime.overlay_pool();
    let held: Vec<_> = (0..4)
        .map(|_| {
            pool.clone()
                .try_acquire_owned()
                .expect("the pool has four slots")
        })
        .collect();

    assert!(
        runtime.reserve_overlay().is_none(),
        "an overlay job was admitted with no slot left in the shared pool"
    );
    // The refusal is capacity only: admission was not consumed by the attempt, so the retry below
    // is not being served by a leftover token.
    assert!(runtime.overlay_admission_available_for_test());

    drop(held);
    let ownership = runtime
        .reserve_overlay()
        .expect("a freed slot did not admit the identical retry");
    assert_eq!(pool.available_permits(), 3);
    drop(ownership);
    assert_eq!(pool.available_permits(), 4);
}

/// I-2 through the runtime rather than the seam alone: one overlay job per actor, and a cancelled
/// waiter does **not** make a second admissible. This is 7.1's first bullet, and the case that
/// motivated weak-handle bookkeeping: the blocking closure still owns the bundle and is still
/// running, so the slot and the admission must stay occupied with no release message from it.
#[test]
fn a_cancelled_overlay_waiter_does_not_free_a_still_running_worker_slot() {
    let mut runtime = pooled(4);
    let pool = runtime.overlay_pool();
    let ownership = runtime
        .reserve_overlay()
        .expect("the first job is admitted");
    assert_eq!(pool.available_permits(), 3);
    assert!(
        runtime.reserve_overlay().is_none(),
        "a second overlay job was admitted for the same actor"
    );

    // The waiter is cancelled: the runtime drops its tracked job and `run` yields CancelledOverlay,
    // which clears the waiter bookkeeping only. `ownership` here stands for the bundle the still
    // running blocking closure owns.
    runtime.overlay_detached = true;
    drop(runtime.overlay.take()); // the runtime's own tracked handle goes with the waiter
    runtime.note_cancelled_overlay_for_test();
    assert!(!runtime.overlay_detached);
    assert!(
        runtime.reserve_overlay().is_none(),
        "a cancelled waiter released admission while its worker was still running"
    );
    assert_eq!(
        pool.available_permits(),
        3,
        "a cancelled waiter refunded a slot its worker still owns"
    );

    // The worker finishes by itself. Both come back with no second visit and no release call.
    drop(ownership);
    assert_eq!(pool.available_permits(), 4);
    assert!(runtime.reserve_overlay().is_some());
}

/// A parked plan is a live job: it still owns the bundle and its pixels are still protected only
/// by its transient hold, so the actor must not admit another overlay until a custody visit
/// consumes it. The reservation also refuses while a capture is queued but not yet detached.
#[test]
fn a_queued_or_parked_overlay_keeps_the_actor_busy() {
    let mut runtime = pooled(4);
    let ownership = runtime
        .reserve_overlay()
        .expect("the first job is admitted");
    runtime.overlay_detached = true;
    assert!(runtime.reserve_overlay().is_none(), "detached");
    runtime.overlay_detached = false;

    drop(ownership);
    assert!(runtime.reserve_overlay().is_some());
}

/// Flow H end to end through the scheduled runtime: H1 probe, H2 detached, H3 paged across
/// background turns, H4 detached, H5 commit and H6 notify.
///
/// Nothing here is simulated. The vault is left exactly where automatic transfer becomes
/// possible by production calls, the probe rediscovers the basis for itself, and each detached
/// stage runs through the real `StudioBackgroundJob::run`.
#[tokio::test]
async fn studio_handoff_runs_through_every_scheduled_stage_and_notifies() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(929);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 83, g, d, target, 1)
    });

    let mut receiver = StudioReceiver::default();
    // An ordinary read establishes the watch the probe walks.
    receiver
        .run(
            &mut server,
            &mut store,
            83,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    // A private pool: permit arithmetic must not contend with other tests in this process on the
    // one global preparation semaphore.
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);
    let free = pool.available_permits();

    // Drive background turns until the transfer completes, detaching whenever the runtime asks.
    // What detaches is recorded, because that is the observable proof that H2 and H4 genuinely
    // left custody rather than running inline.
    let mut detached = Vec::new();
    let mut signing_visits = 0;
    let mut updated = None;
    for _ in 0..60 {
        let (_, changed) = receiver.run(&mut server, &mut store, 83, None).unwrap();
        if receiver.handoff.remaining_for_test().is_some() {
            signing_visits += 1;
        }
        if let Some(work) = receiver.detach(&mut server) {
            detached.push(work.kind_for_test());
            receiver.complete(&mut server, work.run(None).await);
        }
        if changed == Some(target) {
            updated = changed;
            break;
        }
    }
    assert_eq!(
        updated,
        Some(target),
        "the scheduled handoff never completed; detached: {detached:?}"
    );

    // Both expensive stages really detached. A runtime that ran them inline would finish the
    // transfer just as well and would be exactly the defect this whole flow exists to avoid.
    assert!(
        detached.contains(&"handoff-prepare"),
        "H2 did not detach: {detached:?}"
    );
    assert!(
        detached.contains(&"handoff-assemble"),
        "H4 did not detach: {detached:?}"
    );
    assert!(
        signing_visits >= 1,
        "H3 never ran under custody: {detached:?}"
    );

    // H6: the transfer is durable and reported, and the actor is free again.
    let saved = store
        .load_epoch_intents(83, &target.document(&server.group_id()).unwrap())
        .unwrap();
    assert!(
        saved.overlay().is_none(),
        "the transferred branch was retained"
    );
    assert!(
        receiver.handoff.stage_for_test().is_none(),
        "a job was left behind"
    );
    assert_eq!(
        pool.available_permits(),
        free,
        "the finished transfer kept a shared preparation slot"
    );
    assert!(receiver.catchup.overlay_admission_available_for_test());
    assert!(
        receiver
            .take_settlement_notices()
            .iter()
            .any(|(t, _)| *t == target),
        "H6 published no settlement notice for the completed transfer"
    );
}

/// H3 really pages, and the priority yield really yields.
///
/// The happy-path test uses a one-operation branch, so H3 finishes in a single slice and cannot
/// discriminate the turn cap, the two-event rule or the priority gate. This one uses a branch
/// longer than `MAX_SIGNING_TURNS_PER_VISIT`, so signing must span turns, and drives a priority
/// turn in the middle to prove a yield signs nothing and costs no progress.
#[tokio::test]
async fn handoff_signing_pages_across_turns_and_a_priority_turn_signs_nothing() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(1303);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h-paging", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    // Longer than one slice, so paging is forced rather than hoped for.
    let operations = crate::store::MAX_SIGNING_TURNS_PER_VISIT + 5;
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 83, g, d, target, operations)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            83,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    receiver.catchup.inject_overlay_pool_for_test(4);

    // Reach H3.
    for _ in 0..30 {
        receiver.run(&mut server, &mut store, 83, None).unwrap();
        if let Some(work) = receiver.detach(&mut server) {
            receiver.complete(&mut server, work.run(None).await);
        }
        if receiver.handoff.stage_for_test() == Some("signing") {
            break;
        }
    }
    assert_eq!(receiver.handoff.stage_for_test(), Some("signing"));
    assert_eq!(
        receiver.handoff.remaining_for_test(),
        Some(operations),
        "the branch is not the length this test needs"
    );

    // A priority turn signs nothing and leaves the count untouched. This is the two-event rule:
    // a yield is a different outcome from a bounded slice, even though both leave work remaining.
    let before = receiver.handoff.remaining_for_test();
    let yielded = receiver
        .handoff_sign(&mut server, &mut store, true)
        .expect("a slice");
    assert!(
        yielded.yielded(),
        "a priority turn was not reported as a yield"
    );
    assert_eq!(yielded.signed(), 0, "a priority turn signed something");
    assert_eq!(receiver.handoff.remaining_for_test(), before);

    // An ordinary slice is bounded by the turn cap, so it cannot finish this branch in one go.
    let first = receiver
        .handoff_sign(&mut server, &mut store, false)
        .expect("a slice");
    assert!(!first.yielded());
    assert_eq!(
        first.signed(),
        crate::store::MAX_SIGNING_TURNS_PER_VISIT,
        "the slice was not bounded by the turn cap"
    );
    assert_eq!(first.remaining(), 5, "signing did not page");
    assert!(!first.complete());

    // A second slice finishes it, which is what "paged across turns" means.
    let second = receiver
        .handoff_sign(&mut server, &mut store, false)
        .expect("a slice");
    assert_eq!(second.signed(), 5);
    assert!(second.complete());

    // Still nothing durable: H5 alone writes.
    assert!(store
        .load_epoch_intents(83, &target.document(&server.group_id()).unwrap())
        .unwrap()
        .overlay()
        .is_some());
}

/// Which half of a job's pinned authority moved. Both must abandon it, and an earlier version of
/// this check covered only the first: a same-owner MLS commit leaves the observed tenure start
/// untouched, so `Mls` is the case that was silently unhandled.
#[derive(Clone, Copy, Debug)]
enum AuthorityMove {
    Owner,
    Mls,
}

#[tokio::test]
async fn an_owner_change_during_signing_abandons_the_job_without_pausing_the_receiver() {
    authority_change_during_signing(AuthorityMove::Owner).await;
}

/// The case the first version of the authority check could not see. A member joining or leaving,
/// or a key rotating, advances the MLS epoch while the owner and their tenure start are unchanged,
/// so a tenure-only comparison matches and the job is silently unsignable for ever.
#[tokio::test]
async fn an_mls_commit_during_signing_abandons_the_job_without_pausing_the_receiver() {
    authority_change_during_signing(AuthorityMove::Mls).await;
}

/// The two P1s from the Flow H review, which the end-to-end happy path could not see.
///
/// An authority change during H3 must not (a) reach the receiver's storage pause, which would stop
/// catch-up, replay and receive until the user next opened a Studio document, and must not (b)
/// leave the job parked holding this actor's admission and one of four process-wide preparation
/// permits with no path that ever releases them.
async fn authority_change_during_signing(moved: AuthorityMove) {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(1201);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h-authority", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 83, g, d, target, 1)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            83,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    // A private pool: permit arithmetic must not contend with other tests in this process on the
    // one global preparation semaphore.
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);
    let free = pool.available_permits();

    // Drive to the Signing stage, which is where a plan holds a bundle across turns.
    for _ in 0..30 {
        receiver.run(&mut server, &mut store, 83, None).unwrap();
        if let Some(work) = receiver.detach(&mut server) {
            receiver.complete(&mut server, work.run(None).await);
        }
        if receiver.handoff.stage_for_test() == Some("signing") {
            break;
        }
    }
    assert_eq!(
        receiver.handoff.stage_for_test(),
        Some("signing"),
        "the fixture never reached H3, so this proves nothing"
    );
    assert_eq!(pool.available_permits(), free - 1, "H3 holds a shared slot");

    // The authority moves out from under the parked plan. Only that precondition is simulated;
    // the comparison, the abandonment and the receiver's reaction to it are production paths.
    match moved {
        AuthorityMove::Owner => receiver.handoff.stale_tenure_for_test(),
        AuthorityMove::Mls => receiver.handoff.stale_mls_for_test(),
    }

    // A few ordinary turns. Not every turn reaches the background step, so this gives the
    // receiver a fair chance to notice rather than asserting on one particular scheduling path.
    for _ in 0..5 {
        receiver
            .run(&mut server, &mut store, 83, None)
            .expect("an authority change must not fail the background turn");
        assert!(
            !receiver.take_pause_notice(),
            "an authority change paused the receiver"
        );
    }
    assert!(
        receiver.handoff.stage_for_test().is_none(),
        "the unsignable job was left parked"
    );
    assert_eq!(
        pool.available_permits(),
        free,
        "the abandoned job kept its shared preparation slot"
    );
    assert!(receiver.catchup.overlay_admission_available_for_test());

    // And the receiver keeps working: another turn runs without error.
    receiver.run(&mut server, &mut store, 83, None).unwrap();
}

/// The pause that strands its own release.
///
/// A background pass can own a non-detached bundle when an unrelated step errors. That error sets
/// `paused`, and from then on `pending` is false by its leading `!self.paused` and `wake_in`
/// returns nothing, so no further turn is ever scheduled — and `release_if_stalled` lives at the
/// top of `run`, which is exactly the event the paused state prevents. The bundle would hold this
/// actor's admission and one of four process-wide permits until a user happened to open a Studio
/// document successfully.
///
/// The release therefore has to happen at the transition. This test never calls `run` again after
/// the failing one, because in production there would be nothing to call it.
#[tokio::test]
async fn a_pause_releases_the_bundle_it_strands_rather_than_waiting_for_a_turn_that_never_comes() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(1907);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h-pause-edge", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [23; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 90, g, d, target, 1)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            90,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);
    let free = pool.available_permits();

    // A job holding a bundle, in a stage `release_if_stalled` would act on.
    for _ in 0..40 {
        receiver.run(&mut server, &mut store, 90, None).unwrap();
        if let Some(work) = receiver.detach(&mut server) {
            receiver.complete(&mut server, work.run(None).await);
        }
        if matches!(
            receiver.handoff.stage_for_test(),
            Some("signing") | Some("ready")
        ) {
            break;
        }
    }
    assert!(
        matches!(
            receiver.handoff.stage_for_test(),
            Some("signing") | Some("ready")
        ),
        "no job holds a bundle, so this proves nothing"
    );
    assert_eq!(pool.available_permits(), free - 1);

    // The pause arrives. Nothing else happens afterwards, deliberately.
    receiver.pause_at_for_test(&server);

    assert!(
        receiver.handoff.stage_for_test().is_none(),
        "the pause parked a bundle in a stage no turn will ever visit"
    );
    assert_eq!(
        pool.available_permits(),
        free,
        "the pause stranded the shared preparation slot"
    );
    assert!(
        receiver.catchup.overlay_admission_available_for_test(),
        "the pause stranded this actor's admission"
    );
    assert!(
        !receiver.pending(&server) && receiver.wake_in(&server).is_none(),
        "a paused receiver must publish neither work nor a deadline, which is exactly why the \
         release cannot be deferred to a later turn"
    );
}

/// "Is there work now" and "when is there work next" must come from one clock read.
///
/// Sampling them separately leaves a window the size of whatever runs between the two calls. Take
/// `pending` at `D - 1`, let the clock cross `D`, then ask for a deadline: a `wake_in` that only
/// publishes future deadlines answers "nothing to wait for" — correctly, by its own rule — and
/// the actor arms no timer while `studio_pending` is false. The `Ready` job goes on holding
/// admission and a process-wide permit until unrelated work happens by. Same strand as the
/// original finding, squeezed into the expiry boundary.
///
/// The clock here returns `D - 1` to the first read of the pair and `D` to the second, which is
/// the interleaving that window admits. One sample makes the two answers exhaustive: runnable and
/// pending at or past `D`, a strictly positive delay below it, and no third case.
#[tokio::test]
async fn pending_and_the_wake_deadline_come_from_one_clock_read() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(1811);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h-boundary", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [21; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 89, g, d, target, 1)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            89,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);
    let free = pool.available_permits();

    for _ in 0..40 {
        receiver.run(&mut server, &mut store, 89, None).unwrap();
        if let Some(work) = receiver.detach(&mut server) {
            receiver.complete(&mut server, work.run(None).await);
        }
        if receiver.handoff.stage_for_test() == Some("ready") {
            break;
        }
    }
    assert_eq!(
        receiver.handoff.stage_for_test(),
        Some("ready"),
        "the fixture never reached H5, so this proves nothing"
    );
    assert_eq!(pool.available_permits(), free - 1);

    let now = server.runtime_clock().monotonic_ms();
    receiver.handoff.hold_live_job_for_test(now);
    let deadline = now + 30_000;
    let rail: Vec<_> = [target].to_vec();

    // The invariant that makes a single sample sufficient: at every instant there is either work
    // to do now or a deadline to wait for. A window with neither is the strand, and with two
    // clock reads the pair could land either side of the boundary and fall into it.
    for at in [deadline - 2, deadline - 1, deadline, deadline + 1] {
        let runnable = receiver.handoff.runnable(at);
        let wake = receiver.handoff.wake_in(at, &rail);
        assert!(
            runnable || wake.is_some(),
            "at {at} (deadline {deadline}) the job was neither runnable nor waited on, so \
             nothing would ever run it and its permit is stranded"
        );
        assert!(
            !(runnable && wake.is_some()),
            "at {at} the job is runnable and still publishing a deadline, which is a spin"
        );
    }

    // And the production pairing agrees with it, taken as one call rather than two.
    clock.advance_ms(29_999);
    let signal = tokio::sync::watch::channel(false).0;
    assert_eq!(
        receiver.signal_and_wake(&server, &signal),
        Some(1),
        "one millisecond short of the deadline, no timer would be armed"
    );
    clock.advance_ms(1);
    assert!(
        receiver.signal_and_wake(&server, &signal).is_none(),
        "an expired deadline is still being published"
    );
    assert!(
        *signal.borrow(),
        "at the deadline the job was not reported pending, so the driver would never run it"
    );
}

/// Design 7.2, whose section title is "Reservation precedes every body read".
///
/// The H1 probe's rail scan is body reads: one authenticated intent record read and structurally
/// decoded per candidate. Reserving after it meant a background probe could decode several
/// records per turn while holding neither this actor's admission nor one of the four process-wide
/// permits — defeating the admission control the pool exists to provide, in the one flow that
/// runs with no user behind it.
///
/// The second assertion is the other half. Charging a doubling per-target hold for losing a race
/// on capacity penalises a perfectly eligible document for being unlucky, and escalates
/// contention exactly the way it escalates genuine ineligibility.
#[tokio::test]
async fn an_exhausted_pool_stops_the_probe_before_it_reads_any_intent_body() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(1709);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h-admission", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [19; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 88, g, d, target, 1)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            88,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);

    // Every slot taken, as four other actors preparing would take them.
    let taken: Vec<_> = (0..4)
        .map(|_| {
            pool.clone()
                .try_acquire_owned()
                .expect("the injected pool should start empty of holders")
        })
        .collect();
    assert_eq!(pool.available_permits(), 0);

    let before = receiver.handoff.selection_for_test();
    receiver.run(&mut server, &mut store, 88, None).unwrap();

    assert!(
        receiver.handoff.stage_for_test().is_none(),
        "a job was captured with no permit to hold"
    );
    assert_eq!(
        receiver.handoff.selection_for_test(),
        before,
        "the probe scanned the rail, reading intent bodies, while holding no admission and no \
         process-wide permit"
    );
    let now = server.runtime_clock().monotonic_ms();
    assert!(
        !receiver.handoff.held_for_test(target, now),
        "losing a race for capacity charged an eligible document a doubling backoff, escalating \
         contention the same way genuine ineligibility escalates"
    );

    // A refusal must still leave something to come back for. The reservation is a `try_acquire`,
    // so this actor is not queued behind the permit and nothing will tell it when capacity
    // returns; with no job, no per-target hold and no deadline, an eligible draft would sit
    // untransferred until unrelated Studio work happened to produce another custody turn. The
    // earlier version of this test hid that by calling `run` again itself, which production does
    // not get for free.
    let retry = receiver
        .handoff
        .wake_in(now, &[target])
        .expect("a capacity refusal left no future retry, so the transfer is dormant");
    assert!(
        retry > 0 && retry <= 2_000,
        "a capacity retry should be short and flat, not a doubling ineligibility backoff: {retry}"
    );

    // The gate half: while the retry is in the future, the probe must not re-attempt at all.
    // Recording a deadline nothing reads is how an H5 hold became an unpaced retry, and the claim
    // of two-second pacing has to hold under active Studio traffic too.
    //
    // The witness is that the deadline **counts down**. A probe that re-attempts rewrites it to
    // `now + 2000` every turn, so the remaining time would stay pinned at the full interval
    // instead of shrinking. The selection cursor is no witness here: with the pool still
    // exhausted the reservation fails before the cursor moves either way, which is why the first
    // version of this assertion passed against a build with the gate removed.
    clock.advance_ms(500);
    receiver.run(&mut server, &mut store, 88, None).unwrap();
    let after = server.runtime_clock().monotonic_ms();
    assert_eq!(
        receiver.handoff.wake_in(after, &[target]),
        Some(retry - 500),
        "the retry deadline was rewritten, so the probe re-attempted inside its own pacing"
    );

    // The wake half, which the earlier version of this test never proved: expiry must itself
    // schedule the run. With no job, `runnable` is false by definition, so if the deadline simply
    // stops being published at `now == D` the timer that fired is the last one this actor arms.
    let rail: Vec<_> = [target].to_vec();
    assert!(
        !receiver.handoff.probe_due(after, &rail),
        "the retry reported due before its deadline"
    );
    clock.advance_ms(retry - 500);
    let due = server.runtime_clock().monotonic_ms();
    assert!(
        receiver.handoff.probe_due(due, &rail),
        "at its deadline the retry was neither due nor published, so the probe is never requested \
         again and an eligible draft stays untransferred"
    );
    assert!(
        receiver.pending(&server),
        "a due probe did not reach the driver"
    );

    // With capacity back, the same target is picked up normally: the refusal left no trace.
    drop(taken);
    assert_eq!(pool.available_permits(), 4);
    receiver.run(&mut server, &mut store, 88, None).unwrap();
    assert!(
        receiver.handoff.stage_for_test().is_some(),
        "the probe did not recover once capacity returned"
    );
    // Asserted on the retry itself, not through `wake_in`: once a job exists `wake_in` takes its
    // live-job branch and never looks at `probe_retry_at`, so routing the claim through it would
    // stay green with the clear deleted.
    assert_eq!(
        receiver.handoff.probe_retry_for_test(),
        None,
        "the capacity retry outlived the reservation that succeeded"
    );
    assert!(receiver.handoff.wake_in(due, &rail).is_none());
}

/// The other pre-capacity exit: a rail that is not empty but has nothing the probe may inspect.
///
/// `handoff_probe` has two exits before the capacity gate — an empty or busy rail, and a rail
/// where every target is quiet or still held. Both must consume the capacity retry, for the same
/// reason: it is owed only to an eligible target that could not get a permit. The empty-rail exit
/// is covered; this one was not, and a mutant deleting only its clear passed the other two tests.
#[tokio::test]
async fn a_rail_with_nothing_eligible_also_consumes_the_capacity_retry() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(2213);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h-noneligible", &mut rng).unwrap();
    let eligible = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [29; 16],
    };
    let paced = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [30; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 93, g, d, eligible, 1);
        crate::store::save_studio_source_fixture(&mut store, 93, g, d, paced)
    });
    let mut receiver = StudioReceiver::default();
    for t in [eligible, paced] {
        receiver
            .run(
                &mut server,
                &mut store,
                93,
                Some(StudioRequest::Read { target: t }),
            )
            .unwrap();
    }
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);

    // `paced` is held far out; `eligible` is what lets the probe reach the reservation.
    let start = server.runtime_clock().monotonic_ms();
    receiver.handoff.hold_target_for_test(paced, start);

    let taken: Vec<_> = (0..4)
        .map(|_| pool.clone().try_acquire_owned().expect("a free slot"))
        .collect();
    receiver.run(&mut server, &mut store, 93, None).unwrap();
    assert!(
        receiver.handoff.probe_retry_for_test().is_some(),
        "no capacity retry was recorded, so this proves nothing"
    );

    // The eligible target stops being watched. The rail is still non-empty, but everything left
    // on it is held, so the probe now exits at its eligibility test rather than the empty-rail
    // check — a different line, with the same obligation.
    receiver.watches.retain(|(w, _)| w.target != eligible);
    assert!(
        !receiver.watches.is_empty(),
        "this must not be the empty-rail case"
    );
    drop(taken);

    clock.advance_ms(5_000);
    // The probe is called directly. `background_step` only reaches it behind `replay_ready`, and
    // the claim under test is about what this function does at that exit, not about which
    // scheduler turn happens to reach it.
    receiver.handoff_probe(&mut server, &mut store, 93);
    let now = server.runtime_clock().monotonic_ms();
    assert_eq!(
        receiver.handoff.probe_retry_for_test(),
        None,
        "a rail with nothing eligible left the capacity retry owed to a target it no longer has"
    );
    assert!(!receiver.handoff.probe_due(now, &[paced]));
}

/// Two deadline systems, one gate. The capacity retry has to dominate per-target pacing.
///
/// A target whose own hold has expired is due, and `handoff_probe` reaches its eligibility test
/// and passes it — then returns at the capacity gate, which sits after that test and knows
/// nothing about which target is due. So `probe_due` would report work the probe is not permitted
/// to do, every turn, for the whole capacity wait: the two-second pacing throttles the
/// `try_acquire` correctly and does not pace the receiver at all, which keeps the actor on its
/// active Studio cadence taking turns that return immediately.
///
/// Not a strand, so not a P1. But it is the same gate/wake disagreement as everything before it,
/// this time between two independent deadline classes rather than between two readings of one.
#[tokio::test]
async fn a_future_capacity_gate_suppresses_an_expired_target_hold() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(2111);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h-overlap", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [27; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 92, g, d, target, 1)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            92,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    // A second watched target, so the rail can hold a target deadline that is still *future* while
    // another target keeps the probe eligible. One target cannot do both: the probe only arms a
    // capacity gate when something is eligible, and an eligible target's own deadline has by then
    // expired, which `wake_in` filters out either way. That is why the first version of this test
    // could not tell the two forms apart.
    let held = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [28; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::save_studio_source_fixture(&mut store, 92, g, d, held)
    });
    receiver
        .run(
            &mut server,
            &mut store,
            92,
            Some(StudioRequest::Read { target: held }),
        )
        .unwrap();
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);
    let rail: Vec<_> = [target, held].to_vec();

    // `held` is paced 30 s out; `target` stays eligible, which is what lets the probe reach the
    // reservation at all.
    let start = server.runtime_clock().monotonic_ms();
    receiver.handoff.hold_target_for_test(held, start);
    clock.advance_ms(29_000);

    // Capacity is gone when the probe acts, so a two-second gate is armed *over* a target
    // deadline that is only one second away.
    let taken: Vec<_> = (0..4)
        .map(|_| pool.clone().try_acquire_owned().expect("a free slot"))
        .collect();
    receiver.run(&mut server, &mut store, 92, None).unwrap();
    let gate = receiver
        .handoff
        .probe_retry_for_test()
        .expect("no capacity gate was armed, so this proves nothing");
    let now = server.runtime_clock().monotonic_ms();
    assert!(
        now < start + 30_000 && start + 30_000 < gate,
        "the target deadline is not inside the gate, so the overlap under test cannot arise"
    );

    // The actor must wait on the gate, not on the nearer target deadline it could not act on.
    assert_eq!(
        receiver.handoff.wake_in(now, &rail),
        Some(gate - now),
        "the actor is waiting on a target deadline that is earlier than the capacity gate, so it \
         wakes for a turn that can only return at that gate"
    );

    // Past that target deadline but still under the gate: due must stay false.
    clock.advance_ms(1_500);
    let between = server.runtime_clock().monotonic_ms();
    assert!(
        between > start + 30_000 && between < gate,
        "the clock is not between the two deadlines"
    );
    assert!(
        !receiver.handoff.probe_due(between, &rail),
        "an expired target hold reported due underneath a future capacity gate, so the receiver \
         takes a turn that can only return at that gate"
    );

    // At the gate, and only then.
    clock.advance_ms(gate - between);
    let at = server.runtime_clock().monotonic_ms();
    assert!(
        receiver.handoff.probe_due(at, &rail),
        "the capacity gate expired and nothing reported the work due"
    );
    assert!(receiver.handoff.wake_in(at, &rail).is_none());
    drop(taken);
}

/// A capacity retry must not outlive the eligibility that justified it.
///
/// `probe_retry_at` is global rather than per-target, so unlike `next_at` nothing filters it by
/// the current rail. It is owed only because an eligible target could not get a permit; if that
/// target stops being watched before the deadline, the retry has outlived its reason and no other
/// path consumes it. `probe_due` would then report work for ever while `handoff_probe` returns
/// immediately on the empty rail, holding the receiver at its active Studio cadence with nothing
/// a probe could possibly do. No permit is stranded, which is what separates this from the
/// capacity strand, but permanent useless work is still a defect.
///
/// The invariant is: `probe_due` true implies the next probe either attempts capacity, arms
/// another future gate, or consumes the state. Every exit that is not the retry's own gate
/// therefore clears it.
#[tokio::test]
async fn a_capacity_retry_does_not_outlive_the_watch_that_justified_it() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(2003);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h-stale-retry", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [25; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 91, g, d, target, 1)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            91,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);

    // An eligible target that cannot get a permit: the only state that owes a capacity retry.
    let taken: Vec<_> = (0..4)
        .map(|_| pool.clone().try_acquire_owned().expect("a free slot"))
        .collect();
    receiver.run(&mut server, &mut store, 91, None).unwrap();
    assert!(
        receiver.handoff.probe_retry_for_test().is_some(),
        "no capacity retry was recorded, so this proves nothing"
    );

    // The watch goes away before the deadline, as ordinary subscription churn makes it.
    receiver.watches.clear();
    drop(taken);

    // Past the deadline, one turn must settle it rather than leaving it due for ever.
    clock.advance_ms(5_000);
    receiver.run(&mut server, &mut store, 91, None).unwrap();
    let now = server.runtime_clock().monotonic_ms();
    assert!(
        !receiver.handoff.probe_due(now, &[]),
        "an expired capacity retry survived the disappearance of every eligible target, so the \
         receiver reports work for ever while the probe returns immediately"
    );
    assert!(
        !receiver.pending(&server),
        "the receiver is permanently pending with no watch and nothing a probe could do"
    );
    assert_eq!(receiver.handoff.probe_retry_for_test(), None);
}

/// Design 6.1's M4: H3 reauthenticates the wrappers before the first `sign_next` of a visit.
///
/// `sign_next`'s own live-authority recheck is not this. It proves the device, its key, its
/// membership, the MLS epoch, the tenure and the current owner — who is signing, and under what
/// authority. It says nothing about whether the authenticated records H2 reconstructed the
/// candidate from are still the bytes on disk. Between two signing visits they can change, and
/// without this check the device's signing authority is spent on a proposal the contract says to
/// reject before the first signature.
///
/// H5 refuses the result later, so nothing durable goes wrong; the signatures are simply wasted.
/// That is why this is not a P1, and also why an end-to-end happy path can never catch it.
#[tokio::test]
async fn a_wrapper_change_between_signing_visits_signs_nothing_and_abandons() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(1601);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h-stamp", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [17; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 87, g, d, target, 8)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            87,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    receiver.catchup.inject_overlay_pool_for_test(4);

    // Stop at H3 with work still to do.
    for _ in 0..30 {
        receiver.run(&mut server, &mut store, 87, None).unwrap();
        if let Some(work) = receiver.detach(&mut server) {
            receiver.complete(&mut server, work.run(None).await);
        }
        if receiver.handoff.stage_for_test() == Some("signing") {
            break;
        }
    }
    assert_eq!(
        receiver.handoff.stage_for_test(),
        Some("signing"),
        "the fixture never reached H3, so this proves nothing"
    );
    let before = receiver.handoff.remaining_for_test();
    assert!(
        before.is_some_and(|n| n > 0),
        "nothing left to sign, so a refusal to sign proves nothing"
    );

    // An ordinary local edit between visits rewrites the intent wrapper H2 read. Authority is
    // untouched: same device, same key, same membership, same epoch, same owner, same tenure.
    let epoch_id = server
        .sync
        .with_registry_context(|g, d, _, _| store.load_studio_epoch(87, g, target, d))
        .unwrap()
        .expect("the installed successor")
        .doc_id();
    receiver
        .run(
            &mut server,
            &mut store,
            87,
            Some(StudioRequest::Apply {
                target,
                epoch_id,
                nonce: [93; 16],
                body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("moved on".into()))
                    .encode()
                    .unwrap(),
            }),
        )
        .expect("an ordinary edit must still succeed");

    // A priority turn yields first and reauthenticates nothing. 7.3 says a priority turn gives
    // way immediately, and the stamp check reads and hashes the whole intent record plus the
    // bounded source record: body work under custody, which is what the priority gate exists to
    // get out of the way of. The wrapper is already stale here, so a check that ran would abandon
    // the job — on a turn that was never allowed to start a signing visit at all.
    let yielded = receiver
        .handoff_sign(&mut server, &mut store, true)
        .expect("a priority turn must report its yield, not vanish");
    assert!(yielded.yielded(), "a priority turn did not report a yield");
    assert_eq!(yielded.signed(), 0, "a priority yield signed something");
    assert_eq!(
        receiver.handoff.remaining_for_test(),
        before,
        "a priority yield changed the plan"
    );
    assert_eq!(
        receiver.handoff.stage_for_test(),
        Some("signing"),
        "a priority turn reauthenticated the wrappers and abandoned the job, doing exactly the \
         custody work it was supposed to be yielding out of"
    );

    // The next real signing visit must sign nothing and give the job up.
    let slice = receiver.handoff_sign(&mut server, &mut store, false);
    assert!(
        slice.is_none(),
        "H3 signed against records that are no longer the ones it authenticated"
    );
    assert!(
        receiver.handoff.stage_for_test().is_none(),
        "a stale plan was left signable"
    );
}

/// A worker that finishes after the receiver pauses must not re-strand its bundle.
///
/// `release_if_stalled` exempts `Detached` on purpose: the worker owns the bundle, so there is
/// nothing to release. But a worker that succeeds hands the bundle **back**, and the job leaves
/// `Detached` for `Signing` or `Ready` holding this actor's admission and one of four
/// process-wide preparation permits. `pending` begins with `!self.paused`, so the driver
/// schedules no further turn, and the release hook on `run`'s paused path is never reached. The
/// bundle would sit there until the user happened to open a Studio document successfully.
///
/// The whole point is that no further `run` happens after the pause: this test must prove the
/// release without one, because in production there would not be one.
#[tokio::test]
async fn a_worker_finishing_after_a_pause_releases_its_bundle_without_another_visit() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(1511);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h-pause", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [15; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 86, g, d, target, 1)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            86,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);
    let free = pool.available_permits();

    // Get a real detached worker holding the bundle.
    let mut detached = None;
    for _ in 0..30 {
        receiver.run(&mut server, &mut store, 86, None).unwrap();
        if let Some(work) = receiver.detach(&mut server) {
            detached = Some(work);
            break;
        }
    }
    let work = detached.expect("no detached handoff stage, so this proves nothing");
    assert_eq!(
        receiver.handoff.stage_for_test(),
        Some("detached"),
        "the job did not actually detach"
    );
    assert_eq!(
        pool.available_permits(),
        free - 1,
        "the worker is not holding a shared slot, so there is nothing to strand"
    );

    // The receiver faults while the worker is still running.
    receiver.pause_for_test();

    // The worker succeeds anyway and hands the bundle back. No `run` follows, deliberately.
    let result = work.run(None).await;
    receiver.complete(&mut server, result);

    assert!(
        receiver.handoff.stage_for_test().is_none(),
        "a completion arriving during a pause parked the bundle in a stage no turn will ever visit"
    );
    assert_eq!(
        pool.available_permits(),
        free,
        "the shared preparation slot was stranded by the pause"
    );
    assert!(
        receiver.catchup.overlay_admission_available_for_test(),
        "this actor's admission was stranded by the pause"
    );
}

/// A paced refusal has to stop the retry **and** schedule the revisit. One without the other is
/// not pacing.
///
/// An H5 budget refusal records a per-target hold and deliberately keeps the signed `Ready` job
/// rather than discarding a detached vault decode and every signature. Two things then have to be
/// true, and the first version of this had neither.
///
/// The commit gate must read the hold its own failure path wrote. It did not: `can_commit` looked
/// only at the stage, so on a busy actor the most expensive stage in the flow retried on every
/// turn, draining a five-family inventory each time, while the recorded deadline did nothing.
///
/// And a quiescent actor must still come back. A held job reports not-runnable, so `pending` is
/// false and the driver schedules no further Studio turn — while the job holds this actor's
/// admission and one of four process-wide preparation permits. Four such actors take the whole
/// pool and never give it back. The deadline is published so the actor can wake on it.
#[tokio::test]
async fn a_held_handoff_job_neither_retries_inside_its_backoff_nor_stalls_past_it() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(1409);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h-pacing", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [13; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 85, g, d, target, 1)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            85,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);
    let free = pool.available_permits();

    // Drive all the way to Ready, which is the stage an H5 refusal parks.
    for _ in 0..40 {
        receiver.run(&mut server, &mut store, 85, None).unwrap();
        if let Some(work) = receiver.detach(&mut server) {
            receiver.complete(&mut server, work.run(None).await);
        }
        if receiver.handoff.stage_for_test() == Some("ready") {
            break;
        }
    }
    assert_eq!(
        receiver.handoff.stage_for_test(),
        Some("ready"),
        "the fixture never reached H5, so this proves nothing"
    );
    assert_eq!(
        pool.available_permits(),
        free - 1,
        "a Ready job is holding a shared slot, which is what makes stalling here expensive"
    );

    // What an H5 budget refusal records.
    let now = server.runtime_clock().monotonic_ms();
    receiver.handoff.hold_live_job_for_test(now);

    // A busy actor keeps giving the receiver turns. None of them may run H5.
    for _ in 0..10 {
        receiver.run(&mut server, &mut store, 85, None).unwrap();
        assert_eq!(
            receiver.handoff.stage_for_test(),
            Some("ready"),
            "H5 ran inside its own backoff, so the hold is bookkeeping rather than pacing"
        );
    }

    // A quiescent actor is told when to come back, rather than simply being told there is
    // nothing to do. The handoff term is asserted directly: advancing tens of seconds also moves
    // unrelated owner-rotation and replay deadlines, so `pending` alone cannot isolate it.
    assert!(!receiver.pending(&server), "a held job reported pending");
    let wake = receiver
        .wake_in(&server)
        .expect("a held job published no deadline, so a quiescent actor would never revisit it");
    clock.advance_ms(wake - 1);
    let now = server.runtime_clock().monotonic_ms();
    assert!(
        !receiver.handoff.runnable(now),
        "the job became runnable before its deadline"
    );
    assert_eq!(
        receiver.wake_in(&server),
        Some(1),
        "the published deadline did not track the clock"
    );

    clock.advance_ms(1);
    let now = server.runtime_clock().monotonic_ms();
    assert!(
        receiver.handoff.runnable(now),
        "the deadline passed and the job never became runnable again"
    );
    assert!(
        receiver.wake_in(&server).is_none(),
        "an expired hold is still publishing a deadline"
    );
    assert!(
        receiver.pending(&server),
        "a runnable job did not reach the driver"
    );

    // And the revisit actually commits, rather than merely becoming eligible to.
    receiver.run(&mut server, &mut store, 85, None).unwrap();
    assert!(
        receiver.handoff.stage_for_test().is_none(),
        "the job was runnable and the turn still did not run H5"
    );
    assert_eq!(
        pool.available_permits(),
        free,
        "the committed job kept its shared preparation slot"
    );
}

/// A completion has to be routed by job token, not by target.
///
/// `handoff_check_authority` abandons at any stage, `Detached` included, so a worker already
/// holding the bundle keeps running after its job is gone. Once that target's backoff expires the
/// actor legitimately captures a **new** job for the **same** target, and a completion matched on
/// target alone would then let the dead worker's result land on it: for `Cancelled`, clearing a
/// live job outright; for `Prepared`, installing a plan pinned to the superseded epoch, which can
/// never be signed, over a capture that was fine.
///
/// Both jobs here are real, and so is the abandonment between them. Only the late delivery of the
/// first worker's outcome is synthesised, because the point is which job receives it.
#[tokio::test]
async fn a_superseded_workers_completion_leaves_a_new_job_for_the_same_target_alone() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(1301);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"flow-h-token", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [11; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_handoff_ready_fixture(&mut store, 84, g, d, target, 1)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            84,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    receiver.catchup.inject_overlay_pool_for_test(4);

    // The first job, which is about to be superseded.
    for _ in 0..30 {
        receiver.run(&mut server, &mut store, 84, None).unwrap();
        if let Some(work) = receiver.detach(&mut server) {
            receiver.complete(&mut server, work.run(None).await);
        }
        if receiver.handoff.stage_for_test() == Some("signing") {
            break;
        }
    }
    let superseded = receiver
        .handoff
        .token_for_test()
        .expect("the fixture never produced a first job, so this proves nothing");

    // Authority moves, so the job is abandoned and its target is backed off.
    receiver.handoff.stale_mls_for_test();
    for _ in 0..5 {
        receiver.run(&mut server, &mut store, 84, None).unwrap();
    }
    assert!(
        receiver.handoff.stage_for_test().is_none(),
        "the superseded job was not abandoned, so the race under test cannot arise"
    );

    // Past the backoff, the same target is eligible again and a second job is captured.
    clock.advance_ms(60_000);
    let mut live = None;
    for _ in 0..30 {
        receiver.run(&mut server, &mut store, 84, None).unwrap();
        if let Some(token) = receiver.handoff.token_for_test() {
            live = Some(token);
            break;
        }
    }
    let live = live.expect("no second job for the same target, so this proves nothing");
    assert_ne!(
        live, superseded,
        "tokens must not be reused, or they cannot tell two jobs apart"
    );
    let stage = receiver.handoff.stage_for_test();

    // The first worker finally comes back. It is for this target, and for no live job.
    let now = server.runtime_clock().monotonic_ms();
    receiver.handoff_complete(HandoffCompletion::Cancelled(superseded), now);
    assert_eq!(
        receiver.handoff.token_for_test(),
        Some(live),
        "a superseded worker's cancellation cleared the live job for the same target"
    );
    assert_eq!(
        receiver.handoff.stage_for_test(),
        stage,
        "a superseded worker's completion moved the live job's stage"
    );
    receiver.run(&mut server, &mut store, 84, None).unwrap();
}

/// N14(a), the real race rather than a model of it. A background overlay worker is paused after
/// `OverlayOwnership` has moved inside its blocking closure; the waiter is then genuinely
/// cancelled, so `run` returns `CancelledOverlay` and `complete` clears the waiter flag. While the
/// worker is still running, a second overlay job must remain inadmissible and the shared slot must
/// stay taken, because the closure holds the only strong admission `Arc` and nobody sends any
/// release message. When the worker finishes on its own, both recover with no second visit.
#[tokio::test]
async fn a_cancelled_waiter_leaves_a_real_paused_worker_holding_admission_and_its_slot() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(613);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"cancel-race", &mut rng).unwrap();
    let first = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    let capture = server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_closing_capture_fixture(&mut store, 83, g, d, first, false)
    });

    let mut receiver = StudioReceiver::default();
    // A private pool: permit arithmetic must not contend with other tests in this process on the
    // one global preparation semaphore.
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);
    let free = pool.available_permits();
    let ownership = receiver
        .catchup
        .reserve_overlay()
        .expect("the first job is admitted");
    receiver
        .catchup
        .queue_overlay_for_test(capture, ownership, first);
    let work = receiver
        .detach(&mut server)
        .expect("the overlay is selected");
    assert!(receiver.catchup.overlay_detached);
    let (work, entered, release) = work.pause_overlay_for_test();

    // A real cancellation signal, of the kind the actor's lease publishes.
    let (cancel, signal) = tokio::sync::watch::channel(false);
    let cancellation = catcoms_rt::RequestCancellation::new(signal, None);

    // Run the real job concurrently with the race. `join!` polls the job first, so the blocking
    // worker starts before the second future waits on its entry signal.
    let (result, ()) = tokio::join!(work.run(Some(cancellation)), async {
        // Bounded, so a future regression where the worker never reaches the barrier fails with
        // this named assertion instead of hanging the test process.
        tokio::time::timeout(std::time::Duration::from_secs(60), entered)
            .await
            .expect("the worker never reached its barrier")
            .expect("the worker entered while owning the bundle");
        cancel.send(true).expect("the cancellation signal is live");
    });
    assert!(
        matches!(result, StudioBackgroundResult::CancelledOverlay),
        "a cancelled overlay waiter did not report cancellation"
    );

    // The ordinary completion path. It clears the waiter flag and nothing else: the worker is
    // still paused inside `plan`, still owning admission and the shared permit.
    receiver.complete(&mut server, result);
    assert!(!receiver.catchup.overlay_detached);
    assert!(
        receiver.catchup.reserve_overlay().is_none(),
        "a cancelled waiter admitted a second overlay job while its worker was still running"
    );
    assert_eq!(
        pool.available_permits(),
        free - 1,
        "a cancelled waiter refunded a shared slot its worker still owns"
    );

    // Release the worker. It finishes by itself, with no release call from the actor.
    release.send(()).expect("the worker is still running");
    for _ in 0..600 {
        if pool.available_permits() == free {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(
        pool.available_permits(),
        free,
        "the finished worker never returned its shared slot"
    );
    assert!(
        receiver.catchup.reserve_overlay().is_some(),
        "admission never recovered after the worker finished"
    );
}

/// RT-001. A plan that cannot be committed must release admission and its slot from the four-slot
/// process-wide pool the moment the worker finishes, not when some later Save for the same target
/// happens to collect them, and not never. Its media hold has already died with the capture, so
/// nothing is being protected by keeping the rest parked.
///
/// The refusal is real: the document's ordinary intent ledger is at `MAX_INTENT_BYTES_PER_DOCUMENT`,
/// so `IntentLedger::prepare` inside `plan()` refuses. Classification never calls `prepare`, which
/// is why this is reachable only in the detached stage.
#[tokio::test]
async fn a_refused_plan_releases_admission_and_its_pool_slot_without_a_second_visit() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(407);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"refused-plan", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    let capture = server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_closing_capture_fixture(&mut store, 83, g, d, target, true)
    });

    let mut receiver = StudioReceiver::default();
    // A private pool: permit arithmetic must not contend with other tests in this process on the
    // one global preparation semaphore.
    let pool = receiver.catchup.inject_overlay_pool_for_test(4);
    let free = pool.available_permits();
    let ownership = receiver
        .catchup
        .reserve_overlay()
        .expect("the job is admitted");
    assert_eq!(pool.available_permits(), free - 1);
    receiver
        .catchup
        .queue_overlay_for_test(capture, ownership, target);

    let work = receiver
        .detach(&mut server)
        .expect("the overlay is selected");
    assert!(matches!(work, StudioBackgroundJob::OverlayPlan(..)));
    let result = work.run(None).await;
    assert!(
        matches!(&result, StudioBackgroundResult::OverlayPlanned(_, Err(_))),
        "the fixture did not produce a real planning refusal, so this proves nothing"
    );

    // The ordinary completion path, and then NO second Save visit.
    receiver.complete(&mut server, result);
    assert!(
        receiver.catchup.overlay_planned.is_none(),
        "a plan that can never be committed was parked"
    );
    assert!(
        receiver.catchup.overlay_admission_available_for_test(),
        "a refused plan kept this actor's admission"
    );
    assert_eq!(
        pool.available_permits(),
        free,
        "a refused plan kept a slot in the shared preparation pool"
    );
    // And the actor can immediately start new overlay work.
    assert!(receiver.catchup.reserve_overlay().is_some());
}

/// RT-002. S2 is a heavy stage, so 7.3's placement rule applies: authoritative catch-up work is
/// selected first and the overlay plan waits for `replay_ready()`. The design accepts that overlay
/// work may starve under sustained catch-up (L7); the reverse was never accepted.
#[tokio::test]
async fn a_queued_overlay_waits_behind_authoritative_catch_up() {
    let clock = ManualClock::new(1000);
    let mut rng = ChaCha20Rng::seed_from_u64(311);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"priority", &mut rng).unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::save_studio_source_fixture(&mut store, 83, g, d, target)
    });
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            83,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    // Displace the sole graph so the owner turn parks a real source preparation.
    let other = StudioTarget::Flipnote {
        channel: target.channel(),
        object: [8; 16],
    };
    server.sync.with_registry_context(|g, d, _, _| {
        crate::store::save_studio_source_fixture(&mut store, 83, g, d, other)
    });
    server
        .studio_transaction(&mut store, 83, StudioRequest::Read { target: other })
        .unwrap();
    receiver.run(&mut server, &mut store, 83, None).unwrap();
    assert!(
        receiver.catchup.preparation.is_some(),
        "the fixture did not park a real source preparation"
    );
    assert!(!receiver.catchup.replay_ready());

    // A local Save reserves and queues its capture while that authoritative work is waiting.
    // `reserve_overlay` deliberately still succeeds: 7.2 reserves before the first bounded read so
    // that classification and the terminal S1a acknowledgement are not deferred by catch-up.
    let ownership = receiver
        .catchup
        .reserve_overlay()
        .expect("classification must not be blocked by catch-up");
    // A third document, so the capture fixture builds its own source without colliding with the
    // two the catch-up fixtures above already wrote.
    let saving = StudioTarget::Flipnote {
        channel: target.channel(),
        object: [7; 16],
    };
    let capture = server.sync.with_registry_context(|g, d, _, _| {
        crate::store::studio_closing_capture_fixture(&mut store, 83, g, d, saving, false)
    });
    receiver
        .catchup
        .queue_overlay_for_test(capture, ownership, saving);

    // The next detached turn must choose the authoritative preparation, not the overlay.
    let work = receiver.detach(&mut server).expect("some work is selected");
    assert!(
        matches!(work, StudioBackgroundJob::Prepare(..)),
        "an overlay plan overtook parked authoritative source preparation"
    );
    assert!(
        receiver.catchup.overlay.is_some(),
        "the overlay capture was consumed by a turn that should have deferred it"
    );

    // Once the authoritative work completes, the overlay becomes selectable.
    receiver.complete(&mut server, work.run(None).await);
    receiver.catchup.prepared = None;
    assert!(receiver.catchup.replay_ready());
    assert!(
        matches!(
            receiver.detach(&mut server),
            Some(StudioBackgroundJob::OverlayPlan(..))
        ),
        "the overlay stayed deferred after catch-up cleared"
    );
}
