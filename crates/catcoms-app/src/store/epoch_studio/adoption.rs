//! Checkpoint joining reuses Studio's source ownership/accounting and P1's recovery journal.
//! No closure is locally held, so replacing a source never retires any author's durable intent.
use super::*;
use catcoms_replication::studio::{StudioAdoptionPlan, StudioRecovery};
use catcoms_rt::Clock;

/// The same persistence outcomes as Registry adoption; not a second state machine.
pub use crate::store::RegistryAdoptionOutcome as StudioAdoptionOutcome;

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
            &mut WriteHooks::None,
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
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(StudioAdoptionOutcome, EpochStudioState), AppError> {
        current_member(group, device)?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        if receipt.document != document {
            return Err(invalid("checkpoint selection scope mismatch"));
        }
        receipt
            .verify_current_owner(group, tenure)
            .map_err(invalid)?;
        // A persisted owner repair owns this target from B1 until it is terminal and recycled;
        // ordinary discovery must not install into the source it decided about (AG3-DES-034).
        // The one adoption it permits is its own replacement, checked against the source below.
        let held = self.epoch_owner_held_selection(server, &document)?;
        self.resolve_studio_handoff(server, group, target, device, rng, budget)?;
        // Shares the exact source transfer and fresh five-family accounting used for pages.
        // Actual absence is legal only after inventory agrees, never from a remote pointer.
        let source::CheckedReceiveSource {
            mut unit,
            observed,
            before,
            version,
        } = self.checked_studio_receive_source(server, group, target, device, budget)?;
        if held.is_some_and(|selected| {
            selected != receipt.hash()
                || !unit
                    .repair_state()
                    .is_some_and(|state| state.install_pending && state.selected == *receipt)
        }) {
            return Err(invalid("a held repair owns this target"));
        }
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
            WriteStep::new(WriteTag::Source),
            hooks,
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
            hooks,
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
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(StudioAdoptionOutcome, EpochStudioState), AppError> {
        // A committed repair that still owes its replacement continues here with the Repair
        // reason; the core accepts only that repair's exact selected receipt. Ordinary adoption
        // keeps Rewound and the core refuses it for a repair-pending source.
        let plan = if state.unit.repair_install_pending() {
            state
                .unit
                .prepare_repair_adoption(receipt, raw_seed, group, tenure)
        } else {
            state
                .unit
                .prepare_checkpoint_adoption(receipt, raw_seed, group, tenure)
        }
        .map_err(invalid)?;
        self.install_studio_adoption_plan_with_io(
            server, group, target, &plan, tenure, clock, rng, budget, state, observed, hooks,
        )
    }

    /// The recovery-first install half shared by joining, owner takeover and repair replacement.
    /// The successor write consumes a [`CheckedRepairRecovery`] that only the recovery stage
    /// below can mint, so no reordering of this function can replace a source before its whole
    /// version is durably readable as recovery.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn install_studio_adoption_plan_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        plan: &StudioAdoptionPlan,
        tenure: u64,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        mut state: EpochStudioState,
        observed: Option<StorageRecord>,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(StudioAdoptionOutcome, EpochStudioState), AppError> {
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let before = Zeroizing::new(state.unit.snapshot().map_err(invalid)?);
        // The durable predecessor the plan's recovery is about to preserve, read before staging.
        let predecessor = self.durable_studio_digest(server, &document)?;
        let Some(recovery) = self.stage_studio_adoption_recovery(
            server,
            &document,
            target,
            plan,
            predecessor,
            clock,
            rng,
            budget,
            hooks,
        )?
        else {
            return Ok((StudioAdoptionOutcome::RecoveryPending, state));
        };
        // The source never left this exclusive transaction; don't restore/replay it a second
        // time. A cold unchanged flush can have no warm version, so retain its actual observed
        // physical record rather than using a normalized snapshot's size as accounting evidence.
        let current_record = state
            .source
            .as_ref()
            .map(source::SourceVersion::record)
            .or(observed);
        let successor = state
            .unit
            .adopted_successor(plan, group, tenure)
            .map_err(invalid)?;
        // Against the durable predecessor as it is NOW: a source that changed on disk across the
        // recovery barrier is not the version that recovery preserved.
        recovery.check(server, self.durable_studio_digest(server, &document)?, plan)?;
        let saved = self.save_studio_source(
            server,
            successor,
            current_record,
            &before,
            WritePurpose::Settlement,
            rng,
            &mut budget.storage,
            WriteStep::new(WriteTag::Successor),
            hooks,
        )?;
        Ok((StudioAdoptionOutcome::Installed, saved))
    }

    /// B4/B5: validate every retained slot as typed, verify the recovery inventory record,
    /// promote a due eviction, then stage the plan's whole-version snapshot. `None` means a
    /// warning holds the replacement; everything stays retained.
    #[allow(clippy::too_many_arguments)]
    fn stage_studio_adoption_recovery(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        target: StudioTarget,
        plan: &StudioAdoptionPlan,
        predecessor: [u8; 32],
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<Option<CheckedRepairRecovery>, AppError> {
        // Even an empty source cannot discard an older staged warning or bypass verification
        // of retained recovery. Typed scope includes the full channel as well as the object key.
        let checked = (|| {
            let old = self.load_epoch_recovery(server, document)?;
            for held in old.retained().chain(old.staged()) {
                StudioRecovery::from_snapshot(held, document, target.channel()).map_err(invalid)?;
            }
            let observed = self.epoch_recovery_inventory_record(server, document)?;
            let scope = super::super::epoch_recovery::scope_bytes(server, document)?;
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
                document,
                pending,
                clock,
                rng,
                &mut budget.storage,
                hooks,
            )?
            .is_some()
        {
            return Ok(None);
        }
        if let Some(snapshot) = plan.recovery_snapshot() {
            let saved = self.update_epoch_recovery_accounted_with_writer(
                server,
                document,
                EpochRecoveryAction::Stage(snapshot.clone()),
                clock,
                rng,
                &mut budget.storage,
                hooks,
            )?;
            if saved.state.eviction_pending()?.is_some() {
                return Ok(None);
            }
        }
        // Minted only here, after the staged save RETURNED, or after the same typed validation
        // and inventory check concluded an actually empty source needs no snapshot. It binds the
        // durable predecessor as read BEFORE staging, the version that recovery preserves.
        Ok(Some(CheckedRepairRecovery {
            server,
            predecessor,
            plan: adoption_plan_digest(plan)?,
        }))
    }

    /// Digest of the authenticated on-disk source plaintext: the durable predecessor a
    /// replacement binds, never an in-memory snapshot that could disagree with disk.
    fn durable_studio_digest(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> Result<[u8; 32], AppError> {
        let held = self
            .read_studio_record(&scope_bytes(server, document)?)?
            .ok_or_else(|| invalid("replacement predecessor is missing"))?;
        Ok(*blake3::hash(&held.plain).as_bytes())
    }
}

/// Permission to replace exactly one durable predecessor with the successor of exactly one plan.
/// Not Clone, not Copy, not durable and consumed by the check: it cannot outlive its custody visit.
struct CheckedRepairRecovery {
    server: u64,
    predecessor: [u8; 32],
    plan: [u8; 32],
}

impl CheckedRepairRecovery {
    fn check(
        self,
        server: u64,
        durable_predecessor: [u8; 32],
        plan: &StudioAdoptionPlan,
    ) -> Result<(), AppError> {
        if self.server != server
            || self.predecessor != durable_predecessor
            || self.plan != adoption_plan_digest(plan)?
        {
            return Err(invalid("successor write lacks its recovery capability"));
        }
        Ok(())
    }
}

fn adoption_plan_digest(plan: &StudioAdoptionPlan) -> Result<[u8; 32], AppError> {
    let mut hash = blake3::Hasher::new_derive_key("catcoms/studio-adoption-recovery/v1");
    hash.update(&plan.receipt().hash());
    match plan.recovery_snapshot() {
        Some(snapshot) => hash.update(&[1]).update(&snapshot.id().map_err(invalid)?),
        None => hash.update(&[0]),
    };
    Ok(*hash.finalize().as_bytes())
}
