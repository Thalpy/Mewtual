// The connected session: same-request retry identity, unsaved work that survives failure,
// scope and request fencing, coalesced refresh from events, bounded prioritized fetching and the
// recovery walk. Driven entirely through the fake bridge; no Svelte, no timers of its own.
import assert from "node:assert/strict";
import test from "node:test";
import { canonicalJson } from "./studio-contract.ts";
import { base64ToBytes } from "./studio-native.ts";
import { StudioSession } from "./studio-session.ts";
import {
  CHANNEL, ME, ROOK, SERVER, b64, deferred, fakeIpc, flipnoteContent, id32, id64, indexContent, indexEntry, ordinaryView, pixBytes, previewView, recoveryListing, settle, version,
  type FakeIpc,
} from "./studio-testkit.ts";

const OBJ = id32(0x0b);
const F1 = id32(0x11), F2 = id32(0x12);

async function harness(opts: { ipc?: FakeIpc } = {}) {
  const ipc = opts.ipc ?? fakeIpc();
  let seq = 0x1000;
  let now = 50_000;
  const timers: (() => void)[] = [];
  const session = new StudioSession({
    ipc,
    me: ME,
    now: () => now,
    newId: () => (++seq).toString(16).padStart(32, "0"),
    schedule: (fn) => { timers.push(fn); return () => { const i = timers.indexOf(fn); if (i >= 0) timers.splice(i, 1); }; },
    maxFetches: 2,
  });
  const changes = { n: 0 };
  session.onChange(() => changes.n++);
  const fire = () => { const t = timers.splice(0); for (const fn of t) fn(); };
  await session.attach();
  const sync = async () => { for (let i = 0; i < 3; i++) { fire(); await settle(3); } };
  return { ipc, session, fire, sync, changes, tick: (ms: number) => { now += ms; }, calls: (cmd: string) => ipc.calls.filter((c) => c.cmd === cmd) };
}

const docView = (frames = [{ id: F1, cid: id64(1), bytes: pixBytes(1).length }], extra: { epochId?: string; phase?: string } = {}) => ordinaryView(flipnoteContent({ frames }), extra);

test("a Save receipt resolves without rolling back a newer object or Index read", async () => {
  for (const index of [false, true]) {
    const h = await harness(), ack = deferred<unknown>();
    const view = (title: string, epoch: number) => index
      ? ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title }) } }), { epochId: id32(epoch), epoch: String(epoch) })
      : ordinaryView(flipnoteContent({ title, frames: [] }), { epochId: id32(epoch), epoch: String(epoch) });
    h.ipc.on("studio_list", () => index ? view("initial", 1) : ordinaryView(indexContent()));
    h.ipc.on("studio_read", () => view("initial", 1));
    h.ipc.on(index ? "studio_apply_index" : "studio_apply", () => ack.promise);
    h.session.setScope({ server: SERVER, channel: CHANNEL }); if (!index) h.session.open(OBJ); await h.sync();
    if (index) h.session.renameEntry(OBJ, "saved earlier"); else h.session.setTitle(OBJ, "saved earlier");
    h.ipc.on(index ? "studio_list" : "studio_read", () => view("newer remote value", 2));
    if (index) await h.session.refreshIndex(); else await h.session.refreshDoc(OBJ);
    ack.resolve(view("saved earlier", 1)); await settle(4);
    assert.equal(index ? h.session.indexModel?.entries[0].title : h.session.doc?.model?.title, "newer remote value");
    assert.equal(index ? h.session.index?.epochId : h.session.doc?.view?.epochId, id32(2));
    assert.equal(h.session.saves.length, 0, "receipt remains a definite acknowledgement");
    await h.sync();
    assert.equal(index ? h.session.indexModel?.entries[0].title : h.session.doc?.model?.title, "newer remote value");
  }
});

test("a Read already pending before Save cannot overwrite its acknowledged projection", async () => {
  const h = await harness(), oldRead = deferred<unknown>();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([]));
  h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.open(OBJ); await h.sync();
  h.ipc.on("studio_read", () => oldRead.promise); const reading = h.session.refreshDoc(OBJ);
  const saved = ordinaryView(flipnoteContent({ title: "saved newer", frames: [] }));
  h.ipc.on("studio_apply", () => saved); h.session.setTitle(OBJ, "saved newer"); await settle(4);
  oldRead.resolve(ordinaryView(flipnoteContent({ title: "old pending read", frames: [] }))); await reading;
  assert.equal(h.session.doc?.model?.title, "saved newer"); assert.equal(h.session.saves.length, 0);
});

test("Create then open without reading saves its initial frame, including after an overlapping absence read", async () => {
  for (const overlap of [false, true, "read-fails"] as const) {
    const h = await harness(), created = deferred<unknown>(), pix = pixBytes();
    h.ipc.on("studio_list", () => ordinaryView(indexContent()));
    h.ipc.on("studio_create", () => created.promise);
    h.ipc.on("publish_pix", () => ({ cid: id64(5), bytes: pix.length }));
    h.ipc.on("studio_apply", a => {
      const op = JSON.parse(String(a.body));
      return ordinaryView(flipnoteContent({ frames: [{ id: op.frame, cid: op.cid, bytes: op.bytes }] }));
    });
    h.session.setScope({ server: SERVER, channel: CHANNEL }); await h.sync();
    const object = h.session.createFlipnote("new"); h.session.open(object, { read: false }); h.session.insertFrame(object, null, pix);
    if (overlap) {
      h.ipc.on("studio_read", () => null); await h.session.refreshDoc(object);
      h.ipc.on("studio_read", () => { if (overlap === "read-fails") throw new Error("read unavailable"); return docView([]); });
    }
    created.resolve(docView([])); await h.sync();
    if (overlap === "read-fails") {
      assert.equal(h.calls("studio_apply").length, 0);
      assert.equal(h.session.saves[0].status, "blocked", "no Apply was sent, so the first frame is known not written");
      assert.equal(h.session.saves[0].mayHaveCommitted, false);
    } else {
      assert.equal(h.calls("studio_apply").length, 1); assert.equal(h.calls("studio_apply")[0].args.object, object);
      assert.equal(h.session.saves.length, 0); assert.equal(h.session.doc?.model?.frames.length, 1);
    }
    if (!overlap) assert.equal(h.calls("studio_read").length, 0, "read:false navigation is not read admission");
  }
});

test("switching commits the previous editor first and retains an uncertain Save offscreen", async () => {
  const h = await harness(), other = id32(0x0c), ack = deferred<unknown>(), pix = pixBytes(2);
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView());
  h.ipc.on("publish_pix", () => ({ cid: id64(2), bytes: pix.length }));
  h.ipc.on("studio_apply", () => ack.promise);
  h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.open(OBJ); await h.sync();
  let dirty = true;
  const remove = h.session.beforeDocumentClose(() => {
    assert.equal(h.session.doc?.object, OBJ, "flush precedes replacing the editor");
    if (dirty) { h.session.saveFrame(OBJ, F1, pix); dirty = false; }
  });
  h.session.open(other); remove(); await h.sync();
  assert.equal(dirty, false); assert.equal(h.session.doc?.object, other);
  assert.equal(h.calls("studio_apply")[0].args.object, OBJ);
  ack.reject(new Error("response lost")); await h.sync();
  assert.equal(h.session.doc?.object, other, "old completion cannot replace the new editor");
  assert.equal(h.session.saves[0].status, "uncertain");
  assert.deepEqual(h.session.unsavedPix(OBJ, F1), pix);
  const exact = h.calls("studio_apply")[0].args;
  h.ipc.on("studio_apply", () => docView()); h.session.retry(h.session.saves[0].id); await h.sync();
  assert.deepEqual(h.calls("studio_apply")[1].args, exact); assert.equal(h.session.saves.length, 0);
  assert.equal(h.session.doc?.object, other);
});

test("a failed editor flush vetoes switching and keeps the current document", async () => {
  const h = await harness(); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.open(OBJ, { read: false });
  const old = h.session.doc;
  const remove = h.session.beforeDocumentClose(() => { throw new Error("pixels could not be queued"); });
  assert.throws(() => h.session.open(id32(0x0c)), /could not be queued/);
  assert.equal(h.session.doc, old); assert.throws(() => h.session.open(null), /could not be queued/);
  remove(); h.session.open(null); assert.equal(h.session.doc, null);
});

test("setting the scope lists the channel once per coalesced window and a stale list is dropped", async () => {
  const h = await harness();
  const first = deferred<unknown>();
  h.ipc.on("studio_list", () => first.promise);
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.invalidate({ index: true });
  h.session.invalidate({ index: true });
  h.fire();
  assert.equal(h.calls("studio_list").length, 1, "three invalidations, one read");
  // The member switches channels before the answer lands.
  h.ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "here" }) } })));
  h.session.setScope({ server: SERVER, channel: "9" });
  h.fire();
  first.resolve(ordinaryView(indexContent({ objects: { [id32(0x77)]: indexEntry({ title: "elsewhere" }) } })));
  await h.sync();
  assert.deepEqual(h.session.indexModel?.entries.map((e) => e.title), ["here"], "the earlier channel's answer never lands");
  assert.equal(h.calls("studio_list")[1].args.channel, "9");
});

test("opening a flipnote reads it; null is absence; a preview cannot authorize ordinary Apply", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => null);
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  assert.equal(h.session.doc?.absent, true);
  assert.equal(h.session.doc?.view, null);
  h.ipc.on("studio_read", () => previewView(flipnoteContent({ frames: [] })));
  h.session.open(OBJ);
  await h.sync();
  assert.equal(h.session.doc?.view?.awaitingTenureReceipt, true);
  assert.equal(h.session.canEdit(OBJ), true, "the separate local-draft Save route is available");
  assert.match((h.session.editableEpoch(OBJ) as { refused: string }).refused, /read-only preview/);
  assert.equal(h.session.editRefusal(OBJ), "");
  assert.deepEqual(h.session.known.get(OBJ), { awaiting: true, phase: null, epoch: "4" });
  h.ipc.on("studio_read", () => docView([], { phase: "closing" }));
  h.session.open(OBJ);
  await h.sync();
  assert.match((h.session.editableEpoch(OBJ) as { refused: string }).refused, /rotating/);
});

test("an awaiting-tenure edit stays local and retries the exact ticket, nonce and body", async () => {
  const h = await harness();
  let saved = false;
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => previewView(flipnoteContent({ frames: [] })));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: saved ? { branch: id64(2), content: id64(3), generation: "1", accepted: 1 } : null,
    prepared: false, eligibility: saved ? "manual" : null, manualReason: saved ? "unconfirmed" : null,
    handoffAttempt: null, unconfirmedState: saved ? "awaitingSource" : null,
    archive: null, disposed: null, transferred: false,
  }));
  h.ipc.on("studio_overlay_read", (a) => saved ? {
    v: 1, kind: "local-draft", channel: a.channel, object: a.object ?? null,
    eligibility: "manual", manualReason: "unconfirmed", handoffAttempt: null,
    unconfirmedState: "awaitingSource", basis: id64(1), branch: id64(2), contentId: id64(3),
    generation: "1", accepted: 1, transferState: "active", provenance: "unconfirmed",
    replayable: true, readOnly: true, content: flipnoteContent({ fps: 8, frames: [] }),
  } : { v: 1, kind: "absent", channel: a.channel, object: a.object ?? null });
  h.ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ({
    v: 1, kind: "unconfirmedOverlaySaveTicket", channel: a.channel, object: a.object ?? null,
    basis: id64(1), branch: id64(2), provenance: "unconfirmed",
  }));
  let visits = 0;
  h.ipc.on("studio_unconfirmed_overlay_save", (a) => {
    visits++;
    if (visits === 1) return { v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null, state: "pending", retry: "sameRequest" };
    saved = true;
    return { v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null, state: "saved", basis: id64(1), accepted: 1, localOnly: true, provisional: true, provenance: "unconfirmed" };
  });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  assert.equal(h.session.canEdit(OBJ), true, "the preview base stays non-authoritative but local drafting is available");
  assert.match((h.session.editableEpoch(OBJ) as { refused: string }).refused, /read-only preview/, "it still cannot authorize ordinary Apply");
  h.session.setFps(OBJ, 8);
  await h.sync();
  assert.equal(h.session.saves[0]?.status, "uncertain", "a detached pending save may still write");
  assert.equal(h.session.saves[0]?.kind, "overlayApply");
  const first = h.calls("studio_unconfirmed_overlay_save")[0].args;
  h.session.retry(h.session.saves[0].id);
  await h.sync();
  assert.deepEqual(h.calls("studio_unconfirmed_overlay_save")[1].args, first, "retry is byte-for-byte the same request");
  assert.equal(h.calls("studio_unconfirmed_overlay_save_begin").length, 1, "an exact retry does not mint another ticket");
  assert.equal(h.session.saves.length, 0);
  assert.equal(h.session.doc?.model?.fps, 8, "the retained branch projection, not the preview base, is displayed");
  assert.equal(h.session.overlay?.inspection?.kind, "local-draft");
});

test("Closing edits use a distinct local ticket and freeze identity across pending and source replacement", async () => {
  const h = await harness(); let phase = "closing", visits = 0;
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([], { phase }));
  h.ipc.on("studio_closing_overlay_save_begin", a => ({ v: 1, kind: "closingOverlaySaveTicket", channel: a.channel, object: a.object, basis: id64(1), branch: id64(2), provenance: "closing" }));
  h.ipc.on("studio_closing_overlay_save", a => ++visits === 1
    ? { v: 1, kind: "closingOverlaySave", channel: a.channel, object: a.object, state: "pending", retry: "sameRequest" }
    : { v: 1, kind: "closingOverlaySave", channel: a.channel, object: a.object, state: "saved", basis: id64(1), accepted: 1, localOnly: true, provisional: true, provenance: "closing" });
  h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.open(OBJ); await h.sync();
  assert.equal(h.session.canEdit(OBJ), true); assert.ok("refused" in h.session.editableEpoch(OBJ), "Closing never supplies ordinary Apply authority");
  h.session.setTitle(OBJ, "local while rotating"); await h.sync();
  const first = h.calls("studio_closing_overlay_save")[0].args;
  assert.equal(h.session.saves[0].status, "uncertain"); assert.equal(h.session.canReauthor(h.session.saves[0].id), false);
  phase = "open"; await h.session.refreshDoc(OBJ);
  h.session.retry(h.session.saves[0].id); await h.sync();
  assert.deepEqual(h.calls("studio_closing_overlay_save")[1].args, first);
  assert.equal(h.calls("studio_closing_overlay_save_begin").length, 1);
  assert.equal(h.calls("studio_unconfirmed_overlay_save").length, 0); assert.equal(h.calls("studio_apply").length, 0);
  assert.equal(h.session.saves.length, 0);
});

test("a missing signed close is a typed no-write refusal retaining unsaved Closing pixels", async () => {
  const h = await harness(), pix = pixBytes(2);
  h.ipc.on("studio_list", () => ordinaryView(indexContent())); h.ipc.on("studio_read", () => docView([], { phase: "closing" }));
  h.ipc.on("publish_pix", () => ({ cid: id64(2), bytes: pix.length }));
  h.ipc.on("studio_closing_overlay_save_begin", a => ({ v: 1, kind: "closingOverlaySave", channel: a.channel, object: a.object, state: "refused", reason: "receiptChanged", retry: "newTicket" }));
  h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.open(OBJ); await h.sync();
  h.session.saveFrame(OBJ, F1, pix); await h.sync();
  assert.equal(h.session.saves[0].status, "blocked"); assert.equal(h.session.saves[0].mayHaveCommitted, false);
  assert.deepEqual(h.session.unsavedPix(OBJ, F1), pix); assert.equal(h.calls("studio_closing_overlay_save").length, 0);
});

test("pending fault observations retire on matching object and Index invalidations", async () => {
  const h = await harness(); h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.session.setScope({ server: SERVER, channel: CHANNEL }); await h.sync();
  for (const object of [OBJ, null]) {
    const gate = deferred<unknown>(); h.ipc.on("studio_fault_read", () => gate.promise);
    const pending = h.session.readFault(object, "source"), before = h.session.faultGeneration;
    h.ipc.emit("settlement-changed", { server: SERVER, channel: CHANNEL, object, docType: object ? 16 : 15, logicalKey: object ?? id32(7), state: "repairing" });
    assert.equal(h.session.faultGeneration, before + 1);
    gate.resolve({ v: 1, kind: "fault", channel: CHANNEL, object, scope: { kind: "source" }, source: { phase: "open" }, candidates: null, repair: null, lastAttempt: null, mayDecide: false, blockedBy: "noFault", waiting: 0, preservedOperations: 0 });
    assert.equal(await pending, null, "notification invalidates a read even before it has an observation");
  }
});

test("an awaiting-tenure Index edit is read back from its local overlay for the sidebar", async () => {
  const h = await harness();
  let saved = false;
  const base = indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } });
  const local = indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat", expiry: { kind: "never" } }) } });
  h.ipc.on("studio_list", () => previewView(base));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: saved ? { branch: id64(2), content: id64(3), generation: "1", accepted: 1 } : null,
    prepared: false, eligibility: saved ? "manual" : null, manualReason: saved ? "unconfirmed" : null,
    handoffAttempt: null, unconfirmedState: saved ? "awaitingSource" : null,
    archive: null, disposed: null, transferred: false,
  }));
  h.ipc.on("studio_overlay_read", (a) => saved ? {
    v: 1, kind: "local-draft", channel: a.channel, object: a.object ?? null,
    eligibility: "manual", manualReason: "unconfirmed", handoffAttempt: null,
    unconfirmedState: "awaitingSource", basis: id64(1), branch: id64(2), contentId: id64(3),
    generation: "1", accepted: 1, transferState: "active", provenance: "unconfirmed",
    replayable: true, readOnly: true, content: local,
  } : { v: 1, kind: "absent", channel: a.channel, object: a.object ?? null });
  h.ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ({
    v: 1, kind: "unconfirmedOverlaySaveTicket", channel: a.channel, object: a.object ?? null,
    basis: id64(1), branch: id64(2), provenance: "unconfirmed",
  }));
  h.ipc.on("studio_unconfirmed_overlay_save", (a) => {
    saved = true;
    return { v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null, state: "saved", basis: id64(1), accepted: 1, localOnly: true, provisional: true, provenance: "unconfirmed" };
  });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  await h.sync();
  h.session.setEntryExpiry(OBJ, { kind: "never" });
  await h.sync();
  assert.equal(h.calls("studio_unconfirmed_overlay_save")[0].args.object, undefined, "the Index target omits object at IPC");
  assert.equal(h.session.indexOverlay?.inspection?.kind, "local-draft");
  assert.deepEqual(h.session.indexModel?.entries[0].expiry, { kind: "never" });

  const choice = h.session.overlayChoices("copy", null).find((x) => x.choice.kind === "objectTitle");
  assert.ok(choice, "the Index draft exposes its retained values through the same bounded chooser");
  h.ipc.on("studio_overlay_copy_preview", (a) => ({
    v: 1, kind: "overlayCopyPreview",
    source: { channel: a.channel, object: null }, destination: { channel: a.destination.channel, object: null },
    epochId: id32(8), expectedProjection: id64(9), disposition: "ready",
    body: '{"op":"set_title"}', originalAuthor: ME, sourceOps: [id64(10)],
    preservesBranch: false, provisional: true,
  }));
  h.ipc.on("studio_overlay_copy_apply", (a) => ({
    v: 1, kind: "overlayCopyApplied", channel: a.channel, object: null,
    contentSaved: true, alreadySaved: false, provisional: true, branchPreserved: false,
  }));
  await h.session.copyOverlayChoice(null, choice.choice, choice.label, "copy");
  const previewArgs = h.calls("studio_overlay_copy_preview")[0].args;
  assert.equal(previewArgs.object, undefined, "the Index copy source omits object on the wire");
  assert.equal(previewArgs.destination.object, undefined, "the Index copy destination omits object on the wire");
  const applyArgs = h.calls("studio_overlay_copy_apply")[0].args;
  assert.equal(applyArgs.object, undefined, "the Index apply source omits object on the wire");
  assert.equal(applyArgs.edit.destination.object, undefined, "the retained exact apply also omits an Index object");
});

test("a new-ticket refusal cannot be retried as the old request and reauthoring is explicit", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => previewView(flipnoteContent({ frames: [] })));
  let tickets = 0;
  h.ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ({
    v: 1, kind: "unconfirmedOverlaySaveTicket", channel: a.channel, object: a.object ?? null,
    basis: id64(++tickets), branch: id64(20 + tickets), provenance: "unconfirmed",
  }));
  let saves = 0;
  h.ipc.on("studio_unconfirmed_overlay_save", (a) => ++saves === 1
    ? { v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null, state: "refused", reason: "basisChanged", retry: "newTicket" }
    : { v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null, state: "saved", basis: id64(2), accepted: 1, localOnly: true, provisional: true, provenance: "unconfirmed" });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.setFps(OBJ, 9);
  await h.sync();
  const rec = h.session.saves[0];
  assert.equal(rec.kind, "overlayApply");
  assert.equal(rec.status, "blocked", "the typed refusal is not delivery uncertainty");
  if (rec.kind !== "overlayApply") return;
  const first = structuredClone(rec.request);
  h.session.retry(rec.id);
  await h.sync();
  assert.equal(h.calls("studio_unconfirmed_overlay_save").length, 1, "newTicket is not mislabeled as an exact retry");
  h.session.reauthor(rec.id);
  await h.sync();
  assert.equal(h.calls("studio_unconfirmed_overlay_save_begin").length, 2);
  const second = h.calls("studio_unconfirmed_overlay_save")[1].args.save as { nonce: string; body: string };
  assert.notEqual(second.nonce, first?.nonce);
  assert.equal(second.body, first?.body);
  assert.equal(h.session.saves.length, 0);
});

test("a typed Begin refusal is retained as known-not-written and explicit reauthoring obtains a new ticket", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => previewView(flipnoteContent({ frames: [] })));
  let begins = 0;
  h.ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ++begins === 1
    ? { v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null, state: "refused", reason: "previewExpired", retry: "newTicket" }
    : { v: 1, kind: "unconfirmedOverlaySaveTicket", channel: a.channel, object: a.object ?? null, basis: id64(2), branch: id64(22), provenance: "unconfirmed" });
  h.ipc.on("studio_unconfirmed_overlay_save", (a) => ({
    v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null,
    state: "saved", basis: id64(2), accepted: 1, localOnly: true, provisional: true, provenance: "unconfirmed",
  }));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.setFps(OBJ, 9);
  await h.sync();
  const rec = h.session.saves[0];
  assert.equal(rec.status, "blocked");
  assert.equal(rec.kind, "overlayApply");
  if (rec.kind !== "overlayApply") return;
  assert.equal(rec.request, null, "a refusal cannot be fabricated into a ticketed request");
  assert.equal(rec.overlayRetry, "newTicket");
  h.session.retry(rec.id);
  await h.sync();
  assert.equal(begins, 1, "normal retry cannot silently replace an expired ticket");
  h.session.reauthor(rec.id);
  await h.sync();
  assert.equal(begins, 2);
  assert.equal(h.calls("studio_unconfirmed_overlay_save").length, 1);
  assert.equal(h.session.saves.length, 0);
});

test("a proven-no-write Unconfirmed edit re-authors through the document's current ordinary route", async () => {
  const h = await harness();
  let ordinary = false;
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => ordinary ? docView([], { epochId: id32(0xe9) }) : previewView(flipnoteContent({ frames: [] })));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({ v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: null, prepared: false, eligibility: null, manualReason: null, handoffAttempt: null,
    unconfirmedState: null, archive: null, disposed: null, transferred: false }));
  h.ipc.on("studio_overlay_read", (a) => ({ v: 1, kind: "absent", channel: a.channel, object: a.object ?? null }));
  h.ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ({ v: 1, kind: "unconfirmedOverlaySaveTicket",
    channel: a.channel, object: a.object ?? null, basis: id64(1), branch: id64(2), provenance: "unconfirmed" }));
  h.ipc.on("studio_unconfirmed_overlay_save", (a) => ({ v: 1, kind: "unconfirmedOverlaySave",
    channel: a.channel, object: a.object ?? null, state: "refused", reason: "basisChanged", retry: "newTicket" }));
  h.ipc.on("studio_apply", () => docView([], { epochId: id32(0xe9) }));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.setFps(OBJ, 9);
  await h.sync();
  const old = h.session.saves[0];
  assert.equal(old.kind, "overlayApply");
  assert.equal(old.status, "blocked");
  const oldNonce = old.kind === "overlayApply" ? old.request?.nonce : null;
  ordinary = true;
  await h.session.refreshDoc(OBJ);
  assert.equal(h.session.canReauthor(old.id), true);
  h.session.reauthor(old.id);
  await h.sync();
  assert.equal(h.calls("studio_unconfirmed_overlay_save_begin").length, 1, "the new ordinary route does not ask for another overlay ticket");
  const applied = h.calls("studio_apply")[0].args;
  assert.equal(applied.epochId, id32(0xe9));
  assert.notEqual(applied.nonce, oldNonce);
  assert.deepEqual(JSON.parse(String(applied.body)), { field: "fps", op: "set_header", value: 9 });
  assert.equal(h.session.saves.length, 0);
});

test("a proven-no-write Unconfirmed frame re-publishes pixels and re-authors as an ordinary save", async () => {
  const h = await harness();
  let ordinary = false;
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => ordinary ? docView([], { epochId: id32(0xe9) }) : previewView(flipnoteContent({ frames: [] })));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({ v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: null, prepared: false, eligibility: null, manualReason: null, handoffAttempt: null,
    unconfirmedState: null, archive: null, disposed: null, transferred: false }));
  h.ipc.on("studio_overlay_read", (a) => ({ v: 1, kind: "absent", channel: a.channel, object: a.object ?? null }));
  h.ipc.on("publish_pix", () => ({ cid: id64(9), bytes: pixBytes(5).length }));
  h.ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ({ v: 1, kind: "unconfirmedOverlaySaveTicket",
    channel: a.channel, object: a.object ?? null, basis: id64(1), branch: id64(2), provenance: "unconfirmed" }));
  h.ipc.on("studio_unconfirmed_overlay_save", (a) => ({ v: 1, kind: "unconfirmedOverlaySave",
    channel: a.channel, object: a.object ?? null, state: "refused", reason: "pixelUnpublished", retry: "newTicket" }));
  h.ipc.on("studio_apply", () => docView([], { epochId: id32(0xe9) }));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.insertFrame(OBJ, null, pixBytes(5));
  await h.sync();
  const old = h.session.saves[0];
  assert.equal(old.kind, "frame");
  assert.equal(old.status, "blocked");
  ordinary = true;
  await h.session.refreshDoc(OBJ);
  h.session.reauthor(old.id);
  await h.sync();
  assert.equal(h.calls("publish_pix").length, 2, "a stale Unconfirmed publication receipt does not cross routes");
  assert.equal(h.calls("studio_unconfirmed_overlay_save_begin").length, 1);
  const applied = h.calls("studio_apply")[0].args;
  assert.equal(applied.epochId, id32(0xe9));
  assert.equal(JSON.parse(String(applied.body)).op, "insert_frame");
  assert.equal(h.session.saves.length, 0);
});

test("a paused local save is not resent when its required explicit access fails", async () => {
  const h = await harness();
  let reads = 0;
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => {
    if (++reads === 1) return previewView(flipnoteContent({ frames: [] }));
    throw new Error("read transport failed");
  });
  h.ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: null, prepared: false, eligibility: null, manualReason: null,
    handoffAttempt: null, unconfirmedState: null, archive: null, disposed: null, transferred: false,
  }));
  h.ipc.on("studio_overlay_read", (a) => ({ v: 1, kind: "absent", channel: a.channel, object: a.object ?? null }));
  h.ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ({
    v: 1, kind: "unconfirmedOverlaySaveTicket", channel: a.channel, object: a.object ?? null,
    basis: id64(1), branch: id64(2), provenance: "unconfirmed",
  }));
  h.ipc.on("studio_unconfirmed_overlay_save", (a) => ({
    v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null,
    state: "paused", retry: "afterExplicitAccess",
  }));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.setFps(OBJ, 9);
  await h.sync();
  const rec = h.session.saves[0];
  assert.equal(rec.status, "blocked");
  h.session.retry(rec.id);
  await h.sync();
  assert.equal(h.calls("studio_unconfirmed_overlay_save").length, 1, "failed access did not resend the paused Save");
  assert.equal(h.session.saves[0].status, "blocked");
  assert.match(h.session.saves[0].error, /explicit document access failed/);
});

test("a scope change during paused-save access cannot mutate the abandoned target", async () => {
  const h = await harness();
  const resumedRead = deferred<unknown>();
  let reads = 0;
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => ++reads === 1
    ? previewView(flipnoteContent({ frames: [] }))
    : resumedRead.promise);
  h.ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: null, prepared: false, eligibility: null, manualReason: null,
    handoffAttempt: null, unconfirmedState: null, archive: null, disposed: null, transferred: false,
  }));
  h.ipc.on("studio_overlay_read", (a) => ({ v: 1, kind: "absent", channel: a.channel, object: a.object ?? null }));
  h.ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ({
    v: 1, kind: "unconfirmedOverlaySaveTicket", channel: a.channel, object: a.object ?? null,
    basis: id64(1), branch: id64(2), provenance: "unconfirmed",
  }));
  h.ipc.on("studio_unconfirmed_overlay_save", (a) => ({
    v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null,
    state: "paused", retry: "afterExplicitAccess",
  }));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.setFps(OBJ, 9);
  await h.sync();
  h.session.retry(h.session.saves[0].id);
  await settle(2);
  assert.equal(h.calls("studio_read").length, 2, "the explicit access is in flight");
  h.session.setScope({ server: SERVER, channel: "9" });
  resumedRead.resolve(previewView(flipnoteContent({ frames: [] })));
  await h.sync();
  assert.equal(h.calls("studio_unconfirmed_overlay_save").length, 1, "the old target was not retried after its scope was abandoned");
  assert.equal(h.calls("studio_unconfirmed_overlay_save_begin").length, 1);
});

test("a first-attempt local-draft Busy remains a truthful known-no-write result", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => previewView(flipnoteContent({ frames: [] })));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: null, prepared: false, eligibility: null, manualReason: null,
    handoffAttempt: null, unconfirmedState: null, archive: null, disposed: null, transferred: false,
  }));
  h.ipc.on("studio_overlay_read", (a) => ({ v: 1, kind: "absent", channel: a.channel, object: a.object ?? null }));
  h.ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ({
    v: 1, kind: "unconfirmedOverlaySaveTicket", channel: a.channel, object: a.object ?? null,
    basis: id64(1), branch: id64(2), provenance: "unconfirmed",
  }));
  h.ipc.on("studio_unconfirmed_overlay_save", (a) => ({
    v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null,
    state: "busy", retry: "sameRequest",
  }));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.setFps(OBJ, 9);
  await h.sync();
  assert.equal(h.session.saves[0].status, "blocked");
  assert.match(h.session.saves[0].error, /busy/);
});

test("a Busy retry cannot erase an earlier local-draft Save delivery uncertainty", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => previewView(flipnoteContent({ frames: [] })));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: null, prepared: false, eligibility: null, manualReason: null,
    handoffAttempt: null, unconfirmedState: null, archive: null, disposed: null, transferred: false,
  }));
  h.ipc.on("studio_overlay_read", (a) => ({ v: 1, kind: "absent", channel: a.channel, object: a.object ?? null }));
  h.ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ({
    v: 1, kind: "unconfirmedOverlaySaveTicket", channel: a.channel, object: a.object ?? null,
    basis: id64(1), branch: id64(2), provenance: "unconfirmed",
  }));
  let attempts = 0;
  h.ipc.on("studio_unconfirmed_overlay_save", (a) => {
    attempts++;
    if (attempts === 1) throw new Error("outcome=uncertain; commit response was lost");
    if (attempts === 2) return {
      v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null,
      state: "busy", retry: "sameRequest",
    };
    return {
      v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null,
      state: "saved", basis: id64(1), accepted: 1, localOnly: true, provisional: true, provenance: "unconfirmed",
    };
  });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.setFps(OBJ, 9);
  await h.sync();
  const record = h.session.saves[0];
  const exact = h.calls("studio_unconfirmed_overlay_save")[0].args;
  assert.equal(record.status, "uncertain");
  h.session.retry(record.id);
  await h.sync();
  assert.equal(h.session.saves[0].status, "uncertain", "Busy describes only the retry, not the lost earlier result");
  assert.equal(h.session.saves[0].mayHaveCommitted, true);
  assert.match(h.session.saves[0].error, /earlier attempt may already have committed/);
  assert.deepEqual(h.calls("studio_unconfirmed_overlay_save")[1].args, exact);
  h.session.retry(record.id);
  await h.sync();
  assert.deepEqual(h.calls("studio_unconfirmed_overlay_save")[2].args, exact);
  assert.equal(h.session.saves.length, 0, "the exact acknowledgement resolves the retained uncertainty");
});

test("a pixel refusal re-publishes retained bytes before an explicitly re-authored local frame save", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => previewView(flipnoteContent({ frames: [] })));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: null, prepared: false, eligibility: null, manualReason: null,
    handoffAttempt: null, unconfirmedState: null, archive: null, disposed: null, transferred: false,
  }));
  h.ipc.on("studio_overlay_read", (a) => ({ v: 1, kind: "absent", channel: a.channel, object: a.object ?? null }));
  let tickets = 0;
  h.ipc.on("studio_unconfirmed_overlay_save_begin", (a) => ({
    v: 1, kind: "unconfirmedOverlaySaveTicket", channel: a.channel, object: a.object ?? null,
    basis: id64(++tickets), branch: id64(20 + tickets), provenance: "unconfirmed",
  }));
  h.ipc.on("publish_pix", (a) => ({ cid: id64(9), bytes: base64ToBytes(String(a.bytesB64)).length }));
  let saves = 0;
  h.ipc.on("studio_unconfirmed_overlay_save", (a) => ++saves === 1
    ? { v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null, state: "refused", reason: "pixelUnpublished", retry: "newTicket" }
    : { v: 1, kind: "unconfirmedOverlaySave", channel: a.channel, object: a.object ?? null, state: "saved", basis: id64(2), accepted: 1, localOnly: true, provisional: true, provenance: "unconfirmed" });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.insertFrame(OBJ, null, pixBytes(5));
  await h.sync();
  const rec = h.session.saves[0];
  assert.equal(rec.status, "blocked");
  assert.equal(rec.kind, "frame");
  if (rec.kind !== "frame") return;
  const first = h.calls("studio_unconfirmed_overlay_save")[0].args.save as { nonce: string };
  h.session.reauthor(rec.id);
  await h.sync();
  assert.equal(h.calls("publish_pix").length, 2, "the stale publication receipt was not reused");
  assert.equal(h.calls("studio_unconfirmed_overlay_save_begin").length, 2);
  const second = h.calls("studio_unconfirmed_overlay_save")[1].args.save as { nonce: string };
  assert.notEqual(second.nonce, first.nonce);
});

test("a retained-draft copy keeps the exact previewed apply across uncertain delivery", async () => {
  const h = await harness();
  const other = id32(0x0c);
  const historical = flipnoteContent({ title: "retained", frames: [] });
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([]));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: { branch: id64(2), content: id64(3), generation: "1", accepted: 1 },
    prepared: false, eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null,
    unconfirmedState: null, archive: null, disposed: null, transferred: false,
  }));
  h.ipc.on("studio_overlay_read", (a) => ({
    v: 1, kind: "local-draft", channel: a.channel, object: a.object ?? null,
    eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null,
    unconfirmedState: null, basis: id64(1), branch: id64(2), contentId: id64(3), generation: "1",
    accepted: 1, transferState: "active", provenance: "closing", replayable: true, readOnly: true,
    content: historical,
  }));
  h.ipc.on("studio_overlay_copy_preview", () => ({
    v: 1, kind: "overlayCopyPreview",
    source: { channel: CHANNEL, object: OBJ }, destination: { channel: CHANNEL, object: OBJ },
    epochId: id32(8), expectedProjection: id64(9), disposition: "ready",
    body: '{"field":"title","op":"set_header","value":"retained"}',
    originalAuthor: ME, sourceOps: [id64(10)], preservesBranch: false, provisional: true,
  }));
  let applies = 0;
  h.ipc.on("studio_overlay_copy_apply", (a) => {
    applies++;
    if (applies === 1) throw new Error("outcome=uncertain; copy result was not delivered");
    return { v: 1, kind: "overlayCopyApplied", channel: a.channel, object: a.object ?? null, contentSaved: true, alreadySaved: true, provisional: true, branchPreserved: false };
  });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  const choice = h.session.overlayChoices("copy").find((x) => x.choice.kind === "title");
  assert.ok(choice);
  await h.session.copyOverlayChoice(OBJ, choice.choice, choice.label, "copy");
  assert.equal(h.session.overlay?.copy?.status, "uncertain");
  const first = h.calls("studio_overlay_copy_apply")[0].args;
  h.session.open(other);
  await h.sync();
  h.session.open(OBJ);
  await h.sync();
  assert.equal(h.session.overlay?.copy?.status, "uncertain", "navigation kept the unresolved request");
  await assert.rejects(
    h.session.copyOverlayChoice(OBJ, choice.choice, "replacement", "copy"),
    /previous local draft copy is unresolved/,
    "a second action cannot discard the request whose result was lost",
  );
  await h.session.retryOverlayCopy(OBJ);
  assert.deepEqual(h.calls("studio_overlay_copy_apply")[1].args, first);
  assert.equal(h.calls("studio_overlay_copy_preview").length, 1, "retry never asks for a replacement plan");
  assert.equal(h.session.overlay?.copy?.status, "applied");
});

test("a clean stale Copy refusal can re-preview with a fresh nonce", async () => {
  const h = await harness();
  const historical = flipnoteContent({ title: "retained", frames: [] });
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([]));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: { branch: id64(2), content: id64(3), generation: "1", accepted: 1 },
    prepared: false, eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null,
    unconfirmedState: null, archive: null, disposed: null, transferred: false,
  }));
  h.ipc.on("studio_overlay_read", (a) => ({
    v: 1, kind: "local-draft", channel: a.channel, object: a.object ?? null,
    eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null,
    unconfirmedState: null, basis: id64(1), branch: id64(2), contentId: id64(3), generation: "1",
    accepted: 1, transferState: "active", provenance: "closing", replayable: true, readOnly: true,
    content: historical,
  }));
  let previews = 0;
  h.ipc.on("studio_overlay_copy_preview", () => ({
    v: 1, kind: "overlayCopyPreview", source: { channel: CHANNEL, object: OBJ }, destination: { channel: CHANNEL, object: OBJ },
    epochId: id32(8 + previews++), expectedProjection: id64(20 + previews), disposition: "ready",
    body: '{"field":"title","op":"set_header","value":"retained"}', originalAuthor: ME,
    sourceOps: [id64(10)], preservesBranch: false, provisional: true,
  }));
  let applies = 0;
  h.ipc.on("studio_overlay_copy_apply", (a) => ++applies < 3
    ? { v: 1, kind: "overlayCopyRefused", channel: a.channel, object: a.object ?? null, reason: "stalePreview", retry: "repreview", written: false }
    : { v: 1, kind: "overlayCopyApplied", channel: a.channel, object: a.object ?? null, contentSaved: true, alreadySaved: false, provisional: true, branchPreserved: false });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  const choice = h.session.overlayChoices("copy").find((x) => x.choice.kind === "title");
  assert.ok(choice);
  await h.session.copyOverlayChoice(OBJ, choice.choice, choice.label, "copy");
  assert.equal(h.session.overlay?.copy?.status, "refused", "a projection refusal is proven not written");
  const first = h.calls("studio_overlay_copy_apply")[0].args.edit;
  await h.session.repreviewOverlayCopy(OBJ);
  assert.equal(h.session.overlay?.copy?.status, "refused", "an epoch refusal also offers a new preview");
  const second = h.calls("studio_overlay_copy_apply")[1].args.edit;
  assert.notEqual(second.nonce, first.nonce);
  assert.notEqual(second.epochId, first.epochId);
  await h.session.repreviewOverlayCopy(OBJ);
  const third = h.calls("studio_overlay_copy_apply")[2].args.edit;
  assert.notEqual(third.nonce, second.nonce);
  assert.equal(h.session.overlay?.copy?.status, "applied");
  assert.equal(h.calls("studio_overlay_copy_preview").length, 3);
});

test("a clean Copy refusal can be cancelled locally and a stale nonce cannot cancel its replacement", async () => {
  const h = await harness();
  const historical = flipnoteContent({ title: "retained", fps: 6, frames: [] });
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([]));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: { branch: id64(2), content: id64(3), generation: "1", accepted: 1 },
    prepared: false, eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null,
    unconfirmedState: null, archive: null, disposed: null, transferred: false,
  }));
  h.ipc.on("studio_overlay_read", (a) => ({
    v: 1, kind: "local-draft", channel: a.channel, object: a.object ?? null,
    eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null,
    unconfirmedState: null, basis: id64(1), branch: id64(2), contentId: id64(3), generation: "1",
    accepted: 1, transferState: "active", provenance: "closing", replayable: true, readOnly: true,
    content: historical,
  }));
  h.ipc.on("studio_overlay_copy_preview", (a) => ({
    v: 1, kind: "overlayCopyPreview",
    source: { channel: CHANNEL, object: OBJ }, destination: { channel: CHANNEL, object: OBJ },
    epochId: id32(8), expectedProjection: id64(9), disposition: "ready",
    body: JSON.stringify({ op: "set_header", field: a.choice.kind, value: a.choice.kind === "title" ? "retained" : 6 }),
    originalAuthor: ME, sourceOps: [id64(10)], preservesBranch: false, provisional: true,
  }));
  h.ipc.on("studio_overlay_copy_apply", (a) => ({
    v: 1, kind: "overlayCopyRefused", channel: a.channel, object: a.object ?? null,
    reason: "stalePreview", retry: "repreview", written: false,
  }));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  const title = h.session.overlayChoices("copy").find((x) => x.choice.kind === "title");
  const fps = h.session.overlayChoices("copy").find((x) => x.choice.kind === "fps");
  assert.ok(title && fps);

  await h.session.copyOverlayChoice(OBJ, title.choice, title.label, "copy");
  const firstNonce = h.session.overlay?.copy?.edit?.nonce;
  assert.ok(firstNonce);
  assert.equal(h.session.overlay?.copy?.status, "refused");
  assert.equal(h.session.overlay?.copy?.mayHaveCommitted, false);
  const callsBeforeCancel = h.ipc.calls.length;
  assert.equal(h.session.cancelRefusedOverlayCopy(OBJ, id32(0xff)), false, "the exact refused nonce is required");
  assert.equal(h.session.overlay?.copy?.edit?.nonce, firstNonce);
  assert.equal(h.session.cancelRefusedOverlayCopy(OBJ, firstNonce), true);
  assert.equal(h.session.overlay?.copy, null);
  assert.equal(h.ipc.calls.length, callsBeforeCancel, "cancelling a known-no-write request makes no native call");

  await h.session.copyOverlayChoice(OBJ, fps.choice, fps.label, "copy");
  const replacementNonce = h.session.overlay?.copy?.edit?.nonce;
  assert.ok(replacementNonce);
  assert.notEqual(replacementNonce, firstNonce);
  assert.equal(h.session.cancelRefusedOverlayCopy(OBJ, firstNonce), false, "a delayed click for A cannot cancel B");
  assert.equal(h.session.overlay?.copy?.edit?.nonce, replacementNonce);
  assert.equal(h.session.cancelRefusedOverlayCopy(OBJ, replacementNonce), true);
});

test("cancelling one of eight clean Copy refusals releases capacity for a ninth document", async () => {
  const h = await harness();
  const objects = Array.from({ length: 9 }, (_, i) => id32(0x40 + i));
  const historical = flipnoteContent({ title: "retained", frames: [] });
  h.ipc.on("studio_list", () => ordinaryView(indexContent({
    objects: Object.fromEntries(objects.map((object, i) => [object, indexEntry({ title: `copy ${i}` })])),
  })));
  h.ipc.on("studio_read", () => docView([]));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: { branch: id64(2), content: id64(3), generation: "1", accepted: 1 },
    prepared: false, eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null,
    unconfirmedState: null, archive: null, disposed: null, transferred: false,
  }));
  h.ipc.on("studio_overlay_read", (a) => ({
    v: 1, kind: "local-draft", channel: a.channel, object: a.object ?? null,
    eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null,
    unconfirmedState: null, basis: id64(1), branch: id64(2), contentId: id64(3), generation: "1",
    accepted: 1, transferState: "active", provenance: "closing", replayable: true, readOnly: true,
    content: historical,
  }));
  h.ipc.on("studio_overlay_copy_preview", (a) => ({
    v: 1, kind: "overlayCopyPreview",
    source: { channel: a.channel, object: a.object ?? null }, destination: a.destination,
    epochId: id32(8), expectedProjection: id64(9), disposition: "ready",
    body: '{"field":"title","op":"set_header","value":"retained"}', originalAuthor: ME,
    sourceOps: [id64(10)], preservesBranch: false, provisional: true,
  }));
  h.ipc.on("studio_overlay_copy_apply", (a) => ({
    v: 1, kind: "overlayCopyRefused", channel: a.channel, object: a.object ?? null,
    reason: "stalePreview", retry: "repreview", written: false,
  }));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  let firstNonce = "";
  for (const [i, object] of objects.slice(0, 8).entries()) {
    h.session.open(object);
    await h.sync();
    const choice = h.session.overlayChoices("copy").find((x) => x.choice.kind === "title");
    assert.ok(choice);
    await h.session.copyOverlayChoice(object, choice.choice, choice.label, "copy");
    assert.equal(h.session.overlay?.copy?.status, "refused");
    if (i === 0) firstNonce = h.session.overlay?.copy?.edit?.nonce ?? "";
  }
  assert.ok(firstNonce);
  const callsBeforeCancel = h.ipc.calls.length;
  assert.equal(h.session.cancelRefusedOverlayCopy(objects[0], firstNonce), true);
  assert.equal(h.ipc.calls.length, callsBeforeCancel, "capacity release is frontend-only");

  h.session.open(objects[8]);
  await h.sync();
  const ninth = h.session.overlayChoices("copy").find((x) => x.choice.kind === "title");
  assert.ok(ninth);
  await h.session.copyOverlayChoice(objects[8], ninth.choice, ninth.label, "copy");
  assert.equal(h.session.overlay?.copy?.status, "refused");
  assert.equal(h.calls("studio_overlay_copy_apply").length, 9);
});

test("a later stale Copy refusal cannot erase an earlier ambiguous Apply", async () => {
  const h = await harness();
  const historical = flipnoteContent({ title: "retained", frames: [] });
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([]));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({ v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: { branch: id64(2), content: id64(3), generation: "1", accepted: 1 }, prepared: false,
    eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null, unconfirmedState: null,
    archive: null, disposed: null, transferred: false }));
  h.ipc.on("studio_overlay_read", (a) => ({ v: 1, kind: "local-draft", channel: a.channel, object: a.object ?? null,
    eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null, unconfirmedState: null,
    basis: id64(1), branch: id64(2), contentId: id64(3), generation: "1", accepted: 1,
    transferState: "active", provenance: "closing", replayable: true, readOnly: true, content: historical }));
  h.ipc.on("studio_overlay_copy_preview", () => ({ v: 1, kind: "overlayCopyPreview",
    source: { channel: CHANNEL, object: OBJ }, destination: { channel: CHANNEL, object: OBJ }, epochId: id32(8),
    expectedProjection: id64(9), disposition: "ready", body: '{"field":"title","op":"set_header","value":"retained"}',
    originalAuthor: ME, sourceOps: [id64(10)], preservesBranch: false, provisional: true }));
  let applies = 0;
  h.ipc.on("studio_overlay_copy_apply", (a) => {
    applies++;
    if (applies === 1 || applies === 3) throw new Error("delivery lost after possible publication");
    return { v: 1, kind: "overlayCopyRefused", channel: a.channel, object: a.object ?? null, reason: "stalePreview", retry: "repreview", written: false };
  });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  const choice = h.session.overlayChoices("copy").find((x) => x.choice.kind === "title");
  assert.ok(choice);
  await h.session.copyOverlayChoice(OBJ, choice.choice, choice.label, "copy");
  const exact = structuredClone(h.calls("studio_overlay_copy_apply")[0].args);
  await h.session.retryOverlayCopy(OBJ);
  assert.equal(h.session.overlay?.copy?.status, "uncertain");
  assert.equal(h.session.overlay?.copy?.mayHaveCommitted, true);
  assert.match(h.session.overlay?.copy?.error ?? "", /earlier delivery may already have committed/);
  assert.deepEqual(h.calls("studio_overlay_copy_apply")[1].args, exact);
  const nativeCallsBeforeCancel = h.ipc.calls.length;
  assert.equal(h.session.cancelRefusedOverlayCopy(OBJ, exact.edit.nonce), false, "a correct nonce cannot cancel uncertainty");
  assert.equal(h.session.overlay?.copy?.edit?.nonce, exact.edit.nonce, "the ambiguous exact request remains retained");
  assert.equal(h.ipc.calls.length, nativeCallsBeforeCancel, "a refused cancellation attempt cannot mutate native state");
  await h.session.repreviewOverlayCopy(OBJ);
  assert.equal(h.calls("studio_overlay_copy_preview").length, 1, "ambiguous identity cannot be replaced by a new preview");
  const nativeCallsBeforeFirstForget = h.ipc.calls.length;
  assert.equal(h.session.forgetOverlayCopy(OBJ, exact.edit.nonce), true);
  assert.equal(h.session.overlay?.copy, null, "the explicitly forgotten frontend identity releases its slot");
  assert.equal(h.ipc.calls.length, nativeCallsBeforeFirstForget, "forgetting uncertainty performs no backend mutation");
  await h.session.copyOverlayChoice(OBJ, choice.choice, choice.label, "copy");
  assert.equal(h.calls("studio_overlay_copy_preview").length, 2, "the user can deliberately start a new preview");
  const replacement = h.calls("studio_overlay_copy_apply")[2].args.edit;
  assert.notEqual(replacement.nonce, exact.edit.nonce, "the new decision cannot reuse the ambiguous operation identity");
  assert.equal(h.session.overlay?.copy?.status, "uncertain", "the replacement has its own ambiguous delivery");
  const nativeCallsBeforeReplacementForget = h.ipc.calls.length;
  assert.equal(h.session.forgetOverlayCopy(OBJ, exact.edit.nonce), false);
  assert.equal(h.session.overlay?.copy?.edit?.nonce, replacement.nonce, "A's stale confirmation cannot erase uncertain B");
  assert.equal(h.session.forgetOverlayCopy(OBJ, replacement.nonce), true);
  assert.equal(h.session.overlay?.copy, null, "B's own confirmation releases B");
  assert.equal(h.ipc.calls.length, nativeCallsBeforeReplacementForget, "neither stale nor current forget mutates the backend");
});

test("an applying retained-draft copy survives navigation and fences a newer run from its late response", async () => {
  const h = await harness();
  const other = id32(0x0c);
  const historical = flipnoteContent({ title: "retained", fps: 6, frames: [] });
  const firstApply = deferred<unknown>();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([]));
  h.ipc.on("studio_overlay_lifecycle", (a) => ({
    v: 1, kind: "overlayLifecycle", channel: a.channel, object: a.object ?? null,
    branch: { branch: id64(2), content: id64(3), generation: "1", accepted: 1 },
    prepared: false, eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null,
    unconfirmedState: null, archive: null, disposed: null, transferred: false,
  }));
  h.ipc.on("studio_overlay_read", (a) => ({
    v: 1, kind: "local-draft", channel: a.channel, object: a.object ?? null,
    eligibility: "manual", manualReason: "sourceReplaced", handoffAttempt: null,
    unconfirmedState: null, basis: id64(1), branch: id64(2), contentId: id64(3), generation: "1",
    accepted: 1, transferState: "active", provenance: "closing", replayable: true, readOnly: true,
    content: historical,
  }));
  h.ipc.on("studio_overlay_copy_preview", (a) => ({
    v: 1, kind: "overlayCopyPreview",
    source: { channel: CHANNEL, object: OBJ }, destination: { channel: CHANNEL, object: OBJ },
    epochId: id32(8), expectedProjection: id64(9), disposition: "ready",
    body: JSON.stringify({ op: "set_header", field: a.choice.kind, value: a.choice.kind === "title" ? "retained" : 6 }),
    originalAuthor: ME, sourceOps: [id64(10)], preservesBranch: false, provisional: true,
  }));
  let applies = 0;
  h.ipc.on("studio_overlay_copy_apply", (a) => {
    applies++;
    if (applies === 1) return firstApply.promise;
    return { v: 1, kind: "overlayCopyApplied", channel: a.channel, object: a.object ?? null, contentSaved: true, alreadySaved: false, provisional: true, branchPreserved: false };
  });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  const title = h.session.overlayChoices("copy").find((x) => x.choice.kind === "title");
  const fps = h.session.overlayChoices("copy").find((x) => x.choice.kind === "fps");
  assert.ok(title && fps);
  const first = h.session.copyOverlayChoice(OBJ, title.choice, title.label, "copy");
  await settle(2);
  assert.equal(h.calls("studio_overlay_copy_apply").length, 1);
  const applyingNonce = h.session.overlay?.copy?.edit?.nonce;
  assert.ok(applyingNonce);
  assert.equal(h.session.overlay?.copy?.status, "applying");
  assert.equal(h.session.cancelRefusedOverlayCopy(OBJ, applyingNonce), false, "an in-flight Apply cannot be cancelled locally");
  assert.equal(h.session.overlay?.copy?.edit?.nonce, applyingNonce);
  assert.equal(h.calls("studio_overlay_copy_apply").length, 1, "the rejected cancellation makes no second Apply call");
  h.session.open(other);
  await h.sync();
  h.session.open(OBJ);
  await h.sync();
  await assert.rejects(
    h.session.copyOverlayChoice(OBJ, fps.choice, fps.label, "copy"),
    /previous local draft copy is unresolved/,
  );
  firstApply.resolve({ v: 1, kind: "overlayCopyApplied", channel: CHANNEL, object: OBJ, contentSaved: true, alreadySaved: false, provisional: true, branchPreserved: false });
  await first;
  await h.session.copyOverlayChoice(OBJ, fps.choice, fps.label, "copy");
  assert.equal(h.session.overlay?.copy?.label, fps.label);
  assert.equal(h.session.overlay?.copy?.status, "applied");
  assert.equal(h.calls("studio_overlay_copy_apply").length, 2, "the newer run starts only after the older response is classified");
});

test("a frame save publishes then applies with the view's epoch, and an uncertain save retries the identical request", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView());
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  const pix = pixBytes(3);
  h.ipc.on("publish_pix", (a) => ({ cid: id64(0xc3), bytes: base64ToBytes(String(a.bytesB64)).length }));
  let applies = 0;
  // A failure after dispatch: uncertain. (The bridge's pre-dispatch "busy" refusals are definite
  // no-writes since nativeNoWrite, and are resent automatically.)
  h.ipc.on("studio_apply", () => { applies++; throw new Error("Studio transaction expired, cancelled or server stopped"); });
  h.session.saveFrame(OBJ, F1, pix);
  await h.sync();
  const rec = h.session.saves[0];
  assert.equal(rec.status, "uncertain");
  assert.match(rec.error, /expired/);
  assert.ok(rec.kind === "frame");
  assert.deepEqual(rec.pix, pix, "the unsaved pixels stay on the record");
  assert.deepEqual(h.session.unsavedPix(OBJ, F1), pix);
  const firstApply = h.calls("studio_apply")[0].args;
  assert.equal(firstApply.epochId, id32(0xe1));
  assert.deepEqual(JSON.parse(String(firstApply.body)), { op: "replace_frame", frame: F1, cid: id64(0xc3), bytes: pix.length });
  assert.equal(firstApply.body, canonicalJson(JSON.parse(String(firstApply.body))));
  // Retry: same nonce, same epoch, same body, no second publication.
  h.ipc.on("studio_apply", () => docView([{ id: F1, cid: id64(0xc3), bytes: pix.length }]));
  h.session.retry(rec.id);
  await h.sync();
  assert.equal(h.calls("publish_pix").length, 1);
  assert.deepEqual(h.calls("studio_apply")[1].args, firstApply);
  assert.equal(h.session.saves.length, 0, "landed saves leave the list");
  assert.equal(h.session.doc?.model?.frames[0].cid, id64(0xc3), "the returned view replaces the projection");
  assert.equal(applies, 1);
});

test("a storage-busy refusal is repeated silently with the identical request, and lands when storage frees up", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView());
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  let applies = 0;
  h.ipc.on("studio_apply", () => { if (++applies <= 2) throw new Error("Studio storage busy; retry the same request"); return docView([{ id: F1, cid: id64(1), bytes: pixBytes(1).length }], { epochId: id32(0xe1) }); });
  h.session.setTitle(OBJ, "quiet please");
  await h.sync();
  assert.equal(applies, 3, "two refusals, then the write");
  const sent = h.calls("studio_apply").map((c) => c.args);
  assert.deepEqual(sent[0], sent[1]); assert.deepEqual(sent[1], sent[2]);
  assert.equal(h.session.saves.length, 0, "nothing was ever shown as failed");
});

test("a storage-busy refusal that outlasts every pause is known not written, not uncertain", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView());
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  let applies = 0;
  h.ipc.on("studio_apply", () => { applies++; throw new Error("Studio storage busy; retry the same request"); });
  h.session.setTitle(OBJ, "still busy");
  // Each pass fires one pause of the chain: the wrapper's five per send, the session's three resends.
  for (let pass = 0; pass < 12; pass++) await h.sync();
  const rec = h.session.saves[0];
  assert.equal(rec.status, "blocked", "native refuses before executing, so nothing was written");
  assert.equal(rec.mayHaveCommitted, false);
  assert.match(rec.error, /busy/);
  // Two bounded layers (see BUSY_BACKOFF_MS): the session sends four times (one plus three
  // resends), and the wrapper repeats each send five times before the session sees the refusal.
  assert.equal(applies, 4 * 6, "four session sends, each one attempt plus the five bounded repeats");
  assert.equal(h.session.canReauthor(rec.id), true, "a known-no-write save may be re-authored or discarded");
  // The same holds for the overlay status reads: the banner's error is the refusal itself, and
  // only after the same bounded repeats (the read is left running while the pauses are fired).
  let reads = 0;
  h.ipc.on("studio_overlay_read", () => { reads++; throw new Error("Studio storage busy; retry the same request"); });
  void h.session.refreshOverlay(OBJ);
  await h.sync(); await h.sync();
  assert.equal(reads, 6);
  assert.match(h.session.overlay?.error ?? "", /busy/);
});

test("a later read cannot re-author over an ambiguous save; its exact identity stays frozen", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView());
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.ipc.on("studio_apply", () => { throw new Error("Studio request cancelled; its local save may have completed"); });
  h.session.setFps(OBJ, 8);
  await h.sync();
  const rec = h.session.saves[0];
  assert.equal(rec.status, "uncertain");
  assert.equal(rec.epochChanged, false);
  h.ipc.on("studio_read", () => docView(undefined, { epochId: id32(0xe9) }));
  h.ipc.emit("studio-updated", { server: SERVER, channel: CHANNEL, object: OBJ });
  h.fire();
  await h.sync();
  assert.equal(h.session.saves[0].epochChanged, true, "the frozen request now names an older epoch");
  const before = h.calls("studio_apply")[0].args;
  h.ipc.on("studio_apply", () => docView());
  assert.equal(h.session.canReauthor(rec.id), false, "possible prior publication cannot be relabeled as a clean new edit");
  h.session.reauthor(rec.id);
  await h.sync();
  assert.equal(h.calls("studio_apply").length, 1);
  assert.equal(h.session.saves[0].id, rec.id);
  const { server: _server, channel: _channel, ...frozen } = before;
  assert.deepEqual(h.session.saves[0].kind === "apply" ? h.session.saves[0].request : null, frozen);
  h.session.retry(rec.id);
  await h.sync();
  assert.deepEqual(h.calls("studio_apply")[1].args, before, "retry keeps the old epoch, nonce and body");
});

test("create freezes object, nonce, title and timestamp; a retry after failure reuses all four", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  await h.sync();
  h.ipc.on("studio_create", () => { throw new Error("Studio transaction expired, cancelled or server stopped"); });
  const object = h.session.createFlipnote("moon cat");
  await h.sync();
  h.tick(5_000);
  const first = h.calls("studio_create")[0].args;
  assert.equal(first.object, object);
  assert.equal(first.createdAtMs, 50_000);
  h.ipc.on("studio_create", () => docView([]));
  h.session.retry(h.session.saves[0].id);
  await h.sync();
  assert.deepEqual(h.calls("studio_create")[1].args, first);
  assert.equal(h.session.saves.length, 0);
  assert.equal(h.session.known.get(object)?.phase, "open");
});

test("index edits produce the three expiry forms, a tombstone, and a rename that also updates the header", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  h.ipc.on("studio_read", () => docView());
  h.ipc.on("studio_apply_index", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "renamed" }) } })));
  h.ipc.on("studio_apply", () => docView());
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.setEntryExpiry(OBJ, { kind: "unrecorded" });
  h.session.setEntryExpiry(OBJ, { kind: "never" });
  h.session.setEntryExpiry(OBJ, { kind: "at", ms: 0 });
  h.session.deleteEntry(OBJ);
  h.session.setTitle(OBJ, "renamed");
  await h.sync();
  const bodies = h.calls("studio_apply_index").map((c) => String(c.args.body));
  assert.equal(bodies[0], `{"object":"${OBJ}","op":"set_expiry"}`);
  assert.equal(bodies[1], `{"expiry":null,"object":"${OBJ}","op":"set_expiry"}`);
  assert.equal(bodies[2], `{"expiry":0,"object":"${OBJ}","op":"set_expiry"}`);
  assert.equal(bodies[3], `{"object":"${OBJ}","op":"tombstone_object"}`);
  assert.equal(bodies[4], `{"object":"${OBJ}","op":"set_title","title":"renamed"}`);
  assert.equal(String(h.calls("studio_apply")[0].args.body), `{"field":"title","op":"set_header","value":"renamed"}`);
  assert.equal(h.session.indexModel?.entries[0].title, "renamed");
});

test("fetches are bounded by the declared size, limited in flight, ordered by priority and fenced by scope", async () => {
  const h = await harness();
  const pix = pixBytes(4);
  const frames = [1, 2, 3, 4].map((n) => ({ id: id32(0x20 + n), cid: id64(0x40 + n), bytes: pix.length }));
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView(frames));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  const holds = new Map<string, ReturnType<typeof deferred<unknown>>>();
  h.ipc.on("request_blob_bounded", (a) => { const d = deferred<unknown>(); holds.set(String(a.cid), d); return d.promise; });
  h.session.want(frames[0].cid, pix.length, 2);
  h.session.want(frames[1].cid, pix.length, 2);
  h.session.want(frames[2].cid, pix.length, 0); // on screen: jumps the queue
  h.session.want(frames[3].cid, pix.length, 1);
  assert.equal(h.calls("request_blob_bounded").length, 2, "at most two in flight");
  assert.equal(h.session.blobState(frames[2].cid), "queued");
  assert.deepEqual(h.calls("request_blob_bounded")[0].args, { server: SERVER, cid: frames[0].cid, maxBytes: pix.length });
  holds.get(frames[0].cid)!.resolve({ bytes_b64: b64(pix), bytes: pix.length });
  await h.sync();
  assert.equal(h.calls("request_blob_bounded")[2].args.cid, frames[2].cid, "priority 0 runs before priority 1 and 2");
  assert.deepEqual(h.session.blob(frames[0].cid), pix);
  // A body of the wrong length is rejected by the adapter and recorded as invalid, not held.
  holds.get(frames[1].cid)!.resolve({ bytes_b64: b64(pix.subarray(0, 8)), bytes: pix.length });
  await h.sync();
  assert.equal(h.session.blobState(frames[1].cid), "invalid");
  // Unavailable is remembered and not re-asked on the next repaint...
  holds.get(frames[2].cid)!.resolve(null);
  await h.sync();
  assert.equal(h.session.blobState(frames[2].cid), "unavailable");
  const asked = h.calls("request_blob_bounded").length;
  h.session.want(frames[2].cid, pix.length, 0);
  assert.equal(h.calls("request_blob_bounded").length, asked);
  // ...until the object is updated.
  h.ipc.emit("studio-updated", { server: SERVER, channel: CHANNEL, object: OBJ });
  h.session.want(frames[2].cid, pix.length, 0);
  assert.equal(h.calls("request_blob_bounded").length, asked + 1);
  // A result that arrives after a scope change is discarded.
  h.session.setScope({ server: SERVER, channel: "1" });
  holds.get(frames[3].cid)!.resolve({ bytes_b64: b64(pix), bytes: pix.length });
  await h.sync();
  assert.equal(h.session.blob(frames[3].cid), undefined);
});

test("events for another server or channel are ignored; matching ones coalesce into one re-read each", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView());
  h.ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  h.session.watchRecovery(OBJ);
  await h.sync();
  const lists = h.calls("studio_list").length, reads = h.calls("studio_read").length, rec = h.calls("studio_recovery_list").length;
  h.ipc.emit("studio-updated", { server: SERVER + 1, channel: CHANNEL, object: OBJ });
  h.ipc.emit("studio-updated", { server: SERVER, channel: "5", object: OBJ });
  h.fire();
  await h.sync();
  assert.equal(h.calls("studio_list").length, lists);
  h.ipc.emit("studio-updated", { server: SERVER, channel: CHANNEL, object: OBJ });
  h.ipc.emit("studio-updated", { server: SERVER, channel: CHANNEL, object: null });
  h.ipc.emit("settlement-changed", { server: SERVER, docType: 16, logicalKey: OBJ, channel: CHANNEL, object: OBJ, state: "recoveryAvailable" });
  h.ipc.emit("settlement-changed", { server: SERVER, docType: 16, logicalKey: OBJ, channel: CHANNEL, object: OBJ, state: "closing" });
  h.fire();
  await h.sync();
  assert.equal(h.calls("studio_list").length, lists + 1);
  assert.equal(h.calls("studio_read").length, reads + 1);
  assert.equal(h.calls("studio_recovery_list").length, rec + 1);
  h.ipc.emit("studio-receive-paused", { server: SERVER });
  assert.equal(h.session.receivePaused, true);
  h.session.open(OBJ);
  await h.sync();
  assert.equal(h.session.receivePaused, false, "a successful explicit read resumes the receiver");
});

test("all nine native settlement states invalidate media and reread the matching document and recovery", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView());
  h.ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ }));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  h.session.watchRecovery(OBJ);
  await h.sync();
  // Pin the native vocabulary independently: a companion refreshRequired notice must not
  // conceal rejection of repairing/storageRefused at the documented frontend boundary.
  for (const state of ["open", "closing", "settled", "fault", "recoveryAvailable",
    "recoveryEvictionPending", "refreshRequired", "repairing", "storageRefused"]) {
    const reads = h.calls("studio_read").length, recovery = h.calls("studio_recovery_list").length;
    const media = h.session.mediaGeneration;
    for (const foreign of [{ server: SERVER + 1, channel: CHANNEL }, { server: SERVER, channel: "9" }]) {
      h.ipc.emit("settlement-changed", { ...foreign, object: OBJ, logicalKey: OBJ, docType: 16, state });
    }
    assert.equal(h.session.mediaGeneration, media, `${state} cannot invalidate a different scope`);
    h.ipc.emit("settlement-changed", { server: SERVER, channel: CHANNEL, object: OBJ,
      logicalKey: OBJ, docType: 16, state });
    assert.equal(h.session.mediaGeneration, media + 1, `${state} revokes prepared media immediately`);
    await h.sync();
    assert.equal(h.calls("studio_read").length, reads + 1, `${state} rereads the object`);
    assert.equal(h.calls("studio_recovery_list").length, recovery + 1, `${state} rereads recovery`);
  }
  const media = h.session.mediaGeneration, reads = h.calls("studio_read").length;
  h.ipc.emit("settlement-changed", { server: SERVER, channel: CHANNEL, object: OBJ,
    logicalKey: OBJ, docType: 16, state: "repairGranted" });
  await h.sync();
  assert.equal(h.session.mediaGeneration, media, "unknown state cannot acquire contract meaning");
  assert.equal(h.calls("studio_read").length, reads);
  h.session.detach();
});

test("reset forgets scoped state, pending saves and held pixels; late results do not repopulate it", async () => {
  const h = await harness();
  const pending = deferred<unknown>();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView());
  h.ipc.on("publish_pix", () => pending.promise);
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.saveFrame(OBJ, F1, pixBytes(5));
  assert.equal(h.session.saves.length, 1);
  h.session.reset();
  assert.equal(h.session.saves.length, 0);
  assert.equal(h.session.doc, null);
  pending.resolve({ cid: id64(1), bytes: pixBytes(5).length });
  await h.sync();
  assert.equal(h.session.saves.length, 0);
  assert.equal(h.calls("studio_apply").length, 0, "the stale publication never turns into an apply");
});

test("the recovery walk previews each choice in timeline order, holds pixels before applying, and records dispositions", async () => {
  const h = await harness();
  const pix = pixBytes(6);
  const snap = id64(0x501);
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([]));
  h.ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ, versions: [version(1)] }));
  h.ipc.on("studio_recovery_read", () => ({
    v: 1, kind: "recoveryVersion", historical: true, version: version(1), channel: CHANNEL,
    content: flipnoteContent({ frames: [{ id: F1, cid: id64(0x61), bytes: pix.length, op: 300 }, { id: F2, cid: id64(0x62), bytes: pix.length, op: 301, author: ROOK }] }),
  }));
  h.ipc.on("studio_recovery_preview", (a) => {
    const choice = a.choice as { kind: string; id: string; value: string };
    if (choice.id === F1) return { v: 1, kind: "recoveryPreview", snapshot: snap, epochId: id32(0xe1), expectedProjection: id64(0x70), disposition: "ready", body: "{\"op\":\"insert_frame\"}", originalAuthor: ME };
    return { v: 1, kind: "recoveryPreview", snapshot: snap, epochId: id32(0xe1), expectedProjection: id64(0x71), disposition: "conflict", body: null, originalAuthor: ROOK };
  });
  h.ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: pix.length }));
  h.ipc.on("publish_pix", () => ({ cid: id64(0x61), bytes: pix.length }));
  h.ipc.on("studio_recovery_apply", () => ({ v: 1, kind: "recoveryApplied", contentSaved: true, alreadySaved: false, provisional: true, pointerRestored: false }));
  h.ipc.on("studio_recovery_restore_pointer", () => { throw new Error("Registry bucket full"); });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  h.session.watchRecovery(OBJ);
  await h.sync();
  await h.session.runRecovery(snap, "restore");
  const run = h.session.recoveryRun!;
  assert.equal(run.status, "done");
  assert.deepEqual(run.items.map((i) => [i.choice.kind, i.state]), [["frame", "applied"], ["frame", "conflict"]]);
  assert.deepEqual(run.items.map((i) => (i.choice as { value?: string }).value), [id64(300), id64(301)], "the choice names the selected pixel value's opId");
  const previews = h.calls("studio_recovery_preview");
  assert.deepEqual(previews[0].args, { server: SERVER, channel: CHANNEL, object: OBJ, snapshot: snap, choice: { kind: "frame", id: F1, value: id64(300) }, mode: "restore" });
  const fetched = h.calls("request_blob_bounded")[0].args;
  assert.deepEqual(fetched, { server: SERVER, cid: id64(0x61), maxBytes: pix.length }, "fetched by the historical cid and declared size");
  const edit = h.calls("studio_recovery_apply")[0].args.edit as Record<string, unknown>;
  assert.equal(edit.epochId, id32(0xe1));
  assert.equal(edit.expectedProjection, id64(0x70));
  assert.equal(edit.body, "{\"op\":\"insert_frame\"}");
  assert.match(String(edit.nonce), /^[0-9a-f]{32}$/);
  assert.ok(h.calls("studio_read").length >= 2, "re-read after an applied choice");
  await h.session.restorePointer();
  assert.equal(h.session.pointer.status, "blocked");
  assert.match(h.session.pointer.error, /bucket full/);
});

test("the eviction warning is acknowledged with the exact listed pair and the listing is replaced", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_recovery_list", () => recoveryListing({ versions: [version(2), version(1), version(3, { staged: true })], evictionPending: { oldestSnapshot: id64(0x501), stagedSnapshot: id64(0x503), deadlineMs: "60000" } }));
  h.ipc.on("studio_recovery_acknowledge", () => recoveryListing({ kind: "recoveryAcknowledged", versions: [version(2), version(3)] }));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.watchRecovery(null);
  await h.sync();
  await h.session.acknowledgeEviction();
  assert.deepEqual(h.calls("studio_recovery_acknowledge")[0].args, { server: SERVER, channel: CHANNEL, oldestSnapshot: id64(0x501), stagedSnapshot: id64(0x503) });
  assert.equal(h.session.recoveryListing?.kind, "recoveryAcknowledged");
  assert.equal(h.session.recoveryListing?.evictionPending, null);
});

test("a queued save keeps its document's epoch after the member opens another flipnote", async () => {
  const h = await harness();
  const OTHER = id32(0x0c);
  const hold = deferred<unknown>();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", (a) => (a.object === OBJ ? docView() : docView([], { epochId: id32(0xe7) })));
  h.ipc.on("publish_pix", () => hold.promise);
  h.ipc.on("studio_apply", () => docView());
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.saveFrame(OBJ, F1, pixBytes(7));
  h.session.open(OTHER);
  await h.sync();
  assert.equal(h.session.doc?.object, OTHER);
  hold.resolve({ cid: id64(0xc7), bytes: pixBytes(7).length });
  await h.sync();
  const apply = h.calls("studio_apply")[0].args;
  assert.equal(apply.object, OBJ);
  assert.equal(apply.epochId, id32(0xe1), "the first flipnote's epoch, not the one on screen");
  assert.equal(h.session.saves.length, 0);
  assert.equal(h.session.doc?.object, OTHER, "landing a save for another document does not steal the screen");
});

test("choosing a conflicted frame's value re-puts it as a real operation even when it is already selected", async () => {
  const h = await harness();
  const pix = pixBytes(8);
  const mine = { id: F1, cid: id64(0x81), bytes: pix.length, conflicts: [{ cid: id64(0x82), bytes: pix.length, author: ROOK, op: 500 }] };
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([mine]));
  h.ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: pix.length }));
  h.ipc.on("publish_pix", () => ({ cid: id64(0x81), bytes: pix.length }));
  h.ipc.on("studio_apply", () => docView([{ id: F1, cid: id64(0x81), bytes: pix.length }]));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  const selected = h.session.doc!.model!.frames[0];
  await h.session.useVersion(OBJ, F1, { cid: selected.cid, bytes: selected.bytes, author: selected.author, ts: selected.ts, opId: selected.opId }, "replace");
  await h.sync();
  assert.equal(h.calls("studio_apply").length, 1, "a forced re-put is sent, not skipped as unchanged");
  assert.deepEqual(JSON.parse(String(h.calls("studio_apply")[0].args.body)), { op: "replace_frame", frame: F1, cid: id64(0x81), bytes: pix.length });
  assert.deepEqual(h.session.doc?.model?.frames[0].conflicts, []);
  // A plain unchanged save is still a no-op.
  h.session.saveFrame(OBJ, F1, pix);
  await h.sync();
  assert.equal(h.calls("studio_apply").length, 1);
  assert.equal(h.session.saves.length, 0);
});
