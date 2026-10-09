//! The repair step (design 5.7, 10.3). Only this runtime holds the durable owner snapshot, so the
//! explicit decision reaches issuance here, and the owner resumes a persisted decision after a
//! crash in the same slot as rotation, after discovery, seed and page work. Repair work, for a
//! Studio source or a Registry bucket alike, never runs in these entry points: each schedules the
//! detached job in `repair_job`, and the `execute_*` handlers below are its S3. Round-robin over
//! watched targets on a 5 s cadence. A hold, a failure, a started seed fetch or a started resume
//! job defers only that target's next visit, never the cadence for the others: by 60 s, doubling
//! each time the target is deferred again up to 15 min, until it reaches a terminal outcome, the
//! owner decides anew or the person acknowledges its warning. A persistent hold also stops that
//! target's seed refetches and automatic jobs for 60 s. A hold is always a per-target wait:
//! nothing here returns an error that would pause catch-up. This paces the owner's resume visits
//! only; a peer's repaired-seed fetch is paced by the checkpoint slot and holds alone.
use super::repair_job::{RepairInput, RepairSchedule};
use super::*;
use crate::store::{
    OfferedRepairEvidence, PreparedRegistryRepair, StudioRepairOutcome, StudioRepairRequest,
};
use crate::studio::{StudioFaultScope, StudioRepairReport};
use catcoms_replication::{Receipt, ReceiptRepair};
use zeroize::Zeroizing;

/// Remembered terminal repairs (Studio sources and Registry buckets, keyed by target) and
/// last-attempt reports, each bounded the same way. Forgetting a terminal repair costs one more
/// job; forgetting a report only hides it.
const MAX_REMEMBERED_REPAIRS: usize = 64;
/// How long a persistent repair hold suppresses refetching that target's selected seed. The same
/// 60 s the ordinary installer waits after a recovery warning. Also the first step of a target's
/// owner-resume visit deferral, which doubles from here (see `defer_visit`).
const REPAIR_HOLD_BACKOFF_MS: u64 = 60_000;
/// The cap on one target's owner-resume visit deferral: 60 s doubled four times would be 16 min.
const MAX_VISIT_DEFERRAL_MS: u64 = 15 * 60_000;

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

    /// Hold one offered repair, never its target: another repair, or the owed seed of the one the
    /// target already carries, must not wait behind it. An offer is untrusted input, so whatever
    /// stopped it (unverifiable evidence, a held outcome such as a replayed older sequence, or a
    /// failed S3) is charged to that repair alone; otherwise one replayed offer a minute could
    /// keep a target's legitimate replacement from ever being fetched (review MEDIUM-1). The hold
    /// expires rather than becoming terminal, since each of those may change.
    pub(super) fn hold_offer(&mut self, scope: CheckpointTarget, repair: [u8; 32], now: u64) {
        self.hold_offer_for(scope, repair, now, REPAIR_HOLD_BACKOFF_MS);
    }

    /// `hold_offer` for a chosen wait: a stale rebuild holds its offer only as long as it would
    /// have held the target (see `repair_stale`).
    pub(super) fn hold_offer_for(
        &mut self,
        scope: CheckpointTarget,
        repair: [u8; 32],
        now: u64,
        wait_ms: u64,
    ) {
        self.repair_unverifiable.retain(|_, until| now < *until);
        if self.repair_unverifiable.len() >= MAX_REMEMBERED_REPAIRS {
            self.repair_unverifiable.clear();
        }
        self.repair_unverifiable
            .insert((scope, repair), now.saturating_add(wait_ms));
    }

    fn hold_unverifiable(&mut self, scope: CheckpointTarget, repair: &ReceiptRepair, now: u64) {
        self.hold_offer(scope, repair.hash(), now);
        let unverifiable =
            invalid("this device cannot verify that repair yet; it will be offered again");
        self.report_repair(scope, Err(&unverifiable));
    }

    /// The repair Flow D must not take again on this device: the owner's own (it resumes, never
    /// re-applies), one already terminal here, or one under its own per-offer hold.
    fn offer_refused<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &mut Server<T, R>,
        scope: CheckpointTarget,
        repair: &ReceiptRepair,
    ) -> bool {
        let owner = server
            .sync
            .with_registry_context(|g, d, _, _| g.designated_committer() == Some(d.device_id()));
        let now = server.runtime_clock().monotonic_ms();
        owner
            || self.repairs_seen.contains(&(scope, repair.hash()))
            || self
                .repair_unverifiable
                .get(&(scope, repair.hash()))
                .is_some_and(|until| now < *until)
    }

    /// Before anything is reserved or read, the repair must verify against the live owner and
    /// this device's own authoring tenure, the same first step the transaction takes. A newcomer
    /// with `Unknown` or `Imported` tenure therefore schedules nothing (N16), and a repair signed
    /// by anyone but the current owner costs no capture and holds nothing else.
    fn offer_authorized<T: MeshTransport, R: CryptoRngCore>(
        server: &mut Server<T, R>,
        repair: &ReceiptRepair,
    ) -> bool {
        let Some(tenure) = server.sync.authoring_owner_tenure_start() else {
            return false;
        };
        server
            .sync
            .with_registry_context(|g, _, _, _| repair.verify_current_owner(g, tenure).is_ok())
    }

    /// The replacement `bucket` owes, from the retained prepared provider only, exactly as the
    /// router classifies it: `None` is unknown (no warm, current provider for this bucket),
    /// `Some(None)` a checked "owes nothing". Never a custody restore. An unreadable provider is
    /// unknown too; the caller's own fallback decides what unknown costs.
    #[allow(clippy::type_complexity)]
    fn registry_classification<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &ServerStore,
        id: u64,
        bucket: u8,
    ) -> Option<Option<(ReceiptRepair, [Receipt; 2])>> {
        let provider = self.registry_provider.as_mut()?;
        server
            .prepared_registry_owed_repair(store, id, bucket, provider)
            .ok()
            .flatten()
    }

    /// A Registry write invalidates the read-only prepared wrapper. Drop it now unless a Registry
    /// preparation is queued or may be running: its attachment would then find no provider and
    /// fail the whole visit. That one is left to the stamp check every use of the provider makes
    /// (`registry_page_preparation_is_warm`, `check_source`, the attach itself), which refuses
    /// superseded bytes. `preparing` also covers Studio and preview preparation, so a provider
    /// that still holds a graph is dropped regardless: beginning a preparation clears the graph,
    /// so none can be pending against one that has it, and dropping it releases its pool slot now
    /// rather than at its next use or expiry (review LOW-3).
    pub(super) fn invalidate_registry_provider(&mut self) {
        let holds_graph = self
            .registry_provider
            .as_ref()
            .is_some_and(|p| p.has_prepared_source());
        let none_pending = !self.preparing
            && self.registry_preparation.is_none()
            && self.registry_prepared.is_none();
        if holds_graph || none_pending {
            self.registry_provider = None;
        }
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

    /// The owner's explicit decision for the target's Registry bucket: the same refusals as a
    /// Studio source decision, then the same job, scoped to the bucket. Its outcome is read back
    /// through the bucket's fault view.
    pub(in crate::studio::receiver) fn repair_registry_fault<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        request: StudioRepairRequest,
    ) -> Result<StudioControlResponse, AppError> {
        server.require_observed_owner_tenure()?;
        let snapshot = self.owner_snapshot.clone().ok_or_else(|| {
            invalid("only the current owner, with a durable snapshot, may decide")
        })?;
        if !server.owner_head_snapshot_is_current(store, id, &snapshot) {
            return Err(invalid("the durable owner snapshot is stale; retry"));
        }
        let bucket = server.studio_registry_bucket(target)?;
        let scope = CheckpointTarget::Registry(bucket);
        let start = match self.start_repair(
            server,
            store,
            id,
            scope,
            Some(target),
            RepairInput::Decide(request),
        ) {
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
            scope: StudioFaultScope::RegistryBucket(bucket),
            start,
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
    /// Before anything is reserved or read, the repair must pass `offer_authorized`.
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
        let scope = CheckpointTarget::Studio(target);
        if self.offer_refused(server, scope, repair) || !Self::offer_authorized(server, repair) {
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
            RepairSchedule::Scheduled | RepairSchedule::Busy | RepairSchedule::Full => true,
            RepairSchedule::Held => false,
        }
    }

    /// Flow D for a Registry bucket: the same rules as a Studio source, and the same answer. A
    /// faulted bucket blocks discovery, so the repair an owner's answer carries is the bucket's
    /// way out; it is scheduled here and applied by the job's S3 to a bucket rebuilt detached,
    /// with the pair assembled there. `failure_target` is the discovery's Studio target, if any.
    ///
    /// Whether the bucket already owes exactly this repair's replacement is read from the
    /// retained prepared provider only, never from a restore under custody. When that provider is
    /// cold the job runs, and its S3 classification (an `AwaitingSeed` without a second write)
    /// fetches the seed instead.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn offer_registry_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        bucket: u8,
        failure_target: Option<StudioTarget>,
        repair: &ReceiptRepair,
        offered: Option<&Receipt>,
        with_pass: bool,
    ) -> bool {
        let scope = CheckpointTarget::Registry(bucket);
        if self.offer_refused(server, scope, repair) || !Self::offer_authorized(server, repair) {
            return false;
        }
        // Unknown (a cold provider) is treated as "not owed": the job then runs, and its S3
        // classification is exact.
        let owed = self
            .registry_classification(server, store, id, bucket)
            .flatten()
            .filter(|(owed, _)| owed.hash() == repair.hash());
        if let Some((owed, pair)) = owed {
            if !with_pass {
                self.await_repaired_seed(server, store, id, scope, failure_target, &owed, &pair);
            }
            return false;
        }
        let input = RepairInput::Offered {
            repair: Box::new(repair.clone()),
            offered: offered.cloned().map(Box::new),
        };
        match self.start_repair(server, store, id, scope, failure_target, input) {
            RepairSchedule::Scheduled | RepairSchedule::Busy | RepairSchedule::Full => true,
            RepairSchedule::Held => false,
        }
    }

    /// The owner resumes a held Registry decision for the bucket behind `target`. The explicit
    /// decision itself is never made here: only one already persisted at B1 is continued, as a
    /// job. Paced like the Studio step: a hold, failure or started seed fetch defers only this
    /// bucket's work, by 60 s doubling to 15 min, instead of retrying every Registry turn.
    ///
    /// Returns whether a held decision owns the bucket; the caller then gives this turn to it.
    ///
    /// Whether only the seed is missing is read from the prepared provider, which `work_registry`
    /// has just made warm and current for this bucket: exact, and no bucket restore. The owner
    /// record's B3 flag is only the fallback for an unknown provider, because it cannot tell an
    /// owed replacement from one that was installed just before a crash, while the recycle that
    /// follows the install in its own write was lost. That record still holds the decision with
    /// B3 set although the bucket owes nothing. A seed fetch there would be deferred by the held
    /// decision forever, and nothing else recycles an owner's record (review HIGH-1); the resume
    /// does, answering `AlreadyRepaired`.
    pub(super) fn resume_registry_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        bucket: u8,
    ) -> Result<bool, AppError> {
        let now = server.runtime_clock().monotonic_ms();
        let scope = CheckpointTarget::Registry(bucket);
        let Some(snapshot) = self.owner_snapshot.clone() else {
            return Ok(false);
        };
        if !server.owner_head_snapshot_is_current(store, id, &snapshot) {
            return Ok(false);
        }
        let held = server
            .sync
            .with_registry_context(|g, d, _, _| store.held_registry_repair(id, g, bucket, d));
        if self.visit_deferred(scope, now) {
            // Only the work waits. A held decision still owns the bucket's turn, so ordinary
            // bucket work does not run under it, however long this bucket is backing off; that
            // costs one bounded owner-record read per deferred turn. The exceptions are those
            // above and here: with no current owner snapshot, or an unreadable record, the
            // ordinary turn runs as it always did. An unreadable record is reported by the first
            // turn after the deferral ends. (`work_registry` asks `registry_decision_waiting`
            // first and skips the whole turn, so this branch is a backstop.)
            return Ok(matches!(held, Ok(Some(_))));
        }
        let (repair, pair) = match held {
            Ok(Some(held)) => held,
            Ok(None) => return Ok(false),
            Err(error) => {
                // An unreadable owner record is neither silent nor retried every turn.
                self.note_repair_failure(target, &error);
                self.defer_visit(scope, now);
                return Ok(false);
            }
        };
        let classification = self.registry_classification(server, store, id, bucket);
        let guessed = classification.is_none();
        let owes_only_seed = match classification {
            Some(owed) => Ok(owed.is_some_and(|(owed, _)| owed.hash() == repair.hash())),
            None => server.sync.with_registry_context(|g, d, _, _| {
                store.held_registry_repair_applied(id, g, bucket, d)
            }),
        };
        match owes_only_seed {
            Ok(true) => {
                // B2 already crossed: a resume would only flush the same bucket again. Fetch the
                // seed instead; only this bucket's own started fetch defers its next visit.
                self.await_repaired_seed(server, store, id, scope, Some(target), &repair, &pair);
                if self.seed_fetch_settled(scope, now) {
                    self.defer_visit(scope, now);
                } else if guessed && server.sync.studio_page_peers().is_empty() {
                    // As for a cold Studio source (PR #36 review HIGH-1): B3 only guessed, and
                    // there is no peer to fetch from at all, so resume and let S3 classify exactly.
                    self.schedule_resume(server, store, id, scope, Some(target), now);
                }
            }
            Ok(false) => self.schedule_resume(server, store, id, scope, Some(target), now),
            Err(error) => {
                self.note_repair_failure(target, &error);
                self.defer_visit(scope, now);
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
        let scope = CheckpointTarget::Studio(target);
        if self.visit_deferred(scope, now) {
            // Only this target waits; the next turn visits the next one on the 5 s cadence.
            return Ok(None);
        }
        let held = server
            .sync
            .with_registry_context(|g, d, _, _| store.held_studio_repair(id, g, target, d));
        let (held, held_pair) = match held {
            Ok(Some(held)) => held,
            Ok(None) => return Ok(None),
            Err(error) => {
                self.note_repair_failure(target, &error);
                self.defer_visit(scope, now);
                return Ok(None);
            }
        };
        // Is only the seed missing? A warm source answers exactly; a cold one is classified from
        // the owner record's B3 flag, which needs no source restore (review LOW-2 on 4bc753a6).
        let warm = server
            .sync
            .with_registry_context(|g, d, _, _| store.studio_source_is_warm(id, g, target, d));
        let owed = if warm {
            server
                .sync
                .with_registry_context(|g, d, _, _| store.owed_studio_repair(id, g, target, d))
                .filter(|(owed, _)| owed.hash() == held.hash())
        } else {
            let applied = server.sync.with_registry_context(|g, d, _, _| {
                store.held_studio_repair_applied(id, g, target, d)
            });
            match applied {
                // B3 alone cannot tell an owed seed from an install that landed just before a
                // crash, its recycle lost. The first visit fetches the seed, which is cheap, and
                // the router resumes on the spot if the install had landed. Once this target has
                // been deferred before (that fetch, or anything else, came to nothing), resume
                // instead: the job classifies exactly at S3, so recovering a landed install never
                // depends on some peer serving the seed (re-review LOW-1). A fetch that cannot
                // even start resumes at once (below). A seed still owed is then fetched from the
                // job's finish.
                Ok(true) if !self.repair_visits.contains_key(&scope) => {
                    Some((held.clone(), held_pair))
                }
                Ok(_) => None,
                Err(error) => {
                    self.note_repair_failure(target, &error);
                    self.defer_visit(scope, now);
                    return Ok(None);
                }
            }
        };
        if let Some((owed, pair)) = owed {
            // B2 already crossed for this decision; only its seed is missing. A resume would just
            // flush the same source again, so fetch the seed instead. Only this target's own
            // started fetch defers its next visit.
            self.await_repaired_seed(server, store, id, scope, Some(target), &owed, &pair);
            if self.seed_fetch_settled(scope, now) {
                self.defer_visit(scope, now);
                return Ok(None);
            }
            let alone = server.sync.studio_page_peers().is_empty();
            if warm || !alone {
                // Classified exactly, or a peer can be asked once the slot frees (another pass
                // out, a discovery pending). No job would help yet: the next visit asks again, at
                // the cost of a read, and a fetch that then comes to nothing defers this target,
                // after which a cold visit resumes (above).
                return Ok(None);
            }
            // Cold, B3 only guessed "just the seed", and there is no peer to fetch it from at
            // all. Nothing would ever defer this target, so every visit guessed again and never
            // resumed, and an owner alone could not recycle an install already on its own disk
            // (PR #36 review HIGH-1). Resume instead: the job classifies exactly at S3, and a
            // scheduled job defers this target like a started fetch, so this costs at most one
            // job per deferral window. Known cost (PR #36 residual LOW-2): an owner alone whose
            // seed really is still owed, and whose source keeps being evicted, reruns this whole
            // rebuild once per window (decaying to every 15 min) although nothing has changed. A
            // later fix could remember the exact `AwaitingSeed` until the peers or the source
            // change.
        }
        self.schedule_resume(server, store, id, scope, Some(target), now);
        Ok(None)
    }

    /// Schedule the owner's resume job for `scope`, a Studio target or a bucket. A started job
    /// counts like a started fetch: it defers this target's next visit, so the owner runs at most
    /// one resume job per deferral window, whatever the job's finish could start. Without it, an
    /// owed cold source whose finish cannot fetch (another pass out, a discovery pending) and
    /// which ordinary catch-up evicts again before the next visit reran capture, rebuild and a B2
    /// flush on every visit (second re-review MEDIUM). A terminal outcome clears the deferral; a
    /// hold only defers. `Busy` is another job or a full pool, retried on the ordinary cadence.
    fn schedule_resume<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        scope: CheckpointTarget,
        failure_target: Option<StudioTarget>,
        now: u64,
    ) {
        match self.start_repair(
            server,
            store,
            id,
            scope,
            failure_target,
            RepairInput::Resume,
        ) {
            RepairSchedule::Scheduled | RepairSchedule::Held => self.defer_visit(scope, now),
            RepairSchedule::Busy | RepairSchedule::Full => {}
        }
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
        self.finish_studio_repair(server, store, id, target, &repair, outcome, state, false);
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
            let scope = CheckpointTarget::Studio(target);
            self.report_repair(scope, Ok(StudioRepairOutcome::AlreadyRepaired));
            return Ok(None);
        };
        let (outcome, state) = server.resume_studio_fault_repair(
            store, id, target, &snapshot, &repair, &pair, None, budget,
        )?;
        self.finish_studio_repair(server, store, id, target, &repair, outcome, state, false);
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
            self.report_repair(scope, Ok(StudioRepairOutcome::AlreadyRepaired));
            return Ok(None);
        }
        let Some(pair) = server.sync.with_registry_context(|g, d, _, _| {
            store.studio_repair_evidence(id, g, target, d, repair, offered)
        }) else {
            self.hold_unverifiable(scope, repair, now);
            return Ok(None);
        };
        let (outcome, state) =
            server.apply_studio_fault_repair(store, id, target, repair, &pair, None, budget)?;
        self.finish_studio_repair(server, store, id, target, repair, outcome, state, true);
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
        self.finish_studio_repair(server, store, id, target, repair, outcome, state, false);
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
    /// persistent hold, and fetch the selected seed when a replacement is owed. A hold on an
    /// `offered` repair holds that repair, never the target (see `hold_offer`).
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
        offered: bool,
    ) {
        let scope = CheckpointTarget::Studio(target);
        let now = server.runtime_clock().monotonic_ms();
        let phase = state.phase();
        if self.target == Some(target) && self.pass.is_some() {
            // A page fetched before this transaction is stale against the source it wrote. A
            // retarget or replacement leaves that source not Open or replaced, and saving the page
            // then would be refused with its pass already Paused, which pauses all catch-up. S3
            // returns before the page step, so drop the page here; a later pass fetches against
            // the repaired source. Every other path that rewrites a source drops it the same way.
            self.pass = None;
        }
        server
            .sync
            .with_registry_context(|g, d, _, _| store.retain_studio_source(g, d, state));
        self.note_repair(target, outcome, phase);
        self.report_repair(scope, Ok(outcome));
        match outcome {
            outcome if outcome.is_terminal() => {
                self.repair_backoff.remove(&scope);
                self.repair_visits.remove(&scope);
                self.remember_repair(scope, repair);
            }
            StudioRepairOutcome::AwaitingSeed => {
                // The seed fetch below and later offers carry the work from here (review M1); the
                // owner's resume classifies an owed seed exactly, so it never reruns the job. Only
                // this target's own started fetch defers its next visit.
                let pair = server
                    .sync
                    .with_registry_context(|g, d, _, _| store.owed_studio_repair(id, g, target, d));
                if let Some((owed, pair)) = pair {
                    self.await_repaired_seed(server, store, id, scope, Some(target), &owed, &pair);
                }
                if self.seed_fetch_settled(scope, now) {
                    self.defer_visit(scope, now);
                }
            }
            StudioRepairOutcome::Held(_) if offered => self.hold_offer(scope, repair.hash(), now),
            // Recovery warning, storage refusal or a hold: they need the user or the owner.
            _ => self.hold_repair(scope, now),
        }
    }

    /// Stop refetching `target`'s repaired seed and starting its automatic jobs for a while;
    /// expired holds are dropped here, so the map stays as small as the set of targets currently
    /// held. A hold is a persistent outcome, so it also defers the owner's next resume visit of
    /// that target, which is what makes repeated holds back off further (see `defer_visit`).
    pub(super) fn hold_repair(&mut self, target: CheckpointTarget, now: u64) {
        self.repair_backoff.retain(|_, until| now < *until);
        self.repair_backoff
            .insert(target, now.saturating_add(REPAIR_HOLD_BACKOFF_MS));
        self.defer_visit(target, now);
    }

    /// Defer only `target`'s next owner-resume visit. The round-robin keeps its 5 s cadence for
    /// every other target, so a held, failing or already-fetching target can never delay
    /// another's resume (plan D, fairness across held targets). A single shared cadence did: each
    /// held target visited pushed every resume back 60 s, so N held targets could hold a healthy
    /// one off for N minutes. Unlike `hold_repair` this blocks nothing but the visit: an offered
    /// repair or a fetched seed for the target still runs.
    ///
    /// The deferral doubles each time the target is deferred again after its last one ran out,
    /// from 60 s to a 15 min cap (review MEDIUM-1 on the fairness round). A fixed 60 s let total
    /// work grow with the number of held targets: K targets owing seeds no peer serves started K
    /// fetches a minute, each holding the single checkpoint slot ordinary catch-up waits on, and
    /// K targets that need the user reran K whole jobs a minute. Now each target's own rate
    /// decays, without one target's backoff ever delaying another. A deferral that arrives while
    /// one is still running belongs to the same attempt, so it can extend the wait but never
    /// doubles it. A terminal outcome or a new explicit decision clears the entry; one that has
    /// been quiet for a whole cap after it ran out is dropped here, which bounds the map to targets
    /// deferred within the last two cap windows.
    fn defer_visit(&mut self, target: CheckpointTarget, now: u64) {
        let delay = |streak: u32| {
            REPAIR_HOLD_BACKOFF_MS
                .saturating_mul(1u64 << streak.min(4))
                .min(MAX_VISIT_DEFERRAL_MS)
        };
        self.repair_visits
            .retain(|_, (until, _)| now < until.saturating_add(MAX_VISIT_DEFERRAL_MS));
        let entry = match self.repair_visits.get(&target) {
            Some(&(until, streak)) if now < until => {
                (until.max(now.saturating_add(delay(streak))), streak)
            }
            Some(&(_, streak)) => {
                let streak = streak.saturating_add(1);
                (now.saturating_add(delay(streak)), streak)
            }
            None => (now.saturating_add(delay(0)), 0),
        };
        self.repair_visits.insert(target, entry);
    }

    fn visit_deferred(&self, target: CheckpointTarget, now: u64) -> bool {
        self.repair_visits
            .get(&target)
            .is_some_and(|(until, _)| now < *until)
    }

    /// The person resolved what held `target` (a successful Acknowledge of its recovery warning).
    /// Its backoff, doubled while the hold repeated, describes the state before that, so the next
    /// visit or offer may act at once rather than up to 15 min later (re-review MEDIUM-1). Its
    /// bucket is cleared too, since the same warning can hold either; clearing only ever allows
    /// one earlier attempt, and the next hold starts again at 60 s.
    ///
    /// Only an explicit acknowledgement does this. An ordinary Read must not: the renderer reads
    /// again after every refresh notice, so a repair outcome would clear its own backoff.
    pub(in crate::studio::receiver) fn repair_user_resolved<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &Server<T, R>,
        target: StudioTarget,
    ) {
        let mut scopes = vec![CheckpointTarget::Studio(target)];
        if let Ok(bucket) = server.studio_registry_bucket(target) {
            scopes.push(CheckpointTarget::Registry(bucket));
        }
        for scope in scopes {
            self.repair_visits.remove(&scope);
            self.repair_backoff.remove(&scope);
        }
    }

    /// Whether `bucket`'s Registry turn belongs to a held owner decision whose work is backing
    /// off. `work_registry` asks before preparing anything (re-review LOW-3): preparing would be a
    /// detached rebuild when cold, and would evict the single warm source, for a turn the decision
    /// owns anyway. Costs one bounded owner-record read, and only while deferred.
    pub(super) fn registry_decision_waiting<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &mut Server<T, R>,
        store: &ServerStore,
        id: u64,
        bucket: u8,
        now: u64,
    ) -> bool {
        if !self.visit_deferred(CheckpointTarget::Registry(bucket), now) {
            return false;
        }
        let current = self
            .owner_snapshot
            .as_ref()
            .is_some_and(|snapshot| server.owner_head_snapshot_is_current(store, id, snapshot));
        current
            && matches!(
                server.sync.with_registry_context(
                    |g, d, _, _| store.held_registry_repair(id, g, bucket, d)
                ),
                Ok(Some(_))
            )
    }

    /// Whether `target`'s own repaired-seed pass is out: a fetch that started for it.
    fn seed_fetch_started(&self, target: CheckpointTarget) -> bool {
        self.checkpoint
            .as_ref()
            .is_some_and(|pass| pass.inner.target() == target)
    }

    /// Whether `target`'s own repaired-seed fetch is out, or it is backing off: the two cases in
    /// which its next resume visit waits. Another target's pass does not count; that one is only
    /// a queue this target waits in, and the next visit asks again.
    fn seed_fetch_settled(&self, target: CheckpointTarget, now: u64) -> bool {
        self.seed_fetch_started(target)
            || self
                .repair_backoff
                .get(&target)
                .is_some_and(|until| now < *until)
    }

    /// S3 of a Registry bucket job, for every input. `prepared` was found current a moment ago
    /// under this same custody, and each `*_prepared` transaction rechecks it against the live
    /// context and the bytes on disk before using it; otherwise these are the same transactions,
    /// with the same authority checks, that the bucket's synchronous repair called directly.
    ///
    /// `failure_target` is the Studio target the job was asked for, if any: an explicit decision
    /// always has one (its channel is checked at issuance); a bucket pass may have none.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn execute_registry<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        bucket: u8,
        failure_target: Option<StudioTarget>,
        input: RepairInput,
        prepared: PreparedRegistryRepair,
        budget: &mut crate::store::EpochStudioBudget,
    ) -> Result<Option<StudioTarget>, AppError> {
        let scope = CheckpointTarget::Registry(bucket);
        let now = server.runtime_clock().monotonic_ms();
        let offered = matches!(input, RepairInput::Offered { .. });
        let (repair, pair, outcome) = match input {
            RepairInput::Decide(request) => {
                let target = failure_target
                    .ok_or_else(|| invalid("a bucket decision is made for one document"))?;
                let snapshot = self.current_owner_snapshot(server, store, id)?;
                self.note_refresh(failure_target);
                let (repair, outcome, _) = server.issue_registry_fault_repair_prepared(
                    store, id, target, &snapshot, request, prepared, budget,
                )?;
                // The seed fetch needs the decision's pair, which the bounded owner record holds.
                let pair = if outcome == StudioRepairOutcome::AwaitingSeed {
                    server
                        .sync
                        .with_registry_context(|g, d, _, _| {
                            store.held_registry_repair(id, g, bucket, d)
                        })
                        .ok()
                        .flatten()
                        .filter(|(held, _)| held.hash() == repair.hash())
                        .map(|(_, pair)| pair)
                } else {
                    None
                };
                (repair, pair, outcome)
            }
            RepairInput::Resume => {
                let snapshot = self.current_owner_snapshot(server, store, id)?;
                let Some((repair, pair)) = server.sync.with_registry_context(|g, d, _, _| {
                    store.held_registry_repair(id, g, bucket, d)
                })?
                else {
                    // Completed or recycled since S1; nothing is owed.
                    self.report_repair(scope, Ok(StudioRepairOutcome::AlreadyRepaired));
                    return Ok(None);
                };
                self.note_refresh(failure_target);
                let (outcome, _) = server.resume_registry_bucket_repair_prepared(
                    store, id, bucket, &snapshot, &repair, &pair, None, prepared, budget,
                )?;
                (repair, Some(pair), outcome)
            }
            RepairInput::Offered { repair, offered } => {
                match prepared.offered_evidence(&repair, offered.as_deref()) {
                    OfferedRepairEvidence::Terminal => {
                        // Terminal here: later answers carrying it cost no further job.
                        self.remember_repair(scope, &repair);
                        self.report_repair(scope, Ok(StudioRepairOutcome::AlreadyRepaired));
                        return Ok(None);
                    }
                    OfferedRepairEvidence::Unverifiable => {
                        self.hold_unverifiable(scope, &repair, now);
                        return Ok(None);
                    }
                    OfferedRepairEvidence::Pair(pair) => {
                        self.note_refresh(failure_target);
                        let (outcome, _) = server.apply_registry_bucket_repair_prepared(
                            store, id, bucket, &repair, &pair, None, prepared, budget,
                        )?;
                        (*repair, Some(*pair), outcome)
                    }
                }
            }
            RepairInput::Replace { repair, pair, seed } => {
                // The owner installs only as a resume would, through a current durable snapshot;
                // a peer through Flow A, which refuses the owner.
                let owner = server.sync.with_registry_context(|g, d, _, _| {
                    g.designated_committer() == Some(d.device_id())
                });
                let snapshot = if owner {
                    Some(self.current_owner_snapshot(server, store, id)?)
                } else {
                    None
                };
                self.note_refresh(failure_target);
                let (outcome, _) = if let Some(snapshot) = snapshot {
                    server.resume_registry_bucket_repair_prepared(
                        store,
                        id,
                        bucket,
                        &snapshot,
                        &repair,
                        &pair,
                        Some(seed.as_slice()),
                        prepared,
                        budget,
                    )?
                } else {
                    server.apply_registry_bucket_repair_prepared(
                        store,
                        id,
                        bucket,
                        &repair,
                        &pair,
                        Some(seed.as_slice()),
                        prepared,
                        budget,
                    )?
                };
                if outcome.is_terminal()
                    && self
                        .registry_watch
                        .as_ref()
                        .is_some_and(|watch| watch.bucket == bucket)
                {
                    // The replacement opened a new epoch for this bucket. Old concrete-epoch pages
                    // must not enter it; another bucket's tail is left alone (S3 runs any time).
                    self.reset_registry_tail(server, now);
                }
                (*repair, Some(*pair), outcome)
            }
        };
        self.finish_registry_repair(
            server,
            store,
            id,
            bucket,
            failure_target,
            &repair,
            pair,
            outcome,
            offered,
        );
        Ok(None)
    }

    /// Settlement hears of a bucket write only when one is about to run: an offer this bucket
    /// already finished, one it cannot verify, or a resume with nothing held writes nothing and
    /// must not ask the renderer to refresh (review LOW-2).
    fn note_refresh(&mut self, failure_target: Option<StudioTarget>) {
        if let Some(target) = failure_target {
            self.settlement
                .note(target, StudioSettlementState::RefreshRequired);
        }
    }

    /// The Registry counterpart of `finish_studio_repair`. A bucket has no settlement phase of
    /// its own and no warm cache to retain into; the prepared page provider is what goes stale. A
    /// pending Registry page for this bucket needs no drop here: `persist_registry_page` already
    /// discards a page whose physical epoch or phase a local write superseded.
    ///
    /// This bucket waits on its own, never on another's: a hold sets its backoff, which
    /// `start_repair` honours, and an owed seed is fetched, after which only this bucket's next
    /// resume visit is deferred (`defer_visit`, doubling while it repeats), exactly as for a
    /// Studio target.
    #[allow(clippy::too_many_arguments)]
    fn finish_registry_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        bucket: u8,
        failure_target: Option<StudioTarget>,
        repair: &ReceiptRepair,
        pair: Option<[Receipt; 2]>,
        outcome: StudioRepairOutcome,
        offered: bool,
    ) {
        let scope = CheckpointTarget::Registry(bucket);
        let now = server.runtime_clock().monotonic_ms();
        self.invalidate_registry_provider();
        self.report_repair(scope, Ok(outcome));
        if outcome.is_terminal() {
            self.repair_backoff.remove(&scope);
            self.repair_visits.remove(&scope);
            self.remember_repair(scope, repair);
            return;
        }
        match (outcome, pair) {
            (StudioRepairOutcome::AwaitingSeed, Some(pair)) => {
                self.await_repaired_seed(server, store, id, scope, failure_target, repair, &pair);
                if self.seed_fetch_settled(scope, now) {
                    self.defer_visit(scope, now);
                }
            }
            (StudioRepairOutcome::AwaitingSeed, None) => {}
            (StudioRepairOutcome::Held(_), _) if offered => {
                self.hold_offer(scope, repair.hash(), now);
            }
            // Recovery warning, storage refusal or a hold: they need the user or the owner.
            _ => self.hold_repair(scope, now),
        }
    }

    /// Route a checkpoint pass before the ordinary installer, so a repair is never an installer
    /// error that pauses all catch-up:
    /// - the source owes a repair and this pass has fetched exactly its selected seed: hand the
    ///   seed to a repair job, whose S3 installs it through the repair transaction itself, with
    ///   typed RecoveryPending/StorageRefused and owner recycling (the owner only through a
    ///   current durable snapshot);
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
        let owed = match target {
            CheckpointTarget::Studio(studio) => server
                .sync
                .with_registry_context(|g, d, _, _| Ok(store.owed_studio_repair(id, g, studio, d))),
            CheckpointTarget::Registry(bucket) => {
                // Registry reconstruction is detached and already retained by the page provider,
                // which the caller prepared for this bucket. Rebuilding it here, under actor/store
                // custody, would defeat that boundary. Unknown classification is not "no repair":
                // a missing/cold/stale provider must defer rather than fall through to ordinary
                // installation.
                let classification = match self.registry_provider.as_mut() {
                    Some(provider) => {
                        server.prepared_registry_owed_repair(store, id, bucket, provider)
                    }
                    None => Ok(None),
                };
                match classification {
                    Ok(Some(owed)) => Ok(owed),
                    Ok(None) => {
                        self.retry_discovery(now);
                        return Ok(Some(None));
                    }
                    Err(error) => Err(error),
                }
            }
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
            // The owner installs only as a resume would, through a current durable snapshot. S3
            // checks it again, but a snapshot already stale here would cost a whole job, the
            // fetched seed and a held target before saying so (review LOW-5).
            let owner = server.sync.with_registry_context(|g, d, _, _| {
                g.designated_committer() == Some(d.device_id())
            });
            if owner {
                if let Err(stale) = self.current_owner_snapshot(server, store, id) {
                    self.note_repair_failure_for(failure_target, &stale);
                    self.retry_discovery(now);
                    return Ok(Some(None));
                }
            }
            // The seed is taken from the pass only if its selection was made under this device's
            // observed tenure. The install is the job's S3, on a source or bucket rebuilt
            // detached; the owner goes through its current durable snapshot there.
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
            if self.start_repair(server, store, id, target, failure_target, input)
                == RepairSchedule::Busy
            {
                // Another repair job: keep the fetched seed rather than fetch it again, and look
                // again shortly. A full pool is not this: it may be this actor's own overlay or
                // handoff waiting for a slot, which a parked pass would hold back (it blocks
                // `replay_ready`), so that case drops the pass below.
                self.checkpoint = Some(pass);
                self.checkpoint_retry = now.saturating_add(1_000);
                return Ok(Some(None));
            }
            self.retry_discovery(now);
            return Ok(Some(None));
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
                self.resume_landed_install(server, store, id, target, failure_target);
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

    /// The router has just found this source owing nothing while a held decision defers the
    /// install. The source was prepared for this pass a moment ago (a Studio source is warm, and
    /// a bucket was classified from its current provider), so this is exact. If this device is
    /// the owner and its record says B3, the replacement was installed and only the recycle
    /// after it was lost to a crash. Resume now. Waiting for the owner's next visit relied on
    /// the single warm Studio cache surviving 60 s, while ordinary catch-up prepares another
    /// target every few seconds. A cold visit then reads B3 alone, fetches a seed and lands
    /// here again, and recovery waits on luck (review MEDIUM-2 on the fairness round). The
    /// resume answers `AlreadyRepaired` and recycles. One fetched pass leads to at most one
    /// such start, and `start_repair` honours this target's backoff.
    fn resume_landed_install<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: CheckpointTarget,
        failure_target: Option<StudioTarget>,
    ) {
        if self.current_owner_snapshot(server, store, id).is_err() {
            // Without a current durable snapshot S3 would refuse, after a whole capture and
            // rebuild, and hold the target; the owner's visit retries once it has one.
            return;
        }
        let landed = server.sync.with_registry_context(|g, d, _, _| {
            if g.designated_committer() != Some(d.device_id()) {
                return Ok(false);
            }
            match target {
                CheckpointTarget::Studio(studio) => {
                    store.held_studio_repair_applied(id, g, studio, d)
                }
                CheckpointTarget::Registry(bucket) => {
                    store.held_registry_repair_applied(id, g, bucket, d)
                }
            }
        });
        if matches!(landed, Ok(true)) {
            let _ = self.start_repair(
                server,
                store,
                id,
                target,
                failure_target,
                RepairInput::Resume,
            );
        }
    }
}
