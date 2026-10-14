import type { JamTake } from "./jam-contract.ts";
import type { JamArchivalPlayback } from "./jam-engine.ts";
import { JamLatestTaskQueue } from "./jam-publication.ts";
import { StudioPlaybackController, type StudioPlaybackEngine, type StudioPlaybackScope } from "./studio-playback.ts";

export interface JamTakeTransportEngine extends StudioPlaybackEngine {
  setArchivalPlaybackLevel(capability: JamArchivalPlayback, level: number): boolean;
}

/**
 * Shared/local take adapter to the same archival renderer used by Studio. The shared deck keeps
 * its existing authenticated call/transport lease outside this class; imported metadata grants
 * no remote consent. One App-owned queue bounds preparation across replacement instances.
 */
export class JamTakeTransport {
  private readonly controller: StudioPlaybackController;
  private readonly engine: JamTakeTransportEngine;
  private capability: JamArchivalPlayback | null = null;
  private remote = true;
  private level = 1;
  private current: () => boolean = () => false;

  constructor(engine: JamTakeTransportEngine, queue: JamLatestTaskQueue<JamArchivalPlayback | null>, now?: () => number) {
    this.engine = engine;
    const context: Pick<AudioContext, "state" | "resume"> = {
      get state() { return engine.context.state; },
      // A remote deck heartbeat never resumes audio, including if state changes after admission.
      resume: () => this.remote ? Promise.reject(new Error("remote take cannot resume audio")) : engine.context.resume(),
    };
    this.controller = new StudioPlaybackController({ context,
      prepareArchivalPlayback: (take, animation, isCurrent) => queue.submit(async () => {
        const current = () => isCurrent() && this.current() && engine.context.state === "running";
        if (!current()) return null;
        const capability = await engine.prepareArchivalPlayback(take, animation, current);
        if (capability && current()) { this.capability = capability; engine.setArchivalPlaybackLevel(capability, this.level); }
        return capability;
      }),
      archivalPlaybackPlan: (capability) => this.current() ? engine.archivalPlaybackPlan(capability) : null,
      playArchivalEvent: (capability, index, age) => this.current() ? engine.playArchivalEvent(capability, index, age) : { ok: false, reason: "channel" },
      revokeArchivalPlayback: (capability) => { if (this.capability === capability) this.capability = null; return engine.revokeArchivalPlayback(capability); },
    }, now);
  }
  get state() { return this.controller.state; }
  get completed(): boolean { return this.controller.completed; }
  get elapsedMs(): number { return this.controller.elapsedMs; }
  async start(take: JamTake, offsetMs: number, scope: StudioPlaybackScope, remote: boolean, isCurrent: () => boolean): Promise<boolean> {
    this.stop(); this.remote = remote; this.current = isCurrent;
    if (!isCurrent() || (remote && this.engine.context.state !== "running")) return false;
    this.controller.setScope(scope, false);
    return this.controller.startTake(take, offsetMs);
  }
  setLevel(level: number): boolean {
    if (!Number.isFinite(level) || level < 0 || level > 1) return false;
    if (this.capability && !this.current()) return false;
    this.level = level;
    return this.capability ? this.engine.setArchivalPlaybackLevel(this.capability, level) : true;
  }
  tick(): Promise<number> { if (this.engine.context.state !== "running") { this.stop(); return Promise.resolve(0); } return this.controller.tick(); }
  stop(): void { this.current = () => false; this.controller.stop(); }
}
