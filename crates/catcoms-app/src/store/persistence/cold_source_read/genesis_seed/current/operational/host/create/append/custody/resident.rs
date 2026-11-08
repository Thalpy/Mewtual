//! SAME paid verifier -> flat resident after actual WC and retired-lane proof.
//!
//! Parent DATA is prepared outside writers under B's actual metadata payment.
//! The concrete finish guard independently proves fresh WC/new roots and retired
//! lane. No Saved, observation, SDK, read/query or B fee follows the model.
use super::*;
mod next;
mod next_unsigned;
use crate::store::critical_namespaces::current_layout::SavedCompletedV2TextFinishGuard;
use crate::store::persistence::v2_text::{
    CompletedV2TextRetirementSourceData, OriginalV2TextAcceptedCreateObservation, TextContextData,
    TextHeadIdData, TextPendingData, TextRequestDigestData, TextRetirementPlanIdData, TextRootData,
    TextTransactionIdData,
};
use catcoms_replication::chat::cold_index::{DirectoryData, DirectoryTreeData, NodeRefData};
use catcoms_replication::chat::cold_manifest::ArchiveHeadId;
use catcoms_replication::chat::owner_header::{OwnerHeaderId, OwnerStateCommitment};
pub(in crate::store) use next::{OriginalV2TextResidentObservation, V2TextResidentParentData};
pub(in crate::store) use next_unsigned::PreparedV2NextUnsignedAppend;

/// Full bounded parent DATA, including the exact Actions state. A subsequent
/// request still needs fresh WC/accepted/private-root/source proof and new fees.
/// No old Saved, WAL, File, reader, SDK or observation payment lives here.
pub(super) struct ResidentSelectedParent {
    context: TextContextData,
    completion_frame_id: [u8; 32],
    completed_transaction: TextTransactionIdData,
    completed_plan: TextRetirementPlanIdData,
    selected: TextHeadIdData,
    public_header: OwnerHeaderId,
    public_state: OwnerStateCommitment,
    archive: ArchiveHeadId,
    directory_root: NodeRefData,
    founder: DirectoryData,
    token_root: Option<NodeRefData>,
    publication_root: Option<NodeRefData>,
    generation: u64,
    selection_attempt: u64,
    queue_front: u64,
    queue_next: u64,
    live_bytes: u128,
    live_objects: u128,
    reclaimed_bytes: u128,
    reclaimed_objects: u128,
}

/// Parent DATA drops before the SAME boxed verifier/domain/model-payment tails.
/// Neither field contains a completed receipt, SDK, read/query or B metadata fee.
pub(in crate::store) struct PreparedV2TextResidentProjection {
    selected: ResidentSelectedParent,
    owner: Box<ResidentVerifierOwner>,
}

// The domain invoice already names TWO complete verifier metadata layouts. Add
// the exact owner wrapper/identity and flat outer move slots/backings; the existing
// independent domain16KiB cap is unchanged and must still assert as a TOTAL.
pub(in crate::store) const V2_TEXT_RESIDENT_DOMAIN_LAYOUT_BYTES: usize = 2
    * (std::mem::size_of::<ResidentVerifierOwner>()
        - std::mem::size_of::<V2FounderPrefixVerifier>())
    + 2 * std::mem::size_of::<Option<PreparedV2TextResidentProjection>>()
    + 2 * catcoms_replication::chat::MAX_GROUP_ID_BYTES
    + 6 * 64
    + next::RESIDENT_NEXT_METADATA_LAYOUT_BYTES;

// Actual NEW B2MiB names its independently cached preparation and move copies
// before allocation; added cell fields also join its existing exact cell term.
pub(super) const V2_TEXT_RESIDENT_HANDOFF_COPY_LAYOUT_BYTES: usize =
    2 * std::mem::size_of::<ResidentSelectedParent>()
        + 2 * catcoms_replication::chat::MAX_GROUP_ID_BYTES
        + 4 * std::mem::size_of::<std::sync::Weak<()>>()
        + 4 * 64;

impl PreparedV2UnsignedOriginalCustody {
    pub(in crate::store) const fn resident_handoff_metadata_layout_bytes() -> usize {
        V2_TEXT_RESIDENT_HANDOFF_COPY_LAYOUT_BYTES
    }

    /// OUTSIDE writers after genuine SavedWC is cached. Checks and one bounded
    /// context copy precede final proof. A retry compares the existing preparation.
    pub(in crate::store) fn prepare_completed_resident_data(
        &mut self,
        retained: &PreparedV2TextRetainTransaction,
        source: &CompletedV2TextRetirementSourceData<'_>,
    ) -> Result<(), AppError> {
        self.validate_completed_model_source(retained, source)?;
        let cell = self.advance.as_mut().ok_or_else(refused)?;
        if let Some(parent) = cell.resident_parent.as_ref() {
            parent.require_source(source)?;
        } else {
            cell.resident_parent = Some(ResidentSelectedParent::copy_source(source)?);
        }
        // Keep the completed parent before every late negative/lost return.
        self.original.validate_negative()
    }

    /// Full account/SDK/model/snapshot association BEFORE the finish issuer's
    /// FINAL genuine WC/newB proof. This does not itself issue WC/lane permission.
    pub(in crate::store) fn validate_completed_resident_data(
        &self,
        retained: &PreparedV2TextRetainTransaction,
        source: &CompletedV2TextRetirementSourceData<'_>,
    ) -> Result<(), AppError> {
        self.validate_completed_model_source(retained, source)?;
        self.advance
            .as_ref()
            .ok_or_else(refused)?
            .resident_parent
            .as_ref()
            .ok_or_else(refused)?
            .require_source(source)
    }
    fn validate_completed_model_source(
        &self,
        retained: &PreparedV2TextRetainTransaction,
        source: &CompletedV2TextRetirementSourceData<'_>,
    ) -> Result<(), AppError> {
        self.validate_accounts_only()?;
        source.original_data().validate_prepaid_account()?;
        self.require_accepted_advance_cached(retained, source.original_data())?;
        let cell = self.advance.as_ref().ok_or_else(refused)?;
        if cell.phase != advance::AppendAdvancePhase::AdvancedFinal
            || cell.retired.is_some()
            || cell.basis.is_none()
            || cell.transferred_identity.is_some()
        {
            return Err(refused());
        }
        if self
            .data
            .resident_owner_data()?
            .model
            .replay_data()
            .header()
            != source.public_header_data()
        {
            return Err(refused());
        }
        self.original.validate_negative()
    }

    /// ALL fallible checks precede the owning cut. Success returns immediately
    /// after caching resident + explicit transferred phase: Physical MUST next
    /// infallibly cache WHOLE Box<B> into outside disposal before any late check,
    /// hook, callback, destructor or Result. No allocation/source/model drop here.
    pub(in crate::store) fn transfer_completed_resident_under_guard(
        &mut self,
        retained: &PreparedV2TextRetainTransaction,
        original: &OriginalV2TextAcceptedCreateObservation,
        guard: &SavedCompletedV2TextFinishGuard<'_, '_, '_>,
        output: &mut Option<PreparedV2TextResidentProjection>,
    ) -> Result<(), AppError> {
        guard.require_original_accepted(original)?;
        guard.require_original_unsigned(&self.original)?;
        guard.require_original_retain(&retained.original_retain())?;
        if let Some(resident) = output.as_ref() {
            self.require_transferred_resident_data(original, resident)?;
            guard.require_completion_frame_id_data(&resident.selected.completion_frame_id)?;
            return guard.validate_negative();
        }
        self.require_accepted_advance_cached(retained, original)?;
        let cell = self.advance.as_ref().ok_or_else(refused)?;
        let parent = cell.resident_parent.as_ref().ok_or_else(refused)?;
        let owner = self.data.resident_owner_data()?;
        if cell.phase != advance::AppendAdvancePhase::AdvancedFinal
            || cell.retired.is_some()
            || cell.basis.is_none()
            || cell.transferred_identity.is_some()
            || owner.model.replay_data().header().id() != parent.public_header
            || owner.model.replay_data().header().spec().state != parent.public_state
        {
            return Err(refused());
        }
        guard.require_completion_frame_id_data(&parent.completion_frame_id)?;
        self.original.validate_negative()?;
        guard.validate_negative()?;
        // Downgrade is infallible. It retains no transaction or read resources.
        let identity = Arc::downgrade(&owner.identity);
        let owner = self
            .data
            .resident_owner
            .take()
            .expect("checked SAME resident owner");
        let cell = self
            .advance
            .as_mut()
            .expect("checked SAME completed advance cell");
        let selected = cell
            .resident_parent
            .take()
            .expect("checked completed flat parent");
        *output = Some(PreparedV2TextResidentProjection { selected, owner });
        cell.transferred_identity = Some(identity);
        cell.phase = advance::AppendAdvancePhase::TransferredResident;
        Ok(())
    }

    /// Cached exact-owner association for lost-return retry while the old B stays
    /// outside. No Current, account wait or positive origin is renewed.
    pub(in crate::store) fn require_transferred_resident_data(
        &self,
        original: &OriginalV2TextAcceptedCreateObservation,
        resident: &PreparedV2TextResidentProjection,
    ) -> Result<(), AppError> {
        original.require_original_unsigned(&self.original)?;
        let cell = self.advance.as_ref().ok_or_else(refused)?;
        cell.accepted.require_same_original(original)?;
        if cell.phase != advance::AppendAdvancePhase::TransferredResident
            || cell.retired.is_some()
            || cell.basis.is_none()
            || cell.resident_parent.is_some()
            || self.data.resident_owner.is_some()
            || self.data.candidate.is_some()
            || !cell
                .transferred_identity
                .as_ref()
                .ok_or_else(refused)?
                .ptr_eq(&Arc::downgrade(&resident.owner.identity))
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Negative finite cleanup still validates the remaining original SDK fee.
    /// Only explicit transfer permits missing model custody; no positive path can
    /// derive or adopt a replacement after that cut.
    pub(super) fn validate_model_account_or_transferred(&self) -> Result<(), AppError> {
        if let Some(owner) = self.data.resident_owner.as_ref() {
            owner
                .model_fee
                .validate(&self.pool, REPLAY_MATH_PRIVATE_INVOICE)?;
            return owner.domain.validate_accounts_only();
        }
        let cell = self.advance.as_ref().ok_or_else(refused)?;
        if cell.phase != advance::AppendAdvancePhase::TransferredResident
            || cell.retired.is_some()
            || cell.basis.is_none()
            || cell.resident_parent.is_some()
            || cell.transferred_identity.is_none()
            || self.data.candidate.is_some()
        {
            return Err(refused());
        }
        Ok(())
    }
}
impl PreparedV2TextResidentProjection {
    /// Historical recognition of only the LAST completed request, OUTSIDE writers
    /// (context hashing may allocate). This is neither a canonical token hit nor
    /// accepted projection, new-Create, quota, publication or effect permission.
    /// It works after all old request/SDK/read/B owners have actually drained.
    pub(in crate::store) fn require_completed_request_data(
        &self,
        request: &catcoms_sync::owner_v2::V2CreateRequest<'_>,
    ) -> Result<(), AppError> {
        let context = self.selected.context.id_data()?;
        let digest = TextRequestDigestData::from_request_data(
            context,
            request.token,
            request.channel,
            request.text,
        )?;
        if TextTransactionIdData::from_request_data(context, request.token, digest)
            != self.selected.completed_transaction
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Accounting/negative only, never new-request or current projection admission.
    pub(in crate::store) fn validate_negative(&self, store: &ServerStore) -> Result<(), AppError> {
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.owner
            .model_fee
            .validate(pool, REPLAY_MATH_PRIVATE_INVOICE)?;
        self.owner.domain.validate_accounts(store)?;
        self.owner.domain.validate_negative()
    }

    /// Fixed original-object observations, never a replay/model/SDK loan. Compare
    /// the actual installed model with the flat WC parent after whole B disposal.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn resident_header_state_for_test(&self) -> ([u8; 32], [u8; 32], u64) {
        (
            *self.owner.model.replay_data().header().id().as_bytes(),
            *self.selected.public_header.as_bytes(),
            self.owner.model.replay_data().header().spec().sequence,
        )
    }

    /// Preserve raw selected O_B counters and exact one-time WC reclaimed totals
    /// separately. No mutable counter/refund or request-admission API is exposed.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn resident_quota_state_for_test(&self) -> (u128, u128, u128, u128) {
        (
            self.selected.live_bytes,
            self.selected.live_objects,
            self.selected.reclaimed_bytes,
            self.selected.reclaimed_objects,
        )
    }

    #[cfg(all(test, windows))]
    pub(in crate::store) fn cancel_original_domain_for_test(&self) {
        self.owner.domain.cancel();
    }
}

#[cfg(all(test, windows))]
impl PreparedV2UnsignedOriginalCustody {
    /// Register before the real fixture constructs its original domain. This is
    /// only a destructor observation of that actual independent16KiB debit.
    pub(in crate::store) fn with_resident_domain_fee_notice_for_test<T>(
        notice: Arc<std::sync::atomic::AtomicBool>,
        run: impl FnOnce() -> T,
    ) -> T {
        V2TextProjectionDomain::with_fee_drop_notice_for_test(notice, run)
    }
}
impl ResidentSelectedParent {
    fn copy_source(source: &CompletedV2TextRetirementSourceData<'_>) -> Result<Self, AppError> {
        let head = source.head_data();
        head.validate_data()?;
        let TextRootData::Present(directory_root) = head.directory else {
            return Err(refused());
        };
        let founder = source.founder_data();
        if head.pending != TextPendingData::None
            || founder.tree == DirectoryTreeData::Unknown
            || founder.own_tip_header != source.public_header_data().id()
        {
            return Err(refused());
        }
        let reclaimed = source.reclaimed_data();
        head.live_bytes
            .checked_sub(reclaimed.bytes)
            .ok_or_else(refused)?;
        head.live_objects
            .checked_sub(reclaimed.objects)
            .ok_or_else(refused)?;
        Ok(Self {
            context: head.context.clone(),
            completion_frame_id: source.completion_frame_id_data(),
            completed_transaction: source.transaction_data(),
            completed_plan: source.plan_id_data(),
            selected: source.selected_data(),
            public_header: source.public_header_data().id(),
            public_state: source.public_header_data().spec().state,
            archive: head.archive,
            directory_root,
            founder,
            token_root: root_data(head.tokens),
            publication_root: root_data(head.publications),
            generation: head.generation,
            selection_attempt: head.selection_attempt,
            queue_front: head.queue_front,
            queue_next: head.queue_next,
            live_bytes: head.live_bytes,
            live_objects: head.live_objects,
            reclaimed_bytes: reclaimed.bytes,
            reclaimed_objects: reclaimed.objects,
        })
    }
    fn require_source(
        &self,
        source: &CompletedV2TextRetirementSourceData<'_>,
    ) -> Result<(), AppError> {
        let h = source.head_data();
        let r = source.reclaimed_data();
        if self.context != h.context
            || self.completion_frame_id != source.completion_frame_id_data()
            || self.completed_transaction != source.transaction_data()
            || self.completed_plan != source.plan_id_data()
            || self.selected != source.selected_data()
            || self.public_header != source.public_header_data().id()
            || self.public_state != source.public_header_data().spec().state
            || self.archive != h.archive
            || Some(self.directory_root) != root_data(h.directory)
            || self.founder != source.founder_data()
            || self.token_root != root_data(h.tokens)
            || self.publication_root != root_data(h.publications)
            || self.generation != h.generation
            || self.selection_attempt != h.selection_attempt
            || self.queue_front != h.queue_front
            || self.queue_next != h.queue_next
            || self.live_bytes != h.live_bytes
            || self.live_objects != h.live_objects
            || self.reclaimed_bytes != r.bytes
            || self.reclaimed_objects != r.objects
            || h.pending != TextPendingData::None
        {
            return Err(refused());
        }
        Ok(())
    }
}
fn root_data(root: TextRootData) -> Option<NodeRefData> {
    match root {
        TextRootData::Empty => None,
        TextRootData::Present(root) => Some(root),
    }
}
