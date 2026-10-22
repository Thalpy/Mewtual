//! The request class must survive the actual libp2p per-substream codec clone, not merely a
//! helper's expected response size. A deliberately legacy writer below acts as a hostile peer.
use super::*;
use futures::io::Cursor;
use request_response::Codec;
use std::sync::atomic::{AtomicUsize, Ordering};

fn protocol() -> StreamProtocol {
    StreamProtocol::new(RR_PROTOCOL)
}
fn framed(data: &[u8]) -> Cursor<Vec<u8>> {
    let mut bytes = (data.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(data);
    Cursor::new(bytes)
}
async fn within<T>(future: impl Future<Output = T>) -> T {
    let clock = SystemClock;
    tokio::select! {
        result = future => result,
        _ = clock.sleep(Duration::from_secs(8)) => panic!("bounded codec/TCP operation did not complete"),
    }
}

#[tokio::test]
async fn new_service_rejects_oversized_headers_before_body_read_or_allocation() {
    for kind in [FILE_BRIDGE_KIND, FILE_SOURCE_KIND] {
        let mut codec = BytesCodec::default();
        let mut output = Cursor::new(Vec::new());
        codec
            .write_request(&protocol(), &mut output, vec![kind])
            .await
            .unwrap();
        let mut header = Cursor::new(
            ((FILE_SERVICE_RESPONSE_MAX_BYTES + 1) as u32)
                .to_be_bytes()
                .to_vec(),
        );
        let error = codec
            .read_response(&protocol(), &mut header)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            header.position(),
            4,
            "no response body is needed to refuse allocation"
        );

        let mut bytes = ((FILE_SERVICE_REQUEST_MAX_BYTES + 1) as u32)
            .to_be_bytes()
            .to_vec();
        bytes.push(kind);
        let mut header = Cursor::new(bytes);
        let error = codec
            .read_request(&protocol(), &mut header)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            header.position(),
            5,
            "only the kind may precede request allocation"
        );
    }
}

#[tokio::test]
async fn limits_are_value_only_per_clone_and_reset_for_legacy_and_empty_requests() {
    let base = BytesCodec::default();
    let mut new_stream = base.clone();
    let mut legacy_stream = base.clone();
    let mut output = Cursor::new(Vec::new());
    new_stream
        .write_request(&protocol(), &mut output, vec![FILE_BRIDGE_KIND])
        .await
        .unwrap();
    legacy_stream
        .write_request(&protocol(), &mut output, vec![27])
        .await
        .unwrap();
    let large_legacy = vec![9; FILE_SERVICE_RESPONSE_MAX_BYTES + 1];
    assert_eq!(
        legacy_stream
            .read_response(&protocol(), &mut framed(&large_legacy))
            .await
            .unwrap(),
        large_legacy
    );
    assert_eq!(
        new_stream
            .read_response(&protocol(), &mut framed(&large_legacy))
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );

    for request in [vec![27], Vec::new(), vec![5]] {
        // Reuse the previously narrowed clone in both directions. Neither a subsequent legacy
        // request nor an empty request may inherit its preceding 72 KiB response bound.
        new_stream
            .write_request(&protocol(), &mut output, request.clone())
            .await
            .unwrap();
        assert!(new_stream
            .read_response(&protocol(), &mut framed(&large_legacy))
            .await
            .is_ok());
        new_stream
            .read_request(&protocol(), &mut framed(&[FILE_SOURCE_KIND]))
            .await
            .unwrap();
        new_stream
            .read_request(&protocol(), &mut framed(&request))
            .await
            .unwrap();
        assert!(new_stream
            .write_response(&protocol(), &mut output, large_legacy.clone())
            .await
            .is_ok());
    }
    let max = vec![7; MAX_FRAME];
    assert_eq!(
        legacy_stream
            .read_response(&protocol(), &mut framed(&max))
            .await
            .unwrap()
            .len(),
        MAX_FRAME
    );
}

#[tokio::test]
async fn exact_new_limits_work_and_oversized_writes_emit_no_header() {
    for kind in [FILE_BRIDGE_KIND, FILE_SOURCE_KIND] {
        let mut codec = BytesCodec::default();
        let mut exact_request = vec![0; FILE_SERVICE_REQUEST_MAX_BYTES];
        exact_request[0] = kind;
        assert_eq!(
            codec
                .read_request(&protocol(), &mut framed(&exact_request))
                .await
                .unwrap(),
            exact_request
        );
        let mut output = Cursor::new(Vec::new());
        codec
            .write_response(
                &protocol(),
                &mut output,
                vec![0; FILE_SERVICE_RESPONSE_MAX_BYTES],
            )
            .await
            .unwrap();
        assert_eq!(
            output.into_inner().len(),
            FILE_SERVICE_RESPONSE_MAX_BYTES + 4
        );
        let mut output = Cursor::new(Vec::new());
        assert_eq!(
            codec
                .write_response(
                    &protocol(),
                    &mut output,
                    vec![0; FILE_SERVICE_RESPONSE_MAX_BYTES + 1]
                )
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(output.into_inner().is_empty());
        let mut oversized_request = exact_request.clone();
        oversized_request.push(0);
        let mut output = Cursor::new(Vec::new());
        assert_eq!(
            codec
                .write_request(&protocol(), &mut output, oversized_request)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(output.into_inner().is_empty());
        let mut output = Cursor::new(Vec::new());
        codec
            .write_request(&protocol(), &mut output, exact_request)
            .await
            .unwrap();
        assert!(codec
            .read_response(
                &protocol(),
                &mut framed(&vec![0; FILE_SERVICE_RESPONSE_MAX_BYTES])
            )
            .await
            .is_ok());
    }
}

#[tokio::test]
async fn released_global_ceiling_and_truncated_frames_fail_before_false_success() {
    let mut codec = BytesCodec::default();
    let mut too_large = Cursor::new(((MAX_FRAME + 1) as u32).to_be_bytes().to_vec());
    assert_eq!(
        codec
            .read_request(&protocol(), &mut too_large)
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(too_large.position(), 4);
    for bytes in [vec![0, 0], vec![0, 0, 0, 2, 27]] {
        assert_eq!(
            codec
                .read_request(&protocol(), &mut Cursor::new(bytes))
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
    assert!(codec
        .read_request(&protocol(), &mut framed(&[]))
        .await
        .unwrap()
        .is_empty());
    let mut output = Cursor::new(Vec::new());
    assert_eq!(
        codec
            .write_request(&protocol(), &mut output, vec![0; MAX_FRAME + 1])
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert!(output.into_inner().is_empty());
}

/// Only the test peer bypasses the new response writer bound. Its response is really put on a
/// TCP/Noise/yamux stream, so refusal must happen in the production client's read_response.
#[derive(Clone, Debug, Default)]
struct HostileCodec(BytesCodec, Arc<AtomicUsize>);
#[async_trait::async_trait]
impl Codec for HostileCodec {
    type Protocol = StreamProtocol;
    type Request = Vec<u8>;
    type Response = Vec<u8>;
    async fn read_request<T: AsyncRead + Unpin + Send>(
        &mut self,
        p: &StreamProtocol,
        io: &mut T,
    ) -> io::Result<Vec<u8>> {
        self.0.read_request(p, io).await
    }
    async fn read_response<T: AsyncRead + Unpin + Send>(
        &mut self,
        p: &StreamProtocol,
        io: &mut T,
    ) -> io::Result<Vec<u8>> {
        self.0.read_response(p, io).await
    }
    async fn write_request<T: AsyncWrite + Unpin + Send>(
        &mut self,
        p: &StreamProtocol,
        io: &mut T,
        request: Vec<u8>,
    ) -> io::Result<()> {
        self.0.write_request(p, io, request).await
    }
    async fn write_response<T: AsyncWrite + Unpin + Send>(
        &mut self,
        _: &StreamProtocol,
        io: &mut T,
        response: Vec<u8>,
    ) -> io::Result<()> {
        if response.first() == Some(&FILE_SOURCE_KIND)
            && response.len() > FILE_SERVICE_RESPONSE_MAX_BYTES
        {
            // Observe a real oversized header write, not merely a queued server response. The
            // production reader must reject this before body allocation, even if it then resets
            // the stream while this malicious writer is still attempting the body.
            io.write_all(&(response.len() as u32).to_be_bytes()).await?;
            self.1.fetch_add(1, Ordering::SeqCst);
            io.write_all(&response).await?;
            return io.close().await;
        }
        write_frame(io, &response, MAX_FRAME).await
    }
}

#[tokio::test]
async fn real_tcp_substreams_preserve_new_reply_cap_against_a_hostile_writer_and_legacy_overlap() {
    let key = libp2p::identity::Keypair::ed25519_from_bytes([231; 32]).unwrap();
    let oversized_headers = Arc::new(AtomicUsize::new(0));
    let behaviour = request_response::Behaviour::with_codec(
        HostileCodec(BytesCodec::default(), oversized_headers.clone()),
        [(protocol(), request_response::ProtocolSupport::Full)],
        request_response::Config::default().with_request_timeout(catcoms_rt::REQUEST_TIMEOUT),
    );
    let mut hostile = SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_other_transport(infra_transport::client_transport)
        .unwrap()
        .with_behaviour(|_| behaviour)
        .unwrap()
        .build();
    let peer = *hostile.local_peer_id();
    hostile
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .unwrap();
    let address = within(async {
        loop {
            if let SwarmEvent::NewListenAddr { address, .. } = hostile.select_next_some().await {
                break address;
            }
        }
    })
    .await;
    let serving = tokio::spawn(async move {
        while let Some(event) = hostile.next().await {
            if let SwarmEvent::Behaviour(request_response::Event::Message {
                message:
                    request_response::Message::Request {
                        request, channel, ..
                    },
                ..
            }) = event
            {
                let size =
                    FILE_SERVICE_RESPONSE_MAX_BYTES + usize::from(request.get(1) == Some(&0));
                let response = vec![request[0]; size];
                let _ = hostile.behaviour_mut().send_response(channel, response);
            }
        }
    });
    let (client, _) = MeshService::new_tcp(None, &[]).unwrap();
    within(client.dial(format!("{address}/p2p/{peer}").parse().unwrap()))
        .await
        .unwrap();
    let target = to_peer(&peer);
    within(client.wait_for_peer_connected(target))
        .await
        .unwrap();
    let (new, legacy) = within(async {
        tokio::join!(
            client.request_connected(
                target,
                ProtocolId(RR_PROTOCOL),
                Bytes::from_static(&[FILE_BRIDGE_KIND, 1])
            ),
            client.request_connected(
                target,
                ProtocolId(RR_PROTOCOL),
                Bytes::from_static(&[27, 0])
            ),
        )
    })
    .await;
    assert_eq!(new.unwrap().len(), FILE_SERVICE_RESPONSE_MAX_BYTES);
    assert_eq!(legacy.unwrap().len(), FILE_SERVICE_RESPONSE_MAX_BYTES + 1);
    let refused = within(client.request_connected(
        target,
        ProtocolId(RR_PROTOCOL),
        Bytes::from_static(&[FILE_SOURCE_KIND, 0]),
    ))
    .await;
    assert!(
        matches!(refused, Err(TransportError::NoResponse)),
        "malicious oversized source response must fail at its actual reader: {:?}",
        refused.as_ref().map(|bytes| bytes.len())
    );
    assert_eq!(oversized_headers.load(Ordering::SeqCst), 1);
    let (new, legacy) = within(async {
        tokio::join!(
            client.request_connected(
                target,
                ProtocolId(RR_PROTOCOL),
                Bytes::from_static(&[FILE_SOURCE_KIND, 1])
            ),
            client.request_connected(target, ProtocolId(RR_PROTOCOL), Bytes::from_static(&[5, 0])),
        )
    })
    .await;
    assert_eq!(new.unwrap().len(), FILE_SERVICE_RESPONSE_MAX_BYTES);
    assert_eq!(legacy.unwrap().len(), FILE_SERVICE_RESPONSE_MAX_BYTES + 1);
    serving.abort();
    let _ = serving.await;
}
