//! Historical-content IPC. Uses the parent module's one custody/session path, without UI work.
use super::*;
use catcoms_app::studio::{
    RecoveryReason, RecoveryTransition, StudioControlAction as Action, StudioControlRequest,
    StudioControlResponse as Response, StudioRecoveryListing, StudioRecoverySummary,
};

pub(super) fn hash(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("recovery snapshot id must be 64 lowercase hex characters".into());
    }
    hex::decode(value)
        .map_err(|_| "invalid recovery id".to_string())?
        .try_into()
        .map_err(|_| "invalid recovery id".into())
}
fn target(channel: &str, object: Option<&str>) -> Result<StudioTarget, String> {
    let channel = channel_id(channel)?;
    Ok(match object {
        Some(object) => StudioTarget::Flipnote {
            channel,
            object: id(object)?,
        },
        None => StudioTarget::Index { channel },
    })
}
pub(super) async fn invoke_control(
    state: &AppState,
    server: u64,
    target: StudioTarget,
    action: Action,
) -> Result<Value, String> {
    invoke_custody(
        state,
        server,
        InvokeRequest::Control(StudioControlRequest { target, action }),
        |response| match response {
            InvokeResponse::Control(response) => response_value(response),
            _ => Err("mismatched Studio control response".into()),
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn studio_recovery_list(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
) -> Result<Value, String> {
    invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::List,
    )
    .await
}
#[tauri::command]
pub(crate) async fn studio_recovery_read(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    snapshot: String,
) -> Result<Value, String> {
    invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::Read {
            snapshot: hash(&snapshot)?,
        },
    )
    .await
}
#[tauri::command]
pub(crate) async fn studio_recovery_export(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    snapshot: String,
) -> Result<Value, String> {
    invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::Export {
            snapshot: hash(&snapshot)?,
        },
    )
    .await
}
#[tauri::command]
pub(crate) async fn studio_recovery_acknowledge(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    oldest_snapshot: String,
    staged_snapshot: String,
) -> Result<Value, String> {
    invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::Acknowledge {
            oldest_snapshot: hash(&oldest_snapshot)?,
            staged_snapshot: hash(&staged_snapshot)?,
        },
    )
    .await
}

fn summary(v: &StudioRecoverySummary) -> Value {
    json!({"snapshot":hex::encode(v.id),"epoch":v.epoch.to_string(),"staged":v.staged,
    "bytes":v.encoded_bytes,"reason":match v.reason {
        RecoveryReason::Excluded=>"excluded", RecoveryReason::Rewound=>"rewound",
        RecoveryReason::ConflictOverflow=>"conflictOverflow", RecoveryReason::Repair=>"repair",
    }})
}
fn listing(v: StudioRecoveryListing) -> Result<Value, String> {
    let warning = match v.eviction_pending {
        None => Value::Null,
        Some(RecoveryTransition::EvictionPending {
            oldest_snapshot,
            staged_snapshot,
            deadline_ms,
        }) => {
            json!({"oldestSnapshot":hex::encode(oldest_snapshot),"stagedSnapshot":hex::encode(staged_snapshot),"deadlineMs":deadline_ms.to_string()})
        }
        _ => return Err("invalid recovery warning state".into()),
    };
    Ok(
        json!({"v":1,"kind":"recoveryList","channel":u128::from_be_bytes(v.target.channel()).to_string(),
        "object":match v.target {StudioTarget::Index{..}=>None,StudioTarget::Flipnote{object,..}=>Some(hex::encode(object))},
        "source":v.source.map(|s|json!({"epochId":format!("{:032x}",s.epoch_id),"epoch":s.epoch.to_string(),
            "provisional":true,"phase":match s.phase {EpochPhase::Open=>"open",EpochPhase::Closing=>"closing",EpochPhase::Settled=>"settled",EpochPhase::Fault=>"fault"}})),
        "versions":v.versions.iter().map(summary).collect::<Vec<_>>(),"evictionPending":warning,"pendingIntents":v.pending_intents}),
    )
}
pub(super) fn response_value(response: Response) -> Result<Value, String> {
    let value = match response {
        Response::List(value) => listing(value)?,
        Response::Acknowledged(value) => {
            let mut value = listing(value)?;
            value["kind"] = "recoveryAcknowledged".into();
            value
        }
        Response::Version(value) => json!({"v":1,"kind":"recoveryVersion","historical":true,
            "version":summary(&value.summary),"channel":u128::from_be_bytes(value.projection.channel()).to_string(),
            "content":projection_content(&value.projection)}),
        Response::Export { snapshot, bytes } => {
            use base64::Engine;
            json!({"v":1,"kind":"recoveryExport","snapshot":hex::encode(snapshot),
                "format":"p1-recovery-v1","bytes":bytes.len(),"bytesB64":base64::engine::general_purpose::STANDARD.encode(bytes)})
        }
    };
    bounded_view(value)
}
