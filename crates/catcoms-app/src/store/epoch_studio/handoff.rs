//! Explicit core/store transfer only. No background actor loop or network publication batch.
use super::super::epoch_intents::{self, EpochIntentState};
use super::*;
use catcoms_replication::studio::{
    StudioHandoffEvidence, StudioHandoffOutcome, StudioOverlayState,
};

/// What H1 concluded. `Settled` is terminal and already durable: an acknowledgement of a branch
/// this device already transferred. `Captured` is work whose expensive reconstruction has not
/// happened yet.
pub(crate) enum StudioHandoffStart {
    Settled(StudioHandoffOutcome),
    Captured(Box<StudioHandoffCapture>),
}

/// The rule `check_handoff_references` enforces, kept pure so it can be tested on its own (C-3
/// runtime 15.12, step A).
///
/// A handoff replaces the document's source with the candidate. Every blob the overlay's base
/// references must still be referenced afterwards, by the candidate itself or by an intent still
/// pending; otherwise reclamation could delete pixels the base needs. `base` is `None` for a
/// document with no overlay, which has no base to keep.
///
/// **No honest flow reaches a refusal.** H2's `check_overlay_successor` makes the successor's seed
/// the overlay's base, so the candidate always covers it. That is why this rule is unit-tested
/// directly and pinned by a CI mutation: a flow test could never tell a broken rule from a
/// working one.
pub(super) fn base_blobs_covered(
    base: Option<&std::collections::BTreeSet<catcoms_replication::studio::ContentId>>,
    candidate: &std::collections::BTreeSet<catcoms_replication::studio::ContentId>,
    pending: &std::collections::BTreeSet<catcoms_replication::studio::ContentId>,
) -> bool {
    base.is_none_or(|base| {
        base.iter()
            .all(|cid| candidate.contains(cid) || pending.contains(cid))
    })
}

/// Minted only here, after the Prepared record crosses its first durability barrier.
pub(super) struct CheckedHandoffWrite {
    metadata: [u8; 32],
    source: [u8; 32],
    before: blake3::Hash,
}

impl ServerStore {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handoff_studio_overlay(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        basis: [u8; 32],
        tenure: Option<u64>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioHandoffOutcome, AppError> {
        self.handoff_studio_overlay_with_io(
            server,
            group,
            target,
            device,
            basis,
            tenure,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    /// Index creation still requires the actual independently saved object source.
    ///
    /// H1's form, which full-loads each object. H5 runs the same property again, header-only, as
    /// [`Self::check_index_objects_at_commit`]. Both are needed: a scheduled H1 to H5 spans many
    /// background turns, and the stamp covers only the Index document's own intent and source
    /// records, so the referenced Flipnote's source can be evicted, retired or cleaned up in
    /// between without invalidating anything. Committing then would leave a durable Index entry
    /// pointing at a source that no longer exists.
    fn check_index_object_sources(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        document: &LogicalDocument,
        state: &EpochIntentState,
    ) -> Result<(), AppError> {
        let StudioTarget::Index { channel } = target else {
            return Ok(());
        };
        for (_, intent) in state.pending().filter(|(id, _)| state.is_overlay(id)) {
            if let IndexOp::PutObject { object, .. } =
                IndexOp::decode_domain(document, &intent.operation, &intent.author)
                    .map_err(invalid)?
            {
                let referenced = self.load_studio_epoch(
                    server,
                    group,
                    StudioTarget::Flipnote { channel, object },
                    device,
                )?;
                if referenced.is_none_or(|s| s.op_count() == 0 && s.epoch() == 0) {
                    return Err(invalid("overlay references an unavailable Flipnote"));
                }
            }
        }
        Ok(())
    }

    /// H5's form of the Index object check, without a restore (design 9.1.1, amendment A1).
    ///
    /// For each Flipnote the branch's PutObjects name, once each, it requires what H1's full load
    /// established: a record in this channel that holds work (the header-only rule
    /// `studio_object_holds_work` applies), and a valid intent link. Both are checked here on one
    /// authenticated read, rather than by calling that helper and reading again for the link. The
    /// helper's other callers (copy, P2) take no link check and must not change.
    ///
    /// **What it does not prove:** that the body still restores. An authenticated record whose
    /// header and body disagree, or one an older build wrote that a newer restore refuses, passes
    /// here. H1's full load catches both. Between H1 and H5 a record can only change through a
    /// writer in this build, which writes bodies its own restore accepts.
    ///
    /// **Cost:** one authenticated read per distinct object, bounded by the family's sealed cap,
    /// plus a structural read of its intent record when linked.
    fn check_index_objects_at_commit(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        document: &LogicalDocument,
        state: &EpochIntentState,
    ) -> Result<(), AppError> {
        let StudioTarget::Index { channel } = target else {
            return Ok(());
        };
        let mut objects = std::collections::BTreeSet::new();
        for (_, intent) in state.pending().filter(|(id, _)| state.is_overlay(id)) {
            if let IndexOp::PutObject { object, .. } =
                IndexOp::decode_domain(document, &intent.operation, &intent.author)
                    .map_err(invalid)?
            {
                objects.insert(object);
            }
        }
        let unavailable = || invalid("overlay references an unavailable Flipnote");
        for object in objects {
            let flipnote = StudioTarget::Flipnote { channel, object };
            let logical = flipnote.document(&group.group_id()).map_err(invalid)?;
            let scope = scope_bytes(server, &logical)?;
            let record = self.read_studio_record(&scope)?.ok_or_else(unavailable)?;
            // A record stored under another channel's label answers for this object id but
            // names nothing in this channel, exactly as `studio_object_holds_work` treats it.
            let (stored, snapshot) = decode_record(&record.plain, &scope, &logical)?;
            let holds_work = stored == flipnote
                && StudioEpoch::vault_holds_work(snapshot, flipnote).map_err(invalid)?;
            if !holds_work {
                return Err(unavailable());
            }
            self.check_studio_intent_link(server, &logical, &scope, &record.plain)?;
        }
        Ok(())
    }

    /// Ordinary durable IO for the scheduled runtime's H1 visit.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_studio_handoff(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        basis: [u8; 32],
        tenure: Option<u64>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioHandoffStart, AppError> {
        self.start_studio_handoff_with_io(
            server,
            group,
            target,
            device,
            basis,
            tenure,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    /// Ordinary durable IO for the scheduled runtime's H5 visit.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit_studio_handoff(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        commit: StudioHandoffCommit,
        tenure: Option<u64>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioHandoffOutcome, AppError> {
        self.commit_studio_handoff_with_io(
            server,
            group,
            target,
            device,
            commit,
            tenure,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    /// The synchronous adapter: H1, H2, then H3 to H5, with no detach between them. Every caller
    /// that cannot release custody, and every existing test, takes this path. The scheduled
    /// runtime runs the same stages with custody released around H2.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn handoff_studio_overlay_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        basis: [u8; 32],
        tenure: Option<u64>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<StudioHandoffOutcome, AppError> {
        let capture = match self.start_studio_handoff_with_io(
            server, group, target, device, basis, tenure, rng, budget, hooks,
        )? {
            StudioHandoffStart::Settled(outcome) => return Ok(outcome),
            StudioHandoffStart::Captured(capture) => capture,
        };
        // H1 already required a live tenure to mint the authority, so this cannot be absent here.
        let tenure =
            tenure.ok_or_else(|| invalid("overlay handoff needs observed owner tenure"))?;
        // H2, then H3 with no turn cap, no deadline and nothing to yield to, then H4. The
        // scheduled runtime runs these same four with custody released around H2 and H4 and the
        // signing paged across visits.
        let mut plan = capture.prepare()?;
        let slice = plan.sign_slice(device, group, tenure, false, usize::MAX, None)?;
        if !slice.complete() {
            return Err(invalid("handoff signing did not complete"));
        }
        let commit = plan.assemble()?;
        self.commit_studio_handoff_with_io(
            server,
            group,
            target,
            device,
            commit,
            Some(tenure),
            rng,
            budget,
            hooks,
        )
    }

    /// H1: classify, resolve an interrupted Prepared record, authorize, and capture.
    ///
    /// Everything here is cheap by construction: bounded authenticated reads, a structural decode
    /// for classification, and the short live-authority mint. The full branch reconstruction and
    /// the private successor restore belong to H2, which runs detached.
    ///
    /// Both the synchronous adapter and the scheduled runtime enter through this, so there is one
    /// classification and one authorization, not two that can drift.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_studio_handoff_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        basis: [u8; 32],
        tenure: Option<u64>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<StudioHandoffStart, AppError> {
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let mut state = self.checked_epoch_replay_state(
            server,
            &document,
            &mut budget.storage,
            &mut budget.intents,
        )?;
        let metadata = state
            .handoff_metadata()
            .ok_or_else(|| invalid("overlay is missing"))?;
        // Complete target comparison precedes source lookup and sync reservation, even after
        // ordinary ledger retirement. A Flipnote's logical key alone omits its channel.
        if let Some(outcome) = metadata
            .completed_branch(target, device.device_id(), basis)
            .map_err(invalid)?
        {
            self.persist_handoff_intents(
                server,
                &document,
                state,
                true,
                WriteTag::Completed,
                rng,
                budget,
                hooks,
            )?;
            return Ok(StudioHandoffStart::Settled(outcome));
        }
        let overlay = metadata
            .overlay()
            .ok_or_else(|| invalid("saved overlay basis is stale or unknown"))?;
        if overlay.author() != device.device_id() || overlay.basis() != basis {
            return Err(invalid("overlay author or basis mismatch"));
        }
        if state.handoff_prepared() {
            self.resolve_studio_handoff_with_io(
                server, group, target, device, None, rng, budget, hooks,
            )?;
            state = self.checked_epoch_replay_state(
                server,
                &document,
                &mut budget.storage,
                &mut budget.intents,
            )?;
            if let Some(outcome) = state
                .handoff_metadata()
                .ok_or_else(|| invalid("overlay metadata missing"))?
                .completed_branch(target, device.device_id(), basis)
                .map_err(invalid)?
            {
                return Ok(StudioHandoffStart::Settled(outcome));
            }
        }
        let tenure =
            tenure.ok_or_else(|| invalid("overlay handoff needs observed owner tenure"))?;
        // The short live-authority mint. Structural metadata is enough: this reads the target,
        // the active branch's author and its receipt, and checks them against live membership,
        // MLS epoch and the observed tenure. It reconstructs nothing.
        let authority = state
            .handoff_metadata()
            .ok_or_else(|| invalid("overlay metadata missing"))?
            .handoff_authority(device, group, tenure)
            .map_err(invalid)?;
        // The pristine-successor probe (design 14, the guard M6 removes), from the record header
        // alone: a bounded authenticated read, never a restore. H2's `check_overlay_successor`
        // stays authoritative. This refuses, before anything is captured or scheduled, a successor
        // that check would refuse, so a missing, Faulted, replaced or already-edited successor
        // costs a header read rather than a detached reconstruction every probe period. It is the
        // classification the eligibility view reports, and the eligibility tests hold it to the
        // check on every state their fixtures reach. The design 18.3 review (F8) found it unbuilt.
        //
        // Placed after the authority mint, so the live owner's receipt check keeps its own refusal
        // (its tests isolate it), and **before the Index check**, whose restores of every
        // referenced object are the expensive part of H1; a non-pristine Index successor must not
        // pay them every probe period (implementation review of the 18.3 fixes, LOW-1). On the
        // eligible path the source is authenticated twice, here and by capture: a bounded read.
        let overlay = state
            .handoff_metadata()
            .and_then(|metadata| metadata.overlay())
            .ok_or_else(|| invalid("saved overlay basis is stale or unknown"))?;
        let owner = group.designated_committer();
        match self.with_vault_source(server, group, target, |bytes| {
            StudioEpoch::overlay_successor_hold_in_vault(bytes, target, owner, overlay)
        })? {
            None => return Err(invalid("overlay handoff successor source missing")),
            Some(Some(reason)) => {
                return Err(invalid(format!(
                    "overlay handoff successor is not transferable: {reason:?}"
                )))
            }
            Some(None) => {}
        }
        // Index creation still requires the actual independently saved object source.
        self.check_index_object_sources(server, group, target, device, &document, &state)?;
        self.capture_studio_handoff(
            server, group, target, device, &document, basis, tenure, authority,
        )
        .map(Box::new)
        .map(StudioHandoffStart::Captured)
    }

    /// H3, H4 and H5 composed under one custody visit: sign every remaining operation, assemble
    /// the candidate, then run the accepted Prepared -> whole Source -> Completed transaction.
    ///
    /// The scheduled runtime will split H3 into bounded slices and move H4 to a worker; this is
    /// the batch form the synchronous transaction keeps, and the durable half below is unchanged.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn commit_studio_handoff_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        commit: StudioHandoffCommit,
        tenure: Option<u64>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<StudioHandoffOutcome, AppError> {
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        // Tenure is part of the stamp, so a restart for the same owner at the same MLS epoch
        // refuses here rather than letting a batch signed under one tenure become durable under
        // the next.
        if !self.studio_handoff_is_current(group, device, tenure, &commit.stamp)? {
            return Err(invalid("overlay records or context changed; retry"));
        }
        if commit.stamp.server != server || commit.stamp.target != target {
            return Err(invalid("overlay plan belongs to another target"));
        }
        let StudioHandoffCommit {
            stamp,
            basis,
            candidate,
            prepared,
            mut state,
            snapshot,
            prepared_bytes,
            completed_bytes,
            source_bytes,
            facts,
        } = commit;
        let document = stamp.document.clone();
        // No restore (design 9.1): the stamp check above re-read the source and proved it the
        // bytes H2 restored, so H2's facts describe it. `observed` is the record this write
        // replaces, and the digest is the capability's proof of the bytes it may replace.
        let (observed, original_source_hash) =
            self.stamped_studio_source(&stamp, &facts, &mut budget.storage)?;
        // Against the state as H2 read it, before the prepared overlay is installed, exactly as
        // when H4 and H5 were one function.
        self.check_handoff_references(&prepared, &candidate, &state)?;
        self.check_index_objects_at_commit(server, group, target, &document, &state)?;
        let scope = epoch_intents::scope_bytes(server, &document)?;
        let source_scope = scope_bytes(server, &document)?;
        // Physical size and the authenticated plaintext digest are all the unchanged fence needs;
        // decoding again would reconstruct the whole branch a second time.
        let original = self.read_scoped_intent_plain(&scope)?;
        let old = original.as_ref().map(|record| record.physical_bytes);
        let original = original.map(|record| blake3::hash(&record.plain));
        // Design 9.3 step 6 compares with the CAPTURED values (C-2), not only with this visit's
        // own reads. The re-read before the first write, below, is under the same exclusive borrow
        // as this one, so comparing those two with each other cannot fail. Without this, the
        // digest comparison inside `studio_handoff_is_current` above was the only thing standing
        // between a same-size change to the intent record and Prepared overwriting it with H2's
        // ledger, losing whatever ordinary intent the change added (implementation review of the
        // 18.3 fixes, M-1). With it, that comparison is redundant, as design mutation M1 says.
        if !stamp.captured_intent(original, old) {
            return Err(invalid("overlay intent source changed"));
        }
        state.overlay = Some(prepared);
        let intent_id = *blake3::hash(&scope).as_bytes();
        budget.intents.preflight_handoff(
            &self.intent_generation,
            intent_id,
            old,
            prepared_bytes,
            completed_bytes,
        )?;
        budget
            .storage
            .preflight_sequence(
                &budget.scope,
                &[
                    Replacement {
                        record: epoch_intents::storage_record(
                            server,
                            &document,
                            &scope,
                            prepared_bytes,
                        )?,
                        scratch_bytes: 0,
                        purpose: WritePurpose::Ordinary,
                    },
                    Replacement {
                        record: storage_record(
                            server,
                            &document,
                            &source_scope,
                            source_bytes,
                            candidate.storage_protocol_bytes().map_err(invalid)?,
                        )?,
                        scratch_bytes: 0,
                        purpose: WritePurpose::Ordinary,
                    },
                    Replacement {
                        record: epoch_intents::storage_record(
                            server,
                            &document,
                            &scope,
                            completed_bytes,
                        )?,
                        scratch_bytes: 0,
                        purpose: WritePurpose::Ordinary,
                    },
                ],
            )
            .map_err(invalid)?;
        // Recheck the actual intent contents before the first write. The source was checked
        // under the same exclusive borrow; the common writer also authenticates its old file.
        // Comparing the complete authenticated plaintext digest and physical size covers the same
        // canonical bytes the previous decode-then-re-encode compared, without a second decode.
        let actual = self.read_scoped_intent_plain(&scope)?;
        if actual.as_ref().map(|record| record.physical_bytes) != old
            || actual.map(|record| blake3::hash(&record.plain)) != original
        {
            return Err(invalid("overlay intent source changed"));
        }
        let metadata_hash = *blake3::hash(&state.encode(&scope)?).as_bytes();
        self.persist_handoff_intents(
            server,
            &document,
            state,
            false,
            WriteTag::Prepared,
            rng,
            budget,
            hooks,
        )?;
        let capability = CheckedHandoffWrite {
            metadata: metadata_hash,
            source: *blake3::hash(&snapshot).as_bytes(),
            before: original_source_hash,
        };
        // Only this private capability can accompany the checked complete candidate.
        //
        // `before` is empty, so the writer always replaces: the candidate carries the newly
        // signed operations and is never the bytes it replaces, so its flush branch is never
        // the right one here (review of 9.1.1, which dropped the before-snapshot fact).
        let written = self.save_studio_source_checked(
            server,
            candidate,
            Some(observed),
            &[],
            WritePurpose::Ordinary,
            rng,
            &mut budget.storage,
            WriteStep::new(WriteTag::Source),
            hooks,
            None,
            Some(&capability),
        )?;
        // After the write, prove that what landed is the candidate (design 9.1). On any failure,
        // including a re-read that does not authenticate, the commit refuses here with Prepared
        // retained. The next H1, or a fence, resolves from the actual bytes (A4). The budget
        // reserved a footprint for bytes that may not be what landed, so it is spent.
        let verified =
            match self.verify_persisted_studio_source(server, target, written, capability.source) {
                Ok(verified) => verified,
                Err(error) => {
                    budget.storage.invalidate();
                    return Err(invalid(format!(
                    "persisted handoff source could not be proved to be the written candidate; \
                     Prepared retained: \
                     {error}"
                )));
                }
            };
        self.resolve_studio_handoff_with_io(
            server,
            group,
            target,
            device,
            Some(verified),
            rng,
            budget,
            hooks,
        )?;
        let final_state = self.checked_epoch_replay_state(
            server,
            &document,
            &mut budget.storage,
            &mut budget.intents,
        )?;
        final_state
            .handoff_metadata()
            .ok_or_else(|| invalid("completed handoff missing"))?
            .completed_branch(target, device.device_id(), basis)
            .map_err(invalid)?
            .ok_or_else(|| invalid("handoff remains held"))
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_handoff_intents(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        state: EpochIntentState,
        unchanged: bool,
        step: WriteTag,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(), AppError> {
        // `write_prepared_intents` consumes this size; it performs no old-record read of its own.
        let old = self
            .read_scoped_intent_plain(&epoch_intents::scope_bytes(server, document)?)?
            .map(|record| record.physical_bytes);
        self.write_prepared_intents(
            server,
            document,
            state,
            old,
            unchanged,
            rng,
            &mut budget.storage,
            &mut budget.intents,
            WriteStep::new(step),
            hooks,
        )?;
        Ok(())
    }

    /// A coordinator resolves before any journal, recovery or retirement side effect.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_studio_handoff(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<(), AppError> {
        self.resolve_studio_handoff_with_io(
            server,
            group,
            target,
            device,
            None,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }
    /// One resolution algorithm for every Prepared record (design 9.1, "One algorithm").
    ///
    /// `verified` is `Some` only from the H5 commit that has just written and proved the
    /// destination source. It supplies that candidate in place of a restore. Every other caller
    /// passes `None` and restores from disk: H1's interrupted-Prepared resolution, the fences
    /// (adoption, rotation, repair) and restart. The decision table, barriers, generations and
    /// writes are the same either way, with one difference: a verified source that shows anything
    /// but Complete evidence refuses without writing. The commit just wrote a candidate proved to
    /// carry the whole branch, so anything else means the proof and the record disagree, and
    /// `return_to_active` must never be computed from a candidate.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_studio_handoff_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        verified: Option<source::VerifiedPersistedSource>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(), AppError> {
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let mut state = self.checked_epoch_replay_state(
            server,
            &document,
            &mut budget.storage,
            &mut budget.intents,
        )?;
        let Some(metadata) = state.handoff_metadata() else {
            return Ok(());
        };
        metadata.check_target(target).map_err(invalid)?;
        if !metadata.is_prepared() {
            if metadata.has_completed() {
                self.persist_handoff_intents(
                    server,
                    &document,
                    state,
                    true,
                    WriteTag::Completed,
                    rng,
                    budget,
                    hooks,
                )?;
            }
            return Ok(());
        }
        let from_candidate = verified.is_some();
        let (source, observed, before) = match verified {
            Some(verified) => {
                verified.into_checked(self, server, group, target, &mut budget.storage)?
            }
            None => self.checked_studio_source(
                server,
                group,
                target,
                device,
                false,
                &mut budget.storage,
            )?,
        };
        if observed.is_none() {
            return Err(invalid("prepared handoff source missing"));
        }
        let evidence = metadata.evidence(&source, &state.ledger).map_err(invalid)?;
        if from_candidate && evidence != StudioHandoffEvidence::Complete {
            return Err(invalid(
                "verified handoff source does not carry the whole branch; Prepared retained",
            ));
        }
        match evidence {
            StudioHandoffEvidence::Absent => {
                state.overlay = Some(
                    metadata
                        .return_to_active(&source, &state.ledger)
                        .map_err(invalid)?,
                );
                self.persist_handoff_intents(
                    server,
                    &document,
                    state,
                    false,
                    WriteTag::Active,
                    rng,
                    budget,
                    hooks,
                )
            }
            StudioHandoffEvidence::Complete => {
                // Authenticate actual source again and flush without another signed write.
                let source = self.save_studio_source(
                    server,
                    source,
                    observed,
                    &before,
                    WritePurpose::Ordinary,
                    rng,
                    &mut budget.storage,
                    WriteStep::flush_only(
                        WriteTag::Source,
                        "handoff resolution requires unchanged source",
                    ),
                    hooks,
                )?;
                self.check_handoff_references(metadata, &source.unit, &state)?;
                state.overlay = Some(
                    metadata
                        .complete(&source.unit, &state.ledger)
                        .map_err(invalid)?,
                );
                self.persist_handoff_intents(
                    server,
                    &document,
                    state,
                    false,
                    WriteTag::Completed,
                    rng,
                    budget,
                    hooks,
                )
            }
            StudioHandoffEvidence::Hold => Err(invalid(
                "prepared handoff retains conflicting or incomplete signed evidence",
            )),
        }
    }
    fn check_handoff_references(
        &self,
        metadata: &StudioOverlayState,
        source: &StudioEpoch,
        state: &EpochIntentState,
    ) -> Result<(), AppError> {
        let candidate = source.blob_cids().map_err(invalid)?;
        let mut pending = std::collections::BTreeSet::new();
        for (_, intent) in state.pending() {
            pending.extend(
                catcoms_replication::studio::operation_blob_cid(&intent.operation)
                    .map_err(invalid)?,
            );
        }
        let base = metadata
            .overlay()
            .map(|overlay| overlay.base_blob_cids())
            .transpose()
            .map_err(invalid)?;
        if !base_blobs_covered(base.as_ref(), &candidate, &pending) {
            return Err(invalid("handoff would release a base blob reference"));
        }
        Ok(())
    }

    /// Shared final write boundary. An ordinary caller cannot prune Prepared evidence.
    pub(super) fn check_studio_handoff_write(
        &mut self,
        server: u64,
        unit: &mut StudioEpoch,
        budget: &mut EpochStorageBudget,
        capability: Option<&CheckedHandoffWrite>,
    ) -> Result<bool, AppError> {
        let document = unit.document().clone();
        let scope = epoch_intents::scope_bytes(server, &document)?;
        let (state, old) = self
            .read_epoch_intent_record_structural(&scope, &document)
            .inspect_err(|_| budget.invalidate())?;
        let observed = old
            .map(|n| epoch_intents::storage_record(server, &document, &scope, n))
            .transpose()?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        budget
            .verify_record(&storage_scope, *blake3::hash(&scope).as_bytes(), observed)
            .map_err(invalid)?;
        let source_scope = scope_bytes(server, &document)?;
        let actual = self.read_studio_record(&source_scope)?;
        let source_parts = actual
            .as_ref()
            .map(|a| decode_record_link(&a.plain, &source_scope, &document))
            .transpose()?;
        let intent_link = source_parts.as_ref().is_some_and(|(_, _, linked)| *linked);
        let Some(metadata) = state.handoff_metadata() else {
            return if capability.is_none() && !intent_link {
                Ok(false)
            } else {
                Err(invalid("required handoff metadata missing"))
            };
        };
        metadata.check_target(unit.target()).map_err(invalid)?;
        if metadata.is_prepared() {
            let (target, snapshot, _) =
                source_parts.ok_or_else(|| invalid("prepared handoff source missing"))?;
            if let Some(capability) = capability {
                if capability.metadata != *blake3::hash(&state.encode(&scope)?).as_bytes()
                    || actual
                        .as_ref()
                        .is_none_or(|a| blake3::hash(&a.plain) != capability.before)
                    || capability.source
                        != *blake3::hash(&unit.snapshot().map_err(invalid)?).as_bytes()
                    || metadata.evidence(unit, &state.ledger).map_err(invalid)?
                        != StudioHandoffEvidence::Complete
                {
                    return Err(invalid("handoff candidate no longer matches Prepared"));
                }
            }
            let protected = if capability.is_none() {
                state
                    .pending()
                    .filter(|(id, _)| state.is_overlay(id))
                    .map(|(id, _)| *id)
                    .collect()
            } else {
                Default::default()
            };
            if target != unit.target()
                || !unit
                    .preserves_vault_source(snapshot, &protected)
                    .map_err(invalid)?
            {
                return Err(invalid("prepared handoff blocks source replacement"));
            }
        } else if capability.is_some() {
            return Err(invalid("handoff Prepared record missing"));
        }
        // Completed bytes may be readable after an uncertain rename. Flush before any normal
        // source rewrite can subsequently publish ordinary retries or erase older evidence.
        if metadata.has_completed() && !metadata.is_prepared() {
            let record = observed.ok_or_else(|| invalid("completed handoff record missing"))?;
            let reservation = budget
                .reserve_sync(&storage_scope, record)
                .map_err(invalid)?;
            // I-4: the completed-handoff exact retry is a mutation for inventory purposes.
            let path = self.epoch_intent_path(&scope);
            let bytes = old.ok_or_else(|| invalid("completed handoff record missing"))?;
            self.epoch_mutation_guard().sync_intent(&path, bytes)?;
            reservation.commit();
        }
        Ok(true)
    }

    /// A persistent local source link distinguishes a missing handoff record from a remote
    /// source that never had local intents. It survives later rotation/adoption and restart.
    pub(super) fn check_studio_intent_link(
        &self,
        server: u64,
        document: &LogicalDocument,
        scope: &[u8],
        plain: &[u8],
    ) -> Result<(), AppError> {
        let (target, _, linked) = decode_record_link(plain, scope, document)?;
        if linked {
            let state = self.load_epoch_intents_structural(server, document)?;
            state
                .handoff_metadata()
                .ok_or_else(|| invalid("required handoff metadata missing"))?
                .check_target(target)
                .map_err(invalid)?;
        }
        Ok(())
    }

    /// Generic providers refuse a Prepared destination; they never expose a batch between the
    /// source and final intent barriers. After uncertain completion, sync the actual record.
    pub(super) fn check_studio_handoff_publication(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
    ) -> Result<(), AppError> {
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let scope = epoch_intents::scope_bytes(server, &document)?;
        let (state, bytes) = self.read_epoch_intent_record_structural(&scope, &document)?;
        if let Some(metadata) = state.handoff_metadata() {
            metadata.check_target(target).map_err(invalid)?;
            if metadata.is_prepared() {
                return Err(invalid("prepared overlay handoff is not publishable"));
            }
            if metadata.has_completed() {
                // I-4: same exact-retry flush, on the publish check's path. Every page serve of
                // this target comes through here, so a flush this mount already made, of a file
                // nothing has written since, is not repeated (I-4 audit M-3).
                let path = self.epoch_intent_path(&scope);
                let bytes = bytes.ok_or_else(|| invalid("completed handoff record missing"))?;
                self.sync_intent_unless_durable(&path, bytes)?;
            }
        }
        Ok(())
    }
}
