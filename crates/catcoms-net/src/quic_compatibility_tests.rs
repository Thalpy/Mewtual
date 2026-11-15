//! Functional compatibility of the production QUIC transport after security
//! patch updates. This does not reproduce the upstream vulnerabilities or prove
//! physical-network acceptance. No TCP listener, router mapping or remote host
//! can hide a broken UDP/TLS/multiplexed request path in this fixture.
use super::*;

#[tokio::test]
async fn actual_udp_quic_refuses_a_listener_with_the_wrong_expected_peer() {
    use futures::StreamExt;
    use libp2p::swarm::{dial_opts::DialOpts, DialError, SwarmEvent};
    let server_key = keypair_from_seed([0x65; 32]).unwrap();
    let client_key = keypair_from_seed([0x66; 32]).unwrap();
    let actual = server_key.public().to_peer_id();
    let foreign = keypair_from_seed([0x67; 32]).unwrap().public().to_peer_id();
    assert_ne!(actual, foreign);
    // Use the ACTUAL product Swarm builder: raw QUIC authenticates and returns
    // the certificate's peer, while Swarm enforces the expected-peer binding.
    let mut server = build_tcp_swarm_with_key(server_key).unwrap();
    let mut client = build_tcp_swarm_with_key(client_key).unwrap();
    server
        .listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap())
        .unwrap();
    let address = bounded("wrong-peer UDP listener", async {
        loop {
            if let SwarmEvent::NewListenAddr { address, .. } = server.select_next_some().await {
                break address;
            }
        }
    })
    .await;
    client
        .dial(
            DialOpts::peer_id(foreign)
                .addresses(vec![address.clone()])
                .build(),
        )
        .unwrap();
    bounded("actual Swarm wrong-peer rejection",async {
        loop {
            tokio::select! {
                event=client.select_next_some()=>match event {
                    SwarmEvent::OutgoingConnectionError {peer_id,error,..}=>{
                        assert_eq!(peer_id,Some(foreign));
                        assert!(matches!(&error,DialError::WrongPeerId {obtained,..} if obtained==&actual),"unexpected dial error: {error}");
                        break;
                    }
                    SwarmEvent::ConnectionEstablished {peer_id,..}=>panic!("wrong-peer dial adopted {peer_id}"),
                    _=>{}
                },
                _=server.select_next_some()=>{}
            }
        }
    }).await;
    assert_eq!(client.connected_peers().count(), 0);
    // Same listener and original swarms: refusal must not strand future valid
    // work. The correct expected peer then authenticates over the UDP endpoint.
    client
        .dial(DialOpts::peer_id(actual).addresses(vec![address]).build())
        .unwrap();
    bounded("same endpoint with the actual expected peer",async {
        loop {
            tokio::select! {
                event=client.select_next_some()=>match event {
                    SwarmEvent::ConnectionEstablished {peer_id,..}=>{assert_eq!(peer_id,actual);break;},
                    SwarmEvent::OutgoingConnectionError {error,..}=>panic!("correct peer refused: {error}"),
                    _=>{}
                },
                _=server.select_next_some()=>{}
            }
        }
    }).await;
    assert!(client.is_connected(&actual));
    assert!(!client.is_connected(&foreign));
}

async fn bounded<F: Future>(label: &str, future: F) -> F::Output {
    tokio::select! {
        result = future => result,
        _ = SystemClock.sleep(Duration::from_secs(5)) => panic!("QUIC compatibility phase timed out: {label}"),
    }
}

async fn exchange(
    sender: &MeshService,
    receiver: &MeshService,
    request: &'static [u8],
    reply: &'static [u8],
) {
    let serve = async {
        loop {
            match receiver.next_event().await {
                Some(TransportEvent::Request {
                    from,
                    proto,
                    data,
                    responder,
                }) => {
                    assert_eq!(from, sender.local_peer());
                    assert_eq!(proto, ProtocolId(RR_PROTOCOL));
                    assert_eq!(data.as_ref(), request);
                    responder.try_respond(Bytes::from_static(reply)).unwrap();
                    return;
                }
                Some(_) => {}
                None => panic!("receiver stopped before the authenticated request"),
            }
        }
    };
    let (response, ()) = bounded("bidirectional request/reply", async {
        futures::join!(
            sender.request_connected(
                receiver.local_peer(),
                ProtocolId(RR_PROTOCOL),
                Bytes::from_static(request)
            ),
            serve
        )
    })
    .await;
    assert_eq!(response.unwrap().as_ref(), reply);
}

#[tokio::test]
async fn actual_udp_quic_authenticates_bidirectional_requests_and_withdraws_closed_paths() {
    // Caller-supplied deterministic test identities exercise the same production
    // builder. The library constructor deliberately keeps router mapping off.
    let (server, server_id, _) = MeshService::new_tcp_with_key(
        keypair_from_seed([0x61; 32]).unwrap(),
        &["/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()],
        &[],
    )
    .unwrap();
    let (client, _, _) =
        MeshService::new_tcp_with_key(keypair_from_seed([0x62; 32]).unwrap(), &[], &[]).unwrap();
    let listen = bounded("UDP listener", server.next_listen_addr())
        .await
        .unwrap();
    assert!(listen
        .iter()
        .any(|part| matches!(part, libp2p::multiaddr::Protocol::QuicV1)));
    assert!(!listen
        .iter()
        .any(|part| matches!(part, libp2p::multiaddr::Protocol::Tcp(_))));
    bounded(
        "peer-bound UDP dial",
        client.dial(format!("{listen}/p2p/{server_id}").parse().unwrap()),
    )
    .await
    .unwrap();
    let server_peer = server.local_peer();
    assert_eq!(server_peer, phase0_peer_id(&server_id));
    for (node, peer) in [(&client, server_peer), (&server, client.local_peer())] {
        let connected = bounded(
            "authenticated QUIC connection",
            node.wait_for_peer_connected(peer),
        )
        .await
        .unwrap();
        assert_eq!(connected.peer, peer);
        assert!(!connected.active.is_empty());
        assert!(connected
            .active
            .iter()
            .all(|path| path.transport == ConnectionTransport::QuicV1));
    }
    // These are bounded ordinary transport payloads, not forged MLS/chat grants.
    exchange(&client, &server, &[19, 0x41], &[19, 0x51]).await;
    exchange(&server, &client, &[19, 0x42], &[19, 0x52]).await;

    let mut observations = client.connection_snapshot_rx.clone();
    drop(server);
    // Observe remote-path withdrawal. MeshService Drop is not a joined actor-
    // drain or physical-resource proof, and this test must not claim either.
    bounded("closed remote path withdrawal", async {
        loop {
            let current = current_connection_snapshot(&mut observations).unwrap();
            if !current.iter().any(|row| row.peer == server_peer) {
                break;
            }
            observations.changed().await.unwrap();
        }
    })
    .await;
    assert!(matches!(
        bounded(
            "no stale connected request",
            client.request_connected(
                server_peer,
                ProtocolId(RR_PROTOCOL),
                Bytes::from_static(&[19, 0x43]),
            )
        )
        .await,
        Err(TransportError::Unreachable(peer)) if peer == server_peer
    ));
}
