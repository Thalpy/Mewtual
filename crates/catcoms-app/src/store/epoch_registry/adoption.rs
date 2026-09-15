//! Newcomer installation without a locally held predecessor closure. A scoped owner selection
//! seals the whole source first; typed recovery is durable before any replacement. No intent is
//! retired from seed contents. The Server adapter owns discovery provenance and lifetime checks.

use super::*;
use catcoms_replication::registry::RegistryRecovery;
use catcoms_rt::Clock;

/// Saved local outcome, not a delivery acknowledgement or a current-head editing lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryAdoptionOutcome {
    /// The seed-backed successor is durably selected; pending author intents still need replay.
    Installed,
    /// The actual already-installed unit was flushed, preserving subsequent edits and seals.
    AlreadyInstalled,
    /// The full source and selected receipt are saved Closing; fetch the expected seed next.
    AwaitingSeed,
    /// The full source remains Closing until recovery eviction is acknowledged or advanced.
    RecoveryPending,
    /// Verified conflicting receipt evidence is durably saved in the read-only Fault state.
    Fault,
    /// A same-tenure older head did not replace local high-water evidence.
    Stale,
}

// Deterministic failure seams cover both failure-before-write and uncertain post-rename errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AdoptionWrite {
    Source,
    Recovery,
    Successor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AdoptionSync {
    Source,
    Successor,
}

impl ServerStore {
    /// Trusted local adapter, called only under the Server's fresh scoped discovery borrow.
    /// Receipt bytes alone do not confer currency. On any interrupted attempt the next caller
    /// must reacquire that provenance, including after a warning outlives its network handle.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn adopt_registry_checkpoint(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        receipt: &Receipt,
        raw_seed: Option<&[u8]>,
        tenure: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<(RegistryAdoptionOutcome, EpochRegistryState), AppError> {
        self.adopt_registry_checkpoint_with_io(
            server,
            group,
            bucket,
            device,
            receipt,
            raw_seed,
            tenure,
            clock,
            rng,
            budget,
            &mut |_, path, bytes| atomic_write(path, bytes),
            &mut |_, path, bytes| sync_registry(path, bytes),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn adopt_registry_checkpoint_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        receipt: &Receipt,
        raw_seed: Option<&[u8]>,
        tenure: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        writer: &mut impl FnMut(AdoptionWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(AdoptionSync, &Path, u64) -> Result<(), AppError>,
    ) -> Result<(RegistryAdoptionOutcome, EpochRegistryState), AppError> {
        if group.member_signature_key(&device.device_id()).as_deref()
            != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("checkpoint receiver is not a current member"));
        }
        receipt
            .verify_current_owner(group, tenure)
            .map_err(invalid)?;
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        if receipt.document != document {
            return Err(invalid("checkpoint selection scope mismatch"));
        }
        // Fault must cross its OWN save barrier before seed/recovery checks. Returning Err
        // from this mutation callback would discard the detached fault and lose the evidence.
        // Absence is allowed only after the updater verifies the complete inventory: a lost
        // indexed source must never be replaced by a newly invented empty epoch zero.
        let (outcome, mut state) = self.update_registry_with_io(
            server,
            group,
            bucket,
            device,
            true,
            WritePurpose::Settlement,
            rng,
            budget,
            |unit, _| {
                if unit.opened_by(receipt) {
                    return Ok(RegistryAdoptionOutcome::AlreadyInstalled);
                }
                match unit
                    .begin_checkpoint_adoption(receipt.clone(), group, tenure)
                    .map_err(invalid)?
                {
                    ReceiptIngest::Fault => Ok(RegistryAdoptionOutcome::Fault),
                    ReceiptIngest::Stale => Ok(RegistryAdoptionOutcome::Stale),
                    ReceiptIngest::Advanced | ReceiptIngest::Duplicate => {
                        Ok(RegistryAdoptionOutcome::AwaitingSeed)
                    }
                }
            },
            |path, bytes| writer(AdoptionWrite::Source, path, bytes),
            |path, bytes| sync(AdoptionSync::Source, path, bytes),
        )?;
        if outcome != RegistryAdoptionOutcome::AwaitingSeed {
            return Ok((outcome, state));
        }
        let Some(raw_seed) = raw_seed else {
            return Ok((outcome, state));
        };
        // Recheck the exact typed seed here rather than treating a publicly cloneable generic
        // VerifiedCheckpoint as authority. Any error leaves the full saved source Closing.
        let plan = state
            .unit
            .prepare_checkpoint_adoption(receipt, raw_seed, group, tenure)
            .map_err(invalid)?;
        // Old recovery must be typed and inventory-matched even when THIS source is empty.
        // A pending warning for any prior attempt is not silently discarded by retargeting.
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
        // A warning past its deadline is promoted here rather than held forever for an
        // acknowledgement that may never come.
        if self
            .advance_due_epoch_recovery_with_writer(
                server,
                &document,
                pending,
                clock,
                rng,
                budget,
                |path, bytes| writer(AdoptionWrite::Recovery, path, bytes),
            )?
            .is_some()
        {
            return Ok((RegistryAdoptionOutcome::RecoveryPending, state));
        }
        if let Some(snapshot) = plan.recovery_snapshot() {
            let saved = self.update_epoch_recovery_accounted_with_writer(
                server,
                &document,
                EpochRecoveryAction::Stage(snapshot.clone()),
                clock,
                rng,
                budget,
                |path, bytes| writer(AdoptionWrite::Recovery, path, bytes),
            )?;
            if saved.state.eviction_pending()?.is_some() {
                return Ok((RegistryAdoptionOutcome::RecoveryPending, state));
            }
        }
        // No closure is held locally, so NO intents can be retired by this path. Their authors
        // replay them deliberately into the successor; matching a seed value is not finality.
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
                *unit = unit
                    .adopted_successor(&plan, group, tenure)
                    .map_err(invalid)?;
                Ok(RegistryAdoptionOutcome::Installed)
            },
            |path, bytes| writer(AdoptionWrite::Successor, path, bytes),
            |path, bytes| sync(AdoptionSync::Successor, path, bytes),
        )
    }
}
