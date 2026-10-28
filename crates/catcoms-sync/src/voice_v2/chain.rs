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
use super::resources::{
    ByteBudget, Credit, GroupForwardLanes, GroupProgressClaim, ProgressFunding, Reservation,
    ResourceKind, Resources, WorkLane,
};
use super::{CallLease, DeviceId, Nonce, Scope, VoiceError};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

pub(super) const ROW_BYTES: usize = 1536;
// LOW Ready uses no action position. Its plain history fee also covers a conservative
// complete shared-map node allowance, including internal edges. Two unused byte rows
// remain after ten confirmation rows; the two SDP slots get record bytes from Exchange.
pub(in crate::voice_v2) const LOW_READY_RECEIPT_BYTES: usize = ROW_BYTES + 64;
const ROWS: usize = 12;
const NO_ROW: u8 = u8::MAX;
mod admission;
mod capacity;
pub use capacity::ChainCapacity;
mod derivation;
mod established;
/// Separately prepaid native SDP storage; no SDK or protocol permission by itself.
pub mod sdp_handoff;
pub use established::EstablishedKeyCustody;
mod history;
mod lifecycle;
mod outgoing;
mod ready;
mod start;
mod transfer;
mod transition;
pub(crate) use admission::ConfirmationAdmission;
pub(super) use admission::ConfirmationConsent;
pub(in crate::voice_v2) use history::ActionHistoryCustody;
pub use history::PlanActionHistory;
pub use lifecycle::PlanRetirement;
pub(in crate::voice_v2) use lifecycle::SuccessorCustody;
pub(in crate::voice_v2) use start::{StartAdmission, StartCandidate};
pub(in crate::voice_v2) use transfer::{OriginalPlanObservation, SuccessorProof, TransferredParts};
pub use transfer::{PreparedSdpCapacity, TransferredSdpCapacity};
pub(in crate::voice_v2) use transition::AWAIT_READY_MS;
pub use transition::{ConfirmationCustody, FinishEvidence};
pub(in crate::voice_v2) use transition::{FinishIdentity, PreparedFinishWindow};
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
    transition: transition::FinishWindow,
    // Original fixed history already owns this paid allocation without owning Workflow.
    // Only Workflow's LAST field publisher can establish variable-capacity retirement.
    workflow_drained: AtomicBool,
    // Distinct from old-Workflow retirement: a prepared/ticket/Exchange can survive it.
    successor_closed: AtomicBool,
    // Whole-plan retirement additionally makes old confirmation retry CACHE obsolete.
    // Key-only successor closure has a narrower contract and MUST NOT set this latch.
    whole_closed: AtomicBool,
    successor_busy: AtomicBool,
    // Set only by exact authenticated LOW Ready publication. Local Finish/ACK/Established
    // never imply remote readiness. This stops old Source work without closing Await.
    low_ready: AtomicBool,
    // Native certifies terminal history only AFTER its exact Plan-final ACK and actual
    // row destruction. This request never grants an effect or resets an issued position.
    history_requested: AtomicBool,
    // Actual independently funded records, including result/delivery observers which
    // outlive their Action. The last-field publisher runs after their fixed fee returns.
    history_records: AtomicUsize,
    _metadata: Credit,
}
impl PlanBinding {
    fn live(&self) -> bool {
        self.value.live() && self.transition.is_live()
    }
    // The original fixed payment already retains this exact pool. Borrow it rather than
    // adding duplicate pool owners to HistoryPlan and Workflow inside the 2KiB allowance.
    pub(super) fn resources(&self) -> &Arc<Resources> {
        self._metadata.resources()
    }
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
    binding: Arc<PlanBinding>,
    live: CallLease,
    control: Weak<PreparedChain>,
    reserve: Arc<Reservation>,
    state: Mutex<WorkflowState>,
    // MUST remain last. Arc's weak strong_count reaches0 BEFORE Drop/field destruction;
    // neither that count nor Workflow::close proves its actual capacity fields returned.
    // None occurs only when a deterministic test parks this exact original publisher.
    _retirement: Option<RetirementPublisher>,
}
struct RetirementPublisher {
    // With the ORIGINAL publisher, including a parked actual-last-field test. A
    // successor shares this claim; fixed history/observers deliberately do not retain it.
    progress: GroupProgressClaim,
    // The original2KiB payment survives the claim's potentially blocking final release.
    binding: Arc<PlanBinding>,
}
impl Drop for RetirementPublisher {
    fn drop(&mut self) {
        // All preceding Workflow fields have actually dropped. Credits return reservation
        // bytes/lane ownership before their custody can trigger Workflow's last drop. Keep
        // the original fixed metadata paid through this publication and every observer.
        self.binding.workflow_drained.store(true, Ordering::Release);
    }
}

/// Observation of the ORIGINAL retired variable confirmation Workflow. This retains only
/// its already-paid fixed binding, never Workflow, an engine, key, action or effect authority.
/// It proves that Workflow's variable capacity owners have returned, not that fixed history
/// metadata or independently retained packet-table slots disappeared. No constructor by ID,
/// Clone, serialization or conversion back to a grant/control owner exists.
pub struct ConfirmationRetirement {
    binding: Arc<PlanBinding>,
}
impl ConfirmationRetirement {
    /// Caller-owned witness storage, NOT a wire-body size or a new reserve. The native
    /// adapter must also count its enclosing state/task/output in already-paid control memory.
    /// The flag/publisher remain in the original separately paid2KiB retained heap graph.
    pub const OBSERVER_MEMORY_BYTES: usize = std::mem::size_of::<usize>();

    /// True only after this ORIGINAL variable Workflow's fields and credited owners retired.
    /// Fixed history metadata and independently retained packet-table slots are separate.
    /// This is a monotonic snapshot only: no waiter, wakeup or scheduling guarantee exists.
    /// Retain the SAME witness across cancelled work; a missing witness never proves drainage.
    pub fn is_drained(&self) -> bool {
        self.binding.workflow_drained.load(Ordering::Acquire)
    }
}
redacted_debug!(ConfirmationRetirement);

/// Native-only authority to close one ORIGINAL confirmation plan between handoff phases.
/// Unlike a HandoffLoan this owns no variable capacity, input or engine. In particular, an
/// ordinary loan Return followed by End must not leave native cleanup dependent on a future
/// child action being published. Capture this capability before returning the first loan.
///
/// The weak control allocation remains covered by the SAME fixed PlanBinding payment, which
/// outlives it by field order. No handshake lookup, current grant or replacement plan is used.
pub struct ConfirmationControl {
    control: Weak<PreparedChain>,
    binding: Arc<PlanBinding>,
}
redacted_debug!(ConfirmationControl);
impl ConfirmationControl {
    /// Observe only this plan's actual variable-capacity retirement. This samples no Clock,
    /// takes no lock and grants no current authority. Weak control loss alone never sets it.
    pub fn is_drained(&self) -> bool {
        self.binding.workflow_drained.load(Ordering::Acquire)
    }
    /// Permanently close the exact captured plan and retain its original last-field observer.
    /// Call outside Engine/Group/Policy/native registry locks; closing retires unstarted
    /// actions. This proves closure only: actual input/lower/SDK owners must still drain.
    /// Browser keys and RTC readiness are separate and this grants no SDP authority.
    pub fn retire(self) -> ConfirmationRetirement {
        let retirement = ConfirmationRetirement {
            binding: self.binding.clone(),
        };
        if let Some(control) = self.control.upgrade() {
            control.close();
        }
        // If the exact control has already reached strong-zero, PreparedChain::Drop closes
        // it. Do not infer completion from weak-zero: the SAME Workflow publisher below may
        // still be pending, and the returned observer remains false until it actually runs.
        drop(self);
        retirement
    }
}
/// The sole close right follows the actual phase, including an abandoned extracted ticket.
/// Arc reservation observations alone cannot close or refund another live phase's backing.
/// Keep this owner before its enclosing retirement publisher in every retaining graph.
pub(in crate::voice_v2) struct SdpCapacityClosing {
    core: Arc<Reservation>,
    ready: Arc<super::ready::capacity::ReadyCapacity>,
}
impl Drop for SdpCapacityClosing {
    fn drop(&mut self) {
        self.core.close();
        self.ready.close();
    }
}
struct ProgressCapacity {
    // Declaration order closes lending first; the direct unused escrow then returns to
    // the closed core. Actual old loans independently keep only their real debt paid.
    closing: SdpCapacityClosing,
    escrow: Credit,
}
struct ProgressCapacityOwner {
    body: Option<Box<ProgressCapacity>>,
    // OUTSIDE the Box and last: its existing4KiB fee survives body/escrow destruction
    // AND actual Box deallocation. An anchor inside the Box would return too early.
    _metadata: Arc<super::ready::capacity::ReadyCapacity>,
}
impl ProgressCapacityOwner {
    fn new(closing: SdpCapacityClosing, escrow: Credit) -> Self {
        Self {
            // Evaluate this clone before moving closing or allocating the paid body.
            _metadata: closing.ready.clone(),
            body: Some(Box::new(ProgressCapacity { closing, escrow })),
        }
    }
    fn capacity(&self) -> &ProgressCapacity {
        self.body.as_deref().expect("original progress body")
    }
    fn into_parts(mut self) -> ProgressCapacity {
        // Actual deallocation precedes this outer metadata anchor's destruction. The
        // returned closing owner additionally retains Ready through its next affine move.
        *self.body.take().expect("one original progress body")
    }
}
impl Drop for ProgressCapacityOwner {
    fn drop(&mut self) {
        drop(self.body.take()); // Full body + deallocation while _metadata is still paid.
    }
}
#[cfg(test)]
pub(in crate::voice_v2) fn progress_capacity_metadata_bytes() -> usize {
    // The same original Ready fee also covers an inert carrier and conservative overlap
    // while it is decomposed into the eventual paid Root graph.
    std::mem::size_of::<ProgressCapacity>() + ChainCapacity::metadata_bytes()
}
struct WorkflowState {
    closed: bool,
    // None after ordinary closure OR actual affine transfer, never after preparation.
    // This is the actual close-right owner, rather than an optimistic transferred flag.
    progress: Option<ProgressCapacityOwner>,
    // Closing rights move with the lane. Actual old packet rows may occupy every original
    // claim; returning them after extraction must feed the SAME successor reservation.
    packets: Option<super::packets::ConfirmationCapacity>,
    // Taken affinely after freezing this phase. Old close/Drop may retire its reserve
    // and Lower custody, but must never close a lane already owned by the SDP transition.
    crypto: Option<Arc<WorkLane>>,
    // Close takes ONLY this phase's handle. Actual Lower credits and other live phases
    // retain the same group owner; idle inputs/history must not pin process work slots.
    lowers: Option<Arc<GroupForwardLanes>>,
    // One IPC owner for the entire chain, including distinct rows. A lost renderer reply may
    // keep this occupied while the original physical work/result/cleanup still drains.
    handoff_busy: bool,
    // At most one affine successor can retain this plan's prepaid continuation claims.
    // Failed/abandoned preparation clears this latch; transfer never reopens the old phase.
    sdp_preparing: bool,
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
        drop(self.retire_confirmation_observed());
    }

    /// Same irreversible original-plan closure, with a fixed observer for actual VARIABLE
    /// Workflow retirement. First drain selected browser/provider/RPC work and settle its
    /// engine; call outside native/Engine/Group/Policy guards. Returning the observer proves
    /// closure only; delayed input/lower/copy owners may still keep is_drained false.
    /// Successful Derive->Hello/Proof/Finish continuation must use ordinary loan Drop instead:
    /// this closes the entire confirmation plan and is not SDP phase transfer or RTC readiness.
    pub fn retire_confirmation_observed(self) -> ConfirmationRetirement {
        let retired = ConfirmationRetirement {
            binding: self.workflow.binding.clone(),
        };
        if let Some(control) = self.workflow.control.upgrade() {
            control.close();
        } else {
            // The exact control was already destroyed; the loan still owns its original
            // Workflow. It must not become an open reserve just because its control is gone.
            self.workflow.close();
        }
        drop(self);
        retired
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
    /// Native publication carries this original role position into the immutable action.
    /// It is not a renderer-provided slot and remains meaningful after payload compaction.
    pub(super) fn row_index(&self) -> u8 {
        u8::try_from(self.row).expect("protected history has twelve fixed rows")
    }

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
        if Arc::ptr_eq(resources, self.plan.binding.resources())
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
            self.binding.resources(),
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
        let capacity = ChainCapacity::reserve(
            resources.clone(),
            binding.scope.group,
            binding.low,
            slots.clone(),
            packet_slots,
        )?;
        Self::from_capacity(resources, binding, slots, packet_slots, capacity)
    }

    /// Trusted native consumption reborrows the CURRENT exact pools. A held capacity
    /// from a replaced join is not transferable by matching group/role labels.
    pub(super) fn from_capacity(
        resources: Arc<Resources>,
        binding: Binding,
        slots: Arc<slots::Slots>,
        packet_slots: &super::packets::PacketSlots,
        capacity: ChainCapacity,
    ) -> Result<Arc<Self>, VoiceError> {
        capacity.into_chain(&resources, binding, &slots, packet_slots)
    }
    /// End/expiry closes pending effects and returns only unused capacity. Existing engines
    /// and input readers remain charged through actual settlement/last drop, even after End.
    pub fn close(&self) {
        // Claim irreversible closure BEFORE waiting for Control. A Finish publication can
        // otherwise arm a new window after an expiry collector already decided to close it.
        self.binding.transition.close();
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
            && Arc::ptr_eq(resources, self.history.binding.resources())
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
    /// Pure registry validation under Pool; no Workflow/control/resource lock may be
    /// acquired by this check. Only the original paid publisher owns this capacity.
    pub(in crate::voice_v2) fn progress_matches(
        &self,
        resources: &Resources,
        group: [u8; 32],
        key: usize,
    ) -> bool {
        self._retirement
            .as_ref()
            .is_some_and(|publisher| publisher.progress.matches(resources, group, key))
    }

    pub(crate) fn uses_resources(&self, resources: &Arc<Resources>, group: [u8; 32]) -> bool {
        Arc::ptr_eq(resources, self.binding.resources()) && group == self.binding.scope.group
    }
    pub(crate) fn is_live(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        !state.closed && self.live.is_live() && self.binding.live()
    }
    pub(super) fn confirmation_control(&self) -> ConfirmationControl {
        ConfirmationControl {
            control: self.control.clone(),
            binding: self.binding.clone(),
        }
    }
    pub(crate) fn charge(
        self: &Arc<Self>,
        resources: &Arc<Resources>,
        group: [u8; 32],
        bytes: usize,
        kind: ResourceKind,
    ) -> Result<Credit, VoiceError> {
        if !Arc::ptr_eq(resources, self.binding.resources()) || group != self.binding.scope.group {
            return Err(VoiceError::Scope);
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Pure lease observations: callers own time checks. A Clock callback here could revoke
        // authority after the enclosing engine fence checked it.
        if state.closed || !self.live.is_live() || !self.binding.live() {
            return Err(VoiceError::Unauthorized);
        }
        let credit = self.charge_locked(&state, resources, group, bytes, kind)?;
        drop(state);
        Ok(credit.with_custody(self.clone()))
    }
    // Caller owns the workflow state fence and has checked original leases. Do not call
    // charge() recursively while holding that fence (notably from handoff admission).
    fn charge_locked(
        &self,
        state: &WorkflowState,
        resources: &Arc<Resources>,
        group: [u8; 32],
        bytes: usize,
        kind: ResourceKind,
    ) -> Result<Credit, VoiceError> {
        let credit = if kind == ResourceKind::Forward {
            state
                .lowers
                .as_ref()
                .ok_or(VoiceError::Unauthorized)?
                .charge(resources, group, &self.reserve, bytes)?
        } else if matches!(kind, ResourceKind::Crypto | ResourceKind::Retained) {
            ByteBudget::ReservedLane {
                reservation: self.reserve.clone(),
                lane: state
                    .crypto
                    .as_ref()
                    .ok_or(VoiceError::Unauthorized)?
                    .clone(),
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
            &state,
            self.binding.resources(),
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
        if let Some(control) = self.control.upgrade() {
            control.collect_expired(mono);
        } else {
            self.binding.transition.close();
            self.close();
        }
    }
    fn close(&self) {
        let (lowers, crypto, packets, progress) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            (
                state.lowers.take(),
                state.crypto.take(),
                state.packets.take(),
                state.progress.take(),
            )
        };
        // An extracted ticket now owns the SAME core/Ready. Repeated old close/Drop may
        // not close it. Before transfer, this actual owner closes unused bytes outside
        // WorkflowState, then returns escrow; genuine old borrowers retain their debt.
        drop(progress);
        if let Some(crypto) = crypto {
            crypto.close();
        }
        // Outgoing lanes belong to the original group, shared with other native phases.
        // Release this phase's handle outside its state fence. A real Lower or another live
        // phase retains the shared owner, while closed idle inputs no longer pin slots.
        drop(lowers);
        if let Some(packets) = packets {
            packets.close();
        }
    }
}
impl Drop for Workflow {
    fn drop(&mut self) {
        let original = self as *const Self;
        self.close();
        self.binding
            .resources()
            .unregister_workflow(self.binding.scope.group, original);
    }
}

#[cfg(test)]
mod tests;
