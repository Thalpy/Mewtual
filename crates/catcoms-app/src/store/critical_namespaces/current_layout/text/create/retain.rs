//! Distinct actual Current permissions for the first retained-A stage and selection.
//!
//! This family is declared only with the concrete Physical transaction/Ready issuer and
//! owning Root caller. A Draft, parsed WAL, Bound predecessor, resource lease, IO-zero
//! observation or historical SavedA cannot construct either private affine permission.
use super::*;
mod completed;

mod recovery;
use crate::store::owner_evidence::canonical::text_lane::V2TextCreateControlWriteEnd;
use crate::store::persistence::v2_text::{
    OriginalV2RetainObservation, OriginalV2TextRetainTransactionObservation,
    PreparedV2TextRetainTransaction, ReadyV2TextRetainSelection, V2TextRetainStageFence,
};
pub(in crate::store) use recovery::{
    AdmittedV2TextRetainRecoveryRead, AdmittedV2TextRetainRepair, AdmittedV2TextRetainScratch,
    PreparedV2TextRetainRecoveryCapture, PreparedV2TextRetainRepairLease,
    PreparedV2TextRetainScratchLease, V2TextRetainRecoverySourceSlot,
    V2_TEXT_RETAIN_RECOVERY_ADMISSION_LAYOUT_BYTES,
};

/// Exact immutable-A staging only. The concrete runner must save its complete original
/// WORK WAL before creating any named output. This marker cannot select O or issue SavedA.
pub(in crate::store) struct AdmittedV2TextRetainStage {
    original: OriginalV2TextRetainTransactionObservation,
    retain: OriginalV2RetainObservation,
    // All runner-local Files/key/syscall cleanup drains BEFORE this SAME OS lease.
    physical: PreparedV2TextPhysicalLease,
    // Physical cleanup releases the gate/cap, including an unstarted
    // admitted marker or a late worker Err. No count or weak expiry substitutes for it.
    end: V2TextCreateControlWriteEnd,
    // LAST: actual Idle/Stage/Recovery CAS custody survives all Files, physical lease
    // AND control End cleanup. A checked load cannot replace this affine phase fence.
    phase: V2TextRetainStageFence,
}

/// Fresh Current permission to select this exact staged Ready A over its original parent.
/// This owner is distinct from Stage; the runner still compares the FULL original WAL and
/// required frames outside writers before CAS, then completes real file/parent barriers.
pub(in crate::store) struct AdmittedV2TextRetainCommit {
    original: OriginalV2TextRetainTransactionObservation,
    retain: OriginalV2RetainObservation,
    physical: PreparedV2TextPhysicalLease,
    end: V2TextCreateControlWriteEnd,
}

/// Actual marker/temporary precursor metadata included by the paired transaction invoice
/// BEFORE allocation/copy. Root's SDK packet, read/query/Scope and16MiB payload/work costs
/// are additional retained payments. No negative SavedA tail inherits that large payment.
pub(in crate::store) const V2_TEXT_RETAIN_ADMISSION_LAYOUT_BYTES: usize =
    2 * std::mem::size_of::<PreparedV2TextPhysicalLease>()
        + std::mem::size_of::<AdmittedV2TextRetainStage>()
        + std::mem::size_of::<AdmittedV2TextRetainCommit>()
        + 4 * 64;

impl AdmittedV2TextRetainStage {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_after_writer_gate_clear_for_test(
        &mut self,
        hook: Box<dyn FnOnce() + Send>,
    ) {
        self.end.set_after_gate_clear_for_test(hook);
    }

    /// Pure original identity only. An admitted finite worker need not reacquire Current
    /// after Stop, but it may operate only on its SAME actual original transaction.
    pub(in crate::store) fn require_same_transaction(
        &self,
        original: &OriginalV2TextRetainTransactionObservation,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)?;
        self.phase.require_same_transaction(original)
    }
}
impl AdmittedV2TextRetainCommit {
    pub(in crate::store) fn require_same_transaction(
        &self,
        original: &OriginalV2TextRetainTransactionObservation,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)
    }
}

impl V2TextCurrentGroupGuard<'_, '_> {
    /// Borrow ALL caller-owned slots through the actual Scope callback. Once the real
    /// marker is cached outside, Scope's late postcheck can report Err without losing
    /// admitted work, dropping its physical custody under AD or renewing another attempt.
    pub(in crate::store) fn admit_text_retain_stage_into(
        &mut self,
        lane: &V2TextCreateLane,
        predecessor: &BoundV2TextCreatePredecessor,
        prepared: &PreparedV2TextRetainTransaction,
        physical: &mut Option<PreparedV2TextPhysicalLease>,
        admitted: &mut Option<AdmittedV2TextRetainStage>,
    ) -> Result<(), AppError> {
        let original = prepared.observe_original();
        let retain = prepared.original_retain();
        if let Some(existing) = admitted.as_ref() {
            existing.require_same_transaction(&original)?;
            existing.retain.require_same_original(&retain)?;
            existing.physical.require_retain_write_negative(self)?;
            existing.end.require_same_lane(lane)?;
            predecessor.require_same_lane_original(lane, &retain)?;
            return self.validate_source_only();
        }
        let lease = physical.as_ref().ok_or_else(refused)?;
        // All account waits precede the complete bounded physical source proof.
        lease.validate_retain_write_under_guard(self)?;
        prepared
            .resources()
            .require_same_original(self.resources())?;
        predecessor.require_same_lane_original(lane, &retain)?;
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        self.owner.require_text_control_io_quiescent()?;
        self.require_text_control_write_idle_for_query()?;
        prepared.validate_stage_under_current_guard(self)?;
        // The physical issuer has proved actual original C/O/H and its exact immutable
        // initial-A candidate/clear-source facts. No full WAL/page scan occurs under AD.
        // Final cached negatives follow every physical/account wait.
        predecessor.require_same_lane_original(lane, &retain)?;
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        lease.require_retain_write_negative(self)?;
        self.owner.require_text_control_io_quiescent()?;
        self.validate_source_only()?;
        // Pure cached CAS after ALL physical/source checks. Recovery's same atomic
        // Idle->Recovery claim cannot slip between this edge and the outside worker.
        // On a later cap refusal the still-borrowed Cell retains all original payers.
        let phase = prepared.reserve_stage_fence()?;
        let end = lane.reserve_create_control_write_end(&mut self.owner, &retain)?;
        // Gate and cap are now active under this SAME canonical edge. Only infallible
        // owning transfers follow; a new query cannot open an obsolete O/H after unlock.
        *admitted = Some(AdmittedV2TextRetainStage {
            original,
            retain,
            physical: physical
                .take()
                .expect("same imminent outside lease checked"),
            end,
            phase,
        });
        Ok(())
    }

    /// Ready is produced only by this original transaction's complete finite staging.
    /// It does not renew Current. Reacquire this exact source/lane/close/IO admission before
    /// every selector mutation; a Stage marker or decoded/historical Saved is insufficient.
    pub(in crate::store) fn admit_text_retain_commit_into(
        &mut self,
        lane: &V2TextCreateLane,
        predecessor: &BoundV2TextCreatePredecessor,
        ready: &ReadyV2TextRetainSelection,
        physical: &mut Option<PreparedV2TextPhysicalLease>,
        admitted: &mut Option<AdmittedV2TextRetainCommit>,
    ) -> Result<(), AppError> {
        let original = ready.observe_original();
        let retain = ready.original_retain();
        if let Some(existing) = admitted.as_ref() {
            existing.require_same_transaction(&original)?;
            existing.retain.require_same_original(&retain)?;
            existing.physical.require_retain_write_negative(self)?;
            existing.end.require_same_lane(lane)?;
            predecessor.require_same_lane_original(lane, &retain)?;
            return self.validate_source_only();
        }
        let lease = physical.as_ref().ok_or_else(refused)?;
        lease.validate_retain_write_under_guard(self)?;
        ready.resources().require_same_original(self.resources())?;
        predecessor.require_same_lane_original(lane, &retain)?;
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        self.owner.require_text_control_io_quiescent()?;
        self.require_text_control_write_idle_for_query()?;
        ready.validate_ready_under_current_guard(self)?;
        // The bounded Ready proof establishes original parent/contexts and same exact
        // staged IDs/names. The consumed worker still reopens and compares full WAL and
        // required output bytes outside writers BEFORE CAS; only real barriers yield SavedA.
        predecessor.require_same_lane_original(lane, &retain)?;
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        lease.require_retain_write_negative(self)?;
        self.owner.require_text_control_io_quiescent()?;
        self.validate_source_only()?;
        let end = lane.reserve_create_control_write_end(&mut self.owner, &retain)?;
        *admitted = Some(AdmittedV2TextRetainCommit {
            original,
            retain,
            physical: physical
                .take()
                .expect("same imminent outside lease checked"),
            end,
        });
        Ok(())
    }
}

impl std::fmt::Debug for AdmittedV2TextRetainStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedV2TextRetainStage")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for AdmittedV2TextRetainCommit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedV2TextRetainCommit")
            .finish_non_exhaustive()
    }
}
