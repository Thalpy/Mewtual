//! Packed-preserving private token/publication COW DATA. The geometry intentionally follows
//! the Core bounded branch updater, but uses ONLY its own Cid/kind/domain codec. Structural
//! references never authenticate a supplied root or grant physical query/selection authority.
use super::*;

/// One exact old packed row and its parent constraints. Interior max is deliberately unknown;
/// the next lower key is an exclusive bound, never a substitute for the child's actual max.
pub(in crate::store) struct PrivateIndexBranchChildData {
    context: PrivateIndexContextData,
    page: PrivateChatPageId,
    lower: [u8; 32],
    keys: u64,
    height: u8,
    upper: Option<[u8; 32]>,
    parent_max: [u8; 32],
}
impl std::fmt::Debug for PrivateIndexBranchChildData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivateIndexBranchChildData")
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}
impl PrivateIndexBranchChildData {
    pub(in crate::store) fn expected_page_data(&self) -> PrivateChatPageId {
        self.page
    }
    /// Packed row lower key only; edge insertion still opens and checks the actual child.
    pub(in crate::store) fn lower_data(&self) -> [u8; 32] {
        self.lower
    }
    /// Checks the ACTUAL decoded child, including occupancy. Neither this method nor a raw
    /// NodeRef authenticates ciphertext, source selection or permission to read the page.
    pub(in crate::store) fn check_child(
        &self,
        child: &PrivateIndexNodeFrame<'_>,
    ) -> Result<NodeRefData> {
        let node = NodeRefData {
            page: self.page,
            summary: child.summary(),
        };
        self.check_reference(&PrivateIndexChildRefData {
            context: child.context(),
            node,
        })?;
        Ok(node)
    }
    fn check_reference(&self, child: &PrivateIndexChildRefData) -> Result<()> {
        child
            .node
            .summary
            .validate_data(NodePlacementData::Child)
            .map_err(|_| PrivateIndexError::Bound)?;
        let s = child.node.summary;
        if child.context != self.context
            || child.node.page != self.page
            || s.min != self.lower
            || s.keys != self.keys
            || s.height != self.height
            || self.upper.is_some_and(|upper| s.max >= upper)
            || (self.upper.is_none() && s.max != self.parent_max)
        {
            return Err(PrivateIndexError::Context);
        }
        Ok(())
    }
}
impl PrivateIndexNodeFrame<'_> {
    /// Exact child descriptor for descent, including first/last edge insertion. A structural
    /// range miss is not an empty tree: the writer must still open the real edge child.
    pub(in crate::store) fn branch_child_data(
        &self,
        index: usize,
    ) -> Result<PrivateIndexBranchChildData> {
        let (lower, page, keys) = self.branch_row(index)?;
        Ok(PrivateIndexBranchChildData {
            context: self.context,
            page,
            lower,
            keys,
            height: self.summary.height - 1,
            upper: if index + 1 < self.rows {
                Some(self.branch_row(index + 1)?.0)
            } else {
                None
            },
            parent_max: self.summary.max,
        })
    }
    /// Preflight the extra actual page read needed ONLY when a 193-row split would end the
    /// left branch at an unchanged sibling. Its exact max is absent from the packed parent.
    /// At most one such bounded read per splitting level, separately funded by the host.
    pub(in crate::store) fn branch_split_boundary_data(
        &self,
        index: usize,
        replacements: usize,
    ) -> Result<Option<PrivateIndexBranchChildData>> {
        if self.kind != NodeKindData::Branch
            || index >= self.rows
            || !(1..=2).contains(&replacements)
        {
            return Err(PrivateIndexError::Bound);
        }
        let rows = self.rows - 1 + replacements;
        if rows <= MAX_BRANCH_ROWS {
            return Ok(None);
        }
        let end_left = rows / 2 - 1;
        if (index..index + replacements).contains(&end_left) {
            return Ok(None);
        }
        let old_index = if end_left < index {
            end_left
        } else {
            end_left - replacements + 1
        };
        self.branch_child_data(old_index).map(Some)
    }
}

/// One complete encoded node. App seals it once under its real paid work, then derives
/// the page reference from those same immutable bytes. It is not a Saved/index-root proof.
pub(in crate::store) struct EncodedPrivateIndexNodeData {
    plain: Zeroizing<Vec<u8>>,
    summary: NodeSummaryData,
}
impl std::fmt::Debug for EncodedPrivateIndexNodeData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodedPrivateIndexNodeData")
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}
impl EncodedPrivateIndexNodeData {
    pub(in crate::store) fn plain_data(&self) -> &[u8] {
        &self.plain
    }
    pub(in crate::store) fn summary_data(&self) -> NodeSummaryData {
        self.summary
    }
}
/// Owns exactly one rewritten branch or two split branches, never an unbounded inventory.
#[derive(Debug)]
pub(in crate::store) struct PrivateIndexRewriteData {
    nodes: Vec<EncodedPrivateIndexNodeData>,
}
impl PrivateIndexRewriteData {
    pub(in crate::store) fn nodes_data(&self) -> &[EncodedPrivateIndexNodeData] {
        &self.nodes
    }
    pub(in crate::store) fn into_nodes_data(self) -> Vec<EncodedPrivateIndexNodeData> {
        self.nodes
    }
}

enum Row<'a> {
    Old(usize),
    New(&'a PrivateIndexChildRefData),
}

/// Named requested temporary/output inventory for the actual compiled target. Parent and
/// replacement/boundary plaintext, authenticated reads, sealing and retained output debt
/// are separately paid by App. Keep this class alive until these outputs are transferred
/// into their original paid work or dropped; it is not a payment or an RSS guarantee.
pub(in crate::store) const PRIVATE_INDEX_REWRITE_METADATA_BYTES: usize = (MAX_BRANCH_ROWS + 1)
    * std::mem::size_of::<Row<'static>>()
    + 2 * std::mem::size_of::<EncodedPrivateIndexNodeData>()
    + std::mem::size_of::<PrivateIndexRewriteData>()
    + std::mem::size_of::<Encoder>()
    + 4 * std::mem::size_of::<PrivateIndexNodeFrame<'static>>()
    + 2 * std::mem::size_of::<PrivateIndexBranchChildData>()
    + (MAX_LEAF_ROWS + 1) * std::mem::size_of::<LeafEdit<'static>>()
    + MAX_VALUE_BYTES;

/// Metadata plus the two bounded encoded output allocations. These are distinct terms
/// so the actual host can fund retained frames without silently reusing metadata credit.
pub(in crate::store) const PRIVATE_INDEX_REWRITE_TEMPORARY_BYTES: usize =
    PRIVATE_INDEX_REWRITE_METADATA_BYTES + 2 * MAX_INDEX_NODE_BYTES;
const _: () = assert!(PRIVATE_INDEX_REWRITE_TEMPORARY_BYTES < 64 * 1024);
fn row_tuple(
    parent: &PrivateIndexNodeFrame<'_>,
    row: &Row<'_>,
) -> Result<([u8; 32], PrivateChatPageId, u64)> {
    match row {
        Row::Old(index) => parent.branch_row(*index),
        Row::New(child) => Ok((
            child.node.summary.min,
            child.node.page,
            child.node.summary.keys,
        )),
    }
}

/// Replace ONE checked child by one/two checked newly encoded child references. Unchanged
/// rows are copied byte-for-byte; their unrecorded max is never fabricated. This enforces
/// local range/count/COW geometry only. The leaf writer separately proves exact retained
/// values and the original complete lookup; DATA summaries cannot attest those semantics.
pub(in crate::store) fn rewrite_private_branch_data(
    parent: &PrivateIndexNodeFrame<'_>,
    selected_index: usize,
    original_selected: &PrivateIndexChildRefData,
    replacements: &[PrivateIndexChildRefData],
    split_boundary: Option<&PrivateIndexChildRefData>,
    placement: NodePlacementData,
) -> Result<PrivateIndexRewriteData> {
    let old = parent.branch_child_data(selected_index)?;
    old.check_reference(original_selected)?;
    if !(1..=2).contains(&replacements.len()) {
        return Err(PrivateIndexError::Bound);
    }
    let mut keys = 0u64;
    for (i, child) in replacements.iter().enumerate() {
        child
            .node
            .summary
            .validate_data(NodePlacementData::Child)
            .map_err(|_| PrivateIndexError::Bound)?;
        if child.context != parent.context
            || child.node.summary.height != old.height
            || (i != 0 && replacements[i - 1].node.summary.max >= child.node.summary.min)
        {
            return Err(PrivateIndexError::Context);
        }
        keys = keys
            .checked_add(child.node.summary.keys)
            .ok_or(PrivateIndexError::Bound)?;
    }
    let first = replacements[0].node.summary;
    let last = replacements[replacements.len() - 1].node.summary;
    let original = original_selected.node.summary;
    if keys < original.keys
        || (keys != original.keys && original.keys.checked_add(1) != Some(keys))
        || (replacements.len() == 2 && original.keys.checked_add(1) != Some(keys))
        || first.min > original.min
        || last.max < original.max
        || (selected_index != 0 && first.min != original.min)
        || old.upper.is_some_and(|upper| last.max >= upper)
        || (keys == original.keys && (first.min != original.min || last.max != original.max))
    {
        return Err(PrivateIndexError::Context);
    }
    let required_boundary =
        parent.branch_split_boundary_data(selected_index, replacements.len())?;
    let boundary_max = match (required_boundary, split_boundary) {
        (Some(expected), Some(actual)) => {
            expected.check_reference(actual)?;
            Some(actual.node.summary.max)
        }
        (None, None) => None,
        _ => return Err(PrivateIndexError::Dependency),
    };
    let mut rows = Vec::with_capacity(MAX_BRANCH_ROWS + 1);
    for i in 0..parent.rows {
        if i == selected_index {
            rows.extend(replacements.iter().map(Row::New));
        } else {
            rows.push(Row::Old(i));
        }
    }
    // A duplicate spanning the two future split outputs would escape each individual
    // node decoder. Check the COMPLETE bounded sibling inventory before partitioning it.
    for (i, row) in rows.iter().enumerate() {
        let page = row_tuple(parent, row)?.1;
        for old in &rows[..i] {
            if row_tuple(parent, old)?.1 == page {
                return Err(PrivateIndexError::Malformed);
            }
        }
    }
    let max = if selected_index + 1 == parent.rows {
        last.max
    } else {
        parent.summary.max
    };
    let mut nodes = Vec::with_capacity(2);
    if rows.len() <= MAX_BRANCH_ROWS {
        nodes.push(encode_rows(parent, &rows, max, placement)?);
    } else {
        let half = rows.len() / 2;
        let left_max = match &rows[half - 1] {
            Row::New(child) => child.node.summary.max,
            Row::Old(_) => boundary_max.ok_or(PrivateIndexError::Dependency)?,
        };
        nodes.push(encode_rows(
            parent,
            &rows[..half],
            left_max,
            NodePlacementData::Child,
        )?);
        nodes.push(encode_rows(
            parent,
            &rows[half..],
            max,
            NodePlacementData::Child,
        )?);
    }
    Ok(PrivateIndexRewriteData { nodes })
}
fn encode_rows(
    parent: &PrivateIndexNodeFrame<'_>,
    rows: &[Row<'_>],
    max: [u8; 32],
    placement: NodePlacementData,
) -> Result<EncodedPrivateIndexNodeData> {
    let mut keys = 0u64;
    for row in rows {
        keys = keys
            .checked_add(row_tuple(parent, row)?.2)
            .ok_or(PrivateIndexError::Bound)?;
    }
    let summary = NodeSummaryData {
        height: parent.summary.height,
        keys,
        min: row_tuple(parent, &rows[0])?.0,
        max,
    };
    summary
        .validate_data(placement)
        .map_err(|_| PrivateIndexError::Bound)?;
    let mut e = header(
        parent.context,
        NodeKindData::Branch,
        summary,
        rows.len(),
        rows.len() * BRANCH_ROW_BYTES,
    )?;
    for row in rows {
        match row {
            Row::Old(index) => {
                for b in &parent.packed[index * BRANCH_ROW_BYTES..(index + 1) * BRANCH_ROW_BYTES] {
                    e.put_u8(*b);
                }
            }
            Row::New(child) => {
                for b in child.node.summary.min {
                    e.put_u8(b);
                }
                for b in child.node.page.as_bytes() {
                    e.put_u8(*b);
                }
                e.put_u128(u128::from(child.node.summary.keys));
            }
        }
    }
    let plain = Zeroizing::new(e.finish());
    PrivateIndexNodeFrame::decode(&plain, parent.context, placement)?;
    Ok(EncodedPrivateIndexNodeData { plain, summary })
}

enum LeafEdit<'a> {
    Old(usize),
    New(&'a PrivateIndexLeafRowData),
}

/// One insertion/replacement, never deletion. Unchanged packed leaf rows are copied byte
/// for byte, just like branch rows; a future canonical-value change cannot silently rewrite
/// unrelated history during a one-token edit. A complete selected-source query is separate.
pub(in crate::store) fn rewrite_private_leaf_data(
    parent: &PrivateIndexNodeFrame<'_>,
    replacement: &PrivateIndexLeafRowData,
    placement: NodePlacementData,
) -> Result<PrivateIndexRewriteData> {
    if parent.kind != NodeKindData::Leaf {
        return Err(PrivateIndexError::Bound);
    }
    let encoded = replacement.value.encode(parent.context, replacement.key)?;
    let mut rows = Vec::with_capacity(MAX_LEAF_ROWS + 1);
    let mut inserted = false;
    for i in 0..parent.rows {
        let key: [u8; 32] = parent.packed_leaf_row(i)?[..32]
            .try_into()
            .map_err(|_| PrivateIndexError::Malformed)?;
        if !inserted && replacement.key <= key {
            rows.push(LeafEdit::New(replacement));
            inserted = true;
        }
        if key != replacement.key {
            rows.push(LeafEdit::Old(i));
        }
    }
    if !inserted {
        rows.push(LeafEdit::New(replacement));
    }
    let mut nodes = Vec::with_capacity(2);
    if rows.len() <= MAX_LEAF_ROWS {
        nodes.push(encode_leaf_rows(parent, &rows, &encoded, placement)?);
    } else {
        // Exactly33 rows; deterministic16/17. A root split produces two CHILD nodes,
        // whose genuine summaries/page IDs are needed to construct a separate new root.
        nodes.push(encode_leaf_rows(
            parent,
            &rows[..16],
            &encoded,
            NodePlacementData::Child,
        )?);
        nodes.push(encode_leaf_rows(
            parent,
            &rows[16..],
            &encoded,
            NodePlacementData::Child,
        )?);
    }
    Ok(PrivateIndexRewriteData { nodes })
}
fn leaf_key(parent: &PrivateIndexNodeFrame<'_>, row: &LeafEdit<'_>) -> Result<[u8; 32]> {
    match row {
        LeafEdit::Old(i) => parent.packed_leaf_row(*i)?[..32]
            .try_into()
            .map_err(|_| PrivateIndexError::Malformed),
        LeafEdit::New(row) => Ok(row.key),
    }
}
fn encode_leaf_rows(
    parent: &PrivateIndexNodeFrame<'_>,
    rows: &[LeafEdit<'_>],
    new_encoded: &[u8],
    placement: NodePlacementData,
) -> Result<EncodedPrivateIndexNodeData> {
    if rows.is_empty() || rows.len() > MAX_LEAF_ROWS {
        return Err(PrivateIndexError::Bound);
    }
    let summary = NodeSummaryData {
        height: 0,
        keys: rows.len() as u64,
        min: leaf_key(parent, &rows[0])?,
        max: leaf_key(parent, &rows[rows.len() - 1])?,
    };
    summary
        .validate_data(placement)
        .map_err(|_| PrivateIndexError::Bound)?;
    let mut packed = 0usize;
    for row in rows {
        let bytes = match row {
            LeafEdit::Old(i) => parent.packed_leaf_row(*i)?.len(),
            LeafEdit::New(_) => 34 + new_encoded.len(),
        };
        packed = packed.checked_add(bytes).ok_or(PrivateIndexError::Bound)?;
    }
    let mut e = header(
        parent.context,
        NodeKindData::Leaf,
        summary,
        rows.len(),
        packed,
    )?;
    for row in rows {
        match row {
            LeafEdit::Old(i) => {
                for b in parent.packed_leaf_row(*i)? {
                    e.put_u8(*b);
                }
            }
            LeafEdit::New(row) => {
                for b in row.key {
                    e.put_u8(b);
                }
                e.put_u16(u16::try_from(new_encoded.len()).map_err(|_| PrivateIndexError::Bound)?);
                for b in new_encoded {
                    e.put_u8(*b);
                }
            }
        }
    }
    let plain = Zeroizing::new(e.finish());
    PrivateIndexNodeFrame::decode(&plain, parent.context, placement)?;
    Ok(EncodedPrivateIndexNodeData { plain, summary })
}
