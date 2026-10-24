use super::*;
fn logical(serial: u64) -> [u8; 16] {
    let mut bytes = [0; 16];
    bytes[8..].copy_from_slice(&serial.to_be_bytes());
    bytes
}
fn operation(value: u64) -> Operation {
    match value {
        15 => Operation::SdpSealBegin,
        16 => Operation::SdpOpenBegin,
        17 => Operation::SdpInput,
        18 => Operation::SdpEngineAdmit,
        19 => Operation::SdpSealedResult,
        20 => Operation::SdpOpenedResult,
        21 => Operation::SdpAuthenticationFailed,
        22 => Operation::SdpReceiptAck,
        23 => Operation::SdpSettle,
        24 => Operation::SdpReturn,
        25 => Operation::SdpHistory,
        26 => Operation::SdpDelivery,
        27 => Operation::SdpResolve,
        _ => panic!("closed test operation"),
    }
}
fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("../sdp-action-vectors.json")).unwrap()
}
fn decoded(vector: &serde_json::Value) -> (Operation, BodyKind, [u8; 16], Vec<u8>) {
    let op = operation(vector["operation"].as_u64().unwrap());
    let class = if vector["kind"] == 0 {
        BodyKind::Control
    } else {
        assert_eq!(vector["kind"], 2);
        BodyKind::Sdp
    };
    (
        op,
        class,
        hex::decode(vector["logical"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap(),
        hex::decode(vector["body"].as_str().unwrap()).unwrap(),
    )
}
#[test]
fn shared_browser_vectors_decode_only_the_closed_sdp_grammar_and_borrow_the_actual_body() {
    let vectors = vectors();
    for vector in vectors["requests"].as_array().unwrap() {
        let (op, class, row, bytes) = decoded(vector);
        let request = Request::decode(op, class, row, &bytes).unwrap();
        match request {
            Request::Begin(begin)
            | Request::Resolve {
                original: begin, ..
            } => {
                assert_eq!(begin.parent.logical(), logical(1));
                assert_eq!(begin.pc.base.action.serial, 9_007_199_254_740_993);
                assert_eq!(begin.pc.base.action.version, u64::MAX);
                assert_eq!(begin.pc.base.action.input_tag, [17; 32]);
                assert_eq!(begin.pc.base.handshake, [18; 32]);
                assert_eq!(begin.pc.base.key, [19; 32]);
                assert_eq!(begin.pc.base.sequence, 1);
                assert_eq!(begin.pc.storage, [20; 32]);
                assert_eq!(begin.pc.sdk_generation, u64::MAX);
                assert_eq!(begin.pc.sdk, [21; 32]);
                assert_eq!(begin.proof, [22; 32]);
                if let Some(payload) = begin.plaintext {
                    assert_eq!(payload, b"v=0");
                    assert!(std::ptr::eq(payload.as_ptr(), bytes[227..].as_ptr()));
                }
            }
            Request::Existing(existing) => {
                assert_eq!(existing.row.logical(), logical(2));
                assert_eq!(existing.parent.logical(), logical(1));
                let action = match existing.kind {
                    Kind::Input(action)
                    | Kind::AuthenticationFailed(action)
                    | Kind::Settle(action)
                    | Kind::Admit { action, .. }
                    | Kind::Ack { action, .. }
                    | Kind::Delivery { action, .. } => Some(action),
                    Kind::Sealed {
                        action,
                        tag,
                        bytes: payload,
                    }
                    | Kind::Opened {
                        action,
                        tag,
                        bytes: payload,
                    } => {
                        assert_eq!(tag, [23; 32]);
                        assert!(std::ptr::eq(payload.as_ptr(), bytes[100..].as_ptr()));
                        Some(action)
                    }
                    Kind::Return | Kind::History => None,
                };
                if let Some(action) = action {
                    assert_eq!(action.serial, 9_007_199_254_740_993);
                    assert_eq!(action.version, u64::MAX);
                    assert_eq!(action.input_tag, [17; 32]);
                }
            }
        }
        assert!(super::super::wire_shape(
            op,
            class,
            row,
            bytes.len() + 16,
            [9; 16]
        ));
        assert!(
            super::super::Request::decode(op, &bytes).is_err(),
            "no v1 decoder fallback"
        );
        assert!(
            super::super::confirmation::Request::decode(op, class, row, &bytes).is_err(),
            "no v2 confirmation fallback"
        );
        for end in 0..bytes.len() {
            assert!(
                Request::decode(op, class, row, &bytes[..end]).is_err(),
                "truncation {op:?}:{end}"
            );
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(Request::decode(op, class, row, &longer).is_err());
        for version in [0, 1, 2, 3, 4, 5, 6, 7, 9, 255] {
            let mut changed = bytes.clone();
            changed[0] = version;
            assert!(Request::decode(op, class, row, &changed).is_err());
        }
        for wrong in [BodyKind::Control, BodyKind::Data, BodyKind::Sdp] {
            if wrong != class {
                assert!(!wire_shape(op, wrong, row, bytes.len() + 16));
            }
        }
        assert!(!wire_shape(op, class, [0; 16], bytes.len() + 16));
    }
}
#[test]
fn maximum_complete_sdp_lengths_are_distinct_from_confirmation_and_reject_false_lengths() {
    let vectors = vectors();
    for (name, start, max, op) in [
        ("seal-begin", 227, 8192, Operation::SdpSealBegin),
        ("sealed", 100, 8208, Operation::SdpSealedResult),
        ("opened", 100, 8192, Operation::SdpOpenedResult),
    ] {
        let v = vectors["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == name)
            .unwrap();
        let (_, class, row, mut bytes) = decoded(v);
        bytes.resize(start + max, 42);
        bytes[start - 2..start].copy_from_slice(&(max as u16).to_be_bytes());
        assert!(Request::decode(op, class, row, &bytes).is_ok());
        assert!(wire_shape(op, class, row, bytes.len() + 16));
        assert!(!wire_shape(
            Operation::Result,
            BodyKind::Data,
            row,
            bytes.len() + 16
        ));
        bytes.push(42);
        bytes[start - 2..start].copy_from_slice(&((max + 1) as u16).to_be_bytes());
        assert!(Request::decode(op, class, row, &bytes).is_err());
        bytes.truncate(start + max);
        bytes[start - 2..start].copy_from_slice(&((max - 1) as u16).to_be_bytes());
        assert!(Request::decode(op, class, row, &bytes).is_err());
    }
}
#[test]
fn full_original_identity_purpose_and_closed_unions_reject_substitution() {
    let vectors = vectors();
    for v in vectors["requests"].as_array().unwrap() {
        let (op, class, row, bytes) = decoded(v);
        if matches!(
            op,
            Operation::SdpSealBegin | Operation::SdpOpenBegin | Operation::SdpResolve
        ) {
            for range in [1..9, 9..17, 113..121, 153..161] {
                let mut changed = bytes.clone();
                changed[range].fill(0);
                assert!(Request::decode(op, class, row, &changed).is_err());
            }
            if op == Operation::SdpResolve {
                for offset in [225, 226] {
                    let mut changed = bytes.clone();
                    changed[offset] = 2;
                    assert!(Request::decode(op, class, row, &changed).is_err());
                }
            }
        } else {
            assert!(
                Request::decode(op, class, logical(1), &bytes).is_err(),
                "child cannot be its parent"
            );
            let mut changed = bytes.clone();
            changed[1..17].fill(0);
            assert!(Request::decode(op, class, row, &changed).is_err());
            changed = bytes.clone();
            changed[17] = 2;
            assert!(Request::decode(op, class, row, &changed).is_err());
            if !matches!(op, Operation::SdpHistory | Operation::SdpReturn) {
                for range in [18..26, 26..34] {
                    let mut changed = bytes.clone();
                    changed[range].fill(0);
                    assert!(Request::decode(op, class, row, &changed).is_err());
                }
            }
            if matches!(
                op,
                Operation::SdpSealedResult
                    | Operation::SdpOpenedResult
                    | Operation::SdpAuthenticationFailed
                    | Operation::SdpDelivery
            ) {
                let mut changed = bytes.clone();
                changed[17] ^= 1;
                assert!(Request::decode(op, class, row, &changed).is_err());
            }
            if op == Operation::SdpDelivery {
                let mut changed = bytes.clone();
                changed[66] = 2;
                assert!(Request::decode(op, class, row, &changed).is_err());
            }
        }
    }
}
