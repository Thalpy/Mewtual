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

/// Leading byte distinguishing a [`FileManifest`] encoding from a legacy single [`FileRef`] in the
/// CRDT's opaque ref field. A `FileRef::encode` starts with a 4-byte length prefix (`0x00 …`), so a
/// `0xF1` first byte unambiguously marks a manifest.
const MANIFEST_TAG: u8 = 0xF1;

/// Hard upper bound on the chunk count a manifest may declare — a parse guard against a hostile
/// count, well above the real maximum (the product caps files at 256 MiB / 8 MiB chunks = 32).
const MAX_CHUNKS: u32 = 4096;

/// A file described as an ordered list of independently-sealed **chunks** plus the whole-file
/// identity, so a file larger than one blob-fetch response can be transferred chunk-by-chunk. The
/// common small-file case is a 1-chunk manifest. Each chunk is a full [`FileRef`] (its own content
/// key, wrapped under the channel file-wrap key) — reusing the reviewed `seal_file`/`open_file`
/// primitives per chunk, no new crypto. Stored inline in the channel CRDT like a single `FileRef`.
#[derive(Clone, Debug)]
pub struct FileManifest {
    /// Address of the whole-file plaintext — the file's stable identity (UI/embeds use this).
    pub plaintext_cid: Cid,
    /// Whole-file plaintext size in bytes.
    pub total_size: u64,
    /// MIME type.
    pub mime: String,
    /// The chunks in order; concatenating each chunk's decrypted plaintext reconstructs the file.
    pub chunks: Vec<FileRef>,
}

impl FileManifest {
    /// Canonical encoding for embedding in the channel CRDT (tagged so it's distinguishable from a
    /// legacy single `FileRef`).
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u8(MANIFEST_TAG);
        e.put_bytes(self.plaintext_cid.as_bytes()).expect("32 fits");
        e.put_u64(self.total_size);
        e.put_str(&self.mime).expect("mime fits");
        e.put_u32(self.chunks.len() as u32);
        for c in &self.chunks {
            e.put_bytes(&c.encode()).expect("chunk ref fits");
        }
        e.finish()
    }

    /// Parse a [`FileManifest`] produced by [`FileManifest::encode`]. Errors `Malformed` (and
    /// notably without consuming on a missing tag, so the caller can fall back to [`FileRef::decode`]
    /// for a legacy entry).
    pub fn decode(bytes: &[u8]) -> Result<Self, StorageError> {
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| StorageError::Malformed)? != MANIFEST_TAG {
            return Err(StorageError::Malformed);
        }
        let plaintext_cid: [u8; 32] = d
            .get_bytes()
            .map_err(|_| StorageError::Malformed)?
            .try_into()
            .map_err(|_| StorageError::Malformed)?;
        let total_size = d.get_u64().map_err(|_| StorageError::Malformed)?;
        let mime = d
            .get_str()
            .map_err(|_| StorageError::Malformed)?
            .to_string();
        let count = d.get_u32().map_err(|_| StorageError::Malformed)?;
        // Bound the count BEFORE pre-allocating, so a hostile/corrupt manifest with a huge count
        // can't trigger a multi-GB `Vec::with_capacity` and OOM the reader (the bytes themselves
        // are bounded by `get_bytes`, but the outer capacity is not).
        if count > MAX_CHUNKS {
            return Err(StorageError::Malformed);
        }
        let mut chunks = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let raw = d.get_bytes().map_err(|_| StorageError::Malformed)?;
            chunks.push(FileRef::decode(raw)?);
        }
        d.finish().map_err(|_| StorageError::Malformed)?;
        Ok(Self {
            plaintext_cid: Cid::from_bytes(plaintext_cid),
            total_size,
            mime,
            chunks,
        })
    }

    /// Read a ref field that is either a tagged `FileManifest` (new) or a legacy single `FileRef`
    /// (old): try the manifest, else decode one `FileRef` and present it as a 1-chunk manifest.
    pub fn decode_or_legacy(bytes: &[u8]) -> Result<Self, StorageError> {
        match Self::decode(bytes) {
            Ok(m) => Ok(m),
            Err(_) => {
                let r = FileRef::decode(bytes)?;
                Ok(Self {
                    plaintext_cid: r.plaintext_cid,
                    total_size: r.size,
                    mime: r.mime.clone(),
                    chunks: vec![r],
                })
            }
        }
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
    fn manifest_round_trips_and_reassembles() {
        let mut r = rng(7);
        let wrap = [9u8; 32];
        // Three chunks of a "file".
        let parts: [&[u8]; 3] = [b"alpha-", b"bravo-", b"charlie"];
        let whole: Vec<u8> = parts.concat();
        let mut chunks = Vec::new();
        for p in parts {
            let (file_ref, _stored) =
                seal_file(p, "application/octet-stream", &wrap, &mut r).unwrap();
            chunks.push(file_ref);
        }
        let manifest = FileManifest {
            plaintext_cid: Cid::of(&whole),
            total_size: whole.len() as u64,
            mime: "application/octet-stream".into(),
            chunks,
        };
        let encoded = manifest.encode();
        // The tag distinguishes it from a legacy single FileRef.
        assert_eq!(encoded[0], MANIFEST_TAG);

        let decoded = FileManifest::decode(&encoded).unwrap();
        assert_eq!(decoded.total_size, whole.len() as u64);
        assert_eq!(decoded.chunks.len(), 3);
        assert_eq!(decoded.plaintext_cid, Cid::of(&whole));
        // decode_or_legacy also accepts it.
        assert_eq!(
            FileManifest::decode_or_legacy(&encoded)
                .unwrap()
                .chunks
                .len(),
            3
        );
    }

    #[test]
    fn a_manifest_with_an_absurd_chunk_count_is_rejected_before_allocating() {
        // A hostile/corrupt manifest declaring a huge chunk count must be rejected at the count
        // check, not pre-allocate a multi-GB Vec.
        let mut e = catcoms_wire::Encoder::new();
        e.put_u8(MANIFEST_TAG);
        e.put_bytes(&[0u8; 32]).unwrap();
        e.put_u64(0);
        e.put_str("x").unwrap();
        e.put_u32(u32::MAX); // absurd count
        let bytes = e.finish();
        assert!(matches!(
            FileManifest::decode(&bytes),
            Err(StorageError::Malformed)
        ));
    }

    #[test]
    fn legacy_file_ref_reads_as_a_one_chunk_manifest() {
        let mut r = rng(3);
        let wrap = [4u8; 32];
        let (file_ref, _stored) =
            seal_file(b"old single-blob file", "text/plain", &wrap, &mut r).unwrap();
        let legacy = file_ref.encode(); // no manifest tag
        assert_ne!(legacy[0], MANIFEST_TAG);
        // A plain manifest decode rejects it...
        assert!(FileManifest::decode(&legacy).is_err());
        // ...but decode_or_legacy presents it as a 1-chunk manifest with the same identity.
        let m = FileManifest::decode_or_legacy(&legacy).unwrap();
        assert_eq!(m.chunks.len(), 1);
        assert_eq!(m.plaintext_cid, file_ref.plaintext_cid);
        assert_eq!(m.total_size, file_ref.size);
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
