//! Native three-flight key confirmation. This owner holds the exact pending action, reservation
//! and accepted signed-flight hash: async browser work cannot sign/establish a superseded attempt.
//! Plaintext reaches this machine only through an affine, revalidated PacketLedger receipt.
//! Discovery evidence is required once at pair admission. Its later refresh/expiry is deliberately
//! independent of the pair pin; a failed newer join must not evict a working confirmed peer.

use super::*;
use crate::voice_v2::crypto_actions::{
    self, Allocation, CryptoAction, CryptoInput, CryptoOperation, CryptoOutcome,
    CryptoResultReceipt, Key, ResultBinding, TerminalResult,
};
use crate::voice_v2::packets::{
    InboundPacket, PairBinding, ReserveRequest, ReservedPacket, ValidatedInbound,
};
use crate::voice_v2::resources::ByteBudget;

const MAX_PAIRS: usize = 16;
const MAX_CONFIRMING: usize = 8;
const PAIR_BYTES: usize = 6144;
const CONFIRM_DOMAIN: &str = "mewtual/voice/key-confirm/v2";

#[derive(Default)]
pub(super) struct Confirmations {
    pub(super) pairs: BTreeMap<Nonce, Pair>,
    serial: u64,
}
pub(super) struct Pair {
    binding: PairBinding,
    generation: u64,
    transcript: Vec<u8>,
    peer_key: [u8; 65],
    pin: CallLease,
    pub(super) deadline: u64,
    pub(super) attempt: RevocationOwner,
    low: bool,
    starts: u8,
    action_serial: u64,
    low_nonce: Option<Nonce>,
    high_nonce: Option<Nonce>,
    hello: super::super::crypto_authority::SequenceGate,
    accepted_hello: Option<[u8; 32]>,
    accepted_proof: Option<[u8; 32]>,
    accepted_finish: Option<[u8; 32]>,
    action: Option<Action>,
    established: bool,
    retired: bool,
    // Permanent preparation stamp, including after close. Removing it would let the same pair
    // recreate candidate/operation capacity. The closed handle owns no idle progress capacity.
    chain: Option<Arc<super::super::chain::PreparedChain>>,
    _credit: Credit,
}
struct Action {
    serial: u64,
    kind: EncryptedKind,
    plaintext: Vec<u8>,
    draft: Option<ReservedPacket>,
    signed: Option<SignedObject>,
    created: u64,
    live: RevocationOwner,
}
impl Drop for Pair {
    fn drop(&mut self) {
        self.pin.revoke();
        if let Some(chain) = &self.chain {
            chain.close();
        }
    }
}
impl Confirmations {
    pub(super) fn established(&self, handshake: Nonce) -> bool {
        self.pairs
            .get(&handshake)
            .is_some_and(|pair| pair.established && !pair.retired && pair.pin.is_live())
    }
    pub(super) fn source_deadline(&self, handshake: Nonce) -> Result<u64, VoiceError> {
        let pair = self.pairs.get(&handshake).ok_or(VoiceError::Unauthorized)?;
        if pair.retired || !pair.pin.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(pair.deadline)
    }
    pub(super) fn retire_member(&mut self, member: DeviceId) {
        for pair in self
            .pairs
            .values_mut()
            .filter(|pair| pair.binding.peer == member)
        {
            pair.pin.revoke();
            pair.retired = true;
            if let Some(chain) = &pair.chain {
                chain.close();
            }
        }
    }
}

/// Native-selected key derivation input. Re-derivation does not recreate confirmation or counters.
#[derive(Clone)]
pub struct PairSetup {
    pub handshake: Nonce,
    pub transcript: Vec<u8>,
    pub peer: DeviceId,
    pub peer_key: [u8; 65],
    pub local_is_low: bool,
}
/// One exact native action/reservation. UI only encrypts these frozen bytes and submits its
/// ciphertext with the same action and reservation identities; neither identity authorizes a call.
pub struct CryptoFlight {
    pub handshake: Nonce,
    pub action: u64,
    pub reservation: ReservedPacket,
    pub plaintext: Vec<u8>,
}
pub enum ConfirmationOutput {
    Encrypt(CryptoFlight),
    Send(Box<SignedObject>),
}
pub enum ReceiveProgress {
    Decrypt(Box<InboundPacket>),
    Retry(Nonce),
    Confirmed,
}
/// Exact outbound action/reservation tuple, independently of its async browser ciphertext.
pub struct FlightSubmission<'a> {
    pub handshake: Nonce,
    pub action: u64,
    pub reservation: MessageId,
    pub ciphertext: &'a [u8],
}
/// Immutable native-authenticated encrypted tuple and its trusted WebCrypto result. Native
/// compares all encrypted fields against the retained token before consuming closed plaintext.
pub struct OpenedFlight<'a> {
    pub token: u64,
    pub aad: &'a [u8],
    pub sequence: u64,
    pub ciphertext: &'a [u8],
    pub plaintext: &'a [u8],
}
redacted_debug!(
    PairSetup,
    CryptoFlight,
    ConfirmationOutput,
    ReceiveProgress,
    FlightSubmission<'_>,
    OpenedFlight<'_>
);

fn confirmation(kind: EncryptedKind) -> bool {
    matches!(
        kind,
        EncryptedKind::ConfirmHello | EncryptedKind::ConfirmProof | EncryptedKind::ConfirmFinish
    )
}
fn transcript_hash(transcript: &[u8]) -> Nonce {
    let mut hash = blake3::Hasher::new();
    hash.update(b"mewtual/voice/handshake/v2\0");
    hash.update(transcript);
    *hash.finalize().as_bytes()
}
fn plaintext(kind: EncryptedKind, handshake: Nonce, low: Nonce, high: Option<Nonce>) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(CONFIRM_DOMAIN).expect("constant");
    e.put_u8(VERSION);
    e.put_u8(kind as u8);
    fixed(&mut e, &handshake);
    fixed(&mut e, &low);
    if let Some(high) = high {
        fixed(&mut e, &high);
    }
    e.finish()
}
fn decode(
    bytes: &[u8],
    kind: EncryptedKind,
    handshake: Nonce,
) -> Result<(Nonce, Option<Nonce>), VoiceError> {
    if bytes.len() > 512 {
        return Err(VoiceError::Bounds);
    }
    let mut d = Decoder::new(bytes);
    if d.get_str()? != CONFIRM_DOMAIN
        || d.get_u8()? != VERSION
        || d.get_u8()? != kind as u8
        || array::<32>(&mut d)? != handshake
    {
        return Err(VoiceError::Malformed);
    }
    let low = array(&mut d)?;
    let high = if kind == EncryptedKind::ConfirmHello {
        None
    } else {
        Some(array(&mut d)?)
    };
    d.finish()?;
    Ok((low, high))
}
/// All fallible counter/ownership checks precede retirement of the previous packet. The caller
/// installs this value synchronously, after its final pair/token checks, without an await.
struct PreparedActionChange {
    action: Action,
    retire: Option<MessageId>,
}
enum PreparedConfirmation {
    Hello {
        action: Option<PreparedActionChange>,
        low: Nonce,
        high: Option<Nonce>,
    },
    Proof {
        action: PreparedActionChange,
        high: Nonce,
    },
    Finish,
    Rejected(VoiceError),
}
fn prepare_action(
    pair: &Pair,
    kind: EncryptedKind,
    low: Nonce,
    high: Option<Nonce>,
    ledger: &PacketLedger,
    authority: &Authority<'_>,
) -> Result<PreparedActionChange, VoiceError> {
    let serial = pair
        .action_serial
        .checked_add(1)
        .ok_or(VoiceError::Capacity)?;
    // Retire the previous packet's independently captured source owner before publishing a
    // replacement nonce/action. The pair and join can remain live across these transitions.
    let retire = pair.action.as_ref().and_then(|action| {
        action
            .draft
            .as_ref()
            .map(|draft| draft.id)
            .or_else(|| action.signed.as_ref().map(SignedObject::id))
    });
    // This checks the ledger's current owner without retiring anything. There is no fallible
    // step after the caller consumes its incoming token and installs this prepared action.
    if retire.is_some() {
        ledger.validate_establish(&pair.binding.handshake, authority)?;
    }
    Ok(PreparedActionChange {
        action: Action {
            serial,
            kind,
            plaintext: plaintext(kind, pair.binding.handshake, low, high),
            draft: None,
            signed: None,
            created: 0,
            live: RevocationOwner::default(),
        },
        retire,
    })
}
fn apply_action(pair: &mut Pair, prepared: PreparedActionChange, ledger: &PacketLedger) {
    if let Some(id) = prepared.retire {
        ledger.retire_outgoing_validated(id);
    }
    pair.action_serial = prepared.action.serial;
    pair.action = Some(prepared.action);
}
impl LocalJoin {
    fn chain_binding(
        &self,
        handshake: Nonce,
        authority: &Authority<'_>,
    ) -> super::super::chain::Binding {
        let pair = &self.confirmations.pairs[&handshake];
        super::super::chain::Binding {
            scope: self.scope,
            epoch: authority.context.epoch,
            local: self.local,
            local_join: self.key.join,
            local_key: self.key.key_ref,
            handshake,
            peer: pair.binding.peer,
            peer_join: pair.binding.join,
            peer_key: pair.binding.key_ref,
            generation: pair.generation,
            call: self.lease.clone(),
            pair: pair.pin.clone(),
            deadline: pair.deadline,
            low: pair.low,
        }
    }

    /// Prepare inactive native confirmation capacity for one proven pair. Repeated live calls
    /// return the same owner; after close this exact pair cannot prepare again. No readiness,
    /// packet, candidate debit, signing, engine permit or browser capacity is granted here.
    /// The actual native role selects three input/six output table claims for the low side,
    /// or six input/three output claims for the high side, within the existing 32-row limits.
    pub fn prepare_crypto_chain(
        &mut self,
        handshake: Nonce,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<Arc<super::super::chain::PreparedChain>, VoiceError> {
        self.pair_current(handshake, authority, clock)?;
        let binding = self.chain_binding(handshake, authority);
        let slots = self.ledger.action_slots();
        if let Some(chain) = &self.confirmations.pairs[&handshake].chain {
            if chain.matches(&binding, &self.resources, &slots, clock.monotonic_ms()) {
                return Ok(chain.clone());
            }
            chain.close();
            return Err(VoiceError::Unauthorized);
        }
        // An ordinary action cannot be adopted into a prepaid chain with a different fence.
        if self.actions.get(Key::Derive(handshake)).is_some() {
            return Err(VoiceError::Unauthorized);
        }
        let chain = super::super::chain::PreparedChain::new(
            self.resources.clone(),
            binding.clone(),
            slots.clone(),
            self.ledger.packet_slots(),
        )?;
        self.confirmations
            .pairs
            .get_mut(&handshake)
            .expect("current pair")
            .chain = Some(chain.clone());
        let current = self.pair_current(handshake, authority, clock);
        if let Err(error) = current {
            chain.close();
            return Err(error);
        }
        if !chain.matches(
            &self.chain_binding(handshake, authority),
            &self.resources,
            &slots,
            clock.monotonic_ms(),
        ) {
            chain.close();
            return Err(VoiceError::Unauthorized);
        }
        Ok(chain)
    }

    /// First consumer of the inactive plan: exact derivation with separately prepaid history
    /// and variable input. All fallible preparation precedes permanent row publication. Native
    /// callers must still reserve their own registry/browser continuation before readiness.
    pub fn chain_derivation_action(
        &mut self,
        handshake: Nonce,
        chain: &Arc<super::super::chain::PreparedChain>,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        self.pair_current(handshake, authority, clock.as_ref())?;
        if self.confirmations.pairs[&handshake]
            .chain
            .as_ref()
            .is_none_or(|original| !Arc::ptr_eq(original, chain))
            || !chain.matches(
                &self.chain_binding(handshake, authority),
                &self.resources,
                &self.ledger.action_slots(),
                clock.monotonic_ms(),
            )
        {
            return Err(VoiceError::Unauthorized);
        }
        let key = Key::Derive(handshake);
        if let Some(action) = self.actions.get(key) {
            return if action.uses_chain(&chain.lease()) {
                Ok(action)
            } else {
                Err(VoiceError::Unauthorized)
            };
        }
        let (payload_bytes, engine_bytes) =
            crypto_actions::derive_bytes(self.confirmations.pairs[&handshake].transcript.len())?;
        let (history, budget) = chain.prepare_derive()?;
        let prepared = self.actions.prepare(
            key,
            Allocation {
                resources: self.resources.clone(),
                scope: self.scope,
                call: self.lease.clone(),
                budget,
                history: Some(history),
                payload_bytes,
                engine_bytes,
            },
        )?;
        let pending = self
            .pair_derivation_authority(handshake, authority, clock.clone())?
            .with_pending(chain.lease())?;
        let setup = self.pair_setup(handshake, authority, clock.as_ref())?;
        // Clock/membership callbacks above may retire the plan. Repeat native checks before
        // the row fence, then perform only prepaid infallible publication under that fence.
        self.pair_current(handshake, authority, clock.as_ref())?;
        if !chain.matches(
            &self.chain_binding(handshake, authority),
            &self.resources,
            &self.ledger.action_slots(),
            clock.monotonic_ms(),
        ) {
            return Err(VoiceError::Unauthorized);
        }
        let action = self
            .actions
            .publish_chain_derive(prepared, setup, pending)?;
        chain.register_action(&action);
        Ok(action)
    }

    /// Publish one continuing native derive identity for the actual proven pair. Repeated pull
    /// or HMR cannot manufacture another engine fence or reset the pair's original deadline.
    pub fn derivation_action(
        &mut self,
        handshake: Nonce,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        budget: ByteBudget,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        self.pair_current(handshake, authority, clock.as_ref())?;
        if self.confirmations.pairs[&handshake].chain.is_some() {
            return Err(VoiceError::Unauthorized);
        }
        let key = Key::Derive(handshake);
        if let Some(action) = self.actions.get(key) {
            return Ok(action);
        }
        let (payload_bytes, engine_bytes) =
            crypto_actions::derive_bytes(self.confirmations.pairs[&handshake].transcript.len())?;
        let prepared = self.actions.prepare(
            key,
            Allocation {
                resources: self.resources.clone(),
                scope: self.scope,
                call: self.lease.clone(),
                budget,
                history: None,
                payload_bytes,
                engine_bytes,
            },
        )?;
        let pending = self.pair_derivation_authority(handshake, authority, clock.clone())?;
        let setup = self.pair_setup(handshake, authority, clock.as_ref())?;
        Ok(self.actions.publish_derive(prepared, setup, pending))
    }

    /// Capture the exact native confirmation action/reservation and owned input before browser
    /// admission. This factory accepts no arbitrary plaintext, signer or header from IPC.
    pub fn confirmation_encryption_action(
        &mut self,
        flight: CryptoFlight,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        budget: ByteBudget,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        let pending = self.confirmation_encryption_authority(&flight, authority, clock)?;
        let key = Key::Encrypt(flight.handshake, flight.action, flight.reservation.id);
        if let Some(action) = self.actions.get(key) {
            return Ok(action);
        }
        let (payload_bytes, engine_bytes) = crypto_actions::confirmation_bytes(
            flight.reservation.aad.len(),
            flight.plaintext.len() + 16,
        )?;
        let prepared = self.actions.prepare(
            key,
            Allocation {
                resources: self.resources.clone(),
                scope: self.scope,
                call: self.lease.clone(),
                budget,
                history: None,
                payload_bytes,
                engine_bytes,
            },
        )?;
        Ok(self.actions.publish_encrypt(prepared, flight, pending))
    }

    /// Only an actual retained native inbound token can issue a decrypt action. Header data
    /// cannot create a pair. The endpoint must reserve this queue record before its eventual
    /// ingress token/origin transaction; this additive wrapper alone does not close that path.
    pub fn confirmation_decryption_action(
        &mut self,
        event: InboundPacket,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        budget: ByteBudget,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        if !crypto_actions::is_confirmation(event.header.kind) {
            return Err(VoiceError::Unauthorized);
        }
        let pending = self.incoming_crypto_authority(&event, authority, clock)?;
        let key = Key::Decrypt(event.token);
        if let Some(action) = self.actions.get(key) {
            return if action.input_tag() == event.object_hash() {
                Ok(action)
            } else {
                Err(VoiceError::RetryMismatch)
            };
        }
        let (payload_bytes, engine_bytes) =
            crypto_actions::confirmation_bytes(event.aad().len(), event.ciphertext().len())?;
        let prepared = self.actions.prepare(
            key,
            Allocation {
                resources: self.resources.clone(),
                scope: self.scope,
                call: self.lease.clone(),
                budget,
                history: None,
                payload_bytes,
                engine_bytes,
            },
        )?;
        Ok(self.actions.publish_decrypt(prepared, event, pending))
    }

    /// Non-dequeueing bounded pull from this exact native join. A slow consumer cannot suspend
    /// the actor; repeated pulls return the same opaque fence and immutable input custody.
    pub fn pull_crypto_actions(
        &mut self,
        authority: &Authority<'_>,
    ) -> Result<Vec<Arc<CryptoAction>>, VoiceError> {
        self.owned(authority)?;
        Ok(self.actions.pull())
    }

    /// Recover an exact accepted seal without signing again. Historical receipts are separate
    /// from permission to send: this compatibility API exposes bytes only while their original
    /// pair/attempt, signed packet and current native authority remain valid.
    pub fn submit_confirmation_action(
        &mut self,
        action: &CryptoAction,
        ciphertext: &[u8],
        signer: &dyn VoiceSigner,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<SignedObject, VoiceError> {
        let result = self.submit_confirmation_result(
            action,
            ciphertext,
            action.output_tag(ciphertext)?,
            signer,
            authority,
            clock,
        )?;
        if result.retired {
            return Err(VoiceError::Unauthorized);
        }
        // Receipt ACK deliberately ends this compatibility sending seam after compaction.
        // History still describes the past transition, but cannot reconstruct fresh send data.
        let input = action.input()?;
        let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
            return Err(VoiceError::Unauthorized);
        };
        self.pair_current(flight.handshake, authority, clock)?;
        let CryptoOutcome::Sealed { message, hash } = result.outcome else {
            return Err(VoiceError::Unauthorized);
        };
        let signed = self.confirmations.pairs[&flight.handshake]
            .action
            .as_ref()
            .and_then(|pending| pending.signed.as_ref())
            .filter(|signed| signed.id() == message && signed.hash() == hash)
            .ok_or(VoiceError::Reservation)?;
        signed.check_freshness(clock.now_ms())?;
        Ok(signed.clone())
    }

    /// Metadata-only exact result recovery. The native adapter must authorize the registered
    /// action owner before calling; the receipt itself never authorizes transport or RTC work.
    pub fn submit_confirmation_result(
        &mut self,
        action: &CryptoAction,
        ciphertext: &[u8],
        claimed_tag: [u8; 32],
        signer: &dyn VoiceSigner,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<CryptoResultReceipt, VoiceError> {
        if !action.belongs_to(self.scope, &self.lease)
            || action.operation() != CryptoOperation::EncryptConfirmation
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(receipt) = action.recover_result(ciphertext, claimed_tag)? {
            return Ok(receipt);
        }
        let input = match action.input() {
            Ok(input) => input,
            // A concurrent terminal publication/ACK can compact after the history miss.
            // Recheck exact history rather than reconstructing input or repeating effects.
            Err(_) => {
                return action
                    .recover_result(ciphertext, claimed_tag)?
                    .ok_or(VoiceError::Unauthorized)
            }
        };
        let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
            return Err(VoiceError::Unauthorized);
        };
        action.with_result(ciphertext, claimed_tag, || {
            let signed = self.submit_confirmation(
                FlightSubmission {
                    handshake: flight.handshake,
                    action: flight.action,
                    reservation: flight.reservation.id,
                    ciphertext,
                },
                signer,
                authority,
                clock,
            )?;
            Ok(TerminalResult {
                binding: self.result_binding(flight.handshake, u64::MAX, signed.expires()),
                outcome: CryptoOutcome::Sealed {
                    message: signed.id(),
                    hash: signed.hash(),
                },
            })
        })
    }

    /// First closed-plaintext transition cannot race a terminal settlement fence. No plaintext
    /// escapes this native confirmation path; SDP/ICE need a separate dispatch/result lease.
    pub fn commit_confirmation_action(
        &mut self,
        action: &CryptoAction,
        plaintext: &[u8],
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), VoiceError> {
        let result = self.commit_confirmation_result(
            action,
            plaintext,
            action.output_tag(plaintext)?,
            authority,
            clock,
            rng,
        )?;
        if result.retired {
            return Err(VoiceError::Unauthorized);
        }
        match result.outcome {
            CryptoOutcome::Opened => Ok(()),
            CryptoOutcome::Rejected(error) => Err(error),
            _ => Err(VoiceError::Unauthorized),
        }
    }

    /// Record both accepted and closed-rejected plaintext results under the same action fence.
    /// A lookup after settlement/token consumption returns history without repeating effects.
    pub fn commit_confirmation_result(
        &mut self,
        action: &CryptoAction,
        plaintext: &[u8],
        claimed_tag: [u8; 32],
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<CryptoResultReceipt, VoiceError> {
        if !action.belongs_to(self.scope, &self.lease)
            || action.operation() != CryptoOperation::DecryptConfirmation
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(receipt) = action.recover_result(plaintext, claimed_tag)? {
            return Ok(receipt);
        }
        let input = match action.input() {
            Ok(input) => input,
            Err(_) => {
                return action
                    .recover_result(plaintext, claimed_tag)?
                    .ok_or(VoiceError::Unauthorized)
            }
        };
        let CryptoInput::DecryptConfirmation(packet) = input.as_input() else {
            return Err(VoiceError::Unauthorized);
        };
        action.with_result(plaintext, claimed_tag, || {
            self.commit_confirmation_terminal(
                OpenedFlight {
                    token: packet.token,
                    aad: packet.aad(),
                    sequence: packet.header.sequence,
                    ciphertext: packet.ciphertext(),
                    plaintext,
                },
                authority,
                clock,
                rng,
            )
        })
    }

    fn result_binding(&self, handshake: Nonce, deadline: u64, expires: u64) -> ResultBinding {
        let pair = &self.confirmations.pairs[&handshake];
        ResultBinding {
            pair: pair.pin.clone(),
            attempt: Some(pair.attempt.0.clone()),
            deadline: pair.deadline.min(deadline),
            expires: expires.saturating_add(CLOCK_SKEW_MS),
        }
    }

    /// Closed readiness marker for the original native-selected derivation. No key bytes are
    /// accepted or returned. As with confirmation history, this is not a fresh pair/RTC grant.
    /// Derivation belongs to the unchanged ECDH pair, independent of nonce-attempt replacement.
    pub fn commit_derivation_result(
        &mut self,
        action: &CryptoAction,
        claimed_tag: [u8; 32],
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<CryptoResultReceipt, VoiceError> {
        if !action.belongs_to(self.scope, &self.lease)
            || action.operation() != CryptoOperation::Derive
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(receipt) = action.recover_result(&[], claimed_tag)? {
            return Ok(receipt);
        }
        let input = match action.input() {
            Ok(input) => input,
            Err(_) => {
                return action
                    .recover_result(&[], claimed_tag)?
                    .ok_or(VoiceError::Unauthorized)
            }
        };
        let CryptoInput::Derive(setup) = input.as_input() else {
            return Err(VoiceError::Unauthorized);
        };
        action.with_result(&[], claimed_tag, || {
            // History above remains recoverable after an epoch change. First effects still
            // require the epoch captured by this plan, even without another prepare/pull.
            action.check_chain_epoch(authority.context.epoch)?;
            self.pair_current(setup.handshake, authority, clock)?;
            let mut binding = self.result_binding(setup.handshake, u64::MAX, u64::MAX);
            binding.attempt = None;
            Ok(TerminalResult {
                binding,
                outcome: CryptoOutcome::Derived,
            })
        })
    }

    /// Derivation belongs to the unchanged native pair, independently of confirmation nonce
    /// restarts. This is pending eligibility, not evidence of a confirmed pair or RTC authority.
    pub fn pair_derivation_authority(
        &mut self,
        handshake: Nonce,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
    ) -> Result<super::super::crypto_authority::CryptoAuthority, VoiceError> {
        self.pair_current(handshake, authority, clock.as_ref())?;
        let pair = &self.confirmations.pairs[&handshake];
        super::super::crypto_authority::CryptoAuthority::new(
            self.lease.clone(),
            pair.pin.clone(),
            clock,
            pair.deadline,
            u64::MAX,
        )
    }
    /// Data in a CryptoFlight cannot authorize a different/current action. Capture both native
    /// action and unsigned-reservation owners before browser admission; each closes separately.
    pub fn confirmation_encryption_authority(
        &mut self,
        flight: &CryptoFlight,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
    ) -> Result<super::super::crypto_authority::CryptoAuthority, VoiceError> {
        self.pair_current(flight.handshake, authority, clock.as_ref())?;
        let pair = &self.confirmations.pairs[&flight.handshake];
        let action = pair.action.as_ref().ok_or(VoiceError::Reservation)?;
        if action.serial != flight.action
            || action.plaintext != flight.plaintext
            || action
                .draft
                .as_ref()
                .is_none_or(|draft| draft.id != flight.reservation.id)
            || action.signed.is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        self.ledger
            .outgoing_crypto_authority(&flight.reservation, true, authority, clock)?
            .with_pending(action.live.0.clone())?
            .limit_deadline(pair.deadline)
    }
    pub(super) fn collect_confirmations(&mut self, clock: &dyn Clock) {
        for (id, pair) in &mut self.confirmations.pairs {
            if clock.monotonic_ms() >= pair.deadline || !self.lease.is_live() || !pair.pin.is_live()
            {
                if let Some(chain) = &pair.chain {
                    chain.close();
                }
            }
            if !pair.retired
                && (!self.lease.is_live()
                    || !pair.pin.is_live()
                    || (!pair.established && clock.monotonic_ms() >= pair.deadline))
            {
                pair.retired = true;
                pair.pin.revoke();
                pair.action = None;
                self.ledger.retire_handshake(id);
            }
        }
    }
    /// Native pair admission is idempotent for an existing exact transcript and claims its
    /// confirmation owner once. All failed/retired records remain bounded through the local join.
    pub fn begin_pair(
        &mut self,
        fresh: &FreshPeer,
        authority: &Authority<'_>,
        limits: &mut OriginLimits,
        clock: &dyn Clock,
    ) -> Result<PairSetup, VoiceError> {
        self.owned(authority)?;
        // Expiry retires authority/pins, never the retained allocator or pair table entry.
        self.collect_confirmations(clock);
        fresh.authorize(authority.context, authority.member, clock)?;
        if limits.group() != self.scope.group {
            return Err(VoiceError::Scope);
        }
        let own = self.announcement.as_ref().ok_or(VoiceError::Reservation)?;
        own.check_freshness(clock.now_ms())?;
        let transcript = pair_transcript(own, fresh.announcement())?;
        let handshake = transcript_hash(&transcript);
        if self.confirmations.pairs.contains_key(&handshake) {
            return self.pair_setup(handshake, authority, clock);
        }
        if self.confirmations.pairs.len() >= MAX_PAIRS
            || self
                .confirmations
                .pairs
                .values()
                .filter(|pair| {
                    !pair.established && !pair.retired && clock.monotonic_ms() < pair.deadline
                })
                .count()
                >= MAX_CONFIRMING
        {
            return Err(VoiceError::Capacity);
        }
        let Body::Announcement { key, .. } = fresh.announcement().body() else {
            return Err(VoiceError::Malformed);
        };
        let binding = PairBinding {
            peer: fresh.announcement().issuer(),
            join: key.join,
            key_ref: key.key_ref,
            handshake,
        };
        let generation = self
            .confirmations
            .serial
            .checked_add(1)
            .ok_or(VoiceError::Capacity)?;
        let credit = self
            .resources
            .charge(self.scope.group, PAIR_BYTES, ResourceKind::Retained)?;
        let pin = CallLease::default();
        limits.protect_pair(
            binding.peer,
            binding.join,
            binding.key_ref,
            &pin,
            clock.monotonic_ms(),
        )?;
        if let Err(error) = self
            .ledger
            .claim_confirmation(binding.clone(), pin.clone(), authority)
        {
            pin.revoke();
            return Err(error);
        }
        let low = self.local < binding.peer;
        self.confirmations.pairs.insert(
            handshake,
            Pair {
                binding,
                generation,
                transcript,
                peer_key: key.public_key,
                pin,
                deadline: clock.monotonic_ms().saturating_add(MAX_PACKET_LIFETIME_MS),
                attempt: RevocationOwner::default(),
                low,
                starts: 0,
                action_serial: 0,
                low_nonce: None,
                high_nonce: None,
                hello: super::super::crypto_authority::SequenceGate::default(),
                accepted_hello: None,
                accepted_proof: None,
                accepted_finish: None,
                action: None,
                established: false,
                retired: false,
                chain: None,
                _credit: credit,
            },
        );
        self.confirmations.serial = generation;
        self.pair_setup(handshake, authority, clock)
    }
    pub(super) fn pair_current(
        &mut self,
        handshake: Nonce,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<(), VoiceError> {
        self.owned(authority)?;
        let pair = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .ok_or(VoiceError::Unauthorized)?;
        if !(authority.member)(pair.binding.peer) {
            pair.pin.revoke();
            pair.retired = true;
            // This check also runs inside an action's first-result Engine fence. Revoke only
            // the pin here; quiet collection closes unused chain capacity outside that fence.
            self.ledger.retire_handshake(&handshake);
        }
        if pair.retired || !pair.pin.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        if !pair.established && clock.monotonic_ms() >= pair.deadline {
            pair.retired = true;
            pair.pin.revoke();
            self.ledger.retire_handshake(&handshake);
            return Err(VoiceError::Freshness);
        }
        Ok(())
    }
    pub fn pair_setup(
        &mut self,
        handshake: Nonce,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<PairSetup, VoiceError> {
        self.pair_current(handshake, authority, clock)?;
        let pair = &self.confirmations.pairs[&handshake];
        Ok(PairSetup {
            handshake,
            transcript: pair.transcript.clone(),
            peer: pair.binding.peer,
            peer_key: pair.peer_key,
            local_is_low: pair.low,
        })
    }
    /// Fresh retry changes the confirmation nonce/action, never the pair's packet allocator.
    /// Retried starts stay within the original 15s window and a maximum of three native starts.
    pub fn start_confirmation(
        &mut self,
        handshake: Nonce,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), VoiceError> {
        self.pair_current(handshake, authority, clock)?;
        let pair = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("checked pair");
        if !pair.low || pair.established {
            return Err(VoiceError::Unauthorized);
        }
        if pair.starts >= 3 {
            return Err(VoiceError::RateLimited);
        }
        let mut nonce = [0; 32];
        rng.fill_bytes(&mut nonce);
        let prepared = prepare_action(
            pair,
            EncryptedKind::ConfirmHello,
            nonce,
            None,
            &self.ledger,
            authority,
        )?;
        // A fresh nonce supersedes pending old confirmation decrypts, but not the unchanged
        // ECDH pair. Ordinary Hello -> Proof -> Finish progression retains this attempt owner.
        pair.attempt = RevocationOwner::default();
        apply_action(pair, prepared, &self.ledger);
        pair.starts += 1;
        pair.low_nonce = Some(nonce);
        pair.high_nonce = None;
        pair.accepted_proof = None;
        Ok(())
    }
    /// Reservation pressure leaves the validated native action/nonces intact. Exact duplicate
    /// input can retry this method without another decrypt, fresh nonce or confirmation start.
    pub fn prepare_confirmation(
        &mut self,
        handshake: Nonce,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<ConfirmationOutput>, VoiceError> {
        self.pair_current(handshake, authority, clock)?;
        let pair = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("checked pair");
        let Some(action) = &mut pair.action else {
            return Ok(None);
        };
        if let Some(signed) = &action.signed {
            if clock.monotonic_ms()
                >= action
                    .created
                    .saturating_add(MAX_PACKET_LIFETIME_MS + CLOCK_SKEW_MS)
            {
                return Err(VoiceError::Reservation);
            }
            signed.check_freshness(clock.now_ms())?;
            return Ok(Some(ConfirmationOutput::Send(Box::new(signed.clone()))));
        }
        if action.draft.is_some() && clock.monotonic_ms() >= action.created.saturating_add(5000) {
            // The first prepare reply may have been lost, so recovery cannot require its ids.
            // Burn the old action/reservation before any new allocation. Old packet/engine
            // credits remain with their actual owners; this never refunds uncertain crypto.
            let serial = pair
                .action_serial
                .checked_add(1)
                .ok_or(VoiceError::Capacity)?;
            action.live = RevocationOwner::default();
            action.serial = serial;
            pair.action_serial = action.serial;
            self.ledger
                .retire_outgoing(action.draft.as_ref().expect("draft checked").id, authority)?;
            action.draft = None;
        }
        if action.draft.is_none() {
            let public_key = *self
                .announcement
                .as_ref()
                .ok_or(VoiceError::Reservation)?
                .public_key();
            action.draft = Some(self.ledger.reserve(
                public_key,
                ReserveRequest {
                    handshake,
                    kind: action.kind,
                    offer: None,
                    ciphertext_bytes: action.plaintext.len() + 16,
                },
                authority,
                clock,
                rng,
            )?);
            action.created = clock.monotonic_ms();
        }
        Ok(Some(ConfirmationOutput::Encrypt(CryptoFlight {
            handshake,
            action: action.serial,
            reservation: action.draft.as_ref().expect("reserved draft").clone(),
            plaintext: action.plaintext.clone(),
        })))
    }
    /// Engine failure/cancel burns this exact reservation. Chosen confirmation nonces remain;
    /// a later reservation uses a continuing sequence. A late result cannot sign under it.
    pub fn abandon_confirmation(
        &mut self,
        handshake: Nonce,
        action: u64,
        id: MessageId,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<(), VoiceError> {
        self.pair_current(handshake, authority, clock)?;
        let pair = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("checked pair");
        let pending = pair.action.as_mut().ok_or(VoiceError::Reservation)?;
        if pending.serial != action
            || pending.draft.as_ref().is_none_or(|draft| draft.id != id)
            || pending.signed.is_some()
        {
            return Err(VoiceError::Reservation);
        }
        let serial = pair
            .action_serial
            .checked_add(1)
            .ok_or(VoiceError::Capacity)?;
        pending.live = RevocationOwner::default();
        pending.serial = serial;
        pair.action_serial = pending.serial;
        self.ledger.retire_outgoing(id, authority)?;
        pending.draft = None;
        Ok(())
    }
    /// Atomically gate exact current action/reservation, pin ciphertext/signature, then establish
    /// the initiator ONLY for this Finish. Delayed crypto from a superseded attempt cannot sign.
    pub fn submit_confirmation(
        &mut self,
        submission: FlightSubmission<'_>,
        signer: &dyn VoiceSigner,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<SignedObject, VoiceError> {
        let FlightSubmission {
            handshake,
            action,
            reservation: id,
            ciphertext,
        } = submission;
        self.current(signer, authority)?;
        self.pair_current(handshake, authority, clock)?;
        let pending = self.confirmations.pairs[&handshake]
            .action
            .as_ref()
            .ok_or(VoiceError::Reservation)?;
        if pending.serial != action || pending.draft.as_ref().is_none_or(|draft| draft.id != id) {
            return Err(VoiceError::Reservation);
        }
        let kind = pending.kind;
        // Promotion eligibility is checked BEFORE signing, then repeated by the synchronous
        // promotion below. An older candidate cannot publish Finish after a newer pair won.
        if kind == EncryptedKind::ConfirmFinish {
            self.promotion_current(handshake)?;
            self.ledger.validate_establish(&handshake, authority)?;
        }
        let signed = self
            .ledger
            .submit(id, ciphertext, signer, authority, clock)?;
        self.confirmations
            .pairs
            .get_mut(&handshake)
            .expect("checked pair")
            .action
            .as_mut()
            .expect("checked action")
            .signed = Some(signed.clone());
        if kind == EncryptedKind::ConfirmFinish {
            self.apply_promotion(handshake);
        }
        Ok(signed)
    }
    /// A signed duplicate is recognized only against the current validated flight/attempt.
    /// Cached response lookup precedes replay rejection but never authorizes another decrypt.
    pub fn receive_confirmation(
        &mut self,
        object: SignedObject,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<ReceiveProgress, VoiceError> {
        self.owned(authority)?;
        object.check_freshness(clock.now_ms())?;
        let Body::Encrypted { header, .. } = object.body() else {
            return Err(VoiceError::Malformed);
        };
        if object.scope() != self.scope || !confirmation(header.kind) {
            return Err(VoiceError::Scope);
        }
        self.pair_current(header.handshake, authority, clock)?;
        let pair = &self.confirmations.pairs[&header.handshake];
        if object.issuer() != pair.binding.peer
            || header.target != self.local
            || header.source_join != pair.binding.join
            || header.target_join != self.key.join
            || header.source_key_ref != pair.binding.key_ref
            || header.target_key_ref != self.key.key_ref
        {
            return Err(VoiceError::Unauthorized);
        }
        let hash = object.hash();
        if pair.accepted_hello == Some(hash) || pair.accepted_proof == Some(hash) {
            return Ok(ReceiveProgress::Retry(header.handshake));
        }
        if pair.accepted_finish == Some(hash) && pair.established {
            return Ok(ReceiveProgress::Confirmed);
        }
        let pending = if header.kind == EncryptedKind::ConfirmHello {
            // Hello proposes a new attempt. Binding it to the previous attempt would strand an
            // already queued newer Hello when an earlier candidate finishes decrypting first.
            super::super::packets::PendingConfirmation::Hello(
                pair.hello.candidate(header.sequence)?,
            )
        } else {
            super::super::packets::PendingConfirmation::Attempt(pair.attempt.0.clone())
        };
        let deadline = pair.deadline;
        let prepared = self
            .ledger
            .prepare_inbound(object, authority, clock)?
            .bind_confirmation(pending, deadline)?;
        // Binding and its original deadline are already in the prepared token. Final checks
        // run before replay/serial publication; there is no temporarily unbound incoming row.
        let event =
            self.ledger
                .commit_prepared_inbound(prepared, authority, clock, |admission| {
                    admission.check(clock)
                })?;
        Ok(ReceiveProgress::Decrypt(Box::new(event)))
    }
    /// Exact native tuple commit and closed confirmation transition form one actor turn. No bare
    /// caller-supplied header drives state. Malformed plaintext consumes only its own inbound
    /// token and creates no response-cache/progression entry. Valid progress survives later
    /// response-reservation Capacity or engine failure in prepare/abandon, without re-decrypt.
    pub fn commit_confirmation(
        &mut self,
        opened: OpenedFlight<'_>,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), VoiceError> {
        match self
            .commit_confirmation_terminal(opened, authority, clock, rng)?
            .outcome
        {
            CryptoOutcome::Opened => Ok(()),
            CryptoOutcome::Rejected(error) => Err(error),
            _ => unreachable!("closed confirmation outcome"),
        }
    }
    fn commit_confirmation_terminal(
        &mut self,
        opened: OpenedFlight<'_>,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<TerminalResult, VoiceError> {
        let OpenedFlight {
            token,
            aad,
            sequence,
            ciphertext,
            plaintext,
        } = opened;
        if plaintext.len() > 512 || plaintext.len().checked_add(16) != Some(ciphertext.len()) {
            return Err(VoiceError::Bounds);
        }
        let prepared = self
            .ledger
            .validate_inbound_commit(token, aad, sequence, ciphertext, authority, clock)?;
        let handshake = prepared.header.handshake;
        self.pair_current(handshake, authority, clock)?;
        let change = self.prepare_confirmation_change(&prepared, plaintext, authority, rng);
        // Capacity is a proven local refusal BEFORE consumption or nonce/attempt changes.
        // Closed malformed/unauthorized plaintext, however, consumes its exact token and is a
        // retained terminal rejection, so losing that reply cannot create another transition.
        let change = match change {
            Ok(change) => change,
            Err(VoiceError::Capacity) => return Err(VoiceError::Capacity),
            Err(error) => PreparedConfirmation::Rejected(error),
        };
        self.pair_current(handshake, authority, clock)?;
        self.ledger
            .validate_inbound_owner(&prepared, authority, clock)?;
        if matches!(change, PreparedConfirmation::Finish) {
            self.promotion_current(handshake)?;
            self.ledger.validate_establish(&handshake, authority)?;
        }
        let deadline = prepared.retire;
        let expires = prepared.expires;
        let hash = prepared.hash;
        let sequence = prepared.header.sequence;
        let _receipt = self.ledger.consume_validated_inbound(prepared);
        // No fallible work follows this point. The action's result fence encloses consumption,
        // state application and receipt installation; neither settlement nor a second commit
        // can observe the intermediate state.
        let outcome = match change {
            PreparedConfirmation::Rejected(error) => CryptoOutcome::Rejected(error),
            PreparedConfirmation::Hello { action, low, high } => {
                let pair = self
                    .confirmations
                    .pairs
                    .get_mut(&handshake)
                    .expect("checked pair");
                if let Some(action) = action {
                    pair.attempt = RevocationOwner::default();
                    apply_action(pair, action, &self.ledger);
                    pair.low_nonce = Some(low);
                    pair.high_nonce = high;
                    pair.starts += 1; // prepared after the strict starts < 3 check
                    pair.accepted_finish = None;
                }
                pair.hello.accept(sequence);
                pair.accepted_hello = Some(hash);
                CryptoOutcome::Opened
            }
            PreparedConfirmation::Proof { action, high } => {
                let pair = self
                    .confirmations
                    .pairs
                    .get_mut(&handshake)
                    .expect("checked pair");
                apply_action(pair, action, &self.ledger);
                pair.high_nonce = Some(high);
                pair.accepted_proof = Some(hash);
                CryptoOutcome::Opened
            }
            PreparedConfirmation::Finish => {
                self.apply_promotion(handshake);
                self.confirmations
                    .pairs
                    .get_mut(&handshake)
                    .expect("checked pair")
                    .accepted_finish = Some(hash);
                CryptoOutcome::Opened
            }
        };
        Ok(TerminalResult {
            binding: self.result_binding(handshake, deadline, expires),
            outcome,
        })
    }
    fn prepare_confirmation_change(
        &self,
        prepared: &ValidatedInbound,
        bytes: &[u8],
        authority: &Authority<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<PreparedConfirmation, VoiceError> {
        let header = &prepared.header;
        let handshake = header.handshake;
        if !confirmation(header.kind) {
            return Err(VoiceError::Malformed);
        }
        let (low, high) = decode(bytes, header.kind, handshake)?;
        let pair = &self.confirmations.pairs[&handshake];
        match header.kind {
            EncryptedKind::ConfirmHello if !pair.low && !pair.established => {
                pair.hello.candidate(header.sequence)?;
                if pair.low_nonce != Some(low) {
                    if pair.starts >= 3 {
                        return Err(VoiceError::RateLimited);
                    }
                    let mut nonce = [0; 32];
                    rng.fill_bytes(&mut nonce);
                    let action = prepare_action(
                        pair,
                        EncryptedKind::ConfirmProof,
                        low,
                        Some(nonce),
                        &self.ledger,
                        authority,
                    )?;
                    return Ok(PreparedConfirmation::Hello {
                        action: Some(action),
                        low,
                        high: Some(nonce),
                    });
                }
                Ok(PreparedConfirmation::Hello {
                    action: None,
                    low,
                    high: pair.high_nonce,
                })
            }
            EncryptedKind::ConfirmProof if pair.low && !pair.established => {
                if pair.low_nonce != Some(low)
                    || pair.action.as_ref().is_none_or(|action| {
                        action.kind != EncryptedKind::ConfirmHello || action.signed.is_none()
                    })
                {
                    return Err(VoiceError::Unauthorized);
                }
                let high = high.ok_or(VoiceError::Malformed)?;
                let action = prepare_action(
                    pair,
                    EncryptedKind::ConfirmFinish,
                    low,
                    Some(high),
                    &self.ledger,
                    authority,
                )?;
                Ok(PreparedConfirmation::Proof { action, high })
            }
            EncryptedKind::ConfirmFinish if !pair.low && !pair.established => {
                if pair.low_nonce != Some(low)
                    || pair.high_nonce != high
                    || pair.action.as_ref().is_none_or(|action| {
                        action.kind != EncryptedKind::ConfirmProof || action.signed.is_none()
                    })
                {
                    return Err(VoiceError::Unauthorized);
                }
                self.promotion_current(handshake)?;
                self.ledger.validate_establish(&handshake, authority)?;
                Ok(PreparedConfirmation::Finish)
            }
            _ => Err(VoiceError::Unauthorized),
        }
    }
    fn promotion_current(&self, handshake: Nonce) -> Result<(), VoiceError> {
        let pair = &self.confirmations.pairs[&handshake];
        if self.confirmations.pairs.values().any(|other| {
            other.binding.peer == pair.binding.peer
                && other.established
                && !other.retired
                && other.generation > pair.generation
        }) {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    fn apply_promotion(&mut self, handshake: Nonce) {
        self.ledger.establish_validated(&handshake);
        let peer = self.confirmations.pairs[&handshake].binding.peer;
        let generation = self.confirmations.pairs[&handshake].generation;
        for (id, pair) in &mut self.confirmations.pairs {
            if *id != handshake
                && pair.binding.peer == peer
                && pair.generation < generation
                && !pair.retired
            {
                pair.pin.revoke();
                pair.retired = true;
                self.ledger.retire_handshake(id);
            }
        }
        self.confirmations
            .pairs
            .get_mut(&handshake)
            .expect("checked pair")
            .established = true;
    }
    pub fn pair_established(
        &mut self,
        handshake: Nonce,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<bool, VoiceError> {
        self.pair_current(handshake, authority, clock)?;
        Ok(self.confirmations.pairs[&handshake].established)
    }
}

#[cfg(test)]
mod tests;
