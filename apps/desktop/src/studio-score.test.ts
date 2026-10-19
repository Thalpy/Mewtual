// Static score bodies are not an enabled native writer. These vectors pin future operation
// consumers to the same authored-length content the existing export helper already accepts.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { jamPatchId } from "./jam-patch.ts";
import { canonicalJson, SCORE_MAX_CELLS, SCORE_MAX_PATCHES, SCORE_MAX_STEPS, SCORE_ROWS, type ScoreCell, type ScoreOp } from "./studio-contract.ts";
import { exportScoreTake } from "./studio-export.ts";

const vectors = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-score-ops-v1.json", import.meta.url), "utf8")) as {
  version: number; patch_id: string; cases: { name: string; body: string }[];
};
const read = (name: string): ScoreOp => JSON.parse(vectors.cases.find((row) => row.name === name)!.body);

test("score Rust vectors match frontend canonical operation bytes and every operation family", () => {
  assert.equal(vectors.version, 1);
  assert.equal(vectors.cases.length, 18);
  const families = new Set<string>();
  for (const row of vectors.cases) {
    const op: ScoreOp = JSON.parse(row.body);
    assert.equal(canonicalJson(op), row.body, row.name);
    families.add(op.op);
  }
  assert.deepEqual([...families].sort(), ["clear_cell", "remove_patch", "set_cell", "set_header", "set_patch"]);
  assert.deepEqual([SCORE_MAX_STEPS, SCORE_ROWS, SCORE_MAX_CELLS, SCORE_MAX_PATCHES], [256, 25, 4096, 64]);
});

test("score vectors preserve optional authored length while sharing export cells and patch identity", async () => {
  const legacy = read("note_legacy"), explicit = read("note_one"), held = read("note_hold"), patch = read("set_patch");
  assert.ok(legacy.op === "set_cell" && explicit.op === "set_cell" && held.op === "set_cell" && patch.op === "set_patch");
  assert.equal(Object.hasOwn(legacy.value, "len"), false);
  assert.equal(Object.hasOwn(explicit.value, "len"), true);
  assert.notEqual(canonicalJson(legacy), canonicalJson(explicit));
  assert.equal(await jamPatchId(patch.descriptor), vectors.patch_id);
  const ctx = { group: "stable-group-id", exporter: "1".repeat(64) };
  const score = (cell: ScoreCell) => ({ id: "2".repeat(32), bpm: 40, bpb: 4, steps: 256, patches: { [patch.patch]: patch.descriptor }, cells: { "0:0": cell } });
  assert.deepEqual(await exportScoreTake(score(legacy.value), ctx), await exportScoreTake(score(explicit.value), ctx));
  const take = await exportScoreTake(score(held.value), ctx);
  assert.equal(take.events[1].ms, 95999, "the typed cell carries the author's full hold into the existing take format");
});

test("score vectors carry an authored volume into the take's note-on and leave legacy notes without one", async () => {
  const quiet = read("note_volume"), full = read("note_volume_hold"), legacy = read("note_legacy"), patch = read("set_patch");
  assert.ok(quiet.op === "set_cell" && full.op === "set_cell" && legacy.op === "set_cell" && patch.op === "set_patch");
  assert.ok(quiet.value.k === "n" && full.value.k === "n" && legacy.value.k === "n");
  assert.equal(quiet.value.vol, 1); assert.equal(full.value.vol, 100); assert.equal(full.value.len, 4);
  assert.equal(Object.hasOwn(legacy.value, "vol"), false);
  const ctx = { group: "stable-group-id", exporter: "1".repeat(64) };
  const score = (cell: ScoreCell) => ({ id: "2".repeat(32), bpm: 120, bpb: 4, steps: 256, patches: { [patch.patch]: patch.descriptor }, cells: { "8:12": cell } });
  const on = (take: { events: readonly Record<string, unknown>[] }) => take.events.find((e) => e.on === 1)!;
  assert.equal(on(await exportScoreTake(score(quiet.value), ctx)).v, 1, "the level rides the note-on as `v`");
  assert.equal(on(await exportScoreTake(score(full.value), ctx)).v, 100, "an explicit full level is kept, not dropped");
  assert.equal("v" in on(await exportScoreTake(score(legacy.value), ctx)), false, "a note without a level exports exactly as before");
  await assert.rejects(exportScoreTake(score({ ...quiet.value, vol: 0 }), ctx), /volume/);
  await assert.rejects(exportScoreTake(score({ ...quiet.value, vol: 101 }), ctx), /volume/);
});
