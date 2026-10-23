// What the score composer had selected, kept per document so that switching documents and
// coming back finds the same note or phrase. Keys are the composer's own "step:row" cell keys;
// nothing here is read from the wire, and a stale key (a note cleared meanwhile) is harmless:
// the composer only ever acts on the keys that still hold a cell.
import { SCORE_MAX_STEPS, SCORE_ROWS } from "./studio-contract.ts";

export type ScoreSelection = Readonly<{ selected: string | null; picked: readonly string[] }>;

/** One bounded, most-recently-used store of composer selections, keyed by document id. */
export class ScoreSelectionMemory {
  private readonly entries = new Map<string, ScoreSelection>();
  private readonly max: number;
  constructor(max = 32) { this.max = max; }
  /** Keep `selection` for `id`; an empty selection forgets the document instead. */
  remember(id: string, selection: ScoreSelection): void {
    this.entries.delete(id);
    const picked = selection.picked.filter(validScoreKey);
    const selected = selection.selected !== null && validScoreKey(selection.selected) ? selection.selected : null;
    if (selected === null && !picked.length) return;
    this.entries.set(id, { selected, picked });
    while (this.entries.size > this.max) this.entries.delete(this.entries.keys().next().value!);
  }
  recall(id: string): ScoreSelection | null { return this.entries.get(id) ?? null; }
  forget(id: string): void { this.entries.delete(id); }
}

/** A "step:row" key inside the score coordinate bounds. */
export function validScoreKey(key: string): boolean {
  const m = /^(\d{1,3}):(\d{1,2})$/.exec(key);
  return !!m && Number(m[1]) < SCORE_MAX_STEPS && Number(m[2]) < SCORE_ROWS;
}

/** The one memory the Studio reads and writes; it outlives the Studio surface's mounts. */
export const scoreSelectionMemory = new ScoreSelectionMemory();
