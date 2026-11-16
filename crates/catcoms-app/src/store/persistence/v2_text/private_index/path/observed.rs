//! Actual complete encrypted frame lengths, recorded ONLY after successful
//! expected-ID/AEAD/context/summary/child verification. A ceiling is not actual size.
use super::*;
impl PrivateIndexPathData {
    pub(in crate::store) fn original_frame_sizes_data(
        &self,
    ) -> impl ExactSizeIterator<Item = (NodeRefData, usize)> + '_ {
        self.nodes
            .iter()
            .map(|node| (node.reference, node.frame_bytes))
    }
}
