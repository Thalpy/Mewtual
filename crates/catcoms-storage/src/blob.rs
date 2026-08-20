//! Content-addressed blob storage.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use catcoms_crypto::{seal, unseal, SealedBlob};
use catcoms_rt::CryptoRngCore;
use zeroize::Zeroizing;

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
/// without bound. This is a simple **size-bounded FIFO** interim; evicted blobs stay
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
        // Content addressing makes a healthy existing record safely deduplicable, but mere path
        // existence is not health: a truncated/tampered record must be replaceable after a peer
        // has served the CID-verified bytes again. We only overwrite after `get` has proved that
        // the existing record is absent or invalid, so the repair path never discards a valid
        // local copy.
        if !matches!(self.get(&cid), Ok(Some(_))) {
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
    /// Open (creating if needed) a sealing store rooted at `dir`, sealing under `key`.
    pub fn open(dir: impl AsRef<Path>, key: [u8; 32], rng: R) -> Result<Self, StorageError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir).map_err(|e| StorageError::Io(e.to_string()))?;
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
fn encode_sealed(s: &SealedBlob) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.nonce.len() + s.ciphertext.len());
    out.extend_from_slice(&s.nonce);
    out.extend_from_slice(&s.ciphertext);
    out
}

/// Parse a sealed blob from disk bytes.
fn decode_sealed(bytes: &[u8]) -> Result<SealedBlob, StorageError> {
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
    fn put(&mut self, bytes: &[u8]) -> Result<Cid, StorageError> {
        let cid = Cid::of(bytes);
        let path = self.path(&cid);
        // A corrupt sealed file still has the expected CID filename. Validate the record before
        // deduplicating so an authenticated peer fetch can replace it with freshly sealed bytes.
        if !matches!(self.get(&cid), Ok(Some(_))) {
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
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

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
