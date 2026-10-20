// Pure Score composer transitions. Native admission remains the authority; these guards keep
// ordinary clicks from knowingly creating an unexportable score and never trim retained work.
import { JAM_KIT, JAM_MET_BPB_MAX, JAM_MET_BPB_MIN, JAM_MET_BPM_MAX, JAM_MET_BPM_MIN } from "./jam-contract.ts";
import { SCORE_DRUM_LAYER, SCORE_MAX_CELLS, SCORE_MAX_STEPS, SCORE_ROWS, SCORE_VOLUME_MAX, SCORE_VOLUME_MIN, canonicalJson, layerOf, noteKey, parseNoteKey, type ScoreCell, type ScoreOp } from "./studio-contract.ts";
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
  count: "over cap", extent: "past the end", recipe: "no sound", overlap: "overlaps", conflict: "conflict",
};

/** A note entry's `volume` is 1..100; omitted or 100 writes no `vol`, the format's full level. */
export type ScoreEntry = { mode: "note"; patch: string; len: number; volume?: number } | { mode: "drum"; pad: number } | { mode: "clear" };

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
  if (!/^[0-9a-f]{64}$/.test(entry.patch) || !Object.hasOwn(model.patches, entry.patch)) throw new Error("Choose a sound this score holds. A sound still being saved is not available until it is read back.");
  if (!integer(entry.len, 1, model.steps - step)) throw new Error("The authored hold must fit the remaining sixteenth steps. No note is shortened automatically.");
  if (entry.volume !== undefined && !integer(entry.volume, SCORE_VOLUME_MIN, SCORE_VOLUME_MAX)) throw new Error(`Volume is ${SCORE_VOLUME_MIN} to ${SCORE_VOLUME_MAX}.`);
  for (const [key, cell] of Object.entries(model.cells)) {
    const [otherStep, otherRow] = key.split(":").map(Number);
    if (cell.k === "n" && otherRow === row && otherStep !== step && otherStep < step + entry.len && step < otherStep + (cell.len ?? 1)) {
      throw new Error("This hold overlaps another note at the same pitch. Clear or shorten that note first.");
    }
  }
  // Full level is the format's absent value: the editor authors the smallest body that means
  // what was chosen, so a note at 100 stays byte-identical to one placed before levels existed.
  const vol = entry.volume === undefined || entry.volume === SCORE_VOLUME_MAX ? {} : { vol: entry.volume };
  return { op: "set_cell", step, row, value: { k: "n", p: entry.patch, len: entry.len, ...vol } };
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
  return value === null ? "cleared" : value.k === "d" ? `drum ${value.pad}` : `note · ${value.len ?? 1} sixteenth step${(value.len ?? 1) === 1 ? "" : "s"} · sound ${value.p}`;
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
/** Intervals a phrase (or one note) transposes by in one step, in semitones. */
export const SCORE_INTERVALS: readonly { semitones: number; label: string }[] = [
  { semitones: 1, label: "semitone" }, { semitones: 2, label: "tone" }, { semitones: 3, label: "minor 3rd" }, { semitones: 4, label: "major 3rd" },
  { semitones: 5, label: "4th" }, { semitones: 7, label: "5th" }, { semitones: 12, label: "octave" },
];
export const SCORE_DEFAULT_INTERVAL = 12;
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

/**
 * One retained slot, for the attention list and the selected panel. `kind` is the collection:
 * `cell` a legacy one-note-per-cell slot keyed `"S:R"` (removal wins), `note` a layered note keyed
 * `"S:R/L"` (add wins: every live value plays, `removed` means cleared), `patch` a sound, `header`
 * a score header.
 */
export type ScoreDiagnostic = {
  id: string; kind: "header" | "cell" | "note" | "patch"; key: string; removed: boolean; issues: string[];
  alternatives: Valued<unknown, IndexSource>[];
};

/** A header's alternatives whose value differs from the one used. Identical concurrent values are
 *  one value, as readiness (and native) count a header conflict. */
export function scoreHeaderOthers<T>(register: { selected: Valued<T, IndexSource>; conflicts: Valued<T, IndexSource>[] } | null | undefined): Valued<T, IndexSource>[] {
  if (!register) return [];
  const used = canonicalJson(register.selected.value);
  return register.conflicts.filter((alternative) => canonicalJson(alternative.value) !== used);
}

const byNoteKey = (a: string, b: string) => {
  const x = parseNoteKey(a), y = parseNoteKey(b);
  if (!x || !y) return a < b ? -1 : a > b ? 1 : 0;
  return x.step - y.step || x.row - y.row || (a < b ? -1 : a > b ? 1 : 0);
};
const flagsOf = (issues: Readonly<Record<string, boolean>> | undefined) => Object.entries(issues ?? {}).filter(([, v]) => v).map(([k]) => k);

/** All slots survive this view, including out-of-length cells, overflow and removal winners.
 * The component pages DOM rendering, not the evidence. Selected is labelled raw, not effective.
 */
export function scoreDiagnostics(model: ScoreModel): ScoreDiagnostic[] {
  const result: ScoreDiagnostic[] = [];
  for (const field of ["title", "bpm", "bpb", "steps"] as const) {
    const r = model.headers[field];
    if (r) result.push({ id: `header/${field}`, kind: "header", key: field, removed: false, issues: scoreHeaderOthers<unknown>(r).length ? ["conflict"] : [], alternatives: [r.selected, ...r.conflicts] });
  }
  for (const [key, slot] of Object.entries(model.cellSlots).sort(([a], [b]) => byNoteKey(a, b))) result.push({ id: `cell/${key}`, kind: "cell", key, removed: slot.removed,
    issues: flagsOf(model.readiness.cells[key]), alternatives: [slot.register.selected, ...slot.register.conflicts] });
  for (const [key, slot] of Object.entries(model.layerSlots ?? {}).sort(([a], [b]) => byNoteKey(a, b))) result.push({ id: `note/${key}`, kind: "note", key, removed: slot.cleared,
    issues: flagsOf(model.readiness.layers?.[key]), alternatives: [slot.register.selected, ...slot.register.conflicts] });
  for (const [key, slot] of Object.entries(model.patchSlots).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)) result.push({ id: `patch/${key}`, kind: "patch", key, removed: slot.removed,
    issues: flagsOf(model.readiness.patches[key]), alternatives: [slot.register.selected, ...slot.register.conflicts] });
  return result;
}

/**
 * Only the slots a person has to act on: a readiness issue, a retained concurrent value, or a
 * removal that still carries the values it beat. A plain cell with one agreed value is the grid's
 * job, not a list entry. Order is the diagnostics order (headers, then cells by (step, row), then
 * layered notes, then recipes by id) so a page of this list is stable while the document changes
 * elsewhere. A layered note is listed only for an issue: its several values all play (its
 * `conflict` flag says so), and a clear that lost to an add removed nothing.
 */
export function scoreAttention(model: ScoreModel): ScoreDiagnostic[] {
  // Several alternatives holding the same value are one value: nothing to choose between.
  const distinct = (item: ScoreDiagnostic) => new Set(item.alternatives.map((v) => canonicalJson(v.value))).size > 1;
  return scoreDiagnostics(model).filter((item) => item.issues.length > 0 ||
    (item.kind !== "note" && (distinct(item) || (item.removed && item.alternatives.some((v) => v.value !== null)))));
}

/** One slot's diagnostic, for the inspector's selected-cell panel; null when nothing is retained there. */
export function scoreSlotDiagnostic(model: ScoreModel, kind: ScoreDiagnostic["kind"], key: string): ScoreDiagnostic | null {
  return scoreDiagnostics(model).find((item) => item.kind === kind && item.key === key) ?? null;
}

// --- gestures: local projection, inverses, moves, repeats ------------------------------------------
// The composer's multi-step gestures (a move, a repeated bar) and its undo stack are built from
// the same single-cell operations the document already accepts, so nothing here needs a new wire
// grammar. A gesture is a list of operations; its inverse is computed against the projection as
// it was before each operation, so undoing a gesture replays the previous values in reverse. The
// native side applies a gesture's operations as one batch (one transaction) once the autosave
// releases them, so the intermediate state is never shown; the batch is not atomic, and a refused
// item leaves the earlier ones saved with the retained values reachable from the attention list.

/** `model` with `op` applied the way the session's working projection applies it (cells, notes and
 * patches copied). `notes` holds every note by its note key, legacy and layered alike. */
export function scoreApplyLocal(model: ScoreModel, op: ScoreOp): ScoreModel {
  const next = { ...model, cells: { ...model.cells }, notes: { ...(model.notes ?? model.cells) }, patches: { ...model.patches } };
  switch (op.op) {
    case "set_cell": next.cells[`${op.step}:${op.row}`] = next.notes[`${op.step}:${op.row}`] = op.value; break;
    case "clear_cell": delete next.cells[`${op.step}:${op.row}`]; delete next.notes[`${op.step}:${op.row}`]; break;
    case "set_note": next.notes[noteKey(op.step, op.row, layerOf(op.value))] = op.value; break;
    case "clear_note": delete next.notes[noteKey(op.step, op.row, op.layer)]; break;
    case "set_patch": next.patches[op.patch] = op.descriptor as ScoreModel["patches"][string]; break;
    case "remove_patch": delete next.patches[op.patch]; break;
    case "set_header":
      if (op.field === "title") next.title = String(op.value);
      else next[op.field] = Number(op.value);
      break;
  }
  return next;
}

/**
 * The operation that puts back what `op` replaces in `model`, or null when `op` changes nothing
 * recoverable (clearing an empty cell, removing an absent recipe). A previous value is restored
 * as it was, level and hold included; a cell that was empty is cleared again.
 *
 * Undo restores exactly where the value lived: a legacy cell the composer cleared (moving its note
 * into a layer, say) comes back as that legacy cell, never as a layered note. Restoring it into
 * its layer instead would collide with the layered note the same gesture wrote there, and undoing
 * an in-place edit of a legacy note would then clear both.
 */
export function scoreInverse(model: ScoreModel, op: ScoreOp): ScoreOp | null {
  switch (op.op) {
    case "set_cell": case "clear_cell": {
      const prev = model.cells[`${op.step}:${op.row}`];
      if (prev) return { op: "set_cell", step: op.step, row: op.row, value: { ...prev } };
      return op.op === "set_cell" ? { op: "clear_cell", step: op.step, row: op.row } : null;
    }
    case "set_note": case "clear_note": {
      // A layered note: its layer is the sound it plays (or the drum kit), so the inverse names the
      // same note key the operation addressed.
      const layer = op.op === "set_note" ? layerOf(op.value) : op.layer;
      const prev = (model.notes ?? {})[noteKey(op.step, op.row, layer)];
      if (prev) return { op: "set_note", step: op.step, row: op.row, value: { ...prev } };
      return op.op === "set_note" ? { op: "clear_note", step: op.step, row: op.row, layer } : null;
    }
    case "set_patch": {
      const prev = model.patches[op.patch];
      return prev ? { op: "set_patch", patch: op.patch, descriptor: prev } : { op: "remove_patch", patch: op.patch };
    }
    case "remove_patch": {
      const prev = model.patches[op.patch];
      return prev ? { op: "set_patch", patch: op.patch, descriptor: prev } : null;
    }
    case "set_header":
      return op.field === "title" ? { op: "set_header", field: "title", value: model.title } : { op: "set_header", field: op.field, value: model[op.field] };
  }
}

/**
 * Move the onset at `from` so it starts at `to`: a clear of the source and a set of the same cell
 * at the destination, in that order. The destination must lie inside the authored length and be
 * free of every other note along the moved note's hold (the note's own cells do not count), on a
 * row that is not folded away. An unchanged position is no gesture (empty list).
 */
export function scoreMoveOperations(model: ScoreModel, from: string, to: { step: number; row: number }): ScoreOp[] {
  const cell = model.cells[from];
  if (!cell) throw new Error("There is no note to move at that cell.");
  const [fromStep, fromRow] = from.split(":").map(Number);
  if (to.step === fromStep && to.row === fromRow) return [];
  if (!integer(to.step, 0, SCORE_MAX_STEPS - 1) || !integer(to.row, 0, SCORE_ROWS - 1)) throw new Error("That destination is outside the grid.");
  const without = scoreApplyLocal(model, { op: "clear_cell", step: fromStep, row: fromRow });
  const len = cell.k === "n" ? (cell.len ?? 1) : 1;
  if (to.step + len > model.steps) throw new Error("It would run past the end of the score.");
  if (scoreFreeRun(without, to.step, to.row, len) !== len) throw new Error("Another note is in the way there.");
  const entry: ScoreEntry = cell.k === "d" ? { mode: "drum", pad: cell.pad } : { mode: "note", patch: cell.p, len, ...(cell.vol === undefined ? {} : { volume: cell.vol }) };
  return [{ op: "clear_cell", step: fromStep, row: fromRow }, scoreCellOperation(without, to.step, to.row, entry)];
}

/**
 * Copy every onset of bar `bar` (zero-based, `stepsPerBar` steps wide) one bar later: the length
 * grows first if the next bar lies past it, then each cell is set where its destination is free
 * along its whole hold. Occupied destinations are skipped and counted, never overwritten; the
 * caller says how many were. Nothing is written past the format's 256 steps.
 */
export function scoreRepeatBar(model: ScoreModel, bar: number, stepsPerBar: number): { ops: ScoreOp[]; copied: number; skipped: number } {
  if (!integer(bar, 0, SCORE_MAX_STEPS) || !integer(stepsPerBar, 1, SCORE_MAX_STEPS)) throw new Error("That bar is outside the grid.");
  const start = bar * stepsPerBar, next = start + stepsPerBar;
  if (next + stepsPerBar > SCORE_MAX_STEPS) throw new Error("The score cannot grow past 256 steps.");
  const ops: ScoreOp[] = [];
  let working = model;
  if (working.steps < next + stepsPerBar) {
    const grow: ScoreOp = { op: "set_header", field: "steps", value: next + stepsPerBar };
    ops.push(grow); working = scoreApplyLocal(working, grow);
  }
  let copied = 0, skipped = 0;
  const sources = Object.entries(model.cells).map(([key, cell]) => ({ key, cell, step: Number(key.split(":")[0]), row: Number(key.split(":")[1]) }))
    .filter(({ step }) => step >= start && step < next).sort((a, b) => a.step - b.step || a.row - b.row);
  for (const { cell, step, row } of sources) {
    const len = cell.k === "n" ? (cell.len ?? 1) : 1, dest = step + stepsPerBar;
    if (scoreFreeRun(working, dest, row, len) !== len) { skipped++; continue; }
    const entry: ScoreEntry = cell.k === "d" ? { mode: "drum", pad: cell.pad } : { mode: "note", patch: cell.p, len, ...(cell.vol === undefined ? {} : { volume: cell.vol }) };
    // A note whose sound the score no longer holds cannot be placed again; it is skipped like a
    // taken cell rather than aborting the whole bar.
    let op: ScoreOp;
    try { op = scoreCellOperation(working, dest, row, entry); } catch { skipped++; continue; }
    ops.push(op); working = scoreApplyLocal(working, op); copied++;
  }
  return { ops, copied, skipped };
}

// --- phrase selection: several onsets handled as one gesture -------------------------------------

/** Onset keys inside the rectangle spanned by two corners (either order, inclusive), by step then row. */
export function scoreOnsetsIn(model: Pick<ScoreModel, "cells">, a: { step: number; row: number }, b: { step: number; row: number }): string[] {
  const [s0, s1] = [Math.min(a.step, b.step), Math.max(a.step, b.step)], [r0, r1] = [Math.min(a.row, b.row), Math.max(a.row, b.row)];
  return Object.keys(model.cells).map((key) => ({ key, step: Number(key.split(":")[0]), row: Number(key.split(":")[1]) }))
    .filter(({ step, row }) => step >= s0 && step <= s1 && row >= r0 && row <= r1)
    .sort((x, y) => x.step - y.step || x.row - y.row).map(({ key }) => key);
}

function selectionSources(model: ScoreModel, keys: readonly string[]) {
  const sources = [...new Set(keys)].filter((key) => model.cells[key]).map((key) => {
    const cell = model.cells[key], [step, row] = key.split(":").map(Number);
    return { key, cell, step, row, len: cell.k === "n" ? (cell.len ?? 1) : 1 };
  }).sort((x, y) => x.step - y.step || x.row - y.row);
  if (!sources.length) throw new Error("Nothing is selected.");
  return sources;
}
function entryOf(cell: ScoreCell): ScoreEntry {
  return cell.k === "d" ? { mode: "drum", pad: cell.pad } : { mode: "note", patch: cell.p, len: cell.len ?? 1, ...(cell.vol === undefined ? {} : { volume: cell.vol }) };
}

/**
 * Move every selected onset by the same offset as one gesture: every source is cleared, then
 * every note is set at its destination, so the phrase keeps its shape. Destinations are checked
 * against the projection with the whole selection lifted out (the phrase may slide over its own
 * old cells) and against each other; one blocked note refuses the whole move, nothing partial.
 */
export function scoreMoveSelection(model: ScoreModel, keys: readonly string[], dStep: number, dRow: number): ScoreOp[] {
  if (dStep === 0 && dRow === 0) return [];
  const sources = selectionSources(model, keys);
  let working = sources.reduce((m, { step, row }) => scoreApplyLocal(m, { op: "clear_cell", step, row }), model);
  const clears: ScoreOp[] = sources.map(({ step, row }) => ({ op: "clear_cell", step, row }));
  const sets: ScoreOp[] = [];
  // Sets run against the direction of travel (the note furthest ahead first). Each destination is
  // then written before the note behind it overwrites the place it came from, so every prefix of
  // the writes keeps every note, even where values repeat (an accent pattern); orderWrites alone
  // cannot tell two equal values apart (re-review M1).
  const ahead = (s: { step: number; row: number }) => s.step * dStep + s.row * dRow;
  for (const { cell, step, row, len } of [...sources].sort((x, y) => ahead(y) - ahead(x) || x.step - y.step || x.row - y.row)) {
    const to = { step: step + dStep, row: row + dRow };
    if (!integer(to.step, 0, SCORE_MAX_STEPS - 1) || !integer(to.row, 0, SCORE_ROWS - 1)) throw new Error("That would push a note off the grid.");
    if (to.step + len > model.steps) throw new Error("That would push a note past the end of the score.");
    if (scoreFreeRun(working, to.step, to.row, len) !== len) throw new Error("Another note is in the way there.");
    const op = scoreCellOperation(working, to.step, to.row, entryOf(cell));
    sets.push(op); working = scoreApplyLocal(working, op);
  }
  return [...clears, ...sets];
}

/**
 * Copy the selection so the copy starts where the phrase ends: every onset is shifted by the
 * phrase's own extent (first onset to the end of its last note). The score grows first if the
 * copy would run past its length; taken destinations are skipped and counted, never overwritten.
 */
export function scoreDuplicateSelection(model: ScoreModel, keys: readonly string[]): { ops: ScoreOp[]; copied: number; skipped: number; shift: number; keys: string[] } {
  const sources = selectionSources(model, keys);
  const first = sources[0].step, end = Math.max(...sources.map(({ step, len }) => step + len)), shift = end - first;
  if (end + shift > SCORE_MAX_STEPS) throw new Error("The copy would run past 256 steps.");
  const ops: ScoreOp[] = [];
  let working = model;
  if (working.steps < end + shift) {
    const grow: ScoreOp = { op: "set_header", field: "steps", value: end + shift };
    ops.push(grow); working = scoreApplyLocal(working, grow);
  }
  let copied = 0, skipped = 0;
  const placed: string[] = [];
  for (const { cell, step, row, len } of sources) {
    const dest = step + shift;
    if (scoreFreeRun(working, dest, row, len) !== len) { skipped++; continue; }
    let op: ScoreOp;
    try { op = scoreCellOperation(working, dest, row, entryOf(cell)); } catch { skipped++; continue; }
    ops.push(op); working = scoreApplyLocal(working, op); copied++; placed.push(`${dest}:${row}`);
  }
  return { ops, copied, skipped, shift, keys: placed };
}

/** Clear every selected onset as one gesture. */
export function scoreClearSelection(model: ScoreModel, keys: readonly string[]): ScoreOp[] {
  return selectionSources(model, keys).map(({ step, row }) => ({ op: "clear_cell", step, row }));
}

/**
 * The operations of a recorded gesture, checked against the projection as it is NOW (rolling
 * through the list) with the same guards a hand edit gets, so an undo or redo can never write
 * an overlap, a note past the end or a note without its sound after a collaborator moved
 * things. Throws on the first operation that can no longer be replayed; nothing is sent then.
 */
export function scoreReplay(model: ScoreModel, ops: readonly ScoreOp[]): ScoreOp[] {
  // Every note this list clears is lifted out before any of its notes is checked, the way a move is
  // checked against the score with the moving note lifted out: a gesture sends its writes before
  // its clears (so a refused write never leaves a note deleted), and its undo is the same shape,
  // so a nudge's undo would otherwise collide with the very note it is about to clear.
  let working = ops.reduce((m, op) => op.op === "clear_cell" || op.op === "clear_note" ? scoreApplyLocal(m, op) : m, model);
  const checked: ScoreOp[] = [];
  for (const op of ops) {
    let next: ScoreOp = op;
    if (op.op === "set_cell") {
      const value = op.value;
      next = scoreCellOperation(working, op.step, op.row, value.k === "d" ? { mode: "drum", pad: value.pad }
        : { mode: "note", patch: value.p, len: value.len ?? 1, ...(value.vol === undefined ? {} : { volume: value.vol }) });
    } else if (op.op === "set_note") {
      scoreNoteCheck(working, op.step, op.row, op.value);
    } else if (op.op === "set_header") {
      next = scoreHeaderOperation(op.field, String(op.value));
    }
    checked.push(next); if (next.op !== "clear_cell" && next.op !== "clear_note") working = scoreApplyLocal(working, next);
  }
  scoreRoomCheck(model, checked);
  return checked;
}

/**
 * The guard a layered note gets before it is written: inside the grid and the authored length, a
 * sound the score holds, a valid level, and no overlap with another hold of the SAME sound on its
 * row (other sounds overlapping is what layers are for). Throws with the reason; returns nothing.
 */
export function scoreNoteCheck(model: ScoreModel, step: number, row: number, value: ScoreCell): void {
  if (!integer(step, 0, SCORE_MAX_STEPS - 1) || !integer(row, 0, SCORE_ROWS - 1)) throw new Error("Cell is outside the score coordinate bounds.");
  if (step >= model.steps) throw new Error("Cell is beyond the authored score length. Clear it or extend the score first.");
  if (value.k === "d") { if (!integer(value.pad, 0, 9)) throw new Error("Drum pad must be 0–9."); return; }
  if (!/^[0-9a-f]{64}$/.test(value.p) || !Object.hasOwn(model.patches, value.p)) throw new Error("Choose a sound this score holds. A sound still being saved is not available until it is read back.");
  const len = value.len ?? 1;
  if (!integer(len, 1, model.steps - step)) throw new Error("The authored hold must fit the remaining sixteenth steps. No note is shortened automatically.");
  if (value.vol !== undefined && !integer(value.vol, SCORE_VOLUME_MIN, SCORE_VOLUME_MAX)) throw new Error(`Volume is ${SCORE_VOLUME_MIN} to ${SCORE_VOLUME_MAX}.`);
  const own = noteKey(step, row, value.p);
  for (const [key, cell] of Object.entries(model.notes ?? model.cells)) {
    if (key === own || cell.k !== "n" || cell.p !== value.p) continue;
    const at = parseNoteKey(key);
    if (at && at.row === row && at.step < step + len && step < at.step + (cell.len ?? 1)) throw new Error("This hold overlaps another note of the same sound. Clear or shorten that note first.");
  }
}

/** Native refuses a new note slot once the score holds 4096; the writes of a gesture land before
 * its clears, so the count is checked in that order and a full score refuses before anything is
 * sent. A move or a legacy note's edit needs one free place too, for the moment between its write
 * and its clear, so at the cap it is refused like a placement. */
function scoreRoomCheck(model: ScoreModel, ops: readonly ScoreOp[]): void {
  const present = new Set(Object.keys(model.notes ?? model.cells));
  for (const op of ops) {
    const key = op.op === "set_cell" || op.op === "clear_cell" ? `${op.step}:${op.row}` : op.op === "set_note" ? noteKey(op.step, op.row, layerOf(op.value)) : op.op === "clear_note" ? noteKey(op.step, op.row, op.layer) : null;
    if (key === null) continue;
    if (op.op === "clear_cell" || op.op === "clear_note") { present.delete(key); continue; }
    if (!present.has(key)) {
      if (present.size >= SCORE_MAX_CELLS) throw new Error("The score is full (4096 notes). Clear a note first.");
      present.add(key);
    }
  }
}

/**
 * An intentional choice reauthors ordinary operations; old source ids never become authority.
 * Choosing a legacy cell's value moves it into its layer: the layered note is written first and the
 * legacy cell cleared after it, so a refusal of either never loses the note. Choosing a clear clears.
 */
export function scoreDiagnosticOperation(model: ScoreModel, item: ScoreDiagnostic, index: number): ScoreOp[] {
  const chosen = item.alternatives[index];
  if (!chosen) throw new Error("That retained alternative is no longer available.");
  if (item.kind === "header") return [scoreHeaderOperation(item.key as "title" | "bpm" | "bpb" | "steps", String(chosen.value))];
  if (item.kind === "patch") return [chosen.value === null ? { op: "remove_patch", patch: item.key } : { op: "set_patch", patch: item.key, descriptor: chosen.value }];
  const at = parseNoteKey(item.key), value = chosen.value as ScoreCell | null;
  if (!at) throw new Error("That retained value has no place in the grid.");
  if (item.kind === "note") {
    return scoreReplay(model, [value === null ? { op: "clear_note", step: at.step, row: at.row, layer: at.layer! } : { op: "set_note", step: at.step, row: at.row, value }]);
  }
  return scoreReplay(model, value === null ? [{ op: "clear_cell", step: at.step, row: at.row }]
    : [{ op: "set_note", step: at.step, row: at.row, value }, { op: "clear_cell", step: at.step, row: at.row }]);
}

let lastGesture = 0;
/** A fresh id for one composer gesture. Module-wide, so a remount of the same document never
 *  reuses an id whose records are still queued (the session links a gesture's clears to its writes). */
export function scoreGestureId(): number {
  return ++lastGesture;
}

// --- layers: show, ghost, hide; the grid's one-note-per-cell surface -----------------------------
// A score holds one note per sound per cell (docs/design-score-layers.md section 8), so one cell
// can hold several notes. The grid shows and edits one note per cell: the chosen sound's when it
// has one there, else the first shown note by key. Every gesture is built against that surface with
// the one-note-per-cell rules the composer always had, then `scoreSurfaceOps` turns it into layered
// operations on the document. Visibility is per viewer and never written to the document; hiding
// a sound does not mute it.

/** A layer's visibility in the composer: shown (drawn, editable, takes placements), ghosted
 * (outlines only, never interactive; other sounds can be placed over it), hidden (not drawn). */
export type ScoreVisibility = "show" | "ghost" | "hide";
export const SCORE_VISIBILITY_NEXT: Readonly<Record<ScoreVisibility, ScoreVisibility>> = { show: "ghost", ghost: "hide", hide: "show" };
/** The layer of a note whose sound the score no longer holds: always shown, so it can be reviewed. */
export const SCORE_MISSING_LAYER = "missing";

/** The layer the composer draws a note in: its sound, the drum kit, or the missing-sound layer. */
export function scoreLayerOf(model: Pick<ScoreModel, "patches">, cell: ScoreCell): string {
  return cell.k === "d" ? SCORE_DRUM_LAYER : Object.hasOwn(model.patches, cell.p) ? cell.p : SCORE_MISSING_LAYER;
}
export function scoreVisibilityOf(visibility: Readonly<Record<string, ScoreVisibility>>, layer: string): ScoreVisibility {
  return layer === SCORE_MISSING_LAYER ? "show" : Object.hasOwn(visibility, layer) ? visibility[layer] : "show";
}

export type ScoreGhost = { key: string; step: number; row: number; cell: ScoreCell };
export type ScoreSurface = {
  /** The document with `cells` replaced by what the grid shows and edits: one note per cell. */
  model: ScoreModel;
  /** Each surface cell `"S:R"` to the note key behind it (`"S:R"` legacy, `"S:R/L"` layered). */
  keyOf: ReadonlyMap<string, string>;
  /** Notes of ghosted layers, by (step, row): drawn as outlines, never selected or edited. */
  ghosts: readonly ScoreGhost[];
  /** Surface cells where more than one shown note starts (the others are behind the shown one). */
  stacked: ReadonlySet<string>;
};

export function scoreSurface(doc: ScoreModel, visibility: Readonly<Record<string, ScoreVisibility>>, chosen: string | null): ScoreSurface {
  const entries: { key: string; step: number; row: number; cell: ScoreCell; layer: string }[] = [];
  for (const [key, cell] of Object.entries(doc.notes ?? doc.cells)) {
    const at = parseNoteKey(key);
    if (at) entries.push({ key, step: at.step, row: at.row, cell, layer: scoreLayerOf(doc, cell) });
  }
  // Within a cell the chosen sound comes first, then a layered note before a legacy cell (where a
  // note sits in both, the layered copy is the newer one: a discarded clear of the legacy cell left
  // the old value behind, and editing that would overwrite the newer note), then note-key order, so
  // which note a cell shows never depends on map enumeration.
  const legacy = (key: string) => (key.includes("/") ? 0 : 1);
  entries.sort((a, b) => a.step - b.step || a.row - b.row || Number(b.layer === chosen) - Number(a.layer === chosen) ||
    legacy(a.key) - legacy(b.key) || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
  const cells: Record<string, ScoreCell> = {}, keyOf = new Map<string, string>(), ghosts: ScoreGhost[] = [], stacked = new Set<string>();
  for (const entry of entries) {
    const shown = scoreVisibilityOf(visibility, entry.layer);
    if (shown === "hide") continue;
    if (shown === "ghost") { ghosts.push({ key: entry.key, step: entry.step, row: entry.row, cell: entry.cell }); continue; }
    const at = `${entry.step}:${entry.row}`;
    if (Object.hasOwn(cells, at)) { stacked.add(at); continue; }
    cells[at] = entry.cell; keyOf.set(at, entry.key);
  }
  return { model: { ...doc, cells }, keyOf, ghosts, stacked };
}

/** How a ghosted note draws on one page: its sound (null for a drum), and whether this cell is its
 * onset or its last step. The first ghost to reach a cell owns its outline. */
export type ScoreGhostCell = { p: string | null; onset: boolean; last: boolean };
export function scoreGhostCells(ghosts: readonly ScoreGhost[], columns: readonly number[]): Map<string, ScoreGhostCell> {
  const result = new Map<string, ScoreGhostCell>();
  if (!columns.length) return result;
  const first = columns[0], last = columns[columns.length - 1];
  for (const ghost of ghosts) {
    const end = ghost.step + (ghost.cell.k === "n" ? ghost.cell.len ?? 1 : 1) - 1;
    for (let s = Math.max(first, ghost.step); s <= Math.min(last, end); s++) {
      const at = `${s}:${ghost.row}`;
      if (!result.has(at)) result.set(at, { p: ghost.cell.k === "n" ? ghost.cell.p : null, onset: s === ghost.step, last: s === end });
    }
  }
  return result;
}

/** The readiness flags of a note key, in whichever collection holds it. */
export function scoreIssuesAt(model: Pick<ScoreModel, "readiness">, key: string): ScoreCellIssues | undefined {
  return key.includes("/") ? model.readiness.layers?.[key] : model.readiness.cells[key];
}

/**
 * The document operations of a gesture built on the surface. A surface cell op names a cell; the
 * note behind it decides what is written:
 * - a set writes `set_note` in the value's layer, and clears the note it replaces when that note
 *   lived elsewhere (a legacy cell moving into its layer, or a note changing sound);
 * - a clear removes the note behind the cell (`clear_cell` legacy, `clear_note` layered), and a
 *   cell with nothing behind it writes nothing.
 * Order: everything that is not a note first (a grown length must land before the notes that need
 * it), then the writes, then the clears, and only the last action on a note key survives. Writes
 * before clears means a refusal part way never deletes a note it was moving: the worst case is the
 * note in both places, which plays and can be cleared. The result is checked like a replay.
 */
export function scoreSurfaceOps(doc: ScoreModel, surface: Pick<ScoreSurface, "keyOf">, ops: readonly ScoreOp[]): ScoreOp[] {
  const keyOf = new Map(surface.keyOf), other: ScoreOp[] = [];
  const last = new Map<string, ScoreOp>(); // note key -> its final action, in the order last touched
  const notes = doc.notes ?? doc.cells;
  // Note keys this gesture has cleared so far (a picked note's old place, a stale copy). Moves emit
  // their clears first, so a note may land on a key its own phrase vacated.
  const vacated = new Set<string>();
  const clear = (real: string): ScoreOp => {
    const at = parseNoteKey(real)!;
    return at.layer === null ? { op: "clear_cell", step: at.step, row: at.row } : { op: "clear_note", step: at.step, row: at.row, layer: at.layer };
  };
  for (const op of ops) {
    if (op.op !== "set_cell" && op.op !== "clear_cell") { other.push(op); continue; }
    const at = `${op.step}:${op.row}`, behind = keyOf.get(at);
    // A stale legacy copy of the shown layered note's sound in this cell (a discarded clear of it
    // left it behind) is the same note. Editing, moving or clearing the shown note takes the copy
    // with it: left alone it would overlap an edit (which is then refused), or reappear and play
    // after a clear or a move.
    const legacy = notes[at];
    const staleCopy = behind !== undefined && behind !== at && legacy !== undefined && behind.endsWith(`/${layerOf(legacy)}`);
    if (op.op === "set_cell") {
      const key = noteKey(op.step, op.row, layerOf(op.value));
      // The surface holds one note per cell, so a cell can look free while a note of THIS sound sits
      // stacked behind another sound's note there. Writing would silently replace that note, which
      // no one picked (third review M-A): refuse, unless it is the note shown here or one this
      // gesture already moved away.
      if (key !== behind && Object.hasOwn(notes, key) && !vacated.has(key)) throw new Error("Another note of that sound is in the way there.");
      if (behind && behind !== key) { last.delete(behind); last.set(behind, clear(behind)); vacated.add(behind); }
      if (staleCopy) { last.delete(at); last.set(at, clear(at)); vacated.add(at); }
      last.delete(key); last.set(key, { op: "set_note", step: op.step, row: op.row, value: op.value });
      vacated.delete(key);
      keyOf.set(at, key);
    } else if (behind) {
      last.delete(behind); last.set(behind, clear(behind)); vacated.add(behind);
      if (staleCopy) { last.delete(at); last.set(at, clear(at)); vacated.add(at); }
      keyOf.delete(at);
    }
  }
  const actions = [...last.values()];
  const writes = actions.filter((op) => op.op === "set_note"), clears = actions.filter((op) => op.op !== "set_note");
  return scoreReplay(doc, [...other, ...orderWrites(doc, writes), ...clears]);
}

/**
 * Writes in an order that avoids overwriting a note before its value has landed at its new place
 * (review M2). Sliding notes A at 2 and B at 4 two steps later is "4 <- A, 6 <- B": written in that
 * order, a refusal of the second leaves B nowhere but the refused record. So a write that replaces
 * a value some other write of this gesture carries waits for that write. Writes with nothing to
 * wait for keep their order; one that waits follows the write it waits for. Linear in the gesture:
 * one count per written value.
 *
 * Values are compared, not notes, so equal values (an accent pattern a b a) can look like a cycle;
 * a cycle keeps the order it came in. That order is what makes phrase moves safe there:
 * `scoreMoveSelection` emits its sets against the direction of travel. Every prefix of every move
 * of small patterns is checked exhaustively in score-layers.test.ts.
 */
function orderWrites(doc: ScoreModel, writes: readonly ScoreOp[]): ScoreOp[] {
  const notes = doc.notes ?? doc.cells;
  // The same note however it is spelled: a gesture re-authors `{p}` as `{len: 1, p}`, and a full
  // level is written as no `vol` at all.
  const identity = (cell: ScoreCell) => cell.k === "d" ? canonicalJson(cell)
    : canonicalJson({ p: cell.p, len: cell.len ?? 1, vol: cell.vol ?? SCORE_VOLUME_MAX });
  const items = writes.map((op) => {
    const value = op.op === "set_note" ? op.value : null, written = value ? identity(value) : "";
    const current = value && op.op === "set_note" ? notes[noteKey(op.step, op.row, layerOf(value))] : undefined;
    const replaced = current ? identity(current) : null;
    return { op, written, waitsFor: replaced !== null && replaced !== written ? replaced : null };
  });
  const pending = new Map<string, number>();
  for (const item of items) pending.set(item.written, (pending.get(item.written) ?? 0) + 1);
  const waiting = new Map<string, typeof items>();
  const out: ScoreOp[] = [], done = new Set<(typeof items)[number]>();
  const emit = (item: (typeof items)[number]) => {
    out.push(item.op); done.add(item);
    const left = (pending.get(item.written) ?? 1) - 1;
    pending.set(item.written, left);
    if (left === 0) { const released = waiting.get(item.written) ?? []; waiting.delete(item.written); released.forEach(emit); }
  };
  for (const item of items) {
    if (item.waitsFor !== null && (pending.get(item.waitsFor) ?? 0) > 0) {
      const list = waiting.get(item.waitsFor);
      if (list) list.push(item); else waiting.set(item.waitsFor, [item]);
    } else emit(item);
  }
  for (const item of items) if (!done.has(item)) out.push(item.op); // a cycle: original order
  return out;
}
