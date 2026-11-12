//! Closed SAME-verifier resident handoff. Historical WC is not permission: the
//! concrete fresh finish guard proves actual new source, exact WC and retired
//! lane/roots. All preparation/account/model work precedes that final proof.
//! Root caches its resident first; Physical IMMEDIATELY moves the whole remaining
//! Box<B> into outside disposal before any later check/return. No SDK is extracted.
use super::data::*;
use super::*;
use crate::store::critical_namespaces::current_layout::SavedCompletedV2TextFinishGuard;
use crate::store::owner_evidence::canonical::text_lane::V2TextRetirementRootsDrained;
use crate::store::persistence::PreparedV2TextResidentProjection;

// Independently named full NEW B2MiB preparation/finish metadata. Root domain's
// SAME model/domain payments remain separate, and nothing below borrows their
// unused credit. All new type layouts are included before any actual copy/move.
pub(in crate::store::persistence::v2_text::transaction::intent::raw::accepted) const COMPLETED_FINISH_METADATA_LAYOUT_BYTES:usize=
    PreparedV2UnsignedOriginalCustody::resident_handoff_metadata_layout_bytes()
    +crate::store::critical_namespaces::current_layout::V2_TEXT_COMPLETED_FINISH_LAYOUT_BYTES
    +4*std::mem::size_of::<super::resident_source::CompletedV2TextRetirementSourceData<'static>>()
    +4*std::mem::size_of::<Option<PreparedV2TextResidentProjection>>()
    +4*std::mem::size_of::<Option<Box<PreparedV2TextAcceptedCreate>>>()
    +4096;

impl SavedV2TextRetirementCompletion {
    /// Fixed cached DATA only, no hashing/account/codec after the final source proof.
    pub(in crate::store) fn completion_frame_id_data(&self) -> [u8; 32] {
        self.facts.completion_id
    }
    pub(in crate::store) fn selection_expectation_data(
        &self,
    ) -> catcoms_replication::chat::V2HostSelectionExpectation {
        self.facts.selected.selection_expectation_data()
    }
}
impl PreparedV2TextAcceptedCreate {
    /// Outside all writers. The SAME B2MiB pays the one bounded flat parent copy;
    /// same model/domain1MiB/16KiB and SDK2MiB remain independently paid. The Root
    /// preparation is cached before its late domain negative, so retries cannot
    /// allocate a new parent or reconstruct a verifier after partial success.
    pub(in crate::store) fn prepare_completed_resident_data(&mut self) -> Result<(), AppError> {
        self.metadata.validate_prepaid_account()?;
        // Finish this whole-B original check before a disjoint mutable Root loan.
        self.borrow_completed_retirement_source_data()?;
        let source = self.retirement_work.borrow_cached_completed_source_data()?;
        self.signature
            .intent
            .unsigned
            .prepare_completed_resident_data(&self.signature.intent.retained, &source)
    }
    /// Actual account/model/SDK/ownSaved association FIRST; only then the final
    /// genuine WC/newB/absence/source proof. No old S, candidateSome or obsolete
    /// genesis archive is revalidated after the actual Core/WC boundaries.
    pub(in crate::store) fn validate_completed_finish_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        saved: &SavedV2TextRetirementCompletion,
        drained: &V2TextRetirementRootsDrained,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        let own = self
            .retirement_work
            .saved_completion
            .as_ref()
            .ok_or_else(refused)?;
        if !std::ptr::eq(own.as_ref(), saved) {
            return Err(refused());
        }
        let source = self.borrow_completed_retirement_source_data()?;
        self.signature
            .intent
            .unsigned
            .validate_completed_resident_data(&self.signature.intent.retained, &source)?;
        self.validate_retirement_completion_under_current_guard(guard, drained)
    }
    /// Negative cached-output recovery only. After a late callback/Scope error no
    /// fresh positive guard is issued from the TransferredResident disposal. Root
    /// can recognize these exact owners first, dispose old debt outside writers,
    /// and retain the same resident. This does not authorize a second request.
    pub(in crate::store) fn require_finished_resident_outputs(
        input: &Option<Box<Self>>,
        resident: &PreparedV2TextResidentProjection,
        disposal: &Option<Box<Self>>,
    ) -> Result<(), AppError> {
        if input.is_some() {
            return Err(refused());
        }
        let old = disposal.as_ref().ok_or_else(refused)?;
        old.metadata.validate_prepaid_account()?;
        old.signature
            .intent
            .unsigned
            .require_transferred_resident_data(&old.observe_original(), resident)
    }
    /// Owning outside slots are supplied before the Current callback. Success and
    /// late-error retry preserve both the SAME resident and SAME complete disposal;
    /// they never move SDK/model fields independently or drop physical debt here.
    pub(in crate::store) fn finish_resident_into(
        guard: &SavedCompletedV2TextFinishGuard<'_, '_, '_>,
        input: &mut Option<Box<Self>>,
        resident: &mut Option<PreparedV2TextResidentProjection>,
        disposal: &mut Option<Box<Self>>,
    ) -> Result<(), AppError> {
        if input.is_none() {
            let old = disposal.as_ref().ok_or_else(refused)?;
            let resident = resident.as_ref().ok_or_else(refused)?;
            let original = old.observe_original();
            guard.require_original_accepted(&original)?;
            guard.require_original_completion(old.saved_retirement_completion()?.as_ref())?;
            old.signature
                .intent
                .unsigned
                .require_transferred_resident_data(&original, resident)?;
            return guard.validate_negative();
        }
        if resident.is_some() || disposal.is_some() {
            return Err(refused());
        }
        let original = input
            .as_ref()
            .expect("checked same whole B")
            .observe_original();
        guard.require_original_accepted(&original)?;
        guard.require_original_completion(
            input
                .as_ref()
                .expect("checked same B")
                .saved_retirement_completion()?
                .as_ref(),
        )?;
        guard.validate_negative()?;
        let transaction = input.as_mut().expect("checked same B ownership slot");
        // This Root operation has no late failure after its own cut: Err preserves
        // original ownership; Ok already cached resident+TransferredResident. There
        // is NO fallible operation, destructor, callback or allocation between a
        // successful return and the complete physical-disposal owning cache below.
        transaction
            .signature
            .intent
            .unsigned
            .transfer_completed_resident_under_guard(
                &transaction.signature.intent.retained,
                &original,
                guard,
                resident,
            )?;
        *disposal = Some(
            input
                .take()
                .expect("same whole B after successful coupled Root cut"),
        );
        // The outputs are now reviewably owned even if the generic Current postcheck
        // or caller callback subsequently loses the reply/cancels the original scope.
        guard.validate_negative()
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn require_transferred_resident_for_test(
        &self,
        resident: &PreparedV2TextResidentProjection,
    ) -> Result<(), AppError> {
        self.signature
            .intent
            .unsigned
            .require_transferred_resident_data(&self.observe_original(), resident)
    }
}
