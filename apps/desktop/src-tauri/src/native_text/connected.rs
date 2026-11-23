//! Native connected request dispatch for the exact mounted row.
use super::*;
use catcoms_app::store::{
    NativeTextJoinedDependencyProvider, NativeTextJoinedRowSelection,
    NativeTextJoinedSelectionPreparation, NativeTextMemberPreparation, NativeUiInvocationMetadata,
};
use catcoms_rt::{RequestCancellation, TransportEvent};

type Preparation = NativeTextMemberPreparation<catcoms_net::MeshService, OsCryptoRng>;

pub(super) struct Source {
    generation: u64,
    target: Arc<NativeTextVaultAuthenticationCheck>,
    selection: server_table::NativeSelection,
    instance: u64,
    channel: u128,
    scope: RequestCancellation,
    _payment: NativeUiInvocationMetadata,
}

/// Called after authenticated unlock or after publishing a native row.
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
            *slot = Source::for_installed(generation, target, row, account.clone(), payment);
        }
    }
    for source in sources.into_iter().flatten() {
        if matches!(&source.selection, server_table::NativeSelection::Joined(_)) {
            start_dependencies(app, state, Arc::new(source)).await;
            continue;
        }
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
    pub(super) fn for_row(
        generation: u64,
        target: &Arc<NativeTextVaultAuthenticationCheck>,
        row: &server_table::NativeRow,
        account: RequestCancellation,
        payment: &mut Option<NativeUiInvocationMetadata>,
    ) -> Option<Self> {
        row.data().selection.founder()?;
        Self::for_installed(generation, target, row, account, payment)
    }

    fn for_installed(
        generation: u64,
        target: &Arc<NativeTextVaultAuthenticationCheck>,
        row: &server_table::NativeRow,
        account: RequestCancellation,
        payment: &mut Option<NativeUiInvocationMetadata>,
    ) -> Option<Self> {
        let data = row.data();
        if payment.is_none() || !target.same_original(&data.target) {
            return None;
        }
        let scope = row.scope(account)?;
        Some(Self {
            generation,
            target: target.clone(),
            selection: data.selection.clone(),
            instance: data.instance,
            channel: data.channel,
            scope,
            _payment: payment.take()?,
        })
    }

    async fn capture_joined(
        &self,
        state: &AppState,
    ) -> Option<(
        NativeTextJoinedSelectionPreparation<OsCryptoRng>,
        NativeTextJoinedRowSelection,
    )> {
        let server_table::NativeSelection::Joined(row) = &self.selection else {
            return None;
        };
        if self.scope.is_cancelled() {
            return None;
        }
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
            if !self.still_installed(state, &store, &table) {
                return None;
            }
            state
                .native_text
                .sources
                .capture_joined_member(&store, &mut ui, row, &mut output)
                .ok()?;
        }
        Some((output?, row.clone()))
    }

    pub(super) fn negative(&self) -> RequestCancellation {
        self.scope.clone()
    }

    pub(super) fn still_installed(
        &self,
        state: &AppState,
        store: &Option<MountedVault>,
        table: &server_table::ServerTable,
    ) -> bool {
        self.generation == state.ui_session_generation.load(Ordering::SeqCst)
            && !ui_session_closed(state)
            && !self.scope.is_cancelled()
            && !state.native_text.sources.is_closing_cached()
            && store
                .as_ref()
                .and_then(MountedVault::as_text_v2)
                .is_some_and(|vault| self.target.is_for_original_vault(vault))
            && table
                .native(&self.selection.id_data())
                .is_some_and(|row| row.matches(&self.selection, self.instance, &self.target))
    }

    pub(super) async fn capture(&self, state: &AppState) -> Option<Preparation> {
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
            if !self.still_installed(state, &store, &table) {
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

async fn start_dependencies<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    source: Arc<Source>,
) {
    let Some((original, row)) = source.capture_joined(state).await else {
        return;
    };
    let payment = source.clone();
    let opened = tauri::async_runtime::spawn_blocking(move || {
        original.attach_dependency_provider(&row, payment.scope.clone())
    })
    .await;
    if let Ok(Ok(provider)) = opened {
        drop(tauri::async_runtime::spawn(serve_dependencies(
            app.clone(),
            source,
            provider,
        )));
    }
}

async fn serve_dependencies<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    source: Arc<Source>,
    mut provider: NativeTextJoinedDependencyProvider,
) {
    while let Some(request) = provider.next().await {
        let state = app.state::<AppState>();
        let Some((original, row)) = source.capture_joined(&state).await else {
            continue;
        };
        let source = source.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            original.serve_dependency_request(
                &row,
                source.channel,
                request,
                source.scope.clone(),
                &mut OsCryptoRng,
            )
        })
        .await;
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
