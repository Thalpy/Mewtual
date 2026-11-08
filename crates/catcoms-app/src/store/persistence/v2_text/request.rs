//! Private paged token/publication value DATA with independent stable key domains.
//! A complete selected index and original Action verification are still required for a real
//! hit/miss; these values never certify a request, acceptance, Stop or publication eligibility.
use super::common::*;
use crate::AppError;
use catcoms_replication::chat::cold_actions::OriginalActionDigest;
use catcoms_replication::chat::cold_index::ReceiptDigestData;
use catcoms_replication::chat::cold_records::MAX_RECORD_BATCH_ROWS;
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_replication::chat::{ActionId, MAX_TEXT_BYTES};
use catcoms_storage::private_chat_page::PrivateChatPageId;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

data_id!(
    TextRequestDigestData,
    b"catcoms/private-chat-text-request/v1\0"
);
data_id!(
    TextTokenKeyData,
    b"catcoms/private-chat-text-token-key/v1\0"
);
data_id!(
    TextRetainIdData,
    b"catcoms/private-chat-text-retain-id/v1\0"
);
data_id!(
    TextPublicationIdData,
    b"catcoms/private-chat-text-publication-id/v1\0"
);
pub(in crate::store) const MAX_TOKEN_VALUE_BYTES: usize = 336;
pub(in crate::store) const MAX_PUBLICATION_VALUE_BYTES: usize = 181;
const _: () = assert!(1 + 6 * 36 + 1 + 3 * 36 + 2 + 8 == MAX_TOKEN_VALUE_BYTES);
const _: () = assert!(5 * 36 + 1 == MAX_PUBLICATION_VALUE_BYTES);

fn hash_b32(h: &mut blake3::Hasher, field: &[u8; 32]) {
    h.update(&32u32.to_be_bytes());
    h.update(field);
}
impl TextRequestDigestData {
    pub(in crate::store) fn from_request_data(
        context: TextContextIdData,
        token: [u8; 32],
        channel: u128,
        text: &str,
    ) -> Result<Self, AppError> {
        if text.is_empty() || text.len() > MAX_TEXT_BYTES {
            return Err(refused());
        }
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/private-chat-text-request/v1\0");
        hash_b32(&mut h, context.as_bytes());
        hash_b32(&mut h, &token);
        h.update(&channel.to_be_bytes());
        h.update(&(text.len() as u32).to_be_bytes());
        h.update(text.as_bytes());
        Ok(Self::from_bytes(*h.finalize().as_bytes()))
    }
}
impl TextTokenKeyData {
    pub(in crate::store) fn from_request_data(context: TextContextIdData, token: [u8; 32]) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/private-chat-text-token-key/v1\0");
        hash_b32(&mut h, context.as_bytes());
        hash_b32(&mut h, &token);
        Self::from_bytes(*h.finalize().as_bytes())
    }
}
impl TextTransactionIdData {
    pub(in crate::store) fn from_request_data(
        context: TextContextIdData,
        token: [u8; 32],
        request: TextRequestDigestData,
    ) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/private-chat-text-transaction/v1\0");
        hash_b32(&mut h, context.as_bytes());
        hash_b32(&mut h, &token);
        hash_b32(&mut h, request.as_bytes());
        Self::from_bytes(*h.finalize().as_bytes())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct AcceptedLocatorData {
    pub(in crate::store) header: OwnerHeaderId,
    pub(in crate::store) record_page: PrivateChatPageId,
    pub(in crate::store) record_row: u16,
    pub(in crate::store) ordinal: u64,
    pub(in crate::store) receipt: ReceiptDigestData,
}
impl AcceptedLocatorData {
    fn validate_data(self) -> Result<(), AppError> {
        if usize::from(self.record_row) >= MAX_RECORD_BATCH_ROWS {
            return Err(refused());
        }
        // Actual page row-count/selected-tip ordinal/inclusion are unavailable at this
        // DATA layer. The complete selected-source consumer must prove them separately.
        Ok(())
    }
}
impl std::fmt::Debug for AcceptedLocatorData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcceptedLocatorData")
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) enum RequestPhaseData {
    Retained,
    Accepted(AcceptedLocatorData),
    Stopped,
}
impl std::fmt::Debug for RequestPhaseData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Retained => "Retained",
            Self::Accepted(_) => "Accepted { .. }",
            Self::Stopped => "Stopped",
        })
    }
}

/// The token value has no I/S phase: those private controls extend O.pendingTxn without
/// rewriting this tree. Accepted/Stopped are distinct selected-root effects; DATA is no issuer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct TokenValueData {
    pub(in crate::store) transaction: TextTransactionIdData,
    pub(in crate::store) token: [u8; 32],
    pub(in crate::store) request: TextRequestDigestData,
    pub(in crate::store) retain: TextRetainIdData,
    pub(in crate::store) action: ActionId,
    pub(in crate::store) original: OriginalActionDigest,
    pub(in crate::store) phase: RequestPhaseData,
}
impl std::fmt::Debug for TokenValueData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenValueData").finish_non_exhaustive()
    }
}
impl TokenValueData {
    pub(in crate::store) fn key_data(&self, context: TextContextIdData) -> TextTokenKeyData {
        TextTokenKeyData::from_request_data(context, self.token)
    }
    pub(in crate::store) fn check_request_data(
        &self,
        context: TextContextIdData,
        token: [u8; 32],
        channel: u128,
        text: &str,
    ) -> Result<(), AppError> {
        let expected = TextRequestDigestData::from_request_data(context, token, channel, text)?;
        if self.token != token
            || self.request != expected
            || self.transaction
                != TextTransactionIdData::from_request_data(context, token, expected)
        {
            return Err(refused());
        }
        // Exact selected original Action decoding/signature/channel/text checks STILL follow.
        Ok(())
    }
    pub(in crate::store) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if let RequestPhaseData::Accepted(a) = self.phase {
            a.validate_data()?;
        }
        let mut e = Encoder::with_capacity(MAX_TOKEN_VALUE_BYTES);
        e.put_u8(match self.phase {
            RequestPhaseData::Retained => 1,
            RequestPhaseData::Accepted(_) => 2,
            RequestPhaseData::Stopped => 3,
        });
        for b in [
            self.transaction.as_bytes(),
            &self.token,
            self.request.as_bytes(),
            self.retain.as_bytes(),
            self.action.as_bytes(),
            self.original.as_bytes(),
        ] {
            put(&mut e, b)?;
        }
        if let RequestPhaseData::Accepted(a) = self.phase {
            e.put_u8(1);
            put(&mut e, a.header.as_bytes())?;
            put(&mut e, a.record_page.as_bytes())?;
            e.put_u16(a.record_row).put_u64(a.ordinal);
            put(&mut e, a.receipt.as_bytes())?;
        } else {
            e.put_u8(0);
        }
        finish(e, MAX_TOKEN_VALUE_BYTES)
    }
    pub(in crate::store) fn decode_data(raw: &[u8]) -> Result<Self, AppError> {
        if raw.len() > MAX_TOKEN_VALUE_BYTES {
            return Err(refused());
        }
        let mut d = Decoder::new(raw);
        let phase = d.get_u8().map_err(|_| refused())?;
        let transaction = TextTransactionIdData::from_bytes(b32(&mut d)?);
        let token = b32(&mut d)?;
        let request = TextRequestDigestData::from_bytes(b32(&mut d)?);
        let retain = TextRetainIdData::from_bytes(b32(&mut d)?);
        let action = ActionId::from_bytes(b32(&mut d)?);
        let original = OriginalActionDigest::from_bytes(b32(&mut d)?);
        let locator = match d.get_u8().map_err(|_| refused())? {
            0 => None,
            1 => Some(AcceptedLocatorData {
                header: OwnerHeaderId::from_bytes(b32(&mut d)?),
                record_page: PrivateChatPageId::from_bytes(b32(&mut d)?),
                record_row: d.get_u16().map_err(|_| refused())?,
                ordinal: d.get_u64().map_err(|_| refused())?,
                receipt: ReceiptDigestData::from_bytes(b32(&mut d)?),
            }),
            _ => return Err(refused()),
        };
        d.finish().map_err(|_| refused())?;
        let phase = match (phase, locator) {
            (1, None) => RequestPhaseData::Retained,
            (2, Some(a)) => RequestPhaseData::Accepted(a),
            (3, None) => RequestPhaseData::Stopped,
            _ => return Err(refused()),
        };
        if let RequestPhaseData::Accepted(a) = phase {
            a.validate_data()?;
        }
        Ok(Self {
            transaction,
            token,
            request,
            retain,
            action,
            original,
            phase,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum PublicationPhaseData {
    Eligible,
    Acknowledged,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct PublicationValueData {
    pub(in crate::store) transaction: TextTransactionIdData,
    pub(in crate::store) action: ActionId,
    pub(in crate::store) receipt: ReceiptDigestData,
    pub(in crate::store) header: OwnerHeaderId,
    pub(in crate::store) publication: TextPublicationIdData,
    pub(in crate::store) phase: PublicationPhaseData,
}
impl std::fmt::Debug for PublicationValueData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublicationValueData")
            .finish_non_exhaustive()
    }
}
impl PublicationValueData {
    pub(in crate::store) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::with_capacity(MAX_PUBLICATION_VALUE_BYTES);
        for b in [
            self.transaction.as_bytes(),
            self.action.as_bytes(),
            self.receipt.as_bytes(),
            self.header.as_bytes(),
            self.publication.as_bytes(),
        ] {
            put(&mut e, b)?;
        }
        e.put_u8(match self.phase {
            PublicationPhaseData::Eligible => 1,
            PublicationPhaseData::Acknowledged => 2,
        });
        finish(e, MAX_PUBLICATION_VALUE_BYTES)
    }
    pub(in crate::store) fn decode_data(raw: &[u8]) -> Result<Self, AppError> {
        if raw.len() > MAX_PUBLICATION_VALUE_BYTES {
            return Err(refused());
        }
        let mut d = Decoder::new(raw);
        let transaction = TextTransactionIdData::from_bytes(b32(&mut d)?);
        let action = ActionId::from_bytes(b32(&mut d)?);
        let receipt = ReceiptDigestData::from_bytes(b32(&mut d)?);
        let header = OwnerHeaderId::from_bytes(b32(&mut d)?);
        let publication = TextPublicationIdData::from_bytes(b32(&mut d)?);
        let phase = match d.get_u8().map_err(|_| refused())? {
            1 => PublicationPhaseData::Eligible,
            2 => PublicationPhaseData::Acknowledged,
            _ => return Err(refused()),
        };
        d.finish().map_err(|_| refused())?;
        Ok(Self {
            transaction,
            action,
            receipt,
            header,
            publication,
            phase,
        })
    }
    pub(in crate::store) fn ordinal_key_data(ordinal: u64) -> [u8; 32] {
        let mut key = [0; 32];
        key[24..].copy_from_slice(&ordinal.to_be_bytes());
        key
    }
    pub(in crate::store) fn ordinal_from_key_data(key: [u8; 32]) -> Result<u64, AppError> {
        if key[..24].iter().any(|b| *b != 0) {
            return Err(refused());
        }
        Ok(u64::from_be_bytes(
            key[24..].try_into().map_err(|_| refused())?,
        ))
    }
}

#[cfg(test)]
mod tests;
