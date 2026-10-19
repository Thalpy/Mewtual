//! Pending browser work gets an opaque native grant for its actual pair/action/token owners.
//! Data in PairSetup/CryptoFlight/InboundPacket is not itself authority. This module deliberately
//! offers no conversion to a historical commit receipt: that requires the post-transition native
//! Endpoint action/negotiation owner and its prepaid exact-result cache.
use super::{CallLease, VoiceError};
use catcoms_rt::{Clock, RequestAuthority};
use futures::FutureExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// One accepted-Hello floor for the whole native pair. A later candidate remains eligible when
/// an earlier candidate commits; an already accepted/higher sequence permanently closes old work.
#[derive(Clone)]
pub(super) struct SequenceGate(Arc<SequenceState>);
struct SequenceState {
    highest: AtomicU64,
    changed: tokio::sync::watch::Sender<u64>,
}
impl Default for SequenceGate {
    fn default() -> Self {
        Self(Arc::new(SequenceState {
            highest: AtomicU64::new(0),
            changed: tokio::sync::watch::channel(0).0,
        }))
    }
}
impl SequenceGate {
    pub(super) fn candidate(&self, sequence: u64) -> Result<SequenceAuthority, VoiceError> {
        let authority = SequenceAuthority {
            gate: self.clone(),
            sequence,
        };
        if !authority.authorized() {
            return Err(VoiceError::Replay);
        }
        Ok(authority)
    }
    pub(super) fn accept(&self, sequence: u64) {
        // Only the sole native confirmation owner calls this AFTER closed validation and the
        // successful action/nonce transition. A signed header or failed AEAD cannot advance it.
        if sequence > self.0.highest.fetch_max(sequence, Ordering::AcqRel) {
            self.0.changed.send_replace(sequence);
        }
    }
}
#[derive(Clone)]
pub(super) struct SequenceAuthority {
    gate: SequenceGate,
    sequence: u64,
}
impl SequenceAuthority {
    pub(super) fn authorized(&self) -> bool {
        self.sequence > self.gate.0.highest.load(Ordering::Acquire)
    }
    async fn revoked(&self) {
        let mut changed = self.gate.0.changed.subscribe();
        // A lower accepted sequence wakes this future but does not revoke a newer candidate.
        // Subscribe first, then recheck the irreversible floor to close the lost-wake window.
        while self.authorized() {
            if changed.changed().await.is_err() {
                return;
            }
        }
    }
}

/// Native pending-work capability. Constructed only by the actual LocalJoin/PacketLedger; a
/// trusted Endpoint still binds it to its current actor/room/action registry before lending it
/// to native engine admission. Clones retain revocation identity, never renew its deadline.
#[derive(Clone)]
pub struct CryptoAuthority {
    local: CallLease,
    pair: CallLease,
    pending: [Option<CallLease>; 2],
    hello: Option<SequenceAuthority>,
    clock: Arc<dyn Clock>,
    deadline: u64,
    expires: u64,
}
redacted_debug!(CryptoAuthority);
impl CryptoAuthority {
    /// Historical results retain the original native clock, independently of pending token
    /// authorization. A retry must not select a new clock or renew any original deadline.
    pub(super) fn history_time(&self) -> (u64, u64) {
        (self.clock.monotonic_ms(), self.clock.now_ms())
    }
    pub(super) fn new(
        local: CallLease,
        pair: CallLease,
        clock: Arc<dyn Clock>,
        deadline: u64,
        expires: u64,
    ) -> Result<Self, VoiceError> {
        let authority = Self {
            local,
            pair,
            pending: [None, None],
            hello: None,
            clock,
            deadline,
            expires,
        };
        if !authority.authorized() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(authority)
    }
    pub(super) fn with_pending(mut self, lease: CallLease) -> Result<Self, VoiceError> {
        let slot = self
            .pending
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(VoiceError::Bounds)?;
        *slot = Some(lease);
        if !self.authorized() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(self)
    }
    pub(super) fn limit_deadline(mut self, deadline: u64) -> Result<Self, VoiceError> {
        self.deadline = self.deadline.min(deadline);
        if !self.authorized() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(self)
    }
    pub(super) fn with_hello(mut self, hello: SequenceAuthority) -> Result<Self, VoiceError> {
        if self.pending[1].is_some() || self.hello.is_some() {
            return Err(VoiceError::Bounds);
        }
        self.hello = Some(hello);
        if !self.authorized() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(self)
    }
}
#[async_trait::async_trait]
impl RequestAuthority for CryptoAuthority {
    fn authorized(&self) -> bool {
        self.local.is_live()
            && self.pair.is_live()
            && self.pending.iter().flatten().all(CallLease::is_live)
            && self
                .hello
                .as_ref()
                .is_none_or(SequenceAuthority::authorized)
            && self.clock.monotonic_ms() < self.deadline
            && self.clock.now_ms() <= self.expires
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        // Subscribe inside each lease wait before reading its irreversible flag. A consume or
        // supersession between native admission and actual engine start therefore wakes too.
        let mut waits = vec![
            self.local.wait_revoked().boxed(),
            self.pair.wait_revoked().boxed(),
        ];
        for lease in self.pending.iter().flatten() {
            waits.push(lease.wait_revoked().boxed());
        }
        if let Some(hello) = &self.hello {
            waits.push(hello.revoked().boxed());
        }
        waits.push(self.clock.sleep(std::time::Duration::from_millis(
            self.deadline.saturating_sub(self.clock.monotonic_ms()),
        )));
        futures::future::select_all(waits).await;
    }
}
