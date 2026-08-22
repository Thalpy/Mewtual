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

/// Incremental [`Cid`] computation for content that arrives in pieces.
///
/// A streamed upload never holds the whole file at once, so it cannot call [`Cid::of`]; it feeds
/// each chunk through here as the chunk arrives and finishes with the same address [`Cid::of`]
/// would have produced over the concatenation.
#[derive(Clone)]
pub struct CidHasher(blake3::Hasher);

impl CidHasher {
    /// A hasher over no bytes yet.
    pub fn new() -> Self {
        Self(blake3::Hasher::new())
    }

    /// Absorb the next run of bytes.
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// The content address of everything absorbed so far (the hasher stays usable).
    pub fn cid(&self) -> Cid {
        Cid(*self.0.finalize().as_bytes())
    }
}

impl Default for CidHasher {
    fn default() -> Self {
        Self::new()
    }
}

// Manual Debug: the partial hash state says nothing useful and printing it invites treating an
// intermediate digest as an address.
impl fmt::Debug for CidHasher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CidHasher(..)")
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
    fn streaming_matches_the_one_shot_address() {
        let whole = b"the quick brown fox jumps over the lazy dog";
        let mut h = CidHasher::new();
        for piece in whole.chunks(7) {
            h.update(piece);
        }
        assert_eq!(h.cid(), Cid::of(whole));
    }

    #[test]
    fn an_empty_stream_addresses_the_empty_input() {
        assert_eq!(CidHasher::new().cid(), Cid::of(b""));
    }

    #[test]
    fn hex_roundtrips() {
        let cid = Cid::of(b"data");
        assert_eq!(Cid::from_hex(&cid.to_hex()), Some(cid));
        assert_eq!(Cid::from_hex("not hex"), None);
    }
}
