import assert from "node:assert/strict";
import test from "node:test";
import type { JamPatch } from "./jam-contract.ts";
import type { ScoreModel, ScoreSlot, Valued, IndexSource } from "./studio-native.ts";
import type { ScoreCell } from "./studio-contract.ts";
import {
  PAD_CODES, PAD_NAMES, SCORE_DEFAULT_HOLD, SCORE_PAGE_STEPS, SCORE_WIDE_PAGE_STEPS, scoreAttention, scoreBarBeat, scoreCellOperation, scoreContinuations,
  scoreDiagnosticOperation, scoreDiagnostics, scoreDurationMs, scoreExtent, scoreHeaderOperation, scoreHolds, scoreLastSoundingStep,
  scoreNotesAt, scoreOccupiedSteps, scoreOnsetAt, scorePadForRow, scorePage, scorePatchOrder, scorePatchUsage, scorePitch, scorePlayheadStep,
  scoreRowIsBlack, scoreScaleRows, scoreSlotDiagnostic, scoreSnapStart, scoreTapTempo, scoreTimeLabel,
} from "./score-editor.ts";

export const PATCH_ID = "a".repeat(64);
export const PATCH: JamPatch = { v: 1, o: [{ w: 0, t: 0, c: 0, l: 80 }], e: { a: 10, d: 40, s: 70, r: 120 }, f: { m: 0, c: 18000, q: 0, e: 0 }, l: { r: 1, d: 0, t: 0 }, x: { c: 0, d: 0, r: 0 } };
export function alternative<T>(value: T, n = 1): Valued<T, IndexSource> { return { value, source: { opId: n.toString(16).padStart(64, "0"), author: "b".repeat(64), nonce: n.toString(16).padStart(32, "0") } }; }
export function slot<T>(value: T | null, conflicts: (T | null)[] = []): ScoreSlot<T> { return { removed: [value, ...conflicts].includes(null), register: { selected: alternative(value), conflicts: conflicts.map((c, i) => alternative(c, i + 2)) } }; }
export function model(): ScoreModel {
  return { title: "", bpm: 120, bpb: 4, steps: 16, cells: {}, patches: { [PATCH_ID]: PATCH }, cellSlots: {}, patchSlots: { [PATCH_ID]: slot(PATCH) }, headers: { title: null, bpm: null, bpb: null, steps: null }, readiness: { cells: {}, patches: {}, headerConflicts: false }, ready: true };
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
  assert.deepEqual(scoreCellOperation(m, 0, 24, { mode: "note", patch: PATCH_ID, len: 16 }), { op: "set_cell", step: 0, row: 24, value: { k: "n", p: PATCH_ID, len: 16 } });
  assert.deepEqual(scoreCellOperation(m, 15, 0, { mode: "note", patch: PATCH_ID, len: 1 }), { op: "set_cell", step: 15, row: 0, value: { k: "n", p: PATCH_ID, len: 1 } });
  for (const len of [0, 2, 1.5, NaN]) assert.throws(() => scoreCellOperation(m, 15, 0, { mode: "note", patch: PATCH_ID, len }));
  delete m.patches[PATCH_ID];
  assert.throws(() => scoreCellOperation(m, 0, 0, { mode: "note", patch: PATCH_ID, len: 1 }), /active/);
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
  assert.deepEqual(scoreDiagnosticOperation(m, cell, 1), { op: "clear_cell", step: 255, row: 24 });
  assert.throws(() => scoreDiagnosticOperation(m, cell, 0), /authored/);
  assert.deepEqual(scoreDiagnosticOperation(m, recipe, 1), { op: "remove_patch", patch: PATCH_ID });
  assert.deepEqual(scoreDiagnosticOperation(m, evidence[0], 1), { op: "set_header", field: "bpm", value: 140 });
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
function surface(m: ScoreModel, options: { readonlyReason?: string; loading?: boolean; onaddpatch?: () => Promise<string>; onaudition?: (patch: JamPatch, note: number, durationMs: number, level?: number) => void; playing?: boolean; positionMs?: number | null; loop?: boolean; onpost?: () => void } = {}) {
  const target = document.createElement("div"); document.body.appendChild(target);
  const ops: unknown[] = [], notices: string[] = [], loops: boolean[] = [];
  const instance = mount(ScoreEditor, { target, props: {
    model: m, readonlyReason: options.readonlyReason ?? "", loading: options.loading ?? false, error: "", patchLibrary: [{ name: "SYNTH", patch: PATCH }],
    library: [{ name: "SYNTH", patch: PATCH, shelf: "preset" }, { name: "OTHER", patch: { ...PATCH, o: [{ w: 1, t: 0, c: 0, l: 80 }] }, shelf: "mine" }],
    onop: (op: unknown) => { ops.push(op); }, onnotice: (text: string) => { notices.push(text); }, onaddpatch: options.onaddpatch ?? (async () => PATCH_ID),
    onaudition: options.onaudition, playing: options.playing ?? false, positionMs: options.positionMs ?? null, loop: options.loop ?? false, onloop: (v: boolean) => { loops.push(v); },
    onpost: options.onpost,
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
    ui.input('[aria-label="Note hold in sixteenth steps"]', "4");
    ui.click('[data-score-cell="4:1"]');
    assert.deepEqual(heard, [[61, 500]], "placing a note auditions it for its hold at the authored tempo");
    ui.click('[aria-label="Erase"]'); ui.click('[data-score-cell="1:0"]');
    ui.click('[aria-label="Place"]'); ui.click('[data-score-mode="drum"]'); ui.click('[data-score-pad="9"]'); ui.click('[data-score-cell="15:24"]');
    ui.change('[aria-label="Score BPM"]', "240");
    assert.deepEqual(ui.ops, [
      { op: "set_cell", step: 4, row: 1, value: { k: "n", p: PATCH_ID, len: 4 } },
      { op: "clear_cell", step: 0, row: 0 }, { op: "set_cell", step: 15, row: 24, value: { k: "d", pad: 9 } },
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
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 4, row: 0, value: { k: "n", p: PATCH_ID, len: 4 } });
    ui.click('[aria-label="Clear this note"]');
    assert.deepEqual(ui.ops.at(-1), { op: "clear_cell", step: 4, row: 0 });
    ui.cell("6:0").dispatchEvent(new w.MouseEvent("contextmenu", { bubbles: true, cancelable: true })); flushSync();
    assert.deepEqual(ui.ops.at(-1), { op: "clear_cell", step: 4, row: 0 }); assert.equal(ui.ops.length, 3);
  } finally { await ui.done(); }
});

test("compiled composer refuses missing recipes, invalid holds and read-only writes while preserving cleanup", async () => {
  const m = model(); m.patches = {}; m.patchSlots = {};
  let ui = surface(m);
  try {
    assert.ok(ui.cell("0:0").classList.contains("off")); ui.click('[data-score-cell="0:0"]'); assert.equal(ui.ops.length, 0);
    assert.match(ui.target.textContent!, /no recipes yet/);
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
    ui.input('[aria-label="Note hold in sixteenth steps"]', "2");
    ui.click('[data-score-cell="15:0"]');
    assert.deepEqual(ui.ops, [{ op: "set_cell", step: 15, row: 0, value: { k: "n", p: PATCH_ID, len: 1 } }]);
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
    ui.input('[aria-label="Note hold in sixteenth steps"]', "1");
    click("3:2");
    assert.deepEqual(ui.ops, [{ op: "set_cell", step: 2, row: 2, value: { k: "n", p: PATCH_ID, len: 1 } }], "an eighth snap moves the start; the hold stays one step");
    assert.match(ui.target.querySelector(".micro.hi")!.textContent!, /sixteenth/, "the click did not rewrite the hold");
    ui.input('[aria-label="Note hold in sixteenth steps"]', "2"); ui.click('[data-score-snap="4"]');
    click("7:4");
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 7, row: 4, value: { k: "n", p: PATCH_ID, len: 2 } }, "the snapped run 4..7 crosses the note at 5, so the exact cell is meant");
    ui.cell("6:6").dispatchEvent(new w.PointerEvent("pointerover", { bubbles: true })); flushSync();
    assert.ok(ui.cell("6:6").classList.contains("target") && !ui.cell("7:6").classList.contains("target"), "the preview shows the one free cell between two notes");
    click("6:6");
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 6, row: 6, value: { k: "n", p: PATCH_ID, len: 1 } }, "fitted to the gap, never an overlap refusal");
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
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 3, row: 1, value: { k: "d", pad: 1 } });
    assert.ok(ui.cell("3:13").classList.contains("fold"), "C♯5 is outside the scale and outside the lane, so it still folds");
  } finally { await ui.done(); }
});

test("pointer jitter inside the pressed cell is a click: the note gets the previewed hold and the hold is unchanged", async () => {
  const m = model(); m.steps = 16;
  const ui = surface(m);
  const pointer = (type: string, target: EventTarget, init: PointerEventInit = {}) => { target.dispatchEvent(new w.PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init })); flushSync(); };
  const doc = document as unknown as { elementFromPoint: (x: number, y: number) => Element | null };
  const original = doc.elementFromPoint;
  try {
    let under: Element | null = null;
    doc.elementFromPoint = () => under;
    for (const hold of [2, 4]) {
      ui.input('[aria-label="Note hold in sixteenth steps"]', String(hold));
      const row = hold; // a fresh row per hold
      pointer("pointerdown", ui.cell(`2:${row}`)); under = ui.cell(`2:${row}`); pointer("pointermove", window); pointer("pointermove", window);
      pointer("pointerup", window); ui.cell(`2:${row}`).click(); flushSync();
      assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 2, row, value: { k: "n", p: PATCH_ID, len: hold } }, `a jittered click writes the previewed ${hold}-step hold`);
      assert.match(ui.target.querySelector(".micro.hi")!.textContent!, hold === 2 ? /eighth/ : /quarter/, "the hold is still the chosen one");
      // Moving back inside the preview after lengthening never shortens below the preview.
      pointer("pointerdown", ui.cell(`8:${row}`)); under = ui.cell(`12:${row}`); pointer("pointermove", window); under = ui.cell(`9:${row}`); pointer("pointermove", window);
      pointer("pointerup", window);
      assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 8, row, value: { k: "n", p: PATCH_ID, len: hold } });
    }
    assert.equal(ui.ops.length, 4);
    // A preview forced shorter than the hold by a neighbour: drifting one cell inside it writes
    // the preview and leaves the hold alone; only lengthening beyond the preview changes it.
    m.cells["4:7"] = { k: "n", p: PATCH_ID, len: 1 };
    ui.input('[aria-label="Note hold in sixteenth steps"]', "4");
    pointer("pointerdown", ui.cell("2:7")); under = ui.cell("3:7"); pointer("pointermove", window); pointer("pointerup", window);
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 2, row: 7, value: { k: "n", p: PATCH_ID, len: 2 } });
    assert.match(ui.target.querySelector(".micro.hi")!.textContent!, /quarter/, "a forced fit plus drift is not a chosen length");
    pointer("pointerdown", ui.cell("8:7")); under = ui.cell("13:7"); pointer("pointermove", window); under = ui.cell("8:7"); pointer("pointermove", window);
    assert.ok(!ui.cell("13:7").classList.contains("drag") && ui.cell("11:7").classList.contains("drag"), "returning to the pressed cell returns to the preview, even past skipped cells");
    pointer("pointerup", window);
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 8, row: 7, value: { k: "n", p: PATCH_ID, len: 4 } });
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
    assert.deepEqual(ui.ops, [{ op: "set_cell", step: 4, row: 5, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD } }], "a genuinely new press places as usual");
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
  const ui = surface(m);
  try {
    ui.click('[data-score-cell="5:2"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 4, row: 2, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD } }, "an eighth snap lands the click on the even step for the default eighth hold");
    ui.click('[data-score-snap="4"]'); ui.click('[data-score-cell="7:3"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 4, row: 3, value: { k: "n", p: PATCH_ID, len: 2 } });
    m.cells["8:4"] = { k: "n", p: PATCH_ID, len: 1 };
    ui.click('[data-score-snap="2"]'); ui.click('[data-score-cell="9:4"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 9, row: 4, value: { k: "n", p: PATCH_ID, len: 2 } }, "an occupied snap target means the exact cell was meant");
    ui.input('[aria-label="Note hold in sixteenth steps"]', "4"); ui.click('[data-score-cell="12:5"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 12, row: 5, value: { k: "n", p: PATCH_ID, len: 4 } });
    assert.match(ui.target.querySelector(".sc-foot")!.textContent!, /quarter/, "the footer names the remembered hold");
    ui.click('[data-score-mode="drum"]'); ui.click('[data-score-pad="9"]');
    ui.click('[data-score-cell="3:3"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 3, row: 3, value: { k: "d", pad: 3 } }, "rows C4..A4 are the pad lane: the row decides the pad");
    assert.equal(ui.target.querySelector(".sc-lab.lane")!.textContent, "CR"); assert.match(ui.target.querySelector(".sc-lab.lane")!.getAttribute("title")!, /crash/);
    ui.click('[data-score-cell="3:20"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 3, row: 20, value: { k: "d", pad: 9 } }, "elsewhere the chosen pad is placed");
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
    ui.click('[aria-label="Audition"]'); ui.click('[data-score-cell="4:0"]');
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
    assert.deepEqual(ui.ops, [{ op: "set_cell", step: 2, row: 5, value: { k: "n", p: PATCH_ID, len: 5 } }]);
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
    const arrived = { ...model(), steps: 16, patches: { ...model().patches, [other]: PATCH }, cells: { "2:5": { k: "n", p: other, len: 1 } as ScoreCell } };
    (harness as unknown as { update: (next: ScoreModel) => void }).update(arrived); flushSync();
    pointer("pointerup", window);
    assert.deepEqual(ops, [], "the newcomer's note is not overwritten");
    assert.match(notices[0], /another edit/);
    assert.ok(cell("2:5").classList.contains("n"), "and it is what the grid now shows");
  } finally { doc.elementFromPoint = original; await unmount(harness); target.remove(); }
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
    assert.deepEqual(ui.ops, [{ op: "set_cell", step: 2, row: 5, value: { k: "n", p: PATCH_ID, len: 6, vol: 60 } }], "a drag-resize keeps the note's level");
    assert.match(ui.target.querySelector(".micro.hi")!.textContent!, /6 \/ 16$/, "the resized length is the new remembered hold");
    pointer("pointerdown", ui.cell("4:5")); pointer("pointerup", window);
    assert.equal(ui.ops.length, 1, "a release without movement is not a resize");
    ui.click('[data-score-cell="3:5"]');
    ui.change('[aria-label="Recipe for this note"]', other);
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 2, row: 5, value: { k: "n", p: other, len: 3, vol: 60 } });
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
    assert.match(ui.target.querySelector(".sc-head")!.textContent!, /1 to fix/);
  } finally { await ui.done(); }
});

test("compiled attention list exposes removed losing values, re-authors a chosen value and clears a shortened score's cells", async () => {
  const m = model(); m.steps = 1; m.ready = false;
  m.cellSlots["255:24"] = slot<ScoreCell>({ k: "n", p: PATCH_ID, len: 1 }, [null]);
  m.readiness.cells["255:24"] = { count: true, extent: true, recipe: false, overlap: false, conflict: true };
  const ui = surface(m);
  try {
    ui.click('[data-score-tab="score"]');
    assert.match(ui.target.textContent!, /1 to fix/);
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
    assert.deepEqual(ui.ops, [{ op: "set_cell", step: 24, row: 0, value: { k: "d", pad: 1 } }]);
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
    assert.deepEqual(ui.ops, [{ op: "set_cell", step: 2, row: 5, value: { k: "n", p: PATCH_ID, len: 6 } }], "clamped at the authored end (step 7)");
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
    ui.input('[aria-label="Note hold in sixteenth steps"]', "256");
    ui.click('[aria-label="Audition"]');
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

test("post to chat is offered only with a host callback and only for a ready score", async () => {
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
  const unready = model(); unready.steps = 16; unready.ready = false;
  const blocked = surface(unready, { onpost: () => { posted++; } });
  try { assert.equal(blocked.target.querySelector<HTMLButtonElement>("[data-score-post]")!.disabled, true); } finally { await blocked.done(); }
});

test("a note's volume is edited in the selected panel, remembered for the next placement and carried by resize", async () => {
  const m = model(); m.steps = 16; m.cells["0:0"] = { k: "n", p: PATCH_ID, len: 2 };
  const heard: number[] = [];
  const ui = surface(m, { onaudition: (_patch, _note, _ms, level) => heard.push(level ?? -1) });
  try {
    ui.click('[data-score-cell="0:0"]');
    ui.change('[aria-label="Volume of this note"]', "60");
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 0, row: 0, value: { k: "n", p: PATCH_ID, len: 2, vol: 60 } });
    assert.equal(heard.at(-1), 60, "the change is heard at the new level");
    // Placement uses the remembered level; a full level writes no field at all.
    ui.click('[data-score-cell="4:1"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 4, row: 1, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD, vol: 60 } });
    assert.equal(heard.at(-1), 60, "a placed note auditions at the level it was written with");
    ui.input('[aria-label="Placement volume"]', "100");
    ui.click('[data-score-cell="8:1"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 8, row: 1, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD } });
    // Out-of-range input falls back to full level: the next placement writes no field.
    ui.input('[aria-label="Placement volume"]', "0");
    ui.click('[data-score-cell="12:1"]');
    assert.deepEqual(ui.ops.at(-1), { op: "set_cell", step: 12, row: 1, value: { k: "n", p: PATCH_ID, len: SCORE_DEFAULT_HOLD } });
  } finally { await ui.done(); }
  // A note that already carries a level: the steppers move it by ten, and a resize keeps it.
  const quiet = model(); quiet.steps = 16; quiet.cells["0:0"] = { k: "n", p: PATCH_ID, len: 2, vol: 60 };
  const panel = surface(quiet);
  try {
    panel.click('[data-score-cell="0:0"]');
    assert.equal((panel.target.querySelector('[aria-label="Volume of this note"]') as HTMLInputElement).value, "60");
    panel.click('[aria-label="Louder"]');
    assert.deepEqual(panel.ops.at(-1), { op: "set_cell", step: 0, row: 0, value: { k: "n", p: PATCH_ID, len: 2, vol: 70 } });
    panel.click('[aria-label="Quieter"]');
    assert.deepEqual(panel.ops.at(-1), { op: "set_cell", step: 0, row: 0, value: { k: "n", p: PATCH_ID, len: 2, vol: 50 } });
    panel.click('[aria-label="Lengthen this note"]');
    assert.deepEqual(panel.ops.at(-1), { op: "set_cell", step: 0, row: 0, value: { k: "n", p: PATCH_ID, len: 3, vol: 60 } }, "a longer hold keeps the note's level");
    assert.ok(panel.target.querySelector('[data-score-cell="0:0"]')?.getAttribute("aria-label")?.includes("60%"), "the cell names its level");
  } finally { await panel.done(); }
});
