//! Passive discovery on an existing authenticated native connection. The
//! exact selected MLS exporter protects the signed contact proof. The proof
//! separately binds the Admission to the peer on that connection.
use super::*;
use crate::V2MemberContactData;
use catcoms_replication::chat::{V2GeneralPrefixVerifier, V2LiveMemberObservation};

const REQUEST: &[u8] = b"md-v1\0";
const REPLY: &[u8] = b"md-reply-v1\0";
const REQUEST_KEY: &str = "mewtual/chat/member-discovery-request-key/v1";
const REPLY_KEY: &str = "mewtual/chat/member-discovery-reply-key/v1";
const EXACT: &str = "mewtual/chat/member-discovery-exact-request/v1";
type Proof = [u8; V2MemberContactData::CONNECTED_PROOF_BYTES];
const REQUEST_BYTES: usize = REQUEST.len() + OVERHEAD + size_of::<Proof>();
const REPLY_BYTES: usize = REPLY.len() + OVERHEAD + size_of::<Proof>();
// One additional byte is the outer native dependency kind.
const _: () = assert!(REQUEST_BYTES < catcoms_rt::CHAT_V2_DEPENDENCY_REQUEST_MAX_BYTES);
const _: () = assert!(REPLY_BYTES <= 256);

catcoms_rt::redacted_debug! {
    DiscoveryEndpoint<'_>;
    PreparedV2MemberDiscovery;
    OpenedV2MemberDiscovery;
    DiscoveredV2Member;
}

/// A current Admission verified against one real transport peer. This is an
/// observation, so the host checks the original connection and row again
/// before installing ephemeral history eligibility.
pub struct DiscoveredV2Member {
    admission: OwnerFactId,
    observation: V2LiveMemberObservation,
    peer: PeerId,
}

impl DiscoveredV2Member {
    pub fn admission_data(&self) -> OwnerFactId {
        self.admission
    }

    pub fn peer_data(&self) -> PeerId {
        self.peer
    }

    pub fn is_live_cached(&self) -> bool {
        self.observation.is_live_cached()
    }
}

struct Binding {
    scope: V2MemberEnvelopeScopeData,
    route: V2MemberDependencyRoute,
    digest: [u8; 32],
}

pub struct PreparedV2MemberDiscovery {
    binding: Binding,
    wire: Vec<u8>,
}

impl PreparedV2MemberDiscovery {
    pub fn wire_data(&self) -> &[u8] {
        &self.wire
    }
}

pub struct OpenedV2MemberDiscovery {
    binding: Binding,
    requester: DiscoveredV2Member,
}

impl OpenedV2MemberDiscovery {
    pub fn recognizes(wire: &[u8]) -> bool {
        wire.len() == REQUEST_BYTES && Decoder::new(wire).get_bytes() == Ok(REQUEST)
    }

    pub fn requester_data(&self) -> &DiscoveredV2Member {
        &self.requester
    }
}

pub(crate) struct DiscoveryEndpoint<'a> {
    pub(crate) group: &'a ServerGroup,
    pub(crate) device: &'a MlsDevice,
    pub(crate) verifier: &'a V2GeneralPrefixVerifier,
    pub(crate) route: V2MemberDependencyRoute,
    pub(crate) local: OwnerFactId,
}

impl DiscoveryEndpoint<'_> {
    fn scope(&self) -> Result<V2MemberEnvelopeScopeData, SyncError> {
        // The isolated exporter purpose makes this a group-level proof. It
        // carries no channel or address and cannot open channel traffic.
        V2MemberEnvelopeScopeData::for_selected_prefix(self.verifier, 0)
    }

    fn route_key(&self, purpose: &'static str) -> Result<Zeroizing<[u8; 32]>, SyncError> {
        require_route(self.route)?;
        let base = member_exporter_key(self.group, self.device, &self.scope()?, purpose)?;
        let mut key = blake3::Hasher::new_keyed(&base);
        key.update(self.route.requester.as_bytes());
        key.update(self.route.provider.as_bytes());
        Ok(Zeroizing::new(*key.finalize().as_bytes()))
    }

    fn binding(&self, wire: &[u8]) -> Result<Binding, SyncError> {
        Ok(Binding {
            scope: self.scope()?,
            route: self.route,
            digest: blake3::derive_key(EXACT, wire),
        })
    }

    fn reply_key(&self, request: &Binding) -> Result<Zeroizing<[u8; 32]>, SyncError> {
        if request.scope != self.scope()? || request.route != self.route {
            return Err(SyncError::JoinRejected);
        }
        let base = self.route_key(REPLY_KEY)?;
        Ok(Zeroizing::new(
            *blake3::keyed_hash(&base, &request.digest).as_bytes(),
        ))
    }

    fn prove_local(
        &self,
        peer: PeerId,
        contact: &V2MemberContactData<'_>,
    ) -> Result<Proof, SyncError> {
        if self.local != contact.require_review_holder(self.verifier, peer)? {
            return Err(SyncError::JoinRejected);
        }
        Ok(contact.connected_proof())
    }

    fn open_proof(&self, peer: PeerId, proof: &[u8]) -> Result<DiscoveredV2Member, SyncError> {
        let proof: &Proof = proof.try_into().map_err(|_| SyncError::Malformed)?;
        let observation = V2MemberContactData::require_connected_proof(self.verifier, peer, proof)?;
        let admission =
            OwnerFactId::from_bytes(*proof.first_chunk::<32>().ok_or(SyncError::Malformed)?);
        if admission == self.local || !observation.is_live_cached() {
            return Err(SyncError::JoinRejected);
        }
        Ok(DiscoveredV2Member {
            admission,
            observation,
            peer,
        })
    }

    pub(crate) fn prepare(
        &self,
        local: &V2MemberContactData<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedV2MemberDiscovery, SyncError> {
        let proof = self.prove_local(self.route.requester, local)?;
        let key = self.route_key(REQUEST_KEY)?;
        let wire = seal_frame(&key, REQUEST, &proof, rng)?;
        Ok(PreparedV2MemberDiscovery {
            binding: self.binding(&wire)?,
            wire,
        })
    }

    pub(crate) fn open_request(&self, wire: &[u8]) -> Result<OpenedV2MemberDiscovery, SyncError> {
        let key = self.route_key(REQUEST_KEY)?;
        let proof = open_frame(&key, parse_frame(wire, REQUEST, REQUEST_BYTES)?)?;
        let requester = self.open_proof(self.route.requester, &proof)?;
        Ok(OpenedV2MemberDiscovery {
            binding: self.binding(wire)?,
            requester,
        })
    }

    pub(crate) fn seal_reply(
        &self,
        request: &OpenedV2MemberDiscovery,
        local: &V2MemberContactData<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, SyncError> {
        if !request.requester.is_live_cached() {
            return Err(SyncError::JoinRejected);
        }
        let key = self.reply_key(&request.binding)?;
        let proof = self.prove_local(self.route.provider, local)?;
        seal_frame(&key, REPLY, &proof, rng)
    }

    pub(crate) fn open_reply(
        &self,
        request: &PreparedV2MemberDiscovery,
        wire: &[u8],
    ) -> Result<DiscoveredV2Member, SyncError> {
        let key = self.reply_key(&request.binding)?;
        let proof = open_frame(&key, parse_frame(wire, REPLY, REPLY_BYTES)?)?;
        self.open_proof(self.route.provider, &proof)
    }
}
