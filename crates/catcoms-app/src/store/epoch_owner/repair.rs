//! Owner-side repair barriers on the same accounted record as ordinary decisions: B1 binds a
//! signed repair, its admitted pair and the compatible journal candidate in one write; B3 marks
//! local application; recycling removes the resolved pair once the source is durably terminal.
//! Every transition reloads under the contextual guard, so retained evidence is consumed only
//! when it names this observer and an epoch the caller's durable owner snapshot covers.

use super::fault_record::{BindingKind, Pair, ReportAdmission, ValidatedFaultAdmission};
use super::*;
use catcoms_crypto::DeviceId;
use catcoms_replication::{ReceiptRepair, ReceiptRepairPlan, ReplError};
use catcoms_sync::ArchivedOwnerTenure;

/// Allocate the next sequence from the two durable witnesses consulted by both typed stores.
/// Both inputs must already be filtered to the current authenticated issuer tenure.
pub(in crate::store) fn next_repair_sequence(
    source_high_water: u64,
    journal_high_water: u64,
) -> Result<u64, ReplError> {
    ReceiptRepair::next_sequence_after(source_high_water.max(journal_high_water))
}

/// Evidence that the Studio or Registry source for a repair is durably terminal: its saved
/// `repair_state()` names exactly this repair, owes no replacement, and the flush returned.
/// Minted only by a source transaction after that flush; it is not durable and grants nothing
/// beyond permission to recycle owner evidence for this one repair.
pub(in crate::store) struct TerminalRepairSource {
    repair: [u8; 32],
}

impl TerminalRepairSource {
    pub(in crate::store) fn after_flushed_source(repair: [u8; 32]) -> Self {
        Self { repair }
    }
}

impl EpochOwnerReceiptState {
    /// The journal as persisted. Its roles are historical facts, never fresh authority.
    pub(in crate::store) fn journal(&self) -> &OwnerReceiptJournal {
        &self.journal
    }

    /// The held signed repair, the exact pair its binding decides and that binding's slot, if B1
    /// has persisted one.
    pub(in crate::store) fn held_repair(&self) -> Option<(&ReceiptRepair, &Pair, BindingKind)> {
        let record = self.fault_record.as_ref()?;
        let (repair, pair) = record.repair()?;
        Some((repair, pair, record.binding_kind()?))
    }

    /// Barrier B3 has been recorded for the held repair.
    pub(in crate::store) fn repair_applied(&self) -> bool {
        self.fault_record.as_ref().is_some_and(|r| r.applied())
    }

    /// Retained historical pairs and the reserved pair, for active-pair derivation only.
    pub(in crate::store) fn retained_pairs(&self) -> (&[Pair], Option<&Pair>) {
        match &self.fault_record {
            Some(record) => (record.externals(), record.reserved()),
            None => (&[], None),
        }
    }

    /// A retained admission for exactly this pair, after contextual restore.
    pub(in crate::store) fn retained_admission(
        &self,
        hashes: [[u8; 32]; 2],
        observer: &DeviceId,
        durable_epoch: u64,
    ) -> Result<Option<ValidatedFaultAdmission>, AppError> {
        let Some(record) = &self.fault_record else {
            return Ok(None);
        };
        Ok(record
            .contextual(observer, durable_epoch)?
            .retained_admission(hashes))
    }

    /// The durable proof gate (design 6.6) for one receipt under the current tenure identity.
    pub(in crate::store) fn fault_suppresses_proof(
        &self,
        receipt_hash: [u8; 32],
        current_tenure: [u8; 32],
    ) -> bool {
        self.fault_record
            .as_ref()
            .is_some_and(|r| r.suppresses_proof(receipt_hash, current_tenure))
    }

    /// The overflow hold's fingerprint count, for regressions on its canonical lifecycle.
    #[cfg(test)]
    pub(in crate::store) fn fault_overflow_fingerprints(&self) -> Option<usize> {
        self.fault_record
            .as_ref()
            .and_then(|r| r.overflow_fingerprints())
    }

    /// Whether a retained pair includes this receipt, so it must not be served even as a hint.
    pub(in crate::store) fn fault_retains_member(&self, receipt_hash: [u8; 32]) -> bool {
        self.fault_record
            .as_ref()
            .is_some_and(|r| r.retains_member(receipt_hash))
    }

    /// The exact close a pending decision retires, from this already-validated record.
    pub(in crate::store) fn retiring_close(&self) -> Option<&CloseRecord> {
        self.journal.in_flight().and_then(|r| self.close_for(r))
    }
}

/// Derivation rules 2 to 5 of design 5.2, shared by Studio and Registry: the source's own fault,
/// then a LIVE reserved pair, then the lowest historical pair, then a reserved pair that is no
/// longer live. Rule 1, a held repair, is the caller's: once B1 exists that transaction owns the
/// target. Liveness compares the derived tenure id, never a bare start epoch.
pub(in crate::store) fn decidable_pair(
    record: &EpochOwnerReceiptState,
    source_fault: Option<(&Receipt, &Receipt)>,
    expected_tenure: [u8; 32],
) -> Option<(BindingKind, [Receipt; 2])> {
    if let Some((a, b)) = source_fault {
        let mut pair = [a.clone(), b.clone()];
        pair.sort_by_key(Receipt::hash);
        return Some((BindingKind::SourceBound, pair));
    }
    let (externals, reserved) = record.retained_pairs();
    let live = |pair: &&Pair| pair.receipts()[0].tenure_id == expected_tenure;
    if let Some(pair) = reserved.filter(live) {
        return Some((BindingKind::Reserved, pair.receipts().clone()));
    }
    if let Some(pair) = externals.first() {
        return Some((BindingKind::External(0), pair.receipts().clone()));
    }
    reserved.map(|pair| (BindingKind::Reserved, pair.receipts().clone()))
}

/// Whether installing `selected` must wait on a repair: a held owner decision permits only its
/// own selected receipt into a source that owes it, and a source owing a replacement accepts
/// only that replacement. Shared by Studio and Registry so the two holds cannot drift.
pub(in crate::store) fn repair_defers_install(
    held: Option<[u8; 32]>,
    owed: Option<[u8; 32]>,
    selected: [u8; 32],
) -> bool {
    match (held, owed) {
        (Some(held), Some(owed)) => held != owed || owed != selected,
        // Before B2, or terminal but not yet recycled: the held transaction owns the target.
        (Some(_), None) => true,
        (None, Some(owed)) => owed != selected,
        (None, None) => false,
    }
}

impl ServerStore {
    /// The owner's contextual record, its physical stamp verified against the live budget.
    pub(in crate::store) fn checked_owner_repair_state(
        &self,
        server: u64,
        document: &LogicalDocument,
        observer: &DeviceId,
        durable_epoch: u64,
        budget: &mut EpochStorageBudget,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        let (state, record) = self
            .load_epoch_owner_repair_state(server, document, observer, durable_epoch)
            .inspect_err(|_| budget.invalidate())?;
        let scope = scope_bytes(server, document)?;
        budget
            .verify_record(
                &StorageScope::new(server, &document.server_id).map_err(invalid)?,
                *blake3::hash(&scope).as_bytes(),
                record,
            )
            .map_err(invalid)?;
        Ok(state)
    }

    /// Whether the legacy owner driver may run at all: false while any repair state (a held
    /// transaction, retained evidence or an unpublished reconciliation) owns publication. Checked
    /// before rotation so a hold costs nothing per turn instead of a flush and a refusal.
    pub(crate) fn epoch_owner_is_ordinary(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> Result<bool, AppError> {
        let scope = scope_bytes(server, document)?;
        let (state, _) = self.read_epoch_owner_record(&scope, document)?;
        Ok(state.require_ordinary().is_ok())
    }

    /// Whether the legacy owner driver has a pending decision to rotate. A repair-bearing record
    /// answers false: its transaction or unpublished reconciliation owns publication, and the
    /// legacy driver must neither run against it nor turn that hold into a runtime error.
    pub(crate) fn epoch_owner_rotation_pending(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> Result<bool, AppError> {
        let scope = scope_bytes(server, document)?;
        let (state, _) = self.read_epoch_owner_record(&scope, document)?;
        if state.require_ordinary().is_err() {
            return Ok(false);
        }
        Ok(state.pending().is_some())
    }

    /// The selected receipt of a persisted repair transaction claiming this document, if any.
    /// Structural on purpose: a claim counts whoever admitted it, so an unreadable context fences
    /// rather than releases. Only that decision's own replacement may install while it holds.
    pub(crate) fn epoch_owner_held_selection(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> Result<Option<[u8; 32]>, AppError> {
        let scope = scope_bytes(server, document)?;
        let (state, _) = self.read_epoch_owner_record(&scope, document)?;
        Ok(state
            .held_repair()
            .map(|(repair, _, _)| repair.selected_receipt_hash))
    }

    /// Studio prove path: re-save the effective decision before proving it. Unlike the legacy
    /// prepare, a journal carrying repair provenance is accepted (the core journal enforces the
    /// repaired adjacency and evidence-only holds); any tag-3 evidence still refuses.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn prepare_epoch_owner_publication_with_writer(
        &mut self,
        server: u64,
        receipt: Receipt,
        group: &ServerGroup,
        tenure: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        scope_bytes(server, &receipt.document)?;
        if receipt.owner_public_key.len() != 32 || receipt.encode().len() > MAX_RECEIPT_BYTES {
            return Err(invalid("receipt exceeds its bound"));
        }
        let receipt = Receipt::decode(&receipt.encode()).map_err(invalid)?;
        let document = receipt.document.clone();
        self.update_epoch_owner_journal_guarded(
            server,
            &document,
            OwnerGuard::Publication,
            rng,
            budget,
            |journal| journal.prepare(receipt, group, tenure).map_err(invalid),
            hooks,
        )
    }

    /// Studio completion of an exact proved decision, including a repaired reconciliation whose
    /// publication returns the journal to ordinary once the source is also finalized.
    pub(in crate::store) fn mark_epoch_owner_publication(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        receipt_hash: [u8; 32],
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        self.update_epoch_owner_journal_guarded(
            server,
            document,
            OwnerGuard::Publication,
            rng,
            budget,
            |journal| journal.mark_published(receipt_hash).map_err(invalid),
            &mut WriteHooks::None,
        )
    }

    /// Repair-aware load. Retained attestations must name `observer` and an admission epoch no
    /// later than `durable_epoch`, which the caller reads from its durable owner snapshot in the
    /// same custody visit. The physical record is returned for budget verification.
    pub(in crate::store) fn load_epoch_owner_repair_state(
        &self,
        server: u64,
        document: &LogicalDocument,
        observer: &DeviceId,
        durable_epoch: u64,
    ) -> Result<(EpochOwnerReceiptState, Option<StorageRecord>), AppError> {
        let scope = scope_bytes(server, document)?;
        let (state, size) = self.read_epoch_owner_record(&scope, document)?;
        OwnerGuard::Repair {
            observer,
            durable_epoch,
        }
        .check(&state)?;
        let record = size
            .map(|size| storage_record(server, document, &scope, size))
            .transpose()?;
        Ok((state, record))
    }

    /// Re-save the exact authenticated repair journal before distributing a repair that relies on
    /// it. A visible rename from an earlier B1/B3 attempt is not evidence that its parent-directory
    /// durability barrier completed; this no-op semantic transition repeats the ordinary accounted
    /// sealed-record write and rechecks the live observer/MLS epoch on both sides of it.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn resave_epoch_owner_repair_state_with_writer(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        observer: &DeviceId,
        durable_epoch: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(), AppError> {
        self.write_epoch_owner_state(
            server,
            document,
            OwnerGuard::Repair {
                observer,
                durable_epoch,
            },
            rng,
            budget,
            |_| Ok(()),
            hooks,
        )
        .map(drop)
    }

    /// Barrier B1. Persist the signed repair, its admitted pair and the plan's journal candidate
    /// together, including a journal `NoChange`. The plan must have been computed against exactly
    /// this journal; a stale candidate refuses rather than overwriting newer owner work. An exact
    /// retry of the held repair re-saves; a different repair while one is held is refused.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn prepare_epoch_repair_with_writer(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        plan: &ReceiptRepairPlan,
        kind: BindingKind,
        admission: ValidatedFaultAdmission,
        observer: &DeviceId,
        durable_epoch: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        if plan.repair().document != *document {
            return Err(invalid("repair belongs to another logical document"));
        }
        self.write_epoch_owner_state(
            server,
            document,
            OwnerGuard::Repair {
                observer,
                durable_epoch,
            },
            rng,
            budget,
            |state| {
                if !plan.matches_original_journal(&state.journal) {
                    return Err(invalid("repair plan was computed against another journal"));
                }
                state.fault_record = Some(InertFaultRecord::bind(
                    state.fault_record.take(),
                    kind,
                    admission,
                    plan.repair().clone(),
                )?);
                state.journal = plan.journal().clone();
                // A retired pending decision keeps its close inside the journal's provenance;
                // never leave that close bound as if it were still publishable.
                if state.decision_close.as_ref().is_some_and(|(hash, _)| {
                    state
                        .journal
                        .effective_choice()
                        .is_none_or(|r| r.hash() != *hash)
                }) {
                    state.decision_close = None;
                }
                Ok(())
            },
            hooks,
        )
    }

    /// S-3 provider admission (design 6.5, CORE-005) for a reported pair, under the caller's
    /// durable owner tenure and before the response is decided. Only a pair both of whose
    /// receipts match either the current Observed owner or the one archived Observed witness from
    /// the same durable snapshot is admissible. An exact retained attestation is checked first, so
    /// an unresolved pair survives archive turnover. `Ok(None)` writes nothing (not the owner,
    /// unprovable, foreign, malformed, or unavailable historical capacity, or already answered by
    /// `carried`); `Err` is a failed or uncertain B0 write, which the caller must turn into a
    /// fail-closed answer.
    ///
    /// `carried` is the repair the caller's exact source carries right now (its committed
    /// repair state), if any. See `admit_fault_report_with_writer` for why it matters.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn admit_fault_report(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        group: &ServerGroup,
        device: &catcoms_mls::MlsDevice,
        tenure: u64,
        archived_owner: Option<&ArchivedOwnerTenure>,
        report: &[Receipt; 2],
        carried: Option<&ReceiptRepair>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<Option<ReportAdmission>, AppError> {
        self.admit_fault_report_with_writer(
            server,
            document,
            group,
            device,
            tenure,
            archived_owner,
            report,
            carried,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    /// The production admission path with the owner-record writer exposed only for deterministic
    /// crash-boundary tests. Callers must propagate every writer error: a visible or uncertain B0
    /// replacement is not permission to answer the request that carried the report.
    ///
    /// A report of exactly the pair the caller's source already carries a finished repair for is
    /// answered, not admitted. A faulted peer keeps reporting its frozen pair on every discovery
    /// until it applies the repair, and some of those reports reach the owner after its own
    /// decision has finished and recycled the record. Staging one again would reopen a decided
    /// pair: it suppresses proof of the very receipt the repair selected, so no newcomer could
    /// install the repaired document, and it offers the pair for a second decision. Design
    /// 10.3's two-peer actor run found exactly this.
    ///
    /// The report is declined only under the conditions on which the head service carries that
    /// repair in the same answer: no decision is held, the source carries the repair, and it
    /// verifies under this current tenure. Any other case stages as before, so a pair the owner
    /// can no longer answer stays decidable. `carried` is local authenticated state the caller
    /// read from its exact source; it chooses nothing and widens no authority.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn admit_fault_report_with_writer(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        group: &ServerGroup,
        device: &catcoms_mls::MlsDevice,
        tenure: u64,
        archived_owner: Option<&ArchivedOwnerTenure>,
        report: &[Receipt; 2],
        carried: Option<&ReceiptRepair>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<Option<ReportAdmission>, AppError> {
        let observer = device.device_id();
        if group.designated_committer() != Some(observer)
            || group.member_signature_key(&observer).as_deref()
                != Some(device.public_key_bytes().as_slice())
        {
            return Ok(None);
        }
        let Ok(hashes) =
            ValidatedFaultAdmission::canonical_hashes(document, &report[0], &report[1])
        else {
            return Ok(None);
        };
        // This bounded read is deliberately before fresh authority and before any writer. A
        // retained exact attestation remains sufficient after the single archive has turned over.
        let state =
            self.checked_owner_repair_state(server, document, &observer, group.epoch(), budget)?;
        let answered = state.held_repair().is_none()
            && carried.is_some_and(|repair| {
                repair.receipt_hashes == hashes
                    && repair.verify_current_owner(group, tenure).is_ok()
            });
        if answered {
            return Ok(None);
        }
        let admission = match state.retained_admission(hashes, &observer, group.epoch())? {
            Some(admission) => admission,
            None => match ValidatedFaultAdmission::current(
                document, &report[0], &report[1], group, &observer, tenure,
            ) {
                Ok(admission) => admission,
                Err(_) => {
                    let Some(archived) = archived_owner else {
                        return Ok(None);
                    };
                    let Ok(admission) = ValidatedFaultAdmission::historical(
                        document, &report[0], &report[1], group, &observer, archived,
                    ) else {
                        return Ok(None);
                    };
                    if state.retained_pairs().1.is_some() {
                        return Ok(None);
                    }
                    admission
                }
            },
        };
        let current = catcoms_replication::epoch::tenure_id(
            &group.group_id(),
            &device.public_key_bytes(),
            tenure,
        );
        self.stage_epoch_fault_report_with_writer(
            server,
            document,
            admission,
            current,
            &observer,
            group.epoch(),
            rng,
            budget,
            hooks,
        )
        .map(Some)
    }

    /// Barrier B0: durably stage an admitted current- or archived-tenure report before the response
    /// is decided. An exact retained pair re-saves unchanged; the reserved slot takes a new pair.
    /// Only a competing current-tenure report may enter the bounded live overflow hold; historical
    /// evidence refuses when its complete attestation cannot fit. Nothing here chooses a winner.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn stage_epoch_fault_report_with_writer(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        admission: ValidatedFaultAdmission,
        current_tenure: [u8; 32],
        observer: &DeviceId,
        durable_epoch: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<ReportAdmission, AppError> {
        let mut outcome = ReportAdmission::AlreadyRecorded;
        self.write_epoch_owner_state(
            server,
            document,
            OwnerGuard::Repair {
                observer,
                durable_epoch,
            },
            rng,
            budget,
            |state| {
                let (record, admitted) = InertFaultRecord::admit_report(
                    state.fault_record.take(),
                    admission,
                    current_tenure,
                )?;
                state.fault_record = Some(record);
                outcome = admitted;
                Ok(())
            },
            hooks,
        )?;
        Ok(outcome)
    }

    /// Barrier B3: the local source durably crossed B2 for the held repair. An exact retry still
    /// writes, and a hash naming anything but the held repair refuses without mutation.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn mark_epoch_repair_applied_with_writer(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        repair_hash: [u8; 32],
        observer: &DeviceId,
        durable_epoch: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        self.write_epoch_owner_state(
            server,
            document,
            OwnerGuard::Repair {
                observer,
                durable_epoch,
            },
            rng,
            budget,
            |state| {
                state
                    .fault_record
                    .as_mut()
                    .ok_or_else(|| invalid("no held repair to mark applied"))?
                    .mark_applied(repair_hash)
            },
            hooks,
        )
    }

    /// Terminal recycling. Removes the resolved pair, its attestation and the repair, omitting
    /// tag 3 once nothing else is retained, and acknowledges source finalization to the journal
    /// for this repair's provenance. Idempotent: a retry after cleanup re-saves unchanged bytes.
    /// A journal still holding an unpublished reconciliation keeps it; publication is separate.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn finish_epoch_repair_with_writer(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        terminal: &TerminalRepairSource,
        observer: &DeviceId,
        durable_epoch: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<EpochOwnerReceiptState, AppError> {
        self.write_epoch_owner_state(
            server,
            document,
            OwnerGuard::Repair {
                observer,
                durable_epoch,
            },
            rng,
            budget,
            |state| {
                match state.fault_record.take() {
                    Some(record) if record.repair().is_some() => {
                        state.fault_record = record.recycle(terminal.repair)?;
                    }
                    other => state.fault_record = other,
                }
                if state
                    .journal
                    .retained_repair()
                    .is_some_and(|r| r.hash() == terminal.repair)
                {
                    state
                        .journal
                        .mark_repair_source_finalized(terminal.repair)
                        .map_err(invalid)?;
                }
                Ok(())
            },
            hooks,
        )
    }
}
