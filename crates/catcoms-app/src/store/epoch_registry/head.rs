//! Receipt-head selection shares the store's exclusive gate and both inventory checks. Serving
//! never seals a source, changes the selected checkpoint, retires intents or completes publication.
use super::*;
use catcoms_replication::ReceiptRepair;
use catcoms_sync::{receipt_head::ReceiptHeadSelection, ArchivedOwnerTenure};

impl ServerStore {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_registry_head(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        durable_tenure: Option<u64>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<ReceiptHeadSelection, AppError> {
        self.prepare_registry_head_with_sync(
            server,
            group,
            bucket,
            device,
            durable_tenure,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_registry_head_with_sync(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        durable_tenure: Option<u64>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<ReceiptHeadSelection, AppError> {
        self.prepare_registry_head_and_repair(
            server,
            group,
            bucket,
            device,
            durable_tenure,
            None,
            None,
            rng,
            budget,
            hooks,
        )
        .map(|(selection, _)| selection)
    }

    /// Completion of an exact proved Registry decision. Publication-aware like Studio's: a
    /// repaired reconciliation can publish, while a held repair still refuses in the writer.
    pub(crate) fn complete_registry_head_publication(
        &mut self,
        server: u64,
        receipt: &Receipt,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<(), AppError> {
        self.mark_epoch_owner_publication(server, &receipt.document, receipt.hash(), rng, budget)
            .map(|_| ())
    }

    /// The head selection plus a durably applied fault repair safe to serve beside it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_registry_head_with_fault_repair(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        durable_tenure: Option<u64>,
        archived_owner: Option<&ArchivedOwnerTenure>,
        fault_report: Option<&[Receipt; 2]>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<(ReceiptHeadSelection, Option<ReceiptRepair>), AppError> {
        self.prepare_registry_head_and_repair(
            server,
            group,
            bucket,
            device,
            durable_tenure,
            archived_owner,
            fault_report,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn prepare_registry_head_and_repair(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        durable_tenure: Option<u64>,
        archived_owner: Option<&ArchivedOwnerTenure>,
        fault_report: Option<&[Receipt; 2]>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(ReceiptHeadSelection, Option<ReceiptRepair>), AppError> {
        if group.member_signature_key(&device.device_id()).as_deref()
            != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("head provider is not a current member"));
        }
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        // Restore and account the saved source before accepting any report. Head eligibility is
        // deliberately checked later: a valid Fault must refuse service, but it must not prevent
        // an independently authorized report from crossing B0 first.
        let loaded = (|| {
            let bytes = self.read_registry_record(&scope)?;
            let unit = bytes
                .as_ref()
                .map(|bytes| {
                    let (_, snapshot) = decode_record(&bytes.plain, &scope, &document)?;
                    RegistryEpoch::restore(snapshot, group, bucket, device.device_id())
                        .map_err(invalid)
                })
                .transpose()?;
            let record = bytes
                .as_ref()
                .zip(unit.as_ref())
                .map(|(bytes, unit)| {
                    storage_record(
                        server,
                        &document,
                        &scope,
                        bytes.physical_bytes,
                        unit.storage_protocol_bytes().map_err(invalid)?,
                    )
                })
                .transpose()?;
            Ok::<_, AppError>((unit, record))
        })();
        let (unit, record) = match loaded {
            Ok(value) => value,
            Err(error) => {
                budget.invalidate();
                return Err(error);
            }
        };
        budget
            .verify_record(&storage_scope, *blake3::hash(&scope).as_bytes(), record)
            .map_err(invalid)?;
        let applied = unit
            .as_ref()
            .and_then(|u| u.repair_state().map(|state| state.repair));
        self.admit_registry_head_report(
            server,
            &document,
            group,
            device,
            durable_tenure,
            archived_owner,
            fault_report,
            applied.as_ref(),
            rng,
            budget,
            hooks,
        )?;
        // Preserve Fault as an explicit service refusal. Admission above is evidence persistence,
        // not permission to reconstruct, relabel, or serve a head from this source.
        let held = unit
            .as_ref()
            .map(RegistryEpoch::receipt_head)
            .transpose()
            .map_err(invalid)
            .inspect_err(|_| budget.invalidate())?
            .flatten()
            .cloned();
        self.finish_registry_head_source(
            server,
            group,
            bucket,
            device,
            durable_tenure,
            rng,
            budget,
            held,
            record,
            applied,
            hooks,
        )
    }

    /// Persist independently provable fault evidence only after the exact source and its complete
    /// inventory have been authenticated. This barrier intentionally precedes head selection so a
    /// correctly faulted source can refuse service without discarding the report that explains it.
    #[allow(clippy::too_many_arguments)]
    fn admit_registry_head_report(
        &mut self,
        server: u64,
        document: &catcoms_replication::LogicalDocument,
        group: &ServerGroup,
        device: &MlsDevice,
        durable_tenure: Option<u64>,
        archived_owner: Option<&ArchivedOwnerTenure>,
        fault_report: Option<&[Receipt; 2]>,
        carried: Option<&ReceiptRepair>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(), AppError> {
        if let (Some(report), Some(tenure)) = (fault_report, durable_tenure) {
            self.admit_fault_report_with_writer(
                server,
                document,
                group,
                device,
                tenure,
                archived_owner,
                report,
                carried,
                rng,
                budget,
                hooks,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_registry_head_source(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        durable_tenure: Option<u64>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        held: Option<Receipt>,
        record: Option<StorageRecord>,
        applied: Option<ReceiptRepair>,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(ReceiptHeadSelection, Option<ReceiptRepair>), AppError> {
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        // Contextual, so a record a repair transaction holds is read rather than refused; a held
        // repair never permits a proof, and before B2 not even a hint.
        let (journal, owner_record) = self
            .load_epoch_owner_repair_state(server, &document, &device.device_id(), group.epoch())
            .inspect_err(|_| budget.invalidate())?;
        let owner_scope = super::super::epoch_owner::scope_bytes(server, &document)?;
        budget
            .verify_record(
                &storage_scope,
                *blake3::hash(&owner_scope).as_bytes(),
                owner_record,
            )
            .map_err(invalid)?;
        let held = held.as_ref();
        let is_owner = group.designated_committer() == Some(device.device_id());
        let servable = applied.filter(|r| {
            is_owner && durable_tenure.is_some_and(|t| r.verify_current_owner(group, t).is_ok())
        });
        if let Some((pending, _, _)) = journal.held_repair() {
            let applied = servable.filter(|r| r == pending);
            let receipt = applied.as_ref().and(held.cloned());
            if applied.is_some() {
                self.flush_registry_repair_service_barriers(
                    server,
                    &document,
                    record.expect("servable repair source"),
                    group,
                    device,
                    rng,
                    budget,
                    hooks,
                )?;
            }
            return Ok((
                ReceiptHeadSelection {
                    receipt,
                    prove: false,
                },
                applied,
            ));
        }
        // Effective publication choice; identical to pending-then-published when ordinary.
        let own_choice = journal.journal().effective_choice();
        let selected = if is_owner {
            own_choice.or(held)
        } else {
            held.or(own_choice)
        };
        // A pending decision wins over the published journal head. If the source hasn't admitted
        // it yet, it is only a hint: never freshly prove an older published fallback. Requiring
        // exact equality also refuses same-tenure equivocation, inherited-baseline disagreement
        // and a source ahead of the journal, without modifying either file to manufacture agreement.
        // The durable proof gate (6.6) is recomputed from durable state on every request.
        let current = durable_tenure.map(|t| {
            catcoms_replication::epoch::tenure_id(&group.group_id(), &device.public_key_bytes(), t)
        });
        let gated = selected
            .is_some_and(|r| current.is_none_or(|c| journal.fault_suppresses_proof(r.hash(), c)));
        // A disputed receipt is not even offered as a hint, and nothing unserved is proved.
        let receipt = selected
            .filter(|r| !journal.fault_retains_member(r.hash()))
            .cloned();
        let prove = is_owner
            && !gated
            && receipt.is_some()
            && selected == held
            && selected == own_choice
            && durable_tenure.is_some_and(|t| {
                selected.is_some_and(|r| r.verify_current_owner(group, t).is_ok())
            });
        let repair_durable = servable.is_some();
        if repair_durable {
            self.flush_registry_repair_service_barriers(
                server,
                &document,
                record.expect("servable repair source"),
                group,
                device,
                rng,
                budget,
                hooks,
            )?;
        }
        if prove {
            let record = record.ok_or_else(|| invalid("proof source missing"))?;
            if !repair_durable {
                let reservation = budget
                    .reserve_sync(&storage_scope, record)
                    .map_err(invalid)?;
                // I-4: an unchanged-file flush is a mutation for inventory purposes.
                let path = self.registry_epoch_path(&scope);
                let bytes = record.footprint.total().map_err(invalid)?;
                let mutation = self.epoch_mutation_guard();
                hooks.before_sync(WriteTag::Source, &path, bytes)?;
                sync_registry(&mutation, &path, bytes)?;
                hooks.after_sync(WriteTag::Source, &path)?;
                reservation.commit();
            }
            // Re-save even an exact published retry: a previously visible rename is not itself
            // evidence of a successful parent flush. No mark-published or receipt issuance here.
            self.prepare_epoch_owner_publication_with_writer(
                server,
                receipt.clone().expect("selected proof"),
                group,
                durable_tenure.expect("known proof tenure"),
                rng,
                budget,
                &mut WriteHooks::None,
            )?;
        }
        Ok((ReceiptHeadSelection { receipt, prove }, servable))
    }

    #[allow(clippy::too_many_arguments)]
    fn flush_registry_repair_service_barriers(
        &mut self,
        server: u64,
        document: &catcoms_replication::LogicalDocument,
        record: StorageRecord,
        group: &ServerGroup,
        device: &MlsDevice,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(), AppError> {
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let reservation = budget
            .reserve_sync(&storage_scope, record)
            .map_err(invalid)?;
        let scope = scope_bytes(server, document)?;
        let path = self.registry_epoch_path(&scope);
        let bytes = record.footprint.total().map_err(invalid)?;
        let mutation = self.epoch_mutation_guard();
        hooks.before_sync(WriteTag::Source, &path, bytes)?;
        sync_registry(&mutation, &path, bytes)?;
        hooks.after_sync(WriteTag::Source, &path)?;
        reservation.commit();
        self.resave_epoch_owner_repair_state_with_writer(
            server,
            document,
            &device.device_id(),
            group.epoch(),
            rng,
            budget,
            hooks,
        )
    }

    /// Same journal/flush/signing-selection barriers as the explicit cold adapter, without a
    /// second restore of an already prepared source, carrying a durably applied fault repair
    /// when servable. The caller retains its preparation slot.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_registry_head_prepared_with_fault_repair(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        tenure: Option<u64>,
        archived_owner: Option<&ArchivedOwnerTenure>,
        fault_report: Option<&[Receipt; 2]>,
        rng: &mut impl CryptoRngCore,
        prepared: Option<(
            &super::RegistrySourceStamp,
            &catcoms_replication::registry_epoch::catchup::RegistryPageSource,
        )>,
        budget: &mut EpochStorageBudget,
    ) -> Result<(ReceiptHeadSelection, Option<ReceiptRepair>), AppError> {
        // Reauthenticate the exact prepared wrapper and complete physical inventory before the
        // report can write anything. Head eligibility remains a later, separate question.
        let record =
            self.checked_registry_prepared_record(server, group, bucket, device, prepared, budget)?;
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let applied = prepared.and_then(|(_, source)| source.fault_repair());
        self.admit_registry_head_report(
            server,
            &document,
            group,
            device,
            tenure,
            archived_owner,
            fault_report,
            applied.as_ref(),
            rng,
            budget,
            &mut WriteHooks::None,
        )?;
        // Fault remains a hard refusal and invalidates this budget exactly as before; successful
        // B0 persistence does not manufacture a head or permit synchronous reconstruction.
        let head = prepared
            .map(|(_, source)| source.receipt_head().map(|r| r.cloned()).map_err(invalid))
            .transpose()
            .inspect_err(|_| budget.invalidate())?
            .flatten();
        self.finish_registry_head_source(
            server,
            group,
            bucket,
            device,
            tenure,
            rng,
            budget,
            head,
            record,
            applied,
            &mut WriteHooks::None,
        )
    }
}
