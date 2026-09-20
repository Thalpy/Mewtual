//! Explicit core/store transfer only. No background actor loop or network publication batch.
use super::super::epoch_intents::{self, EpochIntentState};
use super::*;
use catcoms_replication::studio::{
    StudioHandoffEvidence, StudioHandoffOutcome, StudioOverlayState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HandoffWrite {
    Prepared,
    Source,
    Completed,
    Active,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HandoffSync {
    Source,
    Intents,
}

/// What H1 concluded. `Settled` is terminal and already durable: an acknowledgement of a branch
/// this device already transferred. `Captured` is work whose expensive reconstruction has not
/// happened yet.
pub(crate) enum StudioHandoffStart {
    Settled(StudioHandoffOutcome),
    Captured(Box<StudioHandoffCapture>),
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
            &mut |m: &EpochMutation<'_>, _, p: &Path, b: &[u8]| m.write(p, b),
            &mut |m, step, p, b| match step {
                HandoffSync::Source => sync_studio(m, p, b),
                HandoffSync::Intents => epoch_intents::sync_intent(m, p, b),
            },
        )
    }

    /// Index creation still requires the actual independently saved object source.
    ///
    /// This runs at **both** H1 and H5, not only at H1. The single custody visit got that for
    /// free; a scheduled H1 to H5 spans many background turns, and the stamp covers only the Index
    /// document's own intent and source records, so the referenced Flipnote's source can be
    /// evicted, retired or cleaned up in between without invalidating anything. Committing then
    /// would leave a durable Index entry pointing at a source that no longer exists.
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
            &mut |m: &EpochMutation<'_>, _, p: &Path, b: &[u8]| m.write(p, b),
            &mut |m, step, p, b| match step {
                HandoffSync::Source => sync_studio(m, p, b),
                HandoffSync::Intents => epoch_intents::sync_intent(m, p, b),
            },
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
            &mut |m: &EpochMutation<'_>, _, p: &Path, b: &[u8]| m.write(p, b),
            &mut |m, step, p, b| match step {
                HandoffSync::Source => sync_studio(m, p, b),
                HandoffSync::Intents => epoch_intents::sync_intent(m, p, b),
            },
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
        writer: &mut impl FnMut(&EpochMutation<'_>, HandoffWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(&EpochMutation<'_>, HandoffSync, &Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioHandoffOutcome, AppError> {
        let capture = match self.start_studio_handoff_with_io(
            server, group, target, device, basis, tenure, rng, budget, writer, sync,
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
            writer,
            sync,
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
        writer: &mut impl FnMut(&EpochMutation<'_>, HandoffWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(&EpochMutation<'_>, HandoffSync, &Path, u64) -> Result<(), AppError>,
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
                HandoffWrite::Completed,
                rng,
                budget,
                writer,
                sync,
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
                server, group, target, device, rng, budget, writer, sync,
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
        // Index creation still requires the actual independently saved object source.
        self.check_index_object_sources(server, group, target, device, &document, &state)?;
        // The short live-authority mint. Structural metadata is enough: this reads the target,
        // the active branch's author and its receipt, and checks them against live membership,
        // MLS epoch and the observed tenure. It reconstructs nothing.
        let authority = state
            .handoff_metadata()
            .ok_or_else(|| invalid("overlay metadata missing"))?
            .handoff_authority(device, group, tenure)
            .map_err(invalid)?;
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
        writer: &mut impl FnMut(&EpochMutation<'_>, HandoffWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(&EpochMutation<'_>, HandoffSync, &Path, u64) -> Result<(), AppError>,
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
        } = commit;
        let document = stamp.document.clone();
        let (source, observed, before) =
            self.checked_studio_source(server, group, target, device, false, &mut budget.storage)?;
        if observed.is_none() {
            return Err(invalid("overlay destination source missing"));
        }
        drop(source);
        let original_source = self
            .read_studio_record(&scope_bytes(server, &document)?)?
            .ok_or_else(|| invalid("overlay destination source missing"))?;
        let original_source_hash = blake3::hash(&original_source.plain);
        drop(original_source);
        // Against the state as H2 read it, before the prepared overlay is installed, exactly as
        // when H4 and H5 were one function.
        self.check_handoff_references(&prepared, &candidate, &state)?;
        self.check_index_object_sources(server, group, target, device, &document, &state)?;
        let scope = epoch_intents::scope_bytes(server, &document)?;
        let source_scope = scope_bytes(server, &document)?;
        // Physical size and the authenticated plaintext digest are all the unchanged fence needs;
        // decoding again would reconstruct the whole branch a second time.
        let original = self.read_scoped_intent_plain(&scope)?;
        let old = original.as_ref().map(|record| record.physical_bytes);
        let original = original.map(|record| blake3::hash(&record.plain));
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
            HandoffWrite::Prepared,
            rng,
            budget,
            writer,
            sync,
        )?;
        let capability = CheckedHandoffWrite {
            metadata: metadata_hash,
            source: *blake3::hash(&snapshot).as_bytes(),
            before: original_source_hash,
        };
        // Only this private capability can accompany the checked complete candidate.
        self.save_studio_source_checked(
            server,
            candidate,
            observed,
            &before,
            WritePurpose::Ordinary,
            rng,
            &mut budget.storage,
            |m, p, b| writer(m, HandoffWrite::Source, p, b),
            |m, p, b| sync(m, HandoffSync::Source, p, b),
            None,
            Some(&capability),
        )?;
        self.resolve_studio_handoff_with_io(
            server, group, target, device, rng, budget, writer, sync,
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
        step: HandoffWrite,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        writer: &mut impl FnMut(&EpochMutation<'_>, HandoffWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(&EpochMutation<'_>, HandoffSync, &Path, u64) -> Result<(), AppError>,
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
            |m, p, b| writer(m, step, p, b),
            |m, p, b| sync(m, HandoffSync::Intents, p, b),
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
            rng,
            budget,
            &mut |m: &EpochMutation<'_>, _, p: &Path, b: &[u8]| m.write(p, b),
            &mut |m, step, p, b| match step {
                HandoffSync::Source => sync_studio(m, p, b),
                HandoffSync::Intents => epoch_intents::sync_intent(m, p, b),
            },
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_studio_handoff_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        writer: &mut impl FnMut(&EpochMutation<'_>, HandoffWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(&EpochMutation<'_>, HandoffSync, &Path, u64) -> Result<(), AppError>,
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
                    HandoffWrite::Completed,
                    rng,
                    budget,
                    writer,
                    sync,
                )?;
            }
            return Ok(());
        }
        let (source, observed, before) =
            self.checked_studio_source(server, group, target, device, false, &mut budget.storage)?;
        if observed.is_none() {
            return Err(invalid("prepared handoff source missing"));
        }
        match metadata.evidence(&source, &state.ledger).map_err(invalid)? {
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
                    HandoffWrite::Active,
                    rng,
                    budget,
                    writer,
                    sync,
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
                    |_, _, _| Err(invalid("handoff resolution requires unchanged source")),
                    |m, p, b| sync(m, HandoffSync::Source, p, b),
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
                    HandoffWrite::Completed,
                    rng,
                    budget,
                    writer,
                    sync,
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
        let mut retained = source.blob_cids().map_err(invalid)?;
        for (_, intent) in state.pending() {
            retained.extend(
                catcoms_replication::studio::operation_blob_cid(&intent.operation)
                    .map_err(invalid)?,
            );
        }
        if let Some(overlay) = metadata.overlay() {
            if !overlay
                .base_blob_cids()
                .map_err(invalid)?
                .is_subset(&retained)
            {
                return Err(invalid("handoff would release a base blob reference"));
            }
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
                // I-4: same exact-retry flush, on the publish check's path.
                let path = self.epoch_intent_path(&scope);
                let bytes = bytes.ok_or_else(|| invalid("completed handoff record missing"))?;
                self.epoch_mutation_guard().sync_intent(&path, bytes)?;
            }
        }
        Ok(())
    }
}
