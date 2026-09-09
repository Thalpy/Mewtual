//! One Studio source and one exclusive store borrow across owner journaling and settlement.
//! P1's existing owner/recovery/intent writers retain their own accounting and crash barriers.
use super::*;
use catcoms_replication::studio::StudioRecovery;
use catcoms_replication::CloseRecord;
use catcoms_rt::Clock;

/// Local state only. Publication requires the existing current-owner proof handoff, not a
/// successful disk transaction. Pending recovery keeps the complete source sealed on disk.
pub use crate::store::RegistryOwnerRotationOutcome as StudioRotationOutcome;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RotationWrite {
    Journal,
    Source,
    Recovery,
    Intents,
    Successor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RotationSync {
    Source,
    Intents,
    Successor,
}

impl ServerStore {
    /// Local scheduling metadata only: the actual transaction rechecks every source and owner
    /// boundary. Do not repeatedly construct/sign large ineligible closures on idle ticks.
    pub(crate) fn studio_owner_rotation_needed(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        budget: &mut EpochStudioBudget,
    ) -> Result<bool, AppError> {
        if group.designated_committer() != Some(device.device_id()) {
            return Ok(false);
        }
        let status =
            self.with_studio_checkpoint_source(server, group, target, device, budget, |state| {
                Ok((state.phase(), state.unit.close_candidate_ready()))
            })?;
        let Some((phase, ready)) = status else {
            return Ok(false);
        };
        if phase == EpochPhase::Fault {
            return Ok(false);
        };
        if ready || phase == EpochPhase::Closing {
            return Ok(true);
        }
        let document = target.document(&group.group_id()).map_err(invalid)?;
        Ok(self
            .load_epoch_owner_receipts(server, &document)?
            .pending()
            .is_some())
    }
    /// Trusted current durable-owner-snapshot callback only. A UI-supplied tenure is not
    /// authority. A large source must already have passed the existing detached preparation.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rotate_studio_owner(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        tenure: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<(StudioRotationOutcome, EpochStudioState), AppError> {
        self.rotate_studio_owner_with_io(
            server,
            group,
            target,
            device,
            tenure,
            clock,
            rng,
            budget,
            &mut |_, p, b| atomic_write(p, b),
            &mut |step, p, b| match step {
                RotationSync::Intents => super::super::epoch_intents::sync_intent(p, b),
                _ => sync_studio(p, b),
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn rotate_studio_owner_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        tenure: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        writer: &mut impl FnMut(RotationWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(RotationSync, &Path, u64) -> Result<(), AppError>,
    ) -> Result<(StudioRotationOutcome, EpochStudioState), AppError> {
        if group.designated_committer() != Some(device.device_id()) || tenure > group.epoch() {
            return Err(invalid(
                "rotation requires current owner and observed tenure",
            ));
        }
        let source::CheckedReceiveSource {
            unit,
            observed,
            before,
            version,
        } = self.checked_studio_receive_source(server, group, target, device, budget)?;
        if observed.is_none() {
            return Err(invalid("rotation source missing"));
        }
        // Flush the actual unchanged source before an irrevocable decision can escape. Keep
        // its actual physical version: normalized restart bytes are not an inventory stamp.
        let mut state = self.save_studio_source_reusing(
            server,
            unit,
            observed,
            &before,
            WritePurpose::Settlement,
            rng,
            &mut budget.storage,
            |p, b| writer(RotationWrite::Source, p, b),
            |p, b| sync(RotationSync::Source, p, b),
            version,
        )?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let checked = (|| {
            let journal = self.load_epoch_owner_receipts(server, &document)?;
            let observed = self.epoch_owner_receipt_inventory_record(server, &document)?;
            let scope = super::super::epoch_owner::scope_bytes(server, &document)?;
            budget
                .storage
                .verify_record(
                    &StorageScope::new(server, &document.server_id).map_err(invalid)?,
                    *blake3::hash(&scope).as_bytes(),
                    observed,
                )
                .map_err(invalid)?;
            Ok::<_, AppError>(journal)
        })();
        let journal = checked.inspect_err(|_| budget.storage.invalidate())?;
        if state.phase() == EpochPhase::Fault {
            return Ok((StudioRotationOutcome::Fault, state));
        }
        let previous = journal.pending().or_else(|| journal.published());
        let resume = previous.filter(|r| {
            r.tenure_start_group_epoch == tenure
                && (journal.pending().is_some() || !state.unit.opened_by(r))
        });
        let decision = if let Some(receipt) = resume {
            let Some(close) = journal.close_for(receipt) else {
                return Ok((StudioRotationOutcome::DecisionNeedsClose, state));
            };
            state
                .unit
                .resume_owner_decision(group, device, tenure, receipt, close)
                .map_err(invalid)?
        } else {
            state
                .unit
                .new_owner_decision(group, device, tenure, previous)
                .map_err(invalid)?
        };
        let saved = self.prepare_studio_owner_decision_with_writer(
            server,
            &decision,
            group,
            tenure,
            rng,
            &mut budget.storage,
            |p, b| writer(RotationWrite::Journal, p, b),
        )?;
        let pending = saved.pending().is_some();
        // The exact decision is durable before sealing. Installed retries must preserve newer
        // content and must not retire any newer successor intents under the predecessor close.
        if state.unit.opened_by(decision.receipt()) {
            return Ok((
                StudioRotationOutcome::AlreadyInstalled {
                    publication_pending: pending,
                },
                state,
            ));
        }
        let record = state
            .source
            .as_ref()
            .map(source::SourceVersion::record)
            .or(observed);
        let before = Zeroizing::new(state.unit.snapshot().map_err(invalid)?);
        let outcome = state
            .unit
            .seal(decision.receipt().clone(), group, tenure)
            .map_err(invalid)?;
        state = self.save_studio_source(
            server,
            state.unit,
            record,
            &before,
            WritePurpose::Settlement,
            rng,
            &mut budget.storage,
            |p, b| writer(RotationWrite::Source, p, b),
            |p, b| sync(RotationSync::Source, p, b),
        )?;
        if outcome == ReceiptIngest::Fault {
            return Ok((StudioRotationOutcome::Fault, state));
        }
        self.settle_studio_source(
            server,
            group,
            target,
            record,
            tenure,
            decision.close(),
            pending,
            state,
            clock,
            rng,
            budget,
            writer,
            sync,
        )
    }

    // Caller retains sole source/store custody. The checkpoint plan verifies receipt, source
    // version and closure; no caller-supplied projection or seed is trusted here.
    #[allow(clippy::too_many_arguments)]
    fn settle_studio_source(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        observed: Option<StorageRecord>,
        tenure: u64,
        close: &CloseRecord,
        pending: bool,
        mut state: EpochStudioState,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        writer: &mut impl FnMut(RotationWrite, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(RotationSync, &Path, u64) -> Result<(), AppError>,
    ) -> Result<(StudioRotationOutcome, EpochStudioState), AppError> {
        let plan = state
            .unit
            .prepare_settlement(close, group, tenure)
            .map_err(invalid)?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        // Even no new snapshot cannot bypass an older pending warning or corrupt typed state.
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
        if checked
            .inspect_err(|_| budget.storage.invalidate())?
            .is_some()
        {
            return Ok((StudioRotationOutcome::RecoveryPending, state));
        }
        if let Some(snapshot) = plan.recovery_snapshot() {
            let saved = self.update_epoch_recovery_accounted_with_writer(
                server,
                &document,
                EpochRecoveryAction::Stage(snapshot.clone()),
                clock,
                rng,
                &mut budget.storage,
                |p, b| writer(RotationWrite::Recovery, p, b),
            )?;
            if saved.state.eviction_pending()?.is_some() {
                return Ok((StudioRotationOutcome::RecoveryPending, state));
            }
        }
        self.retire_studio_intents_with_io(
            server,
            &plan,
            rng,
            &mut budget.storage,
            &mut budget.intents,
            |p, b| writer(RotationWrite::Intents, p, b),
            |p, b| sync(RotationSync::Intents, p, b),
        )?;
        let record = state
            .source
            .as_ref()
            .map(source::SourceVersion::record)
            .or(observed);
        let before = Zeroizing::new(state.unit.snapshot().map_err(invalid)?);
        let successor = state
            .unit
            .checkpoint_successor(&plan, group, tenure)
            .map_err(invalid)?;
        let saved = self.save_studio_source(
            server,
            successor,
            record,
            &before,
            WritePurpose::Settlement,
            rng,
            &mut budget.storage,
            |p, b| writer(RotationWrite::Successor, p, b),
            |p, b| sync(RotationSync::Successor, p, b),
        )?;
        Ok((
            StudioRotationOutcome::Installed {
                publication_pending: pending,
            },
            saved,
        ))
    }
}
