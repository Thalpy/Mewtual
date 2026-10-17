// Pure Score composer transitions. Native admission remains the authority; these guards keep
// ordinary clicks from knowingly creating an unexportable score and never trim retained work.
import { JAM_MET_BPB_MAX, JAM_MET_BPB_MIN, JAM_MET_BPM_MAX, JAM_MET_BPM_MIN } from "./jam-contract.ts";
import { SCORE_MAX_CELLS, SCORE_MAX_STEPS, SCORE_ROWS, type ScoreCell, type ScoreOp } from "./studio-contract.ts";
import type { IndexSource, ScoreModel, Valued } from "./studio-native.ts";

export const SCORE_PAGE_STEPS = 16;
export const SCORE_DIAGNOSTIC_PAGE = 24;
export const SCORE_ALTERNATIVE_PAGE = 8;
const PITCHES = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];
const integer = (n: number, min: number, max: number) => Number.isInteger(n) && n >= min && n <= max;

/** Row zero is MIDI 60 (C4), row 24 MIDI 84 (C6), without a scale-dependent remapping. */
export function scorePitch(row: number): string {
  if (!integer(row, 0, SCORE_ROWS - 1)) throw new Error("Pitch row is outside C4–C6.");
  return `${PITCHES[row % 12]}${4 + Math.floor(row / 12)}`;
}

export function scorePage(steps: number, page: number): number[] {
  if (!integer(steps, 1, SCORE_MAX_STEPS) || !integer(page, 0, Math.ceil(steps / SCORE_PAGE_STEPS) - 1)) throw new Error("Score page is outside the authored length.");
  return Array.from({ length: Math.min(SCORE_PAGE_STEPS, steps - page * SCORE_PAGE_STEPS) }, (_, i) => page * SCORE_PAGE_STEPS + i);
}

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
export function scoreContinuations(model: ScoreModel, page: number): Set<string> {
  const columns = scorePage(model.steps, page), start = columns[0], end = columns.at(-1)!;
  const result = new Set<string>();
  for (const [key, cell] of Object.entries(model.cells)) if (cell.k === "n") {
    const [step, row] = key.split(":").map(Number);
    for (let s = Math.max(start, step + 1); s <= Math.min(end, step + (cell.len ?? 1) - 1); s++) result.add(`${s}:${row}`);
  }
  return result;
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

/** An intentional choice reauthors an ordinary operation; old source ids never become authority. */
export function scoreDiagnosticOperation(model: ScoreModel, item: ScoreDiagnostic, index: number): ScoreOp {
  const chosen = item.alternatives[index];
  if (!chosen) throw new Error("That retained alternative is no longer available.");
  if (item.kind === "header") return scoreHeaderOperation(item.key as "title" | "bpm" | "bpb" | "steps", String(chosen.value));
  if (item.kind === "patch") return chosen.value === null ? { op: "remove_patch", patch: item.key } : { op: "set_patch", patch: item.key, descriptor: chosen.value };
  const [step, row] = item.key.split(":").map(Number), value = chosen.value as ScoreCell | null;
  return scoreCellOperation(model, step, row, value === null ? { mode: "clear" } : value.k === "d" ? { mode: "drum", pad: value.pad } : { mode: "note", patch: value.p, len: value.len ?? 1 });
}
