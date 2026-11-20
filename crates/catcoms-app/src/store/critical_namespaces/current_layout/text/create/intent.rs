//! Distinct actual Current permissions for unsigned-I staging and selector commit.
//!
//! Only the concrete owning issuer can prove its SAME original A, native accepted
//! lookup and opaque SDK plan. Neither a DATA Absent, historical SavedA/SavedI, unsigned
//! packet, Bound predecessor nor physical lease can construct these private markers.
//! I stages/selects an unsigned intent; it supplies NO raw signing permission.
use super::*;
mod completed;
pub(in crate::store) use raw::{
    AdmittedV2TextAcceptedCreateCommit, AdmittedV2TextAcceptedCreateStage,
    SavedAcceptedV2TextAdvanceGuard, SavedAcceptedV2TextFinalGuard,
    V2_TEXT_ACCEPTED_ADVANCE_GUARD_LAYOUT_BYTES, V2_TEXT_ACCEPTED_CREATE_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use raw::{
    AdmittedV2TextAcceptedRetirementCompletion, AdmittedV2TextAcceptedRetirementDeletion,
    V2_TEXT_RETIREMENT_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use raw::{
    SavedCompletedV2TextFinishGuard, V2_TEXT_COMPLETED_FINISH_LAYOUT_BYTES,
};
mod raw;
pub(in crate::store) use raw::{
    AdmittedV2TextRawSignatureCommit, AdmittedV2TextRawSignatureStage,
    V2_TEXT_RAW_SIGNATURE_ADMISSION_LAYOUT_BYTES,
};
mod signing;
use crate::store::owner_evidence::canonical::text_lane::V2TextCreateControlWriteEnd;
use crate::store::persistence::v2_text::{
    OriginalV2RetainObservation, OriginalV2TextUnsignedIntentObservation,
    PreparedV2TextUnsignedIntent, ReadyV2TextUnsignedIntentSelection,
};
use crate::store::persistence::OriginalV2UnsignedPlanObservation;
pub(in crate::store) use signing::{
    SavedUnsignedV2TextSigningGuard, V2_TEXT_SIGNING_GUARD_LAYOUT_BYTES,
};

/// Exact immutable-I/WI staging only. A complete durable WORK envelope must precede
/// every named intent/output/head.stage effect in the concrete finite runner.
pub(in crate::store) struct AdmittedV2TextIntentStage {
    original: OriginalV2TextUnsignedIntentObservation,
    unsigned: OriginalV2UnsignedPlanObservation,
    retain: OriginalV2RetainObservation,
    physical: PreparedV2TextPhysicalLease,
    // LAST: helper-local Files/key/syscall cleanup and this same physical lease drain
    // before the actual finite writer gate/cap. Drop does not retire pending custody.
    end: V2TextCreateControlWriteEnd,
}

/// Fresh Current permission to select this exact staged I over its original A parent.
/// Stage and Commit are intentionally separate. The finite commit rechecks the FULL
/// original WI/intent/output bodies outside writers before CAS and actual barriers.
pub(in crate::store) struct AdmittedV2TextIntentCommit {
    original: OriginalV2TextUnsignedIntentObservation,
    unsigned: OriginalV2UnsignedPlanObservation,
    retain: OriginalV2RetainObservation,
    physical: PreparedV2TextPhysicalLease,
    end: V2TextCreateControlWriteEnd,
}

/// Included in the concrete unsigned-I metadata invoice BEFORE allocation. Actual
/// Root model/SDK, original source, query and16MiB work credits remain ADDITIONAL.
pub(in crate::store) const V2_TEXT_INTENT_ADMISSION_LAYOUT_BYTES: usize =
    2 * std::mem::size_of::<PreparedV2TextPhysicalLease>()
        + std::mem::size_of::<AdmittedV2TextIntentStage>()
        + std::mem::size_of::<AdmittedV2TextIntentCommit>()
        + 4 * 64
        + CREATE_PREDECESSOR_REF_LAYOUT_BYTES;

macro_rules! intent_association {
    ($ty:ident) => {
        impl $ty {
            /// Already admitted finite work checks ONLY its exact original issuer.
            /// It may finish after Stop without renewing Current or authorizing sign.
            pub(in crate::store) fn require_same_intent(
                &self,
                original: &OriginalV2TextUnsignedIntentObservation,
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
intent_association!(AdmittedV2TextIntentStage);
intent_association!(AdmittedV2TextIntentCommit);

macro_rules! intent_admission {
    ($method:ident, $issuer:ident, $marker:ident, $validate:ident) => {
        impl V2TextCurrentGroupGuard<'_, '_> {
            /// Borrow outside owning slots through actual Current. Once a marker is
            /// cached, a late Scope refusal cannot discard/renew the admitted work or
            /// drop its physical custody under AD/canonical/Root writers.
            fn $method(
                &mut self,
                lane: &V2TextCreateLane,
                predecessor: CreatePredecessorRef<'_>,
                issuer: &$issuer,
                physical: &mut Option<PreparedV2TextPhysicalLease>,
                admitted: &mut Option<$marker>,
            ) -> Result<(), AppError> {
                let original = issuer.observe_original();
                let unsigned = issuer.original_unsigned();
                let retain = issuer.original_retain();
                if let Some(existing) = admitted.as_ref() {
                    existing.require_same_intent(&original)?;
                    existing.unsigned.require_same_original(&unsigned)?;
                    existing.retain.require_same_original(&retain)?;
                    existing.physical.require_intent_write_negative(self)?;
                    existing.end.require_same_lane(lane)?;
                    predecessor.require_same_lane_original(lane, &retain)?;
                    return self.validate_source_only();
                }
                let lease = physical.as_ref().ok_or_else(refused)?;
                // All account waits precede complete physical/source/accepted-query
                // proof. This lease check supplies no role or signing authority.
                lease.validate_intent_write_under_guard(self)?;
                issuer.resources().require_same_original(self.resources())?;
                predecessor.require_same_lane_original(lane, &retain)?;
                lane.require_workspace_idle(&mut self.owner, &retain)?;
                self.owner.require_text_control_io_quiescent()?;
                self.require_text_control_write_idle_for_query()?;
                issuer.$validate(self)?;
                // The actual issuer proves SAME A/closed SDK original and its real
                // completed accepted query (Stage) or genuine immutable Ready (Commit).
                // Codec DATA/Empty and historical receipts never substitute. Only
                // cached original/slot/negative checks follow that final proof.
                predecessor.require_same_lane_original(lane, &retain)?;
                lane.require_workspace_idle(&mut self.owner, &retain)?;
                lease.require_intent_write_negative(self)?;
                self.owner.require_text_control_io_quiescent()?;
                self.validate_source_only()?;
                let end = lane.reserve_create_control_write_end(&mut self.owner, &retain)?;
                // SAME canonical edge excludes new query captures before outside IO.
                // No wait/copy/validation follows this real reservation before caching.
                *admitted = Some($marker {
                    original,
                    unsigned,
                    retain,
                    physical: physical.take().expect("same outside I lease checked"),
                    end,
                });
                Ok(())
            }
        }
    };
}
intent_admission!(
    admit_text_intent_stage_with_predecessor_into,
    PreparedV2TextUnsignedIntent,
    AdmittedV2TextIntentStage,
    validate_stage_under_current_guard
);
intent_admission!(
    admit_text_intent_commit_with_predecessor_into,
    ReadyV2TextUnsignedIntentSelection,
    AdmittedV2TextIntentCommit,
    validate_ready_under_current_guard
);
