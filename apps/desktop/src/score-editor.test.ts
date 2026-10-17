import assert from "node:assert/strict";
import test from "node:test";
import type { JamPatch } from "./jam-contract.ts";
import type { ScoreModel, ScoreSlot, Valued, IndexSource } from "./studio-native.ts";
import type { ScoreCell } from "./studio-contract.ts";
import { scoreCellOperation, scoreContinuations, scoreDiagnosticOperation, scoreDiagnostics, scoreHeaderOperation, scorePage, scorePitch } from "./score-editor.ts";

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
  assert.deepEqual(scorePage(1, 0), [0]); assert.deepEqual(scorePage(17, 1), [16]);
  assert.deepEqual(scorePage(256, 15), Array.from({ length: 16 }, (_, i) => i + 240));
  for (const args of [[0, 0], [257, 0], [16, 1], [17, -1], [16, 0.5]]) assert.throws(() => scorePage(args[0], args[1]));
});

test("header operations use authored scalar text and native musical bounds without trimming", () => {
  assert.deepEqual(scoreHeaderOperation("title", " e\u0301🎶 "), { op: "set_header", field: "title", value: " e\u0301🎶 " });
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
  assert.equal(Object.keys(m.cells).length, 2);
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
});

// Exercise the real compiled Svelte component, not a string pattern or substitute grid. The
// repository preloads provide a DOM before Svelte evaluates (same setup as the complete suite).
const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
Object.assign(globalThis, { MouseEvent: w.MouseEvent, Event: w.Event, MutationObserver: w.MutationObserver });
const { default: ScoreEditor } = await import("./ScoreEditor.svelte");
const { mount, unmount, flushSync } = await import("svelte");
function surface(m: ScoreModel, options: { readonlyReason?: string; loading?: boolean; onaddpatch?: () => Promise<string> } = {}) {
  const target = document.createElement("div"); document.body.appendChild(target);
  const ops: unknown[] = [], notices: string[] = [];
  const instance = mount(ScoreEditor, { target, props: {
    model: m, readonlyReason: options.readonlyReason ?? "", loading: options.loading ?? false, error: "", patchLibrary: [{ name: "SYNTH", patch: PATCH }],
    onop: op => { ops.push(op); }, onnotice: text => { notices.push(text); }, onaddpatch: options.onaddpatch ?? (async () => PATCH_ID),
  } }); flushSync();
  return { target, ops, notices,
    click(selector: string) { const button = target.querySelector<HTMLButtonElement>(selector); assert.ok(button, selector); button.click(); flushSync(); },
    change(selector: string, value: string) { const input = target.querySelector<HTMLInputElement | HTMLSelectElement>(selector); assert.ok(input, selector); input.value = value; input.dispatchEvent(new w.Event("change", { bubbles: true })); flushSync(); },
    async done() { await unmount(instance); target.remove(); },
  };
}

test("compiled composer renders 25 real pitches and authors note, clear, drum and header operations", async () => {
  const m = model(); m.cells["0:0"] = { k: "n", p: PATCH_ID, len: 3 };
  const ui = surface(m);
  try {
    assert.equal(ui.target.querySelectorAll("tbody tr").length, 25);
    assert.equal(ui.target.querySelectorAll("[data-score-cell]").length, 25 * 16);
    assert.equal(ui.target.querySelector("tbody tr:first-child th")?.textContent, "C6");
    assert.equal(ui.target.querySelector("tbody tr:last-child th")?.textContent, "C4");
    ui.change('[aria-label="Note hold in sixteenth steps"]', "4");
    // bind:value listens to input, not change, for numeric controls.
    const hold = ui.target.querySelector<HTMLInputElement>('[aria-label="Note hold in sixteenth steps"]')!;
    hold.dispatchEvent(new w.Event("input", { bubbles: true })); flushSync();
    ui.click('[data-score-cell="4:1"]'); ui.click('[data-score-cell="0:0"]');
    const drumButton = [...ui.target.querySelectorAll("button")].find(b => b.textContent === "drum")!; drumButton.click(); flushSync();
    ui.change('[aria-label="Drum pad"]', "9"); ui.click('[data-score-cell="15:24"]');
    ui.change('[aria-label="Score BPM"]', "240");
    assert.deepEqual(ui.ops, [
      { op: "set_cell", step: 4, row: 1, value: { k: "n", p: PATCH_ID, len: 4 } },
      { op: "clear_cell", step: 0, row: 0 }, { op: "set_cell", step: 15, row: 24, value: { k: "d", pad: 9 } },
      { op: "set_header", field: "bpm", value: 240 },
    ]);
  } finally { await ui.done(); }
});

test("compiled composer refuses missing recipes, invalid holds and read-only writes while preserving cleanup", async () => {
  const m = model(); m.patches = {};
  let ui = surface(m);
  try { assert.equal(ui.target.querySelector<HTMLButtonElement>('[data-score-cell="0:0"]')?.disabled, true); ui.click('[data-score-cell="0:0"]'); assert.equal(ui.ops.length, 0); }
  finally { await ui.done(); }
  ui = surface(model(), { readonlyReason: "Awaiting current membership" });
  try {
    assert.equal(ui.target.querySelector<HTMLButtonElement>('[data-score-cell="0:0"]')?.disabled, true);
    ui.click('[data-score-cell="0:0"]'); assert.equal(ui.ops.length, 0);
    assert.match(ui.target.textContent!, /Awaiting current membership/);
  } finally { await ui.done(); }
  ui = surface(model());
  try {
    const hold = ui.target.querySelector<HTMLInputElement>('[aria-label="Note hold in sixteenth steps"]')!;
    hold.value = "2"; hold.dispatchEvent(new w.Event("input", { bubbles: true })); flushSync();
    ui.click('[data-score-cell="15:0"]'); assert.equal(ui.ops.length, 0); assert.match(ui.notices[0], /must fit/);
  } finally { await ui.done(); }
});

test("compiled evidence exposes removed losing values and clears cells outside the shortened score", async () => {
  const m = model(); m.steps = 1; m.ready = false;
  m.cellSlots["255:24"] = slot<ScoreCell>({ k: "n", p: PATCH_ID, len: 1 }, [null]);
  m.readiness.cells["255:24"] = { count: true, extent: true, recipe: false, overlap: false, conflict: true };
  const ui = surface(m);
  try {
    ui.click('[data-score-evidence="cell/255:24"]');
    const inspection = ui.target.querySelector('[aria-label="Retained score alternatives"]')!;
    assert.match(inspection.textContent!, /Effective removal wins/); assert.match(inspection.textContent!, /count · extent · conflict/);
    assert.equal(inspection.querySelectorAll("article").length, 2);
    assert.match(inspection.textContent!, new RegExp(alternative(null, 2).source.opId));
    const clear = [...inspection.querySelectorAll("button")].find(b => b.textContent === "clear this coordinate")!; clear.click(); flushSync();
    assert.deepEqual(ui.ops, [{ op: "clear_cell", step: 255, row: 24 }]);
    assert.match(ui.target.textContent!, /not ready/);
  } finally { await ui.done(); }
});

test("compiled diagnostics page evidence and alternatives without dropping later slots or sources", async () => {
  const m = model(); m.steps = 32;
  for (let step = 0; step < 25; step++) m.cellSlots[`${step}:0`] = slot<ScoreCell>({ k: "d", pad: 0 });
  m.cellSlots["24:0"] = { removed: false, register: { selected: alternative<ScoreCell>({ k: "d", pad: 0 }), conflicts: Array.from({ length: 9 }, (_, i) => alternative<ScoreCell>({ k: "d", pad: 1 }, i + 2)) } };
  const ui = surface(m);
  try {
    assert.equal(ui.target.querySelectorAll("[data-score-evidence]").length, 24);
    assert.equal(ui.target.querySelector('[data-score-evidence="cell/24:0"]'), null);
    ui.click('[aria-label="Next evidence page"]'); ui.click('[data-score-evidence="cell/24:0"]');
    assert.equal(ui.target.querySelectorAll("article.alternative").length, 8);
    ui.click('[aria-label="Next alternative page"]');
    assert.equal(ui.target.querySelectorAll("article.alternative").length, 2);
    assert.match(ui.target.querySelector('[aria-label="Retained score alternatives"]')!.textContent!, new RegExp(alternative(null, 10).source.opId));
    ui.click('[aria-label="Next score page"]'); assert.ok(ui.target.querySelector('[data-score-cell="31:24"]'));
  } finally { await ui.done(); }
});

test("a queued library recipe does not enable note placement before native readback", async () => {
  const m = model();
  let resolve!: (id: string) => void;
  const pending = new Promise<string>(r => { resolve = r; });
  const ui = surface(m, { onaddpatch: () => pending });
  try {
    // Select binding uses Svelte's option identity, with the real event path.
    ui.change('[aria-label="Patch library recipe"]', "0");
    const add = [...ui.target.querySelectorAll("button")].find(b => b.textContent === "add recipe to score")!; add.click(); flushSync();
    assert.match(ui.target.textContent!, /saving recipe/);
    assert.equal(ui.target.querySelector<HTMLButtonElement>('[data-score-cell="0:0"]')!.disabled, false, "the current active recipe is still usable before the new queue resolves");
    resolve("c".repeat(64)); await pending; await Promise.resolve(); flushSync();
    assert.equal(ui.target.querySelector<HTMLButtonElement>('[data-score-cell="0:0"]')!.disabled, true);
    assert.match(ui.target.textContent!, /chosen recipe is not active yet/);
    assert.equal(ui.ops.length, 0);
  } finally { await ui.done(); }
});
