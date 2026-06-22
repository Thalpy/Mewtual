//! At-rest **key vault** (Phase 9a): a passphrase-sealed root DEK on disk, yielding a
//! [`KeyHierarchy`] (`db_key`/`mls_seal_key`/`blob_key`) for all on-disk sealing.
//!
//! The vault file is `version ‖ salt ‖ nonce ‖ sealed-DEK`. The root DEK is sealed with
//! XChaCha20-Poly1305 under a key Argon2id-derived from the passphrase + a per-vault random
//! salt (the salt is not secret; it lives in the file). The passphrase is never stored, so
//! an attacker with the file still needs it to unseal the DEK. A wrong passphrase fails as
//! an authenticated-decryption error ([`StorageError::Crypto`]) — never the wrong key.
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

        // A wrong passphrase fails (authenticated decryption) — not the wrong key — and
        // leaves the vault intact.
        assert!(open_or_create_vault(dir.path(), b"guess", &mut rng).is_err());
        let kh3 = open_or_create_vault(dir.path(), b"correct horse battery", &mut rng).unwrap();
        assert_eq!(db1, kh3.db_key().unwrap());
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
}
