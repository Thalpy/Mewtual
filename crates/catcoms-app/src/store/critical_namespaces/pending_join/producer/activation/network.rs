//! Actual original durable arms → once-bound connected network → retained signed INPUT DATA.
//! There is no SDK/Accepted/ScopeIntent/old-initializer/ready conversion here.
use super::super::super::{execution_payload as payload, execution_state as execution};
use super::super::execution_write::WriteShape;
use super::*;
use catcoms_rt::{Clock, RequestAuthority, SharedRequestKeepalive};
use catcoms_sync::{
    BoundOriginalJoinNetwork, DirectJoinRequestIdentity, OriginalJoinArmData,
    OriginalJoinNetworkContext, OriginalJoinNetworkStatus, OriginalJoinNetworkSubmission,
    OriginalJoinSavedArm, RetainedOriginalJoinNetwork, SealedOriginalJoinInput, SyncError,
};
use std::sync::Mutex;
use std::time::Duration;

// Named logical inventories. Lower16MiB response custody is independent from retained2MiB
// application input; these are NOT allocator/RSS or global native/SDK admission claims.
const INPUT_FEE: usize = 5 * (2 * 1024 * 1024 + 1024 + 16) + 32 * 1024;
// This invoice assumes the repository's actual bounded RR implementation (16MiB maximum
// response). The connected-cancellable capability flag proves its custody/seam contract, NOT
// a generic third-party transport's allocation/RSS bound. Arbitrary host T implementations
// remain trusted code and must supply their own lower-resource contract before production use.
const LOWER_FEE: usize = 16 * 1024 * 1024 + 2 * (9 + 64 * 1024 + 32 * 1024) + 16 * 1024;

struct Control {
    preparation: Mutex<JournalledCheckpointJoinPreparation>,
    identity: Arc<()>,
    binding: [u8; 32],
    request: DirectJoinRequestIdentity,
    negative: RequestCancellation,
    data_negative: RequestCancellation,
    floor: u64,
    target: PeerId,
    _input_fee: CatalogReadReservation,
}
impl std::fmt::Debug for Control {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OriginalJoinControl")
            .finish_non_exhaustive()
    }
}
impl OriginalJoinNetworkContext for Control {
    fn identity(&self) -> Arc<()> {
        self.identity.clone()
    }
    fn binding(&self) -> [u8; 32] {
        self.binding
    }
    fn request(&self) -> DirectJoinRequestIdentity {
        self.request
    }
    fn spent_ordinal_floor(&self) -> u64 {
        self.floor
    }
    fn cancellation(&self) -> RequestCancellation {
        self.negative.clone()
    }
    fn check_original(&self) -> Result<(), SyncError> {
        if self.negative.is_cancelled() {
            Err(SyncError::JoinRejected)
        } else {
            Ok(())
        }
    }
}

pub(in crate::store) struct Shared {
    source: [u8; 32],
    revision: u64,
    control: Arc<Control>,
    cell: Arc<RetainedOriginalJoinNetwork>,
    invocation: Mutex<Option<Invocation>>,
}
impl Shared {
    /// A not-yet-installed ticket has no driver owner. Negative closure retires that upper
    /// capacity without erasing its durable invocation/result. No Store/cell writer is held
    /// when the potentially last lower fee drops. Never wait on a caller-held invocation.
    fn retire_closed_unstarted(&self) {
        if !self.control.negative.is_cancelled() {
            return;
        }
        let Ok(mut invocation) = self.invocation.try_lock() else {
            return;
        };
        let retired = invocation
            .as_mut()
            .map(|original| (original.pending_cancel.take(), original.ticket.take()));
        drop(invocation);
        drop(retired);
    }
}
struct Invocation {
    target: codec::Metadata,
    outcome: Option<Arc<SavedArm>>,
    pending_cancel: Option<RequestCancellation>,
    ticket: Option<Box<dyn OriginalJoinNetworkSubmission>>,
    installed: bool,
}
struct SavedArm {
    control: Arc<Control>,
    data: OriginalJoinArmData,
}
impl std::fmt::Debug for SavedArm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedOriginalJoinArm")
            .finish_non_exhaustive()
    }
}
impl OriginalJoinSavedArm for SavedArm {
    fn context_identity(&self) -> Arc<()> {
        self.control.identity.clone()
    }
    fn binding(&self) -> [u8; 32] {
        self.control.binding
    }
    fn request(&self) -> DirectJoinRequestIdentity {
        self.control.request
    }
    fn data(&self) -> OriginalJoinArmData {
        self.data
    }
    // DATA witness remains valid after task closure. The actual persistence consumer must
    // separately match the original physical Store and immutable source; this grants no send.
    fn check_original_data(&self) -> Result<(), SyncError> {
        if self.control.data_negative.is_cancelled() {
            Err(SyncError::JoinRejected)
        } else {
            Ok(())
        }
    }
}
struct LowerCustody {
    _fee: CatalogReadReservation,
}
impl std::fmt::Debug for LowerCustody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OriginalJoinLowerCustody")
            .finish_non_exhaustive()
    }
}
struct Ticket {
    outcome: Arc<SavedArm>,
    target: PeerId,
    cancel: RequestCancellation,
}
impl std::fmt::Debug for Ticket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OriginalJoinSubmissionTicket")
            .finish_non_exhaustive()
    }
}
impl OriginalJoinNetworkSubmission for Ticket {
    fn saved(&self) -> Arc<dyn OriginalJoinSavedArm> {
        self.outcome.clone()
    }
    fn target(&self) -> PeerId {
        self.target
    }
    fn cancellation(&self) -> RequestCancellation {
        self.cancel.clone()
    }
}
#[derive(Debug)]
struct Deadline {
    clock: Arc<dyn Clock>,
    deadline: u64,
    original: RequestCancellation,
}
#[async_trait::async_trait]
impl RequestAuthority for Deadline {
    fn authorized(&self) -> bool {
        !self.original.is_cancelled() && self.clock.monotonic_ms() < self.deadline
    }
    async fn revoked(&self) {
        let mut original = self.original.clone();
        while self.authorized() {
            let sleep = self.clock.sleep(Duration::from_millis(
                self.deadline.saturating_sub(self.clock.monotonic_ms()),
            ));
            if !self.authorized() {
                return;
            }
            tokio::select! {_=original.cancelled()=>return,_=sleep=>{}}
        }
    }
}

/// Sole typed task/T owner. Store retains only Shared DATA/fees; dropping this object closes
/// its sender before the transport, while an already received input survives in the cell.
pub struct CheckpointJoinNetwork<T> {
    task: OriginalPredecessorTask,
    bound: BoundOriginalJoinNetwork<T>,
    shared: Arc<Shared>,
    _activation_fee: CatalogReadReservation,
}
impl<T> std::fmt::Debug for CheckpointJoinNetwork<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckpointJoinNetwork")
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckpointJoinSubmissionStatus {
    Armed,
    Spent,
    InputHeld,
    Exhausted,
    Closed,
    Busy,
}

impl<T: MeshTransport> ActivatedCheckpointJoin<T> {
    /// Consume the exact original task and T into the journal network owner. All normal local
    /// resource/policy checks precede source transfer. A revoked/invalid conversion closes its
    /// old lease; exact durable keys remain journaled and cannot become a replacement package.
    pub fn into_network_execution(
        self,
        store: &ServerStore,
    ) -> Result<CheckpointJoinNetwork<T>, AppError> {
        let policy = store.pending_join_policy.as_ref().ok_or_else(refused)?;
        policy.execution.as_ref().ok_or_else(refused)?;
        if self.task.is_closed() {
            return Err(refused());
        }
        let input_fee = policy.pool.try_reserve(INPUT_FEE)?;
        self.preparation.confirm_existing(store)?;
        let Self {
            task,
            mut preparation,
            transport,
            target,
            _prefix: _,
            _activation_fee,
        } = self;
        let identity = task.join_execution_identity();
        let negative = task.observer();
        let request = match &preparation.source {
            Source::Local(source) => source.request_identity()?,
            Source::Recovered(source) => source.request_identity()?,
            Source::Taken => return Err(refused()),
        };
        let source = std::mem::replace(&mut preparation.source, Source::Taken);
        let source_id = preparation.metadata.source;
        let revision = preparation.metadata.revision;
        let binding = preparation.metadata.binding;
        let floor = preparation
            .metadata
            .execution
            .map_or(0, |state| state.ordinal);
        let control = Arc::new(Control {
            preparation: Mutex::new(preparation),
            identity,
            binding,
            request,
            negative,
            data_negative: store.chat_mount.observer(),
            floor,
            target,
            _input_fee: input_fee,
        });
        let cell = match source {
            Source::Local(source) => {
                RetainedOriginalJoinNetwork::from_original_local(source, control.clone())?
            }
            Source::Recovered(source) => {
                RetainedOriginalJoinNetwork::from_authenticated_source(source, control.clone())?
            }
            Source::Taken => return Err(refused()),
        };
        let bound = cell
            .clone()
            .bind_original_transport(transport, target)
            .map_err(|_| refused())?;
        let shared = Arc::new(Shared {
            source: source_id,
            revision,
            control,
            cell,
            invocation: Mutex::new(None),
        });
        store.register_join_network_shared(&shared)?;
        Ok(CheckpointJoinNetwork {
            task,
            bound,
            shared,
            _activation_fee,
        })
    }
}

impl ServerStore {
    /// Save actual retained signed reply DATA after the sole network owner/future was dropped.
    /// Walks only the fixed five runtime slots, without a source/master/key getter or an active
    /// task. Repeated success may report the same saved INPUT; this is not SDK/Current evidence.
    /// A cold parsed input has no runtime cell and cannot be promoted through this method.
    pub fn persist_retained_checkpoint_join_inputs(
        &self,
        rng: &mut impl CryptoRngCore,
    ) -> Result<usize, AppError> {
        let retained = {
            let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            let mounted = mounted.as_ref().ok_or_else(refused)?;
            mounted.networks.clone()
        };
        let mut saved = 0;
        for shared in retained.iter().flatten() {
            if shared.cell.try_status() == Some(OriginalJoinNetworkStatus::InputHeld) {
                self.persist_one_retained_checkpoint_input(shared, rng)?;
                saved += 1;
            }
        }
        Ok(saved)
    }

    fn persist_one_retained_checkpoint_input(
        &self,
        shared: &Shared,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let key = Zeroizing::new(self.keys.db_key()?);
        let received = shared.cell.seal_received_input(&key, rng)?;
        drop(key);
        let mut preparation = shared.control.preparation.lock().map_err(|_| refused())?;
        preparation.persist_received_execution_input(self, received, rng)
    }

    fn register_join_network_shared(&self, shared: &Arc<Shared>) -> Result<(), AppError> {
        let preparation = shared.control.preparation.lock().map_err(|_| refused())?;
        // Reconcile/drop old lower owners BEFORE admission/catalog. Once closed with no held
        // input, this exact cell cannot create another result or accept a replacement ticket.
        let previous = {
            let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            mounted
                .as_ref()
                .ok_or_else(refused)?
                .networks
                .iter()
                .flatten()
                .find(|entry| entry.source == shared.source)
                .cloned()
        };
        if let Some(old) = &previous {
            old.retire_closed_unstarted();
            if old.revision >= shared.revision
                || !old.control.negative.is_cancelled()
                || !matches!(
                    old.cell.try_status(),
                    Some(
                        OriginalJoinNetworkStatus::Closed
                            | OriginalJoinNetworkStatus::Spent
                            | OriginalJoinNetworkStatus::Idle
                    )
                )
            {
                return Err(refused());
            }
        }
        let mut retired = None;
        self.with_original_join_catalog_capture(&preparation.original.capture, |permit| {
            let mut mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            let mounted = mounted.as_mut().ok_or_else(refused)?;
            permit.validate_original()?;
            if shared.control.negative.is_cancelled()
                || mounted.pending.is_some()
                || !mounted
                    .current
                    .rows
                    .iter()
                    .flatten()
                    .any(|row| row == &preparation.metadata)
            {
                return Err(refused());
            }
            let index = if let Some(index) = mounted.networks.iter().position(|entry| {
                entry
                    .as_ref()
                    .is_some_and(|entry| entry.source == shared.source)
            }) {
                let old = mounted.networks[index].as_ref().ok_or_else(refused)?;
                if !previous
                    .as_ref()
                    .is_some_and(|expected| Arc::ptr_eq(old, expected))
                    || old.revision >= shared.revision
                    || !old.control.negative.is_cancelled()
                {
                    return Err(refused());
                }
                index
            } else {
                mounted
                    .networks
                    .iter()
                    .position(Option::is_none)
                    .ok_or_else(refused)?
            };
            permit.validate_original()?;
            if shared.control.negative.is_cancelled() {
                return Err(refused());
            }
            retired = mounted.networks[index].replace(shared.clone());
            Ok(())
        })?;
        drop(preparation);
        drop(retired);
        Ok(())
    }
}

fn data(arm: execution::Arm) -> OriginalJoinArmData {
    OriginalJoinArmData {
        grant: arm.grant,
        ordinal: arm.ordinal,
        revision: arm.scheduling_revision,
        activation: arm.activation,
        invocation: arm.invocation,
        deadline_ms: arm.deadline_ms,
    }
}

impl<T: MeshTransport> CheckpointJoinNetwork<T> {
    /// Cancel future submissions without deleting saved units, source keys or received DATA.
    pub fn close(&self) {
        self.task.close();
        self.shared.cell.close();
    }
    /// Bounded local observation; Busy never implies a completed request or saved input.
    pub fn status(&self) -> CheckpointJoinSubmissionStatus {
        self.shared.retire_closed_unstarted();
        match self.shared.cell.try_status() {
            Some(OriginalJoinNetworkStatus::Armed) => CheckpointJoinSubmissionStatus::Armed,
            Some(OriginalJoinNetworkStatus::InputHeld) => CheckpointJoinSubmissionStatus::InputHeld,
            Some(OriginalJoinNetworkStatus::Closed) => CheckpointJoinSubmissionStatus::Closed,
            None | Some(OriginalJoinNetworkStatus::Submitting) => {
                CheckpointJoinSubmissionStatus::Busy
            }
            _ => CheckpointJoinSubmissionStatus::Spent,
        }
    }

    /// Save only the initial configured allowance (and exact v1 NotGranted migration).
    /// This performs no request and creates no ticket. Repeated/lost returns never rewrite
    /// an existing saved grant, including after configuration changes or exhaustion.
    pub fn ensure_initial_grant(
        &mut self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if self.task.is_closed() {
            return Err(refused());
        }
        let settings = store
            .pending_join_policy
            .as_ref()
            .and_then(|policy| policy.execution)
            .ok_or_else(refused)?;
        let mut preparation = self
            .shared
            .control
            .preparation
            .lock()
            .map_err(|_| refused())?;
        if preparation.metadata.execution.is_none() {
            let mut target = preparation.metadata.clone();
            target.execution = Some(execution::State::not_granted());
            let observation = store.capture_original_join_execution(
                &preparation.original.capture,
                &self.task,
                self.shared.source,
                self.shared.revision,
            )?;
            preparation.write_execution_change(
                store,
                &target,
                None,
                WriteShape::Migrate,
                Some(observation),
                rng,
            )?;
        }
        let mut state = preparation.metadata.execution.ok_or_else(refused)?;
        if state.grant == 0 {
            let mut target = preparation.metadata.clone();
            state.grant = 1;
            state.limit = settings.initial_limit;
            target.execution = Some(state);
            let observation = store.capture_original_join_execution(
                &preparation.original.capture,
                &self.task,
                self.shared.source,
                self.shared.revision,
            )?;
            preparation.write_execution_change(
                store,
                &target,
                None,
                WriteShape::OneSource,
                Some(observation),
                rng,
            )?;
        }
        Ok(())
    }

    /// Recover the SAME warm invocation first; creating a distinct later arm is explicit via
    /// next_submission(). Durable units are never reset by this object, task, or mount.
    pub fn ensure_submission_armed(
        &mut self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
        clock: Arc<dyn Clock>,
    ) -> Result<CheckpointJoinSubmissionStatus, AppError> {
        let status = self.status();
        if matches!(
            status,
            CheckpointJoinSubmissionStatus::Armed
                | CheckpointJoinSubmissionStatus::InputHeld
                | CheckpointJoinSubmissionStatus::Closed
                | CheckpointJoinSubmissionStatus::Busy
        ) {
            return Ok(status);
        }
        if !self.bound.can_submit_original() {
            return Err(refused());
        }
        let policy = store.pending_join_policy.as_ref().ok_or_else(refused)?;
        let settings = policy.execution.ok_or_else(refused)?;
        self.ensure_initial_grant(store, rng)?;
        let mut invocation = self.shared.invocation.try_lock().map_err(|_| refused())?;
        let mut preparation = self
            .shared
            .control
            .preparation
            .lock()
            .map_err(|_| refused())?;
        if invocation.is_none() {
            let mut state = preparation.metadata.execution.ok_or_else(refused)?;
            if state.input.is_some() {
                return Ok(CheckpointJoinSubmissionStatus::InputHeld);
            }
            if state.spent >= state.limit {
                return Ok(CheckpointJoinSubmissionStatus::Exhausted);
            }
            // Capacity BEFORE debit; one shared strong lower owner follows the actual driver.
            let lower: SharedRequestKeepalive = Arc::new(LowerCustody {
                _fee: policy.pool.try_reserve(LOWER_FEE)?,
            });
            let mut activation = [0; 32];
            let mut nonce = [0; 32];
            rng.fill_bytes(&mut activation);
            rng.fill_bytes(&mut nonce);
            state.spent = state.spent.checked_add(1).ok_or_else(refused)?;
            state.ordinal = state.ordinal.checked_add(1).ok_or_else(refused)?;
            let deadline = clock
                .monotonic_ms()
                .checked_add(settings.timeout_ms)
                .ok_or_else(refused)?;
            let arm = execution::Arm {
                source: payload::source_identity(&preparation.metadata),
                grant: state.grant,
                ordinal: state.ordinal,
                scheduling_revision: preparation.metadata.revision,
                activation,
                invocation: nonce,
                deadline_ms: deadline,
            };
            state.arm = Some(arm);
            let mut target = preparation.metadata.clone();
            target.execution = Some(state);
            let original = self.shared.control.negative.clone();
            let cancel = original
                .clone()
                .with_authority(Arc::new(Deadline {
                    clock,
                    deadline,
                    original,
                }))
                .retain_custody(lower);
            *invocation = Some(Invocation {
                target,
                outcome: None,
                pending_cancel: Some(cancel),
                ticket: None,
                installed: false,
            });
        }
        let original = invocation.as_mut().ok_or_else(refused)?;
        if original.installed {
            return Ok(self.status());
        }
        let observation = store.capture_original_join_execution(
            &preparation.original.capture,
            &self.task,
            self.shared.source,
            self.shared.revision,
        )?;
        preparation.write_execution_change(
            store,
            &original.target,
            None,
            WriteShape::OneSource,
            Some(observation),
            rng,
        )?;
        // Construct saved outcome ONLY after the actual covering byte/file/root result.
        // This private object is never decoded from the counter labels or an "admitted" flag.
        if original.outcome.is_none() {
            let arm = original
                .target
                .execution
                .and_then(|state| state.arm)
                .ok_or_else(refused)?;
            original.outcome = Some(Arc::new(SavedArm {
                control: self.shared.control.clone(),
                data: data(arm),
            }));
        }
        if self.task.is_closed() {
            let retired = original.pending_cancel.take();
            drop(preparation);
            drop(invocation);
            drop(retired);
            return Ok(CheckpointJoinSubmissionStatus::Closed);
        }
        if original.ticket.is_none() {
            original.ticket = Some(Box::new(Ticket {
                outcome: original.outcome.as_ref().ok_or_else(refused)?.clone(),
                target: self.bound_target(),
                cancel: original.pending_cancel.take().ok_or_else(refused)?,
            }));
        }
        let ticket = original.ticket.take().ok_or_else(refused)?;
        match self.shared.cell.install_original_submission(ticket) {
            Ok(()) => {
                original.installed = true;
                Ok(CheckpointJoinSubmissionStatus::Armed)
            }
            Err(returned) => {
                let (ticket, _) = returned.into_parts();
                original.ticket = Some(ticket);
                Err(refused())
            }
        }
    }
    fn bound_target(&self) -> PeerId {
        self.shared.control.target
    }
    /// Explicitly retire only the previous installed Spent invocation. The next ensure call
    /// must save another debit from the SAME grant before acquiring a once-only ticket.
    pub fn next_submission(&mut self) -> Result<(), AppError> {
        if self.status() != CheckpointJoinSubmissionStatus::Spent {
            return Err(refused());
        }
        let mut invocation = self.shared.invocation.try_lock().map_err(|_| refused())?;
        if invocation
            .as_ref()
            .is_some_and(|original| !original.installed)
        {
            return Err(refused());
        }
        *invocation = None;
        Ok(())
    }
    /// Retain the actual inviter-signed reply as encrypted DATA. A task/pause/row cancellation
    /// may close future sends without discarding this already-authenticated result. Same-key
    /// retry reuses the Sync cache and Store's original candidate; no network or new KP occurs.
    pub fn persist_received_input(
        &mut self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        store.persist_one_retained_checkpoint_input(&self.shared, rng)
    }

    /// Submit only the exact saved arm on the once-owned connected transport. Future Drop
    /// spends that arm; original lower completion ownership survives independently. A valid
    /// signature yields retained DATA, not SDK/selected-KP or member admission success.
    pub async fn exchange_armed(&mut self) -> Result<CheckpointJoinSubmissionStatus, AppError> {
        self.bound.exchange_armed().await?;
        Ok(self.status())
    }
}
