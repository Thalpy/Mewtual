//! A bounded member Add delivery. The signed Header is readable separately
//! from the untrusted body so corrupt bytes cannot hide a signed sibling.
use super::*;
use catcoms_mls::group::text_current_add::MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES;
use catcoms_replication::chat::{
    owner_header::{OwnerHeaderId, SignedOwnerHeader, MAX_HEADER_BYTES},
    owner_profile::{AdmissionBody, AdmissionOrigin, MAX_ADMISSION_BYTES},
    owner_state::{transition::ReplayMath, MAX_STATE_BYTES},
};

const TAG: &[u8] = b"mewtual/chat/member-add-delivery/v2\0";
const REQUEST: &[u8] = b"mewtual/chat/next-member-add/v2\0";
const REQUEST_BYTES: usize = 4 + REQUEST.len() + 2 + 36 + 8 + 36;
const FETCH_TOKEN: &str = "mewtual/chat/next-member-add-token/v2";
const FETCH_VERIFIER: &str = "mewtual/chat/next-member-add-verifier/v2";
const _: () = assert!(REQUEST_BYTES < catcoms_rt::CHAT_V2_GOVERNING_REQUEST_MAX_BYTES);
const OVERHEAD: usize = 4 + TAG.len() + 2 + 3 * 4;
pub const MAX_V2_MEMBER_ADD_FRAME_BYTES: usize = OVERHEAD
    + MAX_HEADER_BYTES
    + MAX_ADMISSION_BYTES
    + MAX_STATE_BYTES
    + MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES;
const _: () =
    assert!(MAX_V2_MEMBER_ADD_FRAME_BYTES <= catcoms_rt::CHAT_V2_GOVERNING_RESPONSE_MAX_BYTES);

/// A bearer request for the immediate successor to one original selected MLS
/// epoch. The founder can retain the one-way verifier separately from the key.
pub struct V2MemberAddRequestData {
    parent: OwnerHeaderId,
    epoch: u64,
    token: Zeroizing<[u8; 32]>,
}

impl V2MemberAddRequestData {
    pub(crate) fn for_selected_group(
        group: &ServerGroup,
        device: &MlsDevice,
        replay: &ReplayMath,
    ) -> Result<Self, SyncError> {
        let header = replay.header();
        let epoch = replay.mls_epoch_data();
        if epoch == 0
            || epoch != group.epoch()
            || !group.is_active()
            || group.group_id_ref() != header.spec().group.as_slice()
            || !group.has_local_signer(&device.public_key_bytes())
        {
            return Err(SyncError::JoinRejected);
        }
        let base = Zeroizing::new(group.routing_metadata_secret(device)?);
        let profile = replay.profile().id();
        let parent = header.id();
        let mut hash = blake3::Hasher::new_derive_key(FETCH_TOKEN);
        hash.update(base.as_slice());
        hash.update(profile.as_bytes());
        hash.update(parent.as_bytes());
        hash.update(&epoch.to_be_bytes());
        Ok(Self {
            parent,
            epoch,
            token: Zeroizing::new(*hash.finalize().as_bytes()),
        })
    }

    pub fn encode(&self) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        let mut e = Encoder::with_capacity(REQUEST_BYTES);
        let bad = |_| SyncError::Malformed;
        e.put_bytes(REQUEST).map_err(bad)?;
        e.put_u16(VERSION);
        e.put_bytes(self.parent.as_bytes()).map_err(bad)?;
        e.put_u64(self.epoch);
        e.put_bytes(self.token.as_slice()).map_err(bad)?;
        Ok(Zeroizing::new(e.finish()))
    }

    pub fn decode(raw: &[u8]) -> Result<Self, SyncError> {
        if raw.len() != REQUEST_BYTES {
            return Err(SyncError::Malformed);
        }
        let mut d = Decoder::new(raw);
        let bad = |_| SyncError::Malformed;
        if d.get_bytes().map_err(bad)? != REQUEST || d.get_u16().map_err(bad)? != VERSION {
            return Err(SyncError::Malformed);
        }
        let parent: [u8; 32] = d
            .get_bytes()
            .map_err(bad)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        let epoch = d.get_u64().map_err(bad)?;
        let token: [u8; 32] = d
            .get_bytes()
            .map_err(bad)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        d.finish().map_err(bad)?;
        if parent == [0; 32] || epoch == 0 || token == [0; 32] {
            return Err(SyncError::Malformed);
        }
        Ok(Self {
            parent: OwnerHeaderId::from_bytes(parent),
            epoch,
            token: Zeroizing::new(token),
        })
    }

    pub fn parent_data(&self) -> (OwnerHeaderId, u64) {
        (self.parent, self.epoch)
    }

    pub fn verifier_data(&self) -> [u8; 32] {
        let mut hash = blake3::Hasher::new_derive_key(FETCH_VERIFIER);
        hash.update(self.parent.as_bytes());
        hash.update(&self.epoch.to_be_bytes());
        hash.update(self.token.as_slice());
        *hash.finalize().as_bytes()
    }

    pub fn matches_verifier_data(&self, expected: [u8; 32]) -> bool {
        blake3::Hash::from(self.verifier_data()) == blake3::Hash::from(expected)
    }
}

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
