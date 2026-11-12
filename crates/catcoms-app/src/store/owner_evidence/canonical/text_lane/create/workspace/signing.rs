//! Pure tail checks while an actual selected-I signing guard holds canonical Current.
//! These negatives do not prove registration, selection, preimage, or permission to sign.
use super::*;

impl V2TextCreateLane {
    /// The guard constructor already proved SAME registration under its held canonical
    /// writer. No new work/close/retirement admission can run while that guard remains
    /// borrowed; outside ends may only finish. Recheck immutable binding and cached
    /// cancellation/cap/gate negatives without any mutation, account/FS/SDK wait.
    pub(in crate::store) fn require_signing_negative(
        &self,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.observe_original().require_same_original(self)?;
        self.control
            .work
            .original
            .get()
            .ok_or_else(refused)?
            .require_same_original(original)?;
        if self.control.work.active.load(Ordering::Acquire)
            || self
                .original_lane
                .body
                .control_write_active
                .load(Ordering::Acquire)
        {
            return Err(refused());
        }
        Ok(())
    }
}
