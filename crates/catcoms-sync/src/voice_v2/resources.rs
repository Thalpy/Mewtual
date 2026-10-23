//! A single native-process budget shared by endpoint caches, helper queues and engine permits.
//! Credits are affine Rust owners: only actual completion/expiry may drop them. Revocation,
//! timeout, a lost IPC waiter and controller replacement do not themselves release a credit.

use super::VoiceError;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Aggregate declared native memory across ALL groups and process infrastructure. A local
/// group policy revision never grants another process pool or increases this hard ceiling.
pub const PROCESS_BYTES: usize = 2 * 1024 * 1024;
/// Declared native group capacity. Actual authenticated discovery, retained transport work,
/// channel/setup custody and the SDP phase must compose; the earlier provisional256KiB
/// budget could refuse an ordinary two-endpoint call before SDP. One group may now occupy
/// at most one quarter of the UNCHANGED2MiB process pool. Work/issuer/record/peer ceilings and
/// actual-last-owner refunds remain separate, unchanged enforcement boundaries.
pub const GROUP_BYTES: usize = 512 * 1024;
const GROUP_RECORDS: usize = 1024;
const SLOT_KINDS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResourceKind {
    Retained,
    Crypto,
    Watcher,
    Challenge,
    Presence,
    Helper,
    Tombstone,
    Origin,
    Forward,
}
impl ResourceKind {
    fn slot(self) -> Option<(usize, usize, usize)> {
        match self {
            Self::Retained => None,
            Self::Crypto => Some((0, 4, 8)),
            Self::Watcher => Some((1, 8, 8 * GROUP_RECORDS)),
            Self::Challenge => Some((2, 32, 64)),
            Self::Presence => Some((3, 64, 512)),
            Self::Helper => Some((4, 256, 1024)),
            Self::Tombstone => Some((5, 256, 1024)),
            Self::Origin => Some((6, 64, 512)),
            Self::Forward => Some((7, 4, 8)),
        }
    }
}
#[derive(Default)]
struct Counts {
    bytes: usize,
    slots: [usize; SLOT_KINDS],
    exchange: std::sync::Weak<super::negotiation::Exchange>,
    workflow: std::sync::Weak<super::chain::Workflow>,
}
#[derive(Default)]
struct State {
    total: Counts,
    groups: HashMap<[u8; 32], Counts>,
}

/// Native creates one pool for every actor/controller in its process. Creating a pool per call
/// would multiply the limits; the native wiring must share this exact owner with helper actors.
#[derive(Default)]
pub struct Resources {
    state: Mutex<State>,
}
redacted_debug!(Resources, Credit, ProcessRetainedCredit);

/// Fixed native process infrastructure charged against the same process byte ceiling as groups.
///
/// This is deliberately a different type from `Credit`: a listener's fixed router scratch/task
/// has no MLS group, and inventing a synthetic group would consume or evade real group records.
/// It cannot become a group budget, reservation, work lane or crypto/forwarding permit. Use
/// ordinary group credits for every group/member-scoped payload and for bootstrap computation.
/// The trusted adapter must include this owner's metadata and actual infrastructure peak in its
/// charge, acquire it before allocation/bind, and retain it until task/output custody drains.
/// Sharing an `Arc` of this owner does not duplicate capacity or grant protocol authority.
pub struct ProcessRetainedCredit {
    pool: Arc<Resources>,
    bytes: usize,
}

impl Drop for ProcessRetainedCredit {
    fn drop(&mut self) {
        let mut state = self
            .pool
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Only admission and this sole affine destructor change this charge. Group retirement
        // can remove all group records while this fixed process infrastructure is still live.
        state.total.bytes -= self.bytes;
    }
}

/// An exact non-cloneable charge. Keep it in a pending-or-uncertain operation/cache record.
pub struct Credit {
    pool: Arc<Resources>,
    group: [u8; 32],
    bytes: usize,
    kind: ResourceKind,
    // A loan returns to its already charged reserve only after its actual owner releases it.
    // Once closed, the reserve instead releases that loan's part of the backing charge.
    reservation: Option<Arc<Reservation>>,
    // A prepaid lane is counted once while idle or borrowed. This guard serializes actual
    // verifier/scratch/engine work without taking a second process Crypto slot.
    lane: Option<LaneBorrow>,
    // Optional native phase/count owner. Its guard retires only with the actual byte credit.
    custody: Option<catcoms_rt::SharedRequestKeepalive>,
    // A native preparation may partition one retained input from its fixed metadata exactly
    // once. This is not a general allocator: tiny repeated partitions could multiply otherwise
    // uncharged owner/control-block metadata even while their byte sum remained bounded.
    partitioned: bool,
}

/// One prepaid exclusive Crypto/Forward slot for a bounded progress workflow. This is capacity only,
/// not authority to verify, decrypt or publish. Actual borrowers retain the slot after close.
pub struct WorkLane {
    pool: Arc<Resources>,
    group: [u8; 32],
    kind: ResourceKind,
    state: Mutex<LaneState>,
}
struct LaneState {
    credit: Option<Credit>,
    busy: bool,
    closed: bool,
}
struct LaneBorrow(Arc<WorkLane>);
redacted_debug!(WorkLane);
impl WorkLane {
    /// Retirement releases an idle lane. An actual borrower remains charged until its last
    /// credit owner drops; a timeout or lost result-RPC cannot make this lane reusable.
    pub fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        if !state.busy {
            state.credit.take();
        }
    }
    fn borrow(
        self: &Arc<Self>,
        pool: &Arc<Resources>,
        group: [u8; 32],
    ) -> Result<LaneBorrow, VoiceError> {
        if !Arc::ptr_eq(pool, &self.pool) || group != self.group {
            return Err(VoiceError::Scope);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || state.busy {
            return Err(VoiceError::Capacity);
        }
        state.busy = true;
        Ok(LaneBorrow(self.clone()))
    }
}
impl Drop for LaneBorrow {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(state.busy, "one actual work lane borrower");
        state.busy = false;
        if state.closed {
            state.credit.take();
        }
    }
}

/// Prepaid progress bytes for one bounded native workflow. Dropping the caller's Arc retires no
/// outstanding custody: every loan keeps the original charge alive. This owner grants capacity
/// only; it grants no call, membership, packet, crypto or forwarding authority.
pub struct Reservation {
    pool: Arc<Resources>,
    group: [u8; 32],
    state: Mutex<ReservationState>,
}
struct ReservationState {
    credit: Credit,
    available: usize,
    closed: bool,
}
redacted_debug!(Reservation);

/// Explicit allocation choice at a trusted native factory. Passing the expected process owner
/// at every charge prevents a reservation from a second same-group pool bypassing global limits.
/// A reserved budget is capacity only and must never substitute for the factory's authority.
#[derive(Clone, Default)]
pub enum ByteBudget {
    #[default]
    Ordinary,
    Reserved(Arc<Reservation>),
    /// Inactive native confirmation plan. Only native factories can obtain this owner; fixed
    /// action history must use its separately prepared rows, never this variable budget.
    Workflow(Arc<super::chain::Workflow>),
    Exchange {
        exchange: Arc<super::negotiation::Exchange>,
        stage: super::negotiation::Stage,
    },
    /// Private native workflow factories must pair this with their phase/count/identity gate.
    /// The lane is acquired synchronously; busy work refuses without waiting on an actor.
    ReservedLane {
        reservation: Arc<Reservation>,
        lane: Arc<WorkLane>,
    },
}
redacted_debug!(ByteBudget);
impl ByteBudget {
    pub fn charge(
        &self,
        resources: &Arc<Resources>,
        group: [u8; 32],
        bytes: usize,
        kind: ResourceKind,
    ) -> Result<Credit, VoiceError> {
        match self {
            Self::Ordinary => resources.charge(group, bytes, kind),
            Self::Workflow(workflow) => workflow.charge(resources, group, bytes, kind),
            Self::Exchange { exchange, stage } => {
                if !exchange.uses_resources(resources, group) {
                    return Err(VoiceError::Scope);
                }
                let stage = if kind == ResourceKind::Crypto {
                    super::negotiation::Stage::Crypto
                } else {
                    *stage
                };
                if stage.resource_kind() != kind {
                    return Err(VoiceError::Scope);
                }
                exchange.charge(stage, bytes)
            }
            Self::Reserved(reservation) => {
                if !Arc::ptr_eq(resources, &reservation.pool) {
                    return Err(VoiceError::Scope);
                }
                reservation.charge(group, bytes, kind)
            }
            Self::ReservedLane { reservation, lane } => {
                if !Arc::ptr_eq(resources, &reservation.pool) {
                    return Err(VoiceError::Scope);
                }
                if matches!(kind, ResourceKind::Crypto | ResourceKind::Forward) {
                    if kind != lane.kind {
                        return Err(VoiceError::Scope);
                    }
                    let borrow = lane.borrow(resources, group)?;
                    // The prepaid lane owns the Crypto slot. Bytes come from the same reserve,
                    // with no nested Crypto acquisition while the single lane is occupied.
                    let mut credit = reservation.charge(group, bytes, ResourceKind::Retained)?;
                    credit.lane = Some(borrow);
                    Ok(credit)
                } else {
                    reservation.charge(group, bytes, kind)
                }
            }
        }
    }
}

impl Resources {
    /// Prepay fixed native process infrastructure without allocating a group or a work slot.
    ///
    /// Zero, overflow and capacity refusal leave all counters unchanged. There is no early
    /// release, partition or conversion API: cancellation, timeout and observer loss must not
    /// refund an actual listener/worker/completed output that still retains this owner.
    pub fn charge_process_retained(
        self: &Arc<Self>,
        bytes: usize,
    ) -> Result<ProcessRetainedCredit, VoiceError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        // Check the operand before subtraction so even usize::MAX is an ordinary refusal.
        if bytes == 0 || bytes > PROCESS_BYTES || state.total.bytes > PROCESS_BYTES - bytes {
            return Err(VoiceError::Capacity);
        }
        state.total.bytes += bytes;
        Ok(ProcessRetainedCredit {
            pool: self.clone(),
            bytes,
        })
    }

    /// One progress owner per process/group, including retired actual input/engine/lower loans.
    /// Do not upgrade/drop an old owner under the pool lock: its destructor reenters this pool.
    pub(crate) fn install_workflow(
        &self,
        group: [u8; 32],
        workflow: &Arc<super::chain::Workflow>,
    ) -> Result<(), VoiceError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let counts = state
            .groups
            .get_mut(&group)
            .expect("prepaid workflow group");
        if counts.workflow.strong_count() != 0 {
            return Err(VoiceError::Capacity);
        }
        counts.workflow = Arc::downgrade(workflow);
        Ok(())
    }
    /// Quiet process housekeeping needs no active UI/actor grant. Clone bounded weak indices
    /// under the pool lock, then enter control/action/reservation locks only after releasing it.
    pub(crate) fn collect_workflows(&self, mono: u64) {
        let workflows: Vec<_> = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .groups
            .values()
            .map(|counts| counts.workflow.clone())
            .collect();
        for workflow in workflows
            .into_iter()
            .filter_map(|workflow| workflow.upgrade())
        {
            workflow.collect_expired(mono);
        }
    }
    pub(crate) fn register_exchange(
        &self,
        group: [u8; 32],
        exchange: &Arc<super::negotiation::Exchange>,
    ) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // The prepaid backing/metadata keeps this group present. A Weak capacity index neither
        // prolongs a call nor resets its limits, and disappears with the bounded group record.
        state
            .groups
            .get_mut(&group)
            .expect("prepaid exchange group")
            .exchange = Arc::downgrade(exchange);
    }
    pub(crate) fn exchange(&self, group: [u8; 32]) -> Option<Arc<super::negotiation::Exchange>> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .groups
            .get(&group)?
            .exchange
            .upgrade()
    }
    pub fn charge(
        self: &Arc<Self>,
        group: [u8; 32],
        bytes: usize,
        kind: ResourceKind,
    ) -> Result<Credit, VoiceError> {
        self.charge_inner(group, bytes, kind, false)
    }
    fn charge_inner(
        self: &Arc<Self>,
        group: [u8; 32],
        bytes: usize,
        kind: ResourceKind,
        prepaid_slot: bool,
    ) -> Result<Credit, VoiceError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let counts = state.groups.get(&group);
        if (kind != ResourceKind::Crypto && bytes == 0 && !prepaid_slot)
            || (counts.is_none() && state.groups.len() >= GROUP_RECORDS)
            || bytes > GROUP_BYTES
            || state.total.bytes > PROCESS_BYTES - bytes
            || counts.is_some_and(|counts| counts.bytes > GROUP_BYTES - bytes)
            || kind.slot().is_some_and(|(slot, group_max, process_max)| {
                state.total.slots[slot] >= process_max
                    || counts.is_some_and(|counts| counts.slots[slot] >= group_max)
            })
        {
            return Err(VoiceError::Capacity);
        }
        state.total.bytes += bytes;
        if let Some((slot, _, _)) = kind.slot() {
            state.total.slots[slot] += 1;
        }
        let counts = state.groups.entry(group).or_default();
        counts.bytes += bytes;
        if let Some((slot, _, _)) = kind.slot() {
            counts.slots[slot] += 1;
        }
        Ok(Credit {
            pool: self.clone(),
            group,
            bytes,
            kind,
            reservation: None,
            lane: None,
            custody: None,
            partitioned: false,
        })
    }

    /// Reserve an entire workflow's conservative peak before acknowledging its first input.
    /// Every covered stage must borrow from this owner rather than charge ordinary bytes again.
    /// The caller must also reserve bounded continuation/result slots before irreversible work.
    pub fn reserve(
        self: &Arc<Self>,
        group: [u8; 32],
        bytes: usize,
    ) -> Result<Arc<Reservation>, VoiceError> {
        let credit = self.charge(group, bytes, ResourceKind::Retained)?;
        Ok(Arc::new(Reservation {
            pool: self.clone(),
            group,
            state: Mutex::new(ReservationState {
                credit,
                available: bytes,
                closed: false,
            }),
        }))
    }
    /// Prepay one slot from the existing four-per-group/eight-per-process Crypto limits.
    /// Ordinary reservations still acquire slots only when work starts. A progress owner must
    /// close this exceptional lane on retirement and lend it serially to all covered stages.
    pub fn reserve_crypto_lane(
        self: &Arc<Self>,
        group: [u8; 32],
    ) -> Result<Arc<WorkLane>, VoiceError> {
        self.reserve_work_lane(group, ResourceKind::Crypto)
    }
    /// Prepay one of the existing four-per-group/eight-per-process lower-flight slots. At most
    /// two such lanes belong to a narrow SDP exchange; ordinary helper work shares the same caps.
    pub fn reserve_forward_lane(
        self: &Arc<Self>,
        group: [u8; 32],
    ) -> Result<Arc<WorkLane>, VoiceError> {
        self.reserve_work_lane(group, ResourceKind::Forward)
    }
    fn reserve_work_lane(
        self: &Arc<Self>,
        group: [u8; 32],
        kind: ResourceKind,
    ) -> Result<Arc<WorkLane>, VoiceError> {
        let credit = self.charge_inner(group, 0, kind, true)?;
        Ok(Arc::new(WorkLane {
            pool: self.clone(),
            group,
            kind,
            state: Mutex::new(LaneState {
                credit: Some(credit),
                busy: false,
                closed: false,
            }),
        }))
    }
    /// Aggregate diagnostics only; no stable group identifiers or payloads leave the pool.
    /// Bytes include fixed process infrastructure as well as all group-scoped owners.
    pub fn counts(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        (state.total.bytes, state.total.slots[0])
    }
    /// Snapshot for trusted native composition diagnostics: original group's bytes/Crypto
    /// positions, excluding process-only infrastructure. Missing means no charged group at
    /// this observation, never a drained action or permission to reserve observed headroom.
    /// Actual charge/admission must still repeat its atomic checks under this pool's fence.
    pub fn group_counts(&self, group: [u8; 32]) -> Option<(usize, usize)> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state
            .groups
            .get(&group)
            .map(|counts| (counts.bytes, counts.slots[0]))
    }
}

impl Reservation {
    /// Permanently stop lending and release only the unused capacity. Outstanding copy, engine
    /// and lower-work owners retain their own bytes/slots until actual settlement. This is a
    /// capacity operation, never evidence that a call/action/workflow has safely completed.
    /// A native workflow gate must still count unresolved work after its reserve is closed.
    pub fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return;
        }
        state.closed = true;
        let unused = std::mem::take(&mut state.available);
        state.credit.release_retained_bytes(unused);
    }

    fn return_loan(&self, bytes: usize) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed {
            state.credit.release_retained_bytes(bytes);
        } else {
            state.available += bytes;
        }
    }

    /// Borrow prepaid bytes, acquiring an actual engine/lower-work slot only at start. Private
    /// owner identity prevents transferring a loan to another process pool or group. Refusal is
    /// atomic: neither available bytes nor slots change unless both checks succeed.
    fn charge(
        self: &Arc<Self>,
        group: [u8; 32],
        bytes: usize,
        kind: ResourceKind,
    ) -> Result<Credit, VoiceError> {
        if group != self.group {
            return Err(VoiceError::Scope);
        }
        let mut reserve = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if reserve.closed
            || (kind != ResourceKind::Crypto && bytes == 0)
            || bytes > reserve.available
        {
            return Err(VoiceError::Capacity);
        }
        let mut state = self
            .pool
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // The original retained charge guarantees this group record exists throughout every
        // loan. Slots remain shared with ordinary work and other reserves, never prepaid idle.
        if kind.slot().is_some_and(|(slot, group_max, process_max)| {
            state.total.slots[slot] >= process_max || state.groups[&group].slots[slot] >= group_max
        }) {
            return Err(VoiceError::Capacity);
        }
        if let Some((slot, _, _)) = kind.slot() {
            state.total.slots[slot] += 1;
            state
                .groups
                .get_mut(&group)
                .expect("retained reserve")
                .slots[slot] += 1;
        }
        reserve.available -= bytes;
        Ok(Credit {
            pool: self.pool.clone(),
            group,
            bytes,
            kind,
            reservation: Some(self.clone()),
            lane: None,
            custody: None,
            partitioned: false,
        })
    }
}

impl Credit {
    /// Borrow the exact pool that owns this credit without allocating a duplicate owner.
    /// Fixed plan metadata uses this for pool-identity checks for its whole retained lifetime.
    pub(super) fn resources(&self) -> &Arc<Resources> {
        &self.pool
    }

    /// Partition one retained action's already prepaid input bytes from its fixed metadata.
    /// No new charge, refund, authority or slot is created. Both partitions retain any original
    /// phase/count custody until its actual last owner drops; reserved portions return under
    /// the same reserve→pool close protocol. Errors leave the original credit intact.
    ///
    /// Only one partition is allowed, on BOTH resulting owners. Crypto/Forward credits and
    /// prepaid work-lane loans must never be split or refunded through this capacity seam.
    pub(super) fn split_retained(&mut self, bytes: usize) -> Result<Credit, VoiceError> {
        if self.kind != ResourceKind::Retained
            || self.lane.is_some()
            || self.partitioned
            || bytes == 0
            || bytes >= self.bytes
        {
            return Err(VoiceError::Bounds);
        }
        self.bytes -= bytes;
        self.partitioned = true;
        Ok(Credit {
            pool: self.pool.clone(),
            group: self.group,
            bytes,
            kind: ResourceKind::Retained,
            reservation: self.reservation.clone(),
            lane: None,
            custody: self.custody.clone(),
            partitioned: true,
        })
    }

    pub(crate) fn with_custody(mut self, custody: catcoms_rt::SharedRequestKeepalive) -> Self {
        assert!(self.custody.is_none(), "one native phase owner per credit");
        self.custody = Some(custody);
        self
    }
    /// Shrink only the ordinary retained backing credit. Reserved loans never refund bytes
    /// directly; their reserve serializes the refund with close/charge. Lock order is always
    /// reserve then pool. The remaining credit destructor releases only its remaining bytes.
    fn release_retained_bytes(&mut self, bytes: usize) {
        debug_assert!(self.kind == ResourceKind::Retained && self.reservation.is_none());
        assert!(bytes <= self.bytes, "reserve backing accounting");
        if bytes == 0 {
            // Closing a zero-byte engine loan may already have removed its group slot record.
            return;
        }
        let mut state = self
            .pool
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.bytes -= bytes;
        state.total.bytes -= bytes;
        let counts = state.groups.get_mut(&self.group).expect("retained reserve");
        counts.bytes -= bytes;
        if counts.bytes == 0 && counts.slots.iter().all(|count| *count == 0) {
            state.groups.remove(&self.group);
        }
    }
}

impl Drop for Credit {
    fn drop(&mut self) {
        let mut state = self
            .pool
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.reservation.is_none() {
            state.total.bytes -= self.bytes;
        }
        if let Some((slot, _, _)) = self.kind.slot() {
            state.total.slots[slot] -= 1;
        }
        if let Some(counts) = state.groups.get_mut(&self.group) {
            if self.reservation.is_none() {
                counts.bytes -= self.bytes;
            }
            if let Some((slot, _, _)) = self.kind.slot() {
                counts.slots[slot] -= 1;
            }
            if counts.bytes == 0 && counts.slots.iter().all(|count| *count == 0) {
                state.groups.remove(&self.group);
            }
        }
        // Never acquire the reservation mutex while holding the pool mutex: charge holds them
        // in the other order. Also release both before dropping the last reserve Arc, whose
        // original credit destructor must be able to acquire the pool mutex again.
        drop(state);
        if let Some(reservation) = self.reservation.take() {
            reservation.return_loan(self.bytes);
        }
        // Automatic lane destruction precedes custody (declaration order). Preserve that
        // order: Workflow's final retirement publisher relies on every actual byte/lane
        // return occurring before the last custody Arc can destroy the Workflow itself.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_composition_snapshot_excludes_process_and_other_group_payments() {
        let pool = Arc::new(Resources::default());
        let process = pool.charge_process_retained(4096).unwrap();
        assert_eq!(pool.group_counts([1; 32]), None);
        let first = pool.charge([1; 32], 2048, ResourceKind::Crypto).unwrap();
        let other = pool.charge([2; 32], 512, ResourceKind::Retained).unwrap();
        assert_eq!(pool.group_counts([1; 32]), Some((2048, 1)));
        assert_eq!(pool.group_counts([2; 32]), Some((512, 0)));
        assert_eq!(pool.counts(), (6656, 1));
        drop(first);
        assert_eq!(pool.group_counts([1; 32]), None);
        assert_eq!(pool.group_counts([2; 32]), Some((512, 0)));
        drop((other, process));
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn process_retained_bounds_and_refusal_are_atomic_without_group_or_slot_records() {
        let pool = Arc::new(Resources::default());
        for invalid in [0, PROCESS_BYTES + 1, usize::MAX] {
            assert!(matches!(
                pool.charge_process_retained(invalid),
                Err(VoiceError::Capacity)
            ));
            assert_eq!(pool.counts(), (0, 0));
        }
        let exact = pool.charge_process_retained(PROCESS_BYTES).unwrap();
        assert_eq!(pool.counts(), (PROCESS_BYTES, 0));
        assert!(pool.charge_process_retained(1).is_err());
        assert!(pool.charge([1; 32], 1, ResourceKind::Crypto).is_err());
        {
            let state = pool.state.lock().unwrap();
            assert!(state.groups.is_empty());
            assert_eq!(state.total.slots, [0; SLOT_KINDS]);
        }
        drop(exact);
        assert_eq!(pool.counts(), (0, 0));
        assert!(pool.charge_process_retained(PROCESS_BYTES).is_ok());
    }

    #[test]
    fn process_retained_shares_process_ceiling_and_preserves_group_and_crypto_limits() {
        let pool = Arc::new(Resources::default());
        let process = pool
            .charge_process_retained(PROCESS_BYTES - GROUP_BYTES)
            .unwrap();
        let group = pool.reserve([1; 32], GROUP_BYTES).unwrap();
        let mut lanes = Vec::new();
        for _ in 0..4 {
            lanes.push(pool.reserve_crypto_lane([1; 32]).unwrap());
        }
        assert!(pool.reserve_crypto_lane([1; 32]).is_err());
        assert!(pool.reserve([1; 32], 1).is_err());
        assert!(pool.reserve([2; 32], 1).is_err());
        assert!(pool.charge_process_retained(1).is_err());
        assert_eq!(pool.counts(), (PROCESS_BYTES, 4));
        drop(process);
        // Process capacity becoming free does not increase the existing group's byte limit.
        assert!(pool.reserve([1; 32], 1).is_err());
        let other = pool.reserve([2; 32], GROUP_BYTES).unwrap();
        drop((group, other, lanes));
        assert_eq!(pool.counts(), (0, 0));
        assert!(pool.state.lock().unwrap().groups.is_empty());
    }

    #[test]
    fn process_retained_survives_group_reserve_close_and_last_zero_byte_lane() {
        let pool = Arc::new(Resources::default());
        let process = pool.charge_process_retained(4096).unwrap();
        let reserve = pool.reserve([3; 32], 8192).unwrap();
        let actual = reserve.charge([3; 32], 2048, ResourceKind::Crypto).unwrap();
        let lane = pool.reserve_crypto_lane([3; 32]).unwrap();
        reserve.close();
        assert_eq!(pool.counts(), (4096 + 2048, 2));
        drop((reserve, actual, lane));
        assert_eq!(pool.counts(), (4096, 0));
        assert!(pool.state.lock().unwrap().groups.is_empty());
        drop(process);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn process_retained_is_independent_of_full_group_table_and_both_drop_orders() {
        for process_first in [false, true] {
            let pool = Arc::new(Resources::default());
            let groups: Vec<_> = (0..GROUP_RECORDS)
                .map(|index| {
                    let mut group = [0; 32];
                    group[..8].copy_from_slice(&(index as u64).to_be_bytes());
                    pool.charge(group, 1, ResourceKind::Retained).unwrap()
                })
                .collect();
            assert!(pool.charge([255; 32], 1, ResourceKind::Retained).is_err());
            let process = pool.charge_process_retained(4096).unwrap();
            assert_eq!(pool.state.lock().unwrap().groups.len(), GROUP_RECORDS);
            if process_first {
                drop(process);
                assert_eq!(pool.counts(), (GROUP_RECORDS, 0));
                drop(groups);
            } else {
                drop(groups);
                assert_eq!(pool.counts(), (4096, 0));
                assert!(pool.state.lock().unwrap().groups.is_empty());
                drop(process);
            }
            assert_eq!(pool.counts(), (0, 0));
        }
    }

    #[test]
    fn process_retained_full_bytes_do_not_borrow_or_release_crypto_and_forward_slots() {
        let pool = Arc::new(Resources::default());
        let process = pool.charge_process_retained(PROCESS_BYTES).unwrap();
        let mut lanes = Vec::new();
        for group in [1, 2] {
            for _ in 0..4 {
                lanes.push(pool.reserve_crypto_lane([group; 32]).unwrap());
                lanes.push(pool.reserve_forward_lane([group; 32]).unwrap());
            }
        }
        assert!(pool.reserve_crypto_lane([3; 32]).is_err());
        assert!(pool.reserve_forward_lane([3; 32]).is_err());
        drop(process);
        assert_eq!(pool.counts(), (0, 8));
        assert!(pool.reserve_crypto_lane([3; 32]).is_err());
        assert!(pool.reserve_forward_lane([3; 32]).is_err());
        drop(lanes);
        assert_eq!(pool.counts(), (0, 0));
        assert!(pool.state.lock().unwrap().groups.is_empty());
    }

    #[test]
    fn process_retained_concurrent_admission_has_one_winner_until_actual_owner_drops() {
        let pool = Arc::new(Resources::default());
        let start = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            let spawn = || {
                scope.spawn(|| {
                    start.wait();
                    pool.charge_process_retained(PROCESS_BYTES / 2 + 1)
                })
            };
            let first = spawn();
            let second = spawn();
            start.wait();
            let results = [first.join().unwrap(), second.join().unwrap()];
            assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
            assert_eq!(pool.counts(), (PROCESS_BYTES / 2 + 1, 0));
            drop(results);
        });
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn process_retained_completed_uncollected_output_and_shared_last_owner_keep_charge() {
        let pool = Arc::new(Resources::default());
        let credit = Arc::new(pool.charge_process_retained(PROCESS_BYTES).unwrap());
        let worker_owner = credit.clone();
        let (done, completed) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            done.send(()).unwrap();
            worker_owner
        });
        drop(credit);
        // Whether running or completed, the uncollected task output still owns its charge.
        completed.recv().unwrap();
        assert_eq!(pool.counts(), (PROCESS_BYTES, 0));
        assert!(pool.charge_process_retained(1).is_err());
        let output = worker.join().unwrap();
        assert_eq!(pool.counts(), (PROCESS_BYTES, 0));
        let last = output.clone();
        drop(output);
        assert!(pool.charge_process_retained(1).is_err());
        drop(last);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn fixed_retained_partition_conserves_ordinary_charge_and_refuses_recursive_splitting() {
        let pool = Arc::new(Resources::default());
        for payload_first in [false, true] {
            let mut metadata = pool.charge([1; 32], 4096, ResourceKind::Retained).unwrap();
            for invalid in [0, 4096, usize::MAX] {
                assert!(metadata.split_retained(invalid).is_err());
                assert_eq!(pool.counts(), (4096, 0));
            }
            let mut payload = metadata.split_retained(2560).unwrap();
            assert_eq!(pool.counts(), (4096, 0));
            assert!(metadata.split_retained(1).is_err());
            assert!(payload.split_retained(1).is_err());
            if payload_first {
                drop(payload);
                assert_eq!(pool.counts(), (1536, 0));
                drop(metadata);
            } else {
                drop(metadata);
                assert_eq!(pool.counts(), (2560, 0));
                drop(payload);
            }
            assert_eq!(pool.counts(), (0, 0));
        }
    }

    #[test]
    fn fixed_reserved_partition_returns_each_actual_part_across_concurrent_close() {
        let pool = Arc::new(Resources::default());
        let reserve = pool.reserve([2; 32], 8192).unwrap();
        let mut metadata = reserve
            .charge([2; 32], 4096, ResourceKind::Retained)
            .unwrap();
        let payload = metadata.split_retained(2560).unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                reserve.close();
            });
            barrier.wait();
            drop(metadata);
        });
        assert_eq!(pool.counts(), (2560, 0));
        drop(reserve);
        assert_eq!(
            pool.counts(),
            (2560, 0),
            "actual last partition owns the retired reserve"
        );
        drop(payload);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn fixed_partition_retains_shared_phase_until_last_part_and_refuses_actual_work_slots() {
        #[derive(Debug)]
        struct Phase(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Phase {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::Release);
            }
        }
        let pool = Arc::new(Resources::default());
        let retired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut metadata = pool
            .charge([3; 32], 4096, ResourceKind::Retained)
            .unwrap()
            .with_custody(Arc::new(Phase(retired.clone())));
        let payload = metadata.split_retained(2560).unwrap();
        drop(metadata);
        assert!(!retired.load(std::sync::atomic::Ordering::Acquire));
        drop(payload);
        assert!(retired.load(std::sync::atomic::Ordering::Acquire));
        for kind in [ResourceKind::Crypto, ResourceKind::Forward] {
            let mut credit = pool.charge([3; 32], 4096, kind).unwrap();
            let before = pool.counts();
            assert!(credit.split_retained(2560).is_err());
            assert_eq!(pool.counts(), before);
        }
        let reserve = pool.reserve([3; 32], 8192).unwrap();
        let lane = pool.reserve_crypto_lane([3; 32]).unwrap();
        let budget = ByteBudget::ReservedLane {
            reservation: reserve.clone(),
            lane: lane.clone(),
        };
        let mut actual = budget
            .charge(&pool, [3; 32], 4096, ResourceKind::Crypto)
            .unwrap();
        let before = pool.counts();
        assert!(
            actual.split_retained(2560).is_err(),
            "work-lane loan has Retained kind internally but still owns a real slot"
        );
        assert_eq!(pool.counts(), before);
        reserve.close();
        lane.close();
        drop((actual, budget, reserve, lane));
        assert_eq!(pool.counts(), (0, 0));
    }
    #[test]
    fn prepaid_lane_serializes_actual_work_and_survives_retirement_without_double_slots() {
        let pool = Arc::new(Resources::default());
        let group = [5; 32];
        let reserve = pool.reserve(group, 224 * 1024).unwrap();
        let lane = pool.reserve_crypto_lane(group).unwrap();
        let budget = ByteBudget::ReservedLane {
            reservation: reserve.clone(),
            lane: lane.clone(),
        };
        let ordinary_bytes = pool
            .charge(group, GROUP_BYTES - 224 * 1024, ResourceKind::Retained)
            .unwrap();
        let ordinary_slots: Vec<_> = (0..3)
            .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
            .collect();
        assert!(pool.charge(group, 0, ResourceKind::Crypto).is_err());
        let verification = Arc::new(
            budget
                .charge(&pool, group, 50_236, ResourceKind::Crypto)
                .unwrap(),
        );
        assert_eq!(pool.counts(), (GROUP_BYTES, 4));
        assert!(budget
            .charge(&pool, group, 61_624, ResourceKind::Crypto)
            .is_err());
        let held = verification.clone();
        drop(verification);
        assert!(budget
            .charge(&pool, group, 61_624, ResourceKind::Crypto)
            .is_err());
        drop(held);
        let engine = Arc::new(
            budget
                .charge(&pool, group, 61_624, ResourceKind::Crypto)
                .unwrap(),
        );
        assert_eq!(
            pool.counts(),
            (GROUP_BYTES, 4),
            "lane is counted once across stage changes"
        );
        reserve.close();
        lane.close();
        lane.close();
        assert_eq!(pool.counts(), (GROUP_BYTES - 224 * 1024 + 61_624, 4));
        assert!(budget
            .charge(&pool, group, 1, ResourceKind::Crypto)
            .is_err());
        drop((budget, reserve, lane));
        assert_eq!(pool.counts().1, 4, "actual engine keeps its retired lane");
        drop(engine);
        assert_eq!(pool.counts(), (GROUP_BYTES - 224 * 1024, 3));
        drop((ordinary_bytes, ordinary_slots));
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn prepaid_lane_refusal_preserves_bytes_and_cannot_use_another_pool_or_group() {
        let pool = Arc::new(Resources::default());
        let other = Arc::new(Resources::default());
        let group = [6; 32];
        let reserve = pool.reserve(group, 1024).unwrap();
        let lane = pool.reserve_crypto_lane(group).unwrap();
        let budget = ByteBudget::ReservedLane {
            reservation: reserve.clone(),
            lane: lane.clone(),
        };
        let before = pool.counts();
        assert_eq!(
            budget.charge(&other, group, 1, ResourceKind::Crypto).err(),
            Some(VoiceError::Scope)
        );
        assert_eq!(
            budget.charge(&pool, [7; 32], 1, ResourceKind::Crypto).err(),
            Some(VoiceError::Scope)
        );
        assert_eq!(
            budget
                .charge(&pool, group, 1025, ResourceKind::Crypto)
                .err(),
            Some(VoiceError::Capacity)
        );
        assert_eq!(pool.counts(), before);
        // Failed byte admission relinquishes its transient lane borrow atomically.
        drop(
            budget
                .charge(&pool, group, 1024, ResourceKind::Crypto)
                .unwrap(),
        );
        assert_eq!(pool.counts(), before);
        lane.close();
        reserve.close();
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn close_releases_only_unused_bytes_and_closed_loans_release_their_backing_once() {
        let pool = Arc::new(Resources::default());
        let reserve = pool.reserve([1; 32], 128).unwrap();
        let ordinary = pool
            .charge([1; 32], GROUP_BYTES - 128, ResourceKind::Retained)
            .unwrap();
        let raw = reserve.charge([1; 32], 32, ResourceKind::Retained).unwrap();
        let engine = reserve.charge([1; 32], 64, ResourceKind::Crypto).unwrap();
        let lower = reserve.charge([1; 32], 16, ResourceKind::Forward).unwrap();
        reserve.close();
        reserve.close();
        assert_eq!(pool.counts(), (GROUP_BYTES - 16, 1));
        assert!(reserve.charge([1; 32], 1, ResourceKind::Retained).is_err());
        assert!(reserve.charge([1; 32], 0, ResourceKind::Crypto).is_err());
        let free = pool.reserve([1; 32], 16).unwrap();
        assert!(pool.reserve([1; 32], 1).is_err());
        drop(raw);
        assert_eq!(pool.counts(), (GROUP_BYTES - 32, 1));
        let next = pool.reserve([1; 32], 32).unwrap();
        let held = Arc::downgrade(&reserve);
        drop(reserve);
        assert!(held.upgrade().is_some());
        drop(engine);
        assert_eq!(pool.counts(), (GROUP_BYTES - 64, 0));
        assert_eq!(pool.state.lock().unwrap().total.slots[7], 1);
        drop(lower);
        assert!(held.upgrade().is_none());
        assert_eq!(pool.counts(), (GROUP_BYTES - 80, 0));
        drop((ordinary, free, next));
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn closed_zero_byte_engine_preserves_its_slot_without_pinching_unused_bytes() {
        let pool = Arc::new(Resources::default());
        let reserve = pool.reserve([1; 32], 128).unwrap();
        let engine = reserve.charge([1; 32], 0, ResourceKind::Crypto).unwrap();
        reserve.close();
        assert_eq!(pool.counts(), (0, 1));
        assert!(reserve.charge([1; 32], 0, ResourceKind::Crypto).is_err());
        let replacement = pool.reserve([1; 32], GROUP_BYTES).unwrap();
        drop(reserve);
        drop(engine);
        // The old zero-byte backing destructor must not delete/refund the new group's credit.
        assert_eq!(pool.counts(), (GROUP_BYTES, 0));
        drop(replacement);
        assert!(pool.state.lock().unwrap().groups.is_empty());
    }

    #[test]
    fn close_releases_exact_capacity_at_the_process_limit() {
        let pool = Arc::new(Resources::default());
        let reserves: Vec<_> = (0..PROCESS_BYTES / GROUP_BYTES)
            .map(|group| pool.reserve([group as u8; 32], GROUP_BYTES).unwrap())
            .collect();
        let held = reserves[0]
            .charge([0; 32], 512, ResourceKind::Crypto)
            .unwrap();
        assert_eq!(pool.counts(), (PROCESS_BYTES, 1));
        reserves[0].close();
        assert_eq!(pool.counts(), (PROCESS_BYTES - GROUP_BYTES + 512, 1));
        let replacement = pool.reserve([99; 32], GROUP_BYTES - 512).unwrap();
        assert!(pool.reserve([99; 32], 1).is_err());
        drop(held);
        let returned = pool.reserve([99; 32], 512).unwrap();
        assert_eq!(pool.counts(), (PROCESS_BYTES, 0));
        drop((reserves, replacement, returned));
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn concurrent_close_keeps_actual_loans_charged_and_never_reopens() {
        let pool = Arc::new(Resources::default());
        let reserve = pool.reserve([1; 32], 128).unwrap();
        let ready = std::sync::Barrier::new(5);
        let release = std::sync::Barrier::new(5);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let reserve = &reserve;
                let ready = &ready;
                let release = &release;
                scope.spawn(move || {
                    let loan = reserve.charge([1; 32], 16, ResourceKind::Crypto).unwrap();
                    ready.wait();
                    release.wait();
                    drop(loan);
                    assert!(reserve.charge([1; 32], 0, ResourceKind::Crypto).is_err());
                });
            }
            ready.wait();
            reserve.close();
            assert_eq!(pool.counts(), (64, 4));
            release.wait();
        });
        assert_eq!(pool.counts(), (0, 0));
        assert!(reserve.charge([1; 32], 16, ResourceKind::Retained).is_err());
        drop(reserve);
        assert!(pool.state.lock().unwrap().groups.is_empty());
    }

    #[test]
    fn reserved_budget_cannot_charge_another_process_pool_with_the_same_group() {
        let pool = Arc::new(Resources::default());
        let another = Arc::new(Resources::default());
        let reserve = pool.reserve([1; 32], 128).unwrap();
        let budget = ByteBudget::Reserved(reserve);
        assert!(budget
            .charge(&another, [1; 32], 128, ResourceKind::Crypto)
            .is_err());
        assert!(budget
            .charge(&pool, [2; 32], 128, ResourceKind::Crypto)
            .is_err());
        assert_eq!(pool.counts(), (128, 0));
        assert_eq!(another.counts(), (0, 0));
        let loan = budget
            .charge(&pool, [1; 32], 128, ResourceKind::Crypto)
            .unwrap();
        assert_eq!(pool.counts(), (128, 1));
        drop(budget);
        assert_eq!(pool.counts(), (128, 1));
        drop(loan);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn zero_byte_engine_loan_retains_reserve_and_slot_until_actual_release() {
        let pool = Arc::new(Resources::default());
        let reserve = pool.reserve([1; 32], 128).unwrap();
        let held = Arc::downgrade(&reserve);
        let engine = reserve.charge([1; 32], 0, ResourceKind::Crypto).unwrap();
        drop(reserve);
        assert!(held.upgrade().is_some());
        assert_eq!(pool.counts(), (128, 1));
        drop(engine);
        assert!(held.upgrade().is_none());
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn concurrent_reserved_charge_and_drop_preserve_exact_shared_accounting() {
        let pool = Arc::new(Resources::default());
        let reserve = pool.reserve([1; 32], 128).unwrap();
        // Four workers never need more than four actual group engine slots or 64 prepaid
        // bytes. This is a lock/accounting stress seam, not a timing or fairness assertion.
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let reserve = &reserve;
                scope.spawn(move || {
                    for _ in 0..100 {
                        let loan = reserve.charge([1; 32], 16, ResourceKind::Crypto).unwrap();
                        drop(loan);
                    }
                });
            }
        });
        assert_eq!(pool.counts(), (128, 0));
        let whole = reserve
            .charge([1; 32], 128, ResourceKind::Retained)
            .unwrap();
        drop((reserve, whole));
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn prepaid_progress_survives_full_ordinary_bytes_and_returns_only_actual_loans() {
        let pool = Arc::new(Resources::default());
        let reserve = pool.reserve([1; 32], 64 * 1024).unwrap();
        let ordinary = pool
            .charge([1; 32], GROUP_BYTES - 64 * 1024, ResourceKind::Retained)
            .unwrap();
        assert_eq!(pool.counts(), (GROUP_BYTES, 0));
        assert!(pool.charge([1; 32], 1, ResourceKind::Retained).is_err());
        let raw = reserve
            .charge([1; 32], 2048, ResourceKind::Retained)
            .unwrap();
        let engine = reserve.charge([1; 32], 8192, ResourceKind::Crypto).unwrap();
        let lower = reserve
            .charge([1; 32], 8192, ResourceKind::Forward)
            .unwrap();
        let rest = reserve
            .charge(
                [1; 32],
                64 * 1024 - 2048 - 8192 - 8192,
                ResourceKind::Retained,
            )
            .unwrap();
        assert!(reserve.charge([1; 32], 1, ResourceKind::Retained).is_err());
        assert_eq!(pool.counts(), (GROUP_BYTES, 1));
        drop(raw);
        // One released copy cannot refund the other actual engine or transport copies.
        assert!(reserve
            .charge([1; 32], 2049, ResourceKind::Retained)
            .is_err());
        let next = reserve
            .charge([1; 32], 2048, ResourceKind::Retained)
            .unwrap();
        assert_eq!(pool.counts(), (GROUP_BYTES, 1));
        drop((next, rest, lower));
        let held = Arc::downgrade(&reserve);
        drop(reserve);
        assert!(held.upgrade().is_some());
        assert_eq!(pool.counts(), (GROUP_BYTES, 1));
        assert!(pool.reserve([1; 32], 1).is_err());
        drop(engine);
        assert!(held.upgrade().is_none());
        assert_eq!(pool.counts(), (GROUP_BYTES - 64 * 1024, 0));
        assert!(pool.reserve([1; 32], 64 * 1024).is_ok());
        drop(ordinary);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn reserve_slot_refusal_scope_and_extreme_sizes_are_atomic() {
        let pool = Arc::new(Resources::default());
        assert!(pool.reserve([1; 32], 0).is_err());
        assert!(pool.reserve([1; 32], usize::MAX).is_err());
        let reserve = pool.reserve([1; 32], 128).unwrap();
        assert!(reserve.charge([2; 32], 1, ResourceKind::Retained).is_err());
        assert!(reserve
            .charge([1; 32], usize::MAX, ResourceKind::Retained)
            .is_err());
        assert!(reserve.charge([1; 32], 0, ResourceKind::Retained).is_err());
        let slots: Vec<_> = (0..4)
            .map(|_| pool.charge([1; 32], 0, ResourceKind::Crypto).unwrap())
            .collect();
        assert!(reserve.charge([1; 32], 128, ResourceKind::Crypto).is_err());
        let whole = reserve
            .charge([1; 32], 128, ResourceKind::Retained)
            .unwrap();
        drop((slots, whole));
        let engine = reserve.charge([1; 32], 128, ResourceKind::Crypto).unwrap();
        assert_eq!(pool.counts(), (128, 1));
        drop(engine);
        assert_eq!(pool.counts(), (128, 0));
        drop(reserve);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn reserve_loans_share_process_slots_and_keep_bytes_after_native_owner_retires() {
        let pool = Arc::new(Resources::default());
        let mut engines = Vec::new();
        for group in [1, 2] {
            let reserve = pool.reserve([group; 32], 256).unwrap();
            for _ in 0..4 {
                engines.push(
                    reserve
                        .charge([group; 32], 64, ResourceKind::Crypto)
                        .unwrap(),
                );
            }
            drop(reserve);
        }
        assert_eq!(pool.counts(), (512, 8));
        let other = pool.reserve([3; 32], 128).unwrap();
        assert!(other.charge([3; 32], 128, ResourceKind::Crypto).is_err());
        engines.pop();
        let last = other.charge([3; 32], 128, ResourceKind::Crypto).unwrap();
        assert_eq!(pool.counts(), (640, 8));
        drop((engines, other));
        assert_eq!(pool.counts(), (128, 1));
        drop(last);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn engine_and_retained_work_share_group_bytes_and_exact_credit_ownership() {
        let pool = Arc::new(Resources::default());
        let cache = pool
            .charge([1; 32], GROUP_BYTES - 128, ResourceKind::Retained)
            .unwrap();
        let running = pool.charge([1; 32], 128, ResourceKind::Crypto).unwrap();
        assert!(pool.charge([1; 32], 1, ResourceKind::Crypto).is_err());
        assert_eq!(pool.counts(), (GROUP_BYTES, 1));
        drop(cache);
        assert_eq!(pool.counts(), (128, 1));
        drop(running);
        assert_eq!(pool.counts(), (0, 0));
    }
    #[test]
    fn process_bytes_and_engine_limits_do_not_multiply_across_groups_or_cloned_handles() {
        let pool = Arc::new(Resources::default());
        let another = pool.clone();
        let mut held = Vec::new();
        for group in 0..PROCESS_BYTES / GROUP_BYTES {
            held.push(
                pool.charge([group as u8; 32], GROUP_BYTES, ResourceKind::Retained)
                    .unwrap(),
            );
        }
        assert!(another.charge([99; 32], 1, ResourceKind::Retained).is_err());
        drop(held);
        assert_eq!(pool.counts(), (0, 0));
        let mut held = Vec::new();
        for group in [1, 2] {
            for _ in 0..4 {
                held.push(pool.charge([group; 32], 0, ResourceKind::Crypto).unwrap());
            }
        }
        assert!(another.charge([3; 32], 0, ResourceKind::Crypto).is_err());
        held.pop();
        let last = another.charge([3; 32], 0, ResourceKind::Crypto).unwrap();
        assert!(pool.charge([1; 32], 0, ResourceKind::Crypto).is_err());
        drop(held);
        drop(last);
        assert_eq!(pool.counts(), (0, 0));
    }
    #[test]
    fn extreme_byte_declarations_refuse_before_arithmetic_or_allocation() {
        let pool = Arc::new(Resources::default());
        assert!(pool
            .charge([1; 32], usize::MAX, ResourceKind::Retained)
            .is_err());
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn tiny_retained_charges_cannot_grow_an_unbounded_group_table() {
        let pool = Arc::new(Resources::default());
        assert!(pool.charge([1; 32], 0, ResourceKind::Retained).is_err());
        let mut held = Vec::new();
        for index in 0..GROUP_RECORDS {
            let mut group = [0; 32];
            group[..8].copy_from_slice(&(index as u64).to_be_bytes());
            held.push(pool.charge(group, 1, ResourceKind::Retained).unwrap());
        }
        assert!(pool.charge([99; 32], 1, ResourceKind::Retained).is_err());
        held.pop();
        assert!(pool.charge([99; 32], 1, ResourceKind::Retained).is_ok());
        drop(held);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn challenge_and_watcher_slots_share_process_ownership_without_multiplying_by_handle() {
        let pool = Arc::new(Resources::default());
        let mut challenges = Vec::new();
        for group in [1, 2] {
            for _ in 0..32 {
                challenges.push(
                    pool.charge([group; 32], 1, ResourceKind::Challenge)
                        .unwrap(),
                );
            }
        }
        assert!(pool.charge([1; 32], 1, ResourceKind::Challenge).is_err());
        assert!(pool
            .clone()
            .charge([3; 32], 1, ResourceKind::Challenge)
            .is_err());
        challenges.pop();
        let replacement = pool.charge([3; 32], 1, ResourceKind::Challenge).unwrap();
        let watchers = (0..8)
            .map(|_| pool.charge([1; 32], 1, ResourceKind::Watcher).unwrap())
            .collect::<Vec<_>>();
        assert!(pool.charge([1; 32], 1, ResourceKind::Watcher).is_err());
        drop(watchers);
        drop(challenges);
        drop(replacement);
        assert_eq!(pool.counts(), (0, 0));
    }
}
