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
    StudioControlResponse as Response, StudioOverlayLifecycle, StudioPreparedInspection,
    StudioReleaseConfirmation,
};
use recovery::{named_hash, target};

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
            let mut value = with_payload(archive_value(&archive, id, physical_bytes)?, &archive)?;
            value["kind"] = "overlayArchiveExport".into();
            bounded_view(value)
        },
    )
    .await
}

/// Hand the caller the canonical payload of the **live** draft, writing nothing.
///
/// The same two visits and the same rebuild as archiving. That sharing is the point rather than an
/// economy: a draft that cannot be replayed must still be exportable, and two serializers would
/// drift, with the one that drifted being the one a user reaches for when their work will not open.
pub(crate) async fn studio_overlay_export(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
) -> Result<Value, String> {
    two_visit_archive(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::ExportOverlay,
        Action::FinishOverlayExport,
        "overlay export",
    )
    .await
}

/// Preserve the live draft as a durable archive. Two custody visits with a detached rebuild between
/// them, like `studio_overlay_read`, because the rebuild is the expensive part and the write is the
/// part that needs custody.
///
/// Not confirmed: this only ever adds evidence. The confirmations in this module guard the two
/// commands that remove it.
pub(crate) async fn studio_overlay_archive(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
) -> Result<Value, String> {
    two_visit_archive(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::ArchiveOverlay,
        Action::FinishOverlayArchive,
        "overlay archive",
    )
    .await
}

/// The shape both archive and export take: begin, rebuild detached, finish. One body rather than
/// two near-copies, because the sharing is the contract - they must produce the same payload.
async fn two_visit_archive(
    state: &AppState,
    server: u64,
    target: StudioTarget,
    begin: Action,
    finish: impl FnOnce(Box<StudioPreparedInspection>) -> Action,
    what: &str,
) -> Result<Value, String> {
    let context = InvokeContext::new(state, server, Some(target)).await?;
    let mismatched = || format!("mismatched {what} response");
    // Decided from the first visit's action before it moves: only archiving writes.
    let writes = matches!(begin, Action::ArchiveOverlay);
    let job = invoke_with_context(
        state,
        &context,
        InvokeRequest::Control(StudioControlRequest {
            target,
            action: begin,
        }),
        |response| match response {
            InvokeResponse::Control(Response::OverlayPreparation(job)) => Ok(job),
            _ => Err(mismatched()),
        },
    )
    .await?;
    let mut cancellation = context.cancellation.clone();
    // The archive rebuild, not the draft rebuild: typed reconstruction is attempted and labelled
    // here rather than required, so a branch nobody can replay can still be preserved or exported.
    let prepared = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(format!("{what} cancelled")),
        result = job.rebuild_for_archive() => result.map_err(|e| e.to_string())?,
    };
    // An archive's finish visit WRITES. `invoke_with_context` already reports a written archive
    // whose result cannot be delivered as uncertain; these are the two ways the visit can fail
    // after its write without producing a result at all: the store says so with the uncertain
    // marker, or the actor dropped the reply. Neither is a refusal, and neither must read as one.
    // Every other error is a pre-write refusal and keeps its plain text.
    let result = invoke_with_context(
        state,
        &context,
        InvokeRequest::Control(StudioControlRequest {
            target,
            action: finish(Box::new(prepared)),
        }),
        |response| match response {
            InvokeResponse::Control(response) => response_value(response),
            _ => Err(mismatched()),
        },
    )
    .await;
    if writes {
        result.map_err(archive_failure)
    } else {
        result
    }
}

/// How an archive's finish visit reports a failure. Uncertain when it may have followed the write:
/// the store marked it so, or the actor dropped the reply after the lease moved. Everything else
/// is a pre-write refusal and keeps its plain text; an already-classified uncertain outcome passes
/// through unchanged.
fn archive_failure(error: String) -> String {
    if error.starts_with("outcome=uncertain; ") {
        error
    } else if error.contains(catcoms_app::UNCERTAIN_OUTCOME)
        || error == catcoms_app::studio::CONTROL_REPLY_DROPPED
    {
        format!("{} ({error})", super::UNDELIVERED_ARCHIVE)
    } else {
        error
    }
}

/// Destroy the preserved archive. `archive` is the id from the read that populated the dialog, so a
/// confirmation typed against one archive cannot destroy a different one.
pub(crate) async fn studio_overlay_archive_release(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    archive: String,
    confirmation: String,
) -> Result<Value, String> {
    let request = release_request(&archive, &confirmation)?;
    classified(
        recovery::invoke_control(
            &state,
            server,
            target(&channel, object.as_deref())?,
            Action::ReleaseOverlayArchive(Box::new(request)),
        )
        .await,
    )
}

/// Mark the outcome a destructive command must never let a renderer confuse with a clean refusal.
///
/// Release in particular can fail *after* the unlink, and section 12.1 is explicit that such a
/// caller must reconcile rather than resend: an exact retry finds nothing and is refused, because
/// the store cannot tell "already released" from "never had one". A renderer that read that as an
/// ordinary refusal would show the archive as still present.
///
/// Keyed off `catcoms_app::UNCERTAIN_OUTCOME` rather than a locally written phrase, because the
/// actor's reply channel carries `Result<_, String>` and the rendered text is all that survives it.
fn classified(result: Result<Value, String>) -> Result<Value, String> {
    result.map_err(|error| {
        if error.contains(catcoms_app::UNCERTAIN_OUTCOME) {
            format!("outcome=uncertain; {error}")
        } else {
            error
        }
    })
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
        archive: named_hash("draft archive", archive)?,
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
            branch: named_hash("branch", &self.branch)?,
            content: named_hash("branch content", &self.content)?,
            accepted: self.accepted,
            mode: self.mode.checked()?,
        })
    }
}

/// Drop the live branch.
pub(crate) async fn studio_overlay_dispose(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    disposal: DisposalRequestInput,
) -> Result<Value, String> {
    classified(
        recovery::invoke_control(
            &state,
            server,
            target(&channel, object.as_deref())?,
            Action::DisposeOverlay(Box::new(disposal.checked()?)),
        )
        .await,
    )
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
///
/// `terminal`, not `provisional`. Everywhere else in this surface "provisional" means an unsettled
/// Save that may still change; an archive is finished evidence that only a release can alter.
fn archive_value(
    archive: &StudioDraftArchive,
    id: [u8; 32],
    physical_bytes: u64,
) -> Result<Value, String> {
    Ok(
        json!({"v":1,"kind":"overlayArchive","archive":hex::encode(id),
        "basis":hex::encode(archive.basis()),"content":hex::encode(archive.content()),
        "branch":hex::encode(archive.branch()),"generation":archive.generation().to_string(),
        "author":hex::encode(archive.author().as_bytes()),"accepted":archive.accepted(),
        "provenance":provenance_value(&archive.provenance()),
        // The archive's own label, recorded when it was written. False says the branch was already
        // unreplayable at preservation time; it never means the archive is damaged.
        "replayable":archive.replayable(),
        "physicalBytes":physical_bytes.to_string(),
        "readOnly":true,"authority":false,"terminal":true}),
    )
}

/// Attach the canonical bytes. One place, so an export of a live draft and an export of a stored
/// archive cannot disagree about the format they name.
fn with_payload(mut value: Value, archive: &StudioDraftArchive) -> Result<Value, String> {
    use base64::Engine;
    let bytes = archive.encode().map_err(|e| e.to_string())?;
    // The design's name (section 11), not one invented here: Agent 4 wires the UI-hooks row
    // against this literal, and a format string that differs from the contract is a format string
    // nobody can look up.
    value["format"] = "catcoms-studio-draft-v1".into();
    value["bytes"] = bytes.len().into();
    value["bytesB64"] = base64::engine::general_purpose::STANDARD
        .encode(&bytes)
        .into();
    Ok(value)
}

/// Every branch-scoped fact carries the branch it is about.
///
/// `archive` and `disposed` routinely describe *other* generations than `branch`: an archive
/// outlives the branch it preserved until someone releases it, and a retained disposal of
/// generation N sits beside a live generation N+1. Flattened into bare presence flags, a renderer
/// would tell the user their current work is preserved when the archive is evidence for work they
/// already disposed of - and the preserving disposal they then ask for is refused at D4.
fn lifecycle_value(v: &StudioOverlayLifecycle) -> Result<Value, String> {
    let (eligibility, manual_reason) = eligibility_fields(v.eligibility);
    Ok(json!({"v":1,"kind":"overlayLifecycle",
        "channel":channel_of(v.target),"object":object_of(v.target),
        "branch":v.branch.as_ref().map(|b| json!({"branch":hex::encode(b.id),
            "content":hex::encode(b.content),"generation":b.generation.to_string(),
            "accepted":b.accepted})),
        "prepared":v.prepared,
        "eligibility":eligibility,
        "manualReason":manual_reason,
        "archive":v.archive.as_ref().map(|a| json!({"archive":hex::encode(a.id),
            "branch":hex::encode(a.branch),"generation":a.generation.to_string(),
            "replayable":a.replayable})),
        "disposed":v.disposed.as_ref().map(|d| {
            let mut value = disposal_mode_value(d.mode);
            value["branch"] = hex::encode(d.branch).into();
            value["generation"] = d.generation.to_string().into();
            value
        }),
        "transferred":v.transferred}))
}

/// P2's two fields, design section 11's `eligibility` and `manualReason`, from one classification.
///
/// One function for both reads, so the lifecycle row and the inspection can never disagree about
/// what a reason is called. `null` for both when no branch is live. The reason names are section
/// 11's, plus `sourceUnreadable`, which the backend adds rather than failing the whole read.
pub(super) fn eligibility_fields(
    eligibility: Option<catcoms_app::studio::types::StudioOverlayEligibility>,
) -> (Value, Value) {
    use catcoms_app::studio::types::{
        StudioOverlayEligibility as E, StudioOverlayManualReason as R,
    };
    match eligibility {
        None => (Value::Null, Value::Null),
        Some(E::Transferable) => ("transferable".into(), Value::Null),
        Some(E::Manual(reason)) => (
            "manual".into(),
            match reason {
                R::NotReplayable => "notReplayable",
                R::Unconfirmed => "unconfirmed",
                R::NotCurrentAuthor => "notCurrentAuthor",
                R::SourceMissing => "sourceMissing",
                R::SourceUnreadable => "sourceUnreadable",
                R::Fault => "fault",
                R::SourceRewound => "sourceRewound",
                R::SourceNotClosing => "sourceNotClosing",
                R::SuccessorMissing => "successorMissing",
                R::ReceiptChanged => "receiptChanged",
                R::SourceReplaced => "sourceReplaced",
                R::SuccessorNotPristine => "successorNotPristine",
                R::ObjectMissing => "objectMissing",
                R::TenureUnknown => "tenureUnknown",
            }
            .into(),
        ),
    }
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
        // Read through the delivery, which refuses once the actor's handoff has expired or been
        // revoked; the result is never read unfenced.
        Response::OverlayArchived(delivered) => delivered.inspect(|v| {
            let mut value = archive_value(&v.archive, v.id, v.physical_bytes)?;
            value["kind"] = "overlayArchived".into();
            // Said positively and separately from the archive's own label: a user who archived an
            // unreplayable branch has preserved their work and should be told so, not left to read
            // a bare false as a failure.
            value["preserved"] = true.into();
            value["notReplayable"] = match &v.replayable {
                Ok(()) => Value::Null,
                Err(reason) => reason.clone().into(),
            };
            Ok::<_, String>(value)
        })??,
        // Nothing was written, so there is no physical size to report and the view must not invent
        // one. The payload is the point.
        Response::OverlayExport(delivered) => delivered.inspect(|v| {
            let mut value = with_payload(archive_value(&v.archive, v.id, 0)?, &v.archive)?;
            value["kind"] = "overlayExport".into();
            value["preserved"] = false.into();
            value["notReplayable"] = match &v.replayable {
                Ok(()) => Value::Null,
                Err(reason) => reason.clone().into(),
            };
            value.as_object_mut().unwrap().remove("physicalBytes");
            Ok::<_, String>(value)
        })??,
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
