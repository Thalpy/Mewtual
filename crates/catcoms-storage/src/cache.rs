//! Bookkeeping for the size-limited primary blob cache (`<vault>/blobs/<group>/`).
//!
//! Three jobs live here, all of them filesystem facts rather than policy:
//!
//! - **Crash-safe temporary names.** The primary cache writes each blob to
//!   `incoming/<cid hex>.<process id>.<mount generation>.<sequence>`, syncs it and renames it into
//!   place, so a CID-named file only ever appears with complete content. The mount generation is a
//!   process-wide counter handed to each primary handle when it opens and kept in a live set until the
//!   handle drops. Recovery can then tell an abandoned temporary file (another process, or a handle
//!   that no longer exists) from one a live handle in this process is still writing.
//! - **The cache scan.** One pass over the directory records each CID-named regular file and its
//!   length (metadata only, nothing is decrypted) into a [`CacheIndex`]. The first scan of a mount
//!   also fixes the *mount set*: the CIDs that were present before this mount wrote anything. Only
//!   those may ever be classed as unclaimed, which keeps a blob published during this mount (for
//!   example a Studio frame stored before any record names it) out of that class.
//! - **Leave cleanup.** The marker, scratch-directory and `kept/` removal steps used when the user
//!   leaves a server. Policy (what may be deleted, Studio pins) stays with the caller; these helpers
//!   only remove what they are pointed at, never follow links, and count what they could not remove
//!   so a caller never reports space it did not free.
//!
//! The eviction *planner* is deliberately not here: it needs the replicated file index and the pin
//! sets, which belong to the application crate.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::cid::Cid;
use crate::StorageError;

/// Subdirectory of the primary cache holding writes in progress. Never inside a Keep directory:
/// Keep's loaders reject any entry they do not recognise.
pub(crate) const INCOMING_DIR: &str = "incoming";
/// Staging subdirectory name, duplicated from `blob.rs` so leave cleanup can name it.
const STAGING_DIR: &str = "staging";
/// Keep subdirectory name, as created by `ServerStore::blob_store`.
const KEPT_DIR: &str = "kept";
/// The Keep lease file inside `kept/` (see `kept.rs`).
const KEPT_LEASE: &str = "lease.lock";
/// Marker that a leave cleanup of this namespace started and has not finished. Its presence in a
/// namespace whose group no running server uses is the only trigger for resumed cleanup.
pub const LEAVING_MARKER: &str = "leaving";
/// Fixed marker content, so a stray empty file is not mistaken for ours.
const LEAVING_TAG: &[u8] = b"mewtual/cache-leaving/v1\n";
/// The shortest valid sealed record: a 24-byte nonce plus the 16-byte tag of an empty plaintext.
/// A CID-named file shorter than this in the sealing store is a torn write and can never verify.
pub const MIN_SEALED_RECORD: u64 = 40;
/// Upper bound on entries one scan will examine. A larger directory is reported incomplete, and
/// an incomplete index never authorizes eviction.
pub const MAX_CACHE_ENTRIES: usize = 1 << 20;
/// Upper bound on entries one tree removal will visit, so a hostile or damaged tree cannot pin a
/// leave cleanup in an unbounded walk. Anything left is counted as failed and retried later.
const MAX_REMOVAL_ENTRIES: usize = 1 << 20;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static LIVE_GENERATIONS: Mutex<BTreeSet<u64>> = Mutex::new(BTreeSet::new());

/// A live primary-cache handle's generation. Registered on creation and removed on drop, so
/// recovery never deletes a temporary file a handle in this process may still be writing.
#[derive(Debug)]
pub(crate) struct MountGeneration(u64);

impl MountGeneration {
    pub(crate) fn register() -> Self {
        let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        // A poisoned set only ever loses the ability to protect in-flight temp files; recovery
        // then treats generations as live (see `is_live`), which keeps files rather than deletes.
        if let Ok(mut live) = LIVE_GENERATIONS.lock() {
            live.insert(generation);
        }
        Self(generation)
    }
    pub(crate) fn value(&self) -> u64 {
        self.0
    }
}

impl Drop for MountGeneration {
    fn drop(&mut self) {
        if let Ok(mut live) = LIVE_GENERATIONS.lock() {
            live.remove(&self.0);
        }
    }
}

/// Whether `generation` belongs to a primary handle that is still open in this process. A
/// poisoned registry answers "live", so the failure mode is a leftover file, never a lost write.
fn is_live(generation: u64) -> bool {
    LIVE_GENERATIONS
        .lock()
        .map(|live| live.contains(&generation))
        .unwrap_or(true)
}

/// The temporary file name for one write. Process id plus generation plus a per-handle sequence
/// separates every concurrent writer without randomness; the file is still opened `create_new`.
pub(crate) fn temp_name(cid: &Cid, generation: u64, sequence: u64) -> String {
    format!(
        "{}.{}.{}.{}",
        cid.to_hex(),
        std::process::id(),
        generation,
        sequence
    )
}

/// `(process id, generation)` of a well-formed temporary name.
fn parse_temp(name: &str) -> Option<(u32, u64)> {
    let mut parts = name.split('.');
    let cid = parts.next()?;
    let pid = parts.next()?.parse().ok()?;
    let generation = parts.next()?.parse().ok()?;
    parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() || Cid::from_hex(cid).is_none() {
        return None;
    }
    Some((pid, generation))
}

/// What one handle knows about its cache directory: the size of every CID-named record, which of
/// them were present before this mount wrote anything, and whether the last scan saw everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheIndex {
    sizes: BTreeMap<Cid, u64>,
    mount: BTreeSet<Cid>,
    complete: bool,
    /// Running sum of `sizes`, so the actor can compare usage with the limit every turn without
    /// walking or cloning the index.
    usage: u64,
}

/// The cheap facts the actor checks every turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheUsage {
    pub bytes: u64,
    pub complete: bool,
}

fn total(sizes: &BTreeMap<Cid, u64>) -> u64 {
    sizes.values().fold(0, |sum, n| sum.saturating_add(*n))
}

impl CacheIndex {
    /// Total sealed bytes of every indexed record.
    pub fn usage(&self) -> u64 {
        self.usage
    }
    /// Usage and completeness together.
    pub fn summary(&self) -> CacheUsage {
        CacheUsage {
            bytes: self.usage,
            complete: self.complete,
        }
    }
    /// Sealed length of each indexed record.
    pub fn sizes(&self) -> &BTreeMap<Cid, u64> {
        &self.sizes
    }
    /// CIDs present at the mount scan that this handle has not written, promoted or deleted since.
    pub fn mount_set(&self) -> &BTreeSet<Cid> {
        &self.mount
    }
    /// False when the last scan skipped an entry it could not read or hit [`MAX_CACHE_ENTRIES`].
    /// An incomplete index must never authorize eviction: unseen bytes make usage a guess.
    pub fn complete(&self) -> bool {
        self.complete
    }
    /// Test and planner constructor: an index over explicit sizes, all of them in the mount set.
    pub fn from_sizes(sizes: BTreeMap<Cid, u64>, complete: bool) -> Self {
        let mount = sizes.keys().copied().collect();
        Self::with_mount(sizes, mount, complete)
    }
    /// Test and planner constructor with an explicit mount set. CIDs the index does not size are
    /// dropped from the mount set, as the store itself would never hold them there.
    pub fn with_mount(sizes: BTreeMap<Cid, u64>, mount: BTreeSet<Cid>, complete: bool) -> Self {
        let mount = mount
            .into_iter()
            .filter(|cid| sizes.contains_key(cid))
            .collect();
        let usage = total(&sizes);
        Self {
            sizes,
            mount,
            complete,
            usage,
        }
    }
    /// This handle stored `cid` with `len` sealed bytes. It leaves the mount set: it was written
    /// during this mount, so it can no longer be presumed unclaimed.
    pub(crate) fn note_written(&mut self, cid: Cid, len: u64) {
        if let Some(old) = self.sizes.insert(cid, len) {
            self.usage = self.usage.saturating_sub(old);
        }
        self.usage = self.usage.saturating_add(len);
        self.mount.remove(&cid);
    }
    /// This handle removed `cid`.
    pub(crate) fn note_removed(&mut self, cid: &Cid) {
        if let Some(old) = self.sizes.remove(cid) {
            self.usage = self.usage.saturating_sub(old);
        }
        self.mount.remove(cid);
    }
}

/// Counts from one scan, for logs and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheScanReport {
    pub entries: usize,
    pub bytes: u64,
    pub complete: bool,
    /// Abandoned temporary files removed from `incoming/` (mount scans only).
    pub recovered_temps: usize,
    /// CID-named files too short to be a sealed record, removed (mount scans of the sealing store).
    pub removed_torn: usize,
}

/// Scan `dir`. A mount scan (`recover`) also removes abandoned temporary files and, when `sealed`,
/// torn records, and fixes the mount set; a refresh keeps the previous mount set minus CIDs that
/// are gone, so blobs other handles wrote since the mount never join it.
pub(crate) fn scan(
    dir: &Path,
    sealed: bool,
    recover: bool,
    previous: Option<&CacheIndex>,
) -> (CacheIndex, CacheScanReport) {
    let mut report = CacheScanReport::default();
    let mut sizes = BTreeMap::new();
    let mut complete = true;
    if recover {
        report.recovered_temps = recover_incoming(&dir.join(INCOMING_DIR));
    }
    match fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                if report.entries >= MAX_CACHE_ENTRIES {
                    complete = false;
                    break;
                }
                let Ok(entry) = entry else {
                    complete = false;
                    continue;
                };
                let Some(cid) = entry.file_name().to_str().and_then(Cid::from_hex) else {
                    continue; // staging/, kept/, incoming/, the leaving marker
                };
                report.entries += 1;
                // `file_type` does not follow links. A link or directory under a CID name is not
                // a record this store wrote; it is neither counted nor ever offered for deletion.
                let Ok(kind) = entry.file_type() else {
                    complete = false;
                    continue;
                };
                if !kind.is_file() {
                    continue;
                }
                let Ok(metadata) = entry.metadata() else {
                    complete = false;
                    continue;
                };
                let len = metadata.len();
                if recover && sealed && len < MIN_SEALED_RECORD {
                    if fs::remove_file(entry.path()).is_ok() {
                        report.removed_torn += 1;
                    } else {
                        complete = false;
                    }
                    continue;
                }
                sizes.insert(cid, len);
            }
        }
        Err(_) => complete = false,
    }
    let mount = if recover {
        sizes.keys().copied().collect()
    } else {
        previous
            .map(|index| {
                index
                    .mount
                    .iter()
                    .filter(|cid| sizes.contains_key(cid))
                    .copied()
                    .collect()
            })
            .unwrap_or_default()
    };
    report.bytes = total(&sizes);
    report.complete = complete;
    (
        CacheIndex {
            usage: report.bytes,
            sizes,
            mount,
            complete,
        },
        report,
    )
}

/// Remove temporary files no live writer can still own: another process's, a dropped handle's,
/// or any name this module did not produce. Returns how many were removed.
fn recover_incoming(incoming: &Path) -> usize {
    let Ok(entries) = fs::read_dir(incoming) else {
        return 0;
    };
    let pid = std::process::id();
    let mut removed = 0;
    for entry in entries.flatten().take(MAX_CACHE_ENTRIES) {
        let abandoned = match entry.file_name().to_str().and_then(parse_temp) {
            Some((owner, generation)) => owner != pid || !is_live(generation),
            None => true,
        };
        if abandoned
            && entry.file_type().is_ok_and(|kind| kind.is_file())
            && fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// Bytes freed and entries that could not be removed by one cleanup step. A caller reports only
/// `freed_bytes` as reclaimed and keeps its resume marker while `failed` is non-zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Removal {
    pub freed_bytes: u64,
    pub failed: usize,
}

impl Removal {
    pub fn add(&mut self, other: Removal) {
        self.freed_bytes = self.freed_bytes.saturating_add(other.freed_bytes);
        self.failed = self.failed.saturating_add(other.failed);
    }
}

/// Write the leave marker into the namespace and flush it, so a cleanup interrupted by a crash or
/// a lock is resumed later. Idempotent.
pub fn write_leaving_marker(namespace: &Path) -> Result<(), StorageError> {
    use std::io::Write;
    let path = namespace.join(LEAVING_MARKER);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .map_err(|e| StorageError::Io(e.to_string()))?;
    file.write_all(LEAVING_TAG)
        .and_then(|()| file.sync_all())
        .map_err(|e| StorageError::Io(e.to_string()))?;
    crate::kept::directory_barrier(namespace)
}

/// Whether `namespace` carries a leave marker written by [`write_leaving_marker`].
pub fn has_leaving_marker(namespace: &Path) -> bool {
    let path = namespace.join(LEAVING_MARKER);
    fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && m.len() == LEAVING_TAG.len() as u64)
        && crate::blob::read_bounded(&path, LEAVING_TAG.len())
            .ok()
            .flatten()
            .is_some_and(|bytes| bytes == LEAVING_TAG)
}

/// Remove the marker, then the namespace directory if nothing else is left in it. Returns whether
/// the namespace itself is gone. Call only once every delete of the cleanup succeeded.
pub fn finish_leaving(namespace: &Path) -> Result<bool, StorageError> {
    match fs::remove_file(namespace.join(LEAVING_MARKER)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(StorageError::Io(e.to_string())),
    }
    // `remove_dir` refuses a non-empty directory, which is exactly the "Studio-pinned blobs keep
    // the namespace" case; that is success, not an error.
    Ok(fs::remove_dir(namespace).is_ok())
}

/// Namespaces under `blobs_root` that carry a leave marker, as `(directory name, path)`.
/// Bounded; names are returned only if they are a plausible namespace (lowercase hex).
pub fn leaving_namespaces(blobs_root: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(blobs_root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten().take(4096) {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.is_empty()
            || name.len() > 512
            || !name
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            continue;
        }
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) && has_leaving_marker(&path) {
            found.push((name, path));
        }
    }
    found.sort();
    found
}

/// Remove the `staging/` and `incoming/` trees of a namespace whose server has left.
pub fn remove_scratch(namespace: &Path) -> Removal {
    let mut removal = Removal::default();
    let mut budget = MAX_REMOVAL_ENTRIES;
    for name in [STAGING_DIR, INCOMING_DIR] {
        remove_tree(&namespace.join(name), &mut removal, &mut budget);
    }
    removal
}

/// The namespace's attach lock file. Every primary cache handle ([`SealingBlobStore::open_cache`],
/// so every attached server and every Studio per-request handle) holds it **shared** for its
/// lifetime; leave cleanup and the rejoin-over-marker step take it **exclusively** before
/// deleting anything and hold it to the end. Unlike the Keep lease, which only the first attach
/// of a group can own (a second attach of the same group, such as a join while the group's server
/// still runs, gets a faulted Keep store and no lease), this is held by every attachment, so an
/// exclusive hold really proves no handle in this process has the namespace open.
///
/// Every attempt is a try-lock, so no caller ever waits on a lock file that cleanup is about to
/// unlink. In-process, the desktop's cleanup claim keeps attaches out while a cleanup holds the
/// lock; an attach that still meets it fails with [`StorageError::VaultBusy`] rather than opening
/// a namespace that is being emptied.
///
/// Residual: Linux emulates `flock` on NFS with per-process POSIX locks, under which a second
/// lock in the same process succeeds, so this proof (like the vault and Keep locks) assumes a
/// local filesystem.
///
/// [`SealingBlobStore::open_cache`]: crate::SealingBlobStore::open_cache
pub const ATTACH_LOCK: &str = "attach.lock";

/// A primary handle's shared hold on [`ATTACH_LOCK`]; dropping it releases the hold.
#[derive(Debug)]
pub(crate) struct AttachShared {
    _held: fs::File,
}

/// Take a shared hold on the namespace's attach lock. Fails with [`StorageError::VaultBusy`] while
/// a cleanup holds it exclusively (a namespace being emptied must not be attached), and with
/// [`StorageError::Io`] when the lock cannot be taken at all: a handle that could not prove it is
/// attached must not exist, or a cleanup could empty the namespace under it.
pub(crate) fn attach_shared(namespace: &Path) -> Result<AttachShared, StorageError> {
    let file = attach_file(namespace)?;
    match file.try_lock_shared() {
        Ok(()) => Ok(AttachShared { _held: file }),
        Err(fs::TryLockError::WouldBlock) => Err(StorageError::VaultBusy),
        Err(fs::TryLockError::Error(e)) => Err(StorageError::Io(e.to_string())),
    }
}

/// An exclusive hold on a namespace's [`ATTACH_LOCK`]: no primary handle is open while it lives.
/// It remembers its namespace, so it can only vouch for that one
/// (see [`SealingBlobStore::open_cache_under`](crate::SealingBlobStore::open_cache_under)).
#[derive(Debug)]
pub struct AttachExclusive {
    namespace: PathBuf,
    _held: fs::File,
}

impl AttachExclusive {
    /// The namespace directory this hold covers.
    pub fn namespace(&self) -> &Path {
        &self.namespace
    }
}

/// Outcome of trying to take the attach lock exclusively.
#[derive(Debug)]
pub enum AttachAttempt {
    Held(AttachExclusive),
    /// A primary handle is open: the namespace is attached, or a handle is still being dropped.
    /// The two cannot be told apart here, so a caller keeps its data (and any marker) and retries.
    Busy,
    /// The lock file could not be opened or locked. Nothing may be deleted.
    Failed,
}

/// Try to take the attach lock exclusively (see [`ATTACH_LOCK`]). Creates the lock file (but
/// never the namespace directory) if it is absent.
pub fn attach_exclusive(namespace: &Path) -> AttachAttempt {
    let Ok(file) = attach_file(namespace) else {
        return AttachAttempt::Failed;
    };
    match file.try_lock() {
        Ok(()) => AttachAttempt::Held(AttachExclusive {
            namespace: namespace.to_path_buf(),
            _held: file,
        }),
        Err(fs::TryLockError::WouldBlock) => AttachAttempt::Busy,
        Err(fs::TryLockError::Error(_)) => AttachAttempt::Failed,
    }
}

fn attach_file(namespace: &Path) -> Result<fs::File, StorageError> {
    fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(namespace.join(ATTACH_LOCK))
        .map_err(|e| StorageError::Io(e.to_string()))
}

/// Release the exclusive hold and unlink the lock file, so an emptied namespace directory can be
/// removed. Missing is fine. Call only as the last step before removing the namespace: from here
/// on nothing proves the namespace unattached, which is safe only because the caller's cleanup
/// claim still keeps every in-process attach out.
pub fn remove_attach_lock(held: AttachExclusive) -> Result<(), StorageError> {
    let path = held.namespace.join(ATTACH_LOCK);
    drop(held);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(StorageError::Io(e.to_string())),
    }
}

/// The namespace's Keep lease (`kept/lease.lock`), held by whoever owns the namespace's Keep store
/// (only the first attach of a group; see [`ATTACH_LOCK`] for the lock every attach holds). Leave
/// cleanup takes it, after the attach lock, so the wholesale `kept/` removal owns the directory.
#[derive(Debug)]
pub struct KeptLease {
    /// Held only for its lock; dropping it releases the lease.
    _held: fs::File,
}

/// Outcome of trying to take the Keep lease.
#[derive(Debug)]
pub enum LeaseAttempt {
    /// Taken, or there is no `kept/` directory (absent, a file or a link), so no Keep store can
    /// hold it. This says nothing about whether the namespace is attached: callers take the
    /// [`ATTACH_LOCK`] exclusively first.
    Held(Option<KeptLease>),
    /// Another handle holds it (a Keep store that has not been dropped yet). Nothing may be
    /// removed from `kept/`.
    Busy,
    /// `kept/` is not a plain directory or the lease could not be opened.
    Failed,
}

/// Try to take the namespace's Keep lease without creating `kept/` if it is absent.
pub fn lock_kept(namespace: &Path) -> LeaseAttempt {
    let kept = namespace.join(KEPT_DIR);
    match fs::symlink_metadata(&kept) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LeaseAttempt::Held(None),
        Err(_) => return LeaseAttempt::Failed,
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => return LeaseAttempt::Held(None),
        Ok(_) => {}
    }
    match crate::vault::try_lock_file(&kept.join(KEPT_LEASE)) {
        Ok(lease) => LeaseAttempt::Held(Some(KeptLease { _held: lease })),
        Err(StorageError::VaultBusy) => LeaseAttempt::Busy,
        Err(_) => LeaseAttempt::Failed,
    }
}

/// Delete `kept/` wholesale under its own lease: every entry except the lease file, then the lease
/// file and the directory. Used only after the server has left, so no Keep record is needed and no
/// device binding is required, which is what makes this step resumable. If another handle holds
/// the lease, nothing is removed and one failure is reported.
pub fn remove_kept_tree(namespace: &Path) -> Removal {
    match lock_kept(namespace) {
        LeaseAttempt::Held(lease) => remove_kept_tree_held(namespace, lease),
        LeaseAttempt::Busy | LeaseAttempt::Failed => Removal {
            freed_bytes: 0,
            failed: 1,
        },
    }
}

/// As [`remove_kept_tree`], with the lease already taken by [`lock_kept`] (it is released here).
pub fn remove_kept_tree_held(namespace: &Path, lease: Option<KeptLease>) -> Removal {
    let kept = namespace.join(KEPT_DIR);
    let mut removal = Removal::default();
    match fs::symlink_metadata(&kept) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return removal,
        Err(_) => {
            removal.failed = 1;
            return removal;
        }
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            // Never follow a link out of the namespace; just unlink the link itself.
            if fs::remove_file(&kept).is_err() {
                removal.failed = 1;
            }
            return removal;
        }
        Ok(_) => {}
    }
    let lease_path = kept.join(KEPT_LEASE);
    let Some(lease) = lease else {
        // `kept/` appeared after `lock_kept` saw none: someone attached meanwhile. Keep it.
        removal.failed = 1;
        return removal;
    };
    let mut budget = MAX_REMOVAL_ENTRIES;
    match fs::read_dir(&kept) {
        Ok(entries) => {
            for entry in entries {
                let Ok(entry) = entry else {
                    removal.failed += 1;
                    continue;
                };
                if entry.file_name() == KEPT_LEASE {
                    continue;
                }
                remove_tree(&entry.path(), &mut removal, &mut budget);
            }
        }
        Err(_) => removal.failed += 1,
    }
    // Close the lease handle before unlinking its file. std opens files with delete sharing, so
    // the unlink itself would work while it is open, but under Windows' classic delete semantics
    // the open file would only be marked for deletion and keep `kept/` non-empty for `remove_dir`.
    drop(lease);
    if removal.failed == 0
        && (fs::remove_file(&lease_path).is_err() || fs::remove_dir(&kept).is_err())
    {
        removal.failed += 1;
    }
    removal
}

/// Remove `path` and everything below it without following links, counting freed file bytes.
fn remove_tree(path: &Path, removal: &mut Removal, budget: &mut usize) {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            removal.failed += 1;
            return;
        }
    };
    if *budget == 0 {
        removal.failed += 1;
        return;
    }
    *budget -= 1;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        match fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => remove_tree(&entry.path(), removal, budget),
                        Err(_) => removal.failed += 1,
                    }
                }
            }
            Err(_) => removal.failed += 1,
        }
        if fs::remove_dir(path).is_err() {
            removal.failed += 1;
        }
    } else {
        // A file, or a link (removed as a link, never followed). On Windows a directory link is
        // removed with `remove_dir`; try that if `remove_file` refuses it.
        let len = if metadata.is_file() {
            metadata.len()
        } else {
            0
        };
        if fs::remove_file(path).is_ok() || fs::remove_dir(path).is_ok() {
            removal.freed_bytes = removal.freed_bytes.saturating_add(len);
        } else {
            removal.failed += 1;
        }
    }
}

#[cfg(test)]
mod tests;
