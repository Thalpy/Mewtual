//! Sealed append records and a separate exact selector. These are data
//! until the producer proves the original head and covers both files.
use super::attempt::JournalKey;
use super::*;
use crate::store::persistence::v2_text::TextRootData;
use catcoms_replication::chat::{
    cold_index::{NodeRefData, MAX_INDEX_FRAME_BYTES},
    owner_bodies::{
        ExactReviewDecisionBody, ReviewStreamEntry, ReviewStreamRecord,
        MAX_REVIEW_STREAM_RECORD_BYTES,
    },
    FounderGenesisAnchorData,
};
use catcoms_wire::{Decoder, Encoder};

pub(super) mod append;
pub(super) mod disk;
pub(super) mod index;
#[cfg(test)]
pub(super) mod tests;

const RECORD: &[u8] = b"catcoms/private-chat-v2-review-journal-record/v1\0";
const HEAD: &[u8] = b"catcoms/private-chat-v2-review-journal-head/v1\0";
const INDEXED_HEAD: &[u8] = b"catcoms/private-chat-v2-review-journal-head/v2\0";
const WAL: &[u8] = b"catcoms/private-chat-v2-review-journal-selection/v1\0";
const WAL_FRAME: &[u8; 8] = b"CTV2RVJ\x01";
const INDEXED_WAL: &[u8] = b"catcoms/private-chat-v2-review-journal-selection/v2\0";
const INDEXED_WAL_FRAME: &[u8; 8] = b"CTV2RVJ\x02";
const WAL_HEADER: usize = WAL_FRAME.len() + 4;
const INTENT_DIGEST_DOMAIN: &str = "catcoms/private-chat-v2-review-journal-selected-intent/v1";
const ATTEMPT_DIGEST_DOMAIN: &str = "catcoms/private-chat-v2-review-journal-selected-attempt/v1";
const RAW_DIGEST_DOMAIN: &str = "catcoms/private-chat-v2-review-journal-raw-frame/v1";
const RECORD_DIGEST_DOMAIN: &str = "catcoms/private-chat-v2-review-journal-record-frame/v1";
const HEAD_DIGEST_DOMAIN: &str = "catcoms/private-chat-v2-review-journal-head-frame/v1";
const SEAL_BYTES: usize = 40;
const DIGEST_FIELD: usize = 4 + 32;
pub(super) const MAX_RECORD_BYTES: usize =
    SEAL_BYTES + 4 + RECORD.len() + 2 * DIGEST_FIELD + 4 + MAX_REVIEW_STREAM_RECORD_BYTES;
const LEGACY_HEAD_BYTES: usize = SEAL_BYTES + 4 + HEAD.len() + 8 + 3 * DIGEST_FIELD;
const INDEX_ROOT_BYTES: usize = 1 + 1 + 8 + 3 * DIGEST_FIELD;
pub(super) const HEAD_BYTES: usize = LEGACY_HEAD_BYTES + 2 * INDEX_ROOT_BYTES;
pub(super) const MAX_SELECTION_BYTES: usize = WAL_HEADER
    + SEAL_BYTES
    + 4
    + INDEXED_WAL.len()
    + 3 * DIGEST_FIELD
    + 5 * 4
    + 2 * HEAD_BYTES
    + MAX_RECORD_BYTES
    + 2 * MAX_INDEX_FRAME_BYTES;

pub(super) struct RetainedDecisionRecord<'a> {
    pub(super) request_digest: [u8; 32],
    raw_digest: [u8; 32],
    pub(super) record: ReviewStreamRecord<'a>,
    frame: &'a [u8],
}

impl<'a> RetainedDecisionRecord<'a> {
    pub(super) fn prepare(
        key: &JournalKey,
        founder: &FounderGenesisAnchorData,
        (entry, decision, held): (&ReviewStreamEntry, &ExactReviewDecisionBody, &[u8]),
        (request_digest, raw_digest): ([u8; 32], [u8; 32]),
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let record =
            ReviewStreamRecord::encode(founder, entry, decision, held).map_err(|_| denied())?;
        let mut encoded = Encoder::with_capacity(16 + RECORD.len() + 64 + record.len());
        for field in [RECORD, &request_digest, &raw_digest, &record] {
            encoded.put_bytes(field).map_err(|_| denied())?;
        }
        key.seal(encoded, rng)
    }

    pub(super) fn open(key: &JournalKey, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if !(SEAL_BYTES + 1..=MAX_RECORD_BYTES).contains(&sealed.len()) {
            return Err(denied());
        }
        key.open(sealed)
    }

    pub(super) fn decode(
        founder: &FounderGenesisAnchorData,
        plain: &'a [u8],
    ) -> Result<Self, AppError> {
        if plain.len() > MAX_RECORD_BYTES - SEAL_BYTES {
            return Err(denied());
        }
        let mut decoder = Decoder::new(plain);
        if read_field(&mut decoder)? != RECORD {
            return Err(denied());
        }
        let request_digest = digest(&mut decoder)?;
        let raw_digest = digest(&mut decoder)?;
        let frame = read_field(&mut decoder)?;
        decoder.finish().map_err(|_| denied())?;
        let record = ReviewStreamRecord::decode(founder, frame).map_err(|_| denied())?;
        Ok(Self {
            request_digest,
            raw_digest,
            record,
            frame,
        })
    }

    pub(super) fn verify_raw_predecessor(&self, digest: [u8; 32]) -> Result<(), AppError> {
        if self.raw_digest != digest {
            return Err(denied());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct DecisionHead {
    pub(super) sequence: u64,
    entry: [u8; 32],
    record: [u8; 32],
    previous_head: [u8; 32],
    pub(super) roots: Option<[NodeRefData; 2]>,
}

impl DecisionHead {
    /// The authenticated original previous head is required separately
    /// when installing this candidate in place of the actual current file.
    pub(super) fn from_record(
        entry: &ReviewStreamEntry,
        sealed_record: &[u8],
        previous_head: Option<(Self, &[u8])>,
        roots: [NodeRefData; 2],
    ) -> Result<Self, AppError> {
        let value = Self {
            sequence: entry.position_data().sequence,
            entry: *entry.id().as_bytes(),
            record: blake3::derive_key(RECORD_DIGEST_DOMAIN, sealed_record),
            previous_head: previous_head.map_or([0; 32], |(_, frame)| head_digest(frame)),
            roots: Some(roots),
        };
        value.require_shape()?;
        value.require_predecessor(entry, previous_head)?;
        Ok(value)
    }

    fn require_shape(&self) -> Result<(), AppError> {
        if self.sequence == 0
            || self.entry == [0; 32]
            || self.record == [0; 32]
            || (self.sequence == 1) != (self.previous_head == [0; 32])
        {
            return Err(denied());
        }
        if let Some([intent, case]) = self.roots {
            if intent.page == case.page
                || intent.summary.keys != self.sequence
                || case.summary.keys > self.sequence
            {
                return Err(denied());
            }
        }
        Ok(())
    }

    pub(super) fn seal(
        self,
        key: &JournalKey,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.require_shape()?;
        let mut encoder = Encoder::with_capacity(HEAD_BYTES - SEAL_BYTES);
        encoder.put_bytes(INDEXED_HEAD).map_err(|_| denied())?;
        encoder.put_u64(self.sequence);
        for field in [self.entry, self.record, self.previous_head] {
            encoder.put_bytes(&field).map_err(|_| denied())?;
        }
        for root in self.roots.ok_or_else(denied)? {
            TextRootData::Present(root).encode(&mut encoder)?;
        }
        key.seal(encoder, rng)
    }

    pub(super) fn open(key: &JournalKey, frame: &[u8]) -> Result<Self, AppError> {
        if !valid_head_size(frame) {
            return Err(denied());
        }
        let plain = key.open(frame)?;
        let mut decoder = Decoder::new(&plain);
        let version = read_field(&mut decoder)?;
        let mut value = Self {
            sequence: decoder.get_u64().map_err(|_| denied())?,
            entry: digest(&mut decoder)?,
            record: digest(&mut decoder)?,
            previous_head: digest(&mut decoder)?,
            roots: None,
        };
        match (version, frame.len()) {
            (tag, LEGACY_HEAD_BYTES) if tag == HEAD => (),
            (tag, HEAD_BYTES) if tag == INDEXED_HEAD => {
                let mut read_root = || match TextRootData::decode(&mut decoder)? {
                    TextRootData::Present(root) => Ok(root),
                    TextRootData::Empty => Err(denied()),
                };
                value.roots = Some([read_root()?, read_root()?]);
            }
            _ => return Err(denied()),
        }
        decoder.finish().map_err(|_| denied())?;
        value.require_shape()?;
        Ok(value)
    }

    pub(super) fn require_record(
        &self,
        entry: &ReviewStreamEntry,
        sealed_record: &[u8],
    ) -> Result<(), AppError> {
        if self.sequence != entry.position_data().sequence
            || &self.entry != entry.id().as_bytes()
            || self.record != blake3::derive_key(RECORD_DIGEST_DOMAIN, sealed_record)
        {
            return Err(denied());
        }
        Ok(())
    }

    pub(super) fn require_predecessor(
        &self,
        entry: &ReviewStreamEntry,
        previous: Option<(Self, &[u8])>,
    ) -> Result<(), AppError> {
        let position = entry.position_data();
        let expected = match previous {
            Some((previous, frame))
                if previous.sequence.checked_add(1) == Some(self.sequence)
                    && position
                        .previous
                        .is_some_and(|entry| entry.as_bytes() == &previous.entry) =>
            {
                head_digest(frame)
            }
            None if self.sequence == 1 && position.previous.is_none() => [0; 32],
            _ => return Err(denied()),
        };
        if expected != self.previous_head || position.sequence != self.sequence {
            return Err(denied());
        }
        Ok(())
    }
}

fn valid_head_size(frame: &[u8]) -> bool {
    matches!(frame.len(), LEGACY_HEAD_BYTES | HEAD_BYTES)
}

pub(super) fn head_digest(frame: &[u8]) -> [u8; 32] {
    blake3::derive_key(HEAD_DIGEST_DOMAIN, frame)
}

/// Freeze the encrypted outputs in one authenticated transaction. A
/// cold writer can finish exact partial writes without invoking the signer.
pub(super) struct DecisionSelection<'a> {
    controls: [[u8; 32]; 3],
    pub(super) previous_head: Option<&'a [u8]>,
    pub(super) record: &'a [u8],
    pub(super) head: &'a [u8],
    pub(super) pages: Option<[&'a [u8]; 2]>,
}

impl<'a> DecisionSelection<'a> {
    pub(super) fn seal(
        key: &JournalKey,
        controls: [[u8; 32]; 3],
        previous_head: Option<&[u8]>,
        record: &[u8],
        head: &[u8],
        pages: [&[u8]; 2],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let previous = previous_head.unwrap_or_default();
        if !(SEAL_BYTES + 1..=MAX_RECORD_BYTES).contains(&record.len())
            || head.len() != HEAD_BYTES
            || previous_head.is_some_and(|frame| !valid_head_size(frame))
            || pages
                .iter()
                .any(|page| page.is_empty() || page.len() > MAX_INDEX_FRAME_BYTES)
        {
            return Err(denied());
        }
        let mut plain = Encoder::with_capacity(
            36 + INDEXED_WAL.len()
                + 96
                + previous.len()
                + record.len()
                + head.len()
                + pages.iter().map(|page| page.len()).sum::<usize>(),
        );
        let [intent, attempt, raw] = controls;
        for field in [
            INDEXED_WAL,
            &intent,
            &attempt,
            &raw,
            previous,
            record,
            head,
            pages[0],
            pages[1],
        ] {
            plain.put_bytes(field).map_err(|_| denied())?;
        }
        let sealed = key.seal(plain, rng)?;
        let size = WAL_HEADER.checked_add(sealed.len()).ok_or_else(denied)?;
        let length = u32::try_from(size).map_err(|_| denied())?;
        let mut frame = Zeroizing::new(Vec::with_capacity(size));
        frame.extend_from_slice(INDEXED_WAL_FRAME);
        frame.extend_from_slice(&length.to_be_bytes());
        frame.extend_from_slice(&sealed);
        Ok(frame)
    }

    pub(super) fn open(key: &JournalKey, frame: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if Self::declared_size(frame) != Some(frame.len()) {
            return Err(denied());
        }
        let plain = key.open(&frame[WAL_HEADER..])?;
        let expected = if frame.starts_with(WAL_FRAME) {
            WAL
        } else {
            INDEXED_WAL
        };
        if read_field(&mut Decoder::new(&plain))? != expected {
            return Err(denied());
        }
        Ok(plain)
    }

    fn declared_size(frame: &[u8]) -> Option<usize> {
        let length = frame
            .strip_prefix(WAL_FRAME)
            .or_else(|| frame.strip_prefix(INDEXED_WAL_FRAME))?
            .get(..4)?
            .try_into()
            .ok()?;
        let length = u32::from_be_bytes(length) as usize;
        (WAL_HEADER + SEAL_BYTES + 1..=MAX_SELECTION_BYTES)
            .contains(&length)
            .then_some(length)
    }

    /// Only the unselected scratch role can recognize a truncated prefix.
    /// A complete corrupt frame cannot be silently replaced.
    pub(super) fn incomplete_scratch(frame: &[u8]) -> bool {
        if frame.len() < WAL_FRAME.len() {
            return WAL_FRAME.starts_with(frame) || INDEXED_WAL_FRAME.starts_with(frame);
        }
        if frame.len() < WAL_HEADER {
            return frame.starts_with(WAL_FRAME) || frame.starts_with(INDEXED_WAL_FRAME);
        }
        Self::declared_size(frame).is_some_and(|length| length > frame.len())
    }

    pub(super) fn decode(plain: &'a [u8]) -> Result<Self, AppError> {
        if plain.len() > MAX_SELECTION_BYTES - WAL_HEADER - SEAL_BYTES {
            return Err(denied());
        }
        let mut reader = Decoder::new(plain);
        let domain = read_field(&mut reader)?;
        if domain != WAL && domain != INDEXED_WAL {
            return Err(denied());
        }
        let controls = [
            digest(&mut reader)?,
            digest(&mut reader)?,
            digest(&mut reader)?,
        ];
        let previous = read_field(&mut reader)?;
        let record = read_field(&mut reader)?;
        let head = read_field(&mut reader)?;
        let pages = if domain == INDEXED_WAL {
            let pages = [read_field(&mut reader)?, read_field(&mut reader)?];
            if pages
                .iter()
                .any(|page| page.is_empty() || page.len() > MAX_INDEX_FRAME_BYTES)
            {
                return Err(denied());
            }
            Some(pages)
        } else {
            None
        };
        reader.finish().map_err(|_| denied())?;
        if !(SEAL_BYTES + 1..=MAX_RECORD_BYTES).contains(&record.len())
            || !valid_head_size(head)
            || (!previous.is_empty() && !valid_head_size(previous))
        {
            return Err(denied());
        }
        Ok(Self {
            controls,
            previous_head: (!previous.is_empty()).then_some(previous),
            record,
            head,
            pages,
        })
    }

    pub(super) fn require_controls(&self, supplied: [[u8; 32]; 3]) -> Result<(), AppError> {
        if supplied != self.controls {
            return Err(denied());
        }
        Ok(())
    }

    /// Only a reader holding the independently covered selected head can
    /// use this check. A pending append still needs all original inputs.
    pub(super) fn require_remaining_controls(
        &self,
        frames: [Option<&[u8]>; 3],
    ) -> Result<(), AppError> {
        require_remaining_controls(self.controls, frames)
    }

    /// Reprove the exact outputs against authenticated pinned data. The
    /// physical caller additionally matches the previous/current files
    /// by original identity and checks membership ancestry before effects.
    pub(super) fn verify_data(
        &self,
        key: &JournalKey,
        founder: &FounderGenesisAnchorData,
        expected: (&ReviewStreamEntry, [u8; 32]),
    ) -> Result<DecisionHead, AppError> {
        let selected = DecisionHead::open(key, self.head)?;
        let plain = RetainedDecisionRecord::open(key, self.record)?;
        let record = RetainedDecisionRecord::decode(founder, &plain)?;
        if record.request_digest != expected.1 || record.record.entry_data() != expected.0 {
            return Err(denied());
        }
        record.verify_raw_predecessor(self.controls[2])?;
        selected.require_record(record.record.entry_data(), self.record)?;
        match (selected.roots, self.pages) {
            (Some(roots), Some(pages)) => {
                key.indexes()
                    .require_first(roots, pages, &record, self.record)?;
            }
            (None, None) => (),
            _ => return Err(denied()),
        }
        let previous = self
            .previous_head
            .map(|bytes| DecisionHead::open(key, bytes).map(|head| (head, bytes)))
            .transpose()?;
        selected.require_predecessor(record.record.entry_data(), previous)?;
        Ok(selected)
    }
}

pub(super) fn control_digests([intent, attempt, raw]: [&[u8]; 3]) -> [[u8; 32]; 3] {
    [
        blake3::derive_key(INTENT_DIGEST_DOMAIN, intent),
        blake3::derive_key(ATTEMPT_DIGEST_DOMAIN, attempt),
        blake3::derive_key(RAW_DIGEST_DOMAIN, raw),
    ]
}

fn require_remaining_controls(
    controls: [[u8; 32]; 3],
    frames: [Option<&[u8]>; 3],
) -> Result<(), AppError> {
    let domains = [
        INTENT_DIGEST_DOMAIN,
        ATTEMPT_DIGEST_DOMAIN,
        RAW_DIGEST_DOMAIN,
    ];
    for ((frame, domain), expected) in frames.into_iter().zip(domains).zip(controls) {
        if frame.is_some_and(|frame| blake3::derive_key(domain, frame) != expected) {
            return Err(denied());
        }
    }
    Ok(())
}

fn read_field<'a>(decoder: &mut Decoder<'a>) -> Result<&'a [u8], AppError> {
    decoder.get_bytes().map_err(|_| denied())
}

fn digest(decoder: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    read_field(decoder)?.try_into().map_err(|_| denied())
}
