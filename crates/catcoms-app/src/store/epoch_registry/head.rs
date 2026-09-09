//! Receipt-head selection shares the store's exclusive gate and both inventory checks. Serving
//! never seals a source, changes the selected checkpoint, retires intents or completes publication.
use super::*;
use catcoms_sync::receipt_head::ReceiptHeadSelection;

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
            sync_registry,
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
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<ReceiptHeadSelection, AppError> {
        if group.member_signature_key(&device.device_id()).as_deref()
            != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("head provider is not a current member"));
        }
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        // Validate the saved source once; neither a missing indexed source nor corruption can
        // hide a fault. Absence is checked against the complete inventory before a None answer.
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
            let held = unit
                .as_ref()
                .map(RegistryEpoch::receipt_head)
                .transpose()
                .map_err(invalid)?
                .flatten()
                .cloned();
            Ok::<_, AppError>((held, record))
        })();
        let (held, record) = match loaded {
            Ok(value) => value,
            Err(error) => {
                budget.invalidate();
                return Err(error);
            }
        };
        budget
            .verify_record(&storage_scope, *blake3::hash(&scope).as_bytes(), record)
            .map_err(invalid)?;
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
            sync,
        )
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
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<ReceiptHeadSelection, AppError> {
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let (journal, owner_record) = (|| {
            Ok::<_, AppError>((
                self.load_epoch_owner_receipts(server, &document)?,
                self.epoch_owner_receipt_inventory_record(server, &document)?,
            ))
        })()
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
        let own_choice = journal.pending().or_else(|| journal.published());
        let is_owner = group.designated_committer() == Some(device.device_id());
        let selected = if is_owner {
            own_choice.or(held)
        } else {
            held.or(own_choice)
        };
        // A pending decision wins over the published journal head. If the source hasn't admitted
        // it yet, it is only a hint: never freshly prove an older published fallback. Requiring
        // exact equality also refuses same-tenure equivocation, inherited-baseline disagreement
        // and a source ahead of the journal, without modifying either file to manufacture agreement.
        let prove = is_owner
            && selected.is_some()
            && selected == held
            && selected == own_choice
            && durable_tenure.is_some_and(|t| {
                selected.is_some_and(|r| r.verify_current_owner(group, t).is_ok())
            });
        let receipt = selected.cloned();
        if prove {
            let record = record.ok_or_else(|| invalid("proof source missing"))?;
            let reservation = budget
                .reserve_sync(&storage_scope, record)
                .map_err(invalid)?;
            sync(
                &self.registry_epoch_path(&scope),
                record.footprint.total().map_err(invalid)?,
            )?;
            reservation.commit();
            // Re-save even an exact published retry: a previously visible rename is not itself
            // evidence of a successful parent flush. No mark-published or receipt issuance here.
            self.prepare_epoch_owner_receipt(
                server,
                receipt.clone().expect("selected proof"),
                group,
                durable_tenure.expect("known proof tenure"),
                rng,
                budget,
            )?;
        }
        Ok(ReceiptHeadSelection { receipt, prove })
    }

    /// Same journal/flush/signing-selection barriers as the explicit cold adapter, without a
    /// second restore of an already prepared source. The caller retains its preparation slot.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_registry_head_prepared(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        tenure: Option<u64>,
        rng: &mut impl CryptoRngCore,
        prepared: Option<(
            &super::RegistrySourceStamp,
            &catcoms_replication::registry_epoch::catchup::RegistryPageSource,
        )>,
        budget: &mut EpochStorageBudget,
    ) -> Result<ReceiptHeadSelection, AppError> {
        let (head, record) = self
            .checked_registry_checkpoint_source(server, group, bucket, device, prepared, budget)?;
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
            sync_registry,
        )
    }
}
