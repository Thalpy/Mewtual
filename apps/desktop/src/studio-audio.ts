import type { JamTake } from "./jam-contract.ts";
import { StudioPlaybackController, type StudioPlaybackEngine, type StudioPlaybackScope, type StudioPlaybackState } from "./studio-playback.ts";
import { planJamArchivalPlayback } from "./jam-archival-plan.ts";
import { decodePixa, encodePixa } from "./pixa.ts";

export type StudioAudioState = Readonly<{ state: StudioPlaybackState; elapsedMs: number }>;
export type StudioAudioActions = {
  start: (kind: "score" | "flipnote", prepare: () => Promise<JamTake | Uint8Array>, loop: boolean, offsetMs?: number) => Promise<boolean>;
  stop: () => void;
};

/** Gesture/preparation/loop coordinator around the SAME bounded archival controller and synth. */
export class StudioAudioSession {
  private readonly controller: StudioPlaybackController;
  private scopeKey = "";
  private generation = 0;
  private deafened = true;
  private preparing = false;
  private active: { kind: "score" | "flipnote"; value: JamTake | Uint8Array; loop: boolean; generation: number } | null = null;
  private elapsed = 0;
  private ticking = false;
  private readonly engine: StudioPlaybackEngine;
  constructor(engine: StudioPlaybackEngine, now?: () => number) { this.engine = engine; this.controller = new StudioPlaybackController(engine, now); }
  get state(): StudioAudioState { return { state: this.preparing ? "preparing" : this.controller.state, elapsedMs: this.controller.state === "playing" ? this.controller.elapsedMs : this.elapsed }; }
  setScope(scope: StudioPlaybackScope | null, deafened: boolean): void {
    const key = scope ? `${scope.generation}:${scope.server}:${scope.channel}:${scope.membership}` : "";
    if (key !== this.scopeKey || deafened !== this.deafened) this.stop();
    this.scopeKey = key; this.deafened = deafened; this.controller.setScope(scope, deafened);
  }
  stop(): void { this.generation++; this.preparing = false; this.active = null; this.elapsed = 0; this.controller.stop(); }
  async start(kind: "score" | "flipnote", prepare: () => Promise<JamTake | Uint8Array>, loop: boolean, offsetMs = 0): Promise<boolean> {
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
      // the initial press must not change the content of a later automatic loop.
      const value = kind === "score" ? planJamArchivalPlayback(input).take : encodePixa(decodePixa(input as Uint8Array));
      const started = kind === "score" ? await this.controller.startTake(value as JamTake, offsetMs) : await this.controller.startPixa(value as Uint8Array, offsetMs);
      if (!current()) return false;
      if (started) this.active = { kind, value, loop: loop && (kind !== "score" || (value as JamTake).events.length > 0), generation };
      return started;
    } finally { if (current()) this.preparing = false; }
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
      await this.controller.tick();
      if (this.active !== active || active.generation !== this.generation) return;
      if (this.controller.state === "stopped") {
        if (!this.controller.completed || !active.loop || this.deafened || !this.scopeKey || this.engine.context.state !== "running") { this.active = null; return; }
        // An explicit loop owns a complete package, including its longest hold/release horizon.
        // Restart mints a fresh engine capability only after all authored sound has completed.
        const started = active.kind === "score" ? await this.controller.startTake(active.value as JamTake) : await this.controller.startPixa(active.value as Uint8Array);
        if (this.active !== active || active.generation !== this.generation) return;
        if (!started) this.active = null;
        else this.elapsed = 0;
      }
    } finally { this.ticking = false; }
  }
}
