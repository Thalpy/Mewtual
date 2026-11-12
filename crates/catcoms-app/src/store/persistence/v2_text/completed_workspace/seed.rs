//! Full original promotion-anchor verification for a known operational workspace.
//!
//! The independent actual Source/Current proof supplied C before this finite read. The
//! seed cannot choose its own namespace, original header/image, manifest or genesis parent.
//! Embedded old private Directory bytes need not still be named after genuine retirement.
use super::*;
use crate::store::persistence::v2_text::archive::TextArchiveOperationData;
use crate::store::persistence::v2_text::head::TextParentData;
use catcoms_replication::chat::acceptance_tree::TreeContext;
use catcoms_replication::chat::cold_index::{
    DirectoryData, DirectoryTreeData, IndexContextData, IndexScopeData, NodeFrame, NodeKindData,
    NodePlacementData, ValueData,
};
use catcoms_storage::private_chat_page::open_private_chat_page;

pub(super) fn verify_anchor(
    plan: &CompletedBase,
    key: &[u8; 32],
    frame: &[u8],
) -> Result<(), AppError> {
    let plain = open_control_data(key, TextControlKindData::Seed.id_data(frame), frame)?;
    let seed = super::super::promotion::seed::parse_data(&plain)?;
    if seed.context != plan.context {
        // The retained original G anchor stays independent of the current later
        // WC/O/H. It cannot choose C or the original profile/header trust.
        return Err(refused());
    }
    // The common parser checks the exact old-ciphertext hash against C.genesis_parent.
    // C was derived from the real retained original frame, not this journal's labels.
    let head_plain = open_control_data(
        key,
        TextControlKindData::Head.id_data(seed.head_frame),
        seed.head_frame,
    )?;
    let head = TextHeadData::decode_data(&head_plain)?;
    let archive_id = TextControlKindData::Archive.id_data(seed.archive_frame);
    if head.context != plan.context
        || head.generation != 1
        || head.selection_attempt != 1
        || head.previous != TextParentData::Genesis(plan.context.spec_data().genesis_parent)
        || head.pending != TextPendingData::None
        || head.tokens != TextRootData::Empty
        || head.publications != TextRootData::Empty
        || head.queue_front != 0
        || head.queue_next != 0
        || head.archive.as_bytes() != archive_id.as_bytes()
    {
        return Err(refused());
    }
    let archive_plain = open_control_data(key, archive_id, seed.archive_frame)?;
    let archive = TextArchiveData::decode_data(&archive_plain)?;
    let a = archive.spec_data();
    if a.context != plan.context.id_data()?
        || a.operation != TextArchiveOperationData::Genesis
        || a.previous.is_some()
        || a.manifest_sequence != 1
        || a.manifest != plan.original_manifest
        || a.header != &plan.original_header
        || a.image != plan.original_image.as_slice()
    {
        return Err(refused());
    }
    let TextRootData::Present(root) = head.directory else {
        return Err(refused());
    };
    let directory_plain = open_private_chat_page(key, root.page, seed.directory_frame)?;
    let directory =
        NodeFrame::decode(&directory_plain, NodePlacementData::Root).map_err(|_| refused())?;
    let c = plan.context.spec_data();
    if directory.context()
        != (IndexContextData {
            vault: c.layout.vault,
            group: c.group,
            profile: c.profile,
            scope: IndexScopeData::Directory,
        })
        || directory.summary() != root.summary
        || directory.kind() != NodeKindData::Leaf
        || directory.row_count() != 1
    {
        return Err(refused());
    }
    directory
        .check_claim_bounds_data(None)
        .map_err(|_| refused())?;
    let expected = DirectoryData {
        tree: DirectoryTreeData::Empty,
        accepted_leaf_count: 0,
        unique_action_count: 0,
        accepted_root: TreeContext {
            profile: c.profile,
            admission: plan.original_admission,
        }
        .empty_hash(),
        own_tip_header: plan.original_header.id(),
        tip_body: c.admission,
        prefix_manifest: plan.original_manifest,
        prefix_sequence: 1,
        prefix_head: head.archive,
    };
    let row = directory.leaf_row(0).map_err(|_| refused())?;
    if row.key != *c.admission.as_bytes()
        || row.value != ValueData::Directory(expected)
        || head.live_bytes != (seed.archive_frame.len() + seed.directory_frame.len()) as u128
        || head.live_objects != 2
    {
        return Err(refused());
    }
    Ok(())
}
