//! Owning first accepted-B selection through paired actual Stage/Commit consumers.
//! Exact WD/WC cleanup and the final resident handoff remain separate unfinished roles. It consumes the WHOLE S only
//! after its SAME actual Core candidate exists; no Header/model/raw bytes are inputs.
//! Old A/I/S metadata1.25MiB +NEW Bmetadata2MiB +active16MiB work are distinct payments.
use super::*;
use crate::store::persistence::v2_text::archive::{
    TextArchiveData, TextArchiveOperationData, TextArchiveSpecData,
};
use crate::store::persistence::v2_text::artifacts::TextArtifactRoleData;
use crate::store::persistence::v2_text::context::TextContextData;
use crate::store::persistence::v2_text::records::PublicationRecordData;
use crate::store::persistence::v2_text::request::{
    AcceptedLocatorData, PublicationPhaseData, PublicationValueData, TextPublicationIdData,
};
use crate::store::persistence::v2_text::retirement::{
    RetirementEntryData, RetirementKindData, RetirementPlanData,
};
use catcoms_replication::chat::cold_index::{
    DirectoryData, DirectoryTreeData, LeafRowData, ValueData,
};
use catcoms_replication::chat::owner_header::SignedOwnerHeader;
use catcoms_storage::private_chat_page::PrivateChatPageId;
mod advance;
mod named;
mod next;
mod producer;
mod resources;
mod semantic;
mod source;
mod warm;
mod work;
pub(in crate::store) use resources::V2TextRetirementRootsClaim;
use resources::{AcceptedMetadataResources, ACCEPTED_METADATA_FEE};
#[cfg(all(test, windows))]
pub(in crate::store) use work::V2TextAcceptedCreateTestCut;

const MAX_ACCEPTED_ARTIFACTS: usize = 70;
// Full simultaneous NEW B metadata invoice; the old A/I/S1.25MiB remains held in FULL.
// Persistent named artifacts can use70 paths +target/pages/head. Current Ready is
// bounded by75 paths/72payloadFiles; advance/final nests the flat proof with a
// pre-retirement journal proof, so80 paths and TWO80-file backings are charged.
// Payload pages/WAL original bytes live under SAME16M; the flat facts intentionally
// copy their first-B complete closure (75,133B maximum) under this NEW debit.
const ACCEPTED_FIRST_ARTIFACT_FRAME_BYTES: usize =
    1110 + 1519 + 2 * 16470 + 16470 + 16470 + 1148 + 2600 + 2565 + 311;
// Closed first-B10-artifact profile: strict complete-frame bound for the full
// original WB comparison. General70-page COW is not admitted by this producer.
const FIRST_B_WORK_WAL_FRAME_BYTES: usize =
    3686 + 36 + 2022 + 4 + 6301 + 10 * 42 + ACCEPTED_FIRST_ARTIFACT_FRAME_BYTES + 40;
// Full simultaneous SAME16M: retained A producer+WA, inert I/S, exact B DAG, WAL
// encode/seal/rawread overlap and bounded node/index/native comparison scratch.
// Old A/I/S metadata, SDK/model, read/query and NEW Bmetadata are ADDITIONAL.
const ACCEPTED_WORK_LAYOUT_BYTES: usize = RETAIN_DATA_LAYOUT_BYTES
    + 2 * (WAL_PLAIN_ALLOCATION_CAP + 40)
    + 8 * FIRST_B_WORK_WAL_FRAME_BYTES
    + 8 * ACCEPTED_FIRST_ARTIFACT_FRAME_BYTES
    + 16 * 16470
    + 16 * 1725
    + 128 * std::mem::size_of::<std::fs::File>()
    + 256 * 1024;
const _: () = assert!(ACCEPTED_WORK_LAYOUT_BYTES <= TEXT_WORK_BUFFER_FEE);
const ACCEPTED_PERSISTENT_METADATA_BYTES:usize=
    4*std::mem::size_of::<PreparedV2TextAcceptedCreate>()
    +4*std::mem::size_of::<AcceptedParent>()
    +8*std::mem::size_of::<OriginalV2TextAcceptedCreateObservation>()
    +4*std::mem::size_of::<ReadyV2TextAcceptedCreateSelection>()
    +4*std::mem::size_of::<SavedV2AcceptedCreate>()
    +4*std::mem::size_of::<source::AcceptedSelectedFacts>()
    +2*MAX_ACCEPTED_ARTIFACTS*std::mem::size_of::<source::AcceptedArtifactFact>()
    +64*std::mem::size_of::<RetirementEntryData>()
    +75*(MAX_PATH+256)
    +ACCEPTED_FIRST_ARTIFACT_FRAME_BYTES+8*(1725+16470+634+676)
    +crate::store::critical_namespaces::current_layout::V2_TEXT_ACCEPTED_CREATE_ADMISSION_LAYOUT_BYTES
    +crate::store::critical_namespaces::current_layout::V2_TEXT_ACCEPTED_ADVANCE_GUARD_LAYOUT_BYTES
    +PreparedV2UnsignedOriginalCustody::accepted_advance_metadata_layout_bytes()
    +32*1024;
const ACCEPTED_CURRENT_METADATA_BYTES: usize =
    80 * (MAX_PATH + 256) + 2 * named::ACCEPTED_NAMED_FILES_LAYOUT_BYTES;
const ACCEPTED_METADATA_LAYOUT_BYTES: usize = ACCEPTED_PERSISTENT_METADATA_BYTES
    + ACCEPTED_CURRENT_METADATA_BYTES
    + resources::ACCEPTED_PAYMENT_LAYOUT_BYTES
    + crate::store::owner_evidence::canonical::text_lane::V2_TEXT_RETIREMENT_ROOTS_LAYOUT_BYTES;
// Actual Windows test-target measurement:1,232,709 bytes (772,549 persistent
// +460,160 complete Current scratch). Reserve a NEW2MiB class in the SAME pool;
// old A/I/S1.25MiB,16MiB work and independent SDK/model/read fees are additional.
// Compile the FULL named invoice, never a locally smaller cap or discounted term.
const _: () = assert!(
    ACCEPTED_METADATA_LAYOUT_BYTES
        + retirement::RETIREMENT_METADATA_LAYOUT_BYTES
        + retirement::COMPLETED_FINISH_METADATA_LAYOUT_BYTES
        <= ACCEPTED_METADATA_FEE
);
const _: () = assert!(
    ACCEPTED_WORK_LAYOUT_BYTES
        + retirement::RETIREMENT_WORK_LAYOUT_BYTES
        + next::COMPLETED_ROOT_LEAF_WORK_BYTES
        <= TEXT_WORK_BUFFER_FEE
);

struct AcceptedPage {
    page: PreparedPrivateChatPage,
    plain_bytes: usize,
    entries: u16,
}
struct AcceptedParent {
    head: TextHeadData,
    head_frame: Zeroizing<Vec<u8>>,
    directory: DirectoryData,
    directory_root: NodeRefData,
    directory_frame_bytes: usize,
    token_root: NodeRefData,
    token_frame: Zeroizing<Vec<u8>>,
    header: SignedOwnerHeader,
    image: Zeroizing<Vec<u8>>,
    // Staging parent only. A future SavedB is FLAT and must not retain or revalidate
    // this old I/S/TA closure after its exact WD plan has legitimately retired them.
    signature: Arc<SavedV2RawAppendSignature>,
    // LAST: every new B observer/fact owns this additional actual metadata debit.
    metadata: AcceptedMetadataResources,
}
#[derive(Clone)]
pub(in crate::store) struct OriginalV2TextAcceptedCreateObservation {
    identity: Weak<()>,
    signature: OriginalV2TextRawSignatureObservation,
    metadata: AcceptedMetadataResources,
}
impl OriginalV2TextAcceptedCreateObservation {
    pub(in crate::store) fn validate_prepaid_account(&self) -> Result<(), AppError> {
        self.metadata.validate_prepaid_account()
    }
    pub(in crate::store) fn require_original_unsigned(
        &self,
        original: &OriginalV2UnsignedPlanObservation,
    ) -> Result<(), AppError> {
        self.signature
            .original
            .original
            .require_same_original(original)
    }
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        self.signature.require_same_original(&other.signature)?;
        self.metadata.require_same_original(&other.metadata)?;
        if !self.identity.ptr_eq(&other.identity) {
            return Err(refused());
        }
        Ok(())
    }
}
/// All frames cache independently before post-seal negatives. This is a fixed DAG:
/// three content pages?manifest/HB?new private roots/P?OB?Plan/WC?WD?WB. No OB
/// self-reference, re-sign, replay fallback or mutable SDK/model extraction exists.
pub(in crate::store) struct PreparedV2TextAcceptedCreate {
    identity: Arc<()>,
    completed: Option<next::CompletedAcceptedOrigin>,
    parent: Arc<AcceptedParent>,
    records: Option<AcceptedPage>,
    nodes: Option<AcceptedPage>,
    header_body: Option<AcceptedPage>,
    manifest: Option<PreparedPrivateChatPage>,
    archive: Option<PreparedTextControlFrameData>,
    actions: Option<PreparedPrivateChatPage>,
    directory: Option<PreparedPrivateChatPage>,
    token: Option<PreparedPrivateChatPage>,
    publications: Option<PreparedPrivateChatPage>,
    publication: Option<PreparedTextControlFrameData>,
    candidate: Option<PreparedTextControlFrameData>,
    plan: Option<Zeroizing<Vec<u8>>>,
    completion: Option<PreparedTextControlFrameData>,
    deletion: Option<PreparedTextControlFrameData>,
    wal: Option<PreparedTextControlFrameData>,
    slots: work::AcceptedSlots,
    ready: Option<ReadyV2TextAcceptedCreateSelection>,
    saved: Option<Arc<SavedV2AcceptedCreate>>,
    retirement_work: retirement::RetirementWorkState,
    #[cfg(test)]
    hooks: work::AcceptedTestHooks,
    #[cfg(all(test, windows))]
    saved_drop_notice: Option<(
        Arc<std::sync::atomic::AtomicBool>,
        Arc<std::sync::atomic::AtomicBool>,
    )>,
    // SAME actual16MiB and real verifier/candidate/SDK/model remain in whole S.
    signature: PreparedV2TextRawSignature,
    // LAST: Box/body/descriptor/observer and all Ready/Saved new-source facts must
    // drain before this independent B2MiB refunds, even after old A/I/S drops.
    metadata: AcceptedMetadataResources,
}
impl PreparedV2TextAcceptedCreate {
    /// OUTSIDE-writer owning Box handoff. NEW Bmetadata is acquired/rechecked BEFORE
    /// any B copies/Arc/Box allocation; every fallible check precedes the only take.
    /// Missing actual candidate/raw/source leaves the entire original S unchanged.
    pub(in crate::store) fn take_into(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        input: &mut Option<PreparedV2TextRawSignature>,
        output: &mut Option<Box<Self>>,
    ) -> Result<(), AppError> {
        if let Some(existing) = output.as_ref() {
            if existing.completed.is_some() || input.is_some() {
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
        if signature.intent.retained.is_completed_parent_data() {
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
            || head.publications != TextRootData::Empty
            || head.queue_front != 0
            || head.queue_next != 0
        {
            return Err(refused());
        }
        // This first accepted producer is paired to the actual first-A source, not
        // caller Empty. Its immutable completed Directory query supplies the row AND
        // exact observed cipher size. General later-source COW is a separate role.
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
            || directory.tree != DirectoryTreeData::Empty
            || directory.accepted_leaf_count != 0
            || directory.unique_action_count != 0
            || directory.own_tip_header != signature.record.public_parent
            || candidate.append_data().old_tip().count != 0
            || candidate.append_data().new_tip().count != 1
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
            || token_root.summary.keys != 1
        {
            return Err(refused());
        }
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
                completed: None,
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
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        self.signature.resources()
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextAcceptedCreateObservation {
        OriginalV2TextAcceptedCreateObservation {
            identity: Arc::downgrade(&self.identity),
            signature: self.signature.observe_original(),
            metadata: self.metadata.clone(),
        }
    }
    pub(in crate::store) fn original_signature(&self) -> OriginalV2TextRawSignatureObservation {
        self.signature.observe_original()
    }
    pub(in crate::store) fn original_intent(&self) -> OriginalV2TextUnsignedIntentObservation {
        self.signature.original_intent()
    }
    pub(in crate::store) fn original_unsigned(&self) -> OriginalV2UnsignedPlanObservation {
        self.signature.original_unsigned()
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.signature.original_retain()
    }
    pub(in crate::store) fn require_request_data(
        &self,
        request: &V2CreateRequest<'_>,
    ) -> Result<(), AppError> {
        self.signature.require_request_data(request)
    }
}

/// Active Ready data retains complete original B outputs and the original work debit.
/// It cannot become a receipt without the distinct real Commit and parent barrier.
pub(in crate::store) struct ReadyV2TextAcceptedCreateSelection {
    original: OriginalV2TextAcceptedCreateObservation,
    facts: Arc<source::AcceptedSelectedFacts>,
    parent: Arc<AcceptedParent>,
    wal_native: SessionId,
    wal_len: usize,
    work: Arc<V2TextWorkResources>,
    metadata: AcceptedMetadataResources,
}
impl ReadyV2TextAcceptedCreateSelection {
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.facts.resources
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextAcceptedCreateObservation {
        self.original.clone()
    }
    pub(in crate::store) fn original_signature(&self) -> OriginalV2TextRawSignatureObservation {
        self.original.signature.clone()
    }
    pub(in crate::store) fn original_intent(&self) -> OriginalV2TextUnsignedIntentObservation {
        self.original.signature.original.clone()
    }
    pub(in crate::store) fn original_unsigned(&self) -> OriginalV2UnsignedPlanObservation {
        self.original.signature.original.original.clone()
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.original.signature.original.retained.original.clone()
    }
    pub(in crate::store) fn validate_ready_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        guard.require_text_control_write_idle_for_query()?;
        self.work.validate_prepaid_account()?;
        self.metadata.validate_prepaid_account()?;
        self.original_unsigned().validate_negative()?;
        let mut held = self.facts.capture_ready(guard, &self.parent.head_frame)?;
        held.add_file(
            owned_path(&self.parent.signature.parent.base.wal_path)?,
            self.wal_native,
            self.wal_len,
        )?;
        held.add_absence(owned_path(
            &self.parent.signature.parent.base.wal_stage_path,
        )?)?;
        self.facts.validate_original(guard)?;
        held.require_unchanged()?;
        self.original_unsigned().validate_negative()?;
        guard.validate_source_only()
    }
}
/// Flat newly selected B closure. No old I/S/TA/Directory File or parent source is
/// retained/reopened: WD may legitimately retire those exact private predecessors.
pub(in crate::store) struct SavedV2AcceptedCreate {
    head: TextHeadIdData,
    facts: Arc<source::AcceptedSelectedFacts>,
    original: OriginalV2TextAcceptedCreateObservation,
    metadata: AcceptedMetadataResources,
}
impl SavedV2AcceptedCreate {
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.facts.resources
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextAcceptedCreateObservation {
        self.original.clone()
    }
    pub(in crate::store) fn original_signature(&self) -> OriginalV2TextRawSignatureObservation {
        self.original.signature.clone()
    }
    pub(in crate::store) fn original_intent(&self) -> OriginalV2TextUnsignedIntentObservation {
        self.original.signature.original.clone()
    }
    pub(in crate::store) fn original_unsigned(&self) -> OriginalV2UnsignedPlanObservation {
        self.original.signature.original.original.clone()
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.original.signature.original.retained.original.clone()
    }
    pub(in crate::store) fn selection_expectation_data(
        &self,
    ) -> catcoms_replication::chat::V2HostSelectionExpectation {
        catcoms_replication::chat::V2HostSelectionExpectation::from_trusted_host(
            self.facts.header.id(),
            self.facts.header.spec().state,
        )
    }
    pub(in crate::store) fn validate_selected_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        if self.head != TextHeadIdData::of_data(&self.facts.selected) {
            return Err(refused());
        }
        guard.require_text_control_write_idle_for_query()?;
        self.metadata.validate_prepaid_account()?;
        let held = self.facts.capture_selected(guard)?;
        self.facts.validate_original(guard)?;
        held.require_unchanged()?;
        guard.validate_source_only()
    }
}
impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn has_ready_data(&self) -> bool {
        self.ready.is_some()
    }
    pub(in crate::store) fn has_saved_data(&self) -> bool {
        self.saved.is_some()
    }
    pub(in crate::store) fn ready_selection(
        &self,
    ) -> Result<&ReadyV2TextAcceptedCreateSelection, AppError> {
        self.ready.as_ref().ok_or_else(refused)
    }
    pub(in crate::store) fn saved_accepted_create(
        &self,
    ) -> Result<&Arc<SavedV2AcceptedCreate>, AppError> {
        self.saved.as_ref().ok_or_else(refused)
    }
    pub(in crate::store) fn validate_stage_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        guard.require_text_control_write_idle_for_query()?;
        self.metadata.validate_prepaid_account()?;
        self.require_original_candidates(guard.store(), guard.source())?;
        if self.wal.is_none() {
            return Err(refused());
        }
        self.parent
            .signature
            .original
            .require_same_original(&self.signature.observe_original())?;
        self.parent
            .signature
            .validate_selected_under_current_guard(guard)?;
        self.original_unsigned().validate_negative()?;
        guard.validate_source_only()
    }
}

#[cfg(all(test, windows))]
impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn observe_saved_facts_drop_for_test(
        &mut self,
        gone: Arc<std::sync::atomic::AtomicBool>,
        refunded: Arc<std::sync::atomic::AtomicBool>,
    ) {
        assert!(
            self.ready.is_none(),
            "install actual B facts observation beforeStage"
        );
        self.saved_drop_notice = Some((gone, refunded));
    }
    /// Actual cached plan names only; this read-only test loan grants no deletion.
    pub(in crate::store) fn original_retired_paths_for_test(
        &self,
    ) -> Result<Vec<PathBuf>, AppError> {
        let plan = RetirementPlanData::decode_data(self.plan.as_ref().ok_or_else(refused)?)?;
        let base = &self.parent.signature.parent.base;
        plan.carry_data()
            .entries_data()
            .iter()
            .map(|e| match e.kind {
                RetirementKindData::AcceptedIndex | RetirementKindData::TokenIndex => {
                    work::artifact_child(base.pages()?, TextArtifactRoleData::AcceptedIndex, &e.id)
                }
                RetirementKindData::Intent => owned_path(&self.signature.parent.intent_path),
                RetirementKindData::Signature => owned_path(&self.parent.signature.signature_path),
                _ => Err(refused()),
            })
            .collect()
    }
}

#[cfg(all(test, windows))]
mod constructor_tests;

mod retirement;
#[cfg(all(test, windows))]
pub(in crate::store) use retirement::V2TextRetirementTestCut;
pub(in crate::store) use retirement::{
    ReadyV2TextRetirementCompletion, SavedV2TextRetirementCompletion, SavedV2TextRetirementDeletion,
};

impl PreparedV2TextAcceptedCreate {
    /// OUTSIDE writers, before ANY RootClose array/Arc/identity allocation.
    pub(in crate::store) fn claim_retirement_roots_preparation(
        &self,
    ) -> Result<V2TextRetirementRootsClaim, AppError> {
        self.metadata.claim_retirement_roots_preparation()
    }
}

pub(in crate::store) use retirement::CompletedV2TextRetirementSourceData;
