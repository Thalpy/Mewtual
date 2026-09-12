//! Explicit, quota-reserved encrypted copies. This is local ownership, independent of replicated
//! expiry and listing deletion. A separate directory per file deliberately avoids shared durable
//! refcounts: partial work and every completed copy have one clear owner and a conservative charge.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use catcoms_crypto::{seal, unseal};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

use crate::blob::{create_blob_directory, decode_sealed, encode_sealed, read_bounded};
use crate::{BlobStore, Cid, SealingBlobStore, StorageError};

/// Per-server local allocation limit, including padding, both encryption envelopes and metadata.
/// Files are opt-in individually; this limit never enables automatic downloads.
pub const KEPT_BYTE_LIMIT: u64 = 1024 * 1024 * 1024;
pub const KEPT_FILE_LIMIT: usize = 32;
const CHUNK_LIMIT: usize = 128;
const META_LIMIT: usize = 80 * 1024;
const CHUNK_LIMIT_BYTES: u64 = crate::CHUNK_PAD_CEILING as u64 + 44;
const META: &str = "manifest";
const PENDING: &str = "pending";
const LEASE: &str = "lease.lock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepPlan {
    pub cid: Cid,
    /// Digest of this exact encrypted manifest, not a digest of an interchangeable variant set.
    pub version: [u8; 32],
    /// Opaque application manifest, sealed at rest with its wrapped keys for later recovery.
    pub manifest: Vec<u8>,
    /// Exact original ciphertext addresses and conservative encoded-byte bounds (before disk seal).
    pub chunks: Vec<(Cid, u64)>,
}

impl KeepPlan {
    fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_bytes(self.cid.as_bytes()).expect("fixed CID");
        e.put_bytes(&self.version).expect("fixed version");
        e.put_bytes(&self.manifest).expect("bounded manifest");
        e.put_u32(self.chunks.len() as u32);
        for (cid, bound) in &self.chunks {
            e.put_bytes(cid.as_bytes()).expect("fixed CID");
            e.put_u64(*bound);
        }
        e.finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, StorageError> {
        let bad = || StorageError::Malformed;
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| bad())? != 1 {
            return Err(bad());
        }
        let cid = Cid::from_bytes(
            d.get_bytes()
                .map_err(|_| bad())?
                .try_into()
                .map_err(|_| bad())?,
        );
        let version = d
            .get_bytes()
            .map_err(|_| bad())?
            .try_into()
            .map_err(|_| bad())?;
        let manifest = d.get_bytes().map_err(|_| bad())?;
        if manifest.len() > 64 * 1024 {
            return Err(bad());
        }
        let manifest = manifest.to_vec();
        let n = d.get_u32().map_err(|_| bad())? as usize;
        if n > CHUNK_LIMIT {
            return Err(bad());
        }
        let mut chunks = Vec::new();
        for _ in 0..n {
            chunks.push((
                Cid::from_bytes(
                    d.get_bytes()
                        .map_err(|_| bad())?
                        .try_into()
                        .map_err(|_| bad())?,
                ),
                d.get_u64().map_err(|_| bad())?,
            ));
        }
        d.finish().map_err(|_| bad())?;
        let plan = Self {
            cid,
            version,
            manifest,
            chunks,
        };
        plan.charge()?;
        Ok(plan)
    }

    fn charge(&self) -> Result<u64, StorageError> {
        if self.chunks.is_empty()
            || self.chunks.len() > CHUNK_LIMIT
            || self.manifest.len() > 64 * 1024
            || self.chunks.iter().any(|(_, n)| *n > CHUNK_LIMIT_BYTES)
            || self
                .chunks
                .iter()
                .map(|(cid, _)| cid)
                .collect::<HashSet<_>>()
                .len()
                != self.chunks.len()
        {
            return Err(StorageError::BlobSizeLimit);
        }
        // Charge the entire reservation, including unwritten chunks. Repeated failures and index
        // churn cannot manufacture unused capacity while a partial directory still occupies disk.
        Ok(self.chunks.iter().map(|(_, n)| n + 40).sum::<u64>() + self.encode().len() as u64 + 40)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptFile {
    pub plan: KeepPlan,
    /// Full file verification completed in this store instance. Restart deliberately resets it.
    pub checked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeptFiles {
    pub supported: bool,
    pub allocated_bytes: u64,
    pub limit_bytes: u64,
    pub files: Vec<KeptFile>,
    /// A generic local-store failure disables admission without disabling the ordinary blob store.
    pub error: Option<String>,
}

// Cloning an RNG state would repeat nonces under the same disk key. Every child store draws
// from ONE shared injected stream instead; clones share ownership, never generator state.
struct SharedRng<R>(std::sync::Arc<std::sync::Mutex<R>>);
impl<R> Clone for SharedRng<R> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<R: CryptoRngCore> catcoms_rt::CryptoRng for SharedRng<R> {}
impl<R: CryptoRngCore> catcoms_rt::RngCore for SharedRng<R> {
    fn next_u32(&mut self) -> u32 {
        self.0.lock().expect("rng mutex poisoned").next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.0.lock().expect("rng mutex poisoned").next_u64()
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        self.0.lock().expect("rng mutex poisoned").fill_bytes(bytes);
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), catcoms_rt::rng::RngError> {
        self.0
            .lock()
            .expect("rng mutex poisoned")
            .try_fill_bytes(bytes)
    }
}

struct Copy<R: CryptoRngCore> {
    file: KeptFile,
    store: SealingBlobStore<SharedRng<R>>,
}
struct Active<R: CryptoRngCore> {
    token: u64,
    plan: KeepPlan,
    /// Existing copies are checked in place; cancellation must not erase a committed copy.
    store: Option<SealingBlobStore<SharedRng<R>>>,
    seen: HashSet<Cid>,
}

/// Composite disk store. Ordinary cache/upload mutation never touches completed kept copies.
/// Failed retained-store attachment leaves primary storage usable and exposes an honest error.
pub struct KeptBlobStore<R: CryptoRngCore> {
    primary: Box<dyn BlobStore + Send>,
    dir: PathBuf,
    key: Zeroizing<[u8; 32]>,
    rng: SharedRng<R>,
    copies: Vec<Copy<R>>,
    active: Option<Active<R>>,
    serial: u64,
    fault: Option<String>,
    // Separate from the vault-wide lock: one vault can attach the same group more than once.
    // Retention requires exactly one owner of its inventory, pending directory and quota.
    lease: Option<fs::File>,
}

impl<R: CryptoRngCore> std::fmt::Debug for KeptBlobStore<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeptBlobStore")
            .field("copies", &self.copies.len())
            .finish_non_exhaustive()
    }
}

impl<R: CryptoRngCore> KeptBlobStore<R> {
    pub fn open(primary: Box<dyn BlobStore + Send>, dir: PathBuf, key: [u8; 32], rng: R) -> Self {
        let mut this = Self {
            primary,
            dir,
            key: Zeroizing::new(key),
            rng: SharedRng(std::sync::Arc::new(std::sync::Mutex::new(rng))),
            copies: Vec::new(),
            active: None,
            serial: 0,
            fault: None,
            lease: None,
        };
        if this.load().is_err() {
            this.fault = Some("Kept storage needs attention; no new copies can be reserved".into());
        }
        this
    }

    fn load(&mut self) -> Result<(), StorageError> {
        create_blob_directory(&self.dir)?;
        self.lease = Some(crate::vault::try_lock_file(&self.dir.join(LEASE))?);
        for path in entries(&self.dir, KEPT_FILE_LIMIT + 2)? {
            if path.file_name().is_some_and(|n| n == LEASE) {
                continue;
            }
            if !fs::symlink_metadata(&path).map_err(io)?.is_dir()
                || fs::symlink_metadata(&path)
                    .map_err(io)?
                    .file_type()
                    .is_symlink()
            {
                return Err(StorageError::Malformed);
            }
            if path.file_name().is_some_and(|n| n == PENDING) {
                // No in-flight owner survived this mount. Cleanup is bounded, and any failure
                // blocks further admission rather than refunding partially removed bytes.
                remove_copy(&path)?;
                continue;
            }
            let cid = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(Cid::from_hex)
                .ok_or(StorageError::Malformed)?;
            if path.file_name().and_then(|n| n.to_str()) != Some(cid.to_hex().as_str()) {
                return Err(StorageError::Malformed);
            }
            let encoded =
                read_bounded(&path.join(META), META_LIMIT)?.ok_or(StorageError::Malformed)?;
            let plain = Zeroizing::new(unseal(&self.key, &decode_sealed(&encoded)?)?);
            let plan = KeepPlan::decode(&plain)?;
            if plan.cid != cid {
                return Err(StorageError::Malformed);
            }
            for record in entries(&path, CHUNK_LIMIT + 1)? {
                let metadata = fs::symlink_metadata(&record).map_err(io)?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(StorageError::Malformed);
                }
                let bound = if record.file_name().is_some_and(|n| n == META) {
                    plan.encode().len() as u64 + 40
                } else {
                    let address = record
                        .file_name()
                        .and_then(|n| n.to_str())
                        .and_then(Cid::from_hex)
                        .ok_or(StorageError::Malformed)?;
                    plan.chunks
                        .iter()
                        .find(|(cid, _)| *cid == address)
                        .ok_or(StorageError::Malformed)?
                        .1
                        + 40
                };
                if metadata.len() > bound {
                    return Err(StorageError::BlobSizeLimit);
                }
            }
            let store = SealingBlobStore::open(path, *self.key, self.rng.clone())?;
            self.copies.push(Copy {
                file: KeptFile {
                    plan,
                    checked: false,
                },
                store,
            });
        }
        self.copies.sort_by_key(|copy| copy.file.plan.cid.to_hex());
        Ok(())
    }

    fn finish_keep_with_sync(
        &mut self,
        token: u64,
        sync_directory: impl Fn(&Path) -> Result<(), StorageError>,
    ) -> Result<(), StorageError> {
        self.ready()?;
        let active = self
            .active
            .as_ref()
            .filter(|a| a.token == token)
            .ok_or(StorageError::Malformed)?;
        if active.seen.len() != active.plan.chunks.len() {
            return Err(StorageError::Malformed);
        }
        let cid = active.plan.cid;
        if active.store.is_some() {
            sync_directory(&self.dir.join(PENDING))?;
            let destination = self.dir.join(cid.to_hex());
            if destination.exists() {
                return Err(StorageError::Malformed);
            }
            fs::rename(self.dir.join(PENDING), &destination).map_err(io)?;
            // Rename is the visibility commit. Even a following flush/open failure must retain
            // accounting and prevent another reservation from reusing this directory's quota.
            if let Err(error) = sync_directory(&self.dir) {
                self.fault = Some("kept-copy commit needs a storage check".into());
                return Err(error);
            }
            let store = SealingBlobStore::open(destination, *self.key, self.rng.clone())?;
            let active = self.active.take().expect("checked active");
            self.copies.push(Copy {
                file: KeptFile {
                    plan: active.plan,
                    checked: true,
                },
                store,
            });
        } else {
            // Rechecking may have recreated missing files. Their directory entries need the same
            // durability barrier as a newly committed copy before its checked verdict is visible.
            sync_directory(&self.dir.join(cid.to_hex()))?;
            self.copies
                .iter_mut()
                .find(|c| c.file.plan.cid == cid)
                .ok_or(StorageError::Malformed)?
                .file
                .checked = true;
            self.active = None;
        }
        Ok(())
    }

    fn ready(&self) -> Result<(), StorageError> {
        if let Some(error) = &self.fault {
            return Err(StorageError::Io(error.clone()));
        }
        Ok(())
    }

    fn retained_get(&self, cid: &Cid, max: usize) -> Result<Option<Vec<u8>>, StorageError> {
        for copy in &self.copies {
            if copy
                .file
                .plan
                .chunks
                .iter()
                .any(|(address, _)| address == cid)
            {
                if let Ok(Some(bytes)) = copy.store.get_bounded(cid, max) {
                    return Ok(Some(bytes));
                }
            }
        }
        Ok(None)
    }
}

fn io(error: std::io::Error) -> StorageError {
    StorageError::Io(error.to_string())
}
fn sync_dir(path: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|e| StorageError::CommittedButNotDurable(e.to_string()))?;
    #[cfg(not(unix))]
    let _ = path; // Same file-flush-only Windows durability seam as the vault and blob store.
    Ok(())
}
fn flush(path: &Path) -> Result<(), StorageError> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|e| StorageError::CommittedButNotDurable(e.to_string()))
}
fn entries(dir: &Path, limit: usize) -> Result<Vec<PathBuf>, StorageError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir).map_err(io)?.take(limit + 1) {
        paths.push(entry.map_err(io)?.path());
        if paths.len() > limit {
            return Err(StorageError::BlobSizeLimit);
        }
    }
    Ok(paths)
}
fn remove_copy(dir: &Path) -> Result<(), StorageError> {
    let paths = entries(dir, CHUNK_LIMIT + 1)?;
    // Validate all leaves before deleting any; never follow links or recursively traverse input.
    for path in &paths {
        let meta = fs::symlink_metadata(path).map_err(io)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(StorageError::Malformed);
        }
    }
    for path in paths {
        fs::remove_file(path).map_err(io)?;
    }
    fs::remove_dir(dir).map_err(io)?;
    if let Some(parent) = dir.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

impl<R: CryptoRngCore> BlobStore for KeptBlobStore<R> {
    fn kept_files(&self) -> KeptFiles {
        let allocated_bytes = self
            .copies
            .iter()
            .map(|c| c.file.plan.charge().unwrap_or(KEPT_BYTE_LIMIT))
            .sum::<u64>()
            + self
                .active
                .as_ref()
                .filter(|a| a.store.is_some())
                .map_or(0, |a| a.plan.charge().unwrap_or(KEPT_BYTE_LIMIT));
        KeptFiles {
            supported: true,
            allocated_bytes,
            limit_bytes: KEPT_BYTE_LIMIT,
            files: self.copies.iter().map(|c| c.file.clone()).collect(),
            error: self.fault.clone(),
        }
    }

    fn begin_keep(&mut self, plan: KeepPlan) -> Result<u64, StorageError> {
        self.ready()?;
        let charge = plan.charge()?;
        if self.active.is_some() {
            return Err(StorageError::Io("another kept copy is in progress".into()));
        }
        let existing = self.copies.iter().find(|c| c.file.plan.cid == plan.cid);
        if let Some(existing) = existing {
            if existing.file.plan != plan {
                return Err(StorageError::Io(
                    "remove the older kept copy before replacing its manifest".into(),
                ));
            }
        } else if self.copies.len() >= KEPT_FILE_LIMIT
            || self.kept_files().allocated_bytes.saturating_add(charge) > KEPT_BYTE_LIMIT
        {
            return Err(StorageError::Io(
                "kept-copy storage limit reached (1 GiB or 32 files per server)".into(),
            ));
        }
        self.serial = self.serial.checked_add(1).ok_or(StorageError::Malformed)?;
        let store = if existing.is_some() {
            None
        } else {
            let path = self.dir.join(PENDING);
            fs::create_dir(&path).map_err(io)?;
            let store = SealingBlobStore::open(&path, *self.key, self.rng.clone())?;
            Some(store)
        };
        let token = self.serial;
        if let Some(copy) = self.copies.iter_mut().find(|c| c.file.plan.cid == plan.cid) {
            copy.file.checked = false;
        }
        self.active = Some(Active {
            token,
            plan,
            store,
            seen: HashSet::new(),
        });
        let active = self.active.as_ref().expect("installed reservation");
        if active.store.is_some() {
            let encoded = encode_sealed(&seal(&self.key, &active.plan.encode(), &mut self.rng)?);
            let path = self.dir.join(PENDING).join(META);
            // The reservation is installed before the first write; a failure remains charged.
            if let Err(error) = fs::write(&path, encoded)
                .map_err(io)
                .and_then(|_| flush(&path))
                .and_then(|_| sync_dir(&self.dir.join(PENDING)))
            {
                let _ = self.abort_keep(token);
                return Err(error);
            }
        }
        Ok(token)
    }

    fn put_keep(&mut self, token: u64, bytes: &[u8]) -> Result<(), StorageError> {
        self.ready()?;
        let active = self
            .active
            .as_mut()
            .filter(|a| a.token == token)
            .ok_or(StorageError::Malformed)?;
        let cid = Cid::of(bytes);
        let bound = active
            .plan
            .chunks
            .iter()
            .find(|(c, _)| *c == cid)
            .ok_or(StorageError::CidMismatch)?
            .1;
        if bytes.len() as u64 > bound {
            return Err(StorageError::BlobSizeLimit);
        }
        if let Some(store) = active.store.as_mut() {
            store.put(bytes)?;
            flush(&self.dir.join(PENDING).join(cid.to_hex()))?;
        } else {
            // A recheck must verify/repair the retained record itself. Healthy primary cache
            // bytes alone cannot certify a missing or corrupt kept copy after restart.
            let copy = self
                .copies
                .iter_mut()
                .find(|c| c.file.plan.cid == active.plan.cid)
                .ok_or(StorageError::Malformed)?;
            copy.store.put(bytes)?;
            flush(&self.dir.join(active.plan.cid.to_hex()).join(cid.to_hex()))?;
        }
        active.seen.insert(cid);
        Ok(())
    }

    fn finish_keep(&mut self, token: u64) -> Result<(), StorageError> {
        self.finish_keep_with_sync(token, sync_dir)
    }

    fn abort_keep(&mut self, token: u64) -> Result<(), StorageError> {
        let active = self
            .active
            .as_ref()
            .filter(|a| a.token == token)
            .ok_or(StorageError::Malformed)?;
        if active.store.is_some() {
            if let Err(error) = remove_copy(&self.dir.join(PENDING)) {
                self.fault = Some("incomplete kept-copy cleanup needs attention".into());
                return Err(error);
            }
        }
        self.active = None;
        Ok(())
    }

    fn forget_kept(&mut self, cid: &Cid) -> Result<(), StorageError> {
        self.ready()?;
        if self.active.is_some() {
            return Err(StorageError::Io("cancel the active kept copy first".into()));
        }
        let index = self
            .copies
            .iter()
            .position(|c| c.file.plan.cid == *cid)
            .ok_or(StorageError::Malformed)?;
        // A partial removal stays charged and loses its present-time checked verdict immediately.
        self.copies[index].file.checked = false;
        // The local release intent is an atomic move into the non-serving cleanup namespace.
        // A partial delete (including loss of its manifest) remains recognizable after restart.
        fs::rename(self.dir.join(cid.to_hex()), self.dir.join(PENDING)).map_err(io)?;
        if let Err(error) = sync_dir(&self.dir).and_then(|_| remove_copy(&self.dir.join(PENDING))) {
            self.fault = Some(
                "kept-copy removal is incomplete; restart the application to retry cleanup".into(),
            );
            return Err(error);
        }
        self.copies.remove(index);
        Ok(())
    }

    fn is_persistent(&self) -> bool {
        self.primary.is_persistent()
    }
    fn put(&mut self, bytes: &[u8]) -> Result<Cid, StorageError> {
        self.primary.put(bytes)
    }
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StorageError> {
        match self.primary.get(cid) {
            Ok(Some(bytes)) => Ok(Some(bytes)),
            primary => match self.retained_get(cid, CHUNK_LIMIT_BYTES as usize)? {
                Some(bytes) => Ok(Some(bytes)),
                None => primary,
            },
        }
    }
    fn get_bounded(&self, cid: &Cid, max: usize) -> Result<Option<Vec<u8>>, StorageError> {
        match self.primary.get_bounded(cid, max) {
            Ok(Some(bytes)) => Ok(Some(bytes)),
            primary => match self.retained_get(cid, max)? {
                Some(bytes) => Ok(Some(bytes)),
                None => primary,
            },
        }
    }
    fn has(&self, cid: &Cid) -> bool {
        self.primary.has(cid)
            || self
                .copies
                .iter()
                .any(|c| c.file.plan.chunks.iter().any(|(id, _)| id == cid) && c.store.has(cid))
    }
    fn delete(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        self.primary.delete(cid)
    }
    fn cids(&self) -> Vec<Cid> {
        let mut cids = self.primary.cids();
        for c in &self.copies {
            cids.extend(c.store.cids());
        }
        cids.sort_by_key(Cid::to_hex);
        cids.dedup();
        cids
    }
    fn put_staged(&mut self, bytes: &[u8]) -> Result<Cid, StorageError> {
        self.primary.put_staged(bytes)
    }
    fn promote_staged(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        self.primary.promote_staged(cid)
    }
    fn promote_staged_bounded(&mut self, cid: &Cid, max: usize) -> Result<bool, StorageError> {
        self.primary.promote_staged_bounded(cid, max)
    }
    fn drop_staged(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        self.primary.drop_staged(cid)
    }
    fn clear_staging(&mut self) -> Result<usize, StorageError> {
        self.primary.clear_staging()
    }
}

#[cfg(test)]
mod tests;
