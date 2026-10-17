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

    /// Derive the id directly from raw Ed25519 public-key bytes; e.g. when the
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

    /// Parse the canonical text form: exactly 64 **lowercase** hex characters, the encoding
    /// [`fmt::Display`] writes and the frontend's `MemberView.identity` carries.
    ///
    /// This is the one accepted spelling of a full device identity at a string boundary (the
    /// desktop bridge's call-signal target, for instance). It is deliberately strict: uppercase,
    /// a `0x` prefix, whitespace or any other length returns `None`, so two different strings can
    /// never name the same device and a caller cannot smuggle a short fingerprint through as an
    /// identity. A successful parse proves only the spelling, never membership or key possession;
    /// callers still check the id against the current roster.
    pub fn from_canonical_hex(text: &str) -> Option<Self> {
        let raw = text.as_bytes();
        if raw.len() != 64 {
            return None;
        }
        fn nibble(c: u8) -> Option<u8> {
            match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                _ => None,
            }
        }
        let mut bytes = [0u8; 32];
        for (i, pair) in raw.chunks_exact(2).enumerate() {
            bytes[i] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Some(Self(bytes))
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
    fn canonical_hex_round_trips_the_display_form_and_nothing_else() {
        let id = DeviceId::from_verifying_key(&vk(4));
        let text = id.to_string();
        assert_eq!(text.len(), 64);
        assert_eq!(DeviceId::from_canonical_hex(&text), Some(id));
        // Every non-canonical spelling of the same id is refused, so one device has one name.
        assert_eq!(DeviceId::from_canonical_hex(&text.to_uppercase()), None);
        assert_eq!(DeviceId::from_canonical_hex(&format!("0x{text}")), None);
        assert_eq!(
            DeviceId::from_canonical_hex(&format!(" {}", &text[1..])),
            None
        );
        assert_eq!(DeviceId::from_canonical_hex(&text[..62]), None);
        assert_eq!(DeviceId::from_canonical_hex(&format!("{text}00")), None);
        // The eight-character display fingerprint is not an identity.
        assert_eq!(DeviceId::from_canonical_hex(&text[..8]), None);
        assert_eq!(DeviceId::from_canonical_hex(&"g".repeat(64)), None);
        // A multi-byte character can never pass as two hex digits.
        assert_eq!(
            DeviceId::from_canonical_hex(&format!("é{}", &text[2..])),
            None
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
