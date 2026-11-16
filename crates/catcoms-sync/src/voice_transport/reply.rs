//! Typed native ACK custody from authenticated intake through actual transport send/failure.
use super::*;
use catcoms_rt::{RequestAuthority, SharedRequestKeepalive};
use futures::FutureExt;
use voice_v2::helper::HelperReceipt;
use voice_v2::runtime::VerificationReplyAuthority;

pub(super) enum Receipt {
    Helper(HelperReceipt),
    Endpoint(crate::voice_endpoint::EndpointReceipt),
}
impl Receipt {
    fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        match self {
            Self::Helper(receipt) => receipt.authorized_at(wall, mono),
            Self::Endpoint(receipt) => receipt.authorized_at(wall, mono),
        }
    }
    async fn revoked(&self) {
        match self {
            Self::Helper(receipt) => receipt.revoked().await,
            Self::Endpoint(receipt) => receipt.revoked().await,
        }
    }
}
struct ReplyAuthority {
    verification: Arc<VerificationReplyAuthority>,
    route: Arc<dyn RequestAuthority>,
    receipt: Option<Receipt>,
}
impl fmt::Debug for ReplyAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VoiceReplyAuthority { .. }")
    }
}
#[async_trait::async_trait]
impl RequestAuthority for ReplyAuthority {
    fn authorized(&self) -> bool {
        // Clock seams can synchronously retire any of the owners below. Take both original
        // native samples first; no Clock callback may follow an irreversible gate observation.
        let (wall, mono) = self.verification.sample_time();
        self.verification.authorized_at(mono)
            && self.route.authorized()
            && self
                .receipt
                .as_ref()
                .is_none_or(|receipt| receipt.authorized_at(wall, mono))
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let mut waits = vec![
            self.verification.revoked().boxed(),
            self.route.revoked().boxed(),
        ];
        if let Some(receipt) = &self.receipt {
            waits.push(receipt.revoked().boxed());
        }
        futures::future::select_all(waits).await;
    }
}
#[derive(Debug)]
struct ReplyCustody {
    _verification: SharedRequestKeepalive,
    // Includes the actual admitted packet and its charge, not just copied status/counts.
    _authority: Arc<ReplyAuthority>,
}
struct ReplyBytes {
    bytes: Vec<u8>,
    _custody: Arc<ReplyCustody>,
}
impl AsRef<[u8]> for ReplyBytes {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

/// Opaque prepared reply. There is deliberately no production Vec/Bytes extraction API.
/// Its bytes own the original verification and packet charges from construction onward; the
/// driver additionally retains that same custody for its serialized lower copy. No timeout,
/// cancellation or lost waiter refunds those still-owned buffers.
pub(crate) struct VoiceTransportReply {
    bytes: Bytes,
    authority: Arc<ReplyAuthority>,
    custody: Arc<ReplyCustody>,
}
impl fmt::Debug for VoiceTransportReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VoiceTransportReply { .. }")
    }
}
impl VoiceTransportReply {
    pub(super) fn new(
        bytes: Vec<u8>,
        verification: Arc<VerificationReplyAuthority>,
        route: Arc<dyn RequestAuthority>,
        receipt: Option<Receipt>,
        work: SharedRequestKeepalive,
    ) -> Option<Self> {
        let authority = Arc::new(ReplyAuthority {
            verification,
            route,
            receipt,
        });
        if !authority.authorized() {
            drop(bytes);
            return None;
        }
        let custody = Arc::new(ReplyCustody {
            _verification: work,
            _authority: authority.clone(),
        });
        let bytes = Bytes::from_owner(ReplyBytes {
            bytes,
            _custody: custody.clone(),
        });
        Some(Self {
            bytes,
            authority,
            custody,
        })
    }
    /// Local queue acceptance only. The driver repeats BOTH original ingress and this original
    /// application gate immediately before send. Reply-wait lifetime must not be included here:
    /// receiving the response itself closes that wait before the driver sends it.
    pub(crate) fn respond(
        self,
        responder: catcoms_rt::Responder,
    ) -> Result<(), catcoms_rt::TransportError> {
        responder.try_respond_with_custody(self.bytes, self.authority, self.custody)
    }
}
// Unit oracles may inspect signatures/corruption without exposing an ownership-stripping API
// to production consumers. Actual transport tests use respond(), retaining real driver custody.
#[cfg(test)]
impl std::ops::Deref for VoiceTransportReply {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}
