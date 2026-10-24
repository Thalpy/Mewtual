//! A new owner can finish an interrupted old-owner rotation without admitting another edit
//! into its frozen source. The result still needs the existing journal and adoption transaction.

use super::*;

impl StudioEpoch {
    /// Local routing hint only; `frozen_owner_decision` repeats authority and source checks.
    pub fn owner_rotation_needs_adoption(&self, tenure: u64) -> bool {
        self.phase() == EpochPhase::Closing
            && (self.adopting
                || self
                    .receipts
                    .latest()
                    .is_some_and(|r| r.tenure_start_group_epoch < tenure))
    }

    /// Prepare/resume the first decision of a NEW tenure on the unchanged Closing source.
    /// `previous`/`saved_close` come from the checked durable journal. A missing saved close
    /// refuses exact resume; a missing/underfull closure holds instead of reopening the source.
    /// Returns the verified exact seed so the store can use whole-version checkpoint adoption.
    pub fn frozen_owner_decision(
        &mut self,
        group: &ServerGroup,
        owner: &MlsDevice,
        tenure: u64,
        previous: Option<&Receipt>,
        saved_close: Option<&CloseRecord>,
    ) -> Result<(StudioOwnerDecision, CheckpointSeed), ReplError> {
        self.check_current_owner(group, owner)?;
        if self.phase() != EpochPhase::Closing {
            return Err(ReplError::EpochClosed);
        }
        let inherited = crate::epoch::succession::frozen_owner_inheritance(
            &self.logical,
            self.epoch(),
            self.opening.as_ref(),
            self.receipts.latest(),
            previous,
            group,
            tenure,
        )?;
        self.refresh_owner(group)?;
        if let Some(receipt) = previous.filter(|r| r.tenure_start_group_epoch == tenure) {
            let close = checked_close(saved_close.ok_or(ReplError::ReceiptConflict)?)?;
            let verified = receipt.verify_current_owner(group, tenure)?;
            if close.closed_epoch != self.epoch() || close.hash() != receipt.close_record_hash {
                return Err(ReplError::ReceiptConflict);
            }
            let (seed, _) = self.checkpoint_for_close(&close, group, Some(&verified))?;
            if seed.change_hash() != receipt.seed_change_hash {
                return Err(ReplError::ReceiptConflict);
            }
            return Ok((
                StudioOwnerDecision {
                    receipt: receipt.clone(),
                    close,
                },
                seed,
            ));
        }
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
        Ok((StudioOwnerDecision { receipt, close }, seed))
    }
}
