//! CatComs cryptographic identity and key management (Phase 1).
//!
//! - [`ids`] — content-addressed [`DeviceId`] / [`UserId`]
//!   (`BLAKE3(label ‖ ed25519_pubkey)`), so an id cannot be forged without the key.
//! - [`identity`] — per-device and per-account Ed25519 keypairs and strict
//!   signature verification. The account key is the human's trust root.
//! - [`cert`] — device-certificate chains: the account key signs the founding
//!   device, and existing devices cross-certify new ones, so the account key need
//!   never leave a device. [`cert::Roster`] resolves the set of currently-valid
//!   devices, applying revocation, chain-depth and device-count limits, and binds
//!   every certificate to its `user_id` so a cert can't be replayed under another
//!   account.
//! - [`pairing`] — the **v2** multi-device model (`docs/design-multi-device.md`):
//!   the member's *origin device* is the identity root, so a
//!   [`pairing::DeviceCertificate`] is `sig_origin(origin ‖ companion ‖ name ‖ ts)`
//!   minted during a SAS-gated grant ceremony, and the chain is exactly one deep.
//!   Supersedes [`cert`]'s account-rooted chain; the two use different signing
//!   domains (`/v2` vs `/v1`) and can never cross-verify. Note the two modules
//!   each define a `DeviceRevocation`; only [`cert`]'s is re-exported at the crate
//!   root, so name the v2 one as [`pairing::DeviceRevocation`].
//! - [`keystore`] — one key hierarchy (a DEK → HKDF subkeys for the DB, MLS value
//!   sealing and blobs), XChaCha20-Poly1305 sealing, a tiered
//!   [`keystore::SecureKeyStore`], and a portable Argon2id passphrase store.
//!
//! All randomness is injected (`&mut impl CryptoRngCore`) and there is no ambient
//! time, so every operation is deterministically testable.

pub mod cert;
pub mod identity;
pub mod ids;
pub mod keystore;
pub mod pairing;

pub use cert::{CertError, CertSigner, DeviceCert, DeviceRevocation, Roster, RosterConfig};
pub use identity::{verify, verify_with_public_bytes, AccountKeypair, DeviceKeypair};
pub use ids::{DeviceId, UserId};
pub use keystore::{
    requires_passphrase_confirmation, seal, unseal, Dek, InMemoryKeyStore, KeyHierarchy, KeyTier,
    KeystoreError, PassphraseKeyStore, SealedBlob, SecureKeyStore,
};
pub use pairing::{
    sas, validate_device_name, DeviceCertificate, PairingError, PairingRequest,
    MAX_DEVICE_NAME_BYTES, SAS_DIGITS, SAS_MODULUS,
};
