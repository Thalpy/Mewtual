use super::*;
use crate::voice_activation_tests::Fixture;
use crate::voice_handoff::factory::tests::{channel_route, park, selection};
use crate::voice_handoff::factory::Refused as NativeRefused;
use crate::voice_handoff::ledger::{Begin, Purpose};
use catcoms_sync::voice_v2::presence::PairSelector;
use std::sync::Arc;
mod recovery;

#[tokio::test]
async fn discovery_cursor_rotates_after_repeated_workers_refuse_before_peer_snapshot() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_, selected, selector) = selection(&f).await;
    let route = channel_route(&f, &selected).await;
    route.acknowledge_setup().unwrap();
    let mut cursor = Cursor::new();
    for peer in 0..3 {
        let servers = f.state.servers.lock().await;
        let stage = cursor.stage[0];
        let (admitted, _, waiter) = cursor.ordinary_work(&context, &route, 0);
        assert!(admitted);
        assert_eq!(cursor.peers[0], peer + 1);
        assert_eq!(cursor.stage[0], (stage + 1) % 4);
        let completed = bounded_native_outcome(waiter.unwrap()).await;
        assert!(completed.refused_before_selection(peer));
        assert!(!completed.refused_before_selection(peer + 1));
        drop(completed);
        assert!(f
            .state
            .voice_handoff
            .lookup_original(&selected, Purpose::Derive(selector))
            .is_none());
        drop(servers);
        let (admitted, _, waiter) = cursor.ordinary_work(&context, &route, 0);
        assert!(admitted);
        let completed = bounded_native_outcome(waiter.unwrap()).await;
        assert!(!completed.refused_before_selection(peer));
        drop(completed);
        assert_eq!(cursor.stage[0], (stage + 2) % 4);
        assert_eq!(cursor.peers[0], peer + 1);
    }
    f.close().await;
}

/// Only the test service's first cadence sleep is held. The actual native Runtime Clock
/// still advances; proof/attempt/deadline authority is never paused or renewed. Install
/// before start and wait for entered BEFORE launching any browser/HTTP client, so that
/// this first sleep belongs to maintenance rather than a connection timeout.
#[derive(Debug)]
struct DeferredFirstSleep {
    original: Arc<dyn Clock>,
    receiver: std::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    entered: Arc<std::sync::atomic::AtomicBool>,
}
impl Clock for DeferredFirstSleep {
    fn now_ms(&self) -> u64 {
        self.original.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        self.original.monotonic_ms()
    }
    fn sleep(
        &self,
        duration: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        let receiver = self.receiver.lock().unwrap().take();
        if let Some(receiver) = receiver {
            assert!(
                duration <= Duration::from_millis(CADENCE_MS),
                "first test sleep must be maintenance"
            );
            self.entered
                .store(true, std::sync::atomic::Ordering::Release);
            Box::pin(async move {
                let _ = receiver.await;
                self.original.sleep(duration).await;
            })
        } else {
            self.original.sleep(duration)
        }
    }
}
pub(in crate::voice_handoff) fn defer_first_maintenance(
    bound: &mut BoundServer,
) -> (
    Arc<std::sync::atomic::AtomicBool>,
    tokio::sync::oneshot::Sender<()>,
) {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    bound.clock = Arc::new(DeferredFirstSleep {
        original: bound.clock.clone(),
        receiver: std::sync::Mutex::new(Some(receiver)),
        entered: entered.clone(),
    });
    (entered, sender)
}

/// Timeout only this observer: the worker remains owned by its original task/Session.
/// In particular, unwinding drops a held map before Endpoint.collect orders End and joins
/// actual custody. A regression to a blocking map must fail promptly, not hang the harness.
async fn bounded_native_outcome(waiter: native::NativeWaiter) -> native::NativeCompletion {
    tokio::time::timeout(Duration::from_secs(10), waiter)
        .await
        .expect("actual native discovery worker exceeded its original fixture bound")
        .expect("original native worker task panicked")
}

/// Exercise the SAME production Cursor across genuine local refusals and its ordinary
/// continuation/discovery turns. Refuse even discovery cursors with the actual Servers
/// guard, so only the alternate peer can be the eventual positive in this two-peer
/// fixture. Also refuse the first odd turn, deterministically exercising consecutive
/// local refusals. That extra refusal must not silently change the positive oracle.
/// No supplied selector, cursor reset, new intent or fabricated grant enters a producer.
pub(in crate::voice_handoff) async fn discovery_after_local_refusal(
    context: &Context,
    route: &Route,
    clock: &dyn Clock,
    force_attachment: bool,
) {
    assert!(
        route.native_ready(),
        "actual setup ACK must precede automatic production"
    );
    let started = clock.monotonic_ms();
    let deadline = started
        .checked_add(10_000)
        .expect("fixture observer deadline");
    let mut cursor = Cursor::new();
    cursor.fixture_observe = true;
    cursor.fixture_attachment_pressure = force_attachment;
    let index = usize::from(route.fixture_slot() - 1);
    let progression = async {
        let mut recovery: Option<recovery::Recovery> = None;
        let mut pressured = false;
        // A genuine successful Discover followed by a later local refusal must not
        // acquire the pre-selection marker used below. This barrier creates no row
        // or protocol effect, so the SAME normal Cursor can still start afterwards.
        native::prove_discovery_late_refusal(context, route, clock).await;
        loop {
            clock.sleep(Duration::from_millis(CADENCE_MS)).await;
            let stage = cursor.stage[index];
            let peer = cursor.peers[index];
            let discovery = matches!(stage, 0 | 2);
            let live = context.upgrade().expect("original mounted context");
            if let Some(original) = &mut recovery {
                assert!(
                    !original.observe(route, clock).await,
                    "only a completed exact turn may publish success"
                );
            }
            let servers =
                if recovery.is_none() && discovery && (peer.is_multiple_of(2) || peer == 1) {
                    Some(live.servers.lock().await)
                } else {
                    None
                };
            let (admitted, selected, waiter) = cursor.ordinary_work(context, route, index);
            if !admitted {
                // Admission refusal does not advance either cursor. The SAME finite
                // observer bounds these ordinary busy turns; no authority is refreshed.
                assert_eq!((cursor.stage[index], cursor.peers[index]), (stage, peer));
                continue;
            }
            if let Some(original) = &recovery {
                if let Some(selected) = selected {
                    assert_eq!(selected, original.selector);
                }
            } else {
                assert!(
                    selected.is_none(),
                    "a new original Pending appeared unexpectedly"
                );
            }
            let completed = waiter
                .unwrap()
                .await
                .expect("original native worker panicked");
            let witness = completed.fixture_witness();
            let observed = witness
                .as_ref()
                .map(|witness| witness.observation())
                .unwrap_or_default();
            if let Some(original) = &mut recovery {
                original.worker(observed);
                let attached = original.observe(route, clock).await;
                if observed.pressured {
                    assert_eq!(
                        completed.result(),
                        Err(NativeRefused::Protocol(
                            catcoms_sync::voice_v2::VoiceError::Capacity
                        ))
                    );
                    assert_eq!(observed.stage, 3, "actual attach/try_bind boundary");
                    assert!(observed.action.is_some() && !attached);
                    pressured = true;
                    cursor.fixture_attachment_pressure = false;
                    witness.as_ref().unwrap().release_pressure();
                }
                match completed.result() {
                    Ok(id) => {
                        assert_eq!(
                            id,
                            original.id(),
                            "no replacement parent/child may satisfy recovery"
                        );
                        if attached {
                            assert!(
                                !force_attachment || pressured,
                                "real attachment Capacity was exercised"
                            );
                            drop(completed);
                            return;
                        }
                    }
                    Err(NativeRefused::Protocol(catcoms_sync::voice_v2::VoiceError::Capacity)) => {
                        assert!(!attached, "Capacity is not current attached success")
                    }
                    Err(error) => panic!(
                        "same original discovery recovery failed: {error:?}; stage {}",
                        observed.stage
                    ),
                }
                drop(completed);
                drop(servers);
                continue; // SAME Cursor; no parity pressure or pre-selection oracle after Begin
            }
            if !discovery {
                drop(completed); // ordinary confirmation; no selector is supplied
                continue;
            }
            assert_eq!(cursor.peers[index], (peer + 1) % 64);
            assert_eq!(cursor.stage[index], (stage + 1) % 4);
            if !peer.is_multiple_of(2) && peer != 1 && completed.result().is_ok() {
                assert!(
                    !force_attachment,
                    "forced attachment must refuse before this positive"
                );
                let original =
                    recovery::Recovery::capture(context, route, observed, clock, false).await;
                assert_eq!(
                    completed.result(),
                    Ok(original.id()),
                    "positive is exact attached registration"
                );
                drop(completed);
                return;
            }
            if completed.result()
                == Err(NativeRefused::Protocol(
                    catcoms_sync::voice_v2::VoiceError::Capacity,
                ))
                && !completed.refused_before_selection(peer)
            {
                assert!(
                    servers.is_none(),
                    "forced pre-snapshot guard cannot enter Root"
                );
                let original =
                    recovery::Recovery::capture(context, route, observed, clock, true).await;
                if observed.pressured {
                    assert_eq!(observed.stage, 3);
                    assert!(observed.action.is_some());
                    pressured = true;
                    cursor.fixture_attachment_pressure = false;
                    witness.as_ref().unwrap().release_pressure();
                }
                recovery = Some(original);
                drop(completed); // actual original NativeHold/payment before ordinary continuation
                continue;
            }
            assert!(
                completed.refused_before_selection(peer),
                "native discovery failed beyond its proven pre-selection boundary: {:?}; cursor/stage {peer}/{stage} -> {}/{}; elapsed/remaining {}/{}ms; {}",
                completed.result(), cursor.peers[index], cursor.stage[index],
                clock.monotonic_ms().saturating_sub(started),
                deadline.saturating_sub(clock.monotonic_ms()),
                route.session().fixture_native_snapshot().describe_last_native()
            );
            drop(completed); // actual NativeHold and8KiB payment retire before another turn
            drop(servers);
            loop {
                match live.ledger.automatic_parent_available(route.selected()) {
                    Ok(true) => break,
                    Err(catcoms_sync::voice_v2::VoiceError::Capacity) => {
                        clock.sleep(Duration::from_millis(10)).await;
                    }
                    result => panic!("pre-selection refusal created a parent: {result:?}"),
                }
            }
        }
    };
    // The explicitly harness-owned recovery/capsule graph is separate from the
    // native8KiB payer. Its optional Arc handles still count in native layout().
    let harness_bytes =
        std::mem::size_of_val(&progression) + 2 * native::fixture::Witness::layout_bytes();
    eprintln!("exact discovery recovery harness graph {harness_bytes}/16384");
    assert!(
        harness_bytes <= 16 * 1024,
        "complete bounded recovery harness frame/capsule overlap"
    );
    tokio::select! {
        biased;
        _ = super::super::expire_at(clock, deadline) => panic!(
            "native alternate discovery exceeded its original10s observer bound; elapsed={}ms; {}",
            clock.monotonic_ms().saturating_sub(started),
            route.session().fixture_native_snapshot().describe_last_native()
        ),
        _ = progression => {},
    }
}

#[tokio::test]
async fn exhausted_monotonic_time_disables_maintenance_without_blocking_host_stop() {
    // A guard bounds a regression inside a single poll: timeout alone cannot interrupt an
    // accidentally always-ready async loop. The actual elapsed/wall values use ManualClock.
    #[derive(Debug)]
    struct Exhausted {
        clock: catcoms_rt::ManualClock,
        samples: std::sync::atomic::AtomicUsize,
    }
    impl Clock for Exhausted {
        fn now_ms(&self) -> u64 {
            self.clock.now_ms()
        }
        fn monotonic_ms(&self) -> u64 {
            assert!(
                self.samples
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    < 8,
                "maintenance spun at exhausted time instead of remaining pollable"
            );
            self.clock.monotonic_ms()
        }
        fn sleep(
            &self,
            duration: Duration,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
            self.clock.sleep(duration)
        }
    }
    let clock = Arc::new(Exhausted {
        clock: catcoms_rt::ManualClock::new(u64::MAX),
        samples: std::sync::atomic::AtomicUsize::new(0),
    });
    let runtime = catcoms_sync::voice_v2::runtime::Runtime::new(clock.clone());
    let state = crate::AppState::default();
    let registry = state.voice_channels.clone();
    let bound = BoundServer::bind_application(
        registry,
        runtime,
        Origin::PackagedHttp,
        Context::new(&state),
    )
    .unwrap();
    // Runtime's EffectiveClock samples its initial anchor during construction. The
    // assertion below covers only the running maintenance/IO lifecycle, not that setup.
    clock.samples.store(0, std::sync::atomic::Ordering::Relaxed);
    let running = bound.start().unwrap();
    // The actual server starts before Stop is sent; exhausted cadence must have no turns.
    tokio::time::timeout(Duration::from_secs(2), async {
        while running.phase.load(std::sync::atomic::Ordering::Acquire) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(
        tokio::time::timeout(Duration::from_secs(2), running.finish())
            .await
            .unwrap()
            .unwrap(),
    );
    assert!(clock.samples.load(std::sync::atomic::Ordering::Relaxed) <= 1);
}

async fn idle(route: &Route) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if route.session().native_hold().is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("original native worker did not drain");
}

#[tokio::test]
async fn continuously_observed_ready_hint_cannot_preempt_or_discard_original_ordinary_workers() {
    for count in [2u8, 4] {
        let f = Fixture::new().await;
        let context = Context::new(&f.state);
        let (_, selected, original) = selection(&f).await;
        let route = channel_route(&f, &selected).await;
        route.acknowledge_setup().unwrap();
        let mut rows = Vec::new();
        for number in 0..count {
            let selector = PairSelector {
                join: [number + 20; 32],
                ..original
            };
            let Begin::Factory(registration) = f
                .state
                .voice_handoff
                .begin(selected.clone(), Purpose::Derive(selector))
                .unwrap()
            else {
                panic!("original Pending");
            };
            registration.mark_handoff().unwrap();
            rows.push((selector, registration.id()));
            drop(registration);
        }
        let mut cursor = Cursor::new();
        let mut seen = Vec::new();
        for _ in 0..count {
            // A stale Ready observation is not authorization: the actual Ready worker
            // finds no KeyClaim and refuses, but it REALLY owns this channel's NativeHold.
            assert!(cursor
                .turn_observing_ready(&context, &f.state.voice_channels, |_| true)
                .is_none());
            assert!(matches!(
                route.session().native_hold(),
                Err(crate::voice_handoff::channel::Refused::Capacity)
            ));
            let proposed = cursor.pending[0];
            // Hold that exact unpolled Ready task across the ordinary visit. A refusal
            // must preserve BOTH the stage and Pending cursor, not discard row0 each time
            // and admit only row1 when the position becomes free.
            assert!(cursor
                .turn_observing_ready(&context, &f.state.voice_channels, |_| true)
                .is_none());
            assert!(!cursor.ready_first[0]);
            assert_eq!(cursor.stage[0], 0);
            assert_eq!(cursor.pending[0], proposed);
            assert!(cursor.ready_first[1..].iter().all(|priority| *priority));
            idle(&route).await;
            let selector = cursor
                .turn_observing_ready(&context, &f.state.voice_channels, |_| true)
                .expect("the next free position must admit the original ordinary Pending");
            assert!(
                !seen.contains(&selector),
                "an earlier original was selected/discarded during Ready contention"
            );
            seen.push(selector);
            assert!(matches!(
                route.session().native_hold(),
                Err(crate::voice_handoff::channel::Refused::Capacity)
            ));
            cursor.urgent_ready(&context, &f.state.voice_channels);
            idle(&route).await;
            // Complete the SAME ordinary stage rotation; no artificial stage/cursor reset.
            for _ in 0..3 {
                assert!(cursor
                    .turn_observing_ready(&context, &f.state.voice_channels, |_| false)
                    .is_none());
                idle(&route).await;
            }
        }
        assert_eq!(seen.len(), usize::from(count));
        for (selector, id) in rows {
            let row = f
                .state
                .voice_handoff
                .lookup_original(&selected, Purpose::Derive(selector))
                .unwrap();
            assert!(
                row.id() == id,
                "no replacement Row/deadline can make this test pass"
            );
            assert!(seen.contains(&selector));
            row.close_pending().unwrap();
        }
        f.close().await;
    }
}

/// Production calls the ordinary turn and urgent scan without yielding between them.
/// A busy refusal MUST preserve Ready's next turn as well as the opposite ordinary debt.
#[tokio::test]
async fn refused_ready_visit_cannot_surrender_its_next_free_native_position_to_ordinary_work() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_, selected, _) = selection(&f).await;
    let route = channel_route(&f, &selected).await;
    route.acknowledge_setup().unwrap();
    let mut cursor = Cursor::new();
    for stage in 0..4 {
        // Model Ready becoming visible just after this actual ordinary admission, as
        // happens when a KeyClaim is adopted between the two production scans.
        let _ = cursor.turn_observing_ready(&context, &f.state.voice_channels, |_| false);
        assert!(route.session().native_hold().is_err());
        assert!(cursor.ready_first[0]);
        let ready_row = cursor.ready_rows[0];
        assert_eq!(cursor.stage[0], (stage + 1) % 4);
        let _ = cursor.turn_observing_ready(&context, &f.state.voice_channels, |_| true);
        assert!(
            cursor.ready_first[0],
            "a refused Ready did not consume a real turn"
        );
        assert_eq!(cursor.ready_rows[0], ready_row);
        idle(&route).await;
        let _ = cursor.turn_observing_ready(&context, &f.state.voice_channels, |_| true);
        assert!(
            !cursor.ready_first[0],
            "Ready must acquire the next free position"
        );
        assert!(route.session().native_hold().is_err());
        assert_eq!(
            cursor.stage[0],
            (stage + 1) % 4,
            "no ordinary worker jumped ahead"
        );
        assert_eq!(cursor.ready_rows[0], (ready_row + 1) % 32);
        idle(&route).await;
    }
    f.close().await;
}

#[tokio::test]
async fn pending_rotation_retries_every_original_across_interleaved_discovery_turns() {
    for count in [2u8, 4] {
        let f = Fixture::new().await;
        let context = Context::new(&f.state);
        let (_, selected, original) = selection(&f).await;
        let route =
            crate::voice_handoff::factory::tests::unacknowledged_channel_route(&f, &selected).await;
        let mut rows = Vec::new();
        for number in 0..count {
            let selector = PairSelector {
                join: [number + 20; 32],
                ..original
            };
            let Begin::Factory(registration) = f
                .state
                .voice_handoff
                .begin(selected.clone(), Purpose::Derive(selector))
                .unwrap()
            else {
                panic!("new original Pending")
            };
            registration.mark_handoff().unwrap();
            rows.push((selector, registration.id()));
            drop(registration); // uncertain publication, with no fresh proven peer at all
        }
        let mut cursor = Cursor::new();
        let before = selected.resources().counts();
        assert!(cursor.turn(&context, &f.state.voice_channels).is_none());
        assert_eq!(
            selected.resources().counts(),
            before,
            "Ready alone cannot start native work before setup ACK"
        );
        route.acknowledge_setup().unwrap(); // fixture models completed original browser setup
        let mut seen = Vec::new();
        for _ in 0..count {
            let selector = cursor
                .turn(&context, &f.state.voice_channels)
                .expect("exact Pending must spawn");
            assert!(
                !seen.contains(&selector),
                "a discarded discovery selection starved another Pending"
            );
            seen.push(selector);
            idle(&route).await;
            for _ in 0..3 {
                assert!(
                    cursor.turn(&context, &f.state.voice_channels).is_none(),
                    "confirmation/discovery is not an exact-Pending retry"
                );
                idle(&route).await;
            }
        }
        assert_eq!(seen.len(), usize::from(count));
        for (selector, id) in rows {
            let row = f
                .state
                .voice_handoff
                .lookup_original(&selected, Purpose::Derive(selector))
                .unwrap();
            assert_eq!(
                row.id(),
                id,
                "no new purpose/serial/deadline replaces an original retry"
            );
            assert!(seen.contains(&selector));
            row.close_pending().unwrap();
        }
        f.close().await;
    }
}

#[tokio::test]
async fn parked_old_discovery_cannot_block_end_replacement_or_another_server() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_, selected, _) = selection(&f).await;
    let route = channel_route(&f, &selected).await;
    route.acknowledge_setup().unwrap();
    let grant = selected.grant().unwrap();
    // Install a genuine second Actor/group on the SAME process Runtime/Context, before
    // parking the old one. No foreign Registry or pool can stand in for native progress.
    let runtime = crate::voice_runtime(&f.state);
    let ticket = runtime.session_ticket();
    // This Actor has never belonged to another native Runtime. Reattaching an already
    // registered foreign-process fixture is correctly unauthorized and is not a substitute
    // for testing two groups in one process.
    use rand_core::SeedableRng;
    let server = catcoms_app::Server::found(
        catcoms_rt::Hub::new().join(catcoms_rt::PeerId::from_u64(8)),
        catcoms_mls::MlsDevice::generate().unwrap(),
        rand_chacha::ChaCha20Rng::seed_from_u64(718),
        Box::new(catcoms_rt::ManualClock::new(1000)),
        "other",
    )
    .unwrap();
    let group = server.group_id();
    let device = server.device_id();
    let (other, mut events, task) = catcoms_app::spawn(server);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let registration = other
        .prepare_voice_actor(runtime.clone(), ticket)
        .await
        .unwrap();
    registration.register().unwrap();
    let mut entry = crate::tests::snapshot_test_entry(1, other.clone(), group, device);
    entry.voice = Some(crate::RegisteredVoiceActor(registration));
    f.state.servers.lock().await.insert(8, entry);
    let parked = park(&f.actor).await;
    let owners = Arc::strong_count(&grant);
    let mut cursor = Cursor::new();
    assert!(cursor.turn(&context, &f.state.voice_channels).is_none());
    tokio::time::timeout(Duration::from_secs(15), async {
        while Arc::strong_count(&grant) <= owners {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("old native discovery never reached its Actor query");
    assert!(route.session().native_hold().is_err());
    // Production End publishes revocation before waiting for the old Actor's Stop. Keep
    // that exact waiter pending while replacement proceeds; do not substitute a native
    // Coordinator-only End for evidence about the complete bridge lifecycle.
    let mut ending = Box::pin(crate::end_voice_owner_inner(&f.state, selected.id()));
    std::future::poll_fn(|cx| {
        use std::future::Future;
        assert!(ending.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert!(!selected.lease().is_live());

    let controller = crate::prepare_voice_controller_inner(&f.state)
        .await
        .unwrap();
    crate::activate_voice_owner_inner(&f.state, controller.clone())
        .await
        .unwrap();
    let owner = crate::prepare_voice_owner_inner(
        &f.state,
        controller,
        8,
        catcoms_app::channel_id("general").to_string(),
    )
    .await
    .unwrap();
    crate::activate_voice_owner_inner(&f.state, owner.clone())
        .await
        .unwrap();
    let executor = hex::encode([10; 32]);
    let admitted = crate::voice_join_key::admit_inner(&f.state, &owner, &executor)
        .await
        .unwrap();
    let point: [u8; 65] = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap();
    crate::voice_join_key::publish_inner(&f.state, &owner, &executor, &admitted.permit, point)
        .await
        .unwrap();
    crate::voice_join_key::settle_inner(&f.state, &owner, [10; 32])
        .await
        .unwrap();
    let replacement = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .selected_handoff_owner(&owner, [10; 32])
        .unwrap();
    let replacement_route = channel_route(&f, &replacement).await;
    replacement_route.acknowledge_setup().unwrap();
    // Empty proof set is deliberately a permanent factory refusal, so create a retained
    // exact Pending to observe actual new-group native scheduling/collection independently.
    let selector = PairSelector {
        peer: catcoms_crypto::DeviceId::from_bytes([92; 32]),
        join: [8; 32],
        key_ref: [7; 32],
        epoch: 0,
    };
    let Begin::Factory(pending) = f
        .state
        .voice_handoff
        .begin(replacement.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("replacement Pending")
    };
    pending.mark_handoff().unwrap();
    let id = pending.id();
    drop(pending);
    assert_eq!(
        cursor.turn(&context, &f.state.voice_channels),
        Some(selector)
    );
    idle(&replacement_route).await;
    assert!(
        route.session().native_hold().is_err(),
        "old Actor remains parked while replacement progressed"
    );
    let row = f
        .state
        .voice_handoff
        .lookup_original(&replacement, Purpose::Derive(selector))
        .unwrap();
    assert_eq!(row.id(), id);
    row.close_pending().unwrap();
    drop(parked);
    tokio::time::timeout(Duration::from_secs(15), ending)
        .await
        .unwrap();
    idle(&route).await;
    drop(grant);
    f.close().await;
    other.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}
