//! The zero-knowledge **rendezvous** node: occupancy admission, census defence, cookie sizing.
//!
//! ## What the upstream server actually does
//!
//! Read from `libp2p-rendezvous` 0.17.1 (`src/server.rs`):
//!
//! - `Registrations::is_allowed` refuses a registration once the table is full. There is **no
//!   eviction, no fairness and no source accounting**: first come, first served, forever. With
//!   the previous configuration (16,384 total at 128 per peer) **128 free keypairs** fill the
//!   global table.
//! - `Config::default()` is `min_ttl = MIN_TTL` (2 h) and `max_ttl = MAX_TTL` (**72 h**). So a
//!   squatter does not even need a two-hourly cron job: it can hold the whole table for three
//!   days per pass. While the table is full, **every** founder's registration and **every**
//!   joiner's discovery fails worldwide.
//! - `Registrations::get` matches every registration when `discover_namespace` is `None`
//!   (`None => Some(registration_id)` in the filter), and `limit` is whatever the caller asked
//!   for (`limit.unwrap_or(u64::MAX)`). One small anonymous request therefore returns the entire
//!   table: every blinded namespace label, every peer id, every advertised address. That is a
//!   global directory of every group and its members' addresses, and roughly five orders of
//!   magnitude of amplification.
//! - Every `Discover` inserts an LRU cookie holding the set of ids returned, sized by
//!   `max_cookies` **independently of** `max_registrations_total`. Worst-case memory is the
//!   product of the two, which is how a few thousand requests exhaust a small VPS.
//!
//! ## What this module can and cannot fix
//!
//! `rendezvous::server::Behaviour` owns its `libp2p_request_response::Behaviour` privately, and
//! `libp2p_request_response`'s `handler` module is private, so a wrapping behaviour **cannot see
//! a request before it is answered**. Concretely:
//!
//! - **Cannot, without forking upstream:** reject a namespace-less `Discover` before it is
//!   served; clamp the caller-supplied `limit` server-side; evict a registration under pressure.
//! - **Can, and does here:** cap the table and the cookie store so the worst case is bounded and
//!   the product of the two is a number an operator can afford; clamp the TTL band so squatting
//!   costs 2 h per pass instead of 72 h; quota connections **and registrations** per source
//!   prefix, and deny the *prefix* when either is exceeded; cap the **registrations returned per
//!   source prefix per window**, which is what bounds a census; detect a namespace-less `Discover`
//!   from the response it produced; rate-limit discovery per peer; and apply back-pressure through
//!   the admission behaviour as occupancy rises.
//!
//! ## Why the deny had to move from the peer to the prefix
//!
//! Every quota here used to be *detected* per prefix and *punished* per `PeerId`, and a `PeerId`
//! is a self-minted keypair. So from one IPv4 address: connect with a fresh keypair, register,
//! disconnect, repeat. The concurrent-connection quota never fired (one connection at a time), the
//! per-peer quota never fired (one registration per identity), and the per-prefix quota fired and
//! then denied an identity the attacker had already thrown away. All 8,192 slots filled from a
//! single address, one handshake and one keypair each, and because upstream cannot evict, a full
//! table means every server creation and every first-time join fails worldwide until entries
//! expire. [`crate::admission::Admission::deny_prefix_for`] is the fix; it is checked before Noise.
//!
//! ## The census defence, stated honestly
//!
//! The namespace-spanning test is **sound but evadable**, and the evasion is one protocol field.
//! `limit` is client-controlled and unclamped, a namespace-less `Discover` mints an all-namespaces
//! cookie, and upstream keys cookies by cookie alone rather than by requesting peer. So
//! `Discover { namespace: None, limit: Some(1) }` returns a one-element response, which spans one
//! namespace and looks entirely ordinary, and the caller pages the whole table one record at a
//! time. The test also cannot fire at all on a table holding a single namespace, which is the
//! state of a freshly deployed node: exactly when the census is cheapest.
//!
//! So the binding limit is [`RendezvousLimits::max_records_per_window`]: **records returned** per
//! source prefix per window, counted from the response, which is invariant to how the caller
//! chooses to page. That is a **rate limit, not a prevention**. It makes a full census cost tens of
//! minutes per source address instead of one request, and parallelising it costs addresses. If the
//! census must be prevented outright rather than slowed, the honest answer is to **vendor
//! `libp2p-rendezvous`'s `Registrations` store**, so a namespace-less `Discover` can be refused
//! before it is served and `limit` can be clamped server-side; nothing a wrapping behaviour can do
//! reaches that far. See `docs/design-zeroconf-reachability.md` P3.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use catcoms_rt::{Clock, SystemClock};
use futures::StreamExt;
use libp2p::core::transport::MemoryTransport;
use libp2p::core::upgrade::Version;
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{
    connection_limits, identify, noise, ping, rendezvous, yamux, Multiaddr, Swarm, SwarmBuilder,
    Transport,
};

use crate::admission::{AddrPrefix, Admission, AdmissionConfig};
use crate::fdlimit::check_open_file_limit;
use crate::identify_config;
use crate::infra_transport::{
    is_advertisable, metered_tcp_transport, metered_ws_transport, WsTlsConfig,
};
use crate::metering::ByteMeters;
use crate::NetError;

/// Deliberate, operator-tunable sizing for a rendezvous node.
#[derive(Debug, Clone)]
pub struct RendezvousLimits {
    /// Registrations the table may hold. This is the ceiling on the census a single successful
    /// namespace-less `Discover` can return, so it is also the amplification ceiling: at roughly
    /// 250 bytes per signed peer record, 8,192 records is about 2 MB. Halving it halves both the
    /// capacity and the worst-case dump.
    pub max_registrations_total: usize,
    /// Registrations one peer id may hold. Peer ids are **per server** in this product, so this
    /// is per (device, server): `rendezvous_namespaces()` yields up to 3 member labels (the
    /// grandfather window) plus one join namespace per outstanding invite, so 16 leaves room for
    /// a founder with a dozen invites out. Upstream's own default is 32; the previous
    /// configuration here was 128, which made 128 keypairs enough to fill 16,384 slots.
    ///
    /// Note this quota is **not** the real defence: peer ids are free. The binding constraint is
    /// [`RendezvousLimits::max_registrations_per_prefix`], which costs an attacker addresses.
    pub max_registrations_per_peer: usize,
    /// Registrations attributable to one source prefix. This is the quota that actually costs an
    /// attacker something, because peer ids are free and addresses are not; exceeding it denies
    /// the **prefix**, not the peer that happened to trip it, which is what makes that true.
    pub max_registrations_per_prefix: usize,
    /// Registration **records returned** to one source prefix per
    /// [`RendezvousLimits::query_window_secs`]. The census limit.
    ///
    /// Counting requests does not bound a census, because `limit` is client-controlled: a caller
    /// that asks for one record at a time stays under any per-request rate limit forever. Counting
    /// records is invariant to how the caller pages.
    ///
    /// 4,096 against an 8,192-row table means a single source needs two full windows to see
    /// everything, and it is cut off for `deny_cooldown_secs` the moment it passes the budget, so
    /// a complete census from one address costs about half an hour rather than one request.
    /// Legitimate use is far below it: a member discovers a handful of namespaces per 60 s tick,
    /// and even a 200-member group across the three grandfathered routing labels is ~600 records.
    pub max_records_per_window: u32,
    /// Stored discovery cookies. Worst-case cookie memory is `max_stored_cookies *
    /// max_registrations_total * ~16 bytes`, because a cookie holds the id set of everything it
    /// has already returned. At 256 x 8,192 that is about 33 MB, which is a number a 1 GB VPS
    /// survives. The previous 4,096 x 16,384 was about 1 GB.
    ///
    /// Sizing it *down* is safe for this product: the client never sends a cookie (see
    /// `MeshService::rendezvous_discover`), and upstream treats a cookie miss as "return
    /// everything again" rather than an error, so an evicted cookie degrades a third-party
    /// client's pagination to a full response instead of breaking it.
    pub max_stored_cookies: usize,
    /// Shortest TTL a client may ask for. Below the spec's 2 h recommendation on purpose: a
    /// shorter TTL is a *client* volunteering to churn its own slot faster, which is strictly
    /// good for the table.
    pub min_ttl_secs: u64,
    /// Longest TTL a client may ask for. Upstream allows 72 h, which lets a squatter hold a slot
    /// for three days per registration. Clamping to 2 h (the protocol's default TTL, which is
    /// what this product's client actually requests) means a squatter must re-register 12 times
    /// a day, and every pass has to get past the prefix quota again.
    pub max_ttl_secs: u64,
    /// Occupancy fraction (in percent) above which the node declares itself saturated: the
    /// per-prefix connection quota tightens and new inbound connections are refused. Back-pressure
    /// rather than a cliff.
    pub saturate_at_percent: u8,
    /// Occupancy fraction (in percent) below which saturation is released. Strictly below
    /// `saturate_at_percent`: without hysteresis the flag flaps at the registration cadence.
    pub desaturate_at_percent: u8,
    /// Discovery requests one peer may make per [`RendezvousLimits::query_window_secs`]. A member
    /// discovers a handful of namespaces per 60 s tick, so this is generous by design; it exists
    /// to bound repeat scraping, not to shape normal traffic.
    pub max_discovers_per_window: u32,
    /// The discovery rate window.
    pub query_window_secs: u64,
    /// How long an abusive caller (table dump, or over the discovery rate) is refused for.
    pub deny_cooldown_secs: u64,
    /// Inbound connections still completing their handshake.
    pub max_pending_incoming: u32,
    /// Established inbound connections.
    pub max_established_incoming: u32,
    /// Connections per peer.
    pub max_established_per_peer: u32,
    /// Soft cap on peers tracked by the byte meter. A rendezvous carries no payload, but every
    /// caller still gets a counter, and peer ids are free: without a cap the map grows with every
    /// identity that has ever connected. Sweeping it back to the live set once it passes this cap
    /// bounds that, and costs nothing, because a rendezvous has no per-peer byte budget for a
    /// reset to undermine.
    pub max_tracked_peers: usize,
    /// How often the sweep runs (clock refresh, rate-window rollover, occupancy re-evaluation).
    pub sweep_secs: u64,
    /// Per-source-prefix connection quotas.
    pub admission: AdmissionConfig,
}

impl Default for RendezvousLimits {
    fn default() -> Self {
        Self {
            max_registrations_total: 8_192,
            max_registrations_per_peer: 16,
            max_registrations_per_prefix: 64,
            max_records_per_window: 4_096,
            max_stored_cookies: 256,
            min_ttl_secs: 300,
            max_ttl_secs: 60 * 60 * 2,
            saturate_at_percent: 80,
            desaturate_at_percent: 60,
            max_discovers_per_window: 32,
            query_window_secs: 60,
            deny_cooldown_secs: 15 * 60,
            max_pending_incoming: 256,
            max_established_incoming: 4_096,
            max_established_per_peer: 8,
            max_tracked_peers: 8_192,
            sweep_secs: 10,
            admission: AdmissionConfig::default(),
        }
    }
}

/// Rough memory one stored registration id costs inside a discovery cookie.
const COOKIE_ID_BYTES: u64 = 16;

/// Ceiling on `max_stored_cookies * max_registrations_total`, i.e. on the stored-id count the
/// cookie store can reach. 4,194,304 ids is about 64 MB, which a 1 GB node survives.
const MAX_COOKIE_IDS: u64 = 4 * 1024 * 1024;

impl RendezvousLimits {
    /// The upstream `rendezvous::server::Config` these limits describe.
    pub fn to_server_config(&self) -> rendezvous::server::Config {
        rendezvous::server::Config::default()
            .with_max_registration_per_peer(self.max_registrations_per_peer)
            .with_max_registration_total(self.max_registrations_total)
            .with_max_stored_cookies(self.max_stored_cookies)
            .with_min_ttl(self.min_ttl_secs)
            .with_max_ttl(self.max_ttl_secs)
    }

    /// The connection caps these limits describe.
    pub fn to_connection_limits(&self) -> connection_limits::ConnectionLimits {
        connection_limits::ConnectionLimits::default()
            .with_max_pending_incoming(Some(self.max_pending_incoming))
            .with_max_established_incoming(Some(self.max_established_incoming))
            .with_max_established_per_peer(Some(self.max_established_per_peer))
    }

    fn validate(&self) -> Result<(), NetError> {
        if self.sweep_secs == 0 || self.query_window_secs == 0 {
            return Err(NetError::Build(
                "rendezvous sweep interval and query window must be non-zero".into(),
            ));
        }
        if self.desaturate_at_percent >= self.saturate_at_percent {
            return Err(NetError::Build(
                "the rendezvous desaturate threshold must be strictly below the saturate \
                 threshold, or the flag flaps"
                    .into(),
            ));
        }
        // The protocol's default TTL (2 h) is what a client that asks for nothing gets; a band
        // that excludes it rejects every well-behaved client.
        if self.min_ttl_secs > rendezvous::DEFAULT_TTL
            || self.max_ttl_secs < rendezvous::DEFAULT_TTL
        {
            return Err(NetError::Build(format!(
                "the TTL band {}..{} excludes the protocol default of {}s, so a client that \
                 requests no explicit TTL would be refused",
                self.min_ttl_secs,
                self.max_ttl_secs,
                rendezvous::DEFAULT_TTL
            )));
        }
        if self.max_registrations_per_peer > self.max_registrations_total {
            return Err(NetError::Build(
                "one peer's registration quota exceeds the whole table".into(),
            ));
        }
        if self.max_registrations_per_prefix > self.max_registrations_total {
            return Err(NetError::Build(
                "one source prefix's registration quota exceeds the whole table".into(),
            ));
        }
        // Cookie memory is the **product** of the two caps, because a cookie holds the id set of
        // everything it has already returned. The defaults are chosen against that product, and
        // nothing stopped an operator raising one of them and OOMing the node: 4,096 cookies over
        // a 16,384-row table is about 1 GB, which is the whole VPS.
        let cookie_ids =
            (self.max_stored_cookies as u64).saturating_mul(self.max_registrations_total as u64);
        if cookie_ids > MAX_COOKIE_IDS {
            return Err(NetError::Build(format!(
                "max_stored_cookies ({}) x max_registrations_total ({}) is {} stored registration \
                 ids, about {} MB at {} bytes each, which is more than a small node can hold. \
                 Lower either number; the cookie store is the one that is safe to shrink, because \
                 upstream treats a cookie miss as 'return everything again' rather than an error.",
                self.max_stored_cookies,
                self.max_registrations_total,
                cookie_ids,
                cookie_ids * COOKIE_ID_BYTES / (1024 * 1024),
                COOKIE_ID_BYTES,
            )));
        }
        Ok(())
    }
}

/// A rendezvous **server**'s behaviours: members register their (signed) peer
/// records under a blinded namespace and discover each other, without the server
/// learning group identity or content. Zero-knowledge like the relay; it only sees
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
    /// Per-source-prefix quotas plus the deny path the census and rate policies drive.
    pub admission: Admission,
}

pub(crate) fn rendezvous_behaviour(key: &libp2p::identity::Keypair) -> RendezvousBehaviour {
    rendezvous_behaviour_with(key, &RendezvousLimits::default())
}

fn rendezvous_behaviour_with(
    key: &libp2p::identity::Keypair,
    limits: &RendezvousLimits,
) -> RendezvousBehaviour {
    RendezvousBehaviour {
        rendezvous: rendezvous::server::Behaviour::new(limits.to_server_config()),
        identify: identify::Behaviour::new(identify_config(key)),
        ping: ping::Behaviour::default(),
        connection_limits: connection_limits::Behaviour::new(limits.to_connection_limits()),
        admission: Admission::new(limits.admission.clone(), 0),
    }
}

/// Build a TCP rendezvous-server swarm. Run it with [`run_rendezvous`].
pub fn build_rendezvous_swarm() -> Result<Swarm<RendezvousBehaviour>, NetError> {
    let swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_behaviour(rendezvous_behaviour)
        .map_err(|e| NetError::Build(e.to_string()))?
        .build();
    Ok(swarm)
}

/// Like [`build_rendezvous_swarm`] but with a **caller-supplied identity**, for a **stable peer id
/// across restarts** (a restart otherwise invalidates every invite carrying the rendezvous addr).
pub fn build_rendezvous_swarm_with_key(
    key: libp2p::identity::Keypair,
) -> Result<Swarm<RendezvousBehaviour>, NetError> {
    let swarm = SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
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
/// or content; only opaque namespace strings and signed peer records.
///
/// This is the **development / test** entry point: no occupancy policy, no census defence and no
/// rate limiting. A deployed node wants [`RendezvousNode`].
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

/// Why a caller was cut off. Kept as a type so the policy is testable without a swarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryVerdict {
    /// Serve it and carry on.
    Allow,
    /// The response spanned more than one namespace, which upstream can only produce for a
    /// **namespace-less** `Discover`: this caller asked for the entire table.
    TableDump,
    /// Over the per-peer discovery rate for this window.
    RateExceeded,
    /// Over the per-source-prefix budget of registration **records returned** for this window.
    /// The verdict that bounds a census, because it counts what the caller received rather than
    /// how many requests they chose to split it across.
    RecordBudgetExceeded,
}

/// A deployable rendezvous node: a sized server plus the admission, occupancy and query policy
/// that upstream does not provide.
pub struct RendezvousNode {
    swarm: Swarm<RendezvousBehaviour>,
    meters: ByteMeters,
    limits: RendezvousLimits,
    clock: Arc<dyn Clock>,
    policy: QueryPolicy,
    occupancy: Occupancy,
}

impl std::fmt::Debug for RendezvousNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendezvousNode")
            .field("peer", self.swarm.local_peer_id())
            .field("limits", &self.limits)
            .field("registrations", &self.occupancy.live.len())
            .finish_non_exhaustive()
    }
}

impl RendezvousNode {
    /// Build a rendezvous node with a persisted identity, explicit limits, and (optionally) a TLS
    /// server certificate for the TCP/443 WebSocket listener.
    pub fn build(
        key: libp2p::identity::Keypair,
        limits: RendezvousLimits,
        ws_tls: Option<WsTlsConfig>,
    ) -> Result<Self, NetError> {
        limits.validate()?;
        let meters = ByteMeters::new();
        let tcp_meters = meters.clone();
        let ws_meters = meters.clone();
        let behaviour_limits = limits.clone();
        // The meter keys its per-prefix counter on the same masks the admission layer denies on.
        let v4_bits = limits.admission.ipv4_prefix_bits;
        let v6_bits = limits.admission.ipv6_prefix_bits;
        let swarm = SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_other_transport(|k| metered_tcp_transport(k, &tcp_meters, v4_bits, v6_bits))
            .map_err(|e| NetError::Build(e.to_string()))?
            .with_other_transport(|k| metered_ws_transport(k, &ws_meters, ws_tls, v4_bits, v6_bits))
            .map_err(|e| NetError::Build(e.to_string()))?
            .with_behaviour(move |k| rendezvous_behaviour_with(k, &behaviour_limits))
            .map_err(|e| NetError::Build(e.to_string()))?
            .build();
        Ok(Self {
            swarm,
            meters,
            policy: QueryPolicy::new(
                limits.query_window_secs,
                limits.max_discovers_per_window,
                limits.max_records_per_window,
            ),
            occupancy: Occupancy::default(),
            limits,
            clock: Arc::new(SystemClock),
        })
    }

    /// Replace the clock (tests drive the sweep with a `ManualClock`).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// This node's libp2p peer id; the `/p2p/<id>` an invite embeds.
    pub fn local_peer_id(&self) -> libp2p::PeerId {
        *self.swarm.local_peer_id()
    }

    /// Live byte counters (a rendezvous carries no payload, but a scrape still costs bandwidth).
    pub fn meters(&self) -> ByteMeters {
        self.meters.clone()
    }

    /// Start listening on `addr`.
    pub fn listen_on(&mut self, addr: Multiaddr) -> Result<(), NetError> {
        self.swarm
            .listen_on(addr.clone())
            .map_err(|e| NetError::Listen(format!("{addr}: {e}")))?;
        Ok(())
    }

    /// Advertise a dialable address (a rendezvous does not need one to serve, but identify and
    /// any future dial-back path do).
    ///
    /// Validated the same way the relay's is. This used to accept anything at all, including
    /// wildcards and RFC1918 addresses, which is the same undialable-address-and-topology-
    /// disclosure problem P12 describes for the relay wearing a different hat: whatever is here
    /// goes out over `identify` to every peer that completes a handshake.
    pub fn add_external_address(&mut self, addr: Multiaddr) -> Result<(), NetError> {
        if !is_advertisable(&addr) {
            return Err(NetError::Listen(format!(
                "{addr} is not plausibly reachable from the internet (wildcard, private, \
                 carrier-grade-NAT, link-local or reserved space) and cannot be advertised: it \
                 would be handed to every peer over identify and dialable by none of them. Give \
                 the rendezvous node's real public address."
            )));
        }
        self.swarm.add_external_address(addr);
        Ok(())
    }

    /// Refuse to start when the process cannot open enough files for its configured connection
    /// cap; see [`crate::fdlimit`]. Public so a caller can pre-flight before its banner.
    pub fn check_fd_limit(&self) -> Result<(), NetError> {
        check_open_file_limit("rendezvous", self.limits.max_established_incoming)
    }

    /// Live registration count.
    pub fn registrations(&self) -> usize {
        self.occupancy.live.len()
    }

    /// Run until the process is stopped.
    pub async fn run(mut self) -> Result<(), NetError> {
        self.check_fd_limit()?;
        let tick = Duration::from_secs(self.limits.sweep_secs);
        let mut last_sweep_ms = 0u64;
        loop {
            // See `RelayNode::run`: a timeout around the cancellation-safe `select_next_some` is
            // the periodic wake-up, and all policy time comes from the injected Clock.
            if let Ok(event) = tokio::time::timeout(tick, self.swarm.select_next_some()).await {
                self.on_event(event);
            }
            self.sweep(&mut last_sweep_ms);
        }
    }

    fn on_event(&mut self, event: SwarmEvent<RendezvousBehaviourEvent>) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                tracing::info!(%address, "rendezvous listening");
            }
            SwarmEvent::Behaviour(RendezvousBehaviourEvent::Rendezvous(e)) => {
                self.on_rendezvous_event(e);
            }
            _ => {}
        }
    }

    fn on_rendezvous_event(&mut self, event: rendezvous::server::Event) {
        let now = self.clock.now_ms();
        match event {
            rendezvous::server::Event::PeerRegistered { peer, registration } => {
                let prefix = self.swarm.behaviour().admission.prefix_of_peer(&peer);
                let added = self
                    .occupancy
                    .add(peer, registration.namespace.clone(), prefix);
                tracing::info!(
                    %peer,
                    total = self.occupancy.live.len(),
                    "rendezvous: peer registered"
                );
                if !added {
                    return; // a renewal of an existing (peer, namespace) slot: nothing new held
                }
                // Per-prefix occupancy quota. Upstream exposes no way to *remove* a registration,
                // so the registration itself stands until its TTL; what we can do is stop the
                // source adding more, immediately and for a cooldown.
                if let Some(prefix) = prefix {
                    // `held` already counts the registration just added, so the test is `>=`:
                    // with `>` the prefix got to hold `limit + 1` slots before anything fired,
                    // which made the effective quota 65 where the documentation said 64.
                    let held = self.occupancy.per_prefix.get(&prefix).copied().unwrap_or(0);
                    if held >= self.limits.max_registrations_per_prefix {
                        tracing::warn!(
                            %peer,
                            %prefix,
                            held,
                            limit = self.limits.max_registrations_per_prefix,
                            "source prefix is over its registration quota; refusing the whole \
                             prefix (denying the peer alone is free to evade: it just makes \
                             another keypair)"
                        );
                        self.deny_prefix(prefix, now);
                        self.deny(peer, now);
                    }
                }
            }
            rendezvous::server::Event::PeerUnregistered { peer, namespace } => {
                self.occupancy.remove(&peer, &namespace);
            }
            rendezvous::server::Event::RegistrationExpired(reg) => {
                self.occupancy.remove(&reg.record.peer_id(), &reg.namespace);
            }
            rendezvous::server::Event::DiscoverServed {
                enquirer,
                registrations,
            } => {
                let prefix = self.swarm.behaviour().admission.prefix_of_peer(&enquirer);
                let verdict = self.policy.judge(enquirer, prefix, &registrations, now);
                match verdict {
                    QueryVerdict::Allow => {}
                    QueryVerdict::TableDump => {
                        tracing::warn!(
                            peer = %enquirer,
                            returned = registrations.len(),
                            "a namespace-less Discover dumped the registration table; cutting the \
                             caller off (upstream answers before a wrapper can refuse, so the \
                             first dump succeeds)"
                        );
                        self.deny_source(enquirer, prefix, now);
                    }
                    QueryVerdict::RateExceeded => {
                        tracing::warn!(
                            peer = %enquirer,
                            window_secs = self.limits.query_window_secs,
                            limit = self.limits.max_discovers_per_window,
                            "discovery rate exceeded; cutting the caller off"
                        );
                        self.deny_source(enquirer, prefix, now);
                    }
                    QueryVerdict::RecordBudgetExceeded => {
                        tracing::warn!(
                            peer = %enquirer,
                            ?prefix,
                            window_secs = self.limits.query_window_secs,
                            limit = self.limits.max_records_per_window,
                            "source has been served its whole window budget of registration \
                             records; cutting the prefix off. This is what bounds a paginated \
                             census: the caller controls how many requests it makes, not how many \
                             records it receives"
                        );
                        self.deny_source(enquirer, prefix, now);
                    }
                }
            }
            other => tracing::debug!(?other, "rendezvous event"),
        }
    }

    /// Refuse this peer for the cooldown and drop it now, so the refusal is immediate rather than
    /// a silent stall.
    ///
    /// On its own this stops nothing durable: the caller mints another keypair. Every abuse path
    /// pairs it with [`RendezvousNode::deny_prefix`].
    fn deny(&mut self, peer: libp2p::PeerId, now_ms: u64) {
        let admission = &mut self.swarm.behaviour_mut().admission;
        admission.set_now_ms(now_ms);
        admission.deny_peer_for(peer, self.limits.deny_cooldown_secs.saturating_mul(1_000));
        let _ = self.swarm.disconnect_peer_id(peer);
    }

    /// Refuse an entire **source prefix** for the cooldown, and drop every peer currently attached
    /// from it. The refusal that costs the offender addresses rather than keypairs.
    fn deny_prefix(&mut self, prefix: AddrPrefix, now_ms: u64) {
        let cooldown = self.limits.deny_cooldown_secs.saturating_mul(1_000);
        let admission = &mut self.swarm.behaviour_mut().admission;
        admission.set_now_ms(now_ms);
        admission.deny_prefix_for(prefix, cooldown);
        let peers = admission.peers_in_prefix(&prefix);
        for peer in peers {
            self.swarm
                .behaviour_mut()
                .admission
                .deny_peer_for(peer, cooldown);
            let _ = self.swarm.disconnect_peer_id(peer);
        }
    }

    /// Deny both halves of a caller: the identity now, and the address it came from for as long as
    /// the cooldown runs. `prefix` is `None` only on the memory transport, which has no address.
    fn deny_source(&mut self, peer: libp2p::PeerId, prefix: Option<AddrPrefix>, now_ms: u64) {
        if let Some(prefix) = prefix {
            self.deny_prefix(prefix, now_ms);
        }
        self.deny(peer, now_ms);
    }

    fn sweep(&mut self, last_sweep_ms: &mut u64) {
        let now = self.clock.now_ms();
        if now.saturating_sub(*last_sweep_ms) < self.limits.sweep_secs.saturating_mul(1_000) {
            return;
        }
        *last_sweep_ms = now;
        self.swarm.behaviour_mut().admission.set_now_ms(now);
        self.policy.prune(now);
        // Peer-id churn would otherwise pin one byte counter per identity for the life of the
        // process. Only counters with no live connection behind them go.
        let churn = self.meters.reap_if_over(self.limits.max_tracked_peers);
        if churn > 0 {
            tracing::debug!(churn, "reaped disconnected peer meters over the soft cap");
        }

        // Occupancy back-pressure with hysteresis: as the table fills, the per-prefix connection
        // quota tightens, so a source that is already holding many connections gets no more while
        // a source holding none can still get its first few. That is breadth-first degradation,
        // and it is all it is: nothing here reserves the remaining registration *slots* for
        // sources that hold none, because upstream owns the table and will not let a wrapper
        // reject a registration. The earlier comment claimed the slots were steered, which was
        // never what the code did.
        let live = self.occupancy.live.len();
        let total = self.limits.max_registrations_total.max(1);
        let pct = (live.saturating_mul(100) / total) as u8;
        let admission = &mut self.swarm.behaviour_mut().admission;
        if !admission.is_saturated() && pct >= self.limits.saturate_at_percent {
            tracing::warn!(
                live,
                total,
                pct,
                "rendezvous table filling; applying back-pressure"
            );
            admission.set_saturated(true);
        } else if admission.is_saturated() && pct <= self.limits.desaturate_at_percent {
            tracing::info!(
                live,
                total,
                pct,
                "rendezvous table drained; back-pressure released"
            );
            admission.set_saturated(false);
        }
    }
}

/// Live registration bookkeeping. Upstream tracks this internally and exposes only events, so it
/// is mirrored here to drive occupancy back-pressure and the per-prefix quota.
#[derive(Debug, Default)]
struct Occupancy {
    live: HashSet<(libp2p::PeerId, rendezvous::Namespace)>,
    prefix_of: HashMap<(libp2p::PeerId, rendezvous::Namespace), AddrPrefix>,
    per_prefix: HashMap<AddrPrefix, usize>,
}

impl Occupancy {
    /// Record a registration. Returns whether it is a *new* slot (a re-registration of the same
    /// `(peer, namespace)` replaces the old one upstream and holds no extra capacity).
    fn add(
        &mut self,
        peer: libp2p::PeerId,
        namespace: rendezvous::Namespace,
        prefix: Option<AddrPrefix>,
    ) -> bool {
        let key = (peer, namespace);
        if !self.live.insert(key.clone()) {
            return false;
        }
        if let Some(prefix) = prefix {
            self.prefix_of.insert(key, prefix);
            *self.per_prefix.entry(prefix).or_insert(0) += 1;
        }
        true
    }

    fn remove(&mut self, peer: &libp2p::PeerId, namespace: &rendezvous::Namespace) {
        let key = (*peer, namespace.clone());
        if !self.live.remove(&key) {
            return;
        }
        if let Some(prefix) = self.prefix_of.remove(&key) {
            if let Some(n) = self.per_prefix.get_mut(&prefix) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    self.per_prefix.remove(&prefix);
                }
            }
        }
    }
}

/// One party's usage inside the current query window: when the window opened, how many discovery
/// requests it has made, and how many registration records it has been served.
#[derive(Debug, Clone, Copy, Default)]
struct QueryUsage {
    window_start_ms: u64,
    requests: u32,
    records: u32,
}

impl QueryUsage {
    /// Roll the window if this one has elapsed, then charge `records` and one request.
    fn charge(&mut self, now_ms: u64, window_ms: u64, records: u32) {
        if now_ms.saturating_sub(self.window_start_ms) >= window_ms {
            *self = QueryUsage {
                window_start_ms: now_ms,
                requests: 0,
                records: 0,
            };
        }
        self.requests = self.requests.saturating_add(1);
        self.records = self.records.saturating_add(records);
    }
}

/// Per-peer discovery rate accounting, per-prefix **record** accounting, and table-dump detection.
#[derive(Debug)]
struct QueryPolicy {
    window_ms: u64,
    max_per_window: u32,
    max_records_per_window: u32,
    seen: HashMap<libp2p::PeerId, QueryUsage>,
    /// Records served per source prefix. Keyed on the address rather than the identity, because
    /// the identity is free and a scraper rotates it between requests to stay under the per-peer
    /// rate limit.
    by_prefix: HashMap<AddrPrefix, QueryUsage>,
}

impl QueryPolicy {
    fn new(window_secs: u64, max_per_window: u32, max_records_per_window: u32) -> Self {
        Self {
            window_ms: window_secs.saturating_mul(1_000),
            max_per_window,
            max_records_per_window,
            seen: HashMap::new(),
            by_prefix: HashMap::new(),
        }
    }

    /// Judge one served `Discover`, charging its cost first and deciding afterwards.
    ///
    /// Accounting comes before every verdict on purpose. The first version returned `TableDump`
    /// before touching the rate counter, so a caller that alternated a dump with ordinary queries
    /// paid nothing toward its own rate limit for the dumps.
    ///
    /// Two tests, in increasing order of how much they are worth:
    ///
    /// 1. **Namespace spanning.** `Registrations::get` filters by namespace when one was given, so
    ///    a single response spanning two namespaces can only have come from a `Discover` with no
    ///    namespace. Sound, no false positives, and it cannot be aimed at a third party. It is also
    ///    evaded by asking for `limit: 1`, and it cannot fire at all while the table holds a single
    ///    namespace, which is a freshly deployed node.
    /// 2. **Records served per source prefix.** The one that binds. The caller chooses how many
    ///    requests to split a census into; it does not choose how many records come back, and the
    ///    record count is the census. See the module docs for what this does and does not achieve.
    fn judge(
        &mut self,
        peer: libp2p::PeerId,
        prefix: Option<AddrPrefix>,
        registrations: &[rendezvous::Registration],
        now_ms: u64,
    ) -> QueryVerdict {
        let served = u32::try_from(registrations.len()).unwrap_or(u32::MAX);
        let peer_usage = self.seen.entry(peer).or_default();
        peer_usage.charge(now_ms, self.window_ms, served);
        let over_rate = peer_usage.requests > self.max_per_window;

        let over_records = match prefix {
            Some(prefix) => {
                let usage = self.by_prefix.entry(prefix).or_default();
                usage.charge(now_ms, self.window_ms, served);
                usage.records > self.max_records_per_window
            }
            // The memory transport has no address, so there is nothing to charge; the per-peer
            // rate limit is all that applies there. Only tests take that path.
            None => false,
        };

        let mut namespaces = registrations.iter().map(|r| &r.namespace);
        if let Some(first) = namespaces.next() {
            if namespaces.any(|n| n != first) {
                return QueryVerdict::TableDump;
            }
        }
        if over_records {
            return QueryVerdict::RecordBudgetExceeded;
        }
        if over_rate {
            return QueryVerdict::RateExceeded;
        }
        QueryVerdict::Allow
    }

    /// Drop windows that have fully elapsed, so the maps do not grow with every peer id and source
    /// ever seen (which would itself be the memory-exhaustion bug in a different place).
    fn prune(&mut self, now_ms: u64) {
        let window = self.window_ms;
        self.seen
            .retain(|_, u| now_ms.saturating_sub(u.window_start_ms) < window);
        self.by_prefix
            .retain(|_, u| now_ms.saturating_sub(u.window_start_ms) < window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::PeerId;

    fn prefix(addr: &str) -> AddrPrefix {
        crate::admission::addr_prefix(&addr.parse::<Multiaddr>().unwrap(), 24, 56).unwrap()
    }

    fn registration(ns: &str) -> rendezvous::Registration {
        let id = libp2p::identity::Keypair::generate_ed25519();
        rendezvous::Registration {
            namespace: rendezvous::Namespace::new(ns.to_owned()).unwrap(),
            record: libp2p::core::PeerRecord::new(
                &id,
                vec!["/ip4/127.0.0.1/tcp/1".parse().unwrap()],
            )
            .unwrap(),
            ttl: 7_200,
        }
    }

    #[test]
    fn the_configured_limits_are_not_the_upstream_defaults() {
        // Upstream 0.17.1: 32 per peer, 10_000 total, 10_000 cookies, TTL band 2h..72h.
        assert_eq!(rendezvous::server::MAX_REGISTRATION_PEER, 32);
        assert_eq!(rendezvous::server::MAX_REGISTRATIONS_TOTAL, 10_000);
        assert_eq!(rendezvous::server::COOKIES_CACHE_SIZE, 10_000);
        assert_eq!(rendezvous::MAX_TTL, 60 * 60 * 72);

        let l = RendezvousLimits::default();
        // A squatter must not be able to hold a slot for three days.
        assert_eq!(l.max_ttl_secs, rendezvous::DEFAULT_TTL);
        assert!(l.max_ttl_secs < rendezvous::MAX_TTL);
        // Cookie memory is the product of the two caps; keep it under about 64 MB at 16 B/id.
        let worst_case_bytes = l.max_stored_cookies * l.max_registrations_total * 16;
        assert!(
            worst_case_bytes < 64 * 1024 * 1024,
            "worst-case cookie memory is {worst_case_bytes} bytes"
        );
        // Peer ids are free, so the meaningful cost of filling the table is *addresses*: with a
        // per-prefix registration quota, an attacker needs this many distinct /24s. The previous
        // configuration needed none at all, just 128 keypairs from one machine.
        let prefixes_to_fill = l.max_registrations_total / l.max_registrations_per_prefix;
        assert!(
            prefixes_to_fill >= 64,
            "only {prefixes_to_fill} source networks would fill the table"
        );
        assert!(l.validate().is_ok());
    }

    #[test]
    fn a_cookie_store_that_could_exhaust_memory_is_refused() {
        // Worst-case cookie memory is the *product* of the two caps, and an operator could raise
        // either one on its own. 4,096 cookies over a 16,384-row table is about 1 GB.
        let l = RendezvousLimits {
            max_stored_cookies: 4_096,
            max_registrations_total: 16_384,
            ..Default::default()
        };
        let err = l.validate().unwrap_err().to_string();
        assert!(err.contains("max_stored_cookies"), "{err}");
        assert!(RendezvousLimits::default().validate().is_ok());

        // A prefix quota larger than the whole table is a quota that can never bind.
        assert!(RendezvousLimits {
            max_registrations_per_prefix: 99_999,
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn incoherent_limits_are_refused() {
        let base = RendezvousLimits::default();
        let l = RendezvousLimits {
            desaturate_at_percent: base.saturate_at_percent,
            ..Default::default()
        };
        assert!(l.validate().is_err());

        // A TTL band that excludes the protocol default would refuse every well-behaved client.
        let l = RendezvousLimits {
            max_ttl_secs: 60,
            ..Default::default()
        };
        assert!(l.validate().is_err());

        let l = RendezvousLimits {
            min_ttl_secs: rendezvous::DEFAULT_TTL + 1,
            ..Default::default()
        };
        assert!(l.validate().is_err());
    }

    #[test]
    fn a_namespace_less_discover_is_detected_from_its_response() {
        let mut p = QueryPolicy::new(60, 32, 4_096);
        let peer = PeerId::random();
        let src = Some(prefix("/ip4/198.51.100.1/tcp/1"));
        // One namespace: a normal, scoped query.
        assert_eq!(
            p.judge(peer, src, &[registration("a"), registration("a")], 0),
            QueryVerdict::Allow
        );
        // Two namespaces in one response can only come from a Discover with no namespace.
        assert_eq!(
            p.judge(peer, src, &[registration("a"), registration("b")], 0),
            QueryVerdict::TableDump
        );
        // An empty response is not a dump.
        assert_eq!(p.judge(peer, src, &[], 0), QueryVerdict::Allow);
    }

    #[test]
    fn the_discovery_rate_limit_actually_binds_and_then_resets() {
        let mut p = QueryPolicy::new(60, 3, 4_096);
        let peer = PeerId::random();
        let other = PeerId::random();
        let src = Some(prefix("/ip4/198.51.100.1/tcp/1"));
        for _ in 0..3 {
            assert_eq!(
                p.judge(peer, src, &[registration("a")], 1_000),
                QueryVerdict::Allow
            );
        }
        assert_eq!(
            p.judge(peer, src, &[registration("a")], 1_000),
            QueryVerdict::RateExceeded,
            "the fourth query in the window must be refused"
        );
        // The budget is per peer, not global.
        assert_eq!(
            p.judge(other, src, &[registration("a")], 1_000),
            QueryVerdict::Allow
        );
        // And it resets when the window rolls.
        assert_eq!(
            p.judge(peer, src, &[registration("a")], 62_000),
            QueryVerdict::Allow
        );
        // Pruning drops elapsed windows so the maps cannot grow without bound.
        p.prune(10_000_000);
        assert!(p.seen.is_empty());
        assert!(p.by_prefix.is_empty());
    }

    #[test]
    fn a_paginated_census_is_stopped_by_the_record_budget() {
        // HIGH 3. The namespace-spanning test is sound and is evaded by one protocol field:
        // `Discover { namespace: None, limit: Some(1) }` returns a one-element response, which
        // spans one namespace and looks entirely ordinary, and the caller pages the whole table one
        // record at a time while rotating its PeerId to stay under the per-peer rate limit. What
        // the caller cannot vary is how many records come back, so that is what is counted.
        let budget = 8u32;
        let mut p = QueryPolicy::new(60, u32::MAX, budget);
        let src = Some(prefix("/ip4/198.51.100.1/tcp/1"));

        // A fresh identity for every single-record request, exactly as the attack does.
        for i in 0..budget {
            assert_eq!(
                p.judge(PeerId::random(), src, &[registration("a")], 1_000),
                QueryVerdict::Allow,
                "record {i} is inside the budget"
            );
        }
        assert_eq!(
            p.judge(PeerId::random(), src, &[registration("a")], 1_000),
            QueryVerdict::RecordBudgetExceeded,
            "rotating the identity must not reset the budget: it is keyed on the address"
        );

        // A different source network is unaffected, so the census costs addresses.
        assert_eq!(
            p.judge(
                PeerId::random(),
                Some(prefix("/ip4/203.0.113.1/tcp/1")),
                &[registration("a")],
                1_000
            ),
            QueryVerdict::Allow
        );

        // A single large response is charged for every record in it, not counted as one query.
        let mut q = QueryPolicy::new(60, u32::MAX, budget);
        let big: Vec<rendezvous::Registration> = (0..budget as usize + 1)
            .map(|_| registration("a"))
            .collect();
        assert_eq!(
            q.judge(PeerId::random(), src, &big, 1_000),
            QueryVerdict::RecordBudgetExceeded
        );

        // And it resets when the window rolls, so an honest heavy member is not banned for life.
        assert_eq!(
            p.judge(PeerId::random(), src, &[registration("a")], 90_000),
            QueryVerdict::Allow
        );
    }

    #[test]
    fn a_table_dump_is_still_charged_to_the_rate_counter() {
        // The verdict used to return before the accounting ran, so dumps were free against the
        // caller's own rate limit.
        let mut p = QueryPolicy::new(60, 2, 4_096);
        let peer = PeerId::random();
        let src = prefix("/ip4/198.51.100.1/tcp/1");
        assert_eq!(
            p.judge(peer, Some(src), &[registration("a"), registration("b")], 0),
            QueryVerdict::TableDump
        );
        assert_eq!(p.seen[&peer].requests, 1, "the dump must be charged");
        assert_eq!(
            p.by_prefix[&src].records, 2,
            "and its records must be charged to the source"
        );
    }

    #[test]
    fn the_default_record_budget_leaves_room_for_a_large_group() {
        // The budget has to bound a census without touching real use. A 200-member group across
        // the three grandfathered routing labels is ~600 records per 60 s tick.
        let l = RendezvousLimits::default();
        assert!(l.max_records_per_window >= 1_024);
        // And a full census must still cost more than one window from a single address.
        assert!(
            (l.max_registrations_total as u32) > l.max_records_per_window,
            "the whole table fits in one window's record budget"
        );
    }

    #[test]
    fn occupancy_counts_slots_not_renewals() {
        let mut o = Occupancy::default();
        let peer = PeerId::random();
        let prefix = crate::admission::addr_prefix(
            &"/ip4/198.51.100.1/tcp/1".parse::<Multiaddr>().unwrap(),
            24,
            64,
        );
        let ns = rendezvous::Namespace::new("g".into()).unwrap();

        assert!(o.add(peer, ns.clone(), prefix));
        assert!(
            !o.add(peer, ns.clone(), prefix),
            "a renewal replaces upstream and must not double-count"
        );
        // The quota test in `on_rendezvous_event` is `held >= limit` against this count, so a
        // limit of N leaves the prefix holding exactly N slots. With `>` it held N + 1.
        assert_eq!(o.live.len(), 1);
        assert_eq!(o.per_prefix.values().sum::<usize>(), 1);

        let ns2 = rendezvous::Namespace::new("h".into()).unwrap();
        assert!(o.add(peer, ns2.clone(), prefix));
        assert_eq!(o.per_prefix.values().sum::<usize>(), 2);

        o.remove(&peer, &ns);
        o.remove(&peer, &ns2);
        assert_eq!(o.live.len(), 0);
        assert!(
            o.per_prefix.is_empty(),
            "the prefix charge must be released"
        );
    }
}
