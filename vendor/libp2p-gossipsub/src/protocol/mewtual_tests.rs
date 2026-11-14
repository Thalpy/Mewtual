//! Payload framing regressions for the narrow Mewtual compatibility patch.
use super::*;

fn codec(maximum: usize) -> GossipsubCodec {
    GossipsubCodec::new(
        maximum,
        ValidationMode::Anonymous,
        HashMap::new(),
        500,
        maximum,
    )
}
fn prefix(length: usize) -> Vec<u8> {
    let mut scratch = unsigned_varint::encode::usize_buffer();
    unsigned_varint::encode::usize(length, &mut scratch).to_vec()
}
fn frame(payload: &[u8]) -> BytesMut {
    let mut bytes = prefix(payload.len());
    bytes.extend_from_slice(payload);
    BytesMut::from(bytes.as_slice())
}
fn legal_payload(length: usize) -> Vec<u8> {
    assert!(length >= 2);
    let mut bytes = Vec::new();
    if length % 2 == 1 {
        bytes.extend_from_slice(&[0x78, 0x80, 1]);
    } // unknown field15, varint128
    while bytes.len() < length {
        bytes.extend_from_slice(&[0x78, 0]);
    }
    assert_eq!(bytes.len(), length);
    bytes
}

#[test]
fn mewtual_declared_oversize_is_rejected_without_waiting_for_or_consuming_body() {
    for declared in [33, 65536, usize::MAX] {
        let mut data = BytesMut::from(prefix(declared).as_slice());
        let before = data.clone();
        assert!(codec(32).decode(&mut data).is_err());
        assert_eq!(data, before);
    }
    // A bounded incomplete body may wait, without consuming even its prefix.
    let mut data = BytesMut::from(prefix(32).as_slice());
    let before = data.clone();
    assert!(codec(32).decode(&mut data).unwrap().is_none());
    assert_eq!(data, before);
}

#[test]
fn mewtual_prefix_truncation_overflow_and_nonminimal_are_distinct() {
    for bytes in [vec![], vec![0x80]] {
        let mut data = BytesMut::from(bytes.as_slice());
        assert!(codec(32).decode(&mut data).unwrap().is_none());
        assert_eq!(data.as_ref(), bytes);
    }
    for bytes in [vec![0x80, 0], vec![0xff; 32], vec![0x80; 32]] {
        let mut data = BytesMut::from(bytes.as_slice());
        assert!(codec(32).decode(&mut data).is_err());
        assert_eq!(data.as_ref(), bytes);
    }
}

#[test]
fn mewtual_exact_payload_and_coalesced_frames_preserve_each_frame_and_partial_tail() {
    for maximum in [32, 127, 128, 65536] {
        let payload = legal_payload(maximum);
        let first = frame(&payload);
        let second = frame(&payload);
        let mut data = first.clone();
        data.extend_from_slice(&second);
        assert!(data.len() > maximum);
        let mut parser = codec(maximum);
        assert!(parser.decode(&mut data).unwrap().is_some());
        assert_eq!(data, second);
        assert!(parser.decode(&mut data).unwrap().is_some());
        assert!(data.is_empty());
        let mut data = first;
        let cut = second.len() - 1;
        data.extend_from_slice(&second[..cut]);
        assert!(parser.decode(&mut data).unwrap().is_some());
        assert_eq!(data.as_ref(), &second[..cut]);
        assert!(parser.decode(&mut data).unwrap().is_none());
        data.extend_from_slice(&second[cut..]);
        assert!(parser.decode(&mut data).unwrap().is_some());
        assert!(data.is_empty());
    }
}

#[test]
fn mewtual_framing_preserves_cumulative_subscription_control_and_publish_limits() {
    let rpc = proto::Rpc {
        subscriptions: vec![proto::SubOpts {
            subscribe: Some(true),
            topic_id: Some("a".repeat(20)),
            ..Default::default()
        }],
        control: Some(proto::ControlMessage {
            graft: vec![proto::ControlGraft {
                topic_id: Some("b".repeat(20)),
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut data = frame(&rpc.encode_to_vec());
    let mut parser = GossipsubCodec::new(4096, ValidationMode::Anonymous, HashMap::new(), 500, 32);
    let error = parser.decode(&mut data).unwrap_err().to_string();
    assert!(error.contains("rpc control size exceeds"));
    let rpc = proto::Rpc {
        publish: vec![proto::Message::default(); 3],
        ..Default::default()
    };
    let mut data = frame(&rpc.encode_to_vec());
    let mut parser = GossipsubCodec::new(4096, ValidationMode::Anonymous, HashMap::new(), 2, 4096);
    assert!(
        parser
            .decode(&mut data)
            .unwrap_err()
            .to_string()
            .contains("too many publish messages")
    );
}
