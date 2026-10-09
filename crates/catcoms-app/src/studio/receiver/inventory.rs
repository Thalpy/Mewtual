//! C-3 runtime adoption: the one shared, turn-based inventory job for background owners.
//!
//! Design `docs/GATE4-AGENT-1-C3-RUNTIME.md`, sections 2.2, 4, 11 and 12. A background owner that
//! needs a Studio budget asks for it in its own turn; this steps the shared job inside that turn,
//! under the visit's one deadline, and the owner whose turn completes the job gets the budget,
//! minted in the same call. No owner owns the job and there is no queue: fairness is the
//! receiver's existing turn allocation, which is what keeps authoritative catch-up first.
//!
//! What this type guarantees, and the review finding each answers:
//! - **A permit before any step that can park** (H4): a step is taken only with a permit from
//!   the shared preparation pool in hand, so a parked body is never resident outside the pool's
//!   accounting. If no permit is free the job does not step at all.
//! - **A job whose body is out for validation cannot be stepped** (M7, N-L1): it is held in a
//!   wrapper with no stepping method until the result is installed, so a write that lands while
//!   the worker runs restarts nothing and a stale result cannot meet a new cursor.
//! - **Results are routed by token** (H3): a result or cancellation for any token other than the
//!   one this runtime is waiting on is ignored. Tokens are never reused, so the result for a job
//!   released while its body was out can never match a later job.
//! - **Results are installed under custody**: a result arrives outside the vault lease, so it is
//!   held and installed at the start of the next turn, which is also where a validation error is
//!   surfaced and the job dropped (M7).
//! - **Instability backs off** (L1): 30 s doubling to 300 s, reset by a completed inventory. A
//!   store-wide condition, so it never becomes a per-target hold, and one owner giving up on the
//!   job (`abandon_job`) does not lift it for the others. An owner refused with `Unstable` must
//!   have its own way forward; it is not told when to come back.
//! - **Identity and idleness** (M6, N-L6): a mount or numeric-server change drops the job, and a
//!   job no owner has asked for in [`IDLE_MS`] is dropped, closing its directory handle. Idleness
//!   is measured in time, not visits: every inbound packet is a visit, so under sustained gossip a
//!   visit count can run out between two of an owner's paced turns and drop a job it still wants,
//!   which restarts it every time and is a livelock for the owner.
//! - **Own writes are not charged** (N-M1), up to [`OWN_RESTARTS`] per job, and neither is the
//!   refresh of a cursor that had done no work. Proved by token identity, not counted: the runtime
//!   marks the vault's inventory token at the end of each visit and compares it at the start of the
//!   next, and a job is only refreshed uncharged if no visit since its cursor began has found the
//!   token moved in between. This softens own-write restarts; it does not make the job immune to
//!   them. Under writes between most turns (sustained receive, a user saving steadily, another
//!   actor) the job can still reach `Unstable`, because the uncached families are re-validated on
//!   every fresh cursor. The owner must therefore have a way forward that does not depend on this
//!   job completing: replay's is its synchronous fallback (`receiver/replay.rs`).
//! - **Only a parked body is pending work** for the driver. Every other state is consumed by an
//!   owner's turn, and owners are paced by their own schedules; advertising a state no turn this
//!   visit can take would hold the driver at its active cadence until the owner's next turn.
use super::*;
use crate::store::{
    EpochInventoryJob, EpochInventoryOutcome, EpochInventoryProfile, EpochInventoryQuiet,
    EpochInventoryStep, EpochStudioBudget, ParkedEpochRecord, ValidatedEpochRecord,
};
use catcoms_mls::ServerGroup;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[cfg(test)]
mod tests;

/// The first backoff after the restart budget is spent.
const BACKOFF_FIRST_MS: u64 = 30_000;
/// The cap the backoff doubles up to.
const BACKOFF_MAX_MS: u64 = 300_000;
/// How long a job may sit unwanted before it is dropped. Well above replay's ordinary spacing
/// between turns (at most once a second on alternate background turns, and about six seconds in
/// a quiet actor that only the native five-second idle wake visits). An owner whose turns are
/// further apart than this, for example because catch-up holds `replay_ready()` false, has its job
/// dropped and starts again, which is why the owner bounds its own patience separately.
const IDLE_MS: u64 = 30_000;
/// Uncharged restarts one job may take for this actor's own writes, on top of the store's own
/// [`crate::store::MAX_INVENTORY_RESTARTS`]. Bounded so the actor's steady writes cannot keep one
/// job restarting, and detached validations churning, forever.
const OWN_RESTARTS: usize = 3;
/// How long one owner's turn may step the shared job. The same order as the H3 signing slice,
/// and like it an experiment configuration until measurement 13.7 calibrates the classifier; the
/// step's one-entry minimum means a visit can overrun it by one entry's work.
pub(super) const INVENTORY_SLICE_MS: u64 = 250;

/// A job held while its parked body is validated elsewhere. Deliberately offers no way to step it.
struct Held(Box<EpochInventoryJob>);

/// The job and the records are boxed so this enum, which lives in the receiver for its whole
/// life, is a few words in every state rather than the size of its largest one.
#[derive(Default)]
enum State {
    #[default]
    Idle,
    Stepping(Box<EpochInventoryJob>),
    /// A body has been taken from the cursor and is waiting for this visit's detach. It carries
    /// the permit that was reserved before the step that parked it.
    Parked {
        job: Box<EpochInventoryJob>,
        body: Box<ParkedEpochRecord>,
        permit: OwnedSemaphorePermit,
    },
    /// The body is out with the background worker.
    Validating {
        job: Held,
        token: u64,
    },
    /// The worker's result arrived; it is installed at the start of the next turn.
    Installing {
        job: Held,
        result: Box<Result<ValidatedEpochRecord, AppError>>,
    },
    Backoff {
        until_ms: u64,
    },
}

/// What one owner's turn got.
pub(super) enum InventoryTurn {
    /// A budget minted in this call from an inventory finished in this call. Use it in this visit.
    Ready(Box<EpochStudioBudget>),
    /// Not this visit: the job stepped and has more to do, is waiting on a detached validation,
    /// or could not take a permit. The owner keeps its own work and asks again on its next turn.
    NotYet,
    /// The vault would not hold still for a whole scan; the runtime is backing off and refuses
    /// every turn until the backoff ends. The owner needs its own way forward meanwhile.
    Unstable,
}

/// The parked body and its permit, handed to the background worker with the token its result
/// must carry back. `pub(crate)` only because the detached-job enum it travels in is.
pub(crate) struct InventoryDetach {
    pub(super) body: Box<ParkedEpochRecord>,
    pub(super) permit: OwnedSemaphorePermit,
    pub(super) token: u64,
}

#[derive(Default)]
pub(super) struct InventoryRuntime {
    state: State,
    context: Option<(Arc<()>, u64)>,
    delay_ms: u64,
    next_token: u64,
    /// When an owner last asked for a budget, for the idle drop.
    last_turn_ms: u64,
    /// The vault's inventory token as this actor left it at the end of its last visit.
    quiet: Option<EpochInventoryQuiet>,
    /// A write landed between two of this actor's visits since the current cursor began, so an
    /// overtaken cursor cannot be blamed on this actor alone. A visit that finds no mark at all
    /// sets it too, having no proof either way.
    foreign: bool,
    /// Uncharged restarts the current job has taken.
    own_restarts: usize,
}

impl InventoryRuntime {
    /// Once per receiver visit, before any owner turn: drop a job that belongs to another mount
    /// or numeric server, drop a job no owner has asked for in [`IDLE_MS`], and clear a backoff
    /// that has run out.
    ///
    /// A job whose body is out (`Validating`) is left alone: its worker holds the permit and will
    /// report back, and releasing it here would only turn that report into a dropped stale result.
    pub(super) fn lifecycle(&mut self, store: &ServerStore, id: u64, now: u64) {
        // The foreign-write check is not here but in `begin_visit`, which runs before anything
        // in the visit (this lifecycle step included) can write.
        let mount = store.registry_mount();
        if self
            .context
            .as_ref()
            .is_some_and(|(m, s)| !Arc::ptr_eq(m, &mount) || *s != id)
        {
            self.release();
        }
        self.context = Some((mount, id));
        match &self.state {
            State::Stepping(_) | State::Installing { .. }
                if now.saturating_sub(self.last_turn_ms) > IDLE_MS =>
            {
                self.state = State::Idle;
            }
            State::Backoff { until_ms } if now >= *until_ms => self.state = State::Idle,
            _ => {}
        }
    }

    /// One owner's turn. The owner has already re-derived its own preconditions in this visit;
    /// on `Ready` it performs its write in this same visit, so nothing lands between mint and use.
    ///
    /// `deadline_ms` is the visit's absolute deadline, sampled once by the receiver and shared by
    /// every owner that runs in the visit.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn budget_turn(
        &mut self,
        store: &mut ServerStore,
        id: u64,
        group: &ServerGroup,
        clock: &dyn catcoms_rt::Clock,
        deadline_ms: u64,
        pool: &Arc<Semaphore>,
    ) -> Result<InventoryTurn, AppError> {
        let now = clock.monotonic_ms();
        self.last_turn_ms = now;
        match &self.state {
            State::Backoff { until_ms } if now < *until_ms => return Ok(InventoryTurn::Unstable),
            State::Backoff { .. } => self.state = State::Idle,
            State::Parked { .. } | State::Validating { .. } => return Ok(InventoryTurn::NotYet),
            State::Idle | State::Stepping(_) | State::Installing { .. } => {}
        }
        // An overtaken job gets a fresh cursor here, before the install or the step below can
        // notice and charge the store's restart budget for it, in two cases: its cursor had done
        // no work to lose (a turn that found the pool full never stepped it), or only this actor's
        // own writes can have overtaken it (N-M1, capped). Only the second case counts toward the
        // cap. A result for the old cursor is discarded with that cursor, but the store memoizes
        // it first (C-3 runtime design 14.3), which is why it is handed over rather than dropped:
        // under gossip this refresh, not the install below, is where most results are refused.
        let overtaken = match &mut self.state {
            State::Stepping(job) => Some((job, None)),
            State::Installing {
                job: Held(job),
                result,
            } => Some((job, (**result).as_ref().ok())),
            _ => None,
        };
        if let Some((job, pending)) = overtaken {
            let nothing_to_lose = !job.has_progress();
            if nothing_to_lose || (!self.foreign && self.own_restarts < OWN_RESTARTS) {
                match store.restart_epoch_inventory_job_uncharged(job, pending) {
                    Ok(false) => {}
                    Ok(true) => {
                        if !nothing_to_lose {
                            self.own_restarts += 1;
                        }
                        // A fresh cursor owes nothing to writes before it began.
                        self.foreign = false;
                        self.state = match std::mem::take(&mut self.state) {
                            State::Stepping(job) | State::Installing { job: Held(job), .. } => {
                                State::Stepping(job)
                            }
                            _ => unreachable!("matched just above"),
                        };
                    }
                    Err(error) => {
                        self.state = State::Idle;
                        return Err(error);
                    }
                }
            }
        }
        // A result that arrived since the last turn is installed first, under this custody.
        // Every failure leaves the state Idle: `mem::take` has already emptied it.
        if matches!(self.state, State::Installing { .. }) {
            let State::Installing {
                job: Held(mut job),
                result,
            } = std::mem::take(&mut self.state)
            else {
                unreachable!("checked above");
            };
            match store.install_validated_job_record(&mut job, (*result)?)? {
                EpochInventoryStep::Unstable => return Ok(self.back_off(now)),
                EpochInventoryStep::Restarted => {
                    self.foreign = false;
                    self.state = State::Stepping(job);
                }
                _ => self.state = State::Stepping(job),
            }
        }
        if matches!(self.state, State::Idle) {
            self.state = State::Stepping(Box::new(
                store.begin_epoch_inventory_job_with(EpochInventoryProfile::receive())?,
            ));
            // A fresh cursor owes nothing to writes before it began.
            self.foreign = false;
            self.own_restarts = 0;
        }
        let Ok(permit) = pool.clone().try_acquire_owned() else {
            return Ok(InventoryTurn::NotYet);
        };
        let State::Stepping(mut job) = std::mem::take(&mut self.state) else {
            unreachable!("only a stepping job reaches here");
        };
        let step = loop {
            match store.drive_epoch_inventory_job(&mut job, clock, deadline_ms)? {
                // A restart found on resuming costs a fresh cursor, not the turn: nothing can
                // overtake that cursor inside this exclusive visit, so it keeps stepping while
                // time remains. Every pass round this loop is a charged restart, so the store's
                // restart budget bounds it.
                EpochInventoryStep::Restarted => self.foreign = false,
                step => break step,
            }
        };
        match step {
            EpochInventoryStep::Parked => {
                let body = store
                    .take_parked_job_record(&mut job)
                    .ok_or_else(|| invalid("a parked inventory job had no parked body"))?;
                self.state = State::Parked {
                    job,
                    body: Box::new(body),
                    permit,
                };
                Ok(InventoryTurn::NotYet)
            }
            EpochInventoryStep::Restarted => unreachable!("absorbed by the loop above"),
            EpochInventoryStep::Unstable => Ok(self.back_off(now)),
            EpochInventoryStep::Stepped(progress) if progress.complete => {
                drop(permit);
                match store.finish_epoch_inventory_job(*job)? {
                    EpochInventoryOutcome::Complete(inventory) => {
                        let budget = store.studio_storage_budget(id, group, &inventory)?;
                        self.delay_ms = 0;
                        Ok(InventoryTurn::Ready(Box::new(budget)))
                    }
                    EpochInventoryOutcome::Restarted(job) => {
                        self.foreign = false;
                        self.state = State::Stepping(job);
                        Ok(InventoryTurn::NotYet)
                    }
                    EpochInventoryOutcome::Unstable => Ok(self.back_off(now)),
                }
            }
            EpochInventoryStep::Stepped(_) => {
                self.state = State::Stepping(job);
                Ok(InventoryTurn::NotYet)
            }
        }
    }

    /// Take a parked body for this visit's detach. The job becomes unsteppable until the result
    /// with the returned token is installed.
    pub(super) fn take_detach(&mut self) -> Option<InventoryDetach> {
        if !matches!(self.state, State::Parked { .. }) {
            return None;
        }
        let State::Parked { job, body, permit } = std::mem::take(&mut self.state) else {
            unreachable!("checked above");
        };
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1);
        self.state = State::Validating {
            job: Held(job),
            token,
        };
        Some(InventoryDetach {
            body,
            permit,
            token,
        })
    }

    /// The detached validation's result, outside custody. Kept only for the token this runtime is
    /// waiting on, and installed at the start of the next turn; anything else, including the
    /// result for a job released while its body was out, is dropped.
    pub(super) fn complete(
        &mut self,
        token: u64,
        result: Box<Result<ValidatedEpochRecord, AppError>>,
    ) {
        if !matches!(&self.state, State::Validating { token: t, .. } if *t == token) {
            return;
        }
        let State::Validating { job, .. } = std::mem::take(&mut self.state) else {
            unreachable!("checked above");
        };
        self.state = State::Installing { job, result };
    }

    /// The validation waiter was cancelled. The worker keeps its permit until it ends; the job is
    /// dropped without charging a restart, and the next turn begins a fresh one.
    pub(super) fn cancelled(&mut self, token: u64) {
        if matches!(&self.state, State::Validating { token: t, .. } if *t == token) {
            self.state = State::Idle;
        }
    }

    /// At the very start of every receiver visit, before anything in it can write: did anyone
    /// write since this actor's last visit ended? Custody is exclusive, so a write inside a visit
    /// is this actor's and a write between visits is not. Finding the token moved taints the
    /// current cursor for good; the next fresh cursor clears it.
    ///
    /// Paired with [`Self::end_visit`] on every path through `run`, early returns included: a
    /// visit that marked at its end without checking at its start would launder the gap before it
    /// into a quiet one.
    pub(super) fn begin_visit(&mut self, store: &ServerStore) {
        if !self
            .quiet
            .as_ref()
            .is_some_and(|mark| store.epoch_inventory_quiet_since(mark))
        {
            self.foreign = true;
        }
    }

    /// At the end of every receiver visit, success or error: mark the vault's inventory token as
    /// this actor left it, so the next visit can tell whether anyone else wrote in between. Any
    /// write this actor makes outside `run` is unbracketed, and so is seen by the next visit as a
    /// moved token, which charges a restart: the safe direction. That covers most control
    /// requests, and the preparation writes the `Apply` and `ApplyOverlayCopy` controls make
    /// before they call `run`.
    pub(super) fn end_visit(&mut self, store: &ServerStore) {
        self.quiet = Some(store.epoch_inventory_quiet_mark());
    }

    /// One owner gives up on the job (replay's patience): drop the job and whatever it holds,
    /// but keep a backoff. The backoff is a store-wide judgement that the vault will not hold
    /// still, so one owner's impatience must not lift it for every other owner.
    pub(super) fn abandon_job(&mut self) {
        if !matches!(self.state, State::Backoff { .. }) {
            self.state = State::Idle;
        }
    }

    /// Pause, lock or a changed context: drop the job and its plaintext now. A result still out
    /// for a released job carries a token nothing waits on any more, so it is dropped on arrival.
    pub(super) fn release(&mut self) {
        self.state = State::Idle;
    }

    /// Work the very next visit can do without any owner: a parked body waiting for its detach,
    /// which holds a pool permit and authenticated plaintext until it goes.
    ///
    /// Deliberately nothing else. A result waiting to be installed, a job with more to step and
    /// a backoff that has run out are all consumed only by an owner's turn, and each owner already
    /// reports its own turn as pending when it is due. Reporting them here as well would make the
    /// driver revisit at its active cadence while the owner's pacing refuses every visit. There
    /// is no wake for the same reason: an owner told `Unstable` takes its own way forward.
    pub(super) fn pending(&self) -> bool {
        matches!(self.state, State::Parked { .. })
    }

    /// The state's name, for receiver-level tests that must see which state a visit left.
    #[cfg(test)]
    pub(super) fn state_for_test(&self) -> &'static str {
        match self.state {
            State::Idle => "idle",
            State::Stepping(_) => "stepping",
            State::Parked { .. } => "parked",
            State::Validating { .. } => "validating",
            State::Installing { .. } => "installing",
            State::Backoff { .. } => "backoff",
        }
    }

    /// Whether the last visit's end mark is the vault's current token.
    #[cfg(test)]
    pub(super) fn marked_for_test(&self, store: &ServerStore) -> bool {
        self.quiet
            .as_ref()
            .is_some_and(|mark| store.epoch_inventory_quiet_since(mark))
    }

    /// Put the runtime into the backoff a restart storm ends in.
    #[cfg(test)]
    pub(super) fn back_off_for_test(&mut self, now: u64) {
        self.back_off(now);
    }

    /// When the current backoff ends, if the runtime is in one.
    #[cfg(test)]
    fn backoff_until_for_test(&self) -> Option<u64> {
        match self.state {
            State::Backoff { until_ms } => Some(until_ms),
            _ => None,
        }
    }

    fn back_off(&mut self, now: u64) -> InventoryTurn {
        self.delay_ms = if self.delay_ms == 0 {
            BACKOFF_FIRST_MS
        } else {
            self.delay_ms.saturating_mul(2).min(BACKOFF_MAX_MS)
        };
        self.state = State::Backoff {
            until_ms: now.saturating_add(self.delay_ms),
        };
        InventoryTurn::Unstable
    }
}
