import assert from "node:assert/strict";
import test from "node:test";
import { CHANNEL, SERVER, fakeIpc, flipnoteContent, id32, id64, isource, ordinaryView, previewView, reg } from "./studio-testkit.ts";
import { overlayLifecycle, parseScoreView, parseStudioView, parseSettlementChanged, parseUnconfirmedOverlaySaveTicket, recoveryRead, scoreModel, studioApply, studioCreate, studioRead } from "./studio-native.ts";
import { legacyJamPatch } from "./jam-patch.ts";

function content() {
  return { kind: "score", title: null, bpm: null, bpb: null, steps: null,
    cells: {} as Record<string, unknown>, patches: {} as Record<string, unknown>,
    readiness: { cells: {} as Record<string, unknown>, patches: {} as Record<string, unknown>, headerConflicts: false } };
}
const cellIssues = { count: false, extent: false, recipe: false, overlap: false, conflict: false };
const patchIssues = { count: false, conflict: false };

test("native score defaults invent no source and preserve lossless ordinary/preview trust", () => {
  for (const raw of [ordinaryView(content(), { epoch: "18446744073709551615" }), previewView(content())]) {
    const view = parseScoreView(raw), model = scoreModel(view.content);
    assert.equal(view.epoch, raw.epoch);
    assert.equal(view.content.kind, "score");
    assert.deepEqual([model.title, model.bpm, model.bpb, model.steps], ["", 120, 4, 16]);
    assert.equal(model.headers.bpm, null);
    assert.equal(model.ready, true);
    assert.equal(parseStudioView(raw).content.kind, "score");
  }
});

test("native score keeps raw setter winners while concurrent clear/remove remains effective", () => {
  const raw = content(), patch = legacyJamPatch("sine"), id = id64(99);
  raw.cells["0:0"] = { removed: true, register: reg({ k: "n", p: id, len: 4 }, isource(1), [{ value: null, source: isource(2) }]) };
  raw.patches[id] = { removed: true, register: reg(patch, isource(3), [{ value: null, source: isource(4) }]) };
  const model = scoreModel(parseScoreView(ordinaryView(raw)).content);
  assert.deepEqual(model.cells, {}); assert.deepEqual(model.patches, {});
  assert.equal(model.cellSlots["0:0"].register.selected.value?.k, "n");
  assert.equal(model.cellSlots["0:0"].register.conflicts[0].value, null);
  assert.equal(model.patchSlots[id].register.selected.source.opId, id64(3));
  assert.equal(model.patchSlots[id].register.conflicts.length, 1);
  raw.cells["0:0"] = { ...(raw.cells["0:0"] as object), removed: false };
  assert.throws(() => parseScoreView(ordinaryView(raw)), /removal evidence/);
});

test("active score values preserve authored length and readiness rather than silently resolving conflict/overflow", () => {
  const raw = content(), id = id64(99);
  raw.cells["255:24"] = { removed: false, register: reg({ k: "n", p: id, len: 1 }, isource(1), [{ value: { k: "d", pad: 9 }, source: isource(2) }]) };
  raw.patches[id] = { removed: false, register: reg(legacyJamPatch("sine"), isource(3)) };
  raw.readiness.cells["255:24"] = { ...cellIssues, extent: true, conflict: true };
  raw.readiness.patches[id] = { ...patchIssues, count: true };
  const model = scoreModel(parseScoreView(ordinaryView(raw)).content);
  assert.deepEqual(model.cells["255:24"], { k: "n", p: id, len: 1 });
  assert.equal(model.cellSlots["255:24"].register.conflicts.length, 1);
  assert.equal(model.ready, false);
  assert.equal(model.readiness.patches[id].count, true);
});

test("score parser refuses malformed addresses/cells/patches and contradictory evidence flags", () => {
  for (const key of ["00:0", "0:00", "256:0", "0:25", "1.0:0", "-1:0"]) {
    const raw = content(); raw.cells[key] = { removed: false, register: reg({ k: "d", pad: 0 }, isource(1)) };
    assert.throws(() => parseScoreView(ordinaryView(raw)), /address/);
  }
  for (const cell of [{ k: "n", p: id64(1), len: 2 }, { k: "d", pad: 10 }, { k: "n", p: id64(1), ts: 0 }]) {
    const raw = content(); raw.cells["255:0"] = { removed: false, register: reg(cell, isource(1)) };
    assert.throws(() => parseScoreView(ordinaryView(raw)));
  }
  const raw = content(); raw.readiness.cells["0:0"] = cellIssues;
  assert.throws(() => parseScoreView(ordinaryView(raw)), /absent slot/);
  raw.readiness.cells = {}; raw.readiness.headerConflicts = true;
  assert.throws(() => parseScoreView(ordinaryView(raw)), /header readiness/);
  const badPatch = content(); badPatch.patches[id64(1)] = { removed: false, register: reg({ ...legacyJamPatch("sine"), unknown: 1 }, isource(1)) };
  assert.throws(() => parseScoreView(ordinaryView(badPatch)), /patch/);
});

test("score register duplicate provenance and excessive evidence are rejected", () => {
  const raw = content(); raw.cells["0:0"] = { removed: false, register: reg({ k: "d", pad: 0 }, isource(1), [{ value: { k: "d", pad: 1 }, source: isource(1) }]) };
  assert.throws(() => parseScoreView(ordinaryView(raw)), /repeats a source/);
  raw.cells["0:0"] = { removed: true, register: reg(null, isource(1), Array.from({ length: 68_192 }, () => ({ value: null, source: isource(2) }))) };
  assert.throws(() => parseScoreView(ordinaryView(raw)), /evidence exceeds/);
});

test("new Score DTO records reject unknown/missing fields instead of silently stripping future content", () => {
  assert.throws(() => parseScoreView(ordinaryView({ ...content(), automation: {} })), /fields/);
  const missing = content() as unknown as Record<string, unknown>; delete missing.bpm;
  assert.throws(() => parseScoreView(ordinaryView(missing)), /fields/);
  const readiness = content(); (readiness.readiness as unknown as Record<string, unknown>).future = false;
  assert.throws(() => parseScoreView(ordinaryView(readiness)), /fields/);
  for (const mutate of [
    (slot: Record<string, unknown>) => { slot.future = {}; },
    (slot: Record<string, unknown>) => { (slot.register as Record<string, unknown>).future = {}; },
    (slot: Record<string, unknown>) => { ((slot.register as Record<string, unknown>).selected as Record<string, unknown>).future = {}; },
    (slot: Record<string, unknown>) => { (((slot.register as Record<string, unknown>).selected as Record<string, unknown>).source as Record<string, unknown>).future = {}; },
  ]) {
    const raw = content(), slot: Record<string, unknown> = { removed: false, register: reg({ k: "d", pad: 0 }, isource(1)) };
    mutate(slot); raw.cells["0:0"] = slot;
    assert.throws(() => parseScoreView(ordinaryView(raw)), /fields/);
  }
  const access = content(); let calls = 0;
  Object.defineProperty(access, "bpm", { enumerable: true, get: () => { calls++; return null; } });
  assert.throws(() => parseScoreView(ordinaryView(access)), /accessor/);
  assert.equal(calls, 0);
  const dynamic = content(); let issueReads = 0;
  Object.defineProperty(dynamic.readiness.cells, "0:0", { enumerable: true, get: () => { issueReads++; return cellIssues; } });
  assert.throws(() => parseScoreView(ordinaryView(dynamic)), /accessor/);
  assert.equal(issueReads, 0);
});

test("score commands pin explicit objectKind while legacy omission still requests Flipnote only", async () => {
  const ipc = fakeIpc(), target = { server: SERVER, channel: CHANNEL }, object = id32(9);
  for (const command of ["studio_read", "studio_create", "studio_apply"]) ipc.on(command, () => ordinaryView(content()));
  assert.equal((await studioRead(ipc, target, object, "score"))?.content.kind, "score");
  assert.equal((await studioCreate(ipc, target, { object, nonce: id32(1), title: "score", createdAtMs: 0, objectKind: "score" })).content.kind, "score");
  assert.equal((await studioApply(ipc, target, { object, epochId: id32(2), nonce: id32(3), body: "{}", objectKind: "score" })).content.kind, "score");
  assert.ok(ipc.calls.every((call) => call.args.objectKind === "score"));
  await assert.rejects(studioRead(ipc, target, object), /Flipnote/);
  ipc.on("studio_read", () => ordinaryView(flipnoteContent({ frames: [] })));
  assert.equal((await studioRead(ipc, target, object))?.content.kind, "flipnote");
  assert.equal("objectKind" in ipc.calls.at(-1)!.args, false);
  await assert.rejects(studioRead(ipc, target, object, "score"), /Score/);
  await overlayLifecycle(ipc, { ...target, object, objectKind: "score" });
  assert.equal(ipc.calls.at(-1)?.args.objectKind, "score");
  await assert.rejects(overlayLifecycle(ipc, { ...target, object: null, objectKind: "score" }), /requires an object/);
  ipc.on("studio_recovery_read", () => ({ v: 1, kind: "recoveryVersion", historical: true,
    version: { snapshot: id64(1), epoch: "1", staged: false, bytes: 1, reason: "rewound" }, channel: CHANNEL,
    content: flipnoteContent({ frames: [] }) }));
  await assert.rejects(recoveryRead(ipc, { ...target, object, objectKind: "score" }, id64(1)), /kind disagrees/);
});

test("stable group and optional Score target/event metadata survive bridge parsing", () => {
  assert.equal(parseScoreView({ ...ordinaryView(content()), group: "aabb00" }).group, "aabb00");
  for (const group of [7, "AA", "a", "aa".repeat(257), ""]) assert.throws(() => parseScoreView({ ...ordinaryView(content()), group }), /group/);
  const event = { server: SERVER, docType: 16, logicalKey: id32(1), channel: CHANNEL, object: id32(1), objectKind: "score", state: "open" };
  assert.equal(parseSettlementChanged(event).objectKind, "score");
  assert.throws(() => parseSettlementChanged({ ...event, object: null }), /objectKind/);
  const ticket = { v: 1, kind: "unconfirmedOverlaySaveTicket", channel: CHANNEL, object: id32(1), objectKind: "score", basis: id64(1), branch: id64(2), provenance: "unconfirmed" };
  assert.equal(parseUnconfirmedOverlaySaveTicket(ticket).objectKind, "score");
});
