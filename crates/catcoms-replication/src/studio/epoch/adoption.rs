//! Studio's typed consumer of the existing P1 checkpoint-adoption protocol. A fetched seed
//! is not permission to replace local work: this plan retains the entire old domain version,
//! and the store must persist it before installing the separately constructed successor.
use super::*;
use crate::{RecoveryReason, RecoverySnapshot, VerifiedCheckpoint};

/// Immutable computation, bound to one exact sealed source and one current-owner receipt.
/// It is not network provenance, a durable recovery acknowledgement, or an editing lease.
pub struct StudioAdoptionPlan {
    receipt: Receipt,
    checkpoint: VerifiedCheckpoint,
    source_version: [u8; 32],
    recovery: Option<RecoverySnapshot>,
}
impl std::fmt::Debug for StudioAdoptionPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioAdoptionPlan")
            .field("checkpoint_epoch", &self.checkpoint.origin().epoch())
            .field("has_recovery", &self.recovery.is_some())
            .finish_non_exhaustive()
    }
}
impl StudioAdoptionPlan {
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }
    pub fn checkpoint(&self) -> &VerifiedCheckpoint {
        &self.checkpoint
    }
    /// None only for the actually empty, unseeded epoch zero. Seed-only values, deleted and
    /// overwritten accepted operations all remain in the whole-version recovery payload.
    pub fn recovery_snapshot(&self) -> Option<&RecoverySnapshot> {
        self.recovery.as_ref()
    }
    pub fn matches_source(&self, source: &mut StudioEpoch) -> Result<bool, ReplError> {
        Ok(self.source_version == source_version(source)?)
    }
}
fn source_version(source: &mut StudioEpoch) -> Result<[u8; 32], ReplError> {
    let mut hash = blake3::Hasher::new_derive_key("catcoms/studio-adoption-source/v1");
    hash.update(&source.snapshot()?);
    Ok(*hash.finalize().as_bytes())
}
impl StudioEpoch {
    /// Exact installed retries preserve subsequent edits and seals; Fault is never success.
    pub fn opened_by(&self, receipt: &Receipt) -> bool {
        self.opening.as_ref() == Some(receipt)
            && matches!(self.phase(), EpochPhase::Open | EpochPhase::Closing)
    }
    /// Seal the full local source using P1's existing bounded adoption/high-water/fault rules.
    /// The caller additionally needs fresh scoped discovery provenance. Returning Fault is a
    /// successful state transition that must be saved, not an error that discards evidence.
    pub fn begin_checkpoint_adoption(
        &mut self,
        receipt: Receipt,
        group: &ServerGroup,
        tenure: u64,
    ) -> Result<ReceiptIngest, ReplError> {
        receipt.verify_current_owner(group, tenure)?;
        if receipt.document != self.logical {
            return Err(ReplError::EpochScope);
        }
        self.refresh_owner(group)?;
        if self.opened_by(&receipt) {
            return Ok(ReceiptIngest::Duplicate);
        }
        let was_faulted = self.phase() == EpochPhase::Fault;
        let outcome = self.receipts.ingest_adoption(
            receipt,
            group,
            tenure,
            &self.gate,
            self.opening.as_ref(),
        )?;
        if outcome != ReceiptIngest::Stale && !was_faulted {
            self.adopting = true;
        }
        Ok(outcome)
    }
    /// Hash and typed channel/root validation occur before planning recovery. The source stays
    /// sealed on a bad seed. No intent is finalized merely because its value appears in a seed.
    pub fn prepare_checkpoint_adoption(
        &mut self,
        receipt: &Receipt,
        raw_seed: &[u8],
        group: &ServerGroup,
        tenure: u64,
    ) -> Result<StudioAdoptionPlan, ReplError> {
        if !self.adopting
            || self.phase() != EpochPhase::Closing
            || self.receipts.is_faulted()
            || self.receipts.latest() != Some(receipt)
        {
            return Err(ReplError::ReceiptConflict);
        }
        let verified = receipt.verify_current_owner(group, tenure)?;
        let checkpoint = match self.target {
            StudioTarget::Index { .. } => {
                StudioIndexProjection::verify_checkpoint(&verified, raw_seed)
            }
            StudioTarget::Flipnote { channel, .. } => {
                FlipnoteFrameProjection::verify_checkpoint(&verified, channel, raw_seed)
            }
        }?;
        let recovery = if self.op_count() == 0 && self.opening.is_none() {
            None
        } else {
            Some(StudioRecovery::snapshot(
                &self.projection()?,
                self.opening.as_ref().map(|r| r.close_record_hash),
                RecoveryReason::Rewound,
                self.opening.as_ref().map_or([0; 32], Receipt::hash),
                &recovery::current_operations(&self.doc)?,
            )?)
        };
        Ok(StudioAdoptionPlan {
            receipt: receipt.clone(),
            checkpoint,
            source_version: source_version(self)?,
            recovery,
        })
    }
    /// Construct, never install, a successor. The storage owner first persists the plan's
    /// recovery and finishes any eviction warning. Rechecking the entire source rejects plans
    /// superseded even by a gate/receipt-only change while their content happened to stay equal.
    pub fn adopted_successor(
        &mut self,
        plan: &StudioAdoptionPlan,
        group: &ServerGroup,
        tenure: u64,
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
            self.target,
            self.actor,
            plan.receipt.clone(),
            tenure,
            plan.checkpoint.bytes(),
        )?;
        successor.receipts = self.receipts.clone();
        successor.receipts.mark_latest_installed();
        Ok(successor)
    }
}

#[cfg(test)]
mod tests;
