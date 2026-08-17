//! Two `MeshService` nodes over real libp2p (memory transport), exercising the
//! `MeshTransport` seam: gossip fan-out and addressed request/response.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use catcoms_net::{
    build_memory_relay_swarm, build_memory_swarm, build_relay_swarm, phase0_peer_id, run_relay,
    MeshService,
};
use catcoms_rt::{MeshTransport, ProtocolId, Topic, TransportEvent};
use libp2p::Multiaddr;

#[tokio::test]
async fn two_mesh_nodes_gossip_and_request_response() {
    let addr: Multiaddr = "/memory/770077".parse().unwrap();
    let a = Arc::new(MeshService::new_memory(Some(addr.clone()), &[]).unwrap());
    let b = Arc::new(MeshService::new_memory(None, std::slice::from_ref(&addr)).unwrap());

    let topic = Topic::new("catcoms/chan/1");
    a.subscribe(topic.clone()).await.unwrap();
    b.subscribe(topic.clone()).await.unwrap();

    // A publishes once; the actor queues it until it learns B is subscribed.
    a.publish(topic.clone(), Bytes::from_static(b"hello mesh"))
        .await
        .unwrap();

    // A answers every request with "pong".
    let a_responder = {
        let a = Arc::clone(&a);
        tokio::spawn(async move {
            while let Some(event) = a.next_event().await {
                if let TransportEvent::Request {
                    data, responder, ..
                } = event
                {
                    assert_eq!(&data[..], b"ping");
                    responder.respond(Bytes::from_static(b"pong"));
                }
            }
        })
    };

    // Drive B: on connect, send a request; collect the gossip message too.
    let drive = async {
        let mut got_gossip = false;
        let mut got_pong = false;
        let mut requested = false;
        while let Some(event) = b.next_event().await {
            match event {
                TransportEvent::PeerConnected(peer) if !requested => {
                    requested = true;
                    let reply = b
                        .request(
                            peer,
                            ProtocolId("/catcoms/rr/1"),
                            Bytes::from_static(b"ping"),
                        )
                        .await
                        .unwrap();
                    assert_eq!(&reply[..], b"pong");
                    got_pong = true;
                }
                TransportEvent::Gossip {
                    data,
                    from,
                    topic: t,
                } => {
                    assert_eq!(&data[..], b"hello mesh");
                    assert_eq!(t, topic);
                    assert_eq!(from, a.local_peer());
                    got_gossip = true;
                }
                _ => {}
            }
            if got_gossip && got_pong {
                break;
            }
        }
        (got_gossip, got_pong)
    };

    let (got_gossip, got_pong) = tokio::time::timeout(Duration::from_secs(20), drive)
        .await
        .expect("mesh test timed out");
    assert!(got_gossip, "gossip message not received");
    assert!(got_pong, "request/response did not complete");

    a_responder.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_tcp_nodes_connect_and_request_response() {
    // Real OS sockets: `new_tcp` listens on an ephemeral loopback port; the bound
    // address is discovered via `next_listen_addr`, then dialed by a second node.
    let (a, _a_id) =
        MeshService::new_tcp(Some("/ip4/127.0.0.1/tcp/0".parse().unwrap()), &[]).unwrap();
    let a = Arc::new(a);
    let a_addr = tokio::time::timeout(Duration::from_secs(10), a.next_listen_addr())
        .await
        .expect("listen-addr timeout")
        .expect("a bound a listen address");

    let (b, _b_id) = MeshService::new_tcp(None, std::slice::from_ref(&a_addr)).unwrap();

    // A answers every request with "pong".
    let a_responder = {
        let a = Arc::clone(&a);
        tokio::spawn(async move {
            while let Some(event) = a.next_event().await {
                if let TransportEvent::Request {
                    data, responder, ..
                } = event
                {
                    assert_eq!(&data[..], b"ping");
                    responder.respond(Bytes::from_static(b"pong"));
                }
            }
        })
    };

    // B: on connect, send a request over the TCP connection.
    let reply = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(TransportEvent::PeerConnected(peer)) = b.next_event().await {
                return b
                    .request(
                        peer,
                        ProtocolId("/catcoms/rr/1"),
                        Bytes::from_static(b"ping"),
                    )
                    .await
                    .unwrap();
            }
        }
    })
    .await
    .expect("tcp request/response timed out");

    assert_eq!(&reply[..], b"pong");
    a_responder.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_reserves_a_circuit_slot_on_a_relay() {
    // A relay-server node forwards circuit traffic for clients behind NAT.
    let relay_addr: Multiaddr = "/memory/999001".parse().unwrap();
    let mut relay_swarm = build_memory_relay_swarm();
    let relay_id = *relay_swarm.local_peer_id();
    relay_swarm.listen_on(relay_addr.clone()).unwrap();
    let relay_task = tokio::spawn(run_relay(relay_swarm));

    // A client connects to the relay, then reserves a slot by listening on its
    // circuit address.
    let client = MeshService::spawn(build_memory_swarm());
    client.dial(relay_addr.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(TransportEvent::PeerConnected(_)) = client.next_event().await {
                break;
            }
        }
    })
    .await
    .expect("client did not connect to the relay");

    let circuit: Multiaddr = format!("{relay_addr}/p2p/{relay_id}/p2p-circuit")
        .parse()
        .unwrap();
    client.listen_on(circuit).await.unwrap();

    // The granted reservation surfaces as a `…/p2p-circuit` listen address.
    let got = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match client.next_listen_addr().await {
                Some(addr) if addr.to_string().contains("p2p-circuit") => return addr,
                Some(_) => continue,
                None => panic!("actor stopped"),
            }
        }
    })
    .await
    .expect("relay reservation was not granted");
    assert!(got.to_string().contains("p2p-circuit"));

    relay_task.abort();
}

/// 6e-3c: **DCUtR hole-punch.** A joiner reaches a server *through a relay*, then
/// DCUtR upgrades that relayed link to a **direct** connection. Real NAT can't be
/// exercised in-process, so this uses TCP loopback (all three nodes are directly
/// dialable) and asserts the *upgrade event path* fires; the same mechanism that,
/// behind real NATs, moves traffic off the relay once the hole is punched. Mirrors
/// `libp2p-dcutr`'s own `connect` test, but driven through `MeshService` and the
/// `next_direct_upgrade()` surface. Loopback hole-punching works because `identify`
/// translates the relay-observed address into each node's real listen port.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relayed_connection_upgrades_to_direct_via_dcutr() {
    use futures::StreamExt as _;
    use libp2p::swarm::SwarmEvent;

    // --- Relay (TCP loopback). Capture its bound address, then run it. ---
    let mut relay_swarm = build_relay_swarm().unwrap();
    let relay_id = *relay_swarm.local_peer_id();
    relay_swarm
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .unwrap();
    let relay_addr: Multiaddr = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let SwarmEvent::NewListenAddr { address, .. } = relay_swarm.select_next_some().await
            {
                relay_swarm.add_external_address(address.clone());
                return address;
            }
        }
    })
    .await
    .expect("relay did not bind a listen address");
    let relay_task = tokio::spawn(run_relay(relay_swarm));

    // --- Server: listens directly (the eventual direct link) and reserves a circuit
    //     slot on the relay. ---
    let (server, server_id) =
        MeshService::new_tcp(Some("/ip4/127.0.0.1/tcp/0".parse().unwrap()), &[]).unwrap();
    server.dial(relay_addr.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(TransportEvent::PeerConnected(_)) = server.next_event().await {
                break;
            }
        }
    })
    .await
    .expect("server did not connect to the relay");

    let circuit: Multiaddr = format!("{relay_addr}/p2p/{relay_id}/p2p-circuit")
        .parse()
        .unwrap();
    server.listen_on(circuit).await.unwrap();
    let server_circuit_addr = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match server.next_listen_addr().await {
                Some(a) if a.to_string().contains("p2p-circuit") => return a,
                Some(_) => continue,
                None => panic!("server actor stopped"),
            }
        }
    })
    .await
    .expect("server's circuit reservation was not granted");

    // --- Joiner: listens directly (so the server can dial it back during the
    //     simultaneous-open) and dials the server's circuit address (via the relay). ---
    let (joiner, _joiner_id) = MeshService::new_tcp(
        Some("/ip4/127.0.0.1/tcp/0".parse().unwrap()),
        std::slice::from_ref(&server_circuit_addr),
    )
    .unwrap();

    // The relayed connection forms first; DCUtR then upgrades it to a direct one,
    // surfaced (on the dialer) via `next_direct_upgrade`.
    let upgraded = tokio::time::timeout(Duration::from_secs(40), joiner.next_direct_upgrade())
        .await
        .expect("DCUtR did not upgrade the relayed connection within the timeout")
        .expect("joiner actor stopped");
    assert_eq!(
        upgraded,
        phase0_peer_id(&server_id),
        "the upgraded peer should be the server"
    );

    // Keep the server handle alive until the assertion (dropping it stops its actor).
    drop(server);
    relay_task.abort();
}
