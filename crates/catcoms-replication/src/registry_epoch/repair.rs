//! Thin typed adapter to the shared source repair state machine. No gate/book escapes and
//! the caller must persist the complete returned source before exposing repair progress.

use super::*;
use crate::epoch::repair_transition::RepairSource;
use crate::{ReceiptRepair, SourceRepairOutcome, SourceRepairState};

impl RegistryEpoch {
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
