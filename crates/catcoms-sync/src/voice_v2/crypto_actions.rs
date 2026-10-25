//! Native typed action custody and an irreversible engine-admission fence. Factories below the
//! actual LocalJoin produce these owners; JSON data and opaque action IDs cannot construct one.
//! The action is registered before a pull can expose it. A lost admit reply must reuse the same
//! actual permit, and settlement before a delayed admission permanently refuses new work.
//!
//! Exact terminal receipts share the action's prepaid record and fence, independently of pending
//! token authority. Continuation prepayment, SDP/RTC dispatch and IPC remain separate boundaries;
//! this action/result owner alone never enables normal kind28 dispatch.

use super::crypto_authority::CryptoAuthority;
use super::packets::InboundPacket;
use super::presence::{CryptoFlight, PairSetup};
use super::resources::{ByteBudget, Credit, ResourceKind, Resources};
use super::{CallLease, EncryptedKind, Nonce, Scope, VoiceError};
use catcoms_rt::RequestAuthority;
use catcoms_wire::Encoder;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const ACTIONS: usize = 32;
const CONTEXT_BYTES: usize = 4096;
const CONFIRM_BYTES: usize = 512;
const RECORD_BYTES: usize = 1024;
mod handoff;
mod results;
mod sdp_input;
mod snapshot;
pub(crate) use sdp_input::ObservedSdpInput;
pub use sdp_input::{SdpAdmittedInput, SdpInputRead};
pub use snapshot::{CryptoActionSnapshot, CryptoActionSnapshotPermit, CryptoActionSnapshotWaiter};
pub(crate) mod slots;
pub use super::chain::{
    ConfirmationControl, ConfirmationCustody, ConfirmationRetirement, EstablishedKeyCustody,
    HandoffLoan, PlanRetirement,
};
pub use handoff::{ProtectedCryptoBinding, ProtectedCryptoPlan};
pub use results::{CryptoHandoffStatus, CryptoOutcome, CryptoResultReceipt, CryptoResultStatus};
pub(super) use results::{ResultBinding, TerminalResult};

/// Closed operation identity; the native factory selects it from actual protocol state.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CryptoOperation {
    Derive,
    EncryptConfirmation,
    DecryptConfirmation,
    /// Closed complete-SDP seal on the original native PC/Ready/Offer identity.
    SealSdp,
    /// Closed complete-SDP open; acceptance does not authorize RTC delivery.
    OpenSdp,
}

/// Borrowed immutable input. An adapter must separately charge any serialized/pulled copies;
/// extracting an uncharged owned payload from this native owner is deliberately unsupported.
pub enum CryptoInput<'a> {
    Derive(&'a PairSetup),
    EncryptConfirmation(&'a CryptoFlight),
    DecryptConfirmation(&'a InboundPacket),
    SealSdp(&'a CryptoFlight),
    OpenSdp(&'a InboundPacket),
}

/// Owned capacity lease for borrowed native input. Compaction can remove the action's own
/// reference, but a serializer/worker still holding this lease retains the original buffers
/// and their credit. This grants no engine, signing, packet or RTC authority.
pub struct CryptoInputLease(Arc<ActionPayload>);

/// One synchronous native admission opportunity. The originating action samples its own
/// Clock before lending this value to a trusted adapter's final UI/selection/route fence.
/// `commit` samples no Clock and invokes no adapter callback. Its private, scoped lifetime
/// prevents retaining this opportunity across an await or returning it from the fence.
/// This is not a renderer permit; the original action remains the engine's identity.
pub struct CryptoEngineAdmission<'a> {
    action: &'a CryptoAction,
    bytes: usize,
    mono: u64,
    wall: u64,
    candidate: &'a mut Option<Credit>,
}
impl CryptoEngineAdmission<'_> {
    /// Consume immediately while the trusted final native fences are still held. Settlement,
    /// revocation or a consumed pending token wins even if it occurred in a Clock callback.
    pub fn commit(self) -> Result<u64, VoiceError> {
        // Time was sampled outside every final fence. Waiting here could admit first work
        // after its deadline; an exact retry must take a new observation instead.
        let mut engine = match self.action.engine.try_lock() {
            Ok(engine) => engine,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        if self.bytes != self.action.engine_bytes {
            return Err(VoiceError::RetryMismatch);
        }
        if engine.settled || engine.terminal.is_some() {
            return Err(VoiceError::Unauthorized);
        }
        self.action.check_pending_at(self.mono, self.wall)?;
        if engine.permit.is_none() {
            // Capacity was prepared before the final Clock sample. No Workflow, lane,
            // reservation or Pool lock (nor a potentially blocking Credit Drop) belongs
            // beneath this native/Engine admission sequence.
            engine.permit = Some(self.candidate.take().ok_or(VoiceError::Capacity)?);
        }
        Ok(self.action.serial)
    }
}
impl CryptoInputLease {
    #[cfg(test)]
    pub(crate) fn references_for_test(&self) -> usize {
        Arc::strong_count(&self.0)
    }
    pub fn as_input(&self) -> CryptoInput<'_> {
        match &self.0.input {
            Input::Derive(input) => CryptoInput::Derive(input),
            Input::Encrypt(input) => CryptoInput::EncryptConfirmation(input),
            Input::Decrypt(input) => CryptoInput::DecryptConfirmation(input),
            Input::SealSdp(input) => CryptoInput::SealSdp(input),
            Input::OpenSdp(input) => CryptoInput::OpenSdp(input),
        }
    }
}
struct ActionPayload {
    input: Input,
    _credit: Credit,
}

enum Input {
    Derive(PairSetup),
    Encrypt(CryptoFlight),
    Decrypt(Box<InboundPacket>),
    SealSdp(CryptoFlight),
    OpenSdp(Arc<InboundPacket>),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Key {
    Derive(Nonce),
    Encrypt(Nonce, u64, Nonce),
    Decrypt(u64),
    SdpSeal(Nonce, Nonce),
    SdpOpen(u64),
}

#[derive(Default)]
struct Engine {
    settled: bool,
    // A private synchronous preparation can run outside Engine without lending away its actual
    // charge. Settlement fences first effects immediately, but returns credit only after this
    // affine loan drains. There is at most one preparation for this native action allocation.
    preparing: bool,
    // Lending authority is pending state, not permanent history. Taking it under this same
    // fence prevents a delayed admission from rebuilding capacity after actual settlement.
    lender: Option<ByteBudget>,
    permit: Option<Credit>,
    terminal: Option<results::RecordedResult>,
    // Published atomically with Opened and original token consumption. This fixed observer
    // retains no Action/Exchange/Phase; native lost-reply recovery returns this SAME record.
    sdp_delivery: Option<Arc<super::sdp::delivery::SdpDelivery>>,
    acknowledged: bool,
}

/// Plain retained history funding. Sharing this fixed fee with the delivery observation must
/// never retain a progress-backed PhaseLoan or the action itself. Keep it last in observers.
pub(in crate::voice_v2) struct RecordFunding {
    _credit: Credit,
}

#[must_use = "retain the preparation loan until all actual work and copies have drained"]
pub(super) struct ResultPreparation<'a> {
    action: &'a CryptoAction,
}
impl Drop for ResultPreparation<'_> {
    fn drop(&mut self) {
        let retired = {
            let mut engine = self.action.engine.lock().unwrap_or_else(|e| e.into_inner());
            debug_assert!(engine.preparing);
            engine.preparing = false;
            (
                if engine.settled {
                    engine.permit.take()
                } else {
                    None
                },
                self.action.take_settled_payload(&engine),
            )
        };
        // This loan must stay in the outer result scope. Never destroy input/phase/credit
        // owners beneath Engine or the short native Group/Policy publication fence.
        drop(retired);
    }
}

/// Opaque registered action. Clones preserve the exact engine fence and its actual charge.
/// Retirement/expiry prevents new effects but never refunds a running/uncertain engine slot.
pub struct CryptoAction {
    serial: u64,
    input_tag: [u8; 32],
    scope: Scope,
    call: CallLease,
    authority: CryptoAuthority,
    // Identity only, never progress custody. The pending authority independently observes this
    // irreversible chain lease; fixed historical results do not own a live Workflow.
    chain: Option<CallLease>,
    // Fixed identity shares the existing paid plan object. It must never retain Workflow:
    // terminal action history is independent of variable progress/IPC capacity.
    binding: Option<Arc<super::chain::PlanBinding>>,
    // Preserve RowIssue's actual native position, not a serial-derived or copied IPC index.
    // The shared binding/chain allocations below remain the authority for plan membership.
    protected_row: Option<u8>,
    // Lock order is Engine -> payload. An input lease clones its Arc under this mutex and
    // releases the guard before any engine operation. Actual payload/credit drops occur after
    // releasing BOTH locks, so downstream destructors cannot reenter an action fence.
    input: Mutex<Option<Arc<ActionPayload>>>,
    operation: CryptoOperation,
    // Closed native outputs are at most8208 bytes. Keep this scalar narrow so shared
    // history funding/delivery bookkeeping also fits the original confirmation split row.
    output_bytes: u16,
    resources: Arc<Resources>,
    engine_bytes: usize,
    engine: Mutex<Engine>,
    _slot: slots::Slot,
    // Last: fixed action/delivery history stays paid through destruction of every observer.
    _record: Arc<RecordFunding>,
}
redacted_debug!(
    CryptoAction,
    CryptoEngineAdmission<'_>,
    CryptoInput<'_>,
    CryptoInputLease,
    CryptoOperation
);

impl CryptoAction {
    pub fn serial(&self) -> u64 {
        self.serial
    }
    pub fn version(&self) -> u64 {
        1
    }
    pub fn input_tag(&self) -> [u8; 32] {
        self.input_tag
    }
    pub(super) fn sdp_binding(&self) -> Option<&Arc<super::sdp::action::ActionBinding>> {
        self.authority.sdp_binding()
    }
    pub(in crate::voice_v2) fn history_funding(&self) -> Arc<RecordFunding> {
        self._record.clone()
    }
    pub(crate) fn sdp_peer(&self) -> Option<catcoms_crypto::DeviceId> {
        self.sdp_binding().map(|binding| binding.peer())
    }
    /// Original allocation identity only, never fresh work permission. Native/Actor SDP
    /// adapters must bind the actual action to the SAME funded PC intent before admitting
    /// a body or result; equal serialized round/Offer labels cannot fund another pair.
    pub fn uses_sdp_intent(&self, intent: &Arc<super::sdp::InitialSdpIntent>) -> bool {
        self.sdp_binding()
            .is_some_and(|binding| binding.belongs_to_intent(intent))
    }
    /// Pure exact Seal-body binding for trusted native publication capture. This works
    /// after input compaction/End and never creates an input lease or calls the Clock.
    /// Open actions have a distinct packet commitment and always refuse this comparison.
    pub fn matches_sdp_plaintext_commitment(&self, tag: &[u8; 32]) -> bool {
        self.sdp_binding()
            .is_some_and(|binding| binding.matches_plaintext_commitment(tag))
    }
    pub fn operation(&self) -> CryptoOperation {
        self.operation
    }
    /// Borrow through an actual owner, never an uncharged Vec extraction. An acknowledged and
    /// settled action may have compacted its input; immutable terminal history remains usable.
    /// Proven unstarted retirement and actual settlement without a terminal result compact too.
    pub fn input(&self) -> Result<CryptoInputLease, VoiceError> {
        self.input
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .cloned()
            .map(CryptoInputLease)
            .ok_or(VoiceError::Unauthorized)
    }
    /// Exact same native call owner and authenticated scope, not merely equal JSON IDs.
    pub fn belongs_to(&self, scope: Scope, call: &CallLease) -> bool {
        self.scope == scope && self.call.is_same(call)
    }
    pub fn uses_resources(&self, resources: &Arc<Resources>) -> bool {
        Arc::ptr_eq(&self.resources, resources)
    }
    pub(super) fn uses_chain(&self, chain: &CallLease) -> bool {
        self.chain
            .as_ref()
            .is_some_and(|original| original.is_same(chain))
    }
    pub(super) fn check_chain_epoch(&self, epoch: u64) -> Result<(), VoiceError> {
        // The exact PlanBinding already owns this immutable epoch. Keeping a second
        // Option<u64> on every action duplicated both identity and retained metadata.
        if self
            .binding
            .as_ref()
            .is_none_or(|original| original.epoch == epoch)
        {
            Ok(())
        } else {
            Err(VoiceError::Unauthorized)
        }
    }
    pub(super) fn pending_observation(&self) -> (u64, u64) {
        self.authority.observation_time()
    }
    /// Pending original-owner check only; caller MUST already hold this action's result fence.
    /// This never reenters Engine or samples a Clock while final native admission is held.
    pub(super) fn check_pending_at(&self, mono: u64, wall: u64) -> Result<(), VoiceError> {
        if self.authority.authorized_at(mono, wall)
            && self.call.is_live()
            && self.chain.as_ref().is_none_or(CallLease::is_live)
        {
            Ok(())
        } else {
            Err(VoiceError::Unauthorized)
        }
    }
    /// Allow bounded private preparation only after this exact engine was admitted. The final
    /// result fence must repeat settlement/permit and original-owner checks: settlement may win
    /// while signatures/RNG/copies are prepared, in which case no first effect is published.
    pub(super) fn begin_result_preparation(&self) -> Result<ResultPreparation<'_>, VoiceError> {
        let (mono, wall) = self.pending_observation();
        let mut engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        if engine.settled || engine.permit.is_none() || engine.terminal.is_some() {
            return Err(VoiceError::Unauthorized);
        }
        self.check_pending_at(mono, wall)?;
        if engine.preparing {
            return Err(VoiceError::Capacity);
        }
        engine.preparing = true;
        Ok(ResultPreparation { action: self })
    }
    pub fn engine_bytes(&self) -> usize {
        self.engine_bytes
    }
    pub fn is_settled(&self) -> bool {
        let engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        engine.settled && !engine.preparing
    }
    #[cfg(test)]
    pub(crate) fn result_fence_available_for_test(&self) -> bool {
        // A deterministic injected Clock/RNG can observe whether preparation accidentally
        // executes beneath Engine, without exposing any permit or mutable engine state.
        self.engine.try_lock().is_ok()
    }
    pub fn is_pending(&self) -> bool {
        let (mono, wall) = self.pending_observation();
        let engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        !engine.settled && engine.terminal.is_none() && self.check_pending_at(mono, wall).is_ok()
    }

    /// The trusted native adapter calls this only for an already registered exact action.
    /// Its immutable serial is the permit identity inside that exact native owner; duplicates
    /// return it without charging again. Capacity is transient and leaves the action eligible.
    pub fn admit_engine(&self, bytes: usize) -> Result<u64, VoiceError> {
        self.with_engine_admission(bytes, |admission| admission.commit())
    }

    /// Sample the ORIGINAL Clock outside Engine and all native gates, then lend a single
    /// non-escaping commit to the adapter. The callback must acquire its final native fences
    /// and consume the opportunity synchronously, without awaiting or calling external code.
    /// EVERY final mutex must use nonblocking admission: contention returns Capacity and an
    /// exact retry samples time again. Waiting after this sample could grant expired work.
    /// The higher-ranked lifetime keeps the opportunity out of `T`; dropping it admits nothing.
    /// In particular, `Ok(T)` returned by the callback is not proof that it called `commit`.
    /// Native adapters must serialize the exact successful `commit()` result as their permit.
    /// A refusal before commit preserves eligibility and never refunds an ambiguous engine.
    ///
    /// ```compile_fail
    /// use catcoms_sync::voice_v2::crypto_actions::{CryptoAction, CryptoEngineAdmission};
    /// use catcoms_sync::voice_v2::VoiceError;
    /// fn escape(action: &CryptoAction) -> Result<CryptoEngineAdmission<'_>, VoiceError> {
    ///     action.with_engine_admission(action.engine_bytes(), |step| Ok(step))
    /// }
    /// ```
    pub fn with_engine_admission<T>(
        &self,
        bytes: usize,
        fence: impl for<'commit> FnOnce(CryptoEngineAdmission<'commit>) -> Result<T, VoiceError>,
    ) -> Result<T, VoiceError> {
        if bytes != self.engine_bytes {
            return Err(VoiceError::RetryMismatch);
        }
        // This preflight may wait, but occurs BEFORE the final Clock observation. Never
        // lend the mutable Engine or a credit owner across callbacks. Exact held permits
        // need no second reservation; a racing admission is reconciled again at commit.
        let lender = {
            let engine = self
                .engine
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if engine.settled || engine.terminal.is_some() {
                return Err(VoiceError::Unauthorized);
            }
            if engine.permit.is_some() {
                None
            } else {
                Some(
                    engine
                        .lender
                        .as_ref()
                        .ok_or(VoiceError::Unauthorized)?
                        .clone(),
                )
            }
        };
        let mut candidate = match lender {
            Some(lender) => match lender.charge(
                &self.resources,
                self.scope.group,
                bytes,
                ResourceKind::Crypto,
            ) {
                Ok(credit) => Some(credit),
                Err(VoiceError::Capacity) => {
                    // Another exact caller may have installed the original permit since
                    // preflight. Its prepaid lane can itself cause this local refusal.
                    // Reconcile before the final Clock; never let capacity hide that permit.
                    let engine = self
                        .engine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    if engine.settled || engine.terminal.is_some() {
                        return Err(VoiceError::Unauthorized);
                    }
                    if engine.permit.is_none() {
                        return Err(VoiceError::Capacity);
                    }
                    None
                }
                Err(error) => return Err(error),
            },
            None => None,
        };
        let (mono, wall) = self.pending_observation();
        let result = fence(CryptoEngineAdmission {
            action: self,
            bytes,
            mono,
            wall,
            candidate: &mut candidate,
        });
        // A refused/unneeded candidate was never an engine permit. Its Drop can wait on
        // capacity fences, so keep it OUTSIDE the native callback/Engine guards, including
        // unwind. Only an actually installed permit remains owned by Engine after return.
        drop(candidate);
        result
    }

    /// Actual engine and last result-RPC settlement only. An unstarted action can be closed too;
    /// a delayed admission cannot recreate its credit. No current-call check is required to
    /// settle old actual work while locked. No-terminal input can retire because this same fence
    /// permanently rejects a later first result. The native registry rejects unknown action IDs.
    pub fn settle_engine(&self) {
        let retired = {
            let mut engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
            engine.settled = true;
            (
                if engine.preparing {
                    None
                } else {
                    engine.permit.take()
                },
                engine.lender.take(),
                self.take_settled_payload(&engine),
            )
        };
        drop(retired);
    }

    fn take_settled_payload(&self, engine: &Engine) -> Option<Arc<ActionPayload>> {
        // No terminal result after actual last-RPC settlement can ever become a first result:
        // the permanent engine fence already refuses it. Such failed/unstarted work has no
        // receipt to ACK. Installed terminal results still require their exact ACK.
        if engine.settled && !engine.preparing && (engine.terminal.is_none() || engine.acknowledged)
        {
            self.input.lock().unwrap_or_else(|e| e.into_inner()).take()
        } else {
            None
        }
    }

    /// Owner retirement may close an unstarted action without inventing actual settlement.
    /// An admitted operation remains charged and is closed only by its real settlement report.
    pub fn retire_if_unstarted(&self) {
        let retired = {
            let mut engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
            // A settled terminal action can also have no permit. Lost ACKs must not be
            // mistaken for proof of unstarted work or discard their still-unacknowledged input.
            if engine.permit.is_none() && engine.terminal.is_none() {
                engine.settled = true;
                (
                    engine.lender.take(),
                    self.input.lock().unwrap_or_else(|e| e.into_inner()).take(),
                )
            } else {
                (None, None)
            }
        };
        drop(retired);
    }

    /// First protocol effects serialize against the same fence used by admit/settle. The sole
    /// actor's closure repeats its current membership/pair/action checks and prepaid result
    /// transaction. It must not await, reenter this action, or acquire a native Coordinator lock.
    /// Exact historical receipts need a separately authorized path; pending authority is consumed
    /// by a successful confirmation transition and is intentionally insufficient for that path.
    #[cfg(test)]
    pub(crate) fn with_pending<T>(
        &self,
        commit: impl FnOnce() -> Result<T, VoiceError>,
    ) -> Result<T, VoiceError> {
        let engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        if engine.settled
            || engine.terminal.is_some()
            || engine.permit.is_none()
            || !self.authority.authorized()
        {
            return Err(VoiceError::Unauthorized);
        }
        let result = commit();
        // Keep the fence held through the whole synchronous effect, including its publication.
        drop(engine);
        result
    }
}

#[derive(Default)]
pub(super) struct ActionQueue {
    serial: u64,
    actions: BTreeMap<Key, Arc<CryptoAction>>,
    cursor: u64,
    slots: Arc<slots::Slots>,
}

pub(super) struct PreparedAction {
    key: Key,
    serial: u64,
    scope: Scope,
    call: CallLease,
    resources: Arc<Resources>,
    budget: ByteBudget,
    engine_bytes: usize,
    record: Credit,
    payload: Credit,
    slot: slots::Slot,
    issue: Option<super::negotiation::ActionIssue>,
    chain_issue: Option<super::chain::RowIssue>,
    // One separately paid PC direction record per affine issue. Builders cannot adopt
    // another issue's equal-labelled record. This owner never escapes into history.
    sdp_binding: Option<Arc<super::sdp::action::ActionBinding>>,
    chain: Option<CallLease>,
    chain_epoch: Option<u64>,
    binding: Option<Arc<super::chain::PlanBinding>>,
    protected_row: Option<u8>,
}

pub(super) struct Allocation {
    pub resources: Arc<Resources>,
    pub scope: Scope,
    pub call: CallLease,
    pub budget: ByteBudget,
    pub history: Option<super::chain::PreparedHistory>,
    pub payload_bytes: usize,
    pub engine_bytes: usize,
}

/// Fully built but unpublished: no action Arc or engine admission escapes this private owner.
pub(super) struct InactiveAction {
    key: Key,
    action: Arc<CryptoAction>,
}

impl PreparedAction {
    pub(super) fn serial(&self) -> u64 {
        self.serial
    }
    #[allow(clippy::too_many_arguments)] // Keep original issue, identity and both native clock samples explicit.
    pub(super) fn sdp_binding(
        &mut self,
        intent: Arc<super::sdp::InitialSdpIntent>,
        header: &super::EncryptedHeader,
        message: Nonce,
        direction: super::sdp::action::Direction,
        body_tag: [u8; 32],
        wall: u64,
        mono: u64,
    ) -> Result<Arc<super::sdp::action::ActionBinding>, VoiceError> {
        if self.sdp_binding.is_some() {
            return Err(VoiceError::Unauthorized);
        }
        let binding = super::sdp::action::ActionBinding::prepare(
            self.issue.as_mut().ok_or(VoiceError::Unauthorized)?,
            intent,
            header,
            message,
            direction,
            body_tag,
            wall,
            mono,
        )?;
        self.sdp_binding = Some(binding.clone());
        Ok(binding)
    }
}
impl InactiveAction {
    pub(super) fn check_pending_at(&self, mono: u64, wall: u64) -> Result<(), VoiceError> {
        self.action.check_pending_at(mono, wall)
    }
}
#[cfg(test)]
pub(super) fn fixed_record_layout_bytes() -> usize {
    // Engine embeds terminal-result metadata; count it once through CryptoAction. Input/body
    // buffers have their own credit. The 128-byte logical map/bookkeeping allowance, like the
    // existing plan growth guard, excludes allocator slack and map-node spare capacity.
    std::mem::size_of::<CryptoAction>()
        + 2 * std::mem::size_of::<usize>()
        + std::mem::size_of::<Key>()
        + std::mem::size_of::<Arc<CryptoAction>>()
        + std::mem::size_of::<RecordFunding>()
        + 2 * std::mem::size_of::<usize>()
        + 128
}

impl ActionQueue {
    /// Adopt the original inert LOW preparation, rather than making ordinary/Active-only
    /// loans before the Offer's atomic publication. All capacity has already been paid.
    pub(super) fn prepare_initial_sdp_seal(
        &mut self,
        key: Key,
        resources: Arc<Resources>,
        scope: Scope,
        call: CallLease,
        exchange: Arc<super::negotiation::Exchange>,
        original: super::negotiation::initial_offer::InitialOfferAction,
    ) -> Result<PreparedAction, VoiceError> {
        if !matches!(key, Key::SdpSeal(_, _)) || !exchange.uses_resources(&resources, scope.group) {
            return Err(VoiceError::Unauthorized);
        }
        if self.actions.len() >= ACTIONS {
            return Err(VoiceError::Capacity);
        }
        let serial = self.serial.checked_add(1).ok_or(VoiceError::Capacity)?;
        self.serial = serial;
        Ok(PreparedAction {
            key,
            serial,
            scope,
            call,
            resources,
            budget: ByteBudget::Exchange {
                exchange,
                stage: super::negotiation::Stage::EncryptAction,
            },
            engine_bytes: original.engine_bytes,
            record: original.history,
            payload: original.payload,
            slot: original.slot,
            issue: Some(original.issue),
            chain_issue: None,
            sdp_binding: None,
            chain: None,
            chain_epoch: None,
            binding: None,
            protected_row: None,
        })
    }
    /// Prepare the same native derive row/input as compatibility publication, without issuing
    /// it. Root borrows the outer issue into its final origin fence; no action escapes early.
    pub(super) fn build_chain_derive(
        mut prepared: PreparedAction,
        input: PairSetup,
        authority: CryptoAuthority,
    ) -> Result<(super::chain::RowIssue, InactiveAction), VoiceError> {
        let issue = prepared.chain_issue.take().ok_or(VoiceError::Scope)?;
        let (key, action, exchange) = Self::build(prepared, Input::Derive(input), authority);
        assert!(
            exchange.is_none(),
            "chain and Exchange cannot share an action"
        );
        Ok((issue, InactiveAction { key, action }))
    }
    pub(super) fn build_chain_encrypt(
        mut prepared: PreparedAction,
        input: CryptoFlight,
        authority: CryptoAuthority,
    ) -> Result<(super::chain::RowIssue, InactiveAction), VoiceError> {
        let issue = prepared.chain_issue.take().ok_or(VoiceError::Scope)?;
        let (key, action, exchange) = Self::build(prepared, Input::Encrypt(input), authority);
        assert!(exchange.is_none(), "confirmation cannot adopt an SDP issue");
        Ok((issue, InactiveAction { key, action }))
    }

    pub(super) fn build_chain_decrypt(
        mut prepared: PreparedAction,
        input: InboundPacket,
        authority: CryptoAuthority,
    ) -> Result<(super::chain::RowIssue, InactiveAction), VoiceError> {
        let issue = prepared.chain_issue.take().ok_or(VoiceError::Scope)?;
        let (key, action, exchange) =
            Self::build(prepared, Input::Decrypt(Box::new(input)), authority);
        assert!(
            exchange.is_none(),
            "confirmation history cannot adopt an SDP issue"
        );
        Ok((issue, InactiveAction { key, action }))
    }
    /// Build outside the final origin fence. The original direction issue remains an
    /// affine outer owner; only the same final packet/role transaction may publish it.
    pub(super) fn build_sdp_seal(
        mut prepared: PreparedAction,
        input: CryptoFlight,
        binding: Arc<super::sdp::action::ActionBinding>,
        authority: CryptoAuthority,
    ) -> Result<(super::negotiation::ActionIssue, InactiveAction), VoiceError> {
        if !prepared
            .sdp_binding
            .as_ref()
            .is_some_and(|original| Arc::ptr_eq(original, &binding))
        {
            return Err(VoiceError::Unauthorized);
        }
        if !binding.matches_plaintext(&input.plaintext)
            || !matches!(prepared.key, Key::SdpSeal(_, _))
            || input.plaintext.len() > super::MAX_SDP_PLAINTEXT
            || input.reservation.aad.len() > super::negotiation::MAX_AAD
            || std::str::from_utf8(&input.plaintext).is_err()
        {
            return Err(VoiceError::Bounds);
        }
        let issue = prepared.issue.take().ok_or(VoiceError::Unauthorized)?;
        let authority = authority.with_sdp(binding)?;
        let (key, action, other) = Self::build(prepared, Input::SealSdp(input), authority);
        assert!(other.is_none());
        Ok((issue, InactiveAction { key, action }))
    }
    /// Share the SAME paid admitted payload. A final-fence refusal must leave its original
    /// native inbox reference intact; dropping a preparation cannot lose a committed token.
    pub(super) fn build_sdp_open(
        mut prepared: PreparedAction,
        input: Arc<InboundPacket>,
        binding: Arc<super::sdp::action::ActionBinding>,
        authority: CryptoAuthority,
    ) -> Result<(super::negotiation::ActionIssue, InactiveAction), VoiceError> {
        if !prepared
            .sdp_binding
            .as_ref()
            .is_some_and(|original| Arc::ptr_eq(original, &binding))
        {
            return Err(VoiceError::Unauthorized);
        }
        if !binding.matches_packet(&input)
            || !matches!(prepared.key, Key::SdpOpen(_))
            || !(16..=super::MAX_SDP_PLAINTEXT + 16).contains(&input.ciphertext().len())
            || input.aad().len() > super::negotiation::MAX_AAD
        {
            return Err(VoiceError::Bounds);
        }
        let issue = prepared.issue.take().ok_or(VoiceError::Unauthorized)?;
        let authority = authority.with_sdp(binding)?;
        let (key, action, other) = Self::build(prepared, Input::OpenSdp(input), authority);
        assert!(other.is_none());
        Ok((issue, InactiveAction { key, action }))
    }
    /// Called only in the infallible part of the native origin/History transaction.
    pub(super) fn publish_inactive(&mut self, inactive: InactiveAction) -> Arc<CryptoAction> {
        assert!(
            self.actions
                .insert(inactive.key, inactive.action.clone())
                .is_none(),
            "native action uniqueness"
        );
        inactive.action
    }
    pub(super) fn with_slots(slots: Arc<slots::Slots>) -> Self {
        Self {
            slots,
            ..Self::default()
        }
    }
    pub(super) fn get(&self, key: Key) -> Option<Arc<CryptoAction>> {
        self.actions.get(&key).cloned()
    }
    /// Recognize an actual retained action allocation. Serial, input tag or JSON equality is
    /// insufficient; the bounded native queue remains the authority after input compaction.
    pub(super) fn key_of(&self, action: &Arc<CryptoAction>) -> Option<Key> {
        self.actions
            .iter()
            .find_map(|(key, original)| Arc::ptr_eq(original, action).then_some(*key))
    }

    /// Prepay the bounded record, owned input and eventual small closed receipt before creating
    /// another input copy. The combined pending/terminal budget never resets within this join.
    /// Larger endpoint/SDP results and protected continuation capacity remain separate work.
    pub(super) fn prepare(
        &mut self,
        key: Key,
        allocation: Allocation,
    ) -> Result<PreparedAction, VoiceError> {
        let Allocation {
            resources,
            scope,
            call,
            budget,
            history,
            payload_bytes,
            engine_bytes,
        } = allocation;
        if self.actions.len() >= ACTIONS {
            return Err(VoiceError::Capacity);
        }
        let serial = self.serial.checked_add(1).ok_or(VoiceError::Capacity)?;
        let binding = history.as_ref().map(|history| history.issue.binding());
        let protected_row = history.as_ref().map(|history| history.issue.row_index());
        if let Some(history) = &history {
            history
                .issue
                .validate(&resources, &self.slots, scope, &call, key, &budget)?;
        } else if matches!(budget, ByteBudget::Workflow(_)) {
            return Err(VoiceError::Scope);
        }
        let slot = if history.is_some() {
            // The row and slot are consumed together below; this branch is handled separately
            // to avoid temporarily allocating an ordinary slot under full protected pressure.
            None
        } else {
            Some(match &budget {
                ByteBudget::Exchange { exchange, .. } => exchange.action_slot(&self.slots)?,
                _ => self.slots.charge(1)?,
            })
        };
        let (record, payload, issue, slot, chain_issue, chain, chain_epoch) = if let Some(history) =
            history
        {
            let chain = history.issue.lease();
            let epoch = history.issue.epoch();
            let payload = budget.charge(
                &resources,
                scope.group,
                payload_bytes,
                ResourceKind::Retained,
            )?;
            (
                history.record,
                payload,
                None,
                history.slot,
                Some(history.issue),
                Some(chain),
                Some(epoch),
            )
        } else {
            let (record, payload, issue) = match &budget {
                ByteBudget::Exchange { exchange, stage } => {
                    if !exchange.uses_resources(&resources, scope.group) {
                        return Err(VoiceError::Scope);
                    }
                    let (record, payload, issue) = exchange.prepare_action_credits(
                        *stage,
                        RECORD_BYTES + CONFIRM_BYTES,
                        payload_bytes,
                    )?;
                    (record, payload, Some(issue))
                }
                ByteBudget::Ordinary
                | ByteBudget::Reserved(_)
                | ByteBudget::ReservedLane { .. } => {
                    // These closed variants have no phase/workflow custody on retained charges.
                    // Their existing conserved partition is safe; Exchange is deliberately handled
                    // above before any phase owner can be attached. Workflow instead requires
                    // independently prepared fixed history in the preceding branch.
                    let bytes = payload_bytes
                        .checked_add(RECORD_BYTES + CONFIRM_BYTES)
                        .ok_or(VoiceError::Bounds)?;
                    let mut record =
                        budget.charge(&resources, scope.group, bytes, ResourceKind::Retained)?;
                    let payload = record.split_retained(payload_bytes)?;
                    (record, payload, None)
                }
                ByteBudget::Workflow(_) => unreachable!("workflow requires prepared fixed history"),
            };
            (
                record,
                payload,
                issue,
                slot.expect("ordinary/exchange slot"),
                None,
                None,
                None,
            )
        };
        self.serial = serial; // Abandoned preparation burns identity, never permits resurrection.
        Ok(PreparedAction {
            key,
            serial,
            scope,
            call,
            resources,
            budget,
            engine_bytes,
            record,
            payload,
            slot,
            issue,
            chain_issue,
            sdp_binding: None,
            chain,
            chain_epoch,
            binding,
            protected_row,
        })
    }

    fn build(
        prepared: PreparedAction,
        input: Input,
        authority: CryptoAuthority,
    ) -> (
        Key,
        Arc<CryptoAction>,
        Option<super::negotiation::ActionIssue>,
    ) {
        assert!(
            prepared.chain_issue.is_none(),
            "chain row commits through its own publication fence"
        );
        let input_tag = tag(&prepared, &input);
        debug_assert_eq!(
            prepared.chain_epoch,
            prepared.binding.as_ref().map(|binding| binding.epoch)
        );
        let (operation, output_bytes) = match &input {
            Input::Derive(_) => (CryptoOperation::Derive, 0),
            Input::Encrypt(flight) => (
                CryptoOperation::EncryptConfirmation,
                flight.plaintext.len() + 16,
            ),
            Input::Decrypt(packet) => (
                CryptoOperation::DecryptConfirmation,
                packet.ciphertext().len() - 16,
            ),
            Input::SealSdp(flight) => (CryptoOperation::SealSdp, flight.plaintext.len() + 16),
            Input::OpenSdp(packet) => (CryptoOperation::OpenSdp, packet.ciphertext().len() - 16),
        };
        let action = Arc::new(CryptoAction {
            serial: prepared.serial,
            input_tag,
            scope: prepared.scope,
            call: prepared.call,
            authority,
            chain: prepared.chain,
            binding: prepared.binding,
            protected_row: prepared.protected_row,
            input: Mutex::new(Some(Arc::new(ActionPayload {
                input,
                _credit: prepared.payload,
            }))),
            operation,
            output_bytes: u16::try_from(output_bytes).expect("closed native output ceiling"),
            resources: prepared.resources,
            engine_bytes: prepared.engine_bytes,
            engine: Mutex::new(Engine {
                lender: Some(prepared.budget),
                ..Engine::default()
            }),
            _slot: prepared.slot,
            _record: Arc::new(RecordFunding {
                _credit: prepared.record,
            }),
        });
        (prepared.key, action, prepared.issue)
    }
    fn publish(
        &mut self,
        prepared: PreparedAction,
        input: Input,
        authority: CryptoAuthority,
    ) -> Arc<CryptoAction> {
        let (key, action, mut issue) = Self::build(prepared, input, authority);
        if let Some(issue) = issue.as_mut() {
            issue.publish();
        }
        assert!(
            self.actions.insert(key, action.clone()).is_none(),
            "native action uniqueness"
        );
        action
    }
    /// Chain derivation prepares all fallible state before its irreversible role issue. The
    /// BTreeMap insertion still uses the allocator, as ordinary publication does; capacity and
    /// all recoverable errors are prepaid, not a promise to survive process allocation failure.
    pub(super) fn publish_chain_derive(
        &mut self,
        prepared: PreparedAction,
        input: PairSetup,
        authority: CryptoAuthority,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        let (issue, inactive) = Self::build_chain_derive(prepared, input, authority)?;
        issue.publish(|| self.publish_inactive(inactive))
    }
    pub(super) fn publish_derive(
        &mut self,
        prepared: PreparedAction,
        input: PairSetup,
        authority: CryptoAuthority,
    ) -> Arc<CryptoAction> {
        self.publish(prepared, Input::Derive(input), authority)
    }
    pub(super) fn publish_encrypt(
        &mut self,
        prepared: PreparedAction,
        input: CryptoFlight,
        authority: CryptoAuthority,
    ) -> Arc<CryptoAction> {
        self.publish(prepared, Input::Encrypt(input), authority)
    }
    pub(super) fn publish_decrypt(
        &mut self,
        prepared: PreparedAction,
        input: InboundPacket,
        authority: CryptoAuthority,
    ) -> Arc<CryptoAction> {
        self.publish(prepared, Input::Decrypt(Box::new(input)), authority)
    }
    pub(super) fn pull(&mut self) -> Vec<Arc<CryptoAction>> {
        // Serial order rotates fairly; pulling observes without dequeueing or minting authority.
        let mut pending: Vec<_> = self
            .actions
            .values()
            .filter(|action| action.is_pending())
            .cloned()
            .collect();
        pending.sort_by_key(|action| (action.serial <= self.cursor, action.serial));
        pending.truncate(4);
        if let Some(last) = pending.last() {
            self.cursor = last.serial;
        }
        pending
    }
}

fn tag(prepared: &PreparedAction, input: &Input) -> [u8; 32] {
    if let Input::Decrypt(packet) = input {
        return packet.object_hash();
    }
    if matches!(input, Input::SealSdp(_) | Input::OpenSdp(_)) {
        return sdp_tag(prepared, input);
    }
    let mut e = Encoder::new();
    e.put_str("mewtual/voice/native-crypto-action/v1")
        .expect("constant");
    prepared.scope.put(&mut e);
    e.put_u64(prepared.serial);
    e.put_u64(1); // Native action version, independent of the signed wire protocol version.
    let mut hash = blake3::Hasher::new();
    // Keep only the small fixed metadata encoding. Stream borrowed variable fields with the
    // wire codec's canonical u32 big-endian length instead of allocating an uncharged second
    // AAD/plaintext/transcript copy while publishing the prepaid record.
    match input {
        Input::Derive(setup) => {
            e.put_u8(1);
            super::fixed(&mut e, &setup.handshake);
            super::fixed(&mut e, setup.peer.as_bytes());
            super::fixed(&mut e, &setup.peer_key);
            e.put_u8(u8::from(setup.local_is_low));
            hash.update(&e.finish());
            tag_bytes(&mut hash, &setup.transcript);
        }
        Input::Encrypt(flight) => {
            e.put_u8(2);
            super::fixed(&mut e, &flight.handshake);
            e.put_u64(flight.action);
            super::fixed(&mut e, &flight.reservation.id);
            e.put_u64(flight.reservation.sequence);
            e.put_u64(flight.reservation.expires);
            hash.update(&e.finish());
            tag_bytes(&mut hash, &flight.reservation.aad);
            tag_bytes(&mut hash, &flight.plaintext);
        }
        Input::Decrypt(_) | Input::SealSdp(_) | Input::OpenSdp(_) => unreachable!(),
    }
    *hash.finalize().as_bytes()
}

/// Separate native action domain. The wire key/nonce domain and original ledger counter
/// remain unchanged; this fingerprint only binds an exact native SDK/result transaction.
fn sdp_tag(prepared: &PreparedAction, input: &Input) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"mewtual/voice/native-sdp-action/v1\0");
    hash.update(&prepared.serial.to_be_bytes());
    match input {
        Input::SealSdp(flight) => {
            hash.update(&[1]);
            hash.update(&flight.handshake);
            hash.update(&flight.reservation.id);
            tag_bytes(&mut hash, &flight.reservation.aad);
            tag_bytes(&mut hash, &flight.plaintext);
        }
        Input::OpenSdp(packet) => {
            hash.update(&[2]);
            hash.update(&packet.token.to_be_bytes());
            hash.update(&packet.object_hash());
            tag_bytes(&mut hash, packet.aad());
            tag_bytes(&mut hash, packet.ciphertext());
        }
        _ => unreachable!("closed SDP input"),
    }
    *hash.finalize().as_bytes()
}
fn tag_bytes(hash: &mut blake3::Hasher, bytes: &[u8]) {
    hash.update(
        &u32::try_from(bytes.len())
            .expect("native bounded action input")
            .to_be_bytes(),
    );
    hash.update(bytes);
}

pub(super) fn derive_bytes(transcript: usize) -> Result<(usize, usize), VoiceError> {
    if transcript > CONTEXT_BYTES {
        return Err(VoiceError::Bounds);
    }
    Ok((transcript + 256, 3 * transcript + 4096 + 256))
}
pub(super) fn confirmation_bytes(
    aad: usize,
    ciphertext: usize,
) -> Result<(usize, usize), VoiceError> {
    if aad > CONTEXT_BYTES || !(16..=CONFIRM_BYTES + 16).contains(&ciphertext) {
        return Err(VoiceError::Bounds);
    }
    // Same conservative peak used by the surviving browser confirmation registry. Charges
    // include engine, digest, caller/result-RPC copies; actual lower work has independent custody.
    Ok((
        aad + ciphertext,
        4 * (aad + ciphertext) + 3 * CONFIRM_BYTES + 2048,
    ))
}
pub(super) fn sdp_bytes(aad: usize, ciphertext: usize) -> Result<(usize, usize), VoiceError> {
    if aad > super::negotiation::MAX_AAD
        || !(16..=super::MAX_SDP_PLAINTEXT + 16).contains(&ciphertext)
    {
        return Err(VoiceError::Bounds);
    }
    Ok((
        aad + ciphertext,
        4 * (aad + ciphertext) + 3 * super::MAX_SDP_PLAINTEXT + 2048,
    ))
}
pub(super) fn is_confirmation(kind: EncryptedKind) -> bool {
    matches!(
        kind,
        EncryptedKind::ConfirmHello | EncryptedKind::ConfirmProof | EncryptedKind::ConfirmFinish
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_crypto::DeviceId;

    #[test]
    fn acknowledged_settled_input_compacts_without_refunding_held_leases_or_resetting_history() {
        for ack_first in [false, true] {
            let pool = Arc::new(Resources::default());
            let call = CallLease::default();
            let mut queue = ActionQueue::default();
            let action = publish(&mut queue, &pool, &call, 1);
            assert_eq!(action.protected_row(), Err(VoiceError::Unauthorized));
            assert!(!action.same_protected_plan(&action));
            assert!(!action.handoff_status().acknowledged);
            let input = action.input().unwrap();
            let payload_bytes = allocation(&pool, &call).payload_bytes;
            action.admit_engine(action.engine_bytes()).unwrap();
            let tag = action.output_tag(&[]).unwrap();
            let receipt = action
                .with_result(&[], tag, || {
                    Ok(TerminalResult {
                        binding: ResultBinding {
                            pair: CallLease::default(),
                            attempt: None,
                            deadline: 16_000,
                            expires: 16_000,
                        },
                        outcome: CryptoOutcome::Derived,
                    })
                })
                .unwrap();
            let changed = CryptoResultReceipt {
                output_tag: [9; 32],
                ..receipt.clone()
            };
            assert_eq!(
                action.acknowledge_result(&changed),
                Err(VoiceError::RetryMismatch)
            );
            assert!(!action.handoff_status().acknowledged);
            if ack_first {
                action.acknowledge_result(&receipt).unwrap();
                assert!(action.handoff_status().acknowledged);
                assert!(action.input().is_ok());
                assert_eq!(pool.counts().1, 1);
                action.settle_engine();
            } else {
                action.settle_engine();
                assert!(!action.handoff_status().acknowledged);
                assert!(
                    action.input().is_ok(),
                    "settlement cannot acknowledge a lost result reply"
                );
                action.acknowledge_result(&receipt).unwrap();
            }
            assert!(action.input().is_err());
            let history = action.handoff_status();
            assert!(history.acknowledged && history.engine.settled);
            assert!(receipt.same_transition(history.engine.receipt.as_ref().unwrap()));
            assert!(matches!(input.as_input(), CryptoInput::Derive(_)));
            let held = pool.counts().0;
            drop(input);
            assert_eq!(pool.counts().0, held - payload_bytes);
            assert_eq!(
                action.recover_result(&[], tag).unwrap(),
                Some(receipt.clone())
            );
            assert!(queue.pull().is_empty());
            assert!(action.admit_engine(action.engine_bytes()).is_err());
            assert!(Arc::ptr_eq(
                &queue.get(Key::Derive([1; 32])).unwrap(),
                &action
            ));
            call.revoke();
            action.acknowledge_result(&receipt).unwrap();
            assert!(action.terminal_status().receipt.unwrap().retired);
            drop((action, queue));
            assert_eq!(pool.counts(), (0, 0));
        }
    }
    use catcoms_rt::ManualClock;

    // Deliberately tests queue/accounting in isolation. Real pair/proof factory coverage is in
    // presence/confirmation/tests.rs; these synthetic inputs make no endpoint authority claim.
    fn setup(seed: u8) -> PairSetup {
        PairSetup {
            handshake: [seed; 32],
            transcript: vec![seed; 64],
            peer: DeviceId::from_bytes([seed; 32]),
            peer_key: [seed; 65],
            local_is_low: true,
        }
    }
    fn allocation(pool: &Arc<Resources>, call: &CallLease) -> Allocation {
        Allocation {
            resources: pool.clone(),
            scope: Scope::for_group(b"queue test", Some([1; 32]), 44),
            call: call.clone(),
            budget: ByteBudget::Ordinary,
            history: None,
            payload_bytes: 320,
            engine_bytes: 4544,
        }
    }
    fn publish(
        queue: &mut ActionQueue,
        pool: &Arc<Resources>,
        call: &CallLease,
        seed: u8,
    ) -> Arc<CryptoAction> {
        publish_budget(queue, pool, call, seed, ByteBudget::Ordinary)
    }
    fn publish_budget(
        queue: &mut ActionQueue,
        pool: &Arc<Resources>,
        call: &CallLease,
        seed: u8,
        budget: ByteBudget,
    ) -> Arc<CryptoAction> {
        let mut allocation = allocation(pool, call);
        allocation.budget = budget;
        let prepared = queue.prepare(Key::Derive([seed; 32]), allocation).unwrap();
        let authority = CryptoAuthority::new(
            call.clone(),
            CallLease::default(),
            Arc::new(ManualClock::new(1000)),
            16_000,
            u64::MAX,
        )
        .unwrap();
        queue.publish_derive(prepared, setup(seed), authority)
    }

    fn finish(action: &CryptoAction, outcome: CryptoOutcome) -> CryptoResultReceipt {
        let tag = action.output_tag(&[]).unwrap();
        action
            .with_result(&[], tag, || {
                Ok(TerminalResult {
                    binding: ResultBinding {
                        pair: CallLease::default(),
                        attempt: None,
                        deadline: 16_000,
                        expires: 16_000,
                    },
                    outcome,
                })
            })
            .unwrap()
    }

    /// Injected providers must be free to inspect/revoke the real action and the native
    /// fence. An Engine-held Clock callback used to deadlock an adapter doing exactly this.
    #[derive(Debug)]
    struct AdmissionClock {
        inner: ManualClock,
        action: Mutex<Option<std::sync::Weak<CryptoAction>>>,
        native: Mutex<()>,
        invalidate: Mutex<Option<(bool, bool, CallLease)>>,
        armed: std::sync::atomic::AtomicBool,
        observations: std::sync::atomic::AtomicUsize,
    }
    impl AdmissionClock {
        fn observe(&self, monotonic: bool) {
            if self.armed.load(std::sync::atomic::Ordering::Acquire) {
                assert!(
                    self.native.try_lock().is_ok(),
                    "Clock ran beneath native fence"
                );
                let action = self
                    .action
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .upgrade()
                    .unwrap();
                assert!(
                    action.result_fence_available_for_test(),
                    "Clock ran beneath Engine"
                );
                let invalidation = {
                    let mut pending = self.invalidate.lock().unwrap();
                    if pending
                        .as_ref()
                        .is_some_and(|(edge, _, _)| *edge == monotonic)
                    {
                        pending.take()
                    } else {
                        None
                    }
                };
                if let Some((_, settle, lease)) = invalidation {
                    if settle {
                        action.settle_engine();
                    } else {
                        lease.revoke();
                    }
                }
                if !monotonic {
                    self.observations
                        .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                }
            }
        }
    }
    impl catcoms_rt::Clock for AdmissionClock {
        fn now_ms(&self) -> u64 {
            self.observe(false);
            self.inner.now_ms()
        }
        fn monotonic_ms(&self) -> u64 {
            self.observe(true);
            self.inner.monotonic_ms()
        }
        fn sleep(&self, duration: std::time::Duration) -> futures::future::BoxFuture<'_, ()> {
            self.inner.sleep(duration)
        }
    }
    fn admission_fixture() -> (
        Arc<CryptoAction>,
        Arc<AdmissionClock>,
        Arc<Resources>,
        CallLease,
    ) {
        let pool = Arc::new(Resources::default());
        let call = CallLease::default();
        let clock = Arc::new(AdmissionClock {
            inner: ManualClock::new(1000),
            action: Mutex::new(None),
            native: Mutex::new(()),
            invalidate: Mutex::new(None),
            armed: std::sync::atomic::AtomicBool::new(false),
            observations: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut queue = ActionQueue::default();
        let prepared = queue
            .prepare(Key::Derive([1; 32]), allocation(&pool, &call))
            .unwrap();
        let authority = CryptoAuthority::new(
            call.clone(),
            CallLease::default(),
            clock.clone(),
            16_000,
            u64::MAX,
        )
        .unwrap();
        let action = queue.publish_derive(prepared, setup(1), authority);
        *clock.action.lock().unwrap() = Some(Arc::downgrade(&action));
        clock
            .armed
            .store(true, std::sync::atomic::Ordering::Release);
        (action, clock, pool, call)
    }

    #[test]
    fn native_admission_samples_clock_before_engine_and_final_native_fence() {
        let (action, clock, pool, _) = admission_fixture();
        let before = pool.counts();
        // A native refusal has no first effect and cannot consume the action's eligibility.
        assert_eq!(
            action.with_engine_admission(action.engine_bytes(), |_| Err::<u64, _>(
                VoiceError::Unauthorized
            )),
            Err(VoiceError::Unauthorized)
        );
        assert_eq!(pool.counts(), before);
        for _ in 0..2 {
            let serial = action
                .with_engine_admission(action.engine_bytes(), |admission| {
                    let _native = clock.native.lock().unwrap();
                    admission.commit()
                })
                .unwrap();
            assert_eq!(serial, action.serial());
            assert_eq!(pool.counts().1, 1, "exact retry shares the original engine");
        }
        assert_eq!(
            clock
                .observations
                .load(std::sync::atomic::Ordering::Acquire),
            3
        );
        action.settle_engine();
        assert_eq!(pool.counts().1, 0);
        assert!(action.admit_engine(action.engine_bytes()).is_err());
    }

    fn deadline_action() -> (Arc<CryptoAction>, Arc<ManualClock>, Arc<Resources>) {
        let pool = Arc::new(Resources::default());
        let call = CallLease::default();
        let clock = Arc::new(ManualClock::new(1000));
        let mut queue = ActionQueue::default();
        let prepared = queue
            .prepare(Key::Derive([1; 32]), allocation(&pool, &call))
            .unwrap();
        let authority =
            CryptoAuthority::new(call, CallLease::default(), clock.clone(), 16_000, u64::MAX)
                .unwrap();
        let action = queue.publish_derive(prepared, setup(1), authority);
        (action, clock, pool)
    }

    #[test]
    fn engine_contended_after_final_clock_refuses_before_unlock_and_never_installs_an_expired_permit(
    ) {
        use std::sync::mpsc;
        let (action, clock, pool) = deadline_action();
        let before = pool.counts();
        clock.set_ms(15_999);
        std::thread::scope(|scope| {
            let (ready, sampled) = mpsc::channel();
            let (proceed, resume) = mpsc::channel();
            let (done, result) = mpsc::channel();
            let actual = action.clone();
            let worker = scope.spawn(move || {
                // This test-only barrier sits in the trusted final callback, AFTER the
                // actual original Clock sample. No Clock callback blocks or takes Engine.
                let admission = actual.with_engine_admission(actual.engine_bytes(), |step| {
                    ready.send(()).unwrap();
                    resume.recv().unwrap();
                    step.commit()
                });
                done.send(admission).unwrap();
            });
            sampled
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            let engine = action.engine.lock().unwrap();
            clock.advance_ms(2);
            proceed.send(()).unwrap();
            let refusal = result.recv_timeout(std::time::Duration::from_secs(5));
            // Always unlock before join, including a regression back to blocking admission.
            assert!(engine.permit.is_none());
            drop(engine);
            worker.join().unwrap();
            assert_eq!(refusal.unwrap(), Err(VoiceError::Capacity));
        });
        assert_eq!(pool.counts(), before);
        assert_eq!(
            action.admit_engine(action.engine_bytes()),
            Err(VoiceError::Unauthorized)
        );
        assert!(action.engine.lock().unwrap().permit.is_none());
    }

    #[test]
    fn final_engine_commit_needs_no_budget_lock_after_its_original_clock_sample() {
        use std::sync::mpsc;
        let (action, clock, pool) = deadline_action();
        clock.set_ms(15_999);
        std::thread::scope(|scope| {
            let (ready, sampled) = mpsc::channel();
            let (proceed, resume) = mpsc::channel();
            let (done, result) = mpsc::channel();
            let actual = action.clone();
            let worker = scope.spawn(move || {
                let admission = actual.with_engine_admission(actual.engine_bytes(), |step| {
                    ready.send(()).unwrap();
                    resume.recv().unwrap();
                    step.commit()
                });
                done.send(admission).unwrap();
            });
            sampled
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            let capacity = pool.lock_capacity_for_test();
            proceed.send(()).unwrap();
            let admitted = result.recv_timeout(std::time::Duration::from_secs(5));
            // A regression must not strand this test: cross the deadline before releasing
            // Pool. Any permit obtained only after that wait is stale and fails the assertion.
            clock.advance_ms(2);
            drop(capacity);
            worker.join().unwrap();
            assert_eq!(admitted.unwrap(), Ok(action.serial()));
        });
        assert_eq!(pool.counts().1, 1);
        action.settle_engine();
        assert_eq!(pool.counts().1, 0);
    }

    #[test]
    fn simultaneous_exact_admissions_keep_one_permit_and_retire_unused_candidates() {
        let (action, _, pool) = deadline_action();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for _ in 0..2 {
                workers.push(scope.spawn(|| {
                    action.with_engine_admission(action.engine_bytes(), |step| {
                        barrier.wait();
                        step.commit()
                    })
                }));
            }
            for worker in workers {
                match worker.join().unwrap() {
                    Ok(serial) => assert_eq!(serial, action.serial()),
                    Err(VoiceError::Capacity) => {}
                    Err(error) => panic!("unexpected exact-admission result: {error:?}"),
                }
            }
        });
        assert_eq!(
            action.admit_engine(action.engine_bytes()),
            Ok(action.serial())
        );
        assert_eq!(pool.counts().1, 1);
        action.settle_engine();
        assert_eq!(pool.counts().1, 0);
    }

    #[test]
    fn local_capacity_refusal_recovers_the_exact_permit_published_after_preflight() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        let pool = Arc::new(Resources::default());
        let call = CallLease::default();
        let sizing = allocation(&pool, &call);
        // Exactly one action's history/input plus ONE engine: once B prepares its credit,
        // A's stale preflight can only get Capacity from this actual original reservation.
        let reserve = pool
            .reserve(
                sizing.scope.group,
                RECORD_BYTES + CONFIRM_BYTES + sizing.payload_bytes + sizing.engine_bytes,
            )
            .unwrap();
        let mut queue = ActionQueue::default();
        let action = publish_budget(
            &mut queue,
            &pool,
            &call,
            1,
            ByteBudget::Reserved(reserve.clone()),
        );
        std::thread::scope(|scope| {
            let (prepared, ready) = mpsc::channel();
            let (publish, proceed) = mpsc::channel();
            let (published, original_result) = mpsc::channel();
            let original = action.clone();
            let b = scope.spawn(move || {
                let result = original.with_engine_admission(original.engine_bytes(), |step| {
                    prepared.send(()).unwrap();
                    proceed.recv().unwrap();
                    step.commit()
                });
                published.send(result).unwrap();
            });
            ready
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            let capacity = reserve.lock_capacity_for_test();
            let before = Arc::strong_count(&reserve);
            let original = action.clone();
            let a = scope.spawn(move || original.admit_engine(original.engine_bytes()));
            // The only new reference is A's original lender clone, which it must acquire
            // after observing permitNone and before blocking on the held reservation.
            let stop = Arc::new(AtomicBool::new(false));
            let (cloned, observed) = mpsc::channel();
            let observed_stop = stop.clone();
            let original_reserve = &reserve;
            let observer = scope.spawn(move || {
                while !observed_stop.load(Ordering::Acquire) {
                    if Arc::strong_count(original_reserve) > before {
                        cloned.send(()).unwrap();
                        return;
                    }
                    std::thread::yield_now();
                }
            });
            let observation = observed.recv_timeout(std::time::Duration::from_secs(5));
            stop.store(true, Ordering::Release);
            observer.join().unwrap();
            publish.send(()).unwrap();
            let first = original_result.recv_timeout(std::time::Duration::from_secs(5));
            drop(capacity);
            b.join().unwrap();
            let second = a.join().unwrap();
            observation.unwrap();
            let first = first.unwrap();
            assert_eq!(first, Ok(action.serial()));
            assert_eq!(second, first);
        });
        assert_eq!(pool.counts().1, 1);
        action.settle_engine();
        assert_eq!(pool.counts().1, 0);
    }

    #[test]
    fn handoff_snapshot_samples_clock_outside_engine_and_ack_remains_independent() {
        let (action, clock, _, call) = admission_fixture();
        // The fixture Clock asserts Engine and native gates are available on EVERY callback.
        assert!(!action.handoff_status().acknowledged);
        action.admit_engine(action.engine_bytes()).unwrap();
        clock
            .armed
            .store(false, std::sync::atomic::Ordering::Release);
        let receipt = finish(&action, CryptoOutcome::Derived);
        clock
            .armed
            .store(true, std::sync::atomic::Ordering::Release);
        let status = action.handoff_status();
        assert!(!status.acknowledged && !status.engine.settled);
        assert!(receipt.same_transition(status.engine.receipt.as_ref().unwrap()));
        call.revoke();
        action.settle_engine();
        let status = action.handoff_status();
        assert!(status.engine.settled);
        assert!(status.engine.receipt.unwrap().retired);
        assert!(
            !status.acknowledged,
            "retirement and settlement are not an ACK"
        );
        action.acknowledge_result(&receipt).unwrap();
        assert!(action.handoff_status().acknowledged);
        assert!(action.input().is_err());
        assert!(!action.is_pending());
    }

    #[test]
    fn final_clock_revocation_or_settlement_before_commit_cannot_admit() {
        for monotonic in [false, true] {
            for settle in [false, true] {
                let (action, clock, pool, call) = admission_fixture();
                *clock.invalidate.lock().unwrap() = Some((monotonic, settle, call.clone()));
                assert_eq!(
                    action.with_engine_admission(action.engine_bytes(), |step| {
                        let _native = clock.native.lock().unwrap();
                        step.commit()
                    }),
                    Err(VoiceError::Unauthorized)
                );
                assert_eq!(
                    pool.counts().1,
                    0,
                    "neither Clock callback can revive admission"
                );
                if settle {
                    assert!(action.is_settled());
                } else {
                    assert!(!call.is_live());
                }
            }
        }
        for during_clock in [false, true] {
            let (action, clock, pool, call) = admission_fixture();
            if during_clock {
                *clock.invalidate.lock().unwrap() = Some((false, false, call.clone()));
            }
            let result = action.with_engine_admission(action.engine_bytes(), |admission| {
                let _native = clock.native.lock().unwrap();
                if !during_clock {
                    call.revoke();
                }
                admission.commit()
            });
            assert_eq!(result, Err(VoiceError::Unauthorized));
            assert_eq!(pool.counts().1, 0);
        }
        let (action, clock, pool, _) = admission_fixture();
        assert_eq!(
            action.with_engine_admission(action.engine_bytes(), |admission| {
                let _native = clock.native.lock().unwrap();
                action.settle_engine();
                admission.commit()
            }),
            Err(VoiceError::Unauthorized)
        );
        assert_eq!(pool.counts().1, 0);
    }

    #[test]
    fn actual_result_preparation_retains_engine_and_input_through_concurrent_settlement() {
        for unwind in [false, true] {
            let pool = Arc::new(Resources::default());
            let call = CallLease::default();
            let mut queue = ActionQueue::default();
            let action = publish(&mut queue, &pool, &call, 1);
            assert!(
                action.begin_result_preparation().is_err(),
                "unadmitted engines cannot prepare"
            );
            action.admit_engine(action.engine_bytes()).unwrap();
            let before = pool.counts();
            assert_eq!(before.1, 1);
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let preparation = action.begin_result_preparation().unwrap();
                assert_eq!(
                    action.begin_result_preparation().err(),
                    Some(VoiceError::Capacity)
                );
                // A real competing settlement request permanently fences a first result, but
                // cannot report actual drainage or refund this loan's running copies/work.
                std::thread::scope(|scope| {
                    scope.spawn(|| action.settle_engine()).join().unwrap();
                });
                assert_eq!(pool.counts(), before);
                assert!(!action.is_settled());
                assert!(!action.terminal_status().settled);
                assert!(action.input().is_ok());
                assert!(action.admit_engine(action.engine_bytes()).is_err());
                assert!(action
                    .with_result(&[], action.output_tag(&[]).unwrap(), || panic!(
                        "settlement won"
                    ))
                    .is_err());
                if unwind {
                    panic!("preparation aborted");
                }
                drop(preparation);
            }));
            assert_eq!(run.is_err(), unwind);
            assert_eq!(pool.counts().1, 0);
            assert!(action.is_settled() && action.terminal_status().settled);
            assert!(action.input().is_err());
            assert!(action.begin_result_preparation().is_err());
        }
    }

    #[test]
    fn completed_preparation_does_not_settle_or_refund_a_running_browser_engine() {
        let pool = Arc::new(Resources::default());
        let call = CallLease::default();
        let mut queue = ActionQueue::default();
        let action = publish(&mut queue, &pool, &call, 1);
        action.admit_engine(action.engine_bytes()).unwrap();
        let before = pool.counts();
        drop(action.begin_result_preparation().unwrap());
        assert_eq!(pool.counts(), before);
        assert!(!action.is_settled());
        let next = action.begin_result_preparation().unwrap();
        let receipt = finish(&action, CryptoOutcome::Derived);
        action.acknowledge_result(&receipt).unwrap();
        action.settle_engine();
        assert_eq!(pool.counts().1, 1);
        assert!(action.input().is_ok());
        drop(next);
        assert_eq!(pool.counts().1, 0);
        assert!(action.input().is_err());
        assert!(receipt.same_transition(&action.terminal_status().receipt.unwrap()));
    }

    #[test]
    fn settled_history_does_not_pin_exchange_but_a_real_input_reader_does() {
        for hold_input in [false, true] {
            let pool = Arc::new(Resources::default());
            let call = CallLease::default();
            let mut queue = ActionQueue::default();
            let exchange = super::super::negotiation::Exchange::new_for_test(
                pool.clone(),
                allocation(&pool, &call).scope.group,
                queue.slots.clone(),
            )
            .unwrap();
            let weak = Arc::downgrade(&exchange);
            let action = publish_budget(&mut queue, &pool, &call, 1, exchange.encryption_budget());
            let input = hold_input.then(|| action.input().unwrap());
            action.admit_engine(action.engine_bytes()).unwrap();
            let receipt = finish(&action, CryptoOutcome::Derived);
            action.acknowledge_result(&receipt).unwrap();
            action.settle_engine();
            exchange.close();
            drop(exchange);
            assert_eq!(weak.upgrade().is_some(), hold_input);
            drop(input);
            assert!(
                weak.upgrade().is_none(),
                "fixed history cannot hold a live exchange"
            );
            assert_eq!(pool.counts(), (RECORD_BYTES + CONFIRM_BYTES, 0));
            assert_eq!(action.terminal_status().receipt, Some(receipt.clone()));
            assert_eq!(
                action.recover_result(&[], receipt.output_tag).unwrap(),
                Some(receipt)
            );
            assert!(action.admit_engine(action.engine_bytes()).is_err());
            drop((action, queue));
            assert_eq!(pool.counts(), (0, 0));
        }
    }

    #[test]
    fn unstarted_retirement_drops_lender_and_input_without_restoring_delayed_admission() {
        for settle_first in [false, true] {
            let pool = Arc::new(Resources::default());
            let call = CallLease::default();
            let mut queue = ActionQueue::default();
            let action = publish(&mut queue, &pool, &call, 1);
            let input = action.input().unwrap();
            if settle_first {
                action.settle_engine();
            }
            action.retire_if_unstarted();
            action.retire_if_unstarted();
            assert!(action.input().is_err());
            assert!(action.admit_engine(action.engine_bytes()).is_err());
            assert!(action.terminal_status().receipt.is_none());
            assert!(queue.pull().is_empty());
            drop(input);
            assert_eq!(pool.counts(), (RECORD_BYTES + CONFIRM_BYTES, 0));
        }
    }

    #[test]
    fn unstarted_exchange_retirement_keeps_only_actual_input_reader_and_fixed_history() {
        let pool = Arc::new(Resources::default());
        let call = CallLease::default();
        let mut queue = ActionQueue::default();
        let exchange = super::super::negotiation::Exchange::new_for_test(
            pool.clone(),
            allocation(&pool, &call).scope.group,
            queue.slots.clone(),
        )
        .unwrap();
        let weak = Arc::downgrade(&exchange);
        let action = publish_budget(&mut queue, &pool, &call, 1, exchange.encryption_budget());
        let reader = action.input().unwrap();
        action.retire_if_unstarted();
        assert!(action.input().is_err());
        assert!(action.admit_engine(action.engine_bytes()).is_err());
        exchange.close();
        drop(exchange);
        assert!(
            weak.upgrade().is_some(),
            "actual reader still owns the input phase"
        );
        assert!(matches!(reader.as_input(), CryptoInput::Derive(_)));
        drop(reader);
        assert!(
            weak.upgrade().is_none(),
            "retired lender and history cannot pin the phase"
        );
        assert_eq!(pool.counts(), (RECORD_BYTES + CONFIRM_BYTES, 0));
        assert!(action.is_settled());
        assert!(action.terminal_status().receipt.is_none());
        drop((action, queue));
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn retirement_preserves_uncertain_engine_and_lost_ack_input_for_both_outcomes() {
        for outcome in [
            CryptoOutcome::Derived,
            CryptoOutcome::Rejected(VoiceError::Unauthorized),
        ] {
            let pool = Arc::new(Resources::default());
            let call = CallLease::default();
            let mut queue = ActionQueue::default();
            let action = publish(&mut queue, &pool, &call, 1);
            action.admit_engine(action.engine_bytes()).unwrap();
            let before = pool.counts();
            action.retire_if_unstarted();
            assert_eq!(pool.counts(), before);
            assert!(!action.is_settled());
            let receipt = finish(&action, outcome);
            action.settle_engine();
            let before = pool.counts();
            action.retire_if_unstarted();
            assert_eq!(pool.counts(), before);
            assert!(action.input().is_ok(), "lost ACK is not unstarted work");
            assert_eq!(action.terminal_status().receipt, Some(receipt.clone()));
            action.acknowledge_result(&receipt).unwrap();
            assert!(action.input().is_err());
        }
    }

    #[test]
    fn published_direction_never_rearms_and_unpublished_abandonment_returns_its_slot() {
        let pool = Arc::new(Resources::default());
        let call = CallLease::default();
        let mut queue = ActionQueue::default();
        let exchange = super::super::negotiation::Exchange::new_for_test(
            pool.clone(),
            allocation(&pool, &call).scope.group,
            queue.slots.clone(),
        )
        .unwrap();
        let mut first = allocation(&pool, &call);
        first.budget = exchange.encryption_budget();
        drop(queue.prepare(Key::Derive([1; 32]), first).unwrap());
        let action = publish_budget(&mut queue, &pool, &call, 2, exchange.encryption_budget());
        action.admit_engine(action.engine_bytes()).unwrap();
        let receipt = finish(&action, CryptoOutcome::Derived);
        action.acknowledge_result(&receipt).unwrap();
        action.settle_engine();
        let mut duplicate_direction = allocation(&pool, &call);
        duplicate_direction.budget = exchange.encryption_budget();
        let before = pool.counts();
        assert!(queue
            .prepare(Key::Derive([3; 32]), duplicate_direction)
            .is_err());
        assert_eq!(pool.counts(), before);
        // A different direction still has its actual protected slot. The derive input here
        // tests capacity in isolation; it does not claim SDP or endpoint authority.
        let open = publish_budget(&mut queue, &pool, &call, 4, exchange.decryption_budget());
        open.retire_if_unstarted();
        exchange.close();
        drop((action, open, queue, exchange));
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn racing_unstarted_retirement_and_admission_cannot_reopen_a_settled_lender() {
        for _ in 0..16 {
            let pool = Arc::new(Resources::default());
            let call = CallLease::default();
            let mut queue = ActionQueue::default();
            let action = publish(&mut queue, &pool, &call, 1);
            let barrier = std::sync::Barrier::new(2);
            std::thread::scope(|scope| {
                let admission = scope.spawn(|| {
                    barrier.wait();
                    action.admit_engine(action.engine_bytes())
                });
                barrier.wait();
                action.retire_if_unstarted();
                let admitted = admission.join().unwrap().is_ok();
                assert_eq!(pool.counts().1, usize::from(admitted));
                assert_eq!(action.is_settled(), !admitted);
            });
            // This is the actual final settlement, including when admission won the race.
            // There is no result receipt in this failed/unstarted path and none can be added.
            action.settle_engine();
            action.retire_if_unstarted();
            assert!(action.input().is_err());
            assert!(action.admit_engine(action.engine_bytes()).is_err());
            assert_eq!(pool.counts(), (RECORD_BYTES + CONFIRM_BYTES, 0));
        }
    }

    #[test]
    fn combined_queue_keeps_terminal_fences_bounded_and_pull_rotates_without_dequeue() {
        let pool = Arc::new(Resources::default());
        let call = CallLease::default();
        let mut queue = ActionQueue::default();
        let actions: Vec<_> = (0..32)
            .map(|seed| publish(&mut queue, &pool, &call, seed))
            .collect();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..8 {
            let pull = queue.pull();
            assert_eq!(pull.len(), 4);
            for action in pull {
                assert!(seen.insert(action.serial()));
            }
        }
        assert_eq!(seen.len(), 32);
        assert_eq!(queue.pull()[0].serial(), 1);
        for action in &actions {
            action.settle_engine();
        }
        assert!(queue.pull().is_empty());
        let before = pool.counts();
        assert!(queue
            .prepare(Key::Derive([99; 32]), allocation(&pool, &call))
            .is_err());
        assert_eq!(pool.counts(), before);
        assert_eq!(queue.serial, 32);
        assert!(Arc::ptr_eq(
            &queue.get(Key::Derive([0; 32])).unwrap(),
            &actions[0]
        ));
        drop(queue);
        assert_eq!(
            pool.counts(),
            before,
            "held terminal owners retain input/history custody"
        );
        drop(actions);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn settlement_waits_for_actual_first_commit_and_permanently_closes_followups() {
        let pool = Arc::new(Resources::default());
        let call = CallLease::default();
        let mut queue = ActionQueue::default();
        let action = publish(&mut queue, &pool, &call, 1);
        action.admit_engine(action.engine_bytes()).unwrap();
        let release = std::sync::Barrier::new(2);
        let effects = std::sync::atomic::AtomicUsize::new(0);
        let (entered, entered_rx) = std::sync::mpsc::channel();
        let (settling, settling_rx) = std::sync::mpsc::channel();
        let (settled, settled_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let action = &action;
            let effects = &effects;
            let release = &release;
            scope.spawn(move || {
                action
                    .with_pending(|| {
                        entered.send(()).unwrap();
                        release.wait();
                        effects.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                        Ok(())
                    })
                    .unwrap()
            });
            entered_rx.recv().unwrap();
            scope.spawn(move || {
                settling.send(()).unwrap();
                action.settle_engine();
                settled.send(()).unwrap();
            });
            settling_rx.recv().unwrap();
            assert!(settled_rx.try_recv().is_err());
            assert_eq!(pool.counts().1, 1);
            release.wait();
        });
        settled_rx.recv().unwrap();
        assert_eq!(effects.load(std::sync::atomic::Ordering::Acquire), 1);
        assert_eq!(pool.counts().1, 0);
        assert!(action
            .with_pending(|| {
                effects.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                Ok(())
            })
            .is_err());
        assert_eq!(effects.load(std::sync::atomic::Ordering::Acquire), 1);
    }

    #[test]
    fn native_derive_tags_bind_typed_inputs_scope_serial_and_version() {
        let pool = Arc::new(Resources::default());
        let call = CallLease::default();
        let mut queue = ActionQueue::default();
        let mut prepared = queue
            .prepare(Key::Derive([1; 32]), allocation(&pool, &call))
            .unwrap();
        let expected = tag(&prepared, &Input::Derive(setup(1)));
        let mut variants = Vec::new();
        let mut different = setup(1);
        different.handshake[0] ^= 1;
        variants.push(different);
        let mut different = setup(1);
        different.transcript.push(2);
        variants.push(different);
        let mut different = setup(1);
        different.peer = DeviceId::from_bytes([2; 32]);
        variants.push(different);
        let mut different = setup(1);
        different.peer_key[0] ^= 1;
        variants.push(different);
        let mut different = setup(1);
        different.local_is_low = false;
        variants.push(different);
        for variant in variants {
            assert_ne!(tag(&prepared, &Input::Derive(variant)), expected);
        }
        prepared.serial = 9_007_199_254_740_993;
        assert_ne!(tag(&prepared, &Input::Derive(setup(1))), expected);
        prepared.serial = 1;
        prepared.scope.room += 1;
        assert_ne!(tag(&prepared, &Input::Derive(setup(1))), expected);
        assert!(derive_bytes(4097).is_err());
        for (aad, ciphertext) in [(4097, 16), (0, 15), (0, 529), (usize::MAX, usize::MAX)] {
            assert!(confirmation_bytes(aad, ciphertext).is_err());
        }
    }

    #[test]
    fn serial_exhaustion_and_unfunded_prepare_do_not_publish_or_reset_fences() {
        let pool = Arc::new(Resources::default());
        let call = CallLease::default();
        let mut queue = ActionQueue {
            serial: u64::MAX,
            ..ActionQueue::default()
        };
        assert!(queue
            .prepare(Key::Derive([1; 32]), allocation(&pool, &call))
            .is_err());
        assert!(queue.actions.is_empty());
        assert_eq!(pool.counts(), (0, 0));
        queue.serial = 0;
        let pressure = pool
            .reserve(
                allocation(&pool, &call).scope.group,
                crate::voice_v2::resources::GROUP_BYTES,
            )
            .unwrap();
        assert!(queue
            .prepare(Key::Derive([1; 32]), allocation(&pool, &call))
            .is_err());
        assert_eq!(queue.serial, 0);
        drop(pressure);
        let abandoned = queue
            .prepare(Key::Derive([1; 32]), allocation(&pool, &call))
            .unwrap();
        drop(abandoned);
        assert_eq!(
            queue.serial, 1,
            "allocated but abandoned identities are burned"
        );
        let action = publish(&mut queue, &pool, &call, 1);
        assert_eq!(action.serial(), 2);
    }
}
