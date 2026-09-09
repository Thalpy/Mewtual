//! Adjacent Studio settlement computation, not the vault transaction or rotation scheduler.
//! The exact named closure supplies the seed; the whole held source supplies recovery evidence.
use super::*;
use crate::{CloseRecord, RecoveryReason, RecoverySnapshot, VerifiedCheckpoint};
use std::collections::BTreeMap;

/// Private provenance ties a seed, recovery and included/excluded author envelopes to one
/// sealed source version. No output is a storage, publication, retirement or pruning permit.
pub struct StudioSettlementPlan {
    receipt: Receipt,
    checkpoint: VerifiedCheckpoint,
    source_version: [u8; 32],
    projection: StudioProjection,
    included: BTreeMap<[u8; 32], LocalIntent>,
    excluded: BTreeMap<[u8; 32], LocalIntent>,
    recovery: Option<RecoverySnapshot>,
}
impl std::fmt::Debug for StudioSettlementPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioSettlementPlan")
            .field("epoch", &self.projection.epoch())
            .field("included", &self.included.len())
            .field("excluded", &self.excluded.len())
            .finish_non_exhaustive()
    }
}
impl StudioSettlementPlan {
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }
    pub fn checkpoint(&self) -> &VerifiedCheckpoint {
        &self.checkpoint
    }
    pub fn source_projection(&self) -> &StudioProjection {
        &self.projection
    }
    /// Retirement must compare these complete author/envelopes, not derived ids alone: a
    /// nonce-derived id deliberately does not bind the operation body.
    pub fn included_operations(&self) -> &BTreeMap<[u8; 32], LocalIntent> {
        &self.included
    }
    /// Historical evidence, not authority to journal/replay another member's operation.
    pub fn excluded_operations(&self) -> &BTreeMap<[u8; 32], LocalIntent> {
        &self.excluded
    }
    /// Already exact-size checked, but still needs the existing durable two-plus-staged store
    /// transaction and eviction hold before the source may be replaced.
    pub fn recovery_snapshot(&self) -> Option<&RecoverySnapshot> {
        self.recovery.as_ref()
    }
    pub fn matches_source(&self, source: &mut StudioEpoch) -> Result<bool, ReplError> {
        Ok(self.source_version == source_version(source)?)
    }
}
fn source_version(source: &mut StudioEpoch) -> Result<[u8; 32], ReplError> {
    let mut hash = blake3::Hasher::new_derive_key("catcoms/studio-settlement-source/v1");
    hash.update(&source.snapshot()?);
    Ok(*hash.finalize().as_bytes())
}
impl StudioEpoch {
    /// Requires the exact held current-owner receipt and a bounded canonical close. It neither
    /// edits nor discards the source. Missing heads, wrong seed, stale authority and malformed
    /// typed projection fail closed. A fetched nonadjacent seed uses the adoption API instead.
    pub fn prepare_settlement(
        &mut self,
        close: &CloseRecord,
        group: &ServerGroup,
        tenure: u64,
    ) -> Result<StudioSettlementPlan, ReplError> {
        let close = owner::checked_close(close)?;
        if self.adopting || self.phase() != EpochPhase::Closing || self.receipts.is_faulted() {
            return Err(ReplError::EpochClosed);
        }
        let receipt = self
            .receipts
            .latest()
            .ok_or(ReplError::ReceiptConflict)?
            .clone();
        let verified = receipt.verify_current_owner(group, tenure)?;
        if receipt.document != self.logical
            || receipt.closed_epoch != self.epoch()
            || receipt.close_record_hash != close.hash()
        {
            return Err(ReplError::ReceiptConflict);
        }
        let (seed, closure) = self.checkpoint_for_close(&close, group, Some(&verified))?;
        let checkpoint = match self.target {
            StudioTarget::Index { .. } => {
                StudioIndexProjection::verify_checkpoint(&verified, seed.bytes())
            }
            StudioTarget::Flipnote { channel, .. } => {
                FlipnoteFrameProjection::verify_checkpoint(&verified, channel, seed.bytes())
            }
        }?;
        let ids = closure.domain_operation_ids()?;
        let (included, excluded): (BTreeMap<_, _>, BTreeMap<_, _>) =
            recovery::current_operations(&self.doc)?
                .into_iter()
                .partition(|(id, _)| ids.contains(id));
        let projection = self.projection()?;
        // A fully included closure can still omit deletions, conflict overflow, over-cap entries
        // or original art insertion gaps. Ask the SAME compactor, not a second guessed cap list.
        let omits_evidence = match &projection {
            StudioProjection::Index(p) => p.checkpoint_omits_evidence()?,
            StudioProjection::Flipnote(p) => p.checkpoint_omits_evidence()?,
        };
        let recovery = if excluded.is_empty() && !omits_evidence {
            None
        } else {
            Some(StudioRecovery::snapshot(
                &projection,
                self.opening.as_ref().map(|r| r.close_record_hash),
                RecoveryReason::Excluded,
                receipt.hash(),
                &excluded,
            )?)
        };
        Ok(StudioSettlementPlan {
            receipt,
            checkpoint,
            source_version: source_version(self)?,
            projection,
            included,
            excluded,
            recovery,
        })
    }

    /// Construct a separate successor only after rechecking the whole source. The caller must
    /// first durably save recovery and retire only full-envelope-matching covered intents under
    /// the same exclusive store gate before selecting it. Returning a unit is NOT installation.
    pub fn checkpoint_successor(
        &mut self,
        plan: &StudioSettlementPlan,
        group: &ServerGroup,
        tenure: u64,
    ) -> Result<Self, ReplError> {
        if self.adopting
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
