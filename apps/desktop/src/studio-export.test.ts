import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import test from "node:test";
import { PATCH_OSC_WAVES, TAKE_MAX_BYTES, type JamPatch } from "./jam-contract.ts";
import { jamPatchId } from "./jam-patch.ts";
import { validateJamTake } from "./jam-recorder.ts";
import { canonicalJson } from "./studio-contract.ts";
import { exportFlipnoteTake, exportScorePlayback, exportScoreTake, scoreStepMs, studioPatchUnion, type ScoreExportInput } from "./studio-export.ts";
import { admittedSounds, scoreVoices } from "./score-voices.ts";

const vector = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
const ctx = { group: vector.group as string, exporter: vector.exporter as string };
function score(overrides: Partial<ScoreExportInput> = {}): ScoreExportInput {
  if (overrides.notes) {
    const { cells: _cells, ...rest } = overrides;
    return { id: vector.object, bpm: 120, bpb: 4, steps: 256, patches: { [vector.patchId]: structuredClone(vector.patch) }, ...rest };
  }
  return { id: vector.object, bpm: 120, bpb: 4, steps: 256, patches: { [vector.patchId]: structuredClone(vector.patch) }, cells: {}, ...overrides };
}
async function patches(count: number): Promise<Record<string, JamPatch>> {
  const result: Record<string, JamPatch> = {};
  for (let i = 0; i < count; i++) {
    const patch: JamPatch = structuredClone(vector.patch);
    patch.o = [{ w: 3, t: 24, c: -50, l: 100 }, { w: 3, t: -24, c: 50, l: 100 }, { w: 3, t: 24, c: -50, l: 100 }];
    patch.e = { a: 5000, d: 5000, s: 100, r: 8000 };
    patch.f = { m: 2, c: 18000, q: 100, e: -100 };
    patch.l = { r: 1200, d: 100, t: 2 }; patch.x = { c: i, d: 100, r: 100 };
    result[await jamPatchId(patch)] = patch;
  }
  return result;
}

test("Gate 6 derivation vectors pin raw ids, framed SHA-256, and epoch beyond JS precision", async () => {
  assert.equal(vector.version, 1);
  const { take, patchIds } = await exportFlipnoteTake({ id: vector.object, epoch: BigInt(vector.epoch), patches: {} }, ctx);
  assert.equal(take.call, vector.flipnoteCall);
  assert.equal(take.lanes[0].sn, vector.flipnoteLane);
  assert.deepEqual(patchIds, []);
  assert.equal(canonicalJson(take), vector.noScoreTake);
  const scored = await exportScoreTake(score(), ctx);
  assert.equal(scored.call, vector.scoreCall);
  assert.equal(scored.lanes[0].sn, vector.scoreLane);
  assert.equal(await jamPatchId(vector.patch), vector.patchId, "existing patch identity is reused");

  for (const epoch of [0n, 1n << 53n, 0xffff_ffff_ffff_ffffn]) {
    // Independent Node oracle, deliberately not the module's hash helper.
    const h = createHash("sha256"), epochBytes = Buffer.alloc(8); epochBytes.writeBigUInt64BE(epoch);
    for (const part of [Buffer.from("catcoms-flipnote-export:v1"), Buffer.from(vector.object, "hex"), epochBytes]) {
      const length = Buffer.alloc(4); length.writeUInt32BE(part.length); h.update(length).update(part);
    }
    const exported = await exportFlipnoteTake({ id: vector.object, epoch, patches: {} }, ctx);
    assert.equal(exported.take.call, h.digest("hex"));
  }
  for (const epoch of [-1n, 1n << 64n, Number.MAX_SAFE_INTEGER, "0"]) {
    await assert.rejects(exportFlipnoteTake({ id: vector.object, epoch: epoch as bigint, patches: {} }, ctx), /invalid epoch/);
  }
});

test("score export has exact note/drum timing, order and contiguous sequence with existing take validation", async () => {
  const cells: ScoreExportInput["cells"] = {
    "2:24": { k: "n", p: vector.patchId }, "0:2": { k: "d", pad: 3 },
    "1:0": { k: "n", p: vector.patchId }, "0:0": { k: "n", p: vector.patchId },
  };
  const take = await exportScoreTake(score({ cells }), ctx);
  assert.equal(validateJamTake(take).ok, true);
  assert.deepEqual(take.events, [
    { ms: 0, lane: 0, n: 60, on: 1, w: "sawtooth", p: 0, q: 0 },
    { ms: 0, lane: 0, n: 3, d: 1, q: 1 },
    { ms: 124, lane: 0, n: 60, on: 0, q: 2 },
    { ms: 125, lane: 0, n: 60, on: 1, w: "sawtooth", p: 0, q: 3 },
    { ms: 249, lane: 0, n: 60, on: 0, q: 4 },
    { ms: 250, lane: 0, n: 84, on: 1, w: "sawtooth", p: 0, q: 5 },
    { ms: 374, lane: 0, n: 84, on: 0, q: 6 },
  ]);
  assert.deepEqual(await exportScoreTake(score({ cells: Object.fromEntries(Object.entries(cells).reverse()) }), ctx), take);
});

test("every tempo and both score bounds produce positive bounded note holds", async () => {
  for (let bpm = 40; bpm <= 240; bpm++) {
    const take = await exportScoreTake(score({ bpm, bpb: bpm % 2 ? 1 : 8, cells: {
      "0:0": { k: "n", p: vector.patchId }, "255:24": { k: "n", p: vector.patchId },
    } }), ctx);
    assert.equal(take.events.length, 4);
    assert.ok(take.events[1].ms > take.events[0].ms);
    assert.ok(take.events[3].ms > take.events[2].ms);
    assert.equal(take.events[3].ms, Math.floor(256 * 15000 / bpm) - 1);
  }
});

test("Score playback carries the authored grid end beside the unchanged jam-take:v1 export", async () => {
  for (const [bpm, steps] of [[120, 16], [120, 64], [40, 256], [240, 1], [97, 37]]) {
    const input = score({ bpm, steps, cells: { "0:0": { k: "n", p: vector.patchId } } });
    const playback = await exportScorePlayback(input, ctx);
    assert.equal(playback.gridMs, Math.floor(steps * 15000 / bpm), `${steps} steps at ${bpm} BPM`);
    assert.deepEqual(playback.take, await exportScoreTake(input, ctx), "the shared take format is unchanged");
    assert.ok(playback.take.events.at(-1)!.ms < playback.gridMs, "trailing rests exist only in gridMs");
  }
  assert.equal((await exportScorePlayback(score({ steps: 16 }), ctx)).gridMs, 2000, "an empty grid still has its length");
});

test("authored grid lengths preserve note-off times, adjacency, independent rows and drum hits", async () => {
  const legacy = score({ cells: { "0:0": { k: "n", p: vector.patchId } } });
  assert.deepEqual(await exportScoreTake(legacy, ctx), await exportScoreTake(score({ cells: { "0:0": { k: "n", p: vector.patchId, len: 1 } } }), ctx));
  const take = await exportScoreTake(score({ cells: {
    "0:0": { k: "n", p: vector.patchId, len: 4 }, "4:0": { k: "n", p: vector.patchId, len: 2 },
    "1:1": { k: "n", p: vector.patchId, len: 3 }, "2:2": { k: "d", pad: 0 },
  } }), ctx);
  assert.deepEqual(take.events.filter((e) => "on" in e && e.on === 0).map((e) => [e.n, e.ms]), [[60, 499], [61, 499], [60, 749]]);
  const longest = await exportScoreTake(score({ bpm: 40, cells: { "0:0": { k: "n", p: vector.patchId, len: 256 } } }), ctx);
  assert.equal(longest.events[1].ms, 95999, "content keeps the authored hold; playback watchdog integration is a later gate item");
});

// [desync fixes], owner decision 2026-10-20: overlapping holds all play. A same-pitch overlap used
// to refuse the export; now each overlapping note takes its own lane, and lengths stay authored.
test("invalid lengths refuse; same-pitch overlap plays on its own lane without shortening anything", async () => {
  for (const len of [0, -1, 1.5, 257, NaN, "4"]) {
    await assert.rejects(exportScoreTake(score({ cells: { "0:0": { k: "n", p: vector.patchId, len: len as number } } }), ctx), /note length/);
  }
  await assert.rejects(exportScoreTake(score({ cells: { "255:0": { k: "n", p: vector.patchId, len: 2 } } }), ctx), /note length/);
  await assert.rejects(exportScoreTake({ ...score(), cells: { "0:0": { k: "d", pad: 0, len: 4 } } }, ctx), /unexpected fields/);
  const other: JamPatch = structuredClone(vector.patch); other.o[0].w = 0;
  const otherId = await jamPatchId(other);
  const cells = { "0:0": { k: "n" as const, p: vector.patchId, len: 4 }, "3:0": { k: "n" as const, p: otherId, len: 1 } };
  const takes = [];
  for (const ordered of [cells, Object.fromEntries(Object.entries(cells).reverse())]) {
    takes.push(await exportScoreTake(score({ patches: { [vector.patchId]: vector.patch, [otherId]: other }, cells: ordered }), ctx));
  }
  assert.deepEqual(takes[0], takes[1], "input order does not matter");
  const take = takes[0];
  assert.equal(take.lanes.length, 2, "the second hold of the same pitch took a second lane");
  assert.notEqual(take.lanes[1].sn, take.lanes[0].sn);
  const ons = take.events.filter((e) => "on" in e && e.on === 1);
  assert.deepEqual(ons.map((e) => [e.ms, e.lane]), [[0, 0], [scoreStepMs(3, 120), 1]]);
  const offs = take.events.filter((e) => "on" in e && e.on === 0);
  assert.deepEqual(offs.map((e) => [e.ms, e.lane]), [[scoreStepMs(4, 120) - 1, 1], [scoreStepMs(4, 120) - 1, 0]].sort((a, b) => a[0] - b[0] || a[1] - b[1]), "both holds keep their authored ends");
});

test("voices: distinct notes in one cell all play, back-to-back notes share a lane, identical notes are refused as a caller bug", async () => {
  const other: JamPatch = structuredClone(vector.patch); other.o[0].w = 0;
  const otherId = await jamPatchId(other);
  const patches = { [vector.patchId]: vector.patch, [otherId]: other };
  const notes = [
    { step: 0, row: 0, value: { k: "n" as const, p: vector.patchId, len: 2 } },
    { step: 0, row: 0, value: { k: "n" as const, p: otherId, len: 2 } }, // a second sound in the same cell
    { step: 2, row: 0, value: { k: "n" as const, p: vector.patchId } }, // follows the first directly
    { step: 0, row: 0, value: { k: "d" as const, pad: 1 } }, // a drum in the same cell
  ];
  const take = await exportScoreTake(score({ patches, notes }), ctx);
  assert.equal(take.lanes.length, 2);
  const ons = take.events.filter((e) => ("on" in e && e.on === 1) || "d" in e);
  assert.equal(ons.length, 4, "every voice sounds");
  assert.deepEqual(take.events.filter((e) => "on" in e && e.on === 1 && e.ms === scoreStepMs(2, 120)).map((e) => e.lane), [0], "a note right after another reuses its lane");
  await assert.rejects(exportScoreTake(score({ patches, notes: [notes[0], notes[0]] }), ctx), /duplicate score note/);
  await assert.rejects(exportScoreTake({ ...score({ patches, notes }), cells: {} } as never, ctx), /unexpected fields/, "one shape or the other, never both");
});

// Design review H1: deduplicating before clipping let two concurrent lengths at the end of a
// shortened score become two identical voices, and the exporter refused the whole score.
test("what score-voices plays always exports: lengths clipped to one note and len 1 vs absent are one voice", async () => {
  const p = vector.patchId, layer = `8:0/${p}`;
  const voices = scoreVoices([
    { key: layer, step: 8, row: 0, layer: p, values: [{ k: "n", p, len: 8 }, { k: "n", p, len: 6 }, null] },
    { key: "2:1", step: 2, row: 1, layer: null, values: [{ k: "n", p, len: 1 }] },
    { key: `2:1/${p}`, step: 2, row: 1, layer: p, values: [{ k: "n", p }] },
  ], 12, admittedSounds([p]));
  assert.equal(voices.play.length, 2, "one clipped note at step 8, one note at step 2");
  const playback = await exportScorePlayback(score({ steps: 12, notes: voices.play }), ctx);
  assert.equal(playback.take.lanes.length, 1, "nothing doubled onto a second lane");
  assert.deepEqual(playback.take.events.filter((e) => "on" in e && e.on === 0).map((e) => e.ms), [scoreStepMs(3, 120) - 1, scoreStepMs(12, 120) - 1]);
});

test("the first oscillator defines the required fallback for all waves and multi-oscillator recipes", async () => {
  for (let w = 0; w < PATCH_OSC_WAVES.length; w++) {
    const patch: JamPatch = structuredClone(vector.patch);
    patch.o = [{ ...patch.o[0], w }, { ...patch.o[0], w: (w + 1) % 4 }];
    const id = await jamPatchId(patch);
    const take = await exportScoreTake(score({ patches: { [id]: patch }, cells: { "0:0": { k: "n", p: id } } }), ctx);
    assert.ok("w" in take.events[0]);
    assert.equal(take.events[0].w, PATCH_OSC_WAVES[w]);
  }
});

test("a maximal late-step 4096-note score with 64 maximal recipes fits the unchanged take cap", async () => {
  const table = await patches(64), ids = Object.keys(table).sort();
  const cells: Record<string, { k: "n"; p: string }> = {};
  // Latest possible addresses maximize timestamp digits. All cells are notes (two events each).
  for (let i = 0; i < 4096; i++) {
    const n = 256 * 25 - 4096 + i;
    cells[`${Math.floor(n / 25)}:${n % 25}`] = { k: "n", p: ids[i % 64] };
  }
  const take = await exportScoreTake(score({ bpm: 40, bpb: 8, patches: table, cells }), { ...ctx, group: "g".repeat(256) });
  assert.equal(take.events.length, 8192);
  assert.equal(take.patches.length, 64);
  assert.equal(validateJamTake(take).ok, true);
  assert.ok(new TextEncoder().encode(canonicalJson(take)).length < TAKE_MAX_BYTES);
});

// Review of e7250d48, test gap: the test above has one lane and no volumes. Layers add both: up to
// 64 same-pitch lanes (two-digit lane numbers on every event) and a `v` on every attack. This is
// the largest combination: 4096 notes as 64 sounds stacked on each of the 64 latest cells, every
// one at an explicit `vol: 100` (written as `v: 100`, the widest value), with 64 maximal recipes
// and the widest-encoding 256-byte group: control characters, which JSON writes as six-byte
// `\u00XX` escapes. The re-review of 73b68ada measured this combination at 516,557 bytes, 7,731
// under the cap; with `v: 99` and a plain group it was 510,915.
test("a maximal layered score, 64 lanes deep with a volume on every note, fits the unchanged take cap", async () => {
  const table = await patches(64), ids = Object.keys(table).sort();
  const notes: { step: number; row: number; value: { k: "n"; p: string; len: number; vol: number } }[] = [];
  for (let cell = 0; cell < 64; cell++) {
    const n = 256 * 25 - 64 + cell, step = Math.floor(n / 25), row = n % 25;
    // One step each: a longer hold would overlap the same pitch a step later and need more lanes.
    for (const p of ids) notes.push({ step, row, value: { k: "n", p, len: 1, vol: 100 } });
  }
  const group = "\u0001".repeat(256);
  const playback = await exportScorePlayback(score({ bpm: 40, bpb: 8, patches: table, notes }), { ...ctx, group });
  const { take } = playback;
  assert.equal(playback.skipped ?? 0, 0, "every note has a lane");
  assert.equal(take.lanes.length, 64);
  assert.equal(take.events.length, 8192);
  assert.equal(take.group, group);
  assert.ok(take.events.every((e) => !("on" in e) || e.on === 0 || e.v === 100), "an explicit full volume on every attack");
  assert.equal(validateJamTake(take).ok, true);
  const bytes = new TextEncoder().encode(canonicalJson(take)).length;
  assert.ok(bytes < TAKE_MAX_BYTES, `${bytes} bytes`);
  assert.ok(bytes > 514_000, `${bytes} bytes: the case is still near the cap, or it no longer tests the margin`);
});

test("patch union keeps score indices, deduplicates exact recipes, and refuses the 65th identity", async () => {
  const table = await patches(65), entries = Object.entries(table).sort(([a], [b]) => a.localeCompare(b));
  const first = Object.fromEntries(entries.slice(0, 63)), last = Object.fromEntries(entries.slice(63, 64));
  const union = await studioPatchUnion(first, last);
  assert.deepEqual(union.ids, [...entries.slice(0, 63).map(([id]) => id), entries[63][0]]);
  assert.deepEqual(await studioPatchUnion(first, { ...first, ...last }), union);
  await assert.rejects(studioPatchUnion(Object.fromEntries(entries.slice(0, 64)), Object.fromEntries(entries.slice(64))), /union exceeds 64/);
  const source = score({ patches: first, cells: { "0:0": { k: "n", p: entries[62][0] } } });
  const linked = await exportFlipnoteTake({ id: vector.object, epoch: 1n, score: source, patches: last }, ctx);
  assert.equal(linked.take.patches.length, 64);
  assert.ok("p" in linked.take.events[0]);
  assert.equal(linked.take.events[0].p, 62, "appended SFX cannot renumber score recipes");
});

test("score export rejects schema, header, cell, identity and recipe violations", async () => {
  for (const invalid of [
    { ...score(), bpm: 39 }, { ...score(), bpm: 241 }, { ...score(), bpb: 0 }, { ...score(), bpb: 9 },
    { ...score(), steps: 0 }, { ...score(), steps: 257 }, { ...score(), extra: true }, { ...score(), id: "A".repeat(32) },
    score({ cells: { "256:0": { k: "d", pad: 0 } } }), score({ cells: { "0:25": { k: "d", pad: 0 } } }),
    score({ cells: { "00:0": { k: "d", pad: 0 } } }), score({ cells: { "0:0": { k: "d", pad: 10 } } }),
    score({ cells: { "0:0": { k: "n", p: "0".repeat(64) } } }),
    { ...score(), cells: { "0:0": { k: "n", p: vector.patchId, author: ctx.exporter } } },
    { ...score(), patches: { ["0".repeat(64)]: vector.patch } },
    { ...score(), patches: { [vector.patchId]: { ...vector.patch, extra: true } } },
    { ...score(), cells: Object.fromEntries(Array.from({ length: 4097 }, (_, i) => [`${Math.floor(i / 25)}:${i % 25}`, { k: "d", pad: 0 }])) },
    { ...score(), get bpm() { throw new Error("must not run accessor"); } },
  ]) await assert.rejects(exportScoreTake(invalid, ctx));
  for (const invalidContext of [{ ...ctx, group: "" }, { ...ctx, group: "\ud800" }, { ...ctx, group: "é".repeat(129) }, { ...ctx, exporter: "f".repeat(8) }]) {
    await assert.rejects(exportScoreTake(score(), invalidContext));
  }
  await assert.rejects(exportScoreTake(score(), { ...ctx, get group() { throw new Error("context accessor must not run"); } }), /accessor fields/);
});

test("mutable score, patch, SFX table and context are captured before any digest await", async () => {
  const source = structuredClone(score({ cells: { "0:0": { k: "n", p: vector.patchId, len: 4 } } }));
  const mutableContext = { ...ctx }, sfx = { [vector.patchId]: structuredClone(vector.patch) };
  const expected = await exportFlipnoteTake({ id: vector.object, epoch: 1n, patches: sfx, score: source }, mutableContext);
  const pending = exportFlipnoteTake({ id: vector.object, epoch: 1n, patches: sfx, score: source }, mutableContext);
  (source as { bpm: number }).bpm = 40;
  (source.cells["0:0"] as { len: number }).len = 1;
  source.patches[vector.patchId].o[0].w = 0;
  (source.cells as Record<string, unknown>)["1:0"] = { k: "d", pad: 0 };
  sfx[vector.patchId].o[0].w = 1; mutableContext.exporter = "a".repeat(64);
  assert.deepEqual(await pending, expected);
});
