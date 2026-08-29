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

use catcoms_crypto::{Dek, KeyHierarchy, PassphraseKeyStore, SealedBlob, SecureKeyStore};
use catcoms_rt::CryptoRngCore;
use zeroize::Zeroizing;

use crate::StorageError;

const VAULT_VERSION_RAW_SECRET: u8 = 1;
/// Version 2 normalizes a legacy-long user secret to a fixed 32-byte, domain-separated BLAKE3
/// value before Argon2. Short secrets continue to use v1 so existing vault bytes remain stable.
const VAULT_VERSION_PREHASHED_SECRET: u8 = 2;
const LONG_SECRET_DOMAIN: &str = "mewtual-vault-long-secret/v2";
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

fn open_or_create_vault_with_hooks(
    dir: impl AsRef<Path>,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
    lock_acquired: impl FnOnce(),
    candidate_ready: impl FnOnce(),
) -> Result<KeyHierarchy, StorageError> {
    validate_existing_vault_secret_shape(passphrase)?;
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).map_err(|e| StorageError::Io(e.to_string()))?;
    // This OS lock is held from the existence check through key generation/read and durable
    // publication. It is interprocess (unlike the desktop's mutex) and released automatically if
    // the process exits, so two first-run apps can never both return different in-memory DEKs.
    let _lock = lock_vault(dir)?;
    lock_acquired();
    let path = dir.join(VAULT_FILE);
    let dek = if path.exists() {
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
        dek
    } else {
        validate_new_vault_secret(passphrase)?;
        let dek = Dek::generate(rng);
        let bytes = seal_and_encode(&dek, passphrase, rng)?;
        candidate_ready();
        write_atomic(&path, &bytes)?;
        dek
    };
    Ok(KeyHierarchy::new(dek))
}

/// Does a vault already exist under `dir`? The gate before [`open_or_create_vault`] is asked to
/// open one: that call *creates* on first use, so a UI with no way to tell the two cases apart
/// turns a mistyped passphrase on a fresh install into a brand-new identity, silently. Says
/// nothing about whether any given passphrase opens it.
pub fn vault_exists(dir: impl AsRef<Path>) -> bool {
    dir.as_ref().join(VAULT_FILE).exists()
}

/// An exclusive installation mount held for the lifetime of the product's loaded server store.
/// Dropping this value, including during process termination, releases the operating-system lock.
#[derive(Debug)]
pub struct VaultSessionGuard {
    _lock: File,
}

/// Refuse a second application process from mounting and mutating the same vault concurrently.
/// Byte-atomic files are not enough for MLS/invite-ledger state: two actors starting from one
/// snapshot could each publish a valid but logically divergent successor. This lock is nonblocking
/// so a suspended owner cannot hang the contender's unlock UI.
pub fn acquire_vault_session(dir: impl AsRef<Path>) -> Result<VaultSessionGuard, StorageError> {
    std::fs::create_dir_all(dir.as_ref()).map_err(|e| StorageError::Io(e.to_string()))?;
    let lock = try_lock_file(&dir.as_ref().join(VAULT_SESSION_LOCK_FILE))?;
    Ok(VaultSessionGuard { _lock: lock })
}

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
    let replacement = seal_and_encode(&dek, new_passphrase, rng)?;
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
        Some(VAULT_VERSION_RAW_SECRET | VAULT_VERSION_PREHASHED_SECRET) => Ok(bytes[0]),
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
        _ => Err(StorageError::Malformed),
    }
}

fn lock_vault(dir: &Path) -> Result<File, StorageError> {
    try_lock_file(&dir.join(VAULT_LOCK_FILE))
}

fn try_lock_file(path: &Path) -> Result<File, StorageError> {
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

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
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
    fn every_vault_authentication_path_rejects_empty_or_oversized_secrets_before_kdf() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x5ec2_e7);
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
        let mut rng = ChaCha20Rng::seed_from_u64(0x1e9a_c7);
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
