// What a Score plays, and what is worth reviewing about it (docs/design-score-layers.md section 3).
//
// One definition for the whole renderer: the working projection's readiness, both exporters and the
// composer's marks all come from here, and Rust's `ScoreProjection::readiness` computes the same
// flags (a shared fixture, studio-score-readiness-v1.json, pins the two). Native sends readiness
// with every view and the editor fingerprints it, so any drift between the two would make a
// readback of the very score on screen look like a source change and stop its playback.
//
// Merging is additive: identical notes collapse to one voice, distinct notes in one layer slot all
// play, overlapping holds all play. Nothing here refuses a score. A voice that cannot play (past the
// end, over the cap, its sound gone) is skipped and counted; a note running past the end is clipped.
//
// Work is O(n log n) in the evidence the view carries (the sort), never quadratic: one member can
// stack thousands of concurrent values on one key, and native computes this on every view.

import { SCORE_MAX_CELLS, SCORE_MAX_PATCHES, canonicalJson, layerOf, type ScoreCell, type ScoreLayer } from "./studio-contract.ts";

/** The diagnostic flags of one slot (a legacy cell `"S:R"` or a layered note `"S:R/L"`). */
export type ScoreVoiceIssues = { count: boolean; extent: boolean; recipe: boolean; overlap: boolean; conflict: boolean };

/**
 * One slot's raw evidence: its register's alternatives, selected first, `null` for a clear. The two
 * collections merge differently and only this module decides how:
 * - `layer: null`, a legacy cell `"S:R"`: removal wins (any clear removes it) and only the selected
 *   value plays. Legacy compaction keeps a bounded number of alternatives, so playing more than the
 *   winner would change what a score sounds like at its next checkpoint.
 * - a layer, a note `"S:R/L"`: add wins (clears are ignored while any value is live) and every
 *   distinct live value plays.
 */
export type ScoreVoiceSlot = { key: string; step: number; row: number; layer: ScoreLayer | null; values: readonly (ScoreCell | null)[] };

/** A note that plays, as the exporter takes it. */
export type ScoreNoteVoice = { step: number; row: number; value: ScoreCell };

export type ScoreVoices = {
  /** Every distinct voice in canonical order (effective values), with the slots it came from. */
  all: { step: number; row: number; value: ScoreCell; keys: string[] }[];
  /** What plays: skipped voices removed. Canonical order, distinct, effective values. */
  play: ScoreNoteVoice[];
  /** Voices that cannot play (past the end, over the cap, sound gone). */
  skipped: number;
  /** Sparse: only slots with something to review, exactly as native sends readiness. */
  issues: Record<string, ScoreVoiceIssues>;
};

/** The sounds a note may use: active recipes, the first 64 by id (native's admission order). */
export function admittedSounds(activeIds: Iterable<string>): Set<string> {
  return new Set([...activeIds].sort().slice(0, SCORE_MAX_PATCHES));
}

/** Distinct values, in the order given; identical notes are one note. */
export function distinctCells(values: Iterable<ScoreCell | null>): ScoreCell[] {
  const seen = new Map<string, ScoreCell>();
  for (const value of values) if (value !== null) { const key = canonicalJson(value); if (!seen.has(key)) seen.set(key, value); }
  return [...seen.values()];
}

const holdEnd = (step: number, value: ScoreCell) => step + (value.k === "n" ? value.len ?? 1 : 1);

/**
 * The value a voice plays and is identified by. A note that starts inside the grid is clipped to
 * its end; `len` is written only above 1, because an absent `len` and `len: 1` are the same note to
 * the exporter (which would otherwise see two identical voices and refuse). `vol` stays as authored:
 * an absent level is not a level of 100 to the take format. A drum is itself.
 */
export function effectiveCell(step: number, value: ScoreCell, steps: number): ScoreCell {
  if (value.k === "d") return value;
  let len = value.len ?? 1;
  if (step < steps) len = Math.min(len, steps - step);
  return { k: "n", p: value.p, ...(len > 1 ? { len } : {}), ...(value.vol === undefined ? {} : { vol: value.vol }) };
}

type Voice = { step: number; row: number; value: ScoreCell; bytes: string; keys: string[] };

export function scoreVoices(slots: readonly ScoreVoiceSlot[], steps: number, admitted: ReadonlySet<string>): ScoreVoices {
  const flags = new Map<string, ScoreVoiceIssues>();
  const flag = (keys: readonly string[], field: keyof ScoreVoiceIssues) => {
    for (const key of keys) {
      const entry = flags.get(key) ?? { count: false, extent: false, recipe: false, overlap: false, conflict: false };
      entry[field] = true; flags.set(key, entry);
    }
  };
  // 1. Candidates per slot, by its collection's rule. `conflict` is about the slot's raw values
  // (more than one distinct value retained), so it does not move when `steps` does.
  // 2. Effective values, with identical voices collapsed across every slot (a legacy cell and a
  // layered note holding the same note are one voice; so are two lengths clipped to one).
  const byIdentity = new Map<string, Voice>();
  for (const slot of slots) {
    let candidates: ScoreCell[];
    if (slot.layer === null) {
      const selected = slot.values[0];
      if (selected === undefined || selected === null || slot.values.includes(null)) continue; // removal wins
      candidates = [selected];
      if (distinctCells(slot.values).length > 1) flag([slot.key], "conflict");
    } else {
      candidates = distinctCells(slot.values);
      if (candidates.length > 1) flag([slot.key], "conflict");
    }
    for (const raw of candidates) {
      if (slot.step >= steps || holdEnd(slot.step, raw) > steps) flag([slot.key], "extent");
      const value = effectiveCell(slot.step, raw, steps), bytes = canonicalJson(value), identity = `${slot.step}:${slot.row}:${bytes}`;
      const voice = byIdentity.get(identity);
      if (!voice) byIdentity.set(identity, { step: slot.step, row: slot.row, value, bytes, keys: [slot.key] });
      else if (!voice.keys.includes(slot.key)) voice.keys.push(slot.key); // at most a legacy and a layer key
    }
  }
  // 3. Canonical order: (step, row, canonical bytes of the effective value). Canonical JSON here is
  // ASCII, so code unit order is byte order, as Rust compares.
  const all = [...byIdentity.values()].sort((a, b) => a.step - b.step || a.row - b.row || (a.bytes < b.bytes ? -1 : a.bytes > b.bytes ? 1 : 0));
  // Overlap: holds on the same row AND in the same layer (different sounds overlapping is what
  // layers are for), over every note voice, played or skipped, with its effective end. Within one
  // (row, layer) the voices are already in start order, so a voice overlaps another exactly when it
  // starts before the furthest end reached so far, or the next voice starts before it ends.
  const lanes = new Map<string, Voice[]>();
  for (const voice of all) {
    if (voice.value.k !== "n") continue;
    const lane = `${voice.row}/${layerOf(voice.value)}`;
    const list = lanes.get(lane);
    if (list) list.push(voice); else lanes.set(lane, [voice]);
  }
  for (const list of lanes.values()) {
    let reach = -1;
    list.forEach((voice, i) => {
      const end = holdEnd(voice.step, voice.value), next = list[i + 1];
      if (voice.step < reach || (next !== undefined && next.step < end)) flag(voice.keys, "overlap");
      reach = Math.max(reach, end);
    });
  }
  // 4. Skip what cannot play: over the cap (in canonical order), a sound not admitted, a start at
  // or after the end. Everything else plays, already clipped.
  const play: ScoreNoteVoice[] = [];
  let skipped = 0;
  all.forEach((voice, index) => {
    const over = index >= SCORE_MAX_CELLS, recipe = voice.value.k === "n" && !admitted.has(voice.value.p);
    if (over) flag(voice.keys, "count");
    if (recipe) flag(voice.keys, "recipe");
    if (over || recipe || voice.step >= steps) { skipped++; return; }
    play.push({ step: voice.step, row: voice.row, value: voice.value });
  });
  return { all: all.map(({ step, row, value, keys }) => ({ step, row, value, keys })), play, skipped,
    issues: Object.fromEntries([...flags].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) };
}
