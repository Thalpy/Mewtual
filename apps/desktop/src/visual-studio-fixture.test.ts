import assert from "node:assert/strict";
import test from "node:test";
import { decodePix } from "./pix.ts";
import { jamPatchId } from "./jam-patch.ts";
import { FLIPNOTE_H, FLIPNOTE_W } from "./studio-contract.ts";
import { base64ToBytes, flipnoteModel, parseFlipnoteView, parseIndexView, parseOverlayInspection, parseOverlayLifecycle, parseRecoveryListing, parseScoreView, scoreModel } from "./studio-native.ts";
import { VISUAL_FIXTURE_NOW, visualFixtureResponse } from "./visual-fixture.ts";
import { VISUAL_STUDIO_ART, VISUAL_STUDIO_CONFLICT_ART, VISUAL_STUDIO_CONFLICT_SCORE, VISUAL_STUDIO_GROUP, VISUAL_STUDIO_ME, VISUAL_STUDIO_PATCH, VISUAL_STUDIO_SCORE } from "./visual-studio-fixture.ts";
import { parseFaultView } from "./studio-fault.ts";

const target = { server: 1, channel: "1" };
const response = (command: string, args: Record<string, unknown> = {}) => visualFixtureResponse(command, { ...target, ...args }, true);
const readArt = (object = VISUAL_STUDIO_ART) => parseFlipnoteView(response("studio_read", { object }));
const readScore = (object = VISUAL_STUDIO_SCORE) => parseScoreView(response("studio_read", { object, objectKind: "score" }));

test("visual repair status has no invented fault authority and all repair/Closing writes refuse", () => {
  const t = { ...target, object: VISUAL_STUDIO_ART };
  for (const scope of ["source", "registry"] as const) {
    const view = parseFaultView(response("studio_fault_read", { object: t.object, scope }), t, scope);
    assert.equal(view.mayDecide, false); assert.equal(view.phase, "open"); assert.deepEqual(view.candidates, []);
  }
  for (const command of ["studio_fault_repair", "studio_closing_overlay_save_begin", "studio_closing_overlay_save"]) assert.throws(() => response(command, { object: t.object }), /does not implement/);
});

test("Studio and chat fixtures share native channel IDs without changing displayed content", () => {
  const base = visualFixtureResponse("resume_session") as { channel: string; channels: { id: string }[] }[];
  const studio = visualFixtureResponse("resume_session", {}, true) as { channel: string; channels: { id: string; name: string }[] }[];
  assert.equal(base[0].channel, "1"); assert.deepEqual(base[0].channels.map(c => c.id), ["1", "2", "3"]);
  assert.equal(studio[0].channel, "1"); assert.deepEqual(studio[0].channels.map(c => c.id), ["1", "2", "3"]); assert.equal(studio[0].channels[0].name, "general");
  assert.deepEqual(response("get_channels"), studio[0].channels);
  assert.deepEqual(response("get_messages"), visualFixtureResponse("get_messages", { server: 1, channel: "general" }));
  assert.deepEqual(response("get_messages", { channel: "2" }), visualFixtureResponse("get_messages", { server: 1, channel: "design" }));
  const members = response("get_members") as {identity: string; you: boolean}[];
  assert.ok(members.every(member => /^[0-9a-f]{64}$/.test(member.identity)));
  assert.equal(members.find(member => member.you)?.identity, VISUAL_STUDIO_ME);
  assert.equal((response("get_roles") as Record<string, string>)[VISUAL_STUDIO_ME], "owner");
  assert.throws(() => visualFixtureResponse("studio_list", target), /does not implement/);
});

test("all four fixture entries parse through real typed native readers with stable group and full identity", async () => {
  const index = parseIndexView(response("studio_list"));
  assert.equal(index.channel, "1"); assert.equal(index.epoch, "3"); assert.equal(index.publication, "local"); assert.equal(index.group, VISUAL_STUDIO_GROUP);
  assert.deepEqual(Object.keys(index.content.objects), [VISUAL_STUDIO_ART, VISUAL_STUDIO_SCORE, VISUAL_STUDIO_CONFLICT_SCORE, VISUAL_STUDIO_CONFLICT_ART]);
  assert.equal(response("get_own_device_id"), VISUAL_STUDIO_ME); assert.match(VISUAL_STUDIO_ME, /^[0-9a-f]{64}$/);
  for (const object of [VISUAL_STUDIO_ART, VISUAL_STUDIO_CONFLICT_ART]) assert.equal(readArt(object).content.kind, "flipnote");
  for (const object of [VISUAL_STUDIO_SCORE, VISUAL_STUDIO_CONFLICT_SCORE]) assert.equal(readScore(object).content.kind, "score");
  const score = scoreModel(readScore().content);
  assert.equal(score.ready, true); assert.equal(score.steps, 32); assert.ok(Object.values(score.cells).some(c => c.k === "n" && c.len === 3));
  assert.equal(await jamPatchId(score.patches[VISUAL_STUDIO_PATCH]), VISUAL_STUDIO_PATCH, "reuse the real shared golden recipe identity");
  assert.equal(readScore().content.title?.selected.source.author, VISUAL_STUDIO_ME);
});

test("art references resolve to distinct owned real PIX1 frames of the exact declared size", () => {
  const art = readArt();
  assert.equal(art.content.timeline.length, 3);
  const bodies: string[] = [];
  for (const id of art.content.timeline) {
    const frame = art.content.frames[id].pixels.selected.value;
    const fetched = response("request_blob_bounded", { cid: frame.cid, maxBytes: frame.bytes }) as { bytes: number; bytes_b64: string };
    const bytes = base64ToBytes(fetched.bytes_b64), decoded = decodePix(bytes);
    assert.equal(bytes.length, frame.bytes); assert.equal(fetched.bytes, frame.bytes);
    assert.equal(decoded.w, FLIPNOTE_W); assert.equal(decoded.h, FLIPNOTE_H); assert.ok(frame.bytes < 65536);
    bodies.push(fetched.bytes_b64);
    bytes.fill(0);
    assert.equal((response("request_blob_bounded", { cid: frame.cid, maxBytes: frame.bytes }) as { bytes_b64: string }).bytes_b64, fetched.bytes_b64);
    assert.throws(() => response("request_blob_bounded", { cid: frame.cid, maxBytes: frame.bytes - 1 }), /wrong declared blob length/);
  }
  assert.equal(new Set(bodies).size, 3);
  assert.equal(response("request_blob_bounded", { cid: "f".repeat(64), maxBytes: 100 }), null);
});

test("Sound fixture preserves authored SFX holds, linked Score and permanent removal evidence", () => {
  const art = flipnoteModel(readArt().content);
  assert.equal(art.sound.ready, true); assert.equal(art.sound.score, VISUAL_STUDIO_SCORE);
  assert.equal(art.sound.sfx.length, 1); assert.equal(art.sound.sfx[0].durationMs, 375);
  assert.equal(art.sound.deletedSfx.length, 1);
  const conflicted = flipnoteModel(readArt(VISUAL_STUDIO_CONFLICT_ART).content);
  assert.equal(conflicted.sound.ready, false); assert.equal(conflicted.sound.readiness.conflicts, true);
  assert.equal(conflicted.sound.sfx[0].conflicts.length, 1);
  const value = conflicted.sound.sfx[0].conflicts[0].value;
  assert.equal(value.op, "set_sfx"); if (value.op === "set_sfx") assert.equal(value.duration_ms, 750);
});

test("unready Score fixture keeps losing setters, remove-wins clear and out-of-length cells honestly", () => {
  const score = scoreModel(readScore(VISUAL_STUDIO_CONFLICT_SCORE).content);
  assert.equal(score.ready, false); assert.equal(score.readiness.headerConflicts, true);
  assert.equal(score.headers.bpm?.conflicts[0].value, 126);
  assert.equal(score.cellSlots["8:7"].removed, true); assert.equal(score.cells["8:7"], undefined);
  assert.equal(score.cellSlots["8:7"].register.selected.value?.k, "n"); assert.equal(score.cellSlots["8:7"].register.conflicts[0].value, null);
  assert.equal(score.readiness.cells["8:7"], undefined, "native readiness skips removed slots, retaining their raw conflict evidence separately");
  assert.equal(score.readiness.cells["255:24"].extent, true); assert.equal(score.cells["255:24"].k, "d");
});

test("empty draft/recovery reads use exact native shapes without fake retained state", () => {
  for (const args of [{}, { object: VISUAL_STUDIO_ART }, { object: VISUAL_STUDIO_SCORE, objectKind: "score" }]) {
    assert.equal(parseOverlayInspection(response("studio_overlay_read", args)).kind, "absent");
    const lifecycle = parseOverlayLifecycle(response("studio_overlay_lifecycle", args));
    assert.equal(lifecycle.branch, null); assert.equal(lifecycle.prepared, false); assert.equal(lifecycle.archive, null); assert.equal(lifecycle.disposed, null);
    const recovery = parseRecoveryListing(response("studio_recovery_list", args));
    assert.deepEqual(recovery.versions, []); assert.equal(recovery.pendingIntents, 0); assert.equal(recovery.source?.phase, "open");
  }
});

test("fixture rejects cross-scope/family requests and every unsupported write, repair or fake export", () => {
  for (const args of [{ server: 2 }, { channel: "general" }, { channel: "01" }, { objectKind: "score" }, { object: "bad" }]) assert.throws(() => response("studio_list", args));
  assert.throws(() => response("studio_read", { object: VISUAL_STUDIO_SCORE }), /family/);
  assert.throws(() => response("studio_read", { object: VISUAL_STUDIO_ART, objectKind: "score" }), /family/);
  assert.equal(response("studio_read", { object: "f".repeat(32) }), null);
  const before = readArt();
  for (const command of ["studio_create", "studio_apply", "studio_apply_index", "studio_repair", "studio_recovery_apply", "publish_pix", "publish_pixa", "export_studio_gif"]) assert.throws(() => response(command, { object: VISUAL_STUDIO_ART }), /Read-only visual Studio fixture/);
  assert.deepEqual(readArt(), before);
  assert.throws(() => response("unknown_dependency"), /does not implement/);
});

test("native fixture reads are isolated and use the existing frozen visual clock", () => {
  const first = readArt(); first.content.timeline.pop(); first.content.sound!.records = {};
  assert.equal(readArt().content.timeline.length, 3); assert.ok(Object.keys(readArt().content.sound!.records).length > 0);
  const selected = readArt().content.frames[readArt().content.timeline[0]].pixels.selected;
  assert.equal(selected.source.ts, VISUAL_FIXTURE_NOW - 60_000 + 100);
});
