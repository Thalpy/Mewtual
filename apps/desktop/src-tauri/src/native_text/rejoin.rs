//! Main-window rejoin on the original authenticated retired vault. File
//! phases cover exact local journals. Only the approved peer receives TCP.
use super::*;
use catcoms_app::store::{
    NativeTextInvitePacketData, NativeTextRejoinExchange, NativeTextRejoinWelcomeStatus,
    NativeTextRemovedJoinedVault, NativeUiInvocationMetadata, MAX_NATIVE_TEXT_INVITE_PACKET_BYTES,
};
use std::net::IpAddr;

pub(super) mod files;
pub(super) const COMMANDS: &[&str] = &["rejoin_server"];
const BUSY: &str = "Rejoin is busy. Try the same invitation again.";
const FAILED: &str = "Rejoin did not finish. Try the same invitation again.";
const INPUT: &str = "Enter a Chat V2 invite and the founder’s approved IP address.";

pub(super) fn handles(command: &str) -> bool {
    COMMANDS.contains(&command)
}

struct Invocation {
    generation: u64,
    target: Arc<NativeTextVaultAuthenticationCheck>,
    scope: RequestCancellation,
    invite: Zeroizing<Vec<u8>>,
    host: IpAddr,
    _permit: OwnedSemaphorePermit,
    _credits: [NativeUiInvocationMetadata; 8],
}

impl Invocation {
    fn is_current(&self, state: &AppState) -> bool {
        retired_ui_current(state, self.generation) && !self.scope.is_cancelled()
    }

    fn original<'a>(
        &self,
        slot: &'a Option<MountedVault>,
    ) -> Result<&'a NativeTextRemovedJoinedVault, &'static str> {
        match slot {
            Some(MountedVault::TextV2Retired(vault))
                if vault.matches_native_authentication_cached(&self.target) =>
            {
                Ok(vault)
            }
            None => Err(BUSY),
            _ => Err(listing::STALE),
        }
    }

    fn invite(&self) -> Result<NativeTextInvitePacketData<'_>, &'static str> {
        NativeTextInvitePacketData::decode(&self.invite).map_err(|_| INPUT)
    }
}

fn capture(state: &AppState, body: &InvokeBody) -> Result<Invocation, &'static str> {
    let generation = state.ui_session_generation.load(Ordering::SeqCst);
    let target = state
        .native_text
        .ui_state
        .original_target()
        .filter(|target| target.is_retired_mount_data() && target.is_live_cached())
        .ok_or(listing::STALE)?;
    let scope = keep_session::current_retired(state, generation).ok_or(listing::STALE)?;
    let InvokeBody::Json(value) = body else {
        return Err(INPUT);
    };
    let fields = value
        .as_object()
        .filter(|fields| fields.len() == 2)
        .ok_or(INPUT)?;
    let invite = fields
        .get("inviteHex")
        .and_then(serde_json::Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() % 2 == 0
                && value.len() <= 2 * MAX_NATIVE_TEXT_INVITE_PACKET_BYTES
        })
        .ok_or(INPUT)?;
    let host: IpAddr = fields
        .get("approvedHost")
        .and_then(serde_json::Value::as_str)
        .filter(|value| value.len() <= 45)
        .ok_or(INPUT)?
        .parse()
        .map_err(|_| INPUT)?;
    if host.is_unspecified() || host.is_multicast() {
        return Err(INPUT);
    }
    let permit = state
        .native_text
        .inflight
        .clone()
        .try_acquire_owned()
        .map_err(|_| BUSY)?;
    let first = state.native_text.ui_budget.prepare().map_err(|_| BUSY)?;
    let mut credits = Vec::with_capacity(8);
    credits.push(first);
    for _ in 1..8 {
        credits.push(state.native_text.ui_budget.prepare().map_err(|_| BUSY)?);
    }
    let mut decoded = Zeroizing::new(vec![0; invite.len() / 2]);
    hex::decode_to_slice(invite.as_bytes(), &mut decoded).map_err(|_| INPUT)?;
    NativeTextInvitePacketData::decode(&decoded).map_err(|_| INPUT)?;
    Ok(Invocation {
        generation,
        target,
        scope,
        invite: decoded,
        host,
        _permit: permit,
        _credits: credits.try_into().ok().expect("eight independent credits"),
    })
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "step")]
enum Checkpoint {
    RestartToContinue,
    RestartToActivate,
}

enum Start {
    Checkpoint(Checkpoint),
    Local,
    Connect(Box<NativeTextRejoinExchange>),
}

pub(super) fn route<R: tauri::Runtime>(invoke: Invoke<R>, spawn: &impl Fn(Task)) -> bool {
    if invoke.acl.is_none()
        || invoke.message.webview_ref().label() != "main"
        || invoke.message.webview_ref().window().label() != "main"
    {
        invoke
            .resolver
            .reject("Rejoin is unavailable from this window.");
        return true;
    }
    let app = invoke.message.webview_ref().app_handle().clone();
    let state = app.state::<AppState>();
    let call = match capture(&state, invoke.message.payload()) {
        Ok(call) => Arc::new(call),
        Err(error) => {
            invoke.resolver.reject(error);
            return true;
        }
    };
    let work = match state.native_text.rejoin_operation.enter() {
        Ok(work) => work,
        Err(_) => {
            invoke.resolver.reject(BUSY);
            return true;
        }
    };
    let resolver = invoke.resolver;
    drop(invoke.message);
    spawn(Box::pin(async move {
        let state = app.state::<AppState>();
        let result = execute(&state, &call).await;
        drop(work);
        // A covered ID9 changes physical authentication on this mount.
        // Its receipt still names this exact retired owner. Only cold
        // startup is allowed to attach a live row.
        let _commit = state.ui_session_commit.lock().await;
        let store = state.store.lock().await;
        let current = call.is_current(&state)
            && matches!(
                store.as_ref(),
                Some(MountedVault::TextV2Retired(vault))
                    if vault.matches_native_original_mount_cached(&call.target)
            );
        match result {
            Ok(step) if current => resolver.resolve(step),
            Err(error) if current => resolver.reject(error),
            _ => resolver.reject(listing::STALE),
        }
    }));
    true
}

async fn execute(state: &AppState, call: &Arc<Invocation>) -> Result<Checkpoint, String> {
    let original = call.clone();
    let start = files::run(state, call, move |vault, ui| {
        let invite = original.invite()?;
        let selected = vault.rejoin_catalog_is_selected();
        match selected {
            Err(catcoms_app::AppError::ChatV2RejoinCatalogReopen) => {
                return Ok(Start::Checkpoint(Checkpoint::RestartToContinue));
            }
            Err(_) => return Err(FAILED.into()),
            Ok(false) => {
                return match vault.select_rejoin_catalog_for_invite(
                    &ui,
                    &invite,
                    SystemClock.now_ms(),
                    &original.scope,
                    &mut OsCryptoRng,
                ) {
                    Ok(true) | Err(catcoms_app::AppError::ChatV2RejoinCatalogReopen) => {
                        Ok(Start::Checkpoint(Checkpoint::RestartToContinue))
                    }
                    Ok(false) => Err(BUSY.into()),
                    Err(_) => Err(FAILED.into()),
                };
            }
            Ok(true) => {}
        }
        let token = joining::request_token(&invite);
        vault
            .cover_rejoin_pending(
                &ui,
                (&invite, token, SystemClock.now_ms()),
                &original.scope,
                &mut OsCryptoRng,
            )
            .map_err(|_| FAILED)?
            .ok_or(BUSY)?;
        if vault
            .recover_rejoin_received(&ui, (&invite, token), &original.scope, &mut OsCryptoRng)
            .map_err(|_| FAILED)?
            .is_some()
        {
            return Ok(Start::Local);
        }
        let exchange = vault
            .prepare_rejoin_exchange(ui, &invite, token, original.scope.clone())
            .map_err(|_| FAILED)?
            .ok_or(BUSY)?;
        Ok(Start::Connect(Box::new(exchange)))
    })
    .await?;
    if !call.is_current(state) {
        return Err(listing::STALE.into());
    }
    let delivery = match start {
        Start::Checkpoint(step) => return Ok(step),
        Start::Local => None,
        Start::Connect(exchange) => {
            exchange
                .connect_approved_host(&call.invite()?, call.host, &state.endpoint_dials)
                .await
                .map_err(|_| "Could not connect to the invited founder.")?;
            #[cfg(test)]
            {
                let edge = state
                    .native_text
                    .rejoin_network_edge
                    .lock()
                    .unwrap()
                    .clone();
                if let Some(edge) = edge {
                    edge.wait_once().await;
                }
            }
            Some(exchange.receive().await.map_err(|_| FAILED)?)
        }
    };
    let original = call.clone();
    files::run(state, call, move |vault, ui| {
        let invite = original.invite()?;
        let invitation = (&invite, joining::request_token(&invite));
        if let Some(delivery) = delivery {
            delivery
                .cover(vault, invitation, &mut OsCryptoRng)
                .map_err(|_| FAILED)?;
        }
        match vault
            .cover_rejoin_welcome(&ui, invitation, &original.scope, &mut OsCryptoRng)
            .map_err(|_| FAILED)?
        {
            NativeTextRejoinWelcomeStatus::Covered(_) => {}
            NativeTextRejoinWelcomeStatus::WaitingForTransport => return Err(BUSY.into()),
            NativeTextRejoinWelcomeStatus::Held => {
                return Err("This rejoin was interrupted during setup. Its files are safe.".into())
            }
        }
        vault
            .cover_rejoin_prefix(&ui, invitation, &original.scope, &mut OsCryptoRng)
            .map_err(|_| FAILED)?
            .ok_or(BUSY)?;
        vault
            .cover_rejoin_index(&ui, invitation, &original.scope, &mut OsCryptoRng)
            .map_err(|_| FAILED)?
            .ok_or(BUSY)?;
        Ok(Checkpoint::RestartToActivate)
    })
    .await
}
