//! Native wrapper-size DATA for a separately audited, independently funded text Add.
//!
//! No operation, object, graph, key or verification authority crosses this API.
//! Every field is the exact compiled `size_of` of ONE named wrapper. Container
//! backing, map nodes, secret/key payloads, allocator overhead, closure/stack
//! frames and transitive allocations are NOT included. A caller must audit
//! simultaneous multiplicities, dynamic capacities, crypto scratch, codec growth
//! and all phases separately before claiming a complete operation reservation.
//!
//! The module is deliberately absent on wasm and unsupported structural feature
//! families. Enabling one requires a new profile/allocation audit, not reuse of
//! this DATA as evidence that an operation is safe or paid.

/// Exact native inline wrapper sizes for the pinned, ordinary-feature SDK.
///
/// Values vary with target/compiler/layout and are not wire or persistence ABI.
/// There is deliberately no aggregate bound, object adopter or admission method.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TextAddLayoutData {
    /// Inline bytes of one `crate::group::mls_group::CreateCommitResult`, excluding every transitive allocation.
    pub create_commit_result_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::PendingCommitState`, excluding every transitive allocation.
    pub pending_commit_state_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::MlsGroupState`, excluding every transitive allocation.
    pub mls_group_state_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::commit_builder::Initial`, excluding every transitive allocation.
    pub commit_initial_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::commit_builder::LoadedPsks`, excluding every transitive allocation.
    pub commit_loaded_psks_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::commit_builder::Complete`, excluding every transitive allocation.
    pub commit_complete_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::commit_builder::CommitBuilder<'static, crate::group::mls_group::commit_builder::Initial>`, excluding every transitive allocation.
    pub commit_builder_initial_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::commit_builder::CommitBuilder<'static, crate::group::mls_group::commit_builder::LoadedPsks>`, excluding every transitive allocation.
    pub commit_builder_loaded_psks_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::commit_builder::CommitBuilder<'static, crate::group::mls_group::commit_builder::Complete>`, excluding every transitive allocation.
    pub commit_builder_complete_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::commit_builder::CommitMessageBundle`, excluding every transitive allocation.
    pub commit_message_bundle_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::staged_commit::StagedCommit`, excluding every transitive allocation.
    pub staged_commit_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::staged_commit::StagedCommitState`, excluding every transitive allocation.
    pub staged_commit_state_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::staged_commit::MemberStagedCommitState`, excluding every transitive allocation.
    pub member_staged_commit_state_bytes: usize,
    /// Inline bytes of one `crate::treesync::TreeSync`, excluding every transitive allocation.
    pub tree_sync_bytes: usize,
    /// Inline bytes of one `crate::treesync::diff::TreeSyncDiff<'static>`, excluding every transitive allocation.
    pub tree_sync_diff_bytes: usize,
    /// Inline bytes of one `crate::treesync::diff::StagedTreeSyncDiff`, excluding every transitive allocation.
    pub staged_tree_sync_diff_bytes: usize,
    /// Inline bytes of one `crate::group::public_group::diff::PublicGroupDiff<'static>`, excluding every transitive allocation.
    pub public_group_diff_bytes: usize,
    /// Inline bytes of one `crate::group::public_group::diff::StagedPublicGroupDiff`, excluding every transitive allocation.
    pub staged_public_group_diff_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::past_secrets::MessageSecretsStore`, excluding every transitive allocation.
    pub message_secrets_store_bytes: usize,
    /// Inline bytes of one `crate::schedule::message_secrets::MessageSecrets`, excluding every transitive allocation.
    pub message_secrets_bytes: usize,
    /// Inline bytes of one `crate::tree::secret_tree::SecretTree`, excluding every transitive allocation.
    pub secret_tree_bytes: usize,
    /// Inline bytes of one `crate::tree::secret_tree::SecretTreeNode`, excluding every transitive allocation.
    pub secret_tree_node_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::proposal_store::ProposalQueue`, excluding every transitive allocation.
    pub proposal_queue_bytes: usize,
    /// Inline bytes of one `crate::group::mls_group::proposal_store::QueuedProposal`, excluding every transitive allocation.
    pub queued_proposal_bytes: usize,
    /// Inline bytes of one `crate::schedule::GroupEpochSecrets`, excluding every transitive allocation.
    pub group_epoch_secrets_bytes: usize,
    /// Inline bytes of one `crate::schedule::psk::store::ResumptionPskStore`, excluding every transitive allocation.
    pub resumption_psk_store_bytes: usize,
    /// Inline bytes of one `crate::treesync::node::encryption_keys::EncryptionKeyPair`, excluding every transitive allocation.
    pub encryption_key_pair_bytes: usize,
    /// Inline bytes of one `crate::treesync::node::parent_node::ParentNode`, excluding every transitive allocation.
    pub parent_node_bytes: usize,
    /// Inline bytes of one `crate::treesync::node::parent_node::PlainUpdatePathNode`, excluding every transitive allocation.
    pub plain_update_path_node_bytes: usize,
    /// Inline bytes of one `crate::treesync::treekem::UpdatePath`, excluding every transitive allocation.
    pub update_path_bytes: usize,
    /// Inline bytes of one `crate::treesync::treekem::UpdatePathNode`, excluding every transitive allocation.
    pub update_path_node_bytes: usize,
    /// Inline bytes of one `crate::framing::MlsMessageOut`, excluding every transitive allocation.
    pub mls_message_out_bytes: usize,
    /// Inline bytes of one `crate::messages::group_info::GroupInfo`, excluding every transitive allocation.
    pub group_info_bytes: usize,
    /// Inline bytes of one `crate::messages::Welcome`, excluding every transitive allocation.
    pub welcome_bytes: usize,
    /// Inline bytes of one `crate::key_packages::KeyPackageIn`, excluding every transitive allocation.
    pub key_package_in_bytes: usize,
    /// Inline bytes of one `crate::treesync::node::leaf_node::LeafNode`, excluding every transitive allocation.
    pub leaf_node_bytes: usize,
    /// Inline bytes of one `crate::treesync::node::leaf_node::LeafNodeIn`, excluding every transitive allocation.
    pub leaf_node_in_bytes: usize,
    /// Inline bytes of one `crate::ciphersuite::Secret`, excluding every transitive allocation.
    pub secret_bytes: usize,
    /// Inline bytes of one `openmls_traits::types::HpkeCiphertext`, excluding every transitive allocation.
    pub hpke_ciphertext_bytes: usize,
    /// Inline bytes of one private `KeyPackageTbs`, excluding every transitive allocation.
    pub key_package_tbs_bytes: usize,
    /// Inline bytes of one private `KeyPackageTbsIn`, excluding every transitive allocation.
    pub key_package_tbs_in_bytes: usize,
    /// Inline bytes of one private `LeafNodePayload`, excluding every transitive allocation.
    pub leaf_node_payload_bytes: usize,
}

/// Return numeric native wrapper sizes without taking or inspecting any SDK object.
///
/// This const function cannot authorize a mutation or reserve memory. In
/// particular, summing its fields is not an Add invoice: nested wrappers overlap
/// and dynamic allocations and concurrent copies require separate analysis.
/// CommitBuilder terms use its actual borrowed-group default generic parameter;
/// they do not include another owning MlsGroup or its graph.
pub const fn text_add_layout_data() -> TextAddLayoutData {
    TextAddLayoutData {
        create_commit_result_bytes: core::mem::size_of::<crate::group::mls_group::CreateCommitResult>(
        ),
        pending_commit_state_bytes: core::mem::size_of::<crate::group::mls_group::PendingCommitState>(
        ),
        mls_group_state_bytes: core::mem::size_of::<crate::group::mls_group::MlsGroupState>(),
        commit_initial_bytes: core::mem::size_of::<crate::group::mls_group::commit_builder::Initial>(
        ),
        commit_loaded_psks_bytes: core::mem::size_of::<
            crate::group::mls_group::commit_builder::LoadedPsks,
        >(),
        commit_complete_bytes: core::mem::size_of::<
            crate::group::mls_group::commit_builder::Complete,
        >(),
        commit_builder_initial_bytes: core::mem::size_of::<
            crate::group::mls_group::commit_builder::CommitBuilder<
                'static,
                crate::group::mls_group::commit_builder::Initial,
            >,
        >(),
        commit_builder_loaded_psks_bytes: core::mem::size_of::<
            crate::group::mls_group::commit_builder::CommitBuilder<
                'static,
                crate::group::mls_group::commit_builder::LoadedPsks,
            >,
        >(),
        commit_builder_complete_bytes: core::mem::size_of::<
            crate::group::mls_group::commit_builder::CommitBuilder<
                'static,
                crate::group::mls_group::commit_builder::Complete,
            >,
        >(),
        commit_message_bundle_bytes: core::mem::size_of::<
            crate::group::mls_group::commit_builder::CommitMessageBundle,
        >(),
        staged_commit_bytes: core::mem::size_of::<
            crate::group::mls_group::staged_commit::StagedCommit,
        >(),
        staged_commit_state_bytes: core::mem::size_of::<
            crate::group::mls_group::staged_commit::StagedCommitState,
        >(),
        member_staged_commit_state_bytes: core::mem::size_of::<
            crate::group::mls_group::staged_commit::MemberStagedCommitState,
        >(),
        tree_sync_bytes: core::mem::size_of::<crate::treesync::TreeSync>(),
        tree_sync_diff_bytes: core::mem::size_of::<crate::treesync::diff::TreeSyncDiff<'static>>(),
        staged_tree_sync_diff_bytes: core::mem::size_of::<crate::treesync::diff::StagedTreeSyncDiff>(
        ),
        public_group_diff_bytes: core::mem::size_of::<
            crate::group::public_group::diff::PublicGroupDiff<'static>,
        >(),
        staged_public_group_diff_bytes: core::mem::size_of::<
            crate::group::public_group::diff::StagedPublicGroupDiff,
        >(),
        message_secrets_store_bytes: core::mem::size_of::<
            crate::group::mls_group::past_secrets::MessageSecretsStore,
        >(),
        message_secrets_bytes: core::mem::size_of::<crate::schedule::message_secrets::MessageSecrets>(
        ),
        secret_tree_bytes: core::mem::size_of::<crate::tree::secret_tree::SecretTree>(),
        secret_tree_node_bytes: core::mem::size_of::<crate::tree::secret_tree::SecretTreeNode>(),
        proposal_queue_bytes: core::mem::size_of::<
            crate::group::mls_group::proposal_store::ProposalQueue,
        >(),
        queued_proposal_bytes: core::mem::size_of::<
            crate::group::mls_group::proposal_store::QueuedProposal,
        >(),
        group_epoch_secrets_bytes: core::mem::size_of::<crate::schedule::GroupEpochSecrets>(),
        resumption_psk_store_bytes: core::mem::size_of::<
            crate::schedule::psk::store::ResumptionPskStore,
        >(),
        encryption_key_pair_bytes: core::mem::size_of::<
            crate::treesync::node::encryption_keys::EncryptionKeyPair,
        >(),
        parent_node_bytes: core::mem::size_of::<crate::treesync::node::parent_node::ParentNode>(),
        plain_update_path_node_bytes: core::mem::size_of::<
            crate::treesync::node::parent_node::PlainUpdatePathNode,
        >(),
        update_path_bytes: core::mem::size_of::<crate::treesync::treekem::UpdatePath>(),
        update_path_node_bytes: core::mem::size_of::<crate::treesync::treekem::UpdatePathNode>(),
        mls_message_out_bytes: core::mem::size_of::<crate::framing::MlsMessageOut>(),
        group_info_bytes: core::mem::size_of::<crate::messages::group_info::GroupInfo>(),
        welcome_bytes: core::mem::size_of::<crate::messages::Welcome>(),
        key_package_in_bytes: core::mem::size_of::<crate::key_packages::KeyPackageIn>(),
        leaf_node_bytes: core::mem::size_of::<crate::treesync::node::leaf_node::LeafNode>(),
        leaf_node_in_bytes: core::mem::size_of::<crate::treesync::node::leaf_node::LeafNodeIn>(),
        secret_bytes: core::mem::size_of::<crate::ciphersuite::Secret>(),
        hpke_ciphertext_bytes: core::mem::size_of::<openmls_traits::types::HpkeCiphertext>(),
        key_package_tbs_bytes: crate::key_packages::TEXT_ADD_KEY_PACKAGE_TBS_LAYOUT_BYTES,
        key_package_tbs_in_bytes:
            crate::key_packages::key_package_in::TEXT_ADD_KEY_PACKAGE_TBS_IN_LAYOUT_BYTES,
        leaf_node_payload_bytes:
            crate::treesync::node::leaf_node::TEXT_ADD_LEAF_NODE_PAYLOAD_LAYOUT_BYTES,
    }
}
