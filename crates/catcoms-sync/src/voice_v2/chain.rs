//! Inactive native confirmation capacity. This is NOT whole-chain readiness: protected native
//! and browser registry rows, atomic encrypted intake, helper agreement and measured SDP/IPC
//! phase transfer must still compose before live dispatch. Packet-table claims also protect the
//! actual pair role's three identities per confirmation role. Derivation and all three native
//! confirmation roles have consumers; SDP phase transfer remains open and live dispatch stays off.
//!
//! Fixed history never owns Workflow, an engine lender or a native registry cell. Workflow owns
//! capacity only and references its control weakly. Actual input/engine/lower credits keep retired
//! progress occupied; an idle old handle cannot. Neither owner may store an action strongly.

use super::crypto_actions::{slots, CryptoAction, Key};
use super::resources::{ByteBudget, Credit, Reservation, ResourceKind, Resources, WorkLane};
use super::{CallLease, DeviceId, Nonce, Scope, VoiceError};
use std::sync::{Arc, Mutex, Weak};

pub(super) const ROW_BYTES: usize = 1536;
const ROWS: usize = 12;
const NO_ROW: u8 = u8::MAX;
mod admission;
mod derivation;
mod outgoing;
mod start;
pub(crate) use admission::ConfirmationAdmission;
pub(super) use admission::ConfirmationConsent;
pub(in crate::voice_v2) use start::{StartAdmission, StartCandidate};
// Confirmation-only envelope. Stacking this on the existing SDP reserve is not whole-chain
// readiness; the supported chain still needs a measured, explicit phase transfer.
const VARIABLE_BYTES: usize = 96 * 1024;
const METADATA_BYTES: usize = 2048;

/// Selected inside LocalJoin from actual native state, never an IPC-supplied descriptor.
#[derive(Clone)]
pub(super) struct Binding {
    pub scope: Scope,
    pub epoch: u64,
    pub local: DeviceId,
    pub local_join: Nonce,
    pub local_key: Nonce,
    pub handshake: Nonce,
    pub peer: DeviceId,
    pub peer_join: Nonce,
    pub peer_key: Nonce,
    pub generation: u64,
    pub call: CallLease,
    pub pair: CallLease,
    pub deadline: u64,
    pub low: bool,
}
impl Binding {
    fn same(&self, other: &Self) -> bool {
        self.scope == other.scope
            && self.epoch == other.epoch
            && self.local == other.local
            && self.local_join == other.local_join
            && self.local_key == other.local_key
            && self.handshake == other.handshake
            && self.peer == other.peer
            && self.peer_join == other.peer_join
            && self.peer_key == other.peer_key
            && self.generation == other.generation
            && self.call.is_same(&other.call)
            && self.pair.is_same(&other.pair)
            && self.deadline == other.deadline
            && self.low == other.low
    }
    fn live(&self) -> bool {
        self.call.is_live() && self.pair.is_live()
    }
}

// Shared fixed-object custody, independent of progress. A closed control or an abandoned row
// can retain these objects after Workflow drains, so their charge must follow the last such
// owner too. No reference points from this owner back into Workflow/control/actions.
pub(super) struct PlanBinding {
    value: Binding,
    _metadata: Credit,
}
impl std::ops::Deref for PlanBinding {
    type Target = Binding;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

struct HistoryState {
    closed: bool,
    serial: u64,
    // Only one unpublished row can cross a synchronous sole-LocalJoin transaction at a time.
    // Published actions/candidates are independent. A global monotonic ticket prevents stale
    // abandonment/publication while this compact mask keeps the original metadata allowance.
    active_row: u8,
    issued: u16,
    // The native canonical role is immutable. Share the original three positions instead of
    // allocating independent low/high tables which could conceal another candidate allowance.
    candidates: Candidates,
    // Header authentication cannot reveal the echoed Hello nonce. Admit Proof (low) or
    // Finish (high) in three independent role positions; never charge the arrival attempt.
    // The bounded list nodes and each response pointer live in their prepaid input records,
    // rather than hiding growing per-role arrays in the shared 2 KiB metadata allowance.
    followups: Option<Arc<ConfirmationAdmission>>,
}
enum Candidates {
    Low([Option<Arc<StartAdmission>>; 3]),
    High([Option<Arc<ConfirmationAdmission>>; 3]),
}
impl Candidates {
    fn high(&self) -> Result<&[Option<Arc<ConfirmationAdmission>>; 3], VoiceError> {
        match self {
            Self::High(rows) => Ok(rows),
            Self::Low(_) => Err(VoiceError::Unauthorized),
        }
    }
    fn high_mut(&mut self) -> Result<&mut [Option<Arc<ConfirmationAdmission>>; 3], VoiceError> {
        match self {
            Self::High(rows) => Ok(rows),
            Self::Low(_) => Err(VoiceError::Unauthorized),
        }
    }
}
struct HistoryPlan {
    resources: Arc<Resources>,
    binding: Arc<PlanBinding>,
    live: CallLease,
    bytes: Arc<Reservation>,
    slots: Arc<slots::ReservedSlots>,
    pool: Arc<slots::Slots>,
    state: Mutex<HistoryState>,
}

/// Capacity only. No action or registry cell can be stored here: those owners can themselves
/// retain actual input credits, which would form an uncollectable custody cycle.
pub struct Workflow {
    resources: Arc<Resources>,
    binding: Arc<PlanBinding>,
    live: CallLease,
    control: Weak<PreparedChain>,
    reserve: Arc<Reservation>,
    crypto: Arc<WorkLane>,
    lowers: [Arc<WorkLane>; 2],
    packets: super::packets::ConfirmationCapacity,
    state: Mutex<WorkflowState>,
}
struct WorkflowState {
    closed: bool,
    // One IPC owner for the entire chain, including distinct rows. A lost renderer reply may
    // keep this occupied while the original physical work/result/cleanup still drains.
    handoff_busy: bool,
}

/// Affine capacity for application-owned native/browser handoff copies. The trusted adapter
/// chooses a measured byte ceiling before creating copies and retains this owner until their
/// actual observable work drains. It grants no engine, result, packet or RTC authority.
///
/// This is a loan from the existing confirmation envelope, never another independent reserve
/// or Crypto slot. Keeping the actual Workflow alive also prevents overlapping replacement
/// envelopes after End; retaining only its Reservation would lose that ownership invariant.
#[must_use = "retain the handoff loan until every application-owned copy and work item drains"]
pub struct HandoffLoan {
    workflow: Arc<Workflow>,
    _credit: Credit,
}
impl HandoffLoan {
    /// Irreversibly retire this ORIGINAL confirmation plan after a derivation-only handoff.
    /// Ordinary loan Drop only returns copy capacity to an open Workflow; it does not close
    /// the plan. This explicit native capability closes that exact plan without a lookup by
    /// handshake ID, a replacement grant, or retaining a strong control owner in history.
    ///
    /// The trusted adapter must first establish actual browser/provider/RPC drainage and
    /// engine settlement. Call outside Engine/Group/Policy/native registry guards: close may
    /// retire unstarted actions and release credits. The consumed loan drops LAST. Other
    /// credited inputs, engines, lower work or copy owners still retain the original Workflow;
    /// returning from this method is closure, never evidence that all96KiB has drained.
    /// This ends the confirmation plan, so it is not the future SDP phase-transfer API.
    pub fn retire_confirmation(self) {
        if let Some(control) = self.workflow.control.upgrade() {
            control.close();
        } else {
            // The exact control was already destroyed; the loan still owns its original
            // Workflow. It must not become an open reserve just because its control is gone.
            self.workflow.close();
        }
        drop(self);
    }
}
impl Drop for HandoffLoan {
    fn drop(&mut self) {
        let mut state = self
            .workflow
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        debug_assert!(state.handoff_busy);
        state.handoff_busy = false;
        // Credit/Workflow destruction follows this guard. Never run downstream resource
        // destructors under the workflow fence.
    }
}
struct Control {
    closed: bool,
    workflow: Option<Arc<Workflow>>,
    actions: [Weak<CryptoAction>; ROWS],
}

/// Opaque inactive native plan. Twelve local rows are protected (derive, three flights for
/// each of three candidates, two SDP directions). Derive and all native confirmation roles consume
/// rows; SDP phase transfer and live readiness remain separate unimplemented boundaries.
/// This grants no network, engine, browser or RTC readiness. Close is irreversible.
pub struct PreparedChain {
    binding: Arc<PlanBinding>,
    live: CallLease,
    history: Arc<HistoryPlan>,
    control: Mutex<Control>,
}
redacted_debug!(Workflow, PreparedChain, HandoffLoan);

pub(super) struct PreparedHistory {
    pub record: Credit,
    pub slot: slots::Slot,
    pub issue: RowIssue,
}
pub(super) struct RowIssue {
    plan: Arc<HistoryPlan>,
    workflow: Weak<Workflow>,
    key: Key,
    ticket: u64,
    row: usize,
    unpublished: bool,
}
impl RowIssue {
    pub(super) fn validate(
        &self,
        resources: &Arc<Resources>,
        slots: &Arc<slots::Slots>,
        scope: Scope,
        call: &CallLease,
        key: Key,
        budget: &ByteBudget,
    ) -> Result<(), VoiceError> {
        let ByteBudget::Workflow(workflow) = budget else {
            return Err(VoiceError::Scope);
        };
        if Arc::ptr_eq(resources, &self.plan.resources)
            && Arc::ptr_eq(slots, &self.plan.pool)
            && scope == self.plan.binding.scope
            && call.is_same(&self.plan.binding.call)
            && self.key == key
            && Arc::ptr_eq(&workflow.binding, &self.plan.binding)
            && self.workflow.ptr_eq(&Arc::downgrade(workflow))
        {
            Ok(())
        } else {
            Err(VoiceError::Scope)
        }
    }
    pub(super) fn lease(&self) -> CallLease {
        self.plan.live.clone()
    }
    pub(super) fn epoch(&self) -> u64 {
        self.plan.binding.epoch
    }
    pub(super) fn binding(&self) -> Arc<PlanBinding> {
        self.plan.binding.clone()
    }

    /// Allocation, tagging, input construction and authority validation precede this boundary.
    /// The closure only inserts the prepared action. Close serializes here; after issuance
    /// there is no recoverable tail that could require a replacement row.
    pub(super) fn publish<T>(mut self, apply: impl FnOnce() -> T) -> Result<T, VoiceError> {
        let mut state = self.plan.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed
            || !self.plan.live.is_live()
            || !self.plan.binding.live()
            || (state.active_row != self.row as u8 || state.serial != self.ticket)
        {
            return Err(VoiceError::Unauthorized);
        }
        state.issued |= 1 << self.row;
        state.active_row = NO_ROW;
        self.unpublished = false;
        Ok(apply())
    }
}
impl Drop for RowIssue {
    fn drop(&mut self) {
        if self.unpublished {
            let mut state = self.plan.state.lock().unwrap_or_else(|e| e.into_inner());
            if !state.closed && state.active_row == self.row as u8 && state.serial == self.ticket {
                state.active_row = NO_ROW;
            }
        }
    }
}
impl HistoryPlan {
    fn prepare(
        self: &Arc<Self>,
        key: Key,
        workflow: &Arc<Workflow>,
    ) -> Result<PreparedHistory, VoiceError> {
        if key != Key::Derive(self.binding.handshake) {
            return Err(VoiceError::Scope);
        }
        self.prepare_row(0, key, workflow)
    }
    fn prepare_row(
        self: &Arc<Self>,
        row: usize,
        key: Key,
        workflow: &Arc<Workflow>,
    ) -> Result<PreparedHistory, VoiceError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || !self.live.is_live() || !self.binding.live() {
            return Err(VoiceError::Unauthorized);
        }
        if row >= ROWS || state.active_row != NO_ROW || state.issued & (1 << row) != 0 {
            return Err(VoiceError::Capacity);
        }
        let ticket = state.serial.checked_add(1).ok_or(VoiceError::Capacity)?;
        let record = ByteBudget::Reserved(self.bytes.clone()).charge(
            &self.resources,
            self.binding.scope.group,
            ROW_BYTES,
            ResourceKind::Retained,
        )?;
        let slot = self.slots.loan(&self.pool)?;
        state.serial = ticket;
        state.active_row = row as u8;
        Ok(PreparedHistory {
            record,
            slot,
            issue: RowIssue {
                plan: self.clone(),
                workflow: Arc::downgrade(workflow),
                key,
                ticket,
                row,
                unpublished: true,
            },
        })
    }
    fn close(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        // Only unused capacity returns. Actual prepared/published rows keep credits and slots
        // independently; neither an idle row issue nor permanent history owns Workflow.
        self.bytes.close();
        self.slots.close();
    }
}

impl PreparedChain {
    pub(super) fn new(
        resources: Arc<Resources>,
        binding: Binding,
        slots: Arc<slots::Slots>,
        packet_slots: &super::packets::PacketSlots,
    ) -> Result<Arc<Self>, VoiceError> {
        if !binding.live() {
            return Err(VoiceError::Unauthorized);
        }
        let packets = packet_slots.reserve_confirmation(binding.low)?;
        let binding = Arc::new(PlanBinding {
            _metadata: resources.charge(
                binding.scope.group,
                METADATA_BYTES,
                ResourceKind::Retained,
            )?,
            value: binding,
        });
        let live = CallLease::default();
        let history = Arc::new(HistoryPlan {
            resources: resources.clone(),
            binding: binding.clone(),
            live: live.clone(),
            bytes: resources.reserve(binding.scope.group, ROWS * ROW_BYTES)?,
            pool: slots.clone(),
            slots: slots.reserve(ROWS)?,
            state: Mutex::new(HistoryState {
                closed: false,
                serial: 0,
                active_row: NO_ROW,
                issued: 0,
                candidates: if binding.low {
                    Candidates::Low(std::array::from_fn(|_| None))
                } else {
                    Candidates::High(std::array::from_fn(|_| None))
                },
                followups: None,
            }),
        });
        let reserve = resources.reserve(binding.scope.group, VARIABLE_BYTES)?;
        let crypto = resources.reserve_crypto_lane(binding.scope.group)?;
        let lowers = [
            resources.reserve_forward_lane(binding.scope.group)?,
            resources.reserve_forward_lane(binding.scope.group)?,
        ];
        let chain = Arc::new_cyclic(|control| Self {
            binding: binding.clone(),
            live: live.clone(),
            history,
            control: Mutex::new(Control {
                closed: false,
                actions: std::array::from_fn(|_| Weak::new()),
                workflow: Some(Arc::new(Workflow {
                    resources: resources.clone(),
                    binding,
                    live,
                    control: control.clone(),
                    reserve,
                    crypto,
                    lowers,
                    packets,
                    state: Mutex::new(WorkflowState {
                        closed: false,
                        handoff_busy: false,
                    }),
                })),
            }),
        });
        // Prepare outside the process lock. A losing plan drops after install releases it;
        // no resource destructor or callback may run under that shared pool mutex.
        {
            let control = chain.control.lock().unwrap_or_else(|e| e.into_inner());
            resources.install_workflow(
                chain.binding.scope.group,
                control.workflow.as_ref().expect("prepared workflow"),
            )?;
        }
        Ok(chain)
    }
    /// End/expiry closes pending effects and returns only unused capacity. Existing engines
    /// and input readers remain charged through actual settlement/last drop, even after End.
    pub fn close(&self) {
        let (workflow, actions) = {
            let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
            if control.closed {
                return;
            }
            control.closed = true;
            self.live.revoke();
            (
                control.workflow.take(),
                std::mem::replace(&mut control.actions, std::array::from_fn(|_| Weak::new())),
            )
        };
        self.history.close();
        // Never enter Engine under Control/History. Weak tracking avoids cycles and final
        // engine/result paths may themselves own Engine while observing native lifecycle.
        for action in actions.into_iter().filter_map(|action| action.upgrade()) {
            action.retire_if_unstarted();
        }
        if let Some(workflow) = workflow {
            workflow.close();
        }
    }
    pub(super) fn matches(
        &self,
        binding: &Binding,
        resources: &Arc<Resources>,
        slots: &Arc<slots::Slots>,
        mono: u64,
    ) -> bool {
        self.binding.same(binding)
            && Arc::ptr_eq(resources, &self.history.resources)
            && Arc::ptr_eq(slots, &self.history.pool)
            && mono < self.binding.deadline
            && self.binding.live()
            && self.live.is_live()
    }
    pub(super) fn lease(&self) -> CallLease {
        self.live.clone()
    }
    pub(super) fn prepare_derive(&self) -> Result<(PreparedHistory, ByteBudget), VoiceError> {
        let control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if control.closed {
            return Err(VoiceError::Unauthorized);
        }
        let workflow = control.workflow.as_ref().ok_or(VoiceError::Unauthorized)?;
        let row = self
            .history
            .prepare(Key::Derive(self.binding.handshake), workflow)?;
        Ok((row, ByteBudget::Workflow(workflow.clone())))
    }
    pub(super) fn register_action(&self, action: &Arc<CryptoAction>) {
        let closed = {
            let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
            if !control.closed {
                // Fixed weak tracking needs no heap allocation and cannot form a custody cycle.
                // Reusing a dead tracking entry never reopens its separately issued role bit.
                let empty = control
                    .actions
                    .iter_mut()
                    .find(|entry| entry.strong_count() == 0)
                    .expect("bounded prepared chain actions");
                *empty = Arc::downgrade(action);
            }
            control.closed
        };
        // Close can race the gap after row publication. Its irreversible lease already fences
        // engines/effects; finish unstarted retirement here without inverting locks.
        if closed {
            action.retire_if_unstarted();
        }
    }
}
impl Drop for PreparedChain {
    fn drop(&mut self) {
        self.close();
    }
}
impl Workflow {
    pub(crate) fn charge(
        self: &Arc<Self>,
        resources: &Arc<Resources>,
        group: [u8; 32],
        bytes: usize,
        kind: ResourceKind,
    ) -> Result<Credit, VoiceError> {
        if !Arc::ptr_eq(resources, &self.resources) || group != self.binding.scope.group {
            return Err(VoiceError::Scope);
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Pure lease observations: callers own time checks. A Clock callback here could revoke
        // authority after the enclosing engine fence checked it.
        if state.closed || !self.live.is_live() || !self.binding.live() {
            return Err(VoiceError::Unauthorized);
        }
        let credit = self.charge_locked(resources, group, bytes, kind)?;
        drop(state);
        Ok(credit.with_custody(self.clone()))
    }
    // Caller owns the workflow state fence and has checked original leases. Do not call
    // charge() recursively while holding that fence (notably from handoff admission).
    fn charge_locked(
        &self,
        resources: &Arc<Resources>,
        group: [u8; 32],
        bytes: usize,
        kind: ResourceKind,
    ) -> Result<Credit, VoiceError> {
        let credit = if kind == ResourceKind::Forward {
            let mut result = Err(VoiceError::Capacity);
            for lane in &self.lowers {
                result = ByteBudget::ReservedLane {
                    reservation: self.reserve.clone(),
                    lane: lane.clone(),
                }
                .charge(resources, group, bytes, kind);
                if !matches!(result, Err(VoiceError::Capacity)) {
                    break;
                }
            }
            result?
        } else if matches!(kind, ResourceKind::Crypto | ResourceKind::Retained) {
            ByteBudget::ReservedLane {
                reservation: self.reserve.clone(),
                lane: self.crypto.clone(),
            }
            .charge(resources, group, bytes, kind)?
        } else {
            return Err(VoiceError::Scope);
        };
        Ok(credit)
    }
    pub(super) fn borrow_handoff(
        self: &Arc<Self>,
        bytes: usize,
    ) -> Result<HandoffLoan, VoiceError> {
        if bytes == 0 || bytes > VARIABLE_BYTES {
            return Err(VoiceError::Bounds);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || !self.live.is_live() || !self.binding.live() {
            return Err(VoiceError::Unauthorized);
        }
        if state.handoff_busy {
            return Err(VoiceError::Capacity);
        }
        let credit = self.charge_locked(
            &self.resources,
            self.binding.scope.group,
            bytes,
            ResourceKind::Retained,
        )?;
        // A failed capacity claim leaves the position available for the exact original retry.
        state.handoff_busy = true;
        drop(state);
        Ok(HandoffLoan {
            workflow: self.clone(),
            _credit: credit.with_custody(self.clone()),
        })
    }
    pub(crate) fn collect_expired(&self, mono: u64) {
        if mono >= self.binding.deadline || !self.binding.live() || !self.live.is_live() {
            if let Some(control) = self.control.upgrade() {
                control.close();
            } else {
                self.close();
            }
        }
    }
    fn close(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        self.reserve.close();
        self.crypto.close();
        for lane in &self.lowers {
            lane.close();
        }
        self.packets.close();
    }
}
impl Drop for Workflow {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests;
