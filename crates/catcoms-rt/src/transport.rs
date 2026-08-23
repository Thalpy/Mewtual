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

/// A peer surfaced by rendezvous discovery (rt-native, libp2p-free). `peer` is the discovered
/// node's opaque transport-id bytes (the same encoding passed back to `dial`/used as a dedup key);
/// `addresses` are its advertised dialable addresses; `namespace` is the rendezvous namespace it
/// was found under. Surfaced only; the discovery/dial policy above the transport decides what to
/// dial (the transport never auto-dials a discovered record).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredPeer {
    /// The discovered node's opaque transport-id bytes.
    pub peer: Vec<u8>,
    /// Its advertised dialable addresses.
    pub addresses: Vec<String>,
    /// The rendezvous namespace it was discovered under.
    pub namespace: String,
    /// The record's own signed sequence number, for the discovery policy's anti-replay freshness.
    pub seq: u64,
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

    /// Send an addressed message to `peer` **without waiting for a reply**, returning as soon as
    /// it is queued for sending.
    ///
    /// For traffic whose reply carries no information: call signalling, where the receiver queues
    /// the payload and deliberately never answers with data. The distinction is not cosmetic.
    /// [`MeshTransport::request`] parks its caller until the remote answers or the request/response
    /// timeout fires, which is seconds against a peer that has gone away; a caller driving an actor
    /// loop stalls every other thing that loop serves for that whole window, including the
    /// disconnect handling and re-dial that would have repaired the route. Delivery problems
    /// surface in the transport's own logs rather than in this return value, because there is no
    /// useful answer to give a caller that is not allowed to wait.
    ///
    /// The default delegates to `request` and discards the reply, which is what a transport whose
    /// requests complete immediately (the in-memory one used by tests) wants anyway.
    async fn notify(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<(), TransportError> {
        self.request(peer, proto, data).await.map(|_| ())
    }

    /// Await the next inbound event. Returns `None` once the transport is closed.
    /// Intended to be driven by a single consumer task.
    async fn next_event(&self) -> Option<TransportEvent>;

    // --- rendezvous discovery (optional; default no-op for transports without it) -----------
    //
    // These let a higher layer drive steady-state rendezvous discovery generically. `rz_node` is
    // the rendezvous node's opaque transport-id bytes; `namespace` is a (member-only-derived)
    // rendezvous namespace; addresses are dialable address strings. The defaults make a transport
    // without rendezvous support inert: the control verbs succeed as no-ops and `next_discovered`
    // never resolves (so a `select!` arm awaiting it simply never fires; returning `None` would
    // busy-loop the caller's loop).

    /// Register our advertised external addresses under `namespace` at rendezvous `rz_node`.
    async fn rendezvous_register(
        &self,
        _namespace: &str,
        _rz_node: &[u8],
    ) -> Result<(), TransportError> {
        Ok(())
    }

    /// Ask `rz_node` for peers registered under `namespace`; results surface via
    /// [`MeshTransport::next_discovered`] and are NEVER auto-dialed.
    async fn rendezvous_discover(
        &self,
        _namespace: &str,
        _rz_node: &[u8],
    ) -> Result<(), TransportError> {
        Ok(())
    }

    /// Dial a peer address string at runtime (the higher layer's chosen dial, post-policy).
    async fn dial_addr(&self, _addr: &str) -> Result<(), TransportError> {
        Ok(())
    }

    /// Advertise `addr` as an externally-reachable address, so a rendezvous registration can flush.
    async fn add_external_addr(&self, _addr: &str) -> Result<(), TransportError> {
        Ok(())
    }

    /// Await the next rendezvous-discovered peer. The default never resolves (a transport without
    /// rendezvous never surfaces one), so a `select!` arm awaiting it is inert.
    async fn next_discovered(&self) -> Option<DiscoveredPeer> {
        std::future::pending().await
    }

    // --- eviction (optional; default no-op for transports without a connection to sever) ------

    /// **Evict** `peer`: sever any live connection to it and refuse new ones from it.
    ///
    /// The membership layer calls this when a Remove commit is applied. Rotating the routing
    /// secret takes the removed member's *keys* away; without this it stays **attached**, which
    /// is what lets it keep a granted circuit reservation and keep observing that the group is
    /// active over a link it already holds. That is not a removal.
    ///
    /// **Best-effort by construction, in two ways, and neither is a bug to be fixed here.**
    ///
    /// 1. The caller only knows the peer id the removed device **asserted about itself** (the
    ///    `peer_id` field of its own signed peer record). The signature binds that value to its
    ///    signer, but nothing binds it to *naming* its signer, so the value is attacker-chosen:
    ///    a member that lied about its peer id evades the disconnect, and one that named a third
    ///    party can aim it at somebody else. Binding a device key to a transport identity is a
    ///    documented deferral; until it lands, the caller is responsible for the checks that
    ///    make acting on the value safe (see `ChannelSync::queue_eviction`), and an implementor
    ///    of this trait must refuse to evict any peer its own configuration relies on.
    /// 2. A transport with no notion of a connection (the in-memory test network) cannot honour
    ///    it at all, which is why the default is inert rather than an error: a caller must not
    ///    have to know which transport it is on, and a failure here must never abort a removal
    ///    that has already been committed to the MLS group.
    ///
    /// Treat it as defence in depth on top of the key rotation, never as the thing that keeps a
    /// removed member out.
    async fn evict_peer(&self, _peer: PeerId) -> Result<(), TransportError> {
        Ok(())
    }

    /// Lift an eviction, because `peer`'s device has been **admitted to the group again**.
    ///
    /// Removal is not the end of a relationship in this product: it ships a re-invite, and a
    /// node's transport identity is stable across restarts, so without this a re-invited member
    /// would dial its inviter, be refused at the connection handler, and see the join time out
    /// with nothing to diagnose. Every peer holding the old eviction would refuse it too.
    ///
    /// The membership layer, not a timer, decides when this fires: readmission is an
    /// authenticated group event, and elapsed time is not evidence of anything.
    async fn unevict_peer(&self, _peer: PeerId) -> Result<(), TransportError> {
        Ok(())
    }
}
