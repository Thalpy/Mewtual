//! Replicated operations: an inner-signed CRDT delta ([`SignedOp`]) and its
//! transport-encrypted form ([`SealedOp`]).
//!
//! The **inner signature** is the load-bearing fix from the design review: each
//! op is signed by its author's device key over `(doc, author_pubkey, delta)`,
//! independently of how it is sealed. So when a catch-up peer decrypts history
//! and re-seals it under the current epoch, the original authorship still
//! verifies — a malicious re-sealer cannot forge or attribute ops it did not
//! author. The author's public key is carried in the op (and must content-address
//! its `author_device`), so authorship is verifiable without consulting a roster.

use catcoms_crypto::{seal, unseal, verify_with_public_bytes, DeviceId, SealedBlob};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_rt::CryptoRngCore;
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
    /// `author_device` is omitted — it is recomputed from `author_pubkey`.
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
    /// Seal `op` under `group`'s current-epoch channel key.
    pub fn seal(
        op: &SignedOp,
        group: &ServerGroup,
        device: &MlsDevice,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, ReplError> {
        let key = group.channel_secret(device, op.doc_type, op.doc_id)?;
        let blob = seal(&key, &op.encode(), rng)?;
        Ok(Self {
            doc_type: op.doc_type,
            doc_id: op.doc_id,
            epoch: group.epoch(),
            blob,
        })
    }

    /// Open a sealed op with the channel key for its epoch.
    pub fn open(&self, channel_key: &[u8; 32]) -> Result<SignedOp, ReplError> {
        let plaintext = unseal(channel_key, &self.blob)?;
        SignedOp::decode(&plaintext)
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
