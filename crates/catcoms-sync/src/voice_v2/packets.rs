//! Native nonce/reservation/replay ownership for one ephemeral call. The actor supplies current
//! membership, directory/policy and exact lease at EVERY operation. Fresh presence, confirmation
//! nonces and OfferId retirement belong to the endpoint state machine above this ledger; creating
//! a pair record here does not prove any of them. No public IPC may expose register/establish.

use super::crypto_actions::slots;
use super::negotiation::{Binding as ExchangeBinding, Exchange, Stage};
use super::resources::{Credit, ResourceKind, Resources};
use super::*;
use catcoms_rt::{Clock, CryptoRngCore, RequestAuthority};
use futures::FutureExt;
use std::collections::BTreeMap;
use std::sync::Arc;

mod answer;
mod first_offer;
mod initial_offer;
#[cfg(test)]
pub(in crate::voice_v2) use first_offer::admission_metadata_bytes;
mod outgoing;
mod sealed;
pub(in crate::voice_v2) use sealed::PreparedSealedPublication;

const PAIRS: usize = 16;
const RESERVATIONS: usize = 32;
const CONSTRUCTION_MS: u64 = 5000;

/// The two native packet tables have independent limits. Capacity reservations and actual
/// rows draw from the same pools, so ordinary traffic cannot occupy a promised continuation.
pub(super) struct PacketSlots {
    incoming: Arc<slots::Slots>,
    outgoing: Arc<slots::Slots>,
}
/// Capacity only, owned by the confirmation workflow rather than permanent action history.
/// Each of three candidate lanes needs two Hello/Finish rows and one Proof row. Direction is
/// selected from the actual native pair role, never from a received descriptor. This does not
/// authorize any of those roles; their protected intake/publication consumers are separate.
pub(super) struct ConfirmationCapacity {
    incoming: Arc<slots::ReservedSlots>,
    outgoing: Arc<slots::ReservedSlots>,
}
/// One native Offer/Answer row per direction, carved from the original confirmation
/// parents after obsolete rows return. No fresh global claim is acquired or refunded.
pub(super) struct SdpPacketCapacity {
    incoming: Arc<slots::ReservedSlots>,
    outgoing: Arc<slots::ReservedSlots>,
}
impl SdpPacketCapacity {
    pub(super) fn incoming_slot(&self, slots: &PacketSlots) -> Result<slots::Slot, VoiceError> {
        self.incoming.loan(&slots.incoming)
    }
    pub(super) fn outgoing_slot(&self, slots: &PacketSlots) -> Result<slots::Slot, VoiceError> {
        self.outgoing.loan(&slots.outgoing)
    }
    pub(super) fn close(&self) {
        self.incoming.close();
        self.outgoing.close();
    }
}
impl Drop for SdpPacketCapacity {
    fn drop(&mut self) {
        self.close();
    }
}
/// Slot custody is independent of payload custody. Keep the exact successor LAST even
/// when an event/byte credit has already dropped, so parent drainage cannot precede Slot.
pub(super) struct PacketSlot {
    _slot: slots::Slot,
    _exchange: Option<Arc<Exchange>>,
}
impl From<slots::Slot> for PacketSlot {
    fn from(slot: slots::Slot) -> Self {
        Self {
            _slot: slot,
            _exchange: None,
        }
    }
}
impl PacketSlot {
    pub(in crate::voice_v2) fn uses_incoming(&self, tables: &PacketSlots) -> bool {
        self._slot.uses_pool(&tables.incoming)
    }
    pub(super) fn successor(slot: slots::Slot, exchange: Arc<Exchange>) -> Self {
        Self {
            _slot: slot,
            _exchange: Some(exchange),
        }
    }
}
impl PacketSlots {
    #[cfg(test)]
    pub(in crate::voice_v2) fn fill_unreserved_for_test(
        &self,
    ) -> (Vec<slots::Slot>, Vec<slots::Slot>) {
        (
            (0..RESERVATIONS)
                .map_while(|_| self.incoming.charge(1).ok())
                .collect(),
            (0..RESERVATIONS)
                .map_while(|_| self.outgoing.charge(1).ok())
                .collect(),
        )
    }
    pub(super) fn new() -> Self {
        Self {
            incoming: Arc::new(slots::Slots::with_limit(RESERVATIONS)),
            outgoing: Arc::new(slots::Slots::with_limit(RESERVATIONS)),
        }
    }
    pub(super) fn reserve_confirmation(
        &self,
        local_is_low: bool,
    ) -> Result<ConfirmationCapacity, VoiceError> {
        // If the second reservation refuses, dropping the first returns its unused claims.
        // Neither table, sequence nor replay window has been published at this point.
        Ok(ConfirmationCapacity {
            incoming: self.incoming.reserve(if local_is_low { 3 } else { 6 })?,
            outgoing: self.outgoing.reserve(if local_is_low { 6 } else { 3 })?,
        })
    }
}
impl ConfirmationCapacity {
    pub(super) fn subreserve_sdp(&self) -> Result<SdpPacketCapacity, VoiceError> {
        let incoming = self.incoming.subreserve(1)?;
        let outgoing = self.outgoing.subreserve(1)?;
        Ok(SdpPacketCapacity { incoming, outgoing })
    }
    pub(super) fn outgoing_slot(&self, slots: &PacketSlots) -> Result<slots::Slot, VoiceError> {
        self.outgoing.loan(&slots.outgoing)
    }
    pub(super) fn confirmation_incoming_slot(
        &self,
        slots: &PacketSlots,
    ) -> Result<slots::Slot, VoiceError> {
        self.incoming.loan(&slots.incoming)
    }
    pub(super) fn close(&self) {
        self.incoming.close();
        self.outgoing.close();
    }
}
impl Drop for ConfirmationCapacity {
    fn drop(&mut self) {
        // This wrapper owns closing rights, not just Arc handles. Abandonment/unwind must
        // return unused claims even when a delayed actual Slot retains the parent allocation.
        self.close();
    }
}

/// A current actor-turn view, not a capability that can be captured over an await.
pub struct Authority<'a> {
    pub context: &'a Context,
    pub lease: &'a CallLease,
    pub member: &'a dyn Fn(DeviceId) -> bool,
}
#[derive(Clone, PartialEq, Eq)]
pub struct PairBinding {
    pub peer: DeviceId,
    pub join: Nonce,
    pub key_ref: [u8; 32],
    pub handshake: [u8; 32],
}

/// Unpublished fixed pair state, constructed outside native Group/Policy. The sole LocalJoin
/// keeps the packet table unchanged until its final synchronous publication. This value owns
/// the exact revocation handle, not another equivalent join or reconstructed allocator.
pub(super) struct PreparedConfirmationClaim(Pair);
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Confirming,
    Established,
    Retired,
}
#[derive(Clone, Default)]
struct Replay {
    highest: u64,
    seen: u128,
}
impl Replay {
    fn admit(&mut self, sequence: u64) -> Result<(), VoiceError> {
        if sequence == 0 {
            return Err(VoiceError::Replay);
        }
        if sequence > self.highest {
            let advance = sequence - self.highest;
            self.seen = if advance >= 128 {
                1
            } else {
                (self.seen << advance) | 1
            };
            self.highest = sequence;
            return Ok(());
        }
        let distance = self.highest - sequence;
        if distance >= 128 || self.seen & (1u128 << distance) != 0 {
            return Err(VoiceError::Replay);
        }
        self.seen |= 1u128 << distance;
        Ok(())
    }
}
struct Pair {
    binding: PairBinding,
    status: Status,
    live: CallLease,
    confirmation_claimed: bool,
    // Irreversible before protected capacity admission. Never clear this when a round
    // retires: ordinary old SDP/ICE/Bye must not become fallback writers afterward.
    sdp_profile: Option<super::SdpProfile>,
    next: u64,
    replay: Replay,
}
impl Pair {
    fn check_profile(&self, kind: EncryptedKind) -> Result<(), VoiceError> {
        let confirmation = matches!(
            kind,
            EncryptedKind::ConfirmHello
                | EncryptedKind::ConfirmProof
                | EncryptedKind::ConfirmFinish
        );
        if !confirmation && self.sdp_profile != kind.round().map(|round| round.profile) {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
}
struct Outgoing {
    draft: PacketDraft,
    ciphertext_bytes: usize,
    construction: u64,
    retire: u64,
    source_deadline: u64,
    sealed: sealed::SealedState,
    live: CallLease,
    crypto: RevocationOwner,
    // Protected output remains tied to its original chain after encryption has settled.
    // This is a revocation identity only, never Workflow progress custody.
    chain: Option<CallLease>,
    // Exact protected producer, already retained by this packet's byte credit. Source must
    // borrow this SAME capacity; a live chain lease alone carries no prepaid progress bytes.
    workflow: Option<Arc<super::chain::Workflow>>,
    _credit: Credit,
    // Native table custody, separate from retained bytes and actual transport copies.
    _slot: PacketSlot,
    exchange: Option<Arc<Exchange>>,
    // Fixed PC/Ready/Offer provenance; a live Exchange does not imply a live PC.
    sdp_binding: Option<Arc<super::sdp::action::ActionBinding>>,
}
#[derive(Clone)]
pub(super) enum PendingConfirmation {
    Attempt(CallLease),
    Hello(super::crypto_authority::SequenceAuthority),
    // Opening/classification only: the authenticated header cannot identify its echoed nonce.
    // Closed plaintext must validate the actual current attempt before any progression.
    ProtectedInbox(CallLease),
}
impl PendingConfirmation {
    fn authorized(&self) -> bool {
        match self {
            Self::Attempt(lease) | Self::ProtectedInbox(lease) => lease.is_live(),
            Self::Hello(gate) => gate.authorized(),
        }
    }
}
struct Incoming {
    object: SignedObject,
    retire: u64,
    token: RevocationOwner,
    confirmation: Option<PendingConfirmation>,
    _credit: Arc<Credit>,
    _slot: PacketSlot,
    exchange: Option<Arc<Exchange>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SdpRole {
    Ready,
    Initiator(OfferId),
    Responder(OfferId),
}
// Role and identities are owned by the sole ledger actor, independently of the shared capacity
// mutex. No Group/Policy commit acquires the capacity mutex while a second thread holds it.
struct SdpExchange {
    owner: Arc<Exchange>,
    handshake: [u8; 32],
    role: SdpRole,
    local: Option<MessageId>,
    remote: Option<MessageId>,
    ready_until: u64,
    retire: Option<u64>,
}

pub struct PacketLedger {
    scope: Scope,
    local: DeviceId,
    join: Nonce,
    key_ref: [u8; 32],
    lease: CallLease,
    resources: Arc<Resources>,
    pairs: BTreeMap<[u8; 32], Pair>,
    outgoing: BTreeMap<MessageId, Outgoing>,
    incoming: BTreeMap<u64, Incoming>,
    serial: u64,
    sdp: Option<SdpExchange>,
    action_slots: Arc<super::crypto_actions::slots::Slots>,
    packet_slots: PacketSlots,
}
pub struct ReserveRequest {
    pub handshake: [u8; 32],
    pub kind: EncryptedKind,
    pub offer: Option<OfferId>,
    pub ciphertext_bytes: usize,
}
#[derive(Clone)]
pub struct ReservedPacket {
    pub id: MessageId,
    pub aad: Vec<u8>,
    pub sequence: u64,
    pub expires: u64,
}
/// Decrypt payload and actual byte custody stay together. Native adapters borrow its bytes;
/// exporting owned payloads requires a separately charged owner, not extracting these Vecs.
///
/// ```compile_fail
/// use catcoms_sync::voice_v2::packets::InboundPacket;
/// fn detach_payload(packet: InboundPacket) -> Vec<u8> { packet.ciphertext }
/// ```
pub struct InboundPacket {
    pub token: u64,
    pub issuer: DeviceId,
    pub header: EncryptedHeader,
    aad: Vec<u8>,
    ciphertext: Vec<u8>,
    object_hash: [u8; 32],
    // The returned decrypt event owns the second serialized copy. Removing/expiring the ledger
    // token must not refund it while an actual native pull/reply still holds these bytes.
    _custody: Arc<Credit>,
}
impl InboundPacket {
    pub(in crate::voice_v2) fn from_first_offer(
        token: u64,
        issuer: DeviceId,
        header: EncryptedHeader,
        ciphertext: Vec<u8>,
        aad: Vec<u8>,
        object_hash: [u8; 32],
        credit: Arc<Credit>,
    ) -> Self {
        Self {
            token,
            issuer,
            header,
            ciphertext,
            aad,
            object_hash,
            _custody: credit,
        }
    }
    pub(crate) fn object_hash(&self) -> [u8; 32] {
        self.object_hash
    }
    pub fn aad(&self) -> &[u8] {
        &self.aad
    }
    pub fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }
}
/// Affine prepared ingress. All retention and decrypt-event bytes are reserved, but neither the
/// replay window nor native original-issuer budget has changed. The endpoint commits both in
/// one synchronous actor turn, after its final exact-owner authorization inside Group -> Policy.
pub struct PreparedInbound {
    incoming: Incoming,
    event: InboundPacket,
    admission: InboundAdmission,
    serial: u64,
}
/// Validated, still unpublished token transaction. The exclusive ledger borrow prevents a
/// second actor operation from changing serial/replay/capacity between validation and the
/// infallible publication. All Clock callbacks run before this value is lent to a short native
/// Group -> Policy -> protected-row transaction; that transaction uses only check_at samples.
///
/// The decrypt event is returned separately so a protected action can MOVE its owned buffers
/// before origin/replay publication, without exporting an action or cloning another body.
pub(super) struct PreparedTokenPublication<'a> {
    ledger: &'a mut PacketLedger,
    incoming: Incoming,
    admission: InboundAdmission,
    token: u64,
    replay: Replay,
    mono: u64,
}
impl PreparedTokenPublication<'_> {
    /// Only this exclusive unpublished native token can construct its pending decrypt owner.
    /// The inactive action remains private until the same origin transaction publishes both.
    pub(super) fn crypto_authority(
        &self,
        clock: Arc<dyn Clock>,
    ) -> Result<super::crypto_authority::CryptoAuthority, VoiceError> {
        let grant = super::crypto_authority::CryptoAuthority::new(
            self.admission.local.clone(),
            self.admission.pair.clone(),
            clock,
            self.admission.deadline,
            self.admission.expires.saturating_add(CLOCK_SKEW_MS),
        )?
        .with_pending(self.incoming.token.0.clone())?;
        match self
            .incoming
            .confirmation
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?
        {
            PendingConfirmation::Hello(gate) => grant.with_hello(gate.clone()),
            PendingConfirmation::Attempt(attempt)
            | PendingConfirmation::ProtectedInbox(attempt) => grant.with_pending(attempt.clone()),
        }
    }
    pub(super) fn admission(&self) -> &InboundAdmission {
        &self.admission
    }
    /// No time seam, allocation failure result, callback or engine lock follows origin commit.
    /// As with existing map publication, process allocator failure is not recoverable here.
    pub(super) fn publish(self) {
        let Body::Encrypted { header, .. } = self.incoming.object.body() else {
            unreachable!("validated encrypted token")
        };
        self.ledger
            .publish_sdp_identity(header, self.incoming.object.id(), true, self.mono);
        self.ledger
            .pairs
            .get_mut(&header.handshake)
            .expect("exclusive validated pair")
            .replay = self.replay;
        self.ledger.serial = self.token;
        self.ledger.incoming.insert(self.token, self.incoming);
    }
}
impl PreparedInbound {
    /// Bind the actual native candidate/attempt before the replay/token transaction. This is
    /// still unpublished preparation: no caller can obtain an action or engine grant from it.
    /// The same binding follows both final origin checks and the eventual retained token.
    pub(super) fn bind_confirmation(
        mut self,
        confirmation: PendingConfirmation,
        deadline: u64,
    ) -> Result<Self, VoiceError> {
        if !matches!(
            (&confirmation, self.event.header.kind),
            (PendingConfirmation::Hello(_), EncryptedKind::ConfirmHello)
                | (
                    PendingConfirmation::Attempt(_) | PendingConfirmation::ProtectedInbox(_),
                    EncryptedKind::ConfirmProof | EncryptedKind::ConfirmFinish
                )
        ) || self.incoming.confirmation.is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        // Neither a later candidate nor an extended packet expiry can renew the pair's
        // original confirmation window. The prepared binding itself consumes no candidate.
        self.incoming.retire = self.incoming.retire.min(deadline);
        self.admission.deadline = self.admission.deadline.min(deadline);
        self.admission.confirmation = Some(confirmation.clone());
        self.incoming.confirmation = Some(confirmation);
        Ok(self)
    }
}
pub(crate) struct InboundAdmission {
    local: CallLease,
    pair: CallLease,
    expires: u64,
    deadline: u64,
    exchange: Option<Arc<Exchange>>,
    confirmation: Option<PendingConfirmation>,
    // Selected once from the native signed object's kind. The public application-only wrapper
    // cannot accidentally publish a confirming token without LocalJoin's prepared binding.
    confirmation_required: bool,
}
impl InboundAdmission {
    pub(crate) fn check(&self, clock: &dyn Clock) -> Result<(), VoiceError> {
        self.check_at(clock.now_ms(), clock.monotonic_ms())
    }
    /// Use the final Group -> Policy transaction's own samples. Calling an arbitrary Clock
    /// implementation again from inside those locks could reenter native state or move time.
    pub(crate) fn check_at(&self, now: u64, mono: u64) -> Result<(), VoiceError> {
        if !self.local.is_live()
            || !self.pair.is_live()
            || (self.confirmation_required && self.confirmation.is_none())
            || self
                .confirmation
                .as_ref()
                .is_some_and(|pending| !pending.authorized())
            || now > self.expires.saturating_add(CLOCK_SKEW_MS)
            || mono >= self.deadline
            || self
                .exchange
                .as_ref()
                .is_some_and(|exchange| !exchange.is_live())
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
}
/// Exact native post-decrypt receipt. Its private fields and affine ownership prevent a forged
/// header or reused receipt from driving confirmation. The Endpoint consumes it in the same
/// actor turn; retained native work keeps the original ciphertext's byte credit until release.
pub struct CommittedPacket {
    object: SignedObject,
    local_lease: CallLease,
    pair_lease: CallLease,
    retire: u64,
    _credit: Arc<Credit>,
}

/// The remainder of a consumed incoming row still owns revocation/watch, phase and table-slot
/// custody. A native final transaction must retire this OUTSIDE Engine/Group/Policy, even though
/// its CommittedPacket separately retains the actual signed body/byte credit.
pub(super) struct ConsumedIncoming {
    _token: RevocationOwner,
    _confirmation: Option<PendingConfirmation>,
    _slot: PacketSlot,
    _exchange: Option<Arc<Exchange>>,
}

/// Private synchronous transaction plan. It must not cross an await: its sole ledger owner
/// prepares closed-result work, repeats final checks, then consumes it without a fallible tail.
pub(super) struct ValidatedInbound {
    token: u64,
    pub message: MessageId,
    pub header: EncryptedHeader,
    pub hash: [u8; 32],
    pub expires: u64,
    pub retire: u64,
    pair: CallLease,
    token_owner: CallLease,
    local: CallLease,
    epoch: u64,
    issued: u64,
    confirmation: Option<PendingConfirmation>,
    exchange: Option<Arc<Exchange>>,
}
impl ValidatedInbound {
    /// Final pure observation for an exclusive synchronous result transaction. Header/tuple
    /// equality and all member/Clock callbacks have already run. These original owners may be
    /// revoked by a later native-origin time sample; never reconstruct them from current IDs.
    pub(super) fn check_at(&self, epoch: u64, wall: u64, mono: u64) -> Result<(), VoiceError> {
        if epoch != self.epoch
            || !self.local.is_live()
            || !self.pair.is_live()
            || !self.token_owner.is_live()
            || self
                .confirmation
                .as_ref()
                .is_some_and(|pending| !pending.authorized())
            || self
                .exchange
                .as_ref()
                .is_some_and(|exchange| !exchange.is_live())
            || mono >= self.retire
        {
            return Err(VoiceError::Unauthorized);
        }
        freshness(self.issued, self.expires, wall)
    }
}

/// Packet-specific source authority, created only for the ledger's exact cached signed bytes.
/// Pair retirement/promotion closes queued sends independently of the still-live local join.
#[derive(Clone)]
pub(super) struct SourcePacketAuthority {
    id: MessageId,
    signature: [u8; 64],
    local: CallLease,
    pair: CallLease,
    packet: CallLease,
    chain: Option<CallLease>,
    workflow: Option<Arc<super::chain::Workflow>>,
    clock: Arc<dyn Clock>,
    deadline: u64,
    exchange: Option<Arc<Exchange>>,
    sdp_binding: Option<Arc<super::sdp::action::ActionBinding>>,
    expires: u64,
}
impl SourcePacketAuthority {
    pub(super) fn obsolete_confirmation(&self) -> bool {
        self.workflow.as_ref().is_some_and(|workflow| {
            workflow.high_confirmation_sources_retired()
                || workflow.low_confirmation_sources_retired()
        })
    }
    /// Always use this packet's actual original clock. The caller may not replace its
    /// monotonic origin with the enclosing source job's clock, even if wall time agrees.
    pub(super) fn observation_time(&self) -> (u64, u64) {
        (self.clock.now_ms(), self.clock.monotonic_ms())
    }
    /// Pure composition after ALL constituent clocks have been sampled through their owners.
    /// No callback may follow these gates in its containing final authority observation.
    pub(super) fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        self.local.is_live()
            && self.pair.is_live()
            && self.packet.is_live()
            && self.chain.as_ref().is_none_or(CallLease::is_live)
            && self
                .workflow
                .as_ref()
                .is_none_or(|workflow| !workflow.low_confirmation_sources_retired())
            && mono < self.deadline
            && wall < self.expires
            && self
                .exchange
                .as_ref()
                .is_none_or(|exchange| exchange.is_live())
            && self
                .sdp_binding
                .as_ref()
                .is_none_or(|binding| binding.current_at(wall, mono))
    }
    pub(super) fn exchange(&self) -> Option<Arc<Exchange>> {
        self.exchange.clone()
    }
    pub(super) fn progress(&self) -> super::source::SourceProgress<'_> {
        match (&self.workflow, &self.exchange) {
            (Some(workflow), None) => super::source::SourceProgress::Workflow(workflow),
            (None, Some(exchange)) => super::source::SourceProgress::Exchange(exchange),
            (None, None) => super::source::SourceProgress::Ordinary,
            // Native packet construction never combines confirmation and SDP producers.
            // Keep this invariant explicit rather than silently selecting one payment.
            (Some(_), Some(_)) => unreachable!("one original packet progress producer"),
        }
    }
    /// A native action can have a shorter lifetime than its signed packet. Capture that
    /// immutable deadline in queued work so an idle endpoint cannot extend it by omitting GC.
    pub(super) fn limit_deadline(mut self, deadline: u64) -> Result<Self, VoiceError> {
        self.deadline = self.deadline.min(deadline);
        if !self.authorized() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(self)
    }
    pub(super) fn check(&self, object: &SignedObject, local: &CallLease) -> bool {
        self.authorized()
            && self.local.is_same(local)
            && self.id == object.id()
            && self.signature == object.signature
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        self.id == other.id
            && self.signature == other.signature
            && self.local.is_same(&other.local)
            && self.pair.is_same(&other.pair)
            && self.packet.is_same(&other.packet)
            && match (&self.chain, &other.chain) {
                (None, None) => true,
                (Some(first), Some(second)) => first.is_same(second),
                _ => false,
            }
            && self.deadline == other.deadline
            && match (&self.workflow, &other.workflow) {
                (None, None) => true,
                (Some(first), Some(second)) => Arc::ptr_eq(first, second),
                _ => false,
            }
            && self.expires == other.expires
            && match (&self.exchange, &other.exchange) {
                (None, None) => true,
                (Some(first), Some(second)) => Arc::ptr_eq(first, second),
                _ => false,
            }
            && match (&self.sdp_binding, &other.sdp_binding) {
                (None, None) => true,
                (Some(first), Some(second)) => Arc::ptr_eq(first, second),
                _ => false,
            }
    }
}
#[async_trait::async_trait]
impl catcoms_rt::RequestAuthority for SourcePacketAuthority {
    fn authorized(&self) -> bool {
        // Sampling can synchronously revoke any captured native owner. No Clock/callback may
        // follow these irreversible lease observations in this capability check.
        let mono = self.clock.monotonic_ms();
        let wall = self.clock.now_ms();
        self.authorized_at(wall, mono)
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let mut waits = vec![
            self.local.wait_revoked().boxed(),
            self.pair.wait_revoked().boxed(),
            self.packet.wait_revoked().boxed(),
            self.clock.sleep(std::time::Duration::from_millis(
                self.deadline.saturating_sub(self.clock.monotonic_ms()),
            )),
        ];
        if let Some(exchange) = &self.exchange {
            waits.push(exchange.revoked().boxed());
        }
        if let Some(binding) = &self.sdp_binding {
            waits.push(binding.closed().boxed());
        }
        if let Some(chain) = &self.chain {
            waits.push(chain.wait_revoked().boxed());
        }
        futures::future::select_all(waits).await;
    }
}
redacted_debug!(SourcePacketAuthority);
impl CommittedPacket {
    pub fn header(&self) -> &EncryptedHeader {
        let Body::Encrypted { header, .. } = self.object.body() else {
            unreachable!()
        };
        header
    }
    pub fn id(&self) -> MessageId {
        self.object.id()
    }
    pub fn hash(&self) -> [u8; 32] {
        self.object.hash()
    }
}
redacted_debug!(
    Authority<'_>,
    PairBinding,
    PacketLedger,
    ReserveRequest,
    ReservedPacket,
    InboundPacket,
    PreparedInbound,
    InboundAdmission,
    CommittedPacket
);

impl PacketLedger {
    #[cfg(test)]
    pub(in crate::voice_v2) fn initial_offer_state_for_test(
        &self,
        handshake: Nonce,
    ) -> (u64, Option<MessageId>) {
        (
            self.pairs[&handshake].next,
            self.sdp.as_ref().and_then(|sdp| sdp.local),
        )
    }
    /// Only exact committed native Ready makes LOW's old Finish obsolete. Closing the
    /// whole chain here would also close the verifier lane before its reply can drain.
    pub(in crate::voice_v2) fn retire_low_confirmation_cache(
        &mut self,
        chain: &super::chain::PreparedChain,
    ) -> Result<(), VoiceError> {
        let binding = chain
            .ready_low_confirmation_binding()
            .ok_or(VoiceError::Unauthorized)?;
        if binding.scope != self.scope
            || binding.local != self.local
            || binding.local_join != self.join
            || binding.local_key != self.key_ref
            || !binding.call.is_same(&self.lease)
        {
            return Err(VoiceError::Scope);
        }
        self.outgoing.retain(|_, outgoing| {
            if outgoing
                .workflow
                .as_ref()
                .is_some_and(|workflow| workflow.belongs_to_plan(chain))
            {
                outgoing.live.revoke();
                false
            } else {
                true
            }
        });
        // A selected Proof was consumed at local Finish, but earlier authenticated
        // candidates may still occupy native inboxes. They cannot progress after Ready.
        // Only remove this exact plan's native cache; independently retained SDK/result
        // input still owns its credit and continues to delay real Workflow drainage.
        self.incoming.retain(|_, incoming| {
            let Body::Encrypted { header, .. } = incoming.object.body() else {
                return true;
            };
            !(incoming.confirmation.is_some()
                && incoming.object.scope() == binding.scope
                && incoming.object.issuer() == binding.peer
                && header.target == binding.local
                && header.source_join == binding.peer_join
                && header.target_join == binding.local_join
                && header.source_key_ref == binding.peer_key
                && header.target_key_ref == binding.local_key
                && header.handshake == binding.handshake
                && header.kind == EncryptedKind::ConfirmProof)
        });
        Ok(())
    }
    /// Remove only obsolete confirmation CACHE owners for this exact native HIGH plan.
    /// Native Finish-open proves LOW received Proof; closing/extraction already forbids
    /// every old action. Fixed Pair counters/replay and native result/admission histories
    /// stay untouched. Actual inputs/SDK/Source/Lower/reply owners retain their own credits.
    /// This is never the LOW Finish retirement path, and runs outside Group/Policy guards.
    pub(in crate::voice_v2) fn retire_high_confirmation_cache(
        &mut self,
        chain: &super::chain::PreparedChain,
    ) -> Result<(), VoiceError> {
        let binding = chain
            .retired_high_confirmation_binding()
            .ok_or(VoiceError::Unauthorized)?;
        if binding.scope != self.scope
            || binding.local != self.local
            || binding.local_join != self.join
            || binding.local_key != self.key_ref
            || !binding.call.is_same(&self.lease)
        {
            return Err(VoiceError::Scope);
        }
        self.outgoing.retain(|_, outgoing| {
            !outgoing
                .workflow
                .as_ref()
                .is_some_and(|workflow| workflow.belongs_to_plan(chain))
        });
        self.incoming.retain(|_, incoming| {
            let Body::Encrypted { header, .. } = incoming.object.body() else {
                return true;
            };
            // LocalJoin permanently selects one protected chain before any ordinary use
            // of this handshake. Its immutable pair binding plus these exact header fields
            // therefore identify this plan; ordinary or unrelated incoming rows stay live.
            // If multiple plans per handshake ever become legal, add original-plan identity
            // to Incoming before changing that selection invariant.
            !(incoming.confirmation.is_some()
                && incoming.object.scope() == binding.scope
                && incoming.object.issuer() == binding.peer
                && header.target == binding.local
                && header.source_join == binding.peer_join
                && header.target_join == binding.local_join
                && header.source_key_ref == binding.peer_key
                && header.target_key_ref == binding.local_key
                && header.handshake == binding.handshake
                && matches!(
                    header.kind,
                    EncryptedKind::ConfirmHello
                        | EncryptedKind::ConfirmProof
                        | EncryptedKind::ConfirmFinish
                ))
        });
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn pair_count_for_test(&self) -> usize {
        self.pairs.len()
    }
    #[cfg(test)]
    pub(super) fn has_sdp_for_test(&self) -> bool {
        self.sdp.is_some()
    }
    /// Protection is selected only before this exact pair has allocated or admitted any packet.
    /// Counters survive cache pruning, so an expired ordinary action cannot look pristine later.
    pub(super) fn confirmation_is_unused(
        &self,
        handshake: Nonce,
        authority: &Authority<'_>,
    ) -> Result<bool, VoiceError> {
        let pair = self.pair(&handshake, authority)?;
        Ok(pair.status == Status::Confirming && pair.next == 0 && pair.replay.highest == 0)
    }
    pub(super) fn select_complete_sdp_profile(
        &mut self,
        handshake: Nonce,
        authority: &Authority<'_>,
    ) -> Result<(), VoiceError> {
        self.pair(&handshake, authority)?;
        self.pairs
            .get_mut(&handshake)
            .expect("same exclusive pair")
            .sdp_profile = Some(super::SdpProfile::CompleteSdpV1);
        Ok(())
    }
    pub fn new(
        context: &Context,
        join: Nonce,
        key_ref: [u8; 32],
        lease: CallLease,
        resources: Arc<Resources>,
    ) -> Result<Self, VoiceError> {
        lease.claim_packet_ledger()?;
        Ok(Self {
            scope: context.scope,
            local: context.local,
            join,
            key_ref,
            lease,
            resources,
            pairs: BTreeMap::new(),
            outgoing: BTreeMap::new(),
            incoming: BTreeMap::new(),
            serial: 0,
            sdp: None,
            action_slots: Arc::new(super::crypto_actions::slots::Slots::default()),
            packet_slots: PacketSlots::new(),
        })
    }
    /// Root transport regressions begin at an established native pair without claiming an
    /// actual browser handshake. Keep this fixture capability unavailable in production/IPC.
    #[cfg(test)]
    pub(crate) fn established_sdp_pair_for_test(
        &mut self,
        binding: PairBinding,
        authority: &Authority<'_>,
        limits: &mut super::limits::OriginLimits,
        helpers: &[DeviceId],
        clock: &dyn Clock,
    ) -> Result<(), VoiceError> {
        let handshake = binding.handshake;
        self.register_pair(binding, authority)?;
        self.establish(&handshake, authority)?;
        self.prepare_sdp_exchange(handshake, authority, limits, helpers, clock)?;
        Ok(())
    }

    /// Native readiness must prepay this before either SDP direction is positively admitted.
    /// It is deliberately separate from confirmation's irreversible transition: a local
    /// capacity refusal cannot strand an already consumed confirmation token.
    pub(super) fn prepare_sdp_exchange(
        &mut self,
        handshake: [u8; 32],
        authority: &Authority<'_>,
        limits: &mut super::limits::OriginLimits,
        helpers: &[DeviceId],
        clock: &dyn Clock,
    ) -> Result<Arc<Exchange>, VoiceError> {
        self.prune(clock);
        let pair = self.pair(&handshake, authority)?;
        if pair.status != Status::Established {
            return Err(VoiceError::Unauthorized);
        }
        if helpers.len() > 2 {
            return Err(VoiceError::Bounds);
        }
        if limits.group() != self.scope.group {
            return Err(VoiceError::Scope);
        }
        let mut helpers = helpers.to_vec();
        helpers.sort();
        if helpers.windows(2).any(|pair| pair[0] == pair[1])
            || helpers
                .iter()
                .any(|helper| *helper == self.local || *helper == pair.binding.peer)
        {
            return Err(VoiceError::Malformed);
        }
        // Readiness cannot promise a helper route that the current scope forbids. Repeat
        // membership even for exact retries; cached capacity is never cached authorization.
        if (!helpers.is_empty()
            && (!authority.context.indirect || authority.context.scope.policy.is_none()))
            || helpers.iter().any(|helper| !(authority.member)(*helper))
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(sdp) = &self.sdp {
            if sdp.owner.is_live() {
                return if sdp.handshake == handshake && sdp.owner.same_helpers(&helpers) {
                    Ok(sdp.owner.clone())
                } else {
                    Err(VoiceError::Capacity)
                };
            }
        }
        let mut members = vec![self.local, pair.binding.peer];
        members.extend_from_slice(&helpers);
        let readiness_stamp = clock.monotonic_ms();
        limits.prepare_exchange_members(&members, &self.resources, readiness_stamp)?;
        let binding = ExchangeBinding {
            scope: self.scope,
            local: self.local,
            join: self.join,
            key_ref: self.key_ref,
            peer: pair.binding.clone(),
            call: self.lease.clone(),
            pair: pair.live.clone(),
            helpers,
        };
        // Release this ledger's closed owner before competing for a new registered phase.
        // Actual input/Source/Lower borrowers or an external closed handle still retain the
        // old owner and correctly refuse replacement until their real last drop.
        drop(self.sdp.take());
        let slots = self.action_slots.reserve(2)?;
        let owner = Exchange::new_bound(self.resources.clone(), binding, slots)?;
        self.sdp = Some(SdpExchange {
            owner: owner.clone(),
            handshake,
            role: SdpRole::Ready,
            local: None,
            remote: None,
            // The same stamp covers prerequisite retention and idle readiness. Preemption
            // during prepayment cannot silently extend readiness beyond those member rows.
            ready_until: readiness_stamp.saturating_add(MAX_DELEGATION_LIFETIME_MS),
            retire: None,
        });
        Ok(owner)
    }
    pub(super) fn action_slots(&self) -> Arc<super::crypto_actions::slots::Slots> {
        self.action_slots.clone()
    }
    pub(super) fn packet_slots(&self) -> &PacketSlots {
        &self.packet_slots
    }
    fn sdp_owner(
        &self,
        header: &EncryptedHeader,
        incoming: bool,
    ) -> Result<Option<Arc<Exchange>>, VoiceError> {
        if !header.kind.is_sdp() {
            return Ok(None);
        }
        let sdp = self.sdp.as_ref().ok_or(VoiceError::Capacity)?;
        if sdp.handshake != header.handshake
            || !sdp.owner.is_live()
            || !sdp.owner.accepts_sdp_kind(header.kind)
        {
            return Err(VoiceError::Unauthorized);
        }
        if (incoming && sdp.remote.is_some()) || (!incoming && sdp.local.is_some()) {
            return Err(VoiceError::Capacity);
        }
        let permitted = match (incoming, header.kind, sdp.role) {
            (_, EncryptedKind::Offer | EncryptedKind::OfferRound(_), SdpRole::Ready) => true,
            (
                true,
                EncryptedKind::Answer | EncryptedKind::AnswerRound(_),
                SdpRole::Initiator(offer),
            )
            | (
                false,
                EncryptedKind::Answer | EncryptedKind::AnswerRound(_),
                SdpRole::Responder(offer),
            ) => header.offer == Some(offer),
            _ => false,
        };
        if !permitted {
            return Err(VoiceError::Unauthorized);
        }
        Ok(Some(sdp.owner.clone()))
    }
    fn publish_sdp_identity(
        &mut self,
        header: &EncryptedHeader,
        id: MessageId,
        incoming: bool,
        mono: u64,
    ) {
        if !header.kind.is_sdp() {
            return;
        }
        let sdp = self
            .sdp
            .as_mut()
            .expect("prepaid SDP owner checked before commit");
        // Idle readiness is separately bounded. Start the immutable work window with the
        // first admitted SDP, so waiting before an Offer cannot shorten its fresh reply.
        sdp.retire
            .get_or_insert(mono.saturating_add(MAX_PACKET_LIFETIME_MS + CLOCK_SKEW_MS));
        if header.kind.is_offer() {
            let offer = header.offer.expect("validated OfferId");
            sdp.role = if incoming {
                SdpRole::Responder(offer)
            } else {
                SdpRole::Initiator(offer)
            };
        }
        if incoming {
            sdp.remote = Some(id);
        } else {
            sdp.local = Some(id);
        }
    }
    fn sdp_deadline(&self, exchange: Option<&Arc<Exchange>>) -> Result<u64, VoiceError> {
        let Some(exchange) = exchange else {
            return Ok(u64::MAX);
        };
        let sdp = self.sdp.as_ref().ok_or(VoiceError::Unauthorized)?;
        if !Arc::ptr_eq(exchange, &sdp.owner) || !exchange.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        // Capacity retirement is collected separately. Every effect-bearing crypto/source
        // grant is bounded here, so delayed housekeeping cannot extend the work window.
        Ok(sdp.retire.unwrap_or(sdp.ready_until))
    }
    fn current(&self, authority: &Authority<'_>) -> Result<(), VoiceError> {
        if !self.lease.is_live()
            || !self.lease.is_same(authority.lease)
            || !authority.lease.is_live()
            || self.scope != authority.context.scope
            || self.local != authority.context.local
            || !(authority.member)(self.local)
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    fn pair(&self, handshake: &[u8; 32], authority: &Authority<'_>) -> Result<&Pair, VoiceError> {
        self.current(authority)?;
        let pair = self.pairs.get(handshake).ok_or(VoiceError::Unauthorized)?;
        if !(authority.member)(pair.binding.peer) {
            pair.live.revoke();
        }
        if pair.status == Status::Retired || !pair.live.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(pair)
    }
    /// Called only after native fresh-join/key admission. Records/counters remain for the whole
    /// local join, including retirement; cache eviction or a failed handshake cannot reset them.
    fn register_pair_owned(
        &mut self,
        binding: PairBinding,
        live: CallLease,
        authority: &Authority<'_>,
    ) -> Result<(), VoiceError> {
        self.current(authority)?;
        if binding.peer == self.local || !(authority.member)(binding.peer) || !live.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(pair) = self.pairs.get(&binding.handshake) {
            return if pair.binding == binding
                && pair.status != Status::Retired
                && pair.live.is_live()
                && pair.live.is_same(&live)
            {
                Ok(())
            } else {
                Err(VoiceError::Unauthorized)
            };
        }
        if self.pairs.len() >= PAIRS {
            return Err(VoiceError::Capacity);
        }
        self.pairs.insert(
            binding.handshake,
            Pair {
                binding,
                status: Status::Confirming,
                live,
                confirmation_claimed: false,
                sdp_profile: None,
                next: 0,
                replay: Replay::default(),
            },
        );
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn register_pair(
        &mut self,
        binding: PairBinding,
        authority: &Authority<'_>,
    ) -> Result<(), VoiceError> {
        let live = self
            .pairs
            .get(&binding.handshake)
            .map_or_else(CallLease::default, |pair| pair.live.clone());
        self.register_pair_owned(binding, live, authority)
    }
    /// A native join owns exactly one confirmation state per stable pair transcript. Failed
    /// handshakes may retry within that state; recreating it must never reset attempt history.
    pub(super) fn claim_confirmation(
        &mut self,
        binding: PairBinding,
        live: CallLease,
        authority: &Authority<'_>,
    ) -> Result<(), VoiceError> {
        let handshake = binding.handshake;
        // The ledger, confirmation owner, cached-flight authority and origin key pin share this
        // ONE irreversible revocation handle. A failed ledger commit must revoke cached paths
        // too; separate "equivalent" booleans would permit removal/re-add resurrection.
        self.register_pair_owned(binding, live, authority)?;
        let pair = self.pairs.get_mut(&handshake).expect("registered pair");
        if pair.confirmation_claimed {
            return Err(VoiceError::Unauthorized);
        }
        pair.confirmation_claimed = true;
        Ok(())
    }

    /// Native production pairs have never occupied this transcript. Unlike compatibility
    /// registration, this cannot adopt an already-created unclaimed/retired ledger row.
    pub(super) fn prepare_confirmation_claim(
        &self,
        binding: PairBinding,
        live: CallLease,
        authority: &Authority<'_>,
    ) -> Result<PreparedConfirmationClaim, VoiceError> {
        self.current(authority)?;
        if binding.peer == self.local
            || !(authority.member)(binding.peer)
            || !live.is_live()
            || self.pairs.contains_key(&binding.handshake)
        {
            return Err(VoiceError::Unauthorized);
        }
        if self.pairs.len() >= PAIRS {
            return Err(VoiceError::Capacity);
        }
        Ok(PreparedConfirmationClaim(Pair {
            binding,
            status: Status::Confirming,
            live,
            confirmation_claimed: true,
            // This constructor is exclusively the native protected-pair transaction.
            // LocalJoin publishes protected_epoch in the SAME tail, so select the packet
            // profile here too. Waiting for prepare_crypto_chain would skip selection
            // because that original epoch is already latched, and native SDP would fail.
            sdp_profile: Some(super::SdpProfile::CompleteSdpV1),
            next: 0,
            replay: Replay::default(),
        }))
    }

    /// Infallible native tail after all call/room/epoch/evidence checks. Both bounded tables
    /// publish in the same actor turn; no async/native callback can run between their inserts.
    pub(super) fn publish_confirmation_claim(&mut self, prepared: PreparedConfirmationClaim) {
        debug_assert!(self.pairs.len() < PAIRS);
        debug_assert!(!self.pairs.contains_key(&prepared.0.binding.handshake));
        self.pairs.insert(prepared.0.binding.handshake, prepared.0);
    }
    /// Private-protocol transition after the role-specific three-flight confirmation succeeds.
    #[cfg(test)]
    pub(super) fn establish(
        &mut self,
        handshake: &[u8; 32],
        authority: &Authority<'_>,
    ) -> Result<(), VoiceError> {
        self.pair(handshake, authority)?;
        self.establish_validated(handshake);
        Ok(())
    }
    pub(super) fn validate_establish(
        &self,
        handshake: &[u8; 32],
        authority: &Authority<'_>,
    ) -> Result<(), VoiceError> {
        self.pair(handshake, authority).map(|_| ())
    }
    /// Apply only inside the synchronous confirmation transaction after final owner validation.
    pub(super) fn establish_validated(&mut self, handshake: &[u8; 32]) {
        self.pairs
            .get_mut(handshake)
            .expect("validated pair")
            .status = Status::Established;
    }
    pub fn retire_member(&mut self, peer: DeviceId) {
        for pair in self
            .pairs
            .values_mut()
            .filter(|pair| pair.binding.peer == peer)
        {
            pair.status = Status::Retired;
            pair.live.revoke();
        }
        self.incoming
            .retain(|_, incoming| incoming.object.issuer != peer);
    }
    pub(super) fn retire_handshake(&mut self, handshake: &[u8; 32]) {
        self.retire_handshake_authority(handshake);
        self.clear_handshake_incoming(handshake);
    }
    /// Immediate revocation only. Native result publication defers actual table cleanup until
    /// Engine/Group/Policy guards return; dropping a row here would retire its real owners.
    pub(super) fn retire_handshake_authority(&mut self, handshake: &[u8; 32]) {
        if let Some(pair) = self.pairs.get_mut(handshake) {
            pair.status = Status::Retired;
            pair.live.revoke();
        }
    }
    pub(super) fn clear_handshake_incoming(&mut self, handshake: &[u8; 32]) {
        self.incoming.retain(|_, incoming| !matches!(incoming.object.body(), Body::Encrypted { header, .. } if &header.handshake == handshake));
    }
    /// Opaque receipts are still revalidated against current native pair ownership before a
    /// protocol transition. A receipt captured before member removal/promotion cannot commit.
    pub fn receipt_current(
        &self,
        receipt: &CommittedPacket,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<(), VoiceError> {
        let monotonic = clock.monotonic_ms();
        receipt.object.check_freshness(clock.now_ms())?;
        let pair = self.pair(&receipt.header().handshake, authority)?;
        if !receipt.local_lease.is_same(&self.lease)
            || !receipt.local_lease.is_live()
            || !receipt.pair_lease.is_same(&pair.live)
            || !receipt.pair_lease.is_live()
            || monotonic >= receipt.retire
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    pub(super) fn collect_expired(&mut self, clock: &dyn Clock) {
        self.prune(clock);
    }
    fn prune(&mut self, clock: &dyn Clock) {
        let now = clock.now_ms();
        let mono = clock.monotonic_ms();
        if self.sdp.as_ref().is_some_and(|sdp| {
            mono >= sdp.retire.unwrap_or(sdp.ready_until) || !sdp.owner.is_live()
        }) {
            let sdp = self.sdp.take().expect("expired original SDP");
            sdp.owner.close();
        }
        self.outgoing.retain(|_, outgoing| {
            mono < outgoing.retire && now <= outgoing.draft.expires.saturating_add(CLOCK_SKEW_MS)
        });
        self.incoming.retain(|_, incoming| {
            mono < incoming.retire && now <= incoming.object.expires.saturating_add(CLOCK_SKEW_MS)
        });
    }
    pub fn reserve(
        &mut self,
        public_key: [u8; 32],
        request: ReserveRequest,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
    ) -> Result<ReservedPacket, VoiceError> {
        self.prune(clock);
        let prepared = self.prepare_outgoing(public_key, request, authority, clock, rng)?;
        let (publication, reservation) =
            self.validate_prepared_outgoing(prepared, authority, clock)?;
        publication.admission().check_at(
            authority.context.epoch,
            clock.now_ms(),
            clock.monotonic_ms(),
        )?;
        // Ordinary callers receive no reservation until its sequence and row are published.
        // A protected producer instead composes this loan with its final candidate/action fence.
        publication.publish();
        Ok(reservation)
    }
    /// One atomic ciphertext pin/signature. A cancelled IPC response can retry exact bytes; an
    /// altered retry never replaces the signed cache. Expired ids cannot reconstruct a draft.
    pub fn submit(
        &mut self,
        id: MessageId,
        ciphertext: &[u8],
        signer: &(impl VoiceSigner + ?Sized),
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<SignedObject, VoiceError> {
        let prepared: PreparedSealedPublication<'_> =
            self.prepare_sealed_publication(id, ciphertext, signer, authority, clock)?;
        prepared.check_at(
            authority.context.epoch,
            clock.now_ms(),
            clock.monotonic_ms(),
        )?;
        Ok(prepared.publish())
    }
    pub(super) fn outgoing_crypto_authority(
        &self,
        reserved: &ReservedPacket,
        confirmation: bool,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
    ) -> Result<super::crypto_authority::CryptoAuthority, VoiceError> {
        self.current(authority)?;
        let outgoing = self
            .outgoing
            .get(&reserved.id)
            .ok_or(VoiceError::Reservation)?;
        let pair = self.pair(&outgoing.draft.header.handshake, authority)?;
        let is_confirmation = matches!(
            outgoing.draft.header.kind,
            EncryptedKind::ConfirmHello
                | EncryptedKind::ConfirmProof
                | EncryptedKind::ConfirmFinish
        );
        if confirmation != is_confirmation
            || outgoing.sealed.published().is_some()
            || !outgoing.live.is_live()
            || reserved.aad != outgoing.draft.aad()
            || reserved.sequence != outgoing.draft.header.sequence
            || reserved.expires != outgoing.draft.expires
        {
            return Err(VoiceError::Unauthorized);
        }
        let grant = super::crypto_authority::CryptoAuthority::new(
            self.lease.clone(),
            pair.live.clone(),
            clock,
            outgoing
                .construction
                .min(outgoing.source_deadline)
                .min(self.sdp_deadline(outgoing.exchange.as_ref())?),
            outgoing.draft.expires,
        )?
        .with_pending(outgoing.crypto.0.clone())?;
        match &outgoing.chain {
            Some(chain) => grant.with_pending(chain.clone()),
            None => Ok(grant),
        }
    }
    pub(super) fn incoming_crypto_authority(
        &self,
        event: &InboundPacket,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
    ) -> Result<super::crypto_authority::CryptoAuthority, VoiceError> {
        self.current(authority)?;
        let incoming = self
            .incoming
            .get(&event.token)
            .ok_or(VoiceError::Reservation)?;
        let Body::Encrypted { header, ciphertext } = incoming.object.body() else {
            unreachable!()
        };
        let pair = self.pair(&header.handshake, authority)?;
        if event.issuer != incoming.object.issuer()
            || &event.header != header
            || event.aad != incoming.object.aad()?
            || &event.ciphertext != ciphertext
        {
            return Err(VoiceError::RetryMismatch);
        }
        let grant = super::crypto_authority::CryptoAuthority::new(
            self.lease.clone(),
            pair.live.clone(),
            clock,
            incoming
                .retire
                .min(self.sdp_deadline(incoming.exchange.as_ref())?),
            incoming.object.expires().saturating_add(CLOCK_SKEW_MS),
        )?
        .with_pending(incoming.token.0.clone())?;
        if matches!(
            header.kind,
            EncryptedKind::ConfirmHello
                | EncryptedKind::ConfirmProof
                | EncryptedKind::ConfirmFinish
        ) {
            match incoming
                .confirmation
                .as_ref()
                .ok_or(VoiceError::Unauthorized)?
            {
                PendingConfirmation::Attempt(lease)
                | PendingConfirmation::ProtectedInbox(lease) => grant.with_pending(lease.clone()),
                PendingConfirmation::Hello(gate) => grant.with_hello(gate.clone()),
            }
        } else {
            Ok(grant)
        }
    }
    /// Bounded, borrowed cache inspection for the native sender. This neither signs an
    /// object nor grants authority to send it; source_authority checks the actual owners.
    pub(super) fn published_source_packets(&self) -> impl Iterator<Item = &SignedObject> {
        // Never expose an inert Prepared signature to Source. Authority is obtained again
        // from the exact original row before each admission; this iterator grants none.
        self.outgoing
            .values()
            .take(RESERVATIONS)
            .filter_map(|outgoing| outgoing.sealed.published())
    }
    /// Exact native outgoing receipt, not a caller-supplied signature or a join-wide grant.
    /// Equality compares immutable fields without allocating canonical packet copies.
    pub(super) fn source_authority(
        &self,
        object: &SignedObject,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
    ) -> Result<SourcePacketAuthority, VoiceError> {
        self.current(authority)?;
        let Body::Encrypted { header, .. } = object.body() else {
            return Err(VoiceError::Unauthorized);
        };
        let pair = self.pair(&header.handshake, authority)?;
        let outgoing = self
            .outgoing
            .get(&object.id())
            .ok_or(VoiceError::Reservation)?;
        if !outgoing.live.is_live()
            || outgoing.sealed.published() != Some(object)
            || outgoing
                .chain
                .as_ref()
                .is_some_and(|chain| !chain.is_live())
            || object.issuer() != self.local
            || object.scope() != self.scope
            || clock.monotonic_ms() >= outgoing.source_deadline
            || clock.now_ms() >= object.expires()
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(SourcePacketAuthority {
            id: object.id(),
            signature: object.signature,
            local: self.lease.clone(),
            pair: pair.live.clone(),
            packet: outgoing.live.clone(),
            chain: outgoing.chain.clone(),
            workflow: outgoing.workflow.clone(),
            deadline: outgoing
                .source_deadline
                .min(self.sdp_deadline(outgoing.exchange.as_ref())?),
            expires: object.expires(),
            exchange: outgoing.exchange.clone(),
            sdp_binding: outgoing.sdp_binding.clone(),
            clock,
        })
    }
    /// Native action/OfferId supersession retires the exact outgoing owner without deleting its
    /// signed cache, refunding the reservation or restoring sequence/attempt allowances.
    pub(super) fn retire_outgoing(
        &self,
        id: MessageId,
        authority: &Authority<'_>,
    ) -> Result<(), VoiceError> {
        self.current(authority)?;
        self.retire_outgoing_validated(id);
        Ok(())
    }
    /// This is the infallible half of a prepared confirmation transition. No new authority is
    /// minted: a stale revocation remains safe, and every later send repeats its current gate.
    pub(super) fn retire_outgoing_validated(&self, id: MessageId) {
        // Expiry may already have pruned an unsigned draft. Idempotent retirement must not
        // prevent its owner's next action; it never recreates a missing reservation.
        if let Some(outgoing) = self.outgoing.get(&id) {
            outgoing.live.revoke();
            outgoing.crypto.0.revoke();
            if let Some(exchange) = &outgoing.exchange {
                exchange.close();
            }
        }
    }
    /// Original signature/route verification precedes this call. Replay is charged at admission,
    /// before UI decrypt; helper replacement and alternate connections cannot reset the window.
    /// This generic wrapper admits application packets only. Confirmation requires LocalJoin's
    /// actual prepared candidate/attempt binding; an unbound confirmation always refuses.
    pub fn admit_inbound(
        &mut self,
        object: SignedObject,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<InboundPacket, VoiceError> {
        let prepared = self.prepare_inbound(object, authority, clock)?;
        self.commit_prepared_inbound(prepared, authority, clock, |admission| {
            admission.check(clock)
        })
    }
    fn inbound_pair<'a>(
        &'a self,
        object: &SignedObject,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<&'a Pair, VoiceError> {
        self.current(authority)?;
        object.check_freshness(clock.now_ms())?;
        if object.scope != self.scope {
            return Err(VoiceError::Scope);
        }
        let Body::Encrypted { header, .. } = &object.body else {
            return Err(VoiceError::Malformed);
        };
        let pair = self.pair(&header.handshake, authority)?;
        pair.check_profile(header.kind)?;
        if object.issuer != pair.binding.peer
            || header.target != self.local
            || header.target_join != self.join
            || header.source_join != pair.binding.join
            || header.target_key_ref != self.key_ref
            || header.source_key_ref != pair.binding.key_ref
        {
            return Err(VoiceError::Unauthorized);
        }
        let confirmation = matches!(
            header.kind,
            EncryptedKind::ConfirmHello
                | EncryptedKind::ConfirmProof
                | EncryptedKind::ConfirmFinish
        );
        if confirmation != (pair.status == Status::Confirming) {
            return Err(VoiceError::Unauthorized);
        }
        Ok(pair)
    }
    /// Preparation may be abandoned on capacity, rate or native-owner refusal without consuming
    /// a replay sequence. An immutable exact retry remains eligible for a later fresh attempt.
    pub(crate) fn prepare_inbound(
        &mut self,
        object: SignedObject,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<PreparedInbound, VoiceError> {
        self.prepare_inbound_with_workflow(object, authority, clock, None)
    }
    /// Protected native confirmation. Both retained serialized copies borrow bytes and the
    /// actual incoming-table position from the original workflow, never ordinary spare capacity.
    pub(super) fn prepare_protected_confirmation(
        &mut self,
        object: SignedObject,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        workflow: Arc<super::chain::Workflow>,
    ) -> Result<PreparedInbound, VoiceError> {
        self.prepare_inbound_with_workflow(object, authority, clock, Some(workflow))
    }
    fn prepare_inbound_with_workflow(
        &mut self,
        object: SignedObject,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        workflow: Option<Arc<super::chain::Workflow>>,
    ) -> Result<PreparedInbound, VoiceError> {
        self.prune(clock);
        let pair = self.inbound_pair(&object, authority, clock)?;
        let Body::Encrypted { header, ciphertext } = &object.body else {
            unreachable!()
        };
        let mut replay = pair.replay.clone();
        replay.admit(header.sequence)?;
        let pair_lease = pair.live.clone();
        if self.incoming.len() >= RESERVATIONS {
            return Err(VoiceError::Capacity);
        }
        let exchange = self.sdp_owner(header, true)?;
        let slot = if let Some(workflow) = &workflow {
            workflow
                .incoming_confirmation_slot(
                    &self.resources,
                    self.scope,
                    &self.lease,
                    &self.packet_slots,
                    &object,
                )?
                .into()
        } else if let Some(exchange) = exchange
            .as_ref()
            .filter(|owner| owner.uses_successor_packet_claims())
        {
            exchange.incoming_sdp_slot(header, &self.packet_slots)?
        } else {
            self.packet_slots.incoming.charge(1)?.into()
        };
        let token = self.serial.checked_add(1).ok_or(VoiceError::Reservation)?;
        let aad = object.aad()?;
        let bytes = 2 * (aad.len() + ciphertext.len()) + 256;
        let credit = Arc::new(if let Some(exchange) = &exchange {
            exchange.charge(Stage::RemotePacket, bytes)?
        } else if let Some(workflow) = workflow {
            super::resources::ByteBudget::Workflow(workflow).charge(
                &self.resources,
                self.scope.group,
                bytes,
                ResourceKind::Retained,
            )?
        } else {
            self.resources
                .charge(self.scope.group, bytes, ResourceKind::Retained)?
        });
        let event = InboundPacket {
            token,
            issuer: object.issuer,
            header: header.clone(),
            aad,
            ciphertext: ciphertext.clone(),
            object_hash: object.hash(),
            _custody: credit.clone(),
        };
        let allowance = object
            .expires
            .saturating_add(CLOCK_SKEW_MS)
            .saturating_sub(clock.now_ms())
            .min(MAX_REMOTE_PACKET_REMAINING_MS);
        let retire = clock.monotonic_ms().saturating_add(allowance);
        Ok(PreparedInbound {
            admission: InboundAdmission {
                local: self.lease.clone(),
                pair: pair_lease,
                expires: object.expires,
                deadline: retire.min(self.sdp_deadline(exchange.as_ref())?),
                exchange: exchange.clone(),
                confirmation: None,
                confirmation_required: matches!(
                    header.kind,
                    EncryptedKind::ConfirmHello
                        | EncryptedKind::ConfirmProof
                        | EncryptedKind::ConfirmFinish
                ),
            },
            incoming: Incoming {
                object,
                retire,
                token: RevocationOwner::default(),
                confirmation: None,
                _credit: credit,
                exchange,
                _slot: slot,
            },
            event,
            serial: self.serial,
        })
    }
    /// The callback is the native origin-budget/endpoint-owner transaction. It runs only after
    /// every fallible ledger check and allocation; on success publication is infallible. No
    /// await, reentrant ledger operation or ledger-state mutation is allowed inside the callback.
    /// It must repeat InboundAdmission::check inside its final native authorization transaction,
    /// immediately before spending origin quota; an earlier preparation check is insufficient.
    pub(crate) fn commit_prepared_inbound(
        &mut self,
        prepared: PreparedInbound,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        commit: impl FnOnce(&InboundAdmission) -> Result<(), VoiceError>,
    ) -> Result<InboundPacket, VoiceError> {
        let (publication, event) = self.validate_prepared_inbound(prepared, authority, clock)?;
        commit(publication.admission())?;
        publication.publish();
        Ok(event)
    }
    /// Finish every fallible ledger check before origin allowance or a protected role is
    /// consumed. The returned exclusive loan cannot be interleaved with another ledger call.
    /// Callers must repeat admission.check_at under their final native authorization fence.
    pub(super) fn validate_prepared_inbound(
        &mut self,
        prepared: PreparedInbound,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<(PreparedTokenPublication<'_>, InboundPacket), VoiceError> {
        self.prune(clock);
        let pair = self.inbound_pair(&prepared.incoming.object, authority, clock)?;
        if self.serial != prepared.serial
            || !self.lease.is_same(&prepared.admission.local)
            || !pair.live.is_same(&prepared.admission.pair)
        {
            return Err(VoiceError::Reservation);
        }
        if self.incoming.len() >= RESERVATIONS {
            return Err(VoiceError::Capacity);
        }
        let mut replay = pair.replay.clone();
        replay.admit(prepared.event.header.sequence)?;
        let current_exchange = self.sdp_owner(&prepared.event.header, true)?;
        if !match (&current_exchange, &prepared.incoming.exchange) {
            (None, None) => true,
            (Some(current), Some(prepared)) => Arc::ptr_eq(current, prepared) && prepared.is_live(),
            _ => false,
        } {
            return Err(VoiceError::Unauthorized);
        }
        prepared.admission.check(clock)?;
        let mono = clock.monotonic_ms();
        Ok((
            PreparedTokenPublication {
                token: prepared.event.token,
                ledger: self,
                incoming: prepared.incoming,
                admission: prepared.admission,
                replay,
                mono,
            },
            prepared.event,
        ))
    }
    /// Called in a FRESH actor turn after decrypt. A valid token for peer B cannot authorize A's
    /// tuple. Wrong tuples do not consume B's token; successful delivery is single-use.
    pub fn commit_inbound(
        &mut self,
        token: u64,
        aad: &[u8],
        sequence: u64,
        ciphertext: &[u8],
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<CommittedPacket, VoiceError> {
        let prepared =
            self.validate_inbound_commit(token, aad, sequence, ciphertext, authority, clock)?;
        self.validate_inbound_owner(&prepared, authority, clock)?;
        Ok(self.consume_validated_inbound(prepared))
    }
    pub(super) fn validate_inbound_commit(
        &mut self,
        token: u64,
        aad: &[u8],
        sequence: u64,
        ciphertext: &[u8],
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<ValidatedInbound, VoiceError> {
        self.prune(clock);
        self.current(authority)?;
        let incoming = self.incoming.get(&token).ok_or(VoiceError::Reservation)?;
        if let Some(exchange) = &incoming.exchange {
            if !exchange.is_live()
                || !self
                    .sdp
                    .as_ref()
                    .is_some_and(|sdp| Arc::ptr_eq(exchange, &sdp.owner))
            {
                return Err(VoiceError::Unauthorized);
            }
        }
        incoming.object.check_freshness(clock.now_ms())?;
        let Body::Encrypted {
            header,
            ciphertext: original,
        } = &incoming.object.body
        else {
            unreachable!()
        };
        if aad != incoming.object.aad()? || sequence != header.sequence || ciphertext != original {
            return Err(VoiceError::RetryMismatch);
        }
        // Engine admission is only a pending check. An older engine result arriving after a
        // nonce restart/higher Hello must refuse before consuming its token or mutating state.
        if incoming
            .confirmation
            .as_ref()
            .is_some_and(|state| !state.authorized())
        {
            return Err(VoiceError::Unauthorized);
        }
        let pair = self.pair(&header.handshake, authority)?.live.clone();
        Ok(ValidatedInbound {
            token,
            message: incoming.object.id(),
            header: header.clone(),
            hash: incoming.object.hash(),
            expires: incoming.object.expires(),
            retire: incoming.retire,
            pair,
            token_owner: incoming.token.0.clone(),
            local: self.lease.clone(),
            epoch: authority.context.epoch,
            issued: incoming.object.issued(),
            confirmation: incoming.confirmation.clone(),
            exchange: incoming.exchange.clone(),
        })
    }
    /// Recheck immediately before consumption, after all closed-result validation, RNG draws
    /// and checked counters. No await/ledger mutation may intervene between this and consume.
    pub(super) fn validate_inbound_owner(
        &self,
        prepared: &ValidatedInbound,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<(), VoiceError> {
        let pair = self.pair(&prepared.header.handshake, authority)?;
        let incoming = self
            .incoming
            .get(&prepared.token)
            .ok_or(VoiceError::Reservation)?;
        let wall = clock.now_ms();
        incoming.object.check_freshness(wall)?;
        let monotonic = clock.monotonic_ms();
        // Clock/native seams can observe End while sampling time. Repeat current local and
        // pair ownership AFTER those samples; no further clock or callback may separate this
        // final owner check from the synchronous token-consumption transaction.
        self.current(authority)?;
        if !prepared.pair.is_same(&pair.live)
            || !prepared.pair.is_live()
            || !prepared.token_owner.is_same(&incoming.token.0)
            || !prepared.token_owner.is_live()
            || monotonic >= prepared.retire
            || incoming
                .confirmation
                .as_ref()
                .is_some_and(|pending| !pending.authorized())
            || incoming
                .exchange
                .as_ref()
                .is_some_and(|exchange| !exchange.is_live())
        {
            return Err(VoiceError::Unauthorized);
        }
        prepared.check_at(authority.context.epoch, wall, monotonic)
    }
    pub(super) fn consume_validated_inbound(
        &mut self,
        prepared: ValidatedInbound,
    ) -> CommittedPacket {
        let (packet, remainder) = self.consume_validated_inbound_deferred(prepared);
        // Compatibility callers are outside a native publication fence. Protected confirmation
        // instead keeps this remainder in its outer retirement owner until all guards return.
        drop(remainder);
        packet
    }

    pub(super) fn consume_validated_inbound_deferred(
        &mut self,
        prepared: ValidatedInbound,
    ) -> (CommittedPacket, ConsumedIncoming) {
        let incoming = self
            .incoming
            .remove(&prepared.token)
            .expect("validated synchronous incoming transaction");
        let packet = CommittedPacket {
            object: incoming.object,
            local_lease: self.lease.clone(),
            pair_lease: prepared.pair,
            retire: incoming.retire,
            _credit: incoming._credit,
        };
        let remainder = ConsumedIncoming {
            _token: incoming.token,
            _confirmation: incoming.confirmation,
            _exchange: incoming.exchange,
            _slot: incoming._slot,
        };
        (packet, remainder)
    }
}

impl Drop for PacketLedger {
    fn drop(&mut self) {
        // Counters cannot be reconstructed under the same join/key after allocator loss. Engine
        // charges live in independent native process records until actual settlement.
        self.lease.revoke();
        if let Some(sdp) = &self.sdp {
            sdp.owner.close();
        }
    }
}

#[cfg(test)]
mod tests;
