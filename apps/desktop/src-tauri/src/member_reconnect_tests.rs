mod member_reconnect_regressions {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn duplicate_address_families_do_not_hide_another_member_finalization_target() {
        let first = PeerId::from_u64(1);
        let second = PeerId::from_u64(2);
        let candidates = HashSet::from([first, second]);
        let evidence = vec![
            AuthenticatedDialRoute {
                peer: first,
                address: "ipv4 observation".into(),
            },
            AuthenticatedDialRoute {
                peer: first,
                address: "ipv6 observation".into(),
            },
            AuthenticatedDialRoute {
                peer: second,
                address: "other member observation".into(),
            },
        ];
        assert_eq!(
            member_reconnect::finalization_targets(&evidence, &candidates),
            std::collections::BTreeSet::from([first, second])
        );
    }

    /// The native capture uses real outbound Noise evidence. The actor fixture uses the same
    /// transport identities on a deterministic Hub so selected members can stop serving while
    /// the TCP observation remains, without substituting descriptor or membership authority.
    async fn capture_worker_three_members(first_two_proven: bool) {
        timeout(Duration::from_secs(30), async {
            let dir = tempfile::tempdir().unwrap();
            let state = mount(dir.path()).await;
            let mut identities: Vec<_> = (0..4).map(|_| new_server_net("", "", "")).collect();
            identities.sort_by_key(|net| {
                phase0_peer_id(&keypair_from_seed(net.key_seed).unwrap().public().to_peer_id())
            });
            let owner_net = identities.remove(0);
            let (owner_tcp, _, _) = MeshService::new_tcp_with_key(
                keypair_from_seed(owner_net.key_seed).unwrap(), &[], &[],
            ).unwrap();
            let owner_peer = owner_tcp.handle().local_peer();
            let hub = catcoms_rt::Hub::new();
            let mut owner = Server::found(
                hub.join(owner_peer), MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(70), Box::new(SystemClock), "owner",
            ).unwrap();
            owner.subscribe_control().await.unwrap();
            owner.publish_self_record(Vec::new(), owner_net.record_seq).unwrap();
            let mut members = Vec::new();
            let mut tcp_members = Vec::new();
            let mut member_peers = Vec::new();
            for (i, network) in identities.iter().enumerate() {
                let (tcp, id, _) = MeshService::new_tcp_with_key(
                    keypair_from_seed(network.key_seed).unwrap(),
                    &["/ip4/127.0.0.1/tcp/0".parse().unwrap()], &[],
                ).unwrap();
                let listener = tcp.next_listen_addr().await.unwrap();
                let peer = tcp.handle().local_peer();
                owner_tcp.handle().dial(format!("{listener}/p2p/{id}").parse().unwrap()).await.unwrap();
                tcp.wait_for_peer_connected(owner_peer).await.unwrap();
                let invite = owner.mint_invite([i as u8 + 70; 16], SystemClock.now_ms() + 60_000, vec![]).unwrap();
                let mut member = tokio::select! {
                    joined = Server::join(
                        hub.join(peer), MlsDevice::generate().unwrap(),
                        ChaCha20Rng::seed_from_u64(71 + i as u64), Box::new(SystemClock),
                        "member", owner_peer, &invite,
                    ) => joined.unwrap(),
                    _ = async { loop { owner.sync_once().await.unwrap(); } } => unreachable!(),
                };
                member.subscribe_control().await.unwrap();
                member.publish_self_record(Vec::new(), network.record_seq).unwrap();
                members.push(member);
                member_peers.push(peer);
                tcp_members.push(tcp);
            }
            for member in &mut members {
                while member.epoch() < owner.epoch() {
                    tokio::select! {
                        result = member.sync_once() => { result.unwrap(); },
                        _ = async { loop { owner.sync_once().await.unwrap(); } } => unreachable!(),
                    }
                }
            }
            assert_eq!(owner.member_finalization_candidates(), member_peers);
            assert!(owner.finalized_member_peers().is_empty());
            if first_two_proven {
                for member in members.iter_mut().take(2) {
                    let peer = member.member_finalization_candidates()[0];
                    assert_eq!(peer, owner_peer);
                    tokio::select! {
                        done = member.finalize_member_connection(owner_peer) => assert!(done.unwrap()),
                        _ = async { loop { owner.sync_once().await.unwrap(); } } => unreachable!(),
                    }
                }
                let proven: HashSet<_> = owner.finalized_member_peers().into_iter().collect();
                assert_eq!(proven, member_peers[..2].iter().copied().collect());
            }
            let healthy = members.pop().unwrap();
            // Closed Hub request receivers fail immediately; the native observation is still
            // real, retained successful Noise evidence. These two cannot complete kind 26.
            drop(members);
            let (healthy_actor, mut healthy_events, healthy_task) = spawn(healthy);
            let healthy_drain = tokio::spawn(async move { while healthy_events.recv().await.is_some() {} });
            let group = owner.group_id();
            let device = owner.device_id();
            let (actor, events, task) = spawn(owner);
            let running = register(&state, actor, owner_tcp.handle(), events, task, group, device).await;
            persist_server_net(&state, 1, &owner_net).await;
            let evidence = owner_tcp.handle().authenticated_dial_route_evidence();
            assert_eq!(evidence.iter().map(|route| route.peer).collect::<HashSet<_>>(), member_peers.iter().copied().collect());
            let (wake, mut receiver) = watch::channel(0);
            tokio::select! {
                _ = member_reconnect::run_capture_worker(&state, 1, 1, &running.actor, &mut receiver) => unreachable!(),
                _ = async {
                    if !first_two_proven {
                        // First pass spends only its two work slots, but its incomplete result
                        // still saves the restrictive snapshot/net barrier. Then wake that SAME
                        // production worker: rotation must offer the third member a turn.
                        loop {
                            SystemClock.sleep(Duration::from_millis(100)).await;
                            if net(&state).await.reconnect_policy == ReconnectPolicy::MemberMesh { break; }
                        }
                        assert!(!running.actor.finalized_member_peers().await.unwrap().contains(&member_peers[2]));
                        wake.send(1).unwrap();
                    }
                    loop {
                        SystemClock.sleep(Duration::from_millis(300)).await;
                        if running.actor.finalized_member_peers().await.unwrap().contains(&member_peers[2]) { break; }
                    }
                } => {}
            }
            if !first_two_proven {
                assert!(member_reconnect::persist(&state, 1, 1, &running.actor, evidence).await.is_err(),
                    "unselected/failing admitted members must remain outstanding after the healthy member progresses");
            }
            stop(&state, running).await;
            healthy_actor.shutdown().await;
            healthy_task.await.unwrap();
            healthy_drain.await.unwrap();
            drop(tcp_members);
        }).await.expect("three-member finalization worker progresses within bounded passes");
    }

    #[tokio::test]
    async fn capture_worker_skips_two_finalized_members_before_spending_its_work_budget() {
        capture_worker_three_members(true).await;
    }

    #[tokio::test]
    async fn capture_worker_rotates_past_two_failed_members_and_keeps_their_obligations() {
        capture_worker_three_members(false).await;
    }

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

    async fn restore(state: &AppState, listen: bool, clock: &ManualClock) -> Running {
        let net = load_or_init_server_net(state, 1, "").await.unwrap();
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
        let (transport, _, _) = MeshService::new_tcp_with_key(
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
            Box::new(clock.clone()),
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
    async fn unfinalized_outbound_evidence_stays_pending_unless_a_saved_current_member_route_covers_it(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let state = mount(dir.path()).await;
        let (mut inviter, mut joiner, inviter_peer, joiner_peer) =
            admitted_reply_pair(Vec::new()).await;
        // Learn both signed descriptors through the actual finalization protocol, then throw
        // away the live proof. The restored actor knows a current member but has no connection.
        tokio::select! {
            outcome = finalize_admission_discovery(&mut joiner, inviter_peer, Vec::new(), 65_536, Duration::from_secs(1)) => { outcome.unwrap(); },
            _ = async { loop { inviter.sync_once().await.unwrap(); } } => unreachable!(),
        }
        let snapshot = inviter.snapshot().unwrap();
        drop(inviter);
        drop(joiner);
        let mut network = new_server_net("", "", "");
        network.key_seed = [21; 32];
        network.key_seed[0] = 111;
        let (transport, _, _) =
            MeshService::new_tcp_with_key(keypair_from_seed(network.key_seed).unwrap(), &[], &[])
                .unwrap();
        assert_eq!(transport.handle().local_peer(), inviter_peer);
        let server = Server::restore(
            &snapshot,
            catcoms_rt::Hub::new().join(inviter_peer),
            ChaCha20Rng::seed_from_u64(41),
            Box::new(ManualClock::new(2_000)),
            "inviter",
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
        let evidence = AuthenticatedDialRoute {
            peer: joiner_peer,
            address: format!("/ip4/127.0.0.1/tcp/9412/p2p/{}", test_libp2p_peer(112)),
        };
        assert!(
            member_reconnect::persist(&state, 1, 1, &running.actor, vec![evidence.clone()])
                .await
                .is_err()
        );
        let mut saved = net(&state).await;
        assert!(
            saved.reconnect_routes.is_empty(),
            "an unbound observation never becomes a sealed listener"
        );
        assert_eq!(
            saved.reconnect_policy,
            ReconnectPolicy::MemberMesh,
            "the durable policy pin still saves despite unfinished reachability"
        );
        // A route sealed during a prior authenticated overlap remains a valid candidate. An
        // unavailable member must not prevent close once that exact direction is already durable.
        saved.reconnect_routes = vec![ReconnectRoute {
            peer_id: *joiner_peer.as_bytes(),
            address: evidence.address.clone(),
        }];
        // Build a prior sealed checkpoint directly: admission network persistence deliberately
        // preserves already-stored route authority instead of replacing it from a stale copy.
        state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .save_server_net(1, &saved, &mut OsCryptoRng)
            .unwrap();
        assert_eq!(net(&state).await.reconnect_routes, saved.reconnect_routes);
        assert!(
            member_reconnect::persist(&state, 1, 1, &running.actor, vec![evidence])
                .await
                .unwrap()
        );
        assert_eq!(net(&state).await.reconnect_routes, saved.reconnect_routes);
        // A sealed route without its current roster descriptor is insufficient: the predicate
        // above consumes actor member_routes, not only a byte match in a network record.
        assert!(running
            .actor
            .member_routes()
            .await
            .iter()
            .any(|route| route.peer_id == Some(*joiner_peer.as_bytes())));
        stop(&state, running).await;
    }

    #[tokio::test]
    async fn only_an_admitted_observed_callback_can_hold_close_pending_before_its_descriptor() {
        timeout(Duration::from_secs(20), async {
            let a_dir = tempfile::tempdir().unwrap();
            let b_dir = tempfile::tempdir().unwrap();
            let a_state = mount(a_dir.path()).await;
            let b_state = mount(b_dir.path()).await;
            let a_net = new_server_net("", "", "");
            let b_net = new_server_net("", "", "");
            let (a_transport, _, _) =
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
                ChaCha20Rng::seed_from_u64(51),
                Box::new(catcoms_rt::SystemClock),
                "a",
            )
            .unwrap();
            a.subscribe_control().await.unwrap();
            a.publish_self_record(Vec::new(), a_net.record_seq).unwrap();
            let invite = a
                .mint_invite([0x71; 16], SystemClock.now_ms() + 60_000, vec![])
                .unwrap();
            let (joined, served) = tokio::join!(
                Server::join_from_reply(
                    hub.join(b_peer),
                    MlsDevice::generate().unwrap(),
                    ChaCha20Rng::seed_from_u64(52),
                    Box::new(catcoms_rt::SystemClock),
                    "b",
                    a_peer,
                    a_peer,
                    &invite,
                    [0x72; 16],
                    b"proven",
                    SystemClock.now_ms() + 60_000
                ),
                a.sync_once(),
            );
            served.unwrap();
            let (mut b, _) = joined.unwrap();
            b.publish_self_record(Vec::new(), b_net.record_seq).unwrap();
            assert!(a
                .member_routes()
                .iter()
                .all(|route| route.peer_id.is_none()));
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
            persist_server_net(&a_state, 1, &a_net).await;
            let stranger = test_libp2p_peer(99);
            let unknown = AuthenticatedDialRoute {
                peer: phase0_peer_id(&stranger),
                address: format!("/ip4/127.0.0.1/tcp/9499/p2p/{stranger}"),
            };
            assert!(
                member_reconnect::persist(&a_state, 1, 1, &a.actor, vec![unknown])
                    .await
                    .unwrap(),
                "a Noise-only infrastructure/helper endpoint cannot hold close pending"
            );
            let evidence = AuthenticatedDialRoute {
                peer: b_peer,
                address: format!("/ip4/127.0.0.1/tcp/9452/p2p/{b_id}"),
            };
            // B accepted membership but its actor is deliberately not serving the new exchange.
            assert!(
                member_reconnect::persist(&a_state, 1, 1, &a.actor, vec![evidence.clone()])
                    .await
                    .is_err()
            );
            assert!(net(&a_state).await.reconnect_routes.is_empty());
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
            // The new owner first drains requests whose caller already timed out. Let the
            // existing per-device serving interval expire before the next close attempt.
            SystemClock.sleep(Duration::from_millis(1_100)).await;
            assert!(
                member_reconnect::persist(&a_state, 1, 1, &a.actor, vec![evidence])
                    .await
                    .unwrap()
            );
            assert_eq!(net(&a_state).await.reconnect_routes.len(), 1);
            stop(&a_state, a).await;
            stop(&b_state, b).await;
        })
        .await
        .expect("an admitted candidate must either finalize or keep close pending");
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
    async fn mesh_restart_keeps_private_alternative_after_refreshing_two_transports_to_an_offline_peer(
    ) {
        timeout(Duration::from_secs(50), async {
            let dir = tempfile::tempdir().unwrap();
            let state = mount(dir.path()).await;
            let a_net = new_server_net("", "", "");
            let (a_tcp, _, _) = MeshService::new_tcp_with_key(
                keypair_from_seed(a_net.key_seed).unwrap(), &[], &[],
            ).unwrap();
            let a_peer = a_tcp.handle().local_peer();
            let hub = catcoms_rt::Hub::new();
            let mut a = Server::found(
                hub.join(a_peer), MlsDevice::generate().unwrap(), ChaCha20Rng::seed_from_u64(80),
                Box::new(SystemClock), "owner",
            ).unwrap();
            a.subscribe_control().await.unwrap();
            a.publish_self_record(Vec::new(), a_net.record_seq).unwrap();
            let mut members = Vec::new();
            let mut transports = Vec::new();
            let mut peers = Vec::new();
            let mut listener_ports = Vec::new();
            for index in 0..2 {
                let network = new_server_net("", "", "");
                let mut listeners = vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()];
                if index == 0 { listeners.push("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()); }
                let (tcp, id, _) = MeshService::new_tcp_with_key(
                    keypair_from_seed(network.key_seed).unwrap(), &listeners, &[],
                ).unwrap();
                let peer = tcp.handle().local_peer();
                for _ in 0..listeners.len() {
                    let listener = tcp.next_listen_addr().await.unwrap();
                    listener_ports.push(listener.clone());
                    let route = format!("{listener}/p2p/{id}");
                    a_tcp.handle().dial(route.parse().unwrap()).await.unwrap();
                    loop {
                        if a_tcp.handle().authenticated_dial_route_evidence().iter().any(|e| e.address == route) { break; }
                        SystemClock.sleep(Duration::from_millis(20)).await;
                    }
                }
                let invite = a.mint_invite([80 + index; 16], SystemClock.now_ms() + 60_000, vec![]).unwrap();
                let mut member = tokio::select! {
                    joined = Server::join(
                        hub.join(peer), MlsDevice::generate().unwrap(),
                        ChaCha20Rng::seed_from_u64(81 + index as u64), Box::new(SystemClock),
                        "member", a_peer, &invite,
                    ) => joined.unwrap(),
                    _ = async { loop { a.sync_once().await.unwrap(); } } => unreachable!(),
                };
                member.subscribe_control().await.unwrap();
                member.publish_self_record(Vec::new(), network.record_seq).unwrap();
                members.push(member);
                transports.push(tcp);
                peers.push(peer);
            }
            for member in &mut members {
                while member.epoch() < a.epoch() {
                    tokio::select! {
                        result = member.sync_once() => { result.unwrap(); },
                        _ = async { loop { a.sync_once().await.unwrap(); } } => unreachable!(),
                    }
                }
                tokio::select! {
                    done = member.finalize_member_connection(a_peer) => assert!(done.unwrap()),
                    _ = async { loop { a.sync_once().await.unwrap(); } } => unreachable!(),
                }
            }
            assert!(a.member_routes().iter().all(|member| member.addresses.is_empty()),
                "neither member advertises a public/cache fallback");
            let group = a.group_id();
            let device = a.device_id();
            let (actor, events, task) = spawn(a);
            let running = register(&state, actor, a_tcp.handle(), events, task, group, device).await;
            persist_server_net(&state, 1, &a_net).await;
            let evidence = a_tcp.handle().authenticated_dial_route_evidence();
            let b: Vec<_> = evidence.iter().filter(|route| route.peer == peers[0]).cloned().collect();
            let c: Vec<_> = evidence.iter().filter(|route| route.peer == peers[1]).cloned().collect();
            assert_eq!(b.len(), 2, "B has independently successful TCP and QUIC observations");
            assert_eq!(c.len(), 1, "C has one private last-good direction");
            assert!(member_reconnect::persist(&state, 1, 1, &running.actor, c.clone()).await.unwrap());
            assert!(member_reconnect::persist(&state, 1, 1, &running.actor, b).await.unwrap());
            let saved = net(&state).await;
            assert_eq!(saved.reconnect_routes.len(), 3);
            assert_eq!(saved.reconnect_routes.iter().filter(|route| route.peer_id == *peers[0].as_bytes()).count(), 2);
            assert!(saved.reconnect_routes.iter().any(|route| route.address == c[0].address));
            assert!(saved.rendezvous.is_empty() && saved.relay.is_empty() && saved.advertise.is_empty());
            stop(&state, running).await;
            drop(a_tcp);
            let channel = channel_id("general");
            let mut c_server = members.pop().unwrap();
            c_server.open_channel(channel).await.unwrap();
            c_server.send_message(channel, "signed history from the retained alternative").await.unwrap();
            let original = c_server.messages(channel).into_iter().find(|message| message.text == "signed history from the retained alternative").unwrap();
            let c_snapshot = c_server.snapshot().unwrap();
            drop(c_server);
            drop(members);
            // B disappears entirely; C remains on its original, private listener.
            drop(transports.remove(0));
            for listener in listener_ports.iter().take(2) {
                loop {
                    let released = if listener.iter().any(|part| matches!(part, Protocol::QuicV1)) {
                        let port = listener.iter().find_map(|part| if let Protocol::Udp(port) = part { Some(port) } else { None }).unwrap();
                        std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok()
                    } else {
                        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, listen_port(listener).unwrap())).is_ok()
                    };
                    if released { break; }
                    SystemClock.sleep(Duration::from_millis(20)).await;
                }
            }
            *state.store.lock().await = None;
            let state = mount(dir.path()).await;
            let clock = ManualClock::new(SystemClock.now_ms());
            let mut c_server = Server::restore(
                &c_snapshot, transports.remove(0), ChaCha20Rng::seed_from_u64(83),
                Box::new(clock.clone()), "alternative",
            ).unwrap();
            c_server.subscribe_control().await.unwrap();
            let (c_actor, mut c_events, c_task) = spawn(c_server);
            let c_drain = tokio::spawn(async move { while c_events.recv().await.is_some() {} });
            tokio::select! {
                _ = async { loop { SystemClock.sleep(Duration::from_millis(100)).await; clock.advance_ms(500); } } => unreachable!(),
                _ = async {
                    let restored = restore(&state, false, &clock).await;
                    timeout(Duration::from_secs(8), async {
                        loop {
                            if restored.mesh.authenticated_dial_routes().iter().any(|route| route.peer == peers[1]) { break; }
                            SystemClock.sleep(Duration::from_millis(20)).await;
                        }
                    }).await.expect("cold native restore attempts the retained private alternative while B is gone");
                    assert!(!restored.mesh.authenticated_dial_routes().iter().any(|route| route.peer == peers[0]));
                    assert_eq!(net(&state).await.key_seed, saved.key_seed);
                    restored.actor.catch_up(peers[1], channel).await;
                    timeout(Duration::from_secs(20), async {
                        loop {
                            SystemClock.sleep(Duration::from_millis(300)).await;
                            if let Some(message) = restored.actor.messages(channel).await.into_iter().find(|message| message.id == original.id) {
                                assert_eq!(message.author, original.author);
                                assert_eq!(message.text, original.text);
                                break;
                            }
                        }
                    }).await.expect("the alternative supplies its original signed retained message over real TCP");
                    stop(&state, restored).await;
                    c_actor.shutdown().await;
                    c_task.await.unwrap();
                    c_drain.await.unwrap();
                } => {}
            }
            drop(transports);
        }).await.expect("bounded multi-member restart fallback");
    }

    #[tokio::test]
    async fn reply_callback_restart_uses_proven_outbound_listener_in_both_start_orders() {
        timeout(Duration::from_secs(60), async {
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
                let (b_transport, b_id, _) = MeshService::new_tcp_with_key(
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
                let recovery_clock = ManualClock::new(SystemClock.now_ms());
                // Advance the injected recovery deadlines while exercising real TCP and actor
                // loops. The 30/31-second reciprocal backoffs must progress on a quiet network;
                // wall-clock service windows still allow the request/response owners to run.
                tokio::select! {
                    _ = async {
                        loop {
                            SystemClock.sleep(Duration::from_millis(100)).await;
                            recovery_clock.advance_ms(500);
                        }
                    } => unreachable!(),
                    _ = async {
                        let (a, b) = if listener_first {
                            let b = restore(&b_state, true, &recovery_clock).await;
                            (restore(&a_state, false, &recovery_clock).await, b)
                        } else {
                            let a = restore(&a_state, false, &recovery_clock).await;
                            catcoms_rt::SystemClock
                                .sleep(Duration::from_millis(100))
                                .await;
                            let b = restore(&b_state, true, &recovery_clock).await;
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
                        // CatchUp enqueues a request; an overlapping reciprocal request can defer it
                        // to the normal recovery queue. Let both sole owners serve between queries.
                        timeout(Duration::from_secs(20), async {
                            loop {
                                SystemClock.sleep(Duration::from_millis(300)).await;
                                if b.actor.messages(channel).await.iter().any(|m| {
                                    m.text == "retained while B was offline"
                                }) {
                                    break;
                                }
                            }
                        })
                        .await
                        .expect("the restored peers recover the original signed offline history");
                        assert!(net(&a_state).await.record_seq > saved_a.record_seq);
                        stop(&a_state, a).await;
                        stop(&b_state, b).await;
                    } => {}
                }

            }
        })
        .await
        .expect("bounded reply restart regression");
    }
}
