//! Native close barrier: save every actor, freeze further acceptance, then destroy the window.
use super::*;
use catcoms_app::shutdown::{FrozenServer, ShutdownLease};
use tokio::sync::OwnedMutexGuard;

const CLOSE_SAVE_DEADLINE: Duration = Duration::from_secs(10);

pub(super) struct ShutdownBarrier {
    frozen: Vec<FrozenServer>,
    _session: OwnedMutexGuard<()>,
    _registry: OwnedMutexGuard<HashMap<u64, ServerEntry>>,
}

impl ShutdownBarrier {
    pub(super) fn stop(self) {
        for server in self.frozen {
            server.stop();
        }
    }
}

fn lease(
    state: &AppState,
    id: u64,
    instance: u64,
    generation: u64,
) -> Result<ShutdownLease, String> {
    let busy =
        || "Message history is busy saving. Keep Mewtual open and retry closing.".to_string();
    let persist = persist_lock_for(state, id)
        .try_lock_owned()
        .map_err(|_| busy())?;
    let session = state
        .ui_session_commit
        .clone()
        .try_lock_owned()
        .map_err(|_| busy())?;
    let store = state.store.clone().try_lock_owned().map_err(|_| busy())?;
    let registry = state.servers.clone().try_lock_owned().map_err(|_| busy())?;
    if !state.session_lock_requested.load(Ordering::Acquire)
        || state.ui_session_generation.load(Ordering::Acquire) != generation
        || registry
            .get(&id)
            .is_none_or(|entry| entry.instance != instance)
    {
        return Err("The vault or group changed while closing; retry closing.".into());
    }
    Ok(ShutdownLease::new(store, id, (persist, session, registry)))
}

pub(super) async fn freeze_servers(state: &AppState) -> Result<ShutdownBarrier, String> {
    let generation = state.ui_session_generation.load(Ordering::Acquire);
    let save = async {
        // Admission may have completed just before close, ahead of the periodic route worker.
        // Save the authenticated policy and actual outbound listener evidence while every actor
        // can still serve its neighbour's connected-only proof request.
        member_reconnect::before_shutdown(state).await?;
        let mut actors: Vec<_> = state
            .servers
            .lock()
            .await
            .iter()
            .map(|(id, entry)| (*id, entry.instance, entry.actor.clone()))
            .collect();
        actors.sort_unstable_by_key(|(id, _, _)| *id);
        let mut frozen = Vec::with_capacity(actors.len());
        for (id, instance, actor) in &actors {
            let ready = actor.prepare_shutdown().await?;
            // No await acquiring locks once Ready owns the actor. Contention drops Ready and
            // every earlier frozen handle, resuming all actors for a later close attempt.
            let custody = lease(state, *id, *instance, generation)?;
            frozen.push(ready.save_and_freeze(custody).await?);
        }
        let session = state
            .ui_session_commit
            .clone()
            .try_lock_owned()
            .map_err(|_| "The vault is busy; retry closing.".to_string())?;
        let registry = state
            .servers
            .clone()
            .try_lock_owned()
            .map_err(|_| "The group list is busy; retry closing.".to_string())?;
        if !state.session_lock_requested.load(Ordering::Acquire)
            || state.ui_session_generation.load(Ordering::Acquire) != generation
            || registry.len() != actors.len()
            || actors.iter().any(|(id, instance, _)| {
                registry
                    .get(id)
                    .is_none_or(|entry| entry.instance != *instance)
            })
        {
            return Err("The vault or group changed while closing; retry closing.".into());
        }
        Ok(ShutdownBarrier {
            frozen,
            _session: session,
            _registry: registry,
        })
    };
    tokio::select! {
        result = save => result,
        _ = SystemClock.sleep(CLOSE_SAVE_DEADLINE) => Err("Message history could not finish saving in time. Keep Mewtual open and retry closing.".into()),
    }
}
