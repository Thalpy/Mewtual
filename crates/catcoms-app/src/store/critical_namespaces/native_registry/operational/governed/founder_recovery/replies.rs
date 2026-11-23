//! Return the original covered Welcome through this same recovered TCP actor.
use super::*;
use catcoms_net::NativeChatJoinRequests;
use catcoms_rt::{Responder, TransportEvent};

pub(super) struct Owner {
    _lifetime: watch::Sender<()>,
    scope: Arc<IngressScope>,
}

pub struct NativeTextColdFounderJoinRequests {
    requests: NativeChatJoinRequests,
    scope: Arc<IngressScope>,
}

pub struct NativeTextColdFounderJoinRequest {
    bytes: bytes::Bytes,
    responder: Responder,
    scope: Arc<IngressScope>,
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
        let (mount, fee) = self.work.recovered_founder_resources(4096)?;
        let (owner, lifetime) = watch::channel(());
        let scope = Arc::new(IngressScope {
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
        {
            return Err(refused());
        }
        let caller = request
            .scope
            .caller
            .clone()
            .with_authority(request.responder.cancellation_authority());
        let (_, fee) = self
            .work
            .recovered_founder_resources(4 * CHAT_V2_JOIN_RESPONSE_MAX_BYTES + 4096)?;
        let delivery = self.encode_recovered_join_delivery(&request.bytes[1..], &caller)?;
        request
            .responder
            .try_respond_with_custody(
                bytes::Bytes::from(delivery),
                request.scope,
                Arc::new(JoinFramePayment { _fee: fee }),
            )
            .map_err(|_| refused())
    }

    pub(in crate::store::critical_namespaces::native_registry) fn encode_recovered_join_delivery(
        &mut self,
        request: &[u8],
        caller: &RequestCancellation,
    ) -> Result<Vec<u8>, AppError> {
        self.restore_first_add(caller)?;
        let original = self.governed.as_ref().ok_or_else(refused)?;
        if !original.covers_request_data(request) {
            return Err(refused());
        }
        let delivery = original.encode_join_delivery(&self.source)?;
        if delivery.len() > CHAT_V2_JOIN_RESPONSE_MAX_BYTES {
            return Err(refused());
        }
        self.restore_first_add(caller)?;
        Ok(delivery)
    }
}
