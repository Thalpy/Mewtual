// What a Score plays (score-voices.ts). The shared cases are also run by Rust readiness, so the two
// cannot drift: a readback whose readiness differs from the working projection's looks like a
// source change and stops playback.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { admittedSounds, scoreVoices, type ScoreVoiceSlot } from "./score-voices.ts";
import { SCORE_MAX_CELLS, parseNoteKey, type ScoreCell } from "./studio-contract.ts";

const shared = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-score-readiness-v1.json", import.meta.url), "utf8")) as {
  cases: { name: string; steps: number; sounds: string[]; slots: { key: string; values: (ScoreCell | null)[] }[];
    issues: Record<string, Partial<Record<string, boolean>>>; play: { step: number; row: number; value: ScoreCell }[]; skipped: number }[];
};

/** The fixture's raw slots, exactly as studio-native's `scoreVoiceSlots` builds them: every
 * alternative, selected first; whether a slot is legacy or layered comes from its key alone. */
function slotsOf(raw: { key: string; values: (ScoreCell | null)[] }[]): ScoreVoiceSlot[] {
  return raw.map(({ key, values }) => {
    const parsed = parseNoteKey(key);
    assert.ok(parsed, key);
    return { key, step: parsed.step, row: parsed.row, layer: parsed.layer, values };
  });
}

for (const c of shared.cases) {
  test(`shared: ${c.name}`, () => {
    const result = scoreVoices(slotsOf(c.slots), c.steps, admittedSounds(c.sounds));
    assert.deepEqual(result.play, c.play);
    assert.equal(result.skipped, c.skipped);
    const expected = Object.fromEntries(Object.entries(c.issues).map(([key, flags]) =>
      [key, { count: false, extent: false, recipe: false, overlap: false, conflict: false, ...flags }]));
    assert.deepEqual(result.issues, expected);
  });
}

test("voices after the 4096th in canonical order are skipped and flagged, whatever order the slots came in", () => {
  const sound = "aa".repeat(32);
  const slots: ScoreVoiceSlot[] = [];
  for (let i = 0; i <= SCORE_MAX_CELLS; i++) slots.push({ key: `${i % 256}:${Math.floor(i / 256)}/d`, step: i % 256, row: Math.floor(i / 256), layer: "d", values: [{ k: "d", pad: 0 }] });
  const forward = scoreVoices(slots, 256, admittedSounds([sound]));
  const reversed = scoreVoices([...slots].reverse(), 256, admittedSounds([sound]));
  assert.deepEqual(forward, reversed);
  assert.equal(forward.play.length, SCORE_MAX_CELLS); assert.equal(forward.skipped, 1);
  // Canonical order is (step, row): the last voice is the one at the highest step of the last row
  // reached, which here is step 255 of row 15.
  assert.deepEqual(Object.keys(forward.issues), ["255:15/d"]);
  assert.equal(forward.issues["255:15/d"].count, true);
});

test("only the first 64 active sounds by id may play", () => {
  const ids = Array.from({ length: 65 }, (_, i) => i.toString(16).padStart(64, "0"));
  const admitted = admittedSounds([...ids].reverse());
  assert.equal(admitted.size, 64); assert.equal(admitted.has(ids[64]), false);
  const result = scoreVoices([{ key: `0:0/${ids[64]}`, step: 0, row: 0, layer: ids[64], values: [{ k: "n", p: ids[64] }] }], 16, admitted);
  assert.equal(result.skipped, 1); assert.equal(result.issues[`0:0/${ids[64]}`].recipe, true);
});

// Design review H3: one member can stack thousands of concurrent values on one key (empty
// predecessors at a stale frontier), and native computes voices on every view. The old pairwise
// overlap walk took 24 s for 20,480 voices on one row and sound; the sweep is a sort plus one pass.
test("overlap over many concurrent values on one row and sound is linear after the sort", () => {
  const sound = "cd".repeat(32), values: ScoreCell[] = [];
  for (let len = 1; values.length < 20_480; len++) for (let vol = 1; vol <= 100 && values.length < 20_480; vol++) values.push({ k: "n", p: sound, len: Math.min(len, 256), vol });
  const slots: ScoreVoiceSlot[] = [{ key: `0:0/${sound}`, step: 0, row: 0, layer: sound, values }];
  const started = performance.now();
  const result = scoreVoices(slots, 256, admittedSounds([sound]));
  const elapsed = performance.now() - started;
  assert.equal(result.all.length, 20_480);
  assert.equal(result.play.length, SCORE_MAX_CELLS);
  assert.equal(result.issues[`0:0/${sound}`].overlap, true);
  assert.ok(elapsed < 2000, `took ${elapsed.toFixed(0)} ms`);
});

test("overlap matches the pairwise definition on random lanes", () => {
  // A small deterministic generator: the sweep must flag exactly the voices that intersect another
  // voice of their (row, sound), whatever the mix of starts and lengths.
  let seed = 7;
  const next = (n: number) => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed % n; };
  const sound = "ef".repeat(32);
  for (let round = 0; round < 50; round++) {
    const slots: ScoreVoiceSlot[] = [];
    const used = new Set<string>();
    for (let i = 0; i < 12; i++) {
      const step = next(24), len = 1 + next(6), key = `${step}:0/${sound}`;
      if (used.has(key)) continue;
      used.add(key);
      slots.push({ key, step, row: 0, layer: sound, values: [{ k: "n", p: sound, ...(len > 1 ? { len } : {}) }] });
    }
    const result = scoreVoices(slots, 256, admittedSounds([sound]));
    for (const a of slots) {
      const aEnd = a.step + ((a.values[0] as { len?: number }).len ?? 1);
      const pairwise = slots.some((b) => b !== a && b.step < aEnd && a.step < b.step + ((b.values[0] as { len?: number }).len ?? 1));
      assert.equal(result.issues[a.key]?.overlap ?? false, pairwise, `round ${round} ${a.key}`);
    }
  }
});

test("note keys round-trip and reject anything that is not canonical", () => {
  const sound = "ab".repeat(32);
  assert.deepEqual(parseNoteKey("255:24"), { step: 255, row: 24, layer: null });
  assert.deepEqual(parseNoteKey(`3:7/${sound}`), { step: 3, row: 7, layer: sound });
  assert.deepEqual(parseNoteKey("0:0/d"), { step: 0, row: 0, layer: "d" });
  for (const bad of ["256:0", "0:25", "01:0", "0:01", "0:0/", "0:0/D", `0:0/${sound.toUpperCase()}`, `0:0/${sound}0`, "0:0/x", " 0:0", "0:0 "]) {
    assert.equal(parseNoteKey(bad), null, bad);
  }
});
