//! Studio's typed owner-decision adapter. P1 still owns close validation, quotas, receipts and
//! finality. These immutable outputs must be journaled together before sealing or publication.
use super::*;
use crate::{CheckpointSeed, CloseRecord, ClosureStats, InheritedCheckpoint, VerifiedReceipt};

/// Checked close/receipt pair, NOT a durable decision or permission to publish/prune. The
/// app must persist this exact pair; after a crash it must resume, never regenerate, the close.
pub struct StudioOwnerDecision {
    receipt: Receipt,
    close: CloseRecord,
}
impl std::fmt::Debug for StudioOwnerDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioOwnerDecision { .. }")
    }
}
impl StudioOwnerDecision {
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }
    pub fn close(&self) -> &CloseRecord {
        &self.close
    }
}

fn checked_receipt(receipt: &Receipt) -> Result<(), ReplError> {
    // Public Rust structs bypass wire parsing; bound allocations before encode/verification.
    if receipt.document.server_id.len() > 256
        || receipt.document.logical_key.len() > 192
        || receipt.owner_public_key.len() != 32
    {
        return Err(ReplError::EpochBound);
    }
    Receipt::decode(&receipt.encode())?;
    Ok(())
}
pub(super) fn checked_close(close: &CloseRecord) -> Result<CloseRecord, ReplError> {
    if close.server_id.len() > 256 || close.author_public_key.len() != 32 || close.heads.len() > 64
    {
        return Err(ReplError::EpochBound);
    }
    CloseRecord::decode(&close.encode())
}

impl StudioEpoch {
    /// Only this owned, typed-admitted epoch can supply the source. A raw signed closure alone
    /// cannot prove Studio operation semantics or replace the prior gated ingest checks.
    pub(super) fn checkpoint_for_close(
        &mut self,
        close: &CloseRecord,
        group: &ServerGroup,
        receipt: Option<&VerifiedReceipt>,
    ) -> Result<(CheckpointSeed, ClosureStats), ReplError> {
        self.gate
            .verify_scope(&self.target.document(&group.group_id())?, self.doc_id())?;
        if close.closed_epoch != self.epoch() {
            return Err(ReplError::EpochScope);
        }
        let unsigned_seed = self.doc.checkpoint_origin().map(|o| o.seed_hash());
        let closure = close.verify_and_validate(
            &self.logical,
            self.doc_id(),
            group,
            receipt,
            &mut self.doc,
            unsigned_seed,
        )?;
        let projection = self.doc.projection_for_closure(&closure.operations)?;
        let typed = self.target.read(&self.logical, self.epoch(), &projection)?;
        Ok((typed.checkpoint(close.hash())?, closure))
    }

    fn check_deciding_owner(
        &self,
        group: &ServerGroup,
        owner: &MlsDevice,
    ) -> Result<(), ReplError> {
        self.check_current_owner(group, owner)?;
        if self.adopting {
            return Err(ReplError::ReceiptConflict);
        }
        Ok(())
    }

    // Frozen takeover reuses identity/Fault checks without relaxing ordinary decision/seal
    // rules. It has its own strictly first-new-tenure and whole-source adoption path below.
    fn check_current_owner(&self, group: &ServerGroup, owner: &MlsDevice) -> Result<(), ReplError> {
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
        if self.receipts.is_faulted() || self.phase() == EpochPhase::Fault {
            return Err(ReplError::ReceiptConflict);
        }
        Ok(())
    }

    /// Derive a new decision from ALL current heads. The caller supplies independently observed
    /// tenure and the checked journal head, and must atomically compare/persist the resulting
    /// pair before use. More than 64 heads refuses, never truncates. Studio has no Registry-only
    /// 4096-epoch ceiling; normal checked integer/seed bounds still apply.
    pub fn new_owner_decision(
        &mut self,
        group: &ServerGroup,
        owner: &MlsDevice,
        tenure: u64,
        previous: Option<&Receipt>,
    ) -> Result<StudioOwnerDecision, ReplError> {
        self.check_deciding_owner(group, owner)?;
        if self.phase() != EpochPhase::Open {
            return Err(ReplError::EpochClosed);
        }
        if tenure > group.epoch() {
            return Err(ReplError::EpochBound);
        }
        if let Some(previous) = previous {
            checked_receipt(previous)?;
            previous.restore_verified_from_vault()?;
        }
        // Identical to P1's Registry tenure policy: a new tenure inherits the INSTALLED opening,
        // while later receipts repeat the journal baseline. Losing the journal is not a reset.
        let inherited =
            if let Some(previous) = previous.filter(|r| r.tenure_start_group_epoch == tenure) {
                previous.verify_current_owner(group, tenure)?;
                if previous.document != self.logical || self.opening.as_ref() != Some(previous) {
                    return Err(ReplError::ReceiptConflict);
                }
                previous.inherited.clone()
            } else {
                if previous.is_some_and(|r| {
                    r.document != self.logical || r.tenure_start_group_epoch >= tenure
                }) {
                    return Err(ReplError::ReceiptConflict);
                }
                match &self.opening {
                    None if self.epoch() == 0 => InheritedCheckpoint::EpochZero,
                    Some(opening) if opening.tenure_start_group_epoch < tenure => {
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
        let (seed, _) = self.checkpoint_for_close(&close, group, None)?;
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
        Ok(StudioOwnerDecision { receipt, close })
    }

    /// Resume the exact persisted pair, including after later Open edits or completed install.
    /// Does not reseed an installed successor or make later operations part of the old closure.
    pub fn resume_owner_decision(
        &mut self,
        group: &ServerGroup,
        owner: &MlsDevice,
        tenure: u64,
        receipt: &Receipt,
        close: &CloseRecord,
    ) -> Result<StudioOwnerDecision, ReplError> {
        self.check_deciding_owner(group, owner)?;
        let close = checked_close(close)?;
        checked_receipt(receipt)?;
        let verified = receipt.verify_current_owner(group, tenure)?;
        if receipt.document != self.logical
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
            let (seed, _) = self.checkpoint_for_close(&close, group, Some(&verified))?;
            if seed.change_hash() != receipt.seed_change_hash {
                return Err(ReplError::ReceiptConflict);
            }
        }
        Ok(StudioOwnerDecision {
            receipt: receipt.clone(),
            close,
        })
    }
}

mod frozen;
#[cfg(test)]
mod tests;
