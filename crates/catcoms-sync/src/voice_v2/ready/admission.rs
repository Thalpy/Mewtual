//! Fixed LOW acceptance history. It owns original paid provenance and a plain history fee,
//! never Workflow, Exchange, an action/input, browser keys or another progress lender.
use crate::voice_v2::chain::OriginalPlanObservation;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::resources::Credit;
use crate::voice_v2::runtime::EffectAuthority;
use crate::voice_v2::*;
use catcoms_rt::{Clock, RequestAuthority};
use futures::FutureExt;
use std::sync::Arc;

pub(crate) struct ReadyAdmission {
    profile: SdpProfile,
    sequence: u64,
    pub(crate) issuer: DeviceId,
    pub(crate) message: MessageId,
    pub(crate) hash: [u8; 32],
    helpers: [Option<DeviceId>; 2],
    issued: u64,
    expires: u64,
    first_mono: u64,
    deadline: u64,
    room: RoomAuthority,
    effect: Arc<EffectAuthority>,
    clock: Arc<dyn Clock>,
    _fee: Credit,
    // LAST: the separately paid original fixed binding covers the retained history
    // reserve/clock/gates through their destruction, including the plain fee above.
    original: OriginalPlanObservation,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_ready_receipt_and_effect_and_root_identity_fit_plain_original_history_fee() {
        use std::mem::size_of;
        // Original PlanBinding/HistoryPlan/Reservation, room and clock have separate
        // native payments. Include this receipt/effect Arc headers and a conservative
        // COMPLETE shared node allowance (keys/values/header/internal edges), not merely
        // one record's amortized share. Root pins that node allowance against its types.
        let bytes = size_of::<ReadyAdmission>()
            + size_of::<EffectAuthority>()
            + 4 * size_of::<usize>()
            + 1024;
        assert!(
            bytes <= crate::voice_v2::chain::LOW_READY_RECEIPT_BYTES,
            "Ready fixed history graph {bytes}"
        );
    }
}
redacted_debug!(ReadyAdmission);
impl ReadyAdmission {
    pub(in crate::voice_v2) fn sdp_round(&self) -> SdpRound {
        // This receipt can ONLY be constructed from tag6/CompleteSdpV1 above.
        SdpRound {
            ready: self.message,
            profile: self.profile,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::voice_v2) fn prepare(
        original: OriginalPlanObservation,
        object: &SignedObject,
        finish_until: u64,
        wall: u64,
        mono: u64,
        room: RoomAuthority,
        effect: Arc<EffectAuthority>,
        clock: Arc<dyn Clock>,
        fee: Credit,
    ) -> Result<Arc<Self>, VoiceError> {
        let Body::PairReadyRound {
            sequence: 1,
            header,
            profile: crate::voice_v2::SdpProfile::CompleteSdpV1,
        } = object.body()
        else {
            return Err(VoiceError::Unauthorized);
        };
        object.check_freshness(wall)?;
        // A historical Ready ACK never renews HIGH's receive promise. Do not add signed
        // clock skew to this effect window; cap it by the actual original native Finish.
        let deadline = finish_until.min(mono.saturating_add(object.expires().saturating_sub(wall)));
        if room.scope != object.scope() || deadline <= mono || header.helpers.len() > 2 {
            return Err(VoiceError::Unauthorized);
        }
        Ok(Arc::new(Self {
            profile: SdpProfile::CompleteSdpV1,
            sequence: 1,
            original,
            issuer: object.issuer(),
            message: object.id(),
            hash: object.hash(),
            helpers: std::array::from_fn(|i| header.helpers.get(i).copied()),
            issued: object.issued(),
            expires: object.expires(),
            first_mono: mono,
            deadline,
            room,
            effect,
            clock,
            _fee: fee,
        }))
    }
    pub(crate) fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        self.original.current()
            && self.room.lease.is_live()
            && self.effect.authorized_at(wall, mono)
            && mono >= self.first_mono
            && mono < self.deadline
            && wall < self.expires
            && self.issued <= wall.saturating_add(CLOCK_SKEW_MS)
    }
    pub(crate) fn authorized(&self) -> bool {
        let wall = self.clock.now_ms();
        let mono = self.clock.monotonic_ms();
        self.authorized_at(wall, mono)
    }
    pub(crate) fn retains_identity(&self) -> bool {
        self.original.binding().call.is_live()
    }
    pub(crate) fn same_call(&self, call: &CallLease) -> bool {
        self.original.binding().call.is_same(call)
    }
    pub(crate) fn same_room(&self, room: &CallLease) -> bool {
        self.room.lease.is_same(room)
    }
    pub(in crate::voice_v2) fn deadline(&self) -> u64 {
        self.deadline
    }
    pub(in crate::voice_v2) fn helpers(&self) -> impl Iterator<Item = DeviceId> + '_ {
        self.helpers.iter().flatten().copied()
    }
    pub(in crate::voice_v2) fn allows_route(
        &self,
        requester: DeviceId,
        envelope: &Envelope,
    ) -> bool {
        match envelope {
            Envelope::Direct(_) => requester == self.issuer,
            Envelope::HelperDelivery { permit, .. } => {
                self.helpers.contains(&Some(requester)) && permit.helper() == requester
            }
            _ => false,
        }
    }
    pub(in crate::voice_v2) fn matches_retry(
        &self,
        p: &UnverifiedReady,
        wall: u64,
        mono: u64,
    ) -> bool {
        self.sequence == p.sequence
            && self.issuer == p.issuer
            && self.message == p.message
            && self.hash == p.hash
            && self.authorized_at(wall, mono)
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
