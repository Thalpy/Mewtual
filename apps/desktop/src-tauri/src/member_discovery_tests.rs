// Included beside the native persistence worker regressions: real actor events and sealed store.
#[tokio::test]
async fn detached_member_maintenance_quiet_legacy_pex_is_saved_before_another_tick_or_close() {
    use catcoms_rt::Hub;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    const SERVER: u64 = 181;
    const INSTANCE: u64 = 182;
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(AppState::default());
    *state.store.lock().await = Some(ServerStore::open(directory.path(), b"correct horse", &mut OsCryptoRng).unwrap());
    // Networking and sealed persistence continue while UI access is locked.
    state.session_lock_requested.store(true, Ordering::Release);
    let hub = Hub::new();
    let sender_lib_peer = libp2p::identity::Keypair::ed25519_from_bytes([181; 32]).unwrap().public().to_peer_id();
    let sender_peer = catcoms_net::phase0_peer_id(&sender_lib_peer);
    let device = MlsDevice::generate().unwrap();
    let group = catcoms_mls::ServerGroup::create(&device).unwrap();
    // The public sync constructor makes an actual unpinned legacy group, avoiding any policy
    // edits or synthetic peer records in the observation path under test.
    let mut legacy = catcoms_sync::ChannelSync::new(hub.join(sender_peer), group, device, ChaCha20Rng::seed_from_u64(181), Box::new(ManualClock::new(1_000)));
    let original = legacy.snapshot().unwrap();
    drop(legacy);
    let mut sender_server = Server::restore(&original, hub.join(sender_peer), ChaCha20Rng::seed_from_u64(181), Box::new(ManualClock::new(1_000)), "sender").unwrap();
    assert_eq!(sender_server.group_mode(), catcoms_app::GroupMode::LegacyUnverified);
    sender_server.subscribe_control().await.unwrap();
    let sender_device = sender_server.device_id();
    let invite = sender_server.mint_invite([181; 16], u64::MAX, vec![]).unwrap();
    let (sender, mut sender_events, sender_task) = spawn(sender_server);
    let sender_drain = tokio::spawn(async move { while sender_events.recv().await.is_some() {} });
    let receiver_server = Server::join(hub.join(PeerId::from_u64(182)), MlsDevice::generate().unwrap(), ChaCha20Rng::seed_from_u64(182), Box::new(ManualClock::new(1_000)), "receiver", sender_peer, &invite).await.unwrap();
    assert_eq!(receiver_server.group_mode(), catcoms_app::GroupMode::LegacyUnverified);
    let group_id = receiver_server.group_id();
    let receiver_device = receiver_server.device_id();
    let (receiver, mut events, receiver_task) = spawn(receiver_server);
    state.servers.lock().await.insert(SERVER, snapshot_test_entry(INSTANCE, receiver.clone(), group_id, receiver_device));
    let wake = install_persistence_signal(&state, SERVER, INSTANCE).await.unwrap();
    let worker_state = state.clone();
    let worker = tokio::spawn(async move { run_persistence_worker(&worker_state, SERVER, INSTANCE, wake).await; });
    let event_state = state.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            note_event_for_persistence(&event_state, SERVER, INSTANCE, &event.event).await;
        }
    });
    wait_for_clean_snapshot(&state, SERVER).await;
    let baseline = state.servers.lock().await[&SERVER].persist.completed;
    sender.publish_self_record(vec![format!("/ip4/203.0.113.1/tcp/22487/p2p/{sender_lib_peer}")], 1).await;
    receiver.drive_discovery().await.unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            if receiver.member_transport_peer(sender_device).await == Some(sender_peer)
                && state.servers.lock().await[&SERVER].persist.completed > baseline { break; }
            SystemClock.sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("late PEX record must wake sealed core persistence without another discovery tick");
    wait_for_clean_snapshot(&state, SERVER).await;
    // Read the saved snapshot BEFORE orderly close can checkpoint it as a side effect.
    let saved = state.store.lock().await.as_ref().unwrap().load_server(SERVER).unwrap();
    let mut offline = Server::restore(&saved, Hub::new().join(PeerId::from_u64(183)), ChaCha20Rng::seed_from_u64(183), Box::new(ManualClock::new(2_000)), "receiver").unwrap();
    let descriptor = offline.sync().known_peer_records().into_iter().find(|record| record.peer_id == *sender_peer.as_bytes()).expect("signed descriptor was durably saved");
    assert!(descriptor.verify_self());
    assert_eq!(DeviceId::from_public_key_bytes(&descriptor.device_pubkey), sender_device);
    assert!(offline.cache_known_records() > 0, "offline restore rebuilds the passive address cache from sealed descriptors");
    assert!(require_unlocked_session(&state).await.is_err());
    sender.shutdown().await; sender_task.await.unwrap(); sender_drain.await.unwrap();
    receiver.shutdown().await; receiver_task.await.unwrap(); forwarder.await.unwrap(); worker.await.unwrap();
}
