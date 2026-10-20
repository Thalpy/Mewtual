import type { JamTake } from "./jam-contract.ts";
import { StudioPlaybackController, type StudioPlaybackEngine, type StudioPlaybackScope, type StudioPlaybackState } from "./studio-playback.ts";
import { planJamArchivalPlayback } from "./jam-archival-plan.ts";
import { decodePixa, encodePixa } from "./pixa.ts";
import type { ScorePlayback } from "./studio-export.ts";

/** `nextPass`: a looping Score has edits waiting for its next pass (being prepared, or ready). */
export type StudioAudioState = Readonly<{ state: StudioPlaybackState; elapsedMs: number; nextPass?: boolean }>;
export type StudioAudioActions = {
  /** `document`: the Studio's own editor Play of the open document (a layered score may hold more
   * notes at once). Received content (chat's PIXA viewer, inline playback) leaves it false. */
  start: (kind: "score" | "flipnote", prepare: () => Promise<ScorePlayback | Uint8Array>, loop: boolean, offsetMs?: number, document?: boolean) => Promise<boolean>;
  stop: () => void;
  /** The looping Score's source changed: prepare its next pass from the document as it is now. */
  refresh?: () => void;
};

/** How many consecutive failed next-pass preparations are retried on later ticks before the
 *  loop keeps its current pass until the source changes again. */
export const STUDIO_REFRESH_RETRIES = 3;

type Active = {
  kind: "score" | "flipnote"; value: JamTake | Uint8Array; gridMs: number; loop: boolean; generation: number;
  /** Carried to every loop restart (see StudioAudioActions.start). */
  document: boolean;
  /** The Score's preparation, kept for `refresh`; null for a flipnote. */
  prepare: (() => Promise<ScorePlayback | Uint8Array>) | null;
  /** The pass prepared from the source as it is now, swapped in at the next loop restart. */
  next: { value: JamTake; gridMs: number } | null;
  /** A refresh is wanted (set by `refresh`, and again by a failed preparation that may retry). */
  dirty: boolean;
  refreshing: boolean;
  failures: number;
};

/** Gesture/preparation/loop coordinator around the SAME bounded archival controller and synth. */
export class StudioAudioSession {
  private readonly controller: StudioPlaybackController;
  private scopeKey = "";
  private generation = 0;
  private deafened = true;
  private preparing = false;
  private active: Active | null = null;
  private elapsed = 0;
  private ticking = false;
  private readonly engine: StudioPlaybackEngine;
  constructor(engine: StudioPlaybackEngine, now?: () => number) { this.engine = engine; this.controller = new StudioPlaybackController(engine, now); }
  get state(): StudioAudioState {
    const active = this.active;
    return { state: this.preparing ? "preparing" : this.controller.state, elapsedMs: this.controller.state === "playing" ? this.controller.elapsedMs : this.elapsed,
      nextPass: !!active && (active.dirty || active.refreshing || active.next !== null) };
  }
  setScope(scope: StudioPlaybackScope | null, deafened: boolean): void {
    const key = scope ? `${scope.generation}:${scope.server}:${scope.channel}:${scope.membership}` : "";
    if (key !== this.scopeKey || deafened !== this.deafened) this.stop();
    this.scopeKey = key; this.deafened = deafened; this.controller.setScope(scope, deafened);
  }
  stop(): void { this.generation++; this.preparing = false; this.active = null; this.elapsed = 0; this.controller.stop(); }
  async start(kind: "score" | "flipnote", prepare: () => Promise<ScorePlayback | Uint8Array>, loop: boolean, offsetMs = 0, document = false): Promise<boolean> {
    this.stop();
    if (!this.scopeKey || this.deafened) return false;
    const generation = this.generation;
    const current = () => generation === this.generation && !!this.scopeKey && !this.deafened;
    // This line runs in the original button's stack, before frame fetches or content hashing.
    // Reopening a suspended context later on an async callback is never used as implicit consent.
    const resumed = this.engine.context.state === "suspended" ? this.engine.context.resume() : Promise.resolve();
    this.preparing = true;
    try {
      const [input] = await Promise.all([prepare(), resumed]);
      if (!current() || this.engine.context.state !== "running") return false;
      // Keep one owned immutable loop source. A caller mutating its original take/Buffer after
      // the initial press must not change the content of a later automatic loop. A Score's grid
      // end is read once: trailing rests are part of the loop, not trimmed to the last release.
      // A Score's source may still change: `refresh` prepares the next pass from it afresh.
      let value: JamTake | Uint8Array, gridMs = 0;
      if (kind === "score") {
        const score = input as ScorePlayback;
        gridMs = score.gridMs;
        value = planJamArchivalPlayback(score.take).take;
      } else value = encodePixa(decodePixa(input as Uint8Array));
      this.controller.document = document;
      const started = kind === "score" ? await this.controller.startTake(value as JamTake, offsetMs, gridMs) : await this.controller.startPixa(value as Uint8Array, offsetMs);
      if (!current()) return false;
      if (started) this.active = { kind, value, gridMs, loop: loop && (kind !== "score" || (value as JamTake).events.length > 0), generation, document,
        prepare: kind === "score" ? prepare : null, next: null, dirty: false, refreshing: false, failures: 0 };
      return started;
    } finally { if (current()) this.preparing = false; }
  }
  /** The looping Score's source changed. Its next pass is prepared now, from the source as it is,
   *  and swapped in when the current pass ends; a pass already under way is never cut. Requests
   *  coalesce: one preparation runs at a time and a change during it runs one more. A one-shot
   *  play, a flipnote or an audition has no next pass and ignores this. */
  refresh(): void {
    const active = this.active;
    if (!active || active.kind !== "score" || !active.loop || !active.prepare) return;
    active.dirty = true; active.failures = 0;
    void this.prepareNext(active);
  }
  private async prepareNext(active: Active): Promise<void> {
    if (active.refreshing) return;
    active.refreshing = true;
    try {
      while (active.dirty && this.active === active && active.generation === this.generation) {
        active.dirty = false;
        try {
          const input = await active.prepare!() as ScorePlayback;
          if (this.active !== active || active.generation !== this.generation) return;
          active.next = { value: planJamArchivalPlayback(input.take).take, gridMs: input.gridMs };
          active.failures = 0;
        } catch {
          // The source moved under the export (an edit landed meanwhile, which asks for another
          // refresh anyway) or the document cannot be exported right now (a save it must settle
          // first). A later tick retries a few times; after that the loop keeps its current pass
          // until the source changes again, which asks afresh.
          if (this.active === active && !active.dirty && ++active.failures < STUDIO_REFRESH_RETRIES) active.dirty = true;
          return;
        }
      }
    } finally { active.refreshing = false; }
  }
  /** One owning App timer calls this; concurrent timer turns never drain overlapping batches. */
  async tick(): Promise<void> {
    if (this.ticking) return;
    const active = this.active;
    if (!active) return;
    if (this.engine.context.state !== "running") { this.stop(); return; }
    this.ticking = true;
    try {
      this.elapsed = this.controller.elapsedMs;
      if (active.dirty && !active.refreshing) void this.prepareNext(active);
      await this.controller.tick();
      if (this.active !== active || active.generation !== this.generation) return;
      if (this.controller.state === "stopped") {
        if (!this.controller.completed || !active.loop || this.deafened || !this.scopeKey || this.engine.context.state !== "running") { this.active = null; return; }
        // An explicit loop owns a complete package, including its longest hold/release horizon
        // and a Score's grid end. Restart mints a fresh engine capability only after both. A
        // Score restarts from the pass prepared since (its edits), else from the same package;
        // a preparation still under way lands the pass after.
        const next = active.next;
        if (next) { active.value = next.value; active.gridMs = next.gridMs; active.next = null; }
        this.controller.document = active.document;
        const started = active.kind === "score" ? await this.controller.startTake(active.value as JamTake, 0, active.gridMs) : await this.controller.startPixa(active.value as Uint8Array);
        if (this.active !== active || active.generation !== this.generation) return;
        if (!started) this.active = null;
        else this.elapsed = 0;
      }
    } finally { this.ticking = false; }
  }
}
