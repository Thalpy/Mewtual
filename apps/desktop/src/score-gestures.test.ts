// The composer's gesture helpers: a local projection, inverses for undo, moves and bar repeats.
import assert from "node:assert/strict";
import test from "node:test";
import type { JamPatch } from "./jam-contract.ts";
import type { ScoreModel } from "./studio-native.ts";
import { scoreApplyLocal, scoreClearSelection, scoreDuplicateSelection, scoreInverse, scoreMoveOperations, scoreMoveSelection, scoreOnsetsIn, scoreRepeatBar, scoreReplay } from "./score-editor.ts";

const PATCH_ID = "a".repeat(64), OTHER_ID = "b".repeat(64);
const PATCH: JamPatch = { v: 1, o: [{ w: 0, t: 0, c: 0, l: 80 }], e: { a: 10, d: 40, s: 70, r: 120 }, f: { m: 0, c: 18000, q: 0, e: 0 }, l: { r: 1, d: 0, t: 0 }, x: { c: 0, d: 0, r: 0 } };
/** `notes` is `cells` itself (a legacy cell is a note under its own key), as score-editor.test.ts builds it. */
function model(steps = 16): ScoreModel {
  const cells: ScoreModel["cells"] = {};
  return { title: "t", bpm: 120, bpb: 4, steps, cells, notes: cells, layerSlots: {}, voices: { all: [], play: [], skipped: 0, issues: {} }, patches: { [PATCH_ID]: PATCH }, cellSlots: {}, patchSlots: {}, headers: { title: null, bpm: null, bpb: null, steps: null }, readiness: { cells: {}, patches: {}, headerConflicts: false }, ready: true };
}

test("the local projection applies every operation the way the working projection does, without touching its input", () => {
  const m = model();
  const next = scoreApplyLocal(m, { op: "set_cell", step: 1, row: 2, value: { k: "n", p: PATCH_ID, len: 2 } });
  assert.deepEqual(next.cells["1:2"], { k: "n", p: PATCH_ID, len: 2 }); assert.equal(m.cells["1:2"], undefined);
  const cleared = scoreApplyLocal(next, { op: "clear_cell", step: 1, row: 2 }); assert.equal(cleared.cells["1:2"], undefined);
  const withPatch = scoreApplyLocal(m, { op: "set_patch", patch: OTHER_ID, descriptor: PATCH }); assert.equal(withPatch.patches[OTHER_ID], PATCH);
  assert.equal(scoreApplyLocal(withPatch, { op: "remove_patch", patch: OTHER_ID }).patches[OTHER_ID], undefined);
  assert.equal(scoreApplyLocal(m, { op: "set_header", field: "bpm", value: 90 }).bpm, 90);
  assert.equal(scoreApplyLocal(m, { op: "set_header", field: "title", value: "new" }).title, "new");
});

test("inverses put back exactly what an operation replaced, and nothing for a no-op clear or removal", () => {
  const m = model(); m.cells["4:4"] = { k: "n", p: PATCH_ID, len: 3, vol: 60 };
  assert.deepEqual(scoreInverse(m, { op: "set_cell", step: 4, row: 4, value: { k: "d", pad: 1 } }), { op: "set_cell", step: 4, row: 4, value: { k: "n", p: PATCH_ID, len: 3, vol: 60 } });
  assert.deepEqual(scoreInverse(m, { op: "clear_cell", step: 4, row: 4 }), { op: "set_cell", step: 4, row: 4, value: { k: "n", p: PATCH_ID, len: 3, vol: 60 } });
  assert.deepEqual(scoreInverse(m, { op: "set_cell", step: 0, row: 0, value: { k: "d", pad: 1 } }), { op: "clear_cell", step: 0, row: 0 });
  assert.equal(scoreInverse(m, { op: "clear_cell", step: 0, row: 0 }), null);
  assert.deepEqual(scoreInverse(m, { op: "set_header", field: "bpm", value: 90 }), { op: "set_header", field: "bpm", value: 120 });
  assert.deepEqual(scoreInverse(m, { op: "set_header", field: "title", value: "x" }), { op: "set_header", field: "title", value: "t" });
  assert.deepEqual(scoreInverse(m, { op: "remove_patch", patch: PATCH_ID }), { op: "set_patch", patch: PATCH_ID, descriptor: PATCH });
  assert.deepEqual(scoreInverse(m, { op: "set_patch", patch: OTHER_ID, descriptor: PATCH }), { op: "remove_patch", patch: OTHER_ID });
  assert.equal(scoreInverse(m, { op: "remove_patch", patch: OTHER_ID }), null);
  const inverse = scoreInverse(m, { op: "clear_cell", step: 4, row: 4 })!;
  m.cells["4:4"] = { k: "d", pad: 0 };
  assert.deepEqual((inverse as { value: unknown }).value, { k: "n", p: PATCH_ID, len: 3, vol: 60 }, "the inverse keeps its own copy of the previous value");
});

test("a move is a clear and a set that keep the note's sound, length and level, and refuse what is in the way", () => {
  const m = model(); m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 3, vol: 70 }; m.cells["8:5"] = { k: "n", p: PATCH_ID, len: 1 }; m.cells["0:0"] = { k: "d", pad: 4 };
  assert.deepEqual(scoreMoveOperations(m, "2:5", { step: 6, row: 9 }), [
    { op: "clear_cell", step: 2, row: 5 }, { op: "set_cell", step: 6, row: 9, value: { k: "n", p: PATCH_ID, len: 3, vol: 70 } },
  ]);
  assert.deepEqual(scoreMoveOperations(m, "2:5", { step: 3, row: 5 }), [
    { op: "clear_cell", step: 2, row: 5 }, { op: "set_cell", step: 3, row: 5, value: { k: "n", p: PATCH_ID, len: 3, vol: 70 } },
  ], "a note may slide over its own old cells");
  assert.deepEqual(scoreMoveOperations(m, "2:5", { step: 2, row: 5 }), [], "no movement is no gesture");
  assert.throws(() => scoreMoveOperations(m, "2:5", { step: 7, row: 5 }), /in the way/, "it would land on the note at step 8");
  assert.throws(() => scoreMoveOperations(m, "2:5", { step: 14, row: 5 }), /past the end/, "it would run past the authored length, and says so");
  assert.throws(() => scoreMoveOperations(m, "3:3", { step: 1, row: 1 }), /no note/);
  assert.deepEqual(scoreMoveOperations(m, "0:0", { step: 1, row: 20 }), [{ op: "clear_cell", step: 0, row: 0 }, { op: "set_cell", step: 1, row: 20, value: { k: "d", pad: 4 } }]);
});

test("repeating a bar copies every onset one bar later, grows the length first and leaves taken cells alone", () => {
  const m = model(16); m.cells["0:5"] = { k: "n", p: PATCH_ID, len: 4, vol: 50 }; m.cells["4:0"] = { k: "d", pad: 1 }; m.cells["12:7"] = { k: "n", p: PATCH_ID, len: 2 };
  const grown = scoreRepeatBar(m, 0, 16);
  assert.deepEqual(grown.ops, [
    { op: "set_header", field: "steps", value: 32 },
    { op: "set_cell", step: 16, row: 5, value: { k: "n", p: PATCH_ID, len: 4, vol: 50 } },
    { op: "set_cell", step: 20, row: 0, value: { k: "d", pad: 1 } },
    { op: "set_cell", step: 28, row: 7, value: { k: "n", p: PATCH_ID, len: 2 } },
  ], "the length grows first, then the cells in step order");
  assert.deepEqual([grown.copied, grown.skipped], [3, 0]);
  const taken = model(32); taken.cells["0:5"] = { k: "n", p: PATCH_ID, len: 4 }; taken.cells["18:5"] = { k: "n", p: PATCH_ID, len: 1 }; taken.cells["4:0"] = { k: "d", pad: 1 };
  const partial = scoreRepeatBar(taken, 0, 16);
  assert.deepEqual(partial.ops, [{ op: "set_cell", step: 20, row: 0, value: { k: "d", pad: 1 } }], "the note would overlap the one at step 18, so it is skipped and counted");
  assert.deepEqual([partial.copied, partial.skipped], [1, 1]);
  assert.deepEqual(scoreRepeatBar(model(16), 0, 16).ops, [{ op: "set_header", field: "steps", value: 32 }], "an empty bar still grows the score");
  assert.throws(() => scoreRepeatBar(model(256), 15, 16), /256/);
  const orphan = model(32); orphan.cells["0:5"] = { k: "n", p: OTHER_ID, len: 2 }; orphan.cells["2:1"] = { k: "d", pad: 0 };
  const skipped = scoreRepeatBar(orphan, 0, 16);
  assert.deepEqual(skipped.ops, [{ op: "set_cell", step: 18, row: 1, value: { k: "d", pad: 0 } }], "a note whose sound is gone is skipped, not fatal");
  assert.deepEqual([skipped.copied, skipped.skipped], [1, 1]);
});

test("a marquee picks the onsets inside its rectangle, in step then row order, whichever corner came first", () => {
  const m = model(); m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 2 }; m.cells["4:3"] = { k: "d", pad: 1 }; m.cells["9:5"] = { k: "n", p: PATCH_ID, len: 1 }; m.cells["3:8"] = { k: "n", p: PATCH_ID, len: 1 };
  assert.deepEqual(scoreOnsetsIn(m, { step: 5, row: 7 }, { step: 1, row: 2 }), ["2:5", "4:3"]);
  assert.deepEqual(scoreOnsetsIn(m, { step: 0, row: 0 }, { step: 15, row: 24 }), ["2:5", "3:8", "4:3", "9:5"]);
  assert.deepEqual(scoreOnsetsIn(m, { step: 3, row: 5 }, { step: 3, row: 5 }), [], "a continuation cell is not an onset");
});

test("a phrase moves as one gesture, keeps its shape, may slide over its own cells and refuses as a whole", () => {
  const m = model(16); m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 2, vol: 50 }; m.cells["4:7"] = { k: "n", p: PATCH_ID, len: 2 }; m.cells["3:0"] = { k: "d", pad: 3 }; m.cells["9:9"] = { k: "n", p: PATCH_ID, len: 1 };
  const keys = ["2:5", "4:7", "3:0"];
  // Declared change (re-review M1): sets run against the direction of travel, the note furthest
  // ahead first, so each note's new place is written before the note behind it overwrites its old
  // one. Moving later and up by (1, 2): 4:7 is furthest ahead, then 2:5, then 3:0.
  assert.deepEqual(scoreMoveSelection(m, keys, 1, 2), [
    { op: "clear_cell", step: 2, row: 5 }, { op: "clear_cell", step: 3, row: 0 }, { op: "clear_cell", step: 4, row: 7 },
    { op: "set_cell", step: 5, row: 9, value: { k: "n", p: PATCH_ID, len: 2 } },
    { op: "set_cell", step: 3, row: 7, value: { k: "n", p: PATCH_ID, len: 2, vol: 50 } },
    { op: "set_cell", step: 4, row: 2, value: { k: "d", pad: 3 } },
  ], "clears first, then sets against the direction of travel; a note may land where a selected note was");
  assert.deepEqual(scoreMoveSelection(m, keys, 0, 0), []);
  assert.throws(() => scoreMoveSelection(m, keys, 5, 2), /in the way/, "the note at 9:9 blocks one of them, so none move");
  assert.throws(() => scoreMoveSelection(m, keys, 12, 0), /past the end/);
  assert.throws(() => scoreMoveSelection(m, keys, 0, 18), /off the grid/);
  assert.throws(() => scoreMoveSelection(m, ["7:7"], 1, 0), /Nothing is selected/);
  assert.deepEqual(scoreClearSelection(m, ["2:5", "9:9", "7:7"]), [{ op: "clear_cell", step: 2, row: 5 }, { op: "clear_cell", step: 9, row: 9 }]);
});

test("a duplicated phrase starts where the phrase ends, grows the score when it must and skips taken cells", () => {
  const m = model(16); m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 2 }; m.cells["5:5"] = { k: "n", p: PATCH_ID, len: 3 }; m.cells["3:0"] = { k: "d", pad: 1 };
  const copy = scoreDuplicateSelection(m, ["2:5", "5:5", "3:0"]);
  assert.equal(copy.shift, 6, "first onset at 2, last note ends at 8");
  assert.deepEqual(copy.ops, [
    { op: "set_cell", step: 8, row: 5, value: { k: "n", p: PATCH_ID, len: 2 } },
    { op: "set_cell", step: 9, row: 0, value: { k: "d", pad: 1 } },
    { op: "set_cell", step: 11, row: 5, value: { k: "n", p: PATCH_ID, len: 3 } },
  ]);
  assert.deepEqual([copy.copied, copy.skipped, copy.keys], [3, 0, ["8:5", "9:0", "11:5"]]);
  const tight = model(8); tight.cells["0:1"] = { k: "n", p: PATCH_ID, len: 4 }; tight.cells["4:1"] = { k: "n", p: PATCH_ID, len: 2 }; tight.cells["7:1"] = { k: "d", pad: 0 };
  const grown = scoreDuplicateSelection(tight, ["0:1", "4:1"]);
  assert.deepEqual(grown.ops[0], { op: "set_header", field: "steps", value: 12 }, "the copy needs steps 6..12");
  assert.deepEqual([grown.copied, grown.skipped], [1, 1], "the second copy would overlap the hit at step 7, so only the first lands");
  const huge = model(256); huge.cells["0:0"] = { k: "n", p: PATCH_ID, len: 200 };
  assert.throws(() => scoreDuplicateSelection(huge, ["0:0"]), /256/);
});

test("a replayed gesture meets the same guards as a hand edit against the document as it is now", () => {
  const m = model(16); m.cells["4:5"] = { k: "n", p: PATCH_ID, len: 2 };
  const restore = [{ op: "set_cell", step: 0, row: 0, value: { k: "n", p: PATCH_ID, len: 4, vol: 30 } }, { op: "set_header", field: "bpm", value: 90 }] as const;
  assert.deepEqual(scoreReplay(m, restore), restore, "what still fits is sent as recorded");
  const overlapping = { ...m, cells: { ...m.cells, "1:0": { k: "n" as const, p: PATCH_ID, len: 1 } } };
  assert.throws(() => scoreReplay(overlapping, restore), /overlaps/, "a collaborator's note now sits under the restored hold");
  assert.throws(() => scoreReplay({ ...m, steps: 2 }, restore), /authored/, "the score was shortened under it");
  assert.throws(() => scoreReplay({ ...m, patches: {} }, restore), /sound this score holds/, "its sound was removed");
  assert.throws(() => scoreReplay(m, [{ op: "set_header", field: "steps", value: 999 }]), /whole number/);
  assert.deepEqual(scoreReplay(m, [{ op: "clear_cell", step: 9, row: 9 }, { op: "remove_patch", patch: OTHER_ID }]), [{ op: "clear_cell", step: 9, row: 9 }, { op: "remove_patch", patch: OTHER_ID }], "clears and removals always replay");
  // Rolling: the second operation is checked after the first has taken effect.
  assert.deepEqual(scoreReplay(m, [{ op: "clear_cell", step: 4, row: 5 }, { op: "set_cell", step: 3, row: 5, value: { k: "n", p: PATCH_ID, len: 3 } }]).length, 2);
});
