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
use crate::studio::StudioOwnerTenure;
use catcoms_crypto::DeviceId;
use catcoms_replication::studio::{
    StudioClosingOverlayBasis, StudioLocalDraft, StudioOverlayAdmission, StudioOverlayBasis,
    StudioOverlayProvenance, StudioOverlayState, StudioUnconfirmedOverlayBasis,
};
use catcoms_replication::{CloseRecord, LocalIntent};

/// How S1b and S3 obtain the fresh basis an authoring request is authorized against (G4-A1-S).
///
/// Flow S is one algorithm for both provenances (design 8.7: "Agent 1's Flow S unchanged" with the
/// mint substituted). Only the mint differs, and it is consumed at exactly two points, S1b and S3,
/// both of which are reached only by NEW authoring. Everything before them (classification,
/// exact retries, terminal acknowledgements, the pending check) never looks at this value, so
/// neither a missing tenure (V8) nor a lost preview can block a retry of durably accepted work.
pub(crate) enum StudioOverlayMint<'a> {
    /// The store mints from the installed Closing source under custody, exactly as before.
    /// Tenure is required only at the mint, S1b and S3 (V1); nothing earlier reads it (V8).
    Closing {
        close: &'a CloseRecord,
        tenure: StudioOwnerTenure,
    },
    /// The caller's attempt at the live-preview mint (`Server::mint_unconfirmed_overlay_basis`),
    /// made in this same custody visit.
    ///
    /// **A failed attempt is a value, not a precondition.** A preview can expire, be evicted or
    /// vanish on restart while its draft is durably accepted, and a retry of that draft must still
    /// be answered. So the caller always gets here, and the failure is surfaced only if the
    /// request turns out to be new authoring. No tenure is ever consulted on this path (8.5).
    ///
    /// Boxed because the basis carries its receipt and seed metadata inline, which dwarfs the
    /// Closing variant. Build it with [`StudioOverlayMint::unconfirmed`].
    ///
    /// Constructed only by tests until the preview Save (G4-A2-PREVIEW) wires it in production.
    #[cfg_attr(not(test), allow(dead_code))]
    Unconfirmed(Result<Box<StudioUnconfirmedOverlayBasis>, AppError>),
}

impl StudioOverlayMint<'_> {
    /// Wrap a live-preview mint attempt exactly as `Server::mint_unconfirmed_overlay_basis`
    /// returned it, failed or not.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn unconfirmed(attempt: Result<StudioUnconfirmedOverlayBasis, AppError>) -> Self {
        Self::Unconfirmed(attempt.map(Box::new))
    }
}

/// A fresh basis of either provenance, owned. The capture carries this - never the preview's seed
/// handle, which keeps the seed and a cache slot alive past eviction and whose expiry would bound
/// the whole Save (design 8.1's S3 hazard).
pub(crate) enum OwnedOverlayBasis {
    Closing(StudioClosingOverlayBasis),
    Unconfirmed(StudioUnconfirmedOverlayBasis),
}

impl OwnedOverlayBasis {
    /// The borrowed form every branch constructor and admission check takes; its variant is what
    /// the branch records as provenance, so a basis cannot be mislabelled on the way in.
    pub(crate) fn as_basis(&self) -> StudioOverlayBasis<'_> {
        match self {
            Self::Closing(basis) => basis.into(),
            Self::Unconfirmed(basis) => basis.into(),
        }
    }
}

impl From<StudioClosingOverlayBasis> for OwnedOverlayBasis {
    fn from(basis: StudioClosingOverlayBasis) -> Self {
        Self::Closing(basis)
    }
}

impl From<StudioUnconfirmedOverlayBasis> for OwnedOverlayBasis {
    fn from(basis: StudioUnconfirmedOverlayBasis) -> Self {
        Self::Unconfirmed(basis)
    }
}

/// The stale-basis refusal, worded for the fresh basis's kind. The Closing wording is exactly what
/// it was before Flow S was parameterized, so every existing caller and test reads the same text.
pub(in crate::store) fn basis_changed(fresh: &OwnedOverlayBasis) -> AppError {
    match fresh {
        OwnedOverlayBasis::Closing(_) => invalid("Closing overlay basis changed"),
        OwnedOverlayBasis::Unconfirmed(_) => {
            invalid("unconfirmed overlay basis changed; prepare again from the current preview")
        }
    }
}

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

/// Authenticated zeroizing plaintext, public context, the private basis and the job-owned media
/// hold. The basis grants permission to prepare local draft data only; the commit re-derives and
/// re-matches it from actual durable state (Closing) or a fresh live mint (Unconfirmed) before
/// anything is written.
pub(crate) struct StudioOverlayCapture {
    stamp: StudioOverlayStamp,
    intent_bytes: Option<Zeroizing<Vec<u8>>>,
    basis: OwnedOverlayBasis,
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
        state
            .ledger
            .prepare(self.intent.author, self.intent.operation.clone())
            .map_err(invalid)?;
        let draft = overlay
            .append(self.basis.as_basis(), &state.ledger, op_id, self.ts)
            .map_err(invalid)?;
        state.overlay = Some(overlay);
        Ok(StudioOverlayPlan {
            stamp: self.stamp,
            state,
            draft,
            media: self.media,
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
                let basis = self.basis.as_basis();
                if state
                    .admit_new_branch(self.stamp.target, self.branch, basis)
                    .map_err(invalid)?
                    != *admission
                {
                    return Err(invalid(
                        "the record no longer admits the branch this request named",
                    ));
                }
                // The provenance argument is the basis's own, so it agrees by construction; it
                // used to be a hard-coded `Closing` that nothing tied to the basis.
                state
                    .new_admitted(basis, *admission, basis.provenance())
                    .map_err(invalid)
            }
            (OverlayBranch::Admitted(admission), None) => {
                let basis = self.basis.as_basis();
                if StudioOverlayState::admit_first_branch(self.branch, basis) != *admission {
                    return Err(invalid(
                        "the record no longer admits the branch this request named",
                    ));
                }
                Ok(StudioOverlayState::new(basis))
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
        // `Into` so the many Closing-only tests keep passing the minted Closing basis as before.
        basis: impl Into<OwnedOverlayBasis>,
        branch: [u8; 32],
        joins: OverlayBranch,
        authoring: AdmittedOverlayAuthoring,
        ts: u64,
    ) -> Result<StudioOverlayCapture, AppError> {
        let basis = basis.into();
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

    /// The fresh basis for S1b or S3, by the mint's provenance. The ONE place either stage obtains
    /// it, so the two cannot drift apart.
    ///
    /// **Closing**, unchanged from before this was parameterized: require the tenure (V1), read
    /// the installed source under custody, and mint from it with the saved signed close.
    ///
    /// **Unconfirmed**, in this order:
    ///
    /// 1. No stored source. An Unconfirmed draft exists only where no installed source does (8.1,
    ///    8.5), and the live mint cannot see the store, so that is checked here, under custody, at
    ///    both stages. It is a presence probe, not a source read: any entry at the record's path
    ///    refuses, including one that is corrupt, unreadable or not a regular file, because each of
    ///    those is a stored source this device cannot vouch is absent. Reading and restoring it
    ///    only to refuse would put a full source reconstruction on the actor for nothing. Absence
    ///    must also agree with the budget, so a record unlinked while still accounted refuses.
    /// 2. The caller's mint attempt, opened only now: an installed source is the more fundamental
    ///    answer, and it is what a user needs to hear first.
    /// 3. The basis is for exactly this target. Admission compares identities only, and a
    ///    Flipnote's logical key omits its channel, so a basis minted for another channel would
    ///    otherwise open a branch recorded under the wrong one.
    /// 4. The mint is of this moment: it observed the current MLS epoch, and its provider is still
    ///    a member. The sanctioned mint rechecks both inside sync's hint callback, so this cannot
    ///    fail for a basis minted in this visit. It exists for one minted earlier and kept, which
    ///    would otherwise skip those rechecks at S1b and S3 alike.
    pub(in crate::store) fn mint_studio_overlay_basis(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        mint: StudioOverlayMint<'_>,
        storage: &mut EpochStorageBudget,
    ) -> Result<OwnedOverlayBasis, AppError> {
        match mint {
            StudioOverlayMint::Closing { close, tenure } => {
                let tenure = crate::studio::require_owner_tenure(tenure)?;
                let (mut source, observed, _) =
                    self.checked_studio_source(server, group, target, device, false, storage)?;
                if observed.is_none() {
                    return Err(invalid("Closing overlay source is missing"));
                }
                source
                    .prepare_closing_overlay(close, group, tenure)
                    .map(OwnedOverlayBasis::Closing)
                    .map_err(invalid)
            }
            StudioOverlayMint::Unconfirmed(minted) => {
                let logical = target.document(&group.group_id()).map_err(invalid)?;
                let scope = scope_bytes(server, &logical)?;
                // Every probe failure invalidates the budget, as `checked_studio_source` does for a
                // failed load: whatever this budget believed about the record is no longer known
                // to be true. A present record is not a failure, so it leaves the budget alone.
                let probed = (|| {
                    let parent = fs::symlink_metadata(self.dir.join("servers"))
                        .map_err(|e| AppError::Io(e.to_string()))?;
                    if !parent.is_dir() || is_link(&parent) {
                        return Err(invalid("parent is not a regular directory"));
                    }
                    match fs::symlink_metadata(self.studio_epoch_path(&scope)) {
                        Ok(_) => Ok(true),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                        Err(e) => Err(AppError::Io(e.to_string())),
                    }
                })();
                if probed.inspect_err(|_| storage.invalidate())? {
                    return Err(invalid(
                        "this document has an installed source; an unconfirmed draft cannot \
                         start or continue beside it",
                    ));
                }
                storage
                    .verify_record(
                        &StorageScope::new(server, &logical.server_id).map_err(invalid)?,
                        *blake3::hash(&scope).as_bytes(),
                        None,
                    )
                    .map_err(invalid)?;
                let basis = *minted?;
                if StudioOverlayBasis::from(&basis).target() != target {
                    return Err(invalid(catcoms_replication::ReplError::EpochScope));
                }
                let StudioOverlayProvenance::Unconfirmed {
                    provider,
                    observed_mls_epoch,
                    ..
                } = basis.provenance()
                else {
                    return Err(invalid(
                        "an unconfirmed basis must carry unconfirmed provenance",
                    ));
                };
                if observed_mls_epoch != group.epoch()
                    || group.member_signature_key(&provider).is_none()
                {
                    return Err(invalid(
                        "unconfirmed overlay basis was minted under another membership; prepare \
                         again from the current preview",
                    ));
                }
                Ok(OwnedOverlayBasis::Unconfirmed(basis))
            }
        }
    }

    /// Commit a planned Closing acceptance. A thin wrapper so every existing caller and test keeps
    /// its exact signature; the algorithm is [`Self::commit_studio_overlay_with_io`].
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
        self.commit_studio_overlay_with_io(
            server,
            group,
            target,
            device,
            StudioOverlayMint::Closing { close, tenure },
            plan,
            rng,
            budget,
            hooks,
        )
    }

    /// Commit a planned acceptance of either provenance. Re-mints the basis through
    /// [`Self::mint_studio_overlay_basis`] and requires the same fingerprint, so the accepted rule
    /// "recheck the same source and authority immediately before the first durable acceptance"
    /// survives the detach. Then installs the ordinary conservative reference holds (I-3) and
    /// performs one accounted intent write.
    ///
    /// For an Unconfirmed plan the caller supplies a mint attempt made in THIS commit visit, never
    /// one parked with the plan: the live check has to be re-run (8.7, "re-enters"). Provider,
    /// MLS epoch and time are admission facts and are not fingerprinted, so a later mint of the
    /// same seed and receipt still matches. A preview of a different candidate does not: its seed
    /// or receipt differs, and so does its fingerprint. Fingerprint domains are separate per
    /// provenance, so equality also proves the kind did not change.
    ///
    /// A plan is committed only with a mint of its own kind, checked before anything reads the
    /// store. Pairing them wrongly would fail closed anyway, but with text about something else
    /// ("Closing overlay basis changed", "source missing"); this names the actual mistake.
    ///
    /// The plan's media hold is released only when this call returns, on success, error and
    /// unwinding alike.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn commit_studio_overlay_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        mint: StudioOverlayMint<'_>,
        plan: StudioOverlayPlan,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<StudioLocalDraft, AppError> {
        match (&mint, plan.state.live_overlay_provenance()) {
            (StudioOverlayMint::Closing { .. }, Some(StudioOverlayProvenance::Closing))
            | (
                StudioOverlayMint::Unconfirmed(_),
                Some(StudioOverlayProvenance::Unconfirmed { .. }),
            ) => {}
            _ => {
                return Err(invalid(
                    "overlay plan and commit mint are for different kinds of draft",
                ))
            }
        }
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        let StudioOverlayPlan {
            stamp,
            state,
            draft,
            media,
        } = plan;
        let AdmittedOverlayMedia {
            origin,
            frame,
            hold,
        } = media;
        if stamp.server != server || stamp.target != target {
            return Err(invalid("overlay plan belongs to another target"));
        }
        // The plan's own copy of the binding capture already checked. A plan is a value the
        // receiver will hold across a detach and hand back here, so the commit does not take the
        // capture's word for which operation these media facts protect.
        if origin.target != target || origin.document != stamp.document {
            return Err(invalid(
                "admitted media does not belong to this authoring request",
            ));
        }
        if !self.studio_overlay_is_current(group, device, &stamp)? {
            return Err(invalid("overlay record or context changed; retry"));
        }
        let document = stamp.document.clone();
        // Re-derive the basis under the same custody as the write. A changed Closing source, a
        // replaced signed close, a changed owner or a tenure that is not Known all discard a
        // Closing plan; an installed source, a lost preview or a changed base discard an
        // Unconfirmed one. S3 is a V1 site for Closing: the requirement is applied inside the mint,
        // at the stage, so Imported and Unknown are refused with their own messages.
        let fresh = self.mint_studio_overlay_basis(
            server,
            group,
            target,
            device,
            mint,
            &mut budget.storage,
        )?;
        if fresh.as_basis().fingerprint() != draft.basis() {
            return Err(basis_changed(&fresh));
        }
        drop(fresh);
        // S3: the referenced pixels must still be physically present. The transient hold is a
        // liveness claim over an address, not proof the bytes survived the detached stage, so
        // this runs before the ordinary holds and before the intent barrier. A missing blob must
        // not become a newly accepted durable reference.
        if let Some((cid, bytes)) = &frame {
            self.check_studio_frame_pixels(&stamp.document.server_id, cid, *bytes)?;
        }
        // I-3, second half: the protection transfer. These run BEFORE the write attempt and while
        // the plan's job-owned hold is still alive, so dropping that hold below is safe whatever
        // the write does. A durable record alone does not repair a reference set that a complete
        // scan installed while this acceptance was in flight.
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
        // Explicit: the hold outlives the write attempt, including its error path.
        drop(hold);
        written?;
        Ok(draft)
    }
}
