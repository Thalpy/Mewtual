//! Journaled export of a completed private Download to a user-visible file.
//!
//! Encrypted acquisition stays the only network phase. Once the whole private copy is verified,
//! native writes plaintext to `<name>.mewtual-part` in the Downloads folder and publishes the final
//! name without replacement. This module makes that export restart-safe:
//!
//! - [`ExportJournal`] is sealed inside the Download record (`mewtual/download-transfer/v2`). It
//!   records the destination folder, both leaf names, the partial file's handle identity and a
//!   checkpoint of fsynced whole chunks. A saved path is never authority on its own.
//! - [`ExportFile`] is an owned value that native moves into `spawn_blocking`. It performs every
//!   external filesystem effect and holds no store borrow, so a stalled Downloads volume can pin
//!   only that save's worker, never the server actor or app Lock.
//! - The `KeptBlobStore` methods at the end are the only journal transitions. They run on the
//!   actor, write only the sealed record in the vault, and are fenced by the export lease
//!   generation or, for the lease-free preflight edges, by an exact journal compare-and-swap.
//!
//! See `docs/communication-recovery/EXTERNAL-DOWNLOAD-RESUME.md` for the state machine and crash
//! table this code implements.
use super::external::{self, FileIdentity, Leaf};
use super::*;
use crate::CidHasher;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Weak};

/// Largest plaintext a journaled export may describe: the 128-chunk plan ceiling at the 8 MiB
/// chunk size (the application's 1 GiB file limit).
pub(super) const EXPORT_MAX_BYTES: u64 = CHUNK_LIMIT as u64 * crate::CHUNK_PAD_CEILING as u64;
/// Checkpoint cadence in whole chunks. Checkpoints are sparse on purpose: every record write is a
/// chance to meet a full vault volume, and resume verification recovers anything written after the
/// last checkpoint anyway.
pub const EXPORT_CHECKPOINT_CHUNKS: usize = 8;

/// Persisted reason a journaled export stopped and needs the user. Both keep encrypted progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportHalt {
    /// The folder, its volume, our permission or the partial file cannot be reached now. Trying
    /// again may succeed later (for example after reconnecting a drive).
    Unavailable,
    /// Something is there but it is not the file Mewtual created, or the folder is a different
    /// directory. Resuming at this destination cannot succeed; save a new copy instead.
    Changed,
}

/// Sealed export journal. It crosses between the actor and native in-process only, never IPC;
/// its folder path, leaf names and identities are redacted from Debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct ExportJournal {
    pub(super) folder: Vec<u8>,
    pub(super) folder_id: FileIdentity,
    pub(super) staging: String,
    pub(super) final_leaf: String,
    pub(super) file_id: FileIdentity,
    pub(super) written_chunks: u32,
    pub(super) written_bytes: u64,
    pub(super) halt: Option<ExportHalt>,
}
impl std::fmt::Debug for ExportJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportJournal")
            .field("written_chunks", &self.written_chunks)
            .field("written_bytes", &self.written_bytes)
            .field("halt", &self.halt)
            .finish_non_exhaustive()
    }
}
impl ExportJournal {
    /// Whole chunks covered by the last durable checkpoint.
    pub fn written_chunks(&self) -> usize {
        self.written_chunks as usize
    }
    pub fn halt(&self) -> Option<ExportHalt> {
        self.halt
    }
    /// The four fields that name the destination object. They never change for one journal.
    pub(super) fn same_destination(&self, other: &Self) -> bool {
        self.folder == other.folder
            && self.folder_id == other.folder_id
            && self.staging == other.staging
            && self.file_id == other.file_id
    }
    pub(super) fn encode(&self, e: &mut Encoder) {
        e.put_bytes(&self.folder).expect("bounded folder");
        e.put_bytes(&self.folder_id.encode())
            .expect("fixed identity");
        e.put_bytes(self.staging.as_bytes()).expect("bounded leaf");
        e.put_bytes(self.final_leaf.as_bytes())
            .expect("bounded leaf");
        e.put_bytes(&self.file_id.encode()).expect("fixed identity");
        e.put_u32(self.written_chunks);
        e.put_u64(self.written_bytes);
        e.put_u8(match self.halt {
            None => 0,
            Some(ExportHalt::Unavailable) => 1,
            Some(ExportHalt::Changed) => 2,
        });
    }
    pub(super) fn decode(d: &mut Decoder<'_>) -> Result<Self, StorageError> {
        let bad = |_| StorageError::Malformed;
        let folder = d.get_bytes().map_err(bad)?.to_vec();
        let folder_id =
            FileIdentity::decode(d.get_bytes().map_err(bad)?).ok_or(StorageError::Malformed)?;
        let staging = std::str::from_utf8(d.get_bytes().map_err(bad)?)
            .map_err(|_| StorageError::Malformed)?
            .to_owned();
        let final_leaf = std::str::from_utf8(d.get_bytes().map_err(bad)?)
            .map_err(|_| StorageError::Malformed)?
            .to_owned();
        let file_id =
            FileIdentity::decode(d.get_bytes().map_err(bad)?).ok_or(StorageError::Malformed)?;
        let written_chunks = d.get_u32().map_err(bad)?;
        let written_bytes = d.get_u64().map_err(bad)?;
        let halt = match d.get_u8().map_err(bad)? {
            0 => None,
            1 => Some(ExportHalt::Unavailable),
            2 => Some(ExportHalt::Changed),
            _ => return Err(StorageError::Malformed),
        };
        let journal = Self {
            folder,
            folder_id,
            staging,
            final_leaf,
            file_id,
            written_chunks,
            written_bytes,
            halt,
        };
        if !journal.valid() {
            return Err(StorageError::Malformed);
        }
        Ok(journal)
    }
    /// Structural bounds only. Whether the platform matches the running one is answered where
    /// the journal is used, as an Unavailable destination, never by faulting the store.
    pub(super) fn valid(&self) -> bool {
        !self.folder.is_empty()
            && self.folder.len() <= external::MAX_FOLDER_BYTES
            && self.folder_id.platform() == self.file_id.platform()
            && external::valid_leaf(&self.staging)
            && external::valid_leaf(&self.final_leaf)
            && self.staging.ends_with(external::STAGING_SUFFIX)
            && self.staging != self.final_leaf
            && self.written_bytes <= EXPORT_MAX_BYTES
            && (self.written_chunks == 0) == (self.written_bytes == 0)
    }
    fn folder_path(&self) -> Option<PathBuf> {
        external::decode_os(self.folder_id.platform(), &self.folder).map(PathBuf::from)
    }
}

/// Durable export state of one Download record. Keep records always carry `None`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) enum Export {
    #[default]
    None,
    Writing(ExportJournal),
    Published(ExportJournal),
}
impl Export {
    pub(super) fn journal(&self) -> Option<&ExportJournal> {
        match self {
            Self::None => None,
            Self::Writing(journal) | Self::Published(journal) => Some(journal),
        }
    }
    pub(super) fn encode(&self, e: &mut Encoder) {
        match self {
            Self::None => {
                e.put_u8(0);
            }
            Self::Writing(journal) => {
                e.put_u8(1);
                journal.encode(e);
            }
            Self::Published(journal) => {
                e.put_u8(2);
                journal.encode(e);
            }
        }
    }
    pub(super) fn decode(d: &mut Decoder<'_>) -> Result<Self, StorageError> {
        match d.get_u8().map_err(|_| StorageError::Malformed)? {
            0 => Ok(Self::None),
            1 => Ok(Self::Writing(ExportJournal::decode(d)?)),
            2 => Ok(Self::Published(ExportJournal::decode(d)?)),
            _ => Err(StorageError::Malformed),
        }
    }
    pub(super) fn state(&self) -> Option<ExportState> {
        match self {
            Self::None => None,
            Self::Writing(journal) => Some(ExportState::Writing(journal.clone())),
            Self::Published(journal) => Some(ExportState::Published(journal.clone())),
        }
    }
}

impl record::Record {
    /// Single-record export rules. Only a DownloadV2 record may carry a journal, only once its
    /// encrypted copy is Completed (or being deleted), and Published means every chunk was saved.
    pub(super) fn validate_export(&self) -> Result<(), StorageError> {
        let Some(journal) = self.export.journal() else {
            return Ok(());
        };
        let published = matches!(self.export, Export::Published(_));
        if self.encoding != record::RecordEncoding::DownloadV2
            || !matches!(self.phase, KeepPhase::Completed | KeepPhase::Deleting)
            || !journal.valid()
            || journal.written_chunks() > self.plan.chunks.len()
            || (published
                && (journal.written_chunks() != self.plan.chunks.len()
                    || journal.halt.is_some()
                    || self.user_paused))
        {
            return Err(StorageError::Malformed);
        }
        Ok(())
    }

    /// The single permitted encoding change for Downloads: V1 to V2 on the edge that starts an
    /// export. Every other successor keeps its encoding, so records without a journal stay
    /// readable by the released reader.
    pub(super) fn upgrades_for_export(&self, next: &Self) -> bool {
        self.encoding == record::RecordEncoding::DownloadV1
            && next.encoding == record::RecordEncoding::DownloadV2
            && self.export == Export::None
            && matches!(next.export, Export::Writing(_))
    }

    /// Journal transitions. Deleting never gains, edits or retires a journal: removal keeps it
    /// until the directory is released, so cleanup can still prove which file is ours. Clearing
    /// a halt or forgetting a journal requires an explicit intent change in the same publication.
    pub(super) fn validate_export_successor(&self, next: &Self) -> Result<(), StorageError> {
        let intent_changed = next.intent != self.intent;
        let completed = self.phase == KeepPhase::Completed && next.phase == KeepPhase::Completed;
        let allowed = if self.phase == KeepPhase::Deleting || next.phase == KeepPhase::Deleting {
            next.export == self.export
        } else {
            match (&self.export, &next.export) {
                (Export::None, Export::None) => true,
                (Export::None, Export::Writing(journal)) => {
                    completed
                        && next.encoding == record::RecordEncoding::DownloadV2
                        && journal.written_chunks == 0
                        && journal.halt.is_none()
                }
                (Export::Writing(old), Export::Writing(new)) => {
                    completed
                        && old.same_destination(new)
                        && (old.halt.is_none() || new.halt.is_some() || intent_changed)
                }
                (Export::Writing(old), Export::Published(new)) => {
                    completed && old == new && old.halt.is_none()
                }
                (Export::Writing(_) | Export::Published(_), Export::None) => {
                    completed && intent_changed
                }
                (Export::Published(old), Export::Published(new)) => old == new,
                (Export::Published(_), Export::Writing(_))
                | (Export::None, Export::Published(_)) => false,
            }
        };
        if allowed {
            Ok(())
        } else {
            Err(StorageError::Malformed)
        }
    }
}

/// Native-only view of a saved journal, used for preflight, reopen and cleanup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportState {
    Writing(ExportJournal),
    Published(ExportJournal),
}
impl ExportState {
    pub fn journal(&self) -> &ExportJournal {
        match self {
            Self::Writing(journal) | Self::Published(journal) => journal,
        }
    }
}

/// Renderer-safe projection: no paths, names or identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportView {
    None,
    Writing {
        saved_chunks: usize,
        halt: Option<ExportHalt>,
        /// A native export task registered for the current lease generation is still alive.
        active: bool,
    },
    Published,
}

/// Progress of one export session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportProgress {
    pub done: usize,
    pub total: usize,
    pub bytes: u64,
}

/// Typed outcome of an export step. Messages never contain paths or names.
#[derive(Debug)]
pub enum ExportError {
    /// A persisted halt; the caller records it through the journal CAS.
    Halted(ExportHalt),
    /// The destination volume is full. Nothing is persisted; resume after freeing space.
    Full,
    /// Another local failure. `retryable` failures may be retried a bounded number of times.
    Local {
        retryable: bool,
        reason: String,
    },
    /// This destination cannot carry a journal; a fresh save keeps the non-resumable writer.
    Unsupported,
    /// The lease, invocation or saved journal changed. Stop without effects.
    Inactive,
    Storage(StorageError),
}
impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Halted(ExportHalt::Unavailable) => f.write_str(
                "The folder or the partly saved file is unavailable. Try again, or save a new copy.",
            ),
            Self::Halted(ExportHalt::Changed) => f.write_str(
                "The partly saved file was changed or replaced. Save a new copy to continue.",
            ),
            Self::Full => {
                f.write_str("Not enough space in Downloads. Free some space, then resume saving.")
            }
            Self::Local { reason, .. } => write!(f, "Saving to Downloads failed: {reason}"),
            Self::Unsupported => {
                f.write_str("This folder cannot keep track of a partly saved file.")
            }
            Self::Inactive => f.write_str("The download is no longer active."),
            Self::Storage(error) => write!(f, "{error}"),
        }
    }
}
impl std::error::Error for ExportError {}
impl From<StorageError> for ExportError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

/// Classify an I/O failure on an identity-proven destination. Full is separate so the caller
/// writes no record on a full volume; unreachable conditions become a persisted halt.
fn local(error: io::Error) -> ExportError {
    match error.kind() {
        io::ErrorKind::StorageFull => ExportError::Full,
        io::ErrorKind::PermissionDenied
        | io::ErrorKind::NotFound
        | io::ErrorKind::NotADirectory
        | io::ErrorKind::ReadOnlyFilesystem => ExportError::Halted(ExportHalt::Unavailable),
        _ => ExportError::Local {
            retryable: true,
            reason: error.to_string(),
        },
    }
}

/// Plaintext layout bound from the original `DownloadReady` manifest. Every accepted chunk must
/// have exactly its declared length, and the whole stream must hash to the plaintext CID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPlan {
    sizes: Vec<u64>,
    total: u64,
    cid: Cid,
}
impl ExportPlan {
    pub fn from_manifest(manifest: &crate::FileManifest) -> Result<Self, StorageError> {
        let sizes: Vec<u64> = manifest.chunks.iter().map(|chunk| chunk.size).collect();
        let sum = sizes
            .iter()
            .try_fold(0u64, |sum, size| sum.checked_add(*size))
            .ok_or(StorageError::BlobSizeLimit)?;
        if sizes.is_empty()
            || sizes.len() > CHUNK_LIMIT
            || sizes
                .iter()
                .any(|size| *size == 0 || *size > crate::CHUNK_PAD_CEILING as u64)
            || sum != manifest.total_size
            || sum > EXPORT_MAX_BYTES
        {
            return Err(StorageError::BlobSizeLimit);
        }
        Ok(Self {
            sizes,
            total: sum,
            cid: manifest.plaintext_cid,
        })
    }
    #[cfg(test)]
    pub(super) fn for_test(chunks: &[&[u8]]) -> Self {
        Self {
            sizes: chunks.iter().map(|chunk| chunk.len() as u64).collect(),
            total: chunks.iter().map(|chunk| chunk.len() as u64).sum(),
            cid: Cid::of(&chunks.concat()),
        }
    }
    fn prefix(&self, chunks: usize) -> Option<u64> {
        (chunks <= self.sizes.len()).then(|| self.sizes[..chunks].iter().sum())
    }
}

/// Liveness token registered with the store while a native export task owns an [`ExportFile`].
/// The store keeps only a weak reference, so a finished, panicked or abandoned task can never
/// leave a Download looking like it is still saving.
#[derive(Debug, Clone)]
pub struct ExportPresence(Weak<()>);

/// Result of a preflight that retains no handle and modifies nothing.
#[derive(Debug)]
pub enum ExportProbe {
    Ready,
    /// The final name already holds our file (crash after the link, before Published).
    Published(PathBuf),
    Halt(ExportHalt),
    /// A transient failure; nothing is known either way.
    Local(String),
}

/// Result of reopening a saved journal for writing.
#[derive(Debug)]
pub enum ExportReopen {
    Ready(Box<ExportFile>),
    Published(PathBuf),
    Halt(ExportHalt),
}

/// Result of preparing publication.
#[derive(Debug)]
pub enum Publication {
    /// Persist this checkpoint (every chunk, and possibly a new final name) BEFORE linking.
    Ready(ExportJournal),
    /// The final name already holds our file (an earlier link whose Published transition was
    /// lost). Persist the every-chunk journal, then record Published; never link again.
    AlreadyPublished(ExportJournal, PathBuf),
}

enum Located {
    Ready(File, PathBuf),
    Published(PathBuf),
    Halt(ExportHalt),
}

/// Find a saved partial using only the sealed folder path and identities. Nothing is created,
/// truncated or deleted here.
fn locate(journal: &ExportJournal, write: bool) -> Result<Located, ExportError> {
    let Some(folder) = journal.folder_path() else {
        return Ok(Located::Halt(ExportHalt::Unavailable));
    };
    match external::folder_identity(&folder) {
        Ok(id) if id == journal.folder_id => {}
        Ok(_) => return Ok(Located::Halt(ExportHalt::Changed)),
        Err(error)
            if external::unreachable(&error) || error.kind() == io::ErrorKind::Unsupported =>
        {
            return Ok(Located::Halt(ExportHalt::Unavailable));
        }
        Err(error) => return Err(local(error)),
    }
    let published_or = |otherwise: ExportHalt| -> Result<Located, ExportError> {
        let path = folder.join(&journal.final_leaf);
        match external::open_leaf(&path, false) {
            Ok(Leaf::Opened(_, observed)) if observed.identity == journal.file_id => {
                Ok(Located::Published(path))
            }
            Ok(_) => Ok(Located::Halt(otherwise)),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                Ok(Located::Halt(otherwise))
            }
            Err(error) => Err(local(error)),
        }
    };
    match external::open_leaf(&folder.join(&journal.staging), write) {
        Ok(Leaf::Opened(file, observed)) => {
            if observed.identity != journal.file_id {
                Ok(Located::Halt(ExportHalt::Changed))
            } else if observed.links != 1 {
                // Link count 2 is our own completed link when the final name has our identity
                // (crash row 6). Any other extra name was made by someone else: never write.
                published_or(ExportHalt::Changed)
            } else {
                Ok(Located::Ready(file, folder))
            }
        }
        Ok(Leaf::Missing) => published_or(ExportHalt::Unavailable),
        Ok(Leaf::Foreign) => Ok(Located::Halt(ExportHalt::Changed)),
        // A file we cannot identify is never ours.
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            Ok(Located::Halt(ExportHalt::Changed))
        }
        Err(error) => match local(error) {
            ExportError::Halted(halt) => Ok(Located::Halt(halt)),
            other => Err(other),
        },
    }
}

/// Remove the saved partial only if its folder and file identity prove it is ours. The final name
/// is never touched; when staging and final are the same object only the staging name goes.
/// Anything that cannot be proven ours (missing folder, other platform, foreign file) is left in
/// place and counts as done, so removal is bounded and never blocks on an unreachable folder.
pub fn remove_partial(journal: &ExportJournal) -> Result<(), ExportError> {
    let Some(folder) = journal.folder_path() else {
        return Ok(());
    };
    if !matches!(external::folder_identity(&folder), Ok(id) if id == journal.folder_id) {
        return Ok(());
    }
    let staging = folder.join(&journal.staging);
    match external::open_leaf(&staging, false) {
        Ok(Leaf::Opened(file, observed)) if observed.identity == journal.file_id => {
            // Close our probe before deleting so Windows sharing rules cannot refuse it.
            let read_only = file
                .metadata()
                .is_ok_and(|metadata| metadata.permissions().readonly());
            drop(file);
            let mut removed = external::remove(&staging);
            #[cfg(windows)]
            if read_only
                && matches!(&removed, Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
            {
                // Windows refuses to delete a read-only file. This one is proven ours, so clear
                // only that attribute and try once more. (Unix unlink ignores the file mode.)
                if let Ok(metadata) = fs::metadata(&staging) {
                    let mut permissions = metadata.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    permissions.set_readonly(false);
                    if fs::set_permissions(&staging, permissions).is_ok() {
                        removed = external::remove(&staging);
                    }
                }
            }
            #[cfg(not(windows))]
            let _ = read_only;
            match removed {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(ExportError::Local {
                    retryable: true,
                    reason: error.to_string(),
                }),
            }
        }
        _ => Ok(()),
    }
}

/// Publication needs a no-replace hard link. FAT and exFAT give a usable-looking file index and
/// creation time but cannot link, so a journal there could never complete and would strand the
/// plaintext partial at "Saving interrupted". Prove linking works on this folder, with the fresh
/// staging file, before the journal names it: link it to an absent probe name, confirm the probe
/// is the same object, then remove only the probe name. Any failure means "do not journal".
fn links_supported(folder: &Path, staging: &str, observed: &external::Observed) -> bool {
    let source = folder.join(staging);
    for attempt in 0..4 {
        let probe = format!("{staging}-probe{attempt}");
        if !external::valid_leaf(&probe) {
            return false;
        }
        let target = folder.join(&probe);
        match external::link(&source, &target) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return false,
        }
        let ours = matches!(
            external::open_leaf(&target, false),
            Ok(Leaf::Opened(_, linked)) if linked.identity == observed.identity
        );
        if ours {
            // Only our just-made second name for our own file is removed here.
            return fs::remove_file(&target).is_ok();
        }
        return false;
    }
    false
}

/// Attempt to make a just-changed directory entry durable. Best effort by design: some network
/// filesystems refuse directory flushes, and a refusal must not strand a completed save. Native
/// calls this after releasing the session commit guard so a slow flush cannot delay Lock.
pub fn flush_destination(journal: &ExportJournal) {
    if let Some(folder) = journal.folder_path() {
        let _ = persistence::directory_barrier(&folder);
    }
}

/// One export session, owned by a native blocking worker. Dropping it closes the handle; the
/// partial file stays on disk with the journal so a later explicit Resume can continue it.
pub struct ExportFile {
    file: File,
    folder: PathBuf,
    journal: ExportJournal,
    base_name: String,
    plan: ExportPlan,
    next: usize,
    offset: u64,
    /// Bytes present when the session opened, capped at the declared size.
    verify_limit: u64,
    /// Bytes covered by the durable checkpoint when the session opened. A mismatch inside this
    /// range is never repaired: it means the file was changed, so the session halts.
    durable: u64,
    /// Set at the first byte the session writes; verification never resumes after that.
    diverged: bool,
    hasher: CidHasher,
    presence: Arc<()>,
}
impl std::fmt::Debug for ExportFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportFile")
            .field("next", &self.next)
            .field("offset", &self.offset)
            .finish_non_exhaustive()
    }
}

impl ExportFile {
    /// Start a fresh export in the freshly resolved Downloads `folder`, created if missing as the
    /// non-resumable writer does. Returns Unsupported when this destination cannot carry a
    /// journal; no file is left behind in that case.
    pub fn create(folder: &Path, name: &str, plan: ExportPlan) -> Result<Self, ExportError> {
        if external::CURRENT_PLATFORM == 0 || !folder.is_absolute() {
            return Err(ExportError::Unsupported);
        }
        let encoded = external::encode_os(folder.as_os_str());
        if encoded.is_empty() || encoded.len() > external::MAX_FOLDER_BYTES {
            return Err(ExportError::Unsupported);
        }
        if !external::valid_leaf(name) || !external::valid_leaf(&external::staging_for(name)) {
            return Err(ExportError::Storage(StorageError::Malformed));
        }
        fs::create_dir_all(folder).map_err(|error| match error.kind() {
            io::ErrorKind::StorageFull => ExportError::Full,
            _ => ExportError::Local {
                retryable: !external::unreachable(&error),
                reason: error.to_string(),
            },
        })?;
        let folder_id = external::folder_identity(folder).map_err(|_| ExportError::Unsupported)?;
        for number in 0..external::MAX_CANDIDATES {
            let final_leaf = external::numbered(name, number);
            let staging = external::staging_for(&final_leaf);
            if !external::valid_leaf(&final_leaf) || !external::valid_leaf(&staging) {
                return Err(ExportError::Storage(StorageError::Malformed));
            }
            if external::occupied(&folder.join(&final_leaf)).map_err(local)? {
                continue;
            }
            let path = folder.join(&staging);
            let file = match external::create_leaf(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(local(error)),
            };
            let observed = match external::observe(&file) {
                Ok(observed)
                    if observed.regular
                        && observed.links == 1
                        && links_supported(folder, &staging, &observed) =>
                {
                    observed
                }
                _ => {
                    // Created by this call a moment ago; remove it so an unsupported
                    // destination leaves nothing behind for the fallback writer to trip over.
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(ExportError::Unsupported);
                }
            };
            // Best effort: make the new directory entry durable before the journal names it.
            let _ = persistence::directory_barrier(folder);
            let journal = ExportJournal {
                folder: encoded,
                folder_id,
                staging,
                final_leaf,
                file_id: observed.identity,
                written_chunks: 0,
                written_bytes: 0,
                halt: None,
            };
            return Ok(Self::session(
                file,
                folder.to_owned(),
                journal,
                name,
                plan,
                0,
            ));
        }
        Err(ExportError::Local {
            retryable: false,
            reason: "too many downloads with this name already exist".into(),
        })
    }

    /// Preflight before an explicit Resume spends any allowance re-verifying the private copy.
    pub fn probe(journal: &ExportJournal) -> ExportProbe {
        // Ask for write access exactly as the reopen will, so a revoked permission is found
        // here, before any allowance is spent. Nothing is written through this handle.
        match locate(journal, true) {
            Ok(Located::Ready(..)) => ExportProbe::Ready,
            Ok(Located::Published(path)) => ExportProbe::Published(path),
            Ok(Located::Halt(halt)) => ExportProbe::Halt(halt),
            Err(ExportError::Halted(halt)) => ExportProbe::Halt(halt),
            Err(error) => ExportProbe::Local(error.to_string()),
        }
    }

    /// Reopen a saved partial for writing. `name` is this invocation's sanitized name, used only
    /// to choose a new final name if the saved one became occupied.
    pub fn reopen(
        journal: &ExportJournal,
        name: &str,
        plan: ExportPlan,
    ) -> Result<ExportReopen, ExportError> {
        if !external::valid_leaf(name) {
            return Err(ExportError::Storage(StorageError::Malformed));
        }
        let (file, folder) = match locate(journal, true)? {
            Located::Ready(file, folder) => (file, folder),
            Located::Published(path) => return Ok(ExportReopen::Published(path)),
            Located::Halt(halt) => return Ok(ExportReopen::Halt(halt)),
        };
        // The checkpoint must describe whole chunks of THIS manifest; equivalent encrypted
        // variants share one plaintext layout, so any disagreement means the journal is not ours.
        if plan.prefix(journal.written_chunks()) != Some(journal.written_bytes) {
            return Ok(ExportReopen::Halt(ExportHalt::Changed));
        }
        let present = file.metadata().map_err(local)?.len();
        Ok(ExportReopen::Ready(Box::new(Self::session(
            file,
            folder,
            journal.clone(),
            name,
            plan,
            present,
        ))))
    }

    fn session(
        file: File,
        folder: PathBuf,
        journal: ExportJournal,
        name: &str,
        plan: ExportPlan,
        present: u64,
    ) -> Self {
        Self {
            file,
            folder,
            durable: journal.written_bytes,
            verify_limit: present.min(plan.total),
            diverged: false,
            journal,
            base_name: name.to_owned(),
            plan,
            next: 0,
            offset: 0,
            hasher: CidHasher::new(),
            presence: Arc::new(()),
        }
    }

    pub fn journal(&self) -> &ExportJournal {
        &self.journal
    }
    pub fn presence(&self) -> ExportPresence {
        ExportPresence(Arc::downgrade(&self.presence))
    }
    pub fn next_index(&self) -> Option<usize> {
        (self.next < self.plan.sizes.len()).then_some(self.next)
    }
    pub fn progress(&self) -> ExportProgress {
        ExportProgress {
            done: self.next,
            total: self.plan.sizes.len(),
            bytes: self.offset,
        }
    }

    /// Verify or write chunk `index`, using plaintext regenerated from the retained encrypted
    /// copy. Inside the checkpoint a mismatch halts as Changed and nothing is modified; after
    /// it, the first short or mismatching chunk of our own identity-proven tail is truncated
    /// and rewritten. Written bytes are fsynced before this returns.
    pub fn accept(
        &mut self,
        index: usize,
        plaintext: &[u8],
    ) -> Result<ExportProgress, ExportError> {
        if self.next_index() != Some(index) || plaintext.len() as u64 != self.plan.sizes[index] {
            return Err(ExportError::Storage(StorageError::Malformed));
        }
        let end = self.offset + plaintext.len() as u64;
        if !self.diverged && end <= self.verify_limit {
            let mut held = vec![0; plaintext.len()];
            let read = self
                .file
                .seek(SeekFrom::Start(self.offset))
                .and_then(|_| self.file.read_exact(&mut held));
            match read {
                Ok(()) if held == plaintext => {
                    self.hasher.update(plaintext);
                    self.next += 1;
                    self.offset = end;
                    return Ok(self.progress());
                }
                Ok(()) if self.offset < self.durable => {
                    return Err(ExportError::Halted(ExportHalt::Changed));
                }
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {}
                Err(error) => return Err(local(error)),
            }
        }
        // From here on this session writes. A file shorter than its checkpoint (torn or lost
        // tail) rewinds to this whole-chunk boundary, which every earlier chunk just verified.
        self.diverged = true;
        let written = (|| {
            if self.file.metadata()?.len() != self.offset {
                self.file.set_len(self.offset)?;
            }
            self.file.seek(SeekFrom::Start(self.offset))?;
            external::inject(external::Effect::Write)?;
            self.file.write_all(plaintext)?;
            self.file.sync_all()
        })();
        written.map_err(local)?;
        self.hasher.update(plaintext);
        self.next += 1;
        self.offset = end;
        Ok(self.progress())
    }

    /// Flush and return the journal describing every chunk processed so far. The caller persists
    /// it through the checkpoint CAS; it never claims bytes that were not just made durable.
    pub fn checkpoint(&mut self) -> Result<ExportJournal, ExportError> {
        self.file.sync_all().map_err(local)?;
        let mut journal = self.journal.clone();
        journal.written_chunks = u32::try_from(self.next).map_err(|_| StorageError::Malformed)?;
        journal.written_bytes = self.offset;
        Ok(journal)
    }

    /// Mark a checkpoint as persisted so later checkpoints compare against it.
    pub fn checkpointed(&mut self, journal: ExportJournal) {
        self.journal = journal;
    }

    /// Check the whole plaintext, trim any trailing bytes past the declared size, flush, and
    /// choose the final name. The returned journal must be persisted before [`Self::link`].
    pub fn prepare_publication(&mut self) -> Result<Publication, ExportError> {
        if self.next_index().is_some()
            || self.offset != self.plan.total
            || self.hasher.cid() != self.plan.cid
        {
            return Err(ExportError::Storage(StorageError::Malformed));
        }
        let trimmed = (|| {
            if self.file.metadata()?.len() != self.offset {
                self.file.set_len(self.offset)?;
            }
            self.file.sync_all()
        })();
        trimmed.map_err(local)?;
        let mut journal = self.checkpoint()?;
        let current = self.folder.join(&journal.final_leaf);
        match external::open_leaf(&current, false) {
            Ok(Leaf::Opened(_, observed)) if observed.identity == journal.file_id => {
                return Ok(Publication::AlreadyPublished(journal, current));
            }
            Ok(Leaf::Missing) => {}
            Ok(_) => {
                // Occupied by something else since the save started. Choose the next absent
                // numbered name; it is persisted before any link attempt.
                let candidate = (0..external::MAX_CANDIDATES)
                    .map(|number| external::numbered(&self.base_name, number))
                    .find(|candidate| {
                        external::valid_leaf(candidate)
                            && *candidate != journal.staging
                            && *candidate != journal.final_leaf
                            && matches!(external::occupied(&self.folder.join(candidate)), Ok(false))
                    })
                    .ok_or_else(|| ExportError::Local {
                        retryable: false,
                        reason: "too many downloads with this name already exist".into(),
                    })?;
                journal.final_leaf = candidate;
            }
            Err(error) => return Err(local(error)),
        }
        Ok(Publication::Ready(journal))
    }

    /// Publish without replacement. Call only after the [`Publication::Ready`] journal is
    /// persisted, and under the native session commit guard so Remove/Pause/Lock cannot
    /// interleave. The final name is verified to hold our identity; it is never deleted.
    pub fn link(&self) -> Result<PathBuf, ExportError> {
        let staging = self.folder.join(&self.journal.staging);
        let target = self.folder.join(&self.journal.final_leaf);
        match external::open_leaf(&staging, false) {
            Ok(Leaf::Opened(_, observed))
                if observed.identity == self.journal.file_id && observed.links == 1 => {}
            Ok(Leaf::Missing) => return Err(ExportError::Halted(ExportHalt::Unavailable)),
            Ok(_) => return Err(ExportError::Halted(ExportHalt::Changed)),
            Err(error) => return Err(local(error)),
        }
        match external::link(&staging, &target) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(ExportError::Local {
                    retryable: true,
                    reason: "the saved name was taken by another file".into(),
                });
            }
            // Creation proved linking works here, so this folder changed underneath us. A retry
            // cannot fix it; stop as a persisted halt instead of a transient failure.
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                return Err(ExportError::Halted(ExportHalt::Unavailable));
            }
            Err(error) => return Err(local(error)),
        }
        match external::open_leaf(&target, false) {
            Ok(Leaf::Opened(_, observed)) if observed.identity == self.journal.file_id => {}
            // Someone swapped the staging name between our check and the link. The new name is
            // not ours to delete; report the change instead of publishing.
            _ => return Err(ExportError::Halted(ExportHalt::Changed)),
        }
        Ok(target)
    }

    /// The journal to persist as Published after a successful [`Self::link`].
    pub fn published_journal(&self) -> ExportJournal {
        self.journal.clone()
    }

    /// After Published: close the handle, then remove the staging name if it is still ours.
    /// Failure is harmless (it is a second name for the saved file) and is not reported.
    pub fn finish(self) {
        let journal = self.journal.clone();
        drop(self);
        let _ = remove_partial(&journal);
    }

    /// Remove a just-created partial whose first journal publication failed.
    pub fn discard(self) {
        self.finish();
    }
}

impl<R: CryptoRngCore> KeptBlobStore<R> {
    /// Exact export-lease fence for journal transitions: same mount, same transfer generation,
    /// same committed plaintext plan, verified in this mount, Completed and not paused/blocked.
    fn export_index(&self, lease: &DownloadExportLease) -> Result<usize, StorageError> {
        self.ready()?;
        if !lease.is_current() || !lease.mount.issued_by(&self.read_epoch) {
            return Err(StorageError::Malformed);
        }
        let index = self.download_index(lease.id)?;
        let transfer = &self.transfers[index];
        if !lease.guard.issued_by(&transfer.read_epoch)
            || transfer.poisoned
            || !transfer.checked
            || transfer.record.user_paused
            || transfer.record.blocked
            || transfer.record.phase != KeepPhase::Completed
            || transfer.record.committed_plan().map(|plan| plan.version) != Some(lease.version)
        {
            return Err(StorageError::Malformed);
        }
        Ok(index)
    }

    /// Native-only saved journal for preflight, reopen and cleanup.
    pub fn download_export_state(
        &self,
        id: DownloadId,
    ) -> Result<Option<ExportState>, StorageError> {
        let index = self.download_index(id)?;
        Ok(self.transfers[index].record.export.state())
    }

    /// Projection for the renderer status: no paths or identities.
    pub(super) fn export_view(&self, transfer: &persistence::Transfer<R>) -> ExportView {
        match &transfer.record.export {
            Export::None => ExportView::None,
            Export::Published(_) => ExportView::Published,
            Export::Writing(journal) => ExportView::Writing {
                saved_chunks: journal.written_chunks(),
                halt: journal.halt,
                active: transfer
                    .export_presence
                    .as_ref()
                    .is_some_and(|(guard, token)| {
                        token.strong_count() != 0
                            && guard.current()
                            && guard.issued_by(&transfer.read_epoch)
                    }),
            },
        }
    }

    fn register_presence(
        &mut self,
        index: usize,
        lease: &DownloadExportLease,
        presence: ExportPresence,
    ) {
        self.transfers[index].export_presence = Some((lease.guard.clone(), presence.0));
    }

    /// None -> Writing(0) for a fresh export: the only edge that upgrades DownloadV1 to V2.
    pub fn begin_download_export(
        &mut self,
        lease: &DownloadExportLease,
        journal: &ExportJournal,
        presence: ExportPresence,
    ) -> Result<(), StorageError> {
        let index = self.export_index(lease)?;
        let transfer = &mut self.transfers[index];
        if transfer.record.export != Export::None
            || journal.written_chunks != 0
            || journal.halt.is_some()
            || !journal.valid()
        {
            return Err(StorageError::Malformed);
        }
        let mut next = transfer.record.successor()?;
        next.encoding = record::RecordEncoding::DownloadV2;
        next.export = Export::Writing(journal.clone());
        transfer.publish(next, &self.metadata_key, &mut self.rng)?;
        self.register_presence(index, lease, presence);
        Ok(())
    }

    /// Register a native task that reopened the exact saved, unhalted journal. No record write.
    pub fn resume_download_export(
        &mut self,
        lease: &DownloadExportLease,
        journal: &ExportJournal,
        presence: ExportPresence,
    ) -> Result<(), StorageError> {
        let index = self.export_index(lease)?;
        if self.transfers[index].record.export != Export::Writing(journal.clone())
            || journal.halt.is_some()
        {
            return Err(StorageError::Malformed);
        }
        self.register_presence(index, lease, presence);
        Ok(())
    }

    /// Writing(expected) -> Writing(next) for the same destination: a progress checkpoint or a
    /// new final name persisted before a link attempt. Never sets or clears a halt.
    pub fn checkpoint_download_export(
        &mut self,
        lease: &DownloadExportLease,
        expected: &ExportJournal,
        next_journal: &ExportJournal,
    ) -> Result<(), StorageError> {
        let index = self.export_index(lease)?;
        let transfer = &mut self.transfers[index];
        if transfer.record.export != Export::Writing(expected.clone())
            || !expected.same_destination(next_journal)
            || next_journal.halt != expected.halt
            || next_journal.halt.is_some()
        {
            return Err(StorageError::Malformed);
        }
        if expected == next_journal {
            return Ok(());
        }
        let mut next = transfer.record.successor()?;
        next.export = Export::Writing(next_journal.clone());
        transfer.publish(next, &self.metadata_key, &mut self.rng)
    }

    /// Writing(expected, every chunk) -> Published after native proved the final name holds our
    /// file. Called under the native session commit guard, after the link.
    pub fn publish_download_export(
        &mut self,
        lease: &DownloadExportLease,
        expected: &ExportJournal,
    ) -> Result<(), StorageError> {
        let index = self.export_index(lease)?;
        self.mark_published(index, expected)
    }

    /// Lease-free recovery of crash row 6: the saved journal already covers every chunk and native
    /// proved, before any re-verification, that the final name holds our file.
    pub fn recover_download_export(
        &mut self,
        id: DownloadId,
        expected: &ExportJournal,
    ) -> Result<(), StorageError> {
        self.ready()?;
        let index = self.download_index(id)?;
        self.mark_published(index, expected)
    }

    fn mark_published(
        &mut self,
        index: usize,
        expected: &ExportJournal,
    ) -> Result<(), StorageError> {
        let transfer = &mut self.transfers[index];
        if transfer.poisoned
            || transfer.record.phase != KeepPhase::Completed
            || transfer.record.export != Export::Writing(expected.clone())
            || expected.halt.is_some()
            || expected.written_chunks() != transfer.record.plan.chunks.len()
        {
            return Err(StorageError::Malformed);
        }
        let mut next = transfer.record.successor()?;
        next.export = Export::Published(expected.clone());
        transfer.publish(next, &self.metadata_key, &mut self.rng)?;
        transfer.export_presence = None;
        Ok(())
    }

    /// Persist a typed halt by exact journal CAS. Lease-free so a preflight before re-verification
    /// can record it; it grants nothing and only stops work.
    pub fn halt_download_export(
        &mut self,
        id: DownloadId,
        expected: &ExportJournal,
        halt: ExportHalt,
    ) -> Result<(), StorageError> {
        self.ready()?;
        let index = self.download_index(id)?;
        let transfer = &mut self.transfers[index];
        if transfer.poisoned
            || transfer.record.phase != KeepPhase::Completed
            || transfer.record.export != Export::Writing(expected.clone())
        {
            return Err(StorageError::Malformed);
        }
        if expected.halt == Some(halt) {
            return Ok(());
        }
        let mut journal = expected.clone();
        journal.halt = Some(halt);
        let mut next = transfer.record.successor()?;
        next.export = Export::Writing(journal);
        transfer.export_presence = None;
        transfer.publish(next, &self.metadata_key, &mut self.rng)
    }

    /// "Save new copy" / "Save another copy": forget the journal under an explicit intent CAS.
    /// Native removes our partial (if provable) first. Retiring the read epoch here stops any
    /// in-flight export of the old journal at its next actor call.
    pub fn abandon_download_export(
        &mut self,
        id: DownloadId,
        expected_intent: u64,
    ) -> Result<u64, StorageError> {
        self.ready()?;
        let index = self.download_index(id)?;
        let transfer = &mut self.transfers[index];
        if transfer.poisoned
            || transfer.record.phase != KeepPhase::Completed
            || transfer.record.intent != expected_intent
            || transfer.record.export == Export::None
        {
            return Err(StorageError::Malformed);
        }
        let intent = expected_intent
            .checked_add(1)
            .ok_or(StorageError::Malformed)?;
        transfer.read_epoch.retire();
        transfer.checked = false;
        transfer.export_presence = None;
        let mut next = transfer.record.successor()?;
        next.export = Export::None;
        next.intent = intent;
        transfer.publish(next, &self.metadata_key, &mut self.rng)?;
        Ok(intent)
    }

    /// Finish a removal whose external cleanup already ran (or that the user chose to leave):
    /// release a Deleting record that still carries its journal.
    pub fn finish_download_cleanup(&mut self, id: DownloadId) -> Result<(), StorageError> {
        let index = self.download_index(id)?;
        if self.transfers[index].record.phase != KeepPhase::Deleting {
            return Err(StorageError::Malformed);
        }
        self.release_transfer(index)
    }
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
