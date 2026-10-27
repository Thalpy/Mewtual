//! Protected LOW Answer input. Acquire the actual original receive position and full
//! RemotePacket bytes before cloning any authenticated ciphertext into native custody.
use super::*;
use crate::voice_v2::sdp::InitialSdpIntent;

impl PacketLedger {
    pub(in crate::voice_v2) fn prepare_answer_inbound(
        &mut self,
        object: &SignedObject,
        intent: &Arc<InitialSdpIntent>,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<PreparedInbound, VoiceError> {
        self.prune(clock);
        let pair = self.inbound_pair(object, authority, clock)?;
        let Body::Encrypted { header, ciphertext } = object.body() else {
            return Err(VoiceError::Unauthorized);
        };
        if pair.status != Status::Established
            || header.kind != EncryptedKind::AnswerRound(intent.round())
            || !intent.local_is_low()
        {
            return Err(VoiceError::Unauthorized);
        }
        let owner = self
            .sdp_owner(header, true)?
            .ok_or(VoiceError::Unauthorized)?;
        let sdp = self.sdp.as_ref().expect("same active native Offer");
        let offer = self
            .outgoing
            .get(&sdp.local.ok_or(VoiceError::Unauthorized)?)
            .ok_or(VoiceError::Unauthorized)?;
        if offer.sealed.published().is_none()
            || !offer
                .sdp_binding
                .as_ref()
                .is_some_and(|binding| binding.belongs_to_intent(intent))
        {
            return Err(VoiceError::Unauthorized);
        }
        let mut replay = pair.replay.clone();
        replay.admit(header.sequence)?;
        let pair_lease = pair.live.clone();
        if self.incoming.len() >= RESERVATIONS {
            return Err(VoiceError::Capacity);
        }
        let token = self.serial.checked_add(1).ok_or(VoiceError::Reservation)?;
        let slot = owner.incoming_sdp_slot(header, &self.packet_slots)?;
        let credit = Arc::new(owner.charge(
            Stage::RemotePacket,
            super::super::negotiation::initial_offer::remote_packet_bytes(),
        )?);
        // Both actual ciphertext copies below are covered by the original full stage.
        let aad = object.aad()?;
        let event = InboundPacket {
            token,
            issuer: object.issuer(),
            header: header.clone(),
            aad,
            ciphertext: ciphertext.clone(),
            object_hash: object.hash(),
            _custody: credit.clone(),
        };
        let wall = clock.now_ms();
        let mono = clock.monotonic_ms();
        let deadline = self
            .sdp_deadline(Some(&owner))?
            .min(mono.saturating_add(intent.remaining_at(wall, mono)?))
            .min(
                mono.saturating_add(
                    object
                        .expires()
                        .saturating_add(CLOCK_SKEW_MS)
                        .saturating_sub(wall),
                ),
            );
        Ok(PreparedInbound {
            incoming: Incoming {
                object: object.clone(),
                retire: deadline,
                token: RevocationOwner::default(),
                confirmation: None,
                _credit: credit,
                _slot: slot,
                exchange: Some(owner.clone()),
            },
            admission: InboundAdmission {
                local: self.lease.clone(),
                pair: pair_lease,
                expires: object.expires(),
                deadline,
                exchange: Some(owner),
                confirmation: None,
                confirmation_required: false,
            },
            event,
            serial: self.serial,
        })
    }
}
impl PreparedInbound {
    pub(in crate::voice_v2) fn sdp_event(&self) -> &InboundPacket {
        &self.event
    }
    pub(in crate::voice_v2) fn sdp_deadline(&self) -> u64 {
        self.admission.deadline
    }
}
