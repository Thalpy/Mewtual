import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { ME, CHANNEL, fsource, flipnoteContent, id32, id64, isource, ordinaryView, pixBytes, reg } from "./studio-testkit.ts";
import { parseFlipnoteView, parseScoreView, scoreModel } from "./studio-native.ts";
import { flipnotePixaFromNative, scorePlaybackFromNative, scoreTakeFromNative } from "./studio-resolved-export.ts";
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

test("conflicted/missing/overflow animation state refuses instead of exporting an incomplete selected view", async () => {
  const view = animation(), held = new Map([[`${cid}:${pixBytes().length}`, pixBytes()]]);
  view.content.frames[frame].pixels.conflicts.push({ value: { cid: id64(99), bytes: 1 }, source: fsource(99) });
  await assert.rejects(flipnotePixaFromNative(object, view, held, ME), /resolve/);
});

// Contract change, owner decision 2026-10-20 ("keep everything, adds win"; nothing blocks
// playback): a Score with header conflicts, concurrent notes, overlaps or a gone sound used to refuse
// here. It now plays every voice score-voices.ts admits and counts the rest as skipped.
test("a Score with conflicts, overlaps and a gone sound plays what it can and counts what it cannot", async () => {
  const vector = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
  const p = vector.patchId as string, gone = id64(77), issue = { count: false, extent: false, recipe: false, overlap: false, conflict: false };
  const view = parseScoreView({ ...ordinaryView({ kind: "score", title: reg("a", isource(1), [{ value: "b", source: isource(2) }]), bpm: null, bpb: null, steps: reg(16, isource(3)),
    cells: { "0:0": { removed: false, register: reg({ k: "n", p, len: 4 }, isource(4)) } },
    // A clear won the register, and two members' different notes are both still live: add wins.
    layers: { [`2:0/${p}`]: { cleared: false, register: reg(null, isource(5), [{ value: { k: "n", p, len: 2 }, source: isource(6) }, { value: { k: "n", p }, source: isource(7) }]) },
      [`0:1/${gone}`]: { cleared: false, register: reg({ k: "n", p: gone }, isource(8)) } },
    patches: { [p]: { removed: false, register: reg(vector.patch, isource(9)) } },
    readiness: { cells: { "0:0": { ...issue, overlap: true } }, patches: {}, headerConflicts: true,
      layers: { [`2:0/${p}`]: { ...issue, overlap: true, conflict: true }, [`0:1/${gone}`]: { ...issue, recipe: true } } } }), group: "deadbeef" });
  const take = await scoreTakeFromNative(object, view, ME);
  const ons = take.events.filter((e) => "on" in e && e.on === 1);
  assert.equal(ons.length, 3, "the legacy note and both concurrent layer notes sound");
  assert.equal(take.lanes.length, 3, "three overlapping holds of one pitch, three lanes");
  assert.equal((await scorePlaybackFromNative(object, view, ME)).skipped, 1, "the note whose sound is gone");
  assert.equal(scoreModel(view.content).notes[`2:0/${p}`]?.k, "n", "a slot whose register a clear won still shows a live note");
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
