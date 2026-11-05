//! Fixed-size DATA only. Hints/receipt hashes never establish actual historical authorization.
use super::*;
use crate::chat::acceptance_tree::{TreeContext, TreeHash};
use crate::chat::cold_actions::OriginalActionDigest;
use crate::chat::cold_manifest::ArchiveHeadId;
use crate::chat::owner_header::{OwnerBodyId, OwnerHeaderId};
use crate::chat::{ActionId, MAX_ACTION_BYTES};

const RECEIPT_DOMAIN: &[u8] = b"catcoms/private-chat-accepted-receipt/v1\0";
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReceiptDigestData([u8; 32]);
impl ReceiptDigestData {
    pub fn from_tuple(
        profile: OwnerFactId,
        admission: OwnerFactId,
        action: ActionId,
        record: [u8; 32],
        ordinal: u64,
        header: OwnerHeaderId,
    ) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(RECEIPT_DOMAIN);
        for bytes in [
            profile.as_bytes(),
            admission.as_bytes(),
            action.as_bytes(),
            &record,
        ] {
            h.update(bytes);
        }
        h.update(&ordinal.to_be_bytes());
        h.update(header.as_bytes());
        Self(*h.finalize().as_bytes())
    }
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
impl std::fmt::Debug for ReceiptDigestData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReceiptDigestData { .. }")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryTreeData {
    Unknown,
    Empty,
    Present(NodeRefData),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DirectoryData {
    pub tree: DirectoryTreeData,
    pub accepted_leaf_count: u64,
    pub unique_action_count: u64,
    pub accepted_root: TreeHash,
    pub own_tip_header: OwnerHeaderId,
    pub tip_body: OwnerBodyId,
    pub prefix_manifest: PrivateChatPageId,
    pub prefix_sequence: u64,
    pub prefix_head: ArchiveHeadId,
}
impl std::fmt::Debug for DirectoryData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectoryData")
            .field("tree", &self.tree)
            .field("leaves", &self.accepted_leaf_count)
            .field("unique", &self.unique_action_count)
            .finish_non_exhaustive()
    }
}
impl DirectoryData {
    fn validate(self, context: IndexContextData<'_>, key: [u8; 32]) -> Result<(), ChatError> {
        if context.scope != IndexScopeData::Directory
            || self.prefix_sequence == 0
            || self.unique_action_count > self.accepted_leaf_count
        {
            return Err(ChatError::Scope);
        }
        match self.tree {
            DirectoryTreeData::Unknown => {}
            DirectoryTreeData::Empty => {
                let empty = TreeContext {
                    profile: context.profile,
                    admission: OwnerFactId::from_bytes(key),
                }
                .empty_hash();
                if self.accepted_leaf_count != 0
                    || self.unique_action_count != 0
                    || self.accepted_root != empty
                    || self.tip_body.as_bytes() != &key
                {
                    return Err(ChatError::Scope);
                }
            }
            DirectoryTreeData::Present(root) => {
                root.summary.validate_data(NodePlacementData::Root)?;
                if root.summary.keys != self.unique_action_count || self.unique_action_count == 0 {
                    return Err(ChatError::Scope);
                }
            }
        }
        Ok(())
    }
    fn decode(context: IndexContextData<'_>, key: [u8; 32], raw: &[u8]) -> Result<Self, ChatError> {
        let mut p = Packed::new(raw);
        let tree = match p.u8()? {
            0 => DirectoryTreeData::Unknown,
            1 => DirectoryTreeData::Empty,
            2 => DirectoryTreeData::Present(NodeRefData {
                page: PrivateChatPageId::from_bytes(p.array()?),
                summary: NodeSummaryData {
                    height: p.u8()?,
                    keys: u64::try_from(p.u128()?).map_err(|_| ChatError::Bound)?,
                    min: p.array()?,
                    max: p.array()?,
                },
            }),
            _ => return Err(ChatError::Malformed),
        };
        let result = Self {
            tree,
            accepted_leaf_count: p.u64()?,
            unique_action_count: p.u64()?,
            accepted_root: TreeHash::from_bytes(p.array()?),
            own_tip_header: OwnerHeaderId::from_bytes(p.array()?),
            tip_body: OwnerBodyId::from_bytes(p.array()?),
            prefix_manifest: PrivateChatPageId::from_bytes(p.array()?),
            prefix_sequence: p.u64()?,
            prefix_head: ArchiveHeadId::from_bytes(p.array()?),
        };
        p.finish()?;
        result.validate(context, key)?;
        Ok(result)
    }
    fn encode(
        self,
        context: IndexContextData<'_>,
        key: [u8; 32],
    ) -> Result<Zeroizing<Vec<u8>>, ChatError> {
        self.validate(context, key)?;
        let mut e = Encoder::with_capacity(MAX_VALUE_BYTES);
        match self.tree {
            DirectoryTreeData::Unknown => {
                e.put_u8(0);
            }
            DirectoryTreeData::Empty => {
                e.put_u8(1);
            }
            DirectoryTreeData::Present(root) => {
                e.put_u8(2);
                raw(&mut e, root.page.as_bytes());
                e.put_u8(root.summary.height)
                    .put_u128(root.summary.keys as u128);
                raw(&mut e, &root.summary.min);
                raw(&mut e, &root.summary.max);
            }
        }
        e.put_u64(self.accepted_leaf_count)
            .put_u64(self.unique_action_count);
        for bytes in [
            self.accepted_root.as_bytes(),
            self.own_tip_header.as_bytes(),
            self.tip_body.as_bytes(),
            self.prefix_manifest.as_bytes(),
        ] {
            raw(&mut e, bytes);
        }
        e.put_u64(self.prefix_sequence);
        raw(&mut e, self.prefix_head.as_bytes());
        let bytes = e.finish();
        debug_assert!(bytes.len() <= 298);
        Ok(Zeroizing::new(bytes))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RetentionData {
    Unknown,
    Known {
        manifest: PrivateChatPageId,
        original: OriginalActionDigest,
        bytes: u32,
    },
}
impl std::fmt::Debug for RetentionData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => f.write_str("Unknown"),
            Self::Known { bytes, .. } => f
                .debug_struct("Known")
                .field("bytes", bytes)
                .finish_non_exhaustive(),
        }
    }
}
/// This hint never creates a historical/ordinary witness. Incomplete/conflict remain PRESENT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimHint {
    Canonical,
    Incomplete,
    Conflict,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ActionClaimData {
    pub record: [u8; 32],
    /// Earliest summarized CLAIM, including incomplete/conflicting summaries. Selecting a
    /// positive canonical valid alias still requires the actual historical source consumer.
    pub ordinal: u64,
    pub channel: u128,
    pub accepting_header: OwnerHeaderId,
    pub header_sequence: u64,
    pub append_body: OwnerBodyId,
    pub acceptance_manifest: PrivateChatPageId,
    pub manifest_sequence: u64,
    pub record_page: PrivateChatPageId,
    pub record_row: u8,
    pub header_body_page: PrivateChatPageId,
    pub retention: RetentionData,
    pub receipt: ReceiptDigestData,
    pub occurrences: u64,
    pub hint: ClaimHint,
}
impl std::fmt::Debug for ActionClaimData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionClaimData")
            .field("hint", &self.hint)
            .field("occurrences", &self.occurrences)
            .finish_non_exhaustive()
    }
}
impl ActionClaimData {
    fn validate(self, context: IndexContextData<'_>, key: [u8; 32]) -> Result<(), ChatError> {
        let IndexScopeData::Actions(admission) = context.scope else {
            return Err(ChatError::Scope);
        };
        if self.header_sequence == 0
            || self.manifest_sequence == 0
            || self.record_row >= 64
            || self.occurrences == 0
            || self.ordinal.checked_add(self.occurrences).is_none()
        {
            return Err(ChatError::Bound);
        }
        match self.retention {
            RetentionData::Known { bytes, .. }
                if bytes == 0 || bytes as usize > MAX_ACTION_BYTES =>
            {
                return Err(ChatError::Bound)
            }
            RetentionData::Unknown if self.hint == ClaimHint::Canonical => {
                return Err(ChatError::Scope)
            }
            _ => {}
        }
        if self.receipt
            != ReceiptDigestData::from_tuple(
                context.profile,
                admission,
                ActionId::from_bytes(key),
                self.record,
                self.ordinal,
                self.accepting_header,
            )
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
    pub(super) fn check_leaf_bound(self, leaves: u64) -> Result<(), ChatError> {
        if self
            .ordinal
            .checked_add(self.occurrences)
            .is_none_or(|end| end > leaves)
        {
            Err(ChatError::Bound)
        } else {
            Ok(())
        }
    }
    fn decode(
        context: IndexContextData<'_>,
        key: [u8; 32],
        bytes: &[u8],
    ) -> Result<Self, ChatError> {
        let mut p = Packed::new(bytes);
        let record = p.array()?;
        let ordinal = p.u64()?;
        let channel = p.u128()?;
        let accepting_header = OwnerHeaderId::from_bytes(p.array()?);
        let header_sequence = p.u64()?;
        let append_body = OwnerBodyId::from_bytes(p.array()?);
        let acceptance_manifest = PrivateChatPageId::from_bytes(p.array()?);
        let manifest_sequence = p.u64()?;
        let record_page = PrivateChatPageId::from_bytes(p.array()?);
        let record_row = p.u8()?;
        let header_body_page = PrivateChatPageId::from_bytes(p.array()?);
        let retention = match p.u8()? {
            0 => RetentionData::Unknown,
            1 => RetentionData::Known {
                manifest: PrivateChatPageId::from_bytes(p.array()?),
                original: OriginalActionDigest::from_bytes(p.array()?),
                bytes: p.u32()?,
            },
            _ => return Err(ChatError::Malformed),
        };
        let receipt = ReceiptDigestData(p.array()?);
        let occurrences = p.u64()?;
        let hint = match p.u8()? {
            1 => ClaimHint::Canonical,
            2 => ClaimHint::Incomplete,
            3 => ClaimHint::Conflict,
            _ => return Err(ChatError::Malformed),
        };
        p.finish()?;
        let result = Self {
            record,
            ordinal,
            channel,
            accepting_header,
            header_sequence,
            append_body,
            acceptance_manifest,
            manifest_sequence,
            record_page,
            record_row,
            header_body_page,
            retention,
            receipt,
            occurrences,
            hint,
        };
        result.validate(context, key)?;
        Ok(result)
    }
    fn encode(
        self,
        context: IndexContextData<'_>,
        key: [u8; 32],
    ) -> Result<Zeroizing<Vec<u8>>, ChatError> {
        self.validate(context, key)?;
        let mut e = Encoder::with_capacity(MAX_VALUE_BYTES);
        raw(&mut e, &self.record);
        e.put_u64(self.ordinal).put_u128(self.channel);
        raw(&mut e, self.accepting_header.as_bytes());
        e.put_u64(self.header_sequence);
        raw(&mut e, self.append_body.as_bytes());
        raw(&mut e, self.acceptance_manifest.as_bytes());
        e.put_u64(self.manifest_sequence);
        raw(&mut e, self.record_page.as_bytes());
        e.put_u8(self.record_row);
        raw(&mut e, self.header_body_page.as_bytes());
        match self.retention {
            RetentionData::Unknown => {
                e.put_u8(0);
            }
            RetentionData::Known {
                manifest,
                original,
                bytes,
            } => {
                e.put_u8(1);
                raw(&mut e, manifest.as_bytes());
                raw(&mut e, original.as_bytes());
                e.put_u32(bytes);
            }
        }
        raw(&mut e, self.receipt.as_bytes());
        e.put_u64(self.occurrences).put_u8(match self.hint {
            ClaimHint::Canonical => 1,
            ClaimHint::Incomplete => 2,
            ClaimHint::Conflict => 3,
        });
        let bytes = e.finish();
        debug_assert!(bytes.len() == 343 || bytes.len() == 275);
        Ok(Zeroizing::new(bytes))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueData {
    Directory(DirectoryData),
    Action(ActionClaimData),
}
impl ValueData {
    pub(super) fn decode(
        context: IndexContextData<'_>,
        key: [u8; 32],
        bytes: &[u8],
    ) -> Result<Self, ChatError> {
        if bytes.is_empty() || bytes.len() > MAX_VALUE_BYTES {
            return Err(ChatError::Bound);
        }
        match context.scope {
            IndexScopeData::Directory => {
                Ok(Self::Directory(DirectoryData::decode(context, key, bytes)?))
            }
            IndexScopeData::Actions(_) => {
                Ok(Self::Action(ActionClaimData::decode(context, key, bytes)?))
            }
        }
    }
    pub(super) fn encode(
        self,
        context: IndexContextData<'_>,
        key: [u8; 32],
    ) -> Result<Zeroizing<Vec<u8>>, ChatError> {
        match self {
            Self::Directory(v) => v.encode(context, key),
            Self::Action(v) => v.encode(context, key),
        }
    }
}
fn raw(e: &mut Encoder, bytes: &[u8]) {
    for b in bytes {
        e.put_u8(*b);
    }
}
