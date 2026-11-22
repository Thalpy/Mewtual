//! Epoch-protected transport bytes for an exact ordinary signed original.
use crate::SyncError;
use catcoms_crypto::{seal, unseal, SealedBlob};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_replication::chat::{
    owner_facts::OwnerFactId, owner_header::OwnerHeaderId, ProjectionAdmissionToken,
    ProjectionView, SignedAction, V2GeneralPrefixVerifier, V2OrdinaryDependency, MAX_ACTION_BYTES,
};
use catcoms_rt::{CryptoRngCore, Topic};
use catcoms_storage::pad::{self, OP_PAD_CEILING, OP_PAD_FLOOR, PAD_FOOTER_BYTES};
use catcoms_wire::{Decoder, DocType, Encoder};
use zeroize::Zeroizing;

const SEALED_TAG: &[u8] = b"mewtual/chat/member-transport/v2\0";
const ORIGINAL_TAG: &[u8] = b"mewtual/chat/member-transport-original/v2\0";
const KEY: &str = "mewtual/chat/member-transport-key/v2";
const TOPIC: &str = "mewtual/chat/member-transport-topic/v2";
const VERSION: u16 = 2;
const MAX_PLAIN: usize = 4 + ORIGINAL_TAG.len() + 2 + 4 + 32 + 4 + MAX_ACTION_BYTES;
const MAX_CIPHER: usize = MAX_PLAIN.next_power_of_two() + PAD_FOOTER_BYTES + 16;
const SEALED_PREFIX: usize = 4 + SEALED_TAG.len() + 2 + 4 + 24 + 4;
pub const MAX_V2_MEMBER_TRANSPORT_BYTES: usize = SEALED_PREFIX + MAX_CIPHER;
const _: () = assert!(MAX_PLAIN < OP_PAD_CEILING);

fn wire_data<T>(value: Result<T, catcoms_wire::WireError>) -> Result<T, SyncError> {
    value.map_err(|_| SyncError::Malformed)
}

/// The mounted receiver supplies these independently selected values. No
/// encrypted frame supplies a group, route, governing selection or epoch.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct V2MemberEnvelopeScopeData {
    pub group: [u8; 16],
    pub profile: OwnerFactId,
    pub governing: OwnerHeaderId,
    pub epoch: u64,
    pub channel: u128,
}

impl V2MemberEnvelopeScopeData {
    pub fn for_selected_prefix(
        verifier: &V2GeneralPrefixVerifier,
        channel: u128,
    ) -> Result<Self, SyncError> {
        let current = verifier.replay_data();
        Ok(Self {
            group: current
                .profile()
                .spec()
                .group
                .as_slice()
                .try_into()
                .map_err(|_| SyncError::Malformed)?,
            profile: current.profile().id(),
            governing: current.header().id(),
            epoch: current.mls_epoch_data(),
            channel,
        })
    }
}

/// An exact authenticated original and its claimed Admission. The receiving
/// member still needs Core verification against its own selected prefix.
pub struct OpenedV2MemberEnvelopeData {
    scope: V2MemberEnvelopeScopeData,
    admission: OwnerFactId,
    action: SignedAction,
    original: Zeroizing<Vec<u8>>,
}

impl OpenedV2MemberEnvelopeData {
    pub fn admission_data(&self) -> OwnerFactId {
        self.admission
    }

    pub fn action_data(&self) -> &SignedAction {
        &self.action
    }

    pub fn original_data(&self) -> &[u8] {
        &self.original
    }

    /// Authorize the exact ordinary action against the receiver's current
    /// verified Core prefix, actual Admission and minimal dependencies.
    pub fn provisional_authority(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        dependencies: &[V2OrdinaryDependency<'_>],
        view: &ProjectionView,
    ) -> Result<ProjectionAdmissionToken, SyncError> {
        if self.scope
            != V2MemberEnvelopeScopeData::for_selected_prefix(verifier, self.scope.channel)?
        {
            return Err(SyncError::Malformed);
        }
        let admission = verifier
            .admission(self.admission)
            .map_err(|_| SyncError::Malformed)?;
        verifier
            .provisional_action(&self.action, &admission, dependencies, view)
            .map_err(|_| SyncError::Malformed)
    }
}

fn require_group(
    group: &ServerGroup,
    device: &MlsDevice,
    scope: &V2MemberEnvelopeScopeData,
) -> Result<(), SyncError> {
    if !group.is_active()
        || group.epoch() != scope.epoch
        || group.group_id().as_slice() != scope.group.as_slice()
        || !group.has_local_signer(&device.public_key_bytes())
        || scope.profile.as_bytes() == &[0; 32]
        || scope.governing.as_bytes() == &[0; 32]
    {
        return Err(SyncError::Malformed);
    }
    Ok(())
}

fn domain_hash(
    domain: &'static str,
    base: &[u8; 32],
    scope: &V2MemberEnvelopeScopeData,
) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key(domain);
    hash.update(base);
    hash.update(&scope.group);
    hash.update(scope.profile.as_bytes());
    hash.update(scope.governing.as_bytes());
    hash.update(&scope.epoch.to_be_bytes());
    hash.update(&scope.channel.to_be_bytes());
    *hash.finalize().as_bytes()
}

fn key(
    group: &ServerGroup,
    device: &MlsDevice,
    scope: &V2MemberEnvelopeScopeData,
) -> Result<Zeroizing<[u8; 32]>, SyncError> {
    require_group(group, device, scope)?;
    // This MLS exporter does not advance an application sender ratchet.
    let base = Zeroizing::new(group.channel_secret(device, DocType::Channel, scope.channel)?);
    Ok(Zeroizing::new(domain_hash(KEY, &base, scope)))
}

/// Blinded topic from the distinct MLS metadata exporter. A topic value is
/// routing DATA and grants no payload decryption or publication admission.
pub fn v2_member_envelope_topic_data(
    group: &ServerGroup,
    device: &MlsDevice,
    scope: &V2MemberEnvelopeScopeData,
) -> Result<Topic, SyncError> {
    require_group(group, device, scope)?;
    let base = Zeroizing::new(group.metadata_secret(device, DocType::Channel, scope.channel)?);
    Ok(Topic::new(domain_hash(TOPIC, &base, scope).to_vec()))
}

fn checked_action(
    original: &[u8],
    scope: &V2MemberEnvelopeScopeData,
    admission: OwnerFactId,
) -> Result<SignedAction, SyncError> {
    if admission.as_bytes() == &[0; 32] {
        return Err(SyncError::Malformed);
    }
    let action = SignedAction::decode(original).map_err(|_| SyncError::Malformed)?;
    if action.spec().realm.group() != scope.group.as_slice()
        || action.spec().realm.channel() != scope.channel
        || !action.verify()
    {
        return Err(SyncError::Malformed);
    }
    Ok(action)
}

/// Seal the exact bytes of this device's signed original. A host must obtain
/// it from the current admitted outbox debt before it can schedule submission.
pub fn seal_v2_member_envelope_data(
    group: &ServerGroup,
    device: &MlsDevice,
    scope: &V2MemberEnvelopeScopeData,
    admission: OwnerFactId,
    original: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<Vec<u8>, SyncError> {
    let action = checked_action(original, scope, admission)?;
    if action.signer().as_slice() != device.public_key_bytes().as_slice() {
        return Err(SyncError::Malformed);
    }
    let key = key(group, device, scope)?;
    let mut plain = Encoder::with_capacity(MAX_PLAIN - MAX_ACTION_BYTES + original.len());
    wire_data(plain.put_bytes(ORIGINAL_TAG))?.put_u16(VERSION);
    wire_data(plain.put_bytes(admission.as_bytes()))?;
    wire_data(plain.put_bytes(original))?;
    let plain = Zeroizing::new(plain.finish());
    let padded = Zeroizing::new(pad::pad(&plain, OP_PAD_FLOOR, OP_PAD_CEILING)?);
    let sealed = seal(&key, &padded, rng).map_err(|_| SyncError::Malformed)?;
    let mut wire = Encoder::with_capacity(SEALED_PREFIX + sealed.ciphertext.len());
    wire_data(wire.put_bytes(SEALED_TAG))?.put_u16(VERSION);
    wire_data(wire.put_bytes(&sealed.nonce))?;
    wire_data(wire.put_bytes(&sealed.ciphertext))?;
    Ok(wire.finish())
}

/// Authenticate and decode only under the receiver's already selected scope.
/// The wire limit is checked before copying ciphertext or entering the SDK.
pub fn open_v2_member_envelope_data(
    group: &ServerGroup,
    device: &MlsDevice,
    scope: &V2MemberEnvelopeScopeData,
    wire: &[u8],
) -> Result<OpenedV2MemberEnvelopeData, SyncError> {
    if wire.len() > MAX_V2_MEMBER_TRANSPORT_BYTES {
        return Err(SyncError::Malformed);
    }
    let mut input = Decoder::new(wire);
    if wire_data(input.get_bytes())? != SEALED_TAG || wire_data(input.get_u16())? != VERSION {
        return Err(SyncError::Malformed);
    }
    let nonce = wire_data(input.get_bytes())?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let encrypted = wire_data(input.get_bytes())?;
    wire_data(input.finish())?;
    let padded = encrypted
        .len()
        .checked_sub(PAD_FOOTER_BYTES + 16)
        .ok_or(SyncError::Malformed)?;
    if padded < OP_PAD_FLOOR || !padded.is_power_of_two() || encrypted.len() > MAX_CIPHER {
        return Err(SyncError::Malformed);
    }
    let key = key(group, device, scope)?;
    let sealed = SealedBlob {
        nonce,
        ciphertext: encrypted.to_vec(),
    };
    let plain = Zeroizing::new(unseal(&key, &sealed).map_err(|_| SyncError::Malformed)?);
    let body = pad::unpad(&plain, OP_PAD_FLOOR, OP_PAD_CEILING)?;
    if body.len() > MAX_PLAIN {
        return Err(SyncError::Malformed);
    }
    let mut input = Decoder::new(body);
    if wire_data(input.get_bytes())? != ORIGINAL_TAG || wire_data(input.get_u16())? != VERSION {
        return Err(SyncError::Malformed);
    }
    let admission = OwnerFactId::from_bytes(
        wire_data(input.get_bytes())?
            .try_into()
            .map_err(|_| SyncError::Malformed)?,
    );
    let original = wire_data(input.get_bytes())?;
    wire_data(input.finish())?;
    let action = checked_action(original, scope, admission)?;
    Ok(OpenedV2MemberEnvelopeData {
        scope: *scope,
        admission,
        action,
        original: Zeroizing::new(original.to_vec()),
    })
}

#[cfg(test)]
mod tests;
