//! Own-only Root advance forwarding. No Header/model/SDK or borrowed owning slot
//! can be supplied or extracted. Core advance and its old-math disposal remain
//! separate from physical B selection and later WD/WC retirement.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    SavedAcceptedV2TextAdvanceGuard, SavedAcceptedV2TextFinalGuard,
};
use crate::store::persistence::V2AdvancedAppendBasis;
#[cfg(all(test, windows))]
mod hooks;
impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn validate_advance_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        saved: &SavedV2AcceptedCreate,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        self.metadata.validate_prepaid_account()?;
        saved
            .original
            .require_same_original(&self.observe_original())?;
        let original = self.observe_original();
        self.signature
            .intent
            .unsigned
            .validate_accepted_advance_data(&self.signature.intent.retained, &original)?;
        let journal = self.capture_pre_retirement_journal(guard)?;
        // FINAL actual NEW B proof, then SAME-held journal/name tail only. B advance
        // is not permitted through corrupt/foreign MAIN or a later dirty stage.
        saved.validate_selected_under_current_guard(guard)?;
        journal.require_unchanged()?;
        guard.validate_source_only()
    }
    pub(in crate::store) fn validate_final_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        saved: &SavedV2AcceptedCreate,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        self.metadata.validate_prepaid_account()?;
        saved
            .original
            .require_same_original(&self.observe_original())?;
        self.signature.intent.unsigned.require_finalization_ready(
            &self.signature.intent.retained,
            &self.observe_original(),
        )?;
        let journal = self.capture_pre_retirement_journal(guard)?;
        saved.validate_selected_under_current_guard(guard)?;
        journal.require_unchanged()?;
        guard.validate_source_only()
    }
    fn capture_pre_retirement_journal<'g>(
        &'g self,
        guard: &'g V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<named::AcceptedNamedFiles<'g>, AppError> {
        let saved = self.saved.as_ref().ok_or_else(refused)?;
        let facts = &saved.facts;
        let ready = self.ready.as_ref().ok_or_else(refused)?;
        let base = &self.parent.signature.parent.base;
        let mut held = named::AcceptedNamedFiles::capture(
            guard,
            &facts.target,
            facts.target_id,
            &facts.pages,
            facts.pages_id,
        )?;
        held.add_exact_file(
            owned_path(&base.wal_path)?,
            ready.wal_native,
            self.wal.as_ref().ok_or_else(refused)?.frame_data(),
        )?;
        for name in ["text.wal.stage.v1", "head.stage", "text.seed.stage.v1"] {
            held.add_absence(fixed_child(&facts.target, name)?)?;
        }
        Ok(held)
    }
    pub(in crate::store) fn advance_accepted_under_guard(
        &mut self,
        guard: &SavedAcceptedV2TextAdvanceGuard<'_, '_, '_>,
    ) -> Result<(), AppError> {
        let original = self.observe_original();
        self.signature.intent.unsigned.advance_accepted_under_guard(
            &self.signature.intent.retained,
            &original,
            guard,
        )
    }
    /// Negative cleanup only; legal after Scope stop. The actual old math drains
    /// outside all writers before a separate fresh final Current proof can succeed.
    pub(in crate::store) fn drain_accepted_retired_math_if_present(
        &mut self,
    ) -> Result<(), AppError> {
        let original = self.observe_original();
        self.signature
            .intent
            .unsigned
            .drain_accepted_retired_math_if_present(&original)
    }
    pub(in crate::store) fn confirm_accepted_final_under_guard(
        &mut self,
        guard: &SavedAcceptedV2TextFinalGuard<'_, '_, '_>,
    ) -> Result<(), AppError> {
        let original = self.observe_original();
        self.signature
            .intent
            .unsigned
            .confirm_accepted_final_under_guard(&self.signature.intent.retained, &original, guard)
    }
    pub(in crate::store) fn accepted_final_basis_data(
        &self,
    ) -> Result<&V2AdvancedAppendBasis, AppError> {
        self.signature
            .intent
            .unsigned
            .accepted_final_basis_data(&self.observe_original())
    }
}
