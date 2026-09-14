// Regressions from the adversarial review of 0b6e870: save ordering within a lane, worker
// ownership across a scope change, the Index read barrier, one shared fetch scheduler, explicit
// blob retry with transient-versus-content failures, and retry identity for recovery applies.
import assert from "node:assert/strict";
import test from "node:test";
import { base64ToBytes } from "./studio-native.ts";
import { StudioSession } from "./studio-session.ts";
import {
  CHANNEL, ME, SERVER, b64, deferred, fakeIpc, flipnoteContent, id32, id64, indexContent, indexEntry, ordinaryView, pixBytes, recoveryListing, settle, version,
} from "./studio-testkit.ts";

const OBJ = id32(0x0b);
const F1 = id32(0x11), F2 = id32(0x12);

async function harness() {
  const ipc = fakeIpc();
  let seq = 0x1000;
  const timers: (() => void)[] = [];
  const session = new StudioSession({
    ipc,
    me: ME,
    now: () => 50_000,
    newId: () => (++seq).toString(16).padStart(32, "0"),
    schedule: (fn) => { timers.push(fn); return () => { const i = timers.indexOf(fn); if (i >= 0) timers.splice(i, 1); }; },
    maxFetches: 2,
  });
  const fire = () => { const t = timers.splice(0); for (const fn of t) fn(); };
  await session.attach();
  const sync = async () => { for (let i = 0; i < 3; i++) { fire(); await settle(3); } };
  return { ipc, session, sync, calls: (cmd: string) => ipc.calls.filter((c) => c.cmd === cmd) };
}

const docView = (frames = [{ id: F1, cid: id64(1), bytes: pixBytes(1).length }], extra: { epochId?: string; phase?: string } = {}) => ordinaryView(flipnoteContent({ frames }), extra);

test("SAVE-001: an uncertain save blocks later saves of its lane until it is retried, then both run in order", async () => {
  const h = await harness();
  const pixA = pixBytes(1), pixB = pixBytes(2);
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([{ id: F1, cid: id64(0x10), bytes: pixA.length }]));
  h.ipc.on("publish_pix", (a) => ({ cid: id64(0xa0 + base64ToBytes(String(a.bytesB64))[80]), bytes: base64ToBytes(String(a.bytesB64)).length }));
  h.ipc.on("studio_apply", () => { throw new Error("Studio request cancelled; its local save may have completed"); });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.saveFrame(OBJ, F1, pixA);
  await h.sync();
  const first = h.session.saves[0];
  assert.equal(first.status, "uncertain");
  h.session.saveFrame(OBJ, F1, pixB);
  await h.sync();
  assert.equal(h.session.saves.length, 2);
  assert.equal(h.session.saves[1].status, "queued", "the newer pixels wait behind the uncertain save");
  assert.equal(h.calls("studio_apply").length, 1, "the second save did not overtake the first");
  assert.equal(h.session.blockedBehind(first.id), 1);
  assert.deepEqual(h.session.unsavedPix(OBJ, F1), pixB, "the editor still shows the newest strokes");
  // An unrelated lane keeps moving.
  h.ipc.on("studio_apply_index", () => ordinaryView(indexContent()));
  h.session.setEntryExpiry(OBJ, { kind: "never" });
  await h.sync();
  assert.equal(h.calls("studio_apply_index").length, 1);
  // Retry delivers the original request first, then the newer pixels, in order.
  const firstArgs = h.calls("studio_apply")[0].args;
  h.ipc.on("studio_apply", (a) => docView([{ id: F1, cid: String(JSON.parse(String(a.body)).cid), bytes: pixA.length }]));
  h.session.retry(first.id);
  await h.sync();
  const applies = h.calls("studio_apply");
  assert.equal(applies.length, 3);
  assert.deepEqual(applies[1].args, firstArgs, "identical retry");
  assert.equal(JSON.parse(String(applies[2].args.body)).cid, id64(0xa0 + pixB[80]), "then the newer pixels");
  assert.equal(h.session.saves.length, 0);
});

test("SAVE-001: discarding the uncertain save releases its lane; an uncertain create holds its first frame", async () => {
  const h = await harness();
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView());
  h.ipc.on("publish_pix", () => ({ cid: id64(0xb1), bytes: pixBytes(1).length }));
  h.ipc.on("studio_apply", () => { throw new Error("Studio storage busy; retry the same request"); });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.saveFrame(OBJ, F1, pixBytes(1));
  await h.sync();
  h.session.setFps(OBJ, 6);
  await h.sync();
  assert.equal(h.calls("studio_apply").length, 1);
  h.ipc.on("studio_apply", () => docView());
  h.session.discard(h.session.saves[0].id);
  await h.sync();
  assert.equal(h.calls("studio_apply").length, 2, "the fps edit runs once the lane is released");
  assert.equal(JSON.parse(String(h.calls("studio_apply")[1].args.body)).field, "fps");
  // Create then its first frame: the frame cannot run while the create is uncertain.
  h.ipc.on("studio_create", () => { throw new Error("Studio actor busy; retry"); });
  const created = h.session.createFlipnote("new one");
  h.session.open(created, { read: false });
  h.session.insertFrame(created, null, pixBytes(2));
  await h.sync();
  assert.equal(h.session.saves.find((s) => s.kind === "create")?.status, "uncertain");
  assert.equal(h.session.saves.find((s) => s.kind === "frame")?.status, "queued");
  assert.equal(h.calls("publish_pix").length, 1, "no publication for the held-back frame");
});

test("SAVE-002: a save from a cleared scope cannot release the new scope's lane", async () => {
  const h = await harness();
  const OBJ2 = id32(0x0d);
  const oldPublish = deferred<unknown>();
  const newPublish = deferred<unknown>();
  let publishes = 0;
  h.ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [OBJ2]: indexEntry({ title: "two" }) } })));
  h.ipc.on("studio_read", () => docView());
  h.ipc.on("publish_pix", () => (++publishes === 1 ? oldPublish.promise : newPublish.promise));
  h.ipc.on("studio_apply", () => docView());
  h.ipc.on("studio_apply_index", () => ordinaryView(indexContent({ objects: { [OBJ2]: indexEntry({ title: "two", expiry: { kind: "never" } }) } })));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.saveFrame(OBJ, F1, pixBytes(1)); // old scope, waits on native
  await h.sync();
  h.session.setScope({ server: SERVER, channel: "9" });
  h.session.open(OBJ2);
  await h.sync();
  h.session.saveFrame(OBJ2, F2, pixBytes(2)); // B1, waits on native
  h.session.setEntryExpiry(OBJ2, { kind: "never" }); // B2, another lane, queued behind the one worker
  await h.sync();
  assert.equal(h.session.saves.map((s) => s.status).join(","), "inflight,queued");
  oldPublish.resolve({ cid: id64(1), bytes: pixBytes(1).length });
  await h.sync();
  assert.equal(h.calls("studio_apply_index").length, 0, "the old worker's completion did not start B2");
  assert.equal(h.session.saves.map((s) => s.status).join(","), "inflight,queued");
  newPublish.resolve({ cid: id64(2), bytes: pixBytes(2).length });
  await h.sync();
  assert.equal(h.calls("studio_apply_index").length, 1, "B2 runs after B1, under the new worker");
  assert.equal(h.session.saves.length, 0);
});

test("READ-001: a list issued before an Index write answered cannot overwrite the landed write", async () => {
  const h = await harness();
  let lists = 0;
  const heldList = deferred<unknown>();
  h.ipc.on("studio_list", () => (++lists === 1 ? ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })) : heldList.promise));
  h.ipc.on("studio_apply_index", () => ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "renamed" }) } })));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  await h.sync();
  h.session.invalidate({ index: true });
  await h.sync();
  assert.equal(lists, 2, "a second list is in flight");
  h.session.renameEntry(OBJ, "renamed");
  await h.sync();
  assert.equal(h.session.indexModel?.entries[0].title, "renamed");
  heldList.resolve(ordinaryView(indexContent({ objects: { [OBJ]: indexEntry({ title: "moon cat" }) } })));
  await h.sync();
  assert.equal(h.session.indexModel?.entries[0].title, "renamed", "the older list did not land");
  assert.equal(h.session.indexLoading, false);
});

test("FETCH-001: action fetches share the two in-flight slots and dedupe by cid", async () => {
  const h = await harness();
  const pix = pixBytes(4);
  const frames = [1, 2, 3].map((n) => ({ id: id32(0x30 + n), cid: id64(0x50 + n), bytes: pix.length }));
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView(frames));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  const holds: ReturnType<typeof deferred<unknown>>[] = [];
  h.ipc.on("request_blob_bounded", () => { const d = deferred<unknown>(); holds.push(d); return d.promise; });
  h.session.want(frames[0].cid, pix.length, 2);
  h.session.want(frames[1].cid, pix.length, 2);
  const action = h.session.fetchNow(frames[2].cid, pix.length);
  const again = h.session.fetchNow(frames[2].cid, pix.length);
  await h.sync();
  assert.equal(h.calls("request_blob_bounded").length, 2, "the action waits for a slot instead of opening a third request");
  assert.equal(h.session.blobState(frames[2].cid), "queued");
  holds[0].resolve({ bytes_b64: b64(pix), bytes: pix.length });
  await h.sync();
  assert.equal(h.calls("request_blob_bounded").length, 3, "one slot released, one job started");
  assert.equal(h.calls("request_blob_bounded")[2].args.cid, frames[2].cid, "the action runs before any later thumbnail");
  holds[2].resolve({ bytes_b64: b64(pix), bytes: pix.length });
  await h.sync();
  assert.deepEqual(await action, pix);
  assert.deepEqual(await again, pix, "both waiters share one job and one invoke");
  assert.equal(h.calls("request_blob_bounded").filter((c) => c.args.cid === frames[2].cid).length, 1);
});

test("FETCH-001: a scope switch keeps a running fetch's slot, and its completion touches only its own job", async () => {
  const h = await harness();
  const pix = pixBytes(5);
  const cid = id64(0x77);
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([{ id: F1, cid, bytes: pix.length }]));
  const holds: ReturnType<typeof deferred<unknown>>[] = [];
  h.ipc.on("request_blob_bounded", () => { const d = deferred<unknown>(); holds.push(d); return d.promise; });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.want(cid, pix.length, 0);
  h.session.want(id64(0x78), pix.length, 0);
  await h.sync();
  assert.equal(holds.length, 2);
  h.session.setScope({ server: SERVER, channel: "1" });
  h.session.open(OBJ);
  await h.sync();
  h.session.want(cid, pix.length, 0); // same cid, new scope
  await h.sync();
  assert.equal(holds.length, 2, "both slots are still owned by the old scope's running fetches");
  assert.equal(h.session.blobState(cid), "queued", "the new request is queued behind them");
  holds[0].resolve({ bytes_b64: b64(pix), bytes: pix.length }); // old-scope completion for the same cid
  await h.sync();
  assert.equal(h.session.blob(cid), undefined, "a stale result is not held");
  assert.equal(holds.length, 3, "the freed slot starts the new scope's job");
  assert.equal(h.session.blobState(cid), "fetching", "the old completion did not clear the new marker");
  holds[2].resolve({ bytes_b64: b64(pix), bytes: pix.length });
  await h.sync();
  assert.deepEqual(h.session.blob(cid), pix);
});

test("FETCH-002: an explicit retry asks again after unavailable, and a busy failure is not treated as rejected content", async () => {
  const h = await harness();
  const pix = pixBytes(6);
  const cid = id64(0x66);
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([{ id: F1, cid, bytes: pix.length }]));
  h.ipc.on("request_blob_bounded", () => null);
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.want(cid, pix.length, 0);
  await h.sync();
  assert.equal(h.session.blobState(cid), "unavailable");
  h.session.want(cid, pix.length, 0);
  assert.equal(h.calls("request_blob_bounded").length, 1, "a repaint does not re-ask");
  h.ipc.on("request_blob_bounded", () => { throw new Error("Studio storage busy; retry the same request"); });
  h.session.retryBlob(cid, pix.length);
  await h.sync();
  assert.equal(h.calls("request_blob_bounded").length, 2, "the explicit retry asked again");
  assert.equal(h.session.blobState(cid), "failed", "a busy rejection is transient, not invalid content");
  assert.match(h.session.blobProblem(cid), /busy/);
  h.ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: pix.length }));
  h.session.want(cid, pix.length, 0);
  assert.equal(h.calls("request_blob_bounded").length, 2, "failed holds until an invalidation or retry");
  h.session.retryBlob(cid, pix.length);
  await h.sync();
  assert.equal(h.session.blobState(cid), "held");
  assert.deepEqual(h.session.blob(cid), pix);
  // Rejected content stays rejected across an ordinary invalidation, but not across a retry.
  h.ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix.subarray(0, 9)), bytes: pix.length }));
  h.session.retryBlob(id64(0x67), pix.length);
  await h.sync();
  assert.equal(h.session.blobState(id64(0x67)), "invalid");
  h.ipc.emit("studio-updated", { server: SERVER, channel: CHANNEL, object: OBJ });
  h.session.want(id64(0x67), pix.length, 0);
  await h.sync();
  assert.equal(h.session.blobState(id64(0x67)), "invalid", "an update does not re-ask for content the decoder rejected");
});

test("RECOVERY-001: an uncertain recovery apply keeps its exact payload; retry resends it without a new preview", async () => {
  const h = await harness();
  const pix = pixBytes(7);
  const snap = id64(0x511);
  let applies = 0;
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([], { epochId: applies ? id32(0xe5) : id32(0xe1) }));
  h.ipc.on("studio_recovery_list", () => recoveryListing({ object: OBJ, versions: [version(1)] }));
  h.ipc.on("studio_recovery_read", () => ({
    v: 1, kind: "recoveryVersion", historical: true, version: version(1), channel: CHANNEL,
    content: flipnoteContent({ frames: [{ id: F1, cid: id64(0x61), bytes: pix.length, op: 300 }, { id: F2, cid: id64(0x62), bytes: pix.length, op: 301 }] }),
  }));
  h.ipc.on("studio_recovery_preview", (a) => ({ v: 1, kind: "recoveryPreview", snapshot: snap, epochId: id32(0xe1), expectedProjection: id64(0x70), disposition: "ready", body: `{"frame":"${(a.choice as { id: string }).id}","op":"insert_frame"}`, originalAuthor: ME }));
  h.ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: pix.length }));
  h.ipc.on("publish_pix", () => ({ cid: id64(0x61), bytes: pix.length }));
  h.ipc.on("studio_recovery_apply", () => { applies++; throw new Error("Studio request cancelled; its local save may have completed"); });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  h.session.watchRecovery(OBJ);
  await h.sync();
  await h.session.runRecovery(snap, "restore");
  await h.sync();
  const run = h.session.recoveryRun!;
  assert.equal(run.status, "stopped");
  assert.deepEqual(run.items.map((i) => i.state), ["uncertain", "pending"], "the walk stops at the unknown outcome; the second frame is not previewed");
  assert.equal(h.calls("studio_recovery_preview").length, 1);
  const recId = h.session.saves.find((s) => s.kind === "recoveryApply")!.id;
  assert.equal(h.session.saves.find((s) => s.id === recId)?.status, "uncertain");
  // An independent copy: the fake bridge already snapshots arguments, and this must not share
  // an object with the record under test either way.
  const firstEdit = structuredClone(h.calls("studio_recovery_apply")[0].args.edit);
  assert.match(String((firstEdit as { nonce: string }).nonce), /^[0-9a-f]{32}$/);
  // The live projection moves on (another edit lands); the frozen payload must not.
  h.ipc.emit("studio-updated", { server: SERVER, channel: CHANNEL, object: OBJ });
  await h.sync();
  assert.equal(h.session.doc?.view?.epochId, id32(0xe5));
  assert.equal(h.session.saves.find((s) => s.id === recId)?.epochChanged, true, "snapshots are re-read, never held");
  h.ipc.on("studio_recovery_apply", () => ({ v: 1, kind: "recoveryApplied", contentSaved: true, alreadySaved: true, provisional: true, pointerRestored: false }));
  h.session.retry(recId);
  await h.sync();
  assert.deepEqual(h.calls("studio_recovery_apply")[1].args.edit, firstEdit, "same nonce, body, epoch and expected projection");
  assert.equal(h.calls("studio_recovery_preview").length, 1, "no fresh preview, so no second edit");
  assert.equal(h.session.saves.length, 0);
});

test("SAVE-003: newer pixels coalesce only into the lane's tail; an intervening conflict choice keeps its place", async () => {
  const h = await harness();
  const pixA = pixBytes(1), pixB = pixBytes(2), pixC = pixBytes(3), pixD = pixBytes(4);
  const cidOf = (p: Uint8Array) => id64(0xa0 + p[80]);
  const conflict = { cid: cidOf(pixC), bytes: pixC.length, author: ME, ts: 5, opId: id64(0x99) };
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([{ id: F1, cid: id64(0x10), bytes: pixA.length }]));
  h.ipc.on("publish_pix", (a) => { const bytes = base64ToBytes(String(a.bytesB64)); return { cid: cidOf(bytes), bytes: bytes.length }; });
  h.ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pixC), bytes: pixC.length }));
  h.ipc.on("studio_apply", () => { throw new Error("Studio request cancelled; its local save may have completed"); });
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  h.session.saveFrame(OBJ, F1, pixA); // A: goes uncertain with a frozen apply
  await h.sync();
  const aId = h.session.saves[0].id;
  assert.equal(h.session.saves[0].status, "uncertain");
  h.session.saveFrame(OBJ, F1, pixB); // B: queued, no identity yet
  await h.session.useVersion(OBJ, F1, conflict, "replace"); // C: an explicit, forced replacement
  h.session.saveFrame(OBJ, F1, pixD); // D: must not slip into B's slot ahead of C
  await h.sync();
  const order = h.session.saves.map((s) => (s.kind === "frame" ? cidOf(s.pix) : s.kind));
  assert.deepEqual(order, [cidOf(pixA), cidOf(pixB), cidOf(pixC), cidOf(pixD)], "A, B, C, D in the order they were made");
  assert.deepEqual(h.session.unsavedPix(OBJ, F1), pixD);
  const firstArgs = h.calls("studio_apply")[0].args;
  let selected = id64(0x10);
  h.ipc.on("studio_apply", (a) => { selected = String(JSON.parse(String(a.body)).cid); return docView([{ id: F1, cid: selected, bytes: pixA.length }]); });
  h.session.retry(aId);
  await h.sync();
  const applied = h.calls("studio_apply").slice(1).map((c) => String(JSON.parse(String(c.args.body)).cid));
  assert.deepEqual(h.calls("studio_apply")[1].args, firstArgs, "the original request retries identically");
  assert.deepEqual(applied, [cidOf(pixA), cidOf(pixB), cidOf(pixC), cidOf(pixD)]);
  assert.equal(h.session.doc?.model?.frames[0].cid, cidOf(pixD), "the newest drawing is what the frame ends up showing");
  assert.equal(h.session.saves.length, 0);
});

test("FETCH-003: every consumer's declared length is checked, through a shared job and through the cache", async () => {
  const h = await harness();
  const pix = pixBytes(8);
  const cid = id64(0x88);
  const N = pix.length;
  h.ipc.on("studio_list", () => ordinaryView(indexContent()));
  h.ipc.on("studio_read", () => docView([{ id: F1, cid, bytes: N }]));
  // The backend answers with the real N bytes whatever the caller declared; the adapter's
  // exact-length check then refuses the answer for the caller that declared N-1.
  h.ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: N }));
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.session.open(OBJ);
  await h.sync();
  const asked = h.calls("request_blob_bounded").length;
  h.session.want(cid, N, 0);
  const wrong = h.session.fetchNow(cid, N - 1).then(() => "resolved", (e: unknown) => e);
  await h.sync();
  assert.equal(h.calls("request_blob_bounded").length - asked, 2, "different declarations do not share one answer");
  assert.deepEqual(h.calls("request_blob_bounded").slice(asked).map((c) => c.args.maxBytes).sort(), [N - 1, N]);
  assert.match(String((await wrong as Error).message ?? wrong), /declared size/, "the N-1 consumer is refused, not handed N bytes");
  assert.deepEqual(h.session.blob(cid, N), pix);
  assert.equal(h.session.blob(cid, N - 1), undefined, "held bytes of another size are not this record's pixels");
  assert.equal(h.session.blobState(cid, N), "held");
  assert.equal(h.session.blobState(cid, N - 1), "invalid");
  assert.match(h.session.blobProblem(cid, N - 1), /not the/);
  // Through the cache: an action with the wrong declaration is refused, never handed N bytes.
  await assert.rejects(h.session.fetchNow(cid, N - 1), /not the/);
  assert.deepEqual(await h.session.fetchNow(cid, N), pix);
  const before = h.calls("request_blob_bounded").length;
  h.session.want(cid, N - 1, 0);
  await h.sync();
  assert.equal(h.calls("request_blob_bounded").length, before, "a known-wrong declaration is not re-asked");
});
