//! Original journal-owned NETWORK/DATA exchange. This never enters the Welcome SDK, creates
//! Fresh/Current, exposes a master, or returns the old unjournalled initializer.
use super::*;
use catcoms_rt::{ConnectionLease, RequestAuthority};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Weak;
use tokio::sync::{watch, Mutex, Notify};

const BODY_LIMIT: usize = 2 * 1024 * 1024;
const INPUT_TAG: &str = "catcoms/original-join-received-input/v1";
const INPUT_PLAIN: usize = BODY_LIMIT + 1024;

/// Durable invocation DATA, not a ticket or proof of a file event. The actual App issuer must
/// pair it with the genuine saved outcome before network authority can be installed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct OriginalJoinArmData {
    pub grant: u64,
    pub ordinal: u64,
    pub revision: u64,
    pub activation: [u8; 32],
    pub invocation: [u8; 32],
    pub deadline_ms: u64,
}
impl fmt::Debug for OriginalJoinArmData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OriginalJoinArmData")
            .finish_non_exhaustive()
    }
}

/// Trusted host boundary, comparable to managed scope reservations. Arbitrary Rust impls are
/// TRUSTED CODE, not authenticated disk evidence. Actual App impls retain original mounted
/// source/fees and weak task observations, but NO Store, vault session/key or sender.
/// Every method must be bounded/pure and must not reacquire Store/pool/cell locks or perform
/// I/O: these checks also run inside the private cell or an actual admission/catalog writer.
pub trait OriginalJoinNetworkContext: Send + Sync + fmt::Debug {
    fn identity(&self) -> Arc<()>;
    fn binding(&self) -> [u8; 32];
    fn request(&self) -> DirectJoinRequestIdentity;
    fn spent_ordinal_floor(&self) -> u64;
    fn cancellation(&self) -> RequestCancellation;
    fn check_original(&self) -> Result<(), SyncError>;
}
/// Actual saved-arm DATA witness, separate from lower request capacity. It must survive a
/// consumed ticket's retirement without retaining its lower-work fee or revoking task owner.
/// Getters/checks are bounded/pure, with no Store/pool/cell lock or I/O (including DATA repair).
pub trait OriginalJoinSavedArm: Send + Sync + fmt::Debug {
    fn context_identity(&self) -> Arc<()>;
    fn binding(&self) -> [u8; 32];
    fn request(&self) -> DirectJoinRequestIdentity;
    fn data(&self) -> OriginalJoinArmData;
    fn check_original_data(&self) -> Result<(), SyncError>;
}
/// Non-Clone production object created only after actual arm/file/root barriers. Sync OWNS
/// the once-only transition. Getters must be bounded/pure; cancellation preserves original
/// source/arm authority and prepaid lower-completion keepalive. There is no default impl.
pub trait OriginalJoinNetworkSubmission: Send + Sync + fmt::Debug {
    fn saved(&self) -> Arc<dyn OriginalJoinSavedArm>;
    fn target(&self) -> PeerId;
    fn cancellation(&self) -> RequestCancellation;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginalJoinNetworkStatus {
    Idle,
    Armed,
    Submitting,
    Spent,
    InputHeld,
    Closed,
}

struct NegativeState {
    closed: AtomicBool,
    wake: Arc<Notify>,
}
struct NegativeOwner {
    state: Arc<NegativeState>,
    signal: watch::Sender<bool>,
}
impl NegativeOwner {
    fn new() -> Self {
        Self {
            state: Arc::new(NegativeState {
                closed: AtomicBool::new(false),
                wake: Arc::new(Notify::new()),
            }),
            signal: watch::channel(false).0,
        }
    }
    fn close(&self) {
        self.state.closed.store(true, Ordering::Release);
        self.state.wake.notify_waiters();
    }
    fn observer(&self) -> RequestCancellation {
        RequestCancellation::new(self.signal.subscribe(), None).with_authority(Arc::new(
            NegativeAuthority {
                state: Arc::downgrade(&self.state),
                wake: self.state.wake.clone(),
            },
        ))
    }
}
impl Drop for NegativeOwner {
    fn drop(&mut self) {
        self.close();
    }
}
struct NegativeAuthority {
    state: Weak<NegativeState>,
    wake: Arc<Notify>,
}
impl fmt::Debug for NegativeAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OriginalJoinNegative")
            .finish_non_exhaustive()
    }
}
#[async_trait::async_trait]
impl RequestAuthority for NegativeAuthority {
    fn authorized(&self) -> bool {
        self.state
            .upgrade()
            .is_some_and(|s| !s.closed.load(Ordering::Acquire))
    }
    async fn revoked(&self) {
        loop {
            let changed = self.wake.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self.authorized() {
                return;
            }
            changed.await;
        }
    }
}

struct ArmSlot {
    _ticket: Box<dyn OriginalJoinNetworkSubmission>,
    cancel: RequestCancellation,
    outcome: Arc<dyn OriginalJoinSavedArm>,
}
struct HeldInput {
    body: Zeroizing<Vec<u8>>,
    outcome: Arc<dyn OriginalJoinSavedArm>,
    sealed: Option<([u8; 32], Arc<SealedOriginalJoinInput>)>,
}
enum Phase {
    Idle,
    Armed(ArmSlot),
    Submitting {
        slot: ArmSlot,
        flight: RequestCancellation,
    },
    Spent(Option<Arc<dyn OriginalJoinSavedArm>>),
    Held(HeldInput),
}
struct State {
    source: Option<PreparedDirectJoin>,
    phase: Phase,
    ordinal: u64,
    target: Option<PeerId>,
}
impl State {
    fn reconcile(&mut self, closed: bool) -> Option<ArmSlot> {
        let retired = match &self.phase {
            Phase::Armed(slot) => closed || slot.cancel.is_cancelled(),
            Phase::Submitting { slot, flight } => {
                closed || slot.cancel.is_cancelled() || flight.is_cancelled()
            }
            _ => false,
        };
        if !retired {
            return None;
        }
        match std::mem::replace(&mut self.phase, Phase::Spent(None)) {
            Phase::Submitting { slot, .. } | Phase::Armed(slot) => {
                self.phase = Phase::Spent(Some(slot.outcome.clone()));
                Some(slot)
            }
            _ => unreachable!("checked exact private phase"),
        }
    }
    fn status(&self) -> OriginalJoinNetworkStatus {
        match &self.phase {
            Phase::Idle => OriginalJoinNetworkStatus::Idle,
            Phase::Armed(_) => OriginalJoinNetworkStatus::Armed,
            Phase::Submitting { .. } => OriginalJoinNetworkStatus::Submitting,
            Phase::Spent(outcome) => {
                let _ = outcome;
                OriginalJoinNetworkStatus::Spent
            }
            Phase::Held(_) => OriginalJoinNetworkStatus::InputHeld,
        }
    }
}

/// Store registers this actual paid cell before any request, independently of caller futures.
/// It owns immutable source DATA and context fees, never T, a vault session, or the task sender.
pub struct RetainedOriginalJoinNetwork {
    state: Mutex<State>,
    local: NegativeOwner,
    bound: AtomicBool,
    context: Arc<dyn OriginalJoinNetworkContext>,
}
impl fmt::Debug for RetainedOriginalJoinNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetainedOriginalJoinNetwork")
            .finish_non_exhaustive()
    }
}
pub struct BoundOriginalJoinNetwork<T> {
    transport: T,
    target: PeerId,
    prefix: PreOwnerConnectionHandoff,
    cell: Arc<RetainedOriginalJoinNetwork>,
    identity: Arc<()>,
}
impl<T> fmt::Debug for BoundOriginalJoinNetwork<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundOriginalJoinNetwork")
            .finish_non_exhaustive()
    }
}
impl<T> Drop for BoundOriginalJoinNetwork<T> {
    fn drop(&mut self) {
        self.cell.close();
    }
}

pub struct ReturnedOriginalJoinSubmission {
    ticket: Box<dyn OriginalJoinNetworkSubmission>,
    error: SyncError,
}
impl fmt::Debug for ReturnedOriginalJoinSubmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReturnedOriginalJoinSubmission")
            .finish_non_exhaustive()
    }
}
impl ReturnedOriginalJoinSubmission {
    pub fn into_parts(self) -> (Box<dyn OriginalJoinNetworkSubmission>, SyncError) {
        (self.ticket, self.error)
    }
}
pub struct ReturnedOriginalJoinBinding<T> {
    transport: T,
    cell: Arc<RetainedOriginalJoinNetwork>,
    error: SyncError,
}
impl<T> fmt::Debug for ReturnedOriginalJoinBinding<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReturnedOriginalJoinBinding")
            .finish_non_exhaustive()
    }
}
impl<T> ReturnedOriginalJoinBinding<T> {
    pub fn into_parts(self) -> (T, Arc<RetainedOriginalJoinNetwork>, SyncError) {
        (self.transport, self.cell, self.error)
    }
}

impl RetainedOriginalJoinNetwork {
    fn new(
        source: PreparedDirectJoin,
        context: Arc<dyn OriginalJoinNetworkContext>,
    ) -> Result<Arc<Self>, SyncError> {
        context.check_original()?;
        if source.completed || source.request_identity()? != context.request() {
            return Err(SyncError::JoinRejected);
        }
        let ordinal = context.spent_ordinal_floor();
        Ok(Arc::new(Self {
            state: Mutex::new(State {
                source: Some(source),
                phase: Phase::Idle,
                ordinal,
                target: None,
            }),
            local: NegativeOwner::new(),
            bound: AtomicBool::new(false),
            context,
        }))
    }
    pub fn from_original_local(
        source: PreparedDirectJoin,
        context: Arc<dyn OriginalJoinNetworkContext>,
    ) -> Result<Arc<Self>, SyncError> {
        if source.request_started
            || source.recovered
            || source
                .sealed
                .as_ref()
                .and_then(|s| s.original_local_binding())
                != Some(context.binding())
        {
            return Err(SyncError::JoinRejected);
        }
        Self::new(source, context)
    }
    pub fn from_authenticated_source(
        source: RecoveredDirectJoinSource,
        context: Arc<dyn OriginalJoinNetworkContext>,
    ) -> Result<Arc<Self>, SyncError> {
        if source.source_binding() != &context.binding() {
            return Err(SyncError::JoinRejected);
        }
        Self::new(source.into_original_network_data(), context)
    }
    pub fn close(&self) {
        self.local.close();
    }
    pub fn try_status(&self) -> Option<OriginalJoinNetworkStatus> {
        let mut state = self.state.try_lock().ok()?;
        let retired = state.reconcile(
            self.local.observer().is_cancelled() || self.context.cancellation().is_cancelled(),
        );
        let status = if matches!(state.phase, Phase::Held(_)) {
            OriginalJoinNetworkStatus::InputHeld
        } else if self.local.observer().is_cancelled() || self.context.cancellation().is_cancelled()
        {
            OriginalJoinNetworkStatus::Closed
        } else {
            state.status()
        };
        drop(state);
        drop(retired);
        Some(status)
    }
    pub fn install_original_submission(
        &self,
        ticket: Box<dyn OriginalJoinNetworkSubmission>,
    ) -> Result<(), ReturnedOriginalJoinSubmission> {
        let outcome = ticket.saved();
        let cancel = ticket.cancellation();
        let data = outcome.data();
        let checked = self
            .context
            .check_original()
            .and_then(|_| outcome.check_original_data())
            .and_then(|_| {
                if self.local.observer().is_cancelled()
                    || cancel.is_cancelled()
                    || cancel.keepalive().is_none()
                    || !Arc::ptr_eq(&self.context.identity(), &outcome.context_identity())
                    || outcome.binding() != self.context.binding()
                    || outcome.request() != self.context.request()
                    || data.grant == 0
                    || data.ordinal == 0
                    || data.revision == 0
                    || data.deadline_ms == 0
                {
                    Err(SyncError::JoinRejected)
                } else {
                    Ok(())
                }
            });
        if let Err(error) = checked {
            return Err(ReturnedOriginalJoinSubmission { ticket, error });
        }
        let Ok(mut state) = self.state.try_lock() else {
            return Err(ReturnedOriginalJoinSubmission {
                ticket,
                error: SyncError::JoinRejected,
            });
        };
        let retired = state.reconcile(
            self.local.observer().is_cancelled() || self.context.cancellation().is_cancelled(),
        );
        if !matches!(state.phase, Phase::Idle | Phase::Spent(_))
            || self.local.observer().is_cancelled()
            || state.target != Some(ticket.target())
            || data.ordinal <= state.ordinal
            || self.context.check_original().is_err()
            || cancel.is_cancelled()
        {
            drop(state);
            drop(retired);
            return Err(ReturnedOriginalJoinSubmission {
                ticket,
                error: SyncError::JoinRejected,
            });
        }
        state.ordinal = data.ordinal;
        state.phase = Phase::Armed(ArmSlot {
            _ticket: ticket,
            cancel,
            outcome,
        });
        drop(state);
        drop(retired);
        Ok(())
    }
    pub fn bind_original_transport<T: MeshTransport>(
        self: Arc<Self>,
        transport: T,
        target: PeerId,
    ) -> Result<BoundOriginalJoinNetwork<T>, ReturnedOriginalJoinBinding<T>> {
        let checked = transport.supports_connection_leases()
            && transport.supports_connected_cancellable_requests()
            && !self.local.observer().is_cancelled()
            && self.context.check_original().is_ok();
        if !checked {
            return Err(ReturnedOriginalJoinBinding {
                transport,
                cell: self,
                error: SyncError::JoinRejected,
            });
        }
        let Ok(mut state) = self.state.try_lock() else {
            return Err(ReturnedOriginalJoinBinding {
                transport,
                cell: self.clone(),
                error: SyncError::JoinRejected,
            });
        };
        if self
            .bound
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            drop(state);
            return Err(ReturnedOriginalJoinBinding {
                transport,
                cell: self,
                error: SyncError::JoinRejected,
            });
        }
        state.target = Some(target);
        drop(state);
        Ok(BoundOriginalJoinNetwork {
            transport,
            target,
            prefix: PreOwnerConnectionHandoff::default(),
            cell: self,
            identity: Arc::new(()),
        })
    }
}

/// Non-Clone continuation DATA bound to ONE actual physical Bound owner. It owns the moved
/// secret source only in its new cell, never a replacement T/prefix or a scope/publication proof.
/// The new bind claim is already reserved true; even its retained DATA Arc cannot bind another T.
pub struct PreparedOriginalJoinContinuation {
    original_bound: Arc<()>,
    original_cell: Arc<RetainedOriginalJoinNetwork>,
    next: Arc<RetainedOriginalJoinNetwork>,
}
impl fmt::Debug for PreparedOriginalJoinContinuation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedOriginalJoinContinuation")
            .finish_non_exhaustive()
    }
}
impl PreparedOriginalJoinContinuation {
    /// Retained DATA/custody only. Its permanently reserved bind claim prevents replacement T.
    pub fn retained_cell_data(&self) -> Arc<RetainedOriginalJoinNetwork> {
        self.next.clone()
    }
}
pub struct ReturnedOriginalJoinContinuation {
    original: PreparedOriginalJoinContinuation,
    error: SyncError,
}
impl fmt::Debug for ReturnedOriginalJoinContinuation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReturnedOriginalJoinContinuation")
            .finish_non_exhaustive()
    }
}
impl ReturnedOriginalJoinContinuation {
    pub fn into_parts(self) -> (PreparedOriginalJoinContinuation, SyncError) {
        (self.original, self.error)
    }
}
impl<T: MeshTransport> BoundOriginalJoinNetwork<T> {
    /// Trusted-host preparation only. Actual App must reserve THIS original new owner with its
    /// durable RetryGrant candidate, then prove its real saved outcome before attachment. No
    /// network/SDK runs here. All checks precede the one source move; failures leave it intact.
    pub fn prepare_original_continuation(
        &mut self,
        context: Arc<dyn OriginalJoinNetworkContext>,
    ) -> Result<PreparedOriginalJoinContinuation, SyncError> {
        if !(self.cell.local.observer().is_cancelled()
            || self.cell.context.cancellation().is_cancelled())
            || context.check_original().is_err()
            || Arc::ptr_eq(&context.identity(), &self.cell.context.identity())
            || context.binding() != self.cell.context.binding()
            || context.request() != self.cell.context.request()
        {
            return Err(SyncError::JoinRejected);
        }
        let floor = context.spent_ordinal_floor();
        // All fresh cell/wakeup allocations precede moving the only private source. The new
        // cell is not exposed, so its mutex has no competing owner or external lock order.
        let next = Arc::new(RetainedOriginalJoinNetwork {
            state: Mutex::new(State {
                source: None,
                phase: Phase::Idle,
                ordinal: floor,
                target: Some(self.target),
            }),
            local: NegativeOwner::new(),
            bound: AtomicBool::new(true),
            context,
        });
        let mut state = self
            .cell
            .state
            .try_lock()
            .map_err(|_| SyncError::JoinRejected)?;
        let retired = state.reconcile(true);
        let result = (|| {
            if matches!(state.phase, Phase::Held(_))
                || state.target != Some(self.target)
                || floor < state.ordinal
                || state.source.as_ref().is_none_or(|source| source.completed)
                || next.context.check_original().is_err()
            {
                return Err(SyncError::JoinRejected);
            }
            let mut next_state = next.state.try_lock().map_err(|_| SyncError::JoinRejected)?;
            let source = state.source.take().ok_or(SyncError::JoinRejected)?;
            // No fallible operation/authority check after source.take. Old cell remains closed
            // and bind=true forever; new cell is reserved to this SAME actual Bound identity.
            next_state.source = Some(source);
            drop(next_state);
            Ok(())
        })();
        drop(state);
        drop(retired); // Last upper lower-capacity owner never drops under the cell guard.
                       // Even a trusted context owning its last metadata fee retires outside the OLD cell
                       // guard on an error. Keep next outside the closure rather than dropping a moved
                       // captured Arc while that writer is held.
        result?;
        Ok(PreparedOriginalJoinContinuation {
            original_bound: self.identity.clone(),
            original_cell: self.cell.clone(),
            next,
        })
    }

    /// Attach only the opaque continuation prepared by THIS physical Bound. The trusted App
    /// caller must already hold its exact Active registration/SavedData outcome. On refusal
    /// return the SAME source owner so a transient error cannot strand its registered identity.
    pub fn attach_original_continuation(
        &mut self,
        original: PreparedOriginalJoinContinuation,
    ) -> Result<(), ReturnedOriginalJoinContinuation> {
        if !Arc::ptr_eq(&self.identity, &original.original_bound)
            || !Arc::ptr_eq(&self.cell, &original.original_cell)
            || !(self.cell.local.observer().is_cancelled()
                || self.cell.context.cancellation().is_cancelled())
            || !original.next.bound.load(Ordering::Acquire)
            || original.next.context.check_original().is_err()
        {
            return Err(ReturnedOriginalJoinContinuation {
                original,
                error: SyncError::JoinRejected,
            });
        }
        let old = std::mem::replace(&mut self.cell, original.next.clone());
        drop(original);
        drop(old); // T/target/ordered lifecycle prefix were never replaced.
        Ok(())
    }
}

#[derive(Debug)]
struct FlightAuthority {
    source: RequestCancellation,
    arm: RequestCancellation,
    local: RequestCancellation,
    flight: RequestCancellation,
    connection: ConnectionLease,
}
#[async_trait::async_trait]
impl RequestAuthority for FlightAuthority {
    fn authorized(&self) -> bool {
        !self.source.is_cancelled()
            && !self.arm.is_cancelled()
            && !self.local.is_cancelled()
            && !self.flight.is_cancelled()
            && self.connection.is_live()
    }
    async fn revoked(&self) {
        let mut source = self.source.clone();
        let mut arm = self.arm.clone();
        let mut local = self.local.clone();
        let mut flight = self.flight.clone();
        if self.authorized() {
            tokio::select! {_=source.cancelled()=>{},_=arm.cancelled()=>{},_=local.cancelled()=>{},_=flight.cancelled()=>{},_=self.connection.revoked()=>{}}
        }
    }
}
impl<T: MeshTransport> BoundOriginalJoinNetwork<T> {
    pub fn can_submit_original(&self) -> bool {
        self.transport.supports_connection_leases()
            && self.transport.supports_connected_cancellable_requests()
            && self
                .transport
                .connection_lease(self.target)
                .is_some_and(|l| l.is_live())
            && !self.cell.local.observer().is_cancelled()
            && self.cell.context.check_original().is_ok()
    }
    pub async fn exchange_armed(&mut self) -> Result<OriginalJoinNetworkStatus, SyncError> {
        let mut state = self.cell.state.lock().await;
        let retired = state.reconcile(
            self.cell.local.observer().is_cancelled()
                || self.cell.context.cancellation().is_cancelled(),
        );
        // Reconciliation normally happens in pre-reservation status. A cancelled prior flight
        // owns no current exchange; release its upper fee outside this mutex and retry later.
        if retired.is_some() {
            drop(state);
            drop(retired);
            return Ok(OriginalJoinNetworkStatus::Spent);
        }
        if matches!(state.phase, Phase::Held(_)) {
            return Ok(OriginalJoinNetworkStatus::InputHeld);
        }
        let Phase::Armed(slot) = &state.phase else {
            return Err(SyncError::JoinRejected);
        };
        if !self.can_submit_original() || slot.cancel.is_cancelled() {
            return Err(SyncError::JoinRejected);
        }
        let connection = self
            .transport
            .connection_lease(self.target)
            .filter(ConnectionLease::is_live)
            .ok_or(SyncError::JoinRejected)?;
        let flight = NegativeOwner::new();
        let authority = Arc::new(FlightAuthority {
            source: self.cell.context.cancellation(),
            arm: slot.cancel.clone(),
            local: self.cell.local.observer(),
            flight: flight.observer(),
            connection,
        });
        if !authority.authorized() {
            return Err(SyncError::JoinRejected);
        }
        let mut cancel = slot.cancel.clone().with_authority(authority);
        let Phase::Armed(slot) = std::mem::replace(&mut state.phase, Phase::Spent(None)) else {
            unreachable!("private armed phase");
        };
        let outcome = slot.outcome.clone();
        state.phase = Phase::Submitting {
            slot,
            flight: flight.observer(),
        };
        let source = state.source.as_mut().ok_or(SyncError::JoinRejected)?;
        source.request_started = true;
        let response = tokio::select! {biased;_=cancel.cancelled()=>Err(TransportError::Cancelled),result=self.transport.request_connected_cancellable(self.target,ProtocolId(RR_PROTOCOL),state.source.as_ref().ok_or(SyncError::JoinRejected)?.request.clone(),cancel.clone())=>result};
        let body = match response {
            Ok(response) => {
                tokio::select! {biased;_=cancel.cancelled()=>Err(SyncError::Transport(TransportError::Cancelled)),result=state.source.as_ref().ok_or(SyncError::JoinRejected)?.receive_original_ready_body_limit(&self.transport,&mut self.prefix,self.target,response,BODY_LIMIT+2)=>result}
            }
            Err(error) => Err(error.into()),
        };
        let body = body.and_then(|body| {
            if body.len() > BODY_LIMIT {
                return Err(SyncError::Malformed);
            }
            state
                .source
                .as_ref()
                .ok_or(SyncError::JoinRejected)?
                .verify_original_ready_body(&body)?;
            Ok(body)
        });
        let phase = match body {
            Ok(body) => Phase::Held(HeldInput {
                body: Zeroizing::new(body),
                outcome: outcome.clone(),
                sealed: None,
            }),
            Err(_) => Phase::Spent(Some(outcome)),
        };
        let status = if matches!(phase, Phase::Held(_)) {
            OriginalJoinNetworkStatus::InputHeld
        } else {
            OriginalJoinNetworkStatus::Spent
        };
        let retired = std::mem::replace(&mut state.phase, phase);
        drop(state);
        drop(retired);
        drop(cancel);
        drop(flight);
        // Network/signed-input failures remain observable as Spent; the actual owner chooses
        // another explicit invocation from its preserved allowance, never resamples a package.
        Ok(status)
    }
}

/// Opaque encrypted received DATA. It does not attest selected-KP SDK acceptance. Keeping this
/// object keeps actual context accounting, never the task sender, a vault key or transport.
pub struct SealedOriginalJoinInput {
    blob: SealedBlob,
    arm: OriginalJoinArmData,
    body_len: u32,
    body_digest: [u8; 32],
    _custody: Arc<dyn OriginalJoinNetworkContext>,
}
impl fmt::Debug for SealedOriginalJoinInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SealedOriginalJoinInputData")
            .finish_non_exhaustive()
    }
}
impl SealedOriginalJoinInput {
    /// Inert equality metadata from the actual original context; no private source bytes or
    /// accepted membership follow from these labels.
    pub fn source_binding(&self) -> [u8; 32] {
        self._custody.binding()
    }
    pub fn request_identity(&self) -> DirectJoinRequestIdentity {
        self._custody.request()
    }

    pub fn encrypted_parts(&self) -> (&[u8; 24], &[u8]) {
        (&self.blob.nonce, &self.blob.ciphertext)
    }
    pub fn arm_data(&self) -> OriginalJoinArmData {
        self.arm
    }
    pub fn body_length(&self) -> u32 {
        self.body_len
    }
    pub fn body_digest(&self) -> [u8; 32] {
        self.body_digest
    }
}
impl RetainedOriginalJoinNetwork {
    /// Caller prepays body/plain/cipher/cache overlap before calling. Retry returns the SAME
    /// cached ciphertext; a changed key refuses. Negative closure never discards held input.
    pub fn seal_received_input(
        &self,
        key: &[u8; 32],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Arc<SealedOriginalJoinInput>, SyncError> {
        let mut state = self.state.try_lock().map_err(|_| SyncError::JoinRejected)?;
        let Phase::Held(held) = &mut state.phase else {
            return Err(SyncError::JoinRejected);
        };
        held.outcome.check_original_data()?;
        let key_id = *blake3::keyed_hash(key, b"catcoms/original-join-input-key/v1\0").as_bytes();
        if let Some((original, sealed)) = &held.sealed {
            return if *original == key_id {
                Ok(sealed.clone())
            } else {
                Err(SyncError::JoinRejected)
            };
        }
        let arm = held.outcome.data();
        let digest = *blake3::hash(&held.body).as_bytes();
        let length = held
            .body
            .len()
            .checked_add(512)
            .filter(|n| *n <= INPUT_PLAIN)
            .ok_or(SyncError::Malformed)?;
        let mut output = Encoder::with_capacity(length);
        output
            .put_str(INPUT_TAG)
            .map_err(|_| SyncError::Malformed)?;
        output.put_u8(1);
        output.put_u8(1);
        output
            .put_bytes(&self.context.binding())
            .map_err(|_| SyncError::Malformed)?;
        output
            .put_bytes(self.context.request().request_digest())
            .map_err(|_| SyncError::Malformed)?;
        output
            .put_bytes(self.context.request().package_reference())
            .map_err(|_| SyncError::Malformed)?;
        put_arm(&mut output, arm)?;
        output
            .put_bytes(&digest)
            .map_err(|_| SyncError::Malformed)?;
        output
            .put_bytes(&held.body)
            .map_err(|_| SyncError::Malformed)?;
        let plain = Zeroizing::new(output.finish());
        if plain.len() > INPUT_PLAIN {
            return Err(SyncError::Malformed);
        }
        let blob = seal(key, &plain, rng).map_err(|_| SyncError::Malformed)?;
        let sealed = Arc::new(SealedOriginalJoinInput {
            blob,
            arm,
            body_len: held.body.len() as u32,
            body_digest: digest,
            _custody: self.context.clone(),
        });
        held.sealed = Some((key_id, sealed.clone()));
        Ok(sealed)
    }
}
fn put_arm(output: &mut Encoder, arm: OriginalJoinArmData) -> Result<(), SyncError> {
    output.put_u64(arm.grant);
    output.put_u64(arm.ordinal);
    output.put_u64(arm.revision);
    output
        .put_bytes(&arm.activation)
        .map_err(|_| SyncError::Malformed)?;
    output
        .put_bytes(&arm.invocation)
        .map_err(|_| SyncError::Malformed)?;
    output.put_u64(arm.deadline_ms);
    Ok(())
}

#[cfg(test)]
mod tests;
