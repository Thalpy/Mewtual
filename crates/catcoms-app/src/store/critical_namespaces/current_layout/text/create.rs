//! Actual Current wrapper for consuming an unused raw exclusion into ordinary ownership.
//!
//! This first shell remains negative-only. A token miss, selected/private-root lease,
//! complete workspace classifier, A/I/S/B finite work and signing each need their OWN
//! concrete physical issuer and distinct admission; conversion supplies none of them.
use super::*;
mod predecessor;
use predecessor::{CreatePredecessorRef, CREATE_PREDECESSOR_REF_LAYOUT_BYTES};
pub(in crate::store) use workspace::{
    AdmittedV2TextCompletedWorkspaceRead, BoundV2TextCompletedCreatePredecessor,
    PreparedV2TextCompletedWorkspaceReadCapture, V2TextCompletedWorkspaceSourceSlot,
    V2_TEXT_COMPLETED_WORKSPACE_ADMISSION_LAYOUT_BYTES,
};

pub(in crate::store) use intent::{
    AdmittedV2TextAcceptedCreateCommit, AdmittedV2TextAcceptedCreateStage,
    SavedAcceptedV2TextAdvanceGuard, SavedAcceptedV2TextFinalGuard,
    V2_TEXT_ACCEPTED_ADVANCE_GUARD_LAYOUT_BYTES, V2_TEXT_ACCEPTED_CREATE_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use intent::{
    AdmittedV2TextAcceptedRetirementCompletion, AdmittedV2TextAcceptedRetirementDeletion,
    V2_TEXT_RETIREMENT_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use intent::{
    AdmittedV2TextRawSignatureCommit, AdmittedV2TextRawSignatureStage,
    V2_TEXT_RAW_SIGNATURE_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use intent::{
    SavedCompletedV2TextFinishGuard, V2_TEXT_COMPLETED_FINISH_LAYOUT_BYTES,
};
mod intent;
pub(in crate::store) use intent::{
    AdmittedV2TextIntentCommit, AdmittedV2TextIntentStage, SavedUnsignedV2TextSigningGuard,
    V2_TEXT_INTENT_ADMISSION_LAYOUT_BYTES, V2_TEXT_SIGNING_GUARD_LAYOUT_BYTES,
};
pub(in crate::store) use retain::{
    AdmittedV2TextRetainRecoveryRead, AdmittedV2TextRetainRepair, AdmittedV2TextRetainScratch,
    PreparedV2TextRetainRecoveryCapture, PreparedV2TextRetainRepairLease,
    PreparedV2TextRetainScratchLease, V2TextRetainRecoverySourceSlot,
    V2_TEXT_RETAIN_RECOVERY_ADMISSION_LAYOUT_BYTES,
};
mod retain;
mod workspace;
use crate::store::owner_evidence::canonical::text_lane::{
    PreparedV2TextCreateLane, V2TextCreateLane, V2TextLane,
};
pub(in crate::store) use retain::{
    AdmittedV2TextRetainCommit, AdmittedV2TextRetainStage, V2_TEXT_RETAIN_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use workspace::{
    AdmittedV2TextWorkspaceRead, BoundV2TextCreatePredecessor, PreparedV2TextWorkspaceReadCapture,
    V2TextWorkspaceSourceSlot, V2_TEXT_WORKSPACE_ADMISSION_LAYOUT_BYTES,
};

impl V2TextCurrentGroupGuard<'_, '_> {
    /// Borrow outside owning slots through the actual Current callback. A late generic
    /// Scope refusal leaves the SAME converted caller cached outside every host writer.
    /// No paid-account wait follows the final real source/ID3/held-name proof.
    pub(in crate::store) fn convert_text_create_lane_into(
        &mut self,
        prepared: &mut PreparedV2TextCreateLane,
        caller: &mut Option<V2TextLane>,
        retained: &mut Option<V2TextCreateLane>,
    ) -> Result<(), AppError> {
        prepared.validate_prepaid_account(self.store())?;
        self.validate_original_namespace()?;
        self.owner
            .convert_text_create_lane_into(prepared, caller, retained)
    }
}
