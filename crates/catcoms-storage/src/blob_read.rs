//! One native-owned, bounded, blocking read. Preparation never opens disk files or accepts a
//! caller path/key. The service layer prepays scratch before preparation and retains that exact
//! permit through actual worker return, independently of its deadline or reply waiter.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use catcoms_crypto::{unseal, SealedBlob};
use zeroize::Zeroizing;

use crate::{Cid, StorageError};

const MAX_BYTES: usize = catcoms_rt::MAX_REQUEST_RESPONSE_FRAME_BYTES;
const MAX_SOURCES: usize = 33; // primary plus every possible completed retained owner
const MAX_PATH_BYTES: usize = 4096;
const METADATA_BOUND: usize = 256 * 1024;

/// Non-cloneable storage epoch. Outstanding read guards never keep their store/winner alive.
/// Replacing this epoch is a synchronous revocation; restoring another store creates a new one.
#[derive(Default, Debug)]
pub(crate) struct ReadEpoch(Arc<()>);
impl ReadEpoch {
    pub(crate) fn retire(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn guard(&self) -> ReadGuard {
        ReadGuard(vec![Arc::downgrade(&self.0)])
    }
}
#[derive(Clone)]
pub(crate) struct ReadGuard(Vec<Weak<()>>);
impl ReadGuard {
    fn current(&self) -> bool {
        self.0.iter().all(|epoch| epoch.strong_count() != 0)
    }
    fn combine(&mut self, other: &Self) -> Result<(), StorageError> {
        // Native wrappers are bounded too; an accidentally recursive store cannot build an
        // unlimited chain of parent capabilities while claiming the fixed metadata allowance.
        if self.0.len() + other.0.len() > 8 {
            return Err(StorageError::BlobSizeLimit);
        }
        self.0.extend(other.0.iter().cloned());
        Ok(())
    }
}

/// Keep preparation diagnostics closed: arbitrary adapter error strings and their spare
/// capacity must not survive inside a fixed-size captured read plan. The worker's service
/// caller may report this fixed failure, while its independently authenticated fallback works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadFault {
    TooLarge,
    WrongContent,
    Unavailable,
}
impl ReadFault {
    fn capture(error: StorageError) -> Self {
        match error {
            StorageError::BlobSizeLimit => Self::TooLarge,
            StorageError::CidMismatch => Self::WrongContent,
            _ => Self::Unavailable,
        }
    }
    fn error(self) -> StorageError {
        match self {
            Self::TooLarge => StorageError::BlobSizeLimit,
            Self::WrongContent => StorageError::CidMismatch,
            Self::Unavailable => StorageError::Io("storage read could not be prepared".into()),
        }
    }
}
enum Source {
    Fault(ReadFault),
    Memory(Vec<u8>),
    Plain(PathBuf),
    Sealed {
        path: PathBuf,
        key: Zeroizing<[u8; 32]>,
    },
}
struct Candidate {
    source: Source,
    guard: ReadGuard,
}

/// Exact CID/bound and captured public storage sources, never staged/private partial content.
/// No Clone/serialization API exists. `run` must execute on the caller's owned blocking worker;
/// this type itself neither spawns tasks nor claims that dropping an upper future cancels I/O.
pub struct BlobReadPlan {
    cid: Cid,
    max: usize,
    sources: Vec<Candidate>,
}
/// Authenticated bytes plus the storage epoch that authorized their capture. Keep this result
/// and its prepaid scratch owner together through actor revalidation and page construction.
pub struct BlobReadResult {
    bytes: Vec<u8>,
    guard: ReadGuard,
}
impl std::fmt::Debug for BlobReadPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BlobReadPlan { .. }")
    }
}
impl std::fmt::Debug for BlobReadResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BlobReadResult { .. }")
    }
}
impl BlobReadResult {
    pub fn is_current(&self) -> bool {
        self.guard.current()
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
impl BlobReadPlan {
    /// Conservative explicit-buffer/metadata ceiling, not an estimate of allocator/OS overhead
    /// or a process-wide cap. It includes retained memory snapshots, one encoded disk buffer,
    /// decoded AEAD input/output and room for a page while the full result is still owned.
    /// For an 8 MiB+44 chunk this is slightly above32 MiB; services must not round it down.
    pub fn memory_bound(max: usize) -> Result<usize, StorageError> {
        if max > MAX_BYTES {
            return Err(StorageError::BlobSizeLimit);
        }
        max.checked_add(41)
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| n.checked_add(METADATA_BOUND))
            .ok_or(StorageError::BlobSizeLimit)
    }
    pub(crate) fn empty(cid: Cid, max: usize) -> Result<Self, StorageError> {
        Self::memory_bound(max)?;
        Ok(Self {
            cid,
            max,
            sources: Vec::new(),
        })
    }
    pub(crate) fn failed(
        cid: Cid,
        max: usize,
        error: StorageError,
        guard: ReadGuard,
    ) -> Result<Self, StorageError> {
        let mut plan = Self::empty(cid, max)?;
        plan.sources.push(Candidate {
            source: Source::Fault(ReadFault::capture(error)),
            guard,
        });
        Ok(plan)
    }
    pub(crate) fn memory(
        cid: Cid,
        max: usize,
        bytes: Option<&[u8]>,
        guard: ReadGuard,
    ) -> Result<Self, StorageError> {
        let mut plan = Self::empty(cid, max)?;
        if let Some(bytes) = bytes {
            if bytes.len() > max {
                return Err(StorageError::BlobSizeLimit);
            }
            let mut snapshot = Vec::new();
            snapshot
                .try_reserve_exact(bytes.len())
                .map_err(|_| StorageError::BlobSizeLimit)?;
            snapshot.extend_from_slice(bytes);
            if snapshot.capacity() > max {
                return Err(StorageError::BlobSizeLimit);
            }
            plan.sources.push(Candidate {
                source: Source::Memory(snapshot),
                guard,
            });
        }
        Ok(plan)
    }
    pub(crate) fn plain(
        cid: Cid,
        max: usize,
        path: PathBuf,
        guard: ReadGuard,
    ) -> Result<Self, StorageError> {
        Self::file(cid, max, path, None, guard)
    }
    pub(crate) fn sealed(
        cid: Cid,
        max: usize,
        path: PathBuf,
        key: [u8; 32],
        guard: ReadGuard,
    ) -> Result<Self, StorageError> {
        Self::file(cid, max, path, Some(key), guard)
    }
    fn file(
        cid: Cid,
        max: usize,
        path: PathBuf,
        key: Option<[u8; 32]>,
        guard: ReadGuard,
    ) -> Result<Self, StorageError> {
        let mut plan = Self::empty(cid, max)?;
        if path.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
            return Err(StorageError::BlobSizeLimit);
        }
        // PathBuf::join may retain geometric spare capacity even when its encoded length fits.
        // Reconstruct a compact owner and bound that allocation, not the caller's old buffer.
        let compact = path.as_os_str().to_os_string();
        if compact.capacity() > MAX_PATH_BYTES {
            return Err(StorageError::BlobSizeLimit);
        }
        let path = PathBuf::from(compact);
        let source = match key {
            Some(key) => Source::Sealed {
                path,
                key: Zeroizing::new(key),
            },
            None => Source::Plain(path),
        };
        plan.sources.push(Candidate { source, guard });
        Ok(plan)
    }
    pub(crate) fn with_parent(mut self, parent: &ReadGuard) -> Result<Self, StorageError> {
        for candidate in &mut self.sources {
            candidate.guard.combine(parent)?;
        }
        Ok(self)
    }
    pub(crate) fn then(mut self, other: Self) -> Result<Self, StorageError> {
        let resident = |sources: &[Candidate]| {
            sources
                .iter()
                .filter_map(|candidate| match &candidate.source {
                    Source::Memory(bytes) => Some(bytes.capacity()),
                    _ => None,
                })
                .sum::<usize>()
        };
        if self.cid != other.cid
            || self.max != other.max
            || self.sources.len() + other.sources.len() > MAX_SOURCES
            || resident(&self.sources).saturating_add(resident(&other.sources)) > self.max
        {
            return Err(StorageError::BlobSizeLimit);
        }
        self.sources.extend(other.sources);
        Ok(self)
    }
    /// Perform only blocking storage work, with fixed pre-allocation and no general read-to-end
    /// growth. A corrupt primary may fall back to a captured committed copy, never a candidate
    /// added after preparation. Check the returned epoch again at the real signing/effect edge.
    pub fn run(self) -> Result<Option<BlobReadResult>, StorageError> {
        let mut error = None;
        for candidate in self.sources {
            if !candidate.guard.current() {
                continue;
            }
            let read = match candidate.source {
                Source::Fault(error) => Err(error.error()),
                Source::Memory(bytes) => Ok(Some(bytes)),
                Source::Plain(path) => read_fixed(&path, self.max),
                Source::Sealed { path, key } => {
                    read_fixed(&path, self.max + 40).and_then(|encoded| {
                        let Some(encoded) = encoded else {
                            return Ok(None);
                        };
                        if encoded.len() <= 24 {
                            return Err(StorageError::Malformed);
                        }
                        let nonce = encoded[..24]
                            .try_into()
                            .map_err(|_| StorageError::Malformed)?;
                        // The separate exact-size buffer is charged above; do not use the existing
                        // unbounded/growing blob reader or silently retain its allocation as well.
                        let mut ciphertext = Vec::new();
                        ciphertext
                            .try_reserve_exact(encoded.len() - 24)
                            .map_err(|_| StorageError::BlobSizeLimit)?;
                        ciphertext.extend_from_slice(&encoded[24..]);
                        unseal(&key, &SealedBlob { nonce, ciphertext })
                            .map(Some)
                            .map_err(StorageError::from)
                    })
                }
            };
            match read {
                Ok(Some(bytes)) if bytes.len() <= self.max && Cid::of(&bytes) == self.cid => {
                    if candidate.guard.current() {
                        return Ok(Some(BlobReadResult {
                            bytes,
                            guard: candidate.guard,
                        }));
                    }
                }
                Ok(Some(bytes)) => {
                    error = Some(if bytes.len() > self.max {
                        StorageError::BlobSizeLimit
                    } else {
                        StorageError::CidMismatch
                    })
                }
                Ok(None) => {}
                Err(fault) => error = Some(fault),
            }
        }
        match error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }
}

fn read_fixed(path: &Path, max: usize) -> Result<Option<Vec<u8>>, StorageError> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(StorageError::Io(error.to_string())),
    };
    let metadata = file
        .metadata()
        .map_err(|error| StorageError::Io(error.to_string()))?;
    if !metadata.is_file() {
        return Err(StorageError::Malformed);
    }
    if metadata.len() > max as u64 {
        return Err(StorageError::BlobSizeLimit);
    }
    let limit = max.checked_add(1).ok_or(StorageError::BlobSizeLimit)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(limit)
        .map_err(|_| StorageError::BlobSizeLimit)?;
    bytes.resize(limit, 0);
    let mut length = 0;
    while length < limit {
        match file.read(&mut bytes[length..]) {
            Ok(0) => break,
            Ok(read) => length += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(StorageError::Io(error.to_string())),
        }
    }
    if length > max {
        return Err(StorageError::BlobSizeLimit);
    }
    bytes.truncate(length);
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests;
