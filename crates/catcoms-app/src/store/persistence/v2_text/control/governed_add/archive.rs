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
mod removal;

const TAG: &[u8] = b"catcoms/private-chat-text-governed-add-archive/v2\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-text-governed-add-archive-key/v2";
const ID_DOMAIN: &str = "catcoms/private-chat-text-governed-add-archive-id/v2";
const VERSION: u16 = 2;
const LATER_TAG: &[u8] = b"catcoms/private-chat-text-governed-add-archive/v3\0";
const LATER_KEY_DOMAIN: &str = "catcoms/private-chat-text-governed-add-archive-key/v3";
const LATER_ID_DOMAIN: &str = "catcoms/private-chat-text-governed-add-archive-id/v3";
const LATER_VERSION: u16 = 3;
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
const LATER_OVERHEAD: usize = OVERHEAD - TAG.len() + LATER_TAG.len();
const LATER_BODY_CAPS: [(usize, usize); BODY_CAPS.len()] = {
    let mut caps = BODY_CAPS;
    caps[0] = (selected_head::FRAME_LEN, selected_head::FRAME_LEN);
    // The immutable predecessor archive stays in its own named file.
    caps[1] = (0, 0);
    caps
};
const fn max_bytes(overhead: usize, caps: &[(usize, usize)]) -> usize {
    let mut max = overhead;
    let mut i = 0;
    while i < caps.len() {
        max += caps[i].1;
        i += 1;
    }
    max
}
pub(super) const MAX_BYTES: usize = max_bytes(OVERHEAD, &BODY_CAPS);
const MAX_LATER_BYTES: usize = max_bytes(LATER_OVERHEAD, &LATER_BODY_CAPS);

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
        self.bytes_with_caps(OVERHEAD, BODY_CAPS)
    }

    fn bytes_with_caps(
        &self,
        overhead: usize,
        caps: [(usize, usize); BODY_CAPS.len()],
    ) -> Result<usize, AppError> {
        self.ordered()
            .iter()
            .zip(caps)
            .try_fold(overhead, |total, (bytes, (min, max))| {
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
    if labels.epoch != 1 {
        return Err(refused());
    }
    seal_archive(key, labels, frames, (TAG, VERSION, KEY_DOMAIN, size), rng)
}

/// Exact bytes to stage after independently validating the predecessor and
/// before covering the new archive and the selector under the same writer.
pub(super) struct LaterArchive {
    pub(super) archive: Zeroizing<Vec<u8>>,
    pub(super) head: Zeroizing<Vec<u8>>,
}

pub(super) fn seal_later_data(
    key: &[u8; 32],
    labels: GovernedArchiveLabels,
    frames: &GovernedArchiveFrames<'_>,
    rng: &mut dyn CryptoRngCore,
) -> Result<LaterArchive, AppError> {
    if labels.epoch < 2 {
        return Err(refused());
    }
    let size = frames.bytes_with_caps(LATER_OVERHEAD, LATER_BODY_CAPS)?;
    let archive = seal_archive(
        key,
        labels,
        frames,
        (LATER_TAG, LATER_VERSION, LATER_KEY_DOMAIN, size),
        rng,
    )?;
    let head = selected_head::GovernedHeadData::after_governed_archive(
        key,
        frames.parent_head,
        labels,
        later_id(&archive),
    )?
    .seal_later_data(key, rng)?;
    Ok(LaterArchive { archive, head })
}

pub(super) fn later_frame_bytes(frames: &GovernedArchiveFrames<'_>) -> Result<usize, AppError> {
    frames.bytes_with_caps(LATER_OVERHEAD, LATER_BODY_CAPS)
}

fn seal_archive(
    key: &[u8; 32],
    labels: GovernedArchiveLabels,
    frames: &GovernedArchiveFrames<'_>,
    (tag, version, domain, size): (&[u8], u16, &str, usize),
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let mut e = Encoder::with_capacity(size - 40);
    put(&mut e, tag)?;
    e.put_u16(version);
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
    let key = Zeroizing::new(blake3::derive_key(domain, key));
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

pub(super) fn later_id(raw: &[u8]) -> ArchiveHeadId {
    ArchiveHeadId::from_bytes(blake3::derive_key(LATER_ID_DOMAIN, raw))
}

fn open_later_data(
    key: &[u8; 32],
    head: &selected_head::GovernedHeadData,
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(LATER_OVERHEAD..=MAX_LATER_BYTES).contains(&raw.len()) || later_id(raw) != head.archive {
        return Err(refused());
    }
    let archive_key = Zeroizing::new(blake3::derive_key(LATER_KEY_DOMAIN, key));
    let plain = Zeroizing::new(unseal(&archive_key, &unframe(raw)?)?);
    let (labels, frames) = decode_later_data(&plain)?;
    let expected = selected_head::GovernedHeadData::after_governed_archive(
        key,
        frames.parent_head,
        labels,
        head.archive,
    )?;
    if expected != *head {
        return Err(refused());
    }
    Ok(plain)
}

pub(super) fn decode_data(
    plain: &[u8],
) -> Result<(GovernedArchiveLabels, GovernedArchiveFrames<'_>), AppError> {
    decode_archive(plain, (TAG, VERSION, BODY_CAPS))
}

fn decode_later_data(
    plain: &[u8],
) -> Result<(GovernedArchiveLabels, GovernedArchiveFrames<'_>), AppError> {
    decode_archive(plain, (LATER_TAG, LATER_VERSION, LATER_BODY_CAPS))
}

fn decode_archive<'a>(
    plain: &'a [u8],
    (tag, version, caps): (&[u8], u16, [(usize, usize); BODY_CAPS.len()]),
) -> Result<(GovernedArchiveLabels, GovernedArchiveFrames<'a>), AppError> {
    let mut d = Decoder::new(plain);
    if field(&mut d, tag.len())? != tag || d.get_u16().map_err(|_| refused())? != version {
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
    for (body, (min, max)) in frames.iter_mut().zip(caps) {
        *body = field(&mut d, max)?;
        if body.len() < min {
            return Err(refused());
        }
    }
    d.finish().map_err(|_| refused())?;
    if (version == VERSION && labels.epoch != 1) || (version == LATER_VERSION && labels.epoch < 2) {
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
