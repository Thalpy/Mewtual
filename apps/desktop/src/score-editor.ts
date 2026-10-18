// Pure Score composer transitions. Native admission remains the authority; these guards keep
// ordinary clicks from knowingly creating an unexportable score and never trim retained work.
import { JAM_KIT, JAM_MET_BPB_MAX, JAM_MET_BPB_MIN, JAM_MET_BPM_MAX, JAM_MET_BPM_MIN } from "./jam-contract.ts";
import { SCORE_MAX_CELLS, SCORE_MAX_STEPS, SCORE_ROWS, type ScoreCell, type ScoreOp } from "./studio-contract.ts";
import type { IndexSource, ScoreModel, ScoreCellIssues, Valued } from "./studio-native.ts";
import { scoreStepMs } from "./studio-export.ts";

/** Steps per grid page on a narrow surface; `SCORE_WIDE_PAGE_STEPS` once the grid card is wide enough. */
export const SCORE_PAGE_STEPS = 16;
export const SCORE_WIDE_PAGE_STEPS = 32;
/** A card at least this wide (CSS px) shows two bars of 4/4 at once; below it, one. */
export const SCORE_WIDE_CARD_PX = 560;
export const SCORE_DIAGNOSTIC_PAGE = 24;
export const SCORE_ALTERNATIVE_PAGE = 8;
/** MIDI note of row zero: the grid is chromatic C4..C6 (2.8). */
export const SCORE_ROW_BASE_NOTE = 60;
/** Steps per beat: the grid is at sixteenth resolution and a beat is a quarter note (2.8). */
export const SCORE_STEPS_PER_BEAT = 4;
const PITCHES = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];
const integer = (n: number, min: number, max: number) => Number.isInteger(n) && n >= min && n <= max;

/** Row zero is MIDI 60 (C4), row 24 MIDI 84 (C6), without a scale-dependent remapping. */
export function scorePitch(row: number): string {
  if (!integer(row, 0, SCORE_ROWS - 1)) throw new Error("Pitch row is outside C4–C6.");
  return `${PITCHES[row % 12]}${4 + Math.floor(row / 12)}`;
}

/** Whether a row is a black key, for the grid's shading. */
export function scoreRowIsBlack(row: number): boolean {
  return PITCHES[row % 12].length > 1;
}

export function scorePage(steps: number, page: number, size = SCORE_PAGE_STEPS): number[] {
  if (!integer(size, 1, SCORE_MAX_STEPS) || !integer(steps, 1, SCORE_MAX_STEPS) || !integer(page, 0, Math.ceil(steps / size) - 1)) throw new Error("Score page is outside the authored length.");
  return Array.from({ length: Math.min(size, steps - page * size) }, (_, i) => page * size + i);
}

/**
 * The grid shows every step that holds a live cell, not only the authored length: a shortened
 * `steps` header leaves cells past the end that still exist, still count and still need clearing,
 * and hiding them would make the "past the end" flag unexplainable. Never below the authored
 * length, never above the global bound.
 */
export function scoreExtent(model: Pick<ScoreModel, "steps" | "cells">): number {
  let extent = Math.max(1, Math.min(SCORE_MAX_STEPS, model.steps));
  for (const [key, cell] of Object.entries(model.cells)) {
    const step = Number(key.split(":")[0]);
    const end = step + (cell.k === "n" ? (cell.len ?? 1) : 1);
    if (end > extent) extent = Math.min(SCORE_MAX_STEPS, end);
  }
  return extent;
}

/** Pad codes for the in-cell glyph, indexed like `JAM_KIT`; two letters fit a 16px cell. */
export const PAD_CODES: readonly string[] = ["KK", "SN", "RM", "CP", "HH", "OH", "LT", "HT", "RD", "CR"];
export const PAD_NAMES: readonly string[] = JAM_KIT.map((pad) => pad.name);

/** Issue flags as short labels, in the order the backend reports them. */
export const SCORE_ISSUE_LABELS: Readonly<Record<keyof ScoreCellIssues, string>> = {
  count: "over cap", extent: "past the end", recipe: "no recipe", overlap: "overlaps", conflict: "conflict",
};

export type ScoreEntry = { mode: "note"; patch: string; len: number } | { mode: "drum"; pad: number } | { mode: "clear" };

/** Clear stays available beyond a shrunken header, at capacity, and with missing recipes. */
export function scoreCellOperation(model: ScoreModel, step: number, row: number, entry: ScoreEntry): ScoreOp {
  if (!integer(step, 0, SCORE_MAX_STEPS - 1) || !integer(row, 0, SCORE_ROWS - 1)) throw new Error("Cell is outside the score coordinate bounds.");
  if (entry.mode === "clear") return { op: "clear_cell", step, row };
  if (step >= model.steps) throw new Error("Cell is beyond the authored score length. Clear it or extend the score first.");
  if (!model.cells[`${step}:${row}`] && Object.keys(model.cells).length >= SCORE_MAX_CELLS) throw new Error("The score already has 4096 active cells. Clear one before adding another.");
  if (entry.mode === "drum") {
    if (!integer(entry.pad, 0, 9)) throw new Error("Drum pad must be 0–9.");
    return { op: "set_cell", step, row, value: { k: "d", pad: entry.pad } };
  }
  if (!/^[0-9a-f]{64}$/.test(entry.patch) || !Object.hasOwn(model.patches, entry.patch)) throw new Error("Choose an active score recipe. A queued recipe is not available until its save is read back.");
  if (!integer(entry.len, 1, model.steps - step)) throw new Error("The authored hold must fit the remaining sixteenth steps. No note is shortened automatically.");
  for (const [key, cell] of Object.entries(model.cells)) {
    const [otherStep, otherRow] = key.split(":").map(Number);
    if (cell.k === "n" && otherRow === row && otherStep !== step && otherStep < step + entry.len && step < otherStep + (cell.len ?? 1)) {
      throw new Error("This hold overlaps another note at the same pitch. Clear or shorten that note first.");
    }
  }
  return { op: "set_cell", step, row, value: { k: "n", p: entry.patch, len: entry.len } };
}

export function scoreHeaderOperation(field: "title" | "bpm" | "bpb" | "steps", input: string): ScoreOp {
  if (field === "title") {
    // JSON can carry lone surrogates, but Studio canonical strings cannot. Do not silently repair
    // the author's title, and leave the complete-envelope byte budget to native admission.
    if (!input.isWellFormed()) throw new Error("The title contains invalid Unicode.");
    return { op: "set_header", field, value: input };
  }
  const n = input.trim() === "" ? NaN : Number(input);
  const [min, max] = field === "bpm" ? [JAM_MET_BPM_MIN, JAM_MET_BPM_MAX] : field === "bpb" ? [JAM_MET_BPB_MIN, JAM_MET_BPB_MAX] : [1, SCORE_MAX_STEPS];
  if (!integer(n, min, max)) throw new Error(`${field === "bpb" ? "Beats per bar" : field.toUpperCase()} must be a whole number from ${min} to ${max}.`);
  return { op: "set_header", field, value: n };
}

export function scoreCellLabel(value: ScoreCell | null): string {
  return value === null ? "cleared" : value.k === "d" ? `drum ${value.pad}` : `note · ${value.len ?? 1} sixteenth step${(value.len ?? 1) === 1 ? "" : "s"} · recipe ${value.p}`;
}

/** Continuations are display-only; they never create another onset or replace an authored hold. */
export function scoreContinuations(model: ScoreModel, page: number, size = SCORE_PAGE_STEPS): Set<string> {
  const columns = scorePage(model.steps, page, size);
  return new Set(scoreHolds(model, columns).keys());
}

export type ScoreHold = { onset: string; p: string; last: boolean; /** Later onsets whose holds also cover this coordinate. */ others: string[] };

/**
 * Ownership of a coordinate when holds overlap (retained concurrent edits can produce that, and
 * readiness flags it): the EARLIEST onset owns it. That is the note the renderer sounds, because
 * a same-pitch attack during a hold is a dropped event, so colour, label, selection, audition
 * and erase all follow the same note. Later covering onsets are listed, never hidden, so the
 * composer can offer them as an explicit choice.
 */
function coveringOnsets(model: Pick<ScoreModel, "cells">, step: number, row: number): string[] {
  const keys: string[] = [];
  for (const [key, cell] of Object.entries(model.cells)) if (cell.k === "n") {
    const [s, r] = key.split(":").map(Number);
    if (r === row && s < step && step < s + (cell.len ?? 1)) keys.push(key);
  }
  return keys.sort((a, b) => Number(a.split(":")[0]) - Number(b.split(":")[0]));
}

/**
 * Every continuation cell inside `columns`, mapped to the onset that owns it, the recipe it
 * sounds with and whether it is that hold's final step. Onsets off the page still project their
 * tail onto it, so a hold that crosses a page boundary keeps its colour and its rounded end.
 */
export function scoreHolds(model: Pick<ScoreModel, "cells">, columns: readonly number[]): Map<string, ScoreHold> {
  const result = new Map<string, ScoreHold>();
  if (!columns.length) return result;
  const start = columns[0], end = columns[columns.length - 1];
  // Earliest onsets first, so the first writer of a coordinate is its owner and later covering
  // onsets only join `others`, whatever order the map enumerates.
  const notes = Object.entries(model.cells).filter((entry): entry is [string, Extract<ScoreCell, { k: "n" }>] => entry[1].k === "n")
    .sort(([a], [b]) => Number(a.split(":")[0]) - Number(b.split(":")[0]));
  for (const [key, cell] of notes) {
    const [step, row] = key.split(":").map(Number), finish = step + (cell.len ?? 1) - 1;
    for (let s = Math.max(start, step + 1); s <= Math.min(end, finish); s++) {
      const at = `${s}:${row}`, existing = result.get(at);
      if (existing) existing.others.push(key);
      else if (!model.cells[at]) result.set(at, { onset: key, p: cell.p, last: s === finish, others: [] });
    }
  }
  return result;
}

/** The onset that owns a coordinate: the cell itself, else the earliest note whose hold covers it. */
export function scoreOnsetAt(model: Pick<ScoreModel, "cells">, step: number, row: number): { key: string; cell: ScoreCell } | null {
  const own = model.cells[`${step}:${row}`];
  if (own) return { key: `${step}:${row}`, cell: own };
  const first = coveringOnsets(model, step, row)[0];
  return first ? { key: first, cell: model.cells[first] } : null;
}

/** Every note sounding at a coordinate, owner first: the cell's own onset, then covering holds by onset. */
export function scoreNotesAt(model: Pick<ScoreModel, "cells">, step: number, row: number): { key: string; cell: ScoreCell }[] {
  const own = model.cells[`${step}:${row}`];
  const covering = coveringOnsets(model, step, row).map((key) => ({ key, cell: model.cells[key] }));
  return own ? [{ key: `${step}:${row}`, cell: own }, ...covering] : covering;
}

/** The note at `key` first, then every other note on its row whose hold overlaps its run, by onset. */
export function scoreOverlappingNotes(model: Pick<ScoreModel, "cells">, key: string): { key: string; cell: ScoreCell }[] {
  const own = model.cells[key];
  if (!own) return [];
  if (own.k !== "n") return [{ key, cell: own }];
  const [step, row] = key.split(":").map(Number), end = step + (own.len ?? 1);
  const others: { key: string; cell: ScoreCell }[] = [];
  for (const [other, cell] of Object.entries(model.cells)) if (other !== key && cell.k === "n") {
    const [s, r] = other.split(":").map(Number);
    if (r === row && s < end && step < s + (cell.len ?? 1)) others.push({ key: other, cell });
  }
  others.sort((a, b) => Number(a.key.split(":")[0]) - Number(b.key.split(":")[0]));
  return [{ key, cell: own }, ...others];
}

// --- placement assistance: snap, default hold, scale view, pad lane ----------------------------

/** Placement snap in sixteenth steps: sixteenth, eighth, quarter. The grid stays at sixteenths. */
export const SCORE_SNAPS: readonly number[] = [1, 2, 4];
/** A placed note starts as an eighth: long enough to hear, short enough to phrase. */
export const SCORE_DEFAULT_HOLD = 2;
/** The step a click at `step` places on under `snap`: never past the authored length. */
export function scoreSnapStart(step: number, snap: number, steps: number): number {
  const unit = SCORE_SNAPS.includes(snap) ? snap : 1;
  return Math.max(0, Math.min(steps - 1, Math.floor(step / unit) * unit));
}

/**
 * How many consecutive cells from `step` on `row` hold no note or continuation, at most `max`
 * and never past the authored length. Zero means the cell itself is taken. A new note is fitted
 * to this run so a click can never collide with a neighbour the preview did not show.
 */
export function scoreFreeRun(model: Pick<ScoreModel, "steps" | "cells">, step: number, row: number, max: number): number {
  const limit = Math.min(max, model.steps - step);
  if (limit <= 0) return 0;
  const taken = new Set<number>();
  for (const [key, cell] of Object.entries(model.cells)) {
    const [s, r] = key.split(":").map(Number);
    if (r !== row) continue;
    const len = cell.k === "n" ? (cell.len ?? 1) : 1;
    for (let i = Math.max(s, step); i < s + len && i < step + limit; i++) taken.add(i);
  }
  let run = 0;
  while (run < limit && !taken.has(step + run)) run++;
  return run;
}

export type ScoreScaleKind = "chromatic" | "major" | "minor" | "pentatonic";
export const SCORE_SCALE_KINDS: readonly ScoreScaleKind[] = ["chromatic", "major", "minor", "pentatonic"];
const SCALE_INTERVALS: Readonly<Record<ScoreScaleKind, readonly number[]>> = {
  chromatic: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11], major: [0, 2, 4, 5, 7, 9, 11], minor: [0, 2, 3, 5, 7, 8, 10], pentatonic: [0, 2, 4, 7, 9],
};
/** Root names for the scale picker, indexed by semitone above C. */
export const SCORE_ROOTS: readonly string[] = PITCHES;
/** Rows that belong to the scale. A view filter: notes outside it stay in the document and the grid. */
export function scoreScaleRows(kind: ScoreScaleKind, root: number): Set<number> {
  const intervals = SCALE_INTERVALS[kind] ?? SCALE_INTERVALS.chromatic;
  const rows = new Set<number>();
  for (let row = 0; row < SCORE_ROWS; row++) if (intervals.includes((((row - root) % 12) + 12) % 12)) rows.add(row);
  return rows;
}

/** In pads mode the bottom ten rows double as the pad lane: row r places pad r. */
export function scorePadForRow(row: number): number | null {
  return integer(row, 0, PAD_CODES.length - 1) ? row : null;
}

/**
 * Tempo from tap timestamps (ms): the mean gap of the last four taps, rounded, within the
 * metronome's bounds. Fewer than two taps, or a pause over two seconds, means no tempo yet.
 */
export function scoreTapTempo(taps: readonly number[]): number | null {
  const recent = taps.slice(-4);
  if (recent.length < 2) return null;
  let total = 0;
  for (let i = 1; i < recent.length; i++) { const gap = recent[i] - recent[i - 1]; if (!(gap > 0) || gap > 2000) return null; total += gap; }
  const bpm = Math.round(60000 / (total / (recent.length - 1)));
  return Math.max(JAM_MET_BPM_MIN, Math.min(JAM_MET_BPM_MAX, bpm));
}

/** Recipe ids in the stable order the rack lists them; the position doubles as the swatch colour. */
export function scorePatchOrder(model: Pick<ScoreModel, "patches">): string[] {
  return Object.keys(model.patches).sort();
}

/** How many live note onsets sound with each recipe (holds count once; pads never count). */
export function scorePatchUsage(model: Pick<ScoreModel, "cells">): Map<string, number> {
  const usage = new Map<string, number>();
  for (const cell of Object.values(model.cells)) if (cell.k === "n") usage.set(cell.p, (usage.get(cell.p) ?? 0) + 1);
  return usage;
}

/** Steps that hold a live onset or continuation, for the overview strip. */
export function scoreOccupiedSteps(model: Pick<ScoreModel, "cells">): Set<number> {
  const steps = new Set<number>();
  for (const [key, cell] of Object.entries(model.cells)) {
    const step = Number(key.split(":")[0]), len = cell.k === "n" ? (cell.len ?? 1) : 1;
    for (let s = step; s < step + len && s < SCORE_MAX_STEPS; s++) steps.add(s);
  }
  return steps;
}

/** The whole authored length in milliseconds under the export's timing rule (2.8). */
export function scoreDurationMs(steps: number, bpm: number): number {
  return scoreStepMs(steps, bpm);
}

/**
 * The step the playhead is on `positionMs` into playback. Playback loops the whole authored
 * length when `loop` is set, otherwise it parks on the last step once the end is reached; a
 * position before the start reads as step zero.
 */
export function scorePlayheadStep(positionMs: number, bpm: number, steps: number, loop: boolean): number {
  if (!Number.isFinite(positionMs) || steps < 1) return 0;
  const total = scoreDurationMs(steps, bpm);
  let ms = Math.max(0, positionMs);
  if (loop && total > 0) ms %= total;
  // Invert on(step) = floor(step × T / bpm): the first step whose onset is still at or before ms.
  const step = Math.floor((ms * bpm) / 15000);
  return Math.min(steps - 1, Math.max(0, step));
}

/** "m:ss.t" for transport readouts. */
export function scoreTimeLabel(ms: number): string {
  const total = Math.max(0, ms) / 1000, minutes = Math.floor(total / 60), seconds = total - minutes * 60;
  return `${minutes}:${seconds < 10 ? "0" : ""}${seconds.toFixed(1)}`;
}

/** Bar and beat (both 1-based) of a step under the authored meter. */
export function scoreBarBeat(step: number, bpb: number): { bar: number; beat: number } {
  const perBar = Math.max(1, bpb) * SCORE_STEPS_PER_BEAT;
  return { bar: Math.floor(step / perBar) + 1, beat: Math.floor((step % perBar) / SCORE_STEPS_PER_BEAT) + 1 };
}

/** The last step any live cell sounds on, or -1 for an empty score ("trim" uses this). */
export function scoreLastSoundingStep(model: Pick<ScoreModel, "cells">): number {
  let last = -1;
  for (const [key, cell] of Object.entries(model.cells)) {
    const step = Number(key.split(":")[0]), end = step + (cell.k === "n" ? (cell.len ?? 1) : 1) - 1;
    if (end > last) last = end;
  }
  return last;
}

export type ScoreDiagnostic = {
  id: string; kind: "header" | "cell" | "patch"; key: string; removed: boolean; issues: string[];
  alternatives: Valued<unknown, IndexSource>[];
};

/** All slots survive this view, including out-of-length cells, overflow and removal winners.
 * The component pages DOM rendering, not the evidence. Selected is labelled raw, not effective.
 */
export function scoreDiagnostics(model: ScoreModel): ScoreDiagnostic[] {
  const result: ScoreDiagnostic[] = [];
  for (const field of ["title", "bpm", "bpb", "steps"] as const) {
    const r = model.headers[field];
    if (r) result.push({ id: `header/${field}`, kind: "header", key: field, removed: false, issues: r.conflicts.length ? ["conflict"] : [], alternatives: [r.selected, ...r.conflicts] });
  }
  for (const [key, slot] of Object.entries(model.cellSlots).sort(([a], [b]) => {
    const [as, ar] = a.split(":").map(Number), [bs, br] = b.split(":").map(Number); return as - bs || ar - br;
  })) result.push({ id: `cell/${key}`, kind: "cell", key, removed: slot.removed,
    issues: Object.entries(model.readiness.cells[key] ?? {}).filter(([, v]) => v).map(([k]) => k), alternatives: [slot.register.selected, ...slot.register.conflicts] });
  for (const [key, slot] of Object.entries(model.patchSlots).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)) result.push({ id: `patch/${key}`, kind: "patch", key, removed: slot.removed,
    issues: Object.entries(model.readiness.patches[key] ?? {}).filter(([, v]) => v).map(([k]) => k), alternatives: [slot.register.selected, ...slot.register.conflicts] });
  return result;
}

/**
 * Only the slots a person has to act on: a readiness issue, a retained concurrent value, or a
 * removal that still carries the values it beat. A plain cell with one agreed value is the grid's
 * job, not a list entry. Order is the diagnostics order (headers, then cells by (step, row), then
 * recipes by id) so a page of this list is stable while the document changes elsewhere.
 */
export function scoreAttention(model: ScoreModel): ScoreDiagnostic[] {
  return scoreDiagnostics(model).filter((item) => item.issues.length > 0 || item.alternatives.length > 1 || (item.removed && item.alternatives.some((v) => v.value !== null)));
}

/** One slot's diagnostic, for the inspector's selected-cell panel; null when nothing is retained there. */
export function scoreSlotDiagnostic(model: ScoreModel, kind: ScoreDiagnostic["kind"], key: string): ScoreDiagnostic | null {
  return scoreDiagnostics(model).find((item) => item.kind === kind && item.key === key) ?? null;
}

/** An intentional choice reauthors an ordinary operation; old source ids never become authority. */
export function scoreDiagnosticOperation(model: ScoreModel, item: ScoreDiagnostic, index: number): ScoreOp {
  const chosen = item.alternatives[index];
  if (!chosen) throw new Error("That retained alternative is no longer available.");
  if (item.kind === "header") return scoreHeaderOperation(item.key as "title" | "bpm" | "bpb" | "steps", String(chosen.value));
  if (item.kind === "patch") return chosen.value === null ? { op: "remove_patch", patch: item.key } : { op: "set_patch", patch: item.key, descriptor: chosen.value };
  const [step, row] = item.key.split(":").map(Number), value = chosen.value as ScoreCell | null;
  return scoreCellOperation(model, step, row, value === null ? { mode: "clear" } : value.k === "d" ? { mode: "drum", pad: value.pad } : { mode: "note", patch: value.p, len: value.len ?? 1 });
}
