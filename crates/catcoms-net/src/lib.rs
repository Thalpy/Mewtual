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
pub mod fdlimit;
pub mod infra_transport;
pub mod metering;
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
use std::pin::Pin;
use std::task::{Context, Poll};

use async_trait::async_trait;
use bytes::Bytes;
use catcoms_rt::{
    DiscoveredPeer, MeshTransport, PeerId, ProtocolId, Responder, Topic, TransportError,
    TransportEvent,
};
use futures::stream::FuturesUnordered;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, StreamExt};
use libp2p::core::transport::MemoryTransport;
use libp2p::core::transport::PortUse;
use libp2p::core::upgrade::Version;
use libp2p::core::Endpoint;
use libp2p::multiaddr::Protocol;
use libp2p::request_response::{OutboundRequestId, ResponseChannel};
use libp2p::swarm::dial_opts::{DialOpts, PeerCondition};
use libp2p::swarm::{
    dummy, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, SwarmEvent, THandler,
    THandlerInEvent, THandlerOutEvent, ToSwarm,
};
use libp2p::{
    allow_block_list, connection_limits, dcutr, gossipsub, identify, noise, relay, rendezvous,
    request_response, yamux, Multiaddr, StreamProtocol, Swarm, SwarmBuilder, Transport,
};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot, Mutex};

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
    /// UPnP/NAT-PMP: best-effort ask the home router to open a port, so this node becomes directly
    /// reachable (the discovered public address is advertised + folded into a fresh invite); no
    /// relay needed when the router cooperates.
    pub upnp: libp2p::upnp::tokio::Behaviour,
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
        let identify = identify::Behaviour::new(identify_config(key));
        let rendezvous_client = rendezvous::client::Behaviour::new(key.clone());
        let upnp = libp2p::upnp::tokio::Behaviour::default();
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
    Ok(SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_other_transport(infra_transport::client_transport)
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_behaviour(MeshBehaviour::new)
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
    /// Start listening on `addr` (e.g. a `…/p2p-circuit` relay reservation).
    Listen(Multiaddr),
    /// Dial `addr` (e.g. a relay, before reserving a circuit on it).
    Dial(Multiaddr),
    /// Advertise `addr` as an external (reachable) address; for a node with a
    /// directly-reachable address (a public IP, or a memory listener in tests) that
    /// does not need a relay circuit to be registerable at a rendezvous. Flushes any
    /// deferred registrations.
    AddExternalAddress(Multiaddr),
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

struct Actor {
    swarm: Swarm<MeshBehaviour>,
    cmd_rx: mpsc::Receiver<Command>,
    event_tx: mpsc::Sender<TransportEvent>,
    listen_tx: mpsc::Sender<Multiaddr>,
    /// UPnP outcome: `Some(addr)` = the router opened a port and reported this public address
    /// (surfaced so a fresh invite can carry a directly-dialable bootstrap, no relay);
    /// `None` = no usable gateway (so a waiter stops promptly instead of timing out).
    upnp_tx: mpsc::Sender<Option<Multiaddr>>,
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
                    // No subscribers known yet; retry when one appears.
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
                // Listening on a `…/p2p-circuit` address is how a reservation is requested, so the
                // relay named in it is an infra target from here on.
                if let Some(relay) = relay_peer_in_circuit_addr(&addr) {
                    self.note_infra(relay);
                }
                if let Err(e) = self.swarm.listen_on(addr.clone()) {
                    tracing::warn!(%addr, error = %e, "listen failed");
                }
            }
            Command::Dial(addr) => self.dial_gated(addr),
            Command::AddExternalAddress(addr) => {
                tracing::debug!(%addr, "add external address");
                self.swarm.add_external_address(addr);
                // A registration may have been waiting on exactly this.
                self.flush_pending_registers();
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
    /// An address with no `/p2p/<id>` names no peer, so it cannot be gated; it is dialed as
    /// before. (The jitter half of P11 lives in the caller that drives the timer.)
    fn dial_gated(&mut self, addr: Multiaddr) {
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
            if let Err(e) = self.swarm.dial(addr.clone()) {
                tracing::warn!(%addr, error = %e, "dial failed");
            }
            return;
        };
        if self.infra_peers.contains(&target) {
            self.dial_infra(target, addr);
            return;
        }
        if self.covered_addrs.len() > MAX_DIAL_LEDGER_PEERS {
            // Entries are dropped when a peer disconnects or a dial fails, so this is the
            // pathological case rather than the normal one. Clearing wholesale costs at worst a
            // duplicate dial attempt.
            tracing::debug!("dial ledger over its cap; clearing");
            self.covered_addrs.clear();
        }
        if !self
            .covered_addrs
            .entry(target)
            .or_default()
            .insert(addr.clone())
        {
            tracing::trace!(%addr, peer = %target, "dial suppressed: this address is already dialing or connected");
            return;
        }
        self.pending_dials.entry(target).or_default().push(addr);
    }

    /// Dial an infra target under the strict one-connection-per-peer condition.
    fn dial_infra(&mut self, target: libp2p::PeerId, addr: Multiaddr) {
        if self.swarm.is_connected(&target) {
            tracing::trace!(%addr, peer = %target, "infra dial suppressed: already connected");
            return;
        }
        let opts = DialOpts::peer_id(target)
            .addresses(vec![addr.clone()])
            .condition(PeerCondition::DisconnectedAndNotDialing)
            .build();
        match self.swarm.dial(opts) {
            Ok(()) => tracing::debug!(%addr, peer = %target, "dialing infra node"),
            // `DialError::DialPeerConditionFalse` is the condition doing its job (a dial to this
            // peer is already in flight), not a failure worth warning about.
            Err(libp2p::swarm::DialError::DialPeerConditionFalse(cond)) => {
                tracing::trace!(%addr, peer = %target, ?cond, "infra dial suppressed: already dialing");
            }
            Err(e) => {
                // Drop the ledger entry so the next tick retries. A dial refused *before* it
                // starts (`max_pending_outgoing` at its cap, say) produces no
                // `OutgoingConnectionError`, so nothing else would ever clear it and the node
                // would stop dialing its own relay or rendezvous for the rest of the process.
                // `flush_dials` already does this for member peers.
                tracing::warn!(%addr, error = %e, "dial failed");
                self.release_failed_dial(Some(target), &e);
            }
        }
    }

    /// Issue one racing dial per member peer whose addresses accumulated during this drain.
    ///
    /// `PeerCondition::Always` is correct here precisely because the gate above already decided:
    /// every address in the batch is one libp2p is not currently trying, and a peer we are already
    /// connected to may still be worth dialing at a *new* address (the relay-to-direct upgrade).
    fn flush_dials(&mut self) {
        for (peer, addrs) in std::mem::take(&mut self.pending_dials) {
            let count = addrs.len();
            let opts = DialOpts::peer_id(peer)
                .addresses(addrs)
                .condition(PeerCondition::Always)
                .build();
            match self.swarm.dial(opts) {
                Ok(()) => tracing::debug!(peer = %peer, addresses = count, "dialing"),
                Err(e) => {
                    tracing::warn!(peer = %peer, error = %e, "dial failed");
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
                if self.peers.insert(peer, peer_id).is_none() {
                    let _ = self
                        .event_tx
                        .send(TransportEvent::PeerConnected(peer))
                        .await;
                }
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                // The attempt is over, so a later tick may retry rather than being suppressed by a
                // ledger entry that nothing would ever clear.
                self.release_failed_dial(peer_id, &error);
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                num_established,
                ..
            } => {
                if num_established == 0 {
                    tracing::debug!(peer = %peer_id, "peer disconnected");
                    // Nothing covers this peer any more, so the next tick is free to dial it.
                    self.clear_dial_ledger(&peer_id);
                    let peer = to_peer(&peer_id);
                    self.peers.remove(&peer);
                    // Always paired: an evicted peer is refused by `Eviction` before a connection
                    // is ever established, so it cannot reach this arm without having been
                    // announced as connected first.
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
            // identify drives DCUtR's external-address candidates (and a relay learns
            // a client's addresses through it). Noisy; trace only.
            SwarmEvent::Behaviour(MeshBehaviourEvent::Identify(e)) => {
                tracing::trace!(?e, "identify event");
            }
            // Relay-client lifecycle (reservation accepted/expired, circuit opened).
            SwarmEvent::Behaviour(MeshBehaviourEvent::RelayClient(e)) => {
                tracing::debug!(?e, "relay-client event");
            }
            // UPnP/NAT-PMP: the router mapped our port and told us our public address. Promote it
            // to an external address (so identify/rendezvous advertise it) and surface it so a
            // fresh invite can carry a directly-dialable bootstrap; direct connect, no relay.
            SwarmEvent::Behaviour(MeshBehaviourEvent::Upnp(e)) => match e {
                libp2p::upnp::Event::NewExternalAddr(addr) => {
                    tracing::info!(%addr, "UPnP mapped a public address; node is directly reachable");
                    self.swarm.add_external_address(addr.clone());
                    self.flush_pending_registers();
                    let _ = self.upnp_tx.try_send(Some(addr));
                }
                libp2p::upnp::Event::ExpiredExternalAddr(addr) => {
                    tracing::info!(%addr, "UPnP port mapping expired");
                    self.swarm.remove_external_address(&addr);
                }
                libp2p::upnp::Event::GatewayNotFound => {
                    tracing::info!("no UPnP gateway found; direct reachability needs a port-forward or a relay");
                    let _ = self.upnp_tx.try_send(None);
                }
                libp2p::upnp::Event::NonRoutableGateway => {
                    tracing::info!("UPnP gateway is not internet-routable (likely CGNAT/double-NAT); a relay is required");
                    let _ = self.upnp_tx.try_send(None);
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

// ----- handle ----------------------------------------------------------------

/// A handle to a running libp2p mesh node, implementing [`MeshTransport`].
#[derive(Debug)]
pub struct MeshService {
    local: PeerId,
    cmd_tx: mpsc::Sender<Command>,
    event_rx: Mutex<mpsc::Receiver<TransportEvent>>,
    listen_rx: Mutex<mpsc::Receiver<Multiaddr>>,
    /// `None` once [`MeshService::take_external_addrs`] has handed the receiver to a background
    /// waiter (UPnP discovery outlives any call a UI can block on).
    upnp_rx: Mutex<Option<mpsc::Receiver<Option<Multiaddr>>>>,
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
        Self::spawn_protecting(swarm, Vec::new())
    }

    /// [`MeshService::spawn`], plus the peers this node was **constructed to dial** (its
    /// bootstrap/infra set). Those are never deniable by an eviction: see `Actor::protected`.
    fn spawn_protecting(swarm: Swarm<MeshBehaviour>, protected: Vec<libp2p::PeerId>) -> Self {
        let local = to_peer(swarm.local_peer_id());
        let (cmd_tx, cmd_rx) = mpsc::channel(256);
        let (event_tx, event_rx) = mpsc::channel(256);
        let (listen_tx, listen_rx) = mpsc::channel(16);
        let (upnp_tx, upnp_rx) = mpsc::channel(16);
        let (upgrade_tx, upgrade_rx) = mpsc::channel(16);
        let (discovered_tx, discovered_rx) = mpsc::unbounded_channel();
        let (registered_tx, registered_rx) = mpsc::unbounded_channel();
        let actor = Actor {
            swarm,
            cmd_rx,
            event_tx,
            listen_tx,
            upnp_tx,
            upgrade_tx,
            discovered_tx,
            registered_tx,
            pending_registers: Vec::new(),
            peers: HashMap::new(),
            pending_req: HashMap::new(),
            pending_publish: Vec::new(),
            infra_peers: HashSet::new(),
            covered_addrs: HashMap::new(),
            pending_dials: HashMap::new(),
            protected: protected.into_iter().collect(),
        };
        tokio::spawn(actor.run());
        Self {
            local,
            cmd_tx,
            event_rx: Mutex::new(event_rx),
            listen_rx: Mutex::new(listen_rx),
            upnp_rx: Mutex::new(Some(upnp_rx)),
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

    /// Await the next public address discovered via **UPnP** (the home router opened a port and
    /// reported our internet-facing address). It is already added as an external address; callers
    /// (e.g. invite minting) can fold it into a directly-dialable bootstrap so a peer can connect
    /// with no relay. UPnP is best-effort: this resolves to `None` both when there is no usable
    /// gateway (signalled promptly, so the caller doesn't wait out a full timeout) and once the
    /// actor stops; either way the caller simply proceeds without a UPnP bootstrap. Also `None`
    /// once [`MeshService::take_external_addrs`] has moved the channel to a background waiter.
    pub async fn next_external_addr(&self) -> Option<Multiaddr> {
        let mut guard = self.upnp_rx.lock().await;
        guard.as_mut()?.recv().await.flatten()
    }

    /// Take ownership of the UPnP external-address channel so a **background** task can wait out a
    /// realistic router-discovery window after this `MeshService` has been moved into the layers
    /// above it. SSDP/IGD discovery routinely needs tens of seconds, which is far longer than
    /// founding a server may block for; the answer therefore has to be collected by somebody who
    /// outlives the call. Returns the receiver on the first call and `None` after that, so exactly
    /// one consumer ever sees each address; [`MeshService::next_external_addr`] then yields `None`.
    pub async fn take_external_addrs(&self) -> Option<mpsc::Receiver<Option<Multiaddr>>> {
        self.upnp_rx.lock().await.take()
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
        self.cmd_tx
            .send(Command::Dial(addr))
            .await
            .map_err(|_| TransportError::Closed)
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
        Ok(Self::spawn_protecting(swarm, protected))
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
        let (svc, id, _bound) = Self::listen_dial_spawn(swarm, &listen, dial)?;
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
        Self::listen_dial_spawn(swarm, listen, dial)
    }

    /// Apply the listen/dial set to a freshly-built swarm and spawn its actor. Shared by the
    /// identity-less and persisted-identity constructors so they cannot drift.
    fn listen_dial_spawn(
        mut swarm: Swarm<MeshBehaviour>,
        listen: &[Multiaddr],
        dial: &[Multiaddr],
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
        Ok((Self::spawn_protecting(swarm, protected), libp2p_id, bound))
    }

    /// A cheap, clonable [`MeshHandle`] to this node's command channel, for driving rendezvous
    /// register/dial **after** the `MeshService` has been moved elsewhere (e.g. into a server
    /// actor); the desktop bridge keeps one to register a fresh invite's namespace post-spawn.
    pub fn handle(&self) -> MeshHandle {
        MeshHandle {
            local: self.local,
            cmd_tx: self.cmd_tx.clone(),
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
}

impl MeshHandle {
    /// This node's transport peer id.
    pub fn local_peer(&self) -> PeerId {
        self.local
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
            .send(Command::Dial(addr))
            .await
            .map_err(|_| TransportError::Closed)
    }

    /// See [`MeshService::add_external_address`].
    pub async fn add_external_address(&self, addr: Multiaddr) -> Result<(), TransportError> {
        self.cmd_tx
            .send(Command::AddExternalAddress(addr))
            .await
            .map_err(|_| TransportError::Closed)
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

    /// A helper peer id distinct per `n`.
    fn ev_peer(n: u64) -> PeerId {
        PeerId::from_u64(n)
    }

    fn libp2p_peer() -> libp2p::PeerId {
        libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id()
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
