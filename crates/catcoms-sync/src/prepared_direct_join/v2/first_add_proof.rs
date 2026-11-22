//! The bounded public genesis and first Add records needed by a new joiner.
use super::*;
use catcoms_replication::chat::{
    cold_records::{encode_header_row, HeaderRow, MAX_HEADER_ROW_BYTES},
    owner_header::SignedOwnerHeader,
    owner_profile::{AdmissionBody, SignedOwnerProfile, MAX_ADMISSION_BYTES, MAX_PROFILE_BYTES},
    owner_state::transition::{ReplayBody, ReplayMath},
};

const TAG: &[u8] = b"mewtual/chat/first-add-proof/v2\0";
const DELIVERY_TAG: &[u8] = b"mewtual/chat/first-join-delivery/v2\0";
const FRAME_OVERHEAD: usize = 4 + TAG.len() + 2 + 5 * 4;
pub const MAX_V2_FIRST_ADD_PROOF_BYTES: usize =
    FRAME_OVERHEAD + MAX_PROFILE_BYTES + 2 * (MAX_ADMISSION_BYTES + MAX_HEADER_ROW_BYTES);
pub const MAX_V2_FIRST_JOIN_DELIVERY_BYTES: usize = 4
    + DELIVERY_TAG.len()
    + 2
    + 2 * 4
    + MAX_V2_FIRST_ADD_PROOF_BYTES
    + MAX_V2_JOIN_RESPONSE_PACKET_BYTES;

pub struct V2AdmissionStepData<'a> {
    pub admission: &'a AdmissionBody,
    pub header: &'a SignedOwnerHeader,
    pub image: &'a [u8],
}

pub struct V2FirstAddProofData<'a> {
    profile: SignedOwnerProfile,
    founder: AdmissionBody,
    genesis: HeaderRow<'a>,
    admission: AdmissionBody,
    add: HeaderRow<'a>,
    bytes: usize,
}

impl<'a> V2FirstAddProofData<'a> {
    pub fn encode(
        profile: &SignedOwnerProfile,
        genesis: V2AdmissionStepData<'_>,
        add: V2AdmissionStepData<'_>,
    ) -> Result<Vec<u8>, SyncError> {
        let encoded_profile = profile.encode();
        let founder = genesis.admission.encode();
        let original =
            encode_header_row(genesis.header, genesis.image).map_err(|_| SyncError::Malformed)?;
        let admission = add.admission.encode();
        let next = encode_header_row(add.header, add.image).map_err(|_| SyncError::Malformed)?;
        let fields = [
            encoded_profile.as_slice(),
            founder.as_slice(),
            original.as_slice(),
            admission.as_slice(),
            next.as_slice(),
        ];
        let length = FRAME_OVERHEAD + fields.iter().map(|field| field.len()).sum::<usize>();
        if length > MAX_V2_FIRST_ADD_PROOF_BYTES {
            return Err(SyncError::Malformed);
        }
        let mut e = Encoder::with_capacity(length);
        e.put_bytes(TAG).map_err(|_| SyncError::Malformed)?;
        e.put_u16(VERSION);
        for field in fields {
            e.put_bytes(field).map_err(|_| SyncError::Malformed)?;
        }
        Ok(e.finish())
    }

    pub fn decode(raw: &'a [u8]) -> Result<Self, SyncError> {
        let malformed = |_| SyncError::Malformed;
        if raw.len() > MAX_V2_FIRST_ADD_PROOF_BYTES {
            return Err(SyncError::Malformed);
        }
        let mut d = Decoder::new(raw);
        if d.get_bytes().map_err(malformed)? != TAG || d.get_u16().map_err(malformed)? != VERSION {
            return Err(SyncError::Malformed);
        }
        let profile = d.get_bytes().map_err(malformed)?;
        let founder = d.get_bytes().map_err(malformed)?;
        let genesis = d.get_bytes().map_err(malformed)?;
        let admission = d.get_bytes().map_err(malformed)?;
        let add = d.get_bytes().map_err(malformed)?;
        d.finish().map_err(malformed)?;
        let invalid = |_| SyncError::JoinRejected;
        Ok(Self {
            profile: SignedOwnerProfile::decode(profile).map_err(invalid)?,
            founder: AdmissionBody::decode(founder).map_err(invalid)?,
            genesis: HeaderRow::decode(genesis).map_err(invalid)?,
            admission: AdmissionBody::decode(admission).map_err(invalid)?,
            add: HeaderRow::decode(add).map_err(invalid)?,
            bytes: raw.len(),
        })
    }

    pub(super) fn verified_admission(
        &self,
        accepted: &GenesisAnchorBindingData,
    ) -> Result<(&SignedOwnerProfile, &AdmissionBody, &SignedOwnerHeader), SyncError> {
        let genesis = self.genesis.header();
        if genesis.spec().group != accepted.group
            || genesis.spec().issuer != accepted.founder
            || self.profile.id().as_bytes() != &accepted.profile
            || self.profile.tenure().as_bytes() != &accepted.tenure
            || self.founder.admission_id().as_bytes() != &accepted.admission
            || genesis.id().as_bytes() != &accepted.header
            || genesis.spec().state.as_bytes() != &accepted.state
        {
            return Err(SyncError::JoinRejected);
        }
        ReplayMath::from_genesis(&self.profile, &self.founder, genesis, self.genesis.image())
            .and_then(|prior| {
                prior.derive_successor(
                    self.add.header(),
                    ReplayBody::Admission(&self.admission),
                    self.add.image(),
                )
            })
            .map_err(|_| SyncError::JoinRejected)?;
        Ok((&self.profile, &self.admission, self.add.header()))
    }

    pub(super) fn frame_bytes(&self) -> usize {
        self.bytes
    }

    pub(super) fn add_image_data(&self) -> &[u8] {
        self.add.image()
    }
}

impl fmt::Debug for V2FirstAddProofData<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("V2FirstAddProofData")
            .finish_non_exhaustive()
    }
}

/// Bounded wire pairing of a signed response and its public proof.
pub struct V2FirstJoinDeliveryData<'a> {
    response: V2JoinResponsePacketData<'a>,
    proof: V2FirstAddProofData<'a>,
}

impl<'a> V2FirstJoinDeliveryData<'a> {
    pub fn encode(response: &[u8], proof: &[u8]) -> Result<Vec<u8>, SyncError> {
        if response.is_empty()
            || response.len() > MAX_V2_JOIN_RESPONSE_PACKET_BYTES
            || proof.is_empty()
            || proof.len() > MAX_V2_FIRST_ADD_PROOF_BYTES
        {
            return Err(SyncError::Malformed);
        }
        let length = 4 + DELIVERY_TAG.len() + 2 + 2 * 4 + response.len() + proof.len();
        let mut e = Encoder::with_capacity(length);
        e.put_bytes(DELIVERY_TAG)
            .map_err(|_| SyncError::Malformed)?;
        e.put_u16(VERSION);
        e.put_bytes(response).map_err(|_| SyncError::Malformed)?;
        e.put_bytes(proof).map_err(|_| SyncError::Malformed)?;
        Ok(e.finish())
    }

    pub fn decode(raw: &'a [u8]) -> Result<Self, SyncError> {
        if raw.len() > MAX_V2_FIRST_JOIN_DELIVERY_BYTES {
            return Err(SyncError::Malformed);
        }
        let malformed = |_| SyncError::Malformed;
        let mut d = Decoder::new(raw);
        if d.get_bytes().map_err(malformed)? != DELIVERY_TAG
            || d.get_u16().map_err(malformed)? != VERSION
        {
            return Err(SyncError::Malformed);
        }
        let response = d.get_bytes().map_err(malformed)?;
        let proof = d.get_bytes().map_err(malformed)?;
        d.finish().map_err(malformed)?;
        if response.len() > MAX_V2_JOIN_RESPONSE_PACKET_BYTES
            || proof.len() > MAX_V2_FIRST_ADD_PROOF_BYTES
        {
            return Err(SyncError::Malformed);
        }
        Ok(Self {
            response: V2JoinResponsePacketData::decode(response)?,
            proof: V2FirstAddProofData::decode(proof)?,
        })
    }

    pub(super) fn data(&self) -> (V2JoinResponseData<'_>, &V2FirstAddProofData<'a>) {
        (self.response.response_data(), &self.proof)
    }
}
