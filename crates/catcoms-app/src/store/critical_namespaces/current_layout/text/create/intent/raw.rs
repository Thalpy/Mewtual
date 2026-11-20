//! Distinct actual S permissions. A raw signature must ALREADY belong to the SAME
//! original SDK plan; these finite Stage/Commit owners cannot invoke a key or accept
//! caller signature bytes. Complete W_S precedes signature/head-stage effects.
use super::*;
mod completed;
pub(in crate::store) use accepted::{
    AdmittedV2TextAcceptedRetirementCompletion, AdmittedV2TextAcceptedRetirementDeletion,
    V2_TEXT_RETIREMENT_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use accepted::{
    SavedCompletedV2TextFinishGuard, V2_TEXT_COMPLETED_FINISH_LAYOUT_BYTES,
};
mod accepted;
use crate::store::persistence::v2_text::{
    OriginalV2TextRawSignatureObservation, PreparedV2TextRawSignature,
    ReadyV2TextRawSignatureSelection,
};
pub(in crate::store) use accepted::{
    AdmittedV2TextAcceptedCreateCommit, AdmittedV2TextAcceptedCreateStage,
    SavedAcceptedV2TextAdvanceGuard, SavedAcceptedV2TextFinalGuard,
    V2_TEXT_ACCEPTED_ADVANCE_GUARD_LAYOUT_BYTES, V2_TEXT_ACCEPTED_CREATE_ADMISSION_LAYOUT_BYTES,
};

pub(in crate::store) struct AdmittedV2TextRawSignatureStage {
    original: OriginalV2TextRawSignatureObservation,
    intent: OriginalV2TextUnsignedIntentObservation,
    unsigned: OriginalV2UnsignedPlanObservation,
    retain: OriginalV2RetainObservation,
    physical: PreparedV2TextPhysicalLease,
    // LAST: all worker-local Files/key cleanup and SAME OS lease precede the actual
    // finite gate/cap release. This does not close or retire original pending debt.
    end: V2TextCreateControlWriteEnd,
}

pub(in crate::store) struct AdmittedV2TextRawSignatureCommit {
    original: OriginalV2TextRawSignatureObservation,
    intent: OriginalV2TextUnsignedIntentObservation,
    unsigned: OriginalV2UnsignedPlanObservation,
    retain: OriginalV2RetainObservation,
    physical: PreparedV2TextPhysicalLease,
    end: V2TextCreateControlWriteEnd,
}

/// Included in SAME actual A/I/S metadata invoice before any S allocation. Existing
/// model1MiB, SDK2MiB and active16MiB work are independently real retained payments.
pub(in crate::store) const V2_TEXT_RAW_SIGNATURE_ADMISSION_LAYOUT_BYTES: usize = 2
    * std::mem::size_of::<PreparedV2TextPhysicalLease>()
    + std::mem::size_of::<AdmittedV2TextRawSignatureStage>()
    + std::mem::size_of::<AdmittedV2TextRawSignatureCommit>()
    + 4 * 64
    + CREATE_PREDECESSOR_REF_LAYOUT_BYTES;

macro_rules! signature_association {
    ($ty:ident) => {
        impl $ty {
            /// Pure exact-original check for already admitted finite IO. A late Stop
            /// does not erase the original output or require a replacement signature.
            pub(in crate::store) fn require_same_signature(
                &self,
                original: &OriginalV2TextRawSignatureObservation,
            ) -> Result<(), AppError> {
                self.original.require_same_original(original)
            }
        }
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    };
}
signature_association!(AdmittedV2TextRawSignatureStage);
signature_association!(AdmittedV2TextRawSignatureCommit);

macro_rules! signature_admission {
    ($method:ident, $issuer:ident, $marker:ident, $validate:ident) => {
        impl V2TextCurrentGroupGuard<'_, '_> {
            /// Borrow the closed actual issuer and outside owning slots. Typed
            /// physical proof authenticates the original I/raw/preimage/Source;
            /// parsed signature records and a historical SavedI cannot mint this.
            fn $method(
                &mut self,
                lane: &V2TextCreateLane,
                predecessor: CreatePredecessorRef<'_>,
                issuer: &$issuer,
                physical: &mut Option<PreparedV2TextPhysicalLease>,
                admitted: &mut Option<$marker>,
            ) -> Result<(), AppError> {
                let original = issuer.observe_original();
                let intent = issuer.original_intent();
                let unsigned = issuer.original_unsigned();
                let retain = issuer.original_retain();
                if let Some(existing) = admitted.as_ref() {
                    existing.require_same_signature(&original)?;
                    existing.intent.require_same_original(&intent)?;
                    existing.unsigned.require_same_original(&unsigned)?;
                    existing.retain.require_same_original(&retain)?;
                    existing.physical.require_signature_write_negative(self)?;
                    existing.end.require_same_lane(lane)?;
                    predecessor.require_same_lane_original(lane, &retain)?;
                    unsigned.validate_negative()?;
                    return self.validate_source_only();
                }
                let lease = physical.as_ref().ok_or_else(refused)?;
                // Real account waits finish before complete actual physical proof.
                // The preacquired lease alone grants neither an S role nor a key cut.
                lease.validate_signature_write_under_guard(self)?;
                issuer.resources().require_same_original(self.resources())?;
                predecessor.require_same_lane_original(lane, &retain)?;
                lane.require_workspace_idle(&mut self.owner, &retain)?;
                self.owner.require_text_control_io_quiescent()?;
                self.require_text_control_write_idle_for_query()?;
                issuer.$validate(self)?;
                // Only cached original/slot/negative checks remain after full source
                // proof. Reserve real writer cap/gate on this SAME canonical edge.
                predecessor.require_same_lane_original(lane, &retain)?;
                lane.require_workspace_idle(&mut self.owner, &retain)?;
                lease.require_signature_write_negative(self)?;
                self.owner.require_text_control_io_quiescent()?;
                self.validate_source_only()?;
                unsigned.validate_negative()?;
                let end = lane.reserve_create_control_write_end(&mut self.owner, &retain)?;
                *admitted = Some($marker {
                    original,
                    intent,
                    unsigned,
                    retain,
                    physical: physical.take().expect("same outside S lease checked"),
                    end,
                });
                Ok(())
            }
        }
    };
}
signature_admission!(
    admit_text_raw_signature_stage_with_predecessor_into,
    PreparedV2TextRawSignature,
    AdmittedV2TextRawSignatureStage,
    validate_stage_under_current_guard
);
signature_admission!(
    admit_text_raw_signature_commit_with_predecessor_into,
    ReadyV2TextRawSignatureSelection,
    AdmittedV2TextRawSignatureCommit,
    validate_ready_under_current_guard
);
