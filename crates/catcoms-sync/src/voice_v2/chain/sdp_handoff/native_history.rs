//! Fixed native SDP-row history, independent of variable buffers/tasks. The original Phase
//! prepares at most one fee per closed Seal/Open purpose. This is payment, NOT a native row
//! issuer or effect grant: the native index must still preserve one original row/purpose.
use super::*;
use std::sync::atomic::AtomicUsize;
use tokio::sync::Notify;

/// Includes the concrete funding/Notify allocation AND the native row/history graph. The
/// native adapter must inventory that whole graph before publication; this is no new pool.
pub const NATIVE_ROW_BYTES: usize = 2304;
const CLOSED: usize = 1 << (usize::BITS - 1);
const COUNT: usize = !CLOSED;

/// The only two native SDP histories within one original complete-SDP phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeRowPurpose {
    Seal,
    Open,
}
impl NativeRowPurpose {
    fn index(self) -> usize {
        match self {
            Self::Seal => 0,
            Self::Open => 1,
        }
    }
}

/// A plain fixed fee and concrete IO counter. It retains no Phase/Row/Copy, SDK result or
/// callback. Native row compaction needs CLOSED+zero, its worker's actual completion and
/// engine settlement/ACK; zero while open is merely a snapshot which may change.
pub struct NativeRowFunding {
    purpose: NativeRowPurpose,
    state: AtomicUsize,
    changed: Notify,
    original: EffectObserver,
    // LAST: counter/Notify/original observer remain paid through every real last Drop.
    _credit: Credit,
}
redacted_debug!(NativeRowFunding, NativeIoClosure);

/// Negative-only admission closure. Construct under a native fence, but move this ticket
/// OUT of every native/Phase guard before destruction: notification may run a waker inline.
pub struct NativeIoClosure(Arc<NativeRowFunding>);
impl Drop for NativeIoClosure {
    fn drop(&mut self) {
        self.0.changed.notify_waiters();
    }
}
impl NativeRowFunding {
    pub fn purpose(&self) -> NativeRowPurpose {
        self.purpose
    }
    /// Allocation provenance only, including after Close. Equal scope/key labels do not
    /// establish that this fee belongs to the exact phase installed in the native PC.
    pub fn uses_phase(&self, phase: &Phase) -> bool {
        Arc::ptr_eq(&self.original.0, &phase.effect)
    }
    /// Pure fixed provenance for the original publication outcome. No Phase or variable
    /// loan is retained by that outcome, including after storage closure/destruction.
    pub(crate) fn uses_effect(&self, effect: &EffectObserver) -> bool {
        self.original.is_same(effect)
    }
    pub fn in_flight(&self) -> usize {
        self.state.load(Ordering::Acquire) & COUNT
    }
    /// Irreversible admission fence, independently of outstanding concrete IO. Native row
    /// installation must not mistake a closing nonzero counter for an open purpose.
    pub fn is_closed(&self) -> bool {
        self.state.load(Ordering::Acquire) & CLOSED != 0
    }
    /// Stable local IO collection only AFTER its original admission closed. This is not
    /// engine/SDK/whole-phase drainage, actual native row issuance or fresh effect authority.
    pub fn is_collected(&self) -> bool {
        self.state.load(Ordering::Acquire) == CLOSED
    }
    pub fn close_io(self: &Arc<Self>) -> NativeIoClosure {
        self.state.fetch_or(CLOSED, Ordering::AcqRel);
        NativeIoClosure(self.clone())
    }
    /// One bounded native cleanup observer may wait AFTER relinquishing all of its bound
    /// Copies/TaskCopies/responses. Waiting with one's own contribution would deadlock.
    pub async fn wait_collected(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.is_collected() {
                return;
            }
            changed.await;
        }
    }
    pub(super) fn acquire_copy(&self) -> Result<(), VoiceError> {
        // Phase Drop frees its variable position BEFORE it retires this counter. A new
        // Copy can exist in that gap; enforce this counter's OWN bound instead of inferring
        // <=16 from Phase.copies. Refusal leaves the caller's same affine Copy unchanged.
        self.state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state & CLOSED == 0 && (state & COUNT) < COPIES).then_some(state + 1)
            })
            .map(|_| ())
            .map_err(|state| {
                if state & CLOSED != 0 {
                    VoiceError::Unauthorized
                } else {
                    VoiceError::Capacity
                }
            })
    }
    fn release_copy(&self) {
        let previous = self.state.fetch_sub(1, Ordering::AcqRel);
        assert!(previous & COUNT > 0, "one original native IO contribution");
        if previous == CLOSED + 1 {
            // Copy credit and Phase count already retired, and their guards are gone.
            self.changed.notify_waiters();
        }
    }
    /// Whole fixed allocation including Arc counters/alignment; add native Row/Index/
    /// wrappers separately before asserting that their shared2304B payment is sufficient.
    pub fn allocation_bytes() -> usize {
        #[repr(C)]
        struct Allocation {
            strong: AtomicUsize,
            weak: AtomicUsize,
            value: NativeRowFunding,
        }
        std::mem::size_of::<Allocation>()
    }
}

/// Runs after the Copy's credit/count retirement, including test-hook unwind. This has
/// no Row callback and cannot reconstruct effect authority. Its fixed fee remains paid.
pub(super) struct Collection(Arc<NativeRowFunding>);
impl Collection {
    pub(super) fn new(funding: Arc<NativeRowFunding>) -> Self {
        Self(funding)
    }
}
impl Drop for Collection {
    fn drop(&mut self) {
        self.0.release_copy();
    }
}

impl Phase {
    /// Prepare/recover the SAME fixed history fee from the original64KiB envelope. Exact
    /// recovery precedes currency checks, but creates no row, worker or effect permission.
    /// Call outside native/Root guards: admission pays before allocating the funding Arc.
    pub fn native_history(
        &self,
        purpose: NativeRowPurpose,
    ) -> Result<Arc<NativeRowFunding>, VoiceError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(original) = &state.history[purpose.index()] {
            return Ok(original.clone());
        }
        if state.status != Status::Open || !self.effect().is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let credit = ByteBudget::Reserved(self.reserve.clone()).charge(
            &self.resources,
            self.scope.group,
            NATIVE_ROW_BYTES,
            ResourceKind::Retained,
        )?;
        let funding = Arc::new(NativeRowFunding {
            purpose,
            state: AtomicUsize::new(0),
            changed: Notify::new(),
            original: self.effect(),
            _credit: credit,
        });
        state.history[purpose.index()] = Some(funding.clone());
        Ok(funding)
    }
}
