import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { StudioAudioSession, type StudioAudioActions } from "./studio-audio.ts";
import { planJamArchivalPlayback } from "./jam-archival-plan.ts";
import type { JamArchivalPlayback } from "./jam-engine.ts";
import { decodePix } from "./pix.ts";
import { CHANNEL, ME, SERVER, b64, fakeIpc, flipnoteContent, fsource, id32, id64, indexContent, indexEntry, ordinaryView, pixBytes, reg } from "./studio-testkit.ts";
const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
Object.assign(globalThis, { MutationObserver: w.MutationObserver, HTMLInputElement: w.HTMLInputElement, HTMLSelectElement: w.HTMLSelectElement, HTMLCanvasElement: w.HTMLCanvasElement, HTMLMediaElement: w.HTMLMediaElement, MouseEvent: w.MouseEvent, Event: w.Event });
globalThis.getComputedStyle = w.getComputedStyle.bind(w);
// This test exercises actual editor input and Save bytes, not the browser's canvas renderer.
w.HTMLCanvasElement.prototype.getContext = (() => null) as typeof w.HTMLCanvasElement.prototype.getContext;
w.HTMLElement.prototype.setPointerCapture = () => {};
const { mount, unmount, flushSync } = await import("svelte");
const { default: Studio } = await import("./Studio.svelte");
const { default: StudioNav } = await import("./StudioNav.svelte");
const { default: StudioPlaybackHarness } = await import("../scripts/fixtures/StudioPlaybackHarness.svelte");
const { ensureStudio, useStudioIpc, disposeStudio, studio } = await import("./studio-state.svelte.ts");
const a = id32(1), b = id32(2), frame = id32(3), cid = id64(4);

test("editor Play starts all six frames and frame-one sound again after stopping on frame three", async () => {
  const ipc = fakeIpc(), pixels = pixBytes(), vector = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
  const ids = Array.from({ length: 6 }, (_, i) => id32(10 + i));
  const content = flipnoteContent({ fps: 2, frames: ids.map(id => ({ id, cid, bytes: pixels.length })) });
  content.sound = { records: {
    [`p/${vector.patchId}`]: reg({ op: "set_patch", patch: vector.patchId, descriptor: vector.patch }, fsource(1)),
    [`s/${id32(20)}`]: reg({ op: "set_sfx", sfx: id32(20), frame: ids[0], patch: vector.patchId, note: 60, duration_ms: 250 }, fsource(2)),
  }, readiness: { sfxOverflow: [], patchOverflow: [], exportOverflow: [], missingFrames: [], missingPatches: [], conflicts: false } };
  ipc.on("studio_list", () => ({ ...ordinaryView(indexContent()), group: "deadbeef" })); ipc.on("studio_read", () => ({ ...ordinaryView(content), group: "deadbeef" }));
  ipc.on("request_blob_bounded", () => ({ bytes: pixels.length, bytes_b64: b64(pixels) }));
  ipc.on("publish_pix", () => ({ cid: id64(8), bytes: pixels.length }));
  ipc.on("studio_apply", () => ({ ...ordinaryView({ ...content, frames: { ...content.frames,
    [ids[0]]: { ...content.frames[ids[0]] as object, pixels: reg({ cid: id64(8), bytes: pixels.length }, fsource(100)) } } }), group: "deadbeef" }));
  useStudioIpc(ipc, 5000); const session = ensureStudio(ME); session.setScope({ server: SERVER, channel: CHANNEL }); session.open(a); studio.selected = a;
  let now = 0;
  const plans = new Map<JamArchivalPlayback, ReturnType<typeof planJamArchivalPlayback>>(), attacks: { note: number | undefined; age: number }[] = [], offsets: number[] = [];
  const audio = new StudioAudioSession({ context: { state: "running", resume: async () => {} },
    prepareArchivalPlayback: async (take, animation) => { const capability = {} as JamArchivalPlayback; plans.set(capability, planJamArchivalPlayback(take, animation)); return capability; },
    archivalPlaybackPlan: capability => plans.get(capability) ?? null,
    playArchivalEvent: (capability, index, age = 0) => { const event = plans.get(capability)!.events[index]; if (event.kind === "on") attacks.push({ note: event.note, age }); return { ok: true }; },
    revokeArchivalPlayback: capability => plans.delete(capability),
  }, () => now);
  audio.setScope({ generation: 1, server: SERVER, channel: CHANNEL, membership: 1 }, false);
  let app: ReturnType<typeof mount<typeof StudioPlaybackHarness>>;
  const actions: StudioAudioActions = {
    start: async (kind, prepare, loop, offset = 0) => { offsets.push(offset); const started = await audio.start(kind, prepare, loop, offset); app.updateAudio(audio.state); return started; },
    stop: () => { audio.stop(); app.updateAudio(audio.state); },
  };
  const target = document.createElement("div"); document.body.append(target);
  app = mount(StudioPlaybackHarness, { target, props: { props: { me: ME, server: SERVER, channel: CHANNEL, nameOf: () => "member", colorOf: () => "#fff", onnotice: () => {}, audioActions: actions } } }); flushSync();
  const settle = async () => { for (let i = 0; i < 12; i++) { session.flush(); await new Promise(resolve => setImmediate(resolve)); flushSync(); } };
  try {
    await settle();
    session.saveFrame(a, ids[0], pixels);
    target.querySelector<HTMLButtonElement>('[title="next frame [.]"]')!.click(); target.querySelector<HTMLButtonElement>('[title="next frame [.]"]')!.click(); flushSync();
    for (let run = 0; run < 2; run++) {
      target.querySelector<HTMLButtonElement>('[title="play [space]"]')!.click();
      // Native WebCrypto preparation completes on a worker, not after a fixed number of
      // microtask/immediate turns. Wait for the actual capability before asserting attacks;
      // the injected audio clock stays fixed, so preparation latency cannot age the note.
      const deadline = performance.now() + 5000;
      do { await settle(); if (audio.state.state === "playing") break; await new Promise(resolve => setTimeout(resolve, 1)); }
      while (performance.now() < deadline);
      assert.equal(audio.state.state, "playing", `Play ${run + 1} must finish preparation`);
      await audio.tick(); app.updateAudio(audio.state); flushSync();
      assert.equal(offsets[run], 0); assert.deepEqual(attacks[run], { note: 60, age: 0 });
      assert.match(target.querySelector(".st-transport .mono.strong")!.textContent!, /1 \/ 6/);
      if (run === 0) { await session.save(); await settle(); assert.equal(audio.state.state, "playing", "saving the same draft must keep playback running"); }
      now += 1000; await audio.tick(); app.updateAudio(audio.state); flushSync();
      assert.match(target.querySelector(".st-transport .mono.strong")!.textContent!, /3 \/ 6/);
      target.querySelector<HTMLButtonElement>('[title="stop [space]"]')!.click(); flushSync();
    }
  } finally { audio.stop(); await unmount(app); disposeStudio(); target.remove(); }
});

test("the mounted editor saves drawn pixels to A before sidebar B opens; refusal retains A", async () => {
  for (const refuse of [false, true]) {
    const ipc = fakeIpc(), pixels = pixBytes(), notices: string[] = []; let starts = 0;
    ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [a]: indexEntry({ title: "A" }), [b]: indexEntry({ title: "B" }) } })));
    ipc.on("studio_read", () => ordinaryView(flipnoteContent({ frames: [{ id: frame, cid, bytes: pixels.length }] })));
    ipc.on("request_blob_bounded", () => ({ bytes: pixels.length, bytes_b64: b64(pixels) }));
    ipc.on("publish_pix", args => ({ cid: id64(5), bytes: atob(String(args.bytesB64)).length }));
    ipc.on("studio_apply", () => ordinaryView(flipnoteContent({ frames: [{ id: frame, cid: id64(5), bytes: pixels.length }] })));
    useStudioIpc(ipc, 5000); const session = ensureStudio(ME);
    const settle = async () => { for (let i = 0; i < 12; i++) { session.flush(); await new Promise(resolve => setImmediate(resolve)); flushSync(); } };
    const target = document.createElement("div"); document.body.append(target);
    const nav = mount(StudioNav, { target, props: { me: ME, server: SERVER, channel: CHANNEL, onopen: () => {}, onnotice: (text: string) => notices.push(text) } });
    const editor = mount(Studio, { target, props: { me: ME, server: SERVER, channel: CHANNEL, nameOf: () => "member", colorOf: () => "#fff", onnotice: (text: string) => notices.push(text),
      audioActions: { start: async () => { starts++; return true; }, stop: () => {} } } }); flushSync();
    try {
      await settle();
      const choose = (id: string) => { const button = [...target.querySelectorAll<HTMLButtonElement>(".studio-obj")].find(n => n.querySelector(".nm")?.textContent === id); assert.ok(button); button.click(); flushSync(); };
      choose("A"); await settle();
      const canvas = target.querySelector<HTMLCanvasElement>(".st-canvas")!, overlay = target.querySelector<HTMLCanvasElement>(".st-overlay")!;
      assert.ok(canvas && overlay, target.textContent!); canvas.getBoundingClientRect = () => ({ x: 0, y: 0, left: 0, top: 0, right: 192, bottom: 144, width: 192, height: 144, toJSON() {} });
      overlay.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0, clientX: 10, clientY: 10 })); flushSync();
      assert.match(target.textContent!, /unsaved/);
      const original = session.saveFrame;
      if (refuse) session.saveFrame = () => { throw new Error("injected enqueue refusal"); };
      const deleteButton = () => [...target.querySelectorAll<HTMLButtonElement>(".st-btn")].find(button => button.textContent?.trim() === "delete")!;
      if (refuse) {
        target.querySelector<HTMLButtonElement>('[title="play [space]"]')!.click(); flushSync(); assert.equal(starts, 0, "Play cannot substitute the old saved frame after staging refused");
        deleteButton().click(); flushSync(); assert.equal(target.querySelector(".st-canvas"), canvas, "Delete cannot drop a raster whose staging refused");
        assert.equal(session.saves.length, 0);
      }
      choose("B");
      await settle();
      if (refuse) {
        assert.equal(studio.selected, a); assert.equal(session.doc?.object, a);
        assert.match(target.textContent!, /unsaved/); assert.ok(notices.some(n => /enqueue refusal/.test(n)));
        session.saveFrame = original;
        const originalRemove = session.removeFrame; session.removeFrame = () => { throw new Error("injected removal refusal"); };
        deleteButton().click(); flushSync(); assert.equal(target.querySelector(".st-canvas"), canvas, "a refused removal keeps the current raster");
        assert.equal(session.workingModel?.frames[0].id, frame);
        session.removeFrame = originalRemove; choose("B");
      }
      await settle();
      assert.equal(studio.selected, b, notices.join("; ")); assert.equal(session.doc?.object, b);
      const publication = ipc.calls.find(c => c.cmd === "publish_pix"); assert.ok(publication);
      const actual = decodePix(Uint8Array.from(atob(String(publication.args.bytesB64)), c => c.charCodeAt(0)));
      assert.notEqual(actual.pixels[10 + 10 * 192], 0, "the actual painted cell survives switching");
      assert.equal(ipc.calls.find(c => c.cmd === "studio_apply")?.args.object, a);
      assert.equal(target.querySelectorAll(".studio-chip").length, 1, "only the current editor is open");
    } finally { await unmount(editor); await unmount(nav); disposeStudio(); target.remove(); }
  }
});
