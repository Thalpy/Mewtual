//! Exact original-B retirement inputs and negative reader disposal. These DATA
//! loans are not root-close/drain, WD, unlink or WC permission; distinct real
//! Current issuers and their finite physical workers will supply those boundaries.
use super::*;
use crate::store::persistence::V2AdvancedAppendBasis;
impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn retirement_roots_data(&self) -> Result<[TextHeadIdData; 4], AppError> {
        self.metadata.validate_prepaid_account()?;
        let plan = RetirementPlanData::decode_data(self.plan.as_ref().ok_or_else(refused)?)?;
        plan.carry_data()
            .roots_data()
            .try_into()
            .map_err(|_| refused())
    }
    pub(in crate::store) fn retirement_plan_id_data(
        &self,
    ) -> Result<TextRetirementPlanIdData, AppError> {
        self.metadata.validate_prepaid_account()?;
        RetirementPlanData::decode_data(self.plan.as_ref().ok_or_else(refused)?)?.id_data()
    }
    pub(in crate::store) fn validate_retirement_close_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        basis: &V2AdvancedAppendBasis,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        basis.require_original_accepted(&self.observe_original())?;
        basis.require_same_original(self.accepted_final_basis_data()?)?;
        // Concrete actual advanced-final and retired-math-none predicates precede
        // the final fresh NEW B +FULLownWB/stage fence. No old query is revalidated.
        self.validate_final_under_current_guard(guard, self.saved.as_ref().ok_or_else(refused)?)
    }
    /// Actual logical Ends, never count==N arithmetic. The original B and completed
    /// actual AdvancedFinal DATA are checked first, but this negative cleanup needs
    /// no live Scope/source so Stop cannot strand its old-root readers forever.
    /// Registry close/join remains a separate real canonical/End proof.
    pub(in crate::store) fn drain_original_query_readers_outside_writers(
        &mut self,
    ) -> Result<(), AppError> {
        let original = self.observe_original();
        self.saved
            .as_ref()
            .ok_or_else(refused)?
            .original
            .require_same_original(&original)?;
        self.accepted_final_basis_data()?
            .require_original_accepted(&original)?;
        self.signature
            .intent
            .retained
            .query
            .drain_completed_reader_outside_writers();
        self.signature
            .intent
            .query
            .drain_completed_reader_outside_writers();
        Ok(())
    }
}

#[cfg(all(test, windows))]
impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn original_query_readers_live_for_test(&self) -> (bool, bool) {
        (
            self.signature
                .intent
                .retained
                .query
                .completed_reader_live_for_test(),
            self.signature.intent.query.completed_reader_live_for_test(),
        )
    }
}

mod continuation;
mod data;
mod work;
pub(in crate::store) use data::{
    ReadyV2TextRetirementCompletion, SavedV2TextRetirementCompletion, SavedV2TextRetirementDeletion,
};
pub(super) use data::{
    RetirementWorkState, RETIREMENT_METADATA_LAYOUT_BYTES, RETIREMENT_WORK_LAYOUT_BYTES,
};
#[cfg(all(test, windows))]
pub(in crate::store) use work::V2TextRetirementTestCut;

mod finish;
mod resident_source;
pub(super) use finish::COMPLETED_FINISH_METADATA_LAYOUT_BYTES;
pub(in crate::store) use resident_source::CompletedV2TextRetirementSourceData;
