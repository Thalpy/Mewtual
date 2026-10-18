import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { StudioAudioSession, type StudioAudioActions } from "./studio-audio.ts";
import { planJamArchivalPlayback } from "./jam-archival-plan.ts";
import type { JamArchivalPlayback } from "./jam-engine.ts";
import { decodePix } from "./pix.ts";
import { CHANNEL, ME, SERVER, b64, deferred, fakeIpc, flipnoteContent, fsource, id32, id64, indexContent, indexEntry, ordinaryView, pixBytes, reg } from "./studio-testkit.ts";
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
const { ensureStudio, useStudioIpc, disposeStudio, studio, studioDraftSnapshot, resetStudio, restoreStudioDraft, saveStudio } = await import("./studio-state.svelte.ts");
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

// Review of 149e7321, finding 1: lock/unlock restores held work with no document open. Opening that
// document used to require a successful departure Save of the very work whose Save, Retry and
// discard controls live inside it, so the sidebar click did nothing and the controls were gone.
test("a restored queued or uncertain edit reopens through the sidebar and waits for an explicit Save or retry", async () => {
  for (const restored of ["queued", "uncertain"] as const) {
    const ipc = fakeIpc(), pixels = pixBytes(), notices: string[] = [], held = deferred<unknown>();
    let fps = 12, holdApply = restored === "uncertain";
    const view = () => ({ ...ordinaryView(flipnoteContent({ fps, frames: [{ id: frame, cid, bytes: pixels.length }] })), group: "deadbeef" });
    ipc.on("studio_list", () => ({ ...ordinaryView(indexContent({ objects: { [a]: indexEntry({ title: "A" }) } })), group: "deadbeef" }));
    ipc.on("studio_read", view);
    ipc.on("request_blob_bounded", () => ({ bytes: pixels.length, bytes_b64: b64(pixels) }));
    ipc.on("studio_apply", args => {
      if (holdApply) return held.promise; // still awaiting its receipt when the lock lands
      const op = JSON.parse(String(args.body)); if (op.op === "set_header" && op.field === "fps") fps = op.value;
      return view();
    });
    useStudioIpc(ipc, 5000); const session = ensureStudio(ME);
    const settle = async () => { for (let i = 0; i < 12; i++) { session.flush(); await new Promise(resolve => setImmediate(resolve)); flushSync(); } };
    const target = document.createElement("div"); document.body.append(target);
    const surfaces = () => [
      mount(StudioNav, { target, props: { me: ME, server: SERVER, channel: CHANNEL, onopen: () => {}, onnotice: (text: string) => notices.push(text) } }),
      mount(Studio, { target, props: { me: ME, server: SERVER, channel: CHANNEL, nameOf: () => "member", colorOf: () => "#fff", onnotice: (text: string) => notices.push(text) } }),
    ];
    let mounted = surfaces(); flushSync();
    const choose = (title: string) => { const button = [...target.querySelectorAll<HTMLButtonElement>(".studio-obj")].find(n => n.querySelector(".nm")?.textContent === title); assert.ok(button, target.textContent!); button.click(); flushSync(); };
    const button = (label: string) => [...target.querySelectorAll<HTMLButtonElement>("button")].find(n => n.textContent?.trim() === label);
    const applies = () => ipc.calls.filter(c => c.cmd === "studio_apply");
    try {
      await settle(); choose("A"); await settle();
      session.setFps(a, 7);
      if (restored === "uncertain") { void session.save().catch(() => {}); await settle(); assert.equal(session.saves[0].status, "inflight"); }
      // Lock: App captures the draft, the whole surface goes, and the session forgets everything.
      const json = studioDraftSnapshot(); assert.ok(json);
      for (const s of mounted) await unmount(s);
      resetStudio(); holdApply = false;
      // Unlock: the draft is handed back and the surfaces remount on the same channel.
      restoreStudioDraft(json); mounted = surfaces(); flushSync(); await settle();
      const sent = applies().length;
      assert.equal(studio.selected, ""); assert.equal(session.doc, null);
      assert.equal(session.saves.length, 1); assert.equal(session.saves[0].status, restored);
      assert.ok(target.querySelector(`[data-held-object="${a}"]`), "the sidebar names the document holding the restored edit");
      choose("A"); await settle();
      assert.equal(studio.selected, a, notices.join("; ")); assert.equal(session.doc?.object, a);
      assert.equal(target.querySelector("[data-held-object]"), null, "the open document is no longer held elsewhere");
      assert.equal(applies().length, sent, "opening the document never replays restored work");
      assert.equal(session.saves.length, 1); assert.equal(session.saves[0].status, restored);
      if (restored === "queued") {
        assert.match(target.textContent!, /Unsaved changes/);
        button("Save")!.click(); flushSync(); await settle();
      } else {
        assert.match(target.textContent!, /save uncertain/); assert.ok(button("discard"), "discard stays reachable");
        button("retry same request")!.click(); flushSync(); await settle();
        assert.deepEqual(applies().at(-1)!.args, applies()[0].args, "retry resends the exact interrupted request");
      }
      assert.equal(applies().length, sent + 1); assert.equal(session.saves.length, 0, notices.join("; "));
      assert.equal(session.doc?.model?.fps, 7);
    } finally { for (const s of mounted) await unmount(s); disposeStudio(); target.remove(); }
  }
});

test("held work for a document the Index does not list is still one sidebar click from its Save", async () => {
  const ipc = fakeIpc(), pixels = pixBytes(), notices: string[] = [];
  const view = (fps = 12) => ({ ...ordinaryView(flipnoteContent({ fps, frames: [{ id: frame, cid, bytes: pixels.length }] })), group: "deadbeef" });
  let fps = 12;
  ipc.on("studio_list", () => ({ ...ordinaryView(indexContent()), group: "deadbeef" }));
  ipc.on("studio_read", () => view(fps));
  ipc.on("request_blob_bounded", () => ({ bytes: pixels.length, bytes_b64: b64(pixels) }));
  ipc.on("studio_apply", args => { const op = JSON.parse(String(args.body)); if (op.field === "fps") fps = op.value; return view(fps); });
  useStudioIpc(ipc, 5000); const session = ensureStudio(ME);
  const settle = async () => { for (let i = 0; i < 12; i++) { session.flush(); await new Promise(resolve => setImmediate(resolve)); flushSync(); } };
  const target = document.createElement("div"); document.body.append(target);
  const surfaces = () => [
    mount(StudioNav, { target, props: { me: ME, server: SERVER, channel: CHANNEL, onopen: () => {}, onnotice: (text: string) => notices.push(text) } }),
    mount(Studio, { target, props: { me: ME, server: SERVER, channel: CHANNEL, nameOf: () => "member", colorOf: () => "#fff", onnotice: (text: string) => notices.push(text) } }),
  ];
  let mounted = surfaces(); flushSync();
  try {
    await settle(); session.open(b); studio.selected = b; flushSync(); await settle();
    session.setFps(b, 9);
    const json = studioDraftSnapshot()!;
    for (const s of mounted) await unmount(s);
    resetStudio(); restoreStudioDraft(json); mounted = surfaces(); flushSync(); await settle();
    assert.equal(target.querySelectorAll(".studio-obj").length, 0, "the Index lists nothing to click");
    const row = target.querySelector<HTMLElement>(`[data-held-object="${b}"]`);
    assert.ok(row, target.textContent!); assert.match(row.textContent!, /unsaved edits · untitled flipnote/);
    row.querySelector("button")!.click(); flushSync(); await settle();
    assert.equal(session.doc?.object, b, notices.join("; ")); assert.equal(studio.selected, b);
    assert.equal(ipc.calls.filter(c => c.cmd === "studio_apply").length, 0);
    [...target.querySelectorAll<HTMLButtonElement>("button")].find(n => n.textContent?.trim() === "Save")!.click(); flushSync(); await settle();
    assert.equal(session.saves.length, 0, notices.join("; ")); assert.equal(fps, 9);
  } finally { for (const s of mounted) await unmount(s); disposeStudio(); target.remove(); }
});

// Re-review of 13e338eb, finding 2: with the document read failing, the editor showed only the read
// error and its read retry, while the exact-request retry and discard lived in another branch, so a
// restored queue that vetoed leaving could not be resolved from the editor at all.
test("with the document read failing, restored queued and uncertain edits keep their Save, retry and discard", async () => {
  for (const restored of ["uncertain", "queued-discard", "queued-save"] as const) {
    const ipc = fakeIpc(), pixels = pixBytes(), notices: string[] = [], held = deferred<unknown>();
    let readsFail = false, holdApply = restored === "uncertain";
    ipc.on("studio_list", () => ({ ...ordinaryView(indexContent({ objects: { [a]: indexEntry({ title: "A" }) } })), group: "deadbeef" }));
    ipc.on("studio_read", () => { if (readsFail) throw new Error("actor busy"); return { ...ordinaryView(flipnoteContent({ frames: [{ id: frame, cid, bytes: pixels.length }] })), group: "deadbeef" }; });
    ipc.on("request_blob_bounded", () => ({ bytes: pixels.length, bytes_b64: b64(pixels) }));
    ipc.on("studio_apply", () => holdApply ? held.promise : assert.fail("nothing may be sent while the document cannot be read"));
    useStudioIpc(ipc, 5000); const session = ensureStudio(ME);
    const settle = async () => { for (let i = 0; i < 12; i++) { session.flush(); await new Promise(resolve => setImmediate(resolve)); flushSync(); } };
    const target = document.createElement("div"); document.body.append(target);
    const surfaces = () => [
      mount(StudioNav, { target, props: { me: ME, server: SERVER, channel: CHANNEL, onopen: () => {}, onnotice: (text: string) => notices.push(text) } }),
      mount(Studio, { target, props: { me: ME, server: SERVER, channel: CHANNEL, nameOf: () => "member", colorOf: () => "#fff", onnotice: (text: string) => notices.push(text) } }),
    ];
    let mounted = surfaces(); flushSync();
    const choose = (title: string) => { const button = [...target.querySelectorAll<HTMLButtonElement>(".studio-obj")].find(n => n.querySelector(".nm")?.textContent === title); assert.ok(button, target.textContent!); button.click(); flushSync(); };
    const within = (selector: string, label: string) => [...target.querySelectorAll<HTMLButtonElement>(`${selector} button`)].find(n => n.textContent?.trim() === label);
    try {
      await settle(); choose("A"); await settle();
      session.setFps(a, 7);
      if (restored === "uncertain") { void session.save().catch(() => {}); await settle(); }
      const json = studioDraftSnapshot()!;
      for (const s of mounted) await unmount(s);
      resetStudio(); holdApply = false; readsFail = true;
      restoreStudioDraft(json); mounted = surfaces(); flushSync(); await settle();
      choose("A"); await settle();
      assert.equal(session.doc?.object, a, notices.join("; ")); assert.match(target.textContent!, /could not read this flipnote: actor busy/);
      if (restored === "uncertain") {
        assert.ok(within("[data-held-save]", "retry same request"), target.textContent!);
        within("[data-held-save]", "discard")!.click(); flushSync(); within("[data-held-save]", "discard for real")!.click(); flushSync();
      } else {
        assert.ok(within("[data-held-queued]", "Save"), target.textContent!);
        if (restored === "queued-discard") {
          within("[data-held-queued]", "discard")!.click(); flushSync(); within("[data-held-queued]", "discard for real")!.click(); flushSync();
        } else {
          // Save cannot read the document either: refused as not written, and the refusal's own
          // retry and discard appear beside the read error rather than replacing it. A discard
          // armed before the Save does not carry over to the record's new state.
          within("[data-held-queued]", "discard")!.click(); flushSync();
          within("[data-held-queued]", "Save")!.click(); flushSync(); await settle();
          assert.equal(session.saves[0].status, "blocked"); assert.match(target.textContent!, /could not read this flipnote/);
          assert.ok(within("[data-held-save]", "retry same request"), target.textContent!);
          assert.equal(within("[data-held-save]", "discard for real"), undefined, "the earlier arming does not survive the status change");
          within("[data-held-save]", "discard")!.click(); flushSync(); within("[data-held-save]", "discard for real")!.click(); flushSync();
        }
      }
      await settle();
      assert.equal(session.saves.length, 0, `${restored}: the edit was resolved from the editor`);
      await saveStudio(); // leaving is no longer vetoed
    } finally { for (const s of mounted) await unmount(s); disposeStudio(); target.remove(); }
  }
});

// Adversarial review of the read-error fix: the held card's Save called session.save(), which sent
// every document's restored work (held precisely until the member acts on it) and was refused by
// any other document's uncertain record.
test("a document's Save sends only that document's restored edits; other documents' stay held", async () => {
  const c = id32(6), ipc = fakeIpc(), pixels = pixBytes(), notices: string[] = [];
  ipc.on("studio_list", () => ({ ...ordinaryView(indexContent({ objects: { [a]: indexEntry({ title: "A" }), [b]: indexEntry({ title: "B" }), [c]: indexEntry({ title: "C" }) } })), group: "deadbeef" }));
  ipc.on("studio_read", () => ({ ...ordinaryView(flipnoteContent({ frames: [{ id: frame, cid, bytes: pixels.length }] })), group: "deadbeef" }));
  ipc.on("request_blob_bounded", () => ({ bytes: pixels.length, bytes_b64: b64(pixels) }));
  ipc.on("studio_apply", () => ({ ...ordinaryView(flipnoteContent({ frames: [{ id: frame, cid, bytes: pixels.length }] })), group: "deadbeef" }));
  useStudioIpc(ipc, 5000); const session = ensureStudio(ME);
  const settle = async () => { for (let i = 0; i < 12; i++) { session.flush(); await new Promise(resolve => setImmediate(resolve)); flushSync(); } };
  const target = document.createElement("div"); document.body.append(target);
  const surfaces = () => [
    mount(StudioNav, { target, props: { me: ME, server: SERVER, channel: CHANNEL, onopen: () => {}, onnotice: (text: string) => notices.push(text) } }),
    mount(Studio, { target, props: { me: ME, server: SERVER, channel: CHANNEL, nameOf: () => "member", colorOf: () => "#fff", onnotice: (text: string) => notices.push(text) } }),
  ];
  let mounted = surfaces(); flushSync();
  const choose = (title: string) => { const button = [...target.querySelectorAll<HTMLButtonElement>(".studio-obj")].find(n => n.querySelector(".nm")?.textContent === title); assert.ok(button, target.textContent!); button.click(); flushSync(); };
  try {
    await settle();
    for (const [id, fps] of [[a, 7], [b, 8], [c, 9]] as const) { session.open(id); await settle(); session.setFps(id, fps); }
    // C was being delivered when the lock landed, so it restores as uncertain.
    const draft = JSON.parse(studioDraftSnapshot()!);
    const interrupted = draft.records.find((r: { object: string }) => r.object === c); interrupted.status = "inflight"; interrupted.attempts = 1;
    for (const s of mounted) await unmount(s);
    resetStudio(); restoreStudioDraft(JSON.stringify(draft)); mounted = surfaces(); flushSync(); await settle();
    choose("A"); await settle();
    const card = target.querySelector("[data-held-queued]"); assert.ok(card, target.textContent!);
    [...card.querySelectorAll<HTMLButtonElement>("button")].find(n => n.textContent?.trim() === "Save")!.click(); flushSync(); await settle();
    assert.deepEqual(ipc.calls.filter(x => x.cmd === "studio_apply").map(x => x.args.object), [a], "only A's edit was sent");
    const left = session.saves.map(s => [s.object, s.status]);
    assert.deepEqual(left, [[b, "queued"], [c, "uncertain"]]);
    assert.equal(session.awaitsExplicitSave(session.saves[0].id), true, "B is still held for its own action");
    assert.ok(!notices.some(n => /Could not confirm the save/.test(n)), notices.join("; "));
  } finally { for (const s of mounted) await unmount(s); disposeStudio(); target.remove(); }
});
