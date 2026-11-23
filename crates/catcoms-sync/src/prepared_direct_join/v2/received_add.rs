//! A bounded member transition delivery. The signed Header is readable separately
//! from the untrusted body so corrupt bytes cannot hide a signed sibling.
use super::*;
use catcoms_mls::group::text_current_add::MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES;
use catcoms_replication::chat::{
    owner_bodies::{RemovalBody, MAX_REMOVAL_BODY_BYTES},
    owner_header::{HeaderKind, OwnerHeaderId, SignedOwnerHeader, MAX_HEADER_BYTES},
    owner_profile::{AdmissionBody, AdmissionOrigin, MAX_ADMISSION_BYTES},
    owner_state::{transition::ReplayMath, MAX_STATE_BYTES},
};

const TAG: &[u8] = b"mewtual/chat/member-add-delivery/v2\0";
const REMOVE_TAG: &[u8] = b"mewtual/chat/member-remove-delivery/v2\0";
const REQUEST: &[u8] = b"mewtual/chat/next-member-add/v2\0";
const REQUEST_BYTES: usize = 4 + REQUEST.len() + 2 + 36 + 8 + 36;
const FETCH_TOKEN: &str = "mewtual/chat/next-member-add-token/v2";
const FETCH_VERIFIER: &str = "mewtual/chat/next-member-add-verifier/v2";
const _: () = assert!(REQUEST_BYTES < catcoms_rt::CHAT_V2_GOVERNING_REQUEST_MAX_BYTES);
const OVERHEAD: usize = 4 + 2 + 3 * 4;
pub const MAX_V2_MEMBER_ADD_FRAME_BYTES: usize = OVERHEAD
    + TAG.len()
    + MAX_HEADER_BYTES
    + MAX_ADMISSION_BYTES
    + MAX_STATE_BYTES
    + MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES;
const MAX_REMOVE_FRAME: usize = OVERHEAD
    + REMOVE_TAG.len()
    + MAX_HEADER_BYTES
    + MAX_REMOVAL_BODY_BYTES
    + MAX_STATE_BYTES
    + MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES;
pub const MAX_V2_MEMBER_TRANSITION_FRAME_BYTES: usize =
    if MAX_REMOVE_FRAME > MAX_V2_MEMBER_ADD_FRAME_BYTES {
        MAX_REMOVE_FRAME
    } else {
        MAX_V2_MEMBER_ADD_FRAME_BYTES
    };
const _: () = assert!(
    MAX_V2_MEMBER_TRANSITION_FRAME_BYTES <= catcoms_rt::CHAT_V2_GOVERNING_RESPONSE_MAX_BYTES
);

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

pub struct V2MemberAddFrameData<'a>(V2MemberTransitionFrameData<'a>);

pub struct V2MemberTransitionFrameData<'a> {
    header: SignedOwnerHeader,
    kind: HeaderKind,
    remainder: &'a [u8],
    digest: blake3::Hash,
}

pub(super) enum TransitionBody {
    Add(AdmissionBody),
    Removal(RemovalBody),
}

impl<'a> V2MemberAddFrameData<'a> {
    pub fn encode(step: V2AdmissionStepData<'_>, commit: &[u8]) -> Result<Vec<u8>, SyncError> {
        let AdmissionOrigin::Add { commit: digest, .. } = step.admission.spec().origin else {
            return Err(SyncError::JoinRejected);
        };
        if step.header.spec().kind != HeaderKind::Admission {
            return Err(SyncError::JoinRejected);
        }
        V2MemberTransitionFrameData::encode(
            TAG,
            step.header,
            &step.admission.encode(),
            step.image,
            commit,
            digest,
        )
    }

    pub fn decode(raw: &'a [u8]) -> Result<Self, SyncError> {
        let frame = V2MemberTransitionFrameData::decode(raw)?;
        if frame.kind != HeaderKind::Admission {
            return Err(SyncError::Malformed);
        }
        Ok(Self(frame))
    }

    pub fn header_data(&self) -> &SignedOwnerHeader {
        self.0.header_data()
    }

    pub(super) fn transition_data(&self) -> &V2MemberTransitionFrameData<'a> {
        &self.0
    }
}

impl<'a> V2MemberTransitionFrameData<'a> {
    pub fn encode_removal(
        header: &SignedOwnerHeader,
        body: &RemovalBody,
        image: &[u8],
        commit: &[u8],
    ) -> Result<Vec<u8>, SyncError> {
        body.verify_header_binding(header)
            .map_err(|_| SyncError::JoinRejected)?;
        Self::encode(
            REMOVE_TAG,
            header,
            &body.encode(),
            image,
            commit,
            body.spec().commit,
        )
    }

    fn encode(
        tag: &[u8],
        header: &SignedOwnerHeader,
        body: &[u8],
        image: &[u8],
        commit: &[u8],
        digest: [u8; 32],
    ) -> Result<Vec<u8>, SyncError> {
        Self::require_bounds(image, commit)?;
        if blake3::hash(commit).as_bytes() != &digest {
            return Err(SyncError::JoinRejected);
        }
        let header = header.encode();
        let mut encoder = Encoder::with_capacity(
            OVERHEAD + tag.len() + header.len() + body.len() + image.len() + commit.len(),
        );
        let bad = |_| SyncError::Malformed;
        encoder.put_bytes(tag).map_err(bad)?;
        encoder.put_u16(VERSION);
        for bytes in [header.as_slice(), body, image] {
            encoder.put_bytes(bytes).map_err(bad)?;
        }
        let mut bytes = encoder.finish();
        bytes.extend_from_slice(commit);
        Ok(bytes)
    }

    pub fn decode(raw: &'a [u8]) -> Result<Self, SyncError> {
        if raw.len() > MAX_V2_MEMBER_TRANSITION_FRAME_BYTES {
            return Err(SyncError::Malformed);
        }
        let mut decoder = Decoder::new(raw);
        let bad = |_| SyncError::Malformed;
        let kind = match decoder.get_bytes().map_err(bad)? {
            TAG if raw.len() <= MAX_V2_MEMBER_ADD_FRAME_BYTES => HeaderKind::Admission,
            REMOVE_TAG if raw.len() <= MAX_REMOVE_FRAME => HeaderKind::Removal,
            _ => return Err(SyncError::Malformed),
        };
        if decoder.get_u16().map_err(bad)? != VERSION {
            return Err(SyncError::Malformed);
        }
        let header = SignedOwnerHeader::decode(decoder.get_bytes().map_err(bad)?)
            .map_err(|_| SyncError::JoinRejected)?;
        Ok(Self {
            header,
            kind,
            remainder: &raw[raw.len() - decoder.remaining()..],
            digest: blake3::hash(raw),
        })
    }

    pub fn header_data(&self) -> &SignedOwnerHeader {
        &self.header
    }

    pub fn kind_data(&self) -> HeaderKind {
        self.kind
    }

    /// Read the quoted admission from a Remove body after the caller observes
    /// its signed Header. Core and MLS still authenticate the original cut.
    pub fn removal_admission_data(
        &self,
    ) -> Result<Option<catcoms_replication::chat::owner_facts::OwnerFactId>, SyncError> {
        if self.kind != HeaderKind::Removal {
            return Ok(None);
        }
        let (TransitionBody::Removal(body), _, _) = self.body_data()? else {
            return Err(SyncError::JoinRejected);
        };
        Ok(Some(body.spec().frozen_tip.context.admission))
    }

    pub(super) fn digest_data(&self) -> blake3::Hash {
        self.digest
    }

    pub(super) fn body_data(&self) -> Result<(TransitionBody, &'a [u8], &'a [u8]), SyncError> {
        let mut decoder = Decoder::new(self.remainder);
        let bad = |_| SyncError::Malformed;
        let body = decoder.get_bytes().map_err(bad)?;
        let image = decoder.get_bytes().map_err(bad)?;
        let commit = &self.remainder[self.remainder.len() - decoder.remaining()..];
        Self::require_bounds(image, commit)?;
        if self.kind != self.header.spec().kind {
            return Err(SyncError::JoinRejected);
        }
        let bad_body = |_| SyncError::JoinRejected;
        let body = match self.kind {
            HeaderKind::Admission => {
                TransitionBody::Add(AdmissionBody::decode(body).map_err(bad_body)?)
            }
            HeaderKind::Removal => {
                TransitionBody::Removal(RemovalBody::decode(body).map_err(bad_body)?)
            }
            _ => return Err(SyncError::Malformed),
        };
        Ok((body, image, commit))
    }

    fn require_bounds(image: &[u8], commit: &[u8]) -> Result<(), SyncError> {
        if image.is_empty()
            || image.len() > MAX_STATE_BYTES
            || commit.is_empty()
            || commit.len() > MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES
        {
            return Err(SyncError::Malformed);
        }
        Ok(())
    }
}
