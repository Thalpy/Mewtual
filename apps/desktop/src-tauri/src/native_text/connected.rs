//! Native-only background JoinRequest dispatch for the exact mounted founder.
use super::*;
use catcoms_app::store::{NativeTextMemberPreparation, NativeUiInvocationMetadata};
use catcoms_rt::{RequestCancellation, TransportEvent};

type Preparation = NativeTextMemberPreparation<catcoms_net::MeshService, OsCryptoRng>;

struct Source {
    generation: u64,
    target: Arc<NativeTextVaultAuthenticationCheck>,
    selection: server_table::NativeSelection,
    instance: u64,
    scope: RequestCancellation,
    _payment: NativeUiInvocationMetadata,
}

/// Called after authenticated native unlock or after publishing a founder row.
/// Each actual mesh admits one receiver. Foreign, removed and superseded rows
/// cannot start a worker, including after a later unlock of the same vault.
pub(super) async fn start_current<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    generation: u64,
    target: &Arc<NativeTextVaultAuthenticationCheck>,
) {
    let mut payments: [_; server_table::NATIVE_ROWS] =
        std::array::from_fn(|_| state.native_text.ui_budget.prepare().ok());
    let mut sources: [Option<Source>; server_table::NATIVE_ROWS] = std::array::from_fn(|_| None);
    {
        let Ok(_commit) = require_ui_session_generation(state, generation).await else {
            return;
        };
        let store = state.store.lock().await;
        let table = state.servers.lock().await;
        if !store
            .as_ref()
            .and_then(MountedVault::as_text_v2)
            .is_some_and(|vault| target.is_for_original_vault(vault))
            || state.native_text.sources.is_closing_cached()
        {
            return;
        }
        let Some(account) = keep_session::current(state, generation) else {
            return;
        };
        for (((_, row), slot), payment) in table.native_iter().zip(&mut sources).zip(&mut payments)
        {
            let data = row.data();
            if payment.is_none() || !target.same_original(&data.target) {
                continue;
            }
            let Some(scope) = row.scope(account.clone()) else {
                continue;
            };
            *slot = Some(Source {
                generation,
                target: target.clone(),
                selection: data.selection.clone(),
                instance: data.instance,
                scope,
                _payment: payment.take().unwrap(),
            });
        }
    }
    for source in sources.into_iter().flatten() {
        let Some(prepared) = source.capture(state).await else {
            continue;
        };
        let scope = source.scope.clone();
        let opened = tauri::async_runtime::spawn_blocking(move || {
            prepared.open_connected_join_requests(scope)
        })
        .await;
        if let Ok(Ok(requests)) = opened {
            drop(tauri::async_runtime::spawn(serve(
                app.clone(),
                source,
                requests,
            )));
        }
    }
}

impl Source {
    async fn capture(&self, state: &AppState) -> Option<Preparation> {
        if self.scope.is_cancelled() {
            return None;
        }
        let mut budget = Some(state.native_text.sources.policy().prepare_member().ok()?);
        let mut ui = Some(
            founding::ui_capture(state, self.generation, &self.target)
                .await
                .ok()?,
        );
        let mut output = None;
        {
            let _commit = require_ui_session_generation(state, self.generation)
                .await
                .ok()?;
            let store = state.store.lock().await;
            let table = state.servers.lock().await;
            let vault = store.as_ref().and_then(MountedVault::as_text_v2)?;
            let row = table.native(&self.selection.id_data())?;
            if self.scope.is_cancelled()
                || !self.target.is_for_original_vault(vault)
                || !row.matches(&self.selection, self.instance, &self.target)
            {
                return None;
            }
            state
                .native_text
                .sources
                .capture_member(&store, &mut ui, &self.selection, &mut budget, &mut output)
                .ok()?;
        }
        output
    }
}

async fn serve<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    source: Source,
    mut requests: catcoms_net::NativeChatJoinRequests,
) {
    while let Some(TransportEvent::Request {
        proto,
        data,
        responder,
        ..
    }) = requests.next().await
    {
        let state = app.state::<AppState>();
        if !responder.is_current() {
            continue;
        }
        let Some(prepared) = source.capture(&state).await else {
            continue;
        };
        let scope = source.scope.clone();
        // Keep one active blocking handler per registration. The queue and
        // lower responder retain bytes and cancellation through actual send.
        let _ = tauri::async_runtime::spawn_blocking(move || {
            prepared.serve_connected_join_request(proto, &data, responder, scope)
        })
        .await;
    }
}
