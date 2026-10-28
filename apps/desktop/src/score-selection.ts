// Selection survives ordinary document switches, but belongs to the actual native group,
// channel and object. Lock/reset and server retirement revoke writers before clearing it.
import { SCORE_MAX_STEPS, SCORE_ROWS } from "./studio-contract.ts";

export type ScoreSelection = Readonly<{ selected: string | null; picked: readonly string[] }>;
export type ScoreSelectionScope = Readonly<{ server: number; group: string; channel: string; object: string }>;
export type ScoreSelectionWriter = Readonly<{
  recall(): ScoreSelection | null;
  remember(selection: ScoreSelection): boolean;
  revoke(): void;
}>;
type WriterCell = { live: boolean; epoch: number; key: string; scope: ScoreSelectionScope | null; current: (() => boolean) | null };
const MAX_ENTRIES = 32;
const MAX_PICKED = SCORE_MAX_STEPS * SCORE_ROWS;
const INACTIVE: ScoreSelectionWriter = Object.freeze({ recall: () => null, remember: () => false, revoke: () => {} });

/** One active editor writer and at most32 remembered native scopes. A returned callback always
 * names its ORIGINAL cell; an old mount can never write through a replacement writer. */
export class ScoreSelectionMemory {
  private readonly entries = new Map<string, { server: number; selection: ScoreSelection }>();
  private active: WriterCell | null = null;
  private epoch = 0;
  private readonly max: number;
  constructor(max = MAX_ENTRIES) { this.max = Number.isFinite(max) ? Math.max(0, Math.min(MAX_ENTRIES, Math.floor(max))) : MAX_ENTRIES; }

  /** Called only after a native-backed view supplies group identity. The caller's pure
   * currentness check binds its original session generation, scope, object and close state. */
  openWriter(scope: ScoreSelectionScope, current: () => boolean): ScoreSelectionWriter {
    if (this.active) this.revoke(this.active);
    // Native groups are bounded variable-length hex (not necessarily64characters).
    if (!Number.isSafeInteger(scope.server) || scope.server < 0 || !scope.group || scope.group.length > 512 ||
        !scope.channel || scope.channel.length > 128 || !scope.object || scope.object.length > 128) return INACTIVE;
    const owned = Object.freeze({ server: scope.server, group: scope.group, channel: scope.channel, object: scope.object });
    const cell: WriterCell = { live: true, epoch: this.epoch,
      key: JSON.stringify([owned.group, owned.channel, owned.object]), scope: owned, current };
    this.active = cell;
    return Object.freeze({
      recall: () => {
        if (!this.valid(cell)) return null;
        const entry = this.entries.get(cell.key);
        return entry ? { selected: entry.selection.selected, picked: [...entry.selection.picked] } : null;
      },
      remember: (selection: ScoreSelection) => {
        if (!this.valid(cell)) return false;
        // Refuse an oversized phrase BEFORE allocating/iterating it. Ordinary selections
        // have at most one key per cell; duplicates do not gain extra retained capacity.
        if (selection.picked.length > MAX_PICKED) return false;
        const picked = [...new Set(selection.picked.filter(validScoreKey))];
        const selected = selection.selected !== null && validScoreKey(selection.selected) ? selection.selected : null;
        // The currentness callback may reenter/reset. Recheck after copying, immediately
        // before publication; never publish through a cell replaced during that callback.
        if (!this.valid(cell)) return false;
        this.entries.delete(cell.key);
        if (selected !== null || picked.length) this.entries.set(cell.key,
          { server: cell.scope!.server, selection: { selected, picked } });
        while (this.entries.size > this.max) this.entries.delete(this.entries.keys().next().value!);
        return true;
      },
      revoke: () => this.revoke(cell),
    });
  }

  /** Negative publication precedes teardown, so late unmount callbacks cannot refill memory. */
  clear(): void {
    this.epoch++;
    if (this.active) this.revoke(this.active);
    this.entries.clear();
  }

  retireServer(server: number): void {
    if (this.active?.scope?.server === server) this.revoke(this.active);
    for (const [key, entry] of this.entries) if (entry.server === server) this.entries.delete(key);
  }

  private valid(cell: WriterCell): boolean {
    if (!cell.live || this.active !== cell || cell.epoch !== this.epoch || !cell.current) return false;
    let current = false;
    try { current = cell.current(); } catch { /* stale editor callbacks are inert */ }
    return current && cell.live && this.active === cell && cell.epoch === this.epoch;
  }

  private revoke(cell: WriterCell): void {
    cell.live = false;
    // Old exported handles retain only the inert cell, not a callback back into Studio.
    cell.current = null; cell.scope = null; cell.key = "";
    if (this.active === cell) this.active = null;
  }
}

/** A "step:row" key inside the score coordinate bounds. */
export function validScoreKey(key: string): boolean {
  const m = /^(\d{1,3}):(\d{1,2})$/.exec(key);
  return !!m && Number(m[1]) < SCORE_MAX_STEPS && Number(m[2]) < SCORE_ROWS;
}

/** The one memory the Studio reads and writes; it outlives the Studio surface's mounts. */
export const scoreSelectionMemory = new ScoreSelectionMemory();
