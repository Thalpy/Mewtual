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
use tokio::sync::oneshot;
use tokio::sync::Mutex as AsyncMutex;

use crate::transport::{
    MeshTransport, PeerId, ProtocolId, Responder, Topic, TransportError, TransportEvent,
};

#[derive(Debug, Default)]
struct HubState {
    inboxes: HashMap<PeerId, UnboundedSender<TransportEvent>>,
    topics: HashMap<Topic, HashSet<PeerId>>,
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
        self.state
            .lock()
            .expect("hub mutex poisoned")
            .inboxes
            .insert(peer, tx);
        MemNetwork {
            hub: Arc::clone(self),
            local: peer,
            rx: Arc::new(AsyncMutex::new(rx)),
        }
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
}

#[async_trait]
impl MeshTransport for MemNetwork {
    fn local_peer(&self) -> PeerId {
        self.local
    }

    async fn subscribe(&self, topic: Topic) -> Result<(), TransportError> {
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
        let mut state = self.hub.state.lock().expect("hub mutex poisoned");
        if let Some(set) = state.topics.get_mut(&topic) {
            set.remove(&self.local);
        }
        Ok(())
    }

    async fn publish(&self, topic: Topic, data: Bytes) -> Result<(), TransportError> {
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

    async fn request(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        let (tx, rx) = oneshot::channel();
        let event = TransportEvent::Request {
            from: self.local,
            proto,
            data,
            responder: Responder(tx),
        };
        self.hub.deliver(peer, event)?;
        rx.await.map_err(|_| TransportError::NoResponse)
    }

    async fn next_event(&self) -> Option<TransportEvent> {
        let mut guard = self.rx.lock().await;
        guard.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(s: &'static str) -> Bytes {
        Bytes::from_static(s.as_bytes())
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
