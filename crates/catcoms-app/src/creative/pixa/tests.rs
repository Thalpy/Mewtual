use super::*;

fn fixture() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../catcoms-replication/tests/fixtures/studio-export-v1.json"
    )))
    .unwrap()
}
fn golden() -> Vec<u8> {
    hex::decode(fixture()["pixaHex"].as_str().unwrap()).unwrap()
}
fn take() -> Value {
    serde_json::from_str(fixture()["noScoreTake"].as_str().unwrap()).unwrap()
}
fn frame() -> Vec<u8> {
    validate_pixa(&golden()).unwrap().frames()[0].to_vec()
}
pub(crate) fn package(frames: &[Vec<u8>], take: &str, entries: &[u8], fps: u8) -> Vec<u8> {
    let mut bytes = b"PIXA".to_vec();
    bytes.push(fps);
    bytes.extend((frames.len() as u16).to_le_bytes());
    for frame in frames {
        bytes.extend((frame.len() as u32).to_le_bytes());
        bytes.extend(frame);
    }
    bytes.extend((take.len() as u32).to_le_bytes());
    bytes.extend(take.as_bytes());
    bytes.extend(((entries.len() / 8) as u16).to_le_bytes());
    bytes.extend(entries);
    bytes
}
fn encoded(mut value: Value) -> String {
    String::from_utf8(canonical(&mut value).unwrap()).unwrap()
}
fn admit(value: Value) -> Result<ValidatedPixa<'static>> {
    // Tests need only a verdict, not a retained view. No leaked allocation or ambient resources.
    validate_pixa(&package(&[frame()], &encoded(value), &[], 12)).map(|_| ValidatedPixa {
        fps: 12,
        frames: vec![],
    })
}

#[test]
fn pixa_shared_rust_typescript_validation_cases_have_identical_verdicts() {
    let shared: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../catcoms-replication/tests/fixtures/studio-pixa-validation-v1.json"
    )))
    .unwrap();
    for case in shared["cases"].as_array().unwrap() {
        let mut value = take();
        for (key, patch) in case["patch"].as_object().unwrap() {
            value[key] = patch.clone();
        }
        assert_eq!(
            admit(value).is_ok(),
            case["accepted"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn pixa_shared_frontend_golden_and_authored_sfx_bounds() {
    let bytes = golden();
    let p = validate_pixa(&bytes).unwrap();
    assert_eq!(p.fps(), 12);
    assert_eq!(p.frames().len(), 1);
    let mut value = take();
    value["patches"] = serde_json::json!([fixture()["patch"].clone()]);
    let patch = StudioPatch::new(&value["patches"][0]).unwrap();
    assert_eq!(hex::encode(patch.id()), fixture()["patchId"]);
    for duration in [1u32, 120, 600_000] {
        let mut entry = vec![0, 0, 0, 60];
        entry.extend(duration.to_le_bytes());
        validate_pixa(&package(&[frame()], &encoded(value.clone()), &entry, 12)).unwrap();
    }
    let mut entry = hex::decode(fixture()["sfxEntryHex"].as_str().unwrap()).unwrap();
    let nonempty = package(&[frame()], &encoded(value.clone()), &entry, 12);
    assert_eq!(
        nonempty.len(),
        fixture()["pixaSfxBytes"].as_u64().unwrap() as usize
    );
    for (at, replacement) in [(0, 1), (2, 1), (3, 128)] {
        let mut bad = entry.clone();
        bad[at] = replacement;
        assert!(validate_pixa(&package(&[frame()], &encoded(value.clone()), &bad, 12)).is_err());
    }
    for duration in [0u32, 600_001, u32::MAX] {
        entry[4..8].copy_from_slice(&duration.to_le_bytes());
        assert!(validate_pixa(&package(&[frame()], &encoded(value.clone()), &entry, 12)).is_err());
    }
}

#[test]
fn pixa_complete_framing_dimensions_and_collection_caps_reject_before_consuming() {
    let good = golden();
    for len in 0..good.len() {
        assert!(validate_pixa(&good[..len]).is_err(), "prefix{len}");
    }
    for (at, v) in [(0, 0), (4, 0), (4, 25), (5, 0), (6, 4)] {
        let mut bad = good.clone();
        bad[at] = v;
        assert!(validate_pixa(&bad).is_err());
    }
    let mut bad = good.clone();
    bad[7..11].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(validate_pixa(&bad).is_err());
    let mut bad = good.clone();
    bad.push(0);
    assert!(validate_pixa(&bad).is_err());
    assert!(validate_pixa(&vec![0; super::super::MAX_BOUNDED_BLOB_BYTES + 1]).is_err());
    let wrong = super::super::tests::golden();
    assert!(validate_pixa(&package(&[wrong], &encoded(take()), &[], 12)).is_err());
    let frames = vec![frame(); 999];
    validate_pixa(&package(&frames, &encoded(take()), &[], 24)).unwrap();
    assert!(validate_pixa(&package(&vec![frame(); 1000], &encoded(take()), &[], 12)).is_err());
    assert!(validate_pixa(&package(
        &vec![vec![0; 65536]; 129],
        &encoded(take()),
        &[],
        12
    ))
    .is_err());
    let mut value = take();
    value["patches"] = serde_json::json!([fixture()["patch"].clone()]);
    let entry = [0, 0, 0, 60, 1, 0, 0, 0];
    validate_pixa(&package(
        &[frame()],
        &encoded(value.clone()),
        &entry.repeat(4096),
        12,
    ))
    .unwrap();
    assert!(validate_pixa(&package(
        &[frame()],
        &encoded(value),
        &entry.repeat(4097),
        12
    ))
    .is_err());
}

#[test]
fn pixa_exact_eight_mib_frame_boundary_and_invalid_utf8_are_pinned() {
    fn pix(runs: usize) -> Vec<u8> {
        let mut bytes = frame()[..23].to_vec();
        let mut remaining = 192 * 144;
        for index in 0..runs {
            let len = 256.min(remaining - (runs - index - 1));
            bytes.extend([(len - 1) as u8, (index % 2) as u8]);
            remaining -= len;
        }
        assert_eq!(remaining, 0);
        bytes
    }
    let full = pix(192 * 144);
    let mut frames = vec![full; 151];
    let remainder = 8 * 1024 * 1024 - frames.iter().map(Vec::len).sum::<usize>();
    frames.push(pix((remainder - 23) / 2));
    let package_bytes = package(&frames, &encoded(take()), &[], 1);
    assert_eq!(
        validate_pixa(&package_bytes)
            .unwrap()
            .frames()
            .iter()
            .map(|f| f.len())
            .sum::<usize>(),
        8 * 1024 * 1024
    );
    frames.push(frame());
    assert!(validate_pixa(&package(&frames, &encoded(take()), &[], 1)).is_err());
    let mut invalid = golden();
    let take_offset = 7 + 4 + frame().len() + 4;
    invalid[take_offset] = 0xff;
    assert!(validate_pixa(&invalid).is_err());
    let text = encoded(take()).replace("\"group\":", "\"group\":\"\\ud800\",\"other\":");
    assert!(validate_pixa(&package(&[frame()], &text, &[], 12)).is_err());
}

#[test]
fn pixa_take_nonce_identity_unknown_fields_and_canonicalization_match_frontend() {
    for nonce in [
        serde_json::json!(1234567890123456u64),
        serde_json::json!(["1234567890123456"]),
        serde_json::json!(null),
        serde_json::json!("0123456789ABCDEf"),
        serde_json::json!("123"),
    ] {
        let mut v = take();
        v["lanes"][0]["sn"] = nonce;
        assert!(admit(v).is_err());
    }
    let mut v = take();
    v["lanes"][0]["sn"] = serde_json::json!("0000000000000000");
    admit(v).unwrap();
    for key in ["group", "call"] {
        let mut v = take();
        v[key] = serde_json::json!("a".repeat(257));
        assert!(admit(v).is_err());
    }
    let mut v = take();
    v["parts"] = serde_json::json!(["same", "same"]);
    assert!(admit(v).is_err());
    let mut v = take();
    v["lanes"] = serde_json::json!([v["lanes"][0].clone(), v["lanes"][0].clone()]);
    assert!(admit(v).is_err());
    let mut v = take();
    v["extra"] = serde_json::json!(1);
    assert!(admit(v).is_err());
    let text = encoded(take());
    for malformed in [
        format!(" {text}"),
        text.replace("\"v\":1", "\"v\":1,\"v\":1"),
        text.replace("\"v\":1", "\"v\":1.0"),
        text.replace("\"bpm\":120", "\"bpm\":1.2e2"),
        text.replace("\"v\":1", "\"v\":-0"),
    ] {
        assert!(validate_pixa(&package(&[frame()], &malformed, &[], 12)).is_err());
    }
}

#[test]
fn pixa_take_events_patches_and_lane_order_are_exact() {
    let mut v = take();
    v["patches"] = serde_json::json!([fixture()["patch"].clone()]);
    v["events"] = serde_json::json!([
        {"ms":0,"lane":0,"n":60,"on":1,"w":"sawtooth","p":0,"q":0},
        {"ms":600000,"lane":0,"n":60,"on":0,"q":1}
    ]);
    admit(v.clone()).unwrap();
    for (key, bad) in [
        ("n", 128),
        ("lane", 1),
        ("p", 1),
        ("q", 4294967296u64),
        ("ms", 600001),
        ("on", 2),
    ] {
        let mut badv = v.clone();
        badv["events"][0][key] = serde_json::json!(bad);
        assert!(admit(badv).is_err());
    }
    let mut bad = v.clone();
    bad["events"][1]["q"] = serde_json::json!(0);
    assert!(admit(bad).is_err());
    let mut bad = v.clone();
    bad["patches"] = serde_json::json!([fixture()["patch"].clone(), fixture()["patch"].clone()]);
    assert!(admit(bad).is_err());
    let mut bad = v.clone();
    bad["patches"][0]["o"][0]["w"] = serde_json::json!(4);
    assert!(admit(bad).is_err());
    let mut bad = v.clone();
    bad["events"][0]["extra"] = serde_json::json!(0);
    assert!(admit(bad).is_err());
    let mut drum = take();
    drum["events"] = serde_json::json!([{"ms":0,"lane":0,"n":9,"d":1,"q":0}]);
    admit(drum.clone()).unwrap();
    drum["events"][0]["n"] = serde_json::json!(10);
    assert!(admit(drum).is_err());
    let mut lanes = v;
    lanes["lanes"] =
        serde_json::json!([{"src":0,"sn":"0000000000000000"},{"src":0,"sn":"1111111111111111"}]);
    lanes["events"] = serde_json::json!([{"ms":1,"lane":0,"n":60,"on":0,"q":0},{"ms":0,"lane":1,"n":60,"on":0,"q":0}]);
    assert!(admit(lanes).is_err(), "noncanonical interlane ordering");
}
