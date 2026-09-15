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
            &mut |_, p, b| atomic_write(p, b),
            &mut |step, p, b| match step {
                HandoffSync::Source => sync_studio(p, b),
                HandoffSync::Intents => epoch_intents::sync_intent(p, b),
            },
        )
    }
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
        writer: &mut impl FnMut(HandoffWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(HandoffSync, &Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioHandoffOutcome, AppError> {
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
            return Ok(outcome);
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
                return Ok(outcome);
            }
        }
        let tenure =
            tenure.ok_or_else(|| invalid("overlay handoff needs observed owner tenure"))?;
        // Index creation still requires the actual independently saved object source.
        if let StudioTarget::Index { channel } = target {
            for (id, intent) in state.pending().filter(|(id, _)| state.is_overlay(id)) {
                let _ = id;
                if let IndexOp::PutObject { object, .. } =
                    IndexOp::decode_domain(&document, &intent.operation, &intent.author)
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
        }
        let (mut source, observed, before) =
            self.checked_studio_source(server, group, target, device, false, &mut budget.storage)?;
        if observed.is_none() {
            return Err(invalid("overlay destination source missing"));
        }
        let original_source = self
            .read_studio_record(&scope_bytes(server, &document)?)?
            .ok_or_else(|| invalid("overlay destination source missing"))?;
        let original_source_hash = blake3::hash(&original_source.plain);
        drop(original_source);
        let metadata = state
            .handoff_metadata()
            .ok_or_else(|| invalid("overlay metadata missing"))?;
        let candidate = metadata
            .prepare_handoff(&mut source, &state.ledger, device, group, tenure, rng)
            .map_err(invalid)?;
        drop(source);
        let (mut candidate, prepared) = candidate.into_parts();
        let completed = prepared
            .complete(&candidate, &state.ledger)
            .map_err(invalid)?;
        self.check_handoff_references(&prepared, &candidate, &state)?;
        let scope = epoch_intents::scope_bytes(server, &document)?;
        // Physical size and the authenticated plaintext digest are all the unchanged fence needs;
        // decoding again would reconstruct the whole branch a second time.
        let original = self.read_scoped_intent_plain(&scope)?;
        let old = original.as_ref().map(|record| record.physical_bytes);
        let original = original.map(|record| blake3::hash(&record.plain));
        let mut completed_state = state.clone();
        completed_state.overlay = Some(completed);
        state.overlay = Some(prepared);
        let prepared_bytes = state.encode(&scope)?.len() as u64 + 40;
        let completed_bytes = completed_state.encode(&scope)?.len() as u64 + 40;
        let source_scope = scope_bytes(server, &document)?;
        let snapshot = Zeroizing::new(candidate.snapshot().map_err(invalid)?);
        let mut e = Encoder::new();
        e.put_bytes(&source_scope).map_err(invalid)?;
        e.put_bytes(&target.channel()).map_err(invalid)?;
        e.put_bytes(&snapshot).map_err(invalid)?;
        e.put_u8(1); // Durable source-to-intent link, also charged by the common writer.
        let source_bytes = e.finish().len() as u64 + 40;
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
            |p, b| writer(HandoffWrite::Source, p, b),
            |p, b| sync(HandoffSync::Source, p, b),
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
        writer: &mut impl FnMut(HandoffWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(HandoffSync, &Path, u64) -> Result<(), AppError>,
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
            |p, b| writer(step, p, b),
            |p, b| sync(HandoffSync::Intents, p, b),
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
            &mut |_, p, b| atomic_write(p, b),
            &mut |step, p, b| match step {
                HandoffSync::Source => sync_studio(p, b),
                HandoffSync::Intents => epoch_intents::sync_intent(p, b),
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
        writer: &mut impl FnMut(HandoffWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(HandoffSync, &Path, u64) -> Result<(), AppError>,
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
                    |_, _| Err(invalid("handoff resolution requires unchanged source")),
                    |p, b| sync(HandoffSync::Source, p, b),
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
        &self,
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
            epoch_intents::sync_intent(
                &self.epoch_intent_path(&scope),
                old.ok_or_else(|| invalid("completed handoff record missing"))?,
            )?;
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
        &self,
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
                epoch_intents::sync_intent(
                    &self.epoch_intent_path(&scope),
                    bytes.ok_or_else(|| invalid("completed handoff record missing"))?,
                )?;
            }
        }
        Ok(())
    }
}
