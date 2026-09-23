//! Constant-sized historical repair evidence. It is neither network authority nor a journal:
//! the source installer still owns durable recovery and the gate transition. Keeping both full
//! receipts lets restart screen the losing inherited baseline after the selected head advances.

use super::*;

/// Validate historical equivocation without requiring a repair or a current-owner claim.
///
/// Both receipts must have their canonical wire shape and authentic signatures, name the full
/// logical document and the same tenure, and actually conflict. Two successive consistent
/// receipts are progress, not evidence. This mints no verified-receipt capability: callers still
/// bind their numeric server/channel/store scope and separately establish live repair authority.
pub fn conflicting_receipt_pair(
    document: &LogicalDocument,
    a: &Receipt,
    b: &Receipt,
) -> Result<(), ReplError> {
    for receipt in [a, b] {
        // Public fields can bypass decode. Check allocation bounds before making an encoded
        // copy, then require the decoder's entire schema rather than just a valid signature.
        if receipt.document.server_id.len() > MAX_SERVER_ID_BYTES
            || receipt.document.logical_key.len() > MAX_LOGICAL_KEY_BYTES
            || receipt.owner_public_key.len() != 32
        {
            return Err(ReplError::EpochBound);
        }
        if Receipt::decode(&receipt.encode())? != *receipt {
            return Err(ReplError::Malformed);
        }
        receipt.verify_signature_only()?;
        if &receipt.document != document {
            return Err(ReplError::EpochScope);
        }
    }
    if !receipts_conflict(a, b) {
        return Err(ReplError::ReceiptConflict);
    }
    Ok(())
}

impl ReceiptRepair {
    /// Bind this v2 decision to both complete conflicting receipts and a nonzero sequence.
    ///
    /// This checks evidence, NOT this repair's signature or present authority. Live callers must
    /// first use `verify_current_owner` with independently observed issuer tenure; sealed restore
    /// additionally checks the historical repair signature, selected/losing roles and sequence.
    /// Receipt order is immaterial, but the decision's hash pair must be canonical and exact.
    pub fn check_evidence(&self, a: &Receipt, b: &Receipt) -> Result<(), ReplError> {
        conflicting_receipt_pair(&self.document, a, b)?;
        let mut hashes = [a.hash(), b.hash()];
        hashes.sort_unstable();
        if self.tenure_id != a.tenure_id
            || self.receipt_hashes != hashes
            || !self.receipt_hashes.contains(&self.selected_receipt_hash)
            || self.issuer_tenure_start_group_epoch.is_none()
            || self.repair_sequence == 0
        {
            return Err(ReplError::ReceiptConflict);
        }
        Ok(())
    }
}

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
        self.repair.check_evidence(&self.selected, &self.losing)?;
        if document != Some(&self.repair.document)
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
    /// Latest resolved sequence, including after a cross-tenure repair removes the current head.
    /// An absent owner journal must not let issuance restart below this durable anti-replay mark.
    pub fn repair_sequence(&self) -> u64 {
        self.repair_sequence
    }

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
