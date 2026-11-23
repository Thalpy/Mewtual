//! A bounded member Add delivery. The signed Header is readable separately
//! from the untrusted body so corrupt bytes cannot hide a signed sibling.
use super::*;
use catcoms_mls::group::text_current_add::MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES;
use catcoms_replication::chat::{
    owner_header::{SignedOwnerHeader, MAX_HEADER_BYTES},
    owner_profile::{AdmissionBody, AdmissionOrigin, MAX_ADMISSION_BYTES},
    owner_state::MAX_STATE_BYTES,
};

const TAG: &[u8] = b"mewtual/chat/member-add-delivery/v2\0";
const OVERHEAD: usize = 4 + TAG.len() + 2 + 3 * 4;
pub const MAX_V2_MEMBER_ADD_FRAME_BYTES: usize = OVERHEAD
    + MAX_HEADER_BYTES
    + MAX_ADMISSION_BYTES
    + MAX_STATE_BYTES
    + MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES;
const _: () =
    assert!(MAX_V2_MEMBER_ADD_FRAME_BYTES <= catcoms_rt::CHAT_V2_GOVERNING_RESPONSE_MAX_BYTES);

pub struct V2MemberAddFrameData<'a> {
    header: SignedOwnerHeader,
    remainder: &'a [u8],
}

impl<'a> V2MemberAddFrameData<'a> {
    pub fn encode(step: V2AdmissionStepData<'_>, commit: &[u8]) -> Result<Vec<u8>, SyncError> {
        let AdmissionOrigin::Add { commit: digest, .. } = step.admission.spec().origin else {
            return Err(SyncError::JoinRejected);
        };
        if step.image.is_empty()
            || step.image.len() > MAX_STATE_BYTES
            || commit.is_empty()
            || commit.len() > MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES
            || blake3::hash(commit).as_bytes() != &digest
        {
            return Err(SyncError::JoinRejected);
        }
        let header = step.header.encode();
        let body = step.admission.encode();
        let mut encoder = Encoder::with_capacity(
            OVERHEAD + header.len() + body.len() + step.image.len() + commit.len(),
        );
        let bad = |_| SyncError::Malformed;
        encoder.put_bytes(TAG).map_err(bad)?;
        encoder.put_u16(VERSION);
        for bytes in [header.as_slice(), body.as_slice(), step.image] {
            encoder.put_bytes(bytes).map_err(bad)?;
        }
        let mut bytes = encoder.finish();
        bytes.extend_from_slice(commit);
        Ok(bytes)
    }

    pub fn decode(raw: &'a [u8]) -> Result<Self, SyncError> {
        if raw.len() > MAX_V2_MEMBER_ADD_FRAME_BYTES {
            return Err(SyncError::Malformed);
        }
        let mut decoder = Decoder::new(raw);
        let bad = |_| SyncError::Malformed;
        if decoder.get_bytes().map_err(bad)? != TAG || decoder.get_u16().map_err(bad)? != VERSION {
            return Err(SyncError::Malformed);
        }
        let header = SignedOwnerHeader::decode(decoder.get_bytes().map_err(bad)?)
            .map_err(|_| SyncError::JoinRejected)?;
        Ok(Self {
            header,
            remainder: &raw[raw.len() - decoder.remaining()..],
        })
    }

    pub fn header_data(&self) -> &SignedOwnerHeader {
        &self.header
    }

    pub(super) fn body_data(&self) -> Result<(AdmissionBody, &'a [u8], &'a [u8]), SyncError> {
        let mut decoder = Decoder::new(self.remainder);
        let bad = |_| SyncError::Malformed;
        let body = decoder.get_bytes().map_err(bad)?;
        let image = decoder.get_bytes().map_err(bad)?;
        let commit = &self.remainder[self.remainder.len() - decoder.remaining()..];
        if image.is_empty()
            || image.len() > MAX_STATE_BYTES
            || commit.is_empty()
            || commit.len() > MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES
        {
            return Err(SyncError::Malformed);
        }
        let admission = AdmissionBody::decode(body).map_err(|_| SyncError::JoinRejected)?;
        Ok((admission, image, commit))
    }
}
