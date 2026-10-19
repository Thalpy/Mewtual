//! Bounded opaque one-helper forwarding. The actor authenticates the direct transport requester,
//! resolves only current roster/peer-bound/connected routes and owns each room's revocation lease.
//! This owner never dials, sees an endpoint key, or treats a submission as playback/delivery.

use super::resources::{Credit, ResourceKind, Resources};
use super::runtime::{ActorOwner, EffectAuthority, IngressTicket};
use super::*;
use catcoms_rt::{Clock, PeerId, RequestAuthority, SharedRequestKeepalive};
use futures::FutureExt;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
mod receipt;
use receipt::AdmissionAuthority;
pub(crate) use receipt::HelperReceipt;

const FANOUT: usize = 4;
const ATTEMPTS: u8 = 3;
const BATCH: usize = 4;
const CURSORS: usize = 64;
type PacketKey = (DeviceId, MessageId);
type WorkKey = (DeviceId, MessageId, DeviceId);

/// Production supplies these only from an authenticated roster binding intersected with the
/// transport's current connected snapshot. Self-signed route advertisements are insufficient.
#[derive(Clone, Copy)]
pub struct BoundRoute {
    pub device: DeviceId,
    pub peer: PeerId,
}

/// Exact local directory incarnation. A helper need not join the room. Its product actor must
/// synchronously revoke this owner on room deletion/replacement, lock and actor retirement.
#[derive(Clone)]
pub struct RoomAuthority {
    pub scope: Scope,
    pub lease: CallLease,
    // Production directory ownership remains charged through every retained/uncertain clone.
    pub(super) custody: Option<SharedRequestKeepalive>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Pending,
    InFlight,
    RemoteAdmitted,
    Exhausted,
    Retired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Admission {
    pub cached: bool,
    pub edges: usize,
}

struct EdgeState {
    attempts: u8,
    stage: Stage,
}
struct Edge {
    target: DeviceId,
    effect: Arc<ForwardAuthority>,
    state: Mutex<EdgeState>,
    in_flight: AtomicBool,
    _credit: Credit,
}
struct Packet {
    hash: [u8; 32],
    envelope: Arc<Envelope>,
    bytes: usize,
    expires: u64,
    deadline: u64,
    edges: BTreeMap<DeviceId, Arc<Edge>>,
    admission: Arc<AdmissionAuthority>,
    _credit: Credit,
}
struct Cursor {
    last: DeviceId,
    expires: u64,
    deadline: u64,
    _credit: Credit,
}
#[derive(Default)]
struct State {
    packets: BTreeMap<PacketKey, Arc<Packet>>,
    cursors: BTreeMap<(DeviceId, u128), Cursor>,
    work_cursor: Option<WorkKey>,
}
pub struct HelperLedger {
    group: [u8; 32],
    clock: Arc<dyn Clock>,
    resources: Arc<Resources>,
    state: Mutex<State>,
}
enum RequestWork {
    Charge(DeviceId),
    Admitted(IngressTicket),
}
redacted_debug!(
    BoundRoute,
    RoomAuthority,
    HelperLedger,
    ForwardJob,
    ForwardCompletion,
    ForwardAuthority,
    Flight
);

impl HelperLedger {
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
        if !std::ptr::eq(Arc::as_ptr(&actor.helper()?), self) {
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
            now <= packet.expires.saturating_add(CLOCK_SKEW_MS)
                || mono < packet.deadline
                || packet
                    .edges
                    .values()
                    .any(|edge| edge.in_flight.load(Ordering::Acquire))
        });
        state.cursors.retain(|_, cursor| {
            now <= cursor.expires.saturating_add(CLOCK_SKEW_MS) || mono < cursor.deadline
        });
    }
    /// Admit immutable original-member bytes once. Duplicates, including different permits,
    /// never replenish targets or attempts. Each helper process enforces its own edge budget;
    /// it does not pretend to share a counter with another machine's helper.
    pub fn admit(
        &self,
        actor: &ActorOwner,
        requester: DeviceId,
        envelope: Envelope,
        context: &Context,
        room: &RoomAuthority,
        routes: &[BoundRoute],
    ) -> Result<Admission, VoiceError> {
        self.admit_inner(
            actor,
            RequestWork::Charge(requester),
            envelope,
            context,
            room,
            routes,
        )
        .map(|receipt| receipt.info())
    }
    /// The transport path has already debited this exact immediate requester before crypto.
    /// Keep its affine frame admission through the whole helper transaction and never spend
    /// requester work a second time. Original issuer admission still follows replay/capacity.
    pub(crate) fn admit_verified(
        &self,
        actor: &ActorOwner,
        ticket: IngressTicket,
        envelope: Envelope,
        context: &Context,
        room: &RoomAuthority,
        routes: &[BoundRoute],
    ) -> Result<HelperReceipt, VoiceError> {
        self.admit_inner(
            actor,
            RequestWork::Admitted(ticket),
            envelope,
            context,
            room,
            routes,
        )
    }
    fn admit_inner(
        &self,
        actor: &ActorOwner,
        work: RequestWork,
        envelope: Envelope,
        context: &Context,
        room: &RoomAuthority,
        routes: &[BoundRoute],
    ) -> Result<HelperReceipt, VoiceError> {
        self.current(actor)?;
        if room.scope != context.scope || !room.lease.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let requester = match &work {
            RequestWork::Charge(requester) => *requester,
            RequestWork::Admitted(ticket) => ticket.requester(),
        };
        let disposition = match &work {
            RequestWork::Charge(_) => actor.authorize_route(requester, &envelope, context)?,
            RequestWork::Admitted(ticket) => {
                actor.authorize_verified_route(ticket, &envelope, context)?
            }
        };
        let final_scope = match disposition {
            Disposition::Forward(final_scope) => final_scope,
            Disposition::Deliver => return Err(VoiceError::Unauthorized),
        };
        let Envelope::ToHelper {
            ref object,
            ref permit,
        } = envelope
        else {
            return Err(VoiceError::Unauthorized);
        };
        let key = (object.issuer(), object.id());
        let hash = object.hash();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.prune(&mut state);
        if let Some(packet) = state.packets.get(&key) {
            if packet.hash != hash {
                return Err(VoiceError::RetryMismatch);
            }
            // A duplicate cannot adopt a new actor, recreated room or longer permit as the
            // owner of old custody. In particular a zero-edge room packet needs this original
            // admission gate even though it has no edge authority to inspect.
            if !packet.admission.authorized() {
                return Err(VoiceError::Unauthorized);
            }
            if let RequestWork::Admitted(ticket) = &work {
                // Cached classification can wait behind this ledger lock too. Repeat the
                // original request/deadline check after its last Clock-bearing owner check.
                ticket.check(actor, &envelope.encode())?;
            }
            return Ok(HelperReceipt::new(packet.clone(), true));
        }
        let routes = checked_routes(routes)?;
        let mut targets = routes
            .keys()
            .copied()
            .filter(|id| *id != object.issuer() && *id != context.local)
            .collect::<Vec<_>>();
        match final_scope {
            // A targeted packet may wait for its exact current member to reconnect. This spends
            // one bounded edge record, but no lower attempt or dial; take_ready requires a fresh
            // proven connected route. Public room fan-out only selects current connected peers.
            FinalScope::Endpoint(target) => targets = vec![target],
            FinalScope::Room => {
                if let Some(cursor) = state.cursors.get(&(object.issuer(), context.scope.room)) {
                    targets.sort_by_key(|id| (*id <= cursor.last, *id));
                }
                targets.truncate(FANOUT);
            }
        }
        let expires = object.expires();
        let delivery_expires = expires.min(permit.expires);
        let admission = Arc::new(AdmissionAuthority {
            helper: actor.effect(object.issuer(), context.local, delivery_expires)?,
            target: object
                .body()
                .target()
                .map(|target| actor.effect(object.issuer(), target, delivery_expires))
                .transpose()?,
            room: room.clone(),
        });
        let remaining = expires
            .saturating_add(CLOCK_SKEW_MS)
            .saturating_sub(self.clock.now_ms());
        if remaining == 0 || remaining > MAX_REMOTE_DELEGATION_REMAINING_MS {
            return Err(VoiceError::Freshness);
        }
        let deadline = self.clock.monotonic_ms().saturating_add(remaining);
        // The two outer variants have identical fields/encoded length; move the original only
        // after final origin admission, preserving one immutable packet without an extra clone.
        let bytes = envelope.encode().len();
        // Parsed packet plus edge/cursor and lower buffers all share the same real process pool.
        // Acquire every persistent charge before changing cursors or publishing this admission.
        let credit = self
            .resources
            .charge(self.group, bytes + 1024, ResourceKind::Helper)?;
        let mut edges = BTreeMap::new();
        for target in &targets {
            let effect = actor.effect(key.0, *target, delivery_expires)?;
            let edge_credit = self
                .resources
                .charge(self.group, 1024, ResourceKind::Helper)?;
            edges.insert(
                *target,
                Arc::new(Edge {
                    target: *target,
                    effect: Arc::new(ForwardAuthority {
                        effect,
                        room: room.lease.clone(),
                        _room_custody: room.custody.clone(),
                    }),
                    state: Mutex::new(EdgeState {
                        attempts: 0,
                        stage: Stage::Pending,
                    }),
                    in_flight: AtomicBool::new(false),
                    _credit: edge_credit,
                }),
            );
        }
        let cursor_key = (key.0, context.scope.room);
        let cursor_credit = if final_scope == FinalScope::Room
            && !targets.is_empty()
            && !state.cursors.contains_key(&cursor_key)
        {
            if state.cursors.len() >= CURSORS {
                return Err(VoiceError::Capacity);
            }
            Some(
                self.resources
                    .charge(self.group, 512, ResourceKind::Helper)?,
            )
        } else {
            None
        };
        // Exact cached packets returned above. Capacity/effect failures have spent only requester
        // work. This final transaction can reject stale renewal without burning origin tokens or
        // extending key protection; after success, every publication below is infallible.
        match &work {
            RequestWork::Charge(_) => {
                actor.commit_route_origin(requester, &envelope, context, &room.lease)?
            }
            RequestWork::Admitted(ticket) => actor.commit_route_origin_checked(
                requester,
                &envelope,
                context,
                &room.lease,
                Some(ticket),
            )?,
        }
        let Envelope::ToHelper { object, permit } = envelope else {
            unreachable!("variant checked above")
        };
        let outgoing = Envelope::HelperDelivery { object, permit };
        if final_scope == FinalScope::Room {
            if let Some(last) = targets.last() {
                if let Some(cursor) = state.cursors.get_mut(&cursor_key) {
                    cursor.last = *last;
                    cursor.expires = expires;
                    cursor.deadline = deadline;
                } else if let Some(credit) = cursor_credit {
                    state.cursors.insert(
                        cursor_key,
                        Cursor {
                            last: *last,
                            expires,
                            deadline,
                            _credit: credit,
                        },
                    );
                }
            }
        }
        let packet = Arc::new(Packet {
            hash,
            envelope: Arc::new(outgoing),
            bytes,
            expires,
            deadline,
            edges,
            admission,
            _credit: credit,
        });
        state.packets.insert(key, packet.clone());
        Ok(HelperReceipt::new(packet, false))
    }

    /// Return finite detached work without awaiting the network on the actor. Re-resolve routes
    /// each turn, then use ONLY request_connected_cancellable with both this authority and its
    /// keepalive. A successful queue/admission response is never endpoint decryption or audio.
    pub fn take_ready(
        &self,
        actor: &ActorOwner,
        routes: &[BoundRoute],
    ) -> Result<Vec<ForwardJob>, VoiceError> {
        self.current(actor)?;
        let routes = checked_routes(routes)?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.prune(&mut state);
        let mut work = state
            .packets
            .iter()
            .flat_map(|(key, packet)| {
                packet
                    .edges
                    .keys()
                    .map(move |target| (key.0, key.1, *target))
            })
            .collect::<Vec<_>>();
        if let Some(cursor) = state.work_cursor {
            work.sort_by_key(|key| (*key <= cursor, *key));
        }
        let mut jobs = Vec::new();
        for key in work {
            let packet = state.packets[&(key.0, key.1)].clone();
            let edge = packet.edges[&key.2].clone();
            let mut status = edge.state.lock().unwrap_or_else(|e| e.into_inner());
            if !edge.effect.authorized() {
                status.stage = Stage::Retired;
                continue;
            }
            if matches!(
                status.stage,
                Stage::RemoteAdmitted | Stage::Exhausted | Stage::Retired
            ) || edge.in_flight.load(Ordering::Acquire)
            {
                continue;
            }
            if status.attempts >= ATTEMPTS {
                status.stage = Stage::Exhausted;
                continue;
            }
            let Some(peer) = routes.get(&edge.target).copied() else {
                continue;
            };
            // Three simultaneous serialized buffers cover the retained outer request plus the
            // nested SignedObject/Envelope encodings used by ACK binding. The outer signing
            // transcript is dropped before framing. Missing capacity spends no attempt.
            let credit = match self.resources.charge(
                self.group,
                3 * packet.bytes + 4096 + ROUTE_METADATA_BYTES,
                ResourceKind::Forward,
            ) {
                Ok(credit) => credit,
                Err(VoiceError::Capacity) => continue,
                Err(error) => return Err(error),
            };
            status.attempts += 1;
            status.stage = Stage::InFlight;
            let attempt = status.attempts;
            edge.in_flight.store(true, Ordering::Release);
            drop(status);
            let flight = Arc::new(Flight {
                edge,
                attempt,
                _credit: credit,
            });
            jobs.push(ForwardJob {
                key,
                packet,
                peer,
                flight,
            });
            state.work_cursor = Some(key);
            if jobs.len() >= BATCH {
                break;
            }
        }
        Ok(jobs)
    }

    /// Only after validating the response's exact peer/member/request/scope may native report
    /// RemoteAdmitted. Dropped/cancelled waiters carry Failed; the lower transport may still own
    /// the flight, so even this commit never clears in_flight or refunds its credit.
    pub fn complete(&self, completion: ForwardCompletion) -> Result<(), VoiceError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let packet = state
            .packets
            .get(&(completion.key.0, completion.key.1))
            .ok_or(VoiceError::Reservation)?;
        let edge = packet
            .edges
            .get(&completion.key.2)
            .ok_or(VoiceError::Reservation)?;
        if !Arc::ptr_eq(packet, &completion.packet) || !Arc::ptr_eq(edge, &completion.flight.edge) {
            return Err(VoiceError::Reservation);
        }
        let mut status = edge.state.lock().unwrap_or_else(|e| e.into_inner());
        if status.attempts != completion.flight.attempt {
            return Err(VoiceError::Reservation);
        }
        status.stage = if !edge.effect.authorized() {
            Stage::Retired
        } else if completion.admitted {
            Stage::RemoteAdmitted
        } else if status.attempts >= ATTEMPTS {
            Stage::Exhausted
        } else {
            Stage::Pending
        };
        Ok(())
    }

    /// Redacted bounded aggregate diagnostics, not a room roster or delivery claim.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.prune(&mut state);
        (
            state.packets.len(),
            state
                .packets
                .values()
                .map(|packet| packet.edges.len())
                .sum(),
            state
                .packets
                .values()
                .flat_map(|packet| packet.edges.values())
                .filter(|edge| edge.in_flight.load(Ordering::Acquire))
                .count(),
        )
    }
}

fn checked_routes(routes: &[BoundRoute]) -> Result<BTreeMap<DeviceId, PeerId>, VoiceError> {
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
    Ok(by_device)
}

struct ForwardAuthority {
    effect: Arc<EffectAuthority>,
    room: CallLease,
    _room_custody: Option<SharedRequestKeepalive>,
}
#[async_trait::async_trait]
impl RequestAuthority for ForwardAuthority {
    fn authorized(&self) -> bool {
        self.room.is_live() && self.effect.authorized()
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        futures::future::select(
            self.effect.revoked().boxed(),
            self.room.wait_revoked().boxed(),
        )
        .await;
    }
}
struct Flight {
    edge: Arc<Edge>,
    attempt: u8,
    _credit: Credit,
}
impl Drop for Flight {
    fn drop(&mut self) {
        // Only the last actual driver/worker/completion owner releases this edge. Timeout,
        // cancellation and actor replacement cannot set this flag early or multiply live work.
        self.edge.in_flight.store(false, Ordering::Release);
    }
}
pub struct ForwardJob {
    key: WorkKey,
    packet: Arc<Packet>,
    peer: PeerId,
    flight: Arc<Flight>,
}
impl ForwardJob {
    pub fn peer(&self) -> PeerId {
        self.peer
    }
    pub fn target(&self) -> DeviceId {
        self.key.2
    }
    pub(crate) fn object_hash(&self) -> [u8; 32] {
        self.packet.hash
    }
    pub fn envelope(&self) -> &Envelope {
        &self.packet.envelope
    }
    pub fn authority(&self) -> Arc<dyn RequestAuthority> {
        self.flight.edge.effect.clone()
    }
    pub fn keepalive(&self) -> SharedRequestKeepalive {
        self.flight.clone()
    }
    /// `admitted=true` requires an exact authenticated remote admission acknowledgement. Caller
    /// must not substitute an ordinary transport submit/notify result or unvalidated response.
    pub fn finish(self, admitted: bool) -> ForwardCompletion {
        ForwardCompletion {
            key: self.key,
            packet: self.packet,
            flight: self.flight,
            admitted,
        }
    }
}
pub struct ForwardCompletion {
    key: WorkKey,
    packet: Arc<Packet>,
    flight: Arc<Flight>,
    admitted: bool,
}

#[cfg(test)]
mod tests;
