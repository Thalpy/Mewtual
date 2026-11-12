//! One bounded actual encrypted-page descent, still DATA rather than selected-source proof.
//! Exact complete ciphertext IDs are authenticated before retaining any plaintext node.
use super::*;
mod observed;
use catcoms_storage::private_chat_page::open_private_chat_page;

const MAX_DESCENT_PAGES: usize = MAX_INDEX_HEIGHT as usize + 1;
/// One extra actual boundary read: complete raw input plus copied ciphertext/plaintext
/// AEAD capacities. The caller drops this scratch before visiting another split level.
pub(in crate::store) const PRIVATE_INDEX_BOUNDARY_OPEN_BYTES: usize =
    MAX_INDEX_FRAME_BYTES + 2 * (MAX_INDEX_NODE_BYTES + 16);
const _: () = assert!(PRIVATE_INDEX_BOUNDARY_OPEN_BYTES == 49_270);
struct OpenedPrivateNodeData {
    plain: Zeroizing<Vec<u8>>,
    reference: NodeRefData,
    placement: NodePlacementData,
    // Exact authenticated complete cipher length, never a frame cap.
    frame_bytes: usize,
}
impl OpenedPrivateNodeData {
    fn node(&self, context: PrivateIndexContextData) -> PrivateIndexNodeFrame<'_> {
        PrivateIndexNodeFrame::decode(&self.plain, context, self.placement)
            .expect("immutable fully checked private node")
    }
    fn child_index(&self, context: PrivateIndexContextData, key: [u8; 32]) -> Result<usize> {
        let node = self.node(context);
        if node.kind() != NodeKindData::Branch {
            return Err(PrivateIndexError::Bound);
        }
        // Edge insertion opens the true old first/last child. An out-of-range key cannot
        // fabricate an empty subtree or turn an exclusive next-lower into an actual max.
        let mut chosen = 0;
        for i in 1..node.row_count() {
            if node.branch_child_data(i)?.lower_data() <= key {
                chosen = i;
            } else {
                break;
            }
        }
        Ok(chosen)
    }
}

pub(in crate::store) struct PrivateIndexPathData {
    context: PrivateIndexContextData,
    root: NodeRefData,
    key: [u8; 32],
    nodes: Vec<OpenedPrivateNodeData>,
}
/// Concrete bounded metadata; all retained path plaintext buffers and output/seal overlaps
/// are additional terms in the caller's real paid work invoice. Nothing here reserves them.
pub(in crate::store) const PRIVATE_INDEX_PATH_METADATA_BYTES: usize =
    std::mem::size_of::<PrivateIndexPathData>()
        + MAX_DESCENT_PAGES * std::mem::size_of::<OpenedPrivateNodeData>()
        + (MAX_LEAF_ROWS + 1) * std::mem::size_of::<PrivateIndexLeafRowData>()
        + 2 * std::mem::size_of::<PrivateIndexChildRefData>();
const _: () = assert!(MAX_DESCENT_PAGES == 10);
impl PrivateIndexPathData {
    pub(in crate::store) fn new_data(
        context: PrivateIndexContextData,
        root: NodeRefData,
        key: [u8; 32],
    ) -> Result<Self> {
        root.summary
            .validate_data(NodePlacementData::Root)
            .map_err(|_| PrivateIndexError::Bound)?;
        if context.kind == PrivateIndexKindData::Publication {
            PublicationValueData::ordinal_from_key_data(key)
                .map_err(|_| PrivateIndexError::Context)?;
        }
        Ok(Self {
            context,
            root,
            key,
            nodes: Vec::with_capacity(MAX_DESCENT_PAGES),
        })
    }
    pub(in crate::store) fn next_page_data(&self) -> Result<Option<PrivateChatPageId>> {
        let Some(last) = self.nodes.last() else {
            return Ok(Some(self.root.page));
        };
        let node = last.node(self.context);
        if node.kind() == NodeKindData::Leaf {
            return Ok(None);
        }
        Ok(Some(
            node.branch_child_data(last.child_index(self.context, self.key)?)?
                .expected_page_data(),
        ))
    }
    /// This opens only the next requested COMPLETE frame. Wrong hash/AEAD, kind, Cid,
    /// child occupancy/height/count/range or root summary leaves the original path intact.
    pub(in crate::store) fn accept_frame_data(
        &mut self,
        key: &[u8; 32],
        frame: &[u8],
    ) -> Result<()> {
        let expected = self.next_page_data()?.ok_or(PrivateIndexError::Bound)?;
        if frame.len() > MAX_INDEX_FRAME_BYTES || self.nodes.len() >= MAX_DESCENT_PAGES {
            return Err(PrivateIndexError::Bound);
        }
        let plain = open_private_chat_page(key, expected, frame)
            .map_err(|_| PrivateIndexError::Authentication)?;
        let placement = if self.nodes.is_empty() {
            NodePlacementData::Root
        } else {
            NodePlacementData::Child
        };
        let node = PrivateIndexNodeFrame::decode(&plain, self.context, placement)?;
        let reference = if let Some(parent) = self.nodes.last() {
            parent
                .node(self.context)
                .branch_child_data(parent.child_index(self.context, self.key)?)?
                .check_child(&node)?
        } else {
            if node.summary() != self.root.summary {
                return Err(PrivateIndexError::Context);
            }
            self.root
        };
        self.nodes.push(OpenedPrivateNodeData {
            plain,
            reference,
            placement,
            frame_bytes: frame.len(),
        });
        Ok(())
    }
    /// A structural result only. A real miss/hit additionally needs the SAME complete
    /// selected O/root/query lifetime and original request/action checks at the host.
    pub(in crate::store) fn row_data(&self) -> Result<Option<PrivateIndexLeafRowData>> {
        let node = self
            .nodes
            .last()
            .ok_or(PrivateIndexError::Dependency)?
            .node(self.context);
        if node.kind() != NodeKindData::Leaf {
            return Err(PrivateIndexError::Dependency);
        }
        for i in 0..node.row_count() {
            let row = node.leaf_row(i)?;
            if row.key == self.key {
                return Ok(Some(row));
            }
            if row.key > self.key {
                break;
            }
        }
        Ok(None)
    }
    pub(in crate::store) fn rewrite_leaf_data(
        &self,
        replacement: &PrivateIndexLeafRowData,
    ) -> Result<PrivateIndexRewriteData> {
        if replacement.key != self.key {
            return Err(PrivateIndexError::Context);
        }
        let old = self.nodes.last().ok_or(PrivateIndexError::Dependency)?;
        rewrite_private_leaf_data(&old.node(self.context), replacement, old.placement)
    }
    pub(in crate::store) fn boundary_page_data(
        &self,
        level: usize,
        replacements: usize,
    ) -> Result<Option<PrivateChatPageId>> {
        let parent = self.nodes.get(level).ok_or(PrivateIndexError::Bound)?;
        if level + 1 >= self.nodes.len() {
            return Err(PrivateIndexError::Dependency);
        }
        Ok(parent
            .node(self.context)
            .branch_split_boundary_data(parent.child_index(self.context, self.key)?, replacements)?
            .map(|child| child.expected_page_data()))
    }
    /// Exactly one extra boundary child may be opened at a splitting level. Authentication
    /// and real descriptor checks are mandatory; neither next-lower minus one nor a caller
    /// summary can invent that unchanged child's maximum. Drop scratch before next level.
    pub(in crate::store) fn rewrite_branch_data(
        &self,
        level: usize,
        replacements: &[PrivateIndexChildRefData],
        boundary: Option<(&[u8; 32], &[u8])>,
    ) -> Result<PrivateIndexRewriteData> {
        let parent = self.nodes.get(level).ok_or(PrivateIndexError::Bound)?;
        let selected = self
            .nodes
            .get(level + 1)
            .ok_or(PrivateIndexError::Dependency)?;
        if replacements.len() == 2 {
            let selected_node = selected.node(self.context);
            let full = match selected_node.kind() {
                NodeKindData::Leaf => MAX_LEAF_ROWS,
                NodeKindData::Branch => MAX_BRANCH_ROWS,
            };
            // Two replacements arise ONLY from an actual full-node insertion split.
            // A caller cannot split a sparse child during an unrelated value update.
            if selected_node.row_count() != full {
                return Err(PrivateIndexError::Bound);
            }
        }
        let node = parent.node(self.context);
        let chosen = parent.child_index(self.context, self.key)?;
        let descriptor = node.branch_split_boundary_data(chosen, replacements.len())?;
        let boundary_plain = match (&descriptor, boundary) {
            (Some(expected), Some((key, raw))) => {
                if raw.len() > MAX_INDEX_FRAME_BYTES {
                    return Err(PrivateIndexError::Bound);
                }
                Some(
                    open_private_chat_page(key, expected.expected_page_data(), raw)
                        .map_err(|_| PrivateIndexError::Authentication)?,
                )
            }
            (None, None) => None,
            _ => return Err(PrivateIndexError::Dependency),
        };
        let checked = if let (Some(expected), Some(plain)) = (&descriptor, &boundary_plain) {
            let child =
                PrivateIndexNodeFrame::decode(plain, self.context, NodePlacementData::Child)?;
            Some(PrivateIndexChildRefData {
                context: self.context,
                node: expected.check_child(&child)?,
            })
        } else {
            None
        };
        rewrite_private_branch_data(
            &node,
            chosen,
            &PrivateIndexChildRefData {
                context: self.context,
                node: selected.reference,
            },
            replacements,
            checked.as_ref(),
            parent.placement,
        )
    }
}
impl std::fmt::Debug for PrivateIndexPathData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivateIndexPathData")
            .field("depth", &self.nodes.len())
            .finish_non_exhaustive()
    }
}
