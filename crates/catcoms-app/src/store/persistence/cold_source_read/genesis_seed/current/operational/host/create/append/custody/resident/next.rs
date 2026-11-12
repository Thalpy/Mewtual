//! Nonextracting completed-parent association for the DISTINCT next request.
//!
//! These loans/observations are DATA and cached negatives, never an index hit,
//! SavedA, completed workspace or current source permission. Concrete Physical
//! and Factory consumers prove the full actual WC/new closure independently.
use super::*;
use crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard;
use std::sync::Weak;

/// Tiny same-residence identity. A reused verifier has the SAME Box identity but
/// a different selected O/WC after the next successful transaction; an old
/// observation cannot silently bind that later residence. No read, B, model or
/// SDK owner is retained. Domain's already-paid original negative is shared.
#[derive(Clone)]
pub(in crate::store) struct OriginalV2TextResidentObservation {
    identity: Weak<()>,
    selected: TextHeadIdData,
    completion: [u8; 32],
    // LAST: the original tiny identity/control tails retain domain funding.
    domain: V2TextProjectionNegative,
}
impl OriginalV2TextResidentObservation {
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        if !self.identity.ptr_eq(&other.identity)
            || self.selected != other.selected
            || self.completion != other.completion
        {
            return Err(refused());
        }
        self.domain.require_same_original(&other.domain)
    }
    pub(in crate::store) fn validate_negative(&self) -> Result<(), AppError> {
        // A Host may retain its separate domain close control after the model
        // owner is destroyed. That alone cannot keep this residence live. This
        // weak count is a refusal filter, never an owning upgrade or permission.
        if self.identity.strong_count() == 0 {
            return Err(refused());
        }
        self.domain.validate_negative()
    }
    pub(super) fn require_owner(&self, owner: &ResidentVerifierOwner) -> Result<(), AppError> {
        if !self.identity.ptr_eq(&Arc::downgrade(&owner.identity)) {
            return Err(refused());
        }
        self.domain.require_same_original(&owner.domain.negative())
    }
}

/// Receiver-only immutable parent loan. Its constructor is private and no model,
/// source owner, Saved receipt or SDK is reachable. Physical still authenticates
/// the actual selected pages and WC before issuing a completed-workspace role.
pub(in crate::store) struct V2TextResidentParentData<'a> {
    parent: &'a ResidentSelectedParent,
}
impl V2TextResidentParentData<'_> {
    pub(in crate::store) fn context_data(&self) -> &TextContextData {
        &self.parent.context
    }
    pub(in crate::store) fn selected_data(&self) -> TextHeadIdData {
        self.parent.selected
    }
    pub(in crate::store) fn public_header_data(&self) -> OwnerHeaderId {
        self.parent.public_header
    }
    pub(in crate::store) fn public_state_data(&self) -> OwnerStateCommitment {
        self.parent.public_state
    }
    pub(in crate::store) fn archive_data(&self) -> ArchiveHeadId {
        self.parent.archive
    }
    pub(in crate::store) fn directory_root_data(&self) -> NodeRefData {
        self.parent.directory_root
    }
    pub(in crate::store) fn founder_data(&self) -> DirectoryData {
        self.parent.founder
    }
    pub(in crate::store) fn token_root_data(&self) -> Option<NodeRefData> {
        self.parent.token_root
    }
    pub(in crate::store) fn publication_root_data(&self) -> Option<NodeRefData> {
        self.parent.publication_root
    }
    pub(in crate::store) fn generation_data(&self) -> u64 {
        self.parent.generation
    }
    pub(in crate::store) fn selection_attempt_data(&self) -> u64 {
        self.parent.selection_attempt
    }
    pub(in crate::store) fn queue_front_data(&self) -> u64 {
        self.parent.queue_front
    }
    pub(in crate::store) fn queue_next_data(&self) -> u64 {
        self.parent.queue_next
    }
    pub(in crate::store) fn completion_frame_id_data(&self) -> [u8; 32] {
        self.parent.completion_frame_id
    }
    pub(in crate::store) fn transaction_data(&self) -> TextTransactionIdData {
        self.parent.completed_transaction
    }
    pub(in crate::store) fn plan_id_data(&self) -> TextRetirementPlanIdData {
        self.parent.completed_plan
    }
    /// RAW selected O_B counters, before the one exact WC discount below.
    pub(in crate::store) fn baseline_data(&self) -> (u128, u128) {
        (self.parent.live_bytes, self.parent.live_objects)
    }
    /// Exact matching completed Plan totals. Neither getter applies another refund.
    pub(in crate::store) fn reclaimed_data(&self) -> (u128, u128) {
        (self.parent.reclaimed_bytes, self.parent.reclaimed_objects)
    }
}
impl PreparedV2TextResidentProjection {
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextResidentObservation {
        OriginalV2TextResidentObservation {
            identity: Arc::downgrade(&self.owner.identity),
            domain: self.owner.domain.negative(),
            selected: self.selected.selected,
            completion: self.selected.completion_frame_id,
        }
    }
    pub(in crate::store) fn parent_data(&self) -> V2TextResidentParentData<'_> {
        V2TextResidentParentData {
            parent: &self.selected,
        }
    }
    /// Account and same model/flat-parent checks BEFORE any final physical proof.
    /// The concrete authenticated source validator must still prove WC/new roots.
    pub(in crate::store) fn validate_completed_parent_data(
        &self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        self.validate_negative(store)?;
        let header = self.owner.model.replay_data().header();
        if header.id() != self.selected.public_header
            || header.spec().state != self.selected.public_state
        {
            return Err(refused());
        }
        self.owner.domain.validate_negative()
    }
    /// Cached original Source/domain/Scope negatives only, suitable around the
    /// FINAL typed workspace proof. No allocation/account wait/IO/model work.
    pub(in crate::store) fn require_current_source_under_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.owner
            .domain
            .require_current_source(guard.scope_admission())?;
        self.owner.domain.validate_negative()
    }

    /// Historical identity comparison only. Selected O/WC must differ after a
    /// later successful transaction even though the SAME original model remains.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn require_same_model_original_for_test(
        &self,
        original: &OriginalV2TextResidentObservation,
    ) -> Result<(), AppError> {
        original.require_owner(&self.owner)
    }
}

// Explicitly additional to the existing domain/custody layout invoices. Tiny
// observations/borrowed views are not hidden in a read or old B allowance.
pub(super) const RESIDENT_NEXT_METADATA_LAYOUT_BYTES: usize =
    8 * std::mem::size_of::<OriginalV2TextResidentObservation>()
        + 4 * std::mem::size_of::<V2TextResidentParentData<'static>>()
        + 4 * 64;
