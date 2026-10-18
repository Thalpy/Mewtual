import assert from "node:assert/strict";
import test from "node:test";
import { StudioSession } from "./studio-session.ts";
import { decodeStudioDraft, encodeStudioDraft, validateDraftOperation } from "./studio-draft.ts";
import { decodePixa } from "./pixa.ts";
import { sanitizeUiContinuity, planLegacyReadMarkMigration } from "./ui-continuity.ts";
import { readFileSync } from "node:fs";
import { workingFlipnote, workingScore } from "./studio-working.ts";
import { scoreModel, parseScoreView } from "./studio-native.ts";
import type { SaveRecord } from "./studio-session.ts";
import { canonicalJson } from "./studio-contract.ts";
import { CHANNEL, ME, SERVER, b64, deferred, fakeIpc, flipnoteContent, id32, id64, indexContent, pixBytes, previewView, settle } from "./studio-testkit.ts";
const object = id32(1), frame = id32(2), cid = id64(3), group = "deadbeef";

async function harness(empty = false) {
  const ipc = fakeIpc(); let now = 0, seq = 100;
  let title = "initial", fps = 12, frames = empty ? [] : [{ id: frame, cid, bytes: pixBytes().length }];
  let epoch = id32(0xe1);
  const view = () => ({ ...ordinary(), group });
  const ordinary = () => ({ v: 1, epochId: epoch, epoch: "3", channel: CHANNEL, publication: "local", provisional: true, phase: "open", content: flipnoteContent({ title, fps, frames }) });
  ipc.on("studio_list", () => ({ ...ordinary(), content: indexContent(), group }));
  ipc.on("studio_read", view);
  ipc.on("publish_pix", a => ({ cid: id64(4), bytes: atob(String(a.bytesB64)).length }));
  ipc.on("studio_apply", a => {
    const op = JSON.parse(String(a.body));
    if (op.op === "set_header") { if (op.field === "title") title = op.value; if (op.field === "fps") fps = op.value; }
    else if (op.op === "insert_frame") frames = [...frames, { id: op.frame, cid: op.cid, bytes: op.bytes }];
    else if (op.op === "replace_frame") frames = frames.map(f => f.id === op.frame ? { ...f, cid: op.cid, bytes: op.bytes } : f);
    return view();
  });
  const timers = new Set<{ due: number; fn: () => void }>();
  const session = new StudioSession({ ipc, me: ME, now: () => now, newId: () => id32(++seq), autosaveMs: 5000,
    schedule: (fn, ms) => { const timer = { due: now + ms, fn }; timers.add(timer); return () => { timers.delete(timer); }; } });
  session.setScope({ server: SERVER, channel: CHANNEL }); session.flush(); session.open(object); await settle(8);
  const tick = async (ms: number) => { now += ms; for (const t of [...timers].sort((a, b) => a.due - b.due)) if (timers.has(t) && t.due <= now) { timers.delete(t); t.fn(); } await settle(8); };
  return { ipc, session, tick, view, rotate: async () => { epoch = id32(0xe2); await session.refreshDoc(object); }, writes: () => ipc.calls.filter(c => ["studio_apply", "publish_pix"].includes(c.cmd)) };
}

test("all ordinary edits update locally, coalesce and save at the first five-second deadline", async () => {
  const h = await harness();
  h.session.setFps(object, 8); h.session.setFps(object, 6);
  h.session.saveFrame(object, frame, pixBytes(1)); h.session.saveFrame(object, frame, pixBytes(2));
  assert.equal(h.session.workingModel?.fps, 6); assert.equal(h.session.doc?.model?.fps, 12);
  assert.equal(h.session.saves.length, 2); assert.equal(h.writes().length, 0);
  await h.tick(4999); h.session.setTitle(object, "latest"); assert.equal(h.writes().length, 0);
  await h.tick(1); assert.equal(h.session.saves.length, 0); assert.equal(h.session.doc?.model?.title, "latest");
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 3);
  h.session.reset();
});

test("Save freezes its revision; a newer raster survives the old ACK and waits its deadline", async () => {
  const h = await harness(), ack = deferred<unknown>(); h.ipc.on("studio_apply", () => ack.promise);
  h.session.saveFrame(object, frame, pixBytes(1)); const saving = h.session.save(); await settle(5);
  const media = h.session.editorMediaGeneration;
  h.session.saveFrame(object, frame, pixBytes(2)); const next = h.session.unsavedPix(object, frame);
  ack.resolve(h.view()); await saving; await settle(5);
  assert.equal(h.session.saves.length, 1); assert.equal(h.session.unsavedPix(object, frame), next);
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 1);
  assert.ok(h.session.editorMediaGeneration > media);
  h.session.reset();
});

test("rotation before or during publication refuses repeated first delivery until explicit reauthor", async () => {
  for (const during of [false, true]) {
    const h = await harness(), publication = deferred<unknown>();
    if (during) h.ipc.on("publish_pix", () => publication.promise);
    h.session.saveFrame(object, frame, pixBytes(1));
    if (!during) await h.rotate();
    const saving = assert.rejects(h.session.save()); await settle(3);
    if (during) { await h.rotate(); publication.resolve({ cid: id64(4), bytes: pixBytes(1).length }); }
    await saving; const saved = h.session.saves[0];
    assert.equal(saved.status, "blocked"); assert.equal(saved.mayHaveCommitted, false);
    h.session.retry(saved.id); await settle(5);
    assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 0);
    h.session.reauthor(saved.id); await settle(5);
    assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 1); h.session.reset();
  }
});

test("a new first frame plays from owned pixels without a native write; autosave ACK keeps media generation", async () => {
  const h = await harness(true); h.session.insertFrame(object, null, pixBytes());
  const animation = decodePixa(await h.session.prepareFlipnotePixa(object));
  assert.equal(animation.frames.length, 1); assert.equal(h.writes().length, 0);
  const revision = h.session.editorMediaGeneration; await h.session.save();
  assert.equal(h.session.editorMediaGeneration, revision, "same pixels and sound stay the same playback source");
  h.session.reset();
});

test("lock snapshot owns PIX and restoration never auto-replays retained work", async () => {
  const h = await harness(); h.session.saveFrame(object, frame, pixBytes(2));
  const draft = decodeStudioDraft(h.session.draftSnapshot()!); assert.equal(draft.scope.channel, CHANNEL);
  assert.deepEqual(draft.records[0].kind === "frame" && draft.records[0].pix, pixBytes(2));
  h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); h.session.open(object); await settle(8);
  h.session.setFps(object, 7); await h.tick(5000); assert.equal(h.writes().length, 0, "new autosave cannot release restored edits");
  await h.session.save(); assert.equal(h.session.saves.length, 0); h.session.reset();
});

test("an uncertain request keeps its exact nonce/body and its draft after retry failure", async () => {
  const h = await harness(); h.ipc.on("studio_apply", () => { throw new Error("receipt lost: " + "猫".repeat(5000)); });
  h.session.setTitle(object, "draft"); await assert.rejects(h.session.save());
  const first = h.ipc.calls.find(c => c.cmd === "studio_apply")!;
  assert.equal(h.session.workingModel?.title, "draft");
  h.session.retry(h.session.saves[0].id); await settle(5);
  assert.deepEqual(h.ipc.calls.filter(c => c.cmd === "studio_apply")[1].args, first.args);
  assert.ok(h.session.saves[0].error.length <= 512);
  assert.equal(decodeStudioDraft(h.session.draftSnapshot()!).records[0].error, h.session.saves[0].error);
  h.session.reset();
});

test("leave saves a new interrupted stroke which arrived while its first revision awaited ACK", async () => {
  const h = await harness(), ack = deferred<unknown>(); let dirty = false, writes = 0;
  h.ipc.on("studio_apply", () => ++writes === 1 ? ack.promise : h.view());
  h.session.beforeSave(() => { if (dirty) { h.session.saveFrame(object, frame, pixBytes(2)); dirty = false; } });
  h.session.saveFrame(object, frame, pixBytes(1)); const leaving = h.session.saveBeforeLeaving(); await settle(5);
  dirty = true; ack.resolve(h.view()); await leaving;
  assert.equal(writes, 2); assert.equal(dirty, false); assert.equal(h.session.saves.length, 0); h.session.reset();
});

test("snapshot rejects malformed retry identity and preserves full u128 scope, Unicode and migration", async () => {
  const h = await harness(); h.session.setTitle(object, "猫".repeat(5000));
  const json = h.session.draftSnapshot()!, decoded = decodeStudioDraft(json);
  assert.equal(decoded.scope.channel, CHANNEL); assert.equal(encodeStudioDraft(decoded), json);
  const current = sanitizeUiContinuity({ drafts: {}, readMarks: {}, studioDraft: json });
  assert.equal(planLegacyReadMarkMigration(current, '{"room":1}').state.studioDraft, json);
  const malformed = JSON.parse(json); malformed.records[0].request.nonce = "bad";
  assert.throws(() => decodeStudioDraft(JSON.stringify(malformed)));
  const unknown = JSON.parse(json); unknown.records[0].request.admin = true;
  assert.throws(() => decodeStudioDraft(JSON.stringify(unknown)));
  for (const pad of [8, 9]) validateDraftOperation({ op: "set_cell", step: 0, row: 0, value: { k: "d", pad } }, "score");
  h.session.reset();
});

test("sound and music use their effective draft, preserving raw evidence and permanent removals", async () => {
  const h = await harness(), vector = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
  h.session.setSoundPatch(object, vector.patchId, vector.patch);
  h.session.setSfx(object, { id: id32(5), frame, patch: vector.patchId, note: 60, durationMs: 500 });
  assert.equal(h.session.workingModel?.sound.sfx.length, 1); assert.equal(h.session.workingModel?.sound.ready, true);
  assert.equal(h.session.workingModel?.sound.records, h.session.doc?.model?.sound.records, "raw evidence stays native");
  h.session.removeSfx(object, id32(5)); h.session.setSfx(object, { id: id32(5), frame, patch: vector.patchId, note: 61, durationMs: 400 });
  assert.equal(h.session.workingModel?.sound.sfx.length, 0, "a pending tombstone cannot be resurrected");
  assert.deepEqual(decodeStudioDraft(h.session.draftSnapshot()!).records.map(r => r.kind), h.session.saves.map(r => r.kind));
  const base = scoreModel(parseScoreView({ ...h.view(), content: { kind: "score", title: null, bpm: null, bpb: null, steps: null,
    cells: {}, patches: {}, readiness: { cells: {}, patches: {}, headerConflicts: false } } }).content);
  const saves = [
    { kind: "apply", op: { op: "set_patch", patch: vector.patchId, descriptor: vector.patch } },
    { kind: "apply", op: { op: "set_cell", step: 0, row: 0, value: { k: "n", p: vector.patchId, len: 4 } } },
    { kind: "apply", op: { op: "set_cell", step: 2, row: 0, value: { k: "n", p: vector.patchId } } },
  ] as unknown as SaveRecord[];
  const working = workingScore(base, saves); assert.equal(working.ready, false); assert.equal(working.readiness.cells["0:0"].overlap, true);
  assert.equal(workingScore(base, saves.slice(0, 2)).ready, true); assert.deepEqual(base.cells, {});
  assert.equal(workingFlipnote(h.session.doc!.model!, h.session.saves, ME).sound.sfx.length, 0);
  h.session.reset();
});

test("a media preparation cannot cross a failed autosave while its frame fetch is pending", async () => {
  const h = await harness(), fetch = deferred<unknown>();
  h.ipc.on("request_blob_bounded", () => fetch.promise);
  h.ipc.on("studio_apply", () => { throw new Error("lost receipt"); });
  h.session.setFps(object, 6);
  const preparing = assert.rejects(h.session.prepareFlipnotePixa(object), /source changed/);
  await settle(3); await assert.rejects(h.session.save());
  fetch.resolve({ bytes: pixBytes().length, bytes_b64: b64(pixBytes()) });
  await preparing; h.session.reset();
});

test("failed Create preserves a long exact title, and Score pads8/9 round-trip without writing", async () => {
  const h = await harness();
  h.ipc.on("studio_create", () => { throw new Error("delivery lost"); });
  const title = "猫".repeat(9000); h.session.createFlipnote(title); await settle(5);
  const restored = decodeStudioDraft(h.session.draftSnapshot()!);
  const create = restored.records.find(r => r.kind === "create"); assert.ok(create?.kind === "create");
  assert.equal(create.request.title, title); assert.ok(create.label.length <= 256);
  h.session.reset();
  const score = { kind: "score", title: null, bpm: null, bpb: null, steps: null, cells: {}, patches: {}, readiness: { cells: {}, patches: {}, headerConflicts: false } };
  h.ipc.on("studio_read", () => ({ ...h.view(), content: score }));
  h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); h.session.open(id32(33), { objectKind: "score" }); await settle(8);
  h.session.setScoreCell(id32(33), 0, 0, { k: "d", pad: 8 }); h.session.setScoreCell(id32(33), 0, 1, { k: "d", pad: 9 });
  assert.equal(h.session.workingScore?.cells["0:1"].k, "d");
  assert.equal(decodeStudioDraft(h.session.draftSnapshot()!).records.length, 2); h.session.reset();
});

test("sidebar and Music share latest navigation ownership across a deferred Save", async () => {
  const h = await harness(), ack = deferred<unknown>(); h.ipc.on("studio_apply", () => ack.promise);
  h.session.setTitle(object, "draft"); const opened: string[] = [];
  const music = h.session.navigate(() => opened.push("music"));
  const sidebar = h.session.navigate(() => opened.push("sidebar"));
  ack.resolve(h.view()); await Promise.all([music, sidebar]); assert.deepEqual(opened, ["sidebar"]);
  // Cancellation after the inner Save has completed still wins at the actual mutation point.
  const late = h.session.navigate(() => opened.push("old")); h.session.cancelNavigation(); await late;
  assert.deepEqual(opened, ["sidebar"]); h.session.reset();
});

test("App scope navigation and editor navigation share the same claim in both orders", async () => {
  for (const appFirst of [true, false]) {
    const h = await harness(), ack = deferred<unknown>(); h.ipc.on("studio_apply", () => ack.promise);
    h.session.setTitle(object, "draft"); const opened: string[] = [];
    const app = () => { const current = h.session.claimNavigation(); return h.session.saveBeforeLeaving().then(() => { if (current()) opened.push("app"); }); };
    const editor = () => h.session.navigate(() => opened.push("editor"));
    const first = appFirst ? app() : editor(), second = appFirst ? editor() : app();
    ack.resolve(h.view()); await Promise.all([first, second]);
    assert.deepEqual(opened, [appFirst ? "editor" : "app"]); h.session.reset();
  }
});

test("window close stages then holds edits until native close succeeds or is cancelled", async () => {
  const h = await harness(); let dirty = true;
  h.session.beforeSave(() => { if (dirty) { h.session.saveFrame(object, frame, pixBytes(1)); dirty = false; } });
  h.session.beginClose(); assert.equal(dirty, false); assert.equal(h.session.canEdit(object), false);
  assert.throws(() => h.session.setFps(object, 5), /window is closing/);
  assert.throws(() => h.session.saveFrame(object, frame, pixBytes(2)), /window is closing/);
  await h.session.saveBeforeLeaving(); assert.equal(h.session.saves.length, 0);
  assert.throws(() => h.session.setScope({ server: SERVER + 1, channel: CHANNEL }), /window is closing/);
  assert.throws(() => h.session.open(id32(99)), /window is closing/);
  await assert.rejects(h.session.navigate(() => assert.fail("close must own navigation")), /window is closing/);
  assert.equal(h.session.canEdit(object), false);
  h.session.endClose(); assert.equal(h.session.canEdit(object), true);
  h.session.setFps(object, 5); assert.equal(h.session.saves.length, 1); h.session.reset();
});

test("admission reserves full local-only ticket bodies and later diagnostics before accepting edits", async () => {
  const h = await harness();
  h.ipc.on("studio_read", () => ({ ...previewView(flipnoteContent({ frames: [] })), group })); await h.session.refreshDoc(object);
  let refused = false;
  for (let i = 0; i < 150; i++) {
    try { h.session.setTitle(object, "a".repeat(60_000) + i); h.session.setFps(object, 1 + i % 24); }
    catch (e) { assert.match(String(e), /Unsaved edits are full/); refused = true; break; }
  }
  assert.equal(refused, true); assert.equal(h.writes().length, 0);
  const draft = decodeStudioDraft(h.session.draftSnapshot()!);
  for (const record of draft.records) if (record.kind === "overlayApply") {
    record.request = { basis: id64(8), branch: id64(9), nonce: id32(record.id), body: canonicalJson(record.op) };
    record.attempts = 1; record.status = "uncertain"; record.mayHaveCommitted = true; record.error = "\\".repeat(512);
  }
  assert.equal(decodeStudioDraft(encodeStudioDraft(draft)).records.length, h.session.saves.length, "freezing complete bodies cannot overflow the retained snapshot");
  h.session.reset();
});
