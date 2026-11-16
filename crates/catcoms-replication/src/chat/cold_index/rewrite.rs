//! Bounded packed-preserving COW geometry. These are DATA codecs, not authenticated selected
//! roots or accepted absence. App must hold its actual paid query/work/source custody and
//! authenticate each page before supplying the decoded child reference used here.
use super::*;

/// One exact old packed row and its parent constraints. Interior max is deliberately unknown;
/// the next lower key is an exclusive bound, never a substitute for the child's actual max.
pub struct BranchChildData<'a> {
    context: IndexContextData<'a>,
    page: PrivateChatPageId,
    lower: [u8; 32],
    keys: u64,
    height: u8,
    upper: Option<[u8; 32]>,
    parent_max: [u8; 32],
}
impl std::fmt::Debug for BranchChildData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BranchChildData")
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}
impl BranchChildData<'_> {
    pub fn expected_page_data(&self) -> PrivateChatPageId {
        self.page
    }
    /// Packed row lower key only; edge insertion still opens and checks the actual child.
    pub fn lower_data(&self) -> [u8; 32] {
        self.lower
    }
    /// Checks the ACTUAL decoded child, including occupancy. Neither this method nor a raw
    /// NodeRef authenticates ciphertext, source selection or permission to read the page.
    pub fn check_child(&self, child: &NodeFrame<'_>) -> Result<NodeRefData, ChatError> {
        let node = NodeRefData {
            page: self.page,
            summary: child.summary(),
        };
        self.check_reference(&ChildRefData {
            context: child.context(),
            node,
        })?;
        Ok(node)
    }
    fn check_reference(&self, child: &ChildRefData<'_>) -> Result<(), ChatError> {
        child.node.summary.validate_data(NodePlacementData::Child)?;
        let s = child.node.summary;
        if child.context != self.context
            || child.node.page != self.page
            || s.min != self.lower
            || s.keys != self.keys
            || s.height != self.height
            || self.upper.is_some_and(|upper| s.max >= upper)
            || (self.upper.is_none() && s.max != self.parent_max)
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
}
impl NodeFrame<'_> {
    /// Exact child descriptor for descent, including first/last edge insertion. A structural
    /// range miss is not an empty tree: the writer must still open the real edge child.
    pub fn branch_child_data(&self, index: usize) -> Result<BranchChildData<'_>, ChatError> {
        let (lower, page, keys) = self.branch_row(index)?;
        Ok(BranchChildData {
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
    pub fn branch_split_boundary_data(
        &self,
        index: usize,
        replacements: usize,
    ) -> Result<Option<BranchChildData<'_>>, ChatError> {
        if self.kind != NodeKindData::Branch
            || index >= self.rows
            || !(1..=2).contains(&replacements)
        {
            return Err(ChatError::Bound);
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

/// One complete encoded branch. App seals it once under its real paid work, then derives
/// the page reference from those same immutable bytes. It is not a Saved/index-root proof.
pub struct EncodedBranchData {
    plain: Zeroizing<Vec<u8>>,
    summary: NodeSummaryData,
}
impl std::fmt::Debug for EncodedBranchData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodedBranchData")
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}
impl EncodedBranchData {
    pub fn plain_data(&self) -> &[u8] {
        &self.plain
    }
    pub fn summary_data(&self) -> NodeSummaryData {
        self.summary
    }
}
/// Owns exactly one rewritten branch or two split branches, never an unbounded inventory.
#[derive(Debug)]
pub struct BranchRewriteData {
    nodes: Vec<EncodedBranchData>,
}
impl BranchRewriteData {
    pub fn nodes_data(&self) -> &[EncodedBranchData] {
        &self.nodes
    }
    pub fn into_nodes_data(self) -> Vec<EncodedBranchData> {
        self.nodes
    }
}

enum Row<'a> {
    Old(usize),
    New(&'a ChildRefData<'a>),
}

/// Named requested temporary/output inventory for the actual compiled target. Parent and
/// replacement/boundary plaintext, authenticated reads, sealing and retained output debt
/// are separately paid by App. Keep this class alive until these outputs are transferred
/// into their original paid work or dropped; it is not a payment or an RSS guarantee.
pub const BRANCH_REWRITE_METADATA_BYTES: usize = (MAX_BRANCH_ROWS + 1)
    * std::mem::size_of::<Row<'static>>()
    + 2 * std::mem::size_of::<EncodedBranchData>()
    + std::mem::size_of::<BranchRewriteData>()
    + std::mem::size_of::<Encoder>()
    + 4 * std::mem::size_of::<NodeFrame<'static>>()
    + 2 * std::mem::size_of::<BranchChildData<'static>>();

/// Metadata plus the two bounded encoded output allocations. These are distinct terms
/// so the actual host can fund retained frames without silently reusing metadata credit.
pub const BRANCH_REWRITE_TEMPORARY_BYTES: usize =
    BRANCH_REWRITE_METADATA_BYTES + 2 * MAX_INDEX_NODE_BYTES;
const _: () = assert!(BRANCH_REWRITE_TEMPORARY_BYTES < 64 * 1024);
fn row_tuple(
    parent: &NodeFrame<'_>,
    row: &Row<'_>,
) -> Result<([u8; 32], PrivateChatPageId, u64), ChatError> {
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
pub fn rewrite_branch_data(
    parent: &NodeFrame<'_>,
    selected_index: usize,
    original_selected: &ChildRefData<'_>,
    replacements: &[ChildRefData<'_>],
    split_boundary: Option<&ChildRefData<'_>>,
    placement: NodePlacementData,
) -> Result<BranchRewriteData, ChatError> {
    let old = parent.branch_child_data(selected_index)?;
    old.check_reference(original_selected)?;
    if !(1..=2).contains(&replacements.len()) {
        return Err(ChatError::Bound);
    }
    let mut keys = 0u64;
    for (i, child) in replacements.iter().enumerate() {
        child.node.summary.validate_data(NodePlacementData::Child)?;
        if child.context != parent.context
            || child.node.summary.height != old.height
            || (i != 0 && replacements[i - 1].node.summary.max >= child.node.summary.min)
        {
            return Err(ChatError::Scope);
        }
        keys = keys
            .checked_add(child.node.summary.keys)
            .ok_or(ChatError::Bound)?;
    }
    let first = replacements[0].node.summary;
    let last = replacements[replacements.len() - 1].node.summary;
    let original = original_selected.node.summary;
    if keys < original.keys
        || keys > original.keys.checked_add(1).ok_or(ChatError::Bound)?
        || first.min > original.min
        || last.max < original.max
        || (selected_index != 0 && first.min != original.min)
        || old.upper.is_some_and(|upper| last.max >= upper)
        || (keys == original.keys && (first.min != original.min || last.max != original.max))
    {
        return Err(ChatError::Scope);
    }
    let required_boundary =
        parent.branch_split_boundary_data(selected_index, replacements.len())?;
    let boundary_max = match (required_boundary, split_boundary) {
        (Some(expected), Some(actual)) => {
            expected.check_reference(actual)?;
            Some(actual.node.summary.max)
        }
        (None, None) => None,
        _ => return Err(ChatError::Dependency),
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
                return Err(ChatError::Malformed);
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
            Row::Old(_) => boundary_max.ok_or(ChatError::Dependency)?,
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
    Ok(BranchRewriteData { nodes })
}
fn encode_rows(
    parent: &NodeFrame<'_>,
    rows: &[Row<'_>],
    max: [u8; 32],
    placement: NodePlacementData,
) -> Result<EncodedBranchData, ChatError> {
    let mut keys = 0u64;
    for row in rows {
        keys = keys
            .checked_add(row_tuple(parent, row)?.2)
            .ok_or(ChatError::Bound)?;
    }
    let summary = NodeSummaryData {
        height: parent.summary.height,
        keys,
        min: row_tuple(parent, &rows[0])?.0,
        max,
    };
    summary.validate_data(placement)?;
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
    NodeFrame::decode(&plain, placement)?;
    Ok(EncodedBranchData { plain, summary })
}
