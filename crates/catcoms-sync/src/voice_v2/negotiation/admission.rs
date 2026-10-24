//! Fixed first-Offer admission history. History retains no live input, receiver, Exchange
//! or progress-backed credit. Those owners must drain before the next peer uses the lane.
use super::*;
use crate::voice_v2::chain::OriginalPlanObservation;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::EffectAuthority;
use crate::voice_v2::{MessageId, OfferId, CLOCK_SKEW_MS};
use catcoms_rt::{Clock, RequestAuthority};
use futures::FutureExt;

/// Plain retained fee paid before Ready. Never split it from a Producer/PhaseLoan-backed
/// credit: split_retained preserves that custody and would keep the live Exchange forever.
pub(in crate::voice_v2) struct ReceiptFunding {
    _credit: Credit,
}
impl ReceiptFunding {
    pub(in crate::voice_v2) fn new(credit: Credit) -> Arc<Self> {
        Arc::new(Self { _credit: credit })
    }
}
pub(crate) struct FirstOfferAdmission {
    original: OriginalPlanObservation,
    pub(crate) issuer: DeviceId,
    pub(crate) message: MessageId,
    pub(crate) hash: [u8; 32],
    pub(crate) sequence: u64,
    pub(crate) token: u64,
    pub(crate) offer: OfferId,
    pub(crate) round: crate::voice_v2::SdpRound,
    // Exact original Ready choice survives progress drainage without retaining Exchange.
    helpers: [Option<DeviceId>; 2],
    issued: u64,
    expires: u64,
    first_mono: u64,
    deadline: u64,
    room: RoomAuthority,
    effect: Arc<EffectAuthority>,
    clock: Arc<dyn Clock>,
    _funding: Arc<ReceiptFunding>,
}
pub(in crate::voice_v2) struct ReceiptConsent {
    pub room: RoomAuthority,
    pub effect: Arc<EffectAuthority>,
    pub clock: Arc<dyn Clock>,
}
redacted_debug!(FirstOfferAdmission, ReceiptFunding);
impl FirstOfferAdmission {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::voice_v2) fn prepare(
        exchange: &Arc<Exchange>,
        prepared: &super::receive::PreparedOffer,
        token: u64,
        first_mono: u64,
        deadline: u64,
        funding: Arc<ReceiptFunding>,
        consent: ReceiptConsent,
    ) -> Result<Arc<Self>, VoiceError> {
        let proof = exchange
            .successor
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let object = prepared.object();
        let super::super::Body::Encrypted { header, .. } = object.body() else {
            return Err(VoiceError::Unauthorized);
        };
        if proof.binding().low
            || consent.room.scope != proof.binding().scope
            || !prepared.belongs_to(exchange)
            || deadline <= first_mono
            || deadline > proof.until().ok_or(VoiceError::Unauthorized)?
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(Arc::new(Self {
            original: proof.original_observation(),
            issuer: object.issuer(),
            message: object.id(),
            hash: prepared.object_hash(),
            sequence: header.sequence,
            token,
            offer: header.offer.ok_or(VoiceError::Unauthorized)?,
            round: header
                .kind
                .round()
                .filter(|round| Some(*round) == exchange.sdp_round())
                .ok_or(VoiceError::Unauthorized)?,
            helpers: std::array::from_fn(|i| exchange.helpers().get(i).copied()),
            issued: object.issued(),
            expires: object.expires(),
            first_mono,
            deadline,
            room: consent.room,
            effect: consent.effect,
            clock: consent.clock,
            _funding: funding,
        }))
    }
    pub(crate) fn same_call(&self, call: &CallLease) -> bool {
        self.original.binding().call.is_same(call)
    }
    pub(in crate::voice_v2) fn allows_route(
        &self,
        requester: DeviceId,
        envelope: &crate::voice_v2::Envelope,
    ) -> bool {
        match envelope {
            crate::voice_v2::Envelope::Direct(_) => requester == self.issuer,
            crate::voice_v2::Envelope::HelperDelivery { permit, .. } => {
                self.helpers.contains(&Some(requester)) && permit.helper() == requester
            }
            _ => false,
        }
    }
    pub(crate) fn same_room(&self, room: &CallLease) -> bool {
        self.room.lease.is_same(room)
    }
    pub(crate) fn epoch(&self) -> u64 {
        self.original.binding().epoch
    }
    pub(crate) fn deadline(&self) -> u64 {
        self.deadline
    }
    pub(crate) fn retains_identity(&self) -> bool {
        self.original.binding().call.is_live()
    }
    pub(crate) fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        self.original.current()
            && self.room.lease.is_live()
            && self.effect.authorized_at(wall, mono)
            && mono >= self.first_mono
            && mono < self.deadline
            && wall <= self.expires.saturating_add(CLOCK_SKEW_MS)
            && self.issued <= wall.saturating_add(CLOCK_SKEW_MS)
    }
    pub(crate) fn authorized(&self) -> bool {
        let wall = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        self.authorized_at(wall, mono)
    }
    pub(crate) async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let binding = self.original.binding();
        futures::future::select_all(vec![
            binding.call.wait_revoked().boxed(),
            binding.pair.wait_revoked().boxed(),
            self.room.lease.wait_revoked().boxed(),
            self.effect.revoked().boxed(),
            self.clock
                .sleep(std::time::Duration::from_millis(
                    self.deadline.saturating_sub(self.clock.monotonic_ms()),
                ))
                .boxed(),
        ])
        .await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_offer_receipt_funding_covers_complete_history_after_progress_drains() {
        use std::mem::size_of;
        // Room/Clock/PlanBinding already have independent native payments. The plain
        // Reservation allocation survives Ready closure, so include it in this fee too.
        let bytes = size_of::<FirstOfferAdmission>()
            + size_of::<ReceiptFunding>()
            + size_of::<EffectAuthority>()
            + size_of::<Reservation>()
            + 4 * 2 * size_of::<usize>()
            // One fixed Root identity (full issuer + message) and enum/map-node allowance.
            // Pair's receipt Arc is already inline in its separately paid Pair body.
            + size_of::<(DeviceId, MessageId)>()
            + 2 * size_of::<usize>()
            + 512;
        assert!(
            bytes <= super::super::super::ready::capacity::RECEIPT,
            "fixed receipt graph: {bytes}"
        );
    }
}
