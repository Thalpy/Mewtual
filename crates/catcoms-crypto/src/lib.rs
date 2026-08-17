//! Mewtual cryptographic identity and key management (Phase 1).
//!
//! - [`ids`]; content-addressed [`DeviceId`] / [`UserId`]
//!   (`BLAKE3(label ‖ ed25519_pubkey)`), so an id cannot be forged without the key.
//! - [`identity`]; per-device and per-account Ed25519 keypairs and strict
//!   signature verification.
//! - [`pairing`]; the **v2** multi-device model (`docs/design-multi-device.md`):
//!   the member's *origin device* is the identity root, so a
//!   [`pairing::DeviceCertificate`] is `sig_origin(origin ‖ companion ‖ name ‖ ts)`
//!   minted during a SAS-gated grant ceremony, and the chain is exactly one deep.
//!   [`pairing::DeviceRevocation`] withdraws one and [`pairing::MasterHandoff`]
//!   moves the right to mint either. This supersedes the deleted v1 module, whose
//!   account-rooted multi-hop chain had no users; every domain here is `/v2`, so
//!   no v1 statement could ever verify against a v2 one.
//! - [`keystore`]; one key hierarchy (a DEK → HKDF subkeys for the DB, MLS value
//!   sealing and blobs), XChaCha20-Poly1305 sealing, a tiered
//!   [`keystore::SecureKeyStore`], and a portable Argon2id passphrase store.
//!
//! All randomness is injected (`&mut impl CryptoRngCore`) and there is no ambient
//! time, so every operation is deterministically testable.

pub mod identity;
pub mod ids;
pub mod keystore;
pub mod pairing;

pub use identity::{verify, verify_with_public_bytes, AccountKeypair, DeviceKeypair};
pub use ids::{DeviceId, UserId};
pub use keystore::{
    requires_passphrase_confirmation, seal, unseal, Dek, InMemoryKeyStore, KeyHierarchy, KeyTier,
    KeystoreError, PassphraseKeyStore, SealedBlob, SecureKeyStore,
};
pub use pairing::{
    sas, validate_device_name, DeviceCertificate, DeviceRevocation, MasterHandoff, PairingError,
    PairingRequest, MAX_DEVICE_NAME_BYTES, SAS_DIGITS, SAS_MODULUS,
};
