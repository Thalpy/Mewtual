use super::*;
use automerge::transaction::Transactable;
use automerge::{ActorId, ObjType};
use catcoms_mls::{MlsDevice, ServerGroup};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

use crate::{epoch_zero_id, Admission, EncryptedDoc, EpochGate, SealedOp, SignedOp};

const CHANNEL: ElementId = [7; 16];
fn logical() -> LogicalDocument {
    flipnote_document(b"frame-causal-tests", [9; 16]).unwrap()
}
fn id(n: u128) -> ElementId {
    n.to_be_bytes()
}
fn author(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 32])
}
fn empty(who: &DeviceId) -> AutoCommit {
    AutoCommit::new().with_actor(ActorId::from(who.as_bytes().to_vec()))
}
fn insert(frame: u128, after: Option<u128>) -> FlipnoteOp {
    FlipnoteOp::InsertFrame {
        frame: id(frame),
        after: after.map(id),
        cid: [frame as u8; 32],
        bytes: 100,
    }
}
fn replace(frame: u128, cid: u8) -> FlipnoteOp {
    FlipnoteOp::ReplaceFrame {
        frame: id(frame),
        cid: [cid; 32],
        bytes: 200,
    }
}
fn title(value: &str) -> FlipnoteOp {
    FlipnoteOp::SetHeader(FlipnoteHeader::Title(value.into()))
}
fn domain(document: &LogicalDocument, op: &FlipnoteOp, nonce: u128) -> DomainOp {
    DomainOp {
        nonce: nonce.to_be_bytes(),
        doc_type: DocType::StudioObject,
        logical_key: document.logical_key.clone(),
        body: op.encode().unwrap(),
    }
}
fn read(doc: &AutoCommit) -> FlipnoteFrameProjection {
    FlipnoteFrameProjection::read(&logical(), CHANNEL, 0, doc).unwrap()
}
fn origins(change: &Change, document: &LogicalDocument) -> (Option<OpId>, Option<OpId>) {
    let change = change.decode();
    let bytes = change
        .operations
        .iter()
        .find_map(|op| match &op.action {
            OpType::Put(ScalarValue::Bytes(bytes)) => Some(bytes),
            _ => None,
        })
        .unwrap();
    let record = decode_record(document, bytes).unwrap();
    (record.anchor, record.before)
}

// TEST-ONLY writer: intentionally no production mutation API until exact checkpoint/recovery
// preflight, intent persistence and publish-after-save adapters exist. Explicit origins let the
// adversarial cases construct well-formed but causally dishonest metadata independently.
fn write(
    doc: &mut AutoCommit,
    document: &LogicalDocument,
    domain: &DomainOp,
    who: &DeviceId,
    ts: u64,
    gap: (Option<OpId>, Option<OpId>),
) -> Result<(), automerge::AutomergeError> {
    for (key, value) in header(document, CHANNEL, 0) {
        if doc.get(ROOT, &key)?.is_none() {
            doc.put(ROOT, key, value)?;
        }
    }
    let op = FlipnoteOp::decode(&domain.body).unwrap();
    let key = entry(&op, &domain.id(who))
        .map(|(key, _)| key)
        .unwrap_or_else(|_| "h/unsupported".into());
    doc.put(
        ROOT,
        key,
        encode_record(domain, who, ts, gap.0, gap.1).unwrap(),
    )?;
    Ok(())
}
fn raw(
    before: &AutoCommit,
    document: &LogicalDocument,
    domain: &DomainOp,
    who: &DeviceId,
    ts: u64,
    gap: (Option<OpId>, Option<OpId>),
) -> (AutoCommit, Change) {
    let mut doc = before
        .clone()
        .with_actor(ActorId::from(who.as_bytes().to_vec()));
    write(&mut doc, document, domain, who, ts, gap).unwrap();
    doc.put(ROOT, format!("_p1/op/{}", hex(&domain.id(who))), 1u64)
        .unwrap();
    doc.commit();
    let change = doc.get_last_local_change().unwrap();
    (doc, change)
}
fn draft(
    before: &AutoCommit,
    document: &LogicalDocument,
    domain: &DomainOp,
    who: &DeviceId,
    ts: u64,
) -> (AutoCommit, Change) {
    let projection = FlipnoteFrameProjection::read(document, CHANNEL, 0, before).unwrap();
    let gap = placement(&projection, &FlipnoteOp::decode(&domain.body).unwrap()).unwrap();
    raw(before, document, domain, who, ts, gap)
}
fn accept(doc: &mut AutoCommit, who: u8, op: &FlipnoteOp, nonce: u128) -> (DomainOp, Change) {
    let domain = domain(&logical(), op, nonce);
    let (next, change) = draft(doc, &logical(), &domain, &author(who), 1234);
    validate_frame_change(&logical(), CHANNEL, 0, &domain, &change, doc).unwrap();
    *doc = next;
    (domain, change)
}
fn merge(base: &AutoCommit, changes: &[(DomainOp, Change)]) -> AutoCommit {
    let mut doc = base.clone();
    for (domain, change) in changes {
        validate_frame_change(&logical(), CHANNEL, 0, domain, change, &doc).unwrap();
        doc.apply_changes([change.clone()]).unwrap();
    }
    doc
}

#[test]
fn frame_change_sequential_placement_and_art_operations_preserve_exact_intent() {
    let mut doc = empty(&author(1));
    accept(
        &mut doc,
        1,
        &FlipnoteOp::SetHeader(FlipnoteHeader::Fps(12)),
        1,
    );
    let a = accept(&mut doc, 1, &insert(1, None), 2);
    let aid = a.0.id(&author(1));
    let b = accept(&mut doc, 1, &insert(2, Some(1)), 3);
    let bid = b.0.id(&author(1));
    let c = accept(&mut doc, 1, &insert(3, Some(1)), 4);
    let d = accept(&mut doc, 1, &insert(4, None), 5);
    assert_eq!(origins(&a.1, &logical()), (None, None));
    assert_eq!(origins(&b.1, &logical()), (Some(aid), None));
    assert_eq!(origins(&c.1, &logical()), (Some(aid), Some(bid)));
    assert_eq!(origins(&d.1, &logical()), (None, Some(aid)));
    assert_eq!(read(&doc).timeline, [id(4), id(1), id(3), id(2)]);
    accept(&mut doc, 2, &replace(2, 8), 1);
    accept(&mut doc, 2, &FlipnoteOp::RemoveFrame { frame: id(3) }, 2);
    assert_eq!(read(&doc).timeline, [id(4), id(1), id(2)]);
    assert_eq!(read(&doc).frames[&id(2)].pixels.selected.value.cid, [8; 32]);
    for (nonce, ts) in [(3, 0), (4, super::super::super::MAX_STUDIO_INTEGER)] {
        let domain = domain(&logical(), &title("same text, new provenance"), nonce);
        let (next, change) = draft(&doc, &logical(), &domain, &author(2), ts);
        validate_frame_change(&logical(), CHANNEL, 0, &domain, &change, &doc).unwrap();
        assert_eq!(origins(&change, &logical()), (None, None));
        doc = next;
        assert_eq!(read(&doc).title.unwrap().selected.source.ts, ts);
    }
}

#[test]
fn frame_change_historical_view_preserves_winners_empty_past_and_reader_bounds() {
    let mut doc = empty(&author(1));
    let mut versions = vec![(vec![], read(&doc))];
    for (nonce, op) in [
        insert(1, None),
        title("before"),
        replace(1, 6),
        title("after"),
        FlipnoteOp::RemoveFrame { frame: id(1) },
    ]
    .iter()
    .enumerate()
    {
        accept(&mut doc, 1, op, nonce as u128 + 1);
        versions.push((doc.get_heads(), read(&doc)));
    }
    for (heads, expected) in &versions {
        assert_eq!(
            &FlipnoteFrameProjection::read_at(&logical(), CHANNEL, 0, &doc, heads).unwrap(),
            expected
        );
    }
    assert!(FlipnoteFrameProjection::read_at(
        &logical(),
        CHANNEL,
        0,
        &doc,
        &[ChangeHash([255; 32])]
    )
    .is_err());
    let mut mixed = doc.get_heads();
    mixed.push(ChangeHash([255; 32]));
    assert!(FlipnoteFrameProjection::read_at(&logical(), CHANNEL, 0, &doc, &mixed).is_err());
    let heads = doc.get_heads();
    assert!(FlipnoteFrameProjection::read_view(
        &logical(),
        CHANNEL,
        0,
        &doc,
        Some(&heads),
        (doc.stats().num_ops - 1, MAX_READ_BYTES)
    )
    .is_err());
    assert!(FlipnoteFrameProjection::read_view(
        &logical(),
        CHANNEL,
        0,
        &doc,
        Some(&heads),
        (MAX_PRIMITIVES, 1)
    )
    .is_err());
    for key in doc.keys(ROOT).collect::<Vec<_>>() {
        doc.delete(ROOT, key).unwrap();
    }
    doc.commit();
    let erased = doc.get_heads();
    assert!(FlipnoteFrameProjection::read(&logical(), CHANNEL, 0, &doc).is_err());
    assert!(FlipnoteFrameProjection::read_at(&logical(), CHANNEL, 0, &doc, &erased).is_err());
    assert!(
        FlipnoteFrameProjection::read_at(&logical(), CHANNEL, 0, &doc, &[])
            .unwrap()
            .frames
            .is_empty()
    );
    assert!(FlipnoteFrameProjection::read_at(&logical(), CHANNEL, 1, &doc, &[]).is_err());
}

#[test]
fn frame_change_old_and_proper_subset_frontiers_ignore_receiver_only_right_origins() {
    let mut base = empty(&author(1));
    let a = accept(&mut base, 1, &insert(1, None), 1);
    let b = accept(&mut base, 1, &insert(2, Some(1)), 2);
    for subset in [false, true] {
        let mut old = base.clone();
        if subset {
            accept(&mut old, 2, &title("one still-live branch"), 1);
        }
        let c = accept(&mut base.clone(), 3, &insert(3, Some(1)), 1);
        let receiver = merge(&old, std::slice::from_ref(&c));
        let request = domain(&logical(), &insert(4, Some(1)), 2);
        let (_, good) = draft(&old, &logical(), &request, &author(2), 7);
        assert_eq!(
            origins(&good, &logical()),
            (Some(a.0.id(&author(1))), Some(b.0.id(&author(1))))
        );
        validate_frame_change(&logical(), CHANNEL, 0, &request, &good, &receiver).unwrap();
        if subset {
            let mut receiver = receiver.clone();
            let heads = receiver.get_heads();
            assert_eq!(heads.len(), 2);
            assert_eq!(good.deps().len(), 1);
            assert!(heads.contains(&good.deps()[0]));
        }
        // A current-view placement looks structurally valid but the author never saw C.
        let (_, forged) = raw(
            &old,
            &logical(),
            &request,
            &author(2),
            7,
            (Some(a.0.id(&author(1))), Some(c.0.id(&author(3)))),
        );
        assert!(
            validate_frame_change(&logical(), CHANNEL, 0, &request, &forged, &receiver).is_err()
        );
    }
}

#[test]
fn frame_change_hidden_direct_children_still_determine_the_gap() {
    let mut doc = empty(&author(1));
    let a = accept(&mut doc, 1, &insert(1, None), 1);
    let b = accept(&mut doc, 1, &insert(2, Some(1)), 2);
    let child = accept(&mut doc, 1, &insert(3, Some(2)), 3);
    accept(&mut doc, 1, &FlipnoteOp::RemoveFrame { frame: id(2) }, 4);
    let request = domain(&logical(), &insert(4, Some(1)), 5);
    let (next, good) = draft(&doc, &logical(), &request, &author(1), 1234);
    let anchor = Some(a.0.id(&author(1)));
    assert_eq!(
        origins(&good, &logical()),
        (anchor, Some(b.0.id(&author(1))))
    );
    validate_frame_change(&logical(), CHANNEL, 0, &request, &good, &doc).unwrap();
    assert_eq!(read(&next).timeline, [id(1), id(4), id(3)]);
    for wrong_right in [None, Some(child.0.id(&author(1)))] {
        let (_, forged) = raw(
            &doc,
            &logical(),
            &request,
            &author(1),
            1234,
            (anchor, wrong_right),
        );
        assert!(validate_frame_change(&logical(), CHANNEL, 0, &request, &forged, &doc).is_err());
    }
    let request = domain(&logical(), &insert(5, Some(2)), 6);
    let (_, forged) = raw(
        &doc,
        &logical(),
        &request,
        &author(1),
        1234,
        (Some(b.0.id(&author(1))), Some(child.0.id(&author(1)))),
    );
    assert!(validate_frame_change(&logical(), CHANNEL, 0, &request, &forged, &doc).is_err());
}

#[test]
fn frame_change_causally_known_losing_child_still_determines_the_gap() {
    let mut base = empty(&author(1));
    let root = accept(&mut base, 1, &insert(1, None), 1);
    // The operation id does not include the body. Pick the larger identity/nonce id for
    // the birth after A, so the winning birth elsewhere hides precisely A's first child.
    let probe = domain(&logical(), &insert(2, None), 1);
    let (loser, winner) = if probe.id(&author(2)) > probe.id(&author(3)) {
        (2, 3)
    } else {
        (3, 2)
    };
    let mut branch = base.clone();
    let losing = accept(&mut branch, loser, &insert(2, Some(1)), 1);
    let child = accept(&mut branch, loser, &insert(3, Some(2)), 2);
    let winning = accept(&mut base.clone(), winner, &insert(2, None), 1);
    let anchor = Some(root.0.id(&author(1)));
    let right = Some(losing.0.id(&author(loser)));
    for changes in [
        vec![losing.clone(), child.clone(), winning.clone()],
        vec![winning.clone(), losing.clone(), child.clone()],
    ] {
        // Both competing births are already in the sender's dependencies, unlike the
        // receiver-only collision test. Filtering to winning births would lose this gap.
        let doc = merge(&base, &changes);
        let projection = read(&doc);
        assert_eq!(projection.timeline, [id(2), id(1), id(3)]);
        assert_eq!(projection.frames[&id(2)].insertions.len(), 2);
        assert_eq!(
            projection.frames[&id(2)].insertions[0].source.op_id,
            winning.0.id(&author(winner))
        );
        let request = domain(&logical(), &insert(4, Some(1)), 1);
        let (next, good) = draft(&doc, &logical(), &request, &author(4), 1234);
        assert_eq!(origins(&good, &logical()), (anchor, right));
        validate_frame_change(&logical(), CHANNEL, 0, &request, &good, &doc).unwrap();
        assert_eq!(read(&next).timeline, [id(2), id(1), id(4), id(3)]);
        for wrong_right in [None, Some(child.0.id(&author(loser)))] {
            let (_, forged) = raw(
                &doc,
                &logical(),
                &request,
                &author(4),
                1234,
                (anchor, wrong_right),
            );
            assert!(
                validate_frame_change(&logical(), CHANNEL, 0, &request, &forged, &doc).is_err()
            );
        }
    }
}

#[test]
fn frame_change_receiver_only_smaller_collision_cannot_rewrite_the_causal_anchor() {
    let mut base = empty(&author(1));
    let root = accept(&mut base, 1, &insert(1, None), 1);
    let mut a = base.clone();
    let ca = accept(&mut a, 2, &insert(2, Some(1)), 1);
    let mut b = base.clone();
    let cb = accept(&mut b, 3, &insert(2, Some(1)), 1);
    let (larger_doc, larger, smaller) = if ca.0.id(&author(2)) > cb.0.id(&author(3)) {
        (a, (ca.clone(), author(2)), (cb.clone(), author(3)))
    } else {
        (b, (cb.clone(), author(3)), (ca.clone(), author(2)))
    };
    let receiver = merge(&larger_doc, std::slice::from_ref(&smaller.0));
    let request = domain(&logical(), &insert(3, Some(2)), 1);
    let (_, good) = draft(&larger_doc, &logical(), &request, &author(4), 1);
    assert_eq!(
        origins(&good, &logical()).0,
        Some(larger.0 .0.id(&larger.1))
    );
    validate_frame_change(&logical(), CHANNEL, 0, &request, &good, &receiver).unwrap();
    let (_, forged) = raw(
        &larger_doc,
        &logical(),
        &request,
        &author(4),
        1,
        (Some(smaller.0 .0.id(&smaller.1)), None),
    );
    assert!(validate_frame_change(&logical(), CHANNEL, 0, &request, &forged, &receiver).is_err());
    let left = merge(
        &base,
        &[ca.clone(), cb.clone(), (request.clone(), good.clone())],
    );
    let right = merge(&base, &[cb, ca, (request, good)]);
    assert_eq!(read(&left), read(&right));
    assert_eq!(read(&left).frames[&id(2)].insertions.len(), 2);
    assert_eq!(
        read(&left).insertion_order().first(),
        Some(&root.0.id(&author(1)))
    );
}

#[test]
fn frame_change_replacement_and_header_predecessors_require_complete_exact_sets() {
    for operation in [
        replace(1, 7),
        title("resolve"),
        FlipnoteOp::SetHeader(FlipnoteHeader::Fps(10)),
    ] {
        let mut base = empty(&author(1));
        accept(&mut base, 1, &insert(1, None), 1);
        let a = accept(&mut base.clone(), 2, &operation, 1);
        let b = accept(&mut base.clone(), 3, &operation, 1);
        let merged = merge(&base, &[a, b]);
        let request = domain(&logical(), &operation, 2);
        let (_, good) = draft(&merged, &logical(), &request, &author(2), 1234);
        validate_frame_change(&logical(), CHANNEL, 0, &request, &good, &merged).unwrap();
        let key = entry(&operation, &request.id(&author(2))).unwrap().0;
        let ObjId::Id(counter, actor, _) = merged.get(ROOT, "v").unwrap().unwrap().1 else {
            panic!("victim")
        };
        for mode in 0..4 {
            let mut decoded = good.decode();
            let op = decoded
                .operations
                .iter_mut()
                .find(|op| matches!(&op.key, Key::Map(k) if k.as_str() == key))
                .unwrap();
            let preds: Vec<_> = op.pred.iter().cloned().collect();
            assert_eq!(preds.len(), 2);
            op.pred = match mode {
                0 => vec![],
                1 => vec![preds[0].clone()],
                2 => vec![preds[0].clone(), preds[0].clone(), preds[1].clone()],
                _ => vec![AmOpId(counter, actor.clone())],
            }
            .into();
            let forged = Change::from(decoded);
            if mode == 2 {
                let decoded = forged.decode();
                let op = decoded
                    .operations
                    .iter()
                    .find(|op| matches!(&op.key, Key::Map(k) if k.as_str() == key))
                    .unwrap();
                assert_eq!(op.pred.len(), 3);
                assert_eq!(op.pred.iter().collect::<BTreeSet<_>>().len(), 2);
            }
            assert!(
                validate_frame_change(&logical(), CHANNEL, 0, &request, &forged, &merged).is_err()
            );
        }
    }
}

#[test]
fn frame_change_missing_deleted_and_over_cap_targets_have_distinct_semantics() {
    let mut doc = empty(&author(1));
    accept(&mut doc, 1, &insert(1, None), 1);
    let empty_past = empty(&author(2));
    let missing = domain(&logical(), &replace(1, 8), 1);
    let (_, change) = raw(
        &empty_past,
        &logical(),
        &missing,
        &author(2),
        1,
        (None, None),
    );
    assert!(validate_frame_change(&logical(), CHANNEL, 0, &missing, &change, &doc).is_err());
    accept(&mut doc, 1, &FlipnoteOp::RemoveFrame { frame: id(1) }, 2);
    for op in [
        insert(1, None),
        replace(1, 9),
        FlipnoteOp::RemoveFrame { frame: id(1) },
        insert(2, Some(1)),
    ] {
        let request = domain(&logical(), &op, 3);
        let (_, forged) = raw(&doc, &logical(), &request, &author(1), 1, (None, None));
        assert!(validate_frame_change(&logical(), CHANNEL, 0, &request, &forged, &doc).is_err());
    }
    // 129 full declarations exceed 8 MiB, without fetching a single blob. This semantic-only
    // harness is NOT cap admission; removal must still have a causal target so it can trim.
    let mut large = empty(&author(1));
    for n in 1..=129 {
        let op = FlipnoteOp::InsertFrame {
            frame: id(n),
            after: if n == 1 { None } else { Some(id(n - 1)) },
            cid: [1; 32],
            bytes: 65536,
        };
        accept(&mut large, 1, &op, n);
    }
    assert!(read(&large).over_cap.contains_key(&id(129)));
    accept(
        &mut large,
        1,
        &FlipnoteOp::RemoveFrame { frame: id(129) },
        130,
    );
    assert!(read(&large).over_cap.is_empty());
}

#[test]
fn frame_change_exact_metadata_and_root_mutations_reject_tampering() {
    let base = empty(&author(1));
    let request = domain(&logical(), &insert(1, None), 1);
    let (_, good) = draft(&base, &logical(), &request, &author(1), 1234);
    assert_eq!(good.len(), 9);
    for mode in 0..13 {
        let mut decoded = good.decode();
        match mode {
            0 => decoded
                .operations
                .retain(|op| !matches!(&op.key, Key::Map(k) if k == "w")),
            1 => decoded
                .operations
                .retain(|op| !matches!(&op.key, Key::Map(k) if k.starts_with("_p1/"))),
            2 => decoded
                .operations
                .retain(|op| !matches!(&op.key, Key::Map(k) if k.starts_with("i/"))),
            _ => {
                let op = decoded
                    .operations
                    .iter_mut()
                    .find(|op| matches!(&op.key, Key::Map(k) if k.starts_with("i/")))
                    .unwrap();
                match mode {
                    3 => op.action = OpType::Delete,
                    4 => op.action = OpType::Make(ObjType::Map),
                    5 => op.insert = true,
                    6 => op.obj = ObjectId::Id(AmOpId(1, good.actor_id().clone())),
                    7 => op.key = Key::Map("arbitrary".into()),
                    _ => {
                        let OpType::Put(ScalarValue::Bytes(bytes)) = &mut op.action else {
                            panic!("bytes")
                        };
                        match mode {
                            8 => bytes[32] ^= 1, // identity collision: first FOUR bytes stay equal
                            9 => bytes[33..41].copy_from_slice(&u64::MAX.to_be_bytes()),
                            10 => bytes.push(0),
                            11 => {
                                *bytes =
                                    encode_record(&request, &author(1), 1234, Some([5; 32]), None)
                                        .unwrap()
                            }
                            _ => {
                                let mut different = request.clone();
                                different.body = FlipnoteOp::InsertFrame {
                                    frame: id(1),
                                    after: None,
                                    cid: [99; 32],
                                    bytes: 100,
                                }
                                .encode()
                                .unwrap();
                                assert_eq!(different.id(&author(1)), request.id(&author(1)));
                                *bytes = encode_record(&different, &author(1), 1234, None, None)
                                    .unwrap();
                            }
                        }
                    }
                }
            }
        }
        assert!(
            validate_frame_change(
                &logical(),
                CHANNEL,
                0,
                &request,
                &Change::from(decoded),
                &base
            )
            .is_err(),
            "mode {mode}"
        );
    }
    // A sound/score/export body is statically valid but not supported by this art validator.
    for operation in [
        FlipnoteOp::SetHeader(FlipnoteHeader::Score(None)),
        FlipnoteOp::RemoveSfx { sfx: id(1) },
        FlipnoteOp::RemovePatch { patch: [1; 32] },
        FlipnoteOp::RemoveExport { export: id(1) },
    ] {
        let unsupported = domain(&logical(), &operation, 1);
        let (_, change) = raw(&base, &logical(), &unsupported, &author(1), 1, (None, None));
        assert!(
            validate_frame_change(&logical(), CHANNEL, 0, &unsupported, &change, &base).is_err()
        );
    }
    for epoch in [1, 4096, u64::MAX] {
        assert!(validate_frame_change(&logical(), CHANNEL, epoch, &request, &good, &base).is_err());
    }
    let (created, _) = draft(&base, &logical(), &request, &author(1), 1234);
    let rename = domain(&logical(), &title("channel-bound"), 2);
    let (_, renamed) = draft(&created, &logical(), &rename, &author(1), 1234);
    validate_frame_change(&logical(), CHANNEL, 0, &rename, &renamed, &created).unwrap();
    assert!(validate_frame_change(&logical(), [8; 16], 0, &rename, &renamed, &created).is_err());
    let (mut rewriting, _) = draft(&created, &logical(), &rename, &author(1), 1234);
    // Build a new causal header write, then change its value back to the expected dimension.
    // Correct bytes do not authorize re-putting an already initialized immutable property.
    rewriting.put(ROOT, "w", 193u64).unwrap();
    rewriting.commit();
    let mut rewritten = renamed.decode();
    let mut header_op = rewriting
        .get_last_local_change()
        .unwrap()
        .decode()
        .operations
        .remove(0);
    header_op.action = OpType::Put(ScalarValue::Uint(192));
    rewritten.operations.push(header_op);
    assert!(validate_frame_change(
        &logical(),
        CHANNEL,
        0,
        &rename,
        &Change::from(rewritten),
        &created
    )
    .is_err());
    for len in [0, 15, 17] {
        let mut wrong = logical();
        wrong.logical_key = vec![1; len];
        assert!(validate_frame_change(&wrong, CHANNEL, 0, &request, &good, &base).is_err());
    }
}

// Signed-gate integration deliberately uses reader-only TEST preflight. This tests semantic and
// signature/rollback boundaries, not production cap/checkpoint admission or durable Save/Load.
struct Fixture {
    owner: MlsDevice,
    group: ServerGroup,
    logical: LogicalDocument,
    rng: ChaCha20Rng,
}
impl Fixture {
    fn new() -> Self {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let logical = flipnote_document(&group.group_id(), [9; 16]).unwrap();
        Self {
            owner,
            group,
            logical,
            rng: ChaCha20Rng::from_seed([29; 32]),
        }
    }
    fn target(&self) -> (EncryptedDoc, EpochGate) {
        let id = epoch_zero_id(DocType::StudioObject, &self.logical.logical_key);
        (
            EncryptedDoc::new(DocType::StudioObject, id, &self.owner.device_id()),
            EpochGate::new(self.logical.clone(), id, 0, self.owner.device_id()),
        )
    }
    fn edit(
        &mut self,
        doc: &mut EncryptedDoc,
        gate: &EpochGate,
        domain: &DomainOp,
        refuse: bool,
    ) -> Result<SealedOp, ReplError> {
        let before = doc.doc().clone();
        let projection = FlipnoteFrameProjection::read(&self.logical, CHANNEL, 0, &before)?;
        let gap = placement(&projection, &FlipnoteOp::decode(&domain.body)?)?;
        doc.edit_domain_preflight_gated(
            &self.logical,
            gate,
            &self.owner,
            &self.group,
            &mut self.rng,
            domain,
            |next| {
                write(
                    next,
                    &self.logical,
                    domain,
                    &self.owner.device_id(),
                    1234,
                    gap,
                )
            },
            |domain, change| {
                validate_frame_change(&self.logical, CHANNEL, 0, domain, change, &before)
            },
            |next| {
                if refuse {
                    Err(ReplError::EpochBound)
                } else {
                    FlipnoteFrameProjection::read(&self.logical, CHANNEL, 0, next).map(|_| ())
                }
            },
        )
        .map(|(sealed, _)| sealed)
    }
    fn ingest(
        &self,
        doc: &mut EncryptedDoc,
        gate: &EpochGate,
        sealed: &SealedOp,
        refuse: bool,
    ) -> Result<Admission, ReplError> {
        let before = doc.doc().clone();
        doc.ingest_domain_preflight_gated(
            &self.logical,
            gate,
            sealed,
            &self.group,
            &self.owner,
            |domain, change| {
                validate_frame_change(&self.logical, CHANNEL, 0, domain, change, &before)
            },
            |next| {
                if refuse {
                    Err(ReplError::EpochBound)
                } else {
                    FlipnoteFrameProjection::read(&self.logical, CHANNEL, 0, next).map(|_| ())
                }
            },
        )
    }
}

#[test]
fn frame_change_signed_gate_rollback_restart_and_retry_metadata_are_exact() {
    let mut f = Fixture::new();
    let (mut source, sg) = f.target();
    let (mut target, tg) = f.target();
    let request = domain(&f.logical, &insert(1, None), 1);
    let pristine = source.snapshot().unwrap();
    let pristine_gate = sg.encode().unwrap();
    assert!(matches!(
        f.edit(&mut source, &sg, &request, true),
        Err(ReplError::EpochBound)
    ));
    assert_eq!(source.snapshot().unwrap(), pristine);
    assert_eq!(sg.encode().unwrap(), pristine_gate);
    let sealed = f.edit(&mut source, &sg, &request, false).unwrap();
    assert!(matches!(
        f.ingest(&mut target, &tg, &sealed, true),
        Err(ReplError::EpochBound)
    ));
    assert_eq!(target.snapshot().unwrap(), pristine);
    assert_eq!(tg.encode().unwrap(), pristine_gate);
    assert_eq!(
        f.ingest(&mut target, &tg, &sealed, false).unwrap(),
        Admission::Accepted
    );
    let saved = target.snapshot().unwrap();
    target = EncryptedDoc::restore_for_actor(&saved, &f.owner.device_id()).unwrap();
    let tg = EpochGate::decode(&tg.encode().unwrap()).unwrap();
    assert_eq!(
        f.ingest(&mut target, &tg, &sealed, false).unwrap(),
        Admission::Duplicate
    );
    let gate_saved = tg.encode().unwrap();
    // Same nonce/body but altered timestamp on an independent root is not the retained envelope.
    let (_, alternate) = raw(
        &empty(&f.owner.device_id()),
        &f.logical,
        &request,
        &f.owner.device_id(),
        999,
        (None, None),
    );
    let signed = SignedOp::sign_domain(
        &f.owner,
        DocType::StudioObject,
        target.doc_id(),
        alternate.raw_bytes().to_vec(),
        &request,
    )
    .unwrap();
    let alternate = SealedOp::seal(&signed, &f.group, &f.owner, &mut f.rng).unwrap();
    let rejection = f.ingest(&mut target, &tg, &alternate, false);
    // This independent root reuses actor sequence 1; AM rejects that equivocation before the
    // gate sees it. Pin the actual boundary rather than pretend this reaches marker validation.
    assert!(
        matches!(rejection, Err(ReplError::Automerge(_))),
        "{rejection:?}"
    );
    assert_eq!(target.snapshot().unwrap(), saved);
    assert_eq!(tg.encode().unwrap(), gate_saved);
    let next = domain(&f.logical, &replace(1, 8), 2);
    f.edit(&mut target, &tg, &next, false).unwrap();
    assert_eq!(
        FlipnoteFrameProjection::read(&f.logical, CHANNEL, 0, target.doc())
            .unwrap()
            .frames[&id(1)]
            .pixels
            .selected
            .value
            .cid,
        [8; 32]
    );
    // Unlike the independent root above, this is a well-formed NEW causal actor sequence. It
    // reuses the accepted replacement's nonce with changed timestamp and must hit the marker
    // conflict check. Neither a signed re-envelope nor changed metadata is an exact retry.
    let saved = target.snapshot().unwrap();
    let gate_saved = tg.encode().unwrap();
    let (_, changed_ts) = raw(
        target.doc(),
        &f.logical,
        &next,
        &f.owner.device_id(),
        9876,
        (None, None),
    );
    let signed = SignedOp::sign_domain(
        &f.owner,
        DocType::StudioObject,
        target.doc_id(),
        changed_ts.raw_bytes().to_vec(),
        &next,
    )
    .unwrap();
    let changed_ts = SealedOp::seal(&signed, &f.group, &f.owner, &mut f.rng).unwrap();
    assert!(matches!(
        f.ingest(&mut target, &tg, &changed_ts, false),
        Err(ReplError::IntentConflict)
    ));
    assert_eq!(target.snapshot().unwrap(), saved);
    assert_eq!(tg.encode().unwrap(), gate_saved);
}

#[test]
fn frame_change_signed_member_order_and_causal_origin_attacks_are_atomic() {
    let mut f = Fixture::new();
    let peer = MlsDevice::generate().unwrap();
    f.group
        .add_member(&f.owner, peer.key_package().unwrap())
        .unwrap();
    let (mut left, lg) = f.target();
    let (mut right, rg) = f.target();
    let a = domain(&f.logical, &insert(1, None), 1);
    let initial = f.edit(&mut left, &lg, &a, false).unwrap();
    f.ingest(&mut right, &rg, &initial, false).unwrap();
    let mut changes = Vec::new();
    for (signer, frame) in [(&f.owner, 2), (&peer, 3)] {
        let request = domain(&f.logical, &insert(frame, Some(1)), 2);
        let (_, change) = draft(left.doc(), &f.logical, &request, &signer.device_id(), 1234);
        let signed = SignedOp::sign_domain(
            signer,
            DocType::StudioObject,
            left.doc_id(),
            change.raw_bytes().to_vec(),
            &request,
        )
        .unwrap();
        changes.push(SealedOp::seal(&signed, &f.group, &f.owner, &mut f.rng).unwrap());
    }
    for sealed in &changes {
        f.ingest(&mut left, &lg, sealed, false).unwrap();
    }
    for sealed in changes.iter().rev() {
        f.ingest(&mut right, &rg, sealed, false).unwrap();
    }
    assert_eq!(
        FlipnoteFrameProjection::read(&f.logical, CHANNEL, 0, left.doc()).unwrap(),
        FlipnoteFrameProjection::read(&f.logical, CHANNEL, 0, right.doc()).unwrap()
    );
    let saved = left.snapshot().unwrap();
    let gate_saved = lg.encode().unwrap();
    let request = domain(&f.logical, &insert(4, Some(1)), 3);
    let projection = FlipnoteFrameProjection::read(&f.logical, CHANNEL, 0, left.doc()).unwrap();
    let gap = placement(&projection, &FlipnoteOp::decode(&request.body).unwrap()).unwrap();
    assert!(gap.1.is_some());
    let (_, forged) = raw(
        left.doc(),
        &f.logical,
        &request,
        &peer.device_id(),
        1234,
        (gap.0, None),
    );
    let signed = SignedOp::sign_domain(
        &peer,
        DocType::StudioObject,
        left.doc_id(),
        forged.raw_bytes().to_vec(),
        &request,
    )
    .unwrap();
    let sealed = SealedOp::seal(&signed, &f.group, &f.owner, &mut f.rng).unwrap();
    assert!(f.ingest(&mut left, &lg, &sealed, false).is_err());
    assert_eq!(left.snapshot().unwrap(), saved);
    assert_eq!(lg.encode().unwrap(), gate_saved);
    // A valid replacement remains valid when the receiver has concurrently learned a deletion.
    let mut last = Vec::new();
    for (signer, op) in [
        (&f.owner, replace(2, 9)),
        (&peer, FlipnoteOp::RemoveFrame { frame: id(2) }),
    ] {
        let request = domain(&f.logical, &op, 4);
        let (_, change) = draft(left.doc(), &f.logical, &request, &signer.device_id(), 1234);
        let signed = SignedOp::sign_domain(
            signer,
            DocType::StudioObject,
            left.doc_id(),
            change.raw_bytes().to_vec(),
            &request,
        )
        .unwrap();
        last.push(SealedOp::seal(&signed, &f.group, &f.owner, &mut f.rng).unwrap());
    }
    for sealed in &last {
        f.ingest(&mut left, &lg, sealed, false).unwrap();
    }
    for sealed in last.iter().rev() {
        f.ingest(&mut right, &rg, sealed, false).unwrap();
    }
    assert_eq!(
        FlipnoteFrameProjection::read(&f.logical, CHANNEL, 0, left.doc()).unwrap(),
        FlipnoteFrameProjection::read(&f.logical, CHANNEL, 0, right.doc()).unwrap()
    );
}
