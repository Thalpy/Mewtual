import assert from "node:assert/strict";
import test from "node:test";
import { StudioSession, affectedObject } from "./studio-session.ts";
import { decodeStudioDraft, encodeStudioDraft, validateDraftOperation } from "./studio-draft.ts";
import { decodePixa } from "./pixa.ts";
import { sanitizeUiContinuity, planLegacyReadMarkMigration } from "./ui-continuity.ts";
import { readFileSync } from "node:fs";
import { workingFlipnote, workingScore } from "./studio-working.ts";
import { scoreModel, parseScoreView } from "./studio-native.ts";
import type { SaveRecord } from "./studio-session.ts";
import { canonicalJson } from "./studio-contract.ts";
import { CHANNEL, ME, SERVER, b64, deferred, fakeIpc, flipnoteContent, id32, id64, indexContent, indexEntry, ordinaryView, pixBytes, previewView, settle } from "./studio-testkit.ts";
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

// Review of d2548a37, P2 2: preparation captured the working model, then looked each frame's unsaved
// pixels up in the live queue across its awaits. An autosave receipt for a later frame, landing
// while an earlier frame's read was pending, removed that record, so the captured `draft:`
// placeholder was fetched as a CID and Play, download, Post and GIF export failed.
test("an autosave landing while a frame read is pending keeps the captured draft frame's pixels", async () => {
  const h = await harness(), read = deferred<unknown>();
  h.ipc.on("request_blob_bounded", () => read.promise);
  const added = h.session.insertFrame(object, frame, pixBytes(5));
  const preparing = h.session.prepareFlipnotePixa(object); await settle(3);
  await h.tick(5000); // the deadline saves the new frame while the first frame's read is pending
  assert.equal(h.session.saves.length, 0, "the autosave landed");
  assert.equal(h.session.unsavedPix(object, added), null, "and its record left the queue");
  read.resolve({ bytes: pixBytes().length, bytes_b64: b64(pixBytes()) });
  const animation = decodePixa(await preparing);
  assert.equal(animation.frames.length, 2);
  assert.deepEqual([...animation.frames[1]], [...pixBytes(5)], "the new frame's own pixels");
  assert.ok(!h.ipc.calls.some(c => JSON.stringify(c.args).includes("draft:")), "no placeholder reached a native lookup");
  h.session.reset();
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

// Review of 149e7321, finding 1. A departure Save covers the document being left; held work for
// another document is reached by opening it, so it can never veto that open.
test("navigation saves only the document it leaves; another document's held work never vetoes reaching it", async () => {
  const h = await harness(), other = id32(9);
  h.session.setFps(object, 7); h.session.open(other); await settle(8); h.session.setFps(other, 8);
  const draft = decodeStudioDraft(h.session.draftSnapshot()!);
  // An interrupted delivery restores as uncertain: only the member may retry or discard it.
  const held = draft.records.find(r => r.object === other)!;
  held.status = "inflight"; held.attempts = 1;
  h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  const applied = () => h.ipc.calls.filter(c => c.cmd === "studio_apply").map(c => c.args.object);
  assert.equal(h.session.doc, null);
  await h.session.navigate(() => h.session.open(object)); await settle(8);
  assert.equal(h.session.doc?.object, object, "nothing open: nothing is left, so nothing is saved first");
  assert.deepEqual(applied(), [], "opening never replays restored work");
  await h.session.navigate(() => h.session.open(other)); await settle(8);
  assert.equal(h.session.doc?.object, other, "the uncertain edit of the target cannot veto opening it");
  assert.deepEqual(applied(), [object], "leaving the first document saved exactly its own held edit");
  assert.deepEqual(h.session.saves.map(s => [s.object, s.status]), [[other, "uncertain"]]);
  await assert.rejects(h.session.saveBeforeLeaving(), /Could not confirm the save/, "leaving the Studio still requires every edit");
  h.session.reset();
});

test("a departure Save leaves the shared deadline running for an Index edit about another document", async () => {
  const h = await harness(), other = id32(9), third = id32(10);
  const listed = () => ({ ...ordinaryView(indexContent({ objects: { [object]: indexEntry({ title: "initial" }), [third]: indexEntry({ title: "third" }) } })), group });
  h.ipc.on("studio_list", listed); h.ipc.on("studio_apply_index", listed);
  await h.session.refreshIndex(); await settle(8);
  h.session.setTitle(object, "renamed"); h.session.setEntryExpiry(third, { kind: "never" });
  assert.deepEqual(h.session.saves.map(s => s.kind).sort(), ["apply", "applyIndex", "applyIndex"]);
  await h.session.navigate(() => h.session.open(other)); await settle(8);
  assert.deepEqual(h.session.saves.map(s => s.object), [third], "both halves of the rename went with the departure");
  await h.tick(5000);
  assert.equal(h.session.saves.length, 0, "the other document's Index edit still saved at its deadline");
  h.session.reset();
});

// Adversarial review of 22e4ed77's fixes, M3: leaving a restored document sent only its document
// half; its Index half (in the shared Index lane) stayed held.
test("leaving a restored, renamed document sends both halves on the ordinary, Closing and Unconfirmed routes", async () => {
  for (const route of ["ordinary", "closing", "unconfirmed"] as const) {
    const h = await harness(), other = id32(9);
    const listed = indexContent({ objects: { [object]: indexEntry({ title: "initial" }), [other]: indexEntry({ title: "other" }) } });
    const view = (content: unknown) => route === "unconfirmed" ? { ...previewView(content), group } : { ...ordinaryView(content, { phase: route === "closing" ? "closing" : "open" }), group };
    const doc = () => view(flipnoteContent({ frames: [] }));
    h.ipc.on("studio_list", () => view(listed)); h.ipc.on("studio_read", doc); h.ipc.on("studio_apply", doc); h.ipc.on("studio_apply_index", () => view(listed));
    for (const kind of ["unconfirmed", "closing"] as const) {
      h.ipc.on(`studio_${kind}_overlay_save_begin`, a => ({ v: 1, kind: `${kind}OverlaySaveTicket`, channel: a.channel, object: a.object ?? null, basis: id64(1), branch: id64(2), provenance: kind }));
      h.ipc.on(`studio_${kind}_overlay_save`, a => ({ v: 1, kind: `${kind}OverlaySave`, channel: a.channel, object: a.object ?? null, state: "saved", basis: id64(1), accepted: 1, localOnly: true, provisional: true, provenance: kind }));
    }
    await h.session.refreshIndex(); await h.session.refreshDoc(object); await settle(8);
    h.session.setTitle(object, "renamed");
    const draft = decodeStudioDraft(h.session.draftSnapshot()!);
    h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
    h.session.open(object); await settle(8);
    await h.session.navigate(() => h.session.open(other)); await settle(8);
    const bodyOf = (args: Record<string, unknown>) => String(args.body ?? (args.save as { body?: unknown } | undefined)?.body ?? "");
    assert.equal(h.ipc.calls.filter(c => /"title"/.test(bodyOf(c.args)) && /renamed/.test(bodyOf(c.args))).length, 2, `${route}: both halves sent`);
    assert.equal(h.session.saves.length, 0, `${route}: nothing held behind`);
    h.session.reset();
  }
});

// Review of 5bc43d2c: departure released the Index half before the document's own Save, and the one
// save worker then waited on the Index receipt, so a slow Index response held the navigation.
test("with the worker free, leaving a document saves it, navigates, and only then sends its Index half", async () => {
  const h = await harness(), other = id32(9), third = id32(10), indexAck = deferred<unknown>();
  const listed = () => ({ ...ordinaryView(indexContent({ objects: { [object]: indexEntry({ title: "initial" }), [other]: indexEntry({ title: "other" }), [third]: indexEntry({ title: "third" }) } })), group });
  h.ipc.on("studio_list", listed);
  h.ipc.on("studio_apply_index", () => indexAck.promise); // the slow Index receipt
  await h.session.refreshIndex(); await settle(8);
  h.session.setTitle(object, "renamed");
  h.session.open(third); await settle(8); h.session.setFps(third, 5); // another document's restored work
  const draft = decodeStudioDraft(h.session.draftSnapshot()!);
  const indexHalf = draft.records.find(r => r.kind === "applyIndex")!;
  h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  h.session.open(object); await settle(8);
  // No clock tick: navigation must not depend on this Index receipt, nor wait for the 10 s timer.
  const outcome = await Promise.race([h.session.navigate(() => h.session.open(other)).then(() => "navigated", e => String(e)), settle(10).then(() => "still leaving")]);
  assert.equal(outcome, "navigated"); assert.equal(h.session.doc?.object, other);
  const writes = h.ipc.calls.filter(c => c.cmd === "studio_apply" || c.cmd === "studio_apply_index").map(c => c.cmd);
  assert.deepEqual(writes, ["studio_apply", "studio_apply_index"], "the document's Save went first and landed");
  const pending = h.session.saves.find(s => s.kind === "applyIndex")!;
  assert.equal(pending.status, "inflight", "the Index half was sent, and is retained until its receipt");
  assert.deepEqual(pending.kind === "applyIndex" && pending.request, indexHalf.kind === "applyIndex" && indexHalf.request, "with its original request identity");
  const sent = h.ipc.calls.find(c => c.cmd === "studio_apply_index")!.args;
  assert.equal(indexHalf.kind === "applyIndex" && sent.nonce, indexHalf.kind === "applyIndex" && indexHalf.request.nonce);
  const held = h.session.saves.find(s => s.object === third)!;
  assert.equal(h.session.awaitsExplicitSave(held.id), true, "the other document's restored work stays held");
  indexAck.resolve(listed()); await settle(8);
  assert.deepEqual(h.session.saves.map(s => s.object), [third]);
  h.session.reset();
});

// Several save workers, one per lane at a time: an Index delivery in flight (from the deadline, an
// earlier departure, or a sidebar retry) no longer holds a document's departure behind it. This
// was the pinned single-worker limit, and the timing variant of the review of 64fed24b.
test("a departure never waits on an Index delivery already in flight", async () => {
  for (const edit of ["before leaving", "while leaving"] as const) {
    const h = await harness(), other = id32(9), titleAck = deferred<unknown>(), indexAck = deferred<unknown>();
    const listed = () => ({ ...ordinaryView(indexContent({ objects: { [object]: indexEntry({ title: "initial" }) } })), group });
    h.ipc.on("studio_list", listed); h.ipc.on("studio_apply_index", () => indexAck.promise);
    let applies = 0; h.ipc.on("studio_apply", () => ++applies === 1 ? titleAck.promise : h.view());
    await h.session.refreshIndex(); await settle(8);
    h.session.setTitle(object, "renamed"); await h.tick(5000); // the deadline releases both halves
    assert.equal(h.session.saves.find(s => s.kind === "applyIndex")?.status, "inflight", `${edit}: the Index half runs on its own worker`);
    if (edit === "before leaving") h.session.setFps(object, 7);
    const leaving = h.session.navigate(() => h.session.open(other)).then(() => "navigated", e => String(e));
    // Review of 64fed24b: an edit made while the departure waits on the title receipt joins the
    // next round of the drain, and must not wait on the Index receipt either.
    if (edit === "while leaving") { await settle(3); h.session.setFps(object, 7); }
    titleAck.resolve(h.view());
    const outcome = await Promise.race([leaving, settle(12).then(() => "still leaving")]);
    assert.equal(outcome, "navigated", `${edit}: B opens while the Index receipt is still pending`);
    assert.equal(h.session.doc?.object, other);
    assert.equal(applies, 2, `${edit}: the title and the fps Save both landed`);
    assert.deepEqual(h.session.saves.map(s => [s.kind, s.status]), [["applyIndex", "inflight"]], `${edit}: only the Index half is still out`);
    indexAck.resolve(listed()); await settle(8); assert.equal(h.session.saves.length, 0);
    h.session.reset();
  }
});

// Lane order holds with several workers: a lane never has two saves in flight, and its records
// run in queue order, so a document's later edits wait for its earlier ones.
test("several workers never run two saves of one lane at once", async () => {
  const h = await harness(), acks: ReturnType<typeof deferred<unknown>>[] = [];
  h.ipc.on("studio_apply", () => { const ack = deferred<unknown>(); acks.push(ack); return ack.promise; });
  h.session.setTitle(object, "one"); h.session.setFps(object, 7); h.session.setTitle(object, "two");
  void h.session.save().catch(() => {}); await settle(8);
  assert.equal(acks.length, 1, "one document lane, one save in flight");
  acks[0].resolve(h.view()); await settle(8);
  assert.equal(acks.length, 2, "the next record of the lane went only after the first answered");
  const bodies = h.ipc.calls.filter(c => c.cmd === "studio_apply").map(c => JSON.parse(String(c.args.body)).field);
  assert.deepEqual(bodies, ["title", "fps"]);
  for (const ack of acks.slice(1)) ack.resolve(h.view());
  await settle(8); acks.at(-1)?.resolve(h.view()); await settle(8);
  h.session.reset();
});

// Review of the 5bc43d2c fix, L1 and L2: a refused departure sends nothing of the Index half, which
// stays recoverable; a superseded one that did drain still sends it, so a drained departure does
// not split the rename.
test("a refused departure holds the restored Index half; a superseded one that drained sends it", async () => {
  for (const ending of ["refused", "superseded"] as const) {
    const h = await harness(), other = id32(9), docAck = deferred<unknown>();
    const listed = () => ({ ...ordinaryView(indexContent({ objects: { [object]: indexEntry({ title: "initial" }) } })), group });
    h.ipc.on("studio_list", listed); h.ipc.on("studio_apply_index", listed);
    await h.session.refreshIndex(); await settle(8);
    h.session.setTitle(object, "renamed");
    const draft = decodeStudioDraft(h.session.draftSnapshot()!);
    h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
    h.session.open(object); await settle(8);
    if (ending === "refused") h.ipc.on("studio_apply", () => { throw new Error("receipt lost"); });
    else h.ipc.on("studio_apply", () => docAck.promise);
    const leaving = h.session.navigate(() => h.session.open(other)).then(() => "navigated", e => String(e));
    const indexWrites = () => h.ipc.calls.filter(c => c.cmd === "studio_apply_index").length;
    if (ending === "refused") {
      assert.match(await leaving, /Could not confirm the save/);
      const half = h.session.saves.find(s => s.kind === "applyIndex")!;
      assert.equal(indexWrites(), 0); assert.equal(h.session.awaitsExplicitSave(half.id), true, "held, not sent");
      assert.equal(h.session.pendingFor(object).length, 2, "both halves still show in the open document");
      h.ipc.on("studio_apply", () => h.view()); h.session.retry(h.session.saves[0].id); await settle(8);
      await h.session.saveDocument(object); await settle(8);
      assert.equal(indexWrites(), 1); assert.equal(h.session.saves.length, 0);
    } else {
      await settle(3); h.session.claimNavigation(); // a newer action that does not leave this document
      docAck.resolve(h.view()); await leaving; await settle(8);
      assert.equal(h.session.doc?.object, object, "the superseded navigation did not happen");
      assert.equal(indexWrites(), 1, "the drained document's Index half was still sent");
    }
    h.session.reset();
  }
});

// Re-review: overlapping Saves that finish out of order must not leave a stale preference behind.
test("overlapping Saves finishing out of order leave no stale preference", async () => {
  const h = await harness(), other = id32(9), ack = deferred<unknown>();
  h.ipc.on("studio_apply", () => ack.promise);
  h.session.setFps(object, 7);
  const leaving = h.session.navigate(() => h.session.open(other)).catch(() => {});
  const saving = h.session.saveDocument(object).catch(() => {});
  await settle(3); ack.resolve(h.view()); await Promise.all([leaving, saving]); await settle(8);
  assert.deepEqual((h.session as unknown as { preferring: unknown[] }).preferring, [], "every finished Save removed its own preference");
  h.session.reset();
});

test("affectedObject names the document a record changes, whatever its native target", () => {
  const base = { id: 1, status: "queued", attempts: 0, error: "", epochChanged: false, mayHaveCommitted: false, label: "x" };
  const record = (r: object) => ({ ...base, ...r }) as unknown as SaveRecord;
  assert.equal(affectedObject(record({ kind: "apply", object })), object, "a document record");
  assert.equal(affectedObject(record({ kind: "applyIndex", object, op: { op: "set_title", object, title: "t" } })), object, "an ordinary Index record");
  for (const op of [{ op: "set_title", object, title: "t" }, { op: "tombstone_object", object }, { op: "set_expiry", object, expiry: 0 }]) {
    assert.equal(affectedObject(record({ kind: "overlayApply", object: null, op })), object, `a local-draft Index ${op.op}`);
  }
  assert.equal(affectedObject(record({ kind: "overlayApply", object: null, op: {} })), null, "an Index record about no one entry");
  for (const kind of ["object", "objectTitle", "objectExpiry", "objectDeletion"]) {
    assert.equal(affectedObject(record({ kind: "recoveryApply", object: null, edit: { choice: { kind, id: object, value: "00" } } })), object, `an Index recovery of one entry (${kind})`);
  }
  assert.equal(affectedObject(record({ kind: "recoveryApply", object: null, edit: { choice: { kind: "title", value: "00" } } })), null, "an Index recovery naming no entry");
});

// Adversarial review of 22e4ed77's fixes, M1: B's Save waited ten seconds and then said "try
// again" when B's Index half sat in the shared Index lane behind A's restored, held Index record.
test("a document's Save behind another document's held Index edit refuses at once and names it", async () => {
  const h = await harness(), other = id32(9);
  const listed = () => ({ ...ordinaryView(indexContent({ objects: { [object]: indexEntry({ title: "first" }), [other]: indexEntry({ title: "second" }) } })), group });
  h.ipc.on("studio_list", listed); h.ipc.on("studio_apply_index", listed);
  await h.session.refreshIndex(); await settle(8);
  h.session.setTitle(object, "first renamed");
  h.session.open(other); await settle(8); h.session.setTitle(other, "second renamed");
  const draft = decodeStudioDraft(h.session.draftSnapshot()!);
  h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  h.session.open(other); await settle(8);
  // No clock tick: the refusal must not wait for the ten-second timer.
  const outcome = await Promise.race([h.session.saveDocument(other).then(() => "saved", e => String(e)), settle(10).then(() => "still waiting")]);
  assert.match(outcome, /waits behind an unsaved edit \(index title of "first"\)/);
  await settle(8);
  const left = h.session.saves.map(s => [s.object, s.kind, h.session.awaitsExplicitSave(s.id)]);
  assert.deepEqual(left, [[object, "apply", true], [object, "applyIndex", true], [other, "applyIndex", false]],
    "B's document half landed; its Index half waits, released, behind A's two held records");
  h.session.reset();
});

test("a restored edit for a document this session never read is admitted against a fresh read", async () => {
  const h = await harness(); h.session.setFps(object, 7);
  const draft = decodeStudioDraft(h.session.draftSnapshot()!);
  h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  const reads = h.ipc.calls.filter(c => c.cmd === "studio_read").length;
  await h.session.save();
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_read").length, reads + 1, "admission read the document first");
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 1); assert.equal(h.session.saves.length, 0);
  h.session.reset();
  // An unreadable document is refused as not written, never as a possible commit or a history change.
  h.ipc.on("studio_read", () => { throw new Error("offline"); });
  h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  await assert.rejects(h.session.save());
  const [refused] = h.session.saves;
  assert.equal(refused.status, "blocked"); assert.equal(refused.mayHaveCommitted, false);
  assert.match(refused.error, /could not read this document/); assert.doesNotMatch(refused.error, /history changed/);
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 1, "nothing further was sent");
  h.session.reset();
});

// Adversarial re-review: the admission read must not make an unsent edit look like a delivery.
test("a lock during the admission read restores the edit as unsent, and an unread refusal re-admits on retry", async () => {
  const h = await harness(); h.session.setFps(object, 7);
  const restore = (draft: ReturnType<typeof decodeStudioDraft>) => {
    h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush();
  };
  restore(decodeStudioDraft(h.session.draftSnapshot()!)); await settle(8);
  const read = deferred<unknown>(); h.ipc.on("studio_read", () => read.promise);
  void h.session.save().catch(() => {}); await settle(3);
  assert.deepEqual([h.session.saves[0].status, h.session.saves[0].attempts], ["queued", 0], "reading is not delivering");
  restore(decodeStudioDraft(h.session.draftSnapshot()!)); await settle(8); // lock lands mid-read
  const [restored] = h.session.saves;
  assert.equal(restored.status, "queued"); assert.equal(restored.mayHaveCommitted, false);
  // Unreadable: refused as not written and not counted as an attempt; reauthor stays available.
  h.ipc.on("studio_read", () => { throw new Error("offline"); });
  await assert.rejects(h.session.save());
  assert.deepEqual([h.session.saves[0].status, h.session.saves[0].attempts], ["blocked", 0]);
  assert.equal(h.session.canReauthor(h.session.saves[0].id), true);
  // The document is rotating when it can be read again: the exact retry must read and compare
  // again, and refuse, rather than skip admission as a repeat delivery.
  h.ipc.on("studio_read", () => ({ ...h.view(), phase: "closing" }));
  h.session.retry(h.session.saves[0].id); await settle(8);
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 0, "nothing was sent into a changed history");
  assert.match(h.session.saves[0].error, /Shared history changed/);
  h.session.reset();
});

test("before the Index is read, a restored edit is refused as unread without counting, and its retry is admitted afresh", async () => {
  const h = await harness(); h.session.setFps(object, 7);
  const draft = decodeStudioDraft(h.session.draftSnapshot()!);
  const listing = deferred<unknown>(); h.ipc.on("studio_list", () => listing.promise);
  h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  await assert.rejects(h.session.save());
  const [refused] = h.session.saves;
  assert.deepEqual([refused.status, refused.attempts, refused.mayHaveCommitted], ["blocked", 0, false]);
  assert.match(refused.error, /has not read this channel's index/); assert.doesNotMatch(refused.error, /server .* changed/);
  listing.resolve({ ...ordinaryView(indexContent()), group }); await settle(8);
  h.ipc.on("studio_read", () => ({ ...h.view(), phase: "closing" })); await h.session.refreshDoc(object);
  h.session.retry(refused.id); await settle(8);
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 0, "the retry compared the binding and refused");
  assert.match(h.session.saves[0].error, /Shared history changed/);
  h.session.reset();
});

// Review of 13792739, P2: a failed pixel publication was classified as a possibly committed
// document edit, so after the epoch moved the frame could be neither retried nor re-authored.
test("a failed pixel publication is not a document write: the frame stays re-authorable after the epoch moves", async () => {
  const h = await harness();
  let publishFails = true;
  h.ipc.on("publish_pix", a => {
    if (publishFails) throw new Error("blob store unavailable");
    return { cid: id64(4), bytes: atob(String(a.bytesB64)).length };
  });
  h.session.saveFrame(object, frame, pixBytes(3));
  await h.tick(5000);
  const [failed] = h.session.saves;
  assert.deepEqual([failed.status, failed.attempts, failed.mayHaveCommitted], ["blocked", 0, false]);
  assert.match(failed.error, /blob store unavailable; nothing of this edit was sent/);
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 0);
  await h.rotate(); publishFails = false;
  h.session.retry(failed.id); await settle(8);
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 0, "the old binding is refused before sending");
  const [refused] = h.session.saves;
  assert.deepEqual([refused.status, refused.mayHaveCommitted], ["blocked", false]);
  assert.equal(h.session.canReauthor(refused.id), true, "the retained pixels can still be saved into the new epoch");
  h.session.reauthor(refused.id); await settle(8);
  const applied = h.ipc.calls.filter(c => c.cmd === "studio_apply");
  assert.equal(applied.length, 1); assert.equal(applied[0].args.epochId, id32(0xe2));
  assert.equal(h.session.saves.length, 0);
  h.session.reset();
});

test("a publication failure never clears an earlier uncertain delivery of the same frame", async () => {
  const h = await harness();
  // A restored frame whose earlier delivery is uncertain but which holds no publication receipt
  // or Apply identity, so the retry publishes again before anything is sent.
  h.session.saveFrame(object, frame, pixBytes(3));
  const json = h.session.draftSnapshot()!, draft = JSON.parse(json);
  Object.assign(draft.records[0], { status: "uncertain", mayHaveCommitted: true, attempts: 1, error: "commit response was lost" });
  h.session.reset(); h.session.restoreDraft(decodeStudioDraft(JSON.stringify(draft)));
  h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); h.session.open(object); await settle(8);
  let publishes = 0;
  h.ipc.on("publish_pix", () => { publishes++; throw new Error("blob store unavailable"); });
  h.session.retry(h.session.saves[0].id); await settle(8);
  const [still] = h.session.saves;
  assert.equal(publishes, 1, "the retry reached the publication step");
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 0);
  assert.deepEqual([still.status, still.mayHaveCommitted], ["uncertain", true], "the earlier delivery's uncertainty stands");
  assert.ok(still.attempts >= 1, "a restored draft must keep attempts >= 1 while mayHaveCommitted");
  assert.match(still.error, /earlier attempt may already have committed/);
  h.session.reset();
});

// Adversarial review of the follow-up, MEDIUM 1: leaving the surface seals the queue at once, so a
// frame still publishing its pixels was sealed `inflight` and restored as "may have committed",
// stranding it again after the epoch moved (the live path fixed above, but through restore).
test("a frame sealed while its pixels were still publishing restores as unsent and re-authorable", async () => {
  const h = await harness(), held = deferred<unknown>();
  h.ipc.on("publish_pix", () => held.promise);
  h.session.saveFrame(object, frame, pixBytes(3));
  await h.session.preserveForDeparture(0); await settle(8);
  assert.equal(h.session.saves[0].status, "inflight", "the departure released it and its publication is out");
  const sealed = decodeStudioDraft(h.session.draftSnapshot()!);
  h.session.reset(); h.session.restoreDraft(sealed);
  h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); h.session.open(object); await settle(8);
  const [restored] = h.session.saves;
  assert.deepEqual([restored.status, restored.mayHaveCommitted, restored.attempts], ["queued", false, 0]);
  h.ipc.on("publish_pix", a => ({ cid: id64(4), bytes: atob(String(a.bytesB64)).length }));
  await h.rotate();
  await assert.rejects(h.session.save());
  const [refused] = h.session.saves;
  assert.deepEqual([refused.status, refused.mayHaveCommitted], ["blocked", false]);
  assert.equal(h.session.canReauthor(refused.id), true);
  h.session.reauthor(refused.id); await settle(8);
  const applied = h.ipc.calls.filter(c => c.cmd === "studio_apply");
  assert.equal(applied.length, 1); assert.equal(applied[0].args.epochId, id32(0xe2));
  h.session.reset();
});

test("a frame sealed after its Apply identity was sent still restores as possibly committed", async () => {
  const h = await harness(), ack = deferred<unknown>();
  h.ipc.on("studio_apply", () => ack.promise);
  h.session.saveFrame(object, frame, pixBytes(3));
  await h.session.preserveForDeparture(0); await settle(8);
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 1, "the Apply is out");
  const sealed = decodeStudioDraft(h.session.draftSnapshot()!);
  h.session.reset(); h.session.restoreDraft(sealed);
  h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  const [restored] = h.session.saves;
  assert.deepEqual([restored.status, restored.mayHaveCommitted], ["uncertain", true]);
  assert.equal(h.session.canReauthor(restored.id), false, "only the exact retry or a discard may resolve it");
  h.session.reset();
});

test("leaving the surface sends queued edits but never waits for their receipt; background progress is not new authoring", async () => {
  const h = await harness(), ack = deferred<unknown>();
  h.ipc.on("studio_apply", () => ack.promise);
  let work = 0; h.session.onWorkChange(() => work++);
  const author = h.session.authorRevision;
  h.session.setFps(object, 7);
  assert.ok(h.session.authorRevision > author, "new authoring moves the author revision");
  assert.ok(work > 0, "a change to retained work is signalled for a checkpoint");
  const held = await h.session.preserveForDeparture(0);
  assert.equal(held, 1, "the record is still held: the departure did not wait for its receipt");
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 1, "leaving still sends what was queued");
  const sealed = h.session.authorRevision, signalled = work;
  ack.resolve(h.view()); await settle(8);
  assert.equal(h.session.saves.length, 0, "the Save lands in the background after the member left");
  assert.equal(h.session.authorRevision, sealed, "a landing receipt is not unpreserved work");
  assert.ok(work > signalled, "the emptier queue is signalled so the next checkpoint drops it");
  h.session.reset();
});

test("leaving the surface keeps a failed Save, with its exact request, instead of refusing to leave", async () => {
  const h = await harness();
  h.ipc.on("studio_apply", () => { throw new Error("outcome=uncertain; commit response was lost"); });
  h.session.setFps(object, 7);
  await assert.rejects(h.session.save(), /Could not confirm the save/);
  const [failed] = h.session.saves;
  assert.equal(failed.status, "uncertain");
  assert.equal(await h.session.preserveForDeparture(0), 1, "the uncertain record is preserved, not a reason to stay");
  assert.deepEqual(decodeStudioDraft(h.session.draftSnapshot()!).records.map(r => [r.id, r.status, r.mayHaveCommitted]), [[failed.id, "uncertain", true]]);
  h.session.reset();
});

// Performance/save review (2026-10-30), finding 3: an admission refusal raised before anything was
// sent still counted as a delivery attempt, so the member's Retry skipped first-delivery admission
// and sent the frozen request to the retired epoch; native's untyped rejection then read as
// "may have committed" and disabled "save again as a new edit".
test("a retry after a pre-dispatch admission refusal compares again instead of sending the frozen request", async () => {
  const h = await harness();
  h.session.setFps(object, 7); // authored against the first epoch
  await h.rotate(); // a newer epoch arrives before the deadline
  await h.tick(5000);
  const [refused] = h.session.saves;
  assert.deepEqual([refused.status, refused.attempts, refused.mayHaveCommitted], ["blocked", 0, false], "a refusal before sending is not a delivery");
  assert.match(refused.error, /Shared history changed/);
  h.session.retry(refused.id); await settle(8);
  assert.equal(h.ipc.calls.filter(c => c.cmd === "studio_apply").length, 0, "the frozen request never left");
  const [again] = h.session.saves;
  assert.deepEqual([again.status, again.attempts, again.mayHaveCommitted], ["blocked", 0, false]);
  assert.equal(h.session.canReauthor(again.id), true, "saving it again as a new edit stays available");
  h.session.reset();
});

// Re-review of 22e4ed77, finding 2: a document's Save covered `object === id`, which a Closing or
// Unconfirmed Index record (native target null, entry in its op) never matches. A restored rename
// then saved only its document half, and the held Index half vanished from the editor.
test("a document's Save sends both halves of a rename on the ordinary, Closing and Unconfirmed routes", async () => {
  for (const route of ["ordinary", "closing", "unconfirmed"] as const) {
    const h = await harness(), other = id32(9);
    const listed = indexContent({ objects: { [object]: indexEntry({ title: "initial" }), [other]: indexEntry({ title: "other" }) } });
    const view = (content: unknown) => route === "unconfirmed" ? { ...previewView(content), group } : { ...ordinaryView(content, { phase: route === "closing" ? "closing" : "open" }), group };
    const doc = () => view(flipnoteContent({ frames: [] }));
    h.ipc.on("studio_list", () => view(listed)); h.ipc.on("studio_read", doc);
    h.ipc.on("studio_apply", doc); h.ipc.on("studio_apply_index", () => view(listed));
    for (const kind of ["unconfirmed", "closing"] as const) {
      h.ipc.on(`studio_${kind}_overlay_save_begin`, a => ({ v: 1, kind: `${kind}OverlaySaveTicket`, channel: a.channel, object: a.object ?? null, basis: id64(1), branch: id64(2), provenance: kind }));
      h.ipc.on(`studio_${kind}_overlay_save`, a => ({ v: 1, kind: `${kind}OverlaySave`, channel: a.channel, object: a.object ?? null, state: "saved", basis: id64(1), accepted: 1, localOnly: true, provisional: true, provenance: kind }));
    }
    await h.session.refreshIndex(); await h.session.refreshDoc(object); await settle(8);
    h.session.setTitle(object, "renamed");
    h.session.open(other); await settle(8); h.session.setFps(other, 5);
    const draft = decodeStudioDraft(h.session.draftSnapshot()!);
    h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
    h.session.open(object); await settle(8);
    const mine = h.session.pendingFor(object);
    assert.equal(mine.length, 2, `${route}: the editor counts both halves of the rename`);
    if (route !== "ordinary") assert.ok(mine.some(s => s.kind === "overlayApply" && s.object === null), `${route}: the Index half keeps the Index as its native target`);
    await h.session.saveDocument(object); await settle(8);
    // Ordinary Apply carries `body`; a local-draft Save nests its request under `save`.
    const bodyOf = (args: Record<string, unknown>) => String(args.body ?? (args.save as { body?: unknown } | undefined)?.body ?? "");
    const titles = h.ipc.calls.filter(c => /"title"/.test(bodyOf(c.args)) && /renamed/.test(bodyOf(c.args)));
    assert.equal(titles.length, 2, `${route}: the document title and the Index title were both sent (${titles.map(c => c.cmd).join(", ")})`);
    if (route !== "ordinary") assert.ok(titles.some(c => c.args.object === undefined || c.args.object === null), `${route}: the Index half still went to the Index`);
    assert.deepEqual(h.session.pendingFor(object), [], `${route}: nothing of this document is left behind`);
    assert.equal(h.session.saves.length, 1); assert.equal(h.session.awaitsExplicitSave(h.session.saves[0].id), true, `${route}: the other document stays held`);
    h.session.reset();
  }
});

test("an admission read overtaken by a newer read of the document says it was still reading", async () => {
  const h = await harness(); h.session.setFps(object, 7);
  const draft = decodeStudioDraft(h.session.draftSnapshot()!);
  h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  const reads: ReturnType<typeof deferred<unknown>>[] = [];
  h.ipc.on("studio_read", () => { const next = deferred<unknown>(); reads.push(next); return next.promise; });
  const saving = assert.rejects(h.session.save()); await settle(3);
  h.session.open(object); await settle(3); // the editor's own read supersedes the admission read
  reads[0].resolve(h.view()); await saving;
  assert.match(h.session.saves[0].error, /still reading this document/); assert.equal(h.session.saves[0].attempts, 0);
  reads[1].resolve(h.view()); await settle(8);
  h.session.retry(h.session.saves[0].id); await settle(8);
  assert.equal(h.session.saves.length, 0, "the retry admitted against the view that landed");
  h.session.reset();
});
