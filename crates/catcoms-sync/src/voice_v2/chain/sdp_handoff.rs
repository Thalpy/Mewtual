//! Separately prepaid SDP handoff storage. This is capacity, not permission to run RTC,
//! sign a packet or use a key. The original KeyClaim must admit/retain this owner before
//! the authenticated SDP service can expose it. Existing confirmation Data remains small.
use super::PlanBinding;
use crate::voice_v2::resources::{ByteBudget, Credit, Reservation, ResourceKind, Resources};
use crate::voice_v2::{CallLease, Scope, VoiceError};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use zeroize::Zeroize;
mod native_history;
pub use native_history::{NativeIoClosure, NativeRowFunding, NativeRowPurpose, NATIVE_ROW_BYTES};

/// One original round's native handoff envelope, separate from its crypto reservation.
/// Includes fixed ownership and every simultaneously retained SDK/HTTP/actor copy that
/// belongs to this native phase. Browser-held buffers have a separate browser payment.
pub const BYTES: usize = 64 * 1024;
const METADATA: usize = 4096;
/// Fixed native SDK-custody wrapper/preparation graph paid by the same metadata anchor.
/// Only one candidate may be prepared per original native Session/phase; the adapter's
/// measured graph must include both that candidate and its affine preparation wrapper.
/// This allowance grants no SDK/RTC authority and does not increase the64KiB envelope.
pub const NATIVE_SDK_METADATA_BYTES: usize = 512;
/// Candidate whole native PC-intent graph, including its Root fixed observation. The
/// complete native/Root layout tests must pin this within the unchanged4KiB metadata.
pub const NATIVE_PC_METADATA_BYTES: usize = 1024;
/// Partition the SAME fixed metadata: original Root intent plus its two direction records,
/// and the complete native claim/preparation/hash graph. No envelope or group limit changes.
pub const NATIVE_PC_ROOT_METADATA_BYTES: usize = 736;
pub const NATIVE_PC_ADAPTER_METADATA_BYTES: usize = 288;
const _: () = assert!(
    NATIVE_PC_ROOT_METADATA_BYTES + NATIVE_PC_ADAPTER_METADATA_BYTES <= NATIVE_PC_METADATA_BYTES
);
// Actual Buffer/Copy/Credit/keepalive/IO graph plus128B caller bookkeeping is288B on the
// supported layout. Round its payment to32B; keep the whole64KiB envelope unchanged.
const COPY_METADATA: usize = 288;
const COPIES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Preparing,
    Open,
    Closing,
    Drained,
}
struct State {
    status: Status,
    copies: usize,
    // Only the first close caller may publish this, AFTER Reservation::close returns.
    // A second idempotent close could otherwise overtake the first caller's credit Drop.
    reserve_closed: bool,
    // Two plain fixed fees, never Copy/Row/Publication owners. Their payment survives
    // variable drainage, and exact recovery cannot allocate a replacement allowance.
    history: [Option<Arc<NativeRowFunding>>; 2],
}

pub struct Phase {
    resources: Arc<Resources>,
    scope: Scope,
    state: Mutex<State>,
    reserve: Arc<Reservation>,
    // Last: exported observers retain this small independently paid anchor, never Phase.
    // This avoids a future Phase -> Root grant -> Phase custody cycle.
    effect: Arc<EffectOwner>,
}
struct EffectOwner {
    // A single pure atomic gate also fences native final admission. No watch callback,
    // resource destructor or phase mutex belongs under the original native UI fence.
    gate: AtomicU8, // 0=preparing, 1=armed, 2=irreversibly closed
    original: Arc<PlanBinding>,
    lease: CallLease,
    _metadata: Credit,
}
/// Read-only effect currency. A bare cloned CallLease would let its watch allocation
/// outlive the Phase payment. This observer retains that exact fixed payment to last Drop.
/// It owns no phase, buffer, SDK promise, crypto lane or native parent row.
pub struct EffectObserver(Arc<EffectOwner>);
redacted_debug!(Phase, EffectObserver, Copy, Buffer, TaskCopy);
impl EffectObserver {
    /// Allocation identity, not authority. Exact native recovery compares the original
    /// phase; a new phase with equal scope labels is a different preparation.
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(in crate::voice_v2) fn belongs_to(&self, key: &super::EstablishedKeyCustody) -> bool {
        key.belongs_to(&self.0.original)
    }
    /// Wait on this original phase's actual close notification. The pure request_close
    /// gate refuses effects immediately; its affine native CloseTicket performs this
    /// notification outside final guards. Retaining the observer keeps the watch paid.
    pub(in crate::voice_v2) async fn closed(&self) {
        if self.is_live() {
            self.0.lease.wait_revoked().await;
        }
    }
    pub fn is_live(&self) -> bool {
        self.0.gate.load(Ordering::Acquire) == 1
            && original_current(&self.0.original)
            && self.0.lease.is_live()
    }
}
fn original_current(original: &PlanBinding) -> bool {
    original.value.live()
        && original.transition.epoch_is_live()
        && !original.successor_closed.load(Ordering::Acquire)
}
/// Affine contribution for one actual worker/copy. Closing never refunds this owner's
/// storage. Its buffer must drop first; only then can this contribution report drainage.
pub struct Copy {
    credit: Option<Credit>,
    phase: Arc<Phase>,
    capacity: usize,
    // Method-free native lifetime custody, never effect authority. Actual SDP RPCs use
    // this to retain their admitted UI commit + unique physical request through queued
    // Actor work and returned results after the submitting worker disappears. A separate
    // task Copy pays that native wrapper; NEVER attach the wrapper to its own funding Copy
    // (Publication -> funding Copy -> Publication would prevent every last Drop).
    keepalive: Option<catcoms_rt::SharedRequestKeepalive>,
    // LAST: independently paid concrete counter; never a callback or Row/Phase backedge.
    history: Option<Arc<NativeRowFunding>>,
}
/// Separate native publication/task funding. Unlike an RPC/body Copy, this owner cannot
/// adopt a payload or attach a keepalive: the native publication wrapper may own it without
/// being able to create a Publication -> funding Copy -> Publication self-cycle.
///
/// ```compile_fail
/// use catcoms_sync::voice_v2::chain::sdp_handoff::TaskCopy;
/// fn attach(payment: &mut TaskCopy, custody: catcoms_rt::SharedRequestKeepalive) {
///     payment.attach_keepalive(custody).unwrap();
/// }
/// ```
pub struct TaskCopy(Copy);
impl TaskCopy {
    pub fn uses_intent(&self, intent: &crate::voice_v2::sdp::InitialSdpIntent) -> bool {
        self.0.uses_intent(intent)
    }
    pub fn covers_work(&self, bytes: usize) -> bool {
        self.0.covers_work(bytes)
    }
    /// Concrete paid collection only. Unlike a generic keepalive this fixed funding
    /// cannot retain TaskCopy, a native publication or a Row, so it creates no self-cycle.
    pub fn bind_native_history(
        &mut self,
        funding: Arc<NativeRowFunding>,
    ) -> Result<(), VoiceError> {
        self.0.bind_native_history(funding)
    }
}
pub struct Buffer {
    bytes: Vec<u8>,
    // Field order is intentional: the allocation returns before last-owner publication.
    _copy: Copy,
}
impl Drop for Buffer {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}
impl Buffer {
    pub fn bind_native_history(
        &mut self,
        funding: Arc<NativeRowFunding>,
    ) -> Result<(), VoiceError> {
        self._copy.bind_native_history(funding)
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
    /// Original backing capacity, not just the visible decoded slice. Native Actor wrappers
    /// must include their separately measured metadata in this same Copy before allocation.
    pub fn capacity(&self) -> usize {
        self.bytes.capacity()
    }
    pub fn covers_work(&self, bytes: usize) -> bool {
        self._copy.covers_work(bytes)
    }
    pub fn uses_intent(&self, intent: &crate::voice_v2::sdp::InitialSdpIntent) -> bool {
        self._copy.uses_intent(intent)
    }
    pub fn has_keepalive(&self) -> bool {
        self._copy.has_keepalive()
    }
    /// Attach only while this affine buffer is uniquely owned, before lending it to Actor
    /// work. It cannot be replaced/removed by a retry. No callback is invoked here; failed
    /// attachment and eventual destruction must still run outside native/Root guards.
    pub fn attach_keepalive(
        &mut self,
        keepalive: catcoms_rt::SharedRequestKeepalive,
    ) -> Result<(), VoiceError> {
        self._copy.attach_keepalive(keepalive)
    }
}
impl Copy {
    /// Bind once to the actual original phase's fixed native-row counter. Native must
    /// select the SAME stored Seal/Open purpose; this proves payment, not authorization.
    pub fn bind_native_history(
        &mut self,
        funding: Arc<NativeRowFunding>,
    ) -> Result<(), VoiceError> {
        if self.history.is_some() {
            return Err(VoiceError::RetryMismatch);
        }
        if !funding.uses_phase(&self.phase) {
            return Err(VoiceError::Unauthorized);
        }
        funding.acquire_copy()?;
        self.history = Some(funding);
        Ok(())
    }
    /// Allocation identity only, even after Close. This grants no fresh SDK/packet/RTC use.
    pub fn uses_intent(&self, intent: &crate::voice_v2::sdp::InitialSdpIntent) -> bool {
        intent.uses_phase(&self.phase)
    }
    pub fn has_keepalive(&self) -> bool {
        self.keepalive.is_some()
    }
    /// Native supplies an actual independently paid publication custodian, not a renderer
    /// descriptor. This is a one-way drain edge, never proof of current authorization. The
    /// custodian's own funding Copy MUST NOT receive this same custodian as its keepalive.
    pub fn attach_keepalive(
        &mut self,
        keepalive: catcoms_rt::SharedRequestKeepalive,
    ) -> Result<(), VoiceError> {
        if self.keepalive.is_some() {
            return Err(VoiceError::RetryMismatch);
        }
        self.keepalive = Some(keepalive);
        Ok(())
    }
    /// Exact phase identity for an affine Actor command owner. This proves payment
    /// provenance only; current Root/native effect admission remains independently required.
    pub fn matches_effect(&self, effect: &EffectObserver) -> bool {
        self.phase.contains_effect(effect)
    }
    /// Backing allowance of this same affine contribution. Public typed RPCs must reject
    /// underpayment before allocating their mailbox/result graph, not only match identity.
    pub fn covers_work(&self, bytes: usize) -> bool {
        bytes <= self.capacity
    }
    /// Only adopt storage for which this SAME contribution was reserved before allocation
    /// or provider entry. Backing capacity, rather than visible length, is the bound.
    pub fn adopt(self, mut bytes: Vec<u8>) -> Result<Buffer, VoiceError> {
        if bytes.capacity() > self.capacity {
            bytes.zeroize();
            drop(bytes);
            return Err(VoiceError::Bounds);
        }
        Ok(Buffer { bytes, _copy: self })
    }
}
impl Drop for Copy {
    fn drop(&mut self) {
        self.retire(|| {});
    }
}
/// Affine finally for credit, Phase position and fixed collection counter. A trusted
/// publication destructor can unwind too; collection must never overtake credit/position
/// retirement in that path. This guard borrows Phase and owns no variable backedge.
struct Retirement<'a> {
    phase: &'a Phase,
    credit: Option<Credit>,
    collection: Option<native_history::Collection>,
}
impl Retirement<'_> {
    fn retire_phase(&mut self) {
        let Some(credit) = self.credit.take() else {
            return;
        };
        drop(credit);
        let mut state = self
            .phase
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.copies = state.copies.checked_sub(1).expect("one original SDP copy");
        if state.status == Status::Closing && state.reserve_closed && state.copies == 0 {
            state.status = Status::Drained;
        }
    }
    fn finish(mut self, after_phase: impl FnOnce()) {
        self.retire_phase();
        after_phase(); // All Phase guards are already gone, even on test-hook unwind.
    }
}
impl Drop for Retirement<'_> {
    fn drop(&mut self) {
        self.retire_phase();
        drop(self.collection.take()); // Notify only after actual credit/count retirement.
    }
}
impl Copy {
    // The production hook is empty. A deterministic test can park the real after-Phase /
    // before-native-counter gap without synthetic counters or enlarging any allowance.
    fn retire(&mut self, after_phase: impl FnOnce()) {
        let Some(credit) = self.credit.take() else {
            return;
        };
        let retirement = Retirement {
            phase: &self.phase,
            credit: Some(credit),
            collection: self.history.take().map(native_history::Collection::new),
        };
        // Buffer's bytes have already zeroized and deallocated. Releasing this keepalive
        // reference can run its last native Drop and enter Row locks, so do it while this
        // credit is paid, outside Phase/Root/native guards. If a worker still owns that
        // publication, its separate TaskCopy pays the later last Drop and blocks drainage.
        drop(self.keepalive.take());
        retirement.finish(after_phase);
    }
}
impl Phase {
    /// Prepare inert capacity before the final native UI/selection/session admission.
    /// A failed preparation creates no effect lease, body buffer or replacement allowance.
    pub(super) fn prepare(original: Arc<PlanBinding>) -> Result<Arc<Self>, VoiceError> {
        if !original_current(&original) {
            return Err(VoiceError::Unauthorized);
        }
        let resources = original.resources().clone();
        let scope = original.scope;
        let reserve = resources.reserve(scope.group, BYTES)?;
        let metadata = ByteBudget::Reserved(reserve.clone()).charge(
            &resources,
            scope.group,
            METADATA,
            ResourceKind::Retained,
        )?;
        Ok(Arc::new(Self {
            resources,
            scope,
            effect: Arc::new(EffectOwner {
                gate: AtomicU8::new(0),
                original,
                lease: CallLease::default(),
                _metadata: metadata,
            }),
            state: Mutex::new(State {
                status: Status::Preparing,
                copies: 0,
                reserve_closed: false,
                history: [None, None],
            }),
            reserve,
        }))
    }
    /// Pure nonblocking tail under the original native admission. The KeyClaim owns the
    /// session/proof checks; this method alone supplies neither identity nor authority.
    pub fn arm(&self) -> Result<(), VoiceError> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        if !original_current(&self.effect.original)
            || !self.effect.lease.is_live()
            || !matches!(state.status, Status::Preparing | Status::Open)
        {
            return Err(VoiceError::Unauthorized);
        }
        match self
            .effect
            .gate
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) | Err(1) => {
                state.status = Status::Open;
                Ok(())
            }
            Err(_) => Err(VoiceError::Unauthorized),
        }
    }
    pub fn contains_effect(&self, effect: &EffectObserver) -> bool {
        Arc::ptr_eq(&self.effect, &effect.0)
    }
    pub fn effect(&self) -> EffectObserver {
        EffectObserver(self.effect.clone())
    }
    pub(super) fn belongs_to(&self, binding: &Arc<PlanBinding>) -> bool {
        Arc::ptr_eq(&self.effect.original, binding)
    }
    pub fn uses(&self, resources: &Arc<Resources>, scope: Scope) -> bool {
        Arc::ptr_eq(&self.resources, resources) && self.scope == scope
    }
    /// Nonblocking observation for native finalization under its ordered try-lock fence.
    pub fn try_status(&self) -> Result<Status, VoiceError> {
        match self.state.try_lock() {
            Ok(state) => Ok(state.status),
            Err(std::sync::TryLockError::Poisoned(error)) => Ok(error.into_inner().status),
            Err(std::sync::TryLockError::WouldBlock) => Err(VoiceError::Capacity),
        }
    }
    /// Stronger whole-native-IO observation. Storage can be Drained in the intentional gap
    /// after Copy credit/count retirement but before its fixed history counter retires.
    /// Native Final/GC must also require this predicate plus its worker/SDK/engine gates.
    /// This performs only a try-lock and atomic loads, with no callbacks or owner drops.
    pub fn try_is_collected(&self) -> Result<bool, VoiceError> {
        let state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        Ok(state.status == Status::Drained
            && state
                .history
                .iter()
                .flatten()
                .all(|funding| funding.is_collected()))
    }
    pub fn status(&self) -> Status {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .status
    }
    /// Charge before allocating a body or constructing a worker. There is no raw Vec
    /// export or refund/reacquisition path: transfer the SAME owner through the pipeline.
    /// Independent task/publication payment, deliberately unable to retain its own native
    /// publication wrapper. It consumes the same original byte/count allowance as any Copy.
    pub fn task_copy(self: &Arc<Self>, bytes: usize) -> Result<TaskCopy, VoiceError> {
        self.copy(bytes).map(TaskCopy)
    }
    pub fn copy(self: &Arc<Self>, bytes: usize) -> Result<Copy, VoiceError> {
        if bytes == 0 {
            return Err(VoiceError::Bounds);
        }
        let charged = bytes.checked_add(COPY_METADATA).ok_or(VoiceError::Bounds)?;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if self.effect.gate.load(Ordering::Acquire) != 1
            || state.status != Status::Open
            || !original_current(&self.effect.original)
            || !self.effect.lease.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        if state.copies == COPIES {
            return Err(VoiceError::Capacity);
        }
        let credit = ByteBudget::Reserved(self.reserve.clone()).charge(
            &self.resources,
            self.scope.group,
            charged,
            ResourceKind::Retained,
        )?;
        state.copies += 1;
        Ok(Copy {
            credit: Some(credit),
            phase: self.clone(),
            capacity: bytes,
            keepalive: None,
            history: None,
        })
    }
    /// Close effects first and return ONLY unused capacity outside native/phase guards.
    /// Actual buffers, workers and uncollected replies continue owning their exact loans.
    /// Pure irreversible refusal bit for KeyRelease's native admission fence. Move an
    /// actual Phase owner out of all native guards before calling close for notification
    /// and resource retirement. Observers and new copies refuse immediately in this gap.
    pub fn request_close(&self) {
        self.effect.gate.store(2, Ordering::Release);
    }
    pub fn close(&self) {
        self.close_with(|| {});
    }
    // The production callback is empty. A deterministic test parks the actual close
    // leader before reserve retirement, without inventing completion or changing time.
    fn close_with(&self, after_claim: impl FnOnce()) {
        self.request_close();
        self.effect.lease.revoke();
        let closures = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if matches!(state.status, Status::Closing | Status::Drained) {
                return;
            }
            state.status = Status::Closing;
            state
                .history
                .each_ref()
                .map(|history| history.as_ref().map(|fee| fee.close_io()))
        };
        after_claim();
        self.reserve.close();
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.reserve_closed = true;
        if state.copies == 0 {
            state.status = Status::Drained;
        }
        drop(state);
        drop(closures); // Notify only after every Phase guard/resource-close operation.
    }
}
impl Drop for Phase {
    fn drop(&mut self) {
        // An abandoned last native owner cannot leave Root/SDK observers live or keep
        // unused variable capacity behind a fixed historical observer. No Copy can remain
        // here: every actual Copy itself owns a strong Phase reference.
        self.request_close();
        self.effect.lease.revoke();
        self.reserve.close();
        let closures = {
            let state = self
                .state
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            state
                .history
                .each_ref()
                .map(|history| history.as_ref().map(|fee| fee.close_io()))
        };
        drop(closures); // No Phase/native guard exists in this exclusive last-owner path.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod native_history;
    struct Alive {
        _actor: crate::voice_v2::runtime::ActorOwner,
        _runtime: Arc<crate::voice_v2::runtime::Runtime>,
        baseline: usize,
    }
    fn fixture() -> (Arc<Resources>, Scope, CallLease, Arc<Phase>, Alive) {
        use crate::voice_v2::crypto_actions::slots;
        use crate::voice_v2::runtime::{Runtime, View};
        use crate::voice_v2::Context;
        let runtime = Runtime::new(Arc::new(catcoms_rt::ManualClock::new(1_000)));
        let bound = super::super::tests::binding(1);
        let scope = bound.scope;
        let call = bound.call.clone();
        let ticket = runtime.session_ticket();
        runtime.activate_session(&ticket).unwrap();
        let (mut actor, registration) = runtime
            .prepare_actor(scope.group, bound.local, &ticket)
            .unwrap();
        registration.register().unwrap();
        actor
            .sync_view(View {
                epoch: bound.epoch,
                members: [bound.local, bound.peer].into(),
                policy: scope.policy,
                eligible: true,
                indirect: true,
            })
            .unwrap();
        let context = Context {
            scope,
            local: bound.local,
            epoch: bound.epoch,
            now: 1_000,
            indirect: true,
        };
        let resources = runtime.resources();
        let chain = super::super::tests::prepare_chain(
            resources.clone(),
            bound,
            Arc::new(slots::Slots::default()),
        )
        .unwrap();
        chain
            .prepare_finish_window(&actor, &context)
            .unwrap()
            .publish_at(
                1_000,
                super::super::FinishIdentity {
                    message: [3; 32],
                    hash: [4; 32],
                },
            )
            .unwrap();
        let original = chain.binding.clone();
        drop(chain); // Actual old progress returns; only the original fixed proof remains.
        let phase = Phase::prepare(original).unwrap();
        // The epoch observation retains actual native gates. Keep the runtime/actor in
        // scope throughout each test: dropping either actually revokes native authority.
        let baseline = resources.counts().0 - BYTES;
        (
            resources,
            scope,
            call,
            phase,
            Alive {
                _actor: actor,
                _runtime: runtime,
                baseline,
            },
        )
    }
    #[test]
    fn fixed_phase_and_copy_graphs_fit_their_original_declared_metadata() {
        use std::mem::size_of;
        let fixed = size_of::<Phase>() + 2 * size_of::<usize>()
            + size_of::<Reservation>() + 2 * size_of::<usize>()
            + size_of::<EffectOwner>() + 2 * size_of::<usize>()
            // Actual independent effect-lease/watch and conservative complete map node.
            // The original call/pool already have their own payments.
            + 256 + 512 + NATIVE_SDK_METADATA_BYTES + NATIVE_PC_METADATA_BYTES;
        assert!(fixed <= METADATA, "SDP fixed graph {fixed}");
        let copy = size_of::<Buffer>() + 128;
        assert!(copy <= COPY_METADATA, "SDP copy graph {copy}");
        assert!(
            size_of::<Retirement<'_>>() <= 128,
            "SDP retirement bookkeeping"
        );
    }
    #[test]
    fn original_sdp_publication_custody_drops_before_credit_and_last_phase_drainage() {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        #[derive(Debug)]
        struct Publication {
            phase: std::sync::Weak<Phase>,
            resources: Arc<Resources>,
            expected: Arc<AtomicUsize>,
            dropped: Arc<AtomicBool>,
            // Separate payment. Attaching this Publication to THIS task Copy would make
            // a cycle; only the distinct body/RPC Copy holds the Publication keepalive.
            _task: TaskCopy,
        }
        impl Drop for Publication {
            fn drop(&mut self) {
                let phase = self.phase.upgrade().unwrap();
                assert!(
                    phase.state.try_lock().is_ok(),
                    "publication Drop beneath Phase"
                );
                assert_eq!(
                    phase.status(),
                    Status::Closing,
                    "native custody precedes Drained"
                );
                assert_eq!(
                    self.resources.counts().0,
                    self.expected.load(Ordering::Acquire),
                    "actual native custody destruction must remain paid"
                );
                self.dropped.store(true, Ordering::Release);
            }
        }
        for worker_first in [false, true] {
            let (resources, _, _, phase, _alive) = fixture();
            phase.arm().unwrap();
            let expected = Arc::new(AtomicUsize::new(0));
            let dropped = Arc::new(AtomicBool::new(false));
            let task = phase.task_copy(1024).unwrap();
            assert!(task.covers_work(1024));
            let publication = Arc::new(Publication {
                phase: Arc::downgrade(&phase),
                resources: resources.clone(),
                expected: expected.clone(),
                dropped: dropped.clone(),
                _task: task,
            });
            let mut buffer = phase.copy(8192).unwrap().adopt(vec![3; 8192]).unwrap();
            buffer.attach_keepalive(publication.clone()).unwrap();
            assert!(buffer.has_keepalive());
            phase.close();
            assert_eq!(phase.status(), Status::Closing);
            if worker_first {
                drop(publication);
                assert!(!dropped.load(Ordering::Acquire));
                expected.store(resources.counts().0, Ordering::Release);
                drop(buffer);
            } else {
                drop(buffer);
                assert!(!dropped.load(Ordering::Acquire));
                assert_eq!(phase.status(), Status::Closing);
                expected.store(resources.counts().0, Ordering::Release);
                drop(publication);
            }
            assert!(dropped.load(Ordering::Acquire));
            assert_eq!(phase.status(), Status::Drained);
            assert_eq!(resources.counts().0, METADATA + _alive.baseline);
        }
    }
    #[test]
    fn original_sdp_copy_cannot_replace_its_publication_custodian() {
        use std::sync::atomic::AtomicBool;
        #[derive(Debug)]
        struct Witness(Arc<AtomicBool>);
        impl Drop for Witness {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let (_, _, _, phase, _alive) = fixture();
        phase.arm().unwrap();
        let first = Arc::new(AtomicBool::new(false));
        let other = Arc::new(AtomicBool::new(false));
        let mut copy = phase.copy(1024).unwrap();
        copy.attach_keepalive(Arc::new(Witness(first.clone())))
            .unwrap();
        assert_eq!(
            copy.attach_keepalive(Arc::new(Witness(other.clone()))),
            Err(VoiceError::RetryMismatch)
        );
        assert!(!first.load(Ordering::Acquire));
        assert!(other.load(Ordering::Acquire));
        phase.close();
        assert_eq!(phase.status(), Status::Closing);
        drop(copy);
        assert!(first.load(Ordering::Acquire));
        assert_eq!(phase.status(), Status::Drained);
    }
    #[test]
    fn original_sdp_buffers_remain_paid_after_close_until_their_actual_last_drop() {
        let (resources, scope, _, phase, _alive) = fixture();
        assert_eq!(resources.counts().0, BYTES + _alive.baseline);
        assert!(phase.copy(8192).is_err());
        phase.arm().unwrap();
        let mut first = phase.copy(8192).unwrap().adopt(vec![3; 8192]).unwrap();
        first.bytes_mut()[0] = 9;
        assert_eq!(first.bytes()[0], 9);
        let second = phase.copy(8192).unwrap().adopt(vec![4; 8192]).unwrap();
        let pressure = resources
            .charge(
                scope.group,
                crate::voice_v2::resources::GROUP_BYTES - resources.counts().0,
                ResourceKind::Retained,
            )
            .unwrap();
        assert!(phase.uses(&resources, scope));
        let effect = phase.effect();
        phase.close();
        assert!(!effect.is_live());
        assert_eq!(phase.status(), Status::Closing);
        assert!(phase.copy(1).is_err());
        assert!(phase.arm().is_err());
        let retained = resources.counts().0;
        phase.close();
        assert_eq!(resources.counts().0, retained);
        drop(first);
        assert_eq!(phase.status(), Status::Closing);
        assert_eq!(resources.counts().0, retained - 8192 - COPY_METADATA);
        drop(second);
        assert_eq!(phase.status(), Status::Drained);
        assert_eq!(resources.counts().0, retained - 2 * (8192 + COPY_METADATA));
        drop((phase, pressure));
        assert_eq!(
            resources.counts().0,
            METADATA + _alive.baseline,
            "exported observer remains paid"
        );
        drop(effect);
        assert_eq!(
            resources.counts(),
            (_alive.baseline - super::super::METADATA_BYTES, 0)
        );
        drop(_alive);
        assert_eq!(resources.counts(), (0, 0));
    }
    #[test]
    fn sdp_reservation_and_copy_limits_refuse_without_rearming_or_accepting_oversized_storage() {
        let (resources, _, call, phase, _alive) = fixture();
        phase.arm().unwrap();
        assert!(phase.copy(0).is_err());
        assert!(phase.copy(usize::MAX).is_err());
        let mut copies = Vec::new();
        for _ in 0..COPIES {
            copies.push(phase.copy(1).unwrap());
        }
        assert!(phase.copy(1).is_err());
        drop(copies);
        assert_eq!(resources.counts().0, BYTES + _alive.baseline);
        assert!(phase.copy(1).unwrap().adopt(Vec::with_capacity(2)).is_err());
        assert_eq!(resources.counts().0, BYTES + _alive.baseline);
        call.revoke();
        assert!(phase.copy(1).is_err());
        assert!(phase.arm().is_err());
        phase.close();
        assert_eq!(phase.status(), Status::Drained);
        assert_eq!(resources.counts().0, METADATA + _alive.baseline);
        drop(phase);
        assert_eq!(
            resources.counts(),
            (_alive.baseline - super::super::METADATA_BYTES, 0)
        );
        drop(_alive);
        assert_eq!(resources.counts(), (0, 0));
    }
    #[test]
    fn second_close_and_last_copy_cannot_overtake_actual_reservation_close() {
        let (resources, _, _, phase, _alive) = fixture();
        phase.arm().unwrap();
        let copy = phase.copy(8192).unwrap();
        let claimed = Arc::new(std::sync::Barrier::new(2));
        let finish = Arc::new(std::sync::Barrier::new(2));
        let leader = {
            let phase = phase.clone();
            let claimed = claimed.clone();
            let finish = finish.clone();
            std::thread::spawn(move || {
                phase.close_with(|| {
                    claimed.wait();
                    finish.wait();
                })
            })
        };
        claimed.wait();
        phase.close(); // Idempotent follower cannot publish the leader's completion.
        drop(copy); // Nor can last-copy Drop publish before reserve retirement returns.
        assert_eq!(phase.status(), Status::Closing);
        assert_eq!(resources.counts().0, BYTES + _alive.baseline);
        finish.wait();
        leader.join().unwrap();
        assert_eq!(phase.status(), Status::Drained);
        assert_eq!(resources.counts().0, METADATA + _alive.baseline);
        drop(phase);
        assert_eq!(
            resources.counts(),
            (_alive.baseline - super::super::METADATA_BYTES, 0)
        );
        drop(_alive);
        assert_eq!(resources.counts(), (0, 0));
    }
    #[test]
    fn exported_effect_observer_keeps_its_actual_fixed_allocation_paid_after_phase_drop() {
        let (resources, _, _, phase, _alive) = fixture();
        phase.arm().unwrap();
        let observer = phase.effect();
        assert!(observer.is_live());
        phase.close();
        assert!(!observer.is_live());
        drop(phase);
        assert_eq!(resources.counts().0, METADATA + _alive.baseline);
        drop(observer);
        assert_eq!(
            resources.counts(),
            (_alive.baseline - super::super::METADATA_BYTES, 0)
        );
        drop(_alive);
        assert_eq!(resources.counts(), (0, 0));
    }
    #[test]
    fn preparing_observer_is_inert_and_abandoned_phase_returns_only_unused_capacity() {
        let (resources, _, _, phase, _alive) = fixture();
        let observer = phase.effect();
        assert!(!observer.is_live());
        phase.arm().unwrap();
        assert!(observer.is_live());
        phase.request_close();
        assert!(!observer.is_live());
        assert!(phase.copy(1).is_err());
        assert!(phase.arm().is_err());
        assert_eq!(
            resources.counts().0,
            BYTES + _alive.baseline,
            "pure close intent does not refund"
        );
        drop(phase);
        assert!(!observer.is_live());
        assert_eq!(resources.counts().0, METADATA + _alive.baseline);
        drop(observer);
        assert_eq!(
            resources.counts(),
            (_alive.baseline - super::super::METADATA_BYTES, 0)
        );
        drop(_alive);
        assert_eq!(resources.counts(), (0, 0));
    }
    #[test]
    fn dropping_unarmed_preparation_keeps_only_exported_observers_fixed_payment() {
        let (resources, _, _, phase, _alive) = fixture();
        let observer = phase.effect();
        drop(phase);
        assert!(!observer.is_live());
        assert_eq!(resources.counts().0, METADATA + _alive.baseline);
        drop(observer);
        assert_eq!(
            resources.counts(),
            (_alive.baseline - super::super::METADATA_BYTES, 0)
        );
        drop(_alive);
        assert_eq!(resources.counts(), (0, 0));
    }
    #[test]
    fn equal_labels_do_not_substitute_the_original_plan_allocation() {
        let (resources, _, _, phase, _alive) = fixture();
        let replacement = super::super::tests::prepare_chain(
            resources.clone(),
            phase.effect.original.value.clone(),
            Arc::new(crate::voice_v2::crypto_actions::slots::Slots::default()),
        )
        .unwrap();
        assert!(phase.effect.original.value.same(&replacement.binding.value));
        assert!(!phase.belongs_to(&replacement.binding));
        assert!(phase.belongs_to(&phase.effect.original));
    }
    #[test]
    fn original_native_actor_or_whole_plan_retirement_fences_prepared_storage() {
        for armed in [false, true] {
            let (_, _, _, phase, alive) = fixture();
            if armed {
                phase.arm().unwrap();
            }
            drop(alive); // Actual actor/session retirement, not an injected current boolean.
            assert!(!phase.effect().is_live());
            assert!(phase.arm().is_err());
            assert!(phase.copy(1).is_err());
            phase.close();
            assert_eq!(phase.status(), Status::Drained);
        }
        let (_, _, _, phase, _alive) = fixture();
        phase
            .effect
            .original
            .successor_closed
            .store(true, Ordering::Release);
        assert!(phase.arm().is_err());
        assert!(phase.copy(1).is_err());
    }
    #[test]
    fn epoch_replacement_fences_storage_before_and_after_arm_without_revoking_the_call() {
        for armed in [false, true] {
            let (_, _, call, phase, mut alive) = fixture();
            if armed {
                phase.arm().unwrap();
            }
            let bound = &phase.effect.original.value;
            alive
                ._actor
                .sync_view(crate::voice_v2::runtime::View {
                    epoch: bound.epoch + 1,
                    members: [bound.local, bound.peer].into(),
                    policy: bound.scope.policy,
                    eligible: true,
                    indirect: true,
                })
                .unwrap();
            assert!(
                call.is_live(),
                "test isolates original epoch observation from call End"
            );
            assert!(!phase.effect().is_live());
            assert!(phase.arm().is_err());
            assert!(phase.copy(1).is_err());
            assert!(Phase::prepare(phase.effect.original.clone()).is_err());
        }
    }
}
