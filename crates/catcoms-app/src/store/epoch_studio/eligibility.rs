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
use crate::studio::StudioOwnerTenure;
use catcoms_replication::studio::{
    IndexOp, StudioEpoch, StudioHandoffEvidence, StudioOverlayEligibility,
    StudioOverlayManualReason as R, StudioOverlayProvenance,
};
use catcoms_replication::ReplError;

impl ServerStore {
    /// `None` when the document has no live branch. Otherwise the most permanent applicable reason
    /// first: provenance, authorship, a Prepared branch's resolution evidence, then (unless that
    /// resolution settles the branch outright) the installed source against the branch's basis,
    /// the sources an Index branch's entries name, the tenure the handoff would sign under, and
    /// whether the branch's receipt is still the current owner's under it.
    ///
    /// Each refusal the automatic handoff makes from DURABLE state has a reason here, in the same
    /// terms: the successor precondition (`check_overlay_successor`), H1's Index object check
    /// (`check_index_object_sources`) and the live authority (`handoff_authority`). Transient
    /// refusals - capacity, a concurrent job, a changed record mid-flight - are not classified;
    /// they say "retry", not "act".
    ///
    /// `tenure` is this device's observed tenure, unconverted. Only `Known` can be transferable;
    /// `Imported` and `Unknown` are named apart (`TenureImported`, `TenureUnknown`) because the
    /// device holds different things, an unverifiable value or nothing. Both end at the same event,
    /// the next contiguous step that derives a tenure here; neither is cured by waiting alone.
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
        tenure: StudioOwnerTenure,
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
        // A Prepared branch is classified by what H1 does with it, which starts with the resolution:
        // from the Prepared record alone and with no tenure (V8), Complete evidence settles it,
        // Hold leaves it held permanently, and Absent returns it to active. The evidence is read
        // from the record's framing, without a restore, by the same comparisons `evidence` makes.
        //
        // - Complete stops here. The resolution settles the branch without a tenure, and the
        //   successor check below would wrongly call it unpristine: the operations the successor
        //   holds are this branch's own.
        // - Absent does NOT stop here. After the resolution, the branch is exactly an active branch
        //   against the same source, and H1 continues through tenure, the Index objects, the live
        //   authority and the successor precondition. So does this, by falling through. Stopping
        //   at `Transferable` here told the user a Faulted successor, or a device with no observed
        //   tenure, would transfer, and H1 then refused in the same call.
        //
        // `resolution` keeps the evidence for the authority check below, which refuses a Prepared
        // branch unless it is handed the Absent that returns it to active.
        let resolution = if state.handoff_prepared() {
            match self.with_vault_source(server, group, target, |bytes| {
                metadata.evidence_in_vault(bytes, &state.ledger)
            }) {
                Ok(None) => return manual(R::SourceMissing),
                Ok(Some(StudioHandoffEvidence::Hold)) => return manual(R::PreparedStuck),
                Ok(Some(StudioHandoffEvidence::Complete)) => {
                    return Ok(Some(StudioOverlayEligibility::Transferable))
                }
                Ok(Some(StudioHandoffEvidence::Absent)) => Some(StudioHandoffEvidence::Absent),
                Err(_) => return manual(R::SourceUnreadable),
            }
        } else {
            None
        };
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
            // One header read per DISTINCT object, at most `MAX_STUDIO_OVERLAY_OPS` of them: each
            // is an unseal of a bounded record, never a restore.
            let mut objects = std::collections::BTreeSet::new();
            for (_, intent) in state.pending().filter(|(id, _)| state.is_overlay(id)) {
                let Ok(op) = IndexOp::decode_domain(&document, &intent.operation, &intent.author)
                else {
                    // The branch holds an entry it cannot decode, so it cannot be replayed into
                    // the successor either; the handoff refuses the same way.
                    return manual(R::NotReplayable);
                };
                if let IndexOp::PutObject { object, .. } = op {
                    objects.insert(object);
                }
            }
            for object in objects {
                let flipnote = StudioTarget::Flipnote { channel, object };
                // An unreadable record is as unusable to the handoff as an absent one.
                if !matches!(
                    self.studio_object_holds_work(server, group, flipnote),
                    Ok(true)
                ) {
                    return manual(R::ObjectMissing);
                }
            }
        }
        // The handoff signs under a fully observed tenure only. The two refusals are named apart
        // for what the device holds, not for how they end (see `TenureUnknown`).
        let tenure = match tenure {
            StudioOwnerTenure::Known(start) => start,
            StudioOwnerTenure::Imported(_) => return manual(R::TenureImported),
            StudioOwnerTenure::Unknown => return manual(R::TenureUnknown),
        };
        // The handoff's live-authority check, for its verdict only: this device is still a member
        // and the branch's receipt is the current owner's under the observed tenure. Authorship was
        // checked above. Asked of the branch as H1 will hold it, so a Prepared branch with Absent
        // evidence is asked as the active branch its resolution returns it to; `handoff_authority`
        // itself refuses anything still marked Prepared.
        if metadata
            .check_handoff_authority_after_resolution(resolution, device, group, tenure)
            .is_err()
        {
            return manual(R::ReceiptChanged);
        }
        Ok(Some(StudioOverlayEligibility::Transferable))
    }

    /// Whether the Flipnote `object` names exists **in `object`'s channel** and holds work: an
    /// operation, or an epoch past zero. The same predicate the handoff's Index object check
    /// applies after a full load (`op_count() > 0 || epoch() > 0`), answered here from the record's
    /// header with no restore, so it is cheap enough to run under custody per object.
    ///
    /// `false` for no record. `false` too for a record stored under **another channel's label**:
    /// a Flipnote's logical key omits its channel, so one record answers for every label of that
    /// object id, and an Index entry in this channel naming another channel's object would name
    /// nothing here. Copy's probe reports that as a missing target rather than failing the whole
    /// preview (the review's wrong-object-channel Low), and P2 calls it `ObjectMissing`. An
    /// unreadable record is an error, for the caller to classify.
    pub(crate) fn studio_object_holds_work(
        &self,
        server: u64,
        group: &ServerGroup,
        object: StudioTarget,
    ) -> Result<bool, AppError> {
        let logical = object.document(&group.group_id()).map_err(invalid)?;
        let scope = scope_bytes(server, &logical)?;
        let Some(record) = self.read_studio_record(&scope)? else {
            return Ok(false);
        };
        let (stored, snapshot) = decode_record(&record.plain, &scope, &logical)?;
        if stored != object {
            return Ok(false);
        }
        StudioEpoch::vault_holds_work(snapshot, object).map_err(invalid)
    }

    /// Run `read` over `target`'s authenticated vault source bytes, unsealed but not restored.
    /// `None` when the document has no source record.
    pub(super) fn with_vault_source<T>(
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
