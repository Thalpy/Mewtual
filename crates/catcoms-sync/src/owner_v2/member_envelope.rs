//! Epoch-protected transport bytes for an exact ordinary signed original.
catcoms_rt::redacted_debug! {
    PreparedV2MemberSubmission;
    V2MemberPublicationObservation;
    V2MemberEnvelopeScopeData;
    OpenedV2MemberEnvelopeData;
    SelectedV2MemberEnvelope;
}

use crate::owner_v2::V2JoinedPublicationFence;
use crate::SyncError;
use catcoms_crypto::{seal, unseal, SealedBlob};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_replication::chat::{
    owner_facts::OwnerFactId, owner_header::OwnerHeaderId, ActionState, ChatError, ChatLedger,
    ProjectionAdmissionToken, ProjectionView, SignedAction, V2AuthenticatedAdmission,
    V2GeneralPrefixVerifier, V2LiveMemberObservation, V2OrdinaryDependency,
    V2OrdinaryReceiveStatus, MAX_ACTION_BYTES,
};
use catcoms_rt::{
    AuthorizedMeshPublication, Clock, CryptoRngCore, PublishSubmission, RequestAuthority,
    RequestCancellation, SystemClock, Topic,
};
use catcoms_storage::pad::{self, OP_PAD_CEILING, OP_PAD_FLOOR, PAD_FOOTER_BYTES};
use catcoms_wire::{Decoder, DocType, Encoder};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
    time::Duration,
};
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
const _: () = assert!(MAX_V2_MEMBER_TRANSPORT_BYTES <= catcoms_rt::MAX_PUBLISH_ONCE_BYTES);

/// One ownership-scoped attempt, including its original mesh and sealed bytes.
/// Only the driver's `Submitted` result can later settle local outbox debt.
#[must_use = "an envelope prepared in memory has not been submitted"]
pub struct PreparedV2MemberSubmission {
    pub(crate) transport: Box<dyn AuthorizedMeshPublication>,
    pub(crate) topic: Topic,
    pub(crate) wire: bytes::Bytes,
    pub(crate) observation: V2MemberPublicationObservation,
}

/// The exact current Core lifetime and finalized MLS selection. This negative
/// observation carries neither the driver nor the original SDK graph.
pub struct V2MemberPublicationObservation {
    pub(crate) member: V2LiveMemberObservation,
    pub(crate) fence: MemberTransportFence,
}

pub(crate) enum MemberTransportFence {
    Joined(V2JoinedPublicationFence),
    Founder {
        seed: Weak<()>,
        scope: RequestCancellation,
    },
}

impl V2MemberPublicationObservation {
    pub fn is_current_cached(&self) -> bool {
        self.member.is_live_cached()
            && match &self.fence {
                MemberTransportFence::Joined(fence) => fence.is_current_cached(),
                MemberTransportFence::Founder { seed, scope } => {
                    seed.strong_count() != 0 && !scope.is_cancelled()
                }
            }
    }

    /// Retire the exact mesh operation with the native caller, Core group or
    /// selected MLS fence. Preserve the original caller and its paid custody.
    pub fn constrain(self, caller: RequestCancellation) -> RequestCancellation {
        let authority = Arc::new(MemberTransportAuthority {
            caller: caller.clone(),
            observation: self,
            retired: AtomicBool::new(false),
        });
        caller.with_authority(authority)
    }
}

impl PreparedV2MemberSubmission {
    pub fn is_current_cached(&self) -> bool {
        self.observation.is_current_cached()
    }

    /// Retain the caller's original lifetime and payment through queue custody.
    /// The synchronous driver edge also checks the original MLS and Core state.
    pub async fn submit(self, caller: RequestCancellation) -> Result<PublishSubmission, SyncError> {
        let Self {
            transport,
            topic,
            wire,
            observation,
        } = self;
        let checked = observation.constrain(caller);
        if checked.is_cancelled() {
            return Err(catcoms_rt::PublishOnceError::Revoked.into());
        }
        Ok(transport.submit(topic, wire, checked).await?)
    }
}

struct MemberTransportAuthority {
    caller: RequestCancellation,
    observation: V2MemberPublicationObservation,
    retired: AtomicBool,
}

impl std::fmt::Debug for MemberTransportAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberTransportAuthority")
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl RequestAuthority for MemberTransportAuthority {
    fn authorized(&self) -> bool {
        if self.retired.load(Ordering::Acquire) {
            return false;
        }
        if self.caller.is_cancelled() || !self.observation.is_current_cached() {
            self.retired.store(true, Ordering::Release);
            return false;
        }
        !self.retired.load(Ordering::Acquire)
    }

    async fn revoked(&self) {
        let mut caller = self.caller.clone();
        while self.authorized() {
            tokio::select! {
                _ = caller.cancelled() => {
                    self.retired.store(true, Ordering::Release);
                    return;
                },
                _ = SystemClock.sleep(Duration::from_millis(25)) => {},
            }
        }
    }
}

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
    /// The caller authenticates the enclosing wire and binds this original
    /// to the expected purpose and request before parsing the signed bytes.
    pub(super) fn from_authenticated_original(
        scope: &V2MemberEnvelopeScopeData,
        admission: OwnerFactId,
        original: &[u8],
    ) -> Result<Self, SyncError> {
        Ok(Self {
            scope: *scope,
            admission,
            action: checked_action(original, scope, admission)?,
            original: Zeroizing::new(original.to_vec()),
        })
    }

    pub(super) fn matches_scope_data(&self, scope: &V2MemberEnvelopeScopeData) -> bool {
        &self.scope == scope
    }

    pub fn admission_data(&self) -> OwnerFactId {
        self.admission
    }

    pub fn action_data(&self) -> &SignedAction {
        &self.action
    }

    pub fn original_data(&self) -> &[u8] {
        &self.original
    }
}

/// An opened original tied to the actual receiver's registered MLS state and
/// selected Core Admission. Equal Profile, Header and key bytes cannot move
/// it into a different runtime's ledger.
pub struct SelectedV2MemberEnvelope {
    pub(crate) opened: OpenedV2MemberEnvelopeData,
    pub(crate) recipient: V2AuthenticatedAdmission,
    pub(crate) recipient_id: OwnerFactId,
    pub(crate) observation: V2MemberPublicationObservation,
}

impl SelectedV2MemberEnvelope {
    pub fn is_current_cached(&self) -> bool {
        self.observation.is_current_cached()
    }

    pub fn original_data(&self) -> &[u8] {
        self.opened.original_data()
    }

    pub fn action_data(&self) -> &SignedAction {
        self.opened.action_data()
    }

    pub fn admission_data(&self) -> OwnerFactId {
        self.opened.admission_data()
    }

    /// Authorize the exact ordinary action against the receiver's current
    /// verified Core prefix, actual Admission and minimal dependencies.
    pub fn provisional_authority(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        dependencies: &[V2OrdinaryDependency<'_>],
        view: &ProjectionView,
    ) -> Result<ProjectionAdmissionToken, SyncError> {
        let admission = self
            .original_admission(verifier)
            .map_err(|_| SyncError::Malformed)?;
        verifier
            .provisional_action(self.action_data(), &admission, dependencies, view)
            .map_err(|_| SyncError::Malformed)
    }

    /// Apply the exact opened action to the caller's paid ledger, under its
    /// actual selected channel view. Only a Valid result has a current
    /// provisional effect. The original bytes remain available to the host.
    pub fn project_provisional(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        dependencies: &[V2OrdinaryDependency<'_>],
        work: usize,
    ) -> Result<ActionState, ChatError> {
        let admission = self.original_admission(verifier)?;
        verifier.project_provisional_action(
            ledger,
            view,
            self.action_data(),
            &admission,
            dependencies,
            work,
        )
    }

    /// Resolve exactly the receiver's available validated parents and target.
    /// The caller may retain this same original for later dependency recovery.
    pub fn project_from_ledger(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        work: usize,
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        let admission = self.original_admission(verifier)?;
        verifier.project_provisional_from_ledger(ledger, view, self.action_data(), &admission, work)
    }

    /// Supply historical dependency knowledge while the receiver selects the
    /// original in its archive. No live projection is issued here.
    pub fn stage_from_ledger(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        ledger: &mut ChatLedger,
        view: &ProjectionView,
        work: usize,
    ) -> Result<V2OrdinaryReceiveStatus, ChatError> {
        let admission = self.original_admission(verifier)?;
        verifier.stage_provisional_from_ledger(ledger, view, self.action_data(), &admission, work)
    }

    fn original_admission(
        &self,
        verifier: &V2GeneralPrefixVerifier,
    ) -> Result<V2AuthenticatedAdmission, ChatError> {
        let scope = &self.opened.scope;
        if !self.is_current_cached()
            || !self
                .recipient
                .same_original(&verifier.admission(self.recipient_id)?)
            || *scope
                != V2MemberEnvelopeScopeData::for_selected_prefix(verifier, scope.channel)
                    .map_err(|_| ChatError::Scope)?
        {
            return Err(ChatError::Scope);
        }
        verifier.admission(self.admission_data())
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
    member_exporter_key(group, device, scope, KEY)
}

pub(super) fn member_exporter_key(
    group: &ServerGroup,
    device: &MlsDevice,
    scope: &V2MemberEnvelopeScopeData,
    domain: &'static str,
) -> Result<Zeroizing<[u8; 32]>, SyncError> {
    require_group(group, device, scope)?;
    // This MLS exporter does not advance an application sender ratchet.
    let base = Zeroizing::new(group.channel_secret(device, DocType::Channel, scope.channel)?);
    Ok(Zeroizing::new(domain_hash(domain, &base, scope)))
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

pub(super) fn checked_action(
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
    OpenedV2MemberEnvelopeData::from_authenticated_original(scope, admission, original)
}

#[cfg(test)]
mod tests;
