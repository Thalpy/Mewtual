//! Bound-plan quiescence for the distinct complete-seed repair and scratch cleanup roles.
//!
//! Neither path may initialize/rebind the lane's opaque plan. A prior ACTUAL Stage bound
//! it, and its real final end owner must have dropped before either capture can inspect
//! a workspace. The caller holds the same canonical guard across capture and admission.
use super::*;

impl V2TextLane {
    /// Negative preparation check only, never a permission to read/repair/unlink a file.
    /// It deliberately cannot manufacture a drained receipt from a caller-reported count.
    pub(in crate::store) fn require_bound_promotion_idle(
        &self,
        namespace: &mut V2TextOwnerNamespace<'_, '_>,
        original: &OriginalV2TextPromotionObservation,
    ) -> Result<(), AppError> {
        self.require_registered(namespace)?;
        self.body.promotion.require_plan(original)?;
        if self.body.promotion.state.load(Ordering::Acquire) != OPEN_IDLE {
            return Err(refused());
        }
        Ok(())
    }

    /// Only after the concrete captured-role proof, including all final source/name checks.
    /// The same affine end/cap applies; no new plan binding or historical Stage admission
    /// is used as this repair's positive permission. Success must move directly to its
    /// external admitted slot, with no later fallible step or destructor under writers.
    pub(in crate::store) fn reserve_bound_promotion_repair_end(
        &self,
        namespace: &mut V2TextOwnerNamespace<'_, '_>,
        original: &OriginalV2TextPromotionObservation,
    ) -> Result<V2TextPromotionWorkEnd, AppError> {
        self.require_bound_promotion_idle(namespace, original)?;
        self.body
            .promotion
            .state
            .compare_exchange(OPEN_IDLE, ACTIVE, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        Ok(V2TextPromotionWorkEnd {
            control: self.body.promotion.clone(),
        })
    }
}
