//! Shared per-element TreeKEM computations with a private serial iterator
//! strategy. Default/legacy callers retain all original native Rayon branches.
use super::*;
use crate::group::mls_group::text_add_execution::TextAddExecution;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

impl PlainUpdatePathNode {
    pub(in crate::treesync) fn encrypt_for_execution(
        &self,
        crypto: &impl OpenMlsCrypto,
        ciphersuite: Ciphersuite,
        public_keys: &[EncryptionKey],
        group_context: &[u8],
        execution: TextAddExecution,
    ) -> Result<UpdatePathNode, LibraryError> {
        let encrypt = |pk: &EncryptionKey| {
            self.path_secret
                .encrypt(crypto, ciphersuite, pk, group_context)
        };
        #[cfg(not(target_arch = "wasm32"))]
        let encrypted_path_secrets = if execution.is_serial() {
            public_keys
                .iter()
                .map(encrypt)
                .collect::<Result<Vec<HpkeCiphertext>, LibraryError>>()?
        } else {
            public_keys
                .par_iter()
                .map(encrypt)
                .collect::<Result<Vec<HpkeCiphertext>, LibraryError>>()?
        };
        #[cfg(target_arch = "wasm32")]
        let encrypted_path_secrets = {
            let _ = execution;
            public_keys
                .iter()
                .map(encrypt)
                .collect::<Result<Vec<HpkeCiphertext>, LibraryError>>()?
        };
        Ok(UpdatePathNode {
            public_key: self.public_key.clone(),
            encrypted_path_secrets,
        })
    }
}
impl ParentNode {
    pub(in crate::treesync) fn derive_path_for_execution(
        crypto: &impl OpenMlsCrypto,
        ciphersuite: Ciphersuite,
        path_secret: PathSecret,
        path_indices: Vec<ParentNodeIndex>,
        execution: TextAddExecution,
    ) -> Result<PathDerivationResult, LibraryError> {
        let mut next_path_secret = path_secret;
        let mut path_secrets = Vec::with_capacity(path_indices.len());
        for _ in 0..path_indices.len() {
            let path_secret = next_path_secret;
            next_path_secret = path_secret.derive_path_secret(crypto, ciphersuite)?;
            path_secrets.push(path_secret);
        }
        type Derived = (
            ((ParentNodeIndex, ParentNode), EncryptionKeyPair),
            PlainUpdatePathNode,
        );
        let derive = |(path_secret, index): (PathSecret, ParentNodeIndex)| {
            let keypair = path_secret.derive_key_pair(crypto, ciphersuite)?;
            let parent_node = ParentNode::from(keypair.public_key().clone());
            let update_path_node = PlainUpdatePathNode {
                public_key: keypair.public_key().clone(),
                path_secret,
            };
            Ok((((index, parent_node), keypair), update_path_node))
        };
        #[cfg(not(target_arch = "wasm32"))]
        let derived = if execution.is_serial() {
            path_secrets
                .into_iter()
                .zip(path_indices)
                .map(derive)
                .collect::<Result<Vec<Derived>, LibraryError>>()?
        } else {
            path_secrets
                .into_par_iter()
                .zip(path_indices)
                .map(derive)
                .collect::<Result<Vec<Derived>, LibraryError>>()?
        };
        #[cfg(target_arch = "wasm32")]
        let derived = {
            let _ = execution;
            path_secrets
                .into_iter()
                .zip(path_indices)
                .map(derive)
                .collect::<Result<Vec<Derived>, LibraryError>>()?
        };
        // Preserve the original owning collect->unzip->unzip sequence. Their
        // overlapping native Vec backing remains an operation-invoice term.
        let (path_with_keypairs, update_path_nodes): (
            Vec<((ParentNodeIndex, ParentNode), EncryptionKeyPair)>,
            Vec<PlainUpdatePathNode>,
        ) = derived.into_iter().unzip();
        let (path, keypairs) = path_with_keypairs.into_iter().unzip();
        let commit_secret = next_path_secret.into();
        Ok((path, update_path_nodes, keypairs, commit_secret))
    }
}
