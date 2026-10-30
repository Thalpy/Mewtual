//! Privileged retained originals, through a synchronous Tauri router rather than async command
//! argument parsing. Tauri parses async CommandArgs after scheduling: capturing custody there
//! would let an entirely unpolled pre-lock invoke adopt a later unlock/registry replacement.
use super::*;
use catcoms_app::admin_chat_ledger::{
    AdminLedgerDetail, AdminLedgerError, AdminLedgerPage, AdminLegacyRow, AdminReadBudget,
    AdminReadScope,
};
use tauri::ipc::{Invoke, InvokeBody, InvokeResponseBody, Response};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

mod dto;
#[cfg(test)]
mod tests;

/// The ONLY commands owned by the synchronous router. Security inventories union these names
/// with generate_handler's unchanged ordinary command set; neither command has a second handler.
pub(super) const COMMANDS: &[&str] = &["get_admin_chat_ledger", "get_admin_chat_ledger_detail"];
const INFLIGHT: usize = 8;
const CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// One active admin pane. Switching its server retires the previous exact native view; account
/// and registry-row cancellation remain independent parents. No history/body or strong Actor
/// sender is cached here: a read observer must not keep a removed server's history/task alive.
pub(super) struct Registry {
    active: StdMutex<Option<ViewOwner>>,
    inflight: Arc<Semaphore>,
    budget: Arc<AdminReadBudget>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            active: StdMutex::new(None),
            inflight: Arc::new(Semaphore::new(INFLIGHT)),
            budget: AdminReadBudget::new(CACHE_BYTES),
        }
    }
}
struct ViewOwner {
    generation: u64,
    server: u64,
    instance: u64,
    scope: AdminReadScope,
    origin: RequestCancellation,
    signal: watch::Sender<bool>,
}
impl Drop for ViewOwner {
    fn drop(&mut self) {
        self.signal.send_replace(true);
    }
}
impl Registry {
    /// Called synchronously when lock registers, before its first await. Sender retirement wakes
    /// quiet Actor caches and never waits for actor/store/registry I/O under this short mutex.
    pub(super) fn retire(&self) {
        drop(self.active.lock().unwrap_or_else(|e| e.into_inner()).take());
    }
}

struct Captured {
    generation: u64,
    server: u64,
    instance: u64,
    actor: ServerActor,
    scope: AdminReadScope,
    origin: RequestCancellation,
    _permit: OwnedSemaphorePermit,
}

/// Synchronous production entry: no future, actor call, body lookup or deferred row adoption.
fn capture(state: &AppState, server: u64, generation: u64) -> Result<Captured, &'static str> {
    let account = keep_session::current(state, generation).ok_or("session-unavailable")?;
    // Contention is a bounded unavailable read, never permission to choose whichever row exists
    // after waiting. Capture instance AND actual Actor while the original registry guard is held.
    let servers = state.servers.try_lock().map_err(|_| "busy")?;
    let entry = servers.get(&server).ok_or("server-unavailable")?;
    let row_origin = entry.files.scope(account);
    let mut active = state
        .admin_chat_ledger
        .active
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let reusable = active.as_ref().is_some_and(|owner| {
        // Native instances are monotonic; the independently owned original EntryOwner observer
        // is irreversible even in a fixture that deliberately reuses an instance number. Only
        // Captured owns the actual current Actor, obtained below under this same registry guard.
        owner.generation == generation
            && owner.server == server
            && owner.instance == entry.instance
            && !owner.origin.is_cancelled()
    });
    if !reusable {
        drop(active.take()); // retire the old pane BEFORE installing an independently scoped view
        let (signal, receiver) = watch::channel(false);
        let origin = keep_session::bind_read(row_origin, RequestCancellation::new(receiver, None));
        let scope = AdminReadScope::new(origin.clone(), state.admin_chat_ledger.budget.clone());
        *active = Some(ViewOwner {
            generation,
            server,
            instance: entry.instance,
            scope,
            origin,
            signal,
        });
    }
    let owner = active.as_ref().expect("installed above");
    if owner.origin.is_cancelled()
        || ui_session_closed(state)
        || state.ui_session_generation.load(Ordering::Acquire) != generation
    {
        drop(active.take());
        return Err("session-unavailable");
    }
    // Switching the single native pane must revoke old queued reads EVEN when all read permits
    // are held behind that old Actor. The first switch can return Busy; cancellation releases
    // those old permits without requiring its unrelated Actor barrier to make progress.
    let _permit = state
        .admin_chat_ledger
        .inflight
        .clone()
        .try_acquire_owned()
        .map_err(|_| "busy")?;
    Ok(Captured {
        generation,
        server,
        instance: entry.instance,
        actor: entry.actor.clone(),
        scope: owner.scope.clone(),
        origin: owner.origin.clone(),
        _permit,
    })
}

enum Request {
    Page {
        server: u64,
        cursor: Option<String>,
        limit: usize,
    },
    Detail {
        server: u64,
        row: String,
        revision: String,
        offset: usize,
    },
    /// A locatable malformed request still reaches the actual Actor permission check before
    /// invalid-reference metadata is returned. No attacker-sized field is retained in this case.
    Invalid { server: u64 },
}
impl Request {
    fn server(&self) -> u64 {
        match self {
            Self::Page { server, .. } | Self::Detail { server, .. } | Self::Invalid { server } => {
                *server
            }
        }
    }
}

fn safe_number(value: &serde_json::Value) -> Option<u64> {
    value.as_u64().filter(|value| *value <= MAX_SAFE_INTEGER)
}
fn arguments(command: &str, body: &InvokeBody) -> Result<Request, &'static str> {
    let InvokeBody::Json(body) = body else {
        return Err("invalid-request");
    };
    let object = body.as_object().ok_or("invalid-request")?;
    let server = object
        .get("server")
        .and_then(safe_number)
        .ok_or("invalid-request")?;
    let invalid = || Request::Invalid { server };
    if command == COMMANDS[0] {
        if object.len() != 3
            || !["server", "cursor", "limit"]
                .iter()
                .all(|key| object.contains_key(*key))
        {
            return Ok(invalid());
        }
        let cursor = match object.get("cursor") {
            Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(value)) if value.len() <= 512 => Some(value.clone()),
            _ => return Ok(invalid()),
        };
        let Some(limit) = object
            .get("limit")
            .and_then(safe_number)
            .and_then(|v| usize::try_from(v).ok())
        else {
            return Ok(invalid());
        };
        Ok(Request::Page {
            server,
            cursor,
            limit,
        })
    } else {
        if object.len() != 4
            || !["server", "rowId", "expectedRevision", "frameOffset"]
                .iter()
                .all(|key| object.contains_key(*key))
        {
            return Ok(invalid());
        }
        let Some(row) = object
            .get("rowId")
            .and_then(|v| v.as_str())
            .filter(|v| v.len() <= 111)
        else {
            return Ok(invalid());
        };
        let Some(revision) = object
            .get("expectedRevision")
            .and_then(|v| v.as_str())
            .filter(|v| v.len() <= 256)
        else {
            return Ok(invalid());
        };
        let Some(offset) = object
            .get("frameOffset")
            .and_then(safe_number)
            .and_then(|v| usize::try_from(v).ok())
        else {
            return Ok(invalid());
        };
        Ok(Request::Detail {
            server,
            row: row.to_owned(),
            revision: revision.to_owned(),
            offset,
        })
    }
}

fn unavailable(error: AdminLedgerError) -> dto::Reply {
    dto::Reply::Unavailable {
        code: match error {
            AdminLedgerError::SessionUnavailable => "session-unavailable",
            AdminLedgerError::NotAuthorized => "not-authorized",
            AdminLedgerError::InvalidRequest => "invalid-request",
            AdminLedgerError::Busy => "busy",
            AdminLedgerError::Capacity | AdminLedgerError::EvidenceUnavailable => {
                "evidence-unavailable"
            }
            AdminLedgerError::Stale => {
                unreachable!("stale obtains an actual current Actor revision")
            }
        },
    }
}

async fn execute(state: &AppState, captured: &Captured, request: Request) -> dto::Reply {
    let mut origin = captured.origin.clone();
    tokio::select! {
        biased;
        _ = origin.cancelled() => dto::Reply::Unavailable { code: "session-unavailable" },
        reply = async {
            // An entirely unpolled old task cannot adopt a newer unlock. Cancellation also
            // releases a read waiting at this initial commit fence, not just its Actor oneshot.
            let Ok(commit) = require_ui_session_generation(state, captured.generation).await else {
                return dto::Reply::Unavailable { code: "session-unavailable" };
            };
            if captured.origin.is_cancelled() {
                return dto::Reply::Unavailable { code: "session-unavailable" };
            }
            drop(commit);
            execute_actor(captured, request).await
        } => reply,
    }
}

/// A pure retained-history read has no ambiguous publication to preserve. Dropping this future
/// after original-scope retirement drops its queued reply receiver; the Actor still independently
/// refuses that cancelled scope if/when it eventually reaches the command.
async fn execute_actor(captured: &Captured, request: Request) -> dto::Reply {
    let result = match request {
        Request::Page { cursor, limit, .. } => captured
            .actor
            .admin_chat_ledger_page(captured.scope.clone(), cursor, limit)
            .await
            .and_then(dto::page),
        Request::Detail {
            row,
            revision,
            offset,
            ..
        } => captured
            .actor
            .admin_chat_ledger_detail(captured.scope.clone(), row, revision, offset)
            .await
            .and_then(dto::detail),
        Request::Invalid { .. } => {
            // The actual Server checks cancellation/role BEFORE the intentionally invalid limit.
            // This avoids a second role implementation or a fake positive authorization probe.
            match captured
                .actor
                .admin_chat_ledger_page(captured.scope.clone(), None, 0)
                .await
            {
                Err(error) => Err(error),
                Ok(_) => Err(AdminLedgerError::InvalidRequest),
            }
        }
    };
    match result {
        Ok(reply) => reply,
        Err(AdminLedgerError::Stale) => match captured
            .actor
            .admin_chat_ledger_revision(captured.scope.clone())
            .await
        {
            Ok(revision) => dto::Reply::Stale { revision },
            Err(AdminLedgerError::Stale) => dto::Reply::Unavailable {
                code: "evidence-unavailable",
            },
            Err(error) => unavailable(error),
        },
        Err(error) => unavailable(error),
    }
}

/// The only plaintext reply publication path. Original-generation commit -> registry is the
/// existing lock order. No await occurs after the final row/origin check and before resolver send.
async fn publish(
    state: &AppState,
    captured: &Captured,
    encoded: String,
    send: impl FnOnce(String),
) {
    let Ok(_commit) = require_ui_session_generation(state, captured.generation).await else {
        send(dto::encode(dto::Reply::Unavailable {
            code: "session-unavailable",
        }));
        return;
    };
    let servers = state.servers.lock().await;
    let current = servers.get(&captured.server).is_some_and(|entry| {
        entry.instance == captured.instance && entry.actor.same_instance(&captured.actor)
    });
    if !current {
        send(dto::encode(dto::Reply::Unavailable {
            code: "server-superseded",
        }));
    } else if captured.origin.is_cancelled() {
        send(dto::encode(dto::Reply::Unavailable {
            code: "session-unavailable",
        }));
    } else {
        send(encoded);
    }
}

/// Install outside generate_handler so capture happens synchronously in Tauri's real dispatcher.
/// Tauri2.12.5 Webview::on_message checks the exact app command ACL before run_invoke_handler;
/// this router additionally refuses missing resolved ACL and any non-main webview/window.
pub(super) fn with_commands<R: tauri::Runtime>(
    fallback: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    router_with_spawn(fallback, |future| {
        drop(tauri::async_runtime::spawn(future));
    })
}

type NativeTask = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;

// The test seam controls scheduling ONLY, never authority, actor results or the production
// capture/publication path. It can hold the real router task completely unpolled across lock.
fn router_with_spawn<R: tauri::Runtime>(
    fallback: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
    spawn: impl Fn(NativeTask) + Send + Sync + 'static,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        let command = invoke.message.command();
        if !COMMANDS.contains(&command) {
            return fallback(invoke);
        }
        if invoke.acl.is_none()
            || invoke.message.webview_ref().label() != "main"
            || invoke.message.webview_ref().window().label() != "main"
        {
            invoke
                .resolver
                .resolve(Response::new(InvokeResponseBody::Json(dto::encode(
                    dto::Reply::Unavailable {
                        code: "not-authorized",
                    },
                ))));
            return true;
        }
        let app = invoke.message.webview_ref().app_handle().clone();
        let state = app.state::<AppState>();
        // Capture before even bounded argument inspection: a concurrent lock/unlock during
        // parsing cannot authorize the original invocation with the subsequently current epoch.
        let generation = state.ui_session_generation.load(Ordering::Acquire);
        let request = arguments(command, invoke.message.payload());
        let captured = request
            .as_ref()
            .map_err(|code| *code)
            .and_then(|request| capture(&state, request.server(), generation));
        let (request, captured) = match (request, captured) {
            (Ok(request), Ok(captured)) => (request, captured),
            (_, Err(code)) => {
                invoke
                    .resolver
                    .resolve(Response::new(InvokeResponseBody::Json(dto::encode(
                        dto::Reply::Unavailable { code },
                    ))));
                return true;
            }
            (Err(code), _) => {
                invoke
                    .resolver
                    .resolve(Response::new(InvokeResponseBody::Json(dto::encode(
                        dto::Reply::Unavailable { code },
                    ))));
                return true;
            }
        };
        drop(state);
        let resolver = invoke.resolver;
        drop(invoke.message); // do not retain the caller's arbitrary JSON allocation across awaits
        spawn(Box::pin(async move {
            let state = app.state::<AppState>();
            let reply = execute(&state, &captured, request).await;
            let encoded = dto::encode(reply);
            publish(&state, &captured, encoded, |encoded| {
                resolver.resolve(Response::new(InvokeResponseBody::Json(encoded)))
            })
            .await;
        }));
        true
    }
}
