//! Content-addressed blob storage.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};

use catcoms_crypto::{seal, unseal, SealedBlob};
use catcoms_rt::CryptoRngCore;
use zeroize::Zeroizing;

use crate::cid::Cid;
use crate::StorageError;

/// A content-addressed blob store. `put` returns the [`Cid`] of the bytes; `get`
/// re-verifies that what comes back matches the address (tamper detection).
///
/// Alongside the store proper there is a **staging area**: a separate namespace for blobs that
/// have been written but do not belong to anything yet. A multi-chunk upload seals into staging
/// and promotes the whole set once its manifest is published, so an upload that never finishes
/// leaves nothing in the store to account for. See [`put_staged`](BlobStore::put_staged).
pub trait BlobStore {
    /// Explicit local copies have a separate quota and ownership from fetched/uploaded cache
    /// bytes. Unsupported stores (especially memory/FIFO stores) never claim durable retention.
    fn kept_files(&self) -> crate::kept::KeptFiles {
        crate::kept::KeptFiles::default()
    }
    /// Reserve the complete exact plan before accepting any retained network bytes.
    fn begin_keep(&mut self, _plan: crate::kept::KeepPlan) -> Result<u64, StorageError> {
        Err(StorageError::Io(
            "kept copies require the encrypted disk store".into(),
        ))
    }
    /// Write only into the reservation selected by this process-local token.
    fn put_keep(&mut self, _token: u64, _bytes: &[u8]) -> Result<(), StorageError> {
        Err(StorageError::Malformed)
    }
    /// Called only after the application has verified the complete ordered plaintext CID.
    fn finish_keep(&mut self, _token: u64) -> Result<(), StorageError> {
        Err(StorageError::Malformed)
    }
    /// Cancel this exact reservation. Failed cleanup must not refund its capacity.
    fn abort_keep(&mut self, _token: u64) -> Result<(), StorageError> {
        Err(StorageError::Malformed)
    }
    /// Explicit local release; replicated expiry/unlisting never calls this operation.
    fn forget_kept(&mut self, _cid: &Cid) -> Result<(), StorageError> {
        Err(StorageError::Malformed)
    }
    /// Whether held bytes survive dropping this store. Publication APIs that promise a saved
    /// blob must fail closed on the default in-memory fallback after an attachment failure.
    fn is_persistent(&self) -> bool {
        false
    }

    /// Store `bytes`, returning their content address.
    fn put(&mut self, bytes: &[u8]) -> Result<Cid, StorageError>;

    /// Fetch the bytes for `cid`, verifying their integrity. `None` if absent.
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StorageError>;

    /// Integrity-checked read bounded before allocating/copying blob content. The bound is on
    /// plaintext bytes; sealing stores allow only their fixed envelope overhead in addition.
    fn get_bounded(&self, cid: &Cid, max_bytes: usize) -> Result<Option<Vec<u8>>, StorageError>;

    /// Whether the store currently holds `cid`.
    fn has(&self, cid: &Cid) -> bool;

    /// Remove a held copy of `cid`. Returns whether a copy was removed; a reference-protecting
    /// adapter may retain it (`false`) or refuse while its inventory is unknown. This is not
    /// evidence that no copy exists; use `has`/verified reads to determine availability.
    fn delete(&mut self, cid: &Cid) -> Result<bool, StorageError>;

    /// All currently-held content addresses.
    fn cids(&self) -> Vec<Cid>;

    /// Store `bytes` in the staging area rather than the store proper.
    ///
    /// A staged blob is invisible to [`get`](BlobStore::get), [`has`](BlobStore::has) and
    /// [`cids`](BlobStore::cids): it is not held content, it is content that might become held.
    /// That invisibility is the point. Chunks of an in-flight upload used to be written straight
    /// into the store, where the only record that they were unpublished was a map in memory, so a
    /// crash mid-upload left blobs no manifest named and no sweep could recognise. Anything left
    /// in staging is unambiguously abandoned, which makes cleanup a decision the store can make
    /// on its own at startup.
    fn put_staged(&mut self, bytes: &[u8]) -> Result<Cid, StorageError>;

    /// Move a staged blob into the store proper. `Ok(false)` if it was not staged.
    ///
    /// This is the commit point of an upload, and it is a move rather than a copy so that
    /// promoting a 256 MiB file does not mean rewriting it.
    fn promote_staged(&mut self, cid: &Cid) -> Result<bool, StorageError>;

    /// Verify and promote within a plaintext limit, then flush the held file before success.
    /// Both staged and existing held copies are read with the bound. Failure after promotion
    /// may leave held content; callers must not publish a reference or delete that shared copy.
    /// Memory stores provide process-local availability only. Filesystem stores sync the file
    /// and, on Unix, its containing directories (matching the vault's platform durability seam).
    fn promote_staged_bounded(&mut self, cid: &Cid, max_bytes: usize)
        -> Result<bool, StorageError>;

    /// Discard one staged blob. `Ok(false)` if it was not staged.
    ///
    /// Cannot touch held content, which is what makes cancelling an upload safe by construction:
    /// there is no dedup question to get wrong, because a staged blob is by definition referenced
    /// by nothing.
    fn drop_staged(&mut self, cid: &Cid) -> Result<bool, StorageError>;

    /// Discard everything in the staging area, returning how many blobs went. Run at startup:
    /// staged content that outlived the process that staged it can never be claimed, because the
    /// only thing that knew what it was for was that process.
    fn clear_staging(&mut self) -> Result<usize, StorageError>;
}

fn verify(cid: &Cid, bytes: Vec<u8>) -> Result<Vec<u8>, StorageError> {
    if Cid::of(&bytes) == *cid {
        Ok(bytes)
    } else {
        Err(StorageError::CidMismatch)
    }
}

/// Default in-memory blob budget (128 MiB). Bounds how much fetched content (avatars,
/// downloaded files) a node caches, so a member spamming large blobs cannot make peers grow
/// without bound. This is a simple **size-bounded FIFO** interim; evicted blobs stay
/// re-fetchable by CID; the full holder-probe retention engine (never-evict-last-copy) is
/// a follow-up.
pub const DEFAULT_BLOB_BUDGET: usize = 128 * 1024 * 1024;

/// An in-memory, size-bounded blob store (FIFO eviction past the byte budget).
#[derive(Debug)]
pub struct MemoryBlobStore {
    blobs: HashMap<Cid, Vec<u8>>,
    /// Written but not yet claimed by anything. Deliberately outside `blobs` and outside the
    /// eviction budget: staged content is not held content.
    staged: HashMap<Cid, Vec<u8>>,
    /// Insertion order, for FIFO eviction.
    order: VecDeque<Cid>,
    total_bytes: usize,
    max_bytes: usize,
}

impl Default for MemoryBlobStore {
    fn default() -> Self {
        Self::with_budget(DEFAULT_BLOB_BUDGET)
    }
}

impl MemoryBlobStore {
    /// A new, empty store with the [`DEFAULT_BLOB_BUDGET`].
    pub fn new() -> Self {
        Self::default()
    }

    /// A new, empty store bounded to `max_bytes` of blob content.
    pub fn with_budget(max_bytes: usize) -> Self {
        Self {
            blobs: HashMap::new(),
            staged: HashMap::new(),
            order: VecDeque::new(),
            total_bytes: 0,
            max_bytes,
        }
    }

    /// Evict the oldest blobs until under budget (keeping at least the most recent, so a
    /// single over-budget blob is still stored).
    fn evict_to_budget(&mut self) {
        while self.total_bytes > self.max_bytes && self.order.len() > 1 {
            if let Some(old) = self.order.pop_front() {
                if let Some(b) = self.blobs.remove(&old) {
                    self.total_bytes -= b.len();
                }
            }
        }
    }
}

impl BlobStore for MemoryBlobStore {
    fn put(&mut self, bytes: &[u8]) -> Result<Cid, StorageError> {
        let cid = Cid::of(bytes);
        if let Entry::Vacant(e) = self.blobs.entry(cid) {
            e.insert(bytes.to_vec()); // consumes the entry, ending the `blobs` borrow
            self.total_bytes += bytes.len();
            self.order.push_back(cid);
            self.evict_to_budget();
        }
        Ok(cid)
    }

    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StorageError> {
        match self.blobs.get(cid) {
            Some(bytes) => verify(cid, bytes.clone()).map(Some),
            None => Ok(None),
        }
    }

    fn has(&self, cid: &Cid) -> bool {
        self.blobs.contains_key(cid)
    }

    fn get_bounded(&self, cid: &Cid, max_bytes: usize) -> Result<Option<Vec<u8>>, StorageError> {
        match self.blobs.get(cid) {
            Some(bytes) if bytes.len() > max_bytes => Err(StorageError::BlobSizeLimit),
            Some(bytes) => verify(cid, bytes.clone()).map(Some),
            None => Ok(None),
        }
    }

    fn delete(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        if let Some(b) = self.blobs.remove(cid) {
            self.total_bytes -= b.len();
            self.order.retain(|c| c != cid);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn cids(&self) -> Vec<Cid> {
        self.blobs.keys().copied().collect()
    }

    fn put_staged(&mut self, bytes: &[u8]) -> Result<Cid, StorageError> {
        let cid = Cid::of(bytes);
        self.staged.entry(cid).or_insert_with(|| bytes.to_vec());
        Ok(cid)
    }

    fn promote_staged(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        match self.staged.remove(cid) {
            Some(bytes) => {
                self.put(&bytes)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn drop_staged(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        Ok(self.staged.remove(cid).is_some())
    }

    fn promote_staged_bounded(
        &mut self,
        cid: &Cid,
        max_bytes: usize,
    ) -> Result<bool, StorageError> {
        let Some(bytes) = self.staged.get(cid) else {
            return Ok(false);
        };
        if bytes.len() > max_bytes {
            return Err(StorageError::BlobSizeLimit);
        }
        verify(cid, bytes.clone())?;
        self.promote_staged(cid)
    }

    fn clear_staging(&mut self) -> Result<usize, StorageError> {
        let n = self.staged.len();
        self.staged.clear();
        Ok(n)
    }
}

/// Subdirectory holding blobs written but not yet claimed by anything.
///
/// A real subdirectory rather than a filename suffix: it cannot collide with a content address
/// (no CID hex is the word "staging"), the store's own `cids()` scan skips it for free because the
/// name does not parse as a CID, and clearing the whole staging area is one directory walk rather
/// than a pattern match over every file in the store.
const STAGING_DIR: &str = "staging";

/// Inspect the opened file before reading. `take` also bounds a file that grows after metadata
/// is checked. Never reserve from an untrusted on-disk length or fall back to `fs::read`.
pub(crate) fn read_bounded(path: &Path, max_bytes: usize) -> Result<Option<Vec<u8>>, StorageError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StorageError::Io(e.to_string())),
    };
    let metadata = file
        .metadata()
        .map_err(|e| StorageError::Io(e.to_string()))?;
    if !metadata.is_file() {
        return Err(StorageError::Malformed);
    }
    if metadata.len() > max_bytes as u64 {
        return Err(StorageError::BlobSizeLimit);
    }
    let mut bytes = Vec::new();
    file.take((max_bytes as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| StorageError::Io(e.to_string()))?;
    if bytes.len() > max_bytes {
        return Err(StorageError::BlobSizeLimit);
    }
    Ok(Some(bytes))
}

fn flush_promoted(dir: &Path, cid: &Cid) -> Result<(), StorageError> {
    // Write access is required for FlushFileBuffers on Windows. No content is changed here.
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.join(cid.to_hex()))
        .and_then(|file| file.sync_all())
        .map_err(|e| StorageError::CommittedButNotDurable(e.to_string()))?;
    #[cfg(unix)]
    for path in [dir.to_path_buf(), dir.join(STAGING_DIR)] {
        std::fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|e| StorageError::CommittedButNotDurable(e.to_string()))?;
    }
    Ok(())
}

/// Establish the namespace before accepting a persistent store. File/leaf-directory fsync alone
/// does not persist a freshly-created group directory's entry in its parent. Sync all ancestors
/// on open, including retries where a previous failed open already made the directories visible.
/// Windows retains the existing vault's file-flush-only platform guarantee.
pub(crate) fn create_blob_directory(dir: &Path) -> Result<(), StorageError> {
    create_blob_directory_with_sync(dir, |path| {
        #[cfg(unix)]
        {
            std::fs::File::open(path)?.sync_all()
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(())
        }
    })
}

fn create_blob_directory_with_sync(
    dir: &Path,
    mut sync: impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<(), StorageError> {
    std::fs::create_dir_all(dir).map_err(|e| StorageError::Io(e.to_string()))?;
    let absolute = std::fs::canonicalize(dir).map_err(|e| StorageError::Io(e.to_string()))?;
    for ancestor in absolute.ancestors() {
        sync(ancestor).map_err(|e| StorageError::Io(e.to_string()))?;
    }
    Ok(())
}

fn staged_path(dir: &Path, cid: &Cid) -> PathBuf {
    dir.join(STAGING_DIR).join(cid.to_hex())
}

/// Create the staging directory and return the path a staged blob should be written to.
fn staged_target(dir: &Path, cid: &Cid) -> Result<PathBuf, StorageError> {
    std::fs::create_dir_all(dir.join(STAGING_DIR)).map_err(|e| StorageError::Io(e.to_string()))?;
    Ok(staged_path(dir, cid))
}

/// Move a staged file into the store proper, or drop it if the store already holds a healthy copy.
///
/// A rename, so promoting a 256 MiB upload costs a directory entry rather than a rewrite. The
/// `healthy` flag is the caller's `get` result for the destination: content addressing makes an
/// existing valid record identical to this one, so the staged duplicate is discarded instead of
/// replacing a file another reader may have open.
fn promote_staged_file(dir: &Path, cid: &Cid, healthy: bool) -> Result<bool, StorageError> {
    let from = staged_path(dir, cid);
    if !from.exists() {
        return Ok(false);
    }
    if healthy {
        std::fs::remove_file(&from).map_err(|e| StorageError::Io(e.to_string()))?;
        return Ok(true);
    }
    std::fs::rename(&from, dir.join(cid.to_hex())).map_err(|e| StorageError::Io(e.to_string()))?;
    Ok(true)
}

fn drop_staged_file(dir: &Path, cid: &Cid) -> Result<bool, StorageError> {
    match std::fs::remove_file(staged_path(dir, cid)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(StorageError::Io(e.to_string())),
    }
}

fn clear_staging_dir(dir: &Path) -> Result<usize, StorageError> {
    let Ok(entries) = std::fs::read_dir(dir.join(STAGING_DIR)) else {
        return Ok(0); // never staged anything, or already gone
    };
    let mut dropped = 0;
    for entry in entries.flatten() {
        if std::fs::remove_file(entry.path()).is_ok() {
            dropped += 1;
        }
    }
    Ok(dropped)
}

/// A filesystem blob store: each blob is a file named by its hex CID.
#[derive(Debug)]
pub struct FsBlobStore {
    dir: PathBuf,
}

impl FsBlobStore {
    // The injected flush exists to exercise the committed-but-not-durable boundary without
    // filesystem-specific permission tricks. Production always supplies `flush_promoted`.
    fn promote_bounded_with_flush(
        &mut self,
        cid: &Cid,
        max_bytes: usize,
        flush: impl FnOnce(&Path, &Cid) -> Result<(), StorageError>,
    ) -> Result<bool, StorageError> {
        let Some(bytes) = read_bounded(&staged_path(&self.dir, cid), max_bytes)? else {
            return Ok(false);
        };
        verify(cid, bytes)?;
        let healthy = matches!(self.get_bounded(cid, max_bytes), Ok(Some(_)));
        if !promote_staged_file(&self.dir, cid, healthy)? {
            return Ok(false);
        }
        self.get_bounded(cid, max_bytes)?
            .ok_or(StorageError::Malformed)?;
        flush(&self.dir, cid)?;
        Ok(true)
    }

    /// Open (creating if needed) a blob store rooted at `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, StorageError> {
        let dir = dir.as_ref().to_path_buf();
        create_blob_directory(&dir)?;
        Ok(Self { dir })
    }

    fn path(&self, cid: &Cid) -> PathBuf {
        self.dir.join(cid.to_hex())
    }
}

impl BlobStore for FsBlobStore {
    fn is_persistent(&self) -> bool {
        true
    }

    fn put(&mut self, bytes: &[u8]) -> Result<Cid, StorageError> {
        let cid = Cid::of(bytes);
        let path = self.path(&cid);
        // Content addressing makes a healthy existing record safely deduplicable, but mere path
        // existence is not health: a truncated/tampered record must be replaceable after a peer
        // has served the CID-verified bytes again. We only overwrite after `get` has proved that
        // the existing record is absent or invalid, so the repair path never discards a valid
        // local copy.
        if !matches!(self.get_bounded(&cid, bytes.len()), Ok(Some(_))) {
            std::fs::write(&path, bytes).map_err(|e| StorageError::Io(e.to_string()))?;
        }
        Ok(cid)
    }

    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StorageError> {
        let path = self.path(cid);
        match std::fs::read(&path) {
            Ok(bytes) => verify(cid, bytes).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StorageError::Io(e.to_string())),
        }
    }

    fn has(&self, cid: &Cid) -> bool {
        self.path(cid).exists()
    }

    fn get_bounded(&self, cid: &Cid, max_bytes: usize) -> Result<Option<Vec<u8>>, StorageError> {
        read_bounded(&self.path(cid), max_bytes)?
            .map(|bytes| verify(cid, bytes))
            .transpose()
    }

    fn promote_staged_bounded(
        &mut self,
        cid: &Cid,
        max_bytes: usize,
    ) -> Result<bool, StorageError> {
        self.promote_bounded_with_flush(cid, max_bytes, flush_promoted)
    }

    fn delete(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        let path = self.path(cid);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(StorageError::Io(e.to_string())),
        }
    }

    fn cids(&self) -> Vec<Cid> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|name| Cid::from_hex(&name))
            .collect()
    }

    fn put_staged(&mut self, bytes: &[u8]) -> Result<Cid, StorageError> {
        let cid = Cid::of(bytes);
        let path = staged_target(&self.dir, &cid)?;
        std::fs::write(&path, bytes).map_err(|e| StorageError::Io(e.to_string()))?;
        Ok(cid)
    }

    fn promote_staged(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        let healthy = matches!(self.get(cid), Ok(Some(_)));
        promote_staged_file(&self.dir, cid, healthy)
    }

    fn drop_staged(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        drop_staged_file(&self.dir, cid)
    }

    fn clear_staging(&mut self) -> Result<usize, StorageError> {
        clear_staging_dir(&self.dir)
    }
}

/// A filesystem blob store that **seals each blob at rest** (Phase 9b). Externally it is a
/// normal [`BlobStore`] keyed by the **plaintext** content address; so the mesh fetch (which
/// addresses blobs by plaintext CID) is unchanged; but on disk every blob is XChaCha20-
/// Poly1305-sealed under a key (the keystore's `blob_key`). A stolen disk yields only
/// ciphertext; the sealing is at the disk boundary, not on the wire. The file is named by
/// the plaintext CID; its contents are `nonce ‖ sealed-plaintext`. Both the AEAD tag and a
/// re-hash of the plaintext (against the requested CID) are checked on read.
pub struct SealingBlobStore<R: CryptoRngCore> {
    dir: PathBuf,
    key: Zeroizing<[u8; 32]>,
    rng: R,
}

impl<R: CryptoRngCore> SealingBlobStore<R> {
    fn read_sealed_bounded(
        &self,
        path: &Path,
        cid: &Cid,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        // XChaCha20-Poly1305 disk framing is nonce(24) + plaintext + tag(16).
        let disk_limit = max_bytes
            .checked_add(40)
            .ok_or(StorageError::BlobSizeLimit)?;
        let Some(encoded) = read_bounded(path, disk_limit)? else {
            return Ok(None);
        };
        let plaintext = unseal(&self.key, &decode_sealed(&encoded)?)?;
        if plaintext.len() > max_bytes {
            return Err(StorageError::BlobSizeLimit);
        }
        verify(cid, plaintext).map(Some)
    }

    /// Open (creating if needed) a sealing store rooted at `dir`, sealing under `key`.
    pub fn open(dir: impl AsRef<Path>, key: [u8; 32], rng: R) -> Result<Self, StorageError> {
        let dir = dir.as_ref().to_path_buf();
        create_blob_directory(&dir)?;
        Ok(Self {
            dir,
            key: Zeroizing::new(key),
            rng,
        })
    }

    fn path(&self, cid: &Cid) -> PathBuf {
        self.dir.join(cid.to_hex())
    }
}

// Manual Debug (for any `R`) that redacts the sealing key and the RNG.
impl<R: CryptoRngCore> std::fmt::Debug for SealingBlobStore<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealingBlobStore")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

/// Frame a sealed blob for disk: `nonce(24) ‖ ciphertext`.
pub(crate) fn encode_sealed(s: &SealedBlob) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.nonce.len() + s.ciphertext.len());
    out.extend_from_slice(&s.nonce);
    out.extend_from_slice(&s.ciphertext);
    out
}

/// Parse a sealed blob from disk bytes.
pub(crate) fn decode_sealed(bytes: &[u8]) -> Result<SealedBlob, StorageError> {
    if bytes.len() <= 24 {
        return Err(StorageError::Malformed);
    }
    let nonce: [u8; 24] = bytes[..24].try_into().expect("length checked");
    Ok(SealedBlob {
        nonce,
        ciphertext: bytes[24..].to_vec(),
    })
}

impl<R: CryptoRngCore> BlobStore for SealingBlobStore<R> {
    fn is_persistent(&self) -> bool {
        true
    }

    fn put(&mut self, bytes: &[u8]) -> Result<Cid, StorageError> {
        let cid = Cid::of(bytes);
        let path = self.path(&cid);
        // A corrupt sealed file still has the expected CID filename. Validate the record before
        // deduplicating so an authenticated peer fetch can replace it with freshly sealed bytes.
        if !matches!(self.get_bounded(&cid, bytes.len()), Ok(Some(_))) {
            let sealed = seal(&self.key, bytes, &mut self.rng)?;
            std::fs::write(&path, encode_sealed(&sealed))
                .map_err(|e| StorageError::Io(e.to_string()))?;
        }
        Ok(cid)
    }

    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StorageError> {
        let path = self.path(cid);
        match std::fs::read(&path) {
            Ok(encoded) => {
                let sealed = decode_sealed(&encoded)?;
                let plaintext = unseal(&self.key, &sealed)?;
                verify(cid, plaintext).map(Some)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StorageError::Io(e.to_string())),
        }
    }

    fn get_bounded(&self, cid: &Cid, max_bytes: usize) -> Result<Option<Vec<u8>>, StorageError> {
        self.read_sealed_bounded(&self.path(cid), cid, max_bytes)
    }

    fn promote_staged_bounded(
        &mut self,
        cid: &Cid,
        max_bytes: usize,
    ) -> Result<bool, StorageError> {
        if self
            .read_sealed_bounded(&staged_path(&self.dir, cid), cid, max_bytes)?
            .is_none()
        {
            return Ok(false);
        }
        let healthy = matches!(self.get_bounded(cid, max_bytes), Ok(Some(_)));
        if !promote_staged_file(&self.dir, cid, healthy)? {
            return Ok(false);
        }
        self.get_bounded(cid, max_bytes)?
            .ok_or(StorageError::Malformed)?;
        flush_promoted(&self.dir, cid)?;
        Ok(true)
    }

    fn has(&self, cid: &Cid) -> bool {
        self.path(cid).exists()
    }

    fn delete(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        let path = self.path(cid);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(StorageError::Io(e.to_string())),
        }
    }

    fn cids(&self) -> Vec<Cid> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|name| Cid::from_hex(&name))
            .collect()
    }

    fn put_staged(&mut self, bytes: &[u8]) -> Result<Cid, StorageError> {
        // Sealed exactly as a held blob is: promotion is a rename, so a staged blob has to already
        // be in its final on-disk form.
        let cid = Cid::of(bytes);
        let path = staged_target(&self.dir, &cid)?;
        let sealed = seal(&self.key, bytes, &mut self.rng)?;
        std::fs::write(&path, encode_sealed(&sealed))
            .map_err(|e| StorageError::Io(e.to_string()))?;
        Ok(cid)
    }

    fn promote_staged(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        let healthy = matches!(self.get(cid), Ok(Some(_)));
        promote_staged_file(&self.dir, cid, healthy)
    }

    fn drop_staged(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        drop_staged_file(&self.dir, cid)
    }

    fn clear_staging(&mut self) -> Result<usize, StorageError> {
        clear_staging_dir(&self.dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn bounded_contract(store: &mut dyn BlobStore) {
        let cid = store.put_staged(b"pixels").unwrap();
        assert_eq!(store.get_bounded(&cid, 6).unwrap(), None);
        assert!(matches!(
            store.promote_staged_bounded(&cid, 5),
            Err(StorageError::BlobSizeLimit)
        ));
        assert!(!store.has(&cid));
        assert!(store.promote_staged_bounded(&cid, 6).unwrap());
        assert_eq!(store.get_bounded(&cid, 6).unwrap().unwrap(), b"pixels");
        assert!(matches!(
            store.get_bounded(&cid, 5),
            Err(StorageError::BlobSizeLimit)
        ));
        assert!(!store.promote_staged_bounded(&cid, 6).unwrap());
        store.put_staged(b"pixels").unwrap();
        assert!(
            store.promote_staged_bounded(&cid, 6).unwrap(),
            "duplicate publication flushes too"
        );
        assert_eq!(store.clear_staging().unwrap(), 0);
        let empty = store.put(b"").unwrap();
        assert_eq!(store.get_bounded(&empty, 0).unwrap(), Some(vec![]));
    }

    #[test]
    fn bounded_blob_contract_all_stores_and_reopen() {
        bounded_contract(&mut MemoryBlobStore::new());
        let raw = tempfile::tempdir().unwrap();
        bounded_contract(&mut FsBlobStore::open(raw.path()).unwrap());
        let sealed = tempfile::tempdir().unwrap();
        bounded_contract(
            &mut SealingBlobStore::open(sealed.path(), [7; 32], ChaCha20Rng::seed_from_u64(1))
                .unwrap(),
        );
        let cid = Cid::of(b"pixels");
        assert_eq!(
            FsBlobStore::open(raw.path())
                .unwrap()
                .get_bounded(&cid, 6)
                .unwrap()
                .unwrap(),
            b"pixels"
        );
        assert_eq!(
            SealingBlobStore::open(sealed.path(), [7; 32], ChaCha20Rng::seed_from_u64(2))
                .unwrap()
                .get_bounded(&cid, 6)
                .unwrap()
                .unwrap(),
            b"pixels"
        );
    }

    #[test]
    fn bounded_promotion_repairs_oversized_destinations_without_reading_them() {
        // A sparse corrupt file makes an accidental unbounded read particularly costly. Both
        // promotion and ordinary fetched-blob dedup must use the incoming plaintext bound.
        for sealed in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut store: Box<dyn BlobStore> = if sealed {
                Box::new(
                    SealingBlobStore::open(dir.path(), [7; 32], ChaCha20Rng::seed_from_u64(1))
                        .unwrap(),
                )
            } else {
                Box::new(FsBlobStore::open(dir.path()).unwrap())
            };
            let cid = store.put_staged(b"pixels").unwrap();
            let path = dir.path().join(cid.to_hex());
            std::fs::File::create(&path)
                .unwrap()
                .set_len(128 * 1024 * 1024)
                .unwrap();
            assert!(matches!(
                store.get_bounded(&cid, 6),
                Err(StorageError::BlobSizeLimit)
            ));
            assert!(store.promote_staged_bounded(&cid, 6).unwrap());
            assert_eq!(store.get_bounded(&cid, 6).unwrap().unwrap(), b"pixels");
            std::fs::File::create(&path)
                .unwrap()
                .set_len(128 * 1024 * 1024)
                .unwrap();
            store.put(b"pixels").unwrap();
            assert_eq!(store.get_bounded(&cid, 6).unwrap().unwrap(), b"pixels");
        }
    }

    #[test]
    fn bounded_promotion_rejects_corrupt_staging_and_preserves_held_copy() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FsBlobStore::open(dir.path()).unwrap();
        let cid = store.put(b"pixels").unwrap();
        store.put_staged(b"pixels").unwrap();
        std::fs::write(staged_path(dir.path(), &cid), b"wrong!").unwrap();
        assert!(matches!(
            store.promote_staged_bounded(&cid, 6),
            Err(StorageError::CidMismatch)
        ));
        assert_eq!(store.get_bounded(&cid, 6).unwrap().unwrap(), b"pixels");
    }

    #[test]
    fn bounded_store_creation_syncs_ancestors_even_after_failed_open() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("blobs").join("group");
        assert!(
            create_blob_directory_with_sync(&dir, |_| Err(std::io::Error::other("sync failed")))
                .is_err()
        );
        assert!(
            dir.exists(),
            "failed open may leave visible directories but no store authority"
        );
        let mut synced = Vec::new();
        create_blob_directory_with_sync(&dir, |path| {
            synced.push(path.to_path_buf());
            Ok(())
        })
        .unwrap();
        let absolute = std::fs::canonicalize(&dir).unwrap();
        assert_eq!(
            synced,
            absolute
                .ancestors()
                .map(Path::to_path_buf)
                .collect::<Vec<_>>()
        );
        assert!(!MemoryBlobStore::new().is_persistent());
        assert!(FsBlobStore::open(&dir).unwrap().is_persistent());
    }

    #[test]
    fn bounded_promotion_flush_failure_preserves_held_bytes_and_retries_after_reopen() {
        for duplicate in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut store = FsBlobStore::open(dir.path()).unwrap();
            if duplicate {
                store.put(b"pixels").unwrap();
            }
            let cid = store.put_staged(b"pixels").unwrap();
            let error = store
                .promote_bounded_with_flush(&cid, 6, |root, cid| {
                    assert_eq!(std::fs::read(root.join(cid.to_hex())).unwrap(), b"pixels");
                    Err(StorageError::CommittedButNotDurable(
                        "injected flush".into(),
                    ))
                })
                .unwrap_err();
            assert!(matches!(error, StorageError::CommittedButNotDurable(_)));
            assert_eq!(store.get_bounded(&cid, 6).unwrap().unwrap(), b"pixels");
            assert_eq!(
                store.clear_staging().unwrap(),
                0,
                "promotion happened, not a rollback"
            );
            drop(store);
            let mut reopened = FsBlobStore::open(dir.path()).unwrap();
            reopened.put_staged(b"pixels").unwrap();
            assert!(reopened.promote_staged_bounded(&cid, 6).unwrap());
            assert_eq!(reopened.get_bounded(&cid, 6).unwrap().unwrap(), b"pixels");
        }
    }

    fn store_roundtrip(store: &mut dyn BlobStore) {
        let cid = store.put(b"some bytes").unwrap();
        assert_eq!(cid, Cid::of(b"some bytes"));
        assert!(store.has(&cid));
        assert_eq!(
            store.get(&cid).unwrap().as_deref(),
            Some(&b"some bytes"[..])
        );
        assert_eq!(store.cids(), vec![cid]);
        assert!(store.delete(&cid).unwrap());
        assert!(!store.has(&cid));
        assert_eq!(store.get(&cid).unwrap(), None);
    }

    /// The whole staging contract, run against every backend so they cannot drift.
    fn staging_contract(store: &mut dyn BlobStore) {
        // Staged content is on disk but is not held content. Everything that answers "do I have
        // this?" must say no, or an unfinished upload starts looking like a real file.
        let cid = store.put_staged(b"chunk of an upload").unwrap();
        assert_eq!(cid, Cid::of(b"chunk of an upload"));
        assert!(!store.has(&cid), "staged is not held");
        assert_eq!(store.get(&cid).unwrap(), None, "and cannot be read as held");
        assert!(store.cids().is_empty(), "nor listed in the inventory");

        // Promotion is what makes it real, and it is a move: nothing stays behind to sweep.
        assert!(store.promote_staged(&cid).unwrap());
        assert!(store.has(&cid));
        assert_eq!(
            store.get(&cid).unwrap().as_deref(),
            Some(&b"chunk of an upload"[..])
        );
        assert_eq!(store.cids(), vec![cid]);
        assert!(!store.promote_staged(&cid).unwrap(), "nothing left staged");
        assert_eq!(store.clear_staging().unwrap(), 0);

        // Dropping a staged blob cannot touch held content. This is the property that makes
        // cancelling an upload safe without a dedup check: the two namespaces are disjoint.
        let staged_again = store.put_staged(b"chunk of an upload").unwrap();
        assert_eq!(staged_again, cid, "same content, same address");
        assert!(store.drop_staged(&cid).unwrap());
        assert!(store.has(&cid), "the promoted copy survived the drop");
        assert!(!store.drop_staged(&cid).unwrap(), "already gone");

        // The startup sweep: whatever is staged when the process starts is unclaimable.
        store.put_staged(b"one").unwrap();
        store.put_staged(b"two").unwrap();
        assert_eq!(store.clear_staging().unwrap(), 2);
        assert!(store.has(&cid), "and it still leaves held content alone");
        assert_eq!(
            store.clear_staging().unwrap(),
            0,
            "sweeping twice is a no-op"
        );
    }

    #[test]
    fn memory_store_honours_the_staging_contract() {
        staging_contract(&mut MemoryBlobStore::new());
    }

    #[test]
    fn fs_store_honours_the_staging_contract() {
        let dir = tempfile::tempdir().unwrap();
        staging_contract(&mut FsBlobStore::open(dir.path()).unwrap());
    }

    #[test]
    fn sealing_store_honours_the_staging_contract() {
        let dir = tempfile::tempdir().unwrap();
        let store = SealingBlobStore::open(dir.path(), [3u8; 32], ChaCha20Rng::seed_from_u64(1));
        staging_contract(&mut store.unwrap());
    }

    #[test]
    fn staged_chunks_do_not_survive_reopening_the_store() {
        // The crash case, as the vault actually sees it: a process staged chunks and died. The
        // next process opens the same directory, finds them, and can say for certain that nothing
        // will ever claim them, because the only thing that knew what they were for is gone.
        let dir = tempfile::tempdir().unwrap();
        let key = [7u8; 32];
        let published = {
            let mut store =
                SealingBlobStore::open(dir.path(), key, ChaCha20Rng::seed_from_u64(2)).unwrap();
            let published = store.put(b"a file that was published").unwrap();
            store.put_staged(b"an upload that was interrupted").unwrap();
            store.put_staged(b"its second chunk").unwrap();
            published
        };

        let mut reopened =
            SealingBlobStore::open(dir.path(), key, ChaCha20Rng::seed_from_u64(3)).unwrap();
        assert_eq!(
            reopened.cids(),
            vec![published],
            "staged never counted as held"
        );
        assert_eq!(
            reopened.clear_staging().unwrap(),
            2,
            "both were still on disk"
        );
        assert!(reopened.has(&published), "the published file is untouched");
        assert_eq!(
            reopened.get(&published).unwrap().as_deref(),
            Some(&b"a file that was published"[..])
        );
    }

    #[test]
    fn memory_store_roundtrips() {
        store_roundtrip(&mut MemoryBlobStore::new());
    }

    #[test]
    fn memory_store_evicts_oldest_over_budget() {
        let mut store = MemoryBlobStore::with_budget(20);
        let a = store.put(b"aaaaaaaaaa").unwrap(); // 10 bytes
        let b = store.put(b"bbbbbbbbbb").unwrap(); // 10 bytes; total 20, at budget
        assert!(store.has(&a) && store.has(&b));
        let c = store.put(b"cccccccccc").unwrap(); // 10 bytes; over budget, evicts oldest (a)
        assert!(!store.has(&a), "the oldest blob is evicted past the budget");
        assert!(store.has(&b) && store.has(&c));
        // Re-putting an existing blob does not double-count toward the budget.
        store.put(b"cccccccccc").unwrap();
        assert!(store.has(&b) && store.has(&c));
        // Deleting frees budget so a new blob does not evict.
        assert!(store.delete(&b).unwrap());
        let d = store.put(b"dddddddddd").unwrap();
        assert!(store.has(&c) && store.has(&d));
    }

    #[test]
    fn fs_store_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        store_roundtrip(&mut FsBlobStore::open(dir.path()).unwrap());
    }

    #[test]
    fn fs_store_detects_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FsBlobStore::open(dir.path()).unwrap();
        let cid = store.put(b"original").unwrap();
        // Corrupt the file on disk under its CID name.
        std::fs::write(dir.path().join(cid.to_hex()), b"tampered!").unwrap();
        assert!(matches!(store.get(&cid), Err(StorageError::CidMismatch)));
    }

    #[test]
    fn fs_store_put_repairs_a_corrupt_existing_record() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FsBlobStore::open(dir.path()).unwrap();
        let cid = store.put(b"original").unwrap();
        std::fs::write(dir.path().join(cid.to_hex()), b"tampered!").unwrap();

        assert_eq!(store.put(b"original").unwrap(), cid);
        assert_eq!(store.get(&cid).unwrap().as_deref(), Some(&b"original"[..]));
    }

    #[test]
    fn sealing_store_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        store_roundtrip(
            &mut SealingBlobStore::open(dir.path(), [3u8; 32], ChaCha20Rng::seed_from_u64(1))
                .unwrap(),
        );
    }

    #[test]
    fn sealing_store_encrypts_at_rest_and_needs_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let key = [7u8; 32];
        let mut store =
            SealingBlobStore::open(dir.path(), key, ChaCha20Rng::seed_from_u64(1)).unwrap();
        let plaintext = b"secret file contents that must not hit disk in the clear".to_vec();
        let cid = store.put(&plaintext).unwrap();
        assert_eq!(cid, Cid::of(&plaintext), "addressed by the plaintext CID");

        // On disk: sealed; the plaintext never appears.
        let on_disk = std::fs::read(dir.path().join(cid.to_hex())).unwrap();
        assert!(
            !on_disk
                .windows(plaintext.len())
                .any(|w| w == &plaintext[..]),
            "the plaintext must not appear on disk"
        );

        // get → plaintext; a fresh store with the same key reads it back (persistence).
        assert_eq!(store.get(&cid).unwrap(), Some(plaintext.clone()));
        let store2 =
            SealingBlobStore::open(dir.path(), key, ChaCha20Rng::seed_from_u64(9)).unwrap();
        assert_eq!(store2.get(&cid).unwrap(), Some(plaintext));

        // Wrong key → authenticated-decryption failure (never silent garbage).
        let store3 =
            SealingBlobStore::open(dir.path(), [9u8; 32], ChaCha20Rng::seed_from_u64(2)).unwrap();
        assert!(store3.get(&cid).is_err());
    }

    #[test]
    fn sealing_store_put_repairs_a_corrupt_existing_record() {
        let dir = tempfile::tempdir().unwrap();
        let key = [7u8; 32];
        let mut store =
            SealingBlobStore::open(dir.path(), key, ChaCha20Rng::seed_from_u64(1)).unwrap();
        let cid = store.put(b"original").unwrap();
        std::fs::write(dir.path().join(cid.to_hex()), b"truncated").unwrap();

        assert_eq!(store.put(b"original").unwrap(), cid);
        assert_eq!(store.get(&cid).unwrap().as_deref(), Some(&b"original"[..]));
    }
}
