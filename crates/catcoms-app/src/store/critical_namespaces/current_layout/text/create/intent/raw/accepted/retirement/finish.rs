//! A real W_C completion retires the old lane. A DISTINCT fresh Current proof
//! then admits the owning resident handoff; selected B or historical WC alone
//! cannot move the verifier, release an exclusion, or authorize request two.
use super::*;
use crate::store::owner_evidence::canonical::text_lane::RetiredV2TextCreateLane;
use crate::store::persistence::v2_text::{
    OriginalV2RetainObservation, SavedV2TextRetirementCompletion, TextRetirementPlanIdData,
};
use crate::store::persistence::OriginalV2UnsignedPlanObservation;
use catcoms_replication::chat::V2HostSelectionExpectation;

/// No temporary PreparedB borrow survives in this guard. The outside genuine
/// SavedWC and removed-lane owners supply lifetime; Root may move only its SAME
/// already-advanced verifier after cached original/WC/model association checks.
pub(in crate::store) struct SavedCompletedV2TextFinishGuard<'cut, 'guard, 'writer> {
    current: &'cut V2TextCurrentGroupGuard<'guard, 'writer>,
    retired: &'cut RetiredV2TextCreateLane,
    saved: &'cut SavedV2TextRetirementCompletion,
    original: OriginalV2TextAcceptedCreateObservation,
    unsigned: OriginalV2UnsignedPlanObservation,
    retain: OriginalV2RetainObservation,
    plan: TextRetirementPlanIdData,
    basis: V2AdvancedAppendBasis,
    completion: [u8; 32],
    expectation: V2HostSelectionExpectation,
}

/// Full fixed guard/owning-retirement layout is included explicitly in the NEW
/// B2MiB metadata invoice. The model/domain and SDK retain their OWN payments.
pub(in crate::store) const V2_TEXT_COMPLETED_FINISH_LAYOUT_BYTES: usize =
    2 * std::mem::size_of::<SavedCompletedV2TextFinishGuard<'static, 'static, 'static>>()
        + crate::store::owner_evidence::canonical::text_lane::V2_TEXT_CREATE_RETIREMENT_LAYOUT_BYTES
        + 4 * 64;

impl V2TextCurrentGroupGuard<'_, '_> {
    /// Cache the actual unique lane, actual removed registry Arc and SAME drained
    /// roots OUTSIDE before the generic Current postcheck. There is no release on
    /// B visibility, ACK, caller cancellation or zero merely supplied by a caller.
    pub(in crate::store) fn retire_completed_text_lane_into(
        &mut self,
        transaction: &PreparedV2TextAcceptedCreate,
        basis: &V2AdvancedAppendBasis,
        lane: &mut Option<V2TextCreateLane>,
        drained: &mut Option<V2TextRetirementRootsDrained>,
        retired: &mut Option<RetiredV2TextCreateLane>,
    ) -> Result<(), AppError> {
        transaction
            .resources()
            .require_same_original(self.resources())?;
        let original = transaction.observe_original();
        let retain = transaction.original_retain();
        // This getter may validate a prepaid account/decode immutable Plan DATA,
        // so obtain it BEFORE the actual final WC/source proof, never afterward.
        let plan = transaction.retirement_plan_id_data()?;
        basis.require_original_accepted(&original)?;
        basis.validate_negative()?;
        if let Some(previous) = retired.as_ref() {
            if lane.is_some() || drained.is_some() {
                return Err(refused());
            }
            previous.require_original(&original, &retain, &plan, basis)?;
            self.owner.require_retired_text_create_lane(previous)?;
            transaction.validate_retirement_completion_under_current_guard(
                self,
                previous.drained_roots(),
            )?;
        } else {
            let caller = lane.as_ref().ok_or_else(refused)?;
            let roots = drained.as_ref().ok_or_else(refused)?;
            caller.require_workspace_idle(&mut self.owner, &retain)?;
            self.owner
                .require_text_retirement_roots_drained(caller, roots)?;
            self.owner.require_text_control_io_quiescent()?;
            self.require_text_control_write_idle_for_query()?;
            transaction.validate_retirement_completion_under_current_guard(self, roots)?;
        }
        // Cached actual identities/source/cap/Ends only after final physical proof.
        // The canonical removal itself repeats these same native negatives before
        // its first store/take; no fallible action follows until outside cache exists.
        basis.validate_negative()?;
        self.validate_source_only()?;
        self.owner.retire_completed_text_create_lane_into(
            lane, &original, &retain, &plan, basis, drained, retired,
        )?;
        basis.validate_negative()?;
        self.validate_source_only()
    }
}

impl<'guard, 'writer> V2TextCurrentGroupGuard<'guard, 'writer> {
    /// The Prepared loan is intentionally NOT `'cut`. Clone SavedWC into an
    /// outside Arc first, then end this temporary validator borrow before moving
    /// Root's private resident owner and the whole original B into outside slots.
    pub(in crate::store) fn admit_completed_finish<'cut>(
        &'cut mut self,
        retired: &'cut RetiredV2TextCreateLane,
        saved: &'cut SavedV2TextRetirementCompletion,
        prepared: &PreparedV2TextAcceptedCreate,
    ) -> Result<SavedCompletedV2TextFinishGuard<'cut, 'guard, 'writer>, AppError> {
        prepared
            .resources()
            .require_same_original(self.resources())?;
        let original = prepared.observe_original();
        let unsigned = prepared.original_unsigned();
        let retain = prepared.original_retain();
        let plan = saved.retirement_plan_id_data();
        let basis = prepared.accepted_final_basis_data()?.clone();
        let completion = saved.completion_frame_id_data();
        let expectation = saved.selection_expectation_data();
        retired.require_original(&original, &retain, &plan, &basis)?;
        unsigned.validate_negative()?;
        basis.validate_negative()?;
        self.owner.require_retired_text_create_lane(retired)?;
        // Concrete account/model/SDK/ownSaved association FIRST, then FINAL exact
        // WC/newB/public/private closure. No old I/S/TA may be required after WD.
        prepared.validate_completed_finish_under_current_guard(
            self,
            saved,
            retired.drained_roots(),
        )?;
        retired.require_original(&original, &retain, &plan, &basis)?;
        self.owner.require_retired_text_create_lane(retired)?;
        unsigned.validate_negative()?;
        basis.validate_negative()?;
        self.validate_source_only()?;
        Ok(SavedCompletedV2TextFinishGuard {
            current: self,
            retired,
            saved,
            original,
            unsigned,
            retain,
            plan,
            basis,
            completion,
            expectation,
        })
    }
}

impl SavedCompletedV2TextFinishGuard<'_, '_, '_> {
    pub(in crate::store) fn require_original_accepted(
        &self,
        original: &OriginalV2TextAcceptedCreateObservation,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)
    }
    pub(in crate::store) fn require_original_unsigned(
        &self,
        original: &OriginalV2UnsignedPlanObservation,
    ) -> Result<(), AppError> {
        self.unsigned.require_same_original(original)
    }
    pub(in crate::store) fn require_original_retain(
        &self,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.retain.require_same_original(original)
    }
    pub(in crate::store) fn require_completion_frame_id_data(
        &self,
        id: &[u8; 32],
    ) -> Result<(), AppError> {
        if self.completion != *id {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn require_original_completion(
        &self,
        saved: &SavedV2TextRetirementCompletion,
    ) -> Result<(), AppError> {
        if !std::ptr::eq(self.saved, saved) {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn expectation_data(&self) -> &V2HostSelectionExpectation {
        &self.expectation
    }
    /// Fixed cached checks only. Called around Root's infallible owning cut; never
    /// performs account waits, opens/decodes, Core replay or SDK materialization.
    pub(in crate::store) fn validate_negative(&self) -> Result<(), AppError> {
        self.unsigned.validate_negative()?;
        self.basis.validate_negative()?;
        self.current.validate_source_only()?;
        self.retired
            .require_original(&self.original, &self.retain, &self.plan, &self.basis)?;
        self.current
            .owner
            .require_retired_text_create_lane(self.retired)?;
        self.current.require_text_control_write_idle_for_query()?;
        self.current.validate_source_only()?;
        self.unsigned.validate_negative()
    }
}
impl std::fmt::Debug for SavedCompletedV2TextFinishGuard<'_, '_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedCompletedV2TextFinishGuard")
            .finish_non_exhaustive()
    }
}
