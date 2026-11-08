//! Shared original own-update path algorithm. The extra private strategy only
//! reaches path derivation's iterator choice; it supplies no SDK/Source permit.
use super::*;
use crate::group::mls_group::text_add_execution::TextAddExecution;
impl TreeSyncDiff<'_> {
    #[allow(clippy::too_many_arguments)] // Same original SDK algorithm/signature plus private strategy.
    pub(crate) fn apply_own_update_path_for_execution(
        &mut self,
        rand: &impl OpenMlsRand,
        crypto: &impl OpenMlsCrypto,
        signer: &impl Signer,
        ciphersuite: Ciphersuite,
        commit_type: &CommitType,
        group_id: GroupId,
        leaf_index: LeafNodeIndex,
        leaf_node_params: UpdateLeafNodeParams,
        vc_override: Option<OwnUpdatePathOverride>,
        execution: TextAddExecution,
    ) -> Result<UpdatePathResult, TreeSyncAddLeaf> {
        // For External Commits, we temporarily add a placeholder leaf node to the tree, because it
        // might be required to make the tree grow to the right size. If we
        // don't do that, calculating the direct path might fail. It's important
        // to not do anything with the value of that leaf until it has been
        // replaced.
        if matches!(commit_type, CommitType::External) {
            let leaf_node = LeafNode::new_placeholder();
            self.add_leaf(leaf_node)?;
        }

        #[cfg(feature = "virtual-clients-draft")]
        let (path_secret_override, leaf_keypair_override) = match vc_override {
            Some(OwnUpdatePathOverride {
                path_secret,
                leaf_encryption_keypair,
            }) => (Some(path_secret), Some(leaf_encryption_keypair)),
            None => (None, None),
        };
        #[cfg(not(feature = "virtual-clients-draft"))]
        let path_secret_override = vc_override.map(|o| o.path_secret);

        // We calculate the parent hash so that we can use it for a fresh leaf
        let (path, update_path_nodes, parent_keypairs, commit_secret) = self
            .derive_path_for_execution(
                rand,
                crypto,
                ciphersuite,
                leaf_index,
                path_secret_override,
                execution,
            )?;
        let parent_hash = self
            .process_update_path(crypto, ciphersuite, leaf_index, path)
            .map_err(|e| match e {
                ApplyUpdatePathError::LibraryError(e) => TreeSyncAddLeaf::LibraryError(e),
                _ => TreeSyncAddLeaf::LibraryError(LibraryError::custom(
                    "Unexpected error in process_update_path",
                )),
            })?;

        // We generate the new leaf with all parameters
        let (leaf_node, node_keypair) = LeafNode::new_with_parent_hash(
            rand,
            crypto,
            ciphersuite,
            &parent_hash,
            leaf_node_params,
            group_id,
            leaf_index,
            signer,
            #[cfg(feature = "virtual-clients-draft")]
            leaf_keypair_override,
        )?;

        // We insert the fresh leaf into the tree.
        self.diff.replace_leaf(leaf_index, leaf_node.into());

        // Prepend parent keypairs with node keypair
        let mut keypairs = vec![node_keypair];
        keypairs.extend(parent_keypairs);

        Ok((update_path_nodes, keypairs, commit_secret))
    }
}
