//! Private candidate work, derived from the actual installed successor. No vault durability.
use super::*;
use crate::IntentLedger;

mod preparation;
pub(in crate::studio) use preparation::PreparedOverlayChanges;

/// The header of a vault source record, read without decoding a single operation.
///
/// The same framing `StudioEpoch::snapshot` writes and `restore_scoped` reads, in the same order and
/// with the same bounds, stopping at the operation count. For classification only: nothing here is
/// verified against the operations, so it must never stand in for a restore.
struct VaultShape<'a> {
    adopting: bool,
    opening: &'a [u8],
    seed: &'a [u8],
    gate: EpochGate,
    operations: usize,
}
impl<'a> VaultShape<'a> {
    fn read(bytes: &'a [u8], target: StudioTarget) -> Result<Self, ReplError> {
        if bytes.len() > MAX_STUDIO_EPOCH_SNAPSHOT_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        let adopting = match d.get_u8().map_err(|_| ReplError::Malformed)? {
            1 => false,
            2 => true,
            _ => return Err(ReplError::Malformed),
        };
        if d.get_bytes().map_err(|_| ReplError::Malformed)? != target.channel() {
            return Err(ReplError::EpochScope);
        }
        let opening = field(&mut d, MAX_RECEIPT_BYTES)?;
        let seed = field(&mut d, MAX_CHECKPOINT_BYTES)?;
        field(&mut d, MAX_RECEIPT_BOOK_BYTES)?;
        let gate = EpochGate::decode(field(&mut d, MAX_EPOCH_GATE_BYTES)?)?;
        let operations = d.get_u32().map_err(|_| ReplError::Malformed)? as usize;
        if operations > MAX_EPOCH_OPERATIONS {
            return Err(ReplError::EpochBound);
        }
        Ok(Self {
            adopting,
            opening,
            seed,
            gate,
            operations,
        })
    }
}

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
    /// [`Self::overlay_successor_hold`] over authenticated vault bytes, WITHOUT restoring the source.
    ///
    /// What a lifecycle read actually calls. Restoring a source decodes and replays up to the full
    /// operation log and builds a projection, under the actor's custody, which is exactly what the
    /// cheap lifecycle row must not cost. Everything the hold compares is in the record's header -
    /// the adopting flag, the opening receipt, the seed bytes, the gate's epoch and phase, and the
    /// operation count - so this reads those and nothing else; the operations are never decoded.
    ///
    /// Two differences from the full hold, both deliberate:
    ///
    /// - **No projection comparison.** With the seed bytes equal and no operation applied, the
    ///   source's projection is the seed's, so the comparison is implied. The handoff still makes it.
    /// - **No author check.** The record does not store the local actor; the caller compares the
    ///   branch's author with the requesting device before calling this.
    ///
    /// `owner` is the live designated committer, which is what a restore installs as the gate's
    /// owner. `studio::epoch::owner::tests::eligibility` holds this, the full hold and the
    /// handoff's own check together on every state its fixtures reach.
    pub fn overlay_successor_hold_in_vault(
        bytes: &[u8],
        target: StudioTarget,
        owner: Option<DeviceId>,
        overlay: &StudioOverlay,
    ) -> Result<Option<StudioOverlayManualReason>, ReplError> {
        use StudioOverlayManualReason as R;
        if overlay.target() != target {
            return Err(ReplError::EpochScope);
        }
        let shape = VaultShape::read(bytes, target)?;
        let phase = shape.gate.phase();
        if phase == EpochPhase::Fault {
            return Ok(Some(R::Fault));
        }
        let closed = overlay.receipt().closed_epoch;
        let successor = closed.checked_add(1).ok_or(ReplError::EpochBound)?;
        let epoch = shape.gate.epoch();
        if epoch < closed {
            return Ok(Some(R::SourceRewound));
        }
        if epoch == closed {
            return Ok(Some(if phase == EpochPhase::Closing {
                R::SuccessorMissing
            } else {
                R::SourceNotClosing
            }));
        }
        if epoch > successor {
            return Ok(Some(R::SourceReplaced));
        }
        let opening = (!shape.opening.is_empty())
            .then(|| Receipt::decode(shape.opening))
            .transpose()?;
        if owner
            != Some(DeviceId::from_public_key_bytes(
                &overlay.receipt().owner_public_key,
            ))
            || opening.as_ref() != Some(overlay.receipt())
        {
            return Ok(Some(R::ReceiptChanged));
        }
        if shape.seed != overlay.seed() {
            return Ok(Some(R::SourceReplaced));
        }
        if phase != EpochPhase::Open || shape.adopting || shape.operations != 0 {
            return Ok(Some(R::SuccessorNotPristine));
        }
        Ok(None)
    }

    /// Whether authenticated vault bytes hold any work: a later epoch or at least one operation.
    ///
    /// The structural form of the H1 and H5 Index check, `check_index_object_sources`, which treats
    /// a source at epoch 0 with no operations exactly like an absent one. Reads the header only.
    pub fn vault_holds_work(bytes: &[u8], target: StudioTarget) -> Result<bool, ReplError> {
        let shape = VaultShape::read(bytes, target)?;
        Ok(shape.gate.epoch() > 0 || shape.operations > 0)
    }

    /// Why this installed source cannot take `overlay` as its handoff successor, or `None` exactly
    /// when [`Self::check_overlay_successor`] would accept it (P2, design section 7).
    ///
    /// A classification over the same conditions as that check, never a second definition of
    /// them: every comparison below is one the check makes. It is the ORACLE for
    /// [`Self::overlay_successor_hold_in_vault`], which production calls; the eligibility tests
    /// hold all three together on every state their fixtures reach. The check stays authoritative;
    /// this only names its refusal.
    ///
    /// Ordered most-permanent first. A faulted, rewound or replaced source is reported as such even
    /// when the successor would also be unpristine, because "wait" is the wrong advice for it.
    ///
    /// Test-only: production classifies from the record header and never restores for it.
    #[cfg(test)]
    pub(in crate::studio) fn overlay_successor_hold(
        &mut self,
        overlay: &StudioOverlay,
        ledger: &IntentLedger,
    ) -> Result<Option<StudioOverlayManualReason>, ReplError> {
        use StudioOverlayManualReason as R;
        // Not a hold: a record for another target or document is a caller error, refused as the
        // check refuses it.
        if self.target != overlay.target() || self.document() != ledger.document() {
            return Err(ReplError::EpochScope);
        }
        if self.actor != overlay.author() {
            return Ok(Some(R::NotCurrentAuthor));
        }
        if self.phase() == EpochPhase::Fault {
            return Ok(Some(R::Fault));
        }
        let closed = overlay.receipt().closed_epoch;
        let successor = closed.checked_add(1).ok_or(ReplError::EpochBound)?;
        let epoch = self.epoch();
        if epoch < closed {
            return Ok(Some(R::SourceRewound));
        }
        if epoch == closed {
            return Ok(Some(if self.phase() == EpochPhase::Closing {
                R::SuccessorMissing
            } else {
                R::SourceNotClosing
            }));
        }
        if epoch > successor {
            return Ok(Some(R::SourceReplaced));
        }
        if self.gate.owner() != DeviceId::from_public_key_bytes(&overlay.receipt().owner_public_key)
            || self.opening.as_ref() != Some(overlay.receipt())
        {
            return Ok(Some(R::ReceiptChanged));
        }
        if self.doc.checkpoint_bytes()?.as_deref() != Some(overlay.seed()) {
            return Ok(Some(R::SourceReplaced));
        }
        if self.phase() != EpochPhase::Open
            || self.adopting
            || self.op_count() != 0
            || self.projection()? != overlay.base_projection()?
        {
            return Ok(Some(R::SuccessorNotPristine));
        }
        Ok(None)
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
