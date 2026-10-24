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
) -> Result<Value, String> {
    let channel = channel_id(&channel)?;
    let target = match object {
        Some(object) => StudioTarget::Flipnote {
            channel,
            object: id(&object)?,
        },
        None => StudioTarget::Index { channel },
    };
    read(&state, server, target).await
}

async fn read(state: &AppState, server: u64, target: StudioTarget) -> Result<Value, String> {
    read_with(state, server, target, |_| {}, view).await
}

async fn read_with(
    state: &AppState,
    server: u64,
    target: StudioTarget,
    after_rebuild: impl FnOnce(&InvokeContext),
    convert: impl FnOnce(StudioOverlayInspection) -> Result<Value, String>,
) -> Result<Value, String> {
    let context = InvokeContext::new(state, server, Some(target)).await?;
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

fn view(read: StudioOverlayInspection) -> Result<Value, String> {
    read.inspect(|target, prepared, draft| {
        let channel = u128::from_be_bytes(target.channel()).to_string();
        let object = match target {
            StudioTarget::Index { .. } => None,
            StudioTarget::Flipnote { object, .. } => Some(hex::encode(object)),
        };
        bounded_view(match draft {
            None => json!({"v":1,"kind":"absent","channel":channel,"object":object}),
            Some(draft) => json!({"v":1,"kind":"local-draft","channel":channel,"object":object,
                "basis":hex::encode(draft.basis()),"accepted":draft.accepted(),
                "transferState":if prepared {"prepared"} else {"active"},
                "readOnly":true,"content":projection_content(draft.projection())}),
        })
    })?
}
