//! Canonical native custody for publishing the original browser-held admitted key.
//! SDK generation remains outside every native UI/Coordinator guard. A publication
//! task has its own payment; the still-active call runner and Generation metadata
//! are not silently charged for this additional execution graph.

use crate::voice_authority::{AdmittedPublicationWork, JoinKeyPublication};
use crate::voice_call_activation::Handle as ActivationHandle;
use catcoms_app::{
    VoiceAdmittedPresencePublisher, VoiceAdmittedPresenceResult, VoiceAdmittedPresenceWork,
    VoiceEndpointError,
};
use catcoms_rt::Clock;
use catcoms_sync::voice_v2::call_activation::ActivationRpcPermit;
use catcoms_sync::voice_v2::resources::Credit;
use catcoms_sync::voice_v2::VoiceError;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};

// The first compiled graph was 9,564 bytes, beyond the provisional 8 KiB.
// The independent 12 KiB payment also retains exact terminal history until ACK;
// group/process ceilings stay unchanged and the full guard remains mandatory.
const PUBLICATION_BYTES: usize = 12 * 1024;

/// Public history for one original publication, never present-time key/RTC
/// authority. ClosedUnknown must not be relabelled as a known Root rejection.
#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum PublicationReply {
    Accepted {
        #[serde(flatten)]
        publication: JoinKeyPublication,
    },
    Rejected {
        #[serde(flatten)]
        identity: PublicationIdentity,
        code: RejectionCode,
    },
    Closed {
        #[serde(flatten)]
        identity: PublicationIdentity,
        code: ClosedCode,
    },
    NotStarted {
        #[serde(flatten)]
        identity: PublicationIdentity,
        code: ClosedCode,
    },
}
#[derive(Clone, serde::Serialize)]
pub(crate) struct PublicationIdentity {
    owner: String,
    executor: String,
    permit: String,
    join: String,
}
#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RejectionCode {
    Capacity,
    Refused,
    Stopped,
}
#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ClosedCode {
    Ended,
    Unconfirmed,
}
#[derive(Clone, Copy)]
enum Disposition {
    Published,
    Rejected(RejectionCode),
    Closed(ClosedCode),
}

struct Funding {
    _credit: Credit,
}

/// Prepared before Coordinator marks a publication in flight. No Root request,
/// key regeneration or positive permission follows merely from this reservation.
pub(crate) struct PrepaidPublication {
    rpc: ActivationRpcPermit,
    funding: Arc<Funding>,
}
impl PrepaidPublication {
    pub(crate) fn reserve(activation: &ActivationHandle) -> Result<Self, VoiceError> {
        // Refuse before ANY publication allocation or native in-flight mutation.
        // Keeping the formula here (also exercised in tests) makes future growth
        // a visible capacity refusal instead of an unaccounted retained task.
        if publication_layout_bytes() > PUBLICATION_BYTES {
            return Err(VoiceError::Capacity);
        }
        let credit = activation.original().reserve_retained(PUBLICATION_BYTES)?;
        let rpc = activation.original().reserve_rpc()?;
        Ok(Self {
            rpc,
            funding: Arc::new(Funding { _credit: credit }),
        })
    }
}

fn activation(state: &crate::AppState, owner: &str) -> Result<ActivationHandle, String> {
    state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .admitted_activation(owner)
        .ok_or_else(|| "voice activation is unavailable".into())
}

pub(super) async fn admit_inner(
    state: &crate::AppState,
    owner: &str,
    executor: [u8; 32],
) -> Result<crate::voice_authority::JoinKeyAdmission, String> {
    // A closed old physical owner may become collectible after its last real
    // header returns. Reap BEFORE fresh Crypto admission or UI/Coordinator locks.
    state.voice_channels.collect_retired(&state.voice_handoff);
    crate::voice_admitted_call::collect_retired(state);
    let original = activation(state, owner)?;
    crate::check_voice_binding(state, original.binding()).await?;
    let _commit =
        crate::require_ui_session_generation(state, original.binding().generation).await?;
    if !crate::voice_binding_current(state, original.binding()).await {
        return Err("voice server changed".into());
    }
    let mut authority = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    crate::check_voice_activation_generation(state, original.binding().generation)?;
    authority.admit_admitted_join_key(&original, executor)
}

pub(super) async fn publish_inner(
    state: &crate::AppState,
    owner: &str,
    executor: [u8; 32],
    permit: &str,
    point: [u8; 65],
) -> Result<JoinKeyPublication, String> {
    // Validate browser-supplied public metadata BEFORE freezing native state or
    // taking UI/Coordinator. Curve parsing is not a permission or SDK claim.
    p256::PublicKey::from_sec1_bytes(&point).map_err(|_| "invalid voice public key")?;
    let original = activation(state, owner)?;
    let pending = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .admitted_publication_task(&original, executor, permit, point);
    if let Some(pending) = pending {
        return pending.reply(state).await;
    }
    crate::check_voice_binding(state, original.binding()).await?;
    // Recover already completed public history before acquiring fresh capacity.
    // These fixed bounded reply buffers are not a new SDK/task/Root operation.
    let completed = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .completed_admitted_join_key_publication(&original, executor, permit, point)?;
    if let Some(reply) = completed {
        return Ok(reply);
    }
    let prepaid = PrepaidPublication::reserve(&original).map_err(|e| e.to_string())?;
    let clock = crate::voice_runtime(state).clock();
    let commit = state.ui_session_commit.clone().lock_owned().await;
    state.ui_session_gate().require_unlocked().await?;
    crate::check_voice_activation_generation(state, original.binding().generation)?;
    if !crate::voice_binding_current(state, original.binding()).await {
        return Err("voice server changed".into());
    }
    // Serialized tail: another invoke may already own the exact canonical Work.
    let pending = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .admitted_publication_task(&original, executor, permit, point);
    if let Some(pending) = pending {
        drop(commit);
        drop(prepaid);
        return pending.reply(state).await;
    }
    let plan = {
        let mut authority = state
            .voice_authority
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::check_voice_activation_generation(state, original.binding().generation)?;
        authority.prepare_admitted_join_key_publication(&original, executor, permit, point)?
    };
    match plan {
        crate::voice_authority::AdmittedPublicationPlan::Accepted(reply) => Ok(reply),
        crate::voice_authority::AdmittedPublicationPlan::Start(work) => {
            // No await/caller callback occurs between native freeze and SAME
            // task installation. End/settle retain publishing rows, so their
            // concurrent negative work cannot replace this original tuple.
            let pending = PendingPublication::new(original, work, prepaid, clock);
            let task = pending.initial_task(state, commit); // cancellation RAII before install/spawn
            if let Err(error) = pending.install(&state.voice_authority) {
                drop(task); // close before releasing UI; never submit an unindexed positive task
                return Err(error);
            }
            drop(tokio::spawn(task));
            pending.reply(state).await
        }
    }
}

/// Recover typed terminal facts before any fresh positive attempt. Exceptions
/// remain uncertainty/pre-admission refusal; callers must never infer a native
/// result from an error String. Exact never-started closure is a separate proof.
pub(super) async fn publish_public_inner(
    state: &crate::AppState,
    owner: &str,
    executor: [u8; 32],
    permit: &str,
    point: [u8; 65],
) -> Result<PublicationReply, String> {
    p256::PublicKey::from_sec1_bytes(&point).map_err(|_| "invalid voice public key")?;
    let original = activation(state, owner)?;
    if let Some(reply) = public_history(state, &original, executor, permit, point).await {
        return Ok(reply);
    }
    let result = publish_inner(state, owner, executor, permit, point).await;
    if let Some(reply) = public_history(state, &original, executor, permit, point).await {
        return Ok(reply);
    }
    result.map(|publication| PublicationReply::Accepted { publication })
}

async fn public_history(
    state: &crate::AppState,
    original: &ActivationHandle,
    executor: [u8; 32],
    permit: &str,
    point: [u8; 65],
) -> Option<PublicationReply> {
    let (pending, not_started, notify_closed) = {
        let authority = state
            .voice_authority
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let pending = authority.admitted_publication_task(original, executor, permit, point);
        // This closes only the original whose actual UI generation is invalid,
        // before certifying no future positive admission. A missing/foreign row
        // still supplies no certificate. Notifications stay outside Coordinator.
        let invalid =
            crate::check_voice_activation_generation(state, original.binding().generation).is_err()
                || !original.original().is_active();
        if invalid {
            original.original().invalidate();
        }
        let proof = if pending.is_none() {
            authority.never_started_closed_admitted_publication(original, executor, permit)
        } else {
            None
        };
        (pending, proof, invalid)
    };
    if notify_closed {
        original.original().notify_closed();
    }
    if let Some(pending) = pending {
        return Some(pending.public_reply(state).await);
    }
    not_started.map(|proof| PublicationReply::NotStarted {
        identity: PublicationIdentity {
            owner: proof.owner().into(),
            executor: hex::encode(proof.executor()),
            permit: proof.permit().into(),
            join: hex::encode(proof.join()),
        },
        code: ClosedCode::Ended,
    })
}

pub(super) async fn settle_inner(
    state: &crate::AppState,
    owner: &str,
    executor: [u8; 32],
) -> Result<(), String> {
    let (pending, settled) = {
        let mut authority = state
            .voice_authority
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let pending = authority.admitted_publication_task_for_cleanup(owner, executor);
        let settled = authority.settle_admitted_join_key(owner, executor)?;
        (pending, settled)
    };
    let drain = settled.finish_outside_guard();
    if let Some(pending) = pending {
        if let Some(task) = pending.negative_task(state) {
            drop(tokio::spawn(task));
        }
    }
    if let Some(drain) = drain {
        drain.wait().await?;
    }
    let retired = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take_retired_admitted_join_key(owner);
    drop(retired);
    crate::voice_admitted_call::collect_retired(state);
    Ok(())
}

/// Explicit acknowledgement of this exact existing terminal cell. This can
/// collect history only after the original task and SDK have actually returned;
/// it does not manufacture a result, settle an engine, or authorize another key.
#[tauri::command]
pub(crate) async fn acknowledge_admitted_voice_key_publication(
    state: tauri::State<'_, crate::AppState>,
    owner: String,
    executor_id: String,
) -> Result<(), String> {
    acknowledge_inner(
        &state,
        &owner,
        crate::voice_join_key::executor(&executor_id)?,
    )
}

pub(super) fn acknowledge_inner(
    state: &crate::AppState,
    owner: &str,
    executor: [u8; 32],
) -> Result<(), String> {
    let pending = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .admitted_publication_task_for_cleanup(owner, executor)
        .ok_or("voice publication is unavailable")?;
    pending.acknowledge_result()?;
    crate::voice_admitted_call::collect_retired(state);
    Ok(())
}

#[tauri::command]
pub(crate) async fn admit_admitted_voice_join_key(
    state: tauri::State<'_, crate::AppState>,
    owner: String,
    executor: String,
) -> Result<crate::voice_authority::JoinKeyAdmission, String> {
    crate::require_unlocked_session(&state).await?;
    admit_inner(&state, &owner, crate::voice_join_key::executor(&executor)?).await
}

#[tauri::command]
pub(crate) async fn publish_admitted_voice_join_key(
    state: tauri::State<'_, crate::AppState>,
    owner: String,
    executor: String,
    permit: String,
    public_key: String,
) -> Result<PublicationReply, String> {
    if public_key.len() != 130
        || !public_key
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("invalid voice public key".into());
    }
    let point = hex::decode(public_key)
        .map_err(|_| "invalid voice public key")?
        .try_into()
        .map_err(|_| "invalid voice public key")?;
    publish_public_inner(
        &state,
        &owner,
        crate::voice_join_key::executor(&executor)?,
        &permit,
        point,
    )
    .await
}

#[tauri::command]
pub(crate) async fn settle_admitted_voice_join_key(
    state: tauri::State<'_, crate::AppState>,
    owner: String,
    executor_id: String,
) -> Result<(), String> {
    settle_inner(
        &state,
        &owner,
        crate::voice_join_key::executor(&executor_id)?,
    )
    .await
}

/// One exact Generation's original pending task. Metadata comparisons are PURE:
/// Coordinator may call matches/is_same but never takes this Cell mutex. The
/// publication worker may then use Cell -> Coordinator ordering for its bounded
/// terminal update without a reverse lock edge or losing canonical Work on unwind.
#[derive(Clone)]
pub(crate) struct PendingPublication {
    cell: Arc<Cell>,
    // LAST, including every canonical registry/callback/observer alias.
    funding: Arc<Funding>,
}
struct Cell {
    activation: ActivationHandle,
    owner: String,
    executor: [u8; 32],
    permit: String,
    join: [u8; 32],
    point: [u8; 65],
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    changed: tokio::sync::Notify,
    returned: Arc<AtomicU64>,
    active_task: AtomicU64,
    completed: AtomicBool,
    acknowledged: AtomicBool,
    #[cfg(test)]
    before_coordinator: Mutex<Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>>,
    #[cfg(test)]
    before_execution_free: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    >,
}
struct State {
    work: Option<AdmittedPublicationWork>,
    rpc: Option<ActivationRpcPermit>,
    actor_work: Option<VoiceAdmittedPresenceWork>,
    outcome: Option<Outcome>,
    result: Option<Result<JoinKeyPublication, String>>,
    disposition: Option<Disposition>,
    started: bool,
    task_serial: u64,
}
enum Outcome {
    Root(VoiceAdmittedPresenceResult),
    // This callback contract certifies no Root entry. An already accepted Work
    // must STILL retire before this can complete the original native operation.
    Unstarted {
        error: VoiceEndpointError,
        // Returned affine custody stays owned until final outside-UI cleanup.
        // It is not a general attempt refund or permission to replay Root work.
        _payment: Option<ActivationRpcPermit>,
    },
    Uncertain,
}
impl PendingPublication {
    pub(crate) fn new(
        activation: ActivationHandle,
        work: AdmittedPublicationWork,
        prepaid: PrepaidPublication,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            cell: Arc::new(Cell {
                owner: work.owner_id().into(),
                executor: work.executor(),
                permit: work.permit().into(),
                join: work.join(),
                point: work.point(),
                activation,
                clock,
                state: Mutex::new(State {
                    work: Some(work),
                    rpc: Some(prepaid.rpc),
                    actor_work: None,
                    outcome: None,
                    result: None,
                    disposition: None,
                    started: false,
                    task_serial: 1,
                }),
                changed: tokio::sync::Notify::new(),
                returned: Arc::new(AtomicU64::new(0)),
                active_task: AtomicU64::new(1),
                completed: AtomicBool::new(false),
                acknowledged: AtomicBool::new(false),
                #[cfg(test)]
                before_coordinator: Mutex::new(None),
                #[cfg(test)]
                before_execution_free: Mutex::new(None),
            }),
            funding: prepaid.funding,
        }
    }
    pub(crate) fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.cell, &other.cell)
    }
    pub(crate) fn owner_id(&self) -> &str {
        &self.cell.owner
    }
    pub(crate) fn matches(&self, work: &AdmittedPublicationWork) -> bool {
        self.matches_generation(
            work.original(),
            work.owner_id(),
            work.executor(),
            work.permit(),
            work.join(),
            work.point(),
        ) && work.original().matches_grant(&work.grant())
    }
    /// Immutable exact-row identity only. Negative cleanup may use this after
    /// End without requiring current UI authority or taking the Cell lock.
    pub(crate) fn matches_generation(
        &self,
        original: &catcoms_sync::voice_v2::call_activation::ActivationObserver,
        owner: &str,
        executor: [u8; 32],
        permit: &str,
        join: [u8; 32],
        point: [u8; 65],
    ) -> bool {
        self.cell.owner == owner
            && self.cell.executor == executor
            && self.cell.permit == permit
            && self.cell.join == join
            && self.cell.point == point
            && self.cell.activation.original().is_same(original)
    }
    /// Install the already-paid SAME task before it can reach Actor. This is
    /// Cell -> Coordinator like terminal completion; the reverse hooks are pure.
    pub(crate) fn install(
        &self,
        authority: &Arc<Mutex<crate::voice_authority::Coordinator>>,
    ) -> Result<(), String> {
        let inner = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
        authority
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .install_admitted_publication_task(inner.work.as_ref().expect("original Work"), self)
    }
    async fn outcome_ready(&self) {
        loop {
            let changed = self.cell.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self
                .cell
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .outcome
                .is_some()
            {
                return;
            }
            changed.await;
        }
    }
    async fn actual_actor_return(&self) {
        self.outcome_ready().await;
        let work = self
            .cell
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .actor_work
            .clone();
        if let Some(work) = work {
            work.wait_retired().await;
        }
    }
    fn publisher(&self) -> Box<Publisher> {
        Box::new(Publisher {
            pending: self.clone(),
            finished: false,
        })
    }
    fn task_returned(&self) -> bool {
        self.cell.returned.load(Ordering::Acquire) == self.cell.active_task.load(Ordering::Acquire)
    }
    /// Observe the original outside-Box return stamp in cross-module fixtures.
    /// No caller can supply a flag, advance the stamp or certify another owner.
    #[cfg(test)]
    pub(crate) fn fixture_task_returned(&self) -> bool {
        self.task_returned()
    }
    /// Pure negative collection predicate, safe under Coordinator: no reverse
    /// Cell lock edge. Completion, explicit acknowledgement and exact outside-
    /// Box return are independent facts; none substitutes for SDK settlement.
    pub(crate) fn is_collectible(&self) -> bool {
        self.cell.completed.load(Ordering::Acquire)
            && self.cell.acknowledged.load(Ordering::Acquire)
            && self.task_returned()
    }
    pub(crate) fn acknowledge_result(&self) -> Result<(), String> {
        if self
            .cell
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .result
            .is_none()
        {
            return Err("voice publication has not completed".into());
        }
        self.cell.acknowledged.store(true, Ordering::Release);
        Ok(())
    }
    pub(crate) async fn reply(
        &self,
        state: &crate::AppState,
    ) -> Result<JoinKeyPublication, String> {
        let mut recovered = false;
        loop {
            let changed = self.cell.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(result) = self
                .cell
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .result
                .clone()
            {
                return result;
            }
            // One explicit negative recovery after actual execution return. A
            // faulty always-ready Clock cannot turn this observer into a new
            // unbounded task loop. Later cleanup/availability may drive it again.
            if !recovered {
                if let Some(task) = self.negative_task(state) {
                    recovered = true;
                    drop(tokio::spawn(task));
                }
            }
            changed.await;
        }
    }
    async fn public_reply(&self, state: &crate::AppState) -> PublicationReply {
        // Wait for the SAME canonical native completion. Never decode Strings
        // to guess whether Root ran; disposition is stored atomically with that
        // outcome from actual typed callbacks and original return witnesses.
        let _ = self.reply(state).await;
        let inner = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
        let identity = || PublicationIdentity {
            owner: self.cell.owner.clone(),
            executor: hex::encode(self.cell.executor),
            permit: self.cell.permit.clone(),
            join: hex::encode(self.cell.join),
        };
        match inner
            .disposition
            .expect("typed original terminal disposition")
        {
            Disposition::Published => {
                // Root may have published before End while final current-native
                // admission refused. Preserve that historical fact without
                // granting present-time key/Ready/RTC authority.
                let publication = match inner.result.as_ref().unwrap() {
                    Ok(publication) => publication.clone(),
                    Err(_) => JoinKeyPublication {
                        owner: self.cell.owner.clone(),
                        executor: hex::encode(self.cell.executor),
                        permit: self.cell.permit.clone(),
                        join: hex::encode(self.cell.join),
                        key_ref: hex::encode(catcoms_sync::voice_v2::key_reference(
                            &self.cell.point,
                        )),
                    },
                };
                PublicationReply::Accepted { publication }
            }
            Disposition::Rejected(code) => PublicationReply::Rejected {
                identity: identity(),
                code,
            },
            Disposition::Closed(code) => PublicationReply::Closed {
                identity: identity(),
                code,
            },
        }
    }
}

struct Publisher {
    pending: PendingPublication,
    finished: bool,
}
impl Publisher {
    fn stage(&mut self, outcome: Outcome, work: Option<&VoiceAdmittedPresenceWork>) {
        let mut candidate = Some(outcome);
        {
            let mut state = self
                .pending
                .cell
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let same = match (state.actor_work.as_ref(), work) {
                (Some(a), Some(b)) => a.is_same(b),
                (None, None) => true,
                _ => false,
            };
            if same && state.outcome.is_none() {
                state.outcome = candidate.take();
            }
        }
        self.finished = true;
        if candidate.is_some() {
            self.pending.cell.activation.close();
        }
        self.pending.cell.changed.notify_waiters();
        drop(candidate); // opaque receipt/custody destructors are outside Cell/Coordinator/UI
    }
}
impl VoiceAdmittedPresencePublisher for Publisher {
    fn accepted(&mut self, work: VoiceAdmittedPresenceWork) -> Result<(), VoiceEndpointError> {
        let mut state = self
            .pending
            .cell
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.actor_work.is_some() {
            return Err(VoiceEndpointError::Protocol(VoiceError::RetryMismatch));
        }
        state.actor_work = Some(work);
        if !self.pending.cell.activation.original().is_active() {
            Err(VoiceEndpointError::Stopped)
        } else {
            Ok(())
        }
    }
    fn completed(
        mut self: Box<Self>,
        result: VoiceAdmittedPresenceResult,
        work: VoiceAdmittedPresenceWork,
    ) {
        self.stage(Outcome::Root(result), Some(&work));
    }
    fn refused(
        mut self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceAdmittedPresenceWork>,
        payment: Option<ActivationRpcPermit>,
    ) {
        self.stage(
            Outcome::Unstarted {
                error,
                _payment: payment,
            },
            work.as_ref(),
        );
    }
}
impl Drop for Publisher {
    fn drop(&mut self) {
        if !self.finished {
            let work = self
                .pending
                .cell
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .actor_work
                .clone();
            self.stage(Outcome::Uncertain, work.as_ref());
        }
    }
}

/// Fixed return signal outside the actual execution Box. All recovery uses the
/// same allocation with an exact checked serial, not a new heap signal per retry.
struct Returned {
    stamp: Arc<AtomicU64>,
    serial: u64,
    cell: Weak<Cell>,
    _funding: Arc<Funding>,
}
impl Drop for Returned {
    fn drop(&mut self) {
        self.stamp.store(self.serial, Ordering::Release);
        if let Some(cell) = self.cell.upgrade() {
            cell.changed.notify_waiters();
        }
    }
}
struct Execution {
    body: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    returned: Option<Returned>,
}
impl Execution {
    fn retire(&mut self) {
        drop(self.body.take());
        drop(self.returned.take());
    }
}
impl Future for Execution {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let Some(body) = &mut self.body else {
            return Poll::Ready(());
        };
        if body.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }
        self.retire();
        Poll::Ready(())
    }
}
impl Drop for Execution {
    fn drop(&mut self) {
        self.retire();
    }
}

/// Weak process references only; no AppState/Actor/Registry backedge enters a key
/// generation. The same original UI mutex and current server instance are checked
/// again at terminal admission, after actual Actor argument return.
#[derive(Clone)]
struct NativeContext {
    authority: Weak<Mutex<crate::voice_authority::Coordinator>>,
    servers: Weak<tokio::sync::Mutex<std::collections::HashMap<u64, crate::ServerEntry>>>,
    requested: Weak<AtomicBool>,
    generation: Weak<AtomicU64>,
    authorized: Weak<AtomicU64>,
    commit: Weak<tokio::sync::Mutex<()>>,
}
impl NativeContext {
    fn new(state: &crate::AppState) -> Self {
        Self {
            authority: Arc::downgrade(&state.voice_authority),
            servers: Arc::downgrade(&state.servers),
            requested: Arc::downgrade(&state.session_lock_requested),
            generation: Arc::downgrade(&state.ui_session_generation),
            authorized: Arc::downgrade(&state.ui_authorized_generation),
            commit: Arc::downgrade(&state.ui_session_commit),
        }
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
    fn binding_current(&self, pending: &PendingPublication) -> Result<bool, ()> {
        let Some(servers) = self.servers.upgrade() else {
            return Ok(false);
        };
        let servers = servers.try_lock().map_err(|_| ())?;
        let binding = pending.cell.activation.binding();
        Ok(servers.get(&binding.server).is_some_and(|entry| {
            entry.instance == binding.instance && entry.group_id == binding.group
        }))
    }
}

struct Worker {
    pending: PendingPublication,
    context: NativeContext,
    commit: Option<tokio::sync::OwnedMutexGuard<()>>,
    negative_only: bool,
    settled: bool,
    returned: Option<Returned>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Pure fence first, then UI release, then all arbitrary notification and
        // heavy destruction. This RAII exists before spawn or first polling.
        if !self.settled {
            self.pending.cell.activation.original().invalidate();
            drop(self.commit.take());
            self.pending.cell.activation.original().notify_closed();
        }
        #[cfg(test)]
        if let Some((entered, resume)) = self
            .pending
            .cell
            .before_execution_free
            .lock()
            .unwrap()
            .take()
        {
            let _ = entered.send(());
            resume.recv().unwrap();
        }
    }
}
impl PendingPublication {
    pub(crate) fn initial_task(
        &self,
        state: &crate::AppState,
        commit: tokio::sync::OwnedMutexGuard<()>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let returned = {
            let mut inner = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
            assert!(!inner.started, "one original key-publication task");
            inner.started = true;
            Returned {
                stamp: self.cell.returned.clone(),
                serial: inner.task_serial,
                cell: Arc::downgrade(&self.cell),
                _funding: self.funding.clone(),
            }
        };
        Worker {
            pending: self.clone(),
            context: NativeContext::new(state),
            commit: Some(commit),
            negative_only: false,
            settled: false,
            returned: Some(returned),
        }
        .run()
    }
    pub(crate) fn negative_task(
        &self,
        state: &crate::AppState,
    ) -> Option<impl Future<Output = ()> + Send + 'static> {
        let returned = {
            let mut inner = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
            if !inner.started
                || inner.result.is_some()
                || !self.cell.activation.original().is_closed()
                || self.cell.returned.load(Ordering::Acquire) != inner.task_serial
            {
                return None;
            }
            let serial = inner.task_serial.checked_add(1)?;
            inner.task_serial = serial;
            self.cell.active_task.store(serial, Ordering::Release);
            Returned {
                stamp: self.cell.returned.clone(),
                serial,
                cell: Arc::downgrade(&self.cell),
                _funding: self.funding.clone(),
            }
        };
        Some(
            Worker {
                pending: self.clone(),
                context: NativeContext::new(state),
                commit: None,
                negative_only: true,
                settled: false,
                returned: Some(returned),
            }
            .run(),
        )
    }
}
impl Worker {
    fn run(mut self) -> Execution {
        let mut execution = Execution {
            body: None,
            returned: self.returned.take(),
        };
        execution.body = Some(Box::pin(self.execute()));
        execution
    }
    fn release_ui(&mut self) {
        drop(self.commit.take());
    }
    fn close_before_ui_release(&mut self) {
        self.pending.cell.activation.original().invalidate();
        self.release_ui();
        self.pending.cell.activation.original().notify_closed();
    }
    fn exact_ui(&self) -> bool {
        let (Some(commit), Some(actual)) = (&self.commit, self.context.commit.upgrade()) else {
            return false;
        };
        Arc::ptr_eq(tokio::sync::OwnedMutexGuard::mutex(commit), &actual)
            && self
                .context
                .generation_current(self.pending.cell.activation.binding().generation)
    }
    async fn closed(&self) -> Result<(), ()> {
        loop {
            if self.pending.cell.activation.original().is_closed()
                || !self
                    .context
                    .generation_current(self.pending.cell.activation.binding().generation)
            {
                return Ok(());
            }
            cadence(self.pending.cell.clock.as_ref()).await?;
        }
    }
    async fn execute(mut self) {
        if !self.negative_only {
            loop {
                if !self.exact_ui() || !self.pending.cell.activation.original().is_active() {
                    self.close_before_ui_release();
                    break;
                }
                match self.context.binding_current(&self.pending) {
                    Ok(true) => break,
                    Ok(false) => {
                        self.close_before_ui_release();
                        break;
                    }
                    Err(()) => {
                        if cadence(self.pending.cell.clock.as_ref()).await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
        if !self.negative_only && self.pending.cell.activation.original().is_active() {
            let prepared = {
                let mut inner = self
                    .pending
                    .cell
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let rpc = inner.rpc.take();
                let work = inner.work.as_ref().expect("canonical key Work");
                rpc.map(|rpc| {
                    (
                        work.grant(),
                        work.original().clone(),
                        work.join(),
                        work.point(),
                        rpc,
                    )
                })
            };
            if let (Some(control), Some((grant, original, join, point, rpc))) =
                (self.pending.cell.activation.publication_control(), prepared)
            {
                drop(control.try_queue_voice_admitted_presence(
                    grant,
                    original,
                    join,
                    point,
                    rpc,
                    self.pending.publisher(),
                ));
            } else {
                // No actor request exists. This is a local unstarted refusal,
                // distinct from an interrupted accepted callback/Root handoff.
                self.pending
                    .publisher()
                    .refused(VoiceEndpointError::Stopped, None, None);
            }
            let closed = tokio::select! {
                biased;
                result = self.closed() => Some(result),
                _ = self.pending.actual_actor_return() => None,
            };
            if closed.is_some() {
                self.close_before_ui_release();
            }
        } else {
            self.close_before_ui_release();
        }
        // Releasing UI after closure cannot discard the SAME real arguments.
        // A parked Root may continue owning them after lock has completed.
        if self
            .pending
            .cell
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .actor_work
            .is_some()
        {
            self.pending.actual_actor_return().await;
        }
        self.finish_original().await;
    }

    async fn finish_original(&mut self) {
        loop {
            if self.commit.is_some() {
                if !self.exact_ui() {
                    self.close_before_ui_release();
                } else {
                    match self.context.binding_current(&self.pending) {
                        Ok(true) => {}
                        Ok(false) => self.close_before_ui_release(),
                        Err(()) => {
                            if cadence(self.pending.cell.clock.as_ref()).await.is_err() {
                                return;
                            }
                            continue;
                        }
                    }
                }
            }
            let mut completion = None;
            let mut retired = None;
            let mut generation_retired = None;
            let mut terminal = false;
            let mut mismatch = false;
            let mut accepted = false;
            let mut disposition = Disposition::Closed(ClosedCode::Unconfirmed);
            let mut coordinator_gone = false;
            #[cfg(test)]
            if let Some((entered, resume)) =
                self.pending.cell.before_coordinator.lock().unwrap().take()
            {
                // Deterministically place lock registration after the outer UI
                // snapshot but before final Coordinator admission, without
                // replacing any actual Root result/Actor-retirement witness.
                entered.wait();
                resume.wait();
            }
            {
                // Deliberate Cell -> Coordinator order. Coordinator's pending
                // hooks compare IMMUTABLE metadata only and never take this lock.
                let mut inner = self
                    .pending
                    .cell
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if inner
                    .actor_work
                    .as_ref()
                    .is_some_and(|work| !work.is_retired())
                {
                    return;
                }
                if let Some(authority) = self.context.authority.upgrade() {
                    let mut authority = authority.lock().unwrap_or_else(|e| e.into_inner());
                    // Lock registers its generation BEFORE awaiting Coordinator.
                    // The outer snapshot therefore cannot authorize this turn:
                    // repeat the pure fence under the final native lock. Closing
                    // here is atomic only; notification/destruction follows UI
                    // release below. A known Root receipt remains historical.
                    let native_current =
                        self.exact_ui() && self.pending.cell.activation.original().is_active();
                    if !native_current {
                        self.pending.cell.activation.original().invalidate();
                    }
                    let State { work, outcome, .. } = &mut *inner;
                    let root = match outcome.as_ref() {
                        Some(Outcome::Root(result)) => Some(result),
                        _ => None,
                    };
                    if let Some(result) = root {
                        let work = work.as_mut().expect("original key Work");
                        accepted = result.is_ok() && native_current;
                        disposition = match result {
                            Ok(_) => Disposition::Published,
                            Err(VoiceError::Capacity) => {
                                Disposition::Rejected(RejectionCode::Capacity)
                            }
                            Err(_) => Disposition::Rejected(RejectionCode::Refused),
                        };
                        completion = authority
                            .finish_admitted_join_key_publication(work, result)
                            .ok();
                        if completion.is_some() {
                            retired =
                                authority.retire_admitted_publication_task(work, &self.pending);
                            generation_retired =
                                authority.take_retired_admitted_join_key(work.owner_id());
                            terminal = true;
                        } else {
                            mismatch = true;
                        }
                    } else if let Some(Outcome::Unstarted { error, .. }) = outcome {
                        disposition = Disposition::Rejected(match *error {
                            VoiceEndpointError::Protocol(VoiceError::Capacity) => {
                                RejectionCode::Capacity
                            }
                            VoiceEndpointError::Stopped => RejectionCode::Stopped,
                            _ => RejectionCode::Refused,
                        });
                        let error = Err(match *error {
                            VoiceEndpointError::Protocol(error) => error,
                            VoiceEndpointError::Stopped => VoiceError::Unauthorized,
                        });
                        let work = work.as_mut().expect("original key Work");
                        completion = authority
                            .finish_admitted_join_key_publication(work, &error)
                            .ok();
                        if completion.is_some() {
                            retired =
                                authority.retire_admitted_publication_task(work, &self.pending);
                            generation_retired =
                                authority.take_retired_admitted_join_key(work.owner_id());
                            terminal = true;
                        }
                    } else if let Some(returned) =
                        self.pending.cell.activation.original().root_returned()
                    {
                        let work = work.as_mut().expect("original key Work");
                        completion = authority
                            .close_admitted_publication_after_root_return(work, &returned)
                            .ok();
                        if completion.is_some() {
                            retired =
                                authority.retire_admitted_publication_task(work, &self.pending);
                            generation_retired =
                                authority.take_retired_admitted_join_key(work.owner_id());
                            terminal = true;
                        }
                    }
                } else {
                    coordinator_gone = true;
                }
            }
            if coordinator_gone {
                // The actual original Coordinator has been destroyed, including
                // its canonical generation table. Actor return was joined above;
                // there is no surviving publication authority/history to recreate.
                self.close_before_ui_release();
                let owners = {
                    let mut inner = self
                        .pending
                        .cell
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    inner.result = Some(Err("Voice is closed.".into()));
                    inner.disposition = Some(Disposition::Closed(ClosedCode::Ended));
                    (
                        inner.work.take(),
                        inner.actor_work.take(),
                        inner.outcome.take(),
                        inner.rpc.take(),
                    )
                };
                drop(owners);
                self.settled = true;
                self.pending.cell.completed.store(true, Ordering::Release);
                self.pending.cell.changed.notify_waiters();
                return;
            }
            if mismatch {
                self.close_before_ui_release();
                let old = self
                    .pending
                    .cell
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .outcome
                    .replace(Outcome::Uncertain);
                drop(old); // rejected opaque receipt drops outside every admission guard
                return; // explicit negative continuation may use actual Root return later
            }
            if terminal {
                if accepted {
                    self.release_ui();
                } else {
                    self.close_before_ui_release();
                }
                let owners = {
                    let mut inner = self
                        .pending
                        .cell
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    (
                        inner.work.take(),
                        inner.actor_work.take(),
                        inner.outcome.take(),
                        inner.rpc.take(),
                    )
                };
                drop(owners);
                drop(retired);
                drop(generation_retired);
                let result = completion
                    .expect("known canonical completion")
                    .finish_outside_guard();
                {
                    let mut inner = self
                        .pending
                        .cell
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    inner.result = Some(result);
                    inner.disposition = Some(disposition);
                }
                self.settled = true;
                self.pending.cell.completed.store(true, Ordering::Release);
                self.pending.cell.changed.notify_waiters();
                return;
            }
            self.close_before_ui_release();
            if cadence(self.pending.cell.clock.as_ref()).await.is_err() {
                return;
            }
        }
    }
}

async fn cadence(clock: &dyn Clock) -> Result<(), ()> {
    let before = clock.monotonic_ms();
    let deadline = before.checked_add(250).ok_or(())?;
    let sleeper = clock.sleep(std::time::Duration::from_millis(250));
    tokio::pin!(sleeper);
    let mut suspended = false;
    let actual = std::future::poll_fn(|cx| match sleeper.as_mut().poll(cx) {
        Poll::Pending => {
            suspended = true;
            Poll::Pending
        }
        Poll::Ready(()) => Poll::Ready(suspended),
    })
    .await;
    if actual && clock.monotonic_ms() >= deadline {
        Ok(())
    } else {
        Err(())
    }
}

/// Complete native publication graph, independent of the active-call runner,
/// Actor RPC, generation watch and Root presence's separately owned payments.
/// Shared owners are deliberately overcounted rather than treating an escaping
/// Weak or callback alias as free after its original table disappears.
fn publication_layout_bytes() -> usize {
    use std::mem::size_of;
    #[repr(C)]
    struct ArcAllocation<T> {
        strong: usize,
        weak: usize,
        value: T,
    }
    fn execution<F: Future>(_: fn(Worker) -> F) -> usize {
        size_of::<F>()
    }
    size_of::<ArcAllocation<Cell>>()
        + size_of::<ArcAllocation<Funding>>()
        + size_of::<ArcAllocation<AtomicU64>>()
        + 2 * 64 // bounded original owner and permit allocations
        + execution(Worker::execute)
        + size_of::<Execution>() + size_of::<Worker>()
        + 3 * size_of::<PendingPublication>()
        + 2 * size_of::<Publisher>() + 2 * size_of::<Outcome>()
        + 2 * size_of::<crate::voice_authority::AdmittedKeyCompletion>()
        + size_of::<crate::voice_authority::AdmittedKeyRetirement>()
        + 2 * size_of::<JoinKeyPublication>() + 6 * 130
        + 2 * size_of::<PublicationReply>() + 5 * 64 // typed public terminal overlap
        + size_of::<ArcAllocation<Mutex<crate::voice_authority::Coordinator>>>()
        + size_of::<ArcAllocation<tokio::sync::Mutex<std::collections::HashMap<u64, crate::ServerEntry>>>>()
        + size_of::<ArcAllocation<AtomicBool>>()
        + 2 * size_of::<ArcAllocation<AtomicU64>>()
        + size_of::<ArcAllocation<tokio::sync::Mutex<()>>>()
        + 256 + 1024 // sanctioned Clock owner and independently boxed sleep
        + 1024 // pinned Tokio task shell, Notify waiters and allocation alignment
}

#[cfg(test)]
mod tests;
