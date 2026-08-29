//! libp2p mesh networking (Phase 6a).
//!
//! [`MeshService`] realizes the Phase-0 [`catcoms_rt::MeshTransport`] seam over
//! **real libp2p** (Noise + yamux, gossipsub for topic fan-out, request/response
//! for addressed exchanges). The whole stack above; encrypted CRDT replication,
//! blob fetch; can therefore run unchanged over either the in-memory test
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
//! ([`MeshService::next_discovered`]) but **never auto-dialed**; a higher layer
//! decides whether to dial (where eclipse-resistance lives).
//!
//! Still to come in later mesh sub-blocks: the member-verifiable discovery tag +
//! eclipse-resistant discovery policy, and the anti-entropy / proposal-commit
//! protocols layered on top of this transport.
//!
//! The **infra** nodes (the zero-knowledge relay and rendezvous) live in their own modules;
//! see [`relay_node`] and [`rendezvous_node`] for their sizing, byte accounting and
//! load-shedding, [`admission`] for the source-prefix quotas both share, and
//! [`infra_transport`] for the metered TCP and TCP/443 WebSocket transports. [`addr`] holds
//! the address classifiers every one of them (and the desktop bridge) shares.

pub mod addr;
pub mod admission;
pub mod autonat_server;
pub mod fdlimit;
pub mod infra_transport;
pub mod join_reply;
pub mod metering;
mod pcp_ipv6;
pub mod relay_node;
pub mod rendezvous_node;

pub use addr::{
    addr_has_dns, addr_is_globally_routable, addr_is_loopback, addr_is_private, addr_is_undialable,
    ipv4_is_globally_routable, ipv4_is_local, ipv4_is_undialable, ipv6_is_globally_routable,
    ipv6_is_loopback,
};
pub use infra_transport::{
    is_advertisable, is_websocket_addr, is_wildcard_addr, load_ws_tls_pem, WsTlsConfig,
};
pub use join_reply::{JoinReply, JoinReplyError, JOIN_REPLY_LIFETIME_MS};
pub use metering::ByteMeters;
pub use relay_node::{
    build_memory_relay_swarm, build_relay_swarm, build_relay_swarm_with_key, run_relay,
    run_relay_with_external, RelayBehaviour, RelayBehaviourEvent, RelayLimits, RelayNode,
};
pub use rendezvous_node::{
    build_memory_rendezvous_swarm, build_rendezvous_swarm, build_rendezvous_swarm_with_key,
    run_rendezvous, QueryVerdict, RendezvousBehaviour, RendezvousBehaviourEvent, RendezvousLimits,
    RendezvousNode,
};

use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::num::NonZeroU16;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use catcoms_rt::{
    Clock, ConnectionDirection, ConnectionFamily, ConnectionPath, ConnectionTransport,
    CryptoRngCore, DiscoveredPeer, MeshTransport, OsCryptoRng, PeerConnectionSnapshot, PeerId,
    ProtocolId, RendezvousRegistration, Responder, SystemClock, Topic, TransportError,
    TransportEvent, MAX_PEER_DIAL_BATCH,
};
use futures::stream::FuturesUnordered;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, StreamExt};
use libp2p::core::transport::MemoryTransport;
use libp2p::core::transport::PortUse;
use libp2p::core::upgrade::Version;
use libp2p::core::{ConnectedPoint, Endpoint};
use libp2p::multiaddr::Protocol;
use libp2p::request_response::{OutboundRequestId, ResponseChannel};
use libp2p::swarm::behaviour::toggle::Toggle;
use libp2p::swarm::dial_opts::{DialOpts, PeerCondition};
use libp2p::swarm::{
    dummy, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, NewExternalAddrCandidate,
    SwarmEvent, THandler, THandlerInEvent, THandlerOutEvent, ToSwarm,
};
use libp2p::{
    allow_block_list, autonat, connection_limits, dcutr, gossipsub, identify, noise, relay,
    rendezvous, request_response, yamux, Multiaddr, StreamProtocol, Swarm, SwarmBuilder, Transport,
};
use thiserror::Error;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::JoinHandle;

/// Max request/response frame size.
const MAX_FRAME: usize = 16 * 1024 * 1024;
/// Cap on discovered records surfaced from a single rendezvous Discover response, so a
/// hostile rendezvous cannot flood the never-dropping discovered queue (the higher
/// layer ranks/dials with its own bounds, but those sit downstream of this queue).
const MAX_DISCOVERED_PER_RESPONSE: usize = 128;

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

/// A peer record surfaced by a rendezvous **discovery**; the (signed) peer id and
/// the addresses it advertised, under a blinded namespace. The transport only
/// *surfaces* these; it never auto-dials them (a higher layer decides whether and
/// when to dial, which is where eclipse-resistance lives).
#[derive(Debug, Clone)]
pub struct Discovered {
    /// The discovered peer (the signer of the record; authenticity is verified by
    /// libp2p when decoding the signed peer record).
    pub peer: libp2p::PeerId,
    /// The addresses the peer advertised.
    pub addresses: Vec<Multiaddr>,
    /// The namespace it was discovered under.
    pub namespace: String,
    /// The record's own signed sequence number (a monotonic counter the registrant signs into its
    /// libp2p PeerRecord). Carried up so the discovery policy can use real signed-freshness for its
    /// anti-replay high-water (rather than a placeholder).
    pub seq: u64,
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

/// One currently-live outbound route whose terminal peer completed libp2p's Noise handshake.
///
/// This is intentionally separate from ordinary connection diagnostics: it contains a network
/// address and must never be shown as membership or presence evidence. The desktop uses it only
/// to retain the exact same-LAN route that a joiner already proved during admission, so a restart
/// does not depend on an unimplemented local-discovery mechanism. Listener source addresses are
/// excluded because their ephemeral source port is not a future dial target; relay circuits are
/// excluded because their lease/consent lifecycle is managed separately.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AuthenticatedDialRoute {
    /// The Phase-0 transport identity authenticated by Noise on this connection.
    pub peer: PeerId,
    /// A canonical, direct IP multiaddr ending in that peer's `/p2p/<id>` component.
    pub address: String,
}

/// Retain two most-recent exact routes per peer for this process lifetime. This is evidence of a
/// completed outbound Noise handshake, not current connectivity; the application still supplies
/// roster and local-consent checks before it may persist or dial one.
const MAX_AUTHENTICATED_ROUTE_EVIDENCE_PER_PEER: usize = 2;
const MAX_AUTHENTICATED_ROUTE_EVIDENCE: usize =
    catcoms_rt::MAX_CONNECTED_PEER_SNAPSHOT * MAX_AUTHENTICATED_ROUTE_EVIDENCE_PER_PEER;

/// The result of one AutoNAT v2 dial-back for one candidate address.
///
/// Reachability is deliberately **per address and per observer**, not a permanent property of the
/// node. A successful v2 result means the named server really opened a fresh connection and the
/// client received the matching nonce; a failure means only that this server could not reach this
/// address at this moment. Callers must not generalise either result to every transport or network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoNatResult {
    /// The local external-address candidate the server tested.
    pub address: Multiaddr,
    /// The public relay/rendezvous node that performed the dial-back.
    pub server: libp2p::PeerId,
    /// `true` only after the callback connection carrying the expected nonce arrived.
    pub reachable: bool,
    /// The upstream error for a failed test. Empty on success.
    pub error: Option<String>,
}

/// The router protocol that produced (or failed to produce) an inbound port mapping.
///
/// UPnP IGD has two cooperating sources: libp2p's behaviour (unbound discovery socket, the OS
/// picks the multicast egress interface) and this crate's bound worker, which repeats the search
/// from the default-route interface because the OS routinely picks a virtual adapter (WSL
/// vEthernet, VirtualBox host-only) and the router never hears the unbound search. Their events
/// share this mechanism but are keyed apart by `local_address` (`None` = libp2p, `Some(v4)` =
/// bound), and a shared lease renewed by both is idempotent at the gateway. The separate
/// `portmapper` clients keep UPnP disabled; they probe PCP first and then NAT-PMP, and report
/// which protocol the gateway actually advertised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PortMappingMechanism {
    /// Universal Plug and Play Internet Gateway Device protocol.
    Upnp,
    /// Port Control Protocol (RFC 6887).
    Pcp,
    /// NAT Port Mapping Protocol.
    NatPmp,
}

impl std::fmt::Display for PortMappingMechanism {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Upnp => "UPnP",
            Self::Pcp => "PCP",
            Self::NatPmp => "NAT-PMP",
        })
    }
}

/// IP transport whose stable listen port is being mapped through the router.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PortMappingTransport {
    /// The libp2p TCP listener.
    Tcp,
    /// The libp2p QUIC listener (a UDP mapping).
    Udp,
}

impl std::fmt::Display for PortMappingTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP/QUIC",
        })
    }
}

/// A best-effort router port-mapping lifecycle event.
///
/// A mapping is evidence of a candidate public address, not proof that the wider internet can
/// reach it. The actor separately offers successful candidates to AutoNAT for a remote callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortMappingEvent {
    /// A gateway created an inbound mapping to this node's stable listen port.
    Mapped {
        /// The protocol that created the mapping.
        mechanism: PortMappingMechanism,
        /// TCP or UDP/QUIC.
        transport: PortMappingTransport,
        /// Exact local address whose listener owns this lease. `None` identifies the legacy
        /// IPv4/default-gateway owner used by UPnP, PCPv4 and NAT-PMP.
        local_address: Option<IpAddr>,
        /// The public multiaddr, without this node's `/p2p/<peer-id>` suffix.
        address: Multiaddr,
    },
    /// A protocol probe found no usable gateway for this transport.
    Unavailable {
        /// Protocol that was unavailable.
        mechanism: PortMappingMechanism,
        /// TCP or UDP/QUIC.
        transport: PortMappingTransport,
        /// Exact local address whose attempt failed, or `None` for the IPv4/default-gateway path.
        local_address: Option<IpAddr>,
        /// Scoped diagnostic; callers must not interpret one protocol's failure as every mapping
        /// mechanism having failed.
        detail: String,
    },
    /// A previously surfaced mapping expired. Callers must stop advertising it unless a renewed
    /// mapping has already replaced it.
    Expired {
        /// Protocol that owned the mapping.
        mechanism: PortMappingMechanism,
        /// TCP or UDP/QUIC.
        transport: PortMappingTransport,
        /// Exact local address whose lease ended, or `None` for the IPv4/default-gateway path.
        local_address: Option<IpAddr>,
        /// The public address that is no longer known to be mapped.
        address: Multiaddr,
    },
}

/// One currently active public router mapping in [`PortMappingSnapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivePortMapping {
    /// Protocol that owns the lease.
    pub mechanism: PortMappingMechanism,
    /// TCP or UDP/QUIC.
    pub transport: PortMappingTransport,
    /// Exact local address that owns the lease. IPv6 pinholes need this to coexist with IPv4 and
    /// with another interface's IPv6 lease.
    pub local_address: Option<IpAddr>,
    /// Public multiaddr, without this node's peer-id suffix.
    pub address: Multiaddr,
}

/// The latest scoped failure for one router protocol and transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortMappingFailure {
    /// Protocol that failed or whose previous lease expired.
    pub mechanism: PortMappingMechanism,
    /// TCP or UDP/QUIC.
    pub transport: PortMappingTransport,
    /// Exact local address whose attempt failed, or `None` for the IPv4/default-gateway path.
    pub local_address: Option<IpAddr>,
    /// Bounded upstream/context detail.
    pub detail: String,
}

/// Coalesced authoritative router-mapping state.
///
/// A snapshot rather than an unbounded event log is important for library users that do not show
/// diagnostics: retaining a `MeshService` without draining diagnostics must consume constant
/// memory. A slow consumer can skip intermediate telemetry but cannot miss the current address set
/// or resurrect an expired route.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortMappingSnapshot {
    /// All live leases, one per mechanism/transport/local-address owner.
    pub active: Vec<ActivePortMapping>,
    /// Latest failures for keys that do not currently have a live lease.
    pub unavailable: Vec<PortMappingFailure>,
}

/// Coalesced AutoNAT evidence, retaining the latest result for every address/server pair.
///
/// `latest` preserves the event-style convenience API used by integration tests, while `results`
/// lets the product rank all still-relevant routes without forgetting a second successful one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoNatSnapshot {
    /// Latest result for each distinct candidate/server pair.
    pub results: Vec<AutoNatResult>,
    /// Result that caused this snapshot update.
    pub latest: Option<AutoNatResult>,
}

/// Current relay-circuit listen addresses owned by this mesh node.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayAddressSnapshot {
    /// Every live `/p2p-circuit` address. Empty after the reservation/listener expires.
    pub addresses: Vec<Multiaddr>,
}

/// Addresses the Swarm has actually reported through `NewListenAddr`, withdrawn again through
/// `ExpiredListenAddr`. This is stronger than a successful `listen_on` request, whose OS bind can
/// still fail asynchronously.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListenerSnapshot {
    /// Current direct and relay listen addresses.
    pub addresses: Vec<Multiaddr>,
}

/// One low-trust observation of this node's outbound socket, reported by a connected Identify
/// peer.  It is diagnostics only: TCP source ports are commonly ephemeral and a peer may lie, so
/// this address must never be advertised or dialled as a listener route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshObservation {
    /// Connected peer that reported the observation.
    pub observer: libp2p::PeerId,
    /// Address the peer says it observed for this connection.
    pub address: Multiaddr,
}

/// Coalesced, bounded observations keyed by connected peer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeshObservationSnapshot {
    /// At most one latest observation per peer; removed when that peer disconnects.
    pub observations: Vec<MeshObservation>,
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

/// The `agent_version` this node reports over `identify`.
///
/// libp2p's default is `rust-libp2p/<version>`, which hands anyone who completes a handshake a
/// precise implementation-and-version fingerprint. Since a node now listens on a *stable* port
/// under a *stable* peer id, a single internet-wide sweep of that port would otherwise build a
/// map of `IP -> peer id -> library version`; a neutral, versionless string gives a scanner
/// nothing to select vulnerable builds by.
const AGENT_VERSION: &str = "catcoms";

/// Build the `identify` config shared by the mesh, relay and rendezvous nodes.
///
/// `hide_listen_addrs` is the load-bearing part. Noise authenticates a connection but does not
/// *authorize* it: identify runs immediately after the handshake, before anything checks
/// membership, so whatever it carries is readable by any host on the internet that can reach the
/// port. By default that includes the node's full listen-address set, i.e. its RFC1918 LAN
/// addresses, which is an unforced disclosure of internal topology. Hiding them keeps the
/// **confirmed external** addresses flowing (see `Behaviour::all_addresses`: only the raw listen
/// set is withheld), which is what relay reservations, rendezvous registration and DCUtR actually
/// consume, so NAT traversal is unaffected.
pub(crate) fn identify_config(key: &libp2p::identity::Keypair) -> identify::Config {
    identify::Config::new(IDENTIFY_PROTO.to_string(), key.public())
        .with_agent_version(AGENT_VERSION.to_string())
        .with_hide_listen_addrs(true)
}

/// The mesh node's libp2p behaviours. `relay_client` + `dcutr` + `identify` give
/// NAT traversal: a node can reserve a slot on a relay, be dialed via a circuit
/// address, and then hole-punch to a direct connection.
#[derive(NetworkBehaviour)]
#[allow(missing_debug_implementations)]
pub struct MeshBehaviour {
    // The two refusal behaviours are declared FIRST, and the order is load-bearing rather than
    // cosmetic. The derive runs each field's connection handlers in declaration order and stops
    // at the first refusal, so anything declared ahead of a protocol behaviour refuses before
    // that behaviour has allocated per-connection state for a caller it is about to lose.
    /// Connection caps so a discovery/registration flood cannot exhaust us.
    pub connection_limits: connection_limits::Behaviour,
    /// The **eviction gate** (P6): refuses every connection to a peer a Remove commit detached,
    /// including the first. See [`Eviction`] for why it is here and not behind `gossipsub`.
    pub eviction: Eviction,
    /// Closes connections that were **already live** when an eviction landed. [`Eviction`]
    /// refuses new ones; this is the half that severs what is already attached.
    pub blocked_peers: allow_block_list::Behaviour<allow_block_list::BlockedPeers>,
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
    /// AutoNAT v2 client: asks a connected public relay/rendezvous node to dial an external
    /// address candidate back from a fresh socket. The corresponding **server** behaviour lives
    /// only on those infrastructure swarms; ordinary members must not become public dial-back
    /// services merely by joining a group.
    pub autonat_client: autonat::v2::client::Behaviour<OsCryptoRng>,
    /// UPnP IGD: best-effort ask a compatible home router to open a port. PCP and NAT-PMP are
    /// separate actor-owned clients because libp2p's behaviour does not implement them.
    pub upnp: Toggle<libp2p::upnp::tokio::Behaviour>,
}

impl MeshBehaviour {
    fn new(
        key: &libp2p::identity::Keypair,
        relay_client: relay::client::Behaviour,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::new_with_port_mapping(key, relay_client, false)
    }

    /// Router mapping is an explicit product capability, not a side effect of constructing any
    /// TCP swarm. Keeping it off in the general/library builders prevents loopback tests, CLI
    /// probes and callers that never opted in from touching the user's gateway.
    fn new_with_port_mapping(
        key: &libp2p::identity::Keypair,
        relay_client: relay::client::Behaviour,
        enable_port_mapping: bool,
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
        let identify = identify::Behaviour::new(identify_config(key));
        let rendezvous_client = rendezvous::client::Behaviour::new(key.clone());
        // Use the repository's sanctioned OS-randomness seam rather than the behaviour's ambient
        // default. AutoNAT v2 nonces are security-sensitive and must follow the same dependency
        // boundary as every other random value in the application.
        let autonat_client = autonat::v2::client::Behaviour::new(
            OsCryptoRng,
            autonat::v2::client::Config::default(),
        );
        let upnp = Toggle::from(enable_port_mapping.then(libp2p::upnp::tokio::Behaviour::default));
        let connection_limits = connection_limits::Behaviour::new(
            connection_limits::ConnectionLimits::default()
                .with_max_pending_incoming(Some(64))
                // A *total* inbound cap, not just a per-peer one: on a stable, well-known port
                // this node is trivially enumerable, so a stranger flood has to hit a ceiling
                // well below resource exhaustion. Far above any real roster.
                .with_max_established_incoming(Some(256))
                .with_max_established_per_peer(Some(8))
                // P5, the two caps that were still unset.
                //
                // Pending **outgoing**: the bound on how much dialling this node can be *told*
                // to do. A dial plan is assembled from address sets this node did not author
                // (up to `MAX_PEX_ADDRESSES` per member record, plus a rendezvous response, plus
                // an invite's bootstrap list), and each pending dial holds a socket, a DNS
                // resolution and a half-open TCP or QUIC handshake. 32 is comfortably above the
                // widest legitimate burst (one member's addresses raced in a single dial, times
                // a few members reconnecting at once after an outage) and far below the point
                // where a fanned-out dial set costs this machine its file descriptors or gets
                // it flagged as a scanner by a home router's connection tracker. What it costs:
                // during a genuine reconnect storm the 33rd dial is refused rather than queued,
                // so convergence takes another discovery tick.
                .with_max_pending_outgoing(Some(32))
                // Total **established**, inbound and outbound together. The inbound cap above
                // does not bound this: relay circuits, rendezvous nodes and every peer this node
                // dialled are outbound, so a node that has been fed a large address set can hold
                // an unbounded number of connections while sitting at zero inbound. 320 is the
                // inbound ceiling plus 64 of headroom for this node's own dials (a roster of any
                // realistic size, plus its infra nodes, plus the second connection DCUtR opens
                // per peer while an upgrade is in flight). What it costs: past 320 the swarm
                // refuses new connections *including outbound ones this node wanted*, so a node
                // deliberately configured with hundreds of peers would stop dialling rather than
                // degrade; that is the intended failure, because the alternative is failing on
                // file descriptors with no diagnosis.
                .with_max_established(Some(320)),
        );
        Ok(Self {
            connection_limits,
            eviction: Eviction::default(),
            blocked_peers: allow_block_list::Behaviour::default(),
            gossipsub,
            request_response,
            relay_client,
            dcutr,
            identify,
            rendezvous_client,
            autonat_client,
            upnp,
        })
    }
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

/// Reconstruct a libp2p ed25519 identity from a 32-byte seed the caller stores itself.
///
/// The seed is the smallest thing a caller can persist that pins a **stable `PeerId`**, and a
/// stable `PeerId` is what keeps an already-issued invite (which embeds `/p2p/<id>`) redeemable
/// after a restart. `seed` is taken by value and zeroized by libp2p on the way in, so the caller's
/// copy is the only one left to protect.
pub fn keypair_from_seed(mut seed: [u8; 32]) -> Result<libp2p::identity::Keypair, NetError> {
    libp2p::identity::Keypair::ed25519_from_bytes(&mut seed)
        .map_err(|e| NetError::Build(format!("bad identity seed: {e}")))
}

/// Build a swarm over TCP, QUIC **and WebSocket**, with DNS resolution, relay-client capable, for
/// real networking. A fresh (throwaway) identity: see [`build_tcp_swarm_with_key`] for the
/// persisted-identity variant every long-lived node wants.
pub fn build_tcp_swarm() -> Result<Swarm<MeshBehaviour>, NetError> {
    // Identical to the keyed builder with a throwaway keypair, which is exactly what
    // `SwarmBuilder::with_new_identity` does internally. Sharing one implementation is what keeps
    // the transport stack from drifting between the two entry points, which is how the missing
    // WebSocket half of rung 4 went unnoticed.
    build_tcp_swarm_with_key(libp2p::identity::Keypair::generate_ed25519())
}

/// Like [`build_tcp_swarm`] but with a **caller-supplied identity**, so a node can persist its
/// keypair and keep a **stable peer id across restarts**; the same reason
/// [`build_relay_swarm_with_key`] exists for the infra nodes. Without it every launch mints a new
/// `PeerId` and silently kills every invite that embedded the old one (and every cached address a
/// peer held for us).
///
/// The transport stack (TCP + WebSocket + QUIC, DNS-resolving) is assembled in
/// [`infra_transport::client_transport`]; see there for why WebSocket and DNS are part of it and
/// what adding DNS changes for layers that assumed `/dns4` addresses were undialable.
pub fn build_tcp_swarm_with_key(
    key: libp2p::identity::Keypair,
) -> Result<Swarm<MeshBehaviour>, NetError> {
    build_tcp_swarm_with_key_and_port_mapping(key, false)
}

/// Build the product TCP swarm with automatic UPnP/PCP/NAT-PMP explicitly enabled.
///
/// General library/CLI constructors deliberately use [`build_tcp_swarm_with_key`] instead, where
/// mapping is disabled: merely binding a test loopback socket must never change a real router.
pub fn build_tcp_swarm_with_key_and_port_mapping(
    key: libp2p::identity::Keypair,
    enable_port_mapping: bool,
) -> Result<Swarm<MeshBehaviour>, NetError> {
    Ok(SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_other_transport(infra_transport::client_transport)
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_behaviour(move |key, relay| {
            MeshBehaviour::new_with_port_mapping(key, relay, enable_port_mapping)
        })
        .map_err(|e| NetError::Build(e.to_string()))?
        .build())
}

// ----- mapping helpers -------------------------------------------------------

fn to_peer(p: &libp2p::PeerId) -> PeerId {
    PeerId::new(*blake3::hash(&p.to_bytes()).as_bytes())
}

/// The Phase-0 [`PeerId`] for a libp2p peer (a BLAKE3 of its bytes); how every
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

/// Reduce libp2p's endpoint to the stable, address-free evidence exposed above the transport.
///
/// Relay wins over its carrier protocol: `/ip4/.../tcp/.../p2p-circuit` is useful evidence that a
/// circuit works, not evidence that this member is directly reachable over TCP. WebSocket likewise
/// wins over its TCP carrier. The order here is therefore part of the diagnostic contract.
fn classify_connection_path(endpoint: &ConnectedPoint) -> ConnectionPath {
    // For an inbound relay circuit the relay route lives on `local_addr`; `send_back_addr` is the
    // remote endpoint *inside* the circuit and can even be a memory address. Direct listeners and
    // every dialer continue to use the remote address. Family and transport must describe the
    // same route or the UI can report a nonsensical "memory circuit over an IPv4 relay".
    let addr = match endpoint {
        ConnectedPoint::Listener { local_addr, .. } if endpoint.is_relayed() => local_addr,
        _ => endpoint.get_remote_address(),
    };
    let family = if addr.iter().any(|p| {
        matches!(p, Protocol::Ip4(_))
            || matches!(p, Protocol::Ip6(ip) if ip.to_ipv4_mapped().is_some())
    }) {
        ConnectionFamily::Ipv4
    } else if addr
        .iter()
        .any(|p| matches!(p, Protocol::Ip6(ip) if ip.to_ipv4_mapped().is_none()))
    {
        ConnectionFamily::Ipv6
    } else if addr.iter().any(|p| {
        matches!(
            p,
            Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_)
        )
    }) {
        ConnectionFamily::Dns
    } else if addr.iter().any(|p| matches!(p, Protocol::Memory(_))) {
        ConnectionFamily::Memory
    } else {
        ConnectionFamily::Unknown
    };
    let transport = if endpoint.is_relayed() {
        ConnectionTransport::CircuitRelay
    } else if addr
        .iter()
        .any(|p| matches!(p, Protocol::Ws(_) | Protocol::Wss(_)))
    {
        ConnectionTransport::WebSocket
    } else if addr.iter().any(|p| matches!(p, Protocol::QuicV1)) {
        ConnectionTransport::QuicV1
    } else if addr.iter().any(|p| matches!(p, Protocol::Tcp(_))) {
        ConnectionTransport::Tcp
    } else if addr.iter().any(|p| matches!(p, Protocol::Memory(_))) {
        ConnectionTransport::Memory
    } else {
        ConnectionTransport::Unknown
    };
    ConnectionPath {
        family,
        transport,
        direction: if endpoint.is_dialer() {
            ConnectionDirection::Dialer
        } else {
            ConnectionDirection::Listener
        },
    }
}

fn active_connection_paths(
    paths: &HashMap<(libp2p::PeerId, ConnectionId), ConnectionPath>,
    peer_id: libp2p::PeerId,
) -> Vec<ConnectionPath> {
    let mut active: Vec<_> = paths
        .iter()
        .filter_map(|((candidate, _), path)| (*candidate == peer_id).then_some(*path))
        .collect();
    active.sort_unstable();
    active.dedup();
    active
}

/// Recover the exact future-dialable route from an authenticated outbound connection.
///
/// libp2p removes the terminal `/p2p` component from some established dialer endpoints. Reattach
/// the identity that Noise actually authenticated, then run the same final direct-route guard used
/// by reciprocal repair. Keeping only literal IP routes avoids turning a remembered DNS answer
/// into a cross-session rebinding capability.
fn authenticated_dial_route(
    peer_id: libp2p::PeerId,
    endpoint: &ConnectedPoint,
) -> Option<AuthenticatedDialRoute> {
    let ConnectedPoint::Dialer { address, .. } = endpoint else {
        return None;
    };
    if is_relayed(address)
        || !address
            .iter()
            .any(|part| matches!(part, Protocol::Ip4(_) | Protocol::Ip6(_)))
        || address
            .iter()
            .any(|part| matches!(part, Protocol::Ws(_) | Protocol::Wss(_) | Protocol::Tls))
        || address.iter().any(|part| {
            matches!(
                part,
                Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_)
            )
        })
    {
        return None;
    }

    let mut route = address.clone();
    match target_peer_in_multiaddr(&route) {
        Some(target) if target != peer_id => return None,
        Some(_) => {}
        None => route.push(Protocol::P2p(peer_id)),
    }
    let peer = to_peer(&peer_id);
    valid_direct_peer_batch(peer, std::slice::from_ref(&route)).then(|| AuthenticatedDialRoute {
        peer,
        address: route.to_string(),
    })
}

/// Address-free dial diagnostics. Member routes can contain private LAN coordinates, so generic
/// transport logging records only enough structure to distinguish family/transport failures.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DialLogShape {
    ipv4: usize,
    ipv6: usize,
    dns: usize,
    tcp: usize,
    websocket: usize,
    quic: usize,
    relay: usize,
}

fn dial_log_shape<'a>(addresses: impl IntoIterator<Item = &'a Multiaddr>) -> DialLogShape {
    let mut shape = DialLogShape::default();
    for address in addresses {
        let parts: Vec<_> = address.iter().collect();
        if parts.iter().any(|part| matches!(part, Protocol::Ip4(_))) {
            shape.ipv4 += 1;
        } else if parts.iter().any(|part| matches!(part, Protocol::Ip6(_))) {
            shape.ipv6 += 1;
        } else if parts.iter().any(|part| {
            matches!(
                part,
                Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_)
            )
        }) {
            shape.dns += 1;
        }
        if parts.iter().any(|part| matches!(part, Protocol::QuicV1)) {
            shape.quic += 1;
        } else if parts
            .iter()
            .any(|part| matches!(part, Protocol::Ws(_) | Protocol::Wss(_) | Protocol::Tls))
        {
            shape.websocket += 1;
        } else if parts.iter().any(|part| matches!(part, Protocol::Tcp(_))) {
            shape.tcp += 1;
        }
        if parts
            .iter()
            .any(|part| matches!(part, Protocol::P2pCircuit))
        {
            shape.relay += 1;
        }
    }
    shape
}

fn dial_error_kind(error: &libp2p::swarm::DialError) -> &'static str {
    match error {
        libp2p::swarm::DialError::LocalPeerId { .. } => "local-peer",
        libp2p::swarm::DialError::NoAddresses => "no-addresses",
        libp2p::swarm::DialError::DialPeerConditionFalse(_) => "condition",
        libp2p::swarm::DialError::Aborted => "aborted",
        libp2p::swarm::DialError::WrongPeerId { .. } => "wrong-peer",
        libp2p::swarm::DialError::Denied { .. } => "denied",
        libp2p::swarm::DialError::Transport(_) => "transport",
    }
}

/// Apply a libp2p close edge to the path ledger. `remaining == 0` is authoritative and clears all
/// rows for the peer, healing any missed/unknown connection id instead of stranding stale detail.
fn forget_connection_path(
    paths: &mut HashMap<(libp2p::PeerId, ConnectionId), ConnectionPath>,
    peer_id: libp2p::PeerId,
    connection_id: ConnectionId,
    remaining: u32,
) {
    paths.remove(&(peer_id, connection_id));
    if remaining == 0 {
        paths.retain(|(candidate, _), _| candidate != &peer_id);
    }
}

/// Mirror the path ledger's authoritative final-close healing for address-bearing route state.
fn forget_authenticated_routes(
    routes: &mut HashMap<(libp2p::PeerId, ConnectionId), AuthenticatedDialRoute>,
    peer: libp2p::PeerId,
    connection: ConnectionId,
    remaining: u32,
) {
    routes.remove(&(peer, connection));
    if remaining == 0 {
        routes.retain(|(candidate, _), _| *candidate != peer);
    }
}

/// Build the exact deterministic value exposed through the private authenticated-route watch.
fn authenticated_route_snapshot(
    routes: &HashMap<(libp2p::PeerId, ConnectionId), AuthenticatedDialRoute>,
) -> Vec<AuthenticatedDialRoute> {
    // A peer may have parallel TCP/QUIC connections to the same socket. Collapse identical routes
    // and retain the hard bound even if alternate swarm limits are configured in the future.
    let mut snapshot: Vec<_> = routes.values().cloned().collect();
    snapshot.sort();
    snapshot.dedup();
    snapshot.truncate(catcoms_rt::MAX_CONNECTED_PEER_SNAPSHOT * MAX_PEER_DIAL_BATCH);
    snapshot
}

/// Record a completed outbound Noise handshake independently of current liveness. A connection
/// can establish and close before the desktop obtains its vault lock; clearing this proof on the
/// close edge would make a valid, explicitly authorized recovery impossible to seal.
fn record_authenticated_route_evidence(
    evidence: &mut VecDeque<AuthenticatedDialRoute>,
    route: AuthenticatedDialRoute,
) {
    evidence.retain(|candidate| candidate != &route);
    while evidence
        .iter()
        .filter(|candidate| candidate.peer == route.peer)
        .count()
        >= MAX_AUTHENTICATED_ROUTE_EVIDENCE_PER_PEER
    {
        let Some(index) = evidence
            .iter()
            .position(|candidate| candidate.peer == route.peer)
        else {
            break;
        };
        evidence.remove(index);
    }
    evidence.push_back(route);
    while evidence.len() > MAX_AUTHENTICATED_ROUTE_EVIDENCE {
        evidence.pop_front();
    }
}

/// Apply one physical close edge and immediately publish its address-bearing result.
///
/// This must not be conditional on the privacy-preserving coarse path set changing: two distinct
/// TCP connections can collapse to the same path shape while carrying different future-dialable
/// addresses. Keeping the state transition and publication together makes that invariant directly
/// regression-testable.
fn forget_and_publish_authenticated_routes(
    routes: &mut HashMap<(libp2p::PeerId, ConnectionId), AuthenticatedDialRoute>,
    snapshots: &watch::Sender<Vec<AuthenticatedDialRoute>>,
    peer: libp2p::PeerId,
    connection: ConnectionId,
    remaining: u32,
) {
    forget_authenticated_routes(routes, peer, connection, remaining);
    snapshots.send_replace(authenticated_route_snapshot(routes));
}

/// The **relay**'s peer id in a circuit address `…/p2p/<relay>/p2p-circuit[/p2p/<target>]`, i.e.
/// the last `/p2p/` component before `/p2p-circuit`.
///
/// Used to recognise a relay as an infra target, which is what earns it the strict one-connection
/// dial condition: infra nodes are where the thundering herd actually lands, because every member
/// of every group converges on the same few of them.
fn relay_peer_in_circuit_addr(addr: &Multiaddr) -> Option<libp2p::PeerId> {
    let mut last_p2p = None;
    for proto in addr.iter() {
        match proto {
            Protocol::P2p(id) => last_p2p = Some(id),
            Protocol::P2pCircuit => return last_p2p,
            _ => {}
        }
    }
    None
}

/// How many peers the per-address dial ledger tracks before it is cleared wholesale. The ledger
/// only holds addresses whose dial has not yet resolved, so it drains itself in the normal case;
/// this bounds the pathological one (many peers, no outcomes) without any time-based expiry.
const MAX_DIAL_LEDGER_PEERS: usize = 1_024;

/// A validated rendezvous infra target: a direct (non-circuit) multiaddr and its peer id.
#[derive(Debug, Clone)]
pub struct RendezvousTarget {
    /// The dialable rendezvous multiaddr.
    pub addr: Multiaddr,
    /// The rendezvous node's libp2p peer id.
    pub peer: libp2p::PeerId,
}

/// Validate **operator-configured** rendezvous addresses: the ones typed by the person running
/// this node (the desktop create-server "rendezvous" field, `catcomsctl serve --rendezvous`).
///
/// The structural checks (6e-3d-9): each must parse as a multiaddr, be **direct** (never a
/// `/p2p-circuit`; a circuit rendezvous would route discovery through a relay), and carry a
/// `/p2p/<id>`; and the peer ids must be **distinct**. The distinct-PeerId check catches
/// accidental-duplicate misconfig **only**; it is *not* anti-collusion, so two
/// secretly-cooperating rendezvous still count as ≤ 1 trust root in the eclipse layer
/// regardless. Returns the parsed targets (in order) on success, so the caller can
/// register/discover/dial against them.
///
/// **No range or name check here, deliberately.** The operator is the trust root for their own
/// node's configuration: pointing it at a rendezvous on their own LAN is a legitimate
/// deployment, and rung 4 of the connectivity ladder *requires* a name, because a TCP/443
/// TLS/WebSocket listener is dialled as `/dns4/<name>/tcp/443/tls/ws`. An address arriving in
/// an **invite** is the opposite situation and gets [`validate_invite_rendezvous_addrs`]; see
/// its doc comment for why one function serving both callers was defect P13.
pub fn validate_operator_rendezvous_addrs(
    addrs: &[String],
) -> Result<Vec<RendezvousTarget>, NetError> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(addrs.len());
    for s in addrs {
        let addr: Multiaddr = s
            .parse()
            .map_err(|e| NetError::Rendezvous(format!("bad rendezvous multiaddr {s:?}: {e}")))?;
        if is_relayed(&addr) {
            return Err(NetError::Rendezvous(format!(
                "rendezvous address must be direct, not a circuit: {s:?}"
            )));
        }
        // Require EXACTLY ONE `/p2p/` stanza: zero is undialable, and more than one is a
        // relay-chain / malformed address that `target_peer_in_multiaddr` would silently
        // resolve to the last id (a confusion vector).
        let p2p_count = addr
            .iter()
            .filter(|p| matches!(p, Protocol::P2p(_)))
            .count();
        if p2p_count != 1 {
            return Err(NetError::Rendezvous(format!(
                "rendezvous address must carry exactly one /p2p/ id: {s:?}"
            )));
        }
        let peer = target_peer_in_multiaddr(&addr).ok_or_else(|| {
            NetError::Rendezvous(format!("rendezvous address has no peer id: {s:?}"))
        })?;
        if !seen.insert(peer) {
            return Err(NetError::Rendezvous(format!(
                "duplicate rendezvous PeerId {peer} (addresses must name distinct nodes)"
            )));
        }
        out.push(RendezvousTarget { addr, peer });
    }
    Ok(out)
}

/// Validate **invite-supplied** rendezvous addresses: the ones written by whoever produced a
/// pasted invite, i.e. by the party we are guarding against.
///
/// Every structural check of [`validate_operator_rendezvous_addrs`] applies, and then each
/// address must be a **globally routable IP literal**: no `/dns4`, `/dns6`, `/dnsaddr`, and no
/// private, link-local, CGNAT (`100.64.0.0/10`), unique-local, multicast, reserved or
/// documentation literal. See [`addr::addr_is_globally_routable`].
///
/// The two variants exist because one function was serving both callers, which was defect P13.
/// The invite's *bootstrap* half was already validated while its *rendezvous* half was not, so
/// the two halves of one attacker-authored invite were held to opposite standards. Adding the
/// WebSocket and DNS transports to the client for rung 4 turned that into a live hole rather
/// than a latent one: a `/dns4/...` in an invite used to fail at transport selection and now
/// resolves and dials. A malicious inviter can point a name at `192.168.1.x`, rotate the A
/// record per query, and have every joiner sweep its own LAN, which is exactly the attack the
/// peer-record address filter was added to stop, reached through the one path with no filter.
///
/// **Loopback is allowed only when the entire set is loopback.** That is the genuine
/// same-machine case (two instances on one dev box; the real-socket end-to-end tests bind
/// loopback and carry it in an invite) and it cannot be used to make a remote joiner probe its
/// own network, because a joiner elsewhere finds nothing there. Loopback *mixed* with a routable
/// address is refused: it is not a fallback for anything, it can only ever probe ports on the
/// reader's own machine. This is the rule the bootstrap half already uses (`dialable_bootstrap`
/// in the desktop bridge), and it shares this crate's classifiers with it rather than restating
/// them.
pub fn validate_invite_rendezvous_addrs(
    addrs: &[String],
) -> Result<Vec<RendezvousTarget>, NetError> {
    let targets = validate_operator_rendezvous_addrs(addrs)?;
    // The same-machine case, judged over the whole set rather than per address.
    if !targets.is_empty() && targets.iter().all(|t| addr::addr_is_loopback(&t.addr)) {
        return Ok(targets);
    }
    for t in &targets {
        if addr::addr_has_dns(&t.addr) {
            return Err(NetError::Rendezvous(format!(
                "an invite's rendezvous address must be an IP literal, not a name resolved at \
                 dial time: {}",
                t.addr
            )));
        }
        if !addr::addr_is_globally_routable(&t.addr) {
            return Err(NetError::Rendezvous(format!(
                "an invite's rendezvous address must be globally routable: {}",
                t.addr
            )));
        }
    }
    Ok(targets)
}

/// Compatibility alias for [`validate_operator_rendezvous_addrs`], the variant this name always
/// meant. Kept so existing callers (including two `catcoms-sync` end-to-end tests) keep
/// compiling; new code should name the variant it wants, because the difference is a trust
/// boundary rather than a style preference. Not marked `#[deprecated]` only because the
/// workspace builds with `-D warnings`.
pub fn validate_rendezvous_addrs(addrs: &[String]) -> Result<Vec<RendezvousTarget>, NetError> {
    validate_operator_rendezvous_addrs(addrs)
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
    /// Send only over a connection that is live when this command is handled. Unlike
    /// [`Command::Request`], this must never consult `recent_peers` or seed an implicit redial.
    /// It is the proof path used after a reciprocal-dial scheduler denies a new socket pass.
    RequestConnected {
        peer: PeerId,
        data: Bytes,
        reply: oneshot::Sender<Result<Bytes, TransportError>>,
    },
    /// Send a request whose reply nobody is waiting for (see [`catcoms_rt::MeshTransport::notify`]).
    /// No `pending_req` entry is registered, so the outcome is logged here and goes nowhere else.
    Notify {
        peer: PeerId,
        data: Bytes,
    },
    /// Fire-and-forget counterpart to `RequestConnected`: never consult `recent_peers`.
    NotifyConnected {
        peer: PeerId,
        data: Bytes,
    },
    /// Start listening on `addr` (e.g. a `…/p2p-circuit` relay reservation).
    Listen(Multiaddr),
    /// Dial `addr` (e.g. a relay, before reserving a circuit on it).
    Dial {
        addr: Multiaddr,
        /// Present for discovery-policy dials. Ownership must remain in this command until the
        /// actor either suppresses the endpoint or commits it at socket submission.
        permit: Option<catcoms_rt::BoxedDialPermit>,
        reply: Option<oneshot::Sender<catcoms_rt::DialSubmission>>,
    },
    /// A reciprocal-repair batch whose direct routes must all terminate at `peer`.
    /// Validation happens inside the actor, immediately before the ordinary dial gate, so an
    /// alternate caller cannot separate a checked peer id from a substituted address.
    DialPeerBatch {
        peer: PeerId,
        addrs: Vec<Multiaddr>,
        permits: Option<Vec<catcoms_rt::BoxedDialPermit>>,
        reply: oneshot::Sender<Result<Vec<catcoms_rt::DialSubmission>, TransportError>>,
    },
    /// Advertise `addr` as an external (reachable) address; for a node with a
    /// directly-reachable address (a public IP, or a memory listener in tests) that
    /// does not need a relay circuit to be registerable at a rendezvous. Flushes any
    /// deferred registrations.
    AddExternalAddress(Multiaddr),
    /// Withdraw one address previously asserted through [`Command::AddExternalAddress`]. Router
    /// mappings may independently own the same socket, so the actor removes it from Swarm only
    /// after the final configured/mapping owner is gone.
    RemoveExternalAddress(Multiaddr),
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
    /// **Evict** a peer (P6): close every connection to it and refuse the next one. Issued by
    /// the membership layer when a Remove commit is applied. See
    /// [`catcoms_rt::MeshTransport::evict_peer`] for why it is best-effort.
    Evict(PeerId),
    /// Lift an eviction, because the peer's device is a **member again**. The product ships a
    /// re-invite button and node identities are stable across restarts, so without this an
    /// ex-member who is later re-invited dials the founder, is refused at the connection handler,
    /// and the join times out with nothing to diagnose.
    Unevict(PeerId),
}

/// Commit an optional scheduler permit for this exact actor-owned endpoint.
///
/// A command without a permit is a locally configured/bootstrap dial and follows the legacy path.
/// A command with a stale permit is suppressed. The exact-address comparison is repeated here,
/// next to submission, even though the sender parsed the same string before enqueueing.
fn commit_dial_permit(permit: Option<catcoms_rt::BoxedDialPermit>, addr: &Multiaddr) -> bool {
    let Some(permit) = permit else {
        return true;
    };
    let canonical = addr.to_string();
    if permit.address() != canonical {
        return false;
    }
    permit
        .commit_if_current()
        .is_some_and(|authorized| authorized == canonical)
}

/// Cap on the eviction deny list.
///
/// 256 removals is far more membership churn than any friend circle or community server will
/// see in one process lifetime, so in practice the list never reaches the bound and nothing is
/// ever silently re-admitted. The cap exists because an unbounded list is a defect regardless of
/// how slowly it fills, not because this one is expected to fill.
const MAX_EVICTED_PEERS: usize = 256;

/// The eviction deny list: peers a Remove commit detached, keyed by the **phase-0** [`PeerId`]
/// (the BLAKE3-of-libp2p-peer-id form every layer above the transport addresses peers by, and the
/// only form the membership layer holds; see `PeerDescriptor::peer_id`).
///
/// Keying on the phase-0 id is what lets the deny be enforced at connection time. `to_peer` is a
/// **forward** hash: the wire always supplies the `libp2p::PeerId`, so the phase-0 id is always
/// computable from it, even though the reverse is not. An earlier version keyed on the
/// `libp2p::PeerId` instead, and could therefore only recognise an evicted peer *after* letting a
/// connection establish, which meant the first post-eviction connection was fully set up (with
/// every behaviour ahead of the deny allocating for it) before being torn down.
///
/// The `Option<libp2p::PeerId>` is not used for recognition; it records whether a **live**
/// connection was closed for this entry via `allow_block_list`, so the same block can be lifted
/// when the entry is dropped by the bound or by [`Command::Unevict`]. One store, so the deny set
/// and the block set cannot drift apart.
///
/// # Expiry
///
/// Entries do **not** expire on a timer. `admission.rs` uses time windows because the peers it
/// denies are anonymous strangers whose ids cost a keypair, so a permanent entry there is both
/// useless (evaded for free) and a memory-growth vector an attacker drives. Neither holds here:
/// the fill rate is one entry per membership removal in one group, which nothing hostile can
/// drive, and a timer would re-admit a peer at a moment nobody chose. What *does* lift an entry
/// is the membership layer saying so: a re-invited member is un-evicted as soon as the group
/// contains its device again (`ChannelSync::drain_evictions`). Readmission is the right signal,
/// time is not.
///
/// The list is process-local and is deliberately **not** persisted: a restart brings up a fresh
/// swarm with no connections and no reservations, and an ex-member reconnecting to it is an
/// unauthenticated stranger holding none of the group's keys. The attachment this exists to break
/// does not survive the restart either.
#[derive(Debug, Default)]
struct EvictedPeers {
    /// Insertion order, so the bound drops the **oldest** eviction first: the newest removals
    /// are the ones whose connections are most likely to still be live.
    order: VecDeque<PeerId>,
    /// Each evicted peer, and the `libp2p::PeerId` whose live connections were closed for it,
    /// if any (see the type docs).
    entries: HashMap<PeerId, Option<libp2p::PeerId>>,
}

impl EvictedPeers {
    /// Record an eviction. Returns the entry pushed out by the bound, if it had a live block, so
    /// the caller can lift it and keep `allow_block_list`'s set the same size as this one.
    fn deny(&mut self, peer: PeerId) -> Option<libp2p::PeerId> {
        if self.entries.contains_key(&peer) {
            return None; // already evicted; do not disturb the ordering
        }
        self.entries.insert(peer, None);
        self.order.push_back(peer);
        if self.order.len() > MAX_EVICTED_PEERS {
            if let Some(old) = self.order.pop_front() {
                return self.entries.remove(&old).flatten();
            }
        }
        None
    }

    /// Lift an eviction. Returns the live block to release, if one was installed.
    fn allow(&mut self, peer: &PeerId) -> Option<libp2p::PeerId> {
        self.order.retain(|p| p != peer);
        self.entries.remove(peer).flatten()
    }

    /// Whether `peer` is currently denied.
    fn is_denied(&self, peer: &PeerId) -> bool {
        self.entries.contains_key(peer)
    }

    /// Note that `allow_block_list` was asked to close live connections to this entry, so the
    /// block can be released again when the entry goes.
    fn note_blocked(&mut self, peer: PeerId, libp2p_peer: libp2p::PeerId) {
        if let Some(slot) = self.entries.get_mut(&peer) {
            *slot = Some(libp2p_peer);
        }
    }

    /// How many peers are currently denied (diagnostics/tests).
    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// A connection was refused because the peer has been evicted from the group.
#[derive(Debug)]
pub struct Evicted(PeerId);

impl std::fmt::Display for Evicted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "peer {:?} has been evicted from the group", self.0)
    }
}

impl std::error::Error for Evicted {}

/// The eviction gate: a [`NetworkBehaviour`] that refuses **every** connection to an evicted
/// peer, including the first one.
///
/// It exists rather than leaning on `allow_block_list` alone because of *where in the pipeline*
/// the refusal has to happen. The `NetworkBehaviour` derive calls each field's
/// `handle_established_inbound_connection` in **declaration order** and stops at the first
/// refusal, so a deny sitting behind `gossipsub` is a deny that runs after gossipsub has already
/// allocated a peer entry, a sender and a handler for the connection, and after
/// `ConnectionEstablished` has been dispatched to the behaviours; that is where gossipsub queues
/// a Subscribe for every mesh topic, and those topic hashes are the freshly-rotated member-only
/// ones, addressed to the member just removed. Whether they reach the wire before the close is a
/// scheduling race nobody here controls, so the refusal has to come first rather than be timed.
/// This behaviour is therefore declared **ahead of** every protocol behaviour in
/// [`MeshBehaviour`].
///
/// The same ordering fixes a leak. A connection refused *behind* gossipsub leaves gossipsub's
/// entry in place forever: it ignores `ListenFailure`, and no close is ever dispatched for a
/// connection that never established, so an evicted peer redialling in a loop grows that entry's
/// connection vector at a rate it chooses. Refused in front of gossipsub, none of it is
/// allocated in the first place.
///
/// `allow_block_list` is still carried alongside, for the one thing this cannot do: closing
/// connections that are **already live** when the eviction lands.
#[derive(Debug, Default)]
pub struct Eviction {
    denied: EvictedPeers,
}

impl Eviction {
    fn enforce(&self, peer: &libp2p::PeerId) -> Result<(), ConnectionDenied> {
        let phase0 = to_peer(peer);
        if self.denied.is_denied(&phase0) {
            tracing::debug!(peer = %peer, "refusing a connection to an evicted peer");
            return Err(ConnectionDenied::new(Evicted(phase0)));
        }
        Ok(())
    }
}

impl NetworkBehaviour for Eviction {
    type ConnectionHandler = dummy::ConnectionHandler;
    type ToSwarm = Infallible;

    fn handle_established_inbound_connection(
        &mut self,
        _: ConnectionId,
        peer: libp2p::PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // The earliest point at which an inbound connection's peer id is known at all (it comes
        // out of Noise) and, because this behaviour is declared first, the earliest point at
        // which anything in this swarm has spent state on it.
        self.enforce(&peer)?;
        Ok(dummy::ConnectionHandler)
    }

    fn handle_pending_outbound_connection(
        &mut self,
        _: ConnectionId,
        peer: Option<libp2p::PeerId>,
        _: &[Multiaddr],
        _: Endpoint,
    ) -> Result<Vec<Multiaddr>, ConnectionDenied> {
        if let Some(peer) = peer {
            self.enforce(&peer)?;
        }
        Ok(Vec::new())
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        peer: libp2p::PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // An address with no `/p2p/` names no peer, so the pending hook above cannot judge it;
        // this catches the dial that only learns who it reached once Noise completed.
        self.enforce(&peer)?;
        Ok(dummy::ConnectionHandler)
    }

    fn on_swarm_event(&mut self, _event: FromSwarm<'_>) {}

    fn on_connection_handler_event(
        &mut self,
        _peer: libp2p::PeerId,
        _: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        libp2p::core::util::unreachable(event)
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        Poll::Pending
    }
}

type InboundResponses = FuturesUnordered<
    Pin<Box<dyn Future<Output = (ResponseChannel<Vec<u8>>, Option<Bytes>)> + Send>>,
>;

/// A PCP/NAT-PMP worker is tied to exactly one stable TCP or UDP listen port. Dropping the actor
/// aborts these workers and stops lease renewal; the router's bounded lease then expires naturally.
/// A detached renewal task must never outlive the server that owned the port.
#[derive(Debug)]
struct PortMapperTask {
    port: NonZeroU16,
    generation: u64,
    handle: Option<JoinHandle<()>>,
    /// The bound-interface UPnP worker running alongside the IPv4 PCP/NAT-PMP worker (`None` for
    /// PCPv6 tasks). Same key, port and generation; its events differ only by `local_address`.
    companion: Option<JoinHandle<()>>,
    /// IPv6 PCP can explicitly delete its lease when a listener disappears. IPv4's retained
    /// `portmapper` implementation has no equivalent cancellation API, so those tasks are simply
    /// aborted and their library-managed bounded lease expires.
    stop: Option<oneshot::Sender<()>>,
}

/// One independently managed listener/router path. IPv4 protocols discover their default route
/// internally and retain their historical single owner; PCPv6 must bind the exact GUA so two
/// interfaces and IPv4 can coexist without overwriting each other's lease state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum PortMappingTarget {
    Ipv4,
    Ipv6(Ipv6Addr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct PortMapperTaskKey {
    transport: PortMappingTransport,
    target: PortMappingTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PortMappingEndpoint {
    key: PortMapperTaskKey,
    port: NonZeroU16,
}

/// Private worker envelope used to reject buffered events from a stopped/replaced listener. The
/// public snapshot deliberately does not expose this process-local generation.
#[derive(Debug)]
struct PortMapperReport {
    key: PortMapperTaskKey,
    port: NonZeroU16,
    generation: u64,
    event: PortMappingEvent,
}

#[derive(Debug, Clone)]
struct PortMapperReporter {
    tx: mpsc::Sender<PortMapperReport>,
    key: PortMapperTaskKey,
    port: NonZeroU16,
    generation: u64,
}

impl PortMapperReporter {
    async fn send(&self, event: PortMappingEvent) -> Result<(), ()> {
        self.tx
            .send(PortMapperReport {
                key: self.key,
                port: self.port,
                generation: self.generation,
                event,
            })
            .await
            .map_err(|_| ())
    }
}

type PortMappingKey = (PortMappingMechanism, PortMappingTransport, Option<IpAddr>);

/// AutoNAT servers and identify candidates are network input. Retaining one observation for every
/// peer/address ever seen would let churn grow a long-lived node without bound, even though the
/// public watch channel itself is coalesced.
const MAX_AUTONAT_OBSERVATIONS: usize = 64;
const MAX_MESH_OBSERVATIONS: usize = 32;

/// How many disconnected peers keep a redial hint (see `Actor::recent_peers`). Comfortably above
/// a friend circle's roster, and small enough that a churning public node cannot grow on it.
const MAX_RECENT_PEERS: usize = 256;

/// How many addresses one redial hint remembers. A peer is reachable over a handful of routes
/// (direct v4/v6, a relay circuit); keeping every address a long-lived peer has ever connected
/// from would let one peer crowd out the rest of the map.
const MAX_RECENT_PEER_ADDRS: usize = 4;

/// A peer's last known transport identity, kept so a request can redial it after its connection
/// dropped. See `Actor::recent_peers` for why the transport remembers this at all.
#[derive(Debug, Clone)]
struct RecentPeer {
    libp2p: libp2p::PeerId,
    /// Addresses this node has actually established a connection over, freshest last. Offered to
    /// the swarm's address book before a redial; never advertised as anything of ours.
    addresses: VecDeque<Multiaddr>,
}

/// Remember a peer's transport identity and the address it connected over, so a later request can
/// redial it once that connection has gone. See `Actor::recent_peers`.
///
/// Unlike [`record_mesh_observation`], a repeat visit does **not** make the peer newest: the map is
/// a redial hint for peers this node keeps talking to, and refreshing on every reconnect would let
/// a pair of chatty peers push out the quiet member whose route is the one worth remembering.
fn record_recent_peer(
    recent: &mut HashMap<PeerId, RecentPeer>,
    order: &mut VecDeque<PeerId>,
    peer: PeerId,
    libp2p_peer: libp2p::PeerId,
    addr: Multiaddr,
) {
    match recent.get_mut(&peer) {
        Some(entry) => {
            // A peer id is derived from its key, so this only ever re-confirms what is already
            // there; assigning keeps the halves consistent if that ever stops being true.
            entry.libp2p = libp2p_peer;
            if !entry.addresses.contains(&addr) {
                entry.addresses.push_back(addr);
                while entry.addresses.len() > MAX_RECENT_PEER_ADDRS {
                    entry.addresses.pop_front();
                }
            }
        }
        None => {
            recent.insert(
                peer,
                RecentPeer {
                    libp2p: libp2p_peer,
                    addresses: VecDeque::from([addr]),
                },
            );
            order.push_back(peer);
            while recent.len() > MAX_RECENT_PEERS {
                let Some(oldest) = order.pop_front() else {
                    break;
                };
                recent.remove(&oldest);
            }
        }
    }
}

fn record_mesh_observation(
    observations: &mut HashMap<libp2p::PeerId, Multiaddr>,
    order: &mut VecDeque<libp2p::PeerId>,
    observer: libp2p::PeerId,
    address: Multiaddr,
) {
    order.retain(|candidate| candidate != &observer);
    observations.insert(observer, address);
    order.push_back(observer);
    while observations.len() > MAX_MESH_OBSERVATIONS {
        let Some(oldest) = order.pop_front() else {
            break;
        };
        observations.remove(&oldest);
    }
}

type AutoNatKey = (Multiaddr, libp2p::PeerId);

fn record_autonat_result(
    results: &mut HashMap<AutoNatKey, AutoNatResult>,
    order: &mut VecDeque<AutoNatKey>,
    result: AutoNatResult,
) {
    let key = (result.address.clone(), result.server);
    // An updated pair becomes newest. Removing the old occurrence also keeps the order queue at
    // exactly one entry per map key, which makes the bound deterministic.
    order.retain(|candidate| candidate != &key);
    results.insert(key.clone(), result);
    order.push_back(key);
    while results.len() > MAX_AUTONAT_OBSERVATIONS {
        let Some(oldest) = order.pop_front() else {
            break;
        };
        results.remove(&oldest);
    }
}

fn forget_autonat_address(
    results: &mut HashMap<AutoNatKey, AutoNatResult>,
    order: &mut VecDeque<AutoNatKey>,
    address: &Multiaddr,
) -> bool {
    let old_len = results.len();
    results.retain(|(candidate, _), _| candidate != address);
    order.retain(|(candidate, _)| candidate != address);
    results.len() != old_len
}

fn autonat_candidate_is_current(
    configured: &HashSet<Multiaddr>,
    mappings: &HashMap<PortMappingKey, Multiaddr>,
    address: &Multiaddr,
) -> bool {
    configured.contains(address) || mappings.values().any(|candidate| candidate == address)
}

/// Automatic discovery must pass the global classifier, while an explicit caller assertion is an
/// independent owner and may intentionally be a LAN-only route. This distinction is load-bearing
/// when a broken gateway returns the exact private socket an operator configured manually.
/// Whether a "router mapping unavailable" answer is worth an info line, or is the same answer
/// this probe already gave.
///
/// A router with no PCP/NAT-PMP re-answers "no" on every discovery cycle, for every mechanism,
/// transport and interface. Logged unconditionally, that was 356 of one real debug log's 601
/// lines: the same sentence as eighteen seconds earlier, burying everything worth reading. The
/// first answer for a probe is news; an identical repeat is not; a *changed* detail is news
/// again, because that is a router whose behaviour actually moved.
fn mapping_unavailable_is_news(previous: Option<&str>, detail: &str) -> bool {
    previous != Some(detail)
}

fn external_address_is_allowed(configured: &HashSet<Multiaddr>, address: &Multiaddr) -> bool {
    configured.contains(address) || addr_is_globally_routable(address)
}

/// Retire one caller-configured external-address owner. `true` means the address has no remaining
/// configured or router-mapping owner and must be removed from Swarm. Keeping this decision pure
/// pins the important IPv6 case where a raw GUA and a PCP firewall pinhole name the same socket.
fn retire_configured_external_address(
    configured: &mut HashSet<Multiaddr>,
    mappings: &HashMap<PortMappingKey, Multiaddr>,
    address: &Multiaddr,
) -> bool {
    configured.remove(address) && !mappings.values().any(|candidate| candidate == address)
}

/// Record a mechanism's ownership of one external address. The returned old address is safe to
/// remove from libp2p only when no other mapping still owns it; `add_new` is false when the
/// address was already present through another mechanism. Manual/configured ownership is checked
/// separately by the actor because it is not a router lease.
fn activate_port_mapping(
    active: &mut HashMap<PortMappingKey, Multiaddr>,
    key: PortMappingKey,
    address: Multiaddr,
) -> (Option<Multiaddr>, bool) {
    let add_new = !active.values().any(|candidate| candidate == &address);
    let previous = active.insert(key, address.clone());
    let remove_old = previous
        .filter(|old| old != &address && !active.values().any(|candidate| candidate == old));
    (remove_old, add_new)
}

/// Drop one matching mapping owner and answer whether the external address has no mapping owner
/// left. A late expiry for an address already replaced under the same key is deliberately inert.
fn expire_port_mapping(
    active: &mut HashMap<PortMappingKey, Multiaddr>,
    key: PortMappingKey,
    address: &Multiaddr,
) -> bool {
    if active.get(&key) != Some(address) {
        return false;
    }
    active.remove(&key);
    !active.values().any(|candidate| candidate == address)
}

impl Drop for PortMapperTask {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        if let Some(companion) = self.companion.take() {
            companion.abort();
        }
    }
}

impl PortMapperTask {
    /// Whether every worker this task spawned is still alive. A dead half (either the PCP/NAT-PMP
    /// worker or its bound-UPnP companion) retires the whole task so both respawn together.
    fn is_fully_running(&self) -> bool {
        self.handle
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
            && self
                .companion
                .as_ref()
                .is_none_or(|companion| !companion.is_finished())
    }

    /// Withdraw product state before calling this; deletion at the router is best effort and must
    /// never delay removal from invites. Give PCPv6 a short grace period to send lifetime zero.
    fn stop_gracefully(mut self) {
        // The companion has no stop channel and no lease-deletion API; abort it and let its
        // bounded lease expire, the same policy as the IPv4 portmapper worker.
        if let Some(companion) = self.companion.take() {
            companion.abort();
        }
        let sent_stop = self.stop.take().is_some_and(|stop| stop.send(()).is_ok());
        let Some(mut handle) = self.handle.take() else {
            return;
        };
        if !sent_stop {
            handle.abort();
            return;
        }
        tokio::spawn(async move {
            tokio::select! {
                _ = &mut handle => {}
                _ = SystemClock.sleep(Duration::from_secs(1)) => handle.abort(),
            }
        });
    }
}

/// Extract the mapping-relevant endpoint from a concrete direct listen address. IPv4 is handled by
/// the established `portmapper` client; an exact global IPv6 listener gets an independent PCP MAP
/// firewall-pinhole worker. Wildcard, loopback and relay routes must never alter the user's router.
fn port_mapping_endpoint(addr: &Multiaddr) -> Option<PortMappingEndpoint> {
    if is_relayed(addr) {
        return None;
    }

    let mut protocols = addr.iter();
    let mut target = None;
    while let Some(protocol) = protocols.next() {
        match protocol {
            Protocol::Ip4(ip) => {
                // Mapping the route-selected LAN interface cannot make a socket that is bound
                // only to localhost accept the forwarded packets.
                target =
                    (!ip.is_loopback() && !ip.is_unspecified()).then_some(PortMappingTarget::Ipv4);
            }
            Protocol::Ip6(ip) => {
                target = crate::addr::ipv6_is_pcp_pinhole_candidate(&ip)
                    .then_some(PortMappingTarget::Ipv6(ip));
            }
            Protocol::Tcp(port) if target.is_some() => {
                return NonZeroU16::new(port).map(|port| PortMappingEndpoint {
                    key: PortMapperTaskKey {
                        transport: PortMappingTransport::Tcp,
                        target: target.expect("guarded above"),
                    },
                    port,
                });
            }
            Protocol::Udp(port)
                if target.is_some() && protocols.any(|p| matches!(p, Protocol::QuicV1)) =>
            {
                return NonZeroU16::new(port).map(|port| PortMappingEndpoint {
                    key: PortMapperTaskKey {
                        transport: PortMappingTransport::Udp,
                        target: target.expect("guarded above"),
                    },
                    port,
                });
            }
            _ => {}
        }
    }
    None
}

/// Extract only the transport/port from a router result. Unlike `port_mapping_endpoint`, this
/// deliberately accepts a non-public result so the actor can turn it into a truthful scoped
/// failure instead of silently discarding the gateway response.
fn port_mapping_transport(addr: &Multiaddr) -> Option<(PortMappingTransport, NonZeroU16)> {
    let mut protocols = addr.iter();
    while let Some(protocol) = protocols.next() {
        match protocol {
            Protocol::Tcp(port) => {
                return NonZeroU16::new(port).map(|port| (PortMappingTransport::Tcp, port));
            }
            Protocol::Udp(port) if protocols.any(|p| matches!(p, Protocol::QuicV1)) => {
                return NonZeroU16::new(port).map(|port| (PortMappingTransport::Udp, port));
            }
            _ => {}
        }
    }
    None
}

fn desired_port_mapping_endpoints(
    listeners: &HashSet<Multiaddr>,
) -> HashMap<PortMapperTaskKey, NonZeroU16> {
    const MAX_IPV6_TARGETS: usize = 2;
    let mut endpoints: Vec<_> = listeners.iter().filter_map(port_mapping_endpoint).collect();
    endpoints.sort_by_key(|endpoint| (endpoint.key, endpoint.port));
    let ipv6_candidates: Vec<_> = endpoints
        .iter()
        .filter_map(|endpoint| match endpoint.key.target {
            PortMappingTarget::Ipv6(address) => Some(address),
            PortMappingTarget::Ipv4 => None,
        })
        .collect();
    let ipv6_targets = select_ipv6_mapping_targets(ipv6_candidates, MAX_IPV6_TARGETS, |address| {
        pcp_ipv6::discover_gateway(address).is_ok()
    });
    let mut desired = HashMap::new();
    for endpoint in endpoints {
        if let PortMappingTarget::Ipv6(address) = endpoint.key.target {
            if !ipv6_targets.contains(&address) {
                continue;
            }
        }
        desired.entry(endpoint.key).or_insert(endpoint.port);
    }
    desired
}

/// Bound interface churn without letting unrouted VPN/privacy addresses crowd out the actual
/// default-route interface. If no candidate currently has a PCP route, deterministic fallbacks
/// still get workers so their retry loops can observe a later network change.
fn select_ipv6_mapping_targets(
    mut candidates: Vec<Ipv6Addr>,
    limit: usize,
    mut has_gateway: impl FnMut(Ipv6Addr) -> bool,
) -> HashSet<Ipv6Addr> {
    candidates.sort_unstable();
    candidates.dedup();
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .map(|address| (has_gateway(address), address))
        .collect();
    ranked.sort_by_key(|(routed, address)| (!*routed, *address));
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, address)| address)
        .collect()
}

fn port_mapper_report_is_current(
    tasks: &HashMap<PortMapperTaskKey, PortMapperTask>,
    report: &PortMapperReport,
) -> bool {
    tasks
        .get(&report.key)
        .is_some_and(|task| task.port == report.port && task.generation == report.generation)
}

/// A retired listener generation must leave neither a lease nor a failure behind. Failures are
/// current-state diagnostics, not history: retaining a removed privacy address grows the map and
/// keeps telling the user a worker that no longer exists is "retrying".
fn clear_retired_port_mapping_failures(
    unavailable: &mut HashMap<PortMappingKey, String>,
    key: PortMapperTaskKey,
) -> bool {
    let local_address = match key.target {
        PortMappingTarget::Ipv4 => None,
        PortMappingTarget::Ipv6(address) => Some(IpAddr::V6(address)),
    };
    let mechanisms: &[PortMappingMechanism] = match key.target {
        PortMappingTarget::Ipv4 => &[PortMappingMechanism::Pcp, PortMappingMechanism::NatPmp],
        PortMappingTarget::Ipv6(_) => &[PortMappingMechanism::Pcp],
    };
    let before = unavailable.len();
    for &mechanism in mechanisms {
        unavailable.remove(&(mechanism, key.transport, local_address));
    }
    // The IPv4 task's bound-UPnP companion keys its failures by the concrete interface address;
    // sweep those too so a retired worker stops claiming it is "retrying".
    if key.target == PortMappingTarget::Ipv4 {
        unavailable.retain(|&(mechanism, transport, local_address), _| {
            !(mechanism == PortMappingMechanism::Upnp
                && transport == key.transport
                && matches!(local_address, Some(IpAddr::V4(_))))
        });
    }
    unavailable.len() != before
}

/// Convert a router's IPv4 socket into the transport-specific libp2p address other peers dial.
fn mapped_multiaddr(addr: SocketAddrV4, transport: PortMappingTransport) -> Multiaddr {
    let base = Multiaddr::empty().with(Protocol::Ip4(*addr.ip()));
    match transport {
        PortMappingTransport::Tcp => base.with(Protocol::Tcp(addr.port())),
        PortMappingTransport::Udp => base.with(Protocol::Udp(addr.port())).with(Protocol::QuicV1),
    }
}

/// Convert an IPv6 PCP MAP result into the exact libp2p route peers should dial.
fn mapped_ipv6_multiaddr(addr: SocketAddrV6, transport: PortMappingTransport) -> Multiaddr {
    let base = Multiaddr::empty().with(Protocol::Ip6(*addr.ip()));
    match transport {
        PortMappingTransport::Tcp => base.with(Protocol::Tcp(addr.port())),
        PortMappingTransport::Udp => base.with(Protocol::Udp(addr.port())).with(Protocol::QuicV1),
    }
}

/// Ordered mechanisms a mapping attempt should try. PCP is the standards-track successor and is
/// preferred, but a successful PCP discovery does not suppress NAT-PMP: if its MAP request times
/// out or is denied, the caller continues to the next entry.
fn mapping_attempt_plan(pcp: bool, nat_pmp: bool) -> Vec<PortMappingMechanism> {
    let mut plan = Vec::with_capacity(2);
    if pcp {
        plan.push(PortMappingMechanism::Pcp);
    }
    if nat_pmp {
        plan.push(PortMappingMechanism::NatPmp);
    }
    plan
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MappingLeaseAction {
    /// The watch repeated the same socket; no product state changes.
    Continue,
    /// The lease disappeared. Withdraw it and return to the outer retry loop.
    Retry,
    /// The gateway renewed at a different public socket. Withdraw the old route, then publish this
    /// replacement while retaining the same worker.
    Replace(SocketAddrV4),
}

fn mapping_lease_action(previous: SocketAddrV4, next: Option<SocketAddrV4>) -> MappingLeaseAction {
    match next {
        Some(next) if next == previous => MappingLeaseAction::Continue,
        Some(next) => MappingLeaseAction::Replace(next),
        None => MappingLeaseAction::Retry,
    }
}

/// Probe PCP/NAT-PMP, create one mapping, and retain/renew it for the lifetime of the worker.
/// UPnP is disabled here because libp2p's behaviour already owns that protocol. Keeping the two
/// implementations disjoint prevents duplicate IGD leases and makes diagnostics truthful.
async fn run_port_mapper_attempt(
    transport: PortMappingTransport,
    port: NonZeroU16,
    tx: &PortMapperReporter,
) -> Result<(), ()> {
    let protocol = match transport {
        PortMappingTransport::Tcp => portmapper::Protocol::Tcp,
        PortMappingTransport::Udp => portmapper::Protocol::Udp,
    };
    let probe_client = portmapper::Client::new(portmapper::Config {
        enable_upnp: false,
        enable_pcp: true,
        enable_nat_pmp: true,
        protocol,
    });

    // Probe before requesting the mapping. Besides producing an honest diagnostic, this lets the
    // library choose NAT-PMP when PCP is absent; an unprobed one-shot would optimistically try PCP
    // and stop after that protocol's timeout without falling through.
    let probe = match probe_client.probe().await {
        Ok(Ok(probe)) => probe,
        Ok(Err(error)) => {
            let detail = error.to_string();
            for mechanism in [PortMappingMechanism::Pcp, PortMappingMechanism::NatPmp] {
                if tx
                    .send(PortMappingEvent::Unavailable {
                        mechanism,
                        transport,
                        local_address: None,
                        detail: detail.clone(),
                    })
                    .await
                    .is_err()
                {
                    return Err(());
                }
            }
            return Ok(());
        }
        Err(error) => {
            let detail = format!("port-mapping probe stopped: {error}");
            for mechanism in [PortMappingMechanism::Pcp, PortMappingMechanism::NatPmp] {
                if tx
                    .send(PortMappingEvent::Unavailable {
                        mechanism,
                        transport,
                        local_address: None,
                        detail: detail.clone(),
                    })
                    .await
                    .is_err()
                {
                    return Err(());
                }
            }
            return Ok(());
        }
    };

    let plan = mapping_attempt_plan(probe.pcp, probe.nat_pmp);
    for mechanism in [PortMappingMechanism::Pcp, PortMappingMechanism::NatPmp] {
        if !plan.contains(&mechanism)
            && tx
                .send(PortMappingEvent::Unavailable {
                    mechanism,
                    transport,
                    local_address: None,
                    detail: "no compatible gateway answered the probe".to_string(),
                })
                .await
                .is_err()
        {
            return Err(());
        }
    }

    drop(probe_client);

    // A probe result says a protocol answered discovery; it does not say the subsequent MAP
    // request will be granted. `portmapper` 0.18 only logs that request's error and leaves its
    // watch channel open at `None`, so waiting on `changed()` alone can hang forever. Give each
    // advertised protocol its own bounded attempt, allowing NAT-PMP to follow a failed PCP MAP.
    const ACQUIRE_WITHIN: Duration = Duration::from_secs(5);
    for mechanism in plan {
        let client = portmapper::Client::new(portmapper::Config {
            enable_upnp: false,
            enable_pcp: mechanism == PortMappingMechanism::Pcp,
            enable_nat_pmp: mechanism == PortMappingMechanism::NatPmp,
            protocol,
        });
        let mut external = client.watch_external_address();
        client.update_local_port(port);
        let first = tokio::select! {
            changed = external.changed() => {
                match changed {
                    Ok(()) => *external.borrow_and_update(),
                    Err(_) => None,
                }
            }
            _ = SystemClock.sleep(ACQUIRE_WITHIN) => None,
        };
        let Some(first) = first else {
            tx.send(PortMappingEvent::Unavailable {
                mechanism,
                transport,
                local_address: None,
                detail: format!(
                    "gateway answered discovery but did not grant a mapping within {}s",
                    ACQUIRE_WITHIN.as_secs()
                ),
            })
            .await
            .map_err(|_| ())?;
            continue;
        };

        tx.send(PortMappingEvent::Mapped {
            mechanism,
            transport,
            local_address: None,
            address: mapped_multiaddr(first, transport),
        })
        .await
        .map_err(|_| ())?;
        let mut previous = first;
        while external.changed().await.is_ok() {
            let next = *external.borrow_and_update();
            match mapping_lease_action(previous, next) {
                MappingLeaseAction::Continue => continue,
                MappingLeaseAction::Retry => {
                    tx.send(PortMappingEvent::Expired {
                        mechanism,
                        transport,
                        local_address: None,
                        address: mapped_multiaddr(previous, transport),
                    })
                    .await
                    .map_err(|_| ())?;
                    // The old lease really expired and the library's renewal attempt did not
                    // replace it. Return to the outer retry instead of waiting forever at `None`.
                    return Ok(());
                }
                MappingLeaseAction::Replace(next) => {
                    tx.send(PortMappingEvent::Expired {
                        mechanism,
                        transport,
                        local_address: None,
                        address: mapped_multiaddr(previous, transport),
                    })
                    .await
                    .map_err(|_| ())?;
                    tx.send(PortMappingEvent::Mapped {
                        mechanism,
                        transport,
                        local_address: None,
                        address: mapped_multiaddr(next, transport),
                    })
                    .await
                    .map_err(|_| ())?;
                    previous = next;
                }
            }
        }
        tx.send(PortMappingEvent::Expired {
            mechanism,
            transport,
            local_address: None,
            address: mapped_multiaddr(previous, transport),
        })
        .await
        .map_err(|_| ())?;
        return Ok(());
    }
    Ok(())
}

/// Retrying matters on laptops: the first probe may run before Wi-Fi has a default route, or the
/// user may move between networks without the stable listener changing. A successful mapping's
/// own lease task renews internally; this interval is reached only after an attempt ended without
/// a live watcher (or that watcher closed).
async fn run_port_mapper(
    transport: PortMappingTransport,
    port: NonZeroU16,
    tx: PortMapperReporter,
) {
    const RETRY_AFTER: Duration = Duration::from_secs(60);
    loop {
        if run_port_mapper_attempt(transport, port, &tx).await.is_err() {
            return;
        }
        SystemClock.sleep(RETRY_AFTER).await;
    }
}

/// How long a bound-search UPnP lease asks for. Bounded so an aborted worker's mapping dies on
/// its own; renewal at half-life keeps a live one continuous.
const UPNP_BOUND_LEASE_SECS: u32 = 2 * 60 * 60;
/// One IGD search from the bound socket. A router answers the multicast within a second on a
/// sane LAN; three seconds keeps the retry loop responsive without spamming discovery.
const UPNP_BOUND_SEARCH_TIMEOUT: Duration = Duration::from_secs(3);

/// The IPv4 source address the kernel would route toward the public internet, learned by
/// "connecting" an unbound UDP socket to a documentation address (RFC 5737; nothing is sent, the
/// kernel just resolves the route). This names the interface whose LAN hosts the default
/// gateway, i.e. the only interface where an IGD search can possibly be answered.
fn default_route_ipv4() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() && !ip.is_loopback() => Some(ip),
        _ => None,
    }
}

/// One bound-interface UPnP IGD attempt: search from the default-route interface, create the
/// mapping, then hold and renew it for the lifetime of the worker.
///
/// This exists because libp2p's UPnP behaviour searches from an unbound `0.0.0.0` socket and
/// lets the OS choose the multicast egress interface. On a machine with virtual adapters (WSL
/// vEthernet, VirtualBox host-only) the OS routinely picks one of those, the router never hears
/// the M-SEARCH, and diagnostics report "no gateway" while the router's UPnP is fine. (Proven
/// live 2026-08-22: the identical SSDP search bound to the LAN interface was answered while the
/// unbound one was not, correlating across twenty debug logs with whether the WSL adapter was
/// up.) Both implementations run and feed one event path, keyed apart by `local_address`
/// (`None` = libp2p, `Some(v4)` = this worker), so whichever hears the gateway supplies the
/// route; a shared lease renewed by both is an idempotent AddPortMapping at the gateway.
async fn run_upnp_bound_mapper_attempt(
    transport: PortMappingTransport,
    port: NonZeroU16,
    tx: &PortMapperReporter,
) -> Result<(), ()> {
    let Some(local_ip) = default_route_ipv4() else {
        // No IPv4 default route: nothing to bind to. Stay silent rather than write over the
        // unbound behaviour's diagnostic key; it reports its own status for this state.
        return Ok(());
    };
    let local_address = Some(IpAddr::V4(local_ip));
    let unavailable = |detail: String| PortMappingEvent::Unavailable {
        mechanism: PortMappingMechanism::Upnp,
        transport,
        local_address,
        detail,
    };
    let gateway = match igd_next::aio::tokio::search_gateway(igd_next::SearchOptions {
        bind_addr: SocketAddr::new(IpAddr::V4(local_ip), 0),
        timeout: Some(UPNP_BOUND_SEARCH_TIMEOUT),
        ..Default::default()
    })
    .await
    {
        Ok(gateway) => gateway,
        Err(error) => {
            tx.send(unavailable(format!(
                "no IGD gateway answered a search bound to {local_ip}: {error}"
            )))
            .await
            .map_err(|_| ())?;
            return Ok(());
        }
    };
    let protocol = match transport {
        PortMappingTransport::Tcp => igd_next::PortMappingProtocol::TCP,
        PortMappingTransport::Udp => igd_next::PortMappingProtocol::UDP,
    };
    let add_lease = || {
        gateway.add_port(
            protocol,
            port.get(),
            SocketAddr::new(IpAddr::V4(local_ip), port.get()),
            UPNP_BOUND_LEASE_SECS,
            "Mewtual",
        )
    };
    if let Err(error) = add_lease().await {
        tx.send(unavailable(format!("gateway refused the mapping: {error}")))
            .await
            .map_err(|_| ())?;
        return Ok(());
    }
    // The gateway's own answer, over the interface that demonstrably reaches it; a CGNAT/double-
    // NAT private answer is converted to an honest Unavailable by the actor's event fold.
    let external = match gateway.get_external_ip().await {
        Ok(IpAddr::V4(ip)) => ip,
        Ok(IpAddr::V6(ip)) => {
            tx.send(unavailable(format!(
                "gateway reported an IPv6 external address ({ip}) for an IPv4 mapping"
            )))
            .await
            .map_err(|_| ())?;
            return Ok(());
        }
        Err(error) => {
            tx.send(unavailable(format!(
                "gateway granted the mapping but the external-address query failed: {error}"
            )))
            .await
            .map_err(|_| ())?;
            return Ok(());
        }
    };
    let mut current = mapped_multiaddr(SocketAddrV4::new(external, port.get()), transport);
    tx.send(PortMappingEvent::Mapped {
        mechanism: PortMappingMechanism::Upnp,
        transport,
        local_address,
        address: current.clone(),
    })
    .await
    .map_err(|_| ())?;
    loop {
        SystemClock
            .sleep(Duration::from_secs(u64::from(UPNP_BOUND_LEASE_SECS / 2)))
            .await;
        // Renewal is the same AddPortMapping. A failure (router rebooted, moved networks, lease
        // slot reclaimed) expires the route and returns to the outer retry, which rediscovers
        // the gateway and recomputes the default-route interface.
        let renewed = match add_lease().await {
            Ok(()) => gateway.get_external_ip().await.ok(),
            Err(_) => None,
        };
        match renewed {
            Some(IpAddr::V4(ip)) => {
                let address = mapped_multiaddr(SocketAddrV4::new(ip, port.get()), transport);
                if address != current {
                    tx.send(PortMappingEvent::Expired {
                        mechanism: PortMappingMechanism::Upnp,
                        transport,
                        local_address,
                        address: current,
                    })
                    .await
                    .map_err(|_| ())?;
                    tx.send(PortMappingEvent::Mapped {
                        mechanism: PortMappingMechanism::Upnp,
                        transport,
                        local_address,
                        address: address.clone(),
                    })
                    .await
                    .map_err(|_| ())?;
                    current = address;
                }
            }
            _ => {
                tx.send(PortMappingEvent::Expired {
                    mechanism: PortMappingMechanism::Upnp,
                    transport,
                    local_address,
                    address: current,
                })
                .await
                .map_err(|_| ())?;
                return Ok(());
            }
        }
    }
}

/// How long a media-socket UPnP mapping asks for. The UPnP path has no renewal loop (the webview
/// owns the socket, not this crate), so the lease must outlive a long call on its own; four hours
/// covers the realistic ones and still dies unattended after a crash. PCP/NAT-PMP leases are
/// short and renewed by the retained client instead.
const MEDIA_MAP_LEASE_SECS: u32 = 4 * 60 * 60;

/// A live media-port mapping: the public socket to advertise plus whatever keeps the route
/// alive. A UPnP lease is bounded and self-sufficient; a PCP/NAT-PMP lease is short and renewed
/// by the retained `portmapper` client, so dropping this struct is what ends that route.
pub struct MediaPortMapping {
    /// The public socket a remote peer can be told about.
    pub external: SocketAddrV4,
    /// Which router protocol granted the route.
    pub mechanism: PortMappingMechanism,
    /// Whether the router's own state confirmed the mapping beyond the grant response: for UPnP,
    /// the mapping table was read back and the entry was present (some gateways acknowledge
    /// AddPortMapping and silently keep nothing); a PCP/NAT-PMP lease is the router's direct
    /// answer, so it counts as confirmed. True external reachability still needs an outside
    /// caller, which is the AutoNAT tier's job, not this field's claim.
    pub confirmed: bool,
    local_port: NonZeroU16,
    keepalive: Option<portmapper::Client>,
}

// `portmapper::Client` is a live handle, not data; report only whether one is retained.
impl std::fmt::Debug for MediaPortMapping {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaPortMapping")
            .field("external", &self.external)
            .field("mechanism", &self.mechanism)
            .field("confirmed", &self.confirmed)
            .field("local_port", &self.local_port)
            .field("keepalive", &self.keepalive.is_some())
            .finish()
    }
}

impl MediaPortMapping {
    /// Release the route. Best-effort for UPnP (the bounded lease dies on its own regardless);
    /// dropping the retained client is the whole release for PCP/NAT-PMP.
    pub async fn release(self) {
        if self.keepalive.is_none() {
            unmap_media_udp_port(self.local_port).await;
        }
    }
}

/// Scan the router's own mapping table for the UDP entry on `port`. The strongest check
/// available from inside the NAT; a gateway that does not implement the table reads as
/// unconfirmed, never as failure. Bounded: no home router legitimately needs more entries.
async fn upnp_mapping_confirmed(
    gateway: &igd_next::aio::Gateway<igd_next::aio::tokio::Tokio>,
    port: NonZeroU16,
) -> bool {
    for index in 0..64u32 {
        match gateway.get_generic_port_mapping_entry(index).await {
            Ok(entry) => {
                if entry.external_port == port.get()
                    && entry.protocol == igd_next::PortMappingProtocol::UDP
                    && entry.enabled
                {
                    return true;
                }
            }
            Err(_) => return false,
        }
    }
    false
}

async fn map_media_port_upnp(
    local_ip: Ipv4Addr,
    port: NonZeroU16,
) -> Result<MediaPortMapping, String> {
    let gateway = igd_next::aio::tokio::search_gateway(igd_next::SearchOptions {
        bind_addr: SocketAddr::new(IpAddr::V4(local_ip), 0),
        timeout: Some(UPNP_BOUND_SEARCH_TIMEOUT),
        ..Default::default()
    })
    .await
    .map_err(|error| format!("no IGD gateway answered a search bound to {local_ip}: {error}"))?;
    gateway
        .add_port(
            igd_next::PortMappingProtocol::UDP,
            port.get(),
            SocketAddr::new(IpAddr::V4(local_ip), port.get()),
            MEDIA_MAP_LEASE_SECS,
            "Mewtual call",
        )
        .await
        .map_err(|error| format!("gateway refused the mapping: {error}"))?;
    let external = match gateway.get_external_ip().await {
        Ok(IpAddr::V4(ip)) => ip,
        Ok(IpAddr::V6(ip)) => {
            return Err(format!(
                "gateway reported an IPv6 external address ({ip}) for an IPv4 mapping"
            ))
        }
        Err(error) => return Err(format!("external-address query failed: {error}")),
    };
    if !ipv4_is_globally_routable(&external) {
        return Err(format!(
            "gateway returned non-public address {external} (likely CGNAT or double NAT)"
        ));
    }
    let confirmed = upnp_mapping_confirmed(&gateway, port).await;
    Ok(MediaPortMapping {
        external: SocketAddrV4::new(external, port.get()),
        mechanism: PortMappingMechanism::Upnp,
        confirmed,
        local_port: port,
        keepalive: None,
    })
}

/// The PCP/NAT-PMP rung for a media port, for routers that don't speak UPnP. Probe first (an
/// unprobed one-shot optimistically tries PCP and stops at its timeout without falling through,
/// same trap as [`run_port_mapper_attempt`]), then give each advertised protocol a bounded
/// attempt. The client that succeeds is retained: it owns the short lease's renewal.
async fn map_media_port_pcp_natpmp(port: NonZeroU16) -> Result<MediaPortMapping, String> {
    const ACQUIRE_WITHIN: Duration = Duration::from_secs(5);
    let probe_client = portmapper::Client::new(portmapper::Config {
        enable_upnp: false,
        enable_pcp: true,
        enable_nat_pmp: true,
        protocol: portmapper::Protocol::Udp,
    });
    let probe = match probe_client.probe().await {
        Ok(Ok(probe)) => probe,
        Ok(Err(error)) => return Err(error.to_string()),
        Err(error) => return Err(format!("port-mapping probe stopped: {error}")),
    };
    drop(probe_client);
    let plan = mapping_attempt_plan(probe.pcp, probe.nat_pmp);
    if plan.is_empty() {
        return Err("no compatible gateway answered the probe".to_string());
    }
    for mechanism in plan {
        let client = portmapper::Client::new(portmapper::Config {
            enable_upnp: false,
            enable_pcp: mechanism == PortMappingMechanism::Pcp,
            enable_nat_pmp: mechanism == PortMappingMechanism::NatPmp,
            protocol: portmapper::Protocol::Udp,
        });
        let mut external = client.watch_external_address();
        client.update_local_port(port);
        let first = tokio::select! {
            changed = external.changed() => match changed {
                Ok(()) => *external.borrow_and_update(),
                Err(_) => None,
            },
            _ = SystemClock.sleep(ACQUIRE_WITHIN) => None,
        };
        if let Some(socket) = first {
            if !ipv4_is_globally_routable(socket.ip()) {
                return Err(format!(
                    "gateway returned non-public address {} (likely CGNAT or double NAT)",
                    socket.ip()
                ));
            }
            return Ok(MediaPortMapping {
                external: socket,
                mechanism,
                confirmed: true,
                local_port: port,
                keepalive: Some(client),
            });
        }
    }
    Err("a gateway answered discovery but granted no mapping".to_string())
}

/// Map one webview media UDP port through the router and return the live mapping. One-shot by
/// design: the mesh's mapping workers own the stable listen port, but a call's ICE agent binds
/// fresh ephemeral ports the mesh never sees. The caller signals the mapping's public socket as
/// an extra ICE candidate; a router mapping forwards from **any** source, which is what makes
/// one mapped side sufficient for a call regardless of the other side's NAT type.
///
/// Tries UPnP over the bound-interface search first (the proven path; see
/// [`run_upnp_bound_mapper_attempt`]), then PCP and NAT-PMP for routers that speak those
/// instead.
///
/// `claimed_ip` is the address the ICE candidate itself named (a mic-granted page gets real IPs
/// in its host candidates); when present it must be the default-route interface, or the port
/// belongs to another adapter and a mapping could never reach it. This replaced an active UDP
/// liveness probe: Windows Firewall rejects unsolicited inbound UDP to the webview with the
/// same ICMP a dead socket produces, so the probe disproved every port on a firewalled machine
/// (observed live 2026-08-22) while real ICE flows, being outbound-first, pass that firewall
/// fine. Never re-add a probe that cannot tell those apart.
pub async fn map_media_udp_port(
    port: NonZeroU16,
    claimed_ip: Option<Ipv4Addr>,
) -> Result<MediaPortMapping, String> {
    let local_ip = default_route_ipv4().ok_or("no IPv4 default route")?;
    if let Some(claimed) = claimed_ip {
        if claimed != local_ip {
            return Err(format!(
                "ICE socket {claimed}:{port} is not on the default-route interface ({local_ip}); a router mapping cannot reach it"
            ));
        }
    }
    match map_media_port_upnp(local_ip, port).await {
        Ok(mapping) => Ok(mapping),
        Err(upnp_error) => match map_media_port_pcp_natpmp(port).await {
            Ok(mapping) => Ok(mapping),
            Err(pcp_error) => Err(format!("UPnP: {upnp_error}; PCP/NAT-PMP: {pcp_error}")),
        },
    }
}

/// Best-effort removal of a mapping created by [`map_media_udp_port`]. Failure is acceptable:
/// the lease is bounded and dies on its own.
pub async fn unmap_media_udp_port(port: NonZeroU16) {
    let Some(local_ip) = default_route_ipv4() else {
        return;
    };
    let Ok(gateway) = igd_next::aio::tokio::search_gateway(igd_next::SearchOptions {
        bind_addr: SocketAddr::new(IpAddr::V4(local_ip), 0),
        timeout: Some(UPNP_BOUND_SEARCH_TIMEOUT),
        ..Default::default()
    })
    .await
    else {
        return;
    };
    let _ = gateway
        .remove_port(igd_next::PortMappingProtocol::UDP, port.get())
        .await;
}

/// The bound-interface UPnP companion to [`run_port_mapper`], with the same retry cadence and
/// the same laptop rationale: the first search may run before Wi-Fi has a default route.
async fn run_upnp_bound_mapper(
    transport: PortMappingTransport,
    port: NonZeroU16,
    tx: PortMapperReporter,
) {
    const RETRY_AFTER: Duration = Duration::from_secs(60);
    loop {
        if run_upnp_bound_mapper_attempt(transport, port, &tx)
            .await
            .is_err()
        {
            return;
        }
        SystemClock.sleep(RETRY_AFTER).await;
    }
}

/// Result of one bounded PCPv6 acquisition. Cancellation is distinct from a gateway failure so a
/// listener removal does not leave a misleading "unavailable" status behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PcpIpv6Acquire {
    Lease(pcp_ipv6::MapLease),
    Stopped,
}

async fn acquire_pcp_ipv6_lease(
    socket: &UdpSocket,
    local_ip: Ipv6Addr,
    local_port: NonZeroU16,
    protocol: pcp_ipv6::MapProtocol,
    nonce: [u8; 12],
    stop: &mut oneshot::Receiver<()>,
    clock: &impl Clock,
) -> Result<PcpIpv6Acquire, pcp_ipv6::PcpIpv6Error> {
    const RETRANSMIT_AFTER: Duration = Duration::from_secs(3);
    const ACQUIRE_WITHIN: Duration = Duration::from_secs(5);
    let request = pcp_ipv6::encode_map_request(
        local_ip,
        local_port,
        protocol,
        nonce,
        pcp_ipv6::REQUESTED_LIFETIME_SECONDS,
        None,
    );
    socket
        .send(&request)
        .await
        .map_err(|error| pcp_ipv6::PcpIpv6Error::Io(error.to_string()))?;
    let started = clock.monotonic_ms();
    let deadline = started.saturating_add(ACQUIRE_WITHIN.as_millis() as u64);
    let mut next_send = started.saturating_add(RETRANSMIT_AFTER.as_millis() as u64);
    let mut packet = [0u8; pcp_ipv6::RECEIVE_BUFFER_SIZE];
    loop {
        let now = clock.monotonic_ms();
        if now >= deadline {
            return Err(pcp_ipv6::PcpIpv6Error::Timeout);
        }
        let wake_at = next_send.min(deadline);
        let wait = Duration::from_millis(wake_at.saturating_sub(now));
        tokio::select! {
            received = socket.recv(&mut packet) => {
                let length = received
                    .map_err(|error| pcp_ipv6::PcpIpv6Error::Io(error.to_string()))?;
                match pcp_ipv6::decode_response(&packet[..length])
                    .and_then(|response| {
                        pcp_ipv6::validate_map_response(response, nonce, protocol, local_port)
                    })
                {
                    Ok(lease) => return Ok(PcpIpv6Acquire::Lease(lease)),
                    // A connected UDP socket already pins the gateway source. Still ignore
                    // unrelated/malformed datagrams rather than allowing them to end acquisition.
                    Err(pcp_ipv6::PcpIpv6Error::NonceMismatch
                        | pcp_ipv6::PcpIpv6Error::ProtocolMismatch
                        | pcp_ipv6::PcpIpv6Error::PortMismatch
                        | pcp_ipv6::PcpIpv6Error::Malformed(_)) => {}
                    Err(error) => return Err(error),
                }
            }
            _ = clock.sleep(wait) => {
                if wake_at == next_send && next_send < deadline {
                    // RFC 6887 requires retransmission with the exact same nonce. Reusing the
                    // encoded packet also guarantees every other request field remains identical.
                    socket
                        .send(&request)
                        .await
                        .map_err(|error| pcp_ipv6::PcpIpv6Error::Io(error.to_string()))?;
                    next_send = deadline;
                }
            }
            _ = &mut *stop => return Ok(PcpIpv6Acquire::Stopped),
        }
    }
}

async fn delete_pcp_ipv6_lease(
    socket: &UdpSocket,
    local_ip: Ipv6Addr,
    local_port: NonZeroU16,
    protocol: pcp_ipv6::MapProtocol,
    nonce: [u8; 12],
) {
    // Publication is withdrawn synchronously by the actor before this best-effort packet is sent.
    // Zero suggested fields avoid accidentally creating or extending a replacement socket.
    let request = pcp_ipv6::encode_map_request(local_ip, local_port, protocol, nonce, 0, None);
    let _ = socket.send(&request).await;
}

/// Maintain one IPv6 PCP firewall pinhole. The UDP socket is bound to the exact global listener
/// address and connected to that interface's scoped default router, which enforces both the PCP
/// Client IP and response-source invariants at the operating-system boundary.
async fn run_ipv6_port_mapper(
    local_ip: Ipv6Addr,
    transport: PortMappingTransport,
    local_port: NonZeroU16,
    tx: PortMapperReporter,
    stop: oneshot::Receiver<()>,
) {
    run_ipv6_port_mapper_with(
        local_ip,
        transport,
        local_port,
        tx,
        stop,
        Ipv6MapperDeps {
            clock: SystemClock,
            random: OsCryptoRng,
            discover_gateway: pcp_ipv6::discover_gateway,
        },
    )
    .await;
}

/// Injected runtime boundary for the PCPv6 worker. Grouping these dependencies keeps the worker's
/// protocol inputs readable while allowing deterministic clock, nonce and route tests.
struct Ipv6MapperDeps<C, R, D> {
    clock: C,
    random: R,
    discover_gateway: D,
}

async fn run_ipv6_port_mapper_with<C, R, D>(
    local_ip: Ipv6Addr,
    transport: PortMappingTransport,
    local_port: NonZeroU16,
    tx: PortMapperReporter,
    mut stop: oneshot::Receiver<()>,
    deps: Ipv6MapperDeps<C, R, D>,
) where
    C: Clock,
    R: CryptoRngCore + Send,
    D: FnMut(Ipv6Addr) -> Result<SocketAddrV6, pcp_ipv6::PcpIpv6Error> + Send,
{
    let Ipv6MapperDeps {
        clock,
        mut random,
        mut discover_gateway,
    } = deps;
    const RETRY_AFTER: Duration = Duration::from_secs(60);
    let protocol = match transport {
        PortMappingTransport::Tcp => pcp_ipv6::MapProtocol::Tcp,
        PortMappingTransport::Udp => pcp_ipv6::MapProtocol::Udp,
    };
    let local_address = Some(IpAddr::V6(local_ip));
    let mut gateway_and_nonce = None::<(SocketAddrV6, [u8; 12])>;

    loop {
        let gateway = match discover_gateway(local_ip) {
            Ok(gateway) => gateway,
            Err(error) => {
                if tx
                    .send(PortMappingEvent::Unavailable {
                        mechanism: PortMappingMechanism::Pcp,
                        transport,
                        local_address,
                        detail: format!("IPv6 firewall pinhole unavailable: {error}"),
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::select! {
                    _ = clock.sleep(RETRY_AFTER) => continue,
                    _ = &mut stop => return,
                }
            }
        };
        let nonce = if let Some((previous_gateway, nonce)) = gateway_and_nonce {
            if previous_gateway == gateway {
                nonce
            } else {
                let mut nonce = [0u8; 12];
                random.fill_bytes(&mut nonce);
                gateway_and_nonce = Some((gateway, nonce));
                nonce
            }
        } else {
            let mut nonce = [0u8; 12];
            random.fill_bytes(&mut nonce);
            gateway_and_nonce = Some((gateway, nonce));
            nonce
        };
        let socket = match UdpSocket::bind(SocketAddrV6::new(local_ip, 0, 0, 0)).await {
            Ok(socket) => socket,
            Err(error) => {
                let detail = format!("IPv6 firewall pinhole source bind failed: {error}");
                let _ = tx
                    .send(PortMappingEvent::Unavailable {
                        mechanism: PortMappingMechanism::Pcp,
                        transport,
                        local_address,
                        detail,
                    })
                    .await;
                tokio::select! {
                    _ = clock.sleep(RETRY_AFTER) => continue,
                    _ = &mut stop => return,
                }
            }
        };
        if let Err(error) = socket.connect(gateway).await {
            let _ = tx
                .send(PortMappingEvent::Unavailable {
                    mechanism: PortMappingMechanism::Pcp,
                    transport,
                    local_address,
                    detail: format!("IPv6 PCP gateway connection failed: {error}"),
                })
                .await;
            tokio::select! {
                _ = clock.sleep(RETRY_AFTER) => continue,
                _ = &mut stop => return,
            }
        }

        let mut lease = match acquire_pcp_ipv6_lease(
            &socket, local_ip, local_port, protocol, nonce, &mut stop, &clock,
        )
        .await
        {
            Ok(PcpIpv6Acquire::Lease(lease)) => lease,
            Ok(PcpIpv6Acquire::Stopped) => {
                delete_pcp_ipv6_lease(&socket, local_ip, local_port, protocol, nonce).await;
                return;
            }
            Err(error) => {
                let retry_after = match &error {
                    pcp_ipv6::PcpIpv6Error::ServerResult {
                        retry_after_seconds,
                        ..
                    } if *retry_after_seconds > 0 => {
                        Duration::from_secs(u64::from(*retry_after_seconds))
                    }
                    _ => RETRY_AFTER,
                };
                if tx
                    .send(PortMappingEvent::Unavailable {
                        mechanism: PortMappingMechanism::Pcp,
                        transport,
                        local_address,
                        detail: format!("IPv6 firewall pinhole unavailable: {error}"),
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::select! {
                    _ = clock.sleep(retry_after) => continue,
                    _ = &mut stop => return,
                }
            }
        };
        let mut address = mapped_ipv6_multiaddr(
            SocketAddrV6::new(lease.external_ip, lease.external_port.get(), 0, 0),
            transport,
        );
        // Start the monotonic lease timer at acceptance, before awaiting downstream publication.
        // A slow UI/snapshot consumer must not extend router state.
        let granted_at_ms = clock.monotonic_ms();
        let mut previous_epoch = lease.epoch;
        let mut previous_received_ms = granted_at_ms;
        let first_jitter = (random.next_u32() & 0xff) as u8;
        let mut schedule =
            pcp_ipv6::LeaseSchedule::new(granted_at_ms, lease.lifetime_seconds, first_jitter);
        if tx
            .send(PortMappingEvent::Mapped {
                mechanism: PortMappingMechanism::Pcp,
                transport,
                local_address,
                address: address.clone(),
            })
            .await
            .is_err()
        {
            return;
        }
        let mut packet = [0u8; pcp_ipv6::RECEIVE_BUFFER_SIZE];
        let ended_with_error = loop {
            let now = clock.monotonic_ms();
            let expires_at_ms = schedule.expires_at_ms();
            if schedule.is_expired(now) {
                break Some("IPv6 PCP firewall pinhole lease expired; retrying".to_string());
            }
            let wake_at = schedule.next_wake_ms();
            tokio::select! {
                _ = &mut stop => {
                    delete_pcp_ipv6_lease(
                        &socket, local_ip, local_port, protocol, nonce,
                    ).await;
                    break None;
                }
                _ = clock.sleep(Duration::from_millis(wake_at.saturating_sub(now))) => {
                    if wake_at >= expires_at_ms {
                        break Some("IPv6 PCP firewall pinhole lease expired; retrying".to_string());
                    }
                    let renew = pcp_ipv6::encode_map_request(
                        local_ip,
                        local_port,
                        protocol,
                        nonce,
                        pcp_ipv6::REQUESTED_LIFETIME_SECONDS,
                        Some((lease.external_ip, lease.external_port)),
                    );
                    if let Err(error) = socket.send(&renew).await {
                        tracing::debug!(%error, %local_ip, %transport, "IPv6 PCP renewal send failed");
                    }
                    schedule.renewal_sent(now);
                }
                received = socket.recv(&mut packet) => {
                    let length = match received {
                        Ok(length) => length,
                        Err(error) => {
                            // A broken local receive path prevents further renewal, but it does
                            // not revoke the lease the router already granted. Keep the address
                            // published through that lease's monotonic deadline.
                            let detail = format!(
                                "IPv6 PCP socket failed; existing pinhole retained until expiry: {error}"
                            );
                            let remaining = Duration::from_millis(
                                schedule.expires_at_ms().saturating_sub(clock.monotonic_ms()),
                            );
                            tokio::select! {
                                _ = clock.sleep(remaining) => break Some(detail),
                                _ = &mut stop => {
                                    delete_pcp_ipv6_lease(
                                        &socket, local_ip, local_port, protocol, nonce,
                                    ).await;
                                    break None;
                                }
                            }
                        },
                    };
                    let response = match pcp_ipv6::decode_response(&packet[..length]) {
                        Ok(response) => response,
                        Err(error) => {
                            tracing::debug!(%error, %local_ip, %transport, "ignored malformed IPv6 PCP response");
                            continue;
                        }
                    };
                    if let pcp_ipv6::DecodedResponse::Announce { result_code, .. } = response {
                        let received_ms = clock.monotonic_ms();
                        if result_code == 0 && pcp_ipv6::epoch_may_have_reset(
                            previous_epoch,
                            previous_received_ms,
                            response.epoch(),
                            received_ms,
                        ) {
                            // Randomize rapid recovery so a restarted gateway does not receive a
                            // synchronized burst from every client on the link. This worker owns
                            // one lease; the documented narrow-client limitation is that sibling
                            // transport leases learn the epoch independently.
                            let delay = Duration::from_millis(u64::from(random.next_u32() % 5_001));
                            schedule.renew_after(received_ms, delay);
                        }
                        previous_epoch = response.epoch();
                        previous_received_ms = received_ms;
                        if result_code != 0 {
                            tracing::debug!(
                                %local_ip,
                                %transport,
                                result_code,
                                "ignored unsuccessful IPv6 PCP ANNOUNCE"
                            );
                        }
                        continue;
                    }
                    let received_epoch = response.epoch();
                    let received_ms = clock.monotonic_ms();
                    let renewed = match pcp_ipv6::validate_map_response(
                        response, nonce, protocol, local_port,
                    ) {
                        Ok(renewed) => renewed,
                        Err(pcp_ipv6::PcpIpv6Error::NonceMismatch
                            | pcp_ipv6::PcpIpv6Error::ProtocolMismatch
                            | pcp_ipv6::PcpIpv6Error::PortMismatch
                            | pcp_ipv6::PcpIpv6Error::Malformed(_)) => continue,
                        Err(pcp_ipv6::PcpIpv6Error::ServerResult {
                            retry_after_seconds,
                            reason,
                            code,
                        }) => {
                            // A renewal refusal is not a deletion: the previously granted lease
                            // remains router state through its original deadline. Honor a nonzero
                            // retry horizon while never moving that deadline.
                            if retry_after_seconds > 0 {
                                schedule.defer_retry(
                                    received_ms,
                                    Duration::from_secs(u64::from(retry_after_seconds)),
                                );
                            }
                            previous_epoch = received_epoch;
                            previous_received_ms = received_ms;
                            tracing::debug!(
                                %local_ip,
                                %transport,
                                code,
                                reason,
                                retry_after_seconds,
                                "IPv6 PCP renewal refused; retaining existing lease until expiry"
                            );
                            continue;
                        }
                        Err(error) => break Some(format!("IPv6 PCP renewal failed: {error}")),
                    };
                    let renewed_address = mapped_ipv6_multiaddr(
                        SocketAddrV6::new(
                            renewed.external_ip,
                            renewed.external_port.get(),
                            0,
                            0,
                        ),
                        transport,
                    );
                    if renewed_address != address {
                        if tx.send(PortMappingEvent::Expired {
                            mechanism: PortMappingMechanism::Pcp,
                            transport,
                            local_address,
                            address: address.clone(),
                        }).await.is_err() {
                            return;
                        }
                        if tx.send(PortMappingEvent::Mapped {
                            mechanism: PortMappingMechanism::Pcp,
                            transport,
                            local_address,
                            address: renewed_address.clone(),
                        }).await.is_err() {
                            return;
                        }
                        address = renewed_address;
                    }
                    lease = renewed;
                    let granted_at_ms = clock.monotonic_ms();
                    previous_epoch = lease.epoch;
                    previous_received_ms = granted_at_ms;
                    let jitter = (random.next_u32() & 0xff) as u8;
                    schedule.renewed(granted_at_ms, lease.lifetime_seconds, jitter);
                }
            }
        };

        if tx
            .send(PortMappingEvent::Expired {
                mechanism: PortMappingMechanism::Pcp,
                transport,
                local_address,
                address,
            })
            .await
            .is_err()
        {
            return;
        }
        let Some(detail) = ended_with_error else {
            return;
        };
        if tx
            .send(PortMappingEvent::Unavailable {
                mechanism: PortMappingMechanism::Pcp,
                transport,
                local_address,
                detail,
            })
            .await
            .is_err()
        {
            return;
        }
        // The old gateway may have disappeared. Discovery on the next pass decides whether this
        // remains the same nonce generation or a new router needs a fresh nonce.
        tokio::select! {
            _ = clock.sleep(RETRY_AFTER) => {}
            _ = &mut stop => return,
        }
    }
}

struct Actor {
    swarm: Swarm<MeshBehaviour>,
    /// Whether this product instance explicitly opted into changing the local gateway.
    enable_port_mapping: bool,
    cmd_rx: mpsc::Receiver<Command>,
    event_tx: mpsc::Sender<TransportEvent>,
    /// Coalesced, present-time liveness/path table. Pre-actor admission and infrastructure waits
    /// observe this instead of consuming the sole ordered event stream that `ChannelSync` owns.
    connection_snapshot_tx: watch::Sender<Vec<PeerConnectionSnapshot>>,
    /// Address-bearing companion to the privacy-preserving connection snapshot. Only current
    /// outbound, Noise-authenticated direct routes enter this table; consumers use it to seal a
    /// same-LAN reconnect hint after admission, never for UI presence.
    authenticated_route_tx: watch::Sender<Vec<AuthenticatedDialRoute>>,
    /// Session-bounded successful outbound routes. Unlike `authenticated_route_tx`, a close does
    /// not erase these before an authorized recovery capture can seal its evidence.
    authenticated_route_evidence_tx: watch::Sender<Vec<AuthenticatedDialRoute>>,
    listen_tx: mpsc::Sender<Multiaddr>,
    active_listeners: HashSet<Multiaddr>,
    listener_snapshot_tx: watch::Sender<ListenerSnapshot>,
    /// Unified UPnP/PCP/NAT-PMP lifecycle stream for the product layer. It persists successful
    /// addresses into invites/peer records and removes them again on lease expiry.
    port_mapping_tx: watch::Sender<PortMappingSnapshot>,
    /// PCP/NAT-PMP workers report back here so the swarm actor alone mutates external addresses.
    port_mapper_tx: mpsc::Sender<PortMapperReport>,
    port_mapper_rx: mpsc::Receiver<PortMapperReport>,
    /// One worker for each transport and independently routed address family/interface. The exact
    /// key is load-bearing: a PCPv6/TCP lease must not overwrite PCPv4/TCP or another GUA.
    port_mapper_tasks: HashMap<PortMapperTaskKey, PortMapperTask>,
    /// Monotonic task identity. A listener can disappear and reappear with the same address/port;
    /// buffered events from its former worker must not become ownership for the new generation.
    next_port_mapper_generation: u64,
    /// Router-lease ownership by mechanism, transport and local-address generation. Libp2p stores
    /// external addresses as a set, so this layer supplies reference counting when mechanisms or
    /// families expose the same dial address.
    active_port_mappings: HashMap<PortMappingKey, Multiaddr>,
    /// Latest scoped failure for keys without a live lease. Coalesced with the active map into a
    /// watch snapshot so an idle/non-UI `MeshService` does not accumulate diagnostic events.
    port_mapping_unavailable: HashMap<PortMappingKey, String>,
    /// Explicit/operator-provided addresses are independent owners. Expiry of an identical router
    /// mapping must never withdraw a still-configured manual forward.
    configured_external_addrs: HashSet<Multiaddr>,
    /// Per-address AutoNAT v2 outcomes. Kept separate from transport connectivity because a
    /// failed dial-back does not mean a peer connection failed, and a successful one is
    /// diagnostic evidence rather than a membership event.
    autonat_tx: watch::Sender<AutoNatSnapshot>,
    /// Latest observation for every address/server pair. A second successful route must survive a
    /// later failure of the first, and a consumer that starts late still needs the full evidence.
    /// The FIFO order enforces `MAX_AUTONAT_OBSERVATIONS` under peer/address churn.
    autonat_results: HashMap<AutoNatKey, AutoNatResult>,
    autonat_order: VecDeque<AutoNatKey>,
    /// Live relay-circuit listen addresses. The product must withdraw one when the reservation's
    /// listener expires instead of continuing to present a dead circuit as "relay ready".
    relay_addresses: HashSet<Multiaddr>,
    relay_address_tx: watch::Sender<RelayAddressSnapshot>,
    /// Latest outbound-address observation from each connected Identify peer. This is kept wholly
    /// separate from external-address ownership so telemetry can never become a dial candidate by
    /// accident.
    mesh_observations: HashMap<libp2p::PeerId, Multiaddr>,
    mesh_observation_order: VecDeque<libp2p::PeerId>,
    mesh_observation_tx: watch::Sender<MeshObservationSnapshot>,
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
    /// Last known libp2p identity and dial addresses for every peer this node has been connected
    /// to, deliberately kept **past** disconnect. `peers` is the live set and `to_peer` is a
    /// one-way BLAKE3, so without this an outbound request to a peer whose connection had dropped
    /// could not even name a dial target and failed instantly, with no attempt to reconnect. That
    /// turned one dropped connection into a permanently dead route for call signalling, which
    /// re-sends on a timer and therefore never recovered on its own.
    ///
    /// Bounded FIFO: this is a redial hint, not an address book, and the authoritative one
    /// (signed peer records) lives above the transport.
    recent_peers: HashMap<PeerId, RecentPeer>,
    recent_peer_order: VecDeque<PeerId>,
    pending_req: HashMap<OutboundRequestId, oneshot::Sender<Result<Bytes, TransportError>>>,
    pending_publish: Vec<(Topic, Bytes)>,
    /// Peers this node uses as **infrastructure**: rendezvous nodes it registers or discovers at,
    /// and relays it holds (or is opening) a circuit reservation on. Learned from the commands
    /// that name them; nothing else can tell an infra target from a member at dial time.
    infra_peers: HashSet<libp2p::PeerId>,
    /// Addresses this node is dialing, or is already connected to a peer over. The dial gate is
    /// keyed on **(peer, address)** rather than on peer: a repeat of an address already covered is
    /// suppressed, a *new* address for a peer we already reach is not.
    covered_addrs: HashMap<libp2p::PeerId, HashSet<Multiaddr>>,
    /// Member-peer dials accumulated during one drain of the command queue, so a peer's addresses
    /// are handed to libp2p as one racing dial rather than N sequential ones.
    pending_dials: HashMap<libp2p::PeerId, Vec<Multiaddr>>,
    /// Coarse path evidence for every established libp2p connection. The swarm's hard limits
    /// bound this to 320 entries globally and eight per peer. Keying by `ConnectionId` is
    /// load-bearing: DCUtR temporarily leaves relay and direct connections live together, and a
    /// close for either one must remove only that path rather than declaring the peer unreachable.
    connection_paths: HashMap<(libp2p::PeerId, ConnectionId), ConnectionPath>,
    /// Exact authenticated dial endpoints, keyed by physical connection so a close removes only
    /// its own route while a simultaneous TCP/QUIC or DCUtR connection survives.
    authenticated_routes: HashMap<(libp2p::PeerId, ConnectionId), AuthenticatedDialRoute>,
    /// The newest bounded outbound Noise successes, retained across close edges for this process.
    authenticated_route_evidence: VecDeque<AuthenticatedDialRoute>,
    /// Peers **this node's own configuration** named as infrastructure or as a bootstrap: the
    /// relays it reserves on, the rendezvous nodes it registers/discovers at, and the addresses
    /// it was constructed to dial. They are never deniable by a membership action.
    ///
    /// This is the transport's half of the defence against an eviction aimed at a third party.
    /// The membership layer resolves a removed device to a peer id that the removed device
    /// *asserted*, and nothing binds that assertion to the device, so a member with a modified
    /// client can name the group's relay in its own peer record and have every other member
    /// block it on removal, taking NAT traversal down group-wide. What a peer *is* to this node
    /// is decided here, locally, by what this node was told to dial; no record from the wire can
    /// change it. Kept separate from `infra_peers`, which carries dial-gating semantics.
    protected: HashSet<libp2p::PeerId>,
}

impl Actor {
    /// Publish the current, bounded connection table without exposing addresses or connection ids.
    fn publish_connection_snapshot(&self) {
        let mut peers: Vec<_> = self
            .peers
            .iter()
            .map(|(peer, libp2p_peer)| PeerConnectionSnapshot {
                peer: *peer,
                active: active_connection_paths(&self.connection_paths, *libp2p_peer),
            })
            .collect();
        peers.sort_by_key(|snapshot| snapshot.peer);
        self.connection_snapshot_tx.send_replace(peers);

        self.authenticated_route_tx
            .send_replace(authenticated_route_snapshot(&self.authenticated_routes));
    }

    /// Publish a full, deterministic path snapshot after one connection edge.
    fn peer_paths_event(
        &self,
        peer_id: libp2p::PeerId,
        newly_established: Option<ConnectionPath>,
    ) -> TransportEvent {
        // Multiple physical connections can have the same coarse description. Do not leak that
        // count into product diagnostics, and keep snapshots stable across HashMap iteration.
        let active = active_connection_paths(&self.connection_paths, peer_id);
        TransportEvent::PeerPathsChanged {
            peer: to_peer(&peer_id),
            active,
            newly_established,
        }
    }

    async fn run(mut self) {
        let mut inbound: InboundResponses = FuturesUnordered::new();
        loop {
            tokio::select! {
                maybe_cmd = self.cmd_rx.recv() => {
                    match maybe_cmd {
                        Some(cmd) => {
                            self.handle_command(cmd);
                            // Drain whatever the caller has already queued before touching the
                            // swarm again. A dial plan arrives as one `Command::Dial` per address
                            // (the sync layer loops over a peer's addresses), so draining first is
                            // what lets those addresses be raced in a single dial. Nothing here
                            // depends on the drain succeeding: an address that misses the batch is
                            // dialed on its own, which is still correct, just less efficient.
                            while let Ok(next) = self.cmd_rx.try_recv() {
                                self.handle_command(next);
                            }
                            self.flush_dials();
                        }
                        None => break, // all handles dropped
                    }
                }
                event = self.swarm.select_next_some() => {
                    self.on_swarm_event(event, &mut inbound).await;
                }
                Some(report) = self.port_mapper_rx.recv() => {
                    let current = port_mapper_report_is_current(&self.port_mapper_tasks, &report);
                    if current {
                        self.on_port_mapping_event(report.event);
                    } else {
                        tracing::debug!(
                            ?report.key,
                            generation = report.generation,
                            "ignoring stale router-mapping worker event"
                        );
                    }
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
        // A normal command-channel shutdown has an explicit empty terminal value. A panic cannot
        // run this line, so readers additionally reject any retained value once the sender closes.
        self.connection_snapshot_tx.send_replace(Vec::new());
        self.authenticated_route_tx.send_replace(Vec::new());
        self.authenticated_route_evidence_tx
            .send_replace(Vec::new());
    }

    /// Reconcile router workers from the authoritative live listener set. This runs for both
    /// `NewListenAddr` and `ExpiredListenAddr`; start-only lifecycle management leaves a router
    /// pinhole and advertised route behind after an interface disappears.
    fn reconcile_port_mappers(&mut self) {
        // Bound privacy-address/interface churn. Select at most two distinct GUAs, then keep both
        // TCP and QUIC workers for those chosen interfaces.
        let desired = desired_port_mapping_endpoints(&self.active_listeners);

        let stale: Vec<_> = self
            .port_mapper_tasks
            .iter()
            .filter_map(|(key, task)| {
                (desired.get(key) != Some(&task.port) || !task.is_fully_running()).then_some(*key)
            })
            .collect();
        for key in stale {
            if let Some(task) = self.port_mapper_tasks.remove(&key) {
                let local_address = match key.target {
                    PortMappingTarget::Ipv4 => None,
                    PortMappingTarget::Ipv6(address) => Some(IpAddr::V6(address)),
                };
                let mechanisms: &[PortMappingMechanism] = match key.target {
                    PortMappingTarget::Ipv4 => {
                        &[PortMappingMechanism::Pcp, PortMappingMechanism::NatPmp]
                    }
                    PortMappingTarget::Ipv6(_) => &[PortMappingMechanism::Pcp],
                };
                // Router deletion is best effort, but live product state is not: withdraw every
                // owner before asking the worker to stop so a late packet cannot keep a stale
                // route in an invite.
                for &mechanism in mechanisms {
                    if let Some(address) = self
                        .active_port_mappings
                        .get(&(mechanism, key.transport, local_address))
                        .cloned()
                    {
                        self.on_port_mapping_event(PortMappingEvent::Expired {
                            mechanism,
                            transport: key.transport,
                            local_address,
                            address,
                        });
                    }
                }
                // The IPv4 task's bound-UPnP companion keys its routes by the concrete interface
                // address, which the actor does not track; withdraw whatever it published.
                if key.target == PortMappingTarget::Ipv4 {
                    let scoped: Vec<_> = self
                        .active_port_mappings
                        .iter()
                        .filter(|((mechanism, transport, local_address), _)| {
                            *mechanism == PortMappingMechanism::Upnp
                                && *transport == key.transport
                                && matches!(local_address, Some(IpAddr::V4(_)))
                        })
                        .map(|(&(mechanism, transport, local_address), address)| {
                            (mechanism, transport, local_address, address.clone())
                        })
                        .collect();
                    for (mechanism, transport, local_address, address) in scoped {
                        self.on_port_mapping_event(PortMappingEvent::Expired {
                            mechanism,
                            transport,
                            local_address,
                            address,
                        });
                    }
                }
                // `Expired` normally records "retrying" and an unavailable-only worker has no
                // active owner to expire at all. This task is being retired, so remove both forms
                // of stale failure before publishing the final coalesced state.
                if clear_retired_port_mapping_failures(&mut self.port_mapping_unavailable, key) {
                    self.publish_port_mapping_snapshot();
                }
                task.stop_gracefully();
            }
        }

        for (key, port) in desired {
            if self.port_mapper_tasks.contains_key(&key) {
                continue;
            }
            let generation = self.next_port_mapper_generation;
            self.next_port_mapper_generation = self.next_port_mapper_generation.wrapping_add(1);
            let tx = PortMapperReporter {
                tx: self.port_mapper_tx.clone(),
                key,
                port,
                generation,
            };
            let (handle, companion, stop) = match key.target {
                PortMappingTarget::Ipv4 => (
                    tokio::spawn(run_port_mapper(key.transport, port, tx.clone())),
                    Some(tokio::spawn(run_upnp_bound_mapper(key.transport, port, tx))),
                    None,
                ),
                PortMappingTarget::Ipv6(local_ip) => {
                    let (stop_tx, stop_rx) = oneshot::channel();
                    (
                        tokio::spawn(run_ipv6_port_mapper(
                            local_ip,
                            key.transport,
                            port,
                            tx,
                            stop_rx,
                        )),
                        None,
                        Some(stop_tx),
                    )
                }
            };
            self.port_mapper_tasks.insert(
                key,
                PortMapperTask {
                    port,
                    generation,
                    handle: Some(handle),
                    companion,
                    stop,
                },
            );
        }
    }

    /// Apply a mapper event to the swarm before surfacing it. Successful mappings become AutoNAT
    /// candidates and rendezvous/identify addresses; expired leases are withdrawn immediately so
    /// a stale public address is not kept in the live peer record.
    fn on_port_mapping_event(&mut self, event: PortMappingEvent) {
        // A router behind CGNAT/double NAT can successfully map its *private* upstream socket.
        // That is useful diagnostic evidence but not an internet route, so never advertise it as
        // one or ask AutoNAT to waste a callback on it.
        let event = match event {
            PortMappingEvent::Mapped {
                mechanism,
                transport,
                local_address,
                address,
            } if !addr_is_globally_routable(&address) => {
                // libp2p-UPnP may already have promoted the address before its behaviour event is
                // surfaced. Remove it unless an explicit/manual owner independently configured
                // the identical socket; the router lease itself is still refused below.
                if !external_address_is_allowed(&self.configured_external_addrs, &address) {
                    self.swarm.remove_external_address(&address);
                    self.forget_autonat_address(&address);
                }
                if let Some(old) = self
                    .active_port_mappings
                    .get(&(mechanism, transport, local_address))
                    .cloned()
                {
                    self.on_port_mapping_event(PortMappingEvent::Expired {
                        mechanism,
                        transport,
                        local_address,
                        address: old,
                    });
                }
                PortMappingEvent::Unavailable {
                    mechanism,
                    transport,
                    local_address,
                    detail: format!(
                        "gateway returned non-public address {address} (likely CGNAT or double NAT)"
                    ),
                }
            }
            event => event,
        };
        match &event {
            PortMappingEvent::Mapped {
                mechanism,
                transport,
                local_address,
                address,
            } => {
                tracing::info!(
                    %mechanism,
                    %transport,
                    %address,
                    "router mapped a public address; offering it to AutoNAT"
                );
                let (remove_old, add_new) = activate_port_mapping(
                    &mut self.active_port_mappings,
                    (*mechanism, *transport, *local_address),
                    address.clone(),
                );
                self.port_mapping_unavailable
                    .remove(&(*mechanism, *transport, *local_address));
                if let Some(old) = remove_old {
                    if !self.configured_external_addrs.contains(&old) {
                        self.swarm.remove_external_address(&old);
                        self.forget_autonat_address(&old);
                    }
                }
                if add_new && !self.configured_external_addrs.contains(address) {
                    self.swarm.behaviour_mut().autonat_client.on_swarm_event(
                        FromSwarm::NewExternalAddrCandidate(NewExternalAddrCandidate {
                            addr: address,
                        }),
                    );
                    self.swarm.add_external_address(address.clone());
                } else if local_address.is_some() {
                    // An IPv6 listener can already be a configured candidate before its firewall
                    // pinhole exists. Re-offer it after a new PCP lease so stale pre-pinhole
                    // AutoNAT evidence cannot be the only observation for this ownership epoch.
                    self.swarm.behaviour_mut().autonat_client.on_swarm_event(
                        FromSwarm::NewExternalAddrCandidate(NewExternalAddrCandidate {
                            addr: address,
                        }),
                    );
                }
                self.flush_pending_registers();
            }
            PortMappingEvent::Unavailable {
                mechanism,
                transport,
                local_address,
                detail,
            } => {
                // A router with no PCP/NAT-PMP re-answers "no" on every discovery cycle, for
                // every mechanism, transport and interface. Logged unconditionally that was 356
                // of one debug log's 601 lines: the single most common message, saying the same
                // thing it said eighteen seconds earlier, burying everything worth reading.
                // The first answer for a given probe is news and stays at info; an identical
                // repeat drops to debug. A *changed* detail is news again.
                let previous = self
                    .port_mapping_unavailable
                    .insert((*mechanism, *transport, *local_address), detail.clone());
                if !mapping_unavailable_is_news(previous.as_deref(), detail) {
                    tracing::debug!(%mechanism, %transport, %detail, "router mapping still unavailable");
                } else {
                    tracing::info!(%mechanism, %transport, %detail, "router mapping unavailable");
                }
            }
            PortMappingEvent::Expired {
                mechanism,
                transport,
                local_address,
                address,
            } => {
                tracing::info!(%mechanism, %transport, %address, "router port mapping expired");
                let matched =
                    self.active_port_mappings
                        .get(&(*mechanism, *transport, *local_address))
                        == Some(address);
                let last_mapping_owner = expire_port_mapping(
                    &mut self.active_port_mappings,
                    (*mechanism, *transport, *local_address),
                    address,
                );
                let still_owned = self
                    .active_port_mappings
                    .values()
                    .any(|candidate| candidate == address)
                    || self.configured_external_addrs.contains(address);
                if *mechanism == PortMappingMechanism::Upnp && still_owned {
                    // libp2p-UPnP emits `ToSwarm::ExternalAddrExpired` before its public expiry
                    // event. Re-add an identical address still owned by PCP/NAT-PMP or a manual
                    // forward; libp2p's external-address set has no ownership count of its own.
                    self.swarm.add_external_address(address.clone());
                } else if last_mapping_owner && !still_owned {
                    self.swarm.remove_external_address(address);
                    self.forget_autonat_address(address);
                }
                if matched {
                    self.port_mapping_unavailable.insert(
                        (*mechanism, *transport, *local_address),
                        format!("previous mapping {address} expired; retrying"),
                    );
                    if local_address.is_some() {
                        // A direct callback result observed before a firewall pinhole ended is not
                        // evidence for a future lease, even when the GUA itself remains configured.
                        self.forget_autonat_address(address);
                    }
                }
            }
        }
        self.publish_port_mapping_snapshot();
    }

    /// Publish the current mapping state as one bounded/coalesced value. A consumer can miss
    /// intermediate retries but never observes an expired address as current.
    fn publish_port_mapping_snapshot(&self) {
        let mut active: Vec<_> = self
            .active_port_mappings
            .iter()
            .map(
                |(&(mechanism, transport, local_address), address)| ActivePortMapping {
                    mechanism,
                    transport,
                    local_address,
                    address: address.clone(),
                },
            )
            .collect();
        active.sort_by(|a, b| {
            (
                a.mechanism,
                a.transport,
                a.local_address,
                a.address.to_string(),
            )
                .cmp(&(
                    b.mechanism,
                    b.transport,
                    b.local_address,
                    b.address.to_string(),
                ))
        });
        let mut unavailable: Vec<_> = self
            .port_mapping_unavailable
            .iter()
            .filter(|(key, _)| !self.active_port_mappings.contains_key(key))
            .map(
                |(&(mechanism, transport, local_address), detail)| PortMappingFailure {
                    mechanism,
                    transport,
                    local_address,
                    detail: detail.clone(),
                },
            )
            .collect();
        unavailable
            .sort_by_key(|failure| (failure.mechanism, failure.transport, failure.local_address));
        self.port_mapping_tx.send_replace(PortMappingSnapshot {
            active,
            unavailable,
        });
    }

    /// Publish all bounded current AutoNAT observations. `latest` is present only for an actual
    /// dial-back result; lifecycle pruning publishes `None` so an expired route cannot be mistaken
    /// for fresh evidence by event-style consumers.
    fn publish_autonat_snapshot(&self, latest: Option<AutoNatResult>) {
        let mut results: Vec<_> = self.autonat_results.values().cloned().collect();
        results.sort_by(|a, b| {
            (a.address.to_string(), a.server.to_string())
                .cmp(&(b.address.to_string(), b.server.to_string()))
        });
        self.autonat_tx
            .send_replace(AutoNatSnapshot { results, latest });
    }

    fn publish_mesh_observations(&self) {
        let mut observations: Vec<_> = self
            .mesh_observations
            .iter()
            .map(|(observer, address)| MeshObservation {
                observer: *observer,
                address: address.clone(),
            })
            .collect();
        observations.sort_by(|a, b| {
            (a.observer.to_string(), a.address.to_string())
                .cmp(&(b.observer.to_string(), b.address.to_string()))
        });
        self.mesh_observation_tx
            .send_replace(MeshObservationSnapshot { observations });
    }

    fn forget_autonat_address(&mut self, address: &Multiaddr) {
        if forget_autonat_address(&mut self.autonat_results, &mut self.autonat_order, address) {
            self.publish_autonat_snapshot(None);
        }
    }

    fn autonat_address_is_current(&self, address: &Multiaddr) -> bool {
        autonat_candidate_is_current(
            &self.configured_external_addrs,
            &self.active_port_mappings,
            address,
        )
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
                    // No subscribers known yet; retry when one appears.
                    tracing::trace!(bytes = len, "publish queued (no subscribers yet)");
                    self.pending_publish.push((topic, data));
                } else {
                    tracing::trace!(bytes = len, "published");
                }
            }
            Command::Request { peer, data, reply } => match self.request_target(&peer) {
                Some(libp2p_peer) => {
                    tracing::debug!(peer = %libp2p_peer, bytes = data.len(), "send request");
                    let id = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_request(&libp2p_peer, data.to_vec());
                    self.pending_req.insert(id, reply);
                }
                None => {
                    tracing::warn!(?peer, "request to unknown peer");
                    let _ = reply.send(Err(TransportError::Unreachable(peer)));
                }
            },
            Command::RequestConnected { peer, data, reply } => {
                let live = self
                    .peers
                    .get(&peer)
                    .copied()
                    // `peers` is updated from swarm events, but make the present-time transport
                    // check as well. No actor event can interleave between this check and
                    // `send_request`, so a disconnected recent peer cannot turn this command
                    // into a dial.
                    .filter(|target| self.swarm.is_connected(target));
                match live {
                    Some(libp2p_peer) => {
                        tracing::debug!(peer = %libp2p_peer, bytes = data.len(), "send connected-only request");
                        let id = self
                            .swarm
                            .behaviour_mut()
                            .request_response
                            .send_request(&libp2p_peer, data.to_vec());
                        self.pending_req.insert(id, reply);
                    }
                    None => {
                        tracing::trace!(?peer, "connected-only request refused: peer is not live");
                        let _ = reply.send(Err(TransportError::Unreachable(peer)));
                    }
                }
            }
            Command::Notify { peer, data } => match self.request_target(&peer) {
                Some(libp2p_peer) => {
                    tracing::debug!(peer = %libp2p_peer, bytes = data.len(), "send notification");
                    // No `pending_req` entry: nobody is waiting, and registering one would keep a
                    // sender alive for the whole request timeout for a reply that is never read.
                    self.swarm
                        .behaviour_mut()
                        .request_response
                        .send_request(&libp2p_peer, data.to_vec());
                }
                None => tracing::warn!(?peer, "notification to unknown peer"),
            },
            Command::NotifyConnected { peer, data } => {
                let live = self
                    .peers
                    .get(&peer)
                    .copied()
                    .filter(|target| self.swarm.is_connected(target));
                match live {
                    Some(libp2p_peer) => {
                        tracing::debug!(peer = %libp2p_peer, bytes = data.len(), "send connected-only notification");
                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_request(&libp2p_peer, data.to_vec());
                    }
                    None => tracing::trace!(
                        ?peer,
                        "connected-only notification refused: peer is not live"
                    ),
                }
            }
            Command::Listen(addr) => {
                // Listening on a `…/p2p-circuit` address is how a reservation is requested, so the
                // relay named in it is an infra target from here on.
                if let Some(relay) = relay_peer_in_circuit_addr(&addr) {
                    self.note_infra(relay);
                }
                if let Err(e) = self.swarm.listen_on(addr.clone()) {
                    tracing::warn!(%addr, error = %e, "listen failed");
                }
            }
            Command::Dial {
                addr,
                permit,
                reply,
            } => {
                let outcome = self.dial_gated(addr, permit);
                if let Some(reply) = reply {
                    let _ = reply.send(outcome);
                }
            }
            Command::DialPeerBatch {
                peer,
                addrs,
                permits,
                reply,
            } => {
                if !valid_direct_peer_batch(peer, &addrs)
                    || permits.as_ref().is_some_and(|p| p.len() != addrs.len())
                {
                    let _ = reply.send(Err(TransportError::InvalidDialBatch));
                    return;
                }
                let outcomes = match permits {
                    Some(permits) => addrs
                        .into_iter()
                        .zip(permits)
                        .map(|(addr, permit)| self.dial_gated(addr, Some(permit)))
                        .collect(),
                    None => addrs
                        .into_iter()
                        .map(|addr| self.dial_gated(addr, None))
                        .collect(),
                };
                let _ = reply.send(Ok(outcomes));
            }
            Command::AddExternalAddress(addr) => {
                tracing::debug!(%addr, "add external address");
                // `Swarm::add_external_address` is an assertion: it marks the address confirmed
                // immediately. The caller may have strong local evidence (a UPnP mapping or an
                // operator-configured address), so preserve that behaviour for rendezvous and
                // invites, but separately offer the same address to AutoNAT for an actual remote
                // dial-back. The upstream v2 client has no public `probe_address` method; feeding
                // the standard behaviour event is its supported candidate input.
                self.swarm.behaviour_mut().autonat_client.on_swarm_event(
                    FromSwarm::NewExternalAddrCandidate(NewExternalAddrCandidate { addr: &addr }),
                );
                self.configured_external_addrs.insert(addr.clone());
                self.swarm.add_external_address(addr);
                // A registration may have been waiting on exactly this.
                self.flush_pending_registers();
            }
            Command::RemoveExternalAddress(addr) => {
                tracing::debug!(%addr, "remove configured external address");
                if retire_configured_external_address(
                    &mut self.configured_external_addrs,
                    &self.active_port_mappings,
                    &addr,
                ) {
                    self.swarm.remove_external_address(&addr);
                    self.forget_autonat_address(&addr);
                }
            }
            Command::RendezvousRegister { namespace, rz_node } => {
                self.note_infra(rz_node);
                // Defer until we have an external address to advertise; flushed on
                // the next confirmed external address (e.g. a circuit reservation).
                self.pending_registers.push((namespace, rz_node));
                self.flush_pending_registers();
            }
            Command::RendezvousDiscover { namespace, rz_node } => {
                self.note_infra(rz_node);
                tracing::debug!(%rz_node, namespace = %namespace, "rendezvous discover");
                self.swarm.behaviour_mut().rendezvous_client.discover(
                    Some(namespace),
                    None,
                    None,
                    rz_node,
                );
            }
            Command::Evict(peer) => self.evict(peer),
            Command::Unevict(peer) => self.unevict(peer),
        }
    }

    /// Learn that `peer` is infrastructure this node uses, and make it undeniable.
    ///
    /// Lifting an eviction here is what closes an ordering window: a relay or rendezvous is
    /// dialed before it is *named* as infra (the dial comes first, the reservation or the
    /// registration second), so a hostile eviction landing in between would otherwise stick. A
    /// peer this node has decided to use as infrastructure outranks anything a removed member
    /// claimed about it.
    fn note_infra(&mut self, peer: libp2p::PeerId) {
        self.infra_peers.insert(peer);
        if self.protected.insert(peer) {
            self.lift_eviction(to_peer(&peer));
        }
    }

    /// Whether a phase-0 id names a peer this node's own configuration protects.
    ///
    /// Linear over a set that holds a handful of relays and rendezvous nodes, deliberately: a
    /// second map keyed on the phase-0 id could drift out of step with this one, and the whole
    /// point of the check is that it cannot be talked out of.
    fn is_protected(&self, peer: PeerId) -> bool {
        self.protected.iter().any(|p| to_peer(p) == peer)
    }

    /// The libp2p peer an outbound request should go to: a live connection when there is one, and
    /// otherwise the last peer we held one with. On that fallback the remembered addresses are
    /// handed to the swarm first, so `send_request` has somewhere to dial rather than failing for
    /// want of an address.
    ///
    /// An evicted peer is still refused. The deny list is enforced when the connection is
    /// established, so naming a dial target here cannot smuggle one past it.
    fn request_target(&mut self, peer: &PeerId) -> Option<libp2p::PeerId> {
        if let Some(live) = self.peers.get(peer).copied() {
            return Some(live);
        }
        let recent = self.recent_peers.get(peer)?.clone();
        tracing::debug!(
            peer = %recent.libp2p,
            addresses = recent.addresses.len(),
            "request to a peer that is not connected; offering its last known addresses for a redial"
        );
        for addr in recent.addresses {
            self.swarm.add_peer_address(recent.libp2p, addr);
        }
        Some(recent.libp2p)
    }

    /// Detach an evicted peer (P6) and keep it detached.
    ///
    /// Two halves, because refusing and severing are different operations. [`Eviction`] refuses
    /// every future connection, and it can do so from the phase-0 id alone because `to_peer` is a
    /// forward hash. `allow_block_list` closes the connections that are **already live**, which
    /// needs the `libp2p::PeerId`; that is known exactly when the peer is connected, which is the
    /// only case in which there is anything to close.
    ///
    /// Closing the connection is also what revokes anything scoped to it: a relay reservation
    /// lives on the connection it was granted over, so a node acting as a relay (rung 2's
    /// switchboards) drops the ex-member's circuit slot here rather than holding it until it
    /// expires. This node runs `relay_client` only, so today there is nothing of the sort to drop.
    fn evict(&mut self, peer: PeerId) {
        // A membership action must never be able to disconnect this node's own infrastructure.
        // The peer id an eviction names came off the wire in a removed member's self-signed
        // record, and nothing binds that value to its signer, so without this check any member
        // could name the group's relay and have everyone block it on the way out.
        if self.is_protected(peer) {
            tracing::warn!(
                ?peer,
                "refusing to evict a peer this node uses as infrastructure or bootstrap"
            );
            return;
        }
        if peer == to_peer(self.swarm.local_peer_id()) {
            tracing::warn!("refusing to evict this node itself");
            return;
        }
        let b = self.swarm.behaviour_mut();
        if let Some(displaced) = b.eviction.denied.deny(peer) {
            // The bound pushed an older eviction out; release its block so `allow_block_list`'s
            // set stays the same size as the deny list rather than growing behind it.
            b.blocked_peers.unblock_peer(displaced);
        }
        match self.peers.get(&peer).copied() {
            Some(libp2p_peer) => {
                tracing::info!(peer = %libp2p_peer, "evicting peer: closing live connections");
                let b = self.swarm.behaviour_mut();
                b.blocked_peers.block_peer(libp2p_peer);
                b.eviction.denied.note_blocked(peer, libp2p_peer);
            }
            None => tracing::info!(?peer, "evicted peer is not connected; refused from here on"),
        }
    }

    /// Lift an eviction: the peer's device is a member again (or the peer turned out to be
    /// infrastructure). Idempotent, and a no-op for a peer that was never evicted.
    fn unevict(&mut self, peer: PeerId) {
        self.lift_eviction(peer);
    }

    fn lift_eviction(&mut self, peer: PeerId) {
        let b = self.swarm.behaviour_mut();
        if let Some(blocked) = b.eviction.denied.allow(&peer) {
            b.blocked_peers.unblock_peer(blocked);
        }
    }

    /// Dial `addr`, gated on **(peer, address)** rather than on peer. Second half of P11.
    ///
    /// `Swarm::dial(Multiaddr)` builds `DialOpts` with `peer_id: None` and
    /// `PeerCondition::Always`, so an existing connection never suppressed a new dial. Combined
    /// with an unjittered per-server discovery timer that dials plus registers plus discovers for
    /// every namespace in the grandfather window, every member of every group reconverged on the
    /// same infra node inside one timer period after an outage: a thundering herd aimed at
    /// precisely the node that just came back.
    ///
    /// The first fix for that gated on the **peer**, and gated too hard. Two things broke:
    ///
    /// - a peer first reached over a relay circuit was never dialed directly when its direct
    ///   address arrived later, because a connection to it already existed. The pair stayed pinned
    ///   to the relay, which is the opposite of what the ladder wants;
    /// - the sync layer issues one `Command::Dial` per address, so for a peer with N addresses the
    ///   first was dialed and addresses 2..N returned `DialPeerConditionFalse` and were
    ///   **discarded**. One stale cached address at the front of the list made a peer unreachable
    ///   for a whole discovery tick.
    ///
    /// So the condition is now split by target:
    ///
    /// - **Infra targets** (rendezvous nodes and relays, learned from the commands that name them)
    ///   keep the strict condition. That is where the herd is, one connection is all anyone needs,
    ///   and they are addressed by a single well-known address anyway.
    /// - **Member peers** are batched: a peer's addresses accumulate over one drain of the command
    ///   queue and go to libp2p as one `DialOpts::peer_id(p).addresses(all)`, which races them
    ///   itself and keeps one connection. A (peer, address) already in flight is suppressed; a new
    ///   address for an already-connected peer is not, which is what lets a direct address
    ///   preempt a relayed connection.
    ///
    /// An address with no `/p2p/<id>` names no authenticated target and is refused. Discovery and
    /// PEX validate the terminal id against the signed record before this point; retaining a bare
    /// fallback here would silently turn a malformed record back into an arbitrary socket dial.
    /// (The jitter half of P11 lives in the caller that drives the timer.)
    fn dial_gated(
        &mut self,
        addr: Multiaddr,
        permit: Option<catcoms_rt::BoxedDialPermit>,
    ) -> catcoms_rt::DialSubmission {
        // A permit is authority for one canonical endpoint, not merely one peer. Check the owned
        // command data before any actor ledger mutation; a mismatched permit drops and refunds.
        if permit
            .as_ref()
            .is_some_and(|permit| permit.address() != addr.to_string())
        {
            tracing::warn!("dial refused: scheduler permit/address mismatch");
            return catcoms_rt::DialSubmission::Suppressed;
        }
        // Routing *through* a relay makes it this node's infrastructure just as surely as
        // reserving on it does, and only the reservation path was noting it. The gate below
        // resolves the LAST `/p2p/` component, which for `…/p2p/RELAY/p2p-circuit/p2p/TARGET` is
        // the target, so a relay used purely as transit was never protected: a companion device
        // claiming the relay's transport id (no *device* legitimately claims one, so both sync
        // checks pass) and then being removed in an ordinary "drop my old phone" would have had
        // every member evict the relay it routes through.
        if let Some(relay) = relay_peer_in_circuit_addr(&addr) {
            self.note_infra(relay);
        }
        let Some(target) = target_peer_in_multiaddr(&addr) else {
            let shape = dial_log_shape(std::iter::once(&addr));
            tracing::warn!(?shape, "dial refused: address has no terminal peer id");
            return catcoms_rt::DialSubmission::Suppressed;
        };
        if self.infra_peers.contains(&target) {
            return self.dial_infra(target, addr, permit);
        }
        if self.covered_addrs.len() > MAX_DIAL_LEDGER_PEERS {
            // Entries are dropped when a peer disconnects or a dial fails, so this is the
            // pathological case rather than the normal one. Clearing wholesale costs at worst a
            // duplicate dial attempt.
            tracing::debug!("dial ledger over its cap; clearing");
            self.covered_addrs.clear();
        }
        if self
            .covered_addrs
            .get(&target)
            .is_some_and(|covered| covered.contains(&addr))
        {
            let shape = dial_log_shape(std::iter::once(&addr));
            tracing::trace!(?shape, peer = %target, "dial suppressed: route is already dialing or connected");
            return catcoms_rt::DialSubmission::Suppressed;
        }
        if !commit_dial_permit(permit, &addr) {
            tracing::trace!(peer = %target, "dial suppressed: scheduler permit expired");
            return catcoms_rt::DialSubmission::Suppressed;
        }
        self.covered_addrs
            .entry(target)
            .or_default()
            .insert(addr.clone());
        self.pending_dials.entry(target).or_default().push(addr);
        catcoms_rt::DialSubmission::Submitted
    }

    /// Dial an infra target under the strict one-connection-per-peer condition.
    fn dial_infra(
        &mut self,
        target: libp2p::PeerId,
        addr: Multiaddr,
        permit: Option<catcoms_rt::BoxedDialPermit>,
    ) -> catcoms_rt::DialSubmission {
        if self.swarm.is_connected(&target) {
            let shape = dial_log_shape(std::iter::once(&addr));
            tracing::trace!(?shape, peer = %target, "infra dial suppressed: already connected");
            return catcoms_rt::DialSubmission::Suppressed;
        }
        if !commit_dial_permit(permit, &addr) {
            tracing::trace!(peer = %target, "infra dial suppressed: scheduler permit expired");
            return catcoms_rt::DialSubmission::Suppressed;
        }
        let shape = dial_log_shape(std::iter::once(&addr));
        let opts = DialOpts::peer_id(target)
            .addresses(vec![addr])
            .condition(PeerCondition::DisconnectedAndNotDialing)
            .build();
        match self.swarm.dial(opts) {
            Ok(()) => {
                tracing::debug!(?shape, peer = %target, "dialing infra node");
                return catcoms_rt::DialSubmission::Submitted;
            }
            // `DialError::DialPeerConditionFalse` is the condition doing its job (a dial to this
            // peer is already in flight), not a failure worth warning about.
            Err(libp2p::swarm::DialError::DialPeerConditionFalse(cond)) => {
                tracing::trace!(?shape, peer = %target, ?cond, "infra dial suppressed: already dialing");
            }
            Err(e) => {
                // Drop the ledger entry so the next tick retries. A dial refused *before* it
                // starts (`max_pending_outgoing` at its cap, say) produces no
                // `OutgoingConnectionError`, so nothing else would ever clear it and the node
                // would stop dialing its own relay or rendezvous for the rest of the process.
                // `flush_dials` already does this for member peers.
                tracing::warn!(?shape, error_kind = dial_error_kind(&e), "dial failed");
                self.release_failed_dial(Some(target), &e);
            }
        }
        catcoms_rt::DialSubmission::Suppressed
    }

    /// Issue one racing dial per member peer whose addresses accumulated during this drain.
    ///
    /// `PeerCondition::Always` is correct here precisely because the gate above already decided:
    /// every address in the batch is one libp2p is not currently trying, and a peer we are already
    /// connected to may still be worth dialing at a *new* address (the relay-to-direct upgrade).
    fn flush_dials(&mut self) {
        for (peer, addrs) in std::mem::take(&mut self.pending_dials) {
            let count = addrs.len();
            // Family/transport counts preserve the useful diagnosis without copying a member's
            // private coordinates into the always-on diagnostic ring or optional file log.
            let shape = dial_log_shape(&addrs);
            let opts = DialOpts::peer_id(peer)
                .addresses(addrs)
                .condition(PeerCondition::Always)
                .build();
            match self.swarm.dial(opts) {
                Ok(()) => tracing::debug!(peer = %peer, addresses = count, ?shape, "dialing"),
                Err(e) => {
                    tracing::warn!(peer = %peer, error_kind = dial_error_kind(&e), ?shape, "dial failed");
                    self.covered_addrs.remove(&peer);
                }
            }
        }
    }

    /// Forget every address recorded for a peer, so a later tick may dial it again. Called when
    /// the peer goes away entirely; while it is reachable, its covered addresses are what stop a
    /// discovery tick reopening the same connection over and over.
    fn clear_dial_ledger(&mut self, peer: &libp2p::PeerId) {
        self.covered_addrs.remove(peer);
    }

    /// Record the address a connection actually came up on, so a repeat dial of it is suppressed.
    ///
    /// The dialed multiaddr usually carries a trailing `/p2p/<id>` that the established endpoint
    /// address does not, so both forms are recorded: the dialed one by [`Actor::dial_gated`] and
    /// the observed one here.
    fn cover_addr(&mut self, peer: libp2p::PeerId, addr: Multiaddr) {
        self.covered_addrs.entry(peer).or_default().insert(addr);
    }

    /// Release the addresses a failed dial had reserved, so the next tick may retry them.
    ///
    /// A transport-level failure names the addresses that failed, and only those are released: the
    /// other addresses of a peer that is up over one of them must stay covered. Any other dial
    /// error is about the peer as a whole, so the whole entry goes.
    fn release_failed_dial(
        &mut self,
        peer: Option<libp2p::PeerId>,
        error: &libp2p::swarm::DialError,
    ) {
        let Some(peer) = peer else { return };
        match error {
            libp2p::swarm::DialError::Transport(failed) => {
                if let Some(set) = self.covered_addrs.get_mut(&peer) {
                    for (addr, _) in failed {
                        set.remove(addr);
                        // The dialed form carries `/p2p/<id>`; the reported one may not.
                        set.remove(&addr.clone().with(Protocol::P2p(peer)));
                    }
                    if set.is_empty() {
                        self.covered_addrs.remove(&peer);
                    }
                }
            }
            _ => {
                self.covered_addrs.remove(&peer);
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
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                tracing::info!(%address, "listening");
                self.active_listeners.insert(address.clone());
                let mut addresses: Vec<_> = self.active_listeners.iter().cloned().collect();
                addresses.sort_by_key(ToString::to_string);
                self.listener_snapshot_tx
                    .send_replace(ListenerSnapshot { addresses });
                if self.enable_port_mapping {
                    self.reconcile_port_mappers();
                }
                // A granted relay-circuit reservation is how a NAT'd node becomes
                // reachable; confirm it as an external address so rendezvous
                // registrations advertise it, then flush any deferred registrations.
                if is_relayed(&address) {
                    self.swarm.add_external_address(address.clone());
                    self.relay_addresses.insert(address.clone());
                    let mut addresses: Vec<_> = self.relay_addresses.iter().cloned().collect();
                    addresses.sort_by_key(ToString::to_string);
                    self.relay_address_tx
                        .send_replace(RelayAddressSnapshot { addresses });
                    self.flush_pending_registers();
                }
                let _ = self.listen_tx.try_send(address);
            }
            SwarmEvent::ExpiredListenAddr { address, .. } => {
                if self.active_listeners.remove(&address) {
                    let mut addresses: Vec<_> = self.active_listeners.iter().cloned().collect();
                    addresses.sort_by_key(ToString::to_string);
                    self.listener_snapshot_tx
                        .send_replace(ListenerSnapshot { addresses });
                    if self.enable_port_mapping {
                        self.reconcile_port_mappers();
                    }
                }
                if self.relay_addresses.remove(&address) {
                    self.swarm.remove_external_address(&address);
                    self.forget_autonat_address(&address);
                    let mut addresses: Vec<_> = self.relay_addresses.iter().cloned().collect();
                    addresses.sort_by_key(ToString::to_string);
                    self.relay_address_tx
                        .send_replace(RelayAddressSnapshot { addresses });
                }
            }
            SwarmEvent::ExternalAddrConfirmed { address } => {
                let has_ip_literal = address
                    .iter()
                    .any(|protocol| matches!(protocol, Protocol::Ip4(_) | Protocol::Ip6(_)));
                if has_ip_literal
                    && !external_address_is_allowed(&self.configured_external_addrs, &address)
                {
                    // UPnP confirms its address in the Swarm before its public behaviour event.
                    // Enforce the canonical classifier here, before any pending rendezvous
                    // registration can publish a multicast/reserved/non-public gateway result.
                    self.swarm.remove_external_address(&address);
                    self.forget_autonat_address(&address);
                } else {
                    self.flush_pending_registers();
                }
            }
            SwarmEvent::ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                ..
            } => {
                let relayed = endpoint.is_relayed();
                let path = classify_connection_path(&endpoint);
                tracing::debug!(peer = %peer_id, relayed, "connection established");
                // Record the address this came up on. Repeating a dial of an address already
                // carrying a connection is the thundering herd P11 describes, and is suppressed;
                // a *new* address for the same peer is not, which is how a direct address preempts
                // a relayed connection instead of the pair staying pinned to the relay.
                self.cover_addr(peer_id, endpoint.get_remote_address().clone());
                let peer = to_peer(&peer_id);
                // Race closer. `Eviction` refuses *new* connections, but a connection that
                // passed the gate before the deny was installed is already established, and
                // whether this arm or the command channel runs first is `select!`'s random
                // choice. When the command won, `Actor::evict` looked in `peers`, found nothing
                // and never asked `allow_block_list` to close anything, leaving a live
                // connection that neither half would ever tear down. Closing it here costs one
                // set lookup per connection and keeps both sets consistent.
                //
                // Deliberately NOT an early return: the peer is still recorded and still
                // announced, so the close below produces a matched `PeerDisconnected` rather
                // than an unpaired one. This is a race closer, never the enforcement.
                if self.swarm.behaviour().eviction.denied.is_denied(&peer) {
                    tracing::info!(peer = %peer_id, "evicted peer raced the deny; closing");
                    let b = self.swarm.behaviour_mut();
                    b.blocked_peers.block_peer(peer_id);
                    b.eviction.denied.note_blocked(peer, peer_id);
                }
                // Only surface `PeerConnected` on the *first* connection to a peer;
                // a DCUtR upgrade opens a second (direct) connection to a peer we
                // already know, and must not look like a new peer to layers above.
                // Kept past the disconnect that follows, so a later request can redial rather than
                // fail for want of a dial target.
                record_recent_peer(
                    &mut self.recent_peers,
                    &mut self.recent_peer_order,
                    peer,
                    peer_id,
                    endpoint.get_remote_address().clone(),
                );
                let first_connection = self.peers.insert(peer, peer_id).is_none();
                self.connection_paths.insert((peer_id, connection_id), path);
                if let Some(route) = authenticated_dial_route(peer_id, &endpoint) {
                    self.authenticated_routes
                        .insert((peer_id, connection_id), route.clone());
                    record_authenticated_route_evidence(
                        &mut self.authenticated_route_evidence,
                        route,
                    );
                    self.authenticated_route_evidence_tx
                        .send_replace(self.authenticated_route_evidence.iter().cloned().collect());
                }
                // Update the queryable state atomically before sending either ordered edge. A
                // waiter that wakes here sees both aggregate liveness and its current path.
                self.publish_connection_snapshot();
                if first_connection {
                    let _ = self
                        .event_tx
                        .send(TransportEvent::PeerConnected(peer))
                        .await;
                }
                // Preserve `PeerConnected` as the first event for a peer so existing consumers
                // keep their one-edge liveness contract. The path snapshot follows immediately;
                // consumers that understand it may refine "connected" to direct vs relay.
                let paths = self.peer_paths_event(peer_id, Some(path));
                let _ = self.event_tx.send(paths).await;
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                // Say so. Every failed dial used to be silent: the only trace an unreachable peer
                // left in a log was quinn's raw `sendmsg` warning for the UDP case, and nothing at
                // all for TCP, so "this node is dialing and failing" and "this node is not dialing"
                // read identically. A `Transport` error names each address it tried, which is the
                // part worth having; the rest carry their reason in the error itself.
                match &error {
                    libp2p::swarm::DialError::Transport(failed) => {
                        for (addr, _cause) in failed {
                            let shape = dial_log_shape(std::iter::once(addr));
                            tracing::warn!(peer = ?peer_id, ?shape, error_kind = "transport", "dial failed");
                        }
                    }
                    other => tracing::warn!(
                        peer = ?peer_id,
                        error_kind = dial_error_kind(other),
                        "dial failed"
                    ),
                }
                // The attempt is over, so a later tick may retry rather than being suppressed by a
                // ledger entry that nothing would ever clear.
                self.release_failed_dial(peer_id, &error);
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                connection_id,
                num_established,
                ..
            } => {
                let previous_paths = active_connection_paths(&self.connection_paths, peer_id);
                forget_and_publish_authenticated_routes(
                    &mut self.authenticated_routes,
                    &self.authenticated_route_tx,
                    peer_id,
                    connection_id,
                    num_established,
                );
                forget_connection_path(
                    &mut self.connection_paths,
                    peer_id,
                    connection_id,
                    num_established,
                );
                let paths_changed =
                    previous_paths != active_connection_paths(&self.connection_paths, peer_id);
                if num_established == 0 {
                    tracing::debug!(peer = %peer_id, "peer disconnected");
                    // Nothing covers this peer any more, so the next tick is free to dial it.
                    self.clear_dial_ledger(&peer_id);
                    let peer = to_peer(&peer_id);
                    self.peers.remove(&peer);
                    self.publish_connection_snapshot();
                    if self.mesh_observations.remove(&peer_id).is_some() {
                        self.mesh_observation_order
                            .retain(|candidate| candidate != &peer_id);
                        self.publish_mesh_observations();
                    }
                    // Always paired: an evicted peer is refused by `Eviction` before a connection
                    // is ever established, so it cannot reach this arm without having been
                    // announced as connected first.
                    let _ = self
                        .event_tx
                        .send(TransportEvent::PeerDisconnected(peer))
                        .await;
                    // `num_established` is libp2p's authoritative count. Clear every entry for
                    // the peer as a fail-safe if an earlier close edge was missed or changed
                    // shape, then publish the empty snapshot after the legacy disconnect edge.
                    if paths_changed {
                        let paths = self.peer_paths_event(peer_id, None);
                        let _ = self.event_tx.send(paths).await;
                    }
                } else if paths_changed {
                    // The address-bearing watch was already published independently above. The
                    // coarse snapshot/event need move only when their deduplicated set changed.
                    self.publish_connection_snapshot();
                    let paths = self.peer_paths_event(peer_id, None);
                    let _ = self.event_tx.send(paths).await;
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
                request_response::Event::OutboundFailure {
                    request_id,
                    peer,
                    error,
                    ..
                },
            )) => {
                // Log the reason. Every outbound failure used to be reported as "transport closed",
                // which reads as a local shutdown; a dial that never landed, a peer that went away
                // mid-request and a request that simply timed out are three different problems and
                // a log that spells them all the same way sends you looking in the wrong place.
                tracing::warn!(peer = %peer, error = %error, "outbound request failed");
                if let Some(reply) = self.pending_req.remove(&request_id) {
                    let _ = reply.send(Err(match error {
                        request_response::OutboundFailure::Timeout => {
                            TransportError::Timeout(to_peer(&peer))
                        }
                        request_response::OutboundFailure::DialFailure
                        | request_response::OutboundFailure::UnsupportedProtocols => {
                            TransportError::Unreachable(to_peer(&peer))
                        }
                        // The remote had the request and the connection died before its answer:
                        // "no response" is the honest description, and unlike `Closed` it does not
                        // claim this node's own transport went down.
                        _ => TransportError::NoResponse,
                    }));
                }
            }
            // DCUtR hole-punch result. On success the relayed link has been upgraded
            // to a direct connection (a fresh `ConnectionEstablished` to the same
            // peer); surface it for diagnostics. On failure the connection stays
            // relayed; still fully functional, just routed through the relay.
            SwarmEvent::Behaviour(MeshBehaviourEvent::Dcutr(dcutr::Event {
                remote_peer_id,
                result,
            })) => match result {
                Ok(conn) => {
                    tracing::info!(peer = %remote_peer_id, ?conn, "DCUtR hole-punch succeeded; connection upgraded to direct");
                    let _ = self.upgrade_tx.try_send(to_peer(&remote_peer_id));
                }
                Err(e) => {
                    tracing::debug!(peer = %remote_peer_id, error = %e, "DCUtR hole-punch failed; staying relayed");
                }
            },
            // Identify's `observed_addr` is useful NAT telemetry but is not a listener address:
            // TCP source ports are usually ephemeral and the reporting peer is untrusted. Keep a
            // bounded, connected-peer snapshot for the assistant and never feed it into Swarm.
            SwarmEvent::Behaviour(MeshBehaviourEvent::Identify(identify::Event::Received {
                peer_id,
                info,
                ..
            })) => {
                record_mesh_observation(
                    &mut self.mesh_observations,
                    &mut self.mesh_observation_order,
                    peer_id,
                    info.observed_addr.clone(),
                );
                self.publish_mesh_observations();
                tracing::trace!(peer = %peer_id, observed = %info.observed_addr, "identify observation recorded");
            }
            SwarmEvent::Behaviour(MeshBehaviourEvent::Identify(e)) => {
                tracing::trace!(?e, "identify event");
            }
            // AutoNAT v2 only reports success after this process receives a fresh callback
            // carrying the nonce it generated. That makes a positive result materially stronger
            // than a server merely claiming it dialled us. Failures remain address- and
            // observer-specific; they are surfaced without turning them into a node-wide state.
            SwarmEvent::Behaviour(MeshBehaviourEvent::AutonatClient(e)) => {
                if !self.autonat_address_is_current(&e.tested_addr) {
                    // Ignore callbacks delivered during the unowned interval after expiry. If the
                    // same socket is later reacquired it becomes eligible again: a callback that
                    // succeeds then really did reach the newly-current route at that moment.
                    tracing::debug!(
                        address = %e.tested_addr,
                        server = %e.server,
                        "ignoring AutoNAT result for an address with no current owner"
                    );
                } else {
                    let reachable = e.result.is_ok();
                    let error = e.result.err().map(|err| err.to_string());
                    if reachable {
                        tracing::info!(
                            address = %e.tested_addr,
                            server = %e.server,
                            "AutoNAT dial-back succeeded"
                        );
                    } else {
                        tracing::info!(
                            address = %e.tested_addr,
                            server = %e.server,
                            error = error.as_deref().unwrap_or("unknown"),
                            "AutoNAT dial-back failed"
                        );
                    }
                    let result = AutoNatResult {
                        address: e.tested_addr,
                        server: e.server,
                        reachable,
                        error,
                    };
                    record_autonat_result(
                        &mut self.autonat_results,
                        &mut self.autonat_order,
                        result.clone(),
                    );
                    self.publish_autonat_snapshot(Some(result));
                }
            }
            // Relay-client lifecycle (reservation accepted/expired, circuit opened).
            SwarmEvent::Behaviour(MeshBehaviourEvent::RelayClient(e)) => {
                tracing::debug!(?e, "relay-client event");
            }
            // libp2p's behaviour is UPnP IGD only (PCP/NAT-PMP are driven by the workers above).
            // Fold both implementations through one event path so candidates, diagnostics and
            // expiry all have identical semantics.
            SwarmEvent::Behaviour(MeshBehaviourEvent::Upnp(e)) => match e {
                libp2p::upnp::Event::NewExternalAddr(addr) => {
                    if let Some((transport, _)) = port_mapping_transport(&addr) {
                        self.on_port_mapping_event(PortMappingEvent::Mapped {
                            mechanism: PortMappingMechanism::Upnp,
                            transport,
                            local_address: None,
                            address: addr,
                        });
                    } else {
                        tracing::warn!(%addr, "UPnP returned an address with no TCP or QUIC port");
                    }
                }
                libp2p::upnp::Event::ExpiredExternalAddr(addr) => {
                    if let Some((transport, _)) = port_mapping_transport(&addr) {
                        self.on_port_mapping_event(PortMappingEvent::Expired {
                            mechanism: PortMappingMechanism::Upnp,
                            transport,
                            local_address: None,
                            address: addr,
                        });
                    } else {
                        self.swarm.remove_external_address(&addr);
                        self.forget_autonat_address(&addr);
                    }
                }
                libp2p::upnp::Event::GatewayNotFound => {
                    for transport in [PortMappingTransport::Tcp, PortMappingTransport::Udp] {
                        self.on_port_mapping_event(PortMappingEvent::Unavailable {
                            mechanism: PortMappingMechanism::Upnp,
                            transport,
                            local_address: None,
                            detail: "no UPnP IGD gateway answered discovery".to_string(),
                        });
                    }
                }
                libp2p::upnp::Event::NonRoutableGateway => {
                    for transport in [PortMappingTransport::Tcp, PortMappingTransport::Udp] {
                        self.on_port_mapping_event(PortMappingEvent::Unavailable {
                            mechanism: PortMappingMechanism::Upnp,
                            transport,
                            local_address: None,
                            detail:
                                "UPnP gateway is not internet-routable (likely CGNAT or double NAT)"
                                    .to_string(),
                        });
                    }
                }
            },
            // Rendezvous client: surface discovered records (NEVER auto-dial them) and
            // our own registrations; log failures/expiry for the caller to react to.
            SwarmEvent::Behaviour(MeshBehaviourEvent::RendezvousClient(e)) => match e {
                rendezvous::client::Event::Discovered { registrations, .. } => {
                    // Cap records ingested per Discover response so a hostile rendezvous
                    // cannot flood the (unbounded, never-dropping) discovered queue; it
                    // sits upstream of the DiscoveryPolicy's dial budget, so that budget
                    // alone does not bound it. The dropped tail is logged.
                    let total = registrations.len();
                    for reg in registrations.into_iter().take(MAX_DISCOVERED_PER_RESPONSE) {
                        let _ = self.discovered_tx.send(Discovered {
                            peer: reg.record.peer_id(),
                            addresses: reg.record.addresses().to_vec(),
                            namespace: reg.namespace.to_string(),
                            seq: reg.record.seq(),
                        });
                    }
                    if total > MAX_DISCOVERED_PER_RESPONSE {
                        tracing::warn!(
                            total,
                            cap = MAX_DISCOVERED_PER_RESPONSE,
                            "rendezvous Discover response capped; dropped the surplus records"
                        );
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

/// Final actor-side guard for reciprocal direct dialing. Validation lives beside command
/// handling so every caller, including a compromised higher layer, must provide one or two
/// direct multiaddrs whose terminal transport identity is exactly the addressed Phase-0 peer.
fn valid_direct_peer_batch(peer: PeerId, addrs: &[Multiaddr]) -> bool {
    !addrs.is_empty()
        && addrs.len() <= MAX_PEER_DIAL_BATCH
        && addrs.iter().all(|addr| {
            !is_relayed(addr)
                && target_peer_in_multiaddr(addr).is_some_and(|target| to_peer(&target) == peer)
        })
}

// ----- handle ----------------------------------------------------------------

/// Clone one watch value only while its actor is still alive. Tokio deliberately retains the last
/// value after the final sender drops; accepting that value would turn an actor crash into stale
/// "connected" evidence during a handoff.
fn current_connection_snapshot(
    snapshots: &mut watch::Receiver<Vec<PeerConnectionSnapshot>>,
) -> Result<Vec<PeerConnectionSnapshot>, TransportError> {
    loop {
        let current = snapshots.borrow_and_update().clone();
        match snapshots.has_changed() {
            Ok(false) => return Ok(current),
            // A replacement landed between the clone and the check. Re-read it rather than
            // returning the stale connected row (most importantly, after a disconnect).
            Ok(true) => continue,
            Err(_) => return Err(TransportError::Closed),
        }
    }
}

/// Address-bearing counterpart to [`current_connection_snapshot`]. Reject a retained watch value
/// after actor shutdown so a caller cannot persist a route that is no longer present-time proof.
fn current_authenticated_routes(
    snapshots: &mut watch::Receiver<Vec<AuthenticatedDialRoute>>,
) -> Result<Vec<AuthenticatedDialRoute>, TransportError> {
    loop {
        let current = snapshots.borrow_and_update().clone();
        match snapshots.has_changed() {
            Ok(false) => return Ok(current),
            Ok(true) => continue,
            Err(_) => return Err(TransportError::Closed),
        }
    }
}

/// A handle to a running libp2p mesh node, implementing [`MeshTransport`].
#[derive(Debug)]
pub struct MeshService {
    local: PeerId,
    cmd_tx: mpsc::Sender<Command>,
    event_rx: Mutex<mpsc::Receiver<TransportEvent>>,
    connection_snapshot_rx: watch::Receiver<Vec<PeerConnectionSnapshot>>,
    authenticated_route_rx: watch::Receiver<Vec<AuthenticatedDialRoute>>,
    authenticated_route_evidence_rx: watch::Receiver<Vec<AuthenticatedDialRoute>>,
    listen_rx: Mutex<mpsc::Receiver<Multiaddr>>,
    listener_snapshot_rx: Mutex<watch::Receiver<ListenerSnapshot>>,
    /// `None` once the desktop has taken the coalesced router-mapping state.
    port_mapping_rx: Mutex<Option<watch::Receiver<PortMappingSnapshot>>>,
    /// `None` once the desktop has taken the AutoNAT evidence snapshot for its background
    /// reachability collector.
    autonat_rx: Mutex<Option<watch::Receiver<AutoNatSnapshot>>>,
    /// `None` once the desktop has taken the live relay-circuit address set.
    relay_address_rx: Mutex<Option<watch::Receiver<RelayAddressSnapshot>>>,
    /// `None` once a product collector has taken the connected-peer observation snapshot.
    mesh_observation_rx: Mutex<Option<watch::Receiver<MeshObservationSnapshot>>>,
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
        Self::spawn_protecting(swarm, Vec::new(), false)
    }

    /// [`MeshService::spawn`], plus the peers this node was **constructed to dial** (its
    /// bootstrap/infra set). Those are never deniable by an eviction: see `Actor::protected`.
    fn spawn_protecting(
        swarm: Swarm<MeshBehaviour>,
        protected: Vec<libp2p::PeerId>,
        enable_port_mapping: bool,
    ) -> Self {
        let local = to_peer(swarm.local_peer_id());
        let (cmd_tx, cmd_rx) = mpsc::channel(256);
        let (event_tx, event_rx) = mpsc::channel(256);
        let (connection_snapshot_tx, connection_snapshot_rx) = watch::channel(Vec::new());
        let (authenticated_route_tx, authenticated_route_rx) = watch::channel(Vec::new());
        let (authenticated_route_evidence_tx, authenticated_route_evidence_rx) =
            watch::channel(Vec::new());
        let (listen_tx, listen_rx) = mpsc::channel(16);
        let (listener_snapshot_tx, listener_snapshot_rx) =
            watch::channel(ListenerSnapshot::default());
        let (port_mapping_tx, port_mapping_rx) = watch::channel(PortMappingSnapshot::default());
        let (port_mapper_tx, port_mapper_rx) = mpsc::channel(32);
        let (autonat_tx, autonat_rx) = watch::channel(AutoNatSnapshot::default());
        let (relay_address_tx, relay_address_rx) = watch::channel(RelayAddressSnapshot::default());
        let (mesh_observation_tx, mesh_observation_rx) =
            watch::channel(MeshObservationSnapshot::default());
        let (upgrade_tx, upgrade_rx) = mpsc::channel(16);
        let (discovered_tx, discovered_rx) = mpsc::unbounded_channel();
        let (registered_tx, registered_rx) = mpsc::unbounded_channel();
        let actor = Actor {
            swarm,
            enable_port_mapping,
            cmd_rx,
            event_tx,
            connection_snapshot_tx,
            authenticated_route_tx,
            authenticated_route_evidence_tx,
            listen_tx,
            active_listeners: HashSet::new(),
            listener_snapshot_tx,
            port_mapping_tx,
            port_mapper_tx,
            port_mapper_rx,
            port_mapper_tasks: HashMap::new(),
            next_port_mapper_generation: 1,
            active_port_mappings: HashMap::new(),
            port_mapping_unavailable: HashMap::new(),
            configured_external_addrs: HashSet::new(),
            autonat_tx,
            autonat_results: HashMap::new(),
            autonat_order: VecDeque::new(),
            relay_addresses: HashSet::new(),
            relay_address_tx,
            mesh_observations: HashMap::new(),
            mesh_observation_order: VecDeque::new(),
            mesh_observation_tx,
            upgrade_tx,
            discovered_tx,
            registered_tx,
            pending_registers: Vec::new(),
            peers: HashMap::new(),
            recent_peers: HashMap::new(),
            recent_peer_order: VecDeque::new(),
            pending_req: HashMap::new(),
            pending_publish: Vec::new(),
            infra_peers: HashSet::new(),
            covered_addrs: HashMap::new(),
            pending_dials: HashMap::new(),
            connection_paths: HashMap::new(),
            authenticated_routes: HashMap::new(),
            authenticated_route_evidence: VecDeque::new(),
            protected: protected.into_iter().collect(),
        };
        tokio::spawn(actor.run());
        Self {
            local,
            cmd_tx,
            event_rx: Mutex::new(event_rx),
            connection_snapshot_rx,
            authenticated_route_rx,
            authenticated_route_evidence_rx,
            listen_rx: Mutex::new(listen_rx),
            listener_snapshot_rx: Mutex::new(listener_snapshot_rx),
            port_mapping_rx: Mutex::new(Some(port_mapping_rx)),
            autonat_rx: Mutex::new(Some(autonat_rx)),
            relay_address_rx: Mutex::new(Some(relay_address_rx)),
            mesh_observation_rx: Mutex::new(Some(mesh_observation_rx)),
            upgrade_rx: Mutex::new(upgrade_rx),
            discovered_rx: Mutex::new(discovered_rx),
            registered_rx: Mutex::new(registered_rx),
        }
    }

    /// Read the current connected-peer/path table without consuming the ordered transport event
    /// stream. The actor publishes an empty replacement on disconnect, so this is present-time
    /// state rather than historical evidence.
    pub fn connection_snapshot(&self) -> Vec<PeerConnectionSnapshot> {
        let mut snapshots = self.connection_snapshot_rx.clone();
        current_connection_snapshot(&mut snapshots).unwrap_or_default()
    }

    /// Wait until one of `wanted` is currently connected without stealing lifecycle, request, or
    /// gossip events from the single consumer that will own this transport afterward.
    ///
    /// The caller supplies the deadline through its injected/runtime timeout policy. Returning
    /// [`TransportError::Closed`] distinguishes actor shutdown from an ordinary timeout.
    pub async fn wait_for_any_connected(
        &self,
        wanted: &[PeerId],
    ) -> Result<PeerConnectionSnapshot, TransportError> {
        let mut snapshots = self.connection_snapshot_rx.clone();
        loop {
            let current = current_connection_snapshot(&mut snapshots)?;
            let connected = current
                .into_iter()
                .find(|entry| wanted.contains(&entry.peer));
            if let Some(connected) = connected {
                return Ok(connected);
            }
            snapshots
                .changed()
                .await
                .map_err(|_| TransportError::Closed)?;
        }
    }

    /// Convenience form of [`MeshService::wait_for_any_connected`] for one peer.
    pub async fn wait_for_peer_connected(
        &self,
        wanted: PeerId,
    ) -> Result<PeerConnectionSnapshot, TransportError> {
        self.wait_for_any_connected(std::slice::from_ref(&wanted))
            .await
    }

    /// Await the next bound listen address (e.g. to learn the real port when
    /// listening on `/ip4/127.0.0.1/tcp/0`, or the circuit address once a relay
    /// reservation is granted). Returns `None` once the actor stops.
    pub async fn next_listen_addr(&self) -> Option<Multiaddr> {
        self.listen_rx.lock().await.recv().await
    }

    /// Await the next authoritative live-listener snapshot. The receiver retains a change that
    /// happened before this call, closing the startup race in UI diagnostics.
    pub async fn next_listener_snapshot(&self) -> Option<ListenerSnapshot> {
        let mut rx = self.listener_snapshot_rx.lock().await;
        rx.changed().await.ok()?;
        let snapshot = rx.borrow_and_update().clone();
        Some(snapshot)
    }

    /// Await the next coalesced UPnP/PCP/NAT-PMP state snapshot.
    pub async fn next_port_mapping_snapshot(&self) -> Option<PortMappingSnapshot> {
        let mut guard = self.port_mapping_rx.lock().await;
        let rx = guard.as_mut()?;
        rx.changed().await.ok()?;
        let snapshot = rx.borrow_and_update().clone();
        Some(snapshot)
    }

    /// Take the single-consumer coalesced mapping state for a background collector. The current
    /// value is retained even when it changed before the collector started.
    pub async fn take_port_mapping_snapshots(
        &self,
    ) -> Option<watch::Receiver<PortMappingSnapshot>> {
        self.port_mapping_rx.lock().await.take()
    }

    /// Await the next coalesced AutoNAT v2 evidence snapshot.
    ///
    /// A success proves only that the named server reached that address at that moment. A failure
    /// is likewise scoped to one address/server pair; callers should keep listening because a
    /// second candidate (IPv6 rather than IPv4, or QUIC rather than TCP) may still succeed.
    pub async fn next_autonat_snapshot(&self) -> Option<AutoNatSnapshot> {
        let mut guard = self.autonat_rx.lock().await;
        let rx = guard.as_mut()?;
        rx.changed().await.ok()?;
        let snapshot = rx.borrow_and_update().clone();
        Some(snapshot)
    }

    /// Take the single-consumer coalesced AutoNAT evidence for a background collector.
    pub async fn take_autonat_snapshots(&self) -> Option<watch::Receiver<AutoNatSnapshot>> {
        self.autonat_rx.lock().await.take()
    }

    /// Take the current relay-circuit address set. An empty later snapshot means the reservation
    /// listener expired and callers must withdraw the circuit from invites/peer records.
    pub async fn take_relay_address_snapshots(
        &self,
    ) -> Option<watch::Receiver<RelayAddressSnapshot>> {
        self.relay_address_rx.lock().await.take()
    }

    /// Take the current connected-peer Identify observations. These values are diagnostics only:
    /// consumers must not add them to invites, external addresses, rendezvous records, or dials.
    pub async fn take_mesh_observation_snapshots(
        &self,
    ) -> Option<watch::Receiver<MeshObservationSnapshot>> {
        self.mesh_observation_rx.lock().await.take()
    }

    /// Await the next peer whose relayed connection DCUtR **upgraded to a direct
    /// one** (NAT hole-punch success). Diagnostics/observability only; the upgrade
    /// is transparent to the layers above (the peer stays the same `PeerId`, traffic
    /// just moves off the relay). Returns `None` once the actor stops.
    pub async fn next_direct_upgrade(&self) -> Option<PeerId> {
        self.upgrade_rx.lock().await.recv().await
    }

    /// Register our (signed) peer record under `namespace` at the rendezvous node
    /// `rz_node`; we must already be connected to it (e.g. dialed via its multiaddr).
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
    /// auto-dialed**; a higher layer decides whether/when to dial (eclipse-resistance).
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

    /// Await the next rendezvous-discovered peer record. Surfaced only; the transport
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
        self.dial_outcome(addr).await.map(|_| ())
    }

    pub async fn dial_outcome(
        &self,
        addr: Multiaddr,
    ) -> Result<catcoms_rt::DialSubmission, TransportError> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Dial {
                addr,
                permit: None,
                reply: Some(reply),
            })
            .await
            .map_err(|_| TransportError::Closed)?;
        rx.await.map_err(|_| TransportError::Closed)
    }

    /// Transfer a discovery scheduler permit into the actor before awaiting its decision.
    pub async fn dial_permit(
        &self,
        permit: catcoms_rt::BoxedDialPermit,
    ) -> Result<catcoms_rt::DialSubmission, TransportError> {
        let addr = permit
            .address()
            .parse::<Multiaddr>()
            .map_err(|_| TransportError::InvalidDialBatch)?;
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Dial {
                addr,
                permit: Some(permit),
                reply: Some(reply),
            })
            .await
            .map_err(|_| TransportError::Closed)?;
        rx.await.map_err(|_| TransportError::Closed)
    }

    /// Submit one bounded direct-route batch whose terminal identity must match `peer`.
    ///
    /// String parsing here is only an early rejection. The actor repeats the peer binding and
    /// direct-route checks on the owned `Multiaddr` values immediately before its dial ledger.
    pub async fn dial_peer_batch(
        &self,
        peer: PeerId,
        addresses: &[String],
    ) -> Result<Vec<catcoms_rt::DialSubmission>, TransportError> {
        if addresses.is_empty() || addresses.len() > MAX_PEER_DIAL_BATCH {
            return Err(TransportError::InvalidDialBatch);
        }
        let addrs = addresses
            .iter()
            .map(|address| {
                address
                    .parse::<Multiaddr>()
                    .map_err(|_| TransportError::InvalidDialBatch)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::DialPeerBatch {
                peer,
                addrs,
                permits: None,
                reply,
            })
            .await
            .map_err(|_| TransportError::Closed)?;
        rx.await.map_err(|_| TransportError::Closed)?
    }

    /// Transfer a peer-bound discovery batch into the actor as one owned command.
    pub async fn dial_peer_permits(
        &self,
        peer: PeerId,
        permits: Vec<catcoms_rt::BoxedDialPermit>,
    ) -> Result<Vec<catcoms_rt::DialSubmission>, TransportError> {
        if permits.is_empty() || permits.len() > MAX_PEER_DIAL_BATCH {
            return Err(TransportError::InvalidDialBatch);
        }
        let addrs = permits
            .iter()
            .map(|permit| {
                permit
                    .address()
                    .parse::<Multiaddr>()
                    .map_err(|_| TransportError::InvalidDialBatch)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::DialPeerBatch {
                peer,
                addrs,
                permits: Some(permits),
                reply,
            })
            .await
            .map_err(|_| TransportError::Closed)?;
        rx.await.map_err(|_| TransportError::Closed)?
    }

    /// Advertise `addr` as an external (reachable) address, so a node with a directly-
    /// reachable address can register at a rendezvous **without** a relay circuit
    /// (a publicly-reachable server, or a memory listener in tests). Flushes any
    /// registration that was deferred for lack of an external address.
    pub async fn add_external_address(&self, addr: Multiaddr) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::AddExternalAddress(addr))
            .await
            .map_err(|_| TransportError::Closed)
    }

    /// Withdraw an address previously asserted with [`MeshService::add_external_address`]. An
    /// identical active router mapping remains advertised until its own lease expires.
    pub async fn remove_external_address(&self, addr: Multiaddr) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::RemoveExternalAddress(addr))
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
        let protected = dial.iter().filter_map(target_peer_in_multiaddr).collect();
        Ok(Self::spawn_protecting(swarm, protected, false))
    }

    /// Build a **TCP + QUIC** node that listens on `listen` and dials `dial`, then spawn it. Also
    /// returns this node's libp2p `PeerId` so the caller can advertise a dialable `/…/p2p/<id>`
    /// bootstrap address (e.g. inside an invite). Real cross-process / cross-machine networking.
    ///
    /// The identity is fresh, so this node's `PeerId` lasts only as long as the process. A node
    /// whose address goes into an invite wants [`MeshService::new_tcp_with_key`] instead.
    pub fn new_tcp(
        listen: Option<Multiaddr>,
        dial: &[Multiaddr],
    ) -> Result<(Self, libp2p::PeerId), NetError> {
        let listen: Vec<Multiaddr> = listen.into_iter().collect();
        let swarm = build_tcp_swarm()?;
        let (svc, id, _bound) = Self::listen_dial_spawn(swarm, &listen, dial, false)?;
        Ok((svc, id))
    }

    /// Like [`MeshService::new_tcp`] but with a **caller-supplied (persisted) identity** and a
    /// *list* of listen addresses, so one node can bind IPv4 + IPv6 and TCP + QUIC under one
    /// `PeerId`. Returns the addresses that actually bound: a single refusal (a saved port that
    /// something else took while we were closed, or an IPv6 listener on a host with no IPv6 stack)
    /// must not sink the node, so it is reported rather than fatal. Only an *empty* result for a
    /// non-empty request is an error.
    pub fn new_tcp_with_key(
        key: libp2p::identity::Keypair,
        listen: &[Multiaddr],
        dial: &[Multiaddr],
    ) -> Result<(Self, libp2p::PeerId, Vec<Multiaddr>), NetError> {
        let swarm = build_tcp_swarm_with_key(key)?;
        Self::listen_dial_spawn(swarm, listen, dial, false)
    }

    /// Product constructor that explicitly enables UPnP/PCP/NAT-PMP for the supplied stable
    /// all-interface listeners. General TCP constructors keep router mutation disabled.
    pub fn new_tcp_with_key_and_port_mapping(
        key: libp2p::identity::Keypair,
        listen: &[Multiaddr],
        dial: &[Multiaddr],
    ) -> Result<(Self, libp2p::PeerId, Vec<Multiaddr>), NetError> {
        let swarm = build_tcp_swarm_with_key_and_port_mapping(key, true)?;
        Self::listen_dial_spawn(swarm, listen, dial, true)
    }

    /// Apply the listen/dial set to a freshly-built swarm and spawn its actor. Shared by the
    /// identity-less and persisted-identity constructors so they cannot drift.
    fn listen_dial_spawn(
        mut swarm: Swarm<MeshBehaviour>,
        listen: &[Multiaddr],
        dial: &[Multiaddr],
        enable_port_mapping: bool,
    ) -> Result<(Self, libp2p::PeerId, Vec<Multiaddr>), NetError> {
        let libp2p_id = *swarm.local_peer_id();
        let mut bound = Vec::new();
        let mut last_err = None;
        for addr in listen {
            match swarm.listen_on(addr.clone()) {
                Ok(_) => bound.push(addr.clone()),
                Err(e) => {
                    tracing::warn!(%addr, error = %e, "listen refused; continuing on the others");
                    last_err = Some(e.to_string());
                }
            }
        }
        if !listen.is_empty() && bound.is_empty() {
            return Err(NetError::Listen(
                last_err.unwrap_or_else(|| "no listen address bound".into()),
            ));
        }
        for addr in dial {
            swarm
                .dial(addr.clone())
                .map_err(|e| NetError::Dial(e.to_string()))?;
        }
        // Whatever this node was constructed to dial is its bootstrap/infra set, chosen locally
        // and never by a peer record, so no eviction may take it away (see `Actor::protected`).
        let protected = dial.iter().filter_map(target_peer_in_multiaddr).collect();
        Ok((
            Self::spawn_protecting(swarm, protected, enable_port_mapping),
            libp2p_id,
            bound,
        ))
    }

    /// A cheap, clonable [`MeshHandle`] to this node's command channel, for driving rendezvous
    /// register/dial **after** the `MeshService` has been moved elsewhere (e.g. into a server
    /// actor); the desktop bridge keeps one to register a fresh invite's namespace post-spawn.
    pub fn handle(&self) -> MeshHandle {
        MeshHandle {
            local: self.local,
            cmd_tx: self.cmd_tx.clone(),
            authenticated_route_rx: self.authenticated_route_rx.clone(),
            authenticated_route_evidence_rx: self.authenticated_route_evidence_rx.clone(),
        }
    }
}

/// A cheap, clonable handle to a spawned [`MeshService`]'s command channel. Exposes the
/// fire-and-forget control verbs (register / discover / dial / advertise) but NOT the
/// single-consumer event receivers. Lets a caller drive rendezvous registration once the owning
/// `MeshService` has been moved away (e.g. the bridge registering a fresh invite's namespace after
/// the server was spawned into its actor). Confirmation ([`MeshService::next_registered`]) stays
/// with the owner; registration is internally deferred + flushed once an external address exists,
/// so a handle's `rendezvous_register` still lands without the handle observing the grant.
#[derive(Clone, Debug)]
pub struct MeshHandle {
    local: PeerId,
    cmd_tx: mpsc::Sender<Command>,
    authenticated_route_rx: watch::Receiver<Vec<AuthenticatedDialRoute>>,
    authenticated_route_evidence_rx: watch::Receiver<Vec<AuthenticatedDialRoute>>,
}

impl MeshHandle {
    /// This node's transport peer id.
    pub fn local_peer(&self) -> PeerId {
        self.local
    }

    /// Snapshot the currently-live outbound direct routes that completed Noise authentication.
    ///
    /// The values contain private network addresses and are intentionally not part of
    /// [`MeshTransport::connection_snapshot`]. Callers should retain only routes needed for a
    /// specific locally-authorized reconnect and must re-check application membership before a
    /// later dial.
    pub fn authenticated_dial_routes(&self) -> Vec<AuthenticatedDialRoute> {
        let mut snapshot = self.authenticated_route_rx.clone();
        current_authenticated_routes(&mut snapshot).unwrap_or_default()
    }

    /// Snapshot this process's bounded recent outbound Noise-authenticated route evidence.
    ///
    /// A route remains here after a short connection closes so an already-authorized recovery can
    /// seal it without racing liveness. This grants no authority: consumers must still require an
    /// exact current member-to-peer claim and local admission/recovery consent.
    pub fn authenticated_dial_route_evidence(&self) -> Vec<AuthenticatedDialRoute> {
        let mut snapshot = self.authenticated_route_evidence_rx.clone();
        current_authenticated_routes(&mut snapshot).unwrap_or_default()
    }

    /// See [`MeshService::rendezvous_register`].
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

    /// See [`MeshService::rendezvous_discover`].
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

    /// See [`MeshService::dial`].
    pub async fn dial(&self, addr: Multiaddr) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::Dial {
                addr,
                permit: None,
                reply: None,
            })
            .await
            .map_err(|_| TransportError::Closed)
    }

    /// Queue one small address set as a single actor-drain dial batch.
    ///
    /// The actor groups adjacent `Command::Dial` values by their terminal peer id before touching
    /// libp2p, so TCP/QUIC candidates race inside one `DialOpts` and the normal `(peer,address)`
    /// ledger suppresses duplicates. This helper intentionally caps at four: its caller is the
    /// pre-membership two-way invite path, where every target came from an untrusted paste.
    pub async fn dial_join_candidates(
        &self,
        addresses: &[Multiaddr],
    ) -> Result<(), TransportError> {
        for address in addresses.iter().take(4) {
            self.cmd_tx
                .send(Command::Dial {
                    addr: address.clone(),
                    permit: None,
                    reply: None,
                })
                .await
                .map_err(|_| TransportError::Closed)?;
        }
        Ok(())
    }

    /// See [`MeshService::dial_peer_batch`].
    pub async fn dial_peer_batch(
        &self,
        peer: PeerId,
        addresses: &[String],
    ) -> Result<Vec<catcoms_rt::DialSubmission>, TransportError> {
        if addresses.is_empty() || addresses.len() > MAX_PEER_DIAL_BATCH {
            return Err(TransportError::InvalidDialBatch);
        }
        let addrs = addresses
            .iter()
            .map(|address| {
                address
                    .parse::<Multiaddr>()
                    .map_err(|_| TransportError::InvalidDialBatch)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::DialPeerBatch {
                peer,
                addrs,
                permits: None,
                reply,
            })
            .await
            .map_err(|_| TransportError::Closed)?;
        rx.await.map_err(|_| TransportError::Closed)?
    }

    /// Send one bounded control request without routing it through the group `ChannelSync` actor.
    /// Used by the short pre-member reply proof so a quiet joiner cannot stall every group
    /// command while the network request waits for its timeout.
    pub async fn request_control(
        &self,
        peer: PeerId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Request { peer, data, reply })
            .await
            .map_err(|_| TransportError::Closed)?;
        rx.await.map_err(|_| TransportError::Closed)?
    }

    /// Send one control request only when `peer` has a live connection at actor handling time.
    ///
    /// This is intentionally narrower than [`MeshHandle::request_control`]: it never consults
    /// the recent-peer cache and therefore cannot initiate an implicit redial. Reciprocal invite
    /// proof retries use it when the shared endpoint scheduler has not granted a new socket pass.
    pub async fn request_control_connected_only(
        &self,
        peer: PeerId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::RequestConnected { peer, data, reply })
            .await
            .map_err(|_| TransportError::Closed)?;
        rx.await.map_err(|_| TransportError::Closed)?
    }

    /// See [`MeshService::add_external_address`].
    pub async fn add_external_address(&self, addr: Multiaddr) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::AddExternalAddress(addr))
            .await
            .map_err(|_| TransportError::Closed)
    }

    /// Withdraw an address previously asserted with [`MeshHandle::add_external_address`]. The
    /// command is actor-owned so it can preserve an identical live router-mapping owner.
    pub async fn remove_external_address(&self, addr: Multiaddr) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::RemoveExternalAddress(addr))
            .await
            .map_err(|_| TransportError::Closed)
    }
}

#[async_trait]
impl MeshTransport for MeshService {
    fn local_peer(&self) -> PeerId {
        self.local
    }

    fn connection_snapshot(&self) -> Vec<PeerConnectionSnapshot> {
        MeshService::connection_snapshot(self)
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

    async fn request_connected(
        &self,
        peer: PeerId,
        _proto: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::RequestConnected { peer, data, reply })
            .await
            .map_err(|_| TransportError::Closed)?;
        rx.await.map_err(|_| TransportError::Closed)?
    }

    /// Queue the send and return; the only wait is for room in the actor's command channel.
    async fn notify(
        &self,
        peer: PeerId,
        _proto: ProtocolId,
        data: Bytes,
    ) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::Notify { peer, data })
            .await
            .map_err(|_| TransportError::Closed)
    }

    async fn notify_connected(
        &self,
        peer: PeerId,
        _proto: ProtocolId,
        data: Bytes,
    ) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::NotifyConnected { peer, data })
            .await
            .map_err(|_| TransportError::Closed)
    }

    async fn next_event(&self) -> Option<TransportEvent> {
        self.event_rx.lock().await.recv().await
    }

    // Rendezvous discovery; delegate to the inherent methods, mapping rt-native opaque bytes/
    // strings to libp2p types. (`MeshService::method(self, ..)` is the explicit inherent call.)
    async fn rendezvous_register(
        &self,
        namespace: &str,
        rz_node: &[u8],
    ) -> Result<(), TransportError> {
        let rz = libp2p::PeerId::from_bytes(rz_node).map_err(|_| TransportError::Closed)?;
        MeshService::rendezvous_register(self, namespace, rz)
            .await
            .map_err(|_| TransportError::Closed)
    }

    async fn rendezvous_discover(
        &self,
        namespace: &str,
        rz_node: &[u8],
    ) -> Result<(), TransportError> {
        let rz = libp2p::PeerId::from_bytes(rz_node).map_err(|_| TransportError::Closed)?;
        MeshService::rendezvous_discover(self, namespace, rz)
            .await
            .map_err(|_| TransportError::Closed)
    }

    async fn dial_addr(&self, addr: &str) -> Result<(), TransportError> {
        let m: Multiaddr = addr.parse().map_err(|_| TransportError::Closed)?;
        MeshService::dial(self, m).await
    }

    async fn dial_addr_outcome(
        &self,
        addr: &str,
    ) -> Result<catcoms_rt::DialSubmission, TransportError> {
        let m: Multiaddr = addr.parse().map_err(|_| TransportError::Closed)?;
        MeshService::dial_outcome(self, m).await
    }

    async fn dial_permit(
        &self,
        permit: catcoms_rt::BoxedDialPermit,
    ) -> Result<catcoms_rt::DialSubmission, TransportError> {
        MeshService::dial_permit(self, permit).await
    }

    async fn dial_peer_batch(
        &self,
        peer: PeerId,
        addresses: &[String],
    ) -> Result<Vec<catcoms_rt::DialSubmission>, TransportError> {
        MeshService::dial_peer_batch(self, peer, addresses).await
    }

    async fn dial_peer_permits(
        &self,
        peer: PeerId,
        permits: Vec<catcoms_rt::BoxedDialPermit>,
    ) -> Result<Vec<catcoms_rt::DialSubmission>, TransportError> {
        MeshService::dial_peer_permits(self, peer, permits).await
    }

    async fn add_external_addr(&self, addr: &str) -> Result<(), TransportError> {
        let m: Multiaddr = addr.parse().map_err(|_| TransportError::Closed)?;
        MeshService::add_external_address(self, m).await
    }

    async fn next_discovered(&self) -> Option<DiscoveredPeer> {
        let d = MeshService::next_discovered(self).await?;
        Some(DiscoveredPeer {
            peer: d.peer.to_bytes(),
            addresses: d.addresses.iter().map(|a| a.to_string()).collect(),
            namespace: d.namespace,
            seq: d.seq,
        })
    }

    async fn next_registered(&self) -> Option<RendezvousRegistration> {
        let registered = MeshService::next_registered(self).await?;
        Some(RendezvousRegistration {
            rendezvous_node: registered.rendezvous_node.to_bytes(),
            namespace: registered.namespace,
            ttl_secs: registered.ttl,
        })
    }

    async fn evict_peer(&self, peer: PeerId) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::Evict(peer))
            .await
            .map_err(|_| TransportError::Closed)
    }

    async fn unevict_peer(&self, peer: PeerId) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::Unevict(peer))
            .await
            .map_err(|_| TransportError::Closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::rng::RngError;
    use catcoms_rt::{CryptoRng, ManualClock, RngCore};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Debug)]
    struct DropObservedPermit {
        address: String,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for DropObservedPermit {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl catcoms_rt::DialPermit for DropObservedPermit {
        fn address(&self) -> &str {
            &self.address
        }

        fn commit_if_current(self: Box<Self>) -> Option<String> {
            Some(self.address.clone())
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct FixedRng(u8);

    impl RngCore for FixedRng {
        fn next_u32(&mut self) -> u32 {
            0
        }

        fn next_u64(&mut self) -> u64 {
            0
        }

        fn fill_bytes(&mut self, dest: &mut [u8]) {
            dest.fill(self.0);
        }

        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), RngError> {
            self.fill_bytes(dest);
            Ok(())
        }
    }

    impl CryptoRng for FixedRng {}

    /// A helper peer id distinct per `n`.
    fn ev_peer(n: u64) -> PeerId {
        PeerId::from_u64(n)
    }

    fn libp2p_peer() -> libp2p::PeerId {
        libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id()
    }

    #[test]
    fn reciprocal_batch_guard_rejects_substitution_relay_and_oversize_inputs() {
        let target = libp2p_peer();
        let other = libp2p_peer();
        let peer = to_peer(&target);
        let v4: Multiaddr = format!("/ip4/198.51.100.7/tcp/22487/p2p/{target}")
            .parse()
            .unwrap();
        let v6: Multiaddr = format!("/ip6/2001:db8::7/udp/22487/quic-v1/p2p/{target}")
            .parse()
            .unwrap();
        assert!(valid_direct_peer_batch(peer, &[v4.clone(), v6.clone()]));
        assert!(!valid_direct_peer_batch(peer, &[]));
        assert!(!valid_direct_peer_batch(
            peer,
            &[v4.clone(), v6.clone(), v4.clone()]
        ));

        let substituted: Multiaddr = format!("/ip4/198.51.100.7/tcp/22487/p2p/{other}")
            .parse()
            .unwrap();
        assert!(!valid_direct_peer_batch(peer, &[substituted]));
        let relay: Multiaddr =
            format!("/ip4/198.51.100.8/tcp/4001/p2p/{other}/p2p-circuit/p2p/{target}")
                .parse()
                .unwrap();
        assert!(!valid_direct_peer_batch(peer, &[relay]));
        let unbound: Multiaddr = "/ip4/198.51.100.7/tcp/22487".parse().unwrap();
        assert!(!valid_direct_peer_batch(peer, &[unbound]));
    }

    #[tokio::test]
    async fn a_cancelled_reply_receiver_cannot_reclaim_a_queued_dial_permit() {
        let target = libp2p_peer();
        let address = format!("/ip4/198.51.100.7/tcp/22487/p2p/{target}");
        let drops = Arc::new(AtomicUsize::new(0));
        let permit = DropObservedPermit {
            address: address.clone(),
            drops: drops.clone(),
        };
        let (command_tx, mut command_rx) = mpsc::channel(1);
        let (reply, reply_rx) = oneshot::channel();
        command_tx
            .send(Command::Dial {
                addr: address.parse().unwrap(),
                permit: Some(Box::new(permit)),
                reply: Some(reply),
            })
            .await
            .unwrap();

        // This is the original failure window: the calling future disappears after enqueue but
        // before the actor replies. The permit must remain owned by the queued command.
        drop(reply_rx);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let queued = command_rx.recv().await.expect("command remained queued");
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(queued);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    fn dialled_endpoint(address: &str) -> ConnectedPoint {
        ConnectedPoint::Dialer {
            address: address.parse().unwrap(),
            role_override: Endpoint::Dialer,
            port_use: PortUse::New,
        }
    }

    #[test]
    fn connection_path_classification_prefers_route_semantics_over_carriers() {
        assert_eq!(
            classify_connection_path(&dialled_endpoint("/ip4/198.51.100.5/tcp/443/wss")),
            ConnectionPath {
                family: ConnectionFamily::Ipv4,
                transport: ConnectionTransport::WebSocket,
                direction: ConnectionDirection::Dialer,
            }
        );
        assert_eq!(
            classify_connection_path(&dialled_endpoint("/ip6/2001:db8::5/udp/22487/quic-v1")),
            ConnectionPath {
                family: ConnectionFamily::Ipv6,
                transport: ConnectionTransport::QuicV1,
                direction: ConnectionDirection::Dialer,
            }
        );

        let relay = libp2p_peer();
        let target = libp2p_peer();
        let relayed =
            format!("/dns4/relay.example/tcp/443/wss/p2p/{relay}/p2p-circuit/p2p/{target}");
        assert_eq!(
            classify_connection_path(&dialled_endpoint(&relayed)),
            ConnectionPath {
                family: ConnectionFamily::Dns,
                // A relay circuit is not a direct WebSocket path to the member.
                transport: ConnectionTransport::CircuitRelay,
                direction: ConnectionDirection::Dialer,
            }
        );

        let inbound = ConnectedPoint::Listener {
            local_addr: "/ip4/0.0.0.0/tcp/22487".parse().unwrap(),
            send_back_addr: "/ip4/203.0.113.10/tcp/53000".parse().unwrap(),
        };
        assert_eq!(
            classify_connection_path(&inbound),
            ConnectionPath {
                family: ConnectionFamily::Ipv4,
                transport: ConnectionTransport::Tcp,
                direction: ConnectionDirection::Listener,
            }
        );

        let inbound_relay = ConnectedPoint::Listener {
            local_addr: format!("/ip4/198.51.100.8/tcp/4001/p2p/{relay}/p2p-circuit")
                .parse()
                .unwrap(),
            send_back_addr: "/memory/9".parse().unwrap(),
        };
        assert_eq!(
            classify_connection_path(&inbound_relay),
            ConnectionPath {
                family: ConnectionFamily::Ipv4,
                transport: ConnectionTransport::CircuitRelay,
                direction: ConnectionDirection::Listener,
            },
            "an inbound relay's family and transport both come from local_addr"
        );

        assert_eq!(
            classify_connection_path(&dialled_endpoint("/ip6/::ffff:192.0.2.4/tcp/22487")).family,
            ConnectionFamily::Ipv4,
            "IPv4-mapped IPv6 is an IPv4 path in the canonical route model"
        );
    }

    #[test]
    fn only_noise_authenticated_outbound_ip_routes_become_reconnect_hints() {
        let target = libp2p_peer();
        let peer = to_peer(&target);
        let direct = dialled_endpoint("/ip4/192.168.1.40/tcp/22487");
        assert_eq!(
            authenticated_dial_route(target, &direct),
            Some(AuthenticatedDialRoute {
                peer,
                address: format!("/ip4/192.168.1.40/tcp/22487/p2p/{target}"),
            })
        );

        let inbound = ConnectedPoint::Listener {
            local_addr: "/ip4/0.0.0.0/tcp/22487".parse().unwrap(),
            send_back_addr: "/ip4/192.168.1.40/tcp/53122".parse().unwrap(),
        };
        assert_eq!(
            authenticated_dial_route(target, &inbound),
            None,
            "an inbound ephemeral source port is not a future listener route"
        );
        assert_eq!(
            authenticated_dial_route(
                target,
                &dialled_endpoint(&format!(
                    "/ip4/192.168.1.1/tcp/4001/p2p/{target}/p2p-circuit"
                )),
            ),
            None,
            "relay lifecycle and consent are not persisted as a direct LAN hint"
        );
        assert_eq!(
            authenticated_dial_route(
                target,
                &dialled_endpoint(&format!("/dns4/member.invalid/tcp/22487/p2p/{target}")),
            ),
            None,
            "cross-session DNS rebinding is not retained"
        );
        assert_eq!(
            authenticated_dial_route(
                target,
                &dialled_endpoint(&format!("/ip4/192.168.1.40/tcp/22487/ws/p2p/{target}")),
            ),
            None,
            "persisted reconnects stay raw TCP/QUIC rather than widening to WebSocket"
        );
    }

    #[test]
    fn dial_diagnostics_keep_route_shape_without_private_coordinates() {
        let private: Multiaddr = "/ip4/192.168.77.91/tcp/22487".parse().unwrap();
        let quic: Multiaddr = "/ip6/fd00::7/udp/22487/quic-v1".parse().unwrap();
        let rendered = format!("{:?}", dial_log_shape([&private, &quic]));

        assert!(!rendered.contains("192.168.77.91"));
        assert!(!rendered.contains("fd00::7"));
        assert_eq!(
            dial_log_shape([&private, &quic]),
            DialLogShape {
                ipv4: 1,
                ipv6: 1,
                tcp: 1,
                quic: 1,
                ..DialLogShape::default()
            }
        );
    }

    #[test]
    fn final_close_clears_every_authenticated_route_for_that_peer() {
        let peer = libp2p_peer();
        let other = libp2p_peer();
        let route = |target: libp2p::PeerId, port| AuthenticatedDialRoute {
            peer: to_peer(&target),
            address: format!("/ip4/192.168.1.40/tcp/{port}/p2p/{target}"),
        };
        let mut routes = HashMap::from([
            ((peer, ConnectionId::new_unchecked(1)), route(peer, 22_487)),
            ((peer, ConnectionId::new_unchecked(2)), route(peer, 22_488)),
            (
                (other, ConnectionId::new_unchecked(3)),
                route(other, 22_489),
            ),
        ]);

        forget_authenticated_routes(&mut routes, peer, ConnectionId::new_unchecked(999), 0);
        assert!(routes.keys().all(|(candidate, _)| *candidate != peer));
        assert!(routes.keys().any(|(candidate, _)| *candidate == other));
    }

    #[test]
    fn recent_authenticated_route_evidence_survives_close_and_is_per_peer_bounded() {
        let transport = libp2p_peer();
        let peer = to_peer(&transport);
        let route = |port| AuthenticatedDialRoute {
            peer,
            address: format!("/ip4/192.168.1.40/tcp/{port}/p2p/{transport}"),
        };
        let mut live =
            HashMap::from([((transport, ConnectionId::new_unchecked(1)), route(22_487))]);
        let mut evidence = VecDeque::new();
        record_authenticated_route_evidence(&mut evidence, route(22_487));
        record_authenticated_route_evidence(&mut evidence, route(22_488));
        record_authenticated_route_evidence(&mut evidence, route(22_489));

        forget_authenticated_routes(&mut live, transport, ConnectionId::new_unchecked(1), 0);
        assert!(live.is_empty(), "present-time liveness remains truthful");
        assert_eq!(
            evidence.into_iter().collect::<Vec<_>>(),
            vec![route(22_488), route(22_489)],
            "a short edge leaves bounded proof for the recovery worker"
        );
    }

    #[test]
    fn partial_close_removes_only_its_authenticated_route_even_when_path_shape_is_unchanged() {
        let peer = libp2p_peer();
        let first = ConnectionId::new_unchecked(1);
        let surviving = ConnectionId::new_unchecked(2);
        let route = |port| AuthenticatedDialRoute {
            peer: to_peer(&peer),
            address: format!("/ip4/192.168.1.40/tcp/{port}/p2p/{peer}"),
        };
        let mut routes = HashMap::from([
            ((peer, first), route(22_487)),
            ((peer, surviving), route(22_488)),
        ]);
        let (tx, mut rx) = watch::channel(authenticated_route_snapshot(&routes));

        // Both physical connections have the same coarse IPv4/TCP/dialer projection. The actor
        // must still advance the independent address-bearing watch when one closes.
        forget_and_publish_authenticated_routes(&mut routes, &tx, peer, first, 1);
        assert!(rx.has_changed().unwrap());
        assert_eq!(rx.borrow_and_update().clone(), vec![route(22_488)]);
        assert_eq!(routes.get(&(peer, surviving)), Some(&route(22_488)));
    }

    #[test]
    fn connection_path_ledger_handles_upgrades_duplicates_and_final_close() {
        let peer = libp2p_peer();
        let other = libp2p_peer();
        let relay = ConnectionPath {
            family: ConnectionFamily::Ipv6,
            transport: ConnectionTransport::CircuitRelay,
            direction: ConnectionDirection::Listener,
        };
        let direct = ConnectionPath {
            family: ConnectionFamily::Ipv4,
            transport: ConnectionTransport::QuicV1,
            direction: ConnectionDirection::Dialer,
        };
        let mut paths = HashMap::from([
            ((peer, ConnectionId::new_unchecked(1)), relay),
            ((peer, ConnectionId::new_unchecked(2)), direct),
            ((other, ConnectionId::new_unchecked(3)), direct),
        ]);

        assert_eq!(active_connection_paths(&paths, peer), vec![direct, relay]);
        forget_connection_path(&mut paths, peer, ConnectionId::new_unchecked(2), 1);
        assert_eq!(active_connection_paths(&paths, peer), vec![relay]);

        // Two physical relay connections intentionally collapse into one coarse UI path. Closing
        // either must leave the path present while the other is established.
        paths.insert((peer, ConnectionId::new_unchecked(4)), relay);
        forget_connection_path(&mut paths, peer, ConnectionId::new_unchecked(1), 1);
        assert_eq!(active_connection_paths(&paths, peer), vec![relay]);

        // The authoritative final count heals a missing close id and preserves other peers.
        forget_connection_path(&mut paths, peer, ConnectionId::new_unchecked(999), 0);
        assert!(active_connection_paths(&paths, peer).is_empty());
        assert_eq!(active_connection_paths(&paths, other), vec![direct]);
    }

    #[test]
    fn router_mapping_uses_the_tcp_and_quic_listen_ports_only() {
        let tcp: Multiaddr = "/ip4/192.168.1.4/tcp/22487".parse().unwrap();
        let quic: Multiaddr = "/ip4/192.168.1.4/udp/22487/quic-v1".parse().unwrap();
        let ipv6_tcp: Multiaddr = "/ip6/2606:4700::10/tcp/22487".parse().unwrap();
        let loopback: Multiaddr = "/ip4/127.0.0.1/tcp/22487".parse().unwrap();
        let plain_udp: Multiaddr = "/ip4/0.0.0.0/udp/22487".parse().unwrap();
        let wildcard: Multiaddr = "/ip4/0.0.0.0/tcp/22487".parse().unwrap();
        let ipv6_only: Multiaddr = "/ip6/::/udp/22487/quic-v1".parse().unwrap();
        let relayed: Multiaddr = "/ip4/198.51.100.8/tcp/4001/p2p/12D3KooWJvFzZpCWKjQbGvYQ8uY4rMw1qQznfKqcxpN6qjHVVqUd/p2p-circuit"
            .parse()
            .unwrap();
        let memory: Multiaddr = "/memory/7".parse().unwrap();

        assert_eq!(
            port_mapping_endpoint(&tcp),
            Some(PortMappingEndpoint {
                key: PortMapperTaskKey {
                    transport: PortMappingTransport::Tcp,
                    target: PortMappingTarget::Ipv4,
                },
                port: NonZeroU16::new(22487).unwrap(),
            })
        );
        assert_eq!(
            port_mapping_endpoint(&quic),
            Some(PortMappingEndpoint {
                key: PortMapperTaskKey {
                    transport: PortMappingTransport::Udp,
                    target: PortMappingTarget::Ipv4,
                },
                port: NonZeroU16::new(22487).unwrap(),
            })
        );
        assert_eq!(
            port_mapping_endpoint(&ipv6_tcp),
            Some(PortMappingEndpoint {
                key: PortMapperTaskKey {
                    transport: PortMappingTransport::Tcp,
                    target: PortMappingTarget::Ipv6("2606:4700::10".parse().unwrap()),
                },
                port: NonZeroU16::new(22487).unwrap(),
            })
        );
        assert_eq!(port_mapping_endpoint(&plain_udp), None);
        assert_eq!(port_mapping_endpoint(&wildcard), None);
        assert_eq!(port_mapping_endpoint(&loopback), None);
        assert_eq!(port_mapping_endpoint(&ipv6_only), None);
        assert_eq!(port_mapping_endpoint(&relayed), None);
        assert_eq!(port_mapping_endpoint(&memory), None);
    }

    #[test]
    fn desired_mapping_workers_bound_ipv6_interfaces_and_follow_listener_loss() {
        let listeners: HashSet<Multiaddr> = [
            "/ip4/192.168.1.4/tcp/22487",
            "/ip4/192.168.1.4/udp/22487/quic-v1",
            "/ip6/2001:569::1/tcp/22487",
            "/ip6/2001:569::1/udp/22487/quic-v1",
            "/ip6/2606:4700::1/tcp/22487",
            "/ip6/2606:4700::1/udp/22487/quic-v1",
            "/ip6/2a00:1450::1/tcp/22487",
            "/ip6/2a00:1450::1/udp/22487/quic-v1",
        ]
        .into_iter()
        .map(|address| address.parse().unwrap())
        .collect();
        let desired = desired_port_mapping_endpoints(&listeners);
        assert_eq!(desired.len(), 6, "two IPv4 and at most four IPv6 workers");
        let selected_v6: HashSet<_> = desired
            .keys()
            .filter_map(|key| match key.target {
                PortMappingTarget::Ipv6(address) => Some(address),
                PortMappingTarget::Ipv4 => None,
            })
            .collect();
        assert_eq!(selected_v6.len(), 2);

        let remaining: HashSet<_> = listeners
            .into_iter()
            .filter(|address| !address.to_string().contains("2001:569::1"))
            .collect();
        let reconciled = desired_port_mapping_endpoints(&remaining);
        assert!(!reconciled
            .keys()
            .any(|key| { key.target == PortMappingTarget::Ipv6("2001:569::1".parse().unwrap()) }));
    }

    #[test]
    fn retired_ipv6_listener_clears_mapped_and_unavailable_state() {
        let local: Ipv6Addr = "2606:4700::10".parse().unwrap();
        let local_owner = Some(IpAddr::V6(local));
        let task = PortMapperTaskKey {
            transport: PortMappingTransport::Tcp,
            target: PortMappingTarget::Ipv6(local),
        };
        let owner = (
            PortMappingMechanism::Pcp,
            PortMappingTransport::Tcp,
            local_owner,
        );
        let mapped: Multiaddr = "/ip6/2606:4700::10/tcp/22487".parse().unwrap();
        let unrelated = (
            PortMappingMechanism::Pcp,
            PortMappingTransport::Udp,
            Some(IpAddr::V6("2a00:1450::10".parse().unwrap())),
        );
        let mut active = HashMap::from([(owner, mapped.clone())]);
        let mut unavailable = HashMap::from([
            (owner, "previous mapping expired; retrying".to_string()),
            (unrelated, "no IPv6 PCP target discovered".to_string()),
        ]);

        assert!(expire_port_mapping(&mut active, owner, &mapped));
        assert!(clear_retired_port_mapping_failures(&mut unavailable, task));
        assert!(!active.contains_key(&owner));
        assert!(!unavailable.contains_key(&owner));
        assert!(unavailable.contains_key(&unrelated));

        unavailable.insert(owner, "no IPv6 PCP target discovered".to_string());
        assert!(clear_retired_port_mapping_failures(&mut unavailable, task));
        assert!(!unavailable.contains_key(&owner));
    }

    /// The IPv4 task owns three diagnostic keys: PCP and NAT-PMP under `None`, plus the
    /// bound-UPnP companion under its concrete interface address. Retiring the task must sweep
    /// all three, while libp2p-UPnP's own `None`-keyed entry and other transports survive.
    #[test]
    fn retired_ipv4_task_sweeps_the_bound_upnp_companion_failures() {
        let task = PortMapperTaskKey {
            transport: PortMappingTransport::Tcp,
            target: PortMappingTarget::Ipv4,
        };
        let bound = (
            PortMappingMechanism::Upnp,
            PortMappingTransport::Tcp,
            Some(IpAddr::V4("192.168.0.231".parse().unwrap())),
        );
        let unbound_upnp = (PortMappingMechanism::Upnp, PortMappingTransport::Tcp, None);
        let other_transport = (
            PortMappingMechanism::Upnp,
            PortMappingTransport::Udp,
            Some(IpAddr::V4("192.168.0.231".parse().unwrap())),
        );
        let mut unavailable = HashMap::from([
            (
                (PortMappingMechanism::Pcp, PortMappingTransport::Tcp, None),
                "no compatible gateway answered the probe".to_string(),
            ),
            (bound, "no IGD gateway answered a search".to_string()),
            (
                unbound_upnp,
                "no UPnP IGD gateway answered discovery".to_string(),
            ),
            (
                other_transport,
                "no IGD gateway answered a search".to_string(),
            ),
        ]);
        assert!(clear_retired_port_mapping_failures(&mut unavailable, task));
        assert!(!unavailable.contains_key(&bound));
        // libp2p's own UPnP diagnostics are not this task's to clear, and the UDP task's
        // companion entry belongs to the UDP retirement.
        assert!(unavailable.contains_key(&unbound_upnp));
        assert!(unavailable.contains_key(&other_transport));
    }

    /// A candidate that names a socket on another adapter must be refused before any router
    /// work: a mapping to the default-route interface could never reach it. (This gate is the
    /// candidate's own claimed address, deliberately not a liveness probe: Windows Firewall
    /// rejects unsolicited inbound UDP to the webview with the same ICMP a dead socket
    /// produces, and a probe that cannot tell those apart vetoed every mapping in the field.)
    #[tokio::test]
    async fn a_media_port_claimed_by_another_interface_is_refused() {
        let Some(local_ip) = default_route_ipv4() else {
            return; // no route on this machine: the gate cannot be exercised
        };
        let port = NonZeroU16::new(50_000).unwrap();
        let other = Ipv4Addr::new(192, 0, 2, 55);
        assert_ne!(other, local_ip, "TEST-NET-1 can never be a real interface");
        let refused = map_media_udp_port(port, Some(other)).await;
        let message = refused.expect_err("a foreign-interface claim must never reach the router");
        assert!(
            message.contains("not on the default-route interface"),
            "refusal must say why: {message}"
        );
    }

    #[test]
    fn routed_ipv6_interfaces_win_the_bounded_worker_slots() {
        let routed: Ipv6Addr = "2a00:1450::1".parse().unwrap();
        let selected = select_ipv6_mapping_targets(
            vec![
                "2001:569::1".parse().unwrap(),
                "2606:4700::1".parse().unwrap(),
                routed,
            ],
            2,
            |candidate| candidate == routed,
        );
        assert!(selected.contains(&routed));
        assert_eq!(selected.len(), 2);
    }

    #[tokio::test]
    async fn a_replaced_mapping_worker_cannot_replay_buffered_events() {
        let key = PortMapperTaskKey {
            transport: PortMappingTransport::Tcp,
            target: PortMappingTarget::Ipv6("2606:4700::10".parse().unwrap()),
        };
        let port = NonZeroU16::new(22487).unwrap();
        let tasks = HashMap::from([(
            key,
            PortMapperTask {
                port,
                generation: 8,
                handle: Some(tokio::spawn(std::future::pending())),
                companion: None,
                stop: None,
            },
        )]);
        let report = |generation| PortMapperReport {
            key,
            port,
            generation,
            event: PortMappingEvent::Unavailable {
                mechanism: PortMappingMechanism::Pcp,
                transport: PortMappingTransport::Tcp,
                local_address: Some(IpAddr::V6("2606:4700::10".parse().unwrap())),
                detail: "test".to_string(),
            },
        };
        assert!(port_mapper_report_is_current(&tasks, &report(8)));
        assert!(!port_mapper_report_is_current(&tasks, &report(7)));
    }

    #[test]
    fn mesh_observations_replace_per_peer_and_stay_bounded() {
        let mut observations = HashMap::new();
        let mut order = VecDeque::new();
        let first = test_peer(1);
        record_mesh_observation(
            &mut observations,
            &mut order,
            first,
            "/ip4/198.51.100.1/tcp/40000".parse().unwrap(),
        );
        record_mesh_observation(
            &mut observations,
            &mut order,
            first,
            "/ip4/198.51.100.2/tcp/40001".parse().unwrap(),
        );
        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations[&first].to_string(),
            "/ip4/198.51.100.2/tcp/40001"
        );

        for n in 2..=(MAX_MESH_OBSERVATIONS as u8 + 2) {
            record_mesh_observation(
                &mut observations,
                &mut order,
                test_peer(n),
                format!("/ip4/198.51.100.{n}/tcp/40000").parse().unwrap(),
            );
        }
        assert_eq!(observations.len(), MAX_MESH_OBSERVATIONS);
        assert!(
            !observations.contains_key(&first),
            "the oldest peer is evicted"
        );
    }

    #[test]
    fn a_peer_is_remembered_with_its_routes_and_the_map_stays_bounded() {
        // The redial hint that makes a request to a disconnected peer survivable. `to_peer` is a
        // one-way hash and `peers` holds only live connections, so without this map an outbound
        // request to a peer whose connection had dropped could not even name a dial target.
        let mut recent = HashMap::new();
        let mut order = VecDeque::new();
        let first = to_peer(&test_peer(1));
        for n in 1..=(MAX_RECENT_PEER_ADDRS + 2) {
            record_recent_peer(
                &mut recent,
                &mut order,
                first,
                test_peer(1),
                format!("/ip4/198.51.100.1/tcp/{}", 40000 + n)
                    .parse()
                    .unwrap(),
            );
        }
        assert_eq!(
            recent[&first].addresses.len(),
            MAX_RECENT_PEER_ADDRS,
            "one peer's route list is capped"
        );
        assert_eq!(
            recent[&first].addresses.back().unwrap().to_string(),
            format!(
                "/ip4/198.51.100.1/tcp/{}",
                40000 + MAX_RECENT_PEER_ADDRS + 2
            ),
            "the freshest route is kept"
        );

        // A repeat visit must not renew the peer's place in the queue, or two chatty peers would
        // evict the quiet member whose route is the one actually worth remembering.
        // `test_peer` only spans a byte, and the bound needs more distinct peers than that.
        let wide_peer = |n: u16| {
            let mut seed = [5u8; 32];
            seed[..2].copy_from_slice(&n.to_be_bytes());
            keypair_from_seed(seed).unwrap().public().to_peer_id()
        };
        for n in 2..=(MAX_RECENT_PEERS as u16 + 2) {
            record_recent_peer(
                &mut recent,
                &mut order,
                to_peer(&wide_peer(n)),
                wide_peer(n),
                format!("/ip4/198.51.100.2/tcp/{n}").parse().unwrap(),
            );
        }
        assert_eq!(recent.len(), MAX_RECENT_PEERS);
        assert!(
            !recent.contains_key(&first),
            "the least recently learned peer is evicted"
        );
    }

    #[tokio::test]
    async fn router_mapping_behaviour_requires_the_explicit_product_builder() {
        let disabled = build_tcp_swarm_with_key(libp2p::identity::Keypair::generate_ed25519())
            .expect("default TCP swarm");
        assert!(!disabled.behaviour().upnp.is_enabled());

        let enabled = build_tcp_swarm_with_key_and_port_mapping(
            libp2p::identity::Keypair::generate_ed25519(),
            true,
        )
        .expect("mapping-enabled TCP swarm");
        assert!(enabled.behaviour().upnp.is_enabled());
    }

    #[test]
    fn mapper_falls_back_from_pcp_to_nat_pmp_and_classifies_lease_changes() {
        assert_eq!(
            mapping_attempt_plan(true, true),
            vec![PortMappingMechanism::Pcp, PortMappingMechanism::NatPmp],
            "a PCP MAP timeout must leave NAT-PMP as the next attempt"
        );
        assert_eq!(
            mapping_attempt_plan(false, true),
            vec![PortMappingMechanism::NatPmp]
        );

        let old: SocketAddrV4 = "8.8.8.8:22487".parse().unwrap();
        let new: SocketAddrV4 = "9.9.9.9:22487".parse().unwrap();
        assert_eq!(
            mapping_lease_action(old, Some(old)),
            MappingLeaseAction::Continue
        );
        assert_eq!(
            mapping_lease_action(old, None),
            MappingLeaseAction::Retry,
            "Some -> None withdraws the route and returns to the outer retry loop"
        );
        assert_eq!(
            mapping_lease_action(old, Some(new)),
            MappingLeaseAction::Replace(new),
            "Some(old) -> Some(new) replaces the published socket in the live worker"
        );
    }

    #[test]
    fn autonat_observations_are_bounded_updated_and_pruned_by_address() {
        let address: Multiaddr = "/ip4/8.8.8.8/tcp/22487".parse().unwrap();
        let mut results = HashMap::new();
        let mut order = VecDeque::new();

        for n in 0..=MAX_AUTONAT_OBSERVATIONS {
            record_autonat_result(
                &mut results,
                &mut order,
                AutoNatResult {
                    address: address.clone(),
                    server: test_peer(n as u8),
                    reachable: n % 2 == 0,
                    error: None,
                },
            );
        }
        assert_eq!(results.len(), MAX_AUTONAT_OBSERVATIONS);
        assert_eq!(order.len(), MAX_AUTONAT_OBSERVATIONS);
        assert!(
            !results.contains_key(&(address.clone(), test_peer(0))),
            "the oldest observation is evicted under server churn"
        );

        let newest = test_peer(MAX_AUTONAT_OBSERVATIONS as u8);
        record_autonat_result(
            &mut results,
            &mut order,
            AutoNatResult {
                address: address.clone(),
                server: newest,
                reachable: true,
                error: None,
            },
        );
        assert_eq!(results.len(), MAX_AUTONAT_OBSERVATIONS);
        assert_eq!(order.back(), Some(&(address.clone(), newest)));
        assert!(forget_autonat_address(&mut results, &mut order, &address));
        assert!(results.is_empty());
        assert!(order.is_empty());

        let pcp = (PortMappingMechanism::Pcp, PortMappingTransport::Tcp, None);
        let reacquired = HashMap::from([(pcp, address.clone())]);
        assert!(
            autonat_candidate_is_current(&HashSet::new(), &reacquired, &address,),
            "same-address remapping establishes a new current route that can be tested again"
        );
        assert!(
            !autonat_candidate_is_current(&HashSet::new(), &HashMap::new(), &address,),
            "a late result in the unowned interval is rejected"
        );
    }

    #[test]
    fn router_mapping_addresses_preserve_the_transport_a_joiner_must_dial() {
        let public: SocketAddrV4 = "203.0.113.7:22487".parse().unwrap();
        assert_eq!(
            mapped_multiaddr(public, PortMappingTransport::Tcp).to_string(),
            "/ip4/203.0.113.7/tcp/22487"
        );
        assert_eq!(
            mapped_multiaddr(public, PortMappingTransport::Udp).to_string(),
            "/ip4/203.0.113.7/udp/22487/quic-v1"
        );
    }

    #[test]
    fn a_repeated_router_answer_is_not_news_twice() {
        // Regression: a router with no PCP/NAT-PMP re-answers "no" every discovery cycle, for
        // every mechanism, transport and interface. Logged unconditionally that was 356 of one
        // real debug log's 601 lines, burying the sixty-odd entries that actually mattered.
        let detail = "no compatible gateway answered the probe";

        // The first answer for a probe is worth saying.
        assert!(mapping_unavailable_is_news(None, detail));
        // Saying it again eighteen seconds later is not.
        assert!(!mapping_unavailable_is_news(Some(detail), detail));
        // A router whose answer actually changed is news again: that is behaviour moving, which
        // is exactly what someone reading the log is looking for.
        assert!(mapping_unavailable_is_news(
            Some(detail),
            "the UPnP IGD gateway answered discovery"
        ));
        // ...and the new answer then goes quiet in its turn.
        assert!(!mapping_unavailable_is_news(
            Some("the UPnP IGD gateway answered discovery"),
            "the UPnP IGD gateway answered discovery"
        ));
    }

    #[test]
    fn different_probes_do_not_suppress_each_other() {
        // Suppression is per probe, keyed by mechanism, transport and interface. If one probe's
        // answer could silence another's, a genuinely new failure would vanish because an
        // unrelated one had already reported the same string.
        let mut seen: HashMap<(&str, &str), String> = HashMap::new();
        let detail = "no compatible gateway answered the probe".to_string();
        for probe in [("PCP", "UDP"), ("NAT-PMP", "UDP"), ("PCP", "TCP")] {
            let previous = seen.insert(probe, detail.clone());
            assert!(
                mapping_unavailable_is_news(previous.as_deref(), &detail),
                "{probe:?} is a distinct probe and its first answer is news"
            );
        }
        // Second time around, every one of them has already said it.
        for probe in [("PCP", "UDP"), ("NAT-PMP", "UDP"), ("PCP", "TCP")] {
            let previous = seen.insert(probe, detail.clone());
            assert!(!mapping_unavailable_is_news(previous.as_deref(), &detail));
        }
    }

    #[test]
    fn a_quiet_router_costs_one_line_per_probe_rather_than_one_per_cycle() {
        // The shape of the original bug, counted: twenty cycles over four probes was 80 lines.
        let detail = "no compatible gateway answered the probe".to_string();
        let mut seen: HashMap<(&str, &str), String> = HashMap::new();
        let mut lines = 0;
        for _cycle in 0..20 {
            for probe in [
                ("PCP", "UDP"),
                ("NAT-PMP", "UDP"),
                ("PCP", "TCP"),
                ("NAT-PMP", "TCP"),
            ] {
                let previous = seen.insert(probe, detail.clone());
                if mapping_unavailable_is_news(previous.as_deref(), &detail) {
                    lines += 1;
                }
            }
        }
        assert_eq!(
            lines, 4,
            "one line per probe for the whole session, not one per cycle"
        );
    }

    #[test]
    fn router_mapping_ownership_keeps_shared_addresses_until_the_last_lease() {
        let address: Multiaddr = "/ip4/8.8.8.8/tcp/22487".parse().unwrap();
        let replacement: Multiaddr = "/ip4/9.9.9.9/tcp/22487".parse().unwrap();
        let upnp = (PortMappingMechanism::Upnp, PortMappingTransport::Tcp, None);
        let pcp = (PortMappingMechanism::Pcp, PortMappingTransport::Tcp, None);
        let mut active = HashMap::new();

        assert_eq!(
            activate_port_mapping(&mut active, upnp, address.clone()),
            (None, true)
        );
        assert_eq!(
            activate_port_mapping(&mut active, pcp, address.clone()),
            (None, false),
            "the swarm already owns this address through UPnP"
        );
        assert!(!expire_port_mapping(&mut active, upnp, &address));
        assert!(expire_port_mapping(&mut active, pcp, &address));

        activate_port_mapping(&mut active, pcp, address.clone());
        assert_eq!(
            activate_port_mapping(&mut active, pcp, replacement.clone()),
            (Some(address.clone()), true),
            "replacing one key retires its unshared old address"
        );
        assert!(
            !expire_port_mapping(&mut active, pcp, &address),
            "a stale expiry cannot remove the replacement"
        );
        assert_eq!(active.get(&pcp), Some(&replacement));

        let private: Multiaddr = "/ip4/192.168.1.9/tcp/22487".parse().unwrap();
        assert!(!external_address_is_allowed(&HashSet::new(), &private));
        assert!(
            external_address_is_allowed(&HashSet::from([private.clone()]), &private),
            "a rejected private router result cannot remove an identical manual LAN route"
        );

        let shared_v6: Multiaddr = "/ip6/2606:4700::10/tcp/22487".parse().unwrap();
        let mut configured = HashSet::from([shared_v6.clone()]);
        let mapped_v6 = (
            PortMappingMechanism::Pcp,
            PortMappingTransport::Tcp,
            Some(IpAddr::V6("2606:4700::10".parse().unwrap())),
        );
        let mappings = HashMap::from([(mapped_v6, shared_v6.clone())]);
        assert!(
            !retire_configured_external_address(&mut configured, &mappings, &shared_v6),
            "removing a raw-interface owner must preserve an identical PCPv6 pinhole"
        );
        assert!(!configured.contains(&shared_v6));
        assert!(retire_configured_external_address(
            &mut HashSet::from([shared_v6.clone()]),
            &HashMap::new(),
            &shared_v6,
        ));

        let v6_owner = (
            PortMappingMechanism::Pcp,
            PortMappingTransport::Tcp,
            Some(IpAddr::V6("2606:4700::10".parse().unwrap())),
        );
        activate_port_mapping(
            &mut active,
            v6_owner,
            "/ip6/2606:4700::10/tcp/22487".parse().unwrap(),
        );
        assert!(active.contains_key(&pcp));
        assert!(active.contains_key(&v6_owner));
        assert_eq!(
            active.len(),
            2,
            "PCPv4 and PCPv6/TCP are independent lease owners"
        );
    }

    fn pcp_map_response(
        request: &[u8],
        result_code: u8,
        lifetime_seconds: u32,
        external_ip: Ipv6Addr,
        external_port: u16,
        epoch: u32,
    ) -> [u8; 60] {
        assert_eq!(request.len(), 60);
        let mut response = [0u8; 60];
        response[0] = 2;
        response[1] = 0x80 | 1;
        response[3] = result_code;
        response[4..8].copy_from_slice(&lifetime_seconds.to_be_bytes());
        response[8..12].copy_from_slice(&epoch.to_be_bytes());
        response[24..36].copy_from_slice(&request[24..36]);
        response[36] = request[36];
        response[40..42].copy_from_slice(&request[40..42]);
        response[42..44].copy_from_slice(&external_port.to_be_bytes());
        response[44..60].copy_from_slice(&external_ip.octets());
        response
    }

    #[tokio::test]
    async fn ipv6_pcp_worker_retains_failed_renewal_replaces_and_expires_monotonically() {
        let server = UdpSocket::bind("[::1]:0").await.unwrap();
        let server_addr = match server.local_addr().unwrap() {
            std::net::SocketAddr::V6(address) => address,
            std::net::SocketAddr::V4(_) => unreachable!("bound IPv6 loopback"),
        };
        let local_ip = Ipv6Addr::LOCALHOST;
        let external_ip: Ipv6Addr = "2606:4700::20".parse().unwrap();
        let port = NonZeroU16::new(22487).unwrap();
        let key = PortMapperTaskKey {
            transport: PortMappingTransport::Udp,
            target: PortMappingTarget::Ipv6(local_ip),
        };
        let (report_tx, mut report_rx) = mpsc::channel(16);
        let reporter = PortMapperReporter {
            tx: report_tx,
            key,
            port,
            generation: 11,
        };
        let (stop_tx, stop_rx) = oneshot::channel();
        let clock = ManualClock::new(10_000);
        let worker_clock = clock.clone();
        let worker = tokio::spawn(run_ipv6_port_mapper_with(
            local_ip,
            PortMappingTransport::Udp,
            port,
            reporter,
            stop_rx,
            Ipv6MapperDeps {
                clock: worker_clock,
                random: FixedRng(0x42),
                discover_gateway: move |_| Ok(server_addr),
            },
        ));

        let mut request = [0u8; 60];
        let (length, client) = server.recv_from(&mut request).await.unwrap();
        assert_eq!(length, 60);
        server
            .send_to(
                &pcp_map_response(&request, 0, 10, external_ip, port.get(), 100),
                client,
            )
            .await
            .unwrap();
        let first = report_rx.recv().await.unwrap();
        assert_eq!(first.generation, 11);
        assert!(matches!(first.event, PortMappingEvent::Mapped { .. }));

        // The first renewal is at half the ten-second grant with deterministic zero jitter.
        clock.advance_ms(5_000);
        let (length, renewal_client) = server.recv_from(&mut request).await.unwrap();
        assert_eq!((length, renewal_client), (60, client));
        server
            .send_to(
                &pcp_map_response(&request, 8, 2, external_ip, port.get(), 105),
                client,
            )
            .await
            .unwrap();
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        assert!(
            report_rx.try_recv().is_err(),
            "a renewal refusal must not withdraw a still-live lease"
        );

        clock.advance_ms(1_999);
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        assert!(server.try_recv_from(&mut request).is_err());
        clock.advance_ms(1);
        let (length, _) = server.recv_from(&mut request).await.unwrap();
        assert_eq!(length, 60);
        let replacement_port = port.get() + 1;
        server
            .send_to(
                &pcp_map_response(&request, 0, 4, external_ip, replacement_port, 107),
                client,
            )
            .await
            .unwrap();
        assert!(matches!(
            report_rx.recv().await.unwrap().event,
            PortMappingEvent::Expired { .. }
        ));
        assert!(matches!(
            report_rx.recv().await.unwrap().event,
            PortMappingEvent::Mapped { ref address, .. }
                if address.to_string().contains(&format!("/{replacement_port}"))
        ));

        // A wall-clock correction cannot extend the replacement's four-second router lease.
        clock.set_wall_ms(1);
        clock.advance_ms(2_000);
        let (length, _) = server.recv_from(&mut request).await.unwrap();
        assert_eq!(length, 60);
        server
            .send_to(
                &pcp_map_response(&request, 8, 100, external_ip, replacement_port, 109),
                client,
            )
            .await
            .unwrap();
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        assert!(report_rx.try_recv().is_err());
        clock.advance_ms(2_000);
        assert!(matches!(
            report_rx.recv().await.unwrap().event,
            PortMappingEvent::Expired { .. }
        ));
        assert!(matches!(
            report_rx.recv().await.unwrap().event,
            PortMappingEvent::Unavailable { ref detail, .. }
                if detail.contains("expired")
        ));

        let _ = stop_tx.send(());
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn ipv6_pcp_adapter_sends_exact_map_and_best_effort_delete() {
        let server = UdpSocket::bind("[::1]:0").await.unwrap();
        let server_addr = server.local_addr().unwrap();
        let local_ip: Ipv6Addr = "2606:4700::10".parse().unwrap();
        let external_ip: Ipv6Addr = "2606:4700::20".parse().unwrap();
        let port = NonZeroU16::new(22487).unwrap();
        let nonce = [0x42; 12];
        let gateway = tokio::spawn(async move {
            let mut packet = [0u8; pcp_ipv6::MAX_PACKET_SIZE];
            let (length, from) = server.recv_from(&mut packet).await.unwrap();
            assert_eq!(length, 60);
            assert_eq!(&packet[8..24], &local_ip.octets());
            assert_eq!(&packet[24..36], &nonce);
            assert_eq!(packet[36], 17);
            assert_eq!(
                u16::from_be_bytes(packet[40..42].try_into().unwrap()),
                22487
            );
            assert_eq!(
                u32::from_be_bytes(packet[4..8].try_into().unwrap()),
                pcp_ipv6::REQUESTED_LIFETIME_SECONDS
            );

            let mut response = [0u8; 60];
            response[0] = 2;
            response[1] = 0x80 | 1;
            response[4..8].copy_from_slice(&300u32.to_be_bytes());
            response[8..12].copy_from_slice(&91u32.to_be_bytes());
            response[24..36].copy_from_slice(&nonce);
            response[36] = 17;
            response[40..42].copy_from_slice(&22487u16.to_be_bytes());
            response[42..44].copy_from_slice(&22487u16.to_be_bytes());
            response[44..60].copy_from_slice(&external_ip.octets());
            server.send_to(&response, from).await.unwrap();

            let (delete_length, delete_from) = server.recv_from(&mut packet).await.unwrap();
            assert_eq!(delete_from, from);
            assert_eq!(delete_length, 60);
            assert_eq!(u32::from_be_bytes(packet[4..8].try_into().unwrap()), 0);
            assert_eq!(&packet[24..36], &nonce);
            assert_eq!(&packet[42..60], &[0u8; 18]);
        });
        let client = UdpSocket::bind("[::1]:0").await.unwrap();
        client.connect(server_addr).await.unwrap();
        let (_stop_tx, mut stop_rx) = oneshot::channel();
        let clock = ManualClock::new(1_000);
        let acquired = acquire_pcp_ipv6_lease(
            &client,
            local_ip,
            port,
            pcp_ipv6::MapProtocol::Udp,
            nonce,
            &mut stop_rx,
            &clock,
        )
        .await
        .unwrap();
        assert_eq!(
            acquired,
            PcpIpv6Acquire::Lease(pcp_ipv6::MapLease {
                external_ip,
                external_port: port,
                lifetime_seconds: 300,
                epoch: 91,
            })
        );
        delete_pcp_ipv6_lease(&client, local_ip, port, pcp_ipv6::MapProtocol::Udp, nonce).await;
        gateway.await.unwrap();
    }

    /// The eviction deny list is bounded, and the bound drops the **oldest** eviction.
    ///
    /// Proved by pushing past the cap rather than by reading the constant: a list that grows
    /// without limit is its own defect, and the only way to know the bound binds is to exceed it.
    #[test]
    fn the_eviction_list_is_bounded_and_drops_the_oldest_entry() {
        let mut ev = EvictedPeers::default();
        for n in 0..MAX_EVICTED_PEERS as u64 {
            assert!(
                ev.deny(ev_peer(n)).is_none(),
                "no displacement below the cap"
            );
        }
        assert_eq!(ev.len(), MAX_EVICTED_PEERS);
        assert!(ev.is_denied(&ev_peer(0)));

        // One past the cap: the list stays at the cap and the FIRST eviction is the one dropped.
        assert!(ev.deny(ev_peer(MAX_EVICTED_PEERS as u64)).is_none());
        assert_eq!(ev.len(), MAX_EVICTED_PEERS, "the cap binds");
        assert!(
            !ev.is_denied(&ev_peer(0)),
            "the oldest eviction is the one dropped"
        );
        assert!(ev.is_denied(&ev_peer(1)), "the next-oldest survives");
        assert!(
            ev.is_denied(&ev_peer(MAX_EVICTED_PEERS as u64)),
            "the newest eviction is retained"
        );
    }

    /// Displacing an entry whose live connections were closed hands its `libp2p::PeerId` back, so
    /// the caller can lift that block. Without it `allow_block_list`'s set would keep growing
    /// behind this one's bound, which would make the bound cosmetic.
    #[test]
    fn a_displaced_eviction_returns_its_block_to_be_lifted() {
        let mut ev = EvictedPeers::default();
        let first = ev_peer(0);
        let blocked = libp2p_peer();
        ev.deny(first);
        ev.note_blocked(first, blocked);

        let mut displaced = None;
        for n in 1..=MAX_EVICTED_PEERS as u64 {
            if let Some(d) = ev.deny(ev_peer(n)) {
                displaced = Some(d);
            }
        }
        assert_eq!(
            displaced,
            Some(blocked),
            "the displaced entry's block must come back so it can be lifted"
        );

        // An entry that never had a live connection closed has no block to lift.
        let mut ev2 = EvictedPeers::default();
        for n in 0..=MAX_EVICTED_PEERS as u64 {
            assert!(ev2.deny(ev_peer(n)).is_none());
        }
    }

    /// Lifting an eviction removes it from **both** halves: the peer stops being denied, and the
    /// block installed for it comes back to be released. This is the re-invite path; an entry
    /// that survived a lift would make a re-invited member permanently unreachable.
    #[test]
    fn lifting_an_eviction_clears_the_deny_and_returns_the_block() {
        let mut ev = EvictedPeers::default();
        let p = ev_peer(7);
        let blocked = libp2p_peer();
        ev.deny(p);
        ev.note_blocked(p, blocked);
        assert!(ev.is_denied(&p));

        assert_eq!(ev.allow(&p), Some(blocked));
        assert!(!ev.is_denied(&p), "a lifted eviction stops denying");
        assert_eq!(ev.len(), 0);
        assert_eq!(ev.allow(&p), None, "lifting twice is a no-op");

        // A lift must also free the slot in the ordering, or the bound would count ghosts.
        for n in 0..MAX_EVICTED_PEERS as u64 {
            ev.deny(ev_peer(n));
        }
        assert_eq!(ev.len(), MAX_EVICTED_PEERS);
        assert!(ev.is_denied(&ev_peer(0)), "no premature displacement");
    }

    /// Re-evicting an already-evicted peer is a no-op and does not disturb the ordering, so a
    /// repeated removal cannot walk an entry to the front of the bound.
    #[test]
    fn re_denying_an_evicted_peer_does_not_disturb_the_ordering() {
        let mut ev = EvictedPeers::default();
        let p = ev_peer(7);
        ev.deny(p);
        assert!(ev.deny(p).is_none());
        assert_eq!(ev.len(), 1);
    }

    /// The gate recognises an evicted peer from the **libp2p** id the wire supplies, by hashing
    /// it forward to the phase-0 id the membership layer works in. This is the property that lets
    /// the refusal happen at connection time instead of after a connection is established.
    #[test]
    fn the_gate_refuses_an_evicted_peer_by_forward_hashing_the_wire_identity() {
        let mut gate = Eviction::default();
        let victim = libp2p_peer();
        let bystander = libp2p_peer();

        assert!(
            gate.enforce(&victim).is_ok(),
            "nothing is denied by default"
        );

        gate.denied.deny(to_peer(&victim));
        assert!(
            gate.enforce(&victim).is_err(),
            "an evicted peer is refused from its libp2p id alone"
        );
        assert!(
            gate.enforce(&bystander).is_ok(),
            "the refusal is targeted, not a blanket close"
        );

        gate.denied.allow(&to_peer(&victim));
        assert!(
            gate.enforce(&victim).is_ok(),
            "a lifted eviction lets the peer back in"
        );
    }

    /// A circuit address names **two** peers, and which one you resolve decides whether the
    /// relay is protected from eviction.
    ///
    /// `dial_gated` gates on `target_peer_in_multiaddr`, which returns the LAST `/p2p/`, i.e. the
    /// target. So a relay used purely as dial transit was never recorded as infrastructure, and a
    /// companion device claiming its transport id could get every member to evict the relay they
    /// route through by being removed in the ordinary way. `dial_gated` now notes the relay half
    /// as well; this pins the discrimination the whole fix rests on.
    #[test]
    fn a_circuit_address_names_the_relay_and_the_target_separately() {
        let relay = libp2p_peer();
        let target = libp2p_peer();
        let circuit: Multiaddr =
            format!("/ip4/198.51.100.1/tcp/4000/p2p/{relay}/p2p-circuit/p2p/{target}")
                .parse()
                .unwrap();

        assert_eq!(
            target_peer_in_multiaddr(&circuit),
            Some(target),
            "the dial gate resolves the TARGET, which is why the relay needed noting separately"
        );
        assert_eq!(
            relay_peer_in_circuit_addr(&circuit),
            Some(relay),
            "and the relay is recoverable from the same address"
        );

        // A plain direct address has a target and no relay, so noting infra off it is a no-op.
        let direct: Multiaddr = format!("/ip4/198.51.100.1/tcp/4000/p2p/{target}")
            .parse()
            .unwrap();
        assert_eq!(target_peer_in_multiaddr(&direct), Some(target));
        assert_eq!(relay_peer_in_circuit_addr(&direct), None);
    }

    /// Pin the actor boundary, not merely the route parser: an untrusted caller can enqueue a
    /// syntactically valid bare socket through the public runtime handle, but it must never reach
    /// libp2p. A peer-bound form of that same address must still connect normally.
    #[tokio::test]
    async fn the_runtime_dial_actor_refuses_a_bare_socket() {
        let listen: Multiaddr = "/memory/918274".parse().unwrap();
        let mut server_swarm = build_memory_swarm();
        let server_id = *server_swarm.local_peer_id();
        server_swarm.listen_on(listen.clone()).unwrap();
        let server = MeshService::spawn(server_swarm);
        let client = MeshService::new_memory(None, &[]).unwrap();

        client.dial(listen.clone()).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), server.next_event())
                .await
                .is_err(),
            "a bare runtime address must be discarded before the swarm dials it"
        );

        let bound: Multiaddr = format!("{listen}/p2p/{server_id}").parse().unwrap();
        client.dial(bound).await.unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), server.next_event())
            .await
            .expect("the peer-bound control route should connect")
            .expect("the server actor should remain alive");
        assert!(matches!(event, TransportEvent::PeerConnected(_)));
    }

    /// Pre-owner waits use the watch table, not the single-consumer event queue. The returned row
    /// must already contain its coarse path, while the future sync owner still receives both
    /// ordered edges. Dropping the remote actor must then remove the row rather than retaining a
    /// stale success.
    #[tokio::test]
    async fn connection_watch_wait_preserves_ordered_events_and_withdraws_on_close() {
        let listen: Multiaddr = "/memory/918276".parse().unwrap();
        let mut server_swarm = build_memory_swarm();
        let server_id = *server_swarm.local_peer_id();
        server_swarm.listen_on(listen.clone()).unwrap();
        let server = MeshService::spawn(server_swarm);
        let client = MeshService::new_memory(None, &[]).unwrap();
        let server_peer = server.local_peer();

        let bound: Multiaddr = format!("{listen}/p2p/{server_id}").parse().unwrap();
        client.dial(bound).await.unwrap();
        let connected = tokio::time::timeout(
            Duration::from_secs(2),
            client.wait_for_peer_connected(server_peer),
        )
        .await
        .expect("connection watch should change")
        .expect("client actor should remain alive");
        assert_eq!(connected.peer, server_peer);
        assert_eq!(connected.active.len(), 1);
        assert_eq!(connected.active[0].transport, ConnectionTransport::Memory);

        assert!(
            matches!(
                client.next_event().await,
                Some(TransportEvent::PeerConnected(peer)) if peer == server_peer
            ),
            "the watch must not consume the aggregate edge"
        );
        assert!(matches!(
            client.next_event().await,
            Some(TransportEvent::PeerPathsChanged { peer, active, .. })
                if peer == server_peer && active == connected.active
        ));

        let mut snapshots = client.connection_snapshot_rx.clone();
        drop(server);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                snapshots
                    .changed()
                    .await
                    .expect("client actor remains alive");
                if snapshots.borrow_and_update().is_empty() {
                    break;
                }
            }
        })
        .await
        .expect("final close should withdraw the watch row");
    }

    #[test]
    fn a_closed_connection_watch_never_returns_its_retained_last_success() {
        let peer = PeerId::from_u64(77);
        let path = ConnectionPath {
            family: ConnectionFamily::Memory,
            transport: ConnectionTransport::Memory,
            direction: ConnectionDirection::Dialer,
        };
        let connected = vec![PeerConnectionSnapshot {
            peer,
            active: vec![path],
        }];
        let (tx, mut rx) = watch::channel(connected.clone());
        tx.send_replace(Vec::new());
        assert!(
            current_connection_snapshot(&mut rx).unwrap().is_empty(),
            "a pending disconnect replacement wins over the retained success"
        );
        tx.send_replace(connected);
        drop(tx);

        assert!(matches!(
            current_connection_snapshot(&mut rx),
            Err(TransportError::Closed)
        ));
    }

    /// Reciprocal proof retries are allowed to continue after the endpoint scheduler refuses a
    /// new socket pass, so their request command must distinguish a live connection from the
    /// actor's ordinary recent-peer auto-redial cache.
    #[tokio::test]
    async fn connected_only_control_requests_never_redial_a_recent_peer() {
        let listen: Multiaddr = "/memory/918275".parse().unwrap();
        let mut server_swarm = build_memory_swarm();
        let server_id = *server_swarm.local_peer_id();
        server_swarm.listen_on(listen.clone()).unwrap();
        let server = MeshService::spawn(server_swarm);
        let client = MeshService::new_memory(None, &[]).unwrap();
        let server_peer = server.local_peer();
        let client_peer = client.local_peer();

        let bound: Multiaddr = format!("{listen}/p2p/{server_id}").parse().unwrap();
        client.dial(bound).await.unwrap();
        for node in [&client, &server] {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if matches!(
                        node.next_event().await,
                        Some(TransportEvent::PeerConnected(_))
                    ) {
                        break;
                    }
                }
            })
            .await
            .expect("the memory peers should connect");
        }

        // Positive half: a live connection carries the proof without any additional dial.
        let handle = client.handle();
        let request = tokio::spawn(async move {
            handle
                .request_control_connected_only(server_peer, Bytes::from_static(b"proof"))
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(TransportEvent::Request {
                    data, responder, ..
                }) = server.next_event().await
                {
                    assert_eq!(&data[..], b"proof");
                    responder.respond(Bytes::from_static(b"ok"));
                    break;
                }
            }
        })
        .await
        .expect("the connected-only request should reach the live peer");
        assert_eq!(&request.await.unwrap().unwrap()[..], b"ok");

        // Populate the client's recent-peer cache by severing the established connection. The
        // server lifts its deny immediately afterwards so any accidental redial would be visible.
        server.evict_peer(client_peer).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(
                    client.next_event().await,
                    Some(TransportEvent::PeerDisconnected(peer)) if peer == server_peer
                ) {
                    break;
                }
            }
        })
        .await
        .expect("the client should observe the severed connection");
        server.unevict_peer(client_peer).await.unwrap();

        let outcome = tokio::time::timeout(
            Duration::from_secs(1),
            client
                .handle()
                .request_control_connected_only(server_peer, Bytes::from_static(b"again")),
        )
        .await
        .expect("a disconnected proof is rejected immediately");
        assert!(matches!(outcome, Err(TransportError::Unreachable(peer)) if peer == server_peer));
        // Path detail is emitted after the aggregate disconnect edge, so an empty
        // `ConnectionPathsChanged` may still be queued here. That refinement is not a redial; the
        // contract this regression protects is that no new aggregate connection edge appears.
        let reconnected = tokio::time::timeout(Duration::from_millis(150), async {
            loop {
                match client.next_event().await {
                    Some(TransportEvent::PeerConnected(peer)) if peer == server_peer => break,
                    Some(_) => continue,
                    None => std::future::pending::<()>().await,
                }
            }
        })
        .await;
        assert!(
            reconnected.is_err(),
            "the connected-only request must not reconnect from recent_peers"
        );
    }

    #[test]
    fn a_seed_pins_a_stable_peer_id() {
        // The whole point of the persisted identity: the same 32 bytes must reproduce the same
        // PeerId on every launch, or every invite that embedded `/p2p/<id>` dies on restart.
        let seed = [7u8; 32];
        let a = keypair_from_seed(seed).unwrap().public().to_peer_id();
        let b = keypair_from_seed(seed).unwrap().public().to_peer_id();
        assert_eq!(a, b, "the same seed must reproduce the same peer id");

        // And a different seed is a different node (no accidental collapse to one identity,
        // which would defeat the per-server identity separation the app relies on).
        let mut other = [7u8; 32];
        other[0] = 8;
        let c = keypair_from_seed(other).unwrap().public().to_peer_id();
        assert_ne!(a, c);
    }

    #[test]
    fn rendezvous_addresses_are_validated() {
        // A well-formed direct address with exactly one /p2p/ passes.
        let good =
            "/ip4/198.51.100.1/tcp/5000/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN";
        let out = validate_rendezvous_addrs(&[good.to_string()]).unwrap();
        assert_eq!(out.len(), 1);
        // A circuit address, a missing peer id, and a duplicate node are all refused.
        assert!(validate_rendezvous_addrs(&[format!("{good}/p2p-circuit")]).is_err());
        assert!(validate_rendezvous_addrs(&["/ip4/198.51.100.1/tcp/5000".to_string()]).is_err());
        assert!(validate_rendezvous_addrs(&[good.to_string(), good.to_string()]).is_err());
    }

    /// A deterministic peer id (no ambient RNG); `n` picks which one.
    fn test_peer(n: u8) -> libp2p::PeerId {
        let mut seed = [3u8; 32];
        seed[0] = n;
        keypair_from_seed(seed).unwrap().public().to_peer_id()
    }

    #[test]
    fn the_operator_variant_still_accepts_a_dns_name() {
        // Rung 4 of the connectivity ladder is a TLS/WebSocket listener on TCP/443, and the
        // whole point of it is that clients dial it by NAME. If the operator variant ever
        // starts rejecting names, rung 4 is broken, so pin it here rather than find out in
        // the field.
        let id = test_peer(1);
        for name in [
            format!("/dns4/rz.example.org/tcp/443/tls/ws/p2p/{id}"),
            format!("/dns6/rz.example.org/tcp/443/tls/ws/p2p/{id}"),
            format!("/dns/rz.example.org/tcp/443/tls/ws/p2p/{id}"),
        ] {
            assert!(
                validate_operator_rendezvous_addrs(std::slice::from_ref(&name)).is_ok(),
                "the operator may configure {name}"
            );
        }
        // The operator is also entitled to point their own node at their own LAN.
        assert!(validate_operator_rendezvous_addrs(&[format!(
            "/ip4/192.168.1.5/tcp/5000/p2p/{id}"
        )])
        .is_ok());
        // And the structural checks still apply to the operator too.
        assert!(
            validate_operator_rendezvous_addrs(&["/ip4/45.79.12.34/tcp/5000".to_string()]).is_err()
        );
    }

    #[test]
    fn the_invite_variant_keeps_the_structural_checks() {
        let id1 = test_peer(1);
        let id2 = test_peer(2);
        let good = format!("/ip4/45.79.12.34/tcp/5000/p2p/{id1}");
        assert_eq!(
            validate_invite_rendezvous_addrs(std::slice::from_ref(&good))
                .unwrap()
                .len(),
            1
        );
        // Circuit, no peer id, two peer ids, duplicate node: refused exactly as before, so the
        // split did not lose a check.
        assert!(validate_invite_rendezvous_addrs(&[format!("{good}/p2p-circuit")]).is_err());
        assert!(
            validate_invite_rendezvous_addrs(&["/ip4/45.79.12.34/tcp/5000".to_string()]).is_err()
        );
        assert!(validate_invite_rendezvous_addrs(&[format!("{good}/p2p/{id2}")]).is_err());
        assert!(validate_invite_rendezvous_addrs(&[good.clone(), good]).is_err());
    }

    #[test]
    fn the_invite_variant_refuses_names_and_unroutable_literals() {
        let id = test_peer(1);
        // A name resolves at dial time to whatever the invite's author currently points it at.
        // With the rung-4 DNS transport wired into the client this is no longer a theoretical
        // hole: it resolves and dials, so an inviter can rotate an A record per query and have
        // every joiner sweep its own LAN. This is defect P13.
        for name in [
            "/dns4/scan.attacker.invalid/tcp/22",
            "/dns6/scan.attacker.invalid/tcp/22",
            "/dns/scan.attacker.invalid/tcp/22",
            "/dnsaddr/scan.attacker.invalid",
        ] {
            let a = format!("{name}/p2p/{id}");
            assert!(
                validate_invite_rendezvous_addrs(std::slice::from_ref(&a)).is_err(),
                "{a} must not survive an invite"
            );
        }

        for host in [
            // Private (RFC1918), the LAN-sweep payload itself.
            "/ip4/192.168.1.5",
            "/ip4/10.0.0.1",
            "/ip4/172.16.4.4",
            // Link-local: a different machine on every network the invite is opened on.
            "/ip4/169.254.1.1",
            "/ip6/fe80::1",
            // Carrier-grade NAT, RFC 6598.
            "/ip4/100.64.0.1",
            "/ip4/100.127.255.254",
            // IPv6 unique-local.
            "/ip6/fd00::1",
            "/ip6/fc00::1",
            // Multicast: not an endpoint at all.
            "/ip4/224.0.0.1",
            "/ip4/239.255.255.250",
            "/ip6/ff02::1",
            // Reserved and unspecified.
            "/ip4/0.0.0.0",
            "/ip4/0.1.2.3",
            "/ip4/240.0.0.1",
            "/ip4/255.255.255.255",
            "/ip6/::",
            // Documentation and benchmark ranges, reserved precisely so they never route.
            "/ip4/192.0.2.1",
            "/ip4/198.51.100.1",
            "/ip4/203.0.113.7",
            "/ip4/198.18.0.1",
            "/ip6/2001:db8::1",
        ] {
            let a = format!("{host}/tcp/5000/p2p/{id}");
            assert!(
                validate_invite_rendezvous_addrs(std::slice::from_ref(&a)).is_err(),
                "{a} must not survive an invite"
            );
        }

        // Two genuinely public literals are what an invite is supposed to carry.
        for host in ["/ip4/45.79.12.34", "/ip6/2606:4700::1111"] {
            let a = format!("{host}/tcp/5000/p2p/{id}");
            assert!(
                validate_invite_rendezvous_addrs(std::slice::from_ref(&a)).is_ok(),
                "{a} is a normal public rendezvous address"
            );
        }
    }

    #[test]
    fn the_invite_variant_is_not_fooled_by_ipv4_in_ipv6() {
        // The mistake this pins: `Ipv6Addr::is_loopback()` is FALSE for `::ffff:127.0.0.1`, so
        // testing the v6 properties directly lets every v4 rule be dodged by writing the same
        // address in the other family. And the fold has to be `to_ipv4_mapped`, not `to_ipv4`:
        // the latter also matches the deprecated `::a.b.c.d` form, which reads `::1` as
        // "0.0.0.1" and misclassifies it.
        let id = test_peer(1);
        let id2 = test_peer(2);
        for host in [
            "/ip6/::ffff:192.168.1.5",
            "/ip6/::ffff:10.0.0.1",
            "/ip6/::ffff:169.254.1.1",
            "/ip6/::ffff:224.0.0.1",
            "/ip6/::ffff:100.64.0.1",
            // The deprecated IPv4-compatible spelling of the same dodge.
            "/ip6/::192.168.1.5",
        ] {
            let a = format!("{host}/tcp/5000/p2p/{id}");
            assert!(
                validate_invite_rendezvous_addrs(std::slice::from_ref(&a)).is_err(),
                "{a} is a v4 address in v6 clothing"
            );
        }
        // A mapped loopback is loopback, so it is judged by the same-machine rule rather than by
        // the range rules: on its own it is the genuine two-instances-on-one-box case...
        assert!(validate_invite_rendezvous_addrs(&[format!(
            "/ip6/::ffff:127.0.0.1/tcp/5000/p2p/{id}"
        )])
        .is_ok());
        // ...and mixed with a routable address it is refused, which is the case that matters:
        // this is the exact spelling that defeated an earlier validator and had the joiner dial
        // its own localhost while ignoring the real addresses.
        assert!(validate_invite_rendezvous_addrs(&[
            format!("/ip6/::ffff:127.0.0.1/tcp/5000/p2p/{id}"),
            format!("/ip4/45.79.12.34/tcp/5000/p2p/{id2}"),
        ])
        .is_err());
        // `::1` itself is loopback (not "0.0.0.1", which is what `to_ipv4` would have made of it),
        // so the same-machine exemption applies to it too.
        assert!(validate_invite_rendezvous_addrs(&[format!("/ip6/::1/tcp/5000/p2p/{id}")]).is_ok());
        assert!(validate_invite_rendezvous_addrs(&[
            format!("/ip6/::1/tcp/5000/p2p/{id}"),
            format!("/ip4/45.79.12.34/tcp/5000/p2p/{id2}"),
        ])
        .is_err());
    }

    #[test]
    fn the_invite_variant_refuses_the_transitional_ranges() {
        // The same dodge by a longer route: each of these embeds an IPv4 address that the host
        // stack unwraps, so a routable-LOOKING v6 literal still reaches a private v4 target and
        // none of the range checks would have seen it. Nothing this product mints publishes one.
        let id = test_peer(1);
        for host in [
            // 2002::/16, 6to4. `2002:c0a8:0101::` is 192.168.1.1.
            "/ip6/2002:c0a8:0101::1",
            "/ip6/2002:2d4f:0c22::1",
            // 64:ff9b::/96 and 64:ff9b:1::/48, NAT64.
            "/ip6/64:ff9b::c0a8:105",
            "/ip6/64:ff9b:1::1",
            // 2001:0::/32, Teredo.
            "/ip6/2001:0:4136:e378:8000:63bf:3fff:fdd2",
        ] {
            let a = format!("{host}/tcp/5000/p2p/{id}");
            assert!(
                validate_invite_rendezvous_addrs(std::slice::from_ref(&a)).is_err(),
                "{a} is a transitional range"
            );
        }
    }

    #[test]
    fn the_invite_variant_allows_loopback_only_when_the_whole_set_is_loopback() {
        let id1 = test_peer(1);
        let id2 = test_peer(2);
        // The genuine same-machine case: two instances on one dev box, and the real-socket
        // end-to-end tests, which bind loopback and carry it in an invite. A joiner elsewhere
        // finds nothing at its own localhost, so this cannot be aimed at anyone.
        let all_loopback = vec![
            format!("/ip4/127.0.0.1/tcp/5000/p2p/{id1}"),
            format!("/ip6/::1/tcp/5001/p2p/{id2}"),
        ];
        assert_eq!(
            validate_invite_rendezvous_addrs(&all_loopback)
                .unwrap()
                .len(),
            2
        );

        // Mixed with anything routable it is refused: it is not a fallback for the routable
        // address, it can only ever probe ports on the reader's own machine.
        let mixed = vec![
            format!("/ip4/127.0.0.1/tcp/5000/p2p/{id1}"),
            format!("/ip4/45.79.12.34/tcp/5000/p2p/{id2}"),
        ];
        assert!(validate_invite_rendezvous_addrs(&mixed).is_err());

        // The exemption is over the WHOLE set, so it cannot be unlocked by a set of one
        // loopback plus one private address either.
        let sneaky = vec![
            format!("/ip4/127.0.0.1/tcp/5000/p2p/{id1}"),
            format!("/ip4/192.168.1.5/tcp/5000/p2p/{id2}"),
        ];
        assert!(validate_invite_rendezvous_addrs(&sneaky).is_err());
    }

    #[test]
    fn the_invite_variant_requires_a_literal_to_judge() {
        // An address with no host component at all cannot be classified, and "classified later,
        // by the dialer" is the hole. The memory transport is test-only and never appears in a
        // real invite, so refusing it costs nothing.
        let id = test_peer(1);
        assert!(validate_invite_rendezvous_addrs(&[format!("/memory/1234/p2p/{id}")]).is_err());
        // An empty vector is not an error; it means "this invite names no rendezvous", which
        // each caller reports in its own words.
        assert!(validate_invite_rendezvous_addrs(&[]).unwrap().is_empty());
    }
}
