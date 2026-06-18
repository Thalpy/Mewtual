//! One key hierarchy, sealing primitives, and a tiered secure key store.
//!
//! There is exactly **one** root data-encryption key (DEK). Everything else is an
//! HKDF subkey of it — the SQLCipher DB key, the openmls value-sealing key, the
//! blob-at-rest key — so a single rotation rekeys the whole device and there is
//! no second, divergent root of trust.
//!
//! The DEK itself is sealed at rest by a [`SecureKeyStore`], which reports a
//! [`KeyTier`] describing how strong that protection is. A platform may offer a
//! hardware-backed store; the portable fallback is [`PassphraseKeyStore`]
//! (Argon2id). If the available tier ever *drops* below what protected the store
//! before, [`requires_passphrase_confirmation`] signals that the user must
//! confirm a passphrase before the store reopens — never a silent downgrade.

use core::fmt;

use catcoms_rt::CryptoRngCore;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroizing;

/// HKDF label for the SQLCipher database key.
pub const DB_KEY_LABEL: &str = "catcoms/db-key/v1";
/// HKDF label for the openmls value-sealing key.
pub const MLS_SEAL_LABEL: &str = "catcoms/mls-seal/v1";
/// HKDF label for the blob-at-rest key.
pub const BLOB_KEY_LABEL: &str = "catcoms/blob/v1";

/// Errors from key derivation, sealing, or the key store.
#[derive(Debug, Error)]
pub enum KeystoreError {
    /// Authenticated decryption failed (wrong key or tampered ciphertext).
    #[error("decryption/authentication failed")]
    Decrypt,
    /// Key derivation failed.
    #[error("key derivation failed")]
    Kdf,
    /// Password hashing (Argon2id) failed.
    #[error("password hashing failed")]
    PasswordHash,
}

/// How strongly the DEK is protected at rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTier {
    /// Hardware-backed (TEE/StrongBox/TPM). `attested` is true only when a key
    /// attestation has been verified to a trusted root.
    Hardware {
        /// Whether the hardware backing was cryptographically attested.
        attested: bool,
    },
    /// OS software keystore (e.g. Secret Service, DPAPI) — same-user readable.
    OsSoftware,
    /// Passphrase-derived (Argon2id), no OS/hardware backing.
    Passphrase,
    /// No at-rest protection.
    None,
}

impl KeyTier {
    /// A monotone strength ranking used to detect downgrades.
    pub fn strength(self) -> u8 {
        match self {
            KeyTier::Hardware { attested: true } => 4,
            KeyTier::Hardware { attested: false } => 3,
            KeyTier::OsSoftware => 2,
            KeyTier::Passphrase => 1,
            KeyTier::None => 0,
        }
    }
}

/// Whether the store must force passphrase confirmation: true when the tier
/// available now is weaker than the tier that protected the store previously.
pub fn requires_passphrase_confirmation(previous: KeyTier, current: KeyTier) -> bool {
    current.strength() < previous.strength()
}

/// The single 32-byte root data-encryption key. Zeroized on drop; redacted in
/// `Debug`.
pub struct Dek(Zeroizing<[u8; 32]>);

impl Dek {
    /// Generate a fresh DEK from injected randomness.
    pub fn generate(rng: &mut impl CryptoRngCore) -> Self {
        let mut k = [0u8; 32];
        rng.fill_bytes(&mut k);
        Self(Zeroizing::new(k))
    }

    /// Wrap raw key bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Borrow the raw key bytes. Handle with care; do not log or persist.
    pub fn expose_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Derive a 32-byte subkey for `label` via HKDF-Expand. Distinct labels give
    /// independent subkeys.
    pub fn subkey(&self, label: &str) -> Result<[u8; 32], KeystoreError> {
        let hk = Hkdf::<Sha256>::from_prk(&self.0[..]).map_err(|_| KeystoreError::Kdf)?;
        let mut out = [0u8; 32];
        hk.expand(label.as_bytes(), &mut out)
            .map_err(|_| KeystoreError::Kdf)?;
        Ok(out)
    }
}

impl fmt::Debug for Dek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Dek(***)")
    }
}

/// XChaCha20-Poly1305 ciphertext with its 24-byte nonce.
#[derive(Debug, Clone)]
pub struct SealedBlob {
    /// The 192-bit random nonce.
    pub nonce: [u8; 24],
    /// Ciphertext (including the Poly1305 tag).
    pub ciphertext: Vec<u8>,
}

/// Seal `plaintext` under `wrap_key` with a fresh random nonce.
pub fn seal(
    wrap_key: &[u8; 32],
    plaintext: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<SealedBlob, KeystoreError> {
    let cipher = XChaCha20Poly1305::new_from_slice(wrap_key).map_err(|_| KeystoreError::Kdf)?;
    let mut nonce = [0u8; 24];
    rng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext)
        .map_err(|_| KeystoreError::Decrypt)?;
    Ok(SealedBlob { nonce, ciphertext })
}

/// Open a blob sealed by [`seal`] under the same `wrap_key`.
pub fn unseal(wrap_key: &[u8; 32], blob: &SealedBlob) -> Result<Vec<u8>, KeystoreError> {
    let cipher = XChaCha20Poly1305::new_from_slice(wrap_key).map_err(|_| KeystoreError::Kdf)?;
    cipher
        .decrypt(XNonce::from_slice(&blob.nonce), blob.ciphertext.as_ref())
        .map_err(|_| KeystoreError::Decrypt)
}

/// The derived-subkey hierarchy rooted at one [`Dek`].
#[derive(Debug)]
pub struct KeyHierarchy {
    dek: Dek,
}

impl KeyHierarchy {
    /// Build the hierarchy over a DEK.
    pub fn new(dek: Dek) -> Self {
        Self { dek }
    }

    /// The SQLCipher database key.
    pub fn db_key(&self) -> Result<[u8; 32], KeystoreError> {
        self.dek.subkey(DB_KEY_LABEL)
    }

    /// The openmls value-sealing key.
    pub fn mls_seal_key(&self) -> Result<[u8; 32], KeystoreError> {
        self.dek.subkey(MLS_SEAL_LABEL)
    }

    /// The blob-at-rest key.
    pub fn blob_key(&self) -> Result<[u8; 32], KeystoreError> {
        self.dek.subkey(BLOB_KEY_LABEL)
    }
}

/// A platform-provided store that seals the DEK at rest and reports its tier.
pub trait SecureKeyStore: fmt::Debug {
    /// The protection tier this store currently provides.
    fn tier(&self) -> KeyTier;

    /// Seal the DEK for storage.
    fn seal_dek(&self, dek: &Dek, rng: &mut dyn CryptoRngCore)
        -> Result<SealedBlob, KeystoreError>;

    /// Recover the DEK from its sealed form.
    fn unseal_dek(&self, blob: &SealedBlob) -> Result<Dek, KeystoreError>;
}

fn dek_from_plaintext(pt: Vec<u8>) -> Result<Dek, KeystoreError> {
    let bytes: [u8; 32] = pt
        .as_slice()
        .try_into()
        .map_err(|_| KeystoreError::Decrypt)?;
    Ok(Dek::from_bytes(bytes))
}

/// Portable key store: wraps the DEK under an Argon2id-derived key. Works on
/// every platform (the universal floor / recovery path).
pub struct PassphraseKeyStore {
    wrap_key: Zeroizing<[u8; 32]>,
}

impl PassphraseKeyStore {
    /// Derive a store from a passphrase and salt (salt must be at least 8 bytes).
    pub fn derive(passphrase: &[u8], salt: &[u8]) -> Result<Self, KeystoreError> {
        let argon = argon2::Argon2::default();
        let mut wrap = [0u8; 32];
        argon
            .hash_password_into(passphrase, salt, &mut wrap)
            .map_err(|_| KeystoreError::PasswordHash)?;
        Ok(Self {
            wrap_key: Zeroizing::new(wrap),
        })
    }
}

impl fmt::Debug for PassphraseKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PassphraseKeyStore { wrap_key: *** }")
    }
}

impl SecureKeyStore for PassphraseKeyStore {
    fn tier(&self) -> KeyTier {
        KeyTier::Passphrase
    }

    fn seal_dek(
        &self,
        dek: &Dek,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<SealedBlob, KeystoreError> {
        seal(&self.wrap_key, dek.expose_bytes(), rng)
    }

    fn unseal_dek(&self, blob: &SealedBlob) -> Result<Dek, KeystoreError> {
        dek_from_plaintext(unseal(&self.wrap_key, blob)?)
    }
}

/// An in-memory key store for tests and ephemeral/dev use. Holds a random wrap
/// key in process memory (no persistence, no hardware backing).
pub struct InMemoryKeyStore {
    wrap_key: Zeroizing<[u8; 32]>,
}

impl InMemoryKeyStore {
    /// Create a store with a fresh random wrap key.
    pub fn generate(rng: &mut impl CryptoRngCore) -> Self {
        let mut k = [0u8; 32];
        rng.fill_bytes(&mut k);
        Self {
            wrap_key: Zeroizing::new(k),
        }
    }
}

impl fmt::Debug for InMemoryKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InMemoryKeyStore { wrap_key: *** }")
    }
}

impl SecureKeyStore for InMemoryKeyStore {
    fn tier(&self) -> KeyTier {
        KeyTier::OsSoftware
    }

    fn seal_dek(
        &self,
        dek: &Dek,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<SealedBlob, KeystoreError> {
        seal(&self.wrap_key, dek.expose_bytes(), rng)
    }

    fn unseal_dek(&self, blob: &SealedBlob) -> Result<Dek, KeystoreError> {
        dek_from_plaintext(unseal(&self.wrap_key, blob)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn rng(seed: u64) -> ChaCha20Rng {
        ChaCha20Rng::seed_from_u64(seed)
    }

    #[test]
    fn subkeys_are_distinct_and_deterministic() {
        let dek = Dek::from_bytes([7u8; 32]);
        let db = dek.subkey(DB_KEY_LABEL).unwrap();
        let mls = dek.subkey(MLS_SEAL_LABEL).unwrap();
        let blob = dek.subkey(BLOB_KEY_LABEL).unwrap();
        assert_ne!(db, mls);
        assert_ne!(db, blob);
        assert_ne!(mls, blob);
        // Deterministic for the same DEK.
        assert_eq!(db, Dek::from_bytes([7u8; 32]).subkey(DB_KEY_LABEL).unwrap());
    }

    #[test]
    fn seal_unseal_roundtrips() {
        let mut r = rng(1);
        let key = [3u8; 32];
        let blob = seal(&key, b"top secret", &mut r).unwrap();
        assert_eq!(unseal(&key, &blob).unwrap(), b"top secret");
    }

    #[test]
    fn unseal_with_wrong_key_fails() {
        let mut r = rng(1);
        let blob = seal(&[3u8; 32], b"secret", &mut r).unwrap();
        assert!(matches!(
            unseal(&[4u8; 32], &blob),
            Err(KeystoreError::Decrypt)
        ));
    }

    #[test]
    fn tampered_ciphertext_fails_authentication() {
        let mut r = rng(1);
        let mut blob = seal(&[3u8; 32], b"secret", &mut r).unwrap();
        blob.ciphertext[0] ^= 0xFF;
        assert!(matches!(
            unseal(&[3u8; 32], &blob),
            Err(KeystoreError::Decrypt)
        ));
    }

    #[test]
    fn passphrase_store_roundtrips_the_dek() {
        let mut r = rng(2);
        let dek = Dek::generate(&mut r);
        let expected = *dek.expose_bytes();

        let store = PassphraseKeyStore::derive(b"correct horse", &[9u8; 16]).unwrap();
        let sealed = store.seal_dek(&dek, &mut r).unwrap();
        let recovered = store.unseal_dek(&sealed).unwrap();
        assert_eq!(recovered.expose_bytes(), &expected);
    }

    #[test]
    fn wrong_passphrase_cannot_unseal() {
        let mut r = rng(3);
        let dek = Dek::generate(&mut r);
        let store = PassphraseKeyStore::derive(b"correct horse", &[9u8; 16]).unwrap();
        let sealed = store.seal_dek(&dek, &mut r).unwrap();

        let wrong = PassphraseKeyStore::derive(b"battery staple", &[9u8; 16]).unwrap();
        assert!(matches!(
            wrong.unseal_dek(&sealed),
            Err(KeystoreError::Decrypt)
        ));
    }

    #[test]
    fn rotation_invalidates_the_old_sealed_dek() {
        let mut r = rng(4);
        let dek = Dek::generate(&mut r);
        let old = PassphraseKeyStore::derive(b"old", &[1u8; 16]).unwrap();
        let sealed_old = old.seal_dek(&dek, &mut r).unwrap();

        // Re-key: a store derived from new material must not open the old blob.
        let new = PassphraseKeyStore::derive(b"new", &[2u8; 16]).unwrap();
        assert!(new.unseal_dek(&sealed_old).is_err());
    }

    #[test]
    fn key_hierarchy_exposes_three_independent_subkeys() {
        let kh = KeyHierarchy::new(Dek::from_bytes([1u8; 32]));
        let db = kh.db_key().unwrap();
        let mls = kh.mls_seal_key().unwrap();
        let blob = kh.blob_key().unwrap();
        assert_ne!(db, mls);
        assert_ne!(mls, blob);
        assert_ne!(db, blob);
    }

    #[test]
    fn downgrade_forces_passphrase_confirmation() {
        let hw = KeyTier::Hardware { attested: true };
        assert!(requires_passphrase_confirmation(hw, KeyTier::Passphrase));
        assert!(requires_passphrase_confirmation(hw, KeyTier::None));
        // Same or stronger tier does not force confirmation.
        assert!(!requires_passphrase_confirmation(hw, hw));
        assert!(!requires_passphrase_confirmation(
            KeyTier::Passphrase,
            KeyTier::OsSoftware
        ));
    }

    #[test]
    fn in_memory_store_roundtrips() {
        let mut r = rng(5);
        let store = InMemoryKeyStore::generate(&mut r);
        let dek = Dek::generate(&mut r);
        let expected = *dek.expose_bytes();
        let sealed = store.seal_dek(&dek, &mut r).unwrap();
        assert_eq!(store.unseal_dek(&sealed).unwrap().expose_bytes(), &expected);
    }
}
