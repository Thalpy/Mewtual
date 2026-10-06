//! Design 10.3's repair job, in four stages.
//!
//! - **S1, custody, bounded.** A slot of the shared four-slot preparation pool and the target's
//!   live claim are reserved before any body is read. Then the bounded authenticated plaintext is
//!   captured exactly as ordinary source preparation captures it.
//! - **S2, detached.** The source is rebuilt from that capture, owning only the capture, the slot
//!   and the claim: no store, Server, vault key or MLS secret.
//! - **S3, custody.** The rebuild is installed, which rechecks actor, owner, MLS epoch, group,
//!   plaintext digest and physical size against what is on disk now. The unchanged store
//!   transaction then runs on that warm source: issuance, resume, Flow D application or the owed
//!   replacement. Every outcome, hold, CORE-007 refusal and owner recycle is therefore the
//!   store's, exactly as it was when the transaction ran directly.
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

    /// Whether a live repair job owns `target`. Consulted by every path that would otherwise
    /// install into, receive into or rotate that source while the job is between S1 and S4.
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
    /// The selected seed for the replacement this source owes, extracted at S1 from a pass whose
    /// selection was made under this device's observed tenure.
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
}

/// S2's input: the bounded authenticated capture, nothing else.
pub(crate) enum RepairRebuild {
    Studio(StudioSourceCapture),
}

/// S2's output, parked until S3 installs it.
pub(crate) enum RepairRebuilt {
    Studio(Box<PreparedStudioSource>),
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

    /// A rebuilt source is parked holding a pool slot until S3 consumes it.
    pub(super) fn repair_parked(&self) -> bool {
        self.repair_job
            .as_ref()
            .is_some_and(|job| matches!(job.stage, RepairStage::Ready(..)))
    }

    /// S1. Reserve before reading, claim, capture. Never pauses catch-up: every failure here is
    /// that target's hold, surfaced through the failure slot.
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
        if let Some(job) = &self.repair_job {
            return if job.target == target && job.input.same_work(&input) {
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
            CheckpointTarget::Registry(_) => {
                Err(invalid("Registry repair awaits its detached rebuild"))
            }
        };
        let rebuild = match captured {
            Ok(Some(rebuild)) => rebuild,
            // A device holding no copy of this document has nothing to repair. That is ordinary
            // for automatic work, so it is only held; an explicit decision is told why.
            Ok(None) if !input.explicit() => {
                self.hold_repair(target, now);
                return RepairSchedule::Held;
            }
            Ok(None) => {
                let absent = invalid("a repair never creates a source");
                self.end_repair_job(target, failure_target, &absent, now);
                return RepairSchedule::Held;
            }
            Err(error) => {
                self.end_repair_job(target, failure_target, &error, now);
                return RepairSchedule::Held;
            }
        };
        if input.explicit() {
            // A new decision's outcome must never be read from an earlier job's report.
            self.repair_reports.remove(&target);
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
                self.end_repair_job(job.target, job.failure_target, &error, now);
            }
            RepairCompletion::Cancelled(_) => {
                let job = self.repair_job.take().expect("live job");
                let cancelled = invalid("the repair was cancelled before it finished");
                self.report_repair(job.target, Err(&cancelled));
            }
        }
    }

    /// Every way a job can end without committing reports for its target, so a fault view never
    /// shows an earlier job's outcome as this one's. Failures that persist also hold the target.
    fn end_repair_job(
        &mut self,
        target: CheckpointTarget,
        failure_target: Option<StudioTarget>,
        error: &AppError,
        now: u64,
    ) {
        self.note_repair_failure_for(failure_target, error);
        self.report_repair(target, Err(error));
        self.hold_repair(target, now);
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

    /// S3 and S4. Budget first, then install the rebuild, run the transaction, and drop the
    /// ownership after the attempt. A failure holds this target only.
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
        let replace = self
            .repair_job
            .as_ref()
            .is_some_and(|job| matches!(job.input, RepairInput::Replace { .. }));
        // The install route keeps its existing budget, which saves the server snapshot first.
        let budget = if replace {
            Self::budget(server, store, id)
        } else {
            Self::inventory_budget(server, store, id)
        };
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
        let mut budget = match budget {
            Ok(budget) => budget,
            Err(error) => {
                self.end_repair_job(target, failure_target, &error, now);
                return Ok(None);
            }
        };
        let result = self.repair_execute(server, store, id, target, input, rebuilt, &mut budget);
        // S4: released only after the commit attempt returned, on success and error alike.
        drop(ownership);
        match result {
            Ok(updated) => Ok(updated),
            Err(error) => {
                self.end_repair_job(target, failure_target, &error, now);
                Ok(None)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn repair_execute<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: CheckpointTarget,
        input: RepairInput,
        rebuilt: RepairRebuilt,
        budget: &mut crate::store::EpochStudioBudget,
    ) -> Result<Option<StudioTarget>, AppError> {
        let now = server.runtime_clock().monotonic_ms();
        let (CheckpointTarget::Studio(studio), RepairRebuilt::Studio(prepared)) = (target, rebuilt)
        else {
            return Err(invalid("Registry repair awaits its detached rebuild"));
        };
        let installed = server.sync.with_registry_context(|g, d, _, _| {
            store.install_prepared_studio_source(g, d, *prepared)
        })?;
        if !installed {
            // The source moved during S2. Nothing was written; automatic work captures afresh
            // after a short wait, and an explicit decision is reported for its person to repeat.
            self.repair_backoff
                .insert(target, now.saturating_add(REPAIR_STALE_RETRY_MS));
            let stale = abandoned(input.explicit(), "the document changed during the repair");
            self.report_repair(target, Err(&stale));
            return Ok(None);
        }
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
            RepairInput::Replace { repair, pair, seed } => {
                self.execute_replace(server, store, id, studio, &repair, &pair, &seed, budget)
            }
        }
    }
}
