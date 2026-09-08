//! Owner decisions are derived from an eligible authenticated closure, never a UI projection.
//! The app must save the exact close AND receipt before sealing or publishing either one.
use super::*;
use crate::registry::checkpoint_registry_close;
use crate::{CloseRecord, InheritedCheckpoint};

fn check_receipt_shape(receipt: &Receipt) -> Result<(), ReplError> {
    // Public structs are not a parser boundary. Bound before encoding/signature allocation.
    if receipt.document.server_id.len() > 256
        || receipt.document.logical_key.len() > 192
        || receipt.owner_public_key.len() != 32
    {
        return Err(ReplError::EpochBound);
    }
    Receipt::decode(&receipt.encode())?;
    Ok(())
}

/// A checked, immutable decision. This is not persistence, publication or permission to prune.
/// Its close remains necessary after restart even if the still-Open source gained more edits.
pub struct RegistryOwnerDecision {
    receipt: Receipt,
    close: CloseRecord,
}

#[cfg(test)]
mod tests;
impl std::fmt::Debug for RegistryOwnerDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegistryOwnerDecision { .. }")
    }
}
impl RegistryOwnerDecision {
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }
    pub fn close(&self) -> &CloseRecord {
        &self.close
    }
}

impl RegistryEpoch {
    fn check_deciding_owner(
        &self,
        group: &ServerGroup,
        owner: &MlsDevice,
    ) -> Result<(), ReplError> {
        if group.group_id() != self.logical.server_id {
            return Err(ReplError::EpochScope);
        }
        if self.actor != owner.device_id()
            || group.designated_committer() != Some(owner.device_id())
            || group.member_signature_key(&owner.device_id()).as_deref()
                != Some(owner.public_key_bytes().as_slice())
        {
            return Err(ReplError::EpochAuthority);
        }
        if self.adopting || self.receipts.is_faulted() || self.phase() == EpochPhase::Fault {
            return Err(ReplError::ReceiptConflict);
        }
        Ok(())
    }

    /// Build a NEW decision only when no current-tenure decision is pending. `previous` is the
    /// owner's checked, published journal head, or an older-tenure choice being superseded.
    /// The store must still atomically compare/prepare its journal before this can be published.
    /// A head set above 64 refuses whole; dropping arbitrary heads is not a safe substitute.
    pub fn new_owner_decision(
        &mut self,
        group: &ServerGroup,
        owner: &MlsDevice,
        tenure: u64,
        previous: Option<&Receipt>,
    ) -> Result<RegistryOwnerDecision, ReplError> {
        self.check_deciding_owner(group, owner)?;
        if self.phase() != EpochPhase::Open {
            return Err(ReplError::EpochClosed);
        }
        if self.epoch() >= MAX_REGISTRY_EPOCH || tenure > group.epoch() {
            return Err(ReplError::EpochBound);
        }
        if let Some(previous) = previous {
            check_receipt_shape(previous)?;
            previous.restore_verified_from_vault()?;
        }
        let same_tenure = previous.filter(|r| r.tenure_start_group_epoch == tenure);
        let inherited = if let Some(previous) = same_tenure {
            previous.verify_current_owner(group, tenure)?;
            if previous.document != self.logical || self.opening.as_ref() != Some(previous) {
                return Err(ReplError::ReceiptConflict);
            }
            previous.inherited.clone()
        } else {
            if previous
                .is_some_and(|r| r.document != self.logical || r.tenure_start_group_epoch >= tenure)
            {
                return Err(ReplError::ReceiptConflict);
            }
            match &self.opening {
                None if self.epoch() == 0 => InheritedCheckpoint::EpochZero,
                Some(opening) if opening.tenure_start_group_epoch < tenure => {
                    // The source constructor/restore already verified this exact opening seed.
                    // Do not take inheritance from a journal whose checkpoint was never installed.
                    InheritedCheckpoint::Checkpoint {
                        epoch: self.epoch(),
                        close_record_hash: opening.close_record_hash,
                        seed_change_hash: opening.seed_change_hash,
                    }
                }
                _ => return Err(ReplError::ReceiptConflict),
            }
        };
        self.refresh_owner(group)?;
        let mut heads = self.doc.heads();
        heads.sort_unstable();
        let close = CloseRecord::sign(&self.logical, self.doc_id(), self.epoch(), heads, owner)?;
        // This validates full signed operation bytes, per-device shares, dependency closure and
        // the typed projection at THESE heads. The live projection is not a closure proof.
        let (seed, _) =
            checkpoint_registry_close(&mut self.doc, &self.gate, self.bucket, &close, group, None)?;
        let receipt = Receipt::sign(
            self.logical.clone(),
            self.epoch(),
            close.hash(),
            seed.change_hash(),
            tenure,
            inherited,
            owner,
        )?;
        receipt.verify_current_owner(group, tenure)?;
        Ok(RegistryOwnerDecision { receipt, close })
    }

    /// Recheck an irrevocable saved decision without ever regenerating it from current heads.
    /// Open is possible after a journal-save / seal failure; later edits then enter recovery.
    /// An installed retry needs no old closure, but still verifies the exact close signature.
    pub fn resume_owner_decision(
        &mut self,
        group: &ServerGroup,
        owner: &MlsDevice,
        tenure: u64,
        receipt: &Receipt,
        close: &CloseRecord,
    ) -> Result<RegistryOwnerDecision, ReplError> {
        self.check_deciding_owner(group, owner)?;
        if close.server_id.len() > 256
            || close.heads.len() > 64
            || close.author_public_key.len() != 32
        {
            return Err(ReplError::EpochBound);
        }
        let close = CloseRecord::decode(&close.encode())?;
        check_receipt_shape(receipt)?;
        let verified = receipt.verify_current_owner(group, tenure)?;
        if receipt.document != self.logical
            || receipt.closed_epoch >= MAX_REGISTRY_EPOCH
            || close.closed_epoch != receipt.closed_epoch
            || close.hash() != receipt.close_record_hash
        {
            return Err(ReplError::ReceiptConflict);
        }
        close.verify_for(&self.logical, close.doc_id, group, Some(&verified))?;
        if !self.opened_by(receipt) {
            if self.epoch() != receipt.closed_epoch
                || self.doc_id() != close.doc_id
                || !matches!(self.phase(), EpochPhase::Open | EpochPhase::Closing)
                || (self.phase() == EpochPhase::Closing && self.receipts.latest() != Some(receipt))
            {
                return Err(ReplError::ReceiptConflict);
            }
            self.refresh_owner(group)?;
            let (seed, _) = checkpoint_registry_close(
                &mut self.doc,
                &self.gate,
                self.bucket,
                &close,
                group,
                Some(&verified),
            )?;
            if seed.change_hash() != receipt.seed_change_hash {
                return Err(ReplError::ReceiptConflict);
            }
        }
        Ok(RegistryOwnerDecision {
            receipt: receipt.clone(),
            close,
        })
    }
}
