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
const { ensureStudio, useStudioIpc, disposeStudio, studio, studioDraftSnapshot, resetStudio, restoreStudioDraft, saveStudio, sendStudio, claimStudioNavigation, beginStudioClose, endStudioClose, setStudioScope } = await import("./studio-state.svelte.ts");
const a = id32(1), b = id32(2), frame = id32(3), cid = id64(4);

test("an external scope request cannot submit restored held work without an explicit Save", async () => {
  const ipc = fakeIpc();
  const view = () => ({ ...ordinaryView(flipnoteContent({ frames: [] })), group: "deadbeef" });
  ipc.on("studio_list", () => ({ ...ordinaryView(indexContent({ objects: { [a]: indexEntry({ title: "A" }) } })), group: "deadbeef" }));
  ipc.on("studio_read", view); ipc.on("studio_apply", view);
  useStudioIpc(ipc, 5000);
  const session = ensureStudio(ME);
  const settle = async (turns = 10) => { for (let i = 0; i < turns; i++) {
    session.flush(); await new Promise(resolve => setImmediate(resolve)); flushSync();
  } };
  const target = document.createElement("div"); document.body.append(target);
  let editor: ReturnType<typeof mount<typeof Studio>> | null = null;
  try {
    session.setScope({ server: SERVER, channel: CHANNEL }); await settle(10); session.open(a); await settle(10);
    assert.ok(session.doc?.model, JSON.stringify(session.doc));
    session.setFps(a, 7);
    const draft = studioDraftSnapshot(); assert.ok(draft);
    resetStudio(); restoreStudioDraft(draft);
    assert.equal(setStudioScope(SERVER, CHANNEL), true); await settle(10);
    assert.ok(session.saves.length);
    assert.equal(session.awaitsExplicitSave(session.saves[0].id), true);
    const original = session.saves[0]; assert.ok(original.kind === "apply");
    assert.equal(setStudioScope(SERVER, "77777777777777777"), false);
    editor = mount(Studio, { target, props: { me: ME, server: SERVER, channel: "77777777777777777",
      nameOf: () => "member", colorOf: () => "#fff", onnotice: () => {} } }); flushSync();
    await settle(10);
    assert.equal(ipc.calls.filter(c => c.cmd === "studio_apply" || c.cmd === "studio_apply_batch").length, 0,
      "mounting another channel does not grant permission to send restored edits");
    assert.equal(session.scope?.channel, CHANNEL);
    const waiting = target.querySelector('[data-scope-wait]'); assert.ok(waiting);
    assert.match(waiting.textContent!, /Unsaved edits.*waiting/);
    assert.doesNotMatch(waiting.textContent!, /Saving your edits/);
    const save = target.querySelector<HTMLButtonElement>('[data-save-held-scope]'); assert.ok(save);
    const actualSave = session.save.bind(session); let waiters = 0;
    session.save = () => {
      waiters++;
      if (waiters === 1) save.click(); // reentrant click before Save stages/releases the records
      return actualSave();
    };
    save.click(); flushSync(); await settle(15);
    assert.equal(waiters, 1, "reentrant clicks cannot create more Save waiters");
    const sent = ipc.calls.filter(c => c.cmd === "studio_apply"); assert.equal(sent.length, 1);
    assert.equal(sent[0].args.channel, CHANNEL, "explicit Save writes the original channel");
    assert.equal(sent[0].args.nonce, original.request.nonce, "it retains the original request identity");
    assert.equal(session.scope?.channel, "77777777777777777");
  } finally { if (editor) await unmount(editor); disposeStudio(); target.remove(); }
});

test("a held-scope Save completion cannot notify a new destination, reset or destroyed panel", async () => {
  const { default: Harness } = await import("../scripts/fixtures/StudioDestinationHarness.svelte");
  for (const replacement of ["destination", "reset", "unmount"] as const) {
    const ipc = fakeIpc(), notices: string[] = [], ack = deferred<unknown>();
    const view = () => ({ ...ordinaryView(flipnoteContent({ frames: [] })), group: "deadbeef" });
    ipc.on("studio_list", () => ({ ...ordinaryView(indexContent({ objects: { [a]: indexEntry({ title: "A" }) } })), group: "deadbeef" }));
    ipc.on("studio_read", view); ipc.on("studio_apply", () => ack.promise);
    useStudioIpc(ipc, 5000); const session = ensureStudio(ME);
    const settle = async () => { for (let i = 0; i < 15; i++) { session.flush(); await new Promise(resolve => setImmediate(resolve)); flushSync(); } };
    const target = document.createElement("div"); document.body.append(target);
    let mounted: ReturnType<typeof mount<typeof Harness>> | null = null;
    try {
      session.setScope({ server: SERVER, channel: CHANNEL }); await settle(); session.open(a); await settle();
      session.setFps(a, 7); const draft = studioDraftSnapshot(); assert.ok(draft);
      resetStudio(); restoreStudioDraft(draft); setStudioScope(SERVER, CHANNEL); await settle();
      mounted = mount(Harness, { target, props: { props: { me: ME, server: SERVER, channel: "77777777777777777",
        nameOf: () => "member", colorOf: () => "#fff", onnotice: text => notices.push(text) } } }); flushSync(); await settle();
      const save = target.querySelector<HTMLButtonElement>('[data-save-held-scope]'); assert.ok(save);
      save.click(); flushSync(); await settle();
      assert.equal(ipc.calls.filter(c => c.cmd === "studio_apply").length, 1);
      if (replacement === "destination") {
        const generation = session.generation;
        (mounted as unknown as { destination(server: number, channel: string): void }).destination(SERVER, "88888888888888888");
        flushSync(); assert.equal(session.generation, generation, "target-only change leaves the source generation intact");
      } else if (replacement === "reset") resetStudio();
      else { await unmount(mounted); mounted = null; }
      ack.reject(new Error("late save failure")); await settle();
      assert.deepEqual(notices, [], `${replacement}: the old waiter cannot publish into a replacement surface`);
      assert.equal(ipc.calls.filter(c => c.cmd === "studio_apply").length, 1, "no new nonce or automatic replacement Save");
    } finally { if (mounted) await unmount(mounted); disposeStudio(); target.remove(); }
  }
});

test("saving a mixed held and blocked scope never retries or reauthors the blocked request", async () => {
  for (const failure of ["epoch studio: edit belongs to a retired epoch", "Studio transaction expired, cancelled or server stopped"]) {
  const ipc = fakeIpc(), notices: string[] = [];
  const view = () => ({ ...ordinaryView(flipnoteContent({ frames: [] })), group: "deadbeef" });
  ipc.on("studio_list", () => ({ ...ordinaryView(indexContent({ objects: { [a]: indexEntry({ title: "A" }) } })), group: "deadbeef" }));
  ipc.on("studio_read", view);
  ipc.on("studio_apply", () => { throw new Error(failure); });
  useStudioIpc(ipc, 5000); const session = ensureStudio(ME);
  const settle = async () => { for (let i = 0; i < 15; i++) { session.flush(); await new Promise(resolve => setImmediate(resolve)); flushSync(); } };
  const target = document.createElement("div"); document.body.append(target);
  let mounted: ReturnType<typeof mount<typeof Studio>> | null = null;
  try {
    session.setScope({ server: SERVER, channel: CHANNEL }); await settle(); session.open(a); await settle();
    session.setFps(a, 7); await session.save().catch(() => {}); session.setFps(a, 8);
    const draft = studioDraftSnapshot(); assert.ok(draft);
    resetStudio(); restoreStudioDraft(draft); setStudioScope(SERVER, CHANNEL); await settle();
    const identities = session.saves.map(s => s.kind === "apply" ? s.request.nonce : null);
    assert.equal(session.needsDecision(session.saves[0].id), failure.includes("retired epoch"));
    assert.equal(session.awaitsExplicitSave(session.saves[1].id), true);
    mounted = mount(Studio, { target, props: { me: ME, server: SERVER, channel: "77777777777777777",
      nameOf: () => "member", colorOf: () => "#fff", onnotice: text => notices.push(text) } }); flushSync(); await settle();
    assert.match(target.querySelector('[data-scope-wait]')!.textContent!, /Unsaved edits.*waiting/);
    const before = ipc.calls.filter(c => c.cmd === "studio_apply" || c.cmd === "studio_apply_batch").length;
    target.querySelector<HTMLButtonElement>('[data-save-held-scope]')!.click(); flushSync(); await settle();
    assert.equal(ipc.calls.filter(c => c.cmd === "studio_apply" || c.cmd === "studio_apply_batch").length, before);
    assert.deepEqual(session.saves.map(s => s.kind === "apply" ? s.request.nonce : null), identities);
    assert.equal(session.scope?.channel, CHANNEL, "a blocked original prevents the switch");
    assert.equal(notices.length, 1, "the current panel reports that Save could not finish");
    assert.match(notices[0], /not saved|not confirmed/);
  } finally { if (mounted) await unmount(mounted); disposeStudio(); target.remove(); }
  }
});

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
        assert.match(target.textContent!, /Not confirmed: fps/); assert.ok(button("Discard"), "discard stays reachable");
        assert.doesNotMatch(target.textContent!, /did not go through/, "an interrupted delivery may have landed; the words must not say it failed");
        button("Retry now")!.click(); flushSync(); await settle();
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
    assert.ok(row, target.textContent!); assert.match(row.textContent!, /untitled flipnote: unsaved edits/);
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
        assert.ok(within("[data-held-save]", "Retry now"), target.textContent!);
        within("[data-held-save]", "Discard")!.click(); flushSync(); within("[data-held-save]", "Really discard?")!.click(); flushSync();
      } else {
        assert.ok(within("[data-held-queued]", "Save"), target.textContent!);
        if (restored === "queued-discard") {
          within("[data-held-queued]", "Discard")!.click(); flushSync(); within("[data-held-queued]", "Really discard?")!.click(); flushSync();
        } else {
          // Save cannot read the document either: refused as not written, and the refusal's own
          // retry and discard appear beside the read error rather than replacing it. A discard
          // armed before the Save does not carry over to the record's new state.
          within("[data-held-queued]", "Discard")!.click(); flushSync();
          within("[data-held-queued]", "Save")!.click(); flushSync(); await settle();
          assert.equal(session.saves[0].status, "blocked"); assert.match(target.textContent!, /could not read this flipnote/);
          assert.ok(within("[data-held-save]", "Retry now"), target.textContent!);
          assert.equal(within("[data-held-save]", "Really discard?"), undefined, "the earlier arming does not survive the status change");
          within("[data-held-save]", "Discard")!.click(); flushSync(); within("[data-held-save]", "Really discard?")!.click(); flushSync();
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
    // D holds only an Index edit about it: the sidebar must still list D as holding work.
    const d = id32(7); session.setEntryExpiry(d, { kind: "never" });
    // C was being delivered when the lock landed, so it restores as uncertain.
    const draft = JSON.parse(studioDraftSnapshot()!);
    const interrupted = draft.records.find((r: { object: string }) => r.object === c); interrupted.status = "inflight"; interrupted.attempts = 1;
    for (const s of mounted) await unmount(s);
    resetStudio(); restoreStudioDraft(JSON.stringify(draft)); mounted = surfaces(); flushSync(); await settle();
    choose("A"); await settle();
    assert.ok(target.querySelector(`[data-held-object="${d}"]`), "an Index-only held edit is listed under the document it changes");
    const card = target.querySelector("[data-held-queued]"); assert.ok(card, target.textContent!);
    [...card.querySelectorAll<HTMLButtonElement>("button")].find(n => n.textContent?.trim() === "Save")!.click(); flushSync(); await settle();
    assert.deepEqual(ipc.calls.filter(x => x.cmd === "studio_apply").map(x => x.args.object), [a], "only A's edit was sent");
    const left = session.saves.map(s => [s.object, s.status]);
    assert.deepEqual(left, [[b, "queued"], [c, "uncertain"], [d, "queued"]]);
    assert.equal(session.awaitsExplicitSave(session.saves[0].id), true, "B is still held for its own action");
    assert.ok(!notices.some(n => /is not confirmed|was not saved|not saved yet/.test(n)), notices.join("; "));
  } finally { for (const s of mounted) await unmount(s); disposeStudio(); target.remove(); }
});

// Re-review of the d2548a37 fix, MEDIUM 1: edits kept for a server that stopped wait for a channel
// that cannot open until Mewtual restarts (a lock does not restart its actor), and while they wait
// every new Studio edit is refused. The sidebar offers an armed discard, so the member is not stuck.
test("the sidebar discards a restored draft still waiting for its channel, and edits are accepted again", async () => {
  const ipc = fakeIpc(), notices: string[] = [];
  ipc.on("studio_list", () => ({ ...ordinaryView(indexContent()), group: "deadbeef" }));
  ipc.on("studio_read", () => ({ ...ordinaryView(flipnoteContent({ frames: [] })), group: "deadbeef" }));
  useStudioIpc(ipc, 5000); const session = ensureStudio(ME);
  const settle = async () => { for (let i = 0; i < 12; i++) { session.flush(); await new Promise(resolve => setImmediate(resolve)); flushSync(); } };
  // A draft for another server, as a closure keeps it: one edit whose delivery was interrupted.
  session.setScope({ server: SERVER + 1, channel: CHANNEL }); session.open(a); await settle();
  ipc.on("studio_apply", () => { throw "Studio transaction expired, cancelled or server stopped"; });
  session.setFps(a, 7); await session.save().catch(() => {});
  assert.equal(session.saves[0].status, "uncertain");
  const json = studioDraftSnapshot()!; resetStudio(); restoreStudioDraft(json);
  const target = document.createElement("div"); document.body.append(target);
  const nav = mount(StudioNav, { target, props: { me: ME, server: SERVER, channel: CHANNEL, onopen: () => {}, onnotice: (text: string) => notices.push(text) } }); flushSync();
  try {
    session.open(a); await settle();
    const row = () => target.querySelector("[data-waiting-draft]");
    assert.ok(row(), target.textContent!); assert.match(row()!.textContent!, /1 edit for another channel/);
    assert.match(row()!.textContent!, /1 of them may already have been saved/, "discarding cannot undo what landed, and says so");
    assert.throws(() => session.setFps(a, 8), /discard them in the Studio sidebar/, "edits wait on the draft");
    const button = (text: string) => [...row()!.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent?.trim() === text)!;
    button("discard restored edits…").click(); flushSync();
    assert.equal(session.waitingDraft?.edits, 1, "arming discards nothing");
    button("discard restored edits for real").click(); await settle();
    assert.equal(session.waitingDraft, null); assert.equal(row(), null);
    assert.equal(studioDraftSnapshot(), null, "nothing of it is kept for the next lock either");
    session.setFps(a, 8); assert.equal(session.saves.length, 1, "new edits are accepted");
  } finally { await unmount(nav); disposeStudio(); target.remove(); }
});

// Even if an external prop change moves the surfaces while a save is pending, the new channel
// must not write into the old one. Normal App departure waits for receipts before switching;
// this deliberately bypasses that gate to pin the surfaces' defensive scope handling.
test("the Studio reopened in another channel waits for the last channel's saves, then switches by itself", async () => {
  const ipc = fakeIpc(), pixels = pixBytes(), other = "77777777777777777", held = deferred<unknown>(), notices: string[] = [];
  const view = () => ({ ...ordinaryView(flipnoteContent({ fps: 12, frames: [{ id: frame, cid, bytes: pixels.length }] })), group: "deadbeef" });
  ipc.on("studio_list", args => ({ ...ordinaryView(indexContent({ objects: args.channel === CHANNEL ? { [a]: indexEntry({ title: "A" }) } : { [b]: indexEntry({ title: "B" }) } })), group: "deadbeef" }));
  ipc.on("studio_read", view);
  ipc.on("request_blob_bounded", () => ({ bytes: pixels.length, bytes_b64: b64(pixels) }));
  ipc.on("studio_apply", () => held.promise);
  useStudioIpc(ipc, 5000); const session = ensureStudio(ME);
  const settle = async () => { for (let i = 0; i < 12; i++) { session.flush(); await new Promise(resolve => setImmediate(resolve)); flushSync(); } };
  const target = document.createElement("div"); document.body.append(target);
  const surfaces = (channel: string) => [
    mount(StudioNav, { target, props: { me: ME, server: SERVER, channel, onopen: () => {}, onnotice: (text: string) => notices.push(text) } }),
    mount(Studio, { target, props: { me: ME, server: SERVER, channel, nameOf: () => "member", colorOf: () => "#fff", onnotice: (text: string) => notices.push(text) } }),
  ];
  let mounted = surfaces(CHANNEL); flushSync();
  try {
    await settle(); session.open(a); studio.selected = a; flushSync(); await settle();
    session.setFps(a, 7);
    sendStudio(); await settle(); // what leaving the Studio does: hand off, never wait
    assert.equal(ipc.calls.filter(c => c.cmd === "studio_apply").length, 1, "the edit was sent on leaving");
    for (const s of mounted) await unmount(s);
    mounted = surfaces(other); flushSync(); await settle();
    assert.equal(session.scope?.channel, CHANNEL, "the session stays with its unsaved edit");
    assert.equal(target.querySelectorAll("[data-scope-wait]").length, 2, `both surfaces say what they wait for: ${target.textContent}`);
    assert.equal(target.querySelector(".studio-new"), null, "nothing is offered that would write into the old channel");
    assert.equal(target.querySelectorAll(".studio-obj").length, 0, "the old channel's list is not shown as this one's");
    assert.equal(target.querySelector(".st-canvas"), null, "no editor for the old channel's document");
    // A new native-backed scope invalidates every old navigation claim, including deferred
    // switches. This integration preserves the accepted generation-fenced departure contract.
    const appRoute = claimStudioNavigation();
    held.resolve(view()); await settle();
    assert.equal(session.scope?.channel, other, "switched by itself once the edit landed");
    assert.equal(appRoute(), false, "the changed scope cannot authorize an old route");
    assert.equal(target.querySelector("[data-scope-wait]"), null);
    assert.ok([...target.querySelectorAll(".studio-obj .nm")].some(n => n.textContent === "B"), target.textContent!);
    assert.deepEqual(notices, [], "nothing to read or press along the way");
    // An immediate switch (nothing queued) is a navigation of its own and still retires older routes.
    const older = claimStudioNavigation();
    for (const s of mounted) await unmount(s);
    mounted = surfaces(CHANNEL); flushSync(); await settle();
    assert.equal(session.scope?.channel, CHANNEL); assert.equal(older(), false);
  } finally { for (const s of mounted) await unmount(s); disposeStudio(); target.remove(); }
});

/** Two surfaces mounted on `channel`, over one fake bridge; `settle` collects what a flush throws. */
function scopeHarness() {
  const ipc = fakeIpc(), pixels = pixBytes(), notices: string[] = [], errors: unknown[] = [];
  const view = () => ({ ...ordinaryView(flipnoteContent({ fps: 12, frames: [{ id: frame, cid, bytes: pixels.length }] })), group: "deadbeef" });
  ipc.on("studio_list", args => ({ ...ordinaryView(indexContent({ objects: args.channel === CHANNEL ? { [a]: indexEntry({ title: "A" }) } : { [b]: indexEntry({ title: "B" }) } })), group: "deadbeef" }));
  ipc.on("studio_read", view);
  ipc.on("request_blob_bounded", () => ({ bytes: pixels.length, bytes_b64: b64(pixels) }));
  useStudioIpc(ipc, 5000); const session = ensureStudio(ME);
  const settle = async () => { for (let i = 0; i < 12; i++) { session.flush(); await new Promise(resolve => setImmediate(resolve)); try { flushSync(); } catch (e) { errors.push(e); } } };
  const target = document.createElement("div"); document.body.append(target);
  let mounted: ReturnType<typeof mount>[] = [];
  const show = async (channel: string) => {
    for (const s of mounted) await unmount(s);
    mounted = [
      mount(StudioNav, { target, props: { me: ME, server: SERVER, channel, onopen: () => {}, onnotice: (text: string) => notices.push(text) } }),
      mount(Studio, { target, props: { me: ME, server: SERVER, channel, nameOf: () => "member", colorOf: () => "#fff", onnotice: (text: string) => notices.push(text) } }),
    ];
    flushSync(); await settle();
  };
  const button = (label: string) => [...target.querySelectorAll<HTMLButtonElement>("button")].find(n => n.textContent?.trim() === label);
  const done = async () => { for (const s of mounted) await unmount(s); disposeStudio(); target.remove(); };
  return { ipc, session, settle, target, show, button, done, view, notices, errors };
}

// Review L2: the last edit landing while the window close held the session made setScope throw
// inside both surfaces' effects. Review M3: the wait panel said "Saving" when nothing was moving, and
// offered Retry for edits a retry cannot move.
test("the channel wait holds through a window close, and says when its edits need the member", async () => {
  const other = "77777777777777777", held = deferred<unknown>(), h = scopeHarness();
  const uncaught = (e: unknown) => h.errors.push(e); process.on("uncaughtException", uncaught);
  try {
    h.ipc.on("studio_apply", () => held.promise);
    await h.show(CHANNEL); h.session.open(a); studio.selected = a; flushSync(); await h.settle();
    h.session.setFps(a, 7); sendStudio(); await h.settle();
    await h.show(other);
    beginStudioClose();
    held.resolve(h.view()); await h.settle(); // the edit lands while the close owns the session
    assert.deepEqual(h.errors, [], "nothing throws inside the surfaces");
    assert.equal(h.session.scope?.channel, CHANNEL, "no switch under a closing window");
    // The surfaces do not even ask while closing (studioScopeWaiting); a direct ask is refused
    // quietly too, rather than reaching setScope's closing refusal.
    assert.equal(setStudioScope(SERVER, other), false);
    endStudioClose(); await h.settle(); // the close was abandoned
    assert.equal(h.session.scope?.channel, other, "the switch happens once the close lets go");
    assert.equal(h.target.querySelector("[data-scope-wait]"), null);

    // An edit native refuses for good: the wait names it and offers only what can move it.
    h.ipc.on("studio_apply", () => { throw "Studio: edit belongs to a retired epoch"; });
    h.session.open(b); studio.selected = b; flushSync(); await h.settle();
    h.session.setFps(b, 5); sendStudio(); await h.settle();
    await h.show(CHANNEL);
    const panel = h.target.querySelector(".studio-empty[data-scope-wait]")?.textContent ?? "";
    assert.match(panel, /Your edits from the last channel need you\./, panel);
    assert.match(panel, /fps/, "the stuck edit is named");
    assert.equal(h.button("Retry"), undefined, "a retry could not change a retired epoch, so it is not offered");
    h.button("Discard")!.click(); flushSync(); h.button("Really discard?")!.click(); flushSync(); await h.settle();
    assert.equal(h.session.scope?.channel, CHANNEL, "discarding the stuck edit let the switch happen");
    assert.deepEqual(h.errors, []);
  } finally { process.off("uncaughtException", uncaught); await h.done(); }
});

// Review M5: a lock snapshot restored for a channel that can no longer be opened refused every new
// Studio edit, everywhere, with no way to let it go.
test("a restored snapshot for another channel says so and can be discarded, which frees editing", async () => {
  const other = "77777777777777777", h = scopeHarness();
  try {
    h.ipc.on("studio_apply", () => new Promise(() => {}));
    await h.show(CHANNEL); h.session.open(a); studio.selected = a; flushSync(); await h.settle();
    h.session.setFps(a, 7);
    const json = studioDraftSnapshot()!;
    resetStudio(); restoreStudioDraft(json);
    await h.show(other); // the restored channel is not the one opened
    assert.ok(h.target.querySelector("[data-restored-elsewhere]"), h.target.textContent!);
    h.session.open(b); await h.settle();
    assert.throws(() => h.session.setFps(b, 3), /restored Studio edits/, "new edits wait for the restored ones");
    h.button("Discard them")!.click(); flushSync();
    assert.ok(h.target.querySelector("[data-restored-elsewhere]"), "one click only arms the discard");
    h.button("Really discard?")!.click(); flushSync(); await h.settle();
    assert.equal(h.target.querySelector("[data-restored-elsewhere]"), null);
    h.session.setFps(b, 3); assert.equal(h.session.saves.length, 1, "editing works again");
    assert.equal(studioDraftSnapshot()?.includes(`"channel":"${CHANNEL}"`) ?? false, false, "the next lock does not carry the discarded snapshot");
  } finally { await h.done(); }
});
