//! One confirmation-phase data loan, shared across its original native action rows. Physical
//! exchange positions are separate: holding one through logical settlement would deadlock the
//! subsequent result/settlement requests. Completed worker/result owners must retain their copy
//! leases until collection; browser input/engine drainage is a separate selected-executor gate.
use catcoms_sync::voice_v2::crypto_actions::{
    CryptoAction, CryptoInput, CryptoInputLease, HandoffLoan, ProtectedCryptoBinding,
};
use catcoms_sync::voice_v2::resources::{Credit, ResourceKind, Resources};
use catcoms_sync::voice_v2::{CallLease, Scope, VoiceError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use zeroize::Zeroize;

const METADATA_BYTES: usize = 2048;
// Twelve protected rows plus four serial intake/serialization/result owners. Refusal occurs
// before copies/work are created; the future channel must keep its physical lane independent.
const COPY_OWNERS: usize = 16;

/// Pay fixed metadata before invoking a Root factory. This is independent of variable copy
/// capacity and remains paid with historical phase receipts after Workflow/inputs have drained.
pub(crate) struct PreparedPhase {
    resources: Arc<Resources>,
    scope: Scope,
    call: CallLease,
    metadata: Credit,
}
impl PreparedPhase {
    pub(crate) fn new(
        resources: Arc<Resources>,
        scope: Scope,
        call: CallLease,
    ) -> Result<Self, VoiceError> {
        if !call.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let metadata = resources.charge(scope.group, METADATA_BYTES, ResourceKind::Retained)?;
        Ok(Self {
            resources,
            scope,
            call,
            metadata,
        })
    }
    /// The actual Root-issued action supplies the sole variable reservation. No pending row
    /// or serialized renderer descriptor can manufacture this capability. Before any browser
    /// projection is exposed, install the returned phase in the process-owned native ledger:
    /// a UI/invoke waiter must never own its sole Arc. Unregistered abandonment is safe only
    /// while no input/provider work has been exposed. This primitive does not enforce that
    /// cross-module registration boundary or itself prove browser-side drainage.
    pub(crate) fn bind(
        self,
        action: &CryptoAction,
        bytes: usize,
    ) -> Result<Arc<Phase>, VoiceError> {
        if !action.belongs_to(self.scope, &self.call) || !action.uses_resources(&self.resources) {
            return Err(VoiceError::Unauthorized);
        }
        let binding = action.protected_binding()?;
        let loan = action.borrow_handoff(bytes)?;
        Ok(Arc::new(Phase {
            resources: self.resources,
            scope: self.scope,
            call: self.call,
            binding,
            capacity: bytes,
            state: Mutex::new(State {
                stage: Stage::Open,
                copies: 0,
                bytes: 0,
                loan: Some(loan),
            }),
            drained: AtomicBool::new(false),
            changed: Notify::new(),
            _metadata: self.metadata,
        }))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Open,
    Closing,
    Draining,
    Drained,
}
struct State {
    stage: Stage,
    copies: usize,
    bytes: usize,
    loan: Option<HandoffLoan>,
}
/// This fixed owner has no strong Action/Input references. Drained history can keep it alive
/// without retaining the original Workflow. Do not mistake this capacity binding for first-
/// effect authority: every actual row still needs its original registered Action and CallGrant.
pub(crate) struct Phase {
    resources: Arc<Resources>,
    scope: Scope,
    call: CallLease,
    binding: ProtectedCryptoBinding,
    capacity: usize,
    state: Mutex<State>,
    drained: AtomicBool,
    changed: Notify,
    _metadata: Credit,
}
pub(crate) struct CopyCustody {
    phase: Arc<Phase>,
    bytes: usize,
}
pub(crate) struct InputCustody {
    input: CryptoInputLease,
    // Field order matters: the native input owner drops before its phase-drain contribution.
    _custody: CopyCustody,
}
impl InputCustody {
    pub(crate) fn input(&self) -> CryptoInput<'_> {
        self.input.as_input()
    }
}
/// No extraction/growth API: the caller cannot move a live Vec away from its prepaid owner.
pub(crate) struct Buffer {
    bytes: Vec<u8>,
    // Drop actual buffer before custody publishes last-owner drainage.
    _custody: CopyCustody,
}
impl Buffer {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        // Best effort within this actual owned buffer, before its custody can report drainage.
        // This makes no promise about allocator slack, provider temporaries or OS/SDK copies.
        self.bytes.zeroize();
    }
}
impl CopyCustody {
    /// Provider output must have been prepaid before invoking that provider. Reject an
    /// unexpectedly larger backing allocation without ever handing it to another worker.
    pub(crate) fn adopt(self, mut bytes: Vec<u8>) -> Result<Buffer, VoiceError> {
        if bytes.capacity() > self.bytes {
            bytes.zeroize();
            drop(bytes); // actual output drains before the refusing custody owner's Drop
            return Err(VoiceError::Bounds);
        }
        Ok(Buffer {
            bytes,
            _custody: self,
        })
    }
}
impl Phase {
    pub(crate) fn copy(self: &Arc<Self>, bytes: usize) -> Result<CopyCustody, VoiceError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.stage != Stage::Open {
            return Err(VoiceError::Unauthorized);
        }
        let total = state.bytes.checked_add(bytes).ok_or(VoiceError::Bounds)?;
        if total > self.capacity || state.copies == COPY_OWNERS {
            return Err(VoiceError::Capacity);
        }
        state.bytes = total;
        state.copies += 1;
        Ok(CopyCustody {
            phase: self.clone(),
            bytes,
        })
    }
    pub(crate) fn buffer(self: &Arc<Self>, bytes: usize) -> Result<Buffer, VoiceError> {
        let custody = self.copy(bytes)?;
        custody.adopt(vec![0; bytes])
    }
    pub(crate) fn input(
        self: &Arc<Self>,
        action: &CryptoAction,
    ) -> Result<InputCustody, VoiceError> {
        if !action.belongs_to(self.scope, &self.call)
            || !action.uses_resources(&self.resources)
            || action.protected_binding()? != self.binding
        {
            return Err(VoiceError::Unauthorized);
        }
        // Metadata equality is copy-capacity bookkeeping only. The registry independently
        // resolves the actual Root-produced row before calling this method. Native input
        // already owns its own paid payload, but still contributes to phase drainage.
        let custody = self.copy(0)?;
        let input = action.input()?;
        Ok(InputCustody {
            input,
            _custody: custody,
        })
    }
    pub(crate) fn is_drained(&self) -> bool {
        self.drained.load(Ordering::Acquire)
    }
    /// The outer serial control-flight owner coalesces the sole cleanup waiter. Its Notified
    /// complete future is prepaid in fixed metadata; do not expose this as unbounded invoke
    /// waiters. The layout test includes this async wrapper, not merely Notify's inner future.
    pub(crate) async fn wait_drained(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.is_drained() {
                return;
            }
            changed.await;
        }
    }
    /// Call only after selected-browser phase/input/engine/RPC drainage and original native
    /// row settlement are established. This fences new copies immediately; actual existing
    /// native input, buffers and completed/uncollected workers still hold the variable loan.
    /// The separately paid control flight must own the proof's eventual physical response.
    pub(crate) fn close(&self) {
        let loan = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stage == Stage::Open {
                state.stage = Stage::Closing;
            }
            Self::take_draining(&mut state)
        };
        self.finish_draining(loan);
    }
    fn take_draining(state: &mut State) -> Option<HandoffLoan> {
        if state.stage == Stage::Closing && state.copies == 0 {
            state.stage = Stage::Draining;
            Some(state.loan.take().expect("one original phase loan"))
        } else {
            None
        }
    }
    fn finish_draining(&self, loan: Option<HandoffLoan>) {
        let Some(loan) = loan else { return };
        // Never publish drained before the actual credit/Workflow last drop. A concurrent
        // close sees Draining and cannot jump ahead of this unique retirement owner.
        drop(loan);
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            debug_assert!(state.stage == Stage::Draining && state.copies == 0 && state.bytes == 0);
            state.stage = Stage::Drained;
        }
        self.drained.store(true, Ordering::Release);
        self.changed.notify_waiters();
    }
}
impl Drop for CopyCustody {
    fn drop(&mut self) {
        let loan = {
            let mut state = self.phase.state.lock().unwrap_or_else(|e| e.into_inner());
            debug_assert!(state.copies > 0 && state.bytes >= self.bytes);
            state.copies -= 1;
            state.bytes -= self.bytes;
            Phase::take_draining(&mut state)
        };
        self.phase.finish_draining(loan);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice_authority::tests::phase_fixture;
    use std::future::{poll_fn, Future};
    use std::pin::Pin;
    use std::task::Poll;

    async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
        poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
    }
    #[test]
    fn phase_fixed_layout_and_bounded_copy_metadata_fit_the_paid_record() {
        let (_coordinator, _join, _action, phase) = phase_fixture();
        let waiter = phase.wait_drained();
        // Notify is inline (no hidden watch-channel Shared allocation). Exactly one parked
        // cleanup future belongs to the outer bounded control-flight/coalescing owner.
        let wrapper = std::mem::size_of::<CopyCustody>()
            .max(std::mem::size_of::<Buffer>())
            .max(std::mem::size_of::<InputCustody>());
        let bytes = std::mem::size_of::<Phase>()
            + 2 * std::mem::size_of::<usize>()
            + COPY_OWNERS * wrapper
            + std::mem::size_of_val(&waiter);
        assert!(
            bytes <= METADATA_BYTES,
            "native phase layout {bytes} exceeds {METADATA_BYTES}"
        );
    }

    #[test]
    fn full_buffer_wrappers_fit_and_return_their_exact_copy_contributions() {
        let (_coordinator, _join, _action, phase) = phase_fixture();
        // Include real Vec headers even for zero-sized buffers: payload capacity alone is
        // not the complete retained allocation. The fixed-layout guard pays every wrapper.
        let mut owners: Vec<_> = (0..COPY_OWNERS)
            .map(|index| phase.buffer(index % 2).unwrap())
            .collect();
        assert_eq!(phase.buffer(0).err(), Some(VoiceError::Capacity));
        phase.close();
        for remaining in (0..COPY_OWNERS).rev() {
            drop(owners.pop().unwrap());
            let state = phase.state.lock().unwrap();
            assert_eq!(state.copies, remaining);
            assert_eq!(state.bytes, remaining / 2);
            assert_eq!(phase.is_drained(), remaining == 0);
        }
        assert!(phase.is_drained());
    }

    #[tokio::test]
    async fn actual_native_input_and_completed_buffer_owners_delay_phase_drainage() {
        let (coordinator, join, action, phase) = phase_fixture();
        let input = phase.input(&action).unwrap();
        assert!(matches!(input.input(), CryptoInput::Derive(_)));
        let mut buffer = phase.buffer(7000).unwrap();
        buffer.bytes_mut().fill(11);
        assert_eq!(buffer.bytes()[0], 11);
        action.admit_engine(action.engine_bytes()).unwrap();
        action.settle_engine();
        assert!(action.input().is_err());
        drop(join);
        let pool = coordinator.runtime().resources();
        let held = pool.counts();
        phase.close();
        phase.close();
        assert!(!phase.is_drained());
        assert_eq!(phase.buffer(1).err(), Some(VoiceError::Unauthorized));
        let mut waiter = Box::pin(phase.wait_drained());
        assert!(poll_once(waiter.as_mut()).await.is_pending());
        // Model a completed native task's still-uncollected output. Its ownership survives
        // result availability, just as JoinSet output must survive until actual collection.
        let completed_output = (buffer, input);
        assert_eq!(pool.counts(), held);
        drop(completed_output);
        assert!(phase.is_drained());
        assert!(poll_once(waiter.as_mut()).await.is_ready());
        assert!(pool.counts().0 < held.0 - 16 * 1024);
        assert_eq!(phase.copy(0).err(), Some(VoiceError::Unauthorized));
    }

    #[test]
    fn phase_copy_bytes_and_owner_limits_never_increase_the_original_reservation() {
        let (coordinator, _join, action, phase) = phase_fixture();
        let pool = coordinator.runtime().resources();
        let original = pool.counts();
        let pressure = pool
            .charge(
                action.protected_binding().unwrap().scope.group,
                256 * 1024 - original.0,
                ResourceKind::Retained,
            )
            .unwrap();
        let full = pool.counts();
        let buffer = phase.buffer(16 * 1024).unwrap();
        assert_eq!(pool.counts(), full);
        assert_eq!(phase.buffer(1).err(), Some(VoiceError::Capacity));
        assert_eq!(phase.copy(usize::MAX).err(), Some(VoiceError::Bounds));
        action.admit_engine(action.engine_bytes()).unwrap();
        assert_eq!(
            pool.counts(),
            full,
            "native copy custody never borrows another engine"
        );
        action.settle_engine();
        drop(buffer);
        let mut owners: Vec<_> = (0..COPY_OWNERS).map(|_| phase.copy(0).unwrap()).collect();
        assert_eq!(phase.copy(0).err(), Some(VoiceError::Capacity));
        owners.pop();
        owners.push(phase.copy(1).unwrap());
        phase.close();
        assert!(!phase.is_drained());
        drop(owners);
        assert!(phase.is_drained());
        drop(pressure);
    }

    #[test]
    fn oversize_provider_output_refuses_and_returns_only_its_own_copy_claim() {
        let (_coordinator, _join, _action, phase) = phase_fixture();
        let first = phase.copy(8).unwrap();
        let second = phase.buffer(20).unwrap();
        assert_eq!(first.adopt(vec![1; 9]).err(), Some(VoiceError::Bounds));
        {
            let state = phase.state.lock().unwrap();
            assert_eq!((state.copies, state.bytes), (1, 20));
        }
        phase.close();
        assert!(!phase.is_drained());
        drop(second);
        assert!(phase.is_drained());
    }

    #[test]
    fn a_second_close_cannot_publish_drainage_ahead_of_the_original_loan_drop() {
        let (coordinator, join, action, phase) = phase_fixture();
        action.settle_engine();
        drop(join);
        let before = coordinator.runtime().resources().counts();
        let loan = {
            let mut state = phase.state.lock().unwrap();
            state.stage = Stage::Closing;
            Phase::take_draining(&mut state)
        };
        assert!(loan.is_some());
        phase.close();
        assert!(
            !phase.is_drained(),
            "the actual retiring owner still holds original credit"
        );
        assert_eq!(coordinator.runtime().resources().counts(), before);
        phase.finish_draining(loan);
        assert!(phase.is_drained());
        assert_eq!(
            coordinator.runtime().resources().counts().0,
            before.0 - 16 * 1024
        );
        phase.close();
        assert!(phase.is_drained());
    }

    #[test]
    fn another_native_pool_or_equal_scope_cannot_supply_phase_input() {
        let (_coordinator, _join, _action, phase) = phase_fixture();
        let (_foreign_coordinator, _foreign_join, foreign, _foreign_phase) = phase_fixture();
        assert_eq!(phase.input(&foreign).err(), Some(VoiceError::Unauthorized));
        let state = phase.state.lock().unwrap();
        assert_eq!((state.copies, state.bytes), (0, 0));
    }

    #[test]
    fn prepared_metadata_capacity_refuses_without_any_variable_loan_or_pool_growth() {
        let pool = Arc::new(Resources::default());
        let scope = Scope::for_group(b"native phase metadata", Some([3; 32]), 77);
        let pressure = pool
            .charge(scope.group, 256 * 1024, ResourceKind::Retained)
            .unwrap();
        let before = pool.counts();
        assert_eq!(
            PreparedPhase::new(pool.clone(), scope, CallLease::default()).err(),
            Some(VoiceError::Capacity)
        );
        assert_eq!(pool.counts(), before);
        drop(pressure);
        assert_eq!(pool.counts(), (0, 0));
    }
}
