//! Historical-content IPC. Uses the parent module's one custody/session path, without UI work.
use super::*;
use catcoms_app::studio::{
    RecoveryReason, RecoveryTransition, StudioControlAction as Action, StudioControlRequest,
    StudioControlResponse as Response, StudioRecoveryApply, StudioRecoveryDisposition,
    StudioRecoveryItem, StudioRecoveryListing, StudioRecoveryMode, StudioRecoverySummary,
};

/// Typed, exact-key choices; no renderer-provided historical value or author is accepted.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum ChoiceInput {
    Frame { id: String, value: String },
    FrameDeletion { id: String },
    Title { value: String },
    Fps { value: String },
    Object { id: String },
    ObjectTitle { id: String, value: String },
    ObjectExpiry { id: String, value: String },
    ObjectDeletion { id: String },
}
impl ChoiceInput {
    pub(super) fn checked(self) -> Result<StudioRecoveryItem, String> {
        use StudioRecoveryItem as I;
        Ok(match self {
            Self::Frame { id: element, value } => I::Frame {
                id: id(&element)?,
                value: hash(&value)?,
            },
            Self::FrameDeletion { id: element } => I::FrameDeletion { id: id(&element)? },
            Self::Title { value } => I::Title {
                value: hash(&value)?,
            },
            Self::Fps { value } => I::Fps {
                value: hash(&value)?,
            },
            Self::Object { id: element } => I::Object { id: id(&element)? },
            Self::ObjectTitle { id: element, value } => I::ObjectTitle {
                id: id(&element)?,
                value: hash(&value)?,
            },
            Self::ObjectExpiry { id: element, value } => I::ObjectExpiry {
                id: id(&element)?,
                value: hash(&value)?,
            },
            Self::ObjectDeletion { id: element } => I::ObjectDeletion { id: id(&element)? },
        })
    }
}
fn mode(mode: &str) -> Result<StudioRecoveryMode, String> {
    match mode {
        "restore" => Ok(StudioRecoveryMode::Restore),
        "copy" => Ok(StudioRecoveryMode::Copy),
        _ => Err("recovery mode must be restore or copy".into()),
    }
}

#[tauri::command]
pub(crate) async fn studio_recovery_preview(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    snapshot: String,
    choice: ChoiceInput,
    mode: String,
) -> Result<Value, String> {
    invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::Preview {
            snapshot: hash(&snapshot)?,
            item: choice.checked()?,
            mode: self::mode(&mode)?,
        },
    )
    .await
}

/// A failed response is retried with this exact payload. Re-previewing is a NEW decision and
/// needs a new nonce; never rewrite a predecessor or body under an earlier nonce.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecoveryApplyInput {
    snapshot: String,
    choice: ChoiceInput,
    mode: String,
    epoch_id: String,
    expected_projection: String,
    nonce: String,
    body: String,
}
impl RecoveryApplyInput {
    pub(super) fn checked(self) -> Result<StudioRecoveryApply, String> {
        if self.body.len() > 64 * 1024 {
            return Err("recovery body exceeds 64 KiB".into());
        }
        Ok(StudioRecoveryApply {
            snapshot: hash(&self.snapshot)?,
            item: self.choice.checked()?,
            mode: mode(&self.mode)?,
            epoch_id: u128::from_be_bytes(id(&self.epoch_id)?),
            expected_projection: hash(&self.expected_projection)?,
            nonce: id(&self.nonce)?,
            body: self.body.into_bytes(),
        })
    }
}
#[tauri::command]
pub(crate) async fn studio_recovery_apply(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    edit: RecoveryApplyInput,
) -> Result<Value, String> {
    invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::Apply(Box::new(edit.checked()?)),
    )
    .await
}

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
pub(crate) async fn studio_recovery_restore_pointer(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
) -> Result<Value, String> {
    invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::RestorePointer,
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
        Response::PointerRestored {
            target,
            epoch,
            registry_epoch_id,
        } => json!({"v":1,"kind":"recoveryPointerRestored",
            "channel":u128::from_be_bytes(target.channel()).to_string(),
            "object":match target {StudioTarget::Index{..}=>None,StudioTarget::Flipnote{object,..}=>Some(hex::encode(object))},
            "checkpointEpoch":epoch.to_string(),"registryEpochId":format!("{registry_epoch_id:032x}"),
            "provisional":true}),
        Response::Preview(v) => {
            json!({"v":1,"kind":"recoveryPreview","snapshot":hex::encode(v.snapshot),
            "epochId":format!("{:032x}",v.epoch_id),"expectedProjection":hex::encode(v.fingerprint),
            "disposition":match v.plan.disposition {
                StudioRecoveryDisposition::Ready=>"ready",StudioRecoveryDisposition::Unchanged=>"unchanged",
                StudioRecoveryDisposition::Conflict=>"conflict",StudioRecoveryDisposition::Deleted=>"deleted",
                StudioRecoveryDisposition::Full=>"full",StudioRecoveryDisposition::MissingTarget=>"missingTarget",
            },"body":v.plan.body.map(String::from_utf8).transpose().map_err(|_|"invalid recovery operation encoding")?,
            "originalAuthor":v.plan.original_author.map(|id|hex::encode(id.as_bytes()))})
        }
        Response::Applied {
            target: _,
            already_saved,
        } => json!({"v":1,"kind":"recoveryApplied",
            "contentSaved":true,"alreadySaved":already_saved,"provisional":true,"pointerRestored":false}),
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
