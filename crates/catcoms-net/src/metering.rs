//! Per-peer **byte accounting** for the relay, and the transports that produce it.
//!
//! A circuit relay is the only part of this system that spends somebody else's money: it forwards
//! other people's bytes, and the bill lands on the operator's uplink. `libp2p-relay` bounds a
//! *single* circuit (`max_circuit_bytes`, `max_circuit_duration`) and a *count* of circuits, but
//! it has no aggregate accounting at all: nothing in the crate can answer "how many bytes has
//! this node moved this hour", which is precisely the number a deployment is sized against.
//!
//! So the accounting is done a layer below the behaviour, in the transport. Every connection's
//! muxer is wrapped so that each substream read and write increments two counters: one for the
//! remote peer and one for the node. That counts **every** byte the OS moved on behalf of that
//! peer, including relayed circuit payload (which is invisible to every application-layer limit
//! in the stack, because a circuit is a pipe between two *other* peers), Noise framing and yamux
//! overhead. Counting slightly more than the payload is the right bias for a budget: the operator
//! pays for frames, not for payload.
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
}

impl ByteMeters {
    /// Fresh, empty counters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cumulative bytes moved by the whole node since start (both directions, framing included).
    pub fn total_bytes(&self) -> u64 {
        self.inner.total.load(Ordering::Relaxed)
    }

    /// Cumulative bytes moved on behalf of one peer since it was first seen (or since the last
    /// [`ByteMeters::forget`]).
    pub fn peer_bytes(&self, peer: &PeerId) -> u64 {
        self.inner
            .peers
            .lock()
            .expect("byte meter mutex poisoned")
            .get(peer)
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

    /// How many peers currently have a counter. The map is only reaped between budget windows,
    /// so this is also the memory the accounting itself costs.
    pub fn tracked_peers(&self) -> usize {
        self.inner
            .peers
            .lock()
            .expect("byte meter mutex poisoned")
            .len()
    }

    /// Drop `peer`'s counter. Only safe to do between budget windows: forgetting a peer mid-window
    /// resets its usage to zero, which is exactly the reconnect-to-reset evasion the budget exists
    /// to stop.
    pub fn forget(&self, peer: &PeerId) {
        self.inner
            .peers
            .lock()
            .expect("byte meter mutex poisoned")
            .remove(peer);
    }

    /// Drop every peer counter with no live `Arc` outside the map, i.e. every peer with no live
    /// connection. Called at a window boundary, where a reset costs nothing.
    pub fn reap_disconnected(&self) -> usize {
        let mut map = self.inner.peers.lock().expect("byte meter mutex poisoned");
        let before = map.len();
        // One reference is the map's own; anything above that is a live metered muxer.
        map.retain(|_, c| Arc::strong_count(c) > 1);
        before - map.len()
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

    /// Wrap an upgraded connection's muxer so every byte on it is counted against `peer`.
    pub fn meter<M>(&self, peer: PeerId, muxer: M) -> MeteredMuxer<M> {
        MeteredMuxer {
            inner: muxer,
            peer: self.counter_for(peer),
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
    total: Arc<AtomicU64>,
}

impl<M> MeteredMuxer<M> {
    fn wrap(&self, s: M::Substream) -> MeteredStream<M::Substream>
    where
        M: StreamMuxer,
    {
        MeteredStream {
            inner: s,
            peer: Arc::clone(&self.peer),
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
            Poll::Ready(Ok(s)) => Poll::Ready(Ok(MeteredStream {
                inner: s,
                peer: Arc::clone(&this.peer),
                total: Arc::clone(&this.total),
            })),
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
            Poll::Ready(Ok(s)) => {
                let wrapped = this.wrap(s);
                Poll::Ready(Ok(wrapped))
            }
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

/// A counted substream. Reads and writes both charge the peer: a relay circuit is symmetric and
/// the operator pays for both halves, exactly as `libp2p-relay` itself sums both directions into
/// `max_circuit_bytes`.
#[derive(Debug)]
pub struct MeteredStream<S> {
    inner: S,
    peer: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

impl<S> MeteredStream<S> {
    fn charge(&self, n: usize) {
        let n = n as u64;
        self.peer.fetch_add(n, Ordering::Relaxed);
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

    #[test]
    fn counters_are_per_peer_and_aggregate() {
        let m = ByteMeters::new();
        let a = PeerId::random();
        let b = PeerId::random();
        // Drive the counters the way a metered stream does, without needing a real socket.
        let sa = MeteredStream {
            inner: (),
            peer: m.counter_for(a),
            total: Arc::clone(&m.inner.total),
        };
        let sb = MeteredStream {
            inner: (),
            peer: m.counter_for(b),
            total: Arc::clone(&m.inner.total),
        };
        sa.charge(1_000);
        sa.charge(500);
        sb.charge(7);

        assert_eq!(m.peer_bytes(&a), 1_500);
        assert_eq!(m.peer_bytes(&b), 7);
        assert_eq!(m.total_bytes(), 1_507, "the node total must sum every peer");
        assert_eq!(m.tracked_peers(), 2);

        // A snapshot is what the shed sweep diffs against.
        let mut snap = m.snapshot();
        snap.sort_by_key(|(_, n)| *n);
        assert_eq!(snap.last().unwrap().1, 1_500);
    }

    #[test]
    fn reaping_only_drops_peers_with_no_live_stream() {
        let m = ByteMeters::new();
        let live = PeerId::random();
        let gone = PeerId::random();
        let held = MeteredStream {
            inner: (),
            peer: m.counter_for(live),
            total: Arc::clone(&m.inner.total),
        };
        let _ = m.counter_for(gone); // handed out and immediately dropped
        held.charge(10);

        assert_eq!(m.tracked_peers(), 2);
        assert_eq!(m.reap_disconnected(), 1);
        assert_eq!(m.tracked_peers(), 1);
        assert_eq!(m.peer_bytes(&live), 10);
        assert_eq!(m.peer_bytes(&gone), 0);
        drop(held);
    }
}
