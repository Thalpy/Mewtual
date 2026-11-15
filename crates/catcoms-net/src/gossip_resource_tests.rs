//! Real negotiated TCP control traffic; no application signature or membership
//! grants are forged. A transport-authenticated stranger can reach this lower
//! gossip boundary, so signed chat payloads do not bound its retained state.
use super::*;
use futures::{AsyncRead, AsyncWrite, AsyncWriteExt, StreamExt};
use libp2p::swarm::SwarmEvent;
use std::io;

#[derive(Clone)]
struct ControlCodec;

/// An observed-refusal probe, unlike ControlCodec's intentionally parked
/// response. It writes the oversize frame only after the receiver actually
/// accepts the near-limit positive control on this SAME negotiated stream.
#[derive(Clone)]
struct RefusalCodec(Arc<tokio::sync::Notify>);
impl request_response::Codec for RefusalCodec {
    type Protocol = StreamProtocol;
    type Request = (Vec<u8>, Vec<u8>);
    type Response = ();
    async fn read_request<T>(&mut self, _: &StreamProtocol, _: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        std::future::pending().await
    }
    async fn read_response<T>(&mut self, _: &StreamProtocol, stream: &mut T) -> io::Result<()>
    where
        T: AsyncRead + Unpin + Send,
    {
        use futures::AsyncReadExt;
        let mut byte = [0];
        if stream.read(&mut byte).await? == 0 {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected response to gossip control",
            ))
        }
    }
    async fn write_request<T>(
        &mut self,
        _: &StreamProtocol,
        stream: &mut T,
        frames: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let (near, oversize) = frames;
        assert!(near.len() < GOSSIP_RPC_MAX_BYTES);
        assert!(oversize.len() > GOSSIP_RPC_MAX_BYTES);
        let mut prefix = Vec::new();
        varint(&mut prefix, near.len());
        stream.write_all(&prefix).await?;
        stream.write_all(&near).await?;
        stream.flush().await?;
        self.0.notified().await;
        prefix.clear();
        varint(&mut prefix, oversize.len());
        stream.write_all(&prefix).await?;
        stream.write_all(&oversize).await?;
        stream.flush().await
    }
    async fn write_response<T>(&mut self, _: &StreamProtocol, _: &mut T, _: ()) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "probe sends no response",
        ))
    }
}

#[tokio::test]
async fn valid_oversize_control_is_observably_refused_then_fresh_stream_progresses() {
    let phase = async {
        let mut receiver = build_tcp_swarm_with_key(keypair_from_seed([85; 32]).unwrap()).unwrap();
        let receiver_id = *receiver.local_peer_id();
        receiver
            .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
            .unwrap();
        let address = loop {
            if let SwarmEvent::NewListenAddr { address, .. } = receiver.select_next_some().await {
                break address;
            }
        };
        let gate = Arc::new(tokio::sync::Notify::new());
        let key = keypair_from_seed([86; 32]).unwrap();
        let bad_peer = key.public().to_peer_id();
        let mut bad = SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_tcp(
                libp2p::tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .unwrap()
            .with_behaviour(|_| {
                request_response::Behaviour::with_codec(
                    RefusalCodec(gate.clone()),
                    [(
                        StreamProtocol::new("/meshsub/1.1.0"),
                        request_response::ProtocolSupport::Full,
                    )],
                    request_response::Config::default(),
                )
            })
            .unwrap()
            .build();
        bad.dial(
            libp2p::swarm::dial_opts::DialOpts::peer_id(receiver_id)
                .addresses(vec![address.clone()])
                .build(),
        )
        .unwrap();
        let baseline = "near-limit-control-positive";
        let denied = "oversize-control-must-not-be-adopted";
        // Unknown nested field15 is legal protobuf and ignored by the Control
        // schema. Both complete RPCs also contain a valid subscription. Neither
        // malformed fields nor a forged length can masquerade as byte refusal.
        let frame = |sentinel: &str, padding: usize| {
            let mut rpc = subscriptions(&[(true, sentinel.into())]);
            let mut control = Vec::new();
            field(&mut control, 15, &vec![0; padding]);
            field(&mut rpc, 3, &control);
            rpc
        };
        let mut input = Some((
            frame(baseline, GOSSIP_RPC_MAX_BYTES - 1024),
            frame(denied, GOSSIP_RPC_MAX_BYTES),
        ));
        let mut request = None;
        let mut accepted_near = false;
        let mut refused = false;
        while !(accepted_near && refused) {
            tokio::select! {
                event=bad.select_next_some()=>match event {
                    SwarmEvent::ConnectionEstablished {peer_id,..} if peer_id==receiver_id && input.is_some()=>{
                        request=Some(bad.behaviour_mut().send_request(&receiver_id,input.take().unwrap()));
                    }
                    SwarmEvent::Behaviour(request_response::Event::Message {message:request_response::Message::Response {request_id,response:()},..})=>{
                        assert_eq!(Some(request_id),request);refused=true; // actual remote stream EOF
                    }
                    SwarmEvent::Behaviour(request_response::Event::OutboundFailure {request_id,error,..})=>{
                        assert_eq!(Some(request_id),request);
                        let closed = match &error {
                            request_response::OutboundFailure::ConnectionClosed => true,
                            request_response::OutboundFailure::Io(io) => matches!(io.kind(),io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted | io::ErrorKind::BrokenPipe),
                            _ => false,
                        };
                        assert!(closed,"not an observed remote stream end: {error}");
                        refused=true;
                    }
                    _=>{}
                },
                event=receiver.select_next_some()=>{
                    if let SwarmEvent::Behaviour(MeshBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed {peer_id,topic}))=event {
                        if peer_id==bad_peer {
                            assert_ne!(topic,gossipsub::IdentTopic::new(denied).hash());
                            if topic==gossipsub::IdentTopic::new(baseline).hash() {accepted_near=true;gate.notify_one();}
                        }
                    }
                }
            }
        }
        // A fresh valid stream must make actual progress on the SAME receiver,
        // not on a replacement peer that lost the refused decoder's state.
        let key = keypair_from_seed([87; 32]).unwrap();
        let good_peer = key.public().to_peer_id();
        let mut good = SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_tcp(
                libp2p::tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .unwrap()
            .with_behaviour(|_| {
                request_response::Behaviour::with_codec(
                    ControlCodec,
                    [(
                        StreamProtocol::new("/meshsub/1.1.0"),
                        request_response::ProtocolSupport::Full,
                    )],
                    request_response::Config::default(),
                )
            })
            .unwrap()
            .build();
        good.dial(
            libp2p::swarm::dial_opts::DialOpts::peer_id(receiver_id)
                .addresses(vec![address])
                .build(),
        )
        .unwrap();
        let receipt = "after-observed-oversize-refusal";
        let mut sent = false;
        loop {
            tokio::select! {
                event=good.select_next_some()=>{
                    if matches!(event,SwarmEvent::ConnectionEstablished {peer_id,..} if peer_id==receiver_id) && !sent {
                        good.behaviour_mut().send_request(&receiver_id,vec![subscriptions(&[(true,receipt.into())])]);sent=true;
                    }
                },
                event=receiver.select_next_some()=>{
                    if let SwarmEvent::Behaviour(MeshBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed {peer_id,topic}))=event {
                        assert!(!(peer_id==bad_peer && topic==gossipsub::IdentTopic::new(denied).hash()));
                        if peer_id==good_peer && topic==gossipsub::IdentTopic::new(receipt).hash() {break;}
                    }
                }
            }
        }
        assert!(sent);
    };
    tokio::select! {
        _=phase=>{},
        _=SystemClock.sleep(Duration::from_secs(10))=>panic!("observed byte-refusal/fresh-stream control exceeded watchdog")
    }
}

impl request_response::Codec for ControlCodec {
    type Protocol = StreamProtocol;
    type Request = Vec<Vec<u8>>;
    type Response = Vec<u8>;

    async fn read_request<T>(&mut self, _: &StreamProtocol, _: &mut T) -> io::Result<Vec<Vec<u8>>>
    where
        T: AsyncRead + Unpin + Send,
    {
        // Keep the peer's negotiated outbound gossip stream open. This test
        // sender deliberately emits controls, not application messages/replies.
        std::future::pending().await
    }
    async fn read_response<T>(&mut self, _: &StreamProtocol, _: &mut T) -> io::Result<Vec<u8>>
    where
        T: AsyncRead + Unpin + Send,
    {
        std::future::pending().await
    }
    async fn write_request<T>(
        &mut self,
        _: &StreamProtocol,
        stream: &mut T,
        frames: Vec<Vec<u8>>,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        for payload in frames {
            assert!(payload.len() < 65_536, "bounded control fixture frame");
            let mut prefix = Vec::new();
            varint(&mut prefix, payload.len());
            stream.write_all(&prefix).await?;
            stream.write_all(&payload).await?;
        }
        stream.flush().await
    }
    async fn write_response<T>(
        &mut self,
        _: &StreamProtocol,
        _: &mut T,
        _: Vec<u8>,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "control fixture has no response",
        ))
    }
}

fn varint(out: &mut Vec<u8>, mut value: usize) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn field(out: &mut Vec<u8>, number: u8, value: &[u8]) {
    out.push(number << 3 | 2);
    varint(out, value.len());
    out.extend_from_slice(value);
}
fn controls_with_receipt(receipt: &str) -> Vec<u8> {
    let mut subscription = vec![8, 1]; // SubOpts.subscribe=true.
    field(&mut subscription, 2, receipt.as_bytes());
    let mut control = Vec::new();
    for number in 0..256 {
        let mut graft = Vec::new();
        field(
            &mut graft,
            1,
            format!("unsolicited-graft-{number:04}").as_bytes(),
        );
        field(&mut control, 3, &graft);
    }
    let mut rpc = Vec::new();
    field(&mut rpc, 1, &subscription);
    // RPC.control is field3 (rpc.proto), independently of the nested
    // ControlMessage.graft field3. An unknown outer field would be ignored,
    // and the valid SUB alone would make a vacuous security test pass.
    field(&mut rpc, 3, &control);
    assert!(rpc.len() < 65_536, "one bounded ordinary gossip frame");
    rpc
}

async fn retained_after_receipt(
    frames: Vec<Vec<u8>>,
    receipt: &str,
) -> (Vec<gossipsub::TopicHash>, usize) {
    let receiver_key = keypair_from_seed([81; 32]).unwrap();
    let receiver_id = receiver_key.public().to_peer_id();
    let sender_key = keypair_from_seed([82; 32]).unwrap();
    let sender_id = sender_key.public().to_peer_id();
    let receipt_hash = gossipsub::IdentTopic::new(receipt).hash();
    let mut receiver = build_tcp_swarm_with_key(receiver_key).unwrap();
    let mut sender = SwarmBuilder::with_existing_identity(sender_key)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .unwrap()
        .with_behaviour(|_| {
            request_response::Behaviour::with_codec(
                ControlCodec,
                [(
                    StreamProtocol::new("/meshsub/1.1.0"),
                    request_response::ProtocolSupport::Full,
                )],
                request_response::Config::default(),
            )
        })
        .unwrap()
        .build();
    receiver
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .unwrap();
    // Real sockets require an outer harness bound; protocol/request limits and
    // the production swarm's mapping/privacy/refusal policy remain unchanged.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let address = loop {
            if let SwarmEvent::NewListenAddr { address, .. } = receiver.select_next_some().await { break address; }
        };
        sender.dial(libp2p::swarm::dial_opts::DialOpts::peer_id(receiver_id).addresses(vec![address]).build()).unwrap();
        let mut frames = Some(frames);
        let first_hash = gossipsub::IdentTopic::new("bounded-topic-0000").hash();
        let mut first_notifications = 0;
        let retained = loop {
            tokio::select! {
                event = sender.select_next_some() => {
                    if matches!(event, SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == receiver_id) && frames.is_some() {
                        sender.behaviour_mut().send_request(&receiver_id, frames.take().unwrap());
                    }
                }
                event = receiver.select_next_some() => {
                    if let SwarmEvent::Behaviour(MeshBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { peer_id, topic })) = event {
                        if peer_id == sender_id && topic == first_hash {
                            first_notifications += 1;
                        }
                        if peer_id == sender_id && topic == receipt_hash {
                            // The real remote subscription is a receipt proving
                            // this exact RPC was processed. Handler processing
                            // is synchronous before returning its queued event;
                            // inspect the resulting state, not just queueing.
                            let topics = receiver.behaviour().gossipsub.all_peers()
                                .find(|(peer, _)| **peer == sender_id).expect("authenticated peer remains connected").1;
                            break topics.into_iter().cloned().collect::<Vec<_>>();
                        }
                    }
                }
            }
        };
        assert!(frames.is_none(), "the control stream must actually be submitted");
        (retained, first_notifications)
    }).await.expect("real TCP control receipt exceeded the harness bound")
}

#[tokio::test]
async fn authenticated_unsolicited_grafts_do_not_grow_retained_remote_topics() {
    let receipt = "graft-receipt-positive-control";
    let (topics, _) = retained_after_receipt(vec![controls_with_receipt(receipt)], receipt).await;
    assert_eq!(
        topics.len(),
        1,
        "GRAFT must not manufacture remote subscriptions"
    );
    assert_eq!(topics[0], gossipsub::IdentTopic::new(receipt).hash());
}

fn subscriptions(values: &[(bool, String)]) -> Vec<u8> {
    let mut rpc = Vec::new();
    for (subscribe, topic) in values {
        let mut option = vec![8, u8::from(*subscribe)];
        field(&mut option, 2, topic.as_bytes());
        field(&mut rpc, 1, &option);
    }
    rpc
}
fn subscribed(count: usize) -> Vec<(bool, String)> {
    (0..count)
        .map(|i| (true, format!("bounded-topic-{i:04}")))
        .collect()
}

#[tokio::test]
async fn subscription_packet_above_two_thousand_is_refused_before_a_later_receipt() {
    let receipt = "after-oversized-subscriptions";
    let (topics, _) = retained_after_receipt(
        vec![
            subscriptions(&subscribed(2001)),
            subscriptions(&[(true, receipt.into())]),
        ],
        receipt,
    )
    .await;
    // Both frames share the same ordered stream. The later receipt proves the
    // rejected first frame was actually encountered, rather than never sent.
    assert_eq!(topics, vec![gossipsub::IdentTopic::new(receipt).hash()]);
}

#[tokio::test]
async fn retained_subscription_bound_accepts_duplicates_and_resumes_after_unsubscribe() {
    let receipt = "after-freeing-one-subscription";
    let first = "bounded-topic-0000";
    let overflow = "must-not-survive-the-full-bound";
    let (topics, first_notifications) = retained_after_receipt(
        vec![
            subscriptions(&subscribed(2000)),
            subscriptions(&[(true, first.into())]),
            subscriptions(&[(true, overflow.into())]),
            subscriptions(&[(false, first.into()), (true, receipt.into())]),
        ],
        receipt,
    )
    .await;
    // Final cardinality alone would also pass if the duplicate had been
    // rejected. Both real notifications must precede the final ordered receipt.
    assert_eq!(
        first_notifications, 2,
        "the duplicate SUB must actually be accepted"
    );
    assert_eq!(topics.len(), 2000);
    assert!(topics.contains(&gossipsub::IdentTopic::new(receipt).hash()));
    assert!(!topics.contains(&gossipsub::IdentTopic::new(first).hash()));
    assert!(!topics.contains(&gossipsub::IdentTopic::new(overflow).hash()));
}

#[tokio::test]
async fn real_tcp_gossip_delivers_at_one_hundred_one_hundred_one_and_two_thousand_topics() {
    // Opt-in failure diagnostics on the test runtime's thread. No process-wide
    // subscriber or production payload logging policy is changed.
    let _trace = std::env::var_os("MEWTUAL_GOSSIP_TEST_TRACE").map(|_| {
        tracing::subscriber::set_default(tracing_subscriber::fmt()
            .with_env_filter("libp2p_gossipsub::handler=debug,libp2p_gossipsub::protocol=debug,catcoms_net=debug")
            .with_ansi(false).with_test_writer().finish())
    });
    use catcoms_rt::{MeshTransport, Topic, TransportEvent};
    for count in [100, 101, 2000] {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            let listen = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
            let (a, a_id, _) =
                MeshService::new_tcp_with_key(keypair_from_seed([83; 32]).unwrap(), &[listen], &[])
                    .unwrap();
            let (b, _, _) =
                MeshService::new_tcp_with_key(keypair_from_seed([84; 32]).unwrap(), &[], &[])
                    .unwrap();
            let mut last = None;
            for i in 0..count {
                let topic = Topic::new(format!("subscription-bound/{i:04}"));
                a.subscribe(topic.clone()).await.unwrap();
                b.subscribe(topic.clone()).await.unwrap();
                last = Some(topic);
            }
            let topic = last.unwrap();
            let payload = Bytes::from(format!("actual-gossip-at-{count}"));
            // The real driver retains this one original until subscription
            // propagation permits publication. A subscribe queue ACK alone is
            // never the success predicate: B must receive the actual payload.
            a.publish(topic.clone(), payload.clone()).await.unwrap();
            let address = a.next_listen_addr().await.unwrap();
            b.dial(format!("{address}/p2p/{a_id}").parse().unwrap())
                .await
                .unwrap();
            loop {
                let event = b
                    .next_event()
                    .await
                    .expect("receiver closed before actual gossip delivery");
                if let TransportEvent::Gossip {
                    from,
                    topic: received_topic,
                    data,
                } = event
                {
                    assert_eq!(from, a.local_peer());
                    assert_eq!(received_topic, topic);
                    assert_eq!(data, payload);
                    break;
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!("real gossip delivery at {count} topics exceeded the harness bound")
        });
    }
}
