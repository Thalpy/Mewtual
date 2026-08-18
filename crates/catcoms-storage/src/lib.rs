//! Storage & retention (Phase 5).
//!
//! - [`cid`]; content addresses ([`Cid`] = `BLAKE3(bytes)`), computed over the
//!   **ciphertext** that actually travels and is stored, so a blob verifies
//!   end-to-end without being decrypted.
//! - [`blob`]; a [`BlobStore`] trait with integrity-checked in-memory and
//!   filesystem backends.
//! - [`filecrypto`]; per-file content keys with a **per-file wrap nonce** (the
//!   review fix against nonce reuse), and a [`FileRef`] carrying the addresses and
//!   the wrapped content key.
//! - [`retention`]; the adjustable expiry model (global → server → file,
//!   most-specific wins), and a GC engine with **decorrelated eviction** (per-file
//!   jitter so the whole group doesn't drop a blob at once) and a **holder probe**
//!   (never evict the last copy). Eviction drops the bytes but keeps the metadata,
//!   so an expired file stays re-fetchable by CID.
//!
//! The metadata index is an in-memory projection here (it is local and
//! rebuildable from CRDT sync); the SQLCipher-backed persistence is wired up with
//! the platform/keystore layer.

pub mod blob;
pub mod cid;
pub mod filecrypto;
pub mod retention;
pub mod vault;

use thiserror::Error;

pub use blob::{BlobStore, FsBlobStore, MemoryBlobStore, SealingBlobStore};
pub use cid::Cid;
pub use filecrypto::{open_file, seal_file, FileManifest, FileRef};
pub use retention::{
    BlobEntry, BlobKind, BlobState, Expiry, ExpiryPolicy, GcReport, HolderOracle, RetentionIndex,
    ServerId, ONE_MONTH_MS,
};
pub use vault::{open_or_create_vault, vault_exists};

/// Errors from the storage layer.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Stored/received bytes do not match their content address.
    #[error("content id mismatch: data does not match its cid")]
    CidMismatch,
    /// A sealing/opening error (wrong key or tampered ciphertext).
    #[error(transparent)]
    Crypto(#[from] catcoms_crypto::KeystoreError),
    /// A filesystem error.
    #[error("io error: {0}")]
    Io(String),
    /// A blob or file reference was malformed.
    #[error("malformed data")]
    Malformed,
}
