//! Byte accounting for the infra nodes, and the counters the load-shed path reads.
//!
//! A circuit relay is the only part of this system that spends somebody else's money: it forwards
//! other people's bytes, and the bill lands on the operator's uplink. `libp2p-relay` bounds a
//! *single* circuit (`max_circuit_bytes`, `max_circuit_duration`) and a *count* of circuits, but
//! it has no aggregate accounting at all: nothing in the crate can answer "how many bytes has
//! this node moved this hour", which is precisely the number a deployment is sized against.
//!
//! So the accounting is done a layer below the behaviour, in the transport. Every connection's
//! muxer is wrapped so each substream read and write charges three counters: the remote **peer**,
//! the remote **source prefix**, and the **node**. Relayed circuit payload is charged twice, once
//! when it is read from the source connection and once when it is written to the destination,
//! which is correct for a bill: one relayed payload byte really does cost one byte of ingress and
//! one byte of egress.
//!
//! ## What this does NOT count, and why that matters
//!
//! The meter wraps the **muxer**, so it sees decrypted, demultiplexed substream payload only.
//! Invisible to it: the Noise handshake, multistream-select negotiation, yamux headers and window
//! updates, and TCP/TLS framing. `total_bytes()` is therefore a **lower bound** on the wire bill,
//! not an upper one. For bulk relayed traffic the gap is small (yamux adds a 12-byte header per
//! frame, and TCP/IP framing adds a few percent); for a **handshake flood** the gap is total,
//! because a connection that completes Noise and then sends nothing moves zero metered bytes.
//! Handshake cost is therefore bounded separately, by the per-prefix connection-rate limit in
//! [`crate::admission`], not by these counters. Sizing that treats this number as the whole bill
//! must add a margin; [`crate::relay_node::RelayLimits`] documents the one used.
//!
//! Moving the meter below the security upgrade would count everything, but the `PeerId` is not
//! known until after that upgrade, and a per-peer budget is one of the levers the shed path
//! needs. The split chosen here keeps per-peer attribution and bounds the unattributable part
//! with a rate limit instead.
//!
//! The counters are plain atomics on the hot path. All policy (windowing, load shed, disconnects)
//! is done by the owning event loop on its tick, using an injected [`catcoms_rt::Clock`].

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures::{AsyncRead, AsyncWrite};
use libp2p::core::muxing::{StreamMuxer, StreamMuxerEvent};
use libp2p::PeerId;

use crate::admission::AddrPrefix;

/// Shared byte counters, cloneable and cheap. Handed to the transport (which increments) and to
/// the event loop (which reads and sheds).
#[derive(Clone, Debug, Default)]
pub struct ByteMeters {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    total: Arc<AtomicU64>,
    /// One counter per peer, handed out by `Arc` so the hot path never takes the map lock.
    peers: Mutex<HashMap<PeerId, Arc<AtomicU64>>>,
    /// One counter per **source prefix**. This is the durable one: a `PeerId` is a free keypair
    /// and an attacker rotates it between connections, so a budget keyed on peer alone is evaded
    /// by reconnecting. An address is not free, so the prefix counter survives identity rotation
    /// for as long as the attacker keeps using the same network.
    prefixes: Mutex<HashMap<AddrPrefix, Arc<AtomicU64>>>,
}

impl ByteMeters {
    /// Fresh, empty counters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cumulative substream bytes moved by the whole node since start (both directions). See the
    /// module docs: this excludes handshake and framing overhead, so it is a lower bound.
    pub fn total_bytes(&self) -> u64 {
        self.inner.total.load(Ordering::Relaxed)
    }

    /// Cumulative bytes moved on behalf of one peer since it was first seen (or since the last
    /// reap).
    pub fn peer_bytes(&self, peer: &PeerId) -> u64 {
        self.inner
            .peers
            .lock()
            .expect("byte meter mutex poisoned")
            .get(peer)
            .map_or(0, |c| c.load(Ordering::Relaxed))
    }

    /// Cumulative bytes moved on behalf of one source prefix.
    pub fn prefix_bytes(&self, prefix: &AddrPrefix) -> u64 {
        self.inner
            .prefixes
            .lock()
            .expect("byte meter mutex poisoned")
            .get(prefix)
            .map_or(0, |c| c.load(Ordering::Relaxed))
    }

    /// Every tracked peer and its cumulative byte count. The event loop diffs this against the
    /// previous sweep to get a rate.
    pub fn snapshot(&self) -> Vec<(PeerId, u64)> {
        self.inner
            .peers
            .lock()
            .expect("byte meter mutex poisoned")
            .iter()
            .map(|(p, c)| (*p, c.load(Ordering::Relaxed)))
            .collect()
    }

    /// Every tracked source prefix and its cumulative byte count.
    pub fn prefix_snapshot(&self) -> Vec<(AddrPrefix, u64)> {
        self.inner
            .prefixes
            .lock()
            .expect("byte meter mutex poisoned")
            .iter()
            .map(|(p, c)| (*p, c.load(Ordering::Relaxed)))
            .collect()
    }

    /// How many peers currently have a counter. Also the memory the accounting itself costs.
    pub fn tracked_peers(&self) -> usize {
        self.inner
            .peers
            .lock()
            .expect("byte meter mutex poisoned")
            .len()
    }

    /// How many source prefixes currently have a counter.
    pub fn tracked_prefixes(&self) -> usize {
        self.inner
            .prefixes
            .lock()
            .expect("byte meter mutex poisoned")
            .len()
    }

    /// Drop `peer`'s counter. Only safe between budget windows: forgetting a peer mid-window
    /// resets its usage to zero, which is the reconnect-to-reset evasion the budget exists to
    /// stop. The prefix counter is unaffected, which is why that one is the durable defence.
    pub fn forget(&self, peer: &PeerId) {
        self.inner
            .peers
            .lock()
            .expect("byte meter mutex poisoned")
            .remove(peer);
    }

    /// Drop every counter with no live connection behind it, and report how many peers went.
    ///
    /// A counter's `Arc` is held by the map and by each live metered muxer, so a strong count of
    /// one means nothing is using it any more. Called at a window boundary, where a reset costs
    /// nothing, and opportunistically when the map has grown past a cap (peer-id churn would
    /// otherwise pin memory for a whole window; the prefix counters still bind in between).
    pub fn reap_disconnected(&self) -> usize {
        let mut peers = self.inner.peers.lock().expect("byte meter mutex poisoned");
        let before = peers.len();
        peers.retain(|_, c| Arc::strong_count(c) > 1);
        let gone = before - peers.len();
        drop(peers);
        let mut prefixes = self
            .inner
            .prefixes
            .lock()
            .expect("byte meter mutex poisoned");
        prefixes.retain(|_, c| Arc::strong_count(c) > 1);
        gone
    }

    /// Reap only if the peer map has grown past `cap`. Bounds the memory peer-id churn can pin
    /// without handing every reconnecting peer a fresh budget on every sweep.
    pub fn reap_if_over(&self, cap: usize) -> usize {
        if self.tracked_peers() > cap {
            self.reap_disconnected()
        } else {
            0
        }
    }

    fn counter_for(&self, peer: PeerId) -> Arc<AtomicU64> {
        self.inner
            .peers
            .lock()
            .expect("byte meter mutex poisoned")
            .entry(peer)
            .or_default()
            .clone()
    }

    fn prefix_counter_for(&self, prefix: AddrPrefix) -> Arc<AtomicU64> {
        self.inner
            .prefixes
            .lock()
            .expect("byte meter mutex poisoned")
            .entry(prefix)
            .or_default()
            .clone()
    }

    /// Wrap an upgraded connection's muxer so every byte on it is charged to `peer`, to `prefix`
    /// (when the remote address had one) and to the node.
    pub fn meter<M>(&self, peer: PeerId, prefix: Option<AddrPrefix>, muxer: M) -> MeteredMuxer<M> {
        MeteredMuxer {
            inner: muxer,
            peer: self.counter_for(peer),
            prefix: prefix.map(|p| self.prefix_counter_for(p)),
            total: Arc::clone(&self.inner.total),
        }
    }
}

/// A [`StreamMuxer`] whose substreams count their traffic. `Unpin` bounds keep the projection
/// safe without `unsafe`; every muxer this is applied to (a boxed muxer, or yamux) is `Unpin`.
#[derive(Debug)]
pub struct MeteredMuxer<M> {
    inner: M,
    peer: Arc<AtomicU64>,
    prefix: Option<Arc<AtomicU64>>,
    total: Arc<AtomicU64>,
}

impl<M> MeteredMuxer<M> {
    fn wrap<S>(&self, s: S) -> MeteredStream<S> {
        MeteredStream {
            inner: s,
            peer: Arc::clone(&self.peer),
            prefix: self.prefix.clone(),
            total: Arc::clone(&self.total),
        }
    }
}

impl<M> StreamMuxer for MeteredMuxer<M>
where
    M: StreamMuxer + Unpin,
    M::Substream: Unpin,
{
    type Substream = MeteredStream<M::Substream>;
    type Error = M::Error;

    fn poll_inbound(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_inbound(cx) {
            Poll::Ready(Ok(s)) => Poll::Ready(Ok(this.wrap(s))),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_outbound(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_outbound(cx) {
            Poll::Ready(Ok(s)) => Poll::Ready(Ok(this.wrap(s))),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_close(cx)
    }

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<StreamMuxerEvent, Self::Error>> {
        Pin::new(&mut self.get_mut().inner).poll(cx)
    }
}

/// A counted substream. Reads and writes both charge: a relay circuit is symmetric and the
/// operator pays for both halves, exactly as `libp2p-relay` itself sums both directions into
/// `max_circuit_bytes`.
#[derive(Debug)]
pub struct MeteredStream<S> {
    inner: S,
    peer: Arc<AtomicU64>,
    prefix: Option<Arc<AtomicU64>>,
    total: Arc<AtomicU64>,
}

impl<S> MeteredStream<S> {
    fn charge(&self, n: usize) {
        let n = n as u64;
        self.peer.fetch_add(n, Ordering::Relaxed);
        if let Some(p) = &self.prefix {
            p.fetch_add(n, Ordering::Relaxed);
        }
        self.total.fetch_add(n, Ordering::Relaxed);
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for MeteredStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let out = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(n)) = &out {
            this.charge(*n);
        }
        out
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for MeteredStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let out = Pin::new(&mut this.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &out {
            this.charge(*n);
        }
        out
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_close(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::Multiaddr;

    fn prefix_of(addr: &str) -> AddrPrefix {
        crate::admission::addr_prefix(&addr.parse::<Multiaddr>().unwrap(), 24, 48).unwrap()
    }

    #[test]
    fn counters_are_per_peer_per_prefix_and_aggregate() {
        let m = ByteMeters::new();
        let a = PeerId::random();
        let b = PeerId::random();
        let p = prefix_of("/ip4/198.51.100.1/tcp/1");
        let ma = m.meter(a, Some(p), ());
        let mb = m.meter(b, Some(p), ());
        ma.wrap(()).charge(1_000);
        ma.wrap(()).charge(500);
        mb.wrap(()).charge(7);

        assert_eq!(m.peer_bytes(&a), 1_500);
        assert_eq!(m.peer_bytes(&b), 7);
        assert_eq!(m.total_bytes(), 1_507, "the node total must sum every peer");
        // The prefix counter is the one that survives identity rotation: both peers came from the
        // same /24, so it holds their sum. This is what makes the budget cost an attacker
        // addresses rather than keypairs.
        assert_eq!(m.prefix_bytes(&p), 1_507);
        assert_eq!(m.tracked_peers(), 2);
        assert_eq!(m.tracked_prefixes(), 1);
    }

    #[test]
    fn reaping_only_drops_counters_with_no_live_stream() {
        let m = ByteMeters::new();
        let live = PeerId::random();
        let gone = PeerId::random();
        let p = prefix_of("/ip4/198.51.100.1/tcp/1");
        let held = m.meter(live, Some(p), ());
        let _ = m.meter(gone, Some(p), ()); // handed out and immediately dropped
        held.wrap(()).charge(10);

        assert_eq!(m.tracked_peers(), 2);
        assert_eq!(m.reap_disconnected(), 1);
        assert_eq!(m.tracked_peers(), 1);
        assert_eq!(m.peer_bytes(&live), 10);
        assert_eq!(m.peer_bytes(&gone), 0);
        // The prefix counter is still live because `held` still holds it.
        assert_eq!(m.prefix_bytes(&p), 10);
        drop(held);
        assert_eq!(m.reap_disconnected(), 1);
        assert_eq!(m.tracked_prefixes(), 0);
    }

    #[test]
    fn the_soft_cap_reap_only_fires_when_the_map_is_large() {
        let m = ByteMeters::new();
        for _ in 0..4 {
            let _ = m.meter(PeerId::random(), None, ());
        }
        assert_eq!(m.reap_if_over(10), 0, "under the cap nothing is reaped");
        assert_eq!(m.tracked_peers(), 4);
        assert_eq!(m.reap_if_over(2), 4, "over the cap the dead entries go");
        assert_eq!(m.tracked_peers(), 0);
    }
}
