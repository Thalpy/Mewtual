//! Closed native packet provenance. Public discovery has no packet owner; protected Ready
//! must never silently fall through to ordinary capacity, helper selection or ACK completion.
use super::*;
use crate::voice_v2::packets::SourcePacketAuthority;
use crate::voice_v2::ready::source_authority::ReadySourceAuthority;

#[derive(Clone)]
pub(super) enum PacketActivity {
    Encrypted(SourcePacketAuthority),
    Ready(ReadySourceAuthority),
}
impl PacketActivity {
    pub(super) fn progress(&self) -> SourceProgress<'_> {
        match self {
            Self::Encrypted(packet) => packet.progress(),
            Self::Ready(ready) => SourceProgress::Ready(ready),
        }
    }
    pub(super) fn exchange(&self) -> Option<Arc<super::super::negotiation::Exchange>> {
        match self {
            Self::Encrypted(packet) => packet.exchange(),
            Self::Ready(ready) => Some(ready.exchange()),
        }
    }
    pub(super) fn observation_time(&self) -> (u64, u64) {
        match self {
            Self::Encrypted(packet) => packet.observation_time(),
            Self::Ready(ready) => ready.observation_time(),
        }
    }
    pub(super) fn authorized(&self) -> bool {
        let (wall, mono) = self.observation_time();
        self.authorized_at(wall, mono)
    }
    pub(super) fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        match self {
            Self::Encrypted(packet) => packet.authorized_at(wall, mono),
            Self::Ready(ready) => ready.authorized_at(wall, mono),
        }
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Encrypted(first), Self::Encrypted(second)) => first.same(second),
            (Self::Ready(first), Self::Ready(second)) => first.same(second),
            _ => false,
        }
    }
    pub(super) fn obsolete_high_confirmation(&self) -> bool {
        matches!(self, Self::Encrypted(packet) if packet.obsolete_high_confirmation())
    }
    pub(super) fn check(&self, object: &SignedObject, local: &CallLease) -> bool {
        match self {
            Self::Encrypted(packet) => packet.check(object, local),
            Self::Ready(ready) => {
                let (wall, mono) = ready.observation_time();
                ready.is_call(local) && ready.check_at(object, wall, mono)
            }
        }
    }
    pub(super) async fn revoked(&self) {
        match self {
            Self::Encrypted(packet) => packet.revoked().await,
            Self::Ready(ready) => ready.revoked().await,
        }
    }
}
