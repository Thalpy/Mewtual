import assert from "node:assert/strict";
import test from "node:test";
import { legacyJamPatch } from "./jam-patch.ts";
import { studioSoundModel } from "./studio-sound.ts";
import { deferred, fsource, id32, id64, reg } from "./studio-testkit.ts";
const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
Object.assign(globalThis, { MutationObserver: w.MutationObserver, HTMLInputElement: w.HTMLInputElement, HTMLMediaElement: w.HTMLMediaElement, Event: w.Event });
const { mount, unmount, flushSync } = await import("svelte");
const { default: StudioSound } = await import("./StudioSound.svelte");
const { default: StudioSoundHarness } = await import("../scripts/fixtures/StudioSoundHarness.svelte");
const patchId = id64(1), frame = id32(2), patch = legacyJamPatch("sine");
function model() { return studioSoundModel({ records: { [`p/${patchId}`]: reg({ op: "set_patch", patch: patchId, descriptor: patch }, fsource(1)) },
  readiness: { sfxOverflow: [], patchOverflow: [], exportOverflow: [], missingFrames: [], missingPatches: [], conflicts: false } }); }
test("Sound panel shows a new-marker default and preserves an explicit long duration", async () => {
  const target = document.createElement("div"); document.body.append(target); const ops: unknown[] = [];
  const app = mount(StudioSound, { target, props: { model: model(), frame, onpatch: async () => patchId, onsfx: (value) => ops.push(value), onremove: () => {}, onremovepatch: () => {}, onnotice: () => {} } }); flushSync();
  try {
    const add = [...target.querySelectorAll("button")].find(b => b.textContent === "Add sound to frame")!; assert.equal(add.disabled, true);
    const select = target.querySelector("select")!; select.value = patchId; select.dispatchEvent(new w.Event("change", { bubbles: true }));
    const duration = target.querySelector<HTMLInputElement>('input[aria-label="Sound length in seconds"]')!;
    assert.equal(duration.value, "0.25"); flushSync(); add.click(); flushSync();
    assert.deepEqual(ops, [{ patch: patchId, note: 60, durationMs: 250 }]);
    duration.value = "95"; duration.dispatchEvent(new w.Event("input", { bubbles: true })); flushSync(); add.click(); flushSync();
    assert.deepEqual(ops[1], { patch: patchId, note: 60, durationMs: 95000 });
  } finally { await unmount(app); target.remove(); }
});
test("emoji file source preserves validated length and does not claim a verified shared catalog", async () => {
  const target = document.createElement("div"); document.body.append(target); const ops: unknown[] = [], notices: string[] = [];
  const app = mount(StudioSound, { target, props: { model: model(), frame, onpatch: async () => patchId, onsfx: (value) => ops.push(value), onremove: () => {}, onremovepatch: () => {}, onnotice: (text) => notices.push(text) } }); flushSync();
  try {
    const file = target.querySelector<HTMLInputElement>('input[type="file"]')!;
    Object.defineProperty(file, "files", { value: [{ size: 500, text: async () => JSON.stringify({ v: 1, img: id64(3), patch, lo: 70, hi: 75, len: 333 }) }] });
    file.dispatchEvent(new w.Event("change", { bubbles: true })); for (let i = 0; i < 5; i++) await Promise.resolve(); flushSync();
    assert.match(target.textContent!, /Imported sound · 0\.333s/); assert.doesNotMatch(target.textContent!, /shared emoji catalog/);
    assert.equal(target.querySelector<HTMLInputElement>('input[aria-label="Sound length in seconds"]')!.disabled, true);
    [...target.querySelectorAll("button")].find(b => b.textContent === "Add sound to frame")!.click(); flushSync();
    assert.deepEqual(ops, [{ patch: patchId, note: 70, durationMs: 333 }]); assert.deepEqual(notices, []);
  } finally { await unmount(app); target.remove(); }
});

test("import restrictions do not leak into another marker or a library instrument", async () => {
  const target = document.createElement("div"); document.body.append(target); const ops: unknown[] = [], m = model(), marker = id32(9);
  m.sfx = [{id: marker, frame, patch: patchId, note: 100, durationMs: 95000, source: fsource(9), conflicts: []}];
  const app = mount(StudioSound, {target, props: {model: m, frame, patchLibrary: [{name: "Sine", patch}], onpatch: async () => patchId, onsfx: value => ops.push(value), onremove() {}, onremovepatch() {}, onnotice() {}}}); flushSync();
  const importSound = async () => {
    const file = target.querySelector<HTMLInputElement>('input[type="file"]')!;
    Object.defineProperty(file, "files", {configurable: true, value: [{size: 500, text: async () => JSON.stringify({v: 1, img: id64(3), patch, lo: 70, hi: 75, len: 333})}]});
    file.dispatchEvent(new Event("change", {bubbles: true})); for (let i = 0; i < 5; i++) await Promise.resolve(); flushSync();
  };
  try {
    await importSound(); [...target.querySelectorAll("button")].find(b => b.textContent === "Edit")!.click(); flushSync();
    const duration = target.querySelector<HTMLInputElement>('[aria-label="Sound length in seconds"]')!;
    assert.equal(duration.disabled, false); assert.equal(duration.value, "95");
    assert.ok(target.querySelector('[aria-label="Pitch"] option[value="100"]'));
    [...target.querySelectorAll("button")].find(b => b.textContent === "Save changes")!.click(); flushSync();
    assert.deepEqual(ops[0], {id: marker, patch: patchId, note: 100, durationMs: 95000});
    await importSound(); const sound = target.querySelector<HTMLSelectElement>('[aria-label="Sound"]')!;
    sound.value = "library:0"; sound.dispatchEvent(new Event("change", {bubbles: true})); for (let i = 0; i < 5; i++) await Promise.resolve(); flushSync();
    assert.equal(duration.disabled, false); assert.ok(target.querySelector('[aria-label="Pitch"] option[value="0"]'));
    duration.value = "95"; duration.dispatchEvent(new Event("input", {bubbles: true})); flushSync();
    [...target.querySelectorAll("button")].find(b => b.textContent === "Add sound to frame")!.click(); flushSync();
    assert.deepEqual(ops[1], {patch: patchId, note: 70, durationMs: 95000});
  } finally { await unmount(app); target.remove(); }
});

test("History can remove an overflowing unused instrument without a selected frame and respects read-only state", async () => {
  for (const readonlyReason of ["", "Stop playback before editing"]) {
    const target = document.createElement("div"); document.body.append(target); const removed: string[] = [], m = model();
    m.readiness.patchOverflow = [patchId]; m.ready = false;
    const app = mount(StudioSound, {target, props: {model: m, frame: "", history: true, readonlyReason, onpatch: async () => patchId, onsfx() {}, onremove() {}, onremovepatch: id => removed.push(id), onnotice() {}}}); flushSync();
    try {
      assert.match(target.textContent!, /over the instrument limit.*unused/);
      const button = target.querySelector<HTMLButtonElement>(`[data-remove-patch="${patchId}"]`)!;
      assert.equal(button.disabled, !!readonlyReason); button.click(); assert.deepEqual(removed, readonlyReason ? [] : [patchId]);
    } finally { await unmount(app); target.remove(); }
  }
});

test("a file chooser/read completing after Sound unmount cannot queue a recipe into another object", async () => {
  const target = document.createElement("div"); document.body.append(target); const read = deferred<string>(); let patches = 0;
  const app = mount(StudioSound, { target, props: { model: model(), frame, onpatch: async () => { patches++; return patchId; }, onsfx: () => {}, onremove: () => {}, onremovepatch: () => {}, onnotice: () => {} } }); flushSync();
  const file = target.querySelector<HTMLInputElement>('input[type="file"]')!;
  Object.defineProperty(file, "files", { value: [{ size: 500, text: () => read.promise }] }); file.dispatchEvent(new w.Event("change", { bubbles: true }));
  await unmount(app); target.remove();
  read.resolve(JSON.stringify({ v: 1, img: id64(3), patch, lo: 70, hi: 75, len: 333 }));
  for (let i = 0; i < 5; i++) await Promise.resolve(); assert.equal(patches, 0);
});

test("orphaned/overflow sound cleanup remains reachable without selecting its deleted frame", async () => {
  const target = document.createElement("div"); document.body.append(target); const removed: string[] = [], marker = id32(9), m = model();
  m.sfx = [{ id: marker, frame, patch: patchId, note: 60, durationMs: 333, source: fsource(9) as import("./studio-native.ts").FrameSource, conflicts: [] }];
  m.ready = false; m.readiness.missingFrames = [marker]; m.readiness.sfxOverflow = [marker];
  const app = mount(StudioSound, { target, props: { model: m, frame: "", history: true, onpatch: async () => patchId, onsfx: () => {}, onremove: id => removed.push(id), onremovepatch: () => {}, onnotice: () => {} } }); flushSync();
  try { assert.match(target.textContent!, /missing frame.*overflow/); target.querySelector<HTMLButtonElement>(`[data-remove-sfx="${marker}"]`)!.click(); assert.deepEqual(removed, [marker]); }
  finally { await unmount(app); target.remove(); }
});

test("changing frame clears the edited marker identity instead of silently moving it to the new frame", async () => {
  const target = document.createElement("div"); document.body.append(target); const ops: { id?: string }[] = [], marker = id32(9), m = model();
  m.sfx = [{ id: marker, frame, patch: patchId, note: 60, durationMs: 333, source: fsource(9) as import("./studio-native.ts").FrameSource, conflicts: [] }];
  const app = mount(StudioSoundHarness, { target, props: { model: m, frame, onpatch: async () => patchId, onsfx: value => ops.push(value), onremove: () => {}, onremovepatch: () => {}, onnotice: () => {} } }); flushSync();
  try {
    [...target.querySelectorAll("button")].find(b => b.textContent === "Edit")!.click(); flushSync();
    assert.ok([...target.querySelectorAll("button")].find(b => b.textContent === "Save changes"));
    app.setFrame(id32(10)); flushSync();
    assert.equal([...target.querySelectorAll("button")].find(b => b.textContent === "Save changes"), undefined);
    const duration = target.querySelector<HTMLInputElement>('input[aria-label="Sound length in seconds"]')!;
    duration.value = "0.5"; duration.dispatchEvent(new w.Event("input", { bubbles: true })); flushSync();
    [...target.querySelectorAll("button")].find(b => b.textContent === "Add sound to frame")!.click(); flushSync();
    assert.equal(ops.length, 1); assert.equal(ops[0].id, undefined, "new frame receives a fresh marker, not the old frame's identity");
  } finally { await unmount(app); target.remove(); }
});

test("frame A→B→A cancels an outstanding emoji file read even when the final frame id matches", async () => {
  const target = document.createElement("div"); document.body.append(target); const read = deferred<string>(); let patches = 0;
  const app = mount(StudioSoundHarness, { target, props: { model: model(), frame, onpatch: async () => { patches++; return patchId; }, onsfx: () => {}, onremove: () => {}, onremovepatch: () => {}, onnotice: () => {} } }); flushSync();
  try {
    const file = target.querySelector<HTMLInputElement>('input[type="file"]')!;
    Object.defineProperty(file, "files", { value: [{ size: 500, text: () => read.promise }] }); file.dispatchEvent(new w.Event("change", { bubbles: true }));
    app.setFrame(id32(10)); flushSync(); app.setFrame(frame); flushSync();
    read.resolve(JSON.stringify({ v: 1, img: id64(3), patch, lo: 70, hi: 75, len: 333 }));
    for (let i = 0; i < 5; i++) await Promise.resolve(); flushSync(); assert.equal(patches, 0); assert.doesNotMatch(target.textContent!, /Imported sound/);
  } finally { await unmount(app); target.remove(); }
});
