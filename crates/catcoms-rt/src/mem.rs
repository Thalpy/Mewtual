//! An in-memory [`MeshTransport`] for deterministic local testing.
//!
//! A [`Hub`] is a shared broker: every node [`Hub::join`]s it to get a
//! [`MemNetwork`] handle. Gossip is delivered to every *other* subscriber of a
//! topic; requests are routed to the target's inbox carrying a oneshot reply
//! channel. No sockets, no real time; N nodes run inside one test.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::Mutex as AsyncMutex;

use crate::transport::{
    ConnectionLease, DialSubmission, MeshTransport, PeerConnectionSnapshot, PeerId, ProtocolId,
    PublishOnceError, PublishSubmission, RequestCancellation, Responder, Topic, TransportError,
    TransportEvent, MAX_CONNECTED_PEER_SNAPSHOT, MAX_PEER_DIAL_BATCH, MAX_PUBLISH_ONCE_BYTES,
    MAX_PUBLISH_ONCE_TOPIC_BYTES,
};

#[derive(Debug, Default)]
struct HubState {
    inboxes: HashMap<PeerId, UnboundedSender<TransportEvent>>,
    topics: HashMap<Topic, HashSet<PeerId>>,
    connections: HashMap<PeerId, ConnectionLease>,
}

/// A shared in-memory broker connecting many [`MemNetwork`] nodes.
#[derive(Debug, Default)]
pub struct Hub {
    state: Mutex<HubState>,
}

impl Hub {
    /// Create a new, empty hub.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register `peer` and return its transport handle.
    pub fn join(self: &Arc<Self>, peer: PeerId) -> MemNetwork {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut state = self.state.lock().expect("hub mutex poisoned");
        let lease = ConnectionLease::default();
        if let Some(old) = state.connections.insert(peer, lease.clone()) {
            old.revoke();
        }
        state.inboxes.insert(peer, tx);
        drop(state);
        MemNetwork {
            hub: Arc::clone(self),
            local: peer,
            rx: Arc::new(AsyncMutex::new(rx)),
            registration: Arc::new(Registration {
                lease,
                links: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Every registered peer except `local`, as the transport's connected set.
    ///
    /// The in-memory transport has no dial step: a registered peer is reachable now, with no
    /// connection to establish first, so "registered" and "connected" are the same statement.
    /// Saying so matters because consumers gate work on this, and inheriting the empty default
    /// would have made those gates read "nothing is connected" in every in-process test while
    /// behaving completely differently over real sockets.
    fn connected(&self, local: PeerId) -> Vec<PeerConnectionSnapshot> {
        self.state
            .lock()
            .expect("hub mutex poisoned")
            .inboxes
            .iter()
            .filter(|(_, sender)| !sender.is_closed())
            .map(|(peer, _)| *peer)
            .filter(|peer| *peer != local)
            .take(MAX_CONNECTED_PEER_SNAPSHOT)
            .map(|peer| PeerConnectionSnapshot {
                peer,
                active: Vec::new(),
            })
            .collect()
    }

    fn deliver(&self, peer: PeerId, event: TransportEvent) -> Result<(), TransportError> {
        let state = self.state.lock().expect("hub mutex poisoned");
        match state.inboxes.get(&peer) {
            Some(tx) => tx.send(event).map_err(|_| TransportError::Closed),
            None => Err(TransportError::Unreachable(peer)),
        }
    }

    fn subscribers(&self, topic: &Topic) -> Vec<PeerId> {
        self.state
            .lock()
            .expect("hub mutex poisoned")
            .topics
            .get(topic)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }
}

/// One node's handle onto a [`Hub`]. Cloneable; clones share the same inbox, so
/// drive [`MemNetwork::next_event`] from a single task.
#[derive(Debug, Clone)]
pub struct MemNetwork {
    hub: Arc<Hub>,
    local: PeerId,
    rx: Arc<AsyncMutex<UnboundedReceiver<TransportEvent>>>,
    registration: Arc<Registration>,
}
impl MemNetwork {
    /// Start a fresh registration under the same peer identity in this deterministic harness.
    /// Old captured connection owners revoke immediately, while an old event reader may still
    /// retain its inbox. This models transport restart without consuming an application's events.
    pub fn rejoin(&self) -> Self {
        self.hub.join(self.local)
    }
}

#[derive(Debug)]
struct Registration {
    lease: ConnectionLease,
    links: Mutex<HashMap<PeerId, (ConnectionLease, ConnectionLease)>>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.lease.revoke();
    }
}

#[async_trait]
impl MeshTransport for MemNetwork {
    fn local_peer(&self) -> PeerId {
        self.local
    }

    fn connection_snapshot(&self) -> Vec<PeerConnectionSnapshot> {
        if !self.registration.lease.is_live() {
            return Vec::new();
        }
        self.hub.connected(self.local)
    }
    fn connection_lease(&self, peer: PeerId) -> Option<ConnectionLease> {
        if peer == self.local || !self.registration.lease.is_live() {
            return None;
        }
        let state = self.hub.state.lock().expect("hub mutex poisoned");
        state
            .inboxes
            .get(&peer)
            .filter(|sender| !sender.is_closed())?;
        let remote = state
            .connections
            .get(&peer)
            .filter(|lease| lease.is_live())
            .cloned()?;
        let mut links = self
            .registration
            .links
            .lock()
            .expect("registration mutex poisoned");
        if let Some((previous, current)) = links.get(&peer) {
            if previous.is_same(&remote) && current.is_live() {
                return Some(current.clone());
            }
        }
        links.retain(|_, (_, lease)| lease.is_live());
        if links.len() >= MAX_CONNECTED_PEER_SNAPSHOT {
            return None;
        }
        let lease = ConnectionLease::dependent(&self.registration.lease, &remote);
        links.insert(peer, (remote, lease.clone()));
        Some(lease)
    }
    fn supports_connection_leases(&self) -> bool {
        true
    }

    async fn subscribe(&self, topic: Topic) -> Result<(), TransportError> {
        if !self.registration.lease.is_live() {
            return Err(TransportError::Closed);
        }
        self.hub
            .state
            .lock()
            .expect("hub mutex poisoned")
            .topics
            .entry(topic)
            .or_default()
            .insert(self.local);
        Ok(())
    }

    async fn unsubscribe(&self, topic: Topic) -> Result<(), TransportError> {
        if !self.registration.lease.is_live() {
            return Err(TransportError::Closed);
        }
        let mut state = self.hub.state.lock().expect("hub mutex poisoned");
        if let Some(set) = state.topics.get_mut(&topic) {
            set.remove(&self.local);
        }
        Ok(())
    }

    async fn publish(&self, topic: Topic, data: Bytes) -> Result<(), TransportError> {
        if !self.registration.lease.is_live() {
            return Err(TransportError::Closed);
        }
        for peer in self.hub.subscribers(&topic) {
            if peer == self.local {
                continue; // gossip does not echo to the publisher
            }
            let event = TransportEvent::Gossip {
                topic: topic.clone(),
                from: self.local,
                data: data.clone(),
            };
            // Peers that have since left are simply skipped.
            let _ = self.hub.deliver(peer, event);
        }
        Ok(())
    }

    async fn publish_once(
        &self,
        topic: Topic,
        data: Bytes,
    ) -> Result<PublishSubmission, PublishOnceError> {
        if !self.registration.lease.is_live() {
            return Err(PublishOnceError::Closed);
        }
        if data.len() > MAX_PUBLISH_ONCE_BYTES
            || topic.as_bytes().len() > MAX_PUBLISH_ONCE_TOPIC_BYTES
        {
            return Err(PublishOnceError::TooLarge);
        }
        // No await or deferred worker: cancellation before this future is polled does nothing;
        // once polled, the whole attempt is synchronous. Compact slices so a tiny test event
        // cannot retain an arbitrarily large backing allocation. This deterministic test broker
        // does not model libp2p's duplicate cache or its bounded per-peer handler queues.
        let topic = Topic::new(Bytes::copy_from_slice(topic.as_bytes()));
        let data = Bytes::copy_from_slice(&data);
        let mut submitted = false;
        for peer in self.hub.subscribers(&topic) {
            if peer != self.local {
                submitted |= self
                    .hub
                    .deliver(
                        peer,
                        TransportEvent::Gossip {
                            topic: topic.clone(),
                            from: self.local,
                            data: data.clone(),
                        },
                    )
                    .is_ok();
            }
        }
        if submitted {
            Ok(PublishSubmission::Submitted)
        } else {
            Err(PublishOnceError::NoPeers)
        }
    }

    async fn request(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        if !self.registration.lease.is_live() {
            return Err(TransportError::Closed);
        }
        let (responder, rx) = Responder::channel();
        let event = TransportEvent::Request {
            from: self.local,
            proto,
            data,
            responder,
        };
        self.hub.deliver(peer, event)?;
        rx.recv().await.ok_or(TransportError::NoResponse)
    }

    async fn request_cancellable(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
        mut cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        if !self.registration.lease.is_live() || cancellation.is_cancelled() {
            return Err(TransportError::Cancelled);
        }
        let (mut responder, response) = Responder::channel();
        responder._keepalive = cancellation.keepalive();
        let event = TransportEvent::Request {
            from: self.local,
            proto,
            data,
            // The unbounded test inbox can outlive this waiting future just like libp2p's
            // outbound table. Keep accounting with the event/responder until it is handled.
            responder,
        };
        self.hub.deliver(peer, event)?;
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(TransportError::Cancelled),
            result = response.recv() => result.ok_or(TransportError::NoResponse),
        }
    }

    async fn request_connected(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        // A node registered in the in-memory hub is its equivalent of a currently live route;
        // there is no recent-address cache and therefore no implicit redial to suppress.
        self.request(peer, proto, data).await
    }

    async fn request_connected_cancellable(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        // Hub delivery cannot dial; its responder retains the accounting until consumed/dropped.
        self.request_cancellable(peer, proto, data, cancellation)
            .await
    }

    async fn notify_connected(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<(), TransportError> {
        if !self.registration.lease.is_live() {
            return Err(TransportError::Closed);
        }
        let (responder, _rx) = Responder::channel();
        self.hub.deliver(
            peer,
            TransportEvent::Request {
                from: self.local,
                proto,
                data,
                responder,
            },
        )
    }

    async fn notify_connected_cancellable(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<(), TransportError> {
        if cancellation.is_cancelled() {
            return Err(TransportError::Cancelled);
        }
        if !self.registration.lease.is_live() {
            return Err(TransportError::Closed);
        }
        let (mut responder, _rx) = Responder::channel();
        // Hub delivery is the entire lower request. Its queued responder holds actual custody
        // until the recipient consumes or discards it; no answer buffer is retained for callers.
        responder._keepalive = cancellation.keepalive();
        self.hub.deliver(
            peer,
            TransportEvent::Request {
                from: self.local,
                proto,
                data,
                responder,
            },
        )
    }

    async fn dial_peer_batch(
        &self,
        peer: PeerId,
        addresses: &[String],
    ) -> Result<Vec<DialSubmission>, TransportError> {
        if addresses.is_empty() || addresses.len() > MAX_PEER_DIAL_BATCH {
            return Err(TransportError::InvalidDialBatch);
        }
        if !self
            .hub
            .state
            .lock()
            .expect("hub mutex poisoned")
            .inboxes
            .contains_key(&peer)
        {
            return Err(TransportError::Unreachable(peer));
        }
        Ok(vec![DialSubmission::Submitted; addresses.len()])
    }

    async fn next_event(&self) -> Option<TransportEvent> {
        let mut guard = self.rx.lock().await;
        guard.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn bytes(s: &'static str) -> Bytes {
        Bytes::from_static(s.as_bytes())
    }
    #[derive(Debug)]
    struct ResponseEffect(tokio::sync::watch::Receiver<bool>);
    #[async_trait]
    impl crate::RequestAuthority for ResponseEffect {
        fn authorized(&self) -> bool {
            *self.0.borrow() && self.0.has_changed().is_ok()
        }
        async fn revoked(&self) {
            let mut signal = self.0.clone();
            while self.authorized() {
                if signal.changed().await.is_err() {
                    return;
                }
            }
        }
    }

    #[tokio::test]
    async fn queued_mem_reply_rechecks_effect_after_handler_handoff() {
        use std::future::Future;
        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let b = hub.join(PeerId::from_u64(2));
        for retire in [false, true] {
            let mut waiting =
                Box::pin(a.request(b.local_peer(), ProtocolId("reply-scope"), bytes("request")));
            std::future::poll_fn(|cx| {
                assert!(waiting.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            let TransportEvent::Request { responder, .. } = b.next_event().await.unwrap() else {
                panic!("expected request")
            };
            let (owner, signal) = tokio::sync::watch::channel(true);
            responder
                .try_respond_with_authority(bytes("reply"), Arc::new(ResponseEffect(signal)))
                .unwrap();
            if retire {
                drop(owner);
            } else {
                assert_eq!(waiting.await.unwrap(), bytes("reply"));
                continue;
            }
            assert!(matches!(waiting.await, Err(TransportError::NoResponse)));
        }
    }

    #[tokio::test]
    async fn connection_lease_replacement_and_actual_last_handle_drop_are_irreversible() {
        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let b = hub.join(PeerId::from_u64(2));
        let old = a.connection_lease(b.local_peer()).unwrap();
        let replacement = hub.join(b.local_peer());
        old.revoked().await;
        assert!(!old.is_live());
        let next = a.connection_lease(b.local_peer()).unwrap();
        assert!(next.is_live() && !next.is_same(&old));
        drop(b);
        assert!(
            next.is_live(),
            "old registration drop cannot revoke replacement"
        );
        let retained = replacement.clone();
        drop(replacement);
        assert!(next.is_live());
        drop(retained);
        next.revoked().await;
        assert!(a.connection_lease(PeerId::from_u64(2)).is_none());
        assert!(a.connection_snapshot().is_empty());
    }
    #[tokio::test]
    async fn connection_lease_local_retirement_refuses_old_worker_without_revoking_other_members() {
        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let b = hub.join(PeerId::from_u64(2));
        let c = hub.join(PeerId::from_u64(3));
        let ab = a.connection_lease(b.local_peer()).unwrap();
        let cb = c.connection_lease(b.local_peer()).unwrap();
        assert!(ab.is_same(&a.connection_lease(b.local_peer()).unwrap()));
        let next = a.rejoin();
        ab.revoked().await;
        assert!(!ab.is_live() && cb.is_live());
        assert!(a.connection_snapshot().is_empty());
        assert!(a.connection_lease(b.local_peer()).is_none());
        assert!(matches!(
            a.request(b.local_peer(), ProtocolId("old"), Bytes::new())
                .await,
            Err(TransportError::Closed)
        ));
        let (_publisher, signal) = tokio::sync::watch::channel(false);
        assert!(matches!(
            a.request_connected_cancellable(
                b.local_peer(),
                ProtocolId("old"),
                Bytes::new(),
                RequestCancellation::new(signal, None)
            )
            .await,
            Err(TransportError::Cancelled)
        ));
        let next_ab = next.connection_lease(b.local_peer()).unwrap();
        assert!(next_ab.is_live() && !next_ab.is_same(&ab));
        drop(next);
        next_ab.revoked().await;
        assert!(!next_ab.is_live() && cb.is_live());
    }

    #[tokio::test]
    async fn publish_once_has_no_deferred_retry_and_never_echoes_to_self() {
        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let b = hub.join(PeerId::from_u64(2));
        let topic = Topic::new("one-shot");
        a.subscribe(topic.clone()).await.unwrap();
        assert_eq!(
            a.publish_once(topic.clone(), bytes("old")).await,
            Err(PublishOnceError::NoPeers)
        );
        b.subscribe(topic.clone()).await.unwrap();
        drop(a.publish_once(topic.clone(), bytes("unpolled")));
        assert!(b.rx.lock().await.try_recv().is_err());
        assert_eq!(
            a.publish_once(topic.clone(), bytes("current")).await,
            Ok(PublishSubmission::Submitted)
        );
        match b.rx.lock().await.try_recv().unwrap() {
            TransportEvent::Gossip {
                topic: actual,
                from,
                data,
            } => {
                assert_eq!(actual, topic);
                assert_eq!(from, a.local_peer());
                assert_eq!(data, bytes("current"));
            }
            _ => panic!("expected gossip"),
        }
        assert!(a.rx.lock().await.try_recv().is_err());
        assert!(b.rx.lock().await.try_recv().is_err());
    }

    #[tokio::test]
    async fn publish_once_checks_exact_input_limits_and_closed_subscribers() {
        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let b = hub.join(PeerId::from_u64(2));
        let topic = Topic::new(vec![1; MAX_PUBLISH_ONCE_TOPIC_BYTES]);
        b.subscribe(topic.clone()).await.unwrap();
        assert_eq!(
            a.publish_once(
                topic.clone(),
                Bytes::from(vec![0; MAX_PUBLISH_ONCE_BYTES + 1])
            )
            .await,
            Err(PublishOnceError::TooLarge)
        );
        assert_eq!(
            a.publish_once(
                Topic::new(vec![1; MAX_PUBLISH_ONCE_TOPIC_BYTES + 1]),
                bytes("x")
            )
            .await,
            Err(PublishOnceError::TooLarge)
        );
        assert!(b.rx.lock().await.try_recv().is_err());
        assert_eq!(
            a.publish_once(topic.clone(), Bytes::from(vec![0; MAX_PUBLISH_ONCE_BYTES]))
                .await,
            Ok(PublishSubmission::Submitted)
        );
        assert!(b.rx.lock().await.try_recv().is_ok());
        drop(b);
        assert_eq!(
            a.publish_once(topic, bytes("gone")).await,
            Err(PublishOnceError::NoPeers)
        );
    }

    #[tokio::test]
    async fn gossip_reaches_other_subscribers() {
        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let b = hub.join(PeerId::from_u64(2));

        let topic = Topic::new("channel-general");
        b.subscribe(topic.clone()).await.unwrap();

        a.publish(topic.clone(), bytes("hello")).await.unwrap();

        match b.next_event().await {
            Some(TransportEvent::Gossip { from, data, .. }) => {
                assert_eq!(from, PeerId::from_u64(1));
                assert_eq!(data, bytes("hello"));
            }
            other => panic!("expected gossip, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn publisher_does_not_receive_its_own_gossip() {
        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));

        let topic = Topic::new("loopback");
        a.subscribe(topic.clone()).await.unwrap();
        a.publish(topic, bytes("echo?")).await.unwrap();

        // Nothing should be queued for the publisher; a request to a missing peer
        // returns promptly, proving no self-gossip is pending ahead of it.
        let err = a
            .request(PeerId::from_u64(99), ProtocolId("x"), bytes("q"))
            .await
            .unwrap_err();
        assert!(matches!(err, TransportError::Unreachable(_)));
    }

    #[tokio::test]
    async fn request_response_roundtrips() {
        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let b = hub.join(PeerId::from_u64(2));

        let proto = ProtocolId("anti-entropy");
        let (reply, served) =
            tokio::join!(a.request(b.local_peer(), proto, bytes("ping")), async {
                match b.next_event().await {
                    Some(TransportEvent::Request {
                        from,
                        proto,
                        data,
                        responder,
                    }) => {
                        responder.respond(bytes("pong"));
                        Some((from, proto, data))
                    }
                    _ => None,
                }
            });

        assert_eq!(reply.unwrap(), bytes("pong"));
        let (from, p, data) = served.expect("server handled a request");
        assert_eq!(from, PeerId::from_u64(1));
        assert_eq!(p, proto);
        assert_eq!(data, bytes("ping"));
    }

    #[tokio::test]
    async fn cancelled_request_retains_accounting_with_the_queued_in_memory_responder() {
        #[derive(Debug)]
        struct DropObserved(Arc<AtomicUsize>);
        impl Drop for DropObserved {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let b = hub.join(PeerId::from_u64(2));
        let drops = Arc::new(AtomicUsize::new(0));
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        let cancellation =
            RequestCancellation::new(receiver, Some(Arc::new(DropObserved(Arc::clone(&drops)))));
        let requester = a.clone();
        let peer = b.local_peer();
        let request = tokio::spawn(async move {
            requester
                .request_cancellable(peer, ProtocolId("bounded"), bytes("request"), cancellation)
                .await
        });
        let responder = match b.next_event().await {
            Some(TransportEvent::Request { responder, .. }) => responder,
            other => panic!("expected request, got {other:?}"),
        };

        cancel.send(true).unwrap();
        assert!(matches!(
            request.await.unwrap(),
            Err(TransportError::Cancelled)
        ));
        assert_eq!(
            drops.load(Ordering::SeqCst),
            0,
            "the queued request, not its cancelled waiter, owns the accounting lifetime"
        );
        responder.respond(bytes("late"));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn abandoned_request_retires_reply_authority_before_queued_event_is_consumed() {
        #[derive(Debug)]
        struct DropObserved(Arc<AtomicUsize>);
        impl Drop for DropObserved {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        use std::future::Future;

        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let b = hub.join(PeerId::from_u64(2));
        let drops = Arc::new(AtomicUsize::new(0));
        let (_cancel, signal) = tokio::sync::watch::channel(false);
        let cancellation =
            RequestCancellation::new(signal, Some(Arc::new(DropObserved(drops.clone()))));
        let mut request = Box::pin(a.request_cancellable(
            b.local_peer(),
            ProtocolId("deferred"),
            bytes("request"),
            cancellation,
        ));
        std::future::poll_fn(|cx| {
            assert!(request.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        // The original caller disappears before its peer even dequeues the event. No observer
        // or forwarding task runs between this drop and the driver's synchronous gate check.
        drop(request);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let responder = match b.next_event().await {
            Some(TransportEvent::Request { responder, .. }) => responder,
            other => panic!("expected queued request, got {other:?}"),
        };
        let authority = responder.cancellation_authority();
        assert!(!authority.authorized());
        authority.revoked().await;
        assert_eq!(
            drops.load(Ordering::SeqCst),
            0,
            "actual event still owns custody"
        );
        drop(responder);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn request_to_unknown_peer_is_unreachable() {
        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let err = a
            .request(PeerId::from_u64(404), ProtocolId("x"), bytes("q"))
            .await
            .unwrap_err();
        assert!(matches!(err, TransportError::Unreachable(_)));
    }

    #[tokio::test]
    async fn connected_only_notification_is_queued_without_waiting_for_a_reply() {
        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let b = hub.join(PeerId::from_u64(2));

        a.notify_connected(b.local_peer(), ProtocolId("repair"), bytes("please-dial"))
            .await
            .unwrap();
        match b.next_event().await {
            Some(TransportEvent::Request {
                from, proto, data, ..
            }) => {
                assert_eq!(from, a.local_peer());
                assert_eq!(proto, ProtocolId("repair"));
                assert_eq!(data, bytes("please-dial"));
            }
            other => panic!("expected connected-only notification, got {other:?}"),
        }

        assert!(matches!(
            a.notify_connected(
                PeerId::from_u64(404),
                ProtocolId("repair"),
                bytes("no implicit dial"),
            )
            .await,
            Err(TransportError::Unreachable(_))
        ));
    }

    #[tokio::test]
    async fn peer_bound_batch_is_small_nonempty_and_requires_a_live_hub_peer() {
        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let b = hub.join(PeerId::from_u64(2));
        let routes = vec!["route-v4".to_string(), "route-v6".to_string()];

        assert_eq!(
            a.dial_peer_batch(b.local_peer(), &routes).await.unwrap(),
            vec![DialSubmission::Submitted, DialSubmission::Submitted]
        );
        assert!(matches!(
            a.dial_peer_batch(b.local_peer(), &[]).await,
            Err(TransportError::InvalidDialBatch)
        ));
        assert!(matches!(
            a.dial_peer_batch(b.local_peer(), &["a".into(), "b".into(), "c".into()],)
                .await,
            Err(TransportError::InvalidDialBatch)
        ));
        assert!(matches!(
            a.dial_peer_batch(PeerId::from_u64(404), &routes).await,
            Err(TransportError::Unreachable(_))
        ));
    }

    #[tokio::test]
    async fn unsubscribe_stops_delivery() {
        let hub = Hub::new();
        let a = hub.join(PeerId::from_u64(1));
        let b = hub.join(PeerId::from_u64(2));

        let topic = Topic::new("t");
        b.subscribe(topic.clone()).await.unwrap();
        b.unsubscribe(topic.clone()).await.unwrap();
        a.publish(topic, bytes("nope")).await.unwrap();

        // b has no subscription, so a request to a dead peer is the only thing
        // that resolves; confirming no gossip was queued for b.
        let err = b
            .request(PeerId::from_u64(404), ProtocolId("x"), bytes("q"))
            .await
            .unwrap_err();
        assert!(matches!(err, TransportError::Unreachable(_)));
    }
}
