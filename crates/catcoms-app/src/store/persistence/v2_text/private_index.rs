//! Separate token/publication index DATA codecs and bounded COW geometry.
//!
//! Cid is a caller expectation, never source/Current/acceptance authority. These domains
//! are NOT accepted-index scopes and introduce no synthetic Admission. Actual selected O,
//! query/resource lifetime, complete request checks, IO, sealing and selector work remain
//! the real caller's separate obligations. Kept undeclared until that caller is paired.
use super::common::{TextContextIdData, TextTransactionIdData};
use super::request::{PublicationValueData, TokenValueData};
use catcoms_replication::chat::cold_index::{
    NodeKindData, NodePlacementData, NodeRefData, NodeSummaryData, MAX_BRANCH_ROWS,
    MAX_INDEX_FRAME_BYTES, MAX_INDEX_HEIGHT, MAX_INDEX_NODE_BYTES, MAX_LEAF_ROWS,
    MIN_CHILD_BRANCH_ROWS, MIN_CHILD_LEAF_ROWS,
};
use catcoms_storage::private_chat_page::PrivateChatPageId;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

mod path;
mod rewrite;
pub(in crate::store) use path::{
    PrivateIndexPathData, PRIVATE_INDEX_BOUNDARY_OPEN_BYTES, PRIVATE_INDEX_PATH_METADATA_BYTES,
};
pub(in crate::store) use rewrite::{
    rewrite_private_branch_data, rewrite_private_leaf_data, EncodedPrivateIndexNodeData,
    PrivateIndexBranchChildData, PrivateIndexRewriteData, PRIVATE_INDEX_REWRITE_METADATA_BYTES,
    PRIVATE_INDEX_REWRITE_TEMPORARY_BYTES,
};

const TOKEN_DOMAIN: &[u8] = b"catcoms/private-chat-text-token-index/v1\0";
const PUBLICATION_DOMAIN: &[u8] = b"catcoms/private-chat-text-publication-index/v1\0";
const VERSION: u16 = 1;
const BRANCH_ROW_BYTES: usize = 80;
const MAX_VALUE_BYTES: usize = 384;
const TOKEN_HEADER_BYTES: usize = 180;
const PUBLICATION_HEADER_BYTES: usize = 186;
const _: () = assert!(
    4 + TOKEN_DOMAIN.len() + 2 + 36 + 1 + 1 + 1 + 2 + 16 + 36 + 36 + 4 == TOKEN_HEADER_BYTES
);
const _: () = assert!(
    4 + PUBLICATION_DOMAIN.len() + 2 + 36 + 1 + 1 + 1 + 2 + 16 + 36 + 36 + 4
        == PUBLICATION_HEADER_BYTES
);
const _: () =
    assert!(PUBLICATION_HEADER_BYTES + MAX_BRANCH_ROWS * BRANCH_ROW_BYTES <= MAX_INDEX_NODE_BYTES);
const _: () = assert!(
    PUBLICATION_HEADER_BYTES + MAX_LEAF_ROWS * (34 + MAX_VALUE_BYTES) <= MAX_INDEX_NODE_BYTES
);
const _: () = assert!(MAX_INDEX_NODE_BYTES == 16384 && MAX_INDEX_FRAME_BYTES == 16470);

/// Fixed nonallocating refusal: untrusted lengths/prefixes cannot select a diagnostic size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum PrivateIndexError {
    Bound,
    Malformed,
    Context,
    Dependency,
    Authentication,
}
type Result<T> = std::result::Result<T, PrivateIndexError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum PrivateIndexKindData {
    Token,
    Publication,
}
impl PrivateIndexKindData {
    fn domain(self) -> &'static [u8] {
        match self {
            Self::Token => TOKEN_DOMAIN,
            Self::Publication => PUBLICATION_DOMAIN,
        }
    }
    fn code(self) -> u8 {
        match self {
            Self::Token => 1,
            Self::Publication => 2,
        }
    }
    fn header_bytes(self) -> usize {
        match self {
            Self::Token => TOKEN_HEADER_BYTES,
            Self::Publication => PUBLICATION_HEADER_BYTES,
        }
    }
}

/// Stable Cid/kind labels only. The same expected context is checked BEFORE any value is
/// parsed or any variable data copied; it never proves which physical root was selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) struct PrivateIndexContextData {
    pub(in crate::store) cid: TextContextIdData,
    pub(in crate::store) kind: PrivateIndexKindData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum PrivateIndexValueData {
    Token(TokenValueData),
    Publication(PublicationValueData),
}
impl PrivateIndexValueData {
    fn check_key(self, context: PrivateIndexContextData, key: [u8; 32]) -> Result<()> {
        match (context.kind, self) {
            (PrivateIndexKindData::Token, Self::Token(value)) => {
                if value.key_data(context.cid).as_bytes() != &key
                    || value.transaction
                        != TextTransactionIdData::from_request_data(
                            context.cid,
                            value.token,
                            value.request,
                        )
                {
                    return Err(PrivateIndexError::Context);
                }
                Ok(())
            }
            (PrivateIndexKindData::Publication, Self::Publication(_)) => {
                PublicationValueData::ordinal_from_key_data(key)
                    .map_err(|_| PrivateIndexError::Context)?;
                Ok(())
            }
            _ => Err(PrivateIndexError::Context),
        }
    }
    fn decode(context: PrivateIndexContextData, key: [u8; 32], raw: &[u8]) -> Result<Self> {
        let value = match context.kind {
            PrivateIndexKindData::Token => Self::Token(
                TokenValueData::decode_data(raw).map_err(|_| PrivateIndexError::Malformed)?,
            ),
            PrivateIndexKindData::Publication => Self::Publication(
                PublicationValueData::decode_data(raw).map_err(|_| PrivateIndexError::Malformed)?,
            ),
        };
        value.check_key(context, key)?;
        Ok(value)
    }
    fn encode(self, context: PrivateIndexContextData, key: [u8; 32]) -> Result<Zeroizing<Vec<u8>>> {
        self.check_key(context, key)?;
        let raw = match self {
            Self::Token(value) => value.encode_data(),
            Self::Publication(value) => value.encode_data(),
        }
        .map_err(|_| PrivateIndexError::Malformed)?;
        if raw.is_empty() || raw.len() > MAX_VALUE_BYTES {
            return Err(PrivateIndexError::Bound);
        }
        Ok(raw)
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::store) struct PrivateIndexLeafRowData {
    pub(in crate::store) key: [u8; 32],
    pub(in crate::store) value: PrivateIndexValueData,
}
#[derive(Clone, Copy, Debug)]
pub(in crate::store) struct PrivateIndexChildRefData {
    pub(in crate::store) context: PrivateIndexContextData,
    pub(in crate::store) node: NodeRefData,
}

/// Complete borrowed node. No map/history, heap allocation, IO or positive proof is made.
pub(in crate::store) struct PrivateIndexNodeFrame<'a> {
    context: PrivateIndexContextData,
    kind: NodeKindData,
    summary: NodeSummaryData,
    rows: usize,
    packed: &'a [u8],
}
impl std::fmt::Debug for PrivateIndexNodeFrame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivateIndexNodeFrame")
            .field("kind", &self.kind)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}
impl<'a> PrivateIndexNodeFrame<'a> {
    pub(in crate::store) fn decode(
        raw: &'a [u8],
        expected: PrivateIndexContextData,
        placement: NodePlacementData,
    ) -> Result<Self> {
        if raw.len() > MAX_INDEX_NODE_BYTES {
            return Err(PrivateIndexError::Bound);
        }
        let mut d = Decoder::new(raw);
        // Reject wrong domain/version/Cid/tree kind before even interpreting row counts.
        if d.get_bytes().map_err(|_| PrivateIndexError::Malformed)? != expected.kind.domain()
            || d.get_u16().map_err(|_| PrivateIndexError::Malformed)? != VERSION
            || b32(&mut d)? != *expected.cid.as_bytes()
            || d.get_u8().map_err(|_| PrivateIndexError::Malformed)? != expected.kind.code()
        {
            return Err(PrivateIndexError::Context);
        }
        let kind = match d.get_u8().map_err(|_| PrivateIndexError::Malformed)? {
            1 => NodeKindData::Leaf,
            2 => NodeKindData::Branch,
            _ => return Err(PrivateIndexError::Malformed),
        };
        let height = d.get_u8().map_err(|_| PrivateIndexError::Malformed)?;
        let rows = usize::from(d.get_u16().map_err(|_| PrivateIndexError::Malformed)?);
        let keys = u64::try_from(d.get_u128().map_err(|_| PrivateIndexError::Malformed)?)
            .map_err(|_| PrivateIndexError::Bound)?;
        let min = b32(&mut d)?;
        let max = b32(&mut d)?;
        if expected.kind == PrivateIndexKindData::Publication {
            PublicationValueData::ordinal_from_key_data(min)
                .map_err(|_| PrivateIndexError::Context)?;
            PublicationValueData::ordinal_from_key_data(max)
                .map_err(|_| PrivateIndexError::Context)?;
        }
        let packed = d.get_bytes().map_err(|_| PrivateIndexError::Malformed)?;
        d.finish().map_err(|_| PrivateIndexError::Malformed)?;
        let summary = NodeSummaryData {
            height,
            keys,
            min,
            max,
        };
        summary
            .validate_data(placement)
            .map_err(|_| PrivateIndexError::Bound)?;
        let node = Self {
            context: expected,
            kind,
            summary,
            rows,
            packed,
        };
        match kind {
            NodeKindData::Leaf => {
                if height != 0 || rows == 0 || rows > MAX_LEAF_ROWS || keys != rows as u64 {
                    return Err(PrivateIndexError::Bound);
                }
                // Finish all row framing/EOF before interpreting ANY variable value.
                let mut p = Packed::new(packed);
                for _ in 0..rows {
                    p.array::<32>()?;
                    let n = usize::from(p.u16()?);
                    if n == 0 || n > MAX_VALUE_BYTES {
                        return Err(PrivateIndexError::Bound);
                    }
                    p.take(n)?;
                }
                p.finish()?;
                let mut previous = None;
                for i in 0..rows {
                    let row = node.leaf_row(i)?;
                    if previous.is_some_and(|old| old >= row.key)
                        || (i == 0 && row.key != min)
                        || (i + 1 == rows && row.key != max)
                    {
                        return Err(PrivateIndexError::Context);
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
                    || packed.len()
                        != rows
                            .checked_mul(BRANCH_ROW_BYTES)
                            .ok_or(PrivateIndexError::Bound)?
                {
                    return Err(PrivateIndexError::Bound);
                }
                let mut previous = None;
                let mut sum = 0u64;
                for i in 0..rows {
                    let (lower, page, count) = node.branch_row(i)?;
                    if u128::from(count) < min_child_keys(height - 1)?
                        || u128::from(count) > max_child_keys(height - 1)?
                    {
                        return Err(PrivateIndexError::Bound);
                    }
                    if previous.is_some_and(|old| old >= lower)
                        || (i == 0 && lower != min)
                        || lower > max
                    {
                        return Err(PrivateIndexError::Context);
                    }
                    for j in 0..i {
                        if node.branch_row(j)?.1 == page {
                            return Err(PrivateIndexError::Malformed);
                        }
                    }
                    let (upper, needed) = if i + 1 < rows {
                        (node.branch_row(i + 1)?.0, count)
                    } else {
                        (max, count - 1)
                    };
                    if !distance_at_least(lower, upper, needed) {
                        return Err(PrivateIndexError::Context);
                    }
                    sum = sum.checked_add(count).ok_or(PrivateIndexError::Bound)?;
                    previous = Some(lower);
                }
                if sum != keys {
                    return Err(PrivateIndexError::Context);
                }
            }
        }
        Ok(node)
    }
    pub(in crate::store) fn context(&self) -> PrivateIndexContextData {
        self.context
    }
    pub(in crate::store) fn summary(&self) -> NodeSummaryData {
        self.summary
    }
    pub(in crate::store) fn kind(&self) -> NodeKindData {
        self.kind
    }
    pub(in crate::store) fn row_count(&self) -> usize {
        self.rows
    }
    fn packed_leaf_row(&self, index: usize) -> Result<&'a [u8]> {
        if self.kind != NodeKindData::Leaf || index >= self.rows {
            return Err(PrivateIndexError::Bound);
        }
        let mut p = Packed::new(self.packed);
        for _ in 0..index {
            p.take(32)?;
            let n = usize::from(p.u16()?);
            p.take(n)?;
        }
        let before = p.bytes;
        p.take(32)?;
        let n = usize::from(p.u16()?);
        p.take(n)?;
        Ok(&before[..34 + n])
    }
    pub(in crate::store) fn leaf_row(&self, index: usize) -> Result<PrivateIndexLeafRowData> {
        let mut p = Packed::new(self.packed_leaf_row(index)?);
        let key = p.array()?;
        let n = usize::from(p.u16()?);
        let value = PrivateIndexValueData::decode(self.context, key, p.take(n)?)?;
        p.finish()?;
        Ok(PrivateIndexLeafRowData { key, value })
    }
    fn branch_row(&self, index: usize) -> Result<([u8; 32], PrivateChatPageId, u64)> {
        if self.kind != NodeKindData::Branch || index >= self.rows {
            return Err(PrivateIndexError::Bound);
        }
        let mut p =
            Packed::new(&self.packed[index * BRANCH_ROW_BYTES..(index + 1) * BRANCH_ROW_BYTES]);
        Ok((
            p.array()?,
            PrivateChatPageId::from_bytes(p.array()?),
            u64::try_from(p.u128()?).map_err(|_| PrivateIndexError::Bound)?,
        ))
    }
}

pub(in crate::store) fn encode_private_leaf_data(
    context: PrivateIndexContextData,
    rows: &[PrivateIndexLeafRowData],
    placement: NodePlacementData,
) -> Result<Zeroizing<Vec<u8>>> {
    if rows.is_empty() || rows.len() > MAX_LEAF_ROWS {
        return Err(PrivateIndexError::Bound);
    }
    let summary = NodeSummaryData {
        height: 0,
        keys: rows.len() as u64,
        min: rows[0].key,
        max: rows[rows.len() - 1].key,
    };
    summary
        .validate_data(placement)
        .map_err(|_| PrivateIndexError::Bound)?;
    let mut size = 0usize;
    let mut previous = None;
    for row in rows {
        if previous.is_some_and(|old| old >= row.key) {
            return Err(PrivateIndexError::Malformed);
        }
        let value = row.value.encode(context, row.key)?;
        size = size
            .checked_add(34 + value.len())
            .ok_or(PrivateIndexError::Bound)?;
        previous = Some(row.key);
    }
    let mut e = header(context, NodeKindData::Leaf, summary, rows.len(), size)?;
    for row in rows {
        let raw = row.value.encode(context, row.key)?;
        for b in row.key {
            e.put_u8(b);
        }
        e.put_u16(u16::try_from(raw.len()).map_err(|_| PrivateIndexError::Bound)?);
        for b in raw.iter() {
            e.put_u8(*b);
        }
    }
    let raw = Zeroizing::new(e.finish());
    PrivateIndexNodeFrame::decode(&raw, context, placement)?;
    Ok(raw)
}
pub(in crate::store) fn encode_private_branch_data(
    context: PrivateIndexContextData,
    children: &[PrivateIndexChildRefData],
    placement: NodePlacementData,
) -> Result<Zeroizing<Vec<u8>>> {
    let minimum = if placement == NodePlacementData::Child {
        MIN_CHILD_BRANCH_ROWS
    } else {
        2
    };
    if children.len() < minimum || children.len() > MAX_BRANCH_ROWS {
        return Err(PrivateIndexError::Bound);
    }
    let first = children[0].node.summary;
    let last = children[children.len() - 1].node.summary;
    let height = first
        .height
        .checked_add(1)
        .filter(|h| *h <= MAX_INDEX_HEIGHT)
        .ok_or(PrivateIndexError::Bound)?;
    let mut keys = 0u64;
    for (i, child) in children.iter().enumerate() {
        child
            .node
            .summary
            .validate_data(NodePlacementData::Child)
            .map_err(|_| PrivateIndexError::Bound)?;
        if context.kind == PrivateIndexKindData::Publication {
            PublicationValueData::ordinal_from_key_data(child.node.summary.min)
                .map_err(|_| PrivateIndexError::Context)?;
            PublicationValueData::ordinal_from_key_data(child.node.summary.max)
                .map_err(|_| PrivateIndexError::Context)?;
        }
        if child.context != context
            || child.node.summary.height != first.height
            || (i > 0 && children[i - 1].node.summary.max >= child.node.summary.min)
        {
            return Err(PrivateIndexError::Context);
        }
        if children[..i]
            .iter()
            .any(|old| old.node.page == child.node.page)
        {
            return Err(PrivateIndexError::Malformed);
        }
        keys = keys
            .checked_add(child.node.summary.keys)
            .ok_or(PrivateIndexError::Bound)?;
    }
    let summary = NodeSummaryData {
        height,
        keys,
        min: first.min,
        max: last.max,
    };
    summary
        .validate_data(placement)
        .map_err(|_| PrivateIndexError::Bound)?;
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
        e.put_u128(u128::from(child.node.summary.keys));
    }
    let raw = Zeroizing::new(e.finish());
    PrivateIndexNodeFrame::decode(&raw, context, placement)?;
    Ok(raw)
}
fn header(
    context: PrivateIndexContextData,
    kind: NodeKindData,
    summary: NodeSummaryData,
    rows: usize,
    packed: usize,
) -> Result<Encoder> {
    let bytes = context
        .kind
        .header_bytes()
        .checked_add(packed)
        .filter(|n| *n <= MAX_INDEX_NODE_BYTES)
        .ok_or(PrivateIndexError::Bound)?;
    let mut e = Encoder::with_capacity(bytes);
    e.put_bytes(context.kind.domain())
        .map_err(|_| PrivateIndexError::Malformed)?;
    e.put_u16(VERSION);
    e.put_bytes(context.cid.as_bytes())
        .map_err(|_| PrivateIndexError::Malformed)?;
    e.put_u8(context.kind.code())
        .put_u8(if kind == NodeKindData::Leaf { 1 } else { 2 })
        .put_u8(summary.height)
        .put_u16(u16::try_from(rows).map_err(|_| PrivateIndexError::Bound)?)
        .put_u128(u128::from(summary.keys));
    e.put_bytes(&summary.min)
        .map_err(|_| PrivateIndexError::Malformed)?;
    e.put_bytes(&summary.max)
        .map_err(|_| PrivateIndexError::Malformed)?;
    e.put_u32(u32::try_from(packed).map_err(|_| PrivateIndexError::Bound)?);
    Ok(e)
}
fn min_child_keys(height: u8) -> Result<u128> {
    if height > MAX_INDEX_HEIGHT {
        return Err(PrivateIndexError::Bound);
    }
    let mut n = MIN_CHILD_LEAF_ROWS as u128;
    for _ in 0..height {
        n = n
            .checked_mul(MIN_CHILD_BRANCH_ROWS as u128)
            .ok_or(PrivateIndexError::Bound)?;
    }
    Ok(n)
}
fn max_child_keys(height: u8) -> Result<u128> {
    if height > MAX_INDEX_HEIGHT {
        return Err(PrivateIndexError::Bound);
    }
    let mut n = MAX_LEAF_ROWS as u128;
    for _ in 0..height {
        n = n
            .checked_mul(MAX_BRANCH_ROWS as u128)
            .ok_or(PrivateIndexError::Bound)?;
    }
    Ok(n.min(u128::from(u64::MAX)))
}
fn distance_at_least(min: [u8; 32], max: [u8; 32], needed: u64) -> bool {
    if min > max {
        return false;
    }
    let mut difference = [0u8; 32];
    let mut borrow = 0u16;
    for i in (0..32).rev() {
        let sub = u16::from(min[i]) + borrow;
        let value = u16::from(max[i]);
        difference[i] = value.wrapping_sub(sub) as u8;
        borrow = u16::from(value < sub);
    }
    difference[..24].iter().any(|b| *b != 0)
        || u64::from_be_bytes(difference[24..].try_into().expect("fixed difference")) >= needed
}
fn b32(d: &mut Decoder<'_>) -> Result<[u8; 32]> {
    d.get_bytes()
        .map_err(|_| PrivateIndexError::Malformed)?
        .try_into()
        .map_err(|_| PrivateIndexError::Malformed)
}
struct Packed<'a> {
    bytes: &'a [u8],
}
impl<'a> Packed<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.bytes.len() {
            return Err(PrivateIndexError::Malformed);
        }
        let (a, b) = self.bytes.split_at(n);
        self.bytes = b;
        Ok(a)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?
            .try_into()
            .map_err(|_| PrivateIndexError::Malformed)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    fn u128(&mut self) -> Result<u128> {
        Ok(u128::from_be_bytes(self.array()?))
    }
    fn finish(self) -> Result<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(PrivateIndexError::Malformed)
        }
    }
}

#[cfg(test)]
mod tests;
