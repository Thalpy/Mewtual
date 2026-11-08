//! SAME original first-A warm recovery, with distinct actual read/repair/scratch roles.
//!
//! No parsed WAL, historical Saved or Stage admission can construct these permissions.
//! The concrete physical producer classifies exact authenticated MAIN or strict original
//! pre-MAIN prefix outside writers; each mutation then needs a NEW actual Current edge.
use super::*;
use crate::store::persistence::v2_text::{
    CapturedV2TextRetainRecovery, OriginalV2TextRetainRecoveryObservation,
    PreparedV2TextRetainRecovery, ReadyV2TextRetainRepair, ReadyV2TextRetainScratch,
    V2TextRetainRecoveryCaptureClaim, V2TextRetainWarmRecoveryClassData,
};

/// Active imminent read only. Source is cached before its first File/key; its SAME
/// physical lease and actual writer End cover even a failed pre-admission capture.
/// Claim LAST retains all original work/metadata payers while source=None or a wrapper
/// was dropped. The immutable expected-frame plan contains no dormant native File.
pub(in crate::store) struct PreparedV2TextRetainRecoveryCapture {
    source: Option<CapturedV2TextRetainRecovery>,
    original: OriginalV2TextRetainRecoveryObservation,
    retain: OriginalV2RetainObservation,
    physical: PreparedV2TextPhysicalLease,
    end: Option<V2TextCreateControlWriteEnd>,
    _claim: V2TextRetainRecoveryCaptureClaim,
}

/// In-place loan only: no take, owning Option or source constructor can detach captured
/// Files/key from the active native lease/end. The physical source fills each field
/// immediately before every later fallible read/proof.
pub(in crate::store) struct V2TextRetainRecoverySourceSlot<'capture> {
    source: &'capture mut Option<CapturedV2TextRetainRecovery>,
}
impl V2TextRetainRecoverySourceSlot<'_> {
    pub(in crate::store) fn get(&self) -> Option<&CapturedV2TextRetainRecovery> {
        self.source.as_ref()
    }
    pub(in crate::store) fn get_mut(&mut self) -> Option<&mut CapturedV2TextRetainRecovery> {
        self.source.as_mut()
    }
    pub(in crate::store) fn cache_initial(&mut self, source: CapturedV2TextRetainRecovery) {
        assert!(self.source.is_none(), "same recovery source already cached");
        *self.source = Some(source);
    }
}

/// Complete bounded READ only, not mutation or selection. The worker consumes and drops
/// source/key/OSlease/end/claim before returning its immutable classified Ready DATA.
pub(in crate::store) struct AdmittedV2TextRetainRecoveryRead {
    active: PreparedV2TextRetainRecoveryCapture,
}

macro_rules! effect_precursor {
    ($ty:ident, $ready:ident) => {
        /// One imminent concrete effect precursor, never a general Ready/lease adopter.
        /// The SAME shared source/effect claim precedes OS lease preparation; a second
        /// read or effect cannot duplicate buffers against the existing payment.
        pub(in crate::store) struct $ty {
            original: OriginalV2TextRetainRecoveryObservation,
            retain: OriginalV2RetainObservation,
            physical: PreparedV2TextPhysicalLease,
            // LAST: no File/cleanup follows this resource-only immutable plan claim.
            _claim: V2TextRetainRecoveryCaptureClaim,
        }
        impl $ty {
            pub(in crate::store) fn prepare(
                store: &ServerStore,
                source: &GenesisSourceDataRef<'_>,
                ready: &$ready,
            ) -> Result<Self, AppError> {
                ready.validate_preparation(store, source)?;
                let claim = ready.claim_capture_preparation()?;
                let physical =
                    PreparedV2TextPhysicalLease::prepare(store, source, ready.resources())?;
                ready.validate_preparation(store, source)?;
                Ok(Self {
                    original: ready.observe_original(),
                    retain: ready.original_retain(),
                    physical,
                    _claim: claim,
                })
            }
        }
    };
}
effect_precursor!(PreparedV2TextRetainRepairLease, ReadyV2TextRetainRepair);
effect_precursor!(PreparedV2TextRetainScratchLease, ReadyV2TextRetainScratch);

/// Actual exact-listed NONSELECTED output repair only. It may not select O, sign, create
/// another transaction or overwrite a wrong prefix. The finite worker rechecks full MAIN
/// and each held prefix outside writers before appending its original known suffix.
pub(in crate::store) struct AdmittedV2TextRetainRepair {
    active: PreparedV2TextRetainRepairLease,
    // LAST: all helper-local Files, plan wrappers and physical lease precede this end.
    end: V2TextCreateControlWriteEnd,
}
/// Actual strict proper-prefix first-WAL scratch only. A complete authenticated stage,
/// unknown wrapper/version/context, dependent output or selected mutation is excluded.
pub(in crate::store) struct AdmittedV2TextRetainScratch {
    active: PreparedV2TextRetainScratchLease,
    end: V2TextCreateControlWriteEnd,
}

/// Charged by the paired complete recovery metadata inventory BEFORE any aliases/copies.
/// Source/SDK/query and SAME16MiB work overlap remain independently retained as documented
/// by the physical producer. This constant grants neither memory nor effect authority.
pub(in crate::store) const V2_TEXT_RETAIN_RECOVERY_ADMISSION_LAYOUT_BYTES: usize = 2
    * std::mem::size_of::<PreparedV2TextRetainRecoveryCapture>()
    + std::mem::size_of::<V2TextRetainRecoverySourceSlot<'static>>()
    + std::mem::size_of::<AdmittedV2TextRetainRecoveryRead>()
    + 2 * std::mem::size_of::<PreparedV2TextRetainRepairLease>()
    + 2 * std::mem::size_of::<PreparedV2TextRetainScratchLease>()
    + std::mem::size_of::<AdmittedV2TextRetainRepair>()
    + std::mem::size_of::<AdmittedV2TextRetainScratch>()
    + 8 * 64;

impl PreparedV2TextRetainRecoveryCapture {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        prepared: &PreparedV2TextRetainRecovery,
    ) -> Result<Self, AppError> {
        prepared.validate_preparation(store, source)?;
        let claim = prepared.claim_capture_preparation()?;
        let physical = PreparedV2TextPhysicalLease::prepare(store, source, prepared.resources())?;
        prepared.validate_preparation(store, source)?;
        Ok(Self {
            source: None,
            original: prepared.observe_original(),
            retain: prepared.original_retain(),
            physical,
            end: None,
            _claim: claim,
        })
    }
    fn source_slot(&mut self) -> V2TextRetainRecoverySourceSlot<'_> {
        V2TextRetainRecoverySourceSlot {
            source: &mut self.source,
        }
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn has_source_for_test(&self) -> bool {
        self.source.is_some()
    }
}

impl AdmittedV2TextRetainRecoveryRead {
    pub(in crate::store) fn require_same_recovery(
        &self,
        original: &OriginalV2TextRetainRecoveryObservation,
    ) -> Result<(), AppError> {
        self.active.original.require_same_original(original)
    }
    /// Run only the concrete bounded outside-writer classifier, returning DATA. A
    /// mutable Captured loan would allow safe mem::swap across two different vault
    /// markers and detach live Files from their actual OS lease. Neither this method
    /// nor a callback exposes an owning or mutable source field to its caller.
    pub(in crate::store) fn classify_data(
        &mut self,
        original: &OriginalV2TextRetainRecoveryObservation,
    ) -> Result<
        (
            V2TextRetainWarmRecoveryClassData,
            Option<catcoms_storage::kept::SessionId>,
            usize,
        ),
        AppError,
    > {
        self.require_same_recovery(original)?;
        let source = self
            .active
            .source
            .as_mut()
            .expect("actual recovery source admitted");
        source.require_same_recovery(original)?;
        source.classify_outside_writers()
    }
}
impl AdmittedV2TextRetainRepair {
    pub(in crate::store) fn require_same_recovery(
        &self,
        original: &OriginalV2TextRetainRecoveryObservation,
    ) -> Result<(), AppError> {
        self.active.original.require_same_original(original)
    }
}
impl AdmittedV2TextRetainScratch {
    pub(in crate::store) fn require_same_recovery(
        &self,
        original: &OriginalV2TextRetainRecoveryObservation,
    ) -> Result<(), AppError> {
        self.active.original.require_same_original(original)
    }
}

impl V2TextCurrentGroupGuard<'_, '_> {
    pub(in crate::store) fn admit_text_retain_recovery_read_into(
        &mut self,
        lane: &V2TextCreateLane,
        predecessor: &BoundV2TextCreatePredecessor,
        prepared: &mut PreparedV2TextRetainRecovery,
        capture: &mut Option<PreparedV2TextRetainRecoveryCapture>,
        admitted: &mut Option<AdmittedV2TextRetainRecoveryRead>,
    ) -> Result<(), AppError> {
        let original = prepared.observe_original();
        let retain = prepared.original_retain();
        if let Some(existing) = admitted.as_ref() {
            existing.require_same_recovery(&original)?;
            existing.active.retain.require_same_original(&retain)?;
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
        active.retain.require_same_original(&retain)?;
        active.physical.validate_retain_write_under_guard(self)?;
        predecessor.require_same_lane_original(lane, &retain)?;
        if let Some(end) = &active.end {
            // Same original failed capture retry owns the gate itself. Do not try to
            // reserve a second end or reject against its own writer-active exclusion.
            end.require_same_lane(lane)?;
            lane.require_registered(&mut self.owner)?;
        } else {
            lane.require_workspace_idle(&mut self.owner, &retain)?;
            self.owner.require_text_control_io_quiescent()?;
            self.require_text_control_write_idle_for_query()?;
            prepared.validate_parent_under_current_guard(self)?;
            active.physical.require_retain_write_negative(self)?;
            self.validate_source_only()?;
            active.end = Some(lane.reserve_create_control_write_end(&mut self.owner, &retain)?);
        }
        // SAME conservative gate/end cached before any retained Source File/key opens.
        // Its full-WAL/prefix classification is a separate finite outside-writer read.
        prepared.capture(self, &mut active.source_slot())?;
        active
            .source
            .as_ref()
            .ok_or_else(refused)?
            .validate_under_current_guard(self)?;
        active.physical.require_retain_write_negative(self)?;
        predecessor.require_same_lane_original(lane, &retain)?;
        active
            .end
            .as_ref()
            .ok_or_else(refused)?
            .require_same_lane(lane)?;
        self.validate_source_only()?;
        *admitted = Some(AdmittedV2TextRetainRecoveryRead {
            active: capture.take().expect("same outside active source checked"),
        });
        Ok(())
    }
}

macro_rules! effect_admission {
    ($method:ident, $ready:ident, $lease:ident, $marker:ident, $validate:ident) => {
        impl V2TextCurrentGroupGuard<'_, '_> {
            pub(in crate::store) fn $method(
                &mut self,
                lane: &V2TextCreateLane,
                predecessor: &BoundV2TextCreatePredecessor,
                ready: &$ready,
                physical: &mut Option<$lease>,
                admitted: &mut Option<$marker>,
            ) -> Result<(), AppError> {
                let original = ready.observe_original();
                let retain = ready.original_retain();
                if let Some(existing) = admitted.as_ref() {
                    existing.require_same_recovery(&original)?;
                    existing.active.retain.require_same_original(&retain)?;
                    existing.end.require_same_lane(lane)?;
                    return self.validate_source_only();
                }
                let active = physical.as_ref().ok_or_else(refused)?;
                active.original.require_same_original(&original)?;
                active.retain.require_same_original(&retain)?;
                active.physical.validate_retain_write_under_guard(self)?;
                predecessor.require_same_lane_original(lane, &retain)?;
                lane.require_workspace_idle(&mut self.owner, &retain)?;
                self.owner.require_text_control_io_quiescent()?;
                self.require_text_control_write_idle_for_query()?;
                ready.$validate(self)?;
                // Full authenticated classification came from the actual read issuer;
                // actual original parent/held names are fresh here. Finite work still
                // repeats all full MAIN/prefix checks before any append/unlink outside AD.
                predecessor.require_same_lane_original(lane, &retain)?;
                lane.require_workspace_idle(&mut self.owner, &retain)?;
                active.physical.require_retain_write_negative(self)?;
                self.owner.require_text_control_io_quiescent()?;
                self.validate_source_only()?;
                let end = lane.reserve_create_control_write_end(&mut self.owner, &retain)?;
                *admitted = Some($marker {
                    active: physical
                        .take()
                        .expect("same imminent typed precursor checked"),
                    end,
                });
                Ok(())
            }
        }
    };
}
effect_admission!(
    admit_text_retain_repair_into,
    ReadyV2TextRetainRepair,
    PreparedV2TextRetainRepairLease,
    AdmittedV2TextRetainRepair,
    validate_repair_under_current_guard
);
effect_admission!(
    admit_text_retain_scratch_into,
    ReadyV2TextRetainScratch,
    PreparedV2TextRetainScratchLease,
    AdmittedV2TextRetainScratch,
    validate_scratch_under_current_guard
);

macro_rules! redacted_recovery_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_recovery_debug!(
    PreparedV2TextRetainRecoveryCapture,
    V2TextRetainRecoverySourceSlot<'_>,
    AdmittedV2TextRetainRecoveryRead,
    PreparedV2TextRetainRepairLease,
    PreparedV2TextRetainScratchLease,
    AdmittedV2TextRetainRepair,
    AdmittedV2TextRetainScratch,
);
