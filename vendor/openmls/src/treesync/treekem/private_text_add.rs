//! Same path-encryption core, with independently selected PRIVATE serial execution.
//! Default calls retain their original native parallel iterators and node operation.
use super::*;
use crate::group::mls_group::text_add_execution::TextAddExecution;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
impl TreeSyncDiff<'_> {
    pub(crate) fn encrypt_path_for_execution(
        &self,
        crypto: &impl OpenMlsCrypto,
        ciphersuite: Ciphersuite,
        path: &[PlainUpdatePathNode],
        group_context: &[u8],
        exclusion_list: &HashSet<&LeafNodeIndex>,
        target: (LeafNodeIndex, TextAddExecution),
    ) -> Result<Vec<UpdatePathNode>, LibraryError> {
        let (own_leaf_index, execution) = target;
        // Copath resolutions with the corresponding public keys.
        let copath_resolutions = self
            .filtered_copath_resolutions(own_leaf_index, exclusion_list)
            .into_iter()
            .map(|resolution| {
                resolution
                    .into_iter()
                    .map(|(_, node_ref)| match node_ref {
                        NodeReference::Leaf(leaf) => leaf.encryption_key().clone(),
                        NodeReference::Parent(parent) => parent.encryption_key().clone(),
                    })
                    .collect::<Vec<EncryptionKey>>()
            })
            .collect::<Vec<Vec<EncryptionKey>>>();

        // There should be as many copath resolutions.
        debug_assert_eq!(copath_resolutions.len(), path.len());

        // Both branches share this exact per-node computation. Serial selects
        // node's serial encryption too, so neither outer nor nested Rayon runs.
        let encrypt = |(node, resolution): (&PlainUpdatePathNode, &Vec<EncryptionKey>)| {
            if execution.is_serial() {
                node.encrypt_for_execution(
                    crypto,
                    ciphersuite,
                    resolution,
                    group_context,
                    execution,
                )
            } else {
                node.encrypt(crypto, ciphersuite, resolution, group_context)
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        let result = if execution.is_serial() {
            path.iter()
                .zip(copath_resolutions.iter())
                .map(encrypt)
                .collect::<Result<Vec<UpdatePathNode>, LibraryError>>()
        } else {
            path.par_iter()
                .zip(copath_resolutions.par_iter())
                .map(encrypt)
                .collect::<Result<Vec<UpdatePathNode>, LibraryError>>()
        };
        #[cfg(target_arch = "wasm32")]
        let result = path
            .iter()
            .zip(copath_resolutions.iter())
            .map(encrypt)
            .collect::<Result<Vec<UpdatePathNode>, LibraryError>>();
        result
    }
}
