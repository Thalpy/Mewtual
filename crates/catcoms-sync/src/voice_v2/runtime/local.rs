//! Local confirmation publication is distinct from authenticated incoming packet admission.
//! No signed packet exists yet: neither an IngressTicket nor an incoming-origin debit may be
//! invented here. The actual signed source submission still performs its own quota admission.
use super::*;

/// Small pure actor cell, separately allocated inside the original actor's payment. Historical
/// observers retain this cell rather than Weak<ActorGate>: a Weak would strand that entire
/// larger allocation after its own credit dropped. No Group, Source, key or Workflow lives here.
pub(super) struct EpochState {
    pub(super) epoch: AtomicU64,
    pub(super) live: AtomicBool,
}

/// Non-authorizing original actor/session/policy observations. Their small complete Arc bodies
/// and headers fit in the original PlanBinding payment; none contains a watch or weak Gate.
pub(in crate::voice_v2) struct EpochObservation {
    actor: Arc<EpochState>,
    session: Arc<AtomicBool>,
    policy: Arc<AtomicBool>,
    epoch: u64,
}
impl EpochObservation {
    pub(in crate::voice_v2) fn is_live(&self) -> bool {
        self.actor.live.load(Ordering::Acquire)
            && self.session.load(Ordering::Acquire)
            && self.policy.load(Ordering::Acquire)
            && self.actor.epoch.load(Ordering::Acquire) == self.epoch
    }
    pub(in crate::voice_v2) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.actor, &other.actor)
            && Arc::ptr_eq(&self.session, &other.session)
            && Arc::ptr_eq(&self.policy, &other.policy)
            && self.epoch == other.epoch
    }
    /// Complete independently retained allocations, including rounded Arc headers. The
    /// enclosing observer value itself is already included by size_of<PlanBinding>.
    #[cfg(test)]
    pub(in crate::voice_v2) const fn retained_graph_bytes() -> usize {
        std::mem::size_of::<(usize, usize, EpochState)>()
            + 2 * std::mem::size_of::<(usize, usize, AtomicBool)>()
    }
}

/// One original Actor clock observation, captured before Engine/native guards. This is
/// stack-only preparation, not authority; consumption repeats current route/session/view
/// under nonblocking Group/Policy fences and never invokes a Clock or arbitrary provider.
pub(in crate::voice_v2) struct PreparedLocalOrigin<'a> {
    actor: &'a ActorOwner,
    peer: DeviceId,
    context: &'a super::super::Context,
    wall: u64,
    mono: u64,
}
impl PreparedLocalOrigin<'_> {
    pub(in crate::voice_v2) fn with_origin(
        self,
        publish: impl FnOnce(u64, u64, LocalOrigin<'_>) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        self.actor
            .with_local_origin_at(self.peer, self.context, self.wall, self.mono, publish)
    }
}

impl ActorOwner {
    /// Preserve the exact native monotonic origin that stamped a Ready. A later Source
    /// ledger may use another clock domain; callers must convert durations, not deadlines.
    pub(in crate::voice_v2) fn preparation_clock(&self) -> Arc<dyn Clock> {
        self.clock.clone()
    }
    /// Native preparation clock only, outside every admission/Engine guard. This observation
    /// carries no authority; actual final publication must freshly enter the native view.
    pub(in crate::voice_v2) fn preparation_time(&self) -> Result<(u64, u64), VoiceError> {
        self.current()?;
        let wall = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        self.current()?;
        Ok((wall, mono))
    }
    /// Capture only while preparing, before Engine/final publication. Later observations use
    /// atomics only; neither expiry collection nor a final Group/Policy closure reenters locks.
    pub(in crate::voice_v2) fn observe_epoch(
        &self,
        context: &super::super::Context,
    ) -> Result<EpochObservation, VoiceError> {
        self.current()?;
        let group = self.group.state.lock().unwrap_or_else(|e| e.into_inner());
        let policy = self.gate.policy.lock().unwrap_or_else(|e| e.into_inner());
        self.check_route_view(&group, &policy, context.local, context)?;
        Ok(EpochObservation {
            actor: self.gate.observation.clone(),
            session: self.gate.session.observation.clone(),
            policy: policy.observation.clone(),
            epoch: context.epoch,
        })
    }
}

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
    /// Pure membership observation while the actual original Group/Policy guards are held.
    /// This cannot prepay rows or grant helper forwarding; native Ready must do that separately.
    pub(in crate::voice_v2) fn contains_members(&self, members: &[DeviceId]) -> bool {
        members
            .iter()
            .all(|member| self._group.members.contains_key(member))
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
    /// Pay the existing endpoint/helper origin rows before promising Ready progress. Native
    /// membership/policy are read under their actual guards; a caller's member predicate is
    /// insufficient. This resets no traffic bucket and creates no forwarding/Ready authority.
    /// Partial payment may leave ordinary bounded rows, as for pair-pin preparation.
    pub(in crate::voice_v2) fn prepare_ready_members(
        &self,
        peer: DeviceId,
        helpers: &[DeviceId],
        context: &super::super::Context,
    ) -> Result<(), VoiceError> {
        if helpers.len() > 2 {
            return Err(VoiceError::Bounds);
        }
        if helpers.windows(2).any(|pair| pair[0] >= pair[1])
            || helpers
                .iter()
                .any(|helper| *helper == context.local || *helper == peer)
        {
            return Err(VoiceError::Malformed);
        }
        self.current()?;
        // Injected time may revoke/reenter native owners. Never sample under either guard
        // or block acquiring those guards. Metadata payment may enter Pool here; this is
        // preparation only. Final publication must sample afresh after releasing all guards.
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
        self.check_route_view(&state, &policy, peer, context)?;
        if peer == context.local
            || !state.members.contains_key(&context.local)
            || (!helpers.is_empty() && (!policy.indirect || policy.pin.is_none()))
            || helpers
                .iter()
                .any(|helper| !state.members.contains_key(helper))
        {
            return Err(VoiceError::Unauthorized);
        }
        let mut members = [context.local, peer, context.local, context.local];
        members[2..2 + helpers.len()].copy_from_slice(helpers);
        state
            .limits
            .prepare_exchange_members(&members[..2 + helpers.len()], &self.resources, mono)
    }

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
        self.prepare_local_origin(peer, context)?
            .with_origin(publish)
    }

    /// Capture the actual Actor origin before an outer Engine fence. Callers must prepare
    /// all fallible bodies/signatures first and consume this synchronously without awaiting.
    pub(in crate::voice_v2) fn prepare_local_origin<'a>(
        &'a self,
        peer: DeviceId,
        context: &'a super::super::Context,
    ) -> Result<PreparedLocalOrigin<'a>, VoiceError> {
        self.current()?;
        let wall = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        Ok(PreparedLocalOrigin {
            actor: self,
            peer,
            context,
            wall,
            mono,
        })
    }

    fn with_local_origin_at(
        &self,
        peer: DeviceId,
        context: &super::super::Context,
        wall: u64,
        mono: u64,
        publish: impl FnOnce(u64, u64, LocalOrigin<'_>) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
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
