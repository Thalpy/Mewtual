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
            value["bytesB64"] = base64::engine::general_purpose::STANDARD
                .encode(&bytes)
                .into();
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
    let request = release_request(&archive, &confirmation)?;
    recovery::invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::ReleaseOverlayArchive(Box::new(request)),
    )
    .await
}

/// Split out so the refusals are reachable without an actor. The confirmation is checked before the
/// archive id so a caller that typed nothing is told that first, rather than being sent away to fix
/// a hex string it will then be refused for anyway.
fn release_request(
    archive: &str,
    confirmation: &str,
) -> Result<StudioArchiveReleaseRequest, String> {
    let confirmation = StudioReleaseConfirmation::parse(confirmation).ok_or_else(|| {
        format!(
            "releasing a draft archive needs the exact confirmation {:?}",
            StudioReleaseConfirmation::TOKEN
        )
    })?;
    Ok(StudioArchiveReleaseRequest {
        archive: hash(archive)?,
        confirmation,
    })
}

/// How the renderer names a disposal mode. Tagged and `deny_unknown_fields` so a payload that is
/// missing, misspells or half-fills its mode is a refusal rather than whichever arm serde can make
/// fit.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum DisposalInput {
    /// The bodies live on in a durable archive the caller has already read back.
    ///
    /// Spelled `{}` rather than as a unit variant deliberately. Serde's internally tagged
    /// representation lets a *unit* variant swallow any other fields in the object, so
    /// `{"kind":"preserve","confirmation":"destroy-local-draft"}` - a caller that meant to discard
    /// and mis-set its kind - would deserialise silently. As a struct variant
    /// `deny_unknown_fields` applies and that payload is refused, which is what a contradictory
    /// request deserves whichever way round the contradiction points.
    Preserve {},
    /// The bodies are destroyed. Requires the literal the user typed.
    Discard { confirmation: String },
}
impl DisposalInput {
    fn checked(self) -> Result<StudioDisposalRequestMode, String> {
        Ok(match self {
            Self::Preserve {} => StudioDisposalRequestMode::Preserve,
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

/// What the renderer echoes back from the lifecycle view and the inspection the user saw.
///
/// One payload rather than three loose arguments because the three values only mean anything
/// together: they all describe the same branch, and the store checks them separately only because
/// they fail for different reasons.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DisposalRequestInput {
    /// Branch identity, from `studio_overlay_lifecycle`.
    branch: String,
    /// The same view's content digest. A disposal naming a branch whose content it never saw is
    /// the thing this field exists to refuse.
    content: String,
    accepted: usize,
    mode: DisposalInput,
}
impl DisposalRequestInput {
    fn checked(self) -> Result<StudioOverlayDisposalRequest, String> {
        Ok(StudioOverlayDisposalRequest {
            branch: hash(&self.branch)?,
            content: hash(&self.content)?,
            accepted: self.accepted,
            mode: self.mode.checked()?,
        })
    }
}

/// Drop the live branch.
#[tauri::command]
pub(crate) async fn studio_overlay_dispose(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    disposal: DisposalRequestInput,
) -> Result<Value, String> {
    recovery::invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::DisposeOverlay(Box::new(disposal.checked()?)),
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
    Ok(
        json!({"v":1,"kind":"overlayArchive","archive":hex::encode(id),
        "basis":hex::encode(archive.basis()),"content":hex::encode(archive.content()),
        "author":hex::encode(archive.author().as_bytes()),"accepted":archive.accepted(),
        "physicalBytes":physical_bytes.to_string(),
        "readOnly":true,"authority":false,"provisional":true}),
    )
}

fn lifecycle_value(v: &StudioOverlayLifecycle) -> Result<Value, String> {
    Ok(json!({"v":1,"kind":"overlayLifecycle",
        "channel":channel_of(v.target),"object":object_of(v.target),
        "branch":v.branch.map(hex::encode),"content":v.content.map(hex::encode),
        "generation":v.generation.to_string(),
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
