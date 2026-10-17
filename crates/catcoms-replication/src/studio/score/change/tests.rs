use super::*;
use automerge::{transaction::Transactable, ActorId};

const CHANNEL: ElementId = [7; 16];
fn logical() -> LogicalDocument {
    score_document(b"score-state-tests", [9; 16]).unwrap()
}
fn author(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 32])
}
fn empty() -> AutoCommit {
    AutoCommit::new().with_actor(ActorId::from(vec![1; 32]))
}
fn patch() -> StudioPatch {
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/studio-score-ops-v1.json"
    ))
    .unwrap();
    let body = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "set_patch")
        .unwrap()["body"]
        .as_str()
        .unwrap();
    match ScoreOp::decode(body.as_bytes()).unwrap() {
        ScoreOp::SetPatch { descriptor, .. } => descriptor,
        _ => unreachable!(),
    }
}
fn set_patch() -> ScoreOp {
    let descriptor = patch();
    ScoreOp::SetPatch {
        patch: descriptor.id(),
        descriptor,
    }
}
fn note(step: u16, row: u8, len: Option<u16>) -> ScoreOp {
    ScoreOp::SetCell {
        step,
        row,
        value: ScoreCell::Note {
            patch: patch().id(),
            len,
        },
    }
}
fn drum(step: u16) -> ScoreOp {
    ScoreOp::SetCell {
        step,
        row: 0,
        value: ScoreCell::Drum { pad: 0 },
    }
}
fn domain(op: &ScoreOp, n: u128) -> DomainOp {
    DomainOp {
        nonce: n.to_be_bytes(),
        doc_type: DocType::StudioObject,
        logical_key: logical().logical_key,
        body: op.encode().unwrap(),
    }
}
fn read(doc: &AutoCommit) -> ScoreProjection {
    ScoreProjection::read(&logical(), CHANNEL, 0, doc).unwrap()
}
// TEST-ONLY independent writer. Intentionally bypasses prepare/check_cell so negative cases
// can create canonically framed, causally dishonest edits without weakening production code.
fn raw(
    before: &AutoCommit,
    epoch: u64,
    op: &ScoreOp,
    n: u128,
    who: u8,
) -> (AutoCommit, Change, DomainOp) {
    let domain = domain(op, n);
    let author = author(who);
    let mut doc = before
        .clone()
        .with_actor(ActorId::from(author.as_bytes().to_vec()));
    for (key, value) in header(&logical(), CHANNEL, epoch) {
        if doc.get(ROOT, &key).unwrap().is_none() {
            doc.put(ROOT, key, value).unwrap();
        }
    }
    let (key, bytes) = record(&domain, &author).unwrap();
    doc.put(ROOT, key, bytes).unwrap();
    doc.put(ROOT, format!("_p1/op/{}", hex(&domain.id(&author))), 1u64)
        .unwrap();
    doc.commit();
    let change = doc.get_last_local_change().unwrap();
    (doc, change, domain)
}
fn edit(before: &AutoCommit, op: &ScoreOp, n: u128, who: u8) -> AutoCommit {
    let (after, change, domain) = raw(before, 0, op, n, who);
    validate_score_change(&logical(), CHANNEL, 0, &domain, &change, before).unwrap();
    after
}
fn merge(before: &AutoCommit, changes: &[Change]) -> AutoCommit {
    let mut doc = before.clone();
    doc.apply_changes(changes.iter().cloned()).unwrap();
    doc
}

fn direct_value<T>(op: &ScoreOp, value: T, nonce: u128) -> ScoreValue<T> {
    let domain = domain(op, nonce);
    let source = decode_record(&logical(), &record(&domain, &author(1)).unwrap().1)
        .unwrap()
        .0;
    ScoreValue { source, value }
}

#[test]
fn score_caps_use_effective_slots_and_checkpoint_reports_every_omission() {
    let mut p = read(&empty());
    p.steps = Some(ScoreRegister {
        selected: direct_value(&ScoreOp::SetHeader(ScoreHeader::Steps(256)), 256, 1),
        conflicts: vec![],
    });
    for n in 0..=SCORE_MAX_CELLS {
        let (step, row) = (
            (n / usize::from(SCORE_ROWS)) as u16,
            (n % usize::from(SCORE_ROWS)) as u8,
        );
        let value = ScoreCell::Drum { pad: 0 };
        let op = ScoreOp::SetCell {
            step,
            row,
            value: value.clone(),
        };
        p.cells.insert(
            (step, row),
            ScoreSlot {
                register: ScoreRegister {
                    selected: direct_value(&op, Some(value), n as u128 + 2),
                    conflicts: vec![],
                },
            },
        );
    }
    let r = p.readiness().unwrap();
    assert_eq!(r.cells.len(), 1);
    assert!(r.cells.values().next().unwrap().count);
    let target = StudioTarget::Score {
        channel: CHANNEL,
        object: [9; 16],
    };
    assert!(matches!(
        target.local_policy(
            &StudioProjection::Score(Box::new(p.clone())),
            &domain(&drum(200), 9999),
            &author(1)
        ),
        Err(ReplError::EpochBound)
    ));
    assert!(target
        .local_policy(
            &StudioProjection::Score(Box::new(p.clone())),
            &domain(&ScoreOp::ClearCell { step: 200, row: 0 }, 9999),
            &author(1)
        )
        .is_ok());
    let seed = p.checkpoint([4; 32]).unwrap();
    let mut next = empty();
    next.apply_changes([Change::from_bytes(seed.bytes().to_vec()).unwrap()])
        .unwrap();
    assert_eq!(
        ScoreProjection::read(&logical(), CHANNEL, 1, &next)
            .unwrap()
            .cells
            .len(),
        SCORE_MAX_CELLS
    );
    assert!(p.checkpoint_omits_evidence().unwrap());
    for n in 0..=SCORE_MAX_PATCHES {
        let mut value = patch().value().clone();
        value["e"]["a"] = json!(n);
        let descriptor = StudioPatch::new(&value).unwrap();
        let id = descriptor.id();
        let op = ScoreOp::SetPatch {
            patch: id,
            descriptor: descriptor.clone(),
        };
        p.patches.insert(
            id,
            ScoreSlot {
                register: ScoreRegister {
                    selected: direct_value(&op, Some(descriptor), n as u128 + 10000),
                    conflicts: vec![],
                },
            },
        );
    }
    assert_eq!(p.readiness().unwrap().patches.len(), 1);
    let id = *p.patches.last_key_value().unwrap().0;
    let removal = direct_value(&ScoreOp::RemovePatch { patch: id }, None, 20000);
    // Even a raw Set winner is NOT an active recipe when a concurrent removal is retained.
    p.patches
        .get_mut(&id)
        .unwrap()
        .register
        .conflicts
        .push(removal);
    assert!(p.readiness().unwrap().patches.is_empty());
    let seed = p.checkpoint([4; 32]).unwrap();
    let mut next = empty();
    next.apply_changes([Change::from_bytes(seed.bytes().to_vec()).unwrap()])
        .unwrap();
    let next = ScoreProjection::read(&logical(), CHANNEL, 1, &next).unwrap();
    assert_eq!(next.patches.len(), SCORE_MAX_PATCHES);
    assert!(!next.patches.contains_key(&id));
}

#[test]
fn score_public_projection_encoding_bounds_data_and_rejects_forged_sources() {
    let mut p = read(&edit(&empty(), &drum(0), 1, 1));
    p.cells
        .get_mut(&(0, 0))
        .unwrap()
        .register
        .selected
        .source
        .op_id = [0; 32];
    assert!(p.checkpoint([4; 32]).is_err());
    let mut p = read(&empty());
    let title = "x".repeat(60_000);
    let op = ScoreOp::SetHeader(ScoreHeader::Title(title.clone()));
    let selected = direct_value(&op, title.clone(), 1);
    let mut conflicts: Vec<_> = (2..120)
        .map(|n| direct_value(&op, title.clone(), n))
        .collect();
    conflicts.sort_by_key(|v| v.source.op_id);
    p.title = Some(ScoreRegister {
        selected,
        conflicts,
    });
    assert!(matches!(p.readiness(), Err(ReplError::EpochBound)));
    assert!(matches!(p.checkpoint([4; 32]), Err(ReplError::EpochBound)));
    // Scope is also validated even when the public content happens to be empty.
    let mut p = read(&empty());
    p.document.logical_key.pop();
    assert!(p.checkpoint([4; 32]).is_err());
}

#[test]
fn score_defaults_have_no_invented_source_and_empty_cleanup_is_intent() {
    let p = read(&empty());
    assert_eq!(
        (p.effective_bpm(), p.effective_bpb(), p.effective_steps()),
        (120, 4, 16)
    );
    assert!(p.title.is_none() && p.bpm.is_none() && p.bpb.is_none() && p.steps.is_none());
    let doc = edit(&empty(), &ScoreOp::ClearCell { step: 255, row: 24 }, 1, 2);
    let doc = edit(
        &doc,
        &ScoreOp::RemovePatch {
            patch: patch().id(),
        },
        2,
        2,
    );
    let p = read(&doc);
    assert!(p.cells[&(255, 24)].removed());
    assert!(p.patches[&patch().id()].removed());
    assert!(p.readiness().unwrap().ready());
    assert!(p.checkpoint_omits_evidence().unwrap());
}

#[test]
fn score_cells_and_recipes_are_reusable_after_observed_removal() {
    let doc = edit(&empty(), &set_patch(), 1, 2);
    let doc = edit(&doc, &note(0, 0, None), 2, 2);
    let doc = edit(&doc, &ScoreOp::ClearCell { step: 0, row: 0 }, 3, 2);
    let doc = edit(
        &doc,
        &ScoreOp::RemovePatch {
            patch: patch().id(),
        },
        4,
        2,
    );
    assert!(read(&doc).cells[&(0, 0)].removed());
    let doc = edit(&doc, &set_patch(), 5, 2);
    let doc = edit(&doc, &note(0, 0, Some(4)), 6, 2);
    let p = read(&doc);
    assert!(p.readiness().unwrap().ready());
    assert_eq!(
        p.cells[&(0, 0)].value(),
        Some(&ScoreCell::Note {
            patch: patch().id(),
            len: Some(4)
        })
    );
    assert!(p.cells[&(0, 0)].register.conflicts.is_empty());
    assert!(p.patches[&patch().id()].register.conflicts.is_empty());
}

#[test]
fn score_removal_dominates_even_when_raw_am_winner_is_set_in_both_orders() {
    let base = edit(&empty(), &set_patch(), 1, 1);
    let (_, clear, clear_domain) = raw(&base, 0, &ScoreOp::ClearCell { step: 0, row: 0 }, 2, 2);
    let (_, set, set_domain) = raw(&base, 0, &note(0, 0, None), 3, 250);
    validate_score_change(&logical(), CHANNEL, 0, &clear_domain, &clear, &base).unwrap();
    validate_score_change(&logical(), CHANNEL, 0, &set_domain, &set, &base).unwrap();
    for changes in [[clear.clone(), set.clone()], [set.clone(), clear.clone()]] {
        let merged = merge(&base, &changes);
        let p = read(&merged);
        let slot = &p.cells[&(0, 0)];
        assert!(
            slot.register.selected.value.is_some(),
            "exercise AM Set winner, not just removal winner"
        );
        assert!(slot.value().is_none());
        assert_eq!(slot.removal_source().unwrap().author, author(2));
        assert_eq!(slot.register.conflicts.len(), 1);
        let repaired = edit(&merged, &note(0, 0, None), 4, 3);
        assert!(read(&repaired).cells[&(0, 0)].value().is_some());
    }
    let (_, remove, _) = raw(
        &base,
        0,
        &ScoreOp::RemovePatch {
            patch: patch().id(),
        },
        5,
        2,
    );
    let (_, readd, _) = raw(&base, 0, &set_patch(), 6, 250);
    let merged = merge(&base, &[remove, readd]);
    let p = read(&merged);
    assert!(p.patches[&patch().id()].register.selected.value.is_some());
    assert!(p.patches[&patch().id()].value().is_none());
    let (_, change, domain) = raw(&merged, 0, &note(0, 0, None), 7, 3);
    assert!(validate_score_change(&logical(), CHANNEL, 0, &domain, &change, &merged).is_err());
}

#[test]
fn score_partially_observed_clear_does_not_overwrite_unseen_clear() {
    let base = edit(&empty(), &set_patch(), 1, 1);
    let (one, a, _) = raw(&base, 0, &ScoreOp::ClearCell { step: 0, row: 0 }, 2, 2);
    let (_, b, _) = raw(&base, 0, &ScoreOp::ClearCell { step: 0, row: 0 }, 3, 3);
    let (_, set, domain) = raw(&one, 0, &note(0, 0, None), 4, 250);
    let receiver = merge(&base, &[a, b]);
    validate_score_change(&logical(), CHANNEL, 0, &domain, &set, &receiver).unwrap();
    let merged = merge(&receiver, &[set]);
    assert!(read(&merged).cells[&(0, 0)].removed());
}

#[test]
fn score_admission_uses_sender_recipe_and_extent_not_receiver_future() {
    let base = edit(
        &empty(),
        &ScoreOp::SetHeader(ScoreHeader::Title("base".into())),
        1,
        1,
    );
    let receiver = edit(&base, &set_patch(), 2, 2);
    let (_, change, domain) = raw(&base, 0, &note(0, 0, None), 3, 3);
    assert!(validate_score_change(&logical(), CHANNEL, 0, &domain, &change, &receiver).is_err());
    let base = edit(&receiver, &ScoreOp::SetHeader(ScoreHeader::Steps(32)), 4, 2);
    let (_, change, domain) = raw(&base, 0, &note(16, 0, Some(16)), 5, 3);
    let receiver = edit(&base, &ScoreOp::SetHeader(ScoreHeader::Steps(1)), 6, 2);
    let receiver = edit(
        &receiver,
        &ScoreOp::RemovePatch {
            patch: patch().id(),
        },
        7,
        2,
    );
    validate_score_change(&logical(), CHANNEL, 0, &domain, &change, &receiver).unwrap();
    let merged = merge(&receiver, &[change]);
    let readiness = read(&merged).readiness().unwrap();
    assert!(readiness.cells[&(16, 0)].extent && readiness.cells[&(16, 0)].recipe);
    let cleaned = edit(&merged, &ScoreOp::ClearCell { step: 16, row: 0 }, 8, 2);
    assert!(read(&cleaned).readiness().unwrap().ready());
    let (_, change, domain) = raw(&cleaned, 0, &drum(1), 9, 3);
    assert!(validate_score_change(&logical(), CHANNEL, 0, &domain, &change, &cleaned).is_err());
}

#[test]
fn score_overlap_and_header_conflicts_remain_explicit_and_editable() {
    let base = edit(&empty(), &set_patch(), 1, 1);
    let a = edit(&base, &note(0, 0, Some(8)), 2, 2);
    let b = edit(&base, &note(4, 0, Some(4)), 3, 3);
    let mut merged = a;
    merged.merge(&mut b.clone()).unwrap();
    let r = read(&merged).readiness().unwrap();
    assert!(r.cells[&(0, 0)].overlap && r.cells[&(4, 0)].overlap);
    let merged = edit(&merged, &ScoreOp::ClearCell { step: 4, row: 0 }, 4, 2);
    assert!(read(&merged).readiness().unwrap().ready());
    let a = edit(&merged, &ScoreOp::SetHeader(ScoreHeader::Bpm(100)), 5, 2);
    let b = edit(&merged, &ScoreOp::SetHeader(ScoreHeader::Bpm(140)), 6, 3);
    let mut merged = a;
    merged.merge(&mut b.clone()).unwrap();
    assert!(read(&merged).readiness().unwrap().header_conflicts);
    let merged = edit(&merged, &ScoreOp::SetHeader(ScoreHeader::Bpm(120)), 7, 2);
    assert!(read(&merged).readiness().unwrap().ready());
}

#[test]
fn score_exact_expansion_rejects_omitted_duplicate_and_cross_key_predecessors() {
    let base = edit(&empty(), &drum(0), 1, 1);
    let a = edit(&base, &drum(0), 2, 2);
    let b = edit(&base, &drum(0), 3, 3);
    let mut merged = a;
    merged.merge(&mut b.clone()).unwrap();
    let (_, change, domain) = raw(&merged, 0, &ScoreOp::ClearCell { step: 0, row: 0 }, 4, 4);
    let decoded = change.decode();
    let pos = decoded
        .operations
        .iter()
        .position(|o| o.key == Key::Map("c/0:0".into()))
        .unwrap();
    let preds: Vec<_> = decoded.operations[pos].pred.iter().cloned().collect();
    assert_eq!(preds.len(), 2);
    let (_, header_id) = merged.get(ROOT, "v").unwrap().unwrap();
    let ObjId::Id(n, actor, _) = header_id else {
        panic!()
    };
    for bad in [
        vec![],
        vec![preds[0].clone()],
        vec![preds[0].clone(), preds[0].clone()],
        vec![AmOpId(n, actor)],
    ] {
        let mut altered = decoded.clone();
        altered.operations[pos].pred = bad.into();
        assert!(validate_score_change(
            &logical(),
            CHANNEL,
            0,
            &domain,
            &Change::from(altered),
            &merged
        )
        .is_err());
    }
    let mut altered = decoded.clone();
    altered
        .operations
        .retain(|o| o.key != Key::Map(format!("_p1/op/{}", hex(&domain.id(&author(4)))).into()));
    assert!(validate_score_change(
        &logical(),
        CHANNEL,
        0,
        &domain,
        &Change::from(altered),
        &merged
    )
    .is_err());
    // An internally plausible record from a different author is not the signed actor's record.
    let mut altered = decoded;
    altered.operations[pos].action =
        OpType::Put(ScalarValue::Bytes(record(&domain, &author(5)).unwrap().1));
    assert!(validate_score_change(
        &logical(),
        CHANNEL,
        0,
        &domain,
        &Change::from(altered),
        &merged
    )
    .is_err());
}

#[test]
fn score_snapshot_retains_removal_and_seed_omits_whole_slot_without_resurrection() {
    let base = edit(&empty(), &set_patch(), 1, 1);
    let (_, clear, _) = raw(&base, 0, &ScoreOp::ClearCell { step: 0, row: 0 }, 2, 2);
    let (_, set, _) = raw(&base, 0, &note(0, 0, None), 3, 250);
    let merged = merge(&base, &[clear, set]);
    let p = read(&merged);
    let bytes = p
        .encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
        .unwrap();
    assert_eq!(
        ScoreProjection::decode_snapshot(&logical(), CHANNEL, 0, &bytes).unwrap(),
        p
    );
    for len in [0, 1, bytes.len() - 1] {
        assert!(ScoreProjection::decode_snapshot(&logical(), CHANNEL, 0, &bytes[..len]).is_err());
    }
    let mut bad = bytes.clone();
    bad[0] = 1;
    assert!(ScoreProjection::decode_snapshot(&logical(), CHANNEL, 0, &bad).is_err());
    assert!(ScoreProjection::decode_snapshot(&logical(), [8; 16], 0, &bytes).is_err());
    assert!(p.checkpoint_omits_evidence().unwrap());
    let seed = p.checkpoint([4; 32]).unwrap();
    let mut next = empty();
    next.apply_changes([Change::from_bytes(seed.bytes().to_vec()).unwrap()])
        .unwrap();
    let seeded = ScoreProjection::read(&logical(), CHANNEL, 1, &next).unwrap();
    assert!(seeded.cells.is_empty());
    assert!(seeded.patches[&patch().id()].value().is_some());
    let (after, change, domain) = raw(&next, 1, &note(0, 0, None), 4, 3);
    validate_score_change(&logical(), CHANNEL, 1, &domain, &change, &next).unwrap();
    assert!(ScoreProjection::read(&logical(), CHANNEL, 1, &after)
        .unwrap()
        .cells[&(0, 0)]
        .value()
        .is_some());
    let (_, reused, domain) = raw(&next, 1, &set_patch(), 1, 1);
    assert!(matches!(
        validate_score_change(&logical(), CHANNEL, 1, &domain, &reused, &next),
        Err(ReplError::IntentConflict)
    ));
    // Reusing the old physical epoch graph is not a seed/receipt installation.
    assert!(ScoreProjection::read(&logical(), CHANNEL, 1, &merged).is_err());
}

#[test]
fn score_post_seed_explicit_register_replaces_fallback_and_remains_reusable() {
    let base = edit(&empty(), &set_patch(), 1, 1);
    let base = edit(&base, &note(0, 0, None), 2, 1);
    let p = read(&base);
    let seed = p.checkpoint([4; 32]).unwrap();
    let mut next = empty();
    next.apply_changes([Change::from_bytes(seed.bytes().to_vec()).unwrap()])
        .unwrap();
    let (cleared, c, d) = raw(&next, 1, &ScoreOp::ClearCell { step: 0, row: 0 }, 3, 2);
    validate_score_change(&logical(), CHANNEL, 1, &d, &c, &next).unwrap();
    let slot = &ScoreProjection::read(&logical(), CHANNEL, 1, &cleared)
        .unwrap()
        .cells[&(0, 0)];
    assert!(slot.removed() && slot.register.conflicts.is_empty());
    let (retyped, c, d) = raw(&cleared, 1, &note(0, 0, Some(2)), 4, 2);
    validate_score_change(&logical(), CHANNEL, 1, &d, &c, &cleared).unwrap();
    assert!(ScoreProjection::read(&logical(), CHANNEL, 1, &retyped)
        .unwrap()
        .cells[&(0, 0)]
        .value()
        .is_some());
}
