//! Native founder command. A renderer request names an idempotent intent, not
//! a Source or an SDK permission. Every stage captures the SAME mounted owner,
//! then leaves all native guards before Root storage/key/runtime work.
use super::*;
use catcoms_app::store::{
    NativeTextCreationBudget, NativeTextCreationInputs, NativeTextCreationJob,
    NativeTextCreationPreparation, NativeTextCreationRefusal, NativeTextCreationRequest,
    NativeTextCreationSettings, NativeTextUiObservation, NativeUiInvocationMetadata,
};
use tauri::ipc::InvokeResponseBody;

type Preparation = NativeTextCreationPreparation<catcoms_net::MeshService, OsCryptoRng>;
type CreationBudget = NativeTextCreationBudget<catcoms_net::MeshService, OsCryptoRng>;
type CreationJob = NativeTextCreationJob<catcoms_net::MeshService, OsCryptoRng>;
const BUSY: &str = "Chat setup is busy. Retry with the same request.";
const SUPERSEDED: &str = "Chat setup was interrupted. Retry with the same request.";

struct Intent {
    request: [u8; 32],
    name: String,
    display_name: String,
    advertise: String,
    relay: String,
    rendezvous: String,
    is_dm: bool,
}
impl Intent {
    fn root_request(&self) -> Result<NativeTextCreationRequest, String> {
        NativeTextCreationRequest::new(
            self.request,
            NativeTextCreationSettings {
                display_name: self.display_name.clone(),
                server_name: self.name.clone(),
                advertise: self.advertise.clone(),
                relay: self.relay.clone(),
                rendezvous: self.rendezvous.clone(),
                is_dm: self.is_dm,
            },
        )
        .map_err(|_| "Invalid server details.".into())
    }
}
struct Invocation {
    generation: u64,
    target: Arc<NativeTextVaultAuthenticationCheck>,
    intent: Intent,
    _permit: OwnedSemaphorePermit,
    // LAST: <= 12 KiB frozen fields + one Root request copy + bounded row/
    // response/envelope overlap. Root runtime/selection have separate payment.
    _metadata: Vec<NativeUiInvocationMetadata>,
}
/// Historical output plus the actual installed association. Never reconstruct
/// this selection from the serialized server ID at response publication.
struct Output {
    body: Option<String>,
    selection: server_table::NativeSelection,
    instance: u64,
    start_connected: bool,
}

fn parse(payload: &InvokeBody) -> Result<Intent, &'static str> {
    let InvokeBody::Json(value) = payload else {
        return Err("Invalid server details.");
    };
    let object = value.as_object().ok_or("Invalid server details.")?;
    if object.len() > 8
        || object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "requestId"
                    | "displayName"
                    | "serverName"
                    | "advertise"
                    | "relay"
                    | "rendezvous"
                    | "isDm"
                    | "trace"
            )
        })
    {
        return Err("Invalid server details.");
    }
    let field = |key: &str, max: usize| {
        object
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| value.len() <= max)
            .ok_or("Invalid server details.")
    };
    let token = field("requestId", 64)?;
    if token.len() != 64 || !token.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err("Keep the original server request ID when retrying.");
    }
    let mut request = [0; 32];
    hex::decode_to_slice(token, &mut request).map_err(|_| "Invalid server request ID.")?;
    if request == [0; 32] {
        return Err("Invalid server request ID.");
    }
    // Validate all lengths before copying any supplied string.
    let display = field("displayName", 256)?;
    let name = field("serverName", 256)?;
    let advertise = field("advertise", 4096)?;
    let relay = field("relay", 4096)?;
    let rendezvous = field("rendezvous", 4096)?;
    let is_dm = object
        .get("isDm")
        .and_then(serde_json::Value::as_bool)
        .ok_or("Invalid server details.")?;
    if display.is_empty()
        || object
            .get("trace")
            .is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|s| s.len() > 128))
    {
        return Err("Invalid server details.");
    }
    Ok(Intent {
        request,
        name: if name.trim().is_empty() {
            display
        } else {
            name
        }
        .to_owned(),
        display_name: display.to_owned(),
        advertise: advertise.to_owned(),
        relay: relay.to_owned(),
        rendezvous: rendezvous.to_owned(),
        is_dm,
    })
}

/// Invoked only by the trusted native-mode router. Generation, ACL, original
/// target and bounded admission are captured before parsing or scheduling.
pub(super) fn route<R: tauri::Runtime>(invoke: Invoke<R>, spawn: &impl Fn(Task)) -> bool {
    if invoke.acl.is_none()
        || invoke.message.webview_ref().label() != "main"
        || invoke.message.webview_ref().window().label() != "main"
    {
        invoke
            .resolver
            .reject("Server creation is not available from this window.");
        return true;
    }
    let app = invoke.message.webview_ref().app_handle().clone();
    let captured = (|| {
        let state = app.state::<AppState>();
        let generation = state.ui_session_generation.load(Ordering::SeqCst);
        if ui_session_closed(&state) || state.native_text.sources.is_closing_cached() {
            return Err("Unlock the vault first.");
        }
        let target = state
            .native_text
            .ui_state
            .original_target()
            .ok_or(SUPERSEDED)?;
        let permit = state
            .native_text
            .inflight
            .clone()
            .try_acquire_owned()
            .map_err(|_| BUSY)?;
        let first = state.native_text.ui_budget.prepare().map_err(|_| BUSY)?;
        let mut metadata = Vec::with_capacity(8);
        metadata.push(first); // payment precedes even this fixed vector allocation
        for _ in 1..8 {
            metadata.push(state.native_text.ui_budget.prepare().map_err(|_| BUSY)?);
        }
        let intent = parse(invoke.message.payload())?;
        Ok(Invocation {
            generation,
            target,
            intent,
            _permit: permit,
            _metadata: metadata,
        })
    })();
    let captured = match captured {
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
        // Paid invocation and all owning results precede final commit in Drop
        // order, including an actual resolver panic or a lost IPC observer.
        let mut result = execute(&state, &captured).await;
        if result.as_ref().is_ok_and(|output| output.start_connected) {
            super::connected::start_current(&app, &state, captured.generation, &captured.target)
                .await;
        }
        #[cfg(test)]
        {
            let edge = state.native_text.text_reply_edge.lock().unwrap().clone();
            if let Some(edge) = edge {
                edge.wait_once().await;
            }
        }
        let _commit = state.ui_session_commit.lock().await;
        let store = state.store.lock().await;
        let table = state.servers.lock().await;
        let original_mount = store
            .as_ref()
            .and_then(MountedVault::as_text_v2)
            .is_some_and(|vault| captured.target.is_for_original_vault(vault));
        let original_row = match &result {
            Ok(output) => table
                .native(&output.selection.id_data())
                .is_some_and(|row| {
                    row.matches(&output.selection, output.instance, &captured.target)
                        && row.data().request == captured.intent.request
                }),
            Err(_) => true, // an error discloses no successful row association
        };
        if state.ui_session_generation.load(Ordering::SeqCst) != captured.generation
            || ui_session_closed(&state)
            || state.native_text.sources.is_closing_cached()
            || !original_mount
            || !original_row
        {
            resolver.reject(SUPERSEDED);
        } else {
            match &mut result {
                // Keep the owning original selection in the OUTER result
                // through callback panic; only pre-serialized DATA moves here.
                Ok(output) => resolver.resolve(InvokeResponseBody::Json(
                    output.body.take().expect("one founder reply"),
                )),
                Err(error) => resolver.reject(&*error),
            }
        }
    }));
    true
}

/// Prepare actual original UI custody in two phases. No counted Source work
/// exists while awaiting either native commit acquisition in this function.
/// The callback moves captures to its caller's OUTER slots. Only static errors
/// leave the guarded section; owning errors are allocated after guard release.
pub(super) async fn with_current_vault(
    state: &AppState,
    generation: u64,
    target: &NativeTextVaultAuthenticationCheck,
    apply: impl FnOnce(
        &Option<MountedVault>,
        &catcoms_app::store::NativeTextVault,
    ) -> Result<(), &'static str>,
) -> Result<(), String> {
    let result = {
        let _commit = require_ui_session_generation(state, generation).await?;
        let store = state.store.lock().await;
        (|| {
            let vault = store
                .as_ref()
                .and_then(MountedVault::as_text_v2)
                .ok_or(SUPERSEDED)?;
            if !target.is_for_original_vault(vault) || state.native_text.sources.is_closing_cached()
            {
                return Err(SUPERSEDED);
            }
            apply(&store, vault)
        })()
    };
    result.map_err(str::to_owned)
}

pub(super) async fn ui_capture(
    state: &AppState,
    generation: u64,
    target: &NativeTextVaultAuthenticationCheck,
) -> Result<NativeTextUiObservation, String> {
    let mut credit = Some(state.native_text.ui_budget.prepare().map_err(|_| BUSY)?);
    let mut preparation = None;
    with_current_vault(state, generation, target, |_, vault| {
        preparation = Some(vault.capture_ui(generation, credit.take().unwrap()));
        Ok(())
    })
    .await?;
    let mut candidate = preparation
        .take()
        .unwrap()
        .finish()
        .map_err(|_| SUPERSEDED)?;
    let mut observation = None;
    with_current_vault(state, generation, target, |_, vault| {
        observation = Some(vault.admit_ui(&mut candidate).map_err(|_| SUPERSEDED)?);
        Ok(())
    })
    .await?;
    Ok(observation.take().unwrap())
}

async fn capture(state: &AppState, invocation: &Invocation) -> Result<Preparation, String> {
    // Pay/copy BEFORE borrowing Store/controller. Every losing input/capture
    // remains in these outer slots until both native guards have left.
    let policy = state.native_text.sources.policy();
    let mut budget: Option<CreationBudget> = Some(policy.prepare_creation().map_err(|_| BUSY)?);
    let mut request = Some(invocation.intent.root_request()?);
    let mut inputs = Some(NativeTextCreationInputs::new(
        OsCryptoRng,
        OsCryptoRng,
        Box::new(SystemClock),
    ));
    let mut ui = Some(ui_capture(state, invocation.generation, &invocation.target).await?);
    let mut output = None;
    with_current_vault(
        state,
        invocation.generation,
        &invocation.target,
        |store, _| {
            state.native_text.sources.capture_creation(
                store,
                &mut ui,
                &mut request,
                &mut inputs,
                &mut budget,
                &mut output,
            )
        },
    )
    .await?;
    Ok(output.take().unwrap())
}

async fn run_setup_step<T: Send + 'static>(
    state: &AppState,
    invocation: &Invocation,
    _name: &'static str,
    run: impl Fn(CreationJob) -> Result<T, NativeTextCreationRefusal> + Copy + Send + 'static,
) -> Result<T, String> {
    let deadline = SystemClock.monotonic_ms().saturating_add(2_000);
    loop {
        let preparation = capture(state, invocation).await?;
        let result = tauri::async_runtime::spawn_blocking(move || run(preparation.finish()?))
            .await
            .map_err(|_| SUPERSEDED)?;
        match result {
            Ok(value) => return Ok(value),
            Err(NativeTextCreationRefusal::Busy) => {
                if SystemClock.monotonic_ms() >= deadline {
                    return Err(BUSY.to_owned());
                }
                #[cfg(test)]
                if let Some(probe) = state.native_text.found_setup_busy.lock().unwrap().as_ref() {
                    probe.notify_one();
                }
                // The completed job returned its original runtime and all guards.
                SystemClock.sleep(Duration::from_millis(25)).await;
            }
            Err(error) => {
                #[cfg(test)]
                eprintln!("Native founder {_name} failed: {error:?}");
                return Err(format!(
                    "Server setup did not finish ({error:?}). Retry with the same request."
                ));
            }
        }
    }
}

async fn execute(state: &AppState, invocation: &Invocation) -> Result<Output, String> {
    if let Some(receipt) = recovered_receipt(state, invocation).await? {
        return Ok(receipt);
    }
    // Each consuming call restores the same whole runtime before acquisition.
    // Only explicit contention retries the current step, with its original data.
    let steps: [(_, fn(CreationJob) -> _); 6] = [
        ("device preparation", CreationJob::prepare_device),
        ("network preparation", CreationJob::prepare_network),
        ("pending network save", CreationJob::cover_pending_network),
        ("transport preparation", CreationJob::prepare_transport),
        ("founder preparation", CreationJob::prepare_founder),
        ("activation save", CreationJob::cover_founder_activation),
    ];
    for (name, step) in steps {
        run_setup_step(state, invocation, name, step).await?;
    }
    let founder = run_setup_step(
        state,
        invocation,
        "stored row selection",
        CreationJob::select_stored_runtime,
    )
    .await?;
    let context = founder.context_data();
    if context != invocation.intent.request {
        return Err(SUPERSEDED.into());
    }
    let selection = server_table::NativeSelection::Founder(founder);
    let row_policy = { state.servers.lock().await.native_budget() };
    let credit = row_policy.prepare().map_err(|_| BUSY)?;
    let instance = state.next_server_instance.fetch_add(1, Ordering::Relaxed);
    let mut installed_instance = instance;
    let mut row = Some(server_table::PreparedNativeRow::new(
        server_table::NativeRowSpec {
            selection: selection.clone(),
            instance,
            target: invocation.target.clone(),
            request: context,
            name: &invocation.intent.name,
            display_name: &invocation.intent.display_name,
            is_dm: invocation.intent.is_dm,
            channel: channel_id("general"),
        },
        credit,
    )?);
    let id = selection.id_data();
    // Freeze and serialize bounded DATA outside final publication guards.
    let response = receipt_body(id, context, channel_id("general"), invocation.intent.is_dm);
    #[cfg(test)]
    {
        let edge = state
            .native_text
            .found_publication_edge
            .lock()
            .unwrap()
            .clone();
        if let Some(edge) = edge {
            edge.wait_once().await;
        }
    }
    let installed = {
        let _commit = require_ui_session_generation(state, invocation.generation).await?;
        let store = state.store.lock().await;
        let mut table = state.servers.lock().await;
        (|| {
            let vault = store
                .as_ref()
                .and_then(MountedVault::as_text_v2)
                .ok_or(SUPERSEDED)?;
            // Recheck closing AFTER acquiring the publication guard. A pre-await
            // check cannot exclude a late insertion behind the close sweep.
            if !invocation.target.is_for_original_vault(vault)
                || state.native_text.sources.is_closing_cached()
            {
                return Err(SUPERSEDED);
            }
            if let Some(existing) = table.native(&id) {
                let previous = existing.data();
                if previous.request != context
                    || !previous.selection.same_original(&selection)
                    || previous.name != invocation.intent.name
                    || previous.display_name != invocation.intent.display_name
                    || previous.is_dm != invocation.intent.is_dm
                    || previous.channel != channel_id("general")
                    || !existing.matches(&selection, previous.instance, &invocation.target)
                {
                    return Err("The server request no longer matches its original row.");
                }
                // Passive retry candidate is dropped only after every guard has
                // left; unlike an installed row, it cannot close this Source.
                installed_instance = previous.instance;
            } else {
                table.install_native(&mut row)?;
            }
            Ok(())
        })()
    };
    installed.map_err(str::to_owned)?;
    Ok(Output {
        body: Some(response),
        selection,
        instance: installed_instance,
        start_connected: true,
    })
}

async fn recovered_receipt(
    state: &AppState,
    invocation: &Invocation,
) -> Result<Option<Output>, String> {
    let request = invocation.intent.root_request()?;
    let mut payment = state.native_text.ui_budget.prepare().ok();
    let evidence =
        {
            let _commit = require_ui_session_generation(state, invocation.generation).await?;
            let store = state.store.lock().await;
            let vault = store
                .as_ref()
                .and_then(MountedVault::as_text_v2)
                .filter(|vault| invocation.target.is_for_original_vault(vault))
                .ok_or(SUPERSEDED)?;
            if state.native_text.sources.is_closing_cached() {
                return Err(SUPERSEDED.into());
            }
            let table = state.servers.lock().await;
            let Some(row) = table.native_iter().map(|(_, row)| row).find(|row| {
                row.data().request == invocation.intent.request
                    && row.data().selection.founder().is_some_and(|original| {
                        original.is_recovered_from_original_vault_cached(vault)
                    })
            }) else {
                return Ok(None);
            };
            let data = row.data();
            let original = data.selection.founder().ok_or(SUPERSEDED)?;
            if !row.matches(&data.selection, data.instance, &invocation.target)
                || !original.matches_creation_request_data(&request)
                || invocation.intent.name != data.name
                || invocation.intent.display_name != data.display_name
                || invocation.intent.is_dm != data.is_dm
            {
                return Err("The server request no longer matches its original row.".into());
            }
            let account = keep_session::current(state, invocation.generation).ok_or(SUPERSEDED)?;
            let source = super::connected::Source::for_row(
                invocation.generation,
                &invocation.target,
                row,
                account,
                &mut payment,
            )
            .ok_or(BUSY)?;
            (
                source,
                data.selection.clone(),
                data.instance,
                data.channel,
                data.is_dm,
            )
        };
    let (source, selection, instance, channel, is_dm) = evidence;
    let caller = source.negative();
    let local = source
        .member_work()
        .foreground(caller.clone())
        .await
        .ok_or(SUPERSEDED)?;
    let prepared = source.capture(state).await.ok_or(SUPERSEDED)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _local = local;
        prepared.verify_recovered_founder(&caller)
    })
    .await
    .map_err(|_| SUPERSEDED)?
    .map_err(|_| SUPERSEDED)?;
    Ok(Some(Output {
        body: Some(receipt_body(
            selection.id_data(),
            invocation.intent.request,
            channel,
            is_dm,
        )),
        selection,
        instance,
        start_connected: false,
    }))
}

fn receipt_body(server: u64, context: [u8; 32], channel: u128, is_dm: bool) -> String {
    let channel = channel.to_string();
    serde_json::json!({"backend":"text-v2", "server":server, "context":hex::encode(context),
        "channel":channel, "channels":[{"id":channel,"name":"general"}],
        "is_dm":is_dm, "storage_warning":null})
    .to_string()
}

const _: () =
    assert!(std::mem::size_of::<Invocation>() + 2 * (3 * 4096 + 2 * 256) + 4096 <= 8 * 4096);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_text_found_arguments_bind_nonzero_token_and_bound_every_retained_field() {
        let base = serde_json::json!({"requestId":"01".repeat(32),"displayName":"Founder",
            "serverName":"  ","advertise":"","relay":"","rendezvous":"","isDm":false});
        let accepted = parse(&InvokeBody::Json(base.clone())).unwrap();
        assert_eq!(accepted.name, "Founder");
        assert_eq!(accepted.request, [1; 32]);
        for (key, value) in [
            ("requestId", serde_json::json!("00".repeat(32))),
            ("requestId", serde_json::json!("zz".repeat(32))),
            ("requestId", serde_json::json!("01".repeat(33))),
            ("displayName", serde_json::json!("")),
            ("serverName", serde_json::json!("x".repeat(257))),
            ("advertise", serde_json::json!("x".repeat(4097))),
            ("relay", serde_json::json!("x".repeat(4097))),
            ("rendezvous", serde_json::json!("x".repeat(4097))),
            ("isDm", serde_json::json!("false")),
            ("trace", serde_json::json!("x".repeat(129))),
            ("protocol", serde_json::json!("native-v2")),
        ] {
            let mut malformed = base.clone();
            malformed[key] = value;
            assert!(parse(&InvokeBody::Json(malformed)).is_err(), "{key}");
        }
    }
}
