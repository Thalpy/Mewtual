use super::*;
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use catcoms_sync::ChannelSync;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

async fn bound_pass() -> (
    Server<MemNetwork, ChaCha20Rng>,
    ServerRegistryReceive,
    ManualClock,
) {
    let hub = Hub::new();
    let clock = ManualClock::new(1000);
    let mut server = Server::found(
        hub.join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(1002),
        Box::new(clock.clone()),
        "local",
    )
    .unwrap();
    let mut remote = Server::restore(
        &server.snapshot().unwrap(),
        hub.join(PeerId::from_u64(2)),
        ChaCha20Rng::seed_from_u64(1003),
        Box::new(clock.clone()),
        "copy",
    )
    .unwrap();
    let peer = remote.local_peer();
    // This same-device fixture tests local pass bounds, not admission of a second identity.
    // The separate joined-member integration tests provide that coverage.
    let (proof, tick) = tokio::join!(
        server
            .sync
            .request_catchup(peer, catcoms_wire::DocType::Wiki, 43),
        remote.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    let watch = server.sync.watch_registry(0, 42);
    let requester = server
        .sync
        .with_registry_context(|_, device, _, _| device.device_id());
    let pass = ServerRegistryReceive {
        permit: server.sync.begin_registry_receive(&watch).unwrap(),
        mount: Arc::new(()),
        server: 1,
        bucket: 0,
        requester,
        peer,
        provider: requester,
        frontier: RegistryFrontier {
            heads: vec![],
            seed: None,
        },
        empty_fallback_used: false,
        cursor: None,
        pending: None,
        pending_epoch: 0,
        state: RegistryReceiveState::Ready,
        progress: Default::default(),
        now: 1000,
        expires: 601_000,
        retry_at: 1000,
        write_at: 1000,
    };
    (server, pass, clock)
}

#[tokio::test]
async fn registry_receiver_bound_provider_and_attempt_limits_precede_network_work() {
    let (mut server, mut pass, clock) = bound_pass().await;
    pass.progress.attempts = MAX_EPOCH_OPERATIONS + 1;
    assert_eq!(
        server.fetch_registry_receive_step(&mut pass).await.unwrap(),
        RegistryReceiveState::RestartRequired
    );
    assert_eq!(pass.progress.attempts, MAX_EPOCH_OPERATIONS + 1);
    // A fixed lifetime cannot be extended by wall-clock changes or explicit retry.
    pass.state = RegistryReceiveState::Paused;
    pass.progress.attempts = 0;
    clock.set_wall_ms(0);
    clock.advance_ms(PASS_LIFETIME_MS);
    pass.retry();
    assert_eq!(
        server.fetch_registry_receive_step(&mut pass).await.unwrap(),
        RegistryReceiveState::RestartRequired
    );
    assert_eq!(pass.progress.attempts, 0);

    let (mut server, mut pass, _) = bound_pass().await;
    // Model the retained old full provider identity after the endpoint proof changes. A valid
    // proof for a different key must not silently retarget this pass's private cursor.
    pass.provider = MlsDevice::generate().unwrap().device_id();
    assert!(server.fetch_registry_receive_step(&mut pass).await.is_err());
    assert_eq!(pass.state(), RegistryReceiveState::Stopped);
    assert_eq!(pass.progress.attempts, 0);
}

#[tokio::test]
async fn registry_receiver_unknown_frontier_falls_back_once_without_refunding_work() {
    let (_, mut pass, _) = bound_pass().await;
    pass.frontier = RegistryFrontier {
        heads: vec![[9; 32]],
        seed: Some([7; 32]),
    };
    pass.progress.attempts = 1;
    pass.retry_at = 2000;
    let before = (pass.progress(), pass.expires, pass.retry_at, pass.provider);
    pass.receive_restart();
    assert_eq!(pass.state, RegistryReceiveState::Ready);
    assert!(pass.frontier.heads.is_empty());
    assert_eq!(pass.frontier.seed, Some([7; 32]));
    assert_eq!(
        (pass.progress(), pass.expires, pass.retry_at, pass.provider),
        before
    );
    pass.receive_restart();
    assert_eq!(pass.state, RegistryReceiveState::RestartRequired);
    pass.retry();
    assert_eq!(pass.state, RegistryReceiveState::RestartRequired);

    // A provider restart after a page cannot reinterpret its continuation under new heads.
    let (_, mut pass, _) = bound_pass().await;
    pass.frontier.heads = vec![[9; 32]];
    pass.progress.received_pages = 1;
    pass.receive_restart();
    assert_eq!(pass.state, RegistryReceiveState::RestartRequired);
    assert_eq!(pass.frontier.heads, vec![[9; 32]]);
}

#[test]
fn registry_receiver_retry_never_reopens_a_terminal_or_held_state() {
    let device = MlsDevice::generate().unwrap();
    let identity = device.device_id();
    let group = ServerGroup::create(&device).unwrap();
    let mut sync = ChannelSync::new(
        Hub::new().join(PeerId::from_u64(1)),
        group,
        device,
        ChaCha20Rng::seed_from_u64(1001),
        Box::new(ManualClock::new(1)),
    );
    let watch = sync.watch_registry(0, 42);
    for state in [
        RegistryReceiveState::Ready,
        RegistryReceiveState::PageReady,
        RegistryReceiveState::Paused,
        RegistryReceiveState::PrefixComplete,
        RegistryReceiveState::RestartRequired,
        RegistryReceiveState::CheckpointRequired,
        RegistryReceiveState::HistoricalAuthorizationRequired,
        RegistryReceiveState::Stopped,
    ] {
        for has_page in [false, true] {
            // Only retry's pure state transition is under test. Network authority and real saved
            // frontiers are exercised by the joined-member tests, not fabricated by this setup.
            let mut pass = ServerRegistryReceive {
                permit: sync.begin_registry_receive(&watch).unwrap(),
                mount: Arc::new(()),
                server: 1,
                bucket: 0,
                requester: identity,
                peer: PeerId::from_u64(2),
                provider: identity,
                frontier: RegistryFrontier {
                    heads: vec![],
                    seed: None,
                },
                empty_fallback_used: false,
                cursor: None,
                pending: has_page.then(|| RegistryOpPage {
                    operations: vec![],
                    next: None,
                }),
                pending_epoch: 0,
                state,
                progress: RegistryReceiveProgress {
                    attempts: 7,
                    persist_attempts: 3,
                    ..Default::default()
                },
                now: 100,
                expires: 600,
                retry_at: 500,
                write_at: 400,
            };
            let before = pass.progress();
            pass.retry();
            assert_eq!(
                pass.state(),
                if state == RegistryReceiveState::Paused {
                    if has_page {
                        RegistryReceiveState::PageReady
                    } else {
                        RegistryReceiveState::Ready
                    }
                } else {
                    state
                }
            );
            assert_eq!(pass.progress(), before);
            assert_eq!(
                (pass.expires, pass.retry_at, pass.write_at),
                (600, 500, 400)
            );
            assert_eq!(pass.pending.is_some(), has_page);
        }
    }
}
