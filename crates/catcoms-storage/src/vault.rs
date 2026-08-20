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

use std::path::Path;

use catcoms_crypto::{Dek, KeyHierarchy, PassphraseKeyStore, SealedBlob, SecureKeyStore};
use catcoms_rt::CryptoRngCore;

use crate::StorageError;

const VAULT_VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const VAULT_FILE: &str = "vault.bin";
/// Bound KDF inputs before Argon2 work and before a bridge accepts an attacker-sized allocation.
pub const MAX_VAULT_SECRET_BYTES: usize = 4096;

/// Open the key vault under `dir`, creating it (a fresh random DEK) on first use, and
/// returning the [`KeyHierarchy`] unsealed with `passphrase`. A wrong passphrase fails with
/// an authentication error rather than silently returning the wrong key, and never
/// overwrites the existing vault.
pub fn open_or_create_vault(
    dir: impl AsRef<Path>,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<KeyHierarchy, StorageError> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).map_err(|e| StorageError::Io(e.to_string()))?;
    let path = dir.join(VAULT_FILE);
    let dek = if path.exists() {
        let bytes = std::fs::read(&path).map_err(|e| StorageError::Io(e.to_string()))?;
        decode_and_unseal(&bytes, passphrase)?
    } else {
        let dek = Dek::generate(rng);
        let bytes = seal_and_encode(&dek, passphrase, rng)?;
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

/// Rewrap the existing root DEK under `new_passphrase`, preserving every derived data key.
///
/// This is intentionally a small atomic rewrite of `vault.bin`, not a decrypt/re-encrypt pass over
/// server snapshots and blobs: the passphrase protects the random DEK, while those records are
/// protected by keys derived from that DEK. The current passphrase is authenticated before any
/// write, a fresh salt and nonce are generated, and a failure leaves the previous vault in place.
pub fn change_vault_passphrase(
    dir: impl AsRef<Path>,
    current_passphrase: &[u8],
    new_passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<(), StorageError> {
    if current_passphrase.is_empty()
        || new_passphrase.is_empty()
        || current_passphrase.len() > MAX_VAULT_SECRET_BYTES
        || new_passphrase.len() > MAX_VAULT_SECRET_BYTES
        || current_passphrase == new_passphrase
    {
        return Err(StorageError::InvalidVaultSecret);
    }
    let path = dir.as_ref().join(VAULT_FILE);
    let old = std::fs::read(&path).map_err(|e| StorageError::Io(e.to_string()))?;
    let dek = decode_and_unseal(&old, current_passphrase)?;
    let replacement = seal_and_encode(&dek, new_passphrase, rng)?;
    // Authenticate the generated wrapper before it can replace the user's only live vault file.
    let _ = decode_and_unseal(&replacement, new_passphrase)?;
    write_atomic(&path, &replacement)
}

/// Seal a DEK under a fresh-salt passphrase store and encode the vault file bytes.
fn seal_and_encode(
    dek: &Dek,
    passphrase: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<Vec<u8>, StorageError> {
    let mut salt = [0u8; SALT_LEN];
    rng.fill_bytes(&mut salt);
    let ks = PassphraseKeyStore::derive(passphrase, &salt)?;
    let sealed = ks.seal_dek(dek, rng)?;
    let mut out = Vec::with_capacity(1 + SALT_LEN + NONCE_LEN + sealed.ciphertext.len());
    out.push(VAULT_VERSION);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&sealed.nonce);
    out.extend_from_slice(&sealed.ciphertext);
    Ok(out)
}

/// Decode the vault file bytes and unseal the DEK with `passphrase`.
fn decode_and_unseal(bytes: &[u8], passphrase: &[u8]) -> Result<Dek, StorageError> {
    let header = 1 + SALT_LEN + NONCE_LEN;
    if bytes.len() <= header || bytes[0] != VAULT_VERSION {
        return Err(StorageError::Malformed);
    }
    let salt = &bytes[1..1 + SALT_LEN];
    let nonce: [u8; NONCE_LEN] = bytes[1 + SALT_LEN..header]
        .try_into()
        .expect("slice length checked above");
    let ciphertext = bytes[header..].to_vec();
    let ks = PassphraseKeyStore::derive(passphrase, salt)?;
    Ok(ks.unseal_dek(&SealedBlob { nonce, ciphertext })?)
}

/// Write `bytes` to `path` atomically (temp file + rename), so a crash mid-write can't
/// leave a half-written vault.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| StorageError::Io(e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| StorageError::Io(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

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
