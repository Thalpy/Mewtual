//! Actual B/final-basis admission to old-root close and metadata detachment. These
//! steps issue neither WD nor WC and never infer drain from registry absence.
use super::*;
mod finish;
pub(in crate::store) use finish::{
    SavedCompletedV2TextFinishGuard, V2_TEXT_COMPLETED_FINISH_LAYOUT_BYTES,
};
mod effects;
use crate::store::owner_evidence::canonical::text_lane::{
    PreparedV2TextRetirementRootsClose, RemovedV2TextRetirementRootGenerations,
    V2TextRetirementRootsClosed, V2TextRetirementRootsDrained,
};
use crate::store::persistence::V2AdvancedAppendBasis;
pub(in crate::store) use effects::{
    AdmittedV2TextAcceptedRetirementCompletion, AdmittedV2TextAcceptedRetirementDeletion,
    V2_TEXT_RETIREMENT_ADMISSION_LAYOUT_BYTES,
};

impl V2TextCurrentGroupGuard<'_, '_> {
    /// Whole outside slots survive errors. The concrete validator proves the NEW B
    /// closure, FULL own WB and actual advanced-final basis BEFORE the canonical
    /// 32-slot close; ScopeStop after the cut leaves the real Closed owner cached.
    pub(in crate::store) fn close_text_retirement_roots_into(
        &mut self,
        lane: &V2TextCreateLane,
        transaction: &PreparedV2TextAcceptedCreate,
        basis: &V2AdvancedAppendBasis,
        prepared: &mut Option<PreparedV2TextRetirementRootsClose>,
        closed: &mut Option<V2TextRetirementRootsClosed>,
    ) -> Result<(), AppError> {
        transaction
            .resources()
            .require_same_original(self.resources())?;
        let original = transaction.observe_original();
        let retain = transaction.original_retain();
        if let Some(input) = prepared.as_ref() {
            input.require_original(&original, basis)?;
        }
        if let Some(existing) = closed.as_ref() {
            existing.require_original(&original, basis)?;
        }
        if prepared.is_some() && closed.is_some() {
            return Err(refused());
        }
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        self.require_text_control_write_idle_for_query()?;
        transaction.validate_retirement_close_under_current_guard(self, basis)?;
        // Only fixed original/native/negative checks and the preallocated canonical
        // scan/move follow the final physical proof. No account, decoder or new File.
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        self.validate_source_only()?;
        self.owner
            .close_text_retirement_roots_into(lane, prepared, closed)?;
        self.validate_source_only()
    }

    /// Fresh complete B/WB/basis proof precedes exact closed-Arc removal. The REAL
    /// removed metadata owner is transferred outside before the late source check;
    /// no destructor or fee refund runs under this guard, even after a lost return.
    pub(in crate::store) fn detach_text_retirement_roots_into(
        &mut self,
        lane: &V2TextCreateLane,
        transaction: &PreparedV2TextAcceptedCreate,
        basis: &V2AdvancedAppendBasis,
        drained: &mut V2TextRetirementRootsDrained,
        removed: &mut Option<RemovedV2TextRetirementRootGenerations>,
    ) -> Result<(), AppError> {
        transaction
            .resources()
            .require_same_original(self.resources())?;
        let original = transaction.observe_original();
        let retain = transaction.original_retain();
        let plan = drained.retirement_plan_id_data();
        drained.require_original(&original, &plan)?;
        drained.require_basis(basis)?;
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        self.require_text_control_write_idle_for_query()?;
        transaction.validate_retirement_close_under_current_guard(self, basis)?;
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        self.validate_source_only()?;
        self.owner
            .detach_text_retirement_roots_into(lane, drained, removed)?;
        self.validate_source_only()
    }
}
