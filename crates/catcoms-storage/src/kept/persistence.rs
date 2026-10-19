//! Filesystem publication for retained transfers. Fixed `current`/`next` roles bound physical
//! replacement space and make ambiguous debit/deletion publication fail closed across restart.
use super::record::{KeepScope, Record, VARIANT_CHUNK_LIMIT, VARIANT_RECORD_LIMIT};
use super::*;
use std::io::Write;

pub(super) const FORMAT: &str = "format-v2";
pub(super) const TRANSFERS: &str = "transfers-v2";
pub(super) const DELETING: &str = "deleting-v2";
pub(super) const FORMAT_CHARGE: u64 = 4096;
const CURRENT: &str = "current";
const NEXT: &str = "next";
const FORMAT_DOMAIN: &[u8] = b"mewtual/kept-format/v2";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WritePhase {
    Created,
    Written,
    FileFlushed,
    Renamed,
    DirectoryFlushed,
}

/// Unlike the legacy file-flush-only storage seam, V2 allowance publication requires the
/// directory barrier too. A failed/unsupported barrier cannot authorize a request or refund work.
pub(super) fn directory_barrier(path: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    let result = fs::File::open(path).and_then(|file| file.sync_all());
    #[cfg(windows)]
    let result = {
        use std::os::windows::fs::OpenOptionsExt;
        // Named Win32 flag values, using std's extension API rather than another dependency.
        // Match the native store writer: writable directory handle and full sharing so this
        // short flush does not exclude replacement/deletion by an already-open sibling handle.
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_SHARE_READ_WRITE_DELETE: u32 = 0x0000_0007;
        fs::OpenOptions::new()
            .write(true)
            .share_mode(FILE_SHARE_READ_WRITE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .and_then(|file| file.sync_all())
    };
    #[cfg(not(any(unix, windows)))]
    let result: std::io::Result<()> = Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "retained transfer directory flush is unavailable on this platform",
    ));
    result.map_err(|error| StorageError::CommittedButNotDurable(error.to_string()))
}

fn format_bytes(scope: &KeepScope) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(FORMAT_DOMAIN).expect("fixed domain");
    e.put_bytes(&scope.group).expect("bounded group");
    e.put_bytes(scope.device.as_bytes()).expect("fixed device");
    e.finish()
}
/// The permanent regular-file barrier makes old readers refuse this retained namespace before
/// they can reinterpret V2 partial work. It is never removed by release/quota cleanup. The same
/// root lease used for legacy inventory must already be held, and old pending cleaned beforehand.
pub(super) fn ensure_format<R: CryptoRngCore>(
    dir: &Path,
    scope: &KeepScope,
    key: &[u8; 32],
    rng: &mut R,
) -> Result<(), StorageError> {
    scope.validate()?;
    let path = dir.join(FORMAT);
    let plain = format_bytes(scope);
    match read_bounded(&path, FORMAT_CHARGE as usize)? {
        Some(encoded) => {
            let saved = Zeroizing::new(unseal(key, &decode_sealed(&encoded)?)?);
            if *saved != plain {
                return Err(StorageError::Malformed);
            }
            flush(&path)?;
            directory_barrier(dir)?;
        }
        None => {
            let encoded = encode_sealed(&seal(key, &plain, rng)?);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(io)?;
            // An incomplete barrier remains recognizable and faults; never remove it and let
            // an old reader mount a namespace whose migration may already have begun.
            file.write_all(&encoded).map_err(io)?;
            file.sync_all().map_err(io)?;
            drop(file);
            directory_barrier(dir)?;
        }
    }
    for name in [TRANSFERS, DELETING] {
        create_blob_directory(&dir.join(name))?;
    }
    directory_barrier(dir)
}

pub(super) struct Transfer<R: CryptoRngCore> {
    pub record: Record,
    pub checked: bool,
    /// Separate from working-candidate runs: only committed-winner replacement/poison/deletion
    /// retires public read results. A debit or private candidate switch cannot revoke that winner.
    pub read_epoch: crate::blob_read::ReadEpoch,
    pub store: SealingBlobStore<SharedRng<R>>,
    pub path: PathBuf,
    /// Set on any attempted publication error, including a visible-but-unflushed rename.
    /// Old in-memory state cannot continue to serve or execute as though the write rolled back.
    pub poisoned: bool,
}

fn read_record(path: &Path, key: &[u8; 32]) -> Result<Option<Record>, StorageError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err(StorageError::Malformed),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io(error)),
    }
    read_bounded(path, VARIANT_RECORD_LIMIT)?
        .map(|encoded| {
            let plain = Zeroizing::new(unseal(key, &decode_sealed(&encoded)?)?);
            Record::decode(&plain)
        })
        .transpose()
}
fn validate_leaves(path: &Path, record: &Record) -> Result<(), StorageError> {
    for leaf in entries(path, VARIANT_CHUNK_LIMIT + 2)? {
        let metadata = fs::symlink_metadata(&leaf).map_err(io)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(StorageError::Malformed);
        }
        let name = leaf
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(StorageError::Malformed)?;
        let limit = if matches!(name, CURRENT | NEXT) {
            record.slot_limit() as u64
        } else {
            let cid = Cid::from_hex(name)
                .filter(|cid| cid.to_hex() == name)
                .ok_or(StorageError::Malformed)?;
            record
                .plans()
                .flat_map(|plan| &plan.chunks)
                .filter(|(address, _)| *address == cid)
                .map(|(_, bound)| *bound + 40)
                .max()
                .ok_or(StorageError::Malformed)?
        };
        if metadata.len() > limit {
            return Err(StorageError::BlobSizeLimit);
        }
    }
    Ok(())
}

impl<R: CryptoRngCore> Transfer<R> {
    pub(super) fn open(
        path: PathBuf,
        scope: &KeepScope,
        key: [u8; 32],
        metadata_key: [u8; 32],
        rng: SharedRng<R>,
    ) -> Result<Self, StorageError> {
        Self::open_with_barriers(
            path,
            scope,
            key,
            metadata_key,
            rng,
            flush,
            directory_barrier,
        )
    }
    pub(super) fn open_with_barriers(
        path: PathBuf,
        scope: &KeepScope,
        key: [u8; 32],
        metadata_key: [u8; 32],
        rng: SharedRng<R>,
        flush_record: impl Fn(&Path) -> Result<(), StorageError>,
        sync_directory: impl Fn(&Path) -> Result<(), StorageError>,
    ) -> Result<Self, StorageError> {
        let metadata = fs::symlink_metadata(&path).map_err(io)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(StorageError::Malformed);
        }
        let current = read_record(&path.join(CURRENT), &metadata_key)?;
        let next = read_record(&path.join(NEXT), &metadata_key)?;
        // Reconciliation itself writes. Validate scope AND canonical path before any flush or
        // rename, including a valid but misplaced initial record or successor.
        for record in current.iter().chain(next.iter()) {
            if record.scope != *scope
                || path.file_name().and_then(|name| name.to_str())
                    != Some(record.plan.cid.to_hex().as_str())
            {
                return Err(StorageError::Malformed);
            }
        }
        // Authentication failure in EITHER file already refused above. Never choose a valid
        // older record merely because the newer one was damaged or incompletely written.
        let record = match (current, next) {
            (Some(current), None) => {
                validate_leaves(&path, &current)?;
                // A previous mount may have observed rename but failed its final barrier.
                // Re-establish both barriers before returning any usable execution owner.
                flush_record(&path.join(CURRENT))?;
                sync_directory(&path)?;
                current
            }
            (Some(current), Some(next)) => {
                current.validate_successor(&next)?;
                if current.scope != *scope {
                    return Err(StorageError::Malformed);
                }
                validate_leaves(&path, &current)?;
                // Readable sealed bytes are not evidence that the previous file flush finished.
                flush_record(&path.join(NEXT))?;
                fs::rename(path.join(NEXT), path.join(CURRENT)).map_err(io)?;
                sync_directory(&path)?;
                next
            }
            (None, Some(next)) => {
                next.validate_initial()?;
                if entries(&path, 1)? != [path.join(NEXT)] {
                    return Err(StorageError::Malformed);
                }
                validate_leaves(&path, &next)?;
                if next.scope != *scope {
                    return Err(StorageError::Malformed);
                }
                flush_record(&path.join(NEXT))?;
                fs::rename(path.join(NEXT), path.join(CURRENT)).map_err(io)?;
                sync_directory(&path)?;
                next
            }
            (None, None) => return Err(StorageError::Malformed),
        };
        if record.scope != *scope
            || path.file_name().and_then(|name| name.to_str())
                != Some(record.plan.cid.to_hex().as_str())
        {
            return Err(StorageError::Malformed);
        }
        validate_leaves(&path, &record)?;
        let store = SealingBlobStore::open(&path, key, rng)?;
        Ok(Self {
            record,
            checked: false,
            read_epoch: crate::blob_read::ReadEpoch::default(),
            store,
            path,
            poisoned: false,
        })
    }
    pub(super) fn create(
        path: PathBuf,
        record: Record,
        key: [u8; 32],
        metadata_key: [u8; 32],
        rng: SharedRng<R>,
    ) -> Result<Self, StorageError> {
        Self::create_with_hook(path, record, key, metadata_key, rng, |_, _| Ok(()))
    }
    pub(super) fn create_with_hook(
        path: PathBuf,
        record: Record,
        key: [u8; 32],
        metadata_key: [u8; 32],
        rng: SharedRng<R>,
        hook: impl FnMut(WritePhase, &Path) -> Result<(), StorageError>,
    ) -> Result<Self, StorageError> {
        record.validate_initial()?;
        if path.file_name().and_then(|name| name.to_str())
            != Some(record.plan.cid.to_hex().as_str())
        {
            return Err(StorageError::Malformed);
        }
        fs::create_dir(&path).map_err(io)?;
        if let Some(parent) = path.parent() {
            directory_barrier(parent)?;
        }
        let store = SealingBlobStore::open(&path, key, rng.clone())?;
        let mut transfer = Self {
            record: record.clone(),
            checked: false,
            read_epoch: crate::blob_read::ReadEpoch::default(),
            store,
            path,
            poisoned: false,
        };
        // Initial publication is the same fixed pending-file transaction, with no predecessor.
        transfer.write_with_hook(&record, &metadata_key, &mut rng.clone(), hook)?;
        Ok(transfer)
    }
    fn write(
        &mut self,
        next: &Record,
        key: &[u8; 32],
        rng: &mut SharedRng<R>,
    ) -> Result<(), StorageError> {
        self.write_with_hook(next, key, rng, |_, _| Ok(()))
    }
    fn write_with_hook(
        &mut self,
        next: &Record,
        key: &[u8; 32],
        rng: &mut SharedRng<R>,
        mut hook: impl FnMut(WritePhase, &Path) -> Result<(), StorageError>,
    ) -> Result<(), StorageError> {
        let result = (|| {
            let plain = Zeroizing::new(next.encode());
            if plain.len() + 40 > next.slot_limit() {
                return Err(StorageError::BlobSizeLimit);
            }
            let encoded = encode_sealed(&seal(key, &plain, rng)?);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.path.join(NEXT))
                .map_err(io)?;
            hook(WritePhase::Created, &self.path.join(NEXT))?;
            file.write_all(&encoded).map_err(io)?;
            hook(WritePhase::Written, &self.path.join(NEXT))?;
            file.sync_all().map_err(io)?;
            drop(file);
            hook(WritePhase::FileFlushed, &self.path.join(NEXT))?;
            // std uses same-directory replacement on Windows as well. Deleting current first
            // would create a crash window with neither a durable debit nor a safe predecessor.
            fs::rename(self.path.join(NEXT), self.path.join(CURRENT)).map_err(io)?;
            hook(WritePhase::Renamed, &self.path.join(CURRENT))?;
            directory_barrier(&self.path)?;
            hook(WritePhase::DirectoryFlushed, &self.path)
        })();
        if result.is_err() {
            self.poisoned = true;
            self.read_epoch.retire();
        }
        result
    }
    pub(super) fn publish(
        &mut self,
        next: Record,
        key: &[u8; 32],
        rng: &mut SharedRng<R>,
    ) -> Result<(), StorageError> {
        if self.poisoned {
            return Err(StorageError::Io(
                "retained transfer needs a storage check".into(),
            ));
        }
        self.record.validate_successor(&next)?;
        self.write(&next, key, rng)?;
        if self.record.committed_plan() != next.committed_plan() {
            self.read_epoch.retire();
        }
        self.record = next;
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn publish_with_hook(
        &mut self,
        next: Record,
        key: &[u8; 32],
        rng: &mut SharedRng<R>,
        hook: impl FnMut(WritePhase, &Path) -> Result<(), StorageError>,
    ) -> Result<(), StorageError> {
        if self.poisoned {
            return Err(StorageError::Malformed);
        }
        self.record.validate_successor(&next)?;
        self.write_with_hook(&next, key, rng, hook)?;
        if self.record.committed_plan() != next.committed_plan() {
            self.read_epoch.retire();
        }
        self.record = next;
        Ok(())
    }
}

/// Only a transfer already durably marked Deleting may move here. After the move, even loss
/// of current/next leaves a non-serving cleanup-only directory, never a new/resumable transfer.
pub(super) fn finish_deleting(path: &Path) -> Result<(), StorageError> {
    let root = fs::symlink_metadata(path).map_err(io)?;
    if !root.is_dir() || root.file_type().is_symlink() {
        return Err(StorageError::Malformed);
    }
    let leaves = entries(path, VARIANT_CHUNK_LIMIT + 2)?;
    for leaf in &leaves {
        let metadata = fs::symlink_metadata(leaf).map_err(io)?;
        let name = leaf
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(StorageError::Malformed)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || (!matches!(name, CURRENT | NEXT)
                && Cid::from_hex(name).is_none_or(|cid| cid.to_hex() != name))
            || metadata.len() > (CHUNK_LIMIT_BYTES + 40).max(VARIANT_RECORD_LIMIT as u64)
        {
            return Err(StorageError::Malformed);
        }
    }
    for leaf in leaves {
        fs::remove_file(leaf).map_err(io)?;
    }
    fs::remove_dir(path).map_err(io)?;
    if let Some(parent) = path.parent() {
        directory_barrier(parent)?;
    }
    Ok(())
}
