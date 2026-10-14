import assert from "node:assert/strict";
import test from "node:test";
import { StudioSession } from "./studio-session.ts";
import { CHANNEL, ME, SERVER, deferred, fakeIpc, flipnoteContent, id32, id64, indexContent, indexEntry, ordinaryView, previewView, recoveryListing, settle } from "./studio-testkit.ts";

const object = id32(42);
function scoreContent() { return { kind: "score", title: null, bpm: null, bpb: null, steps: null, cells: {}, patches: {}, readiness: { cells: {}, patches: {}, headerConflicts: false } }; }
async function harness(preview = false) {
  const ipc = fakeIpc(), timers: (() => void)[] = []; let id = 100;
  const session = new StudioSession({ ipc, me: ME, now: () => 0, newId: () => id32(++id),
    schedule: (fn) => { timers.push(fn); return () => { const index = timers.indexOf(fn); if (index >= 0) timers.splice(index, 1); }; } });
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [object]: indexEntry({ kind: "score", title: "score" }) } })));
  ipc.on("studio_read", () => preview ? previewView(scoreContent()) : ordinaryView(scoreContent()));
  await session.attach(); session.setScope({ server: SERVER, channel: CHANNEL });
  const sync = async () => { for (let pass = 0; pass < 3; pass++) { for (const fn of timers.splice(0)) fn(); await settle(3); } };
  await sync();
  return { ipc, session, sync, calls: (command: string) => ipc.calls.filter((call) => call.cmd === command) };
}

test("Score Index target opens through the same session with retained objectKind on exact Apply retry", async () => {
  const { ipc, session, sync, calls } = await harness();
  session.open(object); await sync();
  assert.equal(calls("studio_read")[0].args.objectKind, "score");
  assert.equal(session.doc?.objectKind, "score"); assert.equal(session.doc?.model, null);
  assert.equal(session.doc?.score?.bpm, 120);
  assert.equal(calls("studio_overlay_read").find((call) => call.args.object === object)?.args.objectKind, "score");
  let attempts = 0;
  ipc.on("studio_apply", () => { if (++attempts === 1) throw new Error("delivery lost"); return ordinaryView(scoreContent()); });
  session.setScoreCell(object, 0, 0, { k: "d", pad: 9 }); await sync();
  assert.equal(session.saves[0]?.status, "uncertain");
  session.retry(session.saves[0].id); await sync();
  assert.deepEqual(calls("studio_apply")[0].args, calls("studio_apply")[1].args);
  assert.equal(calls("studio_apply")[0].args.objectKind, "score");
  assert.deepEqual(JSON.parse(String(calls("studio_apply")[0].args.body)), { op: "set_cell", step: 0, row: 0, value: { k: "d", pad: 9 } });
  assert.equal(session.saves.length, 0);
  assert.throws(() => session.insertFrame(object, null, new Uint8Array([1])), /Flipnote/);
  assert.equal(calls("publish_pix").length, 0);
});

test("Score creation reuses coordinated native Create without inventing a first frame", async () => {
  const { ipc, session, sync, calls } = await harness();
  ipc.on("studio_create", () => ordinaryView(scoreContent()));
  const id = session.createScore("new score"); session.open(id, { read: false }); await sync();
  assert.equal(calls("studio_create")[0].args.objectKind, "score");
  assert.equal(session.doc?.view?.content.kind, "score");
  assert.equal(session.doc?.score?.steps, 16);
  assert.equal(calls("publish_pix").length, 0);
});

test("preview Score Save stays local and preserves kind/ticket/nonce/body on exact retry", async () => {
  const { ipc, session, sync, calls } = await harness(true);
  session.open(object); await sync();
  ipc.on("studio_unconfirmed_overlay_save_begin", () => ({ v: 1, kind: "unconfirmedOverlaySaveTicket", channel: CHANNEL,
    object, basis: id64(1), branch: id64(2), provenance: "unconfirmed" }));
  let attempts = 0;
  ipc.on("studio_unconfirmed_overlay_save", () => ++attempts === 1
    ? { v: 1, kind: "unconfirmedOverlaySave", channel: CHANNEL, object, state: "pending", retry: "sameRequest" }
    : { v: 1, kind: "unconfirmedOverlaySave", channel: CHANNEL, object, state: "saved", basis: id64(1), accepted: 1, localOnly: true, provisional: true, provenance: "unconfirmed" });
  session.clearScoreCell(object, 0, 0); await sync();
  assert.equal(session.saves[0]?.kind, "overlayApply");
  assert.equal(session.saves[0]?.status, "uncertain");
  session.retry(session.saves[0].id); await sync();
  assert.deepEqual(calls("studio_unconfirmed_overlay_save")[0].args, calls("studio_unconfirmed_overlay_save")[1].args);
  assert.equal(calls("studio_unconfirmed_overlay_save")[0].args.objectKind, "score");
  assert.equal(calls("studio_unconfirmed_overlay_save_begin").length, 1);
  assert.equal(calls("studio_apply").length, 0);
});

test("Score recovery choices put recipes first and keep explicit removals Copy-only", async () => {
  const { session } = await harness();
  const raw = scoreContent() as unknown as import("./studio-native.ts").ScoreContent;
  const source = (n: number) => ({ opId: id64(n), author: ME, nonce: id32(n) });
  raw.cells["0:0"] = { removed: true, register: { selected: { value: null, source: source(1) }, conflicts: [] } };
  raw.patches[id64(2)] = { removed: true, register: { selected: { value: null, source: source(3) }, conflicts: [] } };
  const version = { v: 1 as const, kind: "recoveryVersion" as const, historical: true as const, channel: CHANNEL,
    version: { snapshot: id64(4), epoch: "0", staged: false, bytes: 1, reason: "rewound" as const }, content: raw };
  assert.deepEqual(session.choicesFor(version, "restore"), []);
  assert.deepEqual(session.choicesFor(version, "copy").map((choice) => choice.choice), [
    { kind: "scorePatch", patch: id64(2), value: id64(3) }, { kind: "scoreCell", step: 0, row: 0, value: id64(1) },
  ]);
});

test("a deferred Score ACK cannot be redirected by reopening a known object as another family", async () => {
  const { ipc, session, sync, calls } = await harness();
  session.open(object); await sync();
  const ack = deferred<unknown>(); ipc.on("studio_apply", () => ack.promise);
  session.setScoreCell(object, 0, 0, { k: "d", pad: 1 }); await settle(3);
  assert.throws(() => session.open(object, { objectKind: "flipnote", read: false }), /immutable family/);
  assert.throws(() => session.saveFrame(object, id32(9), new Uint8Array([1])), /Flipnote/);
  ack.resolve(ordinaryView(scoreContent())); await sync();
  assert.equal(session.doc?.objectKind, "score"); assert.equal(session.doc?.view?.content.kind, "score");
  assert.equal(session.doc?.model, null); assert.equal(session.saves.length, 0);
  assert.equal(calls("publish_pix").length, 0);
});

test("unknown-family navigation fences old overlay continuations and clears prior-family recovery caches", async () => {
  const { ipc, session, sync, calls } = await harness();
  ipc.on("studio_list", () => ordinaryView(indexContent())); await session.refreshIndex();
  ipc.on("studio_read", () => null);
  const lifecycle = deferred<unknown>();
  ipc.on("studio_overlay_lifecycle", (args) => args.object === object ? lifecycle.promise : ({ v: 1, kind: "overlayLifecycle", channel: CHANNEL,
    object: null, branch: null, prepared: false, eligibility: null, manualReason: null, handoffAttempt: null,
    unconfirmedState: null, archive: null, disposed: null, transferred: false }));
  session.open(object, { objectKind: "score" }); await settle(4);
  session.watchRecovery(object);
  const old = { v: 1 as const, kind: "recoveryVersion" as const, historical: true as const, channel: CHANNEL,
    version: { snapshot: id64(1), epoch: "0", staged: false, bytes: 1, reason: "rewound" as const }, content: scoreContent() as import("./studio-native.ts").ScoreContent };
  session.versions.set(id64(1), old);
  session.open(object, { objectKind: "flipnote", read: false });
  assert.equal(session.versions.size, 0);
  lifecycle.resolve({ v: 1, kind: "overlayLifecycle", channel: CHANNEL, object, branch: null, prepared: false, eligibility: null,
    manualReason: null, handoffAttempt: null, unconfirmedState: null, archive: null, disposed: null, transferred: false });
  await sync();
  assert.equal(calls("studio_overlay_read").filter((call) => call.args.object === object && call.args.objectKind === "score").length, 0,
    "a stale lifecycle result cannot issue another old-family native read");
  assert.equal(session.doc?.objectKind, "flipnote"); assert.equal(session.doc?.score, null); assert.equal(session.doc?.view, null);
});

test("offscreen uncertain Copy pins its original family even after view eviction and Index removal", async () => {
  const { ipc, session, sync, calls } = await harness();
  session.open(object); await sync();
  ipc.on("studio_overlay_copy_preview", () => ({ v: 1, kind: "overlayCopyPreview", source: { channel: CHANNEL, object },
    destination: { channel: CHANNEL, object }, epochId: id32(1), expectedProjection: id64(2), disposition: "ready",
    body: '{"op":"set_header","field":"title","value":"copy"}', originalAuthor: ME, sourceOps: [id64(3)], preservesBranch: false, provisional: true }));
  ipc.on("studio_overlay_copy_apply", () => { throw new Error("delivery lost"); });
  await session.copyOverlayChoice(object, { kind: "scoreHeader", field: "title", value: id64(3) }, "score title", "copy");
  const before = calls("studio_overlay_copy_apply")[0].args;
  assert.equal(session.overlay?.copy?.status, "uncertain");
  ipc.on("studio_list", () => ordinaryView(indexContent())); await session.refreshIndex();
  ipc.on("studio_read", (args) => ordinaryView(args.object === object ? scoreContent() : flipnoteContent({ frames: [] })));
  for (let index = 0; index < 9; index++) { session.open(id32(200 + index)); await sync(); }
  assert.throws(() => session.open(object, { objectKind: "flipnote", read: false }), /immutable family/);
  assert.equal(session.objectKind(object), "score", "offscreen Copy, not the evicted view, retains target identity");
  session.open(object); await sync();
  assert.equal(session.overlay?.copy?.status, "uncertain");
  ipc.on("studio_overlay_copy_apply", () => ({ v: 1, kind: "overlayCopyApplied", channel: CHANNEL, object,
    contentSaved: true, alreadySaved: false, provisional: true, branchPreserved: false }));
  await session.retryOverlayCopy(object);
  assert.deepEqual(calls("studio_overlay_copy_apply")[1].args, before);
  assert.equal(session.overlay?.copy?.status, "applied");
});

test("a pending recovery rail for an offscreen Score cannot populate a reopened unknown Flipnote", async () => {
  const { ipc, session, sync } = await harness();
  ipc.on("studio_list", () => ordinaryView(indexContent())); await session.refreshIndex();
  ipc.on("studio_read", () => null);
  const listing = deferred<unknown>(); ipc.on("studio_recovery_list", () => listing.promise);
  session.open(object, { objectKind: "score", read: false }); session.watchRecovery(object);
  session.open(id32(99), { read: false });
  session.open(object, { objectKind: "flipnote", read: false });
  listing.resolve(recoveryListing({ object })); await sync();
  assert.equal(session.doc?.objectKind, "flipnote");
  assert.equal(session.recoveryListing, null); assert.equal(session.recoveryLoading, false);
  assert.equal(session.recoveryRun, null);
});

test("an offscreen uncertain Score Apply reopens by its retained family after view eviction", async () => {
  const { ipc, session, sync, calls } = await harness();
  session.open(object); await sync();
  ipc.on("studio_apply", () => { throw new Error("delivery lost"); });
  session.setScoreHeader(object, "bpm", 160); await sync();
  const save = session.saves[0], args = calls("studio_apply")[0].args;
  assert.equal(save.status, "uncertain");
  ipc.on("studio_list", () => ordinaryView(indexContent())); await session.refreshIndex();
  ipc.on("studio_read", (request) => ordinaryView(request.object === object ? scoreContent() : flipnoteContent({ frames: [] })));
  for (let index = 0; index < 9; index++) { session.open(id32(300 + index)); await sync(); }
  assert.equal(session.objectKind(object), "score");
  session.open(object); await sync();
  assert.equal(session.doc?.objectKind, "score");
  ipc.on("studio_apply", () => ordinaryView(scoreContent())); session.retry(save.id); await sync();
  assert.deepEqual(calls("studio_apply")[1].args, args);
  assert.equal(session.saves.length, 0);
});
