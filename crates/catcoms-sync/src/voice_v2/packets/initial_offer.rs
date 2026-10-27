//! Private LOW first-Offer constructor. Its capacity-only inert loan is not exposed by
//! ordinary ReserveRequest APIs. No ledger sequence, role, slot claim or nonce becomes
//! externally usable until the final origin transaction publishes this exact draft.
use super::outgoing::{OutgoingAdmission, PreparedOutgoingPublication};
use super::*;
use crate::voice_v2::negotiation::initial_offer::InitialOfferPacket;

pub(in crate::voice_v2) struct InitialOfferOutgoing {
    outgoing: Outgoing,
    reservation: ReservedPacket,
    admission: OutgoingAdmission,
    started: u64,
    header: EncryptedHeader,
}
impl InitialOfferOutgoing {
    pub(in crate::voice_v2) fn header(&self) -> &EncryptedHeader {
        &self.header
    }
    pub(in crate::voice_v2) fn id(&self) -> MessageId {
        self.reservation.id
    }
}
impl PacketLedger {
    /// Native local origin only. RNG and time callbacks happen outside Group/Policy, and
    /// are followed by validation of the actual Pair and original inert intent.
    pub(in crate::voice_v2) fn prepare_initial_offer_outgoing(
        &self,
        public_key: [u8; 32],
        plaintext_bytes: usize,
        original: InitialOfferPacket,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
    ) -> Result<InitialOfferOutgoing, VoiceError> {
        let handshake = original.intent.handshake();
        let pair = self.pair(&handshake, authority)?;
        let kind = EncryptedKind::OfferRound(original.intent.round());
        pair.check_profile(kind)?;
        if self.sdp.is_some()
            || pair.status != Status::Established
            || self.local >= pair.binding.peer
            || DeviceId::from_public_key_bytes(&public_key) != self.local
            || !original
                .exchange
                .uses_resources(&self.resources, self.scope.group)
        {
            return Err(VoiceError::Unauthorized);
        }
        if self.outgoing.len() >= RESERVATIONS {
            return Err(VoiceError::Capacity);
        }
        if plaintext_bytes > MAX_SDP_PLAINTEXT {
            return Err(VoiceError::Bounds);
        }
        let sequence = pair.next.checked_add(1).ok_or(VoiceError::Reservation)?;
        let mut id = [0; 32];
        rng.fill_bytes(&mut id);
        if self.outgoing.contains_key(&id) {
            return Err(VoiceError::Reservation);
        }
        let header = EncryptedHeader {
            source_join: self.join,
            target: pair.binding.peer,
            target_join: pair.binding.join,
            source_key_ref: self.key_ref,
            target_key_ref: pair.binding.key_ref,
            handshake,
            low_to_high: true,
            kind,
            sequence,
            offer: Some(OfferId {
                issuer: self.local,
                message: id,
            }),
        };
        let issued = clock.now_ms();
        let mono = clock.monotonic_ms();
        let expires = issued
            .checked_add(MAX_PACKET_LIFETIME_MS)
            .ok_or(VoiceError::Freshness)?;
        let remaining = original.intent.remaining_at(issued, mono)?;
        let deadline = mono
            .saturating_add(remaining)
            .min(mono.saturating_add(20_000));
        let construction = mono.saturating_add(CONSTRUCTION_MS).min(deadline);
        let draft = PacketDraft::new(self.scope, public_key, id, issued, expires, header.clone())?;
        let aad = draft.aad();
        if aad.len() > super::super::negotiation::MAX_AAD {
            return Err(VoiceError::Bounds);
        }
        let admission = OutgoingAdmission {
            resources: self.resources.clone(),
            local: self.lease.clone(),
            pair: pair.live.clone(),
            chain: None,
            epoch: authority.context.epoch,
            issued,
            expires,
            deadline: construction,
            exchange: Some(original.exchange.clone()),
            initial: Some(original.intent),
        };
        self.current(authority)?;
        admission.check_at(authority.context.epoch, issued, mono)?;
        Ok(InitialOfferOutgoing {
            outgoing: Outgoing {
                draft,
                ciphertext_bytes: plaintext_bytes + 16,
                construction,
                retire: deadline,
                source_deadline: deadline,
                sealed: sealed::SealedState::Unsigned,
                live: CallLease::default(),
                crypto: RevocationOwner::default(),
                chain: None,
                workflow: None,
                _credit: original.credit,
                _slot: original.slot,
                exchange: Some(original.exchange),
                sdp_binding: None,
            },
            reservation: ReservedPacket {
                id,
                aad,
                sequence,
                expires,
            },
            admission,
            started: mono,
            header,
        })
    }
    pub(in crate::voice_v2) fn validate_initial_offer_outgoing(
        &mut self,
        prepared: InitialOfferOutgoing,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<(PreparedOutgoingPublication<'_>, ReservedPacket), VoiceError> {
        let pair = self.pair(&prepared.header.handshake, authority)?;
        let wall = clock.now_ms();
        let mono = clock.monotonic_ms();
        self.current(authority)?;
        if self.sdp.is_some()
            || pair.status != Status::Established
            || !pair.live.is_same(&prepared.admission.pair)
            || !self.lease.is_same(&prepared.admission.local)
            || !Arc::ptr_eq(&self.resources, &prepared.admission.resources)
            || pair.next.checked_add(1) != Some(prepared.header.sequence)
            || self.outgoing.contains_key(&prepared.reservation.id)
            || self.outgoing.len() >= RESERVATIONS
        {
            return Err(VoiceError::Unauthorized);
        }
        prepared
            .admission
            .check_at(authority.context.epoch, wall, mono)?;
        Ok((
            PreparedOutgoingPublication {
                ledger: self,
                outgoing: prepared.outgoing,
                admission: prepared.admission,
                started: prepared.started,
            },
            prepared.reservation,
        ))
    }
}
