//! Affine local packet preparation. A protected producer must combine this unpublished row
//! with its candidate/action under the final native origin fence. The ordinary API publishes
//! synchronously before returning. Preparation alone changes no sequence, OfferId or SDP role.
use super::*;

pub(in crate::voice_v2) struct PreparedOutgoing {
    outgoing: Outgoing,
    reservation: ReservedPacket,
    admission: OutgoingAdmission,
    // Original preparation anchor. Delayed publication cannot renew the packet or SDP window.
    started: u64,
}

/// Original native owners and time bounds, never reconstructed from a caller's reservation.
pub(in crate::voice_v2) struct OutgoingAdmission {
    resources: Arc<Resources>,
    local: CallLease,
    pair: CallLease,
    chain: Option<CallLease>,
    epoch: u64,
    issued: u64,
    expires: u64,
    deadline: u64,
    exchange: Option<Arc<Exchange>>,
}
impl OutgoingAdmission {
    /// Pure final observation: no Clock, callbacks or ledger reentry under a producer fence.
    pub(in crate::voice_v2) fn check_at(
        &self,
        epoch: u64,
        wall: u64,
        mono: u64,
    ) -> Result<(), VoiceError> {
        if epoch != self.epoch
            || !self.local.is_live()
            || !self.pair.is_live()
            || self.chain.as_ref().is_some_and(|chain| !chain.is_live())
            || self.exchange.as_ref().is_some_and(|owner| !owner.is_live())
        {
            return Err(VoiceError::Unauthorized);
        }
        freshness(self.issued, self.expires, wall)?;
        if mono >= self.deadline {
            return Err(VoiceError::Reservation);
        }
        Ok(())
    }
}

/// Exclusive original-ledger loan: after validation no other actor operation can replace
/// sequence/SDP state before the infallible publish. The reserved AAD is returned separately
/// for an inactive action; its byte credit remains owned by this unpublished outgoing row.
pub(in crate::voice_v2) struct PreparedOutgoingPublication<'a> {
    ledger: &'a mut PacketLedger,
    outgoing: Outgoing,
    admission: OutgoingAdmission,
    started: u64,
}
impl PreparedOutgoingPublication<'_> {
    pub(in crate::voice_v2) fn started(&self) -> u64 {
        self.started
    }

    pub(in crate::voice_v2) fn admission(&self) -> &OutgoingAdmission {
        &self.admission
    }

    /// Observe the actual unpublished packet owner. No ordinary ledger lookup or post-publish
    /// construction is needed, and a caller's equal ReservedPacket cannot manufacture this grant.
    pub(in crate::voice_v2) fn crypto_authority(
        &self,
        clock: Arc<dyn Clock>,
    ) -> Result<crate::voice_v2::crypto_authority::CryptoAuthority, VoiceError> {
        let grant = crate::voice_v2::crypto_authority::CryptoAuthority::new(
            self.admission.local.clone(),
            self.admission.pair.clone(),
            clock,
            self.admission.deadline,
            self.admission.expires,
        )?
        .with_pending(self.outgoing.crypto.0.clone())?;
        match &self.admission.chain {
            Some(chain) => grant.with_pending(chain.clone()),
            None => Ok(grant),
        }
    }

    /// No recoverable tail, callback, Clock or engine lock. A protected caller must repeat
    /// admission.check_at under its current native actor/epoch/room/History transaction first.
    pub(in crate::voice_v2) fn publish(self) {
        self.publish_replacing(None);
    }

    /// The producer has validated the exact previous pair action before lending this exclusive
    /// ledger loan. Retirement only revokes that existing packet; its actual credits stay held.
    pub(in crate::voice_v2) fn publish_replacing(self, retire: Option<Nonce>) {
        if let Some(id) = retire {
            self.ledger.retire_outgoing_validated(id);
        }
        let draft = &self.outgoing.draft;
        self.ledger
            .publish_sdp_identity(&draft.header, draft.id, false, self.started);
        self.ledger
            .pairs
            .get_mut(&draft.header.handshake)
            .expect("exclusive validated pair")
            .next = draft.header.sequence;
        assert!(
            self.ledger
                .outgoing
                .insert(draft.id, self.outgoing)
                .is_none(),
            "exclusive outgoing identity"
        );
    }
}

impl PacketLedger {
    pub(in crate::voice_v2) fn prepare_outgoing(
        &self,
        public_key: [u8; 32],
        request: ReserveRequest,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedOutgoing, VoiceError> {
        self.prepare_outgoing_in_budget(public_key, request, authority, clock, rng, None)
    }

    pub(in crate::voice_v2) fn prepare_protected_outgoing(
        &self,
        public_key: [u8; 32],
        request: ReserveRequest,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        workflow: Arc<super::super::chain::Workflow>,
    ) -> Result<PreparedOutgoing, VoiceError> {
        self.prepare_outgoing_in_budget(public_key, request, authority, clock, rng, Some(workflow))
    }

    fn prepare_outgoing_in_budget(
        &self,
        public_key: [u8; 32],
        request: ReserveRequest,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        workflow: Option<Arc<super::super::chain::Workflow>>,
    ) -> Result<PreparedOutgoing, VoiceError> {
        if DeviceId::from_public_key_bytes(&public_key) != self.local {
            return Err(VoiceError::Unauthorized);
        }
        let pair = self.pair(&request.handshake, authority)?;
        let confirmation = matches!(
            request.kind,
            EncryptedKind::ConfirmHello
                | EncryptedKind::ConfirmProof
                | EncryptedKind::ConfirmFinish
        );
        if confirmation != (pair.status == Status::Confirming) {
            return Err(VoiceError::Unauthorized);
        }
        if self.outgoing.len() >= RESERVATIONS {
            return Err(VoiceError::Capacity);
        }
        // Preserve ordinary refusal before RNG. A protected row instead loans the actual
        // matching workflow position after its native header has been formed below.
        let ordinary_slot = if workflow.is_none() {
            Some(self.packet_slots.outgoing.charge(1)?)
        } else {
            None
        };
        if request.ciphertext_bytes < 16
            || request.ciphertext_bytes > request.kind.max_plaintext() + 16
        {
            return Err(VoiceError::Bounds);
        }
        let sequence = pair.next.checked_add(1).ok_or(VoiceError::Reservation)?;
        let binding = pair.binding.clone();
        let pair_owner = pair.live.clone();
        let mut id = [0; 32];
        rng.fill_bytes(&mut id);
        if self.outgoing.contains_key(&id) {
            return Err(VoiceError::Reservation);
        }
        let offer = if request.kind == EncryptedKind::Offer {
            if request.offer.is_some() {
                return Err(VoiceError::Malformed);
            }
            Some(OfferId {
                issuer: self.local,
                message: id,
            })
        } else {
            request.offer
        };
        let header = EncryptedHeader {
            source_join: self.join,
            target: binding.peer,
            target_join: binding.join,
            source_key_ref: self.key_ref,
            target_key_ref: binding.key_ref,
            handshake: binding.handshake,
            low_to_high: self.local < binding.peer,
            kind: request.kind,
            sequence,
            offer,
        };
        let (slot, confirmation_deadline, chain) = if let Some(workflow) = &workflow {
            let (slot, deadline, chain) = workflow.outgoing_confirmation_slot(
                &self.resources,
                authority.context,
                &self.lease,
                &pair_owner,
                &self.packet_slots,
                &header,
            )?;
            (slot, deadline, Some(chain))
        } else {
            (ordinary_slot.expect("ordinary slot"), u64::MAX, None)
        };
        let issued = clock.now_ms();
        let expires = issued
            .checked_add(MAX_PACKET_LIFETIME_MS)
            .ok_or(VoiceError::Freshness)?;
        let draft = PacketDraft::new(self.scope, public_key, id, issued, expires, header)?;
        let aad = draft.aad();
        let exchange = self.sdp_owner(&draft.header, false)?;
        let deadline = self.sdp_deadline(exchange.as_ref())?;
        // Same byte formula for every lane. Workflow loans retain their actual variable owner;
        // permanent result/history rows are paid separately by the protected producer's fixed row.
        let bytes = 2 * (aad.len() + request.ciphertext_bytes) + 256;
        let mono = clock.monotonic_ms();
        let credit = if let Some(workflow) = &workflow {
            workflow.charge(
                &self.resources,
                self.scope.group,
                bytes,
                ResourceKind::Retained,
            )?
        } else if let Some(exchange) = &exchange {
            exchange.charge(Stage::LocalPacket, bytes)?
        } else {
            self.resources
                .charge(self.scope.group, bytes, ResourceKind::Retained)?
        };
        let construction = mono.saturating_add(CONSTRUCTION_MS);
        Ok(PreparedOutgoing {
            outgoing: Outgoing {
                draft,
                ciphertext_bytes: request.ciphertext_bytes,
                construction,
                retire: mono.saturating_add(MAX_PACKET_LIFETIME_MS + CLOCK_SKEW_MS),
                source_deadline: mono
                    .saturating_add(MAX_PACKET_LIFETIME_MS)
                    .min(confirmation_deadline),
                cached: None,
                live: CallLease::default(),
                crypto: RevocationOwner::default(),
                chain: chain.clone(),
                _credit: credit,
                exchange: exchange.clone(),
                _slot: slot,
            },
            reservation: ReservedPacket {
                id,
                aad,
                sequence,
                expires,
            },
            admission: OutgoingAdmission {
                resources: self.resources.clone(),
                local: self.lease.clone(),
                pair: pair_owner,
                chain,
                epoch: authority.context.epoch,
                issued,
                expires,
                deadline: deadline.min(construction).min(confirmation_deadline),
                exchange,
            },
            started: mono,
        })
    }

    pub(in crate::voice_v2) fn validate_prepared_outgoing(
        &mut self,
        prepared: PreparedOutgoing,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<(PreparedOutgoingPublication<'_>, ReservedPacket), VoiceError> {
        let wall = clock.now_ms();
        let mono = clock.monotonic_ms();
        self.current(authority)?;
        let draft = &prepared.outgoing.draft;
        let pair = self.pair(&draft.header.handshake, authority)?;
        if !Arc::ptr_eq(&self.resources, &prepared.admission.resources)
            || !self.lease.is_same(&prepared.admission.local)
            || !pair.live.is_same(&prepared.admission.pair)
            || draft.scope != self.scope
            || DeviceId::from_public_key_bytes(&draft.public_key) != self.local
            || draft.header.source_join != self.join
            || draft.header.source_key_ref != self.key_ref
            || pair.binding.peer != draft.header.target
            || pair.binding.join != draft.header.target_join
            || pair.binding.key_ref != draft.header.target_key_ref
            || pair.next.checked_add(1) != Some(draft.header.sequence)
            || self.outgoing.contains_key(&draft.id)
        {
            return Err(VoiceError::Unauthorized);
        }
        let confirmation = matches!(
            draft.header.kind,
            EncryptedKind::ConfirmHello
                | EncryptedKind::ConfirmProof
                | EncryptedKind::ConfirmFinish
        );
        if confirmation != (pair.status == Status::Confirming) {
            return Err(VoiceError::Unauthorized);
        }
        if self.outgoing.len() >= RESERVATIONS {
            return Err(VoiceError::Capacity);
        }
        let current_exchange = self.sdp_owner(&draft.header, false)?;
        if match (&current_exchange, &prepared.outgoing.exchange) {
            (None, None) => false,
            (Some(current), Some(original)) => !Arc::ptr_eq(current, original),
            _ => true,
        } {
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
