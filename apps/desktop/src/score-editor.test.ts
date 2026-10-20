import assert from "node:assert/strict";
import test from "node:test";
import type { JamPatch } from "./jam-contract.ts";
import type { ScoreModel, ScoreSlot, Valued, IndexSource } from "./studio-native.ts";
import { canonicalJson, parseNoteKey, type ScoreCell } from "./studio-contract.ts";
import type { ScoreSelection } from "./score-selection.ts";
import { admittedSounds, scoreVoices } from "./score-voices.ts";
import { workingScore } from "./studio-working.ts";
import {
  PAD_CODES, PAD_NAMES, SCORE_DEFAULT_HOLD, SCORE_PAGE_STEPS, SCORE_WIDE_PAGE_STEPS, scoreApplyLocal, scoreAttention, scoreBarBeat, scoreCellOperation, scoreContinuations,
  scoreDiagnosticOperation, scoreDiagnostics, scoreDurationMs, scoreExtent, scoreHeaderOperation, scoreHolds, scoreLastSoundingStep,
  scoreNotesAt, scoreOccupiedSteps, scoreOnsetAt, scorePadForRow, scorePage, scorePatchOrder, scorePatchUsage, scorePitch, scorePlayheadStep,
  scoreOpSeveralValues, scoreRowIsBlack, scoreScaleRows, scoreSlotDiagnostic, scoreSnapStart, scoreTapTempo, scoreTimeLabel,
} from "./score-editor.ts";

export const PATCH_ID = "a".repeat(64);
export const PATCH: JamPatch = { v: 1, o: [{ w: 0, t: 0, c: 0, l: 80 }], e: { a: 10, d: 40, s: 70, r: 120 }, f: { m: 0, c: 18000, q: 0, e: 0 }, l: { r: 1, d: 0, t: 0 }, x: { c: 0, d: 0, r: 0 } };
export function alternative<T>(value: T, n = 1): Valued<T, IndexSource> { return { value, source: { opId: n.toString(16).padStart(64, "0"), author: "b".repeat(64), nonce: n.toString(16).padStart(32, "0") } }; }
export function slot<T>(value: T | null, conflicts: (T | null)[] = []): ScoreSlot<T> { return { removed: [value, ...conflicts].includes(null), register: { selected: alternative(value), conflicts: conflicts.map((c, i) => alternative(c, i + 2)) } }; }
/** A legacy-only score, the shape tests have always built. `notes` is the same object as `cells`
 * (every legacy cell is a note under its own key), so a test that sets `m.cells[k]` sets the note
 * the composer's surface reads; `voices` follows from it as studio-native computes it. */
export function model(): ScoreModel {
  const cells: Record<string, ScoreCell> = {};
  return { title: "", bpm: 120, bpb: 4, steps: 16, cells, notes: cells, layerSlots: {}, patches: { [PATCH_ID]: PATCH }, cellSlots: {}, patchSlots: { [PATCH_ID]: slot(PATCH) },
    get voices() { return scoreVoices(Object.entries(this.notes).map(([key, value]) => { const at = parseNoteKey(key)!; return { key, step: at.step, row: at.row, layer: at.layer, values: [value] }; }), this.steps, admittedSounds(Object.keys(this.patches))); },
    set voices(_derived) { /* always derived from notes; the apply harness's copy is ignored */ },
    headers: { title: null, bpm: null, bpb: null, steps: null }, readiness: { cells: {}, patches: {}, headerConflicts: false }, ready: true };
}

test("score pitch and pages cover all 25 chromatic rows and the partial 256-step boundary", () => {
  assert.equal(scorePitch(0), "C4"); assert.equal(scorePitch(1), "C♯4"); assert.equal(scorePitch(12), "C5"); assert.equal(scorePitch(24), "C6");
  assert.throws(() => scorePitch(-1)); assert.throws(() => scorePitch(25)); assert.throws(() => scorePitch(1.5));
  assert.equal(scoreRowIsBlack(1), true); assert.equal(scoreRowIsBlack(0), false); assert.equal(scoreRowIsBlack(24), false);
  assert.deepEqual(scorePage(1, 0), [0]); assert.deepEqual(scorePage(17, 1), [16]);
  assert.deepEqual(scorePage(256, 15), Array.from({ length: 16 }, (_, i) => i + 240));
  assert.deepEqual(scorePage(40, 1, SCORE_WIDE_PAGE_STEPS), Array.from({ length: 8 }, (_, i) => i + 32), "a wide page is 32 steps and still clips at the authored length");
  for (const args of [[0, 0], [257, 0], [16, 1], [17, -1], [16, 0.5]]) assert.throws(() => scorePage(args[0], args[1]));
  assert.throws(() => scorePage(16, 0, 0)); assert.throws(() => scorePage(16, 1, SCORE_WIDE_PAGE_STEPS));
});

test("header operations use authored scalar text and native musical bounds without trimming", () => {
  assert.deepEqual(scoreHeaderOperation("title", " é🎶 "), { op: "set_header", field: "title", value: " é🎶 " });
  assert.throws(() => scoreHeaderOperation("title", "\ud800"), /Unicode/);
  for (const [field, min, max] of [["bpm", 40, 240], ["bpb", 1, 8], ["steps", 1, 256]] as const) {
    for (const n of [min, max]) assert.deepEqual(scoreHeaderOperation(field, String(n)), { op: "set_header", field, value: n });
    for (const bad of ["", " ", "NaN", "Infinity", String(min - 1), String(max + 1), String(min + 0.5)]) assert.throws(() => scoreHeaderOperation(field, bad));
  }
});

test("note and drum placement preserve authored holds and need an active recipe only for notes", () => {
  const m = model();
  // scoreCellOperation builds the composer's surface ops (one note per cell); scoreSurfaceOps turns
  // them into layered document ops (the layer tests below).
  assert.deepEqual(scoreCellOperation(m, 0, 24, { mode: "note", patch: PATCH_ID, len: 16 }), { op: "set_cell", step: 0, row: 24, value: { k: "n", p: PATCH_ID, len: 16 } });
  assert.deepEqual(scoreCellOperation(m, 15, 0, { mode: "note", patch: PATCH_ID, len: 1 }), { op: "set_cell", step: 15, row: 0, value: { k: "n", p: PATCH_ID, len: 1 } });
  for (const len of [0, 2, 1.5, NaN]) assert.throws(() => scoreCellOperation(m, 15, 0, { mode: "note", patch: PATCH_ID, len }));
  delete m.patches[PATCH_ID];
  assert.throws(() => scoreCellOperation(m, 0, 0, { mode: "note", patch: PATCH_ID, len: 1 }), /sound this score holds/);
  for (const pad of [0, 9]) assert.deepEqual(scoreCellOperation(m, 0, 0, { mode: "drum", pad }), { op: "set_cell", step: 0, row: 0, value: { k: "d", pad } });
  for (const pad of [-1, 10, 0.5]) assert.throws(() => scoreCellOperation(m, 0, 0, { mode: "drum", pad }));
});

test("cleanup stays available after header shrink and at capacity; invalid coordinates never do", () => {
  const m = model(); m.steps = 1; m.ready = false;
  for (let i = 0; i < 4096; i++) m.cells[`${Math.floor(i / 25)}:${i % 25}`] = { k: "d", pad: 0 };
  assert.deepEqual(scoreCellOperation(m, 255, 24, { mode: "clear" }), { op: "clear_cell", step: 255, row: 24 });
  assert.throws(() => scoreCellOperation(m, 255, 24, { mode: "drum", pad: 0 }), /authored/);
  m.steps = 256;
  assert.throws(() => scoreCellOperation(m, 255, 24, { mode: "drum", pad: 0 }), /4096/);
  assert.equal(scoreCellOperation(m, 0, 0, { mode: "drum", pad: 9 }).op, "set_cell", "existing active cells remain replaceable at capacity");
  for (const [step, row] of [[-1, 0], [256, 0], [0, 25], [0, -1], [0.5, 0]]) assert.throws(() => scoreCellOperation(m, step, row, { mode: "clear" }));
});

test("same-pitch holds allow adjacency but refuse overlap in either direction without shortening", () => {
  const m = model(); m.cells["4:0"] = { k: "n", p: PATCH_ID, len: 4 };
  assert.throws(() => scoreCellOperation(m, 0, 0, { mode: "note", patch: PATCH_ID, len: 5 }), /overlaps/);
  assert.throws(() => scoreCellOperation(m, 7, 0, { mode: "note", patch: PATCH_ID, len: 1 }), /overlaps/);
  assert.equal(scoreCellOperation(m, 0, 0, { mode: "note", patch: PATCH_ID, len: 4 }).op, "set_cell");
  assert.equal(scoreCellOperation(m, 8, 0, { mode: "note", patch: PATCH_ID, len: 8 }).op, "set_cell");
  assert.equal(scoreCellOperation(m, 4, 1, { mode: "note", patch: PATCH_ID, len: 4 }).op, "set_cell");
  assert.equal(scoreCellOperation(m, 4, 0, { mode: "note", patch: PATCH_ID, len: 1 }).op, "set_cell");
  assert.deepEqual(m.cells["4:0"], { k: "n", p: PATCH_ID, len: 4 });
});

test("hold continuations cross pages without additional onsets or hidden length adjustment", () => {
  const m = model(); m.steps = 32; m.cells["14:0"] = { k: "n", p: PATCH_ID, len: 5 }; m.cells["16:1"] = { k: "d", pad: 3 };
  assert.deepEqual([...scoreContinuations(m, 0)], ["15:0"]);
  assert.deepEqual([...scoreContinuations(m, 1)], ["16:0", "17:0", "18:0"]);
  assert.deepEqual([...scoreContinuations(m, 0, SCORE_WIDE_PAGE_STEPS)], ["15:0", "16:0", "17:0", "18:0"]);
  const holds = scoreHolds(m, scorePage(32, 1));
  assert.deepEqual(holds.get("16:0"), { onset: "14:0", p: PATCH_ID, last: false, others: [] });
  assert.deepEqual(holds.get("18:0"), { onset: "14:0", p: PATCH_ID, last: true, others: [] });
  assert.equal(holds.has("16:1"), false, "a pad hit is never a continuation");
  assert.equal(Object.keys(m.cells).length, 2);
});

test("the onset owning a coordinate is the cell itself or the hold covering it; a pad never covers", () => {
  const m = model(); m.cells["4:0"] = { k: "n", p: PATCH_ID, len: 3 }; m.cells["4:1"] = { k: "d", pad: 2 };
  assert.deepEqual(scoreOnsetAt(m, 4, 0), { key: "4:0", cell: m.cells["4:0"] });
  assert.deepEqual(scoreOnsetAt(m, 6, 0), { key: "4:0", cell: m.cells["4:0"] });
  assert.equal(scoreOnsetAt(m, 7, 0), null); assert.equal(scoreOnsetAt(m, 3, 0), null);
  assert.deepEqual(scoreOnsetAt(m, 4, 1), { key: "4:1", cell: m.cells["4:1"] });
  assert.equal(scoreOnsetAt(m, 5, 1), null);
});

test("the grid's extent follows live cells past a shortened header, never below it or past the global bound", () => {
  const m = model(); m.steps = 16;
  assert.equal(scoreExtent(m), 16);
  m.cells["30:3"] = { k: "n", p: PATCH_ID, len: 4 }; assert.equal(scoreExtent(m), 34, "a hold's tail counts");
  m.cells["255:3"] = { k: "d", pad: 1 }; assert.equal(scoreExtent(m), 256);
  m.steps = 256; m.cells = {}; assert.equal(scoreExtent(m), 256);
  assert.equal(scoreLastSoundingStep(m), -1); m.cells["30:3"] = { k: "n", p: PATCH_ID, len: 4 }; assert.equal(scoreLastSoundingStep(m), 33);
  assert.deepEqual([...scoreOccupiedSteps(m)], [30, 31, 32, 33]);
});

test("rack order, usage and pad glyphs are stable and pads never spend recipe budget", () => {
  const m = model(); const other = "c".repeat(64); m.patches[other] = PATCH;
  m.cells["0:0"] = { k: "n", p: other, len: 8 }; m.cells["0:1"] = { k: "n", p: PATCH_ID, len: 1 }; m.cells["1:1"] = { k: "n", p: PATCH_ID, len: 1 }; m.cells["2:2"] = { k: "d", pad: 9 };
  assert.deepEqual(scorePatchOrder(m), [PATCH_ID, other]);
  assert.deepEqual([...scorePatchUsage(m)], [[other, 1], [PATCH_ID, 2]]);
  assert.equal(PAD_CODES.length, 10); assert.equal(PAD_NAMES.length, 10); assert.equal(PAD_NAMES[0], "kick"); assert.equal(PAD_CODES[9], "CR");
});

test("playhead, time and bar/beat follow the export's timing rule", () => {
  assert.equal(scoreDurationMs(16, 120), 2000); assert.equal(scoreDurationMs(1, 240), 62);
  assert.equal(scorePlayheadStep(0, 120, 16, false), 0); assert.equal(scorePlayheadStep(2874, 120, 16, false), 15); assert.equal(scorePlayheadStep(1999, 120, 16, false), 15);
  assert.equal(scorePlayheadStep(2000, 120, 16, false), 15, "past the end parks on the last step");
  assert.equal(scorePlayheadStep(2000, 120, 16, true), 0, "a loop wraps to the start"); assert.equal(scorePlayheadStep(2125, 120, 16, true), 1);
  assert.equal(scorePlayheadStep(-5, 120, 16, false), 0); assert.equal(scorePlayheadStep(NaN, 120, 16, false), 0);
  assert.equal(scoreTimeLabel(0), "0:00.0"); assert.equal(scoreTimeLabel(61_500), "1:01.5"); assert.equal(scoreTimeLabel(-1), "0:00.0");
  assert.deepEqual(scoreBarBeat(0, 4), { bar: 1, beat: 1 }); assert.deepEqual(scoreBarBeat(22, 4), { bar: 2, beat: 2 }); assert.deepEqual(scoreBarBeat(12, 3), { bar: 2, beat: 1 });
});

test("diagnostics preserve every raw alternative, removal, overflow, out-of-length cell and source", () => {
  const m = model(); m.steps = 1; m.ready = false;
  const note: ScoreCell = { k: "n", p: PATCH_ID, len: 1 };
  m.headers.bpm = { selected: alternative(120), conflicts: [alternative(140, 2)] };
  m.cellSlots["255:24"] = slot(note, [null, { k: "d", pad: 9 }]);
  m.patchSlots[PATCH_ID] = slot(PATCH, [null]); delete m.patches[PATCH_ID];
  m.readiness.cells["255:24"] = { count: true, extent: true, recipe: true, overlap: true, conflict: true };
  m.readiness.patches[PATCH_ID] = { count: true, conflict: true }; m.readiness.headerConflicts = true;
  const evidence = scoreDiagnostics(m), cell = evidence.find(e => e.kind === "cell")!, recipe = evidence.find(e => e.kind === "patch")!;
  assert.equal(evidence.length, 3); assert.deepEqual(cell.alternatives.map(v => v.value), [note, null, { k: "d", pad: 9 }]);
  assert.deepEqual(cell.alternatives.map(v => v.source.opId), [1, 2, 3].map(n => n.toString(16).padStart(64, "0")));
  assert.equal(cell.removed, true); assert.deepEqual(cell.issues, ["count", "extent", "recipe", "overlap", "conflict"]);
  assert.equal(recipe.removed, true); assert.equal(recipe.alternatives[0].value, PATCH);
  // A chosen value is a list of operations now: choosing a legacy cell's value moves it into its layer.
  assert.deepEqual(scoreDiagnosticOperation(m, cell, 1), [{ op: "clear_cell", step: 255, row: 24 }]);
  assert.throws(() => scoreDiagnosticOperation(m, cell, 0), /authored/);
  assert.deepEqual(scoreDiagnosticOperation(m, recipe, 1), [{ op: "remove_patch", patch: PATCH_ID }]);
  assert.deepEqual(scoreDiagnosticOperation(m, evidence[0], 1), [{ op: "set_header", field: "bpm", value: 140 }]);
  assert.throws(() => scoreDiagnosticOperation(m, cell, 3), /no longer/);
  assert.deepEqual(scoreSlotDiagnostic(m, "cell", "255:24"), cell); assert.equal(scoreSlotDiagnostic(m, "cell", "0:0"), null);
});

test("attention keeps only slots a person must act on, in diagnostics order", () => {
  const m = model(); m.steps = 32;
  m.cellSlots["0:0"] = slot<ScoreCell>({ k: "d", pad: 0 }); m.cells["0:0"] = { k: "d", pad: 0 };
  m.cellSlots["4:0"] = slot<ScoreCell>({ k: "d", pad: 0 }, [{ k: "d", pad: 1 }]); m.cells["4:0"] = { k: "d", pad: 0 };
  m.cellSlots["8:0"] = slot<ScoreCell>({ k: "d", pad: 0 }, [null]);
  m.cellSlots["9:0"] = slot<ScoreCell>(null);
  m.cellSlots["40:0"] = slot<ScoreCell>({ k: "d", pad: 0 }); m.cells["40:0"] = { k: "d", pad: 0 }; m.readiness.cells["40:0"] = { count: false, extent: true, recipe: false, overlap: false, conflict: false };
  m.headers.bpm = { selected: alternative(120), conflicts: [alternative(140, 2)] };
  assert.deepEqual(scoreAttention(m).map(i => i.id), ["header/bpm", "cell/4:0", "cell/8:0", "cell/40:0"]);
  assert.equal(scoreAttention(m).find(i => i.id === "cell/8:0")!.removed, true, "a clear that beat a value is listed with that value");
  assert.equal(scoreAttention(m).some(i => i.id === "cell/9:0"), false, "a plain clear with nothing beaten is not a decision");
});

// Exercise the real compiled Svelte component, not a string pattern or substitute grid. The
// repository preloads provide a DOM before Svelte evaluates (same setup as the complete suite).
const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
// The runtime's event delegation checks for HTMLMediaElement on every event it dispatches.
Object.assign(globalThis, { MouseEvent: w.MouseEvent, Event: w.Event, MutationObserver: w.MutationObserver, HTMLMediaElement: w.HTMLMediaElement, KeyboardEvent: w.KeyboardEvent, HTMLInputElement: w.HTMLInputElement });
const { default: ScoreEditor } = await import("./ScoreEditor.svelte");
const { mount, unmount, flushSync } = await import("svelte");
type Surface = ReturnType<typeof surface>;
// `apply` makes the harness behave like the host's working projection: every accepted operation
// is folded into the model object in place, so a later gesture (an undo) sees what was written.
// The DOM is not re-rendered for it (the prop identity is unchanged), which is enough for tests
// that assert the operations sent. `cells` stays the same object as `notes` (see `model()`).
//
// The composer derives its layer surface from the prop, so a test that changes the score under a
// mounted composer and then clicks it must hand over a new model (the Harness fixture's `update`),
// as the host does; an in-place change is seen only by document-space work (undo, redo).
function surface(m: ScoreModel, options: { readonlyReason?: string; loading?: boolean; onaddpatch?: () => Promise<string>; onaudition?: (patch: JamPatch, note: number, durationMs: number, level?: number) => void; onauditionpad?: (pad: number) => void; playing?: boolean; nextPass?: boolean; refused?: number; positionMs?: number | null; loop?: boolean; onpost?: () => void; refuse?: (op: ScoreOp) => boolean; apply?: boolean; onroom?: () => number; selection?: ScoreSelection | null; onselection?: (selection: ScoreSelection) => void } = {}) {
  const target = document.createElement("div"); document.body.appendChild(target);
  const ops: unknown[] = [], notices: string[] = [], loops: boolean[] = [];
  const instance = mount(ScoreEditor, { target, props: {
    model: m, readonlyReason: options.readonlyReason ?? "", loading: options.loading ?? false, error: "", patchLibrary: [{ name: "SYNTH", patch: PATCH }],
    library: [{ name: "SYNTH", patch: PATCH, shelf: "preset" }, { name: "OTHER", patch: { ...PATCH, o: [{ w: 1, t: 0, c: 0, l: 80 }] }, shelf: "mine" }],
    onop: (op: ScoreOp) => { if (options.refuse?.(op)) return false; ops.push(op); if (options.apply) { const next = scoreApplyLocal(m, op); Object.assign(m, next, { cells: next.notes }); } }, onnotice: (text: string) => { notices.push(text); }, onaddpatch: options.onaddpatch ?? (async () => PATCH_ID),
    onaudition: options.onaudition, playing: options.playing ?? false, nextPass: options.nextPass ?? false, refused: options.refused ?? 0, positionMs: options.positionMs ?? null, loop: options.loop ?? false, onloop: (v: boolean) => { loops.push(v); },
    onpost: options.onpost, onauditionpad: options.onauditionpad, onroom: options.onroom, selection: options.selection ?? null, onselection: options.onselection,
  } }); flushSync();
  return { target, ops, notices, loops,
    click(selector: string) { const button = target.querySelector<HTMLButtonElement>(selector); assert.ok(button, selector); button.click(); flushSync(); },
    button(text: string) { const button = [...target.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent?.trim() === text); assert.ok(button, `button ${text}`); return button; },
    change(selector: string, value: string) { const input = target.querySelector<HTMLInputElement>(selector); assert.ok(input, selector); input.value = value; input.dispatchEvent(new w.Event("change", { bubbles: true })); flushSync(); },
    input(selector: string, value: string) { const input = target.querySelector<HTMLInputElement>(selector); assert.ok(input, selector); input.value = value; input.dispatchEvent(new w.Event("input", { bubbles: true })); flushSync(); },
    cell(key: string) { return target.querySelector<HTMLButtonElement>(`[data-score-cell="${key}"]`)!; },
    async done() { await unmount(instance); target.remove(); },
  };
}

test("compiled composer renders 25 pitches on a 16-step page and authors note, clear, drum and header operations", async () => {
  const m = model(); m.cells["0:0"] = { k: "n", p: PATCH_ID, len: 3 };
  const heard: [number, number][] = [];
  const ui = surface(m, { onaudition: (_patch, note, ms) => heard.push([note, ms]) });
  try {
    assert.equal(ui.target.querySelectorAll(".sc-lab").length, 25);
    assert.equal(ui.target.querySelectorAll("[data-score-cell]").length, 25 * SCORE_PAGE_STEPS, "a zero-width card pages 16 steps");
    assert.equal(ui.target.querySelector(".sc-lab")?.textContent, "C6");
    assert.equal([...ui.target.querySelectorAll(".sc-lab")].at(-1)?.textContent, "C4");
    assert.ok(ui.cell("0:0").classList.contains("n")); assert.ok(ui.cell("1:0").classList.contains("held")); assert.ok(ui.cell("2:0").classList.contains("end"));
    assert.equal(ui.cell("3:0").classList.contains("held"), false);
    ui.input('[aria-label="Note length in sixteenth steps"]', "4");
    ui.click('[data-score-cell="4:1"]');
    assert.deepEqual(heard, [[61, 500]], "placing a note auditions it for its hold at the authored tempo");
    ui.click('[aria-label="Erase"]'); ui.click('[data-score-cell="1:0"]');
    ui.click('[aria-label="Place"]'); ui.click('[data-score-mode="drum"]'); ui.click('[data-score-pad="9"]'); ui.click('[data-score-cell="15:24"]');
    ui.change('[aria-label="Score BPM"]', "240");
    // [desync fixes] contract change, owner decision 2026-10-20 (one note per sound per cell): new
    // notes and hits are layered `set_note`s; the legacy note the test seeded clears as the legacy
    // cell it is.
    assert.deepEqual(ui.ops, [
      { op: "set_note", step: 4, row: 1, value: { k: "n", p: PATCH_ID, len: 4 } },
      { op: "clear_cell", step: 0, row: 0 }, { op: "set_note", step: 15, row: 24, value: { k: "d", pad: 9 } },
      { op: "set_header", field: "bpm", value: 240 },
    ]);
    assert.equal(heard.length, 1, "pads and clears do not audition");
  } finally { await ui.done(); }
});

test("compiled composer selects an existing note on click and clears it on right-click, Delete or the inspector", async () => {
  const m = model(); m.cells["4:0"] = { k: "n", p: PATCH_ID, len: 3 }; m.cellSlots["4:0"] = slot<ScoreCell>(m.cells["4:0"]);
  const ui = surface(m);
  try {
    ui.click('[data-score-cell="5:0"]');
    assert.equal(ui.ops.length, 0, "clicking a continuation selects its onset instead of writing");
    assert.ok(ui.cell("4:0").classList.contains("sel")); assert.ok(ui.cell("6:0").classList.contains("sel"));
    assert.match(ui.target.textContent!, /C4/);
    ui.click('[aria-label="Lengthen this note"]');
    // Editing a legacy note moves it into its layer: the layered note first, then the legacy clear.
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 4, row: 0, value: { k: "n", p: PATCH_ID, len: 4 } }, { op: "clear_cell", step: 4, row: 0 }]);
    ui.click('[aria-label="Clear this note"]');
    assert.deepEqual(ui.ops.at(-1), { op: "clear_cell", step: 4, row: 0 });
    ui.cell("6:0").dispatchEvent(new w.MouseEvent("contextmenu", { bubbles: true, cancelable: true })); flushSync();
    assert.deepEqual(ui.ops.at(-1), { op: "clear_cell", step: 4, row: 0 }); assert.equal(ui.ops.length, 4);
  } finally { await ui.done(); }
});

test("compiled composer refuses missing recipes, invalid holds and read-only writes while preserving cleanup", async () => {
  const m = model(); m.patches = {}; m.patchSlots = {};
  let ui = surface(m);
  try {
    assert.ok(ui.cell("0:0").classList.contains("off")); ui.click('[data-score-cell="0:0"]'); assert.equal(ui.ops.length, 0);
    assert.match(ui.target.textContent!, /no sounds yet/);
  } finally { await ui.done(); }
  ui = surface(model(), { readonlyReason: "Awaiting current membership" });
  try {
    assert.equal(ui.cell("0:0").disabled, true);
    ui.click('[data-score-cell="0:0"]'); assert.equal(ui.ops.length, 0);
    assert.match(ui.target.textContent!, /Awaiting current membership/);
    assert.equal(ui.target.querySelector<HTMLInputElement>('[aria-label="Score BPM"]')!.disabled, true);
  } finally { await ui.done(); }
  ui = surface(model());
  try {
    // A new note near the end is fitted to the steps left (the preview shows exactly that);
    // only an existing note is never shortened behind the author's back.
    ui.click('[data-score-snap="1"]');
    ui.input('[aria-label="Note length in sixteenth steps"]', "2");
    ui.click('[data-score-cell="15:0"]');
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 15, row: 0, value: { k: "n", p: PATCH_ID, len: 1 } }]);
    assert.equal(ui.notices.length, 0);
    assert.match(ui.target.querySelector(".micro.hi")!.textContent!, /eighth/, "a forced fit is not remembered as the next hold");
  } finally { await ui.done(); }
});

test("a real pointer click writes exactly the previewed hold, snapping only moves the start, and a fit never collides", async () => {
  const m = model(); m.steps = 16; m.cells["5:4"] = { k: "n", p: PATCH_ID, len: 1 }; m.cells["7:6"] = { k: "n", p: PATCH_ID, len: 1 }; m.cells["5:6"] = { k: "n", p: PATCH_ID, len: 1 };
  const ui = surface(m);
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const click = (key: string) => { pointer("pointerdown", ui.cell(key)); pointer("pointerup", window); ui.cell(key).click(); flushSync(); };
  try {
    ui.input('[aria-label="Note length in sixteenth steps"]', "1");
    click("3:2");
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 2, row: 2, value: { k: "n", p: PATCH_ID, len: 1 } }], "an eighth snap moves the start; the hold stays one step");
    assert.match(ui.target.querySelector(".micro.hi")!.textContent!, /sixteenth/, "the click did not rewrite the hold");
    ui.input('[aria-label="Note length in sixteenth steps"]', "2"); ui.click('[data-score-snap="4"]');
    click("7:4");
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 7, row: 4, value: { k: "n", p: PATCH_ID, len: 2 } }, "the snapped run 4..7 crosses the note at 5, so the exact cell is meant");
    ui.cell("6:6").dispatchEvent(new w.PointerEvent("pointerover", { bubbles: true })); flushSync();
    assert.ok(ui.cell("6:6").classList.contains("target") && !ui.cell("7:6").classList.contains("target"), "the preview shows the one free cell between two notes");
    click("6:6");
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 6, row: 6, value: { k: "n", p: PATCH_ID, len: 1 } }, "fitted to the gap, never an overlap refusal");
    assert.equal(ui.notices.length, 0); assert.match(ui.target.querySelector(".micro.hi")!.textContent!, /eighth/);
  } finally { await ui.done(); }
});

test("lengthen is unavailable at the authored end, and the pad lane never folds under the scale view", async () => {
  const m = model(); m.steps = 16; m.cells["14:0"] = { k: "n", p: PATCH_ID, len: 2 }; m.cellSlots["14:0"] = slot<ScoreCell>(m.cells["14:0"]);
  const ui = surface(m);
  try {
    ui.click('[data-score-cell="15:0"]');
    assert.equal(ui.target.querySelector<HTMLButtonElement>('[aria-label="Lengthen this note"]')!.disabled, true);
    ui.click('[data-score-scale="minor"]');
    assert.ok(ui.cell("3:1").classList.contains("fold"));
    ui.click('[data-score-mode="drum"]');
    assert.equal(ui.cell("3:1").classList.contains("fold"), false, "C♯4 is the snare lane in pads mode");
    assert.equal(ui.cell("3:1").classList.contains("off"), false);
    ui.click('[data-score-cell="3:1"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 3, row: 1, value: { k: "d", pad: 1 } });
    assert.ok(ui.cell("3:13").classList.contains("fold"), "C♯5 is outside the scale and outside the lane, so it still folds");
  } finally { await ui.done(); }
});

test("pointer jitter inside the pressed cell is a click: the note gets the previewed hold and the hold is unchanged", async () => {
  const m = model(); m.steps = 16;
  // A neighbour for the forced-fit case at the end (row 7 only; the earlier rows never meet it).
  m.cells["4:7"] = { k: "n", p: PATCH_ID, len: 1 };
  const ui = surface(m);
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const doc = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
  const original = doc.elementFromPoint;
  try {
    let under: Element | null = null;
    doc.elementFromPoint = () => under;
    for (const hold of [2, 4]) {
      ui.input('[aria-label="Note length in sixteenth steps"]', String(hold));
      const row = hold; // a fresh row per hold
      pointer("pointerdown", ui.cell(`2:${row}`)); under = ui.cell(`2:${row}`); pointer("pointermove", window); pointer("pointermove", window);
      pointer("pointerup", window); ui.cell(`2:${row}`).click(); flushSync();
      assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 2, row, value: { k: "n", p: PATCH_ID, len: hold } }, `a jittered click writes the previewed ${hold}-step hold`);
      assert.match(ui.target.querySelector(".micro.hi")!.textContent!, hold === 2 ? /eighth/ : /quarter/, "the hold is still the chosen one");
      // Moving back inside the preview after lengthening never shortens below the preview.
      pointer("pointerdown", ui.cell(`8:${row}`)); under = ui.cell(`12:${row}`); pointer("pointermove", window); under = ui.cell(`9:${row}`); pointer("pointermove", window);
      pointer("pointerup", window);
      assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 8, row, value: { k: "n", p: PATCH_ID, len: hold } });
    }
    assert.equal(ui.ops.length, 4);
    // A preview forced shorter than the hold by a neighbour (4:7): drifting one cell inside it
    // writes the preview and leaves the hold alone; only lengthening beyond the preview changes it.
    ui.input('[aria-label="Note length in sixteenth steps"]', "4");
    pointer("pointerdown", ui.cell("2:7")); under = ui.cell("3:7"); pointer("pointermove", window); pointer("pointerup", window);
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 2, row: 7, value: { k: "n", p: PATCH_ID, len: 2 } });
    assert.match(ui.target.querySelector(".micro.hi")!.textContent!, /quarter/, "a forced fit plus drift is not a chosen length");
    pointer("pointerdown", ui.cell("8:7")); under = ui.cell("13:7"); pointer("pointermove", window); under = ui.cell("8:7"); pointer("pointermove", window);
    assert.ok(!ui.cell("13:7").classList.contains("drag") && ui.cell("11:7").classList.contains("drag"), "returning to the pressed cell returns to the preview, even past skipped cells");
    pointer("pointerup", window);
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 8, row: 7, value: { k: "n", p: PATCH_ID, len: 4 } });
  } finally { doc.elementFromPoint = original; await ui.done(); }
});

test("a read-only turn mid-press cancels the gesture and swallows its completion click even after the block lifts", async () => {
  const { default: Harness } = await import("../scripts/fixtures/ScoreEditorHarness.svelte");
  const m = model(); m.steps = 16;
  const ops: unknown[] = [], heard: number[] = [];
  const target = document.createElement("div"); document.body.appendChild(target);
  const harness = mount(Harness, { target, props: { props: { model: m, readonlyReason: "", loading: false, error: "", patchLibrary: [{ name: "SYNTH", patch: PATCH }], onop: (op: unknown) => { ops.push(op); }, onaddpatch: async () => PATCH_ID, onaudition: (_p: JamPatch, note: number) => { heard.push(note); }, onnotice: () => {} } } }); flushSync();
  const pointer = (type: string, el: EventTarget, init: PointerEventInit = {}) => { el.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const cell = (key: string) => target.querySelector<HTMLButtonElement>(`[data-score-cell="${key}"]`)!;
  const control = harness as unknown as { setReadonly: (reason: string) => void };
  try {
    pointer("pointerdown", cell("2:5"));
    control.setReadonly("reading"); flushSync();
    control.setReadonly(""); flushSync();
    pointer("pointerup", window); cell("2:5").click(); flushSync();
    assert.deepEqual(ops, []); assert.deepEqual(heard, []);
    await new Promise((r) => setTimeout(r, 5));
    pointer("pointerdown", cell("4:5")); pointer("pointerup", window); cell("4:5").click(); flushSync();
    assert.equal(ops.length, 1, "the next press places normally");
  } finally { await unmount(harness); target.remove(); }
});

test("a press cancelled with Escape is over: its release and click place nothing, and the next press works", async () => {
  const m = model(); m.steps = 16;
  const heard: number[] = [];
  const ui = surface(m, { onaudition: (_p, note) => heard.push(note) });
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  try {
    pointer("pointerdown", ui.cell("2:5"));
    ui.cell("2:5").focus(); ui.cell("2:5").dispatchEvent(new w.KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); flushSync();
    pointer("pointerup", window); ui.cell("2:5").click(); flushSync();
    assert.deepEqual(ui.ops, []); assert.deepEqual(heard, [], "the cancelled press's completion click is swallowed");
    await new Promise((r) => setTimeout(r, 5));
    pointer("pointerdown", ui.cell("4:5")); pointer("pointerup", window); ui.cell("4:5").click(); flushSync();
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 4, row: 5, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD } }], "a genuinely new press places as usual");
    assert.deepEqual(heard, [65]);
  } finally { await ui.done(); }
});

test("keyboard navigation keeps focus when the destination is a folded or past-the-end cell", async () => {
  const m = model(); m.steps = 32; m.cells["15:1"] = { k: "n", p: PATCH_ID, len: 1 }; m.cellSlots["15:1"] = slot<ScoreCell>(m.cells["15:1"]);
  m.cells["31:2"] = { k: "n", p: PATCH_ID, len: 1 }; m.cells["40:2"] = { k: "d", pad: 0 }; m.ready = false; // a tail past the end exists
  const ui = surface(m);
  const key = (el: Element, k: string) => { (el as HTMLElement).focus(); el.dispatchEvent(new w.KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true })); flushSync(); };
  try {
    ui.click('[data-score-scale="major"]');
    assert.ok(ui.cell("15:1").classList.contains("fold"), "the retained C♯4 note sits on a folded row");
    ui.click('[data-score-cell="15:1"]'); assert.equal(ui.ops.length, 0);
    key(ui.cell("15:1"), "ArrowRight");
    await new Promise((r) => setTimeout(r, 0)); flushSync();
    assert.ok(ui.cell("16:1").classList.contains("off"), "the destination cannot take a note");
    assert.equal(document.activeElement, ui.cell("16:1"), "but it can take focus");
    key(ui.cell("16:1"), "ArrowLeft");
    await new Promise((r) => setTimeout(r, 0)); flushSync();
    assert.equal(document.activeElement, ui.cell("15:1"));
    ui.click('[aria-label="Next score page"]'); ui.click('[data-score-cell="31:2"]');
    key(ui.cell("31:2"), "ArrowRight");
    await new Promise((r) => setTimeout(r, 0)); flushSync();
    assert.ok(ui.cell("32:2").classList.contains("past") && ui.cell("32:2").classList.contains("off"));
    assert.equal(document.activeElement, ui.cell("32:2"), "a past-the-end cell takes focus too");
    assert.equal(ui.ops.length, 0);
  } finally { await ui.done(); }
});

test("placement snaps to the chosen unit, remembers the last hold, and the pad lane places its row's pad", async () => {
  const m = model(); m.steps = 32;
  m.cells["8:4"] = { k: "n", p: PATCH_ID, len: 1 }; // the occupied snap target on row 4, below
  const ui = surface(m);
  try {
    ui.click('[data-score-cell="5:2"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 4, row: 2, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD } }, "an eighth snap lands the click on the even step for the default eighth hold");
    ui.click('[data-score-snap="4"]'); ui.click('[data-score-cell="7:3"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 4, row: 3, value: { k: "n", p: PATCH_ID, len: 2 } });
    ui.click('[data-score-snap="2"]'); ui.click('[data-score-cell="9:4"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 9, row: 4, value: { k: "n", p: PATCH_ID, len: 2 } }, "an occupied snap target means the exact cell was meant");
    ui.input('[aria-label="Note length in sixteenth steps"]', "4"); ui.click('[data-score-cell="12:5"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 12, row: 5, value: { k: "n", p: PATCH_ID, len: 4 } });
    assert.match(ui.target.querySelector(".sc-foot")!.textContent!, /quarter/, "the footer names the remembered hold");
    ui.click('[data-score-mode="drum"]'); ui.click('[data-score-pad="9"]');
    ui.click('[data-score-cell="3:3"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 3, row: 3, value: { k: "d", pad: 3 } }, "rows C4..A4 are the pad lane: the row decides the pad");
    assert.equal(ui.target.querySelector(".sc-lab.lane")!.textContent, "CR"); assert.match(ui.target.querySelector(".sc-lab.lane")!.getAttribute("title")!, /crash/);
    ui.click('[data-score-cell="3:20"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 3, row: 20, value: { k: "d", pad: 9 } }, "elsewhere the chosen pad is placed");
  } finally { await ui.done(); }
});

test("overlapping holds have one owner for colour, label, selection, audition and erase, and the other is an explicit choice", async () => {
  const flute = PATCH_ID, bass = "c".repeat(64);
  const pure = model(); pure.patches[bass] = { ...PATCH, o: [{ w: 2, t: -12, c: 0, l: 80 }] };
  // Reversed insertion order: the later-starting note is enumerated first, and must still lose.
  pure.cells["2:0"] = { k: "n", p: bass, len: 8 }; pure.cells["0:0"] = { k: "n", p: flute, len: 8 };
  assert.deepEqual(scoreOnsetAt(pure, 4, 0), { key: "0:0", cell: pure.cells["0:0"] });
  assert.deepEqual(scoreHolds(pure, scorePage(16, 0)).get("4:0"), { onset: "0:0", p: flute, last: false, others: ["2:0"] });
  assert.deepEqual(scoreNotesAt(pure, 4, 0).map((n) => n.key), ["0:0", "2:0"]);
  assert.deepEqual(scoreNotesAt(pure, 2, 0).map((n) => n.key), ["2:0", "0:0"], "a cell's own onset comes first, then the hold covering it");
  const heard: number[] = [];
  const ui = surface(pure, { onaudition: (patch) => heard.push(patch.o[0].t) });
  try {
    assert.equal(ui.cell("4:0").getAttribute("aria-label"), "C4, step 5: SYNTH continues (1 more here)");
    assert.ok(ui.cell("4:0").classList.contains("stacked"));
    ui.click('[data-score-cell="4:0"]');
    assert.ok(ui.cell("0:0").classList.contains("sel"), "selection follows the owner the label names");
    assert.ok(ui.target.querySelector('[data-score-note-select="2:0"]'), "the other note is offered, not hidden");
    ui.click('[aria-label="Listen"]'); ui.click('[data-score-cell="4:0"]');
    assert.deepEqual(heard, [0], "the audition sounds the owner, the flute");
    ui.cell("4:0").dispatchEvent(new w.MouseEvent("contextmenu", { bubbles: true, cancelable: true })); flushSync();
    assert.deepEqual(ui.ops, [{ op: "clear_cell", step: 0, row: 0 }], "erase clears the note the cell shows");
    ui.click('[aria-label="Place"]'); ui.click('[data-score-cell="4:0"]'); ui.click('[data-score-note-clear="2:0"]');
    assert.deepEqual(ui.ops.at(-1), { op: "clear_cell", step: 2, row: 0 }, "the other note clears only by explicit choice");
  } finally { await ui.done(); }
});

test("a cancelled drag writes nothing: pointercancel, Escape and a foreign pointer's release never commit", async () => {
  const m = model(); m.steps = 16;
  const ui = surface(m);
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const doc = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
  const original = doc.elementFromPoint;
  try {
    let under: Element | null = null;
    doc.elementFromPoint = () => under;
    pointer("pointerdown", ui.cell("2:5")); under = ui.cell("6:5"); pointer("pointermove", window);
    assert.ok(ui.cell("6:5").classList.contains("drag"));
    pointer("pointercancel", window);
    assert.equal(ui.cell("6:5").classList.contains("drag"), false); assert.equal(ui.ops.length, 0);
    pointer("pointerdown", ui.cell("2:5")); under = ui.cell("6:5"); pointer("pointermove", window);
    ui.cell("2:5").focus(); ui.cell("2:5").dispatchEvent(new w.KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); flushSync();
    assert.equal(ui.cell("6:5").classList.contains("drag"), false, "Escape drops the gesture");
    pointer("pointerup", window); assert.equal(ui.ops.length, 0, "a release after the cancel has nothing to commit");
    pointer("pointerdown", ui.cell("2:5")); under = ui.cell("6:5"); pointer("pointermove", window);
    pointer("pointerup", window, { pointerId: 2 });
    assert.ok(ui.cell("6:5").classList.contains("drag"), "another pointer's release is not this gesture's");
    assert.equal(ui.ops.length, 0);
    pointer("pointerup", window);
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 2, row: 5, value: { k: "n", p: PATCH_ID, len: 5 } }]);
  } finally { doc.elementFromPoint = original; await ui.done(); }
});

test("a drag whose start cell another edit filled meanwhile is refused instead of overwriting the newcomer", async () => {
  const { default: Harness } = await import("../scripts/fixtures/ScoreEditorHarness.svelte");
  const m = model(); m.steps = 16;
  const ops: unknown[] = [], notices: string[] = [];
  const target = document.createElement("div"); document.body.appendChild(target);
  const harness = mount(Harness, { target, props: { props: { model: m, readonlyReason: "", loading: false, error: "", patchLibrary: [{ name: "SYNTH", patch: PATCH }], onop: (op: unknown) => { ops.push(op); }, onaddpatch: async () => PATCH_ID, onnotice: (t: string) => { notices.push(t); } } } }); flushSync();
  const pointer = (type: string, el: EventTarget, init: PointerEventInit = {}) => { el.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const cell = (key: string) => target.querySelector<HTMLButtonElement>(`[data-score-cell="${key}"]`)!;
  const doc = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
  const original = doc.elementFromPoint;
  try {
    let under: Element | null = null;
    doc.elementFromPoint = () => under;
    pointer("pointerdown", cell("2:5")); under = cell("6:5"); pointer("pointermove", window);
    const other = "c".repeat(64);
    const cells = { "2:5": { k: "n", p: other, len: 1 } as ScoreCell };
    const arrived = { ...model(), steps: 16, patches: { ...model().patches, [other]: PATCH }, cells, notes: cells };
    (harness as unknown as { update: (next: ScoreModel) => void }).update(arrived); flushSync();
    pointer("pointerup", window);
    assert.deepEqual(ops, [], "the newcomer's note is not overwritten");
    assert.match(notices[0], /another edit/);
    assert.ok(cell("2:5").classList.contains("n"), "and it is what the grid now shows");
  } finally { doc.elementFromPoint = original; await unmount(harness); target.remove(); }
});

// Review of e7250d48, MEDIUM 1: a phrase drag kept only cells. A collaborator's note that replaced
// a carried (non-anchor) note during the drag was moved instead, with no warning.
test("a phrase drag whose carried note a collaborator replaced meanwhile moves nothing", async () => {
  const { default: Harness } = await import("../scripts/fixtures/ScoreEditorHarness.svelte");
  const doc = (second: ScoreCell | null) => {
    // The anchor is two steps long, so pressing its onset carries it (a one-step note's only cell is its resize handle).
    const m = layered({ [`0:0/${PATCH_ID}`]: { k: "n", p: PATCH_ID, len: 2 }, ...(second ? { [`2:2/${second.k === "n" ? second.p : "d"}`]: second } : {}) });
    m.steps = 16; return m;
  };
  for (const [name, second] of [["another sound", { k: "n", p: OTHER, vol: 30 } as ScoreCell], ["a changed value", { k: "n", p: PATCH_ID, len: 1, vol: 40 } as ScoreCell], ["removal", null]] as const) {
    const ops: unknown[] = [], notices: string[] = [];
    const target = document.createElement("div"); document.body.appendChild(target);
    const harness = mount(Harness, { target, props: { props: { model: doc({ k: "n", p: PATCH_ID, len: 1 }), readonlyReason: "", loading: false, error: "", patchLibrary: [{ name: "SYNTH", patch: PATCH }], onop: (op: unknown) => { ops.push(op); }, onaddpatch: async () => PATCH_ID, onnotice: (t: string) => { notices.push(t); } } } }); flushSync();
    const pointer = (type: string, el: EventTarget, init: PointerEventInit = {}) => { el.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
    const cell = (key: string) => target.querySelector<HTMLButtonElement>(`[data-score-cell="${key}"]`)!;
    const docRef = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
    const original = docRef.elementFromPoint;
    try {
      cell("0:0").focus(); cell("0:0").dispatchEvent(new w.KeyboardEvent("keydown", { key: "a", ctrlKey: true, bubbles: true, cancelable: true })); flushSync();
      let under: Element | null = null; docRef.elementFromPoint = () => under;
      pointer("pointerdown", cell("0:0")); under = cell("4:0"); pointer("pointermove", window);
      (harness as unknown as { update: (next: ScoreModel) => void }).update(doc(second)); flushSync();
      pointer("pointerup", window);
      assert.deepEqual(ops, [], `${name}: nothing was moved`);
      assert.match(notices.at(-1) ?? "", /changed while you were moving it/, name);
    } finally { docRef.elementFromPoint = original; await unmount(harness); target.remove(); }
  }
});

test("keyboard navigation keeps DOM focus with the selection across a page boundary", async () => {
  const m = model(); m.steps = 32; m.cells["15:4"] = { k: "n", p: PATCH_ID, len: 1 }; m.cellSlots["15:4"] = slot<ScoreCell>(m.cells["15:4"]);
  const ui = surface(m);
  const key = (el: Element, k: string) => { (el as HTMLElement).focus(); el.dispatchEvent(new w.KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true })); flushSync(); };
  try {
    ui.click('[data-score-cell="15:4"]'); assert.equal(ui.ops.length, 0, "clicking a note selects it");
    key(ui.cell("15:4"), "ArrowRight");
    await new Promise((r) => setTimeout(r, 0)); flushSync();
    assert.ok(ui.cell("16:4"), "the next page is shown"); assert.ok(ui.cell("16:4").classList.contains("sel"));
    assert.equal(document.activeElement, ui.cell("16:4"), "focus moved with the selection");
    key(ui.cell("16:4"), "ArrowLeft");
    await new Promise((r) => setTimeout(r, 0)); flushSync();
    assert.equal(document.activeElement, ui.cell("15:4"));
    key(ui.cell("15:4"), "Delete");
    assert.deepEqual(ui.ops, [{ op: "clear_cell", step: 15, row: 4 }], "the key after the page change still edits the score");
  } finally { await ui.done(); }
});

test("dragging a note's last cell resizes it, remembers the length, and the inspector reassigns its recipe", async () => {
  const other = "c".repeat(64);
  const m = model(); m.steps = 16; m.patches[other] = { ...PATCH, o: [{ w: 1, t: 0, c: 0, l: 80 }] }; m.patchSlots[other] = slot(m.patches[other]);
  // The note carries a level: every rewrite below (drag-resize, reassign) must keep it.
  m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 3, vol: 60 }; m.cellSlots["2:5"] = slot<ScoreCell>(m.cells["2:5"]);
  const heard: string[] = [];
  const ui = surface(m, { onaudition: (patch) => heard.push(patch.o[0].w === 1 ? "other" : "synth") });
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const doc = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
  const original = doc.elementFromPoint;
  try {
    let under: Element | null = null;
    doc.elementFromPoint = () => under;
    assert.ok(ui.cell("4:5").classList.contains("end"));
    pointer("pointerdown", ui.cell("4:5")); under = ui.cell("7:5"); pointer("pointermove", window);
    assert.ok(ui.cell("6:5").classList.contains("drag"));
    pointer("pointerup", window);
    // The seeded note is a legacy cell: an edit writes it into its layer, then clears the legacy cell.
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 2, row: 5, value: { k: "n", p: PATCH_ID, len: 6, vol: 60 } }, { op: "clear_cell", step: 2, row: 5 }], "a drag-resize keeps the note's level");
    assert.match(ui.target.querySelector(".micro.hi")!.textContent!, /6 \/ 16$/, "the resized length is the new remembered hold");
    pointer("pointerdown", ui.cell("4:5")); pointer("pointerup", window);
    assert.equal(ui.ops.length, 2, "a release without movement is not a resize");
    ui.click('[data-score-cell="3:5"]');
    ui.change('[aria-label="Sound of this note"]', other);
    assert.deepEqual(ui.ops.slice(-2), [{ op: "set_note", step: 2, row: 5, value: { k: "n", p: other, len: 3, vol: 60 } }, { op: "clear_cell", step: 2, row: 5 }], "a new sound is a new layer");
    assert.deepEqual(heard, ["other"], "a reassigned note is auditioned with its new sound");
  } finally { doc.elementFromPoint = original; await ui.done(); }
});

test("play from the selected step passes the export's millisecond offset; the scale view folds rows without hiding notes", async () => {
  const m = model(); m.steps = 16; m.cells["3:1"] = { k: "n", p: PATCH_ID, len: 1 };
  const plays: (number | undefined)[] = [];
  const target = document.createElement("div"); document.body.appendChild(target);
  const instance = mount(ScoreEditor, { target, props: { model: m, readonlyReason: "", loading: false, error: "", patchLibrary: [{ name: "SYNTH", patch: PATCH }], onop: () => true, onaddpatch: async () => PATCH_ID, onplay: (ms?: number) => { plays.push(ms); }, onnotice: () => {} } }); flushSync();
  const cell = (key: string) => target.querySelector<HTMLButtonElement>(`[data-score-cell="${key}"]`)!;
  try {
    target.querySelector<HTMLButtonElement>('[title="play [space]"]')!.click(); flushSync();
    cell("8:3").click(); flushSync();
    target.querySelector<HTMLButtonElement>('[aria-label="Play from the selected step"]')!.click(); flushSync();
    assert.deepEqual(plays, [undefined, scoreDurationMs(8, 120)]);
    target.querySelector<HTMLButtonElement>('[data-score-scale="minor"]')!.click(); flushSync();
    assert.ok(cell("3:1").classList.contains("fold"), "C♯4 is outside C minor");
    assert.ok(cell("3:1").classList.contains("n"), "the note there is still drawn");
    assert.ok(cell("5:1").classList.contains("off"), "nothing new is placed on a folded row");
    assert.equal(cell("5:0").classList.contains("off"), false);
    assert.match(target.querySelector("[data-score-scale-toggle]")!.textContent!, /C minor/);
    target.querySelector<HTMLButtonElement>("[data-score-scale-toggle]")!.click(); flushSync();
    assert.equal(cell("3:1").classList.contains("fold"), false);
  } finally { await unmount(instance); target.remove(); }
});

test("tempo helpers: tap tempo, scale rows, snap and the pad lane", () => {
  assert.equal(scoreTapTempo([0, 500]), 120); assert.equal(scoreTapTempo([0, 500, 1000, 1500, 2000]), 120, "only the last four taps count");
  assert.equal(scoreTapTempo([0, 3000]), null); assert.equal(scoreTapTempo([0]), null); assert.equal(scoreTapTempo([0, 100]), 240, "clamped to the metronome's bounds");
  assert.deepEqual([...scoreScaleRows("major", 0)].slice(0, 7), [0, 2, 4, 5, 7, 9, 11]); assert.equal(scoreScaleRows("chromatic", 5).size, 25);
  assert.ok(scoreScaleRows("minor", 9).has(0), "A minor contains C"); assert.equal(scoreScaleRows("pentatonic", 0).has(5), false);
  assert.equal(scoreSnapStart(7, 4, 16), 4); assert.equal(scoreSnapStart(15, 2, 8), 7, "never past the authored end"); assert.equal(scoreSnapStart(3, 3, 16), 3, "an unknown unit means no snap");
  assert.equal(scorePadForRow(9), 9); assert.equal(scorePadForRow(10), null); assert.equal(scorePadForRow(-1), null);
});

test("compiled composer keeps cells past a shortened length visible, hatched and clearable, never placeable", async () => {
  const m = model(); m.steps = 8; m.ready = false;
  m.cells["20:3"] = { k: "d", pad: 1 }; m.cellSlots["20:3"] = slot<ScoreCell>(m.cells["20:3"]);
  m.readiness.cells["20:3"] = { count: false, extent: true, recipe: false, overlap: false, conflict: false };
  const ui = surface(m);
  try {
    assert.equal(ui.target.querySelectorAll("[data-score-cell]").length, 25 * 16, "the grid pages over the extent, not only the authored length");
    ui.click('[aria-label="Next score page"]');
    const past = ui.cell("20:3");
    assert.ok(past.classList.contains("past")); assert.ok(past.classList.contains("pad")); assert.equal(past.disabled, false);
    assert.ok(ui.cell("19:3").classList.contains("off"), "nothing can be placed past the end"); ui.click('[data-score-cell="19:3"]'); assert.equal(ui.ops.length, 0);
    assert.equal(ui.cell("19:3").disabled, false, "but the cell stays focusable for keyboard navigation");
    assert.equal(ui.cell("21:3"), null, "the extent ends with the last live cell");
    ui.click('[aria-label="Erase"]'); past.click(); flushSync();
    assert.deepEqual(ui.ops, [{ op: "clear_cell", step: 20, row: 3 }]);
    // Owner decision 2026-10-20: nothing blocks playing or exporting, so a flag is to review, never to fix.
    assert.match(ui.target.querySelector(".sc-head")!.textContent!, /1 to review/);
  } finally { await ui.done(); }
});

test("compiled attention list exposes removed losing values, re-authors a chosen value and clears a shortened score's cells", async () => {
  const m = model(); m.steps = 1; m.ready = false;
  m.cellSlots["255:24"] = slot<ScoreCell>({ k: "n", p: PATCH_ID, len: 1 }, [null]);
  m.readiness.cells["255:24"] = { count: true, extent: true, recipe: false, overlap: false, conflict: true };
  const ui = surface(m);
  try {
    ui.click('[data-score-tab="score"]');
    assert.match(ui.target.textContent!, /1 to review/);
    ui.click('[data-score-evidence="cell/255:24"]');
    const body = ui.target.querySelector(".sc-attbody")!;
    assert.match(body.textContent!, /cleared/); assert.equal(body.querySelectorAll(".sc-alt").length, 2);
    assert.match(ui.target.querySelector('[data-score-evidence="cell/255:24"]')!.textContent!, /over cap.*past the end.*conflict/);
    ui.click('[data-score-alt="0"]');
    assert.equal(ui.ops.length, 0); assert.match(ui.notices[0], /authored/, "re-authoring a value past the end is refused like any placement");
    ui.button("clear").click(); flushSync();
    assert.deepEqual(ui.ops, [{ op: "clear_cell", step: 255, row: 24 }]);
  } finally { await ui.done(); }
});

test("compiled attention list pages 24 at a time and alternatives 8 at a time without dropping later slots", async () => {
  const m = model(); m.steps = 32;
  for (let step = 0; step < 25; step++) { m.cellSlots[`${step}:0`] = slot<ScoreCell>({ k: "d", pad: 0 }, [{ k: "d", pad: 1 }]); m.cells[`${step}:0`] = { k: "d", pad: 0 }; }
  m.cellSlots["24:0"] = { removed: false, register: { selected: alternative<ScoreCell>({ k: "d", pad: 0 }), conflicts: Array.from({ length: 9 }, (_, i) => alternative<ScoreCell>({ k: "d", pad: 1 }, i + 2)) } };
  const ui = surface(m);
  try {
    ui.click('[data-score-tab="score"]');
    assert.equal(ui.target.querySelectorAll("[data-score-evidence]").length, 24);
    assert.equal(ui.target.querySelector('[data-score-evidence="cell/24:0"]'), null);
    ui.click('[aria-label="Next evidence page"]'); ui.click('[data-score-evidence="cell/24:0"]');
    assert.equal(ui.target.querySelectorAll(".sc-alt").length, 8);
    ui.click('[aria-label="Next alternative page"]');
    assert.equal(ui.target.querySelectorAll(".sc-alt").length, 2);
    assert.ok(ui.target.querySelector('[data-score-alt="9"]'));
    ui.click('[data-score-alt="9"]');
    // Choosing a legacy cell's retained value moves it into its layer, then clears the legacy cell.
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 24, row: 0, value: { k: "d", pad: 1 } }, { op: "clear_cell", step: 24, row: 0 }]);
    ui.click('[aria-label="Next score page"]'); assert.ok(ui.cell("31:24"));
  } finally { await ui.done(); }
});

test("a queued library recipe does not enable note placement before native readback", async () => {
  const m = model();
  let resolve!: (id: string) => void;
  const pending = new Promise<string>(r => { resolve = r; });
  const ui = surface(m, { onaddpatch: () => pending });
  try {
    assert.ok(ui.target.querySelector('[data-lib-patch="SYNTH"]')!.classList.contains("indoc"), "a library tile already in the score is shown as such");
    ui.click('[data-lib-patch="OTHER"]');
    assert.equal(ui.target.querySelector('[data-lib-patch="OTHER"]')!.getAttribute("aria-busy"), "true");
    assert.equal(ui.cell("0:0").classList.contains("off"), false, "the current active recipe is still usable before the new queue resolves");
    resolve("c".repeat(64)); await pending; await Promise.resolve(); flushSync();
    assert.ok(ui.cell("0:0").classList.contains("off"));
    assert.ok(ui.target.querySelector(`[data-pending-patch="${"c".repeat(64)}"]`));
    assert.equal(ui.ops.length, 0);
  } finally { await ui.done(); }
});

test("compiled rack arms the removal of a recipe in use and removes an unused one at once", async () => {
  const m = model(); const other = "c".repeat(64); m.patches[other] = { ...PATCH, o: [{ w: 2, t: 0, c: 0, l: 80 }] }; m.patchSlots[other] = slot(m.patches[other]);
  m.cells["0:0"] = { k: "n", p: PATCH_ID, len: 1 };
  const ui = surface(m);
  try {
    ui.click(`[data-remove-patch="${other}"]`);
    assert.deepEqual(ui.ops, [{ op: "remove_patch", patch: other }]);
    ui.click(`[data-remove-patch="${PATCH_ID}"]`);
    assert.equal(ui.ops.length, 1, "a recipe in use arms first");
    assert.equal(ui.target.querySelector(`[data-remove-patch="${PATCH_ID}"]`)!.textContent, "remove anyway");
    ui.click(`[data-remove-patch="${PATCH_ID}"]`);
    assert.deepEqual(ui.ops.at(-1), { op: "remove_patch", patch: PATCH_ID });
  } finally { await ui.done(); }
});

test("compiled hotkeys: tool keys work anywhere in the composer, Backspace and arrows only with focus on the grid", async () => {
  const m = model(); m.cells["4:0"] = { k: "n", p: PATCH_ID, len: 2 }; m.cellSlots["4:0"] = slot<ScoreCell>(m.cells["4:0"]);
  const ui = surface(m);
  const key = (el: Element, k: string) => { (el as HTMLElement).focus(); el.dispatchEvent(new w.KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true })); flushSync(); };
  try {
    ui.click('[data-score-cell="5:0"]'); // selects the onset at 4:0
    const tab = ui.target.querySelector('[data-score-tab="score"]')!;
    key(tab, "Backspace"); key(tab, "Delete"); key(tab, "ArrowLeft");
    assert.equal(ui.ops.length, 0, "a key pressed on the inspector never edits the score");
    assert.ok(ui.cell("4:0").classList.contains("sel"), "nor moves the selection");
    key(tab, "e"); assert.equal(ui.target.querySelector('[aria-label="Erase"]')!.getAttribute("aria-pressed"), "true");
    key(tab, "d"); assert.equal(ui.target.querySelector('[data-score-mode="drum"]')!.getAttribute("aria-pressed"), "true");
    key(ui.cell("5:0"), "ArrowRight"); assert.ok(ui.cell("5:0").classList.contains("sel"), "the selection moves from the onset to the next cell");
    key(ui.cell("5:0"), "ArrowRight"); assert.ok(ui.cell("6:0").classList.contains("sel"));
    key(ui.cell("6:0"), "Backspace"); assert.equal(ui.ops.length, 0, "the selection moved to an empty cell, so there is nothing to clear");
    ui.click('[data-score-cell="5:0"]'); key(ui.cell("5:0"), "Delete");
    assert.deepEqual(ui.ops, [{ op: "clear_cell", step: 4, row: 0 }]);
  } finally { await ui.done(); }
});

test("compiled composer sets a hold by dragging along a row, clamps at the authored end and swallows the trailing click", async () => {
  const m = model(); m.steps = 8; m.cells["12:0"] = { k: "d", pad: 0 }; m.ready = false; // extent 13: columns past the end exist
  const ui = surface(m);
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const doc = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
  const original = doc.elementFromPoint;
  try {
    let under: Element | null = null;
    doc.elementFromPoint = () => under;
    pointer("pointerdown", ui.cell("2:5"));
    under = ui.cell("4:5"); pointer("pointermove", window);
    assert.ok(ui.cell("3:5").classList.contains("drag") && ui.cell("4:5").classList.contains("drag"), "the preview follows the pointer");
    under = ui.cell("4:6"); pointer("pointermove", window);
    assert.equal(ui.cell("4:6").classList.contains("drag"), false, "a hold never crosses rows");
    under = ui.cell("11:5"); pointer("pointermove", window);
    pointer("pointerup", window);
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 2, row: 5, value: { k: "n", p: PATCH_ID, len: 6 } }], "clamped at the authored end (step 7)");
    ui.cell("2:5").click(); flushSync();
    assert.equal(ui.ops.length, 1, "the click the browser fires after a drag is swallowed");
    await new Promise((r) => setTimeout(r, 5));
    ui.cell("3:3").click(); flushSync();
    assert.equal(ui.ops.length, 2, "the next click places again");
  } finally { doc.elementFromPoint = original; await ui.done(); }
});

test("compiled audition tool sounds the hold a placement would get, never past the end, and never a pad", async () => {
  const m = model(); m.steps = 8; m.cells["4:1"] = { k: "d", pad: 2 }; m.cells["0:2"] = { k: "n", p: PATCH_ID, len: 3 };
  const heard: [number, number][] = [];
  const ui = surface(m, { onaudition: (_p, note, ms) => heard.push([note, ms]) });
  try {
    ui.input('[aria-label="Note length in sixteenth steps"]', "256");
    ui.click('[aria-label="Listen"]');
    ui.click('[data-score-cell="6:3"]');
    assert.deepEqual(heard, [[63, scoreDurationMs(2, 120)]], "clamped to the two steps left before the end");
    ui.click('[data-score-cell="4:1"]'); ui.click('[data-score-cell="1:2"]');
    assert.deepEqual(heard, [[63, scoreDurationMs(2, 120)], [62, scoreDurationMs(3, 120)]], "a pad is silent; a hold sounds for its authored length");
    assert.equal(ui.ops.length, 0);
  } finally { await ui.done(); }
});

test("compiled composer skips the audition and the selection when the host refuses the operation", async () => {
  const m = model();
  const heard: number[] = [];
  const target = document.createElement("div"); document.body.appendChild(target);
  const instance = mount(ScoreEditor, { target, props: { model: m, readonlyReason: "", loading: false, error: "", patchLibrary: [], onop: () => false, onaddpatch: async () => PATCH_ID, onaudition: (_p: JamPatch, note: number) => heard.push(note), onnotice: () => {} } }); flushSync();
  try {
    target.querySelector<HTMLButtonElement>('[data-score-cell="3:3"]')!.click(); flushSync();
    assert.deepEqual(heard, []); assert.equal(target.querySelector('[data-score-cell="3:3"]')!.classList.contains("sel"), false);
  } finally { await unmount(instance); target.remove(); }
});

test("compiled grid hollows a whole hold whose recipe is gone and marks a removed recipe's clear as current", async () => {
  const m = model(); m.ready = false; const gone = "c".repeat(64);
  m.cells["4:0"] = { k: "n", p: gone, len: 3 }; m.cellSlots["4:0"] = slot<ScoreCell>(m.cells["4:0"]);
  m.readiness.cells["4:0"] = { count: false, extent: false, recipe: true, overlap: false, conflict: false };
  m.patchSlots[gone] = slot(PATCH, [null]); m.readiness.patches[gone] = { count: false, conflict: true };
  const ui = surface(m);
  try {
    for (const key of ["4:0", "5:0", "6:0"]) assert.ok(ui.cell(key).classList.contains("norecipe"), key);
    ui.click('[data-score-tab="score"]'); ui.click(`[data-score-evidence="patch/${gone}"]`);
    assert.match(ui.target.querySelector(".sc-alt.cur .t")!.textContent!, /cleared/);
    assert.equal(ui.target.querySelectorAll(".sc-alt.cur").length, 1);
  } finally { await ui.done(); }
});

test("compiled trim only ever shortens: cells past the end make the length a decision, not a fit", async () => {
  const m = model(); m.steps = 8; m.ready = false; m.cells["12:0"] = { k: "d", pad: 0 };
  const ui = surface(m);
  try {
    assert.equal(ui.button("trim").disabled, true);
    ui.button("+ bar").click(); flushSync();
    assert.deepEqual(ui.ops, [{ op: "set_header", field: "steps", value: 24 }]);
  } finally { await ui.done(); }
});

test("compiled transport shows the playhead, follows it across pages and exposes loop and length controls", async () => {
  const m = model(); m.steps = 64; m.cells["40:5"] = { k: "d", pad: 0 };
  let ui = surface(m, { playing: true, positionMs: scoreDurationMs(40, 120) + 1 });
  try {
    assert.ok(ui.cell("40:5"), "playback paged the grid to the playhead");
    assert.ok(ui.cell("40:5").classList.contains("ph")); assert.ok(ui.target.querySelector(".sc-ph"));
    assert.match(ui.target.querySelector(".sc-transport")!.textContent!, /41/);
    assert.ok(ui.target.querySelector('[title="stop [space]"]'));
    assert.equal(ui.target.querySelector<HTMLButtonElement>('[aria-label="Loop playback"]')!.disabled, true, "loop cannot change mid-playback");
  } finally { await ui.done(); }
  ui = surface(m);
  try {
    assert.ok(ui.target.querySelector('[title="play [space]"]'));
    ui.click('[aria-label="Loop playback"]'); assert.deepEqual(ui.loops, [true]);
    ui.button("+ bar").click(); flushSync();
    assert.deepEqual(ui.ops.at(-1), { op: "set_header", field: "steps", value: 80 });
    ui.button("trim").click(); flushSync();
    assert.deepEqual(ui.ops.at(-1), { op: "set_header", field: "steps", value: 41 });
    assert.equal(ui.target.querySelectorAll(".sc-ov").length, 64);
    ui.click('[aria-label="Go to step 50"]'); assert.ok(ui.cell("49:0"));
  } finally { await ui.done(); }
});

test("another pointer's press does not revive a cancelled press: its own release is still swallowed", async () => {
  const m = model(); m.steps = 16;
  const ui = surface(m);
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  try {
    pointer("pointerdown", ui.cell("2:5"));
    ui.cell("2:5").focus(); ui.cell("2:5").dispatchEvent(new w.KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); flushSync();
    // A second finger (pointer 2) presses and releases elsewhere while the first is still held.
    pointer("pointerdown", ui.cell("6:5"), { pointerId: 2 }); pointer("pointerup", window, { pointerId: 2 }); ui.cell("6:5").click(); flushSync();
    assert.equal(ui.ops.length, 1, "the second pointer's own tap is an ordinary click");
    // The first pointer's release and completion click are still the cancelled press's.
    pointer("pointerup", window); ui.cell("2:5").click(); flushSync();
    assert.equal(ui.ops.length, 1, "the cancelled press placed nothing");
    pointer("pointerdown", ui.cell("4:5")); pointer("pointerup", window); ui.cell("4:5").click(); flushSync();
    assert.equal(ui.ops.length, 2, "the same pointer's next press works normally");
  } finally { await ui.done(); }
});

test("a re-read of a score already on screen does not lock the grid; only a first read with nothing to show does", async () => {
  const m = model(); m.steps = 16;
  const ui = surface(m, { loading: true });
  try {
    assert.equal(ui.cell("3:3").disabled, false);
    assert.match(ui.target.textContent ?? "", /reading/);
    ui.click('[data-score-cell="3:3"]');
    assert.equal(ui.ops.length, 1, "an edit is accepted while the echo of a save is being read back");
  } finally { await ui.done(); }
  const empty = surface(null as unknown as ScoreModel, { loading: true });
  try { assert.equal(empty.target.querySelectorAll("[data-score-cell]").length, 0); } finally { await empty.done(); }
});

// Contract change, owner decision 2026-10-20 ("nothing blocks playback"): a score with flags used to
// disable post, download, share and play. It now plays and exports what it can.
test("post to chat is offered only with a host callback, and flags never disable it, play or export", async () => {
  const m = model(); m.steps = 16;
  const without = surface(m);
  try { assert.equal(without.target.querySelector("[data-score-post]"), null); } finally { await without.done(); }
  let posted = 0;
  const ui = surface(m, { onpost: () => { posted++; } });
  try {
    const post = ui.target.querySelector<HTMLButtonElement>("[data-score-post]")!;
    assert.ok(post); assert.equal(post.disabled, false);
    post.click(); flushSync(); assert.equal(posted, 1);
  } finally { await ui.done(); }
  const flagged = model(); flagged.steps = 16; flagged.ready = false; flagged.readiness.headerConflicts = true;
  const open = surface(flagged, { onpost: () => { posted++; } });
  try { assert.equal(open.target.querySelector<HTMLButtonElement>("[data-score-post]")!.disabled, false); } finally { await open.done(); }
});

test("a note's volume is edited in the selected panel, remembered for the next placement and carried by resize", async () => {
  const m = model(); m.steps = 16; m.cells["0:0"] = { k: "n", p: PATCH_ID, len: 2 };
  const heard: number[] = [];
  const ui = surface(m, { onaudition: (_patch, _note, _ms, level) => heard.push(level ?? -1) });
  try {
    ui.click('[data-score-cell="0:0"]');
    ui.change('[aria-label="Volume of this note"]', "60");
    // A legacy note's edit lands in its layer, and the legacy cell is cleared after it.
    const legacy = { op: "clear_cell", step: 0, row: 0 };
    assert.deepEqual(ui.ops.slice(-2), [{ op: "set_note", step: 0, row: 0, value: { k: "n", p: PATCH_ID, len: 2, vol: 60 } }, legacy]);
    assert.equal(heard.at(-1), 60, "the change is heard at the new level");
    // Placement uses the remembered level; a full level writes no field at all.
    ui.click('[data-score-cell="4:1"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 4, row: 1, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD, vol: 60 } });
    assert.equal(heard.at(-1), 60, "a placed note auditions at the level it was written with");
    ui.input('[aria-label="Placement volume"]', "100");
    ui.click('[data-score-cell="8:1"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 8, row: 1, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD } });
    // Out-of-range input falls back to full level: the next placement writes no field.
    ui.input('[aria-label="Placement volume"]', "0");
    ui.click('[data-score-cell="12:1"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 12, row: 1, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD } });
  } finally { await ui.done(); }
  // A note that already carries a level: the steppers move it by ten, and a resize keeps it.
  const quiet = model(); quiet.steps = 16; quiet.cells["0:0"] = { k: "n", p: PATCH_ID, len: 2, vol: 60 };
  const panel = surface(quiet);
  try {
    panel.click('[data-score-cell="0:0"]');
    assert.equal((panel.target.querySelector('[aria-label="Volume of this note"]') as HTMLInputElement).value, "60");
    const legacy = { op: "clear_cell", step: 0, row: 0 };
    panel.click('[aria-label="Louder"]');
    assert.deepEqual(panel.ops.slice(-2), [{ op: "set_note", step: 0, row: 0, value: { k: "n", p: PATCH_ID, len: 2, vol: 70 } }, legacy]);
    panel.click('[aria-label="Quieter"]');
    assert.deepEqual(panel.ops.slice(-2), [{ op: "set_note", step: 0, row: 0, value: { k: "n", p: PATCH_ID, len: 2, vol: 50 } }, legacy]);
    panel.click('[aria-label="Lengthen this note"]');
    assert.deepEqual(panel.ops.slice(-2), [{ op: "set_note", step: 0, row: 0, value: { k: "n", p: PATCH_ID, len: 3, vol: 60 } }, legacy], "a longer hold keeps the note's level");
    assert.ok(panel.target.querySelector('[data-score-cell="0:0"]')?.getAttribute("aria-label")?.includes("60%"), "the cell names its level");
  } finally { await panel.done(); }
});

// A move writes the note at its destination first and clears its old place after: a refusal part
// way leaves the note in both places, never in neither (docs/design-score-layers.md section 8).
test("dragging a note's body moves it to another step and pitch as a set and a clear; a blocked drop or no movement writes nothing", async () => {
  const m = model(); m.steps = 16; m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 3, vol: 60 }; m.cells["9:8"] = { k: "n", p: PATCH_ID, len: 1 };
  const heard: number[] = [];
  const ui = surface(m, { onaudition: (_p, note) => heard.push(note) });
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const doc = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
  const original = doc.elementFromPoint;
  try {
    let under: Element | null = null;
    doc.elementFromPoint = () => under;
    // Grabbed one cell into the note (3:5), carried to 7:8: the note keeps its offset, so it lands at 6:8.
    pointer("pointerdown", ui.cell("3:5")); under = ui.cell("7:8"); pointer("pointermove", window);
    for (const key of ["6:8", "7:8", "8:8"]) assert.ok(ui.cell(key).classList.contains("drag"), `${key} previews the landing`);
    assert.ok(ui.cell("2:5").classList.contains("lifting"), "the note's old place fades while it is carried");
    pointer("pointerup", window);
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 6, row: 8, value: { k: "n", p: PATCH_ID, len: 3, vol: 60 } }, { op: "clear_cell", step: 2, row: 5 }]);
    assert.deepEqual(heard, [68], "the moved note is heard at its new pitch");
    ui.ops.length = 0;
    // The model is the test's object, so the note is still at 2:5. A drop onto the note at 9:8 is refused.
    pointer("pointerdown", ui.cell("2:5")); under = ui.cell("8:8"); pointer("pointermove", window);
    assert.ok(ui.cell("9:8").classList.contains("bad"), "the landing shows it cannot take the note");
    pointer("pointerup", window);
    assert.deepEqual(ui.ops, []); assert.match(ui.notices.at(-1) ?? "", /in the way/);
    // No movement: the press is a click that selects.
    pointer("pointerdown", ui.cell("2:5")); pointer("pointerup", window); ui.cell("2:5").click(); flushSync();
    assert.deepEqual(ui.ops, []); assert.ok(ui.cell("2:5").classList.contains("sel"));
  } finally { doc.elementFromPoint = original; await ui.done(); }
});

test("alt plus an arrow nudges the selected note, and the selected panel's move buttons do the same", async () => {
  const m = model(); m.steps = 16; m.cells["4:4"] = { k: "n", p: PATCH_ID, len: 2 };
  const ui = surface(m);
  try {
    ui.click('[data-score-cell="4:4"]'); ui.cell("4:4").focus();
    ui.cell("4:4").dispatchEvent(new w.KeyboardEvent("keydown", { key: "ArrowUp", altKey: true, bubbles: true, cancelable: true })); flushSync();
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 4, row: 5, value: { k: "n", p: PATCH_ID, len: 2 } }, { op: "clear_cell", step: 4, row: 4 }]);
    ui.ops.length = 0;
    ui.click('[data-score-cell="4:4"]');
    ui.click('[aria-label="Move this note later"]');
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 5, row: 4, value: { k: "n", p: PATCH_ID, len: 2 } }, { op: "clear_cell", step: 4, row: 4 }]);
    ui.ops.length = 0;
    // An arrow without alt still moves the selection only.
    ui.cell("4:4").dispatchEvent(new w.KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true })); flushSync();
    assert.deepEqual(ui.ops, []);
  } finally { await ui.done(); }
});

test("undo and redo replay a gesture's previous values; the buttons and ctrl z / ctrl shift z share one stack", async () => {
  const m = model(); m.steps = 16; m.cells["0:0"] = { k: "n", p: PATCH_ID, len: 2, vol: 40 };
  const ui = surface(m, { apply: true });
  const key = (k: string, init: KeyboardEventInit = {}) => { ui.cell("0:0").focus(); ui.cell("0:0").dispatchEvent(new w.KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init })); flushSync(); };
  try {
    assert.equal(ui.target.querySelector<HTMLButtonElement>("[data-score-undo]")!.disabled, true);
    ui.click('[data-score-cell="4:1"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 4, row: 1, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD } });
    assert.equal(ui.target.querySelector<HTMLButtonElement>("[data-score-undo]")!.disabled, false);
    key("z", { ctrlKey: true });
    assert.deepEqual(ui.ops.at(-1), { op: "clear_note", step: 4, row: 1, layer: PATCH_ID }, "undoing a placement clears the note it wrote");
    key("z", { ctrlKey: true, shiftKey: true });
    assert.deepEqual(ui.ops.at(-1), { op: "set_note", step: 4, row: 1, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD } }, "redo places it again");
    ui.ops.length = 0;
    ui.click('[aria-label="Erase"]'); ui.click('[data-score-cell="1:0"]');
    assert.deepEqual(ui.ops, [{ op: "clear_cell", step: 0, row: 0 }]);
    ui.click("[data-score-undo]");
    // The erased note was a legacy cell, so undo restores it there exactly (scoreInverse).
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 0, row: 0, value: { k: "n", p: PATCH_ID, len: 2, vol: 40 } }, "undoing an erase restores the note with its length and level");
    ui.click("[data-score-redo]");
    assert.deepEqual(ui.ops.at(-1), { op: "clear_cell", step: 0, row: 0 });
    assert.equal(ui.target.querySelector<HTMLButtonElement>("[data-score-redo]")!.disabled, true);
    assert.ok(ui.target.querySelector("[data-score-undo]")!.getAttribute("title")!.includes("erase"));
  } finally { await ui.done(); }
});

test("sounds can be listened to without touching the document, and drums are heard when chosen or placed", async () => {
  const m = model(); m.steps = 16;
  const heard: [number, number][] = [], pads: number[] = [];
  const ui = surface(m, { onaudition: (_p, note, ms) => heard.push([note, ms]), onauditionpad: (pad) => pads.push(pad) });
  try {
    ui.click(`[data-doc-listen="${PATCH_ID}"]`);
    assert.deepEqual(heard, [[72, 500]], "one beat at C5, at the score's tempo");
    ui.click('[data-lib-listen="OTHER"]');
    assert.equal(heard.length, 2); assert.deepEqual(ui.ops, [], "listening adds nothing to the score");
    ui.click('[data-score-mode="drum"]'); ui.click('[data-score-pad="3"]');
    assert.deepEqual(pads, [3], "choosing a drum plays it");
    ui.click('[data-score-cell="2:20"]');
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 2, row: 20, value: { k: "d", pad: 3 } }]);
    assert.deepEqual(pads, [3, 3], "placing a hit plays it");
    ui.click('[aria-label="Listen"]'); ui.click('[data-score-cell="5:20"]');
    assert.deepEqual(pads, [3, 3, 3], "the listen tool on an empty drum cell plays the chosen drum");
  } finally { await ui.done(); }
});

test("the selected note's panel stays pinned above every inspector tab, and one grid cell is tabbable", async () => {
  const m = model(); m.steps = 16; m.cells["3:2"] = { k: "n", p: PATCH_ID, len: 2 };
  const ui = surface(m);
  try {
    assert.equal(ui.target.querySelector("[data-score-selected]"), null);
    assert.equal(ui.cell("0:24").tabIndex, 0, "the page's first cell takes the tab stop before anything is selected");
    assert.equal(ui.cell("3:2").tabIndex, -1);
    ui.click('[data-score-cell="3:2"]');
    assert.ok(ui.target.querySelector("[data-score-selected]"));
    assert.equal(ui.cell("3:2").tabIndex, 0); assert.equal(ui.cell("0:24").tabIndex, -1);
    ui.click('[data-score-tab="score"]');
    assert.ok(ui.target.querySelector('[data-score-selected] [aria-label="Sound of this note"]'), "the selected note is still editable from the score tab");
    ui.click('[data-score-tab="history"]');
    assert.ok(ui.target.querySelector('[data-score-selected] [aria-label="Lengthen this note"]'));
  } finally { await ui.done(); }
});

test("repeat bar copies the current bar one bar later and grows the score when it has to", async () => {
  const m = model(); m.steps = 16; m.cells["0:5"] = { k: "n", p: PATCH_ID, len: 4 }; m.cells["8:0"] = { k: "d", pad: 2 };
  const ui = surface(m);
  try {
    ui.click('[data-score-tab="score"]');
    ui.click("[data-score-repeat-bar]");
    assert.deepEqual(ui.ops, [
      { op: "set_header", field: "steps", value: 32 },
      { op: "set_note", step: 16, row: 5, value: { k: "n", p: PATCH_ID, len: 4 } },
      { op: "set_note", step: 24, row: 0, value: { k: "d", pad: 2 } },
    ], "the length grows first, then the cells in step order");
    ui.click("[data-score-undo]");
    assert.deepEqual(ui.ops.slice(3), [
      { op: "clear_note", step: 24, row: 0, layer: "d" }, { op: "clear_note", step: 16, row: 5, layer: PATCH_ID }, { op: "set_header", field: "steps", value: 16 },
    ], "undoing the repeat clears the copies and restores the length, in reverse");
  } finally { await ui.done(); }
});

test("an alt plus arrow nudge is taken on the capture phase and never reaches the app's history keys", async () => {
  const m = model(); m.steps = 16; m.cells["4:4"] = { k: "n", p: PATCH_ID, len: 2 };
  const ui = surface(m);
  const seen: string[] = [];
  const appHistory = (e: KeyboardEvent) => { if (e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) seen.push(e.key); };
  window.addEventListener("keydown", appHistory);
  try {
    ui.click('[data-score-cell="4:4"]'); ui.cell("4:4").focus();
    ui.cell("4:4").dispatchEvent(new w.KeyboardEvent("keydown", { key: "ArrowLeft", altKey: true, bubbles: true, cancelable: true })); flushSync();
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 3, row: 4, value: { k: "n", p: PATCH_ID, len: 2 } }, { op: "clear_cell", step: 4, row: 4 }]);
    assert.deepEqual(seen, [], "the app's own Alt+arrow listener did not see the nudge");
    // Focus outside the grid: the key is the app's again.
    ui.target.querySelector<HTMLButtonElement>('[data-score-tab="score"]')!.focus();
    window.dispatchEvent(new w.KeyboardEvent("keydown", { key: "ArrowLeft", altKey: true, bubbles: true, cancelable: true })); flushSync();
    assert.deepEqual(seen, ["ArrowLeft"]); assert.equal(ui.ops.length, 2);
  } finally { window.removeEventListener("keydown", appHistory); await ui.done(); }
});

test("undo and redo meet the hand-edit guards against the document as it is now, and record fresh inverses", async () => {
  const m = model(); m.steps = 16; m.cells["0:0"] = { k: "n", p: PATCH_ID, len: 2 };
  const ui = surface(m, { apply: true });
  try {
    ui.click('[aria-label="Erase"]'); ui.click('[data-score-cell="0:0"]');
    assert.deepEqual(ui.ops, [{ op: "clear_cell", step: 0, row: 0 }]);
    // A collaborator's note now sits under the erased note's hold: the undo is refused, not forced.
    m.cells["1:0"] = { k: "n", p: PATCH_ID, len: 1 };
    ui.click("[data-score-undo]");
    assert.equal(ui.ops.length, 1, "nothing was sent"); assert.match(ui.notices.at(-1) ?? "", /overlaps/);
    assert.equal(ui.target.querySelector<HTMLButtonElement>("[data-score-undo]")!.disabled, false, "the gesture stays undoable for later");
    delete m.cells["1:0"];
    ui.click("[data-score-undo]");
    // The erased note was a legacy cell, and undo restores legacy cells exactly where they were.
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 0, row: 0, value: { k: "n", p: PATCH_ID, len: 2 } });
    // Between the undo and its redo someone else writes the cell; the redo replaces it, and the
    // next undo restores THEIR note instead of clearing the cell.
    m.cells["0:0"] = { k: "d", pad: 7 };
    ui.click("[data-score-redo]");
    assert.deepEqual(ui.ops.at(-1), { op: "clear_cell", step: 0, row: 0 }, "redo of the erase");
    ui.click("[data-score-undo]");
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 0, row: 0, value: { k: "d", pad: 7 } }, "the inverse recorded at redo time is the collaborator's value");
  } finally { await ui.done(); }
});

test("a gesture the host refuses midway is rolled back, so a move is never left half done", async () => {
  const m = model(); m.steps = 16; m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 3, vol: 60 };
  // The move writes the destination first; refusing its clear leaves an accepted write to put back.
  const ui = surface(m, { refuse: (op) => op.op === "clear_cell" && op.step === 2 });
  try {
    const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
    const doc = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
    const original = doc.elementFromPoint;
    try {
      let under: Element | null = null; doc.elementFromPoint = () => under;
      pointer("pointerdown", ui.cell("2:5")); under = ui.cell("6:5"); pointer("pointermove", window); pointer("pointerup", window);
    } finally { doc.elementFromPoint = original; }
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 6, row: 5, value: { k: "n", p: PATCH_ID, len: 3, vol: 60 } }, { op: "clear_note", step: 6, row: 5, layer: PATCH_ID }], "the accepted write was taken back");
    assert.match(ui.notices.at(-1) ?? "", /nothing was changed/);
    assert.equal(ui.target.querySelector<HTMLButtonElement>("[data-score-undo]")!.title.includes("move"), false, "a rolled-back gesture is not recorded");
  } finally { await ui.done(); }
});

// Review of e7250d48, HIGH 2: rollback kept running older inverses after a newer one was refused.
// A phrase move [set 9 <- pad 2, set 1 <- pad 1, clear 0, clear 8] refused at "clear 8" put back
// "set 0 <- pad 1" (refused, the queue is full), then ran "clear 1", the only copy of pad 1 left.
test("a rollback stops at its first refused step, so it never clears the last copy of a note", async () => {
  const m = model(); m.steps = 16; m.notes = { "0:0/d": { k: "d", pad: 1 }, "8:0/d": { k: "d", pad: 2 } };
  const attempts: ScoreOp[] = [];
  const ui = surface(m, { refuse: (op) => {
    attempts.push(op);
    return (op.op === "clear_note" && op.step === 8) || (op.op === "set_note" && op.step === 0);
  } });
  const key = (k: string, init: KeyboardEventInit = {}) => { ui.cell("0:0").focus(); ui.cell("0:0").dispatchEvent(new w.KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init })); flushSync(); };
  try {
    key("a", { ctrlKey: true });
    ui.click('[aria-label="Move the phrase later"]');
    const accepted = ui.ops as ScoreOp[];
    assert.deepEqual(accepted, [
      { op: "set_note", step: 9, row: 0, value: { k: "d", pad: 2 } }, { op: "set_note", step: 1, row: 0, value: { k: "d", pad: 1 } },
      { op: "clear_note", step: 0, row: 0, layer: "d" },
    ], "the forward prefix, and nothing the failed rollback depended on");
    assert.equal(attempts.some((op) => op.op === "clear_note" && op.step === 1), false, "the clear of pad 1's only copy was never attempted");
    const after = accepted.reduce(scoreApplyLocal, m).notes;
    assert.deepEqual(Object.values(after).filter((c) => c.k === "d").map((c) => (c as { pad: number }).pad).sort(), [1, 2, 2], "pad 1 survives (at its new place)");
    assert.match(ui.notices.at(-1) ?? "", /only partly/);
    assert.match(ui.target.querySelector<HTMLButtonElement>("[data-score-undo]")!.title, /move the phrase/, "the surviving part stays undoable");
  } finally { await ui.done(); }
});

test("the right part of a note's last cell is the resize handle and the rest of the note moves it", async () => {
  const m = model(); m.steps = 16; m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 3 };
  const ui = surface(m);
  const end = ui.cell("4:5");
  Object.defineProperty(end, "clientWidth", { value: 100, configurable: true });
  const press = (offsetX: number) => { const e = new w.PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 1, button: 0 }); Object.defineProperty(e, "offsetX", { value: offsetX }); end.dispatchEvent(e); flushSync(); };
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const doc = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
  const original = doc.elementFromPoint;
  try {
    let under: Element | null = null; doc.elementFromPoint = () => under;
    press(80); under = ui.cell("6:5"); pointer("pointermove", window);
    assert.ok(ui.cell("6:5").classList.contains("drag") && !ui.cell("2:5").classList.contains("lifting"), "the handle resizes");
    pointer("pointerup", window);
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 2, row: 5, value: { k: "n", p: PATCH_ID, len: 5 } }, { op: "clear_cell", step: 2, row: 5 }]);
    ui.ops.length = 0;
    press(20); under = ui.cell("6:7"); pointer("pointermove", window);
    assert.ok(ui.cell("2:5").classList.contains("lifting"), "the body moves");
    pointer("pointerup", window);
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 4, row: 7, value: { k: "n", p: PATCH_ID, len: 3 } }, { op: "clear_cell", step: 2, row: 5 }], "grabbed two cells in, so the note lands two cells before the pointer");
  } finally { doc.elementFromPoint = original; await ui.done(); }
});

test("shift+click and a shift+drag marquee build a phrase; it moves, nudges and clears as one gesture", async () => {
  const m = model(); m.steps = 16; m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 2 }; m.cells["4:7"] = { k: "n", p: PATCH_ID, len: 2 }; m.cells["3:0"] = { k: "d", pad: 3 }; m.cells["12:9"] = { k: "n", p: PATCH_ID, len: 1 };
  const ui = surface(m);
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const shiftClick = (key: string) => { pointer("pointerdown", ui.cell(key), { shiftKey: true }); pointer("pointerup", window, { shiftKey: true }); ui.cell(key).dispatchEvent(new w.MouseEvent("click", { bubbles: true, shiftKey: true })); flushSync(); };
  const doc = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
  const original = doc.elementFromPoint;
  try {
    ui.click('[data-score-cell="2:5"]'); shiftClick("4:7");
    assert.ok(ui.cell("2:5").classList.contains("grp") && ui.cell("4:7").classList.contains("grp"), "the selected note and the shift-clicked one form the phrase");
    assert.match(ui.target.querySelector("[data-score-phrase]")?.parentElement?.textContent ?? "", /2 notes/);
    shiftClick("4:7");
    assert.equal(ui.target.querySelector("[data-score-phrase]"), null, "shift+click again takes it out");
    // A marquee from 1:0 to 6:9 takes the two notes and the hit inside it, not the note at 12:9.
    let under: Element | null = null; doc.elementFromPoint = () => under;
    pointer("pointerdown", ui.cell("1:0"), { shiftKey: true }); under = ui.cell("6:9"); pointer("pointermove", window);
    assert.ok(ui.cell("5:4").classList.contains("mq"), "the rectangle is tinted while it is drawn");
    pointer("pointerup", window);
    assert.match(ui.target.querySelector("[data-score-phrase]")?.parentElement?.textContent ?? "", /3 notes/);
    assert.equal(ui.cell("12:9").classList.contains("grp"), false);
    // Alt+arrow moves the whole phrase: every note written at its new place first (against the
    // direction of travel, the highest note first when moving up), then the old places cleared.
    ui.cell("2:5").focus();
    ui.cell("2:5").dispatchEvent(new w.KeyboardEvent("keydown", { key: "ArrowUp", altKey: true, bubbles: true, cancelable: true })); flushSync();
    assert.deepEqual(ui.ops, [
      { op: "set_note", step: 4, row: 8, value: { k: "n", p: PATCH_ID, len: 2 } }, { op: "set_note", step: 2, row: 6, value: { k: "n", p: PATCH_ID, len: 2 } }, { op: "set_note", step: 3, row: 1, value: { k: "d", pad: 3 } },
      { op: "clear_cell", step: 2, row: 5 }, { op: "clear_cell", step: 3, row: 0 }, { op: "clear_cell", step: 4, row: 7 },
    ]);
    ui.ops.length = 0;
    // The model is the test's object, so the phrase is picked at its new keys but those cells are empty: it dissolves.
    assert.equal(ui.target.querySelector("[data-score-phrase]"), null);
    // Dragging one note of a phrase carries all of it; a blocked member refuses the whole move.
    pointer("pointerdown", ui.cell("1:0"), { shiftKey: true }); under = ui.cell("6:9"); pointer("pointermove", window); pointer("pointerup", window);
    pointer("pointerdown", ui.cell("2:5")); under = ui.cell("10:7"); pointer("pointermove", window);
    assert.ok(ui.cell("12:9").classList.contains("bad"), "the note at 4:7 would land on the note at 12:9");
    pointer("pointerup", window);
    assert.deepEqual(ui.ops, []); assert.match(ui.notices.at(-1) ?? "", /in the way/);
    pointer("pointerdown", ui.cell("2:5")); under = ui.cell("3:5"); pointer("pointermove", window); pointer("pointerup", window);
    assert.deepEqual(ui.ops.map((op) => (op as { op: string }).op), ["set_note", "set_note", "set_note", "clear_cell", "clear_cell", "clear_cell"], "one step later for all three");
    ui.ops.length = 0;
    // Delete clears the phrase.
    pointer("pointerdown", ui.cell("1:0"), { shiftKey: true }); under = ui.cell("6:9"); pointer("pointermove", window); pointer("pointerup", window);
    ui.cell("2:5").focus(); ui.cell("2:5").dispatchEvent(new w.KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true })); flushSync();
    assert.deepEqual(ui.ops, [{ op: "clear_cell", step: 2, row: 5 }, { op: "clear_cell", step: 3, row: 0 }, { op: "clear_cell", step: 4, row: 7 }]);
    assert.equal(ui.target.querySelector("[data-score-selected]"), null);
  } finally { doc.elementFromPoint = original; await ui.done(); }
});

test("ctrl+a picks every note, ctrl+d duplicates the phrase where it ends, escape returns to one note", async () => {
  const m = model(); m.steps = 16; m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 2 }; m.cells["5:5"] = { k: "n", p: PATCH_ID, len: 3 };
  const ui = surface(m);
  const key = (k: string, init: KeyboardEventInit = {}) => { ui.cell("2:5").focus(); ui.cell("2:5").dispatchEvent(new w.KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init })); flushSync(); };
  try {
    key("a", { ctrlKey: true });
    assert.match(ui.target.querySelector("[data-score-phrase]")?.parentElement?.textContent ?? "", /2 notes/);
    key("d", { ctrlKey: true });
    assert.deepEqual(ui.ops, [
      { op: "set_note", step: 8, row: 5, value: { k: "n", p: PATCH_ID, len: 2 } },
      { op: "set_note", step: 11, row: 5, value: { k: "n", p: PATCH_ID, len: 3 } },
    ], "the copy starts at step 8, where the phrase (2..8) ends");
    ui.ops.length = 0;
    key("Escape");
    assert.equal(ui.target.querySelector("[data-score-phrase]"), null, "escape dissolves the phrase");
    ui.click('[data-score-cell="5:5"]');
    ui.click('[aria-label="Duplicate this note"]');
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 8, row: 5, value: { k: "n", p: PATCH_ID, len: 3 } }], "one note duplicates right after itself");
  } finally { await ui.done(); }
});

test("a gesture longer than the host's room is refused whole, before anything is sent", async () => {
  const m = model(); m.steps = 16; for (let step = 0; step < 6; step++) m.cells[`${step * 2}:5`] = { k: "n", p: PATCH_ID, len: 1 };
  const ui = surface(m, { onroom: () => 8 });
  const key = (k: string, init: KeyboardEventInit = {}) => { ui.cell("0:5").focus(); ui.cell("0:5").dispatchEvent(new w.KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init })); flushSync(); };
  try {
    key("a", { ctrlKey: true });
    key("ArrowUp", { altKey: true });
    assert.deepEqual(ui.ops, [], "six notes need twelve operations; eight fit, so nothing is sent");
    assert.equal(ui.notices.length, 1); assert.match(ui.notices[0], /too many changes at once \(12\)/);
    key("Delete");
    assert.equal(ui.ops.length, 6, "six clears fit in eight");
  } finally { await ui.done(); }
});

test("the phrase preview agrees with the write: a phrase that overlaps itself or lost a sound shows red and sends nothing", async () => {
  const m = model(); m.steps = 16; m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 4 }; m.cells["4:5"] = { k: "n", p: PATCH_ID, len: 2 };
  const ui = surface(m);
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const doc = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
  const original = doc.elementFromPoint;
  try {
    let under: Element | null = null; doc.elementFromPoint = () => under;
    pointer("pointerdown", ui.cell("1:4"), { shiftKey: true }); under = ui.cell("6:6"); pointer("pointermove", window); pointer("pointerup", window);
    assert.match(ui.target.querySelector("[data-score-phrase]")?.parentElement?.textContent ?? "", /2 notes/);
    pointer("pointerdown", ui.cell("2:5")); under = ui.cell("2:8"); pointer("pointermove", window);
    assert.ok(ui.cell("2:8").classList.contains("bad"), "the two holds overlap each other, so the phrase cannot be placed anywhere");
    pointer("pointerup", window);
    assert.deepEqual(ui.ops, []); assert.match(ui.notices.at(-1) ?? "", /in the way/);
  } finally { doc.elementFromPoint = original; await ui.done(); }
});

test("taking a note out of a two-note phrase leaves one ordinary selection, and a press outside the phrase ends it", async () => {
  const m = model(); m.steps = 16; m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 2 }; m.cells["6:7"] = { k: "n", p: PATCH_ID, len: 2 }; m.cells["10:9"] = { k: "n", p: PATCH_ID, len: 2 };
  const ui = surface(m);
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const shiftClick = (key: string) => { pointer("pointerdown", ui.cell(key), { shiftKey: true }); pointer("pointerup", window, { shiftKey: true }); ui.cell(key).dispatchEvent(new w.MouseEvent("click", { bubbles: true, shiftKey: true })); flushSync(); };
  const doc = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
  const original = doc.elementFromPoint;
  try {
    ui.click('[data-score-cell="2:5"]'); shiftClick("6:7"); shiftClick("6:7");
    assert.equal(ui.cell("2:5").classList.contains("grp"), false, "no ring on the survivor: it is the ordinary selection");
    shiftClick("10:9");
    assert.ok(ui.cell("2:5").classList.contains("grp") && ui.cell("10:9").classList.contains("grp"), "the next pick joins the survivor, not a leftover");
    ui.cell("2:5").focus(); ui.cell("2:5").dispatchEvent(new w.KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); flushSync();
    assert.equal(ui.target.querySelector("[data-score-phrase]"), null, "one Escape ends the phrase");
    shiftClick("2:5");
    assert.ok(ui.target.querySelector("[data-score-phrase]"), "the selected note (10:9) and the shift-clicked one form a phrase again");
    // Dragging a note outside the phrase moves that note alone and ends the phrase.
    let under: Element | null = null; doc.elementFromPoint = () => under;
    pointer("pointerdown", ui.cell("6:7")); under = ui.cell("7:7"); pointer("pointermove", window); pointer("pointerup", window);
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 7, row: 7, value: { k: "n", p: PATCH_ID, len: 2 } }, { op: "clear_cell", step: 6, row: 7 }]);
    assert.equal(ui.target.querySelector("[data-score-phrase]"), null);
    ui.ops.length = 0;
    ui.cell("7:7").focus(); ui.cell("7:7").dispatchEvent(new w.KeyboardEvent("keydown", { key: "ArrowRight", altKey: true, bubbles: true, cancelable: true })); flushSync();
    assert.equal(ui.ops.length, 0, "the model is the test's object: the moved note's new key is empty, so a nudge has nothing to move and the old phrase is not nudged instead");
  } finally { doc.elementFromPoint = original; await ui.done(); }
});

test("a duplicate that can place nothing says so and sends nothing, not even a longer score", async () => {
  const m = model(); m.steps = 16; m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 2 }; m.cells["4:5"] = { k: "n", p: PATCH_ID, len: 12 };
  const ui = surface(m);
  try {
    ui.click('[data-score-cell="2:5"]'); ui.click('[aria-label="Duplicate this note"]');
    assert.deepEqual(ui.ops, []); assert.match(ui.notices.at(-1) ?? "", /nothing could be copied/);
  } finally { await ui.done(); }
});

test("zoom changes the steps per page from the footer and the keyboard, keeping the first visible step in view", async () => {
  const m = model(); m.steps = 64;
  const ui = surface(m);
  try {
    assert.equal(ui.target.querySelectorAll("[data-score-cell]").length, 25 * 16);
    ui.click("[data-score-zoom-in]");
    assert.equal(ui.target.querySelectorAll("[data-score-cell]").length, 25 * 8);
    assert.equal(ui.target.querySelector("[data-score-zoom]")?.getAttribute("data-score-zoom"), "8");
    assert.equal(ui.target.querySelector<HTMLButtonElement>("[data-score-zoom-in]")!.disabled, true, "eight is the closest");
    ui.click('[aria-label="Next score page"]'); ui.click('[aria-label="Next score page"]');
    assert.ok(ui.cell("16:0"), "page three of eight-step pages starts at step 17");
    ui.click("[data-score-zoom-out]"); ui.click("[data-score-zoom-out]");
    assert.equal(ui.target.querySelectorAll("[data-score-cell]").length, 25 * 32);
    assert.ok(ui.cell("16:0") && ui.cell("0:0"), "the step that was in view still is; the page is realigned to the new size");
    ui.cell("0:0").focus();
    ui.cell("0:0").dispatchEvent(new w.KeyboardEvent("keydown", { key: "-", bubbles: true, cancelable: true })); flushSync();
    assert.equal(ui.target.querySelectorAll("[data-score-cell]").length, 25 * 64);
    assert.equal(ui.target.querySelector<HTMLButtonElement>("[data-score-zoom-out]")!.disabled, true);
    ui.cell("0:0").dispatchEvent(new w.KeyboardEvent("keydown", { key: "=", bubbles: true, cancelable: true })); flushSync();
    assert.equal(ui.target.querySelectorAll("[data-score-cell]").length, 25 * 32);
  } finally { await ui.done(); }
});

test("tabbing into the grid never replaces a selection made elsewhere", async () => {
  const m = model(); m.steps = 16; m.cells["3:2"] = { k: "n", p: PATCH_ID, len: 2 };
  const ui = surface(m);
  try {
    ui.cell("0:24").focus(); flushSync();
    assert.ok(ui.target.querySelector("[data-score-selected]"), "with nothing selected, the focused cell becomes the selection");
    ui.click('[data-score-cell="3:2"]');
    ui.cell("5:10").focus(); flushSync();
    assert.ok(ui.target.querySelector('[data-score-selected] [aria-label="Sound of this note"]'), "the note stays selected when another cell merely receives focus");
  } finally { await ui.done(); }
});

test("the phrase panel transposes the phrase by the chosen interval as one gesture; a member that would leave the grid refuses the whole move", async () => {
  const m = model(); m.steps = 16; m.cells["2:5"] = { k: "n", p: PATCH_ID, len: 2 }; m.cells["4:7"] = { k: "n", p: PATCH_ID, len: 2 };
  const ui = surface(m, { apply: true });
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const shiftClick = (key: string) => { pointer("pointerdown", ui.cell(key), { shiftKey: true }); pointer("pointerup", window, { shiftKey: true }); ui.cell(key).dispatchEvent(new w.MouseEvent("click", { bubbles: true, shiftKey: true })); flushSync(); };
  try {
    ui.click('[data-score-cell="2:5"]');
    assert.equal(ui.target.querySelector("[data-score-transpose]"), null, "one note has the move buttons, not the interval chooser");
    shiftClick("4:7");
    assert.ok(ui.target.querySelector("[data-score-transpose]"), "a phrase offers an interval");
    ui.click('[aria-label="Transpose the phrase up"]');
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 4, row: 19, value: { k: "n", p: PATCH_ID, len: 2 } }, { op: "set_note", step: 2, row: 17, value: { k: "n", p: PATCH_ID, len: 2 } },
      { op: "clear_cell", step: 2, row: 5 }, { op: "clear_cell", step: 4, row: 7 }], "an octave by default, the highest note written first");
    ui.ops.length = 0;
    ui.click('[aria-label="Transpose the phrase up"]');
    assert.deepEqual(ui.ops, [], "another octave would leave the grid: nothing moves, nothing is clamped");
    assert.match(ui.notices.at(-1) ?? "", /off the grid/);
    ui.change('[aria-label="Interval"]', "7");
    ui.click('[aria-label="Transpose the phrase down"]');
    // The phrase now lives in its layer, so moving it again replaces those layered notes.
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 2, row: 10, value: { k: "n", p: PATCH_ID, len: 2 } }, { op: "set_note", step: 4, row: 12, value: { k: "n", p: PATCH_ID, len: 2 } },
      { op: "clear_note", step: 2, row: 17, layer: PATCH_ID }, { op: "clear_note", step: 4, row: 19, layer: PATCH_ID }], "a fifth down, the phrase still picked at its new place");
    assert.match(ui.target.querySelector("[data-score-phrase]")?.parentElement?.textContent ?? "", /2 notes/);
  } finally { await ui.done(); }
});

test("alt+page up/down transposes the selected note by the interval; a note that would leave the grid is refused, not clamped", async () => {
  const m = model(); m.steps = 16; m.cells["4:4"] = { k: "n", p: PATCH_ID, len: 2 }; m.cells["8:20"] = { k: "n", p: PATCH_ID, len: 1 };
  const ui = surface(m, { apply: true });
  const key = (cell: string, name: string) => { ui.cell(cell).focus(); ui.cell(cell).dispatchEvent(new w.KeyboardEvent("keydown", { key: name, altKey: true, bubbles: true, cancelable: true })); flushSync(); };
  try {
    ui.click('[data-score-cell="4:4"]'); key("4:4", "PageUp");
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 4, row: 16, value: { k: "n", p: PATCH_ID, len: 2 } }, { op: "clear_cell", step: 4, row: 4 }]);
    ui.ops.length = 0;
    ui.click('[data-score-cell="8:20"]'); key("8:20", "PageUp");
    assert.deepEqual(ui.ops, []); assert.match(ui.notices.at(-1) ?? "", /off the grid/);
    key("8:20", "PageDown");
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 8, row: 8, value: { k: "n", p: PATCH_ID, len: 1 } }, { op: "clear_cell", step: 8, row: 20 }]);
  } finally { await ui.done(); }
});

test("a remembered selection opens the composer on it, and every change is reported for the next mount", async () => {
  const m = model(); m.steps = 48; m.cells["20:5"] = { k: "n", p: PATCH_ID, len: 2 }; m.cells["22:7"] = { k: "n", p: PATCH_ID, len: 2 };
  const reported: ScoreSelection[] = [];
  const ui = surface(m, { selection: { selected: "20:5", picked: ["20:5", "22:7", "bad", "9:99"] }, onselection: (s) => reported.push(s) });
  try {
    assert.match(ui.target.querySelector("[data-score-phrase]")?.parentElement?.textContent ?? "", /2 notes/, "the phrase is back, without keys the composer could never have written");
    assert.ok(ui.cell("20:5"), "the page holding the selection is the one shown");
    assert.deepEqual(reported.at(-1), { selected: "20:5", picked: ["20:5", "22:7"] });
    ui.cell("20:5").focus(); ui.cell("20:5").dispatchEvent(new w.KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); flushSync();
    assert.deepEqual(reported.at(-1), { selected: "20:5", picked: [] }, "escape ends the phrase and the host hears of it");
    ui.click('[data-score-cell="22:7"]');
    assert.deepEqual(reported.at(-1), { selected: "22:7", picked: [] });
  } finally { await ui.done(); }
  const empty = surface(model(), { selection: { selected: "3:3", picked: [] } });
  try { assert.match(empty.target.querySelector("[data-score-selected]")?.textContent ?? "", /selected cell/, "an empty cell can be the remembered selection"); }
  finally { await empty.done(); }
});

test("the grid stays editable while the score plays: a placed note is written but not auditioned on its own, and the loop shows edits waiting for the next pass", async () => {
  const m = model(); m.steps = 16;
  const heard: number[] = [];
  const ui = surface(m, { apply: true, playing: true, positionMs: 0, loop: true, nextPass: true, onaudition: (_p, note) => heard.push(note) });
  try {
    assert.ok(ui.target.querySelector("[data-score-next-pass]"), "edits wait for the next pass");
    assert.equal(ui.target.querySelector(".st-banner.warn"), null, "playback is not a read-only reason");
    ui.click('[data-score-cell="3:2"]');
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 2, row: 2, value: { k: "n", p: PATCH_ID, len: 2 } }]);
    assert.deepEqual(heard, [], "the loop's next pass plays it; no audition cuts the loop");
    ui.click('[aria-label="Move this note up"]');
    assert.deepEqual(ui.ops.slice(-2), [{ op: "set_note", step: 2, row: 3, value: { k: "n", p: PATCH_ID, len: 2 } }, { op: "clear_note", step: 2, row: 2, layer: PATCH_ID }]);
    assert.deepEqual(heard, [], "nor does a move");
    ui.click('[aria-label="Listen to this note"]');
    assert.deepEqual(heard, [63], "an explicit listen still sounds");
  } finally { await ui.done(); }
});

// Review of e7250d48, LOW 1: notes playback had no voice for are counted, and shown, not hidden.
test("notes that did not sound for want of a voice are shown while the score plays, and only then", async () => {
  for (const [playing, refused, shown] of [[true, 2, "2 not played"], [true, 0, null], [false, 3, null]] as const) {
    const ui = surface(model(), { playing, refused });
    try {
      const chip = ui.target.querySelector("[data-score-refused]");
      assert.equal(chip?.textContent?.trim() ?? null, shown, `playing=${playing} refused=${refused}`);
      if (chip) assert.match(chip.getAttribute("title") ?? "", /2 did not sound/);
    } finally { await ui.done(); }
  }
});

// Owner decisions 2026-10-20: one note per sound per cell, and every sound (and the drum kit) can
// be shown, ghosted or hidden. Ghost is outlines only and never interactive, so other sounds can be
// written over it; only a shown sound takes new notes; the toggle is a symbol, never a word.
const OTHER = "c".repeat(64);
function layered(notes: Record<string, ScoreCell>): ScoreModel {
  const m = model(); m.patches[OTHER] = { ...PATCH, o: [{ w: 2, t: 0, c: 0, l: 80 }] }; m.patchSlots[OTHER] = slot(m.patches[OTHER]);
  m.notes = notes; // separate from the (legacy, empty) cells: these are layered notes
  return m;
}

test("each sound and the drum kit cycle show, ghost and hide with a symbol only; a ghosted sound is written over", async () => {
  const ui = surface(layered({ [`2:5/${OTHER}`]: { k: "n", p: OTHER, len: 2 } }));
  try {
    const toggle = () => ui.target.querySelector<HTMLButtonElement>(`[data-score-visibility="${OTHER}"]`)!;
    assert.ok(ui.target.querySelector('[data-score-visibility="d"]'), "the drum kit has its own toggle");
    assert.equal(toggle().textContent!.trim(), "", "a symbol, no words");
    assert.match(toggle().getAttribute("aria-label")!, /: shown$/);
    assert.ok(ui.cell("2:5").classList.contains("n"), "a shown sound's note is drawn and editable");
    ui.click(`[data-score-visibility="${OTHER}"]`);
    assert.match(toggle().getAttribute("aria-label")!, /: ghosted$/);
    assert.ok(ui.cell("2:5").classList.contains("ghost") && ui.cell("2:5").classList.contains("gon"), "an outline at the onset");
    assert.ok(ui.cell("3:5").classList.contains("gend"), "and the end of its hold");
    assert.equal(ui.cell("2:5").classList.contains("n"), false, "not a note to the grid");
    ui.click('[data-score-cell="2:5"]');
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 2, row: 5, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD } }], "the chosen sound is placed over the ghost, in its own layer");
    ui.click(`[data-score-visibility="${OTHER}"]`);
    assert.match(toggle().getAttribute("aria-label")!, /: hidden$/);
    assert.equal(ui.cell("3:5").classList.contains("ghost") || ui.cell("3:5").classList.contains("held"), false, "a hidden sound is not drawn");
    ui.click(`[data-score-visibility="${OTHER}"]`);
    assert.match(toggle().getAttribute("aria-label")!, /: shown$/, "and back to shown");
  } finally { await ui.done(); }
});

// Review L4 and L6: selection is kept by cell, so ghosting a sound must drop a selection on it (it
// would re-bind to another sound's note in that cell); and two members placing the same note is no
// conflict, so it carries no conflict mark.
test("ghosting a sound drops the selection on it; identical concurrent values are not marked as a conflict", async () => {
  const m = layered({ [`4:2/${OTHER}`]: { k: "n", p: OTHER, len: 1 }, [`8:3/${PATCH_ID}`]: { k: "n", p: PATCH_ID, len: 1 } });
  const source = (n: number) => ({ opId: n.toString(16).padStart(64, "0"), author: "e".repeat(64), nonce: n.toString(16).padStart(32, "0") });
  m.layerSlots = { [`8:3/${PATCH_ID}`]: { cleared: false, register: { selected: { value: { k: "n", p: PATCH_ID, len: 1 }, source: source(1) }, conflicts: [{ value: { k: "n", p: PATCH_ID, len: 1 }, source: source(2) }] } } };
  // Two members set the same tempo at once: one value, not a conflict.
  m.headers.bpm = { selected: { value: 120, source: source(3) }, conflicts: [{ value: 120, source: source(4) }] };
  assert.deepEqual(scoreAttention(m).map((item) => item.id), [], "nothing to choose between");
  const ui = surface(m);
  try {
    assert.equal(ui.cell("8:3").classList.contains("conf"), false, "the same note twice is one note");
    assert.equal(ui.target.querySelector(".sc-field.conf"), null, "no header conflict either");
    ui.click('[data-score-cell="4:2"]');
    assert.ok(ui.target.querySelector('[aria-label="Sound of this note"]'), "the other sound's note is selected");
    ui.click(`[data-score-visibility="${OTHER}"]`);
    assert.equal(ui.target.querySelector('[aria-label="Sound of this note"]'), null, "ghosting its sound drops that selection");
    assert.equal(ui.cell("4:2").classList.contains("sel"), false);
  } finally { await ui.done(); }
});

// Review of e7250d48, design question 1: a slot with several DIFFERENT live values plays them all
// and the grid shows one. Moving or rewriting it would drop the versions the member cannot see, so
// it is refused; an erase (clears alone) or an explicit choice among the versions is the way out.
const severalSource = (n: number) => ({ opId: n.toString(16).padStart(64, "0"), author: "e".repeat(64), nonce: n.toString(16).padStart(32, "0") });
function severalValues(second: ScoreCell = { k: "n", p: PATCH_ID, len: 3 }): ScoreModel {
  const key = `4:2/${PATCH_ID}`, m = layered({ [key]: { k: "n", p: PATCH_ID, len: 1 } });
  m.layerSlots = { [key]: { cleared: false, register: { selected: { value: { k: "n", p: PATCH_ID, len: 1 }, source: severalSource(1) }, conflicts: [{ value: second, source: severalSource(2) }] } } };
  const distinct = canonicalJson(second) !== canonicalJson({ k: "n", p: PATCH_ID, len: 1 });
  m.readiness.layers = { [key]: { count: false, extent: false, recipe: false, overlap: distinct, conflict: distinct } };
  return m;
}

test("the keys a gesture would move or rewrite while they hold several different live values", () => {
  const m = severalValues(), at = { step: 4, row: 2 }, key = `4:2/${PATCH_ID}`;
  const clear: ScoreOp = { op: "clear_note", ...at, layer: PATCH_ID };
  assert.deepEqual(scoreOpSeveralValues(m, [{ op: "set_note", ...at, value: { k: "n", p: PATCH_ID, len: 2 } }]), [key], "a rewrite in place");
  assert.deepEqual(scoreOpSeveralValues(m, [{ op: "set_note", step: 4, row: 3, value: { k: "n", p: PATCH_ID, len: 1 } }, clear], true), [key], "a move away, even from the grid");
  assert.deepEqual(scoreOpSeveralValues(m, [clear], true), [], "the member's own erase removes every version by intent");
  // Re-review M-A: an undo's clears remove the member's own edit, not versions added since.
  assert.deepEqual(scoreOpSeveralValues(m, [clear]), [key], "clears alone that are not an erase");
  assert.deepEqual(scoreOpSeveralValues(m, [{ op: "set_note", ...at, value: { k: "n", p: PATCH_ID, len: 3 } }], false, key), [], "the review list resolving that key");
  // Re-review M-B: resolving a legacy cell does not exempt the layered slot its choice writes.
  assert.deepEqual(scoreOpSeveralValues(m, [{ op: "set_note", ...at, value: { k: "n", p: PATCH_ID, vol: 50 } }, { op: "clear_cell", ...at }], false, "4:2"), [key],
    "only the reviewed key is exempt");
  assert.deepEqual(scoreOpSeveralValues(m, [{ op: "set_note", ...at, value: { k: "n", p: OTHER, len: 1 } }]), [], "another sound's layer in the same cell");
  assert.deepEqual(scoreOpSeveralValues(severalValues({ k: "n", p: PATCH_ID, len: 1 }), [{ op: "set_note", ...at, value: { k: "n", p: PATCH_ID, len: 2 } }]), [], "the same note twice is one note");
  // Re-review L-A: the guard reads readiness, which the working projection recomputes for a queued
  // choice; the raw slots still hold both values until readback.
  const chosen = severalValues(); chosen.readiness.layers = {};
  assert.deepEqual(scoreOpSeveralValues(chosen, [{ op: "set_note", step: 4, row: 3, value: { k: "n", p: PATCH_ID, len: 1 } }, clear]), [], "a version chosen and still saving is the only one");
});

test("the working projection of a queued choice leaves one version, so the grid may edit the note again", () => {
  const key = `4:2/${PATCH_ID}`, base = severalValues();
  const choose = { kind: "apply", object: "x", op: { op: "set_note", step: 4, row: 2, value: { k: "n", p: PATCH_ID, len: 3 } }, label: "choose", id: 1, status: "queued",
    attempts: 0, error: "", epochChanged: false, mayHaveCommitted: false, request: { object: "x", epochId: "y", nonce: "z", body: "" } };
  const working = workingScore(base, [choose as unknown as Parameters<typeof workingScore>[1][number]]);
  assert.equal(base.readiness.layers?.[key]?.conflict, true);
  assert.deepEqual(scoreOpSeveralValues(working, [{ op: "set_note", step: 4, row: 3, value: { k: "n", p: PATCH_ID, len: 3 } }, { op: "clear_note", step: 4, row: 2, layer: PATCH_ID }]), []);
});

test("a note with several different versions is never moved or rewritten: erase it, or choose a version first", async () => {
  const ui = surface(severalValues());
  try {
    ui.click('[data-score-cell="4:2"]');
    ui.click('[aria-label="Move this note up"]');
    assert.deepEqual(ui.ops, [], "a move would drop the version that is not shown");
    assert.match(ui.notices.at(-1) ?? "", /More than one version of this note plays/);
    ui.click('[data-score-alt="1"]');
    assert.equal(ui.ops.length, 1, "choosing a version is how it is resolved");
    assert.deepEqual(ui.ops[0], { op: "set_note", step: 4, row: 2, value: (ui.ops[0] as { value: ScoreCell }).value });
  } finally { await ui.done(); }
  const erase = surface(severalValues());
  try {
    erase.click('[data-score-cell="4:2"]');
    erase.click('[aria-label="Clear this note"]');
    assert.deepEqual(erase.ops, [{ op: "clear_note", step: 4, row: 2, layer: PATCH_ID }], "an erase removes the note in every version");
  } finally { await erase.done(); }
  const same = surface(severalValues({ k: "n", p: PATCH_ID, len: 1 }));
  try {
    same.click('[data-score-cell="4:2"]');
    same.click('[aria-label="Move this note up"]');
    assert.deepEqual(same.ops, [{ op: "set_note", step: 4, row: 3, value: { k: "n", p: PATCH_ID, len: 1 } }, { op: "clear_note", step: 4, row: 2, layer: PATCH_ID }], "two members' identical note moves as one");
  } finally { await same.done(); }
});

// Re-review M-A: an undo whose operations are all clears took the erase exemption and removed a
// version a collaborator added meanwhile, which the member never saw.
test("undoing a placement is refused once a collaborator's different version shares the slot, and stays on the stack", async () => {
  const m = layered({}), ui = surface(m, { apply: true }), key = `4:1/${PATCH_ID}`;
  try {
    ui.click('[data-score-cell="4:1"]');
    assert.equal(ui.ops.length, 1);
    const mine = (ui.ops[0] as { value: ScoreCell }).value;
    m.layerSlots = { [key]: { cleared: false, register: { selected: { value: mine, source: severalSource(1) }, conflicts: [{ value: { k: "n", p: PATCH_ID, len: 5, vol: 20 }, source: severalSource(2) }] } } };
    m.readiness = { ...m.readiness, layers: { [key]: { count: false, extent: false, recipe: false, overlap: true, conflict: true } } };
    ui.click("[data-score-undo]");
    assert.equal(ui.ops.length, 1, "the undo's clear would have removed the collaborator's version too");
    assert.match(ui.notices.at(-1) ?? "", /More than one version of this note plays/);
    assert.equal(ui.target.querySelector<HTMLButtonElement>("[data-score-undo]")!.disabled, false, "the history entry stays");
  } finally { await ui.done(); }
});

// Re-review M-B: `resolving` exempted every key of the gesture. Choosing a removed legacy cell's
// retained value writes its layered key, which here holds two versions nobody chose between.
test("choosing a legacy cell's retained value never overwrites a layered slot with several versions", async () => {
  for (const several of [true, false]) {
    const key = `0:3/${PATCH_ID}`, A: ScoreCell = { k: "n", p: PATCH_ID, len: 2 }, B: ScoreCell = { k: "n", p: PATCH_ID, len: 4, vol: 30 };
    const m = layered({ [key]: A });
    m.cellSlots["0:3"] = slot<ScoreCell>(null, [{ k: "n", p: PATCH_ID, vol: 50 }]); // removed, with a retained value to review
    m.layerSlots = { [key]: { cleared: false, register: { selected: { value: A, source: severalSource(1) }, conflicts: several ? [{ value: B, source: severalSource(2) }] : [] } } };
    m.readiness.layers = { [key]: { count: false, extent: false, recipe: false, overlap: several, conflict: several } };
    const ui = surface(m);
    try {
      ui.click('[data-score-tab="score"]'); ui.click('[data-score-evidence="cell/0:3"]'); ui.click('[data-score-alt="1"]');
      if (several) {
        assert.deepEqual(ui.ops, [], "both layered versions stay");
        assert.match(ui.notices.at(-1) ?? "", /More than one version of this note plays/);
      } else assert.deepEqual(ui.ops.map((op) => (op as ScoreOp).op), ["set_note", "clear_cell"], "with one version the choice goes through");
    } finally { await ui.done(); }
  }
});

// Re-review L-B: undoing the one-press Shorten (or an erase) of a hold past the end replayed a
// length that does not fit and was refused, which also blocked every older undo.
test("a Shorten or an erase of a hold past the end is undone exactly; a hold that fitted when recorded is still refused", async () => {
  const overrun = () => { const m = model(); m.steps = 4; m.cells["2:0"] = { k: "n", p: PATCH_ID, len: 8 }; return m; };
  const shorten = surface(overrun(), { apply: true });
  try {
    shorten.click('[data-score-cell="2:0"]'); shorten.click('[aria-label="Shorten this note"]');
    assert.equal(shorten.ops.length, 2);
    shorten.click("[data-score-undo]");
    assert.deepEqual(shorten.ops.slice(2), [{ op: "set_cell", step: 2, row: 0, value: { k: "n", p: PATCH_ID, len: 8 } }, { op: "clear_note", step: 2, row: 0, layer: PATCH_ID }],
      "the overrun comes back as it was");
    shorten.click("[data-score-redo]");
    assert.deepEqual(shorten.ops.slice(4), [{ op: "set_note", step: 2, row: 0, value: { k: "n", p: PATCH_ID, len: 2 } }, { op: "clear_cell", step: 2, row: 0 }], "and the redo shortens it again");
    shorten.click("[data-score-undo]");
    assert.equal(shorten.ops.length, 8, "the redo's own undo restores the overrun too");
  } finally { await shorten.done(); }
  const erase = surface(overrun(), { apply: true });
  try {
    erase.click('[data-score-cell="2:0"]'); erase.click('[aria-label="Clear this note"]');
    erase.click("[data-score-undo]");
    assert.deepEqual(erase.ops.at(-1), { op: "set_cell", step: 2, row: 0, value: { k: "n", p: PATCH_ID, len: 8 } });
  } finally { await erase.done(); }
  // The rule this relaxes only for an overrun the member had: a hold that fitted when its gesture
  // was sent, and no longer does because the score was shortened since, still is not put back.
  const m = model(); m.cells["2:0"] = { k: "n", p: PATCH_ID, len: 8 };
  const shrunk = surface(m, { apply: true });
  try {
    shrunk.click('[data-score-cell="2:0"]'); shrunk.click('[aria-label="Clear this note"]');
    m.steps = 4; shrunk.click("[data-score-undo]");
    assert.equal(shrunk.ops.length, 1, "refused"); assert.match(shrunk.notices.at(-1) ?? "", /must fit/);
  } finally { await shrunk.done(); }
});

// Re-review LOW-1: the past-the-end mark was one per gesture, so undoing a phrase clear also put
// back a sibling note that fitted when it was recorded and no longer did.
test("undoing a phrase clear may put back only the note that already ran past the end", async () => {
  for (const shrink of [false, true]) {
    const m = model(); m.steps = 8; m.cells["2:0"] = { k: "n", p: PATCH_ID, len: 8 }; m.cells["4:3"] = { k: "n", p: PATCH_ID, len: 4 };
    const ui = surface(m, { apply: true });
    const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
    const shiftClick = (key: string) => { pointer("pointerdown", ui.cell(key), { shiftKey: true }); pointer("pointerup", window, { shiftKey: true }); ui.cell(key).dispatchEvent(new w.MouseEvent("click", { bubbles: true, shiftKey: true })); flushSync(); };
    try {
      ui.click('[data-score-cell="2:0"]'); shiftClick("4:3");
      ui.click('[aria-label="Clear the phrase"]');
      assert.equal(ui.ops.length, 2, `shrink=${shrink}`);
      assert.ok(!ui.notices.some((n) => /undo cannot restore/.test(n)), `shrink=${shrink}: the clear's own check agrees the undo can restore both`);
      if (shrink) m.steps = 6; // a collaborator shortens the score: the hold at 4 no longer fits
      ui.click("[data-score-undo]");
      if (shrink) {
        assert.equal(ui.ops.length, 2, "the sibling that fitted when recorded is not put back past the end");
        assert.match(ui.notices.at(-1) ?? "", /must fit/);
      } else {
        assert.deepEqual(ui.ops.slice(2).map((op) => (op as { step: number }).step).sort(), [2, 4], "both come back, the overrun one included");
      }
    } finally { await ui.done(); }
  }
});

// Review of e7250d48, design question 2: a hold past the end plays clipped, and no edit shortens it
// on its own, so its authored length survives a score that is shortened and grown back. Shorten is
// the explicit step, and one press brings the hold to the end (one step at a time was refused).
test("a hold past the end is never shortened on its own: other edits are refused, Shorten brings it to the end", async () => {
  const m = model(); m.steps = 4; m.cells["2:0"] = { k: "n", p: PATCH_ID, len: 8 };
  const ui = surface(m);
  try {
    ui.click('[data-score-cell="2:0"]');
    ui.click('[aria-label="Move this note up"]');
    assert.deepEqual(ui.ops, [], "a move keeps the length, so it is refused");
    assert.match(ui.notices.at(-1) ?? "", /past the end/);
    const volume = ui.target.querySelector<HTMLInputElement>('[aria-label="Volume of this note"]')!;
    volume.value = "40"; volume.dispatchEvent(new w.Event("change", { bubbles: true })); flushSync();
    assert.deepEqual(ui.ops, [], "and so is a volume change");
    ui.click('[aria-label="Shorten this note"]');
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 2, row: 0, value: { k: "n", p: PATCH_ID, len: 2 } }, { op: "clear_cell", step: 2, row: 0 }],
      "one press shortens it to the end, written into its layer");
  } finally { await ui.done(); }
});

// Third review L2: the playing voices are clipped to the end, so "fit to last note" read 15 for a
// hold from 14 to 21 and was disabled; it must still offer to cover the hold.
test("fit to last note still covers a hold that crosses the end", async () => {
  const m = model(); m.steps = 16; m.cells["14:0"] = { k: "n", p: PATCH_ID, len: 8 };
  const ui = surface(m);
  try {
    ui.click('[data-score-tab="score"]');
    const fit = ui.button("fit to last note");
    assert.equal(fit.disabled, false);
    fit.click(); flushSync();
    assert.deepEqual(ui.ops.at(-1), { op: "set_header", field: "steps", value: 22 });
  } finally { await ui.done(); }
});

// Review of e7250d48, MEDIUM 2: undo, redo and the attention list's actions carry document keys and
// skipped the surface, so they wrote into ghosted and hidden sounds.
test("undo, redo and attention actions never write a ghosted or hidden sound's notes, and keep their history", async () => {
  const m = layered({}), source = (n: number) => ({ opId: n.toString(16).padStart(64, "0"), author: "e".repeat(64), nonce: n.toString(16).padStart(32, "0") });
  const ui = surface(m, { apply: true });
  const key = (k: string, init: KeyboardEventInit = {}) => { ui.cell("0:0").focus(); ui.cell("0:0").dispatchEvent(new w.KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init })); flushSync(); };
  try {
    ui.click('[data-score-cell="4:1"]');
    assert.equal(ui.ops.length, 1);
    ui.click(`[data-score-visibility="${PATCH_ID}"]`); // ghost
    key("z", { ctrlKey: true });
    assert.equal(ui.ops.length, 1, "undo did not clear the ghosted note"); assert.match(ui.notices.at(-1) ?? "", /Show that sound/);
    assert.equal(ui.target.querySelector<HTMLButtonElement>("[data-score-undo]")!.disabled, false, "the history entry stays");
    ui.click(`[data-score-visibility="${PATCH_ID}"]`); ui.click(`[data-score-visibility="${PATCH_ID}"]`); // hide, then show
    key("z", { ctrlKey: true });
    assert.deepEqual(ui.ops.at(-1), { op: "clear_note", step: 4, row: 1, layer: PATCH_ID }, "shown again, the undo runs");
    ui.click(`[data-score-visibility="${PATCH_ID}"]`); ui.click(`[data-score-visibility="${PATCH_ID}"]`); // ghost, hide
    key("z", { ctrlKey: true, shiftKey: true });
    assert.equal(ui.ops.length, 2, "redo did not write into the hidden sound");
  } finally { await ui.done(); }
  // An attention row expanded while the sound was shown cannot clear it once it is ghosted.
  const flagged = layered({ [`2:0/${PATCH_ID}`]: { k: "n", p: PATCH_ID, len: 1 } });
  flagged.layerSlots = { [`2:0/${PATCH_ID}`]: { cleared: false, register: { selected: { value: { k: "n", p: PATCH_ID, len: 1 }, source: source(1) }, conflicts: [] } } };
  flagged.readiness.layers = { [`2:0/${PATCH_ID}`]: { count: false, extent: false, recipe: false, overlap: true, conflict: false } };
  const review = surface(flagged);
  try {
    review.click('[data-score-tab="score"]'); review.click(`[data-score-evidence="note/2:0/${PATCH_ID}"]`);
    review.click('[data-score-tab="sounds"]'); review.click(`[data-score-visibility="${PATCH_ID}"]`); review.click('[data-score-tab="score"]');
    review.button("clear").click(); flushSync();
    assert.deepEqual(review.ops, [], "the ghosted note was not cleared"); assert.match(review.notices.at(-1) ?? "", /Show that sound/);
  } finally { await review.done(); }
});

test("only a shown sound takes new notes: ghosting or hiding the chosen sound turns placement off", async () => {
  const ui = surface(layered({}));
  try {
    for (const state of ["ghosted", "hidden"]) {
      ui.click(`[data-score-visibility="${PATCH_ID}"]`);
      assert.match(ui.target.querySelector(`[data-score-visibility="${PATCH_ID}"]`)!.getAttribute("aria-label")!, new RegExp(state));
      assert.ok(ui.cell("4:4").classList.contains("off"), `${state}: no placement`);
      ui.click('[data-score-cell="4:4"]');
      assert.deepEqual(ui.ops, []);
    }
    ui.click('[data-score-mode="drum"]');
    assert.equal(ui.cell("4:4").classList.contains("off"), false, "the drum kit is its own layer, still shown");
    ui.click('[data-score-visibility="d"]');
    assert.ok(ui.cell("4:4").classList.contains("off"), "a ghosted kit takes no hits");
  } finally { await ui.done(); }
});

test("a cell holding two sounds shows the chosen one, marks the other, and selecting a note picks up its sound", async () => {
  const ui = surface(layered({ [`4:2/${PATCH_ID}`]: { k: "n", p: PATCH_ID, len: 1 }, [`4:2/${OTHER}`]: { k: "n", p: OTHER, len: 1 }, [`8:2/${OTHER}`]: { k: "n", p: OTHER, len: 1 } }));
  try {
    assert.ok(ui.cell("4:2").classList.contains("stacked"), "a second sound is behind the shown one");
    assert.match(ui.cell("4:2").getAttribute("title")!, /more sounds here/);
    assert.match(ui.cell("4:2").getAttribute("aria-label")!, /SYNTH/, "the chosen sound's note is the one shown");
    ui.click('[data-score-cell="8:2"]');
    assert.ok(ui.target.querySelector(`[data-doc-patch="${OTHER}"]`)!.closest(".sc-prow")!.classList.contains("on"), "selecting the other sound's note chose it");
    assert.match(ui.cell("4:2").getAttribute("aria-label")!, /Square sound/, "so the shared cell now shows that sound's note");
    ui.click('[data-score-cell="12:6"]');
    assert.deepEqual(ui.ops, [{ op: "set_note", step: 12, row: 6, value: { k: "n", p: OTHER, len: SCORE_DEFAULT_HOLD } }], "the next placement uses the sound just picked up");
    ui.click('[aria-label="Erase"]'); ui.click('[data-score-cell="4:2"]');
    assert.deepEqual(ui.ops.at(-1), { op: "clear_note", step: 4, row: 2, layer: OTHER }, "erase removes the chosen sound's note, never the other one in that cell");
  } finally { await ui.done(); }
});
