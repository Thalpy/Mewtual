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
    // Protected sealing observes three independent irreversible owners: original packet,
    // nonce/action attempt, and confirmation chain. None can stand in for another. Hello
    // opening uses at most two leases plus its accepted-sequence gate, within the same bound.
    pending: [Option<CallLease>; 3],
    hello: Option<SequenceAuthority>,
    clock: Arc<dyn Clock>,
    deadline: u64,
    expires: u64,
}
redacted_debug!(CryptoAuthority);
impl CryptoAuthority {
    /// Sample this authority's OWN monotonic origin before a short native fence. Later native
    /// callbacks may revoke any gate, so repeat authorized_at only after their final callback.
    pub(super) fn observation_time(&self) -> (u64, u64) {
        (self.clock.monotonic_ms(), self.clock.now_ms())
    }
    pub(super) fn authorized_at(&self, mono: u64, wall: u64) -> bool {
        self.local.is_live()
            && self.pair.is_live()
            && self.pending.iter().flatten().all(CallLease::is_live)
            && self
                .hello
                .as_ref()
                .is_none_or(SequenceAuthority::authorized)
            && mono < self.deadline
            && wall <= self.expires
    }
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
            pending: [None, None, None],
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
            .take(if self.hello.is_some() { 2 } else { 3 })
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
        if self.pending[2].is_some() || self.hello.is_some() {
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
        // Clock is an injected callback and may synchronously close an owner. Observe all
        // original irreversible gates AFTER the final callback, not only on the next poll.
        let (mono, wall) = self.observation_time();
        self.authorized_at(mono, wall)
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

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::ManualClock;

    #[derive(Debug)]
    struct LastClockRevocation {
        inner: ManualClock,
        target: CallLease,
        armed: std::sync::atomic::AtomicBool,
    }
    impl Clock for LastClockRevocation {
        fn now_ms(&self) -> u64 {
            if self.armed.swap(false, Ordering::AcqRel) {
                self.target.revoke();
            }
            self.inner.now_ms()
        }
        fn monotonic_ms(&self) -> u64 {
            self.inner.monotonic_ms()
        }
        fn sleep(
            &self,
            duration: std::time::Duration,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
            self.inner.sleep(duration)
        }
    }

    #[test]
    fn final_pure_pending_observation_never_resamples_clock_and_sees_later_revocation() {
        for index in 0..5 {
            let gates: [CallLease; 5] = std::array::from_fn(|_| CallLease::default());
            let clock = Arc::new(LastClockRevocation {
                inner: ManualClock::new(1000),
                target: gates[index].clone(),
                armed: std::sync::atomic::AtomicBool::new(false),
            });
            let mut authority = CryptoAuthority::new(
                gates[0].clone(),
                gates[1].clone(),
                clock.clone(),
                16_000,
                u64::MAX,
            )
            .unwrap();
            for gate in gates.iter().skip(2) {
                authority = authority.with_pending(gate.clone()).unwrap();
            }
            let (mono, wall) = authority.observation_time();
            clock.armed.store(true, Ordering::Release);
            assert!(authority.authorized_at(mono, wall));
            assert!(
                clock.armed.load(Ordering::Acquire),
                "pure fence must not invoke Clock"
            );
            assert!(gates[index].is_live());
            gates[index].revoke();
            assert!(!authority.authorized_at(mono, wall));
            assert!(clock.armed.load(Ordering::Acquire));
        }
    }

    #[test]
    fn crypto_authority_observes_each_owner_revoked_by_the_last_clock_callback_immediately() {
        for index in 0..5 {
            let gates: [CallLease; 5] = std::array::from_fn(|_| CallLease::default());
            let clock = Arc::new(LastClockRevocation {
                inner: ManualClock::new(1000),
                target: gates[index].clone(),
                armed: std::sync::atomic::AtomicBool::new(false),
            });
            let mut authority = CryptoAuthority::new(
                gates[0].clone(),
                gates[1].clone(),
                clock.clone(),
                15_000,
                16_000,
            )
            .unwrap();
            for gate in &gates[2..] {
                authority = authority.with_pending(gate.clone()).unwrap();
            }
            clock.armed.store(true, Ordering::Release);
            assert!(
                !authority.authorized(),
                "owner {index} closes during this observation"
            );
        }
    }

    #[test]
    fn protected_seal_keeps_three_independent_gates_and_refuses_a_fourth() {
        for retired in 0..3 {
            let gates: [CallLease; 3] = std::array::from_fn(|_| CallLease::default());
            let mut authority = CryptoAuthority::new(
                CallLease::default(),
                CallLease::default(),
                Arc::new(ManualClock::new(1000)),
                15_000,
                16_000,
            )
            .unwrap();
            for gate in &gates {
                authority = authority.with_pending(gate.clone()).unwrap();
            }
            assert!(authority.authorized());
            assert_eq!(
                authority.clone().with_pending(CallLease::default()).err(),
                Some(VoiceError::Bounds)
            );
            gates[retired].revoke();
            assert!(!authority.authorized(), "every original owner is mandatory");
        }
    }

    #[test]
    fn hello_sequence_and_pending_leases_share_a_fixed_three_gate_bound_in_both_orders() {
        let base = || {
            CryptoAuthority::new(
                CallLease::default(),
                CallLease::default(),
                Arc::new(ManualClock::new(1000)),
                15_000,
                16_000,
            )
            .unwrap()
        };
        let gate = SequenceGate::default();
        for hello_first in [false, true] {
            let mut authority = base();
            if hello_first {
                authority = authority.with_hello(gate.candidate(1).unwrap()).unwrap();
            }
            authority = authority
                .with_pending(CallLease::default())
                .unwrap()
                .with_pending(CallLease::default())
                .unwrap();
            if !hello_first {
                authority = authority.with_hello(gate.candidate(1).unwrap()).unwrap();
            }
            assert!(authority.authorized());
            assert_eq!(
                authority.with_pending(CallLease::default()).err(),
                Some(VoiceError::Bounds)
            );
        }
        let authority = base()
            .with_pending(CallLease::default())
            .unwrap()
            .with_pending(CallLease::default())
            .unwrap()
            .with_pending(CallLease::default())
            .unwrap();
        assert_eq!(
            authority.with_hello(gate.candidate(1).unwrap()).err(),
            Some(VoiceError::Bounds)
        );
    }
}
