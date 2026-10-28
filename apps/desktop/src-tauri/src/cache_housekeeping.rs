//! Desktop side of cache housekeeping (`docs/design-cache-housekeeping.md`).
//!
//! The policy runs inside each server actor (`catcoms_app::cache_housekeeping`). This module owns
//! what needs the vault store or spans servers:
//!
//! - **Attach**: the mount scan of the primary cache, finishing an interrupted leave cleanup of
//!   the same group before Keep opens (a rejoin over a `leaving` marker), and loading or migrating
//!   the sealed per-server limit before the actor starts.
//! - **The Studio reference service**: `creative_pinned_cids` needs the mounted store, so it runs
//!   here, at most once a minute with doubling backoff after failures, and then tells every actor
//!   that protection may have changed.
//! - **Leave cleanup and its resume sweep**, under an in-process claim per group so no attach can
//!   reach a namespace while it is being emptied.
//! - The `get_cache_status` and `set_cache_limit` commands.

use super::*;
use catcoms_app::cache_housekeeping::{
    CacheLimit, CacheSetting, CacheSettingLoad, CacheSettingOrigin, CacheStatus,
};
use catcoms_app::cache_plan;
use catcoms_storage::cache as storage_cache;
use catcoms_storage::BlobStore;
use std::collections::HashSet;

/// Studio reference scans run at most this often.
const STUDIO_SCAN_INTERVAL_MS: u64 = 60_000;
/// Backoff ceiling after repeated scan failures.
const STUDIO_SCAN_MAX_BACKOFF_MS: u64 = 30 * 60_000;
/// Leave cleanup deletes this many cache files per blocking batch, then checks the session.
const LEAVE_BATCH: usize = 256;

/// Process-local housekeeping state shared by every server.
#[derive(Default)]
pub(super) struct CacheBridge {
    /// Groups whose namespace a leave cleanup is emptying right now.
    claims: StdMutex<HashSet<Vec<u8>>>,
    /// Woken whenever a claim is released, so a waiting attach can proceed.
    released: tokio::sync::Notify,
    studio: tokio::sync::Mutex<StudioScans>,
    /// Serialises `set_cache_limit`'s save-then-apply, so two quick choices cannot leave the actor
    /// on a different limit from the sealed record.
    limit_writes: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct StudioScans {
    running: bool,
    /// A request arrived while a scan ran or inside the rate window; serve it once allowed.
    pending: bool,
    scheduled: bool,
    next_allowed: Option<u64>,
    failures: u32,
}

/// What the footer and settings page receive.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) struct UiCacheStatus {
    usage_bytes: u64,
    limit_bytes: Option<u64>,
    unlimited: bool,
    pinned_bytes: u64,
    leaving_soon_files: usize,
    leaving_soon_bytes: u64,
    nearly_full: bool,
    over_limit: bool,
    held_by_pins: bool,
    paused: Option<&'static str>,
}

pub(super) fn bridge_status(status: &CacheStatus) -> UiCacheStatus {
    UiCacheStatus {
        usage_bytes: status.usage_bytes,
        limit_bytes: status.limit_bytes,
        unlimited: status.unlimited,
        pinned_bytes: status.pinned_bytes,
        leaving_soon_files: status.leaving_soon_files,
        leaving_soon_bytes: status.leaving_soon_bytes,
        nearly_full: status.nearly_full,
        over_limit: status.over_limit,
        held_by_pins: status.held_by_pins,
        paused: status.paused.map(|pause| pause.as_str()),
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CacheStatusEvt {
    pub(super) server: u64,
    pub(super) status: UiCacheStatus,
}

/// What leaving freed on this device, as the frontend receives it.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub(super) struct LeaveReport {
    /// Bytes actually removed (cache, scratch and Keep).
    freed_bytes: u64,
    /// Cache bytes kept because Studio still references them.
    kept_bytes: u64,
    /// Entries that could not be removed; cleanup resumes after the next unlock.
    failed: usize,
    /// Whether cleanup ran. False with `failed == 0`: nothing was deleted and nothing will be
    /// (shared by another server entry, taken back by a rejoin, never on disk, or never marked),
    /// and the toast must not claim otherwise. False with `failed > 0`: cleanup could not start
    /// yet (the namespace was still open, or the leave is not durable yet) and its marker stays
    /// for the next attach or unlock.
    cleaned: bool,
}

impl From<catcoms_app::store::LeaveReport> for LeaveReport {
    fn from(report: catcoms_app::store::LeaveReport) -> Self {
        Self {
            freed_bytes: report.freed_bytes,
            kept_bytes: report.kept_bytes,
            failed: report.failed,
            cleaned: report.cleaned,
        }
    }
}

impl LeaveReport {
    /// Nothing removed yet, marker kept: the next attach or unlock carries the leave on.
    pub(super) fn failed() -> Self {
        Self {
            failed: 1,
            ..Self::default()
        }
    }
}

/// Exclusive cleanup custody of one group's namespace, released on drop. Taken by `leave_server`
/// before it stops the server (see [`claim_for_leave`]) and by the resume sweep.
pub(super) struct CleanupClaim<'a> {
    bridge: &'a CacheBridge,
    group: Vec<u8>,
}

impl Drop for CleanupClaim<'_> {
    fn drop(&mut self) {
        self.bridge
            .claims
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.group);
        self.bridge.released.notify_waiters();
    }
}

impl CacheBridge {
    /// Take the cleanup claim for `group`, or `None` if another cleanup holds it.
    pub(super) fn claim(&self, group: &[u8]) -> Option<CleanupClaim<'_>> {
        self.claims
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(group.to_vec())
            .then(|| CleanupClaim {
                bridge: self,
                group: group.to_vec(),
            })
    }
    pub(super) fn claimed(&self, group: &[u8]) -> bool {
        self.claims
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(group)
    }
    /// Wait until no leave cleanup holds `group`. Attach calls this, then re-checks under the
    /// store lock, before it opens the namespace.
    pub(super) async fn wait_unclaimed(&self, group: &[u8]) {
        loop {
            let notified = self.released.notified();
            if !self.claimed(group) {
                return;
            }
            notified.await;
        }
    }
}

/// Attach-time work, on a blocking worker with the store lock held: finish an interrupted leave of
/// this group (everything but the cache), open the production blob stack, scan it, and resolve the
/// limit. Returns the store to attach and the limit to hand to the actor.
pub(super) fn prepare_blob_store(
    store: &ServerStore,
    server_id: Option<u64>,
    group: &[u8],
) -> Result<(Box<dyn BlobStore + Send>, CacheLimit), String> {
    let server = server_id.unwrap_or(0);
    let key = hex::encode(group);
    // A rejoin over a marker: the group is live again, so the cache (its own content-addressed
    // data) stays, but the Keep copies and Downloads the user confirmed deleting must go before
    // Keep opens, or a half-deleted `kept/` would fault it. The marker goes only if that worked.
    // All of it happens under the exclusive attach hold: if another handle still has the
    // namespace open (a server of the same group whose leave could not finish, or one still
    // being dropped), its live `incoming/` and `staging/` must not be removed under it, so the
    // step is skipped and retried at the next attach.
    let mut keep_unsafe = false;
    if let Ok(dir) = store.cache_namespace_dir(&key) {
        if storage_cache::has_leaving_marker(&dir) {
            let removal = match storage_cache::attach_exclusive(&dir) {
                storage_cache::AttachAttempt::Held(held) => {
                    let mut removal = storage_cache::remove_scratch(&dir);
                    removal.add(storage_cache::remove_kept_tree(&dir));
                    // The held lock file keeps the namespace directory: only the marker goes. A
                    // marker that cannot be removed counts as a failure, or Keep would open
                    // healthy under it and the next attach would wipe copies made meanwhile.
                    if removal.failed == 0 && storage_cache::finish_leaving(&dir).is_err() {
                        removal.failed += 1;
                    }
                    drop(held);
                    removal
                }
                storage_cache::AttachAttempt::Busy | storage_cache::AttachAttempt::Failed => {
                    storage_cache::Removal {
                        freed_bytes: 0,
                        failed: 1,
                    }
                }
            };
            if removal.failed > 0 {
                // A half-deleted `kept/` must not be loaded as if it were whole, and copies the
                // user confirmed deleting must not come back: Keep stays off for this mount (the
                // marker stays, so the next attach tries again). Eviction then pauses too.
                keep_unsafe = true;
                tracing::warn!(target: "catcoms_app", server, failed = removal.failed, "STORAGE.CACHE.REJOIN_CLEANUP_INCOMPLETE");
            }
        }
    }
    let mut blobs = store.blob_store(&key).map_err(|e| e.to_string())?;
    if keep_unsafe {
        blobs.disable_retention();
    }
    let report = match blobs.cache_scan(true) {
        Ok(report) => report,
        Err(_) => return Ok((blobs, CacheLimit::Pending)),
    };
    if report.recovered_temps > 0 || report.removed_torn > 0 {
        tracing::info!(target: "catcoms_app", server, recovered = report.recovered_temps, torn = report.removed_torn, "STORAGE.CACHE.RECOVERED");
    }
    // A server being founded or joined has no number yet; `adopt_cache_setting` resolves its
    // limit once registration assigns one.
    let Some(server) = server_id else {
        return Ok((blobs, CacheLimit::Pending));
    };
    let limit = resolve_setting(store, server, group, report.complete, report.bytes);
    Ok((blobs, limit))
}

/// Load the sealed setting, or migrate one from usage when none exists. Never migrates from an
/// incomplete scan. A failed save still uses the computed limit for this mount: it is never below
/// usage / 0.8, and it is simply computed again next time.
fn resolve_setting(
    store: &ServerStore,
    server: u64,
    group: &[u8],
    complete: bool,
    usage: u64,
) -> CacheLimit {
    match store.load_cache_setting(server, group) {
        CacheSettingLoad::Found(setting) => setting.as_limit(),
        CacheSettingLoad::Unreadable => CacheLimit::Unreadable,
        CacheSettingLoad::Missing if !complete => CacheLimit::Pending,
        CacheSettingLoad::Missing => {
            let setting = CacheSetting {
                limit: cache_plan::migrated_limit(usage),
                origin: CacheSettingOrigin::Migrated,
            };
            if let Err(error) = store.save_cache_setting(server, group, &setting, &mut OsCryptoRng)
            {
                tracing::warn!(target: "catcoms_app", server, %error, "STORAGE.CACHE.SETTING_SAVE_FAILED");
            }
            setting.as_limit()
        }
    }
}

/// For a server registered after its blob store attached (found, join): resolve its limit now
/// that it has a number, from the actor's own completed mount scan.
pub(super) async fn adopt_cache_setting(
    state: &AppState,
    server: u64,
    group: &[u8],
    actor: &ServerActor,
) {
    let Ok(status) = actor.cache_status().await else {
        return;
    };
    let Some(complete) = adoption_scan(&status) else {
        return; // no size-limited cache: usage is unknown, so nothing may be migrated from it
    };
    let limit = {
        let guard = state.store.lock().await;
        let Some(store) = guard.as_ref() else {
            return; // in-memory fallback store: no size-limited cache
        };
        resolve_setting(store, server, group, complete, status.usage_bytes)
    };
    let _ = actor.set_cache_limit(limit).await;
}

/// Whether a registered server's status can seed a migration: `None` with no size-limited cache
/// (usage then reads 0 and would save a too-small limit), otherwise whether its scan was complete.
fn adoption_scan(status: &CacheStatus) -> Option<bool> {
    use catcoms_app::cache_housekeeping::CachePause;
    match status.paused {
        Some(CachePause::NoCache) => None,
        Some(CachePause::ScanIncomplete) => Some(false),
        _ => Some(true),
    }
}

/// Ask for a Studio reference scan. Never awaits an actor or the store from the caller's task:
/// the work runs in its own task, rate-limited and coalesced.
pub(super) fn request_studio_scan(app: &AppHandle) {
    tauri::async_runtime::spawn(run_studio_scan(app.clone()));
}

/// Boxed so the coalesced follow-up can call it again without an infinitely sized future.
fn run_studio_scan(
    app: AppHandle,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move { studio_scan_once(app).await })
}

async fn studio_scan_once(app: AppHandle) {
    let state = app.state::<AppState>();
    let clock = SystemClock;
    {
        let mut scans = state.cache.studio.lock().await;
        let now = clock.monotonic_ms();
        let allowed = scans.next_allowed.is_none_or(|at| now >= at);
        if scans.running || !allowed {
            scans.pending = true;
            if !scans.running && !scans.scheduled {
                // Serve the coalesced request once the window opens.
                scans.scheduled = true;
                let wait = scans.next_allowed.unwrap_or(now).saturating_sub(now);
                let later = app.clone();
                tauri::async_runtime::spawn(async move {
                    SystemClock.sleep(Duration::from_millis(wait)).await;
                    {
                        let state = later.state::<AppState>();
                        let mut scans = state.cache.studio.lock().await;
                        scans.scheduled = false;
                    }
                    run_studio_scan(later).await;
                });
            }
            return;
        }
        scans.running = true;
        scans.pending = false;
    }
    let guard = state.store.clone().lock_owned().await;
    let outcome = tokio::task::spawn_blocking(move || {
        let mut guard = guard;
        guard
            .as_mut()
            .map(|store| store.creative_pinned_cids().is_ok())
    })
    .await
    .ok()
    .flatten();
    let now = clock.monotonic_ms();
    let pending = {
        let mut scans = state.cache.studio.lock().await;
        scans.running = false;
        match outcome {
            Some(true) => {
                scans.failures = 0;
                scans.next_allowed = Some(now.saturating_add(STUDIO_SCAN_INTERVAL_MS));
            }
            Some(false) => {
                scans.failures = scans.failures.saturating_add(1);
                let backoff = STUDIO_SCAN_INTERVAL_MS
                    .saturating_mul(1u64 << scans.failures.min(16))
                    .min(STUDIO_SCAN_MAX_BACKOFF_MS);
                scans.next_allowed = Some(now.saturating_add(backoff));
                tracing::warn!(target: "catcoms_app", failures = scans.failures, "STORAGE.CACHE.STUDIO_SCAN_FAILED");
            }
            // Locked vault: nothing to scan; a later unlock asks again.
            None => {}
        }
        std::mem::take(&mut scans.pending)
    };
    if outcome == Some(true) {
        let actors: Vec<ServerActor> = state
            .servers
            .lock()
            .await
            .values()
            .map(|entry| entry.actor.clone())
            .collect();
        for actor in actors {
            actor.cache_protection_changed().await;
        }
    }
    if pending {
        run_studio_scan(app).await;
    }
}

/// How long a leave waits for the stopped actor's blob handle to release the namespace's attach
/// lock (and Keep lease).
const LEASE_RELEASE_ATTEMPTS: u32 = 15;
const LEASE_RELEASE_WAIT: Duration = Duration::from_millis(200);

/// Empty the namespace of a group no server uses any more. Studio-referenced blobs stay; the
/// marker stays while anything failed, so the next unlock resumes. Holds the group's claim from
/// start to finish and the store lock only for the reference scan and enumeration. `generation`
/// is the unlocked session that asked; between batches the cleanup stops if it changed (tests
/// with no session pass `None`). `await_release` gives a just-stopped actor's blob handle a few
/// seconds to drop (leave); the sweep has no such actor and does not wait.
///
/// A namespace that stays open (`LeaveBegin::Busy`: some handle holds its attach lock) is never
/// emptied, and its marker is **kept**: the holder may be a live server of the same group (a join
/// that attached before the leave), or the leaving server's own handle still being dropped, and
/// nothing here can tell them apart. Both are safe with the marker in place. A live server's next
/// attach treats it as a rejoin (the cache stays, the confirmed-deleted Keep copies go; it
/// attached while the leaving server held the Keep lease, which `claim_for_leave` guarantees, so
/// its Keep store is faulted for this mount and holds none of its own), and the sweep skips groups
/// a registered server uses; otherwise the next unlock finishes the leave. Reported as a failure.
pub(super) async fn clean_namespace(
    state: &AppState,
    claim: &CleanupClaim<'_>,
    generation: Option<u64>,
    await_release: bool,
) -> LeaveReport {
    use catcoms_app::store::LeaveBegin;
    let key = hex::encode(&claim.group);
    let attempts = if await_release {
        LEASE_RELEASE_ATTEMPTS
    } else {
        1
    };
    let mut begun = None;
    for attempt in 0..attempts {
        // Phase 1, store lock held: marker and lease checks, the reference scan, a cache-only
        // handle and the exact list of CIDs to delete.
        let guard = state.store.clone().lock_owned().await;
        let key = key.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let mut guard = guard;
            guard.as_mut()?.begin_leave_cleanup(&key).ok()
        })
        .await
        .ok()
        .flatten();
        match outcome {
            Some(LeaveBegin::Busy) if attempt + 1 < attempts => {
                SystemClock.sleep(LEASE_RELEASE_WAIT).await;
            }
            other => {
                begun = other;
                break;
            }
        }
    }
    let mut cleanup = match begun {
        Some(LeaveBegin::Ready(cleanup)) => cleanup,
        // Taken back by a rejoin: its files stay, truthfully, and nothing is left to resume.
        Some(LeaveBegin::NoMarker) => return LeaveReport::default(),
        // Still open after the wait (see above), or unreadable: nothing deleted, marker kept.
        Some(LeaveBegin::Busy) | Some(LeaveBegin::Unavailable) | None => {
            return LeaveReport::failed()
        }
    };
    let failed = LeaveReport::failed();
    // Phase 2, no store lock: bounded batches, checking between them that the unlocked session
    // is still the one that asked. A changed session abandons the rest (kept, marker stays).
    loop {
        if let Some(generation) = generation {
            if require_ui_session_generation(state, generation)
                .await
                .is_err()
            {
                cleanup.abandon();
                break;
            }
        }
        let Ok((back, more)) = tokio::task::spawn_blocking(move || {
            let more = cleanup.delete_batch(LEAVE_BATCH);
            (cleanup, more)
        })
        .await
        else {
            return failed; // the worker panicked; the marker stays for the next unlock
        };
        cleanup = back;
        if !more {
            break;
        }
    }
    // Phase 3: scratch, `kept/` and the marker (removed only if nothing failed).
    let report: LeaveReport = tokio::task::spawn_blocking(move || cleanup.finish())
        .await
        .map(LeaveReport::from)
        .unwrap_or(failed);
    tracing::info!(target: "catcoms_app", freed = report.freed_bytes, kept = report.kept_bytes, failed = report.failed, "STORAGE.CACHE.LEAVE_CLEANED");
    report
}

/// After an unlock in which every registry server restored: finish the cleanup of any namespace
/// whose leave was interrupted. A namespace a running server uses is never touched.
pub(super) async fn sweep_leaving(state: &AppState, generation: Option<u64>) {
    let root = {
        let guard = state.store.lock().await;
        let Some(store) = guard.as_ref() else {
            return;
        };
        store.blobs_root()
    };
    let marked = tokio::task::spawn_blocking(move || storage_cache::leaving_namespaces(&root))
        .await
        .unwrap_or_default();
    for (name, _) in marked {
        let Ok(group) = hex::decode(&name) else {
            continue;
        };
        // Check running servers and take the claim together, under the servers lock, so an
        // attach that starts after this point waits for the claim.
        let claim = {
            let servers = state.servers.lock().await;
            if servers.values().any(|entry| entry.group_id == group) {
                continue;
            }
            state.cache.claim(&group)
        };
        let Some(claim) = claim else {
            continue;
        };
        let report = clean_namespace(state, &claim, generation, false).await;
        drop(claim);
        tracing::info!(target: "catcoms_app", namespace = %name, freed = report.freed_bytes, failed = report.failed, "STORAGE.CACHE.LEAVE_RESUMED");
    }
}

/// Take the cleanup claim for `server`'s group while the server is still registered and running,
/// before `leave_server` stops it. `None` only when the server is not registered. Waits while
/// another leave holds the claim: a concurrent leave of the same server, or of an alias row (the
/// sweep never claims a group a registered server uses). The claim is taken even when an alias
/// shares the group; `leave_server` drops it after removing the row if an alias still remains.
/// That way two aliases left together run one after the other, and the last one removed claims
/// while it still runs and cleans the namespace, instead of each seeing the other and neither
/// marking it (which would leave the cache and Keep copies on disk with nothing to resume).
///
/// Why before the stop: the server's own handle holds the namespace's Keep lease until the stop
/// drops it, and from the claim on no attach of the group can start (attach waits on the claim).
/// So any handle still open when the cleanup runs attached while this server held the lease, and
/// has a faulted Keep store with no copies of its own. That is what makes keeping the marker on a
/// busy namespace safe: the holder's next attach removes only the Keep copies the user confirmed
/// deleting. A claim taken after the stop would let a join land in between with a healthy Keep,
/// whose copies the kept marker would later wipe without asking.
///
/// Residual: that argument assumes the leaving server owns the lease. If it attached second in
/// this mount (its Keep faulted) and the owner has since gone, a join of the same group could
/// attach before this claim with a healthy Keep and, if still unregistered at cleanup, sit under
/// the kept marker. That needs three attaches of one group in one mount.
pub(super) async fn claim_for_leave(state: &AppState, server: u64) -> Option<CleanupClaim<'_>> {
    loop {
        let group = {
            let servers = state.servers.lock().await;
            let group = servers.get(&server)?.group_id.clone();
            // Claimed under the servers lock, as the sweep does, so the two cannot interleave.
            if let Some(claim) = state.cache.claim(&group) {
                return Some(claim);
            }
            group
        };
        state.cache.wait_unclaimed(&group).await;
    }
}

/// Write the `leaving` marker into the group's namespace before its server records go, so an
/// interrupted cleanup resumes. `Ok(false)` when the namespace does not exist (nothing to clean).
pub(super) fn mark_leaving(store: &ServerStore, group: &[u8]) -> Result<bool, String> {
    let dir = store
        .cache_namespace_dir(&hex::encode(group))
        .map_err(|e| e.to_string())?;
    if !dir.is_dir() {
        return Ok(false);
    }
    storage_cache::write_leaving_marker(&dir).map_err(|e| e.to_string())?;
    Ok(true)
}

/// This server's cache status on this device (cheap; no decryption).
#[tauri::command]
pub(super) async fn get_cache_status(
    state: State<'_, AppState>,
    server: u64,
) -> Result<UiCacheStatus, String> {
    require_unlocked_session(&state).await?;
    let actor = actor_of(&state, server).await?;
    Ok(bridge_status(&actor.cache_status().await?))
}

/// Choose this server's cache limit on this device: bytes within 1 GiB to 1 TiB, or `null` for No
/// limit. The sealed record is written first; only then does the actor apply it, so a crash never
/// leaves the actor evicting under a limit the next launch would not know.
#[tauri::command]
pub(super) async fn set_cache_limit(
    state: State<'_, AppState>,
    server: u64,
    bytes: Option<u64>,
) -> Result<UiCacheStatus, String> {
    require_unlocked_session(&state).await?;
    if !cache_plan::valid_limit(bytes) {
        return Err("Choose a limit between 1 GiB and 1 TiB, or no limit.".into());
    }
    // Save and apply as one step per process, so the actor always ends on the sealed value.
    let _serial = state.cache.limit_writes.lock().await;
    let (actor, group) = {
        let servers = state.servers.lock().await;
        let entry = servers
            .get(&server)
            .ok_or_else(|| "That server is not open.".to_string())?;
        (entry.actor.clone(), entry.group_id.clone())
    };
    let setting = CacheSetting {
        limit: bytes,
        origin: CacheSettingOrigin::User,
    };
    {
        let guard = state.store.lock().await;
        let store = guard
            .as_ref()
            .ok_or_else(|| "The vault is locked.".to_string())?;
        store
            .save_cache_setting(server, &group, &setting, &mut OsCryptoRng)
            .map_err(|e| e.to_string())?;
    }
    Ok(bridge_status(
        &actor.set_cache_limit(setting.as_limit()).await?,
    ))
}

#[cfg(test)]
mod tests;
