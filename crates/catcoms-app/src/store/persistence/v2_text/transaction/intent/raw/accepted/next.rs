//! DISTINCT second founder acceptance from a completed-parent owning A/I/S chain.
//! The profile permits only the already authenticated root leaves; it preserves old
//! rows and uses actual observed encrypted lengths. Initial first-B stays separate.
use super::*;
mod producer;
mod semantic;

// ADDITIONAL to the full retained A/I/S/B invoice: local authenticated token-A
// path, packed leaf rewrite rows, simultaneous output/plain/seal overlaps and
// observed-entry/move scratch. Original query-owned paths keep their own full fee.
pub(super) const COMPLETED_ROOT_LEAF_WORK_BYTES: usize = 2
    * crate::store::persistence::v2_text::private_index::PRIVATE_INDEX_PATH_METADATA_BYTES
    + 2 * crate::store::persistence::v2_text::index::ACCEPTED_INDEX_PATH_METADATA_BYTES
    + crate::store::persistence::v2_text::private_index::PRIVATE_INDEX_REWRITE_METADATA_BYTES
    + crate::store::persistence::v2_text::private_index::PRIVATE_INDEX_REWRITE_TEMPORARY_BYTES
    + 16 * 16470
    + 32 * std::mem::size_of::<RetirementEntryData>();

pub(super) struct CompletedAcceptedOrigin {
    pub(super) actions: RetirementEntryData,
    pub(super) publications: RetirementEntryData,
}

impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn take_completed_into(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        input: &mut Option<PreparedV2TextRawSignature>,
        output: &mut Option<Box<Self>>,
    ) -> Result<(), AppError> {
        if let Some(existing) = output.as_ref() {
            if existing.completed.is_none() || input.is_some() {
                return Err(refused());
            }
            existing.metadata.validate_prepaid_account()?;
            existing
                .resources()
                .require_bound_original_negative(store, source)?;
            if existing.saved.is_some() {
                return existing
                    .signature
                    .intent
                    .unsigned
                    .validate_accepted_advance_data(
                        &existing.signature.intent.retained,
                        &existing.observe_original(),
                    );
            }
            return existing.signature.intent.unsigned.validate_original(
                store,
                source,
                &existing.signature.intent.retained,
            );
        }
        let signature = input.as_ref().ok_or_else(refused)?;
        if !signature.intent.retained.is_completed_parent_data() {
            return Err(refused());
        }
        let metadata = AcceptedMetadataResources::prepare(store, source, signature)?;
        let saved = signature.saved.as_ref().ok_or_else(refused)?;
        saved
            .original
            .require_same_original(&signature.observe_original())?;
        if saved.selected.as_slice()
            != signature
                .candidate
                .as_ref()
                .ok_or_else(refused)?
                .frame_data()
            || saved.signature_frame.as_slice()
                != signature
                    .signature
                    .as_ref()
                    .ok_or_else(refused)?
                    .frame_data()
        {
            return Err(refused());
        }
        let candidate = signature
            .intent
            .unsigned
            .borrow_verified_candidate_data(&signature.intent.retained)?;
        let original = signature
            .intent
            .unsigned
            .unsigned_data(&signature.intent.retained)?;
        if candidate.header_data().signature()
            != signature.intent.unsigned.existing_raw_signature_data()?
            || candidate.header_data().spec().parent != signature.record.public_parent
            || candidate.header_data().spec().body != original.body.id()
            || candidate.image_data() != original.image
            || !candidate.header_data().verify()
        {
            return Err(refused());
        }
        let key = Zeroizing::new(store.keys.db_key()?);
        let plain = open_control_data(
            &key,
            TextControlKindData::Head.id_data(&saved.selected),
            &saved.selected,
        )?;
        let head = TextHeadData::decode_data(&plain)?;
        if !matches!(head.pending, TextPendingData::Raw { .. })
            || head.context != signature.parent.head.context
            || !matches!(head.publications, TextRootData::Present(root) if root.summary.height == 0 && root.summary.keys == 1)
            || head.queue_front > head.queue_next
            || head.queue_next != 1
        {
            return Err(refused());
        }
        // Only the owning completed-parent A and its genuine completed query can
        // supply these paths. Historical SavedS bytes are constructor DATA; actual
        // Stage re-proves the selected S closure after all waits and copies.
        let path = signature
            .intent
            .query
            .borrow_completed_directory_path_data(&signature.intent.retained)?;
        let row = path.row_data()?.ok_or_else(refused)?;
        let ValueData::Directory(directory) = row.value else {
            return Err(refused());
        };
        let c = head.context.spec_data();
        if row.key != *c.admission.as_bytes()
            || !matches!(directory.tree, DirectoryTreeData::Present(root) if root.summary.height == 0 && root.summary.keys == 1)
            || directory.accepted_leaf_count != 1
            || directory.unique_action_count != 1
            || directory.own_tip_header != signature.record.public_parent
            || candidate.append_data().old_tip().count != directory.accepted_leaf_count
            || candidate.append_data().new_tip().count
                != directory
                    .accepted_leaf_count
                    .checked_add(1)
                    .ok_or_else(refused)?
        {
            return Err(refused());
        }
        let old: Vec<_> = path.original_frame_sizes_data().collect();
        let TextRootData::Present(directory_root) = head.directory else {
            return Err(refused());
        };
        if old.len() != 1
            || old[0].0.page != directory_root.page
            || old[0].0.summary != directory_root.summary
            || old[0].1 < 87
            || old[0].1 > 16470
        {
            return Err(refused());
        }
        let token = signature
            .intent
            .retained
            .token
            .as_ref()
            .ok_or_else(refused)?;
        let TextRootData::Present(token_root) = head.tokens else {
            return Err(refused());
        };
        if token_root.page != token.id()
            || token_root.summary.height != 0
            || token_root.summary.keys != 2
        {
            return Err(refused());
        }
        let DirectoryTreeData::Present(actions_root) = directory.tree else {
            return Err(refused());
        };
        let actions_path = signature
            .intent
            .query
            .borrow_completed_actions_path_data(&signature.intent.retained)?;
        let actions_observed: Vec<_> = actions_path.original_frame_sizes_data().collect();
        let publication_path = signature
            .intent
            .query
            .borrow_completed_publication_path_data(&signature.intent.retained)?;
        let publication_observed: Vec<_> = publication_path.original_frame_sizes_data().collect();
        let TextRootData::Present(publication_root) = head.publications else {
            return Err(refused());
        };
        if directory_root.summary.height != 0
            || directory_root.summary.keys != 1
            || actions_observed.len() != 1
            || actions_observed[0].0 != actions_root
            || publication_observed.len() != 1
            || publication_observed[0].0 != publication_root
        {
            return Err(refused());
        }
        let completed = CompletedAcceptedOrigin {
            actions: RetirementEntryData {
                kind: RetirementKindData::AcceptedIndex,
                id: *actions_root.page.as_bytes(),
                frame_bytes: u32::try_from(actions_observed[0].1).map_err(|_| refused())?,
            },
            publications: RetirementEntryData {
                kind: RetirementKindData::PublicationIndex,
                id: *publication_root.page.as_bytes(),
                frame_bytes: u32::try_from(publication_observed[0].1).map_err(|_| refused())?,
            },
        };
        let parent = Arc::new(AcceptedParent {
            head,
            head_frame: Zeroizing::new(saved.selected.to_vec()),
            directory,
            directory_root,
            directory_frame_bytes: old[0].1,
            token_root,
            token_frame: Zeroizing::new(token.as_bytes().to_vec()),
            header: candidate.header_data().clone(),
            image: Zeroizing::new(candidate.image_data().to_vec()),
            signature: Arc::clone(saved),
            metadata: metadata.clone(),
        });
        signature
            .intent
            .unsigned
            .validate_original(store, source, &signature.intent.retained)?;
        let allocation = Box::<Self>::new_uninit();
        *output = Some(Box::write(
            allocation,
            Self {
                identity: Arc::new(()),
                completed: Some(completed),
                parent,
                records: None,
                nodes: None,
                header_body: None,
                manifest: None,
                archive: None,
                actions: None,
                directory: None,
                token: None,
                publications: None,
                publication: None,
                candidate: None,
                plan: None,
                completion: None,
                deletion: None,
                wal: None,
                slots: work::AcceptedSlots::empty(),
                ready: None,
                saved: None,
                retirement_work: retirement::RetirementWorkState::default(),
                #[cfg(test)]
                hooks: work::AcceptedTestHooks::default(),
                #[cfg(all(test, windows))]
                saved_drop_notice: None,
                signature: input
                    .take()
                    .expect("same checked whole S/verifier/candidate/SDK"),
                metadata,
            },
        ));
        Ok(())
    }
}

#[cfg(all(test, windows))]
impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn next_model_state_for_test(
        &self,
        original: &crate::store::persistence::OriginalV2TextResidentObservation,
    ) -> Result<([u8; 32], u64), AppError> {
        if self.completed.is_none() {
            return Err(refused());
        }
        self.signature
            .intent
            .unsigned
            .next_model_state_for_test(original)
    }
}
#[cfg(all(test, windows))]
impl crate::store::persistence::v2_text::query::PreparedV2TextTokenQuery {
    /// Authenticated structural DATA only. This does not issue a canonical hit or
    /// acceptance receipt, revalidate Current or read a Header from caller bytes.
    pub(in crate::store) fn accepted_header_data_for_test(
        &self,
    ) -> Result<Option<catcoms_replication::chat::owner_header::OwnerHeaderId>, AppError> {
        Ok(match self.token_value_data()?.map(|row| row.phase) {
            Some(RequestPhaseData::Accepted(locator)) => Some(locator.header),
            _ => None,
        })
    }
}
