use super::*;

// Handconstructed bridge DTOs test the actual native serializer's closed shape, not historical
// signature/authentication. Production mapping is covered by the real Server/Actor native tests.
fn sample_row() -> LegacyRow {
    LegacyRow {
        kind: "legacy-change",
        row_id: format!("legacy:1:{}", "aa".repeat(32)),
        op_hash: "aa".repeat(32),
        namespace_group_id: "01".into(),
        channel: "1".into(),
        author_public_key: Some("02".repeat(32)),
        author_id: Some("03".repeat(32)),
        author_fingerprint: Some("03030303".into()),
        claimed_key_byte_count: 32,
        key_shape: "ed25519",
        signed_doc_type: 1,
        signed_doc_id: "1".into(),
        scope: "matching",
        signature: "valid",
        group_binding: "not-signed",
        authority: "legacy-unproven",
        change_hash: None,
        graph: "unavailable",
        presented_ms: None,
        preview: None,
    }
}
fn sample_detail() -> Reply {
    Reply::Detail {
        availability: "incomplete",
        revision: "00112233445566778899aabbccddeeff".into(),
        row: sample_row(),
        original: Original {
            encoding: "hex",
            bytes: "0001".into(),
            offset: 0,
            total_bytes: 2,
            next_offset: None,
        },
        typed_body: None,
        legacy_change: Some(Decoded {
            status: "decoded",
            actor: Some("02".repeat(32)),
            sequence: Some(u64::MAX.to_string()),
            start_op: Some(u64::MAX.to_string()),
            parents: Vec::new(),
            mutations: vec![Mutation {
                operation_id: format!("{}@{}", u64::MAX, "02".repeat(32)),
                object_id: "_root".into(),
                key: MutationKey::Map {
                    value: "counter".into(),
                },
                operation: "set",
                insert: false,
                value: Some(Scalar {
                    kind: "uint",
                    text: u64::MAX.to_string(),
                }),
                predecessors: Vec::new(),
            }],
            mutations_truncated: false,
        }),
        signature_hex: Some("04".repeat(64)),
        fact_ids: Vec::new(),
        missing_fact_ids: Vec::new(),
        evidence_truncated: false,
    }
}

#[test]
fn actual_native_serializer_matches_shared_frontend_contract_fixture() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../security/admin-chat-ledger-dto-fixture.json"
    ))
    .unwrap();
    let page = Reply::Page {
        availability: "incomplete",
        revision: "00112233445566778899aabbccddeeff".into(),
        coverage: Coverage {
            source: "retained-local-inventory",
            typed: "not-active",
            legacy: "incomplete",
            owner_evidence: "not-active",
            unattributed_changes: "unknown",
        },
        rows: vec![sample_row()],
        known_row_count: 1,
        next_cursor: None,
    };
    let page: serde_json::Value = serde_json::from_str(&encode(page)).unwrap();
    let detail: serde_json::Value = serde_json::from_str(&encode(sample_detail())).unwrap();
    assert_eq!(page, fixture["page"]);
    assert_eq!(detail, fixture["detail"]);
    assert_eq!(detail["legacyChange"]["sequence"], u64::MAX.to_string());
    assert_eq!(
        detail["legacyChange"]["mutations"][0]["value"]["text"],
        u64::MAX.to_string()
    );
}

#[test]
fn encoded_reply_ceiling_preserves_raw_chunk_when_projection_escaping_is_too_large() {
    let mut reply = sample_detail();
    let Reply::Detail {
        legacy_change: Some(decoded),
        ..
    } = &mut reply
    else {
        unreachable!()
    };
    decoded.mutations[0].value.as_mut().unwrap().text = "x\0".repeat(250_000);
    let encoded = encode(reply);
    assert!(encoded.len() <= MAX_REPLY_BYTES);
    let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(value["status"], "detail");
    assert_eq!(value["original"]["bytes"], "0001");
    assert_eq!(value["legacyChange"]["status"], "too-large");
    assert_eq!(value["evidenceTruncated"], true);
}

#[test]
fn capped_writer_refuses_before_crossing_its_encoded_byte_allowance() {
    use std::io::Write;
    let mut writer = CappedWriter(Vec::new());
    writer.write_all(&vec![b'x'; MAX_REPLY_BYTES]).unwrap();
    assert!(writer.write_all(b"!").is_err());
    assert_eq!(writer.0.len(), MAX_REPLY_BYTES);
}

#[test]
fn exact_u64_i64_scalar_text_and_mark_refusal_do_not_use_generic_change_serde() {
    assert_eq!(
        scalar(&automerge::ScalarValue::Uint(u64::MAX)).text,
        u64::MAX.to_string()
    );
    assert_eq!(
        scalar(&automerge::ScalarValue::Int(i64::MIN)).text,
        i64::MIN.to_string()
    );
    let change = automerge::ExpandedChange {
        operations: vec![automerge::legacy::Op {
            action: OpType::MarkBegin(automerge::legacy::MarkData {
                name: "retained mark name".into(),
                value: automerge::ScalarValue::Boolean(true),
                expand: true,
            }),
            obj: automerge::legacy::ObjectId::Root,
            key: Key::Map("text".into()),
            pred: automerge::legacy::SortedVec::new(),
            insert: false,
        }],
        actor_id: automerge::ActorId::from(vec![1; 16]),
        hash: None,
        seq: u64::MAX,
        start_op: std::num::NonZeroU64::new(u64::MAX).unwrap(),
        time: i64::MIN,
        message: None,
        deps: Vec::new(),
        extra_bytes: Vec::new(),
    };
    let (decoded, truncated) = decode(LegacyChangeInspection::Decoded(change));
    assert_eq!(decoded.status, "unsupported");
    assert!(decoded.mutations.is_empty());
    assert!(!truncated);
}

#[test]
fn decoded_hex_and_identifier_expansion_uses_its_own_64k_projection_budget() {
    use automerge::transaction::Transactable;
    let mut graph = automerge::AutoCommit::new().with_actor(automerge::ActorId::from(vec![1; 32]));
    graph
        .put(
            automerge::ROOT,
            "bytes",
            automerge::ScalarValue::Bytes(vec![5; 40_000]),
        )
        .unwrap();
    let change = graph.get_last_local_change().unwrap();
    let original = change.raw_bytes().to_vec();
    let inspection = catcoms_replication::history_inspection::inspect_legacy_change(&original);
    assert!(matches!(inspection, LegacyChangeInspection::Decoded(_)));
    let (decoded, truncated) = decode(inspection);
    assert_eq!(decoded.status, "too-large");
    assert!(decoded.actor.is_none());
    assert!(decoded.mutations.is_empty());
    assert!(!decoded.mutations_truncated);
    assert!(truncated);
    assert_eq!(original, change.raw_bytes());
}

#[test]
fn raw_offsets_are_exact_above_u32_and_refuse_above_javascript_integer_range() {
    if usize::BITS > 32 {
        let large = u32::MAX as usize + 1;
        assert_eq!(safe_offset(large).unwrap(), large as u64);
        assert_eq!(
            safe_offset(MAX_SAFE_INTEGER as usize).unwrap(),
            MAX_SAFE_INTEGER
        );
        assert!(safe_offset(MAX_SAFE_INTEGER as usize + 1).is_err());
    }
}

#[test]
fn malformed_decoded_identifier_projection_does_not_claim_frontend_compatible_decoding() {
    fn sample_change() -> automerge::ExpandedChange {
        automerge::ExpandedChange {
            operations: vec![automerge::legacy::Op {
                action: OpType::Put(automerge::ScalarValue::Boolean(true)),
                obj: automerge::legacy::ObjectId::Root,
                key: Key::Map("text".into()),
                pred: automerge::legacy::SortedVec::new(),
                insert: false,
            }],
            actor_id: automerge::ActorId::from(vec![1; 16]),
            hash: None,
            seq: 1,
            start_op: std::num::NonZeroU64::new(1).unwrap(),
            time: 0,
            message: None,
            deps: Vec::new(),
            extra_bytes: Vec::new(),
        }
    }
    let mut zero_sequence = sample_change();
    zero_sequence.seq = 0;
    let mut unsupported_actor = sample_change();
    unsupported_actor.actor_id = automerge::ActorId::from(vec![1; 1]);
    let mut zero_object = sample_change();
    zero_object.operations[0].obj =
        automerge::legacy::ObjectId::Id(automerge::legacy::OpId::new(0, &zero_object.actor_id));
    for change in [zero_sequence, unsupported_actor, zero_object] {
        let (decoded, truncated) = decode(LegacyChangeInspection::Decoded(change));
        assert_eq!(decoded.status, "unsupported");
        assert!(decoded.actor.is_none());
        assert!(decoded.mutations.is_empty());
        assert!(!decoded.mutations_truncated);
        assert!(!truncated);
    }
}
