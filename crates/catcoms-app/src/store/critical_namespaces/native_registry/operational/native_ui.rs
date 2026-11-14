//! Canonical UI generations for the owning native vault. This is lifecycle
//! bookkeeping, not a device, row, source, SDK or native-window credential.
//!
//! An independently paid affine invocation captures metadata before dispatch.
//! Counter construction, discarded candidates and retirement all happen outside
//! guards. Workers receive only weak commands and negative retirement records;
//! neither the affine binding nor generation close owners enter a worker.
use super::*;
use catcoms_rt::{
    NativeChatKeyDrain, NativeChatUiBinding, NativeChatUiCommand, NativeChatUiCurrentCheck,
    NativeChatUiSessionOwner, NativeChatUiSessionPreparation,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Mutex, Weak,
};

const GENERATIONS: usize = 32;
const INVOCATION_BYTES: usize = 4096;
const HOST_BYTES: usize = 4096;
const REGISTRY_BYTES: usize = 16 * 1024;
const MINIMUM_CAPACITY: usize = 2 * GENERATIONS * INVOCATION_BYTES + HOST_BYTES + REGISTRY_BYTES;

/// Trusted process policy; cloning a policy grants no command or native access.
/// Each invocation reserves its OWN candidate payment before borrowing a vault.
#[derive(Clone)]
pub struct NativeUiInvocationBudget {
    pool: CriticalCatalogReadPool,
}
impl NativeUiInvocationBudget {
    /// Process policy DATA; this minimum funds one mounted host and the bounded
    /// independent candidate overlap. It grants no capture or native access.
    pub const fn required_capacity() -> usize {
        MINIMUM_CAPACITY
    }
    pub fn new(pool: CriticalCatalogReadPool) -> Result<Self, AppError> {
        if pool.capacity() < MINIMUM_CAPACITY {
            return Err(refused());
        }
        Ok(Self { pool })
    }
    /// Outside all guards, before original-vault capture. One affine credit can
    /// construct at most one candidate, including a losing/transient candidate.
    pub fn prepare(&self) -> Result<NativeUiInvocationMetadata, AppError> {
        Ok(NativeUiInvocationMetadata {
            fee: Fee::reserve(&self.pool, INVOCATION_BYTES)?,
        })
    }
}

/// Actual paid metadata, not permission. Deliberately non-Clone/Serialize/Debug.
pub struct NativeUiInvocationMetadata {
    fee: Arc<Fee>,
}
struct Fee {
    debit: CatalogReadReservation,
}
impl Fee {
    fn reserve(pool: &CriticalCatalogReadPool, bytes: usize) -> Result<Arc<Self>, AppError> {
        let debit = pool.try_reserve(bytes)?;
        #[cfg(test)]
        let debit = {
            let mut debit = debit;
            NEXT_FEE_NOTICE.with(|slot| {
                if let Some(notice) = slot.borrow_mut().take() {
                    debit.observe_drop_for_test(notice);
                }
            });
            debit
        };
        Ok(Arc::new(Self { debit }))
    }
    fn require(&self, pool: &CriticalCatalogReadPool, bytes: usize) -> Result<(), AppError> {
        self.debit.validate(pool, bytes).map_err(|_| refused())
    }
}

/// Affine close owner. Keep FIRST in the native facade, outside physical work
/// custody. Drop this outside guards; close_first itself is SC-only and is the
/// whole-vault fence before any registry/operation/index lock is attempted.
pub(super) struct UiAdmissionOwner {
    binding: NativeChatUiBinding,
    registry: Arc<Registry>,
}
struct Registry {
    state: Mutex<State>,
    budget: NativeUiInvocationBudget,
    // LAST: actual slots/control objects retire before their independent fee.
    _fee: Arc<Fee>,
}
struct State {
    slots: Vec<Generation>,
    // Never lower this on GC. A retired/pruned generation cannot be reopened
    // merely because native's current atomics were mistakenly rolled back.
    highest: Option<u64>,
}
struct Generation {
    generation: u64,
    owner: Option<NativeChatUiSessionOwner>,
    record: Arc<RetirementRecord>,
}
struct RetirementRecord {
    drain: NativeChatKeyDrain,
    retiring: AtomicBool,
    // Counter holds this FEE, never this record: record->drain->counter->record
    // would be a cycle. Unentered pins may outlive CLOSED+zero and slot GC.
    _fee: Arc<Fee>,
}

impl UiAdmissionOwner {
    /// Build before installing the affine mounted owner; no writer is held.
    /// The actual native factory supplies the real AppState atomics, not JSON.
    pub(super) fn new(
        generation: Arc<AtomicU64>,
        authorized: Arc<AtomicU64>,
        requested: Arc<AtomicBool>,
        budget: NativeUiInvocationBudget,
    ) -> Result<Self, AppError> {
        let host_fee = Fee::reserve(&budget.pool, HOST_BYTES)?;
        let registry_fee = Fee::reserve(&budget.pool, REGISTRY_BYTES)?;
        Ok(Self {
            binding: NativeChatUiBinding::from_trusted_host(
                generation, authorized, requested, host_fee,
            ),
            registry: Arc::new(Registry {
                state: Mutex::new(State {
                    slots: Vec::with_capacity(GENERATIONS),
                    highest: None,
                }),
                budget,
                _fee: registry_fee,
            }),
        })
    }
    pub(super) fn observe_mount(&self) -> catcoms_rt::NativeChatUiMountObservation {
        self.binding.observe_mount()
    }
    pub(super) fn close_first(&self) {
        self.binding.close();
    }
    /// Infallible original capture. This consumes the independently paid
    /// invocation rather than cloning one candidate fee into many candidates.
    /// Retain the returned whole capsule until outside every guard.
    pub(super) fn prepare_capture(
        &self,
        expected: u64,
        credit: NativeUiInvocationMetadata,
    ) -> UiPreparation {
        UiPreparation {
            rt: self.binding.prepare_session(expected, credit.fee.clone()),
            original: Arc::downgrade(&self.registry),
            expected,
            fee: credit.fee,
        }
    }

    /// No allocation, IO, destruction or close occurs under the registry lock.
    /// The candidate remains in the caller's outer slot on EVERY refusal and
    /// also retains the unused loser on an existing canonical-generation hit.
    pub(super) fn admit(&self, candidate: &mut UiCandidate) -> Result<UiCapture, AppError> {
        candidate
            .fee
            .require(&self.registry.budget.pool, INVOCATION_BYTES)?;
        if candidate.used
            || candidate.expected == 0
            || !Weak::ptr_eq(&candidate.original, &Arc::downgrade(&self.registry))
            || !self.binding.is_current(candidate.expected)
        {
            return Err(refused());
        }
        let result = match self.registry.state.try_lock() {
            Ok(mut state) => self.admit_under_registry(candidate, &mut state),
            Err(error) => {
                drop(error);
                None
            }
        };
        // Error formatting allocates; every registry/poison guard is gone.
        result.ok_or_else(refused)
    }

    fn admit_under_registry(
        &self,
        candidate: &mut UiCandidate,
        state: &mut State,
    ) -> Option<UiCapture> {
        if state
            .highest
            .is_some_and(|highest| candidate.expected < highest)
        {
            return None;
        }
        if let Some(slot) = state
            .slots
            .iter()
            .find(|slot| slot.generation == candidate.expected)
        {
            let owner = slot.owner.as_ref()?;
            if !self.binding.is_current(candidate.expected) {
                return None;
            }
            let capture = UiCapture {
                command: owner.observe(),
                current: owner.observe_current(),
                expected: candidate.expected,
                record: slot.record.clone(),
                original: candidate.original.clone(),
                _invocation: candidate.fee.clone(),
            };
            candidate.used = true;
            return Some(capture);
        }
        if state
            .highest
            .is_some_and(|highest| candidate.expected <= highest)
            || state.slots.len() == GENERATIONS
            || !self.binding.is_current(candidate.expected)
        {
            return None;
        }
        let owner = candidate.owner.take()?;
        let capture = UiCapture {
            command: owner.observe(),
            current: owner.observe_current(),
            expected: candidate.expected,
            record: candidate.record.clone(),
            original: candidate.original.clone(),
            _invocation: candidate.fee.clone(),
        };
        // Capacity was allocated at mount time; all following moves/clones are
        // non-fallible and retain the same actual owner and paid record.
        state.slots.push(Generation {
            generation: candidate.expected,
            owner: Some(owner),
            record: candidate.record.clone(),
        });
        state.highest = Some(candidate.expected);
        candidate.used = true;
        Some(capture)
    }

    /// Original canonical observation only; native window/row/source authority
    /// and final primitive/publication admission remain separate checks.
    pub(super) fn require_capture<'a>(
        &self,
        capture: &'a UiCapture,
    ) -> Result<&'a NativeChatUiCommand, AppError> {
        capture
            ._invocation
            .require(&self.registry.budget.pool, INVOCATION_BYTES)?;
        if !Weak::ptr_eq(&capture.original, &Arc::downgrade(&self.registry))
            || !self.binding.is_current(capture.expected)
        {
            return Err(refused());
        }
        let valid = match self.registry.state.try_lock() {
            Ok(state) => state.slots.iter().any(|slot| {
                slot.generation == capture.expected
                    && slot.owner.is_some()
                    && Arc::ptr_eq(&slot.record, &capture.record)
                    && self.binding.is_current(capture.expected)
            }),
            Err(error) => {
                drop(error);
                false
            }
        };
        if !valid {
            return Err(refused());
        }

        Ok(&capture.command)
    }

    /// Final cached negatives only. Full payment/registry validation belongs
    /// outside native commit. The actual capture stays owned outside that guard
    /// through every return/unwind, so this cannot retire a last pin/payment.
    pub(super) fn is_current_cached(&self, capture: &UiCapture) -> bool {
        std::ptr::eq(capture.original.as_ptr(), Arc::as_ptr(&self.registry))
            && self.binding.is_current(capture.expected)
            && !capture
                .record
                .retiring
                .load(std::sync::atomic::Ordering::SeqCst)
            && capture.current.is_current()
    }

    /// Called after native synchronously publishes its actual lock generation/
    /// request flag. Move close owners out BEFORE notifying or waiting. A batch
    /// is returned only if it owns at least one distinct generation/payment;
    /// repeated callers must coalesce with the native-owned retirement task.
    pub(super) fn begin_retirement(&self, through: u64) -> Result<UiRetirement, AppError> {
        let mut batch = RetirementBatch {
            owners: std::array::from_fn(|_| None),
            records: std::array::from_fn(|_| None),
        };
        // Guard declared after batch: an unwind releases it BEFORE any moved
        // close owner/last payment can drop and synchronously run a Waker.
        let mut state = match self.registry.state.try_lock() {
            Ok(state) => state,
            Err(error) => {
                drop(error);
                return Err(refused());
            }
        };
        let mut moved = false;
        let mut pending = false;
        for (index, slot) in state.slots.iter_mut().enumerate() {
            if slot.generation > through {
                continue;
            }
            if slot.owner.is_some() {
                // Withdraw canonical admission BEFORE moving the close owner.
                // The final cached edge must see this even before outside-
                // guard notification/counter retirement has happened.
                slot.record
                    .retiring
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                batch.owners[index] = slot.owner.take();
                moved = true;
            }
            pending |= !slot.record.drain.is_drained();
        }
        // Repeated/no-work replies must hold NO record custody after unlock:
        // concurrent final-End + GC could otherwise make their temporary batch
        // release a last payment under the caller's outer native guard.
        if moved {
            for (index, slot) in state.slots.iter().enumerate() {
                if slot.generation <= through {
                    batch.records[index] = Some(slot.record.clone());
                }
            }
        }
        drop(state);
        #[cfg(test)]
        after_retirement_unlock();
        if moved {
            Ok(UiRetirement {
                state: UiRetirementState::Started,
                batch: Some(batch),
            })
        } else if pending {
            Ok(UiRetirement {
                state: UiRetirementState::AlreadyRetiring,
                batch: None,
            })
        } else {
            Ok(UiRetirement {
                state: UiRetirementState::KeyWorkDrained,
                batch: None,
            })
        }
    }

    /// GC only CLOSED+zero slots whose owner was already moved out. Return all
    /// records for destruction AFTER releasing native and registry guards.
    pub(super) fn collect_drained(&self) -> Result<UiGarbage, AppError> {
        collect_drained(&self.registry)
    }
}

fn collect_drained(registry: &Registry) -> Result<UiGarbage, AppError> {
    let mut garbage = UiGarbage {
        _slots: std::array::from_fn(|_| None),
    };
    let mut state = match registry.state.try_lock() {
        Ok(state) => state,
        Err(error) => {
            drop(error);
            return Err(refused());
        }
    };
    let mut index = 0;
    let mut removed = 0;
    while index < state.slots.len() {
        if state.slots[index].owner.is_none() && state.slots[index].record.drain.is_drained() {
            garbage._slots[removed] = Some(state.slots.swap_remove(index));
            removed += 1;
        } else {
            index += 1;
        }
    }
    Ok(garbage)
}

#[cfg(test)]
thread_local! {
    static NEXT_FEE_NOTICE: std::cell::RefCell<Option<Arc<AtomicBool>>> = const { std::cell::RefCell::new(None) };
    static AFTER_RETIREMENT_UNLOCK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn after_retirement_unlock() {
    if let Some(hook) = AFTER_RETIREMENT_UNLOCK.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

/// Original paid UI metadata capture. Finish outside every native/Root guard.
/// This preparation is not native caller, row or primitive authorization.
pub struct UiPreparation {
    rt: NativeChatUiSessionPreparation,
    original: Weak<Registry>,
    expected: u64,
    fee: Arc<Fee>,
}
impl UiPreparation {
    /// Outside all guards. Current/retired capture failures release only their
    /// own independently paid candidate and cannot reclaim canonical work.
    pub fn finish(self) -> Result<UiCandidate, AppError> {
        let owner = self.rt.finish().map_err(|_| refused())?;
        let record = Arc::new(RetirementRecord {
            drain: owner.observe_retirement(),
            retiring: AtomicBool::new(false),
            _fee: self.fee.clone(),
        });
        Ok(UiCandidate {
            owner: Some(owner),
            record,
            original: self.original,
            expected: self.expected,
            fee: self.fee,
            used: false,
        })
    }
}
/// Opaque independently paid candidate, retained outside native guard scopes
/// even when a different canonical generation wins. Not a close capability.
pub struct UiCandidate {
    owner: Option<NativeChatUiSessionOwner>,
    record: Arc<RetirementRecord>,
    original: Weak<Registry>,
    expected: u64,
    fee: Arc<Fee>,
    used: bool,
}
/// Original canonical lifecycle observation. Its private command cannot be
/// extracted by an external caller or reconstructed from serialized labels.
pub struct UiCapture {
    pub(super) command: NativeChatUiCommand,
    current: NativeChatUiCurrentCheck,
    expected: u64,
    record: Arc<RetirementRecord>,
    original: Weak<Registry>,
    _invocation: Arc<Fee>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Key-work-only retirement status, never whole native lock completion.
pub enum UiRetirementState {
    Started,
    AlreadyRetiring,
    // Key work ONLY: not a completed native UI lock, physical/SDK/worker drain.
    KeyWorkDrained,
}
// Fixed stack custody avoids a large tagged variant AND avoids allocating a
// Box during lock. Only Started carries the already-paid finite batch.
/// Fixed already-paid retirement result. Only Started carries a batch; repeated
/// callers must coalesce through the actual native owner's bounded waiter task.
pub struct UiRetirement {
    state: UiRetirementState,
    batch: Option<RetirementBatch>,
}
impl UiRetirement {
    pub fn state(&self) -> UiRetirementState {
        self.state
    }
    pub fn into_batch(self) -> Option<RetirementBatch> {
        self.batch
    }
    /// Observe only this original batch's key counters before moving it into a
    /// native-owned task. The observation grants no key or close authority. It
    /// lets native recover after an unpolled task is dropped without mistaking
    /// a replacement mount's empty registry for completion of the old work.
    pub fn observe_drain(&self) -> Option<RetirementDrain> {
        self.batch.as_ref().map(|batch| RetirementDrain {
            records: std::array::from_fn(|i| batch.records[i].clone()),
        })
    }
}
/// Original key-counter observation only: neither a whole-vault shutdown proof
/// nor permission to reopen a UI generation. Keep this envelope non-Clone; the
/// coalesced native job shares one independently paid observation by reference.
pub struct RetirementDrain {
    records: [Option<Arc<RetirementRecord>>; GENERATIONS],
}
impl RetirementDrain {
    /// Both closure and zero entered work are required. Zero while an owner is
    /// still open must never permit GC or recovery after a failed native task.
    pub fn is_drained_cached(&self) -> bool {
        self.records
            .iter()
            .flatten()
            .all(|record| record.drain.is_drained())
    }
    /// Wait on the original counters outside every native/Root guard.
    pub async fn wait(&self) {
        for record in self.records.iter().flatten() {
            record.drain.wait().await;
        }
    }
}
/// Affine close/drain custody. Move out before any notification, Drop or wait.
pub struct RetirementBatch {
    owners: [Option<NativeChatUiSessionOwner>; GENERATIONS],
    records: [Option<Arc<RetirementRecord>>; GENERATIONS],
}
impl RetirementBatch {
    /// Outside every guard. Never return success merely because a wait timed
    /// out. Caller owns/coalesces this task independently of its IPC reply.
    pub async fn retire(mut self) {
        for owner in &mut self.owners {
            if let Some(owner) = owner.take() {
                drop(owner.retire());
            }
        }
        for record in self.records.iter().flatten() {
            record.drain.wait().await;
        }
    }
}
/// Drop outside guards. Unentered pins may retain their generation payment
/// after this GC; work completion never means metadata/custody destruction.
pub struct UiGarbage {
    _slots: [Option<Generation>; GENERATIONS],
}

// Public native-facing envelopes are deliberately opaque in diagnostics: no
// original pointers, private atomics, paid internals or retained fields leak.
macro_rules! opaque_debug {
    ($($ty:ty),+ $(,)?) => { $(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+ };
}
opaque_debug!(
    NativeUiInvocationBudget,
    NativeUiInvocationMetadata,
    UiPreparation,
    UiCandidate,
    UiCapture,
    UiRetirement,
    RetirementDrain,
    RetirementBatch,
    UiGarbage
);

const _: () = assert!(
    std::mem::size_of::<RetirementBatch>()
        + std::mem::size_of::<UiCandidate>()
        + std::mem::size_of::<UiPreparation>()
        + std::mem::size_of::<UiCapture>()
        + catcoms_rt::NATIVE_CHAT_UI_FIXED_METADATA_BYTES
        + std::mem::size_of::<UiRetirement>()
        + 6 * 64 // Arc controls and wrapper/error scratch
        <= INVOCATION_BYTES
);

#[cfg(test)]
mod tests;
const _: () = assert!(
    std::mem::size_of::<Registry>()
        + GENERATIONS * std::mem::size_of::<Generation>()
        + std::mem::size_of::<UiGarbage>()
        + 1024
        <= REGISTRY_BYTES
);
