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
//! can measure against. [`crate::metering`] adds per-peer, per-source-prefix and node accounting
//! in the transport, and this module turns it into a load-shed path.
//!
//! ## The shed path, and the two ways the first version of it was wrong
//!
//! **Capacity and budget have to agree.** The meter charges a relayed byte twice (read from the
//! source connection, written to the destination), which is what a bill looks like. One voice
//! circuit at 32 kbit/s each way is 8 KB/s of payload and therefore **16 KB/s of budget**. The
//! first sizing shipped 512 circuits against an 8 GiB hourly node budget, which is 8 MB/s against
//! a 2.3 MB/s allowance: the node exhausted an hour's budget in about 18 minutes **at exactly the
//! load it documented itself as sized for**. The two numbers now have to agree, and
//! [`RelayLimits::validate`] refuses a configuration where they do not; see
//! [`RelayLimits::nominal_window_bytes`] for the arithmetic.
//!
//! **Saturation must be a rate, and must not be a cliff.** Saturation used to be
//! `total_bytes() - base_total > node_budget`, where `total_bytes()` is monotonic and `base_total`
//! moved only when the whole window rolled: once true it stayed true until the top of the hour.
//! Combined with `refuse_all_when_saturated`, an ordinary busy hour took the node **completely
//! dark** for the remainder of it, every hour. Saturation is now computed from a short ring of
//! per-sweep deltas with hysteresis, exactly as the rendezvous occupancy check already did, and it
//! **tightens the per-prefix quota** rather than refusing everybody.
//!
//! **The shed has to reach the load, not the newcomers.** Refusing new inbound while the peers
//! already saturating the node keep flowing is backwards. Over-budget *peers* are disconnected and
//! denied, and over-budget *source prefixes* are disconnected and denied as a prefix; the second
//! is the one that binds, because a `PeerId` is a free keypair and rotating it walked straight
//! around a per-peer budget.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use catcoms_rt::{Clock, OsCryptoRng, SystemClock};
use futures::StreamExt;
use libp2p::core::transport::MemoryTransport;
use libp2p::core::upgrade::Version;
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{
    autonat, connection_limits, identify, noise, ping, relay, yamux, Multiaddr, Swarm,
    SwarmBuilder, Transport,
};

use crate::admission::{AddrPrefix, Admission, AdmissionConfig};
use crate::fdlimit::check_open_file_limit;
use crate::identify_config;
use crate::infra_transport::{
    is_advertisable, metered_tcp_transport, metered_ws_transport, WsTlsConfig,
};
use crate::metering::ByteMeters;
use crate::NetError;

/// What one **nominal** circuit costs the byte budget, per second.
///
/// A voice call is the product's heaviest sustained relayed load: roughly 32 kbit/s in each
/// direction, i.e. 8 KB/s of payload. The meter charges a relayed byte twice, once when it is read
/// from the source connection and once when it is written to the destination, because the operator
/// really does pay for a byte of ingress and a byte of egress. So one circuit bills 16 KB/s.
///
/// This is the unit [`RelayLimits::nominal_window_bytes`] sizes capacity against. It is a
/// deliberately pessimistic yardstick: most circuits are idle reservations or short bursts, and a
/// node whose circuits are all saturated voice calls is the worst case, not the average. Sizing
/// against the average is how the shipped configuration ended up contradicting itself.
pub const NOMINAL_CIRCUIT_BYTES_PER_SEC: u64 = 16 * 1024;

/// AutoNAT v2 callbacks the public relay may have in flight at once.
///
/// Upstream makes every v2 requester upload 30--100 KiB before the much smaller callback, which
/// removes reflection amplification, but its server behaviour has no aggregate dial queue limit.
/// The connection-limits behaviour is declared after it in the derive tree and supplies that
/// missing resource ceiling. Sixty-four is ample for honest probes and small enough that a probe
/// flood cannot consume the relay's file descriptors ahead of circuit traffic.
const MAX_PENDING_AUTONAT_DIALBACKS: u32 = 64;

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
    /// bandwidth, because every circuit can be moving bytes at once, and it is therefore the
    /// number that has to agree with [`RelayLimits::node_budget_bytes`].
    ///
    /// The arithmetic, which the shipped default of 512 failed: one circuit costs
    /// [`NOMINAL_CIRCUIT_BYTES_PER_SEC`] = 16 KB/s of budget, so 512 circuits is 8 MB/s, which
    /// spends an 8 GiB hourly budget in 8 GiB / 8 MB/s = **1,024 seconds, about 17 minutes**. At
    /// 128 the node draws 128 x 16 KB/s = 2 MB/s, i.e. 2 MB/s x 3,600 s = 7.03 GiB per hour, which
    /// fits the 8 GiB budget with about 12% left over for the framing the meter cannot see (it
    /// counts decrypted substream payload only; see [`crate::metering`]).
    ///
    /// Cutting capacity rather than raising the budget is the deliberate choice: the budget is the
    /// number denominated in money, and the stated target for these defaults is a bill an
    /// individual operator absorbs without checking. Matching 512 circuits would need roughly 30
    /// GiB an hour, about 21 TB a month, which is not that machine. An operator with a bigger pipe
    /// raises **both** numbers; `validate` refuses to let them drift apart again. Upstream
    /// ships 16.
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
    /// Bytes the whole node may move per window before it starts shedding.
    /// 8 GiB/hour is roughly 2.3 MB/s sustained, about 5.8 TB of traffic a month, of which
    /// roughly half is egress. That fits inside the monthly allowance of a cheap VPS with room
    /// to spare. Raising it raises the bill linearly and nothing else, but raising it is also what
    /// an operator must do before raising [`RelayLimits::max_circuits`].
    pub node_budget_bytes: u64,
    /// Bytes one **source prefix** may move per window before the prefix is disconnected and
    /// denied. This is the byte budget that actually binds, for the same reason the rendezvous
    /// registration quota is per prefix: a `PeerId` costs an attacker one keypair, so a per-peer
    /// budget is evaded by reconnecting under a new identity and only the node aggregate was left
    /// holding anything.
    ///
    /// 2 GiB per hour is twice a single peer's budget, so an ordinary site with a couple of busy
    /// members is unaffected, and it is a quarter of the node budget, so four such sources cannot
    /// between them exhaust the node without every one of them being shed first. Costing an
    /// attacker four distinct networks per node-budget is not a large cost; it is a real one,
    /// where the previous cost was zero.
    pub prefix_budget_bytes: u64,
    /// How long a peer or prefix that blew its budget is refused for. Long enough to make a retry
    /// loop pointless, short enough that a member who genuinely transferred a lot is back the same
    /// session.
    pub shed_cooldown_secs: u64,
    /// The window the **rate** used for saturation is averaged over. Saturation is a question
    /// about right now ("is this node currently spending faster than its budget affords?"), not a
    /// question about the accumulated total, so it is measured over a short trailing window rather
    /// than against the budget window's running sum. One minute is long enough that a single burst
    /// does not trip it and short enough that the node reacts inside a call setup.
    ///
    /// Must be at least [`RelayLimits::sweep_secs`], because the samples are per-sweep deltas.
    pub rate_window_secs: u64,
    /// Percentage of the budget's sustained rate at which the node declares itself saturated.
    /// 100 means "spending exactly as fast as the budget affords is the trip point", which is the
    /// honest reading: sustaining that rate for a whole window is precisely spending the budget.
    pub saturate_at_percent: u32,
    /// Percentage of the budget's sustained rate below which saturation is released. Strictly
    /// below `saturate_at_percent`: without hysteresis the flag flaps once per sweep, and each
    /// flap changes the admission quota under live traffic.
    pub desaturate_at_percent: u32,
    /// Soft cap on peers tracked by the byte meter. The meter's per-peer map is only guaranteed to
    /// be reaped when the budget window rolls, which is up to an hour; peer-id churn would grow it
    /// for that whole hour. Sweeping it back to the live set once it passes this cap bounds the
    /// memory without handing every reconnecting peer a fresh budget every ten seconds.
    pub max_tracked_peers: usize,
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
            // 128 x 16 KB/s x 3,600 s = 7.03 GiB per hour, inside the 8 GiB node budget below.
            // See the field docs for why this is 128 and not 512.
            max_circuits: 128,
            max_circuits_per_peer: 8,
            max_circuit_duration_secs: 4 * 60 * 60,
            max_circuit_bytes: 256 * 1024 * 1024,
            max_pending_incoming: 256,
            max_established_incoming: 8_192,
            max_established_per_peer: 8,
            budget_window_secs: 60 * 60,
            peer_budget_bytes: 1024 * 1024 * 1024,
            node_budget_bytes: 8 * 1024 * 1024 * 1024,
            prefix_budget_bytes: 2 * 1024 * 1024 * 1024,
            shed_cooldown_secs: 15 * 60,
            sweep_secs: 10,
            rate_window_secs: 60,
            saturate_at_percent: 100,
            desaturate_at_percent: 75,
            max_tracked_peers: 8_192,
            // A saturated relay **tightens** rather than refusing everything. Refusing everything
            // was the old setting and it converted an ordinary busy hour into a total outage,
            // while leaving the peers already causing the load connected: the shed hit only
            // newcomers. The load itself is shed by denying the over-budget peers and prefixes
            // below, which is where the traffic actually is.
            admission: AdmissionConfig {
                refuse_all_when_saturated: false,
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

    /// The connection caps these limits describe, including AutoNAT's outbound callbacks.
    pub fn to_connection_limits(&self) -> connection_limits::ConnectionLimits {
        connection_limits::ConnectionLimits::default()
            .with_max_pending_incoming(Some(self.max_pending_incoming))
            .with_max_established_incoming(Some(self.max_established_incoming))
            .with_max_established_per_peer(Some(self.max_established_per_peer))
            .with_max_pending_outgoing(Some(MAX_PENDING_AUTONAT_DIALBACKS))
            .with_max_established(Some(
                self.max_established_incoming
                    .saturating_add(MAX_PENDING_AUTONAT_DIALBACKS),
            ))
    }

    /// Bytes a **fully loaded** relay moves in one budget window, at the nominal per-circuit rate.
    ///
    /// `max_circuits x NOMINAL_CIRCUIT_BYTES_PER_SEC x budget_window_secs`. At the defaults:
    /// 128 x 16,384 x 3,600 = 7,549,747,200 bytes = 7.03 GiB, against a node budget of 8 GiB.
    ///
    /// This is the number the shipped configuration got wrong, so it is a method rather than a
    /// comment: 512 x 16,384 x 3,600 = 30,198,988,800 bytes = 28.1 GiB against the same 8 GiB
    /// budget, i.e. the node was advertised as able to carry three and a half times what it was
    /// allowed to spend, and shed itself into an outage at its own nominal load.
    pub fn nominal_window_bytes(&self) -> u64 {
        (self.max_circuits as u64)
            .saturating_mul(NOMINAL_CIRCUIT_BYTES_PER_SEC)
            .saturating_mul(self.budget_window_secs)
    }

    /// The sustained byte rate the node budget affords, in bytes per second.
    pub fn budget_bytes_per_sec(&self) -> u64 {
        self.node_budget_bytes / self.budget_window_secs.max(1)
    }

    /// Reject a configuration that cannot work, before a node is started with it.
    fn validate(&self) -> Result<(), NetError> {
        if self.budget_window_secs == 0 || self.sweep_secs == 0 {
            return Err(NetError::Build(
                "relay budget window and sweep interval must be non-zero".into(),
            ));
        }
        if self.rate_window_secs < self.sweep_secs {
            return Err(NetError::Build(format!(
                "the saturation rate window ({}s) is shorter than the sweep interval ({}s), so                  the rate would be computed from fewer than one sample",
                self.rate_window_secs, self.sweep_secs
            )));
        }
        if self.desaturate_at_percent >= self.saturate_at_percent {
            return Err(NetError::Build(
                "the relay desaturate threshold must be strictly below the saturate threshold,                  or the shed flag flaps once per sweep"
                    .into(),
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
        if self.prefix_budget_bytes < self.peer_budget_bytes
            || self.prefix_budget_bytes > self.node_budget_bytes
        {
            return Err(NetError::Build(format!(
                "the per-prefix byte budget ({}) must sit between one peer's ({}) and the whole                  node's ({}): below the peer budget it would shed an honest single user before                  their own budget fired, and above the node budget it could never bind",
                self.prefix_budget_bytes, self.peer_budget_bytes, self.node_budget_bytes
            )));
        }
        // The contradiction that made the first shipped sizing shed itself into an outage at its
        // own documented nominal load. Capacity and budget are two statements about the same
        // machine, so a configuration where the first exceeds the second is refused rather than
        // logged: the operator has to move whichever number they actually meant.
        let nominal = self.nominal_window_bytes();
        if nominal > self.node_budget_bytes {
            return Err(NetError::Build(format!(
                "max_circuits ({}) at the nominal {} bytes/s per circuit would move {} bytes in a                  {}s window, which is more than node_budget_bytes ({}). The node would shed                  itself at its own rated capacity. Either lower max_circuits to about {} or raise                  node_budget_bytes to about {}.",
                self.max_circuits,
                NOMINAL_CIRCUIT_BYTES_PER_SEC,
                nominal,
                self.budget_window_secs,
                self.node_budget_bytes,
                self.node_budget_bytes
                    / NOMINAL_CIRCUIT_BYTES_PER_SEC.saturating_mul(self.budget_window_secs).max(1),
                nominal,
            )));
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
    /// AutoNAT v2 dial-back service. A relay is already the mutually reachable public third party
    /// a NAT'd member depends on, so it is the right place to test that member's direct address.
    /// V2 requires more requester bytes than callback bytes and a fresh callback socket; the
    /// connection limits below additionally cap concurrent outbound probes.
    pub autonat_server: autonat::v2::server::Behaviour<OsCryptoRng>,
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
        autonat_server: autonat::v2::server::Behaviour::new(OsCryptoRng),
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

/// Advertise a bound listen address as external, unless it could not be dialled from outside.
///
/// P12, stated as the property actually needed. `add_external_address` on every `NewListenAddr` is
/// what puts an address into the reservation handed to clients. A `0.0.0.0` bind rarely surfaces
/// *as* `0.0.0.0`, because `libp2p-tcp` expands a wildcard into one `NewListenAddr` per interface;
/// what actually lands in reservations is the node's **private** addresses (`192.168.x.x`,
/// `10.x.x.x`, loopback). Those are undialable from the internet, so the client's circuit listener
/// closes and the user sees a timeout, and they are an unforced disclosure of the operator's
/// internal topology on top. Either way the operator is told nothing.
///
/// The filter used to test for a *wildcard*, which failed open on the most common cloud shape of
/// all: on AWS, GCP, Azure and Hetzner the interface address is RFC1918 behind 1:1 NAT, so
/// `--host 10.0.0.5` with no external address passed the test and auto-advertised `10.0.0.5` into
/// every reservation. [`is_advertisable`] tests for plausible global routability instead.
fn advertise_if_dialable(swarm: &mut Swarm<RelayBehaviour>, address: &Multiaddr) {
    if !is_advertisable(address) {
        tracing::warn!(
            %address,
            "not advertising this listen address: it is not plausibly reachable from the \
             internet, so reservations carrying it would be undialable (and it would disclose the \
             operator's internal topology). Supply the relay's real public address with an \
             explicit external address."
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
        // The meter keys its per-prefix counter on the same masks the admission layer denies on.
        let v4_bits = limits.admission.ipv4_prefix_bits;
        let v6_bits = limits.admission.ipv6_prefix_bits;
        let swarm = SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_other_transport(|k| metered_tcp_transport(k, &tcp_meters, v4_bits, v6_bits))
            .map_err(|e| NetError::Build(e.to_string()))?
            .with_other_transport(|k| metered_ws_transport(k, &ws_meters, ws_tls, v4_bits, v6_bits))
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
        if !is_advertisable(&addr) {
            return Err(NetError::Listen(format!(
                "{addr} is not plausibly reachable from the internet (wildcard, private, \
                 carrier-grade-NAT, link-local or reserved space) and cannot be advertised: \
                 clients would have nothing to dial, and the address would disclose the \
                 operator's internal topology. Give the relay's real public address."
            )));
        }
        self.swarm.add_external_address(addr.clone());
        self.external.push(addr);
        Ok(())
    }

    /// Fail fast on the misconfiguration that produces silent timeouts (P12).
    ///
    /// A relay with no plausibly-routable listener and no external address has nothing dialable
    /// to put in a reservation. Every client that reserves on it gets a circuit address nobody can
    /// reach, and every joiner sees an unactionable timeout. That is an operator error, so it is
    /// reported as one, at startup, naming the fix.
    ///
    /// Public so a caller can pre-flight before printing a start-up banner: a node that prints
    /// "running" and then dies reads as a crash rather than as a configuration mistake.
    pub fn check_advertisable(&self) -> Result<(), NetError> {
        if !self.external.is_empty() {
            return Ok(());
        }
        if self.listen.iter().any(is_advertisable) {
            return Ok(());
        }
        Err(NetError::Listen(format!(
            "none of this relay's listen addresses ({}) is plausibly reachable from the internet \
             and it was given no external address, so every reservation it grants would carry an \
             address no client can dial. This is the ordinary cloud shape: an RFC1918 interface \
             address behind 1:1 NAT looks fine locally and is undialable from outside. Pass the \
             relay's real public address (for example /ip4/45.79.12.34/tcp/4000) before \
             starting it.",
            self.listen
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }

    /// Refuse to start when the process cannot open enough files for its configured connection
    /// cap. See [`crate::fdlimit`]: `EMFILE` in the accept loop is not the clean `ConnectionDenied`
    /// the whole admission design is built on, it is silence.
    ///
    /// Public for the same reason as [`RelayNode::check_advertisable`]: a caller pre-flights
    /// before printing a start-up banner.
    pub fn check_fd_limit(&self) -> Result<(), NetError> {
        check_open_file_limit("relay", self.limits.max_established_incoming)
    }

    /// Run the relay until the process is stopped, sweeping budgets as it goes.
    ///
    /// Returns `Err` only for a startup misconfiguration; once running it does not return.
    pub async fn run(mut self) -> Result<(), NetError> {
        self.check_advertisable()?;
        self.check_fd_limit()?;
        let tick = Duration::from_secs(self.limits.sweep_secs);
        let mut shed = ShedState::new(self.clock.now_ms(), &self.meters, &self.limits);
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
                } else if is_advertisable(&address) {
                    // Suppressing auto-advertisement once the operator has spoken is right (a
                    // wildcard bind expands into private addresses, which are undialable and
                    // disclose the internal topology), but doing it silently is not: it also drops
                    // perfectly good addresses the operator did not think to repeat, and the
                    // WebSocket listener on another port is exactly the one they forget. Say so.
                    tracing::warn!(
                        %address,
                        "this listen address looks dialable but is NOT being advertised, because \
                         explicit external addresses were supplied and those take over entirely. \
                         If clients should reach the relay here (a WebSocket listener on another \
                         port is the usual case), add it to the external addresses too."
                    );
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
        } else {
            // Between windows the map is still bounded, just more coarsely: a soft cap keeps
            // peer-id churn from pinning an entry per identity for up to a whole hour. Only
            // counters with no live connection behind them go, so nobody's budget is reset while
            // they are still using it.
            let churn = self.meters.reap_if_over(self.limits.max_tracked_peers);
            if churn > 0 {
                tracing::debug!(churn, "reaped disconnected peer meters over the soft cap");
            }
        }

        self.update_saturation(shed);
        self.shed_over_budget(shed);
    }

    /// Recompute the saturation flag from a **rolling rate** with hysteresis.
    ///
    /// The first version of this asked whether `total_bytes() - base_total` exceeded the node
    /// budget. `total_bytes()` is monotonic and `base_total` only moved when the whole window
    /// rolled, so once that comparison went true it could not go false again until the top of the
    /// hour: an ordinary busy hour latched the node into its shed state for the remainder of it.
    /// A rate answers the question that was actually being asked, and hysteresis stops the answer
    /// flapping once per sweep, which would otherwise change the admission quota under live
    /// traffic every ten seconds.
    fn update_saturation(&mut self, shed: &mut ShedState) {
        shed.rate.push(self.meters.total_bytes());
        let rate = shed.rate.bytes_per_sec();
        let budget_rate = self.limits.budget_bytes_per_sec();
        let on = u128::from(budget_rate) * u128::from(self.limits.saturate_at_percent) / 100;
        let off = u128::from(budget_rate) * u128::from(self.limits.desaturate_at_percent) / 100;
        let was = self.swarm.behaviour().admission.is_saturated();
        let now_saturated = if was {
            u128::from(rate) >= off
        } else {
            u128::from(rate) > on
        };
        if now_saturated == was {
            return;
        }
        if now_saturated {
            tracing::warn!(
                rate_bytes_per_sec = rate,
                budget_bytes_per_sec = budget_rate,
                "relay is spending faster than its byte budget affords; tightening the per-source \
                 connection quota"
            );
        } else {
            tracing::info!(
                rate_bytes_per_sec = rate,
                budget_bytes_per_sec = budget_rate,
                "relay is back inside its byte budget rate; back-pressure released"
            );
        }
        self.swarm
            .behaviour_mut()
            .admission
            .set_saturated(now_saturated);
    }

    /// Disconnect and deny the peers and **source prefixes** actually over budget.
    ///
    /// The prefix half is the one that binds. A per-peer budget denies a `PeerId`, which is a free
    /// keypair, so an attacker rotating identities never accumulated against any per-peer figure
    /// and only the node aggregate was left holding anything: that is what made a handful of
    /// self-owned circuits enough to take the node's whole hourly budget. Denying the prefix costs
    /// the attacker addresses, and disconnecting the peers behind it reaches the traffic that is
    /// already flowing rather than only the next caller.
    fn shed_over_budget(&mut self, shed: &ShedState) {
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

        // While saturated a whole source network is held to what one peer gets. Under pressure the
        // node has to take capacity back from whoever is using the most of it, and a source that
        // is spreading its traffic over many free identities is precisely who that is.
        let prefix_budget = if self.swarm.behaviour().admission.is_saturated() {
            self.limits
                .prefix_budget_bytes
                .min(self.limits.peer_budget_bytes)
        } else {
            self.limits.prefix_budget_bytes
        };
        for (prefix, used) in prefixes_over_budget(
            &self.meters.prefix_snapshot(),
            &shed.base_prefix,
            prefix_budget,
        ) {
            if self.swarm.behaviour().admission.is_prefix_denied(&prefix) {
                continue;
            }
            let peers = self.swarm.behaviour().peers_in_prefix(&prefix);
            tracing::warn!(
                %prefix,
                used_bytes = used,
                budget_bytes = prefix_budget,
                attached_peers = peers.len(),
                cooldown_secs = self.limits.shed_cooldown_secs,
                "source prefix exceeded its relay byte budget; disconnecting it and refusing the \
                 whole prefix (a PeerId is free, an address is not)"
            );
            self.swarm
                .behaviour_mut()
                .admission
                .deny_prefix_for(prefix, cooldown_ms);
            for peer in peers {
                self.swarm
                    .behaviour_mut()
                    .admission
                    .deny_peer_for(peer, cooldown_ms);
                let _ = self.swarm.disconnect_peer_id(peer);
            }
        }
    }
}

impl RelayBehaviour {
    /// The attached peers known to have come from `prefix`. A thin forwarder so the sweep can read
    /// the admission behaviour and then take a mutable borrow of the swarm without overlapping.
    fn peers_in_prefix(&self, prefix: &AddrPrefix) -> Vec<libp2p::PeerId> {
        self.admission.peers_in_prefix(prefix)
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

/// **Source prefixes** whose traffic this window is over `budget`, with the amount they used.
///
/// The prefix twin of [`peers_over_budget`], and the one that costs an attacker something: a peer
/// budget is denominated in keypairs, which are free.
fn prefixes_over_budget(
    snapshot: &[(AddrPrefix, u64)],
    base: &HashMap<AddrPrefix, u64>,
    budget: u64,
) -> Vec<(AddrPrefix, u64)> {
    snapshot
        .iter()
        .filter_map(|(prefix, total)| {
            let used = total.saturating_sub(base.get(prefix).copied().unwrap_or(0));
            (used > budget).then_some((*prefix, used))
        })
        .collect()
}

/// A short ring of per-sweep byte deltas, giving the node's **current** throughput.
///
/// Saturation is a question about right now, so it needs a rate, and a rate needs deltas. The ring
/// holds one sample per sweep covering `rate_window_secs`, so the reading is an average over the
/// last minute at the default sizing rather than a single sweep's spike or a whole window's
/// monotonic sum. Two samples of history is enough to be useful, so the ring reports a rate as
/// soon as it has one delta rather than staying silent until it is full.
#[derive(Debug)]
struct RateRing {
    samples: VecDeque<u64>,
    capacity: usize,
    sweep_secs: u64,
    last_total: u64,
}

impl RateRing {
    fn new(rate_window_secs: u64, sweep_secs: u64, total: u64) -> Self {
        let sweep_secs = sweep_secs.max(1);
        let capacity = (rate_window_secs / sweep_secs).max(1) as usize;
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
            sweep_secs,
            last_total: total,
        }
    }

    /// Record the node's cumulative total at this sweep, keeping the delta.
    fn push(&mut self, total: u64) {
        let delta = total.saturating_sub(self.last_total);
        self.last_total = total;
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(delta);
    }

    /// Bytes per second over the samples held. Zero before the first sample.
    fn bytes_per_sec(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let sum: u128 = self.samples.iter().map(|d| u128::from(*d)).sum();
        let secs = (self.samples.len() as u128) * u128::from(self.sweep_secs);
        u64::try_from(sum / secs.max(1)).unwrap_or(u64::MAX)
    }
}

/// The rolling-window state the shed sweep keeps between passes.
struct ShedState {
    window_start_ms: u64,
    last_sweep_ms: u64,
    /// Per-peer totals at the start of the window.
    base_peer: HashMap<libp2p::PeerId, u64>,
    /// Per-source-prefix totals at the start of the window.
    base_prefix: HashMap<AddrPrefix, u64>,
    /// Trailing throughput, which is what saturation is decided on.
    rate: RateRing,
}

impl ShedState {
    fn new(now_ms: u64, meters: &ByteMeters, limits: &RelayLimits) -> Self {
        Self {
            window_start_ms: now_ms,
            // Sweep immediately on the first pass so a node that starts already loaded (a restart
            // under attack) sheds without waiting out a full interval.
            last_sweep_ms: 0,
            base_peer: meters.snapshot().into_iter().collect(),
            base_prefix: meters.prefix_snapshot().into_iter().collect(),
            rate: RateRing::new(
                limits.rate_window_secs,
                limits.sweep_secs,
                meters.total_bytes(),
            ),
        }
    }

    fn roll(&mut self, now_ms: u64, meters: &ByteMeters) {
        self.window_start_ms = now_ms;
        self.base_peer = meters.snapshot().into_iter().collect();
        self.base_prefix = meters.prefix_snapshot().into_iter().collect();
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
        // Capacity is no longer asserted as a bare number: it is asserted against the budget, which
        // is the relationship the first sizing got wrong. A fully loaded node at the nominal voice
        // rate must fit inside the bytes it is allowed to spend in one window.
        let l = RelayLimits::default();
        assert!(
            l.nominal_window_bytes() <= l.node_budget_bytes,
            "{} circuits at {} B/s for {}s is {} bytes against a {}-byte budget",
            l.max_circuits,
            NOMINAL_CIRCUIT_BYTES_PER_SEC,
            l.budget_window_secs,
            l.nominal_window_bytes(),
            l.node_budget_bytes
        );
        // ... and it must still be a real relay, not a token one: many times upstream's 16, and
        // enough concurrent calls that a mid-sized community is served.
        assert!(cfg.max_circuits >= 128);
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
    fn capacity_that_outruns_the_budget_is_refused() {
        // CRITICAL 1, made unrepresentable. The shipped sizing was 512 circuits against an 8 GiB
        // hourly budget: 512 x 16 KB/s x 3,600 s = 28.1 GiB, so the node shed itself into a total
        // outage roughly 17 minutes into every hour, at exactly the load it documented itself as
        // sized for. A configuration that says both of those things is now rejected.
        let base = RelayLimits::default();
        let l = RelayLimits {
            max_circuits: 512,
            ..Default::default()
        };
        assert_eq!(l.nominal_window_bytes(), 512 * 16 * 1024 * 3600);
        let err = l.validate().unwrap_err().to_string();
        assert!(err.contains("rated capacity"), "{err}");
        assert!(err.contains("max_circuits"), "{err}");
        // The message has to name the two levers and the size of each, or the operator guesses.
        assert!(err.contains("lower max_circuits to about 145"), "{err}");

        // Raising the budget to match is the operator's other lever, and it is accepted.
        assert!(RelayLimits {
            max_circuits: 512,
            node_budget_bytes: 32 * 1024 * 1024 * 1024,
            prefix_budget_bytes: 8 * 1024 * 1024 * 1024,
            ..Default::default()
        }
        .validate()
        .is_ok());

        // The defaults leave headroom for the framing the meter cannot see, but not a silly amount.
        let head = base.node_budget_bytes - base.nominal_window_bytes();
        assert!(
            head * 100 / base.node_budget_bytes >= 10,
            "only {head} bytes of headroom over the nominal full load"
        );

        // A per-prefix budget outside [peer, node] is incoherent in one direction or the other.
        assert!(RelayLimits {
            prefix_budget_bytes: base.peer_budget_bytes / 2,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(RelayLimits {
            prefix_budget_bytes: base.node_budget_bytes * 2,
            ..Default::default()
        }
        .validate()
        .is_err());

        // And a saturation band with no hysteresis flaps once per sweep.
        assert!(RelayLimits {
            desaturate_at_percent: base.saturate_at_percent,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(RelayLimits {
            rate_window_secs: 1,
            sweep_secs: 10,
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn a_saturated_relay_tightens_rather_than_going_dark() {
        // The other half of CRITICAL 1: `refuse_all_when_saturated` turned overload into a total
        // outage, and because it refused only *new* inbound it left the peers actually causing the
        // load running. The shipped relay must degrade breadth-first instead.
        let l = RelayLimits::default();
        assert!(
            !l.admission.refuse_all_when_saturated,
            "a saturated relay must not refuse every new connection"
        );
        assert!(
            l.admission.max_conns_per_prefix_saturated < l.admission.max_conns_per_prefix,
            "saturation has to actually tighten something"
        );
    }

    #[test]
    fn saturation_follows_the_rate_and_clears_without_waiting_for_the_window() {
        // The old test for this could not exist, because the old computation could not clear: it
        // was `monotonic total - a base that only moved at the top of the hour`. A rate can fall.
        let limits = RelayLimits::default();
        let budget_rate = limits.budget_bytes_per_sec();
        let mut ring = RateRing::new(limits.rate_window_secs, limits.sweep_secs, 0);
        assert_eq!(ring.bytes_per_sec(), 0, "no samples yet is not saturation");

        // Six sweeps at twice the affordable rate.
        let mut total = 0u64;
        for _ in 0..6 {
            total += budget_rate * 2 * limits.sweep_secs;
            ring.push(total);
        }
        assert!(
            ring.bytes_per_sec() > budget_rate,
            "{} vs {budget_rate}",
            ring.bytes_per_sec()
        );

        // Traffic stops. The ring is a *trailing* window, so the reading decays to zero within
        // one rate window and the node comes back on its own, mid-budget-window.
        for _ in 0..6 {
            ring.push(total);
        }
        assert_eq!(
            ring.bytes_per_sec(),
            0,
            "the reading must be able to fall again"
        );
    }

    #[test]
    fn the_per_prefix_byte_budget_binds_on_window_usage() {
        // CRITICAL 2, relay half: the byte budget must deny the *address*, so rotating keypairs
        // stops being free. Same window semantics as the per-peer budget.
        let a = crate::admission::addr_prefix(
            &"/ip4/198.51.100.1/tcp/1".parse::<Multiaddr>().unwrap(),
            24,
            56,
        )
        .unwrap();
        let b = crate::admission::addr_prefix(
            &"/ip4/203.0.113.1/tcp/1".parse::<Multiaddr>().unwrap(),
            24,
            56,
        )
        .unwrap();
        let mut base = HashMap::new();
        base.insert(a, 10_000_000);
        let snapshot = vec![(a, 10_000_500), (b, 1_001)];
        let over = prefixes_over_budget(&snapshot, &base, 1_000);
        assert_eq!(
            over.len(),
            1,
            "only the prefix over its window budget sheds"
        );
        assert_eq!(over[0].0, b);
        assert!(prefixes_over_budget(&[(b, 1_000)], &HashMap::new(), 1_000).is_empty());
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
    fn an_undialable_relay_refuses_to_start() {
        // P12: the misconfiguration that produces a silent timeout for every user is an error,
        // not a doc comment. The check now tests routability rather than wildcard-ness, because
        // the common cloud shape (an RFC1918 interface address behind 1:1 NAT) is not a wildcard
        // and used to sail straight through.
        let key = libp2p::identity::Keypair::generate_ed25519();
        let mut node = RelayNode::build(key, RelayLimits::default(), None).unwrap();
        node.listen = vec!["/ip4/0.0.0.0/tcp/4000".parse().unwrap()];
        let err = node.check_advertisable().unwrap_err();
        assert!(
            err.to_string().contains("no client can dial"),
            "the error must say what is wrong: {err}"
        );

        // `catcomsctl relay --host 10.0.0.5` with no external address: the AWS/GCP/Azure/Hetzner
        // default, which the wildcard-only check passed and then advertised.
        let mut nat = RelayNode::build(
            libp2p::identity::Keypair::generate_ed25519(),
            RelayLimits::default(),
            None,
        )
        .unwrap();
        nat.listen = vec!["/ip4/10.0.0.5/tcp/4000".parse().unwrap()];
        assert!(
            nat.check_advertisable().is_err(),
            "a private interface address behind 1:1 NAT must not pass as advertisable"
        );

        // An explicit public address fixes it.
        node.add_external_address("/ip4/45.79.12.34/tcp/4000".parse().unwrap())
            .unwrap();
        assert!(node.check_advertisable().is_ok());

        // A wildcard, private or reserved external address is refused outright rather than
        // advertised. (198.51.100.0/24 is TEST-NET-2 and is not routed anywhere, which is exactly
        // why the examples in this file no longer use it.)
        let mut node2 = RelayNode::build(
            libp2p::identity::Keypair::generate_ed25519(),
            RelayLimits::default(),
            None,
        )
        .unwrap();
        for bad in [
            "/ip4/0.0.0.0/tcp/4000",
            "/ip4/10.0.0.5/tcp/4000",
            "/ip4/100.64.3.9/tcp/4000",
            "/ip6/fd00::1/tcp/4000",
        ] {
            assert!(
                node2.add_external_address(bad.parse().unwrap()).is_err(),
                "{bad} must be refused"
            );
        }

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
