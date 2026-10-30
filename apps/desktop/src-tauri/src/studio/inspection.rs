//! Read retained local drafts through two visits to the same native operation context.
use super::*;
use catcoms_app::studio::{
    StudioControlAction as Action, StudioControlRequest, StudioControlResponse as Response,
    StudioOverlayInspection,
};

#[tauri::command]
pub(crate) async fn studio_overlay_read(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    object_kind: Option<String>,
) -> Result<Value, String> {
    let target = recovery::target_with_kind(&channel, object.as_deref(), object_kind.as_deref())?;
    read(&state, server, target).await
}

pub(super) async fn read(
    state: &AppState,
    server: u64,
    target: StudioTarget,
) -> Result<Value, String> {
    read_with(state, server, target, |_| {}, view).await
}

async fn read_with(
    state: &AppState,
    server: u64,
    target: StudioTarget,
    after_rebuild: impl FnOnce(&InvokeContext),
    convert: impl FnOnce(StudioOverlayInspection) -> Result<Value, String>,
) -> Result<Value, String> {
    // Read-only: leaves the last inline slot to a member's Save.
    let context = InvokeContext::background(state, server, Some(target)).await?;
    let job = invoke_with_context(
        state,
        &context,
        InvokeRequest::Control(StudioControlRequest {
            target,
            action: Action::InspectOverlay,
        }),
        |response| match response {
            InvokeResponse::Control(Response::OverlayPreparation(job)) => Ok(job),
            _ => Err("mismatched overlay capture response".into()),
        },
    )
    .await?;
    let mut cancellation = context.cancellation.clone();
    let prepared = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err("overlay inspection cancelled".into()),
        result = job.rebuild() => result.map_err(|e| e.to_string())?,
    };
    after_rebuild(&context);
    invoke_with_context(
        state,
        &context,
        InvokeRequest::Control(StudioControlRequest {
            target,
            action: Action::FinishOverlayInspection(Box::new(prepared)),
        }),
        |response| match response {
            InvokeResponse::Control(Response::OverlayInspection(read)) => convert(read),
            _ => Err("mismatched overlay inspection response".into()),
        },
    )
    .await
}

#[cfg(test)]
mod tests;

/// Design section 11's `OverlayInspection`, as far as this scope's prerequisites reach.
///
/// Three shapes, not two. `absent` and `disposed` are different answers: "there is nothing here"
/// and "there was, and here is the terminal record of what happened to it". Collapsing them would
/// tell a user their work vanished.
///
/// **`replayable: false` is a local-draft, not a failure.** Every structural field is present and
/// only `content` is null. The work is still there and still exportable; only the typed view of it
/// is unavailable, and saying "the read failed" instead is both less true and less useful.
///
/// `eligibility` and `manualReason` are P2's classification, shared with the lifecycle row through
/// `lifecycle::eligibility_fields`. `unconfirmedState` is design 8.6's reconciliation, shared
/// through `lifecycle::unconfirmed_state_value`, and null for a Closing branch. Not present, and
/// not by oversight: `archived`, which belongs to `studio_overlay_lifecycle`, since that reads the
/// archive record this capture deliberately does not hold.
fn view(read: StudioOverlayInspection) -> Result<Value, String> {
    read.inspect(|v| {
        let channel = u128::from_be_bytes(v.target.channel()).to_string();
        let object = match v.target {
            StudioTarget::Index { .. } => None,
            StudioTarget::Flipnote { object, .. } | StudioTarget::Score { object, .. } => {
                Some(hex::encode(object))
            }
        };
        let provenance = match v.provenance {
            Some(StudioOverlayProvenance::Closing) => Some("closing"),
            Some(StudioOverlayProvenance::Unconfirmed { .. }) => Some("unconfirmed"),
            None => None,
        };
        let (eligibility, manual_reason) = super::lifecycle::eligibility_fields(v.eligibility);
        let mut value = match (v.branch, v.disposed) {
            (Some(branch), _) => json!({"v":1,"kind":"local-draft","channel":channel,
                "eligibility":eligibility,
                "manualReason":manual_reason,
                "handoffAttempt":super::lifecycle::handoff_attempt_value(v.handoff_attempt),
                "unconfirmedState":super::lifecycle::unconfirmed_state_value(v.unconfirmed),
                "object":object,
                "basis":v.draft.map(|d| hex::encode(d.basis())),
                "branch":hex::encode(branch),
                // Design section 11 calls the branch digest `content` and the typed projection
                // `content_`. That is inverted here on purpose: `content` has been the projection
                // in this view since before this scope, Agent 4 is already wired to it, and
                // renaming a live key to free up a nicer name for a new one is how a renderer ends
                // up reading a digest as a document.
                "contentId":v.content.map(hex::encode),
                "generation":v.generation.to_string(),
                "accepted":v.draft.map_or(0, |d| d.accepted()),
                "transferState":if v.prepared {"prepared"} else {"active"},
                "provenance":provenance,
                "replayable":v.replayable,
                "readOnly":true,
                "content":v.draft.map(|d| projection_content(d.projection())).transpose()?}),
            (None, Some(d)) => json!({"v":1,"kind":"disposed","channel":channel,"object":object,
                "basis":hex::encode(d.basis),"branch":hex::encode(d.branch),
                "generation":d.generation.to_string(),"accepted":d.accepted,
                "mode":match d.mode {
                    StudioDisposalMode::Preserved{..} => "preserved",
                    StudioDisposalMode::Discarded => "discarded",
                },
                "archive":match d.mode {
                    StudioDisposalMode::Preserved{archive} => Some(hex::encode(archive)),
                    StudioDisposalMode::Discarded => None,
                },
                "readOnly":true}),
            (None, None) => json!({"v":1,"kind":"absent","channel":channel,"object":object}),
        };
        if let Some(draft) = v.draft {
            // These describe the exact displayed local projection, not the live owner's epoch.
            // A local-only .pixa derivation must never substitute a different base version.
            value["contentEpoch"] = draft.projection().epoch().to_string().into();
            value["group"] = hex::encode(&draft.projection().document().server_id).into();
        }
        if matches!(v.target, StudioTarget::Score { .. }) {
            value["objectKind"] = "score".into();
        }
        bounded_view(value)
    })?
}
