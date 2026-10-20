//! Inactive native confirmation capacity. This is NOT whole-chain readiness: protected native
//! and browser registry rows, atomic encrypted intake, helper agreement and measured SDP/IPC
//! phase transfer must still compose before live dispatch. Packet-table claims also protect the
//! actual pair role's three confirmation candidates, but only derivation consumes a row here.
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
struct PlanBinding {
    value: Binding,
    _metadata: Credit,
}
impl std::ops::Deref for PlanBinding {
    type Target = Binding;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

#[derive(Clone, Copy)]
enum RowState {
    Available,
    Prepared(u64),
    Issued,
}
struct HistoryState {
    closed: bool,
    serial: u64,
    derive: RowState,
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
    closed: Mutex<bool>,
}
struct Control {
    closed: bool,
    workflow: Option<Arc<Workflow>>,
    actions: Vec<Weak<CryptoAction>>,
}

/// Opaque inactive native plan. Twelve local rows are protected (derive, three flights for
/// each of three candidates, two SDP directions); only derive has a consumer in this slice.
/// This grants no network, engine, browser or RTC readiness. Close is irreversible.
pub struct PreparedChain {
    binding: Arc<PlanBinding>,
    live: CallLease,
    history: Arc<HistoryPlan>,
    control: Mutex<Control>,
}
redacted_debug!(Workflow, PreparedChain);

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

    /// Allocation, tagging, input construction and authority validation precede this boundary.
    /// The closure only inserts the prepared action. Close serializes here; after issuance
    /// there is no recoverable tail that could require a replacement row.
    pub(super) fn publish<T>(mut self, apply: impl FnOnce() -> T) -> Result<T, VoiceError> {
        let mut state = self.plan.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed
            || !self.plan.live.is_live()
            || !self.plan.binding.live()
            || !matches!(state.derive, RowState::Prepared(ticket) if ticket == self.ticket)
        {
            return Err(VoiceError::Unauthorized);
        }
        state.derive = RowState::Issued;
        self.unpublished = false;
        Ok(apply())
    }
}
impl Drop for RowIssue {
    fn drop(&mut self) {
        if self.unpublished {
            let mut state = self.plan.state.lock().unwrap_or_else(|e| e.into_inner());
            if !state.closed
                && matches!(state.derive, RowState::Prepared(ticket) if ticket == self.ticket)
            {
                state.derive = RowState::Available;
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
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || !self.live.is_live() || !self.binding.live() {
            return Err(VoiceError::Unauthorized);
        }
        if !matches!(state.derive, RowState::Available) {
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
        state.derive = RowState::Prepared(ticket);
        Ok(PreparedHistory {
            record,
            slot,
            issue: RowIssue {
                plan: self.clone(),
                workflow: Arc::downgrade(workflow),
                key,
                ticket,
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
                derive: RowState::Available,
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
                actions: Vec::with_capacity(ROWS),
                workflow: Some(Arc::new(Workflow {
                    resources: resources.clone(),
                    binding,
                    live,
                    control: control.clone(),
                    reserve,
                    crypto,
                    lowers,
                    packets,
                    closed: Mutex::new(false),
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
                std::mem::take(&mut control.actions),
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
                assert!(
                    control.actions.len() < ROWS,
                    "bounded prepared chain actions"
                );
                control.actions.push(Arc::downgrade(action));
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
        let closed = self.closed.lock().unwrap_or_else(|e| e.into_inner());
        // Pure lease observations: callers own time checks. A Clock callback here could revoke
        // authority after the enclosing engine fence checked it.
        if *closed || !self.live.is_live() || !self.binding.live() {
            return Err(VoiceError::Unauthorized);
        }
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
        drop(closed);
        Ok(credit.with_custody(self.clone()))
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
        *self.closed.lock().unwrap_or_else(|e| e.into_inner()) = true;
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
