//! Actual selected-B admission to the SAME verifier's bounded advance and final cut.
//! The outside SavedB Arc supplies lifetime, never a mutable PreparedB/SDK/model loan.
//! A separate final issuer proves real retired-math cleanup before its fresh B proof.
use super::*;
use crate::store::persistence::v2_text::SavedV2AcceptedCreate;
use catcoms_replication::chat::V2HostSelectionExpectation;

pub(in crate::store) struct SavedAcceptedV2TextAdvanceGuard<'cut, 'guard, 'writer> {
    current: &'cut V2TextCurrentGroupGuard<'guard, 'writer>,
    lane: &'cut V2TextCreateLane,
    _saved: &'cut SavedV2AcceptedCreate,
    original: OriginalV2TextAcceptedCreateObservation,
    unsigned: OriginalV2UnsignedPlanObservation,
    retain: OriginalV2RetainObservation,
    expectation: V2HostSelectionExpectation,
}

pub(in crate::store) struct SavedAcceptedV2TextFinalGuard<'cut, 'guard, 'writer> {
    current: &'cut V2TextCurrentGroupGuard<'guard, 'writer>,
    lane: &'cut V2TextCreateLane,
    _saved: &'cut SavedV2AcceptedCreate,
    original: OriginalV2TextAcceptedCreateObservation,
    unsigned: OriginalV2UnsignedPlanObservation,
    retain: OriginalV2RetainObservation,
    expectation: V2HostSelectionExpectation,
}

/// Both guarded cuts are separately included in B's NEW2MiB metadata debit. Actual
/// Core model/candidate/retired math uses its original independently retained debit.
pub(in crate::store) const V2_TEXT_ACCEPTED_ADVANCE_GUARD_LAYOUT_BYTES: usize = 2
    * std::mem::size_of::<SavedAcceptedV2TextAdvanceGuard<'static, 'static, 'static>>()
    + 2 * std::mem::size_of::<SavedAcceptedV2TextFinalGuard<'static, 'static, 'static>>()
    + 4 * std::mem::size_of::<V2HostSelectionExpectation>()
    + 4 * 64;

macro_rules! accepted_cut {
    ($method:ident, $guard:ident, $validate:ident) => {
        impl<'guard, 'writer> V2TextCurrentGroupGuard<'guard, 'writer> {
            /// `prepared` has deliberately NO `'cut` lifetime. Clone the genuine
            /// SavedB Arc outside Current first; Root can then mutate only its SAME
            /// whole custody after this temporary physical validator loan ends.
            pub(in crate::store) fn $method<'cut>(
                &'cut mut self,
                lane: &'cut V2TextCreateLane,
                saved: &'cut SavedV2AcceptedCreate,
                prepared: &PreparedV2TextAcceptedCreate,
            ) -> Result<$guard<'cut, 'guard, 'writer>, AppError> {
                let original = prepared.observe_original();
                let unsigned = prepared.original_unsigned();
                let retain = prepared.original_retain();
                let expectation = saved.selection_expectation_data();
                prepared
                    .resources()
                    .require_same_original(self.resources())?;
                lane.require_workspace_idle(&mut self.owner, &retain)?;
                self.owner.require_text_control_io_quiescent()?;
                self.require_text_control_write_idle_for_query()?;
                // Concrete accounts/SDK/candidate or advanced+retiredNone predicate
                // precedes the FINAL flat NEW-B source proof. Old I/S/TA, a parsed
                // locator and a historical SavedB cannot supply this authority.
                prepared.$validate(self, saved)?;
                // All remaining work is cached native/original/negative inspection.
                // The SAME canonical guard stays held through Root's infallible cut.
                lane.require_workspace_idle(&mut self.owner, &retain)?;
                self.owner.require_text_control_io_quiescent()?;
                lane.require_signing_negative(&retain)?;
                self.validate_source_only()?;
                unsigned.validate_negative()?;
                Ok($guard {
                    current: self,
                    lane,
                    _saved: saved,
                    original,
                    unsigned,
                    retain,
                    expectation,
                })
            }
        }
    };
}
accepted_cut!(
    admit_accepted_advance,
    SavedAcceptedV2TextAdvanceGuard,
    validate_advance_under_current_guard
);
accepted_cut!(
    admit_accepted_final,
    SavedAcceptedV2TextFinalGuard,
    validate_final_under_current_guard
);

macro_rules! cut_association {
    ($ty:ident) => {
        impl $ty<'_, '_, '_> {
            pub(in crate::store) fn require_original_accepted(
                &self,
                original: &OriginalV2TextAcceptedCreateObservation,
            ) -> Result<(), AppError> {
                // This SAME observation transitively binds original S/I/SDK/Retain.
                self.original.require_same_original(original)
            }
            pub(in crate::store) fn require_original_unsigned(
                &self,
                original: &OriginalV2UnsignedPlanObservation,
            ) -> Result<(), AppError> {
                self.unsigned.require_same_original(original)
            }
            pub(in crate::store) fn require_original_retain(
                &self,
                original: &OriginalV2RetainObservation,
            ) -> Result<(), AppError> {
                self.retain.require_same_original(original)
            }
            pub(in crate::store) fn expectation_data(&self) -> &V2HostSelectionExpectation {
                &self.expectation
            }
            /// No account/codec/FS/model/SDK work follows the actual final B proof.
            pub(in crate::store) fn validate_negative(&self) -> Result<(), AppError> {
                self.unsigned.validate_negative()?;
                self.current.validate_source_only()?;
                self.lane.require_signing_negative(&self.retain)?;
                self.current.owner.require_text_control_io_quiescent()?;
                self.current.require_text_control_write_idle_for_query()?;
                self.current.validate_source_only()?;
                self.unsigned.validate_negative()
            }
        }
        impl std::fmt::Debug for $ty<'_, '_, '_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    };
}
cut_association!(SavedAcceptedV2TextAdvanceGuard);
cut_association!(SavedAcceptedV2TextFinalGuard);
