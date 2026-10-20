// The composer's layer surface (score-editor.ts): one shown note per cell over a score that holds
// one note per sound per cell, and the translation of grid gestures into layered document ops
// (docs/design-score-layers.md section 8).
import assert from "node:assert/strict";
import test from "node:test";
import type { JamPatch } from "./jam-contract.ts";
import type { ScoreModel } from "./studio-native.ts";
import type { ScoreCell, ScoreOp } from "./studio-contract.ts";
import {
  SCORE_MISSING_LAYER, scoreAttention, scoreDiagnosticOperation, scoreDiagnostics, scoreGhostCells, scoreInverse, scoreApplyLocal, scoreLayerOf, scoreMoveOperations, scoreMoveSelection,
  scoreNoteCheck, scoreReplay, scoreSurface, scoreSurfaceOps, scoreVisibilityOf, type ScoreVisibility,
} from "./score-editor.ts";

const A = "a".repeat(64), B = "b".repeat(64), GONE = "c".repeat(64);
const PATCH: JamPatch = { v: 1, o: [{ w: 0, t: 0, c: 0, l: 80 }], e: { a: 10, d: 40, s: 70, r: 120 }, f: { m: 0, c: 18000, q: 0, e: 0 }, l: { r: 1, d: 0, t: 0 }, x: { c: 0, d: 0, r: 0 } };
const note = (p: string, len = 1): ScoreCell => ({ k: "n", p, ...(len > 1 ? { len } : {}) });

/** A score with legacy `cells` and every note (legacy and layered) in `notes`, as studio-native builds it. */
function score(notes: Record<string, ScoreCell>, steps = 16): ScoreModel {
  const cells = Object.fromEntries(Object.entries(notes).filter(([key]) => !key.includes("/")));
  return { title: "", bpm: 120, bpb: 4, steps, cells, notes: { ...notes }, layerSlots: {}, voices: { all: [], play: [], skipped: 0, issues: {} },
    patches: { [A]: PATCH, [B]: PATCH }, cellSlots: {}, patchSlots: {}, headers: { title: null, bpm: null, bpb: null, steps: null },
    readiness: { cells: {}, patches: {}, headerConflicts: false }, ready: true };
}
const shown = (visibility: Record<string, ScoreVisibility> = {}) => visibility;

test("the surface shows one note per cell: the chosen sound's first, else by note key", () => {
  const m = score({ "0:0": note(A), [`0:0/${B}`]: note(B, 2), "4:1/d": { k: "d", pad: 3 } });
  const forA = scoreSurface(m, shown(), A);
  assert.deepEqual(forA.model.cells["0:0"], note(A));
  assert.equal(forA.keyOf.get("0:0"), "0:0", "a legacy note lives under its own key");
  assert.ok(forA.stacked.has("0:0"), "the cell holds a second sound behind the shown one");
  const forB = scoreSurface(m, shown(), B);
  assert.deepEqual(forB.model.cells["0:0"], note(B, 2));
  assert.equal(forB.keyOf.get("0:0"), `0:0/${B}`);
  assert.equal(forB.keyOf.get("4:1"), "4:1/d");
  assert.deepEqual(scoreSurface(m, shown(), B), scoreSurface({ ...m, notes: Object.fromEntries(Object.entries(m.notes).reverse()) }, shown(), B), "map order never decides");
});

// A legacy cell and a layered note of one sound in one cell. Only an exact duplicate of the shown
// note (one voice) goes with it; a distinct legacy note plays on its own, was not picked, and stays
// (review of e7250d48, HIGH 1: this test used to call a different-length legacy note stale and
// expect it deleted).
test("a legacy duplicate of the shown note goes with it; a distinct legacy note of that sound is never touched", () => {
  const duplicate = score({ "1:1": { k: "n", p: A, len: 1 }, [`1:1/${A}`]: note(A) }); // `len: 1` and no `len` are one note
  const view = scoreSurface(duplicate, shown(), A);
  assert.equal(view.keyOf.get("1:1"), `1:1/${A}`, "the layered copy is the one shown");
  assert.deepEqual(scoreSurfaceOps(duplicate, view, [{ op: "set_cell", step: 1, row: 1, value: note(A, 3) }]),
    [{ op: "set_note", step: 1, row: 1, value: note(A, 3) }, { op: "clear_cell", step: 1, row: 1 }], "an edit merges the duplicate, written first");
  assert.deepEqual(scoreSurfaceOps(duplicate, view, [{ op: "clear_cell", step: 1, row: 1 }]),
    [{ op: "clear_note", step: 1, row: 1, layer: A }, { op: "clear_cell", step: 1, row: 1 }], "a clear takes the duplicate too");
  assert.deepEqual(scoreSurfaceOps(duplicate, view, scoreMoveOperations(view.model, "1:1", { step: 6, row: 1 })),
    [{ op: "set_note", step: 6, row: 1, value: { k: "n", p: A, len: 1 } }, { op: "clear_note", step: 1, row: 1, layer: A }, { op: "clear_cell", step: 1, row: 1 }]);

  // The reviewer's case: a legacy note (vol 35) and a different layered note (len 3, vol 80).
  const quiet: ScoreCell = { k: "n", p: A, vol: 35 }, loud: ScoreCell = { k: "n", p: A, len: 3, vol: 80 };
  const distinct = score({ "0:0": quiet, [`0:0/${A}`]: loud }), at = scoreSurface(distinct, shown(), A);
  const moved = scoreSurfaceOps(distinct, at, scoreMoveOperations(at.model, "0:0", { step: 4, row: 0 }));
  assert.deepEqual(moved, [{ op: "set_note", step: 4, row: 0, value: loud }, { op: "clear_note", step: 0, row: 0, layer: A }], "only the picked note moves");
  assert.deepEqual(moved.reduce(scoreApplyLocal, distinct).notes["0:0"], quiet, "the vol 35 note still plays where it was");
  assert.deepEqual(scoreSurfaceOps(distinct, at, [{ op: "clear_cell", step: 0, row: 0 }]), [{ op: "clear_note", step: 0, row: 0, layer: A }]);
  assert.throws(() => scoreSurfaceOps(distinct, at, [{ op: "set_cell", step: 0, row: 0, value: { ...loud, vol: 60 } }]), /overlaps/,
    "an in-place edit that would overlap the distinct note is refused, never resolved by deleting it");

  // Another sound's legacy note in that cell is a different note and stays.
  const mixed = score({ "1:1": note(B, 2), [`1:1/${A}`]: note(A, 4) });
  assert.deepEqual(scoreSurfaceOps(mixed, scoreSurface(mixed, shown(), A), [{ op: "set_cell", step: 1, row: 1, value: note(A, 3) }]), [{ op: "set_note", step: 1, row: 1, value: note(A, 3) }]);
});

test("ghosted layers are outlines and never on the surface; hidden ones are nowhere; a gone sound is always shown", () => {
  const m = score({ [`0:0/${A}`]: note(A, 3), [`0:0/${B}`]: note(B), [`2:2/${GONE}`]: note(GONE), "5:0/d": { k: "d", pad: 1 } });
  const view = scoreSurface(m, shown({ [A]: "ghost", [B]: "hide", d: "ghost", [GONE]: "hide" }), A);
  assert.deepEqual(Object.keys(view.model.cells), ["2:2"], "only the note whose sound is gone is editable");
  assert.equal(scoreLayerOf(m, note(GONE)), SCORE_MISSING_LAYER);
  assert.equal(scoreVisibilityOf({ [SCORE_MISSING_LAYER]: "hide" }, SCORE_MISSING_LAYER), "show", "a gone sound cannot be hidden from review");
  assert.deepEqual(view.ghosts.map((g) => g.key), [`0:0/${A}`, "5:0/d"]);
  const outline = scoreGhostCells(view.ghosts, [0, 1, 2, 3, 4, 5]);
  assert.deepEqual(outline.get("0:0"), { p: A, onset: true, last: false });
  assert.deepEqual(outline.get("2:0"), { p: A, onset: false, last: true });
  assert.deepEqual(outline.get("5:0"), { p: null, onset: true, last: true }, "a ghosted hit has no sound colour");
  assert.equal(outline.has("3:0"), false);
  assert.deepEqual([...scoreGhostCells(view.ghosts, [1, 2]).keys()], ["1:0", "2:0"], "a hold that starts off the page still draws its tail");
});

test("a ghosted sound can be written over: the cell is empty on the surface and the write is a new layer", () => {
  const m = score({ [`0:3/${A}`]: note(A, 4) });
  const view = scoreSurface(m, shown({ [A]: "ghost" }), B);
  assert.equal(view.model.cells["0:3"], undefined);
  const ops = scoreSurfaceOps(m, view, [{ op: "set_cell", step: 0, row: 3, value: note(B, 2) }]);
  assert.deepEqual(ops, [{ op: "set_note", step: 0, row: 3, value: note(B, 2) }], "A's note is untouched");
});

test("surface writes become layered ops: placement, a legacy note moving into its layer, a sound change", () => {
  const m = score({ "1:1": note(A, 2), [`3:3/${A}`]: note(A) });
  const view = scoreSurface(m, shown(), A);
  assert.deepEqual(scoreSurfaceOps(m, view, [{ op: "set_cell", step: 8, row: 0, value: note(A) }]), [{ op: "set_note", step: 8, row: 0, value: note(A) }]);
  assert.deepEqual(scoreSurfaceOps(m, view, [{ op: "set_cell", step: 1, row: 1, value: note(A, 3) }]),
    [{ op: "set_note", step: 1, row: 1, value: note(A, 3) }, { op: "clear_cell", step: 1, row: 1 }], "the layered note lands before the legacy one is cleared");
  assert.deepEqual(scoreSurfaceOps(m, view, [{ op: "set_cell", step: 3, row: 3, value: note(A, 2) }]), [{ op: "set_note", step: 3, row: 3, value: note(A, 2) }], "same layer: replaced in place");
  assert.deepEqual(scoreSurfaceOps(m, view, [{ op: "set_cell", step: 3, row: 3, value: note(B) }]),
    [{ op: "set_note", step: 3, row: 3, value: note(B) }, { op: "clear_note", step: 3, row: 3, layer: A }], "a new sound is a new layer");
  assert.deepEqual(scoreSurfaceOps(m, view, [{ op: "clear_cell", step: 3, row: 3 }]), [{ op: "clear_note", step: 3, row: 3, layer: A }]);
  assert.deepEqual(scoreSurfaceOps(m, view, [{ op: "clear_cell", step: 9, row: 9 }]), [], "clearing an empty cell writes nothing");
});

test("a move writes before it clears, and a clear of a key the same gesture writes is dropped", () => {
  const m = score({ [`2:0/${A}`]: note(A, 2), [`4:0/${A}`]: note(A, 2) });
  const view = scoreSurface(m, shown(), A);
  // Sliding both notes one step later: 2 -> 3 and 4 -> 5. No key is both cleared and written here.
  const slide = scoreSurfaceOps(m, view, [{ op: "clear_cell", step: 2, row: 0 }, { op: "clear_cell", step: 4, row: 0 }, { op: "set_cell", step: 3, row: 0, value: note(A, 2) }, { op: "set_cell", step: 5, row: 0, value: note(A, 2) }]);
  assert.deepEqual(slide.map((op) => op.op), ["set_note", "set_note", "clear_note", "clear_note"]);
  // Sliding by two: the first note lands where the second was; that key is written, never cleared.
  const two = scoreSurfaceOps(m, view, [{ op: "clear_cell", step: 2, row: 0 }, { op: "clear_cell", step: 4, row: 0 }, { op: "set_cell", step: 4, row: 0, value: note(A, 2) }, { op: "set_cell", step: 6, row: 0, value: note(A, 2) }]);
  assert.deepEqual(two, [{ op: "set_note", step: 4, row: 0, value: note(A, 2) }, { op: "set_note", step: 6, row: 0, value: note(A, 2) }, { op: "clear_note", step: 2, row: 0, layer: A }]);
  // The surface helpers build moves clear-first; translated they are write-first.
  assert.deepEqual(scoreSurfaceOps(m, view, scoreMoveOperations(view.model, "2:0", { step: 8, row: 0 })),
    [{ op: "set_note", step: 8, row: 0, value: note(A, 2) }, { op: "clear_note", step: 2, row: 0, layer: A }]);
});

// Review M2: with distinct values, a slide's writes used to overwrite a note before writing that
// note's own value elsewhere, so a refusal of a later write lost a value. Whatever prefix of the
// writes lands, every original value must still exist somewhere.
test("a slide writes each note's new place before overwriting its old one: no refusal part way loses a value", () => {
  const values = [1, 2, 3, 4, 5].map((v): ScoreCell => ({ k: "n", p: A, vol: v * 10 }));
  const notes: Record<string, ScoreCell> = {};
  values.forEach((value, i) => { notes[`${i}:0/${A}`] = value; });
  const m = score(notes), view = scoreSurface(m, shown(), A);
  for (const by of [1, 2]) {
    const keys = values.map((_, i) => `${i}:0`);
    const ops = scoreSurfaceOps(m, view, scoreMoveSelection(view.model, keys, by, 0));
    const writes = ops.filter((op) => op.op === "set_note");
    assert.equal(writes.length, 5);
    // Each note's level is its own, so the levels present say which notes still exist.
    const levels = (model: ScoreModel) => new Set(Object.values(model.notes).map((cell) => cell.k === "n" ? cell.vol : -1));
    for (let landed = 0; landed <= writes.length; landed++) {
      const kept = levels(writes.slice(0, landed).reduce(scoreApplyLocal, m));
      for (const value of values) assert.ok(value.k === "n" && kept.has(value.vol), `slide ${by}: ${landed} writes landed, vol ${value.k === "n" ? value.vol : ""} kept`);
    }
    const final = ops.reduce(scoreApplyLocal, m).notes;
    assert.deepEqual(Object.keys(final).sort(), values.map((_, i) => `${i + by}:0/${A}`).sort(), "the whole gesture is the slide");
    assert.deepEqual(values.map((_, i) => (final[`${i + by}:0/${A}`] as { vol?: number }).vol), [10, 20, 30, 40, 50]);
  }
});

// Re-review M1: equal values (an accent pattern a b a) made orderWrites see a false cycle and fall
// back to an order that overwrote the b before writing it elsewhere. Exhaustive over small
// patterns, in time and in pitch: every prefix of the writes keeps every note, counted as a
// multiset so two equal notes stay two.
test("every prefix of a phrase move keeps every note, for every small pattern of repeated values", () => {
  const levels = [undefined, 50, 70]; // full level, and two accents
  const cell = (vol: number | undefined): ScoreCell => ({ k: "n", p: A, ...(vol === undefined ? {} : { vol }) });
  const tally = (cells: Iterable<ScoreCell>) => {
    const out = new Map<number, number>();
    for (const c of cells) if (c.k === "n") out.set(c.vol ?? 100, (out.get(c.vol ?? 100) ?? 0) + 1);
    return out;
  };
  let checked = 0;
  for (let pattern = 0; pattern < 4 ** 5; pattern++) {
    for (const axis of ["time", "pitch"] as const) {
      const notes: Record<string, ScoreCell> = {};
      for (let i = 0, rest = pattern; i < 5; i++, rest = Math.floor(rest / 4)) {
        const slot = rest % 4;
        if (slot === 0) continue;
        const [step, row] = axis === "time" ? [4 + i, 3] : [4, 4 + i];
        notes[`${step}:${row}/${A}`] = cell(levels[slot - 1]);
      }
      if (!Object.keys(notes).length) continue;
      const m = score(notes, 32), view = scoreSurface(m, shown(), A), keys = [...view.keyOf.keys()];
      const before = tally(Object.values(notes));
      for (const by of [-3, -2, -1, 1, 2, 3]) {
        let ops: ScoreOp[];
        try { ops = scoreSurfaceOps(m, view, scoreMoveSelection(view.model, keys, axis === "time" ? by : 0, axis === "pitch" ? by : 0)); } catch { continue; }
        const writes = ops.filter((op) => op.op === "set_note");
        for (let landed = 0; landed <= writes.length; landed++) {
          const after = tally(Object.values(writes.slice(0, landed).reduce(scoreApplyLocal, m).notes));
          for (const [vol, count] of before) assert.ok((after.get(vol) ?? 0) >= count, `${axis} pattern ${pattern} by ${by}: after ${landed} writes, vol ${vol} kept ${after.get(vol) ?? 0} of ${count}`);
        }
        assert.deepEqual(tally(Object.values(ops.reduce(scoreApplyLocal, m).notes)), before, "the whole move keeps every note exactly once");
        checked++;
      }
    }
  }
  assert.ok(checked > 10_000, `${checked} moves checked`);
});

// Third review M-A: the surface shows one note per cell, so a destination looked free while a note
// of the moving sound sat stacked behind another sound there, and the move replaced that note.
test("a move never replaces a note it did not pick, even one stacked behind another sound", () => {
  const bass = (vol?: number): ScoreCell => ({ k: "n", p: B, ...(vol === undefined ? {} : { vol }) });
  const m = score({ [`0:5/${B}`]: bass(), [`1:5/${B}`]: bass(30), [`1:5/${A}`]: note(A) });
  const view = scoreSurface(m, shown(), A);
  assert.ok(view.stacked.has("1:5"), "the vol 30 bass is behind the lead");
  assert.throws(() => scoreSurfaceOps(m, view, scoreMoveSelection(view.model, ["0:5", "1:5"], 1, 0)), /in the way/);
  // Changing a shown note's sound onto a cell where that sound already has a note is refused too.
  assert.throws(() => scoreSurfaceOps(m, view, [{ op: "set_cell", step: 1, row: 5, value: bass() }]), /in the way/);

  // Property: random two-sound scores with drums, legacy cells and layered notes mixed (some legacy
  // cells exact duplicates of a layered note, some distinct), random picks and moves. Whatever is
  // sent never changes a note that was not picked; only an exact duplicate of a picked note (one
  // voice with it) may go with it. Every prefix of the writes keeps every note, too.
  const same = (a: ScoreCell, b: ScoreCell) => a.k === "d" ? b.k === "d" && a.pad === b.pad
    : b.k === "n" && a.p === b.p && (a.len ?? 1) === (b.len ?? 1) && (a.vol ?? 100) === (b.vol ?? 100);
  let seed = 11;
  const next = (n: number) => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed % n; };
  let moves = 0;
  for (let round = 0; round < 4000; round++) {
    const notes: Record<string, ScoreCell> = {};
    for (let i = 0; i < 7; i++) {
      const at = `${next(5)}:${next(3)}`, kind = next(4);
      const value: ScoreCell = kind === 3 ? { k: "d", pad: next(3) } : { k: "n", p: kind === 0 ? B : A, ...(next(2) ? { len: 1 + next(2) } : {}), ...(next(2) ? { vol: 10 + next(3) * 10 } : {}) };
      const layer = value.k === "d" ? "d" : value.p;
      if (next(3) === 0) notes[at] = value; else notes[`${at}/${layer}`] = value; // legacy or layered
      const companion = next(4);
      if (companion === 0 && !notes[at]) notes[at] = { ...value }; // an exact legacy duplicate of it
      if (companion === 1 && !notes[at]) notes[at] = value.k === "d" ? { k: "d", pad: (value.pad + 1) % 3 } : { ...value, vol: 95 }; // a distinct one
    }
    const doc = score(notes, 16), chosen = [A, B, "d"][next(3)];
    const surface = scoreSurface(doc, shown(), chosen), cells = [...surface.keyOf.keys()];
    const pickedCells = cells.filter(() => next(2) === 0);
    if (!pickedCells.length) continue;
    const picked = new Set(pickedCells.map((cell) => surface.keyOf.get(cell)!));
    let ops: ScoreOp[];
    try { ops = scoreSurfaceOps(doc, surface, scoreMoveSelection(surface.model, pickedCells, next(5) - 2, next(3) - 1)); } catch { continue; }
    moves++;
    const after = ops.reduce(scoreApplyLocal, doc).notes;
    for (const [key, value] of Object.entries(notes)) {
      if (picked.has(key)) continue;
      const duplicateOfPicked = !key.includes("/") && [...picked].some((p) => p.startsWith(`${key}/`) && same(notes[p], value));
      if (!duplicateOfPicked) assert.deepEqual(after[key], value, `round ${round}: ${key} was not picked`);
    }
    // Every original voice survives every prefix of the writes (a multiset of note identities).
    const tally = (cells: Iterable<ScoreCell>) => { const t = new Map<string, number>(); for (const c of cells) { const k = JSON.stringify(c.k === "d" ? c : { p: c.p, len: c.len ?? 1, vol: c.vol ?? 100 }); t.set(k, (t.get(k) ?? 0) + 1); } return t; };
    const voicesOf = (all: Record<string, ScoreCell>) => {
      // A legacy cell and a layered note holding the same note are one voice.
      const out: ScoreCell[] = [];
      for (const [key, value] of Object.entries(all)) if (key.includes("/") || !Object.entries(all).some(([k, v]) => k.startsWith(`${key}/`) && same(v, value))) out.push(value);
      return tally(out);
    };
    const before = voicesOf(notes), writes = ops.filter((op) => op.op === "set_note");
    for (let landed = 0; landed <= writes.length; landed++) {
      const got = voicesOf(writes.slice(0, landed).reduce(scoreApplyLocal, doc).notes);
      for (const [k, count] of before) assert.ok((got.get(k) ?? 0) >= count, `round ${round}: after ${landed} writes ${k} kept ${got.get(k) ?? 0} of ${count}`);
    }
  }
  assert.ok(moves > 500, `${moves} moves checked`);
});

test("a gesture's length change lands before its notes, and a full score refuses before anything is sent", () => {
  const m = score({}, 16), view = scoreSurface(m, shown(), A);
  const grow = scoreSurfaceOps(m, view, [{ op: "set_cell", step: 20, row: 0, value: note(A) }, { op: "set_header", field: "steps", value: 32 }].reverse() as ScoreOp[]);
  assert.deepEqual(grow.map((op) => op.op), ["set_header", "set_note"]);
  const full: Record<string, ScoreCell> = {};
  for (let i = 0; i < 4096; i++) full[`${i % 256}:${Math.floor(i / 256)}/d`] = { k: "d", pad: 0 };
  const crowded = score(full, 256), at = scoreSurface(crowded, shown(), "d");
  assert.throws(() => scoreSurfaceOps(crowded, at, [{ op: "set_cell", step: 0, row: 20, value: { k: "d", pad: 1 } }]), /4096/);
  // Moving one note of a full score needs a free slot for the moment between its write and its clear.
  assert.throws(() => scoreSurfaceOps(crowded, at, [{ op: "clear_cell", step: 0, row: 0 }, { op: "set_cell", step: 0, row: 20, value: { k: "d", pad: 0 } }]), /4096/);
  assert.deepEqual(scoreSurfaceOps(crowded, at, [{ op: "set_cell", step: 0, row: 0, value: { k: "d", pad: 5 } }]), [{ op: "set_note", step: 0, row: 0, value: { k: "d", pad: 5 } }], "an existing note stays editable at capacity");
});

test("a layered note's guard: inside the length, a held sound, and no overlap with the same sound only", () => {
  const m = score({ [`0:0/${A}`]: note(A, 4), [`0:0/${B}`]: note(B, 8) });
  assert.throws(() => scoreNoteCheck(m, 2, 0, note(A)), /same sound/);
  assert.doesNotThrow(() => scoreNoteCheck(m, 4, 0, note(A)), "right after A's hold");
  assert.doesNotThrow(() => scoreNoteCheck(m, 0, 0, note(A, 2)), "its own key is replaced, not overlapped");
  assert.doesNotThrow(() => scoreNoteCheck({ ...m, notes: { [`0:0/${A}`]: note(A, 4) } }, 2, 0, note(B)), "another sound may overlap");
  assert.throws(() => scoreNoteCheck(m, 16, 0, note(A)), /beyond the authored/);
  assert.throws(() => scoreNoteCheck(m, 14, 0, note(A, 3)), /fit the remaining/);
  assert.throws(() => scoreNoteCheck(m, 0, 1, note(GONE)), /sound this score holds/);
  assert.doesNotThrow(() => scoreNoteCheck(m, 0, 1, { k: "d", pad: 9 }));
});

test("undo of an in-place edit of a legacy note restores the legacy cell and clears the layered copy", () => {
  // The collision the design calls out: the layered note and the restore target share no key.
  const m = score({ "1:1": note(A, 2) }), view = scoreSurface(m, shown(), A);
  const forward = scoreSurfaceOps(m, view, [{ op: "set_cell", step: 1, row: 1, value: note(A, 4) }]);
  let working = m;
  const undos: ScoreOp[] = [];
  for (const op of forward) { const back = scoreInverse(working, op); if (back) undos.push(back); working = scoreApplyLocal(working, op); }
  assert.deepEqual(working.notes, { [`1:1/${A}`]: note(A, 4) });
  const undo = scoreReplay(working, undos.reverse());
  assert.deepEqual(undo, [{ op: "set_cell", step: 1, row: 1, value: note(A, 2) }, { op: "clear_note", step: 1, row: 1, layer: A }]);
  assert.deepEqual(undo.reduce(scoreApplyLocal, working).notes, { "1:1": note(A, 2) }, "exactly the score before the edit");
});

test("replay lifts out every note it clears before checking its writes, so a nudge's undo is never refused by itself", () => {
  const m = score({ [`2:0/${A}`]: note(A, 4) }), view = scoreSurface(m, shown(), A);
  const nudge = scoreSurfaceOps(m, view, scoreMoveOperations(view.model, "2:0", { step: 3, row: 0 }));
  let working = m;
  const undos: ScoreOp[] = [];
  for (const op of nudge) { const back = scoreInverse(working, op); if (back) undos.push(back); working = scoreApplyLocal(working, op); }
  const undo = scoreReplay(working, undos.reverse());
  assert.deepEqual(undo.reduce(scoreApplyLocal, working).notes, m.notes);
});

test("layered slots are diagnostics of their own kind: listed for an issue, chosen values re-authored in their layer", () => {
  const source = (n: number) => ({ opId: n.toString(16).padStart(64, "0"), author: "e".repeat(64), nonce: n.toString(16).padStart(32, "0") });
  const m = score({ [`3:2/${A}`]: note(A, 2) });
  m.layerSlots = {
    [`3:2/${A}`]: { cleared: false, register: { selected: { value: note(A, 2), source: source(1) }, conflicts: [{ value: note(A), source: source(2) }] } },
    [`9:0/${A}`]: { cleared: true, register: { selected: { value: null, source: source(3) }, conflicts: [] } },
  };
  m.readiness.layers = { [`3:2/${A}`]: { count: false, extent: false, recipe: false, overlap: true, conflict: true } };
  const items = scoreDiagnostics(m).filter((item) => item.kind === "note");
  assert.deepEqual(items.map((item) => [item.id, item.removed, item.issues]), [[`note/3:2/${A}`, false, ["overlap", "conflict"]], [`note/9:0/${A}`, true, []]]);
  assert.deepEqual(scoreAttention(m).map((item) => item.id), [`note/3:2/${A}`], "a plain cleared layer slot is nothing to act on");
  const chosen = scoreDiagnosticOperation(m, items[0], 1);
  assert.deepEqual(chosen, [{ op: "set_note", step: 3, row: 2, value: note(A) }]);
});
