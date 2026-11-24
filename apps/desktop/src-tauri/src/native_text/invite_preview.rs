//! Authenticate native invite metadata without opening a route or pinning a group.
use super::*;
use catcoms_app::store::{
    NativeTextInvitePacketData, NativeUiInvocationMetadata, MAX_NATIVE_TEXT_INVITE_PACKET_BYTES,
};

const BUSY: &str = "Invite preview is busy. Try again.";
const INVALID: &str = "Enter a valid Chat V2 invite.";
const EXPIRED: &str = "This Chat V2 invite has expired.";
const STALE: &str = listing::STALE;
// Covers the retained hex, decoded packet, bounded signature/policy scratch
// and result through the worker and the final resolver call.
const CREDITS: usize = 8;

struct Work {
    hex: Zeroizing<String>,
    _payment: Arc<Payment>,
}

struct Payment {
    _credits: Vec<NativeUiInvocationMetadata>,
    _permit: OwnedSemaphorePermit,
}

#[derive(serde::Serialize)]
struct NativePreview {
    #[serde(flatten)]
    metadata: InvitePreview,
    approved_host_required: bool,
}

impl Work {
    fn verify(self) -> Result<(NativePreview, Zeroizing<Vec<u8>>), &'static str> {
        let mut bytes = Zeroizing::new(vec![0; self.hex.len() / 2]);
        hex::decode_to_slice(self.hex.as_bytes(), &mut bytes).map_err(|_| INVALID)?;
        let packet = NativeTextInvitePacketData::decode(&bytes).map_err(|_| INVALID)?;
        let binding = packet.binding();
        let preview = NativePreview {
            metadata: InvitePreview {
                direct_routes: 0,
                rendezvous_routes: 0,
                switchboards: 0,
                expires_at_ms: binding.spec_data().expires_at_ms,
                communication_mode: group_policy::mode_name(catcoms_mls::GroupMode::PeerToPeer),
            },
            approved_host_required: true,
        };
        Ok((preview, bytes))
    }
}

fn now_ms(_state: &AppState) -> u64 {
    #[cfg(test)]
    {
        let now = _state.native_text.invite_preview_now.load(Ordering::SeqCst);
        if now != 0 {
            return now;
        }
    }
    SystemClock.now_ms()
}

pub(super) fn route<R: tauri::Runtime>(invoke: Invoke<R>, spawn: &impl Fn(Task)) -> bool {
    if invoke.acl.is_none()
        || invoke.message.webview_ref().label() != "main"
        || invoke.message.webview_ref().window().label() != "main"
    {
        invoke
            .resolver
            .reject("Invite preview is unavailable from this window.");
        return true;
    }
    let app = invoke.message.webview_ref().app_handle().clone();
    let capture = (|| {
        let state = app.state::<AppState>();
        let generation = state.ui_session_generation.load(Ordering::SeqCst);
        if ui_session_closed(&state) || state.native_text.sources.is_closing_cached() {
            return Err(STALE);
        }
        let target = state.native_text.ui_state.original_target().ok_or(STALE)?;
        let cancellation = keep_session::current(&state, generation).ok_or(STALE)?;
        let InvokeBody::Json(value) = invoke.message.payload() else {
            return Err(INVALID);
        };
        let fields = value.as_object().ok_or(INVALID)?;
        let hex = fields
            .get("inviteHex")
            .and_then(serde_json::Value::as_str)
            .ok_or(INVALID)?;
        if fields.len() != 1
            || hex.is_empty()
            || hex.len() % 2 != 0
            || hex.len() > 2 * MAX_NATIVE_TEXT_INVITE_PACKET_BYTES
        {
            return Err(INVALID);
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
        let payment = Arc::new(Payment {
            _credits: credits,
            _permit: permit,
        });
        let work = Work {
            hex: Zeroizing::new(hex.to_owned()),
            _payment: payment.clone(),
        };
        Ok((generation, target, cancellation, work, payment))
    })();
    let (generation, target, cancellation, work, payment) = match capture {
        Ok(value) => value,
        Err(error) => {
            invoke.resolver.reject(error);
            return true;
        }
    };
    let resolver = invoke.resolver;
    drop(invoke.message);
    spawn(Box::pin(async move {
        let _payment = payment;
        let state = app.state::<AppState>();
        if cancellation.is_cancelled() {
            resolver.reject(STALE);
            return;
        }
        let result = tauri::async_runtime::spawn_blocking(move || work.verify())
            .await
            .unwrap_or(Err(INVALID));
        let result = match result {
            Ok((preview, bytes)) if preview.metadata.expires_at_ms <= now_ms(&state) => {
                let proof =
                    joining::capture_once(&state, generation, &target, |sources, command| {
                        sources.capture_joined_selection(command)
                    })
                    .await;
                match proof {
                    Ok(prepared) if prepared.is_capture_busy_cached() => Err(BUSY),
                    Ok(prepared) => {
                        let caller = cancellation.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            let invite =
                                NativeTextInvitePacketData::decode(&bytes).map_err(|_| INVALID)?;
                            prepared
                                .require_recovery_invite(
                                    &invite,
                                    joining::request_token(&invite),
                                    &caller,
                                )
                                .map_err(|_| EXPIRED)?;
                            Ok((preview, true))
                        })
                        .await
                        .unwrap_or(Err(EXPIRED))
                    }
                    Err(_) => Err(STALE),
                }
            }
            Ok((preview, _)) => Ok((preview, false)),
            Err(error) => Err(error),
        };
        let Ok(_commit) = require_ui_session_generation(&state, generation).await else {
            resolver.reject(STALE);
            return;
        };
        let store = state.store.lock().await;
        let original = store
            .as_ref()
            .and_then(MountedVault::as_text_v2)
            .is_some_and(|vault| target.is_for_original_vault(vault));
        if cancellation.is_cancelled() || state.native_text.sources.is_closing_cached() || !original
        {
            resolver.reject(STALE);
            return;
        }
        match result {
            Ok((preview, recovery))
                if recovery || preview.metadata.expires_at_ms > now_ms(&state) =>
            {
                resolver.resolve(preview)
            }
            Ok(_) => resolver.reject(EXPIRED),
            Err(error) => resolver.reject(error),
        }
    }));
    true
}
