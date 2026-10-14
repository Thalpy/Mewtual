import assert from "node:assert/strict";
import test from "node:test";
import { legacyJamPatch } from "./jam-patch.ts";
import { studioSoundModel } from "./studio-sound.ts";
import { deferred, fsource, id32, id64, reg } from "./studio-testkit.ts";
const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
Object.assign(globalThis, { MutationObserver: w.MutationObserver, HTMLInputElement: w.HTMLInputElement, HTMLMediaElement: w.HTMLMediaElement, Event: w.Event });
const { mount, unmount, flushSync } = await import("svelte");
const { default: StudioSound } = await import("./StudioSound.svelte");
const patchId = id64(1), frame = id32(2), patch = legacyJamPatch("sine");
function model() { return studioSoundModel({ records: { [`p/${patchId}`]: reg({ op: "set_patch", patch: patchId, descriptor: patch }, fsource(1)) },
  readiness: { sfxOverflow: [], patchOverflow: [], exportOverflow: [], missingFrames: [], missingPatches: [], conflicts: false } }); }
test("Sound panel does not invent a hold and preserves an explicit creator duration", async () => {
  const target = document.createElement("div"); document.body.append(target); const ops: unknown[] = [];
  const app = mount(StudioSound, { target, props: { model: model(), frame, onpatch: async () => patchId, onsfx: (value) => ops.push(value), onremove: () => {}, onremovepatch: () => {}, onnotice: () => {} } }); flushSync();
  try {
    const add = [...target.querySelectorAll("button")].find(b => b.textContent === "add sound marker")!; assert.equal(add.disabled, true);
    const select = target.querySelector("select")!; select.value = patchId; select.dispatchEvent(new w.Event("change", { bubbles: true }));
    const duration = target.querySelector<HTMLInputElement>('input[placeholder="from the sound creator"]')!;
    assert.equal(duration.value, ""); duration.value = "95000"; duration.dispatchEvent(new w.Event("input", { bubbles: true })); flushSync(); add.click(); flushSync();
    assert.deepEqual(ops, [{ patch: patchId, note: 60, durationMs: 95000 }]);
  } finally { await unmount(app); target.remove(); }
});
test("emoji file source preserves validated length and does not claim a verified shared catalog", async () => {
  const target = document.createElement("div"); document.body.append(target); const ops: unknown[] = [], notices: string[] = [];
  const app = mount(StudioSound, { target, props: { model: model(), frame, onpatch: async () => patchId, onsfx: (value) => ops.push(value), onremove: () => {}, onremovepatch: () => {}, onnotice: (text) => notices.push(text) } }); flushSync();
  try {
    const file = target.querySelector<HTMLInputElement>('input[type="file"]')!;
    Object.defineProperty(file, "files", { value: [{ size: 500, text: async () => JSON.stringify({ v: 1, img: id64(3), patch, lo: 70, hi: 75, len: 333 }) }] });
    file.dispatchEvent(new w.Event("change", { bubbles: true })); for (let i = 0; i < 5; i++) await Promise.resolve(); flushSync();
    assert.match(target.textContent!, /authored hold 333ms/); assert.match(target.textContent!, /no shared emoji catalog/);
    assert.equal(target.querySelector<HTMLInputElement>('input[placeholder="from the sound creator"]')!.disabled, true);
    [...target.querySelectorAll("button")].find(b => b.textContent === "add sound marker")!.click(); flushSync();
    assert.deepEqual(ops, [{ patch: patchId, note: 70, durationMs: 333 }]); assert.deepEqual(notices, []);
  } finally { await unmount(app); target.remove(); }
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
