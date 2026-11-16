//! Pure negative check for consuming an original raw lane into ordinary ownership.
//!
//! This code is a child of promotion so no general caller can inspect/reconstruct its
//! private state or bind/rebind the opaque plan. The actual canonical mutex serializes
//! this check with the ONLY actual stage/commit admission constructors.
use super::*;

impl PromotionWorkControl {
    pub(in crate::store::owner_evidence::canonical::text_lane) fn require_unused_create_conversion(
        &self,
    ) -> Result<(), AppError> {
        if self.plan.get().is_some() || self.state.load(Ordering::Acquire) != OPEN_IDLE {
            return Err(refused());
        }
        Ok(())
    }
}
