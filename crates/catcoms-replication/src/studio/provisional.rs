//! Bounded seed inspection without an owner/tenure capability. No historical author claim in
//! this projection is authenticated by checking the receipt's self-declared signing key.
use super::{StudioProjection, StudioTarget};
use crate::{Receipt, ReplError};

pub struct UnconfirmedStudioSeed {
    projection: StudioProjection,
}
impl std::fmt::Debug for UnconfirmedStudioSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UnconfirmedStudioSeed { .. }")
    }
}
impl UnconfirmedStudioSeed {
    /// Cold, bounded parsing only. Network provenance and lifecycle must be checked separately
    /// before using the result. This value cannot be installed or used to authorize edits.
    pub fn parse(target: StudioTarget, receipt: &Receipt, bytes: &[u8]) -> Result<Self, ReplError> {
        let doc = crate::checkpoint::inspect_unconfirmed_seed(receipt, bytes)?;
        let epoch = receipt
            .closed_epoch
            .checked_add(1)
            .ok_or(ReplError::EpochBound)?;
        let mut projection = target.read(&receipt.document, epoch, &doc)?;
        // Re-emit the canonical compact seed. Reject extra root data, noncompact history and
        // alternate encodings even when the advertised hash and self-signature match them.
        match &mut projection {
            StudioProjection::Index(p) => p.epoch = receipt.closed_epoch,
            StudioProjection::Flipnote(p) => p.epoch = receipt.closed_epoch,
        }
        if projection.checkpoint(receipt.close_record_hash)?.bytes() != bytes {
            return Err(ReplError::Malformed);
        }
        match &mut projection {
            StudioProjection::Index(p) => p.epoch = epoch,
            StudioProjection::Flipnote(p) => p.epoch = epoch,
        }
        Ok(Self { projection })
    }
    /// Typed content only; seed authors, receipt signer membership and tenure remain unconfirmed.
    pub fn projection(&self) -> &StudioProjection {
        &self.projection
    }
}
