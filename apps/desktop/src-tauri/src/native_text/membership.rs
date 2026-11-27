//! Inspect and remove exact published admissions through the native row.
use super::*;
use catcoms_app::store::NativeTextJoinedContactConsent;
use catcoms_replication::chat::owner_header::OwnerBodyId;
use catcoms_sync::V2_MEMBER_CONTACT_BYTES;
use std::net::IpAddr;
use tauri::ipc::InvokeResponseBody;

pub(super) const BUSY: &str = "Chat membership is busy. Retry the request.";
const FAILED: &str = "Chat membership is unavailable for this conversation.";
pub(super) const COMMANDS: &[&str] = &[
    "get_native_chat_members",
    "remove_native_chat_member",
    "share_native_chat_contact",
    "connect_native_chat_contact",
    "forget_native_chat_contacts",
];
const LIST: &str = COMMANDS[0];
const REMOVE: &str = COMMANDS[1];
const SHARE: &str = COMMANDS[2];
const CONNECT: &str = COMMANDS[3];
const FORGET: &str = COMMANDS[4];

enum Operation {
    List,
    Remove(OwnerBodyId),
    Share,
    Connect([u8; V2_MEMBER_CONTACT_BYTES], IpAddr, bool),
    Forget,
}

pub(super) fn handles(command: &str) -> bool {
    COMMANDS.contains(&command)
}

pub(super) fn route<R: tauri::Runtime>(invoke: Invoke<R>, spawn: &impl Fn(Task)) -> bool {
    if invoke.acl.is_none()
        || invoke.message.webview_ref().label() != "main"
        || invoke.message.webview_ref().window().label() != "main"
    {
        invoke.resolver.reject(FAILED);
        return true;
    }
    let app = invoke.message.webview_ref().app_handle().clone();
    let captured = (|| {
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
        let (server, context, operation) =
            parse(invoke.message.command(), invoke.message.payload())?;
        let permit = state
            .native_text
            .inflight
            .clone()
            .try_acquire_owned()
            .map_err(|_| BUSY)?;
        let mut source_payment = Some(state.native_text.ui_budget.prepare().map_err(|_| BUSY)?);
        let table = state.servers.try_lock().map_err(|_| BUSY)?;
        let row = table.native(&server).ok_or(listing::STALE)?;
        if row.data().request != context {
            return Err(listing::STALE);
        }
        let founder = matches!(
            &row.data().selection,
            server_table::NativeSelection::Founder(_)
        );
        if (!founder && matches!(operation, Operation::Remove(_)))
            || (founder
                && matches!(
                    operation,
                    Operation::Share | Operation::Connect(..) | Operation::Forget
                ))
        {
            return Err(FAILED);
        }
        let consent = match (&operation, &row.data().selection) {
            (Operation::Connect(_, _, true), server_table::NativeSelection::Joined(selection)) => {
                Some(selection.capture_contact_consent().map_err(|_| BUSY)?)
            }
            _ => None,
        };
        let source = connected::Source::for_installed(
            generation,
            &target,
            row,
            account,
            &mut source_payment,
        )
        .ok_or(listing::STALE)?;
        Ok((source, operation, founder, consent, permit))
    })();
    let (source, operation, founder, consent, permit) = match captured {
        Ok(value) => value,
        Err(error) => {
            invoke.resolver.reject(error);
            return true;
        }
    };
    let resolver = invoke.resolver;
    drop(invoke.message);
    spawn(Box::pin(async move {
        let _permit = permit;
        let state = app.state::<AppState>();
        let result = perform(&state, &source, operation, founder, consent).await;
        let result = result.and_then(|body| {
            let _commit = state.ui_session_commit.try_lock().map_err(|_| BUSY)?;
            let store = state.store.try_lock().map_err(|_| BUSY)?;
            let table = state.servers.try_lock().map_err(|_| BUSY)?;
            if !source.still_installed(&state, &store, &table) {
                return Err(listing::STALE);
            }
            Ok((body, _commit, store, table))
        });
        match result {
            Ok(((body, _roster), _commit, _store, _table)) => {
                resolver.resolve(InvokeResponseBody::Json(body))
            }
            Err(error) => resolver.reject(error),
        }
    }));
    true
}

async fn perform(
    state: &AppState,
    source: &connected::Source,
    operation: Operation,
    founder: bool,
    consent: Option<NativeTextJoinedContactConsent>,
) -> Result<(String, Option<catcoms_app::store::NativeTextRoster>), &'static str> {
    let negative = source.negative();
    let local = source
        .member_work()
        .foreground(negative.clone())
        .await
        .ok_or(listing::STALE)?;
    if founder {
        let prepared = source.capture(state).await.ok_or(listing::STALE)?;
        return tauri::async_runtime::spawn_blocking(move || {
            let _local = local;
            match operation {
                Operation::List => encode(prepared.governed_roster(&negative).map_err(refusal)?),
                Operation::Remove(target) => {
                    prepared
                        .remove_governed_admission(target, &negative)
                        .map_err(refusal)?;
                    Ok(("null".into(), None))
                }
                _ => Err(FAILED),
            }
        })
        .await
        .map_err(|_| FAILED)?;
    }
    let (prepared, selection) = source.capture_joined(state).await.ok_or(listing::STALE)?;
    if let Operation::Connect(raw, host, remember) = operation {
        let attempt = tauri::async_runtime::spawn_blocking(move || {
            let _local = local;
            prepared
                .prepare_member_contact(&selection, &raw, host, negative)
                .map_err(refusal)
        })
        .await
        .map_err(|_| FAILED)??;
        if remember {
            let proof = attempt
                .connect_for_contact_approval(&state.endpoint_dials, consent.ok_or(listing::STALE)?)
                .await
                .map_err(|_| FAILED)?;
            let caller = source.negative();
            let local = source
                .member_work()
                .foreground(caller.clone())
                .await
                .ok_or(listing::STALE)?;
            let (prepared, selection) = source.capture_joined(state).await.ok_or(listing::STALE)?;
            let commit = contact_commit(state, source, &caller)?;
            tauri::async_runtime::spawn_blocking(move || {
                let (_local, _commit) = (local, commit);
                prepared
                    .remember_member_contact(&selection, raw, proof, &caller, &mut OsCryptoRng)
                    .map_err(refusal)
            })
            .await
            .map_err(|_| FAILED)??;
        } else {
            attempt
                .connect(&state.endpoint_dials)
                .await
                .map_err(|_| FAILED)?;
        }
        return Ok(("null".into(), None));
    }
    if matches!(operation, Operation::Forget) {
        let commit = contact_commit(state, source, &negative)?;
        tauri::async_runtime::spawn_blocking(move || {
            let (_local, _commit) = (local, commit);
            prepared
                .forget_member_contact_approvals(&selection, &negative, &mut OsCryptoRng)
                .map_err(refusal)
        })
        .await
        .map_err(|_| FAILED)??;
        return Ok(("null".into(), None));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _local = local;
        match operation {
            Operation::List => encode(
                prepared
                    .governed_roster(&selection, &negative)
                    .map_err(refusal)?,
            ),
            Operation::Share => {
                let raw = prepared
                    .sign_member_contact(&selection, &negative)
                    .map_err(refusal)?;
                Ok((
                    serde_json::to_string(&hex::encode(raw)).map_err(|_| FAILED)?,
                    None,
                ))
            }
            _ => Err(FAILED),
        }
    })
    .await
    .map_err(|_| FAILED)?
}

fn contact_commit(
    state: &AppState,
    source: &connected::Source,
    caller: &RequestCancellation,
) -> Result<tokio::sync::OwnedMutexGuard<()>, &'static str> {
    // The row permit is already held. Never wait on the UI fence in this
    // order. A successful guard covers the entire physical save.
    let commit = state
        .ui_session_commit
        .clone()
        .try_lock_owned()
        .map_err(|_| BUSY)?;
    if state.ui_session_generation.load(Ordering::SeqCst) != source.generation_data()
        || ui_session_closed(state)
        || state.native_text.sources.is_closing_cached()
        || caller.is_cancelled()
    {
        return Err(listing::STALE);
    }
    Ok(commit)
}

fn refusal(error: catcoms_app::store::V2MemberSaveRefusal) -> &'static str {
    use catcoms_app::store::V2MemberSaveRefusal;
    match error {
        V2MemberSaveRefusal::Busy => BUSY,
        V2MemberSaveRefusal::Cancelled | V2MemberSaveRefusal::Stale => listing::STALE,
        _ => FAILED,
    }
}

fn encode(
    roster: catcoms_app::store::NativeTextRoster,
) -> Result<(String, Option<catcoms_app::store::NativeTextRoster>), &'static str> {
    let data: Vec<_> = roster
        .members()
        .iter()
        .map(|member| {
            let identity = member.device_id();
            let full = hex::encode(identity.as_bytes());
            serde_json::json!({
                "fingerprint": &full[..16], "identity": full, "you": member.you,
                "founder": member.founder, "admission": hex::encode(member.admission.as_bytes()),
            })
        })
        .collect();
    let body = serde_json::to_string(&serde_json::json!({
        "members": data,
        "canRemove": roster.can_remove(),
    }))
    .map_err(|_| FAILED)?;
    Ok((body, Some(roster)))
}

fn parse(command: &str, body: &InvokeBody) -> Result<(u64, [u8; 32], Operation), &'static str> {
    let InvokeBody::Json(value) = body else {
        return Err(FAILED);
    };
    let fields = value.as_object().ok_or(FAILED)?;
    let server = fields
        .get("server")
        .and_then(serde_json::Value::as_u64)
        .filter(|id| *id != 0)
        .ok_or(FAILED)?;
    let context = decode_exact_hex(
        fields
            .get("expectedContext")
            .and_then(serde_json::Value::as_str)
            .ok_or(FAILED)?,
    )?;
    if context == [0; 32] {
        return Err(FAILED);
    }
    let operation = match command {
        LIST if fields.len() == 2 => Operation::List,
        SHARE if fields.len() == 2 => Operation::Share,
        CONNECT if (4..=5).contains(&fields.len()) => {
            let contact = fields
                .get("contact")
                .and_then(serde_json::Value::as_str)
                .ok_or(FAILED)?;
            let host: IpAddr = fields
                .get("approvedHost")
                .and_then(serde_json::Value::as_str)
                .ok_or(FAILED)?
                .parse()
                .map_err(|_| FAILED)?;
            if host.is_unspecified() || host.is_multicast() {
                return Err(FAILED);
            }
            let remember = if fields.len() == 5 {
                fields
                    .get("remember")
                    .and_then(serde_json::Value::as_bool)
                    .ok_or(FAILED)?
            } else {
                false
            };
            Operation::Connect(decode_exact_hex(contact)?, host, remember)
        }
        FORGET if fields.len() == 2 => Operation::Forget,
        REMOVE if fields.len() == 3 => {
            let label = fields
                .get("expectedAdmission")
                .and_then(serde_json::Value::as_str)
                .ok_or(FAILED)?;
            let id = decode_exact_hex(label)?;
            if id == [0; 32] {
                return Err(FAILED);
            }
            Operation::Remove(OwnerBodyId::from_bytes(id))
        }
        _ => return Err(FAILED),
    };
    Ok((server, context, operation))
}

pub(super) fn decode_exact_hex<const N: usize>(text: &str) -> Result<[u8; N], &'static str> {
    if text.len() != 2 * N
        || text
            .bytes()
            .any(|b| !b.is_ascii_digit() && !(b'a'..=b'f').contains(&b))
    {
        return Err(FAILED);
    }
    let mut bytes = [0; N];
    hex::decode_to_slice(text, &mut bytes).map_err(|_| FAILED)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_membership_requires_the_exact_admission() {
        let original_context = [0x3c; 32];
        let body = |mut json: serde_json::Value| {
            json["expectedContext"] = serde_json::json!(hex::encode(original_context));
            InvokeBody::Json(json)
        };
        assert!(!handles("get_members"));
        assert!(!handles("remove_member"));
        assert!(matches!(
            parse(LIST, &body(serde_json::json!({"server": 1}))),
            Ok((1, context, Operation::List)) if context == original_context
        ));
        let label = "ab".repeat(32);
        let target = OwnerBodyId::from_bytes([0xab; 32]);
        assert!(
            matches!(parse(REMOVE, &body(serde_json::json!({"server": 1, "expectedAdmission": &label}))), Ok((1, context, Operation::Remove(actual))) if context == original_context && actual == target)
        );
        for invalid in [
            serde_json::json!({"server": 1}),
            serde_json::json!({"server": 1, "expectedContext": "00".repeat(32)}),
            serde_json::json!({"server": 1, "expectedContext": "3c".repeat(8)}),
        ] {
            assert!(parse(LIST, &InvokeBody::Json(invalid)).is_err());
        }
        for value in [
            serde_json::json!({"server": 1, "fp": &label}),
            serde_json::json!({"server": 1, "expectedAdmission": &label[..16]}),
            serde_json::json!({"server": 1, "expectedAdmission": label.to_ascii_uppercase()}),
            serde_json::json!({"server": 1, "expectedAdmission": "00".repeat(32)}),
            serde_json::json!({"server": 1, "expectedAdmission": &label, "force": true}),
            serde_json::json!({"server": 0, "expectedAdmission": &label}),
        ] {
            assert!(parse(REMOVE, &body(value)).is_err());
        }
        let contact = "ab".repeat(V2_MEMBER_CONTACT_BYTES);
        assert!(matches!(
            parse(
                CONNECT,
                &body(serde_json::json!({
                    "server": 1, "contact": &contact, "approvedHost": "127.0.0.1",
                }))
            ),
            Ok((1, context, Operation::Connect(_, _, false))) if context == original_context
        ));
        assert!(matches!(
            parse(
                CONNECT,
                &body(serde_json::json!({
                    "server": 1, "contact": &contact, "approvedHost": "127.0.0.1", "remember": true,
                }))
            ),
            Ok((1, context, Operation::Connect(_, _, true))) if context == original_context
        ));
        assert!(parse(
            CONNECT,
            &body(serde_json::json!({
                "server": 1, "contact": &contact, "approvedHost": "127.0.0.1", "remember": "true",
            }))
        )
        .is_err());
        for host in ["0.0.0.0", "localhost", "ff01::1", "::"] {
            assert!(parse(
                CONNECT,
                &body(serde_json::json!({
                    "server": 1, "contact": &contact, "approvedHost": host,
                }))
            )
            .is_err());
        }
        assert!(parse(
            CONNECT,
            &body(serde_json::json!({
                "server": 1, "contact": &contact,
            }))
        )
        .is_err());
    }
}
