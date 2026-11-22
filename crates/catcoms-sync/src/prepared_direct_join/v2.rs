//! Typed V2 join request and its private original KeyPackage provider.
//! The request does not select a transport or confer an installed pin.
use super::*;
use crate::owner_v2::{V2FounderAddRequestData, MAX_FOUNDER_ADD_KEY_PACKAGE_BYTES};
use catcoms_replication::chat::join_bindings::{
    GenesisAnchorBindingData, JoinPayloadKindData, JoinRequestBindingSpecData,
    SignedInviteBindingData, SignedJoinRequestBindingData, MAX_JOIN_BINDING_BYTES,
};
use catcoms_replication::chat::ChatError;
use catcoms_rt::{
    NativeChatKeyEnds, NativeChatKeyPurpose, NativeChatPreparedKeyCut, NativeChatSourceObservation,
};
use catcoms_wire::Encoder;
mod first_add_proof;
mod pending_source;
pub use pending_source::{
    RecoveredV2JoinedOutcome, RecoveredV2PendingJoinSource, SealedV2JoinedOutcome,
    SealedV2PendingJoinSource, V2PendingJoinContextData,
};
mod warm_runtime;
mod welcome;
pub use first_add_proof::{
    PreparedV2JoinedPrefix, V2AdmissionStepData, V2FirstAddProofData, V2FirstJoinDeliveryData,
    MAX_V2_FIRST_ADD_PROOF_BYTES, MAX_V2_FIRST_JOIN_DELIVERY_BYTES,
};
pub use warm_runtime::{ClosedV2JoinedNode, V2JoinedRuntimeInputs};
pub use welcome::{
    CompletedV2JoinCandidate, V2JoinResponseData, V2JoinResponsePacketData,
    MAX_V2_JOIN_RESPONSE_PACKET_BYTES,
};

const TAG: &[u8] = b"mewtual/chat/join-request-packet/v2\0";
const VERSION: u16 = 2;
pub const MAX_V2_JOIN_REQUEST_PACKET_BYTES: usize =
    4 + TAG.len() + 2 + 3 * 4 + 2 * MAX_JOIN_BINDING_BYTES + MAX_FOUNDER_ADD_KEY_PACKAGE_BYTES;

/// Cryptographically consistent request DATA. The received MLS package still
/// needs independent admission validation and the current inviter must be
/// authorized by the owning transaction.
pub struct V2JoinRequestPacketData<'a> {
    invite: SignedInviteBindingData,
    request: SignedJoinRequestBindingData,
    key_package: &'a [u8],
    packet: &'a [u8],
}

impl<'a> V2JoinRequestPacketData<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, SyncError> {
        let malformed = |_| SyncError::Malformed;
        if bytes.len() > MAX_V2_JOIN_REQUEST_PACKET_BYTES {
            return Err(SyncError::Malformed);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes().map_err(malformed)? != TAG || d.get_u16().map_err(malformed)? != VERSION {
            return Err(SyncError::Malformed);
        }
        let invite = d.get_bytes().map_err(malformed)?;
        let request = d.get_bytes().map_err(malformed)?;
        let key_package = d.get_bytes().map_err(malformed)?;
        d.finish().map_err(malformed)?;
        if key_package.is_empty() || key_package.len() > MAX_FOUNDER_ADD_KEY_PACKAGE_BYTES {
            return Err(SyncError::Malformed);
        }
        let invite = SignedInviteBindingData::decode(invite).map_err(|_| SyncError::Malformed)?;
        let request =
            SignedJoinRequestBindingData::decode(request).map_err(|_| SyncError::Malformed)?;
        request
            .check_invite_links_data(&invite)
            .map_err(|_| SyncError::JoinRejected)?;
        request
            .check_key_package_wire_data(key_package)
            .map_err(|_| SyncError::JoinRejected)?;
        Ok(Self {
            invite,
            request,
            key_package,
            packet: bytes,
        })
    }

    /// The complete authenticated wire representation, including both signatures.
    pub fn packet_bytes_data(&self) -> &'a [u8] {
        self.packet
    }

    pub fn founder_add_data(&self) -> V2FounderAddRequestData<'_> {
        V2FounderAddRequestData {
            invite: &self.invite,
            request: &self.request,
            key_package: self.key_package,
        }
    }
}

impl fmt::Debug for V2JoinRequestPacketData<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("V2JoinRequestPacketData")
            .finish_non_exhaustive()
    }
}

/// Retains one exact signed request and the same private KeyPackage bundle.
/// Only fresh preparation can construct this owner. Neither a packet decoder
/// nor historical labels can construct or extract its private provider.
pub struct PreparedV2JoinRequest {
    request: Bytes,
    master: Zeroizing<Vec<u8>>,
    limits: DirectJoinLimits,
    welcome_entered: bool,
    native_origin: Option<NativeChatSourceObservation>,
    // A pending request moves through transport and covering futures even
    // before a completed SDK graph exists. Keep that graph out of those frames.
    completed: Option<Box<CompletedV2JoinCandidate>>,
    sealed_source: Option<SealedV2PendingJoinSource>,
    sealed_outcome: Option<SealedV2JoinedOutcome>,
    initial_prefix_origin: Option<std::sync::Arc<()>>,
}

/// Explicit owning native cut and its externally retained End batch. The
/// cut itself enforces original source allocation, purpose and UI session.
pub struct NativeV2JoinKeyEntry<'a> {
    original: &'a NativeChatSourceObservation,
    cut: &'a mut NativeChatPreparedKeyCut,
    ends: &'a mut NativeChatKeyEnds,
}

impl<'a> NativeV2JoinKeyEntry<'a> {
    pub fn new(
        original: &'a NativeChatSourceObservation,
        cut: &'a mut NativeChatPreparedKeyCut,
        ends: &'a mut NativeChatKeyEnds,
    ) -> Self {
        Self {
            original,
            cut,
            ends,
        }
    }

    fn enter(&mut self, purpose: NativeChatKeyPurpose) -> Result<(), SyncError> {
        self.cut
            .enter(self.original, purpose, self.ends)
            .map_err(|_| SyncError::JoinRejected)
    }
}

impl fmt::Debug for NativeV2JoinKeyEntry<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeV2JoinKeyEntry")
            .finish_non_exhaustive()
    }
}

impl PreparedV2JoinRequest {
    /// The owning caller supplies the explicitly accepted anchor, current
    /// injected time and provider/work budget. This primitive has no authority
    /// to choose a root, reserve native work, publish a request or admit a peer.
    pub fn prepare(
        device: MlsDevice,
        accepted_anchor: &GenesisAnchorBindingData,
        invite: &SignedInviteBindingData,
        token: [u8; 32],
        now_ms: u64,
        limits: DirectJoinLimits,
    ) -> Result<Self, SyncError> {
        require_first_invite(accepted_anchor, invite, token, now_ms)?;
        let spec = invite.spec_data();
        // Verify existing provider capacity before building a KeyPackage.
        // The caller still accounts for the SDK's typed heap and work.
        drop(snapshot_join_device_bounded(
            &device,
            limits.provider,
            limits.backing,
        )?);
        let package = device.key_package_for_invite(&accepted_anchor.group, spec.nonce)?;
        let key_package = catcoms_mls::device::serialize_key_package_bounded(
            &package,
            MAX_FOUNDER_ADD_KEY_PACKAGE_BYTES,
        )?;
        let identity = device.key_package_identity(&package)?;
        let request = SignedJoinRequestBindingData::sign_using(
            JoinRequestBindingSpecData {
                invite: invite.id_data(),
                anchor: accepted_anchor
                    .commitment_data()
                    .map_err(|_| SyncError::Malformed)?,
                joiner: *identity.subject(),
                request_token: token,
                key_package_wire: JoinPayloadKindData::KeyPackageWire
                    .commitment_data(&key_package)
                    .map_err(|_| SyncError::Malformed)?,
                key_package_reference: *identity.key_package_reference(),
                credential: *identity.credential_digest(),
            },
            |bytes| {
                device
                    .sign_retained_ed25519_bounded(
                        identity.subject(),
                        bytes,
                        MAX_JOIN_BINDING_BYTES,
                    )
                    .map_err(|_| ChatError::Signature)
            },
        )
        .map_err(|_| SyncError::JoinRejected)?;
        let request = encode(
            &invite.encode(),
            &request.encode(),
            &key_package,
            limits.exchange,
        )?;
        let master = snapshot_join_device_bounded(&device, limits.provider, limits.backing)?;
        Ok(Self {
            request: Bytes::from(request),
            master,
            limits,
            welcome_entered: false,
            native_origin: None,
            completed: None,
            sealed_source: None,
            sealed_outcome: None,
            initial_prefix_origin: None,
        })
    }

    /// Admit device generation and KeyPackage signing in the same original
    /// native cut. Later Welcome work requires that same source's SDK cut.
    pub fn prepare_native(
        accepted_anchor: &GenesisAnchorBindingData,
        invite: &SignedInviteBindingData,
        token: [u8; 32],
        now_ms: u64,
        limits: DirectJoinLimits,
        mut key: NativeV2JoinKeyEntry<'_>,
    ) -> Result<Self, SyncError> {
        require_first_invite(accepted_anchor, invite, token, now_ms)?;
        key.enter(NativeChatKeyPurpose::JoinerDevice)?;
        let device = MlsDevice::generate()?;
        let mut request = Self::prepare(device, accepted_anchor, invite, token, now_ms, limits)?;
        request.native_origin = Some(key.original.clone());
        Ok(request)
    }

    /// Lost requests can resend these same bytes without building a new package.
    pub fn request_bytes_data(&self) -> &[u8] {
        &self.request
    }

    /// Inspect the original private bundle with a bounded restore. This is a
    /// historical association check, independent of current package lifetime,
    /// and does not produce a device or renew the original invitation.
    pub fn validate_retained_package_data(&self) -> Result<(), SyncError> {
        let packet = V2JoinRequestPacketData::decode(&self.request)?;
        self.restore_exact_device(&packet)?;
        Ok(())
    }

    fn restore_exact_device(
        &self,
        packet: &V2JoinRequestPacketData<'_>,
    ) -> Result<MlsDevice, SyncError> {
        let device = restore_join_device_provider_bounded(
            &self.master,
            self.limits.provider,
            self.limits.backing,
        )?;
        let request = packet.request.spec_data();
        if !device.retained_ed25519_key_matches(&request.joiner) {
            return Err(SyncError::JoinRejected);
        }
        device.validate_retained_join_package(
            packet.key_package,
            &request.key_package_reference,
            &packet.invite.spec_data().anchor.group,
            &packet.invite.spec_data().nonce,
        )?;
        Ok(device)
    }
}

fn require_first_invite(
    accepted_anchor: &GenesisAnchorBindingData,
    invite: &SignedInviteBindingData,
    token: [u8; 32],
    now_ms: u64,
) -> Result<(), SyncError> {
    let spec = invite.spec_data();
    if token == [0; 32]
        || !invite.verify()
        || spec.anchor != *accepted_anchor
        || spec.inviter != accepted_anchor.founder
        || invite.expired_at_data(now_ms)
    {
        return Err(SyncError::JoinRejected);
    }
    Ok(())
}

impl fmt::Debug for PreparedV2JoinRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedV2JoinRequest")
            .finish_non_exhaustive()
    }
}

fn encode(
    invite: &[u8],
    request: &[u8],
    key_package: &[u8],
    cap: usize,
) -> Result<Vec<u8>, SyncError> {
    if invite.len() > MAX_JOIN_BINDING_BYTES
        || request.len() > MAX_JOIN_BINDING_BYTES
        || key_package.is_empty()
        || key_package.len() > MAX_FOUNDER_ADD_KEY_PACKAGE_BYTES
    {
        return Err(SyncError::Malformed);
    }
    let length = 4 + TAG.len() + 2 + 3 * 4 + invite.len() + request.len() + key_package.len();
    if length > cap {
        return Err(SyncError::Malformed);
    }
    let malformed = |_| SyncError::Malformed;
    let mut e = Encoder::with_capacity(length);
    e.put_bytes(TAG).map_err(malformed)?;
    e.put_u16(VERSION);
    e.put_bytes(invite).map_err(malformed)?;
    e.put_bytes(request).map_err(malformed)?;
    e.put_bytes(key_package).map_err(malformed)?;
    Ok(e.finish())
}

#[cfg(test)]
mod tests;
