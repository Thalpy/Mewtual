//! Six actual TCP clients exercise the native offline/restart path without injected history.
//!
//! The blocked peer graph is a deterministic transport restriction, not a claim to emulate NAT.
//! The adapter runs the shared production cadence and native worker/send/close/restore seams;
//! platform interface sampling and the Tauri window/frontend are deliberately outside this test.
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::collections::{BTreeMap, BTreeSet};

mod chat_bubbles;
mod chat_matrix;
mod files;
mod isolation;
mod runtime;
mod signed_history;
mod transport;
mod voice;
mod voice_live;
use runtime::{mount, network, register, Running};
use transport::{forbidden, NetworkRules, Trace, Transport};

/// Each scenario models separate desktops inside ONE test process. Independent scenarios
/// must not accidentally compete for that process's maintenance/file/helper pools. The six
/// clients WITHIN one scenario still run concurrently with unchanged production limits.
struct FixtureAdmission {
    capacity: Arc<tokio::sync::Semaphore>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
async fn isolate() -> FixtureAdmission {
    static CAPACITY: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    let capacity = CAPACITY
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1)))
        .clone();
    let permit = capacity.clone().acquire_owned().await.expect(
        "an earlier recovery fixture failed before actual actor/driver shutdown; inspect its failure first"
    );
    FixtureAdmission {
        capacity,
        _permit: permit,
    }
}

/// Two independent oracles: the product projection and the exact original signed-operation
/// union. Rendered equality alone cannot detect a helper re-authoring or dropping signed history.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct History {
    messages: BTreeMap<String, (String, String)>,
    operations: BTreeMap<[u8; 32], catcoms_replication::SignedOp>,
}
impl History {
    fn new() -> Self {
        Self::default()
    }
    fn extend(&mut self, other: Self) {
        for (id, value) in other.messages {
            if let Some(old) = self.messages.insert(id, value.clone()) {
                assert_eq!(old, value, "partition overlap changed a message");
            }
        }
        for (hash, op) in other.operations {
            if let Some(old) = self.operations.insert(hash, op.clone()) {
                assert_eq!(
                    old, op,
                    "partition overlap changed an original signed operation"
                );
            }
        }
    }
}
const ALL: &[usize] = &[0, 1, 2, 3, 4, 5];

async fn bounded<F: Future>(label: &str, future: F) -> F::Output {
    tokio::select! {
        result = future => result,
        _ = SystemClock.sleep(Duration::from_secs(30)) => panic!("six-client operation exceeded its bound: {label}"),
    }
}

/// Keep ownership outside select's branches: on timeout, unwind starts before the unfinished
/// Scenario drops. Otherwise its strict teardown assertion would hide the original timeout.
async fn scenario_bounded<F: Future>(
    future: F,
    timeout: impl Future<Output = ()>,
    failure: &str,
) -> F::Output {
    tokio::pin!(future);
    tokio::select! {
        result = &mut future => result,
        _ = timeout => panic!("{failure}"),
    }
}

async fn close_driver(driver: &mut Option<watch::Receiver<MeshObservationSnapshot>>) {
    // Keep the pending marker indexed across cancellation/timeout. Only observed closure
    // allows release, even if Running already completed its own actor/save barrier.
    bounded("TCP transport shutdown", async {
        let receiver = driver.as_mut().expect("original driver lifetime");
        while receiver.changed().await.is_ok() {}
    })
    .await;
    driver.take();
}

// Automatic recovery can start just after an empty exchange and just after a cadence tick.
// Account for the released source cooldown, worst-case discovery interval/start spread and
// four bounded transport stages. Command responsiveness keeps its independent 30-second bound.
// Real TCP uses production time: accelerating a shared clock also expires prepared jobs and
// retained completions, even if no network future is currently waiting.
const PERIODIC_RECOVERY_MS: u64 = catcoms_sync::CATCHUP_PEER_COOLDOWN_MS
    + DISCOVERY_INTERVAL_SECS * 1_000
    + DISCOVERY_JITTER_MS
    + DISCOVERY_START_SPREAD_MS
    + 4 * catcoms_rt::REQUEST_TIMEOUT_MS;

async fn periodic_bounded<F: Future>(label: &str, future: F) -> F::Output {
    tokio::select! {
        result = future => result,
        _ = SystemClock.sleep(Duration::from_millis(PERIODIC_RECOVERY_MS)) => panic!("six-client periodic recovery exceeded {PERIODIC_RECOVERY_MS} ms: {label}"),
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Client {
    root: tempfile::TempDir,
    net: ServerNet,
    identity: Option<(Vec<u8>, DeviceId)>,
    generation: u64,
    running: Option<Running>,
    driver: Option<watch::Receiver<MeshObservationSnapshot>>,
    // Transport::new spawns the real driver before it can return a receiver. Cancellation
    // in that interval is unfinished custody, including on the first client/reopen.
    driver_starting: bool,
}
impl Drop for Client {
    fn drop(&mut self) {
        if self.driver_starting || self.running.is_some() || self.driver.is_some() {
            // A failed fixture aborts tasks but cannot claim they synchronously drained.
            // Preserve its vault until those actual owners disappear rather than deleting
            // storage ahead of their asynchronous shutdown and obscuring the original error.
            // This preserves the failed vault for later inspection/explicit cleanup.
            self.root.disable_cleanup(true);
        }
    }
}

struct Scenario {
    clients: Vec<Client>,
    rules: Arc<NetworkRules>,
    clock: SystemClock,
    reverse: bool,
    token: u128,
    // Last: successful scenarios have awaited every actor and driver before returning the
    // isolation turn. Failure closes admission instead of starting another contaminated run.
    admission: Option<FixtureAdmission>,
}
impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(admission) = &self.admission {
            if self.clients.iter().any(|client| {
                client.driver_starting || client.running.is_some() || client.driver.is_some()
            }) {
                admission.capacity.close();
                assert!(std::thread::panicking(), "recovery fixture must await all actual actors/drivers before releasing admission");
            }
        }
    }
}

impl Scenario {
    fn new(reverse: bool, admission: FixtureAdmission) -> Self {
        Self::build(reverse, Some(admission))
    }
    /// Trace-only tests create no actor/driver; admit_prefix refuses this unstarted owner.
    fn observations(reverse: bool) -> Self {
        Self::build(reverse, None)
    }
    fn build(reverse: bool, admission: Option<FixtureAdmission>) -> Self {
        // Opt-in structured diagnostics keep the normal fixture quiet while making a failed
        // stage reproducible without changing its schedule or manufacturing recovery work.
        if std::env::var_os("RUST_LOG").is_some() {
            catcoms_log::init_test();
        }
        let clients: Vec<_> = (0..6)
            .map(|_| {
                let mut net = new_server_net("", "", "");
                net.port = 0;
                Client {
                    root: tempfile::tempdir().unwrap(),
                    net,
                    identity: None,
                    generation: 0,
                    running: None,
                    driver: None,
                    driver_starting: false,
                }
            })
            .collect();
        let libp2p_peers: Vec<_> = clients
            .iter()
            .map(|client| {
                keypair_from_seed(client.net.key_seed)
                    .unwrap()
                    .public()
                    .to_peer_id()
            })
            .collect();
        let peers: Vec<_> = libp2p_peers.iter().map(phase0_peer_id).collect();
        assert_eq!(peers.iter().collect::<BTreeSet<_>>().len(), 6);
        let rules = Arc::new(NetworkRules {
            peers,
            libp2p_peers,
            drop_gossip: AtomicBool::new(false),
            drop_next_call_signal: AtomicBool::new(false),
            delay_replies: AtomicBool::new(false),
            file_reply: StdMutex::new(None),
            trace: StdMutex::new(Trace::default()),
        });
        let clock = SystemClock;
        Self {
            clients,
            rules,
            clock,
            reverse,
            token: 0,
            admission,
        }
    }

    fn online(&self, index: usize) -> &Running {
        self.clients[index].running.as_ref().expect("client online")
    }

    fn address(&self, index: usize) -> Multiaddr {
        format!(
            "/ip4/127.0.0.1/tcp/{}/p2p/{}",
            self.clients[index].net.port, self.rules.libp2p_peers[index]
        )
        .parse()
        .unwrap()
    }

    async fn admit(&mut self) {
        self.admit_prefix(6).await;
    }

    /// Admit actual members through the native production path. The transport adapter's
    /// three-member check needs one unambiguous helper and one blocked target; admitting and
    /// then closing extra members would leave their valid descriptors eligible for probing.
    async fn admit_prefix(&mut self, count: usize) {
        self.admit_prefix_with_files(count, false).await;
    }

    async fn admit_prefix_with_files(&mut self, count: usize, files: bool) {
        assert!(
            self.admission.is_some(),
            "real clients require isolated fixture admission"
        );
        assert!(matches!(count, 3 | 6));
        for index in 0..count {
            let state = mount(self.clients[index].root.path()).await;
            self.clients[index].driver_starting = true;
            let transport =
                Transport::new(index, &mut self.clients[index].net, self.rules.clone()).await;
            let mesh = transport.handle();
            self.clients[index].driver = Some(transport.driver_lifetime().await);
            self.clients[index].driver_starting = false;
            let mut server = if index == 0 {
                Server::found(
                    transport,
                    MlsDevice::generate().unwrap(),
                    ChaCha20Rng::seed_from_u64(90),
                    Box::new(self.clock),
                    "client 1",
                )
                .unwrap()
            } else {
                // Bootstrap is allowed only during original admission. After the first close,
                // every connection must come from native restore/discovery and sealed routes.
                mesh.dial(self.address(0)).await.unwrap();
                bounded(
                    "initial authenticated connection",
                    transport.connected(self.rules.peers[0]),
                )
                .await;
                let invite = bounded(
                    "owner invite",
                    self.online(0).actor.mint_invite(
                        [index as u8; 16],
                        self.clock.now_ms() + 3_600_000,
                        Vec::new(),
                    ),
                )
                .await
                .unwrap();
                let invite = InviteToken::decode(&invite).unwrap();
                bounded(
                    "original member admission",
                    Server::join(
                        transport,
                        MlsDevice::generate().unwrap(),
                        ChaCha20Rng::seed_from_u64(90 + index as u64),
                        Box::new(self.clock),
                        format!("client {}", index + 1),
                        self.rules.peers[0],
                        &invite,
                    ),
                )
                .await
                .unwrap()
            };
            server.set_endpoint_dial_scheduler(state.endpoint_dials.clone());
            if index == 0 {
                server.subscribe_control().await.unwrap();
                server
                    .publish_self_record(Vec::new(), self.clients[index].net.record_seq)
                    .unwrap();
            } else {
                bounded(
                    "production admission finalization",
                    finalize_admission_discovery(
                        &mut server,
                        self.rules.peers[0],
                        Vec::new(),
                        self.clients[index].net.record_seq,
                        Duration::from_secs(2),
                    ),
                )
                .await
                .unwrap();
            }
            let group_id = server.group_id();
            let device_id = server.device_id();
            self.clients[index].identity = Some((group_id.clone(), device_id));
            self.clients[index].generation += 1;
            if files {
                // The file scenario must use the same encrypted/retained native store factory
                // as a real desktop, installed before any actor can create file bytes.
                attach_blob_store(&state, &mut server).await;
            }
            let (actor, events, task) = spawn(server);
            let restored = RestoredActor {
                actor,
                events,
                task,
                group_id,
                device_id,
            };
            self.clients[index].running = Some(
                register(
                    state,
                    restored,
                    mesh,
                    &self.clients[index].net,
                    self.clients[index].generation,
                    &self.clock,
                    index,
                )
                .await,
            );
        }
        bounded("all original admissions become visible", async {
            loop {
                let mut complete = true;
                for index in 0..count {
                    complete &= self.online(index).actor.members().await.len() == count;
                }
                if complete {
                    break;
                }
                SystemClock.sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        // Higher numbered clients retain outbound evidence to the earlier listeners. This makes
        // 2->1/3->1 and 5->4/6->4 available without pretending an inbound connection proves a dial.
        for source in 1..count {
            for target in 0..source {
                if !forbidden(source, target) {
                    self.online(source)
                        .mesh
                        .dial(self.address(target))
                        .await
                        .unwrap();
                }
            }
        }
        self.wait_for_routes(count).await;
        for index in 0..count {
            assert_eq!(
                self.online(index).actor.group_mode().await.unwrap(),
                catcoms_app::GroupMode::PeerToPeer
            );
            assert!(
                self.online(index)
                    .actor
                    .member_routes()
                    .await
                    .iter()
                    .all(|route| route.addresses.is_empty()),
                "the fixture must not publish a shortcut address"
            );
        }
        self.rules.assert_graph();
    }

    async fn wait_for_routes(&self, count: usize) {
        // dial() only queues an attempt. The missed-membership scenario later leaves only
        // clients 1 and 3 online, so it also needs 3's authenticated, persisted route to 1.
        // Establish that custody during original admission, never by injecting a restore hint.
        let required = [(1, 0), (2, 0), (3, 0), (3, 1), (4, 3), (5, 3)];
        tokio::select! {
            _ = async {
                loop {
                    let mut complete = true;
                    for (source, target) in required.into_iter().filter(|(source, target)| *source < count && *target < count) {
                        let net = network(&self.online(source).state).await;
                        complete &= net.reconnect_policy == ReconnectPolicy::MemberMesh && net.reconnect_routes.iter().any(|route| route.peer_id == *self.rules.peers[target].as_bytes());
                    }
                    if complete { break; }
                    SystemClock.sleep(Duration::from_millis(100)).await;
                }
            } => {},
            _ = SystemClock.sleep(Duration::from_secs(90)) => {
                let mut actual = Vec::new();
                for index in 0..count {
                    actual.push(self.recipient_diagnostic(index, channel_id("general"), &History::new()).await);
                }
                panic!("native workers did not retain authenticated bridge routes: {actual:?}");
            }
        }
    }

    async fn reopen(&mut self, indices: &[usize]) {
        self.reopen_in(indices, true).await;
    }

    /// A changing-membership scenario checks its full roster after automatic recovery. Checking
    /// a saved count after spawning would race a valid control commit from an already live peer.
    async fn reopen_in(&mut self, indices: &[usize], fixed_membership: bool) {
        let mut order = indices.to_vec();
        if self.reverse {
            order.reverse();
        }
        for index in order {
            assert!(self.clients[index].running.is_none());
            let state = mount(self.clients[index].root.path()).await;
            let mut net = load_or_init_server_net(&state, 1, "").await.unwrap();
            assert_eq!(net.key_seed, self.clients[index].net.key_seed);
            assert!(net.advertise.is_empty() && net.relay.is_empty() && net.rendezvous.is_empty());
            let snapshot = state
                .store
                .lock()
                .await
                .as_ref()
                .unwrap()
                .load_server(1)
                .unwrap();
            self.clients[index].driver_starting = true;
            let transport = Transport::new(index, &mut net, self.rules.clone()).await;
            let mesh = transport.handle();
            self.clients[index].driver = Some(transport.driver_lifetime().await);
            self.clients[index].driver_starting = false;
            let record = ServerRecord {
                id: 1,
                display_name: format!("client {}", index + 1),
                invite: String::new(),
                is_dm: false,
            };
            let restored = bounded(
                "native actor restore",
                restore_server_actor(
                    &state,
                    &snapshot,
                    &record,
                    transport,
                    ChaCha20Rng::seed_from_u64(
                        300 + index as u64 + self.clients[index].generation * 6,
                    ),
                    Box::new(self.clock),
                    &[],
                    net.record_seq,
                    net.reconnect_policy,
                    net.reconnect_routes
                        .iter()
                        .map(|route| (PeerId::new(route.peer_id), route.address.clone()))
                        .collect(),
                    false,
                ),
            )
            .await
            .unwrap();
            assert_eq!(
                Some(&(restored.group_id.clone(), restored.device_id)),
                self.clients[index].identity.as_ref()
            );
            self.clients[index].net = net;
            self.clients[index].generation += 1;
            self.clients[index].running = Some(
                register(
                    state,
                    restored,
                    mesh,
                    &self.clients[index].net,
                    self.clients[index].generation,
                    &self.clock,
                    index,
                )
                .await,
            );
            if fixed_membership {
                assert_eq!(
                    self.online(index).actor.members().await.len(),
                    6,
                    "membership is fixed across every restart"
                );
            }
        }
    }

    async fn close(&mut self, indices: &[usize]) {
        for index in indices {
            self.clients[*index]
                .running
                .take()
                .expect("client online before close")
                .close()
                .await;
            // This existing watch receiver carries no command sender. Closure proves the real
            // driver dropped its swarm; unlike a plain socket bind probe it does not mistake
            // TIME_WAIT for a live product listener on platforms with different reuse flags.
            // Keep the actual pending receiver indexed until shutdown is OBSERVED. A timeout
            // must not make Scenario::Drop mistake a taken-but-still-live driver for closure.
            close_driver(&mut self.clients[*index].driver).await;
        }
    }

    async fn author(
        &mut self,
        indices: &[usize],
        phase: &str,
        history: &mut History,
    ) -> Vec<String> {
        self.author_in(channel_id("general"), indices, phase, history)
            .await
    }

    async fn author_in(
        &mut self,
        channel: u128,
        indices: &[usize],
        phase: &str,
        history: &mut History,
    ) -> Vec<String> {
        let mut accepted_ids = Vec::with_capacity(indices.len());
        for index in indices {
            self.token += 1;
            let text = format!("{phase}: original client {}", index + 1);
            let before = self.online(*index).history_in(channel).await;
            let id = self
                .online(*index)
                .send_in(channel, self.token, &text)
                .await;
            let device = self.clients[*index].identity.as_ref().unwrap().1;
            let author = fingerprint(&device);
            let after = self.online(*index).history_in(channel).await;
            let original: Vec<_> = after
                .operations
                .iter()
                .filter(|(hash, op)| {
                    op.author_device == device && !before.operations.contains_key(*hash)
                })
                .collect();
            assert_eq!(
                original.len(),
                1,
                "one native durable send must retain one original device-signed operation"
            );
            let (hash, op) = original[0];
            assert!(op.verify());
            assert!(
                history.operations.insert(*hash, op.clone()).is_none(),
                "native acceptance reused an operation identity"
            );
            assert!(
                history
                    .messages
                    .insert(id.clone(), (author, text))
                    .is_none(),
                "native acceptance reused an ID"
            );
            accepted_ids.push(id);
        }
        accepted_ids
    }

    async fn converge(&self, indices: &[usize], expected: &History, phase: &str) {
        self.converge_in(channel_id("general"), indices, expected, phase)
            .await;
    }

    /// Failure observations have their own short bound: an unresponsive actor must not turn a
    /// ninety-second recovery failure into an unbounded diagnostic wait. Never print ServerNet;
    /// it contains the transport identity seed. Routes are projected to fixture indexes and an
    /// equality check against each actual listener, with no raw addresses or secret material.
    async fn recipient_diagnostic(
        &self,
        index: usize,
        channel: u128,
        expected: &History,
    ) -> String {
        async fn observe<F: Future>(future: F) -> Option<F::Output> {
            tokio::select! {
                value = future => Some(value),
                _ = SystemClock.sleep(Duration::from_millis(500)) => None,
            }
        }
        let running = self.online(index);
        // Independent observations distinguish a busy actor from sealed-store contention. One
        // slow read cannot erase the useful state returned by the other bounded probes.
        let (history, net, recovery, routes) = tokio::join!(
            observe(running.history_in(channel)),
            observe(network(&running.state)),
            observe(
                running
                    .actor
                    .recovery_progress(catcoms_wire::DocType::Channel, channel)
            ),
            observe(running.actor.member_routes()),
        );
        let history = history.map_or_else(|| "history actor/snapshot probe exceeded 500ms".to_owned(), |history| {
            // Fixed fixture labels/author identities identify which partition's operation is
            // missing. Cap observations; never dump signed payloads or unbounded history.
            let missing_messages: Vec<_> = expected.messages.iter().filter(|(id, _)| !history.messages.contains_key(*id))
                .take(4).map(|(id, (author, label))| (id, author, label.chars().take(80).collect::<String>())).collect();
            let unexpected_messages: Vec<_> = history.messages.keys().filter(|id| !expected.messages.contains_key(*id)).collect();
            let changed_messages: Vec<_> = expected.messages.iter().filter(|(id, value)| history.messages.get(*id).is_some_and(|actual| actual != *value)).map(|(id, _)| id).collect();
            let missing_operations: Vec<_> = expected.operations.keys().filter(|id| !history.operations.contains_key(*id)).map(hex::encode).collect();
            let extra_operations: Vec<_> = history.operations.keys().filter(|id| !expected.operations.contains_key(*id)).map(hex::encode).collect();
            format!("messages={} ops={} missing_messages={missing_messages:?} unexpected_messages={unexpected_messages:?} changed_messages={changed_messages:?} missing_operations={missing_operations:?} extra_operations={extra_operations:?}", history.messages.len(), history.operations.len())
        });
        let saved = self.sealed_route_diagnostic(net);
        format!("client={} {history} cadence_submitted_passes={} network={saved} recovery={recovery:?} routes={routes:?}",
            index + 1, running.passes.load(Ordering::Acquire))
    }

    /// Project only fixture identities and listener equality; ServerNet itself contains secrets.
    /// Callers bound the store query separately from actor observations so one cannot hide the other.
    fn sealed_route_diagnostic(&self, net: Option<ServerNet>) -> String {
        net.map_or_else(
            || "sealed network store probe exceeded 500ms".to_owned(),
            |net| {
                let targets: Vec<_> = net
                    .reconnect_routes
                    .iter()
                    .map(|route| {
                        let target = self
                            .rules
                            .peers
                            .iter()
                            .position(|peer| peer.as_bytes() == &route.peer_id);
                        (
                            target.map(|target| target + 1),
                            target.is_some_and(|target| {
                                route.address == self.address(target).to_string()
                            }),
                        )
                    })
                    .collect();
                format!(
                    "policy={:?} saved_targets_and_current_listener={targets:?}",
                    net.reconnect_policy
                )
            },
        )
    }

    async fn converge_in(&self, channel: u128, indices: &[usize], expected: &History, phase: &str) {
        eprintln!("six-client phase: {phase}");
        self.rules.begin_phase(phase);
        tokio::select! {
            _ = async {
                loop {
                    let mut complete = true;
                    for index in indices {
                        let history = self.online(*index).history_in(channel).await;
                        self.rules.observe_history(*index, history.messages.len(), history.operations.len());
                        complete &= history == *expected;
                    }
                    if complete { break; }
                    SystemClock.sleep(Duration::from_millis(100)).await;
                }
            } => {},
            _ = SystemClock.sleep(Duration::from_secs(90)) => {
                let mut actual = Vec::new();
                for index in indices {
                    // Keep the exact equality oracle above. Diagnostics identify the failed
                    // recipient and scheduler state without dumping entire signed payloads.
                    actual.push(self.recipient_diagnostic(*index, channel, expected).await);
                }
                let trace = {
                    let trace = self.rules.trace.lock().unwrap();
                    format!("received_kinds={:?}; connected_maintenance_submissions={:?}; peer_dials={:?}; phases={:?}; history_samples_phase_client1_mono_messages_ops={:?}; ordinary_last_192_evicted={}; ordinary_requests={:?}", trace.received_kinds, trace.connected_submissions, trace.peer_dials, trace.phases, trace.history_samples, trace.ordinary_evicted, trace.ordinary)
                }; // Release the diagnostic mutex before panic; secondary tasks retain usable logs.
                panic!("{phase} failed exact ID/author/text/original-signed-operation convergence; expected_messages={} expected_ops={} recipients={actual:?}; {trace}",
                    expected.messages.len(), expected.operations.len());
            }
        }
        self.rules.assert_graph();
    }

    fn command_load(&self, indices: &[usize]) -> Vec<(AbortOnDrop, Arc<AtomicU64>)> {
        indices
            .iter()
            .map(|index| {
                let actor = self.online(*index).actor.clone();
                let reads = Arc::new(AtomicU64::new(0));
                let seen = reads.clone();
                let task = AbortOnDrop(tokio::spawn(async move {
                    loop {
                        actor.messages(channel_id("general")).await;
                        seen.fetch_add(1, Ordering::Release);
                        SystemClock.sleep(Duration::from_millis(5)).await;
                    }
                }));
                (task, reads)
            })
            .collect()
    }
}

async fn six_client_scenario(hostile: bool, admission: FixtureAdmission) {
    let mut scenario = Scenario::new(hostile, admission);
    scenario.admit().await;
    scenario.rules.drop_gossip.store(hostile, Ordering::Release);
    let mut common = History::new();
    scenario
        .author(ALL, "initial common chat", &mut common)
        .await;
    scenario
        .converge(ALL, &common, "initial six-client conversation")
        .await;
    scenario.close(ALL).await;
    scenario.reopen(ALL).await;
    scenario.converge(ALL, &common, "first native reopen").await;
    scenario
        .author(ALL, "conversation after everybody restarts", &mut common)
        .await;
    scenario
        .converge(ALL, &common, "post-restart six-client conversation")
        .await;
    scenario.close(&[3, 4, 5]).await;

    let mut first_partition = common.clone();
    scenario
        .author(
            &[0, 1, 2],
            "clients 1-2-3 bridge through 1",
            &mut first_partition,
        )
        .await;
    scenario
        .converge(&[0, 1, 2], &first_partition, "only clients 1-2-3")
        .await;
    scenario.close(&[0, 1, 2]).await;

    scenario.reopen(&[3]).await;
    let mut second_partition = common.clone();
    let solo_ids = scenario
        .author(&[3], "client 4 completely alone", &mut second_partition)
        .await;
    scenario
        .converge(&[3], &second_partition, "offline native acceptance on 4")
        .await;
    scenario
        .online(3)
        .assert_no_remote_delivery(&solo_ids)
        .await;
    scenario.close(&[3]).await;

    scenario.reopen(&[4, 5]).await;
    let mut fifth = common.clone();
    let mut sixth = common.clone();
    let fifth_ids = scenario.author(&[4], "isolated client 5", &mut fifth).await;
    let sixth_ids = scenario.author(&[5], "isolated client 6", &mut sixth).await;
    let starting_passes = [
        scenario.online(4).passes.load(Ordering::Acquire),
        scenario.online(5).passes.load(Ordering::Acquire),
    ];
    // Observe a full periodic pass, not only the startup wake, while the forbidden pair is alone.
    periodic_bounded("isolated discovery passes", async {
        while (4..6).any(|index| {
            scenario.online(index).passes.load(Ordering::Acquire)
                < (starting_passes[index - 4] + 1).max(2)
        }) {
            SystemClock.sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert_eq!(
        scenario.online(4).history().await,
        fifth,
        "5 cannot receive 6, 4's solo send, or the 1-2-3 batch"
    );
    assert_eq!(
        scenario.online(5).history().await,
        sixth,
        "6 cannot receive 5, 4's solo send, or the 1-2-3 batch"
    );
    scenario
        .online(4)
        .assert_no_remote_delivery(&fifth_ids)
        .await;
    scenario
        .online(5)
        .assert_no_remote_delivery(&sixth_ids)
        .await;
    assert!(scenario
        .online(4)
        .mesh
        .authenticated_dial_routes()
        .is_empty());
    assert!(scenario
        .online(5)
        .mesh
        .authenticated_dial_routes()
        .is_empty());

    scenario
        .rules
        .delay_replies
        .store(hostile, Ordering::Release);
    let mut bridge_load = if hostile {
        scenario.command_load(&[4, 5])
    } else {
        Vec::new()
    };
    scenario.reopen(&[3]).await;
    second_partition.extend(fifth);
    second_partition.extend(sixth);
    if hostile {
        bridge_load.extend(scenario.command_load(&[3]));
    }
    scenario
        .converge(
            &[3, 4, 5],
            &second_partition,
            "4 bridges 5 and 6 with exact original identities",
        )
        .await;
    assert!(bridge_load
        .iter()
        .all(|(_, reads)| reads.load(Ordering::Acquire) > 0));
    drop(bridge_load); // The exact-union assertion happens before granting any idle window.
    scenario.reopen(&[0, 1, 2]).await;
    let mut full = first_partition;
    full.extend(second_partition);
    let reunion_load = if hostile {
        scenario.command_load(ALL)
    } else {
        Vec::new()
    };
    scenario
        .converge(
            ALL,
            &full,
            "all six rejoin and exchange both partition histories",
        )
        .await;
    assert!(reunion_load
        .iter()
        .all(|(_, reads)| reads.load(Ordering::Acquire) > 0));
    drop(reunion_load);
    scenario.close(ALL).await;
    for index in ALL {
        scenario.reopen(&[*index]).await;
        assert_eq!(
            scenario.online(*index).history().await,
            full,
            "client {} must have independently sealed the full union",
            index + 1
        );
        scenario.close(&[*index]).await;
    }
    scenario.rules.assert_graph();
    let trace = scenario.rules.trace.lock().unwrap();
    assert!(!trace.completed_requests.is_empty() && !trace.received_requests.is_empty());
    if hostile {
        assert!(
            trace.dropped_gossip > 0,
            "the adversarial variant must actually discard gossip"
        );
        assert!(
            trace.delayed_replies > 0,
            "command load must overlap real delayed replies"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn six_client_native_restart_and_partition_recovery() {
    let admission = isolate().await;
    scenario_bounded(
        six_client_scenario(false, admission),
        SystemClock.sleep(Duration::from_secs(600)),
        "six-client baseline exceeded its total bound",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn six_client_native_reversed_reopen_gossip_loss_and_command_load() {
    let admission = isolate().await;
    scenario_bounded(
        six_client_scenario(true, admission),
        SystemClock.sleep(Duration::from_secs(600)),
        "six-client hostile variant exceeded its total bound",
    )
    .await;
}
