//! Real product TCP transport; the fixture can only discard gossip and delay completed replies.
use super::*;
use async_trait::async_trait;
use bytes::Bytes;
use catcoms_rt::{
    BoxedDialPermit, DialSubmission, PeerConnectionSnapshot, ProtocolId, PublishOnceError,
    PublishSubmission, Topic, TransportError,
};

#[derive(Default)]
pub(super) struct Trace {
    pub connections: BTreeSet<(usize, usize)>,
    pub received_requests: BTreeSet<(usize, usize)>,
    pub completed_requests: BTreeSet<(usize, usize)>,
    pub dropped_gossip: usize,
    pub delayed_replies: usize,
}

pub(super) struct NetworkRules {
    pub peers: Vec<PeerId>,
    pub libp2p_peers: Vec<libp2p::PeerId>,
    pub drop_gossip: AtomicBool,
    pub delay_replies: AtomicBool,
    pub trace: StdMutex<Trace>,
}

pub(super) fn forbidden(left: usize, right: usize) -> bool {
    matches!((left.min(right), left.max(right)), (1, 2) | (4, 5))
}

impl NetworkRules {
    fn index(&self, peer: PeerId) -> usize {
        self.peers
            .iter()
            .position(|candidate| *candidate == peer)
            .expect("fixture has exactly six identities")
    }

    pub fn assert_graph(&self) {
        let trace = self.trace.lock().unwrap();
        for (kind, edges) in [
            ("connection", &trace.connections),
            ("received request", &trace.received_requests),
            ("completed request", &trace.completed_requests),
        ] {
            assert!(
                edges.iter().all(|(left, right)| !forbidden(*left, *right)),
                "forbidden {kind}: {edges:?}"
            );
        }
    }
}

pub(super) struct Transport {
    inner: MeshService,
    index: usize,
    rules: Arc<NetworkRules>,
}

impl Transport {
    pub async fn new(index: usize, net: &mut ServerNet, rules: Arc<NetworkRules>) -> Self {
        let mut swarm =
            catcoms_net::build_tcp_swarm_with_key(keypair_from_seed(net.key_seed).unwrap())
                .unwrap();
        for other in 0..6 {
            if forbidden(index, other) {
                swarm
                    .behaviour_mut()
                    .blocked_peers
                    .block_peer(rules.libp2p_peers[other]);
            }
        }
        swarm
            .listen_on(format!("/ip4/127.0.0.1/tcp/{}", net.port).parse().unwrap())
            .unwrap();
        let inner = MeshService::spawn(swarm);
        let listener = bounded("TCP listener", inner.next_listen_addr())
            .await
            .unwrap();
        net.port = listen_port(&listener).expect("TCP fixture listener");
        assert_eq!(inner.local_peer(), rules.peers[index]);
        Self {
            inner,
            index,
            rules,
        }
    }

    pub fn handle(&self) -> MeshHandle {
        self.inner.handle()
    }

    pub async fn connected(&self, peer: PeerId) {
        self.inner.wait_for_peer_connected(peer).await.unwrap();
    }

    pub async fn driver_lifetime(&self) -> watch::Receiver<MeshObservationSnapshot> {
        self.inner.take_mesh_observation_snapshots().await.unwrap()
    }

    async fn completed(
        &self,
        peer: PeerId,
        answer: Result<Bytes, TransportError>,
    ) -> Result<Bytes, TransportError> {
        if answer.is_ok() {
            self.rules
                .trace
                .lock()
                .unwrap()
                .completed_requests
                .insert((self.index, self.rules.index(peer)));
            if self.rules.delay_replies.load(Ordering::Acquire) {
                self.rules.trace.lock().unwrap().delayed_replies += 1;
                SystemClock.sleep(Duration::from_millis(125)).await;
            }
        }
        answer
    }
}

#[async_trait]
impl MeshTransport for Transport {
    fn local_peer(&self) -> PeerId {
        self.inner.local_peer()
    }
    fn connection_snapshot(&self) -> Vec<PeerConnectionSnapshot> {
        let snapshot = self.inner.connection_snapshot();
        for connected in &snapshot {
            self.rules
                .trace
                .lock()
                .unwrap()
                .connections
                .insert((self.index, self.rules.index(connected.peer)));
        }
        snapshot
    }
    async fn subscribe(&self, topic: Topic) -> Result<(), TransportError> {
        self.inner.subscribe(topic).await
    }
    async fn unsubscribe(&self, topic: Topic) -> Result<(), TransportError> {
        self.inner.unsubscribe(topic).await
    }
    async fn publish(&self, topic: Topic, data: Bytes) -> Result<(), TransportError> {
        self.inner.publish(topic, data).await
    }
    async fn publish_once(
        &self,
        topic: Topic,
        data: Bytes,
    ) -> Result<PublishSubmission, PublishOnceError> {
        self.inner.publish_once(topic, data).await
    }
    async fn request(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        self.completed(peer, self.inner.request(peer, proto, data).await)
            .await
    }
    async fn request_connected(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        self.completed(peer, self.inner.request_connected(peer, proto, data).await)
            .await
    }
    async fn request_cancellable(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
        mut cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        // Lower-level custody remains with the real driver. The extra response delay is itself
        // cancellable and applies to queued recovery as well as direct requests.
        let answer = self
            .inner
            .request_cancellable(peer, proto, data, cancellation.clone())
            .await;
        tokio::select! {
            answer = self.completed(peer, answer) => answer,
            _ = cancellation.cancelled() => Err(TransportError::Cancelled),
        }
    }
    async fn request_connected_cancellable(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
        mut cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        let answer = self
            .inner
            .request_connected_cancellable(peer, proto, data, cancellation.clone())
            .await;
        tokio::select! {
            answer = self.completed(peer, answer) => answer,
            _ = cancellation.cancelled() => Err(TransportError::Cancelled),
        }
    }
    async fn notify(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<(), TransportError> {
        self.inner.notify(peer, proto, data).await
    }
    async fn notify_connected(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<(), TransportError> {
        self.inner.notify_connected(peer, proto, data).await
    }
    async fn dial_addr(&self, address: &str) -> Result<(), TransportError> {
        self.inner.dial_addr(address).await
    }
    async fn dial_addr_outcome(&self, address: &str) -> Result<DialSubmission, TransportError> {
        self.inner.dial_addr_outcome(address).await
    }
    async fn dial_permit(&self, permit: BoxedDialPermit) -> Result<DialSubmission, TransportError> {
        MeshTransport::dial_permit(&self.inner, permit).await
    }
    async fn dial_peer_batch(
        &self,
        peer: PeerId,
        addresses: &[String],
    ) -> Result<Vec<DialSubmission>, TransportError> {
        self.inner.dial_peer_batch(peer, addresses).await
    }
    async fn dial_peer_permits(
        &self,
        peer: PeerId,
        permits: Vec<BoxedDialPermit>,
    ) -> Result<Vec<DialSubmission>, TransportError> {
        MeshTransport::dial_peer_permits(&self.inner, peer, permits).await
    }
    async fn evict_peer(&self, peer: PeerId) -> Result<(), TransportError> {
        self.inner.evict_peer(peer).await
    }
    async fn unevict_peer(&self, peer: PeerId) -> Result<(), TransportError> {
        self.inner.unevict_peer(peer).await
    }
    async fn next_event(&self) -> Option<TransportEvent> {
        loop {
            let event = self.inner.next_event().await?;
            match &event {
                TransportEvent::Gossip { .. } if self.rules.drop_gossip.load(Ordering::Acquire) => {
                    self.rules.trace.lock().unwrap().dropped_gossip += 1;
                    continue;
                }
                TransportEvent::PeerConnected(peer) => {
                    self.rules
                        .trace
                        .lock()
                        .unwrap()
                        .connections
                        .insert((self.index, self.rules.index(*peer)));
                }
                TransportEvent::PeerPathsChanged { peer, active, .. } if !active.is_empty() => {
                    self.rules
                        .trace
                        .lock()
                        .unwrap()
                        .connections
                        .insert((self.index, self.rules.index(*peer)));
                }
                TransportEvent::Request { from, .. } => {
                    self.rules
                        .trace
                        .lock()
                        .unwrap()
                        .received_requests
                        .insert((self.index, self.rules.index(*from)));
                }
                _ => {}
            }
            return Some(event);
        }
    }
}
