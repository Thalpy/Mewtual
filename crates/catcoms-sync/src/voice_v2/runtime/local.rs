//! Local confirmation publication is distinct from authenticated incoming packet admission.
//! No signed packet exists yet: neither an IngressTicket nor an incoming-origin debit may be
//! invented here. The actual signed source submission still performs its own quota admission.
use super::*;

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
        // The callback may only try-lock its short History fence. Unpublished owners stay
        // outside its environment: their destructors must run after Group/Policy are released.
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
