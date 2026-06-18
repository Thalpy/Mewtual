//! Per-device and per-account Ed25519 keypairs.
//!
//! Every device has its own signing key (its MLS leaf identity); a human's
//! *account* key is the trust root that certifies their devices. Both are kept
//! out of `Debug` output so secrets never leak into logs.

use core::fmt;

use catcoms_rt::CryptoRngCore;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::ids::{DeviceId, UserId};

/// A device's long-term Ed25519 identity keypair.
pub struct DeviceKeypair {
    signing: SigningKey,
}

impl DeviceKeypair {
    /// Generate a fresh keypair from injected randomness.
    pub fn generate(rng: &mut impl CryptoRngCore) -> Self {
        Self {
            signing: SigningKey::generate(rng),
        }
    }

    /// Reconstruct from a 32-byte secret seed (e.g. after unsealing from disk).
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            signing: SigningKey::from_bytes(seed),
        }
    }

    /// The 32-byte secret seed, for sealing at rest. Handle with care.
    pub fn seed(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }

    /// The public verifying key.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    /// This device's content-addressed id.
    pub fn device_id(&self) -> DeviceId {
        DeviceId::from_verifying_key(&self.verifying_key())
    }

    /// Sign a message, returning the 64-byte signature.
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.signing.sign(msg).to_bytes()
    }
}

impl fmt::Debug for DeviceKeypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceKeypair")
            .field("device_id", &self.device_id())
            .finish_non_exhaustive()
    }
}

/// A human account's Ed25519 keypair — the trust root certifying their devices.
pub struct AccountKeypair {
    signing: SigningKey,
}

impl AccountKeypair {
    /// Generate a fresh account keypair from injected randomness.
    pub fn generate(rng: &mut impl CryptoRngCore) -> Self {
        Self {
            signing: SigningKey::generate(rng),
        }
    }

    /// Reconstruct from a 32-byte secret seed.
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            signing: SigningKey::from_bytes(seed),
        }
    }

    /// The 32-byte secret seed, for sealing at rest.
    pub fn seed(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }

    /// The public verifying key.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    /// This account's content-addressed user id.
    pub fn user_id(&self) -> UserId {
        UserId::from_account_key(&self.verifying_key())
    }

    /// Sign a message, returning the 64-byte signature.
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.signing.sign(msg).to_bytes()
    }
}

impl fmt::Debug for AccountKeypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountKeypair")
            .field("user_id", &self.user_id())
            .finish_non_exhaustive()
    }
}

/// Verify an Ed25519 signature over `msg` against `vk` using strict verification
/// (rejects the known malleability / small-order edge cases).
pub fn verify(vk: &VerifyingKey, msg: &[u8], sig: &[u8; 64]) -> bool {
    vk.verify_strict(msg, &Signature::from_bytes(sig)).is_ok()
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
    fn sign_then_verify_roundtrips() {
        let kp = DeviceKeypair::generate(&mut rng(1));
        let sig = kp.sign(b"hello");
        assert!(verify(&kp.verifying_key(), b"hello", &sig));
    }

    #[test]
    fn verify_rejects_tampered_message() {
        let kp = DeviceKeypair::generate(&mut rng(1));
        let sig = kp.sign(b"hello");
        assert!(!verify(&kp.verifying_key(), b"hell0", &sig));
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let a = DeviceKeypair::generate(&mut rng(1));
        let b = DeviceKeypair::generate(&mut rng(2));
        let sig = a.sign(b"hello");
        assert!(!verify(&b.verifying_key(), b"hello", &sig));
    }

    #[test]
    fn seed_roundtrip_preserves_identity() {
        let kp = DeviceKeypair::generate(&mut rng(7));
        let restored = DeviceKeypair::from_seed(&kp.seed());
        assert_eq!(kp.device_id(), restored.device_id());
    }

    #[test]
    fn debug_does_not_leak_secret_seed() {
        let kp = DeviceKeypair::generate(&mut rng(1));
        let shown = format!("{kp:?}");
        let seed_hex: String = kp.seed().iter().map(|b| format!("{b:02x}")).collect();
        assert!(!shown.contains(&seed_hex));
    }
}
