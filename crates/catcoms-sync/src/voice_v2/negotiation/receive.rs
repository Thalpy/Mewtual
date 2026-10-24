//! HIGH's bounded first-Offer inbox, kept OUTSIDE Exchange. Installation consumes the
//! original incoming packet claim and full remote-packet allowance before any Ready promise.
//! A prepared packet borrows that same affine position; refusal returns it to this owner,
//! never to the ordinary table. The native packet transaction consumes it into exact token/
//! event/history custody; this module does not grant successful decrypt or RTC effects.
// Native publication/Offer consumers are deliberately unscheduled until exact token and
// original browser key-phase integration exist. Tests exercise the actual capacity boundary.
#![allow(dead_code)]
use super::*;
use crate::voice_v2::packets::{InboundPacket, PacketSlot, PacketSlots};
use crate::voice_v2::{Body, EncryptedHeader, MessageId, SignedObject};
use std::sync::OnceLock;

/// Exact immutable promise. Exchange stores only this small observer, never the inbox Arc
/// or its credited payload; retaining either there would form a self-custody cycle.
pub(in crate::voice_v2) struct ReadyPublication {
    pub(in crate::voice_v2) sequence: u64,
    pub(in crate::voice_v2) profile: crate::voice_v2::SdpProfile,
    pub(in crate::voice_v2) id: MessageId,
    pub(in crate::voice_v2) hash: [u8; 32],
    pub(in crate::voice_v2) first_mono: u64,
    pub(in crate::voice_v2) deadline: u64,
    pub(in crate::voice_v2) issued: u64,
    pub(in crate::voice_v2) expires: u64,
    pub(in crate::voice_v2) live: CallLease,
}
/// Affine native publication preparation. Constructed only for the original cache's signed
/// fields and live installed inbox, before entering Group/Policy. It holds no new quota.
pub(in crate::voice_v2) struct PreparedPublication {
    marker: Option<ReadyPublication>,
    exchange: Arc<Exchange>,
}

/// Actual paid native receive position. An Arc may be retained by Ready Source/Lower work;
/// the enclosing cache is not its owner in the reverse direction. A permanently spent claim
/// cannot be reinstalled by dropping a prepared or retained packet.
pub(in crate::voice_v2) struct HighReceive {
    slot: Mutex<ReceiveSlot>,
    live: CallLease,
    spent: AtomicBool,
    _credit: Arc<Credit>,
    // LAST, independently of the packet/byte owners above.
    exchange: Arc<Exchange>,
}
struct ReceiveSlot {
    slot: Option<PacketSlot>,
}

/// Paid immutable input, not a successful native token/AEAD/RTC result. Future native final
/// admission must retain this owner while publishing its exact recoverable token. Dropping
/// an uncommitted preparation restores the SAME claim; committing spends it irreversibly.
pub(in crate::voice_v2) struct PreparedOffer {
    object: Option<SignedObject>,
    aad: Vec<u8>,
    hash: [u8; 32],
    slot: Option<PacketSlot>,
    receive: Arc<HighReceive>,
}
pub(in crate::voice_v2) struct RetainedOffer {
    object: SignedObject,
    aad: Vec<u8>,
    hash: [u8; 32],
    _slot: PacketSlot,
    _receive: Arc<HighReceive>,
}
pub(in crate::voice_v2) struct AdmittedOfferParts {
    pub object: SignedObject,
    pub slot: PacketSlot,
    pub credit: Arc<Credit>,
    pub exchange: Arc<Exchange>,
}
redacted_debug!(HighReceive, PreparedOffer, RetainedOffer);

impl Exchange {
    /// Capture first-round identity without retaining this Exchange in permanent/native
    /// intent history. Caller must already own the original paid native handoff phase.
    pub(in crate::voice_v2) fn initial_sdp_metadata(
        &self,
        wall: u64,
        mono: u64,
    ) -> Result<crate::voice_v2::sdp::InitialSdpMetadata, VoiceError> {
        use crate::voice_v2::sdp::{InitialSdpMetadata, ReadyEvidence};
        let proof = self.successor.as_ref().ok_or(VoiceError::Unauthorized)?;
        if !proof.drained() {
            return Err(VoiceError::Capacity);
        }
        if !proof.current_at(mono) || self.activated.load(Ordering::Acquire) {
            return Err(VoiceError::Unauthorized);
        }
        let ready = if proof.binding().low {
            let receipt = self.low_ready.as_ref().ok_or(VoiceError::Unauthorized)?;
            if !self.low_ready_retry_open() || !receipt.authorized_at(wall, mono) {
                return Err(VoiceError::Unauthorized);
            }
            ReadyEvidence::Low(receipt.clone())
        } else {
            if !self.ready_publication_current_at(wall, mono) {
                return Err(VoiceError::Unauthorized);
            }
            let marker = self
                .ready_publication
                .get()
                .ok_or(VoiceError::Unauthorized)?;
            ReadyEvidence::High {
                live: marker.live.clone(),
                issued: marker.issued,
                expires: marker.expires,
                first_mono: marker.first_mono,
                deadline: marker.deadline,
            }
        };
        Ok(InitialSdpMetadata {
            original: proof.original_observation(),
            round: self.sdp_round().ok_or(VoiceError::Unauthorized)?,
            ready,
        })
    }
    pub(in crate::voice_v2) fn ready_source_call(&self, call: &CallLease) -> bool {
        self.successor
            .as_ref()
            .is_some_and(|proof| proof.binding().call.is_same(call))
    }
    /// Pending Ready needs native closure/pair cancellation, independently of generic
    /// Active-only Exchange::revoked. Causal completion has its own receipt-position lease.
    pub(in crate::voice_v2) async fn ready_source_closed(&self) {
        use futures::FutureExt;
        let mut waits = vec![self.live.wait_revoked().boxed()];
        if let Some(binding) = &self.binding {
            waits.push(binding.call.wait_revoked().boxed());
            waits.push(binding.pair.wait_revoked().boxed());
        }
        futures::future::select_all(waits).await;
    }
    pub(in crate::voice_v2) fn ready_source_current_at(
        &self,
        id: MessageId,
        hash: [u8; 32],
        wall: u64,
        mono: u64,
    ) -> bool {
        !self.activated.load(Ordering::Acquire)
            && self.ready_publication_current_at(wall, mono)
            && self
                .ready_publication
                .get()
                .is_some_and(|marker| marker.id == id && marker.hash == hash)
    }
    pub(in crate::voice_v2) fn first_offer_deadline(
        &self,
        expires: u64,
        wall: u64,
        mono: u64,
    ) -> Result<u64, VoiceError> {
        if !self.ready_publication_current_at(wall, mono) || self.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let until = self
            .successor
            .as_ref()
            .and_then(|proof| proof.until())
            .ok_or(VoiceError::Unauthorized)?;
        let deadline = mono.saturating_add(20_000).min(until).min(
            mono.saturating_add(
                expires
                    .saturating_add(super::super::CLOCK_SKEW_MS)
                    .saturating_sub(wall),
            ),
        );
        if deadline <= mono {
            return Err(VoiceError::Freshness);
        }
        Ok(deadline)
    }
    /// Only the actual native first-Offer apply tail can call this. No resource lock or
    /// fallible CAS follows origin debit; receiver's affine claim already fences duplicates.
    pub(in crate::voice_v2) fn publish_first_offer(&self, deadline: u64) {
        assert!(
            !self.activated.load(Ordering::Acquire),
            "one original first Offer"
        );
        self.active_until.store(deadline, Ordering::Release);
        self.activated.store(true, Ordering::Release);
    }
    pub(in crate::voice_v2) fn ready_receive_open(&self) -> bool {
        self.ready_capacity_open()
            && self.ready_receive.get().is_some_and(CallLease::is_live)
            && self.ready_publication.get().is_some()
            && !self.is_live()
    }
    /// This is intentionally separate from generic Active-only lenders. Only first-Offer
    /// receive installation/verification may use it, with no outgoing/action/result access.
    fn charge_receive(self: &Arc<Self>, stage: Stage, bytes: usize) -> Result<Credit, VoiceError> {
        if !matches!(stage, Stage::RemotePacket | Stage::Crypto) {
            return Err(VoiceError::Scope);
        }
        let (index, count, maximum, kind) = stage.contract();
        if bytes == 0 || bytes > maximum {
            return Err(VoiceError::Bounds);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || !self.ready_capacity_open() || self.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        if state.used[index] >= count {
            return Err(VoiceError::Capacity);
        }
        let credit = ByteBudget::ReservedLane {
            reservation: self.reserve.clone(),
            lane: self.lane.clone(),
        }
        .charge(&self.resources, self.group, bytes, kind)?;
        state.used[index] += 1;
        drop(state);
        Ok(credit.with_custody(Arc::new(PhaseLoan {
            exchange: self.clone(),
            index,
        })))
    }

    pub(in crate::voice_v2) fn prepare_high_receive(
        self: &Arc<Self>,
        tables: &PacketSlots,
        mono: u64,
    ) -> Result<Arc<HighReceive>, VoiceError> {
        if !self.ready_artifact_current_at(mono)
            || !self.successor.as_ref().is_some_and(|proof| proof.drained())
            || self.ready_receive.get().is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        // Acquire actual bytes BEFORE removing the actual table position. No allocation or
        // fallible step follows publication of the observer. A failed foreign-table loan
        // drops the byte credit outside State and leaves installation unused.
        let credit = Arc::new(self.charge_receive(Stage::RemotePacket, PACKET_BYTES)?);
        let slot = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed || !self.ready_artifact_current_at(mono) {
                return Err(VoiceError::Unauthorized);
            }
            state
                .packet_claims
                .as_ref()
                .ok_or(VoiceError::Capacity)?
                .incoming_slot(tables)?
        };
        let receive = Arc::new(HighReceive {
            slot: Mutex::new(ReceiveSlot {
                slot: Some(PacketSlot::successor(slot, self.clone())),
            }),
            live: CallLease::default(),
            spent: AtomicBool::new(false),
            _credit: credit,
            exchange: self.clone(),
        });
        // A sole mutable LocalJoin owns installation. The irreversible OnceLock also fences
        // accidental future concurrent factories; no second successful observer is possible.
        self.ready_receive
            .set(receive.live.clone())
            .map_err(|_| VoiceError::Unauthorized)?;
        Ok(receive)
    }

    pub(in crate::voice_v2) fn ready_publication_current_at(&self, wall: u64, mono: u64) -> bool {
        self.ready_artifact_current_at(mono)
            && self.ready_publication.get().is_some_and(|marker| {
                marker.live.is_live()
                    && mono >= marker.first_mono
                    && mono < marker.deadline
                    && wall < marker.expires
                    && marker.issued <= wall.saturating_add(super::super::CLOCK_SKEW_MS)
            })
    }

    /// An untrusted bounded peek may select only this already-paid first-Offer verifier.
    /// It neither installs receive state nor authenticates the packet. Normal requester and
    /// process debits, signatures, route/policy and native final admission still follow.
    pub(crate) fn lends_ready_offer_verification(
        &self,
        requester: DeviceId,
        frame: &[u8],
        wall: u64,
        mono: u64,
    ) -> bool {
        if !self.ready_publication_current_at(wall, mono) || self.is_live() {
            return false;
        }
        let Ok(peek) = super::super::Envelope::peek_encrypted(frame) else {
            return false;
        };
        self.matches_first_offer(peek.scope, peek.issuer, &peek.header)
            && ((peek.envelope_kind == 1 && requester == peek.issuer)
                || (peek.envelope_kind == 3 && self.allows_helper(requester)))
    }

    /// Metadata only. The HIGH value comes from its original native signed publication;
    /// LOW's comes from its actual admitted tag6 receipt, never a browser/caller label.
    pub(in crate::voice_v2) fn sdp_round(&self) -> Option<crate::voice_v2::SdpRound> {
        self.low_ready
            .as_ref()
            .map(|receipt| receipt.sdp_round())
            .or_else(|| {
                self.ready_publication
                    .get()
                    .map(|ready| crate::voice_v2::SdpRound {
                        ready: ready.id,
                        profile: ready.profile,
                    })
            })
    }
    pub(in crate::voice_v2) fn accepts_sdp_kind(&self, kind: EncryptedKind) -> bool {
        if self.successor.is_some() {
            self.sdp_round()
                .is_some_and(|round| kind.round() == Some(round))
        } else {
            matches!(kind, EncryptedKind::Offer | EncryptedKind::Answer)
        }
    }
    pub(in crate::voice_v2) fn matches_first_offer(
        &self,
        scope: Scope,
        issuer: DeviceId,
        header: &EncryptedHeader,
    ) -> bool {
        let Some(proof) = &self.successor else {
            return false;
        };
        let binding = proof.binding();
        !binding.low
            && scope == binding.scope
            && issuer == binding.peer
            && matches!(header.kind, EncryptedKind::OfferRound(round) if Some(round) == self.sdp_round())
            && header.target == binding.local
            && header.source_join == binding.peer_join
            && header.target_join == binding.local_join
            && header.source_key_ref == binding.peer_key
            && header.target_key_ref == binding.local_key
            && header.handshake == binding.handshake
            && header.low_to_high
            && header.offer.is_some()
    }

    pub(crate) fn charge_ready_offer_verification(
        self: &Arc<Self>,
        bytes: usize,
    ) -> Result<Credit, VoiceError> {
        if !self.ready_receive.get().is_some_and(CallLease::is_live)
            || self.ready_publication.get().is_none()
        {
            return Err(VoiceError::Unauthorized);
        }
        self.charge_receive(Stage::Crypto, bytes)
    }
}

impl HighReceive {
    pub(in crate::voice_v2) fn exchange(&self) -> Arc<Exchange> {
        self.exchange.clone()
    }
    pub(in crate::voice_v2) fn prepare_publication(
        &self,
        signed: crate::voice_v2::ready::producer::SignedReadyStamp,
    ) -> Result<PreparedPublication, VoiceError> {
        if !self.current_for(&self.exchange) {
            return Err(VoiceError::Unauthorized);
        }
        let marker = signed.into_marker(&self.exchange, &self.live)?;
        if self
            .exchange
            .ready_publication
            .get()
            .is_some_and(|old| !old.same(&marker))
        {
            return Err(VoiceError::RetryMismatch);
        }
        Ok(PreparedPublication {
            marker: Some(marker),
            exchange: self.exchange.clone(),
        })
    }
    pub(in crate::voice_v2) fn current_for(&self, owner: &Arc<Exchange>) -> bool {
        Arc::ptr_eq(&self.exchange, owner)
            && self.live.is_live()
            && owner
                .ready_receive
                .get()
                .is_some_and(|live| live.is_same(&self.live) && live.is_live())
    }
    /// Stage an authenticated canonical Offer in the original bounded inbox. Caller retains
    /// its verification ticket independently and must still perform the final native token/
    /// replay/origin transaction; this API grants none of those effects.
    pub(in crate::voice_v2) fn prepare_offer(
        self: &Arc<Self>,
        object: &SignedObject,
        wall: u64,
        mono: u64,
    ) -> Result<PreparedOffer, VoiceError> {
        let Body::Encrypted { header, ciphertext } = object.body() else {
            return Err(VoiceError::Unauthorized);
        };
        if !self.current_for(&self.exchange)
            || !self.exchange.ready_publication_current_at(wall, mono)
            || !self
                .exchange
                .matches_first_offer(object.scope(), object.issuer(), header)
            || ciphertext.len() > MAX_SDP_PLAINTEXT + 16
        {
            return Err(VoiceError::Unauthorized);
        }
        object.check_freshness(wall)?;
        let mut state = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if self.spent.load(Ordering::Acquire) || state.slot.is_none() {
            return Err(VoiceError::Capacity);
        }
        // Our existing full RemotePacket payment covers these canonical copies. Refusal
        // before the final take leaves the same slot untouched and no replay/token spent.
        let aad = object.aad()?;
        if 2 * (aad.len() + ciphertext.len()) + 256 > PACKET_BYTES {
            return Err(VoiceError::Bounds);
        }
        let hash = object.hash();
        let object = object.clone();
        Ok(PreparedOffer {
            object: Some(object),
            aad,
            hash,
            slot: state.slot.take(),
            receive: self.clone(),
        })
    }
}
impl ReadyPublication {
    fn same(&self, other: &Self) -> bool {
        self.sequence == other.sequence
            && self.profile == other.profile
            && self.id == other.id
            && self.hash == other.hash
            && self.first_mono == other.first_mono
            && self.deadline == other.deadline
            && self.issued == other.issued
            && self.expires == other.expires
            && self.live.is_same(&other.live)
    }
}
impl PreparedPublication {
    pub(in crate::voice_v2) fn check_at(&self, wall: u64, mono: u64) -> Result<(), VoiceError> {
        let marker = self.marker.as_ref().expect("unpublished Ready marker");
        if !self.exchange.ready_artifact_current_at(mono)
            || !marker.live.is_live()
            || mono < marker.first_mono
            || mono >= marker.deadline
            || wall >= marker.expires
            || marker.issued > wall.saturating_add(super::super::CLOCK_SKEW_MS)
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    /// Final infallible publication. The actual LocalJoin's exclusive borrow serializes
    /// preparations; a retry can observe only the SAME marker. There is no callback, blocking
    /// lock or credited last-owner destruction: native Pair still holds the original owners.
    /// This marker itself spends no packet quota: eventual Source admission owns that debit.
    pub(in crate::voice_v2) fn publish(mut self) {
        let marker = self.marker.take().expect("one Ready publication");
        if let Some(old) = self.exchange.ready_publication.get() {
            assert!(old.same(&marker), "immutable native Ready marker");
        } else {
            assert!(
                self.exchange.ready_publication.set(marker).is_ok(),
                "sole native Ready publisher"
            );
        }
    }
}
impl PreparedOffer {
    pub(in crate::voice_v2) fn belongs_to(&self, exchange: &Arc<Exchange>) -> bool {
        Arc::ptr_eq(&self.receive.exchange, exchange)
    }
    pub(in crate::voice_v2) fn uses_tables(&self, tables: &PacketSlots) -> bool {
        self.slot
            .as_ref()
            .is_some_and(|slot| slot.uses_incoming(tables))
    }
    pub(in crate::voice_v2) fn object_hash(&self) -> [u8; 32] {
        self.hash
    }
    /// Move AAD and pay the second ciphertext copy from the SAME full RemotePacket credit.
    /// Native publication retains this event outside Exchange. On failure the containing
    /// transaction MUST drop this event before PreparedOffer returns its slot.
    pub(in crate::voice_v2) fn prepare_event(
        &mut self,
        token: u64,
    ) -> Result<Box<InboundPacket>, VoiceError> {
        if self.aad.is_empty() {
            return Err(VoiceError::Reservation);
        }
        let object = self.object();
        let Body::Encrypted { header, ciphertext } = object.body() else {
            unreachable!()
        };
        Ok(Box::new(InboundPacket::from_first_offer(
            token,
            object.issuer(),
            header.clone(),
            ciphertext.clone(),
            std::mem::take(&mut self.aad),
            self.hash,
            self.receive._credit.clone(),
        )))
    }
    /// Infallible apply tail only. Actual native final admission has checked every gate;
    /// the exclusive prepared slot makes a second publication impossible without any CAS.
    pub(in crate::voice_v2) fn admit_parts(mut self) -> AdmittedOfferParts {
        self.receive.spent.store(true, Ordering::Release);
        AdmittedOfferParts {
            object: self.object.take().expect("original prepared Offer"),
            slot: self.slot.take().expect("original receive slot"),
            credit: self.receive._credit.clone(),
            exchange: self.receive.exchange.clone(),
        }
    }
    pub(in crate::voice_v2) fn object(&self) -> &SignedObject {
        self.object.as_ref().expect("unpublished Offer")
    }
    pub(in crate::voice_v2) fn check_at(&self, wall: u64, mono: u64) -> Result<(), VoiceError> {
        if !self.receive.current_for(&self.receive.exchange)
            || !self
                .receive
                .exchange
                .ready_publication_current_at(wall, mono)
        {
            return Err(VoiceError::Unauthorized);
        }
        self.object().check_freshness(wall)
    }
    /// Native final admission must call this only in its infallible apply tail, AFTER every
    /// fallible gate and original quota check. The exclusive prepared claim makes this state
    /// transition infallible; network handoff/cancellation never refunds a committed claim.
    pub(in crate::voice_v2) fn retain(mut self) -> RetainedOffer {
        self.receive.spent.store(true, Ordering::Release);
        RetainedOffer {
            object: self.object.take().expect("one immutable Offer"),
            aad: std::mem::take(&mut self.aad),
            hash: self.hash,
            _slot: self.slot.take().expect("same original incoming claim"),
            _receive: self.receive.clone(),
        }
    }
}
impl RetainedOffer {
    pub(in crate::voice_v2) fn object(&self) -> &SignedObject {
        &self.object
    }
    pub(in crate::voice_v2) fn aad(&self) -> &[u8] {
        &self.aad
    }
    pub(in crate::voice_v2) fn hash(&self) -> [u8; 32] {
        self.hash
    }
}
impl Drop for PreparedOffer {
    fn drop(&mut self) {
        self.discard_unpublished(|| {});
    }
}
impl PreparedOffer {
    /// Returning the position is the publication point for another preparer. Destroy its
    /// previous canonical buffers FIRST, outside the position mutex, so concurrent reborrow
    /// never overlaps two payloads under one RemotePacket payment. The seam is test-only at
    /// its call site; production runs no callback or await during this destruction sequence.
    fn discard_unpublished(&mut self, before_restore: impl FnOnce()) {
        if let Some(slot) = self.slot.take() {
            drop(self.object.take());
            drop(std::mem::take(&mut self.aad));
            before_restore();
            let mut state = self.receive.slot.lock().unwrap_or_else(|e| e.into_inner());
            assert!(
                !self.receive.spent.load(Ordering::Acquire) && state.slot.is_none(),
                "one affine prepared Offer"
            );
            state.slot = Some(slot);
        }
    }
    #[cfg(test)]
    pub(in crate::voice_v2) fn discard_before_restore_for_test(
        &mut self,
        before_restore: impl FnOnce(),
    ) {
        self.discard_unpublished(before_restore);
    }
}
impl Drop for HighReceive {
    fn drop(&mut self) {
        self.live.revoke();
    }
}

/// Fixed metadata only. Kept in Exchange's existing paid metadata graph.
pub(super) type PublicationCell = OnceLock<ReadyPublication>;
