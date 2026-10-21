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
mod downloads;
mod export;
mod external;
mod persistence;
mod record;
use downloads::Owner;
pub use downloads::{DownloadExportLease, DownloadId, DownloadRun, DownloadStatus, DownloadStore};
pub use export::{
    flush_destination, partial_presence, remove_partial, ChoiceRefusal, ChosenDestination,
    ExportError, ExportFile, ExportHalt, ExportJournal, ExportPlan, ExportPresence, ExportProbe,
    ExportProgress, ExportReopen, ExportState, ExportView, PartialPresence, PartialRemoval,
    Publication, EXPORT_CHECKPOINT_CHUNKS,
};
use persistence::{Transfer, DELETING, FORMAT, FORMAT_CHARGE, TRANSFERS};
pub use record::{
    KeepAllowance, KeepPhase, KeepRetry, KeepRetryReceipt, KeepScope, KEEP_EXECUTION_SLICE_MS,
};

/// Opaque process-local run identity. It is not serialized or accepted from IPC. One checked
/// process sequence prevents stale same-CID workers from matching a replacement or remount;
/// restarting the process cannot leave an old Rust worker alive with a colliding owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepRun(u64);
impl KeepRun {
    fn fresh() -> Result<Self, StorageError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map(Self)
            .map_err(|_| StorageError::Malformed)
    }
}
/// One requested network window. This is a trusted Rust-side description of an already prepared
/// exact blob request; IPC cannot select its allowance or construct an execution grant.
#[derive(Debug, Clone, Copy)]
pub struct KeepNetworkWork {
    pub bytes: u64,
    pub paged: bool,
}
/// Prepaid runtime authority with an original monotonic deadline. Cloning cannot renew it;
/// pause/replacement/Retry also invalidate its exact run and persistent grant bindings.
#[derive(Debug, Clone)]
pub struct KeepWork {
    run: KeepRun,
    grant: u64,
    deadline: u64,
    // Receivers cannot keep the native run alive. Dropping/replacing the sole Active sender
    // invalidates queued lower work synchronously, before its caller task can run again.
    owner: tokio::sync::watch::Receiver<bool>,
}
impl KeepWork {
    pub fn deadline(&self) -> u64 {
        self.deadline
    }
    pub fn run_is_live(&self) -> bool {
        *self.owner.borrow() && self.owner.has_changed().is_ok()
    }
    pub async fn retired(&self) {
        let mut owner = self.owner.clone();
        while *owner.borrow() && owner.has_changed().is_ok() {
            if owner.changed().await.is_err() {
                return;
            }
        }
    }
}
struct WorkWindow {
    deadline: u64,
    remaining: KeepAllowance,
}

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
    /// Partial/deleting ownership stays separate from completed `files`; progress never claims
    /// that a whole file is retained or presently verified.
    pub transfers: Vec<KeptTransfer>,
    /// A generic local-store failure disables admission without disabling the ordinary blob store.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptTransfer {
    pub plan: KeepPlan,
    pub listing: [u8; 32],
    pub transfer: [u8; 32],
    pub phase: KeepPhase,
    pub verified_chunks: usize,
    pub running: bool,
    /// Explicit user Pause survives runtime cancellation, vault remount and process restart.
    pub user_paused: bool,
    pub blocked: bool,
    pub intent: u64,
    pub exhausted: bool,
    pub grant: u64,
    pub remaining: KeepAllowance,
}

/// Small equality projection for coalescing UI updates without copying opaque plans per page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepProgress {
    pub phase: KeepPhase,
    pub verified_chunks: usize,
    pub running: bool,
    pub user_paused: bool,
    pub blocked: bool,
    pub exhausted: bool,
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
struct Active {
    token: KeepRun,
    identity: Owner,
    plan: KeepPlan,
    /// Existing copies are checked in place; cancellation must not erase a committed copy.
    transfer: Option<usize>,
    seen: HashSet<Cid>,
    window: Option<WorkWindow>,
    owner: tokio::sync::watch::Sender<bool>,
}

/// Composite disk store. Ordinary cache/upload mutation never touches completed kept copies.
/// Failed retained-store attachment leaves primary storage usable and exposes an honest error.
pub struct KeptBlobStore<R: CryptoRngCore> {
    primary: Box<dyn BlobStore + Send>,
    dir: PathBuf,
    key: Zeroizing<[u8; 32]>,
    /// Purpose-separated native intent/budget key. Never use the arbitrary blob-sealing key
    /// here: otherwise cached chosen plaintext could be copied offline into an authorized record.
    metadata_key: Zeroizing<[u8; 32]>,
    rng: SharedRng<R>,
    copies: Vec<Copy<R>>,
    transfers: Vec<Transfer<R>>,
    scope: Option<KeepScope>,
    read_epoch: crate::blob_read::ReadEpoch,
    active: Option<Active>,
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
    pub fn open(
        primary: Box<dyn BlobStore + Send>,
        dir: PathBuf,
        key: [u8; 32],
        metadata_key: [u8; 32],
        rng: R,
    ) -> Self {
        let mut this = Self {
            primary,
            dir,
            key: Zeroizing::new(key),
            metadata_key: Zeroizing::new(metadata_key),
            rng: SharedRng(std::sync::Arc::new(std::sync::Mutex::new(rng))),
            copies: Vec::new(),
            transfers: Vec::new(),
            scope: None,
            read_epoch: crate::blob_read::ReadEpoch::default(),
            active: None,
            fault: None,
            lease: None,
        };
        if key == metadata_key {
            this.fault = Some("retained metadata requires a separate encryption key".into());
            return this;
        }
        if this.load().is_err() {
            this.fault = Some("Kept storage needs attention; no new copies can be reserved".into());
        }
        this
    }

    fn load(&mut self) -> Result<(), StorageError> {
        create_blob_directory(&self.dir)?;
        self.lease = Some(crate::vault::try_lock_file(&self.dir.join(LEASE))?);
        let paths = entries(&self.dir, KEPT_FILE_LIMIT + 5)?;
        let upgraded = paths
            .iter()
            .any(|path| path.file_name().is_some_and(|name| name == FORMAT));
        for path in paths {
            if path.file_name().is_some_and(|n| n == LEASE) {
                continue;
            }
            if path.file_name().is_some_and(|name| name == FORMAT) {
                let metadata = fs::symlink_metadata(&path).map_err(io)?;
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.len() > FORMAT_CHARGE
                {
                    return Err(StorageError::Malformed);
                }
                continue;
            }
            if path
                .file_name()
                .is_some_and(|name| name == TRANSFERS || name == DELETING)
            {
                let metadata = fs::symlink_metadata(&path).map_err(io)?;
                if !upgraded || !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(StorageError::Malformed);
                }
                // No V2 reads, grants, cleanup or retained exposure until immutable binding.
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
                if upgraded {
                    continue;
                }
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

    fn put_keep_with_barriers(
        &mut self,
        token: KeepRun,
        bytes: &[u8],
        flush_file: impl Fn(&Path) -> Result<(), StorageError>,
        sync_directory: impl Fn(&Path) -> Result<(), StorageError>,
    ) -> Result<(), StorageError> {
        self.ready()?;
        let active = self
            .active
            .as_mut()
            .filter(|active| active.token == token)
            .ok_or(StorageError::Malformed)?;
        if !*active.owner.borrow() {
            return Err(StorageError::Malformed);
        }
        let cid = Cid::of(bytes);
        let bound = active
            .plan
            .chunks
            .iter()
            .find(|(address, _)| *address == cid)
            .ok_or(StorageError::CidMismatch)?
            .1;
        if bytes.len() as u64 > bound {
            return Err(StorageError::BlobSizeLimit);
        }
        if let Some(index) = active.transfer {
            let transfer = &mut self.transfers[index];
            if transfer.poisoned || transfer.record.phase == KeepPhase::Deleting {
                return Err(StorageError::Malformed);
            }
            if transfer.record.exhausted {
                return Err(StorageError::KeepExhausted);
            }
            let chunk = transfer
                .record
                .plan
                .chunks
                .iter()
                .position(|(address, _)| *address == cid)
                .ok_or(StorageError::CidMismatch)?;
            let held = matches!(transfer.store.get_bounded(&cid, bound as usize), Ok(Some(held)) if held == bytes);
            if !held {
                transfer.store.put(bytes)?;
            }
            // Readable matching bytes can outlive a failed or interrupted earlier flush. Every
            // run re-establishes content and directory durability before accepting progress;
            // the dedup shortcut avoids rewriting bytes, never their publication barriers.
            flush_file(&transfer.path.join(cid.to_hex()))?;
            sync_directory(&transfer.path)?;
            if !transfer.record.verified[chunk] {
                let mut next = transfer.record.successor()?;
                next.note_verified(chunk);
                transfer.publish(next, &self.metadata_key, &mut self.rng)?;
            }
        } else {
            // Legacy completed repair remains in place; cancellation never erases that copy.
            let copy = self
                .copies
                .iter_mut()
                .find(|copy| copy.file.plan.cid == active.plan.cid)
                .ok_or(StorageError::Malformed)?;
            copy.store.put(bytes)?;
            flush_file(&self.dir.join(active.plan.cid.to_hex()).join(cid.to_hex()))?;
        }
        active.seen.insert(cid);
        Ok(())
    }

    fn finish_keep_with_sync(
        &mut self,
        token: KeepRun,
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
        if let Some(index) = active.transfer {
            let transfer = &mut self.transfers[index];
            if transfer.record.exhausted || transfer.record.phase == KeepPhase::Deleting {
                return Err(StorageError::KeepExhausted);
            }
            sync_directory(&transfer.path)?;
            if transfer.record.phase != KeepPhase::Completed
                || !transfer.record.selected_is_winner()
            {
                let mut next = transfer.record.successor()?;
                next.commit_selected();
                transfer.publish(next, &self.metadata_key, &mut self.rng)?;
            }
            transfer.checked = true;
            self.active = None;
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
        if self.transfers.iter().any(|transfer| transfer.poisoned) {
            return Err(StorageError::Io(
                "retained transfer needs a storage check".into(),
            ));
        }
        Ok(())
    }

    fn retained_get(&self, cid: &Cid, max: usize) -> Result<Option<Vec<u8>>, StorageError> {
        if self.scope.is_none() {
            return Ok(None);
        }
        for transfer in &self.transfers {
            if !transfer.poisoned
                && transfer.record.public_retained()
                && transfer.record.phase == KeepPhase::Completed
                && transfer
                    .record
                    .committed_plan()
                    .is_some_and(|plan| plan.chunks.iter().any(|(address, _)| address == cid))
            {
                if let Ok(Some(bytes)) = transfer.store.get_bounded(cid, max) {
                    return Ok(Some(bytes));
                }
            }
        }
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

    fn begin_with_listing(
        &mut self,
        plan: KeepPlan,
        listing: [u8; 32],
    ) -> Result<KeepRun, StorageError> {
        self.begin_with_candidates(plan, listing, None)
    }

    fn begin_with_candidates(
        &mut self,
        plan: KeepPlan,
        listing: [u8; 32],
        candidates: Option<(Vec<KeepPlan>, usize)>,
    ) -> Result<KeepRun, StorageError> {
        let identity = Owner::Keep(plan.cid);
        self.begin_owned(plan, listing, candidates, identity, None)
    }

    /// Keep and Download share the transaction engine, never its ownership selector. Only a
    /// newly prepared Download record can create its private owner; resume cannot mint one.
    fn begin_owned(
        &mut self,
        plan: KeepPlan,
        listing: [u8; 32],
        candidates: Option<(Vec<KeepPlan>, usize)>,
        identity: Owner,
        prepared: Option<record::Record>,
    ) -> Result<KeepRun, StorageError> {
        self.ready()?;
        let scope = self.scope.clone().ok_or(StorageError::Malformed)?;
        plan.charge()?;
        if candidates.is_some()
            && (self
                .copies
                .iter()
                .any(|c| identity == Owner::Keep(c.file.plan.cid))
                || self.transfers.iter().any(|t| t.record.owner() == identity))
        {
            // Existing exact-only ownership cannot silently acquire new candidates from a later
            // listing, even if its current row happens to resolve to an equivalent plaintext.
            return Err(StorageError::Malformed);
        }
        if self.active.is_some() {
            return Err(StorageError::Io("another kept copy is in progress".into()));
        }
        let token = KeepRun::fresh()?;
        let transfer = if let Some(copy) = self
            .copies
            .iter_mut()
            .find(|copy| identity == Owner::Keep(copy.file.plan.cid))
        {
            if copy.file.plan != plan {
                return Err(StorageError::Malformed);
            }
            copy.file.checked = false;
            None // Existing legacy copies retain their exact saved repair/release contract.
        } else if let Some(index) = self
            .transfers
            .iter()
            .position(|transfer| transfer.record.owner() == identity)
        {
            let transfer = &mut self.transfers[index];
            if transfer.poisoned
                || transfer.record.plan != plan
                || transfer.record.listing != listing
                || transfer.record.phase == KeepPhase::Deleting
            {
                return Err(StorageError::Malformed);
            }
            if matches!(transfer.record.export, export::Export::Published(_)) {
                // Re-verifying a saved file would spend allowance for nothing: the user chooses
                // "Save another copy" (which forgets the journal) or "Clear" instead.
                return Err(StorageError::Io(
                    "This file was already saved. Clear it or save another copy from Transfers."
                        .into(),
                ));
            }
            if transfer.record.exhausted {
                return Err(StorageError::KeepExhausted);
            }
            if transfer.record.user_paused || transfer.record.blocked {
                return Err(StorageError::Io("this transfer is paused".into()));
            }
            if transfer.record.selected_is_winner() {
                if !transfer.record.public_retained() {
                    // Starting another private verification run invalidates previously issued
                    // export observations, even when the saved winner itself is unchanged.
                    // The export presence token is kept: a still-live older session must block
                    // a new one from opening a second handle onto the same partial.
                    transfer.read_epoch.retire();
                }
                transfer.checked = false;
            }
            Some(index)
        } else {
            let record = if let Some(record) = prepared {
                record.validate_initial()?;
                if record.owner() != identity
                    || record.scope != scope
                    || record.plan != plan
                    || record.listing != listing
                {
                    return Err(StorageError::Malformed);
                }
                record
            } else if identity == Owner::Keep(plan.cid) {
                match candidates {
                    Some((plans, selected)) => record::Record::initial_variants(
                        scope.clone(),
                        plans,
                        selected,
                        listing,
                        &mut self.rng,
                    )?,
                    None => record::Record::initial(
                        scope.clone(),
                        plan.clone(),
                        listing,
                        &mut self.rng,
                    )?,
                }
            } else {
                return Err(StorageError::Malformed);
            };
            let charge = record.charge()?;
            if self.copies.len() + self.transfers.len() >= KEPT_FILE_LIMIT
                || self
                    .kept_files()
                    .allocated_bytes
                    .saturating_add(charge)
                    .saturating_add(if self.dir.join(FORMAT).exists() {
                        0
                    } else {
                        FORMAT_CHARGE
                    })
                    > KEPT_BYTE_LIMIT
            {
                return Err(StorageError::Io(
                    "Saved file storage is full (1 GiB or 32 files per server).".into(),
                ));
            }
            let created = (|| {
                persistence::ensure_format(&self.dir, &scope, &self.metadata_key, &mut self.rng)?;
                Transfer::create(
                    self.dir.join(TRANSFERS).join(identity.name()),
                    record,
                    *self.key,
                    *self.metadata_key,
                    self.rng.clone(),
                )
            })();
            let transfer = match created {
                Ok(transfer) => transfer,
                // A full volume refused the first record, and both the torn record and the empty
                // directory were removed: nothing new exists on disk, so admission simply fails.
                Err(error) if persistence::is_clean_disk_full(&error) => return Err(error),
                Err(error) => {
                    // An orphan/ambiguous first publication remains charged conservatively.
                    // Never retry another admission against the previous in-memory inventory.
                    self.fault = Some("retained transfer needs a storage check".into());
                    return Err(error);
                }
            };
            self.transfers.push(transfer);
            Some(self.transfers.len() - 1)
        };
        self.active = Some(Active {
            token,
            identity,
            plan,
            transfer,
            seen: HashSet::new(),
            window: None,
            owner: tokio::sync::watch::channel(true).0,
        });
        Ok(token)
    }

    fn transfer_state(&self, t: &Transfer<R>) -> KeptTransfer {
        KeptTransfer {
            plan: t.record.plan.clone(),
            listing: t.record.listing,
            transfer: t.record.transfer,
            phase: t.record.phase,
            verified_chunks: t.record.verified.iter().filter(|bit| **bit).count(),
            running: self
                .active
                .as_ref()
                .is_some_and(|active| active.identity == t.record.owner()),
            user_paused: t.record.user_paused,
            blocked: t.record.blocked,
            intent: t.record.intent,
            exhausted: t.record.exhausted,
            grant: t.record.grant,
            remaining: t.record.remaining,
        }
    }

    fn release_transfer(&mut self, index: usize) -> Result<(), StorageError> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.identity != self.transfers[index].record.owner())
        {
            return Err(StorageError::Io("pause the active kept copy first".into()));
        }
        let transfer = &mut self.transfers[index];
        if transfer.record.phase != KeepPhase::Deleting {
            let mut next = transfer.record.successor()?;
            next.phase = KeepPhase::Deleting;
            transfer.publish(next, &self.metadata_key, &mut self.rng)?;
        }
        transfer.checked = false;
        transfer.export_presence = None;
        // Invalidate runtime ownership before moving/removing any ciphertext. Late workers keep
        // their actual transport charges, but cannot mutate a replacement same-CID transfer.
        self.active = None;
        let deleting = self.dir.join(DELETING).join(transfer.record.owner().name());
        let cleanup = (|| {
            fs::rename(&transfer.path, &deleting).map_err(io)?;
            persistence::directory_barrier(&self.dir.join(TRANSFERS))?;
            persistence::directory_barrier(&self.dir.join(DELETING))?;
            persistence::finish_deleting(&deleting)
        })();
        if cleanup.is_err() {
            self.fault = Some("kept-copy removal needs a storage check".into());
        }
        cleanup?;
        self.transfers.remove(index);
        Ok(())
    }

    fn retry_owned(
        &mut self,
        identity: Owner,
        request: KeepRetry,
    ) -> Result<KeepRetryReceipt, StorageError> {
        self.ready()?;
        let index = self
            .transfers
            .iter()
            .position(|t| t.record.owner() == identity)
            .ok_or(StorageError::Malformed)?;
        if let Some(receipt) = self.transfers[index]
            .record
            .last_retry
            .filter(|r| r.request == request)
        {
            return Ok(receipt); // History-only retry never needs to disturb today's active owner.
        }
        if self.active.is_some() {
            return Err(StorageError::Io("Pause the active transfer first.".into()));
        }
        let transfer = &mut self.transfers[index];
        let (next, receipt) = transfer.record.retry(request)?;
        if next != transfer.record {
            transfer.publish(next, &self.metadata_key, &mut self.rng)?;
        }
        Ok(receipt)
    }

    fn set_owned_paused(
        &mut self,
        identity: Owner,
        owner: [u8; 32],
        expected_intent: u64,
        paused: bool,
    ) -> Result<(), StorageError> {
        self.ready()?;
        let index = self
            .transfers
            .iter()
            .position(|t| t.record.owner() == identity)
            .ok_or(StorageError::Malformed)?;
        let transfer = &self.transfers[index];
        if matches!(transfer.record.export, export::Export::Published(_)) {
            // A saved file has nothing to pause or resume; Clear or Save another copy apply.
            return Err(StorageError::Malformed);
        }
        let phase_allows_intent = match transfer.record.phase {
            KeepPhase::Resumable => true,
            // Private export can be paused too. Public completed Keeps retain their existing
            // check/repair contract; pausing them is a runtime Stop checking action instead.
            KeepPhase::Completed => !paused || !transfer.record.public_retained(),
            KeepPhase::Deleting => false,
        };
        if transfer.poisoned
            || transfer.record.transfer != owner
            || transfer.record.intent != expected_intent
            || !phase_allows_intent
        {
            return Err(StorageError::Malformed);
        }
        if !paused && transfer.record.exhausted {
            return Err(StorageError::KeepExhausted);
        }
        // Revoke the exact old runtime/export observer BEFORE a potentially failing write.
        // Neither another owner of the same CID nor a later Resume can revive that observer.
        if paused {
            if self.active.as_ref().is_some_and(|a| a.identity == identity) {
                self.active = None;
            }
            if !self.transfers[index].record.public_retained() {
                // Retiring the epoch ends "Saving" in the projection and fences the old session's
                // journal writes. Its presence token stays until that session drops its file, so
                // a Resume cannot open a second writer onto the same partial in the meantime.
                self.transfers[index].read_epoch.retire();
            }
        }
        let mut next = self.transfers[index].record.successor()?;
        next.user_paused = paused;
        if !paused {
            next.blocked = false;
            // Explicit Resume re-checks the destination from scratch; a halt is cleared only by
            // this intent change, and the next reopen persists it again if it still applies.
            if let export::Export::Writing(journal) = &mut next.export {
                journal.halt = None;
            }
        }
        next.intent = expected_intent
            .checked_add(1)
            .ok_or(StorageError::Malformed)?;
        self.transfers[index].publish(next, &self.metadata_key, &mut self.rng)
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
    fn private_downloads(&self) -> Option<&dyn DownloadStore> {
        Some(self)
    }
    fn private_downloads_mut(&mut self) -> Option<&mut dyn DownloadStore> {
        Some(self)
    }
    fn disable_retention(&mut self) {
        self.read_epoch.retire();
        self.scope = None;
        self.active = None;
        self.fault = Some("retained storage identity is unavailable".into());
    }
    fn configure_keep_scope(&mut self, scope: KeepScope) -> Result<(), StorageError> {
        self.ready()?;
        if let Some(current) = &self.scope {
            if *current == scope {
                return Ok(());
            }
            self.read_epoch.retire();
            self.scope = None;
            self.active = None;
            self.fault = Some("retained storage identity changed".into());
            return Err(StorageError::Malformed);
        }
        if let Err(error) = scope.validate() {
            self.disable_retention();
            return Err(error);
        }
        self.scope = Some(scope.clone());
        let result = (|| {
            if !self.dir.join(FORMAT).exists() {
                return Ok(());
            }
            persistence::ensure_format(&self.dir, &scope, &self.metadata_key, &mut self.rng)?;
            // Legacy pending remains cleanup-only even after migration. It is never adopted as
            // resumable work; a release of a legacy completed copy may have stopped here.
            if self.dir.join(PENDING).exists() {
                remove_copy(&self.dir.join(PENDING))?;
            }
            for path in entries(&self.dir.join(DELETING), KEPT_FILE_LIMIT)? {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or(StorageError::Malformed)?;
                if Owner::parse(name).is_none() {
                    return Err(StorageError::Malformed);
                }
                persistence::finish_deleting(&path)?;
            }
            let paths = entries(&self.dir.join(TRANSFERS), KEPT_FILE_LIMIT)?;
            if self.copies.len() + paths.len() > KEPT_FILE_LIMIT {
                return Err(StorageError::BlobSizeLimit);
            }
            let mut loaded = Vec::new();
            for path in paths {
                let identity = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(Owner::parse)
                    .ok_or(StorageError::Malformed)?;
                if self
                    .copies
                    .iter()
                    .any(|copy| identity == Owner::Keep(copy.file.plan.cid))
                {
                    return Err(StorageError::Malformed);
                }
                loaded.push(Transfer::open(
                    path,
                    &scope,
                    *self.key,
                    *self.metadata_key,
                    self.rng.clone(),
                )?);
            }
            let total = self
                .copies
                .iter()
                .map(|copy| copy.file.plan.charge())
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .sum::<u64>()
                + loaded
                    .iter()
                    .map(|transfer| transfer.record.charge())
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .sum::<u64>()
                + FORMAT_CHARGE;
            if total > KEPT_BYTE_LIMIT {
                return Err(StorageError::BlobSizeLimit);
            }
            self.transfers = loaded;
            // A durable deletion intent may have stopped before its cleanup-only move. Scope
            // and full inventory are now validated; finish it without manufacturing a run grant.
            // One pass over a fixed list: a Download that still names an external partial is
            // left in Deleting for an explicit Remove, because only native may touch the
            // Downloads folder and a stalled or missing folder must never stall or loop mount.
            let pending: Vec<Owner> = self
                .transfers
                .iter()
                .filter(|t| {
                    t.record.phase == KeepPhase::Deleting && t.record.export.journal().is_none()
                })
                .map(|t| t.record.owner())
                .collect();
            for owner in pending {
                let index = self
                    .transfers
                    .iter()
                    .position(|t| t.record.owner() == owner)
                    .ok_or(StorageError::Malformed)?;
                self.release_transfer(index)?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.read_epoch.retire();
            self.scope = None;
            self.fault = Some("retained storage needs a storage check".into());
        }
        result
    }
    fn kept_files(&self) -> KeptFiles {
        let known = self
            .copies
            .iter()
            .map(|copy| copy.file.plan.charge().unwrap_or(KEPT_BYTE_LIMIT))
            .sum::<u64>()
            + self
                .transfers
                .iter()
                .map(|transfer| transfer.record.charge().unwrap_or(KEPT_BYTE_LIMIT))
                .sum::<u64>()
            + if self.dir.join(FORMAT).exists() {
                FORMAT_CHARGE
            } else {
                0
            };
        let bound = self.scope.is_some();
        let mut files = if bound {
            self.copies
                .iter()
                .map(|copy| copy.file.clone())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if bound {
            files.extend(
                self.transfers
                    .iter()
                    .filter(|t| {
                        !t.poisoned
                            && t.record.public_retained()
                            && t.record.phase == KeepPhase::Completed
                    })
                    .map(|t| KeptFile {
                        plan: t.record.committed_plan().expect("completed winner").clone(),
                        checked: t.checked,
                    }),
            );
        }
        let transfers = if bound {
            self.transfers
                .iter()
                .filter(|t| t.record.public_retained())
                .map(|t| self.transfer_state(t))
                .collect()
        } else {
            Vec::new()
        };
        let error = self
            .fault
            .clone()
            .or_else(|| {
                self.transfers
                    .iter()
                    .any(|transfer| transfer.poisoned)
                    .then(|| "retained transfer needs a storage check".into())
            })
            .or_else(|| {
                (!bound).then(|| "retained storage is not attached to a server identity".into())
            });
        KeptFiles {
            supported: true,
            allocated_bytes: if self.fault.is_some() {
                KEPT_BYTE_LIMIT
            } else {
                known
            },
            limit_bytes: KEPT_BYTE_LIMIT,
            files,
            transfers,
            error,
        }
    }

    fn begin_keep(&mut self, plan: KeepPlan) -> Result<KeepRun, StorageError> {
        let listing = self
            .transfers
            .iter()
            .find(|t| t.record.owner() == Owner::Keep(plan.cid))
            .map_or(plan.version, |t| t.record.listing);
        self.begin_with_listing(plan, listing)
    }
    fn begin_keep_with_listing(
        &mut self,
        plan: KeepPlan,
        listing: [u8; 32],
    ) -> Result<KeepRun, StorageError> {
        self.begin_with_listing(plan, listing)
    }
    fn kept_saved_plans(&self, cid: &Cid) -> Option<Vec<KeepPlan>> {
        if self.ready().is_err() || self.scope.is_none() {
            return None;
        }
        self.transfers
            .iter()
            .find(|t| {
                t.record.owner() == Owner::Keep(*cid) && t.record.phase != KeepPhase::Deleting
            })
            .map(|t| t.record.plans().cloned().collect())
            .or_else(|| {
                self.copies
                    .iter()
                    .find(|c| c.file.plan.cid == *cid)
                    .map(|c| vec![c.file.plan.clone()])
            })
    }
    fn begin_keep_variants(
        &mut self,
        plans: Vec<KeepPlan>,
        selected: usize,
        listing: [u8; 32],
    ) -> Result<KeepRun, StorageError> {
        let plan = plans.get(selected).ok_or(StorageError::Malformed)?.clone();
        self.begin_with_candidates(plan, listing, Some((plans, selected)))
    }
    fn fallback_keep(
        &mut self,
        run: KeepRun,
        expected: &[u8; 32],
        tried: &[[u8; 32]],
        bad_current: bool,
    ) -> Result<Option<(KeepRun, KeepPlan)>, StorageError> {
        self.ready()?;
        let active = self
            .active
            .as_ref()
            .filter(|a| a.token == run && &a.plan.version == expected)
            .ok_or(StorageError::Malformed)?;
        let Some(index) = active.transfer else {
            return Ok(None);
        };
        let record = &self.transfers[index].record;
        if record.exhausted
            || record.user_paused
            || record.blocked
            || record.phase == KeepPhase::Deleting
        {
            return Err(StorageError::Malformed);
        }
        let (next, start) = record.fallback(expected, tried, bad_current)?;
        let Some(next) = next else {
            return Ok(None);
        };
        let token = if start { Some(KeepRun::fresh()?) } else { None };
        let invalidated_winner = bad_current && record.selected_is_winner();
        let plan = next.plan.clone();
        // Synchronously revoke all old queued lower effects BEFORE attempting publication. A
        // failed publication poisons the journal (or, for a clean disk-full refusal before the
        // rename, leaves the previous record in force) and never restores their authority or
        // budget: the active run stays revoked either way.
        self.active = None;
        if invalidated_winner {
            self.transfers[index].checked = false;
        }
        self.transfers[index].publish(next, &self.metadata_key, &mut self.rng)?;
        if let Some(token) = token {
            self.active = Some(Active {
                token,
                identity: self.transfers[index].record.owner(),
                plan: plan.clone(),
                transfer: Some(index),
                seen: HashSet::new(),
                window: None,
                owner: tokio::sync::watch::channel(true).0,
            });
            Ok(Some((token, plan)))
        } else {
            Ok(None)
        }
    }
    fn keep_work(
        &mut self,
        token: KeepRun,
        network: Option<KeepNetworkWork>,
        mono: u64,
    ) -> Result<KeepWork, StorageError> {
        self.ready()?;
        let active = self
            .active
            .as_mut()
            .filter(|active| active.token == token)
            .ok_or(StorageError::Malformed)?;
        let Some(index) = active.transfer else {
            // Old completed repair has no V2 execution journal. It is not resumable new work.
            return Ok(KeepWork {
                run: token,
                grant: 0,
                deadline: mono
                    .checked_add(KEEP_EXECUTION_SLICE_MS)
                    .ok_or(StorageError::Malformed)?,
                owner: active.owner.subscribe(),
            });
        };
        let transfer = &mut self.transfers[index];
        if transfer.poisoned || transfer.record.phase == KeepPhase::Deleting {
            return Err(StorageError::Malformed);
        }
        if transfer.record.exhausted {
            return Err(StorageError::KeepExhausted);
        }
        if network.is_some_and(|work| {
            work.bytes == 0 || work.bytes > catcoms_rt::MAX_REQUEST_RESPONSE_FRAME_BYTES as u64
        }) {
            return Err(StorageError::BlobSizeLimit);
        }
        // Do not begin a normal request near the tail of a slice then artificially shorten its
        // released timeout. Burn the unused tail and prepay a fresh slice instead; no refund.
        let required_ms = if network.is_some() {
            catcoms_rt::REQUEST_TIMEOUT_MS + 2_000
        } else {
            0
        };
        let expired = active
            .window
            .as_ref()
            .is_none_or(|window| mono.saturating_add(required_ms) >= window.deadline);
        if expired {
            active.window = None;
        }
        let missing_network = network.is_some_and(|work| {
            active.window.as_ref().is_none_or(|window| {
                window.remaining.requests == 0
                    || (work.paged && window.remaining.pages == 0)
                    || window.remaining.bytes < work.bytes
            })
        });
        if expired || missing_network {
            let zero = KeepAllowance {
                requests: 0,
                pages: 0,
                bytes: 0,
                slices: 0,
            };
            let mut reserve = zero;
            reserve.slices = u16::from(expired);
            if let Some(work) = network.filter(|_| missing_network) {
                reserve.requests = transfer.record.remaining.requests.min(16);
                reserve.pages = if work.paged {
                    transfer.record.remaining.pages.min(16)
                } else {
                    0
                };
                reserve.bytes = transfer
                    .record
                    .remaining
                    .bytes
                    .min((work.bytes * 16).clamp(256 * 1024, 16 * 1024 * 1024));
            }
            let prior = active
                .window
                .as_ref()
                .map_or(zero, |window| window.remaining);
            let insufficient = network.is_some_and(|work| {
                missing_network
                    && (prior.requests + reserve.requests == 0
                        || (work.paged && prior.pages + reserve.pages == 0)
                        || prior.bytes + reserve.bytes < work.bytes)
            });
            let deadline = active.window.as_ref().map_or_else(
                || {
                    mono.checked_add(KEEP_EXECUTION_SLICE_MS)
                        .ok_or(StorageError::Malformed)
                },
                |window| Ok(window.deadline),
            )?;
            let mut next = transfer.record.successor()?;
            if insufficient || next.remaining.subtract(reserve).is_err() {
                next.exhausted = true;
                active.owner.send_replace(false);
                transfer.publish(next, &self.metadata_key, &mut self.rng)?;
                return Err(StorageError::KeepExhausted);
            }
            // Durable debit BEFORE dispatch. A crash loses unused batch capacity and the entire
            // slice; reopening never interprets a timestamp as another execution allowance.
            transfer.publish(next, &self.metadata_key, &mut self.rng)?;
            let window = active.window.get_or_insert(WorkWindow {
                deadline,
                remaining: zero,
            });
            window.remaining.requests += reserve.requests;
            window.remaining.pages += reserve.pages;
            window.remaining.bytes += reserve.bytes;
        }
        let window = active.window.as_mut().expect("prepaid execution window");
        if let Some(work) = network {
            window.remaining.requests -= 1;
            window.remaining.pages -= u32::from(work.paged);
            window.remaining.bytes -= work.bytes;
        }
        Ok(KeepWork {
            run: token,
            grant: transfer.record.grant,
            deadline: window.deadline,
            owner: active.owner.subscribe(),
        })
    }
    fn validate_keep_work(&self, work: &KeepWork, mono: u64) -> Result<(), StorageError> {
        self.ready()?;
        if !work.run_is_live() {
            return Err(StorageError::Malformed);
        }
        let active = self
            .active
            .as_ref()
            .filter(|active| active.token == work.run)
            .ok_or(StorageError::Malformed)?;
        if mono >= work.deadline {
            return Err(StorageError::KeepWorkExpired);
        }
        if let Some(index) = active.transfer {
            let transfer = &self.transfers[index];
            if transfer.poisoned
                || transfer.record.grant != work.grant
                || transfer.record.exhausted
                || transfer.record.phase == KeepPhase::Deleting
            {
                return Err(StorageError::Malformed);
            }
        } else if work.grant != 0 {
            return Err(StorageError::Malformed);
        }
        Ok(())
    }
    fn retry_kept(
        &mut self,
        cid: &Cid,
        request: KeepRetry,
    ) -> Result<KeepRetryReceipt, StorageError> {
        self.retry_owned(Owner::Keep(*cid), request)
    }
    fn put_keep(&mut self, token: KeepRun, bytes: &[u8]) -> Result<(), StorageError> {
        self.put_keep_with_barriers(token, bytes, flush, persistence::directory_barrier)
    }
    fn get_keep(
        &self,
        token: KeepRun,
        cid: &Cid,
        max: usize,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let active = self
            .active
            .as_ref()
            .filter(|active| active.token == token)
            .ok_or(StorageError::Malformed)?;
        let bound = active
            .plan
            .chunks
            .iter()
            .find(|(address, _)| address == cid)
            .ok_or(StorageError::Malformed)?
            .1;
        if let Some(index) = active.transfer {
            let transfer = &self.transfers[index];
            if transfer.poisoned || transfer.record.phase == KeepPhase::Deleting {
                return Err(StorageError::Malformed);
            }
            transfer.store.get_bounded(cid, max.min(bound as usize))
        } else {
            self.retained_get(cid, max.min(bound as usize))
        }
    }
    fn finish_keep(&mut self, token: KeepRun) -> Result<(), StorageError> {
        self.finish_keep_with_sync(token, persistence::directory_barrier)
    }
    fn pause_keep(&mut self, token: KeepRun) -> Result<(), StorageError> {
        if self
            .active
            .as_ref()
            .is_none_or(|active| active.token != token)
        {
            return Err(StorageError::Malformed);
        }
        self.active = None;
        Ok(())
    }
    fn block_keep(&mut self, run: KeepRun) -> Result<(), StorageError> {
        self.ready()?;
        let active = self
            .active
            .as_ref()
            .filter(|active| active.token == run)
            .ok_or(StorageError::Malformed)?;
        if let Some(index) = active.transfer {
            let mut next = self.transfers[index].record.successor()?;
            next.blocked = true;
            self.active = None;
            self.transfers[index].publish(next, &self.metadata_key, &mut self.rng)?;
        } else {
            self.active = None;
        }
        Ok(())
    }
    fn keep_progress(&self, cid: &Cid) -> Option<KeepProgress> {
        self.transfers
            .iter()
            .find(|transfer| transfer.record.owner() == Owner::Keep(*cid))
            .map(|transfer| KeepProgress {
                phase: transfer.record.phase,
                verified_chunks: transfer.record.verified.iter().filter(|bit| **bit).count(),
                running: self
                    .active
                    .as_ref()
                    .is_some_and(|active| active.identity == Owner::Keep(*cid)),
                user_paused: transfer.record.user_paused,
                blocked: transfer.record.blocked,
                exhausted: transfer.record.exhausted,
            })
            .or_else(|| {
                self.copies
                    .iter()
                    .find(|copy| copy.file.plan.cid == *cid)
                    .map(|copy| KeepProgress {
                        phase: KeepPhase::Completed,
                        verified_chunks: if copy.file.checked {
                            copy.file.plan.chunks.len()
                        } else {
                            0
                        },
                        running: self
                            .active
                            .as_ref()
                            .is_some_and(|active| active.identity == Owner::Keep(*cid)),
                        user_paused: false,
                        blocked: false,
                        exhausted: false,
                    })
            })
    }
    fn keep_run_current(&self, run: KeepRun, cid: &Cid, version: &[u8; 32]) -> bool {
        self.ready().is_ok()
            && self.active.as_ref().is_some_and(|active| {
                active.token == run
                    && active.identity == Owner::Keep(*cid)
                    && active.plan.cid == *cid
                    && active.plan.version == *version
                    && *active.owner.borrow()
                    && active
                        .transfer
                        .is_none_or(|index| !self.transfers[index].poisoned)
            })
    }
    fn keep_plausibly_local(&self, cid: &Cid) -> bool {
        self.ready().is_ok()
            && self
                .transfers
                .iter()
                .find(|transfer| transfer.record.owner() == Owner::Keep(*cid))
                .is_some_and(|transfer| {
                    !transfer.poisoned
                        && transfer.record.plans().any(|plan| {
                            !transfer.record.plan_is_bad(&plan.version)
                                && plan.chunks.iter().all(|(chunk, _)| {
                                    transfer.store.has(chunk) || self.primary.has(chunk)
                                })
                        })
                })
    }
    fn automatic_keep_candidate(&self, after: Option<Cid>) -> Option<Cid> {
        if self.ready().is_err() || self.active.is_some() {
            return None;
        }
        let candidates = || {
            self.transfers
                .iter()
                .filter(|transfer| {
                    !transfer.poisoned
                        && transfer.record.public_retained()
                        && transfer.record.phase == KeepPhase::Resumable
                        && !transfer.record.user_paused
                        && !transfer.record.blocked
                        && !transfer.record.exhausted
                })
                .map(|transfer| transfer.record.plan.cid)
        };
        candidates()
            .filter(|cid| after.is_none_or(|previous| cid.as_bytes() > previous.as_bytes()))
            .min_by_key(|cid| *cid.as_bytes())
            .or_else(|| candidates().min_by_key(|cid| *cid.as_bytes()))
    }
    fn set_keep_paused(
        &mut self,
        cid: &Cid,
        owner: [u8; 32],
        expected_intent: u64,
        paused: bool,
    ) -> Result<(), StorageError> {
        self.set_owned_paused(Owner::Keep(*cid), owner, expected_intent, paused)
    }
    fn abort_keep(&mut self, token: KeepRun) -> Result<(), StorageError> {
        let active = self
            .active
            .as_ref()
            .filter(|active| active.token == token)
            .ok_or(StorageError::Malformed)?;
        if let Some(index) = active.transfer {
            // A check/repair run must never erase the completed copy it was checking.
            if self.transfers[index].record.phase != KeepPhase::Completed {
                return self.release_transfer(index);
            }
        }
        self.pause_keep(token)
    }
    fn forget_kept(&mut self, cid: &Cid) -> Result<(), StorageError> {
        self.ready()?;
        if let Some(index) = self
            .transfers
            .iter()
            .position(|transfer| transfer.record.owner() == Owner::Keep(*cid))
        {
            return self.release_transfer(index);
        }
        if self.active.is_some() {
            return Err(StorageError::Io("pause the active kept copy first".into()));
        }
        let index = self
            .copies
            .iter()
            .position(|copy| copy.file.plan.cid == *cid)
            .ok_or(StorageError::Malformed)?;
        self.copies[index].file.checked = false;
        self.read_epoch.retire();
        // Legacy pending is always cleanup-only. It can never be adopted as V2 resume work.
        fs::rename(self.dir.join(cid.to_hex()), self.dir.join(PENDING)).map_err(io)?;
        if let Err(error) = sync_dir(&self.dir).and_then(|_| remove_copy(&self.dir.join(PENDING))) {
            self.fault = Some("kept-copy removal is incomplete; restart to retry cleanup".into());
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
    fn prepare_read_bounded(
        &self,
        cid: &Cid,
        max: usize,
    ) -> Result<crate::BlobReadPlan, StorageError> {
        let mut plan = match self.primary.prepare_read_bounded(cid, max) {
            Ok(plan) => plan,
            Err(error) => crate::BlobReadPlan::failed(*cid, max, error, self.read_epoch.guard())?,
        };
        if self.scope.is_none() {
            return Ok(plan);
        }
        let scope = self.read_epoch.guard();
        // Retained fallbacks are captured only from public completed ownership. Never use
        // store.cids() or the shared transfer path to infer authority over private alternatives.
        for transfer in &self.transfers {
            if !transfer.poisoned
                && transfer.record.public_retained()
                && transfer
                    .record
                    .committed_plan()
                    .is_some_and(|p| p.chunks.iter().any(|(id, _)| id == cid))
            {
                plan = plan.then(
                    transfer
                        .store
                        .prepare_read_bounded(cid, max)?
                        .with_parent(&scope)?
                        .with_parent(&transfer.read_epoch.guard())?,
                )?;
            }
        }
        for copy in &self.copies {
            if copy.file.plan.chunks.iter().any(|(id, _)| id == cid) {
                plan = plan.then(
                    copy.store
                        .prepare_read_bounded(cid, max)?
                        .with_parent(&scope)?,
                )?;
            }
        }
        Ok(plan)
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
            || (self.scope.is_some()
                && (self.copies.iter().any(|c| {
                    c.file.plan.chunks.iter().any(|(id, _)| id == cid) && c.store.has(cid)
                }) || self.transfers.iter().any(|t| {
                    !t.poisoned
                        && t.record.public_retained()
                        && t.record.phase == KeepPhase::Completed
                        && t.record
                            .committed_plan()
                            .is_some_and(|plan| plan.chunks.iter().any(|(id, _)| id == cid))
                        && t.store.has(cid)
                })))
    }
    fn delete(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        self.primary.delete(cid)
    }
    fn cids(&self) -> Vec<Cid> {
        let mut cids = self.primary.cids();
        if self.scope.is_some() {
            for c in &self.copies {
                cids.extend(c.store.cids());
            }
            for transfer in &self.transfers {
                if !transfer.poisoned
                    && transfer.record.public_retained()
                    && transfer.record.phase == KeepPhase::Completed
                {
                    // Uncommitted candidate chunks share the private directory, never its
                    // public address list. Only the committed winner's exact leaves are visible.
                    cids.extend(
                        transfer
                            .record
                            .committed_plan()
                            .into_iter()
                            .flat_map(|plan| {
                                plan.chunks
                                    .iter()
                                    .filter(|(cid, _)| transfer.store.has(cid))
                                    .map(|(cid, _)| *cid)
                            }),
                    );
                }
            }
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
mod durable_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod variant_tests;
