//! Separately prepaid SDP handoff storage. This is capacity, not permission to run RTC,
//! sign a packet or use a key. The original KeyClaim must admit/retain this owner before
//! the authenticated SDP service can expose it. Existing confirmation Data remains small.
use super::PlanBinding;
use crate::voice_v2::resources::{ByteBudget, Credit, Reservation, ResourceKind, Resources};
use crate::voice_v2::{CallLease, Scope, VoiceError};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use zeroize::Zeroize;

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
// Actual Buffer/Copy/Credit graph plus128B caller bookkeeping is264B on the
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
redacted_debug!(Phase, EffectObserver, Copy, Buffer);
impl EffectObserver {
    /// Allocation identity, not authority. Exact native recovery compares the original
    /// phase; a new phase with equal scope labels is a different preparation.
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(in crate::voice_v2) fn belongs_to(&self, key: &super::EstablishedKeyCustody) -> bool {
        key.belongs_to(&self.0.original)
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
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}
impl Copy {
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
        // Never publish drainage before this actual reservation loan and its pool credit
        // return. Drop may enter Pool, so do it outside the phase state mutex.
        drop(self.credit.take());
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
    pub fn status(&self) -> Status {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .status
    }
    /// Charge before allocating a body or constructing a worker. There is no raw Vec
    /// export or refund/reacquisition path: transfer the SAME owner through the pipeline.
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
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if matches!(state.status, Status::Closing | Status::Drained) {
                return;
            }
            state.status = Status::Closing;
        }
        after_claim();
        self.reserve.close();
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.reserve_closed = true;
        if state.copies == 0 {
            state.status = Status::Drained;
        }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
