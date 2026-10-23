use super::*;

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("vectors.json")).unwrap()
}
fn bytes(value: &serde_json::Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}
fn logical(value: &serde_json::Value) -> [u8; 16] {
    bytes(value).try_into().unwrap()
}
fn operation(number: u64) -> Operation {
    match number {
        1 => Operation::Input,
        2 => Operation::Admit,
        3 => Operation::Result,
        4 => Operation::ReceiptAck,
        5 => Operation::Settle,
        6 => Operation::ClosePhase,
        8 => Operation::BeginDerive,
        _ => panic!("closed child vector operation"),
    }
}
fn body_kind(number: u64) -> BodyKind {
    match number {
        0 => BodyKind::Control,
        1 => BodyKind::Data,
        _ => panic!("closed body kind"),
    }
}

#[test]
fn child_vectors_bind_full_parent_purpose_header_and_borrow_original_result_bytes() {
    let vectors = vectors();
    let mut maximum_control = 0;
    let mut maximum_data = 0;
    for case in vectors["cases"].as_array().unwrap() {
        let wire = bytes(&case["plaintext"]);
        let op = operation(case["operation"].as_u64().unwrap());
        let kind = body_kind(case["bodyKind"].as_u64().unwrap());
        let decoded = Request::decode(op, kind, logical(&case["logical"]), &wire).unwrap();
        assert_eq!(decoded.selector.peer.as_bytes(), &[0x11; 32]);
        assert_eq!(decoded.selector.join, [0x22; 32]);
        assert_eq!(decoded.selector.key_ref, [0x33; 32]);
        assert_eq!(decoded.selector.epoch, u64::MAX);
        assert_eq!(decoded.parent.logical(), logical(&vectors["parentRow"]));
        assert_eq!(
            decoded.row.map(|row| row.logical()),
            (op != Operation::BeginDerive).then(|| logical(&vectors["childRow"]))
        );
        match decoded.purpose {
            ConfirmationPurpose::Start(previous) => assert_eq!(
                previous,
                case["purpose"]["predecessor"]
                    .as_str()
                    .map(|text| hex::decode(text).unwrap().try_into().unwrap())
            ),
            ConfirmationPurpose::Respond(trigger) => {
                assert_eq!(trigger.logical(), logical(&case["purpose"]["triggerRow"]))
            }
            ConfirmationPurpose::Incoming(role) => {
                assert_eq!(u64::from(role), case["purpose"]["role"].as_u64().unwrap())
            }
        }
        match decoded.kind {
            Kind::Result { action, output, .. } => {
                assert_eq!(action.serial, 9_007_199_254_741_007);
                assert_eq!(action.version, u64::MAX);
                assert_eq!(action.input_tag, [0x44; 32]);
                assert_eq!(output, bytes(&case["request"]["output"]));
                assert_eq!(output.as_ptr(), wire[wire.len() - output.len()..].as_ptr());
                assert!(output.len() <= 528);
            }
            Kind::Begin { retry } => assert_eq!(retry, case["request"]["retry"].as_bool().unwrap()),
            Kind::ClosePhase {
                mode: CloseMode::Return,
            } => assert_eq!(case["request"]["kind"], "close"),
            Kind::ClosePhase { .. } | Kind::ClosePlan | Kind::FinalAck { .. } => {
                panic!("child terminal meaning")
            }
            _ => {}
        }
        if kind == BodyKind::Control {
            maximum_control = maximum_control.max(wire.len());
        } else {
            maximum_data = maximum_data.max(wire.len());
        }
    }
    assert_eq!(maximum_control, 234);
    assert!(maximum_control <= 256);
    assert_eq!(maximum_data, 764);
    assert!(
        std::mem::size_of::<super::super::Request<'static>>() <= 208,
        "v1 graph stays fixed"
    );
    assert!(
        std::mem::size_of::<Request<'static>>() <= 320,
        "fixed metadata only, not a complete adapter inventory"
    );
}

#[test]
fn every_child_truncation_trailing_byte_wrong_version_and_wrong_body_kind_refuses() {
    for case in vectors()["cases"].as_array().unwrap() {
        let wire = bytes(&case["plaintext"]);
        let op = operation(case["operation"].as_u64().unwrap());
        let kind = body_kind(case["bodyKind"].as_u64().unwrap());
        let id = logical(&case["logical"]);
        for length in 0..wire.len() {
            assert!(
                Request::decode(op, kind, id, &wire[..length]).is_err(),
                "{} truncation{length}",
                case["name"]
            );
        }
        let mut changed = wire.clone();
        changed.push(0);
        assert!(Request::decode(op, kind, id, &changed).is_err());
        for version in [0, 1, 3, 255] {
            let mut changed = wire.clone();
            changed[0] = version;
            assert!(Request::decode(op, kind, id, &changed).is_err());
        }
        let wrong = if kind == BodyKind::Control {
            BodyKind::Data
        } else {
            BodyKind::Control
        };
        assert!(Request::decode(op, wrong, id, &wire).is_err());
        assert!(
            super::super::Request::decode(op, &wire).is_err(),
            "no v2-to-v1 fallback"
        );
    }
}

#[test]
fn closed_purposes_parent_trigger_rows_roles_and_header_rows_are_canonical() {
    let v = vectors();
    let first = &v["cases"][2]; // first Input, no operation suffix
    assert_eq!(first["request"]["kind"], "input");
    let original = bytes(&first["plaintext"]);
    let child = logical(&v["childRow"]);
    let decode = |wire: &[u8], id| {
        Request::decode(Operation::Input, BodyKind::Control, id, wire)
            .map(|request| request.purpose)
    };
    for tag in 4..=255 {
        let mut wire = original.clone();
        wire[121] = tag;
        assert!(decode(&wire, child).is_err());
    }
    for corrupt in [0, 1] {
        let mut wire = original.clone();
        if corrupt == 0 {
            wire[113..121].fill(0);
        } else {
            wire[105..113].copy_from_slice(&32u64.to_be_bytes());
        }
        assert!(decode(&wire, child).is_err());
    }
    assert!(decode(&original, [0; 16]).is_err());
    assert!(decode(&original, logical(&v["parentRow"])).is_err());
    let begin = bytes(&v["cases"][0]["plaintext"]);
    assert!(Request::decode(Operation::BeginDerive, BodyKind::Control, child, &begin).is_err());
    let mut incoming = original.clone();
    incoming[121] = 3;
    incoming.push(0);
    for role in 0..=255 {
        incoming[122] = role;
        assert_eq!(decode(&incoming, child).is_ok(), (1..=9).contains(&role));
    }
    let mut respond = original.clone();
    respond[121] = 2;
    respond.extend_from_slice(&[0; 16]);
    assert!(decode(&respond, child).is_err());
    respond[122..130].copy_from_slice(&32u64.to_be_bytes());
    respond[137] = 1;
    assert!(decode(&respond, child).is_err());
}

#[test]
fn children_cannot_retire_acknowledge_or_smuggle_oversized_result_work() {
    let v = vectors();
    let child = logical(&v["childRow"]);
    let original = bytes(&v["cases"][2]["plaintext"]);
    for mode in [1, 2, 3, 255] {
        let mut wire = original.clone();
        wire.push(mode);
        assert!(Request::decode(Operation::ClosePhase, BodyKind::Control, child, &wire).is_err());
    }
    let mut ack = original.clone();
    ack.extend_from_slice(&[1; 32]);
    assert!(Request::decode(Operation::FinalAck, BodyKind::Control, child, &ack).is_err());
    assert!(Request::decode(Operation::Status, BodyKind::Control, child, &original).is_err());
    let case = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "retry-result-max")
        .unwrap();
    let mut wire = bytes(&case["plaintext"]);
    assert_eq!(wire.len(), 764);
    wire[154 + 80..154 + 82].copy_from_slice(&529u16.to_be_bytes());
    wire.push(0);
    assert!(Request::decode(Operation::Result, BodyKind::Data, child, &wire).is_err());
}
