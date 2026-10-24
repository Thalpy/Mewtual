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

test("opening a flipnote reads it; null is absence; a preview is read-only with a stated reason", async () => {
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
  assert.equal(h.session.canEdit(OBJ), false);
  assert.match((h.session.editableEpoch(OBJ) as { refused: string }).refused, /read-only preview/);
  assert.deepEqual(h.session.known.get(OBJ), { awaiting: true, phase: null, epoch: "4" });
  assert.throws(() => h.session.setFps(OBJ, 8), /read-only preview/);
  h.ipc.on("studio_read", () => docView([], { phase: "closing" }));
  h.session.open(OBJ);
  await h.sync();
  assert.match((h.session.editableEpoch(OBJ) as { refused: string }).refused, /rotating/);
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
  h.ipc.on("studio_apply", () => { applies++; throw new Error("Studio storage busy; retry the same request"); });
  h.session.saveFrame(OBJ, F1, pix);
  await h.sync();
  const rec = h.session.saves[0];
  assert.equal(rec.status, "uncertain");
  assert.match(rec.error, /busy/);
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

test("a later read with a different epoch flags the uncertain save; re-authoring is explicit and mints a new nonce", async () => {
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
  h.session.reauthor(rec.id);
  await h.sync();
  const after = h.calls("studio_apply")[1].args;
  assert.equal(after.epochId, id32(0xe9));
  assert.notEqual(after.nonce, before.nonce);
  assert.equal(after.body, before.body);
});

test("create freezes object, nonce, title and timestamp; a retry after failure reuses all four", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  await h.sync();
  h.ipc.on("studio_create", () => { throw new Error("Studio actor busy; retry"); });
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
