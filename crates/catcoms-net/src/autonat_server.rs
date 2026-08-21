//! Policy wrapper for the libp2p AutoNAT v2 server.
//!
//! Upstream's v2 server accepts a requester-supplied callback address and immediately turns it
//! into a `ToSwarm::Dial`.  It intentionally exposes no request-policy hook.  Enabling it on a
//! public relay/rendezvous without another gate therefore lets one authenticated Noise peer make
//! the node perform sustained outbound connection attempts.
//!
//! The two behaviours in this module close that gap without forking libp2p:
//!
//! * [`GuardedAutoNatServer`] delegates the protocol itself, remembers the source IP observed on
//!   the request connection, and tags the callback `ConnectionId` when upstream emits its dial.
//! * [`AutoNatDialGuard`] is declared *before* the protocol behaviours.  libp2p calls its pending
//!   outbound hook before opening the socket, where the complete address list is finally visible.
//!   Only tagged AutoNAT dials are inspected; ordinary relay/rendezvous dials pass untouched.
//!
//! A callback must target the requester's own globally-routable IP literal over direct TCP or
//! QUIC.  Per-peer, per-source-prefix and whole-node request windows bound work on one established
//! connection as well as identity rotation from one network.  This is deliberately stricter than
//! the wire protocol: it may reject a legitimate request to test a second address family. Exact-IP
//! matching still lets a requester probe other ports on its own public NAT/CGNAT address, so the
//! rate and concurrency caps bound that residual rather than claiming to eliminate it.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::fmt;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::addr_is_globally_routable;
use crate::admission::{addr_prefix, AddrPrefix};
use catcoms_rt::{Clock, OsCryptoRng, RngCore, SystemClock};
use either::Either;
use libp2p::core::transport::PortUse;
use libp2p::core::Endpoint;
use libp2p::multiaddr::Protocol;
use libp2p::swarm::{
    dummy, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler, THandlerInEvent,
    THandlerOutEvent, ToSwarm,
};
use libp2p::{autonat, Multiaddr, PeerId};

const WINDOW_MS: u64 = 60_000;
const MAX_PEER_REQUESTS: u32 = 8;
const MAX_PREFIX_REQUESTS: u32 = 32;
const MAX_NODE_REQUESTS: u32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RejectReason {
    UntaggedSource,
    InvalidTarget,
    TargetDiffersFromSource,
    PeerRate,
    PrefixRate,
    NodeRate,
}

impl fmt::Display for RejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::UntaggedSource => "AutoNAT requester source is unavailable",
            Self::InvalidTarget => "AutoNAT callback target is not a public direct TCP/QUIC IP",
            Self::TargetDiffersFromSource => {
                "AutoNAT callback target differs from the requester's observed source IP"
            }
            Self::PeerRate => "AutoNAT callback rate exceeded for this peer",
            Self::PrefixRate => "AutoNAT callback rate exceeded for this source prefix",
            Self::NodeRate => "AutoNAT callback rate exceeded for this node",
        };
        f.write_str(text)
    }
}

impl std::error::Error for RejectReason {}

#[derive(Debug, Clone, Copy)]
struct SourceObservation {
    ip: IpAddr,
    prefix: AddrPrefix,
}

#[derive(Debug, Clone, Copy)]
struct CallbackRequest {
    requester: Option<PeerId>,
    source: Option<SourceObservation>,
}

#[derive(Debug, Clone, Copy)]
struct PendingSource {
    request_connection: ConnectionId,
    requester: PeerId,
    source: SourceObservation,
}

#[derive(Debug, Clone, Copy, Default)]
struct WindowCounter {
    started_ms: u64,
    count: u32,
}

impl WindowCounter {
    fn charge(&mut self, now_ms: u64, limit: u32) -> bool {
        if now_ms.saturating_sub(self.started_ms) >= WINDOW_MS {
            self.started_ms = now_ms;
            self.count = 0;
        }
        if self.count >= limit {
            return false;
        }
        self.count = self.count.saturating_add(1);
        true
    }
}

struct PolicyState {
    clock: Arc<dyn Clock>,
    /// Callback ids emitted by the wrapped server.  The guard ignores every other outbound dial.
    callbacks: HashMap<ConnectionId, CallbackRequest>,
    /// Source address for each currently-established inbound request connection. Infrastructure
    /// connection limits provide the hard bound; `ConnectionClosed` removes it deterministically.
    connections: HashMap<ConnectionId, SourceObservation>,
    /// Set immediately before delegating a handler event. Upstream queues its callback dial from
    /// that same event, so `poll` consumes the exact connection's observation rather than a stale
    /// “last source for peer” guess.
    pending_sources: VecDeque<PendingSource>,
    peers: HashMap<PeerId, WindowCounter>,
    prefixes: HashMap<AddrPrefix, WindowCounter>,
    node: WindowCounter,
    /// The in-memory transport has no IP component.  Only deterministic test builders enable
    /// this; deployed nodes always use the strict policy above.
    allow_memory_for_tests: bool,
}

impl fmt::Debug for PolicyState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PolicyState")
            .field("callbacks", &self.callbacks.len())
            .field("connections", &self.connections.len())
            .field("peer_windows", &self.peers.len())
            .field("prefix_windows", &self.prefixes.len())
            .field("allow_memory_for_tests", &self.allow_memory_for_tests)
            .finish()
    }
}

impl PolicyState {
    fn new(clock: Arc<dyn Clock>, allow_memory_for_tests: bool) -> Self {
        Self {
            clock,
            callbacks: HashMap::new(),
            connections: HashMap::new(),
            pending_sources: VecDeque::new(),
            peers: HashMap::new(),
            prefixes: HashMap::new(),
            node: WindowCounter::default(),
            allow_memory_for_tests,
        }
    }

    fn observe_connection(&mut self, connection_id: ConnectionId, remote: &Multiaddr) {
        // A relayed request's first IP belongs to the relay, not the requester. Treating it as
        // source authority would let the requester probe arbitrary ports on that relay host.
        if remote
            .iter()
            .any(|part| matches!(part, Protocol::P2pCircuit))
        {
            return;
        }
        let Some(ip) = host_ip(remote) else {
            return;
        };
        let Some(prefix) = addr_prefix(remote, 24, 56) else {
            return;
        };
        self.connections
            .insert(connection_id, SourceObservation { ip, prefix });
    }

    fn note_dial_back_command(&mut self, connection_id: ConnectionId, peer: PeerId) {
        if let Some(source) = self.connections.get(&connection_id).copied() {
            self.pending_sources.push_back(PendingSource {
                request_connection: connection_id,
                requester: peer,
                source,
            });
        }
    }

    fn close_connection(&mut self, connection_id: ConnectionId) {
        self.connections.remove(&connection_id);
        self.pending_sources
            .retain(|pending| pending.request_connection != connection_id);
    }

    fn tag_callback(&mut self, connection_id: ConnectionId, peer: Option<PeerId>) {
        let source = peer.and_then(|peer| {
            let index = self
                .pending_sources
                .iter()
                .position(|pending| pending.requester == peer)?;
            self.pending_sources
                .remove(index)
                .map(|pending| pending.source)
        });
        self.callbacks.insert(
            connection_id,
            CallbackRequest {
                requester: peer,
                source,
            },
        );
    }

    fn authorize(
        &mut self,
        connection_id: ConnectionId,
        peer: Option<PeerId>,
        addresses: &[Multiaddr],
    ) -> Result<(), RejectReason> {
        let Some(callback) = self.callbacks.remove(&connection_id) else {
            return Ok(()); // not an AutoNAT callback
        };
        let requester = callback.requester.ok_or(RejectReason::InvalidTarget)?;
        let now_ms = self.clock.now_ms();
        // Expired windows are removed before accepting a new key. The global bucket admits at
        // most 256 attempts per live window, so the two maps are inherently bounded below 256;
        // no attacker-driven arbitrary eviction can reset somebody else's limiter.
        self.peers
            .retain(|_, counter| now_ms.saturating_sub(counter.started_ms) < WINDOW_MS);
        self.prefixes
            .retain(|_, counter| now_ms.saturating_sub(counter.started_ms) < WINDOW_MS);

        // Charge every tagged request once its source is known, including malformed/mismatched
        // targets. Those do not open a socket, but parsing and protocol work are still work.
        if !self.node.charge(now_ms, MAX_NODE_REQUESTS) {
            return Err(RejectReason::NodeRate);
        }
        if !self
            .peers
            .entry(requester)
            .or_default()
            .charge(now_ms, MAX_PEER_REQUESTS)
        {
            return Err(RejectReason::PeerRate);
        }
        if peer != Some(requester) || addresses.len() != 1 {
            return Err(RejectReason::InvalidTarget);
        }
        if self.allow_memory_for_tests && is_memory_address(&addresses[0]) {
            return Ok(());
        }
        let source = callback.source.ok_or(RejectReason::UntaggedSource)?;
        if !self
            .prefixes
            .entry(source.prefix)
            .or_default()
            .charge(now_ms, MAX_PREFIX_REQUESTS)
        {
            return Err(RejectReason::PrefixRate);
        }

        let target =
            direct_target_ip(&addresses[0], requester).ok_or(RejectReason::InvalidTarget)?;
        if target != source.ip {
            return Err(RejectReason::TargetDiffersFromSource);
        }
        Ok(())
    }
}

fn host_ip(address: &Multiaddr) -> Option<IpAddr> {
    address.iter().find_map(|part| match part {
        Protocol::Ip4(ip) => Some(IpAddr::V4(ip)),
        Protocol::Ip6(ip) => Some(IpAddr::V6(ip)),
        _ => None,
    })
}

fn is_memory_address(address: &Multiaddr) -> bool {
    let mut parts = address.iter();
    matches!(parts.next(), Some(Protocol::Memory(_)))
        && parts.all(|part| matches!(part, Protocol::P2p(_)))
}

/// Return the one IP literal only when the whole route is a direct TCP or QUIC endpoint.
fn direct_target_ip(address: &Multiaddr, requester: PeerId) -> Option<IpAddr> {
    if !addr_is_globally_routable(address) {
        return None;
    }
    let mut ip = None;
    let mut tcp = false;
    let mut udp = false;
    let mut quic = false;
    let parts: Vec<_> = address.iter().collect();
    for (index, part) in parts.iter().enumerate() {
        match part {
            Protocol::Ip4(v) => {
                if ip.replace(IpAddr::V4(*v)).is_some() {
                    return None;
                }
            }
            Protocol::Ip6(v) => {
                if ip.replace(IpAddr::V6(*v)).is_some() {
                    return None;
                }
            }
            Protocol::Tcp(port) if *port != 0 && !tcp && !udp => tcp = true,
            Protocol::Udp(port) if *port != 0 && !udp && !tcp => udp = true,
            Protocol::QuicV1 if udp && !quic => quic = true,
            Protocol::P2p(peer) if *peer == requester && index + 1 == parts.len() => {}
            _ => return None,
        }
    }
    if tcp || (udp && quic) {
        ip
    } else {
        None
    }
}

/// First half of the policy pair: delegates the protocol and tags its callback dials.
pub struct GuardedAutoNatServer<R = OsCryptoRng>
where
    R: Clone + Send + RngCore + 'static,
{
    inner: autonat::v2::server::Behaviour<R>,
    state: Arc<Mutex<PolicyState>>,
}

impl<R> fmt::Debug for GuardedAutoNatServer<R>
where
    R: Clone + Send + RngCore + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuardedAutoNatServer")
            .finish_non_exhaustive()
    }
}

/// Second half of the pair.  This must be declared before public protocol behaviours.
#[derive(Clone)]
pub struct AutoNatDialGuard {
    state: Arc<Mutex<PolicyState>>,
}

impl fmt::Debug for AutoNatDialGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AutoNatDialGuard").finish_non_exhaustive()
    }
}

pub(crate) fn guarded_autonat_server(
    allow_memory_for_tests: bool,
) -> (AutoNatDialGuard, GuardedAutoNatServer<OsCryptoRng>) {
    guarded_autonat_server_with_clock(Arc::new(SystemClock), allow_memory_for_tests)
}

fn guarded_autonat_server_with_clock(
    clock: Arc<dyn Clock>,
    allow_memory_for_tests: bool,
) -> (AutoNatDialGuard, GuardedAutoNatServer<OsCryptoRng>) {
    let state = Arc::new(Mutex::new(PolicyState::new(clock, allow_memory_for_tests)));
    (
        AutoNatDialGuard {
            state: state.clone(),
        },
        GuardedAutoNatServer {
            inner: autonat::v2::server::Behaviour::new(OsCryptoRng),
            state,
        },
    )
}

impl AutoNatDialGuard {
    pub(crate) fn set_clock(&mut self, clock: Arc<dyn Clock>) {
        self.state
            .lock()
            .expect("AutoNAT policy lock poisoned")
            .clock = clock;
    }
}

impl NetworkBehaviour for AutoNatDialGuard {
    type ConnectionHandler = dummy::ConnectionHandler;
    type ToSwarm = Infallible;

    fn handle_pending_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        maybe_peer: Option<PeerId>,
        addresses: &[Multiaddr],
        _effective_role: Endpoint,
    ) -> Result<Vec<Multiaddr>, ConnectionDenied> {
        self.state
            .lock()
            .expect("AutoNAT policy lock poisoned")
            .authorize(connection_id, maybe_peer, addresses)
            .map_err(ConnectionDenied::new)?;
        Ok(Vec::new())
    }

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
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
        Ok(dummy::ConnectionHandler)
    }

    fn on_swarm_event(&mut self, _event: FromSwarm<'_>) {}

    fn on_connection_handler_event(
        &mut self,
        _peer_id: PeerId,
        _connection_id: ConnectionId,
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

impl<R> NetworkBehaviour for GuardedAutoNatServer<R>
where
    R: Clone + Send + RngCore + 'static,
{
    type ConnectionHandler =
        <autonat::v2::server::Behaviour<R> as NetworkBehaviour>::ConnectionHandler;
    type ToSwarm = <autonat::v2::server::Behaviour<R> as NetworkBehaviour>::ToSwarm;

    fn handle_pending_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        self.inner
            .handle_pending_inbound_connection(connection_id, local_addr, remote_addr)
    }

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.state
            .lock()
            .expect("AutoNAT policy lock poisoned")
            .observe_connection(connection_id, remote_addr);
        self.inner.handle_established_inbound_connection(
            connection_id,
            peer,
            local_addr,
            remote_addr,
        )
    }

    fn handle_pending_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        maybe_peer: Option<PeerId>,
        addresses: &[Multiaddr],
        effective_role: Endpoint,
    ) -> Result<Vec<Multiaddr>, ConnectionDenied> {
        self.inner.handle_pending_outbound_connection(
            connection_id,
            maybe_peer,
            addresses,
            effective_role,
        )
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        role_override: Endpoint,
        port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.inner.handle_established_outbound_connection(
            connection_id,
            peer,
            addr,
            role_override,
            port_use,
        )
    }

    fn on_swarm_event(&mut self, event: FromSwarm<'_>) {
        if let FromSwarm::ConnectionClosed(closed) = event {
            self.state
                .lock()
                .expect("AutoNAT policy lock poisoned")
                .close_connection(closed.connection_id);
        }
        self.inner.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        // Only the request handler's DialBackCommand creates an outbound callback. Status and
        // dial-back completion events must not leave a source tag behind for a later request.
        if matches!(&event, Either::Right(Either::Left(_))) {
            self.state
                .lock()
                .expect("AutoNAT policy lock poisoned")
                .note_dial_back_command(connection_id, peer_id);
        }
        self.inner
            .on_connection_handler_event(peer_id, connection_id, event);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        let event = match self.inner.poll(cx) {
            Poll::Ready(event) => event,
            Poll::Pending => return Poll::Pending,
        };
        if let ToSwarm::Dial { opts } = &event {
            // Every dial emitted by the wrapped behaviour is AutoNAT-owned. Tag even a future
            // upstream peer-less variant so it fails closed instead of looking like an ordinary
            // infrastructure dial to the guard.
            self.state
                .lock()
                .expect("AutoNAT policy lock poisoned")
                .tag_callback(opts.connection_id(), opts.get_peer_id());
        }
        Poll::Ready(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::ManualClock;

    fn address(text: &str) -> Multiaddr {
        text.parse().expect("test address")
    }

    fn state(clock: &ManualClock) -> PolicyState {
        PolicyState::new(Arc::new(clock.clone()), false)
    }

    fn test_peer(n: u8) -> PeerId {
        let mut seed = [13; 32];
        seed[0] = n;
        crate::keypair_from_seed(seed)
            .unwrap()
            .public()
            .to_peer_id()
    }

    fn tag_request(
        policy: &mut PolicyState,
        callback_id: ConnectionId,
        request_connection: ConnectionId,
        peer: PeerId,
        source: &Multiaddr,
    ) {
        if !policy.connections.contains_key(&request_connection) {
            policy.observe_connection(request_connection, source);
        }
        policy.note_dial_back_command(request_connection, peer);
        policy.tag_callback(callback_id, Some(peer));
    }

    #[test]
    fn callback_target_must_be_public_direct_and_match_observed_source() {
        let clock = ManualClock::new(1_000);
        let mut policy = state(&clock);
        let peer = test_peer(1);
        let source = address("/ip4/45.79.12.34/tcp/50000");
        let request_connection = ConnectionId::new_unchecked(90);

        let good_id = ConnectionId::new_unchecked(1);
        tag_request(&mut policy, good_id, request_connection, peer, &source);
        assert_eq!(
            policy.authorize(
                good_id,
                Some(peer),
                &[address("/ip4/45.79.12.34/udp/22487/quic-v1")],
            ),
            Ok(())
        );

        let other_id = ConnectionId::new_unchecked(2);
        tag_request(&mut policy, other_id, request_connection, peer, &source);
        assert_eq!(
            policy.authorize(other_id, Some(peer), &[address("/ip4/8.8.8.8/tcp/443")],),
            Err(RejectReason::TargetDiffersFromSource)
        );

        let private_id = ConnectionId::new_unchecked(3);
        tag_request(&mut policy, private_id, request_connection, peer, &source);
        assert_eq!(
            policy.authorize(
                private_id,
                Some(peer),
                &[address("/ip4/192.168.1.1/tcp/22")],
            ),
            Err(RejectReason::InvalidTarget)
        );
    }

    #[test]
    fn ordinary_outbound_dials_are_not_subject_to_autonat_policy() {
        let clock = ManualClock::new(1_000);
        let mut policy = state(&clock);
        let id = ConnectionId::new_unchecked(10);
        assert_eq!(
            policy.authorize(
                id,
                Some(test_peer(2)),
                &[address("/ip4/192.168.1.1/tcp/22")],
            ),
            Ok(())
        );
    }

    #[test]
    fn per_peer_limit_resets_on_the_injected_clock_window() {
        let clock = ManualClock::new(10_000);
        let mut policy = state(&clock);
        let peer = test_peer(3);
        let source = address("/ip4/45.79.12.34/tcp/50000");
        let target = address("/ip4/45.79.12.34/tcp/22487");
        let request_connection = ConnectionId::new_unchecked(91);

        for n in 0..MAX_PEER_REQUESTS {
            let id = ConnectionId::new_unchecked(100 + usize::try_from(n).unwrap());
            tag_request(&mut policy, id, request_connection, peer, &source);
            assert_eq!(
                policy.authorize(id, Some(peer), std::slice::from_ref(&target)),
                Ok(())
            );
        }
        let denied = ConnectionId::new_unchecked(200);
        tag_request(&mut policy, denied, request_connection, peer, &source);
        assert_eq!(
            policy.authorize(denied, Some(peer), std::slice::from_ref(&target)),
            Err(RejectReason::PeerRate)
        );

        clock.advance_ms(WINDOW_MS);
        let reset = ConnectionId::new_unchecked(201);
        tag_request(&mut policy, reset, request_connection, peer, &source);
        assert_eq!(policy.authorize(reset, Some(peer), &[target]), Ok(()));
    }

    #[test]
    fn source_prefix_limit_survives_peer_identity_rotation() {
        let clock = ManualClock::new(10_000);
        let mut policy = state(&clock);
        let source = address("/ip4/45.79.12.34/tcp/50000");
        let target = address("/ip4/45.79.12.34/tcp/22487");
        for n in 0..MAX_PREFIX_REQUESTS {
            let peer = test_peer(u8::try_from(n / MAX_PEER_REQUESTS).unwrap() + 20);
            let inbound = ConnectionId::new_unchecked(1_000 + usize::try_from(n).unwrap());
            let callback = ConnectionId::new_unchecked(2_000 + usize::try_from(n).unwrap());
            tag_request(&mut policy, callback, inbound, peer, &source);
            assert_eq!(
                policy.authorize(callback, Some(peer), std::slice::from_ref(&target)),
                Ok(())
            );
        }
        let peer = test_peer(30);
        let callback = ConnectionId::new_unchecked(3_000);
        tag_request(
            &mut policy,
            callback,
            ConnectionId::new_unchecked(3_001),
            peer,
            &source,
        );
        assert_eq!(
            policy.authorize(callback, Some(peer), &[target]),
            Err(RejectReason::PrefixRate)
        );
    }

    #[test]
    fn node_limit_survives_peer_and_prefix_rotation() {
        let clock = ManualClock::new(10_000);
        let mut policy = state(&clock);
        for n in 0..MAX_NODE_REQUESTS {
            let peer = test_peer(u8::try_from(n / MAX_PEER_REQUESTS).unwrap() + 40);
            let subnet = n / MAX_PREFIX_REQUESTS;
            let endpoint = format!("/ip4/45.79.{subnet}.34/tcp/22487");
            let source = address(&endpoint);
            let inbound = ConnectionId::new_unchecked(4_000 + usize::try_from(n).unwrap());
            let callback = ConnectionId::new_unchecked(5_000 + usize::try_from(n).unwrap());
            tag_request(&mut policy, callback, inbound, peer, &source);
            assert_eq!(policy.authorize(callback, Some(peer), &[source]), Ok(()));
        }
        let peer = test_peer(90);
        let source = address("/ip4/45.79.9.34/tcp/22487");
        let callback = ConnectionId::new_unchecked(6_000);
        tag_request(
            &mut policy,
            callback,
            ConnectionId::new_unchecked(6_001),
            peer,
            &source,
        );
        assert_eq!(
            policy.authorize(callback, Some(peer), &[source]),
            Err(RejectReason::NodeRate)
        );
    }

    #[test]
    fn a_request_on_a_long_lived_connection_refreshes_its_source_observation() {
        let clock = ManualClock::new(1_000);
        let mut policy = state(&clock);
        let peer = test_peer(4);
        let source = address("/ip4/45.79.12.34/tcp/50000");
        let request_connection = ConnectionId::new_unchecked(92);
        policy.observe_connection(request_connection, &source);
        clock.advance_ms(24 * 60 * 60 * 1_000);
        let id = ConnectionId::new_unchecked(300);
        policy.note_dial_back_command(request_connection, peer);
        policy.tag_callback(id, Some(peer));
        assert_eq!(
            policy.authorize(id, Some(peer), &[address("/ip4/45.79.12.34/tcp/22487")]),
            Ok(())
        );
        policy.close_connection(request_connection);
        assert!(!policy.connections.contains_key(&request_connection));
    }

    #[test]
    fn a_peerless_wrapped_dial_is_tagged_for_fail_closed_rejection() {
        let clock = ManualClock::new(1_000);
        let mut policy = state(&clock);
        let id = ConnectionId::new_unchecked(400);
        policy.tag_callback(id, None);
        assert_eq!(
            policy.authorize(id, None, &[address("/ip4/45.79.12.34/tcp/9")]),
            Err(RejectReason::InvalidTarget)
        );
    }

    #[test]
    fn pending_sources_are_fifo_per_peer_and_close_cleans_only_that_connection() {
        let clock = ManualClock::new(1_000);
        let mut policy = state(&clock);
        let peer = test_peer(5);
        let first = ConnectionId::new_unchecked(501);
        let second = ConnectionId::new_unchecked(502);
        policy.observe_connection(first, &address("/ip4/45.79.12.34/tcp/50001"));
        policy.observe_connection(second, &address("/ip4/8.8.8.8/tcp/50002"));
        policy.note_dial_back_command(first, peer);
        policy.note_dial_back_command(second, peer);

        policy.close_connection(first);
        assert_eq!(policy.pending_sources.len(), 1);
        assert_eq!(policy.pending_sources[0].request_connection, second);

        let callback = ConnectionId::new_unchecked(503);
        policy.tag_callback(callback, Some(peer));
        assert_eq!(policy.pending_sources.len(), 0);
        assert_eq!(
            policy.authorize(
                callback,
                Some(peer),
                &[address("/ip4/8.8.8.8/udp/22487/quic-v1")],
            ),
            Ok(())
        );
    }

    #[test]
    fn callback_without_a_dial_back_command_has_no_source_tag() {
        let clock = ManualClock::new(1_000);
        let mut policy = state(&clock);
        let peer = test_peer(6);
        let inbound = ConnectionId::new_unchecked(601);
        policy.observe_connection(inbound, &address("/ip4/45.79.12.34/tcp/50001"));
        // A status handler event intentionally does not call `note_dial_back_command`.
        let callback = ConnectionId::new_unchecked(602);
        policy.tag_callback(callback, Some(peer));
        assert!(policy.pending_sources.is_empty());
        assert_eq!(
            policy.authorize(
                callback,
                Some(peer),
                &[address("/ip4/45.79.12.34/tcp/22487")],
            ),
            Err(RejectReason::UntaggedSource)
        );
    }

    #[test]
    fn relayed_request_connection_never_authorizes_a_callback_to_the_relay_ip() {
        let clock = ManualClock::new(1_000);
        let mut policy = state(&clock);
        let peer = test_peer(7);
        let inbound = ConnectionId::new_unchecked(701);
        policy.observe_connection(inbound, &address("/ip4/45.79.12.34/tcp/4001/p2p-circuit"));
        policy.note_dial_back_command(inbound, peer);
        let callback = ConnectionId::new_unchecked(702);
        policy.tag_callback(callback, Some(peer));
        assert_eq!(
            policy.authorize(
                callback,
                Some(peer),
                &[address("/ip4/45.79.12.34/tcp/22487")],
            ),
            Err(RejectReason::UntaggedSource)
        );
    }
}
