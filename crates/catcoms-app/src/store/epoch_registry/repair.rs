//! Registry fault repair (design 5.4), the same transaction as Studio's on the same owner record:
//! issuance at B1, application at B2, B3, recovery-gated replacement and terminal recycling. It
//! matters because a faulted bucket blocks Index and Flipnote discovery outright. Outcomes are
//! read back from the committed source; no intent is retired and nothing is inferred.

use super::super::epoch_owner::{
    decidable_pair, next_repair_sequence, TerminalRepairSource, ValidatedFaultAdmission,
};
use super::*;
use crate::store::{StudioRepairOutcome as RegistryRepairOutcome, StudioRepairRequest};
use catcoms_replication::{
    epoch::tenure_id, ReceiptRepair, RepairDisposition, ReplError, SourceRepairOutcome,
};
use catcoms_rt::Clock;

/// What a bucket can do with a repair an answer offered.
#[derive(Debug)]
pub(crate) enum OfferedRepairEvidence {
    /// This exact repair is already terminal here: nothing to apply, now or later.
    Terminal,
    /// This device does not hold both receipts. That can change once it faults on the pair.
    Unverifiable,
    /// The complete pair, from evidence held plus the offered receipt.
    Pair(Box<[Receipt; 2]>),
}

impl ServerStore {
    /// The checked saved bucket, authenticated against the live budget. A repair never creates
    /// a source, so absence is an error rather than a fresh epoch zero.
    fn checked_registry_unit(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        budget: &mut EpochStorageBudget,
    ) -> Result<RegistryEpoch, AppError> {
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let scope = scope_bytes(server, &document)?;
        let loaded = (|| {
            let bytes = self
                .read_registry_record(&scope)?
                .ok_or_else(|| invalid("a repair never creates a source"))?;
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
            Ok((unit, observed))
        })();
        let (unit, observed) = loaded.inspect_err(|_| budget.invalidate())?;
        budget
            .verify_record(
                &StorageScope::new(server, &document.server_id).map_err(invalid)?,
                *blake3::hash(&scope).as_bytes(),
                Some(observed),
            )
            .map_err(invalid)?;
        Ok(unit)
    }

    /// S-1 for a Registry bucket, followed by Flow A. `tenure` is the authoring start from the
    /// durable owner snapshot in this custody visit; the caller refused Imported and Unknown.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn issue_registry_repair(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        tenure: u64,
        request: StudioRepairRequest,
        raw_seed: Option<&[u8]>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<(ReceiptRepair, RegistryRepairOutcome, EpochRegistryState), AppError> {
        self.issue_registry_repair_with_io(
            server,
            group,
            bucket,
            device,
            tenure,
            request,
            raw_seed,
            clock,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn issue_registry_repair_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        tenure: u64,
        request: StudioRepairRequest,
        raw_seed: Option<&[u8]>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(ReceiptRepair, RegistryRepairOutcome, EpochRegistryState), AppError> {
        current_registry_member(group, device)?;
        if group.designated_committer() != Some(device.device_id()) {
            return Err(invalid("only the current owner may decide a fault"));
        }
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let observer = device.device_id();
        let mut unit = self.checked_registry_unit(server, group, bucket, device, budget)?;
        let owner =
            self.checked_owner_repair_state(server, &document, &observer, group.epoch(), budget)?;
        let (kind, pair, admission, repair) = if let Some((held, held_pair, kind)) =
            owner.held_repair()
        {
            if !request.names(held_pair.hashes()) || held.selected_receipt_hash != request.selected
            {
                return Err(invalid("a different repair is held; resume it first"));
            }
            let admission = owner
                .retained_admission(held_pair.hashes(), &observer, group.epoch())?
                .ok_or_else(|| invalid("held repair lost its admission"))?;
            (kind, held_pair.receipts().clone(), admission, held.clone())
        } else {
            let expected = tenure_id(&group.group_id(), &device.public_key_bytes(), tenure);
            let (kind, pair) = decidable_pair(&owner, unit.fault_evidence(), expected)
                .ok_or_else(|| invalid("no fault is decidable for this bucket"))?;
            if !request.names([pair[0].hash(), pair[1].hash()]) {
                return Err(invalid(
                    "decision names a pair that is not the decidable fault",
                ));
            }
            let admission = match owner.retained_admission(
                [pair[0].hash(), pair[1].hash()],
                &observer,
                group.epoch(),
            )? {
                Some(admission) => admission,
                None if kind == super::super::epoch_owner::BindingKind::SourceBound => {
                    ValidatedFaultAdmission::current(
                        &document, &pair[0], &pair[1], group, &observer, tenure,
                    )?
                }
                None => return Err(invalid("historical owner authority is unavailable")),
            };
            let sequence = next_repair_sequence(
                unit.repair_sequence_for_issuer_tenure(tenure),
                owner.journal().repair_sequence_for_issuer_tenure(tenure),
            )
            .map_err(invalid)?;
            let repair = ReceiptRepair::sign_in_tenure(
                document.clone(),
                pair[0].tenure_id,
                admission.hashes(),
                request.selected,
                sequence,
                tenure,
                device,
            )
            .map_err(invalid)?;
            (kind, pair, admission, repair)
        };
        let plan = unit
            .prepare_receipt_repair(
                &repair,
                &pair[0],
                &pair[1],
                owner.journal(),
                owner.retiring_close(),
                group,
                tenure,
            )
            .map_err(invalid)?;
        // B1: the signed bytes exist for anyone else only after this write returns.
        self.prepare_epoch_repair_with_writer(
            server,
            &document,
            &plan,
            kind,
            admission,
            &observer,
            group.epoch(),
            rng,
            budget,
            hooks,
        )?;
        drop(plan);
        let (outcome, state) = self.apply_registry_repair_with_io(
            server, group, bucket, device, &repair, &pair, tenure, raw_seed, clock, rng, budget,
            hooks,
        )?;
        Ok((repair, outcome, state))
    }

    /// S-2, Flow A for a Registry bucket: the owner resuming its decision or a peer applying a
    /// distributed repair, with identical code and the same committed-state outcome rules.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_registry_repair(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        tenure: u64,
        raw_seed: Option<&[u8]>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<(RegistryRepairOutcome, EpochRegistryState), AppError> {
        self.apply_registry_repair_with_io(
            server,
            group,
            bucket,
            device,
            repair,
            pair,
            tenure,
            raw_seed,
            clock,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_registry_repair_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        tenure: u64,
        raw_seed: Option<&[u8]>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(RegistryRepairOutcome, EpochRegistryState), AppError> {
        current_registry_member(group, device)?;
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        if repair.document != document {
            return Err(invalid("repair belongs to another bucket"));
        }
        repair
            .verify_current_owner(group, tenure)
            .map_err(invalid)?;
        repair.check_evidence(&pair[0], &pair[1]).map_err(invalid)?;
        let observer = device.device_id();
        let is_owner = group.designated_committer() == Some(observer);
        let resolved = self
            .checked_registry_unit(server, group, bucket, device, budget)?
            .repair_state()
            .filter(|s| s.repair == *repair);
        let owner = if is_owner {
            let owner = self.checked_owner_repair_state(
                server,
                &document,
                &observer,
                group.epoch(),
                budget,
            )?;
            match owner.held_repair() {
                Some((held, _, _)) if held.hash() == repair.hash() => {}
                Some(_) => return Err(invalid("another repair owns this target")),
                None if resolved.is_some() => {}
                None => {
                    return Err(invalid(
                        "the owner must persist a decision before applying it",
                    ))
                }
            }
            Some(owner)
        } else {
            // A decision this device persisted in an earlier tenure still owns the bucket.
            if self
                .epoch_owner_held_selection(server, &document)?
                .is_some()
            {
                return Err(invalid(
                    "a decision held from an earlier tenure owns this target",
                ));
            }
            None
        };
        let first = resolved.is_none();
        // B2 through the bucket's single checked writer, or for a retry the flush of unchanged
        // bytes. A hold changes nothing, and the unchanged source is flushed, never rewritten.
        let (applied, mut state) = self.update_registry_with_io(
            server,
            group,
            bucket,
            device,
            false,
            WritePurpose::Settlement,
            rng,
            budget,
            |unit, _| {
                if !first {
                    return Ok(None);
                }
                let outcome = match &owner {
                    Some(owner) => {
                        let plan = unit
                            .prepare_receipt_repair(
                                repair,
                                &pair[0],
                                &pair[1],
                                owner.journal(),
                                owner.retiring_close(),
                                group,
                                tenure,
                            )
                            .map_err(invalid)?;
                        unit.apply_planned_receipt_repair(plan, owner.journal(), group, tenure)
                            .map_err(invalid)?
                    }
                    None => unit
                        .apply_receipt_repair(repair, &pair[0], &pair[1], group, tenure)
                        .map_err(invalid)?,
                };
                Ok(Some(outcome))
            },
            WriteStep::new(WriteTag::Source),
            hooks,
        )?;
        if let Some(SourceRepairOutcome::Held(hold)) = applied {
            return Ok((RegistryRepairOutcome::Held(hold), state));
        }
        let committed = state
            .unit
            .repair_state()
            .filter(|s| s.repair == *repair)
            .ok_or_else(|| invalid("committed source does not carry this repair"))?;
        let mut installed_now = false;
        if committed.install_pending {
            if let Some(owner) = &owner {
                if !owner.repair_applied() && owner.held_repair().is_some() {
                    self.mark_epoch_repair_applied_with_writer(
                        server,
                        &document,
                        repair.hash(),
                        &observer,
                        group.epoch(),
                        rng,
                        budget,
                        hooks,
                    )?;
                }
            }
            let Some(raw_seed) = raw_seed else {
                return Ok((RegistryRepairOutcome::AwaitingSeed, state));
            };
            let plan = match state.unit.prepare_repair_adoption(
                &committed.selected,
                raw_seed,
                group,
                tenure,
            ) {
                Ok(plan) => plan,
                Err(ReplError::EpochBound) => {
                    return Ok((RegistryRepairOutcome::StorageRefused, state))
                }
                Err(error) => return Err(invalid(error)),
            };
            let (adopted, saved) = self.install_registry_adoption_plan_with_io(
                server, group, bucket, device, &plan, tenure, clock, rng, budget, state, hooks,
            )?;
            state = saved;
            match adopted {
                RegistryAdoptionOutcome::Installed => installed_now = true,
                RegistryAdoptionOutcome::RecoveryPending => {
                    return Ok((RegistryRepairOutcome::RecoveryPending, state))
                }
                other => return Err(invalid(format!("unexpected repair install {other:?}"))),
            }
        }
        let outcome = if committed.disposition == RepairDisposition::Screened {
            RegistryRepairOutcome::Screened
        } else if installed_now {
            RegistryRepairOutcome::Installed
        } else if !first && committed.installed {
            RegistryRepairOutcome::AlreadyRepaired
        } else {
            RegistryRepairOutcome::Repaired
        };
        if owner.is_some() {
            let terminal = TerminalRepairSource::after_flushed_source(repair.hash());
            self.finish_epoch_repair_with_writer(
                server,
                &document,
                &terminal,
                &observer,
                group.epoch(),
                rng,
                budget,
                hooks,
            )?;
        }
        Ok((outcome, state))
    }

    /// Flow D pair assembly for a bucket, from evidence this device already holds plus the
    /// receipt the same authenticated answer offered. Unverifiable is never a reason to invent
    /// the missing receipt; it may become verifiable later, unlike a terminal repair.
    pub(crate) fn registry_repair_evidence(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        repair: &ReceiptRepair,
        offered: Option<&Receipt>,
    ) -> Result<OfferedRepairEvidence, AppError> {
        let Some(state) = self.load_registry_epoch(server, group, bucket, device)? else {
            return Ok(OfferedRepairEvidence::Unverifiable);
        };
        let unit = &state.unit;
        if unit
            .repair_state()
            .is_some_and(|s| s.repair == *repair && !s.install_pending)
        {
            return Ok(OfferedRepairEvidence::Terminal);
        }
        let mut held: Vec<Receipt> = Vec::new();
        if let Some((a, b)) = unit.fault_evidence() {
            held.extend([a.clone(), b.clone()]);
        }
        if let Ok(Some(head)) = unit.receipt_head() {
            held.push(head.clone());
        }
        held.extend(unit.opening().cloned());
        held.extend(offered.cloned());
        // After B2 the fault is gone; the bucket's own resolved evidence still holds both.
        if let Some(state) = unit.repair_state() {
            held.extend([state.selected, state.losing]);
        }
        let find = |hash: &[u8; 32]| held.iter().find(|r| r.hash() == *hash).cloned();
        Ok(match repair.receipt_hashes.each_ref().map(find) {
            [Some(a), Some(b)] => OfferedRepairEvidence::Pair(Box::new([a, b])),
            _ => OfferedRepairEvidence::Unverifiable,
        })
    }

    /// S-4 for a bucket, read-only: the decidable pair exactly as issuance would derive it, and
    /// any held or resolved repair. The owner record is consulted only by the current owner.
    pub(crate) fn registry_fault_evidence(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
        tenure: Option<u64>,
    ) -> Result<Option<crate::store::StudioFaultEvidence>, AppError> {
        let Some(state) = self.load_registry_epoch(server, group, bucket, device)? else {
            return Ok(None);
        };
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let owner = match tenure {
            Some(_) if group.designated_committer() == Some(device.device_id()) => Some(
                self.load_epoch_owner_repair_state(
                    server,
                    &document,
                    &device.device_id(),
                    group.epoch(),
                )?
                .0,
            ),
            _ => None,
        };
        let unit = &state.unit;
        let held = owner
            .as_ref()
            .and_then(|o| o.held_repair())
            .map(|(repair, pair, _)| (repair.clone(), pair.receipts().clone()));
        let decidable = match (&held, &owner, tenure) {
            (Some((_, pair)), _, _) => Some(pair.clone()),
            (None, Some(record), Some(tenure)) => {
                let expected = tenure_id(&group.group_id(), &device.public_key_bytes(), tenure);
                decidable_pair(record, unit.fault_evidence(), expected).map(|(_, pair)| pair)
            }
            _ => unit.fault_evidence().map(|(a, b)| {
                let mut pair = [a.clone(), b.clone()];
                pair.sort_by_key(Receipt::hash);
                pair
            }),
        };
        // Further retained pairs only: the decidable one is not also "waiting".
        let retained = owner.as_ref().map_or(0, |record| {
            let (externals, reserved) = record.retained_pairs();
            let hashes = decidable.as_ref().map(|d| [d[0].hash(), d[1].hash()]);
            let decidable_retained = externals
                .iter()
                .chain(reserved)
                .any(|pair| Some(pair.hashes()) == hashes);
            (externals.len() + usize::from(reserved.is_some()))
                .saturating_sub(usize::from(decidable_retained))
        });
        Ok(Some(crate::store::StudioFaultEvidence {
            decidable,
            held: held.map(|(repair, _)| repair),
            resolved: unit.repair_state(),
            source_faulted: unit.fault_evidence().is_some(),
            waiting: retained,
            doc_id: unit.doc_id(),
            epoch: unit.epoch(),
            phase: unit.phase(),
            operations: unit.op_count(),
            opening: unit.opening().map(Receipt::hash),
        }))
    }

    /// The committed repair this bucket still owes a replacement for, with its complete pair.
    pub(crate) fn owed_registry_repair(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
    ) -> Result<Option<(ReceiptRepair, [Receipt; 2])>, AppError> {
        Ok(self
            .load_registry_epoch(server, group, bucket, device)?
            .and_then(|state| state.unit.repair_state())
            .filter(|state| state.install_pending)
            .map(|state| {
                let mut pair = [state.selected, state.losing];
                pair.sort_by_key(Receipt::hash);
                (state.repair, pair)
            }))
    }

    /// Whether a repair hold, not storage, must defer installing `selected` into this bucket.
    /// `owed` is the bucket's own owed replacement, which the caller has already read, so this
    /// costs one small owner-record read rather than another full Registry restore.
    pub(crate) fn registry_install_deferred_by_repair(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        owed: Option<[u8; 32]>,
        selected: &Receipt,
    ) -> Result<bool, AppError> {
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let held = self.epoch_owner_held_selection(server, &document)?;
        Ok(super::super::epoch_owner::repair_defers_install(
            held,
            owed,
            selected.hash(),
        ))
    }

    /// The bucket's frozen fault pair for the W-1 reporter. Read only once the runtime already
    /// knows the bucket is faulted, so ordinary discovery never pays a Registry restore for it.
    pub(crate) fn registry_fault_pair(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
    ) -> Result<Option<[Receipt; 2]>, AppError> {
        Ok(self
            .load_registry_epoch(server, group, bucket, device)?
            .and_then(|state| {
                state.unit.fault_evidence().map(|(a, b)| {
                    let mut pair = [a.clone(), b.clone()];
                    pair.sort_by_key(Receipt::hash);
                    pair
                })
            }))
    }

    /// The owner's held, not yet recycled Registry decision and the pair it binds.
    pub(crate) fn held_registry_repair(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        device: &MlsDevice,
    ) -> Result<Option<(ReceiptRepair, [Receipt; 2])>, AppError> {
        if group.designated_committer() != Some(device.device_id()) {
            return Ok(None);
        }
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let (state, _) = self.load_epoch_owner_repair_state(
            server,
            &document,
            &device.device_id(),
            group.epoch(),
        )?;
        Ok(state
            .held_repair()
            .map(|(repair, pair, _)| (repair.clone(), pair.receipts().clone())))
    }
}

fn current_registry_member(group: &ServerGroup, device: &MlsDevice) -> Result<(), AppError> {
    if group.member_signature_key(&device.device_id()).as_deref()
        != Some(device.public_key_bytes().as_slice())
    {
        return Err(invalid("repair requires current membership"));
    }
    Ok(())
}
