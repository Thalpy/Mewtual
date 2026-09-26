//! Recover individual admission writes without changing their multi-file transaction boundary.
use super::*;

#[derive(Clone)]
pub(super) struct PendingServerNet {
    instance: u64,
    net: ServerNet,
    /// The predecessor seen by the failed write. A retry may replace this exact version, but
    /// concurrent successful route/consent updates take precedence over stale admission fields.
    observed: Option<ServerNet>,
}

fn pending(reason: PersistFailure) -> PersistOutcome {
    PersistOutcome::Pending { reason }
}

pub(super) fn combine_retries(
    first: Option<PersistOutcome>,
    second: Option<PersistOutcome>,
) -> Option<PersistOutcome> {
    match (first, second) {
        (Some(outcome @ PersistOutcome::Pending { .. }), _)
        | (_, Some(outcome @ PersistOutcome::Pending { .. })) => Some(outcome),
        (Some(PersistOutcome::Superseded), _) | (_, Some(PersistOutcome::Superseded)) => {
            Some(PersistOutcome::Superseded)
        }
        (Some(PersistOutcome::Durable), _) | (_, Some(PersistOutcome::Durable)) => {
            Some(PersistOutcome::Durable)
        }
        (None, None) => None,
    }
}

pub(super) fn remove_pending_net(state: &AppState, server: u64, instance: u64) {
    let mut pending = state
        .pending_server_nets
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if pending
        .get(&server)
        .is_some_and(|entry| entry.instance == instance)
    {
        pending.remove(&server);
    }
}

pub(super) async fn prune_stale_pending(state: &AppState) {
    let registry = state.servers.lock().await;
    retain_current(state, &registry);
}

fn retain_current(state: &AppState, registry: &HashMap<u64, ServerEntry>) {
    state
        .pending_server_nets
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|id, pending| {
            registry
                .get(id)
                .is_some_and(|entry| entry.instance == pending.instance)
        });
}

fn matches_live_identity(entry: &ServerEntry, net: &ServerNet) -> bool {
    // Real installed network actors carry at least one of these bindings. Pure local/test
    // actors have neither, and are fenced by the explicitly supplied incarnation instead.
    let Ok(key) = keypair_from_seed(net.key_seed) else {
        return false;
    };
    let libp2p_peer = key.public().to_peer_id();
    entry
        .mesh
        .as_ref()
        .is_none_or(|mesh| mesh.local_peer() == phase0_peer_id(&libp2p_peer))
        && entry
            .interface_routes
            .as_ref()
            .is_none_or(|route| route.peer_id == libp2p_peer.to_string())
}

pub(super) async fn persist_server_net(
    state: &AppState,
    server: u64,
    instance: u64,
    net: &ServerNet,
) -> PersistOutcome {
    let _writing = persist_lock_for(state, server).lock_owned().await;
    let guard = state.store.lock().await;
    let registry = state.servers.lock().await;
    retain_current(state, &registry);
    let Some(entry) = registry
        .get(&server)
        .filter(|entry| entry.instance == instance)
    else {
        return PersistOutcome::Superseded;
    };
    if !matches_live_identity(entry, net) {
        return PersistOutcome::Superseded;
    }
    let observed = match guard
        .as_ref()
        .map(|store| store.load_server_net(server))
        .transpose()
    {
        Ok(value) => value.flatten(),
        Err(error) => {
            tracing::error!(target: "catcoms_app", server, %error, "VAULT.NET_IDENTITY.LOAD_FAILED");
            // Retain the original seed even when the destination cannot be read. Never replace
            // a previously retained newer obligation with this older invocation.
            let mut slots = state
                .pending_server_nets
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            slots.entry(server).or_insert_with(|| PendingServerNet {
                instance,
                net: net.clone(),
                observed: None,
            });
            return pending(PersistFailure::WriteFailed);
        }
    };
    let mut candidate = net.clone();
    if let Some(current) = &observed {
        if current.key_seed != net.key_seed {
            // A conflicting record must not silently switch this running transport's identity.
            state
                .pending_server_nets
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(server)
                .or_insert_with(|| PendingServerNet {
                    instance,
                    net: net.clone(),
                    observed: observed.clone(),
                });
            return pending(PersistFailure::WriteFailed);
        }
        candidate = current.clone();
        candidate.record_seq = candidate.record_seq.max(net.record_seq);
        // The only current callers write initial admission or a changed restored listen port.
        // Current reconnect consent/routes are owned by their own guarded transitions.
        if net.record_seq >= current.record_seq {
            candidate.port = net.port;
        }
    }
    {
        let mut slots = state
            .pending_server_nets
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(older) = slots.get(&server) {
            if older.net.key_seed != candidate.key_seed
                || older.net.record_seq > candidate.record_seq
            {
                return pending(PersistFailure::WriteFailed);
            }
        }
        slots.insert(
            server,
            PendingServerNet {
                instance,
                net: candidate.clone(),
                observed,
            },
        );
    }
    let Some(store) = guard.as_ref() else {
        return pending(PersistFailure::StoreUnavailable);
    };
    write_net(state, store, server, instance, &candidate)
}

fn write_net(
    state: &AppState,
    store: &ServerStore,
    server: u64,
    instance: u64,
    net: &ServerNet,
) -> PersistOutcome {
    match store.save_server_net(server, net, &mut OsCryptoRng) {
        Ok(()) => {
            remove_pending_net(state, server, instance);
            PersistOutcome::Durable
        }
        Err(error) => {
            tracing::error!(target: "catcoms_app", server, %error, "VAULT.NET_IDENTITY.SEAL_FAILED");
            pending(PersistFailure::WriteFailed)
        }
    }
}

/// No actor await: callers must invoke this before taking the vault/registry guards. The
/// numeric-id serializer and final registry guard fence both replacement and leave.
pub(super) async fn retry_server_net(
    state: &AppState,
    server: u64,
    instance: u64,
) -> Option<PersistOutcome> {
    {
        let slots = state
            .pending_server_nets
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if slots
            .get(&server)
            .is_none_or(|pending| pending.instance != instance)
        {
            return None;
        }
    }
    let _writing = persist_lock_for(state, server).lock_owned().await;
    let guard = state.store.lock().await;
    let registry = state.servers.lock().await;
    retain_current(state, &registry);
    let Some(entry) = registry
        .get(&server)
        .filter(|entry| entry.instance == instance)
    else {
        return Some(PersistOutcome::Superseded);
    };
    let retained = state
        .pending_server_nets
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&server)
        .filter(|pending| pending.instance == instance)
        .cloned()?;
    if !matches_live_identity(entry, &retained.net) {
        return Some(pending(PersistFailure::WriteFailed));
    }
    let Some(store) = guard.as_ref() else {
        return Some(pending(PersistFailure::StoreUnavailable));
    };
    let current = match store.load_server_net(server) {
        Ok(net) => net,
        Err(error) => {
            tracing::error!(target: "catcoms_app", server, %error, "VAULT.NET_IDENTITY.LOAD_FAILED");
            return Some(pending(PersistFailure::WriteFailed));
        }
    };
    let mut candidate = retained.net;
    if let Some(current) = &current {
        if current.key_seed != candidate.key_seed {
            return Some(pending(PersistFailure::WriteFailed));
        }
    }
    if current != retained.observed {
        if let Some(mut newer) = current {
            // Never replay stale port, consent, recovery-code or route fields over a successful
            // newer write. Retaining the same identity and monotonic reservation is sufficient.
            newer.record_seq = newer.record_seq.max(candidate.record_seq);
            candidate = newer;
        }
    }
    Some(write_net(state, store, server, instance, &candidate))
}

pub(super) async fn persist_registry(state: &AppState) -> PersistOutcome {
    state
        .registry_persist
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .request();
    write_registry(state).await
}

pub(super) async fn retry_registry(state: &AppState) -> Option<PersistOutcome> {
    {
        let counters = state
            .registry_persist
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !counters.needs_write(counters.requested) {
            return None;
        }
    }
    Some(write_registry(state).await)
}

async fn write_registry(state: &AppState) -> PersistOutcome {
    let guard = state.store.lock().await;
    let Some(store) = guard.as_ref() else {
        return pending(PersistFailure::StoreUnavailable);
    };
    let registry = state.servers.lock().await;
    let covering = state
        .registry_persist
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .requested;
    let records: Vec<_> = registry
        .iter()
        .map(|(id, entry)| ServerRecord {
            id: *id,
            display_name: entry.name.clone(),
            invite: entry.invite.clone().unwrap_or_default(),
            is_dm: entry.is_dm,
        })
        .collect();
    if let Err(error) = store.save_registry(&records, &mut OsCryptoRng) {
        tracing::error!(target: "catcoms_app", %error, "VAULT.REGISTRY.SEAL_FAILED");
        return pending(PersistFailure::WriteFailed);
    }
    state
        .registry_persist
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .completed_through(covering);
    PersistOutcome::Durable
}

/// Called by the existing close coordinator before it freezes actors. Failed admission files
/// keep close pending; this is a retry of individual files, not a cross-file transaction.
pub(super) async fn before_shutdown(state: &AppState) -> Result<(), String> {
    let entries: Vec<_> = state
        .servers
        .lock()
        .await
        .iter()
        .map(|(id, entry)| (*id, entry.instance))
        .collect();
    for (server, instance) in entries {
        if retry_server_net(state, server, instance)
            .await
            .is_some_and(|outcome| outcome != PersistOutcome::Durable)
        {
            return Err("A conversation's network identity could not finish saving. Keep Mewtual open and retry closing.".into());
        }
    }
    if retry_registry(state)
        .await
        .is_some_and(|outcome| outcome != PersistOutcome::Durable)
    {
        return Err(
            "The conversation list could not finish saving. Keep Mewtual open and retry closing."
                .into(),
        );
    }
    Ok(())
}

/// Pre-transport reload has no installed incarnation to own a pending seed. Reserve durably
/// before returning it, and refuse startup on read/write errors instead of minting a new identity.
pub(super) async fn load_or_init_server_net(
    state: &AppState,
    server: u64,
    fallback_rendezvous: &str,
) -> Result<ServerNet, String> {
    let _writing = persist_lock_for(state, server).lock_owned().await;
    let guard = state.store.lock().await;
    let store = guard
        .as_ref()
        .ok_or_else(|| "vault is not mounted".to_string())?;
    let registry = state.servers.lock().await;
    if registry.contains_key(&server) {
        return Err(
            "conversation is already running; its transport identity was not replaced".into(),
        );
    }
    let mut net = store
        .load_server_net(server)
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| new_server_net("", "", fallback_rendezvous));
    net.reserve_record_seq_block();
    store
        .save_server_net(server, &net, &mut OsCryptoRng)
        .map_err(|error| {
            format!("network identity sequence reservation could not finish saving: {error}")
        })?;
    Ok(net)
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::{Hub, ManualClock};
    use std::future::poll_fn;
    use std::task::Poll;

    struct Running {
        actor: ServerActor,
        task: tokio::task::JoinHandle<()>,
        drain: tokio::task::JoinHandle<()>,
    }

    async fn install(state: &AppState, instance: u64, net: &ServerNet) -> Running {
        let peer = keypair_from_seed(net.key_seed)
            .unwrap()
            .public()
            .to_peer_id();
        let server = Server::found(
            Hub::new().join(phase0_peer_id(&peer)),
            MlsDevice::generate().unwrap(),
            OsCryptoRng,
            Box::new(ManualClock::new(1_000)),
            "test",
        )
        .unwrap();
        let group_id = server.group_id();
        let device_id = server.device_id();
        let (actor, mut events, task) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        state.servers.lock().await.insert(
            1,
            ServerEntry {
                actor: actor.clone(),
                instance,
                group_id,
                device_id,
                invite: Some("retained invite".into()),
                name: "first".into(),
                bootstrap: Vec::new(),
                bootstrap_owners: HashMap::new(),
                interface_routes: Some(InterfaceRouteIdentity {
                    port: net.port,
                    peer_id: peer.to_string(),
                }),
                rendezvous: Vec::new(),
                mesh: None,
                is_dm: false,
                switchboard: false,
                record_seq: net.record_seq,
                persist: PersistCounters::default(),
            },
        );
        Running { actor, task, drain }
    }

    async fn stop(running: Running) {
        running.actor.shutdown().await;
        running.task.await.unwrap();
        running.drain.await.unwrap();
    }

    async fn mount(state: &AppState, path: &Path) {
        *state.store.lock().await =
            Some(ServerStore::open(path, b"admission test", &mut OsCryptoRng).unwrap());
    }

    async fn saved_net(state: &AppState) -> ServerNet {
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

    #[tokio::test]
    async fn failed_initial_identity_and_registry_recover_on_quiet_persistence_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::default();
        mount(&state, dir.path()).await;
        let net = new_server_net("", "", "");
        let expected = net.clone();
        let running = install(&state, 10, &net).await;
        // A file at the parent of .net makes the actual encrypted atomic write fail after its
        // absent-record read. No synthetic store or I/O implementation is involved.
        let servers = dir.path().join("servers");
        let parked = dir.path().join("servers-held");
        std::fs::rename(&servers, &parked).unwrap();
        std::fs::write(&servers, b"block network identity writes").unwrap();
        std::fs::create_dir(dir.path().join("registry.bin")).unwrap();
        assert_eq!(
            super::super::persist_server_net(&state, 1, &net).await,
            pending(PersistFailure::WriteFailed)
        );
        assert_eq!(
            super::super::persist_registry(&state).await,
            pending(PersistFailure::WriteFailed)
        );
        drop(net); // The admission-local value that used to be the only copy is gone.
        assert_eq!(state.pending_server_nets.lock().unwrap().len(), 1);
        assert!(before_shutdown(&state).await.is_err());
        for _ in 0..3 {
            assert_eq!(
                retry_pending_persistence(&state, 1, 10).await,
                Some(pending(PersistFailure::WriteFailed))
            );
        }
        assert_eq!(state.pending_server_nets.lock().unwrap().len(), 1);
        assert_eq!(state.registry_persist.lock().unwrap().requested, 1);
        std::fs::remove_file(&servers).unwrap();
        std::fs::rename(&parked, &servers).unwrap();
        // A registry error is still an outstanding close obligation after .net recovers.
        assert!(before_shutdown(&state).await.is_err());
        assert_eq!(saved_net(&state).await, expected);
        std::fs::remove_dir(dir.path().join("registry.bin")).unwrap();
        assert_eq!(
            retry_pending_persistence(&state, 1, 10).await,
            Some(PersistOutcome::Durable)
        );
        assert_eq!(retry_pending_persistence(&state, 1, 10).await, None);
        assert!(state.pending_server_nets.lock().unwrap().is_empty());
        assert_eq!(
            *state.registry_persist.lock().unwrap(),
            PersistCounters {
                requested: 1,
                completed: 1
            }
        );
        before_shutdown(&state).await.unwrap();
        stop(running).await;
        state.store.lock().await.take();
        let reopened = ServerStore::open(dir.path(), b"admission test", &mut OsCryptoRng).unwrap();
        assert_eq!(reopened.load_server_net(1).unwrap().unwrap(), expected);
        let records = reopened.load_registry().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].invite, "retained invite");
    }

    #[tokio::test]
    async fn retry_preserves_newer_successful_sequence_routes_and_consent() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let net = new_server_net("", "", "");
        let running = install(&state, 11, &net).await;
        assert_eq!(
            persist_server_net(&state, 1, 11, &net).await,
            pending(PersistFailure::StoreUnavailable)
        );
        mount(&state, dir.path()).await;
        let mut newer = net.clone();
        newer.record_seq += 9;
        newer.port = newer.port.saturating_add(1);
        newer.reconnect_policy = ReconnectPolicy::MemberMesh;
        newer.switchboard = true;
        newer.pending_recovery_peer = Some([19; 32]);
        newer.pending_recovery_expires_at_ms = 120_000;
        newer.reconnect_routes = vec![ReconnectRoute {
            peer_id: [17; 32],
            address: "/ip4/192.168.1.20/tcp/43001".into(),
        }];
        state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .save_server_net(1, &newer, &mut OsCryptoRng)
            .unwrap();
        assert_eq!(
            retry_server_net(&state, 1, 11).await,
            Some(PersistOutcome::Durable)
        );
        assert_eq!(saved_net(&state).await, newer);
        // A delayed ordinary caller cannot roll the reservation or the newer metadata back.
        assert_eq!(
            persist_server_net(&state, 1, 11, &net).await,
            PersistOutcome::Durable
        );
        assert_eq!(saved_net(&state).await, newer);
        assert!(state.pending_server_nets.lock().unwrap().is_empty());
        stop(running).await;
    }

    #[tokio::test]
    async fn queued_old_identity_retry_cannot_overwrite_replacement_or_remove_its_pending_seed() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let old_net = new_server_net("", "", "");
        let old = install(&state, 12, &old_net).await;
        persist_server_net(&state, 1, 12, &old_net).await;
        let writing = persist_lock_for(&state, 1).lock_owned().await;
        let mut late = Box::pin(retry_server_net(&state, 1, 12));
        poll_fn(|cx| {
            assert!(late.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let new_net = new_server_net("", "", "");
        let new = install(&state, 13, &new_net).await;
        mount(&state, dir.path()).await;
        state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .save_server_net(1, &new_net, &mut OsCryptoRng)
            .unwrap();
        drop(writing);
        assert_eq!(late.await, Some(PersistOutcome::Superseded));
        assert_eq!(
            persist_server_net(&state, 1, 12, &old_net).await,
            PersistOutcome::Superseded
        );
        assert_eq!(
            super::super::persist_server_net(&state, 1, &old_net).await,
            PersistOutcome::Superseded
        );
        assert_eq!(saved_net(&state).await, new_net);
        state.store.lock().await.take();
        persist_server_net(&state, 1, 13, &new_net).await;
        remove_pending_net(&state, 1, 12);
        assert_eq!(state.pending_server_nets.lock().unwrap()[&1].instance, 13);
        remove_pending_net(&state, 1, 13);
        assert!(state.pending_server_nets.lock().unwrap().is_empty());
        stop(old).await;
        stop(new).await;
    }

    #[tokio::test]
    async fn conflicting_saved_seed_is_never_silently_adopted_or_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::default();
        let net = new_server_net("", "", "");
        let running = install(&state, 14, &net).await;
        persist_server_net(&state, 1, 14, &net).await;
        mount(&state, dir.path()).await;
        let foreign = new_server_net("", "", "");
        state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .save_server_net(1, &foreign, &mut OsCryptoRng)
            .unwrap();
        assert_eq!(
            retry_server_net(&state, 1, 14).await,
            Some(pending(PersistFailure::WriteFailed))
        );
        assert_eq!(saved_net(&state).await, foreign);
        assert_eq!(
            state.pending_server_nets.lock().unwrap()[&1].net.key_seed,
            net.key_seed
        );
        assert!(before_shutdown(&state).await.is_err());
        stop(running).await;
    }

    #[tokio::test]
    async fn failed_registry_retry_serializes_the_current_roster_without_ticket_inflation() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::default();
        mount(&state, dir.path()).await;
        let net = new_server_net("", "", "");
        let running = install(&state, 15, &net).await;
        std::fs::create_dir(dir.path().join("registry.bin")).unwrap();
        assert_eq!(
            persist_registry(&state).await,
            pending(PersistFailure::WriteFailed)
        );
        state.servers.lock().await.get_mut(&1).unwrap().name = "latest name".into();
        assert_eq!(
            persist_registry(&state).await,
            pending(PersistFailure::WriteFailed)
        );
        for _ in 0..3 {
            retry_registry(&state).await;
        }
        assert_eq!(
            *state.registry_persist.lock().unwrap(),
            PersistCounters {
                requested: 2,
                completed: 0
            }
        );
        std::fs::remove_dir(dir.path().join("registry.bin")).unwrap();
        assert_eq!(retry_registry(&state).await, Some(PersistOutcome::Durable));
        assert_eq!(
            state
                .store
                .lock()
                .await
                .as_ref()
                .unwrap()
                .load_registry()
                .unwrap()[0]
                .display_name,
            "latest name"
        );
        assert_eq!(
            *state.registry_persist.lock().unwrap(),
            PersistCounters {
                requested: 2,
                completed: 2
            }
        );
        assert_eq!(retry_registry(&state).await, None);
        stop(running).await;
    }

    #[tokio::test]
    async fn reload_never_returns_an_unreserved_or_corrupt_network_identity() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::default();
        mount(&state, dir.path()).await;
        let servers = dir.path().join("servers");
        let parked = dir.path().join("servers-held");
        std::fs::rename(&servers, &parked).unwrap();
        std::fs::write(&servers, b"block reservation write").unwrap();
        assert!(load_or_init_server_net(&state, 1, "")
            .await
            .unwrap_err()
            .contains("reservation"));
        assert!(state.pending_server_nets.lock().unwrap().is_empty());
        std::fs::remove_file(&servers).unwrap();
        std::fs::rename(&parked, &servers).unwrap();
        std::fs::write(servers.join("1.net"), b"corrupt original identity").unwrap();
        assert!(load_or_init_server_net(&state, 1, "").await.is_err());
        assert_eq!(
            std::fs::read(servers.join("1.net")).unwrap(),
            b"corrupt original identity"
        );
        std::fs::remove_file(servers.join("1.net")).unwrap();
        let first = load_or_init_server_net(&state, 1, "").await.unwrap();
        let second = load_or_init_server_net(&state, 1, "").await.unwrap();
        assert_eq!(first.key_seed, second.key_seed);
        assert!(second.record_seq > first.record_seq);
        assert_eq!(saved_net(&state).await, second);
        let running = install(&state, 16, &second).await;
        assert!(load_or_init_server_net(&state, 1, "")
            .await
            .unwrap_err()
            .contains("already running"));
        assert_eq!(saved_net(&state).await, second);
        stop(running).await;
    }
}
