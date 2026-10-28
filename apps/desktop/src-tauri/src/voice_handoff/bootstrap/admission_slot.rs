//! Canonical original admission custody inside the paid physical bootstrap Row.
//! One persistent continuation task owns the original8KiB and native position.
//! Neither local Busy nor a lost IPC observer creates another task or Root mint.
use super::*;
use crate::voice_handoff::requests::negotiation_capacity::{
    Kind as CapacityKind, Request as CapacityRequest, Snapshot, State as CapacityState,
};
use catcoms_app::{VoiceAdmissionPublisher, VoiceAdmissionWork, VoiceEndpointError};
use catcoms_sync::voice_v2::admission::{
    AdmissionPrepared, AdmissionSessionObserver, RootAdmissionSession, RootHighPreclaim,
};
use catcoms_sync::voice_v2::VoiceError;

const NATIVE_TASK_BYTES: usize = 8 * 1024;
struct TaskFunding {
    _credit: Credit,
}
struct NativeCustody {
    route: Route,
    // LAST: actual per-generation state and native position return before the
    // terminal marker. Its plain funding has no Row/task/callback backedge.
    returned: TaskReturned,
}
struct TaskReturned {
    row: Arc<Row>,
    _funding: Arc<TaskFunding>,
    native: Option<crate::voice_handoff::channel::NativeHold>,
    armed: bool,
}
impl Drop for TaskReturned {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut native = self.native.take();
        {
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let slot = &mut state.admission_slot;
            slot.close();
            if !slot.owners_drained() {
                // Emergency task destruction must not abandon its still-active
                // original Work/candidate. Transfer the SAME native position;
                // Session has no Row backedge. Cleanup needs no new task/credit.
                slot.emergency_native = native.take();
            }
        }
        drop(native); // release actual position OUTSIDE State, BEFORE return
        self.row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .admission_slot
            .task_returned = true;
        self.row.admission_changed.notify_waiters();
    }
}
impl NativeCustody {
    fn native(&self) -> &crate::voice_handoff::channel::NativeHold {
        self.returned
            .native
            .as_ref()
            .expect("original native position")
    }
}

/// Negative-only synchronous recovery after the permanent task was destroyed.
/// Keep its position and original funding through all affine owner destruction.
/// No task/Root attempt/payment may be created from this value.
struct EmergencyCustody {
    route: Route,
    native: Option<crate::voice_handoff::channel::NativeHold>,
    _funding: Arc<TaskFunding>,
}
impl Drop for EmergencyCustody {
    fn drop(&mut self) {
        let mut native = self.native.take();
        {
            let mut state = self
                .route
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let slot = &mut state.admission_slot;
            if !slot.owners_drained() {
                slot.emergency_native = native.take();
            }
        }
        drop(native); // actual position return precedes clearing recovery
        self.route
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .admission_slot
            .recovery_active = false;
        self.route.row.admission_changed.notify_waiters();
    }
}
struct Worker {
    context: crate::voice_handoff::context::Context,
    custody: NativeCustody,
}
struct Completion {
    _custody: NativeCustody,
}

/// Values FIRST, exact actual-return marker LAST, including destructor unwind.
struct Reclamation {
    owners: ReclamationOwners,
    returned: ReclamationReturned,
}
struct ReclamationOwners {
    capacity: Option<RootHighPreclaim>,
    registration: Option<RootAdmissionSession>,
    work: Option<VoiceAdmissionWork>,
    unwind: bool,
}
#[cfg(test)]
impl Drop for ReclamationOwners {
    fn drop(&mut self) {
        if self.unwind {
            panic!("controlled post-detach reclamation unwind");
        }
        // Automatic field destruction follows even when this test hook unwinds.
    }
}
struct ReclamationReturned {
    route: Route,
    id: Arc<InvocationId>,
}
impl Drop for ReclamationReturned {
    fn drop(&mut self) {
        let mut state = self
            .route
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.admission_slot.is_original(&self.id) {
            state.admission_slot.reclaiming = false;
        }
    }
}
fn rpc_layout_bytes() -> usize {
    catcoms_app::voice_admission_rpc_bytes()
        + std::mem::size_of::<Publisher>()
        + std::mem::size_of::<(usize, usize, InvocationId)>()
        + std::mem::size_of::<std::sync::MutexGuard<'_, State>>()
        + std::mem::size_of::<Option<VoiceAdmissionWork>>()
        + 128
}
pub(super) fn retained_layout_bytes() -> usize {
    // Existing Row8KiB pays current and one preparing plain marker allocation.
    // Neither marker has a credit/callback/Row backedge. Funding's allocation is
    // independently counted in its original native8KiB graph below.
    2 * std::mem::size_of::<(usize, usize, InvocationId)>()
}

/// Synchronous Control stack in addition to its enclosing async service frame.
/// The independently paid native8KiB owns the actual spawned future/custody graph;
/// include the concrete prepayment/commit temporaries here rather than borrowing
/// space from a future Root invocation or the bootstrap Row's retained fee.
pub(in crate::voice_handoff) fn control_layout_bytes() -> usize {
    std::mem::size_of::<crate::voice_handoff::context::LiveContext>()
        + std::mem::size_of::<NativeCustody>()
        + std::mem::size_of::<Credit>()
        + std::mem::size_of::<Arc<TaskFunding>>()
        + std::mem::size_of::<tokio::runtime::Handle>()
        + std::mem::size_of::<tokio::sync::OwnedMutexGuard<()>>()
        + std::mem::size_of::<std::sync::MutexGuard<'_, crate::voice_authority::Coordinator>>()
        + std::mem::size_of::<std::sync::MutexGuard<'_, State>>()
        + std::mem::size_of::<Snapshot>()
        + 256 // bounded request borrows, negative validation and commit wrappers
}
fn worker_layout<F: std::future::Future<Output = Completion>>(_: fn(Worker) -> F) -> usize {
    std::mem::size_of::<F>()
        + std::mem::size_of::<Worker>()
        + std::mem::size_of::<Completion>()
        + std::mem::size_of::<tokio::task::JoinHandle<Completion>>()
        + std::mem::size_of::<crate::voice_handoff::context::LiveContext>()
        + std::mem::size_of::<Preparation>()
        + std::mem::size_of::<Finalization<'_>>()
        + std::mem::size_of::<Outcome>()
        + std::mem::size_of::<Option<RootHighPreclaim>>()
        + std::mem::size_of::<Option<RootAdmissionSession>>()
        + std::mem::size_of::<Reclamation>()
        // Old aborted task/observer backing can overlap this synchronous recovery
        // frame. TaskReturned is not evidence of allocation deallocation.
        + std::mem::size_of::<EmergencyCustody>()
        + std::mem::size_of::<tokio::task::AbortHandle>()
        + std::mem::size_of::<(usize, usize, TaskFunding)>()
        + 1024 // pinned task/observer, Notify waiter, guards and terminal-return tail
}

async fn run_worker(worker: Worker) -> Completion {
    loop {
        let notified = worker.custody.route.row.admission_changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable(); // arm BEFORE inspecting monotone state
        if let Some(work) = worker.custody.route.admission_work() {
            if !work.is_retired() {
                work.wait_retired().await;
                continue;
            }
        }
        // All fallible synchronous ownership transitions are caught OUTSIDE short
        // guards. Unwind closes/quarantines; it never authorizes another Root mint.
        let step = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let route = &worker.custody.route;
            if let Ok(Some(original)) = route.take_admission_finalization(worker.custody.native()) {
                original.finish(&worker.context);
            }
            if let Some(generation) = route.requested_admission() {
                if let Err(error) =
                    queue_requested(&worker.context, route, worker.custody.native(), generation)
                {
                    if error
                        != crate::voice_handoff::factory::Refused::Protocol(VoiceError::Capacity)
                    {
                        route.close_admission_slot();
                    }
                }
            }
        }));
        if step.is_err() {
            worker.custody.route.close_admission_slot();
        }
        if worker.custody.route.admission_drained() {
            break;
        }
        if let Some(work) = worker.custody.route.admission_work() {
            if !work.is_retired() {
                work.wait_retired().await;
                continue;
            }
        }
        // Busy/candidate restoration MUST sleep. A retained Pending predicate is
        // not a reason for synchronous retry; host cadence/Control supplies a nudge.
        notified.await;
    }
    Completion {
        _custody: worker.custody,
    }
}

fn queue_requested(
    context: &crate::voice_handoff::context::Context,
    route: &Route,
    native: &crate::voice_handoff::channel::NativeHold,
    generation: u64,
) -> Result<(), crate::voice_handoff::factory::Refused> {
    queue_requested_at(context, route, native, generation, || {})
}
fn queue_requested_at(
    context: &crate::voice_handoff::context::Context,
    route: &Route,
    native: &crate::voice_handoff::channel::NativeHold,
    generation: u64,
    before_commit: impl FnOnce(),
) -> Result<(), crate::voice_handoff::factory::Refused> {
    queue_requested_with_hooks(context, route, native, generation, before_commit, || {})
}
fn queue_requested_with_hooks(
    context: &crate::voice_handoff::context::Context,
    route: &Route,
    native: &crate::voice_handoff::channel::NativeHold,
    generation: u64,
    before_commit: impl FnOnce(),
    before_enqueue: impl FnOnce(),
) -> Result<(), crate::voice_handoff::factory::Refused> {
    use crate::voice_handoff::factory;
    use catcoms_sync::voice_v2::admission::AdmissionRpcPermit;
    let (actor, grant) = factory::native::observation_actor(context, route)?;
    let payment = AdmissionRpcPermit::reserve(
        &route.selected().resources(),
        route.selected().binding().scope,
    )?;
    let live = context.upgrade().ok_or(factory::Refused::Closed)?;
    before_commit();
    let commit = live
        .commit
        .clone()
        .try_lock_owned()
        .map_err(|_| VoiceError::Capacity)?;
    factory::require_unlocked_now(&live, route.selected())?;
    let prepared = {
        let authority = match live.authority.try_lock() {
            Ok(held) => held,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity.into()),
        };
        route.begin_admission(&live, &authority, &commit, native, generation)?
    };
    drop(commit);
    drop(live); // mailbox refusal/refund must be outside ALL guards
    if let Some(prepared) = prepared {
        let (original, generation, publisher) = prepared.publisher();
        before_enqueue(); // test barrier at actual Pending -> Actor custody, outside guards
        let queued = actor.try_queue_voice_admission(
            grant.clone(),
            original,
            generation,
            payment,
            publisher,
        );
        drop(queued); // exact canonical Work/outcome, not this observer, owns recovery
    }
    Ok(())
}

/// Stage a bounded request only. The permanent task owns actual Root admission.
/// Existing task selection can never be replaced after lost replies or termination.
pub(in crate::voice_handoff) fn register(
    context: &crate::voice_handoff::context::Context,
    route: &Route,
    generation: u64,
) -> Result<(), crate::voice_handoff::factory::Refused> {
    register_at(context, route, generation, || {})
}
fn register_at(
    context: &crate::voice_handoff::context::Context,
    route: &Route,
    generation: u64,
    before_commit: impl FnOnce(),
) -> Result<(), crate::voice_handoff::factory::Refused> {
    register_with_hooks(context, route, generation, before_commit, |_| {})
}
fn register_with_hooks(
    context: &crate::voice_handoff::context::Context,
    route: &Route,
    generation: u64,
    before_commit: impl FnOnce(),
    spawned: impl FnOnce(tokio::task::AbortHandle),
) -> Result<(), crate::voice_handoff::factory::Refused> {
    use crate::voice_handoff::factory;
    use catcoms_sync::voice_v2::admission::ADMISSION_RPC_BYTES;
    use catcoms_sync::voice_v2::resources::ResourceKind;
    if route.stage_admission_request(generation)? {
        return Ok(());
    }
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| factory::Refused::Stopped)?;
    if rpc_layout_bytes() > ADMISSION_RPC_BYTES || worker_layout(run_worker) > NATIVE_TASK_BYTES {
        return Err(VoiceError::Capacity.into());
    }
    let selected = route.selected();
    let credit = selected.resources().charge(
        selected.binding().scope.group,
        NATIVE_TASK_BYTES,
        ResourceKind::Retained,
    )?;
    let native = route.session().native_hold().map_err(|error| match error {
        Refused::Capacity => factory::Refused::Protocol(VoiceError::Capacity),
        _ => factory::Refused::Closed,
    })?;
    let funding = Arc::new(TaskFunding { _credit: credit });
    // Install the terminal-return guard BEFORE publishing task_started. Unpolled
    // future destruction or startup unwind can never leave a false live task.
    let mut custody = NativeCustody {
        route: route.clone(),
        returned: TaskReturned {
            row: route.row.clone(),
            _funding: funding.clone(),
            native: Some(native),
            armed: false,
        },
    };
    let live = context.upgrade().ok_or(factory::Refused::Closed)?;
    before_commit();
    let commit = live
        .commit
        .clone()
        .try_lock_owned()
        .map_err(|_| VoiceError::Capacity)?;
    factory::require_unlocked_now(&live, route.selected())?;
    {
        let authority = match live.authority.try_lock() {
            Ok(held) => held,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity.into()),
        };
        factory::require_current_selection(live.gate(), &authority, route.selected())?;
        live.channels
            .require_cleanup_route(route)
            .map_err(|_| factory::Refused::Closed)?;
        let mut state = route
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed
            || state.effects_closed
            || state.pending
            || state.browser_setup.is_some()
            || state
                .admission
                .as_ref()
                .is_none_or(AdmissionPin::is_cancelled)
        {
            return Err(factory::Refused::Closed);
        }
        let slot = &mut state.admission_slot;
        if slot.closed || slot.task_started {
            return Err(factory::Refused::Closed);
        }
        custody.returned.armed = true;
        slot.task_started = true;
        slot.task_funding = Some(funding.clone());
        slot.desired = generation;
        slot.requested = Some(generation);
    }
    drop(commit);
    drop(live);
    let future = run_worker(Worker {
        context: context.clone(),
        custody,
    });
    let task = runtime.spawn(future);
    spawned(task.abort_handle());
    drop(task); // observer loss never cancels/restarts the task
    Ok(())
}
impl Route {
    /// Only an already-pinned physical Session can use capacity Control. Observe
    /// creates no work; Register delegates outside State to the one paid task.
    /// Cancel is negative-only and remains available after End/whole-mode close.
    pub(in crate::voice_handoff) fn admission_capacity(
        &self,
        context: &crate::voice_handoff::context::Context,
        request: CapacityRequest,
    ) -> Result<Snapshot, Refused> {
        {
            let state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let mode = state.admission.as_ref().ok_or(Refused::Binding)?;
            if request.kind == CapacityKind::Register
                && (mode.is_cancelled() || state.pending || state.browser_setup.is_some())
            {
                return Err(Refused::Binding);
            }
            state
                .admission_slot
                .require_observable(request.generation)?;
        }
        match request.kind {
            CapacityKind::Register => {
                use crate::voice_handoff::factory::Refused as FactoryRefused;
                match register(context, self, request.generation) {
                    Ok(()) | Err(FactoryRefused::Protocol(VoiceError::Capacity)) => {}
                    Err(FactoryRefused::Protocol(VoiceError::Bounds)) => {
                        return Err(Refused::Bounds)
                    }
                    Err(_) => self.close_admission_slot(),
                }
            }
            CapacityKind::Observe => {} // no nudge, factory, token or capacity allocation
            CapacityKind::Cancel => {
                {
                    let mut state = self
                        .row
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    // A concurrent close must not turn a valid staged successor
                    // into a demand to mutate an unrelated generation's floor.
                    state
                        .admission_slot
                        .require_observable(request.generation)?;
                    if !state.admission_slot.closed {
                        state.admission_slot.cancel(request.generation)?;
                    }
                }
                self.row.admission_changed.notify_waiters();
            }
        }
        self.capacity_snapshot(request.generation)
    }

    fn capacity_snapshot(&self, requested: u64) -> Result<Snapshot, Refused> {
        let mut state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let closed = state.closed
            || state.effects_closed
            || !self.selected().lease().is_live()
            || state
                .admission
                .as_ref()
                .is_none_or(AdmissionPin::is_cancelled);
        let slot = &mut state.admission_slot;
        slot.require_observable(requested)?;
        let invalid_reserved = slot.phase == Phase::Reserved
            && (slot.reclaiming
                || slot.recovery_active
                || !slot.capacity.as_ref().is_some_and(|capacity| {
                    capacity.is_current()
                        && capacity.generation() == slot.generation
                        && slot
                            .registration
                            .as_ref()
                            .is_some_and(|owner| capacity.belongs_to(owner))
                }));
        if closed
            || invalid_reserved
            || slot
                .registration
                .as_ref()
                .is_some_and(|owner| !owner.is_current())
        {
            slot.close();
        }
        let code = if slot.closed {
            if slot.collected() {
                CapacityState::Closed
            } else if slot.phase == Phase::Uncertain {
                CapacityState::Uncertain
            } else {
                CapacityState::Closing
            }
        } else if requested < slot.generation {
            // Advancement required actual old generation return. This is an
            // exact negative certificate, never the successor's reservation.
            if slot.cancelled_through < requested {
                return Err(Refused::Binding);
            }
            CapacityState::Superseded
        } else if slot.cancelled_through >= requested {
            if slot.phase == Phase::Returned
                && !slot.reclaiming
                && slot.capacity.is_none()
                && slot
                    .invocation
                    .as_ref()
                    .is_none_or(|old| old.work.is_none() && old.outcome.is_none())
            {
                CapacityState::Returned
            } else {
                CapacityState::Closing
            }
        } else if requested > slot.generation {
            if slot.task_started && slot.desired == requested {
                CapacityState::Preparing
            } else {
                CapacityState::Unstarted
            }
        } else {
            match slot.phase {
                Phase::Waiting => CapacityState::Waiting,
                Phase::Pending | Phase::Staged | Phase::Finalizing => CapacityState::Preparing,
                Phase::Reserved
                    if !slot.reclaiming
                        && !slot.recovery_active
                        && slot.capacity.as_ref().is_some_and(|capacity| {
                            capacity.is_current()
                                && capacity.generation() == requested
                                && slot
                                    .registration
                                    .as_ref()
                                    .is_some_and(|owner| capacity.belongs_to(owner))
                        }) =>
                {
                    CapacityState::Reserved
                }
                Phase::Reserved | Phase::Uncertain => CapacityState::Uncertain,
                Phase::Returned | Phase::Closed => CapacityState::Closing,
            }
        };
        let snapshot = Snapshot {
            state: code,
            requested_generation: requested,
            canonical_generation: slot.generation,
            cancelled_through: slot.cancelled_through,
            incarnation: slot
                .registration
                .as_ref()
                .map(RootAdmissionSession::incarnation),
        };
        let wake = slot.closed && !slot.collected();
        drop(state);
        if wake {
            self.row.admission_changed.notify_waiters();
        }
        Ok(snapshot)
    }

    /// Recover the original terminal Slot using only its transferred position.
    /// Contending callers create no work. An active Actor is never waited on here;
    /// End returns Capacity and shutdown awaits its SAME retirement separately.
    pub(super) fn recover_terminal_admission(&self) -> bool {
        let Some(custody) = self.take_emergency_custody() else {
            return false;
        };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let native = custody.native.as_ref().expect("original retained position");
            if let Ok(Some(original)) = self.take_admission_finalization(native) {
                original.finish_at(None);
            }
        }));
        drop(custody);
        true
    }
    fn take_emergency_custody(&self) -> Option<EmergencyCustody> {
        let custody = {
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let slot = &mut state.admission_slot;
            if !slot.closed || !slot.task_returned || slot.recovery_active {
                return None;
            }
            let native = slot.emergency_native.take()?;
            slot.recovery_active = true;
            EmergencyCustody {
                route: self.clone(),
                native: Some(native),
                _funding: slot
                    .task_funding
                    .as_ref()
                    .expect("original task funding")
                    .clone(),
            }
        };
        Some(custody)
    }

    pub(super) fn admission_work(&self) -> Option<VoiceAdmissionWork> {
        self.row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .admission_slot
            .invocation
            .as_ref()
            .and_then(|original| original.work.clone())
    }
    fn admission_drained(&self) -> bool {
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let slot = &state.admission_slot;
        slot.owners_drained()
    }
    fn close_admission_slot(&self) {
        self.row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .admission_slot
            .close();
        self.row.admission_changed.notify_waiters();
    }
    fn requested_admission(&self) -> Option<u64> {
        let mut state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let slot = &mut state.admission_slot;
        if slot.closed {
            return None;
        }
        let generation = slot.requested?;
        let eligible = (slot.generation == 0 && generation == 1)
            || (slot.generation == generation
                && slot.phase == Phase::Waiting
                && slot.cancelled_through < generation)
            || (slot.generation.checked_add(1) == Some(generation)
                && slot.phase == Phase::Returned);
        if eligible {
            slot.requested.take()
        } else {
            None
        }
    }
    fn stage_admission_request(
        &self,
        generation: u64,
    ) -> Result<bool, crate::voice_handoff::factory::Refused> {
        use crate::voice_handoff::factory;
        if generation == 0 {
            return Err(VoiceError::Bounds.into());
        }
        let mut state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed
            || state.effects_closed
            || state.pending
            || state.browser_setup.is_some()
            || !self.selected().lease().is_live()
            || state
                .admission
                .as_ref()
                .is_none_or(AdmissionPin::is_cancelled)
        {
            return Err(factory::Refused::Closed);
        }
        let slot = &mut state.admission_slot;
        if slot.closed || slot.task_returned {
            return Err(factory::Refused::Closed);
        }
        if slot.generation == 0 && generation != 1 {
            return Err(VoiceError::Bounds.into());
        }
        if generation > slot.generation
            && slot.generation != 0
            && (slot.generation.checked_add(1) != Some(generation)
                || slot.phase != Phase::Returned
                || slot.cancelled_through < slot.generation
                || slot.reclaiming)
        {
            return Err(VoiceError::Bounds.into());
        }
        if generation <= slot.cancelled_through || generation < slot.generation {
            // Historical/never-started canceled retries must remain observations.
            // In particular do not buy a permanent task for an ordinal it can
            // never submit after its exact negative floor has been latched.
            return Ok(true);
        }
        if !slot.task_started {
            return Ok(false);
        }
        if generation >= slot.generation
            && (slot.generation == 0
                || slot.phase == Phase::Waiting
                || slot.phase == Phase::Returned)
            && generation > slot.cancelled_through
        {
            slot.desired = generation;
            slot.requested = Some(generation);
        }
        drop(state);
        self.row.admission_changed.notify_waiters();
        Ok(true)
    }
    /// Host cadence may nudge but never await/replace the permanent task. This
    /// re-arms only local capacity preparation; it emits no wire attempt or timer.
    pub(in crate::voice_handoff) fn nudge_admission(&self) {
        self.recover_terminal_admission(); // negative only; outside all other guards
        let wake = {
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let closed = state.closed || state.effects_closed || !self.selected().lease().is_live();
            let slot = &mut state.admission_slot;
            if closed
                || slot
                    .registration
                    .as_ref()
                    .is_some_and(|original| !original.is_current())
            {
                slot.close();
            }
            if slot.task_started && !slot.task_returned {
                let retry_current = slot.phase == Phase::Waiting;
                let retry_next = slot.phase == Phase::Returned
                    && slot.generation.checked_add(1) == Some(slot.desired)
                    && slot.cancelled_through >= slot.generation;
                if !slot.closed
                    && (retry_current || retry_next)
                    && slot.desired > slot.cancelled_through
                {
                    slot.requested = Some(slot.desired);
                }
                true
            } else {
                false
            }
        };
        if wake {
            self.row.admission_changed.notify_waiters();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Waiting,
    Pending,
    Staged,
    Finalizing,
    Reserved,
    Returned,
    Uncertain,
    Closed,
}

/// Opaque allocation identity, not a wire nonce. Equal ordinals never identify
/// two pressure retries. Its small allocation belongs to the original Row/task
/// inventories; it contains no Route, callback, credit or other ownership edge.
struct InvocationId;
struct Invocation {
    id: Arc<InvocationId>,
    work: Option<VoiceAdmissionWork>,
    outcome: Option<Outcome>,
}
enum Outcome {
    Prepared(Result<AdmissionPrepared, VoiceError>),
    /// The Actor's exact refusal certifies no Root entry, not actual return yet.
    Unstarted(VoiceEndpointError),
    /// Exact accepted callback refused BEFORE enqueue because this generation
    /// was canceled. Unlike generic Stopped, it does not terminate registration.
    CancelledUnstarted,
    /// Cleanup of already retained capacity, or terminal uncertainty after the
    /// original callback returned. Neither variant certifies no Root entry.
    Capacity(RootHighPreclaim),
    /// Same original candidate retained after a proven local final-fence refusal.
    /// Retrying this value never requeues Root or replaces the registration.
    Candidate(RootHighPreclaim),
    Uncertain,
}

pub(super) struct Slot {
    generation: u64,
    cancelled_through: u64,
    phase: Phase,
    closed: bool,
    reclaiming: bool,
    registration: Option<RootAdmissionSession>,
    capacity: Option<RootHighPreclaim>,
    invocation: Option<Invocation>,
    // One permanent task/position. Funding is plain, with no Row/task backedge.
    task_started: bool,
    task_returned: bool,
    task_funding: Option<Arc<TaskFunding>>,
    emergency_native: Option<crate::voice_handoff::channel::NativeHold>,
    recovery_active: bool,
    desired: u64,
    requested: Option<u64>,
    #[cfg(test)]
    unwind_reclamation: bool,
}
impl Default for Slot {
    fn default() -> Self {
        Self {
            generation: 0,
            cancelled_through: 0,
            phase: Phase::Waiting,
            closed: false,
            reclaiming: false,
            registration: None,
            capacity: None,
            invocation: None,
            task_started: false,
            task_returned: false,
            task_funding: None,
            emergency_native: None,
            recovery_active: false,
            desired: 0,
            requested: None,
            #[cfg(test)]
            unwind_reclamation: false,
        }
    }
}
impl Slot {
    /// Pure ordinal validation. Observing never creates a generation, but an
    /// already-staged successor remains observable if whole-Session close wins
    /// before its worker can install Pending. Unknown skips/wrap always refuse.
    fn require_observable(&self, generation: u64) -> Result<(), Refused> {
        if generation == 0 {
            return Err(Refused::Bounds);
        }
        if generation <= self.generation
            || (self.generation == 0 && generation == 1)
            || (generation == self.desired && self.desired > self.generation)
        {
            return Ok(());
        }
        if self.generation.checked_add(1) == Some(generation)
            && self.phase == Phase::Returned
            && self.cancelled_through >= self.generation
            && !self.reclaiming
            && self.capacity.is_none()
            && self
                .invocation
                .as_ref()
                .is_none_or(|old| old.work.is_none() && old.outcome.is_none())
        {
            return Ok(());
        }
        Err(Refused::Bounds)
    }

    fn owners_drained(&self) -> bool {
        self.closed
            && !self.reclaiming
            && self.registration.is_none()
            && self.capacity.is_none()
            && self.invocation.is_none()
    }
    fn is_original(&self, id: &Arc<InvocationId>) -> bool {
        self.invocation
            .as_ref()
            .is_some_and(|original| Arc::ptr_eq(&original.id, id))
    }

    /// Only negative atomics under State. No actual Root owner/Work is removed or
    /// destroyed here. Whole-mode cancellation ends future negotiation admission;
    /// established media owns a different lifetime and is not closed by this gate.
    pub(super) fn close(&mut self) {
        self.closed = true;
        if let Some(registration) = &self.registration {
            registration.close();
        }
        if self.registration.is_none()
            && self.capacity.is_none()
            && !self.reclaiming
            && self.phase != Phase::Finalizing
            && self
                .invocation
                .as_ref()
                .is_none_or(|original| original.work.is_none() && original.outcome.is_none())
        {
            // No accepted Work can ever enter after removing this exact token:
            // accepted must find Pending first. Only plain marker metadata drops.
            self.invocation.take();
            self.phase = Phase::Closed;
        }
    }

    /// Validate the exact canonical ordinal BEFORE advancing Root's negative floor.
    /// Old Cancel may not poison a successor; future/MAX cannot skip admission.
    fn cancel(&mut self, generation: u64) -> Result<(), Refused> {
        if generation == 0 {
            return Err(Refused::Bounds);
        }
        if self.generation == 0 {
            if generation != 1 {
                return Err(Refused::Bounds);
            }
            // Authoritative never-started negative fence. A producer which has
            // not installed Pending cannot later pass accepted for this ordinal.
            self.generation = 1;
            self.phase = Phase::Returned;
        } else if generation > self.generation {
            let no_work = self
                .invocation
                .as_ref()
                .is_none_or(|old| old.work.is_none() && old.outcome.is_none());
            if self.generation.checked_add(1) != Some(generation)
                || self.phase != Phase::Returned
                || self.cancelled_through < self.generation
                || self.reclaiming
                || self.capacity.is_some()
                || !no_work
            {
                return Err(Refused::Bounds);
            }
            self.generation = generation;
            self.phase = Phase::Returned;
        }
        self.cancelled_through = self.cancelled_through.max(generation);
        if self
            .requested
            .is_some_and(|requested| requested <= generation)
        {
            self.requested = None;
        }
        if let Some(registration) = &self.registration {
            // The stored owner guarantees these temporary observation references
            // cannot run the last core/payment destructor under State.
            registration
                .observer()
                .cancel_through(generation)
                .map_err(|_| Refused::Bounds)?;
        }
        if generation == u64::MAX {
            self.close();
        }
        Ok(())
    }

    /// No-work observation for final channel collection only. Ordinary cancellation
    /// must keep the stable registration for its next admitted generation instead.
    pub(super) fn collected(&self) -> bool {
        (self.closed || !self.task_started)
            && !self.reclaiming
            && self.registration.is_none()
            && self.capacity.is_none()
            && self.invocation.is_none()
            && self.emergency_native.is_none()
            && !self.recovery_active
            && (!self.task_started || self.task_returned)
    }
}

impl Route {
    /// Caller has independently paid its full native producer and owns the actual
    /// native position before entering these short guards. The SAME original UI
    /// mutex is checked, not a bool or an unrelated fixture mutex. No Actor/Root
    /// factory, Clock, await or heavy owner destructor occurs under this fence.
    fn begin_admission(
        &self,
        live: &crate::voice_handoff::context::LiveContext,
        authority: &crate::voice_authority::Coordinator,
        commit: &tokio::sync::OwnedMutexGuard<()>,
        native: &crate::voice_handoff::channel::NativeHold,
        generation: u64,
    ) -> Result<Option<Preparation>, crate::voice_handoff::factory::Refused> {
        use crate::voice_handoff::factory;
        if generation == 0 {
            return Err(VoiceError::Bounds.into());
        }
        if !Arc::ptr_eq(tokio::sync::OwnedMutexGuard::mutex(commit), &live.commit)
            || !native.belongs_to(self.session())
        {
            return Err(factory::Refused::Closed);
        }
        factory::require_current_selection(live.gate(), authority, self.selected())?;
        live.channels
            .require_cleanup_route(self)
            .map_err(|_| factory::Refused::Closed)?;
        // Plain identity only, prepaid by the outer producer BEFORE allocation.
        // A losing candidate grants no factory; its allocation drops outside State.
        let id = Arc::new(InvocationId);
        let mut state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed
            || state.effects_closed
            || state.pending
            || state.browser_setup.is_some()
            || state
                .admission
                .as_ref()
                .is_none_or(AdmissionPin::is_cancelled)
        {
            return Err(factory::Refused::Closed);
        }
        let slot = &mut state.admission_slot;
        if slot.closed || slot.reclaiming {
            return Err(factory::Refused::Closed);
        }
        if let Some(registration) = &slot.registration {
            if !registration.is_current() {
                slot.close();
                return Err(factory::Refused::Closed);
            }
        }
        if slot.generation == 0 {
            if generation != 1 {
                return Err(VoiceError::Bounds.into());
            }
            slot.generation = generation;
        } else if generation < slot.generation {
            // Historical observation never starts Root work.
            return Ok(None);
        } else if generation == slot.generation {
            if slot.cancelled_through >= generation || slot.phase != Phase::Waiting {
                return Ok(None);
            }
            if slot
                .invocation
                .as_ref()
                .is_some_and(|old| old.work.is_some() || old.outcome.is_some())
            {
                return Err(VoiceError::Capacity.into());
            }
        } else {
            let collected = slot
                .invocation
                .as_ref()
                .is_none_or(|old| old.work.is_none() && old.outcome.is_none());
            if slot.generation.checked_add(1) != Some(generation)
                || slot.phase != Phase::Returned
                || slot.cancelled_through < slot.generation
                || slot.capacity.is_some()
                || !collected
            {
                return Err(VoiceError::Bounds.into());
            }
            // One persistent task owns the actual NativeHold. Returned and the
            // empty original Work/output/capacity above certify the PER-GENERATION
            // boundary; its unchanged paid task is not claimed returned/refunded.
            slot.generation = generation;
        }
        let original = slot
            .registration
            .as_ref()
            .map(RootAdmissionSession::observer);
        // Any old invocation remaining here contains only a plain identity; actual
        // Work/Root owners had to be detached and destroyed before Returned/Waiting.
        slot.invocation = Some(Invocation {
            id: id.clone(),
            work: None,
            outcome: None,
        });
        slot.phase = Phase::Pending;
        Ok(Some(Preparation {
            original,
            generation,
            publisher: Publisher {
                route: self.clone(),
                id,
                settled: false,
                cancelled_before_enqueue: false,
            },
        }))
    }
}

/// One affine finisher competes with maintenance for the original staged value.
/// Before consumption, Drop restores it. After consumption, panic is terminal
/// uncertainty; it cannot mint again or overwrite the stable retained registration.
struct Finalization<'a> {
    // The affine finisher cannot outlive or release its ACTUAL native position.
    // A global idle observation is never a substitute for this borrowed guard.
    _native: &'a crate::voice_handoff::channel::NativeHold,
    route: Route,
    id: Arc<InvocationId>,
    outcome: Option<Outcome>,
    completed: bool,
}
impl Route {
    fn take_admission_finalization<'a>(
        &self,
        native: &'a crate::voice_handoff::channel::NativeHold,
    ) -> Result<Option<Finalization<'a>>, Refused> {
        if !native.belongs_to(self.session()) {
            return Err(Refused::Binding);
        }
        let mut state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let closed = state.closed
            || state.effects_closed
            || !self.selected().lease().is_live()
            || state
                .admission
                .as_ref()
                .is_some_and(AdmissionPin::is_cancelled);
        let slot = &mut state.admission_slot;
        if closed {
            slot.close();
        }
        if slot.reclaiming || slot.phase == Phase::Finalizing {
            return Ok(None);
        }
        let Some(original) = slot.invocation.as_ref() else {
            return Ok(None);
        };
        // Retirement fences positive callbacks, not the last plain payer clone.
        if original
            .work
            .as_ref()
            .is_some_and(|work| !work.is_retired())
        {
            return Ok(None);
        }
        let id = original.id.clone();
        let outcome =
            if let Some(outcome) = slot.invocation.as_mut().expect("original").outcome.take() {
                outcome
            } else if (slot.closed || slot.cancelled_through >= slot.generation)
                && slot.capacity.is_some()
            {
                Outcome::Capacity(slot.capacity.take().expect("checked capacity"))
            } else if slot.closed || slot.phase == Phase::Uncertain {
                Outcome::Uncertain
            } else {
                return Ok(None);
            };
        slot.phase = Phase::Finalizing;
        Ok(Some(Finalization {
            _native: native,
            route: self.clone(),
            id,
            outcome: Some(outcome),
            completed: false,
        }))
    }
}

impl Drop for Finalization<'_> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        {
            let mut state = self
                .route
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let slot = &mut state.admission_slot;
            if slot.is_original(&self.id) && slot.phase == Phase::Finalizing {
                if let Some(outcome) = self.outcome.take() {
                    let original = slot.invocation.as_mut().expect("checked invocation");
                    if original.outcome.is_none() {
                        original.outcome = Some(outcome);
                        slot.phase = Phase::Staged;
                    } else {
                        // Restore our original owner for destruction outside State.
                        self.outcome = Some(outcome);
                        slot.close();
                        slot.phase = Phase::Uncertain;
                    }
                } else {
                    slot.close();
                    slot.phase = Phase::Uncertain;
                }
            }
        }
        // Any uninstalled heavy outcome remains a field and drops AFTER this
        // destructor's State guard. It cannot run Root destruction under State.
    }
}

impl Finalization<'_> {
    fn finish(self, context: &crate::voice_handoff::context::Context) {
        self.finish_at(Some(context));
    }
    fn finish_at(mut self, context: Option<&crate::voice_handoff::context::Context>) {
        let outcome = self.outcome.take().expect("one affine outcome");
        let (mut registration, mut capacity, mut wanted, cancellation_may_explain_refusal) =
            match outcome {
                Outcome::Prepared(Ok(prepared)) => {
                    let (registration, capacity) = prepared.into_parts();
                    let wanted = if capacity.is_ok() {
                        Phase::Reserved
                    } else if matches!(capacity, Err(VoiceError::Capacity)) {
                        Phase::Waiting
                    } else {
                        Phase::Closed
                    };
                    let cancellation_may_explain_refusal =
                        matches!(capacity, Err(VoiceError::Unauthorized));
                    (
                        registration,
                        capacity.ok(),
                        wanted,
                        cancellation_may_explain_refusal,
                    )
                }
                Outcome::Prepared(Err(VoiceError::Capacity))
                | Outcome::Unstarted(VoiceEndpointError::Protocol(VoiceError::Capacity)) => {
                    (None, None, Phase::Waiting, false)
                }
                Outcome::Capacity(capacity) => (None, Some(capacity), Phase::Returned, false),
                Outcome::CancelledUnstarted => (None, None, Phase::Returned, false),
                Outcome::Candidate(capacity) => (None, Some(capacity), Phase::Reserved, false),
                Outcome::Prepared(Err(VoiceError::Unauthorized)) => {
                    (None, None, Phase::Closed, true)
                }
                Outcome::Prepared(Err(_)) | Outcome::Unstarted(_) | Outcome::Uncertain => {
                    (None, None, Phase::Closed, false)
                }
            };
        // Preserve the original newly minted registration EVEN if capacity failed
        // or cancellation/End won. This quarantine step grants no permission.
        {
            let mut state = self
                .route
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let slot = &mut state.admission_slot;
            if !slot.is_original(&self.id) || slot.phase != Phase::Finalizing {
                slot.close();
                wanted = Phase::Closed;
            } else {
                if registration.is_some() {
                    if slot.registration.is_none() {
                        slot.registration = registration.take();
                    } else {
                        slot.close();
                        wanted = Phase::Closed;
                    }
                }
                if let Some(owner) = &slot.registration {
                    if slot.cancelled_through != 0 {
                        let _ = owner.observer().cancel_through(slot.cancelled_through);
                    }
                    if slot.closed {
                        owner.close();
                    }
                }
                if slot.closed {
                    // Emergency recovery has no positive context at all. Terminal
                    // owners can only be destroyed, never attached or reminted.
                    wanted = Phase::Closed;
                }
                if !slot.closed
                    && slot.cancelled_through >= slot.generation
                    && (wanted == Phase::Reserved
                        || wanted == Phase::Waiting
                        || (cancellation_may_explain_refusal
                            && slot
                                .registration
                                .as_ref()
                                .is_some_and(RootAdmissionSession::is_current)))
                {
                    // Root reports Unauthorized for a canceled original ordinal
                    // too. Preserve the stable CURRENT registration rather than
                    // turning an ordinary round cancellation into call failure.
                    wanted = Phase::Returned;
                }
            }
        }
        // Unexpected duplicate registration and all other rejected owners are
        // destroyed here, outside every native/Root guard.
        drop(registration);
        if wanted == Phase::Reserved {
            enum Attachment {
                Retained,
                Cancelled,
            }
            // All expensive/async preparation is outside the short final fence.
            // The actual original UI mutex orders this with lock and final close.
            let permitted = (|| {
                use crate::voice_handoff::factory;
                let context = context.ok_or(factory::Refused::Closed)?;
                let live = context.upgrade().ok_or(factory::Refused::Closed)?;
                let commit = live
                    .commit
                    .clone()
                    .try_lock_owned()
                    .map_err(|_| VoiceError::Capacity)?;
                factory::require_unlocked_now(&live, self.route.selected())?;
                {
                    let servers = live.servers.try_lock().map_err(|_| VoiceError::Capacity)?;
                    let binding = self.route.selected().binding();
                    if !servers.get(&binding.server).is_some_and(|entry| {
                        entry.instance == binding.instance && entry.group_id == binding.group
                    }) {
                        return Err(factory::Refused::Closed);
                    }
                }
                let authority = match live.authority.try_lock() {
                    Ok(held) => held,
                    Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
                    Err(std::sync::TryLockError::WouldBlock) => {
                        return Err(VoiceError::Capacity.into())
                    }
                };
                factory::require_current_selection(live.gate(), &authority, self.route.selected())?;
                let mut state = match self.route.row.state.try_lock() {
                    Ok(held) => held,
                    Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
                    Err(std::sync::TryLockError::WouldBlock) => {
                        return Err(VoiceError::Capacity.into())
                    }
                };
                if state.closed
                    || state.effects_closed
                    || state.pending
                    || state.browser_setup.is_some()
                    || state
                        .admission
                        .as_ref()
                        .is_none_or(AdmissionPin::is_cancelled)
                {
                    return Err(factory::Refused::Closed);
                }
                let slot = &mut state.admission_slot;
                if slot.is_original(&self.id)
                    && slot.cancelled_through >= slot.generation
                    && !slot.closed
                {
                    return Ok(Attachment::Cancelled);
                }
                let candidate = capacity.as_ref().ok_or(factory::Refused::Closed)?;
                if !slot.is_original(&self.id)
                    || slot.phase != Phase::Finalizing
                    || slot.closed
                    || slot.cancelled_through >= slot.generation
                    || !candidate.is_current()
                    || candidate.generation() != slot.generation
                    || !slot
                        .registration
                        .as_ref()
                        .is_some_and(|owner| owner.is_current() && candidate.belongs_to(owner))
                    || slot.capacity.is_some()
                {
                    return Err(factory::Refused::Closed);
                }
                slot.capacity = capacity.take();
                // Explicitly release short guards before any outer owner/credit
                // can drop. Returning success is capacity only, never signed Ready.
                drop(state);
                drop(authority);
                drop(commit);
                Ok::<_, factory::Refused>(Attachment::Retained)
            })();
            match permitted {
                Ok(Attachment::Retained) => {}
                Ok(Attachment::Cancelled) => wanted = Phase::Returned,
                Err(crate::voice_handoff::factory::Refused::Protocol(VoiceError::Capacity)) => {
                    // A local busy guard is not call failure or permission to
                    // allocate another HIGH. Restore THIS exact candidate.
                    if self.retain_candidate(&mut capacity) {
                        return;
                    }
                    wanted = Phase::Returned;
                }
                Err(_) => wanted = Phase::Closed,
            }
        }
        // The outer candidate's actual heavy destruction precedes any collection
        // publication, including after positive fence refusal or cancellation.
        drop(capacity);
        self.collect(wanted);
    }

    fn retain_candidate(&mut self, capacity: &mut Option<RootHighPreclaim>) -> bool {
        let mut state = self
            .route
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let closed = state.closed
            || state.effects_closed
            || !self.route.selected().lease().is_live()
            || state
                .admission
                .as_ref()
                .is_some_and(AdmissionPin::is_cancelled);
        let slot = &mut state.admission_slot;
        if closed {
            slot.close();
        }
        if !slot.is_original(&self.id)
            || slot.phase != Phase::Finalizing
            || slot.closed
            || slot.cancelled_through >= slot.generation
            || capacity.is_none()
        {
            return false;
        }
        let original = slot.invocation.as_mut().expect("original invocation");
        if original.outcome.is_some() {
            slot.close();
            return false;
        }
        original.outcome = Some(Outcome::Candidate(
            capacity.take().expect("same original candidate"),
        ));
        slot.phase = Phase::Staged;
        self.completed = true;
        true
    }

    fn collect(&mut self, mut wanted: Phase) {
        loop {
            let (capacity, registration, work, done, unwind) = {
                let mut state = self
                    .route
                    .row
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let closed = state.closed
                    || state.effects_closed
                    || !self.route.selected().lease().is_live()
                    || state
                        .admission
                        .as_ref()
                        .is_some_and(AdmissionPin::is_cancelled);
                let slot = &mut state.admission_slot;
                if closed
                    || wanted == Phase::Closed
                    || slot
                        .registration
                        .as_ref()
                        .is_some_and(|owner| !owner.is_current())
                {
                    slot.close();
                    wanted = Phase::Closed;
                }
                if !slot.is_original(&self.id) {
                    slot.close();
                    slot.phase = Phase::Uncertain;
                    self.completed = true;
                    return;
                }
                let cancelled = slot.cancelled_through >= slot.generation;
                if cancelled && wanted != Phase::Closed {
                    wanted = Phase::Returned;
                }
                slot.reclaiming = true;
                let capacity = if slot.closed || cancelled {
                    slot.capacity.take()
                } else {
                    None
                };
                let registration = if slot.closed {
                    slot.registration.take()
                } else {
                    None
                };
                let work = slot.invocation.as_mut().expect("original").work.take();
                let done = capacity.is_none() && registration.is_none() && work.is_none();
                #[cfg(test)]
                let unwind = std::mem::take(&mut slot.unwind_reclamation);
                #[cfg(not(test))]
                let unwind = false;
                if done {
                    slot.reclaiming = false;
                    slot.phase = wanted;
                    if slot.closed {
                        // Only the plain invocation identity remains. Our outer
                        // token keeps its allocation paid through this publication.
                        slot.invocation.take();
                    }
                }
                (capacity, registration, work, done, unwind)
            };
            // End/Cancel can arrive DURING these destructors. Loop and inspect
            // again before publishing return; never use the earlier closed bit.
            drop(Reclamation {
                owners: ReclamationOwners {
                    capacity,
                    registration,
                    work,
                    unwind,
                },
                returned: ReclamationReturned {
                    route: self.route.clone(),
                    id: self.id.clone(),
                },
            });
            if done {
                self.completed = true;
                return;
            }
        }
    }
}

/// The original queue handoff, created only after native admission installs Pending.
/// Payment/Actor/grant and native position are supplied by the outer paid producer.
/// Dropping an unqueued preparation leaves no way to infer Root publication absence
/// once accepted Work exists; Publisher's Drop below preserves that distinction.
pub(super) struct Preparation {
    pub(super) original: Option<AdmissionSessionObserver>,
    pub(super) generation: u64,
    publisher: Publisher,
}
impl Preparation {
    pub(super) fn publisher(
        self,
    ) -> (
        Option<AdmissionSessionObserver>,
        u64,
        Box<dyn VoiceAdmissionPublisher>,
    ) {
        (self.original, self.generation, Box::new(self.publisher))
    }
}

struct Publisher {
    route: Route,
    id: Arc<InvocationId>,
    settled: bool,
    cancelled_before_enqueue: bool,
}
impl Publisher {
    /// Apply only a proof from this exact invocation. No Root factory or callback
    /// can execute under this quarantine lock; completed owners move into State.
    fn stage(&mut self, mut outcome: Option<Outcome>, work: Option<VoiceAdmissionWork>) {
        {
            let mut state = self
                .route
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let slot = &mut state.admission_slot;
            if slot.is_original(&self.id) {
                let original = slot.invocation.as_mut().expect("checked invocation");
                let same_work = match (&original.work, &work) {
                    (Some(a), Some(b)) => a.is_same(b),
                    (None, None) => true,
                    _ => false,
                };
                if same_work && original.outcome.is_none() {
                    original.outcome = outcome.take();
                    slot.phase = Phase::Staged;
                } else {
                    slot.close();
                    slot.phase = Phase::Uncertain;
                }
            } else {
                // This cannot be a valid late publisher: advancement requires its
                // actual Work/owners returned. Quarantine rather than remint.
                slot.close();
                slot.phase = Phase::Uncertain;
            }
        }
        self.settled = true;
        self.route.row.admission_changed.notify_waiters();
        // On internal mismatch, drop the original heavy outcome AFTER State. The
        // physical Session is terminal; losing it never authorizes another mint.
        drop(outcome);
    }
}
impl VoiceAdmissionPublisher for Publisher {
    fn accepted(&mut self, work: VoiceAdmissionWork) -> Result<(), VoiceEndpointError> {
        let mut state = self
            .route
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let closed =
            state.closed || state.effects_closed || !self.route.selected().lease().is_live();
        let slot = &mut state.admission_slot;
        if !slot.is_original(&self.id) || slot.phase != Phase::Pending {
            return Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized));
        }
        let original = slot.invocation.as_mut().expect("checked invocation");
        if original.work.is_some() {
            return Err(VoiceEndpointError::Protocol(VoiceError::RetryMismatch));
        }
        // Even refusal keeps the exact Work: its callback/argument retirement
        // occurs AFTER the consuming refused callback, not at this observation.
        original.work = Some(work);
        // This proof is narrower than Stopped: the actor has not enqueued any
        // request, and ordinary ordinal cancellation alone won the admission.
        // Whole-mode/End closure still overrides it during final collection.
        self.cancelled_before_enqueue =
            !closed && !slot.closed && slot.cancelled_through >= slot.generation;
        if closed || slot.closed || slot.cancelled_through >= slot.generation {
            Err(VoiceEndpointError::Stopped)
        } else {
            Ok(())
        }
    }

    fn completed(
        mut self: Box<Self>,
        result: Result<AdmissionPrepared, VoiceError>,
        work: VoiceAdmissionWork,
    ) {
        self.stage(Some(Outcome::Prepared(result)), Some(work));
    }

    fn refused(mut self: Box<Self>, error: VoiceEndpointError, work: Option<VoiceAdmissionWork>) {
        let outcome = if self.cancelled_before_enqueue
            && matches!(error, VoiceEndpointError::Stopped)
            && work.is_some()
        {
            Outcome::CancelledUnstarted
        } else {
            Outcome::Unstarted(error)
        };
        self.stage(Some(outcome), work);
    }
}
impl Drop for Publisher {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let mut state = self
            .route
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let slot = &mut state.admission_slot;
        if !slot.is_original(&self.id) {
            return;
        }
        let original = slot.invocation.as_ref().expect("checked invocation");
        if original.work.is_none() && original.outcome.is_none() {
            // accepted ALWAYS precedes enqueue. No Work is a genuine local
            // never-queued preparation; it grants no callback-return certificate.
            slot.phase = if slot.closed || slot.cancelled_through >= slot.generation {
                Phase::Returned
            } else {
                Phase::Waiting
            };
        } else if original.outcome.is_none() {
            // After accepted, caller/task loss or unwind cannot prove no Root
            // publication. Preserve canonical Work and forbid another factory.
            slot.phase = Phase::Uncertain;
            slot.close();
        }
        drop(state);
        self.route.row.admission_changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests;
