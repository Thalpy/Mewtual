//! Content-addressed identifiers.
//!
//! A [`DeviceId`] / [`UserId`] is `BLAKE3(domain_label ‖ ed25519_public_key)`, so
//! it is a verifiable commitment to a key: you cannot present an id without
//! holding (or having seen) the matching public key, and you cannot forge an id
//! for a key you do not control.

use core::fmt;

use ed25519_dalek::VerifyingKey;

const DEVICE_ID_LABEL: &[u8] = b"catcoms/device-id/v1";
const USER_ID_LABEL: &[u8] = b"catcoms/user-id/v1";

fn content_id(label: &[u8], data: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(label);
    hasher.update(data);
    *hasher.finalize().as_bytes()
}

fn write_hex(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for b in bytes {
        write!(f, "{b:02x}")?;
    }
    Ok(())
}

/// A device's content-addressed identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId([u8; 32]);

impl DeviceId {
    /// Derive the id from the device's Ed25519 verifying key.
    pub fn from_verifying_key(vk: &VerifyingKey) -> Self {
        Self::from_public_key_bytes(vk.as_bytes())
    }

    /// Derive the id directly from raw Ed25519 public-key bytes — e.g. when the
    /// key comes from another library (openmls hands the MLS leaf signature key
    /// back as bytes). Same scheme as [`DeviceId::from_verifying_key`].
    pub fn from_public_key_bytes(public_key: &[u8]) -> Self {
        Self(content_id(DEVICE_ID_LABEL, public_key))
    }

    /// Reconstruct an id from raw bytes (e.g. when decoding the wire).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw 32 bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId(")?;
        write_hex(f, &self.0[..4])?;
        write!(f, "…)")
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(f, &self.0)
    }
}

/// A human account's content-addressed identifier (the trust root for a person's
/// devices).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserId([u8; 32]);

impl UserId {
    /// Derive the id from the account's Ed25519 verifying key.
    pub fn from_account_key(vk: &VerifyingKey) -> Self {
        Self(content_id(USER_ID_LABEL, vk.as_bytes()))
    }

    /// Reconstruct an id from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw 32 bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UserId(")?;
        write_hex(f, &self.0[..4])?;
        write!(f, "…)")
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(f, &self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn vk(seed: u64) -> VerifyingKey {
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        ed25519_dalek::SigningKey::generate(&mut rng).verifying_key()
    }

    #[test]
    fn device_id_is_deterministic_content_address() {
        let k = vk(1);
        assert_eq!(
            DeviceId::from_verifying_key(&k),
            DeviceId::from_verifying_key(&k)
        );
    }

    #[test]
    fn different_keys_yield_different_ids() {
        assert_ne!(
            DeviceId::from_verifying_key(&vk(1)),
            DeviceId::from_verifying_key(&vk(2))
        );
    }

    #[test]
    fn device_and_user_domains_are_separated() {
        // Same key bytes, different domain label -> different id.
        let k = vk(3);
        assert_ne!(
            DeviceId::from_verifying_key(&k).as_bytes(),
            UserId::from_account_key(&k).as_bytes()
        );
    }
}
