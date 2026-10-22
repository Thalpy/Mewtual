//! Real product TCP transport with explicit gossip/heartbeat loss and completed-reply delay.
use super::*;
use async_trait::async_trait;
use bytes::Bytes;
use catcoms_rt::{
    BoxedDialPermit, DialSubmission, PeerConnectionSnapshot, ProtocolId, PublishOnceError,
    PublishSubmission, Topic, TransportError,
};

const ORDINARY_TRACE_LIMIT: usize = 192;
const PHASE_TRACE_LIMIT: usize = 16;

#[test]
fn ordinary_trace_bounds_phase_and_request_history_with_explicit_truncation() {
    let scenario = Scenario::new(false);
    for _ in 0..PHASE_TRACE_LIMIT + 3 {
        scenario.rules.begin_phase(&"λ".repeat(200));
    }
    for _ in 0..ORDINARY_TRACE_LIMIT + 5 {
        let observation = scenario
            .rules
            .observe_ordinary(0, scenario.rules.peers[1], 19);
        observation.lower(&Ok(Bytes::new()));
        observation.complete();
    }
    for count in 0..ORDINARY_TRACE_LIMIT + 5 {
        scenario.rules.observe_history(0, count, count);
        scenario.rules.observe_history(0, count, count); // No allocation for unchanged oracle.
    }
    let trace = scenario.rules.trace.lock().unwrap();
    assert_eq!(trace.ordinary.len(), ORDINARY_TRACE_LIMIT);
    assert_eq!(trace.ordinary_evicted, 5);
    assert_eq!(trace.phases.len(), PHASE_TRACE_LIMIT);
    assert_eq!(trace.history_samples.len(), ORDINARY_TRACE_LIMIT);
    assert_eq!(
        trace.history_samples.back().unwrap().4,
        ORDINARY_TRACE_LIMIT + 4
    );
    assert!(trace
        .phases
        .iter()
        .all(|(_, _, label)| label.chars().count() == 96));
    assert!(trace.ordinary.iter().all(|row| row.kind == 19
        && row.observer_completed.is_some()
        && row.observer_dropped.is_none()));
    assert!(trace.ordinary.front().unwrap().id < trace.ordinary.back().unwrap().id);
}

#[test]
fn ordinary_trace_distinguishes_lost_waiter_from_a_returned_transport_error() {
    let scenario = Scenario::new(false);
    let observation = scenario
        .rules
        .observe_ordinary(0, scenario.rules.peers[1], 19);
    observation.lower(&Err(TransportError::NoResponse));
    observation.complete();
    let waiting = scenario
        .rules
        .observe_ordinary(0, scenario.rules.peers[1], 19);
    drop(waiting);
    let trace = scenario.rules.trace.lock().unwrap();
    let returned = &trace.ordinary[0];
    assert_eq!(
        returned.lower_completed.unwrap().1,
        OrdinaryOutcome::NoResponse
    );
    assert!(returned.observer_completed.is_some() && returned.observer_dropped.is_none());
    let abandoned = &trace.ordinary[1];
    assert!(abandoned.lower_completed.is_none() && abandoned.observer_completed.is_none());
    assert!(abandoned.observer_dropped.is_some());
}

#[test]
fn ordinary_trace_retains_only_byte_count_after_a_reply_then_outer_waiter_is_lost() {
    struct Payload(Arc<AtomicBool>, [u8; 4]);
    impl AsRef<[u8]> for Payload {
        fn as_ref(&self) -> &[u8] {
            &self.1
        }
    }
    impl Drop for Payload {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let scenario = Scenario::new(false);
    let dropped = Arc::new(AtomicBool::new(false));
    let answer = Ok(Bytes::from_owner(Payload(dropped.clone(), [9; 4])));
    let observation = scenario
        .rules
        .observe_ordinary(0, scenario.rules.peers[1], 19);
    observation.lower(&answer);
    drop(answer);
    assert!(
        dropped.load(Ordering::Acquire),
        "trace must not retain the reply payload"
    );
    drop(observation); // Mirrors cancellation during the intentional post-response delay.
    let trace = scenario.rules.trace.lock().unwrap();
    let row = trace.ordinary.back().unwrap();
    assert_eq!(row.lower_completed.unwrap().1, OrdinaryOutcome::Reply(4));
    assert!(row.observer_dropped.is_some() && row.observer_completed.is_none());
}

/// Test observations only. These fixed codes retain no payload, address or arbitrary driver
/// error string. A received reply is distinct from its eventual return through injected delay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OrdinaryOutcome {
    Reply(usize),
    Unreachable,
    Timeout,
    Closed,
    NoResponse,
    Cancelled,
    InvalidDialBatch,
}
pub(super) struct OrdinaryRequest {
    id: u64,
    phase: u64,
    from: usize,
    to: usize,
    kind: u8,
    began: u64,
    lower_completed: Option<(u64, OrdinaryOutcome)>,
    observer_completed: Option<u64>,
    observer_dropped: Option<u64>,
}
impl std::fmt::Debug for OrdinaryRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrdinaryRequest")
            .field("id", &self.id)
            .field("phase", &self.phase)
            .field("from_client", &(self.from + 1))
            .field("to_client", &(self.to + 1))
            .field("kind", &self.kind)
            .field("began_mono_ms", &self.began)
            .field(
                "lower_elapsed_and_outcome",
                &self
                    .lower_completed
                    .map(|(at, outcome)| (at.saturating_sub(self.began), outcome)),
            )
            .field(
                "observer_completed_after_ms",
                &self
                    .observer_completed
                    .map(|at| at.saturating_sub(self.began)),
            )
            .field(
                "observer_dropped_after_ms",
                &self
                    .observer_dropped
                    .map(|at| at.saturating_sub(self.began)),
            )
            .finish()
    }
}

/// The observer follows the OUTER test adapter future, not the driver's stream. Dropping it
/// records waiter loss only; it cannot cancel native custody or claim a transport terminal.
struct OrdinaryObservation {
    rules: Arc<NetworkRules>,
    id: u64,
    completed: bool,
}
impl OrdinaryObservation {
    fn lower(&self, answer: &Result<Bytes, TransportError>) {
        let outcome = match answer {
            Ok(bytes) => OrdinaryOutcome::Reply(bytes.len()),
            Err(TransportError::Unreachable(_)) => OrdinaryOutcome::Unreachable,
            Err(TransportError::Timeout(_)) => OrdinaryOutcome::Timeout,
            Err(TransportError::Closed) => OrdinaryOutcome::Closed,
            Err(TransportError::NoResponse) => OrdinaryOutcome::NoResponse,
            Err(TransportError::Cancelled) => OrdinaryOutcome::Cancelled,
            Err(TransportError::InvalidDialBatch) => OrdinaryOutcome::InvalidDialBatch,
        };
        let now = SystemClock.monotonic_ms();
        if let Some(row) = self
            .rules
            .trace
            .lock()
            .unwrap()
            .ordinary
            .iter_mut()
            .find(|row| row.id == self.id)
        {
            row.lower_completed = Some((now, outcome));
        }
    }
    fn complete(mut self) {
        let now = SystemClock.monotonic_ms();
        if let Some(row) = self
            .rules
            .trace
            .lock()
            .unwrap()
            .ordinary
            .iter_mut()
            .find(|row| row.id == self.id)
        {
            row.observer_completed = Some(now);
        }
        self.completed = true;
    }
}
impl Drop for OrdinaryObservation {
    fn drop(&mut self) {
        if !self.completed {
            let now = SystemClock.monotonic_ms();
            if let Some(row) = self
                .rules
                .trace
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .ordinary
                .iter_mut()
                .find(|row| row.id == self.id)
            {
                row.observer_dropped = Some(now);
            }
        }
    }
}

#[test]
fn ordinary_trace_drop_preserves_an_original_failure_after_trace_poisoning() {
    let scenario = Scenario::new(false);
    let observation = scenario
        .rules
        .observe_ordinary(0, scenario.rules.peers[1], 19);
    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _held = scenario.rules.trace.lock().unwrap();
        panic!("injected original diagnostic assertion");
    }));
    assert!(poisoned.is_err());
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(observation))).is_ok());
    let trace = scenario
        .rules
        .trace
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    assert!(trace.ordinary.back().unwrap().observer_dropped.is_some());
}

#[derive(Default)]
pub(super) struct Trace {
    phase: u64,
    next_ordinary: u64,
    /// Ring truncation is explicit; this never claims a complete lifetime network trace.
    pub ordinary_evicted: usize,
    pub ordinary: std::collections::VecDeque<OrdinaryRequest>,
    pub phases: std::collections::VecDeque<(u64, u64, String)>,
    /// phase/client1/monotonic time/message count/original-op count. Sample only changes
    /// from the existing exact oracle; no extra actor command or recovery work is introduced.
    pub history_samples: std::collections::VecDeque<(u64, usize, u64, usize, usize)>,
    history_last: [Option<(u64, usize, usize)>; 6],
    pub connections: BTreeSet<(usize, usize)>,
    pub received_requests: BTreeSet<(usize, usize)>,
    pub received_kinds: BTreeMap<(usize, usize, u8), usize>,
    pub connected_submissions: BTreeMap<(usize, usize, u8), (usize, bool, String)>,
    /// Bounded by the six fixture identities; contains no routes, identity seeds or payloads.
    pub peer_dials: BTreeMap<(usize, usize), (usize, String)>,
    /// Driver-confirmed local dial attempts, excluding suppressed batches/errors. This is
    /// neither a connection nor a remote delivery receipt. At most six fixture identities.
    pub submitted_peer_dials: BTreeMap<(usize, usize), usize>,
    pub completed_requests: BTreeSet<(usize, usize)>,
    /// Actual recovery notifications observed at the receiver's transport. Endpoint admission
    /// is checked separately through authenticated indirect-route evidence, never this counter.
    pub received_recovery: BTreeSet<(usize, usize, u8)>,
    pub dropped_gossip: usize,
    pub delayed_replies: usize,
    /// One deliberately lost legacy heartbeat per voice-baseline phase, never an endpoint receipt.
    pub dropped_call_signals: BTreeMap<(usize, usize), usize>,
    /// Local queue admissions only; these counters cannot establish authenticated delivery.
    pub submitted_call_signals: BTreeMap<(usize, usize), usize>,
}

pub(super) struct NetworkRules {
    pub peers: Vec<PeerId>,
    pub libp2p_peers: Vec<libp2p::PeerId>,
    pub drop_gossip: AtomicBool,
    /// Test-only one-shot loss on client 1's next legacy notification. Other kinds and the
    /// connected-only automatic-recovery adapter always reach the real product transport.
    pub drop_next_call_signal: AtomicBool,
    pub delay_replies: AtomicBool,
    /// One actual first kind29 response can wait for an explicit test-controlled release.
    /// The surrounding cancellable request retains the original owner while this latch waits.
    pub file_reply: StdMutex<Option<HeldFileReply>>,
    pub trace: StdMutex<Trace>,
}

pub(super) struct HeldFileReply {
    pub entered: tokio::sync::oneshot::Sender<()>,
    pub release: tokio::sync::oneshot::Receiver<()>,
}

pub(super) fn forbidden(left: usize, right: usize) -> bool {
    matches!((left.min(right), left.max(right)), (1, 2) | (4, 5))
}

impl NetworkRules {
    pub fn observe_history(&self, client: usize, messages: usize, operations: usize) {
        let now = SystemClock.monotonic_ms();
        let mut trace = self.trace.lock().unwrap();
        let phase = trace.phase;
        let sample = (phase, messages, operations);
        if trace.history_last[client] == Some(sample) {
            return;
        }
        trace.history_last[client] = Some(sample);
        if trace.history_samples.len() == ORDINARY_TRACE_LIMIT {
            trace.history_samples.pop_front();
        }
        trace
            .history_samples
            .push_back((phase, client + 1, now, messages, operations));
    }
    pub fn begin_phase(&self, label: &str) {
        let now = SystemClock.monotonic_ms();
        let mut trace = self.trace.lock().unwrap();
        trace.phase = trace.phase.checked_add(1).expect("bounded fixture phases");
        let phase = trace.phase;
        if trace.phases.len() == PHASE_TRACE_LIMIT {
            trace.phases.pop_front();
        }
        trace
            .phases
            .push_back((phase, now, label.chars().take(96).collect()));
    }

    fn observe_ordinary(
        self: &Arc<Self>,
        from: usize,
        peer: PeerId,
        kind: u8,
    ) -> OrdinaryObservation {
        let to = self.index(peer);
        let now = SystemClock.monotonic_ms();
        let mut trace = self.trace.lock().unwrap();
        trace.next_ordinary = trace
            .next_ordinary
            .checked_add(1)
            .expect("bounded fixture request count");
        let id = trace.next_ordinary;
        let phase = trace.phase;
        if trace.ordinary.len() == ORDINARY_TRACE_LIMIT {
            trace.ordinary.pop_front();
            trace.ordinary_evicted += 1;
        }
        trace.ordinary.push_back(OrdinaryRequest {
            id,
            phase,
            from,
            to,
            kind,
            began: now,
            lower_completed: None,
            observer_completed: None,
            observer_dropped: None,
        });
        OrdinaryObservation {
            rules: self.clone(),
            id,
            completed: false,
        }
    }
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
    fn connection_lease(&self, peer: PeerId) -> Option<catcoms_rt::ConnectionLease> {
        self.inner.connection_lease(peer)
    }
    fn supports_connection_leases(&self) -> bool {
        self.inner.supports_connection_leases()
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
        // Queued document recovery (kind19) uses this ordinary request path, NOT the
        // connected-cancellable maintenance counters. Observe both sides of injected delay.
        let observation =
            self.rules
                .observe_ordinary(self.index, peer, data.first().copied().unwrap_or(0));
        let answer = self.inner.request(peer, proto, data).await;
        observation.lower(&answer);
        let answer = self.completed(peer, answer).await;
        observation.complete();
        answer
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
        let kind = data.first().copied().unwrap_or(0);
        let cancelled_at_entry = cancellation.is_cancelled();
        let answer = self
            .inner
            .request_connected_cancellable(peer, proto, data, cancellation.clone())
            .await;
        {
            let mut trace = self.rules.trace.lock().unwrap();
            let outcome = trace
                .connected_submissions
                .entry((self.index, self.rules.index(peer), kind))
                .or_default();
            outcome.0 += 1;
            outcome.1 = cancelled_at_entry;
            outcome.2 = match &answer {
                Ok(bytes) => format!("ok {} bytes", bytes.len()),
                Err(error) => format!("{error:?}"),
            };
        }
        let held = if self.index == 2 && kind == 29 && answer.is_ok() {
            self.rules.file_reply.lock().unwrap().take()
        } else {
            None
        };
        tokio::select! {
            answer = async {
                if let Some(held) = held {
                    let _ = held.entered.send(());
                    let _ = held.release.await;
                }
                self.completed(peer, answer).await
            } => answer,
            _ = cancellation.cancelled() => Err(TransportError::Cancelled),
        }
    }
    async fn notify(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<(), TransportError> {
        let call_target = (self.index == 1 && data.first() == Some(&8))
            .then(|| (self.index, self.rules.index(peer)));
        if let Some(target) = call_target {
            if self
                .rules
                .drop_next_call_signal
                .swap(false, Ordering::AcqRel)
            {
                *self
                    .rules
                    .trace
                    .lock()
                    .unwrap()
                    .dropped_call_signals
                    .entry(target)
                    .or_default() += 1;
                // `notify` acknowledges local queueing, not endpoint receipt. Deliberately
                // model one lost heartbeat without inventing an authenticated receive event.
                return Ok(());
            }
        }
        self.inner.notify(peer, proto, data).await?;
        if let Some(target) = call_target {
            *self
                .rules
                .trace
                .lock()
                .unwrap()
                .submitted_call_signals
                .entry(target)
                .or_default() += 1;
        }
        Ok(())
    }
    async fn notify_connected(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
    ) -> Result<(), TransportError> {
        self.inner.notify_connected(peer, proto, data).await
    }
    async fn notify_connected_cancellable(
        &self,
        peer: PeerId,
        proto: ProtocolId,
        data: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<(), TransportError> {
        // Preserve the exact authority and lower keepalive. The fail-closed trait default
        // intentionally refuses unsupported adapters; it must not silently replace real TCP.
        self.inner
            .notify_connected_cancellable(peer, proto, data, cancellation)
            .await
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
        let outcome = MeshTransport::dial_peer_permits(&self.inner, peer, permits).await;
        let mut trace = self.rules.trace.lock().unwrap();
        if outcome
            .as_ref()
            .is_ok_and(|outcomes| outcomes.contains(&DialSubmission::Submitted))
        {
            *trace
                .submitted_peer_dials
                .entry((self.index, self.rules.index(peer)))
                .or_default() += 1;
        }
        let entry = trace
            .peer_dials
            .entry((self.index, self.rules.index(peer)))
            .or_default();
        entry.0 += 1;
        entry.1 = format!("{outcome:?}");
        outcome
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
                TransportEvent::Request { from, data, .. } => {
                    let mut trace = self.rules.trace.lock().unwrap();
                    let source = self.rules.index(*from);
                    trace.received_requests.insert((self.index, source));
                    if let Some(kind) = data.first() {
                        *trace
                            .received_kinds
                            .entry((self.index, source, *kind))
                            .or_default() += 1;
                    }
                    if let Some(kind @ (14..=17)) = data.first() {
                        trace.received_recovery.insert((self.index, source, *kind));
                    }
                }
                _ => {}
            }
            return Some(event);
        }
    }
}

/// A successful local submission cannot satisfy this test: the returned witness exists only
/// after the helper authenticates the probe and the requester authenticates its exact result.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn automatic_recovery_notifications_reach_authenticated_tcp_endpoints() {
    let mut scenario = Scenario::new(false);
    // Only these three devices are members. Closing three members of a six-member group would
    // leave authenticated offline targets/helper witnesses in the ordinary recovery schedule,
    // which does not establish receipt through the specific adapter path asserted below.
    scenario.admit_prefix(3).await;
    let target = *scenario.rules.peers[2].as_bytes();
    tokio::select! {
        _ = async {
            loop {
                if scenario
                    .online(1)
                    .actor
                    .member_routes()
                    .await
                    .iter()
                    .any(|route| {
                        route.peer_id == Some(target)
                            && route.indirect_health
                                == catcoms_sync::IndirectRouteHealth::ReachableViaMember
                            && route.indirect_witnesses > 0
                    })
                {
                    break;
                }
                SystemClock.sleep(Duration::from_millis(50)).await;
            }
        } => {},
        _ = SystemClock.sleep(Duration::from_millis(PERIODIC_RECOVERY_MS)) => {
            // These independent, bounded reads distinguish a missing connection, missing
            // authenticated recovery work and missing result reconciliation. Never hold the
            // trace mutex across the diagnostic actor awaits or print private network seeds.
            let mut recipients = Vec::new();
            for index in 0..3 {
                recipients.push(scenario.recipient_diagnostic(index, channel_id("general"), &History::new()).await);
            }
            let trace = {
                let trace = scenario.rules.trace.lock().unwrap();
                format!("received_recovery={:?}; received_kinds={:?}; connections={:?}", trace.received_recovery, trace.received_kinds, trace.connections)
            }; // Release before panic so background recorders do not add mutex-poison failures.
            panic!("automatic authenticated indirect probe exceeded {PERIODIC_RECOVERY_MS} ms: recipients={recipients:?}; {trace}");
        }
    }
    {
        let trace = scenario.rules.trace.lock().unwrap();
        assert!(
            trace.received_recovery.contains(&(0, 1, 16)),
            "probe must reach the permitted helper"
        );
        assert!(
            trace.received_recovery.contains(&(1, 0, 17)),
            "authenticated result must reach its requester"
        );
    }
    scenario.rules.assert_graph();
    scenario.close(&[0, 1, 2]).await;
}
