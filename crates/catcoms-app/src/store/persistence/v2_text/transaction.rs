//! Owning first retained-A transaction. SDK originals and their actual independent fee
//! move together; an empty token label, predecessor or prepared WAL is never a save.
//! The paired stage/commit workers are the only constructors of Ready/Saved outcomes.
use super::archive::TextArchiveData;
use super::artifacts::TextArtifactRoleData;
use super::capture::{original_context, SelectedV2TextHead};
use super::common::*;
use super::control::{open_control_data, PreparedTextControlFrameData, TextControlKindData};
use super::head::{
    index_capture_data, PendingTransactionData, TextHeadData, TextIndexCaptureRoleData,
    TextParentData, TextPendingData, TextRootData,
};
use super::io::{read_control, require_named};
use super::ordinary_data::{
    OriginalV2RetainObservation, PreparedV2RetainData, PreparedV2RetainOriginalCustody,
    V2TextTransactionMetadataResources, RETAIN_DATA_LAYOUT_BYTES, TEXT_TRANSACTION_METADATA_FEE,
    TRANSACTION_METADATA_BASE_LAYOUT_BYTES,
};
use super::private_index::{
    encode_private_leaf_data, PrivateIndexContextData, PrivateIndexKindData,
    PrivateIndexLeafRowData, PrivateIndexValueData,
};
use super::promotion::V2TextQuotaPolicy;
use super::query::PreparedV2TextTokenQuery;
use super::request::{RequestPhaseData, TokenValueData};
use super::resources::{V2TextReadResources, V2TextWorkResources, TEXT_WORK_BUFFER_FEE};
use super::retain::RetainedActionData;
use super::retirement::RetirementCarryData;
use super::wal::{
    TextWalArtifactData, TextWalArtifactsData, TextWalFrameData, TextWorkWalCleanupData,
    TextWorkWalFrameData, TextWorkWalOperationData, WAL_PLAIN_ALLOCATION_CAP,
};
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextRetainCommit, AdmittedV2TextRetainStage, BoundV2TextCreatePredecessor,
    V2TextCurrentGroupGuard, V2_TEXT_RETAIN_ADMISSION_LAYOUT_BYTES,
};
use crate::store::persistence::GenesisSourceDataRef;
use crate::store::ServerStore;
use crate::AppError;
use catcoms_replication::chat::cold_index::{NodePlacementData, NodeRefData, NodeSummaryData};
use catcoms_replication::chat::cold_manifest::ArchiveHeadId;
use catcoms_rt::CryptoRngCore;
use catcoms_storage::kept::SessionId;
use catcoms_storage::private_chat_page::PreparedPrivateChatPage;
use catcoms_sync::owner_v2::V2CreateRequest;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use zeroize::Zeroizing;

mod accepted_source;
mod completed;
pub(in crate::store::persistence::v2_text) use completed::CompletedRetainStageSourceData;
pub(in crate::store) use completed::NextRetainedAResidentSourceData;
mod intent;
mod ready_read;
mod recovery;
mod work;
pub(in crate::store) use accepted_source::RetainedAQuerySourceData;
#[cfg(all(test, windows))]
pub(in crate::store) use recovery::V2TextRetainRecoveryTestCut;
pub(in crate::store) use recovery::{
    CapturedV2TextRetainRecovery, OriginalV2TextRetainRecoveryObservation,
    PreparedV2TextRetainRecovery, ReadyV2TextRetainRepair, ReadyV2TextRetainScratch,
    V2TextRetainRecoveryCaptureClaim, V2TextRetainStageFence, V2TextRetainWarmRecoveryClassData,
};
#[cfg(all(test, windows))]
pub(in crate::store) use work::V2TextRetainTestCut;

const MAX_PATH: usize = 5120;
const MAX_A_ARTIFACTS: usize = 29;
/// Both pre-transfer slots and the concrete owning A cell are present in Root's phase.
/// This is part of transaction metadata1.25MiB, independently of all packet/work/read/SDK credits.
/// The retained append candidate makes that phase 52,352 bytes on the Windows test
/// target. Charge this entire 64KiB allowance in every persistent A/I/S inventory;
/// the combined transaction bound remains 1MiB, without assuming an earlier refund.
pub(in crate::store) const V2_TEXT_RETAIN_ROOT_PHASE_BYTES: usize = 64 * 1024;

/// SAME16MiB covers original A producer, full encoded/sealed WAL overlap, one active
/// Ready copy and all bounded byte comparisons. SDK1MiB and transaction metadata1.25MiB are ADDITIONAL.
pub(in crate::store) const V2_TEXT_RETAIN_WORK_LAYOUT_BYTES: usize = RETAIN_DATA_LAYOUT_BYTES
    + 5 * (WAL_PLAIN_ALLOCATION_CAP + 40)
    + super::wal::retain::FIRST_RETAIN_WAL_VERIFY_BUFFER_BYTES
    + 128 * std::mem::size_of::<std::fs::File>()
    + 128 * 1024;
const _: () = assert!(V2_TEXT_RETAIN_WORK_LAYOUT_BYTES <= TEXT_WORK_BUFFER_FEE);
/// All non-Current terms remain conservatively persistent, including the complete
/// recovery/verifier/Root inventories. A's owned path data is five parent names plus
/// two Saved HA/TA names; sixteen slots also cover construction/return overlap.
/// Read/query/SDK classes separately own their own actual paths, not spare credit.
pub(super) const RETAIN_PERSISTENT_METADATA_LAYOUT_BYTES: usize =
    TRANSACTION_METADATA_BASE_LAYOUT_BYTES
        + 2 * std::mem::size_of::<PreparedV2TextRetainTransaction>()
        + 4 * std::mem::size_of::<ReadyV2TextRetainSelection>()
        + 4 * std::mem::size_of::<SavedV2RetainedAction>()
        + 2 * std::mem::size_of::<RetainParent>()
        + 4 * std::mem::size_of::<work::ReadyRetainFacts>()
        + V2_TEXT_RETAIN_ADMISSION_LAYOUT_BYTES
        + recovery::V2_TEXT_RETAIN_RECOVERY_METADATA_BYTES
        + 4 * std::mem::size_of::<RetainedAQuerySourceData<'static>>()
        + super::wal::retain::FIRST_RETAIN_WAL_VERIFY_METADATA_BYTES
        + 16 * (MAX_PATH + 256)
        + 128 * (std::mem::size_of::<SessionId>() + 64)
        + V2_TEXT_RETAIN_ROOT_PHASE_BYTES
        // The next owning phase is ADDITIONAL; never assume first Root64KiB fits.
        + crate::store::persistence::PreparedV2NextUnsignedAppend::metadata_layout_bytes()
        + 32 * 1024;
/// A Ready's held proof owns at most34 paths:29 artifacts, WAL, target/pages and
/// head.stage OR its absence+main alternate.48 retains native-name/scratch margin.
/// Proofs are private locals tied to the SAME guarded Runtime; they never escape.
pub(super) const RETAIN_CURRENT_SCRATCH_METADATA_BYTES: usize =
    48 * (MAX_PATH + 256) + ready_read::READY_NAMED_FILES_LAYOUT_BYTES;
/// A's standalone invoice is unchanged:16persistent+48scratch is the original64.
pub(in crate::store) const V2_TEXT_RETAIN_METADATA_LAYOUT_BYTES: usize =
    RETAIN_PERSISTENT_METADATA_LAYOUT_BYTES + RETAIN_CURRENT_SCRATCH_METADATA_BYTES;
const _: () = assert!(V2_TEXT_RETAIN_METADATA_LAYOUT_BYTES <= TEXT_TRANSACTION_METADATA_FEE);
/// Conservative peak helper only; no payment, Source, phase or work authority.
pub(super) const fn max_phase_metadata_data(a: usize, b: usize) -> usize {
    if a > b {
        a
    } else {
        b
    }
}

struct RetainParent {
    target: PathBuf,
    target_id: SessionId,
    pages_id: SessionId,
    head_path: PathBuf,
    archive_path: PathBuf,
    wal_path: PathBuf,
    wal_stage_path: PathBuf,
    head_frame: Zeroizing<Vec<u8>>,
    archive_frame: Zeroizing<Vec<u8>>,
    head: TextHeadData,
    archive: TextArchiveData,
    completed: Option<Arc<super::completed_workspace::CompletedV2TextParentData>>,
    resources: V2TextReadResources,
    // LAST: shared frozen metadata remains funded after original Cell/owner destruction.
    metadata: V2TextTransactionMetadataResources,
}

/// Exact original identity only, with its own small metadata payment. No large work,
/// packet, File, Store/SDK/key, physical session, positive Scope or predecessor chain.
#[derive(Clone)]
pub(in crate::store) struct OriginalV2TextRetainTransactionObservation {
    identity: Weak<()>,
    original: OriginalV2RetainObservation,
}
impl OriginalV2TextRetainTransactionObservation {
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        self.original.require_same_original(&other.original)?;
        if !self.identity.ptr_eq(&other.identity) {
            return Err(refused());
        }
        Ok(())
    }
}

pub(in crate::store) struct PreparedV2TextRetainTransaction {
    identity: Arc<()>,
    parent: Arc<RetainParent>,
    policy: V2TextQuotaPolicy,
    token: Option<PreparedPrivateChatPage>,
    candidate: Option<PreparedTextControlFrameData>,
    wal: Option<PreparedTextControlFrameData>,
    carry: Zeroizing<Vec<u8>>,
    completed: Option<completed::CompletedRetainOrigin>,
    slots: work::RetainSlots,
    ready: Option<ReadyV2TextRetainSelection>,
    saved: Option<Arc<SavedV2RetainedAction>>,
    #[cfg(test)]
    hooks: work::RetainTestHooks,
    #[cfg(all(test, windows))]
    saved_drop_notice: Option<(
        Arc<std::sync::atomic::AtomicBool>,
        Arc<std::sync::atomic::AtomicBool>,
    )>,
    // Path/plaintext/actual reader cleanup precedes original SDK/work/metadata refunds.
    recovery: Arc<recovery::RetainRecoveryControl>,
    query: PreparedV2TextTokenQuery,
    custody: PreparedV2RetainOriginalCustody,
}
impl PreparedV2TextRetainTransaction {
    /// Owning OUTSIDE-writer transfer. All fallible checks/copies precede either take;
    /// rejection preserves BOTH original input slots and the unchanged outside result.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn take_into(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        predecessor: &BoundV2TextCreatePredecessor,
        policy: V2TextQuotaPolicy,
        custody: &mut Option<PreparedV2RetainOriginalCustody>,
        query: &mut Option<PreparedV2TextTokenQuery>,
        retained: &mut Option<Self>,
    ) -> Result<(), AppError> {
        if let Some(existing) = retained.as_ref() {
            if custody.is_some() || query.is_some() {
                return Err(refused());
            }
            predecessor.require_same_original(&existing.original_retain())?;
            return existing.custody.validate_accounts(store);
        }
        let original = custody.as_ref().ok_or_else(refused)?;
        let original_query = query.as_ref().ok_or_else(refused)?;
        original.validate_accounts(store)?;
        let producer = original.producer();
        producer.validate_resources_original(store, source)?;
        producer.require_selected_data(selected)?;
        predecessor.require_same_original(&producer.observe_original())?;
        original_query.require_token_parent_data(selected, producer.token_data())?;
        if original_query.token_value_data()?.is_some() {
            return Err(refused());
        }
        let h = selected.head_data();
        if h.generation != 1
            || h.selection_attempt != 1
            || h.pending != TextPendingData::None
            || h.tokens != TextRootData::Empty
            || h.publications != TextRootData::Empty
            || h.queue_next != 0
        {
            return Err(refused());
        }
        policy.require_initial_capacity(h.live_bytes, h.live_objects)?;
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
            completed: None,
            resources: producer.resources().clone(),
            metadata: producer.metadata_resources().clone(),
        });
        let carry =
            RetirementCarryData::from_data(&[selected.head_id_data()], &[])?.encode_data()?;
        producer
            .resources()
            .require_bound_original_negative(store, source)?;
        *retained = Some(Self {
            identity: Arc::new(()),
            parent,
            policy,
            token: None,
            candidate: None,
            wal: None,
            carry,
            completed: None,
            slots: work::RetainSlots::empty(),
            ready: None,
            saved: None,
            #[cfg(test)]
            hooks: work::RetainTestHooks::default(),
            #[cfg(all(test, windows))]
            saved_drop_notice: None,
            recovery: Arc::new(recovery::RetainRecoveryControl::new()),
            query: query.take().expect("same checked original reader"),
            custody: custody
                .take()
                .expect("same checked whole SDK/payment custody"),
        });
        Ok(())
    }
    /// Pure final phase CAS only; actual Current/work authority remains Factory-owned.
    pub(in crate::store) fn reserve_stage_fence(&self) -> Result<V2TextRetainStageFence, AppError> {
        self.recovery.claim_stage(self.observe_original())
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        self.custody.resources()
    }
    pub(in crate::store) fn producer_data(&self) -> &PreparedV2RetainData {
        self.custody.producer()
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.custody.observe_original()
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextRetainTransactionObservation {
        OriginalV2TextRetainTransactionObservation {
            identity: Arc::downgrade(&self.identity),
            original: self.original_retain(),
        }
    }
    pub(in crate::store) fn require_request_data(
        &self,
        request: &V2CreateRequest<'_>,
    ) -> Result<(), AppError> {
        self.producer_data().require_request_data(request)
    }
    pub(in crate::store) fn has_ready_data(&self) -> bool {
        self.ready.is_some()
    }
    pub(in crate::store) fn has_saved_data(&self) -> bool {
        self.saved.is_some()
    }
    pub(in crate::store) fn ready_selection(
        &self,
    ) -> Result<&ReadyV2TextRetainSelection, AppError> {
        self.ready.as_ref().ok_or_else(refused)
    }
    pub(in crate::store) fn saved_retained_action(
        &self,
    ) -> Result<&Arc<SavedV2RetainedAction>, AppError> {
        self.saved.as_ref().ok_or_else(refused)
    }

    /// Cache EACH complete new private frame immediately, before a late actual Scope
    /// cancellation can be observed. Retry validates the SAME bound Scope before RNG.
    pub(in crate::store) fn prepare_candidates(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        if self.completed.is_some() {
            return self.prepare_completed_candidates(store, source, rng);
        }
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
            let plain = encode_private_leaf_data(
                PrivateIndexContextData {
                    cid: ctx,
                    kind: PrivateIndexKindData::Token,
                },
                &[PrivateIndexLeafRowData {
                    key: *value.key_data(ctx).as_bytes(),
                    value: PrivateIndexValueData::Token(value),
                }],
                NodePlacementData::Root,
            )
            .map_err(|_| refused())?;
            self.resources()
                .require_bound_original_negative(store, source)?;
            self.token = Some(PreparedPrivateChatPage::prepare(&key, &plain, rng)?);
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
            let node_key = *value.key_data(ctx).as_bytes();
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
                    summary: NodeSummaryData {
                        height: 0,
                        keys: 1,
                        min: node_key,
                        max: node_key,
                    },
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
                live_bytes: parent.live_bytes.checked_add(bytes).ok_or_else(refused)?,
                live_objects: parent
                    .live_objects
                    .checked_add(objects)
                    .ok_or_else(refused)?,
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
            wal.verify_first_retained_a_parent_data(
                &key,
                super::wal::retain::RetainWalParentData {
                    head: &self.parent.head,
                    archive: &self.parent.archive,
                    id: TextHeadIdData::of_data(&self.parent.head_frame),
                    frame: &self.parent.head_frame,
                },
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
    fn artifacts(&self) -> Result<Vec<TextWalArtifactData<'_>>, AppError> {
        let mut entries = Vec::with_capacity(MAX_A_ARTIFACTS);
        for (role, id, frame) in self.producer_data().artifact_frames_data() {
            entries.push(TextWalArtifactData::from_data(role, id, frame)?);
        }
        let token = self.token.as_ref().ok_or_else(refused)?;
        entries.push(TextWalArtifactData::from_data(
            TextArtifactRoleData::TokenIndex,
            *token.id().as_bytes(),
            token.as_bytes(),
        )?);
        entries.sort_by_key(|entry| (entry.role_data(), entry.id_data()));
        Ok(entries)
    }
    fn artifact_totals(&self) -> Result<(u128, u128), AppError> {
        self.artifacts()?
            .iter()
            .try_fold((0u128, 0u128), |(bytes, objects), entry| {
                Ok((
                    bytes
                        .checked_add(entry.frame_data().len() as u128)
                        .ok_or_else(refused)?,
                    objects + 1,
                ))
            })
    }
    pub(in crate::store) fn validate_stage_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.recovery.require_idle()?;
        guard.require_text_control_write_idle_for_query()?;
        self.custody.validate_accounts(guard.store())?;
        self.producer_data()
            .validate_resources_original(guard.store(), guard.source())?;
        if self.saved.is_some() || self.wal.is_none() {
            return Err(refused());
        }
        self.policy
            .require_initial_capacity(self.parent.head.live_bytes, self.parent.head.live_objects)?;
        self.query.validate_completed_under_current_guard(guard)?;
        self.parent.validate_parent(guard, false, None)?;
        Ok(())
    }
}

pub(in crate::store) struct ReadyV2TextRetainSelection {
    original: OriginalV2TextRetainTransactionObservation,
    facts: Arc<work::ReadyRetainFacts>,
    work: Arc<V2TextWorkResources>,
}
impl ReadyV2TextRetainSelection {
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.facts.parent.resources
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextRetainTransactionObservation {
        self.original.clone()
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.original.original.clone()
    }
    pub(in crate::store) fn validate_ready_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        // Enforce SAME Runtime before allocating/opening proof scratch. A foreign
        // real guard must not spend this payer concurrently outside its mutex.
        self.resources().require_same_original(guard.resources())?;
        guard.require_text_control_write_idle_for_query()?;
        self.work.validate_prepaid_account()?;
        self.facts.parent.metadata.validate_prepaid_account()?;
        // Hold every bounded Ready control/artifact BEFORE the final source/ID3
        // proof. The tail must only revisit these same Files/names, never reopen.
        let named = self.facts.capture_named_files(guard)?;
        self.facts
            .parent
            .validate_parent(guard, true, Some(&self.facts.head))?;
        named.require_unchanged()?;
        guard.validate_source_only()
    }
}

/// Historical A only. No publication/projection, obsolete control FD, full WAL/output
/// buffer, SDK packet, large work loan, raw node or prior receipt chain is retained.
pub(in crate::store) struct SavedV2RetainedAction {
    head: TextHeadIdData,
    // These small copied selected frames/facts drain BEFORE either metadata payer.
    selected: Zeroizing<Vec<u8>>,
    selected_facts: work::SavedRetainFacts,
    original: OriginalV2TextRetainTransactionObservation,
    // LAST: the last escaping1.25MiB transaction metadata debit outlives ALL selected facts/buffers.
    parent: Arc<RetainParent>,
}
impl SavedV2RetainedAction {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn selected_paths_for_test(&self) -> (&Path, &Path, &Path, &Path) {
        let (archive, token) = self.selected_facts.paths_data();
        (
            &self.parent.head_path,
            archive,
            token,
            self.parent.pages().expect("genuine frozen archive parent"),
        )
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.parent.resources
    }
    pub(in crate::store) fn head_id_data(&self) -> TextHeadIdData {
        self.head
    }
    pub(in crate::store) fn validate_selected_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.parent
            .validate_parent(guard, true, Some(&self.selected))?;
        self.selected_facts
            .validate_selected(guard, &self.parent, &self.selected)
    }
}

impl RetainParent {
    fn pages(&self) -> Result<&Path, AppError> {
        self.archive_path.parent().ok_or_else(refused)
    }
    fn validate_parent(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        allow_selected: bool,
        selected: Option<&[u8]>,
    ) -> Result<(), AppError> {
        self.resources.require_same_original(guard.resources())?;
        self.resources
            .validate_original(guard.store(), guard.source())?;
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        if original_context(guard, &key)? != self.head.context {
            return Err(refused());
        }
        let (target, file, id) = guard.target_directory_data();
        if target != self.target || id != self.target_id {
            return Err(refused());
        }
        // A child with equal ciphertext in a replacement text.pages namespace is not
        // the actual selected source. Keep the SAME original directory held throughout.
        let pages_path = self.pages()?;
        let pages = crate::store::persistence::open_archive_directory(pages_path)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            pages_path,
            &pages,
            self.pages_id,
        )?;
        let head = read_control(&self.head_path, 1725)?;
        if head.frame != self.head_frame
            && (!allow_selected || selected != Some(head.frame.as_slice()))
        {
            return Err(refused());
        }
        let archive = read_control(&self.archive_path, self.archive_frame.len())?;
        if archive.frame != self.archive_frame {
            return Err(refused());
        }
        guard.validate_original_namespace()?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(target, file, id)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            pages_path,
            &pages,
            self.pages_id,
        )?;
        require_named(&self.archive_path, &archive)?;
        require_named(&self.head_path, &head)?;
        guard.validate_source_only()
    }
}
fn owned_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().len() > MAX_PATH {
        return Err(refused());
    }
    let mut out = PathBuf::with_capacity(path.as_os_str().len());
    out.push(path);
    if out.capacity() > MAX_PATH {
        return Err(refused());
    }
    Ok(out)
}
fn fixed_child(base: &Path, name: &'static str) -> Result<PathBuf, AppError> {
    if !matches!(
        name,
        "text.wal" | "text.wal.stage.v1" | "head.stage" | "text.seed" | "text.seed.stage.v1"
    ) {
        return Err(refused());
    }
    let cap = base
        .as_os_str()
        .len()
        .checked_add(1 + name.len())
        .filter(|n| *n <= MAX_PATH)
        .ok_or_else(refused)?;
    let mut out = PathBuf::with_capacity(cap);
    out.push(base);
    out.push(name);
    if out.capacity() > MAX_PATH {
        return Err(refused());
    }
    Ok(out)
}
macro_rules! redacted_debug{($($ty:ident),+$(,)?)=>{$(impl std::fmt::Debug for $ty{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.debug_struct(stringify!($ty)).finish_non_exhaustive()}})+};}
redacted_debug!(
    OriginalV2TextRetainTransactionObservation,
    PreparedV2TextRetainTransaction,
    ReadyV2TextRetainSelection,
    SavedV2RetainedAction
);

#[cfg(all(test, windows))]
pub(in crate::store) use intent::V2TextIntentTestCut;
pub(in crate::store) use intent::{
    OriginalV2TextUnsignedIntentObservation, PreparedV2TextUnsignedIntent,
    ReadyV2TextUnsignedIntentSelection, SavedV2UnsignedAppendIntent,
};

#[cfg(all(test, windows))]
pub(in crate::store) use intent::V2TextRawSignatureTestCut;
pub(in crate::store) use intent::{
    OriginalV2TextRawSignatureObservation, PreparedV2TextRawSignature,
    ReadyV2TextRawSignatureSelection, SavedV2RawAppendSignature,
};

#[cfg(all(test, windows))]
pub(in crate::store) use intent::V2TextAcceptedCreateTestCut;
pub(in crate::store) use intent::{
    OriginalV2TextAcceptedCreateObservation, PreparedV2TextAcceptedCreate,
    ReadyV2TextAcceptedCreateSelection, SavedV2AcceptedCreate,
};

pub(in crate::store) use intent::V2TextRetirementRootsClaim;

#[cfg(all(test, windows))]
pub(in crate::store) use intent::V2TextRetirementTestCut;
pub(in crate::store) use intent::{
    ReadyV2TextRetirementCompletion, SavedV2TextRetirementCompletion, SavedV2TextRetirementDeletion,
};

pub(in crate::store) use intent::CompletedV2TextRetirementSourceData;
