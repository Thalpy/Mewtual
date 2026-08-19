//! Replicated operations: an inner-signed CRDT delta ([`SignedOp`]) and its
//! transport-encrypted form ([`SealedOp`]).
//!
//! The **inner signature** is the load-bearing fix from the design review: each
//! op is signed by its author's device key over `(doc, author_pubkey, delta)`,
//! independently of how it is sealed. So when a catch-up peer decrypts history
//! and re-seals it under the current epoch, the original authorship still
//! verifies; a malicious re-sealer cannot forge or attribute ops it did not
//! author. The author's public key is carried in the op (and must content-address
//! its `author_device`), so authorship is verifiable without consulting a roster.
//!
//! **Size quantization (P10).** A [`SealedOp`] is padded to a bucket *before* it is sealed and
//! unpadded *after* it is opened, so the only length a forwarder can measure is the bucket's.
//! This matters because gossipsub is signed: a switchboard member (rung 2 of
//! `docs/design-zeroconf-reachability.md`) forwarding a message sees publisher, topic, sequence,
//! timestamp and, without this, a size that is the message's length plus a constant. The ladder
//! and its costs are documented on [`catcoms_storage::pad::OP_PAD_FLOOR`].
//!
//! The padding is a **transport** concern only. [`SignedOp::encode`] is unchanged, so the op
//! hash, the inner signature preimage and the persisted `EncryptedDoc` snapshot log are all
//! byte-identical to before; re-sealing on catch-up re-pads from the same unpadded op, and the
//! locked "deterministic byte-identical compaction" property (which is about that log) is
//! untouched. The frame is deterministic, so it draws nothing from the injected RNG seam.

use catcoms_crypto::{seal, unseal, verify_with_public_bytes, DeviceId, SealedBlob};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_rt::CryptoRngCore;
use catcoms_storage::pad::{self, OP_PAD_CEILING, OP_PAD_FLOOR};
use catcoms_wire::{Decoder, DocType, Encoder};

use crate::ReplError;

const OP_DOMAIN: &str = "catcoms/op/v1";

/// A CRDT delta authored and inner-signed by one device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedOp {
    /// Which kind of document this op belongs to.
    pub doc_type: DocType,
    /// Which document instance.
    pub doc_id: u128,
    /// The author device's content-addressed id (== hash of `author_pubkey`).
    pub author_device: DeviceId,
    /// The author's Ed25519 public key (raw bytes), so authorship is verifiable
    /// without a roster.
    pub author_pubkey: Vec<u8>,
    /// The opaque automerge change bytes.
    pub delta: Vec<u8>,
    /// The author's signature over the canonical payload.
    pub signature: [u8; 64],
}

fn signing_payload(doc_type: DocType, doc_id: u128, author_pubkey: &[u8], delta: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(OP_DOMAIN).expect("label fits");
    e.put_u16(doc_type.tag());
    e.put_u128(doc_id);
    e.put_bytes(author_pubkey).expect("pubkey fits");
    e.put_bytes(delta).expect("delta fits");
    e.finish()
}

impl SignedOp {
    /// Author and sign an op over `delta` for `(doc_type, doc_id)`.
    pub fn sign(
        device: &MlsDevice,
        doc_type: DocType,
        doc_id: u128,
        delta: Vec<u8>,
    ) -> Result<Self, ReplError> {
        let author_pubkey = device.public_key_bytes();
        let payload = signing_payload(doc_type, doc_id, &author_pubkey, &delta);
        let signature = device.sign(&payload)?;
        Ok(Self {
            doc_type,
            doc_id,
            author_device: device.device_id(),
            author_pubkey,
            delta,
            signature,
        })
    }

    /// Verify the op is authentically authored: the public key content-addresses
    /// the claimed device, and the signature is valid over the payload.
    pub fn verify(&self) -> bool {
        if DeviceId::from_public_key_bytes(&self.author_pubkey) != self.author_device {
            return false;
        }
        let payload = signing_payload(self.doc_type, self.doc_id, &self.author_pubkey, &self.delta);
        verify_with_public_bytes(&self.author_pubkey, &payload, &self.signature)
    }

    /// Canonical plaintext encoding (also the bytes that get sealed). The
    /// `author_device` is omitted; it is recomputed from `author_pubkey`.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u16(self.doc_type.tag());
        e.put_u128(self.doc_id);
        e.put_bytes(&self.author_pubkey).expect("pubkey fits");
        e.put_bytes(&self.delta).expect("delta fits");
        e.put_bytes(&self.signature).expect("64 fits");
        e.finish()
    }

    /// Decode an op produced by [`SignedOp::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        let mut d = Decoder::new(bytes);
        let tag = d.get_u16().map_err(|_| ReplError::Malformed)?;
        let doc_type = DocType::from_tag(tag).ok_or(ReplError::Malformed)?;
        let doc_id = d.get_u128().map_err(|_| ReplError::Malformed)?;
        let author_pubkey = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let delta = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let signature: [u8; 64] = d
            .get_bytes()
            .map_err(|_| ReplError::Malformed)?
            .try_into()
            .map_err(|_| ReplError::Malformed)?;
        d.finish().map_err(|_| ReplError::Malformed)?;
        Ok(Self {
            doc_type,
            doc_id,
            author_device: DeviceId::from_public_key_bytes(&author_pubkey),
            author_pubkey,
            delta,
            signature,
        })
    }

    /// A stable content hash of the op, used for de-duplication.
    pub fn hash(&self) -> [u8; 32] {
        *blake3::hash(&self.encode()).as_bytes()
    }
}

/// A [`SignedOp`] sealed for transport under a channel key at a given epoch.
/// `doc_type`, `doc_id` and `epoch` are in the clear so the receiver can select
/// the right decryption key; the authored content is encrypted.
#[derive(Clone, Debug)]
pub struct SealedOp {
    /// Document type (cleartext routing).
    pub doc_type: DocType,
    /// Document instance (cleartext routing).
    pub doc_id: u128,
    /// The epoch whose channel key sealed this op.
    pub epoch: u64,
    /// The sealed [`SignedOp`] bytes.
    pub blob: SealedBlob,
}

impl SealedOp {
    /// Seal `op` under `group`'s current-epoch channel key, size-quantized (see the module
    /// header): the sealed plaintext is a padded frame, so the ciphertext length reveals the
    /// bucket rather than the op.
    pub fn seal(
        op: &SignedOp,
        group: &ServerGroup,
        device: &MlsDevice,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, ReplError> {
        let key = group.channel_secret(device, op.doc_type, op.doc_id)?;
        let padded = pad::pad(&op.encode(), OP_PAD_FLOOR, OP_PAD_CEILING)
            .map_err(|_| ReplError::Malformed)?;
        let blob = seal(&key, &padded, rng)?;
        Ok(Self {
            doc_type: op.doc_type,
            doc_id: op.doc_id,
            epoch: group.epoch(),
            blob,
        })
    }

    /// Open a sealed op with the channel key for its epoch, peeling the padded frame.
    ///
    /// A malformed frame is a `Malformed` error, exactly like a malformed op: the AEAD has
    /// already established that whoever produced these bytes held the channel key, and a member
    /// that abuses that to send a non-canonical pad gets its op rejected rather than partially
    /// decoded. There is no fallback path for an unpadded payload, and there does not need to be:
    /// a `SealedOp` only ever exists in flight or in an in-memory queue, never at rest (the
    /// persisted log holds unpadded [`SignedOp`]s), so no stored state predates the frame.
    pub fn open(&self, channel_key: &[u8; 32]) -> Result<SignedOp, ReplError> {
        let plaintext = unseal(channel_key, &self.blob)?;
        let body = pad::unpad(&plaintext, OP_PAD_FLOOR, OP_PAD_CEILING)
            .map_err(|_| ReplError::Malformed)?;
        SignedOp::decode(body)
    }

    /// Canonical wire encoding for transport (gossip / catch-up).
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u16(self.doc_type.tag());
        e.put_u128(self.doc_id);
        e.put_u64(self.epoch);
        e.put_bytes(&self.blob.nonce).expect("24 fits");
        e.put_bytes(&self.blob.ciphertext).expect("ciphertext fits");
        e.finish()
    }

    /// Decode a sealed op produced by [`SealedOp::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        let mut d = Decoder::new(bytes);
        let tag = d.get_u16().map_err(|_| ReplError::Malformed)?;
        let doc_type = DocType::from_tag(tag).ok_or(ReplError::Malformed)?;
        let doc_id = d.get_u128().map_err(|_| ReplError::Malformed)?;
        let epoch = d.get_u64().map_err(|_| ReplError::Malformed)?;
        let nonce: [u8; 24] = d
            .get_bytes()
            .map_err(|_| ReplError::Malformed)?
            .try_into()
            .map_err(|_| ReplError::Malformed)?;
        let ciphertext = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        d.finish().map_err(|_| ReplError::Malformed)?;
        Ok(Self {
            doc_type,
            doc_id,
            epoch,
            blob: SealedBlob { nonce, ciphertext },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_crypto::seal as raw_seal;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    /// A one-member group and its device: enough to derive a channel key and seal under it.
    fn solo() -> (MlsDevice, ServerGroup) {
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        (device, group)
    }

    /// Sign an op whose `delta` is `n` bytes, and return it sealed.
    fn sealed_of(device: &MlsDevice, group: &ServerGroup, delta: Vec<u8>) -> (SignedOp, SealedOp) {
        let mut rng = ChaCha20Rng::seed_from_u64(9);
        let op = SignedOp::sign(device, DocType::Channel, 1, delta).unwrap();
        let sealed = SealedOp::seal(&op, group, device, &mut rng).unwrap();
        (op, sealed)
    }

    #[test]
    fn a_sealed_op_round_trips_through_the_pad_to_exactly_the_original() {
        let (device, group) = solo();
        let key = group.channel_secret(&device, DocType::Channel, 1).unwrap();
        for n in [0usize, 1, 100, 400, 1_000] {
            let (op, sealed) = sealed_of(&device, &group, vec![7u8; n]);
            assert_eq!(sealed.open(&key).unwrap(), op, "an op must survive padding");
        }
    }

    #[test]
    fn the_sealed_size_is_the_bucket_not_merely_bigger() {
        // The property padding has to have. A round-trip test would pass without any padding at
        // all; this fails unless the observed length is exactly a ladder value.
        let (device, group) = solo();
        for n in [0usize, 1, 50, 100] {
            let (_, sealed) = sealed_of(&device, &group, vec![7u8; n]);
            // XChaCha20-Poly1305 ciphertext = padded plaintext + 16-byte tag; the padded
            // plaintext is the bucket + the 4-byte length footer.
            assert_eq!(
                sealed.blob.ciphertext.len(),
                OP_PAD_FLOOR + 4 + 16,
                "a {n}-byte delta must seal to the floor bucket"
            );
        }
        // A delta that pushes the op past the floor lands on the next bucket, not somewhere
        // between: the off-by-one at the boundary.
        let plain_len = SignedOp::sign(&device, DocType::Channel, 1, vec![0u8; 0])
            .unwrap()
            .encode()
            .len();
        let just_over = OP_PAD_FLOOR - plain_len + 1;
        let (_, sealed) = sealed_of(&device, &group, vec![7u8; just_over]);
        assert_eq!(sealed.blob.ciphertext.len(), 2 * OP_PAD_FLOOR + 4 + 16);
        let (_, sealed) = sealed_of(&device, &group, vec![7u8; just_over - 1]);
        assert_eq!(sealed.blob.ciphertext.len(), OP_PAD_FLOOR + 4 + 16);
    }

    #[test]
    fn two_differently_sized_ops_in_one_bucket_are_identical_on_the_wire() {
        // What a forwarder measures. Before padding these differed by 300 bytes, which is the
        // message length in the clear.
        let (device, group) = solo();
        let (_, short) = sealed_of(&device, &group, vec![1u8; 2]);
        let (_, long) = sealed_of(&device, &group, vec![1u8; 300]);
        assert_eq!(short.encode().len(), long.encode().len());
    }

    #[test]
    fn the_pad_is_not_observable_outside_the_aead() {
        // Padding is a transport wrapper only: the op's canonical encoding, its content hash and
        // therefore its signature preimage and the persisted log are all untouched, and no
        // cleartext field of the wire form carries the real length.
        let (device, group) = solo();
        let (op, sealed) = sealed_of(&device, &group, b"a distinctive delta body".to_vec());
        assert_eq!(op.encode().len(), 24 + 4 + 2 + 16 + (4 + 32) + (4 + 64));
        assert_eq!(SignedOp::decode(&op.encode()).unwrap().hash(), op.hash());
        let wire = sealed.encode();
        // The only cleartext on the wire is routing (doc type, doc id, epoch, nonce); the body
        // never appears, so the fill cannot be located and stripped.
        assert!(
            !wire
                .windows(b"a distinctive delta body".len())
                .any(|w| w == b"a distinctive delta body"),
            "the plaintext must not appear on the wire"
        );
        // And the decoded header is length-free: it is the same for both bucket-mates.
        let (_, other) = sealed_of(&device, &group, b"x".to_vec());
        let a = SealedOp::decode(&wire).unwrap();
        let b = SealedOp::decode(&other.encode()).unwrap();
        assert_eq!(
            (a.doc_type, a.doc_id, a.epoch),
            (b.doc_type, b.doc_id, b.epoch)
        );
        assert_eq!(a.blob.ciphertext.len(), b.blob.ciphertext.len());
    }

    #[test]
    fn a_hostile_or_unpadded_frame_fails_closed() {
        // The AEAD proves only that the sender held the channel key. A member that abuses that to
        // seal a non-canonical frame must be rejected, not partially decoded, and must not panic.
        let (device, group) = solo();
        let key = group.channel_secret(&device, DocType::Channel, 1).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(5);
        let op = SignedOp::sign(&device, DocType::Channel, 1, b"body".to_vec()).unwrap();
        let plain = op.encode();

        let mut declared_too_long = vec![0u8; OP_PAD_FLOOR + 4];
        declared_too_long[..plain.len()].copy_from_slice(&plain);
        declared_too_long[OP_PAD_FLOOR..].copy_from_slice(&u32::MAX.to_be_bytes());

        let mut wrong_bucket = vec![0u8; 4 * OP_PAD_FLOOR + 4];
        wrong_bucket[..plain.len()].copy_from_slice(&plain);
        wrong_bucket[4 * OP_PAD_FLOOR..].copy_from_slice(&(plain.len() as u32).to_be_bytes());

        let mut noisy = catcoms_storage::pad::pad(&plain, OP_PAD_FLOOR, OP_PAD_CEILING).unwrap();
        noisy[OP_PAD_FLOOR - 1] = 0xAA;

        // ...and a wholly unpadded payload, i.e. what a pre-padding peer would send. There is no
        // fallback: a `SealedOp` never exists at rest, so nothing legitimate is unpadded.
        for frame in [
            declared_too_long,
            wrong_bucket,
            noisy,
            plain.clone(),
            Vec::new(),
        ] {
            let sealed = SealedOp {
                doc_type: DocType::Channel,
                doc_id: 1,
                epoch: group.epoch(),
                blob: raw_seal(&key, &frame, &mut rng).unwrap(),
            };
            assert!(
                matches!(sealed.open(&key), Err(ReplError::Malformed)),
                "a non-canonical frame must fail closed"
            );
        }
    }

    #[test]
    fn sealed_op_encode_decode_roundtrips() {
        let op = SealedOp {
            doc_type: DocType::Channel,
            doc_id: 7,
            epoch: 3,
            blob: SealedBlob {
                nonce: [1u8; 24],
                ciphertext: vec![9, 8, 7, 6],
            },
        };
        let decoded = SealedOp::decode(&op.encode()).unwrap();
        assert_eq!(decoded.doc_type, DocType::Channel);
        assert_eq!(decoded.doc_id, 7);
        assert_eq!(decoded.epoch, 3);
        assert_eq!(decoded.blob.nonce, [1u8; 24]);
        assert_eq!(decoded.blob.ciphertext, vec![9, 8, 7, 6]);
    }
}
