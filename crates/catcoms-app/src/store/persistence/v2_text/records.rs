//! Exact signature/publication control DATA. Parsed raw sigma is not signing admission,
//! and a publication record is not eligible without the original actual SavedB selection.
use super::common::*;
use super::control::{PreparedTextControlFrameData, TextControlKindData};
use super::request::TextRetainIdData;
use crate::AppError;
use catcoms_replication::chat::cold_index::ReceiptDigestData;
use catcoms_replication::chat::cold_manifest::ArchiveHeadId;
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_replication::chat::ActionId;
use catcoms_rt::CryptoRngCore;
use catcoms_wire::Encoder;
use zeroize::Zeroizing;

data_id!(
    TextIntentIdData,
    b"catcoms/private-chat-text-intent-id/v1\0"
);
data_id!(
    TextRawSignatureIdData,
    b"catcoms/private-chat-text-signature-id/v1\0"
);
const SIGNATURE_TAG: &[u8] = b"catcoms/private-chat-text-signature/v1\0";
const PUBLICATION_TAG: &[u8] = b"catcoms/private-chat-text-publication/v1\0";
pub(in crate::store) const MAX_SIGNATURE_PLAIN: usize = 365;
pub(in crate::store) const MAX_PUBLICATION_PLAIN: usize = 271;
const _: () = assert!(4 + SIGNATURE_TAG.len() + 2 + 7 * 36 + 4 + 64 == MAX_SIGNATURE_PLAIN);
const _: () = assert!(4 + PUBLICATION_TAG.len() + 2 + 6 * 36 + 8 == MAX_PUBLICATION_PLAIN);

#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct RawSignatureRecordData {
    pub(in crate::store) context: TextContextIdData,
    pub(in crate::store) transaction: TextTransactionIdData,
    pub(in crate::store) intent: TextIntentIdData,
    pub(in crate::store) pre_intent_head: TextHeadIdData,
    pub(in crate::store) public_parent: OwnerHeaderId,
    pub(in crate::store) unsigned_commitment: [u8; 32],
    pub(in crate::store) action: ActionId,
    pub(in crate::store) raw: [u8; 64],
}
impl std::fmt::Debug for RawSignatureRecordData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawSignatureRecordData")
            .finish_non_exhaustive()
    }
}
impl RawSignatureRecordData {
    pub(in crate::store) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::with_capacity(MAX_SIGNATURE_PLAIN);
        tag(&mut e, SIGNATURE_TAG)?;
        for b in [
            self.context.as_bytes(),
            self.transaction.as_bytes(),
            self.intent.as_bytes(),
            self.pre_intent_head.as_bytes(),
            self.public_parent.as_bytes(),
            &self.unsigned_commitment,
            self.action.as_bytes(),
        ] {
            put(&mut e, b)?;
        }
        put(&mut e, &self.raw)?;
        finish(e, MAX_SIGNATURE_PLAIN)
    }
    pub(in crate::store) fn decode_data(raw: &[u8]) -> Result<Self, AppError> {
        let mut d = input(raw, MAX_SIGNATURE_PLAIN, SIGNATURE_TAG)?;
        let result = Self {
            context: TextContextIdData::from_bytes(b32(&mut d)?),
            transaction: TextTransactionIdData::from_bytes(b32(&mut d)?),
            intent: TextIntentIdData::from_bytes(b32(&mut d)?),
            pre_intent_head: TextHeadIdData::from_bytes(b32(&mut d)?),
            public_parent: OwnerHeaderId::from_bytes(b32(&mut d)?),
            unsigned_commitment: b32(&mut d)?,
            action: ActionId::from_bytes(b32(&mut d)?),
            raw: field(&mut d, 64)?.try_into().map_err(|_| refused())?,
        };
        d.finish().map_err(|_| refused())?;
        Ok(result)
    }
    /// Immutable candidate only; the independently prepaid real transaction retains these
    /// exact bytes on ambiguity. Neither sealing nor its ID selects O_S or renews a signer.
    pub(in crate::store) fn prepare_frame_data(
        &self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<PreparedTextControlFrameData, AppError> {
        PreparedTextControlFrameData::prepare_data(
            TextControlKindData::Signature,
            key,
            &self.encode_data()?,
            rng,
        )
    }
}
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct PublicationRecordData {
    pub(in crate::store) context: TextContextIdData,
    pub(in crate::store) transaction: TextTransactionIdData,
    pub(in crate::store) retain: TextRetainIdData,
    pub(in crate::store) public_header: OwnerHeaderId,
    pub(in crate::store) receipt: ReceiptDigestData,
    pub(in crate::store) archive: ArchiveHeadId,
    pub(in crate::store) ordinal: u64,
}
impl std::fmt::Debug for PublicationRecordData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublicationRecordData")
            .finish_non_exhaustive()
    }
}
impl PublicationRecordData {
    pub(in crate::store) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::with_capacity(MAX_PUBLICATION_PLAIN);
        tag(&mut e, PUBLICATION_TAG)?;
        for b in [
            self.context.as_bytes(),
            self.transaction.as_bytes(),
            self.retain.as_bytes(),
            self.public_header.as_bytes(),
            self.receipt.as_bytes(),
            self.archive.as_bytes(),
        ] {
            put(&mut e, b)?;
        }
        e.put_u64(self.ordinal);
        finish(e, MAX_PUBLICATION_PLAIN)
    }
    pub(in crate::store) fn decode_data(raw: &[u8]) -> Result<Self, AppError> {
        let mut d = input(raw, MAX_PUBLICATION_PLAIN, PUBLICATION_TAG)?;
        let result = Self {
            context: TextContextIdData::from_bytes(b32(&mut d)?),
            transaction: TextTransactionIdData::from_bytes(b32(&mut d)?),
            retain: TextRetainIdData::from_bytes(b32(&mut d)?),
            public_header: OwnerHeaderId::from_bytes(b32(&mut d)?),
            receipt: ReceiptDigestData::from_bytes(b32(&mut d)?),
            archive: ArchiveHeadId::from_bytes(b32(&mut d)?),
            ordinal: d.get_u64().map_err(|_| refused())?,
        };
        d.finish().map_err(|_| refused())?;
        Ok(result)
    }
    pub(in crate::store) fn prepare_frame_data(
        &self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<PreparedTextControlFrameData, AppError> {
        PreparedTextControlFrameData::prepare_data(
            TextControlKindData::Publication,
            key,
            &self.encode_data()?,
            rng,
        )
    }
}

#[cfg(test)]
mod tests;
