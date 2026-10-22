//! Private encrypted download inventory. Renderer identifiers select existing records; they
//! cannot supply manifests, keys, paths or execution allowances. Export remains a separate
//! explicit native invocation with its own original account/entry/Root/storage authority.
use super::*;
use catcoms_storage::kept::{DownloadId, DownloadStatus, KeepPhase};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UiDownload {
    id: String,
    cid: String,
    intent: String,
    grant: String,
    status: &'static str,
    verified_chunks: usize,
    total_chunks: usize,
}
fn project(download: DownloadStatus) -> UiDownload {
    let state = download.state;
    UiDownload {
        id: hex::encode(download.id.as_bytes()),
        cid: state.plan.cid.to_hex(),
        intent: state.intent.to_string(),
        grant: state.grant.to_string(),
        status: if state.phase == KeepPhase::Deleting {
            "removing"
        } else if state.exhausted {
            "exhausted"
        } else if state.user_paused {
            "paused"
        } else if state.blocked {
            "blocked"
        } else if state.running {
            "receiving"
        } else if download.checked {
            "ready"
        } else {
            "incomplete"
        },
        verified_chunks: state.verified_chunks,
        total_chunks: state.plan.chunks.len(),
    }
}
pub(super) fn id(text: &str) -> Result<DownloadId, String> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("Invalid download ID.".into());
    }
    Ok(DownloadId::from_bytes(
        hex::decode(text)
            .map_err(|_| "Invalid download ID.")?
            .try_into()
            .map_err(|_| "Invalid download ID.")?,
    ))
}
pub(super) fn intent(text: &str) -> Result<u64, String> {
    let value = text
        .parse::<u64>()
        .map_err(|_| "Invalid download revision.")?;
    if value.to_string() != text {
        return Err("Invalid download revision.".into());
    }
    Ok(value)
}

#[tauri::command]
pub(super) async fn get_private_downloads(
    state: State<'_, AppState>,
    server: u64,
) -> Result<Vec<UiDownload>, String> {
    let generation = keep_session::invocation_generation(&state).await?;
    let (actor, instance) = actor_instance_of(&state, server).await?;
    let downloads =
        commit_kept_command(&state, server, instance, generation, actor.downloads()).await?;
    Ok(downloads.into_iter().map(project).collect())
}
#[tauri::command]
pub(super) async fn set_private_download_paused(
    state: State<'_, AppState>,
    server: u64,
    download: String,
    expected_intent: String,
    paused: bool,
) -> Result<(), String> {
    let generation = keep_session::invocation_generation(&state).await?;
    let id = id(&download)?;
    let intent = intent(&expected_intent)?;
    let (actor, instance) = actor_instance_of(&state, server).await?;
    commit_kept_command(
        &state,
        server,
        instance,
        generation,
        actor.set_download_paused(id, intent, paused),
    )
    .await
}
#[tauri::command]
pub(super) async fn retry_private_download(
    state: State<'_, AppState>,
    server: u64,
    download: String,
    expected_grant: String,
    request: String,
) -> Result<String, String> {
    let generation = keep_session::invocation_generation(&state).await?;
    let id = id(&download)?;
    let request = kept_retry_request(&download, &expected_grant, &request)?;
    let (actor, instance) = actor_instance_of(&state, server).await?;
    Ok(commit_kept_command(
        &state,
        server,
        instance,
        generation,
        actor.retry_download(id, request),
    )
    .await?
    .granted
    .to_string())
}
#[tauri::command]
pub(super) async fn cancel_private_download(
    state: State<'_, AppState>,
    server: u64,
    download: String,
) -> Result<(), String> {
    let generation = keep_session::invocation_generation(&state).await?;
    let id = id(&download)?;
    let (actor, instance) = actor_instance_of(&state, server).await?;
    commit_kept_command(
        &state,
        server,
        instance,
        generation,
        actor.cancel_download(id),
    )
    .await
}

/// Capacity follows the entire export, including its queued actor plaintext reply. Acquiring
/// this before plan/manifest work and staging prevents unbounded save invocations from waiting
/// with plaintext buffers. Unsupported acquisition is a visible Busy, never an unbounded queue.
pub(super) fn export_slot() -> Result<Arc<tokio::sync::OwnedSemaphorePermit>, String> {
    static SLOTS: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    SLOTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(4)))
        .clone()
        .try_acquire_owned()
        .map(Arc::new)
        .map_err(|_| "Downloads are busy. Try again shortly.".into())
}
