//! The [`MeshTransport`] seam: pub/sub fan-out plus addressed request/response.
//!
//! This is the altitude the replication and group layers actually need:
//! - **gossip** (publish/subscribe) for live CRDT-delta fan-out, and
//! - **request/response** for anti-entropy catch-up and on-demand blob fetch.
//!
//! It maps cleanly onto rust-libp2p (gossipsub + request-response behaviours) in
//! production and onto the in-memory [`crate::mem::MemNetwork`] in tests.

use async_trait::async_trait;
use bytes::Bytes;
use std::fmt;
use thiserror::Error;
use tokio::sync::oneshot;

/// A stable peer identifier. In production this is derived from a libp2p public
/// key; here it is an opaque 32-byte value so tests need no real keys.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PeerId([u8; 32]);

impl PeerId {
    /// Wrap a raw 32-byte identifier.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// A deterministic test peer id derived from a small integer.
    pub fn from_u64(n: u64) -> Self {
        let mut b = [0u8; 32];
        b[24..].copy_from_slice(&n.to_be_bytes());
        Self(b)
    }

    /// The raw 32 bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PeerId({:02x}{:02x}{:02x}{:02x}…)",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
}

/// A pub/sub topic. In production this is a *blinded* topic id derived from the
/// group's metadata key; here it is an opaque label.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Topic(Bytes);

impl Topic {
    /// Construct a topic from any byte source.
    pub fn new(bytes: impl Into<Bytes>) -> Self {
        Self(bytes.into())
    }

    /// The raw topic bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Topic({})", String::from_utf8_lossy(&self.0))
    }
}

/// A request/response protocol selector (e.g. anti-entropy, blob-fetch).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ProtocolId(pub &'static str);

/// Errors surfaced by a transport.
#[derive(Debug, Error)]
pub enum TransportError {
    /// The target peer is not currently reachable.
    #[error("peer {0:?} not reachable")]
    Unreachable(PeerId),
    /// A request was not answered before its deadline.
    #[error("request to {0:?} timed out")]
    Timeout(PeerId),
    /// The transport has shut down.
    #[error("transport closed")]
    Closed,
    /// The remote received a request but dropped it without replying.
    #[error("request handler dropped without responding")]
    NoResponse,
}

/// Reply handle handed to a request handler. Dropping it without calling
/// [`Responder::respond`] surfaces [`TransportError::NoResponse`] to the caller.
#[derive(Debug)]
pub struct Responder(pub(crate) oneshot::Sender<Bytes>);

impl Responder {
    /// Send the reply back to the requester.
    pub fn respond(self, data: Bytes) {
        let _ = self.0.send(data);
    }

    /// Create a responder paired with its receiver. A transport implementation
    /// hands the [`Responder`] to the request handler (inside a
    /// [`TransportEvent::Request`]) and keeps the [`ResponderRx`] to await the
    /// reply and forward it over the wire.
    pub fn channel() -> (Responder, ResponderRx) {
        let (tx, rx) = oneshot::channel();
        (Responder(tx), ResponderRx(rx))
    }
}

/// The receiving half of a [`Responder`], held by a transport implementation.
#[derive(Debug)]
pub struct ResponderRx(oneshot::Receiver<Bytes>);

impl ResponderRx {
    /// Await the reply, or `None` if the responder was dropped without replying.
    pub async fn recv(self) -> Option<Bytes> {
        self.0.await.ok()
    }
}

/// An inbound transport event, drained via [`MeshTransport::next_event`].
#[derive(Debug)]
pub enum TransportEvent {
    /// A gossip message delivered on a subscribed topic.
    Gossip {
        /// The topic it arrived on.
        topic: Topic,
        /// The originating peer.
        from: PeerId,
        /// The payload.
        data: Bytes,
    },
    /// An inbound request awaiting a reply via `responder`.
    Request {
        /// The requesting peer.
        from: PeerId,
        /// The protocol the request was sent on.
        proto: ProtocolId,
        /// The request payload.
        data: Bytes,
        /// Reply handle.
        responder: Responder,
    },
    /// A peer became reachable.
    PeerConnected(PeerId),
    /// A peer became unreachable.
    PeerDisconnected(PeerId),
}

/// The messaging seam. Outbound operations take `&self` so the transport can be
/// shared (e.g. behind an `Arc`); [`MeshTransport::next_event`] is single-consumer.
#[async_trait]
pub trait MeshTransport: Send + Sync {
    /// This node's peer id.
    fn local_peer(&self) -> PeerId;

    /// Start receiving gossip on `topic`.
    async fn subscribe(&self, topic: Topic) -> Result<(), TransportError>;

    /// Stop receiving gossip on `topic`.
    async fn unsubscribe(&self, topic: Topic) -> Result<(), TransportError>;

    /// Fan a message out to every other subscriber of `topic` (best-effort).
    async fn publish(&self, topic: Topic, data: Bytes) -> Result<(), TransportError>;

    /// Send an addressed request to `peer` and await its reply.
    async fn request(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError>;

    /// Await the next inbound event. Returns `None` once the transport is closed.
    /// Intended to be driven by a single consumer task.
    async fn next_event(&self) -> Option<TransportEvent>;
}
