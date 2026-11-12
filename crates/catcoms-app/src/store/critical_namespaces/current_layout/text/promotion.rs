//! Affine admissions for the concrete first operational promotion only.
//!
//! A resource-only OS lease is acquired BEFORE entering AD and transferred to a retained
//! caller slot only after the real Current/kernel/lane/physical-source checks. Stage and
//! selector commit have distinct owners: neither Ready nor an earlier admission renews
//! Current. The physical runner must retain the consumed owner LAST through actual Files,
//! syscall/result cleanup and its explicit outside-writer drain, including every Err path.
use super::*;
mod accepted;
mod intent;
mod query;
mod raw;
mod repair;
mod retain;
mod retirement;
mod workspace;
use crate::store::owner_evidence::canonical::text_lane::{
    RetiredV2TextPromotionLane, V2TextLane, V2TextPromotionLaneDrain, V2TextPromotionLaneDrained,
    V2TextPromotionWorkEnd,
};
use crate::store::persistence::v2_text::{
    OriginalV2TextPromotionObservation, PreparedV2TextPromotion, ReadyV2TextPromotionSelection,
    SavedV2TextPromotion,
};
use catcoms_rt::RequestCancellation;
use catcoms_storage::VaultPhysicalWorkLease;
pub(in crate::store) use repair::{
    AdmittedV2TextPromotionScratch, AdmittedV2TextPromotionSeedRepair,
    PreparedV2TextPromotionScratchCapture, PreparedV2TextPromotionSeedRepairCapture,
    V2TextPromotionScratchSourceSlot, V2TextPromotionSeedRepairSourceSlot,
    V2_TEXT_PROMOTION_REPAIR_ADMISSION_LAYOUT_BYTES,
};

/// Temporary precursor for one imminent admission. Construct immediately outside AD;
/// do not store in dormant debt, observer metadata or the long-lived readonly host.
/// The full paired promotion invoice pays this layout before this loan is prepared.
pub(in crate::store) struct PreparedV2TextPhysicalLease {
    mount: Arc<()>,
    negative: RequestCancellation,
    resources: V2TextReadResources,
    // LAST: no physical cleanup may remain when this SAME vault lock is released.
    _session: VaultPhysicalWorkLease,
}

impl PreparedV2TextPhysicalLease {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        resources: &V2TextReadResources,
    ) -> Result<Self, AppError> {
        resources.validate_original(store, source)?;
        // This constructor is private to the Store family and reaches ONLY its own
        // loaded session. No caller-supplied lease can be rebound to another mount.
        let prepared = Self {
            mount: store.registry_mount(),
            negative: store.chat_mount.observer(),
            resources: resources.clone(),
            _session: store._session.retain_for_physical_work(),
        };
        resources.require_original_negative(store, source)?;
        if prepared.negative.is_cancelled() {
            return Err(refused());
        }
        Ok(prepared)
    }

    /// Account validation may WAIT. All fresh physical original proofs follow it.
    fn validate_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.require_original_negative(guard)?;
        self.resources
            .validate_original(guard.store(), guard.source())?;
        self.require_original_negative(guard)
    }

    /// Cached original observations only. This is never a physical selected-source proof.
    fn require_original_negative(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.resources.require_same_original(guard.resources())?;
        if self.negative.is_cancelled()
            || !Arc::ptr_eq(&self.mount, &guard.store().registry_mount())
        {
            return Err(refused());
        }
        self.resources
            .require_original_negative(guard.store(), guard.source())?;
        guard.validate_source_only()
    }
}

/// Permission for this exact presealed promotion's immutable stage work, never its CAS.
/// Constructed only by the actual held group guard. Non-Clone and non-deserializable.
pub(in crate::store) struct AdmittedV2TextPromotionStage {
    original: OriginalV2TextPromotionObservation,
    // After all physical File debt, and before the final actual-work end owner below.
    physical: PreparedV2TextPhysicalLease,
    // ACTUAL final end owner, after OS lease release and all helper/File cleanup. This
    // makes an unstarted marker count as real active work and caps the lane at one.
    _work_end: V2TextPromotionWorkEnd,
}

/// Fresh permission to select this exact completed Ready promotion over original genesis.
/// This cannot be constructed from stage completion, an OS lease or a saved/parsed flag.
pub(in crate::store) struct AdmittedV2TextPromotionCommit {
    original: OriginalV2TextPromotionObservation,
    physical: PreparedV2TextPhysicalLease,
    _work_end: V2TextPromotionWorkEnd,
}

/// Included by the real promotion metadata/work invoice before payment or allocation.
/// The original Scope/source/SDK accounts remain additional, independently retained fees.
pub(in crate::store) const V2_TEXT_PROMOTION_ADMISSION_LAYOUT_BYTES: usize = 2
    * std::mem::size_of::<PreparedV2TextPhysicalLease>()
    + std::mem::size_of::<AdmittedV2TextPromotionStage>()
    + std::mem::size_of::<AdmittedV2TextPromotionCommit>()
    + 4 * 64;

impl AdmittedV2TextPromotionStage {
    /// Pure exact-plan identity only. After admission the SAME finite effect may complete
    /// even if cancellation wins; it must not reacquire positive Current from this method.
    pub(in crate::store) fn require_same_promotion(
        &self,
        original: &OriginalV2TextPromotionObservation,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)
    }
}
impl AdmittedV2TextPromotionCommit {
    pub(in crate::store) fn require_same_promotion(
        &self,
        original: &OriginalV2TextPromotionObservation,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)
    }
}

impl V2TextCurrentGroupGuard<'_, '_> {
    /// The caller BORROWS both outside slots through the Scope callback. Once transferred,
    /// a late generic postcheck cannot destroy the actual lease/permission under AD or
    /// make the caller mistake an admitted effect for an unstarted one.
    pub(in crate::store) fn admit_text_promotion_stage_into(
        &mut self,
        lane: &V2TextLane,
        prepared: &PreparedV2TextPromotion,
        physical: &mut Option<PreparedV2TextPhysicalLease>,
        admitted: &mut Option<AdmittedV2TextPromotionStage>,
    ) -> Result<(), AppError> {
        let original = prepared.observe_original();
        if let Some(existing) = admitted.as_ref() {
            existing.require_same_promotion(&original)?;
            existing.physical.require_original_negative(self)?;
            lane.require_registered(&mut self.owner)?;
            return self.validate_source_only();
        }
        let lease = physical.as_ref().ok_or_else(refused)?;
        lease.validate_under_current_guard(self)?;
        self.owner
            .require_installed_text_writer(self.scope_admission().writer_observation()?)?;
        prepared.validate_stage_under_current_guard(self)?;
        // No account/IO/codec/SDK wait after the complete physical proof. The actual
        // registered lane and all original cancellation controls are sampled LAST.
        lane.require_registered(&mut self.owner)?;
        lease.require_original_negative(self)?;
        self.validate_source_only()?;
        let work_end = lane.reserve_promotion_stage_end(&mut self.owner, &original)?;
        *admitted = Some(AdmittedV2TextPromotionStage {
            original,
            physical: physical
                .take()
                .expect("same outside lease checked before final proof"),
            _work_end: work_end,
        });
        Ok(())
    }

    /// Staging supplied Ready only. Reacquire this concrete Current/kernel/lane/source
    /// admission before the selector mutation; a historical stage receipt is insufficient.
    pub(in crate::store) fn admit_text_promotion_commit_into(
        &mut self,
        lane: &V2TextLane,
        ready: &ReadyV2TextPromotionSelection,
        physical: &mut Option<PreparedV2TextPhysicalLease>,
        admitted: &mut Option<AdmittedV2TextPromotionCommit>,
    ) -> Result<(), AppError> {
        let original = ready.observe_original();
        if let Some(existing) = admitted.as_ref() {
            existing.require_same_promotion(&original)?;
            existing.physical.require_original_negative(self)?;
            lane.require_registered(&mut self.owner)?;
            return self.validate_source_only();
        }
        let lease = physical.as_ref().ok_or_else(refused)?;
        lease.validate_under_current_guard(self)?;
        self.owner
            .require_installed_text_writer(self.scope_admission().writer_observation()?)?;
        ready.validate_ready_under_current_guard(self)?;
        lane.require_registered(&mut self.owner)?;
        lease.require_original_negative(self)?;
        self.validate_source_only()?;
        let work_end = lane.reserve_promotion_commit_end(&mut self.owner, &original)?;
        *admitted = Some(AdmittedV2TextPromotionCommit {
            original,
            physical: physical
                .take()
                .expect("same outside lease checked before final proof"),
            _work_end: work_end,
        });
        Ok(())
    }

    /// Successful EMPTY-promotion completion only. Fresh selected Saved O_G and exact
    /// first-stage plan binding close admissions before an outside-writer actual drain.
    /// This is not ordinary WD/WC, owner-dead suspension or a parsed Saved substitution.
    pub(in crate::store) fn close_text_promotion_lane_into(
        &mut self,
        lane: &V2TextLane,
        saved: &SavedV2TextPromotion,
        closed: &mut Option<V2TextPromotionLaneDrain>,
    ) -> Result<(), AppError> {
        saved.validate_selected_under_current_guard(self)?;
        let original = saved.observe_original();
        self.validate_source_only()?;
        lane.close_promotion_into(&mut self.owner, &original, closed)
    }

    /// Reacquire actual Current/selected Saved after the OUTSIDE-writer join. Remove only
    /// the same closed entry and cache BOTH unique caller and registry ownership outside
    /// before Scope's fallible postcheck; their final fee/resource destruction follows it.
    pub(in crate::store) fn retire_text_promotion_lane_into(
        &mut self,
        caller: &mut Option<V2TextLane>,
        saved: &SavedV2TextPromotion,
        drained: &V2TextPromotionLaneDrained,
        retired: &mut Option<RetiredV2TextPromotionLane>,
    ) -> Result<(), AppError> {
        saved.validate_selected_under_current_guard(self)?;
        let original = saved.observe_original();
        self.validate_source_only()?;
        self.owner
            .retire_promotion_lane_into(caller, &original, drained, retired)
    }
}

macro_rules! redacted_debug {
    ($($ty:ident),+ $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_debug!(
    PreparedV2TextPhysicalLease,
    AdmittedV2TextPromotionStage,
    AdmittedV2TextPromotionCommit
);
