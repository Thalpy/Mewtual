// Actual native capture calls, store records, actor commands and independent TCP evidence.
// Test-only barriers pause observation/installation; they cannot substitute route authority.
async fn concurrent_capture_retains_newer_route(pause_before_install: bool) {
    timeout(Duration::from_secs(60), async {
        let dir = tempfile::tempdir().unwrap();
        let state = mount(dir.path()).await;
        let owner_net = new_server_net("", "", "");
        let (owner_tcp, _, _) = MeshService::new_tcp_with_key(keypair_from_seed(owner_net.key_seed).unwrap(), &[], &[]).unwrap();
        let owner_peer = owner_tcp.handle().local_peer();
        let hub = catcoms_rt::Hub::new();
        let mut owner = Server::found(hub.join(owner_peer), MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(180), Box::new(SystemClock), "owner").unwrap();
        owner.subscribe_control().await.unwrap();
        owner.publish_self_record(Vec::new(), 65_536).unwrap();
        let mut members = Vec::new();
        let mut remotes = Vec::new();
        let mut peers = Vec::new();
        for index in 0..2 {
            let network = new_server_net("", "", "");
            let (tcp, id, _) = MeshService::new_tcp_with_key(keypair_from_seed(network.key_seed).unwrap(),
                &["/ip4/127.0.0.1/tcp/0".parse().unwrap()], &[]).unwrap();
            let listener = tcp.next_listen_addr().await.unwrap();
            let peer = tcp.handle().local_peer();
            owner_tcp.handle().dial(format!("{listener}/p2p/{id}").parse().unwrap()).await.unwrap();
            tcp.wait_for_peer_connected(owner_peer).await.unwrap();
            let invite = owner.mint_invite([180 + index as u8; 16], SystemClock.now_ms() + 60_000, vec![]).unwrap();
            let mut member = tokio::select! {
                joined = Server::join(hub.join(peer), MlsDevice::generate().unwrap(),
                    ChaCha20Rng::seed_from_u64(181 + index), Box::new(SystemClock), "member", owner_peer, &invite) => joined.unwrap(),
                _ = async { loop { owner.sync_once().await.unwrap(); } } => unreachable!(),
            };
            member.subscribe_control().await.unwrap();
            if index == 0 { member.publish_self_record(Vec::new(), 65_536).unwrap(); }
            members.push(member); remotes.push(tcp); peers.push(peer);
        }
        while members[0].epoch() < owner.epoch() {
            tokio::select! {
                result = members[0].sync_once() => { result.unwrap(); },
                _ = async { loop { owner.sync_once().await.unwrap(); } } => unreachable!(),
            }
        }
        tokio::select! {
            proven = owner.finalize_member_connection(peers[0]) => assert!(proven.unwrap()),
            _ = async { loop { members[0].sync_once().await.unwrap(); } } => unreachable!(),
        }
        assert!(owner.member_routes().iter().any(|route| route.peer_id == Some(*peers[0].as_bytes())));
        assert!(owner.member_routes().iter().all(|route| route.peer_id != Some(*peers[1].as_bytes())),
            "new member is admitted but its signed descriptor is genuinely still unknown");
        let group = owner.group_id(); let device = owner.device_id();
        let (actor, events, task) = spawn(owner);
        let running = register(&state, actor, owner_tcp.handle(), events, task, group, device).await;
        persist_server_net(&state, 1, &owner_net).await;
        let evidence = owner_tcp.handle().authenticated_dial_route_evidence();
        let old_evidence: Vec<_> = evidence.iter().filter(|route| route.peer == peers[0]).cloned().collect();
        assert!(!old_evidence.is_empty());

        let (reached, reached_rx) = tokio::sync::oneshot::channel();
        let (resume, resume_rx) = tokio::sync::oneshot::channel();
        let mut hooks = member_reconnect::CaptureHooks::default();
        if pause_before_install { hooks.before_install = Some((reached, resume_rx)); }
        else { hooks.after_claims = Some((reached, resume_rx)); }
        let mut older = Box::pin(member_reconnect::persist_with_proofs(&state, 1, 1, &running.actor, old_evidence, false, Some(hooks)));
        tokio::select! { result = &mut older => panic!("older capture passed its barrier: {result:?}"), result = reached_rx => result.unwrap() }

        let mut newcomer = members.pop().unwrap();
        newcomer.publish_self_record(Vec::new(), 65_536).unwrap();
        let (new_actor, mut new_events, new_task) = spawn(newcomer);
        let new_drain = tokio::spawn(async move { while new_events.recv().await.is_some() {} });
        assert!(running.actor.finalize_member_connection(peers[1]).await.unwrap());
        assert!(running.actor.member_routes().await.iter().any(|route| route.peer_id == Some(*peers[1].as_bytes())));
        let (queued, queued_rx) = tokio::sync::oneshot::channel();
        let (acquired, mut acquired_rx) = tokio::sync::oneshot::channel();
        let (new_claims, mut new_claims_rx) = tokio::sync::oneshot::channel();
        let (allow_claims, allow_claims_rx) = tokio::sync::oneshot::channel();
        let _ = allow_claims.send(());
        let hooks = member_reconnect::CaptureHooks { before_lock: Some(queued), after_lock: Some(acquired),
            after_claims: Some((new_claims, allow_claims_rx)), ..Default::default() };
        let mut newer = Box::pin(member_reconnect::persist_with_proofs(&state, 1, 1, &running.actor, evidence, false, Some(hooks)));
        tokio::select! { result = &mut newer => panic!("newer capture passed the older transaction: {result:?}"), result = queued_rx => result.unwrap() }
        assert!(std::future::poll_fn(|cx| std::task::Poll::Ready(newer.as_mut().poll(cx).is_pending())).await);
        assert!(matches!(acquired_rx.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)),
            "no Actor await lies between the queued/acquired observations: this is actual capture-lock exclusion");
        assert!(matches!(new_claims_rx.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)),
            "later capture cannot sample claims or install routes ahead of the earlier transaction");
        resume.send(()).unwrap();
        let (old, new) = tokio::join!(&mut older, &mut newer);
        assert!(matches!(old.unwrap(), CaptureOutcome::Saved { .. }));
        assert!(matches!(new.unwrap(), CaptureOutcome::Saved { saved_outgoing_routes: 2, .. }));
        acquired_rx.await.unwrap();
        new_claims_rx.await.unwrap();
        drop((older, newer));
        running.actor.member_count().await; // FIFO barrier after both actual route-install commands.
        let saved = net(&state).await;
        assert_eq!(saved.reconnect_routes.iter().map(|route| PeerId::new(route.peer_id)).collect::<HashSet<_>>(), peers.iter().copied().collect());
        // Cold native restoration independently consumes the saved authority. The older
        // endpoint is now gone; a real Noise connection must reach the newer saved endpoint.
        stop(&state, running).await;
        drop(owner_tcp); drop(remotes.remove(0)); drop(members);
        let restored = restore(&state, false, &ManualClock::new(2_000)).await;
        restored.actor.drive_discovery().await.unwrap();
        timeout(Duration::from_secs(10), async {
            while !restored.mesh.authenticated_dial_route_evidence().iter().any(|route| route.peer == peers[1]) {
                SystemClock.sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("cold restore reaches the newer retained route with the old endpoint offline");
        stop(&state, restored).await;
        new_actor.shutdown().await; new_task.await.unwrap(); new_drain.await.unwrap(); drop(remotes);
    }).await.expect("bounded concurrent native reconnect capture");
}

#[tokio::test]
async fn older_claims_cannot_erase_a_route_saved_by_a_newer_capture() {
    concurrent_capture_retains_newer_route(false).await;
}

#[tokio::test]
async fn older_live_route_installation_cannot_overtake_a_newer_capture() {
    concurrent_capture_retains_newer_route(true).await;
}
