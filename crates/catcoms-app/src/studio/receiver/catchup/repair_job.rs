//! Design 10.3's repair job, in four stages.
//!
//! - **S1, custody, bounded.** A slot of the shared four-slot preparation pool and the target's
//!   live claim are reserved before any body is read. Then the bounded authenticated plaintext is
//!   captured exactly as ordinary source preparation captures it.
//! - **S2, detached.** The source is rebuilt from that capture, owning only the capture, the slot
//!   and the claim: no store, Server, vault key or MLS secret.
//! - **S3, custody.** The rebuild is rechecked: actor, owner, MLS epoch, group, plaintext digest
//!   and physical size against what is on disk now. The unchanged store transaction then runs on
//!   it: issuance, resume, Flow D application or the owed replacement. A Studio rebuild is
//!   installed into the warm source cache that transaction reads; a Registry bucket has no such
//!   cache, so its rebuild is handed to the transaction, which rechecks it again before use.
//!   Every outcome, hold, CORE-007 refusal and owner recycle is therefore the store's, exactly as
//!   it was when the transaction ran directly.
//! - **S4.** The ownership drops after the commit attempt, wherever its last owner is. A cancelled
//!   waiter never releases a slot or a claim that a running worker still owns.
//!
//! One job per actor. The job is detached at the top of the `detach` chain and committed early in
//! `run`, never behind `replay_ready()`: catch-up for a claimed target waits for this job, so
//! parking the job behind that catch-up would wait on itself.
use super::*;
use crate::store::StudioRepairRequest;
use crate::studio::StudioRepairStart;
use catcoms_replication::{Receipt, ReceiptRepair};
use std::sync::Weak;
use zeroize::Zeroizing;

/// A full pool is retried on a flat cadence that charges no target.
const REPAIR_CAPACITY_RETRY_MS: u64 = 2_000;
/// A rebuild that went stale before S3 (an edit, a page, an MLS commit) is not a fault. That
/// target waits this long and is then captured afresh.
const REPAIR_STALE_RETRY_MS: u64 = 5_000;

/// Admission for one repair job: a slot of the shared pool and the target's live claim. Held only
/// for their `Drop`. Both move into the worker for S2, so they end together, where it ends.
pub(crate) struct RepairOwnership {
    _permit: OwnedSemaphorePermit,
    _claim: Arc<()>,
}

/// The runtime half of design 10.3's target claim (AG3-DES-043). A peer applying a distributed
/// repair has no owner record, so before B2 nothing durable represents its job; this does. Only
/// `Weak` handles are kept, so a claim ends exactly where its last `Arc` drops, including inside a
/// worker that outlived the job which started it.
#[derive(Default)]
pub(super) struct RepairClaims {
    live: Vec<(CheckpointTarget, Weak<()>)>,
}

impl RepairClaims {
    fn claim(&mut self, target: CheckpointTarget) -> Option<Arc<()>> {
        self.live.retain(|(_, claim)| claim.strong_count() > 0);
        if self.claimed(target) {
            return None;
        }
        let claim = Arc::new(());
        self.live.push((target, Arc::downgrade(&claim)));
        Some(claim)
    }

    /// Whether a live repair job owns `target` between S1 and S4. Consulted by ordinary installs,
    /// page receive, owner rotation, preparation, foreground Apply and Registry maintenance.
    ///
    /// Not every writer consults it. Gossip ingest into an already warm source, Flow S, and the
    /// handoff's commit (H5) and Flow R's commit (R3) write the Studio source without asking.
    /// Nothing is corrupted: S3 rechecks the source's digest against disk, so any of those
    /// writes makes the rebuild stale and the job writes nothing (`repair_stale`, a 5 s retry),
    /// and R3 has its own stamp fallback. The cost is the wasted rebuild. Agent 1 records a
    /// `repair_claimed` skip at the handoff probe's selection, and a hold at `handoff_commit`,
    /// as a follow-up on its side.
    pub(super) fn claimed(&self, target: CheckpointTarget) -> bool {
        self.live
            .iter()
            .any(|(held, claim)| *held == target && claim.strong_count() > 0)
    }
}

/// What S3 does. Each arm is one call into an existing transaction.
pub(crate) enum RepairInput {
    /// The owner's explicit decision. Issuance, B1 and Flow A all happen at S3.
    Decide(StudioRepairRequest),
    /// The owner continues the decision it persisted at B1.
    Resume,
    /// A distributed repair from an authenticated answer, and the receipt that answer offered.
    /// The pair is assembled at S3 from the rebuilt source, never from this value alone.
    Offered {
        repair: Box<ReceiptRepair>,
        offered: Option<Box<Receipt>>,
    },
    /// The selected seed for the replacement this source owes, extracted from a pass whose
    /// selection was made under this device's observed tenure before the job was asked for.
    Replace {
        repair: Box<ReceiptRepair>,
        pair: Box<[Receipt; 2]>,
        seed: Zeroizing<Vec<u8>>,
    },
}

impl RepairInput {
    /// The same work, so a repeated request for it answers `Scheduled` rather than `Busy`.
    fn same_work(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Decide(a), Self::Decide(b)) => a == b,
            (Self::Resume, Self::Resume) => true,
            (Self::Offered { repair: a, .. }, Self::Offered { repair: b, .. })
            | (Self::Replace { repair: a, .. }, Self::Replace { repair: b, .. }) => {
                a.hash() == b.hash()
            }
            _ => false,
        }
    }

    fn explicit(&self) -> bool {
        matches!(self, Self::Decide(_))
    }

    /// The offered repair's hash, if this job applies one. Whatever stops an offered repair is
    /// charged to that repair, never to its target (`hold_offer`): an offer is untrusted input.
    fn offered_repair(&self) -> Option<[u8; 32]> {
        match self {
            Self::Offered { repair, .. } => Some(repair.hash()),
            _ => None,
        }
    }

    /// Keep evidence that a repeated request for the same work brings and this job lacks. An
    /// offered repair's pair is assembled at S3 from the rebuilt source plus the receipt its
    /// answer offered, and the same signed repair can arrive again, while this job runs, with the
    /// receipt the first answer lacked. Dropping it left S3 unable to verify a repair it now could,
    /// holding that repair for a minute (PR #36 review MEDIUM-1). Nothing here is trusted: S3
    /// uses the receipt only if the repair names its hash, and the transaction checks it all. A
    /// receipt the repair names is never replaced. `Replace` keeps its first seed: that seed was
    /// already checked against the selected receipt under this device's observed tenure.
    fn absorb(&mut self, other: Self) {
        if let (
            Self::Offered { repair, offered },
            Self::Offered {
                offered: Some(new), ..
            },
        ) = (self, other)
        {
            let named = |receipt: &Receipt| repair.receipt_hashes.contains(&receipt.hash());
            if !offered.as_deref().is_some_and(named) && named(&new) {
                *offered = Some(new);
            }
        }
    }
}

/// S2's input: the bounded authenticated capture, nothing else.
pub(crate) enum RepairRebuild {
    Studio(StudioSourceCapture),
    Registry(crate::store::RegistryRepairCapture),
}

/// S2's output, parked until S3 installs or rechecks it.
pub(crate) enum RepairRebuilt {
    Studio(Box<PreparedStudioSource>),
    Registry(Box<crate::store::PreparedRegistryRepair>),
}

enum RepairStage {
    Captured(RepairRebuild, RepairOwnership),
    /// The worker owns the bundle; the job holds nothing.
    Detached,
    Ready(RepairRebuilt, RepairOwnership),
}

pub(super) struct RepairJob {
    /// Never reused within an actor. A worker can outlive its job, and a later job may have the
    /// same target; completions are matched on this, never on the target.
    token: u64,
    target: CheckpointTarget,
    failure_target: Option<StudioTarget>,
    input: RepairInput,
    stage: RepairStage,
    /// Authoring tenure and MLS epoch at S1. Either moving abandons the job.
    tenure: Option<u64>,
    mls: u64,
}

/// A finished S2, tagged with the job token it was detached for.
pub(crate) enum RepairCompletion {
    Rebuilt(u64, Result<(RepairRebuilt, RepairOwnership), AppError>),
    /// The waiter was cancelled or the worker died. Either way the bundle went with the worker.
    Cancelled(u64),
}

/// The result of asking for a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RepairSchedule {
    Scheduled,
    /// Another repair job, or a claim still owned by an abandoned job's worker.
    Busy,
    /// The shared preparation pool is full, or its flat retry has not elapsed. Unlike `Busy`
    /// this can be this actor's own other work waiting for a slot, so nothing should be parked
    /// waiting on it.
    Full,
    /// The target is backing off after a persistent hold, or its capture failed.
    Held,
}

impl RepairSchedule {
    pub(super) fn start(self) -> StudioRepairStart {
        match self {
            Self::Scheduled => StudioRepairStart::Scheduled,
            Self::Busy | Self::Full | Self::Held => StudioRepairStart::Busy,
        }
    }
}

/// The report for a job that ended before committing anything. Only automatic work reruns by
/// itself; an explicit decision persisted nothing before S3, so its person must decide again.
fn abandoned(explicit: bool, why: &str) -> AppError {
    invalid(if explicit {
        format!("abandoned: {why}; nothing was decided, decide again")
    } else {
        format!("abandoned: {why}; it will rerun")
    })
}

impl<T: MeshTransport> StudioBackgroundJob<T> {
    fn repair_rebuild(token: u64, rebuild: RepairRebuild, ownership: RepairOwnership) -> Self {
        Self::RepairRebuild(token, rebuild, ownership)
    }
}

impl RepairRebuild {
    /// S2. Runs on a blocking worker. Ownership is inside the same closure, so on failure it is
    /// released here, in the worker, the moment the rebuild becomes impossible (RT-001).
    pub(super) fn run(
        self,
        ownership: RepairOwnership,
    ) -> Result<(RepairRebuilt, RepairOwnership), AppError> {
        let rebuilt = match self {
            Self::Studio(capture) => capture
                .rebuild()
                .map(|prepared| RepairRebuilt::Studio(Box::new(prepared))),
            Self::Registry(capture) => capture
                .rebuild()
                .map(|prepared| RepairRebuilt::Registry(Box::new(prepared))),
        };
        match rebuilt {
            Ok(rebuilt) => Ok((rebuilt, ownership)),
            Err(error) => {
                drop(ownership);
                Err(error)
            }
        }
    }
}

impl CatchupRuntime {
    /// Whether a live repair job owns `target`.
    pub(in crate::studio::receiver) fn repair_claimed(&self, target: CheckpointTarget) -> bool {
        self.repair_claims.claimed(target)
    }

    /// The live job's target, for a fault view's blocker and repeated-request answers.
    pub(in crate::studio::receiver) fn repair_job_target(&self) -> Option<CheckpointTarget> {
        self.repair_job.as_ref().map(|job| job.target)
    }

    /// A job has work for the next custody visit or detach.
    pub(super) fn repair_pending(&self) -> bool {
        self.repair_job
            .as_ref()
            .is_some_and(|job| !matches!(job.stage, RepairStage::Detached))
    }

    /// The job's S2 is running on a worker.
    #[cfg(test)]
    pub(super) fn repair_detached_for_test(&self) -> bool {
        self.repair_job
            .as_ref()
            .is_some_and(|job| matches!(job.stage, RepairStage::Detached))
    }

    /// A rebuilt source is parked holding a pool slot until S3 consumes it.
    pub(super) fn repair_parked(&self) -> bool {
        self.repair_job
            .as_ref()
            .is_some_and(|job| matches!(job.stage, RepairStage::Ready(..)))
    }

    /// S1. Reserve before reading, claim, capture. Never pauses catch-up: every failure here is a
    /// hold (the target's, or only the offered repair's), surfaced through the failure slot.
    pub(super) fn start_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: CheckpointTarget,
        failure_target: Option<StudioTarget>,
        input: RepairInput,
    ) -> RepairSchedule {
        let now = server.runtime_clock().monotonic_ms();
        if let Some(job) = &mut self.repair_job {
            return if job.target == target && job.input.same_work(&input) {
                // The same work again may carry evidence the running job lacks; keep it for S3.
                job.input.absorb(input);
                RepairSchedule::Scheduled
            } else {
                RepairSchedule::Busy
            };
        }
        // A person's explicit decision is not a refetch loop; only automatic work backs off.
        if !input.explicit() && self.repair_backoff.get(&target).is_some_and(|t| now < *t) {
            return RepairSchedule::Held;
        }
        if now < self.repair_capacity_at {
            return RepairSchedule::Full;
        }
        // 10.3: the slot is reserved before the first body read, and a refusal costs nothing
        // but a flat retry.
        let Ok(permit) = self.preparation_pool().try_acquire_owned() else {
            self.repair_capacity_at = now.saturating_add(REPAIR_CAPACITY_RETRY_MS);
            return RepairSchedule::Full;
        };
        let Some(claim) = self.repair_claims.claim(target) else {
            // An abandoned job's worker still owns this target. It releases where it ends.
            return RepairSchedule::Busy;
        };
        let ownership = RepairOwnership {
            _permit: permit,
            _claim: claim,
        };
        let captured = match target {
            CheckpointTarget::Studio(studio) => server
                .sync
                .with_registry_context(|g, d, _, _| store.capture_studio_source(id, g, studio, d))
                .map(|capture| capture.map(RepairRebuild::Studio)),
            CheckpointTarget::Registry(bucket) => server
                .sync
                .with_registry_context(|g, d, _, _| {
                    store.capture_registry_repair_source(id, g, bucket, d)
                })
                .map(|capture| capture.map(RepairRebuild::Registry)),
        };
        let rebuild = match captured {
            Ok(Some(rebuild)) => rebuild,
            // A device holding no copy of this document has nothing to repair. That is ordinary
            // for automatic work, so it is only held: an offered repair holds itself, so a copy
            // that arrives later is not kept from another repair (PR #36 review MEDIUM-2); the
            // owner's own work holds the target. An explicit decision is told why.
            Ok(None) if !input.explicit() => {
                match input.offered_repair() {
                    Some(repair) => self.hold_offer(target, repair, now),
                    None => self.hold_repair(target, now),
                }
                return RepairSchedule::Held;
            }
            Ok(None) => {
                let absent = invalid("a repair never creates a source");
                self.end_repair_job(target, failure_target, None, &absent, now);
                return RepairSchedule::Held;
            }
            Err(error) => {
                let offered = input.offered_repair();
                self.end_repair_job(target, failure_target, offered, &error, now);
                return RepairSchedule::Held;
            }
        };
        if input.explicit() {
            // A new decision's outcome must never be read from an earlier job's report, and its
            // target starts a fresh resume backoff rather than the last decision's doubled one.
            self.repair_reports.remove(&target);
            self.repair_visits.remove(&target);
        }
        let tenure = server.sync.authoring_owner_tenure_start();
        let mls = server.sync.with_registry_context(|g, _, _, _| g.epoch());
        let token = self.repair_next_token;
        self.repair_next_token = self.repair_next_token.wrapping_add(1);
        self.repair_job = Some(RepairJob {
            token,
            target,
            failure_target,
            input,
            stage: RepairStage::Captured(rebuild, ownership),
            tenure,
            mls,
        });
        RepairSchedule::Scheduled
    }

    /// Hand S2 to a worker. The job keeps only its identity while the worker owns the bundle.
    pub(super) fn repair_detach<T: MeshTransport>(&mut self) -> Option<StudioBackgroundJob<T>> {
        let job = self.repair_job.as_mut()?;
        if !matches!(job.stage, RepairStage::Captured(..)) {
            return None;
        }
        let RepairStage::Captured(rebuild, ownership) =
            std::mem::replace(&mut job.stage, RepairStage::Detached)
        else {
            unreachable!("checked just above")
        };
        Some(StudioBackgroundJob::repair_rebuild(
            job.token, rebuild, ownership,
        ))
    }

    /// Route a finished S2 by token. A completion for any other job is dropped, which releases
    /// whatever it carries.
    pub(super) fn repair_complete(&mut self, completion: RepairCompletion, now: u64) {
        let token = match &completion {
            RepairCompletion::Rebuilt(token, _) | RepairCompletion::Cancelled(token) => *token,
        };
        let live = self
            .repair_job
            .as_ref()
            .is_some_and(|job| job.token == token && matches!(job.stage, RepairStage::Detached));
        if !live {
            return;
        }
        match completion {
            RepairCompletion::Rebuilt(_, Ok((rebuilt, ownership))) => {
                if let Some(job) = self.repair_job.as_mut() {
                    job.stage = RepairStage::Ready(rebuilt, ownership);
                }
            }
            RepairCompletion::Rebuilt(_, Err(error)) => {
                let job = self.repair_job.take().expect("live job");
                let offered = job.input.offered_repair();
                self.end_repair_job(job.target, job.failure_target, offered, &error, now);
            }
            RepairCompletion::Cancelled(_) => {
                let job = self.repair_job.take().expect("live job");
                let cancelled = invalid("the repair was cancelled before it finished");
                self.report_repair(job.target, Err(&cancelled));
            }
        }
    }

    /// Every way a job can end without committing reports for its target, so a fault view never
    /// shows an earlier job's outcome as this one's. A failure also holds: the target for the
    /// device's own work, and for an `offered` repair only that repair, at every stage, so an
    /// untrusted offer never backs off its document (PR #36 review MEDIUM-2).
    fn end_repair_job(
        &mut self,
        target: CheckpointTarget,
        failure_target: Option<StudioTarget>,
        offered: Option<[u8; 32]>,
        error: &AppError,
        now: u64,
    ) {
        self.note_repair_failure_for(failure_target, error);
        self.report_repair(target, Err(error));
        match offered {
            Some(repair) => self.hold_offer(target, repair, now),
            None => self.hold_repair(target, now),
        }
    }

    /// Checked every turn: a new tenure or MLS epoch abandons the job at any stage. A detached
    /// worker keeps its bundle until it ends; its completion then matches no job.
    pub(in crate::studio::receiver) fn repair_check_authority<
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        &mut self,
        server: &mut Server<T, R>,
    ) {
        let Some(job) = &self.repair_job else {
            return;
        };
        let tenure = server.sync.authoring_owner_tenure_start();
        let mls = server.sync.with_registry_context(|g, _, _, _| g.epoch());
        if job.tenure != tenure || job.mls != mls {
            let (target, explicit) = (job.target, job.input.explicit());
            self.repair_job = None;
            let moved = abandoned(explicit, "the owner tenure or MLS epoch changed");
            self.report_repair(target, Err(&moved));
        }
    }

    /// Pausing releases a job that is not detached; a detached worker keeps its bundle, and its
    /// result is released the moment it arrives (see `StudioReceiver::complete`).
    pub(in crate::studio::receiver) fn repair_release_for_pause(&mut self) {
        if self.repair_pending() {
            if let Some(job) = self.repair_job.take() {
                let paused = abandoned(job.input.explicit(), "catch-up paused");
                self.report_repair(job.target, Err(&paused));
            }
        }
    }

    /// S3 and S4. Install the rebuild (or, for a bucket, recheck and memoize it), then build the
    /// budget, run the transaction, and drop the ownership after the attempt. A failure holds this
    /// target only, or for an offered repair only that repair: an offer is untrusted input and
    /// must not hold the target it names.
    pub(super) fn repair_commit<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) -> Result<Option<StudioTarget>, AppError> {
        if !self.repair_parked() {
            return Ok(None);
        }
        let now = server.runtime_clock().monotonic_ms();
        let RepairJob {
            target,
            failure_target,
            input,
            stage,
            ..
        } = self.repair_job.take().expect("parked job");
        let RepairStage::Ready(rebuilt, ownership) = stage else {
            unreachable!("checked by repair_parked")
        };
        let offered = input.offered_repair();
        let result = self.repair_execute(server, store, id, target, failure_target, input, rebuilt);
        // S4: released only after the commit attempt returned, on success and error alike.
        drop(ownership);
        let Err(error) = result else {
            return result;
        };
        if matches!(target, CheckpointTarget::Registry(_)) {
            // A failed attempt may still have written part of the bucket. The stamp check on every
            // use refuses the old graph anyway; dropping it now also frees its pool slot.
            self.invalidate_registry_provider();
        }
        self.end_repair_job(target, failure_target, offered, &error, now);
        Ok(None)
    }

    /// The commit's budget. The install route keeps its existing budget, which saves the server
    /// snapshot first; every other route scans the inventory.
    ///
    /// Called only after the rebuild is installed (Studio) or memoized (a bucket), never before:
    /// its inventory scan must find this job's record warm. S1's capture evicted the warm Studio
    /// copy, so a budget built first validated that record inline, which the receive scan
    /// refuses for a cold record over its cold-byte limit (`STUDIO_RECEIVE_COLD_BYTES`). The
    /// rebuild S2 had already validated was then discarded and the target held (PR #27 review
    /// MEDIUM-1, the ordering Flow R's design review raised as HIGH-2). A budget that fails after
    /// the install leaves only the warm source behind, which is a cache: nothing relies on it.
    fn commit_budget<T: MeshTransport, R: CryptoRngCore>(
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        input: &RepairInput,
    ) -> Result<crate::store::EpochStudioBudget, AppError> {
        if matches!(input, RepairInput::Replace { .. }) {
            Self::budget(server, store, id)
        } else {
            Self::inventory_budget(server, store, id)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn repair_execute<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: CheckpointTarget,
        failure_target: Option<StudioTarget>,
        input: RepairInput,
        rebuilt: RepairRebuilt,
    ) -> Result<Option<StudioTarget>, AppError> {
        match (target, rebuilt) {
            (CheckpointTarget::Studio(studio), RepairRebuilt::Studio(prepared)) => {
                let installed = server.sync.with_registry_context(|g, d, _, _| {
                    store.install_prepared_studio_source(g, d, *prepared)
                })?;
                if !installed {
                    return Ok(self.repair_stale(server, target, &input));
                }
                let budget = &mut Self::commit_budget(server, store, id, &input)?;
                self.settlement
                    .note(studio, StudioSettlementState::RefreshRequired);
                match input {
                    RepairInput::Decide(request) => {
                        self.execute_decision(server, store, id, studio, request, budget)
                    }
                    RepairInput::Resume => self.execute_resume(server, store, id, studio, budget),
                    RepairInput::Offered { repair, offered } => self.execute_offered(
                        server,
                        store,
                        id,
                        studio,
                        &repair,
                        offered.as_deref(),
                        budget,
                    ),
                    RepairInput::Replace { repair, pair, seed } => self
                        .execute_replace(server, store, id, studio, &repair, &pair, &seed, budget),
                }
            }
            (CheckpointTarget::Registry(bucket), RepairRebuilt::Registry(prepared)) => {
                // A bucket has no warm cache to install into: the rebuild itself goes to the
                // transaction, which rechecks it again under this same custody. This read tells a
                // stale rebuild (ordinary, rerun soon) from a failure (held for 60 s), and memoizes
                // the current rebuild's inventory footprint for the budget below.
                let current = server.sync.with_registry_context(|g, d, _, _| {
                    store.warm_registry_repair_inventory(id, g, bucket, d, &prepared)
                })?;
                if !current {
                    return Ok(self.repair_stale(server, target, &input));
                }
                let budget = &mut Self::commit_budget(server, store, id, &input)?;
                self.execute_registry(
                    server,
                    store,
                    id,
                    bucket,
                    failure_target,
                    input,
                    *prepared,
                    budget,
                )
            }
            _ => Err(invalid("a repair rebuild does not match its target")),
        }
    }

    /// The source moved during S2. Nothing was written; automatic work captures afresh after a
    /// short wait, and an explicit decision is reported for its person to repeat. The wait is the
    /// offered repair's alone when the job applied one, so another offer or the target's owed
    /// seed never waits behind it (PR #36 residual LOW-1); otherwise it is the target's.
    fn repair_stale<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        target: CheckpointTarget,
        input: &RepairInput,
    ) -> Option<StudioTarget> {
        let now = server.runtime_clock().monotonic_ms();
        match input.offered_repair() {
            Some(repair) => self.hold_offer_for(target, repair, now, REPAIR_STALE_RETRY_MS),
            None => {
                self.repair_backoff
                    .insert(target, now.saturating_add(REPAIR_STALE_RETRY_MS));
            }
        }
        let stale = abandoned(input.explicit(), "the document changed during the repair");
        self.report_repair(target, Err(&stale));
        None
    }
}
