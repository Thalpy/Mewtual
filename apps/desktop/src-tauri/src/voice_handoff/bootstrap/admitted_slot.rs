//! One original admitted neutral-preparation task on the admitted physical Row.
//! No peer, role, RoundGate, Action or Ready can be produced by this module.
use super::*;
use crate::voice_call_activation::Handle;
use crate::voice_handoff::context::{Context, LiveContext};
use crate::voice_handoff::ledger::AdmittedParentClaim;
use crate::voice_handoff::requests::admitted_admission::{Kind, Request, Snapshot, State as View};
use catcoms_app::{
    VoiceAdmittedAdmissionPublisher, VoiceAdmittedAdmissionResult, VoiceAdmittedAdmissionWork,
    VoiceEndpointError,
};
use catcoms_sync::voice_v2::admission::{AdmissionRpcPermit, ADMISSION_RPC_BYTES};
use catcoms_sync::voice_v2::admitted_admission::{AdmittedAdmissionSession, RootRoundPreclaim};
use catcoms_sync::voice_v2::resources::ResourceKind;
use catcoms_sync::voice_v2::VoiceError;

const TASK_BYTES: usize = 8192;
struct Funding {
    _credit: Credit,
}
struct FundingOwner(Option<Arc<Funding>>);
impl Clone for FundingOwner {
    fn clone(&self) -> Self {
        Self(Some(self.0.as_ref().expect("original funding").clone()))
    }
}
impl Drop for FundingOwner {
    fn drop(&mut self) {
        if let Some(value) = self.0.take().and_then(Arc::into_inner) {
            drop(value);
        }
    }
}
/// The SAME payment is outside the Box and outlives actual Box deallocation.
pub(super) struct PaidSlot {
    body: Option<Box<Slot>>,
    _funding: FundingOwner,
}
impl Drop for PaidSlot {
    fn drop(&mut self) {
        drop(self.body.take());
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting,
    Pending,
    Staged,
    Finalizing,
    Reserved,
    Returning,
    Returned,
    Uncertain,
    Closed,
}
struct InvocationId;
struct Invocation {
    id: Arc<InvocationId>,
    work: Option<VoiceAdmittedAdmissionWork>,
    outcome: Option<Outcome>,
}
// Already-funded affine candidates must remain inline when ordinary allocation
// is full; boxing an error/retry would introduce a new fallible owner then.
#[allow(clippy::large_enum_variant)]
enum Outcome {
    Prepared(VoiceAdmittedAdmissionResult),
    // A pre-enqueue refusal returns this SAME affine contribution. Some Work
    // instead owns its contribution through actual argument/callback return.
    Unstarted(VoiceEndpointError, Option<AdmissionRpcPermit>),
    Candidate(RootRoundPreclaim, Option<AdmittedParentClaim>),
    Unknown,
}
struct Slot {
    generation: u64,
    cancelled: u64,
    phase: Phase,
    closed: bool,
    uncertain: bool,
    retry: bool,
    registration: Option<AdmittedAdmissionSession>,
    capacity: Option<RootRoundPreclaim>,
    parent: Option<AdmittedParentClaim>,
    invocation: Option<Invocation>,
    payment: Option<AdmissionRpcPermit>,
    task_returned: bool,
    emergency: Option<crate::voice_handoff::channel::NativeHold>,
    recovering: bool,
    #[cfg(test)]
    unwind_reclamation: bool,
    #[cfg(test)]
    cancel_after_accept: bool,
    #[cfg(test)]
    cancel_before_attach: bool,
}
impl Slot {
    fn close(&mut self) {
        self.closed = true;
        if let Some(original) = &self.registration {
            original.close();
        }
    }
    fn drained(&self) -> bool {
        self.registration.is_none()
            && self.capacity.is_none()
            && self.parent.is_none()
            && self.invocation.is_none()
            && self.payment.is_none()
            && self.emergency.is_none()
            && !self.recovering
            && self.task_returned
    }
    fn validate(&self, g: u64) -> Result<(), Refused> {
        if g == 0 {
            return Err(Refused::Bounds);
        }
        if g <= self.generation {
            return Ok(());
        }
        if self.generation.checked_add(1) == Some(g)
            && self.phase == Phase::Returned
            && self.cancelled >= self.generation
            && self.capacity.is_none()
            && self.parent.is_none()
            && self.invocation.is_none()
            && !self.recovering
            && self.emergency.is_none()
        {
            return Ok(());
        }
        Err(Refused::Bounds)
    }
}
impl AdmittedStage {
    pub(super) fn close(&mut self) {
        if let Self::Admission(slot) = self {
            slot.body.as_mut().expect("original Slot").close();
        }
    }
    pub(super) fn collected(&self) -> bool {
        match self {
            Self::BootstrapOnly | Self::CancelledBeforeStart(_) => true,
            Self::Admission(slot) => slot.body.as_ref().expect("original Slot").drained(),
        }
    }
}
fn slot(state: &State) -> Option<&Slot> {
    match state.admitted_stage.as_ref()? {
        AdmittedStage::Admission(paid) => paid.body.as_deref(),
        _ => None,
    }
}
fn slot_mut(state: &mut State) -> Option<&mut Slot> {
    match state.admitted_stage.as_mut()? {
        AdmittedStage::Admission(paid) => paid.body.as_deref_mut(),
        _ => None,
    }
}
fn pre_task_floor(state: &State) -> u64 {
    match state.admitted_stage.as_ref() {
        Some(AdmittedStage::CancelledBeforeStart(through)) => *through,
        _ => 0,
    }
}

/// Actual original currentness, never the historical selected-key wrapper alone.
fn current(live: &LiveContext, route: &AdmittedRoute) -> Result<Handle, Refused> {
    let selected = route.selected();
    live.gate()
        .check_generation(selected.binding().generation)
        .map_err(|_| Refused::Binding)?;
    if !live.try_unlocked().map_err(|_| Refused::Capacity)? {
        return Err(Refused::Binding);
    }
    {
        let servers = live.servers.try_lock().map_err(|_| Refused::Capacity)?;
        let binding = selected.binding();
        if !servers.get(&binding.server).is_some_and(|entry| {
            entry.instance == binding.instance && entry.group_id == binding.group
        }) {
            return Err(Refused::Binding);
        }
    }
    let authority = live.authority.try_lock().map_err(|_| Refused::Capacity)?;
    exact(&authority, route)
}
fn exact(
    authority: &crate::voice_authority::Coordinator,
    route: &AdmittedRoute,
) -> Result<Handle, Refused> {
    let handle = authority
        .admitted_activation(route.selected().id())
        .ok_or(Refused::Binding)?;
    if !authority.is_current_admitted_published_key(&handle, route.selected()) {
        return Err(Refused::Binding);
    }
    Ok(handle)
}

impl AdmittedRoute {
    pub(super) fn admitted_snapshot(&self, requested: u64) -> Result<Snapshot, Refused> {
        let state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = slot(&state) else {
            let cancelled = pre_task_floor(&state);
            if requested > cancelled && cancelled.checked_add(1) != Some(requested) {
                return Err(Refused::Bounds);
            }
            return Ok(Snapshot {
                state: if state.closed || !self.selected().lease().is_live() {
                    View::Closed
                } else if requested != 0 && requested < cancelled {
                    View::Superseded
                } else if cancelled != 0 && (requested == 0 || requested == cancelled) {
                    View::Returned
                } else {
                    View::Unstarted
                },
                requested,
                canonical: cancelled,
                cancelled,
                incarnation: None,
            });
        };
        if requested != 0 {
            s.validate(requested)?;
        }
        let view = if requested != 0 && requested < s.generation {
            View::Superseded
        } else if s.uncertain {
            View::Uncertain
        } else {
            match s.phase {
                Phase::Waiting => View::Waiting,
                Phase::Pending | Phase::Staged | Phase::Finalizing => View::Preparing,
                Phase::Reserved => View::Reserved,
                Phase::Returning => View::Closing,
                Phase::Returned => View::Returned,
                Phase::Uncertain => View::Uncertain,
                Phase::Closed => View::Closed,
            }
        };
        let canceled = (requested == 0 || requested == s.generation)
            && s.cancelled >= s.generation
            && !matches!(s.phase, Phase::Returned | Phase::Closed | Phase::Uncertain);
        let closing =
            (s.closed || state.closed || state.effects_closed) && s.phase != Phase::Closed;
        let stale_reserved =
            view == View::Reserved && s.capacity.as_ref().is_none_or(|c| !c.is_current());
        let view = if !s.uncertain && (closing || canceled || stale_reserved) {
            View::Closing
        } else {
            view
        };
        Ok(Snapshot {
            state: view,
            requested,
            canonical: s.generation,
            cancelled: s.cancelled,
            incarnation: s
                .registration
                .as_ref()
                .map(AdmittedAdmissionSession::incarnation),
        })
    }
    pub(in crate::voice_handoff) fn admitted_control(
        &self,
        context: &Context,
        request: Request,
    ) -> Result<Snapshot, Refused> {
        match request.kind {
            Kind::SetupAck => {
                // Only the trusted retained driver may send this after all real
                // setup SDK promises return. This is NOT their return certificate.
                self.row.retire_setup()?;
            }
            Kind::Observe => self.recover_admitted(),
            Kind::Register => register(context, self, request.generation)?,
            Kind::Cancel => self.cancel_generation(request.generation)?,
        }
        self.admitted_snapshot(request.generation)
    }
    fn cancel_generation(&self, g: u64) -> Result<(), Refused> {
        if g == 0 {
            return Err(Refused::Bounds);
        }
        let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = slot_mut(&mut state) {
            s.validate(g)?; // validates skips/MAX and actual prior return BEFORE the floor
            if g > s.generation {
                // The exact next request may have been lost BEFORE native entry.
                // This is negative advancement only: no task/RPC/mint is created.
                s.generation = g;
                s.phase = Phase::Returned;
                s.retry = false;
            }
            s.cancelled = s.cancelled.max(g);
            if let Some(original) = &s.registration {
                original
                    .observer()
                    .cancel_through(g)
                    .map_err(|_| Refused::Bounds)?;
            }
        } else {
            let through = pre_task_floor(&state);
            if g > through && through.checked_add(1) != Some(g) {
                return Err(Refused::Bounds);
            }
            if g > through {
                state.admitted_stage = Some(AdmittedStage::CancelledBeforeStart(g));
            }
        }
        drop(state);
        self.row.admission_changed.notify_waiters();
        Ok(())
    }
    pub(in crate::voice_handoff) fn nudge_admitted(&self) {
        self.recover_admitted();
        {
            let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = slot_mut(&mut state) {
                if !s.closed {
                    s.retry = true;
                }
            }
        }
        self.row.admission_changed.notify_waiters();
    }
    pub(super) fn admitted_work(&self) -> Option<VoiceAdmittedAdmissionWork> {
        let state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        slot(&state)?.invocation.as_ref()?.work.clone()
    }
    /// Negative-only synchronous recovery after the original task disappeared.
    /// Its SAME native position and already-paid results are collected, never
    /// replaced by another task/RPC/registration.
    pub(super) fn recover_admitted(&self) {
        let native = {
            let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(s) = slot_mut(&mut state) else {
                return;
            };
            if !s.task_returned
                || s.recovering
                || s.invocation
                    .as_ref()
                    .and_then(|i| i.work.as_ref())
                    .is_some_and(|w| !w.is_retired())
            {
                return;
            }
            s.close();
            s.recovering = true;
            s.emergency.take()
        };
        let recovery = Recovery {
            route: self.clone(),
            native,
        };
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reclaim(self, true))).is_err() {
            let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = slot_mut(&mut state) {
                s.close();
                s.uncertain = true;
                s.phase = Phase::Uncertain;
            }
        }
        drop(recovery);
    }
}
struct Recovery {
    route: AdmittedRoute,
    native: Option<crate::voice_handoff::channel::NativeHold>,
}
impl Drop for Recovery {
    fn drop(&mut self) {
        drop(self.native.take());
        let mut state = self
            .route
            .row
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(s) = slot_mut(&mut state) {
            s.recovering = false;
        }
        drop(state);
        self.route.row.admission_changed.notify_waiters();
    }
}

struct Custody {
    route: AdmittedRoute,
    returned: TaskReturned,
}
struct TaskReturned {
    row: RowRef,
    native: Option<crate::voice_handoff::channel::NativeHold>,
    armed: bool,
    _funding: FundingOwner,
}
impl Drop for TaskReturned {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut native = self.native.take();
        {
            let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = slot_mut(&mut state) {
                s.close();
                if s.registration.is_some()
                    || s.capacity.is_some()
                    || s.parent.is_some()
                    || s.invocation.is_some()
                    || s.payment.is_some()
                {
                    if s.emergency.is_none() {
                        s.emergency = native.take();
                    }
                }
            }
        }
        drop(native);
        let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = slot_mut(&mut state) {
            s.task_returned = true;
        }
        drop(state);
        self.row.admission_changed.notify_waiters();
    }
}

fn register(context: &Context, route: &AdmittedRoute, g: u64) -> Result<(), Refused> {
    register_with(context, route, g, |_| {})
}
fn register_with(
    context: &Context,
    route: &AdmittedRoute,
    g: u64,
    spawned: impl FnOnce(tokio::task::AbortHandle),
) -> Result<(), Refused> {
    if g == 0 {
        return Err(Refused::Bounds);
    }
    let live = context.upgrade().ok_or(Refused::Binding)?;
    let _handle = current(&live, route)?;
    let exists = {
        let commit = live
            .commit
            .clone()
            .try_lock_owned()
            .map_err(|_| Refused::Capacity)?;
        let _handle = current(&live, route)?;
        let authority = live.authority.try_lock().map_err(|_| Refused::Capacity)?;
        let _handle = exact(&authority, route)?;
        let mut state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || state.effects_closed || state.pending || state.browser_setup.is_some() {
            return Err(Refused::Binding);
        }
        if let Some(s) = slot_mut(&mut state) {
            s.validate(g)?;
            if s.closed {
                return Err(Refused::Binding);
            }
            if g > s.generation {
                s.generation = g;
                s.phase = Phase::Waiting;
            }
            s.retry = true;
            drop(state);
            drop(authority);
            drop(commit);
            true
        } else {
            let through = pre_task_floor(&state);
            if g <= through {
                return Ok(());
            } // late Register cannot revive canceled intent
            if through.checked_add(1) != Some(g) {
                return Err(Refused::Bounds);
            }
            false
        }
    };
    if exists {
        route.row.admission_changed.notify_waiters();
        return Ok(());
    }
    let initial_cancelled =
        pre_task_floor(&route.row.state.lock().unwrap_or_else(|e| e.into_inner()));
    if initial_cancelled.checked_add(1) != Some(g) {
        return Err(Refused::Bounds);
    }
    if task_layout_bytes() > TASK_BYTES || rpc_layout_bytes() > ADMISSION_RPC_BYTES {
        return Err(Refused::Capacity);
    }
    let credit = route
        .selected()
        .resources()
        .charge(
            route.selected().binding().scope.group,
            TASK_BYTES,
            ResourceKind::Retained,
        )
        .map_err(|_| Refused::Capacity)?;
    let funding = FundingOwner(Some(Arc::new(Funding { _credit: credit })));
    let native = route
        .session()
        .native_hold()
        .map_err(|_| Refused::Capacity)?;
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| Refused::Binding)?;
    let mut custody = Custody {
        route: route.clone(),
        returned: TaskReturned {
            row: route.row.clone(),
            native: Some(native),
            armed: false,
            _funding: funding.clone(),
        },
    };
    let paid = PaidSlot {
        body: Some(Box::new(Slot {
            generation: g,
            cancelled: initial_cancelled,
            phase: Phase::Waiting,
            closed: false,
            uncertain: false,
            retry: true,
            registration: None,
            capacity: None,
            parent: None,
            invocation: None,
            payment: None,
            task_returned: false,
            emergency: None,
            recovering: false,
            #[cfg(test)]
            unwind_reclamation: false,
            #[cfg(test)]
            cancel_after_accept: false,
            #[cfg(test)]
            cancel_before_attach: false,
        })),
        _funding: funding,
    };
    let mut paid = Some(paid);
    {
        let _commit = live
            .commit
            .clone()
            .try_lock_owned()
            .map_err(|_| Refused::Capacity)?;
        let _original = current(&live, route)?;
        let authority = live.authority.try_lock().map_err(|_| Refused::Capacity)?;
        let _original = exact(&authority, route)?;
        let mut state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed
            || state.effects_closed
            || state.browser_setup.is_some()
            || slot(&state).is_some()
            || pre_task_floor(&state) != initial_cancelled
        {
            return Err(Refused::Binding);
        }
        state.admitted_stage = Some(AdmittedStage::Admission(
            paid.take().expect("original paid Slot"),
        ));
        custody.returned.armed = true;
    }
    drop(live);
    let task = runtime.spawn(run(context.clone(), custody));
    spawned(task.abort_handle());
    drop(task); // no task/Row ownership cycle
    Ok(())
}

struct Publisher {
    route: AdmittedRoute,
    id: Arc<InvocationId>,
    settled: bool,
}
impl Publisher {
    fn stage(&mut self, mut outcome: Option<Outcome>, work: Option<VoiceAdmittedAdmissionWork>) {
        {
            let mut state = self
                .route
                .row
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(s) = slot_mut(&mut state) {
                if let Some(i) = s
                    .invocation
                    .as_mut()
                    .filter(|i| Arc::ptr_eq(&i.id, &self.id))
                {
                    let same = match (&i.work, &work) {
                        (None, None) => true,
                        (Some(a), Some(b)) => a.is_same(b),
                        _ => false,
                    };
                    if same && i.outcome.is_none() {
                        i.outcome = outcome.take();
                        s.phase = Phase::Staged;
                    } else {
                        s.close();
                        s.uncertain = true;
                        s.phase = Phase::Uncertain;
                    }
                } else {
                    s.close();
                    s.uncertain = true;
                    s.phase = Phase::Uncertain;
                }
            }
        }
        self.settled = true;
        self.route.row.admission_changed.notify_waiters();
        drop(outcome);
    }
}
impl VoiceAdmittedAdmissionPublisher for Publisher {
    fn accepted(&mut self, work: VoiceAdmittedAdmissionWork) -> Result<(), VoiceEndpointError> {
        let mut state = self
            .route
            .row
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let closed =
            state.closed || state.effects_closed || !self.route.selected().lease().is_live();
        let s = slot_mut(&mut state).ok_or(VoiceEndpointError::Stopped)?;
        let i = s
            .invocation
            .as_mut()
            .filter(|i| Arc::ptr_eq(&i.id, &self.id))
            .ok_or(VoiceEndpointError::Stopped)?;
        if i.work.is_some() {
            return Err(VoiceEndpointError::Protocol(VoiceError::RetryMismatch));
        }
        i.work = Some(work);
        let refused = closed || s.closed || s.cancelled >= s.generation;
        #[cfg(test)]
        let cancel = (std::mem::take(&mut s.cancel_after_accept), s.generation);
        drop(state);
        if refused {
            return Err(VoiceEndpointError::Stopped);
        }
        // Deterministically reproduce a concurrent authenticated Cancel AFTER
        // acceptance checks but BEFORE real enqueue. This uses the actual
        // negative API, so Root observes its genuine original observer floor.
        #[cfg(test)]
        if cancel.0 {
            self.route.cancel_generation(cancel.1).unwrap();
        }
        Ok(())
    }
    fn completed(
        mut self: Box<Self>,
        result: VoiceAdmittedAdmissionResult,
        work: VoiceAdmittedAdmissionWork,
    ) {
        self.stage(Some(Outcome::Prepared(result)), Some(work));
    }
    fn refused(
        mut self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceAdmittedAdmissionWork>,
        payment: Option<AdmissionRpcPermit>,
    ) {
        self.stage(Some(Outcome::Unstarted(error, payment)), work);
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
            .unwrap_or_else(|e| e.into_inner());
        if let Some(s) = slot_mut(&mut state) {
            if let Some(i) = s
                .invocation
                .as_mut()
                .filter(|i| Arc::ptr_eq(&i.id, &self.id))
            {
                if i.outcome.is_none() {
                    i.outcome = Some(Outcome::Unknown);
                }
            }
            if s.invocation
                .as_ref()
                .is_some_and(|i| Arc::ptr_eq(&i.id, &self.id))
            {
                s.close();
                s.uncertain = true;
                s.phase = Phase::Uncertain;
            }
        }
        drop(state);
        self.route.row.admission_changed.notify_waiters();
    }
}

fn queue(context: &Context, route: &AdmittedRoute) -> Result<(), Refused> {
    let live = context.upgrade().ok_or(Refused::Binding)?;
    let handle = current(&live, route)?;
    let control = handle.publication_control().ok_or(Refused::Binding)?;
    let grant = handle.grant().ok_or(Refused::Binding)?;
    let (g, original, payment) = {
        let mut state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
        let s = slot_mut(&mut state).ok_or(Refused::Binding)?;
        if s.closed || s.cancelled >= s.generation || s.phase != Phase::Waiting || !s.retry {
            return Ok(());
        }
        // Only an explicit Register or the existing bounded Host cadence grants
        // another turn. A fast Capacity callback cannot spin/requeue itself.
        s.retry = false;
        (
            s.generation,
            s.registration
                .as_ref()
                .map(AdmittedAdmissionSession::observer),
            s.payment.take(),
        )
    };
    let payment = match payment {
        Some(p) => p,
        None => AdmissionRpcPermit::reserve(
            &route.selected().resources(),
            route.selected().binding().scope,
        )
        .map_err(|_| Refused::Capacity)?,
    };
    let mut payment = UnqueuedPayment {
        route: route.clone(),
        payment: Some(payment),
    };
    let id = Arc::new(InvocationId);
    let mut publisher = Box::new(Publisher {
        route: route.clone(),
        id: id.clone(),
        settled: false,
    });
    {
        let _commit = live
            .commit
            .clone()
            .try_lock_owned()
            .map_err(|_| Refused::Capacity)?;
        let _original = current(&live, route)?;
        let authority = live.authority.try_lock().map_err(|_| Refused::Capacity)?;
        let _original = exact(&authority, route)?;
        let mut state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
        let s = slot_mut(&mut state).ok_or(Refused::Binding)?;
        if s.closed
            || s.generation != g
            || s.cancelled >= g
            || s.invocation.is_some()
            || s.phase != Phase::Waiting
        {
            publisher.settled = true;
            return Err(Refused::Binding);
        }
        s.invocation = Some(Invocation {
            id,
            work: None,
            outcome: None,
        });
        s.phase = Phase::Pending;
    }
    drop(live);
    // All native guards have returned before accepted/completed/refused can run.
    let _ = control.try_queue_voice_admitted_admission(
        grant,
        handle.original().clone(),
        original,
        g,
        payment.payment.take().expect("original payment"),
        publisher,
    );
    Ok(())
}
struct UnqueuedPayment {
    route: AdmittedRoute,
    payment: Option<AdmissionRpcPermit>,
}
impl Drop for UnqueuedPayment {
    fn drop(&mut self) {
        let mut payment = self.payment.take();
        {
            let mut state = self
                .route
                .row
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(s) = slot_mut(&mut state) {
                if !s.closed && s.payment.is_none() {
                    s.payment = payment.take();
                }
            }
        }
        drop(payment);
    }
}

struct Reclamation {
    _owners: Reclaimed,
    _returned: ReclaimedMarker,
}
struct Reclaimed {
    _registration: Option<AdmittedAdmissionSession>,
    _capacity: Option<RootRoundPreclaim>,
    _parent: Option<AdmittedParentClaim>,
    _invocation: Option<Invocation>,
    _payment: Option<AdmissionRpcPermit>,
    #[cfg(test)]
    unwind: bool,
}
impl Drop for Reclaimed {
    fn drop(&mut self) {
        // Rust continues dropping every affine field, then ReclaimedMarker,
        // during this controlled first unwind. No production callback is added.
        #[cfg(test)]
        if self.unwind {
            panic!("controlled actual admitted reclamation unwind");
        }
    }
}
struct ReclaimedMarker {
    route: AdmittedRoute,
    g: u64,
    whole: bool,
}
impl Drop for ReclaimedMarker {
    fn drop(&mut self) {
        let mut state = self
            .route
            .row
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(s) = slot_mut(&mut state) {
            if s.generation == self.g {
                s.phase = if self.whole {
                    if s.uncertain {
                        Phase::Uncertain
                    } else {
                        Phase::Closed
                    }
                } else {
                    Phase::Returned
                };
            }
        }
        drop(state);
        self.route.row.admission_changed.notify_waiters();
    }
}
fn reclaim(route: &AdmittedRoute, whole: bool) {
    let reclaimed = {
        let mut state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = slot_mut(&mut state) else {
            return;
        };
        if s.phase == Phase::Returning
            || (!whole && s.phase == Phase::Returned)
            || (whole
                && s.registration.is_none()
                && s.capacity.is_none()
                && s.parent.is_none()
                && s.invocation.is_none()
                && s.payment.is_none())
            || s.invocation
                .as_ref()
                .and_then(|i| i.work.as_ref())
                .is_some_and(|w| !w.is_retired())
        {
            return;
        }
        s.phase = Phase::Returning;
        Reclamation {
            _owners: Reclaimed {
                _registration: if whole { s.registration.take() } else { None },
                _capacity: s.capacity.take(),
                _parent: s.parent.take(),
                _invocation: s.invocation.take(),
                _payment: if whole { s.payment.take() } else { None },
                #[cfg(test)]
                unwind: std::mem::take(&mut s.unwind_reclamation),
            },
            _returned: ReclaimedMarker {
                route: route.clone(),
                g: s.generation,
                whole,
            },
        }
    };
    drop(reclaimed); // Root/phase/Work/credits and stamp destruction outside every guard
}

struct Finalizing {
    route: AdmittedRoute,
    id: Arc<InvocationId>,
    outcome: Option<Outcome>,
    done: bool,
}
impl Drop for Finalizing {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let mut outcome = self.outcome.take().or(Some(Outcome::Unknown));
        {
            let mut state = self
                .route
                .row
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(s) = slot_mut(&mut state) {
                if let Some(i) = s
                    .invocation
                    .as_mut()
                    .filter(|i| Arc::ptr_eq(&i.id, &self.id))
                {
                    if i.outcome.is_none() {
                        i.outcome = outcome.take();
                        s.phase = Phase::Staged;
                    }
                }
            }
        }
        drop(outcome);
    }
}
fn finish(context: &Context, route: &AdmittedRoute) -> Result<(), Refused> {
    let mut fin = {
        let mut state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
        let s = slot_mut(&mut state).ok_or(Refused::Binding)?;
        let Some(i) = s.invocation.as_mut() else {
            return Ok(());
        };
        if i.work.as_ref().is_some_and(|w| !w.is_retired()) || s.phase == Phase::Finalizing {
            return Ok(());
        }
        let Some(outcome) = i.outcome.take() else {
            return Ok(());
        };
        s.phase = Phase::Finalizing;
        Finalizing {
            route: route.clone(),
            id: i.id.clone(),
            outcome: Some(outcome),
            done: false,
        }
    };
    match fin.outcome.take().expect("original outcome") {
        Outcome::Prepared(result) => {
            let (mut registration, capacity) = match result {
                Ok(value) => value.into_parts(),
                Err(error) => (None, Err(error)),
            };
            {
                let mut state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
                let s = slot_mut(&mut state).ok_or(Refused::Binding)?;
                if registration.is_some() && s.registration.is_some() {
                    s.close();
                    s.uncertain = true;
                    s.phase = Phase::Uncertain;
                } else if registration.is_some() {
                    s.registration = registration.take();
                }
            }
            drop(registration);
            match capacity {
                Ok(capacity) => fin.outcome = Some(Outcome::Candidate(capacity, None)),
                Err(error) => {
                    let old = {
                        let mut state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
                        let s = slot_mut(&mut state).ok_or(Refused::Binding)?;
                        if error != VoiceError::Capacity && s.cancelled < s.generation {
                            s.close();
                        }
                        s.phase = Phase::Waiting;
                        s.invocation.take()
                    };
                    drop(old);
                    fin.done = true;
                }
            }
        }
        Outcome::Candidate(capacity, mut parent) => {
            // Keep candidate ownership in the finisher BEFORE any fallible/native
            // callback boundary. Busy restores this SAME value, never requeues Root.
            fin.outcome = Some(Outcome::Candidate(capacity, parent.take()));
            {
                let state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
                let s = slot(&state).ok_or(Refused::Binding)?;
                if s.closed || s.cancelled >= s.generation {
                    return Ok(());
                }
            }
            let live = context.upgrade().ok_or(Refused::Binding)?;
            if let Some(Outcome::Candidate(capacity, parent)) = fin.outcome.as_mut() {
                if parent.is_none() {
                    *parent = Some(
                        live.ledger
                            .reserve_admitted_parent(
                                route.selected().clone(),
                                route.session().identity().clone(),
                                capacity.generation(),
                            )
                            .map_err(|_| Refused::Capacity)?,
                    );
                }
            }
            #[cfg(test)]
            {
                let cancel = {
                    let mut state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
                    let s = slot_mut(&mut state).expect("original Slot");
                    (std::mem::take(&mut s.cancel_before_attach), s.generation)
                };
                if cancel.0 {
                    route.cancel_generation(cancel.1).unwrap();
                }
            }
            let _commit = live
                .commit
                .clone()
                .try_lock_owned()
                .map_err(|_| Refused::Capacity)?;
            let _handle = current(&live, route)?;
            let authority = live.authority.try_lock().map_err(|_| Refused::Capacity)?;
            let _handle = exact(&authority, route)?;
            let mut state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
            let closed = state.closed || state.effects_closed || state.browser_setup.is_some();
            let s = slot_mut(&mut state).ok_or(Refused::Binding)?;
            let Some(Outcome::Candidate(capacity, parent)) = fin.outcome.as_ref() else {
                unreachable!()
            };
            let original = s.registration.as_ref().ok_or(Refused::Binding)?;
            if closed
                || s.closed
                || s.cancelled >= s.generation
                || capacity.generation() != s.generation
                || s.phase != Phase::Finalizing
                || s.capacity.is_some()
                || s.parent.is_some()
                || !s
                    .invocation
                    .as_ref()
                    .is_some_and(|i| Arc::ptr_eq(&i.id, &fin.id))
                || !capacity.belongs_to(original)
                || !capacity.is_current()
                || !parent.as_ref().is_some_and(|p| {
                    p.belongs_to(&live.ledger)
                        && p.matches(route.selected(), route.session().identity(), s.generation)
                })
            {
                return Err(Refused::Binding);
            }
            let Some(Outcome::Candidate(capacity, parent)) = fin.outcome.take() else {
                unreachable!()
            };
            s.capacity = Some(capacity);
            s.parent = parent;
            s.phase = Phase::Reserved;
            let invocation = s.invocation.take();
            drop(state);
            drop(authority);
            drop(_commit);
            drop(live);
            drop(invocation);
            fin.done = true;
        }
        Outcome::Unstarted(error, payment) => {
            let mut payment = payment;
            let old = {
                let mut state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
                let s = slot_mut(&mut state).ok_or(Refused::Binding)?;
                if s.payment.is_none() {
                    s.payment = payment.take();
                }
                if error != VoiceEndpointError::Protocol(VoiceError::Capacity)
                    && s.cancelled < s.generation
                {
                    s.close();
                }
                s.phase = Phase::Waiting;
                s.invocation.take()
            };
            drop(old);
            drop(payment);
            fin.done = true;
        }
        Outcome::Unknown => {
            let mut state = route.row.state.lock().unwrap_or_else(|e| e.into_inner());
            let s = slot_mut(&mut state).ok_or(Refused::Binding)?;
            s.close();
            s.uncertain = true;
            s.phase = Phase::Uncertain;
            // No replacement None is ever authorized by returned Work.
            fin.done = true;
        }
    }
    Ok(())
}

async fn run(context: Context, custody: Custody) {
    // This ACTUAL affine hold is borrowed through all positive finalization.
    if !custody
        .returned
        .native
        .as_ref()
        .is_some_and(|hold| hold.belongs_to(custody.route.session()))
    {
        return;
    }
    loop {
        let notified = custody.route.row.admission_changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if let Some(work) = custody.route.admitted_work() {
            if !work.is_retired() {
                tokio::select! { _=work.wait_retired()=>{},_=notified.as_mut()=>{} }
                continue;
            }
        }
        let step = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for _ in 0..2 {
                if let Err(error) = finish(&context, &custody.route) {
                    if error != Refused::Capacity {
                        let mut state = custody
                            .route
                            .row
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if let Some(s) = slot_mut(&mut state) {
                            if s.cancelled < s.generation {
                                s.close();
                            }
                        }
                    }
                    break;
                }
            }
            let context_dead = context.upgrade().is_none();
            let (whole, cancelled, empty) = {
                let mut state = custody
                    .route
                    .row
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let closed = state.closed
                    || state.effects_closed
                    || !custody.route.selected().lease().is_live()
                    || context_dead;
                let s = slot_mut(&mut state).expect("original Slot");
                if closed || s.registration.as_ref().is_some_and(|r| !r.is_current()) {
                    s.close();
                }
                (
                    s.closed,
                    s.cancelled >= s.generation,
                    s.registration.is_none()
                        && s.capacity.is_none()
                        && s.parent.is_none()
                        && s.invocation.is_none()
                        && s.payment.is_none(),
                )
            };
            if whole || cancelled {
                reclaim(&custody.route, whole);
            } else if let Err(error) = queue(&context, &custody.route) {
                if error != Refused::Capacity {
                    let mut state = custody
                        .route
                        .row
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if let Some(s) = slot_mut(&mut state) {
                        if s.cancelled < s.generation {
                            s.close();
                        }
                    }
                }
            }
            whole && empty
        }));
        match step {
            Ok(true) => return,
            Err(_) => {
                let mut state = custody
                    .route
                    .row
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if let Some(s) = slot_mut(&mut state) {
                    s.close();
                    s.uncertain = true;
                    s.phase = Phase::Uncertain;
                }
            }
            _ => {}
        }
        notified.await;
    }
}

fn rpc_layout_bytes() -> usize {
    catcoms_app::voice_admitted_admission_rpc_bytes()
        + std::mem::size_of::<Publisher>()
        + 2 * std::mem::size_of::<usize>()
        + 128
}
fn task_layout_bytes() -> usize {
    fn future<F: std::future::Future>(_: fn(Context, Custody) -> F) -> usize {
        std::mem::size_of::<F>()
    }
    future(run)
        + 2 * std::mem::size_of::<Slot>()
        + std::mem::size_of::<PaidSlot>()
        + std::mem::size_of::<Finalizing>()
        + std::mem::size_of::<Outcome>()
        + std::mem::size_of::<Reclamation>()
        + std::mem::size_of::<Custody>()
        + std::mem::size_of::<LiveContext>()
        + AdmittedParentClaim::preparation_layout_bytes()
        + std::mem::size_of::<Recovery>()
        + std::mem::size_of::<UnqueuedPayment>()
        + std::mem::size_of::<(usize, usize, Funding)>()
        + 1024
}
/// Synchronous31 frames coexist with the paid HTTP future. Actual Slot/task and
/// Actor argument graphs have their own independently checked contributions.
pub(in crate::voice_handoff) fn control_layout_bytes() -> usize {
    std::mem::size_of::<LiveContext>()
        + std::mem::size_of::<Handle>()
        + std::mem::size_of::<Snapshot>()
        + std::mem::size_of::<Request>()
        + std::mem::size_of::<crate::voice_handoff::channel::PreparedReply>()
        + std::mem::size_of::<crate::voice_handoff::channel::CompletedReply>()
        + crate::voice_handoff::requests::admitted_admission::REPLY_BYTES
        + 512
}
#[cfg(test)]
pub(super) fn register_observing_task(
    context: &Context,
    route: &AdmittedRoute,
    g: u64,
) -> Result<Option<tokio::task::AbortHandle>, Refused> {
    let mut task = None;
    register_with(context, route, g, |actual| task = Some(actual))?;
    Ok(task)
}
#[cfg(test)]
impl AdmittedRoute {
    pub(super) fn admitted_drained_for_test(&self) -> bool {
        self.row
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .admitted_returned()
    }
    pub(super) fn admitted_task_returned_for_test(&self) -> bool {
        let state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        slot(&state).is_some_and(|s| s.task_returned)
    }
    pub(super) fn admitted_graphs_for_test() -> (usize, usize) {
        (task_layout_bytes(), rpc_layout_bytes())
    }
    pub(super) fn unwind_admitted_reclamation_for_test(&self) {
        let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        slot_mut(&mut state)
            .expect("actual original Slot")
            .unwind_reclamation = true;
    }
    pub(super) fn cancel_admitted_race_for_test(&self, after_accept: bool) {
        let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        let s = slot_mut(&mut state).expect("actual original Slot");
        if after_accept {
            s.cancel_after_accept = true;
        } else {
            s.cancel_before_attach = true;
        }
    }
}
