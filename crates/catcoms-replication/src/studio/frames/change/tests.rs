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

fn sound_patch() -> crate::studio::StudioPatch {
    crate::studio::StudioPatch::new(&serde_json::json!({"v":1,"o":[{"w":0,"t":0,"c":0,"l":100}],"e":{"a":0,"d":0,"s":100,"r":500},"f":{"m":0,"c":18000,"q":0,"e":0},"l":{"r":1,"d":0,"t":0},"x":{"c":0,"d":0,"r":0}})).unwrap()
}
fn set_sound(sfx: u128, duration_ms: u32) -> FlipnoteOp {
    FlipnoteOp::SetSfx {
        sfx: id(sfx),
        frame: id(1),
        patch: sound_patch().id(),
        note: 60,
        duration_ms,
    }
}
fn sound_base() -> AutoCommit {
    let mut base = empty(&author(1));
    accept(&mut base, 1, &insert(1, None), 1);
    let descriptor = sound_patch();
    accept(
        &mut base,
        1,
        &FlipnoteOp::SetPatch {
            patch: descriptor.id(),
            descriptor,
        },
        2,
    );
    base
}

#[test]
fn frame_sound_authored_duration_upsert_remove_and_hash_bounds() {
    let mut doc = sound_base();
    accept(&mut doc, 1, &set_sound(1, 600_000), 3);
    accept(&mut doc, 1, &set_sound(1, 1), 4);
    let projection = read(&doc);
    assert!(matches!(
        projection.sound.sfx()[&id(1)].value,
        FlipnoteOp::SetSfx { duration_ms: 1, .. }
    ));
    assert!(projection
        .sound_readiness()
        .unwrap()
        .missing_patches
        .is_empty());
    accept(&mut doc, 1, &FlipnoteOp::RemoveSfx { sfx: id(1) }, 5);
    assert!(read(&doc).sound.sfx().is_empty());
    assert!(
        placement(&read(&doc), &set_sound(1, 500)).is_err(),
        "retained deletion blocks id reuse"
    );
    for duration_ms in [0, 600_001, u32::MAX] {
        assert!(set_sound(2, duration_ms).encode().is_err());
    }
    let mut missing: serde_json::Value =
        serde_json::from_slice(&set_sound(2, 500).encode().unwrap()).unwrap();
    missing.as_object_mut().unwrap().remove("duration_ms");
    assert!(FlipnoteOp::decode(&crate::studio::canonical(&missing).unwrap()).is_err());
    let descriptor = sound_patch();
    assert!(FlipnoteOp::SetPatch {
        patch: [0; 32],
        descriptor
    }
    .encode()
    .is_err());
}

#[test]
fn frame_sound_removal_wins_raw_set_and_readd_consumes_only_observed_values() {
    let mut base = sound_base();
    accept(&mut base, 1, &set_sound(1, 500), 3);
    let remove = domain(
        &logical(),
        &FlipnoteOp::RemovePatch {
            patch: sound_patch().id(),
        },
        4,
    );
    let (_, remove_change) = draft(&base, &logical(), &remove, &author(2), 1234);
    let set = domain(
        &logical(),
        &FlipnoteOp::SetPatch {
            patch: sound_patch().id(),
            descriptor: sound_patch(),
        },
        5,
    );
    let (_, set_change) = draft(&base, &logical(), &set, &author(250), 1234);
    let mut combined = merge(
        &base,
        &[
            (remove.clone(), remove_change.clone()),
            (set.clone(), set_change.clone()),
        ],
    );
    let reversed = merge(&base, &[(set, set_change), (remove, remove_change)]);
    assert_eq!(read(&combined), read(&reversed));
    let p = read(&combined);
    let raw = &p.sound.records[&format!("p/{}", hex(&sound_patch().id()))];
    assert!(
        matches!(raw.selected.value, FlipnoteOp::SetPatch { .. }),
        "regression must exercise a raw Set winner"
    );
    assert!(p.sound.patches().is_empty());
    assert!(p
        .sound_readiness()
        .unwrap()
        .missing_patches
        .contains(&id(1)));
    accept(
        &mut combined,
        1,
        &FlipnoteOp::SetPatch {
            patch: sound_patch().id(),
            descriptor: sound_patch(),
        },
        6,
    );
    assert_eq!(read(&combined).sound.patches().len(), 1);

    let remove_sfx = domain(&logical(), &FlipnoteOp::RemoveSfx { sfx: id(1) }, 7);
    let (_, deletion) = draft(&base, &logical(), &remove_sfx, &author(2), 1234);
    let edit_sfx = domain(&logical(), &set_sound(1, 600_000), 8);
    let (_, edit) = draft(&base, &logical(), &edit_sfx, &author(250), 1234);
    assert!(
        read(&merge(&base, &[(remove_sfx, deletion), (edit_sfx, edit)]))
            .sound
            .sfx()
            .is_empty()
    );
}

#[test]
fn frame_sound_readd_does_not_consume_an_unseen_concurrent_removal() {
    let base = sound_base();
    let ra = domain(
        &logical(),
        &FlipnoteOp::RemovePatch {
            patch: sound_patch().id(),
        },
        20,
    );
    let (branch_a, ca) = draft(&base, &logical(), &ra, &author(2), 1234);
    let rb = domain(
        &logical(),
        &FlipnoteOp::RemovePatch {
            patch: sound_patch().id(),
        },
        21,
    );
    let (_, cb) = draft(&base, &logical(), &rb, &author(3), 1234);
    let readd = domain(
        &logical(),
        &FlipnoteOp::SetPatch {
            patch: sound_patch().id(),
            descriptor: sound_patch(),
        },
        22,
    );
    let (_, cr) = draft(&branch_a, &logical(), &readd, &author(4), 1234);
    let combined = merge(&base, &[(ra, ca), (rb, cb), (readd, cr)]);
    assert!(
        read(&combined).sound.patches().is_empty(),
        "readd observed only one of two concurrent removals"
    );
}

#[test]
fn frame_sound_concurrent_export_candidates_and_removal_keep_all_content_holds() {
    let base = sound_base();
    let export = |cid| FlipnoteOp::SetExport {
        export: id(8),
        cid: [cid; 32],
        bytes: 100,
        expiry: crate::studio::StudioExpiry::Never,
    };
    let a = domain(&logical(), &export(8), 30);
    let (_, ca) = draft(&base, &logical(), &a, &author(2), 1234);
    let b = domain(&logical(), &export(9), 31);
    let (_, cb) = draft(&base, &logical(), &b, &author(3), 1234);
    let mut combined = merge(&base, &[(a, ca), (b, cb)]);
    let p = read(&combined);
    assert_eq!(p.sound.exports().len(), 1);
    assert!(p.sound_readiness().unwrap().conflicts);
    let projected = crate::studio::StudioProjection::Flipnote(Box::new(p));
    assert_eq!(
        crate::studio::references::projection_cids(&projected),
        BTreeSet::from([[1; 32], [8; 32], [9; 32]])
    );
    accept(
        &mut combined,
        1,
        &FlipnoteOp::RemoveExport { export: id(8) },
        32,
    );
    let p = read(&combined);
    assert!(p.sound.exports().is_empty());
    assert!(placement(&p, &export(10)).is_err());
    assert_eq!(
        crate::studio::references::projection_cids(&crate::studio::StudioProjection::Flipnote(
            Box::new(p)
        )),
        BTreeSet::from([[1; 32], [8; 32], [9; 32]])
    );
}

#[test]
fn frame_sound_checks_sender_frontier_and_retains_concurrent_invalidated_reference() {
    let mut base = empty(&author(1));
    accept(&mut base, 1, &insert(1, None), 1);
    let operation = domain(&logical(), &set_sound(1, 500), 10);
    let (_, invalid) = raw(
        &base,
        &logical(),
        &operation,
        &author(2),
        1234,
        (None, None),
    );
    let mut receiver = base.clone();
    accept(
        &mut receiver,
        1,
        &FlipnoteOp::SetPatch {
            patch: sound_patch().id(),
            descriptor: sound_patch(),
        },
        2,
    );
    assert!(
        validate_frame_change(&logical(), CHANNEL, 0, &operation, &invalid, &receiver).is_err(),
        "receiver-only patch cannot grant authority"
    );
    let base = sound_base();
    let (_, sound) = draft(&base, &logical(), &operation, &author(2), 1234);
    let removal = domain(&logical(), &FlipnoteOp::RemoveFrame { frame: id(1) }, 11);
    let (_, removed) = draft(&base, &logical(), &removal, &author(3), 1234);
    let p = read(&merge(&base, &[(removal, removed), (operation, sound)]));
    assert_eq!(
        p.sound.sfx().len(),
        1,
        "valid concurrent sound evidence is not silently deleted"
    );
    assert!(p.sound_readiness().unwrap().missing_frames.contains(&id(1)));
}

#[test]
fn frame_sound_seed_recovery_preserve_unlink_deletion_exports_and_prevent_resurrection() {
    let mut doc = sound_base();
    accept(&mut doc, 1, &set_sound(1, 600_000), 3);
    accept(&mut doc, 1, &FlipnoteOp::RemoveSfx { sfx: id(1) }, 4);
    accept(
        &mut doc,
        1,
        &FlipnoteOp::RemovePatch {
            patch: sound_patch().id(),
        },
        5,
    );
    accept(
        &mut doc,
        1,
        &FlipnoteOp::SetHeader(FlipnoteHeader::Score(None)),
        6,
    );
    accept(
        &mut doc,
        1,
        &FlipnoteOp::SetExport {
            export: id(8),
            cid: [8; 32],
            bytes: 123,
            expiry: crate::studio::StudioExpiry::At(0),
        },
        7,
    );
    accept(&mut doc, 1, &FlipnoteOp::RemoveExport { export: id(8) }, 8);
    let p = read(&doc);
    let bytes = p
        .encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
        .unwrap();
    assert_eq!(
        FlipnoteFrameProjection::decode_snapshot(&logical(), CHANNEL, 0, &bytes).unwrap(),
        p
    );
    assert!(FlipnoteFrameProjection::decode_snapshot(
        &logical(),
        CHANNEL,
        0,
        &bytes[..bytes.len() - 1]
    )
    .is_err());
    assert!(p.checkpoint_omits_evidence().unwrap());
    let seed = p.checkpoint([9; 32]).unwrap();
    let mut next = AutoCommit::new();
    next.apply_changes([Change::from_bytes(seed.bytes().to_vec()).unwrap()])
        .unwrap();
    let projected = FlipnoteFrameProjection::read(&logical(), CHANNEL, 1, &next).unwrap();
    assert!(projected.sound.patches().is_empty());
    assert!(projected.sound.sfx().is_empty());
    assert!(projected.sound.exports().is_empty());
    assert!(
        projected.sound.score().is_some(),
        "explicit unlink retains provenance despite silence"
    );
    let projection = crate::studio::StudioProjection::Flipnote(Box::new(p));
    assert!(
        crate::studio::references::projection_cids(&projection).contains(&[8; 32]),
        "deleted export still pins retained bytes"
    );
}

#[test]
fn frame_sound_post_seed_values_override_fallback_and_cannot_reuse_source_nonce() {
    let mut doc = sound_base();
    accept(&mut doc, 1, &set_sound(1, 500), 3);
    let seed = read(&doc).checkpoint([4; 32]).unwrap();
    let mut next = empty(&author(1));
    next.apply_changes([Change::from_bytes(seed.bytes().to_vec()).unwrap()])
        .unwrap();
    for (nonce, accepted) in [(3, false), (4, true)] {
        let operation = domain(&logical(), &set_sound(1, 600_000), nonce);
        let projected = FlipnoteFrameProjection::read(&logical(), CHANNEL, 1, &next).unwrap();
        let prepared = prepare(&projected, &operation, &author(1), 1234).unwrap();
        let mut candidate = next
            .clone()
            .with_actor(ActorId::from(author(1).as_bytes().to_vec()));
        prepared.write(&mut candidate).unwrap();
        candidate
            .put(
                ROOT,
                format!("_p1/op/{}", hex(&operation.id(&author(1)))),
                1u64,
            )
            .unwrap();
        candidate.commit();
        let change = candidate.get_last_local_change().unwrap();
        assert_eq!(
            validate_frame_change(&logical(), CHANNEL, 1, &operation, &change, &next).is_ok(),
            accepted
        );
        if accepted {
            next = candidate;
        }
    }
    let projected = FlipnoteFrameProjection::read(&logical(), CHANNEL, 1, &next).unwrap();
    let selected = projected.sound.sfx()[&id(1)];
    assert!(matches!(
        selected.value,
        FlipnoteOp::SetSfx {
            duration_ms: 600_000,
            ..
        }
    ));
    assert!(projected.sound.records[&format!("s/{}", hex(&id(1)))]
        .conflicts
        .is_empty());
}

/// Decided contract (GATE6-FRAME-SOUND, checkpoint section): an SFX tombstone and an export
/// birth fence their id only while those records are retained causal history. A seed omits
/// removed slots whole, so after compaction a same-id Set with different content is a fresh slot,
/// both when its author prepares it and when a receiver admits it. A lifetime fence would be a
/// seed format change and must change this test deliberately.
#[test]
fn frame_sound_compacted_removals_stop_fencing_their_ids_after_the_seed() {
    let export = |cid: u8| FlipnoteOp::SetExport {
        export: id(8),
        cid: [cid; 32],
        bytes: 100,
        expiry: crate::studio::StudioExpiry::Never,
    };
    let mut doc = sound_base();
    accept(&mut doc, 1, &set_sound(1, 500), 3);
    accept(&mut doc, 1, &FlipnoteOp::RemoveSfx { sfx: id(1) }, 4);
    accept(&mut doc, 1, &export(8), 5);
    accept(&mut doc, 1, &FlipnoteOp::RemoveExport { export: id(8) }, 6);
    let retained = read(&doc);
    assert!(placement(&retained, &set_sound(1, 600_000)).is_err());
    assert!(placement(&retained, &export(9)).is_err());

    let seed = retained.checkpoint([5; 32]).unwrap();
    // The receiver is a third member, so both new changes below arrive from someone else.
    let mut next = empty(&author(3));
    next.apply_changes([Change::from_bytes(seed.bytes().to_vec()).unwrap()])
        .unwrap();
    let seeded = FlipnoteFrameProjection::read(&logical(), CHANNEL, 1, &next).unwrap();
    assert!(!seeded.sound.known(&id(1), false), "the seed keeps no SFX tombstone");
    assert!(!seeded.sound.known(&id(8), true), "the seed keeps no export birth");

    for (who, op, nonce) in [(1, set_sound(1, 600_000), 10), (2, export(9), 11)] {
        let projected = FlipnoteFrameProjection::read(&logical(), CHANNEL, 1, &next).unwrap();
        let operation = domain(&logical(), &op, nonce);
        // Ordinary local authoring on the author's replica...
        let prepared = prepare(&projected, &operation, &author(who), 1234).unwrap();
        let mut sender = next
            .clone()
            .with_actor(ActorId::from(author(who).as_bytes().to_vec()));
        prepared.write(&mut sender).unwrap();
        sender
            .put(
                ROOT,
                format!("_p1/op/{}", hex(&operation.id(&author(who)))),
                1u64,
            )
            .unwrap();
        sender.commit();
        let change = sender.get_last_local_change().unwrap();
        // ...then the receiver's causal admission of that inbound change.
        validate_frame_change(&logical(), CHANNEL, 1, &operation, &change, &next).unwrap();
        next.apply_changes([change]).unwrap();
    }
    let reborn = FlipnoteFrameProjection::read(&logical(), CHANNEL, 1, &next).unwrap();
    assert!(reborn.sound.sfx()[&id(1)].value == set_sound(1, 600_000));
    assert!(reborn.sound.exports()[&id(8)].value == export(9));
    assert!(
        placement(&reborn, &export(10)).is_err(),
        "the new birth fences its id again while it is retained"
    );
}

#[test]
fn frame_sound_mutable_pred_sets_cannot_hide_a_concurrent_removal() {
    let base = sound_base();
    let a = domain(
        &logical(),
        &FlipnoteOp::RemovePatch {
            patch: sound_patch().id(),
        },
        10,
    );
    let (_, ac) = draft(&base, &logical(), &a, &author(2), 1234);
    let b = domain(
        &logical(),
        &FlipnoteOp::SetPatch {
            patch: sound_patch().id(),
            descriptor: sound_patch(),
        },
        11,
    );
    let (_, bc) = draft(&base, &logical(), &b, &author(3), 1234);
    let combined = merge(&base, &[(a, ac), (b, bc)]);
    let readd = domain(
        &logical(),
        &FlipnoteOp::SetPatch {
            patch: sound_patch().id(),
            descriptor: sound_patch(),
        },
        12,
    );
    let (_, good) = draft(&combined, &logical(), &readd, &author(1), 1234);
    let key = format!("p/{}", hex(&sound_patch().id()));
    let mut decoded = good.decode();
    let op = decoded
        .operations
        .iter_mut()
        .find(|op| matches!(&op.key, Key::Map(k) if k.as_str() == key))
        .unwrap();
    assert_eq!(op.pred.len(), 2);
    op.pred = vec![op.pred.iter().next().unwrap().clone()].into();
    assert!(validate_frame_change(
        &logical(),
        CHANNEL,
        0,
        &readd,
        &Change::from(decoded),
        &combined
    )
    .is_err());
}

fn public_sound_record(p: &mut FlipnoteFrameProjection, op: FlipnoteOp, nonce: u128) {
    let domain = domain(p.document(), &op, nonce);
    let source = FrameSource {
        op_id: domain.id(&author(1)),
        author: author(1),
        nonce: domain.nonce,
        ts: 1234,
    };
    let key = sound::key(&op, &source.op_id).unwrap().0;
    p.sound.records.insert(
        key,
        FrameRegister {
            selected: FrameValue { source, value: op },
            conflicts: vec![],
        },
    );
}

#[test]
fn frame_sound_optional_trailer_rejects_independent_malformed_framing_and_source_reuse() {
    use crate::studio::snapshot::Writer;
    let mut doc = empty(&author(1));
    accept(&mut doc, 1, &insert(1, None), 1);
    let p = read(&doc);
    let art = p
        .encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
        .unwrap();
    for suffix in [
        vec![1, 0, 0, 0, 0],
        vec![2, 0, 0, 0, 0],
        vec![1, 255, 255, 255, 255],
    ] {
        let mut bytes = art.clone();
        bytes.extend(suffix);
        assert!(FlipnoteFrameProjection::decode_snapshot(&logical(), CHANNEL, 0, &bytes).is_err());
    }
    let op = domain(
        &logical(),
        &FlipnoteOp::SetHeader(FlipnoteHeader::Score(None)),
        10,
    );
    let record = encode_record(&op, &author(1), 1234, None, None).unwrap();
    let mut suffix = Writer::new(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES);
    suffix.byte(1).unwrap();
    suffix.count(2).unwrap();
    for _ in 0..2 {
        suffix.bytes(b"h/score").unwrap();
        suffix.count(0).unwrap();
        suffix.bytes(&record).unwrap();
    }
    let mut duplicate = art.clone();
    duplicate.extend(suffix.finish());
    assert!(FlipnoteFrameProjection::decode_snapshot(&logical(), CHANNEL, 0, &duplicate).is_err());
    let mut p = p;
    public_sound_record(
        &mut p,
        FlipnoteOp::SetHeader(FlipnoteHeader::Score(None)),
        10,
    );
    let valid = p
        .encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
        .unwrap();
    for cut in art.len() + 1..valid.len() {
        assert!(
            FlipnoteFrameProjection::decode_snapshot(&logical(), CHANNEL, 0, &valid[..cut])
                .is_err(),
            "trailer truncation {cut}"
        );
    }
    public_sound_record(
        &mut p,
        FlipnoteOp::RemovePatch {
            patch: sound_patch().id(),
        },
        10,
    );
    assert!(
        p.encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
            .is_err(),
        "one source cannot appear under two sound bodies"
    );
    p.sound.records.clear();
    public_sound_record(
        &mut p,
        FlipnoteOp::SetHeader(FlipnoteHeader::Score(None)),
        1,
    );
    assert!(
        p.encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
            .is_err(),
        "sound source cannot reuse an art source"
    );
    assert!(!format!("{:?}", p.sound_readiness().unwrap()).contains(&hex(&id(1))));
}

#[test]
fn frame_sound_capacity_flags_preserve_every_export_and_exact_seed_byte_preflight() {
    let mut p = read(&sound_base());
    for n in 1..=4097 {
        public_sound_record(&mut p, set_sound(n, 600_000), 100 + n);
    }
    for n in 1..=9 {
        public_sound_record(
            &mut p,
            FlipnoteOp::SetExport {
                export: id(n),
                cid: [n as u8; 32],
                bytes: 123,
                expiry: crate::studio::StudioExpiry::Never,
            },
            5000 + n,
        );
    }
    let readiness = p.sound_readiness().unwrap();
    assert_eq!(readiness.sfx_overflow, BTreeSet::from([id(4097)]));
    assert_eq!(readiness.export_overflow, BTreeSet::from([id(9)]));
    let payload = p
        .encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
        .unwrap();
    assert_eq!(p.encode_snapshot(payload.len()).unwrap(), payload);
    assert!(matches!(
        p.encode_snapshot(payload.len() - 1),
        Err(ReplError::EpochBound)
    ));
    let seed = p.checkpoint([1; 32]).unwrap();
    assert!(seed.bytes().len() < MAX_CHECKPOINT_BYTES);
    let mut next = AutoCommit::new();
    next.apply_changes([Change::from_bytes(seed.bytes().to_vec()).unwrap()])
        .unwrap();
    let compact = FlipnoteFrameProjection::read(&logical(), CHANNEL, 1, &next).unwrap();
    assert_eq!(compact.sound.sfx().len(), FLIPNOTE_MAX_SFX);
    assert_eq!(compact.sound.exports().len(), FLIPNOTE_MAX_EXPORTS);
    assert!(p.checkpoint_omits_evidence().unwrap());
    let projection = crate::studio::StudioProjection::Flipnote(Box::new(p.clone()));
    assert_eq!(
        crate::studio::references::projection_cids(&projection).len(),
        9
    );
    let t = crate::studio::StudioTarget::Flipnote {
        channel: CHANNEL,
        object: [9; 16],
    };
    let operation = domain(&logical(), &set_sound(4098, 500), 9000);
    assert!(t.local_policy(&projection, &operation, &author(1)).is_err());
    let cleanup = domain(&logical(), &FlipnoteOp::RemoveSfx { sfx: id(4097) }, 9001);
    assert!(t.local_policy(&projection, &cleanup, &author(1)).is_ok());
    let mut damaged = p;
    damaged
        .sound
        .records
        .values_mut()
        .next()
        .unwrap()
        .selected
        .source
        .op_id = [0; 32];
    assert!(damaged
        .encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
        .is_err());
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
    // Sound removals still require a live target at the sender frontier. Explicit score unlink
    // and removal of an absent reusable patch are now valid intent, tested in frame_sound_*.
    for operation in [
        FlipnoteOp::RemoveSfx { sfx: id(1) },
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
