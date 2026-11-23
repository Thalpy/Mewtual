//! Inspect and remove exact published admissions through the native row.
use super::*;
use catcoms_replication::chat::owner_header::OwnerBodyId;
use tauri::ipc::InvokeResponseBody;

const BUSY: &str = "Chat membership is busy. Retry the request.";
const FAILED: &str = "Chat membership is unavailable for this conversation.";

enum Operation {
    List,
    Remove(OwnerBodyId),
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
        let (server, operation) = parse(invoke.message.command(), invoke.message.payload())?;
        let permit = state
            .native_text
            .inflight
            .clone()
            .try_acquire_owned()
            .map_err(|_| BUSY)?;
        let mut source_payment = Some(state.native_text.ui_budget.prepare().map_err(|_| BUSY)?);
        let table = state.servers.try_lock().map_err(|_| BUSY)?;
        let row = table.native(&server).ok_or(listing::STALE)?;
        let founder = matches!(
            &row.data().selection,
            server_table::NativeSelection::Founder(_)
        );
        if !founder && matches!(operation, Operation::Remove(_)) {
            return Err(FAILED);
        }
        let source =
            connected::Source::for_installed(generation, &target, row, account, &mut source_payment)
                .ok_or(listing::STALE)?;
        Ok((source, operation, founder, permit))
    })();
    let (source, operation, founder, permit) = match captured {
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
        let result = perform(&state, &source, operation, founder).await;
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
            }
        })
        .await
        .map_err(|_| FAILED)?;
    }
    let (prepared, selection) = source.capture_joined(state).await.ok_or(listing::STALE)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _local = local;
        encode(
            prepared
                .governed_roster(&selection, &negative)
                .map_err(refusal)?,
        )
    })
    .await
    .map_err(|_| FAILED)?
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
            let identity = catcoms_crypto::DeviceId::from_public_key_bytes(&member.signer);
            let full = hex::encode(identity.as_bytes());
            serde_json::json!({
                "fingerprint": &full[..16], "identity": full, "you": member.you,
                "founder": member.founder, "admission": hex::encode(member.admission.as_bytes()),
            })
        })
        .collect();
    let body = serde_json::to_string(&data).map_err(|_| FAILED)?;
    Ok((body, Some(roster)))
}

fn parse(command: &str, body: &InvokeBody) -> Result<(u64, Operation), &'static str> {
    let InvokeBody::Json(value) = body else {
        return Err(FAILED);
    };
    let fields = value.as_object().ok_or(FAILED)?;
    let server = fields
        .get("server")
        .and_then(serde_json::Value::as_u64)
        .filter(|id| *id != 0)
        .ok_or(FAILED)?;
    let operation = match command {
        "get_members" if fields.len() == 1 => Operation::List,
        "remove_member" if fields.len() == 2 => {
            let label = fields
                .get("expectedAdmission")
                .and_then(serde_json::Value::as_str)
                .ok_or(FAILED)?;
            if label.len() != 64
                || label
                    .bytes()
                    .any(|b| !b.is_ascii_digit() && !(b'a'..=b'f').contains(&b))
            {
                return Err(FAILED);
            }
            let mut id = [0; 32];
            hex::decode_to_slice(label, &mut id).map_err(|_| FAILED)?;
            if id == [0; 32] {
                return Err(FAILED);
            }
            Operation::Remove(OwnerBodyId::from_bytes(id))
        }
        _ => return Err(FAILED),
    };
    Ok((server, operation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_membership_requires_the_exact_admission() {
        let body = |json| InvokeBody::Json(json);
        assert!(matches!(
            parse("get_members", &body(serde_json::json!({"server": 1}))),
            Ok((1, Operation::List))
        ));
        let label = "ab".repeat(32);
        let target = OwnerBodyId::from_bytes([0xab; 32]);
        assert!(
            matches!(parse("remove_member", &body(serde_json::json!({"server": 1, "expectedAdmission": &label}))), Ok((1, Operation::Remove(actual))) if actual == target)
        );
        for value in [
            serde_json::json!({"server": 1, "fp": &label}),
            serde_json::json!({"server": 1, "expectedAdmission": &label[..16]}),
            serde_json::json!({"server": 1, "expectedAdmission": label.to_ascii_uppercase()}),
            serde_json::json!({"server": 1, "expectedAdmission": "00".repeat(32)}),
            serde_json::json!({"server": 1, "expectedAdmission": &label, "force": true}),
            serde_json::json!({"server": 0, "expectedAdmission": &label}),
        ] {
            assert!(parse("remove_member", &body(value)).is_err());
        }
    }
}
