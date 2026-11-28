//! Reconstruct the paid, authenticated path after one checked point
//! replacement. The caller owns the namespace-specific prior row check,
//! covering write, Head selection and any old-page retirement.
use super::*;
use crate::AppError;

impl PrivateIndexPathData {
    pub(in crate::store) fn rewrite_selected_path(
        &self,
        context: PrivateIndexContextData,
        replacement: &PrivateIndexLeafRowData,
        storage_key: &[u8; 32],
        mut read: impl FnMut(PrivateChatPageId) -> std::result::Result<Zeroizing<Vec<u8>>, AppError>,
        mut output: impl FnMut(&[u8], NodeSummaryData) -> std::result::Result<NodeRefData, AppError>,
    ) -> std::result::Result<(NodeRefData, Vec<NodeRefData>), AppError> {
        let original = self
            .original_frame_sizes_data()
            .map(|(reference, _)| reference)
            .collect::<Vec<_>>();
        let parents = original.len().checked_sub(1).ok_or_else(invalid)?;
        let leaf = self.rewrite_leaf_data(replacement).map_err(|_| invalid())?;
        let mut children = revised_children(context, &leaf, &mut output)?;
        for level in (0..parents).rev() {
            let boundary = self
                .boundary_page_data(level, children.len())
                .map_err(|_| invalid())?
                .map(&mut read)
                .transpose()?;
            let rewritten = self
                .rewrite_branch_data(
                    level,
                    &children,
                    boundary
                        .as_deref()
                        .map(|frame| (storage_key, frame.as_slice())),
                )
                .map_err(|_| invalid())?;
            children = revised_children(context, &rewritten, &mut output)?;
        }
        let root = match children.as_slice() {
            [child] => child.node,
            [_, _] => {
                let plain = encode_private_branch_data(context, &children, NodePlacementData::Root)
                    .map_err(|_| invalid())?;
                let node = PrivateIndexNodeFrame::decode(&plain, context, NodePlacementData::Root)
                    .map_err(|_| invalid())?;
                output(&plain, node.summary())?
            }
            _ => return Err(invalid()),
        };
        Ok((root, original))
    }
}

fn revised_children(
    context: PrivateIndexContextData,
    update: &PrivateIndexRewriteData,
    output: &mut impl FnMut(&[u8], NodeSummaryData) -> std::result::Result<NodeRefData, AppError>,
) -> std::result::Result<Vec<PrivateIndexChildRefData>, AppError> {
    update
        .nodes_data()
        .iter()
        .map(|node| {
            let node = output(node.plain_data(), node.summary_data())?;
            Ok(PrivateIndexChildRefData { context, node })
        })
        .collect()
}

fn invalid() -> AppError {
    crate::store::critical_namespaces::refused()
}
