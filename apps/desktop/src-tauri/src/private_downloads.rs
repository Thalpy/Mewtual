//! Private encrypted download inventory and its journaled export to Downloads. Renderer
//! identifiers select existing records; they cannot supply manifests, keys, paths or execution
//! allowances. Export remains a separate explicit native invocation with its own original
//! account/entry/Root/storage authority.
//!
//! Journaled export (see docs/communication-recovery/EXTERNAL-DOWNLOAD-RESUME.md): every external
//! filesystem effect runs in `spawn_blocking` through the storage crate's `ExportFile`, so a stalled
//! Downloads volume pins only this save's worker. The server actor performs only sealed journal
//! transitions, each fenced by the original invocation, actor/Root generation and storage lease.
//! Publication alone holds the session commit guard, so Pause/Remove/Lock serialize with it.
use super::*;
use catcoms_app::{DownloadExportEdge, DownloadExportTransition, DownloadReady};
use catcoms_storage::kept::{
    flush_destination, remove_partial, DownloadId, DownloadStatus, ExportError, ExportFile,
    ExportHalt, ExportJournal, ExportProbe, ExportProgress, ExportReopen, ExportState, ExportView,
    KeepPhase, Publication, EXPORT_CHECKPOINT_CHUNKS,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UiDownload {
    id: String,
    cid: String,
    intent: String,
    grant: String,
    status: &'static str,
    verified_chunks: usize,
    /// Whole chunks covered by the durable external-save checkpoint (0 without a journal).
    saved_chunks: usize,
    total_chunks: usize,
}

/// Closed renderer status, highest precedence first. Typed storage state decides it; nothing is
/// inferred from a path, and the projection never carries a path, name or file identity.
fn status(download: &DownloadStatus) -> &'static str {
    let state = &download.state;
    let writing = matches!(download.export, ExportView::Writing { .. });
    if state.phase == KeepPhase::Deleting {
        "removing"
    } else if download.export == ExportView::Published {
        "saved"
    } else if let ExportView::Writing {
        halt: Some(halt), ..
    } = download.export
    {
        match halt {
            ExportHalt::Changed => "destination-changed",
            ExportHalt::Unavailable => "destination-unavailable",
        }
    } else if state.exhausted {
        "exhausted"
    } else if state.user_paused {
        if writing {
            "save-paused"
        } else {
            "paused"
        }
    } else if state.blocked {
        "blocked"
    } else if state.running {
        "receiving"
    } else if matches!(download.export, ExportView::Writing { active: true, .. }) {
        "saving"
    } else if writing {
        "save-interrupted"
    } else if download.checked {
        "ready"
    } else {
        "incomplete"
    }
}
fn project(download: DownloadStatus) -> UiDownload {
    let status = status(&download);
    let total_chunks = download.state.plan.chunks.len();
    let saved_chunks = match download.export {
        ExportView::Writing { saved_chunks, .. } => saved_chunks.min(total_chunks),
        ExportView::Published => total_chunks,
        ExportView::None => 0,
    };
    let state = download.state;
    UiDownload {
        id: hex::encode(download.id.as_bytes()),
        cid: state.plan.cid.to_hex(),
        intent: state.intent.to_string(),
        grant: state.grant.to_string(),
        status,
        verified_chunks: state.verified_chunks,
        saved_chunks,
        total_chunks,
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
/// Remove (or Clear a saved row). With an export journal this is two-step: the actor durably
/// publishes Deleting, native removes only the partial file proven to be ours (never the final
/// name), then the actor releases the record. `keepPartial` releases without deleting the file,
/// the escape when removal keeps failing.
#[tauri::command]
pub(super) async fn cancel_private_download(
    state: State<'_, AppState>,
    server: u64,
    download: String,
    keep_partial: Option<bool>,
) -> Result<(), String> {
    let generation = keep_session::invocation_generation(&state).await?;
    let id = id(&download)?;
    let (actor, instance) = actor_instance_of(&state, server).await?;
    remove_download(
        &state,
        server,
        instance,
        generation,
        &actor,
        id,
        keep_partial.unwrap_or(false),
    )
    .await
}

/// Shared Remove/Clear sequence. A removal whose proven partial cannot be deleted stays in the
/// visible Removing state, conservatively charged; it never faults the store.
pub(super) async fn remove_download(
    state: &AppState,
    server: u64,
    instance: u64,
    generation: u64,
    actor: &ServerActor,
    id: DownloadId,
    keep_partial: bool,
) -> Result<(), String> {
    let pending = commit_kept_command(
        state,
        server,
        instance,
        generation,
        actor.cancel_download(id),
    )
    .await?;
    if let Some(saved) = pending {
        if !keep_partial {
            let journal = saved.journal().clone();
            blocking(move || remove_partial(&journal))
                .await?
                .map_err(|_| REMOVE_RETRY.to_string())?;
        }
        commit_kept_command(
            state,
            server,
            instance,
            generation,
            actor.finish_download_cleanup(id),
        )
        .await?;
    }
    Ok(())
}
const REMOVE_RETRY: &str = "The partly saved file could not be removed. Close any app using it, then choose Remove again, or choose Keep partly saved file.";

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

/// Run one external filesystem step off the async workers and the server actor.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| "The save worker stopped unexpectedly.".to_string())
}

/// Transient local destination failures (a lock held by another program, a flaky network share)
/// have their own in-session bound, separate from the network execution allowance: at most
/// three retries per chunk and eight per export session, with growing backoff. Exports never
/// start without an explicit invocation, so any later retry is a user action.
pub(super) const LOCAL_RETRIES_PER_CHUNK: u32 = 3;
pub(super) const LOCAL_RETRIES_PER_SESSION: u32 = 8;
pub(super) fn local_retry_delay(chunk_attempts: u32, session_retries: u32) -> Option<Duration> {
    (chunk_attempts < LOCAL_RETRIES_PER_CHUNK && session_retries < LOCAL_RETRIES_PER_SESSION)
        .then(|| Duration::from_millis(250u64 << chunk_attempts.min(4)))
}

const ALREADY_SAVED: &str =
    "This file was already saved. Clear it or save another copy from Transfers.";

/// Result of checking a saved journal before an explicit Resume spends allowance re-verifying
/// the encrypted copy.
pub(super) enum Preflight {
    Continue,
    /// The final name already holds our file (crash after the link): recorded as saved.
    Saved(PathBuf),
}

/// Destination preflight for Resume. Records a halt or a recovered publication by exact journal
/// CAS, refused by the actor once the original invocation is cancelled.
pub(super) async fn resume_preflight(
    actor: &ServerActor,
    id: DownloadId,
    origin: &RequestCancellation,
) -> Result<Preflight, String> {
    let journal = match actor.download_export_state(id).await? {
        None => return Ok(Preflight::Continue),
        Some(ExportState::Published(_)) => return Err(ALREADY_SAVED.into()),
        Some(ExportState::Writing(journal)) => journal,
    };
    let probe = journal.clone();
    match blocking(move || ExportFile::probe(&probe)).await? {
        ExportProbe::Ready => Ok(Preflight::Continue),
        ExportProbe::Published(path) => {
            actor
                .download_export_edge(
                    id,
                    journal,
                    DownloadExportEdge::RecoverPublished,
                    origin.clone(),
                )
                .await?;
            Ok(Preflight::Saved(path))
        }
        ExportProbe::Halt(halt) => Err(record_halt(actor, id, journal, halt, origin).await),
        ExportProbe::Local(reason) => Err(format!("Saving to Downloads failed: {reason}")),
    }
}

/// "Save new copy" / "Save another copy": remove our proven partial (never the final name), then
/// forget the journal under the explicit intent CAS. Returns the intent to use next.
pub(super) async fn abandon_partial(
    state: &AppState,
    server: u64,
    instance: u64,
    generation: u64,
    actor: &ServerActor,
    id: DownloadId,
    expected_intent: u64,
) -> Result<u64, String> {
    let Some(saved) = actor.download_export_state(id).await? else {
        return Ok(expected_intent);
    };
    let journal = saved.journal().clone();
    blocking(move || remove_partial(&journal))
        .await?
        .map_err(|_| {
            "The partly saved file could not be removed. Close any app using it and try again."
                .to_string()
        })?;
    commit_kept_command(
        state,
        server,
        instance,
        generation,
        actor.abandon_download_export(id, expected_intent),
    )
    .await
}

/// Everything an explicit Resume does before acquisition: optional "Save new copy", the
/// destination preflight, then the intent CAS that clears Pause and any halt. Returns a finished
/// result when the preflight found the file already saved (crash after the link); that record is
/// then cleared, removing only our staging name.
#[allow(clippy::too_many_arguments)]
pub(super) async fn prepare_resume(
    state: &AppState,
    server: u64,
    instance: u64,
    generation: u64,
    actor: &ServerActor,
    origin: &RequestCancellation,
    id: DownloadId,
    expected_intent: u64,
    fresh_copy: bool,
) -> Result<Option<SavedFileResult>, String> {
    let intent = if fresh_copy {
        abandon_partial(
            state,
            server,
            instance,
            generation,
            actor,
            id,
            expected_intent,
        )
        .await?
    } else {
        if let Preflight::Saved(path) = resume_preflight(actor, id, origin).await? {
            let saved = finish_saved_read_if(
                state,
                generation,
                origin,
                &path,
                "application/octet-stream",
                exported_media_validation,
                reveal_path,
                || !origin.is_cancelled(),
            )
            .await?;
            let _ = remove_download(state, server, instance, generation, actor, id, false).await;
            return Ok(Some(saved));
        }
        expected_intent
    };
    commit_kept_command(
        state,
        server,
        instance,
        generation,
        actor.set_download_paused(id, intent, false),
    )
    .await?;
    Ok(None)
}

async fn record_halt(
    actor: &ServerActor,
    id: DownloadId,
    journal: ExportJournal,
    halt: ExportHalt,
    origin: &RequestCancellation,
) -> String {
    if journal.halt() != Some(halt) {
        // Best effort: a refused CAS means something newer already changed the record.
        let _ = actor
            .download_export_edge(id, journal, DownloadExportEdge::Halt(halt), origin.clone())
            .await;
    }
    ExportError::Halted(halt).to_string()
}

/// Native context of one save invocation.
pub(super) struct ExportContext<'a> {
    pub(super) state: &'a AppState,
    pub(super) app: &'a AppHandle,
    pub(super) actor: &'a ServerActor,
    pub(super) server: u64,
    pub(super) instance: u64,
    pub(super) generation: u64,
    pub(super) origin: &'a RequestCancellation,
    pub(super) cid: &'a str,
    pub(super) operation: Option<&'a str>,
}
impl ExportContext<'_> {
    fn progress(&self, ready: &DownloadReady, progress: ExportProgress) {
        if !ready.is_current() {
            return;
        }
        keep_session::publish_progress(self.origin, || {
            let _ = self.app.emit(
                "download-progress",
                DownloadProgressEvt {
                    server: self.server,
                    cid: self.cid.to_owned(),
                    cancellation: self.operation.map(str::to_owned),
                    done: progress.done,
                    total: progress.total,
                    bytes_done: progress.bytes,
                    bytes_total: ready.total_size(),
                    network_bytes_done: 0,
                    provider: None,
                },
            );
        });
    }
    async fn transition(
        &self,
        ready: &DownloadReady,
        transition: DownloadExportTransition,
    ) -> Result<(), String> {
        self.actor
            .download_export_transition(ready.clone(), transition, self.origin.clone())
            .await
    }
    async fn checkpoint(
        &self,
        ready: &DownloadReady,
        mut file: ExportFile,
    ) -> Result<ExportFile, String> {
        let (mut file, journal) = blocking(move || {
            let journal = file.checkpoint();
            (file, journal)
        })
        .await?;
        let journal = journal.map_err(|error| error.to_string())?;
        self.transition(
            ready,
            DownloadExportTransition::Checkpoint {
                expected: file.journal().clone(),
                next: journal.clone(),
            },
        )
        .await?;
        file.checkpointed(journal);
        Ok(file)
    }
    /// End a session on an error. A halt is persisted; disk full writes nothing (the vault may
    /// share that volume); any other failure checkpoints what is durable if it still can.
    async fn stop(&self, ready: &DownloadReady, file: ExportFile, error: ExportError) -> String {
        match error {
            ExportError::Halted(halt) => {
                record_halt(
                    self.actor,
                    ready.id(),
                    file.journal().clone(),
                    halt,
                    self.origin,
                )
                .await
            }
            ExportError::Full | ExportError::Inactive => error.to_string(),
            other => {
                let _ = self.checkpoint(ready, file).await;
                other.to_string()
            }
        }
    }
}

/// Journaled export of a verified private copy. `Ok(None)` means a fresh save cannot be journaled
/// on this destination and the caller keeps today's non-resumable writer; a save that already has
/// a journal never falls back (storage also refuses to release a Writing journal that way).
pub(super) async fn export_journaled(
    ctx: &ExportContext<'_>,
    ready: &DownloadReady,
    downloads: PathBuf,
    name: &str,
) -> Result<Option<PathBuf>, String> {
    let plan = ready.export_plan()?;
    let id = ready.id();
    let base = safe_download_name(name);
    let mut file = match ctx.actor.download_export_state(id).await? {
        None => {
            let (folder, name, plan) = (downloads, base.clone(), plan.clone());
            let file = match blocking(move || ExportFile::create(&folder, &name, plan)).await? {
                Ok(file) => file,
                Err(ExportError::Unsupported) => return Ok(None),
                Err(error) => return Err(error.to_string()),
            };
            let begun = ctx
                .transition(
                    ready,
                    DownloadExportTransition::Begin {
                        journal: file.journal().clone(),
                        presence: file.presence(),
                    },
                )
                .await;
            if let Err(error) = begun {
                let _ = blocking(move || file.discard()).await;
                return Err(error);
            }
            file
        }
        Some(ExportState::Writing(journal)) => {
            let (saved, name, plan) = (journal.clone(), base.clone(), plan.clone());
            match blocking(move || ExportFile::reopen(&saved, &name, plan))
                .await?
                .map_err(|error| error.to_string())?
            {
                ExportReopen::Ready(file) => {
                    ctx.transition(
                        ready,
                        DownloadExportTransition::Resume {
                            journal,
                            presence: file.presence(),
                        },
                    )
                    .await?;
                    *file
                }
                ExportReopen::Published(path) => {
                    ctx.actor
                        .download_export_edge(
                            id,
                            journal.clone(),
                            DownloadExportEdge::RecoverPublished,
                            ctx.origin.clone(),
                        )
                        .await?;
                    // The caller releases the record next; drop our second (staging) name first.
                    let _ = blocking(move || remove_partial(&journal)).await;
                    return Ok(Some(path));
                }
                ExportReopen::Halt(halt) => {
                    return Err(record_halt(ctx.actor, id, journal, halt, ctx.origin).await)
                }
            }
        }
        Some(ExportState::Published(_)) => return Err(ALREADY_SAVED.into()),
    };
    ctx.progress(ready, file.progress());
    let mut session_retries = 0;
    while let Some(index) = file.next_index() {
        let mut chunk = ctx
            .actor
            .read_download_chunk(ready.clone(), index, ctx.origin.clone())
            .await?
            .into_bytes();
        let mut attempts = 0;
        let progress = loop {
            let (returned, kept, outcome) = blocking(move || {
                let outcome = file.accept(index, &chunk);
                (file, chunk, outcome)
            })
            .await?;
            file = returned;
            chunk = kept;
            match outcome {
                Ok(progress) => break progress,
                Err(ExportError::Local {
                    retryable: true,
                    reason,
                }) => match local_retry_delay(attempts, session_retries) {
                    Some(delay) => {
                        attempts += 1;
                        session_retries += 1;
                        SystemClock.sleep(delay).await;
                    }
                    None => {
                        let error = ExportError::Local {
                            retryable: false,
                            reason,
                        };
                        return Err(ctx.stop(ready, file, error).await);
                    }
                },
                Err(error) => return Err(ctx.stop(ready, file, error).await),
            }
        };
        ctx.progress(ready, progress);
        if progress.done % EXPORT_CHECKPOINT_CHUNKS == 0 && file.next_index().is_some() {
            file = ctx.checkpoint(ready, file).await?;
        }
    }
    let mut publication_attempts = 0;
    let path = loop {
        let (returned, prepared) = blocking(move || {
            let prepared = file.prepare_publication();
            (file, prepared)
        })
        .await?;
        file = returned;
        let (journal, already) = match prepared {
            Ok(Publication::Ready(journal)) => (journal, None),
            Ok(Publication::AlreadyPublished(journal, path)) => (journal, Some(path)),
            Err(error) => return Err(ctx.stop(ready, file, error).await),
        };
        // The every-chunk checkpoint, including any new final name, is durable before linking.
        if &journal != file.journal() {
            ctx.transition(
                ready,
                DownloadExportTransition::Checkpoint {
                    expected: file.journal().clone(),
                    next: journal.clone(),
                },
            )
            .await?;
            file.checkpointed(journal);
        }
        let linked = {
            // Guard: the staging recheck, the link, the final-name check and Published. Pause,
            // Remove and Lock take the same guard, so exactly one of them or publication wins.
            let _commit = require_ui_session_generation(ctx.state, ctx.generation).await?;
            let current_entry = ctx
                .state
                .servers
                .lock()
                .await
                .get(&ctx.server)
                .is_some_and(|entry| entry.instance == ctx.instance);
            if ctx.origin.is_cancelled() || !ready.is_current() || !current_entry {
                return Err("The download is no longer active.".into());
            }
            let linked = match already {
                Some(path) => Ok(path),
                None => {
                    let (returned, linked) = blocking(move || {
                        let linked = file.link();
                        (file, linked)
                    })
                    .await?;
                    file = returned;
                    linked
                }
            };
            if linked.is_ok() {
                ctx.transition(
                    ready,
                    DownloadExportTransition::Publish {
                        expected: file.published_journal(),
                    },
                )
                .await?;
            }
            linked
        };
        match linked {
            Ok(path) => break path,
            Err(ExportError::Local {
                retryable: true, ..
            }) if publication_attempts < 2 => publication_attempts += 1,
            Err(error) => return Err(ctx.stop(ready, file, error).await),
        }
    };
    // After the guard: best-effort folder flush and removal of our staging name.
    let journal = file.published_journal();
    let _ = blocking(move || {
        flush_destination(&journal);
        file.finish();
    })
    .await;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_storage::kept::{KeepAllowance, KeepPlan, KeptTransfer};

    fn download(
        phase: KeepPhase,
        export: ExportView,
        edit: impl FnOnce(&mut KeptTransfer),
        checked: bool,
    ) -> DownloadStatus {
        let mut state = KeptTransfer {
            plan: KeepPlan {
                cid: catcoms_storage::Cid::of(b"plaintext"),
                version: [1; 32],
                manifest: Vec::new(),
                chunks: vec![
                    (catcoms_storage::Cid::of(b"a"), 5),
                    (catcoms_storage::Cid::of(b"b"), 5),
                ],
            },
            listing: [0; 32],
            transfer: [2; 32],
            phase,
            verified_chunks: 2,
            running: false,
            user_paused: false,
            blocked: false,
            intent: 3,
            exhausted: false,
            grant: 1,
            remaining: KeepAllowance {
                requests: 1,
                pages: 1,
                bytes: 1,
                slices: 1,
            },
        };
        edit(&mut state);
        DownloadStatus {
            id: DownloadId::from_bytes([2; 32]),
            state,
            checked,
            export,
        }
    }
    fn writing(halt: Option<ExportHalt>, active: bool) -> ExportView {
        ExportView::Writing {
            saved_chunks: 1,
            halt,
            active,
        }
    }

    #[test]
    fn projection_keeps_completed_resumable_and_deleting_states_distinct_and_path_free() {
        let cases = [
            (
                download(KeepPhase::Deleting, ExportView::Published, |_| {}, true),
                "removing",
            ),
            (
                download(KeepPhase::Completed, ExportView::Published, |_| {}, true),
                "saved",
            ),
            (
                download(
                    KeepPhase::Completed,
                    writing(Some(ExportHalt::Changed), false),
                    |_| {},
                    true,
                ),
                "destination-changed",
            ),
            (
                download(
                    KeepPhase::Completed,
                    writing(Some(ExportHalt::Unavailable), false),
                    |s| s.user_paused = true,
                    true,
                ),
                "destination-unavailable",
            ),
            (
                download(
                    KeepPhase::Completed,
                    writing(None, false),
                    |s| s.exhausted = true,
                    false,
                ),
                "exhausted",
            ),
            (
                download(
                    KeepPhase::Completed,
                    writing(None, false),
                    |s| s.user_paused = true,
                    true,
                ),
                "save-paused",
            ),
            (
                download(
                    KeepPhase::Resumable,
                    ExportView::None,
                    |s| s.user_paused = true,
                    false,
                ),
                "paused",
            ),
            (
                download(
                    KeepPhase::Resumable,
                    ExportView::None,
                    |s| s.blocked = true,
                    false,
                ),
                "blocked",
            ),
            (
                download(
                    KeepPhase::Completed,
                    writing(None, false),
                    |s| s.running = true,
                    false,
                ),
                "receiving",
            ),
            (
                download(KeepPhase::Completed, writing(None, true), |_| {}, true),
                "saving",
            ),
            (
                download(KeepPhase::Completed, writing(None, false), |_| {}, false),
                "save-interrupted",
            ),
            (
                download(KeepPhase::Completed, ExportView::None, |_| {}, true),
                "ready",
            ),
            (
                download(KeepPhase::Resumable, ExportView::None, |_| {}, false),
                "incomplete",
            ),
        ];
        for (download, expected) in cases {
            let projected = project(download);
            assert_eq!(projected.status, expected);
            let json = serde_json::to_string(&projected).unwrap();
            assert!(!json.contains("path") && !json.contains("folder"), "{json}");
            if expected == "saved" {
                assert_eq!(projected.saved_chunks, projected.total_chunks);
            }
        }
        let json = serde_json::to_string(&project(download(
            KeepPhase::Completed,
            writing(None, false),
            |_| {},
            true,
        )))
        .unwrap();
        assert!(json.contains("\"savedChunks\":1"), "{json}");
    }

    #[test]
    fn local_write_retries_are_bounded_per_chunk_and_per_session() {
        assert!(local_retry_delay(0, 0).is_some());
        assert!(local_retry_delay(LOCAL_RETRIES_PER_CHUNK - 1, 0).is_some());
        assert!(local_retry_delay(LOCAL_RETRIES_PER_CHUNK, 0).is_none());
        assert!(local_retry_delay(0, LOCAL_RETRIES_PER_SESSION).is_none());
        let mut session = 0;
        let mut chunks = 0;
        // A destination that always fails transiently stops after the session bound.
        'outer: loop {
            let mut attempts = 0;
            while let Some(delay) = local_retry_delay(attempts, session) {
                assert!(delay <= Duration::from_secs(4));
                attempts += 1;
                session += 1;
            }
            chunks += 1;
            if session >= LOCAL_RETRIES_PER_SESSION || chunks > 10 {
                break 'outer;
            }
        }
        assert_eq!(session, LOCAL_RETRIES_PER_SESSION);
    }
}
