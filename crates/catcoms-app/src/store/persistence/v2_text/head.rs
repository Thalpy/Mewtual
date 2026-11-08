//! Outer operational selector O DATA. `previous` is ONLY a CAS commitment, never a retained
//! private-history chain. Actual selection and saved barriers require concrete physical work.
//! Actual Current/capture/promotion consumers are paired; decoding alone grants no effect.
use super::common::*;
use super::context::{TextContextData, MAX_CONTEXT_PLAIN};
use super::control::{PreparedTextControlFrameData, TextControlKindData};
use super::records::{TextIntentIdData, TextRawSignatureIdData};
use super::request::{PublicationValueData, TextRequestDigestData, TextRetainIdData};
use crate::AppError;
use catcoms_replication::chat::cold_index::{NodePlacementData, NodeRefData, NodeSummaryData};
use catcoms_replication::chat::cold_manifest::ArchiveHeadId;
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_rt::CryptoRngCore;
use catcoms_storage::private_chat_page::PrivateChatPageId;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;
mod capture_data;
pub(super) use capture_data::{index_capture_data, TextIndexCaptureRoleData};

const TAG: &[u8] = b"catcoms/private-chat-text-head/v1\0";
pub(in crate::store) const MAX_TEXT_HEAD_PLAIN: usize = 1685;
pub(in crate::store) const MAX_TEXT_HEAD_FRAME: usize = MAX_TEXT_HEAD_PLAIN + 40;
const _: () = assert!(
    4 + TAG.len() + 2 + 4 + MAX_CONTEXT_PLAIN + 8 + 37 + 36 + 3 * 118 + 16 + 363 + 32 + 8
        == MAX_TEXT_HEAD_PLAIN
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum TextParentData {
    Genesis(TextGenesisParentIdData),
    Operational(TextHeadIdData),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) enum TextRootData {
    Empty,
    Present(NodeRefData),
}
impl std::fmt::Debug for TextRootData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "Empty",
            Self::Present(_) => "Present { .. }",
        })
    }
}
impl TextRootData {
    fn decode(d: &mut Decoder<'_>) -> Result<Self, AppError> {
        match d.get_u8().map_err(|_| refused())? {
            0 => Ok(Self::Empty),
            1 => {
                let page = PrivateChatPageId::from_bytes(b32(d)?);
                let height = d.get_u8().map_err(|_| refused())?;
                let keys = d.get_u64().map_err(|_| refused())?;
                let min = b32(d)?;
                let max = b32(d)?;
                let summary = NodeSummaryData {
                    height,
                    keys,
                    min,
                    max,
                };
                summary
                    .validate_data(NodePlacementData::Root)
                    .map_err(|_| refused())?;
                Ok(Self::Present(NodeRefData { page, summary }))
            }
            _ => Err(refused()),
        }
    }
    fn encode(self, e: &mut Encoder) -> Result<(), AppError> {
        match self {
            Self::Empty => {
                e.put_u8(0);
            }
            Self::Present(n) => {
                n.summary
                    .validate_data(NodePlacementData::Root)
                    .map_err(|_| refused())?;
                e.put_u8(1);
                put(e, n.page.as_bytes())?;
                e.put_u8(n.summary.height).put_u64(n.summary.keys);
                put(e, &n.summary.min)?;
                put(e, &n.summary.max)?;
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct PendingTransactionData {
    pub(in crate::store) transaction: TextTransactionIdData,
    pub(in crate::store) token: [u8; 32],
    pub(in crate::store) request: TextRequestDigestData,
    pub(in crate::store) public_parent: OwnerHeaderId,
    pub(in crate::store) retain: TextRetainIdData,
    /// PRE-A capture in Retained, replaced with exact PRE-I O_A/H_A on Unsigned/Raw.
    pub(in crate::store) captured_head: TextHeadIdData,
    pub(in crate::store) captured_archive: ArchiveHeadId,
    pub(in crate::store) accepted_capture: [u8; 32],
}
impl std::fmt::Debug for PendingTransactionData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingTransactionData")
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum TextPendingData {
    None,
    Retained(PendingTransactionData),
    Unsigned {
        transaction: PendingTransactionData,
        intent: TextIntentIdData,
    },
    Raw {
        transaction: PendingTransactionData,
        intent: TextIntentIdData,
        signature: TextRawSignatureIdData,
    },
}
impl TextPendingData {
    pub(in crate::store) fn transaction_data(self) -> Option<PendingTransactionData> {
        match self {
            Self::None => None,
            Self::Retained(t)
            | Self::Unsigned { transaction: t, .. }
            | Self::Raw { transaction: t, .. } => Some(t),
        }
    }
    fn encode(self, e: &mut Encoder) -> Result<(), AppError> {
        let (kind, t, i, s) = match self {
            Self::None => {
                e.put_u8(0);
                return Ok(());
            }
            Self::Retained(t) => (1, t, None, None),
            Self::Unsigned {
                transaction,
                intent,
            } => (2, transaction, Some(intent), None),
            Self::Raw {
                transaction,
                intent,
                signature,
            } => (3, transaction, Some(intent), Some(signature)),
        };
        e.put_u8(kind);
        for b in [
            t.transaction.as_bytes(),
            &t.token,
            t.request.as_bytes(),
            t.public_parent.as_bytes(),
            t.retain.as_bytes(),
            t.captured_head.as_bytes(),
            t.captured_archive.as_bytes(),
            &t.accepted_capture,
        ] {
            put(e, b)?;
        }
        optional(e, i.map(|v| *v.as_bytes()))?;
        optional(e, s.map(|v| *v.as_bytes()))?;
        Ok(())
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, AppError> {
        let kind = d.get_u8().map_err(|_| refused())?;
        if kind == 0 {
            return Ok(Self::None);
        }
        if kind > 3 {
            return Err(refused());
        }
        let t = PendingTransactionData {
            transaction: TextTransactionIdData::from_bytes(b32(d)?),
            token: b32(d)?,
            request: TextRequestDigestData::from_bytes(b32(d)?),
            public_parent: OwnerHeaderId::from_bytes(b32(d)?),
            retain: TextRetainIdData::from_bytes(b32(d)?),
            captured_head: TextHeadIdData::from_bytes(b32(d)?),
            captured_archive: ArchiveHeadId::from_bytes(b32(d)?),
            accepted_capture: b32(d)?,
        };
        let i = read_optional(d)?;
        let s = read_optional(d)?;
        match (kind, i, s) {
            (1, None, None) => Ok(Self::Retained(t)),
            (2, Some(i), None) => Ok(Self::Unsigned {
                transaction: t,
                intent: TextIntentIdData::from_bytes(i),
            }),
            (3, Some(i), Some(s)) => Ok(Self::Raw {
                transaction: t,
                intent: TextIntentIdData::from_bytes(i),
                signature: TextRawSignatureIdData::from_bytes(s),
            }),
            _ => Err(refused()),
        }
    }
}
fn optional(e: &mut Encoder, value: Option<[u8; 32]>) -> Result<(), AppError> {
    if let Some(v) = value {
        e.put_u8(1);
        put(e, &v)?;
    } else {
        e.put_u8(0);
    }
    Ok(())
}
fn read_optional(d: &mut Decoder<'_>) -> Result<Option<[u8; 32]>, AppError> {
    match d.get_u8().map_err(|_| refused())? {
        0 => Ok(None),
        1 => Ok(Some(b32(d)?)),
        _ => Err(refused()),
    }
}

pub(in crate::store) struct TextHeadData {
    pub(in crate::store) context: TextContextData,
    pub(in crate::store) generation: u64,
    pub(in crate::store) previous: TextParentData,
    pub(in crate::store) archive: ArchiveHeadId,
    pub(in crate::store) directory: TextRootData,
    pub(in crate::store) tokens: TextRootData,
    pub(in crate::store) publications: TextRootData,
    pub(in crate::store) queue_front: u64,
    pub(in crate::store) queue_next: u64,
    pub(in crate::store) pending: TextPendingData,
    /// Immutable live-charge baseline; only actual matching terminal WC allows discount.
    pub(in crate::store) live_bytes: u128,
    pub(in crate::store) live_objects: u128,
    /// Monotonic selected-control label, never a reset/refund of source execution allowance.
    pub(in crate::store) selection_attempt: u64,
}
impl std::fmt::Debug for TextHeadData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextHeadData").finish_non_exhaustive()
    }
}
impl TextHeadData {
    pub(in crate::store) fn validate_data(&self) -> Result<(), AppError> {
        let c = self.context.spec_data();
        if self.generation == 0 || self.selection_attempt == 0 || self.queue_front > self.queue_next
        {
            return Err(refused());
        }
        match self.previous {
            TextParentData::Genesis(id)
                if self.generation == 1
                    && id == c.genesis_parent
                    && self.pending == TextPendingData::None
                    && self.tokens == TextRootData::Empty
                    && self.publications == TextRootData::Empty
                    && self.queue_next == 0 => {}
            TextParentData::Operational(_) if self.generation > 1 => {}
            _ => return Err(refused()),
        }
        let TextRootData::Present(directory) = self.directory else {
            return Err(refused());
        };
        directory
            .summary
            .validate_data(NodePlacementData::Root)
            .map_err(|_| refused())?;
        if directory.summary.height != 0
            || directory.summary.keys != 1
            || directory.summary.min != *c.admission.as_bytes()
            || directory.summary.max != *c.admission.as_bytes()
        {
            return Err(refused());
        }
        match self.publications {
            TextRootData::Empty if self.queue_next == 0 => {}
            TextRootData::Present(root) if self.queue_next > 0 => {
                root.summary
                    .validate_data(NodePlacementData::Root)
                    .map_err(|_| refused())?;
                if root.summary.keys != self.queue_next
                    || root.summary.min != PublicationValueData::ordinal_key_data(0)
                    || root.summary.max
                        != PublicationValueData::ordinal_key_data(self.queue_next - 1)
                {
                    return Err(refused());
                }
            }
            _ => return Err(refused()),
        }
        match self.tokens {
            TextRootData::Empty
                if self.pending == TextPendingData::None && self.queue_next == 0 => {}
            TextRootData::Present(root) => {
                root.summary
                    .validate_data(NodePlacementData::Root)
                    .map_err(|_| refused())?;
                if root.summary.keys < self.queue_next {
                    return Err(refused());
                }
            }
            _ => return Err(refused()),
        }
        if let Some(t) = self.pending.transaction_data() {
            if self.tokens == TextRootData::Empty
                || t.transaction
                    != TextTransactionIdData::from_request_data(
                        self.context.id_data()?,
                        t.token,
                        t.request,
                    )
            {
                return Err(refused());
            }
        }
        Ok(())
    }
    pub(in crate::store) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.validate_data()?;
        let context = self.context.encode_data()?;
        let mut e = Encoder::with_capacity(MAX_TEXT_HEAD_PLAIN);
        tag(&mut e, TAG)?;
        put(&mut e, &context)?;
        e.put_u64(self.generation);
        match self.previous {
            TextParentData::Genesis(id) => {
                e.put_u8(1);
                put(&mut e, id.as_bytes())?;
            }
            TextParentData::Operational(id) => {
                e.put_u8(2);
                put(&mut e, id.as_bytes())?;
            }
        }
        put(&mut e, self.archive.as_bytes())?;
        self.directory.encode(&mut e)?;
        self.tokens.encode(&mut e)?;
        self.publications.encode(&mut e)?;
        e.put_u64(self.queue_front).put_u64(self.queue_next);
        self.pending.encode(&mut e)?;
        e.put_u128(self.live_bytes)
            .put_u128(self.live_objects)
            .put_u64(self.selection_attempt);
        finish(e, MAX_TEXT_HEAD_PLAIN)
    }
    pub(in crate::store) fn decode_data(raw: &[u8]) -> Result<Self, AppError> {
        let mut d = input(raw, MAX_TEXT_HEAD_PLAIN, TAG)?;
        let context = field(&mut d, MAX_CONTEXT_PLAIN)?;
        let generation = d.get_u64().map_err(|_| refused())?;
        let previous = match d.get_u8().map_err(|_| refused())? {
            1 => TextParentData::Genesis(TextGenesisParentIdData::from_bytes(b32(&mut d)?)),
            2 => TextParentData::Operational(TextHeadIdData::from_bytes(b32(&mut d)?)),
            _ => return Err(refused()),
        };
        let archive = ArchiveHeadId::from_bytes(b32(&mut d)?);
        let directory = TextRootData::decode(&mut d)?;
        let tokens = TextRootData::decode(&mut d)?;
        let publications = TextRootData::decode(&mut d)?;
        let queue_front = d.get_u64().map_err(|_| refused())?;
        let queue_next = d.get_u64().map_err(|_| refused())?;
        let pending = TextPendingData::decode(&mut d)?;
        let live_bytes = d.get_u128().map_err(|_| refused())?;
        let live_objects = d.get_u128().map_err(|_| refused())?;
        let selection_attempt = d.get_u64().map_err(|_| refused())?;
        d.finish().map_err(|_| refused())?;
        let result = Self {
            context: TextContextData::decode_data(context)?,
            generation,
            previous,
            archive,
            directory,
            tokens,
            publications,
            queue_front,
            queue_next,
            pending,
            live_bytes,
            live_objects,
            selection_attempt,
        };
        result.validate_data()?;
        Ok(result)
    }
    pub(in crate::store) fn require_successor_data(
        &self,
        parent: &Self,
        parent_id: TextHeadIdData,
    ) -> Result<(), AppError> {
        self.validate_data()?;
        parent.validate_data()?;
        if self.context != parent.context
            || self.previous != TextParentData::Operational(parent_id)
            || self.generation != parent.generation.checked_add(1).ok_or_else(refused)?
            || self.selection_attempt
                != parent
                    .selection_attempt
                    .checked_add(1)
                    .ok_or_else(refused)?
        {
            return Err(refused());
        }
        // Operation-specific A/I/S/B/Stop/ACK exact roots, values and arithmetic are proved
        // by the actual typed candidate/selected-parent constructors, never this DATA check.
        Ok(())
    }
    pub(in crate::store) fn prepare_frame_data(
        &self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<PreparedTextControlFrameData, AppError> {
        PreparedTextControlFrameData::prepare_data(
            TextControlKindData::Head,
            key,
            &self.encode_data()?,
            rng,
        )
    }
}

#[cfg(test)]
mod tests;
