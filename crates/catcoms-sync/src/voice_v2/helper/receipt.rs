//! Original helper admission custody, distinct from an endpoint's final-delivery receipt.
use super::*;

/// Every helper packet, including a room fan-out with zero current edges, retains the FIRST
/// actor/helper/issuer/room/permit owners. Targeted packets also retain the exact final member.
/// A room packet deliberately does not AND every fan-out edge: one departed target cannot
/// invalidate custody of unrelated edges. This gate grants no endpoint/media/key authority.
pub(super) struct AdmissionAuthority {
    pub(super) helper: Arc<EffectAuthority>,
    pub(super) target: Option<Arc<EffectAuthority>>,
    pub(super) room: RoomAuthority,
}
impl AdmissionAuthority {
    fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        self.helper.authorized_at(wall, mono)
            && self
                .target
                .as_ref()
                .is_none_or(|target| target.authorized_at(wall, mono))
            && self.room.lease.is_live()
    }
}
#[async_trait::async_trait]
impl RequestAuthority for AdmissionAuthority {
    fn authorized(&self) -> bool {
        self.helper.authorized()
            && self
                .target
                .as_ref()
                .is_none_or(|target| target.authorized())
            && self.room.lease.is_live()
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let mut waits = vec![
            self.helper.revoked().boxed(),
            self.room.lease.wait_revoked().boxed(),
        ];
        if let Some(target) = &self.target {
            waits.push(target.revoked().boxed());
        }
        futures::future::select_all(waits).await;
    }
}

/// Opaque response owner. Keeping just Admission's copied counts would lose the actual packet
/// and its original authority before the transport driver serialized/sent HelperQueued.
pub(crate) struct HelperReceipt {
    packet: Arc<Packet>,
    cached: bool,
}
redacted_debug!(AdmissionAuthority, HelperReceipt);
impl HelperReceipt {
    pub(super) fn new(packet: Arc<Packet>, cached: bool) -> Self {
        Self { packet, cached }
    }
    pub(crate) fn info(&self) -> Admission {
        Admission {
            cached: self.cached,
            edges: self.packet.edges.len(),
        }
    }
    pub(crate) fn authorized(&self) -> bool {
        self.packet.admission.authorized()
    }
    pub(crate) fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        self.packet.admission.authorized_at(wall, mono)
    }
    pub(crate) async fn revoked(&self) {
        self.packet.admission.revoked().await;
    }
}
