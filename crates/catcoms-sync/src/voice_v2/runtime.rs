//! One process owner for V2 time, budgets and group protection across actor/controller changes.
//! Actors are prepared inactive, registered by the native commit, then activated against current
//! MLS state. A captured old session can never activate after lock. This module grants neither
//! room participation nor signing authority; endpoint operations still need their exact owners.

use super::limits::OriginLimits;
use super::resources::{Credit, ResourceKind, Resources};
use super::{CallLease, VoiceError, WatchLease, CLOCK_SKEW_MS};
use catcoms_crypto::DeviceId;
use catcoms_rt::{Clock, RequestAuthority};
use futures::FutureExt;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

const GROUPS: usize = 1024;
const PREPARED_ACTORS: usize = 8;
const PREPARATION_MS: u64 = 30_000;
const MEMBERS: usize = 512;
const CHILDREN: usize = 64;
// Declared serialized-buffer peak, excluding allocator slack: incoming frame, decoded packet,
// SignedObject unsigned encoding, signed object encoding and enclosing Envelope encoding. Outer
// authentication transcripts are dropped before decode; the small ACK fits the fixed allowance.
const VERIFICATION_BUFFERS: usize = 5;
const VERIFICATION_OVERHEAD: usize = 4096;
mod local;
pub(in crate::voice_v2) use local::EpochObservation;
pub(crate) use local::LocalOrigin;

/// Affine final origin transaction lent only while the original Group -> Policy locks and
/// native route/epoch/membership checks remain held. A protected row fence must perform every
/// fallible owner/candidate check BEFORE consuming this value. The apply closure publishes only
/// already-prepaid state; it cannot await, sample Clock, enter Engine, or return a recoverable
/// publication error. No caller can manufacture this transaction from an envelope/header.
pub(crate) struct EndpointOrigin<'a> {
    limits: &'a mut OriginLimits,
    object: &'a super::SignedObject,
    actor: &'a ActorOwner,
    ticket: &'a IngressTicket,
    context: &'a super::Context,
    wall: u64,
    mono: u64,
}
impl EndpointOrigin<'_> {
    pub(crate) fn epoch(&self) -> u64 {
        self.context.epoch
    }
    pub(crate) fn publish(self, apply: impl FnOnce()) -> Result<(), VoiceError> {
        // Session/actor/driver revocation wins atomically before it waits for Policy. The
        // caller may have acquired a nonblocking History fence since the loan was made; repeat
        // these original
        // observations at the quota boundary, without another native time sample or lock.
        // Production original_request is the responder's bounded driver/reply lease; this
        // final observer must not sample Clock or reenter Group/Policy/History.
        self.ticket
            .check_at_commit(self.actor, self.context, self.mono)?;
        self.limits
            .admit_packet(self.object, self.mono, self.wall)?;
        apply();
        Ok(())
    }
}

/// Signed freshness advances with local monotonic elapsed time even during wall-clock rollback.
/// Only local Clock observations set/advance this anchor. Remote timestamps cannot set it. A large
/// correction can make our newly signed objects unacceptable to other peers until clocks align
/// or a fresh process starts. Old joins are never restored across process restart.
struct EffectiveClock {
    source: Arc<dyn Clock>,
    anchor: Mutex<(u64, u64)>, // logical wall, monotonic observation
}
redacted_debug!(EffectiveClock);
impl Clock for EffectiveClock {
    fn now_ms(&self) -> u64 {
        let raw = self.source.now_ms();
        let mono = self.source.monotonic_ms();
        let mut anchor = self.anchor.lock().unwrap_or_else(|e| e.into_inner());
        // Another thread may have observed later elapsed time before this thread takes the lock.
        // Keep both anchors nondecreasing, without double-counting the same elapsed interval.
        let elapsed = mono.saturating_sub(anchor.1);
        anchor.0 = raw.max(anchor.0.saturating_add(elapsed));
        anchor.1 = anchor.1.max(mono);
        anchor.0
    }
    fn monotonic_ms(&self) -> u64 {
        self.source.monotonic_ms()
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.source.sleep(duration)
    }
}

struct Session {
    live: CallLease,
    active: AtomicBool,
    observation: Arc<AtomicBool>,
}
impl Session {
    fn inactive() -> Arc<Self> {
        Arc::new(Self {
            live: CallLease::default(),
            active: AtomicBool::new(false),
            observation: Arc::new(AtomicBool::new(true)),
        })
    }
    fn authorized(&self) -> bool {
        self.live.is_live() && self.active.load(Ordering::Acquire)
    }
}
/// Capture before an unlock/join/restore await, then activate/register only inside its final native
/// commit. A replacement session is a different owner, even if the account and group are identical.
#[derive(Clone)]
pub struct SessionTicket(Arc<Session>);
redacted_debug!(SessionTicket);

struct Member {
    live: CallLease,
    _credit: Credit,
}
struct GroupState {
    revision: u64,
    epoch: Option<u64>,
    members: BTreeMap<DeviceId, Arc<Member>>,
    active: Weak<ActorGate>,
    limits: OriginLimits,
}
struct Group {
    hash: [u8; 32],
    state: Mutex<GroupState>,
    _credit: Credit,
    helper: Arc<super::helper::HelperLedger>,
    source: Arc<super::source::SourceLedger>,
    verification: Arc<Mutex<super::limits::Bucket>>,
}
struct Policy {
    pin: Option<[u8; 32]>,
    eligible: bool,
    indirect: bool,
    live: CallLease,
    children: Vec<CallLease>,
    observation: Arc<AtomicBool>,
}
impl Policy {
    fn revoke(&mut self) {
        self.observation.store(false, Ordering::Release);
        self.live.revoke();
        for child in &self.children {
            child.revoke();
        }
        self.children.clear();
    }
}
struct ActorGate {
    local: DeviceId,
    live: CallLease,
    session: Arc<Session>,
    registered: AtomicBool,
    installed_revision: AtomicU64,
    // Pure observation for already-paid native transition custody. Capture is allowed only
    // under the actual Group/Policy view; no serialized epoch can construct that witness.
    observation: Arc<local::EpochState>,
    prepared_until: u64,
    activated: AtomicBool,
    policy: Mutex<Policy>,
    _credit: Credit,
}
impl ActorGate {
    fn revoke(&self) {
        self.observation.live.store(false, Ordering::Release);
        self.live.revoke();
        self.policy
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .revoke();
    }
    fn authorized(&self) -> bool {
        self.live.is_live()
            && self.session.authorized()
            && self.registered.load(Ordering::Acquire)
            && self.activated.load(Ordering::Acquire)
    }
}
struct State {
    session: Arc<Session>,
    groups: BTreeMap<[u8; 32], Arc<Group>>,
    actors: Vec<Weak<ActorGate>>,
}
/// Native must share this exact instance with every product actor and the browser-credit owner.
pub struct Runtime {
    clock: Arc<EffectiveClock>,
    resources: Arc<Resources>,
    state: Mutex<State>,
    verification: Arc<Mutex<super::limits::Bucket>>,
}
redacted_debug!(Runtime, ActorOwner, ActorRegistration, EffectAuthority);

impl Drop for Runtime {
    fn drop(&mut self) {
        // Detached work may own gates/resources without owning this process registry. Dropping
        // the registry closes those gates; constructing another registry cannot leave old effects
        // live alongside a fresh budget. Actual lower holders still keep their old credits.
        let state = self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        state.session.observation.store(false, Ordering::Release);
        state.session.live.revoke();
        for actor in state.actors.iter().filter_map(Weak::upgrade) {
            actor.revoke();
        }
    }
}

impl Runtime {
    pub fn new(clock: Arc<dyn Clock>) -> Arc<Self> {
        let anchor = (clock.now_ms(), clock.monotonic_ms());
        Arc::new(Self {
            verification: Arc::new(Mutex::new(super::limits::Bucket::new(anchor.1, 128))),
            clock: Arc::new(EffectiveClock {
                source: clock,
                anchor: Mutex::new(anchor),
            }),
            resources: Arc::new(Resources::default()),
            state: Mutex::new(State {
                session: Session::inactive(),
                groups: BTreeMap::new(),
                actors: Vec::new(),
            }),
        })
    }
    pub fn clock(&self) -> Arc<dyn Clock> {
        self.clock.clone()
    }
    pub fn resources(&self) -> Arc<Resources> {
        self.resources.clone()
    }
    /// Bounded process housekeeping, independent of unlocked session/actor authority. Snapshot
    /// group owners before taking any ledger locks: admission uses Source/Helper -> Group ->
    /// Policy, so retaining the process or Group mutex while pruning a ledger would invert it.
    /// Never remove group revisions/membership, pair counters, or uncertain engine operations.
    pub fn collect_expired(&self) {
        let groups: Vec<_> = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .groups
            .values()
            .cloned()
            .collect();
        let now = self.clock.monotonic_ms();
        self.resources.collect_workflows(now);
        for group in groups {
            group.source.collect_expired();
            group.helper.collect_expired();
            group
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .limits
                .collect_expired(now);
        }
    }
    pub fn session_ticket(&self) -> SessionTicket {
        SessionTicket(
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .session
                .clone(),
        )
    }
    pub fn is_current_ticket(&self, ticket: &SessionTicket) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Arc::ptr_eq(&state.session, &ticket.0) && ticket.0.live.is_live()
    }
    /// Synchronous native commit only. Caller must already hold its exact unlocked UI commit guard.
    pub fn activate_session(&self, ticket: &SessionTicket) -> Result<(), VoiceError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !Arc::ptr_eq(&state.session, &ticket.0) || !ticket.0.live.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        ticket.0.active.store(true, Ordering::Release);
        Ok(())
    }
    /// The first part of lock/account replacement, before awaiting any actor or disk operation.
    /// Protection tables and uncertain lower work remain charged; unlock cannot revive old gates.
    pub fn revoke_session(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.session.observation.store(false, Ordering::Release);
        state.session.live.revoke();
        state.session = Session::inactive();
        state.actors.retain(|gate| {
            if let Some(gate) = gate.upgrade() {
                gate.revoke();
                true
            } else {
                false
            }
        });
    }
    pub fn prepare_actor(
        &self,
        hash: [u8; 32],
        local: DeviceId,
        ticket: &SessionTicket,
    ) -> Result<(ActorOwner, ActorRegistration), VoiceError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !Arc::ptr_eq(&state.session, &ticket.0) || !ticket.0.live.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        state.actors.retain(|actor| actor.strong_count() != 0);
        if state
            .actors
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|actor| {
                !actor.registered.load(Ordering::Acquire)
                    && actor.live.is_live()
                    && actor.session.live.is_live()
                    && actor.prepared_until > self.clock.monotonic_ms()
            })
            .count()
            >= PREPARED_ACTORS
        {
            return Err(VoiceError::Capacity);
        }
        if !state.groups.contains_key(&hash) {
            if state.groups.len() >= GROUPS {
                return Err(VoiceError::Capacity);
            }
            let credit = self.resources.charge(hash, 1024, ResourceKind::Retained)?;
            state.groups.insert(
                hash,
                Arc::new(Group {
                    hash,
                    _credit: credit,
                    verification: self.verification.clone(),
                    helper: super::helper::HelperLedger::new(
                        hash,
                        self.clock.clone(),
                        self.resources.clone(),
                    ),
                    source: super::source::SourceLedger::new(
                        hash,
                        self.clock.clone(),
                        self.resources.clone(),
                    ),
                    state: Mutex::new(GroupState {
                        revision: 0,
                        epoch: None,
                        members: BTreeMap::new(),
                        active: Weak::new(),
                        limits: OriginLimits::new(hash, self.resources.clone()),
                    }),
                }),
            );
        }
        let group = state.groups[&hash].clone();
        let revision = group
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .revision;
        let credit = self.resources.charge(hash, 1024, ResourceKind::Retained)?;
        let gate = Arc::new(ActorGate {
            local,
            live: CallLease::default(),
            session: ticket.0.clone(),
            registered: AtomicBool::new(false),
            installed_revision: AtomicU64::new(0),
            observation: Arc::new(local::EpochState {
                epoch: AtomicU64::new(0),
                live: AtomicBool::new(true),
            }),
            prepared_until: self.clock.monotonic_ms().saturating_add(PREPARATION_MS),
            activated: AtomicBool::new(false),
            policy: Mutex::new(Policy {
                pin: None,
                eligible: false,
                indirect: false,
                live: CallLease::default(),
                children: Vec::new(),
                observation: Arc::new(AtomicBool::new(true)),
            }),
            _credit: credit,
        });
        state.actors.push(Arc::downgrade(&gate));
        Ok((
            ActorOwner {
                group: group.clone(),
                gate: gate.clone(),
                revision,
                clock: self.clock.clone(),
                resources: self.resources.clone(),
            },
            ActorRegistration {
                _preparation: Arc::new(PreparationGuard { gate: gate.clone() }),
                gate,
                group,
                expected_revision: revision,
                clock: self.clock.clone(),
            },
        ))
    }
}

/// Unique actor-owned handle. Dropping the real actor irrevocably retires its detached effects.
pub struct ActorOwner {
    group: Arc<Group>,
    gate: Arc<ActorGate>,
    revision: u64,
    clock: Arc<EffectiveClock>,
    resources: Arc<Resources>,
}
/// Native registration capability, intentionally separate from the unique actor owner. Retaining
/// this or losing a reply never keeps a dropped actor's effects authorized.
#[derive(Clone)]
pub struct ActorRegistration {
    gate: Arc<ActorGate>,
    group: Arc<Group>,
    expected_revision: u64,
    clock: Arc<EffectiveClock>,
    _preparation: Arc<PreparationGuard>,
}
struct PreparationGuard {
    gate: Arc<ActorGate>,
}
/// Native installation distinguishes a competing actor from optional V2 capacity/lifetime
/// loss. A conflict must preserve the current native row; unavailability may leave chat usable
/// with no voice attachment. Both outcomes grant no V2 effects.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistrationError {
    #[error("voice actor installation was superseded")]
    Conflict,
    #[error(transparent)]
    Unavailable(VoiceError),
}
impl Drop for PreparationGuard {
    fn drop(&mut self) {
        // The last native preparation waiter vanished before installation. Root may still hold
        // its candidate, but it must never activate and must not consume a live candidate slot.
        if !self.gate.registered.load(Ordering::Acquire) {
            self.gate.revoke();
        }
    }
}
impl ActorRegistration {
    /// Native may skip duplicate already-open unlock work only for this exact session and a
    /// still-installed registration. A new ticket after lock can never match the old actor.
    pub fn is_current_for(&self, ticket: &SessionTicket) -> bool {
        let state = self.group.state.lock().unwrap_or_else(|e| e.into_inner());
        Arc::ptr_eq(&self.gate.session, &ticket.0)
            && self.gate.live.is_live()
            && self.gate.session.live.is_live()
            && self.gate.registered.load(Ordering::Acquire)
            && state.revision == self.gate.installed_revision.load(Ordering::Acquire)
            && state
                .active
                .upgrade()
                .is_some_and(|gate| Arc::ptr_eq(&gate, &self.gate))
    }
    /// Linearize the final native install before publishing its registry entry. Hold the native
    /// exact session/entry commit boundary, with no intervening await or fallible operation.
    /// Preparing competing owners does not retire anything; only one can replace that revision.
    pub fn register(&self) -> Result<(), VoiceError> {
        // Fast rejection for Rust callers that need only the legacy error vocabulary. Native
        // installation uses register_install's atomic conflict classification below instead.
        if !self.gate.live.is_live() || !self.gate.session.live.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        if !self.gate.registered.load(Ordering::Acquire)
            && self.clock.monotonic_ms() >= self.gate.prepared_until
        {
            self.gate.revoke();
            return Err(VoiceError::Unauthorized);
        }
        self.register_install().map_err(|error| match error {
            RegistrationError::Conflict => VoiceError::Unauthorized,
            RegistrationError::Unavailable(error) => error,
        })
    }
    pub fn register_install(&self) -> Result<(), RegistrationError> {
        let mut state = self.group.state.lock().unwrap_or_else(|e| e.into_inner());
        let expected = if self.gate.registered.load(Ordering::Acquire) {
            self.gate.installed_revision.load(Ordering::Acquire)
        } else {
            self.expected_revision
        };
        if state.revision != expected {
            self.gate.revoke();
            return Err(RegistrationError::Conflict);
        }
        // The mutex may have waited behind another native install/control turn. Recheck at this
        // actual admission boundary before retiring the currently installed actor.
        if !self.gate.live.is_live()
            || !self.gate.session.live.is_live()
            || (!self.gate.registered.load(Ordering::Acquire)
                && self.clock.monotonic_ms() >= self.gate.prepared_until)
        {
            self.gate.revoke();
            return Err(RegistrationError::Unavailable(VoiceError::Unauthorized));
        }
        if self.gate.registered.load(Ordering::Acquire) {
            return Ok(());
        }
        let revision = state
            .revision
            .checked_add(1)
            .ok_or(RegistrationError::Unavailable(VoiceError::Capacity))?;
        if let Some(old) = state.active.upgrade() {
            old.revoke();
        }
        state.revision = revision;
        state.active = Arc::downgrade(&self.gate);
        self.gate
            .installed_revision
            .store(revision, Ordering::Release);
        self.gate.registered.store(true, Ordering::Release);
        // Lock can race registration. It irreversibly closes the same session before awaiting
        // any actor; failed native installation must not publish this actor in its registry.
        if !self.gate.session.live.is_live() {
            self.gate.revoke();
            return Err(RegistrationError::Unavailable(VoiceError::Unauthorized));
        }
        Ok(())
    }
    pub fn revoke(&self) {
        self.gate.revoke();
    }
}
impl Drop for ActorOwner {
    fn drop(&mut self) {
        self.gate.revoke();
    }
}

/// Trusted current MLS/policy view, supplied synchronously by the actor after control recovery.
pub struct View {
    pub epoch: u64,
    pub members: BTreeSet<DeviceId>,
    pub policy: Option<[u8; 32]>,
    pub eligible: bool,
    pub indirect: bool,
}
redacted_debug!(View);
impl ActorOwner {
    #[cfg(test)]
    pub(crate) fn with_held_policy_for_test<T>(&self, check: impl FnOnce() -> T) -> T {
        // A separate fixture thread holds the actual Policy mutex. The caller under test must
        // refuse at try_lock, without imposing a lock order on its own executing thread.
        let _policy = self.gate.policy.lock().unwrap_or_else(|e| e.into_inner());
        check()
    }
    #[cfg(test)]
    pub(crate) fn revocation_observer_for_test(&self) -> CallLease {
        // Tests wait for the actual atomic close while revoke_session is blocked on Policy.
        // This clones its original observation gate; it does not fabricate child authority.
        self.gate.live.clone()
    }
    pub(crate) fn uses_resources(&self, resources: &Arc<Resources>) -> bool {
        Arc::ptr_eq(&self.resources, resources)
    }
    pub(super) fn group_hash(&self) -> [u8; 32] {
        self.group.hash
    }
    /// Root retains prepared/registered-inactive candidates until exact install and unlock.
    pub(crate) fn viable(&self) -> bool {
        self.gate.live.is_live()
            && self.gate.session.live.is_live()
            && (self.gate.registered.load(Ordering::Acquire)
                || self.clock.monotonic_ms() < self.gate.prepared_until)
    }
    pub(crate) fn installed(&self) -> bool {
        self.viable() && self.gate.registered.load(Ordering::Acquire)
    }
    /// A prepared actor stays inert until both native registration and the captured unlock commit.
    /// The same-epoch member set must agree with retained process evidence; stale restored actors
    /// wait for control recovery rather than authorizing removed members from an old snapshot.
    pub fn sync_view(&mut self, view: View) -> Result<(), VoiceError> {
        self.sync_view_retiring(view, |_| {})
    }
    /// The sole Root retires owned watcher/pair/proof state for each authenticated removal before
    /// another await or re-add. The callback runs under GroupState, before fallible additions;
    /// it may revoke/drop endpoint-local records only, never reenter runtime/source/helper locks.
    pub(crate) fn sync_view_retiring(
        &mut self,
        view: View,
        mut retire: impl FnMut(DeviceId),
    ) -> Result<(), VoiceError> {
        if !self.gate.live.is_live()
            || !self.gate.session.authorized()
            || !self.gate.registered.load(Ordering::Acquire)
        {
            return Err(VoiceError::Unauthorized);
        }
        if view.members.len() > MEMBERS || !view.members.contains(&self.gate.local) {
            self.gate.revoke();
            return Err(VoiceError::Unauthorized);
        }
        let mut state = self.group.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.epoch.is_some_and(|epoch| {
            view.epoch < epoch
                || (view.epoch == epoch && !view.members.iter().eq(state.members.keys()))
        }) {
            // A not-yet-active restored candidate may catch up. An activated actor reporting a
            // contradictory current view must close its existing authority immediately.
            if self.gate.activated.load(Ordering::Acquire) {
                self.gate.revoke();
            }
            return Err(VoiceError::Freshness);
        }
        if state.revision != self.gate.installed_revision.load(Ordering::Acquire) {
            self.gate.revoke();
            return Err(VoiceError::Unauthorized);
        }
        self.revision = state.revision;
        self.gate.activated.store(true, Ordering::Release);
        // Publish before fallible roster changes. An old transfer window must stop retaining
        // live capacity even when an unrelated add keeps both endpoints in the new epoch.
        // Older views are rejected above, so an old observation can never become current again.
        self.gate
            .observation
            .epoch
            .store(view.epoch, Ordering::Release);
        // Revocation precedes fallible allocation. If new membership cannot be represented, no
        // removed device remains admitted. A later re-add always gets a different Member owner.
        state.members.retain(|id, member| {
            if view.members.contains(id) {
                true
            } else {
                retire(*id);
                member.live.revoke();
                false
            }
        });
        for id in &view.members {
            if !state.members.contains_key(id) {
                let credit =
                    match self
                        .resources
                        .charge(self.group.hash, 128, ResourceKind::Retained)
                    {
                        Ok(credit) => credit,
                        Err(error) => {
                            self.gate.revoke();
                            return Err(error);
                        }
                    };
                state.members.insert(
                    *id,
                    Arc::new(Member {
                        live: CallLease::default(),
                        _credit: credit,
                    }),
                );
            }
        }
        state.epoch = Some(view.epoch);
        let mut policy = self.gate.policy.lock().unwrap_or_else(|e| e.into_inner());
        if policy.pin != view.policy
            || policy.eligible != view.eligible
            || policy.indirect != view.indirect
            || !policy.live.is_live()
        {
            policy.revoke();
            policy.pin = view.policy;
            policy.eligible = view.eligible;
            policy.indirect = view.indirect;
            policy.live = CallLease::default();
            policy.observation = Arc::new(AtomicBool::new(true));
        }
        Ok(())
    }
    fn current(&self) -> Result<(), VoiceError> {
        if self.gate.authorized() {
            Ok(())
        } else {
            Err(VoiceError::Unauthorized)
        }
    }
    pub fn helper(&self) -> Result<Arc<super::helper::HelperLedger>, VoiceError> {
        self.current()?;
        Ok(self.group.helper.clone())
    }
    pub fn source(&self) -> Result<Arc<super::source::SourceLedger>, VoiceError> {
        self.current()?;
        Ok(self.group.source.clone())
    }
    /// Borrow the already-paid exact native membership map for a small pure role check.
    /// This avoids a second roster Vec/HashSet on the retained ACK path. The closure MUST
    /// be bounded and synchronous: no Clock, effect or re-entry into native owners. Any
    /// canonical hashing scratch must already be prepaid by the public verification ticket;
    /// the private same-job ACK path needs none. Authenticate signatures/sample time first.
    pub(crate) fn with_current_members<T>(
        &self,
        context: &super::Context,
        check: impl FnOnce(&dyn Fn(DeviceId) -> bool) -> Result<T, VoiceError>,
    ) -> Result<T, VoiceError> {
        self.current()?;
        let state = self.group.state.lock().unwrap_or_else(|e| e.into_inner());
        let policy = self.gate.policy.lock().unwrap_or_else(|e| e.into_inner());
        self.check_route_view(&state, &policy, context.local, context)?;
        check(&|id| state.members.contains_key(&id))
    }
    pub(crate) fn source_context(&self, scope: super::Scope) -> Result<super::Context, VoiceError> {
        self.current()?;
        // Directory/grant validation also enters this path. EffectiveClock invokes the original
        // injected clock: sampling beneath Group/Policy can deadlock session revocation and
        // let a callback outlive a previous currency check. Sample before nonblocking acquisition,
        // then repeat the original atomic owner check after both guards. SourceLedger must keep
        // already-prepared jobs when this returns local Capacity for a later packet.
        let wall = self.clock.now_ms();
        let state = match self.group.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        let policy = match self.gate.policy.try_lock() {
            Ok(policy) => policy,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        self.current()?;
        if state.revision != self.revision
            || scope.group != self.group.hash
            || scope.policy != policy.pin
            || !policy.eligible
            || !policy.live.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(super::Context {
            scope,
            local: self.gate.local,
            now: wall,
            epoch: state.epoch.ok_or(VoiceError::Unauthorized)?,
            indirect: policy.indirect,
        })
    }
    /// Source admission differs from inbound routing: the issuer must be this native device.
    /// Called under SourceState; all persistent credits are reserved before commit=true. No
    /// unrestricted signing or UI assertion can manufacture the private activity handle.
    pub(super) fn authorize_source(
        &self,
        object: &super::SignedObject,
        activity: &super::source::SourceActivity,
        room: &super::helper::RoomAuthority,
        commit: bool,
    ) -> Result<(), VoiceError> {
        self.current()?;
        let mut state = self.group.state.lock().unwrap_or_else(|e| e.into_inner());
        let policy = self.gate.policy.lock().unwrap_or_else(|e| e.into_inner());
        if state.revision != self.revision
            || object.scope().group != self.group.hash
            || object.scope().policy != policy.pin
            || object.issuer() != self.gate.local
            || room.scope != object.scope()
            || !room.lease.is_live()
            || !state.members.contains_key(&object.issuer())
            || object
                .body()
                .target()
                .is_some_and(|id| !state.members.contains_key(&id))
            || !policy.eligible
            || !policy.live.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        activity.check(object)?;
        let wall = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        object.check_freshness(wall)?;
        if matches!(object.body(), super::Body::Announcement { epoch, .. } if Some(*epoch) > state.epoch)
        {
            return Err(VoiceError::Freshness);
        }
        // Native lock/replacement can close atomics while waiting for this mutex or clock seam.
        self.current()?;
        activity.check(object)?;
        if !room.lease.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        if commit {
            state.limits.admit_packet(object, mono, wall)?;
        }
        Ok(())
    }
    /// The caller has already authenticated a peer-bound request and checked the real product
    /// directory for this room. Repeat retained membership/policy and native time here, then
    /// spend immediate-requester work only. Original quota commits later, together with the
    /// protected new packet record; a cached retry cannot spend someone else's origin allowance.
    pub(super) fn authorize_route(
        &self,
        requester: DeviceId,
        envelope: &super::Envelope,
        context: &super::Context,
    ) -> Result<super::Disposition, VoiceError> {
        self.route_operation(requester, envelope, context, true, |_, _, _| Ok(()))
            .map(|(disposition, ())| disposition)
    }
    /// Caller holds its packet-ledger lock and has reserved all persistent capacity. Revalidate
    /// before the final infallible insertion, without charging requester work a second time.
    pub(super) fn commit_route_origin(
        &self,
        requester: DeviceId,
        envelope: &super::Envelope,
        context: &super::Context,
        room: &CallLease,
    ) -> Result<(), VoiceError> {
        self.commit_route_origin_checked(requester, envelope, context, room, None)
    }
    pub(super) fn commit_route_origin_checked(
        &self,
        requester: DeviceId,
        envelope: &super::Envelope,
        context: &super::Context,
        room: &CallLease,
        ticket: Option<&IngressTicket>,
    ) -> Result<(), VoiceError> {
        self.route_operation(requester, envelope, context, false, |limits, native, _| {
            if !room.is_live() {
                return Err(VoiceError::Unauthorized);
            }
            let now = self.clock.monotonic_ms();
            if let Some(ticket) = ticket {
                ticket.check_at_commit(self, context, now)?;
            }
            limits.admit_packet(envelope.object(), now, native.now)
        })
        .map(|_| ())
    }
    /// Final endpoint admission transaction. All queue/receipt bytes and replay preparation must
    /// precede this call. The callback checks exact native consent (and prepared encrypted pair
    /// authority when applicable) under Group -> Policy immediately before original quota.
    pub(crate) fn commit_endpoint_origin(
        &self,
        ticket: &IngressTicket,
        envelope: &super::Envelope,
        context: &super::Context,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        self.with_endpoint_origin(ticket, envelope, context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            origin.publish(|| ())
        })
    }
    /// This final endpoint path never waits for Group/Policy after sampling time. Contention
    /// reports transient Capacity (the existing retryable transport vocabulary) without origin
    /// debit; retry must enter again with fresh native samples. The general helper route path
    /// deliberately remains separate. A
    /// protected consumer may then enter its short History/candidate fence, check pure leases
    /// using these samples, and atomically spend/publish without a later fallible row operation.
    /// It must use try_lock and abort on contention: blocking after the samples were frozen
    /// could let an original deadline expire without another time validation. Keep unstarted
    /// row owners outside this callback so their Drop cannot wait for History under Policy.
    /// Existing public intake uses the wrapper above; encrypted dispatch remains disabled.
    pub(crate) fn with_endpoint_origin(
        &self,
        ticket: &IngressTicket,
        envelope: &super::Envelope,
        context: &super::Context,
        publish: impl FnOnce(u64, u64, EndpointOrigin<'_>) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        self.current()?;
        // Clock is an injected callback and may itself inspect/revoke native state. Do not
        // invoke it under admission guards, or reuse these samples after any blocking wait.
        let wall = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        let mut state = match self.group.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        let policy = match self.gate.policy.try_lock() {
            Ok(policy) => policy,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        self.check_route_view(&state, &policy, ticket.requester(), context)?;
        let native = super::Context {
            now: wall,
            ..*context
        };
        if envelope.authorize(ticket.requester(), &native, |id| {
            state.members.contains_key(&id)
        })? != super::Disposition::Deliver
        {
            return Err(VoiceError::Unauthorized);
        }
        ticket.check_at_commit(self, context, mono)?;
        // Original request observers supplied by native ingress must be pure, bounded lifetime
        // observations here. Production uses Responder -> ReplyLifetime -> IngressAuthority
        // (connection/response gates), never a Clock-bearing application effect authority.
        publish(
            wall,
            mono,
            EndpointOrigin {
                limits: &mut state.limits,
                object: envelope.object(),
                actor: self,
                ticket,
                context,
                wall,
                mono,
            },
        )
    }

    /// Common pure route/view checks for both ordinary routing and final endpoint admission.
    /// Keep authorization in one place so the nonblocking final path cannot drift from the
    /// existing member, epoch, policy and exact actor-revision boundary.
    fn check_route_view(
        &self,
        state: &GroupState,
        policy: &Policy,
        requester: DeviceId,
        context: &super::Context,
    ) -> Result<(), VoiceError> {
        if state.revision != self.revision
            || context.scope.group != self.group.hash
            || context.local != self.gate.local
            || Some(context.epoch) != state.epoch
            || context.scope.policy != policy.pin
            || context.indirect != policy.indirect
            || !policy.eligible
            || !policy.live.is_live()
            || !state.members.contains_key(&requester)
        {
            return Err(VoiceError::Unauthorized);
        }
        self.current()
    }
    fn route_operation<T>(
        &self,
        requester: DeviceId,
        envelope: &super::Envelope,
        context: &super::Context,
        spend_requester: bool,
        action: impl FnOnce(
            &mut OriginLimits,
            &super::Context,
            super::Disposition,
        ) -> Result<T, VoiceError>,
    ) -> Result<(super::Disposition, T), VoiceError> {
        self.current()?;
        let mut state = self.group.state.lock().unwrap_or_else(|e| e.into_inner());
        let policy = self.gate.policy.lock().unwrap_or_else(|e| e.into_inner());
        self.check_route_view(&state, &policy, requester, context)?;
        let now = self.clock.now_ms();
        if spend_requester {
            state
                .limits
                .requester(requester, self.clock.monotonic_ms())?;
        }
        let native = super::Context { now, ..*context };
        let disposition =
            envelope.authorize(requester, &native, |id| state.members.contains_key(&id))?;
        // Native lock/replacement closes actor/session atomics before waiting for this policy
        // mutex. A clock sample or contended lock may have let that revocation win meanwhile.
        self.current()?;
        action(&mut state.limits, &native, disposition).map(|result| (disposition, result))
    }
    pub fn child_lease(&self) -> Result<CallLease, VoiceError> {
        self.current()?;
        let mut policy = self.gate.policy.lock().unwrap_or_else(|e| e.into_inner());
        if !policy.eligible || !policy.live.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        policy.children.retain(CallLease::is_live);
        if policy.children.len() >= CHILDREN {
            return Err(VoiceError::Capacity);
        }
        let lease = CallLease::default();
        policy.children.push(lease.clone());
        // Revocation uses this same policy lock, so no child is installed after an observed lock.
        if !self.gate.authorized() {
            lease.revoke();
            return Err(VoiceError::Unauthorized);
        }
        Ok(lease)
    }
    pub fn watch_lease(&self) -> Result<WatchLease, VoiceError> {
        self.child_lease().map(WatchLease::new)
    }
    /// Recognize an exact child issued by this current actor/policy. A matching group/room and
    /// live caller-created lease is insufficient provenance for starting a native endpoint.
    pub(crate) fn recognizes_child(&self, child: &CallLease) -> Result<(), VoiceError> {
        self.current()?;
        let policy = self.gate.policy.lock().unwrap_or_else(|e| e.into_inner());
        if !policy.eligible
            || !policy.live.is_live()
            || !child.is_live()
            || !policy.children.iter().any(|issued| issued.is_same(child))
        {
            return Err(VoiceError::Unauthorized);
        }
        self.current()
    }
    /// Short synchronous closure only. This retained owner must not be reconstructed by the UI,
    /// actor replacement or helper selection. Never hold this lock across network/engine awaits
    /// or reenter any actor/runtime/helper/native operation from the closure. Helper admission's
    /// final transaction takes locks in HelperState -> GroupState -> Policy order.
    pub fn with_limits<T>(
        &self,
        action: impl FnOnce(&mut OriginLimits) -> Result<T, VoiceError>,
    ) -> Result<T, VoiceError> {
        self.current()?;
        let mut state = self.group.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.revision != self.revision {
            return Err(VoiceError::Unauthorized);
        }
        action(&mut state.limits)
    }
    /// Capture permanent issuer AND final-member generations for one already-validated packet.
    /// `expires` is the original signed object's expiry, not a helper permit or arrival renewal.
    pub fn effect(
        &self,
        origin: DeviceId,
        target: DeviceId,
        expires: u64,
    ) -> Result<Arc<EffectAuthority>, VoiceError> {
        self.current()?;
        // Clock is an injected callback. Never invoke it under Group/Policy, and do not wait
        // after freezing these samples: contention is retryable before any effect is issued.
        let wall = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        let state = match self.group.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        if state.revision != self.revision {
            return Err(VoiceError::Unauthorized);
        }
        let origin = state
            .members
            .get(&origin)
            .ok_or(VoiceError::Unauthorized)?
            .clone();
        let target = state
            .members
            .get(&target)
            .ok_or(VoiceError::Unauthorized)?
            .clone();
        let policy = match self.gate.policy.try_lock() {
            Ok(policy) => policy,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        self.current()?;
        if !policy.eligible || !policy.live.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let remaining = expires.saturating_add(CLOCK_SKEW_MS).saturating_sub(wall);
        if remaining == 0 || remaining > super::MAX_REMOTE_DELEGATION_REMAINING_MS {
            return Err(VoiceError::Freshness);
        }
        Ok(Arc::new(EffectAuthority {
            actor: self.gate.clone(),
            origin,
            target,
            policy: policy.live.clone(),
            clock: self.clock.clone(),
            expires,
            deadline: mono.saturating_add(remaining),
        }))
    }
}

/// No group mutex or mutable actor snapshot is held by detached lower work. These exact atomic
/// owners cannot be revived by remove/re-add, lock/unlock, policy change or actor replacement.
pub struct EffectAuthority {
    actor: Arc<ActorGate>,
    origin: Arc<Member>,
    target: Arc<Member>,
    policy: CallLease,
    clock: Arc<EffectiveClock>,
    expires: u64,
    deadline: u64,
}

/// Crate-private, affine admission of one exact transport frame. A Noise-bound current member
/// is resolved before minting this token; it debits requester work before signature verification.
/// No original-issuer quota, room/call authority or packet signing permission follows from it.
pub(crate) struct IngressTicket {
    actor: Arc<ActorGate>,
    member: Arc<Member>,
    policy: CallLease,
    epoch: u64,
    requester: DeviceId,
    hash: [u8; 32],
    bytes: usize,
    deadline: u64,
    clock: Arc<EffectiveClock>,
    original_request: Option<Arc<dyn RequestAuthority>>,
    _credit: Arc<Credit>,
}
redacted_debug!(IngressTicket);
impl IngressTicket {
    /// Observe the original requester/actor/session/policy and absolute verification deadline
    /// through an eventual reply send. This is not endpoint admission or byte custody; native
    /// additionally captures an exact RouteGate for epoch/proof/descriptor/connection changes.
    pub(crate) fn reply_authority(&self) -> Arc<VerificationReplyAuthority> {
        Arc::new(VerificationReplyAuthority {
            actor: self.actor.clone(),
            member: self.member.clone(),
            policy: self.policy.clone(),
            clock: self.clock.clone(),
            deadline: self.deadline,
        })
    }
    /// Admission-only original driver lifetime. Do not carry this into a final reply gate or
    /// persistent packet: receiving a reply closes the wait before network send, and already
    /// admitted helper custody may legitimately outlive its requester's uncertain wait.
    pub(crate) fn bind_request(
        mut self,
        original: Arc<dyn RequestAuthority>,
    ) -> Result<Self, VoiceError> {
        if self.original_request.is_some() || !original.authorized() {
            return Err(VoiceError::Unauthorized);
        }
        self.original_request = Some(original);
        Ok(self)
    }
    pub(crate) fn actual_work(&self) -> catcoms_rt::SharedRequestKeepalive {
        self._credit.clone()
    }
    pub(crate) fn requester(&self) -> DeviceId {
        self.requester
    }
    /// Called after the outer peer-bound signature succeeds and again at native routing.
    /// Canonical re-encoding binds decoded objects to the exact admitted inner frame; retaining
    /// the declared buffer reservation covers the simultaneous nested canonical encodings.
    pub(crate) fn check(&self, actor: &ActorOwner, frame: &[u8]) -> Result<(), VoiceError> {
        actor.current()?;
        let state = actor.group.state.lock().unwrap_or_else(|e| e.into_inner());
        let policy = actor.gate.policy.lock().unwrap_or_else(|e| e.into_inner());
        if !Arc::ptr_eq(&self.actor, &actor.gate)
            || state.revision != actor.revision
            || state.epoch != Some(self.epoch)
            || !state
                .members
                .get(&self.requester)
                .is_some_and(|current| Arc::ptr_eq(current, &self.member))
            || !self.member.live.is_live()
            || !policy.live.is_same(&self.policy)
            || !self.policy.is_live()
            || !policy.eligible
            || actor.clock.monotonic_ms() >= self.deadline
            || frame.len() != self.bytes
            || *blake3::hash(frame).as_bytes() != self.hash
        {
            return Err(VoiceError::Unauthorized);
        }
        actor.current()?;
        if self
            .original_request
            .as_ref()
            .is_some_and(|original| !original.authorized())
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    /// Called under the final GroupState -> Policy locks, after all helper reservations. The
    /// earlier exact frame check cannot authorize a ticket that expired while those locks were
    /// contended. Membership/policy generations never revive; route_operation also checks their
    /// current epoch and policy before this closure. Re-entering check() here would deadlock.
    fn check_at_commit(
        &self,
        actor: &ActorOwner,
        context: &super::Context,
        now: u64,
    ) -> Result<(), VoiceError> {
        if !Arc::ptr_eq(&self.actor, &actor.gate)
            || self.epoch != context.epoch
            || !self.member.live.is_live()
            || !self.policy.is_live()
            || now >= self.deadline
        {
            return Err(VoiceError::Unauthorized);
        }
        actor.current()?;
        if self
            .original_request
            .as_ref()
            .is_some_and(|original| !original.authorized())
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
}
impl ActorOwner {
    /// Native ingress and response verification share process work and Crypto capacity with
    /// the actual WebCrypto engine. This provisional128/s burst128 process ceiling is retained
    /// across group/controller replacement; measurements must precede live feature acceptance.
    pub(crate) fn admit_transport_verification(
        &self,
        requester: DeviceId,
        frame: &[u8],
    ) -> Result<IngressTicket, VoiceError> {
        self.admit_transport_verification_in_progress(requester, frame, None, false)
    }
    /// Only a SAME-job native retained envelope supplies progress. An untrusted envelope
    /// can select an existing SDP loan through the bounded peek, never a confirmation owner.
    pub(crate) fn admit_transport_verification_in_progress(
        &self,
        requester: DeviceId,
        frame: &[u8],
        retained_progress: Option<super::source::SourceProgress<'_>>,
        trusted_completion: bool,
    ) -> Result<IngressTicket, VoiceError> {
        if frame.is_empty() || frame.len() > super::MAX_ENVELOPE {
            return Err(VoiceError::Bounds);
        }
        self.current()?;
        // Clock is an injected callback. Sample before either native guard, then acquire
        // without waiting so a busy view cannot age this admission/expiry observation.
        let wall = self.clock.now_ms();
        let now = self.clock.monotonic_ms();
        self.current()?;
        let mut state = match self.group.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        let policy = match self.gate.policy.try_lock() {
            Ok(policy) => policy,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        if state.revision != self.revision || !policy.eligible || !policy.live.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let member = state
            .members
            .get(&requester)
            .cloned()
            .ok_or(VoiceError::Unauthorized)?;
        let epoch = state.epoch.ok_or(VoiceError::Unauthorized)?;
        self.current()?;
        let bytes = VERIFICATION_BUFFERS * frame.len() + VERIFICATION_OVERHEAD;
        let current_exchange = self.resources.exchange(self.group.hash);
        let peeked = if retained_progress.is_none() {
            current_exchange.filter(|exchange| exchange.lends_verification(requester, frame))
        } else {
            None
        };
        let ready_receive = if retained_progress.is_none() && peeked.is_none() {
            self.resources.exchange(self.group.hash).filter(|exchange| {
                exchange.lends_ready_offer_verification(requester, frame, wall, now)
            })
        } else {
            None
        };
        let progress = retained_progress.unwrap_or_else(|| {
            ready_receive.as_ref().map_or_else(
                || {
                    peeked.as_ref().map_or(
                        super::source::SourceProgress::Ordinary,
                        super::source::SourceProgress::Exchange,
                    )
                },
                super::source::SourceProgress::ReadyReceive,
            )
        });
        progress.require_current(&self.resources, self.group.hash)?;
        let allocate = || {
            progress.charge(
                &self.resources,
                self.group.hash,
                bytes,
                super::negotiation::Stage::Crypto,
            )
        };
        // A retained affine completion has already paid for its immutable request and lower
        // custody. Polling its own busy lane must not spend requester/process tokens repeatedly.
        // Untrusted fresh ingress still pays both cheap work budgets BEFORE borrowing capacity.
        let early_credit = if trusted_completion {
            Some(allocate()?)
        } else {
            None
        };
        state.limits.requester(requester, now)?;
        if !self
            .group
            .verification
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .spend(now, 128, 128)
        {
            return Err(VoiceError::RateLimited);
        }
        let credit = match early_credit {
            Some(credit) => credit,
            None => allocate()?,
        };
        self.current()?;
        Ok(IngressTicket {
            actor: self.gate.clone(),
            member,
            policy: policy.live.clone(),
            epoch,
            requester,
            hash: *blake3::hash(frame).as_bytes(),
            bytes: frame.len(),
            deadline: now.saturating_add(5_000),
            clock: self.clock.clone(),
            original_request: None,
            _credit: Arc::new(credit),
        })
    }
    /// Only a matching affine ingress token can replace the normal requester debit. The caller
    /// keeps that token through decode, route classification and final persistent admission.
    pub(crate) fn authorize_verified_route(
        &self,
        ticket: &IngressTicket,
        envelope: &super::Envelope,
        context: &super::Context,
    ) -> Result<super::Disposition, VoiceError> {
        ticket.check(self, &envelope.encode())?;
        self.route_operation(ticket.requester(), envelope, context, false, |_, _, _| {
            Ok(())
        })
        .map(|(disposition, ())| disposition)
    }
}

/// Immutable observing gates only. Driver checks never take Group/Policy/Root locks and cannot
/// refresh membership or an expired request. Credit remains independently held after revocation.
pub(crate) struct VerificationReplyAuthority {
    actor: Arc<ActorGate>,
    member: Arc<Member>,
    policy: CallLease,
    clock: Arc<EffectiveClock>,
    deadline: u64,
}
impl VerificationReplyAuthority {
    /// All reply currency checks share this original native sample. Sample callbacks must run
    /// BEFORE observing any irreversible gate, including other members/room/recipient gates.
    pub(crate) fn sample_time(&self) -> (u64, u64) {
        (self.clock.now_ms(), self.clock.monotonic_ms())
    }
    pub(crate) fn authorized_at(&self, mono: u64) -> bool {
        mono < self.deadline
            && self.actor.authorized()
            && self.member.live.is_live()
            && self.policy.is_live()
    }
}
redacted_debug!(VerificationReplyAuthority);
#[async_trait::async_trait]
impl RequestAuthority for VerificationReplyAuthority {
    fn authorized(&self) -> bool {
        let mono = self.clock.monotonic_ms();
        self.authorized_at(mono)
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let waits = vec![
            self.actor.live.wait_revoked().boxed(),
            self.actor.session.live.wait_revoked().boxed(),
            self.member.live.wait_revoked().boxed(),
            self.policy.wait_revoked().boxed(),
            self.clock
                .sleep(std::time::Duration::from_millis(
                    self.deadline.saturating_sub(self.clock.monotonic_ms()),
                ))
                .boxed(),
        ];
        futures::future::select_all(waits).await;
    }
}
impl EffectAuthority {
    /// Composite lower work samples every owner's ORIGINAL time source before observing any
    /// irreversible gates. Equal wall values do not prove equal monotonic clock origins.
    pub(crate) fn observation_time(&self) -> (u64, u64) {
        (self.clock.now_ms(), self.clock.monotonic_ms())
    }
    /// Final transactions already sampled native time before their short guards. Check those samples
    /// and irreversible gates without reentering the Clock seam or any runtime lock.
    pub(crate) fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        self.actor.authorized()
            && self.origin.live.is_live()
            && self.target.live.is_live()
            && self.policy.is_live()
            && wall <= self.expires.saturating_add(CLOCK_SKEW_MS)
            && mono < self.deadline
    }
}
#[async_trait::async_trait]
impl RequestAuthority for EffectAuthority {
    fn authorized(&self) -> bool {
        self.authorized_at(self.clock.now_ms(), self.clock.monotonic_ms())
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let gates = [
            &self.actor.live,
            &self.actor.session.live,
            &self.origin.live,
            &self.target.live,
            &self.policy,
        ];
        let mut waits = gates
            .iter()
            .map(|gate| gate.wait_revoked().boxed())
            .collect::<Vec<_>>();
        waits.push(self.clock.sleep(std::time::Duration::from_millis(
            self.deadline.saturating_sub(self.clock.monotonic_ms()),
        )));
        futures::future::select_all(waits).await;
    }
}

#[cfg(test)]
mod tests;
