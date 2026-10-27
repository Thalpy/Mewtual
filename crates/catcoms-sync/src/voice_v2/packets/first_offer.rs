//! Restricted first-Offer transaction. Generic Active-only token/action lenders stay closed.
//! Every fallible check precedes one EndpointOrigin debit; publication only moves paid owners.
use super::*;
use crate::voice_v2::negotiation::admission::{ReceiptConsent, ReceiptFunding, SdpAdmission};
use crate::voice_v2::negotiation::receive::{HighReceive, PreparedOffer};

pub(in crate::voice_v2) struct FirstOfferPublication<'a> {
    ledger: &'a mut PacketLedger,
    receipt: Arc<SdpAdmission>,
    replay: Replay,
    pair: CallLease,
    pending: Option<RevocationOwner>,
    // This second payload drops BEFORE PreparedOffer returns its original receive slot.
    event: Option<Arc<InboundPacket>>,
    prepared: Option<PreparedOffer>,
}
#[cfg(test)]
pub(in crate::voice_v2) fn admission_metadata_bytes() -> usize {
    use std::mem::size_of;
    // Count the larger simultaneous structural graph, including one ledger node allowance.
    // Payload Vec storage/AAD are charged separately by the ORIGINAL RemotePacket fee;
    // receiver, byte-credit and slot ancestors belong to the enclosing Exchange graph.
    let preparing = size_of::<FirstOfferPublication<'static>>()
        + size_of::<InboundPacket>()
        + 2 * size_of::<usize>() // actual shared event Arc header
        + size_of::<super::super::LeaseState>()
        + 2 * size_of::<usize>();
    let committed = size_of::<Incoming>()
        + size_of::<InboundPacket>()
        + 2 * size_of::<usize>() // event Arc header remains paid through the last borrower
        + size_of::<SdpExchange>()
        + size_of::<u64>()
        + 512
        + size_of::<super::super::LeaseState>()
        + 2 * size_of::<usize>();
    preparing.max(committed)
}
impl PacketLedger {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::voice_v2) fn prepare_first_offer(
        &mut self,
        receive: &Arc<HighReceive>,
        object: &SignedObject,
        funding: Arc<ReceiptFunding>,
        consent: ReceiptConsent,
        pc: Option<Arc<crate::voice_v2::sdp::InitialSdpIntent>>,
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<FirstOfferPublication<'_>, VoiceError> {
        self.prune(clock);
        let pair = self.inbound_pair(object, authority, clock)?;
        if pair.status != Status::Established || self.sdp.is_some() {
            return Err(VoiceError::Unauthorized);
        }
        let Body::Encrypted { header, .. } = object.body() else {
            return Err(VoiceError::Unauthorized);
        };
        // Reject delayed old/unprofiled SDP BEFORE even proposing a replay bit or token.
        // Ordinary verification capacity must not bypass the exact original Ready echo.
        let owner = receive.exchange();
        if !owner.matches_first_offer(object.scope(), object.issuer(), header) {
            return Err(VoiceError::Unauthorized);
        }
        let mut replay = pair.replay.clone();
        replay.admit(header.sequence)?;
        let pair = pair.live.clone();
        if self.incoming.len() >= RESERVATIONS {
            return Err(VoiceError::Capacity);
        }
        if !owner.uses_resources(&self.resources, self.scope.group) {
            return Err(VoiceError::Scope);
        }
        let wall = clock.now_ms();
        let mono = clock.monotonic_ms();
        let mut prepared = receive.prepare_offer(object, wall, mono)?;
        if !prepared.uses_tables(&self.packet_slots) {
            return Err(VoiceError::Scope);
        }
        let token = self.serial.checked_add(1).ok_or(VoiceError::Reservation)?;
        let deadline = owner.first_offer_deadline(object.expires(), wall, mono)?;
        // PC/Ready was issued before the Offer. Its earlier immutable window must also
        // bound the receipt wait and activated lane; an ACK cannot linger after authority.
        let deadline = match &pc {
            Some(pc) => deadline.min(mono.saturating_add(pc.remaining_at(wall, mono)?)),
            None => deadline,
        };
        let receipt = SdpAdmission::prepare(
            &owner, &prepared, token, mono, deadline, funding, consent, pc,
        )?;
        let pending = RevocationOwner::default();
        let event = prepared.prepare_event(token)?;
        Ok(FirstOfferPublication {
            ledger: self,
            receipt,
            replay,
            pair,
            pending: Some(pending),
            event: Some(event),
            prepared: Some(prepared),
        })
    }
}
impl FirstOfferPublication<'_> {
    pub(in crate::voice_v2) fn receipt(&self) -> Arc<SdpAdmission> {
        self.receipt.clone()
    }
    /// Pure checks inside Group/Policy. EndpointOrigin independently checks actual native
    /// member/route/epoch/request currency, rather than invoking a membership callback here.
    pub(in crate::voice_v2) fn check_at(&self, wall: u64, mono: u64) -> Result<(), VoiceError> {
        self.prepared
            .as_ref()
            .expect("unpublished first Offer")
            .check_at(wall, mono)?;
        if !self.pair.is_live()
            || !self.ledger.lease.is_live()
            || !self.receipt.authorized_at(wall, mono)
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    pub(in crate::voice_v2) fn publish(mut self, store: impl FnOnce(Arc<InboundPacket>)) {
        let parts = self
            .prepared
            .take()
            .expect("one receive claim")
            .admit_parts();
        let handshake = self.event.as_ref().expect("same event").header.handshake;
        let deadline = self.receipt.deadline();
        // Exclusive ledger borrowing fences serial/replay/role until this infallible tail.
        self.ledger.sdp = Some(SdpExchange {
            owner: parts.exchange.clone(),
            handshake,
            role: SdpRole::Responder(self.receipt.offer),
            local: None,
            remote: Some(parts.object.id()),
            ready_until: deadline,
            retire: Some(deadline),
        });
        self.ledger
            .pairs
            .get_mut(&handshake)
            .expect("original pair")
            .replay = std::mem::take(&mut self.replay);
        self.ledger.serial = self.receipt.token;
        self.ledger.incoming.insert(
            self.receipt.token,
            Incoming {
                object: parts.object,
                retire: deadline,
                token: self.pending.take().expect("same token"),
                confirmation: None,
                _credit: parts.credit,
                _slot: parts.slot,
                exchange: Some(parts.exchange.clone()),
            },
        );
        store(self.event.take().expect("one paid native event"));
        // Causal Ready completion/activation is published LAST after actual token, event
        // and fixed admission custody. The store closure only moves already paid owners.
        // Source observes this later; never take SourceState beneath Group/Policy.
        parts.exchange.publish_first_offer(deadline);
    }
}
