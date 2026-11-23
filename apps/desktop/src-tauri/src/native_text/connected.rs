//! Native connected request dispatch for the exact mounted row.
use super::*;
use catcoms_app::store::{
    NativeTextFounderMemberPublication, NativeTextFounderMemberPublicationReceipt,
    NativeTextJoinedDependencyProvider, NativeTextJoinedMemberPublication,
    NativeTextJoinedMemberPublicationReceipt, NativeTextJoinedRowSelection,
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
    member_work: Arc<server_table::NativeMemberWork>,
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
            let governing = if let Some(prepared) = source.capture(state).await {
                let scope = source.negative();
                tauri::async_runtime::spawn_blocking(move || {
                    prepared.open_connected_governing_requests(scope)
                })
                .await
                .ok()
                .and_then(Result::ok)
            } else {
                None
            };
            let source = Arc::new(source);
            let app = app.clone();
            drop(tauri::async_runtime::spawn(async move {
                let mut scope = source.negative();
                tokio::select! {
                    biased;
                    _ = scope.cancelled() => {}
                    _ = async {
                        tokio::join!(
                            serve(&app, &source, requests),
                            drain_outbox(&app, &source),
                            super::receiving::run_founder(&app, &source),
                            super::receiving::serve_founder_dependencies(&app, &source),
                            async {
                                if let Some(requests) = governing {
                                    serve_governing(&app, &source, requests).await;
                                }
                            },
                        );
                    } => {}
                }
            }));
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

    pub(super) fn for_installed(
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

    pub(super) fn member_work(&self) -> &Arc<server_table::NativeMemberWork> {
        &self.member_work
    }

    pub(super) fn channel(&self) -> u128 {
        self.channel
    }

    /// Notify the main renderer of one changed Create. Text is available only
    /// through a fresh guarded history invocation from the installed row.
    pub(super) async fn notify_received<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        state: &AppState,
        action: Option<catcoms_replication::chat::ActionId>,
    ) {
        let Ok(_commit) = require_ui_session_generation(state, self.generation).await else {
            return;
        };
        let store = state.store.lock().await;
        let table = state.servers.lock().await;
        if !self.still_installed(state, &store, &table) {
            return;
        }
        let server = self.selection.id_data();
        if let Some(row) = table.native(&server) {
            let _ = app.emit_to(
                "main",
                "chat-v2-received",
                serde_json::json!({
                    "server": server,
                    "channel": self.channel.to_string(),
                    "context": hex::encode(row.data().request),
                    "action": action.map(|action| hex::encode(action.as_bytes())),
                }),
            );
        }
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
            {
                let table = state.servers.try_lock().ok()?;
                if !self.still_installed(state, &store, &table) {
                    return None;
                }
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
                _ = drain_outbox(&app, &source) => {}
                _ = catch_up_governing(&app, &source) => {}
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
const GOVERNING_RETRY: std::time::Duration = std::time::Duration::from_secs(5);
const GOVERNING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

async fn catch_up_governing<R: tauri::Runtime>(app: &tauri::AppHandle<R>, source: &Source) {
    let mut scope = source.negative();
    let mut waiting = None;
    loop {
        tokio::select! {
            biased;
            _ = scope.cancelled() => return,
            _ = tokio::time::sleep(GOVERNING_RETRY) => {}
        }
        let state = app.state::<AppState>();
        if waiting.is_none() {
            let Some(local) = source.member_work.try_background() else {
                continue;
            };
            let Some((prepared, row)) = source.capture_joined(&state).await else {
                continue;
            };
            let caller = source.negative();
            let Ok(Ok(exchange)) = tauri::async_runtime::spawn_blocking(move || {
                let _local = local;
                prepared.prepare_governing_exchange(&row, caller)
            })
            .await
            else {
                continue;
            };
            let Ok(Ok(response)) =
                tokio::time::timeout(GOVERNING_TIMEOUT, exchange.receive()).await
            else {
                continue;
            };
            waiting = Some(response);
        }
        let Some(local) = source.member_work.try_background() else {
            continue;
        };
        let Some((prepared, row)) = source.capture_joined(&state).await else {
            continue;
        };
        let Some(response) = waiting.take() else {
            continue;
        };
        let caller = source.negative();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let _local = local;
            let result =
                prepared.apply_governing_response(&row, &response, &caller, &mut OsCryptoRng);
            (result, response)
        })
        .await;
        match result {
            Ok((Ok(_), _)) => source.member_work.publication_wake.notify_one(),
            Ok((Err(catcoms_app::store::V2MemberSaveRefusal::Busy), response)) => {
                waiting = Some(response);
            }
            _ => {}
        }
    }
}

async fn drain_outbox<R: tauri::Runtime>(app: &tauri::AppHandle<R>, source: &Arc<Source>) {
    let local = &source.member_work;
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
            advanced = publish_debt(&state, source, local) => advanced,
        };
        tokio::select! {
            biased;
            _ = scope.cancelled() => return,
            _ = tokio::time::sleep(OUTBOX_COOLDOWN) => {},
        }
    }
}

enum PublicationSource {
    Founder(Preparation),
    Joined(
        NativeTextJoinedSelectionPreparation<OsCryptoRng>,
        NativeTextJoinedRowSelection,
    ),
}

enum Publication {
    Founder(NativeTextFounderMemberPublication<catcoms_net::MeshService, OsCryptoRng>),
    Joined(NativeTextJoinedMemberPublication),
}

enum PublicationReceipt {
    Founder(NativeTextFounderMemberPublicationReceipt<catcoms_net::MeshService, OsCryptoRng>),
    Joined(NativeTextJoinedMemberPublicationReceipt),
}

async fn capture_publication(state: &AppState, source: &Source) -> Option<PublicationSource> {
    match &source.selection {
        server_table::NativeSelection::Founder(_) => {
            source.capture(state).await.map(PublicationSource::Founder)
        }
        server_table::NativeSelection::Joined(_) => source
            .capture_joined(state)
            .await
            .map(|(prepared, row)| PublicationSource::Joined(prepared, row)),
    }
}

async fn publish_debt(
    state: &AppState,
    source: &Arc<Source>,
    local: &server_table::NativeMemberWork,
) -> bool {
    let Some(preparation_guard) = local.try_background() else {
        return false;
    };
    let Some(original) = capture_publication(state, source).await else {
        return false;
    };
    let work = source.clone();
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        let _local = preparation_guard;
        match original {
            PublicationSource::Founder(original) => original
                .prepare_founder_member_publication(work.scope.clone())
                .map(|prepared| prepared.map(Publication::Founder)),
            PublicationSource::Joined(original, row) => original
                .prepare_member_publication(&row, work.scope.clone(), &mut OsCryptoRng)
                .map(|prepared| prepared.map(Publication::Joined)),
        }
    })
    .await;
    let Ok(Ok(Some(prepared))) = prepared else {
        return false;
    };
    let submitted = match prepared {
        Publication::Founder(prepared) => prepared
            .submit()
            .await
            .map(|receipt| receipt.map(PublicationReceipt::Founder)),
        Publication::Joined(prepared) => prepared
            .submit()
            .await
            .map(|receipt| receipt.map(PublicationReceipt::Joined)),
    };
    let Ok(Some(receipt)) = submitted else {
        return false;
    };
    // A confirmed send already owns an exact receipt. Wait for the user
    // operation to release local work so this receipt can be checkpointed.
    let Some(checkpoint_guard) = local.foreground(source.scope.clone()).await else {
        return false;
    };
    let Some(original) = capture_publication(state, source).await else {
        return false;
    };
    let work = source.clone();
    matches!(
        tauri::async_runtime::spawn_blocking(move || {
            let _local = checkpoint_guard;
            match (original, receipt) {
                (PublicationSource::Founder(original), PublicationReceipt::Founder(receipt)) => {
                    original.checkpoint_founder_member_publication(receipt, &work.scope)
                }
                (PublicationSource::Joined(original, _), PublicationReceipt::Joined(receipt)) => {
                    original.checkpoint_member_publication(receipt, &work.scope, &mut OsCryptoRng)
                }
                _ => Err(catcoms_app::store::V2MemberSaveRefusal::Stale),
            }
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
        let Some(local) = source.member_work.try_background() else {
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
    app: &tauri::AppHandle<R>,
    source: &Source,
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
        let Some(local) = source.member_work.foreground(source.scope.clone()).await else {
            return;
        };
        let Some(prepared) = source.capture(&state).await else {
            continue;
        };
        let scope = source.scope.clone();
        // Keep one active blocking handler per registration. The queue and
        // lower responder retain bytes and cancellation through actual send.
        let served = tauri::async_runtime::spawn_blocking(move || {
            let _local = local;
            prepared.serve_connected_join_request(proto, &data, responder, scope)
        })
        .await;
        if matches!(served, Ok(Ok(()))) {
            source.member_work.publication_wake.notify_one();
        }
    }
}

async fn serve_governing<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    source: &Source,
    mut requests: catcoms_net::NativeChatJoinRequests,
) {
    while let Some(TransportEvent::Request {
        proto,
        data,
        responder,
        ..
    }) = requests.next().await
    {
        if !responder.is_current() {
            continue;
        }
        let Some(local) = source.member_work.try_background() else {
            continue;
        };
        let Some(prepared) = source.capture(&app.state::<AppState>()).await else {
            continue;
        };
        let scope = source.negative();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let _local = local;
            prepared.serve_connected_governing_request(proto, &data, responder, scope)
        })
        .await;
    }
}
