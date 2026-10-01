//! Owner-side repair barriers on the same accounted record as ordinary decisions: B1 binds a
//! signed repair, its admitted pair and the compatible journal candidate in one write; B3 marks
//! local application; recycling removes the resolved pair once the source is durably terminal.
//! Every transition reloads under the contextual guard, so retained evidence is consumed only
//! when it names this observer and an epoch the caller's durable owner snapshot covers.

use super::fault_record::{BindingKind, Pair, ValidatedFaultAdmission};
use super::*;
use catcoms_crypto::DeviceId;
use catcoms_replication::{ReceiptRepair, ReceiptRepairPlan};

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

    /// Whether a persisted repair transaction claims this document. Structural on purpose: a
    /// claim counts whoever admitted it, so an unreadable context fences rather than releases.
    pub(in crate::store) fn epoch_owner_repair_claimed(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> Result<bool, AppError> {
        let scope = scope_bytes(server, document)?;
        let (state, _) = self.read_epoch_owner_record(&scope, document)?;
        Ok(state.held_repair().is_some())
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
