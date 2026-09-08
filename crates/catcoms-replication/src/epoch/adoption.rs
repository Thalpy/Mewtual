//! A discovered checkpoint can be distant from the source still held locally. Keep its
//! receipt selection separate from ordinary adjacent-epoch settlement, without discarding a
//! single accepted operation. Only a recovery-first installer may replace this frozen source.

use super::*;

impl ReceiptBook {
    /// Current authority is supplied independently by the caller (fresh scoped discovery in
    /// the network adapter). A receipt alone is not proof of currency. Fault is an OUTCOME:
    /// the caller must persist the changed book/gate even though installation cannot proceed.
    pub(crate) fn ingest_adoption(
        &mut self,
        receipt: Receipt,
        group: &ServerGroup,
        tenure_start: u64,
        gate: &EpochGate,
        opening: Option<&Receipt>,
    ) -> Result<ReceiptIngest, ReplError> {
        receipt.verify_current_owner(group, tenure_start)?;
        if receipt.document != gate.document {
            return Err(ReplError::EpochScope);
        }
        let mut inner = gate.inner.lock().expect("epoch gate poisoned");
        if inner.phase == EpochPhase::Settled {
            return Err(ReplError::EpochClosed);
        }
        if self.is_faulted() {
            return Ok(ReceiptIngest::Fault);
        }
        let mut next = self.clone();
        // Retargeting keeps one previous target, while the original seed has its own opening
        // receipt. Either can expose equivocation BELOW the selected high-water. Screening
        // before Stale is essential; neither is an unbounded historical audit trail.
        let conflicting = opening
            .into_iter()
            .chain(self.previous_until_installed.iter())
            .find(|prior| receipts_conflict(prior, &receipt));
        let outcome = if let Some(prior) = conflicting {
            next.fault = Some(canonical_receipt_pair(prior.clone(), receipt));
            ReceiptIngest::Fault
        } else {
            next.ingest_verified(receipt)?
        };
        match outcome {
            ReceiptIngest::Stale => return Ok(outcome),
            ReceiptIngest::Fault => {
                inner.phase = EpochPhase::Fault;
                inner.receipt_hash = None;
            }
            ReceiptIngest::Advanced | ReceiptIngest::Duplicate => {
                if inner.phase == EpochPhase::Fault {
                    return Err(ReplError::ReceiptConflict);
                }
                inner.phase = EpochPhase::Closing;
                inner.receipt_hash = Some(
                    next.latest
                        .as_ref()
                        .ok_or(ReplError::ReceiptConflict)?
                        .hash(),
                );
            }
        }
        // One lock protects the receipt decision and the edit/inbound seal. Retargeting never
        // clears the original log, accepted metadata, or bounded post-seal quarantine.
        *self = next;
        Ok(outcome)
    }

    /// Additional checks for a version-tagged adoption restart. Ordinary restart still uses
    /// the stricter adjacent-epoch rules. Authentication of these historical signatures is
    /// local-vault provenance, not permission to accept an old owner from the network.
    pub(super) fn verify_adoption_state(
        &self,
        document: &LogicalDocument,
        inner: &EpochGateInner,
        opening: Option<&Receipt>,
    ) -> Result<(), ReplError> {
        let latest = self.latest.as_ref().ok_or(ReplError::Malformed)?;
        for receipt in self
            .latest
            .iter()
            .chain(self.previous_until_installed.iter())
            .chain(self.fault.iter().flat_map(|(a, b)| [a, b]))
        {
            if &receipt.document != document
                || receipt.closed_epoch >= crate::registry::MAX_REGISTRY_EPOCH
            {
                return Err(ReplError::EpochScope);
            }
            receipt.restore_verified_from_vault()?;
        }
        let valid = match inner.phase {
            EpochPhase::Closing => {
                !self.is_faulted()
                    && inner.receipt_hash == Some(latest.hash())
                    && self.previous_until_installed.as_ref().is_none_or(|prior| {
                        prior.tenure_id != latest.tenure_id
                            || (prior.closed_epoch < latest.closed_epoch
                                && TenureSelection::from(prior) == TenureSelection::from(latest))
                    })
                    // A source can be far beyond the retained book's previous target. It is
                    // another high-water anchor, not just an inheritance-conflict check.
                    && opening.is_none_or(|prior| {
                        prior.tenure_id != latest.tenure_id
                            || (prior.closed_epoch < latest.closed_epoch
                                && TenureSelection::from(prior) == TenureSelection::from(latest))
                    })
            }
            EpochPhase::Fault => self.fault.as_ref().is_some_and(|(a, b)| {
                receipts_conflict(a, b)
                    && (latest == a
                        || latest == b
                        || [opening, self.previous_until_installed.as_ref()]
                            .into_iter()
                            .flatten()
                            .any(|anchor| anchor == a || anchor == b))
            }),
            EpochPhase::Open | EpochPhase::Settled => false,
        };
        if !valid {
            return Err(ReplError::Malformed);
        }
        Ok(())
    }
}
