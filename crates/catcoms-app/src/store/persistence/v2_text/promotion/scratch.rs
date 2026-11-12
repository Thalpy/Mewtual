//! Explicit PresentUncommitted first-seed workspace cleanup, not error-as-absence.
//!
//! Only the purpose/version-specific `text.seed.stage.v1`, before MAIN or ANY dependent,
//! can enter this role. A valid authenticated frame of ANY supported/unsupported plaintext
//! meaning is preserved; only bounded unauthenticated/torn temporary bytes are eligible.
//! Fresh Current/known original genesis/registered-plan/quiescence admission is separate.
use super::super::control::{inspect_seed_stage_mac_data, TextControlMacInspectionData};
use super::recovery::{
    self, require_absent, require_control_name, HeldRecoveryControl, HeldRecoveryDirectory,
    RecoveryPlan,
};
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextPromotionScratch, V2TextPromotionScratchSourceSlot,
};
use catcoms_storage::kept::SessionId;
use std::fs;

/// No Files/lease in dormant debt. One independent paid plan is retained through every
/// actual active source/marker and any lost cleanup result. Root owns ONE recovery phase.
pub(in crate::store) struct PreparedV2TextPromotionScratch {
    unlinked: Option<Arc<ActuallyUnlinkedSeedScratch>>,
    cleaned: Option<SavedV2TextPromotionScratch>,
    // DATA handoff bookkeeping only, not cleanup/Current/IO authority.
    installed: bool,
    #[cfg(all(test, windows))]
    after_capture: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    fail_after_unlink: bool,
    plan: Arc<RecoveryPlan>,
}
/// Produced ONLY immediately after the real exact-name unlink succeeds. This is necessary
/// when the following parent barrier fails: absence alone could not justify a successful
/// cleanup or resetting the original stage fact on retry. It grants no new unlink.
struct ActuallyUnlinkedSeedScratch {
    original: OriginalV2TextPromotionObservation,
    removed_id: SessionId,
    raw_commitment: [u8; 32],
}
struct SavedV2TextPromotionScratch {
    removed: Arc<ActuallyUnlinkedSeedScratch>,
}

impl PreparedV2TextPromotionScratch {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        original: &PreparedV2TextPromotion,
    ) -> Result<Self, AppError> {
        if !original.slots.permits_first_seed_scratch_data() {
            return Err(refused());
        }
        Ok(Self {
            unlinked: None,
            cleaned: None,
            installed: false,
            #[cfg(all(test, windows))]
            after_capture: None,
            #[cfg(all(test, windows))]
            fail_after_unlink: false,
            plan: RecoveryPlan::from_prepared(store, source, original)?,
        })
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        self.plan.resources()
    }
    pub(in crate::store) fn recovery_resources(
        &self,
    ) -> recovery::V2TextPromotionRecoveryResources {
        self.plan.payment.clone()
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextPromotionObservation {
        self.plan.original.clone()
    }
    /// DATA bookkeeping only. Actual unlink plus the completed parent barrier issued the
    /// cached receipt; this observation cannot create IO/Current/scratch permission.
    pub(in crate::store) fn has_completed_cleanup_data(&self) -> bool {
        self.cleaned.is_some() || self.installed
    }
    pub(in crate::store) fn validate_preparation(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        if self.installed {
            return Err(refused());
        }
        self.plan.validate_account()?;
        self.resources().validate_original(store, source)
    }
    pub(in crate::store) fn capture(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        slot: &mut V2TextPromotionScratchSourceSlot<'_>,
    ) -> Result<(), AppError> {
        if self.installed {
            return Err(refused());
        }
        self.plan.require_guard(guard)?;
        if slot.get().is_none() {
            slot.cache_initial(CapturedV2TextPromotionScratch::empty(
                self.plan.clone(),
                self.unlinked.clone(),
            ));
        }
        let source = slot.get_mut().ok_or_else(refused)?;
        source
            .plan
            .original
            .require_same_original(&self.observe_original())?;
        source.fill(guard)?;
        #[cfg(all(test, windows))]
        if let Some(hook) = self.after_capture.take() {
            hook();
        }
        source.validate_under_current_guard(guard)
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_after_capture_files_for_test(
        &mut self,
        hook: Box<dyn FnOnce() + Send>,
    ) {
        self.after_capture = Some(hook);
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn fail_after_unlink_for_test(&mut self) {
        self.fail_after_unlink = true;
    }
    /// The affine marker retains the exact captured source BEFORE its same-Store lease.
    /// Its actual WorkEnd runs only after both local IO and captured File cleanup drain.
    pub(in crate::store) fn run(
        &mut self,
        admitted: AdmittedV2TextPromotionScratch,
    ) -> Result<(), AppError> {
        admitted.require_same_promotion(&self.observe_original())?;
        self.plan.validate_account()?;
        let source = admitted.source();
        source
            .plan
            .original
            .require_same_original(&self.observe_original())?;
        source.require_current_data()?;
        if self.unlinked.is_none() {
            let stage = source.present_stage()?;
            require_control_name(&self.plan.paths.seed_stage, stage)?;
            let removed_id = stage.id()?;
            let raw_commitment = *blake3::hash(stage.raw()?).as_bytes();
            // Only this purpose/version path is reachable. No MAIN, head.stage, public
            // history, unrelated private page or caller filename is accepted.
            fs::remove_file(&self.plan.paths.seed_stage).map_err(recovery::io)?;
            // Cache the ACTUAL syscall outcome before any fallible post-unlink barrier.
            self.unlinked = Some(Arc::new(ActuallyUnlinkedSeedScratch {
                original: self.observe_original(),
                removed_id,
                raw_commitment,
            }));
            #[cfg(all(test, windows))]
            if std::mem::take(&mut self.fail_after_unlink) {
                return Err(AppError::Io(
                    "injected post-unlink parent-barrier failure".into(),
                ));
            }
        } else {
            self.unlinked
                .as_ref()
                .ok_or_else(refused)?
                .original
                .require_same_original(&self.observe_original())?;
        }
        require_absent(&self.plan.paths.seed_stage)?;
        work::sync_parent(&self.plan.target, self.plan.target_id)?;
        source.target.require_name(&self.plan.target)?;
        require_absent(&self.plan.paths.seed_stage)?;
        if self.cleaned.is_none() {
            self.cleaned = Some(SavedV2TextPromotionScratch {
                removed: self
                    .unlinked
                    .as_ref()
                    .expect("actual unlink cached before barrier")
                    .clone(),
            });
        }
        drop(admitted);
        Ok(())
    }
    /// Clear only the same original prepared stage's negative file fact, AFTER actual
    /// unlink+parent barrier. It does not reseal, erase history, select O or admit IO. The
    /// normal stage path needs a NEW Current/lane/lease admission before writing SAME bytes.
    pub(in crate::store) fn install_cleanup_into(
        &mut self,
        original: &mut PreparedV2TextPromotion,
    ) -> Result<(), AppError> {
        if self.installed {
            return self
                .plan
                .original
                .require_same_original(&original.observe_original());
        }
        self.plan.require_prepared(original)?;
        let saved = self.cleaned.as_ref().ok_or_else(refused)?;
        saved
            .removed
            .original
            .require_same_original(&original.observe_original())?;
        // The commitment is retained through completion for diagnostic/retry identity; it
        // is DATA, not permission. Compare the genuine recorded removal with this phase.
        let pending = self.unlinked.as_ref().ok_or_else(refused)?;
        if saved.removed.raw_commitment != pending.raw_commitment
            || saved.removed.removed_id != pending.removed_id
        {
            return Err(refused());
        }
        original.slots.clear_seed_scratch_data()?;
        self.installed = true;
        Ok(())
    }
}

enum ScratchStage {
    /// Cached before opening the first stage File. Mutation permission is NOT issued here.
    Present(HeldRecoveryControl),
    /// Exact same-process unlink outcome, never a NotFound/decode fallback.
    Removed(Arc<ActuallyUnlinkedSeedScratch>),
}

/// Read closure lives ONLY in the coupled active-capture bundle. On every failure its
/// cached Files drop outside AD BEFORE that bundle's physical lease; no take API detaches it.
pub(in crate::store) struct CapturedV2TextPromotionScratch {
    target: HeldRecoveryDirectory,
    parent: HeldRecoveryControl,
    stage: ScratchStage,
    plan: Arc<RecoveryPlan>,
}
impl CapturedV2TextPromotionScratch {
    fn empty(plan: Arc<RecoveryPlan>, removed: Option<Arc<ActuallyUnlinkedSeedScratch>>) -> Self {
        Self {
            target: HeldRecoveryDirectory::empty(),
            parent: HeldRecoveryControl::empty(),
            stage: match removed {
                Some(r) => ScratchStage::Removed(r),
                None => ScratchStage::Present(HeldRecoveryControl::empty()),
            },
            plan,
        }
    }
    fn present_stage(&self) -> Result<&HeldRecoveryControl, AppError> {
        match &self.stage {
            ScratchStage::Present(stage) => Ok(stage),
            ScratchStage::Removed(_) => Err(refused()),
        }
    }
    fn fill(&mut self, guard: &V2TextCurrentGroupGuard<'_, '_>) -> Result<(), AppError> {
        self.plan.require_guard(guard)?;
        self.target.capture(&self.plan.target)?;
        if self.target.id()? != self.plan.target_id {
            return Err(refused());
        }
        self.parent.capture(&self.plan.paths.head, 4136)?;
        if self.parent.raw()? != self.plan.original_head.as_slice() {
            return Err(refused());
        }
        self.require_no_dependents()?;
        match &mut self.stage {
            ScratchStage::Present(stage) => {
                stage.capture(&self.plan.paths.seed_stage, 32768 + 40)?
            }
            ScratchStage::Removed(removed) => {
                removed
                    .original
                    .require_same_original(&self.plan.original)?;
                require_absent(&self.plan.paths.seed_stage)?;
            }
        }
        Ok(())
    }
    fn require_no_dependents(&self) -> Result<(), AppError> {
        require_absent(&self.plan.paths.seed)?;
        require_absent(&self.plan.paths.pages)?;
        require_absent(&self.plan.paths.head_stage)?;
        require_absent(&self.plan.paths.wal)?;
        require_absent(&self.plan.paths.wal_stage)
    }
    fn require_current_data(&self) -> Result<(), AppError> {
        self.target.require_name(&self.plan.target)?;
        self.parent.require_unchanged(&self.plan.paths.head)?;
        if self.parent.raw()? != self.plan.original_head.as_slice() {
            return Err(refused());
        }
        self.require_no_dependents()?;
        match &self.stage {
            ScratchStage::Present(stage) => stage.require_unchanged(&self.plan.paths.seed_stage)?,
            ScratchStage::Removed(removed) => {
                removed
                    .original
                    .require_same_original(&self.plan.original)?;
                require_absent(&self.plan.paths.seed_stage)?;
            }
        }
        self.target.require_name(&self.plan.target)
    }
    pub(in crate::store) fn validate_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.plan.require_guard(guard)?;
        self.require_current_data()?;
        if let ScratchStage::Present(stage) = &self.stage {
            let key = Zeroizing::new(guard.store().keys.db_key()?);
            match inspect_seed_stage_mac_data(&key, stage.raw()?)? {
                TextControlMacInspectionData::Unauthenticated => {}
                // Preserve complete/authenticated unknown versions, contexts and kinds.
                // A failed full parser is NEVER blanket scratch-disposal permission.
                TextControlMacInspectionData::Authenticated => return Err(refused()),
            }
        }
        work::require_exact_inventory(guard.directory_data().0, &["pin.bin"], false)?;
        let names = [
            "origin",
            "source.seed",
            "source.bin",
            "records.bin",
            "manifest.bin",
            "head.bin",
            "text.seed.stage.v1",
        ];
        work::require_exact_inventory(
            &self.plan.target,
            &names[..if matches!(self.stage, ScratchStage::Present(_)) {
                7
            } else {
                6
            }],
            false,
        )?;
        guard.validate_original_namespace()?;
        self.target.require_name(&self.plan.target)?;
        require_control_name(&self.plan.paths.head, &self.parent)?;
        match &self.stage {
            ScratchStage::Present(stage) => {
                require_control_name(&self.plan.paths.seed_stage, stage)?
            }
            ScratchStage::Removed(_) => require_absent(&self.plan.paths.seed_stage)?,
        }
        self.require_no_dependents()?;
        self.target.require_name(&self.plan.target)?;
        guard.validate_source_only()
    }
}

impl std::fmt::Debug for PreparedV2TextPromotionScratch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2TextPromotionScratch")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for CapturedV2TextPromotionScratch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapturedV2TextPromotionScratch")
            .finish_non_exhaustive()
    }
}
