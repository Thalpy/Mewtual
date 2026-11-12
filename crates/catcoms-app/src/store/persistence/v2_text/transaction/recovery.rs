//! SAME original Cell warm recovery, not cold SDK import or a general filesystem repair.
//!
//! One original Cell claim funds the complete simultaneous expected-WAL/read/AEAD/body
//! inventory. Distinct actual read and mutation markers own the real physical lease/write
//! End. A complete original authenticated MAIN may repair only its named NONSELECTED
//! outputs. A strict original first-stage prefix with no dependents has a separate scratch
//! role. Unknown/corrupt/foreign states are preserved; errors never classify absence.
use super::work::io as native;
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextRetainRecoveryRead, AdmittedV2TextRetainRepair, AdmittedV2TextRetainScratch,
    V2TextRetainRecoverySourceSlot, V2_TEXT_RETAIN_RECOVERY_ADMISSION_LAYOUT_BYTES,
};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;

mod effect;
mod io;
mod source;
#[cfg(all(test, windows))]
pub(in crate::store) use effect::V2TextRetainRecoveryTestCut;
use io::{HeldRetainRecoveryControl, HeldRetainRecoveryDirectory};
pub(in crate::store) use source::CapturedV2TextRetainRecovery;

// Retain WORK base + maximally sized Carry +29 packed entries + actual first-A object
// allowance + envelope. A foreign larger file is preserved before any large allocation.
const MAX_RECOVERY_WAL_FRAME: usize =
    3686 + super::super::retirement::MAX_RETIREMENT_CARRY_BYTES + 29 * 42 + 548084 + 40;
const RECOVERY_WAL_BUFFER_BYTES: usize = 7 * (MAX_RECOVERY_WAL_FRAME + 1) + 64 * 1024;
// The original work invoice explicitly charges five general2MiB WAL overlaps. This
// SAME-Cell role needs at most seven much smaller first-A WAL buffers simultaneously:
// Cell/plan cipher, source raw, cached plaintext, unframe ciphertext and sequential MAC
// plaintext/assembly. Actual original producer/SDK/read costs are separate existing terms.
const _: () = assert!(RECOVERY_WAL_BUFFER_BYTES <= 5 * (WAL_PLAIN_ALLOCATION_CAP + 40));
pub(in crate::store) const V2_TEXT_RETAIN_RECOVERY_METADATA_BYTES: usize =
    4 * std::mem::size_of::<RecoveryPlan>()
        + 4 * std::mem::size_of::<PreparedV2TextRetainRecovery>()
        + 2 * std::mem::size_of::<CapturedV2TextRetainRecovery>()
        + 4 * std::mem::size_of::<ReadyV2TextRetainRepair>()
        + 4 * std::mem::size_of::<ReadyV2TextRetainScratch>()
        + V2_TEXT_RETAIN_RECOVERY_ADMISSION_LAYOUT_BYTES
        + 64 * std::mem::size_of::<std::fs::File>()
        + 8 * (MAX_PATH + 256)
        + 32 * 1024;

/// Stored exactly once in the genuine Cell. An independent wrapper cannot invent a new
/// counter and duplicate copies against one existing work/metadata payment.
pub(super) struct RetainRecoveryControl {
    state: AtomicU8,
}
const IDLE: u8 = 0;
const RECOVERY: u8 = 1;
const STAGE: u8 = 2;
impl RetainRecoveryControl {
    pub(super) fn new() -> Self {
        Self {
            state: AtomicU8::new(IDLE),
        }
    }
    /// Negative phase fence only. Root must destroy its old recovery/Ready aliases
    /// outside writers before asking for a NEW ordinary Stage. An already admitted
    /// finite stage remains allowed to finish through its own actual marker.
    pub(super) fn require_idle(&self) -> Result<(), AppError> {
        if self.state.load(Ordering::Acquire) != IDLE {
            return Err(refused());
        }
        Ok(())
    }
    /// Actual final phase edge, deliberately free of accounts/IO/SDK/key work. Factory
    /// calls this only AFTER its full current/source proof, then caches the affine fence
    /// LAST after the physical lease and native End in the genuine Stage marker.
    pub(super) fn claim_stage(
        self: &Arc<Self>,
        original: OriginalV2TextRetainTransactionObservation,
    ) -> Result<V2TextRetainStageFence, AppError> {
        self.state
            .compare_exchange(IDLE, STAGE, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        let fence = V2TextRetainStageFence {
            control: Arc::clone(self),
            original,
        };
        Ok(fence)
    }
}
/// Negative phase exclusion only, NOT a Current/work/physical/SDK/signing permission.
/// Its private constructor is reachable only from the actual original Cell. Factory
/// retains it LAST through native File/OSlease/End cleanup, closing the check/admit race.
pub(in crate::store) struct V2TextRetainStageFence {
    control: Arc<RetainRecoveryControl>,
    // LAST: independently paid metadata survives the entire escaped native phase tail.
    original: OriginalV2TextRetainTransactionObservation,
}
impl V2TextRetainStageFence {
    pub(in crate::store) fn require_same_transaction(
        &self,
        original: &OriginalV2TextRetainTransactionObservation,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)?;
        if self.control.state.load(Ordering::Acquire) != STAGE {
            return Err(refused());
        }
        Ok(())
    }
}
impl Drop for V2TextRetainStageFence {
    fn drop(&mut self) {
        // Never clear somebody else's phase on an internal invariant failure.
        let result =
            self.control
                .state
                .compare_exchange(STAGE, IDLE, Ordering::AcqRel, Ordering::Acquire);
        debug_assert!(result.is_ok(), "original stage phase fence was not affine");
    }
}
struct WholeRecoveryClaim {
    control: Arc<RetainRecoveryControl>,
    work: Arc<V2TextWorkResources>,
    metadata: V2TextTransactionMetadataResources,
}
impl Drop for WholeRecoveryClaim {
    fn drop(&mut self) {
        let result = self.control.state.compare_exchange(
            RECOVERY,
            IDLE,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        debug_assert!(
            result.is_ok(),
            "whole original recovery claim was not affine"
        );
    }
}
struct RecoveryIdentity;
struct RecoveryPlan {
    original: OriginalV2TextRetainTransactionObservation,
    identity: Arc<RecoveryIdentity>,
    expected: Zeroizing<Vec<u8>>,
    plain: OnceLock<Zeroizing<Vec<u8>>>,
    wal_native: Option<SessionId>,
    first_without_dependents: bool,
    active_capture: AtomicBool,
    scratch_unlinked: AtomicBool,
    parent: Arc<RetainParent>,
    // LAST: all copied frames, Ready/source aliases and active source debt drain first.
    claim: WholeRecoveryClaim,
}
impl RecoveryPlan {
    fn validate_preparation(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.claim.metadata.validate_prepaid_account()?;
        self.claim.work.validate_prepaid_account()?;
        self.parent
            .resources
            .require_bound_original_negative(store, source)
    }
    fn validate_parent(&self, guard: &V2TextCurrentGroupGuard<'_, '_>) -> Result<(), AppError> {
        // These distinct original payments can wait/fail independently. Validate BOTH
        // before the complete held source/name proof, rather than treating a valid
        // read/OS lease as evidence that the16MiB/transaction-metadata1.25MiB debit is still admitted.
        self.claim.work.validate_prepaid_account()?;
        self.claim.metadata.validate_prepaid_account()?;
        self.parent.validate_parent(guard, false, None)
    }
    fn claim_capture(self: &Arc<Self>) -> Result<V2TextRetainRecoveryCaptureClaim, AppError> {
        self.claim.metadata.validate_prepaid_account()?;
        self.claim.work.validate_prepaid_account()?;
        self.active_capture
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        Ok(V2TextRetainRecoveryCaptureClaim {
            plan: Arc::clone(self),
        })
    }
    fn require_no_dependents(&self) -> Result<(), AppError> {
        if !self.first_without_dependents {
            return Err(refused());
        }
        let plain = self.plain.get().ok_or_else(refused)?;
        let TextWalFrameData::Work(wal) = TextWalFrameData::decode_data(plain)? else {
            return Err(refused());
        };
        for artifact in wal.artifacts.entries_data() {
            let a = artifact?;
            let path = native::artifact_child(self.parent.pages()?, a.role_data(), &a.id_data())?;
            native::require_absent(&path)?;
        }
        native::require_absent(&fixed_child(&self.parent.target, "head.stage")?)
    }
}

/// Metadata identity only: no Store/key/SDK/raw packet/physical lease or work permission.
#[derive(Clone)]
pub(in crate::store) struct OriginalV2TextRetainRecoveryObservation {
    identity: Weak<RecoveryIdentity>,
    original: OriginalV2TextRetainTransactionObservation,
}
impl OriginalV2TextRetainRecoveryObservation {
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        self.original.require_same_original(&other.original)?;
        if !self.identity.ptr_eq(&other.identity) {
            return Err(refused());
        }
        Ok(())
    }
}
/// One source/effect precursor shared across ALL role variants. Factory stores this LAST,
/// after actual source Files, physical lease and write End; even source=None stays funded.
pub(in crate::store) struct V2TextRetainRecoveryCaptureClaim {
    plan: Arc<RecoveryPlan>,
}
impl Drop for V2TextRetainRecoveryCaptureClaim {
    fn drop(&mut self) {
        self.plan.active_capture.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum V2TextRetainWarmRecoveryClassData {
    CleanUnstarted,
    CompleteMain,
    CompleteStage,
    FirstWalScratch,
}
enum RecoveryReady {
    Repair(ReadyV2TextRetainRepair),
    Scratch(ReadyV2TextRetainScratch),
    CompleteStage,
    CleanUnstarted,
}

pub(in crate::store) struct PreparedV2TextRetainRecovery {
    ready: Option<RecoveryReady>,
    repaired: Option<(ReadyV2TextRetainSelection, work::RetainSlots)>,
    scratch_completed: bool,
    installed: bool,
    #[cfg(all(test, windows))]
    after_capture: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    after_read: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    fail_after_ready: bool,
    #[cfg(all(test, windows))]
    hooks: effect::RecoveryTestHooks,
    // LAST: no root phase, outcome, plan buffer or capture survives its real work payment.
    plan: Arc<RecoveryPlan>,
}
impl PreparedV2TextRetainRecovery {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        cell: &PreparedV2TextRetainTransaction,
    ) -> Result<Self, AppError> {
        cell.custody.validate_accounts(store)?;
        cell.producer_data()
            .validate_resources_original(store, source)?;
        if cell.ready.is_some() || cell.saved.is_some() {
            return Err(refused());
        }
        let expected = cell.wal.as_ref().ok_or_else(refused)?.frame_data();
        if expected.len() > MAX_RECOVERY_WAL_FRAME {
            return Err(refused());
        }
        let wal_native = cell.slots.wal_native_data();
        // Before ANY new source/copy/lease: one claim in the actual original Cell.
        cell.recovery
            .state
            .compare_exchange(IDLE, RECOVERY, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        let claim = WholeRecoveryClaim {
            control: Arc::clone(&cell.recovery),
            work: Arc::clone(cell.custody.work_resources()),
            metadata: cell.custody.metadata_resources().clone(),
        };
        let expected = Zeroizing::new(expected.to_vec());
        if expected.capacity() > MAX_RECOVERY_WAL_FRAME {
            return Err(refused());
        }
        cell.resources()
            .require_bound_original_negative(store, source)?;
        let plan = Arc::new(RecoveryPlan {
            original: cell.observe_original(),
            identity: Arc::new(RecoveryIdentity),
            expected,
            plain: OnceLock::new(),
            wal_native,
            first_without_dependents: cell.slots.no_dependents_data(),
            active_capture: AtomicBool::new(false),
            scratch_unlinked: AtomicBool::new(false),
            parent: Arc::clone(&cell.parent),
            claim,
        });
        Ok(Self {
            ready: None,
            repaired: None,
            scratch_completed: false,
            installed: false,
            #[cfg(all(test, windows))]
            after_capture: None,
            #[cfg(all(test, windows))]
            after_read: None,
            #[cfg(all(test, windows))]
            fail_after_ready: false,
            #[cfg(all(test, windows))]
            hooks: effect::RecoveryTestHooks::default(),
            plan,
        })
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.plan.parent.resources
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.plan.original.original.clone()
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextRetainRecoveryObservation {
        observe(&self.plan)
    }
    pub(in crate::store) fn validate_preparation(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.plan.validate_preparation(store, source)
    }
    pub(in crate::store) fn claim_capture_preparation(
        &self,
    ) -> Result<V2TextRetainRecoveryCaptureClaim, AppError> {
        self.plan.claim_capture()
    }
    pub(in crate::store) fn validate_parent_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.plan.validate_parent(guard)
    }
    pub(in crate::store) fn capture(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        slot: &mut V2TextRetainRecoverySourceSlot<'_>,
    ) -> Result<(), AppError> {
        self.plan.validate_parent(guard)?;
        if slot.get().is_none() {
            slot.cache_initial(CapturedV2TextRetainRecovery::empty(Arc::clone(&self.plan)));
        }
        let source = slot.get_mut().ok_or_else(refused)?;
        source.require_same_plan(&self.plan)?;
        source.capture_metadata(guard)?;
        #[cfg(all(test, windows))]
        if let Some(hook) = self.after_capture.take() {
            hook();
        }
        source.validate_under_current_guard(guard)
    }
    pub(in crate::store) fn run_read(
        &mut self,
        mut admitted: AdmittedV2TextRetainRecoveryRead,
    ) -> Result<(), AppError> {
        admitted.require_same_recovery(&self.observe_original())?;
        self.plan.claim.work.validate_prepaid_account()?;
        self.plan.claim.metadata.validate_prepaid_account()?;
        let result = (|| {
            #[cfg(all(test, windows))]
            if let Some(hook) = self.after_read.take() {
                hook();
            }
            // Marker retains the owning Source. Never expose &mut Captured: two safe
            // cross-vault mem::swaps could otherwise detach Files from their own OSlease.
            let classified = admitted.classify_data(&self.observe_original())?;
            let next = match classified {
                (V2TextRetainWarmRecoveryClassData::CompleteMain, native, len) => {
                    RecoveryReady::Repair(ReadyV2TextRetainRepair {
                        native: native.ok_or_else(refused)?,
                        len,
                        plan: Arc::clone(&self.plan),
                    })
                }
                (V2TextRetainWarmRecoveryClassData::FirstWalScratch, native, len) => {
                    RecoveryReady::Scratch(ReadyV2TextRetainScratch {
                        native: native.ok_or_else(refused)?,
                        len,
                        plan: Arc::clone(&self.plan),
                    })
                }
                (V2TextRetainWarmRecoveryClassData::CompleteStage, _, _) => {
                    RecoveryReady::CompleteStage
                }
                (V2TextRetainWarmRecoveryClassData::CleanUnstarted, _, _) => {
                    RecoveryReady::CleanUnstarted
                }
            };
            if self.ready.is_some() {
                return Err(refused());
            }
            self.ready = Some(next);
            Ok(())
        })();
        drop(admitted);
        #[cfg(all(test, windows))]
        if result.is_ok() && std::mem::take(&mut self.fail_after_ready) {
            return Err(refused());
        }
        result
    }
    pub(in crate::store) fn class_data(&self) -> Option<V2TextRetainWarmRecoveryClassData> {
        self.ready.as_ref().map(|ready| match ready {
            RecoveryReady::Repair(_) => V2TextRetainWarmRecoveryClassData::CompleteMain,
            RecoveryReady::Scratch(_) => V2TextRetainWarmRecoveryClassData::FirstWalScratch,
            RecoveryReady::CompleteStage => V2TextRetainWarmRecoveryClassData::CompleteStage,
            RecoveryReady::CleanUnstarted => V2TextRetainWarmRecoveryClassData::CleanUnstarted,
        })
    }
    pub(in crate::store) fn ready_repair(&self) -> Result<&ReadyV2TextRetainRepair, AppError> {
        match self.ready.as_ref() {
            Some(RecoveryReady::Repair(ready)) => Ok(ready),
            _ => Err(refused()),
        }
    }
    pub(in crate::store) fn ready_scratch(&self) -> Result<&ReadyV2TextRetainScratch, AppError> {
        match self.ready.as_ref() {
            Some(RecoveryReady::Scratch(ready)) => Ok(ready),
            _ => Err(refused()),
        }
    }
    pub(in crate::store) fn has_completed_repair_data(&self) -> bool {
        self.repaired.is_some()
            || self.installed
                && self.class_data() == Some(V2TextRetainWarmRecoveryClassData::CompleteMain)
    }
    pub(in crate::store) fn has_completed_scratch_data(&self) -> bool {
        self.scratch_completed
            || self.installed
                && self.class_data() == Some(V2TextRetainWarmRecoveryClassData::FirstWalScratch)
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_after_capture_files_for_test(
        &mut self,
        hook: Box<dyn FnOnce() + Send>,
    ) {
        self.after_capture = Some(hook);
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_after_run_source_for_test(
        &mut self,
        hook: Box<dyn FnOnce() + Send>,
    ) {
        self.after_read = Some(hook);
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn fail_after_ready_for_test(&mut self) {
        self.fail_after_ready = true;
    }
}
fn observe(plan: &Arc<RecoveryPlan>) -> OriginalV2TextRetainRecoveryObservation {
    OriginalV2TextRetainRecoveryObservation {
        identity: Arc::downgrade(&plan.identity),
        original: plan.original.clone(),
    }
}

pub(in crate::store) struct ReadyV2TextRetainRepair {
    native: SessionId,
    len: usize,
    plan: Arc<RecoveryPlan>,
}
pub(in crate::store) struct ReadyV2TextRetainScratch {
    native: SessionId,
    len: usize,
    plan: Arc<RecoveryPlan>,
}
macro_rules! ready_api {
    ($ty:ident) => {
        impl $ty {
            pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
                &self.plan.parent.resources
            }
            pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
                self.plan.original.original.clone()
            }
            pub(in crate::store) fn observe_original(
                &self,
            ) -> OriginalV2TextRetainRecoveryObservation {
                observe(&self.plan)
            }
            pub(in crate::store) fn validate_preparation(
                &self,
                store: &ServerStore,
                source: &GenesisSourceDataRef<'_>,
            ) -> Result<(), AppError> {
                self.plan.validate_preparation(store, source)
            }
            pub(in crate::store) fn claim_capture_preparation(
                &self,
            ) -> Result<V2TextRetainRecoveryCaptureClaim, AppError> {
                self.plan.claim_capture()
            }
        }
    };
}
ready_api!(ReadyV2TextRetainRepair);
ready_api!(ReadyV2TextRetainScratch);

macro_rules! redacted_recovery_debug {
    ($($ty:ident),+$(,)?)=>{$(impl std::fmt::Debug for $ty{
        fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+};
}
redacted_recovery_debug!(
    OriginalV2TextRetainRecoveryObservation,
    V2TextRetainRecoveryCaptureClaim,
    PreparedV2TextRetainRecovery,
    CapturedV2TextRetainRecovery,
    ReadyV2TextRetainRepair,
    ReadyV2TextRetainScratch,
    V2TextRetainStageFence
);
