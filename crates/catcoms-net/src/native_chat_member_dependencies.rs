//! Bounded connected dependency requests under the original native mesh and lease.
use super::native_chat_join::{NativeChatJoinIngress, NativeChatJoinRequests};
use super::native_chat_members::NativeChatMemberFrameObservation;
use super::{connection_leases, Command, RR_PROTOCOL};
use bytes::Bytes;
use catcoms_rt::{
    ConnectionLease, PeerId, ProtocolId, RequestAuthority, RequestCancellation, Responder,
    SharedRequestKeepalive, TransportError, TransportEvent, CHAT_V2_DEPENDENCY_KIND,
    CHAT_V2_DEPENDENCY_REQUEST_MAX_BYTES, CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES,
    CHAT_V2_GOVERNING_KIND, CHAT_V2_GOVERNING_REQUEST_MAX_BYTES,
    CHAT_V2_GOVERNING_RESPONSE_MAX_BYTES,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};

type Connections = Mutex<HashMap<PeerId, ConnectionLease>>;

/// Weak access to the actual native mesh's dedicated request class.
#[derive(Clone)]
pub struct NativeChatMemberDependencies {
    pub(super) ingress: NativeChatJoinIngress,
    pub(super) commands: mpsc::WeakSender<Command>,
    pub(super) connections: Weak<Connections>,
    pub(super) outbound: Weak<Semaphore>,
    pub(super) local: PeerId,
}

impl NativeChatMemberDependencies {
    pub fn try_attach(
        &self,
        authority: Arc<dyn RequestAuthority>,
    ) -> Option<NativeChatMemberDependencyRequests> {
        self.ingress
            .try_attach(authority)
            .map(|events| NativeChatMemberDependencyRequests {
                original: self.ingress.clone(),
                events,
            })
    }

    pub fn owns_request(&self, request: &NativeChatMemberDependencyIncoming) -> bool {
        self.ingress.0.ptr_eq(&request.original.0) && request.responder.is_current()
    }

    /// Capture the exact existing peer connection before preparing the wire.
    pub fn capture_request(&self, peer: PeerId) -> Option<NativeChatMemberDependencyExchange> {
        let connection = connection_leases::current(&self.connections.upgrade()?, peer)?;
        Some(self.exchange(peer, connection, None))
    }

    /// Use only the original connected carrier of this received frame. An
    /// author ID or a new connection for the same peer cannot supply its route.
    pub fn capture_frame_request(
        &self,
        observation: &NativeChatMemberFrameObservation,
    ) -> Option<NativeChatMemberDependencyExchange> {
        let (peer, original) = observation.carrier_data()?;
        let current = connection_leases::current(&self.connections.upgrade()?, peer)?;
        current
            .is_same(original)
            .then(|| self.exchange(peer, current, Some(observation.clone())))
    }

    /// Continue catch-up on the original frame's live connection. A direct
    /// request that did not originate from a frame has no follow-up route.
    pub fn capture_frame_followup(
        &self,
        original: &NativeChatMemberDependencyObservation,
    ) -> Option<NativeChatMemberDependencyExchange> {
        if !original.is_current_cached() {
            return None;
        }
        let followup = self.capture_frame_request(original.frame.as_ref()?)?;
        followup
            .observation
            .connection
            .is_same(&original.connection)
            .then_some(followup)
    }

    fn exchange(
        &self,
        peer: PeerId,
        connection: ConnectionLease,
        frame: Option<NativeChatMemberFrameObservation>,
    ) -> NativeChatMemberDependencyExchange {
        NativeChatMemberDependencyExchange {
            commands: self.commands.clone(),
            local: self.local,
            peer,
            observation: NativeChatMemberDependencyObservation { connection, frame },
            outbound: self.outbound.clone(),
        }
    }
}

pub struct NativeChatMemberDependencyRequests {
    original: NativeChatJoinIngress,
    events: NativeChatJoinRequests,
}

impl NativeChatMemberDependencyRequests {
    pub async fn next(&mut self) -> Option<NativeChatMemberDependencyIncoming> {
        while let Some(TransportEvent::Request {
            from,
            proto,
            data,
            responder,
        }) = self.events.next().await
        {
            if proto == ProtocolId(RR_PROTOCOL)
                && data.first() == Some(&CHAT_V2_DEPENDENCY_KIND)
                && data.len() <= CHAT_V2_DEPENDENCY_REQUEST_MAX_BYTES
            {
                return Some(NativeChatMemberDependencyIncoming {
                    original: self.original.clone(),
                    from,
                    data,
                    responder,
                });
            }
        }
        None
    }
}

/// One classified request and its original responder. It cannot be adopted
/// by another actor with the same public peer identity.
pub struct NativeChatMemberDependencyIncoming {
    original: NativeChatJoinIngress,
    from: PeerId,
    data: Bytes,
    responder: Responder,
}

impl NativeChatMemberDependencyIncoming {
    pub fn current_data(&self) -> Option<(PeerId, &[u8])> {
        self.responder
            .is_current()
            .then_some((self.from, &self.data[1..]))
    }

    pub fn cancellation_authority(&self) -> Arc<dyn RequestAuthority> {
        self.responder.cancellation_authority()
    }

    pub fn respond(
        self,
        bytes: Bytes,
        authority: Arc<dyn RequestAuthority>,
        fee: SharedRequestKeepalive,
    ) -> Result<(), TransportError> {
        if bytes.is_empty() || bytes.len() > CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES {
            return Err(TransportError::NoResponse);
        }
        self.responder
            .try_respond_with_custody(bytes, authority, fee)
            .map_err(|_| TransportError::Cancelled)
    }
}

pub struct NativeChatMemberDependencyExchange {
    commands: mpsc::WeakSender<Command>,
    local: PeerId,
    peer: PeerId,
    observation: NativeChatMemberDependencyObservation,
    outbound: Weak<Semaphore>,
}

impl NativeChatMemberDependencyExchange {
    pub fn route_data(&self) -> (PeerId, PeerId) {
        (self.local, self.peer)
    }

    pub fn observe_route(&self) -> NativeChatMemberDependencyObservation {
        self.observation.clone()
    }

    pub fn observe_connection(&self) -> ConnectionLease {
        self.observation.connection.clone()
    }

    pub async fn request(
        self,
        sealed: &[u8],
        caller: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.request_bounded(
            sealed,
            caller,
            CHAT_V2_DEPENDENCY_KIND,
            CHAT_V2_DEPENDENCY_REQUEST_MAX_BYTES,
            CHAT_V2_DEPENDENCY_RESPONSE_MAX_BYTES,
        )
        .await
    }

    pub async fn request_governing(
        self,
        proof: &[u8],
        caller: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.request_bounded(
            proof,
            caller,
            CHAT_V2_GOVERNING_KIND,
            CHAT_V2_GOVERNING_REQUEST_MAX_BYTES,
            CHAT_V2_GOVERNING_RESPONSE_MAX_BYTES,
        )
        .await
    }

    async fn request_bounded(
        self,
        payload: &[u8],
        caller: RequestCancellation,
        kind: u8,
        request_bound: usize,
        response_bound: usize,
    ) -> Result<Bytes, TransportError> {
        if payload.is_empty() || payload.len() >= request_bound {
            return Err(TransportError::NoResponse);
        }
        let authority = Arc::new(OriginalRoute {
            observation: self.observation,
            caller: caller.clone(),
        });
        if !authority.authorized() {
            return Err(TransportError::Cancelled);
        }
        let slot = self
            .outbound
            .upgrade()
            .ok_or(TransportError::Closed)?
            .try_acquire_owned()
            .map_err(|_| TransportError::NoResponse)?;
        let commands = self.commands.upgrade().ok_or(TransportError::Closed)?;
        let mut cancel = caller
            .with_authority(authority)
            .retain_custody(Arc::new(OutboundSlot { _permit: slot }));
        let mut wire = Vec::with_capacity(1 + payload.len());
        wire.push(kind);
        wire.extend_from_slice(payload);
        let (reply, result) = oneshot::channel();
        commands
            .try_send(Command::RequestConnected {
                peer: self.peer,
                data: Bytes::from(wire),
                cancellation: Some(cancel.clone()),
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Closed(_) => TransportError::Closed,
                mpsc::error::TrySendError::Full(_) => TransportError::NoResponse,
            })?;
        drop(commands);
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(TransportError::Cancelled),
            reply = result => {
                let reply = reply.map_err(|_| TransportError::Closed)??;
                if cancel.is_cancelled() {
                    Err(TransportError::Cancelled)
                } else if reply.is_empty() || reply.len() > response_bound {
                    Err(TransportError::NoResponse)
                } else {
                    Ok(reply)
                }
            }
        }
    }
}

#[derive(Debug)]
struct OutboundSlot {
    _permit: OwnedSemaphorePermit,
}

#[derive(Debug)]
struct OriginalRoute {
    observation: NativeChatMemberDependencyObservation,
    caller: RequestCancellation,
}

#[derive(Clone, Debug)]
pub struct NativeChatMemberDependencyObservation {
    connection: ConnectionLease,
    frame: Option<NativeChatMemberFrameObservation>,
}

impl NativeChatMemberDependencyObservation {
    pub fn is_current_cached(&self) -> bool {
        self.connection.is_live()
            && self
                .frame
                .as_ref()
                .is_none_or(NativeChatMemberFrameObservation::is_current_cached)
    }

    pub async fn revoked(&self) {
        if let Some(frame) = &self.frame {
            frame.retired().await;
        } else {
            self.connection.revoked().await;
        }
    }
}

#[async_trait::async_trait]
impl RequestAuthority for OriginalRoute {
    fn authorized(&self) -> bool {
        self.observation.is_current_cached() && !self.caller.is_cancelled()
    }

    async fn revoked(&self) {
        let mut caller = self.caller.clone();
        tokio::select! {
            _ = self.observation.revoked() => {}
            _ = caller.cancelled() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{keypair_from_seed, NativeTcpPreparation};
    use catcoms_rt::{MeshTransport, PublishOnceError, PublishSubmission, Topic};
    use tokio::sync::watch;

    #[tokio::test]
    async fn native_member_dependency_exchange_keeps_the_original_connected_lane() {
        let mut left = NativeTcpPreparation::new((), Arc::new(()));
        let mut right = NativeTcpPreparation::new((), Arc::new(()));
        let (provider, peer, _) = right
            .build(
                keypair_from_seed([120; 32]).unwrap(),
                &["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                &[],
            )
            .unwrap();
        let (requester, _, _) = left
            .build(keypair_from_seed([121; 32]).unwrap(), &[], &[])
            .unwrap();
        let target = provider.local_peer();
        let provider_lane = provider.native_chat_member_dependencies().unwrap();
        let source = requester.native_chat_member_dependencies().unwrap();
        assert!(source.capture_request(target).is_none());
        let (_ready, signal) = watch::channel(false);
        let scope = RequestCancellation::new(signal, None);
        tokio::time::timeout(std::time::Duration::from_secs(16), async {
            let topic = Topic::new(vec![109; 32]);
            let (retire_frame, frame_signal) = watch::channel(false);
            let mut received = requester
                .native_chat_member_ingress()
                .unwrap()
                .open(
                    topic.clone(),
                    64,
                    RequestCancellation::new(frame_signal, None),
                )
                .await
                .unwrap();
            let address = provider.next_listen_addr().await.unwrap();
            assert_eq!(
                requester
                    .dial_outcome(format!("{address}/p2p/{peer}").parse().unwrap())
                    .await
                    .unwrap(),
                catcoms_rt::DialSubmission::Submitted
            );
            requester.wait_for_peer_connected(target).await.unwrap();
            assert!(matches!(
                source
                    .capture_request(target)
                    .unwrap()
                    .request(b"first", scope.clone())
                    .await,
                Err(TransportError::NoResponse)
            ));
            let observation = source.capture_request(target).unwrap().observe_route();
            assert!(source.capture_frame_followup(&observation).is_none());
            loop {
                match provider
                    .publish_once(topic.clone(), Bytes::from_static(b"original frame"))
                    .await
                {
                    Ok(PublishSubmission::Submitted) => break,
                    Err(PublishOnceError::NoPeers) => {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await
                    }
                    result => panic!("unexpected gossip submission {result:?}"),
                }
            }
            let Some(crate::NativeChatMemberGossipEvent::Frame(frame)) = received.next().await
            else {
                panic!("expected the frame from the original provider");
            };
            let original_route = source
                .capture_frame_request(&frame.observe())
                .unwrap()
                .observe_route();
            assert!(provider_lane
                .capture_frame_followup(&original_route)
                .is_none());
            let (retire, signal) = watch::channel(false);
            let candidate = RequestCancellation::new(signal, None);
            let authority = Arc::new(OriginalRoute {
                observation: observation.clone(),
                caller: candidate.clone(),
            });
            let mut requests = provider_lane.try_attach(authority.clone()).unwrap();
            for marker in [[7], [8]] {
                let route = if marker == [7] {
                    source.capture_frame_request(&frame.observe())
                } else {
                    source.capture_frame_followup(&original_route)
                }
                .unwrap();
                assert_eq!(route.route_data(), (requester.local_peer(), target));
                assert!(route.observe_connection().is_same(&observation.connection));
                let serve = async {
                    let request = requests
                        .next()
                        .await
                        .expect("exact native member dependency request");
                    assert!(provider_lane.owns_request(&request));
                    assert!(!source.owns_request(&request));
                    assert_eq!(
                        request.current_data(),
                        Some((requester.local_peer(), marker.as_slice()))
                    );
                    request
                        .respond(
                            Bytes::from_static(b"bounded reply"),
                            authority.clone(),
                            Arc::new(()),
                        )
                        .unwrap();
                };
                let (reply, ()) = tokio::join!(route.request(&marker, scope.clone()), serve);
                assert_eq!(reply.unwrap().as_ref(), b"bounded reply");
            }
            retire_frame.send_replace(true);
            assert!(source.capture_frame_followup(&original_route).is_none());
            assert!(source.capture_request(target).is_some());
            retire.send_replace(true);
            assert!(!authority.authorized());
            assert!(requests.next().await.is_none());
            assert!(matches!(
                source
                    .capture_request(target)
                    .unwrap()
                    .request(b"retired", candidate)
                    .await,
                Err(TransportError::Cancelled)
            ));
            drop(provider);
            observation.revoked().await;
            assert!(!observation.is_current_cached());
            assert!(source.capture_request(target).is_none());
        })
        .await
        .unwrap();
    }
}
