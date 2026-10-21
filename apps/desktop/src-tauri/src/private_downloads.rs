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
    flush_destination, partial_presence, remove_partial, ChosenDestination, DownloadId,
    DownloadStatus, ExportError, ExportFile, ExportHalt, ExportJournal, ExportProbe,
    ExportProgress, ExportReopen, ExportState, ExportView, KeepPhase, PartialPresence,
    PartialRemoval, Publication, EXPORT_CHECKPOINT_CHUNKS,
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
///
/// Returns a notice when the removal finished but a file of this save stays on disk (a saved
/// copy, another name for the partial, or a file at its name that cannot be proven ours). The
/// renderer shows it; a removal is never reported as complete while plaintext remains unmentioned.
#[tauri::command]
pub(super) async fn cancel_private_download(
    state: State<'_, AppState>,
    server: u64,
    download: String,
    keep_partial: Option<bool>,
) -> Result<Option<String>, String> {
    let generation = keep_session::invocation_generation(&state).await?;
    let id = id(&download)?;
    let (actor, instance) = actor_instance_of(&state, server).await?;
    let notice = remove_download(
        &state,
        server,
        instance,
        generation,
        &actor,
        id,
        keep_partial.unwrap_or(false),
    )
    .await?;
    Ok(notice.map(str::to_owned))
}

/// Shared Remove/Clear sequence. A removal whose proven partial cannot be deleted stays in the
/// visible Removing state, conservatively charged; it never faults the store. `Ok(Some(_))` is a
/// finished removal that left a file on disk, with the text to tell the user.
pub(super) async fn remove_download(
    state: &AppState,
    server: u64,
    instance: u64,
    generation: u64,
    actor: &ServerActor,
    id: DownloadId,
    keep_partial: bool,
) -> Result<Option<&'static str>, String> {
    let claim = CleanupClaim::acquire(server, id)?;
    let ports = ActorCleanup {
        state,
        server,
        instance,
        generation,
        actor,
        id,
    };
    remove_sequence(&ports, claim, keep_partial).await
}
const REMOVE_RETRY: &str = "The partly saved file could not be removed. Close any app using it, then choose Remove again, or choose Keep partly saved file.";
const REMOVE_UNREACHABLE: &str = "The folder with the partly saved file is unavailable. Reconnect it and choose Remove again, or choose Keep partly saved file.";
const CLEANUP_BUSY: &str = "This file is still being removed. Try again shortly.";
const ALREADY_REMOVED: &str =
    "The partly saved file had already been removed, so there was nothing to keep.";

/// Process-wide ownership of every destructive cleanup (Remove, Save new copy) and every
/// preserve decision ("Keep partly saved file") for one Download, keyed by (server, Download ID).
///
/// Review U-H1: without it, a Remove stalled just before deleting the partial and a later Keep
/// could both act on the same Deleting journal; Keep reported the file preserved and released the
/// encrypted copy, then the stalled Remove deleted the only remaining copy. A second claimant now
/// gets Busy instead of a promise. The claim is shared with the blocking worker that deletes, so
/// it outlives a dropped invocation (Lock, a closed window) until the delete has actually ended.
#[derive(Clone)]
pub(super) struct CleanupClaim {
    /// Released when the last clone (the invocation's or its blocking worker's) drops.
    _held: Arc<ClaimKey>,
}
struct ClaimKey(u64, [u8; 32]);
impl Drop for ClaimKey {
    fn drop(&mut self) {
        claimed()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(self.0, self.1));
    }
}
/// Claimed (server, Download ID) pairs.
type Claimed = StdMutex<std::collections::HashSet<(u64, [u8; 32])>>;
fn claimed() -> &'static Claimed {
    static CLAIMED: std::sync::OnceLock<Claimed> = std::sync::OnceLock::new();
    CLAIMED.get_or_init(Default::default)
}
impl CleanupClaim {
    pub(super) fn acquire(server: u64, id: DownloadId) -> Result<Self, String> {
        let key = (server, *id.as_bytes());
        if !claimed()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key)
        {
            return Err(CLEANUP_BUSY.into());
        }
        Ok(Self {
            _held: Arc::new(ClaimKey(key.0, key.1)),
        })
    }
}

/// The effects a cleanup sequence needs, behind a seam so the real sequences run in tests with
/// barriers between awaits. Production is [`ActorCleanup`]: guarded actor commands and blocking
/// filesystem workers.
#[async_trait::async_trait]
pub(super) trait CleanupPorts: Send + Sync {
    /// Durably publish Deleting (or release at once without a journal); `Some` is the journal
    /// native must clean up before [`CleanupPorts::finish_cleanup`].
    async fn cancel(&self) -> Result<Option<ExportState>, String>;
    async fn export_state(&self) -> Result<Option<ExportState>, String>;
    /// The blocking work that deletes the proven partial. [`delete_owned`] runs it on a worker
    /// that also owns the claim, so the claim lasts exactly as long as the delete can happen.
    fn remover(&self, journal: ExportJournal) -> Result<Remover, String>;
    /// Read-only: is anything of this save still on disk?
    async fn presence(&self, journal: ExportJournal) -> Result<PartialPresence, String>;
    async fn finish_cleanup(&self) -> Result<(), String>;
    /// Save new copy, step one: retire every observer under the intent CAS, keeping the journal.
    async fn retire(&self, expected_intent: u64) -> Result<u64, String>;
    /// Save new copy, step two: forget the journal under the intent CAS.
    async fn abandon(&self, expected_intent: u64) -> Result<u64, String>;
}

/// Production cleanup effects for one Download of one server instance and UI generation.
pub(super) struct ActorCleanup<'a> {
    pub(super) state: &'a AppState,
    pub(super) server: u64,
    pub(super) instance: u64,
    pub(super) generation: u64,
    pub(super) actor: &'a ServerActor,
    pub(super) id: DownloadId,
}
#[async_trait::async_trait]
impl CleanupPorts for ActorCleanup<'_> {
    async fn cancel(&self) -> Result<Option<ExportState>, String> {
        let (state, server, instance, generation) =
            (self.state, self.server, self.instance, self.generation);
        commit_kept_command(
            state,
            server,
            instance,
            generation,
            self.actor.cancel_download(self.id),
        )
        .await
    }
    async fn export_state(&self) -> Result<Option<ExportState>, String> {
        self.actor.download_export_state(self.id).await
    }
    fn remover(&self, journal: ExportJournal) -> Result<Remover, String> {
        // Review L9: a bounded number of cleanup workers, so a compromised renderer cannot pin
        // unlimited blocking threads on a stalled volume. The permit moves into the worker.
        let slot = cleanup_slot()?;
        Ok(Box::new(move || {
            let _slot = slot;
            remove_partial(&journal)
        }))
    }
    async fn presence(&self, journal: ExportJournal) -> Result<PartialPresence, String> {
        let slot = cleanup_slot()?;
        blocking(move || {
            let _held = slot;
            partial_presence(&journal)
        })
        .await
    }
    async fn finish_cleanup(&self) -> Result<(), String> {
        let (state, server, instance, generation) =
            (self.state, self.server, self.instance, self.generation);
        commit_kept_command(
            state,
            server,
            instance,
            generation,
            self.actor.finish_download_cleanup(self.id),
        )
        .await
    }
    async fn retire(&self, expected_intent: u64) -> Result<u64, String> {
        let (state, server, instance, generation) =
            (self.state, self.server, self.instance, self.generation);
        commit_kept_command(
            state,
            server,
            instance,
            generation,
            self.actor.retire_download_export(self.id, expected_intent),
        )
        .await
        .map_err(|_| STALE_DOWNLOAD.to_string())
    }
    async fn abandon(&self, expected_intent: u64) -> Result<u64, String> {
        let (state, server, instance, generation) =
            (self.state, self.server, self.instance, self.generation);
        commit_kept_command(
            state,
            server,
            instance,
            generation,
            self.actor.abandon_download_export(self.id, expected_intent),
        )
        .await
    }
}
const STALE_DOWNLOAD: &str = "This download changed. Open Transfers and try again.";

/// Small dedicated bound on blocking cleanup workers, separate from the export slots so a Remove
/// can still run while saves are busy.
fn cleanup_slot() -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    static SLOTS: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    SLOTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(4)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| CLEANUP_BUSY.into())
}

/// Blocking delete work for one proven partial; see [`CleanupPorts::remover`].
pub(super) type Remover = Box<dyn FnOnce() -> Result<PartialRemoval, ExportError> + Send>;

/// Run a delete on a blocking worker that owns a clone of `claim`. If the awaiting invocation is
/// dropped (Lock, a closed window), the worker keeps the claim until the delete has finished, so
/// no preserve decision can be made while the file may still disappear.
async fn delete_owned(
    ports: &impl CleanupPorts,
    journal: ExportJournal,
    claim: &CleanupClaim,
) -> Result<Result<PartialRemoval, ExportError>, String> {
    let work = ports.remover(journal)?;
    let claim = claim.clone();
    blocking(move || {
        let _claim = claim;
        work()
    })
    .await
}

/// Remove/Clear under a held [`CleanupClaim`]. With `keep_partial`, the record is released
/// without deleting the file, after reassessing (under the claim) whether the file still exists:
/// it never reports a file kept that an earlier removal already deleted.
pub(super) async fn remove_sequence(
    ports: &impl CleanupPorts,
    claim: CleanupClaim,
    keep_partial: bool,
) -> Result<Option<&'static str>, String> {
    let Some(saved) = ports.cancel().await? else {
        return Ok(None);
    };
    let published = matches!(saved, ExportState::Published(_));
    let journal = saved.journal().clone();
    let notice = if keep_partial {
        match ports.presence(journal).await? {
            PartialPresence::Gone => Some(ALREADY_REMOVED),
            PartialPresence::Present | PartialPresence::Unknown => None,
        }
    } else {
        // Only an outcome that settles the question may forget the journal. A partial that cannot
        // be checked right now stays tracked in Removing.
        match delete_owned(ports, journal, &claim).await? {
            Ok(outcome) if outcome.settled() => outcome.notice(published),
            Ok(PartialRemoval::FolderUnreachable) => return Err(REMOVE_UNREACHABLE.into()),
            Ok(_) | Err(_) => return Err(REMOVE_RETRY.into()),
        }
    };
    ports.finish_cleanup().await?;
    drop(claim);
    Ok(notice)
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
const JOURNAL_EXISTS: &str =
    "This file already has a partly saved copy. Choose Save new copy or Remove in Transfers first.";
/// Returned when the user closes the "Save as" dialog without choosing. Nothing was started or
/// recorded; the renderer treats exactly this text as a quiet cancel, not a failure.
pub(super) const SAVE_AS_CANCELLED: &str = "Save as was cancelled.";
/// Appended when a save uses the non-resumable writer (no stable identity or hard links there).
pub(super) const NOT_RESUMABLE_NOTE: &str =
    "This location cannot keep a partly saved file, so an interrupted save there starts over.";

/// Where one save writes. A chosen destination comes only from the native dialog through
/// [`choose_destination`]; nothing from the renderer can construct one.
pub(super) enum Destination {
    /// The one-click default: the OS Downloads folder, resolved for this invocation.
    Downloads(PathBuf),
    /// A folder and name the user picked in the native "Save as" dialog, for this save only.
    Chosen(Arc<ChosenDestination>),
}

/// The native "Save as" dialog behind a seam, so tests never open a real window. Returns the
/// path the user picked, or `None` when they cancelled.
#[async_trait::async_trait]
pub(super) trait DestinationChooser: Send + Sync {
    async fn choose(
        &self,
        directory: PathBuf,
        suggested_name: String,
    ) -> Result<Option<PathBuf>, String>;
}

/// Production chooser: `rfd`'s OS dialog, configured like tauri-plugin-dialog 2.7. It is built on
/// the main thread (GTK and AppKit require that) through Tauri core's `run_on_main_thread`, then
/// awaited on a dedicated thread; the async command only awaits a oneshot, so neither the async
/// runtime nor the main thread blocks while the dialog is open.
pub(super) struct NativeChooser(AppHandle);
impl NativeChooser {
    pub(super) fn new(app: AppHandle) -> Self {
        Self(app)
    }
}
#[async_trait::async_trait]
impl DestinationChooser for NativeChooser {
    async fn choose(
        &self,
        directory: PathBuf,
        suggested_name: String,
    ) -> Result<Option<PathBuf>, String> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let window = self.0.get_webview_window("main");
        self.0
            .run_on_main_thread(move || {
                let mut dialog = rfd::AsyncFileDialog::new()
                    .set_title("Save as")
                    .set_directory(&directory)
                    .set_file_name(&suggested_name);
                if let Some(window) = &window {
                    dialog = dialog.set_parent(window);
                }
                let pending = dialog.save_file();
                std::thread::spawn(move || {
                    let picked = tauri::async_runtime::block_on(pending);
                    let _ = sender.send(picked.map(|handle| handle.path().to_path_buf()));
                });
            })
            .map_err(|_| "The Save as window could not be opened.".to_string())?;
        receiver
            .await
            .map_err(|_| "The Save as window closed unexpectedly.".to_string())
    }
}

/// Ask the user where to save, starting in Downloads with the sanitized file name. `Ok(None)`
/// means cancelled. Whatever the dialog returns is validated before use: absolute path, usable
/// name, an existing parent directory whose identity is captured, and no existing file or
/// partial at that name (an existing file is refused and left untouched even if the dialog
/// already asked to replace it).
pub(super) async fn choose_destination(
    chooser: &dyn DestinationChooser,
    downloads: PathBuf,
    name: &str,
) -> Result<Option<ChosenDestination>, String> {
    let Some(picked) = chooser.choose(downloads, safe_download_name(name)).await? else {
        return Ok(None);
    };
    blocking(move || ChosenDestination::validate(&picked))
        .await?
        .map(Some)
        .map_err(|refusal| refusal.to_string())
}

/// One chunk through `step` with the bounded local retry policy. `live` (original invocation and
/// export lease) is checked before EVERY attempt, including after each backoff, so a session
/// retired by Lock, Pause, Remove or a replacement never writes plaintext again. `step` moves the
/// subject into a blocking worker and back; `Err` means that worker died.
pub(super) async fn accept_with_retries<T, Step, StepFut, Sleep, SleepFut>(
    mut subject: T,
    mut step: Step,
    live: impl Fn() -> bool,
    sleep: Sleep,
    session_retries: &mut u32,
) -> Result<(T, Result<ExportProgress, ExportError>), String>
where
    Step: FnMut(T) -> StepFut,
    StepFut: Future<Output = Result<(T, Result<ExportProgress, ExportError>), String>>,
    Sleep: Fn(Duration) -> SleepFut,
    SleepFut: Future<Output = ()>,
{
    let mut attempts = 0;
    loop {
        if !live() {
            return Ok((subject, Err(ExportError::Inactive)));
        }
        let (returned, outcome) = step(subject).await?;
        subject = returned;
        match outcome {
            Err(ExportError::Local {
                retryable: true,
                reason,
            }) => match local_retry_delay(attempts, *session_retries) {
                Some(delay) => {
                    attempts += 1;
                    *session_retries += 1;
                    sleep(delay).await;
                }
                None => {
                    let error = ExportError::Local {
                        retryable: false,
                        reason,
                    };
                    return Ok((subject, Err(error)));
                }
            },
            other => return Ok((subject, other)),
        }
    }
}

/// Result of checking a saved journal before an explicit Resume spends allowance re-verifying
/// the encrypted copy.
pub(super) enum Preflight {
    Continue,
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
        // The final name holds our identity, but its content is unproven (review U-M2) and this
        // preflight has no plan to verify it against. Continue: after re-verification the reopen
        // verifies it read-only against the manifest and only then records Published.
        ExportProbe::Published(_candidate) => Ok(Preflight::Continue),
        ExportProbe::Halt(halt) => Err(record_halt(actor, id, journal, halt, origin).await),
        ExportProbe::Local(reason) => Err(format!("Saving to Downloads failed: {reason}")),
    }
}

/// Shown when Save new copy goes ahead although the old partial could not be reached.
const OLD_PARTIAL_UNREACHABLE: &str =
    "The folder with the old partly saved file is unavailable, so that file was left in place.";

/// "Save new copy" / "Save another copy": remove our proven partial (never the final name), then
/// forget the journal. Returns the intent to use next and, when a file of the old save stays on
/// disk, the notice to attach to the new save's result.
pub(super) async fn abandon_partial(
    state: &AppState,
    server: u64,
    instance: u64,
    generation: u64,
    actor: &ServerActor,
    id: DownloadId,
    expected_intent: u64,
) -> Result<(u64, Option<&'static str>), String> {
    let claim = CleanupClaim::acquire(server, id)?;
    let ports = ActorCleanup {
        state,
        server,
        instance,
        generation,
        actor,
        id,
    };
    abandon_sequence(&ports, claim, expected_intent).await
}

/// Save new copy under a held [`CleanupClaim`] (review U-H1). The intent CAS and the retirement
/// of every observer of the old journal happen in one actor step BEFORE the external delete, so
/// a delayed click, a concurrent Resume or a live writer of the old journal cannot act on it
/// while it is being deleted. The journal stays durable until that delete is settled; only then
/// is it forgotten. A failed delete leaves the journal (with its new intent) naming the partial.
pub(super) async fn abandon_sequence(
    ports: &impl CleanupPorts,
    claim: CleanupClaim,
    expected_intent: u64,
) -> Result<(u64, Option<&'static str>), String> {
    let Some(saved) = ports.export_state().await? else {
        return Ok((expected_intent, None));
    };
    let published = matches!(saved, ExportState::Published(_));
    let retired = ports.retire(expected_intent).await?;
    // Re-read under the new intent: the journal being deleted is the one just retired.
    let Some(saved) = ports.export_state().await? else {
        return Err(STALE_DOWNLOAD.into());
    };
    let journal = saved.journal().clone();
    // A folder that cannot be reached is usually why the user wants a new copy; the old partial
    // then stays wherever it is, and the user is told. A reachable partial that cannot be checked
    // is not forgotten.
    let notice =
        match delete_owned(ports, journal, &claim).await? {
            Ok(PartialRemoval::FolderUnreachable) => Some(OLD_PARTIAL_UNREACHABLE),
            Ok(outcome) if outcome.settled() => outcome.notice(published),
            Ok(_) | Err(_) => return Err(
                "The partly saved file could not be removed. Close any app using it and try again."
                    .into(),
            ),
        };
    let intent = ports.abandon(retired).await?;
    drop(claim);
    Ok((intent, notice))
}

/// Add one sentence to a save result's warning, keeping any warning already there.
pub(super) fn append_warning(saved: &mut SavedFileResult, note: &str) {
    saved.warning = Some(match saved.warning.take() {
        Some(warning) => format!("{warning} {note}"),
        None => note.to_string(),
    });
}

/// What an explicit Resume settled before acquisition.
pub(super) struct Prepared {
    /// The preflight found the file already saved (crash after the link): the save is finished.
    pub(super) saved: Option<SavedFileResult>,
    /// A file of an abandoned earlier save stays on disk; append this to the result.
    pub(super) notice: Option<&'static str>,
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
    chosen: bool,
) -> Result<Prepared, String> {
    // A chosen destination always starts a new file. Continuing an existing partial at its own
    // place is plain Resume; redirecting it requires the explicit Save new copy semantics.
    if chosen && !fresh_copy && actor.download_export_state(id).await?.is_some() {
        return Err(JOURNAL_EXISTS.into());
    }
    let mut notice = None;
    let intent = if fresh_copy {
        let (intent, left) = abandon_partial(
            state,
            server,
            instance,
            generation,
            actor,
            id,
            expected_intent,
        )
        .await?;
        notice = left;
        intent
    } else {
        let Preflight::Continue = resume_preflight(actor, id, origin).await?;
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
    Ok(Prepared {
        saved: None,
        notice,
    })
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
            // Conflict: the partial shrank under this session, so its offset no longer describes
            // durable bytes. Write no checkpoint; the next Resume re-verifies from chunk zero.
            ExportError::Full | ExportError::Inactive | ExportError::Conflict => error.to_string(),
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
    destination: &Destination,
    name: &str,
) -> Result<Option<PathBuf>, String> {
    let plan = ready.export_plan()?;
    let id = ready.id();
    let base = safe_download_name(name);
    let mut file = match ctx.actor.download_export_state(id).await? {
        None => {
            let plan = plan.clone();
            let created = match destination {
                Destination::Downloads(folder) => {
                    let (folder, name) = (folder.clone(), base.clone());
                    blocking(move || ExportFile::create(&folder, &name, plan)).await?
                }
                Destination::Chosen(choice) => {
                    let choice = choice.clone();
                    blocking(move || ExportFile::create_chosen(&choice, plan)).await?
                }
            };
            let file = match created {
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
        Some(ExportState::Writing(_)) if matches!(destination, Destination::Chosen(_)) => {
            // A chosen destination is only ever a new file; it never redirects an existing
            // partial. prepare_resume forgets the old journal first when Save as means a new copy.
            return Err(JOURNAL_EXISTS.into());
        }
        Some(ExportState::Writing(journal)) => {
            let (saved, plan) = (journal.clone(), plan.clone());
            match blocking(move || ExportFile::reopen(&saved, plan))
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
        let chunk = Arc::new(
            ctx.actor
                .read_download_chunk(ready.clone(), index, ctx.origin.clone())
                .await?
                .into_bytes(),
        );
        let (returned, outcome) = accept_with_retries(
            file,
            |mut file: ExportFile| {
                let chunk = chunk.clone();
                async move {
                    blocking(move || {
                        let outcome = file.accept(index, &chunk);
                        (file, outcome)
                    })
                    .await
                }
            },
            || !ctx.origin.is_cancelled() && ready.is_current(),
            |delay| async move { SystemClock.sleep(delay).await },
            &mut session_retries,
        )
        .await?;
        file = returned;
        let progress = match outcome {
            Ok(progress) => progress,
            Err(error) => return Err(ctx.stop(ready, file, error).await),
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
            planned: false,
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

    fn transient() -> ExportError {
        ExportError::Local {
            retryable: true,
            reason: "locked".into(),
        }
    }
    fn done() -> ExportProgress {
        ExportProgress {
            done: 1,
            total: 1,
            bytes: 1,
        }
    }
    /// Scripted step: the subject counts attempts; each attempt pops the next outcome.
    async fn scripted(
        outcomes: Vec<Result<ExportProgress, ExportError>>,
        live: impl Fn(u32) -> bool,
        session: &mut u32,
    ) -> (u32, Result<ExportProgress, ExportError>) {
        let outcomes = std::sync::Mutex::new(outcomes.into_iter());
        let calls = std::sync::atomic::AtomicU32::new(0);
        accept_with_retries(
            0u32,
            |count: u32| {
                calls.fetch_add(1, Ordering::SeqCst);
                let next = outcomes.lock().unwrap().next().expect("scripted outcome");
                async move { Ok((count + 1, next)) }
            },
            || live(calls.load(Ordering::SeqCst)),
            |_| async {},
            session,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn transient_failures_retry_within_the_bound_then_stop_non_retryable() {
        let mut session = 0;
        let (attempts, outcome) = scripted(
            vec![Err(transient()), Err(transient()), Ok(done())],
            |_| true,
            &mut session,
        )
        .await;
        assert_eq!((attempts, session), (3, 2));
        assert_eq!(outcome.unwrap(), done());
        let mut session = 0;
        let (attempts, outcome) = scripted(
            (0..10).map(|_| Err(transient())).collect(),
            |_| true,
            &mut session,
        )
        .await;
        assert_eq!(attempts, LOCAL_RETRIES_PER_CHUNK + 1);
        assert!(matches!(
            outcome,
            Err(ExportError::Local {
                retryable: false,
                ..
            })
        ));
        // Disk full and halts are never retried.
        let mut session = 0;
        let (attempts, outcome) = scripted(
            vec![Err(ExportError::Full), Ok(done())],
            |_| true,
            &mut session,
        )
        .await;
        assert_eq!(attempts, 1);
        assert!(matches!(outcome, Err(ExportError::Full)));
    }

    #[tokio::test]
    async fn no_write_attempt_happens_after_the_invocation_or_lease_retires() {
        // Lock (or Pause/Remove/replacement) lands while the first attempt backs off: the retry
        // must not run, so no plaintext is written by a retired session.
        let mut session = 0;
        let (attempts, outcome) = scripted(
            vec![Err(transient()), Ok(done())],
            |calls| calls == 0,
            &mut session,
        )
        .await;
        assert_eq!(attempts, 1, "only the attempt made while live");
        assert!(matches!(outcome, Err(ExportError::Inactive)));
        let mut session = 0;
        let (attempts, outcome) = scripted(vec![Ok(done())], |_| false, &mut session).await;
        assert_eq!(attempts, 0, "a retired session never starts a write");
        assert!(matches!(outcome, Err(ExportError::Inactive)));
    }
}

#[cfg(test)]
#[path = "private_downloads/cleanup_tests.rs"]
mod cleanup_tests;
