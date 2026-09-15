//! Registry uses the same strictly first-new-tenure takeover as Studio. Pointer contents come
//! only from the held closed source, never a clock, remote hint or freshly reconstructed index.

use super::*;
use crate::CheckpointSeed;

impl RegistryEpoch {
    /// Local routing hint; not permission to replace a current decision or skip recovery.
    pub fn owner_rotation_needs_adoption(&self, tenure: u64) -> bool {
        self.phase() == EpochPhase::Closing
            && (self.adopting
                || self
                    .receipts
                    .latest()
                    .is_some_and(|r| r.tenure_start_group_epoch < tenure))
    }

    /// Exact frozen-source counterpart of `new_owner_decision`. Persist the returned decision
    /// before whole-source recovery/adoption; do not use it to reset an ordinary pending seal.
    pub fn frozen_owner_decision(
        &mut self,
        group: &ServerGroup,
        owner: &MlsDevice,
        tenure: u64,
        previous: Option<&Receipt>,
        saved_close: Option<&CloseRecord>,
    ) -> Result<(RegistryOwnerDecision, CheckpointSeed), ReplError> {
        self.check_current_owner(group, owner)?;
        if self.phase() != EpochPhase::Closing {
            return Err(ReplError::EpochClosed);
        }
        if self.epoch() >= MAX_REGISTRY_EPOCH {
            return Err(ReplError::EpochBound);
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
            let close = saved_close.ok_or(ReplError::ReceiptConflict)?;
            if close.server_id.len() > 256
                || close.author_public_key.len() != 32
                || close.heads.len() > 64
            {
                return Err(ReplError::EpochBound);
            }
            let close = CloseRecord::decode(&close.encode())?;
            let verified = receipt.verify_current_owner(group, tenure)?;
            if close.closed_epoch != self.epoch() || close.hash() != receipt.close_record_hash {
                return Err(ReplError::ReceiptConflict);
            }
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
            return Ok((
                RegistryOwnerDecision {
                    receipt: receipt.clone(),
                    close,
                },
                seed,
            ));
        }
        let mut heads = self.doc.heads();
        heads.sort_unstable();
        let close = CloseRecord::sign(&self.logical, self.doc_id(), self.epoch(), heads, owner)?;
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
        Ok((RegistryOwnerDecision { receipt, close }, seed))
    }
}
