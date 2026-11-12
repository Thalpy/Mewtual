//! Embedded first-A consistency/authentication DATA, not a source/Current/Saved issuer.
//!
//! The actual transaction supplies its genuine selected snapshot and original SDK custody.
//! This checks all embedded immutable bytes before that transaction can stage them. It
//! deliberately does not reconstruct an SDK packet, ReplayMath, admission or cold owner.
use super::super::archive::{TextArchiveData, TextArchiveOperationData};
use super::super::capture::SelectedV2TextHead;
use super::super::control::{open_control_data, TextControlIdData};
use super::super::head::{
    index_capture_data, TextHeadData, TextIndexCaptureRoleData, TextPendingData, TextRootData,
};
use super::super::private_index::{
    PrivateIndexContextData, PrivateIndexKindData, PrivateIndexNodeFrame, PrivateIndexValueData,
};
use super::super::request::{RequestPhaseData, TextRequestDigestData, TokenValueData};
use super::super::retain::{
    cipher_digest, CipherChunkData, RetainedActionData, MAX_ENCRYPTED_PAYLOAD_BYTES,
};
use super::*;
mod completed;
use catcoms_replication::chat::cold_actions::{
    assemble_action, ActionChunk, ActionPagesExpectation, MAX_ACTION_CHUNK_PAGE_BYTES,
};
use catcoms_replication::chat::cold_index::{NodeKindData, NodePlacementData};
use catcoms_replication::chat::cold_manifest::{ArchiveOperation, ManifestPage, PageRole};
use catcoms_replication::chat::Body;
use catcoms_storage::private_chat_page::open_private_chat_page;

/// Additional real decoder/assembly overlap, included by the actual Cell's SAME16MiB
/// invoice. Parent/query/SDK and all retained WAL/candidate buffers remain separate terms.
pub(in crate::store) const FIRST_RETAIN_WAL_VERIFY_BUFFER_BYTES: usize = 4
    * MAX_ACTION_CHUNK_PAGE_BYTES
    + 4 * 66057
    + 3 * MAX_ENCRYPTED_PAYLOAD_BYTES
    + 4 * 16470
    + 64 * 1024;
pub(in crate::store) const FIRST_RETAIN_WAL_VERIFY_METADATA_BYTES: usize = MAX_ARTIFACTS
    * std::mem::size_of::<Option<TextWalArtifactData<'static>>>()
    + 2 * std::mem::size_of::<TextHeadData>()
    + 2 * 787
    + 8192;

/// Pure parent DATA from either the genuine captured snapshot or its already-paid
/// transaction copy. Constructing this does not issue original/live source authority.
pub(in crate::store::persistence::v2_text) struct RetainWalParentData<'a> {
    pub(in crate::store::persistence::v2_text) head: &'a TextHeadData,
    pub(in crate::store::persistence::v2_text) archive: &'a TextArchiveData,
    pub(in crate::store::persistence::v2_text) id: TextHeadIdData,
    pub(in crate::store::persistence::v2_text) frame: &'a [u8],
}

impl TextWorkWalFrameData<'_> {
    /// Authenticate every first-A dependency and compare the full original parent bytes.
    /// The snapshot is genuine historical DATA; the actual write still needs NEW Current,
    /// exact lane/predecessor and same-scope/native admission. This method calls no RNG,
    /// clock, signer, exporter, FS or source callback and issues no durable/semantic effect.
    pub(in crate::store) fn verify_first_retained_a_data(
        &self,
        key: &[u8; 32],
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        let source = selected.physical_query_data();
        self.verify_first_retained_a_parent_data(
            key,
            RetainWalParentData {
                head: selected.head_data(),
                archive: selected.archive_data(),
                id: selected.head_id_data(),
                frame: source.head_frame,
            },
        )
    }
    pub(in crate::store::persistence::v2_text) fn verify_first_retained_a_parent_data(
        &self,
        key: &[u8; 32],
        selected: RetainWalParentData<'_>,
    ) -> Result<(), AppError> {
        self.validate_data()?;
        if self.operation != TextWorkWalOperationData::Retain {
            return Err(refused());
        }
        // A genuine initial token root is Empty. It has NO old token page to retire;
        // generic WORK syntax allowing later carry entries cannot authorize one here.
        let TextWorkWalCleanupData::Carry(raw_carry) = &self.cleanup else {
            return Err(refused());
        };
        let carry = RetirementCarryData::decode_data(raw_carry)?;
        if !carry.entries_data().is_empty() || carry.roots_data() != [self.install_parent] {
            return Err(refused());
        }
        let parent = selected.head;
        let context = &parent.context;
        let c = context.spec_data();
        if self.context != context.id_data()?
            || self.install_parent != selected.id
            || self.parent_frame != selected.frame
            || parent.generation != 1
            || parent.selection_attempt != 1
            || parent.pending != TextPendingData::None
            || parent.tokens != TextRootData::Empty
            || parent.queue_front != 0
            || parent.queue_next != 0
            || parent.publications != TextRootData::Empty
            || self.public_parent != selected.archive.spec_data().header.id()
        {
            return Err(refused());
        }
        let candidate_plain = open_control_data(
            key,
            TextControlKindData::Head.id_data(self.candidate_frame),
            self.candidate_frame,
        )?;
        let candidate = TextHeadData::decode_data(&candidate_plain)?;
        candidate.require_successor_data(parent, self.install_parent)?;
        let TextPendingData::Retained(pending) = candidate.pending else {
            return Err(refused());
        };
        if pending.transaction != self.transaction
            || pending.public_parent != self.public_parent
            || pending.captured_head != self.install_parent
            || pending.captured_archive != parent.archive
            || pending.accepted_capture
                != index_capture_data(
                    self.context,
                    TextIndexCaptureRoleData::Directory,
                    parent.directory,
                )?
            || candidate.directory != parent.directory
            || candidate.publications != parent.publications
            || candidate.queue_front != parent.queue_front
            || candidate.queue_next != parent.queue_next
            || candidate.archive == parent.archive
        {
            return Err(refused());
        }
        let mut rows = [None; MAX_ARTIFACTS];
        let mut count = 0;
        for entry in self.artifacts.entries_data() {
            rows[count] = Some(entry?);
            count += 1;
        }
        let entries = &rows[..count];
        let retain_frame = find(
            entries,
            TextArtifactRoleData::Retain,
            *pending.retain.as_bytes(),
        )?;
        let retain_plain = open_control_data(
            key,
            TextControlKindData::Retain.id_data(retain_frame.frame),
            retain_frame.frame,
        )?;
        let retained = RetainedActionData::decode_data(&retain_plain)?;
        if retained.context != self.context
            || retained.transaction != self.transaction
            || retained.token != pending.token
            || retained.request != pending.request
            || retained.transaction
                != TextTransactionIdData::from_request_data(
                    self.context,
                    retained.token,
                    retained.request,
                )
        {
            return Err(refused());
        }
        let (actions, ciphers) = retained.descriptors_data();
        if role_count(entries, TextArtifactRoleData::ActionChunk) != actions.len()
            || role_count(entries, TextArtifactRoleData::CipherChunk) != ciphers.len()
            || role_count(entries, TextArtifactRoleData::TokenIndex) != 1
        {
            return Err(refused());
        }
        let mut action_plains = Vec::with_capacity(actions.len());
        for descriptor in actions {
            let entry = find(
                entries,
                TextArtifactRoleData::ActionChunk,
                *descriptor.page.as_bytes(),
            )?;
            let plain = open_private_chat_page(key, descriptor.page, entry.frame)?;
            if plain.len() != descriptor.plain_bytes as usize {
                return Err(refused());
            }
            action_plains.push(plain);
        }
        let chunks: Vec<_> = action_plains
            .iter()
            .map(|plain| ActionChunk::decode(plain).map_err(|_| refused()))
            .collect::<Result<_, _>>()?;
        let original = assemble_action(
            &ActionPagesExpectation {
                vault: &c.layout.vault,
                group: c.group,
                profile: c.profile,
                action: retained.action,
                original: retained.original,
            },
            &chunks,
        )
        .map_err(|_| refused())?;
        let action = original.action();
        let Body::Create {
            text,
            reply_to: None,
        } = &action.spec().body
        else {
            return Err(refused());
        };
        if original.original_bytes().len() != retained.action_bytes as usize
            || action.signer() != &c.signer
            || action.spec().realm.channel() != retained.channel
            || !action.spec().parents.is_empty()
            || TextRequestDigestData::from_request_data(
                self.context,
                retained.token,
                retained.channel,
                text,
            )? != retained.request
        {
            return Err(refused());
        }
        // Group cipher plaintext is not re-encrypted/decrypted here. The real original SDK
        // packet establishes its action/cipher association; verify exact stored bytes/digest.
        let mut cipher = Zeroizing::new(Vec::with_capacity(retained.cipher_bytes as usize));
        for (index, descriptor) in ciphers.iter().enumerate() {
            let entry = find(
                entries,
                TextArtifactRoleData::CipherChunk,
                *descriptor.page.as_bytes(),
            )?;
            let plain = open_private_chat_page(key, descriptor.page, entry.frame)?;
            if plain.len() != descriptor.plain_bytes as usize {
                return Err(refused());
            }
            let chunk = CipherChunkData::decode_data(&plain)?;
            chunk.require_descriptor_data(&retained, index)?;
            cipher.extend_from_slice(chunk.payload_data());
        }
        if cipher.len() != retained.cipher_bytes as usize || cipher.len() < 41 {
            return Err(refused());
        }
        let nonce: &[u8; 24] = cipher[..24].try_into().map_err(|_| refused())?;
        if cipher_digest(nonce, &cipher[24..])? != retained.cipher {
            return Err(refused());
        }

        let archive_frame = find(
            entries,
            TextArtifactRoleData::Archive,
            *candidate.archive.as_bytes(),
        )?;
        let archive_plain = open_control_data(
            key,
            TextControlIdData::from_data(TextControlKindData::Archive, archive_frame.id),
            archive_frame.frame,
        )?;
        let archive = TextArchiveData::decode_data(&archive_plain)?;
        archive.require_successor_data(selected.archive, parent.archive)?;
        let a = archive.spec_data();
        if a.context != self.context
            || a.operation != TextArchiveOperationData::RetainAction
            || a.header.id() != self.public_parent
        {
            return Err(refused());
        }
        let manifest_frame = find(
            entries,
            TextArtifactRoleData::Manifest,
            *a.manifest.as_bytes(),
        )?;
        let manifest_plain = open_private_chat_page(key, a.manifest, manifest_frame.frame)?;
        let manifest = ManifestPage::decode(&manifest_plain).map_err(|_| refused())?;
        let before = selected.archive.spec_data();
        if manifest.vault_domain() != &c.layout.vault
            || manifest.group() != c.group
            || manifest.profile() != c.profile
            || manifest.sequence() != a.manifest_sequence
            || manifest.previous_manifest() != Some(before.manifest)
            || manifest.original_source_head() != Some(parent.archive)
            || manifest.before_header() != Some(self.public_parent)
            || manifest.after_header() != self.public_parent
            || manifest.operation()
                != (ArchiveOperation::RetainAction {
                    action: retained.action,
                    original: retained.original,
                    total_bytes: retained.action_bytes,
                })
            || manifest.page_count() != actions.len()
        {
            return Err(refused());
        }
        for (index, descriptor) in actions.iter().enumerate() {
            let row = manifest.page(index).map_err(|_| refused())?;
            if row.role != PageRole::OriginalActionChunk
                || row.page != descriptor.page
                || row.plaintext_bytes != descriptor.plain_bytes as usize
                || row.entries != 1
            {
                return Err(refused());
            }
        }
        let TextRootData::Present(token_root) = candidate.tokens else {
            return Err(refused());
        };
        let token_frame = find(
            entries,
            TextArtifactRoleData::TokenIndex,
            *token_root.page.as_bytes(),
        )?;
        let token_plain = open_private_chat_page(key, token_root.page, token_frame.frame)?;
        let token_node = PrivateIndexNodeFrame::decode(
            &token_plain,
            PrivateIndexContextData {
                cid: self.context,
                kind: PrivateIndexKindData::Token,
            },
            NodePlacementData::Root,
        )
        .map_err(|_| refused())?;
        let expected_token = TokenValueData {
            transaction: self.transaction,
            token: retained.token,
            request: retained.request,
            retain: pending.retain,
            action: retained.action,
            original: retained.original,
            phase: RequestPhaseData::Retained,
        };
        if token_node.summary() != token_root.summary
            || token_node.kind() != NodeKindData::Leaf
            || token_node.row_count() != 1
            || token_node.leaf_row(0).map_err(|_| refused())?.value
                != PrivateIndexValueData::Token(expected_token)
        {
            return Err(refused());
        }
        let bytes = self.artifacts.validate_data(self.operation)? as u128;
        if candidate.live_bytes != parent.live_bytes.checked_add(bytes).ok_or_else(refused)?
            || candidate.live_objects
                != parent
                    .live_objects
                    .checked_add(count as u128)
                    .ok_or_else(refused)?
        {
            return Err(refused());
        }
        Ok(())
    }
}

fn find<'a>(
    entries: &[Option<TextWalArtifactData<'a>>],
    role: TextArtifactRoleData,
    id: [u8; 32],
) -> Result<TextWalArtifactData<'a>, AppError> {
    entries
        .iter()
        .flatten()
        .copied()
        .find(|entry| entry.role == role && entry.id == id)
        .ok_or_else(refused)
}
fn role_count(entries: &[Option<TextWalArtifactData<'_>>], role: TextArtifactRoleData) -> usize {
    entries
        .iter()
        .flatten()
        .filter(|entry| entry.role == role)
        .count()
}
