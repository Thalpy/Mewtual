//! A single native-process budget shared by endpoint caches, helper queues and engine permits.
//! Credits are affine Rust owners: only actual completion/expiry may drop them. Revocation,
//! timeout, a lost IPC waiter and controller replacement do not themselves release a credit.

use super::VoiceError;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const PROCESS_BYTES: usize = 2 * 1024 * 1024;
const GROUP_BYTES: usize = 256 * 1024;
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
redacted_debug!(Resources, Credit);

/// An exact non-cloneable charge. Keep it in a pending-or-uncertain operation/cache record.
pub struct Credit {
    pool: Arc<Resources>,
    group: [u8; 32],
    bytes: usize,
    kind: ResourceKind,
    // A loan returns to its already charged reserve only after its actual owner releases it.
    // Once closed, the reserve instead releases that loan's part of the backing charge.
    reservation: Option<Arc<Reservation>>,
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
            Self::Reserved(reservation) => {
                if !Arc::ptr_eq(resources, &reservation.pool) {
                    return Err(VoiceError::Scope);
                }
                reservation.charge(group, bytes, kind)
            }
        }
    }
}

impl Resources {
    pub fn charge(
        self: &Arc<Self>,
        group: [u8; 32],
        bytes: usize,
        kind: ResourceKind,
    ) -> Result<Credit, VoiceError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let counts = state.groups.get(&group);
        if (kind != ResourceKind::Crypto && bytes == 0)
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
    /// Aggregate diagnostics only; no stable group identifiers or payloads leave the pool.
    pub fn counts(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        (state.total.bytes, state.total.slots[0])
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
        })
    }
}

impl Credit {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let reserves: Vec<_> = (0..8)
            .map(|group| pool.reserve([group; 32], GROUP_BYTES).unwrap())
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
        for group in 0..8 {
            held.push(
                pool.charge([group; 32], GROUP_BYTES, ResourceKind::Retained)
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
