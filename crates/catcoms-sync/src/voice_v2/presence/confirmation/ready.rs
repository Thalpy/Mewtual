//! Exact LOW causal Ready intake. Fixed acceptance is published before old Finish sources
//! are stopped. Capacity extraction is a later native turn after actual verifier/reply Drop.
use super::successor::Preparation;
use super::*;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::negotiation::{Exchange, SdpSuccessor};
use crate::voice_v2::ready::admission::ReadyAdmission;
use crate::voice_v2::runtime::{ActorOwner, IngressTicket};

impl LocalJoin {
    /// Native capacity preparation after local Finish; this alone neither accepts Ready
    /// nor stops Finish retries. Retain the SAME owner through later capacity refusals.
    pub(crate) fn prepay_low_sdp_successor(
        &mut self,
        actor: &ActorOwner,
        handshake: Nonce,
        authority: &Authority<'_>,
    ) -> Result<(), VoiceError> {
        self.owned(authority)?;
        let pair = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .ok_or(VoiceError::Unauthorized)?;
        let (_, mono) = actor.preparation_time()?;
        let chain = pair.chain.as_ref().ok_or(VoiceError::Unauthorized)?;
        if !pair.low
            || !pair.established
            || pair.retired
            || !pair.pin.is_live()
            || pair.protected_epoch != Some(authority.context.epoch)
            || !actor.uses_resources(&self.resources)
            || !chain.low_ready_current_at(mono)
            || !chain.successor_actor_matches(actor, authority.context)?
        {
            return Err(VoiceError::Unauthorized);
        }
        // The future LOW Offer uses its own origin row too. Prepay both endpoint rows
        // with the capacity envelope; otherwise a full ordinary pool could admit Ready
        // but permanently refuse this local preparation before touching its paid lane.
        // Helpers are not known until HIGH's signed choice is admitted.
        actor.prepare_ready_members(pair.binding.peer, &[], authority.context)?;
        if pair.sdp_successor.is_none() {
            let capacity = chain
                .workflow()?
                .confirmation_control()
                .prepare_sdp_capacity()?;
            // LOW cannot choose helpers independently. This empty capacity-only placeholder
            // acquires the exact accepted Ready helpers only during affine extraction.
            pair.sdp_successor = Some(Preparation::Reserved {
                capacity,
                helpers: Vec::new(),
            });
        }
        Ok(())
    }
    /// Advance only an actually accepted LOW Ready, outside ingress/reply guards. A real
    /// busy original lane returns Capacity with the SAME prepared owner. No SDP activation,
    /// browser key adoption or renewed receive promise follows from this capacity move.
    pub(crate) fn prepare_low_sdp_successor(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        authority: &Authority<'_>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<SdpSuccessor, VoiceError> {
        self.owned(authority)?;
        let pair = self
            .confirmations
            .pairs
            .get(&handshake)
            .ok_or(VoiceError::Unauthorized)?;
        // chain_binding indexes an already-checked native pair. Public native APIs may
        // receive an unknown/stale handshake; classify that before the infallible helper.
        let binding = self.chain_binding(handshake, authority);
        let receipt = pair
            .ready_admission
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?
            .clone();
        let (wall, mono) = actor.preparation_time()?;
        if !pair.low
            || !pair.established
            || pair.retired
            || !pair.pin.is_live()
            || pair.protected_epoch != Some(authority.context.epoch)
            || room.scope != self.scope
            || !room.lease.is_live()
            || !receipt.same_call(authority.lease)
            || !receipt.same_room(&room.lease)
            || !receipt.authorized_at(wall, mono)
            || !actor.uses_resources(&self.resources)
        {
            return Err(VoiceError::Unauthorized);
        }
        let chain = pair.chain.as_ref().ok_or(VoiceError::Unauthorized)?.clone();
        let helpers: Vec<_> = receipt.helpers().collect();
        actor.prepare_ready_members(binding.peer, &helpers, authority.context)?;
        if pair.sdp_successor.is_none() {
            self.prepay_low_sdp_successor(actor, handshake, authority)?;
        }
        let pair = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("original pair");
        let preparation = pair
            .sdp_successor
            .take()
            .expect("original paid preparation");
        let preparation = match preparation {
            Preparation::Reserved { capacity, .. } => {
                // No wait for Workflow drainage: Control itself still owns it. The existing
                // transfer fence instead requires actual lane idle + no handoff loan.
                let control = chain.workflow()?.confirmation_control();
                match control.transfer_sdp_capacity(capacity) {
                    Ok(ticket) => Preparation::Transferred {
                        owner: Exchange::from_low_transferred(ticket, receipt.clone()),
                        ready: None,
                    },
                    Err((error, capacity)) => {
                        pair.sdp_successor = Some(Preparation::Reserved {
                            capacity,
                            helpers: Vec::new(),
                        });
                        return Err(error);
                    }
                }
            }
            preparation @ Preparation::Transferred { .. } => preparation,
        };
        // Retain before every later fallible fence/registry/packet-claim operation.
        pair.sdp_successor = Some(preparation);
        let Some(Preparation::Transferred { owner, .. }) = &pair.sdp_successor else {
            unreachable!()
        };
        let owner = owner.clone();
        self.ledger.retire_low_confirmation_cache(&chain)?;
        owner.prepare_packet_claims()?;
        actor.with_local_origin(binding.peer, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            if !receipt.authorized_at(wall, mono)
                || !owner.successor_current_at(&binding, &self.resources, mono)
                || !room.lease.is_live()
                || !origin.contains_members(owner.helpers())
            {
                return Err(VoiceError::Unauthorized);
            }
            origin.publish(|| ())
        })?;
        self.resources.register_exchange(self.scope.group, &owner)?;
        Ok(SdpSuccessor { owner })
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_protected_ready(
        &mut self,
        actor: &ActorOwner,
        ticket: &IngressTicket,
        envelope: &Envelope,
        room: &RoomAuthority,
        authority: &Authority<'_>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<Arc<ReadyAdmission>, VoiceError> {
        if room.scope != self.scope
            || !room.lease.is_live()
            || !actor.uses_resources(&self.resources)
            || actor.authorize_verified_route(ticket, envelope, authority.context)?
                != Disposition::Deliver
        {
            return Err(VoiceError::Unauthorized);
        }
        let object = envelope.object();
        let Body::PairReadyRound {
            sequence: 1,
            header,
            profile: crate::voice_v2::SdpProfile::CompleteSdpV1,
        } = object.body()
        else {
            return Err(VoiceError::Unauthorized);
        };
        self.owned(authority)?;
        let pair = self
            .confirmations
            .pairs
            .get_mut(&header.handshake)
            .ok_or(VoiceError::Unauthorized)?;
        if !pair.low
            || !pair.established
            || pair.retired
            || !pair.pin.is_live()
            || pair.protected_epoch != Some(authority.context.epoch)
        {
            return Err(VoiceError::Unauthorized);
        }
        let chain = pair.chain.as_ref().ok_or(VoiceError::Unauthorized)?.clone();
        let existing = pair.ready_admission.clone();
        let receipt = if let Some(receipt) = &existing {
            if receipt.issuer != object.issuer()
                || receipt.message != object.id()
                || receipt.hash != object.hash()
            {
                return Err(VoiceError::RetryMismatch);
            }
            receipt.clone()
        } else {
            chain.prepare_low_ready_admission(actor, object, room)?
        };
        if !receipt.same_call(authority.lease)
            || !receipt.same_room(&room.lease)
            || !receipt.allows_route(ticket.requester(), envelope)
        {
            return Err(VoiceError::Unauthorized);
        }
        actor.with_endpoint_origin(ticket, envelope, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            if !receipt.authorized_at(wall, mono)
                || (existing.is_none() && !chain.low_ready_current_at(mono))
            {
                return Err(VoiceError::Unauthorized);
            }
            if existing.is_some() {
                return Ok(());
            }
            origin.publish(|| {
                pair.ready_admission = Some(receipt.clone());
                chain.publish_low_ready();
            })
        })?;
        // Revoke old packet leases and destroy native cache owners OUTSIDE native guards.
        // This cannot close Await/Workflow: the same lane still funds this Ready reply.
        self.ledger.retire_low_confirmation_cache(&chain)?;
        Ok(receipt)
    }
}
