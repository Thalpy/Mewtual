//! Recovery inputs for a fetched checkpoint whose predecessor need not exist locally.
//! No method in this module removes the source or proves that recovery reached durable storage.

use super::*;
use crate::{registry::RegistryRecovery, LocalIntent, RecoverySnapshot, VerifiedCheckpoint};
use std::collections::BTreeMap;

/// An immutable, typed plan for replacing a still-whole source with the selected checkpoint.
/// Unlike a held-close settlement, there is no locally verified closure to retire intents from.
/// The whole previous version is retained conservatively, including seed-only pointers. A store
/// transaction must save it and finish any eviction warning BEFORE selecting the new unit.
pub struct RegistryAdoptionPlan {
    receipt: Receipt,
    checkpoint: VerifiedCheckpoint,
    source_version: [u8; 32],
    recovery: Option<RecoverySnapshot>,
}

impl std::fmt::Debug for RegistryAdoptionPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryAdoptionPlan")
            .field("checkpoint_epoch", &self.checkpoint.origin().epoch())
            .field("has_recovery", &self.recovery.is_some())
            .finish_non_exhaustive()
    }
}

impl RegistryAdoptionPlan {
    /// Current-owner receipt checked when planning, not a transferable network/lifetime permit.
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }

    /// Exact raw seed with both expected-hash and canonical registry validation completed.
    pub fn checkpoint(&self) -> &VerifiedCheckpoint {
        &self.checkpoint
    }

    /// Whole-source recovery, independent of the destination receipt and quarantine traffic.
    /// Its stable id lets a retarget reuse existing evidence without restarting eviction timers.
    /// None means the source has no pointers, overflow, tombstones OR accepted operations.
    pub fn recovery_snapshot(&self) -> Option<&RecoverySnapshot> {
        self.recovery.as_ref()
    }

    /// Binds this computation to the exact source, including gate and high-water state. This
    /// fingerprint is stricter than the content-only recovery identity and is not a wire hash.
    pub fn matches_source(&self, source: &mut RegistryEpoch) -> Result<bool, ReplError> {
        Ok(self.source_version == settlement::source_version(source)?)
    }
}

impl RegistryEpoch {
    /// Select a received checkpoint while retaining and sealing the FULL local source. The
    /// caller supplies independently established current tenure; the network installer must
    /// additionally hold fresh scoped discovery provenance. This is not a currency lease.
    ///
    /// Fault is returned successfully so an enclosing transaction can durably save evidence.
    /// Do not turn that outcome into an error before the save. Stale changes nothing. An exact
    /// already-installed opening leaves newer edits/seals intact. Other selections use explicit
    /// adoption mode, including when their closed epoch happens to equal the source epoch.
    pub fn begin_checkpoint_adoption(
        &mut self,
        receipt: Receipt,
        group: &ServerGroup,
        expected_tenure_start: u64,
    ) -> Result<ReceiptIngest, ReplError> {
        receipt.verify_current_owner(group, expected_tenure_start)?;
        if receipt.document != self.logical {
            return Err(ReplError::EpochScope);
        }
        if receipt.closed_epoch >= MAX_REGISTRY_EPOCH {
            return Err(ReplError::EpochBound);
        }
        if self.opened_by(&receipt) {
            return Ok(ReceiptIngest::Duplicate);
        }
        let was_faulted = self.phase() == EpochPhase::Fault;
        let outcome = self.receipts.ingest_adoption(
            receipt,
            group,
            expected_tenure_start,
            &self.gate,
            self.opening.as_ref(),
        )?;
        if outcome != ReceiptIngest::Stale && !was_faulted {
            self.adopting = true;
        }
        Ok(outcome)
    }

    /// Prepare a fetched successor and bounded whole-source recovery after its decision has
    /// sealed this source. Bad/absent seeds cannot undo an already returned Fault outcome.
    /// This method does not journal, retire, restore, replay, or acknowledge any local intent.
    pub fn prepare_checkpoint_adoption(
        &mut self,
        receipt: &Receipt,
        raw_seed: &[u8],
        group: &ServerGroup,
        expected_tenure_start: u64,
    ) -> Result<RegistryAdoptionPlan, ReplError> {
        if !self.adopting
            || self.phase() != EpochPhase::Closing
            || self.receipts.is_faulted()
            || self.receipts.latest() != Some(receipt)
        {
            return Err(ReplError::ReceiptConflict);
        }
        let verified = receipt.verify_current_owner(group, expected_tenure_start)?;
        let checkpoint = RegistryProjection::verify_checkpoint(&verified, self.bucket, raw_seed)?;
        let mut operations = BTreeMap::new();
        for op in self.doc.signed_log() {
            let operation = op.parsed_domain_op()?.ok_or(ReplError::Malformed)?;
            operations.insert(
                operation.id(&op.author_device),
                LocalIntent {
                    author: op.author_device,
                    operation,
                },
            );
        }
        let recovery = RegistryRecovery::snapshot_for_rewind(
            self.projection()?,
            self.opening.as_ref(),
            operations,
        )?;
        Ok(RegistryAdoptionPlan {
            receipt: receipt.clone(),
            checkpoint,
            source_version: settlement::source_version(self)?,
            recovery,
        })
    }

    /// Construct (but never install) the replacement after rechecking the exact source and
    /// current authority. The storage owner must first persist the plan's recovery. No intent
    /// is finalized by matching a seed value; authors replay their durable pending operations.
    pub fn adopted_successor(
        &mut self,
        plan: &RegistryAdoptionPlan,
        group: &ServerGroup,
        expected_tenure_start: u64,
    ) -> Result<Self, ReplError> {
        if !self.adopting
            || self.phase() != EpochPhase::Closing
            || self.receipts.is_faulted()
            || self.receipts.latest() != Some(plan.receipt())
            || !plan.matches_source(self)?
        {
            return Err(ReplError::ReceiptConflict);
        }
        let mut successor = Self::from_checkpoint(
            group,
            self.bucket,
            self.actor,
            plan.receipt.clone(),
            expected_tenure_start,
            plan.checkpoint.bytes(),
        )?;
        successor.receipts = self.receipts.clone();
        successor.receipts.mark_latest_installed();
        Ok(successor)
    }
}

#[cfg(test)]
mod tests;
