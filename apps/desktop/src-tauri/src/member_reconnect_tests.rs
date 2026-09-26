mod member_reconnect_regressions {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    struct Running {
        actor: ServerActor,
        mesh: MeshHandle,
        task: tokio::task::JoinHandle<()>,
        drain: tokio::task::JoinHandle<()>,
    }

    async fn register(
        state: &AppState,
        actor: ServerActor,
        mesh: MeshHandle,
        mut events: mpsc::Receiver<catcoms_app::TracedEvent>,
        task: tokio::task::JoinHandle<()>,
        group: Vec<u8>,
        device: DeviceId,
    ) -> Running {
        let mut entry = snapshot_test_entry(1, actor.clone(), group, device);
        entry.mesh = Some(mesh.clone());
        state.servers.lock().await.insert(1, entry);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        Running {
            actor,
            mesh,
            task,
            drain,
        }
    }

    async fn mount(path: &std::path::Path) -> AppState {
        let state = AppState::default();
        *state.store.lock().await =
            Some(ServerStore::open(path, b"reconnect test", &mut OsCryptoRng).unwrap());
        state
    }

    async fn net(state: &AppState) -> ServerNet {
        state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_server_net(1)
            .unwrap()
            .unwrap()
    }

    async fn stop(state: &AppState, running: Running) {
        state.servers.lock().await.clear();
        running.actor.shutdown().await;
        running.task.await.unwrap();
        running.drain.await.unwrap();
        drop(running.mesh);
    }

    async fn restore(state: &AppState, listen: bool) -> Running {
        let net = load_or_init_server_net(state, 1, "").await;
        let snapshot = state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_server(1)
            .unwrap();
        let listeners = if listen {
            vec![format!("/ip4/127.0.0.1/tcp/{}", net.port).parse().unwrap()]
        } else {
            Vec::new()
        };
        let (mut transport, _, _) = MeshService::new_tcp_with_key(
            keypair_from_seed(net.key_seed).unwrap(),
            &listeners,
            &[],
        )
        .unwrap();
        if listen {
            timeout(Duration::from_secs(5), transport.next_listen_addr())
                .await
                .unwrap()
                .unwrap();
        }
        let mesh = transport.handle();
        let record = ServerRecord {
            id: 1,
            display_name: "member".into(),
            invite: String::new(),
            is_dm: false,
        };
        let restored = restore_server_actor(
            state,
            &snapshot,
            &record,
            transport,
            ChaCha20Rng::seed_from_u64(19),
            Box::new(catcoms_rt::SystemClock),
            &[],
            net.record_seq,
            net.reconnect_policy,
            net.reconnect_routes
                .iter()
                .map(|r| (PeerId::new(r.peer_id), r.address.clone()))
                .collect(),
            false,
        )
        .await
        .unwrap();
        register(
            state,
            restored.actor,
            mesh,
            restored.events,
            restored.task,
            restored.group_id,
            restored.device_id,
        )
        .await
    }

    #[tokio::test]
    async fn standing_reconnect_authority_requires_saved_current_core_instance() {
        let dir = tempfile::tempdir().unwrap();
        let state = mount(dir.path()).await;
        let network = new_server_net("", "", "");
        let (transport, _, _) =
            MeshService::new_tcp_with_key(keypair_from_seed(network.key_seed).unwrap(), &[], &[])
                .unwrap();
        let server = Server::found(
            catcoms_rt::Hub::new().join(transport.handle().local_peer()),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(40),
            Box::new(catcoms_rt::SystemClock),
            "member",
        )
        .unwrap();
        let group = server.group_id();
        let device = server.device_id();
        let (actor, events, task) = spawn(server);
        let running = register(
            &state,
            actor,
            transport.handle(),
            events,
            task,
            group,
            device,
        )
        .await;
        persist_server_net(&state, 1, &network).await;
        let mounted = state.store.lock().await.take().unwrap();
        assert!(
            member_reconnect::persist(&state, 1, 1, &running.actor, Vec::new())
                .await
                .is_err()
        );
        assert_eq!(
            mounted
                .load_server_net(1)
                .unwrap()
                .unwrap()
                .reconnect_policy,
            ReconnectPolicy::Disabled
        );
        *state.store.lock().await = Some(mounted);
        state.servers.lock().await.get_mut(&1).unwrap().instance = 2;
        assert!(
            member_reconnect::persist(&state, 1, 1, &running.actor, Vec::new())
                .await
                .is_err()
        );
        assert_eq!(
            net(&state).await.reconnect_policy,
            ReconnectPolicy::Disabled
        );
        assert!(
            member_reconnect::persist(&state, 1, 2, &running.actor, Vec::new())
                .await
                .unwrap()
        );
        let snapshot = state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_server(1)
            .unwrap();
        let saved = Server::restore(
            &snapshot,
            catcoms_rt::Hub::new().join(PeerId::from_u64(40)),
            ChaCha20Rng::seed_from_u64(40),
            Box::new(catcoms_rt::SystemClock),
            "member",
        )
        .unwrap();
        assert_eq!(saved.group_mode(), catcoms_app::GroupMode::PeerToPeer);
        assert_eq!(
            net(&state).await.reconnect_policy,
            ReconnectPolicy::MemberMesh
        );
        stop(&state, running).await;
    }

    #[tokio::test]
    async fn simultaneous_actor_finalization_survives_reciprocal_catchup_waits() {
        timeout(Duration::from_secs(15), async {
            let a_dir = tempfile::tempdir().unwrap();
            let b_dir = tempfile::tempdir().unwrap();
            let a_state = mount(a_dir.path()).await;
            let b_state = mount(b_dir.path()).await;
            let a_net = new_server_net("", "", "");
            let b_net = new_server_net("", "", "");
            let (a_transport, a_id, _) =
                MeshService::new_tcp_with_key(keypair_from_seed(a_net.key_seed).unwrap(), &[], &[])
                    .unwrap();
            let (b_transport, b_id, _) =
                MeshService::new_tcp_with_key(keypair_from_seed(b_net.key_seed).unwrap(), &[], &[])
                    .unwrap();
            let a_peer = a_transport.handle().local_peer();
            let b_peer = b_transport.handle().local_peer();
            let hub = catcoms_rt::Hub::new();
            let mut a = Server::found(
                hub.join(a_peer),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(31),
                Box::new(catcoms_rt::SystemClock),
                "a",
            )
            .unwrap();
            a.subscribe_control().await.unwrap();
            a.publish_self_record(Vec::new(), a_net.record_seq).unwrap();
            let invite = a
                .mint_invite(
                    [0x61; 16],
                    catcoms_rt::SystemClock.now_ms() + 60_000,
                    vec![],
                )
                .unwrap();
            let (joined, served) = tokio::join!(
                Server::join_from_reply(
                    hub.join(b_peer),
                    MlsDevice::generate().unwrap(),
                    ChaCha20Rng::seed_from_u64(32),
                    Box::new(catcoms_rt::SystemClock),
                    "b",
                    a_peer,
                    a_peer,
                    &invite,
                    [0x62; 16],
                    b"proven",
                    catcoms_rt::SystemClock.now_ms() + 60_000
                ),
                a.sync_once(),
            );
            served.unwrap();
            let (mut b, _) = joined.unwrap();
            b.publish_self_record(Vec::new(), b_net.record_seq).unwrap();
            let group = a.group_id();
            let device = a.device_id();
            let (actor, events, task) = spawn(a);
            let a = register(
                &a_state,
                actor,
                a_transport.handle(),
                events,
                task,
                group,
                device,
            )
            .await;
            let group = b.group_id();
            let device = b.device_id();
            let (actor, events, task) = spawn(b);
            let b = register(
                &b_state,
                actor,
                b_transport.handle(),
                events,
                task,
                group,
                device,
            )
            .await;
            persist_server_net(&a_state, 1, &a_net).await;
            persist_server_net(&b_state, 1, &b_net).await;
            let channel = channel_id("general");
            a.actor.open_channel(channel).await;
            b.actor.open_channel(channel).await;
            a.actor.catch_up(b_peer, channel).await;
            b.actor.catch_up(a_peer, channel).await;
            // Inject the host's already-authenticated outbound observations. The separate TCP
            // regression verifies their actual direction; this fixture stresses both sole owners.
            let a_route = AuthenticatedDialRoute {
                peer: b_peer,
                address: format!("/ip4/127.0.0.1/tcp/9412/p2p/{b_id}"),
            };
            let b_route = AuthenticatedDialRoute {
                peer: a_peer,
                address: format!("/ip4/127.0.0.1/tcp/9411/p2p/{a_id}"),
            };
            let (left, right) = tokio::join!(
                member_reconnect::persist(&a_state, 1, 1, &a.actor, vec![a_route]),
                member_reconnect::persist(&b_state, 1, 1, &b.actor, vec![b_route]),
            );
            assert!(left.unwrap());
            assert!(right.unwrap());
            assert_eq!(net(&a_state).await.reconnect_routes.len(), 1);
            assert_eq!(net(&b_state).await.reconnect_routes.len(), 1);
            stop(&a_state, a).await;
            stop(&b_state, b).await;
        })
        .await
        .expect("two real actor loops must not deadlock on reciprocal finalization");
    }

    #[tokio::test]
    async fn reply_callback_restart_uses_proven_outbound_listener_in_both_start_orders() {
        timeout(Duration::from_secs(45), async {
            for listener_first in [true, false] {
                let a_dir = tempfile::tempdir().unwrap();
                let b_dir = tempfile::tempdir().unwrap();
                let a_state = mount(a_dir.path()).await;
                let b_state = mount(b_dir.path()).await;
                let mut a_net = new_server_net("", "", "");
                let mut b_net = new_server_net("", "", "");
                // A cannot accept a connection at all. The original reply callback A -> B is
                // the only usable direction; B's inbound ephemeral source is never a listener.
                a_net.port = 0;
                let (a_transport, _, _) = MeshService::new_tcp_with_key(
                    keypair_from_seed(a_net.key_seed).unwrap(),
                    &[],
                    &[],
                )
                .unwrap();
                let (mut b_transport, b_id, _) = MeshService::new_tcp_with_key(
                    keypair_from_seed(b_net.key_seed).unwrap(),
                    &["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                    &[],
                )
                .unwrap();
                let listener = timeout(Duration::from_secs(5), b_transport.next_listen_addr())
                    .await
                    .unwrap()
                    .unwrap();
                b_net.port = listen_port(&listener).unwrap();
                let route = format!("{listener}/p2p/{b_id}");
                let a_mesh = a_transport.handle();
                let b_mesh = b_transport.handle();
                let a_peer = a_mesh.local_peer();
                let b_peer = b_mesh.local_peer();
                a_mesh.dial(route.parse().unwrap()).await.unwrap();
                timeout(
                    Duration::from_secs(5),
                    b_transport.wait_for_peer_connected(a_peer),
                )
                .await
                .unwrap()
                .unwrap();
                let mut a = Server::found(
                    a_transport,
                    MlsDevice::generate().unwrap(),
                    ChaCha20Rng::seed_from_u64(10),
                    Box::new(catcoms_rt::SystemClock),
                    "inviter",
                )
                .unwrap();
                a.subscribe_control().await.unwrap();
                a.publish_self_record(Vec::new(), a_net.record_seq).unwrap();
                let invite = a
                    .mint_invite(
                        [0x51; 16],
                        catcoms_rt::SystemClock.now_ms() + 60_000,
                        vec![],
                    )
                    .unwrap();
                let group = a.group_id();
                let device = a.device_id();
                let (actor, events, task) = spawn(a);
                let a = register(&a_state, actor, a_mesh, events, task, group, device).await;
                let (mut b, contact) = Server::join_from_reply(
                    b_transport,
                    MlsDevice::generate().unwrap(),
                    ChaCha20Rng::seed_from_u64(11),
                    Box::new(catcoms_rt::SystemClock),
                    "joiner",
                    a_peer,
                    a_peer,
                    &invite,
                    [0x52; 16],
                    b"proven-reply-joiner",
                    catcoms_rt::SystemClock.now_ms() + 60_000,
                )
                .await
                .unwrap();
                assert_eq!(contact, a_peer);
                let _ = finalize_admission_discovery(
                    &mut b,
                    contact,
                    Vec::new(),
                    b_net.record_seq,
                    Duration::from_secs(3),
                )
                .await
                .unwrap();
                let group = b.group_id();
                let device = b.device_id();
                let (actor, events, task) = spawn(b);
                let b = register(&b_state, actor, b_mesh, events, task, group, device).await;
                persist_server_net(&a_state, 1, &a_net).await;
                persist_server_net(&b_state, 1, &b_net).await;
                let channel = channel_id("general");
                a.actor.open_channel(channel).await;
                b.actor.open_channel(channel).await;
                // Exercise the production close pre-pass immediately after successful admission.
                let (left, right) = tokio::join!(
                    member_reconnect::before_shutdown(&a_state),
                    member_reconnect::before_shutdown(&b_state)
                );
                left.unwrap();
                right.unwrap();
                let saved_a = net(&a_state).await;
                let saved_b = net(&b_state).await;
                assert_eq!(saved_a.reconnect_policy, ReconnectPolicy::MemberMesh);
                assert_eq!(saved_b.reconnect_policy, ReconnectPolicy::MemberMesh);
                assert_eq!(
                    saved_a.reconnect_routes,
                    vec![ReconnectRoute {
                        peer_id: *b_peer.as_bytes(),
                        address: route.clone()
                    }]
                );
                assert!(
                    saved_b.reconnect_routes.is_empty(),
                    "inbound source port must never become a listener hint"
                );
                assert_eq!(saved_a.key_seed, a_net.key_seed);
                assert_eq!(saved_b.port, b_net.port);
                assert_eq!(saved_a.record_seq, a_net.record_seq);
                stop(&b_state, b).await;
                // This signed history is available only on A's disk when both applications stop.
                a.actor
                    .send_reply(channel, "retained while B was offline", String::new())
                    .await
                    .unwrap();
                member_reconnect::before_shutdown(&a_state).await.unwrap();
                assert_eq!(
                    net(&a_state).await.reconnect_routes,
                    saved_a.reconnect_routes,
                    "disconnect must retain the last good direction"
                );
                stop(&a_state, a).await;
                timeout(Duration::from_secs(5), async {
                    loop {
                        if std::net::TcpListener::bind((
                            std::net::Ipv4Addr::LOCALHOST,
                            saved_b.port,
                        ))
                        .is_ok()
                        {
                            break;
                        }
                        catcoms_rt::SystemClock
                            .sleep(Duration::from_millis(10))
                            .await;
                    }
                })
                .await
                .unwrap();
                // Reopen the vault too: no retained process memory supplies descriptor authority.
                *a_state.store.lock().await = None;
                *b_state.store.lock().await = None;
                let a_state = mount(a_dir.path()).await;
                let b_state = mount(b_dir.path()).await;
                let (a, b) = if listener_first {
                    let b = restore(&b_state, true).await;
                    (restore(&a_state, false).await, b)
                } else {
                    let a = restore(&a_state, false).await;
                    catcoms_rt::SystemClock
                        .sleep(Duration::from_millis(100))
                        .await;
                    let b = restore(&b_state, true).await;
                    a.actor.drive_discovery().await.unwrap();
                    (a, b)
                };
                timeout(Duration::from_secs(10), async {
                    loop {
                        if a.mesh
                            .authenticated_dial_routes()
                            .iter()
                            .any(|r| r.peer == b_peer)
                        {
                            break;
                        }
                        catcoms_rt::SystemClock
                            .sleep(Duration::from_millis(20))
                            .await;
                    }
                })
                .await
                .expect("saved outbound callback direction reconnects without another invitation");
                assert!(b.mesh.authenticated_dial_route_evidence().is_empty());
                b.actor.catch_up(a_peer, channel).await;
                assert!(b
                    .actor
                    .messages(channel)
                    .await
                    .iter()
                    .any(|m| m.text == "retained while B was offline"));
                assert!(net(&a_state).await.record_seq > saved_a.record_seq);
                stop(&a_state, a).await;
                stop(&b_state, b).await;
            }
        })
        .await
        .expect("bounded reply restart regression");
    }
}
