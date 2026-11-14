//! Two-process compatibility probe, built unchanged against both dependency
//! stacks by check-network-security.py. This uses only public production APIs.
//! Fixed seeds and loopback-only QUIC are test inputs, never shipping identities.
use bytes::Bytes;
use catcoms_net::{keypair_from_seed, seeded_transport_identity_data, MeshService};
use catcoms_rt::{
    Clock, ConnectionTransport, MeshTransport, ProtocolId, SystemClock, Topic, TransportError,
    TransportEvent,
};
use std::{future::Future, io::Write, sync::Arc, time::Duration};

const PROTOCOL: ProtocolId = ProtocolId("/catcoms/rr/1");
const TOPICS: usize = 2_000;
const PAYLOAD: usize = 60 * 1_024;

async fn bounded<T>(label: &str, future: impl Future<Output = T>) -> T {
    tokio::select! {
        result = future => result,
        _ = SystemClock.sleep(Duration::from_secs(15)) => panic!("network compatibility timeout: {label}"),
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
    let arguments: Vec<_> = std::env::args().skip(1).collect();
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
    // Queue all subscriptions before any dial. Their actual hex-encoded initial
    // announcement exceeds 64KiB; shorter raw-topic fixtures miss that boundary.
    for index in 0..TOPICS {
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
    let publisher = tokio::spawn(async move {
        // Isolate protocol negotiation before bulk traffic. This probe proves
        // both functional protocols; sustained-load fairness is separate
        // acceptance and must not be inferred from these exchanges.
        publication_wait.notified().await;
        for serial in 0u32..1_500 {
            let mut payload = vec![own; PAYLOAD];
            payload[PAYLOAD - 4..].copy_from_slice(&serial.to_be_bytes());
            publish_mesh
                .publish(publish_topic.clone(), payload.into())
                .await
                .unwrap();
            SystemClock.sleep(Duration::from_millis(20)).await;
        }
        panic!("finite gossip probe attempts exhausted");
    });
    {
        let reply = mesh.request_connected(peer, PROTOCOL, Bytes::from(vec![19, own, 1]));
        tokio::pin!(reply);
        let mut received_request = false;
        let mut received_gossip = false;
        let mut received_reply = false;
        bounded("bidirectional RR and signed near-limit gossip", async {
            loop {
                tokio::select! {
                    result = &mut reply, if !received_reply => {
                        assert_eq!(result.unwrap().as_ref(), &[19, remote, 2]);
                        received_reply = true;
                        eprintln!("{role}: exact outbound RR reply received");
                    }
                    event = mesh.next_event() => match event.expect("live transport event") {
                        TransportEvent::Request { from, proto, data, responder } => {
                            assert_eq!(from, peer);
                            assert_eq!(proto, PROTOCOL);
                            assert_eq!(data.as_ref(), &[19, remote, 1]);
                            responder.try_respond(Bytes::from(vec![19, own, 2])).unwrap();
                            received_request = true;
                        eprintln!("{role}: exact inbound RR request served");
                        }
                        TransportEvent::Gossip { topic: actual, from, data } => {
                            assert_eq!(from, peer);
                            assert_eq!(actual, topic);
                            assert_eq!(data.len(), PAYLOAD);
                            assert!(data[..PAYLOAD-4].iter().all(|byte| *byte == remote));
                        assert!(u32::from_be_bytes(data[PAYLOAD-4..].try_into().unwrap()) < 1_500);
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
                if received_reply && received_request && received_gossip {
                    break;
                }
            }
        })
        .await;
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
    marker(&format!("COMPLETE {role}"));
}
