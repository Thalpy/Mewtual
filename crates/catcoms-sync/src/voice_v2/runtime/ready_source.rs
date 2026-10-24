//! Ready publication uses the same original-member allowance as other native packets, but
//! its proof has a different, still-inactive phase. No Clock/callback or capacity lender may
//! run under the final short Group -> Policy fence. SourceState, if held, always precedes it.
use super::*;

impl ActorOwner {
    pub(super) fn authorize_ready_source(
        &self,
        object: &super::super::SignedObject,
        activity: &super::super::source::SourceActivity,
        room: &super::super::helper::RoomAuthority,
        commit: bool,
    ) -> Result<(), VoiceError> {
        self.current()?;
        let ready = activity.ready().ok_or(VoiceError::Unauthorized)?;
        // Sample EVERY original time source before observing any irreversible native gate.
        // Never substitute this actor's monotonic domain for the Ready's original stamp.
        let wall = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        let (ready_wall, ready_mono) = ready.observation_time();
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
        let super::super::Body::PairReady { header } = object.body() else {
            return Err(VoiceError::Unauthorized);
        };
        self.current()?;
        if state.revision != self.revision
            || Some(header.epoch) != state.epoch
            || object.scope().group != self.group.hash
            || object.scope().policy != policy.pin
            || object.issuer() != self.gate.local
            || room.scope != object.scope()
            || !room.lease.is_live()
            || !policy.eligible
            || !policy.live.is_live()
            || !state.members.contains_key(&object.issuer())
            || !state.members.contains_key(&header.target)
            || header
                .helpers
                .iter()
                .any(|helper| !state.members.contains_key(helper))
            || (!header.helpers.is_empty() && !policy.indirect)
        {
            return Err(VoiceError::Unauthorized);
        }
        object.check_freshness(wall)?;
        activity.check_ready_at(object, ready_wall, ready_mono)?;
        if commit {
            state.limits.admit_packet(object, mono, wall)?;
        }
        Ok(())
    }
}
