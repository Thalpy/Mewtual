import assert from "node:assert/strict";
import test from "node:test";
import { flipnoteContent, fsource, id32, id64, ordinaryView, reg } from "./studio-testkit.ts";
import { flipnoteModel, parseFlipnoteView, parseFrameSoundContent } from "./studio-native.ts";
import { legacyJamPatch } from "./jam-patch.ts";
import type { FrameSoundContent } from "./studio-native.ts";

const id = id32(1), frame = id32(2), patchId = id64(3), exportId = id32(4);
function sound() { return { records: {} as Record<string, unknown>, readiness: {
  sfxOverflow: [] as string[], patchOverflow: [] as string[], exportOverflow: [] as string[], missingFrames: [] as string[], missingPatches: [] as string[], conflicts: false,
} }; }
const sfx = () => ({ op: "set_sfx", sfx: id, frame, patch: patchId, note: 60, duration_ms: 600_000 });
function model(raw: ReturnType<typeof sound>) {
  return flipnoteModel(parseFlipnoteView(ordinaryView({ ...flipnoteContent({ frames: [] }), sound: raw })).content).sound;
}

test("sound DTO keeps authored duration and every losing full operation/source", () => {
  const raw = sound(); raw.records[`s/${id}`] = reg(sfx(), fsource(1), [{ value: { ...sfx(), duration_ms: 1 }, source: fsource(2) }]);
  raw.readiness.conflicts = true;
  const output = model(raw);
  assert.equal(output.sfx[0].durationMs, 600_000);
  assert.equal(output.sfx[0].conflicts[0].source.opId, id64(2));
  assert.equal(output.sfx[0].conflicts[0].value.op, "set_sfx");
  assert.equal(output.ready, false);
  assert.equal(Object.keys(output.records).length, 1);
});

test("SFX/export deletion survives any setter winner while concurrent recipe removal is effective", () => {
  const raw = sound();
  raw.records[`s/${id}`] = reg(sfx(), fsource(1));
  raw.records[`q/${id}/${id64(2)}`] = reg({ op: "remove_sfx", sfx: id }, fsource(2));
  raw.records[`p/${patchId}`] = reg({ op: "set_patch", patch: patchId, descriptor: legacyJamPatch("sine") }, fsource(3),
    [{ value: { op: "remove_patch", patch: patchId }, source: fsource(4) }]);
  raw.records[`x/${exportId}/${id64(5)}`] = reg({ op: "set_export", export: exportId, cid: id64(6), bytes: 9, expiry: null }, fsource(5));
  raw.records[`z/${exportId}/${id64(7)}`] = reg({ op: "remove_export", export: exportId }, fsource(7));
  const output = model(raw);
  assert.deepEqual(output.sfx, []); assert.deepEqual(output.exports, []); assert.deepEqual(output.patches, {});
  assert.deepEqual(output.deletedSfx, [id]); assert.deepEqual(output.deletedExports, [exportId]);
  assert.equal(output.patchSlots[0].register.selected.value.op, "set_patch");
  assert.equal(output.patchSlots[0].removed, true);
  assert.equal(Object.keys(output.records).length, 5);
});

test("duplicate export births select smallest source operation regardless of map insertion order", () => {
  const raw = sound();
  raw.records[`x/${exportId}/${id64(9)}`] = reg({ op: "set_export", export: exportId, cid: id64(90), bytes: 9, expiry: 0 }, fsource(9));
  raw.records[`x/${exportId}/${id64(1)}`] = reg({ op: "set_export", export: exportId, cid: id64(10), bytes: 10 }, fsource(1));
  raw.readiness.conflicts = true;
  const output = model(raw);
  assert.equal(output.exports[0].cid, id64(10));
  assert.equal(output.exports[0].births.length, 2);
  assert.equal("expiry" in output.exports[0], false, "absence must not inherit an older winning birth's expiry");
  assert.equal(output.ready, false);
});

test("linked-score null selection and old art-only replies preserve honest absence", () => {
  const raw = sound(); raw.records["h/score"] = reg({ op: "set_header", field: "score", value: null }, fsource(1),
    [{ value: { op: "set_header", field: "score", value: id32(9) }, source: fsource(2) }]);
  raw.readiness.conflicts = true;
  const output = model(raw);
  assert.equal(output.score, null); assert.equal(output.scoreRegister?.conflicts.length, 1);
  const art = flipnoteModel(parseFlipnoteView(ordinaryView(flipnoteContent({ frames: [] }))).content);
  assert.deepEqual(art.sound.sfx, []); assert.equal(art.sound.score, null); assert.equal(art.sound.ready, true);
});

test("sound parser refuses old missing-duration SFX and mismatched physical key/body/source", () => {
  const raw = sound();
  raw.records[`s/${id}`] = reg({ op: "set_sfx", sfx: id, frame, patch: patchId, note: 60 }, fsource(1));
  assert.throws(() => parseFrameSoundContent(raw), /operation fields/);
  for (const duration_ms of [0, 600_001, 0.5]) {
    raw.records[`s/${id}`] = reg({ ...sfx(), duration_ms }, fsource(1));
    assert.throws(() => parseFrameSoundContent(raw), /bound|integer/);
  }
  raw.records = { [`s/${id32(9)}`]: reg(sfx(), fsource(1)) };
  assert.throws(() => parseFrameSoundContent(raw), /disagree/);
  raw.records = { [`q/${id}/${id64(9)}`]: reg({ op: "remove_sfx", sfx: id }, fsource(1)) };
  assert.throws(() => parseFrameSoundContent(raw), /disagree/);
  raw.records = { invalid: reg(sfx(), fsource(1)) };
  assert.throws(() => parseFrameSoundContent(raw), /physical key/);
});

test("sound readiness preserves overflow/missing IDs and rejects malformed or unsorted flags", () => {
  const raw = sound(); raw.readiness.sfxOverflow = [id]; raw.readiness.missingFrames = [id];
  raw.readiness.patchOverflow = [patchId]; raw.readiness.exportOverflow = [exportId]; raw.readiness.missingPatches = [id];
  const parsed = parseFrameSoundContent(raw);
  assert.deepEqual(parsed.readiness, raw.readiness);
  raw.readiness.sfxOverflow = [id, id];
  assert.throws(() => parseFrameSoundContent(raw), /sorted unique/);
  raw.readiness.sfxOverflow = [];
  (raw.readiness as unknown as Record<string, unknown>).conflicts = [];
  assert.throws(() => parseFrameSoundContent(raw), /boolean/);
});
