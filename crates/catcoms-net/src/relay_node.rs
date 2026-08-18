//! The zero-knowledge **circuit relay** node: sizing, byte accounting, load shedding.
//!
//! A relay forwards Noise + MLS ciphertext between two peers that cannot connect directly. It
//! never sees plaintext, but it does see (and pay for) every byte. Two things follow, and both
//! were missing before this module existed.
//!
//! **1. The upstream defaults are lab defaults.** `libp2p-relay` 0.21.1
//! (`src/behaviour.rs`, `impl Default for Config`) ships:
//!
//! | field | default | what it means here |
//! |---|---|---|
//! | `max_circuit_bytes` | `1 << 17` = 128 KiB | summed over **both** directions (`copy_future.rs` adds each `forward_data` result into one `bytes_sent`), then the circuit is killed with `Max circuit bytes reached` |
//! | `max_circuit_duration` | 120 s | the circuit dies at two minutes regardless of traffic |
//! | `max_circuits` | 16 | **for the entire node**, not per peer |
//! | `max_reservations` | 128 | the 129th group ever to want reachability is refused |
//! | `max_reservations_per_peer` | 4 | fine |
//! | `reservation_duration` | 3600 s | fine |
//!
//! Against the product that is: an avatar at the 16 MiB blob cap is over 100x the whole circuit
//! budget; a chunked file transfer likewise; a voice call at roughly 32 kbit/s each way spends
//! 128 KiB in about 16 seconds of two-way audio and would die at 120 seconds anyway. The relay
//! as configured could not carry a single feature the product has.
//!
//! **2. Nothing counted the bytes.** `libp2p-relay` bounds one circuit and counts circuits; it
//! has no aggregate accounting, so a deliberately-sized relay is a bandwidth commitment nobody
//! can measure against. [`crate::metering`] adds per-peer accounting in the transport, and this
//! module turns it into a load-shed path: a peer over its budget is disconnected and refused for
//! a cooldown, and a node over its aggregate budget refuses new connections **cleanly** instead
//! of accepting them and timing out. A saturated node that says "no" is a user who tries another
//! rung; a saturated node that hangs is a user who uninstalls.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use catcoms_rt::{Clock, SystemClock};
use futures::StreamExt;
use libp2p::core::transport::MemoryTransport;
use libp2p::core::upgrade::Version;
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{
    connection_limits, identify, noise, ping, relay, yamux, Multiaddr, Swarm, SwarmBuilder,
    Transport,
};

use crate::admission::{Admission, AdmissionConfig};
use crate::identify_config;
use crate::infra_transport::{
    is_wildcard_addr, metered_tcp_transport, metered_ws_transport, WsTlsConfig,
};
use crate::metering::ByteMeters;
use crate::NetError;

/// Deliberate, operator-tunable sizing for a relay. Every number is a bandwidth or memory
/// commitment; the doc comment on each says what it costs.
///
/// The defaults target a single small VPS (one vCPU, 1 GB RAM, a 1 Gbit port with a metered
/// monthly allowance). They are **not** the largest safe values; they are the values whose bill
/// an individual operator can absorb without checking.
#[derive(Debug, Clone)]
pub struct RelayLimits {
    /// Concurrent reservations, i.e. how many NAT'd nodes can be *reachable* through this relay
    /// at once. A reservation is one idle TCP connection plus a map entry: cheap in bandwidth,
    /// real in file descriptors. 4096 reservations is roughly 4096 open sockets, which needs an
    /// `ulimit -n` above the usual 1024 default. Upstream ships 128, which is 128 groups
    /// worldwide.
    pub max_reservations: usize,
    /// Reservations one peer may hold. A device holds one per server it wants reachable, so this
    /// is "servers per device" and 4 is the upstream default.
    pub max_reservations_per_peer: usize,
    /// How long a granted reservation lasts before the client must renew. Shorter recycles slots
    /// from vanished clients faster; longer costs less renewal traffic. One hour is upstream's.
    pub reservation_duration_secs: u64,
    /// Concurrent **circuits**: live forwarded connections. This is the number that decides peak
    /// bandwidth, because every circuit can be moving bytes at once. At 512 circuits each doing a
    /// modest 32 kbit/s of voice both ways, the node is pushing about 4 MB/s. Upstream ships 16.
    pub max_circuits: usize,
    /// Circuits one source peer may open. Bounds how much of the node one member can occupy
    /// before the byte budget notices.
    pub max_circuits_per_peer: usize,
    /// Hard lifetime of a circuit. This is a slot-recycling limit, not a cost limit (the byte
    /// budget is the cost limit): it stops an idle-but-open circuit squatting a slot forever.
    /// Four hours is long enough that a long call or a big transfer is not interrupted, and short
    /// enough that abandoned circuits clear within a working day. Upstream ships 120 seconds.
    pub max_circuit_duration_secs: u64,
    /// Bytes one circuit may move before it is torn down, **summed over both directions**. 256
    /// MiB carries: a 16 MiB avatar sixteen times over; a chunked file transfer of a few hundred
    /// megabytes; or a voice call at 32 kbit/s each way for about nine hours. Set to 0 to disable
    /// the per-circuit cap entirely and rely on the byte budget alone (upstream treats 0 as
    /// unlimited). Upstream ships 128 KiB.
    pub max_circuit_bytes: u64,
    /// Inbound connections still completing their handshake. Bounds a SYN-ish flood.
    pub max_pending_incoming: u32,
    /// Established inbound connections. Must exceed `max_reservations`, because a reservation
    /// *is* a held connection and circuit sources need connections of their own on top.
    pub max_established_incoming: u32,
    /// Connections per peer. Above 1 only because a client may hold a reservation and dial a
    /// circuit at the same time.
    pub max_established_per_peer: u32,
    /// The budget window. All byte budgets are "this many bytes per this many seconds".
    pub budget_window_secs: u64,
    /// Bytes one peer may move per window before it is shed. Counted in both directions,
    /// including Noise and yamux framing (the operator pays for frames, not for payload).
    /// 1 GiB/hour is about 36 hours of voice per hour, or 64 avatar fetches per hour: generous
    /// for a member, immediately obvious for an abuser.
    pub peer_budget_bytes: u64,
    /// Bytes the whole node may move per window before it starts refusing new connections.
    /// 8 GiB/hour is roughly 2.3 MB/s sustained, about 5.8 TB of traffic a month, of which
    /// roughly half is egress. That fits inside the monthly allowance of a cheap VPS with room
    /// to spare. Raising it raises the bill linearly and nothing else.
    pub node_budget_bytes: u64,
    /// How long a peer that blew its budget is refused for. Long enough to make a retry loop
    /// pointless, short enough that a member who genuinely transferred a lot is back the same
    /// session.
    pub shed_cooldown_secs: u64,
    /// How often the shed sweep runs. Also how often the admission behaviour's clock is refreshed
    /// and how stale a budget reading can be, so a peer can overshoot by at most one sweep's
    /// worth of traffic.
    pub sweep_secs: u64,
    /// Per-source-prefix connection quotas (a `/24` or a `/64`), the only dimension a `PeerId`
    /// flood cannot evade for free.
    pub admission: AdmissionConfig,
}

impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            max_reservations: 4_096,
            max_reservations_per_peer: 4,
            reservation_duration_secs: 60 * 60,
            max_circuits: 512,
            max_circuits_per_peer: 8,
            max_circuit_duration_secs: 4 * 60 * 60,
            max_circuit_bytes: 256 * 1024 * 1024,
            max_pending_incoming: 256,
            max_established_incoming: 8_192,
            max_established_per_peer: 8,
            budget_window_secs: 60 * 60,
            peer_budget_bytes: 1024 * 1024 * 1024,
            node_budget_bytes: 8 * 1024 * 1024 * 1024,
            shed_cooldown_secs: 15 * 60,
            sweep_secs: 10,
            // A relay over its aggregate byte budget has nothing left to give: refuse outright,
            // so the client fails fast and tries another rung instead of stalling on a socket.
            admission: AdmissionConfig {
                refuse_all_when_saturated: true,
                ..AdmissionConfig::default()
            },
        }
    }
}

impl RelayLimits {
    /// The upstream `relay::Config` these limits describe.
    ///
    /// Built from `relay::Config::default()` and then overwritten field by field, deliberately:
    /// the default also installs the per-peer and per-IP **rate** limiters (30 reservations per
    /// 2 minutes per peer, 60 per minute per IP, and the same shape for circuit sources), which
    /// are sensible and are not reachable through any other constructor.
    pub fn to_relay_config(&self) -> relay::Config {
        relay::Config {
            max_reservations: self.max_reservations,
            max_reservations_per_peer: self.max_reservations_per_peer,
            reservation_duration: Duration::from_secs(self.reservation_duration_secs),
            max_circuits: self.max_circuits,
            max_circuits_per_peer: self.max_circuits_per_peer,
            max_circuit_duration: Duration::from_secs(self.max_circuit_duration_secs),
            max_circuit_bytes: self.max_circuit_bytes,
            ..relay::Config::default()
        }
    }

    /// The connection caps these limits describe.
    pub fn to_connection_limits(&self) -> connection_limits::ConnectionLimits {
        connection_limits::ConnectionLimits::default()
            .with_max_pending_incoming(Some(self.max_pending_incoming))
            .with_max_established_incoming(Some(self.max_established_incoming))
            .with_max_established_per_peer(Some(self.max_established_per_peer))
    }

    /// Reject a configuration that cannot work, before a node is started with it.
    fn validate(&self) -> Result<(), NetError> {
        if self.budget_window_secs == 0 || self.sweep_secs == 0 {
            return Err(NetError::Build(
                "relay budget window and sweep interval must be non-zero".into(),
            ));
        }
        if (self.max_established_incoming as usize) < self.max_reservations {
            return Err(NetError::Build(format!(
                "max_established_incoming ({}) is below max_reservations ({}): a reservation is a \
                 held connection, so reservations would be refused by the connection cap instead \
                 of by the reservation cap",
                self.max_established_incoming, self.max_reservations
            )));
        }
        if self.peer_budget_bytes > self.node_budget_bytes {
            return Err(NetError::Build(
                "a single peer's byte budget exceeds the whole node's".into(),
            ));
        }
        Ok(())
    }
}

/// A relay **server**'s behaviours: forward circuit traffic between peers that cannot connect
/// directly. Never sees plaintext (Noise + MLS ciphertext only).
#[derive(NetworkBehaviour)]
#[allow(missing_debug_implementations)]
pub struct RelayBehaviour {
    /// The circuit-relay-v2 server.
    pub relay: relay::Behaviour,
    /// Address discovery for clients reserving slots.
    pub identify: identify::Behaviour,
    /// Keep-alive.
    pub ping: ping::Behaviour,
    /// Connection caps. This swarm was the only internet-exposed one without them, which
    /// contradicted the "connection limits on every swarm" claim in `HANDOVER.md`.
    pub connection_limits: connection_limits::Behaviour,
    /// Per-source-prefix quotas plus the load-shed deny path.
    pub admission: Admission,
}

pub(crate) fn relay_behaviour(key: &libp2p::identity::Keypair) -> RelayBehaviour {
    relay_behaviour_with(key, &RelayLimits::default())
}

fn relay_behaviour_with(key: &libp2p::identity::Keypair, limits: &RelayLimits) -> RelayBehaviour {
    RelayBehaviour {
        relay: relay::Behaviour::new(key.public().to_peer_id(), limits.to_relay_config()),
        identify: identify::Behaviour::new(identify_config(key)),
        ping: ping::Behaviour::default(),
        connection_limits: connection_limits::Behaviour::new(limits.to_connection_limits()),
        admission: Admission::new(limits.admission.clone(), 0),
    }
}

/// Build a TCP **relay-server** swarm (forwards circuit traffic for clients behind
/// NAT). Run it with [`run_relay`].
pub fn build_relay_swarm() -> Result<Swarm<RelayBehaviour>, NetError> {
    let swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_behaviour(relay_behaviour)
        .map_err(|e| NetError::Build(e.to_string()))?
        .build();
    Ok(swarm)
}

/// Like [`build_relay_swarm`] but with a **caller-supplied identity**, so an operator can persist
/// the relay's keypair and keep a **stable peer id across restarts**. Otherwise every restart
/// generates a fresh id and silently invalidates every invite that embedded the relay's multiaddr.
pub fn build_relay_swarm_with_key(
    key: libp2p::identity::Keypair,
) -> Result<Swarm<RelayBehaviour>, NetError> {
    let swarm = SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
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

/// Run a relay-server swarm's event loop: forward circuit traffic indefinitely.
/// (Relays only ever route Noise + MLS ciphertext; zero-knowledge.)
///
/// This is the **development / test** entry point: no byte accounting, no load shed, and no
/// startup validation of the advertised address. A deployed node wants [`RelayNode`], which has
/// all three.
///
/// Each bound listen address is registered as an external address so granted reservations carry a
/// usable relayed address (otherwise a client's circuit listener closes with
/// `NoAddressesInReservation`); **wildcard binds are excluded**, because advertising `0.0.0.0`
/// tells the client to dial nothing at all.
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
                advertise_if_dialable(&mut swarm, &address);
                tracing::info!(%address, "relay listening");
            }
            SwarmEvent::Behaviour(RelayBehaviourEvent::Relay(e)) => {
                tracing::debug!(?e, "relay event");
            }
            _ => {}
        }
    }
}

/// Advertise a bound listen address as external, unless it is a wildcard bind.
///
/// P12, stated precisely. `add_external_address` on every `NewListenAddr` is what puts an address
/// into the reservation handed to clients. A `0.0.0.0` bind rarely surfaces *as* `0.0.0.0`,
/// because `libp2p-tcp` expands a wildcard into one `NewListenAddr` per interface; what actually
/// lands in reservations is the node's **private** addresses (`192.168.x.x`, `10.x.x.x`,
/// loopback). Those are undialable from the internet, so the client's circuit listener closes and
/// the user sees a timeout, and they are an unforced disclosure of the operator's internal
/// topology on top. Either way the operator is told nothing.
///
/// This filter catches the literal wildcard (some platforms and transports do surface it), and
/// [`RelayNode`] handles the real case two ways: it refuses to start when the only listeners are
/// wildcard binds and no external address was given, and it suppresses auto-advertisement
/// entirely once an operator has stated the node's real address.
fn advertise_if_dialable(swarm: &mut Swarm<RelayBehaviour>, address: &Multiaddr) {
    if is_wildcard_addr(address) {
        tracing::warn!(
            %address,
            "not advertising a wildcard listen address: reservations would carry an undialable \
             address. Supply the relay's real public address explicitly."
        );
        return;
    }
    swarm.add_external_address(address.clone());
}

/// A deployable relay: a sized swarm, the byte accounting behind it, and the load-shed policy
/// that turns the accounting into refusals.
///
/// Built rather than assembled by hand so the three cannot drift apart: the limits that shaped
/// the `relay::Config` are the same limits the shed sweep enforces.
pub struct RelayNode {
    swarm: Swarm<RelayBehaviour>,
    meters: ByteMeters,
    limits: RelayLimits,
    clock: Arc<dyn Clock>,
    listen: Vec<Multiaddr>,
    external: Vec<Multiaddr>,
}

impl std::fmt::Debug for RelayNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayNode")
            .field("peer", self.swarm.local_peer_id())
            .field("limits", &self.limits)
            .field("listen", &self.listen)
            .field("external", &self.external)
            .finish_non_exhaustive()
    }
}

impl RelayNode {
    /// Build a relay with a persisted identity, explicit limits, and (optionally) a TLS server
    /// certificate for the TCP/443 WebSocket listener.
    ///
    /// The WebSocket transport is always installed; installing it opens nothing. What opens a
    /// port is [`RelayNode::listen_on`] with a `/ws` or `/tls/ws` address, which is what keeps
    /// rung 4 opt-in.
    pub fn build(
        key: libp2p::identity::Keypair,
        limits: RelayLimits,
        ws_tls: Option<WsTlsConfig>,
    ) -> Result<Self, NetError> {
        limits.validate()?;
        let meters = ByteMeters::new();
        let tcp_meters = meters.clone();
        let ws_meters = meters.clone();
        let behaviour_limits = limits.clone();
        let swarm = SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_other_transport(|k| metered_tcp_transport(k, &tcp_meters))
            .map_err(|e| NetError::Build(e.to_string()))?
            .with_other_transport(|k| metered_ws_transport(k, &ws_meters, ws_tls))
            .map_err(|e| NetError::Build(e.to_string()))?
            .with_behaviour(move |k| relay_behaviour_with(k, &behaviour_limits))
            .map_err(|e| NetError::Build(e.to_string()))?
            .build();
        Ok(Self {
            swarm,
            meters,
            limits,
            clock: Arc::new(SystemClock),
            listen: Vec::new(),
            external: Vec::new(),
        })
    }

    /// Replace the clock (tests drive the shed sweep with a `ManualClock`).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// This relay's libp2p peer id; the `/p2p/<id>` an invite embeds.
    pub fn local_peer_id(&self) -> libp2p::PeerId {
        *self.swarm.local_peer_id()
    }

    /// Live byte counters, for an operator dashboard or a test.
    pub fn meters(&self) -> ByteMeters {
        self.meters.clone()
    }

    /// Start listening on `addr`.
    pub fn listen_on(&mut self, addr: Multiaddr) -> Result<(), NetError> {
        self.swarm
            .listen_on(addr.clone())
            .map_err(|e| NetError::Listen(format!("{addr}: {e}")))?;
        self.listen.push(addr);
        Ok(())
    }

    /// Declare a dialable public address for this relay. Reservations carry these to clients, so
    /// a wildcard or otherwise undialable address is refused here rather than silently handed out.
    pub fn add_external_address(&mut self, addr: Multiaddr) -> Result<(), NetError> {
        if is_wildcard_addr(&addr) {
            return Err(NetError::Listen(format!(
                "{addr} is a wildcard address and cannot be advertised: clients would have \
                 nothing to dial. Give the relay's real public address."
            )));
        }
        self.swarm.add_external_address(addr.clone());
        self.external.push(addr);
        Ok(())
    }

    /// Fail fast on the misconfiguration that produces silent timeouts (P12).
    ///
    /// A relay whose only listeners are wildcard binds and which was given no external address
    /// has nothing dialable to put in a reservation. Every client that reserves on it gets a
    /// circuit address nobody can reach, and every joiner sees an unactionable timeout. That is
    /// an operator error, so it is reported as one, at startup, naming the fix.
    ///
    /// Public so a caller can pre-flight before printing a start-up banner: a node that prints
    /// "running" and then dies reads as a crash rather than as a configuration mistake.
    pub fn check_advertisable(&self) -> Result<(), NetError> {
        if !self.external.is_empty() {
            return Ok(());
        }
        if self.listen.iter().any(|a| !is_wildcard_addr(a)) {
            return Ok(());
        }
        Err(NetError::Listen(format!(
            "this relay listens only on wildcard addresses ({}) and was given no external \
             address, so every reservation it grants would carry an address no client can dial. \
             Pass the relay's real public address (for example \
             /ip4/198.51.100.7/tcp/4000) before starting it.",
            self.listen
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }

    /// Run the relay until the process is stopped, sweeping budgets as it goes.
    ///
    /// Returns `Err` only for a startup misconfiguration; once running it does not return.
    pub async fn run(mut self) -> Result<(), NetError> {
        self.check_advertisable()?;
        let tick = Duration::from_secs(self.limits.sweep_secs);
        let mut shed = ShedState::new(self.clock.now_ms(), &self.meters);
        loop {
            // `select_next_some` is cancellation-safe (it only polls the swarm as a stream), so a
            // timeout around it is a clean periodic wake-up. Tokio's periodic-timer and sleep
            // helpers are forbidden by the ambient-dependency gate, and every *policy* decision
            // below reads the injected Clock rather than this wake-up.
            if let Ok(event) = tokio::time::timeout(tick, self.swarm.select_next_some()).await {
                self.on_event(event);
            }
            self.sweep(&mut shed);
        }
    }

    fn on_event(&mut self, event: SwarmEvent<RelayBehaviourEvent>) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                // Once the operator has stated the node's real public address, auto-advertising
                // whatever the OS bound is strictly harmful: a wildcard bind expands to the
                // node's private addresses, which are undialable from the internet and disclose
                // the operator's internal topology to every client that reserves a circuit.
                if self.external.is_empty() {
                    advertise_if_dialable(&mut self.swarm, &address);
                } else {
                    tracing::debug!(%address, "listening (advertising the operator-supplied external addresses instead)");
                }
                tracing::info!(%address, "relay listening");
            }
            SwarmEvent::Behaviour(RelayBehaviourEvent::Relay(e)) => {
                tracing::debug!(?e, "relay event");
            }
            _ => {}
        }
    }

    /// One pass of the load-shed policy. Cheap and idempotent; skipped unless a sweep is due.
    fn sweep(&mut self, shed: &mut ShedState) {
        let now = self.clock.now_ms();
        if now.saturating_sub(shed.last_sweep_ms) < self.limits.sweep_secs.saturating_mul(1_000) {
            return;
        }
        shed.last_sweep_ms = now;
        self.swarm.behaviour_mut().admission.set_now_ms(now);

        // Roll the budget window. Bases are re-read (not zeroed) because the underlying counters
        // are cumulative-since-connect; a window is a difference against them.
        let window_ms = self.limits.budget_window_secs.saturating_mul(1_000);
        if now.saturating_sub(shed.window_start_ms) >= window_ms {
            shed.roll(now, &self.meters);
            // Only between windows: dropping a counter mid-window would let a peer reset its own
            // usage by reconnecting, which is exactly what the budget exists to stop.
            let reaped = self.meters.reap_disconnected();
            tracing::debug!(reaped, "relay budget window rolled");
        }

        let node_used = self.meters.total_bytes().saturating_sub(shed.base_total);
        let saturated = node_used > self.limits.node_budget_bytes;
        if saturated != self.swarm.behaviour().admission.is_saturated() {
            if saturated {
                tracing::warn!(
                    used_bytes = node_used,
                    budget_bytes = self.limits.node_budget_bytes,
                    "relay is over its aggregate byte budget; refusing new connections until the \
                     window rolls"
                );
            } else {
                tracing::info!("relay is back under its aggregate byte budget");
            }
            self.swarm
                .behaviour_mut()
                .admission
                .set_saturated(saturated);
        }

        let cooldown_ms = self.limits.shed_cooldown_secs.saturating_mul(1_000);
        for (peer, used) in peers_over_budget(
            &self.meters.snapshot(),
            &shed.base_peer,
            self.limits.peer_budget_bytes,
        ) {
            if self.swarm.behaviour().admission.is_denied(&peer) {
                continue; // already shed this window
            }
            tracing::warn!(
                %peer,
                used_bytes = used,
                budget_bytes = self.limits.peer_budget_bytes,
                cooldown_secs = self.limits.shed_cooldown_secs,
                "peer exceeded its relay byte budget; disconnecting and refusing it"
            );
            self.swarm
                .behaviour_mut()
                .admission
                .deny_peer_for(peer, cooldown_ms);
            let _ = self.swarm.disconnect_peer_id(peer);
        }
    }
}

/// Peers whose traffic **this window** is over `budget`, with the amount they used.
///
/// Split out of the sweep so the policy is testable without a swarm: the counters are cumulative
/// since a peer was first seen, so window usage is always a difference against the window's
/// baseline, and a peer with no baseline (it appeared mid-window) is measured from zero.
fn peers_over_budget(
    snapshot: &[(libp2p::PeerId, u64)],
    base: &HashMap<libp2p::PeerId, u64>,
    budget: u64,
) -> Vec<(libp2p::PeerId, u64)> {
    snapshot
        .iter()
        .filter_map(|(peer, total)| {
            let used = total.saturating_sub(base.get(peer).copied().unwrap_or(0));
            (used > budget).then_some((*peer, used))
        })
        .collect()
}

/// The rolling-window state the shed sweep keeps between passes.
struct ShedState {
    window_start_ms: u64,
    last_sweep_ms: u64,
    /// Node total at the start of the window; usage is the difference against it.
    base_total: u64,
    /// Per-peer totals at the start of the window.
    base_peer: HashMap<libp2p::PeerId, u64>,
}

impl ShedState {
    fn new(now_ms: u64, meters: &ByteMeters) -> Self {
        Self {
            window_start_ms: now_ms,
            // Sweep immediately on the first pass so a node that starts already loaded (a restart
            // under attack) sheds without waiting out a full interval.
            last_sweep_ms: 0,
            base_total: meters.total_bytes(),
            base_peer: meters.snapshot().into_iter().collect(),
        }
    }

    fn roll(&mut self, now_ms: u64, meters: &ByteMeters) {
        self.window_start_ms = now_ms;
        self.base_total = meters.total_bytes();
        self.base_peer = meters.snapshot().into_iter().collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configured_limits_are_not_the_upstream_defaults() {
        // The whole point of this module: assert the numbers actually changed, and that they
        // changed in the direction that makes the product work. Upstream 0.21.1 defaults are
        // 128 KiB / 120 s / 16 circuits / 128 reservations.
        let defaults = relay::Config::default();
        assert_eq!(defaults.max_circuit_bytes, 1 << 17);
        assert_eq!(defaults.max_circuit_duration, Duration::from_secs(120));
        assert_eq!(defaults.max_circuits, 16);
        assert_eq!(defaults.max_reservations, 128);

        let cfg = RelayLimits::default().to_relay_config();
        // A 16 MiB blob (the response cap) must fit many times over, not be 128x the budget.
        assert!(
            cfg.max_circuit_bytes >= 16 * 1024 * 1024 * 8,
            "a circuit must carry at least eight avatar-sized blobs"
        );
        // A voice call at 32 kbit/s each way is 8 KiB/s summed; it must survive an hour.
        assert!(cfg.max_circuit_bytes >= 8 * 1024 * 3600);
        assert!(cfg.max_circuit_duration >= Duration::from_secs(3600));
        assert!(cfg.max_circuits >= 256);
        assert!(cfg.max_reservations >= 1024);
        // The upstream rate limiters must survive being overridden field by field.
        assert_eq!(cfg.reservation_rate_limiters.len(), 2);
        assert_eq!(cfg.circuit_src_rate_limiters.len(), 2);
    }

    #[test]
    fn incoherent_limits_are_refused() {
        // A connection cap below the reservation cap means reservations are refused by the wrong
        // limit, so the operator tunes the wrong number.
        let l = RelayLimits {
            max_established_incoming: 16,
            ..Default::default()
        };
        assert!(l.validate().is_err());

        let base = RelayLimits::default();
        let l = RelayLimits {
            peer_budget_bytes: base.node_budget_bytes * 2,
            ..Default::default()
        };
        assert!(l.validate().is_err());

        let l = RelayLimits {
            sweep_secs: 0,
            ..Default::default()
        };
        assert!(l.validate().is_err());

        assert!(RelayLimits::default().validate().is_ok());
    }

    #[test]
    fn the_per_peer_byte_budget_binds_on_window_usage_not_lifetime_usage() {
        let a = libp2p::PeerId::random();
        let b = libp2p::PeerId::random();
        let budget = 1_000u64;

        // `a` moved 10 MB before this window opened and only 500 bytes inside it, so it is under
        // budget: shedding on lifetime totals would punish a long-lived honest member forever.
        let mut base = HashMap::new();
        base.insert(a, 10_000_000);
        let snapshot = vec![(a, 10_000_500), (b, 1_001)];
        let over = peers_over_budget(&snapshot, &base, budget);
        assert_eq!(over.len(), 1, "only the peer over its window budget sheds");
        assert_eq!(over[0].0, b);
        assert_eq!(over[0].1, 1_001);

        // Exactly at the budget is not over it.
        assert!(peers_over_budget(&[(b, 1_000)], &HashMap::new(), budget).is_empty());
        // And a peer that first appeared mid-window is measured from zero.
        assert_eq!(
            peers_over_budget(&[(b, 1_001)], &HashMap::new(), budget).len(),
            1
        );
    }

    #[test]
    fn a_wildcard_only_relay_refuses_to_start() {
        // P12: the misconfiguration that produces a silent timeout for every user is an error,
        // not a doc comment.
        let key = libp2p::identity::Keypair::generate_ed25519();
        let mut node = RelayNode::build(key, RelayLimits::default(), None).unwrap();
        node.listen = vec!["/ip4/0.0.0.0/tcp/4000".parse().unwrap()];
        let err = node.check_advertisable().unwrap_err();
        assert!(
            err.to_string().contains("no client can dial"),
            "the error must say what is wrong: {err}"
        );

        // An explicit public address fixes it.
        node.add_external_address("/ip4/198.51.100.7/tcp/4000".parse().unwrap())
            .unwrap();
        assert!(node.check_advertisable().is_ok());

        // And a wildcard external address is refused outright rather than advertised.
        let mut node2 = RelayNode::build(
            libp2p::identity::Keypair::generate_ed25519(),
            RelayLimits::default(),
            None,
        )
        .unwrap();
        assert!(node2
            .add_external_address("/ip4/0.0.0.0/tcp/4000".parse().unwrap())
            .is_err());

        // A concrete (even loopback) listen address is dialable by somebody, so it is allowed:
        // the loopback and in-process harnesses must keep working unchanged.
        let mut node3 = RelayNode::build(
            libp2p::identity::Keypair::generate_ed25519(),
            RelayLimits::default(),
            None,
        )
        .unwrap();
        node3.listen = vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()];
        assert!(node3.check_advertisable().is_ok());
    }
}
