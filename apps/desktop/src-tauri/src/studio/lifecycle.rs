//! Overlay lifecycle IPC: what a retained draft is, the preserved evidence for it, and the two
//! destructive ends.
//!
//! Three of these four commands are read-only and one of the remaining two destroys evidence that
//! nothing else in the system can recreate. The renderer therefore never supplies a decision as a
//! flag: both destructive paths take a typed confirmation that only an exact literal can build, and
//! both name the exact thing they intend to destroy, so a confirmation cannot be spent on an
//! archive or a branch the user never saw.
use super::*;
use catcoms_app::store::{StudioDisposalRequestMode, StudioOverlayDisposalRequest};
// The replication-side types (StudioDraftArchive, StudioOverlayDisposal, StudioDisposalMode,
// StudioDiscardConfirmation, StudioOverlayProvenance) arrive through the parent's `types::*`.
use catcoms_app::studio::{
    StudioArchiveReleaseRequest, StudioControlAction as Action, StudioControlRequest,
    StudioControlResponse as Response, StudioOverlayLifecycle, StudioReleaseConfirmation,
};
use recovery::{hash, target};

#[tauri::command]
pub(crate) async fn studio_overlay_lifecycle(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
) -> Result<Value, String> {
    recovery::invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::OverlayLifecycle,
    )
    .await
}

#[tauri::command]
pub(crate) async fn studio_overlay_archive_read(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
) -> Result<Value, String> {
    recovery::invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::ReadOverlayArchive,
    )
    .await
}

/// The same read, converted to carry the canonical envelope bytes. A separate command rather than a
/// flag on the read: exporting hands the caller the whole archive, and that is a different thing to
/// ask for than looking at what one is.
#[tauri::command]
pub(crate) async fn studio_overlay_archive_export(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
) -> Result<Value, String> {
    invoke_archive(
        &state,
        server,
        target(&channel, object.as_deref())?,
        |archive, id, physical_bytes| {
            use base64::Engine;
            let bytes = archive.encode().map_err(|e| e.to_string())?;
            let mut value = archive_value(&archive, id, physical_bytes)?;
            value["kind"] = "overlayArchiveExport".into();
            value["format"] = "p1-studio-draft-archive-v1".into();
            value["bytes"] = bytes.len().into();
            value["bytesB64"] = base64::engine::general_purpose::STANDARD.encode(&bytes).into();
            bounded_view(value)
        },
    )
    .await
}

/// Destroy the preserved archive. `archive` is the id from the read that populated the dialog, so a
/// confirmation typed against one archive cannot destroy a different one.
#[tauri::command]
pub(crate) async fn studio_overlay_archive_release(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    archive: String,
    confirmation: String,
) -> Result<Value, String> {
    let confirmation = StudioReleaseConfirmation::parse(&confirmation).ok_or_else(|| {
        format!(
            "releasing a draft archive needs the exact confirmation {:?}",
            StudioReleaseConfirmation::TOKEN
        )
    })?;
    recovery::invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::ReleaseOverlayArchive(Box::new(StudioArchiveReleaseRequest {
            archive: hash(&archive)?,
            confirmation,
        })),
    )
    .await
}

/// How the renderer names a disposal mode. Tagged and `deny_unknown_fields` so a payload that meant
/// to preserve and forgot its archive is a refusal, never a silent discard.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum DisposalInput {
    /// The bodies live on in a durable archive the caller has already read back.
    Preserve,
    /// The bodies are destroyed. Requires the literal the user typed.
    Discard { confirmation: String },
}
impl DisposalInput {
    fn checked(self) -> Result<StudioDisposalRequestMode, String> {
        Ok(match self {
            Self::Preserve => StudioDisposalRequestMode::Preserve,
            Self::Discard { confirmation } => StudioDisposalRequestMode::Discard(
                StudioDiscardConfirmation::parse(&confirmation).ok_or_else(|| {
                    format!(
                        "discarding a local draft needs the exact confirmation {:?}",
                        StudioDiscardConfirmation::TOKEN
                    )
                })?,
            ),
        })
    }
}

/// Drop the live branch. `branch`, `content` and `accepted` all come from the inspection the user
/// saw; the store checks them separately because they fail for different reasons.
#[tauri::command]
pub(crate) async fn studio_overlay_dispose(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    branch: String,
    content: String,
    accepted: usize,
    mode: DisposalInput,
) -> Result<Value, String> {
    recovery::invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::DisposeOverlay(Box::new(StudioOverlayDisposalRequest {
            branch: hash(&branch)?,
            content: hash(&content)?,
            accepted,
            mode: mode.checked()?,
        })),
    )
    .await
}

async fn invoke_archive(
    state: &AppState,
    server: u64,
    target: StudioTarget,
    convert: impl FnOnce(StudioDraftArchive, [u8; 32], u64) -> Result<Value, String>,
) -> Result<Value, String> {
    invoke_custody(
        state,
        server,
        InvokeRequest::Control(StudioControlRequest {
            target,
            action: Action::ReadOverlayArchive,
        }),
        |response| match response {
            InvokeResponse::Control(Response::OverlayArchive {
                archive,
                id,
                physical_bytes,
            }) => convert(*archive, id, physical_bytes),
            _ => Err("mismatched draft archive response".into()),
        },
    )
    .await
}

fn channel_of(target: StudioTarget) -> Value {
    u128::from_be_bytes(target.channel()).to_string().into()
}
fn object_of(target: StudioTarget) -> Value {
    match target {
        StudioTarget::Index { .. } => Value::Null,
        StudioTarget::Flipnote { object, .. } => hex::encode(object).into(),
    }
}
fn provenance_value(provenance: &StudioOverlayProvenance) -> Value {
    match provenance {
        StudioOverlayProvenance::Closing => json!({"kind":"closing"}),
        StudioOverlayProvenance::Unconfirmed {
            provider,
            observed_mls_epoch,
            observed_at_ms,
        } => json!({"kind":"unconfirmed","provider":hex::encode(provider.as_bytes()),
            "observedMlsEpoch":observed_mls_epoch.to_string(),
            "observedAtMs":observed_at_ms.to_string()}),
    }
}
fn disposal_mode_value(mode: StudioDisposalMode) -> Value {
    match mode {
        StudioDisposalMode::Preserved { archive } => {
            json!({"mode":"preserved","archive":hex::encode(archive)})
        }
        StudioDisposalMode::Discarded => json!({"mode":"discarded"}),
    }
}

/// Reading an archive is never a basis, a source or an owner claim, and the view says so rather
/// than leaving a renderer to infer it from the absence of a field.
fn archive_value(
    archive: &StudioDraftArchive,
    id: [u8; 32],
    physical_bytes: u64,
) -> Result<Value, String> {
    Ok(json!({"v":1,"kind":"overlayArchive","archive":hex::encode(id),
        "basis":hex::encode(archive.basis()),"content":hex::encode(archive.content()),
        "author":hex::encode(archive.author().as_bytes()),"accepted":archive.accepted(),
        "physicalBytes":physical_bytes.to_string(),
        "readOnly":true,"authority":false,"provisional":true}))
}

fn lifecycle_value(v: &StudioOverlayLifecycle) -> Result<Value, String> {
    Ok(json!({"v":1,"kind":"overlayLifecycle",
        "channel":channel_of(v.target),"object":object_of(v.target),
        "branch":v.branch.map(hex::encode),"generation":v.generation.to_string(),
        "accepted":v.accepted,"archive":v.archive.map(hex::encode),
        "disposed":v.disposed.map(disposal_mode_value),
        "transferred":v.transferred,"provisional":true}))
}

fn disposal_value(v: &StudioOverlayDisposal) -> Result<Value, String> {
    Ok(json!({"v":1,"kind":"overlayDisposed",
        "channel":channel_of(v.target),"object":object_of(v.target),
        "author":hex::encode(v.author.as_bytes()),"provenance":provenance_value(&v.provenance),
        "basis":hex::encode(v.basis),"branch":hex::encode(v.branch),
        "content":hex::encode(v.content),"generation":v.generation.to_string(),
        "disposal":disposal_mode_value(v.mode),"accepted":v.accepted,
        "sequence":v.sequence.to_string(),"atMs":v.at.to_string(),"terminal":true}))
}

/// The lifecycle half of the one `StudioControlResponse` converter. Kept here rather than in
/// `recovery` so that module stays about recovery; `response_value` delegates.
pub(super) fn response_value(response: Response) -> Result<Value, String> {
    let value = match response {
        Response::OverlayLifecycle(v) => lifecycle_value(&v)?,
        Response::OverlayArchive {
            archive,
            id,
            physical_bytes,
        } => archive_value(&archive, id, physical_bytes)?,
        // Both budgets are closed behind this. The caller reconciles before its next write, and the
        // view says so rather than letting a renderer assume it may immediately save again.
        Response::OverlayArchiveReleased => {
            json!({"v":1,"kind":"overlayArchiveReleased","reconcileRequired":true})
        }
        Response::OverlayDisposed(v) => disposal_value(&v)?,
        _ => return Err("mismatched overlay lifecycle response".into()),
    };
    bounded_view(value)
}

#[cfg(test)]
mod tests;
