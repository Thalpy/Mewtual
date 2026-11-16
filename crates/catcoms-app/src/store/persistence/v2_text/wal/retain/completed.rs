//! Completed-WC retained-A verifier. All original signed action/cipher/Retain/
//! public-manifest authentication remains. Only the authenticated WC baseline and
//! genuine nonempty token COW/carry differ; no source permission follows.
use super::*;
impl TextWorkWalFrameData<'_> {
    pub(in crate::store::persistence::v2_text) fn verify_completed_retained_a_parent_data(
        &self,
        key: &[u8; 32],
        selected: RetainWalParentData<'_>,
        completed:&crate::store::persistence::v2_text::completed_workspace::CompletedV2TextParentData,
        old_token: &crate::store::persistence::v2_text::private_index::PrivateIndexPathData,
    ) -> Result<(), AppError> {
        self.validate_data()?;
        if self.operation != TextWorkWalOperationData::Retain {
            return Err(refused());
        }
        let TextWorkWalCleanupData::Carry(raw_carry) = &self.cleanup else {
            return Err(refused());
        };
        let carry = RetirementCarryData::decode_data(raw_carry)?;
        if self.parent_frame != completed.original_head_frame_data()
            || selected.frame != completed.original_head_frame_data()
            || self.install_parent != completed.original_head_id_data()
        {
            return Err(refused());
        }
        let TextRootData::Present(old_root) = selected.head.tokens else {
            return Err(refused());
        };
        if old_root.summary.height != 0
            || old_root.summary.keys != 1
            || old_token.row_data().map_err(|_| refused())?.is_some()
            || old_token.next_page_data().map_err(|_| refused())?.is_some()
        {
            return Err(refused());
        }
        let observed: Vec<_> = old_token.original_frame_sizes_data().collect();
        if observed.len() != 1 || observed[0].0 != old_root {
            return Err(refused());
        }
        let expected = [
            crate::store::persistence::v2_text::retirement::RetirementEntryData {
                kind:
                    crate::store::persistence::v2_text::retirement::RetirementKindData::TokenIndex,
                id: *old_root.page.as_bytes(),
                frame_bytes: u32::try_from(observed[0].1).map_err(|_| refused())?,
            },
        ];
        if carry.roots_data() != [self.install_parent] || carry.entries_data() != expected {
            return Err(refused());
        }
        let parent = selected.head;
        let context = &parent.context;
        let c = context.spec_data();
        if self.context != context.id_data()?
            || self.install_parent != selected.id
            || self.parent_frame != selected.frame
            || parent.pending != TextPendingData::None
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
        let rewrite = old_token
            .rewrite_leaf_data(
                &crate::store::persistence::v2_text::private_index::PrivateIndexLeafRowData {
                    key: *expected_token.key_data(self.context).as_bytes(),
                    value: PrivateIndexValueData::Token(expected_token),
                },
            )
            .map_err(|_| refused())?;
        if rewrite.nodes_data().len() != 1
            || token_node.kind() != NodeKindData::Leaf
            || token_node.row_count() != 2
            || token_node.summary() != token_root.summary
            || rewrite.nodes_data()[0].summary_data() != token_root.summary
            || rewrite.nodes_data()[0].plain_data() != token_plain.as_slice()
        {
            return Err(refused());
        }
        let bytes = self.artifacts.validate_data(self.operation)? as u128;
        let baseline = completed.effective_baseline_data()?;
        if candidate.live_bytes != baseline.bytes.checked_add(bytes).ok_or_else(refused)?
            || candidate.live_objects
                != baseline
                    .objects
                    .checked_add(count as u128)
                    .ok_or_else(refused)?
        {
            return Err(refused());
        }
        Ok(())
    }
}
