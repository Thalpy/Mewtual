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
//! - [`pad`]; deterministic size quantization (P10): a padded frame that goes **inside** the
//!   AEAD, so a forwarder measuring ciphertext lengths sees a bucket instead of the payload's
//!   exact size. Shared by [`filecrypto`] and by `catcoms-replication`'s sealed ops.
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
pub mod pad;
pub mod retention;
pub mod vault;

use thiserror::Error;

pub use blob::{BlobStore, FsBlobStore, MemoryBlobStore, SealingBlobStore};
pub use cid::{Cid, CidHasher};
pub use filecrypto::{open_file, seal_file, FileManifest, FileRef, MAX_CHUNKS};
// `FileRef::wrapped_key` is a public field of this type, so a caller cannot read or build a
// `FileRef` without being able to name it.
pub use catcoms_crypto::SealedBlob;
pub use pad::{
    pad, padded_len, unpad, CHUNK_PAD_CEILING, CHUNK_PAD_FLOOR, OP_PAD_CEILING, OP_PAD_FLOOR,
    PAD_FOOTER_BYTES,
};
pub use retention::{
    BlobEntry, BlobKind, BlobState, Expiry, ExpiryPolicy, GcReport, HolderOracle, RetentionIndex,
    ServerId, ONE_MONTH_MS,
};
pub use vault::{
    acquire_vault_session, change_vault_passphrase, open_or_create_vault, vault_exists,
    verify_vault_passphrase, VaultSessionGuard, MAX_VAULT_SECRET_BYTES,
};

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
    /// The replacement is already visible, but its directory entry could not be flushed. Retrying
    /// the operation is safe; callers must not claim that the previous state was restored.
    #[error("write committed but could not be made durable: {0}")]
    CommittedButNotDurable(String),
    /// Another process is creating, opening, or rewrapping this vault. Callers may retry instead
    /// of blocking a UI command indefinitely behind a suspended process.
    #[error("the vault is busy in another application process; try again")]
    VaultBusy,
    /// A blob or file reference was malformed.
    #[error("malformed data")]
    Malformed,
    /// A vault secret was empty, unreasonably large, or a replacement matched the old secret.
    #[error(
        "the vault secret must be non-empty, reasonably sized, and different when replacing it"
    )]
    InvalidVaultSecret,
}
