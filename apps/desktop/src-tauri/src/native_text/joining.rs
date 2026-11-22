//! Native first-join bridge for an exact invite and an explicitly approved IP.
use super::*;
use catcoms_app::store::{
    NativeTextInvitePacketData, NativeTextJoinedRequestPreparation, NativeTextJoinedRowSelection,
    NativeTextJoinedSelectionPreparation, NativeTextSourceCommand, NativeUiInvocationMetadata,
    MAX_NATIVE_TEXT_INVITE_PACKET_BYTES,
};
use catcoms_rt::RequestCancellation;
use std::net::IpAddr;
use tauri::ipc::InvokeResponseBody;

const INPUT: &str = "Enter a valid Chat V2 invite and an approved host IP address.";
const BUSY: &str = "Chat is busy. Retry the same invitation.";
const FAILED: &str = "Joining did not finish. Retry the same Chat V2 invitation.";
const CREDITS: usize = 8;

type SelectionPreparation = NativeTextJoinedSelectionPreparation<OsCryptoRng>;

struct Invocation {
    generation: u64,
    target: Arc<NativeTextVaultAuthenticationCheck>,
    account: RequestCancellation,
    packet: Zeroizing<Vec<u8>>,
    host: IpAddr,
    name: String,
    display: String,
    _credits: Vec<NativeUiInvocationMetadata>,
    _permit: OwnedSemaphorePermit,
}

impl Invocation {
    fn invite(&self) -> Result<NativeTextInvitePacketData<'_>, &'static str> {
        NativeTextInvitePacketData::decode(&self.packet).map_err(|_| INPUT)
    }

    fn token(invite: &NativeTextInvitePacketData<'_>) -> [u8; 32] {
        blake3::derive_key(
            "mewtual/native-chat-first-join/v2",
            &invite.binding().id_data(),
        )
    }
}

pub(super) fn route<R: tauri::Runtime>(invoke: Invoke<R>, spawn: &impl Fn(Task)) -> bool {
    if invoke.acl.is_none()
        || invoke.message.webview_ref().label() != "main"
        || invoke.message.webview_ref().window().label() != "main"
    {
        invoke
            .resolver
            .reject("Joining is unavailable from this window.");
        return true;
    }
    let app = invoke.message.webview_ref().app_handle().clone();
    let captured = capture_input(&app.state::<AppState>(), invoke.message.payload());
    let invocation = match captured {
        Ok(original) => Arc::new(original),
        Err(error) => {
            invoke.resolver.reject(error);
            return true;
        }
    };
    let resolver = invoke.resolver;
    drop(invoke.message);
    spawn(Box::pin(async move {
        let state = app.state::<AppState>();
        let result = complete(&state, &invocation).await;
        match result {
            Ok(selection) => publish(&state, &invocation, selection, resolver).await,
            Err(error) => resolver.reject(error),
        }
    }));
    true
}

fn capture_input(state: &AppState, payload: &InvokeBody) -> Result<Invocation, &'static str> {
    let generation = state.ui_session_generation.load(Ordering::SeqCst);
    if ui_session_closed(state) || state.native_text.sources.is_closing_cached() {
        return Err(listing::STALE);
    }
    let target = state
        .native_text
        .ui_state
        .original_target()
        .ok_or(listing::STALE)?;
    let account = keep_session::current(state, generation).ok_or(listing::STALE)?;
    let InvokeBody::Json(value) = payload else {
        return Err(INPUT);
    };
    let fields = value
        .as_object()
        .filter(|fields| fields.len() == 6)
        .ok_or(INPUT)?;
    if fields.get("isDm").and_then(serde_json::Value::as_bool) != Some(false)
        || fields
            .get("allowSwitchboards")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
    {
        return Err(INPUT);
    }
    let text = |key| {
        fields
            .get(key)
            .and_then(serde_json::Value::as_str)
            .ok_or(INPUT)
    };
    let hex = text("inviteHex")?;
    if hex.is_empty() || hex.len() % 2 != 0 || hex.len() > 2 * MAX_NATIVE_TEXT_INVITE_PACKET_BYTES {
        return Err(INPUT);
    }
    let host = text("approvedHost")?;
    if host.len() > 45 {
        return Err(INPUT);
    }
    let host: IpAddr = host.parse().map_err(|_| INPUT)?;
    if host.is_unspecified() || host.is_multicast() {
        return Err(INPUT);
    }
    let name = text("serverName")?;
    let display = text("displayName")?;
    if name.trim().is_empty()
        || name.len() > 256
        || display.trim().is_empty()
        || display.len() > 256
    {
        return Err(INPUT);
    }
    let permit = state
        .native_text
        .inflight
        .clone()
        .try_acquire_owned()
        .map_err(|_| BUSY)?;
    let first = state.native_text.ui_budget.prepare().map_err(|_| BUSY)?;
    let mut credits = Vec::with_capacity(CREDITS);
    credits.push(first);
    for _ in 1..CREDITS {
        credits.push(state.native_text.ui_budget.prepare().map_err(|_| BUSY)?);
    }
    let mut packet = Zeroizing::new(vec![0; hex.len() / 2]);
    hex::decode_to_slice(hex.as_bytes(), &mut packet).map_err(|_| INPUT)?;
    Ok(Invocation {
        generation,
        target,
        account,
        packet,
        host,
        name: name.to_owned(),
        display: display.to_owned(),
        _credits: credits,
        _permit: permit,
    })
}

async fn capture_active<T>(
    state: &AppState,
    invocation: &Invocation,
    capture: impl FnOnce(
        &catcoms_app::store::NativeTextSources<catcoms_net::MeshService, OsCryptoRng>,
        NativeTextSourceCommand,
    ) -> T,
) -> Result<T, String> {
    let mut ui =
        Some(founding::ui_capture(state, invocation.generation, &invocation.target).await?);
    let mut output = None;
    founding::with_current_vault(
        state,
        invocation.generation,
        &invocation.target,
        |store, _| {
            state
                .native_text
                .sources
                .capture_join(store, |sources, vault| {
                    let command = vault
                        .bind_native_source_command(&mut ui)
                        .map_err(|_| listing::STALE)?;
                    output = Some(capture(sources, command));
                    Ok(())
                })
        },
    )
    .await?;
    output.ok_or_else(|| listing::STALE.to_owned())
}

async fn complete(
    state: &AppState,
    invocation: &Arc<Invocation>,
) -> Result<NativeTextJoinedRowSelection, String> {
    let prepared: SelectionPreparation = capture_active(state, invocation, |sources, command| {
        sources.capture_joined_selection(command)
    })
    .await?;
    let original = invocation.clone();
    let parked = tauri::async_runtime::spawn_blocking(move || {
        // Refuse malformed/downgraded codes before any visible native work.
        original.invite()?;
        prepared
            .has_parked_runtime(&original.account)
            .map_err(|_| FAILED)
    })
    .await
    .map_err(|_| FAILED)??;
    if !parked {
        let prepared: NativeTextJoinedRequestPreparation =
            capture_active(state, invocation, |sources, command| {
                sources.capture_join_request(command)
            })
            .await?;
        let original = invocation.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let invite = original.invite()?;
            prepared
                .cover(
                    &invite,
                    &invite.binding().spec_data().anchor,
                    Invocation::token(&invite),
                    SystemClock.now_ms(),
                    &original.account,
                    &mut OsCryptoRng,
                )
                .map_err(|_| FAILED)
        })
        .await
        .map_err(|_| FAILED)??;
        let prepared = capture_active(state, invocation, |sources, command| {
            sources.capture_join_request(command)
        })
        .await?;
        let original = invocation.clone();
        let exchange = tauri::async_runtime::spawn_blocking(move || {
            let invite = original.invite()?;
            prepared
                .prepare_exchange(&invite, original.account.clone())
                .map_err(|_| FAILED)
        })
        .await
        .map_err(|_| FAILED)??;
        let invite = invocation.invite()?;
        exchange
            .connect_approved_host(&invite, invocation.host, &state.endpoint_dials)
            .await
            .map_err(|_| FAILED)?;
        let mut delivery = Some(exchange.receive().await.map_err(|_| FAILED)?);
        let mut inputs = Some((
            OsCryptoRng,
            Box::new(SystemClock) as Box<dyn catcoms_rt::Clock + Send>,
        ));
        let mut work = None;
        founding::with_current_vault(
            state,
            invocation.generation,
            &invocation.target,
            |store, _| {
                state.native_text.sources.capture_join(store, |sources, _| {
                    work = Some(
                        sources
                            .capture_join_installation(&mut delivery, &mut inputs)
                            .map_err(|_| FAILED)?,
                    );
                    Ok(())
                })
            },
        )
        .await?;
        let mut work = work.ok_or(FAILED)?;
        tauri::async_runtime::spawn_blocking(move || work.install().map_err(|_| FAILED))
            .await
            .map_err(|_| FAILED)??;
    }
    let prepared = capture_active(state, invocation, |sources, command| {
        sources.capture_joined_selection(command)
    })
    .await?;
    let original = invocation.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let invite = original.invite()?;
        prepared
            .register(
                &invite,
                Invocation::token(&invite),
                original.name.clone(),
                original.display.clone(),
                &original.account,
                &mut OsCryptoRng,
            )
            .map_err(|_| FAILED)
    })
    .await
    .map_err(|_| FAILED)??;
    let prepared = capture_active(state, invocation, |sources, command| {
        sources.capture_joined_selection(command)
    })
    .await?;
    let original = invocation.clone();
    tauri::async_runtime::spawn_blocking(move || {
        prepared
            .select_registered(&original.account)
            .map_err(|_| FAILED.to_owned())
    })
    .await
    .map_err(|_| FAILED)?
}

async fn publish<R: tauri::Runtime>(
    state: &AppState,
    invocation: &Invocation,
    original: NativeTextJoinedRowSelection,
    resolver: tauri::ipc::InvokeResolver<R>,
) {
    let id = original.id_data();
    let request = original.request_data();
    let channel = channel_id("general");
    let label = original.name_data();
    let display = original.display_name_data().unwrap_or("me");
    if display != invocation.display {
        resolver.reject("The invitation already has a registered display name.");
        return;
    }
    let response = serde_json::json!({"backend":"text-v2", "server":id,
        "context":hex::encode(request), "channel":channel.to_string(),
        "name":label, "display_name":display, "is_dm":false})
    .to_string();
    let budget = state.servers.lock().await.native_budget();
    let built = budget
        .prepare()
        .map_err(|_| BUSY.to_owned())
        .and_then(|credit| {
            server_table::PreparedNativeRow::new(
                server_table::NativeRowSpec {
                    selection: server_table::NativeSelection::Joined(original.clone()),
                    instance: state.next_server_instance.fetch_add(1, Ordering::Relaxed),
                    target: invocation.target.clone(),
                    request,
                    name: label,
                    display_name: display,
                    is_dm: false,
                    channel,
                },
                credit,
            )
        });
    let mut candidate = match built {
        Ok(row) => Some(row),
        Err(error) => {
            resolver.reject(error);
            return;
        }
    };
    let selection = server_table::NativeSelection::Joined(original.clone());
    let Ok(_commit) = require_ui_session_generation(state, invocation.generation).await else {
        resolver.reject(listing::STALE);
        return;
    };
    let store = state.store.lock().await;
    let mut table = state.servers.lock().await;
    let current = store
        .as_ref()
        .and_then(MountedVault::as_text_v2)
        .is_some_and(|vault| {
            invocation.target.is_for_original_vault(vault)
                && original.is_from_original_vault_cached(vault)
        });
    if !current
        || !original.is_current_cached()
        || invocation.account.is_cancelled()
        || state.native_text.sources.is_closing_cached()
    {
        resolver.reject(listing::STALE);
        return;
    }
    let admitted = match table.native(&id) {
        Some(row)
            if row.matches(&selection, row.data().instance, &invocation.target)
                && row.data().request == request
                && row.data().name == label
                && row.data().display_name == invocation.display =>
        {
            Ok(())
        }
        Some(_) => Err(listing::STALE),
        None => table.install_native(&mut candidate),
    };
    match admitted {
        Ok(()) => resolver.resolve(InvokeResponseBody::Json(response)),
        Err(error) => resolver.reject(error),
    }
}
