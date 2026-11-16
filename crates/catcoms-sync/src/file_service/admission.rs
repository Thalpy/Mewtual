//! Process-stable file-service replay and execution policy. Native ingress must first establish
//! CURRENT membership/Noise binding; a signature below does not create either. Preflight reserves
//! nothing: acquire actual chain/source capacity, then commit rechecks all policy atomically before
//! lower handoff. Intentionally spent precrypto ingress charges are independent of work debits.
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use catcoms_crypto::DeviceId;
use catcoms_rt::{
    Clock, PeerId, SystemClock, FILE_SERVICE_REQUEST_MAX_BYTES, FILE_SERVICE_RESPONSE_MAX_BYTES,
};
use thiserror::Error;

use super::wire::{Delivery, Grant};

const FRESH_MS: u64 = 20_000;
const REPLAY_MS: u64 = 40_000;
const EFFECT_MS: u64 = 8_000;
const WINDOW_MS: u64 = 1_000;
const MAX_REPLAY_GROUP: usize = 1024;
const MAX_REPLAY_PROCESS: usize = 4096;
const MAX_DEVICES: usize = 1024;
const MAX_GROUPS: usize = 64;
const MIB: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Helper,
    Source,
}

/// Native current endpoint evidence, never reconstructed from a remote claim. The ingress
/// caller owns proof/descriptor/connection-incarnation and membership checks plus revocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServicePeer {
    pub device: DeviceId,
    pub peer: PeerId,
}
pub struct ServiceScope<'a> {
    pub group: &'a [u8],
    pub epoch: u64,
    pub local: ServicePeer,
}

/// Native-only policy observations. Remote refusal must not expose these as a signature/replay
/// oracle; map them to the protocol's bounded unavailable/refused behavior at the service edge.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdmissionError {
    #[error("file service input exceeds its bound")]
    Bounds,
    #[error("file service scope does not match")]
    Scope,
    #[error("file service frame is malformed or unsigned")]
    Signature,
    #[error("file service clock is unavailable")]
    Clock,
    #[error("file service request has expired")]
    Expired,
    #[error("file service request is outside its freshness window")]
    Unfresh,
    #[error("file service capacity is in use")]
    Busy,
    #[error("file service request was already admitted")]
    Replay,
    #[error("file service request nonce was reused with different bytes")]
    NonceChanged,
}

#[derive(Clone, Copy)]
struct Time {
    mono: u64,
    wall: u64,
}
#[derive(Default)]
struct TimeFloor {
    previous: Option<Time>,
    poisoned: bool,
}
impl TimeFloor {
    fn observe(&mut self, clock: &dyn Clock) -> Result<Time, AdmissionError> {
        if self.poisoned {
            return Err(AdmissionError::Clock);
        }
        // Called ONLY under the state mutex. Sampling before locking would let healthy concurrent
        // samples enter in reverse order and falsely poison the entire process on "rollback".
        let mono = clock.monotonic_ms();
        let wall = clock.now_ms();
        let effective = match self.previous {
            None => Some(wall),
            Some(old) => mono
                .checked_sub(old.mono)
                .and_then(|elapsed| old.wall.checked_add(elapsed))
                .map(|floor| floor.max(wall)),
        };
        let Some(wall) = effective else {
            self.poisoned = true;
            return Err(AdmissionError::Clock);
        };
        let time = Time { mono, wall };
        self.previous = Some(time);
        Ok(time)
    }
}
#[derive(Clone, Copy)]
struct Window {
    until: u64,
    used: u64,
}
struct Windows<K> {
    rows: BTreeMap<K, Window>,
    cap: usize,
    limit: u64,
}
impl<K: Ord> Windows<K> {
    fn new(cap: usize, limit: u64) -> Self {
        Self {
            rows: BTreeMap::new(),
            cap,
            limit,
        }
    }
    fn prune(&mut self, now: u64) {
        self.rows.retain(|_, row| row.until > now);
    }
    fn allows(&self, key: &K, amount: u64) -> bool {
        if let Some(row) = self.rows.get(key) {
            row.used
                .checked_add(amount)
                .is_some_and(|total| total <= self.limit)
        } else {
            self.rows.len() < self.cap && amount <= self.limit
        }
    }
    fn charge(&mut self, key: K, amount: u64, until: u64) {
        // Every caller stages all touched rows before mutating any. No callbacks/awaits occur
        // between validation and these bounded infallible counter updates.
        self.rows
            .entry(key)
            .or_insert(Window { until, used: 0 })
            .used += amount;
    }
}
type Group = Box<[u8]>;
type DeviceKey = (Group, DeviceId);
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ReplayKey {
    group: Group,
    local: DeviceId,
    role: Role,
    original: DeviceId,
    nonce: [u8; 16],
}
struct ReplayRow {
    hash: [u8; 32],
    until: u64,
}
struct State {
    time: TimeFloor,
    ingress_device: Windows<DeviceKey>,
    ingress_group: Windows<Group>,
    ingress_process: Windows<()>,
    work_device: Windows<DeviceKey>,
    work_group: Windows<Group>,
    work_process: Windows<()>,
    replay: BTreeMap<ReplayKey, ReplayRow>,
}
impl State {
    fn new() -> Self {
        Self {
            time: TimeFloor::default(),
            ingress_device: Windows::new(MAX_DEVICES, 8),
            ingress_group: Windows::new(MAX_GROUPS, 16),
            ingress_process: Windows::new(1, 32),
            work_device: Windows::new(MAX_DEVICES, 64 * MIB),
            work_group: Windows::new(MAX_GROUPS, 128 * MIB),
            work_process: Windows::new(1, 256 * MIB),
            replay: BTreeMap::new(),
        }
    }
    fn observe(&mut self, clock: &dyn Clock) -> Result<Time, AdmissionError> {
        let time = self.time.observe(clock)?;
        self.ingress_device.prune(time.mono);
        self.ingress_group.prune(time.mono);
        self.ingress_process.prune(time.mono);
        self.work_device.prune(time.mono);
        self.work_group.prune(time.mono);
        self.work_process.prune(time.mono);
        // Strict <20s freshness plus a monotonic-progressing wall floor makes40s safe even for
        // the most future-dated accepted grant. Never evict live rows for fresh identities.
        self.replay.retain(|_, row| row.until > time.mono);
        Ok(time)
    }
}

/// A charged precrypto request, bound to one native scope/sender and one original deadline.
/// No Clone/IPC/serialization API exists; signature validation consumes this ticket once.
pub struct IngressTicket {
    instance: Arc<()>,
    group: Group,
    epoch: u64,
    local: ServicePeer,
    sender: ServicePeer,
    role: Role,
    deadline: u64,
    frame_hash: [u8; 32],
}
/// An immutable cryptographically verified original grant, not current membership or effect
/// authority. The native owner must validate current C/B/route/scope before resource acquisition
/// and again at signing/send. Retaining this object cannot keep an unlock/Root owner alive.
pub struct VerifiedGrant {
    ticket: IngressTicket,
    original: DeviceId,
    helper: DeviceId,
    nonce: [u8; 16],
    hash: [u8; 32],
    created_ms: u64,
    work_bytes: u64,
    original_frame: Box<[u8]>,
}
impl VerifiedGrant {
    pub fn grant(&self) -> Grant<'_> {
        Grant::parse(&self.original_frame).expect("verified immutable frame")
    }
    pub fn role(&self) -> Role {
        self.ticket.role
    }
    pub fn group(&self) -> &[u8] {
        &self.ticket.group
    }
    pub fn local(&self) -> ServicePeer {
        self.ticket.local
    }
    pub fn sender(&self) -> ServicePeer {
        self.ticket.sender
    }
    /// Absolute monotonic deadline in AdmissionRuntime::clock's domain, never renewed by retry.
    pub fn deadline(&self) -> u64 {
        self.ticket.deadline
    }
    fn replay_key(&self) -> ReplayKey {
        ReplayKey {
            group: self.ticket.group.clone(),
            local: self.ticket.local.device,
            role: self.ticket.role,
            original: self.original,
            nonce: self.nonce,
        }
    }
    fn devices(&self) -> ([DeviceId; 2], usize) {
        // A charges original C AND immediate helper B; group/process count actual work ONCE.
        // B's own helper service charges original C, with its local aggregate hosting allowance.
        (
            [self.original, self.helper],
            if self.role() == Role::Source { 2 } else { 1 },
        )
    }
}

/// Stable native process ledger and clock. Reopening a Root, replacing executors or removal/
/// readmission cannot refill it. Only process restart loses these NON-durable service rows;
/// durable origin Keep allowances remain a separate storage contract.
pub struct AdmissionRuntime {
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    instance: Arc<()>,
}
impl AdmissionRuntime {
    #[cfg(test)]
    pub(super) fn testing(clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self::new(clock))
    }

    fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            state: Mutex::new(State::new()),
            instance: Arc::new(()),
        }
    }
    /// The sole production constructor, with the sanctioned native clock. Deterministic tests
    /// inject an isolated clock through the private constructor; no native attachment can do so.
    pub fn process() -> Arc<Self> {
        static PROCESS: OnceLock<Arc<AdmissionRuntime>> = OnceLock::new();
        PROCESS
            .get_or_init(|| Arc::new(Self::new(Arc::new(SystemClock))))
            .clone()
    }
    pub fn clock(&self) -> Arc<dyn Clock> {
        self.clock.clone()
    }
    /// Issue local grants on the same effective wall timeline used for freshness checks. A large
    /// wall correction can leave this process ahead of other devices; service fails closed for
    /// mismatching timestamps rather than reopening old nonces. This does not change OS time.
    pub fn request_time_ms(&self) -> Result<u64, AdmissionError> {
        self.now().map(|time| time.wall)
    }
    fn state(&self) -> Result<std::sync::MutexGuard<'_, State>, AdmissionError> {
        // A panic inside admission must not silently recover a possibly partial policy debit.
        self.state.lock().map_err(|_| AdmissionError::Clock)
    }
    fn now(&self) -> Result<Time, AdmissionError> {
        self.state()?.observe(self.clock.as_ref())
    }
    /// Charge the actual CURRENT bound Noise sender before expensive signatures. The supplied
    /// native scope/identity MUST NOT come from claimed remote fields. Invalid frames and misses
    /// still spend this finite allowance; an original C claim cannot burn C's work quota yet.
    pub fn admit_ingress(
        &self,
        scope: ServiceScope<'_>,
        sender: ServicePeer,
        role: Role,
        frame: &[u8],
    ) -> Result<IngressTicket, AdmissionError> {
        if scope.group.is_empty()
            || scope.group.len() > 256
            || frame.len() > FILE_SERVICE_REQUEST_MAX_BYTES
            || frame.is_empty()
        {
            return Err(AdmissionError::Bounds);
        }
        if scope.local.device == sender.device || scope.local.peer == sender.peer {
            return Err(AdmissionError::Scope);
        }
        let mut state = self.state()?;
        let now = state.observe(self.clock.as_ref())?;
        let until = now
            .mono
            .checked_add(WINDOW_MS)
            .ok_or(AdmissionError::Clock)?;
        let deadline = now
            .mono
            .checked_add(EFFECT_MS)
            .ok_or(AdmissionError::Clock)?;
        let group: Group = scope.group.into();
        let device = (group.clone(), sender.device);
        if !state.ingress_device.allows(&device, 1)
            || !state.ingress_group.allows(&group, 1)
            || !state.ingress_process.allows(&(), 1)
        {
            return Err(AdmissionError::Busy);
        }
        state.ingress_device.charge(device, 1, until);
        state.ingress_group.charge(group.clone(), 1, until);
        state.ingress_process.charge((), 1, until);
        Ok(IngressTicket {
            instance: self.instance.clone(),
            group,
            epoch: scope.epoch,
            local: scope.local,
            sender,
            role,
            deadline,
            frame_hash: *blake3::hash(frame).as_bytes(),
        })
    }
    /// Consume one charged ticket and verify its fixed phase's signatures outside the mutex.
    /// Scope/declared peers must equal native ingress evidence; forwarded C never gains standing.
    pub fn verify(
        &self,
        ticket: IngressTicket,
        frame: &[u8],
    ) -> Result<VerifiedGrant, AdmissionError> {
        if !Arc::ptr_eq(&ticket.instance, &self.instance) {
            return Err(AdmissionError::Scope);
        }
        if frame.len() > FILE_SERVICE_REQUEST_MAX_BYTES {
            return Err(AdmissionError::Bounds);
        }
        if *blake3::hash(frame).as_bytes() != ticket.frame_hash {
            return Err(AdmissionError::Scope);
        }
        let now = self.now()?;
        if now.mono >= ticket.deadline {
            return Err(AdmissionError::Expired);
        }
        let (grant, delivery) = match ticket.role {
            Role::Helper => (
                Grant::parse(frame).map_err(|_| AdmissionError::Signature)?,
                None,
            ),
            Role::Source => {
                let delivery = Delivery::parse(frame).map_err(|_| AdmissionError::Signature)?;
                if delivery.source() != ticket.local.device
                    || delivery.source_peer() != ticket.local.peer
                {
                    return Err(AdmissionError::Scope);
                }
                (
                    Grant::parse(delivery.grant().raw()).map_err(|_| AdmissionError::Signature)?,
                    Some(delivery),
                )
            }
        };
        if grant.group() != ticket.group.as_ref() || grant.epoch() != ticket.epoch {
            return Err(AdmissionError::Scope);
        }
        let role_matches = match ticket.role {
            Role::Helper => {
                grant.requester() == ticket.sender.device
                    && grant.requester_peer() == ticket.sender.peer
                    && grant.helper() == ticket.local.device
                    && grant.helper_peer() == ticket.local.peer
            }
            Role::Source => {
                grant.helper() == ticket.sender.device && grant.helper_peer() == ticket.sender.peer
            }
        };
        if !role_matches {
            return Err(AdmissionError::Scope);
        }
        if now.wall.abs_diff(grant.created_ms()) >= FRESH_MS {
            return Err(AdmissionError::Unfresh);
        }
        if delivery
            .as_ref()
            .is_some_and(|frame| !frame.verify_signature())
            || !grant.verify_signature()
        {
            return Err(AdmissionError::Signature);
        }
        let work_bytes = match ticket.role {
            Role::Helper => {
                4 * FILE_SERVICE_REQUEST_MAX_BYTES as u64
                    + 5 * FILE_SERVICE_RESPONSE_MAX_BYTES as u64
            }
            // No cheap-cache admission exists. Every source read pays its whole declared bound.
            Role::Source => {
                grant.window().max_cipher_bytes as u64 + FILE_SERVICE_RESPONSE_MAX_BYTES as u64
            }
        };
        Ok(VerifiedGrant {
            original: grant.requester(),
            helper: grant.helper(),
            nonce: grant.nonce(),
            hash: grant.hash(),
            created_ms: grant.created_ms(),
            original_frame: grant.raw().into(),
            ticket,
            work_bytes,
        })
    }
    fn check(&self, state: &State, proof: &VerifiedGrant, now: Time) -> Result<(), AdmissionError> {
        if !Arc::ptr_eq(&proof.ticket.instance, &self.instance) {
            return Err(AdmissionError::Scope);
        }
        if now.mono >= proof.deadline() {
            return Err(AdmissionError::Expired);
        }
        if now.wall.abs_diff(proof.created_ms) >= FRESH_MS {
            return Err(AdmissionError::Unfresh);
        }
        let key = proof.replay_key();
        if let Some(row) = state.replay.get(&key) {
            return Err(if row.hash == proof.hash {
                AdmissionError::Replay
            } else {
                AdmissionError::NonceChanged
            });
        }
        if state.replay.len() >= MAX_REPLAY_PROCESS
            || state
                .replay
                .keys()
                .filter(|key| key.group.as_ref() == proof.group())
                .count()
                >= MAX_REPLAY_GROUP
        {
            return Err(AdmissionError::Busy);
        }
        let (devices, count) = proof.devices();
        let mut missing = 0;
        for device in &devices[..count] {
            let key = (proof.ticket.group.clone(), *device);
            if !state.work_device.allows(&key, proof.work_bytes) {
                return Err(AdmissionError::Busy);
            }
            if !state.work_device.rows.contains_key(&key) {
                missing += 1;
            }
        }
        if state.work_device.rows.len() + missing > MAX_DEVICES
            || !state
                .work_group
                .allows(&proof.ticket.group, proof.work_bytes)
            || !state.work_process.allows(&(), proof.work_bytes)
        {
            return Err(AdmissionError::Busy);
        }
        now.mono
            .checked_add(REPLAY_MS)
            .ok_or(AdmissionError::Clock)?;
        now.mono
            .checked_add(WINDOW_MS)
            .ok_or(AdmissionError::Clock)?;
        Ok(())
    }
    /// Check before preparing actual capacity/storage. This is NOT a reservation or permission
    /// to submit; concurrent admission can change any result before commit's mandatory recheck.
    pub fn preflight(&self, proof: &VerifiedGrant) -> Result<(), AdmissionError> {
        let mut state = self.state()?;
        let now = state.observe(self.clock.as_ref())?;
        self.check(&state, proof, now)
    }
    /// Call only after nonblocking actual chain/source reservation and current native authority
    /// checks, BEFORE handoff. Atomically publish the nonce and ALL work debits, or none. A refusal
    /// drops only demonstrably unstarted capacity. Cancellation after commit grants no refund.
    pub fn commit(&self, proof: &VerifiedGrant) -> Result<(), AdmissionError> {
        let mut state = self.state()?;
        let now = state.observe(self.clock.as_ref())?;
        self.check(&state, proof, now)?;
        let until = now.mono + WINDOW_MS;
        let (devices, count) = proof.devices();
        for device in &devices[..count] {
            state.work_device.charge(
                (proof.ticket.group.clone(), *device),
                proof.work_bytes,
                until,
            );
        }
        state
            .work_group
            .charge(proof.ticket.group.clone(), proof.work_bytes, until);
        state.work_process.charge((), proof.work_bytes, until);
        state.replay.insert(
            proof.replay_key(),
            ReplayRow {
                hash: proof.hash,
                until: now.mono + REPLAY_MS,
            },
        );
        Ok(())
    }
}

impl fmt::Debug for ServiceScope<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServiceScope { .. }")
    }
}
impl fmt::Debug for AdmissionRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AdmissionRuntime { .. }")
    }
}
impl fmt::Debug for IngressTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IngressTicket { .. }")
    }
}
impl fmt::Debug for VerifiedGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VerifiedGrant { .. }")
    }
}

#[cfg(test)]
mod tests;
