// Mounted tests for the SOUND inspector tab (StudioSound.svelte) and the pure helpers behind it.
//
// Contract change recorded here: the tab was rebuilt around a recipe rack, a one-octave piano and
// length tiles. The old `<select aria-label="Sound">` / `<select aria-label="Pitch">` surface, the
// "Add sound to frame"/"Save changes"/"Edit"/"Remove" button labels, the `patchLibrary`/`onhistory`
// props and the evidence-mode "over the instrument limit"/"missing frame"/"overflow" copy are gone.
// Tests that pinned those were rewritten for the new surface rather than loosened; the behaviours
// they protected (default 250 ms, exact long holds, import constraints, cleanup without a selected
// frame, the frame-change fence) are all re-pinned below against the new controls.
import assert from "node:assert/strict";
import test from "node:test";
import type { JamPatch } from "./jam-contract.ts";
import { legacyJamPatch } from "./jam-patch.ts";
import { studioSoundModel, type StudioSoundModel } from "./studio-sound.ts";
import { ALL_SOUNDS_PAGE, groupShelves, importName, pageSlice, pianoKeys, rackLabel, registerOf, sortSfx, soundAttention, waveGlyph, MISSING_GLYPH, WAVE_GLYPHS } from "./studio-sound-ui.ts";
import { ME, ROOK, deferred, fsource, id32, id64, reg } from "./studio-testkit.ts";
const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
Object.assign(globalThis, { MutationObserver: w.MutationObserver, HTMLInputElement: w.HTMLInputElement, HTMLMediaElement: w.HTMLMediaElement, Event: w.Event });
const { mount, unmount, flushSync } = await import("svelte");
const { default: StudioSound } = await import("./StudioSound.svelte");
const { default: StudioSoundHarness } = await import("../scripts/fixtures/StudioSoundHarness.svelte");
type Source = import("./studio-native.ts").FrameSource;
const patchId = id64(1), frame = id32(2), patch = legacyJamPatch("sine");
const square = legacyJamPatch("square");
function model() { return studioSoundModel({ records: { [`p/${patchId}`]: reg({ op: "set_patch", patch: patchId, descriptor: patch }, fsource(1)) },
  readiness: { sfxOverflow: [], patchOverflow: [], exportOverflow: [], missingFrames: [], missingPatches: [], conflicts: false } }); }
const marker = (id: string, extra: Partial<import("./studio-sound.ts").SfxView> = {}): import("./studio-sound.ts").SfxView =>
  ({ id, frame, patch: patchId, note: 60, durationMs: 250, source: fsource(9) as Source, conflicts: [], ...extra });
type Sfx = { id?: string; patch: string; note: number; durationMs: number };
/** Every required prop with inert defaults; a test overrides what it observes. */
function props(over: Record<string, unknown> = {}) {
  return { model: model(), frame, frameNumber: 2, frameIndexOf: (id: string) => id === frame ? 2 : 0, who: (a: string) => a === ME ? "you" : "rook",
    tint: (a: string) => a === ME ? "rgb(1, 2, 3)" : "rgb(9, 9, 9)", onpatch: async () => patchId, onsfx: () => {}, onremove: () => {}, onremovepatch: () => {},
    onaudition: () => {}, onnew: () => {}, onnotice: () => {}, ...over };
}
const buttons = (target: HTMLElement) => [...target.querySelectorAll<HTMLButtonElement>("button")];
const byText = (target: HTMLElement, text: string) => buttons(target).find((b) => b.textContent?.trim() === text);
const docTile = (target: HTMLElement, id = patchId) => target.querySelector<HTMLButtonElement>(`[data-doc-patch="${id}"]`)!;
const libTile = (target: HTMLElement, name: string) => target.querySelector<HTMLButtonElement>(`[data-lib-patch="${name}"]`)!;
const lengthField = (target: HTMLElement) => target.querySelector<HTMLInputElement>('input[aria-label="Sound length in seconds"]')!;
const settle = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); flushSync(); };
function mounted(component: unknown, p: Record<string, unknown>) {
  const target = document.createElement("div"); document.body.append(target);
  const app = mount(component as never, { target, props: p as never }); flushSync();
  return { target, app, async done() { await unmount(app); target.remove(); } };
}

// --- pure helpers ---------------------------------------------------------------------------------

test("piano layout: seven white keys, five black keys over the usual boundaries, C4 at register 4", () => {
  const { white, black } = pianoKeys(4);
  assert.deepEqual(white.map((k) => k.note), [60, 62, 64, 65, 67, 69, 71]);
  assert.deepEqual(black.map((k) => k.note), [61, 63, 66, 68, 70]);
  assert.deepEqual(white.map((k) => k.label), ["C4", "D4", "E4", "F4", "G4", "A4", "B4"]);
  assert.ok(black.every((k, i) => k.left > 0 && k.left < 100 && (i === 0 || k.left > black[i - 1].left)));
  assert.equal(registerOf(60), 4); assert.equal(registerOf(0), -1); assert.equal(registerOf(127), 9);
  // Register 9 ends at G9 (127): the layout still returns an octave, the component disables the rest.
  assert.ok(pianoKeys(9).white.some((k) => k.note > 127));
});
test("wave glyphs follow the first oscillator and fall back to the dashed line", () => {
  assert.equal(waveGlyph(patch), WAVE_GLYPHS[0]); assert.equal(waveGlyph(square), WAVE_GLYPHS[2]);
  assert.equal(waveGlyph(null), MISSING_GLYPH); assert.equal(waveGlyph({ ...patch, o: [{ w: 9, t: 0, c: 0, l: 100 }] } as JamPatch), MISSING_GLYPH);
  assert.equal(rackLabel("a very long recipe name"), "A VERY LONG "); assert.equal(importName("C:\\sounds\\meow.json"), "MEOW"); assert.equal(importName("bell.jampatch"), "BELL"); assert.equal(importName(undefined), "IMPORT");
});
test("shelves keep preset/mine/share order and drop empty shelves", () => {
  const groups = groupShelves([{ name: "b", patch, shelf: "share" }, { name: "a", patch, shelf: "preset" }, { name: "c", patch, shelf: "share" }]);
  assert.deepEqual(groups.map((g) => [g.label, g.items.map((i) => i.name)]), [["presets", ["a"]], ["from the share", ["b", "c"]]]);
  assert.deepEqual(groupShelves([]), []);
});
test("attention counts distinct marker ids across every readiness list plus conflicts; recipes separately", () => {
  const m = model(); const a = id32(5), b = id32(6);
  m.sfx = [marker(a, { conflicts: [{ value: { op: "set_sfx", sfx: a, frame, patch: patchId, note: 1, duration_ms: 1 }, source: fsource(3, ROOK) as Source }] }), marker(b)];
  m.readiness = { ...m.readiness, missingFrames: [a, b], sfxOverflow: [b], patchOverflow: [patchId, patchId] };
  assert.deepEqual(soundAttention(m), { markers: 2, recipes: 1 });
  assert.deepEqual(soundAttention(model()), { markers: 0, recipes: 0 });
});
test("all-sounds order is timeline then id, and pages clamp instead of going blank", () => {
  const rows = sortSfx([marker(id32(3), { frame: "f2" }), marker(id32(2), { frame: "f1" }), marker(id32(1), { frame: "f2" }), marker(id32(4), { frame: "gone" })], (f) => ({ f1: 1, f2: 2 })[f] ?? 0);
  assert.deepEqual(rows.map((r) => [r.frameNumber, r.id]), [[0, id32(4)], [1, id32(2)], [2, id32(1)], [2, id32(3)]]);
  assert.deepEqual(pageSlice([1, 2, 3], 5, 2), { items: [3], page: 1, pages: 2 });
  assert.deepEqual(pageSlice([], 3, 8), { items: [], page: 0, pages: 1 });
});

// --- authoring ------------------------------------------------------------------------------------

test("a document recipe tile authors a 250 ms default and keeps an explicit 95 s from the number field", async () => {
  const ops: Sfx[] = [];
  const { target, done } = mounted(StudioSound, props({ onsfx: (v: Sfx) => ops.push(v) }));
  try {
    const add = byText(target, "add to frame 2")!; assert.equal(add.disabled, true, "nothing picked yet");
    docTile(target).click(); flushSync(); assert.equal(docTile(target).getAttribute("aria-pressed"), "true");
    assert.equal(lengthField(target).value, "0.25"); assert.equal(add.disabled, false);
    add.click(); flushSync();
    assert.deepEqual(ops, [{ patch: patchId, note: 60, durationMs: 250 }]);
    const field = lengthField(target); field.value = "95"; field.dispatchEvent(new w.Event("input", { bubbles: true })); flushSync();
    assert.match(target.textContent!, /95\.000/);
    add.click(); flushSync();
    assert.deepEqual(ops[1], { patch: patchId, note: 60, durationMs: 95000 });
    field.value = ""; field.dispatchEvent(new w.Event("input", { bubbles: true })); flushSync();
    assert.equal(add.disabled, true, "an empty length field disables add");
  } finally { await done(); }
});

test("a library tile never calls onpatch on selection; add calls it once and then onsfx with the returned id", async () => {
  const ops: Sfx[] = []; let patches = 0; const libId = id64(7);
  const { target, app, done } = mounted(StudioSoundHarness, props({ library: [{ name: "meow", patch: square, shelf: "preset" }], onpatch: async (p: JamPatch) => { patches++; assert.deepEqual(p, square); return libId; }, onsfx: (v: Sfx) => ops.push(v) }));
  try {
    assert.match(target.textContent!, /presets · 1/);
    libTile(target, "meow").click(); flushSync();
    assert.equal(patches, 0, "selection is deferred"); assert.equal(libTile(target, "meow").getAttribute("aria-pressed"), "true");
    byText(target, "add to frame 2")!.click(); await settle();
    assert.equal(patches, 1); assert.deepEqual(ops, [{ patch: libId, note: 60, durationMs: 250 }]);
    // Until the host re-reads the document the recipe is not there, so another add writes it again
    // (idempotent by id); once the re-read lands, the same selection resolves to the document slot.
    byText(target, "add to frame 2")!.click(); await settle(); assert.equal(patches, 2);
    const next = model(); next.records[`p/${libId}`] = reg({ op: "set_patch", patch: libId, descriptor: square }, fsource(2));
    next.patchSlots.push({ id: libId, removed: false, descriptor: square, register: next.records[`p/${libId}`] as never }); next.patches[libId] = square;
    (app as unknown as { setModel: (m: StudioSoundModel) => void }).setModel(next); flushSync();
    assert.equal(docTile(target, libId).getAttribute("aria-pressed"), "true", "the selection follows the recipe onto the document shelf");
    byText(target, "add to frame 2")!.click(); await settle();
    assert.equal(patches, 2, "the document already holds these bytes"); assert.equal(ops.length, 3); assert.equal(ops[2].patch, libId);
  } finally { await done(); }
});

test("a frame change while onpatch is pending drops the stale onsfx and says so", async () => {
  const ops: Sfx[] = [], notices: string[] = []; const gate = deferred<string>();
  const { target, app, done } = mounted(StudioSoundHarness, props({ library: [{ name: "meow", patch: square, shelf: "mine" }], onpatch: () => gate.promise, onsfx: (v: Sfx) => ops.push(v), onnotice: (t: string, k: string) => notices.push(k) }));
  try {
    libTile(target, "meow").click(); flushSync(); byText(target, "add to frame 2")!.click(); flushSync();
    assert.equal(byText(target, "adding…")!.disabled, true);
    (app as unknown as { setFrame: (f: string) => void }).setFrame(id32(10)); flushSync();
    gate.resolve(id64(7)); await settle();
    assert.deepEqual(ops, []); assert.deepEqual(notices, ["warn"]);
  } finally { await done(); }
});

test("editing a marker saves under the same id, cancel drops it, and a frame change clears the edit", async () => {
  const ops: Sfx[] = []; const m = model(); const id = id32(9); m.sfx = [marker(id, { note: 100, durationMs: 95000 })];
  const { target, app, done } = mounted(StudioSoundHarness, props({ model: m, onsfx: (v: Sfx) => ops.push(v) }));
  try {
    byText(target, "edit")!.click(); flushSync();
    assert.match(target.textContent!, /editing · Sine sound/); assert.equal(lengthField(target).value, "95");
    assert.equal(target.querySelector('button[aria-label="E7"]')?.getAttribute("aria-pressed"), "true", "the piano shows the marker's octave (100 = E7)");
    assert.ok(target.querySelector(".snd.cur"));
    byText(target, "save changes")!.click(); flushSync();
    assert.deepEqual(ops, [{ id, patch: patchId, note: 100, durationMs: 95000 }]);
    byText(target, "edit")!.click(); flushSync(); byText(target, "cancel")!.click(); flushSync();
    assert.equal(byText(target, "save changes"), undefined); assert.equal(lengthField(target).value, "0.25");
    byText(target, "edit")!.click(); flushSync();
    (app as unknown as { setFrame: (f: string) => void }).setFrame(id32(10)); flushSync();
    assert.equal(byText(target, "save changes"), undefined, "the edited identity does not follow the frame");
    docTile(target).click(); flushSync(); byText(target, "add to frame 2")!.click(); flushSync();
    assert.equal(ops[1].id, undefined, "the new frame receives a fresh marker, not the old frame's identity");
  } finally { await done(); }
});

test("an emoji .json import constrains the pitch, locks the length, claims no shared catalog and adds without onpatch at pick time", async () => {
  // The imported bytes (square) are not in the document (sine), so the add must write the recipe.
  const ops: Sfx[] = [], notices: string[] = []; let patches = 0; const libId = id64(7);
  const { target, done } = mounted(StudioSound, props({ onpatch: async (p: JamPatch) => { patches++; assert.deepEqual(p, square); return libId; }, onsfx: (v: Sfx) => ops.push(v), onnotice: (t: string) => notices.push(t) }));
  try {
    const file = target.querySelector<HTMLInputElement>('input[type="file"]')!;
    assert.equal(file.getAttribute("accept"), ".json,.jampatch");
    Object.defineProperty(file, "files", { configurable: true, value: [{ name: "meow.json", size: 500, text: async () => JSON.stringify({ v: 1, img: id64(3), patch: square, lo: 70, hi: 75, len: 333 }) }] });
    file.dispatchEvent(new w.Event("change", { bubbles: true })); await settle();
    assert.equal(patches, 0, "an import is a pending selection, not a document write");
    assert.match(target.textContent!, /imported/); assert.doesNotMatch(target.textContent!, /shared emoji catalog/);
    assert.equal(libTile(target, "MEOW").getAttribute("aria-pressed"), "true");
    assert.equal(lengthField(target).disabled, true); assert.equal(lengthField(target).value, "0.333");
    assert.ok(buttons(target).filter((b) => b.textContent?.trim() === "0.25").every((b) => b.disabled), "length tiles are locked");
    assert.equal(target.querySelector<HTMLButtonElement>('button[aria-label="C4"]')!.disabled, true, "below lo");
    assert.equal(target.querySelector<HTMLButtonElement>('button[aria-label="A♯4"]')!.disabled, false);
    assert.equal(target.querySelector<HTMLButtonElement>('button[aria-label="A♯4"]')!.getAttribute("aria-pressed"), "true", "the note clamps up to lo");
    // Octave steps clamp inside the import's range: up lands on hi (75 = D♯5) and can go no further.
    target.querySelector<HTMLButtonElement>('button[aria-label="Up an octave"]')!.click(); flushSync();
    assert.equal(target.querySelector<HTMLButtonElement>('button[aria-label="D♯5"]')!.getAttribute("aria-pressed"), "true");
    assert.equal(target.querySelector<HTMLButtonElement>('button[aria-label="E5"]')!.disabled, true, "above hi");
    assert.equal(target.querySelector<HTMLButtonElement>('button[aria-label="Up an octave"]')!.disabled, true);
    target.querySelector<HTMLButtonElement>('button[aria-label="Down an octave"]')!.click(); flushSync();
    assert.equal(target.querySelector<HTMLButtonElement>('button[aria-label="A♯4"]')!.getAttribute("aria-pressed"), "true", "and down clamps back to lo");
    byText(target, "add to frame 2")!.click(); await settle();
    assert.equal(patches, 1); assert.deepEqual(ops, [{ patch: libId, note: 70, durationMs: 333 }]); assert.deepEqual(notices, []);
    // Picking another tile releases the constraints: they described that file, not the next recipe.
    docTile(target).click(); flushSync();
    assert.equal(lengthField(target).disabled, false); assert.equal(target.querySelector<HTMLButtonElement>('button[aria-label="C4"]')!.disabled, false);
  } finally { await done(); }
});

test("an oversized or malformed import is refused with a notice and a .jampatch becomes a pending tile", async () => {
  const notices: string[] = []; let patches = 0;
  const { target, done } = mounted(StudioSound, props({ onpatch: async () => { patches++; return patchId; }, onnotice: (t: string) => notices.push(t) }));
  try {
    const file = target.querySelector<HTMLInputElement>('input[type="file"]')!;
    const load = async (f: Record<string, unknown>) => { Object.defineProperty(file, "files", { configurable: true, value: [f] }); file.dispatchEvent(new w.Event("change", { bubbles: true })); await settle(); };
    await load({ name: "big.json", size: 5000, text: async () => "{}" });
    assert.deepEqual(notices, ["sound file exceeds 4KiB"]);
    await load({ name: "bad.jampatch", size: 10, text: async () => "nope" });
    assert.match(notices[1], /not valid JSON/);
    await load({ name: "bell.jampatch", size: 100, text: async () => JSON.stringify(square) });
    assert.equal(notices.length, 2); assert.equal(libTile(target, "BELL").getAttribute("aria-pressed"), "true");
    assert.equal(lengthField(target).disabled, false, "a bare recipe carries no length lock"); assert.equal(patches, 0);
  } finally { await done(); }
});

test("a file read completing after unmount, or after frame A→B→A, selects nothing and writes nothing", async () => {
  const read = deferred<string>(); let patches = 0;
  const first = mounted(StudioSound, props({ onpatch: async () => { patches++; return patchId; } }));
  const file = first.target.querySelector<HTMLInputElement>('input[type="file"]')!;
  Object.defineProperty(file, "files", { configurable: true, value: [{ name: "a.json", size: 500, text: () => read.promise }] }); file.dispatchEvent(new w.Event("change", { bubbles: true }));
  await first.done();
  read.resolve(JSON.stringify({ v: 1, img: id64(3), patch, lo: 70, hi: 75, len: 333 })); await settle(); assert.equal(patches, 0);
  const again = deferred<string>();
  const { target, app, done } = mounted(StudioSoundHarness, props({ onpatch: async () => { patches++; return patchId; } }));
  try {
    const input = target.querySelector<HTMLInputElement>('input[type="file"]')!;
    Object.defineProperty(input, "files", { configurable: true, value: [{ name: "a.json", size: 500, text: () => again.promise }] }); input.dispatchEvent(new w.Event("change", { bubbles: true }));
    const harness = app as unknown as { setFrame: (f: string) => void }; harness.setFrame(id32(10)); flushSync(); harness.setFrame(frame); flushSync();
    again.resolve(JSON.stringify({ v: 1, img: id64(3), patch, lo: 70, hi: 75, len: 333 })); await settle();
    assert.equal(patches, 0); assert.doesNotMatch(target.textContent!, /imported/); assert.equal(lengthField(target).disabled, false);
  } finally { await done(); }
});

test("pitch controls clamp to 0..127 and the piano follows the octave", async () => {
  const { target, done } = mounted(StudioSound, props());
  try {
    const key = (label: string) => target.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
    key("Up an octave").click(); flushSync(); assert.match(target.textContent!, /C5/); assert.equal(key("C5").getAttribute("aria-pressed"), "true");
    key("Down a semitone").click(); flushSync(); assert.match(target.textContent!, /B4/); assert.ok(key("B4"), "the octave view moved down with the note");
    for (let i = 0; i < 12; i++) { key("Up an octave").click(); flushSync(); }
    assert.match(target.textContent!, /midi 127/); assert.equal(key("Up an octave").disabled, true); assert.equal(key("Up a semitone").disabled, true);
    assert.equal(key("G♯9").disabled, true, "notes above 127 are shown but not playable");
    for (let i = 0; i < 12; i++) { key("Down an octave").click(); flushSync(); }
    assert.match(target.textContent!, /midi 0/); assert.equal(key("Down a semitone").disabled, true);
  } finally { await done(); }
});

test("hear it auditions the picked recipe at the chosen pitch and length; the row play auditions the marker", async () => {
  const heard: [JamPatch, number, number][] = []; const m = model(); m.sfx = [marker(id32(9), { note: 64, durationMs: 500 })];
  const { target, done } = mounted(StudioSound, props({ model: m, onaudition: (p: JamPatch, n: number, d: number) => heard.push([p, n, d]) }));
  try {
    assert.equal(byText(target, "hear it")!.disabled, true);
    docTile(target).click(); flushSync(); byText(target, "hear it")!.click();
    assert.equal(heard.length, 1); assert.equal(heard[0][0], m.patches[patchId]); assert.equal(heard[0][1], 60); assert.equal(heard[0][2], 250);
    target.querySelector<HTMLButtonElement>('button[aria-label="Hear this sound"]')!.click();
    assert.deepEqual(heard[1], [m.patches[patchId], 64, 500]);
  } finally { await done(); }
});

test("+ new… opens the soundfont editor seeded with the selection, or with nothing", async () => {
  const seeds: (JamPatch | null)[] = [];
  const { target, done } = mounted(StudioSound, props({ library: [{ name: "meow", patch: square, shelf: "share" }], onnew: (p: JamPatch | null) => seeds.push(p) }));
  try {
    byText(target, "+ new…")!.click(); libTile(target, "meow").click(); flushSync(); byText(target, "+ new…")!.click();
    docTile(target).click(); flushSync(); byText(target, "+ new…")!.click();
    assert.deepEqual(seeds, [null, square, patch]);
  } finally { await done(); }
});

// --- readiness and attention ------------------------------------------------------------------------

test("a marker whose recipe is gone shows the chip, offers pick one, and its remove calls onremove", async () => {
  const removed: string[] = [], ops: Sfx[] = []; const m = model(); const id = id32(9);
  m.sfx = [marker(id, { patch: id64(99) })]; m.readiness.missingPatches = [id]; m.ready = false;
  const { target, done } = mounted(StudioSound, props({ model: m, onremove: (x: string) => removed.push(x), onsfx: (v: Sfx) => ops.push(v) }));
  try {
    assert.match(target.textContent!, /1 sound needs attention/); assert.match(target.textContent!, /missing recipe/); assert.match(target.textContent!, /\? · C4/);
    assert.equal(target.querySelector<HTMLButtonElement>('button[aria-label="Hear this sound"]')!.disabled, true);
    byText(target, "pick one")!.click(); flushSync();
    assert.match(target.textContent!, /editing · pick a recipe/); assert.equal(byText(target, "save changes")!.disabled, true, "no recipe picked yet");
    docTile(target).click(); flushSync(); byText(target, "save changes")!.click(); flushSync();
    assert.deepEqual(ops, [{ id, patch: patchId, note: 60, durationMs: 250 }]);
    target.querySelector<HTMLButtonElement>(`[data-remove-sfx="${id}"]`)!.click(); assert.deepEqual(removed, [id]);
  } finally { await done(); }
});

test("a conflict card offers keep this and use this under the same marker id", async () => {
  const ops: Sfx[] = []; const m = model(); const id = id32(9);
  m.sfx = [marker(id, { conflicts: [{ value: { op: "set_sfx", sfx: id, frame, patch: patchId, note: 69, duration_ms: 500 }, source: fsource(5, ROOK) as Source }] })];
  m.readiness.conflicts = true; m.ready = false;
  const { target, done } = mounted(StudioSound, props({ model: m, onsfx: (v: Sfx) => ops.push(v) }));
  try {
    assert.match(target.textContent!, /another value by rook/); assert.match(target.textContent!, /1 sound needs attention/);
    assert.ok(target.querySelector('.st-alt i[style*="rgb(9, 9, 9)"]'), "the alternative carries its author's tint");
    byText(target, "use this")!.click(); byText(target, "keep this")!.click();
    assert.deepEqual(ops, [{ id, patch: patchId, note: 69, durationMs: 500 }, { id, patch: patchId, note: 60, durationMs: 250 }]);
  } finally { await done(); }
});

test("the attention banner counts recipes over the cap separately and marks their tiles", async () => {
  const m = model(); m.readiness.patchOverflow = [patchId]; m.ready = false;
  const { target, done } = mounted(StudioSound, props({ model: m }));
  try {
    assert.match(target.textContent!, /1 recipe over the cap/); assert.doesNotMatch(target.textContent!, /sounds? needs? attention/);
    assert.ok(docTile(target).classList.contains("gone")); assert.match(docTile(target).textContent!, /over/);
  } finally { await done(); }
});

test("all sounds pages at 24 rows, marks the selected frame, deleted frames and over-cap markers", async () => {
  const removed: string[] = []; const m = model(); const frames = Array.from({ length: 30 }, (_, i) => id32(100 + i));
  m.sfx = frames.map((f, i) => marker(id32(200 + i), { frame: i === 3 ? frame : f, note: i }));
  m.readiness.missingFrames = [id32(205)]; m.readiness.sfxOverflow = [id32(206)]; m.ready = false;
  const { target, done } = mounted(StudioSound, props({ model: m, frameIndexOf: (f: string) => f === frame ? 2 : f === id32(105) ? 0 : 10 + frames.indexOf(f), onremove: (x: string) => removed.push(x) }));
  try {
    const rows = () => [...target.querySelectorAll(".sfx")];
    assert.equal(rows().length, ALL_SOUNDS_PAGE); assert.match(target.textContent!, /all sounds · 30/);
    assert.equal(rows()[0].querySelector(".fr")!.textContent, "?", "a gone frame sorts first and shows ?");
    assert.match(rows()[0].textContent!, /deleted frame/); assert.ok(rows()[1].classList.contains("cur"));
    assert.ok(rows().some((r) => r.classList.contains("over") && /over the cap/.test(r.textContent!)));
    rows()[0].querySelector<HTMLButtonElement>("[data-remove-sfx]")!.click(); assert.deepEqual(removed, [id32(205)]);
    target.querySelector<HTMLButtonElement>('button[aria-label="Next page"]')!.click(); flushSync();
    assert.equal(rows().length, 6); assert.equal(target.querySelector<HTMLButtonElement>('button[aria-label="Next page"]')!.disabled, true);
  } finally { await done(); }
});

test("read-only disables every control, shows the reason once, and hides nothing", async () => {
  const m = model(); m.sfx = [marker(id32(9))];
  const { target, done } = mounted(StudioSound, props({ model: m, readonlyReason: "Stop playback before editing", library: [{ name: "meow", patch: square, shelf: "mine" }] }));
  try {
    assert.equal(target.textContent!.split("Stop playback before editing").length, 2);
    const all = buttons(target); assert.ok(all.length > 20); assert.ok(all.every((b) => b.disabled), "every button is disabled, not hidden");
    assert.equal(lengthField(target).disabled, true); assert.ok(target.querySelector(".piano")); assert.ok(libTile(target, "meow"));
  } finally { await done(); }
});

test("without frames the add form is hidden and the ghost row says so; all sounds still lists", async () => {
  const m = model(); m.sfx = [marker(id32(9), { frame: id32(77) })];
  const { target, done } = mounted(StudioSound, props({ model: m, frame: "", frameNumber: 0, frameIndexOf: () => 0 }));
  try {
    assert.match(target.textContent!, /no frames yet/); assert.equal(target.querySelector(".rack"), null); assert.equal(byText(target, "hear it"), undefined);
    assert.equal(target.querySelectorAll(".sfx").length, 1);
  } finally { await done(); }
});

// --- evidence mode ------------------------------------------------------------------------------------

test("evidence mode lists usage per recipe, offers remove only for unused ones, and honours read-only", async () => {
  for (const readonlyReason of ["", "Stop playback before editing"]) {
    const removed: string[] = []; const m = model(); const used = id64(8);
    m.records[`p/${used}`] = reg({ op: "set_patch", patch: used, descriptor: square }, fsource(2)); m.patchSlots.push({ id: used, removed: false, descriptor: square, register: m.records[`p/${used}`] as never }); m.patches[used] = square;
    m.sfx = [marker(id32(9), { patch: used }), marker(id32(10), { patch: used })];
    m.readiness.patchOverflow = [patchId]; m.ready = false;
    const { target, done } = mounted(StudioSound, props({ model: m, frame: "", history: true, readonlyReason, onremovepatch: (id: string) => removed.push(id) }));
    try {
      assert.match(target.textContent!, /recipes · 2 live · 2 markers/); assert.match(target.textContent!, /2 markers/); assert.match(target.textContent!, /unused/); assert.match(target.textContent!, /over the cap/);
      assert.equal(target.querySelector(`[data-remove-patch="${used}"]`), null, "a used recipe has no remove: removing it would orphan its markers");
      const button = target.querySelector<HTMLButtonElement>(`[data-remove-patch="${patchId}"]`)!;
      assert.equal(button.disabled, !!readonlyReason); button.click(); assert.deepEqual(removed, readonlyReason ? [] : [patchId]);
      assert.equal(target.querySelector(".rack"), null, "no authoring surface in evidence mode");
    } finally { await done(); }
  }
});

test("two share recipes with the same display name are two tiles, and picking one lights only that one", async () => {
  // jamPatchFileName truncates to 12 characters, so `bass-line-v1` and `bass-line-v2` arrive
  // under one name; keying the shelf on the name alone collapsed them (Svelte: each_key_duplicate).
  const other = legacyJamPatch("sawtooth");
  const { target, done } = mounted(StudioSoundHarness, props({ library: [
    { name: "BASS-LINE-V", patch: square, shelf: "share" }, { name: "BASS-LINE-V", patch: other, shelf: "share" },
  ] }));
  try {
    const tiles = [...target.querySelectorAll<HTMLButtonElement>('[data-lib-patch="BASS-LINE-V"]')];
    assert.equal(tiles.length, 2, "same name, different recipe: both tiles render");
    tiles[1].click(); flushSync();
    assert.equal(tiles[0].getAttribute("aria-pressed"), "false"); assert.equal(tiles[1].getAttribute("aria-pressed"), "true");
    tiles[0].click(); flushSync();
    assert.equal(tiles[0].getAttribute("aria-pressed"), "true"); assert.equal(tiles[1].getAttribute("aria-pressed"), "false");
  } finally { await done(); }
});

test("a document re-read while onpatch is pending does not drop the marker; only a frame change does", async () => {
  // The model reference changes on every remote edit. Refusing the marker then would leave the
  // recipe written (a spent slot) without the sound the press asked for.
  const ops: Sfx[] = []; const gate = deferred<string>();
  const { target, app, done } = mounted(StudioSoundHarness, props({ library: [{ name: "meow", patch: square, shelf: "mine" }], onpatch: () => gate.promise, onsfx: (v: Sfx) => ops.push(v) }));
  try {
    libTile(target, "meow").click(); flushSync(); byText(target, "add to frame 2")!.click(); flushSync();
    (app as unknown as { setModel: (m: StudioSoundModel) => void }).setModel(model()); flushSync();
    gate.resolve(id64(7)); await settle();
    assert.deepEqual(ops, [{ patch: id64(7), note: 60, durationMs: 250 }]);
  } finally { await done(); }
});

test("evidence cleanup lists only orphaned/overflow/dangling markers, reachable without a selected frame", async () => {
  const removed: string[] = []; const m = model(); const bad = id32(9), good = id32(10), gone = id64(5);
  m.sfx = [marker(bad, { patch: gone }), marker(good)]; m.records[`p/${gone}`] = reg({ op: "remove_patch", patch: gone }, fsource(4));
  m.patchSlots.push({ id: gone, removed: true, descriptor: null, register: m.records[`p/${gone}`] as never });
  m.ready = false; m.readiness.missingFrames = [bad]; m.readiness.sfxOverflow = [bad]; m.readiness.missingPatches = [bad];
  const { target, done } = mounted(StudioSound, props({ model: m, frame: "", history: true, onremove: (id: string) => removed.push(id) }));
  try {
    assert.match(target.textContent!, /markers needing cleanup · 1/);
    const row = target.querySelector(".sfx")!; assert.match(row.textContent!, /deleted frame/); assert.match(row.textContent!, /missing recipe/); assert.match(row.textContent!, /over the cap/);
    assert.equal(target.querySelector(`[data-remove-sfx="${good}"]`), null);
    target.querySelector<HTMLButtonElement>(`[data-remove-sfx="${bad}"]`)!.click(); assert.deepEqual(removed, [bad]);
    assert.match(target.textContent!, /removed/); assert.match(target.textContent!, /raw records · 2/); assert.match(target.textContent!, /0 deleted marker ids/);
    assert.ok(target.querySelector("pre")!.textContent!.includes("set_patch"));
  } finally { await done(); }
});
