//! The connected founder responder keeps the original native negatives
//! through queued reply admission and pays for the lower transport copies.
use super::*;
use crate::store::critical_namespaces::native_registry::operational::governed::JOIN_PROTOCOL;
use catcoms_rt::{
    NativeChatKeyCurrentCheck, ProtocolId, RequestAuthority, Responder, CHAT_V2_JOIN_KIND,
    CHAT_V2_JOIN_REQUEST_MAX_BYTES, CHAT_V2_JOIN_RESPONSE_MAX_BYTES,
};

struct ReplyAuthority {
    source: Arc<NativeChatKeyCurrentCheck>,
    caller: RequestCancellation,
    waiting: Option<Arc<dyn RequestAuthority>>,
}

impl std::fmt::Debug for ReplyAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinReplyAuthority").finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl RequestAuthority for ReplyAuthority {
    fn authorized(&self) -> bool {
        self.source.is_current()
            && !self.caller.is_cancelled()
            && self
                .waiting
                .as_ref()
                .is_none_or(|waiting| waiting.authorized())
    }

    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let mut caller = self.caller.clone();
        tokio::select! {
            _ = self.source.wait_for_retirement() => {},
            _ = caller.cancelled() => {},
            _ = async {
                if let Some(waiting) = &self.waiting {
                    waiting.revoked().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {},
        }
    }
}

struct ReplyPayment {
    _fee: CatalogReadReservation,
}

impl std::fmt::Debug for ReplyPayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinReplyPayment").finish_non_exhaustive()
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberPreparation<T, R> {
    /// Respond to a classified connection event through the original native
    /// founder. Success reports local queue acceptance.
    pub fn serve_connected_join_request(
        self,
        proto: ProtocolId,
        wire: &[u8],
        responder: Responder,
        caller: RequestCancellation,
    ) -> Result<(), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Error;
        if proto != JOIN_PROTOCOL || wire.len() > CHAT_V2_JOIN_REQUEST_MAX_BYTES {
            return Err(Error::Failed);
        }
        let body = wire
            .strip_prefix(&[CHAT_V2_JOIN_KIND])
            .ok_or(Error::Failed)?;
        let waiting = responder.cancellation_authority();
        if caller.is_cancelled() || !waiting.authorized() {
            return Err(Error::Cancelled);
        }
        let fee = self
            .budget
            .metadata
            .pool
            .try_reserve(4 * CHAT_V2_JOIN_RESPONSE_MAX_BYTES + 4096)
            .map_err(|_| Error::Busy)?;
        let packet =
            catcoms_sync::V2JoinRequestPacketData::decode(body).map_err(|_| Error::Failed)?;
        let (keys, [add, header, welcome]) = self.prepare_join_keys()?;
        let source = Arc::new(welcome.observe_current());
        let work = caller.clone().with_authority(Arc::new(ReplyAuthority {
            source: source.clone(),
            caller: caller.clone(),
            waiting: Some(waiting),
        }));
        let response =
            self.finish_prepared(keys)?
                .run_join_request(&packet, [add, header, welcome], &work)?;
        if work.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let delivery = response.encode_delivery_data().map_err(|_| Error::Failed)?;
        if delivery.len() > CHAT_V2_JOIN_RESPONSE_MAX_BYTES {
            return Err(Error::Failed);
        }
        let authority = Arc::new(ReplyAuthority {
            source,
            caller,
            waiting: None,
        });
        responder
            .try_respond_with_custody(
                bytes::Bytes::from(delivery),
                authority,
                Arc::new(ReplyPayment { _fee: fee }),
            )
            .map_err(|_| Error::Cancelled)
    }
}
