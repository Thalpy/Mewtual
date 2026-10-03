//! Copy-into-current IPC: preview, then apply.
//!
//! The renderer never supplies a plan. It names a source, a destination and one element, gets back
//! a bounded proposed body or an explicit hold, and echoes that body back to apply it. The actor
//! re-derives the plan from records read at apply time and refuses anything that no longer matches,
//! so the echo is a convenience for the renderer and never a shortcut past revalidation.
//!
//! **Copy is projection-level and the view says so.** It recovers the selected value of an element
//! as a new operation authored by the copier; a branch entry superseded within the branch, a
//! conflict alternative, the original authorship of an accepted envelope and the accepted ordering
//! are all lost. `sourceOps` reports exactly which source operations the proposal resolved. No count
//! of copied items establishes that a branch was preserved: only an archive does, which is why
//! `preservesBranch` is stated as `false` rather than left to be inferred.
use super::*;
use catcoms_app::store::StudioOverlayCopyChoice;
use catcoms_app::studio::{
    StudioControlAction as Action, StudioControlRequest, StudioControlResponse as Response,
    StudioOverlayCopyApply, StudioOverlayCopyPreview, StudioRecoveryDisposition,
    StudioRecoveryMode,
};
use recovery::{named_hash, target, ChoiceInput};

fn mode(mode: &str) -> Result<StudioRecoveryMode, String> {
    match mode {
        "restore" => Ok(StudioRecoveryMode::Restore),
        "copy" => Ok(StudioRecoveryMode::Copy),
        _ => Err("copy mode must be restore or copy".into()),
    }
}

/// Where a copy is going. Spelled out rather than reusing the source's channel implicitly, because a
/// cross-document copy names a different object and a renderer that could omit the channel would be
/// relying on this layer to guess it.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DestinationInput {
    channel: String,
    object: Option<String>,
}
impl DestinationInput {
    fn checked(&self) -> Result<StudioTarget, String> {
        target(&self.channel, self.object.as_deref())
    }
}

pub(crate) async fn studio_overlay_copy_preview(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    destination: DestinationInput,
    choice: ChoiceInput,
    mode: String,
) -> Result<Value, String> {
    let source = target(&channel, object.as_deref())?;
    let choice = StudioOverlayCopyChoice {
        destination: destination.checked()?,
        item: choice.checked()?,
        mode: self::mode(&mode)?,
    };
    let context = InvokeContext::new(&state, server, Some(source)).await?;
    let job = invoke_with_context(
        &state,
        &context,
        InvokeRequest::Control(StudioControlRequest {
            target: source,
            action: Action::PrepareOverlayCopy(Box::new(choice)),
        }),
        |response| match response {
            InvokeResponse::Control(Response::OverlayCopyPreparation(job)) => Ok(job),
            _ => Err("mismatched overlay copy response".into()),
        },
    )
    .await?;
    let mut cancellation = context.cancellation.clone();
    let prepared = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err("overlay copy cancelled".into()),
        result = job.plan() => result.map_err(|e| e.to_string())?,
    };
    invoke_with_context(
        &state,
        &context,
        InvokeRequest::Control(StudioControlRequest {
            target: source,
            action: Action::FinishOverlayCopyPreview(Box::new(prepared)),
        }),
        |response| match response {
            // Through the delivery, never unfenced: see `invoke_with_context`.
            InvokeResponse::Control(Response::OverlayCopyPreview(preview)) => {
                preview.inspect(preview_value)?
            }
            _ => Err("mismatched overlay copy response".into()),
        },
    )
    .await
}

/// A failed apply is retried with this exact payload. **Re-previewing is a new decision and needs a
/// new nonce**; never rewrite a predecessor's body under an earlier nonce, or the exact-retry
/// shortcut would acknowledge an operation the user did not send.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CopyApplyInput {
    destination: DestinationInput,
    choice: ChoiceInput,
    mode: String,
    epoch_id: String,
    expected_projection: String,
    nonce: String,
    body: String,
}
impl CopyApplyInput {
    fn checked(self) -> Result<StudioOverlayCopyApply, String> {
        if self.body.len() > 64 * 1024 {
            return Err("copy body exceeds 64 KiB".into());
        }
        Ok(StudioOverlayCopyApply {
            destination: self.destination.checked()?,
            item: self.choice.checked()?,
            mode: mode(&self.mode)?,
            epoch_id: u128::from_be_bytes(id(&self.epoch_id)?),
            expected_projection: named_hash("expected projection", &self.expected_projection)?,
            nonce: id(&self.nonce)?,
            body: self.body.into_bytes(),
        })
    }
}

pub(crate) async fn studio_overlay_copy_apply(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    edit: CopyApplyInput,
) -> Result<Value, String> {
    recovery::invoke_control(
        &state,
        server,
        target(&channel, object.as_deref())?,
        Action::ApplyOverlayCopy(Box::new(edit.checked()?)),
    )
    .await
}

fn disposition(value: StudioRecoveryDisposition) -> &'static str {
    match value {
        StudioRecoveryDisposition::Ready => "ready",
        StudioRecoveryDisposition::Unchanged => "unchanged",
        StudioRecoveryDisposition::Conflict => "conflict",
        StudioRecoveryDisposition::Deleted => "deleted",
        StudioRecoveryDisposition::Full => "full",
        StudioRecoveryDisposition::MissingTarget => "missingTarget",
    }
}

pub(super) fn preview_value(v: &StudioOverlayCopyPreview) -> Result<Value, String> {
    let describe = |t: StudioTarget| {
        json!({"channel":u128::from_be_bytes(t.channel()).to_string(),
        "object":match t {
            StudioTarget::Index{..} => Value::Null,
            StudioTarget::Flipnote{object,..} => hex::encode(object).into(),
        }})
    };
    bounded_view(json!({"v":1,"kind":"overlayCopyPreview",
        "source":describe(v.source),"destination":describe(v.destination),
        "epochId":format!("{:032x}", v.epoch_id),
        "expectedProjection":hex::encode(v.expected_projection),
        "disposition":disposition(v.disposition),
        "body":v.body.clone().map(String::from_utf8).transpose()
            .map_err(|_| "invalid copy operation encoding")?,
        "originalAuthor":v.original_author.map(|id| hex::encode(id.as_bytes())),
        // Exactly what the proposal resolved, and a flat statement that it is not preservation.
        "sourceOps":v.source_ops.iter().map(hex::encode).collect::<Vec<_>>(),
        "preservesBranch":false,"provisional":true}))
}

#[cfg(test)]
mod tests;
