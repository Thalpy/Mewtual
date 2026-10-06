//! The repair step (design 5.7, 10.3). Only this runtime holds the durable owner snapshot, so the
//! explicit decision reaches issuance here, and the owner resumes a persisted decision after a
//! crash in the same slot as rotation, after discovery, seed and page work. Studio repair work
//! never runs in these entry points: each schedules the detached job in `repair_job`, and the
//! `execute_*` handlers below are its S3. Round-robin over watched targets, with a 5 s cadence that
//! backs off to 60 s on any hold; a persistent hold also stops that target's seed refetches and
//! automatic jobs for 60 s. A hold is always a per-target wait: nothing here returns an error that
//! would pause catch-up.
use super::repair_job::{RepairInput, RepairSchedule};
use super::*;
use crate::store::{OfferedRepairEvidence, StudioRepairOutcome, StudioRepairRequest};
use crate::studio::{StudioFaultScope, StudioRepairReport};
use catcoms_replication::{Receipt, ReceiptRepair};
use zeroize::Zeroizing;

/// Remembered terminal Registry repairs, bounded; forgetting one only costs a reload.
const MAX_REMEMBERED_REGISTRY_REPAIRS: usize = 64;
/// Remembered terminal Studio repairs and last-attempt reports, each bounded the same way.
/// Forgetting a terminal repair costs one more job; forgetting a report only hides it.
const MAX_REMEMBERED_REPAIRS: usize = 64;
/// How long a persistent repair hold suppresses refetching that target's selected seed. The same
/// 60 s the ordinary installer waits after a recovery warning.
const REPAIR_HOLD_BACKOFF_MS: u64 = 60_000;

impl CatchupRuntime {
    #[cfg(test)]
    pub(in crate::studio::receiver) fn hold_checkpoint_for_test(
        &mut self,
        pass: crate::studio_exchange::discovery::ServerCheckpointFetch,
    ) {
        self.checkpoint = Some(pass);
    }

    #[cfg(test)]
    pub(in crate::studio::receiver) fn take_checkpoint_for_test(
        &mut self,
    ) -> Option<crate::studio_exchange::discovery::ServerCheckpointFetch> {
        self.checkpoint.take()
    }

    /// Test-only bridge for exercising this private router with a transport-produced pass.
    #[cfg(test)]
    pub(in crate::studio::receiver) fn route_registry_checkpoint_for_test<
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        pass: crate::studio_exchange::discovery::ServerCheckpointFetch,
    ) -> Result<
        (
            bool,
            Option<crate::studio_exchange::discovery::ServerCheckpointFetch>,
        ),
        AppError,
    > {
        self.checkpoint = Some(pass);
        let ordinary = self
            .route_checkpoint_install(server, store, id, None)?
            .is_none();
        Ok((ordinary, self.checkpoint.take()))
    }

    /// Fail-closed integration gate for the Registry repair transactions, which still run
    /// synchronously while the receiver owns Server/store custody. Studio repair runs through the
    /// detached job in `repair_job`; Registry live discovery, owner resume and repaired-seed
    /// installation must not enter their transactions until the same job owns capture, detached
    /// rebuild, result custody and the revalidation at commit for a bucket.
    ///
    /// Keep this as a function rather than a public/configurable flag: unfinished repair is not a
    /// user option and must not be enabled accidentally by configuration or a renderer command.
    pub(super) fn registry_repair_execution_ready() -> bool {
        false
    }

    /// The bounded last-attempt report a fault view shows for `target`.
    pub(in crate::studio::receiver) fn repair_report(
        &self,
        target: CheckpointTarget,
    ) -> Option<StudioRepairReport> {
        self.repair_reports.get(&target).cloned()
    }

    pub(super) fn report_repair(
        &mut self,
        target: CheckpointTarget,
        outcome: Result<StudioRepairOutcome, &AppError>,
    ) {
        if self.repair_reports.len() >= MAX_REMEMBERED_REPAIRS
            && !self.repair_reports.contains_key(&target)
        {
            self.repair_reports.clear();
        }
        let report = match outcome {
            Ok(outcome) => StudioRepairReport::Completed(outcome),
            Err(error) => StudioRepairReport::Failed(error.to_string().chars().take(256).collect()),
        };
        self.repair_reports.insert(target, report);
    }

    fn remember_repair(&mut self, target: CheckpointTarget, repair: &ReceiptRepair) {
        if self.repairs_seen.len() >= MAX_REMEMBERED_REPAIRS {
            self.repairs_seen.clear();
        }
        self.repairs_seen.insert((target, repair.hash()));
    }

    /// The visible exit (Flow X): `Repairing` has exactly this producer, after the application
    /// barrier, while a replacement is owed. A hold changed nothing, so it reports the saved
    /// phase like a terminal outcome does; before B2 a Fault therefore stays Fault.
    fn note_repair(
        &mut self,
        target: StudioTarget,
        outcome: StudioRepairOutcome,
        phase: EpochPhase,
    ) {
        match outcome {
            StudioRepairOutcome::AwaitingSeed => {
                self.settlement
                    .note(target, StudioSettlementState::Repairing);
            }
            StudioRepairOutcome::RecoveryPending => {
                self.settlement
                    .note(target, StudioSettlementState::Repairing);
                self.settlement
                    .note(target, StudioSettlementState::RecoveryEvictionPending);
            }
            StudioRepairOutcome::StorageRefused => {
                self.settlement
                    .note(target, StudioSettlementState::StorageRefused);
            }
            StudioRepairOutcome::Held(_)
            | StudioRepairOutcome::Repaired
            | StudioRepairOutcome::Screened
            | StudioRepairOutcome::Installed
            | StudioRepairOutcome::AlreadyRepaired => self.settlement.note(target, phase.into()),
        }
    }

    fn note_repair_failure(&mut self, target: StudioTarget, error: &AppError) {
        // The same bounded, surfaced diagnostic slot as owner rotation.
        self.owner_failure = Some((target, error.to_string().chars().take(256).collect()));
    }

    /// A bucket pass may belong to no Studio target; its failure is then only held, not shown.
    pub(super) fn note_repair_failure_for(
        &mut self,
        target: Option<StudioTarget>,
        error: &AppError,
    ) {
        if let Some(target) = target {
            self.note_repair_failure(target, error);
        }
    }

    fn remember_registry_repair(&mut self, bucket: u8, repair: &ReceiptRepair) {
        if self.registry_repairs_seen.len() >= MAX_REMEMBERED_REGISTRY_REPAIRS {
            self.registry_repairs_seen.clear();
        }
        self.registry_repairs_seen.insert((bucket, repair.hash()));
    }

    /// A repair that owes its replacement needs the selected checkpoint's seed. No fresh owner
    /// proof will name that receipt while a decision is held or after the owner moved on, so
    /// mint the seed pass from the locally verified repair and let the existing checkpoint
    /// machinery fetch it. `route_checkpoint_install` then installs it through the repair itself.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn await_repaired_seed<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &ServerStore,
        id: u64,
        target: CheckpointTarget,
        failure_target: Option<StudioTarget>,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
    ) {
        let now = server.runtime_clock().monotonic_ms();
        if self.checkpoint.is_some()
            || self.in_flight
            || self.discovery_plan.is_some()
            || self.pass.is_some()
        {
            // One checkpoint pass at a time, never under a pending discovery whose completion
            // would drop it, and never at the cost of another target's page pass, which may
            // already hold a fetched page (S3 runs at any time). The next turn asks again.
            return;
        }
        if self
            .repair_backoff
            .get(&target)
            .is_some_and(|until| now < *until)
        {
            // A persistent hold: refetching the seed would fail the same way.
            return;
        }
        let Some(selected) = pair
            .iter()
            .find(|r| r.hash() == repair.selected_receipt_hash)
        else {
            return;
        };
        // Any member that installed the selected checkpoint can serve its seed, the owner
        // included; rotate so one peer without it cannot be asked forever.
        let peers = server.sync.studio_page_peers();
        if peers.is_empty() {
            return;
        }
        let peer = peers[self.repair_seed_peer % peers.len()];
        self.repair_seed_peer = self.repair_seed_peer.wrapping_add(1);
        match server.select_repaired_checkpoint(store, id, target, repair, selected) {
            Ok(pass) => {
                self.checkpoint = Some(pass);
                self.checkpoint_sealed = true;
                self.checkpoint_peer = Some(peer);
                self.checkpoint_retry = 0;
                self.repair_failure_target = failure_target;
            }
            Err(error) => {
                self.note_repair_failure_for(failure_target, &error);
                // CORE-007 or an unobserved tenure: neither clears within a discovery turn.
                self.hold_repair(target, now);
            }
        }
    }

    /// The owner's explicit `RepairFault`. Without a current durable snapshot this device is
    /// not provably the owner, and it refuses rather than signing on in-memory tenure. Those
    /// refusals come first, before anything is reserved; the decision itself is a job, so the
    /// answer is only whether it was scheduled. Its outcome is read back through the fault view.
    pub(in crate::studio::receiver) fn repair_fault<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        request: StudioRepairRequest,
    ) -> Result<StudioControlResponse, AppError> {
        // V5 first, so an unobserved tenure is refused as such before any other reason.
        server.require_observed_owner_tenure()?;
        let snapshot = self.owner_snapshot.clone().ok_or_else(|| {
            invalid("only the current owner, with a durable snapshot, may decide")
        })?;
        if !server.owner_head_snapshot_is_current(store, id, &snapshot) {
            return Err(invalid("the durable owner snapshot is stale; retry"));
        }
        let scope = CheckpointTarget::Studio(target);
        let start = match self.start_repair(
            server,
            store,
            id,
            scope,
            Some(target),
            RepairInput::Decide(request),
        ) {
            // An explicit decision ignores backoff, so a hold here means it could not start at
            // all; asking again would fail the same way, so say why instead of "busy".
            RepairSchedule::Held => {
                let reason = match self.repair_report(scope) {
                    Some(StudioRepairReport::Failed(reason)) => reason,
                    _ => "the repair could not start".into(),
                };
                return Err(invalid(reason));
            }
            started => started.start(),
        };
        Ok(StudioControlResponse::RepairStarted {
            target,
            scope: StudioFaultScope::Source,
            start,
        })
    }

    /// The owner's explicit decision for the target's Registry bucket. The bucket's prepared
    /// provider is dropped afterwards: any write invalidates a read-only prepared wrapper.
    pub(in crate::studio::receiver) fn repair_registry_fault<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        request: StudioRepairRequest,
    ) -> Result<StudioControlResponse, AppError> {
        if !Self::registry_repair_execution_ready() {
            return Err(invalid(
                "Registry fault repair awaits detached admitted runtime execution",
            ));
        }
        let snapshot = self.owner_snapshot.clone().ok_or_else(|| {
            invalid("only the current owner, with a durable snapshot, may decide")
        })?;
        if !server.owner_head_snapshot_is_current(store, id, &snapshot) {
            return Err(invalid("the durable owner snapshot is stale; retry"));
        }
        let bucket = server.studio_registry_bucket(target)?;
        let mut budget = Self::inventory_budget(server, store, id)?;
        self.settlement
            .note(target, StudioSettlementState::RefreshRequired);
        self.registry_provider = None;
        let (repair, outcome, _) = server.issue_registry_fault_repair(
            store,
            id,
            target,
            &snapshot,
            request,
            None,
            &mut budget,
        )?;
        if outcome == StudioRepairOutcome::AwaitingSeed {
            let held = server
                .sync
                .with_registry_context(|g, d, _, _| store.held_registry_repair(id, g, bucket, d));
            if let Ok(Some((_, pair))) = held {
                self.await_repaired_seed(
                    server,
                    store,
                    id,
                    CheckpointTarget::Registry(bucket),
                    Some(target),
                    &repair,
                    &pair,
                );
            }
        }
        Ok(StudioControlResponse::Repaired {
            target,
            scope: StudioFaultScope::RegistryBucket(bucket),
            outcome,
        })
    }

    /// Flow D: an authenticated answer carried a repair. A peer schedules a job that applies it
    /// at S3, against its own rebuilt source and with the pair assembled there; which case it
    /// lands in is the core's classification alone. The owner never re-applies a decision from an
    /// answer: its own are resumed by `repair_owner`.
    ///
    /// Returns whether the repair took this target. When it did (scheduled, or another job or a
    /// full pool made it wait), the caller must drop any pass from the same answer rather than
    /// let it reach the installer. Otherwise the pass goes to the router, which still defers for
    /// any durable or owed claim: a repair already terminal here, one this device cannot use, a
    /// target backing off, or a replacement already owed for exactly this repair, whose selected
    /// seed the router (or, with no pass, this call) fetches without rerunning the job.
    ///
    /// Before anything is reserved or read, the repair must verify against the live owner and
    /// this device's own authoring tenure, the same first step the transaction takes. A newcomer
    /// with `Unknown` or `Imported` tenure therefore schedules nothing (N16), and a repair signed
    /// by anyone but the current owner costs no capture and holds nothing else.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn offer_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        repair: &ReceiptRepair,
        offered: Option<&Receipt>,
        with_pass: bool,
    ) -> bool {
        let owner = server
            .sync
            .with_registry_context(|g, d, _, _| g.designated_committer() == Some(d.device_id()));
        let scope = CheckpointTarget::Studio(target);
        let now = server.runtime_clock().monotonic_ms();
        if owner
            || self.repairs_seen.contains(&(scope, repair.hash()))
            || self
                .repair_unverifiable
                .get(&(scope, repair.hash()))
                .is_some_and(|until| now < *until)
        {
            return false;
        }
        let Some(tenure) = server.sync.authoring_owner_tenure_start() else {
            return false;
        };
        let authorized = server
            .sync
            .with_registry_context(|g, _, _, _| repair.verify_current_owner(g, tenure).is_ok());
        if !authorized {
            return false;
        }
        let owed = server
            .sync
            .with_registry_context(|g, d, _, _| store.owed_studio_repair(id, g, target, d));
        if let Some((owed, pair)) = owed.filter(|(owed, _)| owed.hash() == repair.hash()) {
            // Already applied here and waiting only for its seed: rerunning the job would just
            // flush the same B2 again. Fetch the seed instead.
            if !with_pass {
                self.await_repaired_seed(server, store, id, scope, Some(target), &owed, &pair);
            }
            return false;
        }
        let input = RepairInput::Offered {
            repair: Box::new(repair.clone()),
            offered: offered.cloned().map(Box::new),
        };
        match self.start_repair(server, store, id, scope, Some(target), input) {
            RepairSchedule::Scheduled | RepairSchedule::Busy => true,
            RepairSchedule::Held => false,
        }
    }

    /// Flow D for a Registry bucket: the same rules as a Studio source. A faulted bucket blocks
    /// discovery, so the repair an owner's answer carries is applied here before anything else.
    /// The bucket's own detached preparation runs first when its source is large.
    pub(super) fn apply_offered_registry_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        bucket: u8,
        repair: &ReceiptRepair,
        offered: Option<&Receipt>,
    ) -> Result<Option<(StudioRepairOutcome, [Receipt; 2])>, AppError> {
        if !Self::registry_repair_execution_ready() {
            return Ok(None);
        }
        let owner = server
            .sync
            .with_registry_context(|g, d, _, _| g.designated_committer() == Some(d.device_id()));
        if owner
            || self
                .registry_repairs_seen
                .contains(&(bucket, repair.hash()))
            || (!store.registry_receive_source_fits(id, &server.group_id(), bucket)?
                && !self.prepare_registry_inventory(server, store, id, bucket)?)
        {
            return Ok(None);
        }
        let pair = server.sync.with_registry_context(|g, d, _, _| {
            store.registry_repair_evidence(id, g, bucket, d, repair, offered)
        });
        let pair = match pair {
            Ok(OfferedRepairEvidence::Pair(pair)) => *pair,
            // Terminal here: remember it so later answers carrying the same repair cost no
            // further Registry restores on the actor. Unverifiable may change, so it is not.
            Ok(OfferedRepairEvidence::Terminal) => {
                self.remember_registry_repair(bucket, repair);
                return Ok(None);
            }
            Ok(OfferedRepairEvidence::Unverifiable) => return Ok(None),
            Err(error) => {
                if let Some(target) = self.target {
                    self.note_repair_failure(target, &error);
                }
                return Ok(None);
            }
        };
        let mut budget = Self::inventory_budget(server, store, id)?;
        self.registry_provider = None;
        match server.apply_registry_bucket_repair(
            store,
            id,
            bucket,
            repair,
            &pair,
            None,
            &mut budget,
        ) {
            Ok((outcome, _)) => {
                if outcome.is_terminal() {
                    self.remember_registry_repair(bucket, repair);
                }
                Ok(Some((outcome, pair)))
            }
            Err(error) => {
                if let Some(target) = self.target {
                    self.note_repair_failure(target, &error);
                }
                Ok(None)
            }
        }
    }

    /// The owner resumes a held Registry decision for the bucket behind `target`. The explicit
    /// decision itself is never made here: only one already persisted at B1 is continued. Paced
    /// like the Studio step: a hold or failure backs off to 60 s instead of every Registry turn.
    pub(super) fn resume_registry_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        bucket: u8,
    ) -> Result<bool, AppError> {
        if !Self::registry_repair_execution_ready() {
            return Ok(false);
        }
        let now = server.runtime_clock().monotonic_ms();
        if now < self.registry_repair_next_at {
            return Ok(false);
        }
        let Some(snapshot) = self.owner_snapshot.clone() else {
            return Ok(false);
        };
        if !server.owner_head_snapshot_is_current(store, id, &snapshot) {
            return Ok(false);
        }
        let held = server
            .sync
            .with_registry_context(|g, d, _, _| store.held_registry_repair(id, g, bucket, d));
        let Ok(Some((repair, pair))) = held else {
            return Ok(false);
        };
        let mut budget = Self::inventory_budget(server, store, id)?;
        self.registry_provider = None;
        match server.resume_registry_fault_repair(
            store,
            id,
            target,
            &snapshot,
            &repair,
            &pair,
            None,
            &mut budget,
        ) {
            Ok((outcome, _)) if outcome.is_terminal() => {}
            Ok((outcome, _)) => {
                self.registry_repair_next_at = now.saturating_add(60_000);
                if outcome == StudioRepairOutcome::AwaitingSeed {
                    self.await_repaired_seed(
                        server,
                        store,
                        id,
                        CheckpointTarget::Registry(bucket),
                        Some(target),
                        &repair,
                        &pair,
                    );
                }
            }
            Err(error) => {
                self.registry_repair_next_at = now.saturating_add(60_000);
                self.note_repair_failure(target, &error);
            }
        }
        Ok(true)
    }

    /// Resume a persisted owner decision. A held B1 decision owns its target until terminal, so
    /// leaving it unresumed after a restart would strand the fault; this step schedules that
    /// resume. Finding the decision is a bounded owner-record read; the source work is the job's.
    pub(super) fn repair_owner<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        watches: &VecDeque<(ServerStudioWatch, u128)>,
    ) -> Result<Option<StudioTarget>, AppError> {
        let now = server.runtime_clock().monotonic_ms();
        if now < self.repair_next_at || watches.is_empty() {
            return Ok(None);
        }
        let Some(snapshot) = self.owner_snapshot.clone() else {
            return Ok(None);
        };
        if !server.owner_head_snapshot_is_current(store, id, &snapshot) {
            return Ok(None);
        }
        let target = watches[self.repair_selection % watches.len()].0.target;
        self.repair_selection = self.repair_selection.wrapping_add(1);
        self.repair_next_at = now.saturating_add(5_000);
        let held = server
            .sync
            .with_registry_context(|g, d, _, _| store.held_studio_repair(id, g, target, d));
        let held = match held {
            Ok(Some((held, _))) => held,
            Ok(None) => return Ok(None),
            Err(error) => {
                self.note_repair_failure(target, &error);
                self.repair_next_at = now.saturating_add(60_000);
                return Ok(None);
            }
        };
        let scope = CheckpointTarget::Studio(target);
        let owed = server
            .sync
            .with_registry_context(|g, d, _, _| store.owed_studio_repair(id, g, target, d));
        if let Some((owed, pair)) = owed.filter(|(owed, _)| owed.hash() == held.hash()) {
            // B2 already crossed for this decision; only its seed is missing. A resume would just
            // flush the same source again, so fetch the seed and come back on the long cadence.
            self.await_repaired_seed(server, store, id, scope, Some(target), &owed, &pair);
            self.repair_next_at = now.saturating_add(60_000);
            return Ok(None);
        }
        // `Busy` is another job or a full pool, retried on the ordinary cadence; only a hold on
        // this target slows the round-robin.
        if self.start_repair(server, store, id, scope, Some(target), RepairInput::Resume)
            == RepairSchedule::Held
        {
            self.repair_next_at = now.saturating_add(60_000);
        }
        Ok(None)
    }

    /// S3 of an explicit decision: issuance, B1 and Flow A in the unchanged transaction, on the
    /// source this job rebuilt. The durable snapshot must still be current at this moment.
    pub(super) fn execute_decision<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        request: StudioRepairRequest,
        budget: &mut crate::store::EpochStudioBudget,
    ) -> Result<Option<StudioTarget>, AppError> {
        let snapshot = self.current_owner_snapshot(server, store, id)?;
        let (repair, outcome, state) = server
            .issue_studio_fault_repair(store, id, target, &snapshot, request, None, budget)?;
        self.finish_studio_repair(server, store, id, target, &repair, outcome, state);
        Ok(Some(target))
    }

    /// S3 of an owner resume: the decision held at B1, read again at this moment.
    pub(super) fn execute_resume<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        budget: &mut crate::store::EpochStudioBudget,
    ) -> Result<Option<StudioTarget>, AppError> {
        let snapshot = self.current_owner_snapshot(server, store, id)?;
        let Some((repair, pair)) = server
            .sync
            .with_registry_context(|g, d, _, _| store.held_studio_repair(id, g, target, d))?
        else {
            // Completed or recycled since S1; nothing is owed.
            return Ok(None);
        };
        let (outcome, state) = server.resume_studio_fault_repair(
            store, id, target, &snapshot, &repair, &pair, None, budget,
        )?;
        self.finish_studio_repair(server, store, id, target, &repair, outcome, state);
        Ok(Some(target))
    }

    /// S3 of Flow D. The pair comes from evidence this rebuilt source holds plus the offered
    /// receipt; a device that cannot assemble it applies nothing and stops asking for a while.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn execute_offered<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        repair: &ReceiptRepair,
        offered: Option<&Receipt>,
        budget: &mut crate::store::EpochStudioBudget,
    ) -> Result<Option<StudioTarget>, AppError> {
        let scope = CheckpointTarget::Studio(target);
        let now = server.runtime_clock().monotonic_ms();
        let terminal = server.sync.with_registry_context(|g, d, _, _| {
            store.studio_repair_is_terminal(id, g, target, d, repair)
        });
        if terminal {
            self.remember_repair(scope, repair);
            return Ok(None);
        }
        let Some(pair) = server.sync.with_registry_context(|g, d, _, _| {
            store.studio_repair_evidence(id, g, target, d, repair, offered)
        }) else {
            // Unverifiable here today. Only this repair is held, never the target: another
            // repair, or this one's owed seed, must not wait behind it.
            self.repair_unverifiable.retain(|_, until| now < *until);
            if self.repair_unverifiable.len() >= MAX_REMEMBERED_REPAIRS {
                self.repair_unverifiable.clear();
            }
            self.repair_unverifiable.insert(
                (scope, repair.hash()),
                now.saturating_add(REPAIR_HOLD_BACKOFF_MS),
            );
            return Ok(None);
        };
        let (outcome, state) =
            server.apply_studio_fault_repair(store, id, target, repair, &pair, None, budget)?;
        self.finish_studio_repair(server, store, id, target, repair, outcome, state);
        Ok(Some(target))
    }

    /// S3 of the owed replacement: the seed S1 extracted, installed through the repair
    /// transaction itself. The owner goes through its current durable snapshot exactly as a
    /// resume does; a peer through Flow A, which refuses the owner.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn execute_replace<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
        seed: &[u8],
        budget: &mut crate::store::EpochStudioBudget,
    ) -> Result<Option<StudioTarget>, AppError> {
        let owner = server
            .sync
            .with_registry_context(|g, d, _, _| g.designated_committer() == Some(d.device_id()));
        let (outcome, state) = if owner {
            let snapshot = self.current_owner_snapshot(server, store, id)?;
            server.resume_studio_fault_repair(
                store,
                id,
                target,
                &snapshot,
                repair,
                pair,
                Some(seed),
                budget,
            )?
        } else {
            server.apply_studio_fault_repair(store, id, target, repair, pair, Some(seed), budget)?
        };
        let now = server.runtime_clock().monotonic_ms();
        if outcome.is_terminal() {
            self.binding = Some((target, state.doc_id()));
            // The replacement opened a new epoch for this target. Its stale discovery binding
            // goes, but S3 runs at any time: another target's discovery is left untouched.
            if self.target == Some(target) {
                self.discovery_needed = None;
                self.discovery_watch = None;
            }
            self.next_at = now;
        }
        self.finish_studio_repair(server, store, id, target, repair, outcome, state);
        Ok(Some(target))
    }

    /// The durable owner snapshot, required current at the moment it is used (design 6.3).
    fn current_owner_snapshot<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &mut Server<T, R>,
        store: &ServerStore,
        id: u64,
    ) -> Result<ServerOwnerSnapshot, AppError> {
        self.owner_snapshot
            .clone()
            .filter(|snapshot| server.owner_head_snapshot_is_current(store, id, snapshot))
            .ok_or_else(|| invalid("the durable owner snapshot is stale; retry"))
    }

    /// Everything a committed repair transaction is followed by, whichever input ran it: retain
    /// the saved source, label it, report it, remember it once terminal, back off from a
    /// persistent hold, and fetch the selected seed when a replacement is owed.
    #[allow(clippy::too_many_arguments)]
    fn finish_studio_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        repair: &ReceiptRepair,
        outcome: StudioRepairOutcome,
        state: crate::store::EpochStudioState,
    ) {
        let scope = CheckpointTarget::Studio(target);
        let now = server.runtime_clock().monotonic_ms();
        let phase = state.phase();
        server
            .sync
            .with_registry_context(|g, d, _, _| store.retain_studio_source(g, d, state));
        self.note_repair(target, outcome, phase);
        self.report_repair(scope, Ok(outcome));
        match outcome {
            outcome if outcome.is_terminal() => {
                self.repair_backoff.remove(&scope);
                self.remember_repair(scope, repair);
            }
            StudioRepairOutcome::AwaitingSeed => {
                // The owner's next resume would only flush the same B2 again; the seed fetch below
                // and later offers carry the work from here (review M1).
                self.repair_next_at = self.repair_next_at.max(now.saturating_add(60_000));
                let pair = server
                    .sync
                    .with_registry_context(|g, d, _, _| store.owed_studio_repair(id, g, target, d));
                if let Some((owed, pair)) = pair {
                    self.await_repaired_seed(server, store, id, scope, Some(target), &owed, &pair);
                }
            }
            // Recovery warning, storage refusal or a hold: they need the user or the owner.
            _ => self.hold_repair(scope, now),
        }
    }

    /// Stop refetching `target`'s repaired seed for a while; expired holds are dropped here, so
    /// the map stays as small as the set of targets currently held.
    pub(super) fn hold_repair(&mut self, target: CheckpointTarget, now: u64) {
        self.repair_backoff.retain(|_, until| now < *until);
        self.repair_backoff
            .insert(target, now.saturating_add(REPAIR_HOLD_BACKOFF_MS));
    }

    /// Route a checkpoint pass before the ordinary installer, so a repair is never an installer
    /// error that pauses all catch-up:
    /// - the source owes a repair and this pass has fetched exactly its selected seed: install
    ///   through the repair transaction itself, with typed RecoveryPending/StorageRefused and
    ///   owner recycling (the owner only through a current durable snapshot);
    /// - the source owes a repair and this pass names another receipt, or has no seed yet: drop
    ///   it and fetch the repair's own selected seed instead, from the source's committed
    ///   evidence (a peer self-heals here);
    /// - a held owner decision owns the target: defer this target only;
    /// - otherwise the ordinary installer proceeds (`Ok(None)`).
    ///
    /// `failure_target` is where diagnostics go; a bucket pass may have none.
    pub(super) fn route_checkpoint_install<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        failure_target: Option<StudioTarget>,
    ) -> Result<Option<Option<StudioTarget>>, AppError> {
        let Some(pass) = self.checkpoint.as_ref() else {
            return Ok(None);
        };
        let target = pass.inner.target();
        let selected = pass.inner.selected_receipt().clone();
        let now = server.runtime_clock().monotonic_ms();
        if self.repair_claimed(target) {
            // A live repair job owns this source between S1 and S4. Nothing installs into it,
            // and this target's discovery is not rescheduled: the job is what unblocks it.
            self.checkpoint = None;
            self.retry_discovery(now);
            return Ok(Some(None));
        }
        let automatic_repair_ready = match target {
            CheckpointTarget::Studio(_) => true,
            CheckpointTarget::Registry(_) => Self::registry_repair_execution_ready(),
        };
        let owed = match target {
            CheckpointTarget::Studio(studio) => server
                .sync
                .with_registry_context(|g, d, _, _| Ok(store.owed_studio_repair(id, g, studio, d))),
            CheckpointTarget::Registry(bucket) if !automatic_repair_ready => {
                // Registry reconstruction is detached and already retained by the page provider.
                // Rebuilding it here, under actor/store custody, would defeat that boundary even
                // though automatic repair execution is disabled. Unknown classification is not
                // "no repair": a missing/cold/stale provider must defer rather than fall through
                // to ordinary installation.
                let classification = match self.registry_provider.as_mut() {
                    Some(provider) => {
                        server.prepared_registry_repair_install_pending(store, id, bucket, provider)
                    }
                    None => Ok(None),
                };
                match classification {
                    Ok(Some(false)) => Ok(None),
                    Ok(Some(true) | None) => {
                        self.checkpoint = None;
                        self.retry_discovery(now);
                        return Ok(Some(None));
                    }
                    Err(error) => {
                        self.note_repair_failure_for(failure_target, &error);
                        self.checkpoint = None;
                        self.retry_discovery(now);
                        return Ok(Some(None));
                    }
                }
            }
            CheckpointTarget::Registry(bucket) => server
                .sync
                .with_registry_context(|g, d, _, _| store.owed_registry_repair(id, g, bucket, d)),
        };
        let owed = match owed {
            Ok(owed) => owed,
            Err(error) => {
                self.note_repair_failure_for(failure_target, &error);
                self.retry_discovery(now);
                return Ok(Some(None));
            }
        };
        if let Some((repair, pair)) = owed {
            if !automatic_repair_ready {
                // Do not let the ordinary installer consume the decision's selected checkpoint.
                // The fetched pass is network-derived and disposable; durable repair/source state
                // remains untouched for the future detached job.
                self.checkpoint = None;
                self.retry_discovery(now);
                return Ok(Some(None));
            }
            let fetched = self
                .checkpoint
                .as_ref()
                .is_some_and(|pass| pass.inner.is_fetched());
            if selected.hash() != repair.selected_receipt_hash || !fetched {
                // A different head, or a proof pass with no seed yet: fetch the repair's own
                // selected seed through a repaired pass instead. That selection is made under
                // this device's authoring tenure, never under a proof's own claim (6.3).
                self.checkpoint = None;
                self.await_repaired_seed(server, store, id, target, failure_target, &repair, &pair);
                if self.checkpoint.is_none() {
                    // No seed pass (a persistent hold, CORE-007, or no reachable peer): hold
                    // this target on the ordinary rotating cadence.
                    self.retry_discovery(now);
                }
                return Ok(Some(None));
            }
            if let CheckpointTarget::Studio(studio) = target {
                // S1 of the replacement: the seed is taken from the pass only if its selection was
                // made under this device's observed tenure. The install is the job's S3, on a
                // source rebuilt detached.
                let pass = self.checkpoint.take().expect("pass");
                let seed = match server.repaired_seed_bytes(store, id, &pass, &repair) {
                    Ok(seed) => seed,
                    Err(error) => {
                        self.note_repair_failure_for(failure_target, &error);
                        self.hold_repair(target, now);
                        self.retry_discovery(now);
                        return Ok(Some(None));
                    }
                };
                let input = RepairInput::Replace {
                    repair: Box::new(repair),
                    pair: Box::new(pair),
                    seed: Zeroizing::new(seed),
                };
                if self.start_repair(server, store, id, target, Some(studio), input)
                    == RepairSchedule::Busy
                {
                    // Another job or a full pool: keep the fetched seed rather than fetch it
                    // again, and look again shortly instead of on every turn.
                    self.checkpoint = Some(pass);
                    self.checkpoint_retry = now.saturating_add(1_000);
                    return Ok(Some(None));
                }
                self.retry_discovery(now);
                return Ok(Some(None));
            }
            let owner = server.sync.with_registry_context(|g, d, _, _| {
                g.designated_committer() == Some(d.device_id())
            });
            // The owner installs only as a resume would: through a current durable snapshot.
            let snapshot = match self.owner_snapshot.clone() {
                Some(snapshot)
                    if owner && server.owner_head_snapshot_is_current(store, id, &snapshot) =>
                {
                    Some(snapshot)
                }
                _ if owner => {
                    let stale = invalid("the durable owner snapshot is stale; retry");
                    self.note_repair_failure_for(failure_target, &stale);
                    self.retry_discovery(now);
                    return Ok(Some(None));
                }
                _ => None,
            };
            let mut budget = Self::budget(server, store, id)?;
            let pass = self.checkpoint.take().expect("pass");
            if let Some(failure_target) = failure_target {
                self.settlement
                    .note(failure_target, StudioSettlementState::RefreshRequired);
            }
            // Registry only: the Studio replacement returned above as a job.
            self.registry_provider = None;
            let installed = server
                .install_repaired_registry_seed(
                    store,
                    id,
                    &pass,
                    &repair,
                    &pair,
                    snapshot.as_ref(),
                    &mut budget,
                )
                .map(|(outcome, _)| outcome);
            return Ok(Some(match installed {
                Ok(outcome) if outcome.is_terminal() => {
                    self.repair_backoff.remove(&target);
                    self.reset_registry_tail(server, now);
                    self.discovery_plan = self.after_registry.take();
                    None
                }
                Ok(_) => {
                    // Recovery warning, storage refusal or a hold: everything is retained. They
                    // need the user or the owner, so the seed is not refetched for a while.
                    self.hold_repair(target, now);
                    self.retry_discovery(now);
                    None
                }
                Err(error) => {
                    self.note_repair_failure_for(failure_target, &error);
                    self.hold_repair(target, now);
                    self.retry_discovery(now);
                    None
                }
            }));
        }
        let deferred = server
            .sync
            .with_registry_context(|g, d, _, _| match target {
                CheckpointTarget::Studio(studio) => {
                    store.studio_install_deferred_by_repair(id, g, studio, d, &selected)
                }
                // The bucket owes nothing (checked above), so only a held decision can defer it.
                CheckpointTarget::Registry(bucket) => {
                    store.registry_install_deferred_by_repair(id, g, bucket, None, &selected)
                }
            });
        match deferred {
            Ok(false) => Ok(None),
            Ok(true) => {
                self.retry_discovery(now);
                Ok(Some(None))
            }
            Err(error) => {
                // An unreadable owner record is not a reason to install, and not silent either.
                self.note_repair_failure_for(failure_target, &error);
                self.retry_discovery(now);
                Ok(Some(None))
            }
        }
    }
}
