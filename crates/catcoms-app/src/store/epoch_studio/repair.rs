//! Studio fault repair: owner issuance (S-1), application shared by owner and peer (S-2) and a
//! read-only evidence view (S-4). Every path resolves an interrupted Prepared handoff first and
//! writes only through the checked source writer. The outcome is always read back from the
//! committed source, never from a plan. Nothing here retires an intent, disposes an overlay or
//! touches a `DraftArchive`: a replacement preserves the losing version as `Repair` recovery.

use super::super::epoch_owner::{
    decidable_pair, next_repair_sequence, BindingKind, EpochOwnerReceiptState,
    TerminalRepairSource, ValidatedFaultAdmission,
};
use super::*;
use catcoms_replication::studio::StudioTarget;
use catcoms_replication::{
    epoch::tenure_id, ReceiptRepair, RepairDisposition, RepairHold, ReplError, SourceRepairOutcome,
    SourceRepairState,
};
use catcoms_rt::Clock;

/// What one repair step durably achieved. Read from the committed source after its barrier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioRepairOutcome {
    /// This document's own Fault ended and no replacement is required. Terminal.
    Repaired,
    /// Terminal for this pair, but NOT about this document's own blocker: a different Fault, if
    /// any, still stands, or the source was healthy. Never read as "usable again".
    Screened,
    /// The fault ended and the selected checkpoint replaced the losing source. Terminal.
    Installed,
    /// The fault ended durably; the selected checkpoint's seed is still required. Not terminal.
    AwaitingSeed,
    /// This exact repair was already durable and its replacement complete.
    AlreadyRepaired,
    /// A recovery eviction warning holds the replacement. Everything is retained.
    RecoveryPending,
    /// The whole-version recovery snapshot exceeds its bound. Everything is retained.
    StorageRefused,
    /// A valid signed repair that cannot change this source now. Nothing was discarded.
    Held(RepairHold),
}

impl StudioRepairOutcome {
    /// Nothing further is owed for this repair on this source.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Repaired | Self::Screened | Self::Installed | Self::AlreadyRepaired
        )
    }
}

/// The renderer's echo of a decision: both receipt hashes and the chosen one. It carries no
/// receipt bytes, tenure, sequence or repair; the store re-derives the pair under custody and
/// refuses if a stale view names a pair that is no longer the decidable one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StudioRepairRequest {
    pub receipt_a: [u8; 32],
    pub receipt_b: [u8; 32],
    pub selected: [u8; 32],
}

impl StudioRepairRequest {
    pub(in crate::store) fn names(&self, hashes: [[u8; 32]; 2]) -> bool {
        let mut named = [self.receipt_a, self.receipt_b];
        named.sort();
        named == hashes && hashes.contains(&self.selected)
    }
}

/// Read-only evidence for one target. Candidates are the derived decidable pair; nothing here
/// grants authority, and a non-owner sees only its own source.
#[derive(Debug)]
pub struct StudioFaultEvidence {
    /// The pair a decision would currently name, ascending by receipt hash.
    pub decidable: Option<[Receipt; 2]>,
    /// A signed repair persisted at B1 and not yet recycled. While present it owns the target.
    pub held: Option<ReceiptRepair>,
    /// The source's committed repair and its continuation.
    pub resolved: Option<SourceRepairState>,
    pub source_faulted: bool,
    /// Further retained pairs that will become decidable in turn.
    pub waiting: usize,
    pub doc_id: u128,
    pub epoch: u64,
    pub phase: EpochPhase,
    /// Accepted operations a replacement would move into recovery, not a claim they are lost.
    pub operations: usize,
    /// The installed opening, so a candidate can say whether this source descends from it.
    pub opening: Option<[u8; 32]>,
}

impl ServerStore {
    /// S-1, owner issuance followed by Flow A on the same custody-held source. `tenure` is the
    /// authoring start read from the durable owner snapshot in this custody visit; the caller
    /// must already have refused `Imported` and `Unknown`. The repair is signed only for the
    /// derived decidable pair, persisted at B1 with its admission, then applied. An exact retry
    /// of a held decision resumes it; a different decision while one is held refuses.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn issue_studio_repair(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        tenure: u64,
        request: StudioRepairRequest,
        raw_seed: Option<&[u8]>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<(ReceiptRepair, StudioRepairOutcome, EpochStudioState), AppError> {
        self.issue_studio_repair_with_io(
            server,
            group,
            target,
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
    pub(super) fn issue_studio_repair_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        tenure: u64,
        request: StudioRepairRequest,
        raw_seed: Option<&[u8]>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(ReceiptRepair, StudioRepairOutcome, EpochStudioState), AppError> {
        current_member(group, device)?;
        if group.designated_committer() != Some(device.device_id()) {
            return Err(invalid("only the current owner may decide a fault"));
        }
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let observer = device.device_id();
        self.resolve_studio_handoff_with_io(server, group, target, device, rng, budget, hooks)?;
        let source = self.checked_studio_receive_source(server, group, target, device, budget)?;
        if source.observed.is_none() {
            return Err(invalid("a repair never creates a source"));
        }
        let owner = self.checked_owner_repair_state(
            server,
            &document,
            &observer,
            group.epoch(),
            &mut budget.storage,
        )?;
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
            let (kind, pair) = decidable_pair(&owner, source.unit.fault_evidence(), expected)
                .ok_or_else(|| invalid("no fault is decidable for this target"))?;
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
                // Only a directly observed current-tenure source pair can be admitted here.
                // A historical pair needs a retained attestation; its signature never suffices.
                None if kind == BindingKind::SourceBound => ValidatedFaultAdmission::current(
                    &document, &pair[0], &pair[1], group, &observer, tenure,
                )?,
                None => return Err(invalid("historical owner authority is unavailable")),
            };
            let sequence = next_repair_sequence(
                source.unit.repair_sequence_for_issuer_tenure(tenure),
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
        let source::CheckedReceiveSource {
            mut unit,
            observed,
            before,
            version,
        } = source;
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
            &mut budget.storage,
            hooks,
        )?;
        drop(plan);
        let source = source::CheckedReceiveSource {
            unit,
            observed,
            before,
            version,
        };
        let (outcome, state) = self.apply_checked_studio_repair(
            server, group, target, device, &repair, &pair, tenure, raw_seed, clock, rng, budget,
            source, hooks,
        )?;
        Ok((repair, outcome, state))
    }

    /// S-2, Flow A: identical code for the owner resuming its own decision and for a peer that
    /// received a distributed repair. `tenure` is the authoring start observed at this custody
    /// visit; `pair` is the complete conflicting evidence the repair names.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_studio_repair(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        tenure: u64,
        raw_seed: Option<&[u8]>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<(StudioRepairOutcome, EpochStudioState), AppError> {
        self.apply_studio_repair_with_io(
            server,
            group,
            target,
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
    pub(super) fn apply_studio_repair_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        tenure: u64,
        raw_seed: Option<&[u8]>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(StudioRepairOutcome, EpochStudioState), AppError> {
        current_member(group, device)?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        scope_bytes(server, &repair.document)?;
        if repair.document != document {
            return Err(invalid("repair belongs to another target"));
        }
        // Live authority and complete evidence before any source work or IO.
        repair
            .verify_current_owner(group, tenure)
            .map_err(invalid)?;
        repair.check_evidence(&pair[0], &pair[1]).map_err(invalid)?;
        self.resolve_studio_handoff_with_io(server, group, target, device, rng, budget, hooks)?;
        let source = self.checked_studio_receive_source(server, group, target, device, budget)?;
        if source.observed.is_none() {
            return Err(invalid("a repair never creates a source"));
        }
        self.apply_checked_studio_repair(
            server, group, target, device, repair, pair, tenure, raw_seed, clock, rng, budget,
            source, hooks,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_checked_studio_repair(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        tenure: u64,
        raw_seed: Option<&[u8]>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        source: source::CheckedReceiveSource,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(StudioRepairOutcome, EpochStudioState), AppError> {
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let observer = device.device_id();
        let is_owner = group.designated_committer() == Some(observer);
        let source::CheckedReceiveSource {
            mut unit,
            observed,
            before,
            version,
        } = source;
        let resolved = unit.repair_state().filter(|s| s.repair == *repair);
        // The owner applies only what B1 persisted. A different held repair owns the target.
        let owner = if is_owner {
            let owner = self.checked_owner_repair_state(
                server,
                &document,
                &observer,
                group.epoch(),
                &mut budget.storage,
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
            // A decision this device persisted in an earlier tenure still owns the target
            // (CORE-007): it is held, never bypassed by applying someone else's repair.
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
        if first {
            let outcome = match &owner {
                Some(owner) => {
                    let journal = owner.journal();
                    let plan = unit
                        .prepare_receipt_repair(
                            repair,
                            &pair[0],
                            &pair[1],
                            journal,
                            owner.retiring_close(),
                            group,
                            tenure,
                        )
                        .map_err(invalid)?;
                    unit.apply_planned_receipt_repair(plan, journal, group, tenure)
                        .map_err(invalid)?
                }
                None => unit
                    .apply_receipt_repair(repair, &pair[0], &pair[1], group, tenure)
                    .map_err(invalid)?,
            };
            if let SourceRepairOutcome::Held(hold) = outcome {
                // Nothing changed; flush the unchanged source so the reply is honest.
                let state = self.save_studio_source_reusing(
                    server,
                    unit,
                    observed,
                    &before,
                    WritePurpose::Settlement,
                    rng,
                    &mut budget.storage,
                    WriteStep::flush_only(WriteTag::Source, "a held repair changes nothing"),
                    hooks,
                    version,
                )?;
                return Ok((StudioRepairOutcome::Held(hold), state));
            }
        }
        // B2, or for a retry the flush of unchanged bytes: visibility is not durability.
        let mut state = self.save_studio_source_reusing(
            server,
            unit,
            observed,
            &before,
            WritePurpose::Settlement,
            rng,
            &mut budget.storage,
            WriteStep::new(WriteTag::Source),
            hooks,
            version,
        )?;
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
                        &mut budget.storage,
                        hooks,
                    )?;
                }
            }
            let Some(raw_seed) = raw_seed else {
                return Ok((StudioRepairOutcome::AwaitingSeed, state));
            };
            let plan = match state.unit.prepare_repair_adoption(
                &committed.selected,
                raw_seed,
                group,
                tenure,
            ) {
                Ok(plan) => plan,
                Err(ReplError::EpochBound) => {
                    return Ok((StudioRepairOutcome::StorageRefused, state))
                }
                Err(error) => return Err(invalid(error)),
            };
            let (adopted, saved) = self.install_studio_adoption_plan_with_io(
                server, group, target, &plan, tenure, clock, rng, budget, state, observed, hooks,
            )?;
            state = saved;
            match adopted {
                StudioAdoptionOutcome::Installed => installed_now = true,
                StudioAdoptionOutcome::RecoveryPending => {
                    return Ok((StudioRepairOutcome::RecoveryPending, state))
                }
                other => return Err(invalid(format!("unexpected repair install {other:?}"))),
            }
        }
        let outcome = if committed.disposition == RepairDisposition::Screened {
            StudioRepairOutcome::Screened
        } else if installed_now {
            StudioRepairOutcome::Installed
        } else if !first && committed.installed {
            StudioRepairOutcome::AlreadyRepaired
        } else {
            StudioRepairOutcome::Repaired
        };
        // The source's own barrier returned and it owes nothing further for this repair.
        if owner.is_some() {
            let terminal = TerminalRepairSource::after_flushed_source(repair.hash());
            self.finish_epoch_repair_with_writer(
                server,
                &document,
                &terminal,
                &observer,
                group.epoch(),
                rng,
                &mut budget.storage,
                hooks,
            )?;
        }
        Ok((outcome, state))
    }

    /// Whether this exact repair is already applied and owes nothing here, read from the warm
    /// source only. A cold source answers `false`, so the caller does the work rather than
    /// skipping it on a guess.
    pub(crate) fn studio_repair_is_terminal(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        repair: &ReceiptRepair,
    ) -> bool {
        self.warm_studio_unit(server, group, target, device, |unit| {
            unit.repair_state()
                .is_some_and(|s| s.repair == *repair && !s.install_pending)
        })
        .unwrap_or(false)
    }

    /// Assemble the complete pair a distributed repair names from evidence this device already
    /// holds (its fault pair, current head and installed opening) plus the receipt the same
    /// authenticated answer offered. `None` means there is nothing to do: either this repair is
    /// already terminal here, or this device cannot verify it, which is never a reason to invent
    /// the missing receipt.
    ///
    /// Warm sources only: the caller prepares the source through the detached pool first, and a
    /// cold source answers `None` rather than paying a full rebuild on the actor.
    pub(crate) fn studio_repair_evidence(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        repair: &ReceiptRepair,
        offered: Option<&Receipt>,
    ) -> Option<[Receipt; 2]> {
        self.warm_studio_unit(server, group, target, device, |unit| {
            if unit
                .repair_state()
                .is_some_and(|s| s.repair == *repair && !s.install_pending)
            {
                return None;
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
            // After B2 the fault is gone; the source's own resolved evidence still holds both.
            if let Some(state) = unit.repair_state() {
                held.extend([state.selected, state.losing]);
            }
            let find = |hash: &[u8; 32]| held.iter().find(|r| r.hash() == *hash).cloned();
            match repair.receipt_hashes.each_ref().map(find) {
                [Some(a), Some(b)] => Some([a, b]),
                _ => None,
            }
        })
        .flatten()
    }

    /// The owner's persisted, not yet recycled decision for this target and the pair it binds.
    /// Read in context: a record admitted by another observer refuses rather than resuming.
    pub(crate) fn held_studio_repair(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
    ) -> Result<Option<(ReceiptRepair, [Receipt; 2])>, AppError> {
        if group.designated_committer() != Some(device.device_id()) {
            return Ok(None);
        }
        let document = target.document(&group.group_id()).map_err(invalid)?;
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

    /// S-4, read-only. Derives the decidable pair exactly as issuance would, under explicit
    /// view access. The owner record is consulted only by the current owner, in context.
    pub(crate) fn studio_fault_evidence(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        tenure: Option<u64>,
    ) -> Result<Option<StudioFaultEvidence>, AppError> {
        let document = target.document(&group.group_id()).map_err(invalid)?;
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
        self.with_studio_source(server, group, target, device, |state| {
            let unit = &state.unit;
            let empty = EpochOwnerReceiptState::default();
            let record = owner.as_ref().unwrap_or(&empty);
            let held = record.held_repair().map(|(repair, _, _)| repair.clone());
            let decidable = match (record.held_repair(), tenure) {
                (Some((_, pair, _)), _) => Some(pair.receipts().clone()),
                (None, Some(tenure)) => {
                    let expected = tenure_id(&group.group_id(), &device.public_key_bytes(), tenure);
                    decidable_pair(record, unit.fault_evidence(), expected).map(|(_, pair)| pair)
                }
                (None, None) => unit.fault_evidence().map(|(a, b)| sorted(a, b)),
            };
            let (externals, reserved) = record.retained_pairs();
            let retained = externals.len() + usize::from(reserved.is_some());
            let waiting = retained.saturating_sub(usize::from(
                decidable
                    .as_ref()
                    .is_some_and(|d| retained_names(record, [d[0].hash(), d[1].hash()])),
            ));
            Ok(StudioFaultEvidence {
                decidable,
                held,
                resolved: unit.repair_state(),
                source_faulted: unit.fault_evidence().is_some(),
                waiting,
                doc_id: unit.doc_id(),
                epoch: unit.epoch(),
                phase: unit.phase(),
                operations: unit.op_count(),
                opening: unit.opening().map(Receipt::hash),
            })
        })
    }
}

fn retained_names(record: &EpochOwnerReceiptState, hashes: [[u8; 32]; 2]) -> bool {
    let (externals, reserved) = record.retained_pairs();
    externals
        .iter()
        .chain(reserved)
        .any(|pair| pair.hashes() == hashes)
}

fn sorted(a: &Receipt, b: &Receipt) -> [Receipt; 2] {
    let mut pair = [a.clone(), b.clone()];
    pair.sort_by_key(Receipt::hash);
    pair
}
