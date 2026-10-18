import assert from "node:assert/strict";
import test from "node:test";
import type { JamPatch } from "./jam-contract.ts";
import type { ScoreModel, ScoreSlot, Valued, IndexSource } from "./studio-native.ts";
import type { ScoreCell } from "./studio-contract.ts";
import {
  PAD_CODES, PAD_NAMES, SCORE_PAGE_STEPS, SCORE_WIDE_PAGE_STEPS, scoreAttention, scoreBarBeat, scoreCellOperation, scoreContinuations,
  scoreDiagnosticOperation, scoreDiagnostics, scoreDurationMs, scoreExtent, scoreHeaderOperation, scoreHolds, scoreLastSoundingStep,
  scoreOccupiedSteps, scoreOnsetAt, scorePage, scorePatchOrder, scorePatchUsage, scorePitch, scorePlayheadStep, scoreRowIsBlack,
  scoreSlotDiagnostic, scoreTimeLabel,
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
  assert.deepEqual(holds.get("16:0"), { onset: "14:0", p: PATCH_ID, last: false });
  assert.deepEqual(holds.get("18:0"), { onset: "14:0", p: PATCH_ID, last: true });
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
Object.assign(globalThis, { MouseEvent: w.MouseEvent, Event: w.Event, MutationObserver: w.MutationObserver });
const { default: ScoreEditor } = await import("./ScoreEditor.svelte");
const { mount, unmount, flushSync } = await import("svelte");
type Surface = ReturnType<typeof surface>;
function surface(m: ScoreModel, options: { readonlyReason?: string; loading?: boolean; onaddpatch?: () => Promise<string>; onaudition?: (patch: JamPatch, note: number, durationMs: number) => void; playing?: boolean; positionMs?: number | null; loop?: boolean } = {}) {
  const target = document.createElement("div"); document.body.appendChild(target);
  const ops: unknown[] = [], notices: string[] = [], loops: boolean[] = [];
  const instance = mount(ScoreEditor, { target, props: {
    model: m, readonlyReason: options.readonlyReason ?? "", loading: options.loading ?? false, error: "", patchLibrary: [{ name: "SYNTH", patch: PATCH }],
    library: [{ name: "SYNTH", patch: PATCH, shelf: "preset" }, { name: "OTHER", patch: { ...PATCH, o: [{ w: 1, t: 0, c: 0, l: 80 }] }, shelf: "mine" }],
    onop: (op: unknown) => { ops.push(op); }, onnotice: (text: string) => { notices.push(text); }, onaddpatch: options.onaddpatch ?? (async () => PATCH_ID),
    onaudition: options.onaudition, playing: options.playing ?? false, positionMs: options.positionMs ?? null, loop: options.loop ?? false, onloop: (v: boolean) => { loops.push(v); },
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
    assert.equal(ui.cell("0:0").disabled, true); ui.click('[data-score-cell="0:0"]'); assert.equal(ui.ops.length, 0);
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
    ui.input('[aria-label="Note hold in sixteenth steps"]', "2");
    ui.click('[data-score-cell="15:0"]'); assert.equal(ui.ops.length, 0); assert.match(ui.notices[0], /must fit/);
  } finally { await ui.done(); }
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
    assert.equal(ui.cell("19:3").disabled, true, "nothing can be placed past the end");
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
    assert.equal(ui.cell("0:0").disabled, false, "the current active recipe is still usable before the new queue resolves");
    resolve("c".repeat(64)); await pending; await Promise.resolve(); flushSync();
    assert.equal(ui.cell("0:0").disabled, true);
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
