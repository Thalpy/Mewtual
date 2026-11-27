//! Bounded connected dependency requests under the original native mesh and lease.
catcoms_rt::redacted_debug! {
    NativeChatMemberDependencies;
    NativeChatMemberDependencyRequests;
    NativeChatMemberDependencyIncoming;
    NativeChatMemberDependencyExchange;
    NativeChatMemberInventoryExchange;
}

use super::native_chat_join::{
    NativeChatIngressAttachRefusal, NativeChatJoinIngress, NativeChatJoinRequests,
};
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
    pub(super) inventory_outbound: Weak<Semaphore>,
    pub(super) local: PeerId,
}

impl NativeChatMemberDependencies {
    pub fn try_attach(
        &self,
        authority: Arc<dyn RequestAuthority>,
    ) -> Option<NativeChatMemberDependencyRequests> {
        self.try_attach_checked(authority).ok()
    }

    pub fn try_attach_checked(
        &self,
        authority: Arc<dyn RequestAuthority>,
    ) -> Result<NativeChatMemberDependencyRequests, NativeChatIngressAttachRefusal> {
        self.ingress.try_attach_checked(authority).map(|events| {
            NativeChatMemberDependencyRequests {
                original: self.ingress.clone(),
                events,
            }
        })
    }

    #[cfg(feature = "native-chat-test-hooks")]
    #[doc(hidden)]
    pub fn hold_ingress_for_test(&self, hold: impl FnOnce()) -> Option<()> {
        let original = self.ingress.0.upgrade()?;
        let _slot = original.lock().ok()?;
        hold();
        Some(())
    }

    pub fn owns_request(&self, request: &NativeChatMemberDependencyIncoming) -> bool {
        self.ingress.0.ptr_eq(&request.original.0) && request.responder.is_current()
    }

    /// Capture the exact existing peer connection before preparing the wire.
    pub fn capture_request(&self, peer: PeerId) -> Option<NativeChatMemberDependencyExchange> {
        let connection = connection_leases::current(&self.connections.upgrade()?, peer)?;
        Some(self.exchange(peer, connection, None))
    }

    /// Capture a bounded history inventory request on its own lower slot.
    /// The trusted App selects this lane before sealing the inventory. The
    /// received wire still enters the original dependency protocol.
    pub fn capture_inventory(&self, peer: PeerId) -> Option<NativeChatMemberInventoryExchange> {
        let mut exchange = self.capture_request(peer)?;
        exchange.outbound = self.inventory_outbound.clone();
        Some(NativeChatMemberInventoryExchange(exchange))
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
    /// Capture the original ingress incarnation for an application
    /// continuation. A different live connection to this peer is ineligible.
    pub fn observe_connection(&self) -> Option<ConnectionLease> {
        self.responder.original_connection()
    }

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

/// An outbound inventory can remain in the driver after its upper await
/// ends. Its separately bounded slot cannot occupy the exact-request slot.
pub struct NativeChatMemberInventoryExchange(NativeChatMemberDependencyExchange);

impl NativeChatMemberInventoryExchange {
    pub fn observe_route(&self) -> NativeChatMemberDependencyObservation {
        self.0.observe_route()
    }

    pub fn observe_connection(&self) -> ConnectionLease {
        self.0.observe_connection()
    }

    pub async fn request(
        self,
        sealed: &[u8],
        caller: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.0.request(sealed, caller).await
    }
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
            .map_err(|error| match error {
                tokio::sync::TryAcquireError::Closed => TransportError::Closed,
                tokio::sync::TryAcquireError::NoPermits => TransportError::Busy,
            })?;
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
                mpsc::error::TrySendError::Full(_) => TransportError::Busy,
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
    use catcoms_rt::{
        Clock, MeshTransport, PublishOnceError, PublishSubmission, SystemClock, Topic,
    };
    use std::time::Duration;
    use tokio::sync::watch;

    #[tokio::test]
    async fn native_inventory_keeps_its_own_physical_slot_while_another_peer_answers_exact() {
        let native = |seed, listen: bool| {
            let mut preparation = NativeTcpPreparation::new((), Arc::new(()));
            let addresses = listen.then(|| "/ip4/127.0.0.1/tcp/0".parse().unwrap());
            preparation
                .build(
                    keypair_from_seed([seed; 32]).unwrap(),
                    addresses.as_slice(),
                    &[],
                )
                .unwrap()
        };
        let (source, _, _) = native(122, false);
        let (holder_a, a_address_peer, _) = native(123, true);
        let (holder_b, b_address_peer, _) = native(124, true);
        let lane = source.native_chat_member_dependencies().unwrap();
        let (_owner, signal) = watch::channel(false);
        let scope = RequestCancellation::new(signal, None);
        tokio::time::timeout(Duration::from_secs(15), async {
            for (holder, address_peer) in [(&holder_a, a_address_peer), (&holder_b, b_address_peer)]
            {
                let address = holder.next_listen_addr().await.unwrap();
                assert_eq!(
                    source
                        .dial_outcome(format!("{address}/p2p/{address_peer}").parse().unwrap())
                        .await
                        .unwrap(),
                    catcoms_rt::DialSubmission::Submitted
                );
                source
                    .wait_for_peer_connected(holder.local_peer())
                    .await
                    .unwrap();
            }
            let a = holder_a.local_peer();
            let b = holder_b.local_peer();
            let stalled = lane.capture_inventory(a).unwrap();
            let authority_a = Arc::new(OriginalRoute {
                observation: stalled.observe_route(),
                caller: scope.clone(),
            });
            let exact = lane.capture_request(b).unwrap();
            let authority_b = Arc::new(OriginalRoute {
                observation: exact.observe_route(),
                caller: scope.clone(),
            });
            let mut requests_a = holder_a
                .native_chat_member_dependencies()
                .unwrap()
                .try_attach(authority_a.clone())
                .unwrap();
            let mut requests_b = holder_b
                .native_chat_member_dependencies()
                .unwrap()
                .try_attach(authority_b.clone())
                .unwrap();
            let signal = scope.clone();
            let inventory =
                tokio::spawn(async move { stalled.request(b"inventory", signal).await });
            let held = requests_a.next().await.unwrap();
            assert_eq!(held.current_data().unwrap().1, b"inventory");
            inventory.abort();
            assert!(inventory.await.unwrap_err().is_cancelled());
            assert!(matches!(
                lane.capture_inventory(b)
                    .unwrap()
                    .request(b"second inventory", scope.clone())
                    .await,
                Err(TransportError::Busy)
            ));

            let serve_exact = async {
                let request = requests_b.next().await.unwrap();
                assert_eq!(request.current_data().unwrap().1, b"exact original");
                request
                    .respond(
                        Bytes::from_static(b"exact reply"),
                        authority_b.clone(),
                        Arc::new(()),
                    )
                    .unwrap();
            };
            let (reply, ()) =
                tokio::join!(exact.request(b"exact original", scope.clone()), serve_exact);
            assert_eq!(reply.unwrap(), Bytes::from_static(b"exact reply"));
            assert!(
                held.current_data().is_some(),
                "the stalled request still holds its original responder"
            );
            held.respond(
                Bytes::from_static(b"discarded inventory reply"),
                authority_a,
                Arc::new(()),
            )
            .unwrap();
            let next_inventory = async {
                loop {
                    let result = lane
                        .capture_inventory(b)
                        .unwrap()
                        .request(b"fresh inventory", scope.clone())
                        .await;
                    if !matches!(&result, Err(TransportError::Busy)) {
                        break result;
                    }
                    SystemClock.sleep(Duration::from_millis(10)).await;
                }
            };
            let serve_inventory = async {
                let incoming = requests_b.next().await.unwrap();
                assert_eq!(incoming.current_data().unwrap().1, b"fresh inventory");
                incoming
                    .respond(Bytes::from_static(b"fresh page"), authority_b, Arc::new(()))
                    .unwrap();
            };
            let (page, ()) = tokio::join!(next_inventory, serve_inventory);
            assert_eq!(page.unwrap(), Bytes::from_static(b"fresh page"));
        })
        .await
        .expect("bounded authenticated TCP inventory and exact requests");
    }

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
        tokio::time::timeout(Duration::from_secs(16), async {
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
                        SystemClock.sleep(Duration::from_millis(20)).await
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
            let held_route = source.capture_request(target).unwrap();
            let held_scope = scope.clone();
            let held_waiter =
                tokio::spawn(async move { held_route.request(b"held original", held_scope).await });
            let held = requests.next().await.unwrap();
            assert_eq!(held.current_data().unwrap().1, b"held original");
            held_waiter.abort();
            assert!(held_waiter.await.unwrap_err().is_cancelled());
            // Losing the observer doesn't refund a lower request. A
            // locally busy retry is distinguishable from remote refusal.
            assert!(matches!(
                source
                    .capture_request(target)
                    .unwrap()
                    .request(b"held original", scope.clone())
                    .await,
                Err(TransportError::Busy)
            ));
            held.respond(
                Bytes::from_static(b"old reply"),
                authority.clone(),
                Arc::new(()),
            )
            .unwrap();
            let repeat = async {
                loop {
                    match source
                        .capture_request(target)
                        .unwrap()
                        .request(b"held original", scope.clone())
                        .await
                    {
                        Err(TransportError::Busy) => {
                            SystemClock.sleep(Duration::from_millis(10)).await
                        }
                        result => break result,
                    }
                }
            };
            let serve_repeat = async {
                let retry = requests.next().await.unwrap();
                assert_eq!(retry.current_data().unwrap().1, b"held original");
                retry
                    .respond(
                        Bytes::from_static(b"retry reply"),
                        authority.clone(),
                        Arc::new(()),
                    )
                    .unwrap();
            };
            let (retry, ()) = tokio::join!(repeat, serve_repeat);
            assert_eq!(retry.unwrap().as_ref(), b"retry reply");
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
