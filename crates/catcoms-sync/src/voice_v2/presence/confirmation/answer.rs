//! Exact native Answer admission. Receipt lookup precedes replay/token work, including
//! after successful Open consumed the original token. No new nonce, inbox or deadline is
//! reconstructed on recovery. Fixed history observes the PC without owning its progress.
use super::*;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::negotiation::admission::{ReceiptConsent, SdpAdmission};
use crate::voice_v2::negotiation::namespace::ReceiptNamespace;
use crate::voice_v2::runtime::{ActorOwner, IngressTicket};

impl LocalJoin {
    /// Capacity-only exemption for the original reserved Answer; final intake separately
    /// validates signatures, current epoch/member/route, actual PC and original Offer.
    pub(crate) fn has_answer_position(
        &self,
        object: &SignedObject,
        namespace: &ReceiptNamespace,
    ) -> bool {
        let Body::Encrypted { header, .. } = object.body() else {
            return false;
        };
        let Some(pair) = self.confirmations.pairs.get(&header.handshake) else {
            return false;
        };
        let (Some(intent), Some(answer)) = (&pair.sdp_intent, &pair.answer_position) else {
            return false;
        };
        pair.low
            && pair.binding.peer == object.issuer()
            && object.scope() == self.scope
            && header.kind == EncryptedKind::AnswerRound(intent.round())
            && header.offer == Some(answer.offer)
            && namespace.owns(&answer.position)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_protected_answer(
        &mut self,
        actor: &ActorOwner,
        ticket: &IngressTicket,
        envelope: &Envelope,
        room: &RoomAuthority,
        namespace: &ReceiptNamespace,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<Arc<SdpAdmission>, VoiceError> {
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
        if !matches!(header.kind, EncryptedKind::AnswerRound(_)) {
            return Err(VoiceError::Unauthorized);
        }
        self.pair_current(header.handshake, authority, clock.as_ref())?;
        let pair = self
            .confirmations
            .pairs
            .get_mut(&header.handshake)
            .ok_or(VoiceError::Unauthorized)?;
        if !pair.low || !pair.established || pair.protected_epoch != Some(authority.context.epoch) {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(receipt) = &pair.answer_receipt {
            if receipt.issuer != object.issuer()
                || receipt.message != object.id()
                || receipt.hash != object.hash()
                || receipt.sequence != header.sequence
                || receipt.offer != header.offer.ok_or(VoiceError::Unauthorized)?
                || Some(receipt.round) != header.kind.round()
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
        let intent = pair
            .sdp_intent
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?
            .clone();
        let answer = pair
            .answer_position
            .as_ref()
            .filter(|answer| namespace.owns(&answer.position))
            .ok_or(VoiceError::Capacity)?;
        if header.kind != EncryptedKind::AnswerRound(intent.round())
            || header.offer != Some(answer.offer)
        {
            return Err(VoiceError::Unauthorized);
        }
        let Some(successor::Preparation::Transferred { owner, .. }) = &pair.sdp_successor else {
            return Err(VoiceError::Unauthorized);
        };
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
        let prepared =
            self.ledger
                .prepare_answer_inbound(object, &intent, authority, clock.as_ref())?;
        let consent = ReceiptConsent {
            room: room.clone(),
            effect: actor.effect(object.issuer(), self.local, object.expires())?,
            clock: clock.clone(),
        };
        let (_, mono) = actor.preparation_time()?;
        let receipt = SdpAdmission::prepare_answer(
            owner,
            object,
            prepared.sdp_event(),
            mono,
            prepared.sdp_deadline(),
            &intent,
            answer.funding.clone(),
            consent,
        )?;
        let (publication, event) =
            self.ledger
                .validate_prepared_inbound(prepared, authority, clock.as_ref())?;
        let event = Arc::new(event);
        let mut event = Some(event);
        let mut publication = Some(publication);
        let mut retired_position = None;
        actor.with_endpoint_origin(ticket, envelope, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            if !room.lease.is_live()
                || !receipt.authorized_at(wall, mono)
                || !namespace.owns(
                    &pair
                        .answer_position
                        .as_ref()
                        .expect("exclusive Answer position")
                        .position,
                )
            {
                return Err(VoiceError::Unauthorized);
            }
            publication
                .as_ref()
                .expect("unpublished Answer")
                .admission()
                .check_at(wall, mono)?;
            origin.publish(|| {
                publication.take().expect("one native Answer").publish();
                pair.sdp_input = event.take();
                pair.answer_receipt = Some(receipt.clone());
                retired_position = pair.answer_position.take();
                retired_position
                    .as_mut()
                    .expect("one namespace position")
                    .position
                    .consume();
            })
        })?;
        drop((retired_position, publication, event));
        Ok(receipt)
    }
}
