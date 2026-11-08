//! Actual successful ordinary-lane removal after fresh physical W_C completion.
//!
//! This owner is deliberately different from successful empty-promotion retirement.
//! The Current issuer must first prove real WC/new-B closure, the SAME final basis,
//! and actual old-root drain. No Drop, absent slot, zero reported by a caller, or
//! historical journal can manufacture this receipt. Suspension is a separate path.
use super::*;
use crate::store::persistence::v2_text::{
    OriginalV2RetainObservation, OriginalV2TextAcceptedCreateObservation, TextRetirementPlanIdData,
};
use crate::store::persistence::V2AdvancedAppendBasis;

/// Both the actual removed lane entry and unique caller survive the late Current
/// postcheck. The SAME genuine root-drain owner moves here; an earlier outside
/// RemovedQueryGenerations array may already have drained independently.
pub(in crate::store) struct RetiredV2TextCreateLane {
    caller: V2TextCreateLane,
    registered: Arc<LaneBody>,
    drained: V2TextRetirementRootsDrained,
    original: OriginalV2TextAcceptedCreateObservation,
    retain: OriginalV2RetainObservation,
    plan: TextRetirementPlanIdData,
    // LAST: the actual final observation retains the independently paid B metadata
    // until this fixed receipt and all removed ownership above have drained.
    basis: V2AdvancedAppendBasis,
}

/// Additional fixed successful-completion ownership, paid in B's NEW2MiB class.
/// No array/Arc allocation occurs at the consuming canonical edge.
pub(in crate::store) const V2_TEXT_CREATE_RETIREMENT_LAYOUT_BYTES: usize =
    2 * std::mem::size_of::<RetiredV2TextCreateLane>() + 4 * 64;

impl RetiredV2TextCreateLane {
    pub(in crate::store) fn require_original(
        &self,
        original: &OriginalV2TextAcceptedCreateObservation,
        retain: &OriginalV2RetainObservation,
        plan: &TextRetirementPlanIdData,
        basis: &V2AdvancedAppendBasis,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)?;
        self.retain.require_same_original(retain)?;
        self.basis.require_same_original(basis)?;
        self.drained.require_original(original, plan)?;
        self.drained.require_basis(basis)?;
        if self.plan != *plan {
            return Err(refused());
        }
        self.require_finish_negative()
    }

    /// Immutable loan of the SAME closed proof; it is not a replacement constructor
    /// or permission to run WD/WC against an absent/retired ordinary lane.
    pub(in crate::store) fn drained_roots(&self) -> &V2TextRetirementRootsDrained {
        &self.drained
    }

    /// Private identity bridge for the concrete root-drain family. Cancellation is
    /// REQUIRED here: all ordinary positive admissions must remain permanently dead.
    /// We compare actual opaque identities rather than resurrecting require_registered.
    pub(in crate::store::owner_evidence::canonical::text_lane) fn require_lane_observation(
        &self,
        original: &OriginalV2TextCreateLaneObservation,
        retain: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.retain.require_same_original(retain)?;
        if !original
            .identity
            .ptr_eq(&Arc::downgrade(&self.caller.control.identity))
            || !original
                .original_lane
                .identity
                .ptr_eq(&Arc::downgrade(&self.registered.identity))
        {
            return Err(refused());
        }
        self.require_finish_negative()
    }

    /// Cached checks only, usable while a genuine finish guard holds canonical.
    /// No account wait, source upgrade, SDK/model operation, File or slot mutation.
    pub(in crate::store) fn require_finish_negative(&self) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.caller.original_lane.body, &self.registered)
            || !self.registered.cancelled.load(Ordering::Acquire)
            || self.registered.control_write_active.load(Ordering::Acquire)
        {
            return Err(refused());
        }
        self.caller
            .control
            .work
            .require_bound_idle_negative(&self.retain)
    }
}

impl V2TextOwnerNamespace<'_, '_> {
    /// Called ONLY after the Current layer's concrete same-original WC/new-B/final
    /// proof. Real IO Ends/work gates are zero under this same canonical writer.
    /// All fallible checks precede the first cancellation/removal; both unique
    /// owners and the actual removed registry Arc cache before any late error.
    pub(in crate::store) fn retire_completed_text_create_lane_into(
        &mut self,
        caller: &mut Option<V2TextCreateLane>,
        original: &OriginalV2TextAcceptedCreateObservation,
        retain: &OriginalV2RetainObservation,
        plan: &TextRetirementPlanIdData,
        basis: &V2AdvancedAppendBasis,
        drained: &mut Option<V2TextRetirementRootsDrained>,
        retired: &mut Option<RetiredV2TextCreateLane>,
    ) -> Result<(), AppError> {
        self.validate_source_only()?;
        basis.require_original_accepted(original)?;
        basis.validate_negative()?;
        if let Some(existing) = retired.as_ref() {
            if caller.is_some() || drained.is_some() {
                return Err(refused());
            }
            existing.require_original(original, retain, plan, basis)?;
            return self.require_retired_text_create_lane(existing);
        }
        let lane = caller.as_ref().ok_or_else(refused)?;
        lane.require_workspace_idle(self, retain)?;
        lane.require_signing_negative(retain)?;
        let roots = drained.as_ref().ok_or_else(refused)?;
        roots.require_original(original, plan)?;
        roots.require_basis(basis)?;
        self.require_text_retirement_roots_drained(lane, roots)?;
        self.require_text_control_io_quiescent()?;
        self.require_text_control_write_idle()?;
        let slots = &mut self
            .text_lane_runtime_mut()
            .buffer
            .as_mut()
            .ok_or_else(refused)?
            .slots;
        let index = slots
            .iter()
            .position(|slot| {
                slot.as_ref()
                    .is_some_and(|entry| Arc::ptr_eq(entry, &lane.original_lane.body))
            })
            .ok_or_else(refused)?;
        // Linearization: permanently poison the OLD control BEFORE removing its
        // actual slot. A successor lane never makes this old caller usable again.
        lane.original_lane
            .body
            .cancelled
            .store(true, Ordering::Release);
        let registered = slots[index].take().expect("same actual lane slot");
        *retired = Some(RetiredV2TextCreateLane {
            caller: caller.take().expect("same unique ordinary lane"),
            registered,
            drained: drained.take().expect("same actual detached/drained roots"),
            original: original.clone(),
            retain: retain.clone(),
            plan: *plan,
            basis: basis.clone(),
        });
        Ok(())
    }

    /// Distinct exact-retired path, never a relaxation of normal require_registered.
    /// The opaque owning receipt proves which Arc THIS edge removed. Registry
    /// absence alone cannot establish retirement, drain or a resident handoff.
    pub(in crate::store) fn require_retired_text_create_lane(
        &self,
        retired: &RetiredV2TextCreateLane,
    ) -> Result<(), AppError> {
        self.validate_source_only()?;
        retired.require_finish_negative()?;
        let body = &retired.registered;
        let admission = self.scope_admission();
        body.source.require_same_original(admission)?;
        if body.key.group() != admission.group()
            || &body.key.signer != admission.signer()
            || body.source_id != admission.source().data().source
            || self
                .text_lane_runtime()
                .buffer
                .as_ref()
                .is_some_and(|buffer| {
                    buffer
                        .slots
                        .iter()
                        .flatten()
                        .any(|slot| Arc::ptr_eq(slot, body))
                })
        {
            return Err(refused());
        }
        self.require_installed_text_writer(admission.writer_observation()?)?;
        self.require_text_retirement_roots_after_lane_retired(retired, &retired.drained)?;
        self.require_text_control_io_quiescent()?;
        self.require_text_control_write_idle()
    }
}

impl std::fmt::Debug for RetiredV2TextCreateLane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetiredV2TextCreateLane")
            .finish_non_exhaustive()
    }
}
