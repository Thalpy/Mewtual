//! Connection **admission control** for the internet-exposed infra nodes.
//!
//! `libp2p`'s [`connection_limits`](libp2p::connection_limits) behaviour counts connections
//! globally and per `PeerId`. Neither dimension binds an attacker: a `PeerId` is a self-minted
//! keypair, so "per peer" costs nothing to evade, and a single global ceiling means the cheapest
//! attacker simply *is* the ceiling. The only identifier on the wire that costs an attacker
//! something to vary is the **source address**, so this behaviour adds a third dimension:
//! a concurrent-connection quota per **address prefix** (a `/24` for IPv4, a `/64` for IPv6),
//! which is roughly "per site" rather than "per socket".
//!
//! It also carries the two levers the load-shed path needs and that upstream does not expose:
//!
//! - a **per-peer deny window**: a peer that blew its byte budget or dumped the registration
//!   table is refused at connection setup for a cooldown, which is a *clean, immediate*
//!   `ConnectionDenied` rather than the silent timeout an overloaded node otherwise produces;
//! - a **saturation flag**: while the node is over its aggregate budget, new inbound connections
//!   are refused outright so the peers already being served keep working. Shedding the marginal
//!   caller is strictly better than degrading everybody.
//!
//! Time is **pushed in** rather than read: the owning event loop calls [`Admission::set_now_ms`]
//! with a [`catcoms_rt::Clock`] reading each tick, so nothing here touches the OS clock (the
//! ambient-dependency gate forbids it, and it keeps deny windows deterministic under test).
//!
//! The behaviour and its event loop live in the **same task** (the loop owns the `Swarm`), so the
//! state is plain `&mut self` reached through `swarm.behaviour_mut()`; no locking is involved.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::task::{Context, Poll};

use libp2p::core::transport::PortUse;
use libp2p::core::Endpoint;
use libp2p::multiaddr::Protocol;
use libp2p::swarm::behaviour::{ConnectionClosed, ConnectionEstablished, ListenFailure};
use libp2p::swarm::{
    dummy, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler, THandlerInEvent,
    THandlerOutEvent, ToSwarm,
};
use libp2p::{Multiaddr, PeerId};

/// The masked source-address prefix a connection came from; the unit an infra node meters
/// strangers in. IPv4 is masked to a `/24` and IPv6 to a `/64` by default: those are the
/// smallest allocations a single party is reliably given, so they are the smallest units that
/// cost an attacker money to multiply. Anything that is not an IP address (the in-memory test
/// transport) has no prefix and is never quota'd.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddrPrefix {
    /// A masked IPv4 network.
    V4([u8; 4]),
    /// A masked IPv6 network.
    V6([u8; 16]),
}

impl std::fmt::Display for AddrPrefix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddrPrefix::V4(o) => write!(f, "{}", Ipv4Addr::from(*o)),
            AddrPrefix::V6(o) => write!(f, "{}", Ipv6Addr::from(*o)),
        }
    }
}

/// Mask `ip` down to the configured prefix length.
fn mask(ip: IpAddr, v4_bits: u8, v6_bits: u8) -> AddrPrefix {
    match ip {
        IpAddr::V4(v4) => {
            let bits = v4_bits.min(32);
            let m: u32 = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            AddrPrefix::V4((u32::from(v4) & m).to_be_bytes())
        }
        IpAddr::V6(v6) => {
            let bits = v6_bits.min(128);
            let m: u128 = if bits == 0 {
                0
            } else {
                u128::MAX << (128 - bits)
            };
            AddrPrefix::V6((u128::from(v6) & m).to_be_bytes())
        }
    }
}

/// The masked prefix a multiaddr's first IP component belongs to, or `None` for an address with
/// no IP component (the memory transport in tests).
pub fn addr_prefix(addr: &Multiaddr, v4_bits: u8, v6_bits: u8) -> Option<AddrPrefix> {
    addr.iter().find_map(|p| match p {
        Protocol::Ip4(v4) => Some(mask(IpAddr::V4(v4), v4_bits, v6_bits)),
        Protocol::Ip6(v6) => Some(mask(IpAddr::V6(v6), v4_bits, v6_bits)),
        _ => None,
    })
}

/// Sizing for [`Admission`].
#[derive(Debug, Clone)]
pub struct AdmissionConfig {
    /// IPv4 prefix length the quota is keyed on (24 = a `/24`).
    pub ipv4_prefix_bits: u8,
    /// IPv6 prefix length the quota is keyed on (64 = a `/64`, the smallest routed allocation).
    pub ipv6_prefix_bits: u8,
    /// Concurrent inbound connections allowed from one prefix. This is the number that decides
    /// how expensive the "one laptop" attack is; it is also the number that hurts a large CGNAT
    /// pool sharing one address, so it is deliberately generous rather than tight.
    pub max_conns_per_prefix: usize,
    /// Concurrent inbound connections allowed from one prefix once the node reports itself
    /// **saturated**. Under pressure, breadth beats depth: keep serving many prefixes a little
    /// rather than a few prefixes a lot.
    pub max_conns_per_prefix_saturated: usize,
    /// Whether saturation refuses **every** new inbound connection, or only tightens the
    /// per-prefix quota.
    ///
    /// The right answer differs by node, so it is a setting rather than a policy. A **relay**
    /// over its aggregate byte budget genuinely cannot serve anybody else, and a definite refusal
    /// is what sends the client to another rung; so it refuses all. A **rendezvous** under
    /// occupancy pressure can still answer queries perfectly well: it is registrations that are
    /// scarce, not capacity, and blanket-refusing would break discovery for people who only want
    /// to read the noticeboard. So it only tightens.
    pub refuse_all_when_saturated: bool,
}

impl Default for AdmissionConfig {
    fn default() -> Self {
        Self {
            ipv4_prefix_bits: 24,
            ipv6_prefix_bits: 64,
            max_conns_per_prefix: 64,
            max_conns_per_prefix_saturated: 8,
            refuse_all_when_saturated: false,
        }
    }
}

/// Why a connection was refused. Surfaced through `ConnectionDenied` so the *caller* sees a
/// definite refusal (`ListenError::Denied` / `DialError::Denied`) instead of a hung socket:
/// the whole point of load shedding is that a full node says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeniedBecause {
    /// The source prefix already holds its quota of concurrent connections.
    PrefixQuota {
        /// The masked prefix.
        prefix: String,
        /// Connections it already holds.
        held: usize,
        /// The quota in force (which tightens while saturated).
        limit: usize,
    },
    /// This peer is inside a deny window (byte budget blown, or abusive query pattern).
    PeerDenied {
        /// Milliseconds still to run on the window.
        remaining_ms: u64,
    },
    /// The node is over its aggregate budget and is shedding new work.
    Saturated,
}

impl std::fmt::Display for DeniedBecause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeniedBecause::PrefixQuota {
                prefix,
                held,
                limit,
            } => write!(
                f,
                "source prefix {prefix} already holds {held} connections (limit {limit})"
            ),
            DeniedBecause::PeerDenied { remaining_ms } => {
                write!(f, "peer is in a deny window for another {remaining_ms}ms")
            }
            DeniedBecause::Saturated => {
                f.write_str("node is over its aggregate budget and is shedding new connections")
            }
        }
    }
}

impl std::error::Error for DeniedBecause {}

/// A snapshot of what admission control is currently holding, for operator logging.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdmissionStats {
    /// Distinct source prefixes with at least one live connection.
    pub prefixes: usize,
    /// Live inbound connections being accounted.
    pub inbound_conns: usize,
    /// Peers currently inside a deny window.
    pub denied_peers: usize,
    /// Connections refused since start.
    pub refused: u64,
}

/// The admission-control [`NetworkBehaviour`]. Compose it into an internet-exposed swarm
/// **alongside** `connection_limits`: that one bounds totals, this one bounds a single source.
#[derive(Debug)]
pub struct Admission {
    cfg: AdmissionConfig,
    /// Pushed in by the owning event loop from an injected `Clock`; never read from the OS.
    now_ms: u64,
    saturated: bool,
    /// Inbound connections per source prefix, counted from the moment the connection is
    /// **pending**. Counting only established connections would let a simultaneous burst from one
    /// source walk straight past the quota: every member of the burst would see zero established
    /// and be admitted, and the quota would bind only on the following burst.
    conns_per_prefix: HashMap<AddrPrefix, HashSet<ConnectionId>>,
    /// Reverse index so a close (or a failed handshake) is decremented from the right prefix.
    prefix_of_conn: HashMap<ConnectionId, AddrPrefix>,
    /// Last known source prefix per peer, so the run loop can charge a *registration* (which
    /// arrives as a behaviour event carrying only a `PeerId`) to the address that made it.
    prefix_of_peer: HashMap<PeerId, AddrPrefix>,
    /// Deny windows, keyed by peer, expiring at an absolute millisecond stamp.
    denied_until: HashMap<PeerId, u64>,
    refused: u64,
}

impl Admission {
    /// A new admission-control behaviour. `now_ms` seeds the clock; the owning loop keeps it
    /// current with [`Admission::set_now_ms`].
    pub fn new(cfg: AdmissionConfig, now_ms: u64) -> Self {
        Self {
            cfg,
            now_ms,
            saturated: false,
            conns_per_prefix: HashMap::new(),
            prefix_of_conn: HashMap::new(),
            prefix_of_peer: HashMap::new(),
            denied_until: HashMap::new(),
            refused: 0,
        }
    }

    /// Advance the behaviour's notion of now (milliseconds since the Unix epoch) and expire any
    /// deny windows that have elapsed. Called from the event loop with an injected `Clock`.
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        self.denied_until.retain(|_, until| *until > now_ms);
    }

    /// Refuse new connections from `peer` for `window_ms`. Extends an existing window rather
    /// than shortening it, so a peer cannot reset its own penalty by re-offending.
    pub fn deny_peer_for(&mut self, peer: PeerId, window_ms: u64) {
        let until = self.now_ms.saturating_add(window_ms);
        let slot = self.denied_until.entry(peer).or_insert(until);
        *slot = (*slot).max(until);
    }

    /// Whether `peer` is currently inside a deny window.
    pub fn is_denied(&self, peer: &PeerId) -> bool {
        self.denied_until
            .get(peer)
            .is_some_and(|until| *until > self.now_ms)
    }

    /// Declare the node over (or back under) its aggregate budget. While saturated the per-prefix
    /// quota drops to `max_conns_per_prefix_saturated` and every prefix already above the tighter
    /// quota is refused new connections.
    pub fn set_saturated(&mut self, saturated: bool) {
        self.saturated = saturated;
    }

    /// Whether the node is currently shedding.
    pub fn is_saturated(&self) -> bool {
        self.saturated
    }

    /// The source prefix a peer last connected from, if it is still known.
    pub fn prefix_of_peer(&self, peer: &PeerId) -> Option<AddrPrefix> {
        self.prefix_of_peer.get(peer).copied()
    }

    /// Live-connection count for a prefix.
    pub fn conns_in_prefix(&self, prefix: &AddrPrefix) -> usize {
        self.conns_per_prefix.get(prefix).map_or(0, HashSet::len)
    }

    /// A snapshot for operator logging.
    pub fn stats(&self) -> AdmissionStats {
        AdmissionStats {
            prefixes: self.conns_per_prefix.len(),
            inbound_conns: self.prefix_of_conn.len(),
            denied_peers: self.denied_until.len(),
            refused: self.refused,
        }
    }

    fn prefix_quota(&self) -> usize {
        if self.saturated {
            self.cfg.max_conns_per_prefix_saturated
        } else {
            self.cfg.max_conns_per_prefix
        }
    }

    fn prefix_for(&self, addr: &Multiaddr) -> Option<AddrPrefix> {
        addr_prefix(addr, self.cfg.ipv4_prefix_bits, self.cfg.ipv6_prefix_bits)
    }

    /// Release the quota slot a connection holds, whether it ever established or not.
    fn release(&mut self, connection_id: ConnectionId) {
        if let Some(prefix) = self.prefix_of_conn.remove(&connection_id) {
            if let Some(set) = self.conns_per_prefix.get_mut(&prefix) {
                set.remove(&connection_id);
                if set.is_empty() {
                    self.conns_per_prefix.remove(&prefix);
                }
            }
        }
    }

    fn refuse(&mut self, why: DeniedBecause) -> ConnectionDenied {
        self.refused = self.refused.saturating_add(1);
        tracing::debug!(reason = %why, "inbound connection refused by admission control");
        ConnectionDenied::new(why)
    }
}

impl NetworkBehaviour for Admission {
    type ConnectionHandler = dummy::ConnectionHandler;
    type ToSwarm = Infallible;

    fn handle_pending_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        _local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        // Refuse as early as possible: at the *pending* stage nothing has been spent on Noise
        // or on an identity handshake yet, so a flood costs the node close to nothing.
        let Some(prefix) = self.prefix_for(remote_addr) else {
            return Ok(()); // non-IP transport (memory, tests): nothing to meter.
        };
        let held = self.conns_in_prefix(&prefix);
        let limit = self.prefix_quota();
        if held >= limit {
            return Err(self.refuse(DeniedBecause::PrefixQuota {
                prefix: prefix.to_string(),
                held,
                limit,
            }));
        }
        // Charge it now, not on establishment: the charge is released by `ConnectionClosed` for a
        // connection that came up and by `ListenFailure` for one that did not.
        self.conns_per_prefix
            .entry(prefix)
            .or_default()
            .insert(connection_id);
        self.prefix_of_conn.insert(connection_id, prefix);
        Ok(())
    }

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // The peer id is only known once Noise has completed, so the deny window can only be
        // enforced here. That is still before any relay reservation or rendezvous request.
        if let Some(until) = self.denied_until.get(&peer).copied() {
            if until > self.now_ms {
                return Err(self.refuse(DeniedBecause::PeerDenied {
                    remaining_ms: until - self.now_ms,
                }));
            }
        }
        if self.saturated && self.cfg.refuse_all_when_saturated {
            return Err(self.refuse(DeniedBecause::Saturated));
        }
        Ok(dummy::ConnectionHandler)
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // Outbound is our own doing (an infra node dials nobody unprompted); never quota it.
        Ok(dummy::ConnectionHandler)
    }

    fn on_swarm_event(&mut self, event: FromSwarm<'_>) {
        match event {
            FromSwarm::ConnectionEstablished(ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                ..
            }) => {
                // Only inbound connections are quota'd; `Listener` is the inbound endpoint.
                if !endpoint.is_listener() {
                    return;
                }
                if let Some(prefix) = self.prefix_for(endpoint.get_remote_address()) {
                    self.conns_per_prefix
                        .entry(prefix)
                        .or_default()
                        .insert(connection_id);
                    self.prefix_of_conn.insert(connection_id, prefix);
                    self.prefix_of_peer.insert(peer_id, prefix);
                }
            }
            FromSwarm::ConnectionClosed(ConnectionClosed {
                peer_id,
                connection_id,
                remaining_established,
                ..
            }) => {
                self.release(connection_id);
                // Keep the peer -> prefix mapping only while the peer is still attached; it is
                // an unbounded map otherwise, which is itself a memory-exhaustion vector.
                if remaining_established == 0 {
                    self.prefix_of_peer.remove(&peer_id);
                }
            }
            FromSwarm::ListenFailure(ListenFailure { connection_id, .. }) => {
                // A pending inbound that never established (handshake failure, or a refusal by
                // this or another behaviour). Release the charge taken at the pending stage, or
                // a failed handshake would occupy a quota slot until the node restarts.
                self.release(connection_id);
            }
            _ => {}
        }
    }

    fn on_connection_handler_event(
        &mut self,
        _peer: PeerId,
        _connection: ConnectionId,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(addr: &str) -> Multiaddr {
        format!("/ip4/{addr}/tcp/4000").parse().unwrap()
    }

    #[test]
    fn addresses_mask_to_their_prefix() {
        // Two hosts in the same /24 must land on the same quota bucket, or the quota is
        // trivially evaded by walking the last octet.
        let a = addr_prefix(&v4("198.51.100.7"), 24, 64).unwrap();
        let b = addr_prefix(&v4("198.51.100.250"), 24, 64).unwrap();
        assert_eq!(a, b);
        let c = addr_prefix(&v4("198.51.101.7"), 24, 64).unwrap();
        assert_ne!(a, c);

        // IPv6 masks to the /64, so a host with a whole /64 to itself cannot mint fresh
        // "sources" out of the interface identifier.
        let d: Multiaddr = "/ip6/2001:db8::1/tcp/4000".parse().unwrap();
        let e: Multiaddr = "/ip6/2001:db8::ffff:ffff/tcp/4000".parse().unwrap();
        assert_eq!(
            addr_prefix(&d, 24, 64).unwrap(),
            addr_prefix(&e, 24, 64).unwrap()
        );

        // The memory transport has no IP and is never metered.
        assert!(addr_prefix(&"/memory/1234".parse().unwrap(), 24, 64).is_none());
    }

    #[test]
    fn the_prefix_quota_actually_binds() {
        let cfg = AdmissionConfig {
            max_conns_per_prefix: 2,
            ..Default::default()
        };
        let mut a = Admission::new(cfg, 0);
        let local: Multiaddr = "/ip4/0.0.0.0/tcp/4000".parse().unwrap();

        // Two connections from one /24 are admitted; the third is refused, and a different /24
        // is unaffected (the quota is per source, not global).
        for i in 0..2u64 {
            let id = ConnectionId::new_unchecked(i as usize);
            a.handle_pending_inbound_connection(id, &local, &v4("198.51.100.1"))
                .expect("under quota");
            a.on_swarm_event(FromSwarm::ConnectionEstablished(ConnectionEstablished {
                peer_id: PeerId::random(),
                connection_id: id,
                endpoint: &libp2p::core::ConnectedPoint::Listener {
                    local_addr: local.clone(),
                    send_back_addr: v4("198.51.100.1"),
                },
                failed_addresses: &[],
                other_established: 0,
            }));
        }
        let denied = a.handle_pending_inbound_connection(
            ConnectionId::new_unchecked(99),
            &local,
            &v4("198.51.100.9"),
        );
        assert!(
            denied.is_err(),
            "the third connection from a /24 must be refused"
        );
        assert!(a
            .handle_pending_inbound_connection(
                ConnectionId::new_unchecked(100),
                &local,
                &v4("203.0.113.9")
            )
            .is_ok());
    }

    #[test]
    fn a_simultaneous_burst_from_one_prefix_is_capped_and_released() {
        // The interesting case is a burst: N sockets from one source opened at once, none of them
        // established yet. Counting only established connections would admit all N.
        let cfg = AdmissionConfig {
            max_conns_per_prefix: 3,
            ..Default::default()
        };
        let mut a = Admission::new(cfg, 0);
        let local: Multiaddr = "/ip4/0.0.0.0/tcp/4000".parse().unwrap();
        for i in 0..3usize {
            a.handle_pending_inbound_connection(
                ConnectionId::new_unchecked(i),
                &local,
                &v4("198.51.100.1"),
            )
            .expect("under quota");
        }
        assert!(
            a.handle_pending_inbound_connection(
                ConnectionId::new_unchecked(3),
                &local,
                &v4("198.51.100.2")
            )
            .is_err(),
            "a pending burst must be counted, not just established connections"
        );

        // A handshake that dies must give its slot back, or a source can permanently poison its
        // own quota (and, more usefully to an attacker, somebody else's shared address).
        a.on_swarm_event(FromSwarm::ListenFailure(ListenFailure {
            local_addr: &local,
            send_back_addr: &v4("198.51.100.1"),
            error: &libp2p::swarm::ListenError::Aborted,
            connection_id: ConnectionId::new_unchecked(0),
            peer_id: None,
        }));
        assert!(a
            .handle_pending_inbound_connection(
                ConnectionId::new_unchecked(4),
                &local,
                &v4("198.51.100.2")
            )
            .is_ok());
    }

    #[test]
    fn a_deny_window_refuses_then_expires() {
        let mut a = Admission::new(AdmissionConfig::default(), 1_000);
        let peer = PeerId::random();
        let local: Multiaddr = "/ip4/0.0.0.0/tcp/4000".parse().unwrap();

        a.deny_peer_for(peer, 60_000);
        assert!(a.is_denied(&peer));
        assert!(a
            .handle_established_inbound_connection(
                ConnectionId::new_unchecked(1),
                peer,
                &local,
                &v4("198.51.100.1")
            )
            .is_err());

        // Re-offending extends rather than replaces, so the penalty cannot be reset downward.
        a.deny_peer_for(peer, 10_000);
        a.set_now_ms(50_000);
        assert!(
            a.is_denied(&peer),
            "the longer window must still be in force"
        );

        a.set_now_ms(120_000);
        assert!(!a.is_denied(&peer));
        assert!(a
            .handle_established_inbound_connection(
                ConnectionId::new_unchecked(2),
                peer,
                &local,
                &v4("198.51.100.1")
            )
            .is_ok());
    }

    #[test]
    fn saturation_sheds_new_work_and_tightens_the_prefix_quota() {
        let cfg = AdmissionConfig {
            max_conns_per_prefix: 64,
            max_conns_per_prefix_saturated: 0,
            refuse_all_when_saturated: true,
            ..Default::default()
        };
        let mut a = Admission::new(cfg, 0);
        let local: Multiaddr = "/ip4/0.0.0.0/tcp/4000".parse().unwrap();
        assert!(a
            .handle_pending_inbound_connection(
                ConnectionId::new_unchecked(1),
                &local,
                &v4("198.51.100.1")
            )
            .is_ok());

        a.set_saturated(true);
        // With `refuse_all_when_saturated` a saturated node refuses at the pending stage (tighter
        // prefix quota) and, for anything that got past it, again at the established stage. Both
        // are definite refusals rather than a silent stall.
        assert!(a
            .handle_pending_inbound_connection(
                ConnectionId::new_unchecked(2),
                &local,
                &v4("198.51.100.1")
            )
            .is_err());
        assert!(a
            .handle_established_inbound_connection(
                ConnectionId::new_unchecked(3),
                PeerId::random(),
                &local,
                &v4("203.0.113.1")
            )
            .is_err());

        // A node that only tightens (the rendezvous shape) still admits a fresh source while
        // saturated: occupancy pressure must not stop somebody merely reading the noticeboard.
        let mut b = Admission::new(
            AdmissionConfig {
                refuse_all_when_saturated: false,
                ..Default::default()
            },
            0,
        );
        b.set_saturated(true);
        assert!(b
            .handle_established_inbound_connection(
                ConnectionId::new_unchecked(4),
                PeerId::random(),
                &local,
                &v4("203.0.113.1")
            )
            .is_ok());
    }
}
