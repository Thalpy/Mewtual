import { TAKE_MAX_DURATION_MS, TAKE_PLAYBACK_EVENTS_PER_TICK, type JamTake } from "./jam-contract.ts";
import { archivalReleaseTailMs, planJamArchivalPlayback, planJamArchivalSeek, type JamArchivalAnimation, type JamArchivalPlan } from "./jam-archival-plan.ts";
import { JamLatestTaskQueue } from "./jam-publication.ts";
import type { JamArchivalPlayback, JamPlayResult } from "./jam-engine.ts";
import { decodePixa } from "./pixa.ts";

/** Current native/UI custody, not metadata from the imported take. Change generation at every ABA edge. */
export type StudioPlaybackScope = Readonly<{ generation: number; server: number; channel: string; membership: number }>;
export type StudioPlaybackState = "stopped" | "preparing" | "playing";

/** Small injectable engine seam; production passes the existing JamEngine, never another synth. */
export interface StudioPlaybackEngine {
  readonly context: Pick<AudioContext, "state" | "resume">;
  prepareArchivalPlayback(take: unknown, animation: JamArchivalAnimation | undefined, isCurrent: () => boolean): Promise<JamArchivalPlayback | null>;
  archivalPlaybackPlan(playback: JamArchivalPlayback): JamArchivalPlan | null;
  playArchivalEvent(playback: JamArchivalPlayback, index: number, ageMs?: number): JamPlayResult | Promise<JamPlayResult>;
  revokeArchivalPlayback(playback: JamArchivalPlayback): boolean;
}

type Active = {
  capability: JamArchivalPlayback;
  plan: JamArchivalPlan;
  generation: number;
  origin: number;
  offset: number;
  /** Natural EOF in elapsed playback time; see completionMs. */
  complete: number;
  next: number;
  seek: readonly number[];
  seekNext: number;
  draining: boolean;
};

/**
 * Refusals that drop one bound event while the rest of the take stays playable: shared voice
 * budget, a same-note overlap and an already-consumed index. Any other refusal (`audio`, a lost
 * `channel`, `invalid`, `muted`, or none given) means the take is not being rendered as authored.
 */
const DROPPED_EVENT: ReadonlySet<JamPlayResult["reason"]> = new Set(["room-held", "source-held", "duplicate", "sequence"]);

/**
 * Natural EOF: every authored release has finished AND any authored length the take format cannot
 * carry (a Score's trailing rests and empty bars) has elapsed. A release crossing that length wins.
 */
function completionMs(plan: JamArchivalPlan, horizonMs: number): number {
  return Math.max(horizonMs, plan.endMs + archivalReleaseTailMs(plan) + 5);
}

function scopeEqual(a: StudioPlaybackScope | null, b: StudioPlaybackScope | null): boolean {
  return !!a && !!b && a.generation === b.generation && a.server === b.server && a.channel === b.channel && a.membership === b.membership;
}

/**
 * Explicit local playback of retained score/take/.pixa content. It neither fetches nor auto-plays
 * incoming content. The owning surface calls setScope on leave, lock, navigation, membership and
 * Deafen edges, and calls tick from one bounded animation/timer loop. There are no hidden timers.
 * Reopening audio does not resume a revoked playback: another explicit press is required.
 */
export class StudioPlaybackController {
  private scope: StudioPlaybackScope | null = null;
  private deafened = true;
  private generation = 0;
  private readonly preparation = new JamLatestTaskQueue<JamArchivalPlayback | null>();
  private active: Active | null = null;
  private stateValue: StudioPlaybackState = "stopped";
  private completedValue = false;
  private lastTick = 0;

  private readonly engine: StudioPlaybackEngine;
  private readonly nowMs: () => number;

  constructor(engine: StudioPlaybackEngine, nowMs: () => number = () => performance.now()) {
    this.engine = engine;
    this.nowMs = nowMs;
  }

  get state(): StudioPlaybackState { return this.stateValue; }
  /** True only for natural horizon completion; cancellation/revocation/render failure is not EOF. */
  get completed(): boolean { return this.completedValue; }
  get elapsedMs(): number {
    const active = this.active;
    return active ? active.offset + Math.max(0, this.nowMs() - active.origin) : 0;
  }

  setScope(scope: StudioPlaybackScope | null, deafened: boolean): void {
    const owned = scope ? { generation: scope.generation, server: scope.server, channel: scope.channel, membership: scope.membership } : null;
    if (owned && (!Number.isSafeInteger(owned.generation) || owned.generation < 0 || !Number.isSafeInteger(owned.server) || owned.server < 0 ||
      !Number.isSafeInteger(owned.membership) || owned.membership < 0 || typeof owned.channel !== "string" || owned.channel.length < 1 || owned.channel.length > 256)) {
      throw new TypeError("invalid studio playback scope");
    }
    if (!scopeEqual(this.scope, owned) || this.deafened !== deafened) this.stop();
    this.scope = owned;
    this.deafened = deafened;
  }

  stop(): void {
    this.completedValue = false;
    this.generation++;
    const active = this.active;
    this.active = null;
    this.stateValue = "stopped";
    if (active) this.engine.revokeArchivalPlayback(active.capability);
  }

  /**
   * Call directly from the local Play gesture; audio resume must not be deferred to a remote callback.
   * `horizonMs` is authored length beyond the last event (a Score grid end). The shared deck passes
   * none: jam-take:v1 carries no such length, so a received take ends with its last release.
   */
  async startTake(take: JamTake, offsetMs = 0, horizonMs = 0, offsetAtStart?: () => number): Promise<boolean> { return this.start(take, undefined, offsetMs, horizonMs, offsetAtStart); }

  /** Byte-capped package decode reuses PIX/take validators before any synthesis is prepared. */
  async startPixa(bytes: Uint8Array, offsetMs = 0): Promise<boolean> {
    let packageValue;
    try { packageValue = decodePixa(bytes); } catch { return false; }
    return this.startAnimation(packageValue.take, { fps: packageValue.fps, frameCount: packageValue.frames.length, sfx: packageValue.sfx }, offsetMs);
  }

  /** The same validated archival schedule for a decoded package; this grants no media consent. */
  async startAnimation(take: JamTake, animation: JamArchivalAnimation, offsetMs = 0, offsetAtStart?: () => number): Promise<boolean> {
    return this.start(take, animation, offsetMs, 0, offsetAtStart);
  }

  private async start(take: unknown, animation: JamArchivalAnimation | undefined, offsetMs: number, horizonMs: number, offsetAtStart?: () => number): Promise<boolean> {
    this.stop();
    if (!this.scope || this.deafened || !Number.isFinite(offsetMs) || offsetMs < 0 ||
      !Number.isSafeInteger(horizonMs) || horizonMs < 0 || horizonMs > TAKE_MAX_DURATION_MS) return false;
    let plan: JamArchivalPlan;
    try { plan = planJamArchivalPlayback(take, animation); } catch { return false; }
    // A late deck join/seek beyond EOF is a completed valid take, not a failed start or a fresh
    // replay of its final drum. No source is minted for an already elapsed complete horizon.
    if (offsetMs >= completionMs(plan, horizonMs)) { this.completedValue = true; return true; }
    const scope = this.scope, generation = this.generation;
    const current = () => generation === this.generation && !this.deafened && scopeEqual(scope, this.scope);
    this.stateValue = "preparing";
    // Invoke synchronously in the gesture turn, then fence both completion and later digests.
    let resumed: Promise<boolean>;
    try { resumed = this.engine.context.state === "suspended" ? this.engine.context.resume().then(() => true, () => false) : Promise.resolve(true); }
    catch { this.stateValue = "stopped"; return false; }
    let capability: JamArchivalPlayback | null;
    try {
      capability = await this.preparation.submit(async () => {
        if (!await resumed) return null;
        return current() ? this.engine.prepareArchivalPlayback(plan.take, plan.animation, current) : null;
      });
    } catch { capability = null; }
    if (!capability || !current()) {
      if (capability) this.engine.revokeArchivalPlayback(capability);
      if (current()) this.stateValue = "stopped";
      return false;
    }
    const admitted = this.engine.archivalPlaybackPlan(capability);
    if (!admitted) { this.engine.revokeArchivalPlayback(capability); this.stateValue = "stopped"; return false; }
    // A room listener must include download AND synth preparation time in its seek. Sample
    // only after the capability is ready; the DJ/local viewer keeps its pressed offset.
    try { if (offsetAtStart) offsetMs = offsetAtStart(); }
    catch { this.engine.revokeArchivalPlayback(capability); this.stateValue = "stopped"; return false; }
    if (!current() || !Number.isFinite(offsetMs) || offsetMs < 0) {
      this.engine.revokeArchivalPlayback(capability); this.stateValue = "stopped"; return false;
    }
    if (offsetMs >= completionMs(admitted, horizonMs)) {
      this.engine.revokeArchivalPlayback(capability); this.stateValue = "stopped"; this.completedValue = true; return true;
    }
    const seek = planJamArchivalSeek(admitted, offsetMs);
    const origin = this.nowMs();
    if (!Number.isFinite(origin)) { this.engine.revokeArchivalPlayback(capability); this.stateValue = "stopped"; return false; }
    this.active = { capability, plan: admitted, generation, origin, offset: offsetMs, complete: completionMs(admitted, horizonMs),
      next: seek.next, seek: seek.sounding, seekNext: 0, draining: false };
    this.lastTick = offsetMs;
    this.stateValue = "playing";
    return true;
  }

  /** At most 128 attacks/releases/cleanups per pass, including reconstructed seek voices. */
  async tick(): Promise<number> {
    const active = this.active;
    if (!active || active.draining) return 0;
    if (!this.scope || this.deafened || active.generation !== this.generation || !this.engine.archivalPlaybackPlan(active.capability)) {
      this.stop(); return 0;
    }
    active.draining = true;
    let count = 0;
    // A regressing/invalid clock never replays history or extends playback authority.
    const readElapsed = () => {
      const observed = this.elapsedMs;
      return this.lastTick = Number.isFinite(observed) ? Math.max(this.lastTick, observed) : this.lastTick;
    };
    try {
      while (count < TAKE_PLAYBACK_EVENTS_PER_TICK) {
        const elapsed = readElapsed();
        let index: number;
        if (active.seekNext < active.seek.length) index = active.seek[active.seekNext++];
        else {
          if (active.next >= active.plan.events.length || active.plan.events[active.next].ms > elapsed) break;
          index = active.next++;
        }
        const event = active.plan.events[index];
        count++;
        // A backgrounded tick must not turn already-ended notes into a new burst. Sequencing and
        // cleanup still advance; only attacks with future authored hold remain eligible.
        const attack = event.kind === "on" || event.kind === "drum";
        if (attack && event.offMs! <= elapsed) continue;
        const result = await this.engine.playArchivalEvent(active.capability, index, attack ? Math.max(0, elapsed - event.ms) : 0);
        if (this.active !== active || active.generation !== this.generation) return count;
        // A resolved refusal is as much a render failure as a rejection: stop without EOF so a
        // shared deck cannot advance past a take that never sounded.
        if (!result.ok && !DROPPED_EVENT.has(result.reason)) { this.stop(); return count; }
      }
      if (active.seekNext >= active.seek.length && active.next >= active.plan.events.length &&
        readElapsed() >= active.complete) { this.stop(); this.completedValue = true; }
      return count;
    } catch {
      // Rendering failure is not success and never strands a long archival note.
      if (this.active === active) this.stop();
      return count;
    } finally { active.draining = false; }
  }
}
