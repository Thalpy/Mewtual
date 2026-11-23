//! Return the original covered Welcome through this same recovered TCP actor.
use super::*;
use catcoms_net::NativeChatJoinRequests;
use catcoms_rt::{Responder, TransportEvent};
use tokio::sync::watch;

pub(super) struct Owner {
    _lifetime: watch::Sender<()>,
    scope: Arc<Scope>,
}

#[derive(Debug)]
struct Scope {
    lifetime: watch::Receiver<()>,
    mount: RequestCancellation,
    caller: RequestCancellation,
}

#[async_trait::async_trait]
impl RequestAuthority for Scope {
    fn authorized(&self) -> bool {
        self.lifetime.has_changed().is_ok()
            && !self.mount.is_cancelled()
            && !self.caller.is_cancelled()
    }

    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let (mut lifetime, mut mount, mut caller) = (
            self.lifetime.clone(),
            self.mount.clone(),
            self.caller.clone(),
        );
        tokio::select! {
            _ = lifetime.changed() => {}
            _ = mount.cancelled() => {}
            _ = caller.cancelled() => {}
        }
    }
}

pub struct NativeTextColdFounderJoinRequests {
    requests: NativeChatJoinRequests,
    scope: Arc<Scope>,
}

pub struct NativeTextColdFounderJoinRequest {
    bytes: bytes::Bytes,
    responder: Responder,
    scope: Arc<Scope>,
}

impl NativeTextColdFounderJoinRequests {
    pub async fn next(&mut self) -> Option<NativeTextColdFounderJoinRequest> {
        loop {
            let event = self.requests.next().await?;
            let TransportEvent::Request {
                proto,
                data,
                responder,
                ..
            } = event
            else {
                continue;
            };
            if proto == JOIN_PROTOCOL
                && data.len() <= CHAT_V2_JOIN_REQUEST_MAX_BYTES
                && data.first() == Some(&CHAT_V2_JOIN_KIND)
            {
                return Some(NativeTextColdFounderJoinRequest {
                    bytes: data,
                    responder,
                    scope: self.scope.clone(),
                });
            }
        }
    }
}

impl NativeTextColdFounderSource {
    pub fn open_recovered_join_requests(
        &mut self,
        caller: RequestCancellation,
    ) -> Result<NativeTextColdFounderJoinRequests, AppError> {
        self.restore_first_add(&caller)?;
        let mesh = self.actor.as_ref().ok_or_else(refused)?.borrow()?;
        let ingress = mesh.native_chat_join_ingress().ok_or_else(refused)?;
        let (mount, fee) = self.storage.index.joined_exchange_resources(4096)?;
        let (owner, lifetime) = watch::channel(());
        let scope = Arc::new(Scope {
            lifetime,
            mount,
            caller: caller.retain_custody(Arc::new(JoinFramePayment { _fee: fee })),
        });
        let requests = ingress.try_attach(scope.clone()).ok_or_else(refused)?;
        self.ingress = Some(Owner {
            _lifetime: owner,
            scope: scope.clone(),
        });
        Ok(NativeTextColdFounderJoinRequests { requests, scope })
    }

    /// Queue the original covered delivery for an exact repeat on the
    /// current recovered ingress. New requests never enter signing or MLS.
    pub fn serve_recovered_join_request(
        &mut self,
        request: NativeTextColdFounderJoinRequest,
    ) -> Result<(), AppError> {
        if !self
            .ingress
            .as_ref()
            .is_some_and(|owner| Arc::ptr_eq(&owner.scope, &request.scope))
            || !request.scope.authorized()
            || !request.responder.is_current()
            || !self
                .governed
                .as_ref()
                .is_some_and(|add| add.covers_request_data(&request.bytes[1..]))
        {
            return Err(refused());
        }
        let caller = request
            .scope
            .caller
            .clone()
            .with_authority(request.responder.cancellation_authority());
        self.restore_first_add(&caller)?;
        let (_, fee) = self
            .storage
            .index
            .joined_exchange_resources(4 * CHAT_V2_JOIN_RESPONSE_MAX_BYTES + 4096)?;
        let original = self.governed.as_ref().ok_or_else(refused)?;
        let delivery = original.encode_join_delivery(&self.source)?;
        if delivery.len() > CHAT_V2_JOIN_RESPONSE_MAX_BYTES || caller.is_cancelled() {
            return Err(refused());
        }
        self.restore_first_add(&caller)?;
        request
            .responder
            .try_respond_with_custody(
                bytes::Bytes::from(delivery),
                request.scope,
                Arc::new(JoinFramePayment { _fee: fee }),
            )
            .map_err(|_| refused())
    }
}
