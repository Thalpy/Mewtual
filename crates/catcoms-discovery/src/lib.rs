//! Pure, deterministic **discovery policy**: turn a pile of discovered peer
//! candidates into a bounded, eclipse-resistant **dial plan**.
//!
//! This crate is the single place that decides *what to dial*. The libp2p Actor
//! (catcoms-net) never auto-dials; it surfaces every signed peer record on a
//! never-dropping queue, and a higher layer feeds those records (plus PEX entries
//! and cross-session cache entries) here. The policy:
//!
//! - **unions** candidates for the same peer across sources, taking the freshest
//!   record and merging addresses,
//! - judges **freshness off the registrant's own signed sequence number** (never a
//!   server-asserted TTL; a colluding rendezvous lies about TTL), dropping a record
//!   whose seq we have already bettered (stale / replayed),
//! - **ranks** peers so a member-tag-verified peer leads, then multi-source
//!   corroboration, then a prior proven contact from the cache, then raw single-
//!   rendezvous candidates (the junk/flood) last; but *never drops* the junk, only
//!   sinks it. (The tag-verified tier is **inert in this workspace**: nothing sets
//!   `Candidate::tag_verified`, by decision rather than omission; see that field's docs,)
//! - counts **≤ 1 trust root per rendezvous** (two colluding rendezvous cannot
//!   manufacture independent corroboration) and **round-robin interleaves** equal-rank
//!   peers across their source so **a single rendezvous** flooding thousands of
//!   records cannot dominate the dial order,
//! - **clamps** the plan to roughly the roster size (you never need to dial more
//!   distinct peers than could plausibly be members), and
//! - meters endpoint attempts against a **Clock-paced, RNG-jittered budget** shared across all
//!   discovery sources, so junk costs at most `B` socket targets per window.
//!
//! It **ranks only; it never gates messaging** and never makes a network call. No
//! ambient time/RNG: a `Clock` and an RNG are injected on every `plan` call, exactly
//! like the rest of the stack, so the whole thing is deterministically testable.
//!
//! The round-robin guarantee is scoped to a **single** rendezvous. Distinct *colluding*
//! rendezvous each earn one front-of-line slot among equal-rank peers; but a
//! verified/cache/PEX honest peer outranks all unverified rendezvous junk by score and
//! still leads, so only an honest yet *unverified, uncorroborated, uncached* peer is
//! pushed back, and that is the documented all-rendezvous-colluding residual (answered
//! by cache + PEX + the membership tag). The caller bounds it further by admitting only
//! records from the invite's fixed rendezvous set.

use std::{
    collections::{BTreeMap, HashMap},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::{Arc, Mutex},
};

use catcoms_rt::{Clock, CryptoRngCore, SystemClock};
use multiaddr::{Multiaddr, Protocol};

mod cache;
mod eclipse;

pub use cache::{AddressCache, CacheConfig, CacheError, CachedPeer};
pub use eclipse::{EclipseConfig, EclipseDetector, EclipseLevel, EclipseObservation};

/// An opaque peer identifier; a libp2p `PeerId`'s bytes (or any stable id). Kept as
/// a `Vec<u8>` so this crate stays free of a libp2p dependency and fully pure.
pub type PeerKey = Vec<u8>;

/// The authenticated signer whose monotonic sequence domain a candidate belongs to.
///
/// A cached PEX `PeerDescriptor` is signed by a member device even though it may self-assert a
/// different transport peer. A rendezvous `PeerRecord` is signed by the transport peer itself.
/// Keeping those domains distinct prevents either signer from pinning or resetting the other's
/// anti-replay high-water while candidates still merge and dial by [`Candidate::peer`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum FreshnessPrincipal {
    Device(PeerKey),
    Transport(PeerKey),
}

/// Where a candidate came from; its **trust-root class**. Eclipse-resistance counts
/// *distinct* roots: every rendezvous is at most one root (so two colluding
/// rendezvous cannot fake corroboration); each PEX-vouching member is one root; a
/// cache entry is a prior proven contact (it becomes a counted root only via the live
/// re-proof it later enables, which is the eclipse detector's concern, not ours).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    /// Discovered at a rendezvous node, identified by its peer-id bytes.
    Rendezvous(PeerKey),
    /// Vouched for by a member over PEX, identified by the voucher's device id.
    Pex(PeerKey),
    /// Loaded from the cross-session address cache (a previously proven member).
    Cache,
}

/// One discovered candidate, before merging. An unverified candidate is never dropped
/// here, only ranked last.
///
/// NOTE (2026-08-19): **no caller in this workspace sets `tag_verified`, and the decision is
/// that none will.** The tag it names is never carried on the wire (see
/// `catcoms_sync::routing_membership_tag` for why the libp2p `PeerRecord` cannot carry it and
/// why doing so anyway would be a new disclosure), and no call site could act on it if it were:
/// the pre-join discovery path is the only one that ranks several candidates against each other
/// and it holds no group secret to recompute a tag with, while the post-join path feeds `plan`
/// one candidate at a time, where a score orders nothing. The field and
/// [`SCORE_TAG_VERIFIED`](self) stay because the ranking tier is correct and costs nothing
/// inert; do not read them as a live signal.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// The canonical transport peer to dial and merge candidates under.
    pub peer: PeerKey,
    /// The addresses the record advertised (as multiaddr strings; opaque here).
    pub addresses: Vec<String>,
    /// Which trust root surfaced this candidate.
    pub source: Source,
    /// The authenticated signer/sequence domain for [`Self::seq`].
    pub freshness: FreshnessPrincipal,
    /// The principal's **own signed** monotonic sequence number. Freshness is judged off this,
    /// never a server TTL.
    ///
    /// INVARIANT: the caller MUST verify the record signature and select the matching freshness
    /// principal: `Transport` for a libp2p `PeerRecord`, `Device` for a member-signed
    /// `PeerDescriptor`. An unauthenticated or mismatched principal could pin another signer's
    /// high-water and suppress its later genuine records (an availability-only self-eclipse).
    pub seq: u64,
    /// Whether the member-only registration tag verified (caller-checked).
    pub tag_verified: bool,
}

/// A peer the policy decided to offer for dialing, with its merged addresses, in
/// rank order. (Dialing itself is the caller's job; this is advice.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDial {
    /// The peer to dial.
    pub peer: PeerKey,
    /// The union of addresses learned for it (bounded by `max_addresses`).
    pub addresses: Vec<String>,
}

/// The host a canonical peer route contacts before Noise authenticates its terminal peer.
///
/// Discovery callers apply their own trust policy to this value: member records accept public IP
/// literals only, while an operator-configured rendezvous may intentionally use a DNS name or LAN
/// address. Keeping syntax parsing here avoids the more dangerous outcome where those callers
/// implement subtly different `/p2p/` and relay-route grammars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteHost {
    /// A literal IPv4 or IPv6 destination.
    Ip(IpAddr),
    /// A DNS destination. Untrusted member/invite routes should reject this variant.
    Dns(String),
}

/// One canonical socket endpoint ready to be charged by [`EndpointDialScheduler`].
///
/// The scheduler principal is carried inside the endpoint, rather than supplied independently by
/// its caller. This prevents one transport peer being charged under raw libp2p bytes in one path
/// and a device id in another. Direct-route attempt keys intentionally exclude the terminal peer
/// id so rotating a claimed identity cannot reset a victim-socket limit. Relay-route keys include
/// the authenticated relay and terminal peers because a circuit is a logical resource distinct
/// from the relay's shared outer socket; the outer host remains bounded by the prefix/process
/// counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialEndpoint {
    address: String,
    principal: CanonicalDialPeer,
    attempt_key: [u8; 32],
    prefix_key: [u8; 32],
}

impl DialEndpoint {
    /// The canonical multiaddr string to pass to the transport after the scheduler grants it.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Test-only constructor for exercising scheduler boundaries without depending on a concrete
    /// multiaddr spelling. Production endpoints can only come from [`parse_peer_dial_route`].
    #[cfg(test)]
    fn from_key_material(
        address: impl Into<String>,
        attempt_material: &[u8],
        prefix_material: &[u8],
        principal: CanonicalDialPeer,
    ) -> Self {
        Self {
            address: address.into(),
            principal,
            attempt_key: *blake3::hash(attempt_material).as_bytes(),
            prefix_key: *blake3::hash(prefix_material).as_bytes(),
        }
    }
}

/// The one identity domain used by the shared per-peer scheduler bucket: the Phase-0 hash of the
/// terminal libp2p `PeerId` carried by a canonical route.
///
/// The bytes are private and no public arbitrary-byte constructor exists. Callers obtain this
/// value from [`ParsedPeerRoute`], tying accounting to the same terminal identity the parser
/// authenticated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanonicalDialPeer([u8; 32]);

impl CanonicalDialPeer {
    /// The canonical Phase-0 peer bytes used for retry/high-water maps outside this crate.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Which resource a canonical route asks the network to establish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialRouteKind {
    /// A direct connection to the terminal peer at the route's socket.
    Direct,
    /// A logical circuit through an authenticated relay to the terminal peer.
    Relay {
        /// Phase-0 identity of the relay named before `/p2p-circuit`.
        relay_peer: CanonicalDialPeer,
        /// Phase-0 identity of the route's final target.
        target_peer: CanonicalDialPeer,
    },
}

/// Concrete direct transport encoded by a canonical route.
///
/// Local reconnect state deliberately accepts a narrower subset than the general discovery
/// grammar, which also supports WebSocket infrastructure routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialRouteTransport {
    Tcp,
    WebSocket,
    QuicV1,
}

/// A syntactically canonical peer-bound route and its scheduler identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPeerRoute {
    /// The network host contacted by the route (a relay host for a circuit address).
    pub host: RouteHost,
    /// The canonical terminal identity used by every per-peer accounting caller.
    pub principal: CanonicalDialPeer,
    /// Whether this establishes a direct connection or a logical relay circuit.
    pub kind: DialRouteKind,
    /// The socket/wrapper shape after the host.
    pub transport: DialRouteTransport,
    /// The endpoint accounting key and canonical address.
    pub endpoint: DialEndpoint,
}

/// Parse the one route grammar accepted from signed peer records and rendezvous results.
///
/// Accepted routes have exactly one host, one non-zero TCP or UDP/QUIC-v1 socket, and a terminal
/// `/p2p/<PeerId>` whose Phase-0 BLAKE3 id equals `expected_peer`. A relay route may contain one
/// additional relay peer followed by exactly `/p2p-circuit/p2p/<expected>`. TCP is raw or uses
/// exactly `/ws`, `/wss`, or `/tls/ws`, matching the product's libp2p WebSocket parser; arbitrary
/// protocols, SNI, repeated/mixed wrappers, trailing components, zero ports, non-canonical
/// spellings, and bare socket addresses fail closed.
///
/// This function validates syntax and peer binding, not whether the host is safe to contact. A
/// caller handling untrusted records must additionally reject [`RouteHost::Dns`] and apply its
/// canonical global-address classifier to [`RouteHost::Ip`].
pub fn parse_peer_dial_route(addr: &str, expected_peer: &[u8; 32]) -> Option<ParsedPeerRoute> {
    let parsed: Multiaddr = addr.parse().ok()?;
    if parsed.to_string() != addr {
        return None;
    }
    let parts: Vec<_> = parsed.iter().collect();
    let (host, host_key, prefix_key) = match parts.first()? {
        Protocol::Ip4(ip) => (
            RouteHost::Ip(IpAddr::V4(*ip)),
            ip.octets().to_vec(),
            ipv4_prefix(*ip),
        ),
        Protocol::Ip6(ip) => {
            // IPv4-mapped/compatible IPv6 reaches the same physical IPv4 host. Keep the original
            // family for the caller's classifier, but normalize scheduler keys so rewriting an
            // address as `::ffff:a.b.c.d` cannot refill endpoint or prefix limits.
            let (host_key, prefix_key) = ip.to_ipv4().map_or_else(
                || (ip.octets().to_vec(), ipv6_prefix(*ip)),
                |v4| (v4.octets().to_vec(), ipv4_prefix(v4)),
            );
            (RouteHost::Ip(IpAddr::V6(*ip)), host_key, prefix_key)
        }
        Protocol::Dns(name)
        | Protocol::Dns4(name)
        | Protocol::Dns6(name)
        | Protocol::Dnsaddr(name) => {
            let normalized = name.to_ascii_lowercase();
            (
                RouteHost::Dns(normalized.clone()),
                normalized.as_bytes().to_vec(),
                normalized.into_bytes(),
            )
        }
        _ => return None,
    };

    let mut index = 1;
    let (transport, transport_tag, port) = match parts.get(index)? {
        Protocol::Tcp(port) if *port != 0 => {
            index += 1;
            let transport = match parts.get(index) {
                Some(Protocol::Ws(path)) | Some(Protocol::Wss(path)) if path.as_ref() == "/" => {
                    index += 1;
                    DialRouteTransport::WebSocket
                }
                Some(Protocol::Ws(_)) | Some(Protocol::Wss(_)) => return None,
                Some(Protocol::Tls) => {
                    index += 1;
                    if !matches!(parts.get(index), Some(Protocol::Ws(path)) if path.as_ref() == "/")
                    {
                        return None;
                    }
                    index += 1;
                    DialRouteTransport::WebSocket
                }
                _ => DialRouteTransport::Tcp,
            };
            (transport, 6u8, *port)
        }
        Protocol::Udp(port) if *port != 0 => {
            index += 1;
            if !matches!(parts.get(index), Some(Protocol::QuicV1)) {
                return None;
            }
            index += 1;
            (DialRouteTransport::QuicV1, 17u8, *port)
        }
        _ => return None,
    };

    let Protocol::P2p(first_peer) = parts.get(index)? else {
        return None;
    };
    index += 1;
    let (target, kind) = if index == parts.len() {
        (first_peer, DialRouteKind::Direct)
    } else {
        if !matches!(parts.get(index), Some(Protocol::P2pCircuit)) {
            return None;
        }
        index += 1;
        let Protocol::P2p(target) = parts.get(index)? else {
            return None;
        };
        index += 1;
        if index != parts.len() {
            return None;
        }
        let relay_peer = CanonicalDialPeer(*blake3::hash(&first_peer.to_bytes()).as_bytes());
        let target_peer = CanonicalDialPeer(*blake3::hash(&target.to_bytes()).as_bytes());
        (
            target,
            DialRouteKind::Relay {
                relay_peer,
                target_peer,
            },
        )
    };
    let principal = CanonicalDialPeer(*blake3::hash(&target.to_bytes()).as_bytes());
    if principal.as_bytes() != expected_peer {
        return None;
    }

    let mut socket_material = Vec::with_capacity(host_key.len() + 3);
    socket_material.push(transport_tag);
    socket_material.extend_from_slice(&port.to_be_bytes());
    socket_material.extend_from_slice(&host_key);
    let attempt_key = match kind {
        DialRouteKind::Direct => *blake3::hash(&socket_material).as_bytes(),
        DialRouteKind::Relay {
            relay_peer,
            target_peer,
        } => scoped_key(
            b"catcoms/dial/relay-circuit/v1",
            &[
                &socket_material,
                relay_peer.as_bytes(),
                target_peer.as_bytes(),
            ],
        ),
    };
    Some(ParsedPeerRoute {
        host,
        principal,
        kind,
        transport,
        endpoint: DialEndpoint {
            address: addr.to_string(),
            principal,
            attempt_key,
            prefix_key: *blake3::hash(&prefix_key).as_bytes(),
        },
    })
}

fn ipv4_prefix(ip: Ipv4Addr) -> Vec<u8> {
    let octets = ip.octets();
    vec![4, octets[0], octets[1], octets[2]]
}

fn ipv6_prefix(ip: Ipv6Addr) -> Vec<u8> {
    let octets = ip.octets();
    let mut prefix = vec![6];
    prefix.extend_from_slice(&octets[..6]); // /48, matching the abuse-accounting boundary.
    prefix
}

/// Process-shared endpoint limits. Defaults bound ordinary desktop discovery while leaving enough
/// room to try TCP+QUIC over both address families for several peers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointDialConfig {
    /// Length of one monotonic accounting window.
    pub window_ms: u64,
    /// Endpoints granted across the whole process per window.
    pub process_limit: u32,
    /// Endpoints granted to one server/group per window.
    pub server_limit: u32,
    /// Endpoints granted to one `(server, peer)` pair per window.
    pub peer_limit: u32,
    /// Attempts to one direct physical socket or one authenticated relay circuit per window.
    /// Relay outer-host pressure is bounded separately by `prefix_limit` and the process cap.
    pub endpoint_limit: u32,
    /// Attempts to one IPv4 /24, IPv6 /48, or DNS host per window.
    pub prefix_limit: u32,
}

impl Default for EndpointDialConfig {
    fn default() -> Self {
        Self {
            window_ms: 60_000,
            process_limit: 32,
            server_limit: 8,
            peer_limit: 4,
            endpoint_limit: 2,
            prefix_limit: 8,
        }
    }
}

#[derive(Debug, Default)]
struct EndpointDialState {
    /// Identifies the accounting window. A permit from an older generation can neither start an
    /// uncharged dial nor decrement counters belonging to the replacement window.
    generation: u64,
    window_start_ms: Option<u64>,
    process_spent: u32,
    server_spent: HashMap<[u8; 32], u32>,
    peer_spent: HashMap<[u8; 32], u32>,
    endpoint_spent: HashMap<[u8; 32], u32>,
    prefix_spent: HashMap<[u8; 32], u32>,
}

/// A cloneable, deterministic endpoint scheduler shared by every server in one application.
///
/// Ranking remains per group, but every actual member-discovery endpoint passes through this
/// handle. The short lock covers counters only; no network call or await occurs while held. State
/// is transient and resets on process restart, while a monotonic `Clock` makes wall-clock changes
/// irrelevant. Unique-key maps cannot exceed `process_limit` entries in one window because keys
/// are inserted only for granted attempts.
#[derive(Debug, Clone)]
pub struct EndpointDialScheduler {
    config: EndpointDialConfig,
    state: Arc<Mutex<EndpointDialState>>,
    /// The one monotonic timeline governing reservations and commits. Keeping the clock on the
    /// scheduler prevents callers from minting a permit under one timeline and committing it
    /// under another, and lets a queued permit enforce its deadline without another reservation
    /// having to roll the window first.
    clock: Arc<dyn Clock>,
}

/// A single-use reservation for one exact canonical endpoint.
///
/// Dropping it before [`catcoms_rt::DialPermit::commit_if_current`] returns every counter it
/// consumed. The permit is intentionally not `Clone`; one plan cannot be replayed into several
/// attempts. Actor-backed transports own it across their command boundary, so caller cancellation
/// cannot refund work that will still run.
#[derive(Debug)]
pub struct EndpointDialPermit {
    scheduler: EndpointDialScheduler,
    address: String,
    server_key: [u8; 32],
    peer_key: [u8; 32],
    endpoint_key: [u8; 32],
    prefix_key: [u8; 32],
    generation: u64,
    window_start_ms: u64,
    started: bool,
}

impl EndpointDialPermit {
    /// Commit this reservation immediately before submitting the exact endpoint to the transport.
    /// Consumes the permit, so a caller cannot start it twice.
    pub fn start(self) -> Option<String> {
        self.commit_current()
    }

    /// The exact canonical endpoint this permit owns, for duplicate/already-connected checks.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Atomically bind this reservation to the window that is still current.
    fn commit_current(mut self) -> Option<String> {
        let state = self
            .scheduler
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let now = self.scheduler.clock.monotonic_ms();
        if state.generation != self.generation
            || now.saturating_sub(self.window_start_ms) >= self.scheduler.config.window_ms.max(1)
        {
            return None;
        }
        drop(state);
        self.started = true;
        Some(std::mem::take(&mut self.address))
    }
}

impl catcoms_rt::DialPermit for EndpointDialPermit {
    fn address(&self) -> &str {
        self.address()
    }

    fn commit_if_current(self: Box<Self>) -> Option<String> {
        (*self).commit_current()
    }
}

impl Drop for EndpointDialPermit {
    fn drop(&mut self) {
        if self.started {
            return;
        }
        let mut state = self
            .scheduler
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state.generation != self.generation {
            // The whole old window was discarded at rollover. Refunding its reservation into the
            // replacement window would create fresh capacity for every delayed/cancelled command.
            return;
        }
        state.process_spent = state.process_spent.saturating_sub(1);
        decrement(&mut state.server_spent, self.server_key);
        decrement(&mut state.peer_spent, self.peer_key);
        decrement(&mut state.endpoint_spent, self.endpoint_key);
        decrement(&mut state.prefix_spent, self.prefix_key);
    }
}

impl EndpointDialScheduler {
    /// Construct an isolated scheduler. Production creates one and clones it into every server;
    /// tests can construct isolated handles without process-global cross-test interference.
    pub fn new(config: EndpointDialConfig) -> Self {
        Self::new_with_clock(config, Arc::new(SystemClock))
    }

    /// Construct an isolated scheduler on an injected monotonic timeline.
    ///
    /// The clock is retained for the scheduler's whole lifetime because permit expiry is checked
    /// at the actor commit boundary, potentially long after the caller that reserved it returned.
    pub fn new_with_clock(config: EndpointDialConfig, clock: Arc<dyn Clock>) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(EndpointDialState::default())),
            clock,
        }
    }

    /// Reserve a bounded subset of endpoints in caller order.
    ///
    /// The canonical peer principal is embedded by the parser in every endpoint. Accepting it as
    /// a separate byte slice previously allowed cache, rendezvous, and pre-join callers to charge
    /// the same transport under three unrelated identity representations.
    pub fn reserve(&self, server: &[u8], endpoints: &[DialEndpoint]) -> Vec<String> {
        self.reserve_permits(server, endpoints)
            .into_iter()
            .filter_map(EndpointDialPermit::start)
            .collect()
    }

    /// Reserve exact, single-use endpoint permits. Unlike [`Self::reserve`], unused results refund
    /// themselves on drop; new socket-starting call sites should use this API.
    pub fn reserve_permits(
        &self,
        server: &[u8],
        endpoints: &[DialEndpoint],
    ) -> Vec<EndpointDialPermit> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let now = self.clock.monotonic_ms();
        let expired = state
            .window_start_ms
            // A zero duration is a configuration mistake, not permission to refill on every
            // reservation in the same actor drain. Clamp it to the smallest monotonic window.
            .is_none_or(|start| now.saturating_sub(start) >= self.config.window_ms.max(1));
        if expired {
            // At u64::MAX, retaining the exhausted counters is safer than reusing a generation
            // while old permits of that generation might exist. Reaching it requires more
            // windows than the process can physically observe.
            let generation = state.generation.saturating_add(1);
            if generation == state.generation {
                return Vec::new();
            }
            *state = EndpointDialState {
                generation,
                window_start_ms: Some(now),
                ..EndpointDialState::default()
            };
        }

        let server_key = scoped_key(b"catcoms/dial/server/v1", &[server]);
        let mut granted = Vec::new();
        for endpoint in endpoints {
            let peer_key = scoped_key(
                b"catcoms/dial/peer/v1",
                &[server, endpoint.principal.as_bytes()],
            );
            if state.process_spent >= self.config.process_limit
                || state.server_spent.get(&server_key).copied().unwrap_or(0)
                    >= self.config.server_limit
                || state.peer_spent.get(&peer_key).copied().unwrap_or(0) >= self.config.peer_limit
                || state
                    .endpoint_spent
                    .get(&endpoint.attempt_key)
                    .copied()
                    .unwrap_or(0)
                    >= self.config.endpoint_limit
                || state
                    .prefix_spent
                    .get(&endpoint.prefix_key)
                    .copied()
                    .unwrap_or(0)
                    >= self.config.prefix_limit
            {
                continue;
            }
            state.process_spent = state.process_spent.saturating_add(1);
            increment(&mut state.server_spent, server_key);
            increment(&mut state.peer_spent, peer_key);
            increment(&mut state.endpoint_spent, endpoint.attempt_key);
            increment(&mut state.prefix_spent, endpoint.prefix_key);
            granted.push(EndpointDialPermit {
                scheduler: self.clone(),
                address: endpoint.address.clone(),
                server_key,
                peer_key,
                endpoint_key: endpoint.attempt_key,
                prefix_key: endpoint.prefix_key,
                generation: state.generation,
                window_start_ms: state
                    .window_start_ms
                    .expect("an active generation always has a window start"),
                started: false,
            });
        }
        granted
    }
}

impl Default for EndpointDialScheduler {
    fn default() -> Self {
        Self::new(EndpointDialConfig::default())
    }
}

fn scoped_key(domain: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    for part in parts {
        hasher.update(&(part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    *hasher.finalize().as_bytes()
}

fn increment(map: &mut HashMap<[u8; 32], u32>, key: [u8; 32]) {
    let value = map.entry(key).or_default();
    *value = value.saturating_add(1);
}

fn decrement(map: &mut HashMap<[u8; 32], u32>, key: [u8; 32]) {
    let Some(value) = map.get_mut(&key) else {
        return;
    };
    *value = value.saturating_sub(1);
    if *value == 0 {
        map.remove(&key);
    }
}

/// Tunable bounds. Defaults suit a desktop node; tests shrink them.
#[derive(Debug, Clone, Copy)]
pub struct PolicyConfig {
    /// `B`: endpoint attempts granted per budget window (shared across all sources).
    pub dial_budget: u32,
    /// Budget-window length on the injected clock (ms).
    pub window_ms: u64,
    /// Max RNG jitter (ms) added to each window length, decorrelating the cadence so
    /// an observer cannot predict exactly when the budget refills.
    pub jitter_ms: u64,
    /// Extra dial slots allowed above `roster_size - 1` (headroom for stale cache /
    /// PEX entries that may no longer resolve).
    pub roster_headroom: usize,
    /// A floor on the dial slots so a tiny group (founder + a seed) can still bootstrap.
    pub min_dial_slots: usize,
    /// Cap on merged addresses retained per peer (anti-bloat).
    pub max_addresses: usize,
    /// Cap on distinct peers whose high-water seq we remember across calls (a
    /// bound on the anti-replay map; coarse eviction past this).
    pub max_tracked_peers: usize,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            dial_budget: 8,
            window_ms: 10_000,
            jitter_ms: 2_000,
            roster_headroom: 4,
            min_dial_slots: 3,
            max_addresses: 8,
            max_tracked_peers: 4096,
        }
    }
}

/// Score weights. A tag-verified member dominates everything; otherwise more
/// distinct trust roots rank higher, and a prior proven contact (cache) outranks a
/// raw single-rendezvous record (the flood). Each rendezvous counts once.
///
/// `SCORE_TAG_VERIFIED` never fires in this workspace, because nothing sets the flag it reads;
/// see [`Candidate::tag_verified`].
const SCORE_TAG_VERIFIED: i64 = 1_000;
const SCORE_PER_RENDEZVOUS: i64 = 30;
const SCORE_PER_PEX: i64 = 50;
const SCORE_CACHE: i64 = 50;
/// Cap distinct roots that contribute to the score, so the ordering can never be
/// inflated without bound (a peer corroborated by 4+ rendezvous is plenty).
const ROOT_CAP: usize = 4;

/// A peer after unioning every candidate that names it.
#[derive(Debug)]
struct Merged {
    peer: PeerKey,
    addresses: Vec<String>,
    /// Distinct rendezvous that surfaced this peer (each = one trust root).
    rendezvous_roots: Vec<PeerKey>,
    /// Distinct members that PEXed this peer.
    pex_roots: Vec<PeerKey>,
    from_cache: bool,
    tag_verified: bool,
    /// Highest signed seq seen for this peer this call.
    seq: u64,
}

impl Merged {
    fn new(peer: PeerKey) -> Self {
        Self {
            peer,
            addresses: Vec::new(),
            rendezvous_roots: Vec::new(),
            pex_roots: Vec::new(),
            from_cache: false,
            tag_verified: false,
            seq: 0,
        }
    }

    fn absorb(&mut self, c: Candidate, max_addresses: usize) {
        self.tag_verified |= c.tag_verified;
        self.seq = self.seq.max(c.seq);
        for a in c.addresses {
            if self.addresses.len() >= max_addresses {
                break;
            }
            if !self.addresses.contains(&a) {
                self.addresses.push(a);
            }
        }
        match c.source {
            Source::Rendezvous(id) => {
                if !self.rendezvous_roots.contains(&id) {
                    self.rendezvous_roots.push(id);
                }
            }
            Source::Pex(id) => {
                if !self.pex_roots.contains(&id) {
                    self.pex_roots.push(id);
                }
            }
            Source::Cache => self.from_cache = true,
        }
    }

    fn score(&self) -> i64 {
        let mut s = 0;
        if self.tag_verified {
            s += SCORE_TAG_VERIFIED;
        }
        s += SCORE_PER_RENDEZVOUS * self.rendezvous_roots.len().min(ROOT_CAP) as i64;
        s += SCORE_PER_PEX * self.pex_roots.len().min(ROOT_CAP) as i64;
        if self.from_cache {
            s += SCORE_CACHE;
        }
        s
    }

    /// The bucket this peer round-robins within, so one rendezvous's flood cannot
    /// dominate: its source rendezvous (smallest id if several), else its PEX
    /// voucher, else the single shared cache bucket.
    fn bucket(&self) -> Vec<u8> {
        if let Some(min) = self.rendezvous_roots.iter().min() {
            let mut b = vec![0u8];
            b.extend_from_slice(min);
            b
        } else if let Some(min) = self.pex_roots.iter().min() {
            let mut b = vec![1u8];
            b.extend_from_slice(min);
            b
        } else {
            vec![2u8]
        }
    }
}

/// The stateful discovery policy. Holds the dial-budget window and the per-peer
/// high-water seq map across calls; `plan` is otherwise a pure function of its inputs.
#[derive(Debug)]
pub struct DiscoveryPolicy {
    config: PolicyConfig,
    /// High-water signed seq per authenticated signer domain, to drop replayed/stale records.
    best_seq: BTreeMap<FreshnessPrincipal, u64>,
    /// Current budget window start (on the injected clock), or `None` before the
    /// first `plan`.
    window_start_ms: Option<u64>,
    /// This window's length (base + RNG jitter), fixed when the window opens.
    window_len_ms: u64,
    /// Endpoint attempts granted so far in the current window.
    spent: u32,
}

impl DiscoveryPolicy {
    /// A policy with default bounds.
    pub fn new() -> Self {
        Self::with_config(PolicyConfig::default())
    }

    /// A policy with explicit bounds.
    pub fn with_config(config: PolicyConfig) -> Self {
        Self {
            config,
            best_seq: BTreeMap::new(),
            window_start_ms: None,
            window_len_ms: config.window_ms,
            spent: 0,
        }
    }

    /// Endpoint attempts still available in the current budget window (diagnostics / tests). Does
    /// not advance the window.
    pub fn remaining_budget(&self) -> u32 {
        self.config.dial_budget.saturating_sub(self.spent)
    }

    /// Forget the signed-sequence high-water for a member device whose record state was removed.
    ///
    /// This removes only the device-signed descriptor domain. A member cannot reset the genuine
    /// transport-signed rendezvous high-water for a peer it merely self-asserted. Process/window
    /// dial budgets are intentionally not refunded by record removal.
    pub fn forget_device_freshness(&mut self, device: &[u8]) {
        self.best_seq
            .remove(&FreshnessPrincipal::Device(device.to_vec()));
    }

    /// Return locally planned endpoint tokens that a final shared scheduler did not grant.
    ///
    /// [`Self::plan`] must reserve before returning so independent callers cannot forget to
    /// account for an address fan-out. A process-wide scheduler can then apply stricter limits;
    /// denied endpoints never touched a socket and must not strand this server behind two
    /// separate budget windows. An oversized refund is ignored (fails closed); the planned typed
    /// batch API should eventually replace this count with an opaque reservation receipt.
    pub fn refund_endpoint_budget(&mut self, endpoints: usize) {
        let Ok(endpoints) = u32::try_from(endpoints) else {
            return;
        };
        if endpoints <= self.spent {
            self.spent -= endpoints;
        }
    }

    /// Rank `candidates` into a bounded dial plan for a group whose roster has
    /// `roster_size` members (including this node). Consumes dial budget for the
    /// addresses it returns. Returns peers in dial order (best first), with each peer's address
    /// list trimmed to the remaining endpoint budget.
    pub fn plan(
        &mut self,
        candidates: Vec<Candidate>,
        roster_size: usize,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
    ) -> Vec<PlannedDial> {
        // 1. Freshness: a peer is stale iff the best seq it presents THIS call is
        //    below the high-water seq we have already accepted for it (a replayed or
        //    superseded record). Drop all of a stale peer's candidates; otherwise
        //    learn the new high-water.
        let mut incoming_max: BTreeMap<FreshnessPrincipal, u64> = BTreeMap::new();
        for c in &candidates {
            let e = incoming_max.entry(c.freshness.clone()).or_insert(c.seq);
            *e = (*e).max(c.seq);
        }
        let mut stale: BTreeMap<FreshnessPrincipal, bool> = BTreeMap::new();
        for (principal, &max_seq) in &incoming_max {
            let is_stale = matches!(self.best_seq.get(principal), Some(&b) if max_seq < b);
            stale.insert(principal.clone(), is_stale);
            if !is_stale {
                let slot = self.best_seq.entry(principal.clone()).or_insert(max_seq);
                *slot = (*slot).max(max_seq);
            } else {
                tracing::trace!("dropping stale-seq peer from discovery plan");
            }
        }
        self.evict_tracked();

        // 2. Merge surviving candidates by peer (union sources + addresses, max seq).
        let mut merged: BTreeMap<PeerKey, Merged> = BTreeMap::new();
        for c in candidates {
            if *stale.get(&c.freshness).unwrap_or(&false) {
                continue;
            }
            merged
                .entry(c.peer.clone())
                .or_insert_with(|| Merged::new(c.peer.clone()))
                .absorb(c, self.config.max_addresses);
        }
        if merged.is_empty() {
            return Vec::new();
        }

        // 3. Assign each peer a within-bucket index so equal-score peers interleave
        //    round-robin across their source (a single rendezvous's Nth record sorts
        //    after every other source's first). Bucket order within is deterministic:
        //    higher score, then fresher seq, then peer bytes.
        let mut items: Vec<Merged> = merged.into_values().collect();
        items.sort_by(|a, b| {
            b.score()
                .cmp(&a.score())
                .then(b.seq.cmp(&a.seq))
                .then(a.peer.cmp(&b.peer))
        });
        let mut bucket_counts: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
        let mut within: Vec<usize> = Vec::with_capacity(items.len());
        for m in &items {
            let idx = bucket_counts.entry(m.bucket()).or_insert(0);
            within.push(*idx);
            *idx += 1;
        }
        // Re-sort by (score desc, within-bucket index asc, peer asc): interleaves
        // equal-rank peers across sources while keeping higher-scored peers first.
        let mut order: Vec<usize> = (0..items.len()).collect();
        order.sort_by(|&i, &j| {
            items[j]
                .score()
                .cmp(&items[i].score())
                .then(within[i].cmp(&within[j]))
                .then(items[i].peer.cmp(&items[j].peer))
        });

        // 4. Roster clamp: never offer more distinct peers than could plausibly be
        //    members (plus headroom), with a small floor for bootstrap.
        let clamp = (roster_size.saturating_sub(1) + self.config.roster_headroom)
            .max(self.config.min_dial_slots);

        // 5. Endpoint budget: a peer with eight addresses costs eight attempts, not one. The
        // process-shared scheduler above this policy applies the wider cross-server caps; this
        // local window prevents one server/source from consuming an arbitrary address fan-out.
        let mut remaining = self.grant_budget(clock, rng);
        let mut out = Vec::new();
        for i in order {
            if out.len() >= clamp || remaining == 0 {
                break;
            }
            let addresses: Vec<String> =
                items[i].addresses.iter().take(remaining).cloned().collect();
            if addresses.is_empty() {
                continue;
            }
            remaining -= addresses.len();
            self.spent = self.spent.saturating_add(addresses.len() as u32);
            out.push(PlannedDial {
                peer: items[i].peer.clone(),
                addresses,
            });
        }
        out
    }

    /// Compute the dials available now, rolling the budget window over (with fresh RNG
    /// jitter) if the previous one has elapsed.
    fn grant_budget(&mut self, clock: &dyn Clock, rng: &mut impl CryptoRngCore) -> usize {
        let now = clock.monotonic_ms();
        let expired = match self.window_start_ms {
            None => true,
            Some(start) => now.saturating_sub(start) >= self.window_len_ms,
        };
        if expired {
            self.window_start_ms = Some(now);
            let jitter = if self.config.jitter_ms == 0 {
                0
            } else {
                (rng.next_u32() as u64) % (self.config.jitter_ms + 1)
            };
            self.window_len_ms = self.config.window_ms + jitter;
            self.spent = 0;
        }
        self.remaining_budget() as usize
    }

    /// Bound the anti-replay seq map: past the cap, drop the lowest-keyed entries
    /// (coarse but deterministic; at worst re-admits one already-seen record).
    fn evict_tracked(&mut self) {
        while self.best_seq.len() > self.config.max_tracked_peers {
            let Some(lowest) = self.best_seq.keys().next().cloned() else {
                break;
            };
            self.best_seq.remove(&lowest);
        }
    }
}

impl Default for DiscoveryPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::ManualClock;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn rng() -> ChaCha20Rng {
        ChaCha20Rng::seed_from_u64(0)
    }

    fn peer(n: u8) -> PeerKey {
        vec![n; 32]
    }

    fn rdv(n: u8) -> PeerKey {
        // A distinct rendezvous-id per `n` (16 bytes, well clear of the peer keys).
        vec![0xA0u8.wrapping_add(n); 16]
    }

    const PEER_A: &str = "12D3KooWHp1hLNjWf4ZM4eLaiUdMGTbGnXDDDkhnE56P9CRbHx8E";
    const PEER_B: &str = "12D3KooWBGfsSWvGFAJeTz3oBPeRFbSadCwedBJvJ6AFAJtfkSD2";

    fn phase0(peer: &str) -> [u8; 32] {
        let address: Multiaddr = format!("/ip4/203.0.113.1/tcp/9/p2p/{peer}")
            .parse()
            .unwrap();
        let raw = address
            .iter()
            .find_map(|part| match part {
                Protocol::P2p(peer) => Some(peer.to_bytes()),
                _ => None,
            })
            .unwrap();
        *blake3::hash(&raw).as_bytes()
    }

    fn cand(p: u8, source: Source, seq: u64, tag_verified: bool) -> Candidate {
        let peer = peer(p);
        let freshness = match &source {
            Source::Cache => FreshnessPrincipal::Device(peer.clone()),
            Source::Rendezvous(_) | Source::Pex(_) => FreshnessPrincipal::Transport(peer.clone()),
        };
        Candidate {
            peer,
            addresses: vec![format!("/ip4/10.0.0.{p}/tcp/4001")],
            source,
            freshness,
            seq,
            tag_verified,
        }
    }

    /// A generous budget + window so the budget never interferes with ranking tests.
    fn ranking_policy() -> DiscoveryPolicy {
        DiscoveryPolicy::with_config(PolicyConfig {
            dial_budget: 1_000,
            roster_headroom: 64,
            min_dial_slots: 64,
            ..PolicyConfig::default()
        })
    }

    #[test]
    fn a_tag_verified_member_and_corroborated_peers_rank_above_junk() {
        let mut pol = ranking_policy();
        let clock = ManualClock::new(0);
        let mut r = rng();
        let candidates = vec![
            // peer 9: junk; single rendezvous, unverified.
            cand(9, Source::Rendezvous(rdv(1)), 1, false),
            // peer 5: corroborated by two distinct rendezvous.
            cand(5, Source::Rendezvous(rdv(1)), 1, false),
            cand(5, Source::Rendezvous(rdv(2)), 1, false),
            // peer 3: tag-verified member (single rendezvous).
            cand(3, Source::Rendezvous(rdv(2)), 1, true),
            // peer 7: cache-only (a prior proven contact).
            cand(7, Source::Cache, 1, false),
        ];
        let plan = pol.plan(candidates, 5, &clock, &mut r);
        let order: Vec<PeerKey> = plan.iter().map(|p| p.peer.clone()).collect();
        assert_eq!(order[0], peer(3), "tag-verified member leads");
        assert_eq!(order[1], peer(5), "two-rendezvous corroboration next");
        assert_eq!(
            order[2],
            peer(7),
            "cache (prior proven contact) beats raw junk"
        );
        assert_eq!(
            *order.last().unwrap(),
            peer(9),
            "single-rendezvous junk ranks last"
        );
    }

    #[test]
    fn a_single_rendezvous_flood_cannot_dominate_dial_order() {
        let mut pol = ranking_policy();
        let clock = ManualClock::new(0);
        let mut r = rng();
        let mut candidates = Vec::new();
        // Rendezvous A floods 200 distinct junk peers.
        for p in 0..200u32 {
            candidates.push(Candidate {
                peer: p.to_be_bytes().to_vec(),
                addresses: vec!["/ip4/10.0.0.1/tcp/1".into()],
                source: Source::Rendezvous(rdv(1)),
                freshness: FreshnessPrincipal::Transport(p.to_be_bytes().to_vec()),
                seq: 1,
                tag_verified: false,
            });
        }
        // Rendezvous B offers a single (equally unverified) peer.
        let b_peer = vec![0xBB; 8];
        candidates.push(Candidate {
            peer: b_peer.clone(),
            addresses: vec!["/ip4/10.0.0.2/tcp/2".into()],
            source: Source::Rendezvous(rdv(2)),
            freshness: FreshnessPrincipal::Transport(b_peer.clone()),
            seq: 1,
            tag_verified: false,
        });
        let plan = pol.plan(candidates, 5, &clock, &mut r);
        // B's lone peer must surface in the first couple of slots, not be buried
        // behind A's flood (round-robin interleave across the source bucket).
        let pos = plan.iter().position(|d| d.peer == b_peer);
        assert!(
            matches!(pos, Some(p) if p <= 1),
            "B's peer should interleave to the front, got {pos:?}"
        );
    }

    #[test]
    fn a_flood_under_a_small_roster_is_clamped() {
        let mut pol = DiscoveryPolicy::with_config(PolicyConfig {
            dial_budget: 1_000, // budget high, so the CLAMP (not the budget) bounds it
            roster_headroom: 4,
            min_dial_slots: 3,
            ..PolicyConfig::default()
        });
        let clock = ManualClock::new(0);
        let mut r = rng();
        let candidates: Vec<Candidate> = (0..500u32)
            .map(|p| Candidate {
                peer: p.to_be_bytes().to_vec(),
                addresses: vec!["/ip4/10.0.0.1/tcp/1".into()],
                source: Source::Rendezvous(rdv(1)),
                freshness: FreshnessPrincipal::Transport(p.to_be_bytes().to_vec()),
                seq: 1,
                tag_verified: false,
            })
            .collect();
        let plan = pol.plan(candidates, 4, &clock, &mut r);
        // roster 4 → (4-1) + headroom 4 = 7 dial slots; 500 is clamped to 7.
        assert_eq!(plan.len(), 7, "500 candidates under roster 4 clamp to 7");
    }

    #[test]
    fn a_stale_seq_record_is_dropped_across_calls() {
        let mut pol = ranking_policy();
        let clock = ManualClock::new(0);
        let mut r = rng();
        // First sighting: peer 5 at seq 10.
        let plan1 = pol.plan(
            vec![cand(5, Source::Rendezvous(rdv(1)), 10, false)],
            5,
            &clock,
            &mut r,
        );
        assert_eq!(plan1.len(), 1);
        // A replayed older record (seq 3) for the same peer is dropped.
        let plan2 = pol.plan(
            vec![cand(5, Source::Rendezvous(rdv(1)), 3, false)],
            5,
            &clock,
            &mut r,
        );
        assert!(plan2.is_empty(), "stale-seq record must be dropped");
        // A fresher record (seq 11) is accepted.
        let plan3 = pol.plan(
            vec![cand(5, Source::Rendezvous(rdv(1)), 11, false)],
            5,
            &clock,
            &mut r,
        );
        assert_eq!(plan3.len(), 1, "a newer seq is accepted");
    }

    #[test]
    fn signer_scoped_freshness_domains_cannot_pin_or_reset_each_other() {
        let mut pol = ranking_policy();
        let clock = ManualClock::new(0);
        let mut r = rng();
        let target = peer(5);
        let device_a = peer(41);
        let device_b = peer(42);
        let candidate = |freshness, seq| Candidate {
            peer: target.clone(),
            addresses: vec!["/ip4/10.0.0.5/tcp/4001".into()],
            source: Source::Cache,
            freshness,
            seq,
            tag_verified: false,
        };

        assert_eq!(
            pol.plan(
                vec![candidate(FreshnessPrincipal::Device(device_a.clone()), 900)],
                5,
                &clock,
                &mut r,
            )
            .len(),
            1
        );
        assert_eq!(
            pol.plan(
                vec![candidate(FreshnessPrincipal::Transport(target.clone()), 1,)],
                5,
                &clock,
                &mut r,
            )
            .len(),
            1,
            "a device descriptor's high sequence cannot stale the transport-signed domain"
        );

        assert_eq!(
            pol.plan(
                vec![candidate(FreshnessPrincipal::Device(device_b), 1)],
                5,
                &clock,
                &mut r,
            )
            .len(),
            1,
            "a second device signer has an independent descriptor sequence domain"
        );

        // Releasing A's member record must not reset the genuine transport's rendezvous
        // high-water: a replayed lower transport-signed sequence remains stale.
        pol.forget_device_freshness(&device_a);
        assert!(
            pol.plan(
                vec![candidate(FreshnessPrincipal::Transport(target.clone()), 0)],
                5,
                &clock,
                &mut r,
            )
            .is_empty(),
            "a member cannot reset the transport signer's anti-replay state"
        );
    }

    #[test]
    fn a_cache_only_peer_is_still_offered() {
        let mut pol = ranking_policy();
        let clock = ManualClock::new(0);
        let mut r = rng();
        let plan = pol.plan(vec![cand(7, Source::Cache, 1, false)], 5, &clock, &mut r);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].peer, peer(7));
    }

    #[test]
    fn the_dial_budget_caps_dials_per_window_and_refills_after_it() {
        let mut pol = DiscoveryPolicy::with_config(PolicyConfig {
            dial_budget: 3,
            window_ms: 1_000,
            jitter_ms: 0, // deterministic window for the assertion
            roster_headroom: 64,
            min_dial_slots: 64,
            ..PolicyConfig::default()
        });
        let clock = ManualClock::new(0);
        let mut r = rng();
        let candidates: Vec<Candidate> = (0..10u8)
            .map(|p| cand(p, Source::Cache, 1, false))
            .collect();
        // First call: budget 3 caps the plan to 3, even though 10 are offered/clamped.
        let plan1 = pol.plan(candidates.clone(), 64, &clock, &mut r);
        assert_eq!(plan1.len(), 3, "budget caps dials this window");
        assert_eq!(pol.remaining_budget(), 0);
        // Same window: nothing more is granted.
        let plan2 = pol.plan(candidates.clone(), 64, &clock, &mut r);
        assert!(plan2.is_empty(), "budget exhausted within the window");
        // After the window elapses, the budget refills.
        clock.advance_ms(1_001);
        let plan3 = pol.plan(candidates, 64, &clock, &mut r);
        assert_eq!(plan3.len(), 3, "budget refills after the window");
    }

    #[test]
    fn one_peer_with_many_addresses_spends_one_budget_unit_per_endpoint() {
        let mut pol = DiscoveryPolicy::with_config(PolicyConfig {
            dial_budget: 3,
            window_ms: 1_000,
            jitter_ms: 0,
            roster_headroom: 8,
            min_dial_slots: 8,
            max_addresses: 8,
            max_tracked_peers: 32,
        });
        let clock = ManualClock::new(0);
        let mut r = rng();
        let candidate = Candidate {
            peer: peer(1),
            addresses: (1..=8)
                .map(|port| format!("/ip4/203.0.113.1/tcp/{port}"))
                .collect(),
            source: Source::Cache,
            freshness: FreshnessPrincipal::Device(peer(1)),
            seq: 1,
            tag_verified: false,
        };
        let plan = pol.plan(vec![candidate.clone()], 2, &clock, &mut r);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].addresses.len(), 3);
        assert_eq!(pol.remaining_budget(), 0);
        assert!(pol.plan(vec![candidate], 2, &clock, &mut r).is_empty());
    }

    #[test]
    fn a_shared_scheduler_deferral_does_not_consume_the_servers_local_window() {
        let mut pol = DiscoveryPolicy::with_config(PolicyConfig {
            dial_budget: 3,
            window_ms: 1_000,
            jitter_ms: 0,
            roster_headroom: 8,
            min_dial_slots: 8,
            max_addresses: 8,
            max_tracked_peers: 32,
        });
        let clock = ManualClock::new(0);
        let mut r = rng();
        let candidate = Candidate {
            peer: peer(1),
            addresses: (1..=3)
                .map(|port| format!("/ip4/203.0.113.1/tcp/{port}"))
                .collect(),
            source: Source::Cache,
            freshness: FreshnessPrincipal::Device(peer(1)),
            seq: 1,
            tag_verified: false,
        };

        let planned = pol.plan(vec![candidate.clone()], 2, &clock, &mut r);
        assert_eq!(planned[0].addresses.len(), 3);
        pol.refund_endpoint_budget(3);
        assert_eq!(pol.remaining_budget(), 3);
        assert_eq!(pol.plan(vec![candidate], 2, &clock, &mut r).len(), 1);
    }

    #[test]
    fn an_oversized_local_budget_refund_is_ignored() {
        let mut pol = DiscoveryPolicy::with_config(PolicyConfig {
            dial_budget: 3,
            window_ms: 1_000,
            jitter_ms: 0,
            roster_headroom: 0,
            min_dial_slots: 1,
            max_addresses: 8,
            max_tracked_peers: 32,
        });
        let clock = ManualClock::new(0);
        let mut r = rng();
        let candidate = Candidate {
            peer: peer(1),
            addresses: vec!["a".into(), "b".into()],
            source: Source::Cache,
            freshness: FreshnessPrincipal::Device(peer(1)),
            seq: 1,
            tag_verified: false,
        };
        assert_eq!(
            pol.plan(vec![candidate], 2, &clock, &mut r)[0]
                .addresses
                .len(),
            2
        );
        pol.refund_endpoint_budget(3);
        assert_eq!(pol.remaining_budget(), 1);
    }

    #[test]
    fn canonical_peer_routes_bind_the_terminal_peer_and_transport_shape() {
        let expected = phase0(PEER_A);
        for good in [
            format!("/ip4/203.0.113.1/tcp/4001/p2p/{PEER_A}"),
            format!("/ip6/2001:db8::1/udp/4001/quic-v1/p2p/{PEER_A}"),
            format!("/ip4/203.0.113.1/tcp/80/ws/p2p/{PEER_A}"),
            format!("/ip4/203.0.113.1/tcp/443/wss/p2p/{PEER_A}"),
            format!("/ip4/203.0.113.1/tcp/443/tls/ws/p2p/{PEER_A}"),
            format!("/ip4/198.51.100.1/tcp/4001/p2p/{PEER_B}/p2p-circuit/p2p/{PEER_A}"),
        ] {
            assert!(
                parse_peer_dial_route(&good, &expected).is_some(),
                "{good} should pass"
            );
        }
        for bad in [
            "/ip4/203.0.113.1/tcp/4001".to_string(),
            format!("/ip4/203.0.113.1/tcp/4001/p2p/{PEER_B}"),
            format!("/ip4/203.0.113.1/tcp/0/p2p/{PEER_A}"),
            format!("/ip4/203.0.113.1/udp/4001/p2p/{PEER_A}"),
            format!("/ip4/203.0.113.1/tcp/4001/http/p2p/{PEER_A}"),
            format!("/ip4/203.0.113.1/tcp/4001/tls/p2p/{PEER_A}"),
            format!("/ip4/203.0.113.1/tcp/4001/tls/sni/example.com/p2p/{PEER_A}"),
            format!("/ip4/203.0.113.1/tcp/4001/tls/sni/example.com/ws/p2p/{PEER_A}"),
            format!("/ip4/203.0.113.1/tcp/4001/tls/wss/p2p/{PEER_A}"),
            format!("/ip4/203.0.113.1/tcp/80/x-parity-ws/%2Fchat/p2p/{PEER_A}"),
            format!("/ip4/203.0.113.1/tcp/443/x-parity-wss/%2Fchat/p2p/{PEER_A}"),
            format!("/ip4/203.0.113.1/tcp/4001/p2p/{PEER_A}/p2p-circuit"),
        ] {
            assert!(
                parse_peer_dial_route(&bad, &expected).is_none(),
                "{bad} should fail"
            );
        }
    }

    #[test]
    fn cloned_schedulers_share_process_and_socket_budgets() {
        let clock = ManualClock::new(0);
        let scheduler = EndpointDialScheduler::new_with_clock(
            EndpointDialConfig {
                window_ms: 1_000,
                process_limit: 3,
                server_limit: 3,
                peer_limit: 3,
                endpoint_limit: 1,
                prefix_limit: 3,
            },
            Arc::new(clock.clone()),
        );
        let other_server = scheduler.clone();
        let route_a = format!("/ip4/203.0.113.9/tcp/4001/p2p/{PEER_A}");
        let route_b = format!("/ip6/::ffff:203.0.113.9/tcp/4001/p2p/{PEER_B}");
        let same_socket_a = parse_peer_dial_route(&route_a, &phase0(PEER_A))
            .unwrap()
            .endpoint;
        let same_socket_b = parse_peer_dial_route(&route_b, &phase0(PEER_B))
            .unwrap()
            .endpoint;
        let socket_c = DialEndpoint::from_key_material(
            "c",
            b"socket-c",
            b"prefix-c",
            CanonicalDialPeer(phase0(PEER_A)),
        );
        let socket_d = DialEndpoint::from_key_material(
            "d",
            b"socket-d",
            b"prefix-d",
            CanonicalDialPeer(phase0(PEER_B)),
        );

        assert_eq!(
            scheduler.reserve(b"server-a", &[same_socket_a]),
            vec![route_a]
        );
        assert!(
            other_server
                .reserve(b"server-b", &[same_socket_b])
                .is_empty(),
            "peer/server rotation and IPv4-mapped spelling must not bypass the socket cap"
        );
        assert_eq!(
            other_server.reserve(b"server-b", &[socket_c, socket_d]),
            vec!["c", "d"]
        );
        assert_eq!(scheduler.state.lock().unwrap().process_spent, 3);
    }

    #[test]
    fn unused_endpoint_permits_refund_every_scope_exactly_once() {
        let clock = ManualClock::new(0);
        let scheduler = EndpointDialScheduler::new_with_clock(
            EndpointDialConfig {
                window_ms: 1_000,
                process_limit: 1,
                server_limit: 1,
                peer_limit: 1,
                endpoint_limit: 1,
                prefix_limit: 1,
            },
            Arc::new(clock),
        );
        let endpoint = DialEndpoint::from_key_material(
            "route",
            b"socket",
            b"prefix",
            CanonicalDialPeer(phase0(PEER_A)),
        );
        let permits = scheduler.reserve_permits(b"server", std::slice::from_ref(&endpoint));
        assert_eq!(permits.len(), 1);
        assert_eq!(permits[0].address(), "route");
        assert!(scheduler
            .reserve_permits(b"server", std::slice::from_ref(&endpoint))
            .is_empty());

        drop(permits);
        let replacement = scheduler.reserve_permits(b"server", &[endpoint]);
        assert_eq!(replacement.len(), 1, "drop returned every charged scope");
        assert_eq!(
            replacement.into_iter().next().unwrap().start(),
            Some("route".into())
        );
        assert_eq!(scheduler.state.lock().unwrap().process_spent, 1);
    }

    #[test]
    fn an_old_permit_cannot_refund_or_start_in_the_replacement_window() {
        let clock = ManualClock::new(0);
        let scheduler = EndpointDialScheduler::new_with_clock(
            EndpointDialConfig {
                window_ms: 100,
                process_limit: 1,
                server_limit: 1,
                peer_limit: 1,
                endpoint_limit: 1,
                prefix_limit: 1,
            },
            Arc::new(clock.clone()),
        );
        let endpoint = |address: &str, key: &[u8]| {
            DialEndpoint::from_key_material(address, key, key, CanonicalDialPeer(phase0(PEER_A)))
        };
        let old = scheduler
            .reserve_permits(b"server", &[endpoint("old", b"old")])
            .pop()
            .unwrap();
        clock.advance_ms(100);
        let current = scheduler
            .reserve_permits(b"server", &[endpoint("current", b"current")])
            .pop()
            .unwrap();

        assert_eq!(old.start(), None, "a queued old-window dial is stale");
        assert_eq!(scheduler.state.lock().unwrap().process_spent, 1);
        assert!(
            scheduler
                .reserve_permits(b"server", &[endpoint("extra", b"extra")])
                .is_empty(),
            "dropping the old generation must not mint capacity in the current one"
        );
        assert_eq!(current.start(), Some("current".into()));
    }

    #[test]
    fn queued_permit_cannot_commit_after_window_deadline_without_an_intervening_reservation() {
        let clock = ManualClock::new(7_000);
        let scheduler = EndpointDialScheduler::new_with_clock(
            EndpointDialConfig {
                window_ms: 100,
                process_limit: 1,
                server_limit: 1,
                peer_limit: 1,
                endpoint_limit: 1,
                prefix_limit: 1,
            },
            Arc::new(clock.clone()),
        );
        let endpoint = DialEndpoint::from_key_material(
            "queued",
            b"queued-socket",
            b"queued-prefix",
            CanonicalDialPeer(phase0(PEER_A)),
        );
        let permit = scheduler
            .reserve_permits(b"server", &[endpoint])
            .pop()
            .expect("initial window grants one permit");

        // No scheduler call happens between the deadline and commit. The permit itself must read
        // the scheduler's retained monotonic clock; relying on lazy rollover would start this old
        // attempt and then replenish the new window on the next reservation.
        clock.advance_ms(100);
        assert_eq!(permit.start(), None);
        assert_eq!(scheduler.state.lock().unwrap().process_spent, 0);
    }

    #[test]
    fn scheduler_windows_use_monotonic_time_and_reset_all_scopes() {
        let clock = ManualClock::new(10_000);
        let scheduler = EndpointDialScheduler::new_with_clock(
            EndpointDialConfig {
                window_ms: 100,
                process_limit: 1,
                server_limit: 1,
                peer_limit: 1,
                endpoint_limit: 1,
                prefix_limit: 1,
            },
            Arc::new(clock.clone()),
        );
        let endpoint = DialEndpoint::from_key_material(
            "route",
            b"socket",
            b"prefix",
            CanonicalDialPeer(phase0(PEER_A)),
        );
        assert_eq!(
            scheduler.reserve(b"server", std::slice::from_ref(&endpoint)),
            vec!["route"]
        );
        clock.set_wall_ms(1);
        assert!(
            scheduler
                .reserve(b"server", std::slice::from_ref(&endpoint))
                .is_empty(),
            "wall-clock correction must not refill the window"
        );
        clock.advance_ms(100);
        assert_eq!(scheduler.reserve(b"server", &[endpoint]), vec!["route"]);
    }

    #[test]
    fn zero_length_scheduler_window_cannot_refill_at_one_instant() {
        let clock = ManualClock::new(50);
        let scheduler = EndpointDialScheduler::new_with_clock(
            EndpointDialConfig {
                window_ms: 0,
                process_limit: 1,
                server_limit: 1,
                peer_limit: 1,
                endpoint_limit: 1,
                prefix_limit: 1,
            },
            Arc::new(clock.clone()),
        );
        let endpoint = DialEndpoint::from_key_material(
            "route",
            b"socket",
            b"prefix",
            CanonicalDialPeer(phase0(PEER_A)),
        );
        assert_eq!(
            scheduler.reserve(b"server", std::slice::from_ref(&endpoint)),
            vec!["route"]
        );
        assert!(scheduler
            .reserve(b"server", std::slice::from_ref(&endpoint))
            .is_empty());
        clock.advance_ms(1);
        assert_eq!(scheduler.reserve(b"server", &[endpoint]), vec!["route"]);
    }

    #[test]
    fn prefix_accounting_is_independent_of_transport_and_peer_identity() {
        let clock = ManualClock::new(0);
        let scheduler = EndpointDialScheduler::new_with_clock(
            EndpointDialConfig {
                window_ms: 1_000,
                process_limit: 4,
                server_limit: 4,
                peer_limit: 4,
                endpoint_limit: 4,
                prefix_limit: 1,
            },
            Arc::new(clock),
        );
        let tcp_route = format!("/ip4/203.0.113.9/tcp/4001/p2p/{PEER_A}");
        let quic_route = format!("/ip4/203.0.113.10/udp/4002/quic-v1/p2p/{PEER_B}");
        let tcp = parse_peer_dial_route(&tcp_route, &phase0(PEER_A))
            .unwrap()
            .endpoint;
        let quic = parse_peer_dial_route(&quic_route, &phase0(PEER_B))
            .unwrap()
            .endpoint;

        assert_eq!(scheduler.reserve(b"server-a", &[tcp]), vec![tcp_route]);
        assert!(
            scheduler.reserve(b"server-b", &[quic]).is_empty(),
            "TCP/QUIC and peer/server rotation must still share one IPv4 /24 bucket"
        );
    }

    #[test]
    fn shared_relay_circuits_do_not_alias_distinct_terminal_peers() {
        const PEER_C: &str = "12D3KooWPiZxJceHKQBZcd79cYdqybt5ijzRGHveTKa3CaEESxVb";
        let clock = ManualClock::new(0);
        let scheduler = EndpointDialScheduler::new_with_clock(
            EndpointDialConfig {
                window_ms: 1_000,
                process_limit: 8,
                server_limit: 8,
                peer_limit: 4,
                endpoint_limit: 2,
                prefix_limit: 8,
            },
            Arc::new(clock),
        );
        let routes = [PEER_A, PEER_B, PEER_C].map(|target| {
            let address =
                format!("/ip4/203.0.113.44/tcp/4001/p2p/{PEER_A}/p2p-circuit/p2p/{target}");
            let parsed = parse_peer_dial_route(&address, &phase0(target)).unwrap();
            assert!(matches!(parsed.kind, DialRouteKind::Relay { .. }));
            (address, parsed.endpoint)
        });
        let endpoints: Vec<_> = routes
            .iter()
            .map(|(_, endpoint)| endpoint.clone())
            .collect();

        assert_eq!(
            scheduler.reserve(b"one-server", &endpoints),
            routes
                .iter()
                .map(|(address, _)| address.clone())
                .collect::<Vec<_>>(),
            "a shared relay socket must not collapse unrelated terminal circuits into one bucket"
        );
        assert_eq!(
            scheduler.reserve(b"one-server", std::slice::from_ref(&endpoints[0])),
            vec![routes[0].0.clone()],
            "the per-circuit limit permits its configured second attempt"
        );
        assert!(
            scheduler
                .reserve(b"one-server", std::slice::from_ref(&endpoints[0]))
                .is_empty(),
            "the same relay circuit still obeys its exact-attempt cap"
        );
    }

    #[test]
    fn merging_unions_addresses_and_sources_for_one_peer() {
        let mut pol = ranking_policy();
        let clock = ManualClock::new(0);
        let mut r = rng();
        let candidates = vec![
            Candidate {
                peer: peer(5),
                addresses: vec!["/ip4/1.1.1.1/tcp/1".into()],
                source: Source::Rendezvous(rdv(1)),
                freshness: FreshnessPrincipal::Transport(peer(5)),
                seq: 4,
                tag_verified: false,
            },
            Candidate {
                peer: peer(5),
                addresses: vec!["/ip4/2.2.2.2/tcp/2".into()],
                source: Source::Cache,
                freshness: FreshnessPrincipal::Device(peer(42)),
                seq: 7,
                tag_verified: false,
            },
        ];
        let plan = pol.plan(candidates, 5, &clock, &mut r);
        assert_eq!(plan.len(), 1, "the two records merge into one peer");
        assert_eq!(plan[0].addresses.len(), 2, "addresses union");
    }
}
