//! Per-file encryption.
//!
//! Each file gets a fresh random content key; the ciphertext is what is stored
//! and content-addressed. The content key is then **wrapped** under a channel
//! wrap key with its own **per-file wrap nonce** — the review fix: because the
//! wrap key is shared per channel-epoch, reusing a nonce across files would be a
//! catastrophic AEAD misuse, so every file's wrap gets a unique random nonce.

use catcoms_crypto::{seal, unseal, SealedBlob};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};

use crate::cid::Cid;
use crate::StorageError;

/// Metadata describing an encrypted file: its content addresses, its wrapped
/// content key, and basic attributes. This is what travels in the channel CRDT;
/// the ciphertext blob itself is fetched by `ciphertext_cid`.
#[derive(Clone, Debug)]
pub struct FileRef {
    /// Address of the plaintext (stable identity of the file's content).
    pub plaintext_cid: Cid,
    /// Address of the stored/transferred ciphertext blob.
    pub ciphertext_cid: Cid,
    /// The content key sealed under the channel wrap key (with a per-file nonce).
    pub wrapped_key: SealedBlob,
    /// Plaintext size in bytes.
    pub size: u64,
    /// MIME type.
    pub mime: String,
}

fn encode_blob(blob: &SealedBlob) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + blob.ciphertext.len());
    out.extend_from_slice(&blob.nonce);
    out.extend_from_slice(&blob.ciphertext);
    out
}

fn decode_blob(bytes: &[u8]) -> Result<SealedBlob, StorageError> {
    if bytes.len() < 24 {
        return Err(StorageError::Malformed);
    }
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&bytes[..24]);
    Ok(SealedBlob {
        nonce,
        ciphertext: bytes[24..].to_vec(),
    })
}

/// Encrypt `plaintext` for storage. Returns its [`FileRef`] and the ciphertext
/// blob to store (named by `ciphertext_cid`). `wrap_key` is the channel's
/// file-wrap key for the current epoch.
pub fn seal_file(
    plaintext: &[u8],
    mime: &str,
    wrap_key: &[u8; 32],
    rng: &mut impl CryptoRngCore,
) -> Result<(FileRef, Vec<u8>), StorageError> {
    let mut content_key = [0u8; 32];
    rng.fill_bytes(&mut content_key);

    let content = seal(&content_key, plaintext, rng)?; // random per-file content nonce
    let stored = encode_blob(&content);
    let ciphertext_cid = Cid::of(&stored);
    let plaintext_cid = Cid::of(plaintext);

    let wrapped_key = seal(wrap_key, &content_key, rng)?; // random per-file wrap nonce

    Ok((
        FileRef {
            plaintext_cid,
            ciphertext_cid,
            wrapped_key,
            size: plaintext.len() as u64,
            mime: mime.to_string(),
        },
        stored,
    ))
}

/// Decrypt a stored ciphertext blob given its [`FileRef`] and the channel wrap
/// key. Verifies both content addresses.
pub fn open_file(
    stored: &[u8],
    file_ref: &FileRef,
    wrap_key: &[u8; 32],
) -> Result<Vec<u8>, StorageError> {
    if Cid::of(stored) != file_ref.ciphertext_cid {
        return Err(StorageError::CidMismatch);
    }
    let content_key_bytes = unseal(wrap_key, &file_ref.wrapped_key)?;
    let content_key: [u8; 32] = content_key_bytes
        .as_slice()
        .try_into()
        .map_err(|_| StorageError::Malformed)?;

    let content = decode_blob(stored)?;
    let plaintext = unseal(&content_key, &content)?;
    if Cid::of(&plaintext) != file_ref.plaintext_cid {
        return Err(StorageError::CidMismatch);
    }
    Ok(plaintext)
}

impl FileRef {
    /// Canonical encoding for embedding in the channel CRDT / wire.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(self.plaintext_cid.as_bytes()).expect("32 fits");
        e.put_bytes(self.ciphertext_cid.as_bytes())
            .expect("32 fits");
        e.put_bytes(&self.wrapped_key.nonce).expect("24 fits");
        e.put_bytes(&self.wrapped_key.ciphertext)
            .expect("wrapped key fits");
        e.put_u64(self.size);
        e.put_str(&self.mime).expect("mime fits");
        e.finish()
    }

    /// Parse a [`FileRef`] produced by [`FileRef::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, StorageError> {
        let mut d = Decoder::new(bytes);
        let plaintext_cid: [u8; 32] = d
            .get_bytes()
            .map_err(|_| StorageError::Malformed)?
            .try_into()
            .map_err(|_| StorageError::Malformed)?;
        let ciphertext_cid: [u8; 32] = d
            .get_bytes()
            .map_err(|_| StorageError::Malformed)?
            .try_into()
            .map_err(|_| StorageError::Malformed)?;
        let nonce: [u8; 24] = d
            .get_bytes()
            .map_err(|_| StorageError::Malformed)?
            .try_into()
            .map_err(|_| StorageError::Malformed)?;
        let ciphertext = d.get_bytes().map_err(|_| StorageError::Malformed)?.to_vec();
        let size = d.get_u64().map_err(|_| StorageError::Malformed)?;
        let mime = d
            .get_str()
            .map_err(|_| StorageError::Malformed)?
            .to_string();
        d.finish().map_err(|_| StorageError::Malformed)?;
        Ok(Self {
            plaintext_cid: Cid::from_bytes(plaintext_cid),
            ciphertext_cid: Cid::from_bytes(ciphertext_cid),
            wrapped_key: SealedBlob { nonce, ciphertext },
            size,
            mime,
        })
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
    fn seal_open_roundtrips() {
        let mut r = rng(1);
        let wrap = [5u8; 32];
        let (file_ref, stored) = seal_file(b"the file body", "text/plain", &wrap, &mut r).unwrap();
        assert_eq!(file_ref.ciphertext_cid, Cid::of(&stored));
        assert_eq!(file_ref.plaintext_cid, Cid::of(b"the file body"));
        assert_eq!(file_ref.size, 13);

        let plaintext = open_file(&stored, &file_ref, &wrap).unwrap();
        assert_eq!(plaintext, b"the file body");
    }

    #[test]
    fn wrong_wrap_key_fails() {
        let mut r = rng(1);
        let (file_ref, stored) =
            seal_file(b"secret", "application/octet-stream", &[5u8; 32], &mut r).unwrap();
        assert!(open_file(&stored, &file_ref, &[6u8; 32]).is_err());
    }

    #[test]
    fn tampered_ciphertext_is_detected() {
        let mut r = rng(1);
        let (file_ref, mut stored) = seal_file(b"secret", "x", &[5u8; 32], &mut r).unwrap();
        stored[30] ^= 0xFF;
        assert!(matches!(
            open_file(&stored, &file_ref, &[5u8; 32]),
            Err(StorageError::CidMismatch)
        ));
    }

    #[test]
    fn each_file_has_a_unique_wrap_nonce_and_content_key() {
        let mut r = rng(1);
        let wrap = [5u8; 32];
        let (ref1, s1) = seal_file(b"same body", "x", &wrap, &mut r).unwrap();
        let (ref2, s2) = seal_file(b"same body", "x", &wrap, &mut r).unwrap();
        // Same plaintext, same wrap key -> the wrap nonces MUST differ (no reuse).
        assert_ne!(ref1.wrapped_key.nonce, ref2.wrapped_key.nonce);
        // ...and independent content keys -> different stored ciphertext.
        assert_ne!(s1, s2);
        // Plaintext address is still stable across both.
        assert_eq!(ref1.plaintext_cid, ref2.plaintext_cid);
    }

    #[test]
    fn file_ref_encode_decode_roundtrips() {
        let mut r = rng(2);
        let (file_ref, _) = seal_file(b"body", "image/png", &[1u8; 32], &mut r).unwrap();
        let decoded = FileRef::decode(&file_ref.encode()).unwrap();
        assert_eq!(decoded.plaintext_cid, file_ref.plaintext_cid);
        assert_eq!(decoded.ciphertext_cid, file_ref.ciphertext_cid);
        assert_eq!(decoded.wrapped_key.nonce, file_ref.wrapped_key.nonce);
        assert_eq!(
            decoded.wrapped_key.ciphertext,
            file_ref.wrapped_key.ciphertext
        );
        assert_eq!(decoded.size, file_ref.size);
        assert_eq!(decoded.mime, file_ref.mime);
    }
}
