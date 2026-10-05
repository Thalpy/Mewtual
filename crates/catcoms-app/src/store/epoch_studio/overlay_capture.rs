//! Staged local acceptance: cheap classification and capture under custody, the expensive typed
//! append on a detached worker, then a stamp-checked commit. The synchronous adapter composes
//! these three stages in order, so there is one algorithm rather than a batch path that can drift
//! from the scheduled one.
//!
//! Ordering is load bearing and is the AG1-001/I3-001 boundary: classification is cheap,
//! structural and reaches no blob; media admission and the job-owned reference hold belong to the
//! new-authoring path only, and are taken before custody is released, never before an accepted
//! request has been recognised.
use super::*;
use crate::store::creative_references::CreativeHold;
use crate::store::epoch_intents::{self, EpochIntentState};
use catcoms_crypto::DeviceId;
use catcoms_replication::studio::{
    StudioClosingOverlayBasis, StudioLocalDraft, StudioOverlayAdmission, StudioOverlayBasis,
    StudioOverlayProvenance, StudioOverlayState, StudioUnconfirmedOverlayBasis,
};
use catcoms_replication::LocalIntent;

pub(super) const MAX_UNCONFIRMED_OVERLAY_OPS: usize = 64;

/// Which branch an authorized new acceptance joins, as S1b decided it under custody.
///
/// Decided against the very record the capture authenticates, and carried rather than re-chosen:
/// the detached plan rechecks it against the bytes it decodes and refuses on any disagreement, but
/// it never picks a branch on its own. Before this existed the plan did exactly that, with
/// `unwrap_or_else(StudioOverlayState::new)` followed by an `append` that minted the next
/// generation whenever no branch was live - so the branch-generation namespace was optional, and
/// a request that skipped admission still got a branch.
pub(crate) enum OverlayBranch {
    /// `classify_request` answered `Active`: the request named the live branch and appends to it.
    Live,
    /// `Unmatched`, resolved at S1b to a new branch: by `admit_new_branch` when the document has an
    /// overlay record, or `admit_first_branch` when it has none. Only `New` ever reaches here;
    /// `Stale` is refused at S1b.
    Admitted(StudioOverlayAdmission),
}

/// Exactly what the media facts were derived from. A-001: without this, "minted by admission" only
/// says the facts are internally consistent, not that they belong to the operation that will
/// actually be appended.
#[derive(PartialEq, Eq)]
struct MediaOrigin {
    operation: [u8; 32],
    target: StudioTarget,
    document: LogicalDocument,
}

/// Verified media facts for one new acceptance, together with the hold that protects them and the
/// identity of the request they came from.
///
/// Only `admit_studio_overlay_authoring` can produce this, and it derives the frame reference from
/// the operation itself. A caller therefore cannot present verified frame facts without the hold
/// that protects them, nor a hold without the facts S3 needs. Before this type existed the staged
/// capture took the three independently and stored them, so a future caller could have disabled the
/// S3 recheck or N12(a)'s protection by passing `None`.
struct AdmittedOverlayMedia {
    origin: MediaOrigin,
    frame: Option<(catcoms_storage::Cid, u64)>,
    hold: Option<CreativeHold>,
}

/// One authorized new acceptance: the intent that will be appended, and the media facts and hold
/// minted from that same operation, on that same target and document.
///
/// A-001. Carrying the intent and the media as separate arguments let a caller pair media admitted
/// for operation A with an intent carrying operation B: the detached plan would append B while S3
/// rechecked and protected A's pixels. Nothing in the synchronous adapter did that, but the point
/// of the staged seam is that the receiver becomes a second caller. Both halves are now minted
/// together, the fields are private and there is no second constructor, so the mismatch cannot be
/// expressed; `capture_studio_overlay_save` rechecks the binding anyway, which is what the
/// `mismatched_for_test` fixture exercises.
pub(crate) struct AdmittedOverlayAuthoring {
    intent: LocalIntent,
    media: AdmittedOverlayMedia,
}

impl std::fmt::Debug for AdmittedOverlayAuthoring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AdmittedOverlayAuthoring { .. }")
    }
}

impl AdmittedOverlayAuthoring {
    /// Test-only: deliberately pair one request's intent with another request's admitted media, so
    /// the binding check at capture has something to refuse. Production has no way to build this.
    #[cfg(test)]
    pub(crate) fn mismatched_for_test(intent: LocalIntent, media_of: Self) -> Self {
        Self {
            intent,
            media: media_of.media,
        }
    }
}

/// The pixel reference a frame operation carries, with its declared size. Index operations and
/// header edits carry none. This only reads the request; possession is a separate question.
fn frame_pixels(
    logical: &LogicalDocument,
    operation: &DomainOp,
) -> Result<Option<(catcoms_storage::Cid, u64)>, AppError> {
    Ok(
        match catcoms_replication::studio::FlipnoteOp::decode_domain(logical, operation)
            .map_err(invalid)?
        {
            catcoms_replication::studio::FlipnoteOp::InsertFrame { cid, bytes, .. }
            | catcoms_replication::studio::FlipnoteOp::ReplaceFrame { cid, bytes, .. } => {
                Some((catcoms_storage::Cid::from_bytes(cid), bytes))
            }
            _ => None,
        },
    )
}

/// Record identity and public live context, captured under custody. It carries no plaintext, no
/// key, no store handle, no Server and no budget, so it is safe to hold across a detach.
pub(crate) struct StudioOverlayStamp {
    mount: Arc<()>,
    server: u64,
    document: LogicalDocument,
    target: StudioTarget,
    actor: DeviceId,
    actor_key: Vec<u8>,
    owner: DeviceId,
    mls: u64,
    /// Captured absence is explicit and must remain absence; it is not the same as unreadable.
    intent: Option<(blake3::Hash, u64)>,
}

impl std::fmt::Debug for StudioOverlayStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioOverlayStamp { .. }")
    }
}

/// The owned form of [`StudioOverlayBasis`]. Captures cross a detached worker boundary, so they
/// cannot retain the borrowed facade used by the replication crate. Keeping the two concrete
/// basis types as variants also makes provenance a property of the capability itself: callers
/// cannot pair an Unconfirmed base with a Closing label (or the reverse).
pub(crate) enum CapturedStudioOverlayBasis {
    Closing(StudioClosingOverlayBasis),
    Unconfirmed(StudioUnconfirmedOverlayBasis),
}

impl CapturedStudioOverlayBasis {
    pub(crate) fn borrowed(&self) -> StudioOverlayBasis<'_> {
        match self {
            Self::Closing(basis) => StudioOverlayBasis::Closing(basis),
            Self::Unconfirmed(basis) => StudioOverlayBasis::Unconfirmed(basis),
        }
    }

    pub(crate) fn fingerprint(&self) -> [u8; 32] {
        self.borrowed().fingerprint()
    }

    fn provenance(&self) -> StudioOverlayProvenance {
        self.borrowed().provenance()
    }
}

impl From<StudioClosingOverlayBasis> for CapturedStudioOverlayBasis {
    fn from(basis: StudioClosingOverlayBasis) -> Self {
        Self::Closing(basis)
    }
}

impl From<StudioUnconfirmedOverlayBasis> for CapturedStudioOverlayBasis {
    fn from(basis: StudioUnconfirmedOverlayBasis) -> Self {
        Self::Unconfirmed(basis)
    }
}

/// Authenticated zeroizing plaintext, public context, the private typed basis and the job-owned
/// media hold. The basis grants permission to prepare local draft data only; the commit re-derives
/// and re-matches it from actual durable state before anything is written.
pub(crate) struct StudioOverlayCapture {
    stamp: StudioOverlayStamp,
    intent_bytes: Option<Zeroizing<Vec<u8>>>,
    basis: CapturedStudioOverlayBasis,
    unconfirmed: bool,
    opens_unconfirmed_branch: bool,
    /// The branch the request named, and what S1b decided it joins.
    branch: [u8; 32],
    joins: OverlayBranch,
    intent: LocalIntent,
    ts: u64,
    /// Verified frame facts and the hold that protects them, minted together at S1b. Possession
    /// itself is never carried across the detach; only what to recheck is.
    media: AdmittedOverlayMedia,
}

impl std::fmt::Debug for StudioOverlayCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioOverlayCapture { .. }")
    }
}

/// A proposal, never a durable effect. It becomes durable only after a custody visit
/// reauthenticates the exact record it was derived from and re-mints the basis.
pub(crate) struct StudioOverlayPlan {
    stamp: StudioOverlayStamp,
    state: EpochIntentState,
    draft: StudioLocalDraft,
    media: AdmittedOverlayMedia,
    unconfirmed: bool,
    opens_unconfirmed_branch: bool,
}

/// A plan after the commit visit has reauthenticated its mount, target, member, owner, MLS epoch
/// and exact intent-record version. It remains non-authoritative until the provenance-specific S3
/// check re-mints the basis and, for Unconfirmed history, proves source absence.
struct CheckedStudioOverlayPlan {
    stamp: StudioOverlayStamp,
    state: EpochIntentState,
    draft: StudioLocalDraft,
    media: AdmittedOverlayMedia,
    unconfirmed: bool,
    opens_unconfirmed_branch: bool,
}

impl std::fmt::Debug for StudioOverlayPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioOverlayPlan { .. }")
    }
}

impl StudioOverlayCapture {
    /// Runs on a blocking worker, off actor and store custody. It owns authenticated plaintext,
    /// public context, the basis and the media hold; it owns no store, Server, device key, MLS
    /// secret or source writer, and it writes nothing.
    ///
    /// This is the expensive stage: `EpochIntentState::decode` reconstructs any retained branch
    /// and `append` replays it again with the new operation, which is the cost the runtime exists
    /// to move off custody.
    pub(crate) fn plan(self) -> Result<StudioOverlayPlan, AppError> {
        let scope = epoch_intents::scope_bytes(self.stamp.server, &self.stamp.document)?;
        let mut state = match &self.intent_bytes {
            Some(bytes) => EpochIntentState::decode(bytes, &scope, &self.stamp.document)?,
            None => EpochIntentState {
                ledger: catcoms_replication::IntentLedger::new(self.stamp.document.clone()),
                overlay: None,
            },
        };
        let op_id = self.intent.operation.id(&self.intent.author);
        if let Some(metadata) = &state.overlay {
            if metadata.target() != self.stamp.target {
                return Err(invalid("overlay belongs to another channel"));
            }
        }
        // Repeated from the classification stage: a worker must not rely on a decision taken
        // against bytes it cannot itself see.
        if state.pending().any(|(id, _)| *id == op_id) {
            return Err(invalid("ordinary intent cannot become an accepted overlay"));
        }
        let mut overlay = self.joined_branch(state.overlay.as_ref())?;
        if self.unconfirmed
            && overlay.overlay().map(|held| held.accepted()).unwrap_or(0)
                >= MAX_UNCONFIRMED_OVERLAY_OPS
        {
            return Err(invalid(
                "Unconfirmed draft operation limit reached (64 accepted operations)",
            ));
        }
        state
            .ledger
            .prepare(self.intent.author, self.intent.operation.clone())
            .map_err(invalid)?;
        let draft = overlay
            .append(self.basis.borrowed(), &state.ledger, op_id, self.ts)
            .map_err(invalid)?;
        state.overlay = Some(overlay);
        Ok(StudioOverlayPlan {
            stamp: self.stamp,
            state,
            draft,
            media: self.media,
            unconfirmed: self.unconfirmed,
            opens_unconfirmed_branch: self.opens_unconfirmed_branch,
        })
    }

    /// The branch state the new operation is appended to: the live one the request named, or the
    /// one S1b admitted. Never a branch chosen here.
    ///
    /// Each arm rechecks S1b's decision against `existing`, the record this worker decoded, rather
    /// than trusting it. The commit's stamp check will later prove those bytes are the ones S1b
    /// read, so a disagreement here means the capture itself is inconsistent, and it is refused.
    fn joined_branch(
        &self,
        existing: Option<&StudioOverlayState>,
    ) -> Result<StudioOverlayState, AppError> {
        match (&self.joins, existing) {
            (OverlayBranch::Live, Some(state)) if state.branch_id() == Some(self.branch) => {
                Ok(state.clone())
            }
            (OverlayBranch::Live, _) => Err(invalid(
                "the live branch this request named is not the one in the record",
            )),
            (OverlayBranch::Admitted(admission), Some(state)) => {
                if state
                    .admit_new_branch(self.stamp.target, self.branch, self.basis.borrowed())
                    .map_err(invalid)?
                    != *admission
                {
                    return Err(invalid(
                        "the record no longer admits the branch this request named",
                    ));
                }
                state
                    .new_admitted(self.basis.borrowed(), *admission, self.basis.provenance())
                    .map_err(invalid)
            }
            (OverlayBranch::Admitted(admission), None) => {
                if StudioOverlayState::admit_first_branch(self.branch, self.basis.borrowed())
                    != *admission
                {
                    return Err(invalid(
                        "the record no longer admits the branch this request named",
                    ));
                }
                Ok(StudioOverlayState::new(self.basis.borrowed()))
            }
        }
    }
}

impl ServerStore {
    /// S1b media admission, for new authoring that has already been authorized. Returns the intent
    /// and its media as one value, because they are only meaningful together (A-001).
    ///
    /// `operation_blob_cid` extracts an address and the typed layer checks declared sizes; neither
    /// establishes that the bytes exist. Validate and promote them into the durable namespace, then
    /// take the job-owned hold, and bind both to the operation, target and document they came from.
    ///
    /// Callers must reach this only after classification has ruled out an acknowledgement AND
    /// after the request's basis has been matched against a freshly derived one. An unmatched
    /// stale request must be refused before any blob is read or promoted and before the reference
    /// rails are consulted.
    pub(crate) fn admit_studio_overlay_authoring(
        &self,
        target: StudioTarget,
        document: &LogicalDocument,
        device: &MlsDevice,
        operation: DomainOp,
    ) -> Result<AdmittedOverlayAuthoring, AppError> {
        let intent = LocalIntent {
            author: device.device_id(),
            operation,
        };
        let origin = MediaOrigin {
            operation: intent.operation.id(&intent.author),
            target,
            document: document.clone(),
        };
        let media = self.admit_studio_overlay_media(origin, document, &intent.operation)?;
        Ok(AdmittedOverlayAuthoring { intent, media })
    }

    fn admit_studio_overlay_media(
        &self,
        origin: MediaOrigin,
        document: &LogicalDocument,
        operation: &DomainOp,
    ) -> Result<AdmittedOverlayMedia, AppError> {
        let frame = match origin.target {
            StudioTarget::Flipnote { .. } => frame_pixels(document, operation)?,
            StudioTarget::Index { .. } => None,
        };
        let Some((cid, bytes)) = frame else {
            return Ok(AdmittedOverlayMedia {
                origin,
                frame: None,
                hold: None,
            });
        };
        let mut blobs = self.blob_store(&hex::encode(&document.server_id))?;
        let pixels = blobs
            .get_bounded(&cid, bytes as usize)?
            .ok_or_else(|| invalid("publish the frame PIX before saving its reference"))?;
        if pixels.len() as u64 != bytes {
            return Err(invalid("frame byte declaration differs from PIX"));
        }
        crate::creative::validate_pix(&pixels)?;
        if pixels[4] != 191 || pixels[5] != 143 {
            return Err(invalid("Flipnote frames must be 192x144"));
        }
        blobs.put_staged(&pixels)?;
        if !blobs.promote_staged_bounded(&cid, pixels.len())? {
            return Err(invalid("frame promotion failed"));
        }
        let hold = self.hold_creative_transient(
            &document.server_id,
            std::collections::BTreeSet::from([*cid.as_bytes()]),
        )?;
        Ok(AdmittedOverlayMedia {
            origin,
            frame: Some((cid, bytes)),
            hold: Some(hold),
        })
    }

    /// S3 possession recheck, immediately before the first durable acceptance. The transient hold
    /// is a liveness claim over an address, not proof the bytes are still present: external
    /// deletion or storage damage during the detached stage must refuse here rather than produce a
    /// durable draft naming pixels the store does not possess. S1b already validated the content
    /// addressed by this CID, so only presence and exact size are rechecked.
    fn check_studio_frame_pixels(
        &self,
        group: &[u8],
        cid: &catcoms_storage::Cid,
        bytes: u64,
    ) -> Result<(), AppError> {
        let blobs = self.blob_store(&hex::encode(group))?;
        match blobs.get_bounded(cid, bytes as usize)? {
            Some(pixels) if pixels.len() as u64 == bytes => Ok(()),
            Some(_) => Err(invalid("frame byte declaration differs from PIX")),
            None => Err(invalid(
                "accepted frame PIX is no longer held; republish it",
            )),
        }
    }

    /// Capture under custody, after classification has ruled out an acknowledgement, the basis has
    /// been matched, the branch has been admitted, and authoring has been admitted.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture_studio_overlay_save(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        basis: impl Into<CapturedStudioOverlayBasis>,
        branch: [u8; 32],
        joins: OverlayBranch,
        authoring: AdmittedOverlayAuthoring,
        ts: u64,
    ) -> Result<StudioOverlayCapture, AppError> {
        current_member(group, device)?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let AdmittedOverlayAuthoring { intent, media } = authoring;
        // A-001. The combined value already makes a cross-operation pairing inexpressible; this
        // recheck is what makes the guarantee testable, and it also catches a capture whose target,
        // document or device differs from the one media was admitted against. The detached plan
        // appends `intent.operation`, so S3 and the job-owned hold must belong to that operation.
        if intent.author != device.device_id()
            || media.origin
                != (MediaOrigin {
                    operation: intent.operation.id(&intent.author),
                    target,
                    document: document.clone(),
                })
        {
            return Err(invalid(
                "admitted media does not belong to this authoring request",
            ));
        }
        let scope = epoch_intents::scope_bytes(server, &document)?;
        let basis = basis.into();
        let unconfirmed = matches!(basis, CapturedStudioOverlayBasis::Unconfirmed(_));
        let opens_unconfirmed_branch = unconfirmed && matches!(&joins, OverlayBranch::Admitted(_));
        // Authenticate the bounded record without decoding it: the detached stage owns the decode.
        let record = self.read_scoped_intent_plain(&scope)?;
        Ok(StudioOverlayCapture {
            stamp: StudioOverlayStamp {
                mount: self.registry_mount(),
                server,
                document,
                target,
                actor: device.device_id(),
                actor_key: device.public_key_bytes(),
                owner: group
                    .designated_committer()
                    .ok_or_else(|| invalid("no current owner"))?,
                mls: group.epoch(),
                intent: record
                    .as_ref()
                    .map(|r| (blake3::hash(&r.plain), r.physical_bytes)),
            },
            intent_bytes: record.map(|r| r.plain),
            basis,
            unconfirmed,
            opens_unconfirmed_branch,
            branch,
            joins,
            intent,
            ts,
            media,
        })
    }

    /// Reacquired custody. Compares mount identity, numeric server, complete target, document,
    /// actor and key, designated owner, MLS epoch, and the complete authenticated plaintext digest
    /// with its physical size. Captured absence must remain absence. It does not decode.
    pub(crate) fn studio_overlay_is_current(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        stamp: &StudioOverlayStamp,
    ) -> Result<bool, AppError> {
        if !Arc::ptr_eq(&stamp.mount, &self.registry_mount())
            || stamp.document.server_id != group.group_id()
            || stamp.actor != device.device_id()
            || stamp.actor_key != device.public_key_bytes()
            || Some(stamp.owner) != group.designated_committer()
            || stamp.mls != group.epoch()
        {
            return Ok(false);
        }
        let scope = epoch_intents::scope_bytes(stamp.server, &stamp.document)?;
        let record = self.read_scoped_intent_plain(&scope)?;
        let version = record
            .as_ref()
            .map(|r| (blake3::hash(&r.plain), r.physical_bytes));
        Ok(version == stamp.intent)
    }

    /// Reauthenticate a detached plan before any provenance-specific S3 work. This deliberately
    /// does not inspect a Closing source or an Unconfirmed preview: doing so belongs after the
    /// structural stamp check, and the caller must still re-mint the exact kind of basis it needs.
    fn check_studio_overlay_plan(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        plan: StudioOverlayPlan,
        budget: &mut EpochStudioBudget,
    ) -> Result<CheckedStudioOverlayPlan, AppError> {
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        let StudioOverlayPlan {
            stamp,
            state,
            draft,
            media,
            unconfirmed,
            opens_unconfirmed_branch,
        } = plan;
        if stamp.server != server || stamp.target != target {
            return Err(invalid("overlay plan belongs to another target"));
        }
        if media.origin.target != target || media.origin.document != stamp.document {
            return Err(invalid(
                "admitted media does not belong to this authoring request",
            ));
        }
        if !self.studio_overlay_is_current(group, device, &stamp)? {
            return Err(invalid("overlay record or context changed; retry"));
        }
        Ok(CheckedStudioOverlayPlan {
            stamp,
            state,
            draft,
            media,
            unconfirmed,
            opens_unconfirmed_branch,
        })
    }

    /// Finish the common S3 transaction after the caller has revalidated the typed basis. The
    /// transient media hold outlives the physical-presence check, reference transfer and intent
    /// write attempt on every success and error path.
    fn finish_studio_overlay_plan(
        &mut self,
        server: u64,
        checked: CheckedStudioOverlayPlan,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<StudioLocalDraft, AppError> {
        let CheckedStudioOverlayPlan {
            stamp,
            state,
            draft,
            media,
            unconfirmed,
            opens_unconfirmed_branch,
        } = checked;
        let AdmittedOverlayMedia {
            origin: _,
            frame,
            hold,
        } = media;
        let document = stamp.document.clone();
        let old_unconfirmed_charge = if unconfirmed && !opens_unconfirmed_branch {
            stamp.intent.map(|(_, size)| size)
        } else {
            None
        };
        let next_unconfirmed_charge = if unconfirmed {
            let scope = epoch_intents::scope_bytes(server, &document)?;
            let next = state
                .encode(&scope)?
                .len()
                .checked_add(40)
                .ok_or_else(|| invalid("Unconfirmed draft accounting overflow"))?
                as u64;
            budget.preflight_unconfirmed(opens_unconfirmed_branch, old_unconfirmed_charge, next)?;
            Some(next)
        } else {
            None
        };
        if let Some((cid, bytes)) = &frame {
            self.check_studio_frame_pixels(&stamp.document.server_id, cid, *bytes)?;
        }
        self.hold_creative(
            &document.server_id,
            state
                .overlay()
                .ok_or_else(|| invalid("overlay base missing"))?
                .base_blob_cids()
                .map_err(invalid),
        );
        for (id, intent) in state.pending() {
            if state.is_overlay(id) {
                self.hold_creative_operation(&document, &intent.operation);
            }
        }
        let old = stamp.intent.map(|(_, physical)| physical);
        let written = self.write_prepared_intents(
            server,
            &document,
            state,
            old,
            false,
            rng,
            &mut budget.storage,
            &mut budget.intents,
            WriteStep::new(WriteTag::Intents),
            hooks,
        );
        drop(hold);
        written?;
        if let Some(next) = next_unconfirmed_charge {
            budget.commit_unconfirmed(opens_unconfirmed_branch, old_unconfirmed_charge, next);
        }
        Ok(draft)
    }

    /// Commit a planned acceptance. Re-mints the Closing basis from actual durable state and
    /// requires the same fingerprint, so the accepted rule "recheck the same source and authority
    /// immediately before the first durable acceptance" survives the detach. Then installs the
    /// ordinary conservative reference holds (I-3) and performs one accounted intent write.
    ///
    /// The plan's media hold is released only when this call returns, on success, error and
    /// unwinding alike.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn commit_studio_overlay_save(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &catcoms_replication::CloseRecord,
        tenure: crate::studio::StudioOwnerTenure,
        plan: StudioOverlayPlan,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<StudioLocalDraft, AppError> {
        let checked =
            self.check_studio_overlay_plan(server, group, target, device, plan, budget)?;
        // Re-derive the basis under the same custody as the write. A changed Closing source, a
        // replaced signed close, a changed owner or a tenure that is not Known all discard the
        // plan. S3 is a V1 site: the requirement is applied here, at the stage, so Imported and
        // Unknown are refused with their own messages.
        let tenure = crate::studio::require_owner_tenure(tenure)?;
        let (mut source, observed, _) =
            self.checked_studio_source(server, group, target, device, false, &mut budget.storage)?;
        if observed.is_none() {
            return Err(invalid("Closing overlay source is missing"));
        }
        let fresh = source
            .prepare_closing_overlay(close, group, tenure)
            .map_err(invalid)?;
        if fresh.fingerprint() != checked.draft.basis() {
            return Err(invalid("Closing overlay basis changed"));
        }
        drop(source);
        self.finish_studio_overlay_plan(server, checked, rng, budget, hooks)
    }

    /// Commit an Unconfirmed plan. The app has just re-minted `fresh` from the live complete
    /// preview; this visit independently reauthenticates the plan and proves that no installed
    /// source exists before comparing the basis and performing the common intent transaction.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit_studio_unconfirmed_overlay_save(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        fresh: StudioUnconfirmedOverlayBasis,
        plan: StudioOverlayPlan,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioLocalDraft, AppError> {
        let checked =
            self.check_studio_overlay_plan(server, group, target, device, plan, budget)?;
        self.check_studio_source_absent(server, group, target, &mut budget.storage)?;
        if fresh.fingerprint() != checked.draft.basis() {
            return Err(invalid(
                "Unconfirmed overlay basis changed; refresh the preview",
            ));
        }
        self.finish_studio_overlay_plan(server, checked, rng, budget, &mut WriteHooks::None)
    }
}
