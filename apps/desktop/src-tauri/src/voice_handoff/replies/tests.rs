use super::*;
use catcoms_sync::voice_v2::crypto_actions::{CryptoResultReceipt, CryptoResultStatus};
use serde_json::Value;

const ERRORS: [VoiceError; 12] = [
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
const NAMES: [&str; 12] = [
    "malformed",
    "bounds",
    "signature",
    "scope",
    "freshness",
    "unauthorized",
    "signing",
    "capacity",
    "replay",
    "reservation",
    "retry-mismatch",
    "rate-limited",
];
fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors.json")).unwrap()
}
fn hash(value: &Value) -> [u8; 32] {
    hex::decode(value.as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap()
}
fn number(value: &Value) -> u64 {
    value.as_str().unwrap().parse().unwrap()
}
fn error(name: &Value) -> VoiceError {
    ERRORS[NAMES
        .iter()
        .position(|candidate| Some(*candidate) == name.as_str())
        .unwrap()]
}
fn fixture(value: &Value) -> Status {
    let action = &value["identity"];
    let receipt = &value["receipt"];
    Status {
        pending: match value["pending"].as_str().unwrap() {
            "not-pending" => PendingStatus::NotPending,
            "open" => PendingStatus::Open,
            "closing" => PendingStatus::Closing,
            "closed-unhanded" => PendingStatus::Closed {
                actor_handoff: false,
            },
            "closed-handed" => PendingStatus::Closed {
                actor_handoff: true,
            },
            _ => panic!("fixture disposition"),
        },
        action: (!action.is_null()).then(|| {
            (
                number(&action["action"]),
                number(&action["version"]),
                hash(&action["inputTag"]),
            )
        }),
        engine: CryptoResultStatus {
            settled: value["settled"].as_bool().unwrap(),
            receipt: (!receipt.is_null()).then(|| CryptoResultReceipt {
                action: number(&receipt["action"]),
                input_tag: hash(&receipt["inputTag"]),
                output_tag: hash(&receipt["outputTag"]),
                outcome: match receipt["outcome"]["kind"].as_str().unwrap() {
                    "derived" => CryptoOutcome::Derived,
                    "sealed" => CryptoOutcome::Sealed {
                        message: hash(&receipt["outcome"]["message"]),
                        hash: hash(&receipt["outcome"]["hash"]),
                    },
                    "opened" => CryptoOutcome::Opened,
                    "rejected" => CryptoOutcome::Rejected(error(&receipt["outcome"]["code"])),
                    _ => panic!("fixture outcome"),
                },
                retired: receipt["retired"].as_bool().unwrap(),
            }),
        },
        acknowledged: value["acknowledged"].as_bool().unwrap(),
        drained: value["localDrained"].as_bool().unwrap(),
    }
}
fn id(value: &Value) -> RowId {
    RowId::from_logical(
        hex::decode(value["row"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn native_browser_history_vectors_cover_all_dispositions_outcomes_errors_and_exact_u64s() {
    let vectors = vectors();
    assert_eq!(STATUS_BYTES, 242);
    for case in vectors["statusCases"].as_array().unwrap() {
        let expected = &case["expected"];
        let encoded = status(id(expected), &fixture(expected)).unwrap();
        assert!(encoded.len() <= 256, "original Control plaintext bound");
        assert_eq!(
            hex::encode(encoded),
            case["plaintext"].as_str().unwrap(),
            "{}",
            case["name"]
        );
    }
    for case in vectors["refusedCases"].as_array().unwrap() {
        let name = &case["expected"]["code"];
        let error = match name.as_str().unwrap() {
            "closed" => Refused::Closed,
            "stopped" => Refused::Stopped,
            _ => Refused::Protocol(error(name)),
        };
        assert_eq!(
            hex::encode(refused(error)),
            case["plaintext"].as_str().unwrap()
        );
    }
    for (tag, error) in ERRORS.into_iter().enumerate() {
        assert_eq!(
            usize::from(error_code(error)),
            tag,
            "closed error tag table"
        );
    }
}

#[test]
fn contradictory_native_history_refuses_instead_of_encoding_successful_cleanup() {
    let vectors = vectors();
    let case = vectors["statusCases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "derived-drained")
        .unwrap();
    let original = fixture(&case["expected"]);
    let id = id(&case["expected"]);
    let mut bad = Vec::new();
    let mut state = original.clone();
    state.action = None;
    bad.push(state);
    let mut state = original.clone();
    state.action.as_mut().unwrap().0 = 0;
    bad.push(state);
    let mut state = original.clone();
    state.action.as_mut().unwrap().1 = 0;
    bad.push(state);
    let mut state = original.clone();
    state.engine.receipt.as_mut().unwrap().action += 1;
    bad.push(state);
    let mut state = original.clone();
    state.engine.receipt.as_mut().unwrap().input_tag[0] ^= 1;
    bad.push(state);
    let mut state = original.clone();
    state.engine.receipt.as_mut().unwrap().retired = false;
    bad.push(state);
    let mut state = original.clone();
    state.engine.receipt = None;
    bad.push(state);
    let mut state = original.clone();
    state.engine.settled = false;
    bad.push(state);
    let mut state = original.clone();
    state.acknowledged = false;
    bad.push(state);
    for pending in [
        PendingStatus::Open,
        PendingStatus::Closing,
        PendingStatus::Closed {
            actor_handoff: true,
        },
        PendingStatus::Closed {
            actor_handoff: false,
        },
    ] {
        let mut state = original.clone();
        state.pending = pending;
        bad.push(state);
    }
    for state in bad {
        assert_eq!(status(id, &state), Err(VoiceError::Malformed));
    }
    // Retirement is observational. Replaying the exact earlier immutable receipt can ACK
    // its original transition even when current status has since observed End/retirement.
    let mut earlier = original.clone();
    earlier.drained = false;
    earlier.engine.receipt.as_mut().unwrap().retired = false;
    assert_eq!(
        &status(id, &earlier).unwrap()[210..],
        &status(id, &original).unwrap()[210..]
    );
}
