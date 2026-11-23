//! Critical4 renderer retry grammar. This validates caller DATA only; native
//! commands must still authenticate actual owners/generations independently.
//! Keep this closed grammar aligned with native-text-intents.ts. Refuse the
//! whole sealed envelope, rather than dropping one unknown retained request.
use serde_json::{Map, Value};
const REFUSED: &str = "Unsupported saved native chat request. The sealed copy has been kept.";
const MAXIMUM: usize = 16;
const SAFE_INTEGER: u64 = 9_007_199_254_740_991;
fn id(value: Option<&Value>) -> Result<&str, &'static str> {
    value
        .and_then(Value::as_str)
        .filter(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                && s.bytes().any(|b| b != b'0')
        })
        .ok_or(REFUSED)
}
fn text(value: Option<&Value>, max: usize, nonempty: bool) -> Result<(), &'static str> {
    value
        .and_then(Value::as_str)
        .filter(|s| s.len() <= max && (!nonempty || !s.is_empty()))
        .map(|_| ())
        .ok_or(REFUSED)
}
fn exact(object: &Map<String, Value>, fields: &[&str]) -> Result<(), &'static str> {
    if object.len() != fields.len() || fields.iter().any(|field| !object.contains_key(*field)) {
        return Err(REFUSED);
    }
    Ok(())
}
pub(crate) fn validate(object: &Map<String, Value>, version: u64) -> Result<(), &'static str> {
    let Some(value) = object.get("nativeTextIntents") else {
        return Ok(());
    };
    if version != 4 {
        return Err(REFUSED);
    }
    let intents = value
        .as_object()
        .filter(|map| map.len() <= MAXIMUM)
        .ok_or(REFUSED)?;
    for (key, item) in intents {
        let item = item.as_object().ok_or(REFUSED)?;
        if id(item.get("token"))? != key {
            return Err(REFUSED);
        }
        match item.get("kind").and_then(Value::as_str) {
            Some("found") => {
                exact(
                    item,
                    &[
                        "kind",
                        "token",
                        "displayName",
                        "serverName",
                        "advertise",
                        "relay",
                        "rendezvous",
                        "isDm",
                    ],
                )?;
                text(item.get("displayName"), 256, true)?;
                text(item.get("serverName"), 256, true)?;
                for field in ["advertise", "relay", "rendezvous"] {
                    text(item.get(field), 4096, false)?;
                }
                item.get("isDm").and_then(Value::as_bool).ok_or(REFUSED)?;
            }
            Some("create") => {
                let fields = [
                    "kind",
                    "token",
                    "server",
                    "channel",
                    "expectedContext",
                    "text",
                    "replyTo",
                ];
                if let Some(reply) = item.get("replyTo") {
                    exact(item, &fields)?;
                    id(Some(reply))?;
                } else {
                    exact(item, &fields[..fields.len() - 1])?;
                }
                item.get("server")
                    .and_then(Value::as_u64)
                    .filter(|n| *n <= SAFE_INTEGER)
                    .ok_or(REFUSED)?;
                let channel = item
                    .get("channel")
                    .and_then(Value::as_str)
                    .filter(|s| s.len() <= 39)
                    .ok_or(REFUSED)?;
                if channel
                    .parse::<u128>()
                    .ok()
                    .is_none_or(|n| n.to_string() != channel)
                {
                    return Err(REFUSED);
                }
                id(item.get("expectedContext"))?;
                text(item.get("text"), 8192, true)?;
            }
            _ => return Err(REFUSED),
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn create() -> Value {
        serde_json::json!({"kind":"create","token":"01".repeat(32),"server":7,"channel":u128::MAX.to_string(),"expectedContext":"02".repeat(32),"text":"exact\n🐈"})
    }
    fn envelope(intent: Value) -> Value {
        let token = "01".repeat(32);
        serde_json::json!({"version":4,"drafts":{},"readMarks":{},"nativeTextIntents":{token:intent}})
    }
    #[test]
    fn native_continuity_critical4_accepts_exact_requests_and_preserves_old_closed_versions() {
        let root = envelope(create());
        assert!(validate(root.as_object().unwrap(), 4).is_ok());
        assert!(crate::validate_ui_state_json(&root.to_string()).is_ok());
        let mut reply = create();
        reply["replyTo"] = "03".repeat(32).into();
        assert!(crate::validate_ui_state_json(&envelope(reply).to_string()).is_ok());
        for version in [1, 2, 3, 5] {
            let mut old = root.clone();
            old["version"] = version.into();
            assert!(crate::validate_ui_state_json(&old.to_string()).is_err());
        }
        let token = "01".repeat(32);
        let found = serde_json::json!({"kind":"found","token":token,"displayName":"Founder","serverName":"Group","advertise":"","relay":"relay","rendezvous":"","isDm":false});
        assert!(crate::validate_ui_state_json(&envelope(found).to_string()).is_ok());
    }
    #[test]
    fn native_continuity_refuses_future_partial_and_oversized_retained_intents() {
        for (field, value) in [
            ("kind", serde_json::json!("future")),
            ("text", serde_json::json!("")),
            ("text", serde_json::json!("💜".repeat(2049))),
            ("token", serde_json::json!("00".repeat(32))),
            ("expectedContext", serde_json::json!("02".repeat(16))),
            ("replyTo", serde_json::json!(null)),
            ("replyTo", serde_json::json!("00".repeat(32))),
            ("channel", serde_json::json!("01")),
            (
                "channel",
                serde_json::json!("340282366920938463463374607431768211456"),
            ),
            ("server", serde_json::json!(SAFE_INTEGER + 1)),
            ("extra", serde_json::json!(true)),
        ] {
            let mut item = create();
            item[field] = value;
            let original = envelope(item).to_string();
            assert!(crate::validate_ui_state_json(&original).is_err(), "{field}");
        }
        for invalid in [Value::Null, serde_json::json!([]), serde_json::json!("bad")] {
            let mut root = envelope(create());
            root["nativeTextIntents"] = invalid;
            assert!(crate::validate_ui_state_json(&root.to_string()).is_err());
        }
        let mut root = envelope(create());
        root["nativeTextIntents"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("text");
        assert!(crate::validate_ui_state_json(&root.to_string()).is_err());
    }
    #[test]
    fn native_continuity_capacity_is_whole_collection_and_does_not_mutate_input() {
        let mut intents = Map::new();
        for number in 1..=MAXIMUM + 1 {
            let token = format!("{number:064x}");
            let mut item = create();
            item["token"] = token.clone().into();
            intents.insert(token, item);
        }
        let mut root = envelope(create());
        root["nativeTextIntents"] = Value::Object(intents);
        let before = root.to_string();
        assert!(crate::validate_ui_state_json(&before).is_err());
        assert_eq!(root.to_string(), before);
        root["nativeTextIntents"]
            .as_object_mut()
            .unwrap()
            .remove(&format!("{:064x}", MAXIMUM + 1));
        assert!(crate::validate_ui_state_json(&root.to_string()).is_ok());
    }
}
