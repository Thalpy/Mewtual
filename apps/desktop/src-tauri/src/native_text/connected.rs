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
    member_work: Option<Arc<server_table::JoinedMemberWork>>,
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
            member_work: data.member_work.clone(),
            _payment: payment.take()?,
        })
    }

    pub(super) async fn capture_joined(
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
            {
                let table = state.servers.try_lock().ok()?;
                if !self.still_installed(state, &store, &table) {
                    return None;
                }
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

    pub(super) fn member_work(&self) -> Option<&Arc<server_table::JoinedMemberWork>> {
        self.member_work.as_ref()
    }

    pub(super) fn channel(&self) -> u128 {
        self.channel
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
        let app = app.clone();
        let receive = match state.native_text.ui_budget.prepare() {
            Ok(payment) => Some((super::receiving::open(state, &source).await, payment)),
            Err(_) => None,
        };
        drop(tauri::async_runtime::spawn(async move {
            tokio::select! {
                _ = serve_dependencies(app.clone(), source.clone(), provider) => {}
                _ = drain_joined_outbox(&app, &source) => {}
                _ = async {
                    if let Some((initial, payment)) = receive {
                        super::receiving::run(&app, &source, initial, payment).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {}
            }
        }));
    }
}

const OUTBOX_COOLDOWN: std::time::Duration = std::time::Duration::from_millis(250);
const OUTBOX_RETRY: std::time::Duration = std::time::Duration::from_secs(5);

async fn drain_joined_outbox<R: tauri::Runtime>(app: &tauri::AppHandle<R>, source: &Arc<Source>) {
    let Some(local) = &source.member_work else {
        return;
    };
    let mut scope = source.scope.clone();
    let mut ready = false;
    loop {
        if !ready {
            tokio::select! {
                biased;
                _ = scope.cancelled() => return,
                _ = local.publication_wake.notified() => {},
                _ = tokio::time::sleep(OUTBOX_RETRY - OUTBOX_COOLDOWN) => {},
            }
            // Give the renderer a chance to enqueue the history read it
            // commonly follows a newly saved receipt with.
            tokio::select! {
                biased;
                _ = scope.cancelled() => return,
                _ = tokio::time::sleep(OUTBOX_COOLDOWN) => {},
            }
        }
        let state = app.state::<AppState>();
        ready = tokio::select! {
            biased;
            _ = scope.cancelled() => return,
            advanced = publish_joined_debt(&state, source, local) => advanced,
        };
        tokio::select! {
            biased;
            _ = scope.cancelled() => return,
            _ = tokio::time::sleep(OUTBOX_COOLDOWN) => {},
        }
    }
}

async fn publish_joined_debt(
    state: &AppState,
    source: &Arc<Source>,
    local: &server_table::JoinedMemberWork,
) -> bool {
    let Some(preparation_guard) = local.try_background() else {
        return false;
    };
    let Some((original, row)) = source.capture_joined(state).await else {
        return false;
    };
    let work = source.clone();
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        let _local = preparation_guard;
        original.prepare_member_publication(&row, work.scope.clone(), &mut OsCryptoRng)
    })
    .await;
    let Ok(Ok(Some(prepared))) = prepared else {
        return false;
    };
    let Ok(Some(receipt)) = prepared.submit().await else {
        return false;
    };
    // A confirmed send already owns an exact receipt. Wait for the user
    // operation to release local work so this receipt can be checkpointed.
    let Some(checkpoint_guard) = local.foreground(source.scope.clone()).await else {
        return false;
    };
    let Some((original, _)) = source.capture_joined(state).await else {
        return false;
    };
    let work = source.clone();
    matches!(
        tauri::async_runtime::spawn_blocking(move || {
            let _local = checkpoint_guard;
            original.checkpoint_member_publication(receipt, &work.scope, &mut OsCryptoRng)
        })
        .await,
        Ok(Ok(_))
    )
}

async fn serve_dependencies<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    source: Arc<Source>,
    mut provider: NativeTextJoinedDependencyProvider,
) {
    while let Some(request) = provider.next().await {
        let state = app.state::<AppState>();
        let Some(local) = source
            .member_work
            .as_ref()
            .and_then(|work| work.try_background())
        else {
            continue;
        };
        let Some((original, row)) = source.capture_joined(&state).await else {
            continue;
        };
        let source = source.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let _local = local;
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
