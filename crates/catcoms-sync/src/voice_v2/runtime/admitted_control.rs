//! Separately typed Control7 route/origin transactions. Clock callbacks precede
//! nonblocking Group -> Policy acquisition; final publication uses pure gates.
use super::super::admitted_control::{wire, ControlLedger};
use super::super::admitted_wire::protocol::Metadata;
use super::*;

pub(crate) struct ControlOrigin<'a> {
    limits: &'a mut OriginLimits,
    actor: &'a ActorOwner,
    ticket: &'a IngressTicket,
    context: &'a super::super::Context,
    issuer: DeviceId,
    mono: u64,
    disposition: super::super::Disposition,
}
impl ControlOrigin<'_> {
    pub(crate) fn disposition(&self) -> super::super::Disposition {
        self.disposition
    }
    pub(crate) fn publish<T>(self, apply: impl FnOnce() -> T) -> Result<T, VoiceError> {
        self.ticket
            .check_at_commit(self.actor, self.context, self.mono)?;
        self.limits.prepared_origin(
            self.issuer,
            super::super::limits::Traffic::Setup,
            self.mono,
        )?;
        Ok(apply())
    }
}
impl ActorOwner {
    pub(crate) fn admit_control_service(&self) -> Result<(), VoiceError> {
        self.current()?;
        let mono = self.clock.monotonic_ms();
        self.current()?;
        self.resources.admit_voice_control(self.group.hash, mono)?;
        self.current()
    }
    /// Lazy fixed backing, paid against the exact existing group/process pool.
    /// This owner has no strong Group/Root backedge. A racing unpublished value
    /// is destroyed after releasing the registry guard, with its payer last.
    pub(crate) fn control(&self) -> Result<ControlLedger, VoiceError> {
        self.current()?;
        if let Some(ledger) = self.control_if_present() {
            return Ok(ledger);
        }
        let prepared =
            ControlLedger::new(self.resources.clone(), self.clock.clone(), self.group.hash)?;
        self.current()?;
        let selected = {
            let mut slot = self.group.control.lock().unwrap_or_else(|e| e.into_inner());
            slot.get_or_insert_with(|| prepared.clone()).clone()
        };
        drop(prepared);
        Ok(selected)
    }
    pub(crate) fn control_if_present(&self) -> Option<ControlLedger> {
        self.group
            .control
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub(crate) fn prepare_control_members(
        &self,
        issuer: DeviceId,
        target: DeviceId,
        context: &super::super::Context,
    ) -> Result<(), VoiceError> {
        self.current()?;
        let mono = self.clock.monotonic_ms();
        let mut state = self
            .group
            .state
            .try_lock()
            .map_err(|_| VoiceError::Capacity)?;
        let policy = self
            .gate
            .policy
            .try_lock()
            .map_err(|_| VoiceError::Capacity)?;
        self.check_route_view(&state, &policy, context.local, context)?;
        if !state.members.contains_key(&issuer) || !state.members.contains_key(&target) {
            return Err(VoiceError::Unauthorized);
        }
        state.limits.prepare_exchange_members(
            &[context.local, issuer, target],
            &self.resources,
            mono,
        )
    }
    pub(crate) fn with_control_origin<T>(
        &self,
        ticket: &IngressTicket,
        envelope: &wire::Envelope<'_>,
        context: &super::super::Context,
        publish: impl FnOnce(u64, u64, ControlOrigin<'_>) -> Result<T, VoiceError>,
    ) -> Result<T, VoiceError> {
        self.current()?;
        let hash = *blake3::hash(envelope.bytes()).as_bytes();
        let wall = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        let mut state = self
            .group
            .state
            .try_lock()
            .map_err(|_| VoiceError::Capacity)?;
        let policy = self
            .gate
            .policy
            .try_lock()
            .map_err(|_| VoiceError::Capacity)?;
        self.check_route_view(&state, &policy, ticket.requester, context)?;
        if !Arc::ptr_eq(&ticket.actor, &self.gate)
            || ticket.hash != hash
            || ticket.bytes != envelope.bytes().len()
            || !state
                .members
                .get(&ticket.requester)
                .is_some_and(|member| Arc::ptr_eq(member, &ticket.member))
            || !policy.live.is_same(&ticket.policy)
        {
            return Err(VoiceError::Unauthorized);
        }
        let native = super::super::Context {
            now: wall,
            ..*context
        };
        let disposition = envelope.authorize(ticket.requester, &native, &|id| {
            state.members.contains_key(&id)
        })?;
        ticket.check_at_commit(self, context, mono)?;
        publish(
            wall,
            mono,
            ControlOrigin {
                limits: &mut state.limits,
                actor: self,
                ticket,
                context,
                issuer: envelope.object().metadata().issuer(),
                mono,
                disposition,
            },
        )
    }
    pub(crate) fn with_control_source<T>(
        &self,
        metadata: &Metadata,
        context: &super::super::Context,
        publish: impl FnOnce(
            u64,
            u64,
            &dyn Fn(DeviceId) -> bool,
            &mut OriginLimits,
        ) -> Result<T, VoiceError>,
    ) -> Result<T, VoiceError> {
        self.current()?;
        let wall = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        let mut state = self
            .group
            .state
            .try_lock()
            .map_err(|_| VoiceError::Capacity)?;
        let policy = self
            .gate
            .policy
            .try_lock()
            .map_err(|_| VoiceError::Capacity)?;
        self.check_route_view(&state, &policy, metadata.issuer(), context)?;
        metadata.fresh(wall)?;
        if metadata.issuer() != context.local
            || metadata.epoch() != context.epoch
            || metadata.scope() != context.scope
        {
            return Err(VoiceError::Unauthorized);
        }
        // Borrow disjoint fields so the closure cannot materialize another roster.
        let GroupState {
            members, limits, ..
        } = &mut *state;
        publish(wall, mono, &|id| members.contains_key(&id), limits)
    }
}
