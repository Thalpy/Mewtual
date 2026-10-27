use super::*;

fn vectors() -> Value {
    serde_json::from_str(include_str!("../../tests/fixtures/studio-ops-v1.json")).unwrap()
}

fn body(name: &str) -> Vec<u8> {
    vectors()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap()["body"]
        .as_str()
        .unwrap()
        .as_bytes()
        .to_vec()
}

fn patch() -> StudioPatch {
    let value: Value = serde_json::from_slice(&body("set_patch")).unwrap();
    StudioPatch::new(&value["descriptor"]).unwrap()
}

fn document(kind: DocType) -> LogicalDocument {
    LogicalDocument::new(b"server".to_vec(), kind, vec![2; 16]).unwrap()
}

fn domain(kind: DocType, body: Vec<u8>) -> DomainOp {
    DomainOp {
        nonce: [7; 16],
        doc_type: kind,
        logical_key: vec![2; 16],
        body,
    }
}

#[test]
fn studio_shared_frontend_golden_vectors_roundtrip_every_operation() {
    let vectors = vectors();
    assert_eq!(vectors["version"], 1);
    let cases = vectors["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 24);
    for row in cases {
        let bytes = row["body"].as_str().unwrap().as_bytes();
        if row["kind"] == "index" {
            let op = IndexOp::decode(bytes).unwrap();
            assert_eq!(op.encode().unwrap(), bytes, "{}", row["name"]);
            assert_eq!(
                IndexOp::decode_domain(
                    &document(DocType::StudioIndex),
                    &domain(DocType::StudioIndex, bytes.to_vec()),
                    &DeviceId::from_bytes([1; 32])
                )
                .unwrap(),
                op
            );
        } else {
            let op = FlipnoteOp::decode(bytes).unwrap();
            assert_eq!(op.encode().unwrap(), bytes, "{}", row["name"]);
            assert_eq!(
                FlipnoteOp::decode_domain(
                    &document(DocType::StudioObject),
                    &domain(DocType::StudioObject, bytes.to_vec())
                )
                .unwrap(),
                op
            );
        }
    }
    assert!(matches!(
        IndexOp::decode(&body("put_object")).unwrap(),
        IndexOp::PutObject {
            kind: StudioKind::Flipnote,
            ts: 123456789,
            expiry: StudioExpiry::Never,
            ..
        }
    ));
    assert!(matches!(
        IndexOp::decode(&body("put_score")).unwrap(),
        IndexOp::PutObject {
            kind: StudioKind::Score,
            ts: 0,
            expiry: StudioExpiry::Unrecorded,
            ..
        }
    ));
    assert!(
        matches!(FlipnoteOp::decode(&body("insert_after")).unwrap(), FlipnoteOp::InsertFrame {after:Some(id), bytes:25, ..} if id == [1;16])
    );
    assert!(matches!(
        FlipnoteOp::decode(&body("set_sfx")).unwrap(),
        FlipnoteOp::SetSfx { note: 127, .. }
    ));
    assert!(matches!(
        FlipnoteOp::decode(&body("score_unlink")).unwrap(),
        FlipnoteOp::SetHeader(FlipnoteHeader::Score(None))
    ));
    assert_eq!(hex(&patch().id()), vectors["patch_id"].as_str().unwrap());
}

#[test]
fn studio_rejects_noncanonical_unknown_duplicate_or_malformed_json() {
    for name in [
        "put_object",
        "set_title",
        "insert_frame",
        "set_patch",
        "score_unlink",
    ] {
        let bytes = body(name);
        let valid: Value = serde_json::from_slice(&bytes).unwrap();
        let mut unknown = valid.clone();
        unknown["unexpected"] = json!(0);
        let mut cases = vec![
            serde_json::to_vec(&unknown).unwrap(),
            [b" ".as_slice(), &bytes].concat(),
        ];
        let mut duplicate = bytes.clone();
        duplicate.splice(1..1, b"\"op\":\"forged\",".iter().copied());
        cases.push(duplicate);
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove("op");
        cases.push(serde_json::to_vec(&missing).unwrap());
        cases.extend([
            b"null".to_vec(),
            b"[]".to_vec(),
            b"{}".to_vec(),
            vec![0xff],
            vec![b'{'; 129],
        ]);
        for invalid in cases {
            if name == "put_object" || name == "set_title" {
                assert!(IndexOp::decode(&invalid).is_err(), "{name}");
            } else {
                assert!(FlipnoteOp::decode(&invalid).is_err(), "{name}");
            }
        }
    }
    for value in [
        "1.0", "1e0", "-0", "-1", "true", "null", "\"12\"", "25", "0",
    ] {
        let bytes = format!("{{\"field\":\"fps\",\"op\":\"set_header\",\"value\":{value}}}");
        assert!(FlipnoteOp::decode(bytes.as_bytes()).is_err(), "{value}");
    }
    // No lossy surrogate replacement: accepted titles contain only valid Unicode scalars.
    for value in [r#""\ud800""#, r#""\udfff""#, r#""\u0061""#] {
        let bytes = format!("{{\"field\":\"title\",\"op\":\"set_header\",\"value\":{value}}}");
        assert!(FlipnoteOp::decode(bytes.as_bytes()).is_err());
    }
}

#[test]
fn studio_json_parser_bounds_valid_nested_input_before_schema_validation() {
    // These inputs are syntactically valid JSON. Unmatched '{' bytes fail immediately and
    // prove nothing about the parser's recursion/work boundary.
    for depth in [8usize, 256] {
        let array = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        let map = format!("{}0{}", "{\"x\":".repeat(depth), "}".repeat(depth));
        for bytes in [array.as_bytes(), map.as_bytes()] {
            assert!(bytes.len() < MAX_DOMAIN_OP_BYTES);
            assert_eq!(parse(bytes).is_ok(), depth == 8);
            assert!(IndexOp::decode(bytes).is_err());
            assert!(FlipnoteOp::decode(bytes).is_err());
        }
    }
}

#[test]
fn studio_ids_header_discriminants_and_forged_attribution_reject() {
    for (name, field_name, bad) in [
        ("insert_frame", "frame", "A".repeat(32)),
        ("insert_frame", "cid", "ab".repeat(31)),
        ("insert_frame", "after", "00".repeat(17)),
        ("remove_export", "export", "../frame".into()),
        ("set_sfx", "patch", "a".repeat(63)),
        ("set_sfx", "sfx", "g".repeat(32)),
        ("score_link", "value", "a".repeat(64)),
    ] {
        let mut v: Value = serde_json::from_slice(&body(name)).unwrap();
        v[field_name] = bad.into();
        assert!(FlipnoteOp::decode(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    for (field_name, value) in [
        ("title", json!(3)),
        ("fps", json!("12")),
        ("w", json!(192)),
        ("score", json!(true)),
    ] {
        let v = json!({"op":"set_header", "field":field_name, "value":value});
        assert!(FlipnoteOp::decode(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    for name in ["insert_frame", "set_export"] {
        let mut v: Value = serde_json::from_slice(&body(name)).unwrap();
        v["author"] = "01".repeat(32).into();
        assert!(FlipnoteOp::decode(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    let mut v: Value = serde_json::from_slice(&body("insert_frame")).unwrap();
    v["after"] = v["frame"].clone();
    assert!(FlipnoteOp::decode(&serde_json::to_vec(&v).unwrap()).is_err());
}

#[test]
fn studio_complete_envelope_scope_and_full_creator_identity_are_required() {
    let doc = document(DocType::StudioIndex);
    let original = domain(DocType::StudioIndex, body("put_object"));
    let mut colliding = [1u8; 32];
    colliding[31] = 2;
    assert!(matches!(
        IndexOp::decode_domain(&doc, &original, &DeviceId::from_bytes(colliding)),
        Err(ReplError::EpochAuthority)
    ));
    let mut wrong = original.clone();
    wrong.logical_key[0] = 3;
    assert!(matches!(
        IndexOp::decode_domain(&doc, &wrong, &DeviceId::from_bytes([1; 32])),
        Err(ReplError::EpochScope)
    ));
    wrong = original.clone();
    wrong.doc_type = DocType::StudioObject;
    assert!(IndexOp::decode_domain(&doc, &wrong, &DeviceId::from_bytes([1; 32])).is_err());
    assert!(
        FlipnoteOp::decode_domain(&doc, &domain(DocType::StudioObject, body("insert_frame")))
            .is_err()
    );
    let mut invalid_target = doc.clone();
    invalid_target.server_id.clear();
    assert!(
        IndexOp::decode_domain(&invalid_target, &original, &DeviceId::from_bytes([1; 32])).is_err()
    );

    // An exactly 64-KiB body is still too large once the P1 framing/key/nonce are charged.
    let empty = IndexOp::SetTitle {
        object: [1; 16],
        title: String::new(),
    }
    .encode()
    .unwrap()
    .len();
    let op = IndexOp::SetTitle {
        object: [1; 16],
        title: "x".repeat(MAX_DOMAIN_OP_BYTES - empty),
    };
    let bytes = op.encode().unwrap();
    assert_eq!(bytes.len(), MAX_DOMAIN_OP_BYTES);
    assert!(IndexOp::decode(&bytes).is_ok());
    assert!(matches!(
        IndexOp::decode_domain(
            &doc,
            &domain(DocType::StudioIndex, bytes),
            &DeviceId::from_bytes([1; 32])
        ),
        Err(ReplError::EpochBound)
    ));
    let overhead = domain(DocType::StudioIndex, vec![]).encode().unwrap().len();
    let exact = IndexOp::SetTitle {
        object: [1; 16],
        title: "x".repeat(MAX_DOMAIN_OP_BYTES - empty - overhead),
    };
    let exact_domain = domain(DocType::StudioIndex, exact.encode().unwrap());
    assert_eq!(exact_domain.encode().unwrap().len(), MAX_DOMAIN_OP_BYTES);
    assert!(IndexOp::decode_domain(&doc, &exact_domain, &DeviceId::from_bytes([1; 32])).is_ok());
    assert!(IndexOp::decode(&vec![b' '; MAX_DOMAIN_OP_BYTES + 1]).is_err());
    assert!(FlipnoteOp::decode(&vec![b' '; MAX_DOMAIN_OP_BYTES + 1]).is_err());
    assert!(IndexOp::SetTitle {
        object: [1; 16],
        title: "\0".repeat(MAX_DOMAIN_OP_BYTES / 2)
    }
    .encode()
    .is_err());
}

#[test]
fn studio_blob_number_and_expiry_boundaries_are_lossless() {
    for (name, max) in [
        ("insert_frame", MAX_FRAME_BYTES),
        ("replace_frame", MAX_FRAME_BYTES),
        ("set_export", MAX_EXPORT_BYTES),
    ] {
        let mut value: Value = serde_json::from_slice(&body(name)).unwrap();
        for bytes in [1, max] {
            value["bytes"] = bytes.into();
            assert!(FlipnoteOp::decode(&serde_json::to_vec(&value).unwrap()).is_ok());
        }
        for bytes in [0, max + 1, u64::MAX] {
            value["bytes"] = bytes.into();
            assert!(FlipnoteOp::decode(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }
    for (name, expiry) in [
        ("expiry_unrecorded", StudioExpiry::Unrecorded),
        ("expiry_never", StudioExpiry::Never),
        ("expiry_zero", StudioExpiry::At(0)),
        ("expiry_max", StudioExpiry::At(MAX_STUDIO_INTEGER)),
    ] {
        assert_eq!(
            IndexOp::decode(&body(name)).unwrap(),
            IndexOp::SetExpiry {
                object: [1; 16],
                expiry
            }
        );
    }
    for name in ["put_object", "expiry_max", "set_export"] {
        let mut value: Value = serde_json::from_slice(&body(name)).unwrap();
        for invalid in [
            json!(-1),
            json!(1.5),
            json!(MAX_STUDIO_INTEGER + 1),
            json!(false),
            json!("never"),
        ] {
            value["expiry"] = invalid;
            let bytes = serde_json::to_vec(&value).unwrap();
            assert!(if name == "set_export" {
                FlipnoteOp::decode(&bytes).is_err()
            } else {
                IndexOp::decode(&bytes).is_err()
            });
        }
    }
    let mut value: Value = serde_json::from_slice(&body("put_object")).unwrap();
    value["ts"] = (MAX_STUDIO_INTEGER + 1).into();
    assert!(IndexOp::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    let mut value: Value = serde_json::from_slice(&body("set_sfx")).unwrap();
    for note in [0, 127] {
        value["note"] = note.into();
        assert!(FlipnoteOp::decode(&serde_json::to_vec(&value).unwrap()).is_ok());
    }
    for note in [-1, 128, 256] {
        value["note"] = note.into();
        assert!(FlipnoteOp::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[test]
fn studio_patch_recipe_is_bounded_and_content_bound_not_opaque_json() {
    let good = patch();
    let mut bad_id = good.id();
    bad_id[0] ^= 1;
    assert!(FlipnoteOp::SetPatch {
        patch: bad_id,
        descriptor: good.clone()
    }
    .encode()
    .is_err());
    let mut v = good.value().clone();
    v["extra"] = Value::Null;
    assert!(StudioPatch::new(&v).is_err());
    for pointer in [
        "/o/0/w", "/o/0/t", "/o/0/c", "/o/0/l", "/e/a", "/e/d", "/e/s", "/e/r", "/f/m", "/f/c",
        "/f/q", "/f/e", "/l/r", "/l/d", "/l/t", "/x/c", "/x/d", "/x/r",
    ] {
        for invalid in [
            json!(99999),
            json!(-99999),
            json!(0.5),
            Value::Null,
            json!(true),
            json!("0"),
        ] {
            let mut v = good.value().clone();
            *v.pointer_mut(pointer).unwrap() = invalid;
            assert!(StudioPatch::new(&v).is_err(), "{pointer}");
        }
    }
    for pointer in ["/o/0", "/e", "/f", "/l", "/x"] {
        let mut v = good.value().clone();
        v.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("hidden".into(), json!(0));
        assert!(StudioPatch::new(&v).is_err());
    }
    for count in [0, 4] {
        let mut v = good.value().clone();
        v["o"] = json!(vec![v["o"][0].clone(); count]);
        assert!(StudioPatch::new(&v).is_err());
    }
    let mut v = good.value().clone();
    v["o"] = json!(vec![v["o"][0].clone(); 3]);
    assert!(StudioPatch::new(&v).is_ok());
    assert_ne!(
        good.id(),
        <[u8; 32]>::from(<sha2::Sha256 as sha2::Digest>::digest(
            serde_json::to_vec(good.value()).unwrap()
        ))
    );
}

#[test]
fn studio_patch_ranges_match_frontend_at_every_numeric_boundary() {
    let ranges: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/studio-patch-ranges-v1.json"
    ))
    .unwrap();
    let ranges = ranges.as_array().unwrap();
    assert_eq!(ranges.len(), 18);
    let recipe = patch();
    for range in ranges {
        let path = range["path"].as_str().unwrap();
        let min = range["min"].as_i64().unwrap();
        let max = range["max"].as_i64().unwrap();
        for (number, accepted) in [(min - 1, false), (min, true), (max, true), (max + 1, false)] {
            let mut value = recipe.value().clone();
            *value.pointer_mut(path).unwrap() = json!(number);
            assert_eq!(
                StudioPatch::new(&value).is_ok(),
                accepted,
                "{path}: {number}"
            );
        }
    }
}

#[test]
fn studio_diagnostics_do_not_disclose_document_content() {
    assert_eq!(
        format!("{:?}", FlipnoteHeader::Title("private title".into())),
        "FlipnoteHeader { .. }"
    );
    assert_eq!(
        format!("{:?}", IndexOp::decode(&body("put_object")).unwrap()),
        "IndexOp { .. }"
    );
    assert_eq!(
        format!("{:?}", FlipnoteOp::decode(&body("set_export")).unwrap()),
        "FlipnoteOp { .. }"
    );
    assert_eq!(format!("{:?}", patch()), "StudioPatch { .. }");
}
