//! Hardcoded first/next predecessor entrypoints share the SAME concrete proof.
//! Completed supplies only its closed native binding/domain negative. It cannot
//! stand in for fresh journal/artifact/Source proof or become a Clear predecessor.
use super::*;
use crate::store::critical_namespaces::current_layout::BoundV2TextCompletedCreatePredecessor;

macro_rules! typed_predecessor_entrypoints {
    ($first:ident, $next:ident, $inner:ident, $issuer:ident, $marker:ident) => {
        impl V2TextCurrentGroupGuard<'_, '_> {
            /// Existing first-request signature and physical contract are unchanged.
            pub(in crate::store) fn $first(
                &mut self,
                lane: &V2TextCreateLane,
                predecessor: &BoundV2TextCreatePredecessor,
                issuer: &$issuer,
                physical: &mut Option<PreparedV2TextPhysicalLease>,
                admitted: &mut Option<$marker>,
            ) -> Result<(), AppError> {
                self.$inner(
                    lane,
                    CreatePredecessorRef::Clear(predecessor),
                    issuer,
                    physical,
                    admitted,
                )
            }
            /// The only next-request conversion is this concrete borrowed variant.
            /// No parsed WC, caller flag or generic binding callback is accepted.
            pub(in crate::store) fn $next(
                &mut self,
                lane: &V2TextCreateLane,
                predecessor: &BoundV2TextCompletedCreatePredecessor,
                issuer: &$issuer,
                physical: &mut Option<PreparedV2TextPhysicalLease>,
                admitted: &mut Option<$marker>,
            ) -> Result<(), AppError> {
                self.$inner(
                    lane,
                    CreatePredecessorRef::Completed(predecessor),
                    issuer,
                    physical,
                    admitted,
                )
            }
        }
    };
}
typed_predecessor_entrypoints!(
    admit_text_intent_stage_into,
    admit_text_completed_intent_stage_into,
    admit_text_intent_stage_with_predecessor_into,
    PreparedV2TextUnsignedIntent,
    AdmittedV2TextIntentStage
);
typed_predecessor_entrypoints!(
    admit_text_intent_commit_into,
    admit_text_completed_intent_commit_into,
    admit_text_intent_commit_with_predecessor_into,
    ReadyV2TextUnsignedIntentSelection,
    AdmittedV2TextIntentCommit
);
