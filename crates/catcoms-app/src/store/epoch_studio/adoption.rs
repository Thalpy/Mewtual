//! Checkpoint joining reuses Studio's source ownership/accounting and P1's recovery journal.
//! No closure is locally held, so replacing a source never retires any author's durable intent.
use super::*;
use catcoms_replication::studio::StudioRecovery;
use catcoms_rt::Clock;

/// The same persistence outcomes as Registry adoption; not a second state machine.
pub use crate::store::RegistryAdoptionOutcome as StudioAdoptionOutcome;

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
    /// Trusted-local only: the app must hold a fresh, privately minted discovery selection as
    /// well as current native/vault custody. A raw receipt passed by the UI is not provenance.
    /// Large sources require the existing detached preparation; this transaction never starts
    /// an unbounded cold history rebuild or awaits the network while holding the source.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn adopt_studio_checkpoint(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        receipt: &Receipt,
        raw_seed: Option<&[u8]>,
        tenure: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<(StudioAdoptionOutcome, EpochStudioState), AppError> {
        self.adopt_studio_checkpoint_with_io(
            server,
            group,
            target,
            device,
            receipt,
            raw_seed,
            tenure,
            clock,
            rng,
            budget,
            &mut |_, path, bytes| atomic_write(path, bytes),
            &mut |_, path, bytes| sync_studio(path, bytes),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn adopt_studio_checkpoint_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        receipt: &Receipt,
        raw_seed: Option<&[u8]>,
        tenure: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        writer: &mut impl FnMut(AdoptionWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(AdoptionSync, &Path, u64) -> Result<(), AppError>,
    ) -> Result<(StudioAdoptionOutcome, EpochStudioState), AppError> {
        current_member(group, device)?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        if receipt.document != document {
            return Err(invalid("checkpoint selection scope mismatch"));
        }
        receipt
            .verify_current_owner(group, tenure)
            .map_err(invalid)?;
        // Shares the exact source transfer and fresh five-family accounting used for pages.
        // Actual absence is legal only after inventory agrees, never from a remote pointer.
        let source::CheckedReceiveSource {
            mut unit,
            observed,
            before,
            version,
        } = self.checked_studio_receive_source(server, group, target, device, budget)?;
        let outcome = if unit.opened_by(receipt) {
            StudioAdoptionOutcome::AlreadyInstalled
        } else {
            match unit
                .begin_checkpoint_adoption(receipt.clone(), group, tenure)
                .map_err(invalid)?
            {
                ReceiptIngest::Fault => StudioAdoptionOutcome::Fault,
                ReceiptIngest::Stale => StudioAdoptionOutcome::Stale,
                ReceiptIngest::Advanced | ReceiptIngest::Duplicate => {
                    StudioAdoptionOutcome::AwaitingSeed
                }
            }
        };
        // Fault and Closing cross their OWN durable barrier even if the seed is absent/bad.
        // Converting Fault to an error here would throw away the evidence with the moved unit.
        let state = self.save_studio_source_reusing(
            server,
            unit,
            observed,
            &before,
            WritePurpose::Settlement,
            rng,
            &mut budget.storage,
            |path, bytes| writer(AdoptionWrite::Source, path, bytes),
            |path, bytes| sync(AdoptionSync::Source, path, bytes),
            version,
        )?;
        if outcome != StudioAdoptionOutcome::AwaitingSeed {
            return Ok((outcome, state));
        }
        let Some(raw_seed) = raw_seed else {
            return Ok((outcome, state));
        };
        self.finish_studio_checkpoint_adoption_with_io(
            server, group, target, receipt, raw_seed, tenure, clock, rng, budget, state, observed,
            writer, sync,
        )
    }

    // Joining and a journaled owner takeover share this exact recovery-first install half.
    // The source is already saved Closing under this exclusive custody; don't cold-load a
    // second mutable graph to finish the same transaction.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_studio_checkpoint_adoption_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        receipt: &Receipt,
        raw_seed: &[u8],
        tenure: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        mut state: EpochStudioState,
        observed: Option<StorageRecord>,
        writer: &mut impl FnMut(AdoptionWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(AdoptionSync, &Path, u64) -> Result<(), AppError>,
    ) -> Result<(StudioAdoptionOutcome, EpochStudioState), AppError> {
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let plan = state
            .unit
            .prepare_checkpoint_adoption(receipt, raw_seed, group, tenure)
            .map_err(invalid)?;
        // Even an empty source cannot discard an older staged warning or bypass verification
        // of retained recovery. Typed scope includes the full channel as well as the object key.
        let checked = (|| {
            let old = self.load_epoch_recovery(server, &document)?;
            for held in old.retained().chain(old.staged()) {
                StudioRecovery::from_snapshot(held, &document, target.channel())
                    .map_err(invalid)?;
            }
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
            old.eviction_pending()
        })();
        let pending = checked.inspect_err(|_| budget.storage.invalidate())?;
        // A warning past its deadline is promoted here rather than held forever for an
        // acknowledgement that may never come.
        if self
            .advance_due_epoch_recovery_with_writer(
                server,
                &document,
                pending,
                clock,
                rng,
                &mut budget.storage,
                |path, bytes| writer(AdoptionWrite::Recovery, path, bytes),
            )?
            .is_some()
        {
            return Ok((StudioAdoptionOutcome::RecoveryPending, state));
        }
        if let Some(snapshot) = plan.recovery_snapshot() {
            let saved = self.update_epoch_recovery_accounted_with_writer(
                server,
                &document,
                EpochRecoveryAction::Stage(snapshot.clone()),
                clock,
                rng,
                &mut budget.storage,
                |path, bytes| writer(AdoptionWrite::Recovery, path, bytes),
            )?;
            if saved.state.eviction_pending()?.is_some() {
                return Ok((StudioAdoptionOutcome::RecoveryPending, state));
            }
        }
        // The source never left this exclusive transaction; don't restore/replay it a second
        // time. A cold unchanged flush can have no warm version, so retain its actual observed
        // physical record rather than using a normalized snapshot's size as accounting evidence.
        let current_record = state
            .source
            .as_ref()
            .map(source::SourceVersion::record)
            .or(observed);
        let before = Zeroizing::new(state.unit.snapshot().map_err(invalid)?);
        let successor = state
            .unit
            .adopted_successor(&plan, group, tenure)
            .map_err(invalid)?;
        let saved = self.save_studio_source(
            server,
            successor,
            current_record,
            &before,
            WritePurpose::Settlement,
            rng,
            &mut budget.storage,
            |path, bytes| writer(AdoptionWrite::Successor, path, bytes),
            |path, bytes| sync(AdoptionSync::Successor, path, bytes),
        )?;
        Ok((StudioAdoptionOutcome::Installed, saved))
    }
}
