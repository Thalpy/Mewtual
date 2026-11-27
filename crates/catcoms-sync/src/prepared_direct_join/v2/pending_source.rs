//! Seal the original private KeyPackage provider under the exact joined pin.
catcoms_rt::redacted_debug! {
    NativeV2RejoinInputs<'_>;
    NativeV2RejoinFailure;
    V2PendingJoinContextData;
    SealedV2PendingJoinSource;
    RecoveredV2PendingJoinSource;
}

use super::*;

const TAG: &[u8] = b"catcoms/private-chat-v2-pending-join-source/v1\0";
const VERSION: u16 = 1;
const REJOIN_VERSION: u16 = 2;
const REMOVE_BYTES: usize = 2 * 36 + 8;
const KEY_DOMAIN: &str = "catcoms/private-chat-v2-pending-join-source-key/v1";
const MAX_MASTER: usize = 512 * 1024;
const FIXED_BYTES: usize = 4 + TAG.len() + 2 + 4 * 36 + 8;
const MAX_PLAIN: usize = FIXED_BYTES + REMOVE_BYTES + MAX_V2_JOIN_REQUEST_PACKET_BYTES + MAX_MASTER;

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

/// The precise terminal candidate already checked by the mounted owner.
pub type SelectedRejoinRemoval = (
    catcoms_replication::chat::owner_header::OwnerHeaderId,
    catcoms_replication::chat::owner_facts::OwnerFactId,
    u64,
);

fn require_removal_coordinates(removed: SelectedRejoinRemoval) -> Result<(), SyncError> {
    if removed.0.as_bytes() == &[0; 32]
        || removed.1.as_bytes() == &[0; 32]
        || !(2..MAX_V2_GOVERNING_JOIN_STEPS as u64).contains(&removed.2)
    {
        return Err(SyncError::JoinRejected);
    }
    Ok(())
}

/// A returned failure identifies whether this attempt entered the device
/// primitive. Lower layers can spend an admission cut without SDK entry.
pub enum NativeV2RejoinFailure {
    /// The device primitive was never called. A new authorized cut may retry.
    BeforeDeviceEntry(SyncError),
    /// Key entry was admitted. Do not draw another original for this intent.
    AfterDeviceEntry(SyncError),
}

impl NativeV2RejoinFailure {
    fn into_sync_error(self) -> SyncError {
        match self {
            Self::BeforeDeviceEntry(error) | Self::AfterDeviceEntry(error) => error,
        }
    }
}

/// Coordinates supplied by the mounted owner after proving the original
/// Admission's removal. Its selected evidence must stay current until the
/// new Pending is durable.
pub struct NativeV2RejoinInputs<'a> {
    pub previous_invite: &'a SignedInviteBindingData,
    pub previous_context: V2PendingJoinContextData,
    pub previous_request: &'a [u8],
    pub previous_signer: [u8; 32],
    pub previous_remove: SelectedRejoinRemoval,
    pub invite: &'a SignedInviteBindingData,
    pub token: [u8; 32],
    pub now_ms: u64,
    pub limits: DirectJoinLimits,
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

    /// The native journal owner must hold the exact selected Pending, its
    /// accepted pin, and the checked pre-Welcome state for this vault. Restore
    /// its private package under a separate current native cut. The Welcome
    /// SDK entry still requires the original durable Entry and its own cut.
    pub fn restore_native_pre_welcome(
        &self,
        key: &[u8; 32],
        expected: V2PendingJoinContextData,
        accepted: &SignedInviteBindingData,
        limits: DirectJoinLimits,
        mut native: NativeV2JoinKeyEntry<'_>,
    ) -> Result<PreparedV2JoinRequest, SyncError> {
        expected.validate()?;
        native.enter(NativeChatKeyPurpose::JoinerPendingSource)?;
        let mut original = self.decode_source(key, expected, accepted, limits)?;
        original.native_origin = Some(native.original.clone());
        original.sealed_source = Some(Self {
            blob: SealedBlob {
                nonce: self.blob.nonce,
                ciphertext: self.blob.ciphertext.clone(),
            },
            original: Some((expected, *blake3::keyed_hash(key, TAG).as_bytes())),
        });
        Ok(original)
    }

    /// Build a fresh request from only the original signing identity.
    /// This uses the new Source's JoinerDevice cut and returns a private
    /// request that can complete only its own KeyPackage and Welcome. The
    /// App owner must separately prove the selected Remove and fenced
    /// namespace before entry and prove ancestry before publishing A2.
    pub fn prepare_native_rejoin(
        &self,
        key: &[u8; 32],
        input: NativeV2RejoinInputs<'_>,
        native: NativeV2JoinKeyEntry<'_>,
    ) -> Result<PreparedV2JoinRequest, SyncError> {
        self.prepare_native_rejoin_classified(key, input, native)
            .map_err(NativeV2RejoinFailure::into_sync_error)
    }

    /// A returned refusal before device entry is distinct from any error
    /// after admitting the SDK. In particular RT may spend a cut while a UI
    /// lock wins its final admission check, with no device call performed.
    pub fn prepare_native_rejoin_classified(
        &self,
        key: &[u8; 32],
        input: NativeV2RejoinInputs<'_>,
        mut native: NativeV2JoinKeyEntry<'_>,
    ) -> Result<PreparedV2JoinRequest, NativeV2RejoinFailure> {
        use NativeV2RejoinFailure::{AfterDeviceEntry, BeforeDeviceEntry};
        let NativeV2RejoinInputs {
            previous_invite,
            previous_context,
            previous_request,
            previous_signer,
            previous_remove,
            invite,
            token,
            now_ms,
            limits,
        } = input;
        previous_context.validate().map_err(BeforeDeviceEntry)?;
        require_removal_coordinates(previous_remove).map_err(BeforeDeviceEntry)?;
        let old = previous_invite.spec_data();
        require_first_invite(&old.anchor, invite, token, now_ms).map_err(BeforeDeviceEntry)?;
        let packet =
            V2JoinRequestPacketData::decode(previous_request).map_err(BeforeDeviceEntry)?;
        let old_request = packet.request.spec_data();
        if packet.invite.encode() != previous_invite.encode()
            || previous_request.len() > limits.exchange
            || old_request.joiner != previous_signer
            || old_request.request_token == token
            || old.nonce == invite.spec_data().nonce
            || old.policy != invite.spec_data().policy
        {
            return Err(BeforeDeviceEntry(SyncError::JoinRejected));
        }

        native
            .enter(NativeChatKeyPurpose::JoinerDevice)
            .map_err(BeforeDeviceEntry)?;
        let original = self
            .decode_source(key, previous_context, previous_invite, limits)
            .map_err(AfterDeviceEntry)?;
        if original.request_bytes_data() != previous_request {
            return Err(AfterDeviceEntry(SyncError::JoinRejected));
        }
        // The caller funds the bounded decrypt/restore overlap and the
        // named 32 KiB one-signer SDK challenge before issuing the cut.
        let isolated = catcoms_mls::persist::restore_join_signer_only_bounded(
            &original.master,
            &previous_signer,
            (limits.provider, limits.backing),
        )
        .map_err(|error| AfterDeviceEntry(error.into()))?;
        drop(original);
        let mut request =
            PreparedV2JoinRequest::prepare(isolated, &old.anchor, invite, token, now_ms, limits)
                .map_err(AfterDeviceEntry)?;
        request.native_origin = Some(native.original.clone());
        request.rejoin_remove = Some(previous_remove);
        Ok(request)
    }

    fn decode_source(
        &self,
        key: &[u8; 32],
        expected: V2PendingJoinContextData,
        accepted: &SignedInviteBindingData,
        limits: DirectJoinLimits,
    ) -> Result<PreparedV2JoinRequest, SyncError> {
        expected.validate()?;
        let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        let plain = Zeroizing::new(unseal(&key, &self.blob).map_err(|_| SyncError::Malformed)?);
        let mut d = Decoder::new(&plain);
        let malformed = |_| SyncError::Malformed;
        if d.get_bytes().map_err(malformed)? != TAG {
            return Err(SyncError::Malformed);
        }
        let version = d.get_u16().map_err(malformed)?;
        for value in expected.fields() {
            if d.get_bytes().map_err(malformed)? != value {
                return Err(SyncError::JoinRejected);
            }
        }
        let rejoin_remove = match version {
            VERSION => None,
            REJOIN_VERSION => {
                use catcoms_replication::chat::{
                    owner_facts::OwnerFactId, owner_header::OwnerHeaderId,
                };
                let head = d
                    .get_bytes()
                    .map_err(malformed)?
                    .try_into()
                    .map_err(|_| SyncError::Malformed)?;
                let admission = d
                    .get_bytes()
                    .map_err(malformed)?
                    .try_into()
                    .map_err(|_| SyncError::Malformed)?;
                let epoch = d.get_u64().map_err(malformed)?;
                let selected = (
                    OwnerHeaderId::from_bytes(head),
                    OwnerFactId::from_bytes(admission),
                    epoch,
                );
                require_removal_coordinates(selected)?;
                Some(selected)
            }
            _ => return Err(SyncError::Malformed),
        };
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
            warm_sdk_payment: None,
            native_origin: None,
            rejoin_remove,
            completed: None,
            sealed_source: None,
            sealed_outcome: None,
            initial_prefix_origin: None,
        };
        original.validate_retained_package_data()?;
        Ok(original)
    }
}

impl PreparedV2JoinRequest {
    /// The original Remove this request must prove in every Welcome.
    pub fn rejoin_remove_data(&self) -> Option<SelectedRejoinRemoval> {
        self.rejoin_remove
    }

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
            let length = FIXED_BYTES
                + self.request.len()
                + self.master.len()
                + self.rejoin_remove.map_or(0, |_| REMOVE_BYTES);
            let malformed = |_| SyncError::Malformed;
            let mut e = Encoder::with_capacity(length);
            e.put_bytes(TAG).map_err(malformed)?;
            e.put_u16(if self.rejoin_remove.is_some() {
                REJOIN_VERSION
            } else {
                VERSION
            });
            for field in expected.fields() {
                e.put_bytes(field).map_err(malformed)?;
            }
            if let Some((head, admission, epoch)) = self.rejoin_remove {
                e.put_bytes(head.as_bytes()).map_err(malformed)?;
                e.put_bytes(admission.as_bytes()).map_err(malformed)?;
                e.put_u64(epoch);
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
    rejoin_remove: Option<SelectedRejoinRemoval>,
}

impl RecoveredV2PendingJoinSource {
    pub fn open(
        sealed: &SealedV2PendingJoinSource,
        key: &[u8; 32],
        expected: V2PendingJoinContextData,
        accepted: &SignedInviteBindingData,
        limits: DirectJoinLimits,
    ) -> Result<Self, SyncError> {
        let original = sealed.decode_source(key, expected, accepted, limits)?;
        Ok(Self {
            request: original.request,
            limits,
            outcome_parent: joined_outcome::parent_data(sealed),
            rejoin_remove: original.rejoin_remove,
        })
    }

    pub fn request_bytes_data(&self) -> &[u8] {
        &self.request
    }

    /// Terminal coordinates committed with this exact Pending. The physical
    /// rejoin owner must compare them with its replayed removal.
    pub fn rejoin_remove_data(&self) -> Option<SelectedRejoinRemoval> {
        self.rejoin_remove
    }

    pub fn validate_first_join_delivery_data(&self, bytes: &[u8]) -> Result<(), SyncError> {
        if self.rejoin_remove.is_some() {
            return Err(SyncError::JoinRejected);
        }
        super::welcome::validate_delivery_data(&self.request, self.limits, bytes)
    }

    pub fn validate_pinned_join_delivery_data(&self, bytes: &[u8]) -> Result<(), SyncError> {
        super::welcome::validate_pinned_delivery_data(
            &self.request,
            self.limits,
            bytes,
            self.rejoin_remove,
        )
    }
}

#[cfg(test)]
mod tests;
