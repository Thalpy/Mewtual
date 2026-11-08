//! Actual A admission from the DISTINCT completed-WC predecessor.
//!
//! The first Clear path remains unchanged. The concrete next-A issuer owns the
//! original completed parent before any replacement and selects its own recorded
//! W_A continuation FIRST after a lost write/rename. Historical WC is never a
//! generic waiver for a dirty workspace or a permit to adopt somebody else's A.
use super::*;
use crate::store::critical_namespaces::current_layout::BoundV2TextCompletedCreatePredecessor;

impl V2TextCurrentGroupGuard<'_, '_> {
    pub(in crate::store) fn admit_text_completed_retain_stage_into(
        &mut self,
        lane: &V2TextCreateLane,
        predecessor: &BoundV2TextCompletedCreatePredecessor,
        prepared: &PreparedV2TextRetainTransaction,
        physical: &mut Option<PreparedV2TextPhysicalLease>,
        admitted: &mut Option<AdmittedV2TextRetainStage>,
    ) -> Result<(), AppError> {
        prepared
            .resources()
            .require_same_original(self.resources())?;
        let original = prepared.observe_original();
        let retain = prepared.original_retain();
        predecessor.require_same_lane_original(lane, &retain)?;
        predecessor.validate_negative()?;
        if let Some(existing) = admitted.as_ref() {
            existing.require_same_transaction(&original)?;
            existing.retain.require_same_original(&retain)?;
            existing.physical.require_retain_write_negative(self)?;
            existing.end.require_same_lane(lane)?;
            return self.validate_source_only();
        }
        let lease = physical.as_ref().ok_or_else(refused)?;
        lease.validate_retain_write_under_guard(self)?;
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        self.owner.require_text_control_io_quiescent()?;
        self.require_text_control_write_idle_for_query()?;
        // All original accounts, exact completed-parent association and actual
        // WC-or-SAME-WA branch inspection precede the FINAL full source proof.
        // A lost WA rename must not fall through to requiring obsolete WC.
        prepared.validate_completed_stage_under_current_guard(self, predecessor)?;
        predecessor.require_same_lane_original(lane, &retain)?;
        predecessor.validate_negative()?;
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        lease.require_retain_write_negative(self)?;
        self.owner.require_text_control_io_quiescent()?;
        self.validate_source_only()?;
        let phase = prepared.reserve_stage_fence()?;
        let end = lane.reserve_create_control_write_end(&mut self.owner, &retain)?;
        // SAME final canonical edge installs the actual writer gate; only owning
        // moves follow before the outside marker cache, including late Stop.
        *admitted = Some(AdmittedV2TextRetainStage {
            original,
            retain,
            physical: physical.take().expect("same completed-parent A lease"),
            end,
            phase,
        });
        Ok(())
    }

    pub(in crate::store) fn admit_text_completed_retain_commit_into(
        &mut self,
        lane: &V2TextCreateLane,
        predecessor: &BoundV2TextCompletedCreatePredecessor,
        ready: &ReadyV2TextRetainSelection,
        physical: &mut Option<PreparedV2TextPhysicalLease>,
        admitted: &mut Option<AdmittedV2TextRetainCommit>,
    ) -> Result<(), AppError> {
        ready.resources().require_same_original(self.resources())?;
        let original = ready.observe_original();
        let retain = ready.original_retain();
        predecessor.require_same_lane_original(lane, &retain)?;
        predecessor.validate_negative()?;
        if let Some(existing) = admitted.as_ref() {
            existing.require_same_transaction(&original)?;
            existing.retain.require_same_original(&retain)?;
            existing.physical.require_retain_write_negative(self)?;
            existing.end.require_same_lane(lane)?;
            return self.validate_source_only();
        }
        let lease = physical.as_ref().ok_or_else(refused)?;
        lease.validate_retain_write_under_guard(self)?;
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        self.owner.require_text_control_io_quiescent()?;
        self.require_text_control_write_idle_for_query()?;
        // A genuine Ready from this next-A cell proves its OWN complete WA and
        // staged outputs. Completion/commit never reinterprets earlier WC DATA.
        ready.validate_completed_ready_under_current_guard(self, predecessor)?;
        predecessor.require_same_lane_original(lane, &retain)?;
        predecessor.validate_negative()?;
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
                .expect("same ready completed-parent A lease"),
            end,
        });
        Ok(())
    }
}
