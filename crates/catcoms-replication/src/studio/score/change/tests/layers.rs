//! Layered notes (docs/design-score-layers.md sections 1-5): add-wins merging through the real
//! causal validator, the 73-byte key bound, the snapshot trailer, compaction and admission.
use super::*;

fn sound() -> ScoreLayer {
    ScoreLayer::Sound(patch().id())
}
fn sound_note(len: Option<u16>, volume: Option<u8>) -> ScoreCell {
    ScoreCell::Note {
        patch: patch().id(),
        len,
        volume,
    }
}
fn set_note(step: u16, row: u8, value: ScoreCell) -> ScoreOp {
    ScoreOp::SetNote { step, row, value }
}
fn clear_note(step: u16, row: u8, layer: ScoreLayer) -> ScoreOp {
    ScoreOp::ClearNote { step, row, layer }
}
/// TEST-ONLY layer slot from (value, nonce) pairs, the first selected; sources are genuine.
fn layer_slot(
    step: u16,
    row: u8,
    layer: ScoreLayer,
    values: &[(Option<ScoreCell>, u128)],
) -> ScoreLayerSlot {
    let mut alternatives: Vec<_> = values
        .iter()
        .map(|(value, nonce)| {
            let op = match value {
                Some(cell) => set_note(step, row, cell.clone()),
                None => clear_note(step, row, layer),
            };
            direct_value(&op, value.clone(), *nonce)
        })
        .collect();
    let selected = alternatives.remove(0);
    alternatives.sort_by_key(|v| v.source.op_id);
    ScoreLayerSlot::new(ScoreRegister {
        selected,
        conflicts: alternatives,
    })
}
fn steps_header(p: &mut ScoreProjection, steps: u16, nonce: u128) {
    p.steps = Some(ScoreRegister {
        selected: direct_value(&ScoreOp::SetHeader(ScoreHeader::Steps(steps)), steps, nonce),
        conflicts: vec![],
    });
}
/// Install a checkpoint seed as the next epoch's genuine baseline and read it back.
fn seeded(p: &ScoreProjection) -> (AutoCommit, ScoreProjection) {
    let seed = p.checkpoint([4; 32]).unwrap();
    let mut next = empty();
    next.apply_changes([Change::from_bytes(seed.bytes().to_vec()).unwrap()])
        .unwrap();
    let read = ScoreProjection::read(&logical(), CHANNEL, p.epoch + 1, &next).unwrap();
    (next, read)
}

#[test]
fn layered_note_add_wins_against_a_concurrent_clear_in_both_automerge_orders() {
    let base = edit(&empty(), &set_patch(), 1, 1);
    let first = sound_note(Some(2), None);
    let base = edit(&base, &set_note(0, 0, first.clone()), 2, 1);
    let replacement = sound_note(Some(4), Some(70));
    // Both authors observed `first`. Run with each as the Automerge winner (the higher actor
    // wins the raw register), so `shown()` is exercised with a live AND a clear `selected`.
    for (clearer, setter) in [(2, 250), (250, 2)] {
        let (_, clear, clear_domain) = raw(&base, 0, &clear_note(0, 0, sound()), 3, clearer);
        let (_, set, set_domain) = raw(&base, 0, &set_note(0, 0, replacement.clone()), 4, setter);
        validate_score_change(&logical(), CHANNEL, 0, &clear_domain, &clear, &base).unwrap();
        validate_score_change(&logical(), CHANNEL, 0, &set_domain, &set, &base).unwrap();
        for changes in [[clear.clone(), set.clone()], [set.clone(), clear.clone()]] {
            let p = read(&merge(&base, &changes));
            let slot = &p.layers[&(0, 0, sound())];
            // The clear removed only what its author saw (`first`); the concurrent set lives.
            assert_eq!(slot.register().selected.value.is_none(), clearer == 250);
            assert_eq!(slot.live(), vec![&replacement]);
            assert!(!slot.cleared());
            assert_eq!(slot.shown(), Some(&replacement));
            let voices = p.voices().unwrap();
            assert_eq!(voices.play.len(), 1);
            assert_eq!(voices.play[0].value, replacement);
            // One live value: not a conflict, nothing to review.
            assert!(p.readiness().unwrap().ready());
        }
    }
    // Two concurrent sets of different notes in one layer slot BOTH play, and say so.
    let (_, a, _) = raw(&base, 0, &set_note(0, 0, sound_note(Some(3), None)), 5, 2);
    let (_, b, _) = raw(&base, 0, &set_note(0, 0, sound_note(None, Some(10))), 6, 3);
    let both = merge(&base, &[a, b]);
    let p = read(&both);
    assert_eq!(p.layers[&(0, 0, sound())].live().len(), 2);
    assert_eq!(p.voices().unwrap().play.len(), 2);
    let r = p.readiness().unwrap();
    assert!(r.layers[&(0, 0, sound())].conflict && r.layers[&(0, 0, sound())].overlap);
    assert!(r.cells.is_empty());
    // A clear that observed every value clears the slot; the slot is then reusable.
    let cleared = edit(&both, &clear_note(0, 0, sound()), 7, 4);
    let p = read(&cleared);
    assert!(p.layers[&(0, 0, sound())].cleared());
    assert!(p.layers[&(0, 0, sound())].shown().is_none());
    assert!(p.voices().unwrap().play.is_empty());
    let again = edit(&cleared, &set_note(0, 0, first.clone()), 8, 4);
    assert_eq!(read(&again).layers[&(0, 0, sound())].live(), vec![&first]);
    // A legacy cell at the same coordinate keeps removal wins: a concurrent clear removes it.
    let (_, set, _) = raw(&base, 0, &note(0, 0, None), 9, 250);
    let (_, clear, _) = raw(&base, 0, &ScoreOp::ClearCell { step: 0, row: 0 }, 10, 2);
    let p = read(&merge(&base, &[set, clear]));
    assert!(p.cells[&(0, 0)].removed());
    assert_eq!(
        p.voices().unwrap().play.len(),
        1,
        "only the layered note plays"
    );
}

#[test]
fn set_note_faces_set_cell_admission_and_clear_note_is_always_admissible() {
    let base = edit(
        &empty(),
        &ScoreOp::SetHeader(ScoreHeader::Title("base".into())),
        1,
        1,
    );
    // No active recipe at the sender's frontier: refused, exactly like set_cell.
    let (_, change, d) = raw(&base, 0, &set_note(0, 0, sound_note(None, None)), 2, 3);
    assert!(validate_score_change(&logical(), CHANNEL, 0, &d, &change, &base).is_err());
    assert!(prepare(&read(&base), &d, &author(3)).is_err());
    // A clear needs nothing: absent slot, gone sound.
    let (_, change, d) = raw(&base, 0, &clear_note(255, 24, sound()), 3, 3);
    validate_score_change(&logical(), CHANNEL, 0, &d, &change, &base).unwrap();
    prepare(&read(&base), &d, &author(3)).unwrap();
    let with = edit(&base, &set_patch(), 4, 2);
    let with = edit(&with, &ScoreOp::SetHeader(ScoreHeader::Steps(32)), 5, 2);
    let (_, change, d) = raw(&with, 0, &set_note(16, 0, sound_note(Some(16), None)), 6, 3);
    validate_score_change(&logical(), CHANNEL, 0, &d, &change, &with).unwrap();
    let (_, change, d) = raw(&with, 0, &set_note(16, 0, sound_note(Some(17), None)), 7, 3);
    assert!(
        validate_score_change(&logical(), CHANNEL, 0, &d, &change, &with).is_err(),
        "a layered hold must fit the header length at the sender's frontier"
    );
    let (_, change, d) = raw(&with, 0, &set_note(32, 0, ScoreCell::Drum { pad: 0 }), 8, 3);
    assert!(validate_score_change(&logical(), CHANNEL, 0, &d, &change, &with).is_err());
    let (_, change, d) = raw(&with, 0, &set_note(31, 0, ScoreCell::Drum { pad: 0 }), 9, 3);
    validate_score_change(&logical(), CHANNEL, 0, &d, &change, &with).unwrap();
}

#[test]
fn reader_and_validator_accept_the_73_byte_key_and_refuse_a_74_byte_key() {
    // Hand-built Automerge: the op-vector file holds bodies, not keys.
    let id = hex(&patch().id());
    let doc = edit(&empty(), &clear_note(255, 24, sound()), 1, 2);
    let key = format!("n/255:24/{id}");
    assert_eq!(key.len(), 73);
    assert!(doc.get(ROOT, &key).unwrap().is_some());
    assert!(read(&doc).layers[&(255, 24, sound())].cleared());
    // The same record bytes under a 74-byte key are refused before the value is decoded.
    let (_, bytes) = record(&domain(&clear_note(255, 24, sound()), 2), &author(2)).unwrap();
    let mut long = doc.clone();
    long.put(ROOT, format!("{key}0"), bytes.clone()).unwrap();
    long.commit();
    assert!(matches!(
        ScoreProjection::read(&logical(), CHANNEL, 0, &long),
        Err(ReplError::EpochBound)
    ));
    // A 73-byte key that is not the canonical spelling of its record is malformed.
    let mut upper = doc.clone();
    upper
        .put(ROOT, format!("n/255:24/{}", id.to_uppercase()), bytes)
        .unwrap();
    upper.commit();
    assert!(matches!(
        ScoreProjection::read(&logical(), CHANNEL, 0, &upper),
        Err(ReplError::Malformed)
    ));
    // A change smuggling an extra 74-byte key beside a valid entry fails exact expansion.
    let d = domain(&clear_note(0, 0, ScoreLayer::Drums), 3);
    let mut branch = doc
        .clone()
        .with_actor(ActorId::from(author(3).as_bytes().to_vec()));
    let (entry, entry_bytes) = record(&d, &author(3)).unwrap();
    branch.put(ROOT, entry, entry_bytes).unwrap();
    branch
        .put(ROOT, format!("_p1/op/{}", hex(&d.id(&author(3)))), 1u64)
        .unwrap();
    branch.put(ROOT, format!("{key}0"), 1u64).unwrap();
    branch.commit();
    let change = branch.get_last_local_change().unwrap();
    assert!(validate_score_change(&logical(), CHANNEL, 0, &d, &change, &doc).is_err());
}

/// A score holding every layer shape a seed can carry: a layered note with a level and a hold,
/// a layered drum, a slot whose clear the Automerge register selected over a live value, a
/// concurrent pair of different notes, a cleared slot, and a legacy note beside them.
fn layered_projection() -> ScoreProjection {
    let base = edit(&empty(), &set_patch(), 1, 1);
    let base = edit(&base, &set_note(2, 3, sound_note(Some(4), Some(60))), 2, 1);
    let base = edit(&base, &note(8, 0, None), 3, 1);
    let base = edit(&base, &set_note(5, 0, ScoreCell::Drum { pad: 2 }), 4, 1);
    let base = edit(&base, &set_note(9, 9, ScoreCell::Drum { pad: 1 }), 5, 1);
    let base = edit(&base, &clear_note(9, 9, ScoreLayer::Drums), 6, 1);
    let base = edit(&base, &set_note(1, 1, sound_note(None, None)), 7, 1);
    // Clear by the higher actor (wins the raw register) against a concurrent re-set.
    let (_, clear, _) = raw(&base, 0, &clear_note(1, 1, sound()), 8, 250);
    let (_, set, _) = raw(&base, 0, &set_note(1, 1, sound_note(Some(2), None)), 9, 2);
    let (_, a, _) = raw(&base, 0, &set_note(6, 1, sound_note(None, Some(10))), 10, 4);
    let (_, b, _) = raw(&base, 0, &set_note(6, 1, sound_note(None, Some(20))), 11, 3);
    read(&merge(&base, &[clear, set, a, b]))
}

#[test]
fn layer_trailer_round_trips_refuses_tampering_and_is_absent_without_layers() {
    let p = layered_projection();
    assert_eq!(p.layers.len(), 5);
    let bytes = p
        .encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
        .unwrap();
    assert_eq!(
        ScoreProjection::decode_snapshot(&logical(), CHANNEL, 0, &bytes).unwrap(),
        p
    );
    // The layer-free prefix is exactly the pre-layer encoding of the same score.
    let mut free = p.clone();
    free.layers.clear();
    let prefix = free
        .encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
        .unwrap();
    assert_eq!(&bytes[..prefix.len()], prefix.as_slice());
    let at = prefix.len();
    assert_eq!(bytes[at], 0x01, "section tag");
    assert_eq!(&bytes[at + 1..at + 5], &5u32.to_be_bytes(), "entry count");
    let decode = |b: &[u8]| ScoreProjection::decode_snapshot(&logical(), CHANNEL, 0, b);
    for tag in [0x00, 0x02, 0xff] {
        let mut bad = bytes.clone();
        bad[at] = tag;
        assert!(decode(&bad).is_err(), "tag {tag}");
    }
    let mut empty_section = prefix.clone();
    empty_section.push(0x01);
    empty_section.extend_from_slice(&0u32.to_be_bytes());
    assert!(matches!(decode(&empty_section), Err(ReplError::Malformed)));
    let mut bare_tag = prefix.clone();
    bare_tag.push(0x01);
    assert!(decode(&bare_tag).is_err());
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(decode(&extra).is_err());
    for len in [at + 1, at + 5, bytes.len() - 1] {
        assert!(decode(&bytes[..len]).is_err());
    }
    // A duplicated (not strictly ascending) entry: claim one more and repeat the last entry.
    let one = {
        let mut q = p.clone();
        let last = q.layers.pop_last().unwrap();
        q.layers.clear();
        q.layers.insert(last.0, last.1);
        q.encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
            .unwrap()
    };
    let entry = one[at + 5..].to_vec();
    let mut duplicate = one[..at + 1].to_vec();
    duplicate.extend_from_slice(&2u32.to_be_bytes());
    duplicate.extend_from_slice(&entry);
    duplicate.extend_from_slice(&entry);
    assert!(matches!(decode(&duplicate), Err(ReplError::Malformed)));
    // An unknown layer tag inside an entry.
    let mut bad_layer = one.clone();
    bad_layer[at + 5 + 8 + 1] = 2;
    assert!(decode(&bad_layer).is_err());
    // A note filed under another sound's layer is refused on encode (and so on decode).
    let mut wrong = p.clone();
    let slot = wrong.layers.remove(&(5, 0, ScoreLayer::Drums)).unwrap();
    wrong.layers.insert((5, 0, sound()), slot);
    assert!(matches!(
        wrong.encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES),
        Err(ReplError::Malformed)
    ));
    // A forged provenance is refused like any other score source.
    let mut forged = p.clone();
    let key = (2, 3, sound());
    let mut register = forged.layers[&key].register().clone();
    register.selected.source.op_id = [0; 32];
    forged.layers.insert(key, ScoreLayerSlot::new(register));
    assert!(forged.checkpoint([4; 32]).is_err());
}

#[test]
fn layered_seed_golden_vector_pins_the_trailer_bytes() {
    let p = layered_projection();
    // Independent of the codec: walk the seeded payload's trailer by hand against the layout in
    // docs/design-score-layers.md section 5 (tag 0x01, u32 count, then per entry a u64 step, a
    // u8 row, a layer tag with 32 raw id bytes for a sound, then the register).
    let (_, installed) = seeded(&p);
    let payload = installed
        .encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
        .unwrap();
    let mut free = installed.clone();
    free.layers.clear();
    let at = free
        .encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
        .unwrap()
        .len();
    // Compaction dropped the cleared (9,9)/d; four entries remain, in (step, row, layer) order.
    assert_eq!(payload[at], 0x01);
    assert_eq!(&payload[at + 1..at + 5], &4u32.to_be_bytes());
    let entry = &payload[at + 5..];
    assert_eq!(&entry[..8], &1u64.to_be_bytes(), "first entry is step 1");
    assert_eq!(entry[8], 1, "row 1");
    assert_eq!(entry[9], 1, "a sound layer");
    assert_eq!(
        &entry[10..42],
        &patch().id(),
        "32 raw id bytes, no length prefix"
    );
    // The register opens with the selected source's length-prefixed author.
    assert_eq!(&entry[42..46], &32u32.to_be_bytes());
    let seed = p.checkpoint([4; 32]).unwrap();
    let recovery = p
        .encode_snapshot(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES)
        .unwrap();
    // Intentional format changes must update these explicitly (docs/design-score-layers.md 5).
    assert_eq!(
        (
            seed.bytes().len(),
            hex(&seed.change_hash()),
            recovery.len(),
            hex(blake3::hash(&recovery).as_bytes())
        ),
        (
            1265,
            "4fe0a504d9006168700d0b7e531de7c06ad761e3df2c33166741c6019acb6251".to_string(),
            1204,
            "281f488bda9fcbe6cea46e7701fcc7af5cbca6f66875717cc8d0b97d2f333722".to_string()
        )
    );
}

#[test]
fn compaction_promotes_shown_collapses_identical_values_and_drops_clears() {
    let p = layered_projection();
    assert!(p.checkpoint_omits_evidence().unwrap());
    let (next, installed) = seeded(&p);
    // Cleared slot dropped whole; nothing of it plays.
    assert!(!installed.layers.contains_key(&(9, 9, ScoreLayer::Drums)));
    // The clear-selected slot promotes its live value and keeps no clear.
    let promoted = &installed.layers[&(1, 1, sound())];
    assert_eq!(
        promoted.register().selected.value,
        Some(sound_note(Some(2), None))
    );
    assert!(promoted.register().conflicts.is_empty());
    // A concurrent pair keeps both values, shown first, the other after it.
    let pair = &p.layers[&(6, 1, sound())];
    let kept = &installed.layers[&(6, 1, sound())];
    assert_eq!(kept.register().selected.value.as_ref(), pair.shown());
    assert_eq!(kept.live().len(), 2);
    // What plays is unchanged by the checkpoint, and the seed is its own fixed point.
    assert_eq!(installed.voices().unwrap(), p.voices().unwrap());
    assert!(!installed.checkpoint_omits_evidence().unwrap());
    // Seeded layer op ids are inherited intents: replaying one is not a new edit.
    let (_, reused, d) = raw(
        &next,
        1,
        &set_note(2, 3, sound_note(Some(4), Some(60))),
        2,
        1,
    );
    assert!(matches!(
        validate_score_change(&logical(), CHANNEL, 1, &d, &reused, &next),
        Err(ReplError::IntentConflict)
    ));
    // And the slot stays editable after the seed.
    let (after, change, d) = raw(&next, 1, &clear_note(2, 3, sound()), 12, 3);
    validate_score_change(&logical(), CHANNEL, 1, &d, &change, &next).unwrap();
    assert!(ScoreProjection::read(&logical(), CHANNEL, 1, &after)
        .unwrap()
        .layers[&(2, 3, sound())]
        .cleared());

    // Identical values collapse to the copy with the lowest op id, even when another copy is
    // the selected one.
    let mut q = read(&edit(&empty(), &set_patch(), 1, 1));
    let value = Some(sound_note(None, None));
    let slot = layer_slot(
        0,
        0,
        sound(),
        &[(value.clone(), 30), (value.clone(), 31), (None, 32)],
    );
    let lowest = slot
        .alternatives()
        .filter(|v| v.value.is_some())
        .map(|v| v.source.op_id)
        .min()
        .unwrap();
    q.layers.insert((0, 0, sound()), slot);
    let (_, compacted) = seeded(&q);
    let slot = &compacted.layers[&(0, 0, sound())];
    assert_eq!(slot.register().selected.source.op_id, lowest);
    assert!(slot.register().conflicts.is_empty());
    // Two raw lengths that clip to one voice keep ONE copy (the shown one): what plays is the
    // same note, and the seed stays bounded by one value per voice.
    let mut q = read(&edit(&empty(), &set_patch(), 1, 1));
    steps_header(&mut q, 12, 40);
    let shown = sound_note(Some(8), None);
    q.layers.insert(
        (8, 0, sound()),
        layer_slot(
            8,
            0,
            sound(),
            &[
                (Some(shown.clone()), 41),
                (Some(sound_note(Some(6), None)), 42),
            ],
        ),
    );
    assert!(q.readiness().unwrap().layers[&(8, 0, sound())].conflict);
    let (_, compacted) = seeded(&q);
    assert_eq!(compacted.layers[&(8, 0, sound())].live(), vec![&shown]);
    assert_eq!(compacted.voices().unwrap(), q.voices().unwrap());
}

#[test]
fn layer_alternatives_are_outside_the_shared_conflict_budget() {
    let mut p = read(&edit(&empty(), &set_patch(), 1, 1));
    steps_header(&mut p, 256, 2);
    // Exhaust the shared budget with legacy fields that each hold two values.
    let fields = crate::studio::snapshot::CONFLICT_FIELDS + 1;
    for n in 0..fields {
        let (step, row) = ((n / 25) as u16, (n % 25) as u8);
        let cell = |pad| ScoreCell::Drum { pad };
        let op = |pad| ScoreOp::SetCell {
            step,
            row,
            value: cell(pad),
        };
        let mut conflicts = vec![direct_value(&op(1), Some(cell(1)), 100_000 + n as u128)];
        conflicts.sort_by_key(|v| v.source.op_id);
        p.cells.insert(
            (step, row),
            ScoreSlot {
                register: ScoreRegister {
                    selected: direct_value(&op(0), Some(cell(0)), 200_000 + n as u128),
                    conflicts,
                },
            },
        );
    }
    // A layer slot with more distinct values than any budgeted field may keep (CONFLICT_VALUES).
    let values: Vec<_> = (0..6u8)
        .map(|v| (Some(sound_note(None, Some(v + 1))), 300_000 + u128::from(v)))
        .collect();
    p.layers
        .insert((200, 0, sound()), layer_slot(200, 0, sound(), &values));
    let (_, compacted) = seeded(&p);
    assert_eq!(compacted.layers[&(200, 0, sound())].live().len(), 6);
    // The legacy field past the budget lost its alternative (existing behaviour, unchanged).
    let last = ((fields - 1) / 25) as u16;
    assert!(compacted.cells[&(last, ((fields - 1) % 25) as u8)]
        .register
        .conflicts
        .is_empty());
    assert_eq!(compacted.voices().unwrap(), p.voices().unwrap());
}

#[test]
fn the_cap_retains_the_first_4096_voices_across_both_collections() {
    let mut p = read(&edit(&empty(), &set_patch(), 1, 1));
    steps_header(&mut p, 256, 2);
    let drum = |pad| ScoreCell::Drum { pad };
    let legacy = |step: u16, row: u8, pad: u8, nonce: u128| ScoreSlot {
        register: ScoreRegister {
            selected: direct_value(
                &ScoreOp::SetCell {
                    step,
                    row,
                    value: drum(pad),
                },
                Some(drum(pad)),
                nonce,
            ),
            conflicts: vec![],
        },
    };
    // Legacy drums at (0,0) and at n = 1..=4096 in (step, row) order: 4097 legacy voices.
    p.cells.insert((0, 0), legacy(0, 0, 0, 10));
    for n in 1..=SCORE_MAX_CELLS {
        let (step, row) = ((n / 25) as u16, (n % 25) as u8);
        p.cells
            .insert((step, row), legacy(step, row, 0, 10 + n as u128));
    }
    // A layered drum at (0,0) with another pad is one MORE voice, early in canonical order; a
    // layered drum identical to the legacy one at (1,0) is the SAME voice, backed by both keys.
    p.layers.insert(
        (0, 0, ScoreLayer::Drums),
        layer_slot(0, 0, ScoreLayer::Drums, &[(Some(drum(1)), 9000)]),
    );
    p.layers.insert(
        (1, 0, ScoreLayer::Drums),
        layer_slot(1, 0, ScoreLayer::Drums, &[(Some(drum(0)), 9001)]),
    );
    let before = p.voices().unwrap();
    assert_eq!(before.play.len(), SCORE_MAX_CELLS);
    assert_eq!(before.skipped, 2);
    assert!(p.checkpoint_omits_evidence().unwrap());
    let (_, compacted) = seeded(&p);
    // Voice order, not collection order: the two last legacy cells go (old `cells.take(4096)`
    // would have kept every legacy cell and nothing else would have been counted).
    assert_eq!(compacted.cells.len(), SCORE_MAX_CELLS - 1);
    for n in [SCORE_MAX_CELLS - 1, SCORE_MAX_CELLS] {
        assert!(!compacted
            .cells
            .contains_key(&((n / 25) as u16, (n % 25) as u8)));
    }
    assert!(compacted.layers.contains_key(&(0, 0, ScoreLayer::Drums)));
    assert!(compacted.layers.contains_key(&(1, 0, ScoreLayer::Drums)));
    assert!(compacted.cells.contains_key(&(1, 0)));
    // Exactly what played before plays after, and nothing is over the cap any more.
    let after = compacted.voices().unwrap();
    assert_eq!(after.play, before.play);
    assert_eq!(after.skipped, 0);
}

#[test]
fn local_admission_counts_legacy_cells_and_live_layer_slots_together() {
    let mut p = read(&edit(&empty(), &set_patch(), 1, 1));
    steps_header(&mut p, 256, 2);
    let pad = ScoreCell::Drum { pad: 0 };
    // 4095 legacy cells plus one live layer slot fill the combined cap; a cleared layer slot
    // does not count.
    for n in 0..SCORE_MAX_CELLS - 1 {
        let (step, row) = ((n / 25) as u16, (n % 25) as u8);
        p.cells.insert(
            (step, row),
            ScoreSlot {
                register: ScoreRegister {
                    selected: direct_value(
                        &ScoreOp::SetCell {
                            step,
                            row,
                            value: pad.clone(),
                        },
                        Some(pad.clone()),
                        10 + n as u128,
                    ),
                    conflicts: vec![],
                },
            },
        );
    }
    p.layers.insert(
        (255, 0, ScoreLayer::Drums),
        layer_slot(255, 0, ScoreLayer::Drums, &[(Some(pad.clone()), 9000)]),
    );
    p.layers.insert(
        (255, 1, ScoreLayer::Drums),
        layer_slot(255, 1, ScoreLayer::Drums, &[(None, 9001)]),
    );
    let target = StudioTarget::Score {
        channel: CHANNEL,
        object: [9; 16],
    };
    let policy = |p: &ScoreProjection, op: &ScoreOp| {
        target.local_policy(
            &StudioProjection::Score(Box::new(p.clone())),
            &domain(op, 99_999),
            &author(1),
        )
    };
    let full = |r: Result<(), ReplError>| matches!(r, Err(ReplError::EpochBound));
    assert!(full(policy(&p, &set_note(200, 0, pad.clone()))));
    assert!(
        full(policy(&p, &drum(200))),
        "legacy growth counts layers too"
    );
    assert!(
        full(policy(&p, &set_note(255, 1, pad.clone()))),
        "re-filling a cleared slot is growth"
    );
    // Replacing a live note in place and every clear stay possible at the cap.
    policy(&p, &set_note(255, 0, ScoreCell::Drum { pad: 5 })).unwrap();
    policy(&p, &clear_note(255, 0, ScoreLayer::Drums)).unwrap();
    policy(&p, &clear_note(0, 0, sound())).unwrap();
    policy(&p, &ScoreOp::ClearCell { step: 0, row: 0 }).unwrap();
    // One slot below the cap, a new layered note is admitted.
    p.layers.remove(&(255, 0, ScoreLayer::Drums));
    policy(&p, &set_note(200, 0, pad)).unwrap();
}

#[test]
fn a_legacy_cell_and_its_layered_copy_are_one_voice_in_a_real_projection() {
    let base = edit(&empty(), &set_patch(), 1, 1);
    let base = edit(&base, &note(3, 2, Some(1)), 2, 1);
    let both = edit(&base, &set_note(3, 2, sound_note(None, None)), 3, 1);
    let p = read(&both);
    let voices = p.voices().unwrap();
    assert_eq!(
        voices.play.len(),
        1,
        "an absent length and a length of 1 are one note"
    );
    assert_eq!(voices.play[0].value, sound_note(None, None));
    assert!(p.readiness().unwrap().ready());
}
