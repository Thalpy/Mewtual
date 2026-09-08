use super::*;
use automerge::transaction::Transactable;
use automerge::{ActorId, Change, ObjType};

const CHANNEL: ElementId = [7; 16];
fn logical() -> LogicalDocument {
    flipnote_document(b"frame-projection-test", [9; 16]).unwrap()
}
fn id(n: u128) -> ElementId {
    n.to_be_bytes()
}
fn empty(actor: u8) -> AutoCommit {
    AutoCommit::new().with_actor(ActorId::from(vec![actor; 32]))
}
fn root(actor: u8) -> AutoCommit {
    let mut doc = empty(actor);
    for (key, value) in header(&logical(), CHANNEL, 0) {
        doc.put(ROOT, key, value).unwrap();
    }
    doc.commit();
    doc
}
fn insert(frame: u128, after: Option<u128>, bytes: u64) -> FlipnoteOp {
    FlipnoteOp::InsertFrame {
        frame: id(frame),
        after: after.map(id),
        cid: [frame as u8; 32],
        bytes,
    }
}
// Test-only unchecked authoring. These record claims do NOT replace the future causal validator.
fn record(
    op: &FlipnoteOp,
    author: u8,
    nonce: u128,
    anchor: Option<OpId>,
    before: Option<OpId>,
) -> (String, Vec<u8>, FrameSource) {
    let author = DeviceId::from_bytes([author; 32]);
    let domain = DomainOp {
        nonce: nonce.to_be_bytes(),
        doc_type: DocType::StudioObject,
        logical_key: logical().logical_key,
        body: op.encode().unwrap(),
    };
    let source = FrameSource {
        op_id: domain.id(&author),
        author,
        nonce: domain.nonce,
        ts: 1234,
    };
    let key = match op {
        FlipnoteOp::InsertFrame { frame, .. } => format!("i/{}/{}", hex(frame), hex(&source.op_id)),
        FlipnoteOp::RemoveFrame { frame } => format!("d/{}/{}", hex(frame), hex(&source.op_id)),
        FlipnoteOp::ReplaceFrame { frame, .. } => format!("r/{}", hex(frame)),
        FlipnoteOp::SetHeader(FlipnoteHeader::Title(_)) => "h/title".into(),
        FlipnoteOp::SetHeader(FlipnoteHeader::Fps(_)) => "h/fps".into(),
        _ => "h/unsupported".into(),
    };
    let mut bytes = vec![1];
    bytes.extend_from_slice(source.author.as_bytes());
    bytes.extend_from_slice(&source.ts.to_be_bytes());
    for origin in [anchor, before] {
        bytes.push(u8::from(origin.is_some()));
        if let Some(origin) = origin {
            bytes.extend_from_slice(&origin);
        }
    }
    bytes.extend_from_slice(&domain.encode().unwrap());
    (key, bytes, source)
}
fn write(
    doc: &mut AutoCommit,
    op: &FlipnoteOp,
    author: u8,
    nonce: u128,
    anchor: Option<OpId>,
    before: Option<OpId>,
) -> FrameSource {
    let (key, bytes, source) = record(op, author, nonce, anchor, before);
    doc.put(ROOT, key, bytes).unwrap();
    doc.put(ROOT, format!("_p1/op/{}", hex(&source.op_id)), 1u64)
        .unwrap();
    source
}
fn read(doc: &AutoCommit) -> FlipnoteFrameProjection {
    FlipnoteFrameProjection::read(&logical(), CHANNEL, 0, doc).unwrap()
}
fn rejected(doc: &AutoCommit) {
    assert!(FlipnoteFrameProjection::read(&logical(), CHANNEL, 0, doc).is_err());
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
fn frame_record_framing_scope_and_noninsertion_metadata_are_exact() {
    let operation = insert(2, Some(1), 10);
    let (_, bytes, source) = record(&operation, 1, 1, Some([2; 32]), Some([3; 32]));
    // Fixed known prefix, independent of the record writer's metadata encoding calls.
    let mut prefix = vec![1; 33]; // version 1 followed by author 01...01
    prefix.extend_from_slice(&[0, 0, 0, 0, 0, 0, 4, 210]); // 1234 ms, big-endian
    prefix.push(1);
    prefix.extend_from_slice(&[2; 32]);
    prefix.push(1);
    prefix.extend_from_slice(&[3; 32]);
    assert_eq!(&bytes[..107], prefix);
    let domain = DomainOp::decode(&bytes[107..]).unwrap();
    assert_eq!(domain.nonce, 1u128.to_be_bytes());
    assert_eq!(domain.body, operation.encode().unwrap());
    assert_eq!(domain.encode().unwrap(), bytes[107..]);
    assert_eq!(decode_record(&logical(), &bytes).unwrap().source, source);
    for change in ["type", "key", "trailing"] {
        let mut domain = domain.clone();
        match change {
            "type" => domain.doc_type = DocType::StudioIndex,
            "key" => domain.logical_key[0] ^= 1,
            _ => {}
        }
        let mut altered = prefix.clone();
        altered.extend_from_slice(&domain.encode().unwrap());
        if change == "trailing" {
            altered.push(0);
        }
        assert!(decode_record(&logical(), &altered).is_err());
    }
    for operation in [
        FlipnoteOp::RemoveFrame { frame: id(1) },
        FlipnoteOp::SetHeader(FlipnoteHeader::Fps(12)),
    ] {
        for (anchor, before) in [(Some([2; 32]), None), (None, Some([3; 32]))] {
            let (_, bytes, _) = record(&operation, 1, 1, anchor, before);
            assert!(decode_record(&logical(), &bytes).is_err());
        }
    }
}

#[test]
fn frames_require_exact_scope_and_pristine_epoch_zero_even_after_all_keys_are_erased() {
    assert!(read(&empty(1)).timeline.is_empty());
    assert!(FlipnoteFrameProjection::read(&logical(), CHANNEL, 1, &empty(1)).is_err());
    assert_eq!(read(&root(1)).document(), &logical());
    let mut wrong = logical();
    wrong.doc_type = DocType::StudioIndex;
    assert!(FlipnoteFrameProjection::read(&wrong, CHANNEL, 0, &root(1)).is_err());
    wrong = logical();
    wrong.logical_key.push(0);
    assert!(FlipnoteFrameProjection::read(&wrong, CHANNEL, 0, &root(1)).is_err());
    assert!(FlipnoteFrameProjection::read(&logical(), [8; 16], 0, &root(1)).is_err());
    for content in [false, true] {
        let mut doc = root(1);
        if content {
            write(&mut doc, &insert(1, None, 10), 1, 1, None, None);
            doc.commit();
        }
        for key in doc.keys(ROOT).collect::<Vec<_>>() {
            doc.delete(ROOT, key).unwrap();
        }
        doc.commit();
        assert!(doc.stats().num_ops > 0);
        rejected(&doc);
    }
    for (key, bad) in [
        ("v", ScalarValue::Uint(2)),
        ("kind", ScalarValue::Str("score".into())),
        ("id", ScalarValue::Str(hex(&[8; 16]).into())),
        ("channel", ScalarValue::Str(hex(&[8; 16]).into())),
        ("epoch", ScalarValue::Uint(1)),
        ("w", ScalarValue::Uint(128)),
        ("h", ScalarValue::Uint(160)),
    ] {
        let mut good = root(255);
        let mut other = empty(0);
        for (property, expected) in header(&logical(), CHANNEL, 0) {
            other
                .put(
                    ROOT,
                    &property,
                    if property == key {
                        bad.clone()
                    } else {
                        expected
                    },
                )
                .unwrap();
        }
        other.commit();
        good.apply_changes(other.get_changes(&[])).unwrap();
        assert_eq!(good.get_all(ROOT, key).unwrap().len(), 2);
        assert!(scalar_eq(
            &good.get(ROOT, key).unwrap().unwrap().0,
            &header(&logical(), CHANNEL, 0)[key]
        ));
        rejected(&good);
        let mut missing = root(1);
        missing.delete(ROOT, key).unwrap();
        rejected(&missing);
    }
}

#[test]
fn prepend_middle_insert_append_and_header_registers_preserve_local_placement() {
    let mut doc = root(1);
    let a = write(&mut doc, &insert(1, None, 10), 1, 1, None, None);
    doc.commit();
    let b = write(&mut doc, &insert(2, Some(1), 20), 1, 2, Some(a.op_id), None);
    doc.commit();
    let c = write(
        &mut doc,
        &insert(3, Some(1), 30),
        1,
        3,
        Some(a.op_id),
        Some(b.op_id),
    );
    doc.commit();
    assert_eq!(read(&doc).timeline, vec![id(1), id(3), id(2)]);
    let first = write(&mut doc, &insert(4, None, 40), 1, 4, None, Some(a.op_id));
    doc.commit();
    // Restore with a missing predecessor normalizes to the explicit current last frame id.
    write(&mut doc, &insert(5, Some(2), 50), 1, 5, Some(b.op_id), None);
    let title = write(
        &mut doc,
        &FlipnoteOp::SetHeader(FlipnoteHeader::Title("Moon 🐱".into())),
        1,
        6,
        None,
        None,
    );
    write(
        &mut doc,
        &FlipnoteOp::SetHeader(FlipnoteHeader::Fps(24)),
        1,
        7,
        None,
        None,
    );
    doc.commit();
    let p = read(&doc);
    assert_eq!(p.timeline, [4, 1, 3, 2, 5].map(id));
    assert_eq!(p.title.as_ref().unwrap().selected.value, "Moon 🐱");
    assert_eq!(p.title.as_ref().unwrap().selected.source, title);
    assert_eq!(p.fps.as_ref().unwrap().selected.value, 24);
    assert_eq!(p.declared_frame_bytes, 150);
    assert!(p.over_cap.is_empty());
    assert_eq!(p.frames[&id(3)].insertions[0].source, c);
    assert_eq!(p.frames[&id(4)].insertions[0].source, first);
}

#[test]
fn concurrent_same_and_different_gaps_converge_and_later_insert_does_not_swap_siblings() {
    let mut base = root(1);
    let parent = write(&mut base, &insert(1, None, 1), 1, 1, None, None);
    base.commit();
    let mut a = base.clone().with_actor(ActorId::from(vec![2; 32]));
    let mut b = base.with_actor(ActorId::from(vec![3; 32]));
    let x = write(
        &mut a,
        &insert(2, Some(1), 1),
        2,
        1,
        Some(parent.op_id),
        None,
    );
    let y = write(
        &mut b,
        &insert(3, Some(1), 1),
        3,
        1,
        Some(parent.op_id),
        None,
    );
    let (mut merged, _) = converge(a, b);
    let (left, right, left_source, right_source) = if x.op_id < y.op_id {
        (2, 3, x, y)
    } else {
        (3, 2, y, x)
    };
    assert_eq!(read(&merged).timeline, [1, left, right].map(id));
    // Force C's rank to exceed both existing children. A smallest-id Kahn walk would incorrectly
    // emit right,C,left; the right-origin forest must preserve C,left,right instead.
    let nonce = (2..1000)
        .find(|nonce| {
            record(
                &insert(4, Some(1), 1),
                2,
                *nonce,
                Some(parent.op_id),
                Some(left_source.op_id),
            )
            .2
            .op_id
                > right_source.op_id
        })
        .unwrap();
    write(
        &mut merged,
        &insert(4, Some(1), 1),
        2,
        nonce,
        Some(parent.op_id),
        Some(left_source.op_id),
    );
    merged.commit();
    assert_eq!(read(&merged).timeline, [1, 4, left, right].map(id));
    let mut a = merged.clone().with_actor(ActorId::from(vec![4; 32]));
    let mut b = merged.with_actor(ActorId::from(vec![5; 32]));
    write(
        &mut a,
        &insert(5, Some(left), 1),
        4,
        2,
        Some(left_source.op_id),
        None,
    );
    write(
        &mut b,
        &insert(6, Some(right), 1),
        5,
        2,
        Some(right_source.op_id),
        None,
    );
    let (a, _) = converge(a, b);
    assert_eq!(read(&a).timeline, [1, 4, left, 5, right, 6].map(id));
}

#[test]
fn concurrent_different_right_origins_under_one_parent_preserve_each_observed_gap() {
    let mut base = root(1);
    let parent = write(&mut base, &insert(1, None, 1), 1, 1, None, None);
    base.commit();
    let mut a = base.clone().with_actor(ActorId::from(vec![2; 32]));
    let mut b = base.with_actor(ActorId::from(vec![3; 32]));
    // Each branch has observed a different first child, not merely a different parent.
    let x = write(
        &mut a,
        &insert(2, Some(1), 1),
        2,
        1,
        Some(parent.op_id),
        None,
    );
    a.commit();
    write(
        &mut a,
        &insert(4, Some(1), 1),
        2,
        2,
        Some(parent.op_id),
        Some(x.op_id),
    );
    let y = write(
        &mut b,
        &insert(3, Some(1), 1),
        3,
        1,
        Some(parent.op_id),
        None,
    );
    b.commit();
    write(
        &mut b,
        &insert(5, Some(1), 1),
        3,
        2,
        Some(parent.op_id),
        Some(y.op_id),
    );
    assert_eq!(read(&a).timeline, [1, 4, 2].map(id));
    assert_eq!(read(&b).timeline, [1, 5, 3].map(id));
    let (a, _) = converge(a, b);
    let expected = if x.op_id < y.op_id {
        [1, 4, 2, 5, 3]
    } else {
        [1, 5, 3, 4, 2]
    };
    assert_eq!(read(&a).timeline, expected.map(id));
}

#[test]
fn anchor_metadata_equivocation_rejects_even_when_both_isolated_origin_choices_are_valid() {
    let base = root(1);
    let mut a = base.clone().with_actor(ActorId::from(vec![2; 32]));
    let mut b = base.with_actor(ActorId::from(vec![3; 32]));
    let first = write(&mut a, &insert(1, None, 1), 2, 1, None, None);
    let other = write(&mut b, &insert(1, None, 1), 3, 1, None, None);
    a.commit();
    b.commit();
    // An equivocating identity reuses a child's body/nonce but resolves its predecessor to a
    // different insertion of the SAME stable frame id. Neither isolated reader can reject it
    // for a missing/wrong-frame origin; only complete-record equivocation catches the merge.
    let child = insert(2, Some(1), 1);
    let one = write(&mut a, &child, 9, 10, Some(first.op_id), None);
    let two = write(&mut b, &child, 9, 10, Some(other.op_id), None);
    a.commit();
    b.commit();
    assert_eq!(one, two);
    assert_eq!(read(&a).timeline, [1, 2].map(id));
    assert_eq!(read(&b).timeline, [1, 2].map(id));
    let ca = a.get_changes(&[]);
    let cb = b.get_changes(&[]);
    a.apply_changes(cb).unwrap();
    b.apply_changes(ca).unwrap();
    rejected(&a);
    rejected(&b);
}

#[test]
fn colliding_frame_ids_keep_both_anchor_branches_without_winner_rewiring_cycles() {
    let mut a = root(1);
    let mut b = root(2);
    let a1 = write(&mut a, &insert(1, None, 10), 1, 1, None, None);
    write(&mut a, &insert(2, Some(1), 20), 1, 2, Some(a1.op_id), None);
    a.commit();
    let b2 = write(&mut b, &insert(2, None, 30), 2, 1, None, None);
    write(&mut b, &insert(1, Some(2), 40), 2, 2, Some(b2.op_id), None);
    b.commit();
    let (mut a, _) = converge(a, b);
    let p = read(&a);
    assert_eq!(p.timeline.len(), 2);
    assert_eq!(p.insertion_order().len(), 4);
    for frame in [id(1), id(2)] {
        assert_eq!(p.frames[&frame].insertions.len(), 2);
        assert!(
            p.frames[&frame].insertions[0].source.op_id
                < p.frames[&frame].insertions[1].source.op_id
        );
    }
    let winner = p.frames[&id(1)].insertions[0].source.op_id;
    let later = write(&mut a, &insert(3, Some(1), 50), 1, 10, Some(winner), None);
    a.commit();
    let deletion = write(
        &mut a,
        &FlipnoteOp::RemoveFrame { frame: id(1) },
        1,
        11,
        None,
        None,
    );
    a.commit();
    let p = read(&a);
    assert!(!p.timeline.contains(&id(1)));
    assert!(p.timeline.contains(&id(3)));
    assert_eq!(p.frames[&id(1)].insertions.len(), 2);
    assert_eq!(p.tombstones[&id(1)], vec![deletion]);
    assert_eq!(p.frames[&id(3)].insertions[0].source, later);
    // Every original blob declaration is still present, even when neither selected nor visible.
    let evidence: BTreeSet<_> = p
        .frames
        .values()
        .flat_map(|entry| entry.insertions.iter().map(|v| v.value.blob.bytes))
        .collect();
    assert_eq!(evidence, BTreeSet::from([10, 20, 30, 40, 50]));
}

#[test]
fn replacement_conflicts_use_actual_automerge_winner_and_survive_concurrent_deletion() {
    let mut base = root(1);
    write(&mut base, &insert(1, None, 10), 1, 1, None, None);
    base.commit();
    let mut a = base.clone().with_actor(ActorId::from(vec![2; 32]));
    let mut b = base.with_actor(ActorId::from(vec![3; 32]));
    for (doc, author, bytes) in [(&mut a, 2, 20), (&mut b, 3, 30)] {
        write(
            doc,
            &FlipnoteOp::ReplaceFrame {
                frame: id(1),
                cid: [author; 32],
                bytes,
            },
            author,
            2,
            None,
            None,
        );
        write(
            doc,
            &FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!("title {author}"))),
            author,
            3,
            None,
            None,
        );
        write(
            doc,
            &FlipnoteOp::SetHeader(FlipnoteHeader::Fps(author)),
            author,
            4,
            None,
            None,
        );
    }
    let (mut a, mut b) = converge(a, b);
    let p = read(&a);
    let entry = &p.frames[&id(1)];
    assert_eq!(entry.pixels.conflicts.len(), 1);
    let (v, _) = a.get(ROOT, format!("r/{}", hex(&id(1)))).unwrap().unwrap();
    let selected = decode_record(&logical(), record_bytes(&v).unwrap()).unwrap();
    assert_eq!(entry.pixels.selected.source, selected.source);
    assert_eq!(p.declared_frame_bytes, entry.pixels.selected.value.bytes);
    assert_eq!(p.title.as_ref().unwrap().conflicts.len(), 1);
    assert_eq!(p.fps.as_ref().unwrap().conflicts.len(), 1);
    write(
        &mut a,
        &FlipnoteOp::RemoveFrame { frame: id(1) },
        2,
        5,
        None,
        None,
    );
    write(
        &mut b,
        &FlipnoteOp::ReplaceFrame {
            frame: id(1),
            cid: [4; 32],
            bytes: 40,
        },
        3,
        5,
        None,
        None,
    );
    let (a, _) = converge(a, b);
    let p = read(&a);
    assert!(p.timeline.is_empty());
    assert_eq!(p.declared_frame_bytes, 0);
    assert_eq!(p.frames[&id(1)].insertions[0].value.blob.bytes, 10);
    assert_eq!(p.frames[&id(1)].pixels.selected.value.bytes, 40);
    assert!(p.frames[&id(1)].pixels.conflicts.is_empty());
}

#[test]
fn frame_count_and_summed_declarations_flag_exact_prefix_boundaries_without_fetching() {
    for size in [1, super::super::MAX_FRAME_BYTES] {
        let count: u128 = if size == 1 { 1000 } else { 130 };
        let mut doc = root(1);
        let mut anchor = None;
        for n in 0..count {
            let bytes = if size > 1 && n >= 128 { 1 } else { size };
            let source = write(
                &mut doc,
                &insert(n, n.checked_sub(1), bytes),
                1,
                n,
                anchor,
                None,
            );
            anchor = Some(source.op_id);
        }
        doc.commit();
        let p = read(&doc);
        assert_eq!(p.timeline.len(), count as usize);
        if size == 1 {
            assert_eq!(p.over_cap.len(), 1);
            assert_eq!(
                p.over_cap[&id(999)],
                FrameLimits {
                    count: true,
                    bytes: false
                }
            );
            assert_eq!(p.declared_frame_bytes, 1000);
            assert!(p.frames.contains_key(&id(999)));
        } else {
            assert_eq!(p.over_cap.len(), 2);
            assert_eq!(
                p.over_cap[&id(128)],
                FrameLimits {
                    count: false,
                    bytes: true
                }
            );
            assert!(p.over_cap.contains_key(&id(129))); // never pack a small later frame around excess
            assert_eq!(p.declared_frame_bytes, FLIPNOTE_FRAME_BYTES + 2);
        }
        write(
            &mut doc,
            &FlipnoteOp::RemoveFrame { frame: id(0) },
            1,
            2000,
            None,
            None,
        );
        doc.commit();
        let p = read(&doc);
        assert!(p.over_cap.is_empty());
        assert_eq!(p.timeline.len(), count as usize - 1);
        assert!(p.frames.contains_key(&id(0))); // removal changes playback, not retained evidence
    }
}

#[test]
fn missing_wrong_parent_self_and_cyclic_origins_reject_instead_of_stranding_frames() {
    let mut valid = root(1);
    let a = write(&mut valid, &insert(1, None, 1), 1, 1, None, None);
    let b = write(
        &mut valid,
        &insert(2, Some(1), 1),
        1,
        2,
        Some(a.op_id),
        None,
    );
    valid.commit();
    for (op, anchor, before) in [
        (insert(3, Some(1), 1), None, None),
        (insert(3, None, 1), Some(a.op_id), None),
        (insert(3, Some(1), 1), Some([0; 32]), None),
        (insert(3, Some(9), 1), Some(a.op_id), None),
        (insert(3, None, 1), None, Some([0; 32])),
        (insert(3, None, 1), None, Some(b.op_id)),
    ] {
        let mut doc = valid.clone();
        write(&mut doc, &op, 1, 3, anchor, before);
        doc.commit();
        rejected(&doc);
    }
    for kind in ["parent", "right", "self"] {
        let mut doc = root(1);
        let op_a = insert(1, if kind == "parent" { Some(2) } else { None }, 1);
        let op_b = insert(2, if kind == "parent" { Some(1) } else { None }, 1);
        let a = record(&op_a, 1, 1, None, None).2.op_id;
        let b = record(&op_b, 1, 2, None, None).2.op_id;
        let (anchor_a, before_a, anchor_b, before_b) = match kind {
            "parent" => (Some(b), None, Some(a), None),
            "right" => (None, Some(b), None, Some(a)),
            _ => (None, Some(a), None, None),
        };
        write(&mut doc, &op_a, 1, 1, anchor_a, before_a);
        write(&mut doc, &op_b, 1, 2, anchor_b, before_b);
        doc.commit();
        rejected(&doc);
    }
}

#[test]
fn malformed_and_unsupported_content_rejects_including_empty_score_and_orphan_replacements() {
    let (key, bytes, _) = record(&insert(1, None, 1), 1, 1, None, None);
    let mut version = bytes.clone();
    version[0] = 2;
    let mut timestamp = bytes.clone();
    timestamp[33..41].copy_from_slice(&u64::MAX.to_be_bytes());
    let mut flag = bytes.clone();
    flag[41] = 2;
    let mut author = bytes.clone();
    author[1] ^= 1;
    for (key, bytes) in [
        (key.clone(), version),
        (key.clone(), timestamp),
        (key.clone(), flag),
        (key.clone(), author),
        (key.clone(), bytes[..42].to_vec()),
        (key.clone(), vec![0; MAX_RECORD_BYTES + 1]),
        (key.to_uppercase(), bytes.clone()),
        (format!("{key}/extra"), bytes.clone()),
        (format!("i/{}/{}", hex(&id(2)), &key[35..]), bytes.clone()),
        ("h/title".into(), bytes.clone()),
    ] {
        let mut doc = root(1);
        doc.put(ROOT, key, bytes).unwrap();
        doc.commit();
        rejected(&doc);
    }
    for op in [
        FlipnoteOp::SetHeader(FlipnoteHeader::Score(None)),
        FlipnoteOp::SetSfx {
            sfx: id(1),
            frame: id(1),
            patch: [1; 32],
            note: 60,
        },
        FlipnoteOp::RemoveSfx { sfx: id(1) },
        FlipnoteOp::RemovePatch { patch: [1; 32] },
        FlipnoteOp::SetExport {
            export: id(1),
            cid: [1; 32],
            bytes: 1,
            expiry: super::super::StudioExpiry::Never,
        },
        FlipnoteOp::RemoveExport { export: id(1) },
    ] {
        let (_, bytes, _) = record(&op, 1, 1, None, None);
        for key in ["h/score", "h/title", "sfx", "exports", "patches"] {
            let mut doc = root(1);
            doc.put(ROOT, key, bytes.clone()).unwrap();
            doc.commit();
            rejected(&doc);
        }
    }
    let mut doc = root(1);
    write(
        &mut doc,
        &FlipnoteOp::ReplaceFrame {
            frame: id(1),
            cid: [1; 32],
            bytes: 1,
        },
        1,
        1,
        None,
        None,
    );
    doc.commit();
    rejected(&doc);
    let mut doc = root(1);
    doc.put_object(ROOT, &key, ObjType::Map).unwrap();
    rejected(&doc);
    for v in [
        ScalarValue::Int(1),
        ScalarValue::Uint(2),
        ScalarValue::Boolean(true),
    ] {
        let mut doc = root(1);
        doc.put(ROOT, format!("_p1/op/{}", hex(&[1; 32])), v)
            .unwrap();
        rejected(&doc);
    }
    let mut doc = root(1);
    doc.put(ROOT, "unknown", 1u64).unwrap();
    rejected(&doc);
}

#[test]
fn same_id_retries_collapse_but_metadata_and_body_equivocation_reject_in_both_orders() {
    let mut base = root(1);
    let parent = write(&mut base, &insert(1, None, 1), 1, 1, None, None);
    base.commit();
    for mutation in ["none", "time", "before", "anchor", "body"] {
        let mut a = base.clone().with_actor(ActorId::from(vec![2; 32]));
        let mut b = base.clone().with_actor(ActorId::from(vec![3; 32]));
        let op = insert(2, None, 1);
        let (key, original, _) = record(&op, 2, 1, None, None);
        let changed = match mutation {
            "time" => {
                let mut changed = original.clone();
                changed[40] ^= 1;
                changed
            }
            "before" => record(&op, 2, 1, None, Some(parent.op_id)).1,
            "anchor" => record(&op, 2, 1, Some(parent.op_id), None).1,
            "body" => record(&insert(2, None, 2), 2, 1, None, None).1,
            _ => original.clone(),
        };
        a.put(ROOT, &key, original).unwrap();
        b.put(ROOT, &key, changed).unwrap();
        a.commit();
        b.commit();
        let ca = a.get_changes(&[]);
        let cb = b.get_changes(&[]);
        a.apply_changes(cb).unwrap();
        b.apply_changes(ca).unwrap();
        for doc in [&a, &b] {
            if mutation == "none" {
                assert_eq!(read(doc).frames[&id(2)].insertions.len(), 1);
            } else {
                rejected(doc);
            }
        }
    }
}

#[test]
fn complete_reader_budgets_include_hidden_values_and_test_exact_inclusive_thresholds() {
    let mut budget = Budget {
        used: 0,
        limit: MAX_READ_BYTES,
    };
    budget.add(MAX_READ_BYTES).unwrap();
    assert!(budget.add(1).is_err());
    assert!(Budget {
        used: usize::MAX,
        limit: usize::MAX
    }
    .add(1)
    .is_err());
    let mut base = root(1);
    let anchor = write(&mut base, &insert(1, None, 1), 1, 1, None, None);
    base.commit();
    let mut a = base.clone().with_actor(ActorId::from(vec![2; 32]));
    let mut b = base.with_actor(ActorId::from(vec![3; 32]));
    for (doc, author) in [(&mut a, 2), (&mut b, 3)] {
        write(
            doc,
            &insert(1, None, 2),
            author,
            1,
            None,
            Some(anchor.op_id),
        );
        write(
            doc,
            &FlipnoteOp::ReplaceFrame {
                frame: id(1),
                cid: [author; 32],
                bytes: 1,
            },
            author,
            2,
            None,
            None,
        );
        write(
            doc,
            &FlipnoteOp::RemoveFrame { frame: id(1) },
            author,
            3,
            None,
            None,
        );
        write(
            doc,
            &FlipnoteOp::SetHeader(FlipnoteHeader::Title("same text".into())),
            author,
            4,
            None,
            None,
        );
    }
    let (doc, _) = converge(a, b);
    let exact = doc
        .keys(ROOT)
        .map(|key| {
            key.len()
                + doc
                    .get_all(ROOT, &key)
                    .unwrap()
                    .iter()
                    .map(|(value, _)| match value {
                        Value::Scalar(v) => match v.as_ref() {
                            ScalarValue::Bytes(b) => b.len(),
                            ScalarValue::Str(s) => s.len(),
                            ScalarValue::Uint(_) => 8,
                            _ => panic!("fixture scalar"),
                        },
                        _ => panic!("flat fixture"),
                    })
                    .sum::<usize>()
        })
        .sum::<usize>();
    let count = doc.stats().num_ops;
    let p =
        FlipnoteFrameProjection::read_bounded(&logical(), CHANNEL, 0, &doc, count, exact).unwrap();
    assert_eq!(p, read(&doc));
    assert!(p.timeline.is_empty());
    assert_eq!(p.title.as_ref().unwrap().conflicts.len(), 1);
    for (primitives, bytes) in [(count - 1, exact), (count, exact - 1)] {
        assert!(matches!(
            FlipnoteFrameProjection::read_bounded(&logical(), CHANNEL, 0, &doc, primitives, bytes),
            Err(ReplError::EpochBound)
        ));
    }
}

#[test]
fn deep_anchor_and_right_origin_forests_are_iterative_and_restart_preserves_all_evidence() {
    for right_origin in [false, true] {
        let mut nodes = BTreeMap::new();
        let mut previous = None;
        for n in 0u128..5000 {
            let op = insert(n, if right_origin { None } else { n.checked_sub(1) }, 1);
            let (_, bytes, _) = record(
                &op,
                1,
                n,
                if right_origin { None } else { previous },
                if right_origin { previous } else { None },
            );
            let r = decode_record(&logical(), &bytes).unwrap();
            let insertion = FrameInsertion {
                checkpoint: false,
                after: if right_origin {
                    None
                } else {
                    n.checked_sub(1).map(id)
                },
                anchor: r.anchor,
                before: r.before,
                blob: FrameBlob {
                    cid: [0; 32],
                    bytes: 1,
                },
            };
            previous = Some(r.source.op_id);
            nodes.insert(
                r.source.op_id,
                Node {
                    frame: id(n),
                    insertion: FrameValue {
                        source: r.source,
                        value: insertion,
                    },
                },
            );
        }
        let sequence = order(&nodes).unwrap();
        assert_eq!(sequence.len(), 5000);
        assert_eq!(
            nodes[&sequence[0]].frame,
            id(if right_origin { 4999 } else { 0 })
        );
    }
    let mut doc = root(1);
    let first = write(&mut doc, &insert(1, None, 12), 1, 1, None, None);
    write(
        &mut doc,
        &insert(2, Some(1), 23),
        1,
        2,
        Some(first.op_id),
        None,
    );
    write(
        &mut doc,
        &FlipnoteOp::RemoveFrame { frame: id(1) },
        1,
        3,
        None,
        None,
    );
    write(
        &mut doc,
        &FlipnoteOp::SetHeader(FlipnoteHeader::Title("private title".into())),
        1,
        4,
        None,
        None,
    );
    doc.commit();
    let projection = read(&doc);
    assert_eq!(projection, read(&AutoCommit::load(&doc.save()).unwrap()));
    let changes = doc
        .get_changes(&[])
        .into_iter()
        .map(|c| Change::from_bytes(c.raw_bytes().to_vec()).unwrap());
    let mut restored = empty(9);
    restored.apply_changes(changes).unwrap();
    assert_eq!(projection, read(&restored));
    let entry = &projection.frames[&id(1)];
    for printed in [
        format!("{projection:?}"),
        format!("{entry:?}"),
        format!("{:?}", entry.insertions[0]),
        format!("{:?}", entry.insertions[0].value),
        format!("{:?}", entry.pixels.selected.value),
        format!("{:?}", entry.pixels),
        format!("{first:?}"),
    ] {
        assert!(!printed.contains("private title"));
        assert!(!printed.contains(&hex(&[1; 32])));
    }
}
