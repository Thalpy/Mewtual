/** Session scheduling only. Observations are never native permission: each leaf must
 * repeat its original binding/negative fences and obtain its own authenticated grant.
 * The caller prepays this whole graph before construction and keeps that fee through
 * actual task/timer return. Stopping this observer does not retire native uncertainty. */
import type { VoiceCryptoChain, VoiceIpcClock } from "./voice-crypto.ts";

export const VOICE_SESSION_LIFECYCLE_BYTES = 2 * 1024;
export type VoiceLifecycleStage = "waiting" | "confirmation" | "handoff" | "pc" | "rtc" | "media" | "retire" | "compact" | "failed" | "advance" | "stopped";
export type VoiceLifecycleObservation = Readonly<{
  generation: object; plan: VoiceCryptoChain | null; stage: VoiceLifecycleStage;
}>;
export type VoiceSessionLifecycleObserver = Readonly<{ close(): Promise<void> }>;
export type VoiceSessionLifecycleActions = Readonly<{
  live(): boolean;
  /** Fixed opaque scheduling metadata only. Never return BrowserChain, SDK or Phase. */
  observe(): VoiceLifecycleObservation;
  /** A returned void attempt may be busy/no-op. Reobserve actual state next turn. */
  step(observation: VoiceLifecycleObservation): Promise<void>;
}>;

type TimerCell = { work: ((generation: object) => void) | null };

/** One coalesced task and one cadence registration per retained Session. It lives
 * OUTSIDE the confirmation pump: compaction must be able to stop and join that pump.
 * No complete old negotiation is retained across its compaction/refund. */
export class VoiceSessionLifecycle {
  readonly handle: VoiceSessionLifecycleObserver;
  #actions: VoiceSessionLifecycleActions | null;
  #clock: VoiceIpcClock | null;
  #task: Promise<void> | null = null;
  #stopped = false;
  #lastObserved = -Infinity;
  #due = 0;
  #dueReached = false;
  #waiting: Promise<void> | null = null;
  #resolve: (() => void) | null = null;
  #timer: unknown = null;
  #timerSet = false;
  #generation: object | null = null;
  #registrationUnknown = false;
  #arming = false;
  #installing = false;
  #cancelling = false;
  #synchronousFire = false;
  #callbackDepth = 0;
  readonly #cell: TimerCell = { work: null };

  constructor(actions: VoiceSessionLifecycleActions, clock: VoiceIpcClock) {
    this.#actions = actions; this.#clock = clock;
    this.#cell.work = generation => this.#onTimer(generation);
    this.handle = Object.freeze({ close: () => this.close() });
  } // Inert: publication/payment precedes every owner/Clock/native callback.

  start(): void {
    this.#task ??= Promise.resolve().then(() => this.#drive()).catch(() => {
      // A bad local scheduler/clock stops adoption. drive's finally already joins every
      // actual registration and clears callbacks before this handled terminal completion.
    });
  }
  /** Internal caller joins actual worker return without requesting a stop. */
  join(): Promise<void> { return this.#task ?? Promise.resolve(); }
  close(): Promise<void> {
    this.#stop();
    if (!this.#task) {
      // Proven unstarted: there cannot be an SDK call, registration or callback debt.
      this.#cell.work = null; this.#actions = null; this.#clock = null;
    }
    return this.#task ?? Promise.resolve();
  }
  #live(): boolean {
    if (this.#stopped || !this.#actions) return false;
    let live = false;
    try { live = this.#actions.live(); } catch { /* A failed owner stops scheduling. */ }
    if (!live) this.#stop();
    return !this.#stopped && live;
  }
  async #drive(): Promise<void> {
    try {
      while (this.#live()) {
        try {
          const actions = this.#actions!, step = actions.step;
          const source = actions.observe();
          // Finish callback-bearing property reads before the last negative check. The
          // tiny snapshot contains no old graph, native proof, role or SDK object.
          const observation = Object.freeze({ generation: source.generation, plan: source.plan, stage: source.stage });
          if (!this.#live()) break; // observe/current callbacks may synchronously End.
          if (observation.stage === "stopped") { this.#stop(); break; }
          if (observation.stage !== "waiting") await step(observation);
        } catch { /* Keep the SAME original obligation; no new identity/deadline/refund. */ }
        if (this.#live()) await this.#wait();
      }
    } finally {
      this.#stop();
      // A scheduler may register then throw, or cancellation may fail. End is not proof
      // that the actual registration returned; retain this task/payment until it joins.
      if (this.#waiting) await this.#waiting;
      this.#cell.work = null; this.#generation = null;
      this.#actions = null; this.#clock = null;
    }
  }
  #now(): number {
    const now = this.#clock!.now();
    if (!Number.isFinite(now) || now < this.#lastObserved) throw new Error("invalid voice lifecycle clock");
    this.#lastObserved = now; return now;
  }
  #wait(): Promise<void> {
    if (this.#waiting) return this.#waiting;
    const began = this.#now(); this.#due = began + 250;
    // Finite numbers alone are insufficient: a huge clock can round +250 back to now
    // and turn a waiting negotiation into an unbounded microtask loop.
    if (!Number.isFinite(this.#due) || this.#due <= began) throw new Error("invalid voice lifecycle cadence");
    this.#dueReached = false;
    const waiting = new Promise<void>(resolve => { this.#resolve = resolve; });
    this.#waiting = waiting;
    this.#arm();
    return waiting;
  }
  #cancel(): void {
    if (!this.#timerSet || this.#cancelling) return;
    this.#cancelling = true;
    try {
      this.#clock!.cancel(this.#timer);
      this.#timerSet = false; this.#timer = null;
    } finally { this.#cancelling = false; }
  }
  #stop(): void {
    this.#stopped = true; // Negative publication precedes cancellation/Clock callbacks.
    try { this.#cancel(); } catch { /* Same handle/debt remains for actual return or retry. */ }
    this.#finishWait();
  }
  #arm(): void {
    if (this.#arming || this.#installing || this.#cancelling) return;
    this.#arming = true;
    try {
      this.#cancel();
      if (this.#stopped) return;
      const remaining = this.#due - this.#now();
      if (this.#stopped) return; // Clock callbacks cannot submit work after End.
      if (remaining <= 0) { this.#dueReached = true; return; }
      const generation = {}; this.#generation = generation;
      const cell = this.#cell;
      this.#registrationUnknown = true; this.#installing = true; this.#synchronousFire = false;
      try {
        this.#timer = this.#clock!.schedule(() => cell.work?.(generation), remaining);
        this.#timerSet = true; this.#registrationUnknown = false;
      } catch { this.#stopped = true; /* Only this registration's callback can clear unknown debt. */ }
      finally { this.#installing = false; }
      // An inline callback is not a future cadence. Stop without recursive scheduling;
      // cancellation still joins its returned handle before task/fee completion.
      if (this.#synchronousFire) this.#stopped = true;
      if (this.#stopped) this.#cancel();
    } catch { this.#stopped = true; }
    finally { this.#arming = false; this.#finishWait(); }
  }
  #onTimer(generation: object): void {
    if (this.#generation !== generation) return; // An old callback cannot join new debt.
    this.#callbackDepth++;
    try {
      if (this.#installing) { this.#synchronousFire = true; this.#registrationUnknown = false; return; }
      this.#timerSet = false; this.#timer = null; this.#registrationUnknown = false;
      if (!this.#stopped) this.#dueReached = this.#now() >= this.#due;
    } catch { this.#stopped = true; }
    finally {
      this.#callbackDepth--;
      if (!this.#stopped && !this.#dueReached) this.#arm();
      this.#finishWait();
    }
  }
  #finishWait(): void {
    if (this.#arming || this.#installing || this.#cancelling || this.#callbackDepth !== 0 ||
        this.#timerSet || this.#registrationUnknown || (!this.#stopped && !this.#dueReached)) return;
    const resolve = this.#resolve;
    this.#resolve = null; this.#waiting = null;
    resolve?.(); // No Clock, native or owner callback after negative checks/publication.
  }
}
