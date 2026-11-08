//! Concrete Current workspace read and native closed logical predecessor.
//!
//! ReadyClear is a physical DATA issuer, not an SDK or selector permit. The native
//! predecessor below is constructible only after the complete real Current/selected/
//! query/absence/control-drain proof. Physical A stage, commit and signing stay distinct.
use super::*;
mod completed;
pub(in crate::store) use completed::{
    AdmittedV2TextCompletedWorkspaceRead, BoundV2TextCompletedCreatePredecessor,
    PreparedV2TextCompletedWorkspaceReadCapture, V2TextCompletedWorkspaceSourceSlot,
    V2_TEXT_COMPLETED_WORKSPACE_ADMISSION_LAYOUT_BYTES,
};

use crate::store::owner_evidence::canonical::text_lane::{
    V2TextCreateBinding, V2TextCreateWorkspaceEnd,
};
use crate::store::persistence::v2_text::{
    CapturedV2TextWorkspace, OriginalV2RetainObservation, PreparedV2TextTokenQuery,
    PreparedV2TextWorkspaceCapture, ReadyV2TextWorkspaceClear, V2TextWorkspaceCaptureClaim,
};

/// Active imminent read only. Source/native payload is FIRST; same-vault lease and actual
/// control end follow; the non-Clone precursor claim keeps both payers alive LAST, even
/// while source=None or after the original prepared workspace/producer was dropped.
pub(in crate::store) struct PreparedV2TextWorkspaceReadCapture {
    source: Option<CapturedV2TextWorkspace>,
    original: OriginalV2RetainObservation,
    physical: PreparedV2TextPhysicalLease,
    end: Option<V2TextCreateWorkspaceEnd>,
    _claim: V2TextWorkspaceCaptureClaim,
}

pub(in crate::store) struct V2TextWorkspaceSourceSlot<'capture> {
    source: &'capture mut Option<CapturedV2TextWorkspace>,
}
impl V2TextWorkspaceSourceSlot<'_> {
    pub(in crate::store) fn get(&self) -> Option<&CapturedV2TextWorkspace> {
        self.source.as_ref()
    }
    pub(in crate::store) fn get_mut(&mut self) -> Option<&mut CapturedV2TextWorkspace> {
        self.source.as_mut()
    }
    pub(in crate::store) fn cache_initial(&mut self, source: CapturedV2TextWorkspace) {
        assert!(
            self.source.is_none(),
            "same workspace source already cached"
        );
        *self.source = Some(source);
    }
}

/// One admitted bounded READ, never a stage, O mutation, signer or deletion. Physical
/// run_read consumes the whole owner and closes ALL helper/source Files/key/OS/end on
/// every outcome before returning. Cached ReadyClear retains DATA/payments only.
pub(in crate::store) struct AdmittedV2TextWorkspaceRead {
    active: PreparedV2TextWorkspaceReadCapture,
}

/// Actual native closed logical predecessor. It provides SAME-request serialization,
/// not a reusable SDK capability or an A/I/S/B write/sign admission. The Root's owning
/// producer must still retain its original request/packet and perform its native/Scope
/// admission at the real key boundary; this type cannot adopt arbitrary Action bytes.
pub(in crate::store) struct BoundV2TextCreatePredecessor {
    binding: V2TextCreateBinding,
}

/// Charged by the physical workspace's transaction-metadata1.25MiB, in addition to its
/// same16MiB concrete open/work inventory, original query/read/SDK and lane payments.
pub(in crate::store) const V2_TEXT_WORKSPACE_ADMISSION_LAYOUT_BYTES: usize = 2
    * std::mem::size_of::<PreparedV2TextWorkspaceReadCapture>()
    + std::mem::size_of::<V2TextWorkspaceSourceSlot<'static>>()
    + std::mem::size_of::<AdmittedV2TextWorkspaceRead>()
    + std::mem::size_of::<BoundV2TextCreatePredecessor>()
    + 4 * 64;

impl PreparedV2TextWorkspaceReadCapture {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        prepared: &PreparedV2TextWorkspaceCapture,
    ) -> Result<Self, AppError> {
        prepared.validate_preparation(store, source)?;
        // The physical plan already owns Root's maximum-one WHOLE-workspace claim.
        // This distinct claim also bounds inactive/active Source precursors to one.
        let claim = prepared.claim_capture_preparation()?;
        let physical = PreparedV2TextPhysicalLease::prepare(store, source, prepared.resources())?;
        prepared.validate_preparation(store, source)?;
        Ok(Self {
            source: None,
            original: prepared.observe_original(),
            physical,
            end: None,
            _claim: claim,
        })
    }

    fn source_slot(&mut self) -> V2TextWorkspaceSourceSlot<'_> {
        V2TextWorkspaceSourceSlot {
            source: &mut self.source,
        }
    }

    #[cfg(all(test, windows))]
    pub(in crate::store) fn has_source_for_test(&self) -> bool {
        self.source.is_some()
    }
}

impl AdmittedV2TextWorkspaceRead {
    pub(in crate::store) fn require_same_work(
        &self,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.active.original.require_same_original(original)
    }

    /// Keep owning Files coupled to THIS marker's vault lease and End. A mutable
    /// Captured loan would permit swapping sources between two different vault markers.
    /// Only the bounded immutable result may escape this nonextracting operation.
    pub(in crate::store) fn prove_clear_data(
        &mut self,
        original: &OriginalV2RetainObservation,
    ) -> Result<[u8; 32], AppError> {
        self.require_same_work(original)?;
        self.active
            .source
            .as_mut()
            .expect("real workspace source admitted")
            .prove_clear_for_original_data(original)
    }
}

impl BoundV2TextCreatePredecessor {
    pub(in crate::store) fn require_same_original(
        &self,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.binding.require_same_original(original)
    }

    pub(in crate::store) fn require_same_lane_original(
        &self,
        lane: &V2TextCreateLane,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.binding.require_same_lane_original(lane, original)
    }
}

impl V2TextCurrentGroupGuard<'_, '_> {
    pub(in crate::store) fn admit_text_workspace_read_into(
        &mut self,
        lane: &V2TextCreateLane,
        prepared: &mut PreparedV2TextWorkspaceCapture,
        query: &PreparedV2TextTokenQuery,
        capture: &mut Option<PreparedV2TextWorkspaceReadCapture>,
        admitted: &mut Option<AdmittedV2TextWorkspaceRead>,
    ) -> Result<(), AppError> {
        let original = prepared.observe_original();
        if let Some(existing) = admitted.as_ref() {
            existing.require_same_work(&original)?;
            existing
                .active
                .end
                .as_ref()
                .ok_or_else(refused)?
                .require_same_lane(lane)?;
            return self.validate_source_only();
        }
        let active = capture.as_mut().ok_or_else(refused)?;
        active.original.require_same_original(&original)?;
        active
            .physical
            .validate_workspace_capture_under_guard(self)?;
        if let Some(end) = &active.end {
            // Same failed-capture retry retains its own actual finite end; it must not
            // require idle against itself or reserve an additional unstarted owner.
            end.require_same_lane(lane)?;
            lane.require_registered(&mut self.owner)?;
        } else {
            lane.require_workspace_idle(&mut self.owner, &original)?;
        }
        prepared.validate_parent_under_current_guard(self, query)?;
        if active.end.is_none() {
            active.end = Some(lane.reserve_workspace_capture_end(&mut self.owner, &original)?);
        }
        // No retained File/key can open before the SAME lane's actual end was cached.
        // A late capture Err keeps source/end/OSlease in this outside owning bundle.
        prepared.capture(self, &mut active.source_slot())?;
        active
            .source
            .as_ref()
            .ok_or_else(refused)?
            .validate_under_current_guard(self)?;
        active.physical.require_workspace_capture_negative(self)?;
        lane.require_registered(&mut self.owner)?;
        active
            .end
            .as_ref()
            .ok_or_else(refused)?
            .require_same_lane(lane)?;
        self.validate_source_only()?;
        *admitted = Some(AdmittedV2TextWorkspaceRead {
            active: capture
                .take()
                .expect("same outside workspace bundle checked"),
        });
        Ok(())
    }

    pub(in crate::store) fn admit_text_create_predecessor_into(
        &mut self,
        lane: &V2TextCreateLane,
        ready: &ReadyV2TextWorkspaceClear,
        query: &PreparedV2TextTokenQuery,
        original: &OriginalV2RetainObservation,
        retained: &mut Option<BoundV2TextCreatePredecessor>,
    ) -> Result<(), AppError> {
        ready.observe_original().require_same_original(original)?;
        if let Some(existing) = retained.as_ref() {
            existing.require_same_lane_original(lane, original)?;
            lane.require_registered(&mut self.owner)?;
            return self.validate_source_only();
        }
        // Even the bounded ephemeral Clear comparison may not open obsolete controls
        // while an actual same-lane finite worker is still outside these writers.
        lane.require_workspace_idle(&mut self.owner, original)?;
        ready.validate_clear_under_current_guard(self, query)?;
        self.owner.require_text_control_io_quiescent()?;
        lane.require_workspace_idle(&mut self.owner, original)?;
        self.validate_source_only()?;
        let binding = lane.bind_create_original(&mut self.owner, original)?;
        // No fallible operation or destructor between actual immutable binding and the
        // OUTSIDE closed-role cache. Scope's final late failure cannot lose this owner.
        *retained = Some(BoundV2TextCreatePredecessor { binding });
        Ok(())
    }
}

macro_rules! redacted_workspace_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_workspace_debug!(
    PreparedV2TextWorkspaceReadCapture,
    V2TextWorkspaceSourceSlot<'_>,
    AdmittedV2TextWorkspaceRead,
    BoundV2TextCreatePredecessor,
);
