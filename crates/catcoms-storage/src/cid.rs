//! Content identifiers.

use core::fmt;

/// A content address: `BLAKE3` of the bytes it names. Computed over ciphertext,
/// so it verifies a blob end-to-end without decrypting it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cid([u8; 32]);

impl Cid {
    /// The content address of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Wrap raw digest bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw 32-byte digest.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex form (used as the on-disk filename).
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Parse a hex form produced by [`Cid::to_hex`].
    pub fn from_hex(s: &str) -> Option<Self> {
        let bytes = hex::decode(s).ok()?;
        let arr: [u8; 32] = bytes.try_into().ok()?;
        Some(Self(arr))
    }
}

impl fmt::Debug for Cid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Cid({:02x}{:02x}{:02x}{:02x}…)",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
}

impl fmt::Display for Cid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_address_is_deterministic_and_distinguishing() {
        assert_eq!(Cid::of(b"hello"), Cid::of(b"hello"));
        assert_ne!(Cid::of(b"hello"), Cid::of(b"world"));
    }

    #[test]
    fn hex_roundtrips() {
        let cid = Cid::of(b"data");
        assert_eq!(Cid::from_hex(&cid.to_hex()), Some(cid));
        assert_eq!(Cid::from_hex("not hex"), None);
    }
}
