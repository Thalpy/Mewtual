//! Six actual TCP clients exercise the native offline/restart path without injected history.
//!
//! The blocked peer graph is a deterministic transport restriction, not a claim to emulate NAT.
//! The adapter runs the shared production cadence and native worker/send/close/restore seams;
//! platform interface sampling and the Tauri window/frontend are deliberately outside this test.
use super::*;
use catcoms_rt::ManualClock;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::collections::{BTreeMap, BTreeSet};

mod runtime;
mod transport;
use runtime::{mount, network, register, Running};
use transport::{forbidden, NetworkRules, Trace, Transport};

type History = BTreeMap<String, (String, String)>;
const ALL: &[usize] = &[0, 1, 2, 3, 4, 5];

async fn bounded<F: Future>(label: &str, future: F) -> F::Output {
    tokio::select! {
        result = future => result,
        _ = SystemClock.sleep(Duration::from_secs(30)) => panic!("six-client operation exceeded its bound: {label}"),
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
}

struct Scenario {
    clients: Vec<Client>,
    rules: Arc<NetworkRules>,
    clock: ManualClock,
    clock_step_ms: Arc<AtomicU64>,
    _ticker: Option<AbortOnDrop>,
    reverse: bool,
    token: u128,
}

impl Scenario {
    fn new(reverse: bool) -> Self {
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
            delay_replies: AtomicBool::new(false),
            trace: StdMutex::new(Trace::default()),
        });
        let clock = ManualClock::new(SystemClock.now_ms());
        let clock_step_ms = Arc::new(AtomicU64::new(100));
        let timer_clock = clock.clone();
        let timer_step = clock_step_ms.clone();
        let ticker = AbortOnDrop(tokio::spawn(async move {
            loop {
                SystemClock.sleep(Duration::from_millis(100)).await;
                timer_clock.advance_ms(timer_step.load(Ordering::Acquire));
            }
        }));
        Self {
            clients,
            rules,
            clock,
            clock_step_ms,
            _ticker: Some(ticker),
            reverse,
            token: 0,
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
        for index in 0..6 {
            let state = mount(self.clients[index].root.path()).await;
            let transport =
                Transport::new(index, &mut self.clients[index].net, self.rules.clone()).await;
            let mesh = transport.handle();
            self.clients[index].driver = Some(transport.driver_lifetime().await);
            let mut server = if index == 0 {
                Server::found(
                    transport,
                    MlsDevice::generate().unwrap(),
                    ChaCha20Rng::seed_from_u64(90),
                    Box::new(self.clock.clone()),
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
                        Box::new(self.clock.clone()),
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
                for index in ALL {
                    complete &= self.online(*index).actor.members().await.len() == 6;
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
        for source in 1..6 {
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
        self.clock_step_ms.store(500, Ordering::Release);
        self.wait_for_routes().await;
        for index in ALL {
            assert_eq!(
                self.online(*index).actor.group_mode().await.unwrap(),
                catcoms_app::GroupMode::PeerToPeer
            );
            assert!(
                self.online(*index)
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

    async fn wait_for_routes(&self) {
        let required = [(1, 0), (2, 0), (3, 0), (4, 3), (5, 3)];
        tokio::select! {
            _ = async {
                loop {
                    let mut complete = true;
                    for (source, target) in required {
                        let net = network(&self.online(source).state).await;
                        complete &= net.reconnect_policy == ReconnectPolicy::MemberMesh && net.reconnect_routes.iter().any(|route| route.peer_id == *self.rules.peers[target].as_bytes());
                    }
                    if complete { break; }
                    SystemClock.sleep(Duration::from_millis(100)).await;
                }
            } => {},
            _ = SystemClock.sleep(Duration::from_secs(90)) => {
                let mut actual = Vec::new();
                for index in ALL { actual.push((*index + 1, network(&self.online(*index).state).await)); }
                panic!("native workers did not retain authenticated bridge routes: {actual:?}");
            }
        }
    }

    async fn reopen(&mut self, indices: &[usize]) {
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
            let transport = Transport::new(index, &mut net, self.rules.clone()).await;
            let mesh = transport.handle();
            self.clients[index].driver = Some(transport.driver_lifetime().await);
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
                    Box::new(self.clock.clone()),
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
            assert_eq!(
                self.online(index).actor.members().await.len(),
                6,
                "membership is fixed across every restart"
            );
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
            let mut driver = self.clients[*index].driver.take().unwrap();
            bounded("TCP transport shutdown", async {
                while driver.changed().await.is_ok() {}
            })
            .await;
        }
    }

    async fn author(
        &mut self,
        indices: &[usize],
        phase: &str,
        history: &mut History,
    ) -> Vec<String> {
        let mut accepted_ids = Vec::with_capacity(indices.len());
        for index in indices {
            self.token += 1;
            let text = format!("{phase}: original client {}", index + 1);
            let id = self.online(*index).send(self.token, &text).await;
            let author = fingerprint(&self.clients[*index].identity.as_ref().unwrap().1);
            assert!(
                history.insert(id.clone(), (author, text)).is_none(),
                "native acceptance reused an ID"
            );
            accepted_ids.push(id);
        }
        accepted_ids
    }

    async fn converge(&self, indices: &[usize], expected: &History, phase: &str) {
        eprintln!("six-client phase: {phase}");
        tokio::select! {
            _ = async {
                loop {
                    let mut complete = true;
                    for index in indices { complete &= self.online(*index).history().await == *expected; }
                    if complete { break; }
                    SystemClock.sleep(Duration::from_millis(100)).await;
                }
            } => {},
            _ = SystemClock.sleep(Duration::from_secs(90)) => {
                let mut actual = Vec::new();
                for index in indices { actual.push((*index + 1, self.online(*index).history().await)); }
                panic!("{phase} failed exact ID/author/text convergence; expected={expected:?}; actual={actual:?}");
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

async fn six_client_scenario(hostile: bool) {
    let mut scenario = Scenario::new(hostile);
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
    bounded("isolated discovery passes", async {
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
    tokio::select! {
        _ = six_client_scenario(false) => {},
        _ = SystemClock.sleep(Duration::from_secs(600)) => panic!("six-client baseline exceeded its total bound"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn six_client_native_reversed_reopen_gossip_loss_and_command_load() {
    tokio::select! {
        _ = six_client_scenario(true) => {},
        _ = SystemClock.sleep(Duration::from_secs(600)) => panic!("six-client hostile variant exceeded its total bound"),
    }
}
