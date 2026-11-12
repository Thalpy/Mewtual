//! Consume the SAME Core candidate only through actual selected-B Current roles.
//!
//! The advance cell is allocated outside writers under B's NEW real metadata fee.
//! Core's existing source+candidate1MiB still owns both mathematical payloads. After
//! the cut, this cell owns actual old math until its outside-writer drain; no Option
//! absence can reconstruct a candidate, verifier, genesis or completed physical proof.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    SavedAcceptedV2TextAdvanceGuard, SavedAcceptedV2TextFinalGuard,
};
use crate::store::persistence::v2_text::OriginalV2TextAcceptedCreateObservation;
use catcoms_replication::chat::V2RetiredAppendData;
#[cfg(all(test, windows))]
mod hooks;

/// No issuing capability follows from this phase alone. Actual B/Current proofs
/// remain independent, and finalization additionally needs an actual old-math drain.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AppendAdvancePhase {
    CandidateReady,
    AdvancedPendingFinal,
    AdvancedFinal,
    // The model moved to the exact flat resident; missing owner/candidate is final.
    TransferredResident,
}

/// Tiny exact-original evidence of a completed final B proof. It contains no model,
/// SDK, Source owner, File or Current permission. A later WC/retirement/projection
/// consumer must still prove its own current original closure and negative lifetime.
/// Clones share only the actual original observations and their metadata payments.
#[derive(Clone)]
pub(in crate::store) struct V2AdvancedAppendBasis {
    unsigned: OriginalV2UnsignedPlanObservation,
    // LAST: copied exact-B facts remain paid by the actual NEW B2MiB, not A's fee.
    accepted: OriginalV2TextAcceptedCreateObservation,
}
impl V2AdvancedAppendBasis {
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        self.unsigned.require_same_original(&other.unsigned)?;
        self.accepted.require_same_original(&other.accepted)
    }
    pub(in crate::store) fn require_original_accepted(
        &self,
        original: &OriginalV2TextAcceptedCreateObservation,
    ) -> Result<(), AppError> {
        self.accepted.require_same_original(original)
    }
    pub(in crate::store) fn require_original_unsigned(
        &self,
        original: &OriginalV2UnsignedPlanObservation,
    ) -> Result<(), AppError> {
        self.unsigned.require_same_original(original)
    }
    /// Cached original Source/domain negatives only. This is neither a fresh B/WC
    /// proof nor permission to install, retire, publish, or renew a closed lifetime.
    pub(in crate::store) fn validate_negative(&self) -> Result<(), AppError> {
        self.unsigned.validate_negative()
    }
}

/// Preallocated EMPTY retired slot, explicit phase and final-basis slot. This Box
/// stays in the outside-owned whole custody while a Current callback borrows it.
/// Payloads must drop before their funding observation; do not reorder the tail.
pub(super) struct AppendAdvanceCell {
    pub(super) retired: Option<V2RetiredAppendData>,
    pub(super) basis: Option<V2AdvancedAppendBasis>,
    pub(super) phase: AppendAdvancePhase,
    // Flat copy lives under B metadata until the real consuming WC finish.
    pub(super) resident_parent: Option<super::resident::ResidentSelectedParent>,
    // Negative exact-owner transfer mark, not a model/resource alias.
    pub(super) transferred_identity: Option<std::sync::Weak<()>>,
    // LAST: its concrete original observation retains the same new B metadata fee.
    pub(super) accepted: OriginalV2TextAcceptedCreateObservation,
}

// Charged by the NEW B2MiB constructor invoice BEFORE the Box below is allocated.
// The old Root/custody invoice independently includes its actual Option<Box> field.
// These wrapper/move/observation terms are additional to Core's TWO-model payload
// invoice; neither active16MiB nor unused A/I/S metadata supplies overlap credit.
const ADVANCE_METADATA_LAYOUT_BYTES: usize = 4 * std::mem::size_of::<AppendAdvanceCell>()
    + 4 * std::mem::size_of::<Option<Box<AppendAdvanceCell>>>()
    + 4 * std::mem::size_of::<V2AdvancedAppendBasis>()
    + 8 * std::mem::size_of::<OriginalV2TextAcceptedCreateObservation>()
    + 4 * std::mem::size_of::<OriginalV2UnsignedPlanObservation>();

impl PreparedV2UnsignedOriginalCustody {
    pub(in crate::store) const fn accepted_advance_metadata_layout_bytes() -> usize {
        ADVANCE_METADATA_LAYOUT_BYTES
    }

    /// OUTSIDE writers and only after the actual new B metadata payment exists.
    /// This inert preallocation is not an advance/finalization receipt. A refusal
    /// preserves the original candidate and any already cached advance cell intact.
    pub(in crate::store) fn prepare_accepted_advance_data(
        &mut self,
        retained: &PreparedV2TextRetainTransaction,
        original: &OriginalV2TextAcceptedCreateObservation,
    ) -> Result<(), AppError> {
        self.validate_accounts_only()?;
        original.validate_prepaid_account()?;
        original.require_original_unsigned(&self.original)?;
        self.require_original_plan_data(retained)?;
        self.original.validate_negative()?;
        if self.advance.is_none() {
            self.require_candidate_matches_sdk_data()?;
            self.advance = Some(Box::new(AppendAdvanceCell {
                retired: None,
                basis: None,
                phase: AppendAdvancePhase::CandidateReady,
                resident_parent: None,
                transferred_identity: None,
                accepted: original.clone(),
            }));
        }
        // Cache the whole paid Box before the late negative. A cancelled retry may
        // retain it for actual drain/disposal, but never replace it with another B.
        self.require_accepted_advance_cached(retained, original)?;
        self.original.validate_negative()
    }

    /// Account/association work BEFORE the issuer's FINAL genuine new-B proof.
    /// This intentionally accepts the already-advanced phase on lost-return retries;
    /// its exact SDK DATA then matches the same installed model, not an absent next.
    pub(in crate::store) fn validate_accepted_advance_data(
        &self,
        retained: &PreparedV2TextRetainTransaction,
        original: &OriginalV2TextAcceptedCreateObservation,
    ) -> Result<(), AppError> {
        self.validate_accounts_only()?;
        original.validate_prepaid_account()?;
        self.require_accepted_advance_cached(retained, original)?;
        self.original.validate_negative()
    }

    /// Exact-B-bound immutable candidate DATA after the paid advance-cell
    /// preallocation. B seals its original pages from this SAME owning candidate;
    /// it cannot derive another one or borrow an already consumed successor.
    /// No Source, account wait, Current proof, allocation or SDK/model loan occurs.
    pub(in crate::store) fn borrow_accepted_candidate_data(
        &self,
        retained: &PreparedV2TextRetainTransaction,
        original: &OriginalV2TextAcceptedCreateObservation,
    ) -> Result<&catcoms_replication::chat::V2VerifiedAppendCandidate, AppError> {
        self.require_accepted_advance_cached(retained, original)?;
        let cell = self.advance.as_ref().ok_or_else(refused)?;
        if cell.phase != AppendAdvancePhase::CandidateReady {
            return Err(refused());
        }
        self.data.candidate.as_ref().ok_or_else(refused)
    }

    /// Concrete pre-final predicate; the physical issuer must call this BEFORE its
    /// FINAL fresh SavedB proof. An already cached final basis permits idempotent
    /// lost-return retries. No boolean, caller-selected phase or old S proof enters.
    pub(in crate::store) fn require_finalization_ready(
        &self,
        retained: &PreparedV2TextRetainTransaction,
        original: &OriginalV2TextAcceptedCreateObservation,
    ) -> Result<(), AppError> {
        self.validate_accepted_advance_data(retained, original)?;
        self.require_finalization_ready_cached()
    }

    /// The actual Current/selected-B guard borrows no PreparedB or mutable custody.
    /// Every fallible original/phase/SDK/negative check precedes Core's owning take.
    /// No account wait, allocation, physical read or old-model destruction occurs here.
    pub(in crate::store) fn advance_accepted_under_guard(
        &mut self,
        retained: &PreparedV2TextRetainTransaction,
        original: &OriginalV2TextAcceptedCreateObservation,
        guard: &SavedAcceptedV2TextAdvanceGuard<'_, '_, '_>,
    ) -> Result<(), AppError> {
        guard.require_original_accepted(original)?;
        guard.require_original_unsigned(&self.original)?;
        guard.require_original_retain(&retained.original_retain())?;
        self.require_accepted_advance_cached(retained, original)?;
        self.original.validate_negative()?;
        guard.validate_negative()?;
        if self.advance.as_ref().ok_or_else(refused)?.phase == AppendAdvancePhase::CandidateReady {
            // Observation only, never an injected callback before the owning cut.
            #[cfg(all(test, windows))]
            hooks::note_core_dispatch();
            let retired = self
                .data
                .resident_owner
                .as_mut()
                .ok_or_else(refused)?
                .model
                .advance_host_selected(guard.expectation_data(), &mut self.data.candidate)
                .map_err(|_| refused())?;
            // From Core's successful take through BOTH owning caches, only these
            // infallible moves may run. In particular do not add a hook, source read,
            // Result, callback, allocation or drop/refund before phase is recorded.
            let cell = self
                .advance
                .as_mut()
                .expect("same checked prepaid advance cell");
            cell.retired = Some(retired);
            cell.phase = AppendAdvancePhase::AdvancedPendingFinal;
            #[cfg(all(test, windows))]
            hooks::after_cache(0)?;
        }
        // Some(retired) and AdvancedPendingFinal survive even this late refusal.
        // A repeat under the same genuine B guard never enters Core a second time.
        guard.validate_negative()?;
        self.original.validate_negative()
    }

    /// Negative-only destruction of the actual old model OUTSIDE ALL writers.
    /// The owning Host runs this before acquiring new Current even on its first
    /// attempt or after Stop. Absence/CandidateReady means there is no old math to
    /// destroy; that no-op grants no final readiness or positive permission.
    pub(in crate::store) fn drain_accepted_retired_math_if_present(
        &mut self,
        original: &OriginalV2TextAcceptedCreateObservation,
    ) -> Result<(), AppError> {
        self.validate_accounts_only()?;
        original.validate_prepaid_account()?;
        original.require_original_unsigned(&self.original)?;
        let Some(cell) = self.advance.as_ref() else {
            return Ok(());
        };
        cell.accepted.require_same_original(original)?;
        if cell.phase == AppendAdvancePhase::CandidateReady {
            return if cell.retired.is_none() {
                Ok(())
            } else {
                Err(refused())
            };
        }
        self.drain_accepted_retired_math(original)
    }

    /// Negative-only destruction of the actual old model OUTSIDE ALL writers.
    /// This must finish before acquiring the distinct final B guard. Cancellation
    /// cannot prevent finite disposal; no new Source/current authority is acquired.
    pub(in crate::store) fn drain_accepted_retired_math(
        &mut self,
        original: &OriginalV2TextAcceptedCreateObservation,
    ) -> Result<(), AppError> {
        self.validate_accounts_only()?;
        original.validate_prepaid_account()?;
        let cell = self.advance.as_mut().ok_or_else(refused)?;
        cell.accepted.require_same_original(original)?;
        original.require_original_unsigned(&self.original)?;
        if cell.phase == AppendAdvancePhase::CandidateReady || self.data.candidate.is_some() {
            return Err(refused());
        }
        // The original model/SDK and B metadata fees remain owned after this actual
        // destructor returns. An inert observation or IO-count zero is not this drain.
        drop(cell.retired.take());
        Ok(())
    }

    /// Distinct final proof AFTER the outside old-math drain. The issuer has already
    /// rechecked the actual complete new B closure independently of old I/S/TA.
    /// Cache the exact final basis/phase before any late negative or callback return.
    pub(in crate::store) fn confirm_accepted_final_under_guard(
        &mut self,
        retained: &PreparedV2TextRetainTransaction,
        original: &OriginalV2TextAcceptedCreateObservation,
        guard: &SavedAcceptedV2TextFinalGuard<'_, '_, '_>,
    ) -> Result<(), AppError> {
        guard.require_original_accepted(original)?;
        guard.require_original_unsigned(&self.original)?;
        guard.require_original_retain(&retained.original_retain())?;
        self.require_accepted_advance_cached(retained, original)?;
        self.require_finalization_ready_cached()?;
        self.original.validate_negative()?;
        guard.validate_negative()?;
        let cell = self
            .advance
            .as_mut()
            .expect("same checked drained advance cell");
        if cell.basis.is_none() {
            // Arc/Weak observation clones only: no allocation, model clone or source
            // renewal. Both originals already own their actual fee tails before this.
            cell.basis = Some(V2AdvancedAppendBasis {
                unsigned: self.original.clone(),
                accepted: cell.accepted.clone(),
            });
            cell.phase = AppendAdvancePhase::AdvancedFinal;
            #[cfg(all(test, windows))]
            hooks::after_cache(1)?;
        }
        guard.validate_negative()?;
        self.original.validate_negative()
    }

    /// Existing sealed exact-original basis DATA only, also after a late error.
    /// It cannot replace a fresh B/WC/retirement/current/projection admission.
    pub(in crate::store) fn accepted_final_basis_data(
        &self,
        original: &OriginalV2TextAcceptedCreateObservation,
    ) -> Result<&V2AdvancedAppendBasis, AppError> {
        let cell = self.advance.as_ref().ok_or_else(refused)?;
        cell.accepted.require_same_original(original)?;
        if cell.phase != AppendAdvancePhase::AdvancedFinal || cell.retired.is_some() {
            return Err(refused());
        }
        cell.basis.as_ref().ok_or_else(refused)
    }

    fn require_candidate_matches_sdk_data(&self) -> Result<(), AppError> {
        let prepared = self.data.prepared.as_ref().ok_or_else(refused)?;
        let data = prepared
            .borrow_existing_materialized_append_data(&self.original.body.original)
            .map_err(|_| refused())?;
        let candidate = self.data.candidate.as_ref().ok_or_else(refused)?;
        if candidate.header_data() != data.header || candidate.image_data() != data.image {
            return Err(refused());
        }
        Ok(())
    }

    /// Pure bounded original/cached DATA checks, safe after the FINAL physical proof.
    /// The Header and image comparisons borrow only immutable paid caches: whole
    /// custody's first-Append handoff already caps the original image at 676 bytes.
    /// No old selected I/S/TA read or rederive is reachable after candidate consumption.
    pub(super) fn require_accepted_advance_cached(
        &self,
        retained: &PreparedV2TextRetainTransaction,
        original: &OriginalV2TextAcceptedCreateObservation,
    ) -> Result<(), AppError> {
        self.require_original_plan_data(retained)?;
        original.require_original_unsigned(&self.original)?;
        let cell = self.advance.as_ref().ok_or_else(refused)?;
        cell.accepted.require_same_original(original)?;
        match cell.phase {
            AppendAdvancePhase::CandidateReady => {
                if cell.retired.is_some() || cell.basis.is_some() {
                    return Err(refused());
                }
                self.require_candidate_matches_sdk_data()
            }
            AppendAdvancePhase::AdvancedPendingFinal | AppendAdvancePhase::AdvancedFinal => {
                if self.data.candidate.is_some()
                    || (cell.phase == AppendAdvancePhase::AdvancedPendingFinal
                        && cell.basis.is_some())
                    || (cell.phase == AppendAdvancePhase::AdvancedFinal
                        && (cell.retired.is_some() || cell.basis.is_none()))
                {
                    return Err(refused());
                }
                let data = self
                    .data
                    .prepared
                    .as_ref()
                    .ok_or_else(refused)?
                    .borrow_existing_materialized_append_data(&self.original.body.original)
                    .map_err(|_| refused())?;
                let model = self.data.resident_owner_data()?.model.replay_data();
                if model.header() != data.header || model.image() != data.image {
                    return Err(refused());
                }
                Ok(())
            }
            AppendAdvancePhase::TransferredResident => Err(refused()),
        }
    }

    fn require_finalization_ready_cached(&self) -> Result<(), AppError> {
        let cell = self.advance.as_ref().ok_or_else(refused)?;
        if !matches!(
            cell.phase,
            AppendAdvancePhase::AdvancedPendingFinal | AppendAdvancePhase::AdvancedFinal
        ) || cell.retired.is_some()
            || self.data.candidate.is_some()
        {
            return Err(refused());
        }
        Ok(())
    }
}
