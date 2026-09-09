//! A Registry pointer is a discoverability hint derived from the actual held Studio source,
//! never a renderer-supplied checkpoint number or permission to install an epoch.
use super::*;
use catcoms_replication::registry::{registry_document, PointerKey, RegistryOp, RegistryRecovery};

impl ServerStore {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn studio_registry_maintenance_hint(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        key: &PointerKey,
        prepared: Option<(
            &crate::store::RegistrySourceStamp,
            &catcoms_replication::registry_epoch::catchup::RegistryPageSource,
        )>,
        budget: &mut EpochStudioBudget,
    ) -> Result<
        Option<catcoms_replication::registry_epoch::catchup::RegistryMaintenanceHint>,
        AppError,
    > {
        self.enter_studio_budget(server, group, budget)?;
        self.registry_maintenance_hint(
            server,
            group,
            bucket,
            device,
            key,
            prepared,
            &mut budget.storage,
            &mut budget.intents,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn maintain_studio_registry_owner(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        tenure: u64,
        clock: &dyn catcoms_rt::Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<Option<(RegistryOwnerRotationOutcome, EpochRegistryState)>, AppError> {
        self.enter_studio_budget(server, group, budget)?;
        self.maintain_registry_owner(
            server,
            group,
            bucket,
            device,
            tenure,
            clock,
            rng,
            &mut budget.storage,
            &mut budget.intents,
        )
    }
    /// Refresh one already-opened object's pointer through the existing signed Registry edit
    /// and intent barriers. Exact retries use a derived nonce, so failure between those barriers
    /// cannot manufacture a new intent on each idle turn. The caller drives ordinary tail sync.
    pub(crate) fn refresh_studio_registry_pointer(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<Option<EpochRegistryState>, AppError> {
        let source =
            self.with_studio_checkpoint_source(server, group, target, device, budget, |s| {
                Ok((s.epoch(), s.phase(), s.op_count()))
            })?;
        let Some((epoch, EpochPhase::Open, count)) = source else {
            return Ok(None);
        };
        if epoch == 0 && count == 0 {
            return Ok(None);
        }
        let logical = target.document(&group.group_id()).map_err(invalid)?;
        let key =
            PointerKey::new(logical.doc_type, logical.logical_key.clone()).map_err(invalid)?;
        let bucket = key.bucket();
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let state = self.load_registry_epoch(server, group, bucket, device)?;
        if state
            .as_ref()
            .is_some_and(|s| s.phase() != EpochPhase::Open)
        {
            return Ok(None);
        }
        let current_id = state
            .as_ref()
            .map(EpochRegistryState::doc_id)
            .unwrap_or_else(|| {
                catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key)
            });
        if let Some(state) = &state {
            let projection = state.projection()?;
            if projection.tombstones.contains(&key)
                || projection
                    .pointers
                    .get(&key)
                    .into_iter()
                    .chain(projection.overflow.get(&key))
                    .any(|current| *current >= epoch)
            {
                // A post-rename failure can leave this exact pointer visible but unflushed.
                // Repair unchanged intent/source durability rather than claiming success from
                // projection equality. Normal idle no-ops use the prepared-source seam below.
                self.flush_checked_epoch_intents(
                    server,
                    &document,
                    &mut budget.storage,
                    &mut budget.intents,
                )?;
                self.ingest_registry_page(
                    server,
                    group,
                    bucket,
                    current_id,
                    device,
                    &[],
                    rng,
                    &mut budget.storage,
                )?;
                return Ok(None);
            }
        }
        // Rotation retires Registry tombstones from the seed. Ordinary refresh is not Restore:
        // do not recreate a deleted pointer while retained/staged recovery still names it.
        let old = self.load_epoch_recovery(server, &document)?;
        let observed = self.epoch_recovery_inventory_record(server, &document)?;
        let scope = super::super::epoch_recovery::scope_bytes(server, &document)?;
        budget
            .storage
            .verify_record(
                &StorageScope::new(server, &document.server_id).map_err(invalid)?,
                *blake3::hash(&scope).as_bytes(),
                observed,
            )
            .map_err(invalid)?;
        for slot in old.retained().chain(old.staged()) {
            if RegistryRecovery::from_snapshot(slot, &document, bucket)
                .map_err(invalid)?
                .projection()
                .tombstones
                .contains(&key)
            {
                return Ok(None);
            }
        }
        let mut hash = blake3::Hasher::new_derive_key("catcoms/studio-registry-refresh/v1");
        for part in [
            logical.server_id.as_slice(),
            &logical.doc_type.tag().to_be_bytes(),
            logical.logical_key.as_slice(),
            &epoch.to_be_bytes(),
            &current_id.to_be_bytes(),
            device.device_id().as_bytes(),
        ] {
            hash.update(&(part.len() as u64).to_be_bytes());
            hash.update(part);
        }
        let mut nonce = [0; 16];
        nonce.copy_from_slice(&hash.finalize().as_bytes()[..16]);
        let operation = RegistryOp::Put { key, epoch }
            .domain_op(&group.group_id(), nonce)
            .map_err(invalid)?;
        let (_, mut state) = self.edit_registry_epoch(
            server,
            group,
            bucket,
            current_id,
            device,
            operation,
            rng,
            &mut budget.storage,
            &mut budget.intents,
        )?;
        // Reuse the existing pure footprint cache; don't retain a second Registry graph.
        self.remember_installed_registry(server, &group.group_id(), bucket, &mut state)?;
        Ok(Some(state))
    }
}
