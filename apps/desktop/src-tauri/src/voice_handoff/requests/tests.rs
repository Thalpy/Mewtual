use super::*;

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("vectors.json")).unwrap()
}
fn operation(number: u64) -> Operation {
    match number {
        1 => Operation::Input,
        2 => Operation::Admit,
        3 => Operation::Result,
        4 => Operation::ReceiptAck,
        5 => Operation::Settle,
        6 => Operation::ClosePhase,
        7 => Operation::FinalAck,
        8 => Operation::BeginDerive,
        _ => panic!("fixture operation"),
    }
}
fn identity(action: Action) {
    assert_eq!(action.serial, 9_007_199_254_741_007);
    assert_eq!(action.version, u64::MAX);
    assert_eq!(action.input_tag, [0x44; 32]);
}

#[test]
fn native_browser_vectors_preserve_complete_purpose_identity_and_borrow_the_exact_output() {
    let vectors = vectors();
    for case in vectors["cases"].as_array().unwrap() {
        let bytes = hex::decode(case["plaintext"].as_str().unwrap()).unwrap();
        let request =
            Request::decode(operation(case["operation"].as_u64().unwrap()), &bytes).unwrap();
        assert_eq!(request.selector.peer.as_bytes(), &[0x11; 32]);
        assert_eq!(request.selector.join, [0x22; 32]);
        assert_eq!(request.selector.key_ref, [0x33; 32]);
        assert_eq!(request.selector.epoch, u64::MAX);
        let source = &case["request"];
        match request.kind {
            Kind::Begin { retry } => assert_eq!(retry, source["retry"].as_bool().unwrap()),
            Kind::Input => assert_eq!(bytes.len(), 105),
            Kind::ClosePhase { mode } => {
                assert_eq!(bytes.len(), 106);
                assert_eq!(mode == CloseMode::Retire, source["mode"] == "retire");
            }
            Kind::ClosePlan => {
                assert_eq!(bytes.len(), 106);
                assert_eq!(source["kind"], "close-plan");
                assert_eq!(bytes[105], 2);
            }
            Kind::Admit {
                action,
                engine_bytes,
            } => {
                identity(action);
                assert_eq!(engine_bytes, u32::MAX);
            }
            Kind::Result {
                action,
                tag,
                output,
            } => {
                identity(action);
                assert_eq!(tag, [0x55; 32]);
                assert_eq!(
                    output,
                    hex::decode(source["output"].as_str().unwrap()).unwrap()
                );
                assert_eq!(
                    output.as_ptr(),
                    bytes[187..].as_ptr(),
                    "decoder makes no result copy"
                );
                assert!(output.len() <= 528);
            }
            Kind::ReceiptAck { action, digest } => {
                identity(action);
                assert_eq!(digest, [0x55; 32]);
            }
            Kind::Settle { action } => identity(action),
            Kind::FinalAck { proof } => assert_eq!(proof, [0x55; 32]),
        }
    }
}

#[test]
fn every_truncation_trailing_byte_unknown_version_and_status_operation_refuses() {
    let vectors = vectors();
    for case in vectors["cases"].as_array().unwrap() {
        let mut bytes = hex::decode(case["plaintext"].as_str().unwrap()).unwrap();
        let operation = operation(case["operation"].as_u64().unwrap());
        for end in 0..bytes.len() {
            assert!(
                Request::decode(operation, &bytes[..end]).is_err(),
                "truncated {} at {end}",
                case["name"]
            );
        }
        bytes.push(0);
        assert!(Request::decode(operation, &bytes).is_err());
        bytes.pop();
        for version in [0, 2, 255] {
            bytes[0] = version;
            assert!(Request::decode(operation, &bytes).is_err());
        }
        bytes[0] = 1;
        assert!(Request::decode(Operation::Status, &bytes).is_err());
    }
}

#[test]
fn flags_zero_identities_and_result_length_are_closed_while_epoch_zero_remains_valid() {
    let vectors = vectors();
    for case in vectors["cases"].as_array().unwrap() {
        let mut bytes = hex::decode(case["plaintext"].as_str().unwrap()).unwrap();
        let operation = operation(case["operation"].as_u64().unwrap());
        bytes[97..105].fill(0);
        assert_eq!(
            Request::decode(operation, &bytes).unwrap().selector.epoch,
            0
        );
        if matches!(
            operation,
            Operation::Admit | Operation::Result | Operation::ReceiptAck | Operation::Settle
        ) {
            for range in [105..113, 113..121] {
                let mut changed = bytes.clone();
                changed[range].fill(0);
                assert!(Request::decode(operation, &changed).is_err());
            }
        }
        if matches!(operation, Operation::BeginDerive | Operation::ClosePhase) {
            // Mode2 is a distinct Plan intent only for ClosePhase; Begin remains boolean.
            for flag in if operation == Operation::BeginDerive {
                [2, 255]
            } else {
                [3, 255]
            } {
                bytes[105] = flag;
                assert!(Request::decode(operation, &bytes).is_err());
            }
        }
        if operation == Operation::Result {
            for length in [1, 527, 529, u16::MAX] {
                let mut changed = bytes.clone();
                changed[185..187].copy_from_slice(&length.to_be_bytes());
                assert!(Request::decode(operation, &changed).is_err());
            }
            let mut oversized = bytes[..187].to_vec();
            oversized[185..187].copy_from_slice(&529u16.to_be_bytes());
            oversized.resize(187 + 529, 0);
            assert!(Request::decode(operation, &oversized).is_err());
        }
    }
}

#[test]
fn receipt_identity_ignores_retirement_but_commits_every_transition_field_and_closed_error() {
    let original = CryptoResultReceipt {
        action: u64::MAX,
        input_tag: [1; 32],
        output_tag: [2; 32],
        outcome: CryptoOutcome::Derived,
        retired: false,
    };
    let digest = receipt_digest(&original);
    let mut changed = original.clone();
    changed.retired = true;
    assert!(original.same_transition(&changed));
    assert_eq!(receipt_digest(&changed), digest);
    for field in 0..3 {
        let mut changed = original.clone();
        match field {
            0 => changed.action -= 1,
            1 => changed.input_tag[31] ^= 1,
            _ => changed.output_tag[31] ^= 1,
        }
        assert_ne!(receipt_digest(&changed), digest);
    }
    let errors = [
        VoiceError::Malformed,
        VoiceError::Bounds,
        VoiceError::Signature,
        VoiceError::Scope,
        VoiceError::Freshness,
        VoiceError::Unauthorized,
        VoiceError::Signing,
        VoiceError::Capacity,
        VoiceError::Replay,
        VoiceError::Reservation,
        VoiceError::RetryMismatch,
        VoiceError::RateLimited,
    ];
    let mut identities = std::collections::BTreeSet::from([digest]);
    for outcome in [
        CryptoOutcome::Opened,
        CryptoOutcome::Sealed {
            message: [7; 32],
            hash: [8; 32],
        },
        CryptoOutcome::Sealed {
            message: [9; 32],
            hash: [8; 32],
        },
        CryptoOutcome::Sealed {
            message: [7; 32],
            hash: [9; 32],
        },
    ]
    .into_iter()
    .chain(errors.map(CryptoOutcome::Rejected))
    {
        let mut changed = original.clone();
        changed.outcome = outcome;
        assert!(!original.same_transition(&changed));
        assert!(
            identities.insert(receipt_digest(&changed)),
            "distinct native transition"
        );
    }
    assert_eq!(identities.len(), 17);
}
