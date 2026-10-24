//! Constant-sized historical repair evidence. It is neither network authority nor a journal:
//! the source installer still owns durable recovery and the gate transition. Keeping both full
//! receipts lets restart screen the losing inherited baseline after the selected head advances.

use super::*;

#[derive(Clone, Debug)]
pub(super) struct ResolvedRepair {
    pub(super) repair: ReceiptRepair,
    pub(super) selected: Receipt,
    pub(super) losing: Receipt,
}

impl ResolvedRepair {
    pub(super) fn encode_into(&self, e: &mut Encoder) {
        for bytes in [
            self.repair.encode(),
            self.selected.encode(),
            self.losing.encode(),
        ] {
            e.put_bytes(&bytes).expect("bounded repair evidence fits");
        }
    }

    pub(super) fn decode_from(d: &mut Decoder<'_>) -> Result<Self, ReplError> {
        Ok(Self {
            repair: ReceiptRepair::decode(d.get_bytes().map_err(|_| ReplError::Malformed)?)?,
            selected: Receipt::decode(d.get_bytes().map_err(|_| ReplError::Malformed)?)?,
            losing: Receipt::decode(d.get_bytes().map_err(|_| ReplError::Malformed)?)?,
        })
    }

    pub(super) fn verify(
        &self,
        document: Option<&LogicalDocument>,
        sequence: u64,
    ) -> Result<(), ReplError> {
        self.repair.verify_historical()?;
        self.selected.restore_verified_from_vault()?;
        self.losing.restore_verified_from_vault()?;
        let mut hashes = [self.selected.hash(), self.losing.hash()];
        hashes.sort_unstable();
        if document != Some(&self.repair.document)
            || self.selected.document != self.repair.document
            || self.losing.document != self.repair.document
            || self.repair.issuer_tenure_start_group_epoch.is_none()
            || self.selected.tenure_id != self.repair.tenure_id
            || self.losing.tenure_id != self.repair.tenure_id
            || !receipts_conflict(&self.selected, &self.losing)
            || self.repair.receipt_hashes != hashes
            || self.repair.selected_receipt_hash != self.selected.hash()
            || sequence != self.repair.repair_sequence
            || sequence == 0
        {
            return Err(ReplError::ReceiptConflict);
        }
        Ok(())
    }
}

impl ReceiptBook {
    /// Latest exact resolved repair, retained across ordinary head advancement/checkpointing.
    /// Historical bytes alone never authorize a new repair or prove the current owner's tenure.
    pub fn latest_repair(&self) -> Option<&ReceiptRepair> {
        self.resolved_repair
            .as_ref()
            .map(|resolved| &resolved.repair)
    }

    /// All receipt-admission paths screen this BEFORE their opening/high-water shortcuts.
    /// Different inherited baselines identify a losing branch; same-baseline same-epoch closes
    /// identify only the named loser because receipts do not contain an ancestry chain. A third
    /// baseline remains new equivocation, not silently covered by the latest signed choice.
    pub(super) fn is_repaired_loser(&self, receipt: &Receipt) -> bool {
        self.resolved_repair.as_ref().is_some_and(|resolved| {
            receipt.document == resolved.repair.document
                && receipt.tenure_id == resolved.repair.tenure_id
                && (receipt.hash() == resolved.losing.hash()
                    || (TenureSelection::from(&resolved.selected)
                        != TenureSelection::from(&resolved.losing)
                        && TenureSelection::from(receipt)
                            == TenureSelection::from(&resolved.losing)))
        })
    }
}

#[cfg(test)]
mod tests;
