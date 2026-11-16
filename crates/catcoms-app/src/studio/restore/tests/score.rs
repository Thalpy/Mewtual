use super::*;
use types::{ScoreCell, ScoreHeader, ScoreLayer, ScoreLayerSlot, ScoreOp, ScoreProjection};

fn fixture() -> Fixture {
    let mut f = Fixture::new(false);
    f.target = StudioTarget::Score {
        channel: [1; 16],
        object: [2; 16],
    };
    f
}
fn score(p: &StudioProjection) -> &ScoreProjection {
    let StudioProjection::Score(p) = p else {
        panic!("score fixture")
    };
    p
}
fn cell(p: &StudioProjection) -> I {
    I::ScoreCell {
        step: 0,
        row: 0,
        value: score(p).cells[&(0, 0)].register.selected.source.op_id,
    }
}
fn drum() -> ScoreOp {
    ScoreOp::SetCell {
        step: 0,
        row: 0,
        value: ScoreCell::Drum { pad: 0 },
    }
}
fn build(f: &Fixture, operations: &[ScoreOp]) -> StudioProjection {
    f.projection(operations.iter().map(|o| o.encode().unwrap()).collect())
}

#[test]
fn score_restore_adds_missing_cells_but_copy_alone_can_replace_or_clear() {
    let f = fixture();
    let empty = build(&f, &[]);
    let old = build(&f, &[drum()]);
    let ready = f.plan(&empty, &old, cell(&old), M::Restore);
    assert_eq!(ready.disposition, D::Ready);
    assert_eq!(
        ScoreOp::decode(ready.body.as_ref().unwrap()).unwrap(),
        drum()
    );
    assert_eq!(
        ready.source_ops,
        vec![score(&old).cells[&(0, 0)].register.selected.source.op_id]
    );
    assert_eq!(
        f.plan(&old, &old, cell(&old), M::Restore).disposition,
        D::Unchanged
    );
    let cleared = build(&f, &[drum(), ScoreOp::ClearCell { step: 0, row: 0 }]);
    assert_eq!(
        f.plan(&cleared, &old, cell(&old), M::Restore).disposition,
        D::Conflict
    );
    assert_eq!(
        f.plan(&cleared, &old, cell(&old), M::Copy).disposition,
        D::Ready
    );
    assert_eq!(
        f.plan(&old, &cleared, cell(&cleared), M::Restore)
            .disposition,
        D::Conflict
    );
    assert_eq!(
        ScoreOp::decode(
            f.plan(&old, &cleared, cell(&cleared), M::Copy)
                .body
                .as_ref()
                .unwrap()
        )
        .unwrap(),
        ScoreOp::ClearCell { step: 0, row: 0 }
    );
    // A compacted-out clear remains negative evidence in retained destination recovery.
    let saved = StudioRecovery::snapshot(
        &cleared,
        None,
        RecoveryReason::Excluded,
        [0; 32],
        &BTreeMap::new(),
    )
    .unwrap();
    let recovery =
        StudioRecovery::from_snapshot(&saved, cleared.document(), cleared.channel()).unwrap();
    assert_eq!(
        plan(
            &empty,
            &old,
            &[recovery],
            cell(&old),
            M::Restore,
            f.device.device_id(),
            PlanScope::SameDocument
        )
        .unwrap()
        .disposition,
        D::Deleted
    );
}

#[test]
fn score_restore_recipe_first_current_extent_headers_and_unknown_sources() {
    let f = fixture();
    let empty = build(&f, &[]);
    let vectors: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../catcoms-replication/tests/fixtures/studio-score-ops-v1.json"
    )))
    .unwrap();
    let body = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "set_patch")
        .unwrap()["body"]
        .as_str()
        .unwrap();
    let patch = ScoreOp::decode(body.as_bytes()).unwrap();
    let ScoreOp::SetPatch { patch: hash, .. } = patch else {
        panic!()
    };
    let note = ScoreOp::SetCell {
        step: 0,
        row: 0,
        value: ScoreCell::Note {
            patch: hash,
            len: Some(8),
            volume: None,
        },
    };
    let old = build(
        &f,
        &[
            patch.clone(),
            note,
            ScoreOp::SetHeader(ScoreHeader::Bpm(90)),
        ],
    );
    assert_eq!(
        f.plan(&empty, &old, cell(&old), M::Restore).disposition,
        D::MissingTarget
    );
    let pi = I::ScorePatch {
        patch: hash,
        value: score(&old).patches[&hash].register.selected.source.op_id,
    };
    assert_eq!(f.plan(&empty, &old, pi, M::Restore).disposition, D::Ready);
    let current = build(&f, std::slice::from_ref(&patch));
    assert_eq!(
        f.plan(&current, &old, cell(&old), M::Restore).disposition,
        D::Ready
    );
    let short = build(
        &f,
        &[patch.clone(), ScoreOp::SetHeader(ScoreHeader::Steps(4))],
    );
    assert_eq!(
        f.plan(&short, &old, cell(&old), M::Copy).disposition,
        D::Conflict
    );
    let removed = build(&f, &[patch, ScoreOp::RemovePatch { patch: hash }]);
    assert_eq!(
        f.plan(&removed, &old, pi, M::Restore).disposition,
        D::Conflict
    );
    assert_eq!(f.plan(&removed, &old, pi, M::Copy).disposition, D::Ready);
    let hi = I::ScoreHeader {
        field: StudioScoreHeaderField::Bpm,
        value: score(&old).bpm.as_ref().unwrap().selected.source.op_id,
    };
    assert_eq!(f.plan(&empty, &old, hi, M::Restore).disposition, D::Ready);
    let edited = build(&f, &[ScoreOp::SetHeader(ScoreHeader::Bpm(130))]);
    assert_eq!(
        f.plan(&edited, &old, hi, M::Restore).disposition,
        D::Conflict
    );
    assert_eq!(f.plan(&edited, &old, hi, M::Copy).disposition, D::Ready);
    assert!(plan(
        &empty,
        &old,
        &[],
        I::ScoreCell {
            step: 0,
            row: 0,
            value: [0; 32]
        },
        M::Copy,
        f.device.device_id(),
        PlanScope::SameDocument
    )
    .is_err());
    let art = Fixture::new(false).projection(vec![]);
    assert!(plan(
        &empty,
        &art,
        &[],
        cell(&old),
        M::Copy,
        f.device.device_id(),
        PlanScope::CrossDocument
    )
    .is_err());
}

fn vector_patch() -> ScoreOp {
    let vectors: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../catcoms-replication/tests/fixtures/studio-score-ops-v1.json"
    )))
    .unwrap();
    let body = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "set_patch")
        .unwrap()["body"]
        .as_str()
        .unwrap();
    ScoreOp::decode(body.as_bytes()).unwrap()
}
fn score_mut(p: &mut StudioProjection) -> &mut ScoreProjection {
    let StudioProjection::Score(p) = p else {
        panic!("score fixture")
    };
    p
}
const DRUMS: ScoreLayer = ScoreLayer::Drums;
fn layered(pad: u8) -> ScoreOp {
    ScoreOp::SetNote {
        step: 0,
        row: 0,
        value: ScoreCell::Drum { pad },
    }
}
fn clear_layer() -> ScoreOp {
    ScoreOp::ClearNote {
        step: 0,
        row: 0,
        layer: DRUMS,
    }
}
/// A layered-note choice for `(0,0)/d`, naming the alternative `pick` selects from the version.
fn note_item(
    p: &StudioProjection,
    pick: impl Fn(&ScoreLayerSlot) -> &types::ScoreValue<Option<ScoreCell>>,
) -> I {
    I::ScoreNote {
        step: 0,
        row: 0,
        layer: DRUMS,
        value: pick(&score(p).layers[&(0, 0, DRUMS)]).source.op_id,
    }
}
fn shown(slot: &ScoreLayerSlot) -> &types::ScoreValue<Option<ScoreCell>> {
    slot.shown_alternative().unwrap()
}
fn selected(slot: &ScoreLayerSlot) -> &types::ScoreValue<Option<ScoreCell>> {
    &slot.register().selected
}
/// Add `extra`'s (0,0)/d selected alternative to `p`'s slot as a concurrent value. The fixture
/// edits sequentially, so a concurrent register is assembled from two genuine sources (distinct
/// positions give distinct nonces and op ids).
fn with_concurrent(p: &mut StudioProjection, extra: &StudioProjection) {
    let alternative = score(extra).layers[&(0, 0, DRUMS)]
        .register()
        .selected
        .clone();
    let p = score_mut(p);
    let mut register = p.layers[&(0, 0, DRUMS)].register().clone();
    register.conflicts.push(alternative);
    register.conflicts.sort_by_key(|v| v.source.op_id);
    p.layers
        .insert((0, 0, DRUMS), ScoreLayerSlot::new(register));
}

#[test]
fn score_note_restore_authors_the_shown_value_and_copy_alone_replaces_or_clears() {
    let f = fixture();
    let empty = build(&f, &[]);
    let old = build(&f, &[layered(3)]);
    let ready = f.plan(&empty, &old, note_item(&old, shown), M::Restore);
    assert_eq!(ready.disposition, D::Ready);
    assert_eq!(
        ScoreOp::decode(ready.body.as_ref().unwrap()).unwrap(),
        layered(3)
    );
    assert_eq!(
        ready.source_ops,
        vec![
            score(&old).layers[&(0, 0, DRUMS)]
                .register()
                .selected
                .source
                .op_id
        ]
    );
    assert_eq!(
        f.plan(&old, &old, note_item(&old, shown), M::Restore)
            .disposition,
        D::Unchanged
    );
    // A present-day clear is a deletion: Restore will not resurrect it, Copy may.
    let cleared = build(&f, &[layered(3), clear_layer()]);
    assert_eq!(
        f.plan(&cleared, &old, note_item(&old, shown), M::Restore)
            .disposition,
        D::Conflict
    );
    assert_eq!(
        f.plan(&cleared, &old, note_item(&old, shown), M::Copy)
            .disposition,
        D::Ready
    );
    // A different note already playing there is replaced only by Copy.
    let other = build(&f, &[layered(5)]);
    assert_eq!(
        f.plan(&other, &old, note_item(&old, shown), M::Restore)
            .disposition,
        D::Conflict
    );
    // A recovered clear: never by Restore; Copy authors ClearNote, and a no-op where nothing plays.
    let clear = note_item(&cleared, selected);
    assert_eq!(
        f.plan(&old, &cleared, clear, M::Restore).disposition,
        D::Conflict
    );
    let copy = f.plan(&old, &cleared, clear, M::Copy);
    assert_eq!(copy.disposition, D::Ready);
    assert_eq!(
        ScoreOp::decode(copy.body.as_ref().unwrap()).unwrap(),
        clear_layer()
    );
    assert_eq!(
        f.plan(&empty, &cleared, clear, M::Copy).disposition,
        D::Unchanged
    );
    // Retained destination history showing the slot cleared blocks resurrection.
    let saved = StudioRecovery::snapshot(
        &cleared,
        None,
        RecoveryReason::Excluded,
        [0; 32],
        &BTreeMap::new(),
    )
    .unwrap();
    let recovery =
        StudioRecovery::from_snapshot(&saved, cleared.document(), cleared.channel()).unwrap();
    assert_eq!(
        plan(
            &empty,
            &old,
            &[recovery],
            note_item(&old, shown),
            M::Restore,
            f.device.device_id(),
            PlanScope::SameDocument
        )
        .unwrap()
        .disposition,
        D::Deleted
    );
    // Unknown slots and values are errors, not proposals.
    for item in [
        I::ScoreNote {
            step: 0,
            row: 0,
            layer: DRUMS,
            value: [0; 32],
        },
        I::ScoreNote {
            step: 0,
            row: 0,
            layer: ScoreLayer::Sound([1; 32]),
            value: score(&old).layers[&(0, 0, DRUMS)]
                .register()
                .selected
                .source
                .op_id,
        },
    ] {
        assert!(plan(
            &empty,
            &old,
            &[],
            item,
            M::Copy,
            f.device.device_id(),
            PlanScope::SameDocument
        )
        .is_err());
    }
}

#[test]
fn score_note_restore_takes_only_the_shown_value_of_a_concurrent_slot() {
    let f = fixture();
    let empty = build(&f, &[]);
    let title = || ScoreOp::SetHeader(ScoreHeader::Title("t".into()));
    // Two live values: restore re-creates only the shown one; another needs Copy.
    let mut old = build(&f, &[layered(3)]);
    with_concurrent(&mut old, &build(&f, &[title(), layered(7)]));
    let slot = &score(&old).layers[&(0, 0, DRUMS)];
    assert_eq!(slot.live().len(), 2);
    let other = slot
        .alternatives()
        .find(|v| v.source.op_id != slot.shown_alternative().unwrap().source.op_id)
        .unwrap()
        .source
        .op_id;
    let ready = f.plan(&empty, &old, note_item(&old, shown), M::Restore);
    assert_eq!(ready.disposition, D::Ready);
    assert_eq!(
        ScoreOp::decode(ready.body.as_ref().unwrap()).unwrap(),
        ScoreOp::SetNote {
            step: 0,
            row: 0,
            value: slot.shown().unwrap().clone()
        }
    );
    let item = I::ScoreNote {
        step: 0,
        row: 0,
        layer: DRUMS,
        value: other,
    };
    assert_eq!(
        f.plan(&empty, &old, item, M::Restore).disposition,
        D::Conflict
    );
    assert_eq!(f.plan(&empty, &old, item, M::Copy).disposition, D::Ready);
    // Current holding the same two live values is NOT unchanged: a SetNote would drop one.
    assert_eq!(
        f.plan(&old, &old, note_item(&old, shown), M::Copy)
            .disposition,
        D::Ready
    );
    // A version whose register selected a clear still shows (and restores) its live value:
    // add wins, the clear does not delete the concurrent set.
    let mut old = build(&f, &[layered(3), clear_layer()]);
    with_concurrent(&mut old, &build(&f, &[title(), title(), layered(9)]));
    let slot = &score(&old).layers[&(0, 0, DRUMS)];
    assert!(slot.register().selected.value.is_none() && !slot.cleared());
    let ready = f.plan(&empty, &old, note_item(&old, shown), M::Restore);
    assert_eq!(ready.disposition, D::Ready);
    assert_eq!(
        ScoreOp::decode(ready.body.as_ref().unwrap()).unwrap(),
        layered(9)
    );
}

#[test]
fn score_note_restore_checks_recipe_extent_and_the_combined_cap() {
    let f = fixture();
    let patch = vector_patch();
    let ScoreOp::SetPatch { patch: hash, .. } = patch else {
        panic!()
    };
    let note = ScoreOp::SetNote {
        step: 0,
        row: 0,
        value: ScoreCell::Note {
            patch: hash,
            len: Some(8),
            volume: Some(40),
        },
    };
    let old = build(&f, &[patch.clone(), note.clone()]);
    let layer = ScoreLayer::Sound(hash);
    let item = I::ScoreNote {
        step: 0,
        row: 0,
        layer,
        value: score(&old).layers[&(0, 0, layer)]
            .register()
            .selected
            .source
            .op_id,
    };
    assert_eq!(
        f.plan(&build(&f, &[]), &old, item, M::Restore).disposition,
        D::MissingTarget
    );
    let current = build(&f, std::slice::from_ref(&patch));
    let ready = f.plan(&current, &old, item, M::Restore);
    assert_eq!(ready.disposition, D::Ready);
    assert_eq!(ScoreOp::decode(ready.body.as_ref().unwrap()).unwrap(), note);
    let short = build(&f, &[patch, ScoreOp::SetHeader(ScoreHeader::Steps(4))]);
    assert_eq!(f.plan(&short, &old, item, M::Copy).disposition, D::Conflict);

    // Full counts legacy cells and live layer slots together, for both item kinds.
    let drums = build(&f, &[layered(3)]);
    let legacy_old = build(&f, &[drum()]);
    let mut now = build(&f, &[]);
    let cell_slot = score(&legacy_old).cells[&(0, 0)].clone();
    let layer_slot = score(&drums).layers[&(0, 0, DRUMS)].clone();
    let p = score_mut(&mut now);
    for n in 0..types::SCORE_MAX_CELLS - 1 {
        p.cells
            .insert(((n / 25 + 1) as u16, (n % 25) as u8), cell_slot.clone());
    }
    p.layers.insert((255, 24, DRUMS), layer_slot.clone());
    assert_eq!(
        f.plan(&now, &drums, note_item(&drums, shown), M::Restore)
            .disposition,
        D::Full
    );
    assert_eq!(
        f.plan(&now, &legacy_old, cell(&legacy_old), M::Restore)
            .disposition,
        D::Full,
        "a legacy restore counts live layer slots too"
    );
    // A cleared layer slot does not occupy the cap.
    let cleared = build(&f, &[layered(3), clear_layer()]);
    let p = score_mut(&mut now);
    p.layers.insert(
        (255, 24, DRUMS),
        score(&cleared).layers[&(0, 0, DRUMS)].clone(),
    );
    assert_eq!(
        f.plan(&now, &drums, note_item(&drums, shown), M::Restore)
            .disposition,
        D::Ready
    );
}

#[test]
fn score_restore_retains_conflicts_and_refuses_growth_past_caps() {
    let f = fixture();
    let old = build(&f, &[drum()]);
    let mut now = build(&f, &[]);
    let StudioProjection::Score(ref mut p) = now else {
        panic!()
    };
    let slot = score(&old).cells[&(0, 0)].clone();
    for step in 0..256 {
        for row in 0..16 {
            p.cells.insert((step, row), slot.clone());
        }
    }
    // Test the planner's capacity seam without thousands of cryptographic edits. The source
    // remains genuine; production projections additionally validate each coordinate/provenance.
    p.cells.remove(&(0, 0));
    p.cells.insert((255, 24), slot.clone());
    assert_eq!(
        f.plan(&now, &old, cell(&old), M::Restore).disposition,
        D::Full
    );
    let StudioProjection::Score(ref mut p) = now else {
        panic!()
    };
    p.cells.insert((0, 0), slot.clone());
    p.cells
        .get_mut(&(0, 0))
        .unwrap()
        .register
        .conflicts
        .push(slot.register.selected);
    assert_eq!(
        f.plan(&now, &old, cell(&old), M::Restore).disposition,
        D::Conflict
    );
}
