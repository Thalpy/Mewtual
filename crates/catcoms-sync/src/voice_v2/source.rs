//! One retained source submission budget per immutable native packet. Helper changes and lost
//! waiters cannot reset it. SourceState -> GroupState -> Policy is the synchronous lock order;
//! no network/engine wait occurs under any of these locks. Endpoint ownership remains separate.
use super::helper::{BoundRoute, RoomAuthority};
use super::resources::{Credit, ResourceKind, Resources};
use super::runtime::{ActorOwner, EffectAuthority};
use super::*;
use catcoms_rt::{Clock, PeerId, RequestAuthority, SharedRequestKeepalive};
use futures::FutureExt;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const PACKETS: usize = 256;
const CURSORS: usize = 64;
const ATTEMPTS: u8 = 3;
/// Eligibility delay only; shared byte/flight capacity and actual lower custody may delay it.
const HEDGE_MS: u64 = 500;
type Key = (DeviceId, MessageId);
mod activity;
use activity::PacketActivity;

/// Borrowed capacity provenance from one immutable native packet. This is deliberately
/// narrower than ByteBudget: public ingress cannot supply a reservation or select a Workflow.
/// Root's retained ACK view carries this together with the SAME packet and cached hash.
#[derive(Clone, Copy)]
pub(crate) enum SourceProgress<'a> {
    Ordinary,
    Workflow(&'a Arc<super::chain::Workflow>),
    Exchange(&'a Arc<super::negotiation::Exchange>),
    // Restricted HIGH first-Offer verification only; never an Active SDP action lender.
    ReadyReceive(&'a Arc<super::negotiation::Exchange>),
    // Exact LOW Ready retry verification after its original Workflow lane was moved.
    LowReadyRetry(&'a Arc<super::negotiation::Exchange>),
    Ready(&'a super::ready::source_authority::ReadySourceAuthority),
}
impl fmt::Debug for SourceProgress<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SourceProgress { .. }")
    }
}
impl SourceProgress<'_> {
    /// Only Ready adds another original time source to the existing capacity lookup.
    /// Runtime samples it before Group/Policy and supplies the frozen sample below.
    pub(crate) fn observation_time(self) -> Option<(u64, u64)> {
        match self {
            Self::Ready(ready) => Some(ready.observation_time()),
            _ => None,
        }
    }
    pub(crate) fn charge(
        self,
        resources: &Arc<Resources>,
        group: [u8; 32],
        bytes: usize,
        stage: super::negotiation::Stage,
    ) -> Result<Credit, VoiceError> {
        use super::negotiation::Stage;
        if !matches!(stage, Stage::Crypto | Stage::Source | Stage::Lower) {
            return Err(VoiceError::Scope);
        }
        match self {
            Self::Ordinary => resources.charge(group, bytes, stage.resource_kind()),
            Self::Workflow(workflow) => {
                workflow.charge(resources, group, bytes, stage.resource_kind())
            }
            Self::Exchange(exchange) => {
                if !exchange.uses_resources(resources, group) {
                    return Err(VoiceError::Scope);
                }
                exchange.charge(stage, bytes)
            }
            Self::ReadyReceive(exchange) => {
                if stage != Stage::Crypto || !exchange.uses_resources(resources, group) {
                    return Err(VoiceError::Scope);
                }
                exchange.charge_ready_offer_verification(bytes)
            }
            Self::Ready(ready) => ready.charge(resources, group, stage, bytes),
            Self::LowReadyRetry(exchange) => {
                if stage != Stage::Crypto || !exchange.uses_resources(resources, group) {
                    return Err(VoiceError::Scope);
                }
                exchange.charge_low_ready_retry_verification(bytes)
            }
        }
    }
    /// Lookup only COMPARES the actual supplied owner. It never constructs packet authority
    /// or replaces a refused/retired owner with current ordinary or another phase's capacity.
    pub(crate) fn require_current(
        self,
        resources: &Arc<Resources>,
        group: [u8; 32],
        sampled: Option<(u64, u64)>,
    ) -> Result<(), VoiceError> {
        let current = match self {
            Self::Ordinary => true,
            Self::Workflow(expected) => {
                expected.uses_resources(resources, group)
                    && expected.is_live()
                    && resources
                        .workflow(group)
                        .as_ref()
                        .is_some_and(|actual| Arc::ptr_eq(actual, expected))
            }
            Self::Exchange(expected) => {
                expected.uses_resources(resources, group)
                    && expected.is_live()
                    && resources
                        .exchange(group)
                        .as_ref()
                        .is_some_and(|actual| Arc::ptr_eq(actual, expected))
            }
            Self::ReadyReceive(expected) => {
                expected.uses_resources(resources, group)
                    && expected.ready_receive_open()
                    && resources
                        .exchange(group)
                        .as_ref()
                        .is_some_and(|actual| Arc::ptr_eq(actual, expected))
            }
            Self::Ready(ready) => {
                let (wall, mono) = sampled.ok_or(VoiceError::Unauthorized)?;
                return ready.require_current_at(resources, group, wall, mono);
            }
            Self::LowReadyRetry(expected) => {
                expected.uses_resources(resources, group)
                    && expected.low_ready_retry_open()
                    && resources
                        .exchange(group)
                        .as_ref()
                        .is_some_and(|actual| Arc::ptr_eq(actual, expected))
            }
        };
        if current {
            Ok(())
        } else {
            Err(VoiceError::Unauthorized)
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Incarnation {
    Joined(JoinKey),
    Watching(Nonce),
}
#[derive(Clone)]
enum ActivityLease {
    Joined(CallLease),
    Watching(WatchLease),
}
impl ActivityLease {
    fn is_live(&self) -> bool {
        match self {
            Self::Joined(lease) => lease.is_live(),
            Self::Watching(lease) => lease.is_live(),
        }
    }
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Joined(first), Self::Joined(second)) => first.is_same(second),
            (Self::Watching(first), Self::Watching(second)) => first.is_same(second),
            _ => false,
        }
    }
    async fn revoked(&self) {
        match self {
            Self::Joined(lease) => lease.wait_revoked().await,
            Self::Watching(lease) => lease.wait_revoked().await,
        }
    }
}
/// Issued only by the actual LocalJoin/Watcher, after its exact authority check. A live lease
/// alone cannot authorize first admission of an old, previously unseen join's signed packet.
#[derive(Clone)]
pub struct SourceActivity {
    scope: Scope,
    local: DeviceId,
    lease: ActivityLease,
    incarnation: Incarnation,
    packet: Option<PacketActivity>,
}
impl SourceActivity {
    pub(super) fn ready(&self) -> Option<&super::ready::source_authority::ReadySourceAuthority> {
        match &self.packet {
            Some(PacketActivity::Ready(ready)) => Some(ready),
            _ => None,
        }
    }
    /// Pure original Ready check under Group/Policy; its clock MUST be sampled beforehand.
    pub(super) fn check_ready_at(
        &self,
        object: &SignedObject,
        wall: u64,
        mono: u64,
    ) -> Result<(), VoiceError> {
        let Some(ready) = self.ready() else {
            return Err(VoiceError::Unauthorized);
        };
        let (
            Incarnation::Joined(key),
            ActivityLease::Joined(call),
            Body::PairReadyRound {
                sequence: 1,
                header,
                profile: super::SdpProfile::CompleteSdpV1,
            },
        ) = (&self.incarnation, &self.lease, object.body())
        else {
            return Err(VoiceError::Unauthorized);
        };
        if self.scope != object.scope()
            || self.local != object.issuer()
            || !call.is_live()
            || key.join != header.source_join
            || key.key_ref != header.source_key_ref
            || !ready.is_call(call)
            || !ready.check_at(object, wall, mono)
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    fn progress(&self) -> SourceProgress<'_> {
        self.packet
            .as_ref()
            .map_or(SourceProgress::Ordinary, PacketActivity::progress)
    }
    fn exchange(&self) -> Option<Arc<super::negotiation::Exchange>> {
        self.packet.as_ref().and_then(PacketActivity::exchange)
    }
    pub(super) fn joined(scope: Scope, local: DeviceId, key: JoinKey, lease: CallLease) -> Self {
        Self {
            scope,
            local,
            lease: ActivityLease::Joined(lease),
            incarnation: Incarnation::Joined(key),
            packet: None,
        }
    }
    pub(super) fn watching(scope: Scope, local: DeviceId, nonce: Nonce, lease: WatchLease) -> Self {
        Self {
            scope,
            local,
            lease: ActivityLease::Watching(lease),
            incarnation: Incarnation::Watching(nonce),
            packet: None,
        }
    }
    fn same(&self, other: &Self) -> bool {
        self.scope == other.scope
            && self.local == other.local
            && self.lease.same(&other.lease)
            && self.incarnation == other.incarnation
            && match (&self.packet, &other.packet) {
                (None, None) => true,
                (Some(first), Some(second)) => first.same(second),
                _ => false,
            }
    }
    pub(super) fn with_packet(mut self, packet: super::packets::SourcePacketAuthority) -> Self {
        self.packet = Some(PacketActivity::Encrypted(packet));
        self
    }
    pub(in crate::voice_v2) fn with_ready(
        mut self,
        ready: super::ready::source_authority::ReadySourceAuthority,
    ) -> Self {
        self.packet = Some(PacketActivity::Ready(ready));
        self
    }
    fn is_live(&self) -> bool {
        self.lease.is_live() && self.packet.as_ref().is_none_or(PacketActivity::authorized)
    }
    fn observation_time(&self) -> Option<(u64, u64)> {
        self.packet.as_ref().map(|packet| packet.observation_time())
    }
    fn obsolete_confirmation(&self) -> bool {
        self.packet
            .as_ref()
            .is_some_and(PacketActivity::obsolete_confirmation)
    }
    fn is_live_at(&self, packet_time: Option<(u64, u64)>) -> bool {
        self.lease.is_live()
            && match (&self.packet, packet_time) {
                (None, None) => true,
                (Some(packet), Some((wall, mono))) => packet.authorized_at(wall, mono),
                _ => false,
            }
    }
    async fn revoked(&self) {
        if let Some(packet) = &self.packet {
            futures::future::select(self.lease.revoked().boxed(), packet.revoked().boxed()).await;
        } else {
            self.lease.revoked().await;
        }
    }
    pub(super) fn check(&self, object: &SignedObject) -> Result<(), VoiceError> {
        let matches = match (&self.incarnation, object.body()) {
            (Incarnation::Joined(key), Body::Announcement { key: issued, .. }) => {
                self.packet.is_none() && key == issued
            }
            (Incarnation::Joined(key), Body::JoinProof { join, key_ref, .. }) => {
                self.packet.is_none() && key.join == *join && key.key_ref == *key_ref
            }
            (Incarnation::Joined(key), Body::Encrypted { header, .. }) => {
                key.join == header.source_join
                    && key.key_ref == header.source_key_ref
                    && self
                        .packet
                        .as_ref()
                        .is_some_and(|packet| match &self.lease {
                            ActivityLease::Joined(lease) => packet.check(object, lease),
                            ActivityLease::Watching(_) => false,
                        })
            }
            (Incarnation::Joined(_), Body::PairReady { .. } | Body::PairReadyRound { .. }) => {
                let Some(ready) = self.ready() else {
                    return Err(VoiceError::Unauthorized);
                };
                let (wall, mono) = ready.observation_time();
                self.check_ready_at(object, wall, mono).is_ok()
            }
            (Incarnation::Watching(nonce), Body::WatchChallenge { watcher, .. }) => {
                nonce == watcher
            }
            _ => false,
        };
        if !matches
            || !self.is_live()
            || self.scope != object.scope()
            || self.local != object.issuer()
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SourceHop {
    Direct(DeviceId),
    Helper(DeviceId),
}
impl SourceHop {
    pub fn device(self) -> DeviceId {
        match self {
            Self::Direct(id) | Self::Helper(id) => id,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceOutcome {
    Pending,
    Unconfirmed,
    EndpointAdmitted,
    Retired,
}
struct PacketState {
    attempts: u8,
    selected: u8,
    next: u64,
    helpers: BTreeSet<DeviceId>,
    direct: BTreeSet<DeviceId>,
    confirmed: bool,
}
struct Packet {
    object: SignedObject,
    hash: [u8; 32],
    bytes: usize,
    activity: SourceActivity,
    room: RoomAuthority,
    authority: Arc<EffectAuthority>,
    deadline: u64,
    state: Mutex<PacketState>,
    active: AtomicUsize,
    _credit: Credit,
}
struct Cursor {
    direct: Option<DeviceId>,
    helper: Option<DeviceId>,
    expires: u64,
    deadline: u64,
    _credit: Credit,
}
#[derive(Default)]
struct State {
    packets: BTreeMap<Key, Arc<Packet>>,
    cursors: BTreeMap<(DeviceId, u128), Cursor>,
    work: Option<Key>,
}
pub struct SourceLedger {
    group: [u8; 32],
    clock: Arc<dyn Clock>,
    resources: Arc<Resources>,
    state: Mutex<State>,
}
struct FlightAuthority {
    hop: Arc<EffectAuthority>,
    packet: Arc<Packet>,
    clock: Arc<dyn Clock>,
    deadline: u64,
}
#[async_trait::async_trait]
impl RequestAuthority for FlightAuthority {
    fn authorized(&self) -> bool {
        // Sample EVERY original clock before checking ANY irreversible owner. Production
        // currently shares Runtime time, but native APIs also accept independent clocks;
        // using the job's monotonic origin for the packet could revive an expired old packet.
        let wall = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        let (hop_wall, hop_mono) = self.hop.observation_time();
        let (packet_wall, packet_mono) = self.packet.authority.observation_time();
        let activity_time = self.packet.activity.observation_time();
        self.hop.authorized_at(hop_wall, hop_mono)
            && self
                .packet
                .authority
                .authorized_at(packet_wall, packet_mono)
            && self.packet.activity.is_live_at(activity_time)
            && self.packet.room.lease.is_live()
            && wall < self.packet.object.expires()
            && mono < self.deadline
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let waits = vec![
            self.hop.revoked().boxed(),
            self.packet.authority.revoked().boxed(),
            self.packet.activity.revoked().boxed(),
            self.packet.room.lease.wait_revoked().boxed(),
            self.clock.sleep(std::time::Duration::from_millis(
                self.deadline.saturating_sub(self.clock.monotonic_ms()),
            )),
        ];
        futures::future::select_all(waits).await;
    }
}
struct Flight {
    packet: Arc<Packet>,
    attempt: u8,
    hop: SourceHop,
    authority: Arc<FlightAuthority>,
    _credit: Credit,
}
impl Drop for Flight {
    fn drop(&mut self) {
        self.packet.active.fetch_sub(1, Ordering::AcqRel);
    }
}
/// One reserved submission. Signing/framing and actual lower connection ownership belong to
/// the sole Root. Once this exists, later preparation/drop conservatively burns the attempt.
pub struct SourceJob {
    key: Key,
    peer: PeerId,
    flight: Arc<Flight>,
}
pub struct SourceCompletion {
    key: Key,
    flight: Arc<Flight>,
    status: Option<crate::VoiceTransportStatus>,
}
redacted_debug!(
    SourceActivity,
    SourceHop,
    SourceLedger,
    Packet,
    FlightAuthority,
    Flight,
    SourceJob,
    SourceCompletion
);

impl SourceJob {
    pub(crate) fn progress(&self) -> SourceProgress<'_> {
        self.flight.packet.activity.progress()
    }
    pub fn peer(&self) -> PeerId {
        self.peer
    }
    pub fn hop(&self) -> SourceHop {
        self.flight.hop
    }
    pub fn object(&self) -> &SignedObject {
        &self.flight.packet.object
    }
    pub fn object_hash(&self) -> [u8; 32] {
        self.flight.packet.hash
    }
    pub fn authority(&self) -> Arc<dyn RequestAuthority> {
        self.flight.authority.clone()
    }
    pub fn keepalive(&self) -> SharedRequestKeepalive {
        self.flight.clone()
    }
    pub fn finish(self, status: Option<crate::VoiceTransportStatus>) -> SourceCompletion {
        SourceCompletion {
            key: self.key,
            flight: self.flight,
            status,
        }
    }
}
impl SourceLedger {
    /// Permanently retire this Root's old endpoint writers at admitted-profile
    /// selection. Watching/WatchChallenge and remote helper forwarding stay separate.
    /// Snapshot only ONE original lease per pass, then revoke outside SourceState:
    /// watch wakeups/destructors must not run under Source/Group/Policy locks. The
    /// fixed packet cap bounds the pass; no unfunded batch or replacement registry.
    /// Existing FlightAuthority observes the SAME lease and cancels its original
    /// lower work. Actual flights/transport keep their charges until real return.
    pub(crate) fn retire_existing_endpoint_sources(&self) {
        for _ in 0..PACKETS {
            let next = {
                let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                state.packets.values().find_map(|packet| {
                    if !matches!(
                        packet.object.body(),
                        Body::Announcement { .. }
                            | Body::JoinProof { .. }
                            | Body::Encrypted { .. }
                            | Body::PairReady { .. }
                            | Body::PairReadyRound { .. }
                    ) {
                        return None;
                    }
                    match &packet.activity.lease {
                        ActivityLease::Joined(call) if call.is_live() => Some(call.clone()),
                        _ => None,
                    }
                })
            };
            let Some(call) = next else { break };
            call.revoke();
        }
    }
    pub(super) fn new(
        group: [u8; 32],
        clock: Arc<dyn Clock>,
        resources: Arc<Resources>,
    ) -> Arc<Self> {
        Arc::new(Self {
            group,
            clock,
            resources,
            state: Mutex::new(State::default()),
        })
    }
    fn current(&self, actor: &ActorOwner) -> Result<(), VoiceError> {
        if !std::ptr::eq(Arc::as_ptr(&actor.source()?), self) {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    pub(super) fn collect_expired(&self) {
        self.prune(&mut self.state.lock().unwrap_or_else(|e| e.into_inner()));
    }
    fn prune(&self, state: &mut State) {
        let now = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        state.packets.retain(|_, packet| {
            // SourceState also fences new Flight publication. Actual Lower owners always
            // retain custody; a permanently ended native join/watch needs no idle retry
            // record until signed expiry. Otherwise preserve the original call's attempts;
            // generic room/pair/authority loss is not permission to recreate a retry record.
            // Extra terminal cases are HIGH's native Finish-open plus old-plan closure,
            // LOW's exact authenticated native Ready, and explicit whole-plan closure.
            // Key-only successor closure remains distinct. LOW local Finish/Established
            // alone is insufficient. Fixed original phase history prevents re-admission, while
            // actual flights/Lower/ACK owners remain charged by the first branch below.
            // Map removal is not drainage proof: external activities/authorities may still
            // retain the original Workflow/Packet and its actual credit after this removal.
            packet.active.load(Ordering::Acquire) != 0
                || (packet.activity.lease.is_live()
                    && !packet.activity.obsolete_confirmation()
                    // Ready may retire only after irreversible native completion/closure or
                    // strict expiry. Exhausted-but-live records keep their spent allowance.
                    && (packet.activity.ready().is_none() || packet.activity.is_live())
                    && (now <= packet.object.expires().saturating_add(CLOCK_SKEW_MS)
                        || mono < packet.deadline))
        });
        state.cursors.retain(|_, cursor| {
            now <= cursor.expires.saturating_add(CLOCK_SKEW_MS) || mono < cursor.deadline
        });
    }
    /// Classify immutable retry before original quota. Reject stale actor/activity before cached
    /// return too; a replacement native activity never adopts the earlier owner's packet.
    pub fn admit(
        &self,
        actor: &ActorOwner,
        object: SignedObject,
        activity: SourceActivity,
        room: RoomAuthority,
    ) -> Result<bool, VoiceError> {
        self.admit_packet(actor, Cow::Owned(object), activity, room)
    }
    /// Native cache-to-Source transfer. The caller borrows its actual published packet;
    /// Source pays canonical scratch and its retained copy BEFORE cloning any payload.
    /// An exact cached retry does not clone, re-sign, renew its deadline or reset attempts.
    /// This is not public ingress: the original LocalJoin/Watcher activity is still required.
    pub(crate) fn admit_borrowed(
        &self,
        actor: &ActorOwner,
        object: &SignedObject,
        activity: SourceActivity,
        room: RoomAuthority,
    ) -> Result<bool, VoiceError> {
        self.admit_packet(actor, Cow::Borrowed(object), activity, room)
    }
    fn admit_packet(
        &self,
        actor: &ActorOwner,
        object: Cow<'_, SignedObject>,
        activity: SourceActivity,
        room: RoomAuthority,
    ) -> Result<bool, VoiceError> {
        self.admit_packet_before_final(actor, object, activity, room, || {})
    }
    /// Deterministic seam immediately before the final Source origin transaction. Production
    /// supplies no callback. Tests can arm a Clock hook only after every preflight/payment,
    /// so an earlier rejection cannot conceal a missing final currency check.
    #[cfg(test)]
    pub(crate) fn admit_borrowed_before_final_for_test(
        &self,
        actor: &ActorOwner,
        object: &SignedObject,
        activity: SourceActivity,
        room: RoomAuthority,
        before_final: impl FnOnce(),
    ) -> Result<bool, VoiceError> {
        self.admit_packet_before_final(actor, Cow::Borrowed(object), activity, room, before_final)
    }
    fn admit_packet_before_final(
        &self,
        actor: &ActorOwner,
        object: Cow<'_, SignedObject>,
        activity: SourceActivity,
        room: RoomAuthority,
        before_final: impl FnOnce(),
    ) -> Result<bool, VoiceError> {
        self.current(actor)?;
        actor.authorize_source(&object, &activity, &room, false)?;
        let key = (object.issuer(), object.id());
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.prune(&mut state);
        if let Some(packet) = state.packets.get(&key) {
            // Both objects are immutable verified native values. Structural equality checks
            // the exact signed bytes without allocating another pair of canonical encodings.
            if packet.object != *object {
                return Err(VoiceError::RetryMismatch);
            }
            if !packet.activity.same(&activity)
                || !packet.authority.authorized()
                || !packet.room.lease.is_same(&room.lease)
            {
                return Err(VoiceError::Unauthorized);
            }
            return Ok(true);
        }
        if state.packets.len() >= PACKETS {
            return Err(VoiceError::Capacity);
        }
        let body_bytes = match object.body() {
            Body::Encrypted { ciphertext, .. } => ciphertext.len(),
            _ => 0,
        };
        // Native PacketLedger already owns cache/event bytes. Reserve additional canonical
        // scratch before hash/encode; a cached exact retry above needs neither allocation.
        let scratch_bytes = if activity.ready().is_some() {
            3 * super::ready::capacity::MAX_WIRE + 4096
        } else {
            2 * (body_bytes + 1024) + 4096
        };
        let progress = activity.progress();
        let _scratch = progress.charge(
            &self.resources,
            self.group,
            scratch_bytes,
            super::negotiation::Stage::Crypto,
        )?;
        let hash = activity
            .ready()
            .map_or_else(|| object.hash(), |ready| ready.hash());
        let now = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        let remaining = object
            .expires()
            .saturating_add(CLOCK_SKEW_MS)
            .saturating_sub(now);
        if remaining == 0 || remaining > MAX_REMOTE_DELEGATION_REMAINING_MS {
            return Err(VoiceError::Freshness);
        }
        let ready_remaining = activity.ready().map(|ready| {
            let (_, native_mono) = ready.observation_time();
            ready.deadline().saturating_sub(native_mono)
        });
        // Ready was stamped in its ORIGINAL native clock. Convert only remaining duration
        // into this ledger's domain; absolute monotonic origins need not be equal.
        let deadline =
            mono.saturating_add(ready_remaining.map_or(remaining, |left| left.min(remaining)));
        let bytes = object.encode().len() + 512;
        let source_metadata = if activity.ready().is_some() {
            super::ready::capacity::SOURCE_METADATA
        } else {
            1024
        };
        let credit = progress.charge(
            &self.resources,
            self.group,
            bytes + source_metadata,
            super::negotiation::Stage::Source,
        )?;
        let cursor_key = (object.issuer(), object.scope().room);
        let cursor_credit = if matches!(object.body(), Body::Announcement { .. })
            && !state.cursors.contains_key(&cursor_key)
        {
            if state.cursors.len() >= CURSORS {
                return Err(VoiceError::Capacity);
            }
            Some(
                self.resources
                    .charge(self.group, 512, ResourceKind::Retained)?,
            )
        } else {
            None
        };
        let authority = actor.effect(
            object.issuer(),
            object.body().target().unwrap_or(object.issuer()),
            object.expires(),
        )?;
        // Owned callers already paid their input copy at their boundary. The native pump
        // borrows its original cache, so only this paid point creates Source's own copy.
        // Allocation remains before the final origin debit and before infallible publication.
        let object = object.into_owned();
        // All fallible persistent reservations precede the original-issuer transaction. Every
        // subsequent map publication is infallible; no rejected packet spends origin quota.
        before_final();
        actor.authorize_source(&object, &activity, &room, true)?;
        if let Some(credit) = cursor_credit {
            state.cursors.insert(
                cursor_key,
                Cursor {
                    direct: None,
                    helper: None,
                    expires: object.expires(),
                    deadline,
                    _credit: credit,
                },
            );
        } else if let Some(cursor) = state.cursors.get_mut(&cursor_key) {
            cursor.expires = cursor.expires.max(object.expires());
            cursor.deadline = cursor.deadline.max(deadline);
        }
        state.packets.insert(
            key,
            Arc::new(Packet {
                object,
                hash,
                bytes,
                activity,
                room,
                authority,
                deadline,
                state: Mutex::new(PacketState {
                    attempts: 0,
                    selected: 0,
                    next: mono,
                    helpers: BTreeSet::new(),
                    direct: BTreeSet::new(),
                    confirmed: false,
                }),
                active: AtomicUsize::new(0),
                _credit: credit,
            }),
        );
        Ok(false)
    }
    /// Fresh proven connected routes only. A 500ms hedge is eligible subject to actual shared
    /// capacity; absent route/capacity does not spend an attempt. Older lower flights may remain
    /// owned while a newer attempt is selected, and each keeps its own independently charged flag.
    pub fn take_ready(
        &self,
        actor: &ActorOwner,
        routes: &[BoundRoute],
    ) -> Result<Vec<SourceJob>, VoiceError> {
        self.take_ready_bounded(actor, routes, 4)
    }

    /// Root supplies its presently free route-registry positions before an attempt is spent.
    /// This is synchronous admission under Root's exclusive borrow, not a refund mechanism.
    pub(crate) fn take_ready_bounded(
        &self,
        actor: &ActorOwner,
        routes: &[BoundRoute],
        limit: usize,
    ) -> Result<Vec<SourceJob>, VoiceError> {
        self.take_ready_using(actor, routes, limit, |origin, target, expires| {
            actor.effect(origin, target, expires)
        })
    }

    // Keep effect preparation synchronous and native-owned. This narrow dependency seam lets
    // deterministic tests place a retryable refusal after an earlier real job was prepared;
    // production always delegates to the original actor above, never caller-supplied authority.
    fn take_ready_using(
        &self,
        actor: &ActorOwner,
        routes: &[BoundRoute],
        limit: usize,
        mut prepare_effect: impl FnMut(
            DeviceId,
            DeviceId,
            u64,
        ) -> Result<Arc<EffectAuthority>, VoiceError>,
    ) -> Result<Vec<SourceJob>, VoiceError> {
        self.current(actor)?;
        let limit = limit.min(4);
        if limit == 0 {
            return Ok(Vec::new());
        }
        if routes.len() > catcoms_rt::MAX_CONNECTED_PEER_SNAPSHOT {
            return Err(VoiceError::Bounds);
        }
        let mut by_device = BTreeMap::new();
        let mut peers = BTreeSet::new();
        for route in routes {
            if by_device.insert(route.device, route.peer).is_some() || !peers.insert(route.peer) {
                return Err(VoiceError::Malformed);
            }
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.prune(&mut state);
        let mut keys: Vec<_> = state.packets.keys().copied().collect();
        if let Some(cursor) = state.work {
            keys.sort_by_key(|key| (*key <= cursor, *key));
        }
        let mut jobs = Vec::new();
        for key in keys {
            let packet = state.packets[&key].clone();
            let mut status = packet.state.lock().unwrap_or_else(|e| e.into_inner());
            if status.attempts >= ATTEMPTS
                || status.confirmed
                || self.clock.monotonic_ms() < status.next
                || !packet.authority.authorized()
                || !packet.room.lease.is_live()
                || packet.activity.check(&packet.object).is_err()
                || self.clock.now_ms() >= packet.object.expires()
            {
                continue;
            }
            let context = match actor.source_context(packet.object.scope()) {
                Ok(context) => context,
                // A later snapshot may encounter native contention after an earlier job spent
                // its attempt. Keep that job and leave this packet untouched for quiet retry.
                Err(VoiceError::Capacity) => continue,
                Err(error) => return Err(error),
            };
            let cursor_key = (key.0, packet.object.scope().room);
            let cursor = state.cursors.get(&cursor_key);
            let mut direct: Vec<_> = by_device
                .keys()
                .copied()
                .filter(|id| {
                    *id != key.0
                        && packet
                            .object
                            .body()
                            .target()
                            .is_none_or(|target| *id == target)
                })
                .collect();
            direct.sort_by_key(|id| {
                (
                    cursor
                        .and_then(|cursor| cursor.direct)
                        .is_some_and(|last| *id <= last),
                    *id,
                )
            });
            let mut helpers: Vec<_> = by_device
                .keys()
                .copied()
                .filter(|id| {
                    *id != key.0
                        && Some(*id) != packet.object.body().target()
                        && packet
                            .activity
                            .exchange()
                            .is_none_or(|exchange| exchange.allows_helper(*id))
                })
                .collect();
            helpers.sort_by_key(|id| {
                (
                    status.helpers.contains(id),
                    cursor
                        .and_then(|cursor| cursor.helper)
                        .is_some_and(|last| *id <= last),
                    *id,
                )
            });
            let first_direct = direct
                .iter()
                .copied()
                .find(|id| !status.direct.contains(id));
            let helper = if context.indirect && context.scope.policy.is_some() {
                helpers
                    .into_iter()
                    .find(|id| status.helpers.contains(id) || status.helpers.len() < 2)
            } else {
                None
            };
            let hop = if let Some(first) = first_direct
                .filter(|_| status.attempts == 0 || packet.object.body().target().is_some())
            {
                SourceHop::Direct(first)
            } else if let Some(helper) = helper {
                SourceHop::Helper(helper)
            } else if let Some(direct) = first_direct.or_else(|| direct.first().copied()) {
                SourceHop::Direct(direct)
            } else {
                continue;
            };
            let effect = match prepare_effect(key.0, hop.device(), packet.object.expires()) {
                Ok(effect) => effect,
                // A later busy Group/Policy must not drop earlier unsubmitted jobs after their
                // attempts were spent. This packet stays untouched; return the earlier jobs.
                Err(VoiceError::Capacity) => continue,
                Err(error) => return Err(error),
            };
            // A successful network submission can return an ACK while the ordinary pool is
            // full. Prepare its immediate responder's normal rate/origin row BEFORE spending
            // the attempt; a previously unseen helper must not strand an admitted completion
            // on a later bookkeeping allocation. This grants/resets no traffic allowance.
            let stamp = self.clock.monotonic_ms();
            match actor.with_limits(|limits| {
                limits.prepare_exchange_members(&[hop.device()], &self.resources, stamp)
            }) {
                Ok(()) => {}
                Err(VoiceError::Capacity) => continue,
                Err(error) => return Err(error),
            }
            // Root framing and route authority share this same affine lower credit. Reserving
            // route metadata here avoids a fallible ordinary allocation after attempt debit.
            // Original ciphertext remains in Packet's independently paid Source record.
            // Root borrows that object, and its exact-capacity encoders retain at most two
            // large buffers (inner+transcript, inner+frame, or Root+driver frame). The private
            // retained ACK view reuses this SAME packet's cached hash, so its small verifier
            // cannot hide two additional SDP encodings. Fixed wrapper/permit/route payments
            // remain unchanged, and the actual driver still retains this whole credit.
            let bytes = 2 * packet.bytes + 4096 + ROUTE_METADATA_BYTES;
            let budget = packet.activity.progress().charge(
                &self.resources,
                self.group,
                bytes,
                super::negotiation::Stage::Lower,
            );
            let credit = match budget {
                Ok(credit) => credit,
                Err(VoiceError::Capacity) => continue,
                Err(error) => return Err(error),
            };
            let now = self.clock.monotonic_ms();
            let wall = self.clock.now_ms();
            let authority = Arc::new(FlightAuthority {
                hop: effect,
                packet: packet.clone(),
                clock: self.clock.clone(),
                deadline: now.saturating_add(packet.object.expires().saturating_sub(wall)),
            });
            if !authority.authorized() {
                continue;
            }
            status.attempts += 1;
            status.selected = status.attempts;
            status.next = now.saturating_add(HEDGE_MS);
            match hop {
                SourceHop::Direct(id) => {
                    status.direct.insert(id);
                    if let Some(cursor) = state.cursors.get_mut(&cursor_key) {
                        cursor.direct = Some(id);
                    }
                }
                SourceHop::Helper(id) => {
                    status.helpers.insert(id);
                    if let Some(cursor) = state.cursors.get_mut(&cursor_key) {
                        cursor.helper = Some(id);
                    }
                }
            }
            let attempt = status.attempts;
            packet.active.fetch_add(1, Ordering::AcqRel);
            drop(status);
            jobs.push(SourceJob {
                key,
                peer: by_device[&hop.device()],
                flight: Arc::new(Flight {
                    packet,
                    attempt,
                    hop,
                    authority,
                    _credit: credit,
                }),
            });
            state.work = Some(key);
            if jobs.len() >= limit {
                break;
            }
        }
        Ok(jobs)
    }
    /// A queued-helper ACK never settles final receipt. An authenticated positive response from
    /// the exact still-current target is monotonic evidence even after a hedge was selected.
    /// Older negative/helper completions cannot overwrite newer progress. Actual lower-flight
    /// retirement remains independent of this transition.
    pub fn complete(&self, completed: SourceCompletion) -> Result<SourceOutcome, VoiceError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let packet = state
            .packets
            .get(&completed.key)
            .ok_or(VoiceError::Reservation)?;
        if !Arc::ptr_eq(packet, &completed.flight.packet) {
            return Err(VoiceError::Reservation);
        }
        let mut status = packet.state.lock().unwrap_or_else(|e| e.into_inner());
        if !completed.flight.authority.authorized() {
            return Ok(SourceOutcome::Retired);
        }
        if completed.status == Some(crate::VoiceTransportStatus::EndpointAdmitted) {
            let SourceHop::Direct(recipient) = completed.flight.hop else {
                return Err(VoiceError::Unauthorized);
            };
            if let Some(target) = packet.object.body().target() {
                if recipient != target {
                    return Err(VoiceError::Unauthorized);
                }
                if packet.activity.ready().is_none() {
                    status.confirmed = true;
                    return Ok(SourceOutcome::EndpointAdmitted);
                }
                // A Ready ACK is delivery metadata only. LOW may still have lost its native
                // reply or not reached Offer preparation; retain the SAME retry allowance.
            }
            // A room broadcast's direct ACK proves this one recipient only. It never prevents
            // remaining source submissions or claims whole-room convergence.
        }
        if status.selected != completed.flight.attempt {
            return Err(VoiceError::Reservation);
        }
        if completed.status == Some(crate::VoiceTransportStatus::HelperQueued)
            && !matches!(completed.flight.hop, SourceHop::Helper(_))
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(if status.confirmed {
            SourceOutcome::EndpointAdmitted
        } else if status.attempts >= ATTEMPTS {
            SourceOutcome::Unconfirmed
        } else {
            SourceOutcome::Pending
        })
    }
    /// Cheap scheduling hint: whether any packet record is retained at all. It neither prunes
    /// nor locks per-packet state, so the per-tick scheduler can call it on every actor turn.
    /// Retained terminal records count until a pruning pass (take_ready/counts) removes them.
    pub(crate) fn is_empty(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .packets
            .is_empty()
    }
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.prune(&mut state);
        (
            state.packets.len(),
            state
                .packets
                .values()
                .map(|packet| packet.state.lock().unwrap().attempts as usize)
                .sum(),
            state
                .packets
                .values()
                .map(|packet| packet.active.load(Ordering::Acquire))
                .sum(),
        )
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod ready_layout {
    use super::*;
    #[test]
    fn ready_source_complete_retained_graph_fits_its_original_stage_payment() {
        use std::mem::size_of;
        // Packet includes SourceActivity/inline authority, RoomAuthority and both Set
        // wrappers ONCE. Their paid backing owners are retained, never reallocated here.
        // Source map has Arc values;1024 covers its complete small leaf (11 Key/Arc slots),
        // and512 each covers the one-direct/two-helper set leaves at their full capacity.
        let graph = size_of::<Packet>()
            + 2 * size_of::<usize>()
            + size_of::<EffectAuthority>()
            + 2 * size_of::<usize>()
            + 2 * size_of::<DeviceId>()
            + size_of::<Key>()
            + size_of::<Arc<Packet>>()
            + 1024
            + 2 * 512;
        assert!(
            graph <= super::super::ready::capacity::SOURCE,
            "Ready Source graph: {graph}"
        );
    }
}
