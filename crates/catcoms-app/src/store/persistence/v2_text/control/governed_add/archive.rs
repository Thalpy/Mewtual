//! Immutable governed Add archive. It retains the exact encrypted journal so
//! selecting a successor can eventually release the fixed transaction names.
use super::header_signing::{ATTEMPT_BYTES, RAW_BYTES, WELCOME_ATTEMPT_BYTES, WELCOME_RAW_BYTES};
use super::outcome::{MAX_FRAME, MIN_FRAME};
use super::sdk_entry::ENTRY_BYTES;
use super::*;
use crate::store::persistence::v2_text::{archive::MAX_ARCHIVE_PLAIN, head::MAX_TEXT_HEAD_FRAME};
use catcoms_replication::chat::{
    cold_manifest::ArchiveHeadId,
    owner_header::{OwnerHeaderId, OwnerStateCommitment},
};

pub(super) mod recovery;

const TAG: &[u8] = b"catcoms/private-chat-text-governed-add-archive/v2\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-text-governed-add-archive-key/v2";
const ID_DOMAIN: &str = "catcoms/private-chat-text-governed-add-archive-id/v2";
const VERSION: u16 = 2;
const BODY_CAPS: [(usize, usize); 13] = [
    (41, MAX_TEXT_HEAD_FRAME),
    (41, MAX_ARCHIVE_PLAIN + 40),
    (FRAME_BYTES, FRAME_BYTES),
    (request::MIN_BYTES, request::MAX_BYTES),
    (ENTRY_BYTES, ENTRY_BYTES),
    (MIN_FRAME, MAX_FRAME),
    (ATTEMPT_BYTES, ATTEMPT_BYTES),
    (RAW_BYTES, RAW_BYTES),
    (governing_record::MIN_BYTES, governing_record::MAX_BYTES),
    (welcome_intent::MIN_BYTES, welcome_intent::MAX_BYTES),
    (WELCOME_ATTEMPT_BYTES, WELCOME_ATTEMPT_BYTES),
    (WELCOME_RAW_BYTES, WELCOME_RAW_BYTES),
    (response::MIN_BYTES, response::MAX_BYTES),
];
const OVERHEAD: usize = 4 + TAG.len() + 2 + 6 * 36 + 8 + 4 * BODY_CAPS.len() + 40;
pub(super) const MAX_BYTES: usize = {
    let mut max = OVERHEAD;
    let mut i = 0;
    while i < BODY_CAPS.len() {
        max += BODY_CAPS[i].1;
        i += 1;
    }
    max
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct GovernedArchiveLabels {
    pub(super) context: TextContextIdData,
    pub(super) before_head: TextHeadIdData,
    pub(super) before_archive: ArchiveHeadId,
    pub(super) header: OwnerHeaderId,
    pub(super) state: OwnerStateCommitment,
    pub(super) epoch: u64,
    pub(super) authenticator: [u8; 32],
}

pub(super) struct GovernedArchiveFrames<'a> {
    pub(super) parent_head: &'a [u8],
    pub(super) parent_archive: &'a [u8],
    pub(super) fence: &'a [u8],
    pub(super) request: &'a [u8],
    pub(super) entry: &'a [u8],
    pub(super) outcome: &'a [u8],
    pub(super) header_attempt: &'a [u8],
    pub(super) raw_header: &'a [u8],
    pub(super) governing: &'a [u8],
    pub(super) welcome_intent: &'a [u8],
    pub(super) welcome_attempt: &'a [u8],
    pub(super) raw_welcome: &'a [u8],
    pub(super) response: &'a [u8],
}

impl<'a> GovernedArchiveFrames<'a> {
    fn ordered(&self) -> [&'a [u8]; BODY_CAPS.len()] {
        [
            self.parent_head,
            self.parent_archive,
            self.fence,
            self.request,
            self.entry,
            self.outcome,
            self.header_attempt,
            self.raw_header,
            self.governing,
            self.welcome_intent,
            self.welcome_attempt,
            self.raw_welcome,
            self.response,
        ]
    }

    pub(super) fn frame_bytes(&self) -> Result<usize, AppError> {
        self.ordered()
            .iter()
            .zip(BODY_CAPS)
            .try_fold(OVERHEAD, |total, (bytes, (min, max))| {
                if !(min..=max).contains(&bytes.len()) {
                    return Err(refused());
                }
                total.checked_add(bytes.len()).ok_or_else(refused)
            })
    }
}

pub(super) fn id_data(raw: &[u8]) -> ArchiveHeadId {
    ArchiveHeadId::from_bytes(blake3::derive_key(ID_DOMAIN, raw))
}

pub(super) fn seal_data(
    key: &[u8; 32],
    labels: GovernedArchiveLabels,
    frames: &GovernedArchiveFrames<'_>,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let size = frames.frame_bytes()?;
    if labels.epoch == 0 {
        return Err(refused());
    }
    let mut e = Encoder::with_capacity(size - 40);
    put(&mut e, TAG)?;
    e.put_u16(VERSION);
    for field in [
        labels.context.as_bytes(),
        labels.before_head.as_bytes(),
        labels.before_archive.as_bytes(),
        labels.header.as_bytes(),
        labels.state.as_bytes(),
        &labels.authenticator,
    ] {
        put(&mut e, field)?;
    }
    e.put_u64(labels.epoch);
    for field in frames.ordered() {
        put(&mut e, field)?;
    }
    let plain = Zeroizing::new(e.finish());
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let raw = Zeroizing::new(frame(&seal(&key, &plain, rng)?));
    if raw.len() != size {
        return Err(refused());
    }
    Ok(raw)
}

pub(super) fn open_data(
    key: &[u8; 32],
    id: ArchiveHeadId,
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(OVERHEAD..=MAX_BYTES).contains(&raw.len()) || id_data(raw) != id {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let plain = Zeroizing::new(unseal(&key, &unframe(raw)?)?);
    decode_data(&plain)?;
    Ok(plain)
}

pub(super) fn decode_data(
    plain: &[u8],
) -> Result<(GovernedArchiveLabels, GovernedArchiveFrames<'_>), AppError> {
    let mut d = Decoder::new(plain);
    if field(&mut d, TAG.len())? != TAG || d.get_u16().map_err(|_| refused())? != VERSION {
        return Err(refused());
    }
    let labels = GovernedArchiveLabels {
        context: TextContextIdData::from_bytes(b32(&mut d)?),
        before_head: TextHeadIdData::from_bytes(b32(&mut d)?),
        before_archive: ArchiveHeadId::from_bytes(b32(&mut d)?),
        header: OwnerHeaderId::from_bytes(b32(&mut d)?),
        state: OwnerStateCommitment::from_bytes(b32(&mut d)?),
        authenticator: b32(&mut d)?,
        epoch: d.get_u64().map_err(|_| refused())?,
    };
    let mut frames = [&[][..]; BODY_CAPS.len()];
    for (body, (min, max)) in frames.iter_mut().zip(BODY_CAPS) {
        *body = field(&mut d, max)?;
        if body.len() < min {
            return Err(refused());
        }
    }
    d.finish().map_err(|_| refused())?;
    if labels.epoch == 0 {
        return Err(refused());
    }
    let [parent_head, parent_archive, fence, request, entry, outcome, header_attempt, raw_header, governing, welcome_intent, welcome_attempt, raw_welcome, response] =
        frames;
    Ok((
        labels,
        GovernedArchiveFrames {
            parent_head,
            parent_archive,
            fence,
            request,
            entry,
            outcome,
            header_attempt,
            raw_header,
            governing,
            welcome_intent,
            welcome_attempt,
            raw_welcome,
            response,
        },
    ))
}

#[cfg(test)]
mod tests;
