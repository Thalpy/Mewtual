#[tokio::test]
async fn native_close_saves_history_and_pending_admission_without_waiting_for_offline_peer() {
    timeout(Duration::from_secs(30), async {
        let dir = tempfile::tempdir().unwrap();
        let state = mount(dir.path()).await;
        *state.session_resumable.lock().await = true;
        let network = new_server_net("", "", "");
        let peer_network = new_server_net("", "", "");
        let (local_tcp, _, _) =
            MeshService::new_tcp_with_key(keypair_from_seed(network.key_seed).unwrap(), &[], &[])
                .unwrap();
        let (remote_tcp, remote_id, _) = MeshService::new_tcp_with_key(
            keypair_from_seed(peer_network.key_seed).unwrap(),
            &["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            &[],
        )
        .unwrap();
        let local_peer = local_tcp.handle().local_peer();
        let remote_peer = remote_tcp.handle().local_peer();
        let listener = remote_tcp.next_listen_addr().await.unwrap();
        local_tcp
            .handle()
            .dial(format!("{listener}/p2p/{remote_id}").parse().unwrap())
            .await
            .unwrap();
        remote_tcp
            .wait_for_peer_connected(local_peer)
            .await
            .unwrap();
        local_tcp
            .wait_for_peer_connected(remote_peer)
            .await
            .unwrap();

        // Actual admission and Noise evidence use the same identities. The Hub lets the remote
        // MLS owner disappear without giving a test provider any opportunity to finalize it.
        let hub = catcoms_rt::Hub::new();
        let mut owner = Server::found(
            hub.join(local_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(110),
            Box::new(SystemClock),
            "owner",
        )
        .unwrap();
        owner.subscribe_control().await.unwrap();
        owner
            .publish_self_record(Vec::new(), network.record_seq)
            .unwrap();
        let invite = owner
            .mint_invite([110; 16], SystemClock.now_ms() + 60_000, vec![])
            .unwrap();
        let (joined, served) = tokio::join!(
            Server::join_from_reply(
                hub.join(remote_peer),
                MlsDevice::generate().unwrap(),
                ChaCha20Rng::seed_from_u64(111),
                Box::new(SystemClock),
                "member",
                local_peer,
                local_peer,
                &invite,
                [111; 16],
                b"accepted",
                SystemClock.now_ms() + 60_000,
            ),
            owner.sync_once(),
        );
        served.unwrap();
        let (member, _) = joined.unwrap();
        assert_eq!(owner.member_finalization_candidates(), vec![remote_peer]);
        assert!(owner.finalized_member_peers().is_empty());
        assert!(owner
            .member_routes()
            .iter()
            .all(|route| route.peer_id.is_none()));
        drop(member);
        drop(remote_tcp);
        timeout(Duration::from_secs(5), async {
            while local_tcp
                .connection_snapshot()
                .iter()
                .any(|row| row.peer == remote_peer)
            {
                SystemClock.sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let evidence = local_tcp.handle().authenticated_dial_route_evidence();
        assert_eq!(
            evidence.len(),
            1,
            "the real outbound observation survives disconnect"
        );
        assert_eq!(evidence[0].peer, remote_peer);

        let group = owner.group_id();
        let device = owner.device_id();
        let (actor, events, task) = spawn(owner);
        let running = register(
            &state,
            actor,
            local_tcp.handle(),
            events,
            task,
            group,
            device,
        )
        .await;
        assert_eq!(
            persist_server_net(&state, 1, &network).await,
            PersistOutcome::Durable
        );
        let channel = channel_id("general");
        running.actor.open_channel(channel).await;
        let op = Operation::start(
            None,
            catcoms_diagnostics::Section::Channels,
            "send_message",
            1,
            None,
        );
        let accepted = durable_chat::send(
            &state,
            &op,
            1,
            &channel.to_string(),
            "saved independently of the offline peer".into(),
            None,
            Some("12".repeat(16)),
            Some(hex::encode(
                running.actor.durable_send_context().await.unwrap(),
            )),
        )
        .await
        .unwrap();
        assert!(accepted.accepted);
        let expected = running.actor.messages(channel).await;
        assert_eq!(expected.len(), 1);
        assert_eq!(
            accepted.message_id.as_deref(),
            Some(expected[0].id.as_str())
        );

        // Local custody failure still refuses close and leaves the actor usable for retry.
        state.session_lock_requested.store(true, Ordering::Release);
        let store = state.store.lock().await.take().unwrap();
        assert!(shutdown::freeze_servers(&state).await.is_err());
        assert!(running.actor.snapshot().await.is_ok());
        *state.store.lock().await = Some(store);

        // A real checkpoint replacement failure must also defer close, retaining the previous
        // accepted snapshot. The obstruction is confined to this test's independent vault.
        let checkpoint = dir.path().join("servers").join("1.bin");
        let retained = dir.path().join("servers").join("1.retained");
        std::fs::rename(&checkpoint, &retained).unwrap();
        std::fs::create_dir(&checkpoint).unwrap();
        assert!(shutdown::freeze_servers(&state).await.is_err());
        assert_eq!(running.actor.messages(channel).await, expected);
        std::fs::remove_dir(&checkpoint).unwrap();
        std::fs::rename(&retained, &checkpoint).unwrap();

        shutdown::freeze_servers(&state)
            .await
            .expect("remote unavailability is saved pending work")
            .stop();
        running.task.await.unwrap();
        running.drain.await.unwrap();
        state.servers.lock().await.clear();
        drop(running.mesh);
        drop(local_tcp);
        drop(state.store.lock().await.take());

        let reopened = mount(dir.path()).await;
        let saved_net = net(&reopened).await;
        assert_eq!(saved_net.reconnect_policy, ReconnectPolicy::MemberMesh);
        assert!(
            saved_net.reconnect_routes.is_empty(),
            "unfinalized listener gained no restart permission"
        );
        let saved = reopened
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_server(1)
            .unwrap();
        let mut restored = Server::restore(
            &saved,
            catcoms_rt::Hub::new().join(local_peer),
            ChaCha20Rng::seed_from_u64(112),
            Box::new(SystemClock),
            "owner",
        )
        .unwrap();
        assert_eq!(restored.messages(channel), expected);
        assert_eq!(restored.member_finalization_candidates(), vec![remote_peer]);
        assert!(restored.finalized_member_peers().is_empty());
        assert!(!restored
            .finalize_member_connection(remote_peer)
            .await
            .unwrap());
        let (actor, mut events, task) = spawn(restored);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        assert!(member_reconnect::saved_warning(&reopened, 1, &actor)
            .await
            .unwrap()
            .contains("another member"));
        actor.shutdown().await;
        task.await.unwrap();
        drain.await.unwrap();
    })
    .await
    .expect("local shutdown never waits for an offline member to prove a route");
}
