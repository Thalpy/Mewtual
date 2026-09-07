use super::*;

fn fixture() -> (RegistryRecovery, RecoverySnapshot) {
    let key = PointerKey::new(DocType::StudioObject, b"private-pointer".to_vec()).unwrap();
    let document = registry_document(b"recovery-test-group", key.bucket()).unwrap();
    let operation = RegistryOp::Put {
        key: key.clone(),
        epoch: 7,
    }
    .domain_op(&document.server_id, [2; 16])
    .unwrap();
    let author = DeviceId::from_bytes([1; 32]);
    let id = operation.id(&author);
    let typed = RegistryRecovery {
        projection: RegistryProjection {
            document: document.clone(),
            bucket: key.bucket(),
            epoch: 0,
            pointers: BTreeMap::from([(key, 7)]),
            overflow: BTreeMap::new(),
            tombstones: BTreeSet::new(),
        },
        receipt_hash: [3; 32],
        excluded: BTreeMap::from([(id, LocalIntent { author, operation })]),
    };
    let snapshot = RecoverySnapshot {
        doc_type: DocType::DocRegistry,
        logical_key: document.logical_key,
        epoch: 0,
        base_close_record_hash: None,
        reason: RecoveryReason::Excluded,
        projection: typed.encode(MAX_RECOVERY_SNAPSHOT_BYTES).unwrap(),
        tombstones: vec![],
        elements: vec![],
        conflicts: vec![],
        applied_ops: vec![id],
    };
    (typed, snapshot)
}

#[test]
fn registry_recovery_codec_roundtrip_is_canonical_and_debug_is_redacted() {
    let (typed, snapshot) = fixture();
    let restored = RegistryRecovery::from_snapshot(
        &snapshot,
        &typed.projection.document,
        typed.projection.bucket,
    )
    .unwrap();
    assert_eq!(restored.projection(), typed.projection());
    assert_eq!(restored.receipt_hash(), typed.receipt_hash());
    assert_eq!(restored.excluded_operations(), typed.excluded_operations());
    assert_eq!(
        restored.encode(MAX_RECOVERY_SNAPSHOT_BYTES).unwrap(),
        snapshot.projection
    );
    assert_eq!(
        format!("{restored:?}"),
        "RegistryRecovery { epoch: 0, pointers: 1, overflow: 0, excluded_operations: 1, .. }"
    );
    assert_eq!(format!("{snapshot:?}"), format!("RecoverySnapshot {{ doc_type: DocRegistry, epoch: 0, reason: Excluded, projection_bytes: {}, applied_operations: 1, .. }}", snapshot.projection.len()));
    // Pins the generic wrapper AND the typed v1 byte layout, canonical domain body and scope.
    assert_eq!(
        blake3::hash(&snapshot.encode().unwrap()).to_hex().as_str(),
        "dd7cc715a18e48bb9ba79bf4def703cc51feb992f1c4393e5d601cbfbcbef55c"
    );
}

#[test]
fn registry_recovery_checks_the_whole_wrapper_not_only_payload() {
    let (typed, snapshot) = fixture();
    let rejects = |s: &RecoverySnapshot| {
        assert!(RegistryRecovery::from_snapshot(
            s,
            &typed.projection.document,
            typed.projection.bucket
        )
        .is_err())
    };
    let mut bad = snapshot.clone();
    bad.logical_key[0] ^= 1;
    rejects(&bad);
    let mut bad = snapshot.clone();
    bad.doc_type = DocType::StudioObject;
    rejects(&bad);
    let mut bad = snapshot.clone();
    bad.epoch = 1;
    rejects(&bad);
    let mut bad = snapshot.clone();
    bad.base_close_record_hash = Some([1; 32]);
    rejects(&bad);
    let mut bad = snapshot.clone();
    bad.reason = RecoveryReason::Rewound;
    rejects(&bad);
    let mut bad = snapshot.clone();
    bad.applied_ops.clear();
    rejects(&bad);
    let mut bad = snapshot.clone();
    bad.applied_ops.push(bad.applied_ops[0]);
    rejects(&bad);
    let mut bad = snapshot.clone();
    bad.elements.push(crate::RecoveryElement {
        element_id: [0; 16],
        predecessor: None,
        op_id: [0; 32],
        author: DeviceId::from_bytes([0; 32]),
    });
    rejects(&bad);
    let mut bad = snapshot.clone();
    bad.projection.push(0);
    rejects(&bad);
    let mut bad = snapshot.clone();
    bad.projection.resize(MAX_RECOVERY_SNAPSHOT_BYTES + 1, 0);
    rejects(&bad);
    let mut wrong = typed.projection.document.clone();
    wrong.server_id.push(0);
    assert!(RegistryRecovery::from_snapshot(&snapshot, &wrong, typed.projection.bucket).is_err());
    assert!(RegistryRecovery::from_snapshot(
        &snapshot,
        &typed.projection.document,
        typed.projection.bucket.wrapping_add(1)
    )
    .is_err());
}

#[test]
fn registry_recovery_refuses_duplicate_keys_bad_counts_and_author_id_substitution() {
    let (mut typed, snapshot) = fixture();
    let document = typed.projection.document.clone();
    let bucket = typed.projection.bucket;
    let rejects = |s: &RecoverySnapshot| {
        assert!(RegistryRecovery::from_snapshot(s, &document, bucket).is_err())
    };
    let offset = 1 + 4 + document.server_id.len() + 1 + 36;
    let mut bad = snapshot.clone();
    bad.projection[offset..offset + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    rejects(&bad);
    let key = typed.projection.pointers.keys().next().unwrap().clone();
    let entry_len = 2 + 4 + key.key.len() + 8;
    let mut bad = snapshot.clone();
    let entry = bad.projection[offset + 4..offset + 4 + entry_len].to_vec();
    bad.projection[offset..offset + 4].copy_from_slice(&2u32.to_be_bytes());
    bad.projection.splice(offset + 4..offset + 4, entry);
    rejects(&bad);
    typed.excluded.values_mut().next().unwrap().author = DeviceId::from_bytes([9; 32]);
    let mut bad = snapshot.clone();
    bad.projection = typed.encode(MAX_RECOVERY_SNAPSHOT_BYTES).unwrap();
    rejects(&bad);
    typed.excluded.values_mut().next().unwrap().author = DeviceId::from_bytes([1; 32]);
    typed.projection.tombstones.insert(key.clone());
    let mut bad = snapshot.clone();
    bad.projection = typed.encode(MAX_RECOVERY_SNAPSHOT_BYTES).unwrap();
    rejects(&bad);
    typed.projection.tombstones.clear();
    typed.projection.overflow.insert(key, 7);
    let mut bad = snapshot.clone();
    bad.projection = typed.encode(MAX_RECOVERY_SNAPSHOT_BYTES).unwrap();
    rejects(&bad);
}

#[test]
fn registry_recovery_exact_size_and_tombstone_only_evidence() {
    let (mut typed, mut snapshot) = fixture();
    let size = snapshot.projection.len();
    assert_eq!(typed.encode(size).unwrap().len(), size);
    assert!(matches!(typed.encode(size - 1), Err(ReplError::EpochBound)));
    let key = typed.projection.pointers.keys().next().unwrap().clone();
    typed.projection.pointers.clear();
    typed.projection.tombstones.insert(key.clone());
    typed.excluded.clear();
    snapshot.projection = typed.encode(MAX_RECOVERY_SNAPSHOT_BYTES).unwrap();
    let restored = RegistryRecovery::from_snapshot(
        &snapshot,
        &typed.projection.document,
        typed.projection.bucket,
    )
    .unwrap();
    assert!(restored.excluded_operations().is_empty());
    assert!(restored.projection().tombstones.contains(&key));
    typed.projection.tombstones.clear();
    snapshot.projection = typed.encode(MAX_RECOVERY_SNAPSHOT_BYTES).unwrap();
    assert!(
        RegistryRecovery::from_snapshot(
            &snapshot,
            &typed.projection.document,
            typed.projection.bucket
        )
        .is_err(),
        "empty evidence must not consume a slot"
    );
}

#[test]
fn registry_recovery_valid_overflow_survives_roundtrip_without_stealing_admitted_slots() {
    let (mut typed, mut snapshot) = fixture();
    for n in 0..1_000_000 {
        let key =
            PointerKey::new(DocType::StudioObject, format!("overflow-{n}").into_bytes()).unwrap();
        if key.bucket() == typed.projection.bucket {
            typed.projection.pointers.insert(key, n);
        }
        if typed.projection.pointers.len() == MAX_REGISTRY_POINTERS + 1 {
            break;
        }
    }
    assert_eq!(typed.projection.pointers.len(), MAX_REGISTRY_POINTERS + 1);
    let (key, epoch) = typed.projection.pointers.pop_last().unwrap();
    typed.projection.overflow.insert(key.clone(), epoch);
    snapshot.projection = typed.encode(MAX_RECOVERY_SNAPSHOT_BYTES).unwrap();
    let restored = RegistryRecovery::from_snapshot(
        &snapshot,
        &typed.projection.document,
        typed.projection.bucket,
    )
    .unwrap();
    assert_eq!(restored.projection(), typed.projection());
    assert_eq!(restored.projection().pointers.len(), MAX_REGISTRY_POINTERS);
    assert_eq!(restored.projection().overflow[&key], epoch);
    assert!(!restored.projection().pointers.contains_key(&key));
}

#[test]
fn registry_recovery_rejects_malformed_excluded_body_and_wrong_bucket_without_trusting_ids() {
    let (typed, snapshot) = fixture();
    let held = &typed.excluded.values().next().unwrap().operation;
    let offset = snapshot.projection.len() - held.encode().unwrap().len() - 4;
    let rejects = |operation: DomainOp| {
        let mut bad = snapshot.clone();
        // Keep the original id/author/nonce and frame a GENERIC-valid operation. Only the typed
        // validator can reject its semantics; the derived id does not hash the body or type.
        let mut e = Encoder::new();
        e.put_bytes(&operation.encode().unwrap()).unwrap();
        bad.projection.truncate(offset);
        bad.projection.extend(e.finish());
        assert!(RegistryRecovery::from_snapshot(
            &bad,
            &typed.projection.document,
            typed.projection.bucket
        )
        .is_err());
    };
    let mut bad = held.clone();
    bad.body = b"{}".to_vec();
    rejects(bad);
    let mut bad = held.clone();
    bad.doc_type = DocType::StudioObject;
    rejects(bad);
    let other = (0..1000)
        .map(|n| PointerKey::new(DocType::StudioObject, format!("wrong-{n}").into_bytes()).unwrap())
        .find(|key| key.bucket() != typed.projection.bucket)
        .unwrap();
    let mut bad = held.clone();
    bad.body = RegistryOp::Put {
        key: other,
        epoch: 7,
    }
    .encode()
    .unwrap();
    rejects(bad);
}
