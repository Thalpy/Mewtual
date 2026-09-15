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
    /// Locally paced Studio maintenance composes the existing Registry rotation and discovery
    /// barriers. No query from another peer is necessary to finish an installed decision.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn maintain_registry_owner(
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
    ) -> Result<Option<(RegistryOwnerRotationOutcome, EpochRegistryState)>, AppError> {
        let Some(state) = self.load_registry_epoch(server, group, bucket, device)? else {
            return Ok(None);
        };
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        if state.phase() == EpochPhase::Fault
            || (state.phase() == EpochPhase::Open
                && !state.unit.close_candidate_ready()
                && self
                    .load_epoch_owner_receipts(server, &document)?
                    .pending()
                    .is_none())
        {
            return Ok(None);
        }
        drop(state);
        let (mut outcome, mut state) = self.rotate_registry_owner(
            server, group, bucket, device, tenure, clock, rng, budget, intents,
        )?;
        if matches!(
            outcome,
            RegistryOwnerRotationOutcome::Installed { .. }
                | RegistryOwnerRotationOutcome::AlreadyInstalled { .. }
        ) {
            let receipt = state
                .unit
                .receipt_head()
                .map_err(invalid)?
                .cloned()
                .ok_or_else(|| invalid("installed Registry receipt missing"))?;
            let id = state.doc_id();
            if state.phase() != EpochPhase::Open
                || !state.unit.opened_by(&receipt)
                || state
                    .unit
                    .checkpoint_bytes_by_hash(id, receipt.seed_change_hash)
                    .map_err(invalid)?
                    .is_none()
            {
                return Err(invalid("Registry head is not an installed checkpoint"));
            }
            // Recheck actual saved source/journal and flush both before freeing the decision
            // slot. Completion is availability, never a claim of transport or peer delivery.
            let selection = self.prepare_registry_head(
                server,
                group,
                bucket,
                device,
                Some(tenure),
                rng,
                budget,
            )?;
            if !selection.prove || selection.receipt.as_ref() != Some(&receipt) {
                return Err(invalid("installed Registry source and journal disagree"));
            }
            self.mark_epoch_owner_receipt_published(
                server,
                &document,
                receipt.hash(),
                rng,
                budget,
            )?;
            outcome = match outcome {
                RegistryOwnerRotationOutcome::Installed { .. } => {
                    RegistryOwnerRotationOutcome::Installed {
                        publication_pending: false,
                    }
                }
                _ => RegistryOwnerRotationOutcome::AlreadyInstalled {
                    publication_pending: false,
                },
            };
        }
        self.remember_installed_registry(server, &group.group_id(), bucket, &mut state)?;
        Ok(Some((outcome, state)))
    }
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
        let (decision, adoption_seed) = if state.unit.owner_rotation_needs_adoption(tenure) {
            if resume.is_some_and(|r| journal.close_for(r).is_none()) {
                return Ok((RegistryOwnerRotationOutcome::DecisionNeedsClose, state));
            }
            let (decision, seed) = state
                .unit
                .frozen_owner_decision(
                    group,
                    device,
                    tenure,
                    previous,
                    previous.and_then(|r| journal.close_for(r)),
                )
                .map_err(invalid)?;
            (decision, Some(seed))
        } else if let Some(receipt) = resume {
            let Some(close) = journal.close_for(receipt) else {
                return Ok((RegistryOwnerRotationOutcome::DecisionNeedsClose, state));
            };
            (
                state
                    .unit
                    .resume_owner_decision(group, device, tenure, receipt, close)
                    .map_err(invalid)?,
                None,
            )
        } else {
            (
                state
                    .unit
                    .new_owner_decision(group, device, tenure, previous)
                    .map_err(invalid)?,
                None,
            )
        };
        let saved =
            self.prepare_registry_owner_decision(server, &decision, group, tenure, rng, budget)?;
        let publication_pending = saved.pending().is_some();
        after(OwnerRotationStep::JournalSaved)?;
        if let Some(seed) = adoption_seed {
            // Reuse Registry's existing two durable adoption barriers. The first call saves
            // the new selection while keeping the complete frozen source; the second uses
            // its typed whole-version recovery before replacing it. No intent is retired
            // merely because this owner selected/recomputed a checkpoint.
            drop(state);
            let (outcome, state) = self.adopt_registry_checkpoint(
                server,
                group,
                bucket,
                device,
                decision.receipt(),
                None,
                tenure,
                clock,
                rng,
                budget,
            )?;
            after(OwnerRotationStep::SourceSealed)?;
            if outcome == RegistryAdoptionOutcome::Fault {
                return Ok((RegistryOwnerRotationOutcome::Fault, state));
            }
            if outcome != RegistryAdoptionOutcome::AwaitingSeed {
                return Err(invalid("unexpected frozen Registry seal outcome"));
            }
            drop(state);
            let (outcome, state) = self.adopt_registry_checkpoint(
                server,
                group,
                bucket,
                device,
                decision.receipt(),
                Some(seed.bytes()),
                tenure,
                clock,
                rng,
                budget,
            )?;
            return match outcome {
                RegistryAdoptionOutcome::Installed => {
                    after(OwnerRotationStep::SuccessorInstalled)?;
                    Ok((
                        RegistryOwnerRotationOutcome::Installed {
                            publication_pending,
                        },
                        state,
                    ))
                }
                RegistryAdoptionOutcome::RecoveryPending => {
                    Ok((RegistryOwnerRotationOutcome::RecoveryPending, state))
                }
                _ => Err(invalid("unexpected frozen Registry adoption outcome")),
            };
        }
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
