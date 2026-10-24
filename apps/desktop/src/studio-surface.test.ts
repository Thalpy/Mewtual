// Compiled-component regressions for the two reactivity findings of the 0b6e870 review: the
// surface must settle after a selection (no recurring recovery or read requests), and session
// changes must reach the DOM through the revision bridge (pending read → editor, landed save →
// new selected value, recovery item → disposition). The Svelte loader in scripts/ compiles the
// real components; jsdom is the document; the fake bridge scripts every native answer.
import assert from "node:assert/strict";
import test from "node:test";
import {
  CHANNEL, ME, SERVER, b64, deferred, fakeIpc, flipnoteContent, id32, id64, indexContent, indexEntry, ordinaryView, pixBytes, recoveryListing, version,
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
  Event: w.Event,
  requestAnimationFrame: (fn: (t: number) => void) => setTimeout(() => fn(Date.now()), 0),
  cancelAnimationFrame: clearTimeout,
});
// No canvas package under jsdom: every drawing path checks for a null context and returns.
w.HTMLCanvasElement.prototype.getContext = (() => null) as unknown as typeof w.HTMLCanvasElement.prototype.getContext;

const state = await import("./studio-state.svelte.ts");
const { default: Studio } = await import("./Studio.svelte");
const { default: StudioNav } = await import("./StudioNav.svelte");
const { mount, unmount, flushSync } = await import("svelte");

function mountStudio(ipc: FakeIpc) {
  state.useStudioIpc(ipc);
  const notices: string[] = [];
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(Studio, {
    target,
    props: { me: ME, server: SERVER, channel: CHANNEL, nameOf: (id: string) => id.slice(0, 4), colorOf: () => "#000000", onnotice: (t: string) => notices.push(t) },
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

async function settled(ms = 200) {
  await wait(ms);
  flushSync();
}

const pix = pixBytes(1);
const frameView = (bytes = pix.length, cid = id64(1)) => ordinaryView(flipnoteContent({ frames: [{ id: F1, cid, bytes }] }));

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
    m.click("button.st-itab", "music");
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
