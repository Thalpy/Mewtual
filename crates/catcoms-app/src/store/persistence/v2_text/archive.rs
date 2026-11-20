//! Immutable public-history head H. Private mutable lookup/token/publication roots belong
//! only to O, so Directory.prefix_head=H and manifest.source_head=previous H stay acyclic.
//! Signature/image consistency here is DATA, not selected chain, scope or host persistence.
use super::common::*;
use super::control::{PreparedTextControlFrameData, TextControlKindData};
use crate::AppError;
use catcoms_replication::chat::cold_manifest::ArchiveHeadId;
use catcoms_replication::chat::owner_header::{HeaderKind, SignedOwnerHeader};
use catcoms_replication::chat::owner_state::StateFrame;
use catcoms_rt::CryptoRngCore;
use catcoms_storage::private_chat_page::PrivateChatPageId;
use catcoms_wire::Encoder;
use zeroize::Zeroizing;
const TAG: &[u8] = b"catcoms/private-chat-text-archive/v1\0";
pub(in crate::store) const MAX_ARCHIVE_PLAIN: usize = 1479;
const _: () = assert!(4 + TAG.len() + 2 + 36 + 36 + 8 + 638 + 680 + 37 + 1 == MAX_ARCHIVE_PLAIN);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum TextArchiveOperationData {
    Genesis,
    RetainAction,
    Append,
}
pub(in crate::store) struct TextArchiveSpecData<'a> {
    pub(in crate::store) context: TextContextIdData,
    pub(in crate::store) manifest: PrivateChatPageId,
    pub(in crate::store) manifest_sequence: u64,
    pub(in crate::store) header: &'a SignedOwnerHeader,
    pub(in crate::store) image: &'a [u8],
    pub(in crate::store) previous: Option<ArchiveHeadId>,
    pub(in crate::store) operation: TextArchiveOperationData,
}
impl std::fmt::Debug for TextArchiveSpecData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextArchiveSpecData")
            .finish_non_exhaustive()
    }
}
pub(in crate::store) struct TextArchiveData {
    context: TextContextIdData,
    manifest: PrivateChatPageId,
    manifest_sequence: u64,
    header: SignedOwnerHeader,
    image: Zeroizing<Vec<u8>>,
    previous: Option<ArchiveHeadId>,
    operation: TextArchiveOperationData,
}
impl std::fmt::Debug for TextArchiveData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextArchiveData").finish_non_exhaustive()
    }
}
impl TextArchiveData {
    pub(in crate::store) fn from_data(s: TextArchiveSpecData<'_>) -> Result<Self, AppError> {
        validate(&s)?;
        Ok(Self {
            context: s.context,
            manifest: s.manifest,
            manifest_sequence: s.manifest_sequence,
            header: s.header.clone(),
            image: Zeroizing::new(s.image.to_vec()),
            previous: s.previous,
            operation: s.operation,
        })
    }
    pub(in crate::store) fn spec_data(&self) -> TextArchiveSpecData<'_> {
        TextArchiveSpecData {
            context: self.context,
            manifest: self.manifest,
            manifest_sequence: self.manifest_sequence,
            header: &self.header,
            image: &self.image,
            previous: self.previous,
            operation: self.operation,
        }
    }
    pub(in crate::store) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        validate(&self.spec_data())?;
        let header = self.header.encode();
        if header.len() > 634 {
            return Err(refused());
        }
        let mut e = Encoder::with_capacity(MAX_ARCHIVE_PLAIN);
        tag(&mut e, TAG)?;
        put(&mut e, self.context.as_bytes())?;
        put(&mut e, self.manifest.as_bytes())?;
        e.put_u64(self.manifest_sequence);
        put(&mut e, &header)?;
        put(&mut e, &self.image)?;
        if let Some(id) = self.previous {
            e.put_u8(1);
            put(&mut e, id.as_bytes())?;
        } else {
            e.put_u8(0);
        }
        e.put_u8(match self.operation {
            TextArchiveOperationData::Genesis => 1,
            TextArchiveOperationData::RetainAction => 2,
            TextArchiveOperationData::Append => 3,
        });
        finish(e, MAX_ARCHIVE_PLAIN)
    }
    pub(in crate::store) fn decode_data(raw: &[u8]) -> Result<Self, AppError> {
        let mut d = input(raw, MAX_ARCHIVE_PLAIN, TAG)?;
        let context = TextContextIdData::from_bytes(b32(&mut d)?);
        let manifest = PrivateChatPageId::from_bytes(b32(&mut d)?);
        let manifest_sequence = d.get_u64().map_err(|_| refused())?;
        let header = field(&mut d, 634)?;
        let image = field(&mut d, 676)?;
        let previous = match d.get_u8().map_err(|_| refused())? {
            0 => None,
            1 => Some(ArchiveHeadId::from_bytes(b32(&mut d)?)),
            _ => return Err(refused()),
        };
        let operation = match d.get_u8().map_err(|_| refused())? {
            1 => TextArchiveOperationData::Genesis,
            2 => TextArchiveOperationData::RetainAction,
            3 => TextArchiveOperationData::Append,
            _ => return Err(refused()),
        };
        d.finish().map_err(|_| refused())?;
        // Complete borrowed cap/framing/EOF before the public header's bounded owning decode.
        let header = SignedOwnerHeader::decode(header).map_err(|_| refused())?;
        Self::from_data(TextArchiveSpecData {
            context,
            manifest,
            manifest_sequence,
            header: &header,
            image,
            previous,
            operation,
        })
    }
    pub(in crate::store) fn require_successor_data(
        &self,
        parent: &Self,
        parent_id: ArchiveHeadId,
    ) -> Result<(), AppError> {
        if self.context != parent.context
            || self.previous != Some(parent_id)
            || self.manifest_sequence
                != parent
                    .manifest_sequence
                    .checked_add(1)
                    .ok_or_else(refused)?
        {
            return Err(refused());
        }
        match self.operation {
            TextArchiveOperationData::Genesis => Err(refused()),
            TextArchiveOperationData::RetainAction
                if self.header == parent.header && self.image == parent.image =>
            {
                Ok(())
            }
            TextArchiveOperationData::Append => {
                let before = StateFrame::decode(&parent.image).map_err(|_| refused())?;
                let after = StateFrame::decode(&self.image).map_err(|_| refused())?;
                let prior = before.meta();
                let next = after.meta();
                let prior_slot = before.slots().next().ok_or_else(refused)?;
                let next_slot = after.slots().next().ok_or_else(refused)?;
                if self.header.spec().parent != parent.header.id()
                    || self.header.spec().sequence
                        != parent
                            .header
                            .spec()
                            .sequence
                            .checked_add(1)
                            .ok_or_else(refused)?
                    || after.meta().previous != before.commitment()
                    || prior.group != next.group
                    || prior.profile != next.profile
                    || prior.issuer != next.issuer
                    || prior.tenure != next.tenure
                    || prior_slot.admission != next_slot.admission
                    || prior_slot.subject != next_slot.subject
                {
                    return Err(refused());
                }
                // Actual body math, accepted index and governing/source selection remain
                // the owned Core verifier + actual saved-B host adapter's obligations.
                Ok(())
            }
            _ => Err(refused()),
        }
    }
    pub(in crate::store) fn prepare_frame_data(
        &self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<PreparedTextControlFrameData, AppError> {
        PreparedTextControlFrameData::prepare_data(
            TextControlKindData::Archive,
            key,
            &self.encode_data()?,
            rng,
        )
    }
}
fn validate(s: &TextArchiveSpecData<'_>) -> Result<(), AppError> {
    if s.image.len() > 676 || s.manifest_sequence == 0 {
        return Err(refused());
    }
    let image = StateFrame::decode(s.image).map_err(|_| refused())?;
    image
        .verify_header_binding(s.header)
        .map_err(|_| refused())?;
    if image.slots().len() != 1
        || image.meta().mls_epoch != 0
        || !matches!(
            (s.header.spec().kind, s.header.spec().sequence),
            (HeaderKind::Admission, 1) | (HeaderKind::AcceptanceAppend, 2..)
        )
    {
        return Err(refused());
    }
    match s.operation {
        TextArchiveOperationData::Genesis
            if s.manifest_sequence == 1
                && s.previous.is_none()
                && s.header.spec().sequence == 1
                && s.header.spec().kind == HeaderKind::Admission =>
        {
            Ok(())
        }
        TextArchiveOperationData::RetainAction
            if s.manifest_sequence > 1 && s.previous.is_some() =>
        {
            Ok(())
        }
        TextArchiveOperationData::Append
            if s.manifest_sequence > 1
                && s.previous.is_some()
                && s.header.spec().kind == HeaderKind::AcceptanceAppend =>
        {
            Ok(())
        }
        _ => Err(refused()),
    }
}

#[cfg(test)]
mod tests;
