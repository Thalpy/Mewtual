import assert from "node:assert/strict";
import test from "node:test";
import { StudioSession } from "./studio-session.ts";
import { decodePixa } from "./pixa.ts";
import { parseFlipnoteView, bytesToBase64 } from "./studio-native.ts";
import { flipnotePixaFromNative } from "./studio-resolved-export.ts";
import { CHANNEL, ME, SERVER, b64, deferred, fakeIpc, flipnoteContent, id32, id64, indexContent, ordinaryView, pixBytes, previewView, settle } from "./studio-testkit.ts";

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

test("an unrebuildable draft never exports its ordinary base", async () => {
  const h = await harness();
  h.ipc.on("studio_overlay_read", () => ({ v: 1, kind: "local-draft", channel: CHANNEL, object, eligibility: "manual", manualReason: "unconfirmed",
    handoffAttempt: null, unconfirmedState: "sourceUnreadable", basis: id64(1), branch: id64(2), contentId: null, generation: "1",
    accepted: 1, transferState: "active", provenance: "unconfirmed", replayable: false, readOnly: true, content: null }));
  await h.session.refreshOverlay(object);
  await assert.rejects(h.session.prepareFlipnotePixa(object), /cannot be rebuilt/);
  assert.equal(h.calls("request_blob_bounded").length, 0);
});

test("post-publication reference refusal preserves the real blob receipt", async () => {
  // Closing now has the separate ticketed local-draft route pinned below. Fault still refuses
  // reference authoring outright; neither that refusal nor an RNG failure loses blob custody.
  for (const phase of ["fault"]) {
    const h = await harness(); h.ipc.on("studio_read", () => ({ ...h.view(), phase })); await h.session.refreshDoc(object);
    h.ipc.on("publish_pixa", (args) => ({ cid: id64(5), bytes: atob(String(args.bytesB64)).length }));
    const result = await h.session.publishFlipnotePixa(object);
    assert.equal(result.publication.cid, id64(5)); assert.equal(result.referenceSaveId, null); assert.match(result.warning!, /blob published.*not saved/);
    assert.equal(h.calls("studio_apply").length, 0);
  }
  const h = await harness(); h.ipc.on("publish_pixa", (args) => ({ cid: id64(6), bytes: atob(String(args.bytesB64)).length }));
  Object.defineProperty(h.session, "newId", { value: () => { throw new Error("RNG unavailable"); } });
  const result = await h.session.publishFlipnotePixa(object); assert.equal(result.publication.cid, id64(6)); assert.match(result.warning!, /RNG unavailable/);
});

test("publication during Closing queues only a local reference and preserves custody when its ticket refuses", async () => {
  for (const closeAvailable of [true, false]) {
    const h = await harness(); h.ipc.on("studio_read", () => ({...h.view(), phase: "closing"})); await h.session.refreshDoc(object);
    h.ipc.on("publish_pixa", args => ({cid: id64(5), bytes: atob(String(args.bytesB64)).length}));
    h.ipc.on("studio_closing_overlay_save_begin", args => closeAvailable
      ? {v: 1, kind: "closingOverlaySaveTicket", channel: args.channel, object: args.object, basis: id64(1), branch: id64(2), provenance: "closing"}
      : {v: 1, kind: "closingOverlaySave", channel: args.channel, object: args.object, state: "refused", reason: "receiptChanged", retry: "newTicket"});
    h.ipc.on("studio_closing_overlay_save", args => ({v: 1, kind: "closingOverlaySave", channel: args.channel, object: args.object, state: "pending", retry: "sameRequest"}));
    const result = await h.session.publishFlipnotePixa(object); await h.sync();
    assert.equal(result.publication.cid, id64(5)); assert.equal(result.referenceSaveId, h.session.saves[0].id);
    assert.equal(result.warning, null, "a queued reference is distinct from an acknowledged Save");
    const save = h.session.saves[0]; assert.equal(save.kind, "overlayApply");
    if (save.kind !== "overlayApply") assert.fail("Closing must never authorize ordinary Apply");
    assert.equal(save.route, "closing"); assert.equal(save.op.op, "set_export");
    if (save.op.op !== "set_export") assert.fail("wrong reference operation");
    assert.equal(save.op.cid, result.publication.cid); assert.equal(save.op.bytes, result.publication.bytes);
    assert.equal(save.status, closeAvailable ? "uncertain" : "blocked"); assert.equal(save.mayHaveCommitted, closeAvailable);
    assert.equal(h.calls("studio_closing_overlay_save_begin").length, 1);
    assert.equal(h.calls("studio_closing_overlay_save").length, closeAvailable ? 1 : 0);
    assert.equal(h.calls("studio_apply").length, 0); assert.equal(h.calls("studio_unconfirmed_overlay_save").length, 0);
  }
});

test("a successful Closing reference Save retains the actual blob receipt and uses only the Closing route", async () => {
  const h = await harness(); h.ipc.on("studio_read", () => ({ ...h.view(), phase: "closing" })); await h.session.refreshDoc(object);
  h.ipc.on("publish_pixa", args => ({ cid: id64(5), bytes: atob(String(args.bytesB64)).length }));
  h.ipc.on("studio_closing_overlay_save_begin", a => ({ v: 1, kind: "closingOverlaySaveTicket", channel: a.channel, object: a.object, basis: id64(1), branch: id64(2), provenance: "closing" }));
  h.ipc.on("studio_closing_overlay_save", a => ({ v: 1, kind: "closingOverlaySave", channel: a.channel, object: a.object, state: "saved", basis: id64(1), accepted: 1, localOnly: true, provisional: true, provenance: "closing" }));
  const result = await h.session.publishFlipnotePixa(object); await h.sync();
  assert.equal(result.publication.cid, id64(5)); assert.ok(result.publication.bytes > 0); assert.ok(result.referenceSaveId);
  assert.equal(h.session.saves.length, 0); assert.equal(h.calls("studio_apply").length, 0); assert.equal(h.calls("studio_unconfirmed_overlay_save").length, 0);
  const body = JSON.parse(String((h.calls("studio_closing_overlay_save")[0].args.save as { body: string }).body));
  assert.equal(body.cid, result.publication.cid); assert.equal(body.bytes, result.publication.bytes);
});

test("timeline gaps, duplicate ids and lying aggregate declarations refuse before frame fetches", async () => {
  for (const change of [(c: ReturnType<typeof flipnoteContent>) => c.timeline.push(id32(9)),
    (c: ReturnType<typeof flipnoteContent>) => c.timeline.push(frame), (c: ReturnType<typeof flipnoteContent>) => c.declaredFrameBytes = 1]) {
    const h = await harness(); change(h.content); await h.session.refreshDoc(object);
    await assert.rejects(h.session.prepareFlipnotePixa(object), /timeline|aggregate/); assert.equal(h.calls("request_blob_bounded").length, 0);
  }
});

test("all Score component operations, including the shared title grammar, refuse a Flipnote target", async () => {
  const h = await harness();
  const ops: import("./studio-contract.ts").ScoreOp[] = [{ op: "set_header", field: "title", value: "music" },
    { op: "set_header", field: "bpm", value: 120 }, { op: "set_cell", step: 0, row: 0, value: { k: "d", pad: 0 } },
    { op: "clear_cell", step: 0, row: 0 }, { op: "remove_patch", patch: id64(1) }];
  for (const op of ops) assert.throws(() => h.session.applyScoreOp(object, op), /pinned Score/);
  assert.equal(h.session.saves.length, 0); assert.equal(h.calls("studio_apply").length, 0); assert.equal(h.calls("studio_apply_index").length, 0);
});

test("Flow-S export custody refusals are proven no-write PIXA diagnostics, never ambiguous PIX failures", async () => {
  for (const reason of ["exportUnpublished", "exportInvalid", "exportMissing"]) {
    const h = await harness(); h.ipc.on("studio_read", () => ({ ...previewView(h.content), group: "deadbeef" })); await h.session.refreshDoc(object);
    h.ipc.on("publish_pixa", (args) => ({ cid: id64(5), bytes: atob(String(args.bytesB64)).length }));
    h.ipc.on("studio_unconfirmed_overlay_save_begin", () => ({ v: 1, kind: "unconfirmedOverlaySaveTicket", channel: CHANNEL, object, basis: id64(1), branch: id64(2), provenance: "unconfirmed" }));
    h.ipc.on("studio_unconfirmed_overlay_save", () => ({ v: 1, kind: "unconfirmedOverlaySave", channel: CHANNEL, object, state: "refused", reason, retry: "newTicket" }));
    const result = await h.session.publishFlipnotePixa(object); await h.sync();
    assert.ok(result.referenceSaveId); assert.equal(h.session.saves[0].status, "blocked"); assert.equal(h.session.saves[0].mayHaveCommitted, false);
    assert.match(h.session.saves[0].error, /PIXA/); assert.match(h.session.saves[0].error, new RegExp(reason));
    h.session.retry(h.session.saves[0].id); await h.sync(); assert.equal(h.calls("studio_unconfirmed_overlay_save").length, 1, "newTicket is not an exact retry");
  }
});

test("saved export readback fetches exact package bytes, validates group, and never routes PIXA through the PIX cache", async () => {
  const h = await harness(), exportId = id32(8), exportCid = id64(9);
  const bytes = await flipnotePixaFromNative(object, parseFlipnoteView(h.view()), new Map([[`${cid}:${pixBytes().length}`, pixBytes()]]), ME);
  const content = { ...h.content, sound: { records: { [`x/${exportId}/${id64(44)}`]: { selected: { value: { op: "set_export", export: exportId, cid: exportCid, bytes: bytes.length }, source: { opId: id64(44), author: ME, nonce: id32(44), ts: 44 } }, conflicts: [] } },
    readiness: { sfxOverflow: [], patchOverflow: [], exportOverflow: [], missingFrames: [], missingPatches: [], conflicts: false } } };
  h.ipc.on("studio_read", () => ({ ...h.view(), content })); await h.session.refreshDoc(object);
  h.ipc.on("request_blob_bounded", (args) => { assert.equal(args.cid, exportCid); assert.equal(args.maxBytes, bytes.length); return { bytes_b64: bytesToBase64(bytes), bytes: bytes.length }; });
  assert.deepEqual(await h.session.preparePublishedPixa(object, exportId), bytes);
  assert.equal(h.session.blob(exportCid), undefined, "PIXA is not admitted to the PIX-only thumbnail cache");
  h.ipc.on("studio_read", () => ({ ...h.view(), content, group: "aabb" })); await h.session.refreshDoc(object);
  await assert.rejects(h.session.preparePublishedPixa(object, exportId), /another group/);
});
