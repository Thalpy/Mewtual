//! Finite paid-work index path DATA, using Core's actual packed-row and child checks.
//!
//! This owner performs no filesystem I/O and retains no key/Store/source/permission. The
//! physical query worker must hold the original selected-root query + OS-work leases while
//! reading the requested complete page, then reprove the SAME selected source before using
//! any result. A structural row/miss here NEVER certifies accepted-history absence.
use super::common::refused;
use crate::AppError;
use catcoms_replication::chat::cold_index::{
    encode_leaf_data, rewrite_branch_data, BranchRewriteData, ChildRefData, IndexContextData,
    IndexScopeData, LeafRowData, NodeFrame, NodeKindData, NodePlacementData, NodeRefData,
    MAX_INDEX_FRAME_BYTES, MAX_INDEX_HEIGHT, MAX_INDEX_NODE_BYTES, MAX_LEAF_ROWS,
};
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_storage::private_chat_page::{open_private_chat_page, PrivateChatPageId};
use zeroize::Zeroizing;
mod observed;

pub(in crate::store) const MAX_DESCENT_PAGES: usize = MAX_INDEX_HEIGHT as usize + 1;
pub(in crate::store) const MAX_BOUNDARY_READS: usize = MAX_INDEX_HEIGHT as usize;
/// ONE split-boundary raw/copy/plain overlap; release before the next level. Old path nodes
/// and any output encoders/seals remain separately invoiced by the whole transaction.
pub(in crate::store) const BOUNDARY_OPEN_BYTES: usize =
    MAX_INDEX_FRAME_BYTES + 2 * (MAX_INDEX_NODE_BYTES + 16);
const _: () =
    assert!(MAX_DESCENT_PAGES == 10 && MAX_BOUNDARY_READS == 9 && BOUNDARY_OPEN_BYTES == 49_270);

struct ContextData {
    vault: [u8; 32],
    group: [u8; 256],
    group_len: usize,
    profile: OwnerFactId,
    scope: IndexScopeData,
}
impl ContextData {
    fn from_data(context: IndexContextData<'_>) -> Result<Self, AppError> {
        if context.group.is_empty() || context.group.len() > 256 {
            return Err(refused());
        }
        let mut group = [0; 256];
        group[..context.group.len()].copy_from_slice(context.group);
        Ok(Self {
            vault: context.vault,
            group,
            group_len: context.group.len(),
            profile: context.profile,
            scope: context.scope,
        })
    }
    fn borrow(&self) -> IndexContextData<'_> {
        IndexContextData {
            vault: self.vault,
            group: &self.group[..self.group_len],
            profile: self.profile,
            scope: self.scope,
        }
    }
}

struct OpenedNodeData {
    plain: Zeroizing<Vec<u8>>,
    reference: NodeRefData,
    placement: NodePlacementData,
    // Exact encrypted input length, cached only after ALL authentication below.
    frame_bytes: usize,
}

/// Concrete one-path owner/scratch layouts for the independent metadata invoice. Payload
/// buffers use the separate whole-work invoice. Core's private branch rewrite scratch has
/// its OWN named layout term; counting BranchRewriteData alone cannot pay its packed rows.
pub(in crate::store) const ACCEPTED_INDEX_PATH_METADATA_BYTES: usize =
    std::mem::size_of::<AcceptedIndexPathData>()
        + MAX_DESCENT_PAGES * std::mem::size_of::<OpenedNodeData>()
        + (MAX_LEAF_ROWS + 1) * std::mem::size_of::<LeafRowData>()
        + 2 * std::mem::size_of::<Zeroizing<Vec<u8>>>()
        + 2 * std::mem::size_of::<ChildRefData<'static>>()
        + std::mem::size_of::<BranchRewriteData>();
impl OpenedNodeData {
    fn node(&self) -> NodeFrame<'_> {
        NodeFrame::decode(&self.plain, self.placement).expect("immutable checked node DATA")
    }
    fn child_index(&self, key: [u8; 32]) -> Result<usize, AppError> {
        let node = self.node();
        if node.kind() != NodeKindData::Branch {
            return Err(refused());
        }
        // First/last edge insertion follows the actual old child. An out-of-range key
        // never manufactures an empty subtree or a sibling maximum.
        let mut chosen = 0;
        for i in 1..node.row_count() {
            if node
                .branch_child_data(i)
                .map_err(|_| refused())?
                .lower_data()
                <= key
            {
                chosen = i;
            } else {
                break;
            }
        }
        Ok(chosen)
    }
}

/// At most ten independently authenticated plaintext nodes. Context is a fixed bounded copy,
/// not an allocation per node; all opened page buffers still need whole-operation payment.
pub(in crate::store) struct AcceptedIndexPathData {
    context: ContextData,
    root: NodeRefData,
    key: [u8; 32],
    leaves: Option<u64>,
    nodes: Vec<OpenedNodeData>,
}
impl std::fmt::Debug for AcceptedIndexPathData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcceptedIndexPathData")
            .finish_non_exhaustive()
    }
}
impl AcceptedIndexPathData {
    pub(in crate::store) fn new_data(
        context: IndexContextData<'_>,
        root: NodeRefData,
        key: [u8; 32],
        leaves: Option<u64>,
    ) -> Result<Self, AppError> {
        root.summary
            .validate_data(NodePlacementData::Root)
            .map_err(|_| refused())?;
        match (context.scope, leaves) {
            (IndexScopeData::Directory, None) => {}
            (IndexScopeData::Actions(_), Some(n)) if n > 0 && n >= root.summary.keys => {}
            _ => return Err(refused()),
        }
        Ok(Self {
            context: ContextData::from_data(context)?,
            root,
            key,
            leaves,
            nodes: Vec::with_capacity(MAX_DESCENT_PAGES),
        })
    }
    pub(in crate::store) fn next_page_data(&self) -> Result<Option<PrivateChatPageId>, AppError> {
        let Some(last) = self.nodes.last() else {
            return Ok(Some(self.root.page));
        };
        let node = last.node();
        if node.kind() == NodeKindData::Leaf {
            return Ok(None);
        }
        let index = last.child_index(self.key)?;
        Ok(Some(
            node.branch_child_data(index)
                .map_err(|_| refused())?
                .expected_page_data(),
        ))
    }
    /// Caller supplies only the next actual complete encrypted page requested above. The
    /// whole expected ciphertext ID is checked before copying/decrypting. Failure leaves
    /// all original path nodes unchanged; no new source can replace this path's root/key.
    pub(in crate::store) fn accept_frame_data(
        &mut self,
        key: &[u8; 32],
        frame: &[u8],
    ) -> Result<(), AppError> {
        let expected = self.next_page_data()?.ok_or_else(refused)?;
        if frame.len() > MAX_INDEX_FRAME_BYTES || self.nodes.len() >= MAX_DESCENT_PAGES {
            return Err(refused());
        }
        let plain = open_private_chat_page(key, expected, frame)?;
        let placement = if self.nodes.is_empty() {
            NodePlacementData::Root
        } else {
            NodePlacementData::Child
        };
        let node = NodeFrame::decode(&plain, placement).map_err(|_| refused())?;
        if node.context() != self.context.borrow() {
            return Err(refused());
        }
        node.check_claim_bounds_data(self.leaves)
            .map_err(|_| refused())?;
        let reference = if let Some(parent) = self.nodes.last() {
            parent
                .node()
                .branch_child_data(parent.child_index(self.key)?)
                .map_err(|_| refused())?
                .check_child(&node)
                .map_err(|_| refused())?
        } else {
            if node.summary() != self.root.summary {
                return Err(refused());
            }
            self.root
        };
        self.nodes.push(OpenedNodeData {
            plain,
            reference,
            placement,
            frame_bytes: frame.len(),
        });
        Ok(())
    }
    pub(in crate::store) fn row_data(&self) -> Result<Option<LeafRowData>, AppError> {
        let node = self.nodes.last().ok_or_else(refused)?.node();
        if node.kind() != NodeKindData::Leaf {
            return Err(refused());
        }
        for i in 0..node.row_count() {
            let row = node.leaf_row(i).map_err(|_| refused())?;
            if row.key == self.key {
                return Ok(Some(row));
            }
            if row.key > self.key {
                break;
            }
        }
        Ok(None)
    }
    /// Encodes one replacement leaf or deterministic16/17 split. Only local DATA geometry
    /// is checked: actual complete-hit/miss and action/receipt semantics belong to Root.
    /// Output is plaintext, so no RNG/nonce is consumed until the real candidate owner seals.
    pub(in crate::store) fn rewrite_leaf_data(
        &self,
        replacement: LeafRowData,
    ) -> Result<Vec<Zeroizing<Vec<u8>>>, AppError> {
        if replacement.key != self.key {
            return Err(refused());
        }
        let old = self.nodes.last().ok_or_else(refused)?;
        let node = old.node();
        if node.kind() != NodeKindData::Leaf {
            return Err(refused());
        }
        let mut rows = Vec::with_capacity(MAX_LEAF_ROWS + 1);
        let mut inserted = false;
        for i in 0..node.row_count() {
            let row = node.leaf_row(i).map_err(|_| refused())?;
            if !inserted && replacement.key <= row.key {
                rows.push(replacement);
                inserted = true;
            }
            if row.key != replacement.key {
                rows.push(row);
            }
        }
        if !inserted {
            rows.push(replacement);
        }
        let mut result = Vec::with_capacity(2);
        if rows.len() <= MAX_LEAF_ROWS {
            result.push(
                encode_leaf_data(self.context.borrow(), &rows, old.placement)
                    .map_err(|_| refused())?,
            );
        } else {
            for part in [&rows[..16], &rows[16..]] {
                result.push(
                    encode_leaf_data(self.context.borrow(), part, NodePlacementData::Child)
                        .map_err(|_| refused())?,
                );
            }
        }
        Ok(result)
    }
    /// Parent level is an actual retained descent node; outputs at the next level have
    /// already been independently sealed by the candidate owner. Asking for an extra page
    /// merely freezes its identity; actual I/O remains outside all global guards.
    pub(in crate::store) fn boundary_page_data(
        &self,
        level: usize,
        replacements: usize,
    ) -> Result<Option<PrivateChatPageId>, AppError> {
        let parent = self.nodes.get(level).ok_or_else(refused)?;
        if level + 1 >= self.nodes.len() {
            return Err(refused());
        }
        Ok(parent
            .node()
            .branch_split_boundary_data(parent.child_index(self.key)?, replacements)
            .map_err(|_| refused())?
            .map(|b| b.expected_page_data()))
    }
    pub(in crate::store) fn rewrite_branch_data(
        &self,
        level: usize,
        replacements: &[ChildRefData<'_>],
        boundary: Option<(&[u8; 32], &[u8])>,
    ) -> Result<BranchRewriteData, AppError> {
        let parent = self.nodes.get(level).ok_or_else(refused)?;
        let selected = self.nodes.get(level + 1).ok_or_else(refused)?;
        let parent_node = parent.node();
        let chosen = parent.child_index(self.key)?;
        let expected = parent_node
            .branch_split_boundary_data(chosen, replacements.len())
            .map_err(|_| refused())?;
        // Keep this ONE additional raw/open/decoded buffer through rewrite; drop it on return
        // before another splitting level is visited. No sibling-max guess is permitted.
        let boundary_plain = match (&expected, boundary) {
            (Some(descriptor), Some((key, raw))) => {
                if raw.len() > MAX_INDEX_FRAME_BYTES {
                    return Err(refused());
                }
                Some(open_private_chat_page(
                    key,
                    descriptor.expected_page_data(),
                    raw,
                )?)
            }
            (None, None) => None,
            _ => return Err(refused()),
        };
        let boundary_ref = if let (Some(descriptor), Some(plain)) = (&expected, &boundary_plain) {
            let node = NodeFrame::decode(plain, NodePlacementData::Child).map_err(|_| refused())?;
            if node.context() != self.context.borrow() {
                return Err(refused());
            }
            node.check_claim_bounds_data(self.leaves)
                .map_err(|_| refused())?;
            Some(ChildRefData {
                context: self.context.borrow(),
                node: descriptor.check_child(&node).map_err(|_| refused())?,
            })
        } else {
            None
        };
        rewrite_branch_data(
            &parent_node,
            chosen,
            &ChildRefData {
                context: self.context.borrow(),
                node: selected.reference,
            },
            replacements,
            boundary_ref.as_ref(),
            parent.placement,
        )
        .map_err(|_| refused())
    }
}

#[cfg(test)]
mod tests;
