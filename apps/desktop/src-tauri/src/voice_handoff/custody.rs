//! One confirmation-phase data loan, shared across its original native action rows. Physical
//! exchange positions are separate: holding one through logical settlement would deadlock the
//! subsequent result/settlement requests. Completed worker/result owners must retain their copy
//! leases until collection; browser input/engine drainage is a separate selected-executor gate.
use catcoms_app::{VoiceConfirmationBody, VOICE_CONFIRMATION_BODY_METADATA_BYTES};
use catcoms_sync::voice_v2::crypto_actions::{
    ConfirmationRetirement, CryptoAction, CryptoInput, CryptoInputLease, HandoffLoan,
    ProtectedCryptoPlan,
};
use catcoms_sync::voice_v2::resources::{Credit, ResourceKind, Resources};
use catcoms_sync::voice_v2::{CallLease, Scope, VoiceError};
use std::ops::Range;
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
        self.try_bind(action, bytes).map_err(|(error, _)| error)
    }
    /// Preserve the original fixed preparation after a proven local refusal. A native
    /// factory may already have published its Action, so attachment pressure must not lose
    /// that Action or require fresh metadata payment on its next exact retry. This returns
    /// the SAME preparation; it refunds no engine, nonce, attempt or transport handoff.
    // A local refusal returns the SAME already-paid preparation inline. Boxing its error
    // would add a fallible, separately owned allocation while ordinary capacity is full.
    #[allow(clippy::result_large_err)]
    pub(crate) fn try_bind(
        self,
        action: &CryptoAction,
        bytes: usize,
    ) -> Result<Arc<Phase>, (VoiceError, Self)> {
        if !action.belongs_to(self.scope, &self.call) || !action.uses_resources(&self.resources) {
            return Err((VoiceError::Unauthorized, self));
        }
        let plan = match action.protected_plan() {
            Ok(plan) => plan,
            Err(error) => return Err((error, self)),
        };
        let loan = match action.borrow_handoff(bytes) {
            Ok(loan) => loan,
            Err(error) => return Err((error, self)),
        };
        Ok(Arc::new(Phase {
            resources: self.resources,
            scope: self.scope,
            call: self.call,
            plan,
            capacity: bytes,
            state: Mutex::new(State {
                stage: Stage::Open,
                copies: 0,
                bytes: 0,
                loan: Some(loan),
                retire_confirmation: false,
                retirement: None,
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
/// The first explicit close fixes the meaning of this original loan. Returning an action
/// phase permits confirmation continuation; retiring closes its entire original Workflow.
/// An ordinary close cannot be upgraded while a delayed copy still owns the loan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CloseMode {
    Return,
    Retire,
}
struct State {
    stage: Stage,
    copies: usize,
    bytes: usize,
    loan: Option<HandoffLoan>,
    retire_confirmation: bool,
    // Original fixed Root witness, never Workflow. Keep it in paid native state across
    // cancelled/lost callers; no handshakes/IDs or later factory can reconstruct this proof.
    retirement: Option<ConfirmationRetirement>,
}
/// Snapshot of final-plan retirement, separate from this Phase's LOCAL16KiB loan return.
/// This supplies no wakeup or scheduling guarantee and is not a browser/final-delivery proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmationStatus {
    NotRequested,
    Closing,
    Draining,
    Drained,
}
/// This fixed owner has no strong Action/Input references. Drained history can keep it alive
/// without retaining the original Workflow. Do not mistake this capacity binding for first-
/// effect authority: every actual row still needs its original registered Action and CallGrant.
pub(crate) struct Phase {
    resources: Arc<Resources>,
    scope: Scope,
    call: CallLease,
    // Root's SAME independently paid fixed identity, never an Action/Workflow/input owner.
    // It cannot retain a dead Action allocation after that Action's record credit drops.
    plan: ProtectedCryptoPlan,
    capacity: usize,
    state: Mutex<State>,
    drained: AtomicBool,
    changed: Notify,
    _metadata: Credit,
}
/// The SAME original loan between local close admission and actual Root retirement. Move it
/// out of all Row/Phase guards before dropping it. Unwind cannot publish local completion
/// ahead of Root: Drop performs the one original retirement and publishes its actual witness.
pub(super) struct PhaseClosure {
    phase: Arc<Phase>,
    loan: Option<(HandoffLoan, bool)>,
}
impl Drop for PhaseClosure {
    fn drop(&mut self) {
        self.phase.finish_draining(self.loan.take());
    }
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
/// Pin this exact paid buffer, including its original backing capacity, without extracting
/// or duplicating its Vec. Drop the real buffer before the extra actor metadata contribution.
struct ResultOwner {
    buffer: Buffer,
    range: Range<usize>,
    _metadata: CopyCustody,
}
impl AsRef<[u8]> for ResultOwner {
    fn as_ref(&self) -> &[u8] {
        &self.buffer.bytes()[self.range.clone()]
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
    /// Borrow the SAME paid phase already retained by this actual copy contribution. Native
    /// Work needs no redundant strong Phase handle; this owner survives until after its
    /// worker flag is cleared and the last-copy drainage notification can be observed.
    pub(super) fn phase(&self) -> &Arc<Phase> {
        &self.phase
    }
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
    /// Reserve actor metadata BEFORE taking the original result buffer. A local refusal
    /// returns that exact Buffer, allowing an exact retry with no second SDK run or copy.
    /// Call outside Root/native locks: pinning runs this owner's AsRef and refusal drops
    /// may release actual credits. This trusted Rust owner is not an IPC constructor.
    pub(crate) fn result_body(
        self: &Arc<Self>,
        buffer: Buffer,
        range: Range<usize>,
    ) -> Result<VoiceConfirmationBody, (VoiceError, Buffer)> {
        if !Arc::ptr_eq(&buffer._custody.phase, self)
            || range.start > range.end
            || range.end > buffer.bytes().len()
            || range.len() > 528
        {
            return Err((VoiceError::Bounds, buffer));
        }
        let metadata = match self.copy(VOICE_CONFIRMATION_BODY_METADATA_BYTES) {
            Ok(metadata) => metadata,
            Err(error) => return Err((error, buffer)),
        };
        let owner = ResultOwner {
            buffer,
            range,
            _metadata: metadata,
        };
        // Constructor layout/length bounds are established above and by the concrete guard
        // below. No recoverable capacity refusal occurs after moving the original buffer.
        // Never add a fallible admission between that move and pinning without returning it.
        Ok(VoiceConfirmationBody::from_owner(owner).expect("validated paid result owner"))
    }
    /// Capacity binding for the separately prepaid physical channel, never Root effect authority.
    pub(super) fn uses_pool(&self, resources: &Arc<Resources>, group: [u8; 32]) -> bool {
        Arc::ptr_eq(&self.resources, resources) && self.scope.group == group
    }
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
        if !self.plan.contains(action)
            || !action.belongs_to(self.scope, &self.call)
            || !action.uses_resources(&self.resources)
        {
            return Err(VoiceError::Unauthorized);
        }
        // The original native row still has to resolve this EXACT child's allocation before
        // calling; same-plan provenance only permits sharing the original capacity, never an
        // engine/effect or adoption of an unregistered action. Native input already owns its
        // own paid payload, but its lease independently contributes to phase drainage.
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
    /// Read only the original retained state/atomic: no grant, Clock, allocation or Root call.
    /// Ordinary Phase drainage remains NotRequested even when End independently closes Root.
    /// Final-intent compaction retains this SAME paid Phase/witness in the native Row.
    /// Missing caller handles never establish variable retirement.
    pub(crate) fn confirmation_status(&self) -> ConfirmationStatus {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.retire_confirmation {
            ConfirmationStatus::NotRequested
        } else if let Some(retirement) = &state.retirement {
            if retirement.is_drained() {
                ConfirmationStatus::Drained
            } else {
                ConfirmationStatus::Draining
            }
        } else {
            ConfirmationStatus::Closing
        }
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
        // Best-effort local cleanup cannot change a previously selected terminal mode.
        // The authenticated adapter uses close_mode directly and reports a mismatch.
        let _ = self.close_mode(CloseMode::Return);
    }
    /// Derivation-only pilot closure AFTER selected SDK/RPC drainage and native settlement.
    /// Unlike ordinary close, explicitly retire the ORIGINAL confirmation plan when the last
    /// actual local copy owner returns its loan. Root retirement runs outside State below.
    /// This is not SDP phase transfer or an assertion that other Root lower/input owners drained.
    pub(crate) fn close_confirmation(&self) -> Result<(), VoiceError> {
        self.close_mode(CloseMode::Retire)
    }
    /// Idempotent only for the SAME closure mode, even before the last copy returns. Fix
    /// intent under State, then retire Root outside the guard. No browser flag establishes
    /// settlement or SDK drainage: the trusted adapter must establish those independently.
    pub(super) fn close_mode(&self, mode: CloseMode) -> Result<(), VoiceError> {
        let loan = self.prepare_close(mode)?;
        self.finish_draining(loan);
        Ok(())
    }
    /// Pure local admission under the established Row -> Phase lock order. The returned
    /// ticket owns any actual Root loan; its caller MUST drop it outside every native guard.
    pub(super) fn begin_close(
        self: &Arc<Self>,
        mode: CloseMode,
    ) -> Result<PhaseClosure, VoiceError> {
        Ok(PhaseClosure {
            phase: self.clone(),
            loan: self.prepare_close(mode)?,
        })
    }
    fn prepare_close(&self, mode: CloseMode) -> Result<Option<(HandoffLoan, bool)>, VoiceError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let retire = mode == CloseMode::Retire;
        if state.stage != Stage::Open && state.retire_confirmation != retire {
            return Err(VoiceError::Unauthorized);
        }
        state.retire_confirmation = retire;
        if state.stage == Stage::Open {
            state.stage = Stage::Closing;
        }
        Ok(Self::take_draining(&mut state))
    }
    fn take_draining(state: &mut State) -> Option<(HandoffLoan, bool)> {
        if state.stage == Stage::Closing && state.copies == 0 {
            state.stage = Stage::Draining;
            Some((
                state.loan.take().expect("one original phase loan"),
                state.retire_confirmation,
            ))
        } else {
            None
        }
    }
    fn finish_draining(&self, loan: Option<(HandoffLoan, bool)>) {
        let Some((loan, retire_confirmation)) = loan else {
            return;
        };
        // This completion means LOCAL loan return, not the whole Workflow's last drop.
        // A concurrent close sees Draining and cannot jump ahead of this unique owner.
        // Root close/drop runs outside State; returned proof stays in SAME paid native State.
        let retirement = if retire_confirmation {
            Some(loan.retire_confirmation_observed())
        } else {
            drop(loan);
            None
        };
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            debug_assert!(state.stage == Stage::Draining && state.copies == 0 && state.bytes == 0);
            debug_assert!(state.retirement.is_none());
            // Publish original proof BEFORE local completion/notification. A lost observer
            // can retry the snapshot without taking or replacing this affine witness.
            state.retirement = retirement;
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
            // A terminal native row retains this SAME Phase/payment until its delivery
            // token drops (Live field order / State body before final_phase). Count the
            // full independent token allocation here, rather than hiding it in Row's cap.
            + std::mem::size_of::<super::super::channel::SessionIdentity>()
            + 2 * std::mem::size_of::<usize>()
            + 2 * std::mem::size_of::<usize>()
            + COPY_OWNERS * wrapper
            + std::mem::size_of_val(&waiter);
        assert!(
            bytes <= METADATA_BYTES,
            "native phase layout {bytes} exceeds {METADATA_BYTES}"
        );
    }

    #[test]
    fn same_metadata_from_an_independent_actual_plan_cannot_borrow_the_original_phase() {
        let (coordinator, selected, _selector) = super::super::ledger::tests::selection();
        let (_join, action, _pair) =
            super::super::ledger::tests::actual_action(&coordinator, &selected);
        let phase = PreparedPhase::new(
            selected.resources(),
            selected.binding().scope,
            selected.lease().clone(),
        )
        .unwrap()
        .bind(&action, 16 * 1024)
        .unwrap();
        // PacketLedger deliberately forbids constructing a second ledger under one actual
        // call lease. Use an independent original grant/pool; its deterministic projection
        // still equals the original, while Root provenance and capacity remain independent.
        let (foreign_coordinator, foreign_selected, _) = super::super::ledger::tests::selection();
        let (_foreign_join, foreign, _foreign_pair) =
            super::super::ledger::tests::actual_action(&foreign_coordinator, &foreign_selected);
        assert_eq!(action.protected_binding(), foreign.protected_binding());
        assert!(!foreign.belongs_to(selected.binding().scope, selected.lease()));
        assert!(!foreign.uses_resources(&selected.resources()));
        assert!(!foreign.same_protected_plan(&action));
        let before = selected.resources().counts();
        assert_eq!(phase.input(&foreign).err(), Some(VoiceError::Unauthorized));
        assert_eq!(selected.resources().counts(), before);
        assert!(phase.input(&action).is_ok());
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

    #[test]
    fn explicit_confirmation_close_waits_for_actual_copies_then_retires_the_original_plan() {
        // Keep the actual join/controller alive: End must not supply the retirement being
        // tested. A locally returned 16KiB loan alone leaves the 96KiB Workflow open.
        let (coordinator, _join, action, phase) = phase_fixture();
        let pool = coordinator.runtime().resources();
        let copy = phase.buffer(100).unwrap();
        assert_eq!(
            phase.confirmation_status(),
            ConfirmationStatus::NotRequested
        );
        action.admit_engine(action.engine_bytes()).unwrap();
        action.settle_engine();
        let held = pool.counts();
        phase.close_confirmation().unwrap();
        phase.close_confirmation().unwrap();
        assert_eq!(phase.confirmation_status(), ConfirmationStatus::Closing);
        assert!(!phase.is_drained());
        assert_eq!(pool.counts(), held);
        assert_eq!(phase.copy(0).err(), Some(VoiceError::Unauthorized));
        drop(copy);
        assert!(phase.is_drained());
        assert_eq!(phase.confirmation_status(), ConfirmationStatus::Drained);
        assert!(pool.counts().0 <= held.0 - 96 * 1024);
        let retired = pool.counts();
        phase.close_confirmation().unwrap();
        phase.close();
        assert_eq!(pool.counts(), retired, "replay cannot retire another plan");
    }

    #[test]
    fn original_root_input_delays_confirmation_snapshot_after_local_phase_drains() {
        let (coordinator, _join, action, phase) = phase_fixture();
        let pool = coordinator.runtime().resources();
        // This is an ACTUAL Root input, intentionally outside native Phase copy bookkeeping.
        // Returning every native copy therefore cannot prove this owner's retirement.
        let input = action.input().unwrap();
        action.admit_engine(action.engine_bytes()).unwrap();
        action.settle_engine();
        let pressure = pool
            .charge(
                action.protected_binding().unwrap().scope.group,
                catcoms_sync::voice_v2::resources::GROUP_BYTES - pool.counts().0,
                ResourceKind::Retained,
            )
            .unwrap();
        phase.close_confirmation().unwrap();
        assert!(phase.is_drained(), "local16KiB loan has actually returned");
        let pending = pool.counts();
        for _ in 0..8 {
            assert_eq!(phase.confirmation_status(), ConfirmationStatus::Draining);
            phase.close_confirmation().unwrap();
            phase.close();
            assert_eq!(
                pool.counts(),
                pending,
                "snapshot/replay admits no replacement capacity"
            );
        }
        // Drop the querying native handle and resolve through the SAME retained phase,
        // rather than moving the affine Root proof into cancellable caller work.
        let retained = phase.clone();
        drop(phase);
        drop(input);
        assert_eq!(retained.confirmation_status(), ConfirmationStatus::Drained);
        let completed = pool.counts();
        assert_eq!(retained.confirmation_status(), ConfirmationStatus::Drained);
        assert_eq!(pool.counts(), completed);
        drop(pressure);
    }

    #[test]
    fn original_final_retirement_ticket_cannot_be_overtaken_or_replaced() {
        let (_coordinator, _join, action, phase) = phase_fixture();
        action.settle_engine();
        // Park the SAME production transition ticket before its actual Root close/drop.
        // Another close sees intent, but cannot publish a witness or completion ahead of it.
        let loan = {
            let mut state = phase.state.lock().unwrap();
            state.retire_confirmation = true;
            state.stage = Stage::Closing;
            Phase::take_draining(&mut state)
        };
        assert!(loan.is_some());
        phase.close_confirmation().unwrap();
        phase.close();
        assert!(!phase.is_drained());
        assert_eq!(phase.confirmation_status(), ConfirmationStatus::Closing);
        assert!(phase.state.lock().unwrap().retirement.is_none());
        phase.finish_draining(loan);
        assert!(phase.is_drained());
        assert_eq!(phase.confirmation_status(), ConfirmationStatus::Drained);
        phase.close_confirmation().unwrap();
        assert_eq!(phase.confirmation_status(), ConfirmationStatus::Drained);
        assert!(phase.state.lock().unwrap().retirement.is_some());
    }

    #[test]
    fn result_body_uses_prepaid_phase_capacity_and_its_actual_owner_delays_retirement() {
        let (coordinator, _join, action, phase) = phase_fixture();
        let pool = coordinator.runtime().resources();
        let pressure = pool
            .charge(
                action.protected_binding().unwrap().scope.group,
                catcoms_sync::voice_v2::resources::GROUP_BYTES - pool.counts().0,
                ResourceKind::Retained,
            )
            .unwrap();
        let full = pool.counts();
        let mut buffer = phase.buffer(528).unwrap();
        buffer.bytes_mut().fill(9);
        let body = phase
            .result_body(buffer, 16..528)
            .unwrap_or_else(|_| panic!("prepaid body"));
        assert_eq!(
            pool.counts(),
            full,
            "result pinning takes no ordinary group capacity"
        );
        {
            let state = phase.state.lock().unwrap();
            assert_eq!(
                (state.copies, state.bytes),
                (2, 528 + VOICE_CONFIRMATION_BODY_METADATA_BYTES)
            );
        }
        action.settle_engine();
        phase.close_confirmation().unwrap();
        assert!(!phase.is_drained());
        drop(body);
        assert!(phase.is_drained());
        assert!(pool.counts().0 <= full.0 - 96 * 1024);
        drop(pressure);
    }

    #[test]
    fn result_body_capacity_refusal_returns_the_exact_buffer_for_retry_without_a_copy() {
        let (_coordinator, _join, _action, phase) = phase_fixture();
        let mut buffer = phase.buffer(528).unwrap();
        buffer.bytes_mut()[17] = 91;
        let pointer = buffer.bytes().as_ptr();
        let pressure = phase
            .buffer(16 * 1024 - 528 - VOICE_CONFIRMATION_BODY_METADATA_BYTES + 1)
            .unwrap();
        let (error, buffer) = match phase.result_body(buffer, 16..528) {
            Err(refusal) => refusal,
            Ok(_) => panic!("expected local capacity refusal"),
        };
        assert_eq!(error, VoiceError::Capacity);
        assert_eq!(buffer.bytes().as_ptr(), pointer);
        assert_eq!(buffer.bytes()[17], 91);
        drop(pressure);
        let body = phase
            .result_body(buffer, 16..528)
            .unwrap_or_else(|_| panic!("exact retry"));
        phase.close();
        assert!(!phase.is_drained());
        drop(body);
        assert!(phase.is_drained());
    }

    #[test]
    fn result_body_rejects_foreign_phase_bad_ranges_and_lengths_before_taking_the_buffer() {
        let (_coordinator, _join, _action, phase) = phase_fixture();
        let (_other, _other_join, _other_action, foreign) = phase_fixture();
        let buffer = phase.buffer(600).unwrap();
        let pointer = buffer.bytes().as_ptr();
        let (error, mut buffer) = foreign.result_body(buffer, 0..528).err().unwrap();
        assert_eq!(error, VoiceError::Bounds);
        let reversed = Range { start: 9, end: 8 }; // deliberately invalid input, not iteration
        for range in [reversed, 0..601, 0..529] {
            let (error, returned) = phase.result_body(buffer, range).err().unwrap();
            assert_eq!(error, VoiceError::Bounds);
            assert_eq!(returned.bytes().as_ptr(), pointer);
            buffer = returned;
        }
        drop(buffer);
        let layout = std::alloc::Layout::new::<ResultOwner>();
        assert!(layout.size() <= 128 && layout.align() <= 64);
    }

    #[test]
    fn ordinary_phase_drainage_cannot_later_be_relabelled_as_confirmation_retirement() {
        let (coordinator, _join, action, phase) = phase_fixture();
        action.settle_engine();
        let held = coordinator.runtime().resources().counts();
        phase.close();
        assert!(phase.is_drained());
        assert_eq!(
            phase.confirmation_status(),
            ConfirmationStatus::NotRequested
        );
        assert_eq!(coordinator.runtime().resources().counts(), held);
        assert_eq!(phase.close_confirmation(), Err(VoiceError::Unauthorized));
        assert_eq!(
            phase.confirmation_status(),
            ConfirmationStatus::NotRequested
        );
        assert_eq!(coordinator.runtime().resources().counts(), held);
    }

    #[test]
    fn ordinary_closing_cannot_escalate_while_the_original_copy_still_holds_the_loan() {
        let (coordinator, _join, action, phase) = phase_fixture();
        let copy = phase.copy(0).unwrap();
        action.settle_engine();
        let held = coordinator.runtime().resources().counts();
        phase.close();
        assert!(!phase.is_drained());
        assert_eq!(phase.close_confirmation(), Err(VoiceError::Unauthorized));
        drop(copy);
        assert!(phase.is_drained());
        assert_eq!(
            phase.confirmation_status(),
            ConfirmationStatus::NotRequested
        );
        assert_eq!(coordinator.runtime().resources().counts(), held);

        let (_coordinator, _join, _action, phase) = phase_fixture();
        let loan = {
            let mut state = phase.state.lock().unwrap();
            state.stage = Stage::Closing;
            Phase::take_draining(&mut state)
        };
        assert_eq!(phase.close_confirmation(), Err(VoiceError::Unauthorized));
        assert!(
            !phase.is_drained(),
            "another owner still holds the ordinary retiring loan"
        );
        phase.finish_draining(loan);
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
                catcoms_sync::voice_v2::resources::GROUP_BYTES - original.0,
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
            .charge(
                scope.group,
                catcoms_sync::voice_v2::resources::GROUP_BYTES,
                ResourceKind::Retained,
            )
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
