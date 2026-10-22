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
use catcoms_app::{DownloadExportEdge, DownloadExportTransition, DownloadPreflight, DownloadReady};
use catcoms_storage::kept::{
    flush_destination, partial_presence, remove_partial, ChoiceRefusal, ChosenDestination,
    DownloadId, DownloadStatus, ExportError, ExportFile, ExportHalt, ExportJournal, ExportPlan,
    ExportProbe, ExportProgress, ExportReopen, ExportState, ExportView, KeepPhase, PartialPresence,
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
    /// A "Save as" choice is remembered for this row's next save. Never the path or name.
    chosen_place: bool,
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
    let chosen_place = download.planned;
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
        chosen_place,
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
    /// Read-only: is anything of this save still on disk? Best effort: production answers
    /// `Unknown` at once when no cleanup slot is free, and [`remove_sequence`] stops waiting after
    /// [`KEEP_PRESENCE_WAIT`] (re-review 2 MEDIUM).
    async fn presence(&self, journal: ExportJournal) -> Result<PartialPresence, String>;
    /// Sanctioned-clock wait bounding how long Keep waits for [`CleanupPorts::presence`].
    async fn wait(&self, delay: Duration);
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
        bounded_presence(journal).await
    }
    async fn wait(&self, delay: Duration) {
        SystemClock.sleep(delay).await;
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

/// How long "Keep partly saved file" waits for its read-only presence check before releasing the
/// record anyway with no promise about the file (re-review 2 MEDIUM).
pub(super) const KEEP_PRESENCE_WAIT: Duration = Duration::from_secs(3);

/// Production presence check for Keep. It takes a cleanup slot like every blocking cleanup worker
/// (review L9), but when all slots are busy (for example four Removes stuck on a dead share, of
/// any downloads) it answers `Unknown` without touching the filesystem instead of Busy: Keep is
/// the escape from a stuck removal, `Unknown` makes no promise, and the held claim alone already
/// excludes a concurrent delete of this Download.
pub(super) async fn bounded_presence(journal: ExportJournal) -> Result<PartialPresence, String> {
    let Ok(slot) = cleanup_slot() else {
        return Ok(PartialPresence::Unknown);
    };
    blocking(move || {
        let _held = slot;
        partial_presence(&journal)
    })
    .await
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
        // Re-review 2 MEDIUM: the check only decides whether to say "already removed". A hung
        // share (folder identity can block until the OS gives up, forever on a hard NFS mount), a
        // busy slot or a failed check all mean "no promise", and Keep still releases the record.
        // The worker of a check that times out keeps running read-only; it cannot delete.
        let checked = tokio::select! {
            biased;
            presence = ports.presence(journal) => presence.ok(),
            () = ports.wait(KEEP_PRESENCE_WAIT) => None,
        };
        match checked {
            Some(PartialPresence::Gone) => Some(ALREADY_REMOVED),
            Some(PartialPresence::Present | PartialPresence::Unknown) | None => None,
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
        .map_err(|_| "Too many files are being saved. Try again shortly.".into())
}

/// Run one external filesystem step off the async workers and the server actor.
///
/// Shared with the non-resumable fallback writer in `lib.rs` (`stream_download_staged`), whose
/// staging, writes, sync, cleanup and publication must stay off the runtime for the same reason:
/// a stalled share pins one blocking-pool thread, never a tokio worker that chat and sync need.
pub(super) async fn blocking<T: Send + 'static>(
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
/// A Save as invocation found its choice already consumed by another save of the same file.
pub(super) const PLACE_CHANGED: &str =
    "This file is already being saved somewhere else. Open Transfers to see it.";
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

/// Whether a finished group-file save sits directly in the resolved Downloads folder, for the
/// result's `inDownloads` flag (picker re-review LOW 1b and its re-check). Where the file was
/// actually written decides, not which destination was selected: a resumed or recovered journal
/// keeps its own sealed folder, which may be a place chosen earlier, and a Save as into the
/// Downloads folder itself is in Downloads. This drives notice wording only, never authority. It
/// fails closed: no resolvable Downloads folder, or any path-form mismatch (case, links), means
/// "not asserted", so the notice names no folder.
pub(super) fn saved_in_downloads(downloads: Option<&Path>, saved: &Path) -> bool {
    downloads.is_some_and(|folder| saved.parent() == Some(folder))
}

/// Where a fresh save of this Download goes. The record, not the invocation, decides: its
/// remembered choice (this invocation's, an earlier one that survived a restart, or a newer Save
/// as that replaced it), else Downloads. Begin later refuses any journal that does not match it.
///
/// - `planned`: the record's remembered choice as acquisition returned it; `Some(Err(_))` is a
///   choice this platform cannot decode (the vault moved to another operating system), refused
///   rather than redirected to Downloads.
/// - `chose_now`: this invocation opened Save as. If the record no longer remembers any choice,
///   another save already took it into a journal, and this Save as is refused with
///   [`PLACE_CHANGED`], never silently redirected to Downloads.
/// - `downloads`: resolved only when Downloads is actually the destination.
pub(super) fn select_destination(
    planned: Option<Result<Arc<ChosenDestination>, ChoiceRefusal>>,
    chose_now: bool,
    downloads: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<Destination, String> {
    match (planned, chose_now) {
        (Some(Ok(choice)), _) => Ok(Destination::Chosen(choice)),
        (Some(Err(refusal)), _) => Err(refusal.to_string()),
        (None, true) => Err(PLACE_CHANGED.into()),
        (None, false) => downloads().map(Destination::Downloads),
    }
}

/// The native "Save as" dialog behind a seam, so tests never open a real window. Returns the
/// path the user picked, or `None` when they cancelled. `directory` is where the dialog starts;
/// `None` (no Downloads folder could be resolved) lets the OS pick its own default, because Save
/// as is the remedy for a Downloads folder that does not work and must not depend on one.
#[async_trait::async_trait]
pub(super) trait DestinationChooser: Send + Sync {
    async fn choose(
        &self,
        directory: Option<PathBuf>,
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
        directory: Option<PathBuf>,
        suggested_name: String,
    ) -> Result<Option<PathBuf>, String> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let window = self.0.get_webview_window("main");
        self.0
            .run_on_main_thread(move || {
                let mut dialog = rfd::AsyncFileDialog::new()
                    .set_title("Save as")
                    .set_file_name(&suggested_name);
                if let Some(directory) = &directory {
                    dialog = dialog.set_directory(directory);
                }
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
        // A dropped sender (the main-thread closure never ran, or the dialog task ended without
        // a result) means no path was chosen: the same as Cancel, nothing started.
        Ok(receiver.await.unwrap_or(None))
    }
}

/// Process-wide: at most one "Save as" dialog at a time (picker review M4). A second request
/// while one is open is refused instead of stacking unowned dialogs.
static DIALOG_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
struct DialogTurn;
impl DialogTurn {
    fn take() -> Option<Self> {
        DIALOG_OPEN
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_ok()
            .then_some(Self)
    }
}
impl Drop for DialogTurn {
    fn drop(&mut self) {
        DIALOG_OPEN.store(false, std::sync::atomic::Ordering::Release);
    }
}
pub(super) const DIALOG_BUSY: &str = "A Save as window is already open.";
/// The invocation that opened the dialog ended (Lock, unlock, server replacement) first.
pub(super) const DIALOG_ABANDONED: &str = "The download is no longer active.";

/// Ask the user where to save, starting in Downloads (or the OS default when `downloads` is
/// `None`, picker review L6) with the sanitized file name. `Ok(None)` means cancelled. Whatever
/// the dialog returns is validated before use: absolute path, usable name, an existing parent
/// directory whose identity is captured, and no existing file or partial at that name (an
/// existing file is refused and left untouched even if the dialog already asked to replace it).
///
/// The one-dialog turn is an RAII guard held by this future: it is released when the dialog
/// answers, when the chooser fails, and when the awaiting invocation is dropped. A dialog whose
/// answer never comes (for example a GTK dialog future that never completes) would otherwise hold
/// the turn for the life of the process, because Tauri does not drop a running command. So the
/// wait also ends when `cancelled` resolves (production: the invocation's origin, cancelled by
/// Lock, unlock or a server replacement), which releases the turn; a still-open OS window is then
/// orphaned and its answer discarded (re-review 2 residual).
pub(super) async fn choose_destination(
    chooser: &dyn DestinationChooser,
    downloads: Option<PathBuf>,
    name: &str,
    cancelled: impl Future<Output = ()>,
) -> Result<Option<ChosenDestination>, String> {
    let _turn = DialogTurn::take().ok_or(DIALOG_BUSY)?;
    // The suggestion is a peer-chosen name. The Windows file dialog expands `%VAR%` in its name
    // box when the user presses Save, so "%APPDATA%_x" would turn into a path elsewhere that the
    // user never saw or typed. A suggestion never carries `%` (picker review residual).
    let suggested = safe_download_name(name).replace('%', "_");
    let answer = tokio::select! {
        biased;
        answer = chooser.choose(downloads, suggested) => answer?,
        () = cancelled => return Err(DIALOG_ABANDONED.into()),
    };
    let Some(picked) = answer else {
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
    /// The final name already holds our file (crash after the link), its content was verified
    /// read-only against the saved plan, and Published is recorded. The preflight observation
    /// still fences the response (review U-M4).
    Saved(PathBuf, Box<DownloadPreflight>),
}

/// Destination preflight for Resume. Every effect is fenced by one [`DownloadPreflight`]
/// observation (review U-M3): a probe released after freeze/thaw, Lock, remount or a newer
/// Resume records nothing. A final name holding our identity is recorded as Published only after
/// read-only verification against the saved plan (review U-M2).
pub(super) async fn resume_preflight(
    actor: &ServerActor,
    id: DownloadId,
    origin: &RequestCancellation,
    check_plan: bool,
) -> Result<Preflight, String> {
    let preflight = actor.download_preflight(id, origin.clone()).await?;
    let journal = match preflight.state() {
        None => {
            // A remembered choice is the destination; spare a doomed re-verification. Skipped
            // when this invocation's own Save as choice will replace it (`check_plan` false).
            if check_plan {
                plan_preflight(preflight.planned()).await?;
            }
            return Ok(Preflight::Continue);
        }
        Some(ExportState::Published(_)) => return Err(ALREADY_SAVED.into()),
        Some(ExportState::Writing(journal)) => journal.clone(),
    };
    let probe = journal.clone();
    match blocking(move || ExportFile::probe(&probe)).await? {
        ExportProbe::Ready => Ok(Preflight::Continue),
        ExportProbe::Published(candidate) => {
            // Without a decodable saved manifest, leave it to the reopen after re-verification,
            // which verifies against the acquired manifest.
            let Some(plan) = preflight.plan().cloned() else {
                return Ok(Preflight::Continue);
            };
            match blocking(move || candidate.verify(&plan)).await? {
                Ok(path) => {
                    actor
                        .download_export_edge(
                            preflight.clone(),
                            journal,
                            DownloadExportEdge::RecoverPublished,
                            origin.clone(),
                        )
                        .await
                        .map_err(user_message)?;
                    Ok(Preflight::Saved(path, Box::new(preflight)))
                }
                Err(ExportError::Halted(halt)) => {
                    Err(record_preflight_halt(actor, &preflight, journal, halt, origin).await)
                }
                Err(error) => Err(error.to_string()),
            }
        }
        ExportProbe::Halt(halt) => {
            Err(record_preflight_halt(actor, &preflight, journal, halt, origin).await)
        }
        // Review L5: the probe's text is already a complete message; never prefix it again.
        ExportProbe::Local(reason) => Err(reason),
    }
}

/// Re-review 2 LOW 3: check a remembered "Save as" choice read-only, on the blocking pool, before
/// an explicit Resume spends allowance re-verifying the encrypted copy. An unplugged, swapped or
/// occupied chosen place is refused here with the same message creation would give, so repeated
/// "Save to chosen place" clicks cannot exhaust the allowance. No choice: nothing to check.
pub(super) async fn plan_preflight(
    planned: Option<Result<Arc<ChosenDestination>, ChoiceRefusal>>,
) -> Result<(), String> {
    match planned {
        None => Ok(()),
        Some(Err(refusal)) => Err(refusal.to_string()),
        Some(Ok(choice)) => blocking(move || choice.preflight())
            .await?
            .map_err(|refusal| refusal.to_string()),
    }
}

/// Review L5: storage refusals reach the actor boundary as terse internal texts. Show the user
/// what they mean instead.
pub(super) fn user_message(error: String) -> String {
    let malformed = catcoms_storage::StorageError::Malformed.to_string();
    if error == malformed {
        STALE_DOWNLOAD.to_string()
    } else if let Some(reason) = error.strip_prefix("io error: ") {
        reason.to_string()
    } else {
        error
    }
}

/// Finish a save the preflight recovered (crash after the link): inspect and reveal the verified
/// file, clear the record, and return the path only if the original invocation, UI session and
/// the observed actor/Root generation are all still current AFTER that cleanup (review U-M4).
/// A cleanup failure in a live session is reported as a warning on the success, never discarded.
pub(super) async fn finish_recovered_save<A, R, F>(
    state: &AppState,
    generation: u64,
    origin: &RequestCancellation,
    path: &Path,
    allowed: A,
    reveal: R,
    cleanup: F,
) -> Result<SavedFileResult, String>
where
    A: Fn() -> bool,
    R: FnOnce(&Path) -> Result<(), String>,
    F: Future<Output = Result<(), String>>,
{
    let saved = finish_saved_read_if(
        state,
        generation,
        origin,
        path,
        "application/octet-stream",
        inspect_published,
        reveal,
        || !origin.is_cancelled() && allowed(),
    )
    .await?;
    finish_download_response(state, generation, origin, saved, allowed, cleanup).await
}

/// Shown with a successful save whose temporary staging name could not be removed (review L3).
pub(super) const STAGING_LEFT: &str = "A temporary second name for the saved file could not be removed yet. Clear this file in Transfers to remove it.";

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
    // The claim (plain Resume only) is held until the intent CAS below has committed.
    let (intent, _claim) = if fresh_copy {
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
        (intent, None)
    } else {
        // Re-review 2 LOW 2: Resume is excluded by the same claim as Remove, Keep and Save new
        // copy. After Save new copy retires the journal, a Resume carrying the new intent (a
        // second window, or a list refreshed meanwhile) could otherwise reopen and lock the
        // partial its delete worker is about to remove. Held across the preflight and the intent
        // CAS only; the save itself runs without it, and a later cleanup retires its session.
        let claim = CleanupClaim::acquire(server, id)?;
        if let Preflight::Saved(path, preflight) =
            resume_preflight(actor, id, origin, !chosen).await?
        {
            // Clearing the recovered record takes the claim itself.
            drop(claim);
            let cleanup = async {
                remove_download(state, server, instance, generation, actor, id, false)
                    .await
                    .map(|_| ())
            };
            let saved = finish_recovered_save(
                state,
                generation,
                origin,
                &path,
                || preflight.invocation_current(),
                reveal_path,
                cleanup,
            )
            .await?;
            return Ok(Prepared {
                saved: Some(saved),
                notice: None,
            });
        }
        (expected_intent, Some(claim))
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

/// Record a halt the preflight found, fenced by its observation. Best effort: a refusal means
/// something newer (a remount, a newer Resume, Lock) already moved on.
async fn record_preflight_halt(
    actor: &ServerActor,
    preflight: &DownloadPreflight,
    journal: ExportJournal,
    halt: ExportHalt,
    origin: &RequestCancellation,
) -> String {
    if journal.halt() != Some(halt) {
        let _ = actor
            .download_export_edge(
                preflight.clone(),
                journal,
                DownloadExportEdge::Halt(halt),
                origin.clone(),
            )
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
/// What one save's journaled export needs from outside the sequence, behind a seam so the real
/// sequence runs in tests across its awaits with real files. Production is [`ActorExport`].
#[async_trait::async_trait]
pub(super) trait ExportPorts: Send + Sync {
    /// The record's saved export state now.
    async fn export_state(&self) -> Result<Option<ExportState>, String>;
    /// One lease-fenced journal transition on the actor.
    async fn transition(&self, transition: DownloadExportTransition) -> Result<(), String>;
    /// Plaintext of chunk `index`, regenerated from the retained encrypted copy.
    async fn chunk(&self, index: usize) -> Result<Vec<u8>, String>;
    /// The original invocation and the export lease are still current.
    fn live(&self) -> bool;
    /// The same check as an owned value a blocking worker can carry and evaluate when it starts.
    fn liveness(&self) -> Liveness;
    /// Take the session commit guard (Pause, Remove and Lock take it too) and confirm the
    /// invocation, lease and registry entry under it. Held until the returned value drops.
    async fn commit(&self) -> Result<Box<dyn Send + '_>, String>;
    fn progress(&self, progress: ExportProgress);
    /// Sanctioned-clock backoff between bounded local retries.
    async fn backoff(&self, delay: Duration);
}

/// An owned liveness check for blocking workers (see [`ExportPorts::liveness`]).
pub(super) type Liveness = Arc<dyn Fn() -> bool + Send + Sync>;

/// Production export effects for one save invocation and its acquired copy.
pub(super) struct ActorExport<'a> {
    pub(super) ctx: &'a ExportContext<'a>,
    pub(super) ready: &'a DownloadReady,
}
#[async_trait::async_trait]
impl ExportPorts for ActorExport<'_> {
    async fn export_state(&self) -> Result<Option<ExportState>, String> {
        self.ctx.actor.download_export_state(self.ready.id()).await
    }
    async fn transition(&self, transition: DownloadExportTransition) -> Result<(), String> {
        self.ctx
            .actor
            .download_export_transition(self.ready.clone(), transition, self.ctx.origin.clone())
            .await
    }
    async fn chunk(&self, index: usize) -> Result<Vec<u8>, String> {
        Ok(self
            .ctx
            .actor
            .read_download_chunk(self.ready.clone(), index, self.ctx.origin.clone())
            .await?
            .into_bytes())
    }
    fn live(&self) -> bool {
        !self.ctx.origin.is_cancelled() && self.ready.is_current()
    }
    fn liveness(&self) -> Liveness {
        let (origin, ready) = (self.ctx.origin.clone(), self.ready.clone());
        Arc::new(move || !origin.is_cancelled() && ready.is_current())
    }
    async fn commit(&self) -> Result<Box<dyn Send + '_>, String> {
        let commit = require_ui_session_generation(self.ctx.state, self.ctx.generation).await?;
        let current_entry = self
            .ctx
            .state
            .servers
            .lock()
            .await
            .get(&self.ctx.server)
            .is_some_and(|entry| entry.instance == self.ctx.instance);
        if !self.live() || !current_entry {
            return Err("The download is no longer active.".into());
        }
        Ok(Box::new(commit))
    }
    fn progress(&self, progress: ExportProgress) {
        if !self.ready.is_current() {
            return;
        }
        let ctx = self.ctx;
        keep_session::publish_progress(ctx.origin, || {
            let _ = ctx.app.emit(
                "download-progress",
                DownloadProgressEvt {
                    server: ctx.server,
                    cid: ctx.cid.to_owned(),
                    cancellation: ctx.operation.map(str::to_owned),
                    done: progress.done,
                    total: progress.total,
                    bytes_done: progress.bytes,
                    bytes_total: self.ready.total_size(),
                    network_bytes_done: 0,
                    provider: None,
                },
            );
        });
    }
    async fn backoff(&self, delay: Duration) {
        SystemClock.sleep(delay).await;
    }
}

/// Owns a value whose drop does destination I/O (an [`ExportFile`] closes its OS handle and
/// releases its lock, which can stall on a dead share) on the async side of the export sequence.
/// Whatever path drops it there, an early `?` return or an error, closes it on the blocking pool
/// instead of a runtime worker (re-review 2 LOW 7). A value moved into a blocking step leaves
/// with [`OnPool::take`] and comes back with [`OnPool::put`]; while it is inside that step the
/// step owns it, so a dropped invocation also lets it close on the pool.
pub(super) struct OnPool<T: Send + 'static>(Option<T>);
impl<T: Send + 'static> OnPool<T> {
    pub(super) fn new(value: T) -> Self {
        Self(Some(value))
    }
    /// Move the value out for one blocking step. Only called between steps, when it is present.
    pub(super) fn take(&mut self) -> T {
        self.0
            .take()
            .expect("the session value is present between steps")
    }
    pub(super) fn put(&mut self, value: T) {
        self.0 = Some(value);
    }
    /// Hand the value (if a step has not consumed it) to a caller that closes it on the pool
    /// itself, for example together with removing the file it names.
    pub(super) fn release(&mut self) -> Option<T> {
        self.0.take()
    }
    fn get(&self) -> &T {
        self.0
            .as_ref()
            .expect("the session value is present between steps")
    }
    fn get_mut(&mut self) -> &mut T {
        self.0
            .as_mut()
            .expect("the session value is present between steps")
    }
}
impl<T: Send + 'static> Drop for OnPool<T> {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            match tokio::runtime::Handle::try_current() {
                Ok(runtime) => {
                    // Fire and forget: closing needs no answer. If the runtime is shutting down
                    // the closure (and the value) is dropped where it stands.
                    drop(runtime.spawn_blocking(move || drop(value)));
                }
                Err(_) => drop(value),
            }
        }
    }
}

/// Persist this session's durable progress through the checkpoint CAS.
async fn checkpoint(ports: &impl ExportPorts, file: &mut OnPool<ExportFile>) -> Result<(), String> {
    let mut session = file.take();
    let (session, journal) = blocking(move || {
        let journal = session.checkpoint();
        (session, journal)
    })
    .await?;
    file.put(session);
    let journal = journal.map_err(|error| error.to_string())?;
    ports
        .transition(DownloadExportTransition::Checkpoint {
            expected: file.get().journal().clone(),
            next: journal.clone(),
        })
        .await?;
    file.get_mut().checkpointed(journal);
    Ok(())
}

/// Record a halt found during the session, fenced by its export lease. Best effort: a refusal
/// means something newer already changed the record.
async fn record_halt(ports: &impl ExportPorts, journal: ExportJournal, halt: ExportHalt) -> String {
    if journal.halt() != Some(halt) {
        let _ = ports
            .transition(DownloadExportTransition::Halt {
                expected: journal,
                halt,
            })
            .await;
    }
    ExportError::Halted(halt).to_string()
}

/// End a session on an error. A halt is persisted. Only a transient local failure (after its
/// bounded retries) checkpoints what is durable. Review L4: a verification failure (a CID or
/// layout mismatch, `Storage`), a conflict, disk full or an inactive session never advance the
/// checkpoint, so a mismatched stream can never reach the every-chunk journal that recovery
/// would accept.
async fn stop(
    ports: &impl ExportPorts,
    mut file: OnPool<ExportFile>,
    error: ExportError,
) -> String {
    match error {
        ExportError::Halted(halt) => record_halt(ports, file.get().journal().clone(), halt).await,
        ExportError::Local { .. } => {
            let message = error.to_string();
            let _ = checkpoint(ports, &mut file).await;
            message
        }
        other => other.to_string(),
    }
}

/// A completed journaled save.
pub(super) struct Exported {
    pub(super) path: PathBuf,
    /// No staging name of ours can remain. Review L3: when false, the caller keeps the Published
    /// record so the leftover second plaintext name stays visible until Clear removes it.
    pub(super) staging_cleared: bool,
}

/// Journaled export of a verified private copy. `Ok(None)` means a fresh save cannot be journaled
/// on this destination and the caller keeps today's non-resumable writer; a save that already has
/// a journal never falls back (storage also refuses to release a Writing journal that way).
pub(super) async fn export_journaled(
    ctx: &ExportContext<'_>,
    ready: &DownloadReady,
    destination: &Destination,
    name: &str,
) -> Result<Option<Exported>, String> {
    let plan = ready.export_plan()?;
    export_sequence(&ActorExport { ctx, ready }, plan, destination, name).await
}

/// The export sequence itself, over [`ExportPorts`].
pub(super) async fn export_sequence(
    ports: &impl ExportPorts,
    plan: ExportPlan,
    destination: &Destination,
    name: &str,
) -> Result<Option<Exported>, String> {
    let base = safe_download_name(name);
    let mut file = match ports.export_state().await? {
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
            let begun = ports
                .transition(DownloadExportTransition::Begin {
                    journal: file.journal().clone(),
                    presence: file.presence(),
                })
                .await;
            if let Err(error) = begun {
                let _ = blocking(move || file.discard()).await;
                return Err(user_message(error));
            }
            OnPool::new(file)
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
                    let file = OnPool::new(*file);
                    ports
                        .transition(DownloadExportTransition::Resume {
                            journal,
                            presence: file.get().presence(),
                        })
                        .await
                        .map_err(user_message)?;
                    file
                }
                // The reopen verified the content read-only against this manifest (review U-M2).
                ExportReopen::Published(path) => {
                    ports
                        .transition(DownloadExportTransition::RecoverPublished {
                            expected: journal.clone(),
                        })
                        .await
                        .map_err(user_message)?;
                    // The caller releases the record next; drop our second (staging) name first,
                    // and keep the record if that cannot be settled (review L3).
                    let staging_cleared = blocking(move || {
                        matches!(remove_partial(&journal), Ok(outcome) if outcome.settled())
                    })
                    .await?;
                    return Ok(Some(Exported {
                        path,
                        staging_cleared,
                    }));
                }
                ExportReopen::Halt(halt) => return Err(record_halt(ports, journal, halt).await),
            }
        }
        Some(ExportState::Published(_)) => return Err(ALREADY_SAVED.into()),
    };
    ports.progress(file.get().progress());
    let mut session_retries = 0;
    while let Some(index) = file.get().next_index() {
        let chunk = Arc::new(ports.chunk(index).await?);
        // Liveness is checked before each dispatch AND again when the blocking worker actually
        // starts: a step queued behind a busy blocking pool may begin after Lock, Pause or a
        // replacement retired this session, and then it writes nothing. A step that has already
        // started runs to completion (its write is fsynced and only the next checkpoint, which
        // is lease-fenced, could ever describe it).
        let liveness = ports.liveness();
        let (returned, outcome) = accept_with_retries(
            file.take(),
            |mut file: ExportFile| {
                let chunk = chunk.clone();
                let liveness = liveness.clone();
                async move {
                    blocking(move || {
                        if !liveness() {
                            return (file, Err(ExportError::Inactive));
                        }
                        let outcome = file.accept(index, &chunk);
                        (file, outcome)
                    })
                    .await
                }
            },
            || ports.live(),
            |delay| ports.backoff(delay),
            &mut session_retries,
        )
        .await?;
        file.put(returned);
        let progress = match outcome {
            Ok(progress) => progress,
            Err(error) => return Err(stop(ports, file, error).await),
        };
        ports.progress(progress);
        if progress.done % EXPORT_CHECKPOINT_CHUNKS == 0 && file.get().next_index().is_some() {
            checkpoint(ports, &mut file).await?;
        }
    }
    // Review U-L1: publication retries are charged to the same per-session budget as chunk
    // retries and wait on the sanctioned clock, never a separate unpaced allowance.
    let mut publication_attempts = 0;
    let path = loop {
        let mut session = file.take();
        let (session, prepared) = blocking(move || {
            let prepared = session.prepare_publication();
            (session, prepared)
        })
        .await?;
        file.put(session);
        let (journal, already) = match prepared {
            Ok(Publication::Ready(journal)) => (journal, None),
            Ok(Publication::AlreadyPublished(journal, path)) => (journal, Some(path)),
            Err(error) => return Err(stop(ports, file, error).await),
        };
        // The every-chunk checkpoint, including any new final name, is durable before linking.
        if &journal != file.get().journal() {
            ports
                .transition(DownloadExportTransition::Checkpoint {
                    expected: file.get().journal().clone(),
                    next: journal.clone(),
                })
                .await
                .map_err(user_message)?;
            file.get_mut().checkpointed(journal);
        }
        let linked = {
            // Guard: the staging recheck, the link, the final-name check and Published. Pause,
            // Remove and Lock take the same guard, so exactly one of them or publication wins.
            let _commit = ports.commit().await?;
            let linked = match already {
                Some(path) => Ok(path),
                None => {
                    let mut session = file.take();
                    let (session, linked) = blocking(move || {
                        let linked = session.link();
                        (session, linked)
                    })
                    .await?;
                    file.put(session);
                    linked
                }
            };
            if linked.is_ok() {
                ports
                    .transition(DownloadExportTransition::Publish {
                        expected: file.get().published_journal(),
                    })
                    .await
                    .map_err(user_message)?;
            }
            linked
        };
        match linked {
            Ok(path) => break path,
            Err(ExportError::Local {
                retryable: true,
                reason,
            }) => match local_retry_delay(publication_attempts, session_retries) {
                Some(delay) => {
                    publication_attempts += 1;
                    session_retries += 1;
                    if !ports.live() {
                        return Err("The download is no longer active.".into());
                    }
                    ports.backoff(delay).await;
                }
                None => {
                    let error = ExportError::Local {
                        retryable: false,
                        reason,
                    };
                    return Err(stop(ports, file, error).await);
                }
            },
            Err(error) => return Err(stop(ports, file, error).await),
        }
    };
    // After the guard: best-effort folder flush and removal of our staging name.
    let journal = file.get().published_journal();
    let session = file.take();
    let staging_cleared = blocking(move || {
        flush_destination(&journal);
        session.finish()
    })
    .await?;
    Ok(Some(Exported {
        path,
        staging_cleared,
    }))
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
    #[test]
    fn storage_refusals_become_user_messages_without_doubled_prefixes() {
        // Review L5.
        let malformed = catcoms_storage::StorageError::Malformed.to_string();
        assert_eq!(user_message(malformed), STALE_DOWNLOAD);
        assert_eq!(
            user_message("io error: This file is already being saved. Try again shortly.".into()),
            "This file is already being saved. Try again shortly."
        );
        assert_eq!(user_message("Server stopped".into()), "Server stopped");
        let probe = ExportError::Local {
            retryable: true,
            reason: "busy".into(),
        }
        .to_string();
        assert_eq!(probe.matches("failed").count(), 1, "{probe}");
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

#[cfg(test)]
#[path = "private_downloads/export_tests.rs"]
mod export_tests;

#[cfg(test)]
#[path = "private_downloads/chooser_tests.rs"]
mod chooser_tests;
