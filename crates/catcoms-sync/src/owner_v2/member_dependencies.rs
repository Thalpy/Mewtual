//! One exact dependency request under the current group channel exporter.
//! The connected route is bound into the request. Neither a request nor a
//! response grants a current view, historical acceptance, or global absence.
use super::member_envelope::member_exporter_key;
use super::{
    open_v2_member_envelope_data, seal_v2_member_envelope_data, OpenedV2MemberEnvelopeData,
    SelectedV2MemberEnvelope, V2MemberEnvelopeScopeData, V2MemberPublicationObservation,
    MAX_V2_MEMBER_TRANSPORT_BYTES,
};
use crate::SyncError;
use catcoms_crypto::{seal, unseal, SealedBlob};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_replication::chat::{
    owner_facts::OwnerFactId, ActionId, SignedAction, V2AuthenticatedAdmission,
};
use catcoms_rt::{CryptoRngCore, PeerId};
use catcoms_wire::{Decoder, Encoder, WireError};
use zeroize::Zeroizing;

const REQUEST_TAG: &[u8] = b"mewtual/chat/member-dependency-request/v2\0";
const REPLY_TAG: &[u8] = b"mewtual/chat/member-dependency-reply/v2\0";
const REQUEST_KEY: &str = "mewtual/chat/member-dependency-request-key/v2";
const REPLY_KEY: &str = "mewtual/chat/member-dependency-reply-key/v2";
const EXACT_REQUEST: &str = "mewtual/chat/member-dependency-request-id/v2";
const VERSION: u16 = 1;
const OVERHEAD: usize = 4 + 2 + 4 + 24 + 4 + 16;
const REQUEST_BODY: usize = 3 * 36;
pub const MAX_V2_DEPENDENCY_REQUEST_BYTES: usize = REQUEST_TAG.len() + OVERHEAD + REQUEST_BODY;
pub const MAX_V2_DEPENDENCY_REPLY_BYTES: usize =
    REPLY_TAG.len() + OVERHEAD + 4 + MAX_V2_MEMBER_TRANSPORT_BYTES;
const _: () = assert!(
    1 + MAX_V2_DEPENDENCY_REQUEST_BYTES <= catcoms_rt::CHAT_V2_DEPENDENCY_REQUEST_MAX_BYTES
);
const _: () =
    assert!(MAX_V2_DEPENDENCY_REPLY_BYTES <= catcoms_rt::CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES);

/// The ordered, already authenticated connection endpoints expected by the
/// current source. A peer ID here is routing data, not a member signer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct V2MemberDependencyRoute {
    pub requester: PeerId,
    pub provider: PeerId,
}

/// A borrow of the original founder's selected MLS graph and Core Admission.
/// The host still owns the connected request and selected outbox receipt.
pub struct V2FounderMemberDependencyAccess<'a> {
    pub(super) group: &'a ServerGroup,
    pub(super) device: &'a MlsDevice,
    pub(super) scope: V2MemberEnvelopeScopeData,
    pub(super) member: &'a V2AuthenticatedAdmission,
    pub(super) local: PeerId,
    pub(super) observation: V2MemberPublicationObservation,
}

impl V2FounderMemberDependencyAccess<'_> {
    pub fn prepare_request(
        &self,
        original: &SelectedV2MemberEnvelope,
        route: V2MemberDependencyRoute,
        action: ActionId,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedV2MemberDependencyRequest, SyncError> {
        let spec = original.action_data().spec();
        if route.requester != self.local
            || !self.observation.is_current_cached()
            || !original.is_current_cached()
            || !original.recipient.same_original(self.member)
            || !original.opened.matches_scope_data(&self.scope)
            || (spec.parents.binary_search(&action).is_err() && spec.body.create() != Some(action))
        {
            return Err(SyncError::JoinRejected);
        }
        PreparedV2MemberDependencyRequest::prepare(
            self.group,
            self.device,
            self.scope,
            route,
            action,
            rng,
        )
    }

    pub fn open_reply(
        self,
        route: V2MemberDependencyRoute,
        request: &PreparedV2MemberDependencyRequest,
        wire: &[u8],
    ) -> Result<Option<SelectedV2MemberEnvelope>, SyncError> {
        if route.requester != self.local || !self.observation.is_current_cached() {
            return Err(SyncError::JoinRejected);
        }
        let opened = request.open_reply(self.group, self.device, &self.scope, route, wire)?;
        if !self.observation.is_current_cached() {
            return Err(SyncError::JoinRejected);
        }
        Ok(opened.map(|opened| SelectedV2MemberEnvelope {
            opened,
            recipient: self.member.clone(),
            recipient_id: self.member.admission_data().admission_id(),
            observation: self.observation,
        }))
    }

    pub fn route_data(&self, requester: PeerId) -> V2MemberDependencyRoute {
        V2MemberDependencyRoute {
            requester,
            provider: self.local,
        }
    }

    pub fn open_request(
        &self,
        route: V2MemberDependencyRoute,
        wire: &[u8],
    ) -> Result<OpenedV2MemberDependencyRequest, SyncError> {
        if route.provider != self.local || !self.observation.is_current_cached() {
            return Err(SyncError::JoinRejected);
        }
        let request = OpenedV2MemberDependencyRequest::open(
            self.group,
            self.device,
            self.scope,
            route,
            wire,
        )?;
        if !self.observation.is_current_cached() {
            return Err(SyncError::JoinRejected);
        }
        Ok(request)
    }

    pub fn seal_reply(
        &self,
        route: V2MemberDependencyRoute,
        request: &OpenedV2MemberDependencyRequest,
        saved: Option<(OwnerFactId, &[u8])>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, SyncError> {
        if route.provider != self.local
            || !self.observation.is_current_cached()
            || saved.is_some_and(|(admission, _)| {
                admission != self.member.admission_data().admission_id()
            })
        {
            return Err(SyncError::JoinRejected);
        }
        let reply = request.seal_reply(self.group, self.device, &self.scope, route, saved, rng)?;
        if !self.observation.is_current_cached() {
            return Err(SyncError::JoinRejected);
        }
        Ok(reply)
    }

    pub fn into_observation(self) -> V2MemberPublicationObservation {
        self.observation
    }
}

struct RequestBinding {
    scope: V2MemberEnvelopeScopeData,
    route: V2MemberDependencyRoute,
    action: ActionId,
    exact: [u8; 32],
}

/// A retry borrows the exact ciphertext originally prepared for this route.
pub struct PreparedV2MemberDependencyRequest {
    binding: RequestBinding,
    wire: Vec<u8>,
}

impl PreparedV2MemberDependencyRequest {
    pub fn prepare(
        group: &ServerGroup,
        device: &MlsDevice,
        scope: V2MemberEnvelopeScopeData,
        route: V2MemberDependencyRoute,
        action: ActionId,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, SyncError> {
        require_route(route)?;
        let key = member_exporter_key(group, device, &scope, REQUEST_KEY)?;
        let mut body = Encoder::with_capacity(REQUEST_BODY);
        data(body.put_bytes(route.requester.as_bytes()))?;
        data(body.put_bytes(route.provider.as_bytes()))?;
        data(body.put_bytes(action.as_bytes()))?;
        let plain = Zeroizing::new(body.finish());
        let wire = seal_frame(&key, REQUEST_TAG, &plain, rng)?;
        Ok(Self {
            binding: RequestBinding {
                scope,
                route,
                action,
                exact: request_id(&wire),
            },
            wire,
        })
    }

    pub fn wire_data(&self) -> &[u8] {
        &self.wire
    }

    /// Authenticate receipt under the same actual selected scope and original
    /// connection endpoints. Empty means this provider returned no original.
    pub fn open_reply(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        current: &V2MemberEnvelopeScopeData,
        route: V2MemberDependencyRoute,
        wire: &[u8],
    ) -> Result<Option<OpenedV2MemberEnvelopeData>, SyncError> {
        self.binding.require_selected(current, route)?;
        let sealed = parse_frame(wire, REPLY_TAG, MAX_V2_DEPENDENCY_REPLY_BYTES)?;
        let key = self.binding.reply_key(group, device)?;
        let plain = open_frame(&key, sealed)?;
        let mut body = Decoder::new(&plain);
        let envelope = data(body.get_bytes())?;
        data(body.finish())?;
        if envelope.is_empty() {
            return Ok(None);
        }
        let opened = open_v2_member_envelope_data(group, device, current, envelope)?;
        if opened.action_data().id() != self.binding.action {
            return Err(SyncError::Malformed);
        }
        Ok(Some(opened))
    }
}

/// An authenticated request from the bound current group channel and route.
/// The serving source must reprove its own original state before looking up
/// the requested action. Merely opening these bytes permits no disk access.
pub struct OpenedV2MemberDependencyRequest(RequestBinding);

impl OpenedV2MemberDependencyRequest {
    pub fn open(
        group: &ServerGroup,
        device: &MlsDevice,
        scope: V2MemberEnvelopeScopeData,
        route: V2MemberDependencyRoute,
        wire: &[u8],
    ) -> Result<Self, SyncError> {
        require_route(route)?;
        let sealed = parse_frame(wire, REQUEST_TAG, MAX_V2_DEPENDENCY_REQUEST_BYTES)?;
        let key = member_exporter_key(group, device, &scope, REQUEST_KEY)?;
        let plain = open_frame(&key, sealed)?;
        if plain.len() != REQUEST_BODY {
            return Err(SyncError::Malformed);
        }
        let mut body = Decoder::new(&plain);
        if data(body.get_bytes())? != route.requester.as_bytes()
            || data(body.get_bytes())? != route.provider.as_bytes()
        {
            return Err(SyncError::Malformed);
        }
        let action = ActionId::from_bytes(
            data(body.get_bytes())?
                .try_into()
                .map_err(|_| SyncError::Malformed)?,
        );
        data(body.finish())?;
        Ok(Self(RequestBinding {
            scope,
            route,
            action,
            exact: request_id(wire),
        }))
    }

    pub fn action_id_data(&self) -> ActionId {
        self.0.action
    }

    /// Build a bounded answer for this exact request. The caller supplies the
    /// already selected saved original and its actual current Admission.
    pub fn seal_reply(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        current: &V2MemberEnvelopeScopeData,
        route: V2MemberDependencyRoute,
        saved: Option<(OwnerFactId, &[u8])>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, SyncError> {
        self.0.require_selected(current, route)?;
        let key = self.0.reply_key(group, device)?;
        let envelope = if let Some((admission, original)) = saved {
            let action = SignedAction::decode(original).map_err(|_| SyncError::Malformed)?;
            if action.id() != self.0.action {
                return Err(SyncError::Malformed);
            }
            seal_v2_member_envelope_data(group, device, current, admission, original, rng)?
        } else {
            Vec::new()
        };
        let mut body = Encoder::with_capacity(4 + envelope.len());
        data(body.put_bytes(&envelope))?;
        seal_frame(&key, REPLY_TAG, &Zeroizing::new(body.finish()), rng)
    }
}

impl RequestBinding {
    fn require_selected(
        &self,
        scope: &V2MemberEnvelopeScopeData,
        route: V2MemberDependencyRoute,
    ) -> Result<(), SyncError> {
        if self.scope != *scope || self.route != route {
            return Err(SyncError::Malformed);
        }
        Ok(())
    }

    fn reply_key(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
    ) -> Result<Zeroizing<[u8; 32]>, SyncError> {
        let base = member_exporter_key(group, device, &self.scope, REPLY_KEY)?;
        Ok(Zeroizing::new(
            *blake3::keyed_hash(&base, &self.exact).as_bytes(),
        ))
    }
}

fn require_route(route: V2MemberDependencyRoute) -> Result<(), SyncError> {
    if route.requester == route.provider
        || route.requester.as_bytes() == &[0; 32]
        || route.provider.as_bytes() == &[0; 32]
    {
        return Err(SyncError::Malformed);
    }
    Ok(())
}

fn request_id(wire: &[u8]) -> [u8; 32] {
    blake3::derive_key(EXACT_REQUEST, wire)
}

fn seal_frame(
    key: &[u8; 32],
    tag: &[u8],
    plain: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<Vec<u8>, SyncError> {
    let sealed = seal(key, plain, rng).map_err(|_| SyncError::Malformed)?;
    let mut out = Encoder::with_capacity(tag.len() + OVERHEAD + plain.len());
    data(out.put_bytes(tag))?.put_u16(VERSION);
    data(out.put_bytes(&sealed.nonce))?;
    data(out.put_bytes(&sealed.ciphertext))?;
    Ok(out.finish())
}

fn parse_frame<'a>(
    wire: &'a [u8],
    tag: &[u8],
    maximum: usize,
) -> Result<([u8; 24], &'a [u8]), SyncError> {
    if wire.len() > maximum {
        return Err(SyncError::Malformed);
    }
    let mut input = Decoder::new(wire);
    if data(input.get_bytes())? != tag || data(input.get_u16())? != VERSION {
        return Err(SyncError::Malformed);
    }
    let nonce = data(input.get_bytes())?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let encrypted = data(input.get_bytes())?;
    data(input.finish())?;
    if encrypted.len() < 16 {
        return Err(SyncError::Malformed);
    }
    Ok((nonce, encrypted))
}

fn open_frame(
    key: &[u8; 32],
    (nonce, encrypted): ([u8; 24], &[u8]),
) -> Result<Zeroizing<Vec<u8>>, SyncError> {
    let sealed = SealedBlob {
        nonce,
        ciphertext: encrypted.to_vec(),
    };
    Ok(Zeroizing::new(
        unseal(key, &sealed).map_err(|_| SyncError::Malformed)?,
    ))
}

fn data<T>(value: Result<T, WireError>) -> Result<T, SyncError> {
    value.map_err(|_| SyncError::Malformed)
}
