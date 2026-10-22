//! Session-bound streamed-upload admission. The map owns unpublished chunks until one final
//! commit transfers them to an actual publication worker. Lock drains only the map; it waits
//! for an already-admitted worker through the same native commit mutex.
use super::*;
use tokio::sync::OwnedMutexGuard;

/// Opaque ownership transfer. Callers can inspect the original actor but cannot detach the
/// upload from its commit guard. Consume synchronously in `spawn_publication`, before awaiting.
pub(super) struct AdmittedUpload {
    upload: RegisteredUpload,
    commit: OwnedMutexGuard<()>,
}
impl std::fmt::Debug for AdmittedUpload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedUpload").finish_non_exhaustive()
    }
}
impl AdmittedUpload {
    pub(super) fn origin(&self) -> (ServerActor, u64) {
        (self.upload.actor.clone(), self.upload.instance)
    }
}

fn current(state: &AppState, generation: u64) -> bool {
    !ui_session_closed(state) && state.ui_session_generation.load(Ordering::Acquire) == generation
}

/// Install only into the session/registry incarnation captured before begin's actor awaits.
/// Retired uploads are cleaned on quota refusal too, without holding the shared uploads map.
pub(super) async fn register(
    state: &AppState,
    token: String,
    upload: RegisteredUpload,
    now: u64,
) -> Result<usize, String> {
    let _commit = require_ui_session_generation(state, upload.generation).await?;
    let servers = state.servers.lock().await;
    if servers.get(&upload.server).is_none_or(|entry| {
        entry.instance != upload.instance || !entry.actor.same_instance(&upload.actor)
    }) {
        return Err("the server changed while the upload was starting".into());
    }
    // Registry removal also needs commit. Release its guard before waiting for the map, then
    // repeat the synchronous session gate: a lock request can arrive during that map wait.
    drop(servers);
    let (retired, result) = {
        let mut uploads = state.uploads.lock().await;
        if !current(state, upload.generation) {
            return Err("the UI session changed while the upload was starting".into());
        }
        let retired: Vec<_> = uploads
            .iter()
            .filter(|(key, old)| {
                (key.0 == upload.server && old.upload_id == upload.upload_id)
                    || old.idle_since(now.saturating_sub(UPLOAD_IDLE_TIMEOUT_MS))
            })
            .map(|(key, _)| key.clone())
            .collect();
        let retired: Vec<_> = retired
            .iter()
            .filter_map(|key| uploads.remove(key))
            .collect();
        let staged: u64 = uploads.values().map(|up| up.staged_bytes()).sum();
        let result = if uploads.len() >= MAX_PENDING_UPLOADS {
            Err("too many uploads are already in flight".into())
        } else if staged.saturating_add(upload.chunk_total as u64 * CHUNK_BYTES as u64)
            > MAX_STAGED_UPLOAD_BYTES
        {
            Err("too much upload data is already waiting to be published".into())
        } else {
            uploads.insert((upload.server, token), upload);
            Ok(())
        };
        (retired, result)
    };
    let count = retired.len();
    // Keep commit through actual cleanup: lock must not finish while a removed reservation's
    // chunks are still waiting for their original actor to discard them.
    for old in retired {
        discard_pending_upload(old).await;
    }
    result.map(|()| count)
}

/// The original UI generation is read from native custody, never supplied by the WebView.
/// Acquire commit BEFORE map removal; all refusal cleanup runs outside the map guard.
pub(super) async fn take(state: &AppState, key: &UploadKey) -> Result<AdmittedUpload, String> {
    let (generation, instance, actor) = state
        .uploads
        .lock()
        .await
        .get(key)
        .map(|upload| (upload.generation, upload.instance, upload.actor.clone()))
        .ok_or("no such upload")?;
    // Owned form of require_ui_session_generation: this guard must follow detached actual
    // publication, rather than borrow the invoke future. Preserve that helper's full gates.
    let commit = state.ui_session_commit.clone().lock_owned().await;
    let mut refusal = require_unlocked_session(state).await.err();
    let same_actor = state
        .servers
        .lock()
        .await
        .get(&key.0)
        .is_some_and(|entry| entry.instance == instance && entry.actor.same_instance(&actor));
    let upload = state
        .uploads
        .lock()
        .await
        .remove(key)
        .ok_or("no such upload")?;
    // Repeat after the map await, before the transfer's linearization point. A lock request
    // publishes its atomic generation before waiting for our commit mutex.
    if upload.generation != generation || !current(state, upload.generation) {
        refusal = Some("the UI session changed while the upload was in progress".into());
    } else if !same_actor || upload.instance != instance || !upload.actor.same_instance(&actor) {
        refusal = Some("the server changed while the upload was in progress".into());
    } else if !upload.is_complete() {
        refusal = Some("the upload did not send every chunk".into());
    }
    if let Some(error) = refusal {
        discard_pending_upload(upload).await;
        return Err(error);
    }
    Ok(AdmittedUpload { upload, commit })
}

/// Transfer admission directly into a detached worker. Dropping its JoinHandle does not abort
/// queued actor work or return commit early. Only this worker may discard its chunk references.
/// The second stage runs after commit release: ordinary persistence takes the per-server lock,
/// and leave_server orders that lock BEFORE commit. Reversing those locks can deadlock a leave.
pub(super) fn spawn_publication<T, R, P, PF, S, SF>(
    admitted: AdmittedUpload,
    publish: P,
    settled: S,
) -> tokio::task::JoinHandle<R>
where
    T: Send + 'static,
    R: Send + 'static,
    P: FnOnce(RegisteredUpload) -> PF + Send + 'static,
    PF: Future<Output = Result<T, String>> + Send + 'static,
    S: FnOnce(Result<T, String>) -> SF + Send + 'static,
    SF: Future<Output = R> + Send + 'static,
{
    tokio::spawn(async move {
        let AdmittedUpload { upload, commit } = admitted;
        let outcome = publish(upload).await;
        drop(commit);
        settled(outcome).await
    })
}

#[cfg(test)]
mod tests;
