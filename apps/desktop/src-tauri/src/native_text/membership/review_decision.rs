//! Select one exact decision on the original recovered founder row.
use super::{decode_exact_hex, listing, RequestCancellation, BUSY};
use catcoms_app::store::{
    NativeTextInitialReviewRequest, NativeTextMemberPreparation, V2MemberSaveRefusal,
};
use catcoms_net::MeshService;
use catcoms_replication::chat::{
    cold_actions::OriginalActionDigest,
    owner_bodies::{ExactReviewOriginal, ReviewCaseId, ReviewDecision},
    owner_header::{OwnerBodyId, OwnerHeaderId},
    ActionId,
};
use catcoms_rt::OsCryptoRng;
use serde_json::{json, Map, Value};

const INVALID: &str = "This held-post decision is invalid.";
const FAILED: &str = "Could not save this decision. Refresh held posts and try again.";

pub(super) struct Input {
    pub(super) channel: u128,
    case: ReviewCaseId,
    action: ActionId,
    original: ExactReviewOriginal,
    previous: Option<OwnerBodyId>,
    intent: [u8; 16],
    choice: ReviewDecision,
}

pub(super) fn parse(fields: &Map<String, Value>) -> Result<Input, &'static str> {
    // Keep the accepted payload closed. The command never takes a proposed
    // original body, signer, or authority from the renderer.
    if fields.len() != 11 {
        return Err(INVALID);
    }
    let previous = match fields.get("previousDecision").ok_or(INVALID)? {
        Value::Null => None,
        value => Some(OwnerBodyId::from_bytes(identifier(value)?)),
    };
    let choice = match fields.get("choice").and_then(Value::as_str) {
        Some("approve") => ReviewDecision::Approve,
        Some("reject") => ReviewDecision::Reject,
        Some("reopen") => ReviewDecision::Reopen,
        _ => return Err(INVALID),
    };
    if choice == ReviewDecision::Reopen && previous.is_none() {
        return Err(INVALID);
    }
    let channel = canonical_decimal(fields, "channel", 39)?;
    let remove_epoch = canonical_decimal(fields, "removalEpoch", 20)?
        .try_into()
        .ok()
        .filter(|epoch| *epoch != 0)
        .ok_or(INVALID)?;
    Ok(Input {
        channel,
        case: ReviewCaseId::from_bytes(identifier(fields.get("caseId").ok_or(INVALID)?)?),
        action: ActionId::from_bytes(identifier(fields.get("action").ok_or(INVALID)?)?),
        original: ExactReviewOriginal {
            digest: OriginalActionDigest::from_bytes(identifier(
                fields.get("originalDigest").ok_or(INVALID)?,
            )?),
            remove_header: OwnerHeaderId::from_bytes(identifier(
                fields.get("removalHeader").ok_or(INVALID)?,
            )?),
            remove_epoch,
        },
        previous,
        intent: identifier(fields.get("intent").ok_or(INVALID)?)?,
        choice,
    })
}

fn canonical_decimal(
    fields: &Map<String, Value>,
    key: &str,
    maximum: usize,
) -> Result<u128, &'static str> {
    let raw = fields
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| text.len() <= maximum)
        .ok_or(INVALID)?;
    raw.parse::<u128>()
        .ok()
        .filter(|value| value.to_string() == raw)
        .ok_or(INVALID)
}

fn identifier<const N: usize>(value: &Value) -> Result<[u8; N], &'static str> {
    let id = decode_exact_hex(value.as_str().ok_or(INVALID)?).map_err(|_| INVALID)?;
    if id == [0; N] {
        return Err(INVALID);
    }
    Ok(id)
}

pub(super) fn select(
    prepared: NativeTextMemberPreparation<MeshService, OsCryptoRng>,
    input: Input,
    caller: &RequestCancellation,
) -> Result<String, &'static str> {
    let mut request = NativeTextInitialReviewRequest::new(
        input.channel,
        (input.case, input.action, input.original),
        input.intent,
        input.choice,
    );
    if let Some(previous) = input.previous {
        request = request.for_successor(previous);
    }
    let entry = prepared
        .select_founder_review(request, caller)
        .map_err(|error| match error {
            V2MemberSaveRefusal::Busy => BUSY,
            V2MemberSaveRefusal::Stale | V2MemberSaveRefusal::Cancelled => listing::STALE,
            _ => FAILED,
        })?;
    let step = entry.case_step_data().ok_or(FAILED)?;
    if step.case_data() != input.case || step.choice_data() != input.choice {
        return Err(FAILED);
    }
    let choice = match input.choice {
        ReviewDecision::Approve => "approve",
        ReviewDecision::Reject => "reject",
        ReviewDecision::Reopen => "reopen",
    };
    // This is the exact request's saved result. A separate audit read may
    // report a later current decision for the same case.
    serde_json::to_string(&json!({
        "status": "saved",
        "caseId": hex::encode(input.case.as_bytes()),
        "intent": hex::encode(input.intent),
        "choice": choice,
        "decisionBodyId": hex::encode(entry.decision_id_data().as_bytes()),
        "streamEntryId": hex::encode(entry.id().as_bytes()),
    }))
    .map_err(|_| FAILED)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Value {
        json!({
            "server": 1,
            "expectedContext": "11".repeat(32),
            "channel": "412",
            "caseId": "22".repeat(32),
            "action": "33".repeat(32),
            "originalDigest": "44".repeat(32),
            "removalHeader": "55".repeat(32),
            "removalEpoch": "7",
            "previousDecision": "66".repeat(32),
            "intent": "77".repeat(16),
            "choice": "reopen",
        })
    }

    #[test]
    fn native_held_decision_uses_exact_case_revision_and_nonce() {
        let original = input();
        let received = parse(original.as_object().unwrap());
        assert!(matches!(received, Ok(Input {
            channel: 412,
            previous: Some(previous),
            intent,
            choice: ReviewDecision::Reopen,
            ..
        }) if previous.as_bytes() == &[0x66; 32] && intent == [0x77; 16]));
        for (key, value) in [
            ("intent", json!("77".repeat(32))),
            ("intent", json!("00".repeat(16))),
            ("channel", json!("0412")),
            ("removalEpoch", json!(7)),
            ("removalEpoch", json!("18446744073709551616")),
            ("removalHeader", json!("AA".repeat(32))),
            ("action", json!("00".repeat(32))),
            ("previousDecision", Value::Null),
            ("choice", json!("delete")),
            ("originalBytes", json!("ignored metadata")),
        ] {
            let mut bad = original.clone();
            bad[key] = value;
            assert!(parse(bad.as_object().unwrap()).is_err(), "{key}");
        }
        let mut initial = original;
        initial["choice"] = json!("approve");
        initial["previousDecision"] = Value::Null;
        assert!(matches!(
            parse(initial.as_object().unwrap()),
            Ok(Input {
                previous: None,
                choice: ReviewDecision::Approve,
                ..
            })
        ));
    }
}
