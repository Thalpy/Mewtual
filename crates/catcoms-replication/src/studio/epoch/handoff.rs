//! Private candidate work, derived from the actual installed successor. No vault durability.
use super::*;
use crate::IntentLedger;

mod preparation;
pub(in crate::studio) use preparation::PreparedOverlayChanges;

impl StudioEpoch {
    pub(in crate::studio) fn copy_handoff_source(
        &mut self,
        group: &ServerGroup,
    ) -> Result<Self, ReplError> {
        Self::restore(&self.snapshot()?, group, self.target, self.actor)
    }
    /// Cheap exact-history fence over authenticated vault bytes. It does not install/decode
    /// an editable source or infer authority; malformed framing refuses the replacement.
    pub fn preserves_vault_source(
        &mut self,
        bytes: &[u8],
        protected: &std::collections::BTreeSet<[u8; 32]>,
    ) -> Result<bool, ReplError> {
        if bytes.len() > MAX_STUDIO_EPOCH_SNAPSHOT_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        if !matches!(d.get_u8().map_err(|_| ReplError::Malformed)?, 1 | 2) {
            return Err(ReplError::Malformed);
        }
        if d.get_bytes().map_err(|_| ReplError::Malformed)? != self.target.channel() {
            return Ok(false);
        }
        let opening = field(&mut d, MAX_RECEIPT_BYTES)?;
        let seed = field(&mut d, MAX_CHECKPOINT_BYTES)?;
        field(&mut d, MAX_RECEIPT_BOOK_BYTES)?;
        let gate = EpochGate::decode(field(&mut d, MAX_EPOCH_GATE_BYTES)?)?;
        if gate.verify_scope(&self.logical, self.doc_id()).is_err()
            || gate.epoch() != self.epoch()
            || self
                .opening
                .as_ref()
                .map(Receipt::encode)
                .unwrap_or_default()
                != opening
            || self.doc.checkpoint_bytes()?.unwrap_or_default() != seed
        {
            return Ok(false);
        }
        let count = d.get_u32().map_err(|_| ReplError::Malformed)? as usize;
        if count > MAX_EPOCH_OPERATIONS {
            return Err(ReplError::EpochBound);
        }
        let next: std::collections::BTreeSet<_> = self
            .doc
            .signed_log()
            .iter()
            .map(|op| *blake3::hash(&op.encode()).as_bytes())
            .collect();
        let mut total = 0usize;
        let mut kept = true;
        let mut old = std::collections::BTreeSet::new();
        for _ in 0..count {
            let raw = field(&mut d, MAX_SIGNED_EPOCH_OP_BYTES)?;
            total = total.checked_add(raw.len()).ok_or(ReplError::EpochBound)?;
            if total > MAX_EPOCH_BYTES {
                return Err(ReplError::EpochBound);
            }
            let hash = *blake3::hash(raw).as_bytes();
            kept &= next.contains(&hash);
            old.insert(hash);
        }
        d.finish().map_err(|_| ReplError::Malformed)?;
        for op in self.doc.signed_log() {
            let domain = op.parsed_domain_op()?.ok_or(ReplError::Malformed)?;
            if protected.contains(&domain.id(&op.author_device))
                && !old.contains(blake3::hash(&op.encode()).as_bytes())
            {
                return Ok(false);
            }
        }
        Ok(kept)
    }
    pub(in crate::studio) fn check_overlay_successor(
        &mut self,
        overlay: &StudioOverlay,
        ledger: &IntentLedger,
    ) -> Result<(), ReplError> {
        if self.target != overlay.target() || self.document() != ledger.document() {
            return Err(ReplError::EpochScope);
        }
        if self.actor != overlay.author()
            || self.gate.owner()
                != DeviceId::from_public_key_bytes(&overlay.receipt().owner_public_key)
        {
            return Err(ReplError::EpochAuthority);
        }
        if self.phase() != EpochPhase::Open
            || self.adopting
            || self.opening.as_ref() != Some(overlay.receipt())
            || self.epoch()
                != overlay
                    .receipt()
                    .closed_epoch
                    .checked_add(1)
                    .ok_or(ReplError::EpochBound)?
            || self.doc.checkpoint_bytes()?.as_deref() != Some(overlay.seed())
            || self.op_count() != 0
            || self.projection()? != overlay.base_projection()?
        {
            return Err(ReplError::EpochClosed);
        }
        Ok(())
    }

    /// Exact signed bytes, including delta and timestamp. Seed markers are deliberately absent.
    pub(in crate::studio) fn overlay_signed_hash(
        &self,
        intent: &LocalIntent,
    ) -> Result<Option<[u8; 32]>, ReplError> {
        Ok(self.held(intent.author, &intent.operation)?.map(|op| {
            blake3::derive_key("catcoms/studio-overlay-signed-operation/v1", &op.encode())
        }))
    }
}
