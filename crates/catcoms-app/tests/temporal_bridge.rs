//! A retained-history bridge across disjoint sessions, at the product actor/store boundary.
//!
//! There is never an A--C transport edge, including admission. Gossip is dropped, and no
//! snapshot is passed between devices: only each device's own sealed vault survives its crash.
//! Requests and replies remain the production authenticated join and catch-up protocols.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use catcoms_app::{spawn, GroupMode, Server, ServerActor, ServerStore};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_rt::{
    Clock, Hub, ManualClock, MemNetwork, MeshTransport, PeerConnectionSnapshot, PeerId, ProtocolId,
    RequestCancellation, Topic, TransportError, TransportEvent,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use tokio::task::JoinHandle;

const A: u64 = 1;
const B: u64 = 2;
const C: u64 = 3;
const NOW: u64 = 1_700_000_000_000;
const WAIT: Duration = Duration::from_secs(30);
const PASSWORD: &[u8] = b"independent temporal bridge test vault";

type TestServer = Server<ChainTransport, ChaCha20Rng>;
type Edge = (PeerId, PeerId, u8);

/// The Hub supplies bytes and responders; this wrapper supplies real connection lifecycle
/// events, denies non-neighbour requests, and discards every gossip publication.
#[derive(Default)]
struct Chain {
    hub: Arc<Hub>,
    live: Mutex<HashMap<PeerId, mpsc::UnboundedSender<TransportEvent>>>,
    delivered: Arc<Mutex<Vec<Edge>>>,
}

impl Chain {
    fn adjacent(a: PeerId, b: PeerId) -> bool {
        a != b && (a == PeerId::from_u64(B) || b == PeerId::from_u64(B))
    }

    fn join(self: &Arc<Self>, id: u64) -> ChainTransport {
        let peer = PeerId::from_u64(id);
        let inner = self.hub.join(peer);
        let (tx, rx) = mpsc::unbounded_channel();
        let mut live = self.live.lock().unwrap();
        assert!(!live.contains_key(&peer), "one transport owner per device");
        for (other, events) in live
            .iter()
            .filter(|(other, _)| Self::adjacent(peer, **other))
        {
            events.send(TransportEvent::PeerConnected(peer)).unwrap();
            tx.send(TransportEvent::PeerConnected(*other)).unwrap();
        }
        live.insert(peer, tx);
        ChainTransport {
            inner,
            chain: Arc::clone(self),
            lifecycle: AsyncMutex::new(rx),
        }
    }
}

struct ChainTransport {
    inner: MemNetwork,
    chain: Arc<Chain>,
    lifecycle: AsyncMutex<mpsc::UnboundedReceiver<TransportEvent>>,
}

impl ChainTransport {
    fn admit(&self, peer: PeerId, data: &Bytes) -> Result<(), TransportError> {
        let local = self.local_peer();
        if !Chain::adjacent(local, peer) || !self.chain.live.lock().unwrap().contains_key(&peer) {
            return Err(TransportError::Unreachable(peer));
        }
        self.chain.delivered.lock().unwrap().push((
            local,
            peer,
            data.first().copied().unwrap_or(u8::MAX),
        ));
        Ok(())
    }
}

impl Drop for ChainTransport {
    fn drop(&mut self) {
        let peer = self.inner.local_peer();
        let mut live = self.chain.live.lock().unwrap();
        live.remove(&peer);
        for (_, events) in live
            .iter()
            .filter(|(other, _)| Chain::adjacent(peer, **other))
        {
            let _ = events.send(TransportEvent::PeerDisconnected(peer));
        }
    }
}

#[async_trait]
impl MeshTransport for ChainTransport {
    fn local_peer(&self) -> PeerId {
        self.inner.local_peer()
    }

    fn connection_snapshot(&self) -> Vec<PeerConnectionSnapshot> {
        self.chain
            .live
            .lock()
            .unwrap()
            .keys()
            .copied()
            .filter(|peer| Chain::adjacent(self.local_peer(), *peer))
            .map(|peer| PeerConnectionSnapshot {
                peer,
                active: Vec::new(),
            })
            .collect()
    }

    async fn subscribe(&self, _: Topic) -> Result<(), TransportError> {
        Ok(())
    }
    async fn unsubscribe(&self, _: Topic) -> Result<(), TransportError> {
        Ok(())
    }
    async fn publish(&self, _: Topic, _: Bytes) -> Result<(), TransportError> {
        Ok(())
    }

    async fn request(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        self.admit(peer, &data)?;
        self.inner.request(peer, proto, data).await
    }

    async fn request_cancellable(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.admit(peer, &data)?;
        self.inner
            .request_cancellable(peer, proto, data, cancellation)
            .await
    }

    async fn request_connected(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        self.admit(peer, &data)?;
        self.inner.request_connected(peer, proto, data).await
    }

    async fn request_connected_cancellable(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.admit(peer, &data)?;
        self.inner
            .request_connected_cancellable(peer, proto, data, cancellation)
            .await
    }

    async fn next_event(&self) -> Option<TransportEvent> {
        let mut lifecycle = self.lifecycle.lock().await;
        tokio::select! {
            biased;
            event = lifecycle.recv() => event,
            event = self.inner.next_event() => event,
        }
    }
}

struct Node {
    actor: ServerActor,
    owner: JoinHandle<()>,
    events: JoinHandle<()>,
}

impl Node {
    async fn start(mut server: TestServer) -> Self {
        server.subscribe_control().await.unwrap();
        let (actor, mut events, owner) = spawn(server);
        // A real host drains the bounded event channel continuously. Do not let an unread test
        // channel block the actor being tested, and do not feed received data back into it.
        let events = tokio::spawn(async move { while events.recv().await.is_some() {} });
        let node = Self {
            actor,
            owner,
            events,
        };
        node.actor.member_count().await; // startup subscription/directory barrier
        node
    }

    async fn checkpoint(&self, store: &ServerStore, rng: &mut ChaCha20Rng) {
        // The host's explicit durable checkpoint. No snapshot is returned to the test or passed
        // to a different member. A later crash cannot silently take another final snapshot.
        store
            .save_server(1, &self.actor.snapshot().await.unwrap(), rng)
            .unwrap();
    }

    async fn crash(&mut self) {
        self.owner.abort();
        let _ = (&mut self.owner).await;
        self.events.abort();
        let _ = (&mut self.events).await;
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        self.owner.abort();
        self.events.abort();
    }
}

async fn mint(actor: &ServerActor, nonce: u8) -> InviteToken {
    InviteToken::decode(
        &actor
            .mint_invite([nonce; 16], u64::MAX, Vec::new())
            .await
            .unwrap(),
    )
    .unwrap()
}

async fn fingerprint(actor: &ServerActor) -> String {
    actor
        .members()
        .await
        .into_iter()
        .find(|member| member.is_self)
        .unwrap()
        .fingerprint
}

/// Deadlines run on the injected clock, including mutually outstanding actor requests. Keep
/// the actual operation alive while time advances; repeatedly cancelling it would hide a lost
/// continuation or manufacture progress by resubmitting the command under test.
async fn bounded<T>(
    clock: &ManualClock,
    label: &str,
    work: impl std::future::Future<Output = T>,
) -> T {
    eprintln!("temporal phase: {label}");
    tokio::pin!(work);
    tokio::time::timeout(WAIT, async {
        loop {
            match tokio::time::timeout(Duration::from_millis(20), &mut work).await {
                Ok(value) => {
                    eprintln!("temporal complete: {label}");
                    return value;
                }
                Err(_) => {
                    clock.advance_ms(100);
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("temporal bridge stalled: {label}"))
}

async fn await_history(node: &Node, clock: &ManualClock, channel: u128, expected: &[String]) {
    bounded(
        clock,
        "automatic directory and retained history reconciliation",
        async {
            let mut previous = (usize::MAX, usize::MAX);
            loop {
                let channels = node.actor.channels().await;
                let messages = node.actor.messages(channel).await;
                if previous != (channels.len(), messages.len()) {
                    eprintln!(
                        "temporal inventory: {} channels, {} messages, target present={}",
                        channels.len(),
                        messages.len(),
                        channels.iter().any(|entry| entry.id == channel)
                    );
                    previous = (channels.len(), messages.len());
                }
                if channels.iter().any(|entry| entry.id == channel)
                    && messages
                        .iter()
                        .map(|message| &message.text)
                        .eq(expected.iter())
                {
                    return;
                }
                // The product's periodic discovery command schedules reconciliation. It does not
                // name the new channel, transfer operations, or open that channel on the receiver.
                node.actor.drive_discovery().await.unwrap();
                // Leave a service window between observations: the actor cancels its sync future
                // for each command, so tight polling would repeatedly interrupt recovery.
                // The outer clock driver still advances request/retry deadlines while idle.
                let _ =
                    tokio::time::timeout(Duration::from_millis(300), std::future::pending::<()>())
                        .await;
                clock.advance_ms(1_000);
            }
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_history_crosses_a_crashed_sealed_bridge_without_an_origin_edge() {
    let clock = ManualClock::new(NOW);
    bounded(
        &clock,
        "complete temporal bridge scenario",
        temporal_bridge(&clock),
    )
    .await;
}

async fn temporal_bridge(clock: &ManualClock) {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let c_dir = tempfile::tempdir().unwrap();
    let mut vault_rng = ChaCha20Rng::seed_from_u64(701);
    let a_store = ServerStore::open(a_dir.path(), PASSWORD, &mut vault_rng).unwrap();
    let b_store = ServerStore::open(b_dir.path(), PASSWORD, &mut vault_rng).unwrap();
    let c_store = ServerStore::open(c_dir.path(), PASSWORD, &mut vault_rng).unwrap();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let chain = Arc::new(Chain {
        delivered: Arc::clone(&trace),
        ..Chain::default()
    });

    let mut a_server = Server::found(
        chain.join(A),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(A),
        Box::new(clock.clone()),
        "origin",
    )
    .unwrap();
    a_server.publish_self_record(Vec::new(), 1).unwrap();
    let mut a = Node::start(a_server).await;
    let invite_b = mint(&a.actor, 1).await;
    let mut b_server = bounded(
        clock,
        "B joins A",
        Server::join(
            chain.join(B),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(B),
            Box::new(clock.clone()),
            "bridge",
            PeerId::from_u64(A),
            &invite_b,
        ),
    )
    .await
    .unwrap();
    b_server.publish_self_record(Vec::new(), 1).unwrap();
    // The production eager admission exchange supplies the inviter descriptor required by the
    // explicit helper gate. It neither bypasses the roster nor grants a general proxy route.
    bounded(
        clock,
        "B connected descriptor exchange",
        b_server.request_pex_connected(PeerId::from_u64(A)),
    )
    .await
    .unwrap();
    assert!(bounded(
        clock,
        "B admission finalization",
        b_server.finalize_member_connection(PeerId::from_u64(A))
    )
    .await
    .unwrap());
    let mut b = Node::start(b_server).await;
    let invite_c = mint(&a.actor, 2).await;
    assert!(
        bounded(
            clock,
            "one-time helper capability",
            b.actor.authorize_join_helper(
                PeerId::from_u64(C),
                invite_c.invite_nonce,
                invite_c.inviter_device_id,
                PeerId::from_u64(A),
                clock.now_ms() + 30_000,
            )
        )
        .await
    );
    let mut c_server = bounded(
        clock,
        "C joins through B only",
        Server::join_via_helper(
            chain.join(C),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(C),
            Box::new(clock.clone()),
            "later reader",
            PeerId::from_u64(B),
            PeerId::from_u64(A),
            &invite_c,
        ),
    )
    .await
    .unwrap();
    c_server.publish_self_record(Vec::new(), 1).unwrap();
    bounded(
        clock,
        "C connected descriptor exchange",
        c_server.request_pex_connected(PeerId::from_u64(B)),
    )
    .await
    .unwrap();
    assert!(bounded(
        clock,
        "C admission finalization",
        c_server.finalize_member_connection(PeerId::from_u64(B))
    )
    .await
    .unwrap());
    let mut c = Node::start(c_server).await;
    assert_eq!(c.actor.member_count().await, 3);
    assert_eq!(b.actor.member_count().await, 3);
    assert_eq!(c.actor.group_mode().await.unwrap(), GroupMode::PeerToPeer);
    let c_identity = fingerprint(&c.actor).await;
    c.checkpoint(&c_store, &mut vault_rng).await;
    c.crash().await;
    drop(c_store);
    assert!(!chain
        .live
        .lock()
        .unwrap()
        .contains_key(&PeerId::from_u64(C)));

    // This directory entry did not exist in C's session; opening only `general` or preserving a
    // visible-channel cache cannot satisfy the test. B must acquire its signed directory first.
    let channel = bounded(
        clock,
        "origin creates a new channel",
        a.actor.create_channel("later-history"),
    )
    .await
    .unwrap();
    let expected: Vec<String> = (0..12)
        .map(|n| format!("origin-only retained message {n:02}"))
        .collect();
    for text in &expected {
        a.actor.send_message(channel.id, text.clone()).await;
    }
    await_history(&b, clock, channel.id, &expected).await;
    let original = a.actor.messages(channel.id).await;
    assert_eq!(
        b.actor.messages(channel.id).await,
        original,
        "bridge keeps original authorship and IDs"
    );
    a.checkpoint(&a_store, &mut vault_rng).await;
    b.checkpoint(&b_store, &mut vault_rng).await;
    let b_identity = fingerprint(&b.actor).await;
    a.crash().await;
    b.crash().await;
    drop(a_store);
    drop(b_store);
    assert!(chain.live.lock().unwrap().is_empty());
    drop(chain);

    // A is gone permanently. Fresh vault handles, Hub, transports, RNGs and actors exclude
    // residual socket queues, live author state, or an in-memory bridge snapshot as an oracle.
    let chain = Arc::new(Chain {
        delivered: Arc::clone(&trace),
        ..Chain::default()
    });
    let b_store = ServerStore::open(b_dir.path(), PASSWORD, &mut vault_rng).unwrap();
    let c_store = ServerStore::open(c_dir.path(), PASSWORD, &mut vault_rng).unwrap();
    let b_server = Server::restore(
        &b_store.load_server(1).unwrap(),
        chain.join(B),
        ChaCha20Rng::seed_from_u64(102),
        Box::new(clock.clone()),
        "bridge",
    )
    .unwrap();
    assert_eq!(b_server.my_fingerprint(), b_identity);
    assert_eq!(
        b_server.messages(channel.id),
        original,
        "B reopens retained evidence before any peer exists"
    );
    let mut b = Node::start(b_server).await;
    let c_server = Server::restore(
        &c_store.load_server(1).unwrap(),
        chain.join(C),
        ChaCha20Rng::seed_from_u64(103),
        Box::new(clock.clone()),
        "later reader",
    )
    .unwrap();
    assert_eq!(c_server.my_fingerprint(), c_identity);
    assert!(!c_server
        .channels()
        .iter()
        .any(|entry| entry.id == channel.id));
    assert!(
        c_server.messages(channel.id).is_empty(),
        "C has no origin history in its own vault"
    );
    let mut c = Node::start(c_server).await;
    await_history(&c, clock, channel.id, &expected).await;
    assert_eq!(
        c.actor.messages(channel.id).await,
        original,
        "C recovers exact signed origin messages through reopened B"
    );
    c.checkpoint(&c_store, &mut vault_rng).await;
    b.crash().await;
    c.crash().await;
    drop(c_store);

    // The third device now independently retains the bridge's answer, even with nobody online.
    let c_store = ServerStore::open(c_dir.path(), PASSWORD, &mut vault_rng).unwrap();
    let offline = Arc::new(Chain::default());
    let c_restored = Server::restore(
        &c_store.load_server(1).unwrap(),
        offline.join(C),
        ChaCha20Rng::seed_from_u64(203),
        Box::new(clock.clone()),
        "later reader",
    )
    .unwrap();
    assert_eq!(c_restored.messages(channel.id), original);
    let trace = trace.lock().unwrap();
    assert!(trace
        .iter()
        .all(|(from, to, _)| Chain::adjacent(*from, *to)));
    assert!(trace
        .iter()
        .any(|(from, to, kind)| *from == PeerId::from_u64(B)
            && *to == PeerId::from_u64(A)
            && *kind == 19));
    assert!(trace
        .iter()
        .any(|(from, to, kind)| *from == PeerId::from_u64(C)
            && *to == PeerId::from_u64(B)
            && *kind == 19));
}
