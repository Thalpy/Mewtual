// Compiled-component regressions for the two reactivity findings of the 0b6e870 review: the
// surface must settle after a selection (no recurring recovery or read requests), and session
// changes must reach the DOM through the revision bridge (pending read → editor, landed save →
// new selected value, recovery item → disposition). The Svelte loader in scripts/ compiles the
// real components; jsdom is the document; the fake bridge scripts every native answer.
import assert from "node:assert/strict";
import test from "node:test";
import { StudioAudioSession, type StudioAudioActions, type StudioAudioState } from "./studio-audio.ts";
import { planJamArchivalPlayback } from "./jam-archival-plan.ts";
import type { JamArchivalPlayback } from "./jam-engine.ts";
import {
  CHANNEL, ME, SERVER, b64, deferred, fakeIpc, flipnoteContent, id32, id64, indexContent, indexEntry, ordinaryView, pixBytes, previewView, recoveryListing, version,
  type FakeIpc,
} from "./studio-testkit.ts";

const OBJ = id32(0x0b);
const F1 = id32(0x11);
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

// The harness installs only what a sanitizer needs; a mounted editor needs a little more.
const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
Object.assign(globalThis, {
  getComputedStyle: w.getComputedStyle.bind(w),
  HTMLCanvasElement: w.HTMLCanvasElement,
  HTMLInputElement: w.HTMLInputElement,
  HTMLMediaElement: w.HTMLMediaElement, // the runtime's event delegation checks for it on every event
  KeyboardEvent: w.KeyboardEvent,
  MouseEvent: w.MouseEvent,
  CustomEvent: w.CustomEvent,
  MutationObserver: w.MutationObserver,
  Event: w.Event,
  requestAnimationFrame: (fn: (t: number) => void) => setTimeout(() => fn(Date.now()), 0),
  cancelAnimationFrame: clearTimeout,
});
// No canvas package under jsdom: every drawing path checks for a null context and returns.
w.HTMLCanvasElement.prototype.getContext = (() => null) as unknown as typeof w.HTMLCanvasElement.prototype.getContext;

const state = await import("./studio-state.svelte.ts");
const { default: Studio } = await import("./Studio.svelte");
const { default: StudioNav } = await import("./StudioNav.svelte");
const { default: StudioFaultControls } = await import("./StudioFaultControls.svelte");
const { default: ServerStudioSettings } = await import("./ServerStudioSettings.svelte");
const { mount, unmount, flushSync } = await import("svelte");

function mountStudio(ipc: FakeIpc, extra: Partial<{ audioActions: import("./studio-audio.ts").StudioAudioActions; patchLibrary: readonly { name: string; patch: import("./jam-contract.ts").JamPatch }[]; patchShelves: readonly { name: string; patch: import("./jam-contract.ts").JamPatch; shelf: "preset" | "mine" | "share" }[]; onpostPixa: (bytes: Uint8Array, server: number) => Promise<void> }> = {}) {
  state.useStudioIpc(ipc);
  const notices: string[] = [];
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(Studio, {
    target,
    props: { me: ME, server: SERVER, channel: CHANNEL, nameOf: (id: string) => id.slice(0, 4), colorOf: () => "#000000", onnotice: (t: string) => notices.push(t), ...extra },
  });
  flushSync();
  const count = (cmd: string) => ipc.calls.filter((c) => c.cmd === cmd).length;
  const text = () => target.textContent ?? "";
  const click = (selector: string, label?: string) => {
    const el = [...target.querySelectorAll<HTMLButtonElement>(selector)].find((b) => !label || b.textContent?.trim() === label);
    assert.ok(el, `no ${selector} ${label ?? ""}`);
    el.click();
    flushSync();
  };
  return { app, target, notices, count, text, click, done: () => { unmount(app); target.remove(); state.disposeStudio(); } };
}

function mountStudioNav(ipc: FakeIpc) {
  state.useStudioIpc(ipc);
  const notices: string[] = [];
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(StudioNav, {
    target,
    props: { me: ME, server: SERVER, channel: CHANNEL, onopen: () => {}, onnotice: (t: string) => notices.push(t) },
  });
  flushSync();
  const count = (cmd: string) => ipc.calls.filter((c) => c.cmd === cmd).length;
  const text = () => target.textContent ?? "";
  const click = (label: string) => {
    const el = [...target.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === label);
    assert.ok(el, `no button ${label}`);
    el.click();
    flushSync();
  };
  return { app, target, notices, count, text, click, done: () => { unmount(app); target.remove(); state.disposeStudio(); } };
}

async function settled(ms = 200) {
  await wait(ms);
  flushSync();
}

function mountHistorySettings(ipc: FakeIpc) {
  const target = document.createElement("div"); document.body.append(target);
  const app = mount(ServerStudioSettings, { target, props: { server: SERVER, channels: [{id: CHANNEL, name: "general"}], initialChannel: CHANNEL, owner: true, ipc } });
  flushSync();
  return { target, text: () => target.textContent ?? "", count: (cmd: string) => ipc.calls.filter(c => c.cmd === cmd).length,
    click: (_selector: string, label: string) => { const button = [...target.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent?.trim() === label); assert.ok(button, `missing ${label}`); button.click(); flushSync(); },
    chooseObject: async (manual = false) => {
      const kind = target.querySelector<HTMLSelectElement>('[aria-label="History type"]')!; kind.value = "flipnote"; kind.dispatchEvent(new Event("change", {bubbles: true})); await settled();
      const input = target.querySelector<HTMLInputElement | HTMLSelectElement>(manual ? '[aria-label="Document ID"]' : '[aria-label="History document"]')!;
      input.value = OBJ; input.dispatchEvent(new Event(manual ? "input" : "change", {bubbles: true})); await settled();
    }, done: () => { unmount(app); target.remove(); } };
}

test("G7-UI: actual repair controls require a selected observed receipt and never call scheduled repaired", async () => {
  const ipc = fakeIpc(), a = id64(1), b = id64(2);
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "faulted art" }) } })));
  ipc.on("studio_read", () => ordinaryView(flipnoteContent({ frames: [] }), { phase: "fault" }));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  ipc.on("studio_fault_read", args => ({ v: 1, kind: "fault", channel: CHANNEL, object: args.object, scope: args.scope === "registry" ? { kind: "registryBucket", bucket: 3 } : { kind: "source" }, source: { phase: "fault" },
    candidates: [a, b].map(receipt => ({ receipt, closedEpoch: "3", closeRecord: id64(3), seedChange: id64(4), inheritedEpoch: null, locallyInstalled: receipt === a })), mayDecide: true, blockedBy: null, waiting: 0, preservedOperations: 2, repair: null, lastAttempt: null }));
  ipc.on("studio_fault_repair", () => ({ v: 1, kind: "faultRepairStarted", channel: CHANNEL, scope: { kind: "source" }, start: "scheduled", refreshRequired: true }));
  const m = mountHistorySettings(ipc);
  try {
    await settled(); await m.chooseObject();
    assert.equal(m.count("studio_fault_repair"), 0, "opening settings never issues a repair");
    m.click("button", "Check history"); await settled();
    const repair = [...m.target.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent === "Keep selected version")!;
    assert.equal(repair.disabled, true);
    const radios = m.target.querySelectorAll<HTMLInputElement>('.fault-controls input[type="radio"]'); assert.equal(radios.length, 2);
    radios[1].click(); flushSync(); assert.equal(repair.disabled, false);
    repair.click(); flushSync(); await settled();
    assert.equal(m.count("studio_fault_repair"), 1);
    assert.deepEqual(ipc.calls.find(c => c.cmd === "studio_fault_repair")!.args.decision, { receiptA: a, receiptB: b, selected: b });
    assert.match(m.text(), /Repair started. Check status for the result/);
    assert.equal(m.target.querySelectorAll('.fault-controls input[type="radio"]').length, 0, "an old choice cannot be reused after scheduling");
  } finally { m.done(); }
});

const pix = pixBytes(1);

test("G7-UI: Closing FPS controls save locally and retain their exact request on retry", async () => {
  const ipc = fakeIpc(); let visits = 0;
  ipc.on("studio_list", () => ordinaryView(indexContent()));
  ipc.on("studio_read", () => ordinaryView(flipnoteContent({ frames: [] }), { phase: "closing" }));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  ipc.on("studio_closing_overlay_save_begin", a => ({ v: 1, kind: "closingOverlaySaveTicket", channel: a.channel, object: a.object, basis: id64(1), branch: id64(2), provenance: "closing" }));
  ipc.on("studio_closing_overlay_save", a => ++visits === 1 ? { v: 1, kind: "closingOverlaySave", channel: a.channel, object: a.object, state: "busy", retry: "sameRequest" }
    : { v: 1, kind: "closingOverlaySave", channel: a.channel, object: a.object, state: "saved", basis: id64(1), accepted: 1, localOnly: true, provisional: true, provenance: "closing" });
  const m = mountStudio(ipc);
  try {
    state.studio.selected = OBJ; flushSync(); await settled(350);
    const fps = m.target.querySelector<HTMLInputElement>('label.st-chip input[type="number"]');
    assert.ok(fps && !fps.disabled); fps.value = "17"; fps.dispatchEvent(new Event("change", { bubbles: true })); await settled();
    assert.equal(m.count("studio_closing_overlay_save"), 1); assert.match(m.text(), /local save not written/);
    const first = ipc.calls.find(c => c.cmd === "studio_closing_overlay_save")!.args;
    m.click("button.st-btn", "retry same request"); await settled();
    assert.deepEqual(ipc.calls.filter(c => c.cmd === "studio_closing_overlay_save")[1].args, first);
    assert.equal(m.count("studio_closing_overlay_save_begin"), 1); assert.equal(m.count("studio_apply"), 0); assert.equal(m.count("studio_unconfirmed_overlay_save"), 0);
    m.click("button.st-itab", "history"); await settled(); assert.match(m.text(), /Updating shared history/);
  } finally { m.done(); }
});

test("G7-UI: Index and unavailable object repair stay reachable when ordinary reads fail", async () => {
  const ipc = fakeIpc(); ipc.on("studio_list", () => { throw new Error("Index Registry fault"); });
  ipc.on("studio_read", () => { throw new Error("object Registry fault"); });
  const calls: unknown[] = [];
  ipc.on("studio_fault_read", a => { calls.push(a.object); return { v: 1, kind: "fault", channel: CHANNEL, object: a.object, scope: { kind: "source" }, source: { phase: "open" }, candidates: null, repair: null, lastAttempt: null, mayDecide: false, blockedBy: "noFault", waiting: 0, preservedOperations: 0 }; });
  const m = mountHistorySettings(ipc);
  try { await settled(); m.click("button", "Check history"); await settled(); assert.deepEqual(calls, [null]);
    await m.chooseObject(true); m.click("button", "Check history"); await settled(); assert.deepEqual(calls, [null, OBJ]); }
  finally { m.done(); }
});

test("G7-UI: simultaneous Index/document repair selections stay independent and diagnostics escape", async () => {
  const a = id64(1), b = id64(2);
  const session = { readFault: async () => ({ scope: { kind: "source" }, phase: "fault", candidates: [a, b].map(receipt => ({ receipt, closedEpoch: "3", closeRecord: id64(3), seedChange: id64(4), inheritedEpoch: null, locallyInstalled: false })), mayDecide: true, blockedBy: null, waiting: 0, preservedOperations: 1, repairing: false, lastOutcome: "failed", lastDetail: "<script>storage refused</script>" }) } as unknown as import("./studio-session.ts").StudioSession;
  const targets = [document.createElement("div"), document.createElement("div")];
  targets.forEach(t => document.body.appendChild(t));
  const components = targets.map((target, index) => mount(StudioFaultControls, { target, props: { session, object: index ? OBJ : null, revision: 0 } }));
  try {
    flushSync(); targets.forEach(t => t.querySelector<HTMLButtonElement>("button")!.click()); await settled();
    const radios = targets.map(t => t.querySelectorAll<HTMLInputElement>('input[type="radio"]'));
    radios[0][0].click(); flushSync(); radios[1][1].click(); flushSync();
    assert.equal(radios[0][0].checked, true); assert.equal(radios[1][1].checked, true); assert.notEqual(radios[0][0].name, radios[1][1].name);
    for (const target of targets) { assert.match(target.textContent ?? "", /<script>storage refused<\/script>/); assert.equal(target.querySelector("script"), null); assert.equal([...target.querySelectorAll("button")].find(b => b.textContent?.startsWith("Keep selected"))!.disabled, false); }
  } finally { components.forEach(c => unmount(c)); targets.forEach(t => t.remove()); }
});
test("server history list ignores stale success and failure after channel A → B → A", async () => {
  for (const rejectOld of [false, true]) {
    const ipc = fakeIpc(), reads = [deferred<unknown>(), deferred<unknown>(), deferred<unknown>()]; let calls = 0;
    ipc.on("studio_list", () => reads[calls++].promise);
    const target = document.createElement("div"); document.body.append(target);
    const app = mount(ServerStudioSettings, {target, props: {server: SERVER, initialChannel: CHANNEL, channels: [{id: CHANNEL, name: "A"}, {id: "2", name: "B"}], owner: true, ipc}}); flushSync();
    try {
      const channel = target.querySelector<HTMLSelectElement>('[aria-label="History channel"]')!;
      channel.value = "2"; channel.dispatchEvent(new Event("change", {bubbles: true})); flushSync();
      channel.value = CHANNEL; channel.dispatchEvent(new Event("change", {bubbles: true})); flushSync();
      assert.equal(calls, 3);
      reads[2].resolve(ordinaryView(indexContent({objects: {[OBJ]: indexEntry({title: "Newest list"})}}))); await settled();
      const kind = target.querySelector<HTMLSelectElement>('[aria-label="History type"]')!; kind.value = "flipnote"; kind.dispatchEvent(new Event("change", {bubbles: true})); flushSync();
      if (rejectOld) reads[0].reject(new Error("superseded request")); else reads[0].resolve(ordinaryView(indexContent({objects: {[OBJ]: indexEntry({title: "Old list"})}})));
      reads[1].reject(new Error("old B failure")); await settled();
      assert.match(target.textContent!, /Newest list/); assert.doesNotMatch(target.textContent!, /Old list|Couldn’t read/);
    } finally { await unmount(app); target.remove(); }
  }
});

const frameView = (bytes = pix.length, cid = id64(1)) => ordinaryView(flipnoteContent({ frames: [{ id: F1, cid, bytes }] }));

test("repair explains missing owner history without promising a timed identity check", async () => {
  for (const blockedBy of ["tenureUnverified", "tenureUnobserved"]) {
    const target = document.createElement("div"); document.body.append(target);
    const session = {readFault: async () => ({phase: "fault", candidates: [], blockedBy, mayDecide: false, repairing: false})} as unknown as import("./studio-session.ts").StudioSession;
    const app = mount(StudioFaultControls, {target, props: {session, object: OBJ, revision: 0}}); flushSync();
    try { target.querySelector<HTMLButtonElement>("button")!.click(); await settled(); assert.match(target.textContent!, /missing the owner history/); assert.doesNotMatch(target.textContent!, /being checked|shortly/); }
    finally { await unmount(app); target.remove(); }
  }
});

test("Studio Post to chat exports once through its attachment callback and rejects a stale preparation", async () => {
  const ipc = fakeIpc();
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_read", () => frameView());
  ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: pix.length }));
  const posted: { bytes: Uint8Array; server: number }[] = [], bytes = new Uint8Array([1, 2, 3]);
  const m = mountStudio(ipc, { onpostPixa: async (bytes, server) => { posted.push({bytes, server}); } });
  try {
    state.studio.selected = OBJ; flushSync(); await settled();
    const session = state.ensureStudio(ME);
    session.prepareFlipnotePixa = async () => bytes;
    const post = [...m.target.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent?.trim() === "Post to chat")!;
    assert.equal(post.disabled, false); post.click(); await settled();
    assert.deepEqual(posted, [{bytes, server: SERVER}]);
    assert.equal(m.count("publish_pixa"), 0, "posting does not create a Studio history reference");
    const delayed = deferred<Uint8Array>(); session.prepareFlipnotePixa = () => delayed.promise;
    post.click(); flushSync(); state.studio.selected = id32(99); flushSync(); await settled();
    delayed.resolve(bytes); await settled();
    assert.equal(posted.length, 1); assert.match(m.notices.join(" "), /flipnote changed/i);
  } finally { m.done(); }
});

test("UI-001: selecting a flipnote reads it and lists its recovery once, then the surface settles", async () => {
  const ipc = fakeIpc();
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_read", () => frameView());
  ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: pix.length }));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ, versions: [version(1)] }));
  ipc.on("publish_pix", () => ({ cid: id64(2), bytes: pixBytes(2).length }));
  ipc.on("studio_apply", () => frameView(pixBytes(2).length, id64(2)));
  const m = mountStudio(ipc);
  try {
    await settled();
    assert.equal(m.count("studio_list"), 1);
    state.studio.selected = OBJ;
    flushSync();
    await settled(400);
    assert.equal(m.count("studio_read"), 1, "one read for the selection");
    assert.equal(m.count("studio_recovery_list"), 1, "one listing for the selection");
    assert.ok(m.target.querySelector("input.st-title"), "the editor is on screen");
    // Unrelated revisions (a blob landing, a save landing) do not restart the listing or the read.
    const session = state.ensureStudio(ME);
    session.saveFrame(OBJ, F1, pixBytes(2));
    await settled(400);
    assert.equal(m.count("studio_recovery_list"), 1);
    assert.equal(m.count("studio_read"), 1);
    assert.equal(m.count("studio_apply"), 1);
    await settled(500);
    assert.equal(m.count("studio_recovery_list"), 1, "still one listing after the surface has been idle");
    assert.equal(m.count("studio_list"), 1, "and no index re-list without an event");
  } finally {
    m.done();
  }
});

test("UI-002: a pending read becomes the editor, a landed save changes the shown value, a recovery item shows its disposition", async () => {
  const ipc = fakeIpc();
  const read = deferred<unknown>();
  let reads = 0;
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_read", () => (++reads === 1 ? read.promise : frameView(4321, id64(2))));
  ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: pix.length }));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ, versions: [version(2)] }));
  ipc.on("publish_pix", () => ({ cid: id64(2), bytes: pixBytes(2).length }));
  ipc.on("studio_apply", () => frameView(4321, id64(2)));
  ipc.on("studio_recovery_read", () => ({ v: 1, kind: "recoveryVersion", historical: true, version: version(2), channel: CHANNEL, content: flipnoteContent({ frames: [{ id: F1, cid: id64(9), bytes: pix.length, op: 300 }] }) }));
  ipc.on("studio_recovery_preview", () => ({ v: 1, kind: "recoveryPreview", snapshot: id64(0x502), epochId: id32(0xe1), expectedProjection: id64(0x70), disposition: "conflict", body: null, originalAuthor: null }));
  const m = mountStudio(ipc);
  try {
    await settled();
    state.studio.selected = OBJ;
    flushSync();
    await settled();
    // (1) pending read: the surface says so and shows no editor yet.
    assert.match(m.text(), /Reading…/);
    assert.equal(m.target.querySelector("input.st-title"), null);
    read.resolve(frameView());
    await settled();
    const title = m.target.querySelector<HTMLInputElement>("input.st-title");
    assert.ok(title, "the read landed in the editor");
    assert.equal(title.value, "moon cat");
    assert.match(m.text(), /0\.3 kib/, "the frame line shows the declared size of the selected value");
    // (2) a landed save: the selected value's declared size changes on screen.
    const session = state.ensureStudio(ME);
    session.saveFrame(OBJ, F1, pixBytes(2));
    await settled(400);
    assert.equal(m.count("studio_apply"), 1);
    assert.match(m.text(), /4\.2 kib/, "the returned view replaced the projection on screen");
    assert.doesNotMatch(m.text(), /0\.3 kib/);
    // (3) a recovery walk: the item's disposition reaches the rail.
    m.click("button.st-itab", "history");
    await settled();
    assert.match(m.text(), /previous version · epoch 2/);
    m.click("button.st-btn", "restore");
    await settled(400);
    assert.equal(m.count("studio_recovery_preview"), 1);
    const items = [...m.target.querySelectorAll(".st-run li")].map((li) => li.textContent?.replace(/\s+/g, " ").trim());
    assert.equal(items.length, 1);
    assert.match(items[0] ?? "", /frame 1.*conflict/);
    assert.match(m.text(), /walked/);
  } finally {
    m.done();
  }
});

test("UI-003: a sidebar row rendered while a create is in flight changes when that create goes uncertain", async () => {
  const ipc = fakeIpc();
  const create = deferred<unknown>();
  ipc.on("studio_list", () => ordinaryView(indexContent()));
  ipc.on("studio_create", () => create.promise);
  ipc.on("publish_pix", () => ({ cid: id64(3), bytes: pixBytes(0).length }));
  state.useStudioIpc(ipc);
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(StudioNav, { target, props: { me: ME, server: SERVER, channel: CHANNEL, onopen: () => {}, onnotice: () => {} } });
  flushSync();
  try {
    await settled();
    const button = target.querySelector<HTMLButtonElement>("button.studio-new");
    assert.ok(button && !button.disabled, "the index landed and creating is offered");
    button.click();
    flushSync();
    await settled();
    const row = () => target.querySelector(".studio-obj.pending");
    assert.match(row()?.textContent ?? "", /creating…/, "the pending row is on screen while the create is held");
    create.reject(new Error("Studio actor busy; retry"));
    await settled();
    assert.equal(ipc.calls.filter((c) => c.cmd === "studio_create").length, 1);
    assert.match(row()?.textContent ?? "", /create uncertain · retry in the editor/, "the same keyed row shows the new status");
    assert.doesNotMatch(row()?.textContent ?? "", /creating…/);
  } finally {
    unmount(app);
    target.remove();
    state.disposeStudio();
  }
});

test("UI-003: a pending frame's thumbnail rendered while its publish is in flight changes when the save goes uncertain", async () => {
  const ipc = fakeIpc();
  const publish = deferred<unknown>();
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_read", () => frameView());
  ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: pix.length }));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  ipc.on("publish_pix", () => publish.promise);
  const m = mountStudio(ipc);
  try {
    await settled();
    state.studio.selected = OBJ;
    flushSync();
    await settled(400);
    const session = state.ensureStudio(ME);
    session.insertFrame(OBJ, F1, pixBytes(2));
    await settled();
    const pendingThumb = () => [...m.target.querySelectorAll(".st-thumb")].find((t) => t.querySelector(".fr.pending"));
    assert.ok(pendingThumb(), "the pending thumbnail is on screen while the publish is held");
    assert.equal(pendingThumb()?.querySelector(".ix")?.textContent, "…");
    assert.equal(pendingThumb()?.querySelector(".fr.pending.uncertain"), null);
    publish.reject(new Error("Studio storage busy; retry the same request"));
    await settled();
    assert.equal(pendingThumb()?.querySelector(".ix")?.textContent, "?", "the same thumbnail now says the save is uncertain");
    assert.ok(pendingThumb()?.querySelector(".fr.pending.uncertain"));
    assert.match(m.text(), /save uncertain/);
    assert.match(m.text(), /Studio storage busy/);
  } finally {
    m.done();
  }
});

test("UI-004: an awaiting-tenure edit is visibly local-only and its retained draft can be exported", async () => {
  const ipc = fakeIpc();
  let saved = false;
  const content = () => flipnoteContent({ fps: saved ? 8 : 12, frames: [] });
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_read", () => previewView(flipnoteContent({ fps: 12, frames: [] })));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: saved ? { branch: id64(2), content: id64(3), generation: "1", accepted: 1 } : null,
    prepared: false, eligibility: saved ? "manual" : null, manualReason: saved ? "unconfirmed" : null,
    handoffAttempt: null, unconfirmedState: saved ? "awaitingSource" : null,
    archive: null, disposed: null, transferred: false,
  }));
  ipc.on("studio_overlay_read", (a) => saved ? {
    v: 1, kind: "local-draft", channel: a.channel, object: a.object ?? null,
    eligibility: "manual", manualReason: "unconfirmed", handoffAttempt: null,
    unconfirmedState: "awaitingSource", basis: id64(1), branch: id64(2), contentId: id64(3),
    generation: "1", accepted: 1, transferState: "active", provenance: "unconfirmed",
    replayable: true, readOnly: true, content: content(),
  } : { v: 1, kind: "absent", channel: a.channel, object: a.object ?? null });
  ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ({
    v: 1, kind: "unconfirmedOverlaySaveTicket", channel: a.channel, object: a.object ?? null,
    basis: id64(1), branch: id64(2), provenance: "unconfirmed",
  }));
  let saves = 0;
  ipc.on("studio_unconfirmed_overlay_save", (a) => {
    if (++saves === 1) return { v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null, state: "busy", retry: "sameRequest" };
    saved = true;
    return { v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null, state: "saved", basis: id64(1), accepted: 1, localOnly: true, provisional: true, provenance: "unconfirmed" };
  });
  ipc.on("studio_overlay_export", () => ({
    v: 1, kind: "overlayExport", archive: id64(9), basis: id64(1), content: id64(3), branch: id64(2),
    generation: "1", author: ME, accepted: 1, provenance: { kind: "unconfirmed", provider: id64(8), observedMlsEpoch: "4", observedAtMs: "5" },
    replayable: true, readOnly: true, authority: false, terminal: true, preserved: false, notReplayable: null,
    format: "catcoms-studio-draft-v1", bytes: 3, bytesB64: b64(new Uint8Array([1, 2, 3])),
  }));
  const m = mountStudio(ipc);
  try {
    await settled();
    state.studio.selected = OBJ;
    flushSync();
    await settled(400);
    assert.match(m.text(), /awaiting tenure/);
    const fps = m.target.querySelector<HTMLInputElement>('label.st-chip input[type="number"]');
    assert.ok(fps && !fps.disabled, "preview-local editing is offered without treating the base as authoritative");
    fps.value = "8";
    fps.dispatchEvent(new Event("change", { bubbles: true }));
    await settled(500);
    assert.equal(m.count("studio_unconfirmed_overlay_save_begin"), 1);
    assert.equal(m.count("studio_unconfirmed_overlay_save"), 1);
    assert.match(m.text(), /local save not written/);
    assert.doesNotMatch(m.text(), /save uncertain/);
    assert.match(m.text(), /backend confirmed that this attempt wrote nothing/);
    m.click("button.st-btn", "retry same request");
    await settled(500);
    assert.equal(m.count("studio_unconfirmed_overlay_save"), 2);
    assert.match(m.text(), /local-only draft/);
    m.click("button.st-itab", "history");
    await settled();
    assert.match(m.text(), /created while the owner's tenure was unconfirmed/);
    m.click("button.st-btn", "export");
    await settled();
    assert.equal(m.count("studio_overlay_export"), 1);
    assert.match(m.text(), /local draft backup ready/);
  } finally {
    m.done();
  }
});

test("UI-005: a local-only Index draft exposes export, archive, release and disposal actions", async () => {
  const ipc = fakeIpc();
  let live = true;
  let archived = false;
  const branch = id64(2), contentId = id64(3), archiveId = id64(5);
  const archive = (kind: "overlayArchived" | "overlayArchive" | "overlayExport") => ({
    v: 1, kind, archive: archiveId, basis: id64(1), content: contentId, branch,
    generation: "1", author: ME, accepted: 1, provenance: { kind: "unconfirmed", provider: id64(8), observedMlsEpoch: "4", observedAtMs: "5" },
    replayable: true, readOnly: true, authority: false, terminal: true,
    ...(kind === "overlayArchived" ? { preserved: true, notReplayable: null } : {}),
    ...(kind === "overlayExport" ? { preserved: false, notReplayable: null, format: "catcoms-studio-draft-v1", bytes: 3, bytesB64: b64(new Uint8Array([1, 2, 3])) } : { physicalBytes: "100" }),
  });
  ipc.on("studio_list", () => previewView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: null,
    branch: live ? { branch, content: contentId, generation: "1", accepted: 1 } : null,
    prepared: false, eligibility: live ? "manual" : null, manualReason: live ? "unconfirmed" : null,
    handoffAttempt: null, unconfirmedState: live ? "awaitingSource" : null,
    archive: archived ? { archive: archiveId, branch, generation: "1", replayable: true } : null,
    disposed: live ? null : { branch, generation: "1", mode: "discarded" }, transferred: false,
  }));
  ipc.on("studio_overlay_read", (a) => live ? ({
    v: 1, kind: "local-draft", channel: a.channel, object: null,
    eligibility: "manual", manualReason: "unconfirmed", handoffAttempt: null,
    unconfirmedState: "awaitingSource", basis: id64(1), branch, contentId,
    generation: "1", accepted: 1, transferState: "active", provenance: "unconfirmed",
    replayable: true, readOnly: true, content: indexContent({ objects: { [OBJ]: indexEntry({ title: "local moon cat" }) } }),
  }) : ({
    v: 1, kind: "disposed", channel: a.channel, object: null, basis: id64(1), branch,
    generation: "1", accepted: 1, mode: "discarded", archive: null, readOnly: true,
  }));
  ipc.on("studio_overlay_export", () => archive("overlayExport"));
  ipc.on("studio_overlay_archive", () => { archived = true; return archive("overlayArchived"); });
  ipc.on("studio_overlay_archive_read", () => archive("overlayArchive"));
  ipc.on("studio_overlay_archive_release", () => { archived = false; return { v: 1, kind: "overlayArchiveReleased", reconcileRequired: true }; });
  ipc.on("studio_overlay_dispose", (a) => {
    live = false;
    return { v: 1, kind: "overlayDisposed", channel: a.channel, object: null, author: ME,
      provenance: { kind: "unconfirmed", provider: id64(8), observedMlsEpoch: "4", observedAtMs: "5" },
      basis: id64(1), branch, content: contentId, generation: "1", disposal: { mode: "discarded" },
      accepted: 1, sequence: "1", atMs: "2", terminal: true };
  });
  ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ({
    v: 1, kind: "unconfirmedOverlaySaveTicket", channel: a.channel, object: null,
    basis: id64(1), branch, provenance: "unconfirmed",
  }));
  let indexSaves = 0;
  ipc.on("studio_unconfirmed_overlay_save", (a) => ++indexSaves === 1
    ? { v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: null, state: "busy", retry: "sameRequest" }
    : { v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: null, state: "saved", basis: id64(1), accepted: 2, localOnly: true, provisional: true, provenance: "unconfirmed" });
  const m = mountStudioNav(ipc);
  try {
    await settled(400);
    assert.match(m.text(), /local-only index draft/);
    state.ensureStudio(ME).setEntryExpiry(OBJ, { kind: "never" });
    await settled(300);
    assert.match(m.text(), /index save not written/);
    assert.match(m.text(), /backend confirmed that this attempt wrote nothing/);
    m.click("retry exact index save");
    await settled(400);
    assert.equal(m.count("studio_unconfirmed_overlay_save"), 2);
    assert.doesNotMatch(m.text(), /index save not written/);

    m.click("export index draft");
    await settled();
    assert.equal(m.count("studio_overlay_export"), 1);
    assert.equal(ipc.calls.find((c) => c.cmd === "studio_overlay_export")?.args.object, undefined);
    assert.match(m.text(), /index draft backup ready/);

    m.click("preserve index draft");
    await settled(400);
    assert.equal(m.count("studio_overlay_archive"), 1);
    assert.equal(ipc.calls.find((c) => c.cmd === "studio_overlay_archive")?.args.object, undefined);
    assert.match(m.text(), /retained index draft archive/);
    m.click("release index archive…");
    assert.match(m.text(), /release index archive for real/);
    state.setStudioScope(SERVER, "9");
    await settled(400);
    assert.doesNotMatch(m.text(), /release index archive for real/, "confirmation did not follow the user into another channel");
    assert.equal(m.count("studio_overlay_archive_release"), 0);
    state.setStudioScope(SERVER, CHANNEL);
    await settled(400);
    assert.doesNotMatch(m.text(), /release index archive for real/, "returning does not revive stale consent");
    m.click("release index archive…");
    m.click("release index archive for real");
    await settled(400);
    assert.equal(m.count("studio_overlay_archive_release"), 1);
    assert.equal(ipc.calls.find((c) => c.cmd === "studio_overlay_archive_release")?.args.object, undefined);

    m.click("discard index draft…");
    m.click("discard index draft for real");
    await settled(400);
    assert.equal(m.count("studio_overlay_dispose"), 1);
    assert.equal(ipc.calls.find((c) => c.cmd === "studio_overlay_dispose")?.args.object, undefined);
    assert.match(m.text(), /Local index draft disposed/);
  } finally {
    m.done();
  }
});

test("UI-006: a Flipnote Copy keeps its exact retry control after the source is disposed", async () => {
  const ipc = fakeIpc();
  let disposed = false;
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_read", () => ordinaryView(flipnoteContent({ frames: [] })));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  ipc.on("studio_overlay_lifecycle", (a) => ({ v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: disposed ? null : { branch: id64(2), content: id64(3), generation: "1", accepted: 1 },
    prepared: false, eligibility: disposed ? null : "manual", manualReason: disposed ? null : "sourceReplaced",
    handoffAttempt: null, unconfirmedState: null, archive: null,
    disposed: disposed ? { branch: id64(2), generation: "1", mode: "discarded" } : null, transferred: false }));
  ipc.on("studio_overlay_read", (a) => disposed
    ? { v: 1, kind: "disposed", channel: a.channel, object: a.object ?? null, basis: id64(1), branch: id64(2),
      generation: "1", accepted: 1, mode: "discarded", archive: null, readOnly: true }
    : { v: 1, kind: "local-draft", channel: a.channel, object: a.object ?? null,
      eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null, unconfirmedState: null,
      basis: id64(1), branch: id64(2), contentId: id64(3), generation: "1", accepted: 1,
      transferState: "active", provenance: "closing", replayable: true, readOnly: true,
      content: flipnoteContent({ title: "retained", frames: [] }) });
  ipc.on("studio_overlay_copy_preview", () => ({ v: 1, kind: "overlayCopyPreview",
    source: { channel: CHANNEL, object: OBJ }, destination: { channel: CHANNEL, object: OBJ },
    epochId: id32(8), expectedProjection: id64(9), disposition: "ready",
    body: '{"field":"title","op":"set_header","value":"retained"}', originalAuthor: ME,
    sourceOps: [id64(10)], preservesBranch: false, provisional: true }));
  let applies = 0;
  ipc.on("studio_overlay_copy_apply", (a) => {
    if (++applies === 1) throw new Error("copy delivery lost");
    return { v: 1, kind: "overlayCopyRefused", channel: a.channel, object: a.object ?? null,
      reason: "stalePreview", retry: "repreview", written: false };
  });
  const m = mountStudio(ipc);
  try {
    await settled();
    state.studio.selected = OBJ;
    flushSync();
    await settled(400);
    const session = state.ensureStudio(ME);
    const choice = session.overlayChoices("copy", OBJ).find((x) => x.choice.kind === "title");
    assert.ok(choice);
    await session.copyOverlayChoice(OBJ, choice.choice, choice.label, "copy");
    flushSync();
    const exact = structuredClone(ipc.calls.find((c) => c.cmd === "studio_overlay_copy_apply")?.args);
    disposed = true;
    await session.refreshOverlay(OBJ);
    flushSync();
    m.click("button.st-itab", "history");
    await settled();
    assert.match(m.text(), /retained copy title.*uncertain/s);
    assert.match(m.text(), /local draft disposed/);
    m.click("button.st-btn", "retry exact copy");
    await settled(300);
    assert.deepEqual(ipc.calls.filter((c) => c.cmd === "studio_overlay_copy_apply")[1].args, exact);
    assert.match(m.text(), /Stopping tracking does not undo the copy or prove that it never landed/);
    const callsBeforeForget = ipc.calls.length;
    m.click("button.st-btn", "stop tracking…");
    assert.match(m.text(), /stop tracking for real/);
    m.click("button.st-btn", "stop tracking for real");
    await settled();
    assert.ok(m.notices.some((n) => /stopped tracking the uncertain copy/.test(n)), "the confirmation handler ran");
    assert.equal(session.overlay?.copy, null, "the session released the confirmed Copy identity");
    assert.doesNotMatch(m.text(), /retained copy title/);
    assert.equal(ipc.calls.length, callsBeforeForget, "forgetting the local retry identity invokes no native command");
  } finally {
    m.done();
  }
});

test("UI-007: an Index Copy keeps its exact retry control after source transfer", async () => {
  const ipc = fakeIpc();
  let transferred = false;
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_overlay_lifecycle", (a) => ({ v: 1, kind: "overlayLifecycle", channel: a.channel, object: null,
    branch: transferred ? null : { branch: id64(2), content: id64(3), generation: "1", accepted: 1 },
    prepared: false, eligibility: transferred ? null : "manual", manualReason: transferred ? null : "sourceReplaced",
    handoffAttempt: null, unconfirmedState: null, archive: null, disposed: null, transferred }));
  ipc.on("studio_overlay_read", (a) => transferred
    ? { v: 1, kind: "absent", channel: a.channel, object: null }
    : { v: 1, kind: "local-draft", channel: a.channel, object: null,
      eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null, unconfirmedState: null,
      basis: id64(1), branch: id64(2), contentId: id64(3), generation: "1", accepted: 1,
      transferState: "active", provenance: "closing", replayable: true, readOnly: true,
      content: indexContent({ objects: { [OBJ]: indexEntry({ title: "retained" }) } }) });
  ipc.on("studio_overlay_copy_preview", () => ({ v: 1, kind: "overlayCopyPreview",
    source: { channel: CHANNEL, object: null }, destination: { channel: CHANNEL, object: null },
    epochId: id32(8), expectedProjection: id64(9), disposition: "ready", body: '{"op":"set_title"}',
    originalAuthor: ME, sourceOps: [id64(10)], preservesBranch: false, provisional: true }));
  let applies = 0;
  ipc.on("studio_overlay_copy_apply", (a) => {
    if (++applies === 1) throw new Error("index copy delivery lost");
    return { v: 1, kind: "overlayCopyRefused", channel: a.channel, object: null,
      reason: "stalePreview", retry: "repreview", written: false };
  });
  const m = mountStudioNav(ipc);
  try {
    await settled(400);
    const session = state.ensureStudio(ME);
    const choice = session.overlayChoices("copy", null).find((x) => x.choice.kind === "objectTitle");
    assert.ok(choice);
    await session.copyOverlayChoice(null, choice.choice, choice.label, "copy");
    flushSync();
    const exact = structuredClone(ipc.calls.find((c) => c.cmd === "studio_overlay_copy_apply")?.args);
    transferred = true;
    await session.refreshIndex();
    flushSync();
    assert.match(m.text(), /retained index copy.*uncertain/s);
    assert.match(m.text(), /transferred into successor history/);
    m.click("retry exact index copy");
    await settled(300);
    assert.deepEqual(ipc.calls.filter((c) => c.cmd === "studio_overlay_copy_apply")[1].args, exact);
    assert.match(m.text(), /Stopping tracking does not undo the copy or prove that it never landed/);
    const callsBeforeForget = ipc.calls.length;
    m.click("stop tracking index copy…");
    assert.match(m.text(), /stop tracking index copy for real/);
    m.click("stop tracking index copy for real");
    await settled();
    assert.ok(m.notices.some((n) => /stopped tracking the uncertain index copy/.test(n)), "the index confirmation handler ran");
    assert.equal(session.indexOverlay?.copy, null, "the session released the confirmed Index Copy identity");
    assert.doesNotMatch(m.text(), /retained index copy/);
    assert.equal(ipc.calls.length, callsBeforeForget, "forgetting the index retry identity invokes no native command");
  } finally {
    m.done();
  }
});

test("UI-008: a cleanly refused Flipnote Copy has an explicit local-only cancel action", async () => {
  const ipc = fakeIpc();
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_read", () => ordinaryView(flipnoteContent({ frames: [] })));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  ipc.on("studio_overlay_lifecycle", (a) => ({ v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: { branch: id64(2), content: id64(3), generation: "1", accepted: 1 }, prepared: false,
    eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null, unconfirmedState: null,
    archive: null, disposed: null, transferred: false }));
  ipc.on("studio_overlay_read", (a) => ({ v: 1, kind: "local-draft", channel: a.channel, object: a.object ?? null,
    eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null, unconfirmedState: null,
    basis: id64(1), branch: id64(2), contentId: id64(3), generation: "1", accepted: 1,
    transferState: "active", provenance: "closing", replayable: true, readOnly: true,
    content: flipnoteContent({ title: "retained", frames: [] }) }));
  ipc.on("studio_overlay_copy_preview", (a) => ({ v: 1, kind: "overlayCopyPreview",
    source: { channel: a.channel, object: a.object ?? null }, destination: a.destination,
    epochId: id32(8), expectedProjection: id64(9),
    disposition: "ready", body: '{"field":"title","op":"set_header","value":"retained"}',
    originalAuthor: ME, sourceOps: [id64(10)], preservesBranch: false, provisional: true }));
  ipc.on("studio_overlay_copy_apply", (a) => ({ v: 1, kind: "overlayCopyRefused",
    channel: a.channel, object: a.object ?? null, reason: "stalePreview", retry: "repreview", written: false }));
  const m = mountStudio(ipc);
  try {
    await settled();
    state.studio.selected = OBJ;
    flushSync();
    await settled(400);
    const session = state.ensureStudio(ME);
    const choice = session.overlayChoices("copy", OBJ).find((x) => x.choice.kind === "title");
    assert.ok(choice);
    await session.copyOverlayChoice(OBJ, choice.choice, choice.label, "copy");
    flushSync();
    m.click("button.st-itab", "history");
    await settled();
    assert.match(m.text(), /retained copy title.*refused/s);
    assert.match(m.text(), /preview again as a new copy/);
    assert.match(m.text(), /cancel this copy/);
    const appliesBeforeCancel = m.count("studio_overlay_copy_apply");
    m.click("button.st-btn", "cancel this copy");
    await settled();
    assert.equal(session.overlay?.copy, null);
    assert.doesNotMatch(m.text(), /retained copy title/);
    assert.equal(m.count("studio_overlay_copy_apply"), appliesBeforeCancel, "cancel did not retry Apply");
    assert.ok(m.notices.some((n) => /backend confirmed that it wrote nothing/.test(n)));
  } finally {
    m.done();
  }
});

test("UI-009: a cleanly refused Index Copy has an explicit local-only cancel action", async () => {
  const ipc = fakeIpc();
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_overlay_lifecycle", (a) => ({ v: 1, kind: "overlayLifecycle", channel: a.channel, object: null,
    branch: { branch: id64(2), content: id64(3), generation: "1", accepted: 1 }, prepared: false,
    eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null, unconfirmedState: null,
    archive: null, disposed: null, transferred: false }));
  ipc.on("studio_overlay_read", (a) => ({ v: 1, kind: "local-draft", channel: a.channel, object: null,
    eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null, unconfirmedState: null,
    basis: id64(1), branch: id64(2), contentId: id64(3), generation: "1", accepted: 1,
    transferState: "active", provenance: "closing", replayable: true, readOnly: true,
    content: indexContent({ objects: { [OBJ]: indexEntry({ title: "retained" }) } }) }));
  ipc.on("studio_overlay_copy_preview", (a) => ({ v: 1, kind: "overlayCopyPreview",
    source: { channel: a.channel, object: a.object ?? null }, destination: a.destination,
    epochId: id32(8), expectedProjection: id64(9),
    disposition: "ready", body: '{"op":"set_title"}', originalAuthor: ME,
    sourceOps: [id64(10)], preservesBranch: false, provisional: true }));
  ipc.on("studio_overlay_copy_apply", (a) => ({ v: 1, kind: "overlayCopyRefused",
    channel: a.channel, object: null, reason: "stalePreview", retry: "repreview", written: false }));
  const m = mountStudioNav(ipc);
  try {
    await settled(400);
    const session = state.ensureStudio(ME);
    const choice = session.overlayChoices("copy", null).find((x) => x.choice.kind === "objectTitle");
    assert.ok(choice);
    await session.copyOverlayChoice(null, choice.choice, choice.label, "copy");
    flushSync();
    assert.match(m.text(), /retained index copy.*refused/s);
    assert.match(m.text(), /preview again as a new index copy/);
    assert.match(m.text(), /cancel this index copy/);
    const appliesBeforeCancel = m.count("studio_overlay_copy_apply");
    m.click("cancel this index copy");
    await settled();
    assert.equal(session.indexOverlay?.copy, null);
    assert.doesNotMatch(m.text(), /retained index copy/);
    assert.equal(m.count("studio_overlay_copy_apply"), appliesBeforeCancel, "cancel did not retry Index Apply");
    assert.ok(m.notices.some((n) => /backend confirmed that it wrote nothing/.test(n)));
  } finally {
    m.done();
  }
});

test("FETCH-004: a loaded raster follows the frame's reference; a changed declaration or cid locks the canvas until valid bytes exist", async () => {
  const ipc = fakeIpc();
  const N = pix.length;
  const cid1 = id64(1), cid2 = id64(2);
  let view = frameView(N, cid1);
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_read", () => view);
  ipc.on("request_blob_bounded", (a) => (a.cid === cid1 ? { bytes_b64: b64(pix), bytes: N } : null));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  const m = mountStudio(ipc);
  const wrap = () => m.target.querySelector(".st-canvas-wrap");
  const veil = () => m.target.querySelector(".st-veil")?.textContent ?? "";
  try {
    await settled();
    state.studio.selected = OBJ;
    flushSync();
    await settled(400);
    assert.ok(wrap(), "the editor is on screen");
    assert.equal(wrap()?.classList.contains("locked"), false, "a valid held frame is editable");
    assert.equal(veil(), "", "no veil over a loaded frame");
    // (1) same cid, different declaration: the session rejects the reference; the raster must not
    // keep standing in for it.
    const asked = m.count("request_blob_bounded");
    view = frameView(N - 1, cid1);
    ipc.emit("studio-updated", { server: SERVER, channel: CHANNEL, object: OBJ });
    await settled(400);
    assert.equal(m.count("studio_read"), 2);
    assert.equal(wrap()?.classList.contains("locked"), true, "the canvas is locked, not editable under a rejected reference");
    assert.match(veil(), /these pixels were rejected/);
    assert.match(veil(), new RegExp(`held pixels are ${N} bytes, not the ${N - 1}`));
    assert.equal(m.count("request_blob_bounded"), asked, "a known-wrong declaration is not re-asked");
    // Back to a valid, loaded, unlocked raster: the changed-cid leg must start from one, or it
    // would only show an empty frame turning into another empty frame.
    view = frameView(N, cid1);
    ipc.emit("studio-updated", { server: SERVER, channel: CHANNEL, object: OBJ });
    await settled(400);
    assert.equal(wrap()?.classList.contains("locked"), false, "the valid reference is editable again");
    assert.equal(veil(), "");
    assert.equal(m.count("request_blob_bounded"), asked, "held bytes for the valid reference were not re-asked");
    // (2) a different cid whose bytes are still in flight: the loaded raster must go the moment
    // the reference changes, not once transport has answered.
    const hold = deferred<unknown>();
    ipc.on("request_blob_bounded", (a) => (a.cid === cid1 ? { bytes_b64: b64(pix), bytes: N } : hold.promise));
    view = frameView(N, cid2);
    ipc.emit("studio-updated", { server: SERVER, channel: CHANNEL, object: OBJ });
    await settled(400);
    assert.equal(wrap()?.classList.contains("locked"), true, "a loaded raster does not stand in for a reference whose bytes are not here");
    assert.match(veil(), /fetching/);
    assert.equal(m.count("request_blob_bounded"), asked + 1, "the new reference was asked for once");
    hold.resolve(null);
    await settled(400);
    assert.equal(wrap()?.classList.contains("locked"), true);
    assert.match(veil(), /pixels not available yet/);
    assert.equal(m.count("request_blob_bounded"), asked + 1, "unavailable is not re-asked on repaint");
    // Bytes arrive for the new reference: editable again.
    ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: N }));
    m.click("button.st-btn", "ask again");
    await settled(400);
    assert.equal(wrap()?.classList.contains("locked"), false, "valid bytes for the current reference unlock the canvas");
    assert.equal(veil(), "");
    // (3) pending strokes are the reference until their save lands: a metadata change does not
    // discard them or lock the member out of their own work.
    const publish = deferred<unknown>();
    ipc.on("publish_pix", () => publish.promise);
    state.ensureStudio(ME).saveFrame(OBJ, F1, pixBytes(2));
    await settled();
    view = frameView(N - 1, cid2);
    ipc.emit("studio-updated", { server: SERVER, channel: CHANNEL, object: OBJ });
    await settled(400);
    assert.equal(wrap()?.classList.contains("locked"), false, "unsaved strokes stay editable");
    assert.match(m.target.querySelector(".st-readout.right")?.textContent ?? "", /saving/);
    assert.equal(veil(), "", "no rejection veil over the member's own pending strokes");
  } finally {
    m.done();
  }
});

// --- Gate 5: frame claims on the real surface ---------------------------------------------------

function mountStudioWithClaims(ipc: FakeIpc, claims: unknown, claimActions: unknown) {
  state.useStudioIpc(ipc);
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(Studio, {
    target,
    props: {
      me: ME, server: SERVER, channel: CHANNEL, nameOf: (id: string) => `n-${id.slice(0, 4)}`, colorOf: () => "#000000",
      onnotice: () => {}, claims, claimActions,
    } as never,
  });
  flushSync();
  const text = () => target.textContent ?? "";
  const button = (label: string) => [...target.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === label);
  return { target, text, button, done: () => { unmount(app); target.remove(); state.disposeStudio(); } };
}

function recordingActions() {
  const calls: string[] = [];
  const log = (name: string) => (...args: unknown[]) => { calls.push([name, ...args].join(" ")); };
  return {
    calls,
    actions: Object.freeze({
      select: log("select"), claim: log("claim"), autoClaim: log("autoClaim"), release: log("release"),
      ask: log("ask"), cancelAsk: log("cancelAsk"), pass: log("pass"),
    }),
  };
}

function openFlipnoteIpc(): FakeIpc {
  const ipc = fakeIpc();
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_read", () => frameView());
  ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: pix.length }));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  return ipc;
}

const PEER = "b2".repeat(32);

test("CLAIM-UI-001: a peer's claim shows on the timeline and frame card with its countdown, and Ask goes to the session", async () => {
  const { calls, actions } = recordingActions();
  const claims = {
    self: ME,
    entries: [{ peer: PEER, obj: OBJ, fr: F1, kind: "claim", secondsLeft: 74 }],
    local: { claim: null, ask: null },
    asksOnMine: [],
  };
  const m = mountStudioWithClaims(openFlipnoteIpc(), claims, actions);
  try {
    await settled();
    state.studio.selected = OBJ;
    flushSync();
    await settled(400);
    assert.match(m.text(), /n-b2b2 is drawing this frame · 74 s/);
    const marker = m.target.querySelector(".st-thumb .claim");
    assert.ok(marker, "the timeline marks the claimed frame");
    assert.match(marker!.getAttribute("title") ?? "", /74 s unless refreshed/);
    assert.equal(marker!.classList.contains("contested"), false);
    const ask = m.button("ask to draw it");
    assert.ok(ask, "a frame someone else claims offers Ask, not a lock");
    ask!.click();
    flushSync();
    assert.ok(calls.includes(`select ${OBJ} ${F1}`), "opening the frame reported the selection to the session");
    assert.deepEqual(calls.filter((c) => !c.startsWith("select")), [`ask ${OBJ} ${F1}`]);
    // The canvas is still editable: a claim never locks a frame.
    assert.equal(m.target.querySelector(".st-canvas-wrap")?.classList.contains("locked"), false);
  } finally {
    m.done();
  }
});

test("CLAIM-UI-002: an ask on my frame is a sticky notice with a countdown and Pass", async () => {
  const { calls, actions } = recordingActions();
  const claims = {
    self: ME,
    entries: [{ peer: PEER, obj: OBJ, fr: F1, kind: "ask", secondsLeft: 52 }],
    local: { claim: { obj: OBJ, fr: F1 }, ask: null },
    asksOnMine: [{ peer: PEER, obj: OBJ, fr: F1, kind: "ask", secondsLeft: 52 }],
  };
  const m = mountStudioWithClaims(openFlipnoteIpc(), claims, actions);
  try {
    await settled();
    state.studio.selected = OBJ;
    flushSync();
    await settled(400);
    const notice = m.target.querySelector(".st-ask");
    assert.ok(notice, "the ask notice is in the always-visible transport bar");
    assert.match(notice!.textContent ?? "", /n-b2b2 asks for frame 1 · 52 s/);
    assert.match(m.text(), /you are drawing this frame/);
    m.button("pass")!.click();
    flushSync();
    assert.ok(calls.includes(`pass ${OBJ} ${F1}`), calls.join("; "));
  } finally {
    m.done();
  }
});

test("CLAIM-UI-003: outside a call the surface offers no claim controls or explanatory filler", async () => {
  const m = mountStudioWithClaims(openFlipnoteIpc(), null, null);
  try {
    await settled();
    state.studio.selected = OBJ;
    flushSync();
    await settled(400);
    assert.doesNotMatch(m.text(), /appear while you are in a call|never lock a frame/);
    assert.equal(m.button("claim this frame"), undefined);
    assert.equal(m.button("ask to draw it"), undefined);
    assert.equal(m.target.querySelector(".st-thumb .claim"), null);
  } finally {
    m.done();
  }
});

test("G6-UI-001: native Score composer routes real drum edits and retains exact retry/recovery controls", async () => {
  const ipc = fakeIpc();
  const score = { kind: "score", title: null, bpm: null, bpb: null, steps: null, cells: {}, patches: {}, readiness: { cells: {}, patches: {}, headerConflicts: false } };
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ kind: "score", title: "native music" }) } })));
  ipc.on("studio_read", () => ({ ...ordinaryView(score), group: "deadbeef" }));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ, versions: [version(1)] }));
  ipc.on("studio_apply", () => { throw new Error("ACK lost"); });
  const m = mountStudio(ipc);
  try {
    await settled(); state.studio.selected = OBJ; flushSync(); await settled(400);
    assert.ok(m.target.querySelector('[aria-label="Score composer"]'));
    // The composer's inspector carries the shared history rail as its third tab.
    m.click("button.st-itab", "history");
    m.click("button", "copy");
    assert.match(m.text(), /Score copy may replace same-coordinate cells and recipes, title, BPM, beats per bar and Steps/);
    assert.match(m.text(), /not an atomic restore/);
    assert.doesNotMatch(m.text(), /copy replaces the current title and fps/);
    assert.equal(ipc.calls.find(c => c.cmd === "studio_read")?.args.objectKind, "score");
    m.click("button.st-itab", "patches");
    m.target.querySelector<HTMLButtonElement>('[data-score-mode="drum"]')!.click(); flushSync();
    m.target.querySelector<HTMLButtonElement>('[data-score-cell="0:0"]')!.click(); flushSync(); await settled(300);
    const first = ipc.calls.find(c => c.cmd === "studio_apply")!.args;
    assert.equal(first.objectKind, "score"); assert.equal(JSON.parse(String(first.body)).op, "set_cell");
    assert.match(m.text(), /save uncertain/); m.click("button", "retry same request"); await settled(300);
    assert.deepEqual(ipc.calls.filter(c => c.cmd === "studio_apply")[1].args, first);
    assert.match(m.text(), /history/); assert.equal(m.count("studio_recovery_list"), 1);
    assert.equal(m.count("publish_pix"), 0);
  } finally { m.done(); }
});

test("G6-UI-002: Play calls preparation in the gesture adapter and source invalidation immediately stops ownership", async () => {
  const ipc = openFlipnoteIpc(); let stops = 0, starts = 0;
  const m = mountStudio(ipc, { audioActions: { stop: () => { stops++; }, start: async (kind, prepare) => { starts++; assert.equal(kind, "flipnote"); await assert.rejects(prepare(), /stable native group/); return false; } } });
  try {
    await settled(); state.studio.selected = OBJ; flushSync(); await settled(400);
    m.click("button", "play"); await settled(100); assert.equal(starts, 1);
    const before = stops; state.ensureStudio(ME).invalidate({ objects: [OBJ] }); flushSync(); await settled(30);
    assert.ok(stops > before, "stop reacts to the immediate source generation, not only a later refreshed projection");
  } finally { m.done(); }
});

test("G7-UI: document Play after a sounding audition lowers the audition flag, so the transport reads stop", async () => {
  // An audition (the popup's piano) and document playback share one audio session. When Play
  // replaces a still-sounding note the session goes "playing" -> "preparing" -> "playing" without
  // the UI ever observing "stopped"; the flag that marks an audition must not survive that.
  const ipc = openFlipnoteIpc();
  ipc.on("studio_read", () => ({ ...frameView(), group: "deadbeef" }));
  const { default: Harness } = await import("../scripts/fixtures/StudioPlaybackHarness.svelte");
  const { legacyJamPatch } = await import("./jam-patch.ts");
  const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
  state.useStudioIpc(ipc);
  const kinds: string[] = [];
  const target = document.createElement("div"); document.body.appendChild(target);
  let harness: { updateAudio: (s: { state: "stopped" | "playing"; elapsedMs: number }) => void } | null = null;
  const audioActions = {
    start: async (kind: "score" | "flipnote") => { kinds.push(kind); harness?.updateAudio({ state: "playing", elapsedMs: 0 }); return true; },
    stop: () => { harness?.updateAudio({ state: "stopped", elapsedMs: 0 }); },
  };
  const app = mount(Harness, { target, props: { props: { me: ME, server: SERVER, channel: CHANNEL, nameOf: (id: string) => id.slice(0, 4), colorOf: () => "#000000", onnotice: () => {}, audioActions, patchLibrary: [{ name: "LANDING", patch: legacyJamPatch("sine") }] } } });
  harness = app as unknown as typeof harness; flushSync();
  const button = (label: RegExp) => [...target.querySelectorAll<HTMLButtonElement>("button")].find((b) => label.test(b.textContent?.trim() ?? ""));
  try {
    await settled(); state.studio.selected = OBJ; flushSync(); await settled(400);
    button(/^sound$/)!.click(); flushSync();
    button(/new…/)!.click(); flushSync(); await settled(50);
    const key = target.querySelector<HTMLButtonElement>('[aria-label="Play C4"]');
    assert.ok(key, "the soundfont popup is open with its audition keyboard");
    key.dispatchEvent(new w.PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 1 })); flushSync(); await settled(50);
    assert.deepEqual(kinds, ["score"], "an audition is a score-kind start of a one-note take");
    assert.ok(target.querySelector('[title="play [space]"]'), "a sounding audition is not document playback");
    button(/^cancel$/)!.click(); flushSync();
    target.querySelector<HTMLButtonElement>('[title="play [space]"]')!.click(); flushSync(); await settled(100);
    assert.deepEqual(kinds, ["score", "flipnote"]);
    assert.ok(target.querySelector('[title="stop [space]"]'), "document playback after an audition must read stop");
  } finally { unmount(app); target.remove(); state.disposeStudio(); }
});

test("G7-UI: a library tile adds a Score set_patch through the session and selects the recipe once the view holds it", async () => {
  const ipc = fakeIpc();
  const { legacyJamPatch, jamPatchId } = await import("./jam-patch.ts");
  const patch = legacyJamPatch("sine"), id = await jamPatchId(patch);
  let current: Record<string, unknown> = { kind: "score", title: null, bpm: null, bpb: null, steps: null, cells: {}, patches: {}, readiness: { cells: {}, patches: {}, headerConflicts: false } };
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ kind: "score", title: "music" }) } })));
  ipc.on("studio_read", () => ({ ...ordinaryView(current), group: "deadbeef" }));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  ipc.on("studio_apply", (args) => {
    const body = JSON.parse(String(args.body)) as { op: string; patch: string };
    assert.equal(body.op, "set_patch"); assert.equal(body.patch, id);
    current = { ...current, patches: { [id]: { removed: false, register: { selected: { value: patch, source: { opId: id64(0x51), author: ME, nonce: id32(0x51) } }, conflicts: [] } } } };
    return { ...ordinaryView(current), group: "deadbeef" };
  });
  const m = mountStudio(ipc, { patchLibrary: [{ name: "LANDING", patch }], patchShelves: [{ name: "LANDING", patch, shelf: "preset" }] });
  try {
    await settled(); state.studio.selected = OBJ; flushSync(); await settled(400);
    assert.equal(m.target.querySelector<HTMLButtonElement>('[data-score-cell="0:0"]')!.disabled, true, "no recipe, no note placement");
    m.target.querySelector<HTMLButtonElement>('[data-lib-patch="LANDING"]')!.click(); flushSync(); await settled(400);
    const apply = ipc.calls.find(c => c.cmd === "studio_apply")!.args;
    assert.equal(apply.objectKind, "score");
    assert.equal(m.target.querySelector(`[data-doc-patch="${id}"]`)?.getAttribute("aria-pressed"), "true", "the added recipe is the one notes are placed with");
    assert.equal(m.target.querySelector<HTMLButtonElement>('[data-score-cell="0:0"]')!.disabled, false);
    assert.match(m.target.querySelector(".sc-foot")!.textContent!, /LANDING/);
  } finally { m.done(); }
});

// Review of 149e7321, finding 5. A second note cancels the first while it is still preparing.
// Preparations are serialized, so the first request resolves false while the second waits its
// turn. That stale completion used to lower the flag owned by the second note, so once it sounded
// the editor read its clock as document playback, scrubbed the timeline and said the audition had
// failed. The real audio session decides cancellation; the test only holds each preparation open.
test("G7-UI: a superseded audition's late completion cannot turn the newer note into document playback", async () => {
  const { default: Harness } = await import("../scripts/fixtures/StudioPlaybackHarness.svelte");
  const { legacyJamPatch } = await import("./jam-patch.ts");
  const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
  const frames = Array.from({ length: 6 }, (_, i) => ({ id: id32(0x40 + i), cid: id64(1), bytes: pix.length }));
  const ipc = fakeIpc();
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  ipc.on("studio_read", () => ({ ...ordinaryView(flipnoteContent({ fps: 12, frames })), group: "deadbeef" }));
  ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: pix.length }));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  state.useStudioIpc(ipc);
  let now = 0;
  const gates: ReturnType<typeof deferred<void>>[] = [];
  const plans = new Map<JamArchivalPlayback, ReturnType<typeof planJamArchivalPlayback>>();
  const audio = new StudioAudioSession({ context: { state: "running", resume: async () => {} },
    prepareArchivalPlayback: async (take, animation) => {
      const gate = deferred<void>(); gates.push(gate); await gate.promise;
      const capability = {} as JamArchivalPlayback; plans.set(capability, planJamArchivalPlayback(take, animation)); return capability;
    },
    archivalPlaybackPlan: capability => plans.get(capability) ?? null,
    playArchivalEvent: () => ({ ok: true }),
    revokeArchivalPlayback: capability => plans.delete(capability),
  }, () => now);
  audio.setScope({ generation: 1, server: SERVER, channel: CHANNEL, membership: 1 }, false);
  const notices: string[] = [], outcomes: boolean[] = [];
  const target = document.createElement("div"); document.body.appendChild(target);
  let harness: { updateAudio: (s: StudioAudioState) => void } | null = null;
  const audioActions: StudioAudioActions = {
    start: async (kind, prepare, loop, offset = 0) => { const started = await audio.start(kind, prepare, loop, offset); outcomes.push(started); harness?.updateAudio(audio.state); return started; },
    stop: () => { audio.stop(); harness?.updateAudio(audio.state); },
  };
  const app = mount(Harness, { target, props: { props: { me: ME, server: SERVER, channel: CHANNEL, nameOf: (id: string) => id.slice(0, 4), colorOf: () => "#000000", onnotice: (t: string) => notices.push(t), audioActions, patchLibrary: [{ name: "LANDING", patch: legacyJamPatch("sine") }] } } });
  harness = app as unknown as typeof harness; flushSync();
  const button = (label: RegExp) => [...target.querySelectorAll<HTMLButtonElement>("button")].find((b) => label.test(b.textContent?.trim() ?? ""));
  const press = (note: string) => {
    const key = target.querySelector<HTMLButtonElement>(`[aria-label="Play ${note}"]`); assert.ok(key, `no ${note} key`);
    key.dispatchEvent(new w.PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 1 })); flushSync();
    harness!.updateAudio(audio.state); flushSync();
  };
  try {
    await settled(); state.studio.selected = OBJ; flushSync(); await settled(400);
    button(/^sound$/)!.click(); flushSync();
    button(/new…/)!.click(); flushSync(); await settled(50);
    press("C4"); await settled(20);
    press("E4"); await settled(20);
    assert.equal(gates.length, 1, "the second note waits behind the first preparation");
    gates[0].resolve(); await settled(20);
    assert.deepEqual(outcomes, [false], "the cancelled note resolved false while the newer one is pending");
    assert.equal(gates.length, 2); gates[1].resolve(); await settled(20);
    assert.deepEqual(outcomes, [false, true]);
    now += 200; await audio.tick(); harness!.updateAudio(audio.state); flushSync(); await settled(20);
    assert.equal(audio.state.state, "playing", "the newer note is still sounding");
    assert.match(target.querySelector(".st-transport .mono.strong")!.textContent!, /1 \/ 6/, "an audition never scrubs the timeline");
    assert.ok(target.querySelector('[title="play [space]"]'), "a sounding audition is not document playback");
    assert.ok(!notices.some((n) => /audition unavailable/.test(n)), notices.join("; "));
  } finally { audio.stop(); unmount(app); target.remove(); state.disposeStudio(); }
});

test("G6-UI-003: Score loop is an explicit transport toggle, off by default, and forwards only the local choice", async () => {
  const ipc = fakeIpc();
  const score = { kind: "score", title: null, bpm: null, bpb: null, steps: null, cells: {}, patches: {}, readiness: { cells: {}, patches: {}, headerConflicts: false } };
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ kind: "score", title: "music" }) } })));
  ipc.on("studio_read", () => ({ ...ordinaryView(score), group: "deadbeef" }));
  ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  const loops: boolean[] = [];
  const m = mountStudio(ipc, { audioActions: { stop: () => {}, start: async (_kind, _prepare, loop) => { loops.push(loop); return true; } } });
  try {
    await settled(); state.studio.selected = OBJ; flushSync(); await settled(400);
    const toggle = m.target.querySelector<HTMLButtonElement>('button[aria-label="Loop playback"]');
    assert.ok(toggle); assert.equal(toggle.getAttribute("aria-pressed"), "false");
    const play = () => { m.target.querySelector<HTMLButtonElement>('[title="play [space]"]')!.click(); flushSync(); };
    play(); await settled(50); assert.deepEqual(loops, [false]);
    toggle.click(); flushSync(); assert.equal(toggle.getAttribute("aria-pressed"), "true");
    play(); await settled(50); assert.deepEqual(loops, [false, true]);
  } finally { m.done(); }
});
