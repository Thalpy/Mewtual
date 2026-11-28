//! An action lookup points to the exact public header and a separate
//! encrypted receipt. The immutable header indexes keep their own roles.
use super::*;
use catcoms_replication::chat::ActionId;

const TAG: &[u8] = b"catcoms/private-chat-reviewed-original-index-value/v1\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-reviewed-original-action-key/v1";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct ApprovedReviewIndexValueData {
    pub(in crate::store) action: ActionId,
    pub(in crate::store) header: ReceivedReviewIndexValueData,
    pub(in crate::store) receipt: [u8; 32],
}

catcoms_rt::redacted_debug! { ApprovedReviewIndexValueData; }

impl ApprovedReviewIndexValueData {
    pub(in crate::store) fn action_key(cid: TextContextIdData, action: ActionId) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new_derive_key(KEY_DOMAIN);
        hasher.update(cid.as_bytes());
        hasher.update(action.as_bytes());
        *hasher.finalize().as_bytes()
    }

    pub(super) fn require_key(
        self,
        context: PrivateIndexContextData,
        expected: [u8; 32],
    ) -> Result<()> {
        if context.kind != PrivateIndexKindData::ReviewApprovedAction
            || Self::action_key(context.cid, self.action) != expected
        {
            return Err(PrivateIndexError::Context);
        }
        self.require_shape()
    }

    fn require_shape(self) -> Result<()> {
        if self.action.as_bytes() == &[0; 32] || self.receipt == [0; 32] {
            return Err(PrivateIndexError::Malformed);
        }
        Ok(())
    }

    pub(super) fn encode(self) -> Result<Zeroizing<Vec<u8>>> {
        self.require_shape()?;
        let header = self.header.encode()?;
        let width = 4 * 4 + TAG.len() + 2 * 32 + header.len();
        if width > MAX_VALUE_BYTES {
            return Err(PrivateIndexError::Bound);
        }
        let mut output = Encoder::with_capacity(width);
        for field in [TAG, self.action.as_bytes(), &header, &self.receipt] {
            output
                .put_bytes(field)
                .map_err(|_| PrivateIndexError::Bound)?;
        }
        Ok(Zeroizing::new(output.finish()))
    }

    pub(super) fn decode(raw: &[u8]) -> Result<Self> {
        if raw.is_empty() || raw.len() > MAX_VALUE_BYTES {
            return Err(PrivateIndexError::Bound);
        }
        let mut input = Decoder::new(raw);
        let invalid = |_| PrivateIndexError::Malformed;
        if input.get_bytes().map_err(invalid)? != TAG {
            return Err(PrivateIndexError::Malformed);
        }
        let action = ActionId::from_bytes(
            input
                .get_bytes()
                .map_err(invalid)?
                .try_into()
                .map_err(|_| PrivateIndexError::Malformed)?,
        );
        let header = ReceivedReviewIndexValueData::decode(input.get_bytes().map_err(invalid)?)?;
        let receipt = input
            .get_bytes()
            .map_err(invalid)?
            .try_into()
            .map_err(|_| PrivateIndexError::Malformed)?;
        input.finish().map_err(invalid)?;
        let row = Self {
            action,
            header,
            receipt,
        };
        row.require_shape()?;
        Ok(row)
    }
}
