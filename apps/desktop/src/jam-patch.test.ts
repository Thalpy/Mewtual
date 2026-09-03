import assert from "node:assert/strict";
import test from "node:test";
import { JAM_PATCH_ID_HEX_CHARS, type JamPatch } from "./jam-contract.ts";
import { jamPatchId, legacyJamPatch, parseJamPatchJson, validateJamPatch, verifyJamPatchId } from "./jam-patch.ts";

export const TEST_PATCH: JamPatch = {
  v: 1,
  o: [
    { w: 3, t: 0, c: -12, l: 34 },
    { w: 3, t: 0, c: 0, l: 33 },
    { w: 3, t: 0, c: 12, l: 33 },
  ],
  e: { a: 850, d: 1_200, s: 72, r: 6_000 },
  f: { m: 0, c: 1_800, q: 28, e: 45 },
  l: { r: 18, d: 12, t: 1 },
  x: { c: 26, d: 14, r: 38 },
};

test("patch validation canonicalizes declaration order and produces a full SHA-256 id", async () => {
  const shuffled = {
    x: { r: 38, c: 26, d: 14 },
    l: { t: 1, d: 12, r: 18 },
    f: { e: 45, q: 28, c: 1_800, m: 0 },
    e: { r: 6_000, s: 72, d: 1_200, a: 850 },
    o: TEST_PATCH.o.map((osc) => ({ l: osc.l, c: osc.c, t: osc.t, w: osc.w })),
    v: 1,
  };
  const checked = validateJamPatch(shuffled);
  assert.equal(checked.ok, true);
  if (!checked.ok) return;
  assert.equal(checked.canonical, JSON.stringify(TEST_PATCH));
  const id = await jamPatchId(checked.patch);
  assert.match(id, new RegExp(`^[0-9a-f]{${JAM_PATCH_ID_HEX_CHARS}}$`));
  assert.equal(id.length, 64, "content identity must not regress to the old 64-bit truncation");
  assert.equal(await verifyJamPatchId(id, checked.patch), true);
  assert.equal(await verifyJamPatchId(id.slice(0, 16), checked.patch), false);
});

test("all patch sources share strict unknown-field, integer and range validation", () => {
  assert.equal(validateJamPatch({ ...TEST_PATCH, script: "osc.connect(destination)" }).ok, false);
  assert.equal(validateJamPatch({ ...TEST_PATCH, o: [...TEST_PATCH.o, TEST_PATCH.o[0]] }).ok, false);
  assert.equal(validateJamPatch({ ...TEST_PATCH, e: { ...TEST_PATCH.e, r: 8_001 } }).ok, false);
  assert.equal(validateJamPatch({ ...TEST_PATCH, f: { ...TEST_PATCH.f, c: Number.NaN } }).ok, false);
  assert.equal(validateJamPatch({ ...TEST_PATCH, l: { ...TEST_PATCH.l, r: 1.5 } }).ok, false);
  assert.equal(validateJamPatch({ ...TEST_PATCH, x: { ...TEST_PATCH.x, r: 101 } }).ok, false);
  assert.equal(parseJamPatchJson("not json").ok, false);
});

test("non-plain and prototype-bearing patch records fail closed", () => {
  class PatchClass { v = 1; o = TEST_PATCH.o; e = TEST_PATCH.e; f = TEST_PATCH.f; l = TEST_PATCH.l; x = TEST_PATCH.x; }
  assert.equal(validateJamPatch(new PatchClass()).ok, false);
  const polluted = JSON.parse(JSON.stringify(TEST_PATCH)) as Record<string, unknown>;
  polluted.__proto__ = { gain: 10_000 };
  assert.equal(validateJamPatch(polluted).ok, false);
  let getterReads = 0;
  const accessor = { ...TEST_PATCH } as Record<string, unknown>;
  Object.defineProperty(accessor, "x", { enumerable: true, get: () => { getterReads += 1; return TEST_PATCH.x; } });
  assert.equal(validateJamPatch(accessor).ok, false);
  assert.equal(getterReads, 0, "validation must reject accessors without executing them");
});

test("legacy fallback remains a valid single-oscillator recipe", () => {
  for (const wave of ["sine", "triangle", "square", "sawtooth"] as const) {
    const patch = legacyJamPatch(wave);
    const checked = validateJamPatch(patch);
    assert.equal(checked.ok, true);
    assert.equal(patch.o.length, 1);
  }
});
