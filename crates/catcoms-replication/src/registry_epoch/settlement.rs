//! Receipt-bound, read-only preparation. A plan is recovery INPUT, never permission to prune.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::registry::checkpoint_registry_close;
use crate::{CloseRecord, LocalIntent, VerifiedCheckpoint};

/// Exact checkpoint and recovery inputs computed from one accepted source version. No method
/// installs this plan, retires intents or discards history. A future settlement transaction must
/// reload and revalidate under the document gate, persist typed recovery first, then install.
/// Keeping fields private prevents constructing a plan from an unchecked projection or seed.
/// Outputs are bounded by the retained source caps; fitting a future encoded recovery snapshot
/// and its storage reservation is a separate preflight which can still hold settlement Closing.
pub struct RegistrySettlementPlan {
    receipt: Receipt,
    checkpoint: VerifiedCheckpoint,
    source_version: [u8; 32],
    source_projection: RegistryProjection,
    included: BTreeSet<[u8; 32]>,
    included_operations: BTreeMap<[u8; 32], LocalIntent>,
    excluded: BTreeMap<[u8; 32], LocalIntent>,
    source_base_close: Option<[u8; 32]>,
}

impl std::fmt::Debug for RegistrySettlementPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistrySettlementPlan")
            .field("epoch", &self.source_projection.epoch)
            .field("included_operations", &self.included.len())
            .field("excluded_operations", &self.excluded.len())
            .finish_non_exhaustive()
    }
}

impl RegistrySettlementPlan {
    /// Close which opened the SOURCE epoch, not the close selecting its successor.
    pub fn source_base_close(&self) -> Option<[u8; 32]> {
        self.source_base_close
    }

    /// Build bounded typed recovery evidence. None means no excluded operations, overflow or
    /// tombstones need retention. This is not persistence or permission to install/prune.
    pub fn recovery_snapshot(&self) -> Result<Option<crate::RecoverySnapshot>, ReplError> {
        crate::registry::RegistryRecovery::snapshot_for_plan(self)
    }

    /// Selected held receipt, freshly checked against the supplied current owner and tenure.
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }

    /// Canonical seed rebuilt from exactly the receipted closure and verified by expected hash.
    pub fn checkpoint(&self) -> &VerifiedCheckpoint {
        &self.checkpoint
    }

    /// Fingerprint of the entire normalized local restart unit, including accepted log, seed,
    /// gate and receipts. Local serialization-version dependent; NOT a wire identifier or proof
    /// of durability/currency. Scope plus receipt alone cannot identify excluded accepted work.
    pub fn source_version(&self) -> [u8; 32] {
        self.source_version
    }

    /// Full source materialization, including overflow and tombstones omitted from the seed.
    pub fn source_projection(&self) -> &RegistryProjection {
        &self.source_projection
    }

    /// Author-derived ids actually inside this closure, not merely present in its marker map.
    pub fn included_operation_ids(&self) -> &BTreeSet<[u8; 32]> {
        &self.included
    }

    /// Full authenticated envelopes covered by the receipt. Retirement must compare these,
    /// not ids alone: a nonce-derived id deliberately does not bind the operation body.
    pub fn included_operations(&self) -> &BTreeMap<[u8; 32], LocalIntent> {
        &self.included_operations
    }

    /// Accepted source operations outside the closure, in canonical id order. These are
    /// author-attributed recovery data, NOT replay permission: only an operation's own local
    /// author may journal/replay it. Late quarantined bodies were never accepted and are absent.
    pub fn excluded_operations(&self) -> &BTreeMap<[u8; 32], LocalIntent> {
        &self.excluded
    }

    /// Reject stale computation even when the scope and selected receipt still match. This is
    /// only a local version comparison; callers must separately recheck current authority and
    /// hold the exclusive document gate throughout any future durable settlement transaction.
    pub fn matches_source(&self, source: &mut RegistryEpoch) -> Result<bool, ReplError> {
        Ok(self.source_version == source_version(source)?)
    }
}

pub(super) fn source_version(source: &mut RegistryEpoch) -> Result<[u8; 32], ReplError> {
    let mut hash = blake3::Hasher::new_derive_key("catcoms/registry-settlement-source/v1");
    hash.update(&source.snapshot()?);
    Ok(*hash.finalize().as_bytes())
}

impl RegistryEpoch {
    /// True only for this exact opening receipt, never a later epoch or a fault. Installed
    /// retries may flush this unit, but must not reconstruct its seed over subsequent edits.
    pub fn opened_by(&self, receipt: &Receipt) -> bool {
        self.opening.as_ref() == Some(receipt)
            && matches!(self.phase(), EpochPhase::Open | EpochPhase::Closing)
    }

    /// Build a separate successor after rechecking the entire source and current authority.
    /// This is not permission to discard the source: the store must first flush it, save
    /// recovery and retire only receipt-covered intents before atomically selecting this unit.
    pub fn checkpoint_successor(
        &mut self,
        plan: &RegistrySettlementPlan,
        group: &ServerGroup,
        expected_tenure_start: u64,
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
            self.bucket,
            self.actor,
            plan.receipt.clone(),
            expected_tenure_start,
            plan.checkpoint.bytes(),
        )?;
        // Carry anti-replay repair state; the standalone constructor starts a fresh book.
        successor.receipts = self.receipts.clone();
        successor.receipts.mark_latest_installed();
        Ok(successor)
    }

    /// Prepare recovery/checkpoint inputs without changing accepted history, gate or receipts.
    /// Closing and an exact held current-owner receipt are required. The caller supplies the
    /// independently observed tenure start, never a value taken on trust from that receipt.
    /// Failure (missing heads, malformed close, wrong seed, stale authority) leaves source intact.
    pub fn prepare_settlement(
        &mut self,
        close: &CloseRecord,
        group: &ServerGroup,
        expected_tenure_start: u64,
    ) -> Result<RegistrySettlementPlan, ReplError> {
        // CloseRecord is public: bound fields BEFORE encode/hash (including its u16 head count).
        if close.server_id.len() > 256
            || close.author_public_key.len() != 32
            || close.heads.len() > 64
        {
            return Err(ReplError::EpochBound);
        }
        let close = CloseRecord::decode(&close.encode())?;
        if self.adopting || self.phase() != EpochPhase::Closing || self.receipts.is_faulted() {
            return Err(ReplError::EpochClosed);
        }
        let receipt = self.receipts.latest().ok_or(ReplError::ReceiptConflict)?;
        let verified = receipt.verify_current_owner(group, expected_tenure_start)?;
        // Historical-author authorization in the close verifier is not enough: even a CURRENT
        // author must supply the exact selected close, never a different otherwise valid one.
        if receipt.document != self.logical
            || receipt.closed_epoch != self.epoch()
            || receipt.close_record_hash != close.hash()
        {
            return Err(ReplError::ReceiptConflict);
        }
        let receipt = receipt.clone();
        let (seed, closure) = checkpoint_registry_close(
            &mut self.doc,
            &self.gate,
            self.bucket,
            &close,
            group,
            Some(&verified),
        )?;
        let checkpoint =
            RegistryProjection::verify_checkpoint(&verified, self.bucket, seed.bytes())?;
        let included = closure.domain_operation_ids()?;
        let mut included_operations = BTreeMap::new();
        let mut excluded = BTreeMap::new();
        for op in self.doc.signed_log() {
            let operation = op.parsed_domain_op()?.ok_or(ReplError::Malformed)?;
            let id = operation.id(&op.author_device);
            let target = if included.contains(&id) {
                &mut included_operations
            } else {
                &mut excluded
            };
            target.insert(
                id,
                LocalIntent {
                    author: op.author_device,
                    operation,
                },
            );
        }
        Ok(RegistrySettlementPlan {
            receipt,
            checkpoint,
            included,
            included_operations,
            excluded,
            source_projection: self.projection()?,
            source_version: source_version(self)?,
            source_base_close: self
                .opening
                .as_ref()
                .map(|receipt| receipt.close_record_hash),
        })
    }
}

#[cfg(test)]
mod tests;
