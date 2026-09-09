//! Checkpoint service uses the same sole prepared source and owner journal as other P1 paths.
//! A remote query cannot trigger cold reconstruction, create epoch zero or serialize a server.
use super::*;
use catcoms_sync::receipt_head::ReceiptHeadSelection;

impl ServerStore {
    pub(crate) fn prepared_studio_maintenance_state(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        budget: &mut EpochStudioBudget,
    ) -> Result<Option<(u64, EpochPhase)>, AppError> {
        self.with_studio_checkpoint_source(server, group, target, device, budget, |state| {
            Ok((state.epoch(), state.phase()))
        })
    }
    /// Local publication means the exact checkpoint is durably available to keyed discovery,
    /// not that a remote member received it. Only the Server's current durable-owner callback
    /// may use this path. A merely decided/Closing source cannot complete an in-flight choice.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn complete_studio_installed_head(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        tenure: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<(), AppError> {
        let installed = self
            .with_studio_checkpoint_source(server, group, target, device, budget, |state| {
                let receipt = state
                    .unit
                    .receipt_head()
                    .map_err(invalid)?
                    .cloned()
                    .ok_or_else(|| invalid("installed receipt missing"))?;
                if state.phase() != EpochPhase::Open || !state.unit.opened_by(&receipt) {
                    return Err(invalid("owner head is not an installed open checkpoint"));
                }
                let doc_id = state.doc_id();
                if state
                    .unit
                    .checkpoint_bytes_by_hash(doc_id, receipt.seed_change_hash)
                    .map_err(invalid)?
                    .is_none()
                {
                    return Err(invalid("installed owner seed missing"));
                }
                Ok(receipt)
            })?
            .ok_or_else(|| invalid("installed owner source missing"))?;
        // Reuse source+journal equality, current owner/tenure, inventory and exact flush barriers.
        // No async work can interleave between these checks and the completion write.
        let selected =
            self.prepare_studio_head(server, group, target, device, Some(tenure), rng, budget)?;
        if !selected.prove || selected.receipt.as_ref() != Some(&installed) {
            return Err(invalid("installed owner source and journal disagree"));
        }
        self.complete_studio_head(server, &installed, rng, budget)
    }
    pub(crate) fn complete_registry_studio_handoff(
        &mut self,
        server: u64,
        receipt: &Receipt,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<(), AppError> {
        self.mark_epoch_owner_receipt_published(
            server,
            &receipt.document,
            receipt.hash(),
            rng,
            &mut budget.storage,
        )?;
        Ok(())
    }
    /// Narrow checked status used after detached source preparation. No cold restore, no
    /// mutation and no inferred checkpoint: absence must agree with the inventory as usual.
    pub(crate) fn prepared_studio_status(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        budget: &mut EpochStudioBudget,
    ) -> Result<Option<(u128, EpochPhase)>, AppError> {
        self.with_studio_checkpoint_source(server, group, target, device, budget, |state| {
            Ok((state.doc_id(), state.phase()))
        })
    }
    pub(super) fn with_studio_checkpoint_source<V>(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        budget: &mut EpochStudioBudget,
        read: impl FnOnce(&mut EpochStudioState) -> Result<V, AppError>,
    ) -> Result<Option<V>, AppError> {
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let scope = scope_bytes(server, &document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let result = if self.studio_source_is_warm(server, group, target, device) {
            self.with_prepared_studio_source(server, group, target, device, |state| {
                let record = state
                    .source
                    .as_ref()
                    .ok_or_else(|| invalid("prepared source has no physical stamp"))?
                    .record();
                budget
                    .storage
                    .verify_record(
                        &storage_scope,
                        *blake3::hash(&scope).as_bytes(),
                        Some(record),
                    )
                    .map_err(invalid)?;
                read(state).map(Some)
            })
        } else {
            // Zero-body local probe: actual absence is checked against inventory. Any existing
            // file refuses BEFORE body allocation and must take detached preparation instead.
            // This is not a remote existence predicate or permission to invent an empty source.
            (|| {
                self.read_studio_record_bounded(&scope, 0)?;
                budget
                    .storage
                    .verify_record(&storage_scope, *blake3::hash(&scope).as_bytes(), None)
                    .map_err(invalid)?;
                Ok(None)
            })()
        };
        result.inspect_err(|_| budget.storage.invalidate())
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn read_studio_seed(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        doc_id: u128,
        hash: [u8; 32],
        budget: &mut EpochStudioBudget,
    ) -> Result<Option<Vec<u8>>, AppError> {
        self.with_studio_checkpoint_source(server, group, target, device, budget, |state| {
            state
                .unit
                .checkpoint_bytes_by_hash(doc_id, hash)
                .map_err(invalid)
        })
        .map(Option::flatten)
    }
    /// A proof needs exact agreement between saved source and pending-preferred journal, plus
    /// the caller's current durable owner snapshot. Source flush and journal re-save precede it.
    /// Reading an old receipt cannot manufacture tenure, advance finality or recover a lost file.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_studio_head(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        durable_tenure: Option<u64>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<ReceiptHeadSelection, AppError> {
        self.prepare_studio_head_with_io(
            server,
            group,
            target,
            device,
            durable_tenure,
            rng,
            budget,
            sync_studio,
            atomic_write,
        )
    }
    // Deterministic failure seams use the same transaction/reservations as production.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_studio_head_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        durable_tenure: Option<u64>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
    ) -> Result<ReceiptHeadSelection, AppError> {
        let source =
            self.with_studio_checkpoint_source(server, group, target, device, budget, |state| {
                Ok((
                    state.unit.receipt_head().map_err(invalid)?.cloned(),
                    state
                        .source
                        .as_ref()
                        .ok_or_else(|| invalid("head source has no physical stamp"))?
                        .record(),
                ))
            })?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let journal = (|| {
            let journal = self.load_epoch_owner_receipts(server, &document)?;
            let owner_record = self.epoch_owner_receipt_inventory_record(server, &document)?;
            let owner_scope = super::super::epoch_owner::scope_bytes(server, &document)?;
            budget
                .storage
                .verify_record(
                    &storage_scope,
                    *blake3::hash(&owner_scope).as_bytes(),
                    owner_record,
                )
                .map_err(invalid)?;
            Ok::<_, AppError>(journal)
        })()
        .inspect_err(|_| budget.storage.invalidate())?;
        let held = source.as_ref().and_then(|(r, _)| r.as_ref());
        let own_choice = journal.pending().or_else(|| journal.published());
        let is_owner = group.designated_committer() == Some(device.device_id());
        let selected = if is_owner {
            own_choice.or(held)
        } else {
            held.or(own_choice)
        };
        let prove = is_owner
            && selected.is_some()
            && selected == held
            && selected == own_choice
            && durable_tenure.is_some_and(|t| {
                selected.is_some_and(|r| r.verify_current_owner(group, t).is_ok())
            });
        let receipt = selected.cloned();
        if prove {
            let record = source.expect("matched proof source").1;
            let reservation = budget
                .storage
                .reserve_sync(&storage_scope, record)
                .map_err(invalid)?;
            let scope = scope_bytes(server, &document)?;
            sync(
                &self.studio_epoch_path(&scope),
                record.footprint.total().map_err(invalid)?,
            )?;
            reservation.commit();
            self.prepare_epoch_owner_with_writer(
                server,
                receipt.clone().expect("selected proof"),
                group,
                durable_tenure.expect("proof tenure"),
                rng,
                &mut budget.storage,
                writer,
            )?;
        }
        Ok(ReceiptHeadSelection { receipt, prove })
    }
    /// Complete either a checked reply handoff or exact durable installed-head availability,
    /// under the same exclusive transaction. Neither outcome attests remote delivery. A failed
    /// write requires exact retry after inventory reconciliation.
    pub(crate) fn complete_studio_head(
        &mut self,
        server: u64,
        receipt: &Receipt,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<(), AppError> {
        self.mark_epoch_owner_receipt_published(
            server,
            &receipt.document,
            receipt.hash(),
            rng,
            &mut budget.storage,
        )?;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn complete_studio_head_with_test_failure(
        &mut self,
        server: u64,
        receipt: &Receipt,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        after: bool,
    ) -> Result<(), AppError> {
        self.mark_epoch_owner_with_test_failure(server, receipt, rng, &mut budget.storage, after)?;
        Ok(())
    }
}
