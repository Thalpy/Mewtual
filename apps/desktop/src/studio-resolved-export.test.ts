import assert from "node:assert/strict";
import test from "node:test";
import { ME, CHANNEL, fsource, flipnoteContent, id32, id64, ordinaryView, pixBytes, reg } from "./studio-testkit.ts";
import { parseFlipnoteView, parseScoreView } from "./studio-native.ts";
import { flipnotePixaFromNative, scoreTakeFromNative } from "./studio-resolved-export.ts";
import { decodePixa } from "./pixa.ts";

const object = id32(1), frame = id32(2), cid = id64(3);
function score() { return parseScoreView({ ...ordinaryView({ kind: "score", title: null, bpm: null, bpb: null, steps: null,
  cells: {}, patches: {}, readiness: { cells: {}, patches: {}, headerConflicts: false } }), group: "deadbeef" }); }
function animation() { return parseFlipnoteView({ ...ordinaryView(flipnoteContent({ frames: [{ id: frame, cid, bytes: pixBytes().length }] })), group: "deadbeef" }); }

test("native score/animation export use trusted group, exact u64 epoch and all held frame bytes", async () => {
  const take = await scoreTakeFromNative(object, score(), ME); assert.equal(take.group, "deadbeef");
  const view = animation(); view.epoch = "18446744073709551615";
  const bytes = await flipnotePixaFromNative(object, view, new Map([[`${cid}:${pixBytes().length}`, pixBytes()]]), ME);
  const packageValue = decodePixa(bytes);
  assert.equal(packageValue.take.group, "deadbeef"); assert.equal(packageValue.frames.length, 1);
  await assert.rejects(flipnotePixaFromNative(object, view, new Map(), ME), /all exact/);
  delete view.group;
  await assert.rejects(flipnotePixaFromNative(object, view, new Map(), ME), /stable native group/);
});

test("linked score requires exact native target and same group/channel; no silent no-score fallback", async () => {
  const view = animation(), linked = score(), scoreId = id32(4);
  view.content.sound = { records: { "h/score": reg({ op: "set_header", field: "score", value: scoreId }, fsource(1)) },
    readiness: { sfxOverflow: [], patchOverflow: [], exportOverflow: [], missingFrames: [], missingPatches: [], conflicts: false } };
  const held = new Map([[`${cid}:${pixBytes().length}`, pixBytes()]]);
  await assert.rejects(flipnotePixaFromNative(object, view, held, ME), /linked Score/);
  await assert.rejects(flipnotePixaFromNative(object, view, held, ME, { object, view: linked }), /linked Score/);
  linked.group = "aabb";
  await assert.rejects(flipnotePixaFromNative(object, view, held, ME, { object: scoreId, view: linked }), /linked Score/);
  linked.group = "deadbeef"; linked.channel = "1";
  await assert.rejects(flipnotePixaFromNative(object, view, held, ME, { object: scoreId, view: linked }), /linked Score/);
  linked.channel = CHANNEL;
  assert.equal(decodePixa(await flipnotePixaFromNative(object, view, held, ME, { object: scoreId, view: linked })).frames.length, 1);
});

test("conflicted/missing/overflow native state refuses instead of exporting an incomplete selected view", async () => {
  const view = animation(), held = new Map([[`${cid}:${pixBytes().length}`, pixBytes()]]);
  view.content.frames[frame].pixels.conflicts.push({ value: { cid: id64(99), bytes: 1 }, source: fsource(99) });
  await assert.rejects(flipnotePixaFromNative(object, view, held, ME), /resolve/);
  const unready = score(); unready.content.readiness.headerConflicts = true;
  await assert.rejects(scoreTakeFromNative(object, unready, ME), /resolve score/);
});

test("leaf export refuses missing/duplicate timeline records, ignored active entries and aggregate lies", async () => {
  const held = new Map([[`${cid}:${pixBytes().length}`, pixBytes()]]);
  for (const mutate of [(v: ReturnType<typeof animation>) => v.content.timeline.push(id32(9)),
    (v: ReturnType<typeof animation>) => v.content.timeline.push(frame),
    (v: ReturnType<typeof animation>) => v.content.frames[id32(9)] = v.content.frames[frame],
    (v: ReturnType<typeof animation>) => v.content.declaredFrameBytes = 1]) {
    const view = animation(); mutate(view); await assert.rejects(flipnotePixaFromNative(object, view, held, ME), /timeline|aggregate/);
  }
});
