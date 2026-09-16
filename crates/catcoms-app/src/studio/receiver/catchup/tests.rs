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
    let pool = receiver.catchup.overlay_pool();
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
