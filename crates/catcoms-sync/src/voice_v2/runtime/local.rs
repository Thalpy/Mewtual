//! Local confirmation publication is distinct from authenticated incoming packet admission.
//! No signed packet exists yet: neither an IngressTicket nor an incoming-origin debit may be
//! invented here. The actual signed source submission still performs its own quota admission.
use super::*;

/// A provisional origin key pin grants neither a pair nor traffic quota. Failed unpublished
/// preparation revokes it without reacquiring Group; ordinary bounded collection prunes the
/// dead clone later. Publication transfers its one live pin to the actual Pair.
pub(crate) struct PairPinReservation {
    pin: CallLease,
    published: bool,
}
impl PairPinReservation {
    pub(crate) fn publish(&mut self) {
        self.published = true;
    }
}
impl Drop for PairPinReservation {
    fn drop(&mut self) {
        if !self.published {
            self.pin.revoke();
        }
    }
}

/// Affine permission to publish already-prepaid local state while the original Group/Policy
/// view remains held. It grants no signing, engine, transport or remote packet admission.
pub(crate) struct LocalOrigin<'a> {
    actor: &'a ActorOwner,
    context: &'a super::super::Context,
    // Lifetime witnesses borrow the ACTUAL held views, rather than only the longer-lived
    // actor/context. Even a future callback API change cannot let this loan outlive its guards.
    _group: &'a GroupState,
    _policy: &'a Policy,
}
impl LocalOrigin<'_> {
    pub(crate) fn epoch(&self) -> u64 {
        self.context.epoch
    }

    /// All pair/room/candidate checks must already have passed. Atomic session revocation may
    /// win before it waits for Policy, so repeat that pure observation immediately before apply.
    pub(crate) fn publish(self, apply: impl FnOnce()) -> Result<(), VoiceError> {
        self.actor.current()?;
        apply();
        Ok(())
    }
}

impl ActorOwner {
    #[cfg(test)]
    pub(crate) fn pair_pin_observer_for_test(
        &self,
        peer: DeviceId,
        join: super::super::Nonce,
        key_ref: [u8; 32],
    ) -> Arc<dyn Fn() -> usize + Send + Sync> {
        let group = self.group.clone();
        Arc::new(move || {
            group
                .state
                .try_lock()
                .expect("inspection outside native Group")
                .limits
                .live_pair_pins_for_test(peer, join, key_ref)
        })
    }

    /// Tests hold the actual original mutex in an independent thread after a Clock barrier.
    /// No replacement policy/group or caller-constructed authority is involved.
    #[cfg(test)]
    pub(crate) fn native_view_holder_for_test(
        &self,
        policy: bool,
        held: std::sync::mpsc::SyncSender<()>,
        released: std::sync::mpsc::Receiver<()>,
    ) -> Box<dyn FnOnce() + Send> {
        let group = self.group.clone();
        let gate = self.gate.clone();
        Box::new(move || {
            let wait = || {
                held.send(()).unwrap();
                let _ = released.recv();
            };
            if policy {
                let _guard = gate.policy.lock().unwrap_or_else(|e| e.into_inner());
                wait();
            } else {
                let _guard = group.state.lock().unwrap_or_else(|e| e.into_inner());
                wait();
            }
        })
    }
    /// Prepay endpoint origin rows and the original key binding's bounded pin position. Samples
    /// precede both try-locks; refusal spends no requester/setup/room allowance. A later failed
    /// pin/publication may leave the normal paid60s rows, never refreshed traffic buckets.
    /// A final native transaction must still admit the pair; this is protective capacity only.
    pub(crate) fn prepare_pair_pin(
        &self,
        binding: &super::super::packets::PairBinding,
        context: &super::super::Context,
        pin: &CallLease,
    ) -> Result<PairPinReservation, VoiceError> {
        let reservation = PairPinReservation {
            pin: pin.clone(),
            published: false,
        };
        self.current()?;
        let _wall = self.clock.now_ms();
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
        self.check_route_view(&state, &policy, binding.peer, context)?;
        if binding.peer == context.local
            || !state.members.contains_key(&context.local)
            || !pin.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        // A protected pair can publish its first signed confirmation before the local
        // Announcement has entered Source. Prepay BOTH existing origin/requester records
        // before accepting that pair, so Source admission and its peer's ACK cannot ask the
        // ordinary pool for a new row after the confirmation envelope was admitted. This
        // creates no traffic allowance, resets no bucket and keeps normal60s row retention.
        state.limits.prepare_exchange_members(
            &[context.local, binding.peer],
            &self.resources,
            mono,
        )?;
        state
            .limits
            .protect_pair(binding.peer, binding.join, binding.key_ref, pin, mono)?;
        Ok(reservation)
    }
    /// A local caller supplies actual native pair/room owners, never an envelope-shaped stand-in
    /// for network ingress. Sample injected time outside every guard and refuse contention so
    /// no blocking acquisition can silently age those samples before final publication.
    pub(crate) fn with_local_origin(
        &self,
        peer: DeviceId,
        context: &super::super::Context,
        publish: impl FnOnce(u64, u64, LocalOrigin<'_>) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        self.current()?;
        let wall = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
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
        self.check_route_view(&state, &policy, peer, context)?;
        if peer == context.local || !state.members.contains_key(&context.local) {
            return Err(VoiceError::Unauthorized);
        }
        // The callback may only try-lock its short History fence or publish an already-prepared
        // result while the caller holds Engine. Never acquire Engine from this callback: that
        // would invert Engine -> Group -> Policy. Unpublished owners stay outside its
        // environment so their destructors run after Group/Policy have been released.
        publish(
            wall,
            mono,
            LocalOrigin {
                actor: self,
                context,
                _group: &state,
                _policy: &policy,
            },
        )
    }
}
