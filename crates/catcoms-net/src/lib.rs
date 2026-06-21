//! libp2p mesh networking (Phase 6a).
//!
//! [`MeshService`] realizes the Phase-0 [`catcoms_rt::MeshTransport`] seam over
//! **real libp2p** (Noise + yamux, gossipsub for topic fan-out, request/response
//! for addressed exchanges). The whole stack above — encrypted CRDT replication,
//! blob fetch — can therefore run unchanged over either the in-memory test
//! transport or this libp2p mesh.
//!
//! A background actor task owns the libp2p `Swarm`; the handle talks to it over
//! channels (commands in, [`catcoms_rt::TransportEvent`]s out). Phase-0 ids map to
//! libp2p ids internally (a peer is addressed by a BLAKE3 of its libp2p PeerId,
//! with a live map back to the real PeerId for dialing), and topics are
//! hex-encoded so arbitrary (blinded) topic bytes are valid gossipsub topics.
//!
//! Circuit-relay v2 (reserve a slot, be dialed via a circuit address) and DCUtR
//! hole-punching (upgrade a relayed link to a direct one) are wired in: a relayed
//! connection automatically attempts a direct upgrade, and the upgrade is surfaced
//! via [`MeshService::next_direct_upgrade`]. A rendezvous **client** is wired too:
//! [`MeshService::rendezvous_register`]/[`MeshService::rendezvous_discover`] register
//! under and discover blinded namespaces; discovered records are *surfaced*
//! ([`MeshService::next_discovered`]) but **never auto-dialed** — a higher layer
//! decides whether to dial (where eclipse-resistance lives).
//!
//! Still to come in later mesh sub-blocks: the member-verifiable discovery tag +
//! eclipse-resistant discovery policy, and the anti-entropy / proposal-commit
//! protocols layered on top of this transport.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use catcoms_rt::{
    MeshTransport, PeerId, ProtocolId, Responder, Topic, TransportError, TransportEvent,
};
use futures::stream::FuturesUnordered;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, StreamExt};
use libp2p::core::transport::MemoryTransport;
use libp2p::core::upgrade::Version;
use libp2p::multiaddr::Protocol;
use libp2p::request_response::{OutboundRequestId, ResponseChannel};
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{
    connection_limits, dcutr, gossipsub, identify, noise, ping, relay, rendezvous,
    request_response, tcp, yamux, Multiaddr, StreamProtocol, Swarm, SwarmBuilder, Transport,
};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot, Mutex};

/// Max request/response frame size.
const MAX_FRAME: usize = 16 * 1024 * 1024;

/// The request/response protocol id (anti-entropy / blob fetch).
const RR_PROTOCOL: &str = "/catcoms/rr/1";

/// Errors building or driving the mesh.
#[derive(Debug, Error)]
pub enum NetError {
    /// Failed to build the libp2p transport/swarm.
    #[error("swarm build error: {0}")]
    Build(String),
    /// Failed to start listening.
    #[error("listen error: {0}")]
    Listen(String),
    /// Failed to dial a peer.
    #[error("dial error: {0}")]
    Dial(String),
    /// A rendezvous register/discover could not be issued (bad namespace, or the
    /// transport actor has stopped).
    #[error("rendezvous error: {0}")]
    Rendezvous(String),
}

/// A peer record surfaced by a rendezvous **discovery** — the (signed) peer id and
/// the addresses it advertised, under a blinded namespace. The transport only
/// *surfaces* these; it never auto-dials them (a higher layer decides whether and
/// when to dial, which is where eclipse-resistance lives).
#[derive(Debug, Clone)]
pub struct Discovered {
    /// The discovered peer (the signer of the record — authenticity is verified by
    /// libp2p when decoding the signed peer record).
    pub peer: libp2p::PeerId,
    /// The addresses the peer advertised.
    pub addresses: Vec<Multiaddr>,
    /// The namespace it was discovered under.
    pub namespace: String,
}

/// Confirmation that our own peer record was **registered** at a rendezvous node,
/// with the granted TTL (the caller schedules re-registration before it expires).
#[derive(Debug, Clone)]
pub struct Registered {
    /// The namespace we registered under.
    pub namespace: String,
    /// The granted time-to-live, in seconds.
    pub ttl: u64,
    /// The rendezvous node that granted it.
    pub rendezvous_node: libp2p::PeerId,
}

// ----- request/response codec ------------------------------------------------

/// A minimal length-prefixed raw-bytes codec for request/response.
#[derive(Clone, Default, Debug)]
pub struct BytesCodec;

async fn read_frame<T: AsyncRead + Unpin + Send>(io: &mut T) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    io.read_exact(&mut len).await?;
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut buf = vec![0u8; n];
    io.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_frame<T: AsyncWrite + Unpin + Send>(io: &mut T, data: &[u8]) -> io::Result<()> {
    let len = u32::try_from(data.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))?;
    io.write_all(&len.to_be_bytes()).await?;
    io.write_all(data).await?;
    io.close().await?;
    Ok(())
}

#[async_trait::async_trait]
impl request_response::Codec for BytesCodec {
    type Protocol = StreamProtocol;
    type Request = Vec<u8>;
    type Response = Vec<u8>;

    async fn read_request<T>(&mut self, _: &StreamProtocol, io: &mut T) -> io::Result<Vec<u8>>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_frame(io).await
    }

    async fn read_response<T>(&mut self, _: &StreamProtocol, io: &mut T) -> io::Result<Vec<u8>>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_frame(io).await
    }

    async fn write_request<T>(
        &mut self,
        _: &StreamProtocol,
        io: &mut T,
        req: Vec<u8>,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_frame(io, &req).await
    }

    async fn write_response<T>(
        &mut self,
        _: &StreamProtocol,
        io: &mut T,
        resp: Vec<u8>,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_frame(io, &resp).await
    }
}

// ----- behaviour & swarm construction ---------------------------------------

/// The protocol string advertised by `identify`.
const IDENTIFY_PROTO: &str = "/catcoms/id/1";

/// The mesh node's libp2p behaviours. `relay_client` + `dcutr` + `identify` give
/// NAT traversal: a node can reserve a slot on a relay, be dialed via a circuit
/// address, and then hole-punch to a direct connection.
#[derive(NetworkBehaviour)]
#[allow(missing_debug_implementations)]
pub struct MeshBehaviour {
    /// Topic fan-out.
    pub gossipsub: gossipsub::Behaviour,
    /// Addressed request/response.
    pub request_response: request_response::Behaviour<BytesCodec>,
    /// Relay-client: reserve a slot on a relay; be reachable via a circuit address.
    pub relay_client: relay::client::Behaviour,
    /// Direct Connection Upgrade through Relay (hole punching).
    pub dcutr: dcutr::Behaviour,
    /// Address discovery (observed external address; required by DCUtR).
    pub identify: identify::Behaviour,
    /// Rendezvous client: register our (signed) peer record under a blinded namespace
    /// and discover other members, without hard-coded bootstrap addresses.
    pub rendezvous_client: rendezvous::client::Behaviour,
    /// Connection caps so a discovery/registration flood cannot exhaust us.
    pub connection_limits: connection_limits::Behaviour,
}

impl MeshBehaviour {
    fn new(
        key: &libp2p::identity::Keypair,
        relay_client: relay::client::Behaviour,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(key.clone()),
            gossipsub::Config::default(),
        )?;
        let request_response = request_response::Behaviour::with_codec(
            BytesCodec,
            [(
                StreamProtocol::new(RR_PROTOCOL),
                request_response::ProtocolSupport::Full,
            )],
            request_response::Config::default(),
        );
        let dcutr = dcutr::Behaviour::new(key.public().to_peer_id());
        let identify = identify::Behaviour::new(identify::Config::new(
            IDENTIFY_PROTO.to_string(),
            key.public(),
        ));
        let rendezvous_client = rendezvous::client::Behaviour::new(key.clone());
        let connection_limits = connection_limits::Behaviour::new(
            connection_limits::ConnectionLimits::default()
                .with_max_pending_incoming(Some(64))
                .with_max_established_per_peer(Some(8)),
        );
        Ok(Self {
            gossipsub,
            request_response,
            relay_client,
            dcutr,
            identify,
            rendezvous_client,
            connection_limits,
        })
    }
}

/// A relay **server**'s behaviours: forward circuit traffic between peers that
/// cannot connect directly. Never sees plaintext (Noise + MLS ciphertext only).
#[derive(NetworkBehaviour)]
#[allow(missing_debug_implementations)]
pub struct RelayBehaviour {
    /// The circuit-relay-v2 server.
    pub relay: relay::Behaviour,
    /// Address discovery for clients reserving slots.
    pub identify: identify::Behaviour,
    /// Keep-alive.
    pub ping: ping::Behaviour,
}

/// Build a swarm over the in-memory transport (deterministic local testing),
/// relay-client capable.
pub fn build_memory_swarm() -> Swarm<MeshBehaviour> {
    SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_other_transport(|key| {
            MemoryTransport::default()
                .upgrade(Version::V1)
                .authenticate(noise::Config::new(key).expect("noise config"))
                .multiplex(yamux::Config::default())
        })
        .expect("memory transport")
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .expect("relay client")
        .with_behaviour(MeshBehaviour::new)
        .expect("behaviour")
        .build()
}

/// Build a swarm over TCP (Noise + yamux), relay-client capable, for real networking.
pub fn build_tcp_swarm() -> Result<Swarm<MeshBehaviour>, NetError> {
    let swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_behaviour(MeshBehaviour::new)
        .map_err(|e| NetError::Build(e.to_string()))?
        .build();
    Ok(swarm)
}

/// Build a TCP **relay-server** swarm (forwards circuit traffic for clients behind
/// NAT). Run it with [`run_relay`].
pub fn build_relay_swarm() -> Result<Swarm<RelayBehaviour>, NetError> {
    let swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_behaviour(relay_behaviour)
        .map_err(|e| NetError::Build(e.to_string()))?
        .build();
    Ok(swarm)
}

/// Build a relay-server swarm over the in-memory transport (deterministic tests).
pub fn build_memory_relay_swarm() -> Swarm<RelayBehaviour> {
    SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_other_transport(|key| {
            MemoryTransport::default()
                .upgrade(Version::V1)
                .authenticate(noise::Config::new(key).expect("noise config"))
                .multiplex(yamux::Config::default())
        })
        .expect("memory transport")
        .with_behaviour(relay_behaviour)
        .expect("relay behaviour")
        .build()
}

fn relay_behaviour(key: &libp2p::identity::Keypair) -> RelayBehaviour {
    RelayBehaviour {
        relay: relay::Behaviour::new(key.public().to_peer_id(), relay::Config::default()),
        identify: identify::Behaviour::new(identify::Config::new(
            IDENTIFY_PROTO.to_string(),
            key.public(),
        )),
        ping: ping::Behaviour::default(),
    }
}

/// Run a relay-server swarm's event loop: forward circuit traffic indefinitely.
/// (Relays only ever route Noise + MLS ciphertext — zero-knowledge.)
///
/// Each bound listen address is registered as an **external address** so granted
/// reservations carry a usable relayed address (otherwise a client's circuit
/// listener closes with `NoAddressesInReservation`). A production relay on a
/// public IP behind 0.0.0.0 should additionally have its real public address added
/// — pass it via [`run_relay_with_external`].
pub async fn run_relay(swarm: Swarm<RelayBehaviour>) {
    run_relay_with_external(swarm, Vec::new()).await
}

/// Like [`run_relay`] but also advertises each address in `external` (e.g. the
/// relay's real public `/ip4/.../tcp/...` when listening on `0.0.0.0`).
pub async fn run_relay_with_external(mut swarm: Swarm<RelayBehaviour>, external: Vec<Multiaddr>) {
    for addr in external {
        swarm.add_external_address(addr);
    }
    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => {
                // Advertise the bound address so reservations can hand it to clients.
                swarm.add_external_address(address.clone());
                tracing::info!(%address, "relay listening");
            }
            SwarmEvent::Behaviour(RelayBehaviourEvent::Relay(e)) => {
                tracing::debug!(?e, "relay event");
            }
            _ => {}
        }
    }
}

// ----- rendezvous server -----------------------------------------------------

/// A rendezvous **server**'s behaviours: members register their (signed) peer
/// records under a blinded namespace and discover each other, without the server
/// learning group identity or content. Zero-knowledge like the relay — it only sees
/// opaque namespace strings and signed peer records (member addresses + a TTL).
#[derive(NetworkBehaviour)]
#[allow(missing_debug_implementations)]
pub struct RendezvousBehaviour {
    /// The rendezvous registration/discovery protocol.
    pub rendezvous: rendezvous::server::Behaviour,
    /// Address discovery (lets a registering client learn its observed address).
    pub identify: identify::Behaviour,
    /// Keep-alive.
    pub ping: ping::Behaviour,
    /// Connection caps so a registration/discovery flood cannot exhaust the server.
    pub connection_limits: connection_limits::Behaviour,
}

fn rendezvous_behaviour(key: &libp2p::identity::Keypair) -> RendezvousBehaviour {
    // Storage caps bound a registration flood; the spec-recommended TTL band (2h–72h)
    // is the default. A per-PeerId request-rate token bucket is a hardening follow-up.
    let config = rendezvous::server::Config::default()
        .with_max_registration_per_peer(128)
        .with_max_registration_total(16_384)
        .with_max_stored_cookies(4_096);
    RendezvousBehaviour {
        rendezvous: rendezvous::server::Behaviour::new(config),
        identify: identify::Behaviour::new(identify::Config::new(
            IDENTIFY_PROTO.to_string(),
            key.public(),
        )),
        ping: ping::Behaviour::default(),
        connection_limits: connection_limits::Behaviour::new(
            connection_limits::ConnectionLimits::default()
                .with_max_pending_incoming(Some(64))
                .with_max_established_incoming(Some(4_096))
                .with_max_established_per_peer(Some(8)),
        ),
    }
}

/// Build a TCP rendezvous-server swarm. Run it with [`run_rendezvous`].
pub fn build_rendezvous_swarm() -> Result<Swarm<RendezvousBehaviour>, NetError> {
    let swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_behaviour(rendezvous_behaviour)
        .map_err(|e| NetError::Build(e.to_string()))?
        .build();
    Ok(swarm)
}

/// Build a rendezvous-server swarm over the in-memory transport (deterministic tests).
pub fn build_memory_rendezvous_swarm() -> Swarm<RendezvousBehaviour> {
    SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_other_transport(|key| {
            MemoryTransport::default()
                .upgrade(Version::V1)
                .authenticate(noise::Config::new(key).expect("noise config"))
                .multiplex(yamux::Config::default())
        })
        .expect("memory transport")
        .with_behaviour(rendezvous_behaviour)
        .expect("rendezvous behaviour")
        .build()
}

/// Run a rendezvous-server swarm's event loop indefinitely: register members and
/// answer discovery under blinded namespaces. The server never learns group identity
/// or content — only opaque namespace strings and signed peer records.
pub async fn run_rendezvous(mut swarm: Swarm<RendezvousBehaviour>) {
    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => {
                tracing::info!(%address, "rendezvous listening");
            }
            SwarmEvent::Behaviour(RendezvousBehaviourEvent::Rendezvous(e)) => match &e {
                rendezvous::server::Event::PeerRegistered { peer, .. } => {
                    tracing::info!(%peer, "rendezvous: peer registered");
                }
                _ => tracing::debug!(?e, "rendezvous event"),
            },
            _ => {}
        }
    }
}

// ----- mapping helpers -------------------------------------------------------

fn to_peer(p: &libp2p::PeerId) -> PeerId {
    PeerId::new(*blake3::hash(&p.to_bytes()).as_bytes())
}

/// The Phase-0 [`PeerId`] for a libp2p peer (a BLAKE3 of its bytes) — how every
/// layer above the transport addresses it.
pub fn phase0_peer_id(p: &libp2p::PeerId) -> PeerId {
    to_peer(p)
}

/// The **target** peer of a multiaddr: the last `/p2p/<id>` component. For a relay
/// circuit address `…/p2p/<relay>/p2p-circuit/p2p/<target>` this is the target (not
/// the relay); for a direct `…/p2p/<peer>` it is that peer.
pub fn target_peer_in_multiaddr(addr: &Multiaddr) -> Option<libp2p::PeerId> {
    let mut target = None;
    for proto in addr.iter() {
        if let libp2p::multiaddr::Protocol::P2p(id) = proto {
            target = Some(id);
        }
    }
    target
}

/// Whether a multiaddr is a relay-circuit address (`…/p2p-circuit/…`). A connection
/// established over such an address is relayed; DCUtR then tries to upgrade it to a
/// direct one.
fn is_relayed(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| p == Protocol::P2pCircuit)
}

fn to_ident(topic: &Topic) -> gossipsub::IdentTopic {
    gossipsub::IdentTopic::new(hex::encode(topic.as_bytes()))
}

fn from_topic_hash(hash: &gossipsub::TopicHash) -> Topic {
    Topic::new(hex::decode(hash.as_str()).unwrap_or_default())
}

// ----- actor -----------------------------------------------------------------

enum Command {
    Subscribe(Topic),
    Unsubscribe(Topic),
    Publish(Topic, Bytes),
    Request {
        peer: PeerId,
        data: Bytes,
        reply: oneshot::Sender<Result<Bytes, TransportError>>,
    },
    /// Start listening on `addr` (e.g. a `…/p2p-circuit` relay reservation).
    Listen(Multiaddr),
    /// Dial `addr` (e.g. a relay, before reserving a circuit on it).
    Dial(Multiaddr),
    /// Register our peer record under `namespace` at the rendezvous node `rz_node`
    /// (must already be connected to it). Deferred until we have an external address.
    RendezvousRegister {
        namespace: rendezvous::Namespace,
        rz_node: libp2p::PeerId,
    },
    /// Discover peers under `namespace` from the rendezvous node `rz_node`.
    RendezvousDiscover {
        namespace: rendezvous::Namespace,
        rz_node: libp2p::PeerId,
    },
}

type InboundResponses = FuturesUnordered<
    Pin<Box<dyn Future<Output = (ResponseChannel<Vec<u8>>, Option<Bytes>)> + Send>>,
>;

struct Actor {
    swarm: Swarm<MeshBehaviour>,
    cmd_rx: mpsc::Receiver<Command>,
    event_tx: mpsc::Sender<TransportEvent>,
    listen_tx: mpsc::Sender<Multiaddr>,
    /// Peers whose relayed connection DCUtR upgraded to a direct one (diagnostics).
    upgrade_tx: mpsc::Sender<PeerId>,
    /// Rendezvous-discovered peer records, surfaced but never auto-dialed. Unbounded
    /// so candidates are never dropped (they feed higher-layer security counters).
    discovered_tx: mpsc::UnboundedSender<Discovered>,
    /// Our own successful registrations (with the granted TTL).
    registered_tx: mpsc::UnboundedSender<Registered>,
    /// Registrations deferred until we have a confirmed external address to advertise.
    pending_registers: Vec<(rendezvous::Namespace, libp2p::PeerId)>,
    peers: HashMap<PeerId, libp2p::PeerId>,
    pending_req: HashMap<OutboundRequestId, oneshot::Sender<Result<Bytes, TransportError>>>,
    pending_publish: Vec<(Topic, Bytes)>,
}

impl Actor {
    async fn run(mut self) {
        let mut inbound: InboundResponses = FuturesUnordered::new();
        loop {
            tokio::select! {
                maybe_cmd = self.cmd_rx.recv() => {
                    match maybe_cmd {
                        Some(cmd) => self.handle_command(cmd),
                        None => break, // all handles dropped
                    }
                }
                event = self.swarm.select_next_some() => {
                    self.on_swarm_event(event, &mut inbound).await;
                }
                Some((channel, resp)) = inbound.next(), if !inbound.is_empty() => {
                    if let Some(bytes) = resp {
                        let _ = self
                            .swarm
                            .behaviour_mut()
                            .request_response
                            .send_response(channel, bytes.to_vec());
                    }
                }
            }
        }
    }

    fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::Subscribe(topic) => {
                tracing::debug!(topic = %hex::encode(topic.as_bytes()), "subscribe");
                let _ = self
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .subscribe(&to_ident(&topic));
            }
            Command::Unsubscribe(topic) => {
                tracing::debug!(topic = %hex::encode(topic.as_bytes()), "unsubscribe");
                let _ = self
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .unsubscribe(&to_ident(&topic));
            }
            Command::Publish(topic, data) => {
                let len = data.len();
                if self
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(to_ident(&topic), data.to_vec())
                    .is_err()
                {
                    // No subscribers known yet — retry when one appears.
                    tracing::trace!(bytes = len, "publish queued (no subscribers yet)");
                    self.pending_publish.push((topic, data));
                } else {
                    tracing::trace!(bytes = len, "published");
                }
            }
            Command::Request { peer, data, reply } => match self.peers.get(&peer) {
                Some(libp2p_peer) => {
                    tracing::debug!(peer = %libp2p_peer, bytes = data.len(), "send request");
                    let id = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_request(libp2p_peer, data.to_vec());
                    self.pending_req.insert(id, reply);
                }
                None => {
                    tracing::warn!(?peer, "request to unknown peer");
                    let _ = reply.send(Err(TransportError::Unreachable(peer)));
                }
            },
            Command::Listen(addr) => {
                if let Err(e) = self.swarm.listen_on(addr.clone()) {
                    tracing::warn!(%addr, error = %e, "listen failed");
                }
            }
            Command::Dial(addr) => {
                if let Err(e) = self.swarm.dial(addr.clone()) {
                    tracing::warn!(%addr, error = %e, "dial failed");
                }
            }
            Command::RendezvousRegister { namespace, rz_node } => {
                // Defer until we have an external address to advertise; flushed on
                // the next confirmed external address (e.g. a circuit reservation).
                self.pending_registers.push((namespace, rz_node));
                self.flush_pending_registers();
            }
            Command::RendezvousDiscover { namespace, rz_node } => {
                tracing::debug!(%rz_node, namespace = %namespace, "rendezvous discover");
                self.swarm.behaviour_mut().rendezvous_client.discover(
                    Some(namespace),
                    None,
                    None,
                    rz_node,
                );
            }
        }
    }

    /// Issue any deferred rendezvous registrations now that we may have an external
    /// address. A register that still has no external address stays queued.
    fn flush_pending_registers(&mut self) {
        let pending = std::mem::take(&mut self.pending_registers);
        for (namespace, rz_node) in pending {
            match self.swarm.behaviour_mut().rendezvous_client.register(
                namespace.clone(),
                rz_node,
                None,
            ) {
                Ok(()) => {
                    tracing::debug!(%rz_node, namespace = %namespace, "rendezvous register issued");
                }
                Err(rendezvous::client::RegisterError::NoExternalAddresses) => {
                    self.pending_registers.push((namespace, rz_node)); // retry later
                }
                Err(e) => {
                    tracing::warn!(error = ?e, "rendezvous register failed");
                }
            }
        }
    }

    fn flush_pending_publish(&mut self) {
        let pending = std::mem::take(&mut self.pending_publish);
        for (topic, data) in pending {
            if self
                .swarm
                .behaviour_mut()
                .gossipsub
                .publish(to_ident(&topic), data.to_vec())
                .is_err()
            {
                self.pending_publish.push((topic, data));
            }
        }
    }

    async fn on_swarm_event(
        &mut self,
        event: SwarmEvent<MeshBehaviourEvent>,
        inbound: &mut InboundResponses,
    ) {
        // Opportunistically retry deferred registrations: an external address may
        // have become available (and propagated to the behaviour) since the last try.
        if !self.pending_registers.is_empty() {
            self.flush_pending_registers();
        }
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                tracing::info!(%address, "listening");
                // A granted relay-circuit reservation is how a NAT'd node becomes
                // reachable; confirm it as an external address so rendezvous
                // registrations advertise it, then flush any deferred registrations.
                if is_relayed(&address) {
                    self.swarm.add_external_address(address.clone());
                    self.flush_pending_registers();
                }
                let _ = self.listen_tx.try_send(address);
            }
            SwarmEvent::ExternalAddrConfirmed { .. } => {
                self.flush_pending_registers();
            }
            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                let relayed = is_relayed(endpoint.get_remote_address());
                tracing::debug!(peer = %peer_id, relayed, "connection established");
                let peer = to_peer(&peer_id);
                // Only surface `PeerConnected` on the *first* connection to a peer;
                // a DCUtR upgrade opens a second (direct) connection to a peer we
                // already know, and must not look like a new peer to layers above.
                if self.peers.insert(peer, peer_id).is_none() {
                    let _ = self
                        .event_tx
                        .send(TransportEvent::PeerConnected(peer))
                        .await;
                }
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                num_established,
                ..
            } => {
                if num_established == 0 {
                    tracing::debug!(peer = %peer_id, "peer disconnected");
                    let peer = to_peer(&peer_id);
                    self.peers.remove(&peer);
                    let _ = self
                        .event_tx
                        .send(TransportEvent::PeerDisconnected(peer))
                        .await;
                }
            }
            SwarmEvent::Behaviour(MeshBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                message,
                propagation_source,
                ..
            })) => {
                let from = message
                    .source
                    .map(|s| to_peer(&s))
                    .unwrap_or_else(|| to_peer(&propagation_source));
                let topic = from_topic_hash(&message.topic);
                tracing::trace!(bytes = message.data.len(), "gossip received");
                let _ = self
                    .event_tx
                    .send(TransportEvent::Gossip {
                        topic,
                        from,
                        data: Bytes::from(message.data),
                    })
                    .await;
            }
            SwarmEvent::Behaviour(MeshBehaviourEvent::Gossipsub(
                gossipsub::Event::Subscribed { .. },
            )) => {
                self.flush_pending_publish();
            }
            SwarmEvent::Behaviour(MeshBehaviourEvent::RequestResponse(
                request_response::Event::Message { peer, message, .. },
            )) => match message {
                request_response::Message::Request {
                    request, channel, ..
                } => {
                    let from = to_peer(&peer);
                    self.peers.entry(from).or_insert(peer);
                    let (responder, rx) = Responder::channel();
                    let _ = self
                        .event_tx
                        .send(TransportEvent::Request {
                            from,
                            proto: ProtocolId(RR_PROTOCOL),
                            data: Bytes::from(request),
                            responder,
                        })
                        .await;
                    inbound.push(Box::pin(async move { (channel, rx.recv().await) }));
                }
                request_response::Message::Response {
                    request_id,
                    response,
                } => {
                    if let Some(reply) = self.pending_req.remove(&request_id) {
                        let _ = reply.send(Ok(Bytes::from(response)));
                    }
                }
            },
            SwarmEvent::Behaviour(MeshBehaviourEvent::RequestResponse(
                request_response::Event::OutboundFailure { request_id, .. },
            )) => {
                if let Some(reply) = self.pending_req.remove(&request_id) {
                    let _ = reply.send(Err(TransportError::Closed));
                }
            }
            // DCUtR hole-punch result. On success the relayed link has been upgraded
            // to a direct connection (a fresh `ConnectionEstablished` to the same
            // peer); surface it for diagnostics. On failure the connection stays
            // relayed — still fully functional, just routed through the relay.
            SwarmEvent::Behaviour(MeshBehaviourEvent::Dcutr(dcutr::Event {
                remote_peer_id,
                result,
            })) => match result {
                Ok(conn) => {
                    tracing::info!(peer = %remote_peer_id, ?conn, "DCUtR hole-punch succeeded — connection upgraded to direct");
                    let _ = self.upgrade_tx.try_send(to_peer(&remote_peer_id));
                }
                Err(e) => {
                    tracing::debug!(peer = %remote_peer_id, error = %e, "DCUtR hole-punch failed — staying relayed");
                }
            },
            // identify drives DCUtR's external-address candidates (and a relay learns
            // a client's addresses through it). Noisy; trace only.
            SwarmEvent::Behaviour(MeshBehaviourEvent::Identify(e)) => {
                tracing::trace!(?e, "identify event");
            }
            // Relay-client lifecycle (reservation accepted/expired, circuit opened).
            SwarmEvent::Behaviour(MeshBehaviourEvent::RelayClient(e)) => {
                tracing::debug!(?e, "relay-client event");
            }
            // Rendezvous client: surface discovered records (NEVER auto-dial them) and
            // our own registrations; log failures/expiry for the caller to react to.
            SwarmEvent::Behaviour(MeshBehaviourEvent::RendezvousClient(e)) => match e {
                rendezvous::client::Event::Discovered { registrations, .. } => {
                    for reg in registrations {
                        let _ = self.discovered_tx.send(Discovered {
                            peer: reg.record.peer_id(),
                            addresses: reg.record.addresses().to_vec(),
                            namespace: reg.namespace.to_string(),
                        });
                    }
                }
                rendezvous::client::Event::Registered {
                    rendezvous_node,
                    ttl,
                    namespace,
                } => {
                    tracing::info!(%rendezvous_node, namespace = %namespace, ttl, "rendezvous registered");
                    let _ = self.registered_tx.send(Registered {
                        namespace: namespace.to_string(),
                        ttl,
                        rendezvous_node,
                    });
                }
                rendezvous::client::Event::RegisterFailed {
                    rendezvous_node,
                    error,
                    ..
                } => {
                    tracing::warn!(%rendezvous_node, ?error, "rendezvous registration refused");
                }
                rendezvous::client::Event::DiscoverFailed {
                    rendezvous_node,
                    error,
                    ..
                } => {
                    tracing::warn!(%rendezvous_node, ?error, "rendezvous discovery failed");
                }
                rendezvous::client::Event::Expired { peer } => {
                    tracing::debug!(%peer, "rendezvous registration expired");
                }
            },
            _ => {}
        }
    }
}

// ----- handle ----------------------------------------------------------------

/// A handle to a running libp2p mesh node, implementing [`MeshTransport`].
#[derive(Debug)]
pub struct MeshService {
    local: PeerId,
    cmd_tx: mpsc::Sender<Command>,
    event_rx: Mutex<mpsc::Receiver<TransportEvent>>,
    listen_rx: Mutex<mpsc::Receiver<Multiaddr>>,
    upgrade_rx: Mutex<mpsc::Receiver<PeerId>>,
    discovered_rx: Mutex<mpsc::UnboundedReceiver<Discovered>>,
    registered_rx: Mutex<mpsc::UnboundedReceiver<Registered>>,
}

// `Command` holds a oneshot sender; keep it out of `Debug`.
impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Command")
    }
}

impl MeshService {
    /// Spawn the actor for an already-built (listening/dialing) swarm.
    pub fn spawn(swarm: Swarm<MeshBehaviour>) -> Self {
        let local = to_peer(swarm.local_peer_id());
        let (cmd_tx, cmd_rx) = mpsc::channel(256);
        let (event_tx, event_rx) = mpsc::channel(256);
        let (listen_tx, listen_rx) = mpsc::channel(16);
        let (upgrade_tx, upgrade_rx) = mpsc::channel(16);
        let (discovered_tx, discovered_rx) = mpsc::unbounded_channel();
        let (registered_tx, registered_rx) = mpsc::unbounded_channel();
        let actor = Actor {
            swarm,
            cmd_rx,
            event_tx,
            listen_tx,
            upgrade_tx,
            discovered_tx,
            registered_tx,
            pending_registers: Vec::new(),
            peers: HashMap::new(),
            pending_req: HashMap::new(),
            pending_publish: Vec::new(),
        };
        tokio::spawn(actor.run());
        Self {
            local,
            cmd_tx,
            event_rx: Mutex::new(event_rx),
            listen_rx: Mutex::new(listen_rx),
            upgrade_rx: Mutex::new(upgrade_rx),
            discovered_rx: Mutex::new(discovered_rx),
            registered_rx: Mutex::new(registered_rx),
        }
    }

    /// Await the next bound listen address (e.g. to learn the real port when
    /// listening on `/ip4/127.0.0.1/tcp/0`, or the circuit address once a relay
    /// reservation is granted). Returns `None` once the actor stops.
    pub async fn next_listen_addr(&self) -> Option<Multiaddr> {
        self.listen_rx.lock().await.recv().await
    }

    /// Await the next peer whose relayed connection DCUtR **upgraded to a direct
    /// one** (NAT hole-punch success). Diagnostics/observability only — the upgrade
    /// is transparent to the layers above (the peer stays the same `PeerId`, traffic
    /// just moves off the relay). Returns `None` once the actor stops.
    pub async fn next_direct_upgrade(&self) -> Option<PeerId> {
        self.upgrade_rx.lock().await.recv().await
    }

    /// Register our (signed) peer record under `namespace` at the rendezvous node
    /// `rz_node` — we must already be connected to it (e.g. dialed via its multiaddr).
    /// The registration is **deferred** internally until we have an external address
    /// to advertise (a direct listen address or a relay-circuit reservation); the
    /// granted TTL surfaces via [`MeshService::next_registered`].
    pub async fn rendezvous_register(
        &self,
        namespace: &str,
        rz_node: libp2p::PeerId,
    ) -> Result<(), NetError> {
        let namespace = rendezvous::Namespace::new(namespace.to_owned())
            .map_err(|_| NetError::Rendezvous("namespace too long".into()))?;
        self.cmd_tx
            .send(Command::RendezvousRegister { namespace, rz_node })
            .await
            .map_err(|_| NetError::Rendezvous("transport closed".into()))
    }

    /// Discover peers under `namespace` from the rendezvous node `rz_node`. Discovered
    /// records surface via [`MeshService::next_discovered`] and are **never
    /// auto-dialed** — a higher layer decides whether/when to dial (eclipse-resistance).
    pub async fn rendezvous_discover(
        &self,
        namespace: &str,
        rz_node: libp2p::PeerId,
    ) -> Result<(), NetError> {
        let namespace = rendezvous::Namespace::new(namespace.to_owned())
            .map_err(|_| NetError::Rendezvous("namespace too long".into()))?;
        self.cmd_tx
            .send(Command::RendezvousDiscover { namespace, rz_node })
            .await
            .map_err(|_| NetError::Rendezvous("transport closed".into()))
    }

    /// Await the next rendezvous-discovered peer record. Surfaced only — the transport
    /// never auto-dials it. Returns `None` once the actor stops.
    pub async fn next_discovered(&self) -> Option<Discovered> {
        self.discovered_rx.lock().await.recv().await
    }

    /// Await the next confirmation that our own record was registered at a rendezvous
    /// (with the granted TTL). Returns `None` once the actor stops.
    pub async fn next_registered(&self) -> Option<Registered> {
        self.registered_rx.lock().await.recv().await
    }

    /// Start listening on `addr` at runtime. Used to **reserve a relay slot** by
    /// listening on `<relay>/p2p/<relay-id>/p2p-circuit`; the granted circuit
    /// address then arrives via [`MeshService::next_listen_addr`].
    pub async fn listen_on(&self, addr: Multiaddr) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::Listen(addr))
            .await
            .map_err(|_| TransportError::Closed)
    }

    /// Dial `addr` at runtime (e.g. a relay, before reserving a circuit on it).
    pub async fn dial(&self, addr: Multiaddr) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::Dial(addr))
            .await
            .map_err(|_| TransportError::Closed)
    }

    /// Build a memory-transport node that listens on `listen` and dials `dial`,
    /// then spawn it. For deterministic local testing.
    pub fn new_memory(listen: Option<Multiaddr>, dial: &[Multiaddr]) -> Result<Self, NetError> {
        let mut swarm = build_memory_swarm();
        if let Some(addr) = listen {
            swarm
                .listen_on(addr)
                .map_err(|e| NetError::Listen(e.to_string()))?;
        }
        for addr in dial {
            swarm
                .dial(addr.clone())
                .map_err(|e| NetError::Dial(e.to_string()))?;
        }
        Ok(Self::spawn(swarm))
    }

    /// Build a **TCP** node that listens on `listen` and dials `dial`, then spawn
    /// it. Also returns this node's libp2p `PeerId` so the caller can advertise a
    /// dialable `/…/p2p/<id>` bootstrap address (e.g. inside an invite). Real
    /// cross-process / cross-machine networking.
    pub fn new_tcp(
        listen: Option<Multiaddr>,
        dial: &[Multiaddr],
    ) -> Result<(Self, libp2p::PeerId), NetError> {
        let mut swarm = build_tcp_swarm()?;
        let libp2p_id = *swarm.local_peer_id();
        if let Some(addr) = listen {
            swarm
                .listen_on(addr)
                .map_err(|e| NetError::Listen(e.to_string()))?;
        }
        for addr in dial {
            swarm
                .dial(addr.clone())
                .map_err(|e| NetError::Dial(e.to_string()))?;
        }
        Ok((Self::spawn(swarm), libp2p_id))
    }
}

#[async_trait]
impl MeshTransport for MeshService {
    fn local_peer(&self) -> PeerId {
        self.local
    }

    async fn subscribe(&self, topic: Topic) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::Subscribe(topic))
            .await
            .map_err(|_| TransportError::Closed)
    }

    async fn unsubscribe(&self, topic: Topic) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::Unsubscribe(topic))
            .await
            .map_err(|_| TransportError::Closed)
    }

    async fn publish(&self, topic: Topic, data: Bytes) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::Publish(topic, data))
            .await
            .map_err(|_| TransportError::Closed)
    }

    async fn request(
        &self,
        peer: PeerId,
        _proto: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Request { peer, data, reply })
            .await
            .map_err(|_| TransportError::Closed)?;
        rx.await.map_err(|_| TransportError::Closed)?
    }

    async fn next_event(&self) -> Option<TransportEvent> {
        self.event_rx.lock().await.recv().await
    }
}
