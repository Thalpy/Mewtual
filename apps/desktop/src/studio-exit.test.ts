// The Studio exit gate (studio-exit.ts) for App routes that change the main pane directly. Review of
// 149e7321, finding 2: Ctrl+F assigned `view = "chat"` without Studio's Save decision, and Studio's
// unmount ignores a refused commit, so a drawing the full draft queue had refused was destroyed.
// studio-exit-routes.test.ts pins that every App route uses this gate; this file proves the gate.
import assert from "node:assert/strict";
import test from "node:test";
import { createStudioExit, type StudioExitHost } from "./studio-exit.ts";
import { decodePix } from "./pix.ts";
import { CHANNEL, ME, SERVER, b64, deferred, fakeIpc, flipnoteContent, id32, id64, indexContent, ordinaryView, pixBytes } from "./studio-testkit.ts";

/** A scripted host: `pending` stands for the session's retained edits, `saves` for each Save. */
function fakeHost(opts: { onStudio?: boolean; closing?: boolean; pending?: number; saves?: (() => Promise<boolean>)[] } = {}) {
  let serial = 0, pending = opts.pending ?? 0;
  const log = { claims: 0, saves: 0 };
  const host: StudioExitHost = {
    onStudio: () => opts.onStudio ?? true,
    hasWork: () => pending > 0,
    closing: () => opts.closing ?? false,
    claim: () => { const mine = ++serial; log.claims++; return () => mine === serial; },
    save: () => { log.saves++; const next = opts.saves?.shift(); return next ? next() : Promise.resolve(true); },
  };
  return { gate: createStudioExit(host), log, setPending: (n: number) => { pending = n; }, supersede: () => { serial++; } };
}

test("with nothing to save, or off the Studio, a route runs in the same turn and still claims navigation", () => {
  for (const [onStudio, pending] of [[true, 0], [false, 3]] as const) {
    const h = fakeHost({ onStudio, pending }); let went = false;
    h.gate.leaveThen(() => { went = true; });
    assert.equal(went, true); assert.equal(h.log.claims, 1, "a newer route always supersedes an older pending one"); assert.equal(h.log.saves, 0);
  }
});

test("a refused Save keeps the route from running", async () => {
  const h = fakeHost({ pending: 1, saves: [async () => false] }); let went = false;
  h.gate.leaveThen(() => { went = true; }); await new Promise(resolve => setImmediate(resolve));
  assert.equal(went, false); assert.equal(h.log.saves, 1);
  assert.equal(await fakeHost({ pending: 1, saves: [async () => false] }).gate.release(), false);
});

test("a successful Save runs the route only while it is still the newest claim", async () => {
  const ack = deferred<boolean>(), h = fakeHost({ pending: 1, saves: [() => ack.promise] }); let went = 0;
  h.gate.leaveThen(() => { went++; });
  h.supersede(); h.setPending(0); ack.resolve(true); await new Promise(resolve => setImmediate(resolve));
  assert.equal(went, 0, "a newer navigation, lock or reload owns the screen now");
  const fresh = fakeHost({ pending: 1, saves: [async () => true] });
  fresh.gate.leaveThen(() => { went++; }); fresh.setPending(0); await new Promise(resolve => setImmediate(resolve));
  assert.equal(went, 1);
});

test("a stroke drawn while Save awaited its receipt is saved too before leaving", async () => {
  // The first Save confirms; a stroke landed meanwhile, so work remains until the second.
  let went = false, saves = 0;
  const gate = createStudioExit({ onStudio: () => true, closing: () => false, hasWork: () => saves < 2,
    claim: () => () => true, save: async () => { saves++; return true; } });
  gate.leaveThen(() => { went = true; }); for (let i = 0; i < 4; i++) await new Promise(resolve => setImmediate(resolve));
  assert.equal(saves, 2); assert.equal(went, true);
});

// Performance/save review (2026-10-30), finding 2: leaving the surface waited for definite native
// receipts, so a Save that failed or was still running trapped the member in the editor.
test("with preserve, leaving the surface seals the work locally instead of waiting for receipts", async () => {
  let sealed = false, preserves = 0, saves = 0, went = false;
  const gate = createStudioExit({ onStudio: () => true, closing: () => false, hasWork: () => true,
    claim: () => () => true, save: async () => { saves++; return false; },
    preserve: async () => { preserves++; sealed = true; return true; }, preserved: () => sealed });
  gate.leaveThen(() => { went = true; }); for (let i = 0; i < 4; i++) await new Promise(resolve => setImmediate(resolve));
  assert.equal(went, true, "held work that is sealed on this device does not keep the member here");
  assert.deepEqual([preserves, saves], [1, 0], "no receipt is awaited for a surface departure");
  // Already sealed: the next departure runs in the same turn.
  let again = false; gate.leaveThen(() => { again = true; });
  assert.equal(again, true); assert.equal(preserves, 1);
});

test("with preserve, a refused seal keeps the member here, and a channel change still needs receipts", async () => {
  let saves = 0, went = false;
  const gate = createStudioExit({ onStudio: () => true, closing: () => false, hasWork: () => true,
    claim: () => () => true, save: async () => { saves++; return false; },
    preserve: async () => false, preserved: () => false });
  gate.leaveThen(() => { went = true; }); await new Promise(resolve => setImmediate(resolve));
  assert.equal(went, false, "a stroke that cannot be staged, or a snapshot that was not written, refuses");
  assert.equal(await gate.begin().scope(), false);
  assert.equal(saves, 1, "the scope route used the receipt-waiting Save");
});

test("with preserve, a stroke drawn while the snapshot was written is sealed by another round", async () => {
  let author = 1, sealedAt = 0, preserves = 0, went = false;
  const gate = createStudioExit({ onStudio: () => true, closing: () => false, hasWork: () => true,
    claim: () => () => true, save: async () => false,
    preserve: async () => { preserves++; sealedAt = author; if (preserves === 1) author++; return true; },
    preserved: () => sealedAt === author });
  gate.leaveThen(() => { went = true; }); for (let i = 0; i < 6; i++) await new Promise(resolve => setImmediate(resolve));
  assert.equal(preserves, 2); assert.equal(went, true);
});

// The route an action carries across its awaits (re-review of 13e338eb, finding 1).
test("a route never renews its claim: once a newer action begins, it resolves false and saves nothing", async () => {
  const h = fakeHost({ pending: 1, saves: [async () => true] });
  const older = h.gate.begin();
  h.gate.begin(); // the member went back to the Studio
  assert.equal(older.current(), false);
  assert.equal(await older.leave(), false); assert.equal(await older.scope(), false);
  assert.equal(h.log.saves, 0, "a retired action asks nothing of the newer one's Studio");
  assert.equal(older.current(), false, "asking does not revive it");
});

test("leave saves only a Studio on screen; scope saves retained work wherever it is", async () => {
  const off = fakeHost({ onStudio: false, pending: 1 });
  const route = off.gate.begin();
  assert.equal(await route.leave(), true); assert.equal(off.log.saves, 0, "leaving another surface leaves the queue alone");
  off.setPending(1); // scope: the session's channel is about to change under the queue
  const scoped = off.gate.begin(), saving = scoped.scope();
  off.setPending(0); assert.equal(await saving, true); assert.equal(off.log.saves, 1);
});

test("one route asked twice at once saves once per round and both answers agree", async () => {
  const ack = deferred<boolean>(); let pending = 1, saves = 0;
  const gate = createStudioExit({ onStudio: () => true, closing: () => false, hasWork: () => pending > 0,
    claim: () => () => true, save: () => { saves++; return ack.promise.then(ok => { pending = 0; return ok; }); } });
  const route = gate.begin(), first = route.leave(), second = route.leave();
  ack.resolve(true);
  assert.deepEqual(await Promise.all([first, second]), [true, true]);
  assert.ok(saves <= 2, `${saves} saves`);
});

test("endless new work during Saves ends in a refusal, never a departure or an unbounded loop", async () => {
  let saves = 0, told = 0;
  const gate = createStudioExit({ onStudio: () => true, closing: () => false, hasWork: () => true, claim: () => () => true,
    save: async () => { saves++; return true; }, exhausted: () => { told++; } });
  assert.equal(await gate.release(), false); assert.equal(saves, 16); assert.equal(told, 1, "the member is told why nothing happened");
});

test("a route never races the window-close sequence's own Save", async () => {
  const h = fakeHost({ pending: 1, closing: true });
  assert.equal(await h.gate.release(), false); assert.equal(h.log.saves, 0);
});

// The review's reproduction on the real session and a mounted editor: the bounded draft queue is
// full (512 records, saving blocked), a stroke is drawn, staging refuses it and the pixels stay on
// the canvas. A route asking the gate must leave the editor, and those pixels, where they are.
test("a drawing the full draft queue refused keeps its editor and pixels when a direct route asks to leave", async () => {
  const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
  Object.assign(globalThis, { MutationObserver: w.MutationObserver, HTMLInputElement: w.HTMLInputElement, HTMLSelectElement: w.HTMLSelectElement, HTMLCanvasElement: w.HTMLCanvasElement, HTMLMediaElement: w.HTMLMediaElement, MouseEvent: w.MouseEvent, Event: w.Event });
  globalThis.getComputedStyle = w.getComputedStyle.bind(w);
  w.HTMLCanvasElement.prototype.getContext = (() => null) as typeof w.HTMLCanvasElement.prototype.getContext;
  w.HTMLElement.prototype.setPointerCapture = () => {};
  const { mount, unmount, flushSync } = await import("svelte");
  const { default: Studio } = await import("./Studio.svelte");
  const { ensureStudio, useStudioIpc, disposeStudio, studio, studioHasPending, saveStudio, claimStudioNavigation } = await import("./studio-state.svelte.ts");
  const object = id32(1), frame = id32(3), cid = id64(4), pixels = pixBytes(), notices: string[] = [];
  const ipc = fakeIpc();
  ipc.on("studio_list", () => ({ ...ordinaryView(indexContent()), group: "deadbeef" }));
  ipc.on("studio_read", () => ({ ...ordinaryView(flipnoteContent({ frames: [{ id: frame, cid, bytes: pixels.length }] })), group: "deadbeef" }));
  ipc.on("request_blob_bounded", () => ({ bytes: pixels.length, bytes_b64: b64(pixels) }));
  ipc.on("studio_apply", () => new Promise(() => {})); // saving stays blocked: nothing drains the queue
  useStudioIpc(ipc, 5000); const session = ensureStudio(ME);
  const settle = async () => { for (let i = 0; i < 12; i++) { session.flush(); await new Promise(resolve => setImmediate(resolve)); flushSync(); } };
  const target = document.createElement("div"); document.body.append(target);
  const editor = mount(Studio, { target, props: { me: ME, server: SERVER, channel: CHANNEL, nameOf: () => "member", colorOf: () => "#fff", onnotice: (text: string) => notices.push(text) } });
  let unmounted = false;
  try {
    await settle(); studio.selected = object; flushSync(); await settle();
    // Alternating fields never coalesce, so each edit is its own retained record.
    for (let i = 0; session.saves.length < 512; i++) { if (i % 2) session.setFps(object, 1 + (i % 24)); else session.setTitle(object, `take ${i}`); }
    const canvas = target.querySelector<HTMLCanvasElement>(".st-canvas")!, overlay = target.querySelector<HTMLCanvasElement>(".st-overlay")!;
    assert.ok(canvas && overlay, target.textContent!);
    canvas.getBoundingClientRect = () => ({ x: 0, y: 0, left: 0, top: 0, right: 192, bottom: 144, width: 192, height: 144, toJSON() {} });
    overlay.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0, clientX: 10, clientY: 10 })); flushSync();
    overlay.dispatchEvent(new MouseEvent("pointerup", { bubbles: true, button: 0, clientX: 10, clientY: 10 })); flushSync();
    assert.ok(notices.some(n => /Too many unsaved edits/.test(n)), notices.join("; "));
    assert.equal(session.saves.length, 512); assert.ok(session.saves.every(s => s.kind === "apply"), "the refused stroke is not in the queue");
    // The App wiring of this gate, over the same production studio-state functions App uses.
    const gate = createStudioExit({ onStudio: () => true, closing: () => false, hasWork: studioHasPending, claim: claimStudioNavigation,
      save: async () => { try { await saveStudio(); return true; } catch (e) { notices.push(String(e)); return false; } } });
    let left = false;
    gate.leaveThen(() => { left = true; unmounted = true; void unmount(editor); }); // what Ctrl+F's route does
    await settle();
    assert.equal(left, false, "the route did not run"); assert.equal(target.querySelector(".st-canvas"), canvas, "the editor is still mounted");
    assert.ok(notices.some(n => /Could not save this drawing/.test(n)), notices.join("; "));
    assert.equal(await gate.release(), false);
    // The pixels are still the editor's: free one slot and the next staging carries the stroke.
    session.discard(session.saves.at(-1)!.id); session.stageDrawing();
    const staged = session.saves.at(-1)!;
    assert.ok(staged.kind === "frame", "the stroke staged once there was room");
    assert.notEqual(decodePix(staged.pix).pixels[10 + 10 * 192], 0, "the refused stroke survived the attempted exit");
  } finally { if (!unmounted) await unmount(editor); disposeStudio(); target.remove(); }
});
