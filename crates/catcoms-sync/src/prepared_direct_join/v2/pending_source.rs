//! Seal the original private KeyPackage provider under the exact joined pin.
use super::*;

const TAG: &[u8] = b"catcoms/private-chat-v2-pending-join-source/v1\0";
const VERSION: u16 = 1;
const KEY_DOMAIN: &str = "catcoms/private-chat-v2-pending-join-source-key/v1";
const MAX_MASTER: usize = 512 * 1024;
const MAX_PLAIN: usize =
    4 + TAG.len() + 2 + 4 * 36 + 8 + MAX_V2_JOIN_REQUEST_PACKET_BYTES + MAX_MASTER;

mod joined_outcome;
pub use joined_outcome::{RecoveredV2JoinedOutcome, SealedV2JoinedOutcome};

/// The actual journal and pin owner supplies these exact immutable labels.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct V2PendingJoinContextData {
    pub vault: [u8; 32],
    pub root: [u8; 32],
    pub original_pending: [u8; 32],
    pub pin: [u8; 32],
}

impl V2PendingJoinContextData {
    fn fields(&self) -> [&[u8; 32]; 4] {
        [&self.vault, &self.root, &self.original_pending, &self.pin]
    }

    fn validate(&self) -> Result<(), SyncError> {
        if self.fields().into_iter().any(|field| *field == [0; 32]) {
            return Err(SyncError::Malformed);
        }
        Ok(())
    }
}

pub struct SealedV2PendingJoinSource {
    blob: SealedBlob,
    original: Option<(V2PendingJoinContextData, [u8; 32])>,
}

impl SealedV2PendingJoinSource {
    /// The caller owns the bounded encrypted copy and its independent fee.
    pub fn from_encrypted_parts(nonce: [u8; 24], ciphertext: &[u8]) -> Result<Self, SyncError> {
        if !(17..=MAX_PLAIN + 16).contains(&ciphertext.len()) {
            return Err(SyncError::Malformed);
        }
        Ok(Self {
            blob: SealedBlob {
                nonce,
                ciphertext: ciphertext.to_vec(),
            },
            original: None,
        })
    }

    pub fn encrypted_parts(&self) -> (&[u8; 24], &[u8]) {
        (&self.blob.nonce, &self.blob.ciphertext)
    }
}

impl PreparedV2JoinRequest {
    /// Freeze one exact ciphertext before any local Welcome SDK entry. Only
    /// the mounted journal can authorize a later resume from these bytes.
    pub fn seal_pending_source(
        &mut self,
        key: &[u8; 32],
        expected: V2PendingJoinContextData,
        rng: &mut impl CryptoRngCore,
    ) -> Result<&SealedV2PendingJoinSource, SyncError> {
        expected.validate()?;
        if self.welcome_entered || self.completed.is_some() {
            return Err(SyncError::JoinRejected);
        }
        let key_id = *blake3::keyed_hash(key, TAG).as_bytes();
        if self.sealed_source.is_none() {
            if self.master.is_empty()
                || self.master.len() > self.limits.provider.min(MAX_MASTER)
                || self.request.len() > self.limits.exchange.min(MAX_V2_JOIN_REQUEST_PACKET_BYTES)
            {
                return Err(SyncError::Malformed);
            }
            let length = MAX_PLAIN - MAX_V2_JOIN_REQUEST_PACKET_BYTES - MAX_MASTER
                + self.request.len()
                + self.master.len();
            let malformed = |_| SyncError::Malformed;
            let mut e = Encoder::with_capacity(length);
            e.put_bytes(TAG).map_err(malformed)?;
            e.put_u16(VERSION);
            for field in expected.fields() {
                e.put_bytes(field).map_err(malformed)?;
            }
            e.put_bytes(&self.request).map_err(malformed)?;
            let mut plain = Zeroizing::new(e.finish());
            plain.extend_from_slice(&(self.master.len() as u32).to_be_bytes());
            plain.extend_from_slice(&self.master);
            let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
            let blob = seal(&key, &plain, rng).map_err(|_| SyncError::Malformed)?;
            self.sealed_source = Some(SealedV2PendingJoinSource {
                blob,
                original: Some((expected, key_id)),
            });
        }
        self.sealed_source
            .as_ref()
            .filter(|source| source.original == Some((expected, key_id)))
            .ok_or(SyncError::JoinRejected)
    }
}

/// Authenticated historical custody. No public conversion grants another
/// possible Welcome attempt or turns a recovered graph into fresh membership.
pub struct RecoveredV2PendingJoinSource {
    request: Bytes,
    limits: DirectJoinLimits,
    outcome_parent: [u8; 32],
}

impl RecoveredV2PendingJoinSource {
    pub fn open(
        sealed: &SealedV2PendingJoinSource,
        key: &[u8; 32],
        expected: V2PendingJoinContextData,
        accepted: &SignedInviteBindingData,
        limits: DirectJoinLimits,
    ) -> Result<Self, SyncError> {
        expected.validate()?;
        let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        let plain = Zeroizing::new(unseal(&key, &sealed.blob).map_err(|_| SyncError::Malformed)?);
        let mut d = Decoder::new(&plain);
        let malformed = |_| SyncError::Malformed;
        if d.get_bytes().map_err(malformed)? != TAG || d.get_u16().map_err(malformed)? != VERSION {
            return Err(SyncError::Malformed);
        }
        for value in expected.fields() {
            if d.get_bytes().map_err(malformed)? != value {
                return Err(SyncError::JoinRejected);
            }
        }
        let request = d.get_bytes().map_err(malformed)?;
        let master = d.get_bytes().map_err(malformed)?;
        d.finish().map_err(malformed)?;
        if request.len() > limits.exchange
            || master.is_empty()
            || master.len() > limits.provider.min(MAX_MASTER)
        {
            return Err(SyncError::Malformed);
        }
        let packet = V2JoinRequestPacketData::decode(request)?;
        if packet.invite.encode() != accepted.encode()
            || packet.invite.spec_data().inviter != accepted.spec_data().anchor.founder
        {
            return Err(SyncError::JoinRejected);
        }
        let original = PreparedV2JoinRequest {
            request: Bytes::copy_from_slice(request),
            master: Zeroizing::new(master.to_vec()),
            limits,
            welcome_entered: false,
            completed: None,
            sealed_source: None,
            sealed_outcome: None,
        };
        original.validate_retained_package_data()?;
        Ok(Self {
            request: original.request,
            limits,
            outcome_parent: joined_outcome::parent_data(sealed),
        })
    }

    pub fn request_bytes_data(&self) -> &[u8] {
        &self.request
    }

    pub fn validate_first_join_delivery_data(&self, bytes: &[u8]) -> Result<(), SyncError> {
        super::welcome::validate_delivery_data(&self.request, self.limits, bytes)
    }
}

#[cfg(test)]
mod tests;
