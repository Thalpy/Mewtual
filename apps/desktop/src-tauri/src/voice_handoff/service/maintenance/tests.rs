use super::*;
use crate::voice_activation_tests::Fixture;
use crate::voice_handoff::factory::tests::{channel_route, park, selection};
use crate::voice_handoff::ledger::{Begin, Purpose};
use catcoms_sync::voice_v2::presence::PairSelector;
use std::sync::Arc;

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
async fn pending_rotation_retries_every_original_across_interleaved_discovery_turns() {
    for count in [2u8, 4] {
        let f = Fixture::new().await;
        let context = Context::new(&f.state);
        let (_, selected, original) = selection(&f).await;
        let route = channel_route(&f, &selected).await;
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
