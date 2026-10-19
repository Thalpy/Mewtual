use super::*;
use types::{ScoreCell, ScoreHeader, ScoreOp, ScoreProjection};

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
