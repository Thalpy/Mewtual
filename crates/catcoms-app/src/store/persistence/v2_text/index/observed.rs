//! Exact observed encrypted-frame geometry, never inferred from a maximum/plain size.
//! Nodes enter this owner only after full ID/AEAD/context/child/claim verification.
use super::*;

impl AcceptedIndexPathData {
    pub(in crate::store) fn original_frame_sizes_data(
        &self,
    ) -> impl ExactSizeIterator<Item = (NodeRefData, usize)> + '_ {
        self.nodes
            .iter()
            .map(|node| (node.reference, node.frame_bytes))
    }
}
