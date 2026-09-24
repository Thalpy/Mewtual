//! Thin typed adapter to the shared source repair state machine. No gate/book escapes and
//! the caller must persist the complete returned source before exposing repair progress.

use super::*;
use crate::epoch::repair_transition::RepairSource;
use crate::{
    CloseRecord, OwnerReceiptJournal, ReceiptRepair, ReceiptRepairPlan, SourceRepairOutcome,
    SourceRepairState,
};

impl RegistryEpoch {
    /// Compute compatible source and journal candidates without changing either. A held source
    /// returns ReceiptConflict. Independent effects and non-losing heads need not be equal.
    ///
    /// The caller still admits historical evidence, holds custody and enforces the durable
    /// transaction/sequence claim, including NoChange. Persist the complete owner candidate at
    /// B1 before applying this plan; preparation itself performs no IO and grants no authority.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_receipt_repair(
        &mut self,
        repair: &ReceiptRepair,
        a: &Receipt,
        b: &Receipt,
        journal: &OwnerReceiptJournal,
        retiring_close: Option<&CloseRecord>,
        group: &ServerGroup,
        issuer_tenure_start: u64,
    ) -> Result<ReceiptRepairPlan, ReplError> {
        // Reject invalid authority before serializing the bounded but potentially large source.
        repair.verify_current_owner(group, issuer_tenure_start)?;
        repair.check_evidence(a, b)?;
        let version = ReceiptRepairPlan::source_version(&self.snapshot()?);
        RepairSource {
            document: &self.logical,
            gate: &self.gate,
            book: &mut self.receipts,
            opening: self.opening.as_ref(),
            adopting: &mut self.adopting,
            binding: &mut self.repair_binding,
        }
        .plan_joint(
            repair,
            a,
            b,
            journal,
            retiring_close,
            version,
            group,
            issuer_tenure_start,
        )
    }

    /// Apply an exact prepared source candidate after B1, rechecking current authority and the
    /// complete source/journal versions. `current_journal` must be the actual B1 journal loaded
    /// under custody; this in-memory comparison cannot certify its durability or target claim.
    /// Persist the complete changed source at B2 before reporting progress. Stale plans refuse
    /// without reclassification or mutation; a restart needs a fresh plan for the held repair.
    pub fn apply_planned_receipt_repair(
        &mut self,
        plan: ReceiptRepairPlan,
        current_journal: &OwnerReceiptJournal,
        group: &ServerGroup,
        issuer_tenure_start: u64,
    ) -> Result<SourceRepairOutcome, ReplError> {
        plan.repair()
            .verify_current_owner(group, issuer_tenure_start)?;
        let version = ReceiptRepairPlan::source_version(&self.snapshot()?);
        RepairSource {
            document: &self.logical,
            gate: &self.gate,
            book: &mut self.receipts,
            opening: self.opening.as_ref(),
            adopting: &mut self.adopting,
            binding: &mut self.repair_binding,
        }
        .commit_joint(plan, current_journal, version, group, issuer_tenure_start)
    }

    /// Apply a current-owner signed decision to this privately owned source. The caller must
    /// independently admit historical evidence and enforce the durable owner/source transaction,
    /// custody and journal compatibility before publication. This method performs no IO.
    pub fn apply_receipt_repair(
        &mut self,
        repair: &ReceiptRepair,
        a: &Receipt,
        b: &Receipt,
        group: &ServerGroup,
        issuer_tenure_start: u64,
    ) -> Result<SourceRepairOutcome, ReplError> {
        RepairSource {
            document: &self.logical,
            gate: &self.gate,
            book: &mut self.receipts,
            opening: self.opening.as_ref(),
            adopting: &mut self.adopting,
            binding: &mut self.repair_binding,
        }
        .apply(repair, a, b, group, issuer_tenure_start)
    }

    /// Historical decision plus the actual current continuation, never a live authority lease.
    /// Legacy unbound bookkeeping returns None rather than inventing the original action.
    pub fn repair_state(&self) -> Option<SourceRepairState> {
        self.repair_binding.as_ref()?.state(
            &self.receipts,
            self.phase(),
            self.opening.as_ref(),
            self.adopting,
        )
    }

    /// Whether THIS repair still requires replacement. Screening never owns ordinary adoption.
    pub fn repair_install_pending(&self) -> bool {
        self.repair_state()
            .is_some_and(|state| state.install_pending)
    }

    /// Complete retained fault evidence, including an unrelated fault left intact by screening.
    pub fn fault_evidence(&self) -> Option<(&Receipt, &Receipt)> {
        self.receipts.fault_evidence()
    }
}

#[cfg(test)]
mod tests;
