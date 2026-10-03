//! Bounded seed inspection without an owner/tenure capability. No historical author claim in
//! this projection is authenticated by checking the receipt's self-declared signing key.
use super::{StudioProjection, StudioTarget};
use crate::{Receipt, ReplError};
use automerge::AutoCommit;
use std::collections::{BTreeMap, HashSet};
mod tail;
pub use tail::UnconfirmedStudioTailPreparation;
#[cfg(test)]
mod tests;

pub struct UnconfirmedStudioSeed {
    projection: StudioProjection,
    target: StudioTarget,
    doc: AutoCommit,
    doc_id: u128,
    applied: HashSet<[u8; 32]>,
    operations: BTreeMap<[u8; 32], crate::LocalIntent>,
    encoded_bytes: usize,
    /// The exact checkpoint bytes `parse_live_transfer` was given and proved canonical (design
    /// 8.1 part 1).
    ///
    /// Kept because nothing else can reproduce them once a tail is applied: the tail advances
    /// `projection`, so `projection.checkpoint(..)` stops matching the seed (review finding A2),
    /// and an unconfirmed draft must be based on the seed itself, not on the merged preview.
    /// Immutable after parsing; no tail operation touches it. Zeroizing for consistency with the
    /// raw transfer buffer it came from, **not** as a confidentiality property: the same content
    /// lives un-zeroized in `doc` and `projection`. Up to `MAX_CHECKPOINT_BYTES` (2 MiB), retained
    /// beside the parsed graph for as long as a ready preview is, which the design's memory
    /// accounting counts.
    seed_bytes: zeroize::Zeroizing<Vec<u8>>,
}
impl std::fmt::Debug for UnconfirmedStudioSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UnconfirmedStudioSeed { .. }")
    }
}
impl UnconfirmedStudioSeed {
    /// Cold, bounded parsing only. Network provenance and lifecycle must be checked separately
    /// before using the result. This value cannot be installed or used to authorize edits.
    ///
    /// Retains a copy of `bytes` once they are proved to be the canonical seed (see `seed_bytes`).
    /// A caller that only wants the graph uses [`Self::parse_graph`] and skips that copy.
    ///
    /// **For `catcoms-sync`'s live seed transfer only** (design 8.1 (ii)). A value of this type is
    /// what the Unconfirmed basis mint takes, so whoever can make one from bytes of their choosing
    /// could mint a preview base from an archive, an installed checkpoint or copied callback
    /// bytes. Hidden and named for its one use, and pinned with the mint by the repository gate:
    /// `clippy.toml`'s `disallowed-methods` and a source-scan test.
    #[doc(hidden)]
    pub fn parse_live_transfer(
        target: StudioTarget,
        receipt: &Receipt,
        bytes: &[u8],
    ) -> Result<Self, ReplError> {
        let (doc, projection, doc_id) = Self::parsed(target, receipt, bytes)?;
        Ok(Self {
            projection,
            target,
            doc,
            doc_id,
            applied: HashSet::new(),
            operations: BTreeMap::new(),
            encoded_bytes: 0,
            seed_bytes: zeroize::Zeroizing::new(bytes.to_vec()),
        })
    }

    /// The same checks as [`Self::parse_live_transfer`], returning only the private data graph, never a
    /// checkpoint or epoch capability. For local drafts and archives, which hold the seed bytes
    /// themselves and so must not pay for a second retained copy.
    pub(in crate::studio) fn parse_graph(
        target: StudioTarget,
        receipt: &Receipt,
        bytes: &[u8],
    ) -> Result<(AutoCommit, StudioProjection), ReplError> {
        let (doc, projection, _) = Self::parsed(target, receipt, bytes)?;
        Ok((doc, projection))
    }

    fn parsed(
        target: StudioTarget,
        receipt: &Receipt,
        bytes: &[u8],
    ) -> Result<(AutoCommit, StudioProjection, u128), ReplError> {
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
        let doc_id = crate::epoch_id(
            receipt.document.doc_type,
            &receipt.document.logical_key,
            epoch,
            &receipt.close_record_hash,
        );
        Ok((doc, projection, doc_id))
    }
    /// Typed content only; seed authors, receipt signer membership and tenure remain unconfirmed.
    pub fn projection(&self) -> &StudioProjection {
        &self.projection
    }
    pub fn doc_id(&self) -> u128 {
        self.doc_id
    }
    /// The target this seed was parsed for, which the Unconfirmed basis mint re-parses against.
    pub(in crate::studio) fn target(&self) -> StudioTarget {
        self.target
    }
    /// The exact seed checkpoint bytes `parse_live_transfer` accepted, unchanged by any tail since.
    ///
    /// Unconfirmed bytes, never authority, exactly like `projection`. **Custody is the holder's
    /// job:** `catcoms-sync` keeps its prepared seed's fields private and exposes these only
    /// through `with_provisional_studio_seed`, which re-checks the hint's current scope on every
    /// use (design 8.1 part 2). A consumer that persists them must re-parse them against their
    /// receipt first rather than trust this retention (part 3).
    pub fn seed_bytes(&self) -> &[u8] {
        &self.seed_bytes
    }
}
