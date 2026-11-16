//! At-rest **key vault** (Phase 9a): a passphrase-sealed root DEK on disk, yielding a
//! [`KeyHierarchy`] (`db_key`/`mls_seal_key`/`blob_key`) for all on-disk sealing.
//!
//! The vault file is `version ‖ salt ‖ nonce ‖ sealed-DEK`. The root DEK is sealed with
//! XChaCha20-Poly1305 under a key Argon2id-derived from the passphrase + a per-vault random
//! salt (the salt is not secret; it lives in the file). The passphrase is never stored, so
//! an attacker with the file still needs it to unseal the DEK. A wrong passphrase fails as
//! an authenticated-decryption error ([`StorageError::Crypto`]); never the wrong key.
//!
//! This is the keystore wiring; the higher layers seal blobs / docs / MLS state under the
//! derived subkeys. The root DEK is currently passphrase-protected only; an OS-keychain
//! tier (`KeyTier::OsSoftware`) so the passphrase isn't needed every launch is future work.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use catcoms_crypto::{Dek, KeyHierarchy, PassphraseKeyStore, SealedBlob, SecureKeyStore};
use catcoms_rt::CryptoRngCore;
use zeroize::Zeroizing;

use crate::StorageError;

const VAULT_VERSION_RAW_SECRET: u8 = 1;
/// Version 2 normalizes a legacy-long user secret to a fixed 32-byte, domain-separated BLAKE3
/// value before Argon2. Short secrets continue to use v1 so existing vault bytes remain stable.
const VAULT_VERSION_PREHASHED_SECRET: u8 = 2;
const LONG_SECRET_DOMAIN: &str = "mewtual-vault-long-secret/v2";
/// Managed chat admission requires native readers that understand its independent fences.
/// A distinct KDF domain binds this version cryptographically: merely changing 3 to 1/2 must
/// not let an old reader decrypt a managed vault while ignoring its retirement inventory.
const VAULT_VERSION_CHAT_SCOPE: u8 = 3;
const CHAT_SCOPE_SECRET_DOMAIN: &str = "mewtual-vault-chat-scope/v3";
/// A scope-only v3 reader does not enforce immutable owner roots/canonical heads. This critical
/// format therefore has its OWN KDF domain, not a feature bit an older reader could ignore.
const VAULT_VERSION_OWNER_EVIDENCE: u8 = 4;
const OWNER_EVIDENCE_SECRET_DOMAIN: &str = "mewtual-vault-chat-owner-evidence/v4";
/// Owner operations must be fenced BEFORE a v4 reader reaches its older stage cleaners. This
/// cryptographic version gate preserves the DEK; it is not an external rollback anchor. Replaying
/// an authentic old wrapper alone can still reveal that same DEK to an old binary.
const VAULT_VERSION_OWNER_OPERATIONS: u8 = 5;
const OWNER_OPERATIONS_SECRET_DOMAIN: &str = "mewtual-vault-chat-owner-evidence/v5";
/// V6 readers must understand the mandatory sealed critical-namespace catalog before touching
/// older staging/cleanup paths. The initial host supports EMPTY catalogs only. A distinct KDF
/// domain fences normal v1-v5 readers, while preserving the DEK and the documented authentic
/// old-wrapper component-rollback limitation.
const VAULT_VERSION_CRITICAL_NAMESPACES: u8 = 6;
const CRITICAL_NAMESPACES_SECRET_DOMAIN: &str = "mewtual-vault-critical-namespaces/v6";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const SEALED_DEK_LEN: usize = 32 + 16; // 32-byte DEK plus Poly1305 authentication tag.
const VAULT_ENCODED_LEN: usize = 1 + SALT_LEN + NONCE_LEN + SEALED_DEK_LEN;
const VAULT_FILE: &str = "vault.bin";
const VAULT_LOCK_FILE: &str = ".vault.lock";
const VAULT_SESSION_LOCK_FILE: &str = ".vault.session.lock";
/// Bound KDF inputs before Argon2 work and before a bridge accepts an attacker-sized allocation.
pub const MAX_VAULT_SECRET_BYTES: usize = 4096;
/// Compatibility ceiling for a v1 vault created before the 4096-byte input bound existed. The
/// first successful open migrates such a wrapper to v2; this cap keeps even that one-time legacy
/// path bounded rather than accepting an attacker-sized IPC allocation forever.
const MAX_LEGACY_VAULT_SECRET_BYTES: usize = 64 * 1024;

/// Closed classification of the SAME wrapper bytes that actually authenticated the root DEK.
/// This is metadata, not a mount/admission capability. A host must not reconstruct it by reading
/// a mutable wrapper header again after opening the keys or accept it from a wire boolean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultFormat {
    /// Original raw or legacy-long normalized passphrase wrapper; no managed chat authority.
    Legacy,
    /// Cryptographically distinct v3 wrapper; an actual mounted Store must require its inventory.
    ChatScope,
    /// Mandatory v4 owner pin/canonical-head inventory, in addition to local admission lineage.
    OwnerEvidence,
    /// Mandatory v5 activation intent/operation-aware namespace, preserving every v3/v4 record.
    OwnerOperations,
    /// Mandatory v6 sealed namespace catalog in addition to all v3/v4/v5 inventories.
    CriticalNamespaces,
}

impl VaultFormat {
    /// Format metadata only: all supported owner formats require the owner-pin/canonical inventory.
    /// This never establishes a live admission, historical membership, Saved head or writer.
    pub fn requires_owner_evidence(self) -> bool {
        matches!(
            self,
            Self::OwnerEvidence | Self::OwnerOperations | Self::CriticalNamespaces
        )
    }

    /// Metadata only. V6 additionally requires actual catalog validation by its mounted host;
    /// this predicate cannot create that validation, an owner writer or publication authority.
    pub fn requires_owner_operations(self) -> bool {
        matches!(self, Self::OwnerOperations | Self::CriticalNamespaces)
    }
}

/// Authenticated keys and their indivisible format classification. Construction is private;
/// the ordinary key-only open API remains a compatibility wrapper. No private key is printed.
pub struct OpenedVault {
    keys: KeyHierarchy,
    format: VaultFormat,
}
impl std::fmt::Debug for OpenedVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenedVault").finish_non_exhaustive()
    }
}
impl OpenedVault {
    /// Consume the authenticated result without rereading mutable wrapper bytes. Neither field
    /// alone authenticates a runtime Store lifetime or replaces its independent admission CAS.
    pub fn into_parts(self) -> (KeyHierarchy, VaultFormat) {
        (self.keys, self.format)
    }
}

/// Open the key vault under `dir`, creating it (a fresh random DEK) on first use, and
/// returning the [`KeyHierarchy`] unsealed with `passphrase`. A wrong passphrase fails with
/// an authentication error rather than silently returning the wrong key, and never
/// overwrites the existing vault. An OS-backed sibling lock serializes the full check/generate/
/// publish transaction across application processes, so concurrent first opens cannot return
/// different DEKs. A contender gets [`StorageError::VaultBusy`] and may retry; it never blocks.
pub fn open_or_create_vault(
    dir: impl AsRef<Path>,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<KeyHierarchy, StorageError> {
    open_or_create_vault_with_hooks(dir, passphrase, rng, || {}, || {})
}

/// Open the actual wrapper once and retain its authenticated format with the keys. Managed
/// readers cross a real root-directory durability barrier before this returns, including when
/// the prior process died after a wrapper/metadata rename. The host must still validate the
/// mandatory managed inventory under its exclusive mount BEFORE returning a usable Store.
pub fn open_or_create_vault_classified(
    dir: impl AsRef<Path>,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<OpenedVault, StorageError> {
    open_classified_with_hooks(
        dir,
        passphrase,
        rng,
        || {},
        || {},
        |_| {},
        sync_managed_directory,
    )
}

fn open_or_create_vault_with_hooks(
    dir: impl AsRef<Path>,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
    lock_acquired: impl FnOnce(),
    candidate_ready: impl FnOnce(),
) -> Result<KeyHierarchy, StorageError> {
    open_classified_with_hooks(
        dir,
        passphrase,
        rng,
        lock_acquired,
        candidate_ready,
        |_| {},
        sync_managed_directory,
    )
    .map(|opened| opened.into_parts().0)
}

fn open_classified_with_hooks(
    dir: impl AsRef<Path>,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
    lock_acquired: impl FnOnce(),
    candidate_ready: impl FnOnce(),
    authenticated: impl FnOnce(&Path),
    sync_parent: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<OpenedVault, StorageError> {
    validate_existing_vault_secret_shape(passphrase)?;
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).map_err(|e| StorageError::Io(e.to_string()))?;
    // This OS lock is held from the existence check through key generation/read and durable
    // publication. It is interprocess (unlike the desktop's mutex) and released automatically if
    // the process exits, so two first-run apps can never both return different in-memory DEKs.
    let _lock = lock_vault(dir)?;
    lock_acquired();
    let path = dir.join(VAULT_FILE);
    let (dek, format) = if path.exists() {
        let bytes = read_vault_wrapper(&path)?;
        let version = vault_version(&bytes)?;
        let dek = decode_and_unseal(&bytes, passphrase)?;
        if version == VAULT_VERSION_RAW_SECRET && passphrase.len() > MAX_VAULT_SECRET_BYTES {
            // Old builds admitted arbitrarily long secrets. Preserve plausible existing users,
            // but immediately replace the wrapper so Argon2 never receives the long input again.
            let migrated =
                seal_and_encode_version(&dek, passphrase, rng, VAULT_VERSION_PREHASHED_SECRET)?;
            let _ = decode_and_unseal(&migrated, passphrase)?;
            write_atomic(&path, &migrated)?;
        }
        let format = match version {
            VAULT_VERSION_RAW_SECRET | VAULT_VERSION_PREHASHED_SECRET => VaultFormat::Legacy,
            VAULT_VERSION_CHAT_SCOPE => VaultFormat::ChatScope,
            VAULT_VERSION_OWNER_EVIDENCE => VaultFormat::OwnerEvidence,
            VAULT_VERSION_OWNER_OPERATIONS => VaultFormat::OwnerOperations,
            VAULT_VERSION_CRITICAL_NAMESPACES => VaultFormat::CriticalNamespaces,
            // New authenticated versions must extend this classification deliberately. Refuse
            // rather than treating a future mandatory format as legacy or panicking on it.
            _ => return Err(StorageError::Malformed),
        };
        (dek, format)
    } else {
        validate_new_vault_secret(passphrase)?;
        let dek = Dek::generate(rng);
        let bytes = seal_and_encode(&dek, passphrase, rng)?;
        candidate_ready();
        write_atomic(&path, &bytes)?;
        (dek, VaultFormat::Legacy)
    };
    // The deterministic test hook may alter the physical header after authentication. Never
    // reread it to infer format: equality of a later public byte is not authenticated metadata.
    authenticated(dir);
    if format != VaultFormat::Legacy {
        sync_parent(dir).map_err(|error| {
            StorageError::Io(format!("managed vault reopen durability barrier: {error}"))
        })?;
    }
    Ok(OpenedVault {
        keys: KeyHierarchy::new(dek),
        format,
    })
}

#[cfg(test)]
#[path = "vault_classification_tests.rs"]
mod classification_tests;
#[cfg(test)]
#[path = "vault_critical_namespace_tests.rs"]
mod critical_namespace_tests;
#[cfg(test)]
#[path = "vault_owner_evidence_tests.rs"]
mod owner_evidence_tests;
#[cfg(test)]
#[path = "vault_owner_operations_tests.rs"]
mod owner_operations_tests;

/// Does a vault already exist under `dir`? The gate before [`open_or_create_vault`] is asked to
/// open one: that call *creates* on first use, so a UI with no way to tell the two cases apart
/// turns a mistyped passphrase on a fresh install into a brand-new identity, silently. Says
/// nothing about whether any given passphrase opens it.
pub fn vault_exists(dir: impl AsRef<Path>) -> bool {
    dir.as_ref().join(VAULT_FILE).exists()
}

/// Unauthenticated header inspection for diagnostics/format tests only. A later header read
/// MUST NOT classify an already-opened Store: use [`open_or_create_vault_classified`] so the
/// format comes from the same bytes that authenticated its keys. This returns no live permit.
pub fn chat_scope_vault_managed(dir: impl AsRef<Path>) -> Result<bool, StorageError> {
    Ok(matches!(
        vault_version(&read_vault_wrapper(&dir.as_ref().join(VAULT_FILE))?)?,
        VAULT_VERSION_CHAT_SCOPE
            | VAULT_VERSION_OWNER_EVIDENCE
            | VAULT_VERSION_OWNER_OPERATIONS
            | VAULT_VERSION_CRITICAL_NAMESPACES
    ))
}

/// Commit mandatory managed-chat format after the host has durably initialized its sealed
/// admission inventory. The host must retain the exclusive vault session throughout activation.
/// An earlier staged inventory is inert until this succeeds; errors, including post-rename
/// uncertainty, must not return a usable managed store or expose an authoring context.
///
/// The root DEK and every derived data key are preserved. Existing managed wrappers are rewritten
/// too: retry must cross a real directory-sync barrier rather than acknowledge possibly renamed
/// bytes from a prior failed activation. This is not a reader-downgrade or data migration API.
pub fn activate_chat_scope_vault(
    dir: impl AsRef<Path>,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<(), StorageError> {
    activate_chat_scope_vault_with_hooks(
        dir.as_ref(),
        passphrase,
        rng,
        |_, _| {},
        sync_managed_directory,
    )
}

fn activate_chat_scope_vault_with_hooks(
    dir: &Path,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
    phase: impl FnMut(AtomicWritePhase, &Path),
    sync_parent: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<(), StorageError> {
    activate_managed_vault_with_hooks(
        dir,
        passphrase,
        rng,
        VAULT_VERSION_CHAT_SCOPE,
        phase,
        sync_parent,
    )
}

/// Activate critical owner-evidence storage ONLY after the actual mounted host has durably
/// initialized its immutable root inventory while preserving every existing scope row. This
/// low-level key-wrapper operation creates no owner pin, admission, writer or publication grant.
/// On failure the host must not expose an owner-managed Store. Exact retries cross a real sync.
pub fn activate_owner_evidence_vault(
    dir: impl AsRef<Path>,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<(), StorageError> {
    activate_managed_vault_with_hooks(
        dir.as_ref(),
        passphrase,
        rng,
        VAULT_VERSION_OWNER_EVIDENCE,
        |_, _| {},
        sync_managed_directory,
    )
}

/// Commit the critical operation-aware wrapper after the host has durably preserved its v3/v4
/// inventories and activation intent. This low-level primitive creates none of those records and
/// returns no Store/owner capability. Exact retries cross the real directory barrier. The host
/// must drop its pre-exposure mount on any error, including a possibly visible replacement.
pub fn activate_owner_operations_vault(
    dir: impl AsRef<Path>,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<(), StorageError> {
    activate_managed_vault_with_hooks(
        dir.as_ref(),
        passphrase,
        rng,
        VAULT_VERSION_OWNER_OPERATIONS,
        |_, _| {},
        sync_managed_directory,
    )
}

/// Commit v6 only AFTER the actual private mounted host durably validates its supported sealed
/// catalog and mandatory older inventories. This low-level wrapper primitive does not create or
/// validate any catalog. Failure must drop the host's private mount, including after rename.
pub fn activate_critical_namespace_vault(
    dir: impl AsRef<Path>,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<(), StorageError> {
    activate_managed_vault_with_hooks(
        dir.as_ref(),
        passphrase,
        rng,
        VAULT_VERSION_CRITICAL_NAMESPACES,
        |_, _| {},
        sync_managed_directory,
    )
}

fn activate_managed_vault_with_hooks(
    dir: &Path,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
    requested_version: u8,
    phase: impl FnMut(AtomicWritePhase, &Path),
    sync_parent: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<(), StorageError> {
    validate_existing_vault_secret_shape(passphrase)?;
    let _lock = lock_vault(dir)?;
    let path = dir.join(VAULT_FILE);
    let old = read_vault_wrapper(&path)?;
    let old_version = vault_version(&old)?;
    let dek = decode_and_unseal(&old, passphrase)?;
    let version = match (requested_version, old_version) {
        (VAULT_VERSION_CRITICAL_NAMESPACES, _)
        | (
            VAULT_VERSION_CHAT_SCOPE
            | VAULT_VERSION_OWNER_EVIDENCE
            | VAULT_VERSION_OWNER_OPERATIONS,
            VAULT_VERSION_CRITICAL_NAMESPACES,
        ) => VAULT_VERSION_CRITICAL_NAMESPACES,
        // All known critical versions are sticky. Older setup/secret callers cannot silently
        // turn off operation semantics after a v5 upgrade. Unknown requests still refuse.
        (VAULT_VERSION_OWNER_OPERATIONS, _)
        | (
            VAULT_VERSION_CHAT_SCOPE | VAULT_VERSION_OWNER_EVIDENCE,
            VAULT_VERSION_OWNER_OPERATIONS,
        ) => VAULT_VERSION_OWNER_OPERATIONS,
        (VAULT_VERSION_CHAT_SCOPE, VAULT_VERSION_OWNER_EVIDENCE)
        | (VAULT_VERSION_OWNER_EVIDENCE, _) => VAULT_VERSION_OWNER_EVIDENCE,
        (VAULT_VERSION_CHAT_SCOPE, _) => VAULT_VERSION_CHAT_SCOPE,
        _ => return Err(StorageError::Malformed),
    };
    let replacement = seal_and_encode_version(&dek, passphrase, rng, version)?;
    let _ = decode_and_unseal(&replacement, passphrase)?;
    write_atomic_with_hook_and_sync(&path, &replacement, phase, sync_parent)
}

/// Exclusive installation session held by the loaded Store. The SAME operating-system lock
/// can additionally be retained by admitted physical work, so Store destruction cannot permit
/// a replacement mount while an old blocking write is still draining. This is not a key handle.
pub struct VaultSessionGuard {
    lock: Arc<File>,
}
impl std::fmt::Debug for VaultSessionGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultSessionGuard").finish_non_exhaustive()
    }
}
impl VaultSessionGuard {
    /// Fresh-only negative restriction under this already-held lifetime lock.
    /// Inspect at most two entries and allow only this exact lock object. A
    /// missing wrapper must never cause new keys beside old encrypted data.
    /// This performs no KDF, creates no vault and grants no mounted authority.
    pub fn require_fresh_directory(&self, dir: impl AsRef<Path>) -> Result<(), StorageError> {
        let mut entries =
            std::fs::read_dir(dir.as_ref()).map_err(|e| StorageError::Io(e.to_string()))?;
        let entry = entries
            .next()
            .ok_or(StorageError::Malformed)?
            .map_err(|e| StorageError::Io(e.to_string()))?;
        if entry.file_name() != OsStr::new(VAULT_SESSION_LOCK_FILE)
            || !entry
                .file_type()
                .map_err(|e| StorageError::Io(e.to_string()))?
                .is_file()
            || entries.next().is_some()
        {
            return Err(StorageError::Malformed);
        }
        let actual = File::open(entry.path()).map_err(|e| StorageError::Io(e.to_string()))?;
        if crate::kept::session_id(&actual).map_err(|e| StorageError::Io(e.to_string()))?
            != crate::kept::session_id(&self.lock).map_err(|e| StorageError::Io(e.to_string()))?
        {
            return Err(StorageError::Malformed);
        }
        Ok(())
    }

    /// Retain ONLY the original OS lock for one running physical job. This neither keeps the
    /// Store/keys/creator alive nor grants admission, Current or permission to begin new work.
    /// The concrete host must separately prepay/admit the job and hold this loan until every
    /// write, sync, result-file handle and potentially blocking cleanup has drained.
    ///
    /// Never store the loan in inert debt, an observer or an abandoned caller/waiter. It has
    /// no Clone, path/File access, unlock or authority API. No second file is opened/duplicated:
    /// the original file closes only after the last Store guard or running-work loan drops.
    pub fn retain_for_physical_work(&self) -> VaultPhysicalWorkLease {
        VaultPhysicalWorkLease {
            _lock: self.lock.clone(),
        }
    }
}

/// OS exclusion only, not a mounted Store, live scope, key or source/persistence capability.
/// Move into admitted RUNNING work, with its files/payloads declared before this field so
/// their I/O and cleanup drain FIRST. Dropping the original Store must still revoke all its
/// publication authority immediately; this resource only blocks a competing physical session.
#[must_use = "physical work must retain its session lease through all I/O and file cleanup"]
pub struct VaultPhysicalWorkLease {
    _lock: Arc<File>,
}
impl std::fmt::Debug for VaultPhysicalWorkLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultPhysicalWorkLease")
            .finish_non_exhaustive()
    }
}

/// Refuse a second application process from mounting and mutating the same vault concurrently.
/// Byte-atomic files are not enough for MLS/invite-ledger state: two actors starting from one
/// snapshot could each publish a valid but logically divergent successor. This lock is nonblocking
/// so a suspended owner cannot hang the contender's unlock UI.
pub fn acquire_vault_session(dir: impl AsRef<Path>) -> Result<VaultSessionGuard, StorageError> {
    std::fs::create_dir_all(dir.as_ref()).map_err(|e| StorageError::Io(e.to_string()))?;
    let lock = try_lock_file(&dir.as_ref().join(VAULT_SESSION_LOCK_FILE))?;
    Ok(VaultSessionGuard {
        lock: Arc::new(lock),
    })
}

#[cfg(test)]
#[path = "vault_physical_work_tests.rs"]
mod physical_work_tests;

/// Authenticate `passphrase` against an existing vault without acquiring another lifetime
/// session lock or returning its keys. The desktop uses this when an explicitly locked webview
/// unlocks again while the native actors and [`VaultSessionGuard`] intentionally remain mounted.
///
/// The short transaction lock keeps verification ordered with a passphrase rewrap. An absent
/// vault is an error rather than a creation opportunity: callers on this path already own the
/// mounted installation and must never create a second identity by accident.
pub fn verify_vault_passphrase(
    dir: impl AsRef<Path>,
    passphrase: &[u8],
) -> Result<(), StorageError> {
    validate_existing_vault_secret_shape(passphrase)?;
    let dir = dir.as_ref();
    let _lock = lock_vault(dir)?;
    let bytes = read_vault_wrapper(&dir.join(VAULT_FILE))?;
    let _ = decode_and_unseal(&bytes, passphrase)?;
    Ok(())
}

/// Authenticate the SAME mounted critical vault under one wrapper transaction.
/// Unlike path-only verification, a valid wrapper copied from another vault
/// cannot authorize access to retained keys. Same-DEK passphrase rewrap remains
/// valid. This checks format/key identity only, never catalog or UI authority.
pub fn verify_critical_vault_passphrase_for_keys(
    dir: impl AsRef<Path>,
    passphrase: &[u8],
    mounted: &KeyHierarchy,
) -> Result<(), StorageError> {
    validate_existing_vault_secret_shape(passphrase)?;
    let dir = dir.as_ref();
    let _lock = lock_vault(dir)?;
    let bytes = read_vault_wrapper(&dir.join(VAULT_FILE))?;
    if vault_version(&bytes)? != VAULT_VERSION_CRITICAL_NAMESPACES {
        return Err(StorageError::Malformed);
    }
    let candidate = KeyHierarchy::new(decode_and_unseal(&bytes, passphrase)?);
    let expected = Zeroizing::new(mounted.db_key()?);
    let actual = Zeroizing::new(candidate.db_key()?);
    // Hash's equality is constant-time in the pinned BLAKE3 implementation.
    // Compare fixed-size key commitments; temporary raw subkeys zeroize.
    if blake3::hash(expected.as_ref()) != blake3::hash(actual.as_ref()) {
        return Err(StorageError::Malformed);
    }
    Ok(())
}

/// Rewrap the existing root DEK under `new_passphrase`, preserving every derived data key.
///
/// This is intentionally a small atomic rewrite of `vault.bin`, not a decrypt/re-encrypt pass over
/// server snapshots and blobs: the passphrase protects the random DEK, while those records are
/// protected by keys derived from that DEK. The current passphrase is authenticated before any
/// write, a fresh salt and nonce are generated, and a failure leaves the previous vault in place.
/// The same non-blocking interprocess vault lock prevents concurrent rewraps from both reporting
/// success; a contender receives [`StorageError::VaultBusy`].
pub fn change_vault_passphrase(
    dir: impl AsRef<Path>,
    current_passphrase: &[u8],
    new_passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<(), StorageError> {
    change_vault_passphrase_with_hooks(dir, current_passphrase, new_passphrase, rng, || {}, || {})
}

fn change_vault_passphrase_with_hooks(
    dir: impl AsRef<Path>,
    current_passphrase: &[u8],
    new_passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
    lock_acquired: impl FnOnce(),
    candidate_ready: impl FnOnce(),
) -> Result<(), StorageError> {
    validate_existing_vault_secret_shape(current_passphrase)?;
    validate_new_vault_secret(new_passphrase)?;
    if current_passphrase == new_passphrase {
        return Err(StorageError::InvalidVaultSecret);
    }
    let dir = dir.as_ref();
    let _lock = lock_vault(dir)?;
    lock_acquired();
    let path = dir.join(VAULT_FILE);
    let old = read_vault_wrapper(&path)?;
    let dek = decode_and_unseal(&old, current_passphrase)?;
    // A secret change must never silently re-enable old binaries that ignore admission fences.
    let version = vault_version(&old)?;
    let replacement = match version {
        VAULT_VERSION_CHAT_SCOPE
        | VAULT_VERSION_OWNER_EVIDENCE
        | VAULT_VERSION_OWNER_OPERATIONS
        | VAULT_VERSION_CRITICAL_NAMESPACES => {
            seal_and_encode_version(&dek, new_passphrase, rng, version)?
        }
        _ => seal_and_encode(&dek, new_passphrase, rng)?,
    };
    // Authenticate the generated wrapper before it can replace the user's only live vault file.
    let _ = decode_and_unseal(&replacement, new_passphrase)?;
    candidate_ready();
    write_atomic(&path, &replacement)
}

/// Bound and reject degenerate KDF inputs before acquiring a filesystem lock or entering Argon2.
/// This is shared by every authentication route so a bridge cannot accidentally expose an
/// unbounded variant when it adds a new unlock flow.
fn validate_new_vault_secret(secret: &[u8]) -> Result<(), StorageError> {
    if secret.is_empty() || secret.len() > MAX_VAULT_SECRET_BYTES {
        return Err(StorageError::InvalidVaultSecret);
    }
    Ok(())
}

fn validate_existing_vault_secret_shape(secret: &[u8]) -> Result<(), StorageError> {
    if secret.is_empty() || secret.len() > MAX_LEGACY_VAULT_SECRET_BYTES {
        return Err(StorageError::InvalidVaultSecret);
    }
    Ok(())
}

/// Seal a DEK under a fresh-salt passphrase store and encode the vault file bytes.
fn seal_and_encode(
    dek: &Dek,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<Vec<u8>, StorageError> {
    seal_and_encode_version(dek, passphrase, rng, VAULT_VERSION_RAW_SECRET)
}

fn seal_and_encode_version(
    dek: &Dek,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
    version: u8,
) -> Result<Vec<u8>, StorageError> {
    let mut salt = [0u8; SALT_LEN];
    rng.fill_bytes(&mut salt);
    let ks = derive_passphrase_store(version, passphrase, &salt)?;
    let sealed = ks.seal_dek(dek, rng)?;
    let mut out = Vec::with_capacity(1 + SALT_LEN + NONCE_LEN + sealed.ciphertext.len());
    out.push(version);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&sealed.nonce);
    out.extend_from_slice(&sealed.ciphertext);
    Ok(out)
}

/// Decode the vault file bytes and unseal the DEK with `passphrase`.
fn decode_and_unseal(bytes: &[u8], passphrase: &[u8]) -> Result<Dek, StorageError> {
    let header = 1 + SALT_LEN + NONCE_LEN;
    if bytes.len() <= header {
        return Err(StorageError::Malformed);
    }
    let version = vault_version(bytes)?;
    let salt = &bytes[1..1 + SALT_LEN];
    let nonce: [u8; NONCE_LEN] = bytes[1 + SALT_LEN..header]
        .try_into()
        .expect("slice length checked above");
    let ciphertext = bytes[header..].to_vec();
    let ks = derive_passphrase_store(version, passphrase, salt)?;
    Ok(ks.unseal_dek(&SealedBlob { nonce, ciphertext })?)
}

/// Read the fixed-size wrapper without ever allocating according to attacker-controlled file
/// length. The vault format has no variable fields: accepting any other length is corruption, not
/// forward compatibility (a future format must introduce a new exact version/shape deliberately).
fn read_vault_wrapper(path: &Path) -> Result<[u8; VAULT_ENCODED_LEN], StorageError> {
    let mut file = File::open(path).map_err(|e| StorageError::Io(e.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|e| StorageError::Io(e.to_string()))?;
    if metadata.len() != VAULT_ENCODED_LEN as u64 {
        return Err(StorageError::Malformed);
    }
    let mut bytes = [0u8; VAULT_ENCODED_LEN];
    file.read_exact(&mut bytes)
        .map_err(|_| StorageError::Malformed)?;
    Ok(bytes)
}

fn vault_version(bytes: &[u8]) -> Result<u8, StorageError> {
    match bytes.first().copied() {
        Some(
            VAULT_VERSION_RAW_SECRET
            | VAULT_VERSION_PREHASHED_SECRET
            | VAULT_VERSION_CHAT_SCOPE
            | VAULT_VERSION_OWNER_EVIDENCE
            | VAULT_VERSION_OWNER_OPERATIONS
            | VAULT_VERSION_CRITICAL_NAMESPACES,
        ) => Ok(bytes[0]),
        _ => Err(StorageError::Malformed),
    }
}

fn derive_passphrase_store(
    version: u8,
    passphrase: &[u8],
    salt: &[u8],
) -> Result<PassphraseKeyStore, StorageError> {
    match version {
        VAULT_VERSION_RAW_SECRET => Ok(PassphraseKeyStore::derive(passphrase, salt)?),
        VAULT_VERSION_PREHASHED_SECRET => {
            let mut hasher = blake3::Hasher::new_derive_key(LONG_SECRET_DOMAIN);
            hasher.update(&(passphrase.len() as u64).to_be_bytes());
            hasher.update(passphrase);
            let normalized = Zeroizing::new(*hasher.finalize().as_bytes());
            Ok(PassphraseKeyStore::derive(normalized.as_slice(), salt)?)
        }
        VAULT_VERSION_CHAT_SCOPE
        | VAULT_VERSION_OWNER_EVIDENCE
        | VAULT_VERSION_OWNER_OPERATIONS
        | VAULT_VERSION_CRITICAL_NAMESPACES => {
            let domain = match version {
                VAULT_VERSION_CRITICAL_NAMESPACES => CRITICAL_NAMESPACES_SECRET_DOMAIN,
                VAULT_VERSION_OWNER_OPERATIONS => OWNER_OPERATIONS_SECRET_DOMAIN,
                VAULT_VERSION_OWNER_EVIDENCE => OWNER_EVIDENCE_SECRET_DOMAIN,
                _ => CHAT_SCOPE_SECRET_DOMAIN,
            };
            let mut hasher = blake3::Hasher::new_derive_key(domain);
            hasher.update(&(passphrase.len() as u64).to_be_bytes());
            hasher.update(passphrase);
            let normalized = Zeroizing::new(*hasher.finalize().as_bytes());
            Ok(PassphraseKeyStore::derive(normalized.as_slice(), salt)?)
        }
        _ => Err(StorageError::Malformed),
    }
}

fn lock_vault(dir: &Path) -> Result<File, StorageError> {
    try_lock_file(&dir.join(VAULT_LOCK_FILE))
}

pub(crate) fn try_lock_file(path: &Path) -> Result<File, StorageError> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| StorageError::Io(error.to_string()))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Err(StorageError::VaultBusy),
        Err(std::fs::TryLockError::Error(error)) => {
            return Err(StorageError::Io(error.to_string()));
        }
    }
    Ok(lock)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AtomicWritePhase {
    TempSynced,
    Renamed,
}

static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(0);
const MAX_STAGING_ATTEMPTS: usize = 1_024;

struct StagingPath {
    path: PathBuf,
    remove_on_drop: bool,
}

impl Drop for StagingPath {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn staging_candidate(path: &Path, id: u64) -> PathBuf {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_else(|| OsStr::new("record")));
    name.push(format!(".mewtual-stage-{}-{id}.tmp", std::process::id()));
    parent.join(name)
}

fn open_staging_candidate(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn create_staging_file(path: &Path) -> Result<(File, StagingPath), StorageError> {
    for _ in 0..MAX_STAGING_ATTEMPTS {
        let id = NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed);
        let candidate = staging_candidate(path, id);
        match open_staging_candidate(&candidate) {
            Ok(file) => {
                return Ok((
                    file,
                    StagingPath {
                        path: candidate,
                        remove_on_drop: true,
                    },
                ));
            }
            // `create_new` refuses both a regular collision and a pre-planted symlink.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(StorageError::Io(error.to_string())),
        }
    }
    Err(StorageError::Io(
        "could not create a collision-free vault staging file".into(),
    ))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path).and_then(|directory| directory.sync_all())
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    // Match the application's real Windows server-record barrier. Backup semantics permits a
    // directory handle, write access permits FlushFileBuffers, and full sharing avoids turning
    // this short barrier into a rename exclusion. These are the Win32 CreateFile flag values,
    // also used by this crate's existing kept/external Windows directory-handle code.
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_SHARE_READ_WRITE_DELETE: u32 = 0x0000_0007;
    OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ_WRITE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .and_then(|directory| directory.sync_all())
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(any(unix, windows))]
fn sync_managed_directory(path: &Path) -> std::io::Result<()> {
    sync_directory(path)
}

#[cfg(not(any(unix, windows)))]
fn sync_managed_directory(_path: &Path) -> std::io::Result<()> {
    // Legacy formats retain their earlier atomic-only policy on unsupported targets, but a
    // mandatory anti-downgrade format cannot claim activation without its durability barrier.
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "managed chat vault activation requires a real directory-sync implementation",
    ))
}

/// Write `bytes` through a unique create-new sibling, then durably publish it. Unique staging
/// prevents concurrent writers from mutating one another's open inode; the vault lock additionally
/// serializes the logical first-create/rewrap transaction.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    write_atomic_with_hook_and_sync(path, bytes, |_, _| {}, sync_directory)
}

fn write_atomic_with_hook_and_sync(
    path: &Path,
    bytes: &[u8],
    mut phase: impl FnMut(AtomicWritePhase, &Path),
    sync_parent: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<(), StorageError> {
    let (mut staged, mut staging) = create_staging_file(path)?;
    staged
        .write_all(bytes)
        .map_err(|error| StorageError::Io(error.to_string()))?;
    staged
        .sync_all()
        .map_err(|error| StorageError::Io(error.to_string()))?;
    drop(staged);
    phase(AtomicWritePhase::TempSynced, &staging.path);
    std::fs::rename(&staging.path, path).map_err(|error| StorageError::Io(error.to_string()))?;
    staging.remove_on_drop = false;
    phase(AtomicWritePhase::Renamed, &staging.path);
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        sync_parent(parent)
            .map_err(|error| StorageError::CommittedButNotDurable(error.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    use std::process::{Child, Command, Stdio};

    const CHILD_ROLE: &str = "MEWTUAL_VAULT_TEST_ROLE";
    const CHILD_DIR: &str = "MEWTUAL_VAULT_TEST_DIR";
    const CHILD_POLL: std::time::Duration = std::time::Duration::from_millis(20);
    const CHILD_POLL_ATTEMPTS: usize = 3_000;

    fn marker(dir: &Path, name: &str) -> PathBuf {
        dir.join(name)
    }

    fn wait_for(path: &Path) {
        for _ in 0..CHILD_POLL_ATTEMPTS {
            if path.is_file() {
                return;
            }
            std::thread::sleep(CHILD_POLL);
        }
        panic!("timed out waiting for {}", path.display());
    }

    fn appears_within(path: &Path, attempts: usize) -> bool {
        for _ in 0..attempts {
            if path.is_file() {
                return true;
            }
            std::thread::sleep(CHILD_POLL);
        }
        false
    }

    struct ChildGuard(Child);

    impl ChildGuard {
        fn wait_success(&mut self) {
            for _ in 0..CHILD_POLL_ATTEMPTS {
                match self.0.try_wait().expect("poll vault test child") {
                    Some(status) if status.success() => return,
                    Some(status) => panic!("vault test child exited {status}"),
                    None => std::thread::sleep(CHILD_POLL),
                }
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
            panic!("vault test child timed out");
        }
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    fn spawn_raw_child(role: &str, dir: &Path) -> Child {
        Command::new(std::env::current_exe().expect("current storage test binary"))
            .args([
                "--exact",
                "vault::tests::vault_lock_child",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_ROLE, role)
            .env(CHILD_DIR, dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn vault test child")
    }

    fn spawn_child(role: &str, dir: &Path) -> ChildGuard {
        ChildGuard(spawn_raw_child(role, dir))
    }

    #[cfg(target_os = "linux")]
    fn wait_child_status(child: &mut Child) -> std::process::ExitStatus {
        for _ in 0..CHILD_POLL_ATTEMPTS {
            if let Some(status) = child.try_wait().expect("poll vault test child") {
                return status;
            }
            std::thread::sleep(CHILD_POLL);
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("vault test child timed out");
    }

    fn record_result(path: &Path, result: Result<String, StorageError>) {
        let text = match result {
            Ok(value) => format!("ok:{value}"),
            Err(error) => format!("err:{error}"),
        };
        std::fs::write(path, text).unwrap();
    }

    fn child_lock_hook(dir: &Path, role: &str) {
        std::fs::write(marker(dir, &format!("{role}-lock-acquired")), b"ok").unwrap();
    }

    fn child_candidate_hook(dir: &Path, role: &str) {
        std::fs::write(marker(dir, &format!("{role}-candidate")), b"ok").unwrap();
        if role.ends_with('a') {
            wait_for(&marker(dir, "release-a"));
        }
    }

    /// Run A until it has generated a candidate while holding the lock, then start B. If the lock
    /// is removed, B reports lock acquisition and completes before A is released, deterministically
    /// recreating the last-writer-wins bug. With the lock, B cannot enter the transaction.
    fn run_serialization_race(dir: &Path, a: &str, b: &str) -> (String, String, bool) {
        let mut first = spawn_child(a, dir);
        wait_for(&marker(dir, &format!("{a}-candidate")));
        let mut second = spawn_child(b, dir);
        wait_for(&marker(dir, &format!("{b}-started")));
        let second_finished_while_first_held =
            appears_within(&marker(dir, &format!("{b}-result")), 50);
        std::fs::write(marker(dir, "release-a"), b"ok").unwrap();
        first.wait_success();
        second.wait_success();
        let first = std::fs::read_to_string(marker(dir, &format!("{a}-result"))).unwrap();
        let second = std::fs::read_to_string(marker(dir, &format!("{b}-result"))).unwrap();
        (first, second, second_finished_while_first_held)
    }

    #[test]
    fn concurrent_first_create_returns_only_the_single_published_dek() {
        let dir = tempfile::tempdir().unwrap();
        let (first, second, returned_promptly) =
            run_serialization_race(dir.path(), "create-a", "create-b");
        assert!(first.starts_with("ok:"), "{first}");
        assert!(returned_promptly, "a busy second process must not block");
        assert!(second.contains("vault is busy"), "{second}");

        let expected = first.strip_prefix("ok:").unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(303);
        let reopened = open_or_create_vault(dir.path(), b"shared secret", &mut rng).unwrap();
        assert_eq!(hex::encode(reopened.db_key().unwrap()), expected);
    }

    #[test]
    fn concurrent_rewrap_allows_only_the_process_that_authenticated_current_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(404);
        let original = open_or_create_vault(dir.path(), b"old secret", &mut rng)
            .unwrap()
            .db_key()
            .unwrap();

        let (first, second, returned_promptly) =
            run_serialization_race(dir.path(), "change-a", "change-b");
        assert_eq!(first, "ok:changed");
        assert!(returned_promptly, "a busy second rewrap must not block");
        assert!(second.contains("vault is busy"), "{second}");
        assert!(open_or_create_vault(dir.path(), b"old secret", &mut rng).is_err());
        assert!(open_or_create_vault(dir.path(), b"new-b", &mut rng).is_err());
        let reopened = open_or_create_vault(dir.path(), b"new-a", &mut rng).unwrap();
        assert_eq!(reopened.db_key().unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn a_preplanted_vault_staging_symlink_is_never_followed() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(VAULT_FILE);
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"must stay intact").unwrap();
        let planted = staging_candidate(&path, u64::MAX);
        symlink(&victim, &planted).unwrap();

        let error = open_staging_candidate(&planted).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        write_atomic(&path, b"complete wrapper").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"must stay intact");
        assert_eq!(std::fs::read(path).unwrap(), b"complete wrapper");
    }

    #[test]
    fn a_post_rename_sync_failure_is_classified_as_committed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(VAULT_FILE);
        let result = write_atomic_with_hook_and_sync(
            &path,
            b"complete wrapper",
            |_, _| {},
            |_| Err(std::io::Error::other("injected directory sync failure")),
        );
        assert!(matches!(
            result,
            Err(StorageError::CommittedButNotDurable(message))
                if message == "injected directory sync failure"
        ));
        assert_eq!(std::fs::read(path).unwrap(), b"complete wrapper");
    }

    #[cfg(target_os = "linux")]
    fn staging_files_for(path: &Path) -> Vec<PathBuf> {
        let parent = path.parent().unwrap();
        let prefix = format!(
            ".{}.mewtual-stage-",
            path.file_name().unwrap().to_string_lossy()
        );
        std::fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|candidate| {
                let name = candidate.file_name().unwrap().to_string_lossy();
                name.starts_with(&prefix) && name.ends_with(".tmp")
            })
            .collect()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn aborting_a_vault_write_never_publishes_a_partial_wrapper() {
        use std::os::unix::process::ExitStatusExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(VAULT_FILE);
        write_atomic(&path, b"old complete wrapper").unwrap();

        let mut before = spawn_raw_child("abort-temp", dir.path());
        assert_eq!(wait_child_status(&mut before).signal(), Some(6));
        assert_eq!(std::fs::read(&path).unwrap(), b"old complete wrapper");
        let staged = staging_files_for(&path);
        assert_eq!(staged.len(), 1);
        assert_eq!(std::fs::read(&staged[0]).unwrap(), b"new complete wrapper");
        std::fs::remove_file(&staged[0]).unwrap();

        let mut after = spawn_raw_child("abort-renamed", dir.path());
        assert_eq!(wait_child_status(&mut after).signal(), Some(6));
        assert_eq!(std::fs::read(&path).unwrap(), b"new complete wrapper");
        assert!(staging_files_for(&path).is_empty());
    }

    #[test]
    #[ignore = "spawned by the cross-process vault serialization tests"]
    fn vault_lock_child() {
        let role = std::env::var(CHILD_ROLE).expect("vault child role");
        let dir = PathBuf::from(std::env::var_os(CHILD_DIR).expect("vault child directory"));
        std::fs::write(marker(&dir, &format!("{role}-started")), b"ok").unwrap();
        let seed = if role.ends_with('a') { 501 } else { 502 };
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let result = match role.as_str() {
            "create-a" | "create-b" => open_or_create_vault_with_hooks(
                &dir,
                b"shared secret",
                &mut rng,
                || child_lock_hook(&dir, &role),
                || child_candidate_hook(&dir, &role),
            )
            .map(|keys| hex::encode(keys.db_key().unwrap())),
            "change-a" => change_vault_passphrase_with_hooks(
                &dir,
                b"old secret",
                b"new-a",
                &mut rng,
                || child_lock_hook(&dir, &role),
                || child_candidate_hook(&dir, &role),
            )
            .map(|()| "changed".into()),
            "change-b" => change_vault_passphrase_with_hooks(
                &dir,
                b"old secret",
                b"new-b",
                &mut rng,
                || child_lock_hook(&dir, &role),
                || child_candidate_hook(&dir, &role),
            )
            .map(|()| "changed".into()),
            "abort-temp" | "abort-renamed" => {
                let wanted = if role == "abort-temp" {
                    AtomicWritePhase::TempSynced
                } else {
                    AtomicWritePhase::Renamed
                };
                write_atomic_with_hook_and_sync(
                    &dir.join(VAULT_FILE),
                    b"new complete wrapper",
                    |phase, _| {
                        if phase == wanted {
                            std::process::abort();
                        }
                    },
                    sync_directory,
                )
                .map(|()| "abort failpoint was not reached".into())
            }
            _ => panic!("unknown vault child role {role}"),
        };
        record_result(&marker(&dir, &format!("{role}-result")), result);
    }

    #[test]
    fn fresh_directory_is_bound_to_the_actual_held_session_lock() {
        let first = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let first_guard = acquire_vault_session(first.path()).unwrap();
        let other_guard = acquire_vault_session(other.path()).unwrap();
        first_guard.require_fresh_directory(first.path()).unwrap();
        other_guard.require_fresh_directory(other.path()).unwrap();
        assert!(first_guard.require_fresh_directory(other.path()).is_err());
        assert!(other_guard.require_fresh_directory(first.path()).is_err());
        std::fs::write(first.path().join("unknown"), b"kept").unwrap();
        assert!(first_guard.require_fresh_directory(first.path()).is_err());
        assert_eq!(
            std::fs::read(first.path().join("unknown")).unwrap(),
            b"kept"
        );
    }

    #[test]
    fn critical_mounted_verification_binds_keys_and_format_but_allows_same_dek_rewrap() {
        let first = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xca31);
        let keys = open_or_create_vault(first.path(), b"original", &mut rng).unwrap();
        let legacy = std::fs::read(first.path().join(VAULT_FILE)).unwrap();
        activate_critical_namespace_vault(first.path(), b"original", &mut rng).unwrap();
        verify_critical_vault_passphrase_for_keys(first.path(), b"original", &keys).unwrap();
        assert!(verify_critical_vault_passphrase_for_keys(first.path(), b"wrong", &keys).is_err());
        let original = std::fs::read(first.path().join(VAULT_FILE)).unwrap();
        let other_keys = open_or_create_vault(other.path(), b"foreign", &mut rng).unwrap();
        activate_critical_namespace_vault(other.path(), b"foreign", &mut rng).unwrap();
        std::fs::copy(other.path().join(VAULT_FILE), first.path().join(VAULT_FILE)).unwrap();
        // The old path-only check succeeds: the new boundary must prove more.
        verify_vault_passphrase(first.path(), b"foreign").unwrap();
        assert!(
            verify_critical_vault_passphrase_for_keys(first.path(), b"foreign", &keys).is_err()
        );
        verify_critical_vault_passphrase_for_keys(first.path(), b"foreign", &other_keys).unwrap();
        std::fs::write(first.path().join(VAULT_FILE), legacy).unwrap();
        assert!(
            verify_critical_vault_passphrase_for_keys(first.path(), b"original", &keys).is_err()
        );
        std::fs::write(first.path().join(VAULT_FILE), original).unwrap();
        change_vault_passphrase(first.path(), b"original", b"replacement", &mut rng).unwrap();
        verify_critical_vault_passphrase_for_keys(first.path(), b"replacement", &keys).unwrap();
        assert!(
            verify_critical_vault_passphrase_for_keys(first.path(), b"original", &keys).is_err()
        );
    }

    #[test]
    fn vault_round_trips_and_rejects_a_wrong_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(1);

        // First open creates the vault.
        let kh1 = open_or_create_vault(dir.path(), b"correct horse battery", &mut rng).unwrap();
        let db1 = kh1.db_key().unwrap();

        // Re-opening with the right passphrase yields the SAME derived subkeys.
        let kh2 = open_or_create_vault(dir.path(), b"correct horse battery", &mut rng).unwrap();
        assert_eq!(db1, kh2.db_key().unwrap());
        assert_eq!(kh1.blob_key().unwrap(), kh2.blob_key().unwrap());
        assert_eq!(kh1.mls_seal_key().unwrap(), kh2.mls_seal_key().unwrap());

        // A wrong passphrase fails (authenticated decryption); not the wrong key; and
        // leaves the vault intact.
        assert!(open_or_create_vault(dir.path(), b"guess", &mut rng).is_err());
        let kh3 = open_or_create_vault(dir.path(), b"correct horse battery", &mut rng).unwrap();
        assert_eq!(db1, kh3.db_key().unwrap());
    }

    #[test]
    fn managed_chat_wrapper_preserves_data_keys_and_cryptographically_rejects_header_downgrade() {
        for passphrase in [
            b"scope secret".to_vec(),
            vec![b'l'; MAX_VAULT_SECRET_BYTES + 1],
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut rng = ChaCha20Rng::seed_from_u64(0x005c_0a31);
            // A real legacy-long wrapper needs the historical raw creation seam; ordinary new
            // vault creation correctly refuses such inputs. Its successful compatibility open
            // migrates to v2 before managed activation, exercising both predecessor formats.
            let dek = Dek::generate(&mut rng);
            let legacy =
                seal_and_encode_version(&dek, &passphrase, &mut rng, VAULT_VERSION_RAW_SECRET)
                    .unwrap();
            write_atomic(&dir.path().join(VAULT_FILE), &legacy).unwrap();
            let original = open_or_create_vault(dir.path(), &passphrase, &mut rng).unwrap();
            assert!(!chat_scope_vault_managed(dir.path()).unwrap());
            activate_chat_scope_vault(dir.path(), &passphrase, &mut rng).unwrap();
            assert!(chat_scope_vault_managed(dir.path()).unwrap());
            let managed = read_vault_wrapper(&dir.path().join(VAULT_FILE)).unwrap();
            let reopened = open_or_create_vault(dir.path(), &passphrase, &mut rng).unwrap();
            assert_eq!(original.db_key().unwrap(), reopened.db_key().unwrap());
            assert_eq!(original.blob_key().unwrap(), reopened.blob_key().unwrap());
            assert_eq!(
                original.mls_seal_key().unwrap(),
                reopened.mls_seal_key().unwrap()
            );
            for old_version in [VAULT_VERSION_RAW_SECRET, VAULT_VERSION_PREHASHED_SECRET] {
                let mut tampered = managed;
                tampered[0] = old_version;
                assert!(
                    decode_and_unseal(&tampered, &passphrase).is_err(),
                    "a parse tag alone must not allow an old reader to bypass retirement fences"
                );
            }
            change_vault_passphrase(
                dir.path(),
                &passphrase,
                b"replacement scope secret",
                &mut rng,
            )
            .unwrap();
            assert!(chat_scope_vault_managed(dir.path()).unwrap());
            let after =
                open_or_create_vault(dir.path(), b"replacement scope secret", &mut rng).unwrap();
            assert_eq!(original.db_key().unwrap(), after.db_key().unwrap());
        }
    }

    #[test]
    fn managed_activation_uses_real_replacement_boundary_and_retry_flushes_same_dek() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x005c_0a32);
        let original = open_or_create_vault(dir.path(), b"scope secret", &mut rng).unwrap();
        let old = read_vault_wrapper(&dir.path().join(VAULT_FILE)).unwrap();
        let before_rename = activate_chat_scope_vault_with_hooks(
            dir.path(),
            b"scope secret",
            &mut rng,
            |phase, staged| {
                if matches!(phase, AtomicWritePhase::TempSynced) {
                    std::fs::remove_file(staged).unwrap();
                }
            },
            sync_directory,
        );
        assert!(matches!(before_rename, Err(StorageError::Io(_))));
        assert_eq!(
            read_vault_wrapper(&dir.path().join(VAULT_FILE)).unwrap(),
            old
        );
        let after_rename = activate_chat_scope_vault_with_hooks(
            dir.path(),
            b"scope secret",
            &mut rng,
            |_, _| {},
            |_| {
                Err(std::io::Error::other(
                    "injected managed activation directory-sync failure",
                ))
            },
        );
        assert!(matches!(
            after_rename,
            Err(StorageError::CommittedButNotDurable(_))
        ));
        assert!(
            chat_scope_vault_managed(dir.path()).unwrap(),
            "actual rename installed managed bytes"
        );
        activate_chat_scope_vault(dir.path(), b"scope secret", &mut rng).unwrap();
        let reopened = open_or_create_vault(dir.path(), b"scope secret", &mut rng).unwrap();
        assert_eq!(original.db_key().unwrap(), reopened.db_key().unwrap());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn managed_activation_real_platform_sync_failure_is_not_acknowledged() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("original");
        let missing = root.path().join("missing-sync-parent");
        let mut rng = ChaCha20Rng::seed_from_u64(0x005c_0a33);
        let original = open_or_create_vault(&dir, b"scope secret", &mut rng).unwrap();
        let renamed = std::cell::Cell::new(false);
        let result = activate_chat_scope_vault_with_hooks(
            &dir,
            b"scope secret",
            &mut rng,
            |phase, _| {
                if matches!(phase, AtomicWritePhase::Renamed) {
                    renamed.set(true);
                }
            },
            |parent| {
                assert_eq!(parent, dir);
                assert!(
                    renamed.get(),
                    "real wrapper replacement must precede this barrier"
                );
                assert!(std::fs::symlink_metadata(&missing)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound));
                // Windows correctly prevents renaming a parent containing our live transaction
                // lock. Instead route this fault seam to a verified missing directory and call
                // the REAL platform callback. No fabricated error is returned; a Windows no-op
                // callback would wrongly report success and make this regression fail.
                sync_managed_directory(&missing)
            },
        );
        assert!(matches!(
            result,
            Err(StorageError::CommittedButNotDurable(_))
        ));
        assert!(chat_scope_vault_managed(&dir).unwrap());
        activate_chat_scope_vault(&dir, b"scope secret", &mut rng).unwrap();
        let reopened = open_or_create_vault(&dir, b"scope secret", &mut rng).unwrap();
        assert_eq!(original.db_key().unwrap(), reopened.db_key().unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn vault_directory_sync_uses_a_real_windows_handle() {
        let root = tempfile::tempdir().unwrap();
        sync_managed_directory(root.path()).expect("existing Windows directory flushes");
        assert_eq!(
            sync_managed_directory(&root.path().join("absent"))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotFound
        );
    }

    #[test]
    fn every_vault_authentication_path_rejects_empty_or_oversized_secrets_before_kdf() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x005e_c2e7);
        let oversized = vec![b'x'; MAX_VAULT_SECRET_BYTES + 1];

        for invalid in [b"".as_slice(), oversized.as_slice()] {
            assert!(matches!(
                open_or_create_vault(dir.path(), invalid, &mut rng),
                Err(StorageError::InvalidVaultSecret)
            ));
        }
        assert!(
            !vault_exists(dir.path()),
            "invalid first-run input must not publish a vault"
        );

        open_or_create_vault(dir.path(), b"valid secret", &mut rng).unwrap();
        for invalid in [b"".as_slice(), oversized.as_slice()] {
            assert!(verify_vault_passphrase(dir.path(), invalid).is_err());
            assert!(
                change_vault_passphrase(dir.path(), invalid, b"replacement", &mut rng).is_err()
            );
            assert!(matches!(
                change_vault_passphrase(dir.path(), b"valid secret", invalid, &mut rng),
                Err(StorageError::InvalidVaultSecret)
            ));
        }
        verify_vault_passphrase(dir.path(), b"valid secret").unwrap();
    }

    #[test]
    fn a_legacy_long_secret_opens_and_migrates_to_a_fixed_kdf_input() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x001e_9ac7);
        let legacy_secret = vec![b'l'; MAX_VAULT_SECRET_BYTES + 1];
        let dek = Dek::generate(&mut rng);
        let legacy =
            seal_and_encode_version(&dek, &legacy_secret, &mut rng, VAULT_VERSION_RAW_SECRET)
                .unwrap();
        write_atomic(&dir.path().join(VAULT_FILE), &legacy).unwrap();

        let expected = KeyHierarchy::new(dek).db_key().unwrap();
        let opened = open_or_create_vault(dir.path(), &legacy_secret, &mut rng).unwrap();
        assert_eq!(opened.db_key().unwrap(), expected);
        assert_eq!(
            std::fs::read(dir.path().join(VAULT_FILE)).unwrap()[0],
            VAULT_VERSION_PREHASHED_SECRET,
            "the successful compatibility open must retire raw long-secret Argon input"
        );
        assert_ne!(
            std::fs::read(dir.path().join(VAULT_FILE)).unwrap()[0],
            VAULT_VERSION_RAW_SECRET,
            "the migration is intentionally forward-only: a v1-only reader rejects this wrapper"
        );
        assert!(verify_vault_passphrase(dir.path(), &legacy_secret).is_ok());
        assert!(open_or_create_vault(dir.path(), b"wrong", &mut rng).is_err());
    }

    #[test]
    fn an_oversized_vault_wrapper_is_rejected_without_allocating_its_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(VAULT_FILE);
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        // A sparse file makes the hostile-size case cheap for the test while preserving the exact
        // metadata shape that previously drove `fs::read` to allocate according to file length.
        file.set_len(1024 * 1024 * 1024).unwrap();
        drop(file);

        assert!(matches!(
            verify_vault_passphrase(dir.path(), b"any secret"),
            Err(StorageError::Malformed)
        ));
        let mut rng = ChaCha20Rng::seed_from_u64(0xb0_0ded);
        assert!(matches!(
            open_or_create_vault(dir.path(), b"any secret", &mut rng),
            Err(StorageError::Malformed)
        ));
    }

    #[test]
    fn existence_flips_only_once_a_vault_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(3);
        // A directory that exists but holds no vault is still a first run: the UI keys its whole
        // setup-vs-unlock decision on this, and a false positive would hide the confirm step.
        assert!(!vault_exists(dir.path()));
        open_or_create_vault(dir.path(), b"pw", &mut rng).unwrap();
        assert!(vault_exists(dir.path()));
        // A failed unlock must not look like a fresh machine on the next launch.
        assert!(open_or_create_vault(dir.path(), b"wrong", &mut rng).is_err());
        assert!(vault_exists(dir.path()));
    }

    #[test]
    fn the_vault_file_does_not_leak_a_derived_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(2);
        let kh = open_or_create_vault(dir.path(), b"pw", &mut rng).unwrap();
        let db = kh.db_key().unwrap();
        let file = std::fs::read(dir.path().join("vault.bin")).unwrap();
        assert!(
            !file.windows(32).any(|w| w == db),
            "the on-disk vault must not contain any derived key (the DEK is sealed)"
        );
    }

    #[test]
    fn changing_the_passphrase_rewraps_the_same_dek_and_retires_the_old_secret() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(9);
        let before = open_or_create_vault(dir.path(), b"old secret", &mut rng).unwrap();
        let db = before.db_key().unwrap();
        let blob = before.blob_key().unwrap();

        change_vault_passphrase(dir.path(), b"old secret", b"new secret", &mut rng).unwrap();
        assert!(open_or_create_vault(dir.path(), b"old secret", &mut rng).is_err());
        let after = open_or_create_vault(dir.path(), b"new secret", &mut rng).unwrap();
        assert_eq!(after.db_key().unwrap(), db, "data keys must not rotate");
        assert_eq!(
            after.blob_key().unwrap(),
            blob,
            "existing blobs must remain openable"
        );
    }

    #[test]
    fn a_failed_passphrase_change_never_replaces_the_working_vault() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(10);
        open_or_create_vault(dir.path(), b"right", &mut rng).unwrap();
        let original = std::fs::read(dir.path().join(VAULT_FILE)).unwrap();

        assert!(change_vault_passphrase(dir.path(), b"wrong", b"new", &mut rng).is_err());
        assert_eq!(
            std::fs::read(dir.path().join(VAULT_FILE)).unwrap(),
            original
        );
        assert!(open_or_create_vault(dir.path(), b"right", &mut rng).is_ok());
        assert!(matches!(
            change_vault_passphrase(dir.path(), b"right", b"right", &mut rng),
            Err(StorageError::InvalidVaultSecret)
        ));
    }
}
