//! Native text factory entry. Production selection stays OFF until authoring,
//! cold operational restore, joined sources and transport acceptance exist.
//! The warm fixture uses this SAME synchronous router and authentication flow;
//! its trusted Rust policy selects implementation, never grants authority.
use super::*;
use catcoms_app::store::{NativeTextVaultAuthenticationCheck, NativeUiInvocationBudget};
use tauri::ipc::{Invoke, InvokeBody};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

mod boot;
pub(super) mod retirement;
#[cfg(test)]
mod tests;
pub(super) mod ui_state;

const ENABLED: bool = false;
const MAX_SECRET: usize = 4096;
type Task = std::pin::Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

pub(super) struct Registry {
    pub(super) selected: bool,
    inflight: Arc<Semaphore>,
    bootstrap: Arc<StdMutex<Option<boot::Bootstrap>>>,
    ui_budget: NativeUiInvocationBudget,
    pub(super) ui_state: ui_state::Controller,
    retirement: retirement::Coalescer,
    pub(super) preludes: Arc<retirement::Prelude>,
    #[cfg(test)]
    directory: Option<PathBuf>,
    #[cfg(test)]
    final_edge: Option<Arc<tests::AdmissionBarrier>>,
    #[cfg(test)]
    reply_edge: StdMutex<Option<Arc<tests::AdmissionBarrier>>>,
    #[cfg(test)]
    retirement_queue: StdMutex<Option<Arc<StdMutex<Vec<Task>>>>>,
    #[cfg(test)]
    retirement_enqueued: tokio::sync::Notify,
    #[cfg(test)]
    pub(super) lock_prelude_edge: StdMutex<Option<Arc<tests::AdmissionBarrier>>>,
    #[cfg(test)]
    pub(super) lock_commit_edge: StdMutex<Option<Arc<tests::AdmissionBarrier>>>,
    #[cfg(test)]
    panic_retirement_publication: AtomicBool,
    #[cfg(test)]
    pub(super) unlock_commit_wait: tokio::sync::Notify,
    #[cfg(test)]
    ticket_edge: StdMutex<Option<Arc<tests::AdmissionBarrier>>>,
}
impl Default for Registry {
    fn default() -> Self {
        use catcoms_app::store::CriticalCatalogReadPool;
        let retirement = retirement::Coalescer::default();
        let preludes = retirement.prelude();
        let ui_budget = NativeUiInvocationBudget::new(
            CriticalCatalogReadPool::new(NativeUiInvocationBudget::required_capacity())
                .expect("fixed native UI policy"),
        )
        .expect("fixed native UI capacity");
        let ui_state =
            ui_state::Controller::new(ui_budget.prepare().expect("UI-state control payment"));
        Self {
            selected: false,
            inflight: Arc::new(Semaphore::new(1)),
            bootstrap: Arc::new(StdMutex::new(Some(boot::Bootstrap::Fresh))),
            ui_budget,
            ui_state,
            retirement,
            preludes,
            #[cfg(test)]
            directory: None,
            #[cfg(test)]
            final_edge: None,
            #[cfg(test)]
            reply_edge: StdMutex::new(None),
            #[cfg(test)]
            retirement_queue: StdMutex::new(None),
            #[cfg(test)]
            retirement_enqueued: tokio::sync::Notify::new(),
            #[cfg(test)]
            lock_prelude_edge: StdMutex::new(None),
            #[cfg(test)]
            lock_commit_edge: StdMutex::new(None),
            #[cfg(test)]
            panic_retirement_publication: AtomicBool::new(false),
            #[cfg(test)]
            unlock_commit_wait: tokio::sync::Notify::new(),
            #[cfg(test)]
            ticket_edge: StdMutex::new(None),
        }
    }
}

struct Captured {
    generation: u64,
    secret: Zeroizing<Vec<u8>>,
    _permit: OwnedSemaphorePermit,
}

fn arguments(payload: &InvokeBody, generation: u64) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    let InvokeBody::Json(value) = payload else {
        return Err("Invalid unlock request.");
    };
    let object = value.as_object().ok_or("Invalid unlock request.")?;
    if object.len() > 3
        || object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "passphrase" | "expectedUiGeneration" | "trace"
            )
        })
    {
        return Err("Invalid unlock request.");
    }
    let supplied = object
        .get("expectedUiGeneration")
        .and_then(serde_json::Value::as_str)
        .ok_or("Missing lock generation.")?;
    if parse_unlock_generation(supplied)? != generation {
        return Err("Unlock was superseded. Try again.");
    }
    let secret = object
        .get("passphrase")
        .and_then(serde_json::Value::as_str)
        .ok_or("Missing secret.")?;
    if secret.is_empty() {
        return Err("Enter your vault secret.");
    }
    if secret.len() > MAX_SECRET {
        return Err("The secret is too long.");
    }
    if object
        .get("trace")
        .is_some_and(|trace| !trace.is_null() && trace.as_str().is_none_or(|v| v.len() > 128))
    {
        return Err("Invalid unlock request.");
    }
    Ok(Zeroizing::new(secret.as_bytes().to_vec()))
}

/// Trusted startup initialization, before manage(), captures or any task. Zero
/// remains Root's unauthenticated marker. Never normalize a captured invoke to
/// a new generation in the router or its asynchronous continuation.
pub(super) fn initial_state() -> AppState {
    initial_state_selected(ENABLED)
}
fn initial_state_selected(enabled: bool) -> AppState {
    let mut state = AppState::default();
    state.native_text.selected = enabled;
    if enabled {
        state.ui_session_generation.store(1, Ordering::SeqCst);
        state.ui_authorized_generation.store(0, Ordering::SeqCst);
        state.session_lock_requested.store(true, Ordering::SeqCst);
    }
    state
}

pub(super) fn with_commands<R: tauri::Runtime>(
    fallback: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    router(fallback, ENABLED, |task| {
        drop(tauri::async_runtime::spawn(task));
    })
}

// Scheduling is the only test seam. No bool/result replaces window/ACL,
// passphrase, original generation, mounted owner or final authorization checks.
fn router<R: tauri::Runtime>(
    fallback: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
    enabled: bool,
    spawn: impl Fn(Task) + Send + Sync + 'static,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        if enabled
            && matches!(
                invoke.message.command(),
                "lock_session" | "close_vault_window"
            )
        {
            return ui_state::snapshots::route(invoke, &spawn);
        }
        if enabled && matches!(invoke.message.command(), "get_ui_state" | "save_ui_state") {
            return ui_state::route(invoke, &spawn);
        }
        if !enabled || invoke.message.command() != "unlock" {
            return fallback(invoke);
        }
        if invoke.acl.is_none()
            || invoke.message.webview_ref().label() != "main"
            || invoke.message.webview_ref().window().label() != "main"
        {
            invoke
                .resolver
                .reject("Unlock is not authorized from this window.");
            return true;
        }
        let app = invoke.message.webview_ref().app_handle().clone();
        let captured = {
            let state = app.state::<AppState>();
            // BEFORE parsing or scheduling: a completely unpolled old invoke
            // cannot adopt the authorization of a subsequent lock/unlock cycle.
            let generation = state.ui_session_generation.load(Ordering::SeqCst);
            let captured = state
                .native_text
                .inflight
                .clone()
                .try_acquire_owned()
                .map_err(|_| "Unlock is already running.")
                .and_then(|permit| {
                    arguments(invoke.message.payload(), generation).map(|secret| Captured {
                        generation,
                        secret,
                        _permit: permit,
                    })
                });
            match captured {
                Ok(captured) => captured,
                Err(error) => {
                    invoke.resolver.reject(error);
                    return true;
                }
            }
        };
        let directory = {
            #[cfg(test)]
            let override_path = app.state::<AppState>().native_text.directory.clone();
            #[cfg(not(test))]
            let override_path: Option<PathBuf> = None;
            match override_path
                .map(Ok)
                .unwrap_or_else(|| app.path().app_data_dir().map(|p| p.join("vault")))
            {
                Ok(directory) => directory,
                Err(_) => {
                    invoke.resolver.reject("Could not locate the vault.");
                    return true;
                }
            }
        };
        let resolver = invoke.resolver;
        drop(invoke.message); // retain only one bounded secret, never arbitrary JSON
        spawn(Box::pin(async move {
            let state = app.state::<AppState>();
            let mut notice = None;
            let result = execute(&state, &captured, directory, &mut notice).await;
            #[cfg(test)]
            {
                let edge = state.native_text.reply_edge.lock().unwrap().clone();
                if let Some(edge) = edge {
                    edge.wait_once().await;
                }
            }
            let mut _retired_notice = None;
            let _commit = state.ui_session_commit.lock().await;
            if state.ui_session_generation.load(Ordering::SeqCst) != captured.generation {
                resolver.reject("Unlock was superseded. Try again.");
            } else {
                let current = if let Some(n) = notice.as_ref() {
                    state.native_text.preludes.is_idle_cached()
                        && state
                            .native_text
                            .ui_state
                            .snapshots
                            .notice_is_current(&state, n)
                            .await
                } else {
                    false
                };
                match result {
                    Ok(rows) => resolver.resolve(rows),
                    Err(error) => resolver.reject(error),
                }
                if current
                    && state.ui_session_generation.load(Ordering::SeqCst) == captured.generation
                {
                    _retired_notice = state
                        .native_text
                        .ui_state
                        .snapshots
                        .publish_notice(notice.as_ref().unwrap());
                }
            }
        }));
        true
    }
}

async fn authenticate_mounted(
    state: &AppState,
    secret: &[u8],
) -> Result<Option<NativeTextVaultAuthenticationCheck>, String> {
    // Unique payment before capture; every early return drops it only AFTER
    // the inner Store guard. Root returns an infallible whole capsule even Busy.
    let credit = state
        .native_text
        .ui_budget
        .prepare()
        .map_err(|_| "Vault authentication is busy. Try again.")?;
    let work = {
        let guard = state.store.lock().await;
        match guard.as_ref() {
            None => return Ok(None),
            Some(MountedVault::TextV2(vault)) => vault.prepare_native_authentication(credit),
            Some(MountedVault::Legacy(_)) => {
                return Err("This vault is using the existing chat backend.".into())
            }
        }
    };
    let secret = Zeroizing::new(secret.to_vec());
    tauri::async_runtime::spawn_blocking(move || work.verify(&secret))
        .await
        .map_err(|_| "Vault authentication failed.")?
        .map(Some)
        .map_err(|_| "Could not unlock the vault.".into())
}

async fn execute(
    state: &AppState,
    captured: &Captured,
    directory: PathBuf,
    notice: &mut Option<ui_state::snapshots::Notice>,
) -> Result<Vec<ReloadedServer>, String> {
    let _mount = state.vault_mount.lock().await;
    // An older lock can publish this generation before revoking voice state.
    // Snapshot its original ticket only AFTER that mutation prelude ends, but
    // BEFORE KDF/verification, so authentication cannot adopt a later Host.
    state.native_text.preludes.wait_idle().await;
    if state.ui_session_generation.load(Ordering::SeqCst) != captured.generation {
        return Err("Unlock was superseded. Try again.".into());
    }
    let ticket = voice_runtime(state).session_ticket();
    #[cfg(test)]
    {
        let edge = state.native_text.ticket_edge.lock().unwrap().clone();
        if let Some(edge) = edge {
            edge.wait_once().await;
        }
    }
    let mut authentication = authenticate_mounted(state, &captured.secret).await?;
    if authentication.is_none() {
        let mut plan = {
            let mut slot = state
                .native_text
                .bootstrap
                .lock()
                .map_err(|_| "Vault startup is unavailable.")?;
            slot.take().ok_or("Vault startup is already running.")?
        };
        let secret = captured.secret.clone();
        let atoms = (
            state.ui_session_generation.clone(),
            state.ui_authorized_generation.clone(),
            state.session_lock_requested.clone(),
        );
        let budget = state.native_text.ui_budget.clone();
        // Whole original storage work, outside store/UI/operation guards. A
        // waiter disappearing does not cancel this owned native router task.
        let result = tauri::async_runtime::spawn_blocking(move || {
            let result = plan.advance(&directory, &secret, atoms, budget);
            (plan, result)
        })
        .await;
        let (mut plan, outcome) = match result {
            Ok(result) => result,
            Err(_) => {
                *state
                    .native_text
                    .bootstrap
                    .lock()
                    .map_err(|_| "Vault startup is unavailable.")? = Some(boot::Bootstrap::Failed);
                return Err("Vault startup failed. Its files have been kept.".into());
            }
        };
        let ready = plan.take_ready();
        {
            let mut slot = state
                .native_text
                .bootstrap
                .lock()
                .map_err(|_| "Vault startup is unavailable.")?;
            *slot = Some(plan); // previous value is None; no owner destroyed here
        }
        if let Some(vault) = ready {
            let mut owner = Some(vault);
            {
                let mut guard = state.store.lock().await;
                if guard.is_none() {
                    *guard = owner.take().map(MountedVault::TextV2);
                }
            }
            // Destruction of a losing actual owner is outside the store guard.
            if owner.is_some() {
                drop(owner);
                return Err("The mounted vault changed.".into());
            }
        }
        outcome?;
        // Retrying retained metadata work is NOT authentication. Even if a
        // previous correct secret created the plan, THIS caller must verify
        // its own secret against the same completed mounted keystore.
        authentication = authenticate_mounted(state, &captured.secret).await?;
    }
    let authentication = Arc::new(authentication.ok_or("Vault startup did not complete.")?);
    #[cfg(test)]
    if let Some(edge) = &state.native_text.final_edge {
        edge.wait_once().await;
    }
    finalize_unlock_session_with_voice_checked(
        state,
        captured.generation,
        false,
        &ticket,
        || authentication.is_live_cached(),
        Some(authentication.clone()),
        Some(notice),
    )
    .await?;
    // This warm milestone admits only a fresh empty vault; no fake Legacy
    // actors are created from index DATA. Native authoring/restore come next.
    Ok(Vec::new())
}
