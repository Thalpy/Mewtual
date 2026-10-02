//! P2: the actionable state of a document's live draft (design sections 7 and 11).
//!
//! A read, never a decision. It names what the automatic handoff would do with this branch now
//! and, when that is "refuse", why, so a caller can show a user the manual path and the reason
//! instead of a bare branch and an error string later. The handoff re-derives everything under
//! custody when it actually runs; nothing here is consulted by it or mints anything for it.
use super::*;
use catcoms_replication::studio::{
    StudioOverlayEligibility, StudioOverlayManualReason as R, StudioOverlayProvenance,
};

impl ServerStore {
    /// `None` when the document has no live branch. Otherwise the most permanent applicable reason
    /// first: provenance, then authorship, then the installed source against the branch's basis
    /// (`StudioEpoch::overlay_successor_hold`, pinned to the handoff's own precondition), then the
    /// tenure the handoff would sign under, then whether the branch's receipt is still the current
    /// owner's under it.
    ///
    /// `tenure` is the AUTHORING value: `Some` only for a fully observed tenure. `Imported` and
    /// `Unknown` both read as `TenureUnknown`, which is the handoff's own refusal for them.
    ///
    /// A `Prepared` branch is classified like any other: its successor and authority are what its
    /// resolution needs too. Whether it is in flight is reported separately (`prepared`), and the
    /// two are deliberately not merged.
    pub(crate) fn studio_overlay_eligibility(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        tenure: Option<u64>,
    ) -> Result<Option<StudioOverlayEligibility>, AppError> {
        let manual = |reason| Ok(Some(StudioOverlayEligibility::Manual(reason)));
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let state = self.load_epoch_intents_structural(server, &document)?;
        let Some(metadata) = state.handoff_metadata() else {
            return Ok(None);
        };
        let Some(overlay) = metadata.overlay() else {
            return Ok(None);
        };
        if !matches!(metadata.provenance(), StudioOverlayProvenance::Closing) {
            return manual(R::Unconfirmed);
        }
        if overlay.author() != device.device_id() {
            return manual(R::NotCurrentAuthor);
        }
        // A source that fails to load or does not pair with this branch is a reason, not an error:
        // the lifecycle read must still report the branch, since export, archive and disposal do
        // not need the source and are exactly what a user in this state reaches for.
        let mut source = match self.load_studio_epoch(server, group, target, device) {
            Ok(Some(source)) => source,
            Ok(None) => return manual(R::SourceMissing),
            Err(_) => return manual(R::SourceUnreadable),
        };
        match source.unit.overlay_successor_hold(overlay, &state.ledger) {
            Ok(Some(reason)) => return manual(reason),
            Ok(None) => {}
            Err(_) => return manual(R::SourceUnreadable),
        }
        let Some(tenure) = tenure else {
            return manual(R::TenureUnknown);
        };
        // The handoff's live-authority mint, run for its verdict only and then dropped. It checks
        // that this device is still a member, and that the branch's receipt is the current
        // owner's under the observed tenure; the earlier checks have already covered authorship.
        // A Prepared branch has already minted one, and the mint refuses to mint twice, so its
        // in-flight resolution is not second-guessed here.
        if !state.handoff_prepared() && metadata.handoff_authority(device, group, tenure).is_err() {
            return manual(R::ReceiptChanged);
        }
        Ok(Some(StudioOverlayEligibility::Transferable))
    }
}
