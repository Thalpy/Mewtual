//! Publish the covered V2 invitation of the original founder row.
use super::*;
use catcoms_app::store::NativeUiInvocationMetadata;
use tauri::ipc::InvokeResponseBody;

pub(super) const BUSY: &str = "Invitations are busy. Retry the same request.";
pub(super) const FAILED: &str = "Could not prepare this Chat V2 invitation.";
const INVITE_LIFETIME_MS: u64 = 3_600_000;
const CREDITS: usize = 8;

struct Payment {
    _credits: Vec<NativeUiInvocationMetadata>,
    _permit: OwnedSemaphorePermit,
}

pub(super) fn route<R: tauri::Runtime>(invoke: Invoke<R>, spawn: &impl Fn(Task)) -> bool {
    if invoke.acl.is_none()
        || invoke.message.webview_ref().label() != "main"
        || invoke.message.webview_ref().window().label() != "main"
    {
        invoke
            .resolver
            .reject("Invitations are unavailable from this window.");
        return true;
    }
    let app = invoke.message.webview_ref().app_handle().clone();
    let capture = (|| {
        let state = app.state::<AppState>();
        let generation = state.ui_session_generation.load(Ordering::SeqCst);
        if ui_session_closed(&state) || state.native_text.sources.is_closing_cached() {
            return Err(listing::STALE);
        }
        let target = state
            .native_text
            .ui_state
            .original_target()
            .ok_or(listing::STALE)?;
        let account = keep_session::current(&state, generation).ok_or(listing::STALE)?;
        let InvokeBody::Json(value) = invoke.message.payload() else {
            return Err(FAILED);
        };
        let fields = value
            .as_object()
            .filter(|fields| fields.len() == 2)
            .ok_or(FAILED)?;
        let server = fields
            .get("server")
            .and_then(serde_json::Value::as_u64)
            .ok_or(FAILED)?;
        let context = fields
            .get("expectedContext")
            .and_then(serde_json::Value::as_str)
            .ok_or(FAILED)
            .and_then(|encoded| membership::decode_exact_hex::<32>(encoded).map_err(|_| FAILED))?;
        if context == [0; 32] {
            return Err(FAILED);
        }
        let permit = state
            .native_text
            .inflight
            .clone()
            .try_acquire_owned()
            .map_err(|_| BUSY)?;
        let mut source_payment = Some(state.native_text.ui_budget.prepare().map_err(|_| BUSY)?);
        let first = state.native_text.ui_budget.prepare().map_err(|_| BUSY)?;
        let mut credits = Vec::with_capacity(CREDITS);
        credits.push(first);
        for _ in 1..CREDITS {
            credits.push(state.native_text.ui_budget.prepare().map_err(|_| BUSY)?);
        }
        let payment = Arc::new(Payment {
            _credits: credits,
            _permit: permit,
        });
        let source = {
            let table = state.servers.try_lock().map_err(|_| BUSY)?;
            let row = table.native(&server).ok_or(listing::STALE)?;
            if row.data().request != context {
                return Err(listing::STALE);
            }
            connected::Source::for_row(generation, &target, row, account, &mut source_payment)
                .ok_or(listing::STALE)?
        };
        // The native journal scopes this stable row token to its selected
        // parent. A retry at the same parent uses the same original entry.
        let token = blake3::derive_key("mewtual/native-chat-first-invite/v2", &context);
        Ok((source, token, payment))
    })();
    let (source, token, payment) = match capture {
        Ok(value) => value,
        Err(error) => {
            invoke.resolver.reject(error);
            return true;
        }
    };
    let resolver = invoke.resolver;
    drop(invoke.message);
    spawn(Box::pin(async move {
        let state = app.state::<AppState>();
        let Some(prepared) = source.capture(&state).await else {
            resolver.reject(listing::STALE);
            return;
        };
        let negative = source.negative();
        let worker_payment = payment.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let _payment = worker_payment;
            let publication = prepared
                .finish()
                .map_err(|_| BUSY)?
                .prepare_invite_publication(token, INVITE_LIFETIME_MS, negative)
                .map_err(|_| FAILED)?;
            let body = serde_json::to_string(&hex::encode(publication.packet_bytes_data()))
                .map_err(|_| FAILED)?;
            Ok::<_, &'static str>((Box::new(publication), body, _payment))
        })
        .await
        .unwrap_or(Err(FAILED));
        let (publication, body, _output_payment) = match result {
            Ok(value) => value,
            Err(error) => {
                resolver.reject(error);
                return;
            }
        };
        // Lock and retirement can be waiting for this loan to return.
        let admit = (|| {
            let commit = state.ui_session_commit.try_lock().map_err(|_| BUSY)?;
            let store = state.store.try_lock().map_err(|_| BUSY)?;
            let table = state.servers.try_lock().map_err(|_| BUSY)?;
            if !source.still_installed(&state, &store, &table) || !publication.is_current_cached() {
                return Err(listing::STALE);
            }
            if publication.expires_at_ms_data() <= SystemClock.now_ms() {
                return Err("This Chat V2 invitation has expired.");
            }
            Ok((commit, store, table))
        })();
        match admit {
            Ok((_commit, _store, _table)) => resolver.resolve(InvokeResponseBody::Json(body)),
            Err(error) => resolver.reject(error),
        }
    }));
    true
}
