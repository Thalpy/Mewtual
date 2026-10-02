//! P2: the actionable state of a document's live draft (design sections 7 and 11).
//!
//! A read, never a decision. It names what the automatic handoff would do with this branch now
//! and, when that is "refuse", why, so a caller can show a user the manual path and the reason
//! instead of a bare branch and an error string later. The handoff re-derives everything under
//! custody when it actually runs; nothing here is consulted by it or mints anything for it.
//!
//! **Cheap by construction.** The lifecycle row runs under the actor's custody on every read, so
//! nothing here restores a source: each source record is unsealed and its header read, and no
//! operation is decoded or replayed (`StudioEpoch::overlay_successor_hold_in_vault`,
//! `vault_holds_work`). The branch itself is read structurally, as the lifecycle already did.
use super::*;
use catcoms_replication::studio::{
    IndexOp, StudioEpoch, StudioOverlayEligibility, StudioOverlayManualReason as R,
    StudioOverlayProvenance,
};
use catcoms_replication::ReplError;

impl ServerStore {
    /// `None` when the document has no live branch. Otherwise the most permanent applicable reason
    /// first: provenance, authorship, the installed source against the branch's basis, the sources
    /// an Index branch's entries name, the tenure the handoff would sign under, and whether the
    /// branch's receipt is still the current owner's under it.
    ///
    /// Each refusal the automatic handoff makes from DURABLE state has a reason here, in the same
    /// terms: the successor precondition (`check_overlay_successor`), H1's Index object check
    /// (`check_index_object_sources`) and the live authority (`handoff_authority`). Transient
    /// refusals - capacity, a concurrent job, a changed record mid-flight - are not classified;
    /// they say "retry", not "act".
    ///
    /// `tenure` is the AUTHORING value: `Some` only for a fully observed tenure. `Imported` and
    /// `Unknown` both read as `TenureUnknown`, which is the handoff's own refusal for them.
    ///
    /// Errors only for a request this record does not answer (another target); a source that is
    /// missing, unreadable or for another branch is a REASON, so the lifecycle row still reports
    /// the branch a user needs to export, archive or dispose of.
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
        // A Flipnote's logical key omits its channel, so the same record answers for every
        // channel label of one object id. The handoff refuses the wrong one; so does this.
        metadata.check_target(target).map_err(invalid)?;
        let Some(overlay) = metadata.overlay() else {
            return Ok(None);
        };
        if !matches!(metadata.provenance(), StudioOverlayProvenance::Closing) {
            return manual(R::Unconfirmed);
        }
        if overlay.author() != device.device_id() {
            return manual(R::NotCurrentAuthor);
        }
        let owner = group.designated_committer();
        match self.with_vault_source(server, group, target, |bytes| {
            StudioEpoch::overlay_successor_hold_in_vault(bytes, target, owner, overlay)
        }) {
            Ok(None) => return manual(R::SourceMissing),
            Ok(Some(Some(reason))) => return manual(reason),
            Ok(Some(None)) => {}
            Err(_) => return manual(R::SourceUnreadable),
        }
        if let StudioTarget::Index { channel } = target {
            for (_, intent) in state.pending().filter(|(id, _)| state.is_overlay(id)) {
                let Ok(op) = IndexOp::decode_domain(&document, &intent.operation, &intent.author)
                else {
                    // The branch holds an entry it cannot decode, so it cannot be replayed into
                    // the successor either; the handoff refuses the same way.
                    return manual(R::NotReplayable);
                };
                if let IndexOp::PutObject { object, .. } = op {
                    let flipnote = StudioTarget::Flipnote { channel, object };
                    let holds_work = self.with_vault_source(server, group, flipnote, |bytes| {
                        StudioEpoch::vault_holds_work(bytes, flipnote)
                    });
                    if !matches!(holds_work, Ok(Some(true))) {
                        return manual(R::ObjectMissing);
                    }
                }
            }
        }
        let Some(tenure) = tenure else {
            return manual(R::TenureUnknown);
        };
        // The receipt's owner must still be the current one under the observed tenure: the live
        // half of `check_live`, asked directly so a Prepared branch is held to it as well. For an
        // active branch the authority mint adds membership and runs for its verdict only; a
        // Prepared branch has already minted one, and the mint refuses to mint twice.
        if !overlay.receipt_owner_is_current(group, tenure)
            || (!state.handoff_prepared()
                && metadata.handoff_authority(device, group, tenure).is_err())
        {
            return manual(R::ReceiptChanged);
        }
        Ok(Some(StudioOverlayEligibility::Transferable))
    }

    /// Run `read` over `target`'s authenticated vault source bytes, unsealed but not restored.
    /// `None` when the document has no source record.
    fn with_vault_source<T>(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        read: impl FnOnce(&[u8]) -> Result<T, ReplError>,
    ) -> Result<Option<T>, AppError> {
        let logical = target.document(&group.group_id()).map_err(invalid)?;
        let scope = scope_bytes(server, &logical)?;
        let Some(record) = self.read_studio_record(&scope)? else {
            return Ok(None);
        };
        let (stored, snapshot) = decode_record(&record.plain, &scope, &logical)?;
        if stored != target {
            return Err(invalid("wrong object channel"));
        }
        read(snapshot).map(Some).map_err(invalid)
    }
}
