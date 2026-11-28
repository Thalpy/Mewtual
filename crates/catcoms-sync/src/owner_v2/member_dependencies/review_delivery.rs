//! Cover one exact review-stream read on a current member connection.
//! The connected host must independently authenticate the original founder
//! and the requesting Admission. The group exporter supplies neither.
use super::*;
use crate::V2MemberContactData;
use catcoms_replication::chat::{
    owner_bodies::{
        ReviewStreamEntry, ReviewStreamEntryId, ReviewStreamQuery, ReviewStreamRecord,
        MAX_REVIEW_STREAM_ENTRY_BYTES, MAX_REVIEW_STREAM_RECORD_BYTES,
    },
    FounderGenesisAnchorData, V2GeneralPrefixVerifier, V2LiveMemberObservation,
};

// Keep the signature and complete expected identity under kind 32's
// existing cap, including the kind byte paid by the network wrapper.
const REQUEST: &[u8] = b"mr-v1\0";
const REPLY: &[u8] = b"mewtual/chat/review-delivery-reply/v1\0";
const REQUEST_KEY: &str = "mewtual/chat/review-delivery-request-key/v1";
const REPLY_KEY: &str = "mewtual/chat/review-delivery-reply-key/v1";
const EXACT: &str = "mewtual/chat/review-delivery-exact-request/v1";
const PROOF_BYTES: usize = V2MemberContactData::REVIEW_REQUEST_PROOF_BYTES;
const REQUEST_BODY: usize = 1 + 8 + 16 + 36 + 4 + PROOF_BYTES;
// The dedicated exporter purpose makes zero a group-wide carrier here.
// The approval channel is authenticated inside that request and record.
pub const REVIEW_STREAM_CARRIER: u128 = 0;
const MAX_REQUEST: usize = REQUEST.len() + OVERHEAD + REQUEST_BODY;
const MAX_RESPONSE: usize = REPLY.len() + OVERHEAD + 1 + 4 + MAX_REVIEW_STREAM_RECORD_BYTES;
const _: () = assert!(MAX_REQUEST < catcoms_rt::CHAT_V2_DEPENDENCY_REQUEST_MAX_BYTES);
const _: () = assert!(MAX_RESPONSE <= catcoms_rt::CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES);

catcoms_rt::redacted_debug! {
    ReviewEndpoint<'_>;
    PreparedV2ReviewDelivery;
    OpenedV2ReviewDelivery;
    V2ReviewDeliveryData;
}

pub(crate) struct ReviewEndpoint<'a> {
    pub(crate) group: &'a ServerGroup,
    pub(crate) device: &'a MlsDevice,
    pub(crate) scope: V2MemberEnvelopeScopeData,
    pub(crate) route: V2MemberDependencyRoute,
    pub(crate) founder: OwnerFactId,
}

struct RequestBinding {
    scope: V2MemberEnvelopeScopeData,
    route: V2MemberDependencyRoute,
    founder: OwnerFactId,
    query: ReviewStreamQuery,
    digest: [u8; 32],
}

pub struct PreparedV2ReviewDelivery {
    binding: RequestBinding,
    wire: Vec<u8>,
}

pub struct OpenedV2ReviewDelivery {
    binding: RequestBinding,
    requester: V2LiveMemberObservation,
}

pub enum V2ReviewDeliveryData {
    Header(Box<ReviewStreamEntry>),
    Approval(Zeroizing<Vec<u8>>),
}

impl PreparedV2ReviewDelivery {
    pub fn wire_data(&self) -> &[u8] {
        &self.wire
    }
}

impl OpenedV2ReviewDelivery {
    pub fn recognizes(wire: &[u8]) -> bool {
        wire.len() <= MAX_REQUEST
            && Decoder::new(wire)
                .get_bytes()
                .is_ok_and(|tag| tag == REQUEST)
    }

    pub fn query_data(&self) -> ReviewStreamQuery {
        self.binding.query
    }

    pub fn requester_data(&self) -> &V2LiveMemberObservation {
        &self.requester
    }

    pub fn into_requester(self) -> V2LiveMemberObservation {
        self.requester
    }
}

impl ReviewEndpoint<'_> {
    fn route_key(&self, purpose: &'static str) -> Result<Zeroizing<[u8; 32]>, SyncError> {
        require_route(self.route)?;
        let base = member_exporter_key(self.group, self.device, &self.scope, purpose)?;
        let mut key = blake3::Hasher::new_keyed(&base);
        key.update(self.route.requester.as_bytes());
        key.update(self.route.provider.as_bytes());
        key.update(self.founder.as_bytes());
        Ok(Zeroizing::new(*key.finalize().as_bytes()))
    }

    fn binding(&self, query: ReviewStreamQuery, wire: &[u8]) -> RequestBinding {
        RequestBinding {
            scope: self.scope,
            route: self.route,
            founder: self.founder,
            query,
            digest: blake3::derive_key(EXACT, wire),
        }
    }

    fn reply_key(&self, exact: &RequestBinding) -> Result<Zeroizing<[u8; 32]>, SyncError> {
        if exact.scope != self.scope || exact.route != self.route || exact.founder != self.founder {
            return Err(SyncError::JoinRejected);
        }
        let base = self.route_key(REPLY_KEY)?;
        Ok(Zeroizing::new(
            *blake3::keyed_hash(&base, &exact.digest).as_bytes(),
        ))
    }

    pub(crate) fn prepare(
        &self,
        contact: &V2MemberContactData<'_>,
        query: ReviewStreamQuery,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedV2ReviewDelivery, SyncError> {
        if query.sequence_data() == 0 {
            return Err(SyncError::Malformed);
        }
        let key = self.route_key(REQUEST_KEY)?;
        let mut request = Encoder::with_capacity(REQUEST_BODY);
        match query {
            ReviewStreamQuery::Header(sequence) => {
                request.put_u8(0).put_u64(sequence);
            }
            ReviewStreamQuery::Approval {
                sequence,
                entry,
                channel,
            } => {
                request.put_u8(1).put_u64(sequence).put_u128(channel);
                data(request.put_bytes(entry.as_bytes()))?;
            }
        }
        data(request.put_bytes(&contact.review_request_proof()))?;
        let wire = seal_frame(&key, REQUEST, &request.finish(), rng)?;
        Ok(PreparedV2ReviewDelivery {
            binding: self.binding(query, &wire),
            wire,
        })
    }

    pub(super) fn open_request(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        wire: &[u8],
    ) -> Result<OpenedV2ReviewDelivery, SyncError> {
        if V2MemberEnvelopeScopeData::for_selected_prefix(verifier, REVIEW_STREAM_CARRIER)?
            != self.scope
        {
            return Err(SyncError::JoinRejected);
        }
        let key = self.route_key(REQUEST_KEY)?;
        let request = open_frame(&key, parse_frame(wire, REQUEST, MAX_REQUEST)?)?;
        let mut read = Decoder::new(&request);
        let kind = data(read.get_u8())?;
        let sequence = data(read.get_u64())?;
        if sequence == 0 {
            return Err(SyncError::Malformed);
        }
        let query = match kind {
            0 => ReviewStreamQuery::Header(sequence),
            1 => ReviewStreamQuery::Approval {
                sequence,
                channel: data(read.get_u128())?,
                entry: ReviewStreamEntryId::from_bytes_data(
                    data(read.get_bytes())?
                        .try_into()
                        .map_err(|_| SyncError::Malformed)?,
                )
                .map_err(|_| SyncError::Malformed)?,
            },
            _ => return Err(SyncError::Malformed),
        };
        let proof = data(read.get_bytes())?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        data(read.finish())?;
        let requester = V2MemberContactData::require_review_request_proof(
            verifier,
            self.route.requester,
            proof,
        )?;
        Ok(OpenedV2ReviewDelivery {
            binding: self.binding(query, wire),
            requester,
        })
    }

    pub(super) fn seal_reply(
        &self,
        pin: &FounderGenesisAnchorData,
        request: &OpenedV2ReviewDelivery,
        reply: Option<&[u8]>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, SyncError> {
        if !request.requester.is_live_cached() {
            return Err(SyncError::JoinRejected);
        }
        let key = self.reply_key(&request.binding)?;
        if let Some(bytes) = reply {
            // Enforce the same disclosure check on the sending side.
            // A valid private record is never a valid Header response.
            self.check_reply(pin, request.binding.query, bytes)?;
        }
        let mut plain = Encoder::with_capacity(1 + reply.map_or(0, |bytes| 4 + bytes.len()));
        plain.put_u8(u8::from(reply.is_some()));
        if let Some(bytes) = reply {
            data(plain.put_bytes(bytes))?;
        }
        seal_frame(&key, REPLY, &Zeroizing::new(plain.finish()), rng)
    }

    pub(crate) fn open_reply(
        &self,
        pin: &FounderGenesisAnchorData,
        request: &PreparedV2ReviewDelivery,
        wire: &[u8],
    ) -> Result<Option<V2ReviewDeliveryData>, SyncError> {
        let key = self.reply_key(&request.binding)?;
        let plain = open_frame(&key, parse_frame(wire, REPLY, MAX_RESPONSE)?)?;
        let mut read = Decoder::new(&plain);
        let payload = match data(read.get_u8())? {
            0 => None,
            1 => Some(data(read.get_bytes())?),
            _ => return Err(SyncError::Malformed),
        };
        data(read.finish())?;
        payload
            .map(
                |bytes| match self.check_reply(pin, request.binding.query, bytes)? {
                    Some(header) => Ok(V2ReviewDeliveryData::Header(Box::new(header))),
                    None => Ok(V2ReviewDeliveryData::Approval(Zeroizing::new(
                        bytes.to_vec(),
                    ))),
                },
            )
            .transpose()
    }

    fn check_reply(
        &self,
        pin: &FounderGenesisAnchorData,
        query: ReviewStreamQuery,
        bytes: &[u8],
    ) -> Result<Option<ReviewStreamEntry>, SyncError> {
        if pin.admission != self.founder
            || pin.group.as_slice() != self.scope.group
            || pin.profile != self.scope.profile
        {
            return Err(SyncError::JoinRejected);
        }
        match query {
            ReviewStreamQuery::Header(sequence) => {
                if bytes.len() != MAX_REVIEW_STREAM_ENTRY_BYTES {
                    return Err(SyncError::Malformed);
                }
                let header = ReviewStreamEntry::decode(bytes).map_err(|_| SyncError::Malformed)?;
                header
                    .verify_for_pin(pin)
                    .map_err(|_| SyncError::JoinRejected)?;
                if header.position_data().sequence != sequence || header.case_step_data().is_none()
                {
                    return Err(SyncError::JoinRejected);
                }
                Ok(Some(header))
            }
            ReviewStreamQuery::Approval {
                sequence,
                entry,
                channel,
            } => {
                let record = ReviewStreamRecord::decode_public_approval(pin, entry, bytes)
                    .map_err(|_| SyncError::JoinRejected)?;
                if record.entry_data().position_data().sequence != sequence
                    || record
                        .decision_data()
                        .decision()
                        .spec()
                        .target
                        .spec()
                        .channel
                        != channel
                {
                    return Err(SyncError::JoinRejected);
                }
                Ok(None)
            }
        }
    }
}

impl V2FounderMemberDependencyAccess<'_> {
    pub fn open_review_delivery_request(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        pin: &FounderGenesisAnchorData,
        requester: PeerId,
        wire: &[u8],
    ) -> Result<OpenedV2ReviewDelivery, SyncError> {
        self.review_delivery_endpoint(pin, requester)?
            .open_request(verifier, wire)
    }

    pub fn seal_review_delivery_reply(
        &self,
        pin: &FounderGenesisAnchorData,
        requester: PeerId,
        request: &OpenedV2ReviewDelivery,
        bytes: Option<&[u8]>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<u8>, SyncError> {
        self.review_delivery_endpoint(pin, requester)?
            .seal_reply(pin, request, bytes, rng)
    }

    fn review_delivery_endpoint(
        &self,
        pin: &FounderGenesisAnchorData,
        requester: PeerId,
    ) -> Result<ReviewEndpoint<'_>, SyncError> {
        if !self.observation.is_current_cached()
            || self.member.admission_data().admission_id() != pin.admission
            || self.member.admission_data().spec().subject != pin.founder
            || self.scope.profile != pin.profile
            || self.scope.group.as_slice() != pin.group.as_slice()
            || self.scope.channel != REVIEW_STREAM_CARRIER
        {
            return Err(SyncError::JoinRejected);
        }
        Ok(ReviewEndpoint {
            group: self.group,
            device: self.device,
            scope: self.scope,
            route: self.route_data(requester),
            founder: pin.admission,
        })
    }
}
