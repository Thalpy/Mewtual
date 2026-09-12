//! Recovery-first registry settlement step. It saves recovery, never replaces the source.

use super::*;
use catcoms_replication::registry::RegistryRecovery;
use catcoms_replication::CloseRecord;
use catcoms_rt::Clock;

impl ServerStore {
    /// Recompute a receipt-bound plan from the checked saved registry and durably stage its
    /// typed recovery using the server's complete budget. No caller-supplied stale plan is used.
    /// None means no excluded edits, overflow or tombstones need a new snapshot. Some returns
    /// the saved slots/warning; EvictionPending must hold future installation in Closing.
    ///
    /// This does NOT install a checkpoint, retire intents or prune history. Source bytes remain
    /// unchanged on success/error. Failed I/O grants no success and requires full reconciliation.
    /// A first/second snapshot can still be refused at the content cap: the future multi-record
    /// transaction must reserve settlement-wide headroom rather than crediting early deletion.
    #[allow(clippy::too_many_arguments)]
    pub fn stage_registry_recovery(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        close_bytes: &[u8],
        expected_tenure_start: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<Option<EpochRecoveryUpdate>, AppError> {
        self.stage_registry_recovery_with_writer(
            server,
            group,
            bucket,
            device,
            close_bytes,
            expected_tenure_start,
            clock,
            rng,
            budget,
            atomic_write,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn stage_registry_recovery_with_writer(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        close_bytes: &[u8],
        expected_tenure_start: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
    ) -> Result<Option<EpochRecoveryUpdate>, AppError> {
        let close = CloseRecord::decode(close_bytes).map_err(invalid)?;
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        // Authenticate the actual source footprint too. Verifying only the recovery record would
        // let a stale/partial inventory miss the entire accepted log while admitting new bytes.
        let loaded = (|| {
            let held = self.read_registry_record(&scope)?;
            let Some(bytes) = held else {
                return Ok(None);
            };
            let (stored_bucket, snapshot) = decode_record(&bytes.plain, &scope, &document)?;
            if stored_bucket != bucket {
                return Err(invalid("wrong bucket"));
            }
            let unit = RegistryEpoch::restore(snapshot, group, bucket, device.device_id())
                .map_err(invalid)?;
            let observed = storage_record(
                server,
                &document,
                &scope,
                bytes.physical_bytes,
                unit.storage_protocol_bytes().map_err(invalid)?,
            )?;
            Ok(Some((unit, observed)))
        })();
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                budget.invalidate();
                return Err(error);
            }
        };
        budget
            .verify_record(
                &storage_scope,
                *blake3::hash(&scope).as_bytes(),
                loaded.as_ref().map(|(_, observed)| *observed),
            )
            .map_err(invalid)?;
        let (mut unit, _) = loaded.ok_or_else(|| invalid("registry recovery source is missing"))?;
        let plan = unit
            .prepare_settlement(&close, group, expected_tenure_start)
            .map_err(invalid)?;
        let Some(snapshot) = plan.recovery_snapshot().map_err(invalid)? else {
            return Ok(None);
        };
        // The old generic adapter admits opaque projections. Validate EVERY existing registry
        // slot before returning them as a typed saved result; corrupt/legacy opaque payloads are
        // preserved and fail closed, never replaced as if absent. No await or store borrow escapes.
        let checked = (|| {
            let old = self.load_epoch_recovery(server, &document)?;
            for held in old.retained().chain(old.staged()) {
                RegistryRecovery::from_snapshot(held, &document, bucket).map_err(invalid)?;
            }
            Ok(())
        })();
        if let Err(error) = checked {
            budget.invalidate();
            return Err(error);
        }
        self.update_epoch_recovery_accounted_with_writer(
            server,
            &document,
            EpochRecoveryAction::Stage(snapshot),
            clock,
            rng,
            budget,
            writer,
        )
        .map(Some)
    }
}
