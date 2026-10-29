//! Inactive trusted call activation. No Tauri registration or key/RTC producer uses
//! this entry yet. The canonical task, not an invoke waiter, owns native admission.
#![allow(dead_code)] // staged internal entry; enabling it requires separate protocol gates
use crate::{voice_authority, AppState, ServerEntry, UiSessionGate, VoiceActivation};
use catcoms_app::{
    PreparedVoiceCall, VoiceActivationControl, VoiceCallActivationPublisher,
    VoiceCallActivationResult, VoiceCallActivationWork, VoiceEndpointError,
};
use catcoms_rt::Clock;
use catcoms_sync::voice_v2::call_activation::{
    ActivationObserver, ActivationOwner, ActivationRpcPermit, RootResidenceReturned,
    RootResidenceUnclaimed, RootSelectedCall,
};
use catcoms_sync::voice_v2::{CallGrant, VoiceError};
use std::collections::HashMap;
use std::future::{poll_fn, Future};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::task::Poll;
use tokio::sync::{Mutex, OwnedMutexGuard};

pub(crate) const MAX_ACTIVATIONS: usize = 32;
const NATIVE_BYTES: usize = 16 * 1024;
const CADENCE_MS: u64 = 250;
struct Funding {
    _credit: catcoms_sync::voice_v2::resources::Credit,
}
#[derive(Clone)]
pub(crate) struct Handle {
    cell: Arc<Cell>,
    // All raw Cell aliases stay private and pair with this SAME last plain payer.
    funding: Arc<Funding>,
}
struct Cell {
    id: String,
    controller: String,
    binding: voice_authority::Binding,
    original: ActivationObserver,
    clock: Arc<dyn Clock>,
    state: StdMutex<State>,
    changed: tokio::sync::Notify,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    Revoked,
    Capacity,
    Stopped,
    Interrupted,
    Protocol(VoiceError),
}
impl Failure {
    fn message(self) -> String {
        match self {
            Self::Revoked => "The call ended before it could start.".into(),
            Self::Capacity => "Voice is busy. Try again.".into(),
            Self::Stopped => "This server is closed.".into(),
            Self::Interrupted => "The call could not finish starting.".into(),
            Self::Protocol(error) => error.to_string(),
        }
    }
}
#[derive(Clone, Copy)]
enum Terminal {
    Accepted([u8; 32]),
    Refused(Failure),
}
enum StorageProof {
    Returned(RootResidenceReturned),
    Unclaimed(RootResidenceUnclaimed),
}
impl StorageProof {
    fn matches(&self, original: &ActivationObserver) -> bool {
        match self {
            Self::Returned(proof) => proof.matches(original),
            Self::Unclaimed(proof) => proof.matches(original),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Select,
    Clear,
}
enum Outcome {
    Completed(VoiceCallActivationResult),
    Unstarted(VoiceEndpointError, Option<ActivationRpcPermit>),
    Uncertain,
}
struct Invocation {
    serial: u64,
    kind: Kind,
    work: Option<VoiceCallActivationWork>,
    outcome: Option<Outcome>,
}
struct TaskTicket {
    serial: u64,
    returned: Arc<AtomicBool>,
}
struct State {
    owner: Option<ActivationOwner>,
    grant: Option<Arc<CallGrant>>,
    control: Option<VoiceActivationControl>,
    select_payment: Option<ActivationRpcPermit>,
    clear_payment: Option<ActivationRpcPermit>,
    invocation: Option<Invocation>,
    serial: u64,
    terminal: Option<Terminal>,
    acknowledged: bool,
    storage: Option<StorageProof>,
    cleanup_done: bool,
    task: TaskTicket,
    task_started: bool,
}
impl Handle {
    pub(crate) fn new(
        id: String,
        controller: String,
        binding: voice_authority::Binding,
        prepared: PreparedVoiceCall,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, VoiceError> {
        // These are trusted native identities, not wire data. Still pin every
        // retained allocation before construction; a large caller capacity must
        // not silently become an unaccounted owned String/Vec in the Cell.
        if id.is_empty()
            || id.capacity() > 64
            || controller.is_empty()
            || controller.capacity() > 64
            || binding.group.capacity() > 128
            || prepared.grant().scope() != binding.scope
        {
            return Err(VoiceError::Bounds);
        }
        let (prepared, control) = prepared.into_parts();
        let original = prepared.observer();
        let funding = Arc::new(Funding {
            _credit: original.reserve_retained(NATIVE_BYTES)?,
        });
        // Reserve negative progress BEFORE selection. No fresh ordinary payment is
        // needed after Root has accepted custody or the ordinary pool becomes full.
        let select_payment = original.reserve_rpc()?;
        let clear_payment = original.reserve_rpc()?;
        let (grant, owner) = prepared.into_parts();
        Ok(Self {
            cell: Arc::new(Cell {
                id,
                controller,
                binding,
                original,
                clock,
                state: StdMutex::new(State {
                    owner: Some(owner),
                    grant: Some(grant),
                    control: Some(control),
                    select_payment: Some(select_payment),
                    clear_payment: Some(clear_payment),
                    invocation: None,
                    serial: 0,
                    terminal: None,
                    acknowledged: false,
                    storage: None,
                    cleanup_done: false,
                    task: TaskTicket {
                        serial: 1,
                        returned: Arc::new(AtomicBool::new(false)),
                    },
                    task_started: false,
                }),
                changed: tokio::sync::Notify::new(),
            }),
            funding,
        })
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.cell, &other.cell)
    }
    pub(crate) fn id(&self) -> &str {
        &self.cell.id
    }
    pub(crate) fn binding(&self) -> &voice_authority::Binding {
        &self.cell.binding
    }
    pub(crate) fn controller(&self) -> &str {
        &self.cell.controller
    }
    pub(crate) fn original(&self) -> &ActivationObserver {
        &self.cell.original
    }
    pub(crate) fn grant(&self) -> Option<Arc<CallGrant>> {
        self.cell
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .grant
            .clone()
    }
    pub(crate) fn active(&self) -> bool {
        self.original().is_active()
            && matches!(
                self.cell
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .terminal,
                Some(Terminal::Accepted(_))
            )
    }
    pub(crate) fn close(&self) {
        self.original().close();
        self.cell.changed.notify_waiters();
    }
    pub(crate) fn admit(
        &self,
        selected: RootSelectedCall,
        grant: &Arc<CallGrant>,
        join: [u8; 32],
    ) -> Result<(), VoiceError> {
        if !selected.original().is_same(self.original()) {
            return Err(VoiceError::Unauthorized);
        }
        selected.commit_native(grant)?;
        let mut state = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.terminal.is_some() {
            return Err(VoiceError::RetryMismatch);
        }
        state.terminal = Some(Terminal::Accepted(join));
        // The final Coordinator fence calls this synchronously. Its canonical
        // task notifies AFTER dropping Coordinator and its UI guard; an arbitrary
        // waiter waker must never become a callback under those admission locks.
        Ok(())
    }
    fn refuse(&self, failure: Failure) {
        self.latch_refusal(failure);
        self.notify_closed();
    }
    // Only irreversible atomics/Cell metadata. The original native task may call
    // this before exceptional UI-guard release without executing any waker.
    fn latch_refusal(&self, failure: Failure) {
        self.original().invalidate();
        let mut state = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.terminal.is_none() {
            state.terminal = Some(Terminal::Refused(failure));
        }
    }
    fn notify_closed(&self) {
        self.original().notify_closed();
        self.cell.changed.notify_waiters();
    }
    fn begin(&self, kind: Kind) -> Result<(u64, ActivationRpcPermit, Box<Publisher>), Failure> {
        let mut state = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.invocation.is_some() {
            return Err(Failure::Interrupted);
        }
        let serial = state.serial.checked_add(1).ok_or(Failure::Interrupted)?;
        let payment = match kind {
            Kind::Select => state.select_payment.take(),
            Kind::Clear => state.clear_payment.take(),
        }
        .ok_or(Failure::Interrupted)?;
        state.serial = serial;
        state.invocation = Some(Invocation {
            serial,
            kind,
            work: None,
            outcome: None,
        });
        drop(state);
        Ok((
            serial,
            payment,
            Box::new(Publisher {
                handle: self.clone(),
                serial,
                settled: false,
            }),
        ))
    }
    async fn outcome_ready(&self) {
        loop {
            let notified = self.cell.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self
                .cell
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .invocation
                .as_ref()
                .is_some_and(|inv| inv.outcome.is_some())
            {
                return;
            }
            notified.await;
        }
    }
    fn work(&self) -> Option<VoiceCallActivationWork> {
        self.cell
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .invocation
            .as_ref()
            .and_then(|inv| inv.work.clone())
    }
    fn take_outcome(&self) -> Option<Outcome> {
        let invocation = {
            let mut state = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
            let inv = state.invocation.as_ref()?;
            if inv.outcome.is_none() || inv.work.as_ref().is_some_and(|work| !work.is_retired()) {
                return None;
            }
            state.invocation.take()
        };
        // Actual Work/argument retirement precedes moving/destroying the original
        // outcome; all heavy destruction happens outside the Cell mutex.
        invocation.and_then(|mut inv| inv.outcome.take())
    }
    async fn reply(&self) -> Reply {
        loop {
            let notified = self.cell.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self
                .cell
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .terminal
                .is_some()
            {
                return Reply {
                    original: self.clone(),
                };
            }
            notified.await;
        }
    }
    pub(crate) fn collectible(&self) -> bool {
        let state = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
        self.original().is_closed()
            && state.acknowledged
            && state.cleanup_done
            && state.invocation.is_none()
            && state.grant.is_none()
            && state.control.is_none()
            && state
                .storage
                .as_ref()
                .is_some_and(|proof| proof.matches(self.original()))
            && state.task.returned.load(Ordering::Acquire)
    }
}
/// Local trusted receiver receipt. Dropping a lost reply does NOT acknowledge it.
/// No Tauri/HTTP/wire encoding is provided by this inactive leaf.
pub(crate) struct Reply {
    original: Handle,
}
impl Reply {
    pub(crate) fn result(&self) -> Result<VoiceActivation, String> {
        let terminal = self
            .original
            .cell
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .terminal
            .expect("latched reply");
        match terminal {
            Terminal::Accepted(join) => Ok(VoiceActivation {
                owner: self.original.id().into(),
                group: Some(hex::encode(self.original.binding().scope.group)),
                join: Some(hex::encode(join)),
            }),
            Terminal::Refused(error) => Err(error.message()),
        }
    }
    /// Explicit observed-result acknowledgement, bound to this exact original Cell.
    /// Collection still independently requires actual Work/storage/task return.
    pub(crate) fn acknowledge(self) {
        self.original
            .cell
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .acknowledged = true;
    }
}
struct Publisher {
    handle: Handle,
    serial: u64,
    settled: bool,
}
impl Publisher {
    fn stage(&mut self, outcome: Outcome, work: Option<VoiceCallActivationWork>) {
        let mut pending = Some(outcome);
        {
            let mut state = self
                .handle
                .cell
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(inv) = state
                .invocation
                .as_mut()
                .filter(|inv| inv.serial == self.serial)
            {
                let same = match (&inv.work, &work) {
                    (Some(a), Some(b)) => a.is_same(b),
                    (None, None) => true,
                    _ => false,
                };
                if same && inv.outcome.is_none() {
                    inv.outcome = pending.take();
                }
            }
        }
        if pending.is_some() {
            self.handle.close();
        }
        self.settled = true;
        self.handle.cell.changed.notify_waiters();
        drop(pending);
    }
}
impl VoiceCallActivationPublisher for Publisher {
    fn accepted(&mut self, work: VoiceCallActivationWork) -> Result<(), VoiceEndpointError> {
        let mut state = self
            .handle
            .cell
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let inv = state
            .invocation
            .as_mut()
            .filter(|inv| inv.serial == self.serial)
            .ok_or(VoiceEndpointError::Stopped)?;
        if inv.work.is_some() {
            return Err(VoiceEndpointError::Protocol(VoiceError::RetryMismatch));
        }
        inv.work = Some(work);
        if inv.kind == Kind::Select && self.handle.original().is_closed() {
            Err(VoiceEndpointError::Stopped)
        } else {
            Ok(())
        }
    }
    fn completed(
        mut self: Box<Self>,
        result: VoiceCallActivationResult,
        work: VoiceCallActivationWork,
    ) {
        self.stage(Outcome::Completed(result), Some(work));
    }
    fn refused(
        mut self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceCallActivationWork>,
        payment: Option<ActivationRpcPermit>,
    ) {
        self.stage(Outcome::Unstarted(error, payment), work);
    }
}
impl Drop for Publisher {
    fn drop(&mut self) {
        if !self.settled {
            self.stage(Outcome::Uncertain, self.handle.work());
        }
    }
}

#[derive(Clone)]
struct Context {
    servers: Weak<Mutex<HashMap<u64, ServerEntry>>>,
    store: Weak<Mutex<Option<crate::ServerStore>>>,
    resumable: Weak<Mutex<bool>>,
    requested: Weak<AtomicBool>,
    generation: Weak<AtomicU64>,
    authorized: Weak<AtomicU64>,
    commit: Weak<Mutex<()>>,
    authority: Weak<StdMutex<voice_authority::Coordinator>>,
}
struct LiveContext {
    servers: Arc<Mutex<HashMap<u64, ServerEntry>>>,
    store: Arc<Mutex<Option<crate::ServerStore>>>,
    resumable: Arc<Mutex<bool>>,
    requested: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    authorized: Arc<AtomicU64>,
    commit: Arc<Mutex<()>>,
    authority: Arc<StdMutex<voice_authority::Coordinator>>,
}
impl Context {
    fn new(state: &AppState) -> Self {
        Self {
            servers: Arc::downgrade(&state.servers),
            store: Arc::downgrade(&state.store),
            resumable: Arc::downgrade(&state.session_resumable),
            requested: Arc::downgrade(&state.session_lock_requested),
            generation: Arc::downgrade(&state.ui_session_generation),
            authorized: Arc::downgrade(&state.ui_authorized_generation),
            commit: Arc::downgrade(&state.ui_session_commit),
            authority: Arc::downgrade(&state.voice_authority),
        }
    }
    fn upgrade(&self) -> Option<LiveContext> {
        Some(LiveContext {
            servers: self.servers.upgrade()?,
            store: self.store.upgrade()?,
            resumable: self.resumable.upgrade()?,
            requested: self.requested.upgrade()?,
            generation: self.generation.upgrade()?,
            authorized: self.authorized.upgrade()?,
            commit: self.commit.upgrade()?,
            authority: self.authority.upgrade()?,
        })
    }
    fn generation_current(&self, expected: u64) -> bool {
        let (Some(requested), Some(generation), Some(authorized)) = (
            self.requested.upgrade(),
            self.generation.upgrade(),
            self.authorized.upgrade(),
        ) else {
            return false;
        };
        !requested.load(Ordering::Acquire)
            && generation.load(Ordering::Acquire) == expected
            && authorized.load(Ordering::Acquire) == expected
    }
}
impl LiveContext {
    fn gate(&self) -> UiSessionGate<'_> {
        UiSessionGate {
            resumable: &self.resumable,
            requested: &self.requested,
            generation: &self.generation,
            authorized: &self.authorized,
            store: &self.store,
        }
    }
}

/// Marker owns no Cell/Root/Actor backedge. It follows all actual task arguments,
/// before the final plain payer. The original signal is never a caller-supplied bool.
struct TaskReturned {
    signal: Arc<AtomicBool>,
    _funding: Arc<Funding>,
}
impl Drop for TaskReturned {
    fn drop(&mut self) {
        self.signal.store(true, Ordering::Release);
    }
}
struct Worker {
    commit: Option<OwnedMutexGuard<()>>,
    context: Context,
    handle: Handle,
    negative_only: bool,
    // LAST owned argument: every actual Context/Handle/guard alias returns first.
    returned: TaskReturned,
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Taking the guard local also orders unwind if a framework wake panics:
        // the shared gate is closed before it can be released, and heavy fields
        // then destroy outside UI. This RAII exists BEFORE spawn/first polling.
        let commit = self.commit.take();
        self.handle.latch_refusal(Failure::Interrupted);
        drop(commit);
        self.handle.notify_closed();
    }
}
impl Handle {
    fn initial_worker(&self, context: Context, commit: OwnedMutexGuard<()>) -> Worker {
        let signal = {
            let mut state = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
            assert!(!state.task_started, "one canonical task");
            state.task_started = true;
            state.task.returned.clone()
        };
        Worker {
            commit: Some(commit),
            context,
            handle: self.clone(),
            negative_only: false,
            returned: TaskReturned {
                signal,
                _funding: self.funding.clone(),
            },
        }
    }
    fn negative_worker(&self, context: Context) -> Option<Worker> {
        let signal = {
            let mut state = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
            if !self.original().is_closed()
                || state.cleanup_done
                || !state.task_started
                || !state.task.returned.load(Ordering::Acquire)
            {
                return None;
            }
            // Consume the EXACT old returned ticket under the SAME Cell. Concurrent
            // close/prepare/recovery hooks cannot allocate overlapping workers on
            // one payer. No positive path is reachable from a replacement task.
            let serial = state.task.serial.checked_add(1)?;
            let signal = Arc::new(AtomicBool::new(false));
            state.task = TaskTicket {
                serial,
                returned: signal.clone(),
            };
            signal
        };
        Some(Worker {
            commit: None,
            context,
            handle: self.clone(),
            negative_only: true,
            returned: TaskReturned {
                signal,
                _funding: self.funding.clone(),
            },
        })
    }
    fn control(&self) -> Option<VoiceActivationControl> {
        self.cell
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .control
            .clone()
    }
    fn restore_payment(&self, kind: Kind, payment: Option<ActivationRpcPermit>) {
        let Some(payment) = payment else { return };
        let mut state = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
        let slot = match kind {
            Kind::Select => &mut state.select_payment,
            Kind::Clear => &mut state.clear_payment,
        };
        if slot.is_none() {
            *slot = Some(payment);
        }
    }
    fn capture_storage(&self) -> bool {
        let proof = self
            .original()
            .root_returned()
            .map(StorageProof::Returned)
            .or_else(|| {
                self.original()
                    .seal_unclaimed_residence()
                    .map(StorageProof::Unclaimed)
            });
        if let Some(proof) = proof {
            self.cell
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .storage = Some(proof);
            true
        } else {
            false
        }
    }
    fn release_grant(&self) {
        let grant = self
            .cell
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .grant
            .take();
        drop(grant); // after actual argument retirement; never under Cell/UI/Coordinator
    }
}
impl Worker {
    fn release_commit(&mut self) {
        drop(self.commit.take());
    }
    fn fail(&mut self, failure: Failure) {
        self.handle.latch_refusal(failure);
        self.release_commit();
        self.handle.notify_closed();
    }
    fn final_admission(&self, selected: RootSelectedCall) -> Result<(), Failure> {
        let guard = self.commit.as_ref().ok_or(Failure::Revoked)?;
        let live = self.context.upgrade().ok_or(Failure::Revoked)?;
        if !Arc::ptr_eq(OwnedMutexGuard::mutex(guard), &live.commit) {
            return Err(Failure::Revoked);
        }
        live.gate()
            .check_generation(self.handle.binding().generation)
            .map_err(|_| Failure::Revoked)?;
        {
            let servers = live.servers.try_lock().map_err(|_| Failure::Capacity)?;
            let binding = self.handle.binding();
            if !servers.get(&binding.server).is_some_and(|entry| {
                entry.instance == binding.instance && entry.group_id == binding.group
            }) {
                return Err(Failure::Revoked);
            }
        }
        let mut authority = live.authority.lock().unwrap_or_else(|e| e.into_inner());
        // Pure final check of the ORIGINAL held guard/generation; do NOT reacquire it.
        live.gate()
            .check_generation(self.handle.binding().generation)
            .map_err(|_| Failure::Revoked)?;
        authority
            .finish_admitted_activation(&self.handle, selected)
            .map_err(Failure::Protocol)
    }
    fn retire_original_native_owner(&self) {
        if let Some(live) = self.context.upgrade() {
            let owner = live
                .authority
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retire_admitted_activation(&self.handle);
            drop(owner); // no last affine Grant/Room destructor under Coordinator/UI
        }
    }
    async fn wait_actual_invocation(&self) {
        self.handle.outcome_ready().await;
        if let Some(work) = self.handle.work() {
            work.wait_retired().await;
        }
    }
    /// Observe the SAME original phase and UI generation. Do not subscribe an
    /// admitted child-watch waiter here: inherited Runtime/Coordinator revocation
    /// may send that watch while its guards are held. The Clock supplies only a
    /// bounded observation wake; it cannot keep an expired/closed gate current.
    async fn wait_close_fence(&self) -> Result<(), Failure> {
        loop {
            if self.handle.original().is_closed()
                || !self
                    .context
                    .generation_current(self.handle.binding().generation)
            {
                return Ok(());
            }
            cadence_tick(self.handle.cell.clock.as_ref()).await?;
        }
    }
    async fn run(mut self) {
        if !self.negative_only && !self.handle.original().is_closed() {
            let queued = match (
                self.handle.control(),
                self.handle.grant(),
                self.handle.begin(Kind::Select),
            ) {
                (Some(control), Some(grant), Ok((_, payment, publisher))) => {
                    let result = control.try_queue_voice_call_selection(
                        grant,
                        self.handle.original().clone(),
                        payment,
                        publisher,
                    );
                    drop(result); // original canonical Work/outcome, not this observer, owns it
                    true
                }
                (_, _, Err(error)) => {
                    self.fail(error);
                    false
                }
                _ => {
                    self.fail(Failure::Stopped);
                    false
                }
            };
            if queued {
                let closed = tokio::select! {
                    biased;
                    closed = self.wait_close_fence() => Some(closed),
                    _ = self.wait_actual_invocation() => None,
                };
                if let Some(closed) = closed {
                    self.fail(closed.err().unwrap_or(Failure::Revoked));
                } else {
                    match self.handle.take_outcome() {
                        Some(Outcome::Completed(VoiceCallActivationResult::Selected(Ok(
                            selected,
                        )))) => {
                            if let Err(error) = self.final_admission(selected) {
                                self.fail(error);
                            } else {
                                self.release_commit();
                                self.handle.cell.changed.notify_waiters();
                            }
                        }
                        Some(Outcome::Completed(VoiceCallActivationResult::Selected(Err(
                            error,
                        )))) => self.fail(Failure::Protocol(error)),
                        Some(Outcome::Unstarted(error, payment)) => {
                            self.handle.restore_payment(Kind::Select, payment);
                            self.fail(match error {
                                VoiceEndpointError::Stopped => Failure::Stopped,
                                VoiceEndpointError::Protocol(VoiceError::Capacity) => {
                                    Failure::Capacity
                                }
                                VoiceEndpointError::Protocol(error) => Failure::Protocol(error),
                            });
                        }
                        _ => self.fail(Failure::Interrupted),
                    }
                }
            }
        } else {
            self.fail(Failure::Revoked);
        }
        if self.handle.active() {
            // No UI guard/SDK/HTTP/network is held while the logical call is active.
            if let Err(error) = self.wait_close_fence().await {
                self.fail(error);
            }
        }
        self.handle.close();
        self.release_commit();
        self.retire_original_native_owner();
        // Cancellation never discards actual command/argument custody. A parked
        // Actor can outlive lock completion while this original paid task waits here.
        if self
            .handle
            .cell
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .invocation
            .is_some()
        {
            self.wait_actual_invocation().await;
            drop(self.handle.take_outcome());
        }
        self.handle.release_grant();
        self.negative_cleanup().await;
    }
    async fn negative_cleanup(&self) {
        loop {
            let Some(control) = self.handle.control() else {
                if self.handle.capture_storage() {
                    break;
                }
                if cadence_tick(self.handle.cell.clock.as_ref()).await.is_err() {
                    return;
                }
                continue;
            };
            let clear = self.handle.begin(Kind::Clear);
            if let Ok((_, payment, publisher)) = clear {
                drop(control.try_queue_voice_call_clear(
                    self.handle.original().clone(),
                    payment,
                    publisher,
                ));
                drop(control);
                self.wait_actual_invocation().await;
                match self.handle.take_outcome() {
                    Some(Outcome::Completed(VoiceCallActivationResult::Cleared(Ok(())))) => {
                        let control = self
                            .handle
                            .cell
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .control
                            .take();
                        drop(control);
                    }
                    Some(Outcome::Unstarted(error, payment)) => {
                        self.handle.restore_payment(Kind::Clear, payment);
                        if error == VoiceEndpointError::Stopped {
                            let control = self
                                .handle
                                .cell
                                .state
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .control
                                .take();
                            drop(control);
                        }
                    }
                    _ => {
                        let control = self
                            .handle
                            .cell
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .control
                            .take();
                        drop(control);
                    }
                }
            } else {
                // An entered ambiguous/queued-destruction path cannot mint another
                // payment or restart an Actor. Its real return marker stays decisive.
                drop(control);
                let control = self
                    .handle
                    .cell
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .control
                    .take();
                drop(control);
            }
            if self.handle.capture_storage() {
                break;
            }
            // A faulty clock leaves bounded paid negative debt in the canonical
            // Cell. Do not spin, mint another task/payment, or pretend collection.
            if cadence_tick(self.handle.cell.clock.as_ref()).await.is_err() {
                return;
            }
        }
        let owners = {
            let mut state = self
                .handle
                .cell
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            state.cleanup_done = true;
            (
                state.owner.take(),
                state.control.take(),
                state.select_payment.take(),
                state.clear_payment.take(),
            )
        };
        drop(owners);
        self.handle.cell.changed.notify_waiters();
    }
}

/// One sanctioned positive-duration Clock registration. A broken inline/early/
/// backwards/exhausted source is a local refusal, never an always-ready retry loop
/// or a deadline extension. A completed Root command remains independently
/// pollable and cancels this exact original timer future.
async fn cadence_tick(clock: &dyn Clock) -> Result<(), Failure> {
    let before = clock.monotonic_ms();
    let deadline = before.checked_add(CADENCE_MS).ok_or(Failure::Interrupted)?;
    let sleeper = clock.sleep(std::time::Duration::from_millis(CADENCE_MS));
    tokio::pin!(sleeper);
    let mut suspended = false;
    let actually_suspended = poll_fn(|cx| match sleeper.as_mut().poll(cx) {
        Poll::Pending => {
            suspended = true;
            Poll::Pending
        }
        Poll::Ready(()) => Poll::Ready(suspended),
    })
    .await;
    let after = clock.monotonic_ms();
    if !actually_suspended || after < deadline {
        return Err(Failure::Interrupted);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
