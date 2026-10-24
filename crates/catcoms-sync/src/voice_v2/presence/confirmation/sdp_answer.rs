//! Sole protected HIGH Answer factory. It derives Offer/round from the actual accepted
//! Open and original applied delivery; header receipt, a caller bool or historical RTC
//! admission alone cannot publish an Answer. This is not a browser SDK completion producer.
use super::*;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::ActorOwner;
use crate::voice_v2::sdp::action::{plaintext_tag, Direction};
use crate::voice_v2::sdp::{InitialSdpIntent, SdpDelivery};

impl LocalJoin {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_high_sdp_answer(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        intent: &Arc<InitialSdpIntent>,
        delivery: &Arc<SdpDelivery>,
        plaintext: &[u8],
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        rng: &mut impl CryptoRngCore,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        self.owned(authority)?;
        if plaintext.len() > crate::voice_v2::MAX_SDP_PLAINTEXT
            || std::str::from_utf8(plaintext).is_err()
        {
            return Err(VoiceError::Bounds);
        }
        let pair = self
            .confirmations
            .pairs
            .get(&handshake)
            .ok_or(VoiceError::Unauthorized)?;
        let receipt = pair.first_offer.as_ref().ok_or(VoiceError::Unauthorized)?;
        let open = self
            .actions
            .get(Key::SdpOpen(receipt.token))
            .ok_or(VoiceError::Unauthorized)?;
        if pair.low
            || !pair.established
            || pair.retired
            || !pair.pin.is_live()
            || pair.protected_epoch != Some(authority.context.epoch)
            || !pair
                .sdp_intent
                .as_ref()
                .is_some_and(|original| Arc::ptr_eq(original, intent))
            || !delivery.same_admission(receipt)
            || !open.owns_sdp_delivery(delivery)?
            || room.scope != self.scope
            || !room.lease.is_live()
            || !receipt.same_room(&room.lease)
            || !receipt.same_call(authority.lease)
            || !actor.uses_resources(&self.resources)
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(action) = &pair.sdp_seal {
            let binding = action.sdp_binding().ok_or(VoiceError::Unauthorized)?;
            return if binding.belongs_to_intent(intent) && binding.matches_plaintext(plaintext) {
                Ok(action.clone())
            } else {
                Err(VoiceError::RetryMismatch)
            };
        }
        let kind = EncryptedKind::AnswerRound(intent.round());
        if !delivery.permits_answer(handshake, kind, Some(receipt.offer)) {
            return Err(VoiceError::Unauthorized);
        }
        let peer = pair.binding.peer;
        let Some(successor::Preparation::Transferred { owner, .. }) = &pair.sdp_successor else {
            return Err(VoiceError::Unauthorized);
        };
        let owner = owner.clone();
        let (wall, mono) = actor.preparation_time()?;
        if !delivery.current_at(wall, mono) {
            return Err(VoiceError::Unauthorized);
        }
        let public_key = *self
            .announcement
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?
            .public_key();
        let packet = self.ledger.prepare_applied_sdp_answer(
            public_key,
            ReserveRequest {
                handshake,
                kind,
                offer: Some(receipt.offer),
                ciphertext_bytes: plaintext.len() + 16,
            },
            delivery,
            authority,
            clock.as_ref(),
            rng,
        )?;
        let message = packet.id();
        let header = packet.header().clone();
        let (payload_bytes, engine_bytes) =
            crypto_actions::sdp_bytes(packet.aad_len(), plaintext.len() + 16)?;
        let mut prepared = self.actions.prepare(
            Key::SdpSeal(handshake, message),
            Allocation {
                resources: self.resources.clone(),
                scope: self.scope,
                call: self.lease.clone(),
                budget: ByteBudget::Exchange {
                    exchange: owner,
                    stage: crate::voice_v2::negotiation::Stage::EncryptAction,
                },
                history: None,
                payload_bytes,
                engine_bytes,
            },
        )?;
        let (wall, mono) = actor.preparation_time()?;
        let binding = prepared.sdp_binding(
            intent.clone(),
            &header,
            message,
            Direction::Seal,
            plaintext_tag(plaintext),
            wall,
            mono,
        )?;
        let (mut publication, reservation) =
            self.ledger
                .validate_prepared_outgoing(packet, authority, clock.as_ref())?;
        publication.bind_sdp(binding.clone());
        let pending = publication.crypto_authority(clock.clone())?;
        let flight = CryptoFlight {
            handshake,
            action: prepared.serial(),
            reservation,
            plaintext: plaintext.to_vec(),
        };
        let (mut issue, inactive) = crypto_actions::ActionQueue::build_sdp_seal(
            prepared,
            flight,
            binding.clone(),
            pending,
        )?;
        let mut inactive = Some(inactive);
        let mut publication = Some(publication);
        let mut published = None;
        let own_mono = clock.monotonic_ms();
        let own_wall = clock.now_ms();
        let pair = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("exclusive original HIGH Pair");
        actor.with_local_origin(peer, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            if !room.lease.is_live()
                || !binding.current_at(wall, mono)
                || !delivery.current_at(wall, mono)
                || !delivery.permits_answer(handshake, kind, header.offer)
            {
                return Err(VoiceError::Unauthorized);
            }
            publication
                .as_ref()
                .expect("one original Answer")
                .admission()
                .check_at(origin.epoch(), wall, mono)?;
            inactive
                .as_ref()
                .expect("one original Seal")
                .check_pending_at(own_mono, own_wall)?;
            origin.publish(|| {
                issue.publish();
                let action = self
                    .actions
                    .publish_inactive(inactive.take().expect("one Answer action"));
                pair.sdp_seal = Some(action.clone());
                published = Some(action);
                publication.take().expect("one Answer draft").publish();
            })
        })?;
        drop((inactive, publication, issue));
        Ok(published.expect("one native HIGH Answer"))
    }
}
