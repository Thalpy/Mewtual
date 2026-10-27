//! A borrowed final RTC-delivery tail on the actual Actor. Native admission surrounds
//! this step; only its consuming commit enters current Group/Policy. No Clock, owned
//! preparation, SDK value or asynchronous work may move underneath those native guards.
use super::{SdpDelivery, VoiceError};
use crate::voice_v2::crypto_actions::CryptoAction;
use std::sync::Arc;

/// One exact original Open/delivery/effect opportunity. These pointers bind actual
/// allocations, not equal serialized labels. A previously Admitted/Applied observation
/// is historical: fresh admission still requires this current step. This loan cannot
/// escape the exclusive Actor call and grants neither a reusable RTC token nor SDK proof.
pub struct SdpDeliveryAdmission<'a> {
    action: &'a Arc<CryptoAction>,
    delivery: &'a Arc<SdpDelivery>,
    applied: bool,
    wall: u64,
    mono: u64,
    commit: &'a mut dyn FnMut() -> Result<(), VoiceError>,
}
impl<'a> SdpDeliveryAdmission<'a> {
    pub(crate) fn new(
        action: &'a Arc<CryptoAction>,
        delivery: &'a Arc<SdpDelivery>,
        applied: bool,
        wall: u64,
        mono: u64,
        commit: &'a mut dyn FnMut() -> Result<(), VoiceError>,
    ) -> Self {
        Self {
            action,
            delivery,
            applied,
            wall,
            mono,
            commit,
        }
    }
    /// Callback-free provenance check before native UI/route/registered-row admission.
    /// Another Action with matching labels or another delivery for the same PC refuses.
    pub fn matches(
        &self,
        action: &Arc<CryptoAction>,
        delivery: &Arc<SdpDelivery>,
        applied: bool,
    ) -> bool {
        Arc::ptr_eq(self.action, action)
            && Arc::ptr_eq(self.delivery, delivery)
            && self.applied == applied
    }
    /// Captured original preparation samples. No Clock or authority is acquired here.
    pub fn observed_time(&self) -> (u64, u64) {
        (self.wall, self.mono)
    }
    /// Consume the final opportunity beneath native's original lexical admission gates.
    /// Only current try-locks and the original prepaid atomic transition happen here.
    pub fn commit(self) -> Result<(), VoiceError> {
        (self.commit)()
    }
}
impl std::fmt::Debug for SdpDeliveryAdmission<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SdpDeliveryAdmission { .. }")
    }
}

/// Full fixed Root frame paid additively by App's original metadata RPC. Delivery and
/// Action allocations retain their existing history fees; this inventory pays only the
/// context, original borrowed origin, exact selectors and synchronous call wrappers.
pub const fn sdp_delivery_admission_bytes() -> usize {
    let current = std::mem::size_of::<crate::voice_v2::Context>()
        + std::mem::size_of::<crate::voice_v2::helper::RoomAuthority>()
        + std::mem::size_of::<crate::voice_v2::packets::Authority<'static>>()
        + std::mem::size_of::<crate::voice_v2::runtime::PreparedLocalOrigin<'static>>()
        + std::mem::size_of::<SdpDeliveryAdmission<'static>>()
        + std::mem::size_of::<[Option<catcoms_crypto::DeviceId>; 4]>()
        + 10 * std::mem::size_of::<Arc<()>>()
        + 4 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 256; // borrowed closure/final-owner, samples, discriminator and result wrappers
               // The stable directory/group/resource checks are allocation- and callback-free.
               // Original Open history completes before current Context/origin preparation, so its
               // fixed receipt/Engine/Clock frame is a disjoint peak, not an unlisted extra frame.
    let history = std::mem::size_of::<Option<super::SdpOpenResult>>()
        + std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 4 * std::mem::size_of::<Arc<()>>()
        + 128;
    if history > current {
        history
    } else {
        current
    }
}
