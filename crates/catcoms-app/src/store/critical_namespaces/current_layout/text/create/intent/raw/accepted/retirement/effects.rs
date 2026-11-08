//! DISTINCT retirement effects. Deletion first persists WD and drains its actual
//! unlink/files/pages work. Only a NEW Current completion admission may select WC.
//! Neither parsed WD nor an IO count can replace the genuine completion issuer.
use super::*;
use crate::store::owner_evidence::canonical::text_lane::V2TextCreateControlWriteEnd;
use crate::store::persistence::v2_text::{
    OriginalV2RetainObservation, OriginalV2TextAcceptedCreateObservation,
    ReadyV2TextRetirementCompletion, TextRetirementPlanIdData,
};

pub(in crate::store) struct AdmittedV2TextAcceptedRetirementDeletion {
    original: OriginalV2TextAcceptedCreateObservation,
    plan: TextRetirementPlanIdData,
    retain: OriginalV2RetainObservation,
    basis: V2AdvancedAppendBasis,
    physical: PreparedV2TextPhysicalLease,
    // LAST: actual Files/key/unlink/parent-barrier cleanup and SAME physical lease
    // drain before this finite writer gate/cap. Stop cannot erase admitted work.
    end: V2TextCreateControlWriteEnd,
}
pub(in crate::store) struct AdmittedV2TextAcceptedRetirementCompletion {
    original: OriginalV2TextAcceptedCreateObservation,
    plan: TextRetirementPlanIdData,
    retain: OriginalV2RetainObservation,
    basis: V2AdvancedAppendBasis,
    physical: PreparedV2TextPhysicalLease,
    end: V2TextCreateControlWriteEnd,
}

pub(in crate::store) const V2_TEXT_RETIREMENT_ADMISSION_LAYOUT_BYTES: usize = 2
    * std::mem::size_of::<PreparedV2TextPhysicalLease>()
    + std::mem::size_of::<AdmittedV2TextAcceptedRetirementDeletion>()
    + std::mem::size_of::<AdmittedV2TextAcceptedRetirementCompletion>()
    + 4 * 64;

macro_rules! retirement_marker {
    ($ty:ident) => {
        impl $ty {
            /// Exact original only. Already admitted finite work may complete after
            /// ScopeStop without renewing Current, root-drain or publication rights.
            pub(in crate::store) fn require_same_retirement(
                &self,
                original: &OriginalV2TextAcceptedCreateObservation,
                plan: &TextRetirementPlanIdData,
            ) -> Result<(), AppError> {
                self.original.require_same_original(original)?;
                if self.plan != *plan {
                    return Err(refused());
                }
                Ok(())
            }
        }
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    };
}
retirement_marker!(AdmittedV2TextAcceptedRetirementDeletion);
retirement_marker!(AdmittedV2TextAcceptedRetirementCompletion);

macro_rules! retirement_admission {
    ($method:ident,$issuer:ident,$marker:ident,$validate:ident) => {
        impl V2TextCurrentGroupGuard<'_, '_> {
            /// Outside marker/lease slots survive every refusal. The immutable
            /// actual issuer supplies its OWN final basis and exact plan; physical
            /// proof covers new-B/WB or durable-WD/absence/cleanup as appropriate.
            pub(in crate::store) fn $method(
                &mut self,
                lane: &V2TextCreateLane,
                issuer: &$issuer,
                basis: &V2AdvancedAppendBasis,
                drained: &V2TextRetirementRootsDrained,
                physical: &mut Option<PreparedV2TextPhysicalLease>,
                admitted: &mut Option<$marker>,
            ) -> Result<(), AppError> {
                // Reject a foreign runtime/read body BEFORE paths, Files or account
                // waits; the shared B scratch invoice requires this early identity.
                issuer.resources().require_same_original(self.resources())?;
                let original = issuer.observe_original();
                let retain = issuer.original_retain();
                let plan = issuer.retirement_plan_id_data()?;
                drained.require_original(&original, &plan)?;
                drained.require_basis(basis)?;
                basis.validate_negative()?;
                if let Some(existing) = admitted.as_ref() {
                    existing.require_same_retirement(&original, &plan)?;
                    existing.retain.require_same_original(&retain)?;
                    existing.basis.require_same_original(basis)?;
                    existing.physical.require_retirement_write_negative(self)?;
                    existing.end.require_same_lane(lane)?;
                    return self.validate_source_only();
                }
                let lease = physical.as_ref().ok_or_else(refused)?;
                lease.validate_retirement_write_under_guard(self)?;
                lane.require_workspace_idle(&mut self.owner, &retain)?;
                self.owner
                    .require_text_retirement_roots_drained(lane, drained)?;
                self.owner.require_text_control_io_quiescent()?;
                self.require_text_control_write_idle_for_query()?;
                // ALL account waits and role-specific journal/source/deletion
                // inspections precede the FINAL concrete full physical proof.
                issuer.$validate(self, drained)?;
                // Cached exact-original/native negatives only after final proof.
                drained.require_original(&original, &plan)?;
                drained.require_basis(basis)?;
                basis.validate_negative()?;
                lane.require_workspace_idle(&mut self.owner, &retain)?;
                self.owner
                    .require_text_retirement_roots_drained(lane, drained)?;
                lease.require_retirement_write_negative(self)?;
                self.owner.require_text_control_io_quiescent()?;
                self.validate_source_only()?;
                let end = lane.reserve_create_control_write_end(&mut self.owner, &retain)?;
                // Same canonical edge samples actual group IO-zero and closes NEW
                // captures. Only infallible moves remain before outside ownership.
                *admitted = Some($marker {
                    original,
                    plan,
                    retain,
                    basis: basis.clone(),
                    physical: physical.take().expect("same outside retirement lease"),
                    end,
                });
                Ok(())
            }
        }
    };
}
retirement_admission!(
    admit_text_retirement_deletion_into,
    PreparedV2TextAcceptedCreate,
    AdmittedV2TextAcceptedRetirementDeletion,
    validate_retirement_deletion_under_current_guard
);
retirement_admission!(
    admit_text_retirement_completion_into,
    ReadyV2TextRetirementCompletion,
    AdmittedV2TextAcceptedRetirementCompletion,
    validate_completion_under_current_guard
);
