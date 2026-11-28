//! Immutable lane selector for a group. Only the current vault manifest
//! can select one of these independently authenticated frames.
use super::{header::ReceiverKey, manifest, *};
use crate::store::persistence::v2_text::TextRootData;
use catcoms_replication::chat::{cold_index::NodeRefData, owner_bodies::ReviewStreamEntry};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};

const TAG: &[u8] = b"catcoms/private-chat-review-receiver-head/v1\0";
const APPROVAL_TAG: &[u8] = b"catcoms/private-chat-review-receiver-head/v2\0";
const SEAL_BYTES: usize = 40;
const ROOT_BYTES: usize = 1 + 1 + 8 + 3 * 36;
const PLAIN_BYTES: usize = 4 + TAG.len() + 8 + 3 * 36 + 2 * ROOT_BYTES;
pub(super) const HEAD_BYTES: usize = SEAL_BYTES + PLAIN_BYTES;
pub(super) const MAX_HEAD_BYTES: usize = HEAD_BYTES + ROOT_BYTES;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct LaneHead {
    pub(super) sequence: u64,
    pub(super) entry: [u8; 32],
    pub(super) ordinal: NodeRefData,
    pub(super) cases: NodeRefData,
    pub(super) approvals: Option<NodeRefData>,
    pub(super) predecessor: [u8; 32],
    pub(super) fork: [u8; 32],
}

impl LaneHead {
    pub(super) fn next(
        signed: &ReviewStreamEntry,
        selected: Option<(Self, &[u8])>,
        [ordinal, cases]: [NodeRefData; 2],
    ) -> Result<Self, AppError> {
        let position = signed.position_data();
        let predecessor = match selected {
            Some((old, sealed))
                if !old.is_frozen()
                    && old.sequence.checked_add(1) == Some(position.sequence)
                    && position
                        .previous
                        .is_some_and(|prev| prev.as_bytes() == &old.entry) =>
            {
                manifest::head_digest(sealed)
            }
            None if position.sequence == 1 && position.previous.is_none() => [0; 32],
            _ => return Err(denied()),
        };
        let head = Self {
            sequence: position.sequence,
            entry: *signed.id().as_bytes(),
            ordinal,
            cases,
            approvals: selected.and_then(|(old, _)| old.approvals),
            predecessor,
            fork: [0; 32],
        };
        head.require_shape()?;
        Ok(head)
    }

    pub(super) fn freeze(self, original: &[u8], pair: &[u8]) -> Result<Self, AppError> {
        if self.is_frozen() || !Self::valid_width(original.len()) || pair.is_empty() {
            return Err(denied());
        }
        let head = Self {
            predecessor: manifest::head_digest(original),
            fork: blake3::derive_key(
                "catcoms/private-chat-review-receiver-selected-fork/v1",
                pair,
            ),
            ..self
        };
        head.require_shape()?;
        Ok(head)
    }

    pub(super) fn is_frozen(&self) -> bool {
        self.fork != [0; 32]
    }

    /// Add one exact approved original while keeping both selected public
    /// indexes and the global stream sequence intact.
    pub(super) fn cover_approval(
        self,
        original: &[u8],
        approvals: NodeRefData,
    ) -> Result<Self, AppError> {
        let next_count = self
            .approvals
            .map_or(0, |root| root.summary.keys)
            .checked_add(1)
            .ok_or_else(denied)?;
        if self.is_frozen()
            || !Self::valid_width(original.len())
            || approvals.summary.keys != next_count
        {
            return Err(denied());
        }
        let head = Self {
            predecessor: manifest::head_digest(original),
            approvals: Some(approvals),
            ..self
        };
        head.require_shape()?;
        Ok(head)
    }

    pub(super) fn valid_width(width: usize) -> bool {
        width == HEAD_BYTES || width == MAX_HEAD_BYTES
    }

    pub(super) fn seal(
        &self,
        key: &ReceiverKey,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.require_shape()?;
        let tag = if self.approvals.is_some() {
            APPROVAL_TAG
        } else {
            TAG
        };
        let mut output = Encoder::with_capacity(
            PLAIN_BYTES + ROOT_BYTES * usize::from(self.approvals.is_some()),
        );
        output
            .put_bytes(tag)
            .map_err(|_| denied())?
            .put_u64(self.sequence);
        for field in [&self.entry, &self.predecessor, &self.fork] {
            output.put_bytes(field).map_err(|_| denied())?;
        }
        TextRootData::Present(self.ordinal).encode(&mut output)?;
        TextRootData::Present(self.cases).encode(&mut output)?;
        if let Some(approvals) = self.approvals {
            TextRootData::Present(approvals).encode(&mut output)?;
        }
        key.seal(output, rng)
    }

    pub(super) fn open(key: &ReceiverKey, raw: &[u8]) -> Result<Self, AppError> {
        if !Self::valid_width(raw.len()) {
            return Err(denied());
        }
        let plain = key.open(raw, MAX_HEAD_BYTES)?;
        let mut input = Decoder::new(&plain);
        let with_approvals = raw.len() == MAX_HEAD_BYTES;
        let tag = if with_approvals { APPROVAL_TAG } else { TAG };
        if input.get_bytes().map_err(|_| denied())? != tag {
            return Err(denied());
        }
        let sequence = input.get_u64().map_err(|_| denied())?;
        let mut digest = || {
            input
                .get_bytes()
                .map_err(|_| denied())?
                .try_into()
                .map_err(|_| denied())
        };
        let entry = digest()?;
        let predecessor = digest()?;
        let fork = digest()?;
        let mut root = || match TextRootData::decode(&mut input)? {
            TextRootData::Present(root) => Ok(root),
            TextRootData::Empty => Err(denied()),
        };
        let head = Self {
            sequence,
            entry,
            predecessor,
            fork,
            ordinal: root()?,
            cases: root()?,
            approvals: if with_approvals { Some(root()?) } else { None },
        };
        input.finish().map_err(|_| denied())?;
        head.require_shape()?;
        Ok(head)
    }

    fn require_shape(&self) -> Result<(), AppError> {
        if self.sequence == 0
            || self.entry == [0; 32]
            || (self.predecessor == [0; 32])
                != (self.sequence == 1 && !self.is_frozen() && self.approvals.is_none())
            || self.ordinal.page == self.cases.page
            || self.ordinal.summary.keys != self.sequence
            || !(1..=self.sequence).contains(&self.cases.summary.keys)
            || self.approvals.is_some_and(|root| {
                root.page == self.cases.page
                    || root.page == self.ordinal.page
                    || !(1..=self.cases.summary.keys).contains(&root.summary.keys)
            })
        {
            return Err(denied());
        }
        Ok(())
    }
}
