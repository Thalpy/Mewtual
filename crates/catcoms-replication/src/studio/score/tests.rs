use super::*;

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/studio-score-ops-v1.json"
    ))
    .unwrap()
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

fn document() -> LogicalDocument {
    LogicalDocument::new(b"server".to_vec(), DocType::StudioObject, vec![2; 16]).unwrap()
}

fn domain(body: Vec<u8>) -> DomainOp {
    DomainOp {
        nonce: [7; 16],
        doc_type: DocType::StudioObject,
        logical_key: vec![2; 16],
        body,
    }
}

#[test]
fn shared_score_vectors_roundtrip_all_variants_and_boundaries() {
    let vectors = vectors();
    assert_eq!(vectors["version"], 1);
    let cases = vectors["cases"].as_array().unwrap();
    // Declared contract change (docs/design-score-layers.md section 9): 18 pre-layer bodies plus
    // five set_note/clear_note bodies. The renderer's studio-score.test.ts asserts the same.
    assert_eq!(cases.len(), 23);
    let mut families = std::collections::BTreeSet::new();
    for row in cases {
        let bytes = row["body"].as_str().unwrap().as_bytes();
        let op = ScoreOp::decode(bytes).unwrap();
        assert_eq!(op.encode().unwrap(), bytes, "{}", row["name"]);
        assert_eq!(
            ScoreOp::decode_domain(&document(), &domain(bytes.to_vec())).unwrap(),
            op
        );
        families.insert(
            serde_json::from_slice::<Value>(bytes).unwrap()["op"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    assert_eq!(
        families.into_iter().collect::<Vec<_>>(),
        [
            "clear_cell",
            "clear_note",
            "remove_patch",
            "set_cell",
            "set_header",
            "set_note",
            "set_patch"
        ]
    );
    assert_eq!(
        (
            SCORE_MAX_STEPS,
            SCORE_ROWS,
            SCORE_MAX_CELLS,
            SCORE_MAX_PATCHES
        ),
        (256, 25, 4096, 64)
    );
}

#[test]
fn optional_length_is_not_silently_normalized_or_coerced() {
    let legacy = ScoreOp::decode(&body("note_legacy")).unwrap();
    let explicit = ScoreOp::decode(&body("note_one")).unwrap();
    assert_ne!(legacy, explicit);
    assert!(matches!(
        legacy,
        ScoreOp::SetCell {
            value: ScoreCell::Note { len: None, .. },
            ..
        }
    ));
    assert!(matches!(
        explicit,
        ScoreOp::SetCell {
            value: ScoreCell::Note { len: Some(1), .. },
            ..
        }
    ));
    let mut last: Value = serde_json::from_slice(&body("note_last")).unwrap();
    last["value"]["len"] = json!(2);
    assert!(
        ScoreOp::decode(&canonical(&last).unwrap()).is_err(),
        "hold cannot extend beyond format step 256"
    );
}

#[test]
fn optional_volume_is_bounded_and_not_silently_normalized() {
    let quiet = ScoreOp::decode(&body("note_volume")).unwrap();
    let full = ScoreOp::decode(&body("note_volume_hold")).unwrap();
    assert!(matches!(
        quiet,
        ScoreOp::SetCell {
            value: ScoreCell::Note {
                len: None,
                volume: Some(1),
                ..
            },
            ..
        }
    ));
    // An explicit 100 is retained as written, exactly like an explicit length of 1.
    assert!(matches!(
        full,
        ScoreOp::SetCell {
            step: 8,
            row: 12,
            value: ScoreCell::Note {
                len: Some(4),
                volume: Some(100),
                ..
            },
        }
    ));
    assert_ne!(ScoreOp::decode(&body("note_legacy")).unwrap(), quiet);
    for invalid in [
        json!(0),
        json!(101),
        json!(256),
        json!(1.5),
        json!("60"),
        Value::Null,
        json!(-1),
    ] {
        let mut value: Value = serde_json::from_slice(&body("note_volume")).unwrap();
        value["value"]["vol"] = invalid.clone();
        assert!(
            ScoreOp::decode(&canonical(&value).unwrap()).is_err(),
            "{invalid}"
        );
    }
    // The in-memory constructor cannot smuggle an out-of-range volume past the codec either.
    for volume in [0, 101] {
        assert!(ScoreOp::SetCell {
            step: 0,
            row: 0,
            value: ScoreCell::Note {
                patch: [0; 32],
                len: None,
                volume: Some(volume),
            },
        }
        .encode()
        .is_err());
    }
}

#[test]
fn score_numeric_types_and_static_ranges_are_strict() {
    for (name, pointer, invalid) in [
        (
            "note_one",
            "/step",
            vec![
                json!(-1),
                json!(256),
                json!(65536),
                json!(1.5),
                json!("0"),
                Value::Null,
            ],
        ),
        (
            "note_one",
            "/row",
            vec![
                json!(-1),
                json!(25),
                json!(256),
                json!(1.5),
                json!("0"),
                Value::Null,
            ],
        ),
        (
            "note_one",
            "/value/len",
            vec![
                json!(-1),
                json!(0),
                json!(257),
                json!(65536),
                json!(1.5),
                json!("1"),
                Value::Null,
            ],
        ),
        (
            "drum_last",
            "/value/pad",
            vec![
                json!(-1),
                json!(10),
                json!(256),
                json!(1.5),
                json!("0"),
                Value::Null,
            ],
        ),
        (
            "bpm_min",
            "/value",
            vec![
                json!(39),
                json!(241),
                json!(65536),
                json!(120.5),
                json!("120"),
                Value::Null,
            ],
        ),
        (
            "bpb_min",
            "/value",
            vec![
                json!(0),
                json!(9),
                json!(256),
                json!(1.5),
                json!("1"),
                Value::Null,
            ],
        ),
        (
            "steps_min",
            "/value",
            vec![
                json!(0),
                json!(257),
                json!(65536),
                json!(1.5),
                json!("1"),
                Value::Null,
            ],
        ),
    ] {
        for invalid in invalid {
            let mut value: Value = serde_json::from_slice(&body(name)).unwrap();
            *value.pointer_mut(pointer).unwrap() = invalid;
            assert!(
                ScoreOp::decode(&canonical(&value).unwrap()).is_err(),
                "{name} {pointer}: {value}"
            );
        }
    }
    for op in [
        ScoreOp::ClearCell { step: 256, row: 0 },
        ScoreOp::ClearCell { step: 0, row: 25 },
        ScoreOp::SetCell {
            step: 0,
            row: 0,
            value: ScoreCell::Note {
                patch: [0; 32],
                len: Some(0),
                volume: None,
            },
        },
        ScoreOp::SetCell {
            step: 256,
            row: 0,
            value: ScoreCell::Note {
                patch: [0; 32],
                len: Some(1),
                volume: None,
            },
        },
        ScoreOp::SetCell {
            step: 0,
            row: 0,
            value: ScoreCell::Note {
                patch: [0; 32],
                len: None,
                volume: Some(0),
            },
        },
        ScoreOp::SetCell {
            step: 0,
            row: 0,
            value: ScoreCell::Note {
                patch: [0; 32],
                len: None,
                volume: Some(101),
            },
        },
        ScoreOp::SetCell {
            step: 0,
            row: 0,
            value: ScoreCell::Drum { pad: 10 },
        },
        ScoreOp::SetHeader(ScoreHeader::Bpm(39)),
        ScoreOp::SetHeader(ScoreHeader::Bpb(9)),
        ScoreOp::SetHeader(ScoreHeader::Steps(0)),
    ] {
        assert!(
            op.encode().is_err(),
            "public constructors are not validation"
        );
    }
}

#[test]
fn score_rejects_unknown_missing_duplicate_and_noncanonical_fields() {
    for name in [
        "note_one",
        "drum_last",
        "clear_min",
        "set_patch",
        "remove_patch",
        "title",
        "bpm_min",
        "layer_note",
        "layer_drum",
        "layer_clear_drums",
        "layer_clear_sound",
    ] {
        let bytes = body(name);
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        let mut extra = value.clone();
        extra["author"] = json!("forged");
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove("op");
        let mut duplicate = bytes.clone();
        duplicate.splice(1..1, b"\"op\":\"forged\",".iter().copied());
        for invalid in [
            canonical(&extra).unwrap(),
            canonical(&missing).unwrap(),
            duplicate,
            [b" ".as_slice(), &bytes].concat(),
            [bytes.as_slice(), b" "].concat(),
        ] {
            assert!(ScoreOp::decode(&invalid).is_err(), "{name}");
        }
    }
    for bytes in [
        br#"{"op":"set_cell","row":0,"step":0,"value":{"k":"d","len":1,"pad":0}}"#.as_slice(),
        br#"{"op":"set_cell","row":0,"step":0,"value":{"k":"d","pad":0,"provenance":{}}}"#,
        br#"{"op":"set_cell","row":0,"step":0,"value":{"k":"d","pad":0,"pad":0}}"#,
        br#"{"op":"clear_cell","row":0,"step":-0}"#,
        br#"{"op":"clear_cell","row":0,"step":0.0}"#,
        br#"{"op":"clear_cell","row":0,"step":0e0}"#,
        br#"{"op":"set_header","field":"title","value":"\ud800"}"#,
        br#"{"field":"fps","op":"set_header","value":12}"#,
        br#"{"op":"set_cell","row":0,"step":0,"value":[]}"#,
    ] {
        assert!(ScoreOp::decode(bytes).is_err());
    }
    assert!(ScoreOp::decode(&[b' '; MAX_DOMAIN_OP_BYTES + 1]).is_err());
}

#[test]
fn score_patch_hashes_reuse_the_existing_recipe_identity() {
    let value: Value = serde_json::from_slice(&body("set_patch")).unwrap();
    let descriptor = StudioPatch::new(&value["descriptor"]).unwrap();
    assert_eq!(
        hex(&descriptor.id()),
        vectors()["patch_id"].as_str().unwrap()
    );
    assert!(ScoreOp::SetPatch {
        patch: [0; 32],
        descriptor
    }
    .encode()
    .is_err());
    for patch in [
        json!("A".repeat(64)),
        json!("a".repeat(63)),
        json!(0),
        json!(["a".repeat(64)]),
    ] {
        let mut invalid = value.clone();
        invalid["patch"] = patch;
        assert!(ScoreOp::decode(&canonical(&invalid).unwrap()).is_err());
    }
    let mut invalid = value.clone();
    invalid["descriptor"]["hidden"] = json!(0);
    assert!(ScoreOp::decode(&canonical(&invalid).unwrap()).is_err());
    let mut invalid = value;
    invalid["patch"] = json!("0".repeat(64));
    assert!(ScoreOp::decode(&canonical(&invalid).unwrap()).is_err());
}

#[test]
fn score_envelopes_bind_exact_type_key_shape_and_complete_byte_budget() {
    let doc = document();
    let op = domain(body("note_one"));
    let mut wrong = op.clone();
    wrong.doc_type = DocType::StudioIndex;
    assert!(matches!(
        ScoreOp::decode_domain(&doc, &wrong),
        Err(ReplError::EpochScope)
    ));
    let mut wrong = op.clone();
    wrong.logical_key[0] ^= 1;
    assert!(matches!(
        ScoreOp::decode_domain(&doc, &wrong),
        Err(ReplError::EpochScope)
    ));
    for len in [15, 17] {
        let mut wrong = op.clone();
        wrong.logical_key = vec![2; len];
        let wrong_doc = LogicalDocument::new(
            b"server".to_vec(),
            DocType::StudioObject,
            wrong.logical_key.clone(),
        )
        .unwrap();
        assert!(matches!(
            ScoreOp::decode_domain(&wrong_doc, &wrong),
            Err(ReplError::EpochScope)
        ));
    }
    let wrong_doc =
        LogicalDocument::new(b"server".to_vec(), DocType::StudioIndex, vec![2; 16]).unwrap();
    assert!(matches!(
        ScoreOp::decode_domain(&wrong_doc, &op),
        Err(ReplError::EpochScope)
    ));

    let empty = ScoreOp::SetHeader(ScoreHeader::Title(String::new()))
        .encode()
        .unwrap();
    let overhead = domain(empty.clone()).encode().unwrap().len() - empty.len();
    let title_bytes = MAX_DOMAIN_OP_BYTES - overhead - empty.len();
    let exact_body = ScoreOp::SetHeader(ScoreHeader::Title("x".repeat(title_bytes)))
        .encode()
        .unwrap();
    let exact = domain(exact_body);
    assert_eq!(exact.encode().unwrap().len(), MAX_DOMAIN_OP_BYTES);
    assert!(ScoreOp::decode_domain(&doc, &exact).is_ok());
    let above = domain(
        ScoreOp::SetHeader(ScoreHeader::Title("x".repeat(title_bytes + 1)))
            .encode()
            .unwrap(),
    );
    assert!(
        ScoreOp::decode(&above.body).is_ok(),
        "body alone fits, complete envelope does not"
    );
    assert!(matches!(
        ScoreOp::decode_domain(&doc, &above),
        Err(ReplError::EpochBound)
    ));
}

#[test]
fn score_specific_bodies_do_not_become_art_operations() {
    for name in [
        "note_legacy",
        "drum_last",
        "clear_min",
        "bpm_min",
        "bpb_min",
        "steps_min",
        "layer_note",
        "layer_clear_drums",
        "layer_clear_sound",
    ] {
        assert!(FlipnoteOp::decode(&body(name)).is_err());
    }
    // Shared patch/title shapes cannot establish a trusted root kind. Callers must select it.
    for name in ["set_patch", "remove_patch", "title"] {
        assert!(FlipnoteOp::decode(&body(name)).is_ok());
    }
}

#[test]
fn layered_note_bodies_take_their_layer_from_the_value_and_share_set_cell_bounds() {
    let id = fixed_hex::<32>(vectors()["patch_id"].as_str().unwrap()).unwrap();
    let ScoreOp::SetNote { value, .. } = ScoreOp::decode(&body("layer_note")).unwrap() else {
        panic!("set_note decodes as SetNote");
    };
    assert_eq!(value.layer(), ScoreLayer::Sound(id));
    assert_eq!(
        ScoreOp::decode(&body("layer_drum")).unwrap(),
        ScoreOp::SetNote {
            step: 255,
            row: 24,
            value: ScoreCell::Drum { pad: 9 }
        }
    );
    assert_eq!(
        ScoreOp::decode(&body("layer_clear_sound")).unwrap(),
        ScoreOp::ClearNote {
            step: 255,
            row: 24,
            layer: ScoreLayer::Sound(id)
        }
    );
    assert_eq!(
        ScoreOp::decode(&body("layer_clear_drums")).unwrap(),
        ScoreOp::ClearNote {
            step: 0,
            row: 0,
            layer: ScoreLayer::Drums
        }
    );
    // The longest key the format can produce is exactly the reader's 73-byte bound.
    let key = state::operation_key(&ScoreOp::decode(&body("layer_clear_sound")).unwrap());
    assert_eq!(key, format!("n/255:24/{}", hex(&id)));
    assert_eq!(key.len(), 73);
    assert_eq!(
        state::operation_key(&ScoreOp::decode(&body("layer_drum")).unwrap()),
        "n/255:24/d"
    );
    // One spelling per layer: `d` or 64 LOWERCASE hex, nothing else.
    assert_eq!(ScoreLayer::parse("d").unwrap(), ScoreLayer::Drums);
    assert_eq!(ScoreLayer::parse(&hex(&id)).unwrap().key(), hex(&id));
    for bad in [
        String::new(),
        "D".into(),
        "drums".into(),
        hex(&id).to_uppercase(),
        hex(&id)[..63].to_string(),
        format!("{}0", hex(&id)),
        format!(" {}", &hex(&id)[1..]),
    ] {
        assert!(ScoreLayer::parse(&bad).is_err(), "{bad:?}");
        let mut value: Value = serde_json::from_slice(&body("layer_clear_drums")).unwrap();
        value["layer"] = json!(bad);
        assert!(ScoreOp::decode(&canonical(&value).unwrap()).is_err());
    }
    for invalid in [json!(null), json!(0), json!(["d"])] {
        let mut value: Value = serde_json::from_slice(&body("layer_clear_drums")).unwrap();
        value["layer"] = invalid;
        assert!(ScoreOp::decode(&canonical(&value).unwrap()).is_err());
    }
    // A set_note carries no layer field of its own: a smuggled one is an unknown field.
    let mut value: Value = serde_json::from_slice(&body("layer_note")).unwrap();
    value["layer"] = json!("d");
    assert!(ScoreOp::decode(&canonical(&value).unwrap()).is_err());
    // Exactly the set_cell value grammar and static bounds.
    for (pointer, invalid) in [
        ("/step", json!(256)),
        ("/row", json!(25)),
        ("/value/len", json!(0)),
        ("/value/len", json!(257)),
        ("/value/vol", json!(0)),
        ("/value/vol", json!(101)),
        ("/value/p", json!("A".repeat(64))),
    ] {
        let mut value: Value = serde_json::from_slice(&body("layer_note")).unwrap();
        if let Some(field) = pointer.strip_prefix("/value/") {
            value["value"][field] = invalid;
        } else {
            value[&pointer[1..]] = invalid;
        }
        assert!(
            ScoreOp::decode(&canonical(&value).unwrap()).is_err(),
            "{pointer}"
        );
    }
    let mut last: Value = serde_json::from_slice(&body("layer_note_last")).unwrap();
    last["value"]["len"] = json!(2);
    assert!(
        ScoreOp::decode(&canonical(&last).unwrap()).is_err(),
        "a layered hold cannot extend beyond format step 256 either"
    );
    for op in [
        ScoreOp::ClearNote {
            step: 256,
            row: 0,
            layer: ScoreLayer::Drums,
        },
        ScoreOp::SetNote {
            step: 0,
            row: 25,
            value: ScoreCell::Drum { pad: 0 },
        },
        ScoreOp::SetNote {
            step: 0,
            row: 0,
            value: ScoreCell::Drum { pad: 10 },
        },
    ] {
        assert!(op.encode().is_err());
    }
    assert_eq!(format!("{:?}", ScoreLayer::Sound(id)), "ScoreLayer { .. }");
    assert!(ScoreLayer::Drums < ScoreLayer::Sound([0; 32]));
}

#[test]
fn score_debug_redacts_notes_patches_and_titles() {
    assert_eq!(
        format!("{:?}", ScoreOp::decode(&body("note_one")).unwrap()),
        "ScoreOp { .. }"
    );
    assert_eq!(
        format!("{:?}", ScoreHeader::Title("private song title".into())),
        "ScoreHeader { .. }"
    );
    assert_eq!(
        format!(
            "{:?}",
            ScoreCell::Note {
                patch: [9; 32],
                len: Some(4),
                volume: Some(70),
            }
        ),
        "ScoreCell { .. }"
    );
}
