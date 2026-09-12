import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { JAM_SAVED_PATCHES_MAX, PATCH_CUTOFF_MAX_HZ, type JamPatch } from "./jam-contract.ts";
import {
  EMPTY_STAGE_STASH, JAM_FILTER_ON, JAM_FILTER_OFF, JAM_ENV_OFF, JAM_SENDS_OFF,
  envOff, filterOff, keepSavedPatch, sendsOff, setFilterMode, toggleStage, uniqueSavedName,
  type JamSavedPatch,
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

function library(names: readonly string[]): JamSavedPatch[] {
  return names.map((name) => ({ name, patch: { ...base, e: { ...base.e, a: name.length } } }));
}

test("saving into a full library refuses rather than dropping somebody's oldest patch", () => {
  // The counterpart to the naming test above. That one proved a full library can still produce a
  // free name; this one proves that using it does not destroy the entry at the other end. The cap
  // used to be `.slice(-12)` on an append, so the thirteenth save silently evicted the first.
  const full = library(Array.from({ length: JAM_SAVED_PATCHES_MAX }, (_u, i) => `P${i}`));
  const name = uniqueSavedName(full.map((entry) => entry.name), "NEW");
  assert.ok(!full.some((entry) => entry.name === name));
  assert.equal(keepSavedPatch(full, name, base), null, "the thirteenth save is refused, not absorbed");
  assert.equal(full.length, JAM_SAVED_PATCHES_MAX, "and the caller's library is untouched");
  assert.equal(full[0].name, "P0", "including the entry a FIFO cap would have taken");
});

test("a full library still takes an overwrite, because typing a name that exists means replace", () => {
  const full = library(Array.from({ length: JAM_SAVED_PATCHES_MAX }, (_u, i) => `P${i}`));
  const replaced = keepSavedPatch(full, "P3", base);
  assert.ok(replaced, "naming an existing tile is a request to overwrite that one");
  assert.equal(replaced.length, JAM_SAVED_PATCHES_MAX, "an overwrite does not grow the library");
  assert.deepEqual(replaced.map((entry) => entry.name).sort(), full.map((entry) => entry.name).sort());
  assert.deepEqual(replaced.find((entry) => entry.name === "P3")?.patch, base);
  assert.deepEqual(replaced.find((entry) => entry.name === "P0")?.patch, full[0].patch,
    "and it replaces only the tile that was named");
});

test("a library below the cap keeps appending, and a fresh name never displaces one that is there", () => {
  let current: JamSavedPatch[] = [];
  for (let i = 0; i < JAM_SAVED_PATCHES_MAX; i += 1) {
    const next = keepSavedPatch(current, `P${i}`, base);
    assert.ok(next, `save ${i + 1} of ${JAM_SAVED_PATCHES_MAX} fits`);
    current = next;
    assert.equal(current.length, i + 1);
  }
  assert.equal(keepSavedPatch(current, "P0", base)?.length, JAM_SAVED_PATCHES_MAX, "replacing still fits");
  assert.equal(keepSavedPatch(current, "OVERFLOW", base), null);
});

function stageOffFor(patch: JamPatch, stage: "e" | "f" | "x"): boolean {
  return stage === "e" ? envOff(patch) : stage === "f" ? filterOff(patch) : sendsOff(patch);
}

const appSource = () => readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");

test("no editor control carries its own copy of a patch bound", () => {
  // jam-contract.ts says it is the single source of truth for these numbers, and it was not: the
  // knobs held their own 24, 50, 5000, 8000, 18000, 1200 and 100. Nothing was wrong while the
  // copies happened to agree, which is exactly the condition under which a later change to one of
  // them goes unnoticed. A control may now only spread its entry from the table.
  const source = appSource();
  const knobs = source.match(/\{@render jamKnob\(\{[^\n]*\}\)\}/g) ?? [];
  assert.ok(knobs.length >= 15, `expected the editor's knobs, found ${knobs.length}`);
  for (const knob of knobs) {
    assert.match(knob, /\.\.\.PATCH_PARAM\.[oefl x]\.[a-z]\b/,
      `a knob must take its range from the contract: ${knob.slice(0, 90)}`);
    assert.doesNotMatch(knob, /\bmin:\s*-?\d/, `a knob may not restate a floor: ${knob.slice(0, 90)}`);
    assert.doesNotMatch(knob, /\bmax:\s*-?\d/, `a knob may not restate a ceiling: ${knob.slice(0, 90)}`);
  }
});

test("no editor scope is drawn against its own copy of a patch ceiling", () => {
  // The scopes divide by the bound to normalise a value into the box, so each one held a ceiling
  // too. A scope with a stale ceiling does not throw: it quietly draws the wrong shape, which is
  // the failure this whole surface exists to prevent.
  const source = appSource();
  const start = source.indexOf("{#snippet jamScopeEnv(");
  const end = source.indexOf("{#snippet jamRecBanner()}", start);
  assert.ok(start >= 0 && end > start);
  const scopes = source.slice(start, end);
  // Only the values that can be nothing BUT a bound. Small integers are left alone because a scope
  // is full of honest drawing constants (box coordinates, bar offsets, stroke positions).
  for (const bound of ["5000", "8000", "18000", "1200", "900", "0.61"]) {
    assert.ok(!new RegExp(`\\b${bound.replace(".", "\\.")}\\b`).test(scopes),
      `the scopes still carry ${bound}, which is a patch bound and belongs in PATCH_PARAM`);
  }
  assert.ok((scopes.match(/PATCH_PARAM\./g) ?? []).length >= 10,
    "each scope should normalise against the contract's own ceilings");
});

test("the filter's neutral setting is not described to anyone as a bypass", () => {
  // `jam-patch:v1` cannot say "no filter", so the neutral setting is the top of the cutoff range
  // and every voice still runs through a lowpass at that corner. The button used to say OFF and
  // claim the tone passed through whole, which is a promise the renderer does not keep, and one
  // that gets less true the lower the output's sample rate is.
  const source = appSource();
  const button = source.split("\n").find((line) => line.includes('jamToggleStage("f")'));
  assert.ok(button, "the filter stage still has its neutral-setting button");
  assert.match(button, />WIDE</, "the label says what the setting is, not what it is not");
  assert.doesNotMatch(button, /passes through whole/,
    "no copy may claim a bypass the engine does not implement");
  assert.match(button, /not a bypass/, "and the tooltip says so plainly");
});

test("the share drawer is not keyed by a content address, which is not a listing id", () => {
  // The fileshare deliberately lists one set of bytes under several names and folders. Two people
  // building the same recipe produce byte-identical canonical patch JSON, so the collision is easy
  // to reach here. mediaChoices already learned this for the deck by collapsing repeats; the patch
  // drawer keeps both tiles, so it must not tell Svelte they are one row.
  const source = appSource();
  assert.match(source, /\{#each jamSharedPatches as sp\}/,
    "the shared-patch list must not be keyed by CID");
  assert.doesNotMatch(source, /\{#each jamSharedPatches as sp \(/);
});

test("the instrument row reports whether the recipe on screen is the one being played", () => {
  // The publication barrier is correct and stays: an announce per keystroke would exhaust a
  // receiver's patch budget. What was wrong is that the UI claimed the new patch was the sound for
  // up to the debounce plus the announce interval before it was. The badge is derived from the
  // published bytes rather than from a flag, so no failure path can leave it stuck on LIVE.
  const source = appSource();
  assert.match(source, /jamPublishedCanonical = JSON\.stringify\(captured\.descriptor\)/,
    "the published recipe's own bytes are what the badge compares against");
  assert.match(source, /const jamPatchApplying = \$derived\([\s\S]{0,200}jamDraftCanonical !== jamPublishedCanonical/);
  assert.match(source, /jamPublishedCanonical = null;/, "leaving a call publishes nothing");
  assert.match(source, />APPLYING</);
  assert.match(source, />LIVE</);
  // A discrete choice has no next keystroke to coalesce with, so it should not wait out a debounce
  // meant for a slider drag. The receiver-safe interval remains the authority on when it is sent.
  const select = source.indexOf("function selectJamPreset(");
  const dirty = source.indexOf("function jamPatchDirty(", select);
  assert.ok(select >= 0 && dirty > select);
  assert.match(source.slice(select, dirty), /jamPatchDirty\(true\)/,
    "picking a whole recipe asks for publication now");
});
