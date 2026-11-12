//! Receiver-only immutable WC parent loan for the later REAL consuming finish.
//! This is not Current, a retired lane, quota permission or a model/source adopter.
//! Root must pay its exact flat-copy invoice BEFORE copying and freshly prove this
//! SAME actual WC+new roots+drained/retired lane before moving the original verifier.
use super::data::*;
use super::*;
use crate::store::persistence::v2_text::retirement::RetiredTotalsData;

pub(in crate::store) struct CompletedV2TextRetirementSourceData<'a> {
    saved: &'a Arc<SavedV2TextRetirementCompletion>,
    facts: &'a RetirementFacts,
}
impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn borrow_completed_retirement_source_data(
        &self,
    ) -> Result<CompletedV2TextRetirementSourceData<'_>, AppError> {
        let saved = self
            .retirement_work
            .saved_completion
            .as_ref()
            .ok_or_else(refused)?;
        let facts = self.retirement_work.facts.as_ref().ok_or_else(refused)?;
        facts
            .original
            .require_same_original(&self.observe_original())?;
        if !Arc::ptr_eq(facts, &saved.facts)
            || !Arc::ptr_eq(&facts.selected, self.saved.as_ref().ok_or_else(refused)?)
            || facts.completion.as_slice()
                != self.completion.as_ref().ok_or_else(refused)?.frame_data()
            || facts.deletion.as_slice() != self.deletion.as_ref().ok_or_else(refused)?.frame_data()
            || facts.selected.facts.selected.as_slice()
                != self.candidate.as_ref().ok_or_else(refused)?.frame_data()
            || facts.plan.as_slice() != self.plan.as_ref().ok_or_else(refused)?.as_slice()
        {
            return Err(refused());
        }
        // No codec, allocations, key, IO or weak upgrade. These parsed head/founder
        // values were derived from the exact original encrypted O_B/Directory under
        // their distinct B payment; fresh WC closure remains the consumer's duty.
        Ok(CompletedV2TextRetirementSourceData { saved, facts })
    }
}
impl CompletedV2TextRetirementSourceData<'_> {
    pub(in crate::store) fn saved_wc_data(&self) -> &Arc<SavedV2TextRetirementCompletion> {
        self.saved
    }
    pub(in crate::store) fn original_data(&self) -> &OriginalV2TextAcceptedCreateObservation {
        &self.facts.original
    }
    pub(in crate::store) fn head_data(&self) -> &TextHeadData {
        &self.facts.selected_head
    }
    pub(in crate::store) fn selected_data(&self) -> TextHeadIdData {
        self.facts.selected.head
    }
    pub(in crate::store) fn completion_frame_id_data(&self) -> [u8; 32] {
        self.facts.completion_id
    }
    pub(in crate::store) fn plan_id_data(&self) -> TextRetirementPlanIdData {
        self.facts.plan_id
    }
    pub(in crate::store) fn transaction_data(&self) -> TextTransactionIdData {
        self.facts.transaction
    }
    pub(in crate::store) fn public_header_data(&self) -> &SignedOwnerHeader {
        &self.facts.selected.facts.header
    }
    pub(in crate::store) fn founder_data(&self) -> DirectoryData {
        self.facts.founder
    }
    pub(in crate::store) fn reclaimed_data(&self) -> RetiredTotalsData {
        self.facts.reclaimed
    }
}

impl RetirementWorkState {
    /// Module-private disjoint-field loan used only AFTER the owning B's full
    /// original-frame association check. It avoids borrowing the whole B while
    /// mutating its own closed Root custody. No caller accepts paths/roots/flags.
    pub(super) fn borrow_cached_completed_source_data(
        &self,
    ) -> Result<CompletedV2TextRetirementSourceData<'_>, AppError> {
        let saved = self.saved_completion.as_ref().ok_or_else(refused)?;
        let facts = self.facts.as_ref().ok_or_else(refused)?;
        if !Arc::ptr_eq(facts, &saved.facts) {
            return Err(refused());
        }
        Ok(CompletedV2TextRetirementSourceData { saved, facts })
    }
}
