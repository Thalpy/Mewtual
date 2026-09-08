//! Explicit owner rotation: checked source -> irrevocable close/receipt journal -> seal ->
//! existing recovery-first installation. No publisher or autonomous scheduler is implied.
use super::*;
use catcoms_rt::Clock;

/// Local durable progress, not a delivery acknowledgement. A pending publication prevents the
/// next decision until a separately checked publication handoff is durably recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryOwnerRotationOutcome {
    Installed {
        publication_pending: bool,
    },
    AlreadyInstalled {
        publication_pending: bool,
    },
    RecoveryPending,
    Fault,
    /// Legacy receipt-only state cannot reproduce an irrevocable close from newer live heads.
    DecisionNeedsClose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OwnerRotationStep {
    JournalSaved,
    SourceSealed,
    SuccessorInstalled,
}

impl ServerStore {
    /// Trusted entry from the Server's current durable-owner-snapshot callback only.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rotate_registry_owner(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        tenure: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<(RegistryOwnerRotationOutcome, EpochRegistryState), AppError> {
        self.rotate_registry_owner_with_hook(
            server,
            group,
            bucket,
            device,
            tenure,
            clock,
            rng,
            budget,
            intents,
            |_| Ok(()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn rotate_registry_owner_with_hook(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        tenure: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        mut after: impl FnMut(OwnerRotationStep) -> Result<(), AppError>,
    ) -> Result<(RegistryOwnerRotationOutcome, EpochRegistryState), AppError> {
        if group.designated_committer() != Some(device.device_id())
            || group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
            || tenure > group.epoch()
        {
            return Err(invalid(
                "rotation requires the current full owner and known tenure",
            ));
        }
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        // No creating empty sources. The exact full source is inventory-checked and flushed
        // before any decision can escape; a lost indexed file must not become epoch zero.
        let (_, mut state) = self.update_registry_with_io(
            server,
            group,
            bucket,
            device,
            false,
            WritePurpose::Settlement,
            rng,
            budget,
            |_, _| Ok(()),
            |_, _| Err(invalid("rotation source preparation cannot rewrite")),
            sync_registry,
        )?;
        let checked = (|| {
            let journal = self.load_epoch_owner_receipts(server, &document)?;
            let observed = self.epoch_owner_receipt_inventory_record(server, &document)?;
            let scope = super::super::epoch_owner::scope_bytes(server, &document)?;
            budget
                .verify_record(
                    &StorageScope::new(server, &document.server_id).map_err(invalid)?,
                    *blake3::hash(&scope).as_bytes(),
                    observed,
                )
                .map_err(invalid)?;
            Ok::<_, AppError>(journal)
        })();
        let journal = match checked {
            Ok(journal) => journal,
            Err(error) => {
                budget.invalidate();
                return Err(error);
            }
        };
        if state.phase() == EpochPhase::Fault {
            return Ok((RegistryOwnerRotationOutcome::Fault, state));
        }
        let previous = journal.pending().or_else(|| journal.published());
        // A pending decision always resumes, even after installation; a published decision also
        // resumes if its source is not installed yet. Never replace either with newer live heads.
        let resume = previous.filter(|r| {
            r.tenure_start_group_epoch == tenure
                && (journal.pending().is_some() || !state.unit.opened_by(r))
        });
        let decision = if let Some(receipt) = resume {
            let Some(close) = journal.close_for(receipt) else {
                return Ok((RegistryOwnerRotationOutcome::DecisionNeedsClose, state));
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
        let saved =
            self.prepare_registry_owner_decision(server, &decision, group, tenure, rng, budget)?;
        let publication_pending = saved.pending().is_some();
        after(OwnerRotationStep::JournalSaved)?;
        let (admission, sealed) = self.seal_registry_epoch(
            server,
            group,
            bucket,
            device,
            decision.receipt().clone(),
            tenure,
            rng,
            budget,
        )?;
        after(OwnerRotationStep::SourceSealed)?;
        if admission == ReceiptIngest::Fault {
            return Ok((RegistryOwnerRotationOutcome::Fault, sealed));
        }
        let (outcome, installed) = self.install_registry_checkpoint(
            server,
            group,
            bucket,
            device,
            &decision.receipt().encode(),
            &decision.close().encode(),
            tenure,
            clock,
            rng,
            budget,
            intents,
        )?;
        let outcome = match outcome {
            RegistryInstallOutcome::Installed => RegistryOwnerRotationOutcome::Installed {
                publication_pending,
            },
            RegistryInstallOutcome::AlreadyInstalled => {
                RegistryOwnerRotationOutcome::AlreadyInstalled {
                    publication_pending,
                }
            }
            RegistryInstallOutcome::RecoveryPending => {
                RegistryOwnerRotationOutcome::RecoveryPending
            }
        };
        if !matches!(outcome, RegistryOwnerRotationOutcome::RecoveryPending) {
            after(OwnerRotationStep::SuccessorInstalled)?;
        }
        Ok((outcome, installed))
    }
}
