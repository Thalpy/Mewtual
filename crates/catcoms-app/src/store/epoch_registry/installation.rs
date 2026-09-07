//! One exclusive-store-borrow settlement sequence. Every destructive step follows a durable
//! proof: source flush -> recovery -> included-intent retirement -> atomic successor selection.
//! Crashes leave either the full Closing source or the checked successor, never a partial unit.

use super::*;
use catcoms_replication::{registry::RegistryRecovery, CloseRecord};
use catcoms_rt::Clock;

/// Installation result, not network publication or evidence of current-owner discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryInstallOutcome {
    /// Recovery and receipt-covered retirement are durable; the successor is now selected.
    Installed,
    /// Exact receipt already opened this epoch. Its actual (possibly edited) unit was flushed.
    AlreadyInstalled,
    /// Recovery eviction needs acknowledgement or the existing timed-advance action.
    RecoveryPending,
}

// Private deterministic failure seams. Production always uses the same vault write/sync paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InstallWrite {
    Recovery,
    Intents,
    Successor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InstallSync {
    Source,
    Intents,
    Successor,
}

impl ServerStore {
    /// Install the exact held current-owner registry receipt. All steps share one store borrow,
    /// and every reload checks the saved source/accounting. No caller-provided seed is trusted.
    /// A pending recovery warning holds Closing. A failure returns no editable successor; retry
    /// after reconciling uncertain inventories. Keep the SAME receipt/close on retry.
    ///
    /// Included intents may already be retired after a failed final replacement: their full
    /// receipted source is durable until replacement succeeds. Excluded/unaccepted intents stay
    /// pending. This does not replay them, repair faults, discover heads, or send network data.
    /// Conservative per-record reserves and the physical 64-MiB intent cap can refuse even a
    /// shrinking replacement; universal settlement progress at a full quota is not provided yet.
    #[allow(clippy::too_many_arguments)]
    pub fn install_registry_checkpoint(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        receipt_bytes: &[u8],
        close_bytes: &[u8],
        expected_tenure_start: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<(RegistryInstallOutcome, EpochRegistryState), AppError> {
        self.install_registry_checkpoint_with_io(
            server,
            group,
            bucket,
            device,
            receipt_bytes,
            close_bytes,
            expected_tenure_start,
            clock,
            rng,
            budget,
            intents,
            &mut |_, path, bytes| atomic_write(path, bytes),
            &mut |step, path, bytes| match step {
                InstallSync::Intents => super::super::epoch_intents::sync_intent(path, bytes),
                _ => sync_registry(path, bytes),
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn install_registry_checkpoint_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        receipt_bytes: &[u8],
        close_bytes: &[u8],
        expected_tenure_start: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        writer: &mut impl FnMut(InstallWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(InstallSync, &Path, u64) -> Result<(), AppError>,
    ) -> Result<(RegistryInstallOutcome, EpochRegistryState), AppError> {
        // Bounded canonical decoders precede vault work, and authority comes from current MLS
        // state plus independently observed tenure, never a field trusted from the receipt.
        let receipt = Receipt::decode(receipt_bytes).map_err(invalid)?;
        let close = CloseRecord::decode(close_bytes).map_err(invalid)?;
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let verified = receipt
            .verify_current_owner(group, expected_tenure_start)
            .map_err(invalid)?;
        if receipt.document != document
            || receipt.close_record_hash != close.hash()
            || receipt.closed_epoch != close.closed_epoch
        {
            return Err(invalid("installation receipt/close scope mismatch"));
        }
        // The receipt binds the close's unsigned identity, not its signature. Verify the latter
        // even on installed retries, where the original source document has already been retired.
        close
            .verify_for(&document, close.doc_id, group, Some(&verified))
            .map_err(invalid)?;
        let (plan, state) = self.update_registry_with_io(
            server,
            group,
            bucket,
            device,
            false,
            WritePurpose::Settlement,
            rng,
            budget,
            |unit, _| {
                if unit.opened_by(&receipt) {
                    return Ok(None);
                }
                let plan = unit
                    .prepare_settlement(&close, group, expected_tenure_start)
                    .map_err(invalid)?;
                if plan.receipt() != &receipt {
                    return Err(invalid("installation receipt is not the held decision"));
                }
                Ok(Some(plan))
            },
            |_, _| Err(invalid("source preparation must not rewrite the epoch")),
            |path, bytes| sync(InstallSync::Source, path, bytes),
        )?;
        let Some(plan) = plan else {
            // A post-rename retry must never reinstall a seed over newer edits or retire intents
            // against the old closure. The updater just flushed the actual successor unchanged.
            return Ok((RegistryInstallOutcome::AlreadyInstalled, state));
        };

        // Check even when this settlement produces NO snapshot: an older pending warning or
        // corrupt generic/legacy slot must not disappear behind an empty new recovery result.
        let checked = (|| {
            let old = self.load_epoch_recovery(server, &document)?;
            for held in old.retained().chain(old.staged()) {
                RegistryRecovery::from_snapshot(held, &document, bucket).map_err(invalid)?;
            }
            let observed = self.epoch_recovery_inventory_record(server, &document)?;
            let scope = super::super::epoch_recovery::scope_bytes(server, &document)?;
            budget
                .verify_record(
                    &StorageScope::new(server, &document.server_id).map_err(invalid)?,
                    *blake3::hash(&scope).as_bytes(),
                    observed,
                )
                .map_err(invalid)?;
            old.eviction_pending()
        })();
        let pending = match checked {
            Ok(pending) => pending,
            Err(error) => {
                budget.invalidate();
                return Err(error);
            }
        };
        if pending.is_some() {
            return Ok((RegistryInstallOutcome::RecoveryPending, state));
        }
        if let Some(saved) = self.stage_registry_recovery_with_writer(
            server,
            group,
            bucket,
            device,
            close_bytes,
            expected_tenure_start,
            clock,
            rng,
            budget,
            |path, bytes| writer(InstallWrite::Recovery, path, bytes),
        )? {
            if saved.state.eviction_pending()?.is_some() {
                return Ok((RegistryInstallOutcome::RecoveryPending, state));
            }
        }
        self.retire_registry_intents_with_io(
            server,
            &plan,
            rng,
            budget,
            intents,
            |path, bytes| writer(InstallWrite::Intents, path, bytes),
            |path, bytes| sync(InstallSync::Intents, path, bytes),
        )?;
        self.update_registry_with_io(
            server,
            group,
            bucket,
            device,
            false,
            WritePurpose::Settlement,
            rng,
            budget,
            |unit, _| {
                let successor = unit
                    .checkpoint_successor(&plan, group, expected_tenure_start)
                    .map_err(invalid)?;
                *unit = successor;
                Ok(RegistryInstallOutcome::Installed)
            },
            |path, bytes| writer(InstallWrite::Successor, path, bytes),
            |path, bytes| sync(InstallSync::Successor, path, bytes),
        )
    }
}
