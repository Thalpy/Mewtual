//! Conservative COMPILE-CHECKED native element caps for a separately audited
//! one-Add invoice. These exclude all transitive allocations and grant no SDK,
//! resource, graph, Source, membership or publication authority.
//!
//! A common upper cap keeps the host invoice small without exposing every
//! private SDK type/field. Recompile assertions, not observed runtime lengths,
//! guard upgrades/compiler/target layout changes. Callers still count actual
//! bounded vector growth requests, maps, payloads, copies and crypto separately.
use crate::{
    binary_tree::{LeafNodeIndex, ParentNodeIndex},
    group::mls_group::{proposal_store::QueuedProposal, Member},
    tree::{
        secret_tree::SecretTreeNode,
        sender_ratchet::{DecryptionRatchet, RatchetKeyMaterial, RatchetSecret, SenderRatchet},
    },
    treesync::{
        node::{
            encryption_keys::EncryptionKeyPair,
            parent_node::{ParentNode, PlainUpdatePathNode},
        },
        treesync_node::{TreeSyncLeafNode, TreeSyncParentNode},
    },
};

/// Inline byte upper bound for EACH listed Vec/map element, never its payload.
pub const TEXT_ADD_ELEMENT_BYTES: usize = 512;
/// Native alignment ceiling used in conservative map-node layout arithmetic.
pub const TEXT_ADD_ELEMENT_ALIGN: usize = 64;
/// Smaller inline cap for references, indices and fixed output-buffer headers.
pub const TEXT_ADD_REFERENCE_BYTES: usize = 64;

macro_rules! bounded {
    ($($ty:ty),+ $(,)?) => {$(
        const _: () = assert!(core::mem::size_of::<$ty>() <= TEXT_ADD_ELEMENT_BYTES);
        const _: () = assert!(core::mem::align_of::<$ty>() <= TEXT_ADD_ELEMENT_ALIGN);
    )+};
}
bounded!(
    TreeSyncLeafNode,
    TreeSyncParentNode,
    Option<TreeSyncLeafNode>,
    Option<TreeSyncParentNode>,
    Option<SecretTreeNode>,
    Option<SenderRatchet>,
    RatchetSecret,
    DecryptionRatchet,
    RatchetKeyMaterial,
    Member,
    (LeafNodeIndex, TreeSyncLeafNode),
    (ParentNodeIndex, TreeSyncParentNode),
    ((ParentNodeIndex, ParentNode), EncryptionKeyPair),
    (
        ((ParentNodeIndex, ParentNode), EncryptionKeyPair),
        PlainUpdatePathNode
    ),
    (crate::ciphersuite::hash_ref::ProposalRef, QueuedProposal),
    crate::ciphersuite::Secret,
    Option<crate::treesync::Node>,
    openmls_traits::types::HpkeCiphertext,
);

macro_rules! reference_bounded {
    ($($ty:ty),+ $(,)?) => {$(
        const _: () = assert!(core::mem::size_of::<$ty>() <= TEXT_ADD_REFERENCE_BYTES);
        const _: () = assert!(core::mem::align_of::<$ty>() <= TEXT_ADD_ELEMENT_ALIGN);
    )+};
}
reference_bounded!(
    crate::binary_tree::array_representation::TreeNodeIndex,
    (
        crate::binary_tree::array_representation::TreeNodeIndex,
        crate::treesync::node::NodeReference<'static>
    ),
    crate::treesync::EncryptionKey,
    Vec<u8>,
    Vec<crate::treesync::EncryptionKey>,
    openmls_traits::types::HpkeCiphertext,
    (
        crate::group::GroupEpoch,
        crate::schedule::ResumptionPskSecret
    ),
    Option<SecretTreeNode>,
);
