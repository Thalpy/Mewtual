//! Point rows for the exact signed public header selected by ordinal or
//! as the latest decision for a case. No founder intent appears here.
use super::*;
use catcoms_replication::chat::owner_bodies::ReviewCaseId;

const DOMAIN: &[u8] = b"catcoms/private-chat-review-received-index-value/v1\0";
const ORDINAL_KEY: &str = "catcoms/private-chat-review-received-ordinal-key/v1";
const CASE_KEY: &str = "catcoms/private-chat-review-received-case-key/v1";
const VALUE_BYTES: usize = 4 + DOMAIN.len() + 8 + 3 * 36;
const _: () = assert!(VALUE_BYTES <= MAX_VALUE_BYTES);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct ReceivedReviewIndexValueData {
    pub(in crate::store) sequence: u64,
    pub(in crate::store) case: ReviewCaseId,
    pub(in crate::store) entry: [u8; 32],
    pub(in crate::store) frame: [u8; 32],
}

catcoms_rt::redacted_debug! { ReceivedReviewIndexValueData; }

impl ReceivedReviewIndexValueData {
    pub(in crate::store) fn ordinal_key(cid: TextContextIdData, sequence: u64) -> [u8; 32] {
        key(ORDINAL_KEY, cid, &sequence.to_be_bytes())
    }

    pub(in crate::store) fn case_key(cid: TextContextIdData, case: ReviewCaseId) -> [u8; 32] {
        key(CASE_KEY, cid, case.as_bytes())
    }

    pub(super) fn require_key(
        self,
        context: PrivateIndexContextData,
        expected: [u8; 32],
    ) -> Result<()> {
        let actual = match context.kind {
            PrivateIndexKindData::ReviewReceivedOrdinal => {
                Self::ordinal_key(context.cid, self.sequence)
            }
            PrivateIndexKindData::ReviewReceivedCase => Self::case_key(context.cid, self.case),
            _ => return Err(PrivateIndexError::Context),
        };
        if actual != expected {
            return Err(PrivateIndexError::Context);
        }
        self.require_shape()
    }

    fn require_shape(self) -> Result<()> {
        if self.sequence == 0
            || self.case.as_bytes() == &[0; 32]
            || self.entry == [0; 32]
            || self.frame == [0; 32]
        {
            return Err(PrivateIndexError::Malformed);
        }
        Ok(())
    }

    pub(super) fn encode(self) -> Result<Zeroizing<Vec<u8>>> {
        self.require_shape()?;
        let mut output = Encoder::with_capacity(VALUE_BYTES);
        output
            .put_bytes(DOMAIN)
            .map_err(|_| PrivateIndexError::Bound)?
            .put_u64(self.sequence);
        for bytes in [self.case.as_bytes(), &self.entry, &self.frame] {
            output
                .put_bytes(bytes)
                .map_err(|_| PrivateIndexError::Bound)?;
        }
        Ok(Zeroizing::new(output.finish()))
    }

    pub(super) fn decode(raw: &[u8]) -> Result<Self> {
        if raw.len() != VALUE_BYTES {
            return Err(PrivateIndexError::Malformed);
        }
        let mut input = Decoder::new(raw);
        if input
            .get_bytes()
            .map_err(|_| PrivateIndexError::Malformed)?
            != DOMAIN
        {
            return Err(PrivateIndexError::Malformed);
        }
        let sequence = input.get_u64().map_err(|_| PrivateIndexError::Malformed)?;
        let mut digest = || {
            input
                .get_bytes()
                .map_err(|_| PrivateIndexError::Malformed)?
                .try_into()
                .map_err(|_| PrivateIndexError::Malformed)
        };
        let row = Self {
            sequence,
            case: ReviewCaseId::from_bytes(digest()?),
            entry: digest()?,
            frame: digest()?,
        };
        input.finish().map_err(|_| PrivateIndexError::Malformed)?;
        row.require_shape()?;
        Ok(row)
    }
}

fn key(domain: &str, cid: TextContextIdData, suffix: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(domain);
    hasher.update(cid.as_bytes());
    hasher.update(suffix);
    *hasher.finalize().as_bytes()
}
