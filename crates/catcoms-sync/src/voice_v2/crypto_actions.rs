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
mod results;
pub(crate) mod slots;
pub use results::{CryptoOutcome, CryptoResultReceipt, CryptoResultStatus};
pub(super) use results::{ResultBinding, TerminalResult};

/// Closed operation identity; the native factory selects it from actual protocol state.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CryptoOperation {
    Derive,
    EncryptConfirmation,
    DecryptConfirmation,
}

/// Borrowed immutable input. An adapter must separately charge any serialized/pulled copies;
/// extracting an uncharged owned payload from this native owner is deliberately unsupported.
pub enum CryptoInput<'a> {
    Derive(&'a PairSetup),
    EncryptConfirmation(&'a CryptoFlight),
    DecryptConfirmation(&'a InboundPacket),
}

/// Owned capacity lease for borrowed native input. Compaction can remove the action's own
/// reference, but a serializer/worker still holding this lease retains the original buffers
/// and their credit. This grants no engine, signing, packet or RTC authority.
pub struct CryptoInputLease(Arc<ActionPayload>);
impl CryptoInputLease {
    pub fn as_input(&self) -> CryptoInput<'_> {
        match &self.0.input {
            Input::Derive(input) => CryptoInput::Derive(input),
            Input::Encrypt(input) => CryptoInput::EncryptConfirmation(input),
            Input::Decrypt(input) => CryptoInput::DecryptConfirmation(input),
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
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Key {
    Derive(Nonce),
    Encrypt(Nonce, u64, Nonce),
    Decrypt(u64),
}

#[derive(Default)]
struct Engine {
    settled: bool,
    // Lending authority is pending state, not permanent history. Taking it under this same
    // fence prevents a delayed admission from rebuilding capacity after actual settlement.
    lender: Option<ByteBudget>,
    permit: Option<Credit>,
    terminal: Option<results::RecordedResult>,
    acknowledged: bool,
}

/// Opaque registered action. Clones preserve the exact engine fence and its actual charge.
/// Retirement/expiry prevents new effects but never refunds a running/uncertain engine slot.
pub struct CryptoAction {
    serial: u64,
    input_tag: [u8; 32],
    scope: Scope,
    call: CallLease,
    authority: CryptoAuthority,
    // Lock order is Engine -> payload. An input lease clones its Arc under this mutex and
    // releases the guard before any engine operation. Actual payload/credit drops occur after
    // releasing BOTH locks, so downstream destructors cannot reenter an action fence.
    input: Mutex<Option<Arc<ActionPayload>>>,
    operation: CryptoOperation,
    output_bytes: usize,
    resources: Arc<Resources>,
    engine_bytes: usize,
    engine: Mutex<Engine>,
    _record: Credit,
    _slot: slots::Slot,
}
redacted_debug!(
    CryptoAction,
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
    pub fn engine_bytes(&self) -> usize {
        self.engine_bytes
    }
    pub fn is_settled(&self) -> bool {
        self.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .settled
    }
    pub fn is_pending(&self) -> bool {
        let engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        !engine.settled && engine.terminal.is_none() && self.authority.authorized()
    }

    /// The trusted native adapter calls this only for an already registered exact action.
    /// Its immutable serial is the permit identity inside that exact native owner; duplicates
    /// return it without charging again. Capacity is transient and leaves the action eligible.
    pub fn admit_engine(&self, bytes: usize) -> Result<u64, VoiceError> {
        let mut engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        if bytes != self.engine_bytes {
            return Err(VoiceError::RetryMismatch);
        }
        if engine.settled || engine.terminal.is_some() || !self.authority.authorized() {
            return Err(VoiceError::Unauthorized);
        }
        if engine.permit.is_none() {
            engine.permit = Some(
                engine
                    .lender
                    .as_ref()
                    .ok_or(VoiceError::Unauthorized)?
                    .charge(
                        &self.resources,
                        self.scope.group,
                        bytes,
                        ResourceKind::Crypto,
                    )?,
            );
        }
        Ok(self.serial)
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
                engine.permit.take(),
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
        if engine.settled && (engine.terminal.is_none() || engine.acknowledged) {
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
}

pub(super) struct Allocation {
    pub resources: Arc<Resources>,
    pub scope: Scope,
    pub call: CallLease,
    pub budget: ByteBudget,
    pub payload_bytes: usize,
    pub engine_bytes: usize,
}

impl ActionQueue {
    pub(super) fn with_slots(slots: Arc<slots::Slots>) -> Self {
        Self {
            slots,
            ..Self::default()
        }
    }
    pub(super) fn get(&self, key: Key) -> Option<Arc<CryptoAction>> {
        self.actions.get(&key).cloned()
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
            payload_bytes,
            engine_bytes,
        } = allocation;
        if self.actions.len() >= ACTIONS {
            return Err(VoiceError::Capacity);
        }
        let serial = self.serial.checked_add(1).ok_or(VoiceError::Capacity)?;
        let slot = match &budget {
            ByteBudget::Exchange { exchange, .. } => exchange.action_slot(&self.slots)?,
            _ => self.slots.charge(1)?,
        };
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
            ByteBudget::Ordinary | ByteBudget::Reserved(_) | ByteBudget::ReservedLane { .. } => {
                // These closed variants have no phase/workflow custody on retained charges.
                // Their existing conserved partition is safe; Exchange is deliberately handled
                // above before any phase owner can be attached. A future Workflow variant must
                // supply independently prepared history rather than enter this branch.
                let bytes = payload_bytes
                    .checked_add(RECORD_BYTES + CONFIRM_BYTES)
                    .ok_or(VoiceError::Bounds)?;
                let mut record =
                    budget.charge(&resources, scope.group, bytes, ResourceKind::Retained)?;
                let payload = record.split_retained(payload_bytes)?;
                (record, payload, None)
            }
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
        })
    }

    fn publish(
        &mut self,
        prepared: PreparedAction,
        input: Input,
        authority: CryptoAuthority,
    ) -> Arc<CryptoAction> {
        let input_tag = tag(&prepared, &input);
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
        };
        let action = Arc::new(CryptoAction {
            serial: prepared.serial,
            input_tag,
            scope: prepared.scope,
            call: prepared.call,
            authority,
            input: Mutex::new(Some(Arc::new(ActionPayload {
                input,
                _credit: prepared.payload,
            }))),
            operation,
            output_bytes,
            resources: prepared.resources,
            engine_bytes: prepared.engine_bytes,
            engine: Mutex::new(Engine {
                lender: Some(prepared.budget),
                ..Engine::default()
            }),
            _record: prepared.record,
            _slot: prepared.slot,
        });
        if let Some(issue) = prepared.issue {
            issue.publish();
        }
        assert!(
            self.actions.insert(prepared.key, action.clone()).is_none(),
            "native action uniqueness"
        );
        action
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
        Input::Decrypt(_) => unreachable!(),
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
            if ack_first {
                action.acknowledge_result(&receipt).unwrap();
                assert!(action.input().is_ok());
                assert_eq!(pool.counts().1, 1);
                action.settle_engine();
            } else {
                action.settle_engine();
                assert!(
                    action.input().is_ok(),
                    "settlement cannot acknowledge a lost result reply"
                );
                action.acknowledge_result(&receipt).unwrap();
            }
            assert!(action.input().is_err());
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
            .reserve(allocation(&pool, &call).scope.group, 256 * 1024)
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
