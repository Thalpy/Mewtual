//! Fresh complete-seed repair and pre-MAIN scratch cleanup admissions.
//!
//! Neither historical Stage admission nor a parsed seed grants these capabilities. Before
//! capture, the same original bound lane must have no actual admitted end owner. The same
//! canonical guard remains held through capture, full source proof and a new affine claim.
//! Every captured File moves BEFORE the physical lease; the actual work end drops LAST.
use super::*;
mod capture;
use crate::store::persistence::v2_text::{
    CapturedV2TextPromotionScratch, CapturedV2TextPromotionSeedRepair,
    PreparedV2TextPromotionScratch, PreparedV2TextPromotionSeedRepair,
};
pub(in crate::store) use capture::{
    PreparedV2TextPromotionScratchCapture, PreparedV2TextPromotionSeedRepairCapture,
    V2TextPromotionScratchSourceSlot, V2TextPromotionSeedRepairSourceSlot,
};

pub(in crate::store) struct AdmittedV2TextPromotionSeedRepair {
    original: OriginalV2TextPromotionObservation,
    capture: PreparedV2TextPromotionSeedRepairCapture,
    _work_end: V2TextPromotionWorkEnd,
}
pub(in crate::store) struct AdmittedV2TextPromotionScratch {
    original: OriginalV2TextPromotionObservation,
    capture: PreparedV2TextPromotionScratchCapture,
    _work_end: V2TextPromotionWorkEnd,
}

/// Actual simultaneous admitted-role layouts, included in the paired physical invoice
/// BEFORE its fees, copied frames, native paths or captured Files are allocated/opened.
pub(in crate::store) const V2_TEXT_PROMOTION_REPAIR_ADMISSION_LAYOUT_BYTES: usize =
    std::mem::size_of::<AdmittedV2TextPromotionSeedRepair>()
        + std::mem::size_of::<AdmittedV2TextPromotionScratch>()
        + std::mem::size_of::<PreparedV2TextPromotionSeedRepairCapture>()
        + std::mem::size_of::<PreparedV2TextPromotionScratchCapture>()
        + std::mem::size_of::<V2TextPromotionSeedRepairSourceSlot<'static>>()
        + std::mem::size_of::<V2TextPromotionScratchSourceSlot<'static>>()
        + 4 * 64;

impl AdmittedV2TextPromotionSeedRepair {
    /// Cached exact-plan identity only, safe for this SAME admitted finite work even after
    /// source revocation. It supplies no fresh Current, signing or selector permission.
    pub(in crate::store) fn require_same_promotion(
        &self,
        original: &OriginalV2TextPromotionObservation,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)
    }
    pub(in crate::store) fn source(&self) -> &CapturedV2TextPromotionSeedRepair {
        self.capture
            .source_data()
            .expect("actual source checked before affine admission")
    }
}
impl AdmittedV2TextPromotionScratch {
    pub(in crate::store) fn require_same_promotion(
        &self,
        original: &OriginalV2TextPromotionObservation,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)
    }
    pub(in crate::store) fn source(&self) -> &CapturedV2TextPromotionScratch {
        self.capture
            .source_data()
            .expect("actual source checked before affine admission")
    }
}

impl V2TextCurrentGroupGuard<'_, '_> {
    /// The caller constructs ONE active `capture` bundle OUTSIDE AD. Every captured File
    /// remains structurally coupled to its same-vault OS lease on ALL errors. Borrowed fill
    /// slots expose no take/owned-source API. Drop unadmitted bundles outside writers; no
    /// active bundle or physical lease belongs in dormant recovery DATA.
    pub(in crate::store) fn admit_text_promotion_seed_repair_into(
        &mut self,
        lane: &V2TextLane,
        repair: &mut PreparedV2TextPromotionSeedRepair,
        capture: &mut Option<PreparedV2TextPromotionSeedRepairCapture>,
        admitted: &mut Option<AdmittedV2TextPromotionSeedRepair>,
    ) -> Result<(), AppError> {
        let original = repair.observe_original();
        if let Some(existing) = admitted.as_ref() {
            existing.require_same_promotion(&original)?;
            existing.capture.require_original_negative(self)?;
            lane.require_registered(&mut self.owner)?;
            return self.validate_source_only();
        }
        let bundle = capture.as_mut().ok_or_else(refused)?;
        bundle.require_same_promotion(&original)?;
        bundle.validate_resources(self)?;
        // Actual prior-end quiescence is required BEFORE any source File is selected.
        // This does not bind/rebind a plan or create a drain/work permission by itself.
        lane.require_bound_promotion_idle(&mut self.owner, &original)?;
        repair.capture(self, &mut bundle.source_slot())?;
        bundle.source_data()?.validate_under_current_guard(self)?;
        lane.require_registered(&mut self.owner)?;
        bundle.require_original_negative(self)?;
        self.validate_source_only()?;
        let work_end = lane.reserve_bound_promotion_repair_end(&mut self.owner, &original)?;
        // Capture and every fallible proof preceded reserve. The same caller slots are
        // still mutably borrowed, so only infallible transfers follow the affine claim.
        *admitted = Some(AdmittedV2TextPromotionSeedRepair {
            original,
            capture: capture
                .take()
                .expect("same whole source/lease bundle checked before reserve"),
            _work_end: work_end,
        });
        Ok(())
    }

    /// Separate unauthenticated/torn scratch role, requiring MAIN absence and no dependent
    /// workspace. A complete authenticated unsupported/foreign stage must refuse in capture;
    /// no generic decoder error, prior Saved flag or path-presence test can authorize unlink.
    pub(in crate::store) fn admit_text_promotion_scratch_into(
        &mut self,
        lane: &V2TextLane,
        scratch: &mut PreparedV2TextPromotionScratch,
        capture: &mut Option<PreparedV2TextPromotionScratchCapture>,
        admitted: &mut Option<AdmittedV2TextPromotionScratch>,
    ) -> Result<(), AppError> {
        let original = scratch.observe_original();
        if let Some(existing) = admitted.as_ref() {
            existing.require_same_promotion(&original)?;
            existing.capture.require_original_negative(self)?;
            lane.require_registered(&mut self.owner)?;
            return self.validate_source_only();
        }
        let bundle = capture.as_mut().ok_or_else(refused)?;
        bundle.require_same_promotion(&original)?;
        bundle.validate_resources(self)?;
        lane.require_bound_promotion_idle(&mut self.owner, &original)?;
        scratch.capture(self, &mut bundle.source_slot())?;
        bundle.source_data()?.validate_under_current_guard(self)?;
        lane.require_registered(&mut self.owner)?;
        bundle.require_original_negative(self)?;
        self.validate_source_only()?;
        let work_end = lane.reserve_bound_promotion_repair_end(&mut self.owner, &original)?;
        *admitted = Some(AdmittedV2TextPromotionScratch {
            original,
            capture: capture
                .take()
                .expect("same whole source/lease bundle checked before reserve"),
            _work_end: work_end,
        });
        Ok(())
    }
}

impl std::fmt::Debug for AdmittedV2TextPromotionSeedRepair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedV2TextPromotionSeedRepair")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for AdmittedV2TextPromotionScratch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedV2TextPromotionScratch")
            .finish_non_exhaustive()
    }
}
