//! Actual protected first-Offer admission and recoverable fixed identity. This admits native
//! input custody, never successful AEAD, an RTC description or an audio connection.
use super::*;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::negotiation::admission::{FirstOfferAdmission, ReceiptConsent};
use crate::voice_v2::runtime::{ActorOwner, IngressTicket};

impl LocalJoin {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_protected_first_offer(
        &mut self,
        actor: &ActorOwner,
        ticket: &IngressTicket,
        envelope: &Envelope,
        room: &RoomAuthority,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<Arc<FirstOfferAdmission>, VoiceError> {
        if room.scope != self.scope
            || !room.lease.is_live()
            || !actor.uses_resources(&self.resources)
            || actor.authorize_verified_route(ticket, envelope, authority.context)?
                != Disposition::Deliver
        {
            return Err(VoiceError::Unauthorized);
        }
        let object = envelope.object();
        let Body::Encrypted { header, .. } = object.body() else {
            return Err(VoiceError::Unauthorized);
        };
        if header.kind != EncryptedKind::Offer {
            return Err(VoiceError::Unauthorized);
        }
        self.pair_current(header.handshake, authority, clock.as_ref())?;
        let binding = self.chain_binding(header.handshake, authority);
        let pair = self
            .confirmations
            .pairs
            .get_mut(&header.handshake)
            .ok_or(VoiceError::Unauthorized)?;
        if pair.low || !pair.established || pair.protected_epoch != Some(authority.context.epoch) {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(receipt) = &pair.first_offer {
            if receipt.issuer != object.issuer()
                || receipt.message != object.id()
                || receipt.hash != object.hash()
                || receipt.sequence != header.sequence
            {
                return Err(VoiceError::RetryMismatch);
            }
            if !receipt.same_room(&room.lease)
                || !receipt.same_call(authority.lease)
                || receipt.epoch() != authority.context.epoch
                || !receipt.allows_route(ticket.requester(), envelope)
            {
                return Err(VoiceError::Unauthorized);
            }
            actor.with_endpoint_origin(
                ticket,
                envelope,
                authority.context,
                |wall, mono, _origin| {
                    final_owner(wall, mono)?;
                    if receipt.authorized_at(wall, mono) {
                        Ok(())
                    } else {
                        Err(VoiceError::Unauthorized)
                    }
                },
            )?;
            return Ok(receipt.clone());
        }
        let Some(successor::Preparation::Transferred {
            owner,
            ready: Some(ready),
        }) = &mut pair.sdp_successor
        else {
            return Err(VoiceError::Unauthorized);
        };
        if !owner.successor_current_at(&binding, &self.resources, clock.monotonic_ms()) {
            return Err(VoiceError::Unauthorized);
        }
        // Loan selection is not admission authority. Ordinary Crypto may verify a valid
        // LOW-signed permit for a different helper; it must still respect Ready's frozen
        // choice here, including cached recovery after the live Exchange has drained.
        let allowed_route = match envelope {
            Envelope::Direct(_) => ticket.requester() == object.issuer(),
            Envelope::HelperDelivery { permit, .. } => {
                owner.allows_helper(ticket.requester()) && permit.helper() == ticket.requester()
            }
            _ => false,
        };
        if !allowed_route {
            return Err(VoiceError::Unauthorized);
        }
        let receive = ready.receive()?;
        let funding = ready.receipt_funding()?;
        let consent = ReceiptConsent {
            room: room.clone(),
            effect: actor.effect(object.issuer(), self.local, object.expires())?,
            clock: clock.clone(),
        };
        let publication = self.ledger.prepare_first_offer(
            &receive,
            object,
            funding,
            consent,
            authority,
            clock.as_ref(),
        )?;
        let receipt = publication.receipt();
        // Keep all actual owners outside a closure that might be destroyed while native
        // guards are held. Failed final admission returns the SAME input slot and no token.
        let mut publication = Some(publication);
        actor.with_endpoint_origin(ticket, envelope, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            if !room.lease.is_live() || !owner.successor_current_at(&binding, &self.resources, mono)
            {
                return Err(VoiceError::Unauthorized);
            }
            publication
                .as_ref()
                .expect("unpublished token")
                .check_at(wall, mono)?;
            origin.publish(|| {
                publication
                    .take()
                    .expect("one native token publication")
                    .publish(|event| {
                        pair.sdp_input = Some(event);
                        pair.first_offer = Some(receipt.clone());
                        ready.commit_receipt();
                    });
            })
        })?;
        Ok(receipt)
    }
}
