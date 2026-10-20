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
impl PacketSlots {
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
    pub(super) fn close(&self) {
        self.incoming.close();
        self.outgoing.close();
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
    next: u64,
    replay: Replay,
}
struct Outgoing {
    draft: PacketDraft,
    ciphertext_bytes: usize,
    construction: u64,
    retire: u64,
    source_deadline: u64,
    cached: Option<SignedObject>,
    live: CallLease,
    crypto: RevocationOwner,
    _credit: Credit,
    exchange: Option<Arc<Exchange>>,
    // Native table custody, separate from retained bytes and actual transport copies.
    _slot: slots::Slot,
}
#[derive(Clone)]
pub(super) enum PendingConfirmation {
    Attempt(CallLease),
    Hello(super::crypto_authority::SequenceAuthority),
}
impl PendingConfirmation {
    fn authorized(&self) -> bool {
        match self {
            Self::Attempt(lease) => lease.is_live(),
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
    exchange: Option<Arc<Exchange>>,
    _slot: slots::Slot,
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
                    PendingConfirmation::Attempt(_),
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

/// Private synchronous transaction plan. It must not cross an await: its sole ledger owner
/// prepares closed-result work, repeats final checks, then consumes it without a fallible tail.
pub(super) struct ValidatedInbound {
    token: u64,
    pub header: EncryptedHeader,
    pub hash: [u8; 32],
    pub expires: u64,
    pub retire: u64,
    pair: CallLease,
    token_owner: CallLease,
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
    clock: Arc<dyn Clock>,
    deadline: u64,
    exchange: Option<Arc<Exchange>>,
    expires: u64,
}
impl SourcePacketAuthority {
    pub(super) fn exchange(&self) -> Option<Arc<Exchange>> {
        self.exchange.clone()
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
            && self.deadline == other.deadline
            && self.expires == other.expires
            && match (&self.exchange, &other.exchange) {
                (None, None) => true,
                (Some(first), Some(second)) => Arc::ptr_eq(first, second),
                _ => false,
            }
    }
}
#[async_trait::async_trait]
impl catcoms_rt::RequestAuthority for SourcePacketAuthority {
    fn authorized(&self) -> bool {
        self.local.is_live()
            && self.pair.is_live()
            && self.packet.is_live()
            && self.clock.monotonic_ms() < self.deadline
            && self.clock.now_ms() < self.expires
            && self
                .exchange
                .as_ref()
                .is_none_or(|exchange| exchange.is_live())
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
        if !matches!(header.kind, EncryptedKind::Offer | EncryptedKind::Answer) {
            return Ok(None);
        }
        let sdp = self.sdp.as_ref().ok_or(VoiceError::Capacity)?;
        if sdp.handshake != header.handshake || !sdp.owner.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        if (incoming && sdp.remote.is_some()) || (!incoming && sdp.local.is_some()) {
            return Err(VoiceError::Capacity);
        }
        let permitted = match (incoming, header.kind, sdp.role) {
            (_, EncryptedKind::Offer, SdpRole::Ready) => true,
            (true, EncryptedKind::Answer, SdpRole::Initiator(offer))
            | (false, EncryptedKind::Answer, SdpRole::Responder(offer)) => {
                header.offer == Some(offer)
            }
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
        if !matches!(header.kind, EncryptedKind::Offer | EncryptedKind::Answer) {
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
        if header.kind == EncryptedKind::Offer {
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
        if let Some(pair) = self.pairs.get_mut(handshake) {
            pair.status = Status::Retired;
            pair.live.revoke();
        }
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
        if let Some(sdp) = &self.sdp {
            if mono >= sdp.retire.unwrap_or(sdp.ready_until) || !sdp.owner.is_live() {
                sdp.owner.close();
            }
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
        if DeviceId::from_public_key_bytes(&public_key) != self.local {
            return Err(VoiceError::Unauthorized);
        }
        let pair = self.pair(&request.handshake, authority)?;
        let confirmation = matches!(
            request.kind,
            EncryptedKind::ConfirmHello
                | EncryptedKind::ConfirmProof
                | EncryptedKind::ConfirmFinish
        );
        if confirmation != (pair.status == Status::Confirming) {
            return Err(VoiceError::Unauthorized);
        }
        if self.outgoing.len() >= RESERVATIONS {
            return Err(VoiceError::Capacity);
        }
        let slot = self.packet_slots.outgoing.charge(1)?;
        if request.ciphertext_bytes < 16
            || request.ciphertext_bytes > request.kind.max_plaintext() + 16
        {
            return Err(VoiceError::Bounds);
        }
        let sequence = pair.next.checked_add(1).ok_or(VoiceError::Reservation)?;
        let binding = pair.binding.clone();
        let mut id = [0; 32];
        rng.fill_bytes(&mut id);
        if self.outgoing.contains_key(&id) {
            return Err(VoiceError::Reservation);
        }
        let offer = if request.kind == EncryptedKind::Offer {
            if request.offer.is_some() {
                return Err(VoiceError::Malformed);
            }
            Some(OfferId {
                issuer: self.local,
                message: id,
            })
        } else {
            request.offer
        };
        let header = EncryptedHeader {
            source_join: self.join,
            target: binding.peer,
            target_join: binding.join,
            source_key_ref: self.key_ref,
            target_key_ref: binding.key_ref,
            handshake: binding.handshake,
            low_to_high: self.local < binding.peer,
            kind: request.kind,
            sequence,
            offer,
        };
        let issued = clock.now_ms();
        let expires = issued
            .checked_add(MAX_PACKET_LIFETIME_MS)
            .ok_or(VoiceError::Freshness)?;
        let draft = PacketDraft::new(self.scope, public_key, id, issued, expires, header)?;
        let aad = draft.aad();
        let exchange = self.sdp_owner(&draft.header, false)?;
        // Prepay canonical bytes plus the cache/event copy and bounded record overhead before
        // returning AAD. Native engine permits use this same process pool concurrently.
        let bytes = 2 * (aad.len() + request.ciphertext_bytes) + 256;
        let mono = clock.monotonic_ms();
        let credit = if let Some(exchange) = &exchange {
            exchange.charge(Stage::LocalPacket, bytes)?
        } else {
            self.resources
                .charge(self.scope.group, bytes, ResourceKind::Retained)?
        };
        self.publish_sdp_identity(&draft.header, id, false, mono);
        self.pairs
            .get_mut(&binding.handshake)
            .expect("checked pair")
            .next = sequence;
        self.outgoing.insert(
            id,
            Outgoing {
                draft,
                ciphertext_bytes: request.ciphertext_bytes,
                construction: mono.saturating_add(CONSTRUCTION_MS),
                retire: mono.saturating_add(MAX_PACKET_LIFETIME_MS + CLOCK_SKEW_MS),
                // Source eligibility is fixed at native reservation, independently of later
                // wall corrections and separate clock sample timing on exact admission retries.
                source_deadline: mono.saturating_add(MAX_PACKET_LIFETIME_MS),
                cached: None,
                live: CallLease::default(),
                crypto: RevocationOwner::default(),
                _credit: credit,
                exchange,
                _slot: slot,
            },
        );
        Ok(ReservedPacket {
            id,
            aad,
            sequence,
            expires,
        })
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
        self.prune(clock);
        self.current(authority)?;
        let outgoing = self.outgoing.get(&id).ok_or(VoiceError::Reservation)?;
        if !outgoing.live.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        if outgoing
            .exchange
            .as_ref()
            .is_some_and(|exchange| !exchange.is_live())
        {
            return Err(VoiceError::Unauthorized);
        }
        freshness(
            outgoing.draft.issued,
            outgoing.draft.expires,
            clock.now_ms(),
        )?;
        self.pair(&outgoing.draft.header.handshake, authority)?;
        if signer.voice_public_key()? != outgoing.draft.public_key {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(cached) = &outgoing.cached {
            let Body::Encrypted {
                ciphertext: original,
                ..
            } = &cached.body
            else {
                unreachable!()
            };
            return if ciphertext == original {
                Ok(cached.clone())
            } else {
                Err(VoiceError::RetryMismatch)
            };
        }
        if clock.monotonic_ms() >= outgoing.construction {
            return Err(VoiceError::Reservation);
        }
        if ciphertext.len() != outgoing.ciphertext_bytes {
            return Err(VoiceError::Bounds);
        }
        // Native time/signer seams above may observe End or membership replacement. Recheck
        // irreversible owners after their last sample and immediately before first signing.
        self.pair(&outgoing.draft.header.handshake, authority)?;
        if !outgoing.live.is_live()
            || !outgoing.crypto.0.is_live()
            || outgoing
                .exchange
                .as_ref()
                .is_some_and(|exchange| !exchange.is_live())
        {
            return Err(VoiceError::Unauthorized);
        }
        let signed = outgoing.draft.seal(signer, ciphertext)?;
        let outgoing = self.outgoing.get_mut(&id).expect("checked reservation");
        outgoing.cached = Some(signed.clone());
        // Signing consumes pending encryption eligibility. Source still owns outgoing.live;
        // an exact cached submit retry remains valid without authorizing another engine seal.
        outgoing.crypto.0.revoke();
        Ok(signed)
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
            || outgoing.cached.is_some()
            || !outgoing.live.is_live()
            || reserved.aad != outgoing.draft.aad()
            || reserved.sequence != outgoing.draft.header.sequence
            || reserved.expires != outgoing.draft.expires
        {
            return Err(VoiceError::Unauthorized);
        }
        super::crypto_authority::CryptoAuthority::new(
            self.lease.clone(),
            pair.live.clone(),
            clock,
            outgoing
                .construction
                .min(outgoing.source_deadline)
                .min(self.sdp_deadline(outgoing.exchange.as_ref())?),
            outgoing.draft.expires,
        )?
        .with_pending(outgoing.crypto.0.clone())
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
                PendingConfirmation::Attempt(lease) => grant.with_pending(lease.clone()),
                PendingConfirmation::Hello(gate) => grant.with_hello(gate.clone()),
            }
        } else {
            Ok(grant)
        }
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
            || outgoing.cached.as_ref() != Some(object)
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
            deadline: outgoing
                .source_deadline
                .min(self.sdp_deadline(outgoing.exchange.as_ref())?),
            expires: object.expires(),
            exchange: outgoing.exchange.clone(),
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
        let slot = self.packet_slots.incoming.charge(1)?;
        let token = self.serial.checked_add(1).ok_or(VoiceError::Reservation)?;
        let aad = object.aad()?;
        let exchange = self.sdp_owner(header, true)?;
        let bytes = 2 * (aad.len() + ciphertext.len()) + 256;
        let credit = Arc::new(if let Some(exchange) = &exchange {
            exchange.charge(Stage::RemotePacket, bytes)?
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
            header: header.clone(),
            hash: incoming.object.hash(),
            expires: incoming.object.expires(),
            retire: incoming.retire,
            pair,
            token_owner: incoming.token.0.clone(),
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
        incoming.object.check_freshness(clock.now_ms())?;
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
        Ok(())
    }
    pub(super) fn consume_validated_inbound(
        &mut self,
        prepared: ValidatedInbound,
    ) -> CommittedPacket {
        let incoming = self
            .incoming
            .remove(&prepared.token)
            .expect("validated synchronous incoming transaction");
        CommittedPacket {
            object: incoming.object,
            local_lease: self.lease.clone(),
            pair_lease: prepared.pair,
            retire: incoming.retire,
            _credit: incoming._credit,
        }
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
