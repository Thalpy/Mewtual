//! Real pinned-SDK resumption stores, not fabricated Fresh/Current/source/payment witnesses.
//! Test-only JSON mutations stay bounded and are never used by the production scanner. Do
//! not print these generated private secrets, entire rows or provider snapshots on failure.
use super::*;
use serde_json::{json, Value};

struct InitialSource {
    name: &'static str,
    snapshot: Zeroizing<Vec<u8>>,
    epoch: u64,
    maximum: u64,
    entries: usize,
}

fn initial_sources() -> [InitialSource; 3] {
    let owner = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&owner).unwrap();
    let founder = snapshot_server(&owner, &group).unwrap();
    let package = member
        .key_package_for_invite(group.group_id_ref(), [0x72; 16])
        .unwrap();
    let selected = member.key_package_reference(&package).unwrap();
    let welcome = group.add_member(&owner, package).unwrap().welcome;
    let joined = ServerGroup::join_for_key_package(&member, &welcome, &selected).unwrap();
    assert_eq!(group.epoch(), 1);
    assert_eq!(joined.epoch(), 1);
    assert!(group.epoch_authenticator_id() == joined.epoch_authenticator_id());
    [
        InitialSource {
            name: "founder",
            snapshot: founder,
            epoch: 0,
            maximum: 32,
            entries: 1,
        },
        InitialSource {
            name: "owner-first-Add",
            snapshot: snapshot_server(&owner, &group).unwrap(),
            epoch: 1,
            maximum: 32,
            entries: 2,
        },
        InitialSource {
            name: "selected-Welcome",
            snapshot: snapshot_server(&member, &joined).unwrap(),
            epoch: 1,
            maximum: 0,
            entries: 0,
        },
    ]
}

fn raw_row(source: &[u8], label: &[u8]) -> Vec<u8> {
    let mut decoder = Decoder::new(source);
    let map = deserialize_map(decoder.get_bytes().unwrap()).unwrap();
    decoder.get_bytes().unwrap();
    decoder.get_bytes().unwrap();
    decoder.finish().unwrap();
    let mut matching = map.iter().filter(|(key, _)| key.starts_with(label));
    let row = matching
        .next()
        .expect("genuine producer has the requested row")
        .1
        .clone();
    assert!(matching.next().is_none(), "fixture row must be unambiguous");
    row
}
fn resumption(source: &[u8]) -> Value {
    serde_json::from_slice(&raw_row(source, b"ResumptionPsk")).unwrap()
}
fn row(maximum: u64, cursor: u64, entries: Value) -> Value {
    json!({"max_number_of_secrets":maximum,"resumption_psk":entries,"cursor":cursor})
}
fn encoded(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

#[test]
fn genuine_founder_add_and_welcome_keep_their_exact_resumption_rows_after_bounded_restore() {
    assert_eq!(
        GRAPH,
        128 * 1024,
        "this regression must not raise the graph cap"
    );
    let resources = pool();
    for source in initial_sources() {
        let before = source.snapshot.to_vec();
        let original_row = raw_row(&source.snapshot, b"ResumptionPsk");
        let data = resumption(&source.snapshot);
        assert_eq!(
            data["max_number_of_secrets"].as_u64(),
            Some(source.maximum),
            "{}",
            source.name
        );
        assert_eq!(
            data["cursor"].as_u64(),
            Some(source.entries as u64),
            "{}",
            source.name
        );
        let entries = data["resumption_psk"].as_array().unwrap();
        assert_eq!(entries.len(), source.entries, "{}", source.name);
        for (index, entry) in entries.iter().enumerate() {
            let tuple = entry.as_array().unwrap();
            assert_eq!(tuple.len(), 2);
            assert_eq!(tuple[0].as_u64(), Some(index as u64));
            let wrapper = tuple[1].as_object().unwrap();
            assert_eq!(wrapper.len(), 1);
            assert!(wrapper.contains_key("secret"));
            let secret = wrapper["secret"].as_object().unwrap();
            assert_eq!(secret.len(), 1);
            let value = secret["value"].as_object().unwrap();
            assert_eq!(value.len(), 1);
            let bytes = value["vec"].as_array().unwrap();
            assert_eq!(bytes.len(), 32);
            assert!(bytes
                .iter()
                .all(|byte| byte.as_u64().is_some_and(|n| n <= 255)));
        }
        // This SDK config scalar is zero for ALL three real cases. It does not describe the
        // founder builder's actual retained max32 store, and must never justify trimming it.
        let config: Value =
            serde_json::from_slice(&raw_row(&source.snapshot, b"MlsGroupJoinConfig")).unwrap();
        assert_eq!(config["number_of_resumption_psks"].as_u64(), Some(0));
        SDK_LOAD_ENTRIES.with(|counter| counter.set(0));
        let (device, group) = load(&source.snapshot, &resources, GRAPH).unwrap();
        assert_eq!(SDK_LOAD_ENTRIES.with(Cell::get), 1);
        assert_eq!(group.epoch(), source.epoch);
        assert_eq!(resources.lock().unwrap().used, GRAPH);
        let after = snapshot_server(&device, &group).unwrap();
        assert!(
            raw_row(&after, b"ResumptionPsk") == original_row,
            "{} resumption epoch/secret/cursor bytes were lost or rewritten",
            source.name
        );
        assert!(
            source.snapshot.as_slice() == before.as_slice(),
            "source mutation: {}",
            source.name
        );
        drop(group);
        assert_eq!(
            resources.lock().unwrap().used,
            GRAPH,
            "device retains the SAME graph fee"
        );
        drop(device);
        assert_eq!(resources.lock().unwrap().used, 0);
    }
}

fn negative_rows(
    source: &InitialSource,
    original_owner_entries: &[Value],
) -> Vec<(&'static str, Vec<u8>)> {
    let history = Value::Array(original_owner_entries[..source.epoch as usize + 1].to_vec());
    let secret = original_owner_entries[0][1].clone();
    let replace_first = |entry: Value| {
        let mut entries = history.as_array().unwrap().clone();
        entries[0] = entry;
        Value::Array(entries)
    };
    let mut cases = Vec::new();
    let mut add = |name, data: Value| cases.push((name, encoded(&data)));
    for maximum in [1, 31, 33, u64::MAX] {
        add(
            "unsupported maximum",
            row(maximum, source.epoch + 1, history.clone()),
        );
    }
    for cursor in [0, source.epoch + 2, u64::MAX] {
        add("cursor mismatch", row(32, cursor, history.clone()));
    }
    for scalar in [
        Value::Null,
        json!(-1),
        json!(0.0),
        json!("0"),
        json!(true),
        json!([]),
    ] {
        let mut maximum = row(32, source.epoch + 1, history.clone());
        maximum["max_number_of_secrets"] = scalar.clone();
        add("maximum scalar type", maximum);
        let mut cursor = row(32, source.epoch + 1, history.clone());
        cursor["cursor"] = scalar;
        add("cursor scalar type", cursor);
    }
    for scalar in [
        Value::Null,
        json!(false),
        json!(0),
        json!("entries"),
        json!({}),
    ] {
        let mut malformed = row(32, source.epoch + 1, history.clone());
        malformed["resumption_psk"] = scalar;
        add("entry list type", malformed);
    }
    add("empty max32 store", row(32, 0, json!([])));
    add(
        "max0 nonempty store",
        row(0, 1, json!([[0, secret.clone()]])),
    );
    add("max0 nonzero cursor", row(0, 1, json!([])));
    add(
        "three entries",
        row(
            32,
            3,
            json!([
                [0, secret.clone()],
                [1, secret.clone()],
                [2, secret.clone()]
            ]),
        ),
    );
    add(
        "duplicate epoch",
        row(32, 2, json!([[0, secret.clone()], [0, secret.clone()]])),
    );
    add(
        "out of order",
        row(32, 2, json!([[1, secret.clone()], [0, secret.clone()]])),
    );
    add(
        "future or gap epoch",
        row(32, 2, json!([[0, secret.clone()], [2, secret.clone()]])),
    );
    add(
        "maximum future epoch",
        row(
            32,
            2,
            json!([[0, secret.clone()], [u64::MAX, secret.clone()]]),
        ),
    );
    add("missing epoch0", row(32, 1, json!([[1, secret.clone()]])));
    if source.epoch == 1 {
        add("missing epoch1", row(32, 1, json!([[0, secret.clone()]])));
    } else {
        add(
            "extra epoch1 at epoch0",
            row(32, 2, json!([[0, secret.clone()], [1, secret.clone()]])),
        );
    }
    for epoch in [
        Value::Null,
        json!(-1),
        json!(0.0),
        json!("0"),
        json!(true),
        json!({}),
    ] {
        add(
            "malformed epoch type",
            row(
                32,
                source.epoch + 1,
                replace_first(json!([epoch, secret.clone()])),
            ),
        );
    }
    for entry in [
        json!([]),
        json!([0]),
        json!([0, secret.clone(), secret.clone()]),
        Value::Null,
        json!(true),
        json!(0),
        json!("tuple"),
        json!({"epoch":0,"secret":secret.clone()}),
    ] {
        add(
            "tuple arity or type",
            row(32, source.epoch + 1, replace_first(entry)),
        );
    }
    for malformed in [
        Value::Null,
        json!(false),
        json!(0),
        json!("secret"),
        json!([]),
        json!({}),
        json!({"secret":null}),
        json!({"secret":{"value":null}}),
        json!({"secret":{"value":{"vec":null}}}),
        json!({"secret":{"value":{"vec":vec![0;31]}}}),
        json!({"secret":{"value":{"vec":vec![0;33]}}}),
        json!({"secret":{"value":{"vec":vec![0;32]},"unknown":0}}),
        json!({"secret":{"value":{"vec":vec![0;32],"unknown":0}}}),
        json!({"secret":{"value":{"vec":vec![0;32]}},"unknown":0}),
    ] {
        add(
            "secret wrapper length or type",
            row(32, source.epoch + 1, replace_first(json!([0, malformed]))),
        );
    }
    for value in [
        json!(-1),
        json!(256),
        json!(0.5),
        json!("byte"),
        Value::Null,
        json!(false),
    ] {
        let mut malformed = secret.clone();
        malformed["secret"]["value"]["vec"][0] = value;
        add(
            "secret byte type or range",
            row(32, source.epoch + 1, replace_first(json!([0, malformed]))),
        );
    }
    for malformed in [Value::Null, json!(false), json!(0), json!("row"), json!([])] {
        add("row type", malformed);
    }
    for field in ["max_number_of_secrets", "resumption_psk", "cursor"] {
        let mut missing = row(32, source.epoch + 1, history.clone());
        missing.as_object_mut().unwrap().remove(field);
        add("missing field", missing);
    }
    let mut unknown = row(32, source.epoch + 1, history.clone());
    unknown["unknown"] = json!(0);
    add("unknown field", unknown);
    drop(add);
    let valid = encoded(&row(32, source.epoch + 1, history));
    let mut duplicate = valid.clone();
    assert_eq!(duplicate.pop(), Some(b'}'));
    duplicate.extend_from_slice(br#","cursor":1}"#);
    cases.push(("duplicate field", duplicate));
    cases.push(("truncated row", valid[..valid.len() - 1].to_vec()));
    cases.push(("trailing data", [valid.as_slice(), b" true"].concat()));
    cases
}

#[test]
fn every_invalid_resumption_shape_refuses_before_signer_without_source_change_or_fee_leak() {
    let sources = initial_sources();
    let owner = resumption(&sources[1].snapshot);
    let entries = owner["resumption_psk"].as_array().unwrap();
    assert_eq!(
        entries.len(),
        2,
        "use actual independently generated SDK secrets"
    );
    let resources = pool();
    for source in &sources {
        // A positive control for each SAME source prevents a blanket-refusal test passing.
        drop(load(&source.snapshot, &resources, GRAPH).unwrap());
        assert_eq!(resources.lock().unwrap().used, 0);
        for (name, malformed_row) in negative_rows(source, entries) {
            let damaged = changed(&source.snapshot, b"ResumptionPsk", &malformed_row);
            let before = damaged.clone();
            SDK_LOAD_ENTRIES.with(|counter| counter.set(0));
            assert!(
                load(&damaged, &resources, GRAPH).is_err(),
                "{}: {name}",
                source.name
            );
            assert_eq!(
                SDK_LOAD_ENTRIES.with(Cell::get),
                0,
                "{}: {name}",
                source.name
            );
            assert!(damaged == before, "source changed: {}: {name}", source.name);
            assert_eq!(resources.lock().unwrap().used, 0, "{}: {name}", source.name);
        }
    }
}
