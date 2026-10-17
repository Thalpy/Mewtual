use super::*;
use types::FlipnoteFrameProjection;

fn patch() -> FlipnoteOp {
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
    FlipnoteOp::decode(body.as_bytes()).unwrap()
}
fn patch_id() -> [u8; 32] {
    let FlipnoteOp::SetPatch { patch, .. } = patch() else {
        panic!()
    };
    patch
}
fn sfx(id: u8) -> FlipnoteOp {
    FlipnoteOp::SetSfx {
        sfx: [id; 16],
        frame: [1; 16],
        patch: patch_id(),
        note: 72,
        duration_ms: 375,
    }
}
fn export(id: u8) -> FlipnoteOp {
    FlipnoteOp::SetExport {
        export: [id; 16],
        cid: [8; 32],
        bytes: 500,
        expiry: StudioExpiry::Never,
    }
}
fn art(p: &StudioProjection) -> &FlipnoteFrameProjection {
    let StudioProjection::Flipnote(p) = p else {
        panic!()
    };
    p
}
fn art_mut(p: &mut StudioProjection) -> &mut FlipnoteFrameProjection {
    let StudioProjection::Flipnote(p) = p else {
        panic!()
    };
    p
}
fn build(f: &Fixture, operations: &[FlipnoteOp]) -> StudioProjection {
    f.projection(operations.iter().map(|o| o.encode().unwrap()).collect())
}
fn choice(p: &StudioProjection, operation: &FlipnoteOp) -> I {
    let value = art(p)
        .sound
        .records
        .values()
        .flat_map(|r| std::iter::once(&r.selected).chain(&r.conflicts))
        .find(|v| &v.value == operation)
        .unwrap();
    I::SoundRecord {
        value: value.source.op_id,
    }
}
fn with_frame(f: &Fixture) -> StudioProjection {
    f.projection(vec![insert(1, None, 1)])
}
fn with_patch(f: &Fixture) -> StudioProjection {
    f.projection(vec![insert(1, None, 1), patch().encode().unwrap()])
}

#[test]
fn studio_sound_restore_recovers_recipe_then_sfx_and_preserves_exact_authored_source() {
    let f = Fixture::new(false);
    let old = f.projection(vec![
        insert(1, None, 1),
        patch().encode().unwrap(),
        sfx(2).encode().unwrap(),
    ]);
    let empty = with_frame(&f);
    let ready = f.plan(&empty, &old, choice(&old, &patch()), M::Restore);
    assert_eq!(ready.disposition, D::Ready);
    assert_eq!(
        FlipnoteOp::decode(ready.body.as_ref().unwrap()).unwrap(),
        patch()
    );
    assert_eq!(ready.original_author, Some(f.device.device_id()));
    assert_eq!(ready.source_ops.len(), 1);
    assert_eq!(
        f.plan(&empty, &old, choice(&old, &sfx(2)), M::Restore)
            .disposition,
        D::MissingTarget
    );
    let ready = f.plan(&with_patch(&f), &old, choice(&old, &sfx(2)), M::Restore);
    assert_eq!(ready.disposition, D::Ready);
    assert_eq!(
        FlipnoteOp::decode(ready.body.as_ref().unwrap()).unwrap(),
        sfx(2)
    );
    assert_eq!(
        f.plan(&old, &old, choice(&old, &sfx(2)), M::Restore)
            .disposition,
        D::Unchanged
    );
    let no_frame = build(&f, &[patch()]);
    assert_eq!(
        f.plan(&no_frame, &old, choice(&old, &sfx(2)), M::Copy)
            .disposition,
        D::MissingTarget
    );
}

#[test]
fn studio_sound_recipe_remove_wins_and_only_copy_can_readd_or_choose_a_losing_set() {
    let f = Fixture::new(false);
    let old = build(&f, &[patch()]);
    let empty = build(&f, &[]);
    let removed = build(
        &f,
        &[patch(), FlipnoteOp::RemovePatch { patch: patch_id() }],
    );
    assert_eq!(
        f.plan(&removed, &old, choice(&old, &patch()), M::Restore)
            .disposition,
        D::Deleted
    );
    assert_eq!(
        f.plan(&removed, &old, choice(&old, &patch()), M::Copy)
            .disposition,
        D::Ready
    );
    let readded = build(
        &f,
        &[
            patch(),
            FlipnoteOp::RemovePatch { patch: patch_id() },
            patch(),
        ],
    );
    let snapshot = StudioRecovery::snapshot(
        &removed,
        None,
        RecoveryReason::Excluded,
        [0; 32],
        &BTreeMap::new(),
    )
    .unwrap();
    let retained =
        StudioRecovery::from_snapshot(&snapshot, removed.document(), removed.channel()).unwrap();
    assert_eq!(
        plan(
            &readded,
            &old,
            &[retained],
            choice(&old, &patch()),
            M::Restore,
            f.device.device_id(),
            PlanScope::SameDocument
        )
        .unwrap()
        .disposition,
        D::Unchanged,
        "older removal evidence must not relabel an already-present identical recipe as deleted"
    );
    let removal = FlipnoteOp::RemovePatch { patch: patch_id() };
    assert_eq!(
        f.plan(&empty, &removed, choice(&removed, &removal), M::Restore)
            .disposition,
        D::Conflict
    );
    assert_eq!(
        f.plan(&old, &removed, choice(&removed, &removal), M::Copy)
            .disposition,
        D::Ready
    );
    let mut conflicted = old.clone();
    art_mut(&mut conflicted)
        .sound
        .records
        .values_mut()
        .next()
        .unwrap()
        .conflicts
        .push(
            art(&removed)
                .sound
                .records
                .values()
                .next()
                .unwrap()
                .selected
                .clone(),
        );
    assert!(art(&conflicted).sound.patches().is_empty());
    assert_eq!(
        f.plan(
            &empty,
            &conflicted,
            choice(&conflicted, &patch()),
            M::Restore
        )
        .disposition,
        D::Conflict
    );
    assert_eq!(
        f.plan(&empty, &conflicted, choice(&conflicted, &patch()), M::Copy)
            .disposition,
        D::Ready
    );
}

#[test]
fn studio_sound_permanent_sfx_and_export_deletions_block_both_modes_even_after_compaction() {
    let f = Fixture::new(false);
    let old = f.projection(vec![
        insert(1, None, 1),
        patch().encode().unwrap(),
        sfx(2).encode().unwrap(),
        export(3).encode().unwrap(),
    ]);
    let removed = f.projection(vec![
        insert(1, None, 1),
        patch().encode().unwrap(),
        sfx(2).encode().unwrap(),
        export(3).encode().unwrap(),
        FlipnoteOp::RemoveSfx { sfx: [2; 16] }.encode().unwrap(),
        FlipnoteOp::RemoveExport { export: [3; 16] }
            .encode()
            .unwrap(),
    ]);
    let saved = StudioRecovery::snapshot(
        &removed,
        None,
        RecoveryReason::Excluded,
        [0; 32],
        &BTreeMap::new(),
    )
    .unwrap();
    let history =
        StudioRecovery::from_snapshot(&saved, removed.document(), removed.channel()).unwrap();
    for operation in [sfx(2), export(3)] {
        for mode in [M::Restore, M::Copy] {
            assert_eq!(
                f.plan(&removed, &old, choice(&old, &operation), mode)
                    .disposition,
                D::Deleted
            );
            assert_eq!(
                plan(
                    &with_patch(&f),
                    &old,
                    std::slice::from_ref(&history),
                    choice(&old, &operation),
                    mode,
                    f.device.device_id(),
                    PlanScope::SameDocument
                )
                .unwrap()
                .disposition,
                D::Deleted
            );
        }
    }
    for operation in [
        FlipnoteOp::RemoveSfx { sfx: [2; 16] },
        FlipnoteOp::RemoveExport { export: [3; 16] },
    ] {
        assert_eq!(
            f.plan(&old, &removed, choice(&removed, &operation), M::Restore)
                .disposition,
            D::Conflict
        );
        assert_eq!(
            f.plan(&old, &removed, choice(&removed, &operation), M::Copy)
                .disposition,
            D::Ready
        );
        assert_eq!(
            f.plan(&removed, &removed, choice(&removed, &operation), M::Copy)
                .disposition,
            D::Unchanged
        );
    }
}

#[test]
fn studio_sound_export_births_are_immutable_and_restore_cannot_choose_conflicting_source_births() {
    let f = Fixture::new(false);
    let old = build(&f, &[export(3)]);
    let empty = build(&f, &[]);
    assert_eq!(
        f.plan(&empty, &old, choice(&old, &export(3)), M::Restore)
            .disposition,
        D::Ready
    );
    assert_eq!(
        f.plan(&old, &old, choice(&old, &export(3)), M::Copy)
            .disposition,
        D::Unchanged
    );
    let mut different = export(3);
    if let FlipnoteOp::SetExport { bytes, .. } = &mut different {
        *bytes += 1;
    }
    let destination = build(&f, &[different.clone()]);
    for mode in [M::Restore, M::Copy] {
        assert_eq!(
            f.plan(&destination, &old, choice(&old, &export(3)), mode)
                .disposition,
            D::Conflict
        );
    }
    // Two independently authored births, not an illegal sequential replacement of one id.
    let competing = build(
        &f,
        &[FlipnoteOp::SetHeader(FlipnoteHeader::Fps(12)), different],
    );
    let mut collisions = old.clone();
    art_mut(&mut collisions)
        .sound
        .records
        .extend(art(&competing).sound.records.clone());
    assert_eq!(
        f.plan(
            &empty,
            &collisions,
            choice(&collisions, &export(3)),
            M::Restore
        )
        .disposition,
        D::Conflict
    );
    assert_eq!(
        f.plan(
            &empty,
            &collisions,
            choice(&collisions, &export(3)),
            M::Copy
        )
        .disposition,
        D::Ready
    );
}

#[test]
fn studio_sound_score_link_restore_is_additive_and_null_or_replacement_needs_copy() {
    let f = Fixture::new(false);
    let empty = build(&f, &[]);
    let link = FlipnoteOp::SetHeader(FlipnoteHeader::Score(Some([9; 16])));
    let unlink = FlipnoteOp::SetHeader(FlipnoteHeader::Score(None));
    let old = build(&f, std::slice::from_ref(&link));
    let cleared = build(&f, std::slice::from_ref(&unlink));
    assert_eq!(
        f.plan(&empty, &old, choice(&old, &link), M::Restore)
            .disposition,
        D::Ready,
        "custody must still probe actual Score"
    );
    assert_eq!(
        f.plan(&cleared, &old, choice(&old, &link), M::Restore)
            .disposition,
        D::Conflict
    );
    assert_eq!(
        f.plan(&cleared, &old, choice(&old, &link), M::Copy)
            .disposition,
        D::Ready
    );
    assert_eq!(
        f.plan(&old, &cleared, choice(&cleared, &unlink), M::Restore)
            .disposition,
        D::Conflict
    );
    assert_eq!(
        f.plan(&old, &cleared, choice(&cleared, &unlink), M::Copy)
            .disposition,
        D::Ready
    );
    assert_eq!(
        f.plan(&old, &old, choice(&old, &link), M::Restore)
            .disposition,
        D::Unchanged
    );
    assert!(plan(
        &empty,
        &old,
        &[],
        I::SoundRecord { value: [0xff; 32] },
        M::Copy,
        f.device.device_id(),
        PlanScope::SameDocument
    )
    .is_err());
}

#[test]
fn studio_sound_source_conflicts_and_destination_caps_are_not_silently_overwritten() {
    let f = Fixture::new(false);
    let old = f.projection(vec![
        insert(1, None, 1),
        patch().encode().unwrap(),
        sfx(2).encode().unwrap(),
    ]);
    let mut conflict = old.clone();
    let key = format!("s/{}", hex::encode([2; 16]));
    let mut alternative = art(&old).sound.records[&key].selected.clone();
    alternative.source.op_id = [0xfe; 32];
    if let FlipnoteOp::SetSfx { note, .. } = &mut alternative.value {
        *note = 79;
    }
    art_mut(&mut conflict)
        .sound
        .records
        .get_mut(&key)
        .unwrap()
        .conflicts
        .push(alternative);
    assert_eq!(
        f.plan(
            &with_patch(&f),
            &conflict,
            choice(&conflict, &sfx(2)),
            M::Restore
        )
        .disposition,
        D::Conflict
    );
    assert_eq!(
        f.plan(
            &with_patch(&f),
            &conflict,
            choice(&conflict, &sfx(2)),
            M::Copy
        )
        .disposition,
        D::Ready
    );
    let mut full = with_patch(&f);
    let template = art(&old).sound.records[&key].clone();
    for n in 1000..1000 + types::FLIPNOTE_MAX_SFX {
        let mut r = template.clone();
        let id = (n as u128).to_be_bytes();
        if let FlipnoteOp::SetSfx { sfx, .. } = &mut r.selected.value {
            *sfx = id;
        }
        art_mut(&mut full)
            .sound
            .records
            .insert(format!("s/{}", hex::encode(id)), r);
    }
    assert_eq!(
        f.plan(&full, &old, choice(&old, &sfx(2)), M::Copy)
            .disposition,
        D::Full
    );
    let export_old = build(&f, &[export(3)]);
    let mut full_exports = build(&f, &[]);
    for n in 100..100 + types::FLIPNOTE_MAX_EXPORTS {
        let op = export(n as u8);
        let single = build(&f, &[op]);
        art_mut(&mut full_exports)
            .sound
            .records
            .extend(art(&single).sound.records.clone());
    }
    assert_eq!(
        f.plan(
            &full_exports,
            &export_old,
            choice(&export_old, &export(3)),
            M::Restore
        )
        .disposition,
        D::Full
    );
}

#[test]
fn studio_sound_recipe_capacity_unknown_art_source_and_cross_family_choices_refuse() {
    let f = Fixture::new(false);
    let old = build(&f, &[patch()]);
    let mut full = build(&f, &[]);
    let template = art(&old).sound.records.values().next().unwrap().clone();
    for n in 0..types::FLIPNOTE_MAX_PATCHES {
        let mut r = template.clone();
        let hash = [n as u8; 32];
        if let FlipnoteOp::SetPatch { patch, .. } = &mut r.selected.value {
            *patch = hash;
        }
        art_mut(&mut full)
            .sound
            .records
            .insert(format!("p/{}", hex::encode(hash)), r);
    }
    assert_eq!(
        f.plan(&full, &old, choice(&old, &patch()), M::Copy)
            .disposition,
        D::Full
    );
    let title = build(
        &f,
        &[FlipnoteOp::SetHeader(FlipnoteHeader::Title("art".into()))],
    );
    let art_source = art(&title).title.as_ref().unwrap().selected.source.op_id;
    assert!(plan(
        &title,
        &title,
        &[],
        I::SoundRecord { value: art_source },
        M::Copy,
        f.device.device_id(),
        PlanScope::SameDocument
    )
    .is_err());
    let score_target = StudioTarget::Score {
        channel: f.target.channel(),
        object: [2; 16],
    };
    let score = f.projection_of(score_target, vec![]);
    assert!(plan(
        &score,
        &old,
        &[],
        choice(&old, &patch()),
        M::Copy,
        f.device.device_id(),
        PlanScope::SameDocument
    )
    .is_err());
}
