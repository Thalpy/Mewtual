//! The headless native adapter: real sealed stores, actor workers, restore, send and close.
use super::*;

pub(super) const PASSWORD: &[u8] = b"six independent recovery vaults";

pub(super) struct Running {
    pub state: Arc<AppState>,
    pub actor: ServerActor,
    pub mesh: MeshHandle,
    pub passes: Arc<AtomicU64>,
    task: Option<tokio::task::JoinHandle<()>>,
    drain: Option<tokio::task::JoinHandle<()>>,
    background: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        for task in &self.background {
            task.abort();
        }
        if let Some(task) = &self.task {
            task.abort();
        }
        if let Some(task) = &self.drain {
            task.abort();
        }
    }
}

pub(super) async fn mount(path: &Path) -> Arc<AppState> {
    let state = Arc::new(AppState::default());
    *state.store.lock().await = Some(ServerStore::open(path, PASSWORD, &mut OsCryptoRng).unwrap());
    *state.session_resumable.lock().await = true;
    state
}

pub(super) async fn network(state: &AppState) -> ServerNet {
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

pub(super) async fn register(
    state: Arc<AppState>,
    restored: RestoredActor,
    mesh: MeshHandle,
    net: &ServerNet,
    instance: u64,
    clock: &ManualClock,
    index: usize,
) -> Running {
    let RestoredActor {
        actor,
        mut events,
        task,
        group_id,
        device_id,
    } = restored;
    state.servers.lock().await.insert(
        1,
        ServerEntry {
            actor: actor.clone(),
            instance,
            group_id,
            device_id,
            invite: None,
            name: "six-client fixture".into(),
            bootstrap: Vec::new(),
            bootstrap_owners: HashMap::new(),
            interface_routes: None,
            rendezvous: Vec::new(),
            mesh: Some(mesh.clone()),
            is_dm: false,
            switchboard: false,
            record_seq: net.record_seq,
            persist: PersistCounters::default(),
        },
    );
    assert_eq!(
        persist_server_net(&state, 1, net).await,
        PersistOutcome::Durable
    );
    assert_eq!(persist_registry(&state).await, PersistOutcome::Durable);
    let persist_wake = install_persistence_signal(&state, 1, instance)
        .await
        .unwrap();
    let mut capture_wake =
        replace_reconnect_capture_signal(&state.reconnect_capture_signals, 1).unwrap();
    let drain_state = state.clone();
    let drain = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            note_event_for_persistence(&drain_state, 1, instance, &event.event).await;
            // Match the production event forwarder's capture wake; never await the actor here.
            if matches!(
                event.event,
                AppEvent::ConnectivityChanged { .. } | AppEvent::MemberRoutesChanged
            ) {
                if let Some(signal) = drain_state
                    .reconnect_capture_signals
                    .lock()
                    .unwrap()
                    .get(&1)
                {
                    signal.send_modify(|generation| *generation = generation.wrapping_add(1));
                }
            }
        }
        remove_persistence_signal(&drain_state, 1, instance);
    });
    let save_state = state.clone();
    let persist = tokio::spawn(async move {
        run_persistence_worker(&save_state, 1, instance, persist_wake).await;
    });
    let capture_state = state.clone();
    let capture_actor = actor.clone();
    let capture = tokio::spawn(async move {
        member_reconnect::run_capture_worker(
            &capture_state,
            1,
            instance,
            &capture_actor,
            &mut capture_wake,
        )
        .await;
    });
    let passes = Arc::new(AtomicU64::new(0));
    let seen_passes = passes.clone();
    let cadence_state = state.clone();
    let cadence_actor = actor.clone();
    let cadence_clock = clock.clone();
    let changes = state.network_changes.subscribe();
    let cadence = tokio::spawn(async move {
        discovery_timer::run(
            &cadence_clock,
            changes,
            |base, spread| Duration::from_millis(base + spread * (index as u64 + 1) / 7),
            || {
                let state = cadence_state.clone();
                let actor = cadence_actor.clone();
                let passes = seen_passes.clone();
                async move {
                    if state
                        .servers
                        .lock()
                        .await
                        .get(&1)
                        .map(|entry| entry.instance)
                        != Some(instance)
                    {
                        return false;
                    }
                    if actor.drive_discovery().await.is_err() {
                        return false;
                    }
                    retry_pending_persistence(&state, 1, instance).await;
                    persist_live_local_reconnect_routes_in_state(&state, 1, instance, &actor).await;
                    passes.fetch_add(1, Ordering::Release);
                    true
                }
            },
        )
        .await;
    });
    // Initialize the same channel projection through the actor, not a pre-spawn private view.
    bounded(
        "open general channel",
        actor.open_channel(channel_id("general")),
    )
    .await;
    Running {
        state,
        actor,
        mesh,
        passes,
        task: Some(task),
        drain: Some(drain),
        background: vec![persist, capture, cadence],
    }
}

impl Running {
    pub async fn close(mut self) {
        // Production workers remain active during the native barrier. A contended lease must
        // resume the actor and report busy; retrying that result keeps the real close race here.
        self.state
            .session_lock_requested
            .store(true, Ordering::Release);
        let barrier = bounded("native close barrier", async {
            loop {
                match shutdown::freeze_servers(&self.state).await {
                    Ok(barrier) => break barrier,
                    Err(error) if error.contains("busy") => {
                        SystemClock.sleep(Duration::from_millis(20)).await
                    }
                    Err(error) => panic!("native close refused: {error}"),
                }
            }
        })
        .await;
        barrier.stop();
        for task in self.background.drain(..) {
            task.abort();
            let _ = task.await;
        }
        bounded("actor stop after saved close", self.task.take().unwrap())
            .await
            .unwrap();
        bounded("event drain after close", self.drain.take().unwrap())
            .await
            .unwrap();
        self.state.servers.lock().await.clear();
        self.state.reconnect_capture_signals.lock().unwrap().clear();
        self.state.persistence_signals.lock().unwrap().clear();
        self.state.store.lock().await.take();
    }

    pub async fn history(&self) -> History {
        let messages = bounded(
            "native message projection",
            self.actor.messages(channel_id("general")),
        )
        .await;
        let mut history = History::new();
        for message in messages {
            assert!(!message.id.is_empty(), "accepted messages have stable IDs");
            assert!(
                history
                    .insert(message.id, (message.author, message.text))
                    .is_none(),
                "duplicate message ID"
            );
        }
        history
    }

    pub async fn send(&self, token_number: u128, text: &str) -> String {
        let channel = channel_id("general").to_string();
        let token = hex::encode(token_number.to_be_bytes());
        let context = hex::encode(
            bounded("send authoring context", self.actor.durable_send_context())
                .await
                .unwrap(),
        );
        bounded("native durable acceptance", async {
            loop {
                let op = Operation::start(
                    None,
                    catcoms_diagnostics::Section::Channels,
                    "send_message",
                    1,
                    Some(&channel),
                );
                match durable_chat::send(
                    &self.state,
                    &op,
                    1,
                    &channel,
                    text.into(),
                    None,
                    Some(token.clone()),
                    Some(context.clone()),
                )
                .await
                {
                    Ok(outcome) => {
                        assert!(
                            outcome.accepted,
                            "fixture store must accept the native durable barrier"
                        );
                        assert_eq!(outcome.persistence, PersistOutcome::Durable);
                        break outcome.message_id.expect("durably accepted ID");
                    }
                    // Background persistence may momentarily own the native serializer. Retry the
                    // exact token and basis; a retry must never become a freshly authored message.
                    Err(error) if error.to_string().contains("busy") => {
                        SystemClock.sleep(Duration::from_millis(25)).await
                    }
                    Err(error) => panic!("native send refused: {error:?}"),
                }
            }
        })
        .await
    }
}
