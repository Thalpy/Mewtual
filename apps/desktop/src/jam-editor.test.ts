import assert from "node:assert/strict";
import test from "node:test";
import { PATCH_CUTOFF_MAX_HZ, type JamPatch } from "./jam-contract.ts";
import {
  EMPTY_STAGE_STASH, JAM_FILTER_ON, JAM_FILTER_OFF, JAM_ENV_OFF, JAM_SENDS_OFF,
  envOff, filterOff, sendsOff, setFilterMode, toggleStage, uniqueSavedName,
} from "./jam-editor.ts";
import { validateJamPatch } from "./jam-patch.ts";

const base: JamPatch = {
  v: 1,
  o: [{ w: 3, t: 0, c: -8, l: 84 }],
  e: { a: 1_800, d: 2_500, s: 80, r: 4_000 },
  f: { m: 2, c: 777, q: 88, e: -23 },
  l: { r: 40, d: 30, t: 1 },
  x: { c: 55, d: 18, r: 72 },
};

test("every stage's off state is a legal patch and reads as off", () => {
  const off: JamPatch = { ...base, e: { ...JAM_ENV_OFF }, f: { ...JAM_FILTER_OFF }, x: { ...JAM_SENDS_OFF } };
  // The whole reason off is a value rather than a flag: it has to survive the one validator, so
  // that a bypassed patch can be announced, recorded and shared like any other.
  assert.equal(validateJamPatch(off).ok, true);
  assert.equal(envOff(off), true);
  assert.equal(filterOff(off), true);
  assert.equal(sendsOff(off), true);
  assert.equal(envOff(base), false);
  assert.equal(filterOff(base), false);
  assert.equal(sendsOff(base), false);
});

test("a stage switched off and back on returns the settings it had", () => {
  for (const stage of ["e", "f", "x"] as const) {
    const gone = toggleStage(base, EMPTY_STAGE_STASH, stage);
    assert.equal(stageOffFor(gone.patch, stage), true, `${stage} switched off`);
    assert.deepEqual(gone.stash[stage], base[stage], `${stage} stashed what it held`);
    const back = toggleStage(gone.patch, gone.stash, stage);
    assert.deepEqual(back.patch[stage], base[stage], `${stage} came back as itself`);
    assert.equal(back.stash[stage], null, "and released the stash it restored from");
  }
});

test("a stage with nothing stashed comes back as a usable default, not as off", () => {
  // The case that reaches this: a patch that arrived from the share already bypassed.
  const arrived: JamPatch = { ...base, f: { ...JAM_FILTER_OFF } };
  const back = toggleStage(arrived, EMPTY_STAGE_STASH, "f");
  assert.equal(filterOff(back.patch), false, "switching on must actually switch something on");
  assert.deepEqual(back.patch.f, { ...JAM_FILTER_ON });
});

test("picking a filter type out of OFF turns the filter back on", () => {
  // The defect: OFF is encoded with mode 0, so LP wrote the mode it already had and did nothing at
  // all, while HP and BP changed the mode but kept OFF's 18 kHz cutoff, which for a highpass
  // removes everything anyone is playing.
  const off = toggleStage(base, EMPTY_STAGE_STASH, "f");
  assert.equal(filterOff(off.patch), true);

  const lp = setFilterMode(off.patch, off.stash, 0);
  assert.equal(filterOff(lp.patch), false, "LP out of OFF must not be a no-op");
  // The stashed cutoff, resonance and envelope amount come back, under the type that was asked
  // for: the button says LP, so the bandpass it was switched off as does not come back with it.
  assert.deepEqual(lp.patch.f, { ...base.f, m: 0 });

  const hp = setFilterMode(off.patch, off.stash, 1);
  assert.equal(hp.patch.f.m, 1);
  assert.ok(hp.patch.f.c < PATCH_CUTOFF_MAX_HZ, "a highpass must not inherit the OFF cutoff");

  const bp = setFilterMode(off.patch, EMPTY_STAGE_STASH, 2);
  assert.deepEqual(bp.patch.f, { ...JAM_FILTER_ON, m: 2 }, "with no stash, a usable default");

  // An ordinary mode change on an active filter still only changes the mode.
  const plain = setFilterMode(base, EMPTY_STAGE_STASH, 1);
  assert.deepEqual(plain.patch.f, { ...base.f, m: 1 });
});

test("a stash never crosses from one recipe to another", () => {
  // The component resets the stash whenever a whole recipe is adopted. This is the state that
  // reset prevents: patch A's band-pass filter restored into patch B, then persisted and published
  // as the user's own sound.
  const offInA = toggleStage(base, EMPTY_STAGE_STASH, "f");
  const b: JamPatch = { ...base, f: { ...JAM_FILTER_OFF }, o: [{ w: 0, t: 0, c: 0, l: 100 }] };
  const leaked = toggleStage(b, offInA.stash, "f");
  assert.deepEqual(leaked.patch.f, base.f, "carrying the stash across is exactly the defect");
  const clean = toggleStage(b, EMPTY_STAGE_STASH, "f");
  assert.deepEqual(clean.patch.f, { ...JAM_FILTER_ON }, "reset at the selection boundary fixes it");
});

test("a library name is never taken from an existing entry by accident", () => {
  // Twelve characters is what a tile can show, so two shared files that differ only after the
  // twelfth arrive under one key. Keeping the second used to destroy the first.
  const first = uniqueSavedName([], "abcdefghijkl-1");
  assert.equal(first, "ABCDEFGHIJKL");
  const second = uniqueSavedName([first], "abcdefghijkl-2");
  assert.notEqual(second, first);
  assert.ok(second.length <= 12, "the disambiguated name still fits the tile");
  const third = uniqueSavedName([first, second], "abcdefghijkl-3");
  assert.ok(third !== first && third !== second);

  // The generated default must not depend on how full the library is: at capacity every unnamed
  // save used to be called PATCH 13 and replace the one before it.
  const full = Array.from({ length: 12 }, (_u, i) => (i === 0 ? "PATCH" : `PATCH ${i + 1}`));
  const next = uniqueSavedName(full, "PATCH");
  assert.ok(!full.includes(next), `a full library still yields a free name, got ${next}`);
  assert.equal(uniqueSavedName([], ""), "PATCH", "an empty base is still a name");
});

function stageOffFor(patch: JamPatch, stage: "e" | "f" | "x"): boolean {
  return stage === "e" ? envOff(patch) : stage === "f" ? filterOff(patch) : sendsOff(patch);
}
