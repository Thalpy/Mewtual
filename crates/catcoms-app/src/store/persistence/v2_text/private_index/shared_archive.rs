//! Exact original locators in the independently selected received archive.
//! The raw ActionId is the tree key, so successor traversal retains its
//! natural total order. This DATA never supplies selected-source authority.
use super::*;
use catcoms_replication::chat::{owner_facts::OwnerFactId, ActionId};

const TAG: &[u8] = b"catcoms/private-chat-shared-archive-value/v1\0";
const BYTES: usize = 4 + TAG.len() + 5 * 36 + 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct SharedArchiveIndexValueData {
    pub(in crate::store) action: ActionId,
    pub(in crate::store) target: ActionId,
    pub(in crate::store) admission: OwnerFactId,
    pub(in crate::store) manifest: PrivateChatPageId,
    pub(in crate::store) bundle_digest: [u8; 32],
    pub(in crate::store) bundle_bytes: u64,
}
catcoms_rt::redacted_debug! { SharedArchiveIndexValueData; }

impl SharedArchiveIndexValueData {
    pub(super) fn require_key(self, context: PrivateIndexContextData, key: [u8; 32]) -> Result<()> {
        if context.kind != PrivateIndexKindData::SharedArchiveAction
            || self.action.as_bytes() != &key
            || self.manifest.as_bytes() == &[0; 32]
            || self.admission.as_bytes() == &[0; 32]
            || self.bundle_bytes == 0
        {
            return Err(PrivateIndexError::Context);
        }
        Ok(())
    }

    pub(super) fn encode(self) -> Result<Zeroizing<Vec<u8>>> {
        let mut e = Encoder::with_capacity(BYTES);
        for value in [
            TAG,
            self.action.as_bytes(),
            self.target.as_bytes(),
            self.admission.as_bytes(),
            self.manifest.as_bytes(),
            &self.bundle_digest,
        ] {
            e.put_bytes(value).map_err(|_| PrivateIndexError::Bound)?;
        }
        e.put_u64(self.bundle_bytes);
        Ok(Zeroizing::new(e.finish()))
    }

    pub(super) fn decode(raw: &[u8]) -> Result<Self> {
        if raw.len() != BYTES {
            return Err(PrivateIndexError::Malformed);
        }
        let mut d = Decoder::new(raw);
        if d.get_bytes().map_err(|_| PrivateIndexError::Malformed)? != TAG {
            return Err(PrivateIndexError::Malformed);
        }
        let value = Self {
            action: ActionId::from_bytes(b32(&mut d)?),
            target: ActionId::from_bytes(b32(&mut d)?),
            admission: OwnerFactId::from_bytes(b32(&mut d)?),
            manifest: PrivateChatPageId::from_bytes(b32(&mut d)?),
            bundle_digest: b32(&mut d)?,
            bundle_bytes: d.get_u64().map_err(|_| PrivateIndexError::Malformed)?,
        };
        d.finish().map_err(|_| PrivateIndexError::Malformed)?;
        Ok(value)
    }
}

const _: () = assert!(BYTES <= MAX_VALUE_BYTES);
