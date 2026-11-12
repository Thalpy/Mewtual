//! DISTINCT retained-A from a genuine completed WC predecessor. Initial Clear
//! stays initial-only. The raw prior O remains the CAS parent while effective
//! quota is raw prior baseline MINUS its exact authenticated WC total, ONCE.
use super::*;
mod resident_source;
use crate::store::critical_namespaces::current_layout::BoundV2TextCompletedCreatePredecessor;
use crate::store::persistence::v2_text::completed_workspace::{
    CompletedV2TextParentData, OriginalV2TextCompletedWorkspaceObservation,
    ReadyV2TextCompletedWorkspace,
};
use crate::store::persistence::v2_text::retirement::{RetirementEntryData, RetirementKindData};
use crate::store::persistence::OriginalV2TextResidentObservation;
pub(in crate::store) use resident_source::NextRetainedAResidentSourceData;

pub(super) struct CompletedRetainOrigin {
    original: OriginalV2TextCompletedWorkspaceObservation,
    token_summary: Option<NodeSummaryData>,
    replaced_tokens: Vec<RetirementEntryData>,
}
impl PreparedV2TextRetainTransaction {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn take_from_completed_into(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        predecessor: &BoundV2TextCompletedCreatePredecessor,
        ready: &ReadyV2TextCompletedWorkspace,
        policy: V2TextQuotaPolicy,
        custody: &mut Option<PreparedV2RetainOriginalCustody>,
        query: &mut Option<PreparedV2TextTokenQuery>,
        retained: &mut Option<Self>,
    ) -> Result<(), AppError> {
        if let Some(existing) = retained.as_ref() {
            if custody.is_some() || query.is_some() {
                return Err(refused());
            }
            let completed = existing.completed.as_ref().ok_or_else(refused)?;
            predecessor.require_same_original(&existing.original_retain())?;
            predecessor.require_completed_original(&completed.original)?;
            return existing.custody.validate_accounts(store);
        }
        let original = custody.as_ref().ok_or_else(refused)?;
        let original_query = query.as_ref().ok_or_else(refused)?;
        original.validate_accounts(store)?;
        let producer = original.producer();
        producer.validate_resources_original(store, source)?;
        producer.require_selected_data(selected)?;
        predecessor.require_same_original(&producer.observe_original())?;
        let completed_parent = ready.completed_parent_data()?;
        let completed_original = ready.observe_completed_original();
        predecessor.require_completed_original(&completed_original)?;
        predecessor.require_same_resident(completed_parent.original_resident())?;
        completed_original.require_original_retained(&producer.observe_original())?;
        completed_parent.require_selected_data(selected)?;
        original_query.require_token_parent_data(selected, producer.token_data())?;
        if original_query.token_value_data()?.is_some() {
            return Err(refused());
        }
        let h = selected.head_data();
        // Initial completed profile: the actual previous accepted token and queue
        // roots are nonempty LEAVES. No split, branch adoption or empty substitute.
        let TextRootData::Present(root) = h.tokens else {
            return Err(refused());
        };
        let TextRootData::Present(publications) = h.publications else {
            return Err(refused());
        };
        if h.pending != TextPendingData::None
            || root.summary.height != 0
            || root.summary.keys != 1
            || publications.summary.height != 0
            || publications.summary.keys != 1
        {
            return Err(refused());
        }
        let path = original_query.path_data()?.ok_or_else(refused)?;
        let old: Vec<_> = path.original_frame_sizes_data().collect();
        if old.len() != 1
            || old[0].0 != root
            || path.next_page_data().map_err(|_| refused())?.is_some()
        {
            return Err(refused());
        }
        let replaced_tokens = old
            .iter()
            .map(|(node, bytes)| {
                Ok(RetirementEntryData {
                    kind: RetirementKindData::TokenIndex,
                    id: *node.page.as_bytes(),
                    frame_bytes: u32::try_from(*bytes).map_err(|_| refused())?,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let effective = completed_parent.effective_baseline_data()?;
        policy.require_initial_capacity(effective.bytes, effective.objects)?;
        let physical = selected.physical_query_data();
        let parent = Arc::new(RetainParent {
            target: owned_path(physical.target)?,
            target_id: physical.target_id,
            pages_id: physical.pages_id,
            head_path: owned_path(physical.head_path)?,
            archive_path: owned_path(physical.archive_path)?,
            wal_path: fixed_child(physical.target, "text.wal")?,
            wal_stage_path: fixed_child(physical.target, "text.wal.stage.v1")?,
            head_frame: Zeroizing::new(physical.head_frame.to_vec()),
            archive_frame: Zeroizing::new(physical.archive_frame.to_vec()),
            head: TextHeadData::decode_data(&h.encode_data()?)?,
            archive: TextArchiveData::decode_data(&selected.archive_data().encode_data()?)?,
            completed: Some(Arc::clone(completed_parent)),
            resources: producer.resources().clone(),
            metadata: producer.metadata_resources().clone(),
        });
        let carry = RetirementCarryData::from_data(&[selected.head_id_data()], &replaced_tokens)?
            .encode_data()?;
        producer
            .resources()
            .require_bound_original_negative(store, source)?;
        // All THREE unique originals remain unchanged on every owning refusal. The
        // actual native Bound remains Root-owned and must match every later role.
        *retained = Some(Self {
            identity: Arc::new(()),
            parent,
            policy,
            token: None,
            candidate: None,
            wal: None,
            carry,
            completed: Some(CompletedRetainOrigin {
                original: completed_original,
                token_summary: None,
                replaced_tokens,
            }),
            slots: work::RetainSlots::empty(),
            ready: None,
            saved: None,
            #[cfg(test)]
            hooks: work::RetainTestHooks::default(),
            #[cfg(all(test, windows))]
            saved_drop_notice: None,
            recovery: Arc::new(recovery::RetainRecoveryControl::new()),
            query: query.take().expect("same original completed token reader"),
            custody: custody.take().expect("same whole paid Create SDK custody"),
        });
        Ok(())
    }
    pub(in crate::store) fn is_completed_parent_data(&self) -> bool {
        self.completed.is_some()
    }
    /// Exact old token retirement list only, never an arbitrary parsed carry list.
    pub(in crate::store) fn require_completed_carry_data(
        &self,
        carry: &RetirementCarryData,
    ) -> Result<(), AppError> {
        let completed = self.completed.as_ref().ok_or_else(refused)?;
        if carry.roots_data() != [TextHeadIdData::of_data(&self.parent.head_frame)]
            || carry.entries_data() != completed.replaced_tokens
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn validate_completed_stage_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
        predecessor: &BoundV2TextCompletedCreatePredecessor,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        guard.require_text_control_write_idle_for_query()?;
        self.custody.validate_accounts(guard.store())?;
        let original = self.completed.as_ref().ok_or_else(refused)?;
        predecessor.require_same_original(&self.original_retain())?;
        predecessor.require_completed_original(&original.original)?;
        original.original.original_resident().validate_negative()?;
        self.producer_data()
            .validate_resources_original(guard.store(), guard.source())?;
        let completed = self.parent.completed.as_ref().ok_or_else(refused)?;
        // Actual own WA continuation precedes the now-obsolete prior WC. The
        // Source proof below still binds the same raw O/H or own lost selected A.
        if self.completed_wal_is_installed_data()? {
            self.parent.validate_parent(
                guard,
                true,
                self.candidate.as_ref().map(|c| c.frame_data()),
            )?;
        } else {
            // All query account/physical IO precedes the final WC+owned-stage
            // proof. Afterwards only its same registered reader negative may run.
            self.query.validate_completed_under_current_guard(guard)?;
            let stage = CompletedRetainStageSourceData { owner: self };
            completed.validate_owned_retain_stage_under_current_guard(guard, &stage)?;
            self.query
                .require_completed_reader_under_current_guard(guard)?;
        }
        original.original.original_resident().validate_negative()?;
        guard.validate_source_only()
    }
}
impl ReadyV2TextRetainSelection {
    pub(in crate::store) fn validate_completed_ready_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
        predecessor: &BoundV2TextCompletedCreatePredecessor,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        let completed = self.facts.parent.completed.as_ref().ok_or_else(refused)?;
        predecessor.require_same_original(&self.original_retain())?;
        predecessor.require_completed_original(&completed.observe_completed_original())?;
        completed.original_resident().validate_negative()?;
        self.validate_ready_under_current_guard(guard)?;
        completed.original_resident().validate_negative()?;
        guard.validate_source_only()
    }
}

impl PreparedV2TextRetainTransaction {
    pub(in crate::store) fn prepare_completed_candidates(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.custody.validate_accounts(store)?;
        self.producer_data()
            .validate_resources_original(store, source)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let (retain_frame, retain_id) = self
            .producer_data()
            .retain_frame_data()?
            .ok_or_else(refused)?;
        let retain_plain = open_control_data(
            &key,
            TextControlKindData::Retain.id_data(retain_frame),
            retain_frame,
        )?;
        let retained = RetainedActionData::decode_data(&retain_plain)?;
        let value = TokenValueData {
            transaction: retained.transaction,
            token: retained.token,
            request: retained.request,
            retain: retain_id,
            action: retained.action,
            original: retained.original,
            phase: RequestPhaseData::Retained,
        };
        let ctx = self.parent.head.context.id_data()?;
        if self.token.is_none() {
            let path = self.query.path_data()?.ok_or_else(refused)?;
            if path.row_data().map_err(|_| refused())?.is_some() {
                return Err(refused());
            }
            let replacement = PrivateIndexLeafRowData {
                key: *value.key_data(ctx).as_bytes(),
                value: PrivateIndexValueData::Token(value),
            };
            let rewrite = path
                .rewrite_leaf_data(&replacement)
                .map_err(|_| refused())?;
            if rewrite.nodes_data().len() != 1 {
                return Err(refused());
            }
            let node = &rewrite.nodes_data()[0];
            if node.summary_data().height != 0 || node.summary_data().keys != 2 {
                return Err(refused());
            }
            self.resources()
                .require_bound_original_negative(store, source)?;
            let sealed = PreparedPrivateChatPage::prepare(&key, node.plain_data(), rng)?;
            // Both ciphertext and descriptor cache before late Source cancellation.
            self.token = Some(sealed);
            self.completed.as_mut().ok_or_else(refused)?.token_summary = Some(node.summary_data());
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        if self.candidate.is_none() {
            let token = self.token.as_ref().ok_or_else(refused)?;
            let archive = self
                .producer_data()
                .artifact_frames_data()
                .find(|(role, _, _)| *role == TextArtifactRoleData::Archive)
                .ok_or_else(refused)?
                .1;
            let (bytes, objects) = self.artifact_totals()?;
            let parent = &self.parent.head;
            let baseline = self
                .parent
                .completed
                .as_ref()
                .ok_or_else(refused)?
                .effective_baseline_data()?;
            let candidate = TextHeadData {
                context: parent.context.clone(),
                generation: parent.generation.checked_add(1).ok_or_else(refused)?,
                previous: TextParentData::Operational(TextHeadIdData::of_data(
                    &self.parent.head_frame,
                )),
                archive: ArchiveHeadId::from_bytes(archive),
                directory: parent.directory,
                tokens: TextRootData::Present(NodeRefData {
                    page: token.id(),
                    summary: self
                        .completed
                        .as_ref()
                        .ok_or_else(refused)?
                        .token_summary
                        .ok_or_else(refused)?,
                }),
                publications: parent.publications,
                queue_front: parent.queue_front,
                queue_next: parent.queue_next,
                pending: TextPendingData::Retained(PendingTransactionData {
                    transaction: retained.transaction,
                    token: retained.token,
                    request: retained.request,
                    public_parent: self.parent.archive.spec_data().header.id(),
                    retain: retain_id,
                    captured_head: TextHeadIdData::of_data(&self.parent.head_frame),
                    captured_archive: parent.archive,
                    accepted_capture: index_capture_data(
                        ctx,
                        TextIndexCaptureRoleData::Directory,
                        parent.directory,
                    )?,
                }),
                live_bytes: baseline.bytes.checked_add(bytes).ok_or_else(refused)?,
                live_objects: baseline.objects.checked_add(objects).ok_or_else(refused)?,
                selection_attempt: parent
                    .selection_attempt
                    .checked_add(1)
                    .ok_or_else(refused)?,
            };
            self.resources()
                .require_bound_original_negative(store, source)?;
            self.candidate = Some(candidate.prepare_frame_data(&key, rng)?);
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        if self.wal.is_none() {
            let candidate = self.candidate.as_ref().ok_or_else(refused)?;
            let artifacts = self.artifacts()?;
            let wal = TextWorkWalFrameData {
                context: ctx,
                install_parent: TextHeadIdData::of_data(&self.parent.head_frame),
                transaction: retained.transaction,
                public_parent: self.parent.archive.spec_data().header.id(),
                operation: TextWorkWalOperationData::Retain,
                parent_frame: &self.parent.head_frame,
                candidate: TextHeadIdData::from_bytes(*candidate.id_data().as_bytes()),
                candidate_frame: candidate.frame_data(),
                cleanup: TextWorkWalCleanupData::Carry(&self.carry),
                artifacts: TextWalArtifactsData::Borrowed(&artifacts),
            };
            wal.verify_completed_retained_a_parent_data(
                &key,
                crate::store::persistence::v2_text::wal::retain::RetainWalParentData {
                    head: &self.parent.head,
                    archive: &self.parent.archive,
                    id: TextHeadIdData::of_data(&self.parent.head_frame),
                    frame: &self.parent.head_frame,
                },
                self.parent.completed.as_ref().ok_or_else(refused)?,
                self.query.path_data()?.ok_or_else(refused)?,
            )?;
            let plain = TextWalFrameData::Work(wal).encode_data()?;
            self.resources()
                .require_bound_original_negative(store, source)?;
            let complete = PreparedTextControlFrameData::prepare_data(
                TextControlKindData::Wal,
                &key,
                &plain,
                rng,
            )?;
            self.wal = Some(complete);
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        Ok(())
    }
}

impl PreparedV2TextRetainTransaction {
    fn completed_wal_is_installed_data(&self) -> Result<bool, AppError> {
        let Some(id) = self.slots.wal_native_data() else {
            return Ok(false);
        };
        match catcoms_storage::kept::session_at(&self.parent.wal_path, id) {
            catcoms_storage::kept::SessionAt::Ours => {
                let metadata = std::fs::symlink_metadata(&self.parent.wal_path)
                    .map_err(|e| AppError::Io(e.to_string()))?;
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.len()
                        != self.wal.as_ref().ok_or_else(refused)?.frame_data().len() as u64
                {
                    return Err(refused());
                }
                Ok(true)
            }
            catcoms_storage::kept::SessionAt::Unknown => Err(refused()),
            // Other is expected while our own WA is still at stage and the exact
            // prior WC remains MAIN. The separate full WC proof must still pass;
            // it is NEVER inferred from this negative comparison or an IO error.
            catcoms_storage::kept::SessionAt::Other => Ok(false),
            catcoms_storage::kept::SessionAt::Absent => Err(refused()),
        }
    }
}

impl PreparedV2TextRetainTransaction {
    pub(in crate::store) fn completed_retirement_entries_data(
        &self,
    ) -> Result<&[RetirementEntryData], AppError> {
        Ok(&self.completed.as_ref().ok_or_else(refused)?.replaced_tokens)
    }
}

/// Receiver-bound sealed data for ONE actual nextA stage. No caller-selected ID,
/// path, byte slice, empty flag or work permission can construct this type.
pub(in crate::store::persistence::v2_text) struct CompletedRetainStageSourceData<'a> {
    owner: &'a PreparedV2TextRetainTransaction,
}
impl CompletedRetainStageSourceData<'_> {
    pub(in crate::store::persistence::v2_text) fn require_parent(
        &self,
        parent: &CompletedV2TextParentData,
    ) -> Result<(), AppError> {
        let own = self.owner.parent.completed.as_ref().ok_or_else(refused)?;
        if !std::ptr::eq(own.as_ref(), parent) {
            return Err(refused());
        }
        self.owner
            .completed
            .as_ref()
            .ok_or_else(refused)?
            .original
            .require_same_original(&parent.observe_completed_original())
    }
    pub(in crate::store::persistence::v2_text) fn recorded_stage_data(
        &self,
    ) -> Result<(Option<SessionId>, usize), AppError> {
        Ok((
            self.owner.slots.wal_native_data(),
            self.owner
                .wal
                .as_ref()
                .ok_or_else(refused)?
                .frame_data()
                .len(),
        ))
    }
}

#[cfg(all(test, windows))]
impl PreparedV2TextRetainTransaction {
    pub(in crate::store) fn original_completed_wal_output_for_test(
        &self,
    ) -> Result<(PathBuf, Zeroizing<Vec<u8>>), AppError> {
        self.completed.as_ref().ok_or_else(refused)?;
        Ok((
            owned_path(&self.parent.wal_stage_path)?,
            Zeroizing::new(self.wal.as_ref().ok_or_else(refused)?.frame_data().to_vec()),
        ))
    }
}
