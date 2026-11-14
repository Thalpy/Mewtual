//! Two-process compatibility probe, built unchanged against both dependency
//! stacks by check-network-security.py. This uses only public production APIs.
//! Fixed seeds and loopback-only QUIC are test inputs, never shipping identities.
use bytes::Bytes;
use catcoms_net::{keypair_from_seed, seeded_transport_identity_data, MeshService};
use catcoms_rt::{
    Clock, ConnectionTransport, MeshTransport, ProtocolId, SystemClock, Topic, TransportError,
    TransportEvent,
};
use std::{
    future::Future,
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

// Example-only bounded observations from BOTH unmodified dependency stacks.
// Never emit per-event tracing or format subscription arrays/payloads/keys.
static COUNTERS: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];
struct Diagnostics {
    target: String,
}
struct FixedText {
    bytes: [u8; 160],
    len: usize,
}
impl Default for FixedText {
    fn default() -> Self {
        Self {
            bytes: [0; 160],
            len: 0,
        }
    }
}
impl std::fmt::Write for FixedText {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        if value.len() > self.bytes.len() - self.len {
            return Err(std::fmt::Error);
        }
        self.bytes[self.len..self.len + value.len()].copy_from_slice(value.as_bytes());
        self.len += value.len();
        Ok(())
    }
}
impl FixedText {
    fn text(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap()
    }
}
#[derive(Default)]
struct Fields {
    message: FixedText,
    topic: FixedText,
    error: FixedText,
    held: u64,
    bytes: u64,
}
impl tracing::field::Visit for Fields {
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "held" => self.held = value,
            "held_bytes" => self.bytes = value,
            _ => (),
        }
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        let output = match field.name() {
            "message" => &mut self.message,
            "topic" => &mut self.topic,
            "error" => &mut self.error,
            _ => return,
        };
        let _ = write!(output, "{value:?}");
    }
}
// Implement the small core Subscriber directly: the pinned old Net manifest
// has tracing but no tracing-subscriber. Its manifest and lock stay unchanged.
impl tracing::Subscriber for Diagnostics {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        let fields = metadata.fields();
        (metadata.target().starts_with("libp2p_gossipsub")
            && ((fields.field("peer").is_some() && fields.field("topic").is_some())
                || fields.field("message_id").is_some()))
            || (metadata.target() == "catcoms_net"
                && (fields.field("bytes").is_some() || fields.field("held").is_some())
                && fields.field("data").is_none())
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        // No span context is observed or exported by this event-only probe.
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let metadata = event.metadata();
        let fields = metadata.fields();
        // In both stacks the selected subscription event has peer+topic; the
        // post-sign event has message_id. Exclude source/subscriptions/rpc/data
        // callsites BEFORE Visit, so embedded giant Debug messages are not run.
        let gossip = metadata.target().starts_with("libp2p_gossipsub")
            && ((fields.field("peer").is_some() && fields.field("topic").is_some())
                || fields.field("message_id").is_some());
        let net = metadata.target() == "catcoms_net"
            && (fields.field("bytes").is_some() || fields.field("held").is_some())
            && fields.field("data").is_none();
        if !gossip && !net {
            return;
        }
        let mut observed = Fields::default();
        event.record(&mut observed);
        let message = observed.message.text();
        let increment = |i: usize| {
            COUNTERS[i].fetch_add(1, Ordering::Relaxed);
        };
        match message {
            "SUBSCRIPTION: Adding gossip peer to topic" => {
                increment(0);
                if observed.topic.text() == self.target {
                    COUNTERS[1].store(1, Ordering::Relaxed);
                }
            }
            "Publishing message" => increment(2),
            "published" => increment(3),
            "publish held for retry" => {
                if observed.error.text().contains("NoPeers")
                    || observed.error.text().contains("subscribed")
                {
                    increment(5);
                }
                if observed.error.text().contains("AllQueuesFull")
                    || observed.error.text().contains("queues")
                {
                    increment(6);
                }
                COUNTERS[7].store(observed.bytes, Ordering::Relaxed);
                // Publish the completed tuple last: the main task uses this
                // count as its pressure barrier across two Tokio workers.
                COUNTERS[4].fetch_add(1, Ordering::Release);
            }
            "gossip received" => increment(8),
            _ => (),
        }
    }
}
fn diagnostic_summary(label: &str) {
    let read = |i: usize| COUNTERS[i].load(Ordering::Relaxed);
    eprintln!("probe observation {label}: adopted={} target={} signed={} published={} first_holds={} first_no_peers={} first_queues_full={} last_first_hold_bytes={} gossip_received={}", read(0),read(1),read(2),read(3),read(4),read(5),read(6),read(7),read(8));
}

const PROTOCOL: ProtocolId = ProtocolId("/catcoms/rr/1");
const TOPICS: usize = 2_000;
const PAYLOAD: usize = 60 * 1_024;

async fn bounded<T>(label: &str, future: impl Future<Output = T>) -> T {
    tokio::select! {
        result = future => result,
        _ = SystemClock.sleep(Duration::from_secs(15)) => {
            diagnostic_summary(label);
            panic!("network compatibility timeout: {label}")
        },
    }
}

fn marker(value: &str) {
    println!("{value}");
    std::io::stdout().flush().unwrap();
}

async fn command(expected: &'static str) {
    // The Python owner bounds and reaps the whole process on every failure.
    // Keep the runtime alive while waiting so Mesh Drop can actually retire the
    // actor before the other process's negative connected-request observation.
    tokio::task::spawn_blocking(move || {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        assert_eq!(line.trim_end(), expected);
    })
    .await
    .unwrap();
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    tracing::subscriber::set_global_default(Diagnostics {
        target: hex::encode(format!("migration-topic-{:08}", TOPICS - 1)),
    })
    .unwrap();
    let mut arguments: Vec<_> = std::env::args().skip(1).collect();
    // Closed controller-selected scenarios, identical in both source builds.
    let scenario = arguments.pop().expect("explicit probe scenario");
    let pressure = scenario == "--pressure";
    let phased = scenario == "--wait-target";
    assert!(pressure || phased);
    let server = arguments.as_slice() == ["server"];
    assert!(server || (arguments.len() == 2 && arguments[0] == "client"));
    let role = if server { "server" } else { "client" };
    let own = if server { 0x73 } else { 0x74 };
    let remote = if server { 0x74 } else { 0x73 };
    let expected = seeded_transport_identity_data(&[remote; 32]).unwrap();
    let listeners = if server {
        vec!["/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()]
    } else {
        Vec::new()
    };
    let (mesh, id, _) =
        MeshService::new_tcp_with_key(keypair_from_seed([own; 32]).unwrap(), &listeners, &[])
            .unwrap();
    let mesh = Arc::new(mesh);
    assert_eq!(
        id.to_string(),
        seeded_transport_identity_data(&[own; 32])
            .unwrap()
            .peer_id_data()
    );
    // Wire compatibility queues all topics before dial; their real hex hello
    // exceeds64KiB. The pressure server deliberately withholds all topics until
    // the client has proved128 originals held and completed an independent RR.
    for index in 0..if server && pressure { 0 } else { TOPICS } {
        mesh.subscribe(Topic::new(format!("migration-topic-{index:08}")))
            .await
            .unwrap();
    }
    let topic = Topic::new(format!("migration-topic-{:08}", TOPICS - 1));
    if server {
        let address = bounded("old listener", mesh.next_listen_addr())
            .await
            .unwrap();
        marker(&format!("READY {id} {address}"));
    } else {
        // Accept only the exact fixed test peer on loopback UDP. Neither the
        // controller nor an untrusted stdout line can nominate an external dial.
        let prefix = "/ip4/127.0.0.1/udp/";
        let suffix = format!("/quic-v1/p2p/{}", expected.peer_id_data());
        let port = arguments[1]
            .strip_prefix(prefix)
            .and_then(|v| v.strip_suffix(&suffix))
            .and_then(|v| v.parse::<u16>().ok())
            .filter(|p| *p != 0)
            .expect("exact loopback QUIC test endpoint");
        mesh.dial(format!("{prefix}{port}{suffix}").parse().unwrap())
            .await
            .unwrap();
    }
    let peer = expected.phase0_peer_data();
    let connected = bounded(
        "authenticated connection",
        mesh.wait_for_peer_connected(peer),
    )
    .await
    .unwrap();
    assert_eq!(connected.peer, peer);
    assert!(!connected.active.is_empty());
    assert!(connected
        .active
        .iter()
        .all(|path| path.transport == ConnectionTransport::QuicV1));
    eprintln!("{role}: authenticated QUIC connection");

    // Keep sending after this endpoint's positive observation until the
    // controller has both observations. One side receiving first must not
    // prematurely stop the other side's probe. This actual owner is joined
    // before Mesh Drop, rather than keeping its command sender alive.
    let publish_mesh = mesh.clone();
    let publish_topic = topic.clone();
    let publication_start = Arc::new(tokio::sync::Notify::new());
    let publication_wait = publication_start.clone();
    let release_publication = Arc::new(tokio::sync::Notify::new());
    let publication_release = release_publication.clone();
    let publisher = tokio::spawn(async move {
        // Isolate protocol negotiation before bulk traffic. This probe proves
        // both functional protocols; sustained-load fairness is separate
        // acceptance and must not be inferred from these exchanges.
        publication_wait.notified().await;
        if pressure && server {
            publication_release.notified().await;
        }
        if phased {
            // The immutable predecessor's all-topic retry amplification is
            // tested separately from wire compatibility. Wait for its ACTUAL
            // target adoption, not a local subscribe ACK or a fixed delay.
            // The enclosing original 15-second exchange deadline still applies.
            while COUNTERS[1].load(Ordering::Relaxed) == 0 {
                SystemClock.sleep(Duration::from_millis(5)).await;
            }
        }
        let attempts = if pressure && !server { 128 } else { 1_500 };
        for serial in 0u32..attempts {
            let mut payload = vec![own; PAYLOAD];
            payload[PAYLOAD - 4..].copy_from_slice(&serial.to_be_bytes());
            publish_mesh
                .publish(publish_topic.clone(), payload.into())
                .await
                .unwrap();
            // In the current/current pressure scenario the client first fills
            // the REAL held queue while the server has no subscriptions at all.
            if !(pressure && !server && serial < 128) {
                SystemClock.sleep(Duration::from_millis(20)).await;
            }
        }
        if pressure && !server {
            // No post-release replacement can satisfy the pressure receipt.
            // Preserve the actual publisher owner until controller cancellation.
            std::future::pending::<()>().await;
        }
        panic!("finite gossip probe attempts exhausted");
    });
    {
        let reply = mesh.request_connected(peer, PROTOCOL, Bytes::from(vec![19, own, 1]));
        tokio::pin!(reply);
        let mut received_request = false;
        let mut received_gossip = false;
        let mut received_reply = false;
        let pressure_reply = mesh.request_connected(peer, PROTOCOL, Bytes::from(vec![19, own, 3]));
        tokio::pin!(pressure_reply);
        let mut pressure_answered = !pressure || server;
        let mut pressure_served = !pressure || !server;
        let original_receipt_ack =
            mesh.request_connected(peer, PROTOCOL, Bytes::from(vec![19, own, 5]));
        tokio::pin!(original_receipt_ack);
        let mut receipt_confirmed = !pressure;
        let subscription_mesh = mesh.clone();
        let subscribe_after_hold = async move {
            command("release-target").await;
            release_publication.notify_one();
            for index in 0..TOPICS {
                subscription_mesh
                    .subscribe(Topic::new(format!("migration-topic-{index:08}")))
                    .await
                    .unwrap();
            }
            drop(subscription_mesh);
        };
        tokio::pin!(subscribe_after_hold);
        let held_notification = async {
            while COUNTERS[4].load(Ordering::Acquire) < 128 {
                SystemClock.sleep(Duration::from_millis(5)).await;
            }
            assert_eq!(COUNTERS[4].load(Ordering::Acquire), 128);
            assert_eq!(COUNTERS[5].load(Ordering::Relaxed), 128);
            assert_eq!(COUNTERS[6].load(Ordering::Relaxed), 0);
            assert_eq!(COUNTERS[1].load(Ordering::Relaxed), 0);
            assert_eq!(COUNTERS[7].load(Ordering::Relaxed), 128 * PAYLOAD as u64);
            marker("HELD client 128");
        };
        tokio::pin!(held_notification);
        let mut subscribed = !pressure || !server;
        let mut notified = !pressure || server;
        bounded("bidirectional RR and signed near-limit gossip", async {
            loop {
                tokio::select! {
                    _ = &mut subscribe_after_hold, if !subscribed => { subscribed = true; }
                    _ = &mut held_notification, if !notified => { notified = true; }
                    result = &mut pressure_reply, if pressure && !server && notified && !pressure_answered => {
                        assert_eq!(result.unwrap().as_ref(), &[19, remote, 4]);
                        assert_eq!(COUNTERS[1].load(Ordering::Relaxed), 0, "subscriptions are still withheld");
                        pressure_answered = true;
                        marker("PRESSURE_RR client");
                    }
                    result = &mut original_receipt_ack, if pressure && server && received_gossip && !receipt_confirmed => {
                        assert_eq!(result.unwrap().as_ref(), &[19, remote, 6]);
                        receipt_confirmed = true;
                    }
                    result = &mut reply, if !received_reply => {
                        assert_eq!(result.unwrap().as_ref(), &[19, remote, 2]);
                        received_reply = true;
                        eprintln!("{role}: exact outbound RR reply received");
                    }
                    event = mesh.next_event() => match event.expect("live transport event") {
                        TransportEvent::Request { from, proto, data, responder } => {
                            assert_eq!(from, peer);
                            assert_eq!(proto, PROTOCOL);
                            let is_pressure = pressure && server && data.as_ref() == [19, remote, 3];
                            let is_receipt_ack = pressure && !server && data.as_ref() == [19, remote, 5];
                            if is_receipt_ack {
                                // The original actor can surface this causally
                                // later request only AFTER its synchronous
                                // held128 retry batch returned. A mere early
                                // server gossip or target flag cannot prove it.
                                receipt_confirmed = true;
                            } else if is_pressure {
                                assert!(!subscribed, "pressure RR is served before subscription release");
                                pressure_served = true;
                            } else {
                                assert_eq!(data.as_ref(), &[19, remote, 1]);
                                received_request = true;
                            }
                            responder.try_respond(Bytes::from(vec![19, own, if is_receipt_ack { 6 } else if is_pressure { 4 } else { 2 }])).unwrap();
                        eprintln!("{role}: exact inbound RR request served");
                        }
                        TransportEvent::Gossip { topic: actual, from, data } => {
                            assert_eq!(from, peer);
                            assert_eq!(actual, topic);
                            assert_eq!(data.len(), PAYLOAD);
                            assert!(data[..PAYLOAD-4].iter().all(|byte| *byte == remote));
                        let serial = u32::from_be_bytes(data[PAYLOAD-4..].try_into().unwrap());
                        assert!(serial < if pressure && server { 128 } else { 1_500 });
                        if !received_gossip {
                            eprintln!("{role}: exact near-limit gossip received");
                        }
                            received_gossip = true;
                        }
                        _ => {}
                    }
                }
                if received_reply && received_request {
                    publication_start.notify_one();
                }
                if received_reply && received_request && received_gossip && subscribed && notified && pressure_answered && pressure_served && receipt_confirmed {
                    break;
                }
            }
        })
        .await;
        if pressure && !server {
            // 128 first attempts +128 exact-topic retries + every full retry
            // possible within15s at the production2s cadence (including one
            // already-ready timer). Whole-queue retry per unrelated SUB breaks
            // this bound even on a fast host; no timeout-only oracle is used.
            let maximum = 128 * (2 + (15 / 2 + 1));
            let attempts = COUNTERS[2].load(Ordering::Relaxed);
            if !(256..=maximum).contains(&attempts) {
                diagnostic_summary("signing_bound_exceeded");
            }
            assert!(
                (256..=maximum).contains(&attempts),
                "pressure signing attempts {attempts} outside active oracle range 256..={maximum}"
            );
            assert_eq!(COUNTERS[4].load(Ordering::Acquire), 128);
        }
    }
    marker(&format!("EXCHANGED {role} {TOPICS} {PAYLOAD}"));
    if server {
        command("drop").await;
        publisher.abort();
        assert!(publisher.await.unwrap_err().is_cancelled());
        // The completed request future's lexical scope has also ended before
        // requiring the last actual command sender here.
        drop(Arc::try_unwrap(mesh).expect("publisher's actual Mesh owner retired"));
        marker("DROPPED server");
        command("exit").await;
    } else {
        // Establish a controlled before/after edge. An independently lost path
        // must not produce a queued withdrawal marker before old Mesh Drop.
        command("check-connected").await;
        assert!(mesh
            .connection_snapshot()
            .iter()
            .any(|row| row.peer == peer && !row.active.is_empty()));
        marker("PRESENT client");
        command("observe-withdrawal").await;
        bounded("actual remote path withdrawal", async {
            loop {
                if !mesh
                    .connection_snapshot()
                    .iter()
                    .any(|row| row.peer == peer)
                {
                    break;
                }
                // The snapshot is the oracle; yielding merely lets the actor
                // deliver its real closure, and cannot manufacture withdrawal.
                SystemClock.sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(matches!(
            bounded("withdrawn connected request", mesh.request_connected(peer, PROTOCOL, Bytes::from_static(&[19, 0, 3]))).await,
            Err(TransportError::Unreachable(actual)) if actual == peer
        ));
        marker("WITHDRAWN client");
        publisher.abort();
        assert!(publisher.await.unwrap_err().is_cancelled());
    }
    diagnostic_summary("completed");
    marker(&format!("COMPLETE {role}"));
}
