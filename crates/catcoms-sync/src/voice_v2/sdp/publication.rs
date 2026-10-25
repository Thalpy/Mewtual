//! Original invocation outcome, retained independently of Pair/LocalJoin teardown.
//!
//! A missing Actor reply cannot prove that publication did not happen. The actual Root
//! publisher must record its SAME Action in its infallible final tail, before activation,
//! notifications or destruction. All three recorded Root factories write this outcome.
//! The native adapter must include the whole slot/candidate/writer/observation graph in
//! its original fixed row fee and install just one slot under its unique Index fence.
use super::{InitialSdpIntent, VoiceError};
use crate::voice_v2::chain::sdp_handoff::{NativeRowFunding, NativeRowPurpose};
use crate::voice_v2::crypto_actions::CryptoAction;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

const IDLE: u8 = 0;
const RESERVED: u8 = 1;
const ENTERED: u8 = 2;
const UNCERTAIN: u8 = 3;
const NEVER_ENTERED: u8 = 4;
const PUBLISHED: u8 = 5;
const REFUSED: u8 = 16;

/// Validated fixed candidate. Preparation invokes no Clock, allocator or native/Root
/// locks. Only the original native row's unique installation may allocate its slot.
pub struct Prepared {
    intent: Arc<InitialSdpIntent>,
    body: Option<[u8; 32]>,
    funding: Arc<NativeRowFunding>,
}

/// Fixed original outcome. There is deliberately no Phase/Row/Copy/body/publication
/// guard backedge. Intent/fee/ActionBinding must never retain this slot in return.
pub struct Outcome {
    state: AtomicU8,
    // Only the unique affine entered Writer may initialize this. Readers call get(),
    // never get_or_init/wait: final publication cannot block on a competing initializer.
    published: OnceLock<Arc<CryptoAction>>,
    intent: Arc<InitialSdpIntent>,
    body: Option<[u8; 32]>,
    // LAST: fixed observations and the actual slot stay paid through their destruction.
    funding: Arc<NativeRowFunding>,
}

/// One real invocation, owned before Actor enqueue. Dropping a never-entered command
/// certifies only that invocation. Once entered, generic Drop always stays uncertain.
pub struct Writer {
    outcome: Arc<Outcome>,
    stage: Stage,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    PreRoot,
    Entered,
    Finished,
}

/// Fixed history, not fresh work or permission to use an SDK. A negative is useful for
/// native compaction only after admission CLOSED+zero IO and actual worker retirement.
pub enum History {
    Idle,
    InFlight,
    NeverEntered,
    Refused(VoiceError),
    Uncertain,
    Published(Arc<CryptoAction>),
}
redacted_debug!(Prepared, Outcome, Writer, History);

impl Prepared {
    /// Final native installation must repeat exact allocation/body provenance against
    /// its stored parent. Sharing a Phase or serialized labels is insufficient.
    pub fn matches(
        &self,
        intent: &Arc<InitialSdpIntent>,
        funding: &Arc<NativeRowFunding>,
        body: Option<[u8; 32]>,
    ) -> bool {
        Arc::ptr_eq(intent, &self.intent)
            && Arc::ptr_eq(funding, &self.funding)
            && body == self.body
    }
    /// The exact body commitment must come from bounded original request bytes before
    /// native admission guards. This is payment/provenance, never current effect authority.
    pub fn new(
        intent: Arc<InitialSdpIntent>,
        funding: Arc<NativeRowFunding>,
        body: Option<[u8; 32]>,
    ) -> Result<Self, VoiceError> {
        if !funding.uses_effect(&intent.effect)
            || (funding.purpose() == NativeRowPurpose::Seal) != body.is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        // Validate provenance first: an invalid candidate must not choose the intent's
        // permanent profile. Once chosen, even a later native installation refusal may
        // not reopen the unrecorded path.
        intent.select_recorded_publication()?;
        Ok(Self {
            intent,
            funding,
            body,
        })
    }

    /// Trusted native installation only, ONCE after shared-index uniqueness. A candidate
    /// can be retried without allocating speculative slots. This construction is infallible
    /// apart from process-aborting OOM and calls no Clock, Root lock or callback.
    pub fn install(&self) -> Arc<Outcome> {
        Arc::new(Outcome {
            state: AtomicU8::new(IDLE),
            published: OnceLock::new(),
            intent: self.intent.clone(),
            body: self.body,
            funding: self.funding.clone(),
        })
    }
}

impl Outcome {
    /// Pure positive provenance for native capture. It neither enters the engine nor
    /// clones an input/Action owner under native guards.
    pub fn owns_publication(&self, action: &Arc<CryptoAction>) -> bool {
        self.published
            .get()
            .is_some_and(|original| Arc::ptr_eq(original, action))
    }
    /// Stable negative for the canonical native row ONLY after actual IO admission
    /// closes and every bound Copy retires. Native must also hold its final Index/State
    /// fence and prove worker/observer retirement. Read freshly there, never cache this.
    pub fn is_proven_unpublished(&self) -> bool {
        if !self.funding.is_collected() {
            return false;
        }
        let state = self.state.load(Ordering::Acquire);
        (matches!(state, IDLE | NEVER_ENTERED) || error(state).is_some())
            && self.published.get().is_none()
    }
    /// Live retry eligibility only, never a negative drainage witness. The native adapter
    /// must hold its original Index/parent/row fence and exclusively reclaim the worker
    /// before obtaining another Writer. An entered interruption, non-capacity refusal or
    /// any published Action must be recovered/closed instead of submitted again.
    pub fn permits_local_retry(&self) -> bool {
        let state = self.state.load(Ordering::Acquire);
        !self.funding.is_closed()
            && (matches!(state, IDLE | NEVER_ENTERED)
                || state == REFUSED + code(VoiceError::Capacity))
            && self.published.get().is_none()
    }
    /// One exclusive publisher at a time. A proven prepublication Capacity refusal does
    /// not consume the original row forever: exact retry uses this SAME slot and deadline.
    /// Native must still repeat its own current/physical/final admission before queueing.
    pub fn try_writer(self: &Arc<Self>) -> Result<Writer, VoiceError> {
        self.try_writer_after_observation(|| {})
    }
    fn try_writer_after_observation(
        self: &Arc<Self>,
        after_observation: impl FnOnce(),
    ) -> Result<Writer, VoiceError> {
        if self.funding.is_closed() {
            return Err(VoiceError::Unauthorized);
        }
        after_observation();
        self.state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state == IDLE || state == NEVER_ENTERED || error(state).is_some())
                    .then_some(RESERVED)
            })
            .map_err(|state| {
                if matches!(state, RESERVED | ENTERED) {
                    VoiceError::Capacity
                } else {
                    VoiceError::Unauthorized
                }
            })?;
        // Close may win after the first observation but before this exclusive reservation.
        // Do not let that stale observation return a new writer into a collected row.
        if self.funding.is_closed() {
            self.state.store(NEVER_ENTERED, Ordering::Release);
            return Err(VoiceError::Unauthorized);
        }
        Ok(Writer {
            outcome: self.clone(),
            stage: Stage::PreRoot,
        })
    }
    #[cfg(test)]
    pub(in crate::voice_v2) fn try_writer_after_observation_for_test(
        self: &Arc<Self>,
        hook: impl FnOnce(),
    ) -> Result<Writer, VoiceError> {
        self.try_writer_after_observation(hook)
    }

    /// Historical observation only; no native guard may be held by a future consumer
    /// that subsequently reconciles the returned Action's engine or Clock history.
    pub fn history(&self) -> History {
        // Positive evidence wins even if the publisher unwound immediately after the
        // single initialization and before its final scalar observation was installed.
        if let Some(action) = self.published.get() {
            return History::Published(action.clone());
        }
        match self.state.load(Ordering::Acquire) {
            IDLE => History::Idle,
            RESERVED | ENTERED => History::InFlight,
            NEVER_ENTERED => History::NeverEntered,
            state if error(state).is_some() => History::Refused(error(state).unwrap()),
            _ => History::Uncertain,
        }
    }

    /// Whole Arc allocation; native must separately count its row and transient wrappers.
    pub fn allocation_bytes() -> usize {
        #[repr(C)]
        struct Allocation {
            strong: std::sync::atomic::AtomicUsize,
            weak: std::sync::atomic::AtomicUsize,
            value: Outcome,
        }
        std::mem::size_of::<Allocation>()
    }
}

impl Writer {
    /// Pure native final-admission identity; not Root entry or a new effect grant. The
    /// affine token must still be owned by the original Registration's real worker.
    pub fn belongs_to(&self, outcome: &Arc<Outcome>) -> bool {
        Arc::ptr_eq(&self.outcome, outcome)
            && self.stage == Stage::PreRoot
            && self.outcome.state.load(Ordering::Acquire) == RESERVED
            && !self.outcome.funding.is_closed()
    }
    /// Root entry is irreversible uncertainty unless its final tail records publication
    /// or the ordinary return path proves prepublication refusal. Not exposed to App.
    pub(crate) fn enter(
        &mut self,
        intent: &Arc<InitialSdpIntent>,
        body: Option<[u8; 32]>,
    ) -> Result<(), VoiceError> {
        if self.stage != Stage::PreRoot
            || !Arc::ptr_eq(intent, &self.outcome.intent)
            || body != self.outcome.body
        {
            return Err(VoiceError::Unauthorized);
        }
        intent.check_publication_profile(true)?;
        self.outcome
            .state
            .compare_exchange(RESERVED, ENTERED, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| VoiceError::Unauthorized)?;
        self.stage = Stage::Entered;
        Ok(())
    }

    /// Validate original intent/purpose/body before any Root action publication. This is
    /// fixed identity only; the factory must independently validate current authority.
    pub(crate) fn check_entered(
        &self,
        intent: &Arc<InitialSdpIntent>,
        body: Option<[u8; 32]>,
    ) -> Result<(), VoiceError> {
        if self.stage != Stage::Entered
            || !Arc::ptr_eq(intent, &self.outcome.intent)
            || body != self.outcome.body
            || self.outcome.state.load(Ordering::Acquire) != ENTERED
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }

    /// Call ONLY in Root's successful final publication tail. All provenance validation
    /// must precede the first mutation. There is one affine writer and no other initializer,
    /// so OnceLock::set cannot wait here. The stored Action owns its original Root payment.
    pub(crate) fn published(&mut self, action: &Arc<CryptoAction>) {
        assert!(self.stage == Stage::Entered);
        assert_eq!(self.outcome.state.load(Ordering::Acquire), ENTERED);
        assert!(self.outcome.published.set(action.clone()).is_ok());
        self.stage = Stage::Finished;
        self.outcome.state.store(PUBLISHED, Ordering::Release);
    }

    /// Explicit normal prepublication Err only, outside Root's final origin guards. A
    /// previously recorded success can never be downgraded by a delayed error path.
    pub(crate) fn refused(&mut self, reason: VoiceError) {
        if self.stage != Stage::Entered {
            return;
        }
        // Retire this local token BEFORE publishing retry eligibility. Otherwise its
        // later Drop could mistake a successor's RESERVED/ENTERED state for its own.
        self.stage = Stage::Finished;
        let _ = self.outcome.state.compare_exchange(
            ENTERED,
            REFUSED + code(reason),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // There is no arbitrary native callback or notification in any Root guard.
        // Actual RPC Work/Copy retirement supplies the independent cleanup wake later.
        let transition = match self.stage {
            Stage::PreRoot => Some((RESERVED, NEVER_ENTERED)),
            Stage::Entered => Some((ENTERED, UNCERTAIN)),
            Stage::Finished => None,
        };
        if let Some((from, to)) = transition {
            let _ =
                self.outcome
                    .state
                    .compare_exchange(from, to, Ordering::AcqRel, Ordering::Acquire);
        }
    }
}

fn code(error: VoiceError) -> u8 {
    match error {
        VoiceError::Malformed => 0,
        VoiceError::Bounds => 1,
        VoiceError::Signature => 2,
        VoiceError::Scope => 3,
        VoiceError::Freshness => 4,
        VoiceError::Unauthorized => 5,
        VoiceError::Signing => 6,
        VoiceError::Capacity => 7,
        VoiceError::Replay => 8,
        VoiceError::Reservation => 9,
        VoiceError::RetryMismatch => 10,
        VoiceError::RateLimited => 11,
    }
}
fn error(state: u8) -> Option<VoiceError> {
    Some(match state.checked_sub(REFUSED)? {
        0 => VoiceError::Malformed,
        1 => VoiceError::Bounds,
        2 => VoiceError::Signature,
        3 => VoiceError::Scope,
        4 => VoiceError::Freshness,
        5 => VoiceError::Unauthorized,
        6 => VoiceError::Signing,
        7 => VoiceError::Capacity,
        8 => VoiceError::Replay,
        9 => VoiceError::Reservation,
        10 => VoiceError::RetryMismatch,
        11 => VoiceError::RateLimited,
        _ => return None,
    })
}
