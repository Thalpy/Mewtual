//! The Rust half of the shared voice/readiness contract. Every case in
//! `tests/fixtures/studio-score-readiness-v1.json` is also run by the renderer's
//! `score-voices.test.ts`; a case passing on one side only is a drift that stops playback.
use super::*;
use crate::studio::snapshot::derived;
use catcoms_crypto::DeviceId;
use catcoms_rt::Clock;
use serde_json::{json, Value};

fn document() -> LogicalDocument {
    score_document(b"score-voices", [9; 16]).unwrap()
}

/// A genuine (derivable) source, so projections built from these pass the snapshot codec's
/// provenance checks when a test goes through `readiness()`/`checkpoint()`.
fn source(n: u128) -> ScoreSource {
    let author = DeviceId::from_bytes([5; 32]);
    let nonce = n.to_be_bytes();
    ScoreSource {
        op_id: derived(&document(), &author, nonce),
        author,
        nonce,
    }
}

/// TEST-ONLY register constructor: `values[0]` is the selected alternative and the rest become
/// its conflicts in the codec's required ascending op-id order, exactly how the fixture lists a
/// register ("selected first").
fn register<T>(values: Vec<T>, counter: &mut u128) -> ScoreRegister<T> {
    let mut alternatives: Vec<_> = values
        .into_iter()
        .map(|value| {
            *counter += 1;
            ScoreValue {
                source: source(*counter),
                value,
            }
        })
        .collect();
    let selected = alternatives.remove(0);
    alternatives.sort_by_key(|v| v.source.op_id);
    ScoreRegister {
        selected,
        conflicts: alternatives,
    }
}

type Slots = (
    BTreeMap<ScoreAddress, ScoreSlot<ScoreCell>>,
    BTreeMap<ScoreNoteAddress, ScoreLayerSlot>,
);

/// TEST-ONLY projection slots from fixture rows: a key without `/` is a legacy cell, `S:R/L` a
/// layered note; `null` is a clear.
fn slots(rows: &[Value]) -> Slots {
    let mut counter = 0;
    let (mut cells, mut layers) = (BTreeMap::new(), BTreeMap::new());
    for row in rows {
        let key = row["key"].as_str().unwrap();
        let (address, layer) = match key.split_once('/') {
            Some((address, layer)) => (address, Some(ScoreLayer::parse(layer).unwrap())),
            None => (key, None),
        };
        let (step, row_index) = address.split_once(':').unwrap();
        let (step, row_index): (u16, u8) = (step.parse().unwrap(), row_index.parse().unwrap());
        let values: Vec<Option<ScoreCell>> = row["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| (!v.is_null()).then(|| ScoreCell::read(v).unwrap()))
            .collect();
        let register = register(values, &mut counter);
        match layer {
            None => {
                cells.insert((step, row_index), ScoreSlot { register });
            }
            Some(layer) => {
                layers.insert((step, row_index, layer), ScoreLayerSlot::new(register));
            }
        }
    }
    (cells, layers)
}

fn flags(issues: &ScoreCellIssues) -> Value {
    json!({"count":issues.count,"extent":issues.extent,"recipe":issues.recipe,
        "overlap":issues.overlap,"conflict":issues.conflict})
}

/// The renderer's sparse `issues` record, rebuilt from the Rust maps with the DTO's key spelling.
fn issues(report: &VoiceReport) -> Value {
    let mut out = serde_json::Map::new();
    for ((step, row), issues) in &report.cells {
        out.insert(format!("{step}:{row}"), flags(issues));
    }
    for ((step, row, layer), issues) in &report.layers {
        out.insert(format!("{step}:{row}/{}", layer.key()), flags(issues));
    }
    Value::Object(out)
}

fn play(voices: &ScoreVoices) -> Value {
    Value::Array(
        voices
            .play
            .iter()
            .map(|v| json!({"step":v.step,"row":v.row,"value":v.value.value(v.step).unwrap()}))
            .collect(),
    )
}

fn admitted(ids: impl IntoIterator<Item = ContentId>) -> BTreeSet<ContentId> {
    // As `ScoreProjection::admitted_sounds`: the first 64 by id.
    let ids: BTreeSet<_> = ids.into_iter().collect();
    ids.into_iter().take(SCORE_MAX_PATCHES).collect()
}

#[test]
fn shared_readiness_fixture_cases_match_the_renderer() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/studio-score-readiness-v1.json"
    ))
    .unwrap();
    assert_eq!(fixture["version"], 1);
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() >= 14, "the shared fixture lost cases");
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let (cells, layers) = slots(case["slots"].as_array().unwrap());
        let steps = u16::try_from(case["steps"].as_u64().unwrap()).unwrap();
        let sounds = admitted(
            case["sounds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| fixed_hex::<32>(s.as_str().unwrap()).unwrap()),
        );
        let report = compute(&cells, &layers, steps, &sounds);
        assert_eq!(play(&report.public()), case["play"], "{name}: play");
        assert_eq!(
            report.skipped as u64,
            case["skipped"].as_u64().unwrap(),
            "{name}: skipped"
        );
        let mut expected = serde_json::Map::new();
        for (key, partial) in case["issues"].as_object().unwrap() {
            let mut all = json!({"count":false,"extent":false,"recipe":false,"overlap":false,"conflict":false});
            for (flag, value) in partial.as_object().unwrap() {
                assert!(all.get(flag).is_some(), "{name}: unknown flag {flag}");
                all[flag] = value.clone();
            }
            expected.insert(key.clone(), all);
        }
        assert_eq!(issues(&report), Value::Object(expected), "{name}: issues");
        // Sparse: no slot is listed without a flag (an all-false entry would change the DTO).
        for issue in report.cells.values().chain(report.layers.values()) {
            assert_ne!(*issue, ScoreCellIssues::default(), "{name}: dense entry");
        }
    }
}

#[test]
fn canonical_voice_bytes_equal_the_shared_canonical_encoder() {
    let patch = [0xab; 32];
    let mut cells = vec![];
    for pad in 0..=9 {
        cells.push(ScoreCell::Drum { pad });
    }
    for len in [None, Some(1), Some(2), Some(10), Some(256)] {
        for volume in [None, Some(1), Some(50), Some(100)] {
            cells.push(ScoreCell::Note { patch, len, volume });
        }
    }
    for cell in cells {
        assert_eq!(
            canonical_bytes(&cell),
            canonical(&cell.value(0).unwrap()).unwrap()
        );
    }
    // Order is canonical BYTES, not numbers: a hold of 10 sorts before a hold of 2, and a note
    // with a level sorts before the same note without one, exactly as the renderer compares.
    let note = |len, volume| canonical_bytes(&ScoreCell::Note { patch, len, volume });
    assert!(note(Some(10), None) < note(Some(2), None));
    assert!(note(Some(2), Some(50)) < note(Some(2), None));
    assert!(canonical_bytes(&ScoreCell::Drum { pad: 9 }) < note(None, None));
}

#[test]
fn voices_after_the_4096th_in_canonical_order_are_skipped_and_flagged() {
    let mut counter = 0;
    let mut layers = BTreeMap::new();
    for i in 0..=SCORE_MAX_CELLS {
        let (step, row) = ((i % 256) as u16, (i / 256) as u8);
        layers.insert(
            (step, row, ScoreLayer::Drums),
            ScoreLayerSlot::new(register(
                vec![Some(ScoreCell::Drum { pad: 0 })],
                &mut counter,
            )),
        );
    }
    let report = compute(&BTreeMap::new(), &layers, 256, &BTreeSet::new());
    assert_eq!(report.public().play.len(), SCORE_MAX_CELLS);
    assert_eq!(report.skipped, 1);
    // Canonical order is (step, row): the last voice is step 255 of the last row reached (15).
    assert_eq!(
        issues(&report),
        json!({"255:15/d":{"count":true,"extent":false,"recipe":false,"overlap":false,"conflict":false}})
    );
}

#[test]
fn overlap_sweep_matches_the_pairwise_definition_on_random_lanes() {
    // The same deterministic generator as the renderer's test: the sweep must flag exactly the
    // voices that intersect another voice of their (row, sound), whatever the starts and lengths.
    let mut seed: u64 = 7;
    let mut next = |n: u64| {
        seed = (seed * 1_103_515_245 + 12_345) % 2_147_483_648;
        seed % n
    };
    let patch = [0xef; 32];
    for round in 0..200 {
        let mut counter = 0;
        let mut layers = BTreeMap::new();
        let mut holds = vec![];
        for _ in 0..12 {
            let (step, len) = (next(24) as u16, 1 + next(6) as u16);
            let key = (step, 0, ScoreLayer::Sound(patch));
            if layers.contains_key(&key) {
                continue;
            }
            holds.push((step, len));
            let cell = ScoreCell::Note {
                patch,
                len: (len > 1).then_some(len),
                volume: None,
            };
            layers.insert(
                key,
                ScoreLayerSlot::new(register(vec![Some(cell)], &mut counter)),
            );
        }
        let report = compute(&BTreeMap::new(), &layers, 256, &admitted([patch]));
        for &(step, len) in &holds {
            let pairwise = holds
                .iter()
                .any(|&(other, olen)| other != step && other < step + len && step < other + olen);
            let flagged = report
                .layers
                .get(&(step, 0, ScoreLayer::Sound(patch)))
                .is_some_and(|i| i.overlap);
            assert_eq!(flagged, pairwise, "round {round} step {step}");
        }
    }
}

/// The patch from the shared op vectors, with its real id (a sound id is its descriptor's hash).
fn vector_patch() -> StudioPatch {
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

/// Design review H3: one member can stack thousands of concurrent values on one key, and native
/// computes readiness on every view (src-tauri `projection_content`). The renderer's old pairwise
/// walk took 24 s for this shape; the whole Rust readiness, measurement included, must stay far
/// below a second even in a debug build.
#[test]
fn readiness_over_many_concurrent_values_on_one_row_and_sound_is_not_quadratic() {
    let descriptor = vector_patch();
    let patch = descriptor.id();
    let mut counter = 0;
    let mut p = ScoreProjection::empty(document(), [7; 16], 0);
    p.steps = Some(register(vec![256u16], &mut counter));
    p.patches.insert(
        patch,
        ScoreSlot {
            register: register(vec![Some(descriptor)], &mut counter),
        },
    );
    let mut values = vec![];
    'fill: for len in 1..=256u16 {
        for volume in 1..=100u8 {
            if values.len() == 20_480 {
                break 'fill;
            }
            values.push(Some(ScoreCell::Note {
                patch,
                len: Some(len),
                volume: Some(volume),
            }));
        }
    }
    p.layers.insert(
        (0, 0, ScoreLayer::Sound(patch)),
        ScoreLayerSlot::new(register(values, &mut counter)),
    );
    // Wall time through the sanctioned seam (the ambient gate forbids `Instant` here).
    let clock = catcoms_rt::SystemClock;
    let timed = |f: &mut dyn FnMut()| {
        let started = clock.monotonic_ms();
        f();
        clock.monotonic_ms().saturating_sub(started)
    };
    let mut report = None;
    let voice_ms =
        timed(&mut || report = Some(compute(&p.cells, &p.layers, 256, &admitted([patch]))));
    let mut readiness = None;
    let readiness_ms = timed(&mut || readiness = Some(p.readiness().unwrap()));
    let voices = p.voices().unwrap();
    assert_eq!(voices, report.unwrap().public());
    assert_eq!(voices.play.len(), SCORE_MAX_CELLS);
    assert_eq!(voices.skipped, 20_480 - SCORE_MAX_CELLS);
    let issues = readiness.unwrap().layers[&(0, 0, ScoreLayer::Sound(patch))];
    assert!(issues.overlap && issues.count && issues.conflict);
    assert!(!issues.extent && !issues.recipe);
    // Measured in a debug build on the development host: the voice derivation takes about 70 ms
    // and the whole readiness about 650 ms (almost all of it the pre-existing, linear provenance
    // measurement of 20,480 records). A pairwise overlap walk took 50 s for the derivation alone,
    // so these bounds keep wide headroom for a loaded CI host while still failing a quadratic
    // regression by orders of magnitude.
    assert!(voice_ms < 1_000, "voice derivation took {voice_ms} ms");
    assert!(readiness_ms < 5_000, "readiness took {readiness_ms} ms");
}
