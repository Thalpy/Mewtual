//! Native three-flight key confirmation. This owner holds the exact pending action, reservation
//! and accepted signed-flight hash: async browser work cannot sign/establish a superseded attempt.
//! Plaintext reaches this machine only through an affine, revalidated PacketLedger receipt.
//! Discovery evidence is required once at pair admission. Its later refresh/expiry is deliberately
//! independent of the pair pin; a failed newer join must not evict a working confirmed peer.

use super::*;
use crate::voice_v2::crypto_actions::{self, Allocation, CryptoAction, CryptoInput, Key};
use crate::voice_v2::packets::{
    CommittedPacket, InboundPacket, PairBinding, ReserveRequest, ReservedPacket,
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
    }
}
impl Confirmations {
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
fn set_action(
    pair: &mut Pair,
    kind: EncryptedKind,
    low: Nonce,
    high: Option<Nonce>,
    ledger: &PacketLedger,
    authority: &Authority<'_>,
) -> Result<(), VoiceError> {
    let serial = pair
        .action_serial
        .checked_add(1)
        .ok_or(VoiceError::Capacity)?;
    // Retire the previous packet's independently captured source owner before publishing a
    // replacement nonce/action. The pair and join can remain live across these transitions.
    if let Some(action) = &pair.action {
        if let Some(id) = action
            .draft
            .as_ref()
            .map(|draft| draft.id)
            .or_else(|| action.signed.as_ref().map(SignedObject::id))
        {
            ledger.retire_outgoing(id, authority)?;
        }
    }
    pair.action = Some(Action {
        serial,
        kind,
        plaintext: plaintext(kind, pair.binding.handshake, low, high),
        draft: None,
        signed: None,
        created: 0,
        live: RevocationOwner::default(),
    });
    pair.action_serial = serial;
    Ok(())
}

impl LocalJoin {
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

    /// First ciphertext submission uses the same fence as native admission/settlement. An
    /// endpoint result owner still needs to cache its exact committed reply before returning it;
    /// this wrapper deliberately provides no lost-reply recovery or public IPC by itself.
    pub fn submit_confirmation_action(
        &mut self,
        action: &CryptoAction,
        ciphertext: &[u8],
        signer: &dyn VoiceSigner,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<SignedObject, VoiceError> {
        if !action.belongs_to(self.scope, &self.lease) {
            return Err(VoiceError::Unauthorized);
        }
        let CryptoInput::EncryptConfirmation(flight) = action.input() else {
            return Err(VoiceError::Unauthorized);
        };
        action.with_pending(|| {
            self.submit_confirmation(
                FlightSubmission {
                    handshake: flight.handshake,
                    action: flight.action,
                    reservation: flight.reservation.id,
                    ciphertext,
                },
                signer,
                authority,
                clock,
            )
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
        if !action.belongs_to(self.scope, &self.lease) {
            return Err(VoiceError::Unauthorized);
        }
        let CryptoInput::DecryptConfirmation(packet) = action.input() else {
            return Err(VoiceError::Unauthorized);
        };
        action.with_pending(|| {
            self.commit_confirmation(
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
        // A fresh nonce supersedes pending old confirmation decrypts, but not the unchanged
        // ECDH pair. Ordinary Hello -> Proof -> Finish progression retains this attempt owner.
        pair.attempt = RevocationOwner::default();
        set_action(
            pair,
            EncryptedKind::ConfirmHello,
            nonce,
            None,
            &self.ledger,
            authority,
        )?;
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
            self.promote(handshake, authority, clock)?;
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
        let event = self.ledger.admit_inbound(object, authority, clock)?;
        self.ledger.bind_incoming_confirmation(event.token, pending);
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
        let receipt = self
            .ledger
            .commit_inbound(token, aad, sequence, ciphertext, authority, clock)?;
        self.accept_confirmation(receipt, plaintext, authority, clock, rng)
    }
    fn accept_confirmation(
        &mut self,
        receipt: CommittedPacket,
        bytes: &[u8],
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), VoiceError> {
        self.ledger.receipt_current(&receipt, authority, clock)?;
        let header = receipt.header();
        let handshake = header.handshake;
        self.pair_current(handshake, authority, clock)?;
        if !confirmation(header.kind) {
            return Err(VoiceError::Malformed);
        }
        let (low, high) = decode(bytes, header.kind, handshake)?;
        let pair = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("checked pair");
        match header.kind {
            EncryptedKind::ConfirmHello if !pair.low && !pair.established => {
                pair.hello.candidate(header.sequence)?;
                if pair.low_nonce != Some(low) {
                    if pair.starts >= 3 {
                        return Err(VoiceError::RateLimited);
                    }
                    let mut nonce = [0; 32];
                    rng.fill_bytes(&mut nonce);
                    pair.attempt = RevocationOwner::default();
                    set_action(
                        pair,
                        EncryptedKind::ConfirmProof,
                        low,
                        Some(nonce),
                        &self.ledger,
                        authority,
                    )?;
                    pair.low_nonce = Some(low);
                    pair.high_nonce = Some(nonce);
                    pair.starts += 1;
                    pair.accepted_finish = None;
                }
                pair.hello.accept(header.sequence);
                pair.accepted_hello = Some(receipt.hash());
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
                set_action(
                    pair,
                    EncryptedKind::ConfirmFinish,
                    low,
                    Some(high),
                    &self.ledger,
                    authority,
                )?;
                pair.high_nonce = Some(high);
                pair.accepted_proof = Some(receipt.hash());
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
                self.promote(handshake, authority, clock)?;
                self.confirmations
                    .pairs
                    .get_mut(&handshake)
                    .expect("checked pair")
                    .accepted_finish = Some(receipt.hash());
            }
            _ => return Err(VoiceError::Unauthorized),
        }
        Ok(())
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
    fn promote(
        &mut self,
        handshake: Nonce,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<(), VoiceError> {
        self.pair_current(handshake, authority, clock)?;
        self.promotion_current(handshake)?;
        self.ledger.establish(&handshake, authority)?;
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
        Ok(())
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
