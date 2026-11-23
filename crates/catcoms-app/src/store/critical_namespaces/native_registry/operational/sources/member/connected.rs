//! The connected founder responder keeps the original native negatives
//! through queued reply admission and pays for the lower transport copies.
use super::*;
use crate::store::critical_namespaces::native_registry::operational::governed::JOIN_PROTOCOL;
use catcoms_rt::{
    NativeChatKeyCurrentCheck, ProtocolId, RequestAuthority, Responder, CHAT_V2_JOIN_KIND,
    CHAT_V2_JOIN_REQUEST_MAX_BYTES, CHAT_V2_JOIN_RESPONSE_MAX_BYTES,
};
use tokio::sync::watch;

struct JoinAuthority {
    mount: Arc<Scope>,
    source: Arc<NativeChatKeyCurrentCheck>,
    caller: RequestCancellation,
    waiting: Option<Arc<dyn RequestAuthority>>,
}

pub(super) fn constrain_founder_transport(
    mount: Arc<Scope>,
    source: NativeChatKeyCurrentCheck,
    caller: RequestCancellation,
    invocation: Option<watch::Receiver<bool>>,
) -> RequestCancellation {
    let signal = invocation.map_or_else(
        || caller.clone(),
        |live| RequestCancellation::new(live, caller.keepalive()),
    );
    signal.with_authority(Arc::new(JoinAuthority {
        mount,
        source: Arc::new(source),
        caller,
        waiting: None,
    }))
}

impl std::fmt::Debug for JoinAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeJoinAuthority")
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl RequestAuthority for JoinAuthority {
    fn authorized(&self) -> bool {
        self.mount.is_open()
            && self.source.is_current()
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

struct JoinPayment {
    _fee: CatalogReadReservation,
}

impl std::fmt::Debug for JoinPayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeJoinPayment").finish_non_exhaustive()
    }
}

impl<R: CryptoRngCore> NativeTextMemberPreparation<catcoms_net::MeshService, R> {
    /// Attach one paid receiver to the exact retained founder's native mesh.
    /// The receiver grants no member operation. Every request must separately
    /// capture the same installed row and original UI generation before work.
    pub fn open_connected_join_requests(
        self,
        caller: RequestCancellation,
    ) -> Result<catcoms_net::NativeChatJoinRequests, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Error;
        let fee = self
            .budget
            .metadata
            .pool
            .try_reserve(4096)
            .map_err(|_| Error::Busy)?;
        let (_, source) = self.prepare_using_issuer(|owner, command| {
            owner
                .prepare_cut(command, catcoms_rt::NativeChatKeyPurpose::JoinWelcome)
                .map(|cut| cut.observe_current())
                .map_err(|_| refused())
        })?;
        let ingress = self
            .row
            .original
            .join_ingress
            .as_ref()
            .ok_or(Error::Unavailable)?;
        let authority = Arc::new(JoinAuthority {
            mount: self.scope.clone(),
            source: Arc::new(source),
            caller: caller.retain_custody(Arc::new(JoinPayment { _fee: fee })),
            waiting: None,
        });
        if !authority.authorized() {
            return Err(Error::Cancelled);
        }
        ingress.try_attach(authority).ok_or(Error::Busy)
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
        let mount = self.scope.clone();
        let work = caller.clone().with_authority(Arc::new(JoinAuthority {
            mount: mount.clone(),
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
        let authority = Arc::new(JoinAuthority {
            mount,
            source,
            caller,
            waiting: None,
        });
        responder
            .try_respond_with_custody(
                bytes::Bytes::from(delivery),
                authority,
                Arc::new(JoinPayment { _fee: fee }),
            )
            .map_err(|_| Error::Cancelled)
    }
}
