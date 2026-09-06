// jam-editor.ts: the patch editor's state transitions, as pure functions.
//
// These used to live inline in App.svelte, where they had no way to be tested and three separate
// state defects hid in them at once: an LP button that could not turn a filter back on, a bypass
// stash that restored one patch's settings into another, and a library keyed by a truncated
// display label that silently destroyed the recipe it collided with. None of those need a webview
// to reproduce; all of them need one to find, which is the wrong way round.
//
// The component keeps the bindings, the audio and the announce pacing. Everything here is a
// function from (patch, editor state) to the next (patch, editor state), so a test can assert what
// a button does without mounting anything.

import { PATCH_CUTOFF_MAX_HZ, type JamPatch } from "./jam-contract.ts";

/** Which shaping stages carry a bypass. The oscillator stack is not one: it IS the sound. */
export type JamStage = "e" | "f" | "x";

/**
 * What each stage held when it was switched off, so switching it back on returns the sound rather
 * than a default. Editor state, never patch state: it is not announced, recorded or saved.
 */
export type JamStageStash = {
  e: JamPatch["e"] | null;
  f: JamPatch["f"] | null;
  x: JamPatch["x"] | null;
};

export const EMPTY_STAGE_STASH: JamStageStash = { e: null, f: null, x: null };

// "Off" is a VALUE, never a flag. `jam-patch:v1` admits exactly six keys and rejects a patch
// carrying more, so an `enabled` bit would be a different format that every other build refuses
// outright. Each stage already has a setting that does nothing, so OFF writes that, and the lamp
// is derived from the values rather than stored beside them: a patch loaded from the share lights
// the right lamps with no extra state to keep in step, and nudging any knob turns its stage back
// on by itself, because the values simply stop being neutral.
export const JAM_ENV_OFF = { a: 0, d: 0, s: 100, r: 0 } as const; // a gate: full while held
export const JAM_FILTER_OFF = { m: 0, c: PATCH_CUTOFF_MAX_HZ, q: 0, e: 0 } as const; // wide open
export const JAM_SENDS_OFF = { c: 0, d: 0, r: 0 } as const; // nothing reaches the room

// What a stage comes back as when it is switched on with nothing of its own to restore, which is
// the case for a patch that arrived from the share already bypassed.
export const JAM_ENV_ON = { a: 12, d: 380, s: 45, r: 120 } as const;
export const JAM_FILTER_ON = { m: 0, c: 2_400, q: 20, e: 0 } as const;
export const JAM_SENDS_ON = { c: 0, d: 20, r: 30 } as const;

export function envOff(patch: JamPatch): boolean {
  return patch.e.a === 0 && patch.e.d === 0 && patch.e.s === 100 && patch.e.r === 0;
}
export function filterOff(patch: JamPatch): boolean {
  return patch.f.m === 0 && patch.f.c >= PATCH_CUTOFF_MAX_HZ && patch.f.q === 0 && patch.f.e === 0;
}
export function sendsOff(patch: JamPatch): boolean {
  return patch.x.c === 0 && patch.x.d === 0 && patch.x.r === 0;
}
export function stageOff(patch: JamPatch, stage: JamStage): boolean {
  return stage === "e" ? envOff(patch) : stage === "f" ? filterOff(patch) : sendsOff(patch);
}

export type JamEditorStep = { patch: JamPatch; stash: JamStageStash };

function clone(patch: JamPatch): JamPatch {
  return JSON.parse(JSON.stringify(patch)) as JamPatch;
}

/** Switch a stage off (stashing what it held) or back on (restoring it). */
export function toggleStage(patch: JamPatch, stash: JamStageStash, stage: JamStage): JamEditorStep {
  const next = clone(patch);
  const off = stageOff(patch, stage);
  if (stage === "e") {
    if (off) return { patch: { ...next, e: { ...(stash.e ?? JAM_ENV_ON) } }, stash: { ...stash, e: null } };
    return { patch: { ...next, e: { ...JAM_ENV_OFF } }, stash: { ...stash, e: { ...patch.e } } };
  }
  if (stage === "f") {
    if (off) return { patch: { ...next, f: { ...(stash.f ?? JAM_FILTER_ON) } }, stash: { ...stash, f: null } };
    return { patch: { ...next, f: { ...JAM_FILTER_OFF } }, stash: { ...stash, f: { ...patch.f } } };
  }
  if (off) return { patch: { ...next, x: { ...(stash.x ?? JAM_SENDS_ON) } }, stash: { ...stash, x: null } };
  return { patch: { ...next, x: { ...JAM_SENDS_OFF } }, stash: { ...stash, x: { ...patch.x } } };
}

/**
 * Choose a filter type, including out of OFF.
 *
 * Writing the mode and nothing else made two of the three buttons wrong from the neutral state.
 * LP was a plain no-op, because OFF already IS mode 0, so the one button meaning "an ordinary
 * lowpass" could not turn the filter back on. HP and BP did change the mode but inherited the OFF
 * cutoff of 18 kHz, which for a highpass removes everything anyone was playing. Picking an active
 * type is therefore one transition: bring the filter back, then set the type that was asked for.
 */
export function setFilterMode(patch: JamPatch, stash: JamStageStash, mode: number): JamEditorStep {
  const next = clone(patch);
  if (!filterOff(patch)) {
    next.f.m = mode;
    return { patch: next, stash };
  }
  return { patch: { ...next, f: { ...(stash.f ?? JAM_FILTER_ON), m: mode } }, stash: { ...stash, f: null } };
}

/**
 * A library name that is not already taken, from a base that may be.
 *
 * The saved-patch library is keyed by its DISPLAY label, capped at the twelve characters a tile
 * can show. Two shared files whose names differ only after the twelfth character therefore arrive
 * under one key, and keeping the second destroyed the first: a decision about how wide a button is
 * became a decision about whose recipe survives. Imports take the next free label instead. Saving
 * under a name the user typed still replaces, because that is what typing an existing name means.
 */
export function uniqueSavedName(taken: readonly string[], base: string, limit = 12): string {
  const stem = base.slice(0, limit).toUpperCase() || "PATCH";
  if (!taken.includes(stem)) return stem;
  for (let n = 2; n <= 99; n += 1) {
    const suffix = ` ${n}`;
    const candidate = `${stem.slice(0, limit - suffix.length).trimEnd()}${suffix}`;
    if (!taken.includes(candidate)) return candidate;
  }
  return stem; // ninety-nine collisions on one stem: replacing is the honest outcome
}
