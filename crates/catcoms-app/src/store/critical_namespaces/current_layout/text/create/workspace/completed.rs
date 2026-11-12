//! DISTINCT completed-WC predecessor for a subsequent founder Create.
//!
//! Clear continues to mean every WAL is absent. This family instead requires the
//! exact completed WC and selected public/private parent of the SAME live resident.
//! It grants neither token-miss authority from DATA nor A/selector/sign permission.
use super::*;
use crate::store::persistence::v2_text::{
    CapturedV2TextCompletedWorkspace, OriginalV2TextCompletedWorkspaceObservation,
    PreparedV2TextCompletedWorkspaceCapture, ReadyV2TextCompletedWorkspace,
    V2TextCompletedWorkspaceCaptureClaim,
};
use crate::store::persistence::{
    OriginalV2TextResidentObservation, PreparedV2TextResidentProjection,
};

/// Source FIRST, then the SAME physical mount lease, actual lane End and claim
/// LAST. Even an unstarted/failed capture is structurally paid and must drain its
/// Files before the OS lease, real cap, and central one-workspace allowance end.
pub(in crate::store) struct PreparedV2TextCompletedWorkspaceReadCapture {
    source: Option<CapturedV2TextCompletedWorkspace>,
    original: OriginalV2RetainObservation,
    resident: OriginalV2TextResidentObservation,
    physical: PreparedV2TextPhysicalLease,
    end: Option<V2TextCreateWorkspaceEnd>,
    _claim: V2TextCompletedWorkspaceCaptureClaim,
}

/// This fill loan is constructible ONLY by the actual native capture owner. The
/// hard-coded physical capture may fill it; no owning Option/take escapes.
pub(in crate::store) struct V2TextCompletedWorkspaceSourceSlot<'capture> {
    source: &'capture mut Option<CapturedV2TextCompletedWorkspace>,
}
impl V2TextCompletedWorkspaceSourceSlot<'_> {
    pub(in crate::store) fn get(&self) -> Option<&CapturedV2TextCompletedWorkspace> {
        self.source.as_ref()
    }
    pub(in crate::store) fn get_mut(&mut self) -> Option<&mut CapturedV2TextCompletedWorkspace> {
        self.source.as_mut()
    }
    pub(in crate::store) fn cache_initial(&mut self, source: CapturedV2TextCompletedWorkspace) {
        assert!(
            self.source.is_none(),
            "same completed workspace already captured"
        );
        *self.source = Some(source);
    }
}

/// One finite readonly inspection. There is no mutable Captured getter: two
/// different-vault markers cannot swap owning Files away from their actual lease.
pub(in crate::store) struct AdmittedV2TextCompletedWorkspaceRead {
    active: PreparedV2TextCompletedWorkspaceReadCapture,
}

/// A fresh logical native binding to one actual next request and completed parent.
/// The physical next-A cell must OWN its exact original predecessor before any
/// WAL replacement; after that admission, the cached same-A continuation wins.
/// This type alone cannot reopen an obsolete WC or authorize an A/sign/key cut.
pub(in crate::store) struct BoundV2TextCompletedCreatePredecessor {
    binding: V2TextCreateBinding,
    completed: OriginalV2TextCompletedWorkspaceObservation,
    resident: OriginalV2TextResidentObservation,
}

/// NEW completed-parent metadata1MiB names every simultaneous native wrapper. Its own
/// 16MiB work, paid token/read/lane, resident1MiB/domain and SDK fees are additional.
pub(in crate::store) const V2_TEXT_COMPLETED_WORKSPACE_ADMISSION_LAYOUT_BYTES: usize = 2
    * std::mem::size_of::<PreparedV2TextCompletedWorkspaceReadCapture>()
    + std::mem::size_of::<V2TextCompletedWorkspaceSourceSlot<'static>>()
    + std::mem::size_of::<AdmittedV2TextCompletedWorkspaceRead>()
    + 2 * std::mem::size_of::<BoundV2TextCompletedCreatePredecessor>()
    + 4 * 64;

impl PreparedV2TextCompletedWorkspaceReadCapture {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        prepared: &PreparedV2TextCompletedWorkspaceCapture,
    ) -> Result<Self, AppError> {
        prepared.validate_preparation(store, source)?;
        // The whole physical plan already holds the request's central maximum-one
        // workspace claim. Acquire its distinct active cap BEFORE a lease or shell.
        let claim = prepared.claim_capture_preparation()?;
        let physical = PreparedV2TextPhysicalLease::prepare(store, source, prepared.resources())?;
        prepared.validate_preparation(store, source)?;
        Ok(Self {
            source: None,
            original: prepared.observe_original(),
            resident: prepared.original_resident(),
            physical,
            end: None,
            _claim: claim,
        })
    }

    fn source_slot(&mut self) -> V2TextCompletedWorkspaceSourceSlot<'_> {
        V2TextCompletedWorkspaceSourceSlot {
            source: &mut self.source,
        }
    }

    #[cfg(all(test, windows))]
    pub(in crate::store) fn has_source_for_test(&self) -> bool {
        self.source.is_some()
    }
}

impl AdmittedV2TextCompletedWorkspaceRead {
    pub(in crate::store) fn require_same_work(
        &self,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.active.original.require_same_original(original)
    }

    /// Bounded outside-writer DATA work. It reaches only this marker's real
    /// captured source. Stop after admission does not prevent finite cleanup or
    /// exact cached DATA, but no new positive resident/work permission is renewed.
    pub(in crate::store) fn prove_completed_data(
        &mut self,
        original: &OriginalV2RetainObservation,
    ) -> Result<[u8; 32], AppError> {
        self.require_same_work(original)?;
        self.active
            .source
            .as_mut()
            .expect("actual completed workspace source admitted")
            .prove_completed_for_original_data(original)
    }
}

impl BoundV2TextCompletedCreatePredecessor {
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
    /// Pure exact association for the Physical next-A constructor. Neither this
    /// observation nor parsed matching WC bytes can construct the closed role.
    pub(in crate::store) fn require_completed_original(
        &self,
        completed: &OriginalV2TextCompletedWorkspaceObservation,
    ) -> Result<(), AppError> {
        self.completed.require_same_original(completed)
    }
    pub(in crate::store) fn require_same_resident(
        &self,
        resident: &OriginalV2TextResidentObservation,
    ) -> Result<(), AppError> {
        self.resident.require_same_original(resident)
    }

    /// Cached domain/source negative only; identity DATA remains comparable after
    /// Stop, but any new SDK/effect admission must reject the original stop.
    pub(in crate::store) fn validate_negative(&self) -> Result<(), AppError> {
        self.resident.validate_negative()
    }
}

impl V2TextCurrentGroupGuard<'_, '_> {
    pub(in crate::store) fn admit_text_completed_workspace_read_into(
        &mut self,
        lane: &V2TextCreateLane,
        prepared: &mut PreparedV2TextCompletedWorkspaceCapture,
        query: &PreparedV2TextTokenQuery,
        resident: &PreparedV2TextResidentProjection,
        capture: &mut Option<PreparedV2TextCompletedWorkspaceReadCapture>,
        admitted: &mut Option<AdmittedV2TextCompletedWorkspaceRead>,
    ) -> Result<(), AppError> {
        // Same runtime/read-body BEFORE owned proof allocation or any File/account
        // work. Resident/source group labels can never substitute for identity.
        prepared
            .resources()
            .require_same_original(self.resources())?;
        let original = prepared.observe_original();
        let resident_original = resident.observe_original();
        prepared
            .original_resident()
            .require_same_original(&resident_original)?;
        resident.require_current_source_under_guard(self)?;
        if let Some(existing) = admitted.as_ref() {
            existing.require_same_work(&original)?;
            existing
                .active
                .resident
                .require_same_original(&resident_original)?;
            existing
                .active
                .end
                .as_ref()
                .ok_or_else(refused)?
                .require_same_lane(lane)?;
            existing
                .active
                .physical
                .require_workspace_capture_negative(self)?;
            return self.validate_source_only();
        }
        let active = capture.as_mut().ok_or_else(refused)?;
        active.original.require_same_original(&original)?;
        active.resident.require_same_original(&resident_original)?;
        active
            .physical
            .validate_workspace_capture_under_guard(self)?;
        if let Some(end) = &active.end {
            end.require_same_lane(lane)?;
            lane.require_registered(&mut self.owner)?;
        } else {
            lane.require_workspace_idle(&mut self.owner, &original)?;
        }
        // Complete exact selected-parent/query/resident proof BEFORE retained
        // source opens. All local ephemeral comparison Files drain before return.
        prepared.validate_parent_under_current_guard(self, query, resident)?;
        if active.end.is_none() {
            active.end = Some(lane.reserve_workspace_capture_end(&mut self.owner, &original)?);
        }
        prepared.capture(self, &mut active.source_slot())?;
        active
            .source
            .as_ref()
            .ok_or_else(refused)?
            .validate_under_current_guard(self)?;
        active.physical.require_workspace_capture_negative(self)?;
        active.resident.validate_negative()?;
        resident.require_current_source_under_guard(self)?;
        lane.require_registered(&mut self.owner)?;
        active
            .end
            .as_ref()
            .ok_or_else(refused)?
            .require_same_lane(lane)?;
        self.validate_source_only()?;
        *admitted = Some(AdmittedV2TextCompletedWorkspaceRead {
            active: capture
                .take()
                .expect("same outside completed-workspace owner"),
        });
        Ok(())
    }

    pub(in crate::store) fn admit_text_completed_create_predecessor_into(
        &mut self,
        lane: &V2TextCreateLane,
        ready: &ReadyV2TextCompletedWorkspace,
        query: &PreparedV2TextTokenQuery,
        resident: &PreparedV2TextResidentProjection,
        original: &OriginalV2RetainObservation,
        retained: &mut Option<BoundV2TextCompletedCreatePredecessor>,
    ) -> Result<(), AppError> {
        ready.resources().require_same_original(self.resources())?;
        ready.observe_original().require_same_original(original)?;
        let resident_original = resident.observe_original();
        ready
            .original_resident()
            .require_same_original(&resident_original)?;
        resident.require_current_source_under_guard(self)?;
        // These fixed observations are gathered BEFORE final physical/source proof.
        let completed = ready.observe_completed_original();
        if let Some(existing) = retained.as_ref() {
            existing.require_same_lane_original(lane, original)?;
            existing.require_completed_original(&completed)?;
            existing.require_same_resident(&resident_original)?;
            return self.validate_source_only();
        }
        lane.require_workspace_idle(&mut self.owner, original)?;
        self.require_text_control_write_idle_for_query()?;
        ready.validate_completed_under_current_guard(self, query, resident)?;
        // Native/source/domain/actual finite IO checks only AFTER final proof.
        self.owner.require_text_control_io_quiescent()?;
        lane.require_workspace_idle(&mut self.owner, original)?;
        resident_original.validate_negative()?;
        resident.require_current_source_under_guard(self)?;
        self.validate_source_only()?;
        let binding = lane.bind_create_original(&mut self.owner, original)?;
        // No fallible action/destructor between the logical first binding and its
        // outside owning cache. A lost return cannot permit a competing request.
        *retained = Some(BoundV2TextCompletedCreatePredecessor {
            binding,
            completed,
            resident: resident_original,
        });
        Ok(())
    }
}

macro_rules! completed_workspace_debug {
    ($($ty:ty),+ $(,)?) => {$(impl std::fmt::Debug for $ty {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+};
}
completed_workspace_debug!(
    PreparedV2TextCompletedWorkspaceReadCapture,
    V2TextCompletedWorkspaceSourceSlot<'_>,
    AdmittedV2TextCompletedWorkspaceRead,
    BoundV2TextCompletedCreatePredecessor,
);
