//! Two `MeshService` nodes over real libp2p (memory transport), exercising the
//! `MeshTransport` seam: gossip fan-out and addressed request/response.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use catcoms_net::MeshService;
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
