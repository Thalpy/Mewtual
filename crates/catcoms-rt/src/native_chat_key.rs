//! Concrete native text-key lifecycle primitives.
//!
//! These counters are not SDK, membership, file, or signing-plan authority. The
//! actual Root producer must establish those independently at the primitive.
//! The native private factory supplies the REAL AppState atomics at synchronous
//! invocation capture. An arbitrary Rust caller supplying its own atomics has
//! authenticated nothing. NativeRequired propagation and that checked consumer
//! are mandatory before these primitives may enable a production path.
//!
//! All participating generation/authorization/request writes and admission/close
//! operations use sequential consistency. A generation publication therefore
//! participates in the same ordering proof as final counter admission; a later
//! registry close is not a substitute for reading the actual original atomics.
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use tokio::sync::Notify;

mod sdk_restore;
pub use sdk_restore::{NativeChatSdkRestoreEnds, NativeChatWarmSdkReopenCut};

const CLOSED: usize = 1 << (usize::BITS - 1);
const MAX_ENTRIES: usize = 32;
const PURPOSES: usize = 5;

/// Closed purposes selected by concrete producers, never a WebView permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeChatKeyPurpose {
    Device,
    FounderSdk,
    BootstrapProfile,
    BootstrapHeader,
    MemberAction,
}
impl NativeChatKeyPurpose {
    fn slot(self) -> usize {
        match self {
            Self::Device => 0,
            Self::FounderSdk => 1,
            Self::BootstrapProfile => 2,
            Self::BootstrapHeader => 3,
            Self::MemberAction => 4,
        }
    }
}

/// Refusal before primitive entry. This never refunds an SDK/Core/key attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeChatKeyRefusal {
    Unavailable,
    Busy,
    Foreign,
    WrongPurpose,
    AlreadyEntered,
}

struct Counter {
    state: AtomicUsize,
    drained: Notify,
    // Retains the actual payer/physical metadata, NOT a positive source owner.
    // The Root factory must supply its real paid allocation, never a status flag.
    _custody: Option<Arc<dyn Send + Sync>>,
}
impl Counter {
    fn new(custody: Option<Arc<dyn Send + Sync>>) -> Self {
        Self {
            state: AtomicUsize::new(0),
            drained: Notify::new(),
            _custody: custody,
        }
    }
    fn enter(&self) -> Result<(), NativeChatKeyRefusal> {
        let state = self.state.load(Ordering::SeqCst);
        if state >= MAX_ENTRIES {
            return Err(if state & CLOSED != 0 {
                NativeChatKeyRefusal::Unavailable
            } else {
                NativeChatKeyRefusal::Busy
            });
        }
        #[cfg(test)]
        observe_cut_boundary(CutBoundary::BeforeCounterCas);
        // Exactly one attempt at the Root-held final edge. Contention is a
        // retryable local refusal, never an unbounded CAS loop or key debit.
        self.state
            .compare_exchange(state, state + 1, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|state| {
                if state & CLOSED != 0 {
                    NativeChatKeyRefusal::Unavailable
                } else {
                    NativeChatKeyRefusal::Busy
                }
            })
    }
    fn close_admission_sc(&self) {
        self.state.fetch_or(CLOSED, Ordering::SeqCst);
    }
    fn close_and_notify(&self) {
        self.close_admission_sc();
        // A source may have been fenced under its native registry guard before
        // reaching this outside-guard retirement. Notify idle observers even
        // then: testing only the previous==0 transition would lose that wake.
        // Repeated idle notifications are harmless; registered waiters leave
        // the list on notification, and later waiters recheck CLOSED+zero.
        // Nonzero admitted work is covered by its final End.
        if self.state.load(Ordering::SeqCst) == CLOSED {
            self.drained.notify_waiters();
        }
    }
    fn open(&self) -> bool {
        self.state.load(Ordering::SeqCst) & CLOSED == 0
    }
    fn leave(&self) {
        let previous = self.state.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(previous & !CLOSED > 0, "original key End retired twice");
        if previous & !CLOSED == 1 {
            self.drained.notify_waiters();
        }
    }
}

struct Host {
    generation: Arc<AtomicU64>,
    authorized: Arc<AtomicU64>,
    requested: Arc<AtomicBool>,
    closed: AtomicBool,
    // Fee-only metadata custody, not a retirement record/issuer. It follows
    // actual Host pins even after the affine binding leaves its registry.
    _custody: Arc<dyn Send + Sync>,
}
impl Host {
    fn current(&self, expected: u64) -> bool {
        !self.closed.load(Ordering::SeqCst)
            && !self.requested.load(Ordering::SeqCst)
            && self.authorized.load(Ordering::SeqCst) == expected
            && self.generation.load(Ordering::SeqCst) == expected
    }
}

/// Non-Clone binding to one actual native host. Its destructor closes every
/// observation before releasing the shared host metadata.
pub struct NativeChatUiBinding {
    original: Arc<Host>,
}
impl NativeChatUiBinding {
    /// TRUSTED-HOST seam. Only native's private checked factory may supply these
    /// actual atomics in production. This constructor does not authenticate the
    /// caller, identify a vault, or prove that an unlock really occurred.
    /// Supply the actual independent Host metadata payment, never a registry
    /// record that owns a drain back to this metadata (which would make a cycle).
    pub fn from_trusted_host(
        generation: Arc<AtomicU64>,
        authorized: Arc<AtomicU64>,
        requested: Arc<AtomicBool>,
        host_metadata: Arc<dyn Send + Sync>,
    ) -> Self {
        Self {
            original: Arc::new(Host {
                generation,
                authorized,
                requested,
                closed: AtomicBool::new(false),
                _custody: host_metadata,
            }),
        }
    }

    /// Native's canonical per-generation registry owns the returned close owner.
    /// Capture expected BEFORE parsing the original synchronous invocation; a
    /// worker must never call this with a later generation selected after await.
    /// Reserve generation metadata and its retirement slot BEFORE entry. Its
    /// fee-only custody follows actual pins, not just CLOSED+zero work status.
    /// This combined prepare+finish call must run outside ALL native, registry,
    /// Store and Root guards, including every refusal/drop path.
    pub fn open_session(
        &self,
        expected: u64,
        generation_metadata: Arc<dyn Send + Sync>,
    ) -> Result<NativeChatUiSessionOwner, NativeChatKeyRefusal> {
        self.prepare_session(expected, generation_metadata).finish()
    }

    /// Capture only original paid metadata and the invocation's expected
    /// generation while borrowing the affine owner. No allocation, counter,
    /// close, notification or current permission is issued. Finish AFTER all
    /// native/registry/Store/Root guards have been released.
    pub fn prepare_session(
        &self,
        expected: u64,
        generation_metadata: Arc<dyn Send + Sync>,
    ) -> NativeChatUiSessionPreparation {
        NativeChatUiSessionPreparation {
            host: self.original.clone(),
            expected,
            generation_metadata,
        }
    }

    /// Present-time negative check only. This cannot authorize a command,
    /// source, row, device or SDK and must not replace their final admission.
    pub fn is_current(&self, expected: u64) -> bool {
        self.original.current(expected)
    }

    /// Negative whole-mount observation for locked reauthentication. It
    /// deliberately ignores UI generation/authorization and cannot issue a
    /// session or key work. Its paid Host metadata survives the affine owner.
    pub fn observe_mount(&self) -> NativeChatUiMountObservation {
        NativeChatUiMountObservation {
            original: self.original.clone(),
        }
    }

    /// Irreversibly close this original mounted host BEFORE taking any native,
    /// registry, operation or Root guard. SC-only: no walk, wait or notification.
    /// UI lock retires its generations instead; it must retain the mounted host.
    pub fn close(&self) {
        self.original.closed.store(true, Ordering::SeqCst);
    }
}

/// Paid metadata pin observing only irreversible whole-host closure. An open
/// result grants no native caller, authenticated secret, UI session or key use.
pub struct NativeChatUiMountObservation {
    original: Arc<Host>,
}
impl std::fmt::Debug for NativeChatUiMountObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeChatUiMountObservation")
            .finish_non_exhaustive()
    }
}
impl NativeChatUiMountObservation {
    /// Compare the original runtime allocation, never labels, shared UI atomics
    /// or replacement state. Equality grants no permission and ignores UI epochs;
    /// callers must independently require current authorization and live negatives.
    pub fn same_original(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.original, &other.original)
    }
    /// SC-only check, including while the UI is locked. Never a permission.
    pub fn is_open(&self) -> bool {
        !self.original.closed.load(Ordering::SeqCst)
    }
}

/// Original metadata preparation, never an affine close owner or permission.
/// Retain/drop this outside guards too: its last fee/Host pin may retire there.
/// It cannot be retargeted to the generation current when a worker later polls.
pub struct NativeChatUiSessionPreparation {
    host: Arc<Host>,
    expected: u64,
    generation_metadata: Arc<dyn Send + Sync>,
}
impl NativeChatUiSessionPreparation {
    /// Allocate the original generation outside every native/registry/Store/
    /// Root guard. Both refusals and owner destruction may release payment;
    /// an eventual idle retirement may synchronously run arbitrary Wakers.
    pub fn finish(self) -> Result<NativeChatUiSessionOwner, NativeChatKeyRefusal> {
        if !self.host.current(self.expected) {
            return Err(NativeChatKeyRefusal::Unavailable);
        }
        let value = NativeChatUiSessionOwner {
            original: Arc::new(UiSession {
                host: self.host,
                expected: self.expected,
                // CLOSED+zero ends admitted work, not unentered metadata pins.
                // Counter custody preserves the actual generation payment in
                // prepared cuts, Ends and drains after registry-slot reuse.
                counter: Arc::new(Counter::new(Some(self.generation_metadata))),
            }),
        };
        if !value.original.current() {
            return Err(NativeChatKeyRefusal::Unavailable);
        }
        Ok(value)
    }
}
impl Drop for NativeChatUiBinding {
    fn drop(&mut self) {
        self.close();
    }
}

struct UiSession {
    host: Arc<Host>,
    expected: u64,
    counter: Arc<Counter>,
}
impl UiSession {
    fn current(&self) -> bool {
        self.counter.open() && self.host.current(self.expected)
    }
}

/// Native's affine owner for one original authenticated UI generation.
/// Retire AND Drop outside native/registry/Store/Root guards: idle retirement
/// may synchronously wake preissued observers through arbitrary Wakers.
pub struct NativeChatUiSessionOwner {
    original: Arc<UiSession>,
}
impl NativeChatUiSessionOwner {
    /// Pin only this original session's paid metadata for a final negative
    /// check. Observe the actual canonical owner, never a losing candidate.
    /// Retain/drop the pin outside native/Root commit guards; it is not an
    /// issuer, row/window credential, primitive admission or drainage proof.
    pub fn observe_current(&self) -> NativeChatUiCurrentCheck {
        NativeChatUiCurrentCheck {
            original: self.original.clone(),
        }
    }
    /// Negative retirement observation, allocated/paid with the generation
    /// BEFORE commands may observe it. It grants no close or key authority and
    /// waits for CLOSED+zero, including when first polled while still open.
    /// The host must bound/coalesce its actual waiting tasks independently.
    pub fn observe_retirement(&self) -> NativeChatKeyDrain {
        NativeChatKeyDrain {
            original: self.original.counter.clone(),
        }
    }
    /// Weak command observation; it cannot keep this session open or replace
    /// its generation after lock/unlock.
    pub fn observe(&self) -> NativeChatUiCommand {
        NativeChatUiCommand {
            original: Arc::downgrade(&self.original),
        }
    }
    /// Close synchronously, then return negative drainage. Call this, drop the
    /// owner, and wait outside every native UI/registry/Store and Root guard.
    /// Retirement may synchronously wake observers; no timeout is drainage.
    pub fn retire(self) -> NativeChatKeyDrain {
        self.original.counter.close_and_notify();
        NativeChatKeyDrain {
            original: self.original.counter.clone(),
        }
    }
}
impl Drop for NativeChatUiSessionOwner {
    fn drop(&mut self) {
        self.original.counter.close_and_notify();
    }
}

/// Immutable weak observation captured by the real synchronous native router.
#[derive(Clone)]
pub struct NativeChatUiCommand {
    original: Weak<UiSession>,
}

/// Strong metadata, not a session owner. Owner retirement still closes the
/// counter immediately; this pin only keeps the actual counter/Host fee alive.
pub struct NativeChatUiCurrentCheck {
    original: Arc<UiSession>,
}
impl NativeChatUiCurrentCheck {
    /// Cached SC negatives only: no Weak upgrade, mutex, pool, IO, allocation
    /// or temporary custody destruction. The caller retains this pin through
    /// the entire final guard scope, including refusal and unwind.
    pub fn is_current(&self) -> bool {
        self.original.current()
    }
}

struct Source {
    host: Arc<Host>,
    counter: Arc<Counter>,
}

/// Non-Clone source-close owner. Root's private native operation must associate
/// this exact owner with its real mount/row/invocation/request and creator.
/// Merely constructing this lifecycle object establishes none of those facts.
/// Retire AND Drop outside native/registry/Store/Root guards; counter closure
/// can synchronously run arbitrary Wakers.
pub struct NativeChatSourceOwner {
    original: Arc<Source>,
}
impl NativeChatSourceOwner {
    /// TRUSTED-SOURCE seam for the concrete Root producer. `custody` only keeps
    /// paid physical metadata alive; an arbitrary Arc is not authorization.
    /// The UI host binding stays fixed across later authenticated generations.
    pub fn for_trusted_source(
        command: &NativeChatUiCommand,
        custody: Arc<dyn Send + Sync>,
    ) -> Result<Self, NativeChatKeyRefusal> {
        let ui = command
            .original
            .upgrade()
            .ok_or(NativeChatKeyRefusal::Unavailable)?;
        if !ui.current() {
            return Err(NativeChatKeyRefusal::Unavailable);
        }
        Ok(Self {
            original: Arc::new(Source {
                host: ui.host.clone(),
                counter: Arc::new(Counter::new(Some(custody))),
            }),
        })
    }
    /// Weak observation to bind the actual retained creator/cache/Host.
    pub fn observe(&self) -> NativeChatSourceObservation {
        NativeChatSourceObservation {
            original: Arc::downgrade(&self.original),
        }
    }
    /// Stop new key entry before taking a native row/operation/Store guard.
    /// This is SC atomics only: it neither notifies arbitrary Wakers nor drops
    /// payment/custody. It grants no completed Leave, worker/IO/SDK drainage or
    /// key-erasure proof. Move/retire the owner outside guards afterward.
    pub fn close(&self) {
        self.original.counter.close_admission_sc();
    }
    /// Negative key-work observation, preissued with the paid source metadata.
    /// It is not a source-close capability, metadata retirement or worker drain.
    /// Actual waiting tasks must be independently bounded by the native owner.
    pub fn observe_retirement(&self) -> NativeChatKeyDrain {
        NativeChatKeyDrain {
            original: self.original.counter.clone(),
        }
    }
    /// Prepare before taking Root writers. No counter/key admission occurs here.
    pub fn prepare_cut(
        &self,
        command: &NativeChatUiCommand,
        purpose: NativeChatKeyPurpose,
    ) -> Result<NativeChatPreparedKeyCut, NativeChatKeyRefusal> {
        let ui = command
            .original
            .upgrade()
            .ok_or(NativeChatKeyRefusal::Unavailable)?;
        if !Arc::ptr_eq(&ui.host, &self.original.host) {
            return Err(NativeChatKeyRefusal::Foreign);
        }
        if !ui.current() || !self.original.counter.open() {
            return Err(NativeChatKeyRefusal::Unavailable);
        }
        Ok(NativeChatPreparedKeyCut {
            // Pin ONLY negative metadata before a Root writer can exist. An
            // early refusal at the primitive must never drop a last temporary
            // Arc and release physical/payment custody under that writer.
            ui,
            source: self.original.clone(),
            purpose,
            entered: false,
        })
    }
    /// Irreversibly retire this source before waiting on an escaping worker.
    /// Call/Drop outside native/registry/Store/Root guards: closure can run Wakers.
    pub fn retire(self) -> NativeChatKeyDrain {
        self.original.counter.close_and_notify();
        NativeChatKeyDrain {
            original: self.original.counter.clone(),
        }
    }
}
impl Drop for NativeChatSourceOwner {
    fn drop(&mut self) {
        self.original.counter.close_and_notify();
    }
}

/// Weak original-source identity, never a row label or decoded permission.
#[derive(Clone)]
pub struct NativeChatSourceObservation {
    original: Weak<Source>,
}
impl NativeChatSourceObservation {
    /// Actual original allocation equality, never a decoded row/source label.
    /// Equality alone authenticates no mount, plan or native command.
    pub fn same_original(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.original, &other.original)
    }
}

/// One concrete per-purpose check at the actual SDK/key primitive. The owning
/// caller prepares it before Root writers and keeps the End batch outside them.
pub struct NativeChatPreparedKeyCut {
    // Strong metadata pins, NEVER affine owners. Source/UI owner Drop still
    // closes their counters immediately. Keep this cut outside Root writers.
    ui: Arc<UiSession>,
    source: Arc<Source>,
    purpose: NativeChatKeyPurpose,
    entered: bool,
}
impl NativeChatPreparedKeyCut {
    /// Cached negative check with no temporary metadata upgrade/destructor.
    /// It is not primitive, SDK, file or source admission.
    pub fn is_cancelled(&self) -> bool {
        !self.ui.current() || !self.source.counter.open()
    }
    /// Non-admitting check for cached native progress before IO/bookkeeping.
    /// Actual primitive entry MUST still call `enter` after physical/Core
    /// proofs. An already-entered cut may check cached DATA, but grants no new
    /// key call and can never pass after its original host/session/source closes.
    pub fn preflight(
        &self,
        original: &NativeChatSourceObservation,
        purpose: NativeChatKeyPurpose,
    ) -> Result<(), NativeChatKeyRefusal> {
        if self.purpose != purpose {
            return Err(NativeChatKeyRefusal::WrongPurpose);
        }
        if !std::ptr::eq(Arc::as_ptr(&self.source), original.original.as_ptr()) {
            return Err(NativeChatKeyRefusal::Foreign);
        }
        if self.is_cancelled() {
            return Err(NativeChatKeyRefusal::Unavailable);
        }
        Ok(())
    }
    /// Call AFTER actual file/source/Core proofs and BEFORE the irreversible
    /// SDK consumed CAS or Core KeyEntered. No SDK, IO, callback or allocation
    /// follows these final atomic checks before that local entry marker.
    ///
    /// Partial-entry refusal retains its End in `ends` too. The caller MUST own
    /// that batch outside all nested Root/native guards, so refusal and unwind
    /// cannot run a waiter wakeup inside those guards. This is not a refund.
    pub fn enter(
        &mut self,
        original: &NativeChatSourceObservation,
        purpose: NativeChatKeyPurpose,
        ends: &mut NativeChatKeyEnds,
    ) -> Result<(), NativeChatKeyRefusal> {
        if self.entered || ends.slots[self.purpose.slot()].is_some() {
            return Err(NativeChatKeyRefusal::AlreadyEntered);
        }
        if self.purpose != purpose {
            return Err(NativeChatKeyRefusal::WrongPurpose);
        }
        if !std::ptr::eq(Arc::as_ptr(&self.source), original.original.as_ptr()) {
            return Err(NativeChatKeyRefusal::Foreign);
        }
        if !Arc::ptr_eq(&self.ui.host, &self.source.host) {
            return Err(NativeChatKeyRefusal::Foreign);
        }
        #[cfg(test)]
        observe_cut_boundary(CutBoundary::BeforePrecheck);
        if self.is_cancelled() {
            return Err(NativeChatKeyRefusal::Unavailable);
        }
        #[cfg(test)]
        observe_cut_boundary(CutBoundary::BeforeUiCounter);
        self.ui.counter.enter()?;
        self.entered = true;
        let end = ends.slots[self.purpose.slot()].insert(KeyEnd {
            ui: self.ui.clone(),
            source: self.source.clone(),
            source_entered: false,
        });
        #[cfg(test)]
        observe_cut_boundary(CutBoundary::AfterUiCounter);
        end.source.counter.enter()?;
        end.source_entered = true;
        #[cfg(test)]
        observe_cut_boundary(CutBoundary::BeforeFinal);
        if !end.ui.current() || !end.source.counter.open() {
            return Err(NativeChatKeyRefusal::Unavailable);
        }
        Ok(())
    }
}

struct KeyEnd {
    ui: Arc<UiSession>,
    source: Arc<Source>,
    source_entered: bool,
}
impl Drop for KeyEnd {
    fn drop(&mut self) {
        if self.source_entered {
            self.source.counter.leave();
        }
        self.ui.counter.leave();
    }
}

/// Fixed-size caller-owned End custody. Construct OUTSIDE the outermost Root
/// guard and keep it there through the call, including refusal/unwind. A Root
/// method borrows it; it must not replace or drop its contents inside a writer.
pub struct NativeChatKeyEnds {
    slots: [Option<KeyEnd>; PURPOSES],
}
impl NativeChatKeyEnds {
    pub fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
        }
    }
}
impl Default for NativeChatKeyEnds {
    fn default() -> Self {
        Self::new()
    }
}

/// Negative-only closed counter drainage. It cannot issue another key cut.
#[derive(Clone)]
pub struct NativeChatKeyDrain {
    original: Arc<Counter>,
}

/// Fixed application-wrapper overlap for one Host/UI generation, including
/// conservative Arc controls and preparation/negative owners. Caller custody,
/// actual waiting tasks, SDK/OS heaps and allocator/RSS are separate invoices.
/// Root adds its record, candidate, fixed retirement batch and registry storage.
pub const NATIVE_CHAT_UI_FIXED_METADATA_BYTES: usize = std::mem::size_of::<Host>()
    + std::mem::size_of::<UiSession>()
    + std::mem::size_of::<Counter>()
    + std::mem::size_of::<NativeChatUiBinding>()
    + std::mem::size_of::<NativeChatUiSessionPreparation>()
    + std::mem::size_of::<NativeChatUiSessionOwner>()
    + std::mem::size_of::<NativeChatUiCommand>()
    + std::mem::size_of::<NativeChatUiCurrentCheck>()
    + std::mem::size_of::<NativeChatKeyDrain>()
    + 8 * 64;

impl NativeChatKeyDrain {
    /// No admitted key work remains on this CLOSED original. Unentered cuts,
    /// workers and metadata may still exist and retain their independent fees.
    /// This is not destruction, source authority or an SDK/physical drain.
    pub fn is_drained(&self) -> bool {
        self.original.state.load(Ordering::SeqCst) == CLOSED
    }
    /// Register before observing zero, avoiding the final-End lost wakeup.
    /// Call only outside every guard that an admitted operation can need.
    pub async fn wait(&self) {
        loop {
            let notified = self.original.drained.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_drained() {
                return;
            }
            notified.await;
        }
    }
}

macro_rules! redacted_debug {
    ($($name:ty),+ $(,)?) => {$(
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($name), " { .. }"))
            }
        }
    )+};
}
redacted_debug!(
    NativeChatUiBinding,
    NativeChatUiSessionOwner,
    NativeChatUiSessionPreparation,
    NativeChatUiCommand,
    NativeChatUiCurrentCheck,
    NativeChatSourceOwner,
    NativeChatSourceObservation,
    NativeChatPreparedKeyCut,
    NativeChatKeyEnds,
    NativeChatKeyDrain,
    NativeChatWarmSdkReopenCut,
    NativeChatSdkRestoreEnds
);

// Deterministic negative scheduling controls only. Neither callbacks nor the
// control inventory exist in a production primitive tail.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum CutBoundary {
    BeforeCounterCas,
    BeforePrecheck,
    BeforeUiCounter,
    AfterUiCounter,
    BeforeFinal,
}
#[cfg(test)]
type CutBoundaryHook = Option<(CutBoundary, Box<dyn FnOnce()>)>;
#[cfg(test)]
thread_local! {
    static CUT_BOUNDARY: std::cell::RefCell<CutBoundaryHook> = const {
        std::cell::RefCell::new(None)
    };
}
#[cfg(test)]
fn observe_cut_boundary(boundary: CutBoundary) {
    let hook = CUT_BOUNDARY.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|(expected, _)| *expected == boundary)
        {
            slot.take().map(|(_, hook)| hook)
        } else {
            None
        }
    });
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
mod tests;
