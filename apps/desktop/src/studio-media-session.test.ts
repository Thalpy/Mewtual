import assert from "node:assert/strict";
import test from "node:test";
import { StudioSession } from "./studio-session.ts";
import { decodePixa } from "./pixa.ts";
import { CHANNEL, ME, SERVER, b64, deferred, fakeIpc, flipnoteContent, id32, id64, indexContent, ordinaryView, pixBytes, settle } from "./studio-testkit.ts";

const object = id32(1), frame = id32(2), cid = id64(3);
async function harness() {
  const ipc = fakeIpc(), timers: (() => void)[] = []; let id = 100;
  const session = new StudioSession({ ipc, me: ME, newId: () => id32(++id), schedule: (fn) => { timers.push(fn); return () => {}; } });
  const content = flipnoteContent({ frames: [{ id: frame, cid, bytes: pixBytes().length }] });
  const view = () => ({ ...ordinaryView(content), group: "deadbeef" });
  ipc.on("studio_list", () => ordinaryView(indexContent())); ipc.on("studio_read", view);
  ipc.on("request_blob_bounded", () => ({ bytes: pixBytes().length, bytes_b64: b64(pixBytes()) }));
  const sync = async () => { for (let i = 0; i < 4; i++) { for (const fn of timers.splice(0)) fn(); await settle(4); } };
  await session.attach(); session.setScope({ server: SERVER, channel: CHANNEL }); session.open(object); await sync();
  return { session, ipc, sync, content, view, calls: (cmd: string) => ipc.calls.filter((call) => call.cmd === cmd) };
}

test("session prepares every exact native frame and publishes blob separately from an exact reference Save", async () => {
  const h = await harness();
  const packageBytes = await h.session.prepareFlipnotePixa(object);
  assert.equal(decodePixa(packageBytes).frames.length, 1);
  const ack = deferred<unknown>(); h.ipc.on("publish_pixa", (args) => ({ cid: id64(4), bytes: atob(String(args.bytesB64)).length }));
  h.ipc.on("studio_apply", () => ack.promise);
  const result = await h.session.publishFlipnotePixa(object);
  assert.equal(result.referenceSaveId, h.session.saves[0].id);
  assert.equal(h.session.saves[0].status, "inflight", "blob acknowledgement is not a Studio reference acknowledgement");
  const apply = h.calls("studio_apply")[0].args;
  assert.equal(JSON.parse(String(apply.body)).cid, id64(4));
  ack.resolve(h.view()); await h.sync(); assert.equal(h.session.saves.length, 0);
});

test("one preparation bound and navigation/invalidation fence prevent stale complete-package success", async () => {
  const h = await harness(), fetch = deferred<unknown>();
  h.ipc.on("request_blob_bounded", () => fetch.promise);
  const pending = h.session.prepareFlipnotePixa(object);
  await assert.rejects(h.session.prepareFlipnotePixa(object), /still running/);
  h.session.invalidate({ objects: [object] });
  fetch.resolve({ bytes: pixBytes().length, bytes_b64: b64(pixBytes()) });
  await assert.rejects(pending, /source changed/);
  await h.sync();
  const bytes = await h.session.prepareFlipnotePixa(object); assert.equal(decodePixa(bytes).frames.length, 1);
});

test("publication after navigation reports durable blob custody but never redirects its reference", async () => {
  const h = await harness(), publication = deferred<unknown>(), started = deferred<void>(); let length = 0;
  h.ipc.on("publish_pixa", (args) => { length = atob(String(args.bytesB64)).length; started.resolve(); return publication.promise; });
  const pending = h.session.publishFlipnotePixa(object); await started.promise;
  h.session.open(id32(9), { read: false }); publication.resolve({ cid: id64(7), bytes: length });
  const result = await pending;
  assert.equal(result.referenceSaveId, null); assert.match(result.warning!, /blob published.*not saved/);
  assert.equal(h.calls("studio_apply").length, 0);
});

test("draft media uses actual draft group/epoch and refuses old metadata instead of base substitution", async () => {
  const h = await harness();
  const draft = { v: 1, kind: "local-draft", channel: CHANNEL, object, eligibility: "manual", manualReason: "unconfirmed",
    handoffAttempt: null, unconfirmedState: "awaitingSource", basis: id64(1), branch: id64(2), contentId: id64(3), generation: "1",
    accepted: 1, transferState: "active", provenance: "unconfirmed", replayable: true, readOnly: true, content: h.content };
  h.ipc.on("studio_overlay_read", () => draft); await h.session.refreshOverlay(object);
  await assert.rejects(h.session.prepareFlipnotePixa(object), /actual native epoch\/group/);
  h.ipc.on("studio_overlay_read", () => ({ ...draft, contentEpoch: "18446744073709551615", group: "aabb" })); await h.session.refreshOverlay(object);
  const bytes = await h.session.prepareFlipnotePixa(object); assert.equal(decodePixa(bytes).take.group, "aabb");
  assert.deepEqual(bytes, await h.session.prepareFlipnotePixa(object), "the actual u64 draft projection produces a deterministic complete package");
});

test("SFX session requires explicit authored hold and retains it in exact retry body", async () => {
  const h = await harness(); h.ipc.on("studio_apply", () => { throw new Error("delivery lost"); });
  assert.throws(() => h.session.setSfx(object, { id: id32(8), frame, patch: id64(9), note: 60, durationMs: 0 }), /authored duration/);
  h.session.setSfx(object, { id: id32(8), frame, patch: id64(9), note: 60, durationMs: 95_000 }); await h.sync();
  const body = h.calls("studio_apply")[0].args; assert.equal(JSON.parse(String(body.body)).duration_ms, 95_000);
  h.session.retry(h.session.saves[0].id); await h.sync(); assert.deepEqual(h.calls("studio_apply")[1].args, body);
});
