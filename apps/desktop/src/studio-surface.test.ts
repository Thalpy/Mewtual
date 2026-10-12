// Compiled-component regressions for the two reactivity findings of the 0b6e870 review: the
// surface must settle after a selection (no recurring recovery or read requests), and session
// changes must reach the DOM through the revision bridge (pending read → editor, landed save →
// new selected value, recovery item → disposition). The Svelte loader in scripts/ compiles the
// real components; jsdom is the document; the fake bridge scripts every native answer.
import assert from "node:assert/strict";
import test from "node:test";
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
    m.click("button.st-itab", "music");
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
    m.click("button.st-itab", "music");
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
    m.click("button.st-itab", "music");
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
