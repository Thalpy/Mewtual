use super::*;
use automerge::transaction::Transactable;
use automerge::{ActorId, Change, ObjType};

fn logical() -> LogicalDocument {
    studio_index_document(b"index-projection-test", [7; 16]).unwrap()
}

fn empty(actor: u8) -> AutoCommit {
    AutoCommit::new().with_actor(ActorId::from(vec![actor; 32]))
}

fn root(actor: u8) -> AutoCommit {
    let mut doc = empty(actor);
    for (key, value) in header(&logical(), 0) {
        doc.put(ROOT, key, value).unwrap();
    }
    doc.commit();
    doc
}

fn put(id: u8, author: u8, title: &str) -> IndexOp {
    IndexOp::PutObject {
        object: [id; 16],
        kind: StudioKind::Flipnote,
        title: title.into(),
        created_by: DeviceId::from_bytes([author; 32]),
        ts: 1234,
        expiry: StudioExpiry::Never,
    }
}

// This deliberately unchecked writer exists only in tests. Shipping it without signed causal
// validation would allow forged authors, marker deletion and arbitrary predecessor mutations.
fn record(operation: &IndexOp, author: u8, nonce: u128) -> (String, Vec<u8>, IndexSource) {
    let author = DeviceId::from_bytes([author; 32]);
    let domain = DomainOp {
        nonce: nonce.to_be_bytes(),
        doc_type: DocType::StudioIndex,
        logical_key: logical().logical_key,
        body: operation.encode().unwrap(),
    };
    let source = IndexSource {
        op_id: domain.id(&author),
        author,
        nonce: domain.nonce,
    };
    let key = match operation {
        IndexOp::PutObject { object, .. } => format!("i/{}/{}", hex(object), hex(&source.op_id)),
        IndexOp::TombstoneObject { object } => format!("d/{}/{}", hex(object), hex(&source.op_id)),
        IndexOp::SetTitle { object, .. } => format!("t/{}", hex(object)),
        IndexOp::SetExpiry { object, .. } => format!("e/{}", hex(object)),
    };
    let mut bytes = vec![1];
    bytes.extend_from_slice(source.author.as_bytes());
    bytes.extend_from_slice(&domain.encode().unwrap());
    (key, bytes, source)
}

fn write(doc: &mut AutoCommit, operation: &IndexOp, author: u8, nonce: u128) -> IndexSource {
    let (key, bytes, source) = record(operation, author, nonce);
    doc.put(ROOT, key, bytes).unwrap();
    doc.put(ROOT, format!("_p1/op/{}", hex(&source.op_id)), 1u64)
        .unwrap();
    source
}

fn read(doc: &AutoCommit) -> StudioIndexProjection {
    StudioIndexProjection::read(&logical(), 0, doc).unwrap()
}

fn converge(mut a: AutoCommit, mut b: AutoCommit) -> (AutoCommit, AutoCommit) {
    a.commit();
    b.commit();
    let ca = a.get_changes(&[]);
    let cb = b.get_changes(&[]);
    a.apply_changes(cb).unwrap();
    b.apply_changes(ca).unwrap();
    assert_eq!(read(&a), read(&b));
    (a, b)
}

#[test]
fn empty_scope_headers_and_three_state_expiry_are_exact() {
    assert!(read(&empty(1)).objects.is_empty());
    assert!(StudioIndexProjection::read(&logical(), 1, &empty(1)).is_err());
    assert_eq!(read(&root(1)).document(), &logical());
    for key in [vec![], vec![0; 15], vec![0; 17]] {
        let mut other = logical();
        other.logical_key = key;
        assert!(StudioIndexProjection::read(&other, 0, &root(1)).is_err());
    }
    let mut other = logical();
    other.doc_type = DocType::StudioObject;
    assert!(StudioIndexProjection::read(&other, 0, &root(1)).is_err());
    let mut missing = root(1);
    missing.delete(ROOT, "kind").unwrap();
    assert!(StudioIndexProjection::read(&logical(), 0, &missing).is_err());
    for had_content in [false, true] {
        let mut erased = root(1);
        if had_content {
            write(&mut erased, &put(3, 1, "erased"), 1, 1);
            erased.commit();
        }
        for key in erased.keys(ROOT).collect::<Vec<_>>() {
            erased.delete(ROOT, key).unwrap();
        }
        erased.commit();
        assert_eq!(erased.keys(ROOT).count(), 0);
        assert!(erased.stats().num_ops > 0);
        assert!(StudioIndexProjection::read(&logical(), 0, &erased).is_err());
    }
    let mut marker_only = empty(1);
    marker_only
        .put(ROOT, format!("_p1/op/{}", hex(&[0; 32])), 1u64)
        .unwrap();
    assert!(StudioIndexProjection::read(&logical(), 0, &marker_only).is_err());

    let mut doc = root(1);
    let creation = write(&mut doc, &put(3, 1, "Moon 🐱"), 1, 1);
    doc.commit();
    let entry = &read(&doc).objects[&[3; 16]];
    assert_eq!(entry.creations.len(), 1);
    assert_eq!(entry.creations[0].value.title, "Moon 🐱");
    assert_eq!(entry.creations[0].value.kind, StudioKind::Flipnote);
    assert_eq!(entry.creations[0].value.created_by, creation.author);
    assert_eq!(entry.creations[0].value.ts, 1234);
    assert_eq!(entry.title.selected.source, creation);
    assert_eq!(entry.expiry.selected.value, StudioExpiry::Never);
    for (nonce, expiry) in [
        StudioExpiry::At(0),
        StudioExpiry::At(25),
        StudioExpiry::Never,
        StudioExpiry::Unrecorded,
    ]
    .into_iter()
    .enumerate()
    {
        let source = write(
            &mut doc,
            &IndexOp::SetExpiry {
                object: [3; 16],
                expiry,
            },
            1,
            nonce as u128 + 2,
        );
        doc.commit();
        let p = read(&doc);
        assert_eq!(p.objects[&[3; 16]].expiry.selected.value, expiry);
        assert_eq!(p.objects[&[3; 16]].expiry.selected.source, source);
        assert!(p.objects[&[3; 16]].expiry.conflicts.is_empty());
    }
}

#[test]
fn independent_roots_merge_and_insertion_collisions_use_smallest_derived_id() {
    // Both devices initialize the headers independently, rather than sharing a nested map.
    let mut a = root(1);
    let mut b = root(2);
    let ia = write(&mut a, &put(3, 1, "first"), 1, 4);
    let mut score = put(3, 2, "second");
    if let IndexOp::PutObject { kind, .. } = &mut score {
        *kind = StudioKind::Score;
    }
    let ib = write(&mut b, &score, 2, 5);
    write(&mut a, &put(4, 1, "another object"), 1, 6);
    let (a, _) = converge(a, b);
    let p = read(&a);
    assert_eq!(p.objects.len(), 2);
    let entry = &p.objects[&[3; 16]];
    assert_eq!(entry.creations.len(), 2);
    assert_eq!(entry.creations[0].source.op_id, ia.op_id.min(ib.op_id));
    assert_eq!(entry.creations[1].source.op_id, ia.op_id.max(ib.op_id));
    assert_eq!(entry.title.selected.value, entry.creations[0].value.title);
    assert_eq!(entry.title.selected.source, entry.creations[0].source);
    assert_eq!(
        entry.creations[0].value.kind,
        if ia.op_id < ib.op_id {
            StudioKind::Flipnote
        } else {
            StudioKind::Score
        }
    );
}

#[test]
fn scalar_conflicts_use_automerge_winner_and_keep_equal_values_with_distinct_authors() {
    let mut base = root(1);
    write(&mut base, &put(3, 1, "initial"), 1, 1);
    base.commit();
    for equal in [false, true] {
        let mut a = base.clone().with_actor(ActorId::from(vec![2; 32]));
        let mut b = base.clone().with_actor(ActorId::from(vec![3; 32]));
        write(
            &mut a,
            &IndexOp::SetTitle {
                object: [3; 16],
                title: "alpha".into(),
            },
            2,
            10,
        );
        write(
            &mut b,
            &IndexOp::SetTitle {
                object: [3; 16],
                title: if equal { "alpha" } else { "beta" }.into(),
            },
            3,
            20,
        );
        write(
            &mut a,
            &IndexOp::SetExpiry {
                object: [3; 16],
                expiry: StudioExpiry::Unrecorded,
            },
            2,
            11,
        );
        write(
            &mut b,
            &IndexOp::SetExpiry {
                object: [3; 16],
                expiry: StudioExpiry::Never,
            },
            3,
            21,
        );
        let (mut a, _) = converge(a, b);
        let p = read(&a);
        let entry = &p.objects[&[3; 16]];
        for (tag, selected, conflicts) in [
            (
                "t",
                &entry.title.selected.source,
                entry.title.conflicts.len(),
            ),
            (
                "e",
                &entry.expiry.selected.source,
                entry.expiry.conflicts.len(),
            ),
        ] {
            let (value, _) = a
                .get(ROOT, format!("{tag}/{}", hex(&[3; 16])))
                .unwrap()
                .unwrap();
            let (source, _) = decode_record(&logical(), record_bytes(&value).unwrap()).unwrap();
            assert_eq!(selected, &source);
            assert_eq!(conflicts, 1);
        }
        assert_ne!(
            entry.title.selected.source.author,
            entry.title.conflicts[0].source.author
        );
        write(
            &mut a,
            &IndexOp::SetTitle {
                object: [3; 16],
                title: "resolved".into(),
            },
            2,
            12,
        );
        a.commit();
        assert_eq!(read(&a).objects[&[3; 16]].title.selected.value, "resolved");
        assert!(read(&a).objects[&[3; 16]].title.conflicts.is_empty());
    }
}

#[test]
fn a_late_smaller_creation_does_not_erase_an_explicit_rename_or_expiry() {
    let mut candidates: Vec<_> = (0..4)
        .map(|nonce| record(&put(3, 1, "initial"), 1, nonce))
        .collect();
    candidates.sort_by_key(|(_, _, source)| source.op_id);
    let (small_key, small_bytes, small) = candidates.remove(0);
    let (large_key, large_bytes, _) = candidates.pop().unwrap();
    let base = root(1);
    let mut a = base.clone().with_actor(ActorId::from(vec![2; 32]));
    let mut b = base.with_actor(ActorId::from(vec![3; 32]));
    a.put(ROOT, large_key, large_bytes).unwrap();
    a.commit();
    write(
        &mut a,
        &IndexOp::SetTitle {
            object: [3; 16],
            title: "renamed".into(),
        },
        2,
        9,
    );
    write(
        &mut a,
        &IndexOp::SetExpiry {
            object: [3; 16],
            expiry: StudioExpiry::At(0),
        },
        2,
        10,
    );
    b.put(ROOT, small_key, small_bytes).unwrap();
    let (a, _) = converge(a, b);
    let entry = &read(&a).objects[&[3; 16]];
    assert_eq!(entry.creations[0].source, small);
    assert_eq!(entry.creations.len(), 2);
    assert_eq!(entry.title.selected.value, "renamed");
    assert_eq!(entry.expiry.selected.value, StudioExpiry::At(0));
}

#[test]
fn deletions_win_before_after_and_concurrently_without_losing_authors_or_hidden_content() {
    let base = root(1);
    let mut a = base.clone().with_actor(ActorId::from(vec![2; 32]));
    let mut b = base.with_actor(ActorId::from(vec![3; 32]));
    let da = write(&mut a, &IndexOp::TombstoneObject { object: [3; 16] }, 2, 1);
    a.commit();
    assert_eq!(read(&a).tombstones[&[3; 16]], vec![da.clone()]);
    assert!(read(&a).deleted_objects.is_empty());
    write(&mut b, &put(3, 3, "hidden"), 3, 1);
    write(
        &mut b,
        &IndexOp::SetTitle {
            object: [3; 16],
            title: "hidden rename".into(),
        },
        3,
        2,
    );
    let db = write(&mut b, &IndexOp::TombstoneObject { object: [3; 16] }, 3, 3);
    let (mut a, _) = converge(a, b);
    let p = read(&a);
    assert!(p.objects.is_empty());
    assert!(p.overflow.is_empty());
    assert_eq!(
        p.deleted_objects[&[3; 16]].title.selected.value,
        "hidden rename"
    );
    let mut expected = vec![da, db];
    expected.sort_by_key(|source| source.op_id);
    assert_eq!(p.tombstones[&[3; 16]], expected);
    write(&mut a, &put(3, 2, "cannot resurrect"), 2, 3);
    a.commit();
    assert!(read(&a).objects.is_empty());
    assert_eq!(read(&a).deleted_objects[&[3; 16]].creations.len(), 2);
}

#[test]
fn sixty_five_objects_merge_to_sixty_four_visible_and_explicit_overflow_then_reclaim() {
    let mut a = root(1);
    let mut b = root(2);
    for id in 0..=64u8 {
        let (doc, author) = if id % 2 == 0 {
            (&mut a, 1)
        } else {
            (&mut b, 2)
        };
        write(doc, &put(id, author, "entry"), author, u128::from(id));
    }
    let (mut a, _) = converge(a, b);
    let p = read(&a);
    assert_eq!(p.objects.len(), MAX_INDEX_OBJECTS);
    assert_eq!(
        p.objects.keys().copied().collect::<Vec<_>>(),
        (0..64).map(|id| [id; 16]).collect::<Vec<_>>()
    );
    assert_eq!(
        p.overflow.keys().copied().collect::<Vec<_>>(),
        vec![[64; 16]]
    );
    assert_eq!(p.overflow[&[64; 16]].title.selected.value, "entry");
    write(
        &mut a,
        &IndexOp::TombstoneObject { object: [0; 16] },
        1,
        100,
    );
    a.commit();
    let p = read(&a);
    assert_eq!(p.objects.len(), MAX_INDEX_OBJECTS);
    assert!(p.objects.contains_key(&[64; 16]));
    assert!(p.overflow.is_empty());
    assert!(p.deleted_objects.contains_key(&[0; 16]));
}

#[test]
fn scope_checks_cover_losing_headers_and_all_roots_not_only_automerge_winners() {
    for (key, value) in [
        ("v", ScalarValue::Uint(2)),
        ("kind", ScalarValue::Str("flipnote".into())),
        ("channel", ScalarValue::Str(hex(&[8; 16]).into())),
        ("epoch", ScalarValue::Uint(1)),
    ] {
        let mut good = root(255);
        let mut bad = empty(0);
        // Independent initializations have equal counters and different actors. Re-putting an
        // unchanged inherited value is a no-op in Automerge, so would not create this conflict.
        for (property, expected) in header(&logical(), 0) {
            let supplied = if property == key {
                value.clone()
            } else {
                expected
            };
            bad.put(ROOT, property, supplied).unwrap();
        }
        bad.commit();
        good.apply_changes(bad.get_changes(&[])).unwrap();
        assert_eq!(good.get_all(ROOT, key).unwrap().len(), 2);
        assert!(scalar_eq(
            &good.get(ROOT, key).unwrap().unwrap().0,
            &header(&logical(), 0)[key]
        ));
        assert!(StudioIndexProjection::read(&logical(), 0, &good).is_err());
    }
}

#[test]
fn malformed_records_paths_orphan_fields_and_wrong_scalar_types_reject() {
    let (key, bytes, _) = record(&put(3, 1, "private title"), 1, 1);
    let mut variants = Vec::new();
    let mut version = bytes.clone();
    version[0] = 2;
    variants.push((key.clone(), ScalarValue::Bytes(version)));
    variants.push((key.clone(), ScalarValue::Bytes(bytes[..32].to_vec())));
    let mut bad_author = bytes.clone();
    bad_author[1] ^= 1;
    variants.push((key.clone(), ScalarValue::Bytes(bad_author)));
    variants.push((
        format!("i/{}/{}", hex(&[4; 16]), &key[35..]),
        ScalarValue::Bytes(bytes.clone()),
    ));
    variants.push((format!("{key}0"), ScalarValue::Bytes(bytes.clone())));
    variants.push((key.to_uppercase(), ScalarValue::Bytes(bytes.clone())));
    variants.push((
        format!("i/{}/{}", hex(&[3; 16]), hex(&[0; 32])),
        ScalarValue::Bytes(bytes.clone()),
    ));
    variants.push((
        format!("t/{}", hex(&[3; 16])),
        ScalarValue::Bytes(bytes.clone()),
    ));
    variants.push((key.clone(), ScalarValue::Str("not bytes".into())));
    variants.push(("unknown".into(), ScalarValue::Uint(1)));
    variants.push((format!("d/{}", hex(&[3; 16])), ScalarValue::Boolean(true)));
    variants.push((format!("_p1/op/{}", hex(&[0; 32])), ScalarValue::Int(1)));
    variants.push((format!("_p1/op/{}", hex(&[0; 32])), ScalarValue::Uint(2)));
    variants.push(("_p1/op/not-an-id".into(), ScalarValue::Uint(1)));
    for (property, value) in variants {
        let mut doc = root(1);
        doc.put(ROOT, property, value).unwrap();
        doc.commit();
        assert!(StudioIndexProjection::read(&logical(), 0, &doc).is_err());
    }
    for operation in [
        IndexOp::SetTitle {
            object: [3; 16],
            title: "orphan".into(),
        },
        IndexOp::SetExpiry {
            object: [3; 16],
            expiry: StudioExpiry::Unrecorded,
        },
    ] {
        let mut doc = root(1);
        write(&mut doc, &operation, 1, 1);
        doc.commit();
        assert!(StudioIndexProjection::read(&logical(), 0, &doc).is_err());
    }
    let mut doc = root(1);
    doc.put_object(ROOT, key, ObjType::Map).unwrap();
    assert!(StudioIndexProjection::read(&logical(), 0, &doc).is_err());
    for change in ["type", "key"] {
        let mut domain = DomainOp::decode(&bytes[33..]).unwrap();
        if change == "type" {
            domain.doc_type = DocType::StudioObject;
        } else {
            domain.logical_key[0] ^= 1;
        }
        let mut forged = bytes[..33].to_vec();
        forged.extend_from_slice(&domain.encode().unwrap());
        assert!(decode_record(&logical(), &forged).is_err());
    }
}

#[test]
fn exact_duplicate_records_collapse_but_nonce_equivocation_rejects_in_both_orders() {
    for (object, title, valid) in [
        (3, "first", true),
        (3, "different", false),
        (4, "first", false),
    ] {
        let base = root(1);
        let mut a = base.clone().with_actor(ActorId::from(vec![2; 32]));
        let mut b = base.with_actor(ActorId::from(vec![3; 32]));
        // A checkpoint/replay can produce byte-identical records under different AM actors.
        write(&mut a, &put(3, 1, "first"), 1, 9);
        write(&mut b, &put(object, 1, title), 1, 9);
        a.commit();
        b.commit();
        let ca = a.get_changes(&[]);
        let cb = b.get_changes(&[]);
        a.apply_changes(cb).unwrap();
        b.apply_changes(ca).unwrap();
        for doc in [&a, &b] {
            let result = StudioIndexProjection::read(&logical(), 0, doc);
            if valid {
                assert_eq!(result.unwrap().objects[&[3; 16]].creations.len(), 1);
            } else {
                assert!(result.is_err());
            }
        }
    }
}

#[test]
fn losing_deleted_and_overflow_records_count_against_the_full_reader_budget() {
    let mut budget = ReadBudget {
        used: 0,
        limit: MAX_INDEX_READ_BYTES,
    };
    budget.add(MAX_INDEX_READ_BYTES).unwrap();
    assert!(budget.add(1).is_err());
    assert!(ReadBudget {
        used: usize::MAX,
        limit: usize::MAX
    }
    .add(1)
    .is_err());
    let mut oversized = root(1);
    oversized
        .put(ROOT, "x".repeat(MAX_ROOT_KEY_BYTES + 1), 1u64)
        .unwrap();
    assert!(matches!(
        StudioIndexProjection::read(&logical(), 0, &oversized),
        Err(ReplError::EpochBound)
    ));
    assert!(decode_record(&logical(), &vec![0; MAX_RECORD_BYTES + 1]).is_err());
    // The production cap's inclusive boundary is pinned above. Use the private lower-limit
    // seam for real-CRDT accounting: a multi-MiB Automerge commit makes this test unnecessarily
    // expensive, while the same admission predicate can be exercised at each fixture's exact
    // byte/primitive count. In particular, NO losing, deleted or over-slot record can be skipped.
    for mode in ["deleted", "collision", "overflow"] {
        let mut doc = root(1);
        for n in 0..65u8 {
            let id = if mode == "collision" { 0 } else { n };
            write(&mut doc, &put(id, 1, "entry"), 1, u128::from(n));
            if mode == "deleted" {
                write(
                    &mut doc,
                    &IndexOp::TombstoneObject { object: [id; 16] },
                    1,
                    u128::from(n) + 1000,
                );
            }
        }
        doc.commit();
        let exact = reader_bytes(&doc);
        let primitives = doc.stats().num_ops;
        assert_eq!(
            StudioIndexProjection::read_bounded(&logical(), 0, &doc, primitives, exact).unwrap(),
            read(&doc)
        );
        assert!(matches!(
            StudioIndexProjection::read_bounded(&logical(), 0, &doc, primitives, exact - 1),
            Err(ReplError::EpochBound)
        ));
        assert!(matches!(
            StudioIndexProjection::read_bounded(&logical(), 0, &doc, primitives - 1, exact),
            Err(ReplError::EpochBound)
        ));
    }
}

// Independent accounting assertion for the physical format; do not call ReadBudget::value here.
fn reader_bytes(doc: &AutoCommit) -> usize {
    doc.keys(ROOT)
        .map(|key| {
            key.len()
                + doc
                    .get_all(ROOT, &key)
                    .unwrap()
                    .into_iter()
                    .map(|(value, _)| {
                        let Value::Scalar(value) = value else {
                            panic!("fixture must be flat");
                        };
                        match value.as_ref() {
                            ScalarValue::Bytes(bytes) => bytes.len(),
                            ScalarValue::Str(text) => text.len(),
                            ScalarValue::Uint(_) => 8,
                            _ => panic!("unexpected fixture scalar"),
                        }
                    })
                    .sum::<usize>()
        })
        .sum()
}

#[test]
fn save_load_preserves_projection_and_debug_never_prints_private_content() {
    let mut doc = root(1);
    write(&mut doc, &put(3, 1, "secret name"), 1, 1);
    doc.commit();
    let p = read(&doc);
    let bytes = doc.save();
    let loaded = AutoCommit::load(&bytes).unwrap();
    assert_eq!(p, read(&loaded));
    // Also reconstruct from actual changes, the form the future P1 signed log restores.
    let changes: Vec<_> = doc
        .get_changes(&[])
        .into_iter()
        .map(|c| Change::from_bytes(c.raw_bytes().to_vec()).unwrap())
        .collect();
    let mut rebuilt = empty(9);
    rebuilt.apply_changes(changes).unwrap();
    assert_eq!(p, read(&rebuilt));
    let entry = &p.objects[&[3; 16]];
    for debug in [
        format!("{p:?}"),
        format!("{entry:?}"),
        format!("{:?}", entry.title),
        format!("{:?}", entry.title.selected),
        format!("{:?}", entry.creations[0].value),
        format!("{:?}", entry.creations[0].source),
    ] {
        assert!(!debug.contains("secret"));
        assert!(!debug.contains(&hex(&[1; 32])));
        assert!(!debug.contains(&hex(&[3; 16])));
    }
}
