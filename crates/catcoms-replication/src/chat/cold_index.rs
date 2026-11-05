//! Private index node/query DATA only. A self-selected expected root, valid ciphertext or a
//! structural miss cannot supply complete-prefix, Current, historical or native-read authority.
//! The actual Store/source producer, balanced writer, WAL/GC and namespace remain separate gates.
use super::owner_facts::OwnerFactId;
use super::{ChatError, MAX_GROUP_ID_BYTES};
use catcoms_storage::private_chat_page::PrivateChatPageId;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

mod query;
mod values;
pub use query::{
    open_query_data, ChildStepData, DirectoryHitData, DirectoryRouteData, IndexDataError,
    IndexQueryData, MissData, QueryRootData, SearchData,
};
pub use values::{
    ActionClaimData, ClaimHint, DirectoryData, DirectoryTreeData, ReceiptDigestData, RetentionData,
    ValueData,
};

const DOMAIN: &[u8] = b"catcoms/private-chat-accepted-index/v1\0";
const VERSION: u16 = 1;
pub const MAX_INDEX_NODE_BYTES: usize = 16_384;
pub const MAX_INDEX_FRAME_BYTES: usize = MAX_INDEX_NODE_BYTES + 86;
pub const MAX_LEAF_ROWS: usize = 32;
pub const MIN_CHILD_LEAF_ROWS: usize = 16;
pub const MAX_BRANCH_ROWS: usize = 192;
pub const MIN_CHILD_BRANCH_ROWS: usize = 96;
pub const MAX_INDEX_HEIGHT: u8 = 9;
pub const MAX_VALUE_BYTES: usize = 384;
const BRANCH_ROW_BYTES: usize = 80;
const HEADER_MAX: usize =
    4 + DOMAIN.len() + 2 + 36 + 4 + MAX_GROUP_ID_BYTES + 36 + 1 + 36 + 1 + 1 + 2 + 16 + 2 * 36 + 4;
const _: () = assert!(HEADER_MAX == 510);
const _: () = assert!(HEADER_MAX + MAX_BRANCH_ROWS * BRANCH_ROW_BYTES <= MAX_INDEX_NODE_BYTES);
const _: () =
    assert!(HEADER_MAX + MAX_LEAF_ROWS * (32 + 2 + MAX_VALUE_BYTES) <= MAX_INDEX_NODE_BYTES);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IndexScopeData {
    Directory,
    Actions(OwnerFactId),
}
impl std::fmt::Debug for IndexScopeData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Directory => "Directory",
            Self::Actions(_) => "Actions { .. }",
        })
    }
}
/// Untrusted context labels; equality is data binding, never root selection or membership.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct IndexContextData<'a> {
    pub vault: [u8; 32],
    pub group: &'a [u8],
    pub profile: OwnerFactId,
    pub scope: IndexScopeData,
}
impl std::fmt::Debug for IndexContextData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexContextData")
            .field("actions", &matches!(self.scope, IndexScopeData::Actions(_)))
            .finish_non_exhaustive()
    }
}
impl IndexContextData<'_> {
    fn validate(self) -> Result<(), ChatError> {
        if self.group.is_empty() || self.group.len() > MAX_GROUP_ID_BYTES {
            Err(ChatError::Bound)
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodePlacementData {
    Root,
    Child,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKindData {
    Leaf,
    Branch,
}

/// Fixed untrusted summary. Its constructor/decoder grants no complete subtree/source status.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NodeSummaryData {
    pub height: u8,
    pub keys: u64,
    pub min: [u8; 32],
    pub max: [u8; 32],
}
impl std::fmt::Debug for NodeSummaryData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeSummaryData")
            .field("height", &self.height)
            .field("keys", &self.keys)
            .finish_non_exhaustive()
    }
}
impl NodeSummaryData {
    pub fn validate_data(self, placement: NodePlacementData) -> Result<(), ChatError> {
        if self.height > MAX_INDEX_HEIGHT
            || self.keys == 0
            || self.min > self.max
            || !distance_at_least(self.min, self.max, self.keys.saturating_sub(1))
        {
            return Err(ChatError::Bound);
        }
        let minimum = if self.height == 0 {
            if placement == NodePlacementData::Child {
                MIN_CHILD_LEAF_ROWS as u128
            } else {
                1
            }
        } else {
            let children = if placement == NodePlacementData::Child {
                MIN_CHILD_BRANCH_ROWS
            } else {
                2
            };
            min_child_keys(self.height - 1)?
                .checked_mul(children as u128)
                .ok_or(ChatError::Bound)?
        };
        let mut maximum = MAX_LEAF_ROWS as u128;
        for _ in 0..self.height {
            maximum = maximum
                .checked_mul(MAX_BRANCH_ROWS as u128)
                .ok_or(ChatError::Bound)?;
        }
        if (self.keys as u128) < minimum || (self.keys as u128) > maximum {
            return Err(ChatError::Bound);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NodeRefData {
    pub page: PrivateChatPageId,
    pub summary: NodeSummaryData,
}
impl std::fmt::Debug for NodeRefData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeRefData")
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy)]
pub struct ChildRefData<'a> {
    pub context: IndexContextData<'a>,
    pub node: NodeRefData,
}
#[derive(Clone, Copy)]
pub struct LeafRowData {
    pub key: [u8; 32],
    pub value: ValueData,
}

/// Complete borrowed structural node. No full historical/row map or variable owning decode.
pub struct NodeFrame<'a> {
    context: IndexContextData<'a>,
    kind: NodeKindData,
    summary: NodeSummaryData,
    rows: usize,
    packed: &'a [u8],
}
impl std::fmt::Debug for NodeFrame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexNodeFrame")
            .field("kind", &self.kind)
            .field("summary", &self.summary)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}
impl<'a> NodeFrame<'a> {
    pub fn decode(raw: &'a [u8], placement: NodePlacementData) -> Result<Self, ChatError> {
        if raw.len() > MAX_INDEX_NODE_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(raw);
        if d.get_bytes()? != DOMAIN || d.get_u16()? != VERSION {
            return Err(ChatError::Malformed);
        }
        let vault = digest_d(&mut d)?;
        let group = d.get_bytes()?;
        let profile = OwnerFactId::from_bytes(digest_d(&mut d)?);
        let scope = match d.get_u8()? {
            1 => IndexScopeData::Directory,
            2 => IndexScopeData::Actions(OwnerFactId::from_bytes(digest_d(&mut d)?)),
            _ => return Err(ChatError::Malformed),
        };
        let kind = match d.get_u8()? {
            1 => NodeKindData::Leaf,
            2 => NodeKindData::Branch,
            _ => return Err(ChatError::Malformed),
        };
        let height = d.get_u8()?;
        let rows = usize::from(d.get_u16()?);
        let keys = u64::try_from(d.get_u128()?).map_err(|_| ChatError::Bound)?;
        let min = digest_d(&mut d)?;
        let max = digest_d(&mut d)?;
        let packed = d.get_bytes()?;
        d.finish()?;
        let context = IndexContextData {
            vault,
            group,
            profile,
            scope,
        };
        context.validate()?;
        let summary = NodeSummaryData {
            height,
            keys,
            min,
            max,
        };
        summary.validate_data(placement)?;
        let result = Self {
            context,
            kind,
            summary,
            rows,
            packed,
        };
        match kind {
            NodeKindData::Leaf => {
                if height != 0 || rows == 0 || rows > MAX_LEAF_ROWS || keys != rows as u64 {
                    return Err(ChatError::Bound);
                }
                // Complete every row's length framing before interpreting any variable value.
                let mut p = Packed::new(packed);
                for _ in 0..rows {
                    p.array::<32>()?;
                    let n = usize::from(p.u16()?);
                    if n == 0 || n > MAX_VALUE_BYTES {
                        return Err(ChatError::Bound);
                    }
                    p.take(n)?;
                }
                p.finish()?;
                let mut previous = None;
                for index in 0..rows {
                    let row = result.leaf_row(index)?;
                    if previous.is_some_and(|old| old >= row.key) {
                        return Err(ChatError::Malformed);
                    }
                    if (index == 0 && row.key != min) || (index + 1 == rows && row.key != max) {
                        return Err(ChatError::Scope);
                    }
                    previous = Some(row.key);
                }
            }
            NodeKindData::Branch => {
                let minimum = if placement == NodePlacementData::Child {
                    MIN_CHILD_BRANCH_ROWS
                } else {
                    2
                };
                if height == 0
                    || rows < minimum
                    || rows > MAX_BRANCH_ROWS
                    || packed.len() != rows * BRANCH_ROW_BYTES
                {
                    return Err(ChatError::Bound);
                }
                let mut previous = None;
                let mut sum = 0u128;
                for (index, row) in packed.chunks_exact(BRANCH_ROW_BYTES).enumerate() {
                    let mut p = Packed::new(row);
                    let lower = p.array::<32>()?;
                    let page = p.array::<32>()?;
                    let count = p.u128()?;
                    p.finish()?;
                    if count > u64::MAX as u128
                        || count < min_child_keys(height - 1)?
                        || count > max_child_keys(height - 1)?
                    {
                        return Err(ChatError::Bound);
                    }
                    if previous.is_some_and(|old| old >= lower)
                        || (index == 0 && lower != min)
                        || lower > max
                    {
                        return Err(ChatError::Scope);
                    }
                    if packed
                        .chunks_exact(BRANCH_ROW_BYTES)
                        .take(index)
                        .any(|old| old[32..64] == page)
                    {
                        return Err(ChatError::Malformed);
                    }
                    let (upper, needed) = if index + 1 < rows {
                        let next: [u8; 32] = packed
                            [(index + 1) * BRANCH_ROW_BYTES..(index + 1) * BRANCH_ROW_BYTES + 32]
                            .try_into()
                            .map_err(|_| ChatError::Malformed)?;
                        (next, count as u64) // exclusive next-lower: distance must cover count keys
                    } else {
                        (max, count as u64 - 1)
                    };
                    if !distance_at_least(lower, upper, needed) {
                        return Err(ChatError::Scope);
                    }
                    sum = sum
                        .checked_add(count)
                        .filter(|n| *n <= u64::MAX as u128)
                        .ok_or(ChatError::Bound)?;
                    previous = Some(lower);
                }
                if sum != keys as u128 {
                    return Err(ChatError::Scope);
                }
            }
        }
        Ok(result)
    }
    pub fn context(&self) -> IndexContextData<'a> {
        self.context
    }
    pub fn summary(&self) -> NodeSummaryData {
        self.summary
    }
    pub fn kind(&self) -> NodeKindData {
        self.kind
    }
    pub fn row_count(&self) -> usize {
        self.rows
    }
    pub fn leaf_row(&self, index: usize) -> Result<LeafRowData, ChatError> {
        if self.kind != NodeKindData::Leaf || index >= self.rows {
            return Err(ChatError::Bound);
        }
        let mut p = Packed::new(self.packed);
        for _ in 0..index {
            p.take(32)?;
            let n = usize::from(p.u16()?);
            p.take(n)?;
        }
        let key = p.array::<32>()?;
        let n = usize::from(p.u16()?);
        let value = ValueData::decode(self.context, key, p.take(n)?)?;
        Ok(LeafRowData { key, value })
    }
    pub(crate) fn branch_row(
        &self,
        index: usize,
    ) -> Result<([u8; 32], PrivateChatPageId, u64), ChatError> {
        if self.kind != NodeKindData::Branch || index >= self.rows {
            return Err(ChatError::Bound);
        }
        let mut p =
            Packed::new(&self.packed[index * BRANCH_ROW_BYTES..(index + 1) * BRANCH_ROW_BYTES]);
        Ok((
            p.array()?,
            PrivateChatPageId::from_bytes(p.array()?),
            u64::try_from(p.u128()?).map_err(|_| ChatError::Bound)?,
        ))
    }
}

pub fn encode_leaf_data(
    context: IndexContextData<'_>,
    rows: &[LeafRowData],
    placement: NodePlacementData,
) -> Result<Zeroizing<Vec<u8>>, ChatError> {
    context.validate()?;
    if rows.is_empty() || rows.len() > MAX_LEAF_ROWS {
        return Err(ChatError::Bound);
    }
    let summary = NodeSummaryData {
        height: 0,
        keys: rows.len() as u64,
        min: rows[0].key,
        max: rows[rows.len() - 1].key,
    };
    summary.validate_data(placement)?;
    let mut previous = None;
    let mut size = 0usize;
    for row in rows {
        if previous.is_some_and(|old| old >= row.key) {
            return Err(ChatError::Malformed);
        }
        let value = row.value.encode(context, row.key)?;
        size = size.checked_add(34 + value.len()).ok_or(ChatError::Bound)?;
        previous = Some(row.key);
    }
    let mut e = header(context, NodeKindData::Leaf, summary, rows.len(), size)?;
    for row in rows {
        let value = row.value.encode(context, row.key)?;
        for b in row.key {
            e.put_u8(b);
        }
        e.put_u16(value.len() as u16);
        for b in value.iter() {
            e.put_u8(*b);
        }
    }
    let output = Zeroizing::new(e.finish());
    NodeFrame::decode(&output, placement)?;
    Ok(output)
}
pub fn encode_branch_data(
    context: IndexContextData<'_>,
    children: &[ChildRefData<'_>],
    placement: NodePlacementData,
) -> Result<Zeroizing<Vec<u8>>, ChatError> {
    context.validate()?;
    let minimum = if placement == NodePlacementData::Child {
        MIN_CHILD_BRANCH_ROWS
    } else {
        2
    };
    if children.len() < minimum || children.len() > MAX_BRANCH_ROWS {
        return Err(ChatError::Bound);
    }
    let first = children[0].node.summary;
    let last = children[children.len() - 1].node.summary;
    let height = first
        .height
        .checked_add(1)
        .filter(|h| *h <= MAX_INDEX_HEIGHT)
        .ok_or(ChatError::Bound)?;
    let mut keys = 0u64;
    for (index, child) in children.iter().enumerate() {
        child.node.summary.validate_data(NodePlacementData::Child)?;
        if child.context != context || child.node.summary.height != first.height {
            return Err(ChatError::Scope);
        }
        if index != 0 && children[index - 1].node.summary.max >= child.node.summary.min {
            return Err(ChatError::Scope);
        }
        if children[..index]
            .iter()
            .any(|old| old.node.page == child.node.page)
        {
            return Err(ChatError::Malformed);
        }
        keys = keys
            .checked_add(child.node.summary.keys)
            .ok_or(ChatError::Bound)?;
    }
    let summary = NodeSummaryData {
        height,
        keys,
        min: first.min,
        max: last.max,
    };
    summary.validate_data(placement)?;
    let mut e = header(
        context,
        NodeKindData::Branch,
        summary,
        children.len(),
        children.len() * BRANCH_ROW_BYTES,
    )?;
    for child in children {
        for b in child.node.summary.min {
            e.put_u8(b);
        }
        for b in child.node.page.as_bytes() {
            e.put_u8(*b);
        }
        e.put_u128(child.node.summary.keys as u128);
    }
    let output = Zeroizing::new(e.finish());
    NodeFrame::decode(&output, placement)?;
    Ok(output)
}
fn header(
    context: IndexContextData<'_>,
    kind: NodeKindData,
    summary: NodeSummaryData,
    rows: usize,
    packed: usize,
) -> Result<Encoder, ChatError> {
    let mut e = Encoder::with_capacity(MAX_INDEX_NODE_BYTES);
    e.put_bytes(DOMAIN)?;
    e.put_u16(VERSION);
    e.put_bytes(&context.vault)?;
    e.put_bytes(context.group)?;
    e.put_bytes(context.profile.as_bytes())?;
    match context.scope {
        IndexScopeData::Directory => {
            e.put_u8(1);
        }
        IndexScopeData::Actions(a) => {
            e.put_u8(2);
            e.put_bytes(a.as_bytes())?;
        }
    }
    e.put_u8(if kind == NodeKindData::Leaf { 1 } else { 2 })
        .put_u8(summary.height)
        .put_u16(rows as u16)
        .put_u128(summary.keys as u128);
    e.put_bytes(&summary.min)?;
    e.put_bytes(&summary.max)?;
    e.put_u32(u32::try_from(packed).map_err(|_| ChatError::Bound)?);
    Ok(e)
}
fn min_child_keys(height: u8) -> Result<u128, ChatError> {
    if height > MAX_INDEX_HEIGHT {
        return Err(ChatError::Bound);
    }
    let mut n = MIN_CHILD_LEAF_ROWS as u128;
    for _ in 0..height {
        n = n
            .checked_mul(MIN_CHILD_BRANCH_ROWS as u128)
            .ok_or(ChatError::Bound)?;
    }
    Ok(n)
}
// Fixed256-bit subtraction, retaining only whether the distance can cover a u64 key count.
// Reject impossible separator/count labels without a big integer, allocation or history scan.
fn distance_at_least(min: [u8; 32], max: [u8; 32], needed: u64) -> bool {
    if min > max {
        return false;
    }
    let mut difference = [0u8; 32];
    let mut borrow = 0u16;
    for i in (0..32).rev() {
        let sub = min[i] as u16 + borrow;
        let value = max[i] as u16;
        difference[i] = value.wrapping_sub(sub) as u8;
        borrow = u16::from(value < sub);
    }
    difference[..24].iter().any(|b| *b != 0)
        || u64::from_be_bytes(difference[24..].try_into().expect("fixed difference")) >= needed
}
fn max_child_keys(height: u8) -> Result<u128, ChatError> {
    if height > MAX_INDEX_HEIGHT {
        return Err(ChatError::Bound);
    }
    let mut n = MAX_LEAF_ROWS as u128;
    for _ in 0..height {
        n = n
            .checked_mul(MAX_BRANCH_ROWS as u128)
            .ok_or(ChatError::Bound)?;
    }
    Ok(n.min(u64::MAX as u128))
}
fn digest_d(d: &mut Decoder<'_>) -> Result<[u8; 32], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}
struct Packed<'a> {
    bytes: &'a [u8],
}
impl<'a> Packed<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], ChatError> {
        if n > self.bytes.len() {
            return Err(ChatError::Malformed);
        }
        let (a, b) = self.bytes.split_at(n);
        self.bytes = b;
        Ok(a)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], ChatError> {
        self.take(N)?.try_into().map_err(|_| ChatError::Malformed)
    }
    fn u8(&mut self) -> Result<u8, ChatError> {
        Ok(self.array::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, ChatError> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, ChatError> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, ChatError> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn u128(&mut self) -> Result<u128, ChatError> {
        Ok(u128::from_be_bytes(self.array()?))
    }
    fn finish(self) -> Result<(), ChatError> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(ChatError::Malformed)
        }
    }
}

#[cfg(test)]
mod tests;
