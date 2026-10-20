import assert from "node:assert/strict";
import test from "node:test";
import { seededNoise, SFX, sfxRecipe, type Breakpoints } from "./clip-sfx.ts";

const ascending = (points: Breakpoints) => points.every((p, i) => i === 0 || p[0] >= points[i - 1][0]);

test("every built-in sound is well formed: unique id, starts and ends silent, inside its length", () => {
  const ids = new Set<string>();
  for (const recipe of SFX) {
    assert.ok(!ids.has(recipe.id), `duplicate id ${recipe.id}`);
    ids.add(recipe.id);
    assert.ok(recipe.seconds > 0 && recipe.seconds <= 2, recipe.id);
    assert.ok(recipe.name && recipe.emoji, recipe.id);
    assert.ok(recipe.voices.length > 0, recipe.id);
    for (const voice of recipe.voices) {
      assert.ok(ascending(voice.level), `${recipe.id} level is in time order`);
      assert.equal(voice.level[0][1], 0, `${recipe.id} starts silent`);
      assert.equal(voice.level.at(-1)![1], 0, `${recipe.id} ends silent, so it never clicks off`);
      assert.ok(voice.level.at(-1)![0] <= recipe.seconds + 1e-9, `${recipe.id} fades out inside its length`);
      assert.ok(voice.level.every(([, v]) => v >= 0 && v <= 1), `${recipe.id} level is 0..1`);
      if (voice.wave === "noise") assert.ok(Number.isInteger(voice.seed), `${recipe.id} noise is seeded`);
      else {
        assert.ok(voice.pitch && ascending(voice.pitch), `${recipe.id} has a pitch line in time order`);
        // Under Nyquist at the lowest common rate, and above the 1 Hz floor exponential ramps need.
        assert.ok(voice.pitch!.every(([, hz]) => hz >= 20 && hz < 22_050), `${recipe.id} pitch is audible`);
      }
      if (voice.filter) assert.ok(ascending(voice.filter.cutoff) && voice.filter.cutoff.every(([, hz]) => hz >= 20 && hz < 22_050));
    }
  }
});

/** A breakpoint line's value at `t` (straight lines between points, flat past the ends). */
function levelAt(points: Breakpoints, t: number): number {
  if (t <= points[0][0]) return points[0][1];
  for (let i = 1; i < points.length; i++) {
    const [t0, v0] = points[i - 1], [t1, v1] = points[i];
    if (t <= t1) return t1 === t0 ? v1 : v0 + ((v1 - v0) * (t - t0)) / (t1 - t0);
  }
  return points.at(-1)![1];
}

test("no built-in sound's voices add up past full scale at any moment, so none can clip alone", () => {
  // Found in Edge: Pow peaked at 1.43 and Thud at 1.13 when two loud voices struck together.
  for (const recipe of SFX) {
    for (let t = 0; t <= recipe.seconds; t += 0.0005) {
      const sum = recipe.voices.reduce((s, v) => s + levelAt(v.level, t), 0);
      assert.ok(sum <= 1 + 1e-9, `${recipe.id} sums to ${sum.toFixed(3)} at ${t.toFixed(4)} s`);
    }
  }
});

test("built-in sounds are found by id, and an unknown id is none", () => {
  assert.equal(sfxRecipe("pow")?.name, "Pow");
  assert.equal(sfxRecipe("nope"), null);
});

test("noise is the same for the same seed and different for another, inside [-1, 1)", () => {
  const a = seededNoise(7, 2048), b = seededNoise(7, 2048), c = seededNoise(8, 2048);
  assert.deepEqual(a, b);
  assert.notDeepEqual(a, c);
  assert.ok(a.every((v) => v >= -1 && v < 1));
  const mean = a.reduce((s, v) => s + v, 0) / a.length;
  assert.ok(Math.abs(mean) < 0.06, "centred on zero, so it adds no offset to the mix");
});
