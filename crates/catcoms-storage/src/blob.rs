//! Content-addressed blob storage.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use crate::cid::Cid;
use crate::StorageError;

/// A content-addressed blob store. `put` returns the [`Cid`] of the bytes; `get`
/// re-verifies that what comes back matches the address (tamper detection).
pub trait BlobStore {
    /// Store `bytes`, returning their content address.
    fn put(&mut self, bytes: &[u8]) -> Result<Cid, StorageError>;

    /// Fetch the bytes for `cid`, verifying their integrity. `None` if absent.
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StorageError>;

    /// Whether the store currently holds `cid`.
    fn has(&self, cid: &Cid) -> bool;

    /// Remove `cid`. Returns whether it was present.
    fn delete(&mut self, cid: &Cid) -> Result<bool, StorageError>;

    /// All currently-held content addresses.
    fn cids(&self) -> Vec<Cid>;
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
/// without bound. This is a simple **size-bounded FIFO** interim — evicted blobs stay
/// re-fetchable by CID; the full holder-probe retention engine (never-evict-last-copy) is
/// a follow-up.
pub const DEFAULT_BLOB_BUDGET: usize = 128 * 1024 * 1024;

/// An in-memory, size-bounded blob store (FIFO eviction past the byte budget).
#[derive(Debug)]
pub struct MemoryBlobStore {
    blobs: HashMap<Cid, Vec<u8>>,
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
}

/// A filesystem blob store: each blob is a file named by its hex CID.
#[derive(Debug)]
pub struct FsBlobStore {
    dir: PathBuf,
}

impl FsBlobStore {
    /// Open (creating if needed) a blob store rooted at `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, StorageError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir).map_err(|e| StorageError::Io(e.to_string()))?;
        Ok(Self { dir })
    }

    fn path(&self, cid: &Cid) -> PathBuf {
        self.dir.join(cid.to_hex())
    }
}

impl BlobStore for FsBlobStore {
    fn put(&mut self, bytes: &[u8]) -> Result<Cid, StorageError> {
        let cid = Cid::of(bytes);
        let path = self.path(&cid);
        if !path.exists() {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn memory_store_roundtrips() {
        store_roundtrip(&mut MemoryBlobStore::new());
    }

    #[test]
    fn memory_store_evicts_oldest_over_budget() {
        let mut store = MemoryBlobStore::with_budget(20);
        let a = store.put(b"aaaaaaaaaa").unwrap(); // 10 bytes
        let b = store.put(b"bbbbbbbbbb").unwrap(); // 10 bytes — total 20, at budget
        assert!(store.has(&a) && store.has(&b));
        let c = store.put(b"cccccccccc").unwrap(); // 10 bytes — over budget, evicts oldest (a)
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
}
